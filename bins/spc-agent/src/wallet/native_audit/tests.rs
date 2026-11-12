//! Typed service tests. Wallet startup metadata is fixture-controlled; CPU peer
//! credentials/proc facts and the Unix/source-verifier exchange are real. The
//! collector's empty-history seal is mocked, never funded/native-run evidence.
use super::*;
use bitcoin::secp256k1::SecretKey;
use bitcoin::{Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness};
use spc_scan::native_activation::PeerRegistry;
use spc_scan::source_audit::{
    history_digest, HistoryCheckpoint, JobProof, SourceBundle, WindowEvidence, WindowFacts,
};
use spc_scan::source_feed::{Checkpoint, Frame, TraceSeal};
use std::io::{BufRead, Write};
use std::process::{Command, Stdio};
include!("joint_test.rs");

fn key(n: u8) -> Keypair {
    Keypair::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&[n; 32]).unwrap())
}
fn job(identity: [u8; 32]) -> (source_feed::ReceivedJob, SourceBundle) {
    let send = spc_crypto::SendKey::generate(1).unwrap();
    let immutable = spc_crypto::emit_script_sig(1, send.x_only());
    let mut script = immutable.clone();
    script.extend([0; 4]);
    let mut pool_script = vec![0x51, 0x20];
    pool_script.extend(key(9).x_only_public_key().0.serialize());
    let tx = Transaction {
        version: bitcoin::transaction::Version::ONE,
        lock_time: bitcoin::absolute::LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::null(),
            script_sig: ScriptBuf::from_bytes(script),
            sequence: Sequence::MAX,
            witness: Witness::new(),
        }],
        output: vec![TxOut {
            value: Amount::from_sat(5_000_000_000),
            script_pubkey: ScriptBuf::from_bytes(pool_script),
        }],
    };
    let raw = bitcoin::consensus::serialize(&tx);
    let start = 4 + 1 + 36 + 1 + immutable.len();
    let mut reference = [0xff; 32];
    reference[0] = 0x7f;
    let cfg = spc_pool::config::Accounting {
        work_unit_version: 2,
        work_reference_target: Some(hex::encode(reference)),
        ..Default::default()
    };
    let fingerprint = hex::decode(
        cfg.profile_json()["work_unit_fingerprint"]
            .as_str()
            .unwrap(),
    )
    .unwrap()
    .try_into()
    .unwrap();
    let received = source_feed::ReceivedJob {
        job_id: 17,
        prev_hash: [0; 32],
        version: 0x2000_0000,
        nbits: 0x207f_ffff,
        ntime: 1,
        coinbase_prefix: raw[..start].to_vec(),
        coinbase_suffix: raw[start + 4..].to_vec(),
        merkle_path: vec![],
        share_target_be: reference,
        pool_extranonce: vec![],
        miner_extranonce_len: 4,
    };
    let pending = BTreeMap::from([(hex::encode(identity), 0)]);
    let history = history_digest(2, &pending).unwrap();
    let bundle = SourceBundle {
        schema: "spc.payout-proof/1".into(),
        source_complete: true,
        source_watermark: 3,
        job: JobProof {
            job_id: (1u64 << 32) | 17,
            generation: 17,
            height: 1,
            prev_hash: received.prev_hash,
            version: received.version,
            ntime: received.ntime,
            nbits: received.nbits,
            coinbase_prefix: received.coinbase_prefix.clone(),
            coinbase_suffix: received.coinbase_suffix.clone(),
            extranonce_len: 4,
            merkle_path: vec![],
            share_target_be: reference,
            credited_work: 1,
            transactions: vec![],
            prevout_transactions: vec![],
        },
        jobs: vec![],
        reference_target_be: reference,
        work_unit_fingerprint: fingerprint,
        window_n: 1,
        window_target: "2".into(),
        high_water: BTreeMap::from([(0, 0)]),
        finding_slice_id: 1,
        slices: vec![],
        subsidy: 5_000_000_000,
        fees: 0,
        fee_bps: 100,
        prior: HistoryCheckpoint {
            watermark: 2,
            digest: history,
            pending,
        },
        owed: BTreeMap::from([(hex::encode(identity), 0)]),
        window: Some(WindowEvidence {
            facts: WindowFacts::from_work(2, 0),
            eligible_history_complete: true,
        }),
        witness_reserved: BTreeMap::new(),
    };
    (received, bundle)
}
fn candidate(job: source_feed::ReceivedJob, identity: String) -> CandidateRequest {
    CandidateRequest {
        schema: wire::SCHEMA.into(),
        run_id: "test-run".into(),
        miner_id: "miner-alice".into(),
        connection_id: "conn-alice".into(),
        identity,
        wallet_instance: [1; 16],
        peer_generation: 1,
        connection_config_digest: [2; 32],
        expires_at_unix_ms: now().unwrap() + 9000,
        session_epoch: 0,
        candidate_epoch: 1,
        nonce: [7; 16],
        job,
    }
}

