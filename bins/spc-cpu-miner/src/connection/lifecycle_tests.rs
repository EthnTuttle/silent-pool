//! Deterministic backpressure at the real session/writer task boundary. A
//! one-byte duplex transport MUST partially write and return Pending. No OS
//! buffer saturation or success-only mock is used. Wallet signatures are test
//! fixtures; protocol, audit gate, worker hashing and stop-all are production.
use super::*;
use crate::budget::HashBudget;
use crate::config::{ReconnectConfig, SourceAuditConfig, WorkerConfig};
use crate::hash::NtimePolicy;
use crate::report::OutputMessage;
use bitcoin::secp256k1::{Keypair, Secp256k1, SecretKey};
use spc_scan::native_activation::{self as wire, CandidateRequest, ProcessIdentity};
use spc_scan::source_feed::{self as feed, Checkpoint, CommittedEvidence, Consumed, TraceSeal};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::task::Poll;
use std::time::Instant;
use stratum_core::binary_sv2::{Seq0255, Sv2Option, B064K};
use stratum_core::framing_sv2::framing::EncodableFrame;
use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream};
use tokio::net::UnixListener;
use tokio::sync::oneshot;

#[derive(Clone, Debug)]
struct PendingWrite {
    kind: u8,
    written: usize,
    total: usize,
}
struct Probe {
    pending: watch::Sender<Option<PendingWrite>>,
    dropped: AtomicBool,
    completed: AtomicUsize,
    closed: AtomicBool,
    calls: Mutex<Vec<u8>>,
}
struct BoundedWriter {
    stream: DuplexStream,
    probe: Arc<Probe>,
}
impl outbound::FrameWriter for BoundedWriter {
    async fn write(&mut self, frame: outbound::OwnedFrame) -> anyhow::Result<()> {
        let kind = frame.header().msg_type();
        self.probe.calls.lock().unwrap().push(kind);
        let bytes = frame.as_bytes();
        let mut written = 0;
        while written < bytes.len() {
            let n = std::future::poll_fn(|cx| {
                match Pin::new(&mut self.stream).poll_write(cx, &bytes[written..]) {
                    Poll::Pending => {
                        self.probe.pending.send_replace(Some(PendingWrite {
                            kind,
                            written,
                            total: bytes.len(),
                        }));
                        Poll::Pending
                    }
                    result => result,
                }
            })
            .await?;
            anyhow::ensure!(n > 0, "test transport write zero");
            written += n;
        }
        self.probe.completed.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn close(&mut self) -> anyhow::Result<()> {
        self.probe.closed.store(true, Ordering::SeqCst);
        self.stream.shutdown().await?;
        Ok(())
    }
}
impl Drop for BoundedWriter {
    fn drop(&mut self) {
        self.probe.dropped.store(true, Ordering::SeqCst);
    }
}
fn bounded_writer() -> (
    BoundedWriter,
    DuplexStream,
    Arc<Probe>,
    watch::Receiver<Option<PendingWrite>>,
) {
    let (stream, peer) = tokio::io::duplex(1);
    let (pending, observed) = watch::channel(None);
    let probe = Arc::new(Probe {
        pending,
        dropped: AtomicBool::new(false),
        completed: AtomicUsize::new(0),
        closed: AtomicBool::new(false),
        calls: Mutex::new(vec![]),
    });
    (
        BoundedWriter {
            stream,
            probe: probe.clone(),
        },
        peer,
        probe,
        observed,
    )
}
async fn pending_write(rx: &mut watch::Receiver<Option<PendingWrite>>, kind: u8) {
    let state = timeout(
        Duration::from_secs(2),
        rx.wait_for(|p| p.as_ref().is_some_and(|p| p.kind == kind)),
    )
    .await
    .unwrap()
    .unwrap()
    .clone()
    .unwrap();
    assert!(
        state.written > 0 && state.written < state.total,
        "must observe a real partial, pending write: {state:?}"
    );
}
fn frame<T: binary_sv2::Serialize + binary_sv2::GetSize>(
    message: T,
    kind: u8,
    channel: bool,
) -> SerializedFrame {
    let message = MessageFrame::from_message(message, kind, 0, channel).unwrap();
    let mut bytes = vec![0; message.encoded_length()];
    message.encode_into(&mut bytes).unwrap();
    SerializedFrame::from_bytes(bytes).unwrap()
}
async fn read_kind(peer: &mut DuplexStream) -> u8 {
    let mut header = [0u8; 6];
    timeout(Duration::from_secs(2), peer.read_exact(&mut header))
        .await
        .unwrap()
        .unwrap();
    let len = u32::from_le_bytes([header[3], header[4], header[5], 0]) as usize;
    let mut payload = vec![0; len];
    timeout(Duration::from_secs(2), peer.read_exact(&mut payload))
        .await
        .unwrap()
        .unwrap();
    header[2]
}
fn key(n: u8) -> Keypair {
    Keypair::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&[n; 32]).unwrap())
}
fn approved(request: &CandidateRequest) -> wire::Response {
    let (wallet, collector, pool) = (key(7), key(8), key(9));
    let mut seal = TraceSeal {
        session: [1; 16],
        seq: 1,
        digest: [2; 32],
        signature: String::new(),
    };
    seal.signature = feed::sign(seal.signing_digest(), &pool);
    let mut checkpoint = Checkpoint {
        attempt: request.nonce,
        wire_job_digest: request.job.digest(),
        job_id: request.full_job_id().unwrap(),
        generation: u64::from(request.job.job_id),
        source_watermark: 1,
        history_watermark: 1,
        bundle_digest: [3; 32],
        history_digest: [4; 32],
        expires_at_unix_ms: request.expires_at_unix_ms,
        complete: true,
        pool: seal,
        signature: String::new(),
    };
    checkpoint.signature = feed::sign(checkpoint.signing_digest(), &collector);
    let mut consumed = Consumed {
        attempt: request.nonce,
        job_id: checkpoint.job_id,
        generation: checkpoint.generation,
        bundle_digest: checkpoint.bundle_digest,
        history_digest: checkpoint.history_digest,
        checkpoint_digest: checkpoint.signing_digest(),
        window: spc_scan::source_audit::WindowFacts::from_work(9, 9),
        signature: String::new(),
    };
    consumed.signature = feed::sign(consumed.signing_digest(), &wallet);
    let committed = CommittedEvidence {
        attempt: request.nonce,
        ack_digest: consumed.signing_digest(),
        signature: feed::sign(
            feed::committed_digest(request.nonce, consumed.signing_digest()),
            &collector,
        ),
    };
    let mut receipt = wire::ActivationReceipt {
        request_digest: request.signing_digest().unwrap(),
        candidate_epoch: request.candidate_epoch,
        nonce: request.nonce,
        evidence: feed::VerifiedSourceEvidence {
            schema: feed::SCHEMA.into(),
            checkpoint,
            consumed,
            committed,
        },
        wallet_signature: String::new(),
    };
    receipt.wallet_signature =
        feed::sign(wire::activation_digest(request, &receipt).unwrap(), &wallet);
    wire::Response::Approved {
        schema: wire::SCHEMA.into(),
        capability: wire::CAPABILITY.into(),
        receipt,
    }
}
struct PendingAudit {
    request: CandidateRequest,
    release: oneshot::Sender<()>,
}
struct Fixture {
    _dir: tempfile::TempDir,
    handle: Option<ConnectionHandle>,
    worker: Arc<WorkerHandle>,
    snapshot: SharedConnectionSnapshot,
    inbound: mpsc::Sender<anyhow::Result<SerializedFrame>>,
    peer: DuplexStream,
    probe: Arc<Probe>,
    pending_write: watch::Receiver<Option<PendingWrite>>,
    requests: mpsc::Receiver<PendingAudit>,
    events: mpsc::Receiver<OutputMessage>,
    wallet: JoinHandle<()>,
}
impl Fixture {
    async fn start(id: &str, shares: bool) -> Self {
        Self::start_with_capture(id, shares, None).await
    }
    async fn start_with_capture(
        id: &str,
        shares: bool,
        capture: Option<Arc<crate::evidence::CaptureStore>>,
    ) -> Self {
        let dir = tempfile::Builder::new()
            .prefix("native-backpressure-")
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir_in("/tmp/opencode")
            .unwrap();
        let socket = dir.path().join("native-audit.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        let meta = std::fs::metadata(&socket).unwrap();
        let (tx, requests) = mpsc::channel(4);
        let wallet = tokio::spawn(async move {
            let mut tasks = tokio::task::JoinSet::new();
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let tx = tx.clone();
                tasks.spawn(async move {
                    let Ok(len) = stream.read_u32_le().await else {
                        return;
                    };
                    assert!(len as usize <= wire::MAX_REQUEST_BYTES);
                    let mut bytes = vec![0; len as usize];
                    if stream.read_exact(&mut bytes).await.is_err() {
                        return;
                    }
                    let request = wire::decode_request(&bytes).unwrap();
                    let (release, wait) = oneshot::channel();
                    if tx
                        .send(PendingAudit {
                            request: request.clone(),
                            release,
                        })
                        .await
                        .is_err()
                    {
                        return;
                    }
                    if wait.await.is_err() {
                        return;
                    }
                    let reply = wire::encode_response(&approved(&request)).unwrap();
                    if stream.write_u32_le(reply.len() as u32).await.is_ok() {
                        let _ = stream.write_all(&reply).await;
                    }
                });
            }
        });
        let stat = std::fs::read_to_string("/proc/self/stat").unwrap();
        let ticks = stat
            .rsplit_once(')')
            .unwrap()
            .1
            .split_whitespace()
            .nth(19)
            .unwrap()
            .parse()
            .unwrap();
        let mut config = ConnectionConfig {
            id: id.into(),
            endpoint: "127.0.0.1:34254".into(),
            identity: spc_crypto::address::encode_sp_address(
                spc_crypto::Network::Regtest,
                &key(7).public_key(),
                &key(10).public_key(),
            )
            .unwrap()
            .into(),
            authority_public_key: key(9).x_only_public_key().0.to_string(),
            min_extranonce_size: 4,
            reconnect: ReconnectConfig {
                enabled: false,
                ..ReconnectConfig::default()
            },
            workers: vec![WorkerConfig {
                id: format!("{id}-worker").into(),
                budget: HashBudget {
                    hashes_per_second: Some(40),
                    total_attempts: Some(1000),
                },
                ntime_policy: NtimePolicy::Hold,
                max_ntime_roll_seconds: 0,
                batch_size: 1,
            }],
            source_audit: SourceAuditConfig::Required {
                run_id: "test-run".into(),
                miner_id: "test-miner".into(),
                wallet_socket: socket,
                wallet_instance: [5; 16],
                wallet_process: ProcessIdentity {
                    pid: std::process::id(),
                    uid: meta.uid(),
                    start_time_ticks: ticks,
                    boot_id: std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
                        .unwrap()
                        .trim()
                        .into(),
                    executable_sha256: [1; 32],
                    run_token_sha256: [2; 32],
                    config_sha256: [3; 32],
                },
                wallet_socket_device: meta.dev(),
                wallet_socket_inode: meta.ino(),
                peer_generation: 1,
                connection_config_digest: [0; 32],
                collector_pubkey: key(8).x_only_public_key().0.to_string(),
                pool_pubkey: key(9).x_only_public_key().0.to_string(),
            },
        };
        let digest =
            wire::connection_config_digest(&serde_json::to_value(&config).unwrap()).unwrap();
        if let SourceAuditConfig::Required {
            connection_config_digest,
            ..
        } = &mut config.source_audit
        {
            *connection_config_digest = digest;
        }
        config.validate().unwrap();
        let (event_tx, events) = mpsc::channel(256);
        let sink = EventSink::new(event_tx, "test-miner".into(), Instant::now());
        let (found, mut found_rx) = mpsc::channel(16);
        let worker = WorkerHandle::start(
            1,
            0,
            1,
            config.workers[0].clone(),
            config.id.clone(),
            found,
            sink.clone(),
        );
        let snapshot = Arc::new(Mutex::new(ConnectionSnapshot::new(
            &config.id,
            &config.endpoint,
            &config.identity,
        )));
        let (inbound, mut frame_rx) = mpsc::channel(16);
        let (shutdown, mut stopping) = watch::channel(false);
        let (transport, peer, probe, pending_write) = bounded_writer();
        let (cfg, workers, state, ev) = (
            config.clone(),
            vec![worker.clone()],
            snapshot.clone(),
            sink.clone(),
        );
        let join = tokio::spawn(async move {
            let _ = run_protocol(
                &cfg,
                1,
                &workers,
                &state,
                &ev,
                &mut stopping,
                &mut found_rx,
                "127.0.0.1:34254".parse().unwrap(),
                &mut frame_rx,
                transport,
                capture,
            )
            .await;
        });
        let handle = ConnectionHandle {
            config,
            workers: vec![worker.clone()],
            snapshot: snapshot.clone(),
            shutdown,
            join: Some(join),
            events: sink,
        };
        let mut f = Self {
            _dir: dir,
            handle: Some(handle),
            worker,
            snapshot,
            inbound,
            peer,
            probe,
            pending_write,
            requests,
            events,
            wallet,
        };
        assert_eq!(read_kind(&mut f.peer).await, MESSAGE_TYPE_SETUP_CONNECTION);
        f.inbound
            .send(Ok(frame(
                SetupConnectionSuccess {
                    used_version: 2,
                    flags: 0,
                },
                MESSAGE_TYPE_SETUP_CONNECTION_SUCCESS,
                false,
            )))
            .await
            .unwrap();
        assert_eq!(
            read_kind(&mut f.peer).await,
            MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL
        );
        let mut target = [0; 32];
        target[0] = 1;
        if shares {
            target.fill(255);
        }
        f.inbound
            .send(Ok(frame(
                OpenExtendedMiningChannelSuccess {
                    request_id: 1,
                    channel_id: 41,
                    target: U256::from(&target),
                    extranonce_size: 4,
                    extranonce_prefix: B032::new(&[1; 4]).unwrap(),
                    group_channel_id: 9,
                },
                MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL_SUCCESS,
                false,
            )))
            .await
            .unwrap();
        f.job(1, true).await;
        f.inbound
            .send(Ok(frame(
                SetNewPrevHash {
                    channel_id: 41,
                    job_id: 1,
                    prev_hash: U256::from(&[2; 32]),
                    min_ntime: 100,
                    nbits: 0x207fffff,
                },
                MESSAGE_TYPE_MINING_SET_NEW_PREV_HASH,
                true,
            )))
            .await
            .unwrap();
        f
    }
    async fn job(&self, id: u32, future: bool) {
        let tx = bitcoin::Transaction {
            version: bitcoin::transaction::Version::ONE,
            lock_time: bitcoin::absolute::LockTime::ZERO,
            input: vec![bitcoin::TxIn {
                previous_output: bitcoin::OutPoint::null(),
                script_sig: bitcoin::script::Builder::new()
                    .push_int(1)
                    .push_slice([0; 8])
                    .into_script(),
                sequence: bitcoin::Sequence::MAX,
                witness: bitcoin::Witness::new(),
            }],
            output: vec![bitcoin::TxOut {
                value: bitcoin::Amount::from_sat(1),
                script_pubkey: bitcoin::ScriptBuf::new(),
            }],
        };
        let raw = bitcoin::consensus::serialize(&tx);
        self.inbound
            .send(Ok(frame(
                NewExtendedMiningJob {
                    channel_id: 41,
                    job_id: id,
                    min_ntime: Sv2Option::new(if future { None } else { Some(100) }),
                    version: 1,
                    version_rolling_allowed: false,
                    merkle_path: Seq0255::new(Vec::<U256>::new()).unwrap(),
                    coinbase_tx_prefix: B064K::try_from(&raw[..44]).unwrap(),
                    coinbase_tx_suffix: B064K::try_from(&raw[52..]).unwrap(),
                },
                MESSAGE_TYPE_NEW_EXTENDED_MINING_JOB,
                true,
            )))
            .await
            .unwrap();
    }
    async fn request(&mut self) -> PendingAudit {
        timeout(Duration::from_secs(2), self.requests.recv())
            .await
            .unwrap()
            .unwrap()
    }
    fn attempts(&self) -> u64 {
        self.worker.snapshot().attempted.parse().unwrap()
    }
    async fn hash(&self) {
        timeout(Duration::from_secs(2), async {
            while self.attempts() == 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }
    async fn activate(&mut self) -> Arc<crate::native_gate::AuditPermit> {
        self.request().await.release.send(()).unwrap();
        self.hash().await;
        self.worker.audit_permit().unwrap()
    }
    async fn stop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.shutdown().await;
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(mut handle) = self.handle.take() {
            handle.request_stop();
            if let Some(join) = handle.join.take() {
                join.abort();
            }
        }
        self.wallet.abort();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn blocked_partial_submission_does_not_delay_candidate_or_target_revocation() {
    for target in [false, true] {
        let mut f = Fixture::start("candidate", true).await;
        let permit = f.activate().await;
        pending_write(&mut f.pending_write, MESSAGE_TYPE_SUBMIT_SHARES_EXTENDED).await;
        if target {
            f.inbound
                .send(Ok(frame(
                    SetTarget {
                        channel_id: 41,
                        maximum_target: U256::from(&[0x7f; 32]),
                    },
                    MESSAGE_TYPE_SET_TARGET,
                    true,
                )))
                .await
                .unwrap();
        } else {
            f.job(2, false).await;
        }
        timeout(Duration::from_millis(250), async {
            while permit.valid() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(permit
            .with_valid(|| panic!("revoked hash entered"))
            .is_none());
        assert!(
            !f.probe.dropped.load(Ordering::SeqCst),
            "writer remains genuinely blocked during revocation proof"
        );
        tokio::time::sleep(Duration::from_millis(30)).await;
        let stopped = f.attempts();
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert_eq!(stopped, f.attempts());
        assert!(f.worker.snapshot().active_job_id.is_none());
        if target {
            f.job(2, false).await;
        }
        let old = f.request().await;
        f.job(3, false).await;
        let next = f.request().await;
        assert!(next.request.candidate_epoch > old.request.candidate_epoch);
        let _ = old.release.send(());
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(stopped, f.attempts());
        assert!(f.snapshot.lock().unwrap().active_job.is_none());
        f.handle.as_ref().unwrap().request_stop();
        let _ = next.release.send(());
        f.stop().await;
        assert!(f.probe.dropped.load(Ordering::SeqCst));
        assert!(!f.probe.closed.load(Ordering::SeqCst));
        assert!(!f
            .probe
            .calls
            .lock()
            .unwrap()
            .contains(&MESSAGE_TYPE_CLOSE_CHANNEL));
        let mut partial = Vec::new();
        f.peer.read_to_end(&mut partial).await.unwrap();
        assert_eq!(
            partial.len(),
            1,
            "abandoned partial frame must never be resumed"
        );
        assert_eq!(stopped, f.attempts());
        while let Ok(event) = f.events.try_recv() {
            if let OutputMessage::Event(event) = event {
                if let MinerEvent::SourceAuditCommitted {
                    candidate_epoch, ..
                } = event.event
                {
                    assert!(
                        candidate_epoch < old.request.candidate_epoch,
                        "late approval activated"
                    );
                }
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_all_fences_later_workers_before_first_blocked_close_finishes() {
    let mut first = Fixture::start("a-first", false).await;
    let mut later = Fixture::start("z-later", false).await;
    let first_permit = first.activate().await;
    let later_permit = later.activate().await;
    let handles = BTreeMap::from([
        ("a-first".into(), first.handle.take().unwrap()),
        ("z-later".into(), later.handle.take().unwrap()),
    ]);
    let stopping = tokio::spawn(shutdown_connections(handles));
    pending_write(&mut first.pending_write, MESSAGE_TYPE_CLOSE_CHANNEL).await;
    assert!(!stopping.is_finished(), "first close must still be blocked");
    assert!(!first_permit.valid() && !later_permit.valid());
    assert!(!first.probe.dropped.load(Ordering::SeqCst));
    assert!(later.worker.snapshot().active_job_id.is_none());
    tokio::time::sleep(Duration::from_millis(30)).await;
    let counts = (first.attempts(), later.attempts());
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_eq!(counts, (first.attempts(), later.attempts()));
    timeout(Duration::from_secs(2), stopping)
        .await
        .unwrap()
        .unwrap();
    assert!(
        first.probe.dropped.load(Ordering::SeqCst) && later.probe.dropped.load(Ordering::SeqCst)
    );
    assert!(!first.probe.closed.load(Ordering::SeqCst));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn outbound_queue_is_bounded_and_drop_discards_partial_transport() {
    let (transport, mut peer, probe, mut pending) = bounded_writer();
    let writer = outbound::Writer::new(transport);
    let send = || {
        writer.send(
            SubmitSharesExtended {
                channel_id: 41,
                sequence_number: 1,
                job_id: 1,
                nonce: 0,
                ntime: 100,
                version: 1,
                extranonce: B032::new(&[0; 4]).unwrap(),
            },
            MESSAGE_TYPE_SUBMIT_SHARES_EXTENDED,
            true,
        )
    };
    send().unwrap();
    pending_write(&mut pending, MESSAGE_TYPE_SUBMIT_SHARES_EXTENDED).await;
    for _ in 0..outbound::CAPACITY {
        send().unwrap();
    }
    assert_eq!(outbound::queued(&writer), outbound::CAPACITY);
    assert!(send().is_err());
    drop(writer);
    timeout(Duration::from_secs(1), async {
        while !probe.dropped.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let mut partial = Vec::new();
    peer.read_to_end(&mut partial).await.unwrap();
    assert_eq!(partial.len(), 1);
    assert_eq!(probe.completed.load(Ordering::SeqCst), 0);
    assert_eq!(probe.calls.lock().unwrap().len(), 1);
}

fn capture_fixture(records: u64) -> (tempfile::TempDir, Arc<crate::evidence::CaptureStore>) {
    let dir = tempfile::Builder::new()
        .prefix("native-evidence-test-")
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir_in("/tmp/opencode")
        .unwrap();
    let store = crate::evidence::CaptureStore::test_store(dir.path(), records);
    (dir, store)
}
fn fixture_keys() -> wire::VerificationKeys {
    wire::VerificationKeys {
        wallet: key(7).x_only_public_key().0,
        collector: key(8).x_only_public_key().0,
        pool: key(9).x_only_public_key().0,
    }
}
async fn retained_event(
    f: &mut Fixture,
) -> (
    spc_scan::native_evidence::DurableHandle,
    crate::report::EventEnvelope,
) {
    timeout(Duration::from_secs(2), async {
        loop {
            if let OutputMessage::Event(event) =
                f.events.recv().await.expect("native event stream closed")
            {
                if let MinerEvent::SourceAuditCommitted {
                    durable_evidence, ..
                } = &event.event
                {
                    return (
                        durable_evidence.clone().expect("required durable handle"),
                        *event,
                    );
                }
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn durable_public_bytes_exist_before_activation_and_exact_replay_is_immutable() {
    use spc_scan::native_evidence as evidence;
    let (_dir, store) = capture_fixture(16);
    let delay = crate::evidence::TestDelay::new();
    let _release = delay.guard();
    store.delay_next(delay.clone());
    let mut f = Fixture::start_with_capture("retained", false, Some(store.clone())).await;
    let pending = f.request().await;
    let request_bytes = wire::encode_request(&pending.request).unwrap();
    let response_bytes = wire::encode_response(&approved(&pending.request)).unwrap();
    pending.release.send(()).unwrap();
    timeout(Duration::from_secs(2), delay.entered.notified())
        .await
        .unwrap();
    assert_eq!(f.attempts(), 0);
    assert!(f.snapshot.lock().unwrap().active_job.is_none());
    let before_capture = serde_json::json!({"worker":f.worker.snapshot(),"connection":f.snapshot.lock().unwrap().clone()});
    while let Ok(OutputMessage::Event(event)) = f.events.try_recv() {
        assert!(!matches!(
            event.event,
            MinerEvent::SourceAuditCommitted { .. }
                | MinerEvent::JobActivated { .. }
                | MinerEvent::AuditFirstHash { .. }
        ));
    }
    delay.release();
    let (handle, committed) = retained_event(&mut f).await;
    let info = store.info();
    let path = info.directory.join(&handle.record_relative_path);
    let bytes = std::fs::read(&path).unwrap();
    let manifest = std::fs::read(info.directory.join(&handle.manifest_relative_path)).unwrap();
    let (record, request, _) =
        evidence::decode_record(&bytes, &manifest, &handle, &fixture_keys()).unwrap();
    assert_eq!(record.request_json_hex, hex::encode(&request_bytes));
    assert_eq!(record.response_json_hex, hex::encode(&response_bytes));
    assert_eq!(request.nonce, pending.request.nonce);
    assert_eq!(record.activation, "not_asserted");
    assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
    f.hash().await;
    let mut after_commit = Vec::new();
    timeout(Duration::from_secs(2), async {
        loop {
            if let OutputMessage::Event(event) = f.events.recv().await.expect("first-hash event") {
                let first_hash = matches!(event.event, MinerEvent::AuditFirstHash { .. });
                after_commit.push(event);
                if first_hash {
                    break;
                }
            }
        }
    })
    .await
    .unwrap();
    let activated = after_commit
        .iter()
        .find(|e| matches!(e.event, MinerEvent::JobActivated { .. }))
        .unwrap();
    let hashed = after_commit
        .iter()
        .find(|e| matches!(e.event, MinerEvent::AuditFirstHash { .. }))
        .unwrap();
    assert!(committed.event_id < activated.event_id && activated.event_id < hashed.event_id);
    if let Ok(output) = std::env::var("SPC_NATIVE_EVIDENCE_TEST_OUTPUT") {
        use std::io::Write;
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        let output = std::path::PathBuf::from(output);
        assert!(
            output.starts_with("/tmp/opencode")
                && output
                    .components()
                    .all(|c| !matches!(c, std::path::Component::ParentDir))
        );
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&output)
            .unwrap();
        let observed = serde_json::json!({
            "scope":"actual retention writer/protocol/hash-worker component with signed simulated wallet and fixture startup metadata; not controller/full-history acceptance",
            "before_capture":before_capture,"committed":committed,"after_commit":after_commit,
            "request_digest":handle.request_digest,"record_sha256":handle.record_sha256,
            "wallet_pubkey":fixture_keys().wallet.to_string(),"collector_pubkey":fixture_keys().collector.to_string(),"pool_pubkey":fixture_keys().pool.to_string()
        });
        for (name, data) in [
            ("record.json", bytes.clone()),
            ("manifest.json", manifest.clone()),
            ("handle.json", serde_json::to_vec(&handle).unwrap()),
            ("locator.json", serde_json::to_vec(&info).unwrap()),
            (
                "observation.json",
                serde_json::to_vec_pretty(&observed).unwrap(),
            ),
        ] {
            let mut file = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(output.join(name))
                .unwrap();
            file.write_all(&data).unwrap();
            file.sync_all().unwrap();
        }
        std::fs::File::open(output).unwrap().sync_all().unwrap();
    }
    let repeated = store
        .retain(
            request_bytes.clone(),
            response_bytes.clone(),
            fixture_keys(),
            record.verified_at_unix_ms,
            Instant::now() + Duration::from_secs(1),
            Arc::new(AtomicBool::new(true)),
        )
        .await
        .unwrap();
    assert_eq!(handle, repeated);
    assert_eq!(bytes, std::fs::read(&path).unwrap());
    let mut different_wire_bytes = response_bytes.clone();
    different_wire_bytes.push(b' ');
    assert!(store
        .retain(
            request_bytes.clone(),
            different_wire_bytes,
            fixture_keys(),
            record.verified_at_unix_ms,
            Instant::now() + Duration::from_secs(1),
            Arc::new(AtomicBool::new(true))
        )
        .await
        .is_err());
    let mut changed = bytes.clone();
    changed[0] = b'[';
    assert!(evidence::decode_record(&changed, &manifest, &handle, &fixture_keys()).is_err());
    let mut foreign = handle.clone();
    foreign.request_digest = "00".repeat(32);
    assert!(evidence::decode_record(&bytes, &manifest, &foreign, &fixture_keys()).is_err());
    f.stop().await;
    // A missing record is not silently reconstructed from remembered success.
    std::fs::remove_file(&path).unwrap();
    assert!(store
        .retain(
            request_bytes,
            response_bytes,
            fixture_keys(),
            record.verified_at_unix_ms,
            Instant::now() + Duration::from_secs(1),
            Arc::new(AtomicBool::new(true))
        )
        .await
        .is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retention_budget_and_permission_failure_leave_candidate_unknown() {
    for failure_mode in 0..3 {
        let permissions = failure_mode == 1;
        let (_dir, store) = capture_fixture(if failure_mode == 2 { 16 } else { 1 });
        let mut f =
            Fixture::start_with_capture("failed-retention", false, Some(store.clone())).await;
        if permissions {
            let info = store.info();
            std::fs::set_permissions(
                info.directory.join(info.manifest_relative_path),
                std::fs::Permissions::from_mode(0o644),
            )
            .unwrap();
        } else {
            f.activate().await;
            if failure_mode == 2 {
                store.exhaust_bytes_for_test();
            }
            f.job(2, false).await;
        }
        let pending = f.request().await;
        pending.release.send(()).unwrap();
        timeout(Duration::from_secs(2), async {
            while f.snapshot.lock().unwrap().source_audit_state.as_deref() != Some("unknown") {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert!(f.snapshot.lock().unwrap().active_job.is_none());
        tokio::time::sleep(Duration::from_millis(30)).await;
        let attempts = f.attempts();
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert_eq!(attempts, f.attempts());
        if permissions {
            assert_eq!(attempts, 0);
        }
        f.stop().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn blocked_retention_is_cancelled_on_supersession_and_stop_without_publication() {
    let (_dir, store) = capture_fixture(16);
    let delay = crate::evidence::TestDelay::new();
    let _release = delay.guard();
    store.delay_next(delay.clone());
    let mut f = Fixture::start_with_capture("cancel-retention", false, Some(store.clone())).await;
    let old = f.request().await;
    old.release.send(()).unwrap();
    timeout(Duration::from_secs(2), delay.entered.notified())
        .await
        .unwrap();
    f.job(2, false).await;
    let new = f.request().await;
    assert!(new.request.candidate_epoch > old.request.candidate_epoch);
    new.release.send(()).unwrap();
    // New request can reach the wallet while old storage is genuinely blocked.
    f.handle.as_ref().unwrap().request_stop();
    assert_eq!(f.attempts(), 0);
    delay.release();
    f.stop().await;
    assert_eq!(f.attempts(), 0);
    while let Ok(OutputMessage::Event(event)) = f.events.try_recv() {
        assert!(!matches!(
            event.event,
            MinerEvent::SourceAuditCommitted { .. }
                | MinerEvent::JobActivated { .. }
                | MinerEvent::AuditFirstHash { .. }
        ));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expired_blocking_retention_returns_no_durable_authority() {
    let (_dir, store) = capture_fixture(16);
    let delay = crate::evidence::TestDelay::new();
    let _release = delay.guard();
    store.delay_next(delay.clone());
    let mut f = Fixture::start("retention-timeout", false).await;
    let request = f.request().await;
    let req = wire::encode_request(&request.request).unwrap();
    let response = wire::encode_response(&approved(&request.request)).unwrap();
    let deadline = Instant::now() + Duration::from_millis(100);
    let captured = store.clone();
    let task = tokio::spawn(async move {
        captured
            .retain(
                req,
                response,
                fixture_keys(),
                crate::native_gate::now_ms().unwrap(),
                deadline,
                Arc::new(AtomicBool::new(true)),
            )
            .await
    });
    timeout(Duration::from_secs(2), delay.entered.notified())
        .await
        .unwrap();
    tokio::time::sleep_until((deadline + Duration::from_millis(10)).into()).await;
    delay.release();
    assert!(task.await.unwrap().is_err());
    assert_eq!(f.attempts(), 0);
    f.stop().await;
}
