//! HTTP protocol bounds: 16 KiB header budget enforced before buffering,
//! bounded body, unsupported framing, cursor validation, and SSE
//! Last-Event-ID precedence (Fable B8/B10 edges).

mod common;

use std::io::{BufRead, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use spc_lab::http::{HttpServer, WebAssets};

const READ_DEADLINE: Duration = Duration::from_secs(5);

struct RawResponse {
    status: u16,
    head: String,
    body: String,
}

/// Read a full response with a deadline instead of a sleep. Tolerates
/// partial reads and the server closing the connection.
fn read_response(stream: &mut TcpStream, deadline: Instant) -> std::io::Result<RawResponse> {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 1024];
    let mut split = None;
    loop {
        if Instant::now() >= deadline {
            break;
        }
        stream.set_read_timeout(Some(Duration::from_millis(100)))?;
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => bytes.extend_from_slice(&chunk[..n]),
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::TimedOut =>
            {
                // Once the head is complete, only drain until the server
                // closes; the timeout with nothing left means the body is
                // done for a Connection: close response.
                if split.is_some() {
                    break;
                }
                continue;
            }
            Err(error) => return Err(error),
        }
        if split.is_none() {
            split = bytes.windows(4).position(|window| window == b"\r\n\r\n");
        }
        // Connection: close responses end when the server closes the socket;
        // SSE heads complete earlier and end via the timeout path above.
    }
    let split = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap_or(bytes.len());
    let head = String::from_utf8_lossy(&bytes[..split]).to_string();
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let body = String::from_utf8_lossy(&bytes[split + 4..]).to_string();
    Ok(RawResponse { status, head, body })
}

fn connect(addr: SocketAddr) -> std::io::Result<TcpStream> {
    let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))?;
    stream.set_nodelay(true)?;
    Ok(stream)
}

fn get(addr: SocketAddr, path: &str, extra: &[(&str, &str)]) -> std::io::Result<RawResponse> {
    let mut stream = connect(addr)?;
    let mut request = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n");
    for (name, value) in extra {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes())?;
    stream.flush()?;
    read_response(&mut stream, Instant::now() + READ_DEADLINE)
}

/// A minimal SSE client that can send `Last-Event-ID` alongside `after`.
struct SseClient {
    reader: std::io::BufReader<TcpStream>,
}

impl SseClient {
    fn open(
        addr: SocketAddr,
        run_id: &str,
        after: u64,
        extra: &[(&str, &str)],
    ) -> std::io::Result<Self> {
        let mut stream = connect(addr)?;
        stream.set_read_timeout(Some(READ_DEADLINE))?;
        let mut request = format!(
            "GET /api/lab/v1/runs/{run_id}/events?after={after} HTTP/1.1\r\nHost: {addr}\r\nConnection: keep-alive\r\n"
        );
        for (name, value) in extra {
            request.push_str(&format!("{name}: {value}\r\n"));
        }
        request.push_str("\r\n");
        stream.write_all(request.as_bytes())?;
        stream.flush()?;
        let mut reader = std::io::BufReader::new(stream);
        let mut status = String::new();
        reader.read_line(&mut status)?;
        assert!(status.contains("200"), "SSE status: {status}");
        loop {
            let mut line = String::new();
            reader.read_line(&mut line)?;
            if line == "\r\n" || line == "\n" || line.is_empty() {
                break;
            }
        }
        Ok(Self { reader })
    }

    fn next_event(&mut self) -> std::io::Result<Option<(String, u64, String)>> {
        let mut id = 0u64;
        let mut event = "message".to_string();
        loop {
            let mut line = String::new();
            if self.reader.read_line(&mut line)? == 0 {
                return Ok(None);
            }
            if let Some(value) = line.strip_prefix("id: ") {
                id = value.trim().parse().unwrap_or(0);
            }
            if let Some(value) = line.strip_prefix("event: ") {
                event = value.trim().to_string();
            }
            if let Some(data) = line.strip_prefix("data: ") {
                return Ok(Some((event, id, data.trim().to_string())));
            }
            if line == "\r\n" || line == "\n" {
                // End of one frame without data (comment/keepalive): reset.
                event = "message".to_string();
            }
        }
    }
}

fn started_server(
    label: &str,
) -> (
    common::TestDir,
    std::sync::Arc<spc_lab::engine::RunEngine>,
    HttpServer,
) {
    let dir = common::TestDir::new(label);
    let engine = common::prepare_engine(&dir, "scenario_v1_one_pool_two_miners.json");
    engine.start().unwrap();
    let server = HttpServer::start(engine.clone(), "127.0.0.1:0", WebAssets::fallback()).unwrap();
    (dir, engine, server)
}

fn snapshot_cursor(engine: &spc_lab::engine::RunEngine) -> u64 {
    engine.snapshot(None).snapshot_cursor
}

