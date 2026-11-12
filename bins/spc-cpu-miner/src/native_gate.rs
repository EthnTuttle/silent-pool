//! Cancellable public-only wallet exchange and native activation state.
use crate::config::{ConnectionConfig, SourceAuditConfig};
use crate::report::{EventSink, JobSnapshot, MinerEvent, SharedConnectionSnapshot};
use crate::session::JobTemplate;
use crate::worker::WorkerHandle;
use spc_scan::native_activation::{self as wire, CandidateRequest, VerifiedActivation};
use spc_scan::source_feed::ReceivedJob;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, RwLock,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::task::JoinHandle;

pub fn now_ms() -> Result<u64, &'static str> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "native_gate_clock")?
        .as_millis()
        .try_into()
        .map_err(|_| "native_gate_clock")
}

/// Sample monotonic time first: time spent obtaining wall time can only make
/// the derived deadline earlier. Keep the verification sample across handoff.
#[derive(Clone, Copy, Debug)]
struct ClockSample {
    monotonic: Instant,
    wall_ms: u64,
}
impl ClockSample {
    fn now() -> Result<Self, &'static str> {
        let monotonic = Instant::now();
        Ok(Self {
            monotonic,
            wall_ms: now_ms()?,
        })
    }

    fn deadline(self, expires_at_unix_ms: u64) -> Result<Instant, &'static str> {
        let remaining = expires_at_unix_ms
            .checked_sub(self.wall_ms)
            .filter(|n| *n > 0)
            .ok_or("native_gate_expired")?;
        self.monotonic
            .checked_add(Duration::from_millis(remaining))
            .ok_or("native_gate_expired")
    }
}

pub(crate) struct VerifiedCandidate {
    proof: VerifiedActivation,
    verified_at: ClockSample,
    durable: Option<spc_scan::native_evidence::DurableHandle>,
}

fn activation_deadline(
    expires_at_unix_ms: u64,
    admission_deadline: Instant,
    verified_at: ClockSample,
    activated_at: ClockSample,
) -> Result<Instant, &'static str> {
    let deadline = admission_deadline
        .min(verified_at.deadline(expires_at_unix_ms)?)
        .min(activated_at.deadline(expires_at_unix_ms)?);
    if verified_at.monotonic >= admission_deadline || activated_at.monotonic >= deadline {
        return Err("native_gate_expired");
    }
    Ok(deadline)
}

/// The read lease covers exactly one SHA256d call. Revocation waits only for
/// in-flight hashes, never network I/O, and no next hash can cross revocation.
#[derive(Debug)]
pub struct AuditPermit {
    pub proof: VerifiedActivation,
    deadline: Instant,
    valid: AtomicBool,
    lease: RwLock<()>,
}
impl AuditPermit {
    fn new(verified: VerifiedCandidate, admission_deadline: Instant) -> Result<Self, &'static str> {
        let deadline = activation_deadline(
            verified.proof.expires_at_unix_ms,
            admission_deadline,
            verified.verified_at,
            ClockSample::now()?,
        )?;
        Ok(Self {
            proof: verified.proof,
            deadline,
            valid: AtomicBool::new(true),
            lease: RwLock::new(()),
        })
    }
    pub fn valid(&self) -> bool {
        match ClockSample::now() {
            Ok(now) => self.valid_at(now),
            Err(_) => {
                self.valid.store(false, Ordering::SeqCst);
                false
            }
        }
    }
    fn valid_at(&self, now: ClockSample) -> bool {
        if now.monotonic >= self.deadline || now.wall_ms >= self.proof.expires_at_unix_ms {
            // A later clock rollback must not resurrect an observed expiry.
            self.valid.store(false, Ordering::SeqCst);
        }
        self.valid.load(Ordering::SeqCst)
    }
    pub fn with_valid<T>(&self, hash: impl FnOnce() -> T) -> Option<T> {
        let _guard = self.lease.read().ok()?;
        self.valid().then(hash)
    }
    pub(crate) fn revoke(&self) {
        let _guard = self.lease.write().unwrap_or_else(|e| e.into_inner());
        self.valid.store(false, Ordering::SeqCst);
    }
}

