//! Native SV2 listener: Noise handshake, SetupConnection, extended channel
//! open, jobs and extended shares. The identity rule is the same one the SV1
//! listener uses (SPEC §3.1, decision 2026-11-02).

use std::net::SocketAddr;
use std::sync::Arc;

use bitcoin::hashes::{sha256d, Hash};
use bitcoin::pow::{CompactTarget, Target};
use bitcoin::BlockHash;
use stratum_apps::key_utils::{Secp256k1PublicKey, Secp256k1SecretKey};
use stratum_core::binary_sv2::{self, Seq0255, Str0255, Sv2Option, B032, B064K, U256};
use stratum_core::common_messages_sv2::{
    SetupConnection, SetupConnectionSuccess, MESSAGE_TYPE_SETUP_CONNECTION,
    MESSAGE_TYPE_SETUP_CONNECTION_SUCCESS,
};
use stratum_core::framing_sv2::framing::MessageFrame;
use stratum_core::mining_sv2::NewExtendedMiningJob;
use stratum_core::mining_sv2::{
    OpenExtendedMiningChannel, OpenExtendedMiningChannelSuccess, OpenMiningChannelError,
    SetNewPrevHash, SubmitSharesExtended, SubmitSharesSuccess,
    CHANNEL_BIT_MINING_SET_NEW_PREV_HASH, CHANNEL_BIT_NEW_EXTENDED_MINING_JOB,
    CHANNEL_BIT_SUBMIT_SHARES_SUCCESS, MESSAGE_TYPE_MINING_SET_NEW_PREV_HASH,
    MESSAGE_TYPE_NEW_EXTENDED_MINING_JOB, MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL,
    MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL_SUCCESS, MESSAGE_TYPE_OPEN_MINING_CHANNEL_ERROR,
    MESSAGE_TYPE_SUBMIT_SHARES_EXTENDED, MESSAGE_TYPE_SUBMIT_SHARES_SUCCESS,
};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, Mutex};

use crate::identity::{parse_identity, ChannelIdentity};
use crate::sv1::{Job, Share};
use crate::type32::Type32Store;
use spc_share_ext::{Message as Type32Message, EXTENSION_TYPE};

#[derive(Clone)]
pub struct Sv2Server {
    network: spc_crypto::Network,
    authority_pk: Secp256k1PublicKey,
    authority_sk: Secp256k1SecretKey,
    current: Arc<Mutex<Option<Job>>>,
    jobs: broadcast::Sender<Job>,
    shares: mpsc::Sender<Share>,
    type32: Arc<std::sync::Mutex<Type32Store>>,
}

struct Channel {
    channel_id: u32,
    identity: ChannelIdentity,
    extranonce_prefix: Vec<u8>,
}

impl Sv2Server {
    pub fn new(
        network: spc_crypto::Network,
        shares: mpsc::Sender<Share>,
        type32: Arc<std::sync::Mutex<Type32Store>>,
    ) -> Self {
        let secp = secp256k1_legacy::Secp256k1::new();
        let mut rng = rand::thread_rng();
        let kp = secp256k1_legacy::Keypair::new(&secp, &mut rng);
        let pk = Secp256k1PublicKey(kp.x_only_public_key().0);
        let sk = Secp256k1SecretKey(kp.secret_key());
        let (jobs, _) = broadcast::channel(16);
        Self {
            network,
            authority_pk: pk,
            authority_sk: sk,
            current: Arc::new(Mutex::new(None)),
            jobs,
            shares,
            type32,
        }
    }

    pub async fn publish(&self, job: Job) {
        *self.current.lock().await = Some(job.clone());
        let _ = self.jobs.send(job);
    }

