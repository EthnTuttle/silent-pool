//! Host-mode node P2P wiring.
//!
//! A host run (no namespace topology) whose nodes were rendered in the full
//! isolated listener shape is peered in a ring over the leased loopback P2P
//! ports; namespace wiring through published addresses is unchanged; caller
//! P2P flags and ambiguous shapes fail closed before any process is
//! launched.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use spc_lab::config::{AdapterConfig, LabConfig};
use spc_lab::engine::RunEngine;
use spc_lab::scenario::Scenario;
use spc_lab::traffic::launch_profile::LaunchProfile;

use common::{
    fixture_bin, fixture_config, fixture_scenario, fixture_scenario_text, prepare_engine_with,
    prepare_engine_with_addresses, TestDir,
};

/// The traffic node adapter shape from `build_lab_config`: the full isolated
/// listener flags on a fixture binary. The fixture's `hold` mode only reads
/// `--port`/`--height-file`/`--ready-file`, so the rbitcoin-shaped arguments
/// pass through to the rendered argv exactly as a real node's would.
fn isolated_node_config() -> LabConfig {
    let mut config = fixture_config();
    let adapter = config.nodes.get_mut("fixture-node").unwrap();
    adapter.args = vec![
        "node".into(),
        "--port".into(),
        "{rpc_port}".into(),
        "--height-file".into(),
        "{height_file}".into(),
        "--network".into(),
        "regtest".into(),
        "--datadir".into(),
        "{run_dir}/private/node-{entity}".into(),
        "--rpc".into(),
        "--rpc-listen".into(),
        "{rpc_addr}:{rpc_port}".into(),
        "--rpc-cookie-file".into(),
        "{run_dir}/private/node-rpc-{entity}.cookie".into(),
        "--regtest-pow-bits".into(),
        "207fffff".into(),
        "--no-listen".into(),
        "--no-seeds".into(),
        "--no-discover".into(),
        "--max-outbound".into(),
        "1".into(),
        "--api-log".into(),
        "{run_dir}/private/node-api-{entity}.jsonl".into(),
        "--log-level".into(),
        "error".into(),
    ];
    config
}

/// A node adapter with no P2P flags at all: same capabilities and effective
/// profile as the fixture node (preflight requires them), bare argv.
fn bare_node_adapter() -> AdapterConfig {
    let mut adapter = AdapterConfig {
        program: fixture_bin(),
        args: vec![
            "node".into(),
            "--port".into(),
            "{rpc_port}".into(),
            "--height-file".into(),
            "{height_file}".into(),
        ],
        capabilities: vec!["fixture.scripted".into()],
        fixture: true,
        effective_profile: None,
        rpc_auth: None,
        env: BTreeMap::new(),
    };
    let reference = fixture_config();
    let reference = reference.nodes.get("fixture-node").unwrap();
    adapter.capabilities = reference.capabilities.clone();
    adapter.effective_profile = reference.effective_profile.clone();
    adapter
}

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.windows(2)
        .find(|window| window[0] == flag)
        .map(|window| window[1].clone())
}

fn node_args<'a>(manifest: &'a spc_lab::manifest::RunManifest, id: &str) -> &'a Vec<String> {
    &manifest
        .process("node", id)
        .unwrap_or_else(|| panic!("node {id} process record missing"))
        .args
}

fn p2p_port(manifest: &spc_lab::manifest::RunManifest, id: &str) -> u16 {
    manifest.resources.listeners[&format!("node.{id}.p2p")].port
}

fn start_fails(engine: &RunEngine) -> spc_lab::error::EngineError {
    match engine.start() {
        Ok(()) => panic!("start must fail closed"),
        Err(error) => error,
    }
}

/// The pre-launch node validation must leave zero NODE effects. Storage
/// roles (redis/postgres) launch before the nodes and are stopped by the
/// failing start; only node records would prove the gate fired too late.
fn assert_no_node_effects(engine: &RunEngine) {
    let manifest = engine.manifest_snapshot();
    assert!(
        manifest
            .processes
            .iter()
            .all(|record| record.role != "node"),
        "node processes must not be recorded: {:?}",
        manifest
            .processes
            .iter()
            .map(|record| (record.role.as_str(), record.entity.as_str()))
            .collect::<Vec<_>>()
    );
    assert!(manifest.traffic_launch_profiles.is_empty());
}

