// Actual native CPU / wallet-service joint component test. Source history and
// wallet startup metadata are explicit fixtures, not funded acceptance.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "explicit sealed native CPU binary and output receipt required"]
async fn sealed_native_cpu_waits_for_actual_wallet_collector_commit_before_first_hash() {
    use bitcoin::hashes::{sha256, Hash, HashEngine};
    // Supplied by the retention owner's exact shared DTO module (017fa69f...).
    // The primary union must include that module; no wallet production change.
    use spc_scan::native_evidence as evidence;
    use std::io::Read;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let output =
        PathBuf::from(std::env::var("SPC_NATIVE_JOINT_RECEIPT").expect("fresh receipt output"));
    assert!(output.is_absolute() && output.starts_with("/tmp/opencode") && !output.exists());
    let artifact_root = output.with_extension("capture");
    assert!(
        !artifact_root.exists(),
        "retention artifacts must use a fresh directory"
    );
    let binary = PathBuf::from(std::env::var("SPC_NATIVE_CPU_BIN").expect("sealed CPU path"));
    let mut hash = sha256::Hash::engine();
    let mut file = std::fs::File::open(&binary).unwrap();
    let mut chunk = [0u8; 65536];
    loop {
        let n = file.read(&mut chunk).unwrap();
        if n == 0 {
            break;
        }
        hash.input(&chunk[..n]);
    }
    let binary_hash = sha256::Hash::from_engine(hash).to_string();
    assert_eq!(
        binary_hash,
        "fbef4e1c02a8f722705b75dc6a17d79793fc7e73d77cfa06aba250e983c7e5ea"
    );
    let mut f = Fixture::new().await;
    let context = f.runtime.context.clone();
    // Same fully verified empty-history fixture, now delivered to the actual
    // sealed native CPU over real standard Noise/SV2. No node or funding RPC.
    let start = f.bundle.job.coinbase_prefix.len();
    let mut raw = f.bundle.job.coinbase_prefix.clone();
    raw.extend([0; 4]);
    raw.extend(&f.bundle.job.coinbase_suffix);
    let mut tx: Transaction = bitcoin::consensus::deserialize(&raw).unwrap();
    let mut script = tx.input[0].script_sig.clone().into_bytes();
    script.extend([0; 4]);
    tx.input[0].script_sig = ScriptBuf::from_bytes(script);
    let raw = bitcoin::consensus::serialize(&tx);
    f.bundle.job.coinbase_prefix = raw[..start].to_vec();
    f.bundle.job.coinbase_suffix = raw[start + 8..].to_vec();
    f.bundle.job.extranonce_len = 8;
    let accounting = spc_pool::config::Accounting {
        work_unit_version: 2,
        work_reference_target: Some(hex::encode(f.bundle.reference_target_be)),
        share_work_multiplier: Some(1),
        ..Default::default()
    };
    let policy = accounting
        .share_policy(spc_crypto::Network::Regtest)
        .unwrap();
    let work = policy
        .job_work(f.bundle.job.nbits, f.bundle.job.generation)
        .unwrap();
    assert_eq!(
        work.share_target.to_be_bytes(),
        f.bundle.reference_target_be
    );
    let (shares, _received_shares) = tokio::sync::mpsc::channel(64);
    let server = spc_pool::sv2_server::Sv2Server::new(
        spc_crypto::Network::Regtest,
        shares,
        Arc::new(Mutex::new(spc_pool::type32::Type32Store::default())),
        8,
    )
    .with_type32_enabled(false);
    let (endpoint, listener) = server.bind("127.0.0.1:0").await.unwrap();
    server
        .publish(spc_pool::sv1::Job {
            job_id: format!("{:08x}", f.bundle.job.job_id as u32),
            prev_hash: f.bundle.job.prev_hash,
            height: 1,
            version: f.bundle.job.version,
            nbits: f.bundle.job.nbits,
            ntime: f.bundle.job.ntime,
            coinb1: f.bundle.job.coinbase_prefix.clone(),
            coinb2: f.bundle.job.coinbase_suffix.clone(),
            merkle_branches: vec![],
            clean_jobs: true,
            work,
        })
        .await;
    let bundle = f.bundle;
    let collector = f.collector;
    let (consumed_tx, consumed_rx) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let wallet_public = context.keys.wallet;
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
        assert_eq!(
            job.job_id, 17,
            "SV2 carries the u32 wire ID; the proof retains full height/id"
        );
        assert_eq!(job.coinbase_prefix, bundle.job.coinbase_prefix);
        assert_eq!(job.coinbase_suffix, bundle.job.coinbase_suffix);
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
            panic!("consumed")
        };
        assert_eq!(schema, source_feed::SCHEMA);
        assert_eq!(ack.window, WindowFacts::from_work(2, 0));
        source_feed::verify(ack.signing_digest(), &ack.signature, &wallet_public).unwrap();
        consumed_tx
            .send((attempt, job.digest(), ack.signing_digest()))
            .unwrap();
        released.await.unwrap();
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
    let drive = async {
        // Startup-only, bounded capture in this fixture's private native root.
        // The manifest/session is created by the sealed CPU, not by this test.
        let run_root = context.private.parent().unwrap();
        std::fs::set_permissions(run_root, std::fs::Permissions::from_mode(0o700)).unwrap();
        let run_manifest = run_root.join("run-manifest.json");
        private_write(&run_manifest, br#"{"run_id":"test-run","scope":"synthetic joint-test run marker; not a controller manifest"}"#);
        let capture_parent = context.private.join("native-evidence");
        let capture_root = capture_parent.join("miner-alice");
        for directory in [&capture_parent, &capture_root] {
            std::fs::create_dir(directory).unwrap();
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let capture = evidence::Config {
            schema: evidence::CONFIG_SCHEMA.into(),
            run_id: "test-run".into(),
            directory: capture_root.clone(),
            run_manifest: run_manifest.clone(),
            max_records: 8,
            max_bytes: 8 * 1024 * 1024,
            max_sessions: 2,
        };
        capture.validate("miner-alice").unwrap();
        assert!(std::fs::read_dir(&capture_root).unwrap().next().is_none());
        let config = context.private.join("native-cpu.toml");
        let cpu_config = serde_json::json!({"config_version":1,"miner_id":"miner-alice",
            "event_queue_capacity":4096,"connections":[],"native_evidence":capture});
        let config_bytes = toml::to_string(&cpu_config).unwrap().into_bytes();
        assert!(!cpu_config["native_evidence"]
            .as_object()
            .unwrap()
            .keys()
            .any(|k| k.contains("secret") || k.contains("seed") || k.contains("scan_key")));
        private_write(&config, &config_bytes);
        let mut cpu = tokio::process::Command::new(&binary)
            .arg("--config")
            .arg(&config)
            .env("SPC_LAB_RUN_TOKEN", "test-run:fixture")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut input = cpu.stdin.take().unwrap();
        let mut lines = BufReader::new(cpu.stdout.take().unwrap()).lines();
        async fn send(
            input: &mut tokio::process::ChildStdin,
            id: &str,
            op: &str,
            payload: serde_json::Value,
        ) {
            let mut bytes=serde_json::to_vec(&serde_json::json!({"protocol":"spc-cpu-miner-control","version":1,"command_id":id,"operation":op,"payload":payload})).unwrap();
            bytes.push(b'\n');
            input.write_all(&bytes).await.unwrap();
            input.flush().await.unwrap();
        }
        async fn next(
            lines: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
        ) -> serde_json::Value {
            let line = tokio::time::timeout(Duration::from_secs(4), lines.next_line())
                .await
                .unwrap()
                .unwrap()
                .expect("native output");
            serde_json::from_str(&line).unwrap()
        }
        send(&mut input, "ready", "snapshot", serde_json::json!({})).await;
        let startup = loop {
            let message = next(&mut lines).await;
            if message["command_id"] == "ready" {
                assert_eq!(message["outcome"], "succeeded");
                break message;
            }
        };
        let (observed, path) = io::process(
            cpu.id().unwrap(),
            context.instance.process.uid,
            &context.private,
            Some(&config),
            "test-run",
        )
        .unwrap();
        assert_eq!(hex::encode(observed.executable_sha256), binary_hash);
        assert_eq!(observed.config_sha256, io::hash(&config_bytes));
        assert_eq!(path, config);
        let session = evidence::session_name(&observed);
        let manifest_relative = format!("{session}/manifest.json");
        let manifest_bytes = joint_capture_file(
            &capture_root,
            &manifest_relative,
            evidence::MAX_MANIFEST_BYTES,
            observed.uid,
        );
        let manifest = evidence::decode_manifest(&manifest_bytes).unwrap();
        assert_eq!(manifest.config, capture);
        assert_eq!(manifest.process, observed);
        assert_eq!(manifest.config_path, config);
        assert_eq!(manifest.miner_id, "miner-alice");
        let manifest_hash = evidence::sha256_hex(&manifest_bytes);
        let locator = serde_json::json!({"schema":evidence::LOCATOR_SCHEMA,"capability":evidence::CAPABILITY,
            "directory":capture_root,"manifest_relative_path":manifest_relative,"manifest_sha256":manifest_hash});
        assert_eq!(
            startup["result"]["native_evidence"], locator,
            "normal Snapshot must bind the durable capture capability to its actual manifest"
        );
        assert!(startup["result"]["workers"].as_array().unwrap().is_empty());
        assert!(startup["result"]["connections"]
            .as_array()
            .unwrap()
            .is_empty());
        let mut connection = serde_json::json!({"id":"conn-alice","endpoint":endpoint.to_string(),"identity":context.instance.identity,"authority_public_key":server.authority_public_key().to_string(),"min_extranonce_size":4,
            "reconnect":{"enabled":false,"initial_backoff_ms":100,"max_backoff_ms":5000,"handshake_timeout_ms":10000},
            "workers":[{"id":"worker-alice","budget":{"hashes_per_second":40,"total_attempts":40},"ntime_policy":"hold","max_ntime_roll_seconds":600,"batch_size":1}],
            "source_audit":{"mode":"required","run_id":"test-run","miner_id":"miner-alice","wallet_socket":context.instance.socket_path,"wallet_instance":context.instance.instance_id,"wallet_process":context.instance.process,"wallet_socket_device":context.instance.socket_device,"wallet_socket_inode":context.instance.socket_inode,"peer_generation":1,"connection_config_digest":vec![0u8;32],"collector_pubkey":context.instance.collector_pubkey,"pool_pubkey":context.instance.pool_pubkey}});
        let digest = wire::connection_config_digest(&connection).unwrap();
        connection["source_audit"]["connection_config_digest"] = serde_json::json!(digest);
        let registry = PeerRegistry {
            schema: wire::REGISTRY_SCHEMA.into(),
            run_id: "test-run".into(),
            wallet_instance: context.instance.instance_id,
            generation: 1,
            peers: vec![wire::PeerBinding {
                miner_id: "miner-alice".into(),
                connection_id: "conn-alice".into(),
                identity: context.instance.identity.clone(),
                pool_id: context.instance.pool_id.clone(),
                front_id: context.instance.front_id.clone(),
                generation: 1,
                process: observed,
                config_path: path.to_str().unwrap().into(),
                connection_config_digest: digest,
            }],
        };
        io::atomic_private(
            &context.root.join(wire::REGISTRY_FILENAME),
            &serde_json::to_vec(&registry).unwrap(),
            context.instance.process.uid,
        )
        .unwrap();
        send(
            &mut input,
            "add",
            "add_connection",
            serde_json::json!({"connection":connection}),
        )
        .await;
        let (source_attempt, source_job_digest, source_consumed_digest) =
            tokio::time::timeout(Duration::from_secs(4), consumed_rx)
                .await
                .unwrap()
                .unwrap();
        send(
            &mut input,
            "before-commit",
            "snapshot",
            serde_json::json!({}),
        )
        .await;
        let before = loop {
            let message = next(&mut lines).await;
            assert!(
                !matches!(
                    message["event"]["name"].as_str(),
                    Some(
                        "source_audit_committed"
                            | "job_activated"
                            | "audit_first_hash"
                            | "submitted"
                    )
                ),
                "native activated before collector COMMITTED: {message}"
            );
            if message["command_id"] == "add" {
                assert_eq!(message["outcome"], "running", "{message}");
            }
            if message["command_id"] == "before-commit" {
                break message;
            }
        };
        let workers = before["result"]["workers"].as_array().unwrap();
        assert_eq!(workers.len(), 1);
        assert_eq!(workers[0]["attempted"], "0");
        assert!(workers[0]["active_job_id"].is_null());
        assert!(before["result"]["connections"][0]["active_job"].is_null());
        assert_eq!(before["result"]["native_evidence"], locator);
        assert_eq!(
            joint_capture_file(
                &capture_root,
                &manifest_relative,
                evidence::MAX_MANIFEST_BYTES,
                context.instance.process.uid
            ),
            manifest_bytes
        );
        let before_files = std::fs::read_dir(capture_root.join(&session))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect::<Vec<_>>();
        assert_eq!(before_files, vec![std::ffi::OsString::from("manifest.json")],
            "before collector COMMITTED there must be no retained request/Approved pair or pending capture");
        release.send(()).unwrap();
        let mut committed = None;
        let mut activated = None;
        let mut retained = None;
        let first = loop {
            let message = next(&mut lines).await;
            match message["event"]["name"].as_str() {
                Some("source_audit_committed") => {
                    assert!(committed.is_none(), "one initial activation is expected");
                    assert_eq!(message["event"]["payload"]["capability"], wire::CAPABILITY);
                    let handle: evidence::DurableHandle = serde_json::from_value(
                        message["event"]["payload"]["durable_evidence"].clone(),
                    )
                    .expect("durable handle is mandatory");
                    assert_eq!(handle.capability, evidence::CAPABILITY);
                    assert_eq!(handle.manifest_relative_path, manifest_relative);
                    assert_eq!(handle.manifest_sha256, manifest_hash);
                    let record_bytes = joint_capture_file(
                        &capture_root,
                        &handle.record_relative_path,
                        evidence::MAX_RECORD_BYTES,
                        context.instance.process.uid,
                    );
                    // Shared decoder validates record/manifest/payload hashes,
                    // exact request binding, signatures and captured expiry.
                    let (record, request, response) = evidence::decode_record(
                        &record_bytes,
                        &manifest_bytes,
                        &handle,
                        &context.keys,
                    )
                    .unwrap();
                    assert_eq!(request.nonce, source_attempt);
                    assert_eq!(request.job.digest(), source_job_digest);
                    assert_eq!(request.identity, context.instance.identity);
                    assert_eq!(request.wallet_instance, context.instance.instance_id);
                    assert_eq!(request.connection_config_digest, digest);
                    assert_eq!(request.peer_generation, 1);
                    assert_eq!(
                        record.request_digest,
                        message["event"]["payload"]["request_digest"]
                    );
                    assert_eq!(
                        record.checkpoint_digest,
                        message["event"]["payload"]["checkpoint_digest"]
                    );
                    assert_eq!(record.activation, "not_asserted");
                    let request_bytes = hex::decode(&record.request_json_hex).unwrap();
                    let response_bytes = hex::decode(&record.response_json_hex).unwrap();
                    assert_eq!(evidence::sha256_hex(&request_bytes), record.request_sha256);
                    assert_eq!(
                        evidence::sha256_hex(&response_bytes),
                        record.response_sha256
                    );
                    assert_eq!(wire::encode_request(&request).unwrap(), request_bytes);
                    let cached = context
                        .tracker
                        .lock()
                        .unwrap()
                        .latest
                        .get(&(request.miner_id.clone(), request.connection_id.clone()))
                        .unwrap()
                        .cached
                        .as_ref()
                        .unwrap()
                        .1
                        .clone();
                    assert_eq!(
                        &cached[4..],
                        response_bytes.as_slice(),
                        "retained response must be the exact actual wallet-service response"
                    );
                    let Response::Approved { receipt, .. } = &response else {
                        panic!("retained Approved required")
                    };
                    assert_eq!(
                        receipt.evidence.consumed.signing_digest(),
                        source_consumed_digest
                    );
                    assert_eq!(
                        hex::encode(source_consumed_digest),
                        message["event"]["payload"]["consumed_digest"]
                    );
                    retained = Some((handle, record, record_bytes, request_bytes, response_bytes));
                    committed = Some(message);
                }
                Some("job_activated") => {
                    assert!(committed.is_some() && retained.is_some());
                    activated = Some(message);
                }
                Some("audit_first_hash") => {
                    assert!(committed.is_some() && activated.is_some() && retained.is_some());
                    break message;
                }
                _ => {}
            }
        };
        let committed = committed.unwrap();
        let activated = activated.unwrap();
        assert!(committed["event_id"].as_u64() < activated["event_id"].as_u64());
        assert!(activated["event_id"].as_u64() < first["event_id"].as_u64());
        assert_eq!(
            committed["event"]["payload"]["request_digest"],
            first["event"]["payload"]["request_digest"]
        );
        assert_eq!(
            committed["event"]["payload"]["candidate_epoch"],
            first["event"]["payload"]["candidate_epoch"]
        );
        send(&mut input, "stop", "stop", serde_json::json!({})).await;
        drop(input);
        assert!(tokio::time::timeout(Duration::from_secs(4), cpu.wait())
            .await
            .unwrap()
            .unwrap()
            .success());
        let (handle, record, record_bytes, request_bytes, response_bytes) = retained.unwrap();
        // Keep CPU-created bytes unchanged before the temporary runtime is
        // dropped. Original locator paths remain provenance, not live leases.
        let preserved_capture = artifact_root.join("native-evidence");
        let preserved_session = preserved_capture.join(&session);
        for directory in [&artifact_root, &preserved_capture, &preserved_session] {
            std::fs::create_dir(directory).unwrap();
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        assert_eq!(
            joint_capture_file(
                &capture_root,
                &manifest_relative,
                evidence::MAX_MANIFEST_BYTES,
                context.instance.process.uid
            ),
            manifest_bytes
        );
        assert_eq!(
            joint_capture_file(
                &capture_root,
                &handle.record_relative_path,
                evidence::MAX_RECORD_BYTES,
                context.instance.process.uid
            ),
            record_bytes
        );
        private_write(&preserved_capture.join(&manifest_relative), &manifest_bytes);
        private_write(
            &preserved_capture.join(&handle.record_relative_path),
            &record_bytes,
        );
        private_write(&artifact_root.join("request.json"), &request_bytes);
        private_write(&artifact_root.join("response.json"), &response_bytes);
        private_write(&artifact_root.join("cpu-config.toml"), &config_bytes);
        private_write(
            &artifact_root.join("run-manifest.fixture.json"),
            &io::private_read(&run_manifest, context.instance.process.uid).unwrap(),
        );
        let receipt = serde_json::json!({"scope":"actual sealed retention-enabled native CPU + actual typed wallet service; fixture wallet startup metadata and independent-key synthetic empty-history collector; not actual controller/full-history/funded acceptance or separate N1/N2 coverage",
            "native_binary_sha256":binary_hash,"wallet_source_code":std::env::var("SPC_WALLET_TEST_SOURCE").ok(),
            "startup_snapshot":startup,"capture_config":capture,"capture_locator":locator,"capture_manifest":manifest,
            "capture_manifest_sha256":manifest_hash,"retained_handle":handle,"retained_record_sha256":evidence::sha256_hex(&record_bytes),
            "request_sha256":record.request_sha256,"response_sha256":record.response_sha256,
            "original_capture_root":capture_root,"preserved_capture_root":preserved_capture,"before_capture_and_commit":before,
            "committed":committed,"activated":activated,"first_hash":first,"funded_acceptance":false,"N1_N2_coverage_claimed":false});
        private_write(&output, &serde_json::to_vec_pretty(&receipt).unwrap());
    };
    tokio::select! { _=drive=>{},result=f.runtime.serve_owner(&f.keys)=>panic!("wallet owner stopped: {result:?}") }
    collecting.await.unwrap();
    f.runtime.stop().await.unwrap();
    listener.abort();
}

fn joint_capture_file(root: &Path, relative: &str, bound: usize, uid: u32) -> Vec<u8> {
    let relative = Path::new(relative);
    assert!(!relative.as_os_str().is_empty() && !relative.is_absolute());
    assert!(relative
        .components()
        .all(|part| matches!(part, std::path::Component::Normal(_))));
    // This deliberately small fixture also fits the wallet's existing private
    // reader bound, with unchanged owner/mode/link and read-stability checks.
    let bytes = io::private_read(&root.join(relative), uid).unwrap();
    assert!(bytes.len() <= bound);
    bytes
}
