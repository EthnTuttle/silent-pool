//! AP-3a wallet registration integration tests (plan 4.1.1 client
//! semantics, AP-3 acceptance).

#![cfg(feature = "ark")]

mod wallet_common;

use std::net::SocketAddr;

use bitcoin::hashes::{sha256, Hash as _};
use spc_agent::wallet::registration::RegistrationOutcome;
use spc_agent::wallet::WalletRole;
use spc_crypto::Network;
use wallet_common::{provision_files, test_settings, MockTransport};

/// A deterministic non-fixture master seed (fresh scenario runs may
/// generate new masters; they are never used as recovery proof).
fn scenario_master(tag: u8) -> [u8; 64] {
    let mut seed = [0u8; 64];
    seed[..32].copy_from_slice(
        &sha256::Hash::hash(format!("spc-ap3a-scenario-{tag}-left").as_bytes()).to_byte_array(),
    );
    seed[32..].copy_from_slice(
        &sha256::Hash::hash(format!("spc-ap3a-scenario-{tag}-right").as_bytes()).to_byte_array(),
    );
    seed
}

/// Start a role from a scenario seed, keeping the key-file dir alive
/// for the caller (the returned `TempDir` must stay in scope).
async fn start_role_async(tag: u8, datadir: &std::path::Path) -> (WalletRole, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let (sources, _sp_address) =
        provision_files(dir.path(), &scenario_master(tag), Network::Regtest);
    let settings = test_settings(&sources, datadir, SocketAddr::from(([127, 0, 0, 1], 0)));
    let role = WalletRole::start(settings).await.unwrap();
    (role, dir)
}