struct Candidate {
    request: CandidateRequest,
    job: Arc<JobTemplate>,
    extranonce_size: usize,
    admission_deadline: Instant,
}
pub struct CandidateGate {
    config: ConnectionConfig,
    epoch: u64,
    candidate: Option<Candidate>,
    pending: Option<JoinHandle<Result<VerifiedCandidate, &'static str>>>,
    active: Option<Arc<AuditPermit>>,
    retry_at: Option<Instant>,
    capture: Option<Arc<crate::evidence::CaptureStore>>,
    current: Option<Arc<AtomicBool>>,
}
impl CandidateGate {
    pub fn new(config: &ConnectionConfig) -> Self {
        Self {
            config: config.clone(),
            epoch: 0,
            candidate: None,
            pending: None,
            active: None,
            retry_at: None,
            capture: None,
            current: None,
        }
    }
    pub fn required(&self) -> bool {
        self.config.source_audit.required()
    }
    pub(crate) fn with_capture(
        mut self,
        capture: Option<Arc<crate::evidence::CaptureStore>>,
    ) -> Self {
        self.capture = capture;
        self
    }
    pub fn invalidate(
        &mut self,
        workers: &[Arc<WorkerHandle>],
        snapshot: &SharedConnectionSnapshot,
    ) -> anyhow::Result<()> {
        if !self.required() {
            return Ok(());
        }
        if let Some(task) = self.pending.take() {
            task.abort();
        }
        if let Some(current) = self.current.take() {
            current.store(false, Ordering::SeqCst);
        }
        if let Some(permit) = self.active.take() {
            permit.revoke();
        }
        self.candidate = None;
        self.retry_at = None;
        for worker in workers {
            worker.clear_work();
        }
        let mut state = snapshot.lock().expect("snapshot lock");
        state.active_job = None;
        state.candidate_job = None;
        state.source_audit_state = Some("awaiting_candidate".into());
        state.source_audit_ready = false;
        self.epoch = self
            .epoch
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("native candidate epoch exhausted"))?;
        Ok(())
    }
    pub fn start(
        &mut self,
        job: Arc<JobTemplate>,
        extranonce_size: usize,
        session: u16,
        snapshot: &SharedConnectionSnapshot,
        events: &EventSink,
    ) -> anyhow::Result<()> {
        let SourceAuditConfig::Required {
            run_id,
            miner_id,
            wallet_instance,
            peer_generation,
            connection_config_digest,
            ..
        } = &self.config.source_audit
        else {
            return Ok(());
        };
        if self.pending.is_some() || self.active.is_some() {
            return Err(anyhow::anyhow!("native gate candidate was not invalidated"));
        }
        let admitted_at = ClockSample::now().map_err(anyhow::Error::msg)?;
        let admission_deadline = admitted_at.monotonic + Duration::from_secs(wire::DEADLINE_SECS);
        let request = CandidateRequest {
            schema: wire::SCHEMA.into(),
            run_id: run_id.clone(),
            miner_id: miner_id.clone(),
            connection_id: self.config.id.to_string(),
            identity: self.config.identity.to_string(),
            wallet_instance: *wallet_instance,
            peer_generation: *peer_generation,
            connection_config_digest: *connection_config_digest,
            expires_at_unix_ms: admitted_at
                .wall_ms
                .checked_add(wire::DEADLINE_SECS * 1000)
                .ok_or_else(|| anyhow::anyhow!("native gate clock overflow"))?,
            session_epoch: u64::from(session),
            candidate_epoch: self.epoch,
            nonce: rand::random(),
            job: ReceivedJob {
                job_id: job.job_id.into(),
                prev_hash: job.prev_hash,
                version: job.version,
                nbits: job.nbits,
                ntime: job.min_ntime,
                coinbase_prefix: job.coinbase_tx_prefix.clone(),
                coinbase_suffix: job.coinbase_tx_suffix.clone(),
                merkle_path: job.merkle_path.clone(),
                share_target_be: job.share_target.to_be_bytes(),
                pool_extranonce: job.extranonce_prefix.clone(),
                miner_extranonce_len: extranonce_size.try_into()?,
            },
        };
        let digest = request.signing_digest().map_err(anyhow::Error::msg)?;
        events.emit(MinerEvent::JobCandidate {
            connection_id: self.config.id.clone(),
            session_slot: session,
            candidate_epoch: self.epoch,
            request_digest: hex::encode(digest),
            job: JobSnapshot::from(job.as_ref()),
        });
        {
            let mut state = snapshot.lock().expect("snapshot lock");
            state.candidate_job = Some(JobSnapshot::from(job.as_ref()));
            state.active_job = None;
            state.source_audit_state = Some("pending".into());
            state.source_audit_ready = false;
        }
        let task_request = request.clone();
        let config = self.config.source_audit.clone();
        let capture = self.capture.clone();
        let current = Arc::new(AtomicBool::new(true));
        self.current = Some(current.clone());
        self.pending = Some(tokio::spawn(async move {
            tokio::time::timeout_at(
                admission_deadline.into(),
                exchange(&config, &task_request, capture, admission_deadline, current),
            )
            .await
            .unwrap_or(Err("native_gate_timeout"))
        }));
        self.candidate = Some(Candidate {
            request,
            job,
            extranonce_size,
            admission_deadline,
        });
        Ok(())
    }
    pub(crate) async fn completed(&mut self) -> Result<VerifiedCandidate, &'static str> {
        let Some(task) = self.pending.as_mut() else {
            return std::future::pending().await;
        };
        let result = task.await.unwrap_or(Err("native_gate_cancelled"));
        self.pending = None;
        result
    }
    pub fn pending(&self) -> bool {
        self.pending.is_some()
    }
    pub fn deadline(&self) -> Option<tokio::time::Instant> {
        if self.pending.is_some() {
            self.candidate.as_ref().map(|c| c.admission_deadline.into())
        } else {
            self.active
                .as_ref()
                .map(|p| p.deadline.into())
                .or_else(|| self.retry_at.map(Into::into))
        }
    }
    pub fn accept_share(&self, job: &Arc<JobTemplate>) -> bool {
        !self.required()
            || (self.active.as_ref().is_some_and(|p| p.valid())
                && self
                    .candidate
                    .as_ref()
                    .is_some_and(|c| Arc::ptr_eq(&c.job, job)))
    }
    pub(crate) fn finish(
        &mut self,
        result: Result<VerifiedCandidate, &'static str>,
        workers: &[Arc<WorkerHandle>],
        snapshot: &SharedConnectionSnapshot,
        events: &EventSink,
        session: u16,
    ) {
        let checked = (|| {
            let verified = result?;
            let proof = &verified.proof;
            let candidate = self.candidate.as_ref().ok_or("native_gate_superseded")?;
            if !self
                .current
                .as_ref()
                .is_some_and(|c| c.load(Ordering::SeqCst))
            {
                return Err("native_gate_superseded");
            }
            if proof.candidate_epoch != self.epoch
                || proof.request_digest != candidate.request.signing_digest()?
            {
                return Err("native_gate_superseded");
            }
            let durable = verified.durable.clone();
            if self.capture.is_some()
                && durable
                    .as_ref()
                    .is_none_or(|h| h.request_digest != hex::encode(proof.request_digest))
            {
                return Err("native_evidence_required");
            }
            let permit = Arc::new(AuditPermit::new(verified, candidate.admission_deadline)?);
            let p = &permit.proof;
            if !events.try_emit(MinerEvent::SourceAuditCommitted {
                connection_id: self.config.id.clone(),
                session_slot: session,
                candidate_epoch: self.epoch,
                capability: wire::CAPABILITY.into(),
                job_id: p.job_id,
                generation: p.generation,
                request_digest: hex::encode(p.request_digest),
                checkpoint_digest: hex::encode(p.checkpoint_digest),
                consumed_digest: hex::encode(p.consumed_digest),
                committed_digest: hex::encode(p.committed_digest),
                expires_at_unix_ms: p.expires_at_unix_ms,
                window: p.window,
                durable_evidence: durable,
            }) || !events.try_emit(MinerEvent::JobActivated {
                connection_id: self.config.id.clone(),
                job: JobSnapshot::from(candidate.job.as_ref()),
            }) {
                return Err("native_gate_event_backpressure");
            }
            // Both public ordering events are queued BEFORE any worker can hash.
            self.active = Some(Arc::clone(&permit));
            {
                let mut state = snapshot.lock().expect("snapshot lock");
                state.active_job = Some(JobSnapshot::from(candidate.job.as_ref()));
                state.candidate_job = None;
                state.source_audit_state = Some(
                    if p.window.underfilled {
                        "committed_underfilled"
                    } else {
                        "committed_full"
                    }
                    .into(),
                );
                state.source_audit_ready = true;
            }
            for worker in workers {
                worker.publish_audited(
                    session,
                    candidate.extranonce_size,
                    Arc::clone(&candidate.job),
                    Arc::clone(&permit),
                );
            }
            Ok::<_, &'static str>(())
        })();
        if let Err(code) = checked {
            // A complete but unavailable feed may become ready without a new
            // pool job. Retry with a NEW epoch/nonce, one in flight, no hashes.
            self.retry_at = Instant::now().checked_add(Duration::from_secs(1));
            let mut state = snapshot.lock().expect("snapshot lock");
            state.active_job = None;
            state.source_audit_state = Some("unknown".into());
            state.source_audit_ready = false;
            drop(state);
            events.emit(MinerEvent::SourceAuditUnknown {
                connection_id: self.config.id.clone(),
                session_slot: session,
                candidate_epoch: self.epoch,
                code: code.into(),
            });
        }
    }
}
impl Drop for CandidateGate {
    fn drop(&mut self) {
        if let Some(current) = self.current.take() {
            current.store(false, Ordering::SeqCst);
        }
        if let Some(t) = self.pending.take() {
            t.abort();
        }
        if let Some(p) = self.active.take() {
            p.revoke();
        }
    }
}

