use spc_lab::adapters::RpcNodeControl;
use spc_lab::ids::NodeId;
use spc_lab::resource::Endpoint;
use spc_lab::traffic::rpc::{RpcOutcome, UnknownClass};
use std::io::{Read, Write};
use std::net::TcpListener;

fn call(body: Vec<u8>) -> RpcOutcome {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        let mut bytes = Vec::new();
        let mut block = [0; 1024];
        loop {
            let n = socket.read(&mut block).unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&block[..n]);
            if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                let text = std::str::from_utf8(&bytes[..end]).unwrap();
                let length = text
                    .lines()
                    .find_map(|line| line.strip_prefix("Content-Length: "))
                    .unwrap()
                    .trim()
                    .parse::<usize>()
                    .unwrap();
                if bytes.len() >= end + 4 + length {
                    break;
                }
            }
        }
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        socket.write_all(header.as_bytes()).unwrap();
        let _ = socket.write_all(&body);
    });
    let rpc = RpcNodeControl::new(
        NodeId::trusted("fixture"),
        Endpoint {
            host: "127.0.0.1".into(),
            port: address.port(),
        },
        None,
    );
    let result = rpc.call_traffic(7, "getmempoolinfo", serde_json::json!([]));
    server.join().unwrap();
    result
}
#[test]
fn host_wire_checks_id_integer_errors_and_body_bounds() {
    assert_eq!(
        call(br#"{"id":8,"result":{"size":1},"error":null}"#.to_vec()),
        RpcOutcome::unknown(UnknownClass::Protocol)
    );
    assert_eq!(
        call(br#"{"id":7,"error":{"code":"-26","message":"version"}}"#.to_vec()),
        RpcOutcome::unknown(UnknownClass::Shape)
    );
    assert_eq!(
        call(br#"{"id":7,"error":{"code":-26,"message":"version"}}"#.to_vec()),
        RpcOutcome::NodeRejected {
            code: -26,
            message: "version".into()
        }
    );
    assert_eq!(
        call(br#"{"id":7,"result":{"size":1},"error":null}"#.to_vec()),
        RpcOutcome::Ok {
            result: serde_json::json!({"size":1})
        }
    );
    assert_eq!(
        call(vec![b' '; spc_lab_txd::control::MAX_BODY + 1]),
        RpcOutcome::unknown(UnknownClass::Shape)
    );
}
