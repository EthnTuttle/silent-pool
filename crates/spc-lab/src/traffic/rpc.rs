//! Typed traffic-only HTTP transport; legacy NodeControl calls keep their API.
use crate::resource::Endpoint;
use serde_json::Value;
use spc_lab_txd::control::{parse_rpc_response, MAX_BODY};
pub use spc_lab_txd::control::{RpcOutcome, RpcRequest, UnknownClass};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

pub fn call_direct(
    endpoint: &Endpoint,
    auth: Option<&str>,
    id: u64,
    method: &str,
    params: Value,
) -> RpcOutcome {
    let result = (|| -> Result<Vec<u8>, UnknownClass> {
        let body = serde_json::to_vec(
            &serde_json::json!({"jsonrpc":"1.0", "id":id, "method":method, "params":params}),
        )
        .map_err(|_| UnknownClass::Shape)?;
        if body.len() > MAX_BODY || method.len() > 64 {
            return Err(UnknownClass::Shape);
        }
        let addr = endpoint.socket_addr().map_err(|_| UnknownClass::Shape)?;
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut socket =
            TcpStream::connect_timeout(&addr, Duration::from_secs(3)).map_err(io_class)?;
        socket
            .set_write_timeout(Some(Duration::from_secs(3)))
            .map_err(io_class)?;
        let mut headers = format!("POST / HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n", endpoint.display(), body.len());
        if let Some(auth) = auth {
            if auth.contains(['\r', '\n']) {
                return Err(UnknownClass::Shape);
            }
            headers.push_str(&format!("Authorization: {auth}\r\n"));
        }
        headers.push_str("\r\n");
        socket
            .write_all(headers.as_bytes())
            .and_then(|_| socket.write_all(&body))
            .map_err(io_class)?;
        let mut bytes = Vec::new();
        let mut chunk = [0; 8192];
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or(UnknownClass::Timeout)?;
            if remaining.is_zero() {
                return Err(UnknownClass::Timeout);
            }
            socket.set_read_timeout(Some(remaining)).map_err(io_class)?;
            let n = socket.read(&mut chunk).map_err(io_class)?;
            if n == 0 {
                break;
            }
            if bytes.len() + n > MAX_BODY + 8192 {
                return Err(UnknownClass::Shape);
            }
            bytes.extend_from_slice(&chunk[..n]);
        }
        let split = bytes
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .ok_or(UnknownClass::Protocol)?;
        if split > 8192 {
            return Err(UnknownClass::Shape);
        }
        let headers = std::str::from_utf8(&bytes[..split]).map_err(|_| UnknownClass::Protocol)?;
        let mut lines = headers.split("\r\n");
        let status = lines.next().ok_or(UnknownClass::Protocol)?;
        if !status.starts_with("HTTP/1.") {
            return Err(UnknownClass::Protocol);
        }
        let mut length = None;
        for line in lines {
            let (name, value) = line.split_once(':').ok_or(UnknownClass::Protocol)?;
            if name.eq_ignore_ascii_case("transfer-encoding") {
                return Err(UnknownClass::Protocol);
            }
            if name.eq_ignore_ascii_case("content-length") {
                if length.is_some() {
                    return Err(UnknownClass::Protocol);
                }
                length = Some(
                    value
                        .trim()
                        .parse::<usize>()
                        .map_err(|_| UnknownClass::Protocol)?,
                );
            }
        }
        let body = &bytes[split + 4..];
        if body.len() > MAX_BODY || length.is_some_and(|n| n != body.len()) {
            return Err(UnknownClass::Shape);
        }
        Ok(body.to_vec())
    })();
    match result {
        Ok(bytes) => parse_rpc_response(&bytes, id),
        Err(class) => RpcOutcome::unknown(class),
    }
}

fn io_class(error: std::io::Error) -> UnknownClass {
    match error.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => UnknownClass::Timeout,
        _ => UnknownClass::Transport,
    }
}