fn start_ticks(pid: u32) -> Result<u64, &'static str> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .map_err(|_| "native_gate_wallet_process")?;
    stat.rsplit_once(')')
        .ok_or("native_gate_wallet_process")?
        .1
        .split_whitespace()
        .nth(19)
        .ok_or("native_gate_wallet_process")?
        .parse()
        .map_err(|_| "native_gate_wallet_process")
}
fn private_socket(config: &SourceAuditConfig) -> Result<(), &'static str> {
    let SourceAuditConfig::Required {
        wallet_socket,
        wallet_process,
        wallet_socket_device,
        wallet_socket_inode,
        ..
    } = config
    else {
        return Err("native_gate_not_required");
    };
    let meta = std::fs::symlink_metadata(wallet_socket).map_err(|_| "native_gate_unavailable")?;
    let parent =
        std::fs::symlink_metadata(wallet_socket.parent().ok_or("native_gate_socket_path")?)
            .map_err(|_| "native_gate_unavailable")?;
    if !meta.file_type().is_socket()
        || meta.mode() & 0o777 != 0o600
        || meta.uid() != wallet_process.uid
        || !parent.is_dir()
        || parent.mode() & 0o777 != 0o700
        || parent.uid() != wallet_process.uid
        || meta.dev() != *wallet_socket_device
        || meta.ino() != *wallet_socket_inode
        || start_ticks(wallet_process.pid)? != wallet_process.start_time_ticks
        || std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .map_err(|_| "native_gate_clock")?
            .trim()
            != wallet_process.boot_id
    {
        return Err("native_gate_wallet_binding");
    }
    Ok(())
}
async fn exchange(
    config: &SourceAuditConfig,
    request: &CandidateRequest,
    capture: Option<Arc<crate::evidence::CaptureStore>>,
    admission_deadline: Instant,
    current: Arc<AtomicBool>,
) -> Result<VerifiedCandidate, &'static str> {
    private_socket(config)?;
    let SourceAuditConfig::Required {
        wallet_socket,
        wallet_process,
        collector_pubkey,
        pool_pubkey,
        ..
    } = config
    else {
        return Err("native_gate_not_required");
    };
    let keys = wire::VerificationKeys::for_regtest_identity(
        &request.identity,
        collector_pubkey,
        pool_pubkey,
    )?;
    let mut stream = UnixStream::connect(wallet_socket)
        .await
        .map_err(|_| "native_gate_unavailable")?;
    let peer = stream.peer_cred().map_err(|_| "native_gate_peer")?;
    if peer.uid() != wallet_process.uid
        || peer.pid().and_then(|p| u32::try_from(p).ok()) != Some(wallet_process.pid)
    {
        return Err("native_gate_peer");
    }
    private_socket(config)?;
    let request_bytes = wire::encode_request(request)?;
    stream
        .write_u32_le(request_bytes.len() as u32)
        .await
        .map_err(|_| "native_gate_transport")?;
    stream
        .write_all(&request_bytes)
        .await
        .map_err(|_| "native_gate_transport")?;
    stream.flush().await.map_err(|_| "native_gate_transport")?;
    let len = stream
        .read_u32_le()
        .await
        .map_err(|_| "native_gate_transport")? as usize;
    if len == 0 || len > wire::MAX_RESPONSE_BYTES {
        return Err("native_gate_frame_bound");
    }
    let mut bytes = vec![0; len];
    stream
        .read_exact(&mut bytes)
        .await
        .map_err(|_| "native_gate_transport")?;
    private_socket(config)?;
    let verified_at = ClockSample::now()?;
    let proof = wire::verify_response(
        request,
        &wire::decode_response(&bytes)?,
        &keys,
        verified_at.wall_ms,
    )?;
    let deadline = admission_deadline.min(verified_at.deadline(proof.expires_at_unix_ms)?);
    let durable = match capture {
        Some(store) => Some(
            store
                .retain(
                    request_bytes,
                    bytes,
                    keys,
                    verified_at.wall_ms,
                    deadline,
                    current.clone(),
                )
                .await?,
        ),
        None => None,
    };
    if !current.load(Ordering::SeqCst) || Instant::now() >= deadline {
        return Err("native_evidence_expired_or_cancelled");
    }
    Ok(VerifiedCandidate {
        proof,
        verified_at,
        durable,
    })
}

#[cfg(test)]
#[path = "native_gate/clock_tests.rs"]
mod clock_tests;