    pub async fn bind(
        &self,
        addr: &str,
    ) -> std::io::Result<(SocketAddr, tokio::task::JoinHandle<()>)> {
        let listener = TcpListener::bind(addr).await?;
        let local = listener.local_addr()?;
        let server = self.clone();
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

    async fn handle(&self, socket: TcpStream) -> Result<(), String> {
        let stream = stratum_apps::network_helpers::accept_noise_connection(
            socket,
            self.authority_pk,
            self.authority_sk,
            3600,
        )
        .await
        .map_err(|e| format!("noise: {e}"))?;
        let (mut read, mut write) = stream.into_split();
        let mut jobs = self.jobs.subscribe();
        let mut channel: Option<Channel> = None;
        let mut setup_done = false;

        loop {
            tokio::select! {
                frame = read.read_frame() => {
                    let mut frame = frame.map_err(|e| format!("read: {e}"))?;
                    let msg_type = frame.header().msg_type();
                    let ext_type = frame.header().ext_type_without_channel_msg();
                    let mut payload = frame.payload().to_vec();
                    if ext_type == EXTENSION_TYPE {
                        self.handle_type32(&mut write, msg_type, &payload).await?;
                        continue;
                    }
                    match msg_type {
                        MESSAGE_TYPE_SETUP_CONNECTION => {
                            let _msg: SetupConnection = binary_sv2::from_bytes(&mut payload)
                                .map_err(|e| format!("setup: {e}"))?;
                            write_msg(&mut write, SetupConnectionSuccess { used_version: 2, flags: 0 }, MESSAGE_TYPE_SETUP_CONNECTION_SUCCESS, false).await?;
                            setup_done = true;
                        }
                        MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL => {
                            if !setup_done {
                                return Err("open before setup".into());
                            }
                            let msg: OpenExtendedMiningChannel = binary_sv2::from_bytes(&mut payload)
                                .map_err(|e| format!("open: {e}"))?;
                            let raw = String::from_utf8_lossy(msg.user_identity.as_bytes()).to_string();
                            let identity = match parse_identity(&raw, self.network()) {
                                Ok(id) => id,
                                Err(_) => {
                                    let err = open_channel_error(
                                        msg.request_id,
                                        "invalid-user-identity",
                                    );
                                    write_msg(&mut write, err, MESSAGE_TYPE_OPEN_MINING_CHANNEL_ERROR, false).await?;
                                    continue;
                                }
                            };
                            // One connection is one identity (SPEC §3.1): the
                            // first accepted open fixes the connection's
                            // `sp_id`; a later open presenting a different
                            // address is rejected without creating a channel.
                            let conn_sp_id = channel.as_ref().map(|c| c.identity.sp_id);
                            if identity_conflicts(conn_sp_id, identity.sp_id) {
                                let err = open_channel_error(
                                    msg.request_id,
                                    "invalid-user-identity",
                                );
                                write_msg(&mut write, err, MESSAGE_TYPE_OPEN_MINING_CHANNEL_ERROR, false).await?;
                                continue;
                            }
                            let mut prefix = vec![0u8; 4];
                            use rand::RngCore;
                            rand::rngs::OsRng.fill_bytes(&mut prefix);
                            let ch = Channel {
                                channel_id: 1,
                                identity,
                                extranonce_prefix: prefix.clone(),
                            };
                            let target_bytes = [0xffu8; 32];
                            let target = U256::from(&target_bytes);
                            let success = OpenExtendedMiningChannelSuccess {
                                request_id: msg.request_id,
                                channel_id: ch.channel_id,
                                target,
                                extranonce_size: 4,
                                extranonce_prefix: B032::new(&prefix).map_err(|e| format!("prefix: {e}"))?,
                                group_channel_id: 0,
                            };
                            write_msg(&mut write, success, MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL_SUCCESS, false).await?;
                            if let Some(job) = self.current.lock().await.clone() {
                                send_job(&mut write, &ch, &job).await?;
                            }
                            channel = Some(ch);
                        }
                        MESSAGE_TYPE_SUBMIT_SHARES_EXTENDED => {
                            let msg: SubmitSharesExtended = binary_sv2::from_bytes(&mut payload)
                                .map_err(|e| format!("submit: {e}"))?;
                            let Some(ch) = &channel else { continue };
                            if msg.channel_id != ch.channel_id || msg.job_id != self.current_job_id().await {
                                continue;
                            }
                            let job = match self.current.lock().await.clone() {
                                Some(j) => j,
                                None => continue,
                            };
                            let mut extranonce = ch.extranonce_prefix.clone();
                            extranonce.extend_from_slice(msg.extranonce.as_bytes());
                            let mut coinbase = job.coinb1.clone();
                            coinbase.extend_from_slice(&extranonce);
                            coinbase.extend_from_slice(&job.coinb2);
                            let mut merkle_root = sha256d::Hash::hash(&coinbase).to_byte_array();
                            for branch in job.merkle_branches.iter().rev() {
                                let mut buf = Vec::with_capacity(64);
                                buf.extend_from_slice(&merkle_root);
                                buf.extend_from_slice(branch);
                                merkle_root = sha256d::Hash::hash(&buf).to_byte_array();
                            }
                            let mut header = Vec::with_capacity(80);
                            header.extend_from_slice(&msg.version.to_le_bytes());
                            header.extend_from_slice(&job.prev_hash);
                            header.extend_from_slice(&merkle_root);
                            header.extend_from_slice(&msg.ntime.to_le_bytes());
                            header.extend_from_slice(&job.nbits.to_le_bytes());
                            header.extend_from_slice(&msg.nonce.to_le_bytes());
                            let share_hash = sha256d::Hash::hash(&header).to_byte_array();
                            let share = Share {
                                sp_id: ch.identity.sp_id,
                                b_scan: ch.identity.b_scan,
                                b_spend: ch.identity.b_spend,
                                job_id: format!("{:08x}", msg.job_id),
                                nonce: msg.nonce,
                                ntime: msg.ntime,
                                version: msg.version,
                                extranonce,
                                share_index: msg.sequence_number,
                                job_fees: 0,
                                coinbase,
                                merkle_root,
                                share_hash,
                            };
                            let _ = self.shares.send(share).await;
                            let ok = SubmitSharesSuccess {
                                channel_id: ch.channel_id,
                                last_sequence_number: msg.sequence_number,
                                new_submits_accepted_count: 1,
                                new_shares_sum: 1,
                            };
                            write_msg(&mut write, ok, MESSAGE_TYPE_SUBMIT_SHARES_SUCCESS, CHANNEL_BIT_SUBMIT_SHARES_SUCCESS).await?;
                        }
                        _ => {}
                    }
                }
                job = jobs.recv() => {
                    match job {
                        Ok(job) => {
                            if let Some(ch) = &channel {
                                send_job(&mut write, ch, &job).await?;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {}
                        Err(_) => break,
                    }
                }
            }
        }
        Ok(())
    }

    fn network(&self) -> spc_crypto::Network {
        self.network
    }

    async fn handle_type32(
        &self,
        write: &mut stratum_apps::network_helpers::noise_stream::NoiseTcpWriteHalf,
        msg_type: u8,
        payload: &[u8],
    ) -> Result<(), String> {
        let message = match Type32Message::decode(msg_type, payload) {
            Ok(m) => m,
            Err(_) => {
                let err = Type32Message::Error {
                    request_id: 0,
                    code: "invalid-message".into(),
                };
                return write_type32(write, &err).await;
            }
        };
        let response = match message {
            Type32Message::GetWindow { request_id, .. } => {
                let slices = self.type32.lock().map(|s| s.window()).unwrap_or_default();
                Type32Message::GetWindowSuccess { request_id, slices }
            }
            Type32Message::GetShares {
                request_id,
                slice_id,
                indices,
            } => {
                let shares = self
                    .type32
                    .lock()
                    .map(|s| s.shares(slice_id, &indices))
                    .unwrap_or_default();
                Type32Message::GetSharesSuccess { request_id, shares }
            }
            Type32Message::GetTransactionsInJob { request_id, .. } => {
                Type32Message::GetTransactionsInJobSuccess {
                    request_id,
                    txids: Vec::new(),
                }
            }
            Type32Message::NewBlockFound { .. } => return Ok(()),
            other => Type32Message::Error {
                request_id: 0,
                code: format!("unexpected message {}", other.message_type()),
            },
        };
        write_type32(write, &response).await
    }

    async fn current_job_id(&self) -> u32 {
        self.current
            .lock()
            .await
            .as_ref()
            .map(|j| parse_job_id(&j.job_id))
            .unwrap_or(0)
    }
}

async fn send_job(
    write: &mut stratum_apps::network_helpers::noise_stream::NoiseTcpWriteHalf,
    ch: &Channel,
    job: &Job,
) -> Result<(), String> {
    let prev = SetNewPrevHash {
        channel_id: ch.channel_id,
        job_id: parse_job_id(&job.job_id),
        prev_hash: U256::from(&job.prev_hash),
        min_ntime: job.ntime,
        nbits: job.nbits,
    };
    write_msg(
        write,
        prev,
        MESSAGE_TYPE_MINING_SET_NEW_PREV_HASH,
        CHANNEL_BIT_MINING_SET_NEW_PREV_HASH,
    )
    .await?;

    let merkle_path: Vec<U256> = job.merkle_branches.iter().map(U256::from).collect();
    let job_msg = NewExtendedMiningJob {
        channel_id: ch.channel_id,
        job_id: parse_job_id(&job.job_id),
        min_ntime: Sv2Option::new(Some(job.ntime)),
        version: job.version,
        version_rolling_allowed: true,
        merkle_path: Seq0255::try_from(merkle_path).map_err(|e| format!("merkle path: {e}"))?,
        coinbase_tx_prefix: B064K::new(&job.coinb1).map_err(|e| format!("prefix: {e}"))?,
        coinbase_tx_suffix: B064K::new(&job.coinb2).map_err(|e| format!("suffix: {e}"))?,
    };
    write_msg(
        write,
        job_msg,
        MESSAGE_TYPE_NEW_EXTENDED_MINING_JOB,
        CHANNEL_BIT_NEW_EXTENDED_MINING_JOB,
    )
    .await
}

async fn write_msg<T>(
    write: &mut stratum_apps::network_helpers::noise_stream::NoiseTcpWriteHalf,
    msg: T,
    msg_type: u8,
    channel_msg: bool,
) -> Result<(), String>
where
    T: stratum_core::binary_sv2::Serialize + stratum_core::binary_sv2::GetSize,
{
    let frame = MessageFrame::from_message(msg, msg_type, 0, channel_msg)
        .map_err(|e| format!("frame: {e}"))?;
    write
        .write_frame(frame)
        .await
        .map_err(|e| format!("write: {e}"))
}

async fn write_type32(
    write: &mut stratum_apps::network_helpers::noise_stream::NoiseTcpWriteHalf,
    msg: &Type32Message,
) -> Result<(), String> {
    let payload = msg.encode().map_err(|e| format!("type32 encode: {e}"))?;
    let mut bytes = Vec::with_capacity(6 + payload.len());
    bytes.extend_from_slice(&EXTENSION_TYPE.to_le_bytes());
    bytes.push(msg.message_type());
    let len = u32::try_from(payload.len()).map_err(|_| "type32 too long")?;
    bytes.extend_from_slice(&len.to_le_bytes()[..3]);
    bytes.extend_from_slice(&payload);
    let frame = stratum_core::codec_sv2::SerializedFrame::from_bytes(bytes)
        .map_err(|e| format!("type32 frame: {e:?}"))?;
    write
        .write_frame(frame)
        .await
        .map_err(|e| format!("type32 write: {e}"))
}

fn parse_job_id(s: &str) -> u32 {
    u32::from_str_radix(s, 16).unwrap_or(0)
}

/// Build an `OpenMiningChannelError` with a static error code.
fn open_channel_error(request_id: u32, code: &str) -> OpenMiningChannelError<'_> {
    OpenMiningChannelError {
        request_id,
        error_code: Str0255::try_from(code).expect("static error code fits Str0255"),
    }
}

/// SPEC §3.1: extended channels on one connection must present the same
/// address. `conn` is the connection's first accepted `sp_id` (if any); a
/// candidate that differs from it conflicts and must be answered with
/// `invalid-user-identity` without creating a channel. A `.worker` suffix
/// does not change `sp_id`, so re-opening with a new worker name is allowed.
fn identity_conflicts(conn: Option<[u8; 32]>, candidate: [u8; 32]) -> bool {
    conn.is_some_and(|existing| existing != candidate)
}

/// Target helper for tests.
pub fn target_from_bits(bits: u32) -> Target {
    Target::from_compact(CompactTarget::from_consensus(bits))
}

pub fn hash_meets_target(hash: [u8; 32], bits: u32) -> bool {
    target_from_bits(bits).is_met_by(BlockHash::from_byte_array(hash))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::parse_identity;
    use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};

    fn addr(seed: u8) -> String {
        let secp = Secp256k1::new();
        let scan = SecretKey::from_slice(&[seed; 32]).unwrap();
        let spend = SecretKey::from_slice(&[seed.wrapping_add(1); 32]).unwrap();
        spc_crypto::address::encode_sp_address(
            spc_crypto::Network::Testnet,
            &PublicKey::from_secret_key(&secp, &scan),
            &PublicKey::from_secret_key(&secp, &spend),
        )
        .unwrap()
    }

    fn sp_id(raw: &str) -> [u8; 32] {
        parse_identity(raw, spc_crypto::Network::Testnet)
            .unwrap()
            .sp_id
    }

    #[test]
    fn first_open_never_conflicts() {
        let a = sp_id(&addr(3));
        assert!(!identity_conflicts(None, a));
    }

    #[test]
    fn same_identity_does_not_conflict() {
        let a = sp_id(&addr(3));
        assert!(!identity_conflicts(Some(a), a));
    }

    #[test]
    fn different_identity_conflicts() {
        let a = sp_id(&addr(3));
        let b = sp_id(&addr(4));
        assert!(identity_conflicts(Some(a), b));
        assert!(identity_conflicts(Some(b), a));
    }

    #[test]
    fn worker_suffix_keeps_the_connection_identity() {
        let base = addr(3);
        let plain = sp_id(&base);
        let suffixed = sp_id(&format!("{base}.rig.1"));
        assert_eq!(plain, suffixed);
        assert!(!identity_conflicts(Some(plain), suffixed));
    }

    #[test]
    fn channel_error_code_is_invalid_user_identity() {
        let err = open_channel_error(7, "invalid-user-identity");
        assert_eq!(err.request_id, 7);
        assert_eq!(err.error_code.as_bytes(), b"invalid-user-identity");
    }
}