#[test]
fn request_line_without_crlf_is_rejected_within_the_16kib_bound() {
    let (_dir, engine, server) = started_server("http-protocol-nocrlf");
    let addr = server.addr();
    let mut stream = connect(addr).unwrap();
    // Exactly 16 KiB with no CRLF, and nothing more ever sent: the server
    // must answer 400 from the budget alone, without waiting for a
    // terminator or buffering past it (a read_line-then-check implementation
    // blocks here until the deadline). Writing exactly the budget also means
    // the server consumes every byte, so the response arrives as a clean FIN.
    stream.write_all(&[b'A'; 16 * 1024]).unwrap();
    stream.flush().unwrap();
    let response = read_response(&mut stream, Instant::now() + READ_DEADLINE).unwrap();
    assert_eq!(response.status, 400);
    assert!(
        response.head.contains("Bad Request"),
        "unexpected head: {}",
        response.head
    );
    engine.stop().unwrap();
    server.shutdown();
}

/// Build a request whose byte total is exactly the 16 KiB budget, with the
/// final line unterminated. The server consumes every byte before rejecting,
/// so the response is delivered as a clean FIN (no unread data, no RST) and
/// the client can assert the status deterministically.
fn exact_budget_request(addr: SocketAddr) -> Vec<u8> {
    let request_line = "GET / HTTP/1.1\r\n".to_string();
    let host = format!("Host: {addr}\r\n");
    let fixed = request_line.len() + host.len();
    assert!(fixed + 8 < 16 * 1024);
    let pad = 16 * 1024 - fixed - "X-Pad: ".len();
    let mut request = request_line;
    request.push_str(&host);
    request.push_str("X-Pad: ");
    request.push_str(&"P".repeat(pad));
    // Unterminated final padding: the budget runs out with no CRLF in sight.
    request.into_bytes()
}

#[test]
fn header_block_at_the_total_16kib_budget_is_rejected_without_a_terminator() {
    let (_dir, engine, server) = started_server("http-protocol-headers");
    let addr = server.addr();
    // The request line plus one header line exactly fill the budget and the
    // header never terminates: the bound fires before the terminator does.
    assert_eq!(exact_budget_request(addr).len(), 16 * 1024);
    let mut stream = connect(addr).unwrap();
    stream.write_all(&exact_budget_request(addr)).unwrap();
    stream.flush().unwrap();
    let response = read_response(&mut stream, Instant::now() + READ_DEADLINE).unwrap();
    assert_eq!(response.status, 400);

    // Many small headers whose total (with the request line) reaches the
    // budget, again with the final line unterminated.
    let request_line = "GET / HTTP/1.1\r\n".to_string();
    let mut request = request_line.clone();
    request.push_str(&format!("Host: {addr}\r\n"));
    let mut total = request.len();
    let mut index = 0;
    while total + 16 < 16 * 1024 {
        let line = format!("X-Pad-{index}: v\r\n");
        total += line.len();
        request.push_str(&line);
        index += 1;
    }
    // Replace the terminator with an unterminated final header that lands
    // exactly on the budget.
    let remaining = 16 * 1024 - total - "X-End: ".len();
    request.push_str("X-End: ");
    request.push_str(&"Z".repeat(remaining));
    assert_eq!(request.len(), 16 * 1024);
    let mut stream = connect(addr).unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    stream.flush().unwrap();
    let response = read_response(&mut stream, Instant::now() + READ_DEADLINE).unwrap();
    assert_eq!(response.status, 400);
    engine.stop().unwrap();
    server.shutdown();
}

