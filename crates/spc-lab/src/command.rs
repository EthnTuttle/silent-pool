//! Durable command envelope, idempotency and crash reconciliation.
//!
//! A command is appended to the log before it can have an external effect. A
//! retry with the same id and payload returns the stored outcome; the same id
//! with a different payload is a durable conflict. A command interrupted by a
//! supervisor crash is never assumed successful.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::ids::{ActorId, CommandId, FrontId, IdentityId, MinerId, NodeId, RunId, WorkerId};
use crate::manifest::{RunManifest, RunState};
use crate::scenario::ConnectionSpec;
use crate::scenario::WorkerSpec;
use crate::wide::WideU128;

pub mod traffic_audit;
pub mod traffic_budget;
use traffic_budget::{BudgetOperation, BudgetState};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LabCommand {
    pub command_id: CommandId,
    pub run_id: RunId,
    pub issued_at_wall: u64,
    pub actor: ActorId,
    pub kind: CommandKind,
}

impl LabCommand {
    pub fn new(run_id: RunId, actor: ActorId, kind: CommandKind) -> Self {
        Self {
            command_id: CommandId::generate(),
            run_id,
            issued_at_wall: now_ms(),
            actor,
            kind,
        }
    }

    pub fn with_id(mut self, command_id: CommandId) -> Self {
        self.command_id = command_id;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CommandKind {
    /// Closed before-sign lab fixture; never an arbitrary version knob.
    AcceptanceVersionRejectionV1 {
        user: IdentityId,
        destination_address: String,
        amount_sats: u64,
        fee_rate_sats_vb: u64,
    },
    /// Explicit private lab acceptance control; never a general RPC route.
    TrafficAcceptance {
        key: traffic_audit::HookKey,
        control: traffic_audit::HookAction,
    },
    BootstrapFund {
        driver_id: ActorId,
        node_id: NodeId,
    },
    BootstrapControl {
        operation_id: CommandId,
        action: BootstrapAction,
    },
    PauseTxDriver {
        driver_id: ActorId,
    },
    ResumeTxDriver {
        driver_id: ActorId,
    },
    RestartTxDriver {
        driver_id: ActorId,
    },
    SendUserPayment {
        user: IdentityId,
        destination_address: String,
        amount_sats: u64,
        fee_rate_sats_vb: u64,
    },
    WalletRefresh {
        identity_id: IdentityId,
        vtxo_ids: Vec<String>,
    },
    WalletOffboard {
        identity_id: IdentityId,
        vtxo_ids: Vec<String>,
        address: String,
    },
    WalletReadInputs {
        identity_id: IdentityId,
    },
    StartRun {
        #[serde(default)]
        reason: Option<String>,
    },
    StopRun {
        reason: String,
    },
    AddMiner {
        miner_id: MinerId,
        implementation: String,
        connections: Vec<ConnectionSpec>,
        workers: Vec<WorkerSpec>,
    },
    RemoveMiner {
        miner_id: MinerId,
    },
    AddConnection {
        miner_id: MinerId,
        connection: ConnectionSpec,
    },
    RemoveConnection {
        miner_id: MinerId,
        connection_id: crate::ids::ConnectionId,
    },
    AddWorker {
        miner_id: MinerId,
        worker: WorkerSpec,
    },
    SetWorkerBudget {
        miner_id: MinerId,
        worker_id: WorkerId,
        hashes_per_second: Option<WideU128>,
        total_hashes: Option<WideU128>,
    },
    PauseWorkers {
        miner_id: MinerId,
        workers: Vec<WorkerId>,
    },
    ResumeWorkers {
        miner_id: MinerId,
        workers: Vec<WorkerId>,
    },
    SwitchPool {
        miner_id: MinerId,
        connection_id: crate::ids::ConnectionId,
        front_id: FrontId,
        identity_id: IdentityId,
    },
    WaitHeight {
        node_id: NodeId,
        height: u64,
        timeout_secs: u64,
    },
    Assert {
        assertion_id: String,
    },
    KillRole {
        role: String,
        entity_id: String,
    },
    RestartRole {
        role: String,
        entity_id: String,
    },
    PartitionLink {
        link_id: crate::ids::LinkId,
    },
    HealLink {
        link_id: crate::ids::LinkId,
    },
    InjectTx {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        driver_id: Option<ActorId>,
        adapter: String,
        payload: serde_json::Value,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BootstrapAction {
    Pause,
    Resume,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeState {
    Accepted,
    RejectedPrecondition,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    BlockedOnHeight,
    Unknown,
    RejectedConflict,
}

impl OutcomeState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::RejectedPrecondition => "rejected_precondition",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::BlockedOnHeight => "blocked_on_height",
            Self::Unknown => "unknown",
            Self::RejectedConflict => "rejected_conflict",
        }
    }

    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Accepted | Self::Running)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandOutcome {
    pub command_id: CommandId,
    pub state: OutcomeState,
    pub detail: String,
    pub resolved_at: u64,
    /// Durable command-log sequence this outcome was resolved at.
    #[serde(default)]
    pub journal_seq: u64,
}

impl CommandOutcome {
    pub fn new(command_id: CommandId, state: OutcomeState, detail: impl Into<String>) -> Self {
        Self {
            command_id,
            state,
            detail: detail.into(),
            resolved_at: now_ms(),
            journal_seq: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "record", rename_all = "snake_case", deny_unknown_fields)]
#[allow(clippy::large_enum_variant)]
enum LogLine {
    TrafficAudit {
        seq: u64,
        version: u32,
        boot_id: String,
        monotonic_ms: u64,
        event: traffic_audit::Event,
    },
    HashStart {
        seq: u64,
        operation: BudgetOperation,
    },
    HashReserve {
        seq: u64,
        operation_id: CommandId,
        driver_incarnation: String,
        batch_id: u64,
        reserved_attempts: u64,
        cumulative_reserved: u64,
    },
    HashReconcile {
        seq: u64,
        operation_id: CommandId,
        driver_incarnation: String,
        batch_id: u64,
        measured_attempts: Option<u64>,
    },
    Begin {
        seq: u64,
        command: LabCommand,
        payload_hash: String,
        /// Persisted hash-scheme discriminator for replay binding. Absent on
        /// every pre-access (legacy) entry: the legacy kind-only hash scheme
        /// is used and the fields are not serialized, so legacy journal lines
        /// stay byte-for-byte identical. An expanded-access entry always
        /// carries an explicit scheme; a missing or unknown scheme on such an
        /// entry fails closed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hash_scheme: Option<String>,
        /// The authenticated principal the entry's hash is bound to
        /// (claim user id, "operator" or "controller"). Absent on legacy
        /// entries.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        principal: Option<String>,
    },
    Finish {
        seq: u64,
        command_id: CommandId,
        outcome: CommandOutcome,
    },
    Conflict {
        seq: u64,
        command_id: CommandId,
        existing_hash: String,
        incoming_hash: String,
    },
}

#[derive(Debug, Clone)]
struct Entry {
    seq: u64,
    command: LabCommand,
    payload_hash: String,
    hash_scheme: Option<String>,
    principal: Option<String>,
    outcome: Option<CommandOutcome>,
}

struct State {
    file: File,
    wallet_resolution_failed: bool,
    next_seq: u64,
    entries: BTreeMap<CommandId, Entry>,
    traffic_strict: bool,
    poisoned: bool,
    budgets: BTreeMap<CommandId, BudgetState>,
    audit: traffic_audit::AuditState,
}

/// The result of asking the log whether a command is new.
#[derive(Debug, Clone)]
pub enum BeginOutcome {
    New,
    Existing(CommandOutcome),
    InFlight(CommandOutcome),
    Conflict(CommandOutcome),
    /// The stored entry cannot serve this begin: a different principal
    /// presented the id, or the entry's hash scheme is missing/unknown for
    /// an authenticated (expanded-access) begin. Closed without a journal
    /// write and without revealing that an outcome exists.
    Denied(CommandOutcome),
}

/// Legacy hash scheme: the payload hash covers `command.kind` only. Every
/// pre-access journal entry and every entry of a run with expanded access
/// off uses exactly this scheme, unchanged.
pub const HASH_SCHEME_KIND_ONLY: &str = "kind_only_v1";

/// Expanded-access hash scheme: the payload hash covers the hash-scheme
/// tag, the authenticated principal, the run id and the full `kind`
/// payload, so idempotence is keyed by principal, run and full command
/// semantics - never by `command_id` + kind alone.
pub const HASH_SCHEME_EXPANDED: &str = "expanded_v1";

/// Append-only durable command log.
pub struct CommandLog {
    path: PathBuf,
    state: Mutex<State>,
}

impl CommandLog {
    pub fn open(path: impl AsRef<Path>) -> std::io::Result<Self> {
        Self::open_mode(path.as_ref(), false)
    }

    /// Strict mode of the existing command log, required before any traffic
    /// bootstrap authorization. Legacy serialization and principal binding stay
    /// unchanged. Complete corrupt records are never skipped in this mode.
    pub fn open_traffic(path: impl AsRef<Path>) -> std::io::Result<Self> {
        Self::open_mode(path.as_ref(), true)
    }

    fn open_mode(path: &Path, traffic_strict: bool) -> std::io::Result<Self> {
        let path = path.to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .mode(0o600)
            .open(&path)?;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        let mut state = State {
            file,
            wallet_resolution_failed: false,
            next_seq: 1,
            entries: BTreeMap::new(),
            traffic_strict,
            poisoned: false,
            budgets: BTreeMap::new(),
            audit: Default::default(),
        };
        // Validate before repairing a provably incomplete final tail. A
        // complete JSON object without its newline is ambiguous and refused.
        let bytes = std::fs::read(&path)?;
        let complete_len = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |n| n + 1);
        if traffic_strict && complete_len < bytes.len() {
            match serde_json::from_slice::<LogLine>(&bytes[complete_len..]) {
                Err(error) if error.is_eof() => {}
                _ => return Err(traffic_budget::invalid("ambiguous command log tail")),
            }
        }
        if let Ok(reader) = File::open(&path) {
            use std::io::Read;
            let length = if traffic_strict {
                complete_len as u64
            } else {
                u64::MAX
            };
            for line in BufReader::new(reader.take(length)).lines() {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                let parsed = if traffic_strict {
                    spc_lab_txd::control::strict_json(line.as_bytes())
                        .map_err(|e| e.to_string())
                        .and_then(|v| {
                            serde_json::from_value::<LogLine>(v).map_err(|e| e.to_string())
                        })
                } else {
                    serde_json::from_str::<LogLine>(&line).map_err(|e| e.to_string())
                };
                let Ok(record) = parsed else {
                    if traffic_strict {
                        return Err(traffic_budget::invalid(
                            "corrupt complete command log record",
                        ));
                    }
                    // A torn final line is possible after a crash. It cannot
                    // authorize an effect and is ignored for replay.
                    // Preserve legacy reader behavior, but do not permit a
                    // wallet resolution/intent to append behind a torn tail.
                    state.wallet_resolution_failed = true;
                    continue;
                };
                if traffic_strict {
                    let seq = match &record {
                        LogLine::TrafficAudit { seq, .. }
                        | LogLine::Begin { seq, .. }
                        | LogLine::Finish { seq, .. }
                        | LogLine::Conflict { seq, .. }
                        | LogLine::HashStart { seq, .. }
                        | LogLine::HashReserve { seq, .. }
                        | LogLine::HashReconcile { seq, .. } => *seq,
                    };
                    if seq != state.next_seq || seq == u64::MAX {
                        return Err(traffic_budget::invalid(
                            "command log sequence gap or overflow",
                        ));
                    }
                }
                match record {
                    LogLine::TrafficAudit {
                        seq,
                        version,
                        ref boot_id,
                        ref event,
                        ..
                    } => {
                        if version != 1 || boot_id.is_empty() {
                            return Err(traffic_budget::invalid("audit version/clock binding"));
                        }
                        traffic_audit::replay(&mut state, seq, event)?;
                        state.next_seq += 1;
                    }
                    ref budget @ (LogLine::HashStart { .. }
                    | LogLine::HashReserve { .. }
                    | LogLine::HashReconcile { .. }) => {
                        if !traffic_strict {
                            return Err(traffic_budget::invalid(
                                "traffic command log requires strict mode",
                            ));
                        }
                        traffic_budget::apply(&mut state, budget)?;
                        state.next_seq += 1;
                    }
                    LogLine::Begin {
                        seq,
                        command,
                        payload_hash,
                        hash_scheme,
                        principal,
                    } => {
                        state.next_seq = state.next_seq.max(seq + 1);
                        state.entries.insert(
                            command.command_id.clone(),
                            Entry {
                                seq,
                                command,
                                payload_hash,
                                hash_scheme,
                                principal,
                                outcome: None,
                            },
                        );
                    }
                    LogLine::Finish {
                        seq,
                        command_id,
                        outcome,
                    } => {
                        state.next_seq = state.next_seq.max(seq + 1);
                        if let Some(entry) = state.entries.get_mut(&command_id) {
                            entry.outcome = Some(outcome);
                        }
                    }
                    LogLine::Conflict { seq, .. } => {
                        state.next_seq = state.next_seq.max(seq + 1);
                    }
                }
            }
        }
        if traffic_strict && complete_len < bytes.len() {
            state.file.set_len(complete_len as u64)?;
            state.file.sync_data()?;
        }
        Ok(Self {
            path,
            state: Mutex::new(state),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn operator_owned(&self, id: &CommandId) -> bool {
        self.state
            .lock()
            .unwrap()
            .entries
            .get(id)
            .is_some_and(|entry| {
                entry.principal.as_deref() == Some(crate::access::PRINCIPAL_OPERATOR)
                    && entry.hash_scheme.as_deref() == Some(HASH_SCHEME_EXPANDED)
            })
    }

    pub fn last_seq(&self) -> u64 {
        self.state.lock().unwrap().next_seq.saturating_sub(1)
    }

    pub fn payload_hash(command: &LabCommand) -> String {
        let encoded = serde_json::to_vec(&command.kind).unwrap_or_default();
        hex::encode(Sha256::digest(encoded))
    }

    /// The expanded-access payload hash: principal, run and full `kind`
    /// semantics under one explicit scheme tag.
    pub fn expanded_payload_hash(command: &LabCommand, principal: &str) -> String {
        let kind = serde_json::to_vec(&command.kind).unwrap_or_default();
        let mut hasher = Sha256::new();
        hasher.update(HASH_SCHEME_EXPANDED.as_bytes());
        hasher.update(b"\n");
        hasher.update(principal.as_bytes());
        hasher.update(b"\n");
        hasher.update(command.run_id.as_str().as_bytes());
        hasher.update(b"\n");
        hasher.update(&kind);
        hex::encode(hasher.finalize())
    }

    fn denied(command_id: &CommandId) -> BeginOutcome {
        BeginOutcome::Denied(CommandOutcome::new(
            command_id.clone(),
            OutcomeState::RejectedPrecondition,
            "command id is not available for this principal",
        ))
    }

    fn write_conflict_locked(
        &self,
        state: &mut State,
        command_id: &CommandId,
        existing_hash: String,
        incoming_hash: String,
    ) -> std::io::Result<CommandOutcome> {
        let conflict = CommandOutcome::new(
            command_id.clone(),
            OutcomeState::RejectedConflict,
            format!(
                "command id reused with a different payload (existing {existing_hash:?}, incoming {incoming_hash:?})"
            ),
        );
        let seq = state.next_seq;
        state.next_seq += 1;
        write_line(
            state,
            &LogLine::Conflict {
                seq,
                command_id: command_id.clone(),
                existing_hash,
                incoming_hash,
            },
        )?;
        Ok(conflict)
    }

    /// Durably record the intent to apply a command if it is new.
    ///
    /// Legacy path (expanded access off): the payload hash covers the kind
    /// only, exactly as before the access work, and the persisted line
    /// carries no scheme discriminator. A scheme-bearing (expanded) entry is
    /// never served to a legacy begin.
    pub fn begin(&self, command: &LabCommand) -> std::io::Result<BeginOutcome> {
        let mut state = self.state.lock().unwrap();
        let hash = Self::payload_hash(command);
        let existing = state.entries.get(&command.command_id).map(|entry| {
            (
                entry.payload_hash.clone(),
                entry.hash_scheme.is_some(),
                entry.outcome.clone(),
            )
        });
        if let Some((existing_hash, scheme_bound, existing_outcome)) = existing {
            // An expanded (scheme-bearing) entry is not a legacy outcome.
            if scheme_bound {
                return Ok(Self::denied(&command.command_id));
            }
            if existing_hash == hash {
                return Ok(match existing_outcome {
                    Some(outcome) => BeginOutcome::Existing(outcome),
                    None => BeginOutcome::InFlight(CommandOutcome::new(
                        command.command_id.clone(),
                        OutcomeState::Running,
                        "command is recorded but has no durable final outcome",
                    )),
                });
            }
            let conflict =
                self.write_conflict_locked(&mut state, &command.command_id, existing_hash, hash)?;
            return Ok(BeginOutcome::Conflict(conflict));
        }

        if crate::wallet_control::identity(&command.kind).is_some()
            && state.wallet_resolution_failed
        {
            return Err(std::io::Error::other("wallet journal unavailable"));
        }
        let seq = state.next_seq;
        state.next_seq += 1;
        write_line(
            &mut state,
            &LogLine::Begin {
                seq,
                command: command.clone(),
                payload_hash: hash.clone(),
                hash_scheme: None,
                principal: None,
            },
        )?;
        state.entries.insert(
            command.command_id.clone(),
            Entry {
                seq,
                command: command.clone(),
                payload_hash: hash,
                hash_scheme: None,
                principal: None,
                outcome: None,
            },
        );
        Ok(BeginOutcome::New)
    }

    /// The expanded-access begin: the payload hash is bound to the
    /// authenticated principal and the run before any stored outcome can be
    /// returned. A `command_id` presented by a different principal, or an
    /// entry whose persisted scheme is missing or unknown, fails closed with
    /// a generic denial, no journal write, and no leak that an outcome
    /// exists.
    pub fn begin_scoped(
        &self,
        command: &LabCommand,
        principal: &str,
    ) -> std::io::Result<BeginOutcome> {
        let mut state = self.state.lock().unwrap();
        let hash = Self::expanded_payload_hash(command, principal);
        let existing = state.entries.get(&command.command_id).map(|entry| {
            (
                entry.payload_hash.clone(),
                entry.hash_scheme.clone(),
                entry.principal.clone(),
                entry.outcome.clone(),
            )
        });
        if let Some((existing_hash, existing_scheme, existing_principal, existing_outcome)) =
            existing
        {
            match existing_scheme.as_deref() {
                // A legacy unbound entry is never an authenticated outcome.
                None => return Ok(Self::denied(&command.command_id)),
                // An unknown scheme discriminator fails closed.
                Some(HASH_SCHEME_EXPANDED) => {}
                Some(_) => return Ok(Self::denied(&command.command_id)),
            }
            if existing_principal.as_deref() != Some(principal) {
                // Cross-principal reuse: no leak that an outcome exists.
                return Ok(Self::denied(&command.command_id));
            }
            if existing_hash == hash {
                return Ok(match existing_outcome {
                    Some(outcome) => BeginOutcome::Existing(outcome),
                    None => BeginOutcome::InFlight(CommandOutcome::new(
                        command.command_id.clone(),
                        OutcomeState::Running,
                        "command is recorded but has no durable final outcome",
                    )),
                });
            }
            let conflict =
                self.write_conflict_locked(&mut state, &command.command_id, existing_hash, hash)?;
            return Ok(BeginOutcome::Conflict(conflict));
        }

        if crate::wallet_control::identity(&command.kind).is_some()
            && state.wallet_resolution_failed
        {
            return Err(std::io::Error::other("wallet journal unavailable"));
        }
        let seq = state.next_seq;
        state.next_seq += 1;
        write_line(
            &mut state,
            &LogLine::Begin {
                seq,
                command: command.clone(),
                payload_hash: hash.clone(),
                hash_scheme: Some(HASH_SCHEME_EXPANDED.to_string()),
                principal: Some(principal.to_string()),
            },
        )?;
        state.entries.insert(
            command.command_id.clone(),
            Entry {
                seq,
                command: command.clone(),
                payload_hash: hash,
                hash_scheme: Some(HASH_SCHEME_EXPANDED.to_string()),
                principal: Some(principal.to_string()),
                outcome: None,
            },
        );
        Ok(BeginOutcome::New)
    }

    pub fn finish(&self, command_id: &CommandId, outcome: &CommandOutcome) -> std::io::Result<()> {
        let mut state = self.state.lock().unwrap();
        if let Some(entry) = state.entries.get(command_id) {
            if entry.outcome.is_some() {
                return Ok(());
            }
        }
        let seq = state.next_seq;
        state.next_seq += 1;
        write_line(
            &mut state,
            &LogLine::Finish {
                seq,
                command_id: command_id.clone(),
                outcome: outcome.clone(),
            },
        )?;
        if let Some(entry) = state.entries.get_mut(command_id) {
            entry.outcome = Some(outcome.clone());
        }
        Ok(())
    }

    /// Read-only replay lookup. Scope and complete payload are checked before
    /// returning any stored outcome; missing entries never create a Begin.
    pub fn lookup_scoped(
        &self,
        command: &LabCommand,
        principal: Option<&str>,
    ) -> Option<BeginOutcome> {
        let state = self.state.lock().unwrap();
        let entry = state.entries.get(&command.command_id)?;
        if !entry_scope_matches(entry, command, principal) {
            return Some(Self::denied(&command.command_id));
        }
        Some(match &entry.outcome {
            Some(outcome) => BeginOutcome::Existing(outcome.clone()),
            None => BeginOutcome::InFlight(CommandOutcome::new(
                command.command_id.clone(),
                OutcomeState::Running,
                "command is recorded but its outcome is unavailable",
            )),
        })
    }

    pub fn wallet_resolutions_available(&self) -> bool {
        !self.state.lock().unwrap().wallet_resolution_failed
    }

    /// Wallet-only append-only resolution. The existing reader already takes
    /// the last Finish. Legacy finish() remains byte/behavior compatible.
    /// Terminal outcomes are immutable; identical checks do not append.
    pub fn supersede_nonterminal(
        &self,
        command: &LabCommand,
        principal: Option<&str>,
        outcome: &CommandOutcome,
    ) -> std::io::Result<(CommandOutcome, bool)> {
        let mut state = self.state.lock().unwrap();
        let invalid = || std::io::Error::other("wallet resolution is not available");
        let entry = state.entries.get(&command.command_id).ok_or_else(invalid)?;
        if crate::wallet_control::identity(&command.kind).is_none()
            || outcome.command_id != command.command_id
            || !entry_scope_matches(entry, command, principal)
        {
            return Err(invalid());
        }
        if let Some(previous) = &entry.outcome {
            if !matches!(
                previous.state,
                OutcomeState::Running | OutcomeState::Unknown
            ) || (previous.state == outcome.state && previous.detail == outcome.detail)
            {
                return Ok((previous.clone(), false));
            }
        }
        if state.wallet_resolution_failed {
            return Err(std::io::Error::other(
                "wallet resolution journal requires reopen",
            ));
        }
        let seq = state.next_seq;
        let mut durable = outcome.clone();
        durable.journal_seq = seq;
        let record = LogLine::Finish {
            seq,
            command_id: command.command_id.clone(),
            outcome: durable.clone(),
        };
        if let Err(error) = write_line(&mut state, &record) {
            // A partial write/sync has an uncertain durability boundary. Do
            // not update RAM or append more wallet resolutions until reopen.
            state.wallet_resolution_failed = true;
            return Err(error);
        }
        state.next_seq += 1;
        state.entries.get_mut(&command.command_id).unwrap().outcome = Some(durable.clone());
        Ok((durable, true))
    }

    pub fn outcome(&self, command_id: &CommandId) -> Option<CommandOutcome> {
        let state = self.state.lock().unwrap();
        state
            .entries
            .get(command_id)
            .and_then(|entry| entry.outcome.clone())
    }

    pub fn in_flight(&self) -> Vec<LabCommand> {
        let state = self.state.lock().unwrap();
        let mut entries: Vec<&Entry> = state
            .entries
            .values()
            .filter(|entry| entry.outcome.is_none())
            .collect();
        entries.sort_by_key(|entry| entry.seq);
        entries
            .into_iter()
            .map(|entry| entry.command.clone())
            .collect()
    }

    pub fn entries(&self) -> Vec<(LabCommand, Option<CommandOutcome>)> {
        let state = self.state.lock().unwrap();
        let mut entries: Vec<&Entry> = state.entries.values().collect();
        entries.sort_by_key(|entry| entry.seq);
        entries
            .into_iter()
            .map(|entry| (entry.command.clone(), entry.outcome.clone()))
            .collect()
    }

    /// Resolve crash-interrupted commands from the persisted run manifest.
    ///
    /// A pipe-backed control effect cannot be observed after a crash, so
    /// pause/resume/budget are `unknown`, never inferred from empty memory.
    pub fn reconcile(&self, manifest: &RunManifest) -> std::io::Result<Vec<CommandOutcome>> {
        let in_flight = self.in_flight();
        let mut outcomes = Vec::new();
        for command in in_flight {
            let outcome = match &command.kind {
                CommandKind::StartRun { .. } => match manifest.state {
                    RunState::Running | RunState::Stopping | RunState::Stopped => {
                        CommandOutcome::new(
                            command.command_id.clone(),
                            OutcomeState::Succeeded,
                            "reconciled: run manifest shows the start took effect",
                        )
                    }
                    _ => CommandOutcome::new(
                        command.command_id.clone(),
                        OutcomeState::Unknown,
                        "reconciled: run manifest does not prove the start took effect",
                    ),
                },
                CommandKind::StopRun { .. } => match manifest.state {
                    RunState::Stopped | RunState::CompletedWithErrors => CommandOutcome::new(
                        command.command_id.clone(),
                        OutcomeState::Succeeded,
                        "reconciled: run manifest shows the stop took effect",
                    ),
                    _ => CommandOutcome::new(
                        command.command_id.clone(),
                        OutcomeState::Unknown,
                        "reconciled: run manifest does not prove the stop took effect",
                    ),
                },
                CommandKind::AddMiner { miner_id, .. } => {
                    match manifest.process("miner", miner_id.as_str()) {
                        Some(record)
                            if matches!(
                                record.state,
                                crate::supervisor::ProcessState::Running
                                    | crate::supervisor::ProcessState::Adopted
                                    | crate::supervisor::ProcessState::AdoptedControlLost
                            ) =>
                        {
                            CommandOutcome::new(
                                command.command_id.clone(),
                                OutcomeState::Succeeded,
                                "reconciled: a verified miner process record is present",
                            )
                        }
                        _ => CommandOutcome::new(
                            command.command_id.clone(),
                            OutcomeState::Unknown,
                            "reconciled: no verified miner process record proves the add took effect",
                        ),
                    }
                }
                CommandKind::RemoveMiner { miner_id } => {
                    match manifest.process("miner", miner_id.as_str()) {
                        None => CommandOutcome::new(
                            command.command_id.clone(),
                            OutcomeState::Succeeded,
                            "reconciled: no miner process record remains",
                        ),
                        Some(record)
                            if matches!(
                                record.state,
                                crate::supervisor::ProcessState::Exited
                                    | crate::supervisor::ProcessState::Stopped
                                    | crate::supervisor::ProcessState::OrphanUnverified
                            ) =>
                        {
                            CommandOutcome::new(
                                command.command_id.clone(),
                                OutcomeState::Succeeded,
                                "reconciled: the miner process record is terminal",
                            )
                        }
                        Some(_) => CommandOutcome::new(
                            command.command_id.clone(),
                            OutcomeState::Unknown,
                            "reconciled: the miner process is still recorded live",
                        ),
                    }
                }
                CommandKind::PauseWorkers { .. }
                | CommandKind::ResumeWorkers { .. }
                | CommandKind::SetWorkerBudget { .. }
                | CommandKind::AddConnection { .. }
                | CommandKind::RemoveConnection { .. }
                | CommandKind::AddWorker { .. }
                | CommandKind::SwitchPool { .. } => CommandOutcome::new(
                    command.command_id.clone(),
                    OutcomeState::Unknown,
                    "reconciled: pipe-backed control effect cannot be observed after a crash",
                ),
                _ => CommandOutcome::new(
                    command.command_id.clone(),
                    OutcomeState::Unknown,
                    "reconciled: no authoritative manifest effect for this command",
                ),
            };
            self.finish(&command.command_id, &outcome)?;
            outcomes.push(outcome);
        }
        Ok(outcomes)
    }
}

fn entry_scope_matches(entry: &Entry, command: &LabCommand, principal: Option<&str>) -> bool {
    let hash = match principal {
        Some(principal)
            if entry.hash_scheme.as_deref() == Some(HASH_SCHEME_EXPANDED)
                && entry.principal.as_deref() == Some(principal) =>
        {
            CommandLog::expanded_payload_hash(command, principal)
        }
        None if entry.hash_scheme.is_none() && entry.principal.is_none() => {
            CommandLog::payload_hash(command)
        }
        _ => return false,
    };
    entry.payload_hash == hash
        && entry.command.run_id == command.run_id
        && entry.command.actor == command.actor
}

fn write_line(state: &mut State, record: &LogLine) -> std::io::Result<()> {
    if state.poisoned {
        return Err(traffic_budget::invalid(
            "command log authorization handle poisoned",
        ));
    }
    let result = (|| {
        let mut line = serde_json::to_vec(record)?;
        line.push(b'\n');
        state.file.write_all(&line)?;
        state.file.flush()?;
        state.file.sync_data()
    })();
    if state.traffic_strict && result.is_err() {
        state.poisoned = true;
    }
    if result.is_err() {
        // Every failed append can leave a partial tail, including a failed
        // initial Begin or a legacy Finish. Preserve its original error and
        // legacy behavior, but close subsequent wallet intent/resolution writes.
        state.wallet_resolution_failed = true;
    }
    result
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod wallet_resolution_tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn failed_initial_begin_latches_wallet_guard_before_another_intent() {
        for scoped in [true, false] {
            let dir = PathBuf::from(format!(
                "/tmp/opencode/wallet-begin-failure-{}-{}-{scoped}",
                std::process::id(),
                now_ms()
            ));
            let log = CommandLog::open(dir.join("commands.jsonl")).unwrap();
            let command = LabCommand::new(
                RunId::new("run").unwrap(),
                ActorId::new("user/alice").unwrap(),
                CommandKind::WalletRefresh {
                    identity_id: IdentityId::new("alice").unwrap(),
                    vtxo_ids: vec!["ab".repeat(36)],
                },
            );
            let (writer, mut reader) = std::os::unix::net::UnixStream::pair().unwrap();
            let fd: std::os::fd::OwnedFd = writer.into();
            log.state.lock().unwrap().file = File::from(fd);
            let begin = |c: &LabCommand| {
                if scoped {
                    log.begin_scoped(c, "alice")
                } else {
                    log.begin(c)
                }
            };
            // Bytes write, sync fails: the original intent was not acknowledged.
            assert!(begin(&command).is_err());
            assert!(!log.wallet_resolutions_available());
            reader
                .set_read_timeout(Some(std::time::Duration::from_secs(1)))
                .unwrap();
            assert!(reader.read(&mut [0u8; 4096]).unwrap() > 0);
            let mut next = command.clone();
            next.command_id = CommandId::generate();
            assert!(begin(&next).is_err());
            assert!(log.outcome(&next.command_id).is_none());
            reader.set_nonblocking(true).unwrap();
            assert_eq!(
                reader.read(&mut [0u8; 4096]).unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock
            );
            drop(log);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn failed_wallet_resolution_write_or_sync_never_claims_durable_success() {
        for sync_failure in [false, true] {
            let dir = PathBuf::from(format!(
                "/tmp/opencode/wallet-resolution-{}-{}-{sync_failure}",
                std::process::id(),
                now_ms()
            ));
            let path = dir.join("commands.jsonl");
            let log = CommandLog::open(&path).unwrap();
            let command = LabCommand::new(
                RunId::new("run").unwrap(),
                ActorId::new("user/alice").unwrap(),
                CommandKind::WalletRefresh {
                    identity_id: IdentityId::new("alice").unwrap(),
                    vtxo_ids: vec!["ab".repeat(36)],
                },
            );
            log.begin_scoped(&command, "alice").unwrap();
            let running = CommandOutcome::new(
                command.command_id.clone(),
                OutcomeState::Running,
                "submitted",
            );
            log.finish(&command.command_id, &running).unwrap();
            let bytes = std::fs::read(&path).unwrap();
            let mut peer = None;
            log.state.lock().unwrap().file = if sync_failure {
                let (writer, reader) = std::os::unix::net::UnixStream::pair().unwrap();
                peer = Some(reader);
                let fd: std::os::fd::OwnedFd = writer.into();
                File::from(fd)
            } else {
                OpenOptions::new().write(true).open("/dev/full").unwrap()
            };
            let terminal = CommandOutcome::new(
                command.command_id.clone(),
                OutcomeState::Succeeded,
                "confirmed",
            );
            assert!(log
                .supersede_nonterminal(&command, Some("alice"), &terminal)
                .is_err());
            if let Some(mut reader) = peer {
                reader
                    .set_read_timeout(Some(std::time::Duration::from_secs(1)))
                    .unwrap();
                assert!(reader.read(&mut [0u8; 4096]).unwrap() > 0);
            }
            assert_eq!(
                log.outcome(&command.command_id).unwrap().state,
                OutcomeState::Running
            );
            assert!(log
                .supersede_nonterminal(&command, Some("alice"), &terminal)
                .is_err());
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
            drop(log);
            let reopened = CommandLog::open(&path).unwrap();
            assert_eq!(
                reopened.outcome(&command.command_id).unwrap().state,
                OutcomeState::Running
            );
            drop(reopened);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }
}
