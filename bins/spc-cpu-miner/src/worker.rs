use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use bitcoin::BlockHash;
use tokio::sync::mpsc;

use crate::budget::{HashBudget, Pacer};
use crate::config::WorkerConfig;
use crate::hash::{assemble_coinbase, HeaderPreimage, SearchCursor};
use crate::ids::{ConnectionId, JobId, WorkerId};
use crate::report::{EventSink, MinerEvent, WorkerSnapshot};
use crate::session::JobTemplate;
use crate::target::meets_target;

#[derive(Debug)]
pub struct WorkAssignment {
    pub sequence: u64,
    pub session_slot: u16,
    pub extranonce_size: usize,
    pub job: Arc<JobTemplate>,
    pub audit: Option<Arc<crate::native_gate::AuditPermit>>,
}

#[derive(Debug)]
pub struct FoundShare {
    pub worker_id: WorkerId,
    pub session_slot: u16,
    pub job: Arc<JobTemplate>,
    pub header: [u8; 80],
    pub hash: BlockHash,
    pub extranonce: Vec<u8>,
    pub nonce: u32,
    pub ntime: u32,
    pub version: u32,
}

#[derive(Debug)]
struct DynamicWorker {
    budget: HashBudget,
    paused: bool,
}

#[derive(Debug, Default)]
struct WorkerStats {
    active_job_id: Option<JobId>,
    active_sequence: u64,
    stale_cancelled: u64,
}

#[derive(Debug)]
struct PreparedHeader {
    sequence: u64,
    ntime: u32,
    extranonce: Vec<u8>,
    preimage: HeaderPreimage,
}

#[derive(Debug)]
pub struct WorkerHandle {
    pub id: WorkerId,
    pub connection_id: ConnectionId,
    pub worker_slot: u16,
    config: WorkerConfig,
    dynamic: RwLock<DynamicWorker>,
    assignment: RwLock<Option<Arc<WorkAssignment>>>,
    next_assignment: AtomicU64,
    cancelled: AtomicBool,
    wake: (Mutex<()>, Condvar),
    stats: Mutex<WorkerStats>,
    found_queue_depth: AtomicU64,
    pacer: Mutex<Pacer>,
    join: Mutex<Option<JoinHandle<()>>>,
}

impl WorkerHandle {
    pub fn start(
        worker_slot: u16,
        worker_index: u32,
        worker_count: u32,
        config: WorkerConfig,
        connection_id: ConnectionId,
        found: mpsc::Sender<FoundShare>,
        events: EventSink,
    ) -> Arc<Self> {
        let handle = Arc::new(Self {
            id: config.id.clone(),
            connection_id,
            worker_slot,
            dynamic: RwLock::new(DynamicWorker {
                budget: config.budget.clone(),
                paused: false,
            }),
            assignment: RwLock::new(None),
            next_assignment: AtomicU64::new(1),
            cancelled: AtomicBool::new(false),
            wake: (Mutex::new(()), Condvar::new()),
            stats: Mutex::new(WorkerStats::default()),
            found_queue_depth: AtomicU64::new(0),
            pacer: Mutex::new(Pacer::new(Instant::now())),
            config,
            join: Mutex::new(None),
        });
        let thread_handle = Arc::clone(&handle);
        let thread_name = handle.id.clone();
        let join = std::thread::Builder::new()
            .name(format!("spc-hash-{thread_name}"))
            .spawn(move || thread_handle.run(worker_index, worker_count, found, events))
            .expect("failed to start bounded CPU hash worker");
        *handle.join.lock().expect("worker join lock") = Some(join);
        handle
    }

    pub fn publish(&self, session_slot: u16, extranonce_size: usize, job: Arc<JobTemplate>) {
        self.publish_inner(session_slot, extranonce_size, job, None);
    }

    pub(crate) fn publish_audited(
        &self,
        session_slot: u16,
        extranonce_size: usize,
        job: Arc<JobTemplate>,
        permit: Arc<crate::native_gate::AuditPermit>,
    ) {
        self.publish_inner(session_slot, extranonce_size, job, Some(permit));
    }

