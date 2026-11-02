//! Minimal JDS listener: publishes `SetPayoutDistribution` and accepts a
//! declaration whose `distribution_id` is current or the same-height previous
//! id. Full payout recomputation is in [`crate::jds::Jds::validate`].

use std::net::SocketAddr;
use std::sync::Arc;

use spc_ext::{DistributionIdTlv, SetPayoutDistribution};
use stratum_apps::key_utils::{Secp256k1PublicKey, Secp256k1SecretKey};
use stratum_core::binary_sv2::{self, B0255, B064K};
use stratum_core::codec_sv2::SerializedFrame;
use stratum_core::common_messages_sv2::{
    SetupConnection, SetupConnectionSuccess, MESSAGE_TYPE_SETUP_CONNECTION,
    MESSAGE_TYPE_SETUP_CONNECTION_SUCCESS,
};
use stratum_core::job_declaration_sv2::{
    AllocateMiningJobToken, AllocateMiningJobTokenSuccess, DeclareMiningJob,
    MESSAGE_TYPE_ALLOCATE_MINING_JOB_TOKEN, MESSAGE_TYPE_ALLOCATE_MINING_JOB_TOKEN_SUCCESS,
    MESSAGE_TYPE_DECLARE_MINING_JOB,
};
use stratum_core::mining_sv2::MESSAGE_TYPE_SET_CUSTOM_MINING_JOB;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, Mutex};

use crate::jds::Jds;

pub const EXT_PAYOUTS: u16 = 0x0003;

pub struct JdsServer {
    jds: Arc<Mutex<Jds>>,
    current: Arc<Mutex<Option<Vec<u8>>>>,
    height: Arc<Mutex<u32>>,
    accepted: Arc<Mutex<Option<u64>>>,
    dist_tx: broadcast::Sender<Vec<u8>>,
    pk: Secp256k1PublicKey,
    sk: Secp256k1SecretKey,
}

impl JdsServer {
    pub fn new(front_id: u8) -> Self {
        let secp = secp256k1_legacy::Secp256k1::new();
        let mut rng = rand::thread_rng();
        let kp = secp256k1_legacy::Keypair::new(&secp, &mut rng);
        let (dist_tx, _) = broadcast::channel(16);
        Self {
            jds: Arc::new(Mutex::new(Jds::new(front_id))),
            current: Arc::new(Mutex::new(None)),
            height: Arc::new(Mutex::new(0)),
            accepted: Arc::new(Mutex::new(None)),
            dist_tx,
            pk: Secp256k1PublicKey(kp.x_only_public_key().0),
            sk: Secp256k1SecretKey(kp.secret_key()),
        }
    }

    pub async fn publish(
        &self,
        height: u32,
        distribution: SetPayoutDistribution,
    ) -> Result<u64, String> {
        let mut jds = self.jds.lock().await;
        let id = jds.publish(height, distribution);
        let current = jds.current().cloned().ok_or("no current distribution")?;
        drop(jds);
        let payload = current.encode().map_err(|e| e.to_string())?;
        *self.current.lock().await = Some(payload.clone());
        *self.height.lock().await = height;
        let _ = self.dist_tx.send(payload);
        Ok(id)
    }

    pub async fn accepted(&self) -> Option<u64> {
        *self.accepted.lock().await
    }

    pub async fn bind(
        &self,
        addr: &str,
    ) -> std::io::Result<(SocketAddr, tokio::task::JoinHandle<()>)> {
        let listener = TcpListener::bind(addr).await?;
        let local = listener.local_addr()?;
        let server = Arc::new(JdsServerRef {
            jds: self.jds.clone(),
            current: self.current.clone(),
            height: self.height.clone(),
            accepted: self.accepted.clone(),
            dist_tx: self.dist_tx.clone(),
            pk: self.pk,
            sk: self.sk,
            token_seq: Arc::new(Mutex::new(0)),
        });
        let handle = tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let server = server.clone();
                tokio::spawn(async move {
                    let _ = server.handle(socket).await;
                });
            }
        });
        Ok((local, handle))
    }
}

struct JdsServerRef {
    jds: Arc<Mutex<Jds>>,
    current: Arc<Mutex<Option<Vec<u8>>>>,
    height: Arc<Mutex<u32>>,
    accepted: Arc<Mutex<Option<u64>>>,
    dist_tx: broadcast::Sender<Vec<u8>>,
    pk: Secp256k1PublicKey,
    sk: Secp256k1SecretKey,
    token_seq: Arc<Mutex<u64>>,
}

