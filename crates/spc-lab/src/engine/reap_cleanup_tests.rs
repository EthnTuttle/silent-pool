//! Cleanup-purpose traffic binding validation (`attach_for_cleanup` /
//! `validate_persisted_traffic_bindings`): the persisted-only checks that
//! let `reap` reopen a stopped traffic run. Manifest state is constructed
//! directly here; the capability-record lineage gate is exercised by the
//! projection tests and the live adopt path, which cleanup deliberately
//! skips (a stopped run's capability record is unchanged on disk).
use super::*;
use crate::supervisor::{ControlTransport, ProcessRecord, ProcessState, Readiness};

fn traffic_engine() -> Arc<RunEngine> {
    let mut s: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/scenario_v1_one_pool_two_miners.json"
    ))
    .unwrap();
    s["run"]["id"] = serde_json::json!(CommandId::generate().to_string());
    s["expanded_access"] = serde_json::json!(true);
    s["traffic"] = serde_json::json!({"version":1,"required_capability":"spc.node.regtest.mempool_rpc.v1","protocol":{"control":"spc-lab-tx-control/v1","history_version":3},
        "bootstrap":{"enabled":true,"mining_mode":"native","funding_blocks":101,"distribution_outputs_per_wallet":8,"excluded_from_statistics":true},
        "driver":{"actors":[{"id":"txd-0","network_wallets":8,"user_payment_wallets":2,"hosts_bootstrap_funding":true}],"user_payment_wallets":2,"history":"traffic/"},
        "rate":{"ordinary_tx_per_sec":0.2,"pacing":"fixed_jitter"},"fees":{"rate_sats_vb":{"min":1,"max":20},"max_fee_sats_per_tx":100000},"sizes":{"classes_vb":[{"vb":220,"mix":1.0}],"max_vb":1000},
        "spend_types":["p2pkh"],"ark":{"enabled":false,"registration_before_mining":true,"chain_effect_budget_tx_per_sec":0.0},"caps":{"max_inflight_broadcasts":1,"max_unknown_retries":3,"max_mempool_backlog_tx":1000,"reorg_resubmit_max":3},"pause_semantics":"no_broadcast_after_ack"});
    s["phases"] = serde_json::json!([{"id":"bootstrap","commands":[{"kind":{"type":"bootstrap_fund","driver_id":"txd-0","node_id":s["nodes"][0]["id"]}}]}]);
    let scenario: Scenario = serde_json::from_value(s).unwrap();
    let adapter: crate::config::AdapterConfig = serde_json::from_value(serde_json::json!({"program":"/bin/true","args":[],"fixture":true,"capabilities":["fixture.scripted","spc.node.regtest.mempool_rpc.v1","spc.pool.work_unit_v2","spc.pool.network_block_work"],"effective_profile":{
        "implementation":"fixture-node","network":"regtest","consensus_profile_id":"fixture.regtest.retarget","profile_fingerprint":null,"pow_limit_bits":"207fffff","retarget_floor_bits":"1e7fffff","initial_target_bits":"1e7fffff","regtest_pow_bits_overlay":"1e7fffff","target_spacing_secs":10,"target_timespan_secs":240,"retarget_interval_blocks":24,"no_pow_retargeting":false,"allow_min_difficulty_blocks":false,"minimum_difficulty_time_gap_secs":null,"subsidy_initial_sats":"5000000000","subsidy_halving_interval_blocks":150,"coinbase_maturity_blocks":100,"witness_commitment_supported":true,"tdp_session_cap":8},"rpc_auth":null,"env":{}})).unwrap();
    let mut config = LabConfig::default();
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
    let root =
        PathBuf::from("/tmp/opencode").join(format!("reap-cleanup-{}", CommandId::generate()));
    std::fs::create_dir(&root).unwrap();
    RunEngine::prepare(
        scenario,
        EngineOptions {
            run_dir: root.clone(),
            config,
            base_revision: "reap-cleanup-test".into(),
            dirty_tree_hash: "fixture".into(),
            owner_instance: "reap-cleanup-test".into(),
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
    .unwrap()
}

/// A peered launch profile for node-a, as a host-ring launch would record.
fn peered_profile() -> crate::traffic::launch_profile::LaunchProfile {
    let argv: Vec<String> = [
        "--network",
        "regtest",
        "--datadir",
        "/private/probe/datadir",
        "--rpc",
        "--rpc-listen",
        "127.0.0.1:19443",
        "--rpc-cookie-file",
        "/private/probe/rpc.cookie",
        "--regtest-pow-bits",
        "207fffff",
        "--no-seeds",
        "--no-discover",
        "--max-outbound",
        "1",
        "--listen",
        "127.0.0.1:20001",
        "--connect",
        "127.0.0.1:20002",
        "--api-log",
        "/private/probe/api.jsonl",
        "--log-level",
        "error",
    ]
    .iter()
    .map(|argument| argument.to_string())
    .collect();
    crate::traffic::launch_profile::LaunchProfile::capture(
        crate::supervisor::hash_file(Path::new("/bin/true")).unwrap(),
        argv,
        BTreeMap::new(),
    )
    .unwrap()
}

/// A persisted process record matching the profile. The PID is impossible
/// on purpose: cleanup validation must not touch live processes.
fn node_record(profile: &crate::traffic::launch_profile::LaunchProfile) -> ProcessRecord {
    ProcessRecord {
        role: "node".into(),
        entity: "node-a".into(),
        pid: u32::MAX,
        pgid: u32::MAX,
        start_time_ticks: 1,
        executable_path: PathBuf::from("/bin/true"),
        executable_sha256: profile.binary_sha256.clone(),
        run_token: "reap-cleanup-test".into(),
        args: profile.argv.clone(),
        log_path: PathBuf::from("/tmp/opencode/reap-cleanup-test.log"),
        readiness: Readiness::Immediate,
        state: ProcessState::Stopped,
        adopted: false,
        control: ControlTransport::None,
    }
}

/// Write a driver config file and record its mapping; returns the path.
fn install_driver_config(engine: &RunEngine, contents: &str) -> PathBuf {
    let path = engine.run_dir.join("driver-config.json");
    std::fs::write(&path, contents).unwrap();
    let hash = crate::supervisor::hash_file(&path).unwrap();
    let mut manifest = engine.manifest.lock().unwrap();
    manifest.traffic_driver_paths.insert(
        "txd-0".into(),
        serde_json::json!({"config_path": path, "config_sha256": hash}),
    );
    manifest.save(&engine.manifest_path).unwrap();
    path
}

#[test]
fn persisted_bindings_accept_a_stopped_run() {
    let engine = traffic_engine();
    let profile = peered_profile();
    let record = node_record(&profile);
    install_driver_config(&engine, "{}");
    {
        let mut manifest = engine.manifest.lock().unwrap();
        manifest
            .traffic_launch_profiles
            .insert(NodeId::trusted("node-a"), profile);
        manifest.upsert_process(record);
        manifest.save(&engine.manifest_path).unwrap();
    }
    engine.validate_persisted_traffic_bindings().unwrap();
}

#[test]
fn live_reverify_refuses_what_cleanup_validation_accepts() {
    // The AP5 reap blocker, pinned: a stopped traffic run's nodes are no
    // longer Owned, so the LIVE attach path (`reverify_node_record`, the
    // live-process half of `reverify_traffic_binding`) refuses the run -
    // while the cleanup path (persisted bindings only) accepts exactly the
    // same on-disk state and can reap it.
    let engine = traffic_engine();
    let profile = peered_profile();
    let record = node_record(&profile);
    install_driver_config(&engine, "{}");
    {
        let mut manifest = engine.manifest.lock().unwrap();
        manifest
            .traffic_launch_profiles
            .insert(NodeId::trusted("node-a"), profile.clone());
        manifest.upsert_process(record.clone());
        manifest.save(&engine.manifest_path).unwrap();
    }
    let node = engine.scenario.nodes[0].clone();
    let live = engine
        .reverify_node_record(&node, &profile, &record)
        .unwrap_err();
    assert_eq!(live.code(), "traffic_launch_profile_mismatch");
    assert!(
        live.to_string().contains("ownership/argv/binary changed"),
        "{live}"
    );
    engine.validate_persisted_traffic_bindings().unwrap();
}

#[test]
fn persisted_bindings_reject_changed_argv() {
    let engine = traffic_engine();
    let profile = peered_profile();
    let mut record = node_record(&profile);
    record.args.push("--extra".into());
    {
        let mut manifest = engine.manifest.lock().unwrap();
        manifest
            .traffic_launch_profiles
            .insert(NodeId::trusted("node-a"), profile);
        manifest.upsert_process(record);
        manifest.save(&engine.manifest_path).unwrap();
    }
    let error = engine.validate_persisted_traffic_bindings().unwrap_err();
    assert_eq!(error.code(), "traffic_launch_profile_mismatch");
    assert!(error.to_string().contains("persisted node argv"));
}

#[test]
fn persisted_bindings_reject_changed_binary() {
    let engine = traffic_engine();
    let profile = peered_profile();
    let mut record = node_record(&profile);
    record.executable_sha256 = "0".repeat(64);
    {
        let mut manifest = engine.manifest.lock().unwrap();
        manifest
            .traffic_launch_profiles
            .insert(NodeId::trusted("node-a"), profile);
        manifest.upsert_process(record);
        manifest.save(&engine.manifest_path).unwrap();
    }
    let error = engine.validate_persisted_traffic_bindings().unwrap_err();
    assert_eq!(error.code(), "traffic_launch_profile_mismatch");
}

#[test]
fn missing_process_record_is_not_a_cleanup_failure() {
    // A node that was never recorded (or whose record was lost) is reported
    // by reap as expected-but-absent, not by the binding validation.
    let engine = traffic_engine();
    let profile = peered_profile();
    {
        let mut manifest = engine.manifest.lock().unwrap();
        manifest
            .traffic_launch_profiles
            .insert(NodeId::trusted("node-a"), profile);
        manifest.save(&engine.manifest_path).unwrap();
    }
    engine.validate_persisted_traffic_bindings().unwrap();
    // And reap reports the expected-but-absent node as exited.
    let results = engine.reap().unwrap();
    assert_eq!(
        results,
        vec![(
            "node".to_string(),
            "node-a".to_string(),
            ProcessState::Exited
        )],
        "expected-but-absent nodes report as exited"
    );
}

#[test]
fn persisted_bindings_reject_missing_profile() {
    let engine = traffic_engine();
    install_driver_config(&engine, "{}");
    // Traffic scenario, but no launch profile was ever recorded.
    let error = engine.validate_persisted_traffic_bindings().unwrap_err();
    assert_eq!(error.code(), "traffic_launch_profile_mismatch");
    assert!(error.to_string().contains("missing recorded profile"));
}

#[test]
fn persisted_bindings_reject_changed_driver_config() {
    let engine = traffic_engine();
    let profile = peered_profile();
    let record = node_record(&profile);
    let path = install_driver_config(&engine, "{}");
    {
        let mut manifest = engine.manifest.lock().unwrap();
        manifest
            .traffic_launch_profiles
            .insert(NodeId::trusted("node-a"), profile);
        manifest.upsert_process(record);
        manifest.save(&engine.manifest_path).unwrap();
    }
    engine.validate_persisted_traffic_bindings().unwrap();
    std::fs::write(&path, "{\"changed\":true}").unwrap();
    let error = engine.validate_persisted_traffic_bindings().unwrap_err();
    assert_eq!(error.code(), "traffic_config_binding");
    assert!(error.to_string().contains("actor config changed"));
}
