//! Execution/admission scaffolding only. Probe bytes below are NOT a native
//! activation frame, collector proof, signature, or funded acceptance result.
use super::*;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};
use tokio::io::AsyncReadExt;
use tokio::sync::{mpsc, Notify};

struct Lease {
    uid: u32,
    valid: AtomicBool,
}
impl EndpointLease for Lease {
    fn owner_uid(&self) -> u32 {
        self.uid
    }
    fn validate(&self) -> anyhow::Result<()> {
        ensure!(
            self.valid.load(Ordering::SeqCst),
            "scripted replaced endpoint"
        );
        Ok(())
    }
}
struct Authorizer {
    expected_pid: u32,
    current: AtomicBool,
    calls: AtomicUsize,
    delay: Duration,
}
impl PeerAuthorizer for Authorizer {
    type Scope = ();
    fn authorize<'a>(
        &'a self,
        _: &'a mut UnixStream,
        peer: KernelPeer,
        _: Instant,
    ) -> AuditFuture<'a, ()> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(self.delay).await;
            ensure!(
                peer.pid.get() == self.expected_pid && self.current.load(Ordering::SeqCst),
                "scripted foreign/unbound process scope"
            );
            Ok(())
        })
    }
    fn revalidate<'a>(&'a self, peer: KernelPeer, _: &'a ()) -> AuditFuture<'a, ()> {
        Box::pin(async move {
            ensure!(
                peer.pid.get() == self.expected_pid && self.current.load(Ordering::SeqCst),
                "scripted stale run/config/start-token scope"
            );
            Ok(())
        })
    }
}
struct Handler {
    entered: mpsc::UnboundedSender<()>,
    release: Arc<Notify>,
    prepare_calls: AtomicUsize,
    finish_calls: AtomicUsize,
    deny: bool,
    response: Vec<u8>,
}
impl ProtocolHandler<()> for Handler {
    fn validate_release<'a>(&'a self, _: &'a (), _: &'a [u8]) -> AuditFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
    type Pending = ();
    fn prepare<'a>(&'a self, _: &'a mut UnixStream, _: &'a (), _: Instant) -> AuditFuture<'a, ()> {
        Box::pin(async move {
            self.prepare_calls.fetch_add(1, Ordering::SeqCst);
            let _ = self.entered.send(());
            ensure!(!self.deny, "scripted missing history, no result");
            self.release.notified().await;
            Ok(())
        })
    }
    fn finish<'a>(&'a self, _: (), _: &'a (), _: Instant) -> AuditFuture<'a, Vec<u8>> {
        Box::pin(async move {
            self.finish_calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.response.clone())
        })
    }
}
fn setup() -> (
    tempfile::TempDir,
    StdUnixListener,
    Arc<Lease>,
    Arc<Authorizer>,
) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let listener = StdUnixListener::bind(dir.path().join("probe.sock")).unwrap();
    std::fs::set_permissions(
        dir.path().join("probe.sock"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let lease = Arc::new(Lease {
        uid: std::fs::metadata("/proc/self").unwrap().uid(),
        valid: AtomicBool::new(true),
    });
    let authorizer = Arc::new(Authorizer {
        expected_pid: std::process::id(),
        current: AtomicBool::new(true),
        calls: AtomicUsize::new(0),
        delay: Duration::ZERO,
    });
    (dir, listener, lease, authorizer)
}
fn limits() -> Limits {
    Limits {
        concurrent_sessions: 2,
        total_deadline: Duration::from_secs(1),
        max_encoded_response_bytes: 32,
    }
}
fn handler(deny: bool) -> (Arc<Handler>, mpsc::UnboundedReceiver<()>) {
    let (entered, rx) = mpsc::unbounded_channel();
    (
        Arc::new(Handler {
            entered,
            release: Arc::new(Notify::new()),
            prepare_calls: AtomicUsize::new(0),
            finish_calls: AtomicUsize::new(0),
            deny,
            response: b"transport-probe-only".to_vec(),
        }),
        rx,
    )
}
async fn ended(mut stream: UnixStream) -> Vec<u8> {
    let mut bytes = vec![];
    tokio::time::timeout(Duration::from_secs(3), stream.read_to_end(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    bytes
}
async fn entered(rx: &mut mpsc::UnboundedReceiver<()>) {
    tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn native_scaffold_foreign_uid_or_pid_is_rejected_before_handler() {
    for wrong_uid in [true, false] {
        let (dir, listener, mut lease, mut authorizer) = setup();
        if wrong_uid {
            let uid = lease.uid.wrapping_add(1);
            Arc::get_mut(&mut lease).unwrap().uid = uid;
        } else {
            Arc::get_mut(&mut authorizer).unwrap().expected_pid = u32::MAX;
        }
        let (handler, _) = handler(false);
        let service = Service::start(listener, lease, limits(), authorizer, handler.clone())
            .await
            .unwrap();
        assert!(ended(
            UnixStream::connect(dir.path().join("probe.sock"))
                .await
                .unwrap()
        )
        .await
        .is_empty());
        assert_eq!(handler.prepare_calls.load(Ordering::SeqCst), 0);
        service.stop().await.unwrap();
    }
}

#[tokio::test]
async fn native_scaffold_unknown_history_and_replaced_scope_never_release_bytes() {
    for deny in [true, false] {
        let (dir, listener, lease, authorizer) = setup();
        let (handler, mut rx) = handler(deny);
        let service = Service::start(
            listener,
            lease,
            limits(),
            authorizer.clone(),
            handler.clone(),
        )
        .await
        .unwrap();
        let client = UnixStream::connect(dir.path().join("probe.sock"))
            .await
            .unwrap();
        entered(&mut rx).await;
        if !deny {
            authorizer.current.store(false, Ordering::SeqCst);
            handler.release.notify_one();
        }
        assert!(ended(client).await.is_empty());
        assert_eq!(handler.finish_calls.load(Ordering::SeqCst), 0);
        service.stop().await.unwrap();
    }
}

#[tokio::test]
async fn native_scaffold_bounded_workers_close_overload_and_shutdown_cancels_waiters() {
    let (dir, listener, lease, authorizer) = setup();
    let (handler, mut rx) = handler(false);
    let service = Service::start(listener, lease, limits(), authorizer, handler.clone())
        .await
        .unwrap();
    let first = UnixStream::connect(dir.path().join("probe.sock"))
        .await
        .unwrap();
    entered(&mut rx).await;
    let second = UnixStream::connect(dir.path().join("probe.sock"))
        .await
        .unwrap();
    entered(&mut rx).await;
    let excess = UnixStream::connect(dir.path().join("probe.sock"))
        .await
        .unwrap();
    assert!(ended(excess).await.is_empty());
    assert_eq!(handler.prepare_calls.load(Ordering::SeqCst), 2);
    service.stop().await.unwrap();
    assert!(ended(first).await.is_empty());
    assert!(ended(second).await.is_empty());
    assert_eq!(handler.finish_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn native_scaffold_admission_is_inside_total_deadline() {
    let (dir, listener, lease, mut authorizer) = setup();
    Arc::get_mut(&mut authorizer).unwrap().delay = Duration::from_millis(200);
    let (handler, _) = handler(false);
    let mut bound = limits();
    bound.total_deadline = Duration::from_millis(30);
    let service = Service::start(listener, lease, bound, authorizer, handler.clone())
        .await
        .unwrap();
    assert!(ended(
        UnixStream::connect(dir.path().join("probe.sock"))
            .await
            .unwrap()
    )
    .await
    .is_empty());
    assert_eq!(handler.prepare_calls.load(Ordering::SeqCst), 0);
    service.stop().await.unwrap();
}

#[tokio::test]
async fn native_scaffold_endpoint_revocation_and_response_bounds_are_enforced() {
    for revoke in [true, false] {
        let (dir, listener, lease, authorizer) = setup();
        let (mut handler, mut rx) = handler(false);
        if !revoke {
            Arc::get_mut(&mut handler).unwrap().response = vec![0; 33];
        }
        let service = Service::start(
            listener,
            lease.clone(),
            limits(),
            authorizer,
            handler.clone(),
        )
        .await
        .unwrap();
        let client = UnixStream::connect(dir.path().join("probe.sock"))
            .await
            .unwrap();
        entered(&mut rx).await;
        if revoke {
            lease.valid.store(false, Ordering::SeqCst);
        }
        handler.release.notify_one();
        assert!(ended(client).await.is_empty());
        service.stop().await.unwrap();
    }
}

struct BlockingHandler {
    entered: mpsc::UnboundedSender<()>,
    gate: Arc<(Mutex<bool>, Condvar)>,
    blocked: AtomicBool,
}
impl ProtocolHandler<()> for BlockingHandler {
    fn validate_release<'a>(&'a self, _: &'a (), _: &'a [u8]) -> AuditFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
    type Pending = ();
    fn prepare<'a>(&'a self, _: &'a mut UnixStream, _: &'a (), _: Instant) -> AuditFuture<'a, ()> {
        Box::pin(async move {
            self.blocked.store(true, Ordering::SeqCst);
            let _ = self.entered.send(());
            let (mutex, cond) = &*self.gate;
            let (released, _) = cond
                .wait_timeout_while(mutex.lock().unwrap(), Duration::from_secs(2), |released| {
                    !*released
                })
                .unwrap();
            self.blocked.store(false, Ordering::SeqCst);
            ensure!(*released, "scripted verifier release timeout");
            Ok(())
        })
    }
    fn finish<'a>(&'a self, _: (), _: &'a (), _: Instant) -> AuditFuture<'a, Vec<u8>> {
        Box::pin(async { anyhow::bail!("no native proof supplied by blocking transport fixture") })
    }
}

#[tokio::test(flavor = "current_thread")]
async fn native_scaffold_blocked_audit_does_not_hold_command_queue_or_wallet_database() {
    use crate::wallet::maintenance::{submit_command, MaintenanceAction, WalletCommand};
    use crate::wallet::{StoreIdentity, WalletStore};
    let (dir, listener, lease, authorizer) = setup();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let handler = Arc::new(BlockingHandler {
        entered: tx,
        gate: gate.clone(),
        blocked: AtomicBool::new(false),
    });
    let service = Service::start(listener, lease, limits(), authorizer, handler.clone())
        .await
        .unwrap();
    let client = UnixStream::connect(dir.path().join("probe.sock"))
        .await
        .unwrap();
    entered(&mut rx).await;
    let wallet = tempfile::tempdir().unwrap();
    let store = WalletStore::open(
        wallet.path(),
        &StoreIdentity {
            wallet_fingerprint: bitcoin::bip32::Fingerprint::from([1; 4]),
            sp_id: [2; 32],
            network: "regtest".into(),
        },
    )
    .unwrap();
    let command = WalletCommand {
        operation_id: hex::encode([3; 32]),
        vtxo_ids: vec![hex::encode([4; 36])],
        action: MaintenanceAction::Refresh,
    };
    assert!(handler.blocked.load(Ordering::SeqCst));
    submit_command(wallet.path(), &command).unwrap();
    store.reserve_maintenance(&command, Some(b"[]")).unwrap();
    assert!(wallet
        .path()
        .join("commands")
        .join(format!("{}.json", command.operation_id))
        .is_file());
    assert!(
        handler.blocked.load(Ordering::SeqCst),
        "queue/DB progressed before audit release"
    );
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
    assert!(ended(client).await.is_empty());
    service.stop().await.unwrap();
}

#[test]
fn native_scaffold_transport_config_rejects_unbounded_resources() {
    for bad in [
        Limits {
            concurrent_sessions: 0,
            ..limits()
        },
        Limits {
            concurrent_sessions: 5,
            ..limits()
        },
        Limits {
            total_deadline: Duration::from_secs(11),
            ..limits()
        },
        Limits {
            max_encoded_response_bytes: 0,
            ..limits()
        },
    ] {
        assert!(bad.validate().is_err());
    }
    limits().validate().unwrap();
}