impl JdsServerRef {
    async fn handle(&self, socket: TcpStream) -> Result<(), String> {
        let stream =
            stratum_apps::network_helpers::accept_noise_connection(socket, self.pk, self.sk, 3600)
                .await
                .map_err(|e| format!("noise: {e}"))?;
        let (mut read, mut write) = stream.into_split();
        let mut jobs = self.dist_tx.subscribe();

        loop {
            tokio::select! {
                frame = read.read_frame() => {
                    let mut frame = frame.map_err(|e| format!("read: {e}"))?;
                    let msg_type = frame.header().msg_type();
                    let ext = frame.header().ext_type_without_channel_msg();
                    let mut payload = frame.payload().to_vec();
                    match (ext, msg_type) {
                        (0, MESSAGE_TYPE_SETUP_CONNECTION) => {
                            let _: SetupConnection = binary_sv2::from_bytes(&mut payload)
                                .map_err(|e| format!("setup: {e}"))?;
                            write_msg(&mut write, SetupConnectionSuccess { used_version: 2, flags: 0 }, MESSAGE_TYPE_SETUP_CONNECTION_SUCCESS).await?;
                            if let Some(p) = self.current.lock().await.clone() {
                                write_dist(&mut write, &p).await?;
                            }
                        }
                        (EXT_PAYOUTS, 0) => { /* client must not publish */ }
                        (_, MESSAGE_TYPE_ALLOCATE_MINING_JOB_TOKEN) => {
                            let msg: AllocateMiningJobToken = binary_sv2::from_bytes(&mut payload)
                                .map_err(|e| format!("allocate: {e}"))?;
                            let mut seq = self.token_seq.lock().await;
                            *seq = seq.wrapping_add(1);
                            let token = format!("spc-token-{}", *seq);
                            drop(seq);
                            let success = AllocateMiningJobTokenSuccess {
                                request_id: msg.request_id,
                                mining_job_token: B0255::new(token.as_bytes())
                                    .map_err(|e| format!("token: {e}"))?,
                                coinbase_outputs: B064K::new(&[])
                                    .map_err(|e| format!("outputs: {e}"))?,
                            };
                            write_msg(
                                &mut write,
                                success,
                                MESSAGE_TYPE_ALLOCATE_MINING_JOB_TOKEN_SUCCESS,
                            )
                            .await?;
                        }
                        (_, MESSAGE_TYPE_DECLARE_MINING_JOB) => {
                            let msg: DeclareMiningJob = binary_sv2::from_bytes(&mut payload)
                                .map_err(|e| format!("declare: {e}"))?;
                            if let Some(id) = find_tlv(msg.excess_data.as_bytes()) {
                                if self.jds.lock().await.accepts_id(id) {
                                    *self.accepted.lock().await = Some(id);
                                }
                            }
                            let _ = self.height.lock().await;
                        }
                        (_, MESSAGE_TYPE_SET_CUSTOM_MINING_JOB) => {
                            if let Some(id) = find_tlv(&payload) {
                                if self.jds.lock().await.accepts_id(id) {
                                    *self.accepted.lock().await = Some(id);
                                }
                            }
                        }
                        _ => {}
                    }
                }
                dist = jobs.recv() => {
                    match dist {
                        Ok(p) => write_dist(&mut write, &p).await?,
                        Err(broadcast::error::RecvError::Lagged(_)) => {}
                        Err(_) => break,
                    }
                }
            }
        }
        Ok(())
    }
}

fn find_tlv(bytes: &[u8]) -> Option<u64> {
    if bytes.len() == 13 {
        return DistributionIdTlv::decode(bytes).ok().map(|t| t.id);
    }
    // The TLV may be appended to a larger excess-data field.
    bytes
        .windows(13)
        .rev()
        .find_map(|w| DistributionIdTlv::decode(w).ok().map(|t| t.id))
}

async fn write_msg<T>(
    write: &mut stratum_apps::network_helpers::noise_stream::NoiseTcpWriteHalf,
    msg: T,
    msg_type: u8,
) -> Result<(), String>
where
    T: binary_sv2::Serialize + binary_sv2::GetSize,
{
    let frame =
        stratum_core::framing_sv2::framing::MessageFrame::from_message(msg, msg_type, 0, false)
            .map_err(|e| format!("frame: {e}"))?;
    write
        .write_frame(frame)
        .await
        .map_err(|e| format!("write: {e}"))
}

async fn write_dist(
    write: &mut stratum_apps::network_helpers::noise_stream::NoiseTcpWriteHalf,
    payload: &[u8],
) -> Result<(), String> {
    let mut bytes = Vec::with_capacity(6 + payload.len());
    bytes.extend_from_slice(&EXT_PAYOUTS.to_le_bytes());
    bytes.push(0);
    let len = u32::try_from(payload.len()).map_err(|_| "too long")?;
    bytes.extend_from_slice(&len.to_le_bytes()[..3]);
    bytes.extend_from_slice(payload);
    let frame = SerializedFrame::from_bytes(bytes).map_err(|e| format!("frame: {e:?}"))?;
    write
        .write_frame(frame)
        .await
        .map_err(|e| format!("write: {e}"))
}
