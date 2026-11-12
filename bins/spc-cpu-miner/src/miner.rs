use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::AtomicU32;
use std::sync::Arc;

use crate::config::{validate_id, ConnectionConfig};
use crate::connection::{start_connection, ConnectionHandle};
use crate::control::{Command, CommandLedger, CommandResult, Operation};
use crate::ids::{ConnectionId, MinerId, WorkerId};
use crate::report::{EventSink, MinerSnapshot};
use crate::worker::WorkerHandle;

pub struct Miner {
    miner_id: MinerId,
    events: EventSink,
    connections: BTreeMap<ConnectionId, ConnectionHandle>,
    workers: BTreeMap<WorkerId, Arc<WorkerHandle>>,
    next_worker_slot: Arc<AtomicU32>,
    next_session_slot: Arc<AtomicU32>,
    ledger: CommandLedger,
    stopping: bool,
}

impl Miner {
    pub fn new(miner_id: MinerId, events: EventSink) -> Self {
        Self {
            miner_id,
            events,
            connections: BTreeMap::new(),
            workers: BTreeMap::new(),
            next_worker_slot: Arc::new(AtomicU32::new(0)),
            next_session_slot: Arc::new(AtomicU32::new(0)),
            ledger: CommandLedger::default(),
            stopping: false,
        }
    }

    pub fn is_stopping(&self) -> bool {
        self.stopping
    }

    pub async fn execute(&mut self, command: Command) -> CommandResult {
        if let Some(replay) = self.ledger.begin(&command) {
            return replay;
        }
        let result = match command.validate() {
            Ok(()) => self.execute_valid(&command).await,
            Err(error) => CommandResult::rejected(
                command.command_id.clone(),
                "invalid_command",
                error.to_string(),
            ),
        };
        self.ledger.complete(&command, result.clone());
        result
    }

    async fn execute_valid(&mut self, command: &Command) -> CommandResult {
        let id = command.command_id.clone();
        match &command.operation {
            Operation::AddConnection { connection } => {
                if self.stopping {
                    return CommandResult::rejected(id, "miner_stopping", "miner is stopping");
                }
                if self.connections.contains_key(&connection.id) {
                    return CommandResult::rejected(
                        id,
                        "connection_exists",
                        format!("connection {} already exists", connection.id),
                    );
                }
                if connection
                    .workers
                    .iter()
                    .any(|worker| self.workers.contains_key(&worker.id))
                {
                    return CommandResult::rejected(
                        id,
                        "worker_exists",
                        "a worker id already exists on another connection",
                    );
                }
                match self.start(connection.clone()) {
                    Ok(worker_ids) => CommandResult::running(
                        id,
                        serde_json::json!({
                            "connection_id": connection.id,
                            "worker_ids": worker_ids,
                            "state": "starting",
                        }),
                    ),
                    Err(error) => CommandResult::failed(id, "connection_start_failed", error),
                }
            }
            Operation::RemoveConnection { connection_id } => {
                if validate_id(connection_id).is_err() {
                    return CommandResult::rejected(
                        id,
                        "invalid_connection_id",
                        "invalid connection id",
                    );
                }
                let Some(handle) = self.connections.remove(connection_id) else {
                    return CommandResult::rejected(
                        id,
                        "unknown_connection",
                        format!("unknown connection {connection_id}"),
                    );
                };
                self.forget_workers(&handle);
                let worker_count = handle.workers.len();
                handle.shutdown().await;
                CommandResult::succeeded(
                    id,
                    serde_json::json!({"connection_id": connection_id, "workers_removed": worker_count}),
                )
            }
            Operation::SetWorkerBudget { worker_id, budget } => {
                let Some(worker) = self.workers.get(worker_id) else {
                    return CommandResult::rejected(
                        id,
                        "unknown_worker",
                        format!("unknown worker {worker_id}"),
                    );
                };
                worker.set_budget(budget.clone());
                CommandResult::succeeded(id, serde_json::json!(worker.snapshot()))
            }
            Operation::PauseWorkers { worker_ids } | Operation::ResumeWorkers { worker_ids } => {
                let pause = matches!(command.operation, Operation::PauseWorkers { .. });
                if worker_ids
                    .iter()
                    .any(|worker| !self.workers.contains_key(worker))
                {
                    return CommandResult::rejected(
                        id,
                        "unknown_worker",
                        "at least one worker id does not exist; no workers changed",
                    );
                }
                let unique: BTreeSet<_> = worker_ids.iter().collect();
                for worker_id in unique {
                    self.workers[worker_id].set_paused(pause);
                    // `unique` borrows the command IDs; all existence checks
                    // completed before the first state change.
                }
                let snapshots: Vec<_> = worker_ids
                    .iter()
                    .map(|worker_id| self.workers[worker_id].snapshot())
                    .collect();
                CommandResult::succeeded(id, serde_json::json!({"workers": snapshots}))
            }
            Operation::SwitchPool {
                connection_id,
                endpoint,
                identity,
                authority_public_key,
                min_extranonce_size,
            } => {
                if self.stopping {
                    return CommandResult::rejected(id, "miner_stopping", "miner is stopping");
                }
                let Some(previous) = self.connections.get(connection_id) else {
                    return CommandResult::rejected(
                        id,
                        "unknown_connection",
                        format!("unknown connection {connection_id}"),
                    );
                };
                let mut next = previous.config.clone();
                if next.source_audit.required() {
                    return CommandResult::rejected(
                        id,
                        "audit_rebind_required",
                        "audited pool switches require a new controller-bound AddConnection",
                    );
                }
                next.endpoint = endpoint.clone();
                next.identity = identity.clone();
                next.authority_public_key = authority_public_key.clone();
                if let Some(size) = min_extranonce_size {
                    next.min_extranonce_size = *size;
                }
                if let Err(error) = next.validate() {
                    return CommandResult::rejected(id, "invalid_switch", error.to_string());
                }
                // Finish the real disconnect before any new registration.
                let previous = self
                    .connections
                    .remove(connection_id)
                    .expect("checked above");
                self.forget_workers(&previous);
                previous.shutdown().await;
                match self.start(next) {
                    Ok(worker_ids) => CommandResult::running(
                        id,
                        serde_json::json!({
                            "connection_id": connection_id,
                            "worker_ids": worker_ids,
                            "state": "starting_after_real_disconnect",
                        }),
                    ),
                    Err(error) => CommandResult::failed(id, "switch_start_failed", error),
                }
            }
            Operation::Snapshot {} => {
                CommandResult::succeeded(id, serde_json::json!(self.snapshot()))
            }
            Operation::Stop {} => {
                self.stopping = true;
                self.stop_connections().await;
                CommandResult::succeeded(id, serde_json::json!({"stopping": true}))
            }
        }
    }

