use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Context};
use stratum_apps::network_helpers::connect_with_noise;
use stratum_apps::network_helpers::noise_stream::{NoiseTcpReadHalf, NoiseTcpWriteHalf};
use stratum_core::binary_sv2::{self, Str0255, B032, U256};
use stratum_core::codec_sv2::SerializedFrame;
use stratum_core::common_messages_sv2::{
    Protocol, SetupConnection, SetupConnectionSuccess, MESSAGE_TYPE_SETUP_CONNECTION,
    MESSAGE_TYPE_SETUP_CONNECTION_SUCCESS,
};
use stratum_core::framing_sv2::framing::MessageFrame;
use stratum_core::mining_sv2::{
    CloseChannel, NewExtendedMiningJob, OpenExtendedMiningChannel,
    OpenExtendedMiningChannelSuccess, OpenMiningChannelError, SetExtranoncePrefix, SetNewPrevHash,
    SetTarget, SubmitSharesError, SubmitSharesExtended, SubmitSharesSuccess,
    CHANNEL_BIT_CLOSE_CHANNEL, CHANNEL_BIT_SUBMIT_SHARES_EXTENDED, MESSAGE_TYPE_CLOSE_CHANNEL,
    MESSAGE_TYPE_MINING_SET_NEW_PREV_HASH, MESSAGE_TYPE_NEW_EXTENDED_MINING_JOB,
    MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL, MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL_SUCCESS,
    MESSAGE_TYPE_OPEN_MINING_CHANNEL_ERROR, MESSAGE_TYPE_SET_EXTRANONCE_PREFIX,
    MESSAGE_TYPE_SET_TARGET, MESSAGE_TYPE_SUBMIT_SHARES_ERROR, MESSAGE_TYPE_SUBMIT_SHARES_EXTENDED,
    MESSAGE_TYPE_SUBMIT_SHARES_SUCCESS,
};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};

use crate::config::ConnectionConfig;
use crate::ids::ConnectionId;
use crate::native_gate::CandidateGate;
use crate::report::{
    ConnectionSnapshot, EventSink, FutureJobSnapshot, JobSnapshot, MinerEvent,
    SharedConnectionSnapshot, SubmissionSnapshot, SubmissionStatus,
};
use crate::session::{ProtocolState, StateChanges};
use crate::target::{target_from_le_bytes, target_le_hex};
use crate::worker::{FoundShare, WorkerHandle};

const OPEN_REQUEST_ID: u32 = 1;
const SHARE_QUEUE_CAPACITY: usize = 1024;
// NewExtendedMiningJob's two B064K halves plus 255 U256 siblings and fixed
// fields fit below this cap. Reject oversized frames before queueing them.
const MAX_NATIVE_FRAME: usize = 142 * 1024;

mod outbound;

#[cfg(test)]
#[path = "connection/lifecycle_tests.rs"]
mod lifecycle_tests;

pub struct ConnectionHandle {
    pub config: ConnectionConfig,
    pub workers: Vec<Arc<WorkerHandle>>,
    pub snapshot: SharedConnectionSnapshot,
    shutdown: watch::Sender<bool>,
    join: Option<JoinHandle<()>>,
    events: EventSink,
}

impl ConnectionHandle {
    pub fn request_stop(&self) {
        // Synchronous revocation does not depend on the session's writer or
        // scheduling. stop fences each audited hash and bars later publication.
        for worker in &self.workers {
            worker.stop();
        }
        self.snapshot
            .lock()
            .expect("connection snapshot lock")
            .clear_jobs();
        let _ = self.shutdown.send(true);
    }

    pub async fn shutdown(mut self) {
        self.request_stop();
        if let Some(join) = self.join.take() {
            let mut join = join;
            if timeout(Duration::from_secs(5), &mut join).await.is_err() {
                join.abort();
                let _ = join.await;
            }
        }
        for worker in &self.workers {
            worker.stop();
        }
        for worker in &self.workers {
            worker.join();
        }
        set_state(
            &self.events,
            &self.snapshot,
            None,
            "stopped",
            Some("connection and workers stopped".into()),
        );
    }
}

pub(crate) async fn shutdown_connections(handles: BTreeMap<ConnectionId, ConnectionHandle>) {
    // Signal/fence every connection before the first graceful-close/join await.
    for handle in handles.values() {
        handle.request_stop();
    }
    for (_, handle) in handles {
        handle.shutdown().await;
    }
}

#[allow(clippy::too_many_arguments)]
pub fn start_connection(
    config: ConnectionConfig,
    next_worker_slot: Arc<AtomicU32>,
    next_session_slot: Arc<AtomicU32>,
    events: EventSink,
) -> anyhow::Result<ConnectionHandle> {
    start_connection_with_capture(config, next_worker_slot, next_session_slot, events, None)
}

