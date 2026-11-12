//! Native SV2 listener: Noise handshake, SetupConnection, extended channel
//! open, jobs and extended shares. The identity rule is the same one the SV1
//! listener uses (SPEC §3.1, decision 2026-11-02).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use bitcoin::hashes::{sha256d, Hash};
use bitcoin::pow::Target;
use bitcoin::BlockHash;
use stratum_apps::key_utils::{Secp256k1PublicKey, Secp256k1SecretKey};
use stratum_core::binary_sv2::{self, Seq0255, Str0255, Sv2Option, B032, B064K, U256};
use stratum_core::common_messages_sv2::{
    SetupConnection, SetupConnectionSuccess, MESSAGE_TYPE_SETUP_CONNECTION,
    MESSAGE_TYPE_SETUP_CONNECTION_SUCCESS,
};
use stratum_core::extensions_sv2::{
    RequestExtensions, RequestExtensionsSuccess, EXTENSION_TYPE_EXTENSIONS_NEGOTIATION,
    MESSAGE_TYPE_REQUEST_EXTENSIONS, MESSAGE_TYPE_REQUEST_EXTENSIONS_SUCCESS,
};
use stratum_core::framing_sv2::framing::MessageFrame;
use stratum_core::mining_sv2::NewExtendedMiningJob;
use stratum_core::mining_sv2::{
    OpenExtendedMiningChannel, OpenExtendedMiningChannelSuccess, OpenMiningChannelError,
    SetNewPrevHash, SetTarget, SubmitSharesError, SubmitSharesExtended, SubmitSharesSuccess,
    CHANNEL_BIT_MINING_SET_NEW_PREV_HASH, CHANNEL_BIT_NEW_EXTENDED_MINING_JOB,
    CHANNEL_BIT_SET_TARGET, CHANNEL_BIT_SUBMIT_SHARES_ERROR, CHANNEL_BIT_SUBMIT_SHARES_SUCCESS,
    MESSAGE_TYPE_MINING_SET_NEW_PREV_HASH, MESSAGE_TYPE_NEW_EXTENDED_MINING_JOB,
    MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL, MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL_SUCCESS,
    MESSAGE_TYPE_OPEN_MINING_CHANNEL_ERROR, MESSAGE_TYPE_SET_TARGET,
    MESSAGE_TYPE_SUBMIT_SHARES_ERROR, MESSAGE_TYPE_SUBMIT_SHARES_EXTENDED,
    MESSAGE_TYPE_SUBMIT_SHARES_SUCCESS,
};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, oneshot, Mutex};

use crate::identity::{parse_identity, ChannelIdentity};
use crate::registration::ArkRegistrationHandler;
use crate::sv1::{ntime_allowed, nullifier, version_allowed, Job, Share};
use crate::type32::Type32Store;
use spc_ext::EXT_DELIVERY;
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
    type32_enabled: bool,
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
    front_id: u8,
    next_channel_id: Arc<AtomicU32>,
    next_extranonce_prefix: Arc<AtomicU32>,
    outcomes: ShareOutcomeHub,
    registration: Option<Arc<dyn ArkRegistrationHandler>>,
}

/// Final front-side outcome for one ingress submission. A success is sent only
/// after `Front::on_share` has accepted the share into its slice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShareOutcome {
    Accepted { credited_work: u64 },
    Rejected { error_code: String },
}

#[derive(Clone, Default)]
struct ShareOutcomeHub {
    pending: Arc<StdMutex<PendingOutcomes>>,
}

type PendingOutcomes = HashMap<(u32, u32), oneshot::Sender<ShareOutcome>>;

impl ShareOutcomeHub {
    fn register(&self, channel_id: u32, sequence: u32) -> oneshot::Receiver<ShareOutcome> {
        let (tx, rx) = oneshot::channel();
        let previous = self
            .pending
            .lock()
            .expect("share outcome registry poisoned")
            .insert((channel_id, sequence), tx);
        assert!(previous.is_none(), "duplicate live SV2 sequence number");
        rx
    }

    fn complete(&self, channel_id: u32, sequence: u32, outcome: ShareOutcome) {
        if let Some(tx) = self
            .pending
            .lock()
            .expect("share outcome registry poisoned")
            .remove(&(channel_id, sequence))
        {
            let _ = tx.send(outcome);
        }
    }

