//! SV2 miner-side session state, Noise authentication, jobs and correlated
//! share results. Future jobs activate only through `SetNewPrevHash`; targets
//! are captured when a job becomes active (sv2-spec §5.3.21).

use std::collections::VecDeque;

use bitcoin::hashes::Hash;
use bitcoin::pow::Target as PowTarget;
use spc_crypto::Network;
use spc_pool::net::{Egress, Target as DialTarget};
use stratum_apps::key_utils::Secp256k1PublicKey;
use stratum_apps::network_helpers::connect_with_noise;
use stratum_apps::network_helpers::noise_stream::{NoiseTcpReadHalf, NoiseTcpWriteHalf};
use stratum_core::binary_sv2::{self, Str0255, B032, U256};
use stratum_core::common_messages_sv2::{
    Protocol, SetupConnection, SetupConnectionSuccess, MESSAGE_TYPE_SETUP_CONNECTION,
    MESSAGE_TYPE_SETUP_CONNECTION_SUCCESS,
};
use stratum_core::framing_sv2::framing::MessageFrame;
use stratum_core::mining_sv2::{
    NewExtendedMiningJob, OpenExtendedMiningChannel, OpenExtendedMiningChannelSuccess,
    OpenMiningChannelError, SetNewPrevHash, SetTarget, SubmitSharesError, SubmitSharesExtended,
    SubmitSharesSuccess, MESSAGE_TYPE_MINING_SET_NEW_PREV_HASH,
    MESSAGE_TYPE_NEW_EXTENDED_MINING_JOB, MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL,
    MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL_SUCCESS, MESSAGE_TYPE_OPEN_MINING_CHANNEL_ERROR,
    MESSAGE_TYPE_SET_TARGET, MESSAGE_TYPE_SUBMIT_SHARES_ERROR, MESSAGE_TYPE_SUBMIT_SHARES_EXTENDED,
    MESSAGE_TYPE_SUBMIT_SHARES_SUCCESS,
};
use thiserror::Error;
use tokio::sync::mpsc;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("network: {0}")]
    Network(String),
    #[error("codec: {0}")]
    Codec(String),
    #[error("unexpected message {0}")]
    Unexpected(u8),
    #[error("protocol: {0}")]
    Protocol(String),
    #[error("invalid address")]
    Address,
    #[error("bad miner extranonce length: expected {expected}, got {actual}")]
    Extranonce { expected: usize, actual: usize },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoolJob {
    pub job_id: u32,
    pub prev_hash: [u8; 32],
    pub version: u32,
    pub nbits: u32,
    pub ntime: u32,
    pub coinbase_prefix: Vec<u8>,
    pub coinbase_suffix: Vec<u8>,
    /// Deepest-first, exactly as received on the wire.
    pub merkle_path: Vec<[u8; 32]>,
    /// The channel target captured when this job became active. Later
    /// `SetTarget` messages do not retroactively change it.
    pub share_target: PowTarget,
}