pub(crate) fn start_connection_with_capture(
    config: ConnectionConfig,
    next_worker_slot: Arc<AtomicU32>,
    next_session_slot: Arc<AtomicU32>,
    events: EventSink,
    capture: Option<Arc<crate::evidence::CaptureStore>>,
) -> anyhow::Result<ConnectionHandle> {
    config.validate()?;
    let (found_tx, found_rx) = mpsc::channel(SHARE_QUEUE_CAPACITY);
    let worker_count = u32::try_from(config.workers.len()).context("too many workers")?;
    let mut workers: Vec<Arc<WorkerHandle>> = Vec::with_capacity(config.workers.len());
    for (index, worker) in config.workers.iter().enumerate() {
        let slot = next_worker_slot.fetch_add(1, Ordering::SeqCst);
        if slot > u32::from(u16::MAX) {
            for worker in &workers {
                worker.stop();
            }
            for worker in &workers {
                worker.join();
            }
            return Err(anyhow!("worker slot space exhausted"));
        }
        workers.push(WorkerHandle::start(
            slot as u16,
            index as u32,
            worker_count,
            worker.clone(),
            config.id.clone(),
            found_tx.clone(),
            events.clone(),
        ));
    }
    drop(found_tx);
    let snapshot = Arc::new(Mutex::new(ConnectionSnapshot::new(
        &config.id,
        &config.endpoint,
        &config.identity,
    )));
    let (shutdown, shutdown_rx) = watch::channel(false);
    let task_config = config.clone();
    let task_workers = workers.clone();
    let task_snapshot = Arc::clone(&snapshot);
    let task_events = events.clone();
    let handle_events = events.clone();
    let join = tokio::spawn(run_connection(
        task_config,
        task_workers,
        task_snapshot,
        task_events,
        next_session_slot,
        shutdown_rx,
        found_rx,
        capture,
    ));
    Ok(ConnectionHandle {
        config,
        workers,
        snapshot,
        shutdown,
        join: Some(join),
        events: handle_events,
    })
}

#[derive(Debug)]
struct SessionExit {
    reason: String,
    opened: bool,
    close_initiated: bool,
}

enum SessionEvent {
    Shutdown,
    Incoming(Option<anyhow::Result<SerializedFrame>>),
    Found(Option<FoundShare>),
    Approved(Box<Result<crate::native_gate::VerifiedCandidate, &'static str>>),
    WriterFinished(anyhow::Result<()>),
    Expired,
}

#[allow(clippy::too_many_arguments)]
async fn run_connection(
    config: ConnectionConfig,
    workers: Vec<Arc<WorkerHandle>>,
    snapshot: SharedConnectionSnapshot,
    events: EventSink,
    next_session_slot: Arc<AtomicU32>,
    mut shutdown: watch::Receiver<bool>,
    mut found_rx: mpsc::Receiver<FoundShare>,
    capture: Option<Arc<crate::evidence::CaptureStore>>,
) {
    let mut backoff = config.reconnect.initial_backoff_ms;
    loop {
        if *shutdown.borrow() {
            break;
        }
        let raw_session = next_session_slot.fetch_add(1, Ordering::SeqCst);
        if raw_session > u32::from(u16::MAX) {
            set_state(
                &events,
                &snapshot,
                None,
                "failed",
                Some("reconnect session slot space exhausted".into()),
            );
            break;
        }
        let session_slot = raw_session as u16;
        snapshot
            .lock()
            .expect("connection snapshot lock")
            .session_slot = Some(session_slot);
        set_state(&events, &snapshot, Some(session_slot), "connecting", None);
        let result = run_session(
            &config,
            session_slot,
            &workers,
            &snapshot,
            &events,
            &mut shutdown,
            &mut found_rx,
            capture.clone(),
        )
        .await;
        for worker in &workers {
            worker.clear_work();
        }
        while found_rx.try_recv().is_ok() {}
        match result {
            Ok(exit) => {
                set_state(
                    &events,
                    &snapshot,
                    Some(session_slot),
                    "disconnected",
                    Some(exit.reason.clone()),
                );
                if exit.opened {
                    backoff = config.reconnect.initial_backoff_ms;
                }
                if exit.close_initiated {
                    break;
                }
            }
            Err(error) => {
                set_state(
                    &events,
                    &snapshot,
                    Some(session_slot),
                    "disconnected",
                    Some(format!("{error:#}")),
                );
            }
        }
        snapshot
            .lock()
            .expect("connection snapshot lock")
            .clear_jobs();
        if *shutdown.borrow() {
            break;
        }
        if !config.reconnect.enabled {
            break;
        }
        snapshot
            .lock()
            .expect("connection snapshot lock")
            .counters
            .reconnects += 1;
        set_state(
            &events,
            &snapshot,
            Some(session_slot),
            "reconnect_wait",
            Some(format!("retry in {backoff}ms")),
        );
        tokio::select! {
            _ = sleep(Duration::from_millis(backoff)) => {}
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() { break; }
            }
        }
        backoff = backoff
            .saturating_mul(2)
            .min(config.reconnect.max_backoff_ms);
    }
    set_state(&events, &snapshot, None, "stopped", None);
}

