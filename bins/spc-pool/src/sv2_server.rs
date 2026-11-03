//! Native SV2 listener: Noise handshake, SetupConnection, extended channel
//! open, jobs and extended shares. The identity rule is the same one the SV1
//! listener uses (SPEC §3.1, decision 2026-11-02).

use std::net::SocketAddr;
use std::sync::Arc;

use bitcoin::hashes::{sha256d, Hash};
use bitcoin::pow::Target;
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
    SetNewPrevHash, SubmitSharesError, SubmitSharesExtended, SubmitSharesSuccess,
    CHANNEL_BIT_MINING_SET_NEW_PREV_HASH, CHANNEL_BIT_NEW_EXTENDED_MINING_JOB,
    CHANNEL_BIT_SUBMIT_SHARES_ERROR, CHANNEL_BIT_SUBMIT_SHARES_SUCCESS,
    MESSAGE_TYPE_MINING_SET_NEW_PREV_HASH, MESSAGE_TYPE_NEW_EXTENDED_MINING_JOB,
    MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL, MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL_SUCCESS,
    MESSAGE_TYPE_OPEN_MINING_CHANNEL_ERROR, MESSAGE_TYPE_SUBMIT_SHARES_ERROR,
    MESSAGE_TYPE_SUBMIT_SHARES_EXTENDED, MESSAGE_TYPE_SUBMIT_SHARES_SUCCESS,
};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, Mutex};