#[test]
fn host_two_nodes_peer_in_a_ring() {
    let dir = TestDir::new("p2p-ring-two");
    let engine = prepare_engine_with(
        &dir,
        fixture_scenario("scenario_v1_two_pools.json"),
        isolated_node_config(),
    );
    engine.start().unwrap();
    let manifest = engine.manifest_snapshot();
    engine.stop().unwrap();
    let port_a = p2p_port(&manifest, "node-a");
    let port_b = p2p_port(&manifest, "node-b");
    // IDs and ports map uniquely: never the loopback IP alone.
    assert_ne!(port_a, port_b);
    for (id, peer) in [("node-a", "node-b"), ("node-b", "node-a")] {
        let args = node_args(&manifest, id);
        assert!(
            !args.contains(&"--no-listen".to_string()),
            "{id} kept --no-listen: {args:?}"
        );
        assert_eq!(
            flag_value(args, "--listen"),
            Some(format!(
                "127.0.0.1:{}",
                if id == "node-a" { port_a } else { port_b }
            )),
            "{id} must listen on its own leased port: {args:?}"
        );
        assert_eq!(
            flag_value(args, "--connect"),
            Some(format!(
                "127.0.0.1:{}",
                if peer == "node-a" { port_a } else { port_b }
            )),
            "{id} must dial its ring successor: {args:?}"
        );
        // The isolation flags stay: the peered profile shape is
        // --no-seeds/--no-discover/--max-outbound 1 plus the two endpoints.
        for retained in ["--no-seeds", "--no-discover"] {
            assert!(args.contains(&retained.to_string()), "{id} argv: {args:?}");
        }
        assert_eq!(flag_value(args, "--max-outbound"), Some("1".to_string()));
    }
}

#[test]
fn host_three_nodes_peer_in_scenario_order() {
    let dir = TestDir::new("p2p-ring-three");
    let mut value: serde_json::Value =
        serde_json::from_str(&fixture_scenario_text("scenario_v1_two_pools.json")).unwrap();
    value["run"]["id"] = serde_json::json!("run-fixture-ring-three");
    value["nodes"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "id": "node-c",
            "network_id": "net-multi",
            "implementation": "fixture-node",
            "p2p": {"listen": "127.0.0.1:0"},
            "rpc": {"listen": "127.0.0.1:0"},
            "tdp": {"listen": "127.0.0.1:0", "session_cap": 8}
        }));
    value["pools"][1]["node_ids"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!("node-c"));
    value["links"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "id": "link-pool-b-node-c",
            "from": {"kind": "pool", "id": "pool-b"},
            "to": {"kind": "node", "id": "node-c"},
            "policy": "normal"
        }));
    let scenario = Scenario::from_json(&value.to_string()).unwrap();
    let engine = prepare_engine_with(&dir, scenario, isolated_node_config());
    engine.start().unwrap();
    let manifest = engine.manifest_snapshot();
    engine.stop().unwrap();
    let ports: BTreeMap<&str, u16> = ["node-a", "node-b", "node-c"]
        .iter()
        .map(|id| (*id, p2p_port(&manifest, id)))
        .collect();
    // Distinct ports per node.
    assert_eq!(
        ports.values().collect::<BTreeSet<_>>().len(),
        3,
        "leased ports must be unique: {ports:?}"
    );
    // Ring successor in scenario order: a -> b -> c -> a.
    for (id, successor) in [
        ("node-a", "node-b"),
        ("node-b", "node-c"),
        ("node-c", "node-a"),
    ] {
        let args = node_args(&manifest, id);
        assert_eq!(
            flag_value(args, "--listen"),
            Some(format!("127.0.0.1:{}", ports[id])),
            "{id} listen: {args:?}"
        );
        assert_eq!(
            flag_value(args, "--connect"),
            Some(format!("127.0.0.1:{}", ports[successor])),
            "{id} connect: {args:?}"
        );
    }
}

#[test]
fn single_node_stays_isolated() {
    let dir = TestDir::new("p2p-single");
    let engine = prepare_engine_with(
        &dir,
        fixture_scenario("scenario_v1_one_pool_two_miners.json"),
        isolated_node_config(),
    );
    engine.start().unwrap();
    let manifest = engine.manifest_snapshot();
    engine.stop().unwrap();
    let args = node_args(&manifest, "node-a");
    assert!(args.contains(&"--no-listen".to_string()), "{args:?}");
    assert!(flag_value(args, "--listen").is_none(), "{args:?}");
    assert!(flag_value(args, "--connect").is_none(), "{args:?}");
}

