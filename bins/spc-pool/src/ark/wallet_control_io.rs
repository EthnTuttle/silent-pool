//! Bounded, read-only Bitcoin RPC batches. No wallet or network money effect.
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bitcoincore_rpc::{jsonrpc, Client, RpcApi};

pub(super) const READ_DEADLINE: Duration = Duration::from_secs(5);
pub(super) const RPC_TIMEOUT: Duration = Duration::from_secs(1);
pub(super) const SYNC_INTERVAL: Duration = Duration::from_secs(10);
pub(super) const MAINTENANCE_INTERVAL: Duration = Duration::from_secs(60);
pub(super) const MAX_BATCH_BLOCKS: usize = 32;
const MAX_CALLS: usize = 160;

struct Budget {
    deadline: Instant,
    remaining: usize,
}

pub(super) struct ReadRpc {
    client: Client,
    budget: Mutex<Budget>,
}

impl ReadRpc {
    pub fn new(url: &str, user: &str, pass: &str) -> anyhow::Result<Self> {
        let mut transport = jsonrpc::http::simple_http::Builder::new()
            .url(url)?
            .timeout(RPC_TIMEOUT);
        if !user.is_empty() {
            transport = transport.auth(user, Some(pass));
        }
        let client = Client::from_jsonrpc(jsonrpc::Client::with_transport(transport.build()));
        Ok(Self {
            client,
            budget: Mutex::new(Budget {
                deadline: Instant::now() + READ_DEADLINE,
                remaining: MAX_CALLS,
            }),
        })
    }

    pub fn reset(&self) {
        *self.budget.lock().expect("read budget mutex") = Budget {
            deadline: Instant::now() + READ_DEADLINE,
            remaining: MAX_CALLS,
        };
    }
}

impl RpcApi for ReadRpc {
    fn call<T: for<'a> serde::Deserialize<'a>>(
        &self,
        method: &str,
        args: &[serde_json::Value],
    ) -> Result<T, bitcoincore_rpc::Error> {
        // Prevent a future caller from accidentally cancelling/budgeting a write.
        if !matches!(
            method,
            "getblock"
                | "getblockheader"
                | "getblockhash"
                | "getbestblockhash"
                | "getblockcount"
                | "getblockchaininfo"
                | "getrawtransaction"
                | "gettxout"
        ) {
            return Err(bitcoincore_rpc::Error::ReturnedError(
                "read-only RPC method refused".into(),
            ));
        }
        {
            let mut budget = self.budget.lock().expect("read budget mutex");
            if budget.remaining == 0 || Instant::now() >= budget.deadline {
                return Err(bitcoincore_rpc::Error::ReturnedError(
                    "read RPC batch budget exhausted".into(),
                ));
            }
            budget.remaining -= 1;
        }
        self.client.call(method, args)
    }
}

pub(super) type Emitter = bdk_bitcoind_rpc::Emitter<Arc<ReadRpc>>;
pub(super) type BlockEvent = bdk_bitcoind_rpc::BlockEvent<bitcoin::Block>;

pub(super) struct ReadBatch {
    pub blocks: Vec<BlockEvent>,
    pub complete: bool,
}

/// The blocking emitter runs without the wallet operation mutex. Each RPC has
/// a real socket timeout; the whole batch has a call/block/deadline budget.
/// Preserve emitter progress across partial batches and deep reorg searches.
pub(super) fn read_batch(emitter: &mut Emitter, rpc: &ReadRpc) -> ReadBatch {
    rpc.reset();
    let mut blocks = Vec::new();
    for _ in 0..MAX_BATCH_BLOCKS {
        match emitter.next_block() {
            Ok(Some(block)) => blocks.push(block),
            Ok(None) => {
                return ReadBatch {
                    blocks,
                    complete: true,
                }
            }
            Err(_) => break,
        }
    }
    ReadBatch {
        blocks,
        complete: false,
    }
}

#[derive(Default)]
pub(super) struct Freshness {
    completed: Option<(bitcoin::BlockHash, Instant)>,
}
impl Freshness {
    pub fn invalidate(&mut self) {
        self.completed = None;
    }
    pub fn complete(&mut self, hash: bitcoin::BlockHash) {
        self.completed = Some((hash, Instant::now()));
    }
    pub fn current(&self, hash: bitcoin::BlockHash) -> bool {
        self.completed
            .is_some_and(|(h, at)| h == hash && at.elapsed() <= Duration::from_secs(30))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::hashes::Hash;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    fn reply(mut stream: std::net::TcpStream) {
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut length = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            if let Some(n) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = n.trim().parse().unwrap();
            }
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(request["method"], "getblockcount");
        let bytes =
            serde_json::to_vec(&serde_json::json!({"id":request["id"],"result":0,"error":null}))
                .unwrap();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            bytes.len()
        )
        .unwrap();
        stream.write_all(&bytes).unwrap();
    }

    #[test]
    fn large_read_scan_stops_at_its_rpc_budget_before_another_network_request() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                reply(listener.accept().unwrap().0);
            }
        });
        let rpc = ReadRpc::new(&format!("http://{address}"), "", "").unwrap();
        rpc.budget.lock().unwrap().remaining = 2;
        assert_eq!(rpc.get_block_count().unwrap(), 0);
        assert_eq!(rpc.get_block_count().unwrap(), 0);
        let error = rpc.get_block_count().unwrap_err();
        assert!(error.to_string().contains("budget exhausted"));
        server.join().unwrap();
        assert!(rpc
            .call::<serde_json::Value>("sendrawtransaction", &[])
            .unwrap_err()
            .to_string()
            .contains("read-only"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stalled_socket_read_is_bounded_and_does_not_own_wallet_send_capacity() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (connected, seen) = tokio::sync::oneshot::channel();
        let server = std::thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            connected.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(1600));
        });
        let rpc = ReadRpc::new(&format!("http://{address}"), "", "").unwrap();
        let capacity = Arc::new(tokio::sync::Mutex::new(()));
        let started = Instant::now();
        let read = tokio::task::spawn_blocking(move || rpc.get_block_count());
        seen.await.unwrap();
        assert!(capacity.try_lock().is_ok());
        assert!(read.await.unwrap().is_err());
        assert!(started.elapsed() < READ_DEADLINE);
        server.join().unwrap();
    }

    #[test]
    fn incomplete_or_expired_chain_observation_is_never_freshened() {
        let hash = bitcoin::BlockHash::all_zeros();
        let mut freshness = Freshness::default();
        assert!(!freshness.current(hash));
        freshness.complete(hash);
        assert!(freshness.current(hash));
        let completed = freshness.completed.unwrap();
        freshness.invalidate();
        assert!(!freshness.current(hash));
        freshness.completed = Some((completed.0, Instant::now() - Duration::from_secs(31)));
        assert!(!freshness.current(hash));
    }
}
