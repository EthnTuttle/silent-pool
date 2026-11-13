//! Actual engine token generation and Supervisor export, with fixture adapters.
//! No daemon/network funding. The joined gate uses the actual agent libtest
//! executable from the same source, without a spc-lab -> spc-agent dependency.
use super::*;
use crate::engine::{EngineOptions, RunEngine};
use crate::supervisor::{ProcessSpec, Readiness};
use std::collections::BTreeMap;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Run {
    engine: Arc<RunEngine>,
    root: PathBuf,
}
impl Drop for Run {
    fn drop(&mut self) {
        self.engine
            .supervisor
            .lock()
            .unwrap()
            .stop_all(Duration::from_secs(2));
        if std::thread::panicking() {
            eprintln!(
                "native startup failure artifacts retained privately: {}",
                self.root.display()
            );
        } else {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}

fn engine(traffic: bool) -> Run {
    let root = std::env::temp_dir().join(format!(
        "native-token-{}",
        hex::encode(rand::random::<[u8; 12]>())
    ));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&root)
        .unwrap();
    let root = root.canonicalize().unwrap();
    let mut scenario: serde_json::Value = serde_json::from_str(include_str!(
        "../tests/fixtures/scenario_v1_one_pool_two_miners.json"
    ))
    .unwrap();
    scenario["run"]["id"] =
        format!("native-token-{}", hex::encode(rand::random::<[u8; 8]>())).into();
    if traffic {
        scenario["expanded_access"] = true.into();
        scenario["traffic"] = serde_json::json!({"version":1,"required_capability":"spc.node.regtest.mempool_rpc.v1","protocol":{"control":"spc-lab-tx-control/v1","history_version":3},
            "bootstrap":{"enabled":true,"mining_mode":"native","funding_blocks":101,"distribution_outputs_per_wallet":8,"excluded_from_statistics":true},
            "driver":{"actors":[{"id":"txd-0","network_wallets":8,"user_payment_wallets":2,"hosts_bootstrap_funding":true}],"user_payment_wallets":2,"history":"traffic/"},
            "rate":{"ordinary_tx_per_sec":0.2,"pacing":"fixed_jitter"},"fees":{"rate_sats_vb":{"min":1,"max":20},"max_fee_sats_per_tx":100000},"sizes":{"classes_vb":[{"vb":220,"mix":1.0}],"max_vb":1000},
            "spend_types":["p2pkh"],"ark":{"enabled":false,"registration_before_mining":true,"chain_effect_budget_tx_per_sec":0.0},"caps":{"max_inflight_broadcasts":1,"max_unknown_retries":3,"max_mempool_backlog_tx":1000,"reorg_resubmit_max":3},"pause_semantics":"no_broadcast_after_ack"});
        scenario["phases"] = serde_json::json!([{"id":"bootstrap","commands":[{"kind":{"type":"bootstrap_fund","driver_id":"txd-0","node_id":"node-a"}}]}]);
    }
    let adapter: crate::config::AdapterConfig = serde_json::from_value(serde_json::json!({"program":std::env::current_exe().unwrap(),"args":[],"fixture":true,"capabilities":["fixture.scripted","spc.node.regtest.mempool_rpc.v1","spc.pool.work_unit_v2","spc.pool.network_block_work"],"effective_profile":{
        "implementation":"fixture-node","network":"regtest","consensus_profile_id":"fixture.regtest.retarget","profile_fingerprint":null,"pow_limit_bits":"207fffff","retarget_floor_bits":"1e7fffff","initial_target_bits":"1e7fffff","regtest_pow_bits_overlay":"1e7fffff","target_spacing_secs":10,"target_timespan_secs":240,"retarget_interval_blocks":24,"no_pow_retargeting":false,"allow_min_difficulty_blocks":false,"minimum_difficulty_time_gap_secs":null,"subsidy_initial_sats":"5000000000","subsidy_halving_interval_blocks":150,"coinbase_maturity_blocks":100,"witness_commitment_supported":true,"tdp_session_cap":8},"rpc_auth":null,"env":{}})).unwrap();
    let mut config = crate::config::LabConfig::default();
    config.nodes.insert("fixture-node".into(), adapter.clone());
    config.pools.insert("fixture-pool".into(), adapter.clone());
    config
        .miners
        .insert("fixture-miner".into(), adapter.clone());
    config.redis = Some(adapter.clone());
    config.postgres = Some(adapter.clone());
    let mut native = adapter;
    native.fixture = false;
    config.traffic_probe = Some(native.clone());
    config.tx_driver = Some(native);
    let engine = RunEngine::prepare(
        serde_json::from_value(scenario).unwrap(),
        EngineOptions {
            run_dir: root.clone(),
            config,
            base_revision: "native-token-test".into(),
            dirty_tree_hash: "fixture-adapters".into(),
            owner_instance: "native-token-owner".into(),
            evidence_capacity: 1024,
            namespaces: BTreeMap::new(),
            nsenter: None,
            user_namespace: None,
            user_namespace_owner: None,
            namespace_owners: BTreeMap::new(),
            namespace_broker: None,
            lifetime_helper: None,
            addresses: BTreeMap::new(),
            link_backend: None,
            operator_ui: false,
        },
    )
    .unwrap();
    Run { engine, root }
}

fn profile() -> (super::super::WalletAgentSpec, String) {
    use bitcoin::secp256k1::{Keypair, Secp256k1, SecretKey};
    let key =
        |n| Keypair::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&[n; 32]).unwrap());
    let address = spc_crypto::address::encode_sp_address(
        spc_crypto::Network::Regtest,
        &key(1).public_key(),
        &key(2).public_key(),
    )
    .unwrap();
    let agent = serde_json::from_value(serde_json::json!({"identity_id":"id-alice","front_id":"front-a1",
        "native_audit":{"collector_socket":"collector.sock", "pool_pubkey":key(3).x_only_public_key().0.to_string(),
            "collector_pubkey":key(4).x_only_public_key().0.to_string()}})).unwrap();
    (agent, address)
}

