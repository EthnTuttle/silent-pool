// Actual native CPU / wallet-service joint component test. Source history and
// wallet startup metadata are explicit fixtures, not funded acceptance.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "explicit sealed native CPU binary and output receipt required"]
async fn sealed_native_cpu_waits_for_actual_wallet_collector_commit_before_first_hash() {
    use bitcoin::hashes::{sha256, Hash, HashEngine};
    use std::io::Read;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
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
        "7b62234fad71dc5413edc4c7fddcf16d082445da2bec52bf20b1d6e092997cb3"
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
        consumed_tx.send(()).unwrap();
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
        let config = context.private.join("native-cpu.toml");
        private_write(&config,b"config_version = 1\nminer_id = 'miner-alice'\nevent_queue_capacity = 4096\nconnections = []\n");
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
        loop {
            let message = next(&mut lines).await;
            if message["command_id"] == "ready" {
                assert_eq!(message["outcome"], "succeeded");
                break;
            }
        }
        let (observed, path) = io::process(
            cpu.id().unwrap(),
            context.instance.process.uid,
            &context.private,
            Some(&config),
            "test-run",
        )
        .unwrap();
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
        release.send(()).unwrap();
        let mut committed = None;
        let mut activated = None;
        let first = loop {
            let message = next(&mut lines).await;
            match message["event"]["name"].as_str() {
                Some("source_audit_committed") => committed = Some(message),
                Some("job_activated") => {
                    assert!(committed.is_some());
                    activated = Some(message);
                }
                Some("audit_first_hash") => {
                    assert!(committed.is_some() && activated.is_some());
                    break message;
                }
                _ => {}
            }
        };
        let committed = committed.unwrap();
        let activated = activated.unwrap();
        assert!(committed["event_id"].as_u64() < activated["event_id"].as_u64());
        assert!(activated["event_id"].as_u64() < first["event_id"].as_u64());
        send(&mut input, "stop", "stop", serde_json::json!({})).await;
        drop(input);
        assert!(tokio::time::timeout(Duration::from_secs(4), cpu.wait())
            .await
            .unwrap()
            .unwrap()
            .success());
        let receipt = serde_json::json!({"scope":"actual sealed native CPU + actual typed wallet service; fixture wallet startup metadata and independent-key synthetic empty-history collector; no funded/whole-Q acceptance","native_binary_sha256":binary_hash,"wallet_source_code":std::env::var("SPC_WALLET_TEST_SOURCE").ok(),"before_commit":before,"committed":committed,"activated":activated,"first_hash":first,"funded_acceptance":false});
        let output =
            PathBuf::from(std::env::var("SPC_NATIVE_JOINT_RECEIPT").expect("receipt output"));
        assert!(output.starts_with("/tmp/opencode"));
        private_write(&output, &serde_json::to_vec_pretty(&receipt).unwrap());
    };
    tokio::select! { _=drive=>{},result=f.runtime.serve_owner(&f.keys)=>panic!("wallet owner stopped: {result:?}") }
    collecting.await.unwrap();
    f.runtime.stop().await.unwrap();
    listener.abort();
}