/// The role starts, loads state, and stays explicitly
/// `awaiting_registration` without a transport (AP-2 owns the 0x5350
/// codec). It never pretends to be connected, and it never mines.
#[tokio::test]
async fn role_stays_awaiting_registration_without_a_transport() {
    let dir = tempfile::tempdir().unwrap();
    let (role, _keys_dir) = start_role_async(1, &dir.path().join("datadir")).await;
    assert!(!role
        .store()
        .is_registered(&role.registration_digest())
        .unwrap());
    let status = std::sync::Arc::clone(role.status());
    assert_eq!(
        status.state(),
        spc_agent::wallet::WalletState::AwaitingRegistration
    );

    // Dormant run() parks until shutdown; the dormancy reason is only
    // honest once the loop has actually started, so run it on a task
    // and poll the snapshot until it reports the missing transport.
    let shutdown = role.shutdown_handle();
    let task = tokio::spawn(role.run(None));
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    let snapshot = loop {
        let snapshot = status.snapshot().unwrap();
        if snapshot.last_error.is_some() {
            break snapshot;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "dormant run never reported a last_error: {snapshot:?}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    };

    // The dormant status is honest about the missing transport: the
    // state is `dormant` (no transport wired), distinct from
    // `awaiting_registration` (transport wired, no ack yet).
    assert!(!snapshot.registered);
    assert_eq!(snapshot.state, "dormant");
    assert!(snapshot.spendable_sats.is_none());
    assert!(
        snapshot
            .last_error
            .as_deref()
            .unwrap_or("")
            .contains("AP-2"),
        "dormancy is explained, not hidden: {snapshot:?}"
    );

    // The parked loop stops cleanly on shutdown.
    shutdown.send(true).unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(status.state(), spc_agent::wallet::WalletState::Stopped);
}

/// The full client request-id policy through the role: transport
/// failure and retryable errors reuse the reserved id; `applied`
/// commits; a terminal mismatch unwedges with a fresh id; the counter
/// never wraps back.
#[tokio::test]
async fn registration_attempts_follow_the_request_id_policy() {
    let dir = tempfile::tempdir().unwrap();
    let (role, _keys_dir) = start_role_async(2, &dir.path().join("datadir")).await;

    // 1. Transport failure: id 1 reserved, kept pending.
    let mut transport = MockTransport::new(vec![Err("front unreachable".into())]);
    let a = role
        .with_registration(|r| r.attempt(&mut transport))
        .unwrap();
    assert_eq!((a.request_id, a.journaled), (1, false));
    assert_eq!(a.outcome, RegistrationOutcome::Unavailable);
    assert_eq!(
        role.store().pending_registration_request_id().unwrap(),
        Some(1)
    );

    // 2. Retryable server error: still the same attempt.
    let mut transport = MockTransport::new(vec![Ok(RegistrationOutcome::Retryable {
        error_code: "registration-commit-unknown".into(),
    })]);
    let a = role
        .with_registration(|r| r.attempt(&mut transport))
        .unwrap();
    assert_eq!(a.request_id, 1);
    assert!(!a.journaled);
    assert!(!role
        .store()
        .is_registered(&role.registration_digest())
        .unwrap());

    // 3. Applied: journaled, registered, status flips.
    let mut transport = MockTransport::new(vec![Ok(RegistrationOutcome::Applied)]);
    let a = role
        .with_registration(|r| r.attempt(&mut transport))
        .unwrap();
    assert_eq!(a.request_id, 1);
    assert!(a.journaled);
    assert!(role
        .store()
        .is_registered(&role.registration_digest())
        .unwrap());
    let snapshot = role.status().snapshot().unwrap();
    assert!(snapshot.registered);

    // 4. A new registration after a terminal outcome gets a fresh id
    //    (no self-wedge): mismatch first, then success on id 2.
    let mut transport = MockTransport::new(vec![
        Ok(RegistrationOutcome::Mismatch),
        Ok(RegistrationOutcome::Applied),
    ]);
    let a = role
        .with_registration(|r| r.attempt(&mut transport))
        .unwrap();
    assert_eq!(a.request_id, 2);
    assert!(a.journaled);
    let a = role
        .with_registration(|r| r.attempt(&mut transport))
        .unwrap();
    assert_eq!(a.request_id, 3, "incremented past the mismatched id");
    assert!(role
        .store()
        .is_registered(&role.registration_digest())
        .unwrap());
    assert_eq!(role.store().last_request_id().unwrap(), 3);
}

/// `superseded` means "applied historically, not live": the agent is
/// not registered and must re-register with a higher id.
#[tokio::test]
async fn superseded_is_not_a_live_registration() {
    let dir = tempfile::tempdir().unwrap();
    let (role, _keys_dir) = start_role_async(3, &dir.path().join("datadir")).await;
    let mut transport = MockTransport::new(vec![Ok(RegistrationOutcome::Superseded)]);
    let a = role
        .with_registration(|r| r.attempt(&mut transport))
        .unwrap();
    assert_eq!(a.request_id, 1);
    assert!(a.journaled);
    assert!(!role
        .store()
        .is_registered(&role.registration_digest())
        .unwrap());
    assert!(!role.status().snapshot().unwrap().registered);
}

/// The wallet role never co-runs with the mining-side roles (SPEC
/// 11.5 isolation, fail closed).
#[test]
fn wallet_role_never_coruns_with_mining_roles() {
    assert!(spc_agent::validate_role_selection(&["wallet"]).is_ok());
    assert!(spc_agent::validate_role_selection(&["translator"]).is_ok());
    assert!(spc_agent::validate_role_selection(&["jdc"]).is_ok());
    assert!(spc_agent::validate_role_selection(&["translator", "jdc"]).is_ok());
    for bad in [
        vec!["wallet", "translator"],
        vec!["translator", "wallet"],
        vec!["wallet", "jdc"],
        vec!["jdc", "wallet", "translator"],
    ] {
        let err = spc_agent::validate_role_selection(&bad).unwrap_err();
        assert!(
            err.contains("cannot co-run"),
            "combination {bad:?} must fail closed: {err}"
        );
    }
}

/// A role cannot open a datadir bound to a different identity: fail
/// closed, state untouched, nothing regenerated.
#[tokio::test]
async fn state_bound_to_a_different_identity_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    let datadir = dir.path().join("datadir");
    let (role, _keys_dir) = start_role_async(4, &datadir).await;
    drop(role);

    // A different seed against the same datadir: refused.
    let result = {
        let key_dir = tempfile::tempdir().unwrap();
        let (sources, _) = provision_files(key_dir.path(), &scenario_master(5), Network::Regtest);
        let settings = test_settings(&sources, &datadir, SocketAddr::from(([127, 0, 0, 1], 0)));
        WalletRole::start(settings).await
    };
    // `WalletRole` is not `Debug` (it holds key material); take the Err
    // side without asserting the Ok side.
    let err = match result {
        Err(e) => e,
        Ok(_) => panic!("mismatched identity must fail closed"),
    };
    assert!(
        format!("{err}").contains("fingerprint mismatch"),
        "unexpected error: {err}"
    );
    // The datadir and its state survive intact.
    assert!(datadir.join("state.sqlite").exists());
}

/// A shared counting transport: `run` owns the boxed instance while the
/// test keeps a handle for exact send/id assertions.
struct CountingTransport {
    inner: std::sync::Arc<CountingInner>,
}

struct CountingInner {
    results: std::sync::Mutex<Vec<Result<RegistrationOutcome, String>>>,
    sent: std::sync::Mutex<Vec<u32>>,
}

impl CountingTransport {
    fn new(
        results: Vec<Result<RegistrationOutcome, String>>,
    ) -> (Self, std::sync::Arc<CountingInner>) {
        let inner = std::sync::Arc::new(CountingInner {
            results: std::sync::Mutex::new(results),
            sent: std::sync::Mutex::new(Vec::new()),
        });
        (
            CountingTransport {
                inner: std::sync::Arc::clone(&inner),
            },
            inner,
        )
    }
}

impl spc_agent::wallet::RegistrationTransport for CountingTransport {
    fn send(
        &mut self,
        _payload: &spc_agent::wallet::RegistrationPayload,
        request_id: u32,
    ) -> anyhow::Result<RegistrationOutcome> {
        self.inner.sent.lock().unwrap().push(request_id);
        let mut results = self.inner.results.lock().unwrap();
        if results.is_empty() {
            return Ok(RegistrationOutcome::Retryable {
                error_code: "registration-busy".into(),
            });
        }
        results.remove(0).map_err(|e| anyhow::anyhow!(e))
    }
}

/// run()-level fatal semantics (the terminal-loop fix): a hard terminal
/// (`bad-signature`) sends exactly ONE frame, moves the status to
/// `error` with the exact code, and returns `Err` - the loop never
/// retries a bad signature. Paused time keeps the backoff bounded.
#[tokio::test(start_paused = true)]
async fn run_stops_fatally_on_bad_signature_after_one_send() {
    let dir = tempfile::tempdir().unwrap();
    let (role, _keys_dir) = start_role_async(6, &dir.path().join("datadir")).await;
    let status = std::sync::Arc::clone(role.status());
    let (transport, inner) = CountingTransport::new(vec![Ok(RegistrationOutcome::Terminal {
        error_code: "bad-signature".into(),
    })]);

    let err = role.run(Some(Box::new(transport))).await.unwrap_err();
    assert!(
        err.to_string().contains("bad-signature"),
        "the fatal error surfaces the exact code: {err}"
    );
    assert!(
        err.to_string().contains("operator restart"),
        "the fatal error states the recovery path: {err}"
    );
    // Exactly one send: the loop stopped instead of retrying forever.
    assert_eq!(*inner.sent.lock().unwrap(), vec![1], "exactly one send");
    // Status: error state with the exact code, honest to the operator.
    assert_eq!(status.state(), spc_agent::wallet::WalletState::Error);
    let snapshot = status.snapshot().unwrap();
    assert_eq!(snapshot.state, "error");
    assert_eq!(snapshot.last_error.as_deref(), Some("bad-signature"));
}

/// The bounded unwedge at run() level: mismatch, mismatch (same
/// payload) sends exactly ids 1 and 2, then stops fatally. A wedge kind
/// never spins forever on the same payload.
#[tokio::test(start_paused = true)]
async fn run_stops_fatally_after_two_consecutive_wedge_terminals() {
    let dir = tempfile::tempdir().unwrap();
    let (role, _keys_dir) = start_role_async(7, &dir.path().join("datadir")).await;
    let status = std::sync::Arc::clone(role.status());
    let (transport, inner) = CountingTransport::new(vec![
        Ok(RegistrationOutcome::Mismatch),
        Ok(RegistrationOutcome::Mismatch),
    ]);

    let err = role.run(Some(Box::new(transport))).await.unwrap_err();
    assert!(
        err.to_string().contains("registration-mismatch"),
        "the fatal error names the wedge code: {err}"
    );
    assert_eq!(
        *inner.sent.lock().unwrap(),
        vec![1, 2],
        "exactly two sends with fresh ids (the one unwedge retry)"
    );
    let snapshot = status.snapshot().unwrap();
    assert_eq!(snapshot.state, "error");
    assert_eq!(
        snapshot.last_error.as_deref(),
        Some("registration-mismatch")
    );
}

/// Retryable failures never stop the loop: after six failed attempts
/// local `registration_unavailable` is surfaced in the status while the
/// 30 s retries continue, and a shutdown still stops the role cleanly.
#[tokio::test(start_paused = true)]
async fn run_surfaces_unavailable_and_keeps_retrying_until_shutdown() {
    let dir = tempfile::tempdir().unwrap();
    let (role, _keys_dir) = start_role_async(8, &dir.path().join("datadir")).await;
    let status = std::sync::Arc::clone(role.status());
    let shutdown = role.shutdown_handle();
    // Every send fails at the transport (locally unavailable).
    let (transport, inner) = CountingTransport::new(vec![Err("no route to ark".into())]);
    let task = tokio::spawn(role.run(Some(Box::new(transport))));

    // Paused time auto-advances the backoff; wait for the sixth failure
    // to surface registration_unavailable (the exhaustion signal, not
    // the per-attempt transport-unavailable message).
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(600);
    loop {
        let snapshot = status.snapshot().unwrap();
        if snapshot
            .last_error
            .as_deref()
            .is_some_and(|e| e.contains("registration_unavailable (6+ failed attempts)"))
        {
            assert!(
                inner.sent.lock().unwrap().len() >= 6,
                "unavailable is surfaced by the sixth failed attempt"
            );
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "run never surfaced registration_unavailable: {:?}",
            snapshot
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    // The retries continue past the sixth attempt (30 s cap, forever).
    let n_at_surface = inner.sent.lock().unwrap().len();
    tokio::time::sleep(std::time::Duration::from_secs(120)).await;
    assert!(
        inner.sent.lock().unwrap().len() > n_at_surface,
        "the 30s-capped retries continue while unavailable is surfaced"
    );
    // The role stays honest: not connected, never registered.
    assert!(!status.snapshot().unwrap().registered);

    // Clean shutdown still works from the retry loop.
    shutdown.send(true).unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(status.state(), spc_agent::wallet::WalletState::Stopped);
}

/// The stock bark persister foundation at role level (blocker 1): the
/// role creates `<datadir>/bark` with 0700/0600 files on first start,
/// and a restart with the same identity reopens it without
/// re-initializing.
#[tokio::test]
async fn role_opens_the_stock_bark_persister_across_restarts() {
    let dir = tempfile::tempdir().unwrap();
    let datadir = dir.path().join("datadir");
    let (role, _keys_dir) = start_role_async(9, &datadir).await;
    let bark_dir = datadir.join("bark");
    assert!(bark_dir.join("db.sqlite").exists(), "the stock db exists");
    assert!(
        bark_dir.join("spc-initialized").exists(),
        "the completion marker exists"
    );
    assert!(role.bark().is_fresh(), "the first start initializes");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&bark_dir).unwrap().permissions().mode() & 0o777,
            0o700,
            "bark dir is 0700"
        );
        assert_eq!(
            std::fs::metadata(bark_dir.join("db.sqlite"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600,
            "bark db is 0600"
        );
    }
    drop(role);

    // Same identity, same datadir: a normal reopen, not a re-init.
    let (role2, _keys_dir2) = start_role_async(9, &datadir).await;
    assert!(
        !role2.bark().is_fresh(),
        "a restart must not re-initialize the bark wallet"
    );
}

/// Fix-r2 (Fable A cross-binding): a wholly missing `bark/` directory
/// while `state.sqlite` records an initialized bark wallet is partial
/// datadir damage - the role refuses to start rather than silently
/// re-initializing a fresh bark wallet. The deliberate reset (the whole
/// datadir gone, both databases together) stays fresh; see the
/// recovery tests.
#[tokio::test]
async fn role_refuses_whole_bark_dir_loss_as_partial_damage() {
    let dir = tempfile::tempdir().unwrap();
    let keys_dir = tempfile::tempdir().unwrap();
    let datadir = dir.path().join("datadir");
    let (sources, _sp_address) =
        provision_files(keys_dir.path(), &scenario_master(10), Network::Regtest);
    let settings = test_settings(&sources, &datadir, SocketAddr::from(([127, 0, 0, 1], 0)));
    {
        let role = WalletRole::start(settings.clone()).await.unwrap();
        assert!(
            role.store().bark_initialized().unwrap(),
            "a completed first start records the bark cross-binding"
        );
    }

    // Partial damage: only the bark directory is removed; state.sqlite
    // (and the cross-binding row in its meta table) survive.
    std::fs::remove_dir_all(datadir.join("bark")).unwrap();
    assert!(datadir.join("state.sqlite").exists());

    let err = WalletRole::start(settings)
        .await
        .err()
        .expect("partial bark-dir loss must fail closed");
    assert!(
        format!("{err}").contains("partial datadir damage"),
        "the refusal names the damage class: {err}"
    );
    // The refusal must not have re-created the bark directory.
    assert!(
        !datadir.join("bark").exists(),
        "a refused start must not re-initialize the bark wallet"
    );
}