#[test]
fn namespace_topology_wiring_is_unchanged() {
    let dir = TestDir::new("p2p-namespace");
    let mut addresses = BTreeMap::new();
    addresses.insert("node.node-a.p2p_addr".to_string(), "10.231.0.2".to_string());
    addresses.insert(
        "node.node-a.p2p_connect_addr".to_string(),
        "10.231.0.3".to_string(),
    );
    addresses.insert("node.node-b.p2p_addr".to_string(), "10.231.0.3".to_string());
    addresses.insert(
        "node.node-b.p2p_connect_addr".to_string(),
        "10.231.0.2".to_string(),
    );
    let engine = prepare_engine_with_addresses(
        &dir,
        fixture_scenario("scenario_v1_two_pools.json"),
        isolated_node_config(),
        addresses,
    );
    engine.start().unwrap();
    let manifest = engine.manifest_snapshot();
    engine.stop().unwrap();
    let port_a = p2p_port(&manifest, "node-a");
    let port_b = p2p_port(&manifest, "node-b");
    let a = node_args(&manifest, "node-a");
    let b = node_args(&manifest, "node-b");
    // Published-address wiring: listen on the node's own published P2P
    // address, connect to the peer's, exactly as before the host ring.
    assert_eq!(
        flag_value(a, "--listen"),
        Some(format!("10.231.0.2:{port_a}")),
        "{a:?}"
    );
    assert_eq!(
        flag_value(a, "--connect"),
        Some(format!("10.231.0.3:{port_b}")),
        "{a:?}"
    );
    assert_eq!(
        flag_value(b, "--listen"),
        Some(format!("10.231.0.3:{port_b}")),
        "{b:?}"
    );
    assert_eq!(
        flag_value(b, "--connect"),
        Some(format!("10.231.0.2:{port_a}")),
        "{b:?}"
    );
}

#[test]
fn caller_p2p_flags_fail_closed_before_launch() {
    let dir = TestDir::new("p2p-caller-flag");
    let mut scenario = fixture_scenario("scenario_v1_two_pools.json");
    scenario.nodes[0]
        .args
        .extend(["--listen".to_string(), "127.0.0.1:12345".to_string()]);
    let engine = prepare_engine_with(&dir, scenario, isolated_node_config());
    let error = start_fails(&engine);
    assert!(
        error.to_string().contains("node_p2p_argv_invalid"),
        "{error}"
    );
    // Zero node process, profile or manifest effects.
    assert_no_node_effects(&engine);
}

#[test]
fn caller_connect_flag_fails_closed_before_launch() {
    let dir = TestDir::new("p2p-caller-connect");
    let mut scenario = fixture_scenario("scenario_v1_two_pools.json");
    scenario.nodes[1]
        .args
        .extend(["--connect".to_string(), "127.0.0.1:12345".to_string()]);
    let engine = prepare_engine_with(&dir, scenario, isolated_node_config());
    let error = start_fails(&engine);
    assert!(
        error.to_string().contains("node_p2p_argv_invalid"),
        "{error}"
    );
    assert_no_node_effects(&engine);
}

#[test]
fn duplicate_isolation_flags_fail_closed_before_launch() {
    let dir = TestDir::new("p2p-duplicate-flag");
    let mut scenario = fixture_scenario("scenario_v1_two_pools.json");
    // The adapter already carries --no-seeds: one more is a duplicate.
    scenario.nodes[0].args.push("--no-seeds".to_string());
    let engine = prepare_engine_with(&dir, scenario, isolated_node_config());
    let error = start_fails(&engine);
    assert!(
        error.to_string().contains("node_p2p_argv_invalid"),
        "{error}"
    );
    assert_no_node_effects(&engine);
}

#[test]
fn mixed_shapes_fail_closed_before_launch() {
    let dir = TestDir::new("p2p-mixed-shape");
    let mut value: serde_json::Value =
        serde_json::from_str(&fixture_scenario_text("scenario_v1_two_pools.json")).unwrap();
    value["run"]["id"] = serde_json::json!("run-fixture-mixed-shape");
    value["nodes"][1]["implementation"] = serde_json::json!("fixture-node-bare");
    let scenario = Scenario::from_json(&value.to_string()).unwrap();
    let mut config = isolated_node_config();
    config
        .nodes
        .insert("fixture-node-bare".to_string(), bare_node_adapter());
    let engine = prepare_engine_with(&dir, scenario, config);
    let error = start_fails(&engine);
    assert!(
        error.to_string().contains("node_p2p_plan_invalid"),
        "{error}"
    );
    assert_no_node_effects(&engine);
}

#[test]
fn ring_argv_is_a_normalizeable_peered_profile() {
    // The argv the host ring produces (isolated flags minus --no-listen,
    // plus --listen/--connect over leased loopback ports) must pass the
    // frozen launch-profile grammar as the peered shape with owned
    // endpoints, exactly like namespace wiring.
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
    // The engine stamps the supervisor run token into every traffic node's
    // env before capture; normalize requires it.
    let mut env = BTreeMap::new();
    env.insert(
        "SPC_LAB_RUN_TOKEN".to_string(),
        "ring-profile-token-0123456789abcdef".to_string(),
    );
    let profile = LaunchProfile::capture("ab".repeat(32), argv, env).unwrap();
    let owned: BTreeSet<String> = ["127.0.0.1:19443", "127.0.0.1:20001", "127.0.0.1:20002"]
        .iter()
        .map(|endpoint| endpoint.to_string())
        .collect();
    profile
        .normalize(&owned, |_path| Err("no config file expected".into()))
        .expect("ring argv must normalize as the peered profile shape");
}