fn config(
    run: &Run,
    bad_digest: bool,
) -> (
    super::super::WalletAgentSpec,
    String,
    PathBuf,
    serde_json::Value,
) {
    let (agent, address) = profile();
    let private = run.root.join("private");
    std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o700)).unwrap();
    let wallet = private.join("wallet-id-alice");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&wallet)
        .unwrap();
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(wallet.join("state"))
        .unwrap();
    // Call the production renderer against the actual prepared engine.
    let mut native = render(&run.engine, &agent, &address).unwrap().unwrap();
    if bad_digest {
        native["run_token_hash"] = hex::encode(Sha256::digest(b"foreign-run:owner")).into();
    }
    let value = serde_json::json!({"upstream":{"address":"127.0.0.1:34254",
        "authority_public_key":"9auqWEzQDVyd2oe1JVGFLMLHZtCo2FFqZwtKA5gd9xbuEu7PH72", "sp_address":address},
        "ark":{"enabled":true,"network":"regtest","datadir":wallet.join("state"),"native_audit":native}});
    let path = wallet.join("agent.toml");
    write(&path, &value);
    (agent, address, path, value)
}
fn write(path: &Path, value: &serde_json::Value) {
    std::fs::write(path, toml::to_string(value).unwrap()).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn actual_engine_opaque_and_literal_markers_render_exact_digest() {
    for traffic in [true, false] {
        let run = engine(traffic);
        let (_, _, _, value) = config(&run, false);
        let supervisor = run.engine.supervisor.lock().unwrap();
        let token = supervisor.run_token();
        if traffic {
            assert_eq!(token.len(), 64);
            assert!(!token.starts_with(&format!("{}:", run.engine.run_id)));
        } else {
            assert!(token.starts_with(&format!("{}:", run.engine.run_id)));
        }
        assert_eq!(
            value["ark"]["native_audit"]["run_token_hash"],
            hex::encode(Sha256::digest(token.as_bytes()))
        );
    }
}

#[test]
fn run_token_hash_is_not_an_operator_profile_field() {
    let (agent, _) = profile();
    let mut value = serde_json::to_value(agent.native_audit.unwrap()).unwrap();
    value["run_token_hash"] = "aa".repeat(32).into();
    assert!(serde_json::from_value::<NativeAuditProfile>(value).is_err());
}

/// Build anchors are supplied by testing/native-joint/startup-gate.py AFTER
/// it compiles the actual same-source agent harness. No runtime path override.
// option_env + expect is deliberate fail-closed design: the anchors are only
// exported by the gate's same-source hooks, and their absence must panic when
// this ignored test is actually executed (proving a stale/unanchored binary),
// never at compile time - env! would break every build without the anchors.
#[test]
#[allow(clippy::option_env_unwrap)]
#[ignore = "explicit same-source agent harness required; run startup-gate.py"]
fn native_run_token_gate() {
    let harness =
        Path::new(option_env!("SPC_NATIVE_STARTUP_AGENT_HARNESS").expect("build anchor required"));
    let expected_hash =
        option_env!("SPC_NATIVE_STARTUP_AGENT_SHA256").expect("build hash required");
    assert_eq!(
        crate::supervisor::hash_file(harness).unwrap(),
        expected_hash
    );
    for (traffic, bad_digest) in [(true, false), (false, false), (true, true)] {
        let run = engine(traffic);
        let (agent, address, path, value) = config(&run, bad_digest);
        let receipt = path
            .parent()
            .unwrap()
            .join("state/native-bootstrap-probe.json");
        let mut spec = ProcessSpec::new(
            "wallet",
            "id-alice",
            harness,
            run.root.join("private/probe.log"),
        );
        spec.args = vec![
            "wallet::native_audit::tests::startup_tests::native_bootstrap_probe".into(),
            "--ignored".into(),
            "--exact".into(),
            "--nocapture".into(),
            "--".into(),
            "--config".into(),
            path.display().to_string(),
        ];
        spec.pipe_stdin = true;
        spec.readiness = Readiness::Immediate;
        // No SPC_LAB_RUN_TOKEN override: the real Supervisor exports its token.
        // A wrong-digest startup rejects before the supervisor's Immediate
        // readiness poll can return a handle, so the observed nonzero exit
        // surfaces as ExitedDuringReadiness; for bad_digest that IS the
        // expected fail-closed rejection (no receipt, instance or stdin).
        // Positive startups stay alive until released, so an early exit
        // there remains a loud failure.
        let launched = {
            let mut supervisor = run.engine.supervisor.lock().unwrap();
            supervisor.launch_with_pipes(spec)
        };
        let mut launched = match launched {
            Ok(launched) => launched,
            Err(crate::supervisor::SupervisorError::ExitedDuringReadiness { status, .. })
                if bad_digest =>
            {
                assert_ne!(status, "exit status: 0");
                assert!(!receipt.exists());
                assert!(!path
                    .parent()
                    .unwrap()
                    .join("state")
                    .join(spc_scan::native_activation::INSTANCE_FILENAME)
                    .exists());
                continue;
            }
            Err(other) => panic!("unexpected wallet probe launch error: {other:?}"),
        };
        run.engine
            .manifest
            .lock()
            .unwrap()
            .processes
            .push(launched.record.clone());
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut exit = None;
        while !receipt.exists() && Instant::now() < deadline {
            exit = run
                .engine
                .supervisor
                .lock()
                .unwrap()
                .try_wait_child(launched.record.pid)
                .unwrap();
            if exit.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        if bad_digest {
            assert!(exit.is_some_and(|status| !status.success()));
            assert!(!receipt.exists());
            assert!(!path
                .parent()
                .unwrap()
                .join("state")
                .join(spc_scan::native_activation::INSTANCE_FILENAME)
                .exists());
            continue;
        }
        assert!(
            receipt.exists(),
            "agent failed before native publication; see private probe.log"
        );
        let instance =
            spc_scan::native_activation::decode_instance(&std::fs::read(&receipt).unwrap())
                .unwrap();
        assert_eq!(instance.process.pid, launched.record.pid);
        assert_eq!(
            hex::encode(instance.process.run_token_sha256),
            value["ark"]["native_audit"]["run_token_hash"]
        );
        assert_eq!(instance.config_path, path.to_str().unwrap());
        validate_persisted(&run.engine, &agent, &address, &path).unwrap();
        println!(
            "native-startup-case {}",
            serde_json::json!({
            "scope":"actual prepared-engine renderer/Supervisor/loaded-config/native-service; fixture infrastructure",
            "traffic":traffic,"published_instance":instance,
            "source_id":option_env!("SPC_BUILD_SOURCE_ID").expect("same-source build anchor"),
            "agent_harness_sha256":expected_hash})
        );
        launched.stdin.take().unwrap().write_all(b"x").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = run
                .engine
                .supervisor
                .lock()
                .unwrap()
                .try_wait_child(launched.record.pid)
                .unwrap()
            {
                assert!(status.success());
                break;
            }
            assert!(Instant::now() < deadline, "agent probe did not retire");
            std::thread::sleep(Duration::from_millis(10));
        }
        // Same actual recorded marker, but a persisted run/hash/identity/key
        // contradiction or missing expected field must reject before rewrite.
        for field in [
            "run_id",
            "run_token_hash",
            "identity_id",
            "front_id",
            "pool_id",
            "pool_pubkey",
        ] {
            let mut changed = value.clone();
            changed["ark"]["native_audit"][field] = "foreign".into();
            write(&path, &changed);
            assert!(
                validate_persisted(&run.engine, &agent, &address, &path).is_err(),
                "{field}"
            );
            assert!(validate_before_write(&run.engine, &agent, &address, &path).is_err());
        }
        let mut missing = value.clone();
        missing["ark"]["native_audit"]
            .as_object_mut()
            .unwrap()
            .remove("run_token_hash");
        write(&path, &missing);
        assert!(validate_persisted(&run.engine, &agent, &address, &path).is_err());
        write(&path, &value);
        {
            let mut manifest = run.engine.manifest.lock().unwrap();
            let records = std::mem::take(&mut manifest.processes);
            drop(manifest);
            assert!(validate_persisted(&run.engine, &agent, &address, &path).is_err());
            run.engine.manifest.lock().unwrap().processes = records;
        }
        if traffic {
            // Reopening an exited instance with foreign token/config provenance
            // must fail before replacing that instance or publishing a socket.
            let state = path.parent().unwrap().join("state");
            let instance_path = state.join(spc_scan::native_activation::INSTANCE_FILENAME);
            for token_mismatch in [true, false] {
                let mut stale = instance.clone();
                if token_mismatch {
                    stale.process.run_token_sha256 = [91; 32];
                } else {
                    stale.process.config_sha256 = [92; 32];
                }
                let bytes = serde_json::to_vec(&stale).unwrap();
                std::fs::write(&instance_path, &bytes).unwrap();
                std::fs::set_permissions(&instance_path, std::fs::Permissions::from_mode(0o600))
                    .unwrap();
                if receipt.exists() {
                    std::fs::remove_file(&receipt).unwrap();
                }
                let mut spec = ProcessSpec::new(
                    "wallet",
                    "id-alice",
                    harness,
                    run.root.join("private/probe.log"),
                );
                spec.args = vec![
                    "wallet::native_audit::tests::startup_tests::native_bootstrap_probe".into(),
                    "--ignored".into(),
                    "--exact".into(),
                    "--nocapture".into(),
                    "--".into(),
                    "--config".into(),
                    path.display().to_string(),
                ];
                // A fast rejection exit races the supervisor's Immediate
                // readiness check inside launch(): try_wait is polled first,
                // so the observed nonzero exit can surface as
                // ExitedDuringReadiness instead of a child handle. Both
                // outcomes are the same fail-closed rejection; everything
                // below still asserts no publication and no instance rewrite.
                // The guard is dropped before matching so the Ok arm may
                // re-lock the supervisor for try_wait_child.
                let launched = {
                    let mut supervisor = run.engine.supervisor.lock().unwrap();
                    supervisor.launch(spec)
                };
                match launched {
                    Ok(record) => {
                        let deadline = Instant::now() + Duration::from_secs(5);
                        loop {
                            if let Some(status) = run
                                .engine
                                .supervisor
                                .lock()
                                .unwrap()
                                .try_wait_child(record.pid)
                                .unwrap()
                            {
                                assert!(!status.success());
                                break;
                            }
                            assert!(Instant::now() < deadline, "stale instance was not rejected");
                            std::thread::sleep(Duration::from_millis(10));
                        }
                    }
                    Err(crate::supervisor::SupervisorError::ExitedDuringReadiness {
                        status,
                        ..
                    }) => {
                        assert_ne!(status, "exit status: 0");
                    }
                    Err(other) => panic!("unexpected stale-instance launch error: {other:?}"),
                }
                assert!(!receipt.exists());
                assert_eq!(std::fs::read(&instance_path).unwrap(), bytes);
                assert!(!state
                    .join(spc_scan::native_activation::SOCKET_FILENAME)
                    .exists());
            }
        }
    }
}