#[test]
fn tracker_exact_replay_supersession_and_nonce_rebinding_fail_closed() {
    let address = spc_crypto::address::encode_sp_address(
        spc_crypto::Network::Regtest,
        &key(7).public_key(),
        &key(8).public_key(),
    )
    .unwrap();
    let (job, _) = job(spc_crypto::sp_id(
        &key(7).public_key(),
        &key(8).public_key(),
    ));
    let mut r = candidate(job, address);
    let mut tracker = Tracker::default();
    let (old, cache) = tracker.admit(&r, now().unwrap()).unwrap();
    assert!(cache.is_none());
    assert!(matches!(
        tracker.admit(&r, now().unwrap()),
        Err(UnknownCode::Unavailable)
    ));
    tracker
        .cache(&r, r.expires_at_unix_ms, b"exact cached frame".to_vec())
        .unwrap();
    assert_eq!(
        tracker.admit(&r, now().unwrap()).unwrap().1.unwrap(),
        b"exact cached frame"
    );
    r.expires_at_unix_ms += 1;
    assert!(matches!(
        tracker.admit(&r, now().unwrap()),
        Err(UnknownCode::Rejected)
    ));
    r.expires_at_unix_ms -= 1;
    r.candidate_epoch += 1;
    assert!(
        matches!(
            tracker.admit(&r, now().unwrap()),
            Err(UnknownCode::Rejected)
        ),
        "same nonce cannot bind another candidate"
    );
    r.nonce = [8; 16];
    tracker.admit(&r, now().unwrap()).unwrap();
    assert!(*old.borrow());
    r.candidate_epoch = 1;
    r.nonce = [9; 16];
    assert!(matches!(
        tracker.admit(&r, now().unwrap()),
        Err(UnknownCode::Superseded)
    ));
}

fn private_write(path: &Path, bytes: &[u8]) {
    std::fs::write(path, bytes).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
}