    fn publish_inner(
        &self,
        session_slot: u16,
        extranonce_size: usize,
        job: Arc<JobTemplate>,
        audit: Option<Arc<crate::native_gate::AuditPermit>>,
    ) {
        let mut assignment = self.assignment.write().expect("worker assignment lock");
        if self.cancelled.load(Ordering::SeqCst) {
            return;
        }
        let sequence = self.next_assignment.fetch_add(1, Ordering::SeqCst);
        *assignment = Some(Arc::new(WorkAssignment {
            sequence,
            session_slot,
            extranonce_size,
            job,
            audit,
        }));
        drop(assignment);
        self.wake.1.notify_all();
    }

    pub fn clear_work(&self) {
        let previous = {
            let mut assignment = self.assignment.write().expect("worker assignment lock");
            self.next_assignment.fetch_add(1, Ordering::SeqCst);
            assignment.take()
        };
        // Never hold assignment/stats locks while fencing an in-flight hash.
        // This also reaches a permit when stop originates outside the actor.
        if let Some(permit) = previous.as_ref().and_then(|work| work.audit.as_ref()) {
            permit.revoke();
        }
        let mut stats = self.stats.lock().expect("worker stats lock");
        if stats.active_job_id.take().is_some() {
            stats.stale_cancelled = stats.stale_cancelled.saturating_add(1);
        }
        drop(stats);
        self.wake.1.notify_all();
    }

    pub fn set_budget(&self, budget: HashBudget) {
        self.dynamic.write().expect("worker dynamic lock").budget = budget;
        self.wake.1.notify_all();
    }

    pub fn set_paused(&self, paused: bool) {
        self.dynamic.write().expect("worker dynamic lock").paused = paused;
        self.wake.1.notify_all();
    }

    pub fn stop(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.clear_work();
        self.wake.1.notify_all();
    }

    pub fn join(&self) {
        if let Some(join) = self.join.lock().expect("worker join lock").take() {
            let _ = join.join();
        }
    }

    #[cfg(test)]
    pub(crate) fn audit_permit(&self) -> Option<Arc<crate::native_gate::AuditPermit>> {
        self.assignment
            .read()
            .unwrap()
            .as_ref()
            .and_then(|work| work.audit.clone())
    }

    pub fn snapshot(&self) -> WorkerSnapshot {
        let dynamic = self.dynamic.read().expect("worker dynamic lock");
        let stats = self.stats.lock().expect("worker stats lock");
        let assignment = self.assignment.read().expect("worker assignment lock");
        let now = Instant::now();
        let pacer = self.pacer.lock().expect("worker pacer lock");
        let attempted = pacer.attempted();
        let elapsed = pacer.elapsed(now);
        let achieved = if elapsed.is_zero() {
            0
        } else {
            (u128::from(attempted) * 1_000_000_000 / elapsed.as_nanos()).min(u128::from(u64::MAX))
                as u64
        };
        let newest = self
            .next_assignment
            .load(Ordering::SeqCst)
            .saturating_sub(1);
        let work_backlog = if assignment.is_some() {
            newest.saturating_sub(stats.active_sequence)
        } else {
            0
        };
        WorkerSnapshot {
            id: self.id.clone(),
            connection_id: self.connection_id.clone(),
            paused: dynamic.paused,
            requested: dynamic.budget.clone(),
            attempted: attempted.to_string(),
            elapsed_ms: elapsed.as_millis().min(u128::from(u64::MAX)) as u64,
            achieved_hps: achieved,
            budget_hps: dynamic.budget.hashes_per_second,
            saturated: pacer.saturated(),
            queue_depth: work_backlog + self.found_queue_depth.load(Ordering::SeqCst),
            active_job_id: assignment
                .as_ref()
                .filter(|work| {
                    work.sequence == newest && work.audit.as_ref().is_none_or(|p| p.valid())
                })
                .map(|work| work.job.job_id),
            stale_cancelled: stats.stale_cancelled,
        }
    }

