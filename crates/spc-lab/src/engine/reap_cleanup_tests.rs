//! Cleanup-purpose traffic binding validation (`attach_for_cleanup` /
//! `validate_persisted_traffic_bindings`): the persisted-only checks that
//! let `reap` reopen a stopped traffic run. Manifest state is constructed
//! directly here. The capability-record lineage gate is exercised by the
//! projection tests and the live adopt path; cleanup verifies a record
//! that IS present exactly as strictly as Live (rewritten history still
//! rejects) and tolerates only the never-granted ABSENCE: a run whose
//! probe stopped before granting the capability (FAILED/INCOMPLETE
//! provisioning) has no record to verify, and that absence is not
//! permission to block resource cleanup.
use super::*;
use crate::supervisor::{ControlTransport, ProcessRecord, ProcessState, Readiness};
use crate::traffic::projection::TrafficCapabilityRecord;

/// Build a traffic engine over the one-pool fixture with `node_count`
/// nodes on net-a (extra nodes are copies of node-a with a fresh id and
/// no links; a link-less node is valid, which is exactly the shape a
/// partially provisioned run leaves behind).
fn traffic_engine_with_nodes(node_count: usize) -> Arc<RunEngine> {
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
    {
        let mut nodes = s["nodes"].as_array().unwrap().clone();
        while nodes.len() < node_count {
            let mut extra = nodes[0].clone();
            extra["id"] = serde_json::json!(format!("node-{}", (b'a' + nodes.len() as u8) as char));
            nodes.push(extra);
        }
        s["nodes"] = Value::Array(nodes);
    }
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

fn traffic_engine() -> Arc<RunEngine> {
    traffic_engine_with_nodes(1)
}

/// Two traffic nodes: node-a and node-b on net-a, node-b unlinked. A run
/// that stopped mid-provisioning leaves exactly this shape behind.
fn traffic_engine_two_nodes() -> Arc<RunEngine> {
    traffic_engine_with_nodes(2)
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
fn never_launched_nodes_are_not_a_cleanup_failure() {
    // The other half of the AP5 reap blocker: a traffic run whose nodes
    // were never launched (the probe stopped before bootstrap, before any
    // profile or process was recorded) has nothing to contradict. The
    // absence is not a persisted contradiction: validation accepts, and
    // reap reports the node as expected-not-launched.
    let engine = traffic_engine();
    engine.validate_persisted_traffic_bindings().unwrap();
    let results = engine.reap().unwrap();
    assert_eq!(
        results,
        vec![(
            "node".to_string(),
            "node-a".to_string(),
            ProcessState::Exited
        )],
        "never-launched nodes report as expected-not-launched"
    );
}

#[test]
fn persisted_bindings_reject_record_without_profile() {
    // The inverse absence is a contradiction: a recorded node process
    // whose launch profile is gone is rewritten history, and cleanup
    // fails closed on it.
    let engine = traffic_engine();
    let profile = peered_profile();
    let record = node_record(&profile);
    install_driver_config(&engine, "{}");
    {
        let mut manifest = engine.manifest.lock().unwrap();
        // Process record persisted, but no launch profile was recorded.
        manifest.upsert_process(record);
        manifest.save(&engine.manifest_path).unwrap();
    }
    let error = engine.validate_persisted_traffic_bindings().unwrap_err();
    assert_eq!(error.code(), "traffic_launch_profile_mismatch");
    assert!(error.to_string().contains("without recorded profile"));
}

#[test]
fn partial_provisioning_reaps_the_never_launched_node_too() {
    // INCOMPLETE provisioning: node-a was launched and stopped, node-b
    // was never reached. The persisted validation must not demand
    // node-b's launch profile (its absence is expected-not-launched, not
    // a contradiction), and reap must report node-b alongside node-a.
    let engine = traffic_engine_two_nodes();
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
    let run_dir = engine.run_dir.clone();
    drop(engine); // Release the owner lock.
    let reaper = RunEngine::attach_for_cleanup(&run_dir, "partial-provision-reap").unwrap();
    let results = reaper.reap().unwrap();
    assert_eq!(
        results,
        vec![
            (
                "node".to_string(),
                "node-a".to_string(),
                ProcessState::Exited
            ),
            (
                "node".to_string(),
                "node-b".to_string(),
                ProcessState::Exited
            ),
        ],
        "node-a reports from its record; node-b reports expected-not-launched"
    );
}

#[test]
fn cleanup_attaches_a_never_granted_capability_run() {
    // The AP5 r22 shared-run reap blocker, pinned end-to-end at the engine
    // boundary: the run stopped before the traffic capability was granted
    // (prepare state - no capability record, no launch profiles, no
    // processes, zero command bytes). Live attach refuses that manifest
    // before the owner lock; cleanup attach accepts it, reaps without
    // inventing a capability, a command or a process, and a second
    // cleanup behaves identically.
    let engine = traffic_engine();
    let before = engine.manifest_snapshot();
    assert!(before.traffic_capability.is_none());
    assert_eq!(before.state, RunState::Created);
    assert!(before.processes.is_empty());
    let commands = engine.run_dir.join("private").join("commands.jsonl");
    let commands_before = std::fs::read(&commands).unwrap();
    let run_dir = engine.run_dir.clone();
    drop(engine); // Release the owner lock.

    // The live gate stays strict: no capability record, no adopt.
    let live = RunEngine::attach(&run_dir, "ap5-blocker-live").unwrap_err();
    assert_eq!(live.code(), "traffic_capability_unrecorded");

    // Cleanup accepts the never-granted run and reaps it.
    let reaper = RunEngine::attach_for_cleanup(&run_dir, "ap5-blocker-reap").unwrap();
    let results = reaper.reap().unwrap();
    assert_eq!(
        results,
        vec![(
            "node".to_string(),
            "node-a".to_string(),
            ProcessState::Exited
        )],
        "the never-launched node reports expected-not-launched"
    );
    let after = reaper.manifest_snapshot();
    assert!(
        after.traffic_capability.is_none(),
        "cleanup must not invent a capability record"
    );
    assert_eq!(after.state, RunState::Created);
    assert!(after.processes.is_empty());
    drop(reaper); // Release the owner lock again.

    // Cleanup logged no command: the command log is byte-identical.
    assert_eq!(std::fs::read(&commands).unwrap(), commands_before);

    // Idempotent: a second cleanup reports the same thing.
    let again = RunEngine::attach_for_cleanup(&run_dir, "ap5-blocker-reap-2").unwrap();
    assert_eq!(
        again.reap().unwrap(),
        vec![(
            "node".to_string(),
            "node-a".to_string(),
            ProcessState::Exited
        )]
    );
}

#[test]
fn reap_never_signals_a_process_it_does_not_own() {
    // Safe PID identity: a live foreign process recorded with a stale
    // start-time tick fails verification as PidReused. Cleanup reports
    // the node as OrphanUnverified without signalling the foreign
    // process, which survives untouched.
    let engine = traffic_engine();
    let profile = peered_profile();
    let mut record = node_record(&profile);
    let mut foreign = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    record.pid = foreign.id();
    record.pgid = foreign.id();
    record.start_time_ticks = 1; // Never this process's real start time.
    record.state = ProcessState::Running; // Force the verify path.
    install_driver_config(&engine, "{}");
    {
        let mut manifest = engine.manifest.lock().unwrap();
        manifest
            .traffic_launch_profiles
            .insert(NodeId::trusted("node-a"), profile);
        manifest.upsert_process(record);
        manifest.save(&engine.manifest_path).unwrap();
    }
    let run_dir = engine.run_dir.clone();
    drop(engine); // Release the owner lock.
    let reaper = RunEngine::attach_for_cleanup(&run_dir, "foreign-pid-reap").unwrap();
    let results = reaper.reap().unwrap();
    assert_eq!(
        results,
        vec![(
            "node".to_string(),
            "node-a".to_string(),
            ProcessState::OrphanUnverified
        )],
        "a reused or foreign PID is never signalled"
    );
    assert!(
        foreign.try_wait().unwrap().is_none(),
        "the foreign process must survive cleanup untouched"
    );
    let _ = foreign.kill();
    let _ = foreign.wait();
}

#[test]
fn cleanup_rejects_a_present_but_invalid_capability_record() {
    // A capability record that IS present is verified exactly as strictly
    // as Live: cleanup refuses rewritten history even though it never
    // grants or exercises the capability itself.
    let engine = traffic_engine();
    let record: TrafficCapabilityRecord = serde_json::from_value(serde_json::json!({
        "schema_version": 0,
        "probe": "not-the-canonical-probe",
        "probe_version": "reap-cleanup-test",
        "report_schema_version": 1,
        "report_sha256": format!("{:0>64}", "a"),
        "contract_hash": format!("{:0>64}", "b"),
        "generated_utc": "2026-01-01T00:00:00Z",
        "run_token": "reap-cleanup-test",
        "target_label": "reap-cleanup-test",
        "target_kind": "owned-fresh-regtest-node",
        "profile_name": "reap-cleanup-test",
        "expected_network": "regtest",
        "expected_genesis": format!("{:0>64}", "c"),
        "node_binary_sha256": null,
        "traffic_consumer": null,
        "consumer_model_failures": [],
        "methods": {},
        "foundational": {},
        "check_states": {},
        "observations": {},
        "verdict": {
            "status": "unknown",
            "funded_state": "unknown",
            "full_acceptance": false
        },
        "recognized": false,
        "behavior_proven": false,
        "unproven_check_ids": []
    }))
    .unwrap();
    {
        let mut manifest = engine.manifest.lock().unwrap();
        manifest.traffic_capability = Some(record);
        manifest.save(&engine.manifest_path).unwrap();
    }
    let run_dir = engine.run_dir.clone();
    drop(engine); // Release the owner lock.
    let error = RunEngine::attach_for_cleanup(&run_dir, "invalid-capability-reap").unwrap_err();
    assert_eq!(error.code(), "traffic_capability_record_invalid");
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
