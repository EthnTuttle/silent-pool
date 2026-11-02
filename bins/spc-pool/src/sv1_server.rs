//! Native SV1 TCP listener. The protocol rules live in [`crate::sv1`]; this
//! module only moves lines and jobs.

use std::net::SocketAddr;
use std::sync::Arc;

use serde_json::Value;
use spc_crypto::Network;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, Mutex};

use crate::sv1::{notify, Job, Session, Share};

/// Shared listener state. `publish` is called by the front whenever a new job
/// is built; `shares` is consumed by the slice pipeline.
#[derive(Clone)]
pub struct Sv1Server {
    network: Network,
    extranonce1_len: usize,
    extranonce2_len: usize,
    difficulty: f64,
    current: Arc<Mutex<Option<Job>>>,
    jobs: broadcast::Sender<Job>,
    shares: mpsc::Sender<Share>,
}

impl Sv1Server {
    pub fn new(
        network: Network,
        extranonce1_len: usize,
        extranonce2_len: usize,
        difficulty: f64,
        shares: mpsc::Sender<Share>,
    ) -> Self {
        let (jobs, _) = broadcast::channel(16);
        Self {
            network,
            extranonce1_len,
            extranonce2_len,
            difficulty,
            current: Arc::new(Mutex::new(None)),
            jobs,
            shares,
        }
    }

    pub async fn publish(&self, job: Job) {
        *self.current.lock().await = Some(job.clone());
        let _ = self.jobs.send(job);
    }

    /// Bind and serve until the listener is dropped. Returns the bound address.
    pub async fn bind(
        &self,
        addr: &str,
    ) -> std::io::Result<(SocketAddr, tokio::task::JoinHandle<()>)> {
        let listener = TcpListener::bind(addr).await?;
        let local = listener.local_addr()?;
        let server = self.clone();
        let handle = tokio::spawn(async move {
            while let Ok((socket, _peer)) = listener.accept().await {
                let server = server.clone();
                tokio::spawn(async move {
                    let _ = server.handle(socket).await;
                });
            }
        });
        Ok((local, handle))
    }

    async fn handle(&self, socket: TcpStream) -> std::io::Result<()> {
        let (read, mut write) = socket.into_split();
        let mut lines = BufReader::new(read).lines();
        let mut extranonce1 = vec![0u8; self.extranonce1_len];
        {
            use rand::RngCore;
            rand::rngs::OsRng.fill_bytes(&mut extranonce1);
        }
        let mut session = Session::new(
            self.network,
            rand_channel_id(),
            extranonce1,
            self.extranonce2_len,
        );
        let mut jobs = self.jobs.subscribe();
        let mut subscribed = false;

        loop {
            tokio::select! {
                line = lines.next_line() => {
                    let Some(line) = line? else { break };
                    let (notifications, response) = session.handle_line(&line);
                    for n in notifications {
                        write.write_all(n.as_bytes()).await?;
                        write.write_all(b"\n").await?;
                    }
                    if let Some(r) = response {
                        write.write_all(r.as_bytes()).await?;
                        write.write_all(b"\n").await?;
                    }
                    if let Some(share) = session.take_share() {
                        let _ = self.shares.send(share).await;
                    }
                    if !subscribed && session.sp_id().is_some() {
                        subscribed = true;
                        send_difficulty(&mut write, self.difficulty).await?;
                        if let Some(job) = self.current.lock().await.clone() {
                            session.set_job(job.clone());
                            send_notify(&mut write, &job).await?;
                        }
                    }
                }
                job = jobs.recv() => {
                    match job {
                        Ok(job) => {
                            if subscribed {
                                session.set_job(job.clone());
                                send_notify(&mut write, &job).await?;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            // A miner missed a job; the next clean job resets it.
                        }
                        Err(_) => break,
                    }
                }
            }
        }
        Ok(())
    }
}

async fn send_difficulty<W: AsyncWriteExt + Unpin>(w: &mut W, d: f64) -> std::io::Result<()> {
    let line = serde_json::json!({
        "id": null,
        "method": "mining.set_difficulty",
        "params": [d],
    });
    write_line(w, &line).await
}

async fn send_notify<W: AsyncWriteExt + Unpin>(w: &mut W, job: &Job) -> std::io::Result<()> {
    let line = serde_json::json!({
        "id": null,
        "method": "mining.notify",
        "params": notify(job),
    });
    write_line(w, &line).await
}

async fn write_line<W: AsyncWriteExt + Unpin>(w: &mut W, v: &Value) -> std::io::Result<()> {
    w.write_all(v.to_string().as_bytes()).await?;
    w.write_all(b"\n").await
}

fn rand_channel_id() -> u32 {
    use rand::RngCore;
    let mut b = [0u8; 4];
    rand::thread_rng().fill_bytes(&mut b);
    u32::from_le_bytes(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
    use tokio::io::AsyncBufReadExt;

    fn address() -> String {
        let secp = Secp256k1::new();
        let a = SecretKey::from_slice(&[3u8; 32]).unwrap();
        let b = SecretKey::from_slice(&[4u8; 32]).unwrap();
        spc_crypto::address::encode_sp_address(
            Network::Testnet,
            &PublicKey::from_secret_key(&secp, &a),
            &PublicKey::from_secret_key(&secp, &b),
        )
        .unwrap()
    }

    fn job() -> Job {
        Job {
            job_id: "00000001".into(),
            prev_hash: [0x11; 32],
            height: 800_000,
            version: 0x2000_0000,
            nbits: 0x1d00_ffff,
            ntime: 1_700_000_000,
            coinb1: vec![0x01, 0x00, 0x00, 0x00],
            coinb2: vec![0xff; 8],
            merkle_branches: vec![],
            clean_jobs: true,
        }
    }

    #[tokio::test]
    async fn sv1_end_to_end_over_tcp() {
        let (tx, mut rx) = mpsc::channel(8);
        let server = Sv1Server::new(Network::Testnet, 4, 4, 1.0, tx);
        let (addr, _h) = server.bind("127.0.0.1:0").await.unwrap();

        let stream = TcpStream::connect(addr).await.unwrap();
        let (read, mut write) = stream.into_split();
        let mut lines = BufReader::new(read).lines();

        write
            .write_all(br#"{"id":1,"method":"mining.subscribe","params":[]}"#)
            .await
            .unwrap();
        write.write_all(b"\n").await.unwrap();
        let line = lines.next_line().await.unwrap().unwrap();
        assert!(line.contains("aaaaaaaa") || line.contains("\"result\""));

        let auth = serde_json::json!({
            "id": 2, "method": "mining.authorize", "params": [address(), "x"]
        });
        write.write_all(auth.to_string().as_bytes()).await.unwrap();
        write.write_all(b"\n").await.unwrap();
        let line = lines.next_line().await.unwrap().unwrap();
        assert!(line.contains("true"), "{line}");

        // set_difficulty arrives after authorize.
        let line = lines.next_line().await.unwrap().unwrap();
        assert!(line.contains("mining.set_difficulty"), "{line}");

        server.publish(job()).await;
        let line = lines.next_line().await.unwrap().unwrap();
        assert!(line.contains("mining.notify"), "{line}");

        let submit = serde_json::json!({
            "id": 3, "method": "mining.submit",
            "params": ["w", "00000001", "00000000", "6553f100", "00000000"]
        });
        write
            .write_all(submit.to_string().as_bytes())
            .await
            .unwrap();
        write.write_all(b"\n").await.unwrap();
        let line = lines.next_line().await.unwrap().unwrap();
        assert!(line.contains("true"), "{line}");

        let share = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(share.extranonce.len(), 8);
    }
}