struct Fixture {
    _dir: tempfile::TempDir,
    keys: WalletKeys,
    runtime: NativeRuntime,
    request: CandidateRequest,
    bundle: SourceBundle,
    collector: tokio::net::UnixListener,
}
impl Fixture {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let private = dir.path().join("private");
        std::fs::create_dir(&private).unwrap();
        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o700)).unwrap();
        let wallet = private.join("wallet-alice");
        let provision =
            super::super::provision::provision(&wallet, &private.join("cold-alice")).unwrap();
        let keys = WalletKeys::load(&super::super::KeySources {
            scan_xprv: wallet.join("scan.xprv"),
            ark_xprv: wallet.join("ark.xprv"),
            child_seed: wallet.join("child.seed"),
            sp_address: provision.sp_address.clone(),
            network: spc_crypto::Network::Regtest,
        })
        .unwrap();
        let root = wallet.join("state");
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        let config = wallet.join("agent.toml");
        private_write(
            &config,
            b"fixture wallet startup metadata; no live bootstrap claim",
        );
        let socket = root.join(wire::SOCKET_FILENAME);
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        let sm = socket.metadata().unwrap();
        let rm = root.metadata().unwrap();
        let collector_path = private.join("collector.sock");
        let collector = tokio::net::UnixListener::bind(&collector_path).unwrap();
        std::fs::set_permissions(&collector_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let instance = WalletInstance {
            schema: wire::INSTANCE_SCHEMA.into(),
            instance_id: [1; 16],
            run_id: "test-run".into(),
            identity_id: "alice".into(),
            identity: provision.sp_address.clone(),
            pool_id: "pool-a".into(),
            front_id: "front-a".into(),
            process: ProcessIdentity {
                pid: std::process::id(),
                uid: io::uid().unwrap(),
                start_time_ticks: io::start_ticks(std::process::id()).unwrap(),
                boot_id: std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
                    .unwrap()
                    .trim()
                    .into(),
                executable_sha256: [9; 32],
                run_token_sha256: io::hash(b"test-run:fixture"),
                config_sha256: io::hash(&std::fs::read(&config).unwrap()),
            },
            config_path: config.to_str().unwrap().into(),
            socket_path: socket.to_str().unwrap().into(),
            socket_device: sm.dev(),
            socket_inode: sm.ino(),
            collector_pubkey: key(10).x_only_public_key().0.to_string(),
            pool_pubkey: key(9).x_only_public_key().0.to_string(),
        };
        let verification = wire::VerificationKeys::for_regtest_identity(
            &instance.identity,
            &instance.collector_pubkey,
            &instance.pool_pubkey,
        )
        .unwrap();
        let context = Arc::new(ContextState {
            source: SourceBinding {
                socket: collector_path,
                pool: verification.pool,
                collector: verification.collector,
            },
            instance,
            root,
            private,
            root_device: rm.dev(),
            root_inode: rm.ino(),
            keys: verification,
            published: AtomicBool::new(false),
            tracker: Mutex::new(Tracker::default()),
        });
        let (auditors, factory) = auditor_channel(4).unwrap();
        let (signer, signing) = mpsc::channel(4);
        let service = service::Service::start(
            listener,
            context.clone(),
            service::Limits {
                concurrent_sessions: 4,
                total_deadline: Duration::from_secs(10),
                max_encoded_response_bytes: wire::MAX_RESPONSE_BYTES + 4,
            },
            Arc::new(Authorizer {
                context: context.clone(),
            }),
            Arc::new(Handler {
                context: context.clone(),
                auditors,
                signer,
            }),
        )
        .await
        .unwrap();
        io::atomic_private(
            &context.root.join(wire::INSTANCE_FILENAME),
            &serde_json::to_vec(&context.instance).unwrap(),
            context.instance.process.uid,
        )
        .unwrap();
        context.published.store(true, Ordering::SeqCst);
        let (job, bundle) = job(keys.sp_id);
        let request = candidate(job, provision.sp_address);
        Self {
            _dir: dir,
            keys,
            runtime: NativeRuntime {
                context,
                service: Some(service),
                factory: Some(factory),
                signing: Some(signing),
            },
            request,
            bundle,
            collector,
        }
    }
}

