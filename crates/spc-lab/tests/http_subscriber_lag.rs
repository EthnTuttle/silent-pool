mod common;

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::Duration;

use spc_lab::evidence::{EvidenceActor, EvidencePayload, UnknownEvidence};
use spc_lab::http::{HttpServer, WebAssets};
use spc_lab::ids::SourceId;

#[test]
fn an_overflowed_sse_subscriber_gets_an_explicit_resync_not_a_silent_skip() {
    let dir = common::TestDir::new("http-subscriber-lag");
    let engine = common::prepare_engine(&dir, "scenario_v1_one_pool_two_miners.json");
    engine.start().unwrap();
    let server = HttpServer::start(engine.clone(), "127.0.0.1:0", WebAssets::fallback()).unwrap();
    let addr = server.addr();
    let run_id = engine.run_id().to_string();
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    write!(
        stream,
        "GET /api/lab/v1/runs/{run_id}/events?after=0 HTTP/1.1\r\nHost: {addr}\r\nConnection: keep-alive\r\n\r\n"
    )
    .unwrap();
    stream.flush().unwrap();
    let mut reader = BufReader::new(stream);
    let mut status = String::new();
    reader.read_line(&mut status).unwrap();
    assert!(status.contains("200"), "{status}");
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if line == "\r\n" || line == "\n" {
            break;
        }
    }
    // Do not consume the stream while the bounded queue overflows.
    for index in 0..300 {
        engine.evidence().append(
            SourceId::new("test").unwrap(),
            format!("lag-{index}"),
            EvidenceActor::harness(),
            None,
            EvidencePayload::Unknown(UnknownEvidence {
                topic: "lag".to_string(),
                detail: "x".repeat(64 * 1024),
            }),
        );
    }
    // The server must close and answer with resync, rather than let the client
    // continue across a silent id gap.
    let mut saw_resync = false;
    for _ in 0..2_000 {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap() == 0 {
            break;
        }
        if line.trim() == "event: resync" {
            saw_resync = true;
            break;
        }
    }
    assert!(saw_resync, "overflowed subscriber did not receive resync");
    assert!(engine
        .evidence()
        .gaps()
        .iter()
        .any(|gap| gap.scope == spc_lab::evidence::GapScope::Subscriber));
    // Durable assertions remain valid because the store did not lose anything.
    assert_eq!(
        engine
            .evidence()
            .evaluate_assertion(&spc_lab::scenario::AssertionSpec {
                id: "started".to_string(),
                kind: spc_lab::scenario::AssertionKind::Evidence,
                predicate: "exists(run_state)".to_string(),
                deadline_secs: 1,
            }),
        spc_lab::evidence::AssertionEvaluation::Passed
    );
    engine.stop().unwrap();
    server.shutdown();
}