impl PoolJob {
    /// Wallet/collector integration for an ACTUAL received SV2 job. No scan
    /// secret is put in a CPU-miner config or protocol message. The caller
    /// supplies independently captured proof/history seals, not pool owed sats.
    pub fn verify_source_bundle(
        &self,
        bundle: &[u8],
        trusted: &spc_scan::source_audit::TrustedSource,
        pool_extranonce: &[u8],
        miner_extranonce_len: usize,
        b_scan: &bitcoin::secp256k1::SecretKey,
        b_spend: &bitcoin::secp256k1::PublicKey,
    ) -> Result<(Vec<spc_scan::Found>, u64), crate::verify::VerifyError> {
        use spc_scan::source_audit::{JobProof, SourceBundle};
        if bundle.len() > 4 * 1024 * 1024 {
            return Err(crate::verify::VerifyError::Entitlement);
        }
        let source: SourceBundle =
            serde_json::from_slice(bundle).map_err(|_| crate::verify::VerifyError::Entitlement)?;
        let extranonce_len = pool_extranonce
            .len()
            .checked_add(miner_extranonce_len)
            .ok_or(crate::verify::VerifyError::Entitlement)?;
        if extranonce_len > 63 {
            return Err(crate::verify::VerifyError::Entitlement);
        }
        let mut raw = self.coinbase_prefix.clone();
        raw.extend_from_slice(pool_extranonce);
        raw.extend(std::iter::repeat_n(0, miner_extranonce_len));
        raw.extend_from_slice(&self.coinbase_suffix);
        let tx: bitcoin::Transaction = bitcoin::consensus::deserialize(&raw)
            .map_err(|_| crate::verify::VerifyError::Entitlement)?;
        let height = spc_scan::parse_bip34_height(
            tx.input
                .first()
                .ok_or(crate::verify::VerifyError::Entitlement)?
                .script_sig
                .as_bytes(),
        )
        .ok_or(crate::verify::VerifyError::Entitlement)?;
        // Generation is source-attested; actual wire bytes/target are checked.
        let issued = JobProof {
            job_id: (u64::from(height) << 32) | u64::from(self.job_id),
            generation: source.job.generation,
            height,
            prev_hash: self.prev_hash,
            version: self.version,
            ntime: self.ntime,
            nbits: self.nbits,
            coinbase_prefix: self.coinbase_prefix.clone(),
            coinbase_suffix: self.coinbase_suffix.clone(),
            extranonce_len,
            merkle_path: self.merkle_path.clone(),
            share_target_be: self.share_target.to_be_bytes(),
            credited_work: source.job.credited_work,
            transactions: vec![],
            prevout_transactions: vec![],
        };
        crate::verify::verify_source_entitlement(bundle, trusted, &issued, b_scan, b_spend)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoolShare {
    pub job_id: u32,
    pub nonce: u32,
    pub ntime: u32,
    pub version: u32,
    /// Miner-controlled bytes only; never prepend the pool prefix here.
    pub extranonce: Vec<u8>,
    pub sequence_number: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShareResult {
    /// The front accepted the share into its slice. This is not a claim that a
    /// candidate block was accepted by the node or settled by payout.
    Accepted {
        sequence_number: u32,
        credited_work: u64,
    },
    Rejected {
        sequence_number: u32,
        error_code: String,
    },
}

#[derive(Clone)]
struct ChainTip {
    prev_hash: [u8; 32],
    min_ntime: u32,
    nbits: u32,
}

struct JobAnnouncement {
    job_id: u32,
    min_ntime: Option<u32>,
    version: u32,
    coinbase_prefix: Vec<u8>,
    coinbase_suffix: Vec<u8>,
    merkle_path: Vec<[u8; 32]>,
}

struct JobState {
    tip: Option<ChainTip>,
    future: Option<JobAnnouncement>,
    active_job_id: Option<u32>,
    channel_target: PowTarget,
}

impl JobState {
    fn new(target: PowTarget) -> Self {
        Self {
            tip: None,
            future: None,
            active_job_id: None,
            channel_target: target,
        }
    }

    fn on_target(&mut self, target: PowTarget) -> Result<(), ClientError> {
        if target == PowTarget::ZERO {
            return Err(ClientError::Protocol("zero SetTarget".into()));
        }
        // Already active nonfuture jobs intentionally keep their captured
        // target. Queued future jobs read channel_target at activation.
        self.channel_target = target;
        Ok(())
    }

    fn on_job(&mut self, job: JobAnnouncement) -> Result<Option<PoolJob>, ClientError> {
        let Some(min_ntime) = job.min_ntime else {
            self.future = Some(job);
            return Ok(None);
        };
        let tip = self.tip.as_ref().ok_or_else(|| {
            ClientError::Protocol("nonfuture job arrived before SetNewPrevHash".into())
        })?;
        if min_ntime < tip.min_ntime {
            return Err(ClientError::Protocol(
                "nonfuture job min_ntime is below the active tip".into(),
            ));
        }
        let job = self.activate(job, tip.clone(), min_ntime);
        Ok(Some(job))
    }

    fn on_prev_hash(&mut self, prev: SetNewPrevHash) -> Result<PoolJob, ClientError> {
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(prev.prev_hash.as_bytes());
        let future = self.future.take().filter(|job| job.job_id == prev.job_id);
        let Some(job) = future else {
            return Err(ClientError::Protocol(format!(
                "SetNewPrevHash names unknown future job {}",
                prev.job_id
            )));
        };
        let tip = ChainTip {
            prev_hash: bytes,
            min_ntime: prev.min_ntime,
            nbits: prev.nbits,
        };
        Ok(self.activate(job, tip, prev.min_ntime))
    }

    fn activate(&mut self, job: JobAnnouncement, tip: ChainTip, min_ntime: u32) -> PoolJob {
        self.tip = Some(tip.clone());
        self.active_job_id = Some(job.job_id);
        PoolJob {
            job_id: job.job_id,
            prev_hash: tip.prev_hash,
            version: job.version,
            nbits: tip.nbits,
            ntime: min_ntime,
            coinbase_prefix: job.coinbase_prefix,
            coinbase_suffix: job.coinbase_suffix,
            merkle_path: job.merkle_path,
            share_target: self.channel_target,
        }
    }

    fn accepts_job(&self, job_id: u32) -> bool {
        self.active_job_id == Some(job_id)
    }
}

enum ClientEvent {
    Job(PoolJob),
    Result(ShareResult),
    Ignored,
}

fn target_from_wire(bytes: &[u8]) -> Result<PowTarget, ClientError> {
    let raw: [u8; 32] = bytes
        .try_into()
        .map_err(|_| ClientError::Protocol("target is not 32 bytes".into()))?;
    let target = PowTarget::from_le_bytes(raw);
    if target == PowTarget::ZERO {
        return Err(ClientError::Protocol("zero channel target".into()));
    }
    Ok(target)
}

fn parse_job(message: NewExtendedMiningJob) -> JobAnnouncement {
    let min_ntime = message.min_ntime.into_inner();
    JobAnnouncement {
        job_id: message.job_id,
        min_ntime,
        version: message.version,
        coinbase_prefix: message.coinbase_tx_prefix.as_bytes().to_vec(),
        coinbase_suffix: message.coinbase_tx_suffix.as_bytes().to_vec(),
        merkle_path: message
            .merkle_path
            .iter()
            .map(|node| {
                let mut bytes = [0u8; 32];
                bytes.copy_from_slice(node.as_bytes());
                bytes
            })
            .collect(),
    }
}

async fn read_event(
    read: &mut NoiseTcpReadHalf,
    state: &mut JobState,
) -> Result<ClientEvent, ClientError> {
    loop {
        let mut frame = read
            .read_frame()
            .await
            .map_err(|e| ClientError::Network(e.to_string()))?;
        let ty = frame.header().msg_type();
        let mut payload = frame.payload().to_vec();
        match ty {
            MESSAGE_TYPE_NEW_EXTENDED_MINING_JOB => {
                let message: NewExtendedMiningJob = binary_sv2::from_bytes(&mut payload)
                    .map_err(|e| ClientError::Codec(e.to_string()))?;
                if let Some(job) = state.on_job(parse_job(message))? {
                    return Ok(ClientEvent::Job(job));
                }
            }
            MESSAGE_TYPE_MINING_SET_NEW_PREV_HASH => {
                let message: SetNewPrevHash = binary_sv2::from_bytes(&mut payload)
                    .map_err(|e| ClientError::Codec(e.to_string()))?;
                return Ok(ClientEvent::Job(state.on_prev_hash(message)?));
            }
            MESSAGE_TYPE_SET_TARGET => {
                let message: SetTarget = binary_sv2::from_bytes(&mut payload)
                    .map_err(|e| ClientError::Codec(e.to_string()))?;
                state.on_target(target_from_wire(message.maximum_target.as_bytes())?)?;
            }
            MESSAGE_TYPE_SUBMIT_SHARES_SUCCESS => {
                let message: SubmitSharesSuccess = binary_sv2::from_bytes(&mut payload)
                    .map_err(|e| ClientError::Codec(e.to_string()))?;
                return Ok(ClientEvent::Result(ShareResult::Accepted {
                    sequence_number: message.last_sequence_number,
                    credited_work: message.new_shares_sum,
                }));
            }
            MESSAGE_TYPE_SUBMIT_SHARES_ERROR => {
                let message: SubmitSharesError = binary_sv2::from_bytes(&mut payload)
                    .map_err(|e| ClientError::Codec(e.to_string()))?;
                return Ok(ClientEvent::Result(ShareResult::Rejected {
                    sequence_number: message.sequence_number,
                    error_code: String::from_utf8_lossy(message.error_code.as_bytes()).into(),
                }));
            }
            _ => return Ok(ClientEvent::Ignored),
        }
    }
}

pub struct Sv2Client {
    read: NoiseTcpReadHalf,
    write: NoiseTcpWriteHalf,
    channel_id: u32,
    pub extranonce_prefix: Vec<u8>,
    pub extranonce_size: u16,
    sequence: u32,
    state: JobState,
    jobs: VecDeque<PoolJob>,
    results: VecDeque<ShareResult>,
}

impl Sv2Client {
    /// Wallet-only connection: consumes the mining client so this connection
    /// cannot also submit shares. Registration uses the same native channel.
    #[cfg(feature = "ark")]
    pub(crate) fn into_wallet_connection(self) -> (NoiseTcpReadHalf, NoiseTcpWriteHalf) {
        (self.read, self.write)
    }

    /// Dial the pool and authenticate its provisioned authority. A wrong pin
    /// fails the Noise handshake; there is no unauthenticated fallback.
    pub async fn connect(
        target: &DialTarget,
        egress: &Egress,
        sp_address: &str,
        authority_public_key: Secp256k1PublicKey,
        _network: Network,
    ) -> Result<Self, ClientError> {
        Self::connect_inner(target, egress, sp_address, authority_public_key, false).await
    }

    #[cfg(feature = "ark")]
    pub(crate) async fn connect_wallet(
        target: &DialTarget,
        egress: &Egress,
        sp_address: &str,
        authority_public_key: Secp256k1PublicKey,
    ) -> Result<Self, ClientError> {
        Self::connect_inner(target, egress, sp_address, authority_public_key, true).await
    }

    async fn connect_inner(
        target: &DialTarget,
        egress: &Egress,
        sp_address: &str,
        authority_public_key: Secp256k1PublicKey,
        delivery: bool,
    ) -> Result<Self, ClientError> {
        let stream = egress
            .connect(target)
            .await
            .map_err(|e| ClientError::Network(e.to_string()))?;
        let stream = connect_with_noise(stream, Some(authority_public_key))
            .await
            .map_err(|e| ClientError::Network(e.to_string()))?;
        let (mut read, mut write) = stream.into_split();

        let s = |x: &'static str| Str0255::try_from(x).unwrap();
        let setup = SetupConnection {
            protocol: Protocol::MiningProtocol,
            min_version: 2,
            max_version: 2,
            flags: 0,
            endpoint_host: s("127.0.0.1"),
            endpoint_port: 0,
            vendor: s("spc-agent"),
            hardware_version: s("1"),
            firmware: s("1"),
            device_id: s("1"),
        };
        write_msg(&mut write, setup, MESSAGE_TYPE_SETUP_CONNECTION, false).await?;
        let mut frame = read
            .read_frame()
            .await
            .map_err(|e| ClientError::Network(e.to_string()))?;
        if frame.header().msg_type() != MESSAGE_TYPE_SETUP_CONNECTION_SUCCESS {
            return Err(ClientError::Unexpected(frame.header().msg_type()));
        }
        let mut payload = frame.payload().to_vec();
        let _: SetupConnectionSuccess =
            binary_sv2::from_bytes(&mut payload).map_err(|e| ClientError::Codec(e.to_string()))?;

        if delivery {
            use stratum_core::extensions_sv2::{
                RequestExtensions, RequestExtensionsSuccess, EXTENSION_TYPE_EXTENSIONS_NEGOTIATION,
                MESSAGE_TYPE_REQUEST_EXTENSIONS, MESSAGE_TYPE_REQUEST_EXTENSIONS_SUCCESS,
            };
            let request = RequestExtensions {
                request_id: 1,
                requested_extensions: binary_sv2::Seq064K::new(vec![spc_ext::EXT_DELIVERY])
                    .map_err(|e| ClientError::Codec(e.to_string()))?,
            };
            let frame = MessageFrame::from_message(
                request,
                MESSAGE_TYPE_REQUEST_EXTENSIONS,
                EXTENSION_TYPE_EXTENSIONS_NEGOTIATION,
                false,
            )
            .map_err(|e| ClientError::Codec(e.to_string()))?;
            write
                .write_frame(frame)
                .await
                .map_err(|e| ClientError::Network(e.to_string()))?;
            let mut frame = read
                .read_frame()
                .await
                .map_err(|e| ClientError::Network(e.to_string()))?;
            if frame.header().ext_type_without_channel_msg()
                != EXTENSION_TYPE_EXTENSIONS_NEGOTIATION
                || frame.header().msg_type() != MESSAGE_TYPE_REQUEST_EXTENSIONS_SUCCESS
            {
                return Err(ClientError::Protocol(
                    "delivery negotiation rejected".into(),
                ));
            }
            let mut payload = frame.payload().to_vec();
            let response: RequestExtensionsSuccess = binary_sv2::from_bytes(&mut payload)
                .map_err(|e| ClientError::Codec(e.to_string()))?;
            if response.request_id != 1
                || !response
                    .supported_extensions
                    .iter()
                    .any(|id| *id == spc_ext::EXT_DELIVERY)
            {
                return Err(ClientError::Protocol("unsupported-extension".into()));
            }
        }

        let max_target_bytes = [0xffu8; 32];
        let open = OpenExtendedMiningChannel {
            request_id: 1,
            user_identity: Str0255::try_from(sp_address).map_err(|_| ClientError::Address)?,
            nominal_hash_rate: 1.0,
            max_target: U256::from(&max_target_bytes),
            min_extranonce_size: 4,
        };
        write_msg(
            &mut write,
            open,
            MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL,
            false,
        )
        .await?;
        let mut frame = read
            .read_frame()
            .await
            .map_err(|e| ClientError::Network(e.to_string()))?;
        if frame.header().msg_type() == MESSAGE_TYPE_OPEN_MINING_CHANNEL_ERROR {
            let mut payload = frame.payload().to_vec();
            let error: OpenMiningChannelError = binary_sv2::from_bytes(&mut payload)
                .map_err(|e| ClientError::Codec(e.to_string()))?;
            return Err(ClientError::Protocol(
                String::from_utf8_lossy(error.error_code.as_bytes()).into(),
            ));
        }
        if frame.header().msg_type() != MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL_SUCCESS {
            return Err(ClientError::Unexpected(frame.header().msg_type()));
        }
        let mut payload = frame.payload().to_vec();
        let success: OpenExtendedMiningChannelSuccess =
            binary_sv2::from_bytes(&mut payload).map_err(|e| ClientError::Codec(e.to_string()))?;
        let target = target_from_wire(success.target.as_bytes())?;
        Ok(Self {
            read,
            write,
            channel_id: success.channel_id,
            extranonce_prefix: success.extranonce_prefix.as_bytes().to_vec(),
            extranonce_size: success.extranonce_size,
            sequence: 0,
            state: JobState::new(target),
            jobs: VecDeque::new(),
            results: VecDeque::new(),
        })
    }

    /// Wait for an activated job. A future job is not returned until its
    /// matching `SetNewPrevHash` arrives.
    pub async fn next_job(&mut self) -> Result<PoolJob, ClientError> {
        loop {
            if let Some(job) = self.jobs.pop_front() {
                return Ok(job);
            }
            match read_event(&mut self.read, &mut self.state).await? {
                ClientEvent::Job(job) => self.jobs.push_back(job),
                ClientEvent::Result(result) => self.results.push_back(result),
                ClientEvent::Ignored => {}
            }
        }
    }

    /// Wait for the correlated front-side result of a submission.
    pub async fn next_result(&mut self) -> Result<ShareResult, ClientError> {
        loop {
            if let Some(result) = self.results.pop_front() {
                return Ok(result);
            }
            match read_event(&mut self.read, &mut self.state).await? {
                ClientEvent::Job(job) => self.jobs.push_back(job),
                ClientEvent::Result(result) => self.results.push_back(result),
                ClientEvent::Ignored => {}
            }
        }
    }

    pub async fn submit(&mut self, share: PoolShare) -> Result<u32, ClientError> {
        if share.extranonce.len() != usize::from(self.extranonce_size) {
            return Err(ClientError::Extranonce {
                expected: usize::from(self.extranonce_size),
                actual: share.extranonce.len(),
            });
        }
        if !self.state.accepts_job(share.job_id) {
            return Err(ClientError::Protocol(format!(
                "job {} is not active",
                share.job_id
            )));
        }
        let sequence = self.sequence;
        let msg = SubmitSharesExtended {
            channel_id: self.channel_id,
            sequence_number: sequence,
            job_id: share.job_id,
            nonce: share.nonce,
            ntime: share.ntime,
            version: share.version,
            extranonce: B032::new(&share.extranonce)
                .map_err(|e| ClientError::Codec(e.to_string()))?,
        };
        self.sequence = self.sequence.wrapping_add(1);
        write_msg(
            &mut self.write,
            msg,
            MESSAGE_TYPE_SUBMIT_SHARES_EXTENDED,
            true,
        )
        .await?;
        Ok(sequence)
    }

    pub fn channel_id(&self) -> u32 {
        self.channel_id
    }

    pub fn channel_target(&self) -> PowTarget {
        self.state.channel_target
    }

    pub fn split(self) -> (PoolJobReader, PoolShareWriter, mpsc::Receiver<ShareResult>) {
        let (result_tx, result_rx) = mpsc::channel(128);
        for result in self.results {
            let _ = result_tx.try_send(result);
        }
        let reader = PoolJobReader {
            read: self.read,
            channel_id: self.channel_id,
            extranonce_prefix: self.extranonce_prefix,
            extranonce_size: self.extranonce_size,
            state: self.state,
            jobs: self.jobs,
            result_tx,
        };
        let writer = PoolShareWriter {
            write: self.write,
            channel_id: self.channel_id,
            extranonce_size: self.extranonce_size,
            sequence: self.sequence,
        };
        (reader, writer, result_rx)
    }
}

async fn write_msg<T>(
    w: &mut NoiseTcpWriteHalf,
    msg: T,
    msg_type: u8,
    channel_msg: bool,
) -> Result<(), ClientError>
where
    T: binary_sv2::Serialize + binary_sv2::GetSize,
{
    let frame = MessageFrame::from_message(msg, msg_type, 0, channel_msg)
        .map_err(|e| ClientError::Codec(e.to_string()))?;
    w.write_frame(frame)
        .await
        .map_err(|e| ClientError::Network(e.to_string()))
}

/// Convert a pool job to the SV1 job shape. The translator uses an empty
/// extranonce1 and gives the miner only the pool's miner-extranonce bytes.
pub fn to_sv1_job(job: &PoolJob, extranonce_prefix: &[u8]) -> spc_pool::sv1::Job {
    let mut coinb1 = job.coinbase_prefix.clone();
    coinb1.extend_from_slice(extranonce_prefix);
    spc_pool::sv1::Job {
        job_id: format!("{:08x}", job.job_id),
        prev_hash: job.prev_hash,
        height: 0,
        version: job.version,
        nbits: job.nbits,
        ntime: job.ntime,
        coinb1,
        coinb2: job.coinbase_suffix.clone(),
        merkle_branches: job.merkle_path.clone(),
        clean_jobs: true,
        work: spc_pool::sv1::JobWork::legacy(1, job.nbits, 0),
    }
}

/// Merkle root and header hash for a share, given the full coinbase.
pub fn share_hash(
    version: u32,
    prev_hash: [u8; 32],
    coinbase: &[u8],
    merkle_path: &[[u8; 32]],
    ntime: u32,
    nbits: u32,
    nonce: u32,
) -> [u8; 32] {
    let mut root = bitcoin::hashes::sha256d::Hash::hash(coinbase).to_byte_array();
    for sibling in merkle_path {
        let mut buf = Vec::with_capacity(64);
        buf.extend_from_slice(&root);
        buf.extend_from_slice(sibling);
        root = bitcoin::hashes::sha256d::Hash::hash(&buf).to_byte_array();
    }
    let mut header = Vec::with_capacity(80);
    header.extend_from_slice(&version.to_le_bytes());
    header.extend_from_slice(&prev_hash);
    header.extend_from_slice(&root);
    header.extend_from_slice(&ntime.to_le_bytes());
    header.extend_from_slice(&nbits.to_le_bytes());
    header.extend_from_slice(&nonce.to_le_bytes());
    bitcoin::hashes::sha256d::Hash::hash(&header).to_byte_array()
}

pub struct PoolJobReader {
    read: NoiseTcpReadHalf,
    pub channel_id: u32,
    pub extranonce_prefix: Vec<u8>,
    pub extranonce_size: u16,
    state: JobState,
    jobs: VecDeque<PoolJob>,
    result_tx: mpsc::Sender<ShareResult>,
}

impl PoolJobReader {
    pub async fn next_job(&mut self) -> Result<PoolJob, ClientError> {
        loop {
            if let Some(job) = self.jobs.pop_front() {
                return Ok(job);
            }
            match read_event(&mut self.read, &mut self.state).await? {
                ClientEvent::Job(job) => self.jobs.push_back(job),
                ClientEvent::Result(result) => {
                    let _ = self.result_tx.send(result).await;
                }
                ClientEvent::Ignored => {}
            }
        }
    }
}

pub struct PoolShareWriter {
    write: NoiseTcpWriteHalf,
    channel_id: u32,
    extranonce_size: u16,
    sequence: u32,
}

impl PoolShareWriter {
    pub async fn submit(&mut self, share: PoolShare) -> Result<u32, ClientError> {
        if share.extranonce.len() != usize::from(self.extranonce_size) {
            return Err(ClientError::Extranonce {
                expected: usize::from(self.extranonce_size),
                actual: share.extranonce.len(),
            });
        }
        let sequence = self.sequence;
        let msg = SubmitSharesExtended {
            channel_id: self.channel_id,
            sequence_number: sequence,
            job_id: share.job_id,
            nonce: share.nonce,
            ntime: share.ntime,
            version: share.version,
            extranonce: B032::new(&share.extranonce)
                .map_err(|e| ClientError::Codec(e.to_string()))?,
        };
        self.sequence = self.sequence.wrapping_add(1);
        write_msg(
            &mut self.write,
            msg,
            MESSAGE_TYPE_SUBMIT_SHARES_EXTENDED,
            true,
        )
        .await?;
        Ok(sequence)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stratum_core::binary_sv2::{Sv2Option, B064K};

    fn announcement(job_id: u32, min_ntime: Option<u32>) -> JobAnnouncement {
        JobAnnouncement {
            job_id,
            min_ntime,
            version: 0x2000_0000,
            coinbase_prefix: vec![1],
            coinbase_suffix: vec![2],
            merkle_path: vec![],
        }
    }

    fn initial_target() -> PowTarget {
        PowTarget::from_le_bytes([0x11; 32])
    }

    fn updated_target() -> PowTarget {
        PowTarget::from_le_bytes([0x22; 32])
    }

    #[test]
    fn target_before_future_activation_applies_at_activation() {
        let mut state = JobState::new(initial_target());
        state.on_job(announcement(7, None)).unwrap();
        state.on_target(updated_target()).unwrap();
        let mut prev_bytes = [0u8; 32];
        prev_bytes.copy_from_slice(&[0x33; 32]);
        let active = state
            .on_prev_hash(SetNewPrevHash {
                channel_id: 1,
                job_id: 7,
                prev_hash: U256::from(&prev_bytes),
                min_ntime: 100,
                nbits: 0x207f_ffff,
            })
            .unwrap();
        assert_eq!(active.share_target, updated_target());
    }

    #[test]
    fn target_after_activation_does_not_rewrite_active_job() {
        let mut state = JobState::new(initial_target());
        state.on_job(announcement(7, None)).unwrap();
        let active = state
            .on_prev_hash(SetNewPrevHash {
                channel_id: 1,
                job_id: 7,
                prev_hash: U256::from(&[0x33; 32]),
                min_ntime: 100,
                nbits: 0x207f_ffff,
            })
            .unwrap();
        state.on_target(updated_target()).unwrap();
        assert_eq!(active.share_target, initial_target());
        assert_eq!(state.channel_target, updated_target());
    }

    #[test]
    fn prev_hash_requires_an_already_received_future_job() {
        let mut state = JobState::new(initial_target());
        let error = state
            .on_prev_hash(SetNewPrevHash {
                channel_id: 1,
                job_id: 99,
                prev_hash: U256::from(&[0x33; 32]),
                min_ntime: 100,
                nbits: 0x207f_ffff,
            })
            .unwrap_err();
        assert!(error.to_string().contains("unknown future job"));
    }

    #[test]
    fn deep_merkle_path_folds_without_reversal() {
        let hash = |data: &[u8]| bitcoin::hashes::sha256d::Hash::hash(data).to_byte_array();
        let coinbase = hash(b"coinbase");
        let first = [0x11; 32];
        let second_input = [&[0x22; 32][..], &[0x33; 32][..]].concat();
        let second = hash(&second_input);
        let top_input = [
            &hash(&[&coinbase[..], &first[..]].concat())[..],
            &second[..],
        ]
        .concat();
        let expected = hash(&top_input);

        // A fake header tag makes the exact root observable without PoW. The
        // independent expected tree above has two distinct levels.
        let mut header = Vec::new();
        header.extend_from_slice(&0u32.to_le_bytes());
        header.extend_from_slice(&[0u8; 32]);
        header.extend_from_slice(&expected);
        header.extend_from_slice(&0u32.to_le_bytes());
        header.extend_from_slice(&0u32.to_le_bytes());
        header.extend_from_slice(&0u32.to_le_bytes());
        let expected_hash = hash(&header);
        assert_eq!(
            share_hash(0, [0; 32], b"coinbase", &[first, second], 0, 0, 0),
            expected_hash
        );
    }

    #[test]
    fn target_wire_order_is_little_endian() {
        let mut bytes = [0u8; 32];
        bytes[0] = 0x80;
        bytes[31] = 1;
        let target = target_from_wire(&bytes).unwrap();
        assert_eq!(target.to_le_bytes(), bytes);
        assert_ne!(target.to_be_bytes(), bytes);
    }

    #[test]
    fn new_extended_job_shape_supports_session_vectors() {
        // Keep the upstream codec's empty-vs-present min_ntime conversion in
        // this module's vector tests rather than testing only our own boolean.
        let message = NewExtendedMiningJob {
            channel_id: 1,
            job_id: 7,
            min_ntime: Sv2Option::new(None),
            version: 0x2000_0000,
            version_rolling_allowed: true,
            merkle_path: vec![].try_into().unwrap(),
            coinbase_tx_prefix: B064K::new(&[1]).unwrap(),
            coinbase_tx_suffix: B064K::new(&[2]).unwrap(),
        };
        assert_eq!(parse_job(message).min_ntime, None);
    }
}