#[allow(clippy::too_many_arguments)]
async fn run_session(
    config: &ConnectionConfig,
    session_slot: u16,
    workers: &[Arc<WorkerHandle>],
    snapshot: &SharedConnectionSnapshot,
    events: &EventSink,
    shutdown: &mut watch::Receiver<bool>,
    found_rx: &mut mpsc::Receiver<FoundShare>,
    capture: Option<Arc<crate::evidence::CaptureStore>>,
) -> anyhow::Result<SessionExit> {
    let timeout_duration = Duration::from_millis(config.reconnect.handshake_timeout_ms);
    let stream = timeout(timeout_duration, TcpStream::connect(&config.endpoint))
        .await
        .map_err(|_| anyhow!("tcp connect timeout"))??;
    let local = stream.local_addr()?;
    let pinned = config.authority()?;
    // Authentication cannot be disabled: Some(pinned) is always supplied.
    let stream = timeout(timeout_duration, connect_with_noise(stream, Some(pinned)))
        .await
        .map_err(|_| anyhow!("noise authentication timeout"))?
        .map_err(|error| anyhow!("noise/pinned-authority handshake failed: {error}"))?;
    let (read, write) = stream.into_split();
    set_state(
        events,
        snapshot,
        Some(session_slot),
        "noise_authenticated",
        None,
    );
    let (frame_tx, mut frame_rx) = mpsc::channel(256);
    let _pump = AbortOnDrop(tokio::spawn(read_pump(read, frame_tx)));

    run_protocol(
        config,
        session_slot,
        workers,
        snapshot,
        events,
        shutdown,
        found_rx,
        local,
        &mut frame_rx,
        write,
        capture,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn run_protocol(
    config: &ConnectionConfig,
    session_slot: u16,
    workers: &[Arc<WorkerHandle>],
    snapshot: &SharedConnectionSnapshot,
    events: &EventSink,
    shutdown: &mut watch::Receiver<bool>,
    found_rx: &mut mpsc::Receiver<FoundShare>,
    local: std::net::SocketAddr,
    frame_rx: &mut mpsc::Receiver<anyhow::Result<SerializedFrame>>,
    transport: impl outbound::FrameWriter,
    capture: Option<Arc<crate::evidence::CaptureStore>>,
) -> anyhow::Result<SessionExit> {
    let timeout_duration = Duration::from_millis(config.reconnect.handshake_timeout_ms);
    let mut write = outbound::Writer::new(transport);

    let local_ip = local.ip().to_string();
    let endpoint_host = str0(&local_ip)?;
    let setup = SetupConnection {
        protocol: Protocol::MiningProtocol,
        min_version: 2,
        max_version: 2,
        flags: 0,
        endpoint_host,
        endpoint_port: local.port(),
        vendor: str0("spc-cpu-miner")?,
        hardware_version: str0("cpu-sha256d")?,
        firmware: str0(env!("CARGO_PKG_VERSION"))?,
        device_id: str0(&config.id)?,
    };
    write.send(setup, MESSAGE_TYPE_SETUP_CONNECTION, false)?;
    let mut frame = receive_within(frame_rx, timeout_duration).await?;
    if frame.header().msg_type() != MESSAGE_TYPE_SETUP_CONNECTION_SUCCESS {
        return Err(anyhow!("expected SetupConnection.Success"));
    }
    let setup_success: SetupConnectionSuccess = decode(&mut frame)?;
    if setup_success.used_version != 2 {
        return Err(anyhow!("pool selected unsupported SV2 version"));
    }

    let max_target = [0xffu8; 32];
    let open = OpenExtendedMiningChannel {
        request_id: OPEN_REQUEST_ID,
        user_identity: str0(&config.identity)?,
        nominal_hash_rate: config.nominal_hash_rate(),
        max_target: U256::from(&max_target),
        min_extranonce_size: config.min_extranonce_size,
    };
    write.send(open, MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL, false)?;
    let success = {
        let mut frame = receive_within(frame_rx, timeout_duration).await?;
        match frame.header().msg_type() {
            MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL_SUCCESS => {
                let success: OpenExtendedMiningChannelSuccess = decode(&mut frame)?;
                if success.request_id != OPEN_REQUEST_ID {
                    return Err(anyhow!("channel-open request_id mismatch"));
                }
                success.into_owned()
            }
            MESSAGE_TYPE_OPEN_MINING_CHANNEL_ERROR => {
                let error: OpenMiningChannelError = decode(&mut frame)?;
                if error.request_id != OPEN_REQUEST_ID {
                    return Err(anyhow!("channel-open error request_id mismatch"));
                }
                return Err(anyhow!(
                    "channel open rejected: {}",
                    error.error_code.as_utf8_or_hex()
                ));
            }
            other => return Err(anyhow!("expected channel-open response, got {other:#x}")),
        }
    };
    let initial_target = target_from_le_bytes(success.target.as_bytes())?;
    if success.extranonce_size < config.min_extranonce_size || success.extranonce_size > 32 {
        return Err(anyhow!("pool offered unusable extranonce size"));
    }
    let mut protocol = ProtocolState::new(
        success.channel_id,
        success.group_channel_id,
        initial_target,
        success.extranonce_prefix.as_bytes().to_vec(),
        usize::from(success.extranonce_size),
    )?;
    {
        let mut snapshot = snapshot.lock().expect("connection snapshot lock");
        snapshot.channel_id = Some(success.channel_id);
        snapshot.group_channel_id = Some(success.group_channel_id);
        snapshot.extranonce_size = Some(success.extranonce_size);
        snapshot.channel_target_le = Some(target_le_hex(initial_target));
    }
    set_state(events, snapshot, Some(session_slot), "channel_open", None);

    let mut gate = CandidateGate::new(config).with_capture(capture);
    if !gate.required() {
        snapshot.lock().expect("snapshot lock").source_audit_state =
            Some("explicit_unaudited".into());
        events.emit(MinerEvent::ExplicitUnaudited {
            connection_id: config.id.clone(),
        });
    }

    let mut sequence_number = 0u32;
    let mut outstanding = SubmissionTracker::new(MAX_OUTSTANDING_SHARES);
    let session_result = async {
        loop {
            let deadline = gate.deadline();
            let event = if *shutdown.borrow() {
                SessionEvent::Shutdown
            } else {
                tokio::select! {
                    biased;
                    _ = shutdown.changed() => SessionEvent::Shutdown,
                    _ = async {match deadline {Some(at)=>tokio::time::sleep_until(at).await,None=>std::future::pending::<()>().await}} => SessionEvent::Expired,
                    incoming = frame_rx.recv() => SessionEvent::Incoming(incoming),
                    result = write.finished() => SessionEvent::WriterFinished(result),
                    approved = gate.completed() => SessionEvent::Approved(Box::new(approved)),
                    found = found_rx.recv() => SessionEvent::Found(found),
                }
            };
            match event {
                SessionEvent::Shutdown => {
                    return Ok(SessionExit {
                        reason: "local shutdown".into(),
                        opened: true,
                        close_initiated: true,
                    });
                }
                SessionEvent::WriterFinished(result) => {
                    result?;
                    return Err(anyhow!("native writer stopped"));
                }
                SessionEvent::Found(found) => {
                    let Some(found) = found else {
                        return Err(anyhow!("hash worker queue closed"));
                    };
                    if found.session_slot != session_slot || !gate.accept_share(&found.job) {
                        continue;
                    }
                    let hash_meets_share =
                        crate::target::meets_target(&found.hash, found.job.share_target);
                    if !hash_meets_share {
                        return Err(anyhow!("internal share target validation failed"));
                    }
                    let block_qualified =
                        crate::target::meets_target(&found.hash, found.job.network_target);
                    let share = SubmissionSnapshot::new(
                        &config.id,
                        &found.worker_id,
                        session_slot,
                        sequence_number,
                        &found.job,
                        found.header,
                        found.hash,
                        &found.extranonce,
                        found.nonce,
                        found.ntime,
                        found.version,
                    );
                    let submit = SubmitSharesExtended {
                        channel_id: success.channel_id,
                        sequence_number,
                        job_id: found.job.job_id.into(),
                        nonce: found.nonce,
                        ntime: found.ntime,
                        version: found.version,
                        extranonce: B032::new(&found.extranonce)
                            .map_err(|error| anyhow!(error.to_string()))?,
                    };
                    write.send(
                        submit,
                        MESSAGE_TYPE_SUBMIT_SHARES_EXTENDED,
                        CHANNEL_BIT_SUBMIT_SHARES_EXTENDED,
                    )?;
                    // Submitted means accepted for bounded transport, not an
                    // ingress acknowledgement. Teardown marks unresolved rows
                    // transport-unknown, including frames not fully written.
                    let mut snapshot_guard = snapshot.lock().expect("connection snapshot lock");
                    snapshot_guard.counters.submitted += 1;
                    if block_qualified {
                        snapshot_guard.counters.block_qualified_submissions += 1;
                    }
                    let history_evicted = snapshot_guard.push_submission(share.clone());
                    drop(snapshot_guard);
                    if let Some(evicted) = history_evicted {
                        mark_evicted(snapshot, events, &evicted, "known-history-cap-evicted");
                    }
                    events.emit(MinerEvent::Submitted {
                        share: share.clone(),
                    });
                    if block_qualified {
                        events.emit(MinerEvent::BlockQualified {
                            connection_id: config.id.clone(),
                            sequence_number,
                            block_hash: share.block_hash.clone(),
                            candidate_submission: "unknown".into(),
                        });
                    }
                    if let Some(evicted) =
                        outstanding.insert(share).map_err(|error| anyhow!(error))?
                    {
                        mark_evicted(snapshot, events, &evicted, "outstanding-cap-evicted");
                    }
                    sequence_number = sequence_number
                        .checked_add(1)
                        .ok_or_else(|| anyhow!("submit sequence exhausted"))?;
                }
                SessionEvent::Incoming(incoming) => {
                    let Some(incoming) = incoming else {
                        return Err(anyhow!("pool connection closed"));
                    };
                    let mut frame = incoming?;
                    let msg_type = frame.header().msg_type();
                    match msg_type {
                        MESSAGE_TYPE_NEW_EXTENDED_MINING_JOB => {
                            gate.invalidate(workers, snapshot)?;
                            let message: NewExtendedMiningJob = decode(&mut frame)?;
                            let changes = protocol.on_new_extended(message)?;
                            apply_changes(
                                &config.id,
                                workers,
                                session_slot,
                                &protocol,
                                changes,
                                snapshot,
                                events,
                                "new-active-job",
                                &mut gate,
                            )?;
                        }
                        MESSAGE_TYPE_MINING_SET_NEW_PREV_HASH => {
                            gate.invalidate(workers, snapshot)?;
                            let message: SetNewPrevHash = decode(&mut frame)?;
                            let changes = protocol.on_set_new_prev_hash(message)?;
                            apply_changes(
                                &config.id,
                                workers,
                                session_slot,
                                &protocol,
                                changes,
                                snapshot,
                                events,
                                "new-prev-hash",
                                &mut gate,
                            )?;
                        }
                        MESSAGE_TYPE_SET_TARGET => {
                            gate.invalidate(workers, snapshot)?;
                            let message: SetTarget = decode(&mut frame)?;
                            let changes = protocol.on_set_target(message)?;
                            apply_changes(
                                &config.id,
                                workers,
                                session_slot,
                                &protocol,
                                changes,
                                snapshot,
                                events,
                                "set-target",
                                &mut gate,
                            )?;
                        }
                        MESSAGE_TYPE_SET_EXTRANONCE_PREFIX => {
                            gate.invalidate(workers, snapshot)?;
                            let message: SetExtranoncePrefix = decode(&mut frame)?;
                            protocol.set_extranonce_prefix(
                                message.channel_id,
                                message.extranonce_prefix.as_bytes().to_vec(),
                            )?;
                        }
                        MESSAGE_TYPE_SUBMIT_SHARES_SUCCESS => {
                            let response: SubmitSharesSuccess = decode(&mut frame)?;
                            if response.channel_id != success.channel_id {
                                return Err(anyhow!("share success channel_id mismatch"));
                            }
                            let acknowledged = outstanding
                                .complete_success(
                                    response.last_sequence_number,
                                    response.new_submits_accepted_count,
                                )
                                .map_err(|error| anyhow!(error))?;
                            let newly_resolved = acknowledged.len() as u64;
                            let mut snapshot_guard =
                                snapshot.lock().expect("connection snapshot lock");
                            snapshot_guard.counters.ingress_success_replies += 1;
                            snapshot_guard.counters.ingress_accepted_reported = snapshot_guard
                                .counters
                                .ingress_accepted_reported
                                .saturating_add(newly_resolved);
                            for share in acknowledged {
                                snapshot_guard.update_submission(
                                    session_slot,
                                    share.sequence_number,
                                    |stored| {
                                        stored.status = SubmissionStatus::IngressAck;
                                        stored.error_code = None;
                                    },
                                );
                            }
                            drop(snapshot_guard);
                            events.emit(MinerEvent::IngressAck {
                                connection_id: config.id.clone(),
                                sequence_number: response.last_sequence_number,
                                accepted_count: response.new_submits_accepted_count,
                                new_shares_sum: response.new_shares_sum,
                            });
                            for sequence in outstanding.drain_late_evicted() {
                                events.emit(MinerEvent::LateVerdict {
                                    connection_id: config.id.clone(),
                                    sequence_numbers: vec![sequence],
                                    outcome: "ingress_ack".into(),
                                });
                            }
                        }
                        MESSAGE_TYPE_SUBMIT_SHARES_ERROR => {
                            let response: SubmitSharesError = decode(&mut frame)?;
                            if response.channel_id != success.channel_id {
                                return Err(anyhow!("share error channel_id mismatch"));
                            }
                            outstanding
                                .complete_error(response.sequence_number)
                                .map_err(|error| anyhow!(error))?;
                            let code = response.error_code.as_utf8_or_hex();
                            snapshot
                                .lock()
                                .expect("connection snapshot lock")
                                .counters
                                .ingress_rejected += 1;
                            snapshot
                                .lock()
                                .expect("connection snapshot lock")
                                .update_submission(
                                    session_slot,
                                    response.sequence_number,
                                    |stored| {
                                        stored.status = SubmissionStatus::IngressRejected;
                                        stored.error_code = Some(code.clone());
                                    },
                                );
                            events.emit(MinerEvent::IngressRejected {
                                connection_id: config.id.clone(),
                                sequence_number: response.sequence_number,
                                error_code: code,
                            });
                            for sequence in outstanding.drain_late_evicted() {
                                events.emit(MinerEvent::LateVerdict {
                                    connection_id: config.id.clone(),
                                    sequence_numbers: vec![sequence],
                                    outcome: "ingress_rejected".into(),
                                });
                            }
                        }
                        MESSAGE_TYPE_CLOSE_CHANNEL => {
                            let response: CloseChannel = decode(&mut frame)?;
                            return Ok(SessionExit {
                                reason: format!(
                                    "pool closed channel: {}",
                                    response.reason_code.as_utf8_or_hex()
                                ),
                                opened: true,
                                close_initiated: false,
                            });
                        }
                        _ => {
                            // Unknown extension frames are ignored per the SV2
                            // extension rules; no feature is inferred from them.
                        }
                    }
                }
                SessionEvent::Approved(approved) => {
                    gate.finish(*approved, workers, snapshot, events, session_slot);
                }
                SessionEvent::Expired => {
                    let pending = gate.pending();
                    gate.invalidate(workers, snapshot)?;
                    if pending {
                        gate.finish(
                            Err("native_gate_timeout"),
                            workers,
                            snapshot,
                            events,
                            session_slot,
                        );
                    } else if let Some(job) = protocol.active_job() {
                        gate.start(
                            Arc::new(job.clone()),
                            protocol.extranonce_size(),
                            session_slot,
                            snapshot,
                            events,
                        )?;
                    }
                }
            }
        }
    };
    let result: anyhow::Result<SessionExit> = session_result.await;
    // No graceful I/O precedes this fence. This applies to shutdown, pool
    // disconnect, writer failure/overflow and all protocol-error exits.
    let invalidated = gate.invalidate(workers, snapshot);
    for worker in workers {
        worker.clear_work();
    }
    snapshot
        .lock()
        .expect("connection snapshot lock")
        .clear_jobs();
    invalidated?;
    let still_pending = outstanding.finish_transport();
    let unknown: Vec<u32> = still_pending
        .iter()
        .map(|share| share.sequence_number)
        .collect();
    if !unknown.is_empty() {
        {
            let mut snapshot_guard = snapshot.lock().expect("connection snapshot lock");
            for sequence in &unknown {
                snapshot_guard.update_submission(session_slot, *sequence, |stored| {
                    stored.status = SubmissionStatus::TransportUnknown;
                });
            }
        }
        events.emit(MinerEvent::SubmissionsUnknown {
            connection_id: config.id.clone(),
            sequence_numbers: unknown,
            reason: "session ended before an ingress verdict".into(),
        });
    }
    if result.as_ref().is_ok_and(|exit| exit.close_initiated) {
        write.close(success.channel_id).await;
    }
    result
}

pub fn mark_evicted(
    snapshot: &SharedConnectionSnapshot,
    events: &EventSink,
    share: &SubmissionSnapshot,
    reason: &str,
) {
    if reason == "known-history-cap-evicted" || share.status != SubmissionStatus::Submitted {
        events.emit(MinerEvent::SubmissionsEvicted {
            connection_id: share.connection_id.clone(),
            sequence_numbers: vec![share.sequence_number],
            reason: reason.into(),
        });
        return;
    }
    let connection_id = share.connection_id.clone();
    snapshot
        .lock()
        .expect("connection snapshot lock")
        .update_submission(share.session_slot, share.sequence_number, |stored| {
            stored.status = SubmissionStatus::TransportUnknown;
            stored.error_code = Some(reason.into());
        });
    events.emit(MinerEvent::SubmissionsUnknown {
        connection_id,
        sequence_numbers: vec![share.sequence_number],
        reason: reason.into(),
    });
}

#[allow(clippy::too_many_arguments)]
fn apply_changes(
    connection_id: &ConnectionId,
    workers: &[Arc<WorkerHandle>],
    session_slot: u16,
    protocol: &ProtocolState,
    changes: StateChanges,
    snapshot: &SharedConnectionSnapshot,
    events: &EventSink,
    reason: &str,
    gate: &mut CandidateGate,
) -> anyhow::Result<()> {
    if changes.target_changed {
        let target = protocol.channel_target();
        snapshot
            .lock()
            .expect("connection snapshot lock")
            .target_generation = protocol.target_generation();
        events.emit(MinerEvent::TargetUpdated {
            connection_id: connection_id.clone(),
            target_generation: protocol.target_generation(),
            share_target_le: target_le_hex(target),
        });
    }
    if !changes.invalidated.is_empty() {
        for worker in workers {
            worker.clear_work();
        }
        let mut snapshot_guard = snapshot.lock().expect("connection snapshot lock");
        snapshot_guard.counters.stale_cancelled = snapshot_guard
            .counters
            .stale_cancelled
            .saturating_add(changes.invalidated.len() as u64);
        events.emit(MinerEvent::JobsInvalidated {
            connection_id: connection_id.clone(),
            job_ids: changes.invalidated,
            reason: reason.into(),
        });
    }
    {
        let mut snapshot_guard = snapshot.lock().expect("connection snapshot lock");
        snapshot_guard.channel_target_le = Some(target_le_hex(protocol.channel_target()));
        snapshot_guard.target_generation = protocol.target_generation();
        if !gate.required() {
            snapshot_guard.active_job = protocol.active_job().map(JobSnapshot::from);
        }
        snapshot_guard.future_jobs = protocol
            .future_jobs()
            .map(|(_, job)| FutureJobSnapshot::from(job))
            .collect();
    }
    for job_template in changes.activated {
        let job = Arc::new(job_template);
        if gate.required() {
            gate.start(
                job,
                protocol.extranonce_size(),
                session_slot,
                snapshot,
                events,
            )?;
            continue;
        }
        events.emit(MinerEvent::JobActivated {
            connection_id: connection_id.clone(),
            job: JobSnapshot::from(job.as_ref()),
        });
        for worker in workers {
            worker.publish(session_slot, protocol.extranonce_size(), Arc::clone(&job));
        }
    }
    Ok(())
}

async fn receive_within(
    rx: &mut mpsc::Receiver<anyhow::Result<SerializedFrame>>,
    duration: Duration,
) -> anyhow::Result<SerializedFrame> {
    timeout(duration, rx.recv())
        .await
        .map_err(|_| anyhow!("SV2 receive timeout"))?
        .ok_or_else(|| anyhow!("SV2 connection closed"))?
}

async fn read_pump(read: NoiseTcpReadHalf, tx: mpsc::Sender<anyhow::Result<SerializedFrame>>) {
    let mut read = read;
    loop {
        match read.read_frame().await {
            Ok(frame) => {
                if frame.header().payload_length() > MAX_NATIVE_FRAME {
                    let _ = tx.send(Err(anyhow!("native SV2 frame bound"))).await;
                    return;
                }
                if tx.send(Ok(frame)).await.is_err() {
                    return;
                }
            }
            Err(error) => {
                let _ = tx.send(Err(anyhow!("noise read failed: {error}"))).await;
                return;
            }
        }
    }
}

struct AbortOnDrop(JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn decode<'a, T>(frame: &'a mut SerializedFrame) -> anyhow::Result<T>
where
    T: stratum_core::binary_sv2::Deserialize<'a>,
{
    binary_sv2::from_bytes(frame.payload()).map_err(|error| anyhow!(error.to_string()))
}

fn str0(value: &str) -> anyhow::Result<Str0255<'_>> {
    Str0255::try_from(value).map_err(|error| anyhow!(error.to_string()))
}

/// Bounded, correlation-only share state. Successful batches resolve precisely
/// the oldest no-error submissions through `last_sequence_number`; only the
/// response's aggregate share sum is retained, never apportioned across jobs.
pub const MAX_OUTSTANDING_SHARES: usize = 1024;

#[derive(Debug)]
pub struct SubmissionTracker {
    capacity: usize,
    pending: BTreeMap<u32, SubmissionSnapshot>,
    highest_submitted: Option<u32>,
    last_success: Option<(u32, u32)>,
    retired: BTreeSet<u32>,
    late_evicted: BTreeSet<u32>,
}

impl SubmissionTracker {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            pending: BTreeMap::new(),
            highest_submitted: None,
            last_success: None,
            retired: BTreeSet::new(),
            late_evicted: BTreeSet::new(),
        }
    }

    /// Insert one new submission, returning a pending submission evicted when
    /// the bound is reached. The caller must report that eviction as unknown;
    /// duplicate/reused sequence IDs are rejected before any eviction.
    pub fn insert(
        &mut self,
        share: SubmissionSnapshot,
    ) -> Result<Option<SubmissionSnapshot>, String> {
        let sequence = share.sequence_number;
        if self.pending.contains_key(&sequence) {
            return Err(format!("duplicate submit sequence {sequence}"));
        }
        self.highest_submitted = Some(
            self.highest_submitted
                .map_or(sequence, |old| old.max(sequence)),
        );
        self.pending.insert(sequence, share);
        if self.pending.len() > self.capacity {
            let evicted = self.pending.pop_first().map(|(_, evicted)| evicted);
            if let Some(share) = &evicted {
                self.retired.insert(share.sequence_number);
                if self.retired.len() > self.capacity {
                    self.retired.pop_first();
                }
            }
            return Ok(evicted);
        }
        Ok(None)
    }

    /// Resolve a protocol Success. Empty/repeated responses change no share
    /// history. An exact repeat of the same batch is a no-op. Inconsistent or
    /// unknown counts fail closed and leave every affected share unconfirmed.
    pub fn complete_success(
        &mut self,
        last_sequence_number: u32,
        new_submits_accepted_count: u32,
    ) -> Result<Vec<SubmissionSnapshot>, String> {
        let highest = self
            .highest_submitted
            .ok_or_else(|| "share success before any submission".to_string())?;
        if last_sequence_number > highest {
            return Err(format!(
                "share success names unsent sequence {last_sequence_number}"
            ));
        }
        if new_submits_accepted_count == 0 {
            return Ok(Vec::new());
        }
        if !self.pending.contains_key(&last_sequence_number) {
            if self.retired.contains(&last_sequence_number) {
                self.late_evicted.insert(last_sequence_number);
                return Ok(Vec::new());
            }
            if self.last_success == Some((last_sequence_number, new_submits_accepted_count)) {
                return Ok(Vec::new());
            }
            return Err(format!(
                "share success has unknown sequence {last_sequence_number}"
            ));
        }
        // The named sequence is itself the last correct result acknowledged by
        // this response. For a multi-share batch, all no-error predecessors
        // must be covered by its count. Otherwise the assignment would guess
        // which of multiple eligible predecessors the pool accepted.
        let preceding = new_submits_accepted_count.saturating_sub(1) as usize;
        let eligible_prefix = self.pending.range(..last_sequence_number).count();
        if new_submits_accepted_count > 1 && eligible_prefix != preceding {
            return Err(format!(
                "ambiguous batch prefix: {eligible_prefix} pending before {last_sequence_number}, count {new_submits_accepted_count}"
            ));
        }
        // Later, still-unprocessed submits remain pending. With an exact prefix
        // cardinality the accepted set is fully identified and can be resolved.

        let mut selected: Vec<_> = self
            .pending
            .range(..last_sequence_number)
            .take(preceding)
            .map(|(sequence, _)| *sequence)
            .collect();
        selected.push(last_sequence_number);
        if selected.len() != new_submits_accepted_count as usize {
            return Err(format!(
                "inconsistent batch count {new_submits_accepted_count} ending at {last_sequence_number}"
            ));
        }
        let mut acknowledged = Vec::with_capacity(selected.len());
        for sequence in selected {
            acknowledged.push(
                self.pending
                    .remove(&sequence)
                    .expect("selected pending sequence missing"),
            );
        }
        self.last_success = Some((last_sequence_number, new_submits_accepted_count));
        Ok(acknowledged)
    }

    pub fn complete_error(&mut self, sequence: u32) -> Result<Option<SubmissionSnapshot>, String> {
        if self.retired.contains(&sequence) {
            self.late_evicted.insert(sequence);
            return Ok(None);
        }
        self.pending
            .remove(&sequence)
            .map(Some)
            .ok_or_else(|| format!("share error has unknown sequence {sequence}"))
    }

    pub fn drain_late_evicted(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.late_evicted).into_iter().collect()
    }

    /// Only pending shares can become transport unknown. Already acknowledged
    /// or rejected shares are absent by construction.
    pub fn finish_transport(self) -> Vec<SubmissionSnapshot> {
        self.pending.into_values().collect()
    }

    pub fn pending_sequences(&self) -> Vec<u32> {
        self.pending.keys().copied().collect()
    }
}

fn set_state(
    events: &EventSink,
    snapshot: &SharedConnectionSnapshot,
    session_slot: Option<u16>,
    state: &str,
    detail: Option<String>,
) {
    let (connection_id, event_detail) = {
        let mut snapshot = snapshot.lock().expect("connection snapshot lock");
        snapshot.state = state.into();
        snapshot.detail = detail;
        (snapshot.id.clone(), snapshot.detail.clone())
    };
    events.emit(MinerEvent::ConnectionState {
        connection_id,
        session_slot,
        state: state.into(),
        detail: event_detail,
    });
}