fn cpu(context: &ContextState, r: &CandidateRequest, registered: bool) -> std::process::Child {
    let config = context.private.join("cpu.toml");
    private_write(
        &config,
        b"config_version = 1\nminer_id = 'miner-alice'\nconnections = []\n",
    );
    assert!(!std::fs::read_to_string(&config).unwrap().contains("scan"));
    let body = wire::encode_request(r).unwrap();
    let mut frame = (body.len() as u32).to_le_bytes().to_vec();
    frame.extend(body);
    let script="import socket,sys,struct\nsys.stdout.write('READY\\n');sys.stdout.flush()\nsys.stdin.buffer.read(1)\ns=socket.socket(socket.AF_UNIX);s.connect(sys.argv[3]);s.sendall(bytes.fromhex(sys.argv[4]))\ndef rd(n):\n b=b''\n while len(b)<n:\n  x=s.recv(n-len(b))\n  if not x: raise RuntimeError('closed')\n  b+=x\n return b\nh=rd(4);b=rd(struct.unpack('<I',h)[0]);sys.stdout.buffer.write(b)\n";
    let mut child = Command::new("python3")
        .args([
            "-c",
            script,
            "--config",
            config.to_str().unwrap(),
            &context.instance.socket_path,
            &hex::encode(frame),
        ])
        .env("SPC_LAB_RUN_TOKEN", "test-run:fixture")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Observe only the actual interpreter, after any executable wrapper has
    // completed. The fixture still waits for release before opening the socket.
    let mut ready = String::new();
    std::io::BufReader::new(child.stdout.as_mut().unwrap())
        .read_line(&mut ready)
        .unwrap();
    assert_eq!(ready, "READY\n");
    let (process, path) = io::process(
        child.id(),
        context.instance.process.uid,
        &context.private,
        Some(&config),
        "test-run",
    )
    .unwrap();
    let registry = PeerRegistry {
        schema: wire::REGISTRY_SCHEMA.into(),
        run_id: "test-run".into(),
        wallet_instance: context.instance.instance_id,
        generation: 1,
        peers: if registered {
            vec![wire::PeerBinding {
                miner_id: r.miner_id.clone(),
                connection_id: r.connection_id.clone(),
                identity: r.identity.clone(),
                pool_id: "pool-a".into(),
                front_id: "front-a".into(),
                generation: 1,
                process,
                config_path: path.to_str().unwrap().into(),
                connection_config_digest: r.connection_config_digest,
            }]
        } else {
            vec![]
        },
    };
    io::atomic_private(
        &context.root.join(wire::REGISTRY_FILENAME),
        &serde_json::to_vec(&registry).unwrap(),
        context.instance.process.uid,
    )
    .unwrap();
    child
}
async fn result(mut child: std::process::Child) -> Response {
    child.stdin.take().unwrap().write_all(b"x").unwrap();
    let output = tokio::task::spawn_blocking(move || child.wait_with_output().unwrap())
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "fixture CPU: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    wire::decode_response(&output.stdout).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_cpu_peer_full_owned_source_commit_then_loaded_wallet_native_signature() {
    let mut f = Fixture::new().await;
    let child = cpu(&f.runtime.context, &f.request, true);
    let bundle = f.bundle;
    let collector = f.collector;
    let collecting = tokio::spawn(async move {
        let (mut stream, _) = collector.accept().await.unwrap();
        let Frame::Job {
            schema,
            attempt,
            job,
        } = crate::source_feed::read_frame(&mut stream).await.unwrap()
        else {
            panic!("source job")
        };
        assert_eq!(schema, source_feed::SCHEMA);
        assert_eq!(attempt, [7; 16]);
        let bytes = serde_json::to_vec(&bundle).unwrap();
        let mut trace = TraceSeal {
            session: [1; 16],
            seq: 3,
            digest: [2; 32],
            signature: String::new(),
        };
        trace.signature = source_feed::sign(trace.signing_digest(), &key(9));
        let mut checkpoint = Checkpoint {
            attempt,
            wire_job_digest: job.digest(),
            job_id: bundle.job.job_id,
            generation: 17,
            source_watermark: 3,
            history_watermark: 2,
            bundle_digest: io::hash(&bytes),
            history_digest: bundle.prior.digest,
            expires_at_unix_ms: now().unwrap() + 15000,
            complete: true,
            pool: trace,
            signature: String::new(),
        };
        checkpoint.signature = source_feed::sign(checkpoint.signing_digest(), &key(10));
        crate::source_feed::write_frame(
            &mut stream,
            &Frame::Proof {
                schema: source_feed::SCHEMA.into(),
                bundle_hex: hex::encode(bytes),
                checkpoint,
            },
        )
        .await
        .unwrap();
        let Frame::Consumed { schema, ack } =
            crate::source_feed::read_frame(&mut stream).await.unwrap()
        else {
            panic!("source consumed")
        };
        assert_eq!(schema, source_feed::SCHEMA);
        assert_eq!(ack.window, WindowFacts::from_work(2, 0));
        crate::source_feed::write_frame(
            &mut stream,
            &Frame::Committed {
                schema: source_feed::SCHEMA.into(),
                attempt,
                ack_digest: ack.signing_digest(),
                signature: source_feed::sign(
                    source_feed::committed_digest(attempt, ack.signing_digest()),
                    &key(10),
                ),
            },
        )
        .await
        .unwrap();
    });
    let reply = tokio::select! {r=result(child)=>r,r=f.runtime.serve_owner(&f.keys)=>panic!("owner stopped: {r:?}")};
    let proof =
        wire::verify_response(&f.request, &reply, &f.runtime.context.keys, now().unwrap()).unwrap();
    assert!(proof.window.underfilled);
    assert_eq!(proof.expires_at_unix_ms, f.request.expires_at_unix_ms);
    collecting.await.unwrap();
    f.runtime.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unregistered_cpu_and_stale_wallet_instance_never_reach_auditor() {
    for stale in [false, true] {
        let mut f = Fixture::new().await;
        if stale {
            f.request.wallet_instance = [99; 16];
        }
        let child = cpu(&f.runtime.context, &f.request, stale);
        let reply = tokio::select! {r=result(child)=>r,r=f.runtime.serve_owner(&f.keys)=>panic!("owner stopped: {r:?}")};
        assert!(matches!(
            reply,
            Response::Unknown {
                code: UnknownCode::Rejected,
                ..
            }
        ));
        assert!(f.runtime.context.tracker.lock().unwrap().latest.is_empty());
        f.runtime.stop().await.unwrap();
    }
}

#[test]
fn native_profile_scope_is_regtest_and_confined() {
    let profile:crate::config::NativeAudit=serde_json::from_value(serde_json::json!({"run_id":"r","pool_id":"p","front_id":"f","identity_id":"i","collector_socket":"/private/collector.sock","pool_pubkey":key(9).x_only_public_key().0.to_string(),"collector_pubkey":key(10).x_only_public_key().0.to_string()})).unwrap();
    let address = spc_crypto::address::encode_sp_address(
        spc_crypto::Network::Regtest,
        &key(7).public_key(),
        &key(8).public_key(),
    )
    .unwrap();
    profile
        .validate(&address, spc_crypto::Network::Regtest)
        .unwrap();
    assert!(profile
        .validate(&address, spc_crypto::Network::Mainnet)
        .is_err());
    let mut wrong = profile;
    wrong.collector_socket = "/private/../outside.sock".into();
    assert!(wrong
        .validate(&address, spc_crypto::Network::Regtest)
        .is_err());
}

#[tokio::test]
async fn actual_instance_and_socket_replacement_invalidate_existing_service() {
    let f = Fixture::new().await;
    let c = &f.runtime.context;
    let path = c.root.join(wire::INSTANCE_FILENAME);
    let mut changed = c.instance.clone();
    changed.instance_id = [99; 16];
    io::atomic_private(
        &path,
        &serde_json::to_vec(&changed).unwrap(),
        c.instance.process.uid,
    )
    .unwrap();
    assert!(c.endpoint().is_err());
    io::atomic_private(
        &path,
        &serde_json::to_vec(&c.instance).unwrap(),
        c.instance.process.uid,
    )
    .unwrap();
    let socket = PathBuf::from(&c.instance.socket_path);
    let saved = socket.with_extension("saved");
    std::fs::rename(&socket, &saved).unwrap();
    let replacement = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(c.endpoint().is_err());
    drop(replacement);
    std::fs::remove_file(&socket).unwrap();
    std::fs::rename(saved, socket).unwrap();
    c.endpoint().unwrap();
    f.runtime.stop().await.unwrap();
}