use crate::identity::{parse_identity, ChannelIdentity};
use crate::sv1::{effective_share_target, ntime_allowed, nullifier, version_allowed, Job, Share};
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
    /// Non-coinbase txids of the published job, for the type-32
    /// `GetTransactionsInJob` response (SPEC §3.7).
    job_txids: Arc<Mutex<Vec<[u8; 32]>>>,
    /// Pool share difficulty: the channel target announced on open and the
    /// difficulty every extended share is validated against (decision
    /// 2026-11-02-share-target-policy).
    share_difficulty: u64,
    /// Total extranonce length the coinbase is built for (SPEC §2.4.1):
    /// SV1 `extranonce1 + extranonce2`, here `extranonce_prefix` +
    /// miner-chosen `extranonce_size`. Every channel must offer exactly
    /// this total (review 2.1).
    expected_extranonce_len: usize,
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
        expected_extranonce_len: usize,
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
            job_txids: Arc::new(Mutex::new(Vec::new())),
            share_difficulty: 1,
            expected_extranonce_len,
        }
    }

    /// Set the pool share difficulty used for the channel target and for share
    /// validation. Zero clamps to 1; use the configured
    /// `[accounting] share_difficulty` (validated `> 0`).
    pub fn with_share_difficulty(mut self, share_difficulty: u64) -> Self {
        self.share_difficulty = share_difficulty.max(1);
        self
    }

    /// The share difficulty this listener announces and enforces.
    pub fn share_difficulty(&self) -> u64 {
        self.share_difficulty
    }

    pub async fn publish(&self, job: Job) {
        *self.current.lock().await = Some(job.clone());
        let _ = self.jobs.send(job);
    }

    /// Publish the non-coinbase txids of the current job for the type-32
    /// `GetTransactionsInJob` response.
    pub async fn set_job_txids(&self, txids: Vec<[u8; 32]>) {
        *self.job_txids.lock().await = txids;
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
        // Per-connection replay guard, cleared whenever the job changes
        // (SPEC §3.4). The front keeps the process-wide set.
        let mut seen: std::collections::HashSet<[u8; 32]> = std::collections::HashSet::new();
        let mut seen_job: Option<String> = None;

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
                            let miner_extranonce_size: u16 = 4;
                            let mut prefix = vec![0u8; 4];
                            use rand::RngCore;
                            rand::rngs::OsRng.fill_bytes(&mut prefix);
                            // SPEC §2.4.1 invariant, made explicit (review
                            // 2.1): every ingress rolls the same total
                            // extranonce length — SV1 4+4, here prefix +
                            // miner-chosen — because the front builds the
                            // coinbase scriptSig for exactly that total. A
                            // channel that cannot offer it must not open.
                            if prefix.len() + usize::from(miner_extranonce_size)
                                != self.expected_extranonce_len
                            {
                                let err = open_channel_error(
                                    msg.request_id,
                                    "min-extranonce-size-too-large",
                                );
                                write_msg(&mut write, err, MESSAGE_TYPE_OPEN_MINING_CHANNEL_ERROR, false).await?;
                                continue;
                            }
                            let ch = Channel {
                                channel_id: 1,
                                identity,
                                extranonce_prefix: prefix.clone(),
                            };
                            // The announced target is the pool share target for
                            // the current template: never harder than the
                            // network target (decision
                            // 2026-11-02-share-target-policy). Before the first
                            // template exists there is no network target to
                            // derive from; announce the easiest possible target
                            // (the submit-time check below is authoritative and
                            // still rejects anything below the pool target).
                            let job = self.current.lock().await.clone();
                            let share_target = match &job {
                                Some(job) => effective_share_target(self.share_difficulty, job.nbits),
                                None => Target::from_be_bytes([0xff; 32]),
                            };
                            let target_bytes = share_target.to_be_bytes();
                            let target = U256::from(&target_bytes);
                            let success = OpenExtendedMiningChannelSuccess {
                                request_id: msg.request_id,
                                channel_id: ch.channel_id,
                                target,
                                extranonce_size: miner_extranonce_size,
                                extranonce_prefix: B032::new(&prefix).map_err(|e| format!("prefix: {e}"))?,
                                group_channel_id: 0,
                            };
                            write_msg(&mut write, success, MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL_SUCCESS, false).await?;
                            if let Some(job) = job {
                                send_job(&mut write, &ch, &job).await?;
                            }
                            channel = Some(ch);
                        }
                        MESSAGE_TYPE_SUBMIT_SHARES_EXTENDED => {
                            let msg: SubmitSharesExtended = binary_sv2::from_bytes(&mut payload)
                                .map_err(|e| format!("submit: {e}"))?;
                            // Every rejection answers with an explicit
                            // `SubmitSharesError`; nothing is silently dropped
                            // (SPEC §3.4, §3.6).
                            let Some(ch) = channel.as_ref() else {
                                send_submit_error(&mut write, msg.channel_id, msg.sequence_number, "invalid-channel-id").await?;
                                continue;
                            };
                            let Some(job) = self.current.lock().await.clone() else {
                                send_submit_error(&mut write, ch.channel_id, msg.sequence_number, "invalid-job-id").await?;
                                continue;
                            };
                            if let Some(code) = submission_error(
                                msg.channel_id,
                                ch.channel_id,
                                &job,
                                msg.job_id,
                                msg.ntime,
                                msg.version,
                            ) {
                                send_submit_error(&mut write, ch.channel_id, msg.sequence_number, code).await?;
                                continue;
                            }
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
                            // Fail closed: only shares meeting the pool share
                            // target for this job reach accounting.
                            let share_target = effective_share_target(self.share_difficulty, job.nbits);
                            if !share_target.is_met_by(BlockHash::from_byte_array(share_hash)) {
                                send_submit_error(&mut write, ch.channel_id, msg.sequence_number, "difficulty-too-low").await?;
                                continue;
                            }
                            // Per-connection replay guard (defence in depth;
                            // the front keeps the process-wide set).
                            if seen_job.as_deref() != Some(job.job_id.as_str()) {
                                seen.clear();
                                seen_job = Some(job.job_id.clone());
                            }
                            let nullifier = nullifier(
                                ch.channel_id,
                                &job.job_id,
                                msg.nonce,
                                msg.ntime,
                                msg.version,
                                &extranonce,
                            );
                            if !seen.insert(nullifier) {
                                send_submit_error(&mut write, ch.channel_id, msg.sequence_number, "duplicate-share").await?;
                                continue;
                            }
                            let share = Share {
                                sp_id: ch.identity.sp_id,
                                b_scan: ch.identity.b_scan,
                                b_spend: ch.identity.b_spend,
                                channel_id: ch.channel_id,
                                job_id: job.job_id.clone(),
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
                    txids: self.job_txids.lock().await.clone(),
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

/// Cheap pre-hash validation of an extended submission (SPEC §3.4, §3.6).
///
/// Returns the SV2 error code to send, or `None` when the submission is
/// well-formed for `job` on `expected_channel`. Never silently ignored: an
/// unknown or stale job, a rolled version outside the BIP 320 mask and an
/// `ntime` outside the job's roll window are all explicit rejections.
fn submission_error(
    channel_id: u32,
    expected_channel: u32,
    job: &Job,
    job_id: u32,
    ntime: u32,
    version: u32,
) -> Option<&'static str> {
    if channel_id != expected_channel {
        return Some("invalid-channel-id");
    }
    if job_id != parse_job_id(&job.job_id) {
        return Some("stale-share");
    }
    if !ntime_allowed(job.ntime, ntime) {
        return Some("invalid-ntime");
    }
    if !version_allowed(job.version, version) {
        return Some("invalid-version");
    }
    None
}

/// Answer a rejected submission with the SV2 error code from the spec's list
/// (`stale-share`, `difficulty-too-low`, ...) rather than dropping it.
async fn send_submit_error(
    write: &mut stratum_apps::network_helpers::noise_stream::NoiseTcpWriteHalf,
    channel_id: u32,
    sequence_number: u32,
    code: &'static str,
) -> Result<(), String> {
    let Ok(error_code) = Str0255::try_from(code) else {
        return Ok(());
    };
    let err = SubmitSharesError {
        channel_id,
        sequence_number,
        error_code,
    };
    write_msg(
        write,
        err,
        MESSAGE_TYPE_SUBMIT_SHARES_ERROR,
        CHANNEL_BIT_SUBMIT_SHARES_ERROR,
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

/// Target helper for tests (the consensus target for `bits`).
pub fn target_from_bits(bits: u32) -> Target {
    crate::sv1::network_target(bits)
}

pub fn hash_meets_target(hash: [u8; 32], bits: u32) -> bool {
    target_from_bits(bits).is_met_by(BlockHash::from_byte_array(hash))
}

/// The target this listener puts in
/// `OpenExtendedMiningChannelSuccess.target` for `share_difficulty` at the
/// current template's `bits`, as big-endian bytes.
pub fn announced_share_target(share_difficulty: u64, bits: u32) -> [u8; 32] {
    effective_share_target(share_difficulty, bits).to_be_bytes()
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

    #[test]
    fn announced_target_is_the_pool_share_target() {
        // Network difficulty 256: the pool share target is the easier one and
        // is what the channel is opened with.
        let bits = 0x1c00_ffff;
        let announced = announced_share_target(4, bits);
        let expected = crate::sv1::target_for_difficulty(4).to_be_bytes();
        assert_eq!(announced, expected);

        // Regtest bits: the announced target is never harder than the network
        // target, so a regtest miner is not asked for mainnet work.
        let regtest = announced_share_target(u64::MAX, 0x207f_ffff);
        assert_eq!(
            regtest,
            crate::sv1::network_target(0x207f_ffff).to_be_bytes()
        );
        // And it is never the old all-ones placeholder.
        assert_ne!(regtest, [0xffu8; 32]);
    }

    fn job() -> Job {
        Job {
            job_id: "00000005".into(),
            prev_hash: [0x11; 32],
            height: 800_000,
            version: 0x2000_0000,
            nbits: 0x207f_ffff,
            ntime: 1_700_000_000,
            coinb1: vec![0x01, 0x00, 0x00, 0x00],
            coinb2: vec![0xff; 8],
            merkle_branches: vec![],
            clean_jobs: true,
        }
    }

    #[test]
    fn submissions_are_rejected_with_explicit_codes() {
        let j = job();
        // Well-formed for the current job and channel.
        assert_eq!(submission_error(1, 1, &j, 5, j.ntime, j.version), None);
        // Wrong channel.
        assert_eq!(
            submission_error(2, 1, &j, 5, j.ntime, j.version),
            Some("invalid-channel-id")
        );
        // Stale/unknown job id.
        assert_eq!(
            submission_error(1, 1, &j, 6, j.ntime, j.version),
            Some("stale-share")
        );
        // ntime far before the job (beyond the +/- roll window).
        assert_eq!(
            submission_error(
                1,
                1,
                &j,
                5,
                j.ntime - crate::sv1::MAX_NTIME_ROLL_SECS - 1,
                j.version
            ),
            Some("invalid-ntime")
        );
        // A job that predates by a few seconds is still honest work.
        assert_eq!(submission_error(1, 1, &j, 5, j.ntime - 5, j.version), None);
        assert_eq!(
            submission_error(
                1,
                1,
                &j,
                5,
                j.ntime + crate::sv1::MAX_NTIME_ROLL_SECS + 1,
                j.version
            ),
            Some("invalid-ntime")
        );
        // Rolled bits outside the BIP 320 mask.
        assert_eq!(
            submission_error(1, 1, &j, 5, j.ntime, j.version | 0x0000_0001),
            Some("invalid-version")
        );
        // A legal roll passes.
        assert_eq!(
            submission_error(1, 1, &j, 5, j.ntime + 30, j.version ^ 0x0000_2000),
            None
        );
    }

    #[test]
    fn per_connection_replay_set_rejects_a_second_identical_share() {
        let j = job();
        let mut seen: std::collections::HashSet<[u8; 32]> = std::collections::HashSet::new();
        let key = |nonce: u32| nullifier(1, &j.job_id, nonce, j.ntime, j.version, &[0xaa; 8]);
        assert!(seen.insert(key(7)));
        assert!(!seen.insert(key(7)), "the same share must not replay");
        assert!(seen.insert(key(8)));
    }

    #[test]
    fn share_difficulty_builder_clamps_to_one() {
        let (tx, _rx) = mpsc::channel(1);
        let server = Sv2Server::new(
            spc_crypto::Network::Regtest,
            tx,
            std::sync::Arc::new(std::sync::Mutex::new(Type32Store::default())),
            8,
        );
        assert_eq!(server.share_difficulty(), 1);
        let server = server.with_share_difficulty(0);
        assert_eq!(server.share_difficulty(), 1);
        let server = server.with_share_difficulty(4_096);
        assert_eq!(server.share_difficulty(), 4_096);
    }
}