#[test]
fn oversized_body_and_unsupported_framing_are_rejected() {
    let (_dir, engine, server) = started_server("http-protocol-body");
    let addr = server.addr();
    let run_id = engine.run_id().to_string();

    // 300 KiB content-length: rejected before the body is read, so the client
    // does not need to send it.
    let mut stream = connect(addr).unwrap();
    write!(
        stream,
        "POST /api/lab/v1/runs/{run_id}/commands HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: 300000\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    stream.flush().unwrap();
    let response = read_response(&mut stream, Instant::now() + READ_DEADLINE).unwrap();
    assert_eq!(response.status, 413);

    // Chunked framing is refused clearly instead of misread as a body.
    let mut stream = connect(addr).unwrap();
    write!(
        stream,
        "POST /api/lab/v1/runs/{run_id}/commands HTTP/1.1\r\nHost: {addr}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n0\r\n\r\n"
    )
    .unwrap();
    stream.flush().unwrap();
    let response = read_response(&mut stream, Instant::now() + READ_DEADLINE).unwrap();
    assert_eq!(response.status, 400);
    let explained = format!("{}{}", response.head, response.body).to_ascii_lowercase();
    assert!(
        explained.contains("transfer-encoding"),
        "rejection must name the problem: {}{}",
        response.head,
        response.body
    );

    // A malformed content-length is a 400, never a silent zero-length body.
    let mut stream = connect(addr).unwrap();
    write!(
        stream,
        "POST /api/lab/v1/runs/{run_id}/commands HTTP/1.1\r\nHost: {addr}\r\nContent-Length: not-a-number\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    stream.flush().unwrap();
    let response = read_response(&mut stream, Instant::now() + READ_DEADLINE).unwrap();
    assert_eq!(response.status, 400);
    engine.stop().unwrap();
    server.shutdown();
}

#[test]
fn malformed_cursors_are_rejected_not_silently_zero() {
    let (_dir, engine, server) = started_server("http-protocol-cursors");
    let addr = server.addr();
    let run_id = engine.run_id().to_string();

    let response = get(
        addr,
        &format!("/api/lab/v1/runs/{run_id}/events?after=bogus"),
        &[],
    )
    .unwrap();
    assert_eq!(response.status, 400);
    assert!(response.head.contains("Bad Request"));

    let response = get(
        addr,
        &format!("/api/lab/v1/runs/{run_id}/snapshot?cursor=1.5"),
        &[],
    )
    .unwrap();
    assert_eq!(response.status, 400);

    let response = get(
        addr,
        &format!("/api/lab/v1/runs/{run_id}/events?after=0"),
        &[("Last-Event-ID", "not-a-number")],
    )
    .unwrap();
    assert_eq!(response.status, 400);

    // A well-formed cursor is still accepted.
    let response = get(
        addr,
        &format!("/api/lab/v1/runs/{run_id}/events?after=0"),
        &[],
    )
    .unwrap();
    assert_eq!(response.status, 200);
    engine.stop().unwrap();
    server.shutdown();
}

#[test]
fn last_event_id_takes_precedence_over_after_and_avoids_duplicate_replay() {
    let (_dir, engine, server) = started_server("http-protocol-last-event-id");
    let addr = server.addr();
    let run_id = engine.run_id().to_string();

    // Guarantee events beyond id 2 exist.
    let command = common::command(
        &run_id,
        serde_json::json!({"type": "pause_workers", "miner_id": "miner-1", "workers": []}),
    );
    common::http_request(
        addr,
        "POST",
        &format!("/api/lab/v1/runs/{run_id}/commands"),
        Some(&command.to_string()),
    )
    .unwrap();
    let last = snapshot_cursor(&engine);
    assert!(last >= 4, "expected a few events, got {last}");

    // after=0 would replay from id 1; Last-Event-ID: 2 must win.
    let mut client = SseClient::open(addr, &run_id, 0, &[("Last-Event-ID", "2")]).unwrap();
    let mut seen = Vec::new();
    for _ in 0..(last - 2) {
        let (_, id, _) = client
            .next_event()
            .unwrap()
            .expect("replayed the retained tail");
        seen.push(id);
    }
    assert_eq!(
        seen.first().copied(),
        Some(3),
        "replay resumes at 3: {seen:?}"
    );
    assert!(seen.iter().all(|id| *id > 2), "no duplicate <= 2: {seen:?}");
    engine.stop().unwrap();
    server.shutdown();
}

#[test]
fn future_cursor_is_a_resync_never_a_silent_skip() {
    let (_dir, engine, server) = started_server("http-protocol-future-cursor");
    let addr = server.addr();
    let run_id = engine.run_id().to_string();
    let last = snapshot_cursor(&engine);

    // A cursor beyond the last event must produce an explicit resync
    // snapshot, not an empty stream that silently misses everything.
    let mut client = SseClient::open(addr, &run_id, last + 1_000, &[]).unwrap();
    let (event, id, data) = client
        .next_event()
        .unwrap()
        .expect("future cursor answers with a resync");
    assert_eq!(event, "resync");
    let snapshot: serde_json::Value = serde_json::from_str(&data).unwrap();
    assert_eq!(snapshot["resync_required"], true);
    assert!(snapshot["snapshot_cursor"].as_u64().unwrap() < last + 1_000);
    assert!(id > 0);
    engine.stop().unwrap();
    server.shutdown();
}

#[test]
fn ordinary_sse_events_use_the_generic_message_type() {
    let (_dir, engine, server) = started_server("http-protocol-message-type");
    let addr = server.addr();
    let run_id = engine.run_id().to_string();
    let last = snapshot_cursor(&engine);

    let mut client = SseClient::open(addr, &run_id, 0, &[]).unwrap();
    let mut kinds = Vec::new();
    for _ in 0..last {
        let (event, id, data) = client.next_event().unwrap().expect("replayed event");
        assert!(id > 0);
        let parsed: serde_json::Value = serde_json::from_str(&data).unwrap();
        // Every ordinary evidence event carries its kind in the payload and
        // uses the generic SSE type, so additive kinds need no client change.
        assert_eq!(
            event, "message",
            "payload kind: {}",
            parsed["payload"]["kind"]
        );
        assert!(
            parsed["payload"]["kind"].is_string(),
            "payload must carry kind: {data}"
        );
        kinds.push(parsed["payload"]["kind"].clone());
    }
    assert!(!kinds.is_empty());
    engine.stop().unwrap();
    server.shutdown();
}
