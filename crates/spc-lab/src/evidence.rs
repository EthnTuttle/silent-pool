//! Append-only evidence, distinct evidence states and resumable snapshots.
//!
//! Missing evidence is never converted to "invalid". A canonical observation
//! is time-scoped to one node and one anchored tip; a later reorg adds history
//! rather than deleting it. A slow observer receives a gap, never a blocked
//! producer.

use std::collections::{BTreeMap, VecDeque};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::sync::mpsc::{Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::{
    CommandId, FrontId, IdentityId, JobId, MinerId, NodeId, RunId, ShareId, SourceId,
};
use crate::scenario::{AssertionKind, AssertionSpec};
use crate::wide::WideU128;

pub type EventId = u64;
pub type SectionId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    Harness,
    Node,
    Pool,
    Front,
    Miner,
    Observer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceActor {
    pub kind: ActorKind,
    pub id: String,
}

impl EvidenceActor {
    pub fn harness() -> Self {
        Self {
            kind: ActorKind::Harness,
            id: "harness".to_string(),
        }
    }

    pub fn new(kind: ActorKind, id: impl Into<String>) -> Self {
        Self {
            kind,
            id: id.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceEvent {
    pub run_id: RunId,
    pub event_id: EventId,
    pub source: SourceId,
    pub source_cursor: String,
    pub actor: EvidenceActor,
    #[serde(default)]
    pub command_id: Option<CommandId>,
    pub occurred_at_wall: u64,
    pub occurred_at_mono: u64,
    pub ingested_at: u64,
    pub payload: EvidencePayload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobActivation {
    Future,
    Active,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalResult {
    Yes,
    No,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobIssued {
    pub front_id: FrontId,
    pub channel_id: u64,
    pub job_id: JobId,
    pub generation: u64,
    pub nbits: u32,
    pub share_target: String,
    pub m_issue: u64,
    pub work_unit_version: u16,
    pub work_unit_fingerprint: String,
    pub issued_at: u64,
    pub activation: JobActivation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareHeader {
    pub version: u32,
    pub prev_hash: String,
    pub merkle_root: String,
    pub ntime: u32,
    pub nbits: u32,
    pub nonce: u32,
}

impl ShareHeader {
    pub fn serialize(&self) -> Result<[u8; 80], EvidenceError> {
        let prev = decode_reversed_hex(&self.prev_hash)?;
        let merkle = decode_reversed_hex(&self.merkle_root)?;
        let mut out = [0u8; 80];
        out[0..4].copy_from_slice(&self.version.to_le_bytes());
        out[4..36].copy_from_slice(&prev);
        out[36..68].copy_from_slice(&merkle);
        out[68..72].copy_from_slice(&self.ntime.to_le_bytes());
        out[72..76].copy_from_slice(&self.nbits.to_le_bytes());
        out[76..80].copy_from_slice(&self.nonce.to_le_bytes());
        Ok(out)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Submitted {
    pub miner_id: MinerId,
    pub identity_id: IdentityId,
    pub share_id: ShareId,
    pub job_id: JobId,
    pub generation: u64,
    pub header: ShareHeader,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IngressAck {
    pub front_id: FrontId,
    pub share_id: ShareId,
    pub job_id: JobId,
    pub sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IngressRejected {
    pub front_id: FrontId,
    pub share_id: ShareId,
    pub code: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Accounted {
    pub front_id: FrontId,
    pub identity_id: IdentityId,
    pub share_id: ShareId,
    pub job_id: JobId,
    pub generation: u64,
    pub nbits: u32,
    pub share_target: String,
    pub d: WideU128,
    pub slice_id: String,
    pub diff_sum: WideU128,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountingRejected {
    pub front_id: FrontId,
    pub share_id: ShareId,
    pub code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub pool_id: String,
    pub candidate_id: String,
    pub block_hash: String,
    pub height: u64,
    pub job_id: JobId,
    pub share_id: ShareId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalAt {
    pub node_id: NodeId,
    pub block_hash: String,
    pub height: u64,
    pub anchored_tip: String,
    pub observed_at: u64,
    pub source: SourceId,
    pub result: CanonicalResult,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeRejected {
    pub node_id: NodeId,
    pub block_hash: String,
    pub height: u64,
    pub reason: String,
    pub observed_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settled {
    pub block_hash: String,
    pub height: u64,
    pub settlement_id: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Delivered {
    pub identity_id: IdentityId,
    pub block_hash: String,
    pub amount_sats: WideU128,
    pub detail: String,
}

pub const TOPOLOGY_EVIDENCE_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControllerObservation {
    pub reachable: Option<bool>,
    pub url: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedChainAnchor {
    pub height: u64,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AtomicTipObservation {
    pub node: String,
    pub height: Option<u64>,
    pub hash: Option<String>,
    /// The actual `previousblockhash` from the header response, when present.
    /// This is not used to infer a fork from height differences.
    pub parent_hash: Option<String>,
    pub atomic: bool,
    pub peers: Option<usize>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrontApiObservation {
    pub front: FrontId,
    pub reachable: bool,
    pub response_bytes: Option<usize>,
    pub error: Option<String>,
}

/// A bounded broker call: either a decoded JSON answer or a concrete unknown
/// reason. `ok == false` never gets erased into a null policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerCallObservation {
    pub ok: bool,
    pub value: Option<Value>,
    pub error: Option<String>,
}

impl BrokerCallObservation {
    pub fn success(value: Value) -> Self {
        Self {
            ok: true,
            value: Some(value),
            error: None,
        }
    }

    pub fn failure(error: impl Into<String>) -> Self {
        Self {
            ok: false,
            value: None,
            error: Some(error.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsensusCounterObservation {
    pub link: String,
    pub counters: BrokerCallObservation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerEvidenceObservation {
    pub counters: Option<BrokerCallObservation>,
    pub no_bypass: Option<BrokerCallObservation>,
    #[serde(default)]
    pub consensus_counters: Vec<ConsensusCounterObservation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TopologyPhaseEvidence {
    pub schema_version: u16,
    pub phase: String,
    pub outcome: crate::command::CommandOutcome,
    pub link: Option<String>,
    pub pre_partition_common_anchor: Option<SharedChainAnchor>,
    pub controller: ControllerObservation,
    pub nodes: Vec<AtomicTipObservation>,
    pub fronts: Vec<FrontApiObservation>,
    pub broker_evidence: Option<BrokerEvidenceObservation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TopologyConvergenceEvidence {
    pub schema_version: u16,
    pub phase: String,
    pub status: String,
    pub attempts: u32,
    pub pre_partition_common_anchor: Option<SharedChainAnchor>,
    pub views: Vec<AtomicTipObservation>,
    pub controls: Vec<Value>,
    pub policy: crate::scenario::ConvergenceSpec,
    pub unknown_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnknownEvidence {
    pub topic: String,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GapScope {
    /// A durable store write/parse/loss event; assertions are inconclusive.
    Store,
    /// One observer's queue overflow; the durable store is intact.
    Subscriber,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gap {
    pub scope: GapScope,
    pub subscriber: u64,
    pub from_event_id: EventId,
    pub to_event_id: EventId,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessObservation {
    pub role: String,
    pub entity: String,
    pub state: String,
    pub pid: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandObservation {
    pub command_id: CommandId,
    pub state: String,
    pub detail: String,
    #[serde(default)]
    pub journal_seq: u64,
}

/// One raw LISTEN-row observation at an exact planned address: the raw
/// kernel state field, the socket inode and whether that inode was
/// open by the node process's fd table at that bracket. This is the
/// raw ownership evidence; the boolean alone is not the proof.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListenerObservation {
    /// Raw kernel TCP state of the LISTEN row (`0A`).
    pub state: String,
    /// The LISTEN socket's inode at the exact planned address.
    pub inode: u64,
    /// The inode was open by the node process's fd table (raw
    /// membership as read).
    pub fd_membership: bool,
}

/// A node's raw P2P and RPC listener evidence (contract version 3):
/// the exact owned LISTEN address plus the raw row/inode/fd-membership
/// observation BEFORE and AFTER the peer samples. Both brackets must
/// confirm ownership; `None` marks an observation that could not be
/// made (fail closed by the gate, recorded for diagnosis).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeListenerEvidence {
    /// The node's bound RPC control listener address.
    pub rpc_listen: String,
    /// The node's planned P2P listener address.
    pub p2p_listen: String,
    pub rpc_before: Option<ListenerObservation>,
    pub rpc_after: Option<ListenerObservation>,
    pub p2p_before: Option<ListenerObservation>,
    pub p2p_after: Option<ListenerObservation>,
}

/// A node's public, nonsecret process identity bindings for the
/// readiness brackets (contract version 3): the engine run, the
/// supervisor-recorded PID/start-ticks/executable binding, the boot
/// id, and the observed namespace and executable identities that the
/// brackets proved stable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeProcessBinding {
    /// The engine run this observation belongs to.
    pub run_id: String,
    pub pid: u32,
    pub start_time_ticks: u64,
    /// The supervisor-recorded executable path.
    pub executable_path: String,
    /// The supervisor-recorded executable sha256 (the launch/source
    /// generation binding; no run tokens are persisted).
    pub executable_sha256: String,
    /// The observed `/proc/<pid>/exe` (stable across the brackets).
    pub observed_exe: Option<String>,
    /// The observed netns identity `(st_dev, st_ino)`.
    pub netns_identity: Option<[u64; 2]>,
    /// The host controller's observed netns identity.
    pub host_netns_identity: Option<[u64; 2]>,
    pub boot_id: Option<String>,
}

/// One node's observed P2P peer state at the bootstrap readiness gate.
/// `listener_owned` carries the exact-address file-descriptor proof
/// summary; `listener_evidence` carries the raw rows. `peer_samples`
/// carries both resampled getpeerinfo reads with their after-read
/// stamps (contract version 3: raw RPC handshake fields are persisted,
/// never inferred).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeNetworkReadinessNode {
    pub node: crate::ids::NodeId,
    /// Peer addresses this node dialed (getpeerinfo inbound=false,
    /// sample B).
    pub outbound: Vec<String>,
    /// Peer addresses this node accepted (getpeerinfo inbound=true,
    /// sample B).
    pub inbound: Vec<String>,
    #[serde(default)]
    pub listener_owned: Option<bool>,
    /// The two resampled peer reads (A after the before bracket, B
    /// before the after bracket) with their after-read monotonic
    /// stamps.
    pub peer_samples: spc_lab_txd::net_readiness::PeerDualSample,
    /// Raw listener ownership evidence, before and after.
    pub listener_evidence: NodeListenerEvidence,
    /// Public nonsecret process identity bindings.
    pub process_binding: NodeProcessBinding,
}

/// Observed-connectivity evidence recorded BEFORE any bootstrap hash budget
/// is reserved: the planned peer graph (from the recorded launch profiles)
/// against what the nodes actually report, with the per-directed-dial
/// `receiver_handshake_with_owned_reverse_tcp` proofs when the gate passed.
///
/// Contract version 3 reports carry the whole-acquisition clock record
/// (start, never-reset deadline, after-reads completion stamp and the
/// source-declared bounds). Reports of earlier contract versions fail
/// to parse against these types; they never satisfy a version 3
/// consumer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeNetworkReadinessReport {
    pub status: String,
    pub detail: Option<String>,
    /// Observational contract version of the readiness evidence
    /// (3 = receiver-handshake with owned reverse TCP, whole-acquisition
    /// deadline and raw bracket/namespace evidence).
    #[serde(default)]
    pub contract_version: u32,
    /// The evidence form name of the proofs in this report.
    #[serde(default)]
    pub evidence_form: Option<String>,
    /// The whole-acquisition clock record (contract version 3:
    /// mandatory; earlier versions fail to parse).
    pub acquisition: spc_lab_txd::net_readiness::ReadinessAcquisitionRecord,
    /// Planned undirected edges as sorted node-id pairs.
    pub planned_edges: Vec<(crate::ids::NodeId, crate::ids::NodeId)>,
    /// Planned edges covered by at least one proven directed dial.
    pub covered_edges: Vec<(crate::ids::NodeId, crate::ids::NodeId)>,
    /// Planned directed dials (dialer, acceptor): each is an independent
    /// obligation.
    #[serde(default)]
    pub planned_dials: Vec<(crate::ids::NodeId, crate::ids::NodeId)>,
    /// Directed dials proven by a complete typed proof.
    #[serde(default)]
    pub covered_dials: Vec<(crate::ids::NodeId, crate::ids::NodeId)>,
    pub nodes: Vec<NodeNetworkReadinessNode>,
    /// The public proof records (empty when the gate did not pass).
    #[serde(default)]
    pub proofs: Vec<spc_lab_txd::net_readiness::DirectedDialProof>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum EvidencePayload {
    TrafficStage {
        stage: crate::traffic::runtime::TrafficStage,
    },
    TrafficMeasured {
        boundary: crate::traffic::runtime::MeasuredBoundary,
    },
    NodeNetworkReadiness {
        status: String,
        report: NodeNetworkReadinessReport,
    },
    TxTraffic {
        driver_id: crate::ids::ActorId,
        incarnation: String,
        observation: spc_lab_txd::control::TrafficObservation,
    },
    JobIssued(JobIssued),
    Submitted(Submitted),
    IngressAck(IngressAck),
    IngressRejected(IngressRejected),
    Accounted(Accounted),
    AccountingRejected(AccountingRejected),
    Candidate(Candidate),
    CanonicalAt(CanonicalAt),
    NodeRejected(NodeRejected),
    Settled(Settled),
    Delivered(Delivered),
    TopologyPhase(TopologyPhaseEvidence),
    TopologyConvergence(TopologyConvergenceEvidence),
    Unknown(UnknownEvidence),
    EventLost(Gap),
    Inconclusive {
        reason: String,
    },
    RunState {
        state: String,
    },
    Process(ProcessObservation),
    Command(CommandObservation),
    LinkPolicy {
        link_id: String,
        policy: String,
    },
    SpcEvidence(crate::spc_evidence::SpcEvidenceRecord),
    MinerEvent {
        miner_id: MinerId,
        incarnation: String,
        driver_event_id: u64,
        name: String,
        payload: serde_json::Value,
        dropped_before: u64,
    },
}

impl EvidencePayload {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::JobIssued(_) => "job_issued",
            Self::Submitted(_) => "submitted",
            Self::IngressAck(_) => "ingress_ack",
            Self::IngressRejected(_) => "ingress_rejected",
            Self::Accounted(_) => "accounted",
            Self::AccountingRejected(_) => "accounting_rejected",
            Self::Candidate(_) => "candidate",
            Self::CanonicalAt(_) => "canonical_at",
            Self::NodeRejected(_) => "node_rejected",
            Self::Settled(_) => "settled",
            Self::Delivered(_) => "delivered",
            Self::TopologyPhase(_) => "topology_phase",
            Self::TopologyConvergence(_) => "topology_convergence",
            Self::Unknown(_) => "unknown",
            Self::EventLost(_) => "event_lost",
            Self::Inconclusive { .. } => "inconclusive",
            Self::RunState { .. } => "run_state",
            Self::Process(_) => "process",
            Self::Command(_) => "command",
            Self::LinkPolicy { .. } => "link_policy",
            Self::MinerEvent { .. } => "miner_event",
            Self::TxTraffic { .. } => "tx_traffic",
            Self::TrafficStage { .. } => "traffic_stage",
            Self::TrafficMeasured { .. } => "traffic_measured_from",
            Self::NodeNetworkReadiness { .. } => "node_network_readiness",
            Self::SpcEvidence(_) => "spc_evidence",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntitySnapshot {
    pub kind: String,
    pub id: String,
    pub status: String,
    pub detail: String,
    /// Present on worker rows so the browser can send a worker command with
    /// both the owning miner and the worker ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub miner_id: Option<MinerId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Watermark {
    pub source: SourceId,
    pub last_event_id: EventId,
    pub last_event_kind: String,
    pub last_event_wall: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceSummary {
    pub total_events: u64,
    pub counts: BTreeMap<String, u64>,
    pub gap_count: u64,
    pub last_event_id: EventId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub run_id: RunId,
    pub cursor: EventId,
    pub retained_from: EventId,
    pub resync_required: bool,
    pub snapshot_cursor: EventId,
    pub generated_at: u64,
    pub entities: Vec<EntitySnapshot>,
    pub watermarks: Vec<Watermark>,
    pub gaps: Vec<Gap>,
    pub evidence: EvidenceSummary,
}

#[derive(Debug)]
pub struct Subscription {
    pub replay: Vec<Arc<EvidenceEvent>>,
    pub receiver: Receiver<Arc<EvidenceEvent>>,
    pub cursor: EventId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SubscribeResult {
    Replay {
        cursor: EventId,
        retained_from: EventId,
    },
    ResyncRequired {
        cursor: EventId,
        retained_from: EventId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssertionEvaluation {
    Passed,
    Failed(String),
    Inconclusive(String),
    BlockedOnHeight(String),
    TimedOut(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurrentStatus {
    pub node_id: NodeId,
    pub block_hash: String,
    pub result: CanonicalResult,
    pub observed_at: u64,
    pub anchored_tip: String,
    pub detail: String,
    pub history: Vec<CanonicalAt>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EvidenceError {
    #[error("bad hex field {0:?}")]
    BadHex(String),
    #[error("evidence io: {0}")]
    Io(String),
}

struct Subscriber {
    id: u64,
    tx: SyncSender<Arc<EvidenceEvent>>,
    last_sent: EventId,
}

struct State {
    next_id: EventId,
    events: VecDeque<Arc<EvidenceEvent>>,
    subscribers: Vec<Subscriber>,
    next_subscriber: u64,
    gaps: Vec<Gap>,
    writer: Option<File>,
    capacity: usize,
}

pub struct EvidenceStore {
    run_id: RunId,
    state: Mutex<State>,
}

impl EvidenceStore {
    pub fn new(run_id: RunId, capacity: usize) -> Self {
        Self {
            run_id,
            state: Mutex::new(State {
                next_id: 1,
                events: VecDeque::new(),
                subscribers: Vec::new(),
                next_subscriber: 1,
                gaps: Vec::new(),
                writer: None,
                capacity: capacity.max(1),
            }),
        }
    }

    pub fn open(
        run_id: RunId,
        path: impl AsRef<Path>,
        capacity: usize,
    ) -> Result<Self, EvidenceError> {
        let store = Self::new(run_id, capacity);
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent).map_err(|e| EvidenceError::Io(e.to_string()))?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(path.as_ref())
            .map_err(|e| EvidenceError::Io(e.to_string()))?;
        let _ = std::fs::set_permissions(path.as_ref(), std::fs::Permissions::from_mode(0o600));
        store.state.lock().unwrap().writer = Some(file);
        Ok(store)
    }

    pub fn load(
        run_id: RunId,
        path: impl AsRef<Path>,
        capacity: usize,
    ) -> Result<Self, EvidenceError> {
        let store = Self::new(run_id, capacity);
        if path.as_ref().exists() {
            let file = File::open(path.as_ref()).map_err(|e| EvidenceError::Io(e.to_string()))?;
            let mut state = store.state.lock().unwrap();
            let mut loaded = VecDeque::new();
            for line in std::io::BufReader::new(file).lines() {
                let line = line.map_err(|e| EvidenceError::Io(e.to_string()))?;
                if line.trim().is_empty() {
                    continue;
                }
                // A torn final line after a crash is not evidence.
                if let Ok(event) = serde_json::from_str::<EvidenceEvent>(&line) {
                    state.next_id = state.next_id.max(event.event_id + 1);
                    loaded.push_back(Arc::new(event));
                }
            }
            while loaded.len() > state.capacity {
                loaded.pop_front();
            }
            state.events = loaded;
            let writer = OpenOptions::new()
                .create(true)
                .append(true)
                .mode(0o600)
                .open(path.as_ref())
                .map_err(|e| EvidenceError::Io(e.to_string()))?;
            let _ = std::fs::set_permissions(path.as_ref(), std::fs::Permissions::from_mode(0o600));
            state.writer = Some(writer);
        } else {
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .mode(0o600)
                .open(path.as_ref())
                .map_err(|e| EvidenceError::Io(e.to_string()))?;
            let _ = std::fs::set_permissions(path.as_ref(), std::fs::Permissions::from_mode(0o600));
            store.state.lock().unwrap().writer = Some(file);
        }
        Ok(store)
    }

    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    pub fn append(
        &self,
        source: SourceId,
        source_cursor: impl Into<String>,
        actor: EvidenceActor,
        command_id: Option<CommandId>,
        payload: EvidencePayload,
    ) -> EventId {
        self.append_at(source, source_cursor, actor, command_id, payload, now_ms())
    }

    pub fn append_at(
        &self,
        source: SourceId,
        source_cursor: impl Into<String>,
        actor: EvidenceActor,
        command_id: Option<CommandId>,
        payload: EvidencePayload,
        occurred_at_wall: u64,
    ) -> EventId {
        let mut state = self.state.lock().unwrap();
        append_locked(
            &mut state,
            &self.run_id,
            source,
            source_cursor.into(),
            actor,
            command_id,
            payload,
            occurred_at_wall,
        )
    }

    pub fn last_event_id(&self) -> EventId {
        self.state.lock().unwrap().next_id.saturating_sub(1)
    }

    pub fn retained_from(&self) -> EventId {
        let state = self.state.lock().unwrap();
        state
            .events
            .front()
            .map(|event| event.event_id)
            .unwrap_or(state.next_id)
    }

    pub fn events(&self) -> Vec<Arc<EvidenceEvent>> {
        self.state.lock().unwrap().events.iter().cloned().collect()
    }

    pub fn gaps(&self) -> Vec<Gap> {
        self.state.lock().unwrap().gaps.clone()
    }

    pub fn snapshot(&self, cursor: Option<EventId>, entities: Vec<EntitySnapshot>) -> Snapshot {
        self.snapshot_with(cursor, || entities)
    }

    /// Capture the cursor and entity projection under one evidence lock. A
    /// client can therefore replay from `snapshot_cursor` without an event
    /// slipping between the cursor capture and the entity read.
    pub fn snapshot_with<F>(&self, cursor: Option<EventId>, entities: F) -> Snapshot
    where
        F: FnOnce() -> Vec<EntitySnapshot>,
    {
        let state = self.state.lock().unwrap();
        let retained_from = state
            .events
            .front()
            .map(|event| event.event_id)
            .unwrap_or(state.next_id);
        let last = state.next_id.saturating_sub(1);
        let requested = cursor.unwrap_or(0);
        let resync_required =
            requested != 0 && (requested < retained_from.saturating_sub(1) || requested > last);
        let entities = entities();
        let mut counts: BTreeMap<String, u64> = BTreeMap::new();
        let mut watermarks: BTreeMap<SourceId, Watermark> = BTreeMap::new();
        for event in &state.events {
            *counts.entry(event.payload.kind().to_string()).or_insert(0) += 1;
            watermarks.insert(
                event.source.clone(),
                Watermark {
                    source: event.source.clone(),
                    last_event_id: event.event_id,
                    last_event_kind: event.payload.kind().to_string(),
                    last_event_wall: event.occurred_at_wall,
                },
            );
        }
        Snapshot {
            run_id: self.run_id.clone(),
            cursor: last,
            retained_from,
            resync_required,
            snapshot_cursor: last,
            generated_at: now_ms(),
            entities,
            watermarks: watermarks.into_values().collect(),
            gaps: state.gaps.clone(),
            evidence: EvidenceSummary {
                total_events: last,
                counts,
                gap_count: state.gaps.len() as u64,
                last_event_id: last,
            },
        }
    }

    pub fn durable_gaps(&self) -> Vec<Gap> {
        self.state
            .lock()
            .unwrap()
            .gaps
            .iter()
            .filter(|gap| gap.scope == GapScope::Store)
            .cloned()
            .collect()
    }

    pub fn record_gap(&self, gap: Gap) {
        self.state.lock().unwrap().gaps.push(gap);
    }

    /// Subscribe after a cursor. The returned replay vector and receiver are
    /// both bounded by the retention ring and the per-subscriber channel.
    pub fn subscribe(
        &self,
        after: EventId,
    ) -> Result<(Subscription, SubscribeResult), EvidenceError> {
        let mut state = self.state.lock().unwrap();
        let retained_from = state
            .events
            .front()
            .map(|event| event.event_id)
            .unwrap_or(state.next_id);
        let last = state.next_id.saturating_sub(1);
        if after != 0 && (after < retained_from.saturating_sub(1) || after > last) {
            let (_tx, receiver) = std::sync::mpsc::sync_channel(1);
            return Ok((
                Subscription {
                    replay: Vec::new(),
                    receiver,
                    cursor: after,
                },
                SubscribeResult::ResyncRequired {
                    cursor: after,
                    retained_from,
                },
            ));
        }
        let replay: Vec<_> = state
            .events
            .iter()
            .filter(|event| event.event_id > after)
            .cloned()
            .collect();
        let id = state.next_subscriber;
        state.next_subscriber += 1;
        let (tx, receiver) = std::sync::mpsc::sync_channel(128);
        let last_sent = replay.last().map(|event| event.event_id).unwrap_or(after);
        state.subscribers.push(Subscriber { id, tx, last_sent });
        let result = SubscribeResult::Replay {
            cursor: after,
            retained_from,
        };
        Ok((
            Subscription {
                replay,
                receiver,
                cursor: after,
            },
            result,
        ))
    }

    /// Number of subscribers, used by tests to verify a browser disconnect
    /// removes only that observer.
    pub fn subscriber_count(&self) -> usize {
        self.state.lock().unwrap().subscribers.len()
    }

    pub fn current_status(&self, node: &NodeId, block_hash: &str) -> CurrentStatus {
        let state = self.state.lock().unwrap();
        let history: Vec<CanonicalAt> = state
            .events
            .iter()
            .filter_map(|event| match &event.payload {
                EvidencePayload::CanonicalAt(value)
                    if &value.node_id == node && value.block_hash == block_hash =>
                {
                    Some(value.clone())
                }
                _ => None,
            })
            .collect();
        match history.last() {
            Some(latest) => CurrentStatus {
                node_id: node.clone(),
                block_hash: block_hash.to_string(),
                result: latest.result,
                observed_at: latest.observed_at,
                anchored_tip: latest.anchored_tip.clone(),
                detail: latest.detail.clone(),
                history,
            },
            None => CurrentStatus {
                node_id: node.clone(),
                block_hash: block_hash.to_string(),
                result: CanonicalResult::Unknown,
                observed_at: 0,
                anchored_tip: String::new(),
                detail: "no canonical_at observation for this node and block".to_string(),
                history,
            },
        }
    }

    pub fn has_node_rejection(&self, block_hash: &str) -> Option<NodeRejected> {
        let state = self.state.lock().unwrap();
        state
            .events
            .iter()
            .rev()
            .find_map(|event| match &event.payload {
                EvidencePayload::NodeRejected(value) if value.block_hash == block_hash => {
                    Some(value.clone())
                }
                _ => None,
            })
    }

    pub fn evaluate_assertion(&self, spec: &AssertionSpec) -> AssertionEvaluation {
        let gaps = self.durable_gaps();
        let predicate = spec.predicate.trim();
        if predicate == "no_gap" {
            return if gaps.is_empty() {
                AssertionEvaluation::Passed
            } else {
                AssertionEvaluation::Inconclusive(format!("{} observer gap(s)", gaps.len()))
            };
        }
        if !gaps.is_empty() && spec.kind != AssertionKind::Statistics {
            return AssertionEvaluation::Inconclusive(format!(
                "evidence has {} explicit gap(s); the assertion cannot be completed",
                gaps.len()
            ));
        }
        if predicate.starts_with("exists(") {
            let kinds = predicate
                .trim_start_matches("exists(")
                .trim_end_matches(')')
                .split("&&")
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>();
            if kinds.is_empty() {
                return AssertionEvaluation::Inconclusive(
                    "exists() has no evidence kind".to_string(),
                );
            }
            let events = self.events();
            for part in &kinds {
                let negated = part.starts_with('!');
                let kind = part.trim_start_matches('!').trim();
                let kind = match kind {
                    "share_accounted" => "accounted",
                    "share_submitted" => "submitted",
                    "share_candidate" => "candidate",
                    other => other,
                };
                let found = events.iter().any(|event| event.payload.kind() == kind);
                if negated && found {
                    return AssertionEvaluation::Failed(format!(
                        "predicate required absence of {kind}, but it exists"
                    ));
                }
                if !negated && !found {
                    return AssertionEvaluation::Inconclusive(format!(
                        "{kind} was not observed; absence is not invalid"
                    ));
                }
            }
            return AssertionEvaluation::Passed;
        }
        if predicate.starts_with("accounted_work(") {
            let args = parse_kv(
                predicate
                    .trim_start_matches("accounted_work(")
                    .trim_end_matches(')'),
            );
            let Some(front) = args
                .get("front")
                .and_then(|value| crate::ids::FrontId::new(value.clone()).ok())
            else {
                return AssertionEvaluation::Inconclusive(
                    "accounted_work requires front=<front id>".to_string(),
                );
            };
            let min = args
                .get("min")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(1);
            let count = self
                .events()
                .iter()
                .filter(|event| {
                    matches!(
                        &event.payload,
                        EvidencePayload::SpcEvidence(record)
                            if record.event == "accounted"
                                && record.front_id.as_ref() == Some(&front)
                    )
                })
                .count() as u64;
            return if count >= min {
                AssertionEvaluation::Passed
            } else {
                AssertionEvaluation::Inconclusive(format!(
                    "front {front} has {count} accounted record(s), requires {min}; absence is not invalid"
                ))
            };
        }
        if predicate.starts_with("canonical_at(") {
            // Format: canonical_at(node=<id>,block=<hex>,at=<observed_at>)
            let args = parse_kv(
                predicate
                    .trim_start_matches("canonical_at(")
                    .trim_end_matches(')'),
            );
            let node = args.get("node").cloned();
            let block = args.get("block").cloned();
            let at = args.get("at").and_then(|value| value.parse::<u64>().ok());
            let (Some(node), Some(block), Some(at)) = (node, block, at) else {
                return AssertionEvaluation::Inconclusive(
                    "canonical_at requires node=, block= and at=".to_string(),
                );
            };
            let Ok(node) = NodeId::new(node) else {
                return AssertionEvaluation::Inconclusive("bad node id".to_string());
            };
            let status = self.current_status(&node, &block);
            match status.result {
                CanonicalResult::Yes if status.observed_at == at => AssertionEvaluation::Passed,
                CanonicalResult::No if status.observed_at == at => AssertionEvaluation::Failed(
                    "canonical_at at the named observation says no".to_string(),
                ),
                CanonicalResult::Unknown => AssertionEvaluation::Inconclusive(
                    "canonical_at at the named observation is unknown".to_string(),
                ),
                _ => AssertionEvaluation::Inconclusive(
                    "the named observation is not the current observation".to_string(),
                ),
            }
        } else {
            // An unknown predicate is never a pass.
            AssertionEvaluation::Inconclusive(format!(
                "predicate {:?} is not executable by the runner",
                spec.predicate
            ))
        }
    }

    /// Durable source payloads remain separate from chart projections.
    pub fn export_jsonl(&self, path: impl AsRef<Path>) -> Result<(), EvidenceError> {
        let events = self.events();
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent).map_err(|e| EvidenceError::Io(e.to_string()))?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(path.as_ref())
            .map_err(|e| EvidenceError::Io(e.to_string()))?;
        let _ = std::fs::set_permissions(path.as_ref(), std::fs::Permissions::from_mode(0o600));
        for event in events {
            let line = serde_json::to_vec(&*event).map_err(|e| EvidenceError::Io(e.to_string()))?;
            file.write_all(&line)
                .and_then(|_| file.write_all(b"\n"))
                .map_err(|e| EvidenceError::Io(e.to_string()))?;
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn append_locked(
    state: &mut State,
    run_id: &RunId,
    source: SourceId,
    source_cursor: String,
    actor: EvidenceActor,
    command_id: Option<CommandId>,
    payload: EvidencePayload,
    occurred_at_wall: u64,
) -> EventId {
    let event_id = state.next_id;
    state.next_id += 1;
    let event = Arc::new(EvidenceEvent {
        run_id: run_id.clone(),
        event_id,
        source,
        source_cursor,
        actor,
        command_id,
        occurred_at_wall,
        occurred_at_mono: mono_now_ns(),
        ingested_at: now_ms(),
        payload,
    });
    if let Some(writer) = state.writer.as_mut() {
        if let Ok(mut line) = serde_json::to_vec(&*event) {
            line.push(b'\n');
            if let Err(error) = writer.write_all(&line).and_then(|_| writer.flush()) {
                state.gaps.push(Gap {
                    scope: GapScope::Store,
                    subscriber: 0,
                    from_event_id: event_id,
                    to_event_id: event_id,
                    reason: format!("durable evidence write failed: {error}"),
                });
            }
        }
    }
    state.events.push_back(event.clone());
    if state.events.len() > state.capacity {
        state.events.pop_front();
    }
    // Notify observers without ever waiting on one.
    let mut overflowed = Vec::new();
    state
        .subscribers
        .retain_mut(|subscriber| match subscriber.tx.try_send(event.clone()) {
            Ok(()) => {
                subscriber.last_sent = event_id;
                true
            }
            Err(TrySendError::Full(_)) => {
                // Do not silently continue with a buffered subscriber that
                // missed an event. Close its stream; the HTTP layer answers
                // the terminal disconnect with an explicit resync snapshot.
                overflowed.push(subscriber.id);
                false
            }
            Err(TrySendError::Disconnected(_)) => false,
        });
    for subscriber in overflowed {
        let gap = Gap {
            scope: GapScope::Subscriber,
            subscriber,
            from_event_id: event_id,
            to_event_id: event_id,
            reason: "subscriber queue full; event dropped without blocking producer".to_string(),
        };
        state.gaps.push(gap.clone());
        // The gap is itself an event in the append-only log; it is not forced
        // into the stalled subscriber's queue.
        let gap_id = state.next_id;
        state.next_id += 1;
        let gap_event = Arc::new(EvidenceEvent {
            run_id: run_id.clone(),
            event_id: gap_id,
            source: SourceId::trusted("observer"),
            source_cursor: format!("gap-{gap_id}"),
            actor: EvidenceActor::harness(),
            command_id: None,
            occurred_at_wall,
            occurred_at_mono: mono_now_ns(),
            ingested_at: now_ms(),
            payload: EvidencePayload::EventLost(gap),
        });
        if let Some(writer) = state.writer.as_mut() {
            if let Ok(mut line) = serde_json::to_vec(&*gap_event) {
                line.push(b'\n');
                let _ = writer.write_all(&line);
                let _ = writer.flush();
            }
        }
        state.events.push_back(gap_event.clone());
        if state.events.len() > state.capacity {
            state.events.pop_front();
        }
    }
    event_id
}

fn parse_kv(value: &str) -> BTreeMap<String, String> {
    value
        .split(',')
        .filter_map(|part| {
            let (key, value) = part.split_once('=')?;
            Some((key.trim().to_string(), value.trim().to_string()))
        })
        .collect()
}

fn decode_reversed_hex(value: &str) -> Result<[u8; 32], EvidenceError> {
    let value = value.trim().trim_start_matches("0x");
    let mut bytes = hex::decode(value).map_err(|_| EvidenceError::BadHex(value.to_string()))?;
    if bytes.len() != 32 {
        return Err(EvidenceError::BadHex(value.to_string()));
    }
    bytes.reverse();
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

fn mono_now_ns() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_nanos() as u64
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_observation_is_unknown_not_invalid() {
        let store = EvidenceStore::new(RunId::new("run-a").unwrap(), 100);
        let status =
            store.current_status(&NodeId::new("node-a").unwrap(), "00".repeat(32).as_str());
        assert_eq!(status.result, CanonicalResult::Unknown);
        assert!(store.has_node_rejection("00".repeat(32).as_str()).is_none());
    }
}