    fn abandon(&self, channel_id: u32, sequence: u32) {
        self.complete(
            channel_id,
            sequence,
            ShareOutcome::Rejected {
                error_code: "invalid-share".into(),
            },
        );
    }
}

struct Channel {
    front_id: u8,
    channel_id: u32,
    identity: ChannelIdentity,
    extranonce_prefix: Vec<u8>,
    extranonce_size: u16,
    last_target: Target,
    prev_hash: Option<[u8; 32]>,
    nbits: Option<u32>,
}

impl Channel {
    fn front_id(&self) -> u8 {
        self.front_id
    }
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
            // Existing library users are legacy v1 unless they explicitly
            // disable the extension. Production always passes the config flag.
            type32_enabled: true,
            job_txids: Arc::new(Mutex::new(Vec::new())),
            share_difficulty: 1,
            expected_extranonce_len,
            front_id: 0,
            next_channel_id: Arc::new(AtomicU32::new(1)),
            next_extranonce_prefix: Arc::new(AtomicU32::new(0)),
            outcomes: ShareOutcomeHub::default(),
            registration: None,
        }
    }

    /// Install the real `0x5350` handler. Without it negotiation omits the
    /// extension and direct frames fail closed.
    pub fn with_ark_registration(mut self, handler: Arc<dyn ArkRegistrationHandler>) -> Self {
        self.registration = Some(handler);
        self
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

    /// Enable or refuse the project type-32 service at the actual SV2
    /// ingress. This is independent of the HTTP capability advertisement.
    pub fn with_type32_enabled(mut self, enabled: bool) -> Self {
        self.type32_enabled = enabled;
        self
    }

    /// Stop accepting work for the last successfully published generation.
    /// A failed front refresh must not leave the ingress selling stale work.
    pub async fn clear_current(&self) {
        *self.current.lock().await = None;
    }

    /// Namespace deterministic channel prefixes by front (MULTI-REGION).
    pub fn with_front_id(mut self, front_id: u8) -> Self {
        self.front_id = front_id;
        self
    }

    /// Use a provisioned, stable Noise authority instead of the ephemeral key
    /// generated for unit-test constructors.
    pub fn with_authority_secret_key(mut self, secret: Secp256k1SecretKey) -> Self {
        self.authority_sk = secret;
        self.authority_pk = Secp256k1PublicKey::from(secret);
        self
    }

    pub fn authority_public_key(&self) -> Secp256k1PublicKey {
        self.authority_pk
    }

    /// Resolve a queued share only after the front has accepted or rejected
    /// it. Duplicate/stale completions have no receiver and are harmless.
    pub fn resolve_share(&self, channel_id: u32, sequence: u32, outcome: ShareOutcome) {
        self.outcomes.complete(channel_id, sequence, outcome);
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
                    if ext_type == EXT_DELIVERY {
                        let response = match (self.registration.as_ref(), channel.as_ref()) {
                            (Some(handler), Some(identity)) => {
                                handler.handle(&identity.identity, msg_type, &payload).await
                            }
                            _ => {
                                let request_id = spc_ext::correlated_request_id(msg_type, &payload);
                                let error = spc_ext::SetArkIdentityError::new(
                                    request_id,
                                    false,
                                    if channel.is_none() { "invalid-channel" } else { "unsupported-extension" },
                                    "0x5350 registration is unavailable on this listener",
                                );
                                if msg_type == spc_ext::MSG_REGISTER_EPOCH_MAILBOXES {
                                    spc_ext::DeliveryMessage::RegisterEpochMailboxesError(error)
                                } else {
                                    spc_ext::DeliveryMessage::SetArkIdentityError(error)
                                }
                            }
                        };
                        write_delivery(&mut write, &response).await?;
                        continue;
                    }
                    if ext_type == EXTENSION_TYPE_EXTENSIONS_NEGOTIATION
                        && msg_type == MESSAGE_TYPE_REQUEST_EXTENSIONS
                    {
                        let request: RequestExtensions = binary_sv2::from_bytes(&mut payload)
                            .map_err(|error| format!("request extensions: {error}"))?;
                        let supported: Vec<u16> = request
                            .requested_extensions
                            .iter()
                            .copied()
                            .filter(|extension| {
                                matches!(*extension, EXTENSION_TYPE_EXTENSIONS_NEGOTIATION | spc_ext::EXT_PAYOUTS)
                                    || (*extension == EXTENSION_TYPE && self.type32_enabled)
                                    || (*extension == EXT_DELIVERY && self.registration.is_some())
                            })
                            .collect();
                        let response = RequestExtensionsSuccess {
                            request_id: request.request_id,
                            supported_extensions: stratum_core::binary_sv2::Seq064K::new(supported)
                                .map_err(|error| format!("request extensions: {error}"))?,
                        };
                        write_msg_ext(
                            &mut write,
                            response,
                            MESSAGE_TYPE_REQUEST_EXTENSIONS_SUCCESS,
                            EXTENSION_TYPE_EXTENSIONS_NEGOTIATION,
                        ).await?;
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
                            if channel.is_some() {
                                let err = open_channel_error(msg.request_id, "channel-capacity-exhausted");
                                write_msg(&mut write, err, MESSAGE_TYPE_OPEN_MINING_CHANNEL_ERROR, false).await?;
                                continue;
                            }
                            let raw = String::from_utf8_lossy(msg.user_identity.as_bytes()).to_string();
                            let identity = match parse_identity(&raw, self.network()) {
                                Ok(id) => id,
                                Err(_) => {
                                    let err = open_channel_error(msg.request_id, "invalid-user-identity");
                                    write_msg(&mut write, err, MESSAGE_TYPE_OPEN_MINING_CHANNEL_ERROR, false).await?;
                                    continue;
                                }
                            };
                            let miner_extranonce_size: u16 = 4;
                            if msg.min_extranonce_size > miner_extranonce_size {
                                let err = open_channel_error(msg.request_id, "unsupported-min-extranonce-size");
                                write_msg(&mut write, err, MESSAGE_TYPE_OPEN_MINING_CHANNEL_ERROR, false).await?;
                                continue;
                            }
                            if !msg.nominal_hash_rate.is_finite() || msg.nominal_hash_rate <= 0.0 {
                                let err = open_channel_error(msg.request_id, "invalid-nominal-hashrate");
                                write_msg(&mut write, err, MESSAGE_TYPE_OPEN_MINING_CHANNEL_ERROR, false).await?;
                                continue;
                            }
                            let Some((channel_id, prefix)) = self.allocate_channel() else {
                                let err = open_channel_error(msg.request_id, "channel-capacity-exhausted");
                                write_msg(&mut write, err, MESSAGE_TYPE_OPEN_MINING_CHANNEL_ERROR, false).await?;
                                continue;
                            };
                            // Prefix + negotiated miner bytes must equal the
                            // coinbase's reserved extranonce length exactly.
                            if prefix.len() + usize::from(miner_extranonce_size)
                                != self.expected_extranonce_len
                            {
                                let err = open_channel_error(msg.request_id, "min-extranonce-size-too-large");
                                write_msg(&mut write, err, MESSAGE_TYPE_OPEN_MINING_CHANNEL_ERROR, false).await?;
                                continue;
                            }
                            let job = self.current.lock().await.clone();
                            let share_target = match &job {
                                Some(job) => job.work.bitcoin_share_target(),
                                None => Target::from_le_bytes([0xff; 32]),
                            };
                            let mut maximum_target = [0u8; 32];
                            maximum_target.copy_from_slice(msg.max_target.as_bytes());
                            let maximum_target = Target::from_le_bytes(maximum_target);
                            if maximum_target == Target::ZERO
                                || maximum_target.to_be_bytes() < share_target.to_be_bytes()
                            {
                                let err = open_channel_error(msg.request_id, "max-target-out-of-range");
                                write_msg(&mut write, err, MESSAGE_TYPE_OPEN_MINING_CHANNEL_ERROR, false).await?;
                                continue;
                            }
                            let mut ch = Channel {
                                front_id: self.front_id,
                                channel_id,
                                identity,
                                extranonce_prefix: prefix.clone(),
                                extranonce_size: miner_extranonce_size,
                                last_target: share_target,
                                prev_hash: None,
                                nbits: None,
                            };
                            let target_bytes = share_target.to_le_bytes();
                            let success = OpenExtendedMiningChannelSuccess {
                                request_id: msg.request_id,
                                channel_id: ch.channel_id,
                                target: U256::from(&target_bytes),
                                extranonce_size: miner_extranonce_size,
                                extranonce_prefix: B032::new(&prefix).map_err(|e| format!("prefix: {e}"))?,
                                group_channel_id: 0,
                            };
                            write_msg(&mut write, success, MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL_SUCCESS, false).await?;
                            if let Some(job) = job {
                                send_job(&mut write, &mut ch, &job).await?;
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
                            if msg.extranonce.as_bytes().len() != usize::from(ch.extranonce_size) {
                                send_submit_error(&mut write, ch.channel_id, msg.sequence_number, "bad-extranonce-size").await?;
                                continue;
                            }
                            let mut extranonce = ch.extranonce_prefix.clone();
                            extranonce.extend_from_slice(msg.extranonce.as_bytes());
                            let mut coinbase = job.coinb1.clone();
                            coinbase.extend_from_slice(&extranonce);
                            coinbase.extend_from_slice(&job.coinb2);
                            let mut merkle_root = sha256d::Hash::hash(&coinbase).to_byte_array();
                            for branch in &job.merkle_branches {
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
                            let share_target = job.work.bitcoin_share_target();
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
                            let channel_id = ch.channel_id;
                            let outcome = self.outcomes.register(channel_id, msg.sequence_number);
                            if self.shares.send(share).await.is_err() {
                                self.outcomes.abandon(channel_id, msg.sequence_number);
                                send_submit_error(&mut write, channel_id, msg.sequence_number, "invalid-share").await?;
                                continue;
                            }
                            // Keep one pinned outcome future and one fixed
                            // ten-second deadline while delivering any jobs
                            // that arrive first. A new job must not wait for
                            // this share's front outcome, but it must also not
                            // extend the share's timeout or reprice the share.
                            let deadline =
                                tokio::time::Instant::now() + Duration::from_secs(10);
                            let mut outcome = Box::pin(outcome);
                            let mut jobs_open = true;
                            let outcome = loop {
                                if jobs_open {
                                    tokio::select! {
                                        result = &mut outcome => break result,
                                        _ = tokio::time::sleep_until(deadline) => {
                                            self.outcomes.abandon(channel_id, msg.sequence_number);
                                            crate::evidence::share_outcome_unknown(
                                                self.front_id,
                                                channel_id,
                                                &job.job_id,
                                                msg.sequence_number,
                                                "front outcome timed out or was lost; accounting state is unknown",
                                            );
                                            return Err("share outcome unknown".into());
                                        }
                                        job = jobs.recv() => {
                                            match job {
                                                Ok(job) => {
                                                    if let Some(ch) = channel.as_mut() {
                                                        send_job(&mut write, ch, &job).await?;
                                                    }
                                                }
                                                Err(broadcast::error::RecvError::Lagged(_)) => {
                                                    return Err("miner lagged behind clean job publication".into());
                                                }
                                                Err(broadcast::error::RecvError::Closed) => {
                                                    // Keep waiting for the share
                                                    // outcome on its original
                                                    // deadline; there is no job
                                                    // publisher left to preempt.
                                                    jobs_open = false;
                                                }
                                            }
                                        }
                                    }
                                } else {
                                    tokio::select! {
                                        result = &mut outcome => break result,
                                        _ = tokio::time::sleep_until(deadline) => {
                                            self.outcomes.abandon(channel_id, msg.sequence_number);
                                            crate::evidence::share_outcome_unknown(
                                                self.front_id,
                                                channel_id,
                                                &job.job_id,
                                                msg.sequence_number,
                                                "front outcome timed out or was lost; accounting state is unknown",
                                            );
                                            return Err("share outcome unknown".into());
                                        }
                                    }
                                }
                            };
                            let outcome = match outcome {
                                Ok(outcome) => outcome,
                                Err(_) => {
                                    self.outcomes.abandon(channel_id, msg.sequence_number);
                                    crate::evidence::share_outcome_unknown(
                                        self.front_id,
                                        channel_id,
                                        &job.job_id,
                                        msg.sequence_number,
                                        "front outcome timed out or was lost; accounting state is unknown",
                                    );
                                    return Err("share outcome unknown".into());
                                }
                            };
                            match outcome {
                                ShareOutcome::Accepted { credited_work } => {
                                    let ok = SubmitSharesSuccess {
                                        channel_id,
                                        last_sequence_number: msg.sequence_number,
                                        new_submits_accepted_count: 1,
                                        new_shares_sum: credited_work,
                                    };
                                    write_msg(&mut write, ok, MESSAGE_TYPE_SUBMIT_SHARES_SUCCESS, CHANNEL_BIT_SUBMIT_SHARES_SUCCESS).await?;
                                }
                                ShareOutcome::Rejected { error_code } => {
                                    send_submit_error(&mut write, channel_id, msg.sequence_number, &error_code).await?;
                                }
                            }
                        }
                        _ => {}
                    }
                }
                job = jobs.recv() => {
                    match job {
                        Ok(job) => {
                            if let Some(ch) = channel.as_mut() {
                                send_job(&mut write, ch, &job).await?;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            return Err("miner lagged behind clean job publication".into());
                        }
                        Err(_) => break,
                    }
                }
            }
        }
        Ok(())
    }

    fn allocate_channel(&self) -> Option<(u32, Vec<u8>)> {
        let channel_id = self.next_channel_id.fetch_add(1, Ordering::Relaxed);
        if channel_id == 0 || channel_id > 0x7fff_ffff {
            return None;
        }
        let sequence = self.next_extranonce_prefix.fetch_add(1, Ordering::Relaxed);
        if sequence > 0x00ff_ffff {
            return None;
        }
        let mut prefix = vec![0u8; 4];
        prefix[0] = self.front_id;
        prefix[1..].copy_from_slice(&sequence.to_le_bytes()[..3]);
        Some((channel_id, prefix))
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
        if !self.type32_enabled {
            let request_id = match message {
                Type32Message::GetWindow { request_id, .. }
                | Type32Message::GetWindowSuccess { request_id, .. }
                | Type32Message::GetWindowBusy { request_id, .. }
                | Type32Message::GetShares { request_id, .. }
                | Type32Message::GetSharesSuccess { request_id, .. }
                | Type32Message::GetTransactionsInJob { request_id, .. }
                | Type32Message::GetTransactionsInJobSuccess { request_id, .. }
                | Type32Message::Error { request_id, .. } => request_id,
                Type32Message::NewBlockFound { .. } => 0,
            };
            return write_type32(
                write,
                &Type32Message::Error {
                    request_id,
                    code: "unsupported-extension".into(),
                },
            )
            .await;
        }
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
    ch: &mut Channel,
    job: &Job,
) -> Result<(), String> {
    // SetTarget affects later jobs and already queued future jobs. It never
    // rewrites the target of an already active non-future job, so each job is
    // sent after the target it must capture.
    let target = job.work.bitcoin_share_target();
    if target != ch.last_target {
        let target_bytes = target.to_le_bytes();
        write_msg(
            write,
            SetTarget {
                channel_id: ch.channel_id,
                maximum_target: U256::from(&target_bytes),
            },
            MESSAGE_TYPE_SET_TARGET,
            CHANNEL_BIT_SET_TARGET,
        )
        .await?;
        ch.last_target = target;
    }

    // `nbits` is carried only by SetNewPrevHash. A same-prevhash bits
    // transition is therefore a new activation too (future job + prevhash).
    let activation_required = ch.prev_hash != Some(job.prev_hash) || ch.nbits != Some(job.nbits);
    let merkle_path: Vec<U256> = job.merkle_branches.iter().map(U256::from).collect();
    let job_msg = NewExtendedMiningJob {
        channel_id: ch.channel_id,
        job_id: parse_job_id(&job.job_id),
        // Empty min_ntime marks a future job. Activation must name a job the
        // client already holds; unchanged-(prevhash, bits) updates are active.
        min_ntime: Sv2Option::new(if activation_required {
            None
        } else {
            Some(job.ntime)
        }),
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
    .await?;

    if activation_required {
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
    }
    ch.prev_hash = Some(job.prev_hash);
    ch.nbits = Some(job.nbits);
    crate::evidence::job_issued(
        ch.front_id(),
        ch.channel_id,
        job,
        if activation_required {
            "future"
        } else {
            "active"
        },
    );
    Ok(())
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
    code: &str,
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

async fn write_delivery(
    write: &mut stratum_apps::network_helpers::noise_stream::NoiseTcpWriteHalf,
    message: &spc_ext::DeliveryMessage,
) -> Result<(), String> {
    let payload = message
        .encode()
        .map_err(|error| format!("0x5350 encode: {error}"))?;
    write_raw_extension(write, EXT_DELIVERY, message.message_type(), &payload).await
}

async fn write_msg_ext<T>(
    write: &mut stratum_apps::network_helpers::noise_stream::NoiseTcpWriteHalf,
    message: T,
    msg_type: u8,
    ext_type: u16,
) -> Result<(), String>
where
    T: stratum_core::binary_sv2::Serialize + stratum_core::binary_sv2::GetSize,
{
    let frame = MessageFrame::from_message(message, msg_type, ext_type, false)
        .map_err(|error| format!("extension frame: {error}"))?;
    write
        .write_frame(frame)
        .await
        .map_err(|error| format!("extension write: {error}"))
}

async fn write_raw_extension(
    write: &mut stratum_apps::network_helpers::noise_stream::NoiseTcpWriteHalf,
    ext_type: u16,
    msg_type: u8,
    payload: &[u8],
) -> Result<(), String> {
    let mut bytes = Vec::with_capacity(6 + payload.len());
    bytes.extend_from_slice(&ext_type.to_le_bytes());
    bytes.push(msg_type);
    let len = u32::try_from(payload.len()).map_err(|_| "extension payload too long")?;
    bytes.extend_from_slice(&len.to_le_bytes()[..3]);
    bytes.extend_from_slice(payload);
    let frame = stratum_core::codec_sv2::SerializedFrame::from_bytes(bytes)
        .map_err(|error| format!("extension frame: {error:?}"))?;
    write
        .write_frame(frame)
        .await
        .map_err(|error| format!("extension write: {error}"))
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

/// Target helper for tests (the consensus target for `bits`).
pub fn target_from_bits(bits: u32) -> Target {
    crate::sv1::network_target(bits)
}

pub fn hash_meets_target(hash: [u8; 32], bits: u32) -> bool {
    target_from_bits(bits).is_met_by(BlockHash::from_byte_array(hash))
}

/// The target this listener puts in
/// `OpenExtendedMiningChannelSuccess.target` for `share_difficulty` at the
/// current template's `bits`, in SV2 U256 little-endian wire order.
pub fn announced_share_target(share_difficulty: u64, bits: u32) -> [u8; 32] {
    crate::sv1::effective_share_target(share_difficulty, bits).to_le_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let expected = crate::sv1::target_for_difficulty(4).to_le_bytes();
        assert_eq!(announced, expected);
        assert_ne!(
            announced,
            crate::sv1::target_for_difficulty(4).to_be_bytes()
        );

        // Regtest bits: the announced target is never harder than the network
        // target, so a regtest miner is not asked for mainnet work.
        let regtest = announced_share_target(u64::MAX, 0x207f_ffff);
        assert_eq!(
            regtest,
            crate::sv1::network_target(0x207f_ffff).to_le_bytes()
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
            work: crate::sv1::JobWork::legacy(1, 0x207f_ffff, 1),
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
    fn channel_ids_and_prefixes_are_unique_by_construction() {
        let (tx, _rx) = mpsc::channel(1);
        let store = std::sync::Arc::new(std::sync::Mutex::new(Type32Store::default()));
        let first = Sv2Server::new(spc_crypto::Network::Regtest, tx.clone(), store.clone(), 8)
            .with_front_id(1);
        let second = Sv2Server::new(spc_crypto::Network::Regtest, tx, store, 8).with_front_id(2);
        let mut prefixes = std::collections::HashSet::new();
        let mut first_channels = std::collections::HashSet::new();
        let mut second_channels = std::collections::HashSet::new();
        for _ in 0..1_000 {
            let (a_id, a_prefix) = first.allocate_channel().unwrap();
            let (b_id, b_prefix) = second.allocate_channel().unwrap();
            assert!(a_id < 0x8000_0000);
            assert!(b_id < 0x8000_0000);
            assert!(first_channels.insert(a_id));
            assert!(second_channels.insert(b_id));
            assert!(prefixes.insert(a_prefix));
            assert!(prefixes.insert(b_prefix));
        }
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