    fn start(&mut self, config: ConnectionConfig) -> Result<Vec<WorkerId>, String> {
        if let crate::config::SourceAuditConfig::Required { miner_id, .. } = &config.source_audit {
            if miner_id.as_str() != self.miner_id.as_str() {
                return Err("source audit miner binding mismatch".into());
            }
        }
        let worker_ids = config
            .workers
            .iter()
            .map(|worker| worker.id.clone())
            .collect();
        let handle = start_connection(
            config.clone(),
            Arc::clone(&self.next_worker_slot),
            Arc::clone(&self.next_session_slot),
            self.events.clone(),
        )
        .map_err(|error| error.to_string())?;
        for worker in &handle.workers {
            self.workers.insert(worker.id.clone(), Arc::clone(worker));
        }
        self.connections.insert(config.id.clone(), handle);
        Ok(worker_ids)
    }

    fn forget_workers(&mut self, handle: &ConnectionHandle) {
        for worker in &handle.workers {
            self.workers.remove(&worker.id);
        }
    }

    pub fn snapshot(&self) -> MinerSnapshot {
        MinerSnapshot {
            snapshot_version: 1,
            miner_id: self.miner_id.clone(),
            occurred_at_mono_ms: self.events.mono_ms(),
            events_dropped_total: self.events.dropped(),
            stopping: self.stopping,
            connections: self
                .connections
                .values()
                .map(|handle| {
                    handle
                        .snapshot
                        .lock()
                        .expect("connection snapshot lock")
                        .clone()
                })
                .collect(),
            workers: self
                .workers
                .values()
                .map(|worker| worker.snapshot())
                .collect(),
        }
    }

    pub async fn stop_connections(&mut self) {
        self.stopping = true;
        let handles = std::mem::take(&mut self.connections);
        self.workers.clear();
        crate::connection::shutdown_connections(handles).await;
    }
}