    fn run(
        self: Arc<Self>,
        worker_index: u32,
        worker_count: u32,
        found: mpsc::Sender<FoundShare>,
        events: EventSink,
    ) {
        let mut pacer = Pacer::new(Instant::now());
        let mut cursor: Option<SearchCursor> = None;
        let mut cursor_session = None;
        let mut prepared: Option<PreparedHeader> = None;
        let mut exhaustion_reported = false;
        let mut first_hash_sequence = None;

        while !self.cancelled.load(Ordering::SeqCst) {
            if self.dynamic.read().expect("worker dynamic lock").paused {
                pacer.pause(Instant::now());
                self.wait_work();
                self.store_pacer(&pacer);
                continue;
            }
            let budget = self
                .dynamic
                .read()
                .expect("worker dynamic lock")
                .budget
                .clone();
            let now = Instant::now();
            if !budget.allows(pacer.attempted()) {
                pacer.pause(now);
                self.wait_work();
                self.store_pacer(&pacer);
                continue;
            }
            let assignment = self
                .assignment
                .read()
                .expect("worker assignment lock")
                .clone();
            let Some(assignment) = assignment else {
                prepared = None;
                pacer.pause(now);
                self.wait_work();
                self.store_pacer(&pacer);
                continue;
            };
            if assignment.audit.as_ref().is_some_and(|p| !p.valid()) {
                pacer.pause(now);
                self.wait_work();
                self.store_pacer(&pacer);
                continue;
            }
            pacer.resume(now);
            {
                let mut stats = self.stats.lock().expect("worker stats lock");
                if stats.active_sequence != assignment.sequence {
                    if stats.active_job_id.is_some() {
                        stats.stale_cancelled = stats.stale_cancelled.saturating_add(1);
                    }
                    stats.active_sequence = assignment.sequence;
                    stats.active_job_id = Some(assignment.job.job_id);
                    exhaustion_reported = false;
                }
            }
            if cursor_session != Some(assignment.session_slot)
                || cursor
                    .as_ref()
                    .is_none_or(|cursor| cursor.extranonce().len() != assignment.extranonce_size)
            {
                match SearchCursor::new(
                    self.worker_slot,
                    assignment.session_slot,
                    worker_index,
                    worker_count,
                    assignment.extranonce_size,
                ) {
                    Ok(new_cursor) => {
                        cursor = Some(new_cursor);
                        cursor_session = Some(assignment.session_slot);
                        prepared = None;
                    }
                    Err(_) => {
                        events.emit(MinerEvent::WorkerSearchExhausted {
                            connection_id: self.connection_id.clone(),
                            worker_id: self.id.clone(),
                            job_id: assignment.job.job_id,
                            ntime: assignment.job.min_ntime,
                            reason: "negotiated extranonce cannot support unique search".into(),
                        });
                        self.wait_work();
                        continue;
                    }
                }
            }
            let current_cursor = cursor.as_mut().expect("cursor initialized above");
            let job = Arc::clone(&assignment.job);
            let ntime = self.config.ntime_policy.ntime(
                job.min_ntime,
                job.activated_at,
                Instant::now(),
                self.config.max_ntime_roll_seconds,
            );
            let dirty = prepared.as_ref().is_none_or(|prepared| {
                prepared.sequence != assignment.sequence
                    || prepared.ntime != ntime
                    || prepared.extranonce != current_cursor.extranonce()
            });
            if dirty {
                let coinbase = assemble_coinbase(
                    &job.coinbase_tx_prefix,
                    &job.extranonce_prefix,
                    current_cursor.extranonce(),
                    &job.coinbase_tx_suffix,
                );
                prepared = Some(PreparedHeader {
                    sequence: assignment.sequence,
                    ntime,
                    extranonce: current_cursor.extranonce().to_vec(),
                    preimage: HeaderPreimage::new(
                        job.version,
                        job.prev_hash,
                        &coinbase,
                        &job.merkle_path,
                        ntime,
                        job.nbits,
                    ),
                });
                exhaustion_reported = false;
            }
            let preimage = &prepared.as_ref().expect("prepared above").preimage;
            let latest_sequence = self
                .next_assignment
                .load(Ordering::SeqCst)
                .saturating_sub(1);
            if latest_sequence != assignment.sequence {
                continue;
            }

            let available = pacer.available_attempts(&budget, Instant::now());
            if available == 0 {
                let delay = pacer.required_delay(&budget, Instant::now());
                if !delay.is_zero() {
                    pacer.note_pace_sleep();
                    self.wait_duration(delay);
                }
                self.store_pacer(&pacer);
                continue;
            }
            let batch = budget
                .remaining(pacer.attempted())
                .min(u64::from(self.config.batch_size))
                .min(available);
            if batch == 0 {
                continue;
            }
            let mut attempts = 0u64;
            let mut revoked = false;
            for _ in 0..batch {
                if self.cancelled.load(Ordering::SeqCst)
                    || self
                        .next_assignment
                        .load(Ordering::SeqCst)
                        .saturating_sub(1)
                        != assignment.sequence
                    || self.dynamic.read().expect("worker dynamic lock").paused
                {
                    revoked = true;
                    break;
                }
                let Some(nonce) = current_cursor.next_nonce() else {
                    break;
                };
                let hashed = match &assignment.audit {
                    Some(permit) => permit.with_valid(|| preimage.hash_nonce(nonce)),
                    None => Some(preimage.hash_nonce(nonce)),
                };
                let Some((header, hash)) = hashed else {
                    revoked = true;
                    break;
                };
                attempts += 1;
                if first_hash_sequence != Some(assignment.sequence) {
                    first_hash_sequence = Some(assignment.sequence);
                    if let Some(permit) = &assignment.audit {
                        events.emit(MinerEvent::AuditFirstHash {
                            connection_id: self.connection_id.clone(),
                            worker_id: self.id.clone(),
                            session_slot: assignment.session_slot,
                            candidate_epoch: permit.proof.candidate_epoch,
                            request_digest: hex::encode(permit.proof.request_digest),
                            job_id: job.job_id,
                        });
                    }
                }
                if !meets_target(&hash, job.share_target) {
                    continue;
                }
                self.found_queue_depth.fetch_add(1, Ordering::SeqCst);
                let share = FoundShare {
                    worker_id: self.id.clone(),
                    session_slot: assignment.session_slot,
                    job: Arc::clone(&job),
                    header,
                    hash,
                    extranonce: current_cursor.extranonce().to_vec(),
                    nonce,
                    ntime,
                    version: job.version,
                };
                // Found solutions precede pacing sleeps. A full bounded queue
                // applies explicit network backpressure, never candidate pacing.
                let send_result = found.blocking_send(share);
                self.found_queue_depth.fetch_sub(1, Ordering::SeqCst);
                if send_result.is_err() {
                    self.stop();
                }
                break;
            }
            pacer.note_attempts(attempts);
            if revoked {
                self.store_pacer(&pacer);
                continue;
            }
            if attempts == 0 {
                if current_cursor.roll_extranonce() {
                    prepared = None;
                    self.store_pacer(&pacer);
                    continue;
                }
                let later = Instant::now();
                let later_ntime = self.config.ntime_policy.ntime(
                    job.min_ntime,
                    job.activated_at,
                    later,
                    self.config.max_ntime_roll_seconds,
                );
                if later_ntime != ntime {
                    current_cursor.reset_nonce_for_new_time();
                    prepared = None;
                    continue;
                }
                if !exhaustion_reported {
                    events.emit(MinerEvent::WorkerSearchExhausted {
                        connection_id: self.connection_id.clone(),
                        worker_id: self.id.clone(),
                        job_id: job.job_id,
                        ntime,
                        reason: "nonce and miner-controlled extranonce exhausted at current ntime"
                            .into(),
                    });
                    exhaustion_reported = true;
                }
                self.wait_work();
                self.store_pacer(&pacer);
                continue;
            }
            pacer.finish_sample(Instant::now(), &budget);
            self.store_pacer(&pacer);
        }
        self.store_pacer(&pacer);
    }

    fn store_pacer(&self, pacer: &Pacer) {
        *self.pacer.lock().expect("worker pacer lock") = pacer.clone();
    }

    fn wait_work(&self) {
        self.wait_duration(Duration::from_millis(20));
    }

    /// A condvar wait, not an uninterruptible sleep: budget changes, pause,
    /// replacement work and stop all wake here and are observed next loop.
    fn wait_duration(&self, duration: Duration) {
        let guard = self.wake.0.lock().expect("worker wake lock");
        let _ = self
            .wake
            .1
            .wait_timeout(guard, duration)
            .expect("worker wake lock poisoned");
    }
}
