use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use bitcoin::hashes::Hash as _;
use bitcoin::BlockHash;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;

use crate::budget::HashBudget;
use crate::control::CommandResult;
use crate::ids::{ConnectionId, IdentityId, JobId, MinerId, WorkerId};
use crate::session::{JobTemplate, PendingJob};
use crate::target::target_le_hex;

pub const EVENT_VERSION: u16 = 1;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubmissionStatus {
    Submitted,
    IngressAck,
    IngressRejected,
    TransportUnknown,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubmissionSnapshot {
    pub connection_id: ConnectionId,
    pub worker_id: WorkerId,
    pub session_slot: u16,
    pub sequence_number: u32,
    pub job_id: JobId,
    pub target_generation: u64,
    pub nonce: u32,
    pub ntime: u32,
    pub version: u32,
    pub extranonce: String,
    pub header: String,
    pub block_hash: String,
    pub hash_le: String,
    pub share_target_le: String,
    pub network_target_le: String,
    pub block_qualified: bool,
    pub status: SubmissionStatus,
    /// The SV2 success reply is ingress evidence only. No local accounting
    /// inference is made, so this stays unknown for the life of this process.
    pub accounted: String,
    /// A block-qualified share is not itself a pool-to-node candidate proof.
    /// The candidate state is unknown until an independent pool observer sees it.
    pub candidate_submission: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

impl SubmissionSnapshot {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        connection_id: &ConnectionId,
        worker_id: &WorkerId,
        session_slot: u16,
        sequence_number: u32,
        job: &JobTemplate,
        header: [u8; 80],
        hash: BlockHash,
        extranonce: &[u8],
        nonce: u32,
        ntime: u32,
        version: u32,
    ) -> Self {
        let block_qualified = crate::target::meets_target(&hash, job.network_target);
        Self {
            connection_id: connection_id.clone(),
            worker_id: worker_id.clone(),
            session_slot,
            sequence_number,
            job_id: job.job_id,
            target_generation: job.target_generation,
            nonce,
            ntime,
            version,
            extranonce: hex::encode(extranonce),
            header: hex::encode(header),
            block_hash: hash.to_string(),
            hash_le: hex::encode(hash.to_byte_array()),
            share_target_le: target_le_hex(job.share_target),
            network_target_le: target_le_hex(job.network_target),
            block_qualified,
            status: SubmissionStatus::Submitted,
            accounted: "unknown".into(),
            candidate_submission: if block_qualified {
                "unknown"
            } else {
                "not_applicable"
            }
            .into(),
            error_code: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobSnapshot {
    pub job_id: JobId,
    pub target_generation: u64,
    pub version: u32,
    pub version_rolling_allowed: bool,
    pub prev_hash_le: String,
    pub min_ntime: u32,
    pub nbits: u32,
    pub share_target_le: String,
    pub network_target_le: String,
    pub merkle_path_le: Vec<String>,
    pub coinbase_tx_prefix: String,
    pub coinbase_tx_suffix: String,
    pub extranonce_prefix: String,
}

impl From<&JobTemplate> for JobSnapshot {
    fn from(job: &JobTemplate) -> Self {
        Self {
            job_id: job.job_id,
            target_generation: job.target_generation,
            version: job.version,
            version_rolling_allowed: job.version_rolling_allowed,
            prev_hash_le: hex::encode(job.prev_hash),
            min_ntime: job.min_ntime,
            nbits: job.nbits,
            share_target_le: target_le_hex(job.share_target),
            network_target_le: target_le_hex(job.network_target),
            merkle_path_le: job.merkle_path.iter().map(hex::encode).collect(),
            coinbase_tx_prefix: hex::encode(&job.coinbase_tx_prefix),
            coinbase_tx_suffix: hex::encode(&job.coinbase_tx_suffix),
            extranonce_prefix: hex::encode(&job.extranonce_prefix),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FutureJobSnapshot {
    pub job_id: JobId,
    pub version: u32,
    pub coinbase_tx_prefix: String,
    pub coinbase_tx_suffix: String,
    pub extranonce_prefix: String,
}

impl From<&PendingJob> for FutureJobSnapshot {
    fn from(job: &PendingJob) -> Self {
        Self {
            job_id: job.job_id,
            version: job.version,
            coinbase_tx_prefix: hex::encode(&job.coinbase_tx_prefix),
            coinbase_tx_suffix: hex::encode(&job.coinbase_tx_suffix),
            extranonce_prefix: hex::encode(&job.extranonce_prefix),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ConnectionCounters {
    pub submitted: u64,
    pub ingress_success_replies: u64,
    pub ingress_accepted_reported: u64,
    pub ingress_rejected: u64,
    pub block_qualified_submissions: u64,
    pub reconnects: u64,
    pub stale_cancelled: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConnectionSnapshot {
    pub id: ConnectionId,
    pub endpoint: String,
    pub identity: IdentityId,
    pub state: String,
    pub detail: Option<String>,
    pub session_slot: Option<u16>,
    pub channel_id: Option<u32>,
    pub group_channel_id: Option<u32>,
    pub extranonce_size: Option<u16>,
    pub channel_target_le: Option<String>,
    pub target_generation: u64,
    pub active_job: Option<JobSnapshot>,
    #[serde(default)]
    pub candidate_job: Option<JobSnapshot>,
    #[serde(default)]
    pub source_audit_state: Option<String>,
    #[serde(default)]
    pub source_audit_ready: bool,
    pub future_jobs: Vec<FutureJobSnapshot>,
    pub counters: ConnectionCounters,
    pub submissions: VecDeque<SubmissionSnapshot>,
}

impl ConnectionSnapshot {
    pub fn new(id: &ConnectionId, endpoint: &str, identity: &IdentityId) -> Self {
        Self {
            id: id.clone(),
            endpoint: endpoint.into(),
            identity: identity.clone(),
            state: "added".into(),
            detail: None,
            session_slot: None,
            channel_id: None,
            group_channel_id: None,
            extranonce_size: None,
            channel_target_le: None,
            target_generation: 0,
            active_job: None,
            candidate_job: None,
            source_audit_state: None,
            source_audit_ready: false,
            future_jobs: Vec::new(),
            counters: ConnectionCounters::default(),
            submissions: VecDeque::new(),
        }
    }

    pub fn clear_jobs(&mut self) {
        self.channel_target_le = None;
        self.target_generation = 0;
        self.active_job = None;
        self.candidate_job = None;
        self.source_audit_state = None;
        self.source_audit_ready = false;
        self.future_jobs.clear();
    }

    /// Retain bounded known history and explicitly return evictions. Callers
    /// distinguish resolved evictions from still-pending unknown submissions.
    pub fn push_submission(&mut self, share: SubmissionSnapshot) -> Option<SubmissionSnapshot> {
        let evicted = if self.submissions.len() == 256 {
            self.submissions.pop_front()
        } else {
            None
        };
        self.submissions.push_back(share);
        evicted
    }

    pub fn update_submission(
        &mut self,
        session_slot: u16,
        sequence: u32,
        update: impl FnOnce(&mut SubmissionSnapshot),
    ) -> bool {
        if let Some(share) =
            self.submissions.iter_mut().rev().find(|share| {
                share.session_slot == session_slot && share.sequence_number == sequence
            })
        {
            update(share);
            true
        } else {
            false
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkerSnapshot {
    pub id: WorkerId,
    pub connection_id: ConnectionId,
    pub paused: bool,
    pub requested: HashBudget,
    pub attempted: String,
    pub elapsed_ms: u64,
    pub achieved_hps: u64,
    pub budget_hps: Option<u64>,
    pub saturated: bool,
    pub queue_depth: u64,
    pub active_job_id: Option<JobId>,
    pub stale_cancelled: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MinerSnapshot {
    pub snapshot_version: u16,
    pub miner_id: MinerId,
    pub occurred_at_mono_ms: u64,
    pub events_dropped_total: u64,
    pub stopping: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_evidence: Option<crate::evidence::CaptureInfo>,
    pub connections: Vec<ConnectionSnapshot>,
    pub workers: Vec<WorkerSnapshot>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "name", content = "payload", rename_all = "snake_case")]
pub enum MinerEvent {
    JobCandidate {
        connection_id: ConnectionId,
        session_slot: u16,
        candidate_epoch: u64,
        request_digest: String,
        job: JobSnapshot,
    },
    SourceAuditCommitted {
        connection_id: ConnectionId,
        session_slot: u16,
        candidate_epoch: u64,
        capability: String,
        job_id: u64,
        generation: u64,
        request_digest: String,
        checkpoint_digest: String,
        consumed_digest: String,
        committed_digest: String,
        expires_at_unix_ms: u64,
        window: spc_scan::source_audit::WindowFacts,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        durable_evidence: Option<spc_scan::native_evidence::DurableHandle>,
    },
    SourceAuditUnknown {
        connection_id: ConnectionId,
        session_slot: u16,
        candidate_epoch: u64,
        code: String,
    },
    AuditFirstHash {
        connection_id: ConnectionId,
        worker_id: WorkerId,
        session_slot: u16,
        candidate_epoch: u64,
        request_digest: String,
        job_id: JobId,
    },
    ExplicitUnaudited {
        connection_id: ConnectionId,
    },
    ConnectionState {
        connection_id: ConnectionId,
        session_slot: Option<u16>,
        state: String,
        detail: Option<String>,
    },
    TargetUpdated {
        connection_id: ConnectionId,
        target_generation: u64,
        share_target_le: String,
    },
    JobActivated {
        connection_id: ConnectionId,
        job: JobSnapshot,
    },
    JobsInvalidated {
        connection_id: ConnectionId,
        job_ids: Vec<JobId>,
        reason: String,
    },
    Submitted {
        share: SubmissionSnapshot,
    },
    IngressAck {
        connection_id: ConnectionId,
        sequence_number: u32,
        accepted_count: u32,
        new_shares_sum: u64,
    },
    IngressRejected {
        connection_id: ConnectionId,
        sequence_number: u32,
        error_code: String,
    },
    SubmissionsUnknown {
        connection_id: ConnectionId,
        sequence_numbers: Vec<u32>,
        reason: String,
    },
    SubmissionsEvicted {
        connection_id: ConnectionId,
        sequence_numbers: Vec<u32>,
        reason: String,
    },
    LateVerdict {
        connection_id: ConnectionId,
        sequence_numbers: Vec<u32>,
        outcome: String,
    },
    BlockQualified {
        connection_id: ConnectionId,
        sequence_number: u32,
        block_hash: String,
        candidate_submission: String,
    },
    WorkerSearchExhausted {
        connection_id: ConnectionId,
        worker_id: WorkerId,
        job_id: JobId,
        ntime: u32,
        reason: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub protocol: String,
    pub version: u16,
    pub kind: String,
    pub source: String,
    pub event_id: u64,
    pub source_cursor: u64,
    pub miner_id: MinerId,
    pub occurred_at_wall_ms: u64,
    pub occurred_at_mono_ms: u64,
    pub dropped_before: u64,
    pub event: MinerEvent,
}

#[derive(Debug)]
pub enum OutputMessage {
    Event(Box<EventEnvelope>),
    Command(CommandResult),
}

#[derive(Debug)]
struct PublicationState {
    next_id: u64,
    dropped: u64,
    ids_exhausted: bool,
}

#[derive(Clone, Debug)]
pub struct EventSink {
    tx: mpsc::Sender<OutputMessage>,
    miner_id: MinerId,
    publication: Arc<Mutex<PublicationState>>,
    started: Instant,
}

impl EventSink {
    pub fn new(tx: mpsc::Sender<OutputMessage>, miner_id: MinerId, started: Instant) -> Self {
        Self {
            tx,
            miner_id,
            publication: Arc::new(Mutex::new(PublicationState {
                next_id: 1,
                dropped: 0,
                ids_exhausted: false,
            })),
            started,
        }
    }

    fn publication(&self) -> MutexGuard<'_, PublicationState> {
        match self.publication.lock() {
            Ok(guard) => guard,
            // Publication must remain coherent even after a producer panic.
            // No consumer or stdout I/O is performed while this guard is held.
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    pub fn emit(&self, event: MinerEvent) {
        let _ = self.try_emit(event);
    }

    /// Native activation needs a successful nonblocking enqueue before work
    /// publication. Other telemetry keeps its existing loss accounting.
    pub fn try_emit(&self, event: MinerEvent) -> bool {
        let occurred_at_wall_ms = unix_ms();
        let occurred_at_mono_ms = self.mono_ms();
        let miner_id = self.miner_id.clone();
        // One critical section serializes the allocation cursor, cumulative
        // loss count and bounded enqueue. `try_send` remains nonblocking; the
        // queue still drops rather than applying consumer backpressure.
        let mut state = self.publication();
        if state.ids_exhausted {
            state.dropped = state.dropped.saturating_add(1);
            return false;
        }
        let event_id = state.next_id;
        if event_id == u64::MAX {
            state.ids_exhausted = true;
        } else {
            state.next_id += 1;
        }
        let envelope = EventEnvelope {
            protocol: "spc-cpu-miner-events".into(),
            version: EVENT_VERSION,
            kind: "event".into(),
            source: "process".into(),
            event_id,
            source_cursor: event_id,
            miner_id,
            occurred_at_wall_ms,
            occurred_at_mono_ms,
            dropped_before: state.dropped,
            event,
        };
        if self
            .tx
            .try_send(OutputMessage::Event(Box::new(envelope)))
            .is_err()
        {
            state.dropped = state.dropped.saturating_add(1);
            return false;
        }
        true
    }

    pub fn dropped(&self) -> u64 {
        self.publication().dropped
    }

    pub fn mono_ms(&self) -> u64 {
        self.started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
    }
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

pub async fn write_output<W>(mut rx: mpsc::Receiver<OutputMessage>, mut writer: W)
where
    W: AsyncWrite + Unpin,
{
    while let Some(message) = rx.recv().await {
        let line = match message {
            OutputMessage::Event(event) => serde_json::to_vec(&event),
            OutputMessage::Command(command) => serde_json::to_vec(&command),
        };
        let Ok(mut line) = line else { return };
        line.push(b'\n');
        if writer.write_all(&line).await.is_err() || writer.flush().await.is_err() {
            return;
        }
    }
}

pub type SharedConnectionSnapshot = Arc<Mutex<ConnectionSnapshot>>;
pub type SnapshotMap = BTreeMap<ConnectionId, SharedConnectionSnapshot>;
