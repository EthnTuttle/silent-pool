use serde_json::{json, Value};
use spc_lab::traffic::launch_profile::{canonical_bytes, digest, LaunchProfile};
use std::collections::BTreeSet;

// Semantics and digest goldens from the frozen canonical producer r4:
// launch-profile-goldens.json 4613f329a2455807afc39e82487dbd7541798740e53d6a2d2807328e76d83c9a
// launch-profile-rpc-policy-goldens.json 7d87ad97d2937412a62f5ac4edd8d156a9ea02e600a3a6011ad469da569885ca
fn namespace() -> LaunchProfile {
    serde_json::from_value(json!({"profile_version":1,"binary_sha256":"ab".repeat(32),
        "argv":["--network","regtest","--datadir","/run/lab/private/node-a","--rpc","--rpc-listen","10.231.0.2:18443",
            "--rpc-cookie-file","/run/lab/private/a.cookie","--regtest-pow-bits","200fffff","--no-seeds","--no-discover","--max-outbound","1",
            "--api-log","/run/lab/private/a.jsonl","--log-level","info","--max-run-secs","300","--listen","10.231.0.2:18444","--connect","10.231.0.3:18444","--config","/policy/node.conf"],
        "env":{"SPC_LAB_RUN_TOKEN":"golden_public_fixture_token","RBITCOIN_RPC_PACKAGE_DIALECT":"1"},
        "config_files":[{"argv_index":26,"sha256":"97d0b33aa1795ae46b51aafafbcfb4e004d32576fd7061a149c424ce0d9e36be"}],
        "substitutions":[{"class":"datadir","argv_index":3},{"class":"rpc-listen","argv_index":6},{"class":"cookie-file","argv_index":8},
            {"class":"p2p-isolation","span":[11,15]},{"class":"api-log","span":[15,19]},{"class":"max-run-secs","argv_index":20},{"class":"p2p-isolation","span":[21,25]}]})).unwrap()
}
fn owned() -> BTreeSet<String> {
    ["10.231.0.2:18444", "10.231.0.3:18444", "10.231.0.2:18443"]
        .into_iter()
        .map(str::to_string)
        .collect()
}
fn normalize(p: &LaunchProfile) -> Result<Value, &'static str> {
    p.normalize(&owned(), |_| Ok(b"minrelaytxfee=0.00001000\n".to_vec()))
}

#[test]
fn canonical_producer_goldens_bind_default_split_equals_and_policy_change() {
    let profile = namespace();
    let default = normalize(&profile).unwrap();
    assert_eq!(
        digest(&default).unwrap(),
        "626ecf7558b9696a6ce1e6891eb0a06f22287fe1350a9804128bb8df24b31f89"
    );
    let variants: &[(&[&str], &str)] = &[
        (
            &["--rpc-compat-profile", "bark-v31"],
            "ce22700b1e71f41fcf8100825ff04b33a929870d3f84c47afd6d14c512974b96",
        ),
        (
            &["--rpc-compat-profile=bark-v31"],
            "2277d7cc0aa0e0f0715ad38954cbb0d5fe6d2c3f509bdb5a85b8e36c9764aa37",
        ),
    ];
    for (tokens, expected) in variants {
        let mut p = profile.clone();
        p.argv.extend(tokens.iter().map(|s| s.to_string()));
        let normalized = normalize(&p).unwrap();
        assert_ne!(normalized, default);
        assert_eq!(digest(&normalized).unwrap(), *expected);
        p.env
            .insert("RBITCOIN_RPC_PACKAGE_DIALECT".into(), "false".into());
        assert!(normalize(&p).is_err());
    }
    let mut changed = profile.clone();
    changed.argv[10] = "207fffff".into();
    assert_eq!(
        digest(&normalize(&changed).unwrap()).unwrap(),
        "ec11fee39c1ba97662f36c8621138248f02a1e2a37a764ebf8d0c32c2cc785f7"
    );
    let mut duplicate = profile;
    duplicate.argv.extend(["--datadir".into(), "/other".into()]);
    assert!(normalize(&duplicate).is_err());
    assert!(canonical_bytes(&json!({"money":0.00001})).is_err());
    assert!(canonical_bytes(&json!({"env":"bad\nvalue"})).is_err());
}

#[test]
fn missing_consensus_facts_and_profile_mismatch_never_recognize() {
    assert!(spc_lab::traffic::runtime::consensus_facts(
        &json!({"consensus":{"network":"regtest"}})
    )
    .is_err());
    let binding = spc_lab::traffic::runtime::NodeBinding {
        binary_sha256: "ab".repeat(32),
        genesis: "cd".repeat(32),
        comparison_sha256: "ef".repeat(32),
        consensus: json!({"network":"regtest"}),
    };
    let nodes = [(spc_lab::ids::NodeId::trusted("node"), binding)]
        .into_iter()
        .collect();
    assert!(spc_lab::traffic::runtime::bind_report(
        &json!({"node":{"binary_sha256":"ab".repeat(32)}}),
        &nodes
    )
    .is_err());
}

#[test]
fn stage_table_keeps_mutations_closed_during_bootstrap() {
    use spc_lab::command::CommandKind;
    use spc_lab::traffic::runtime::{admitted, TrafficStage};
    let driver = spc_lab::ids::ActorId::trusted("txd");
    let bootstrap = CommandKind::BootstrapFund {
        driver_id: driver.clone(),
        node_id: spc_lab::ids::NodeId::trusted("node"),
    };
    assert!(admitted(TrafficStage::Provisioned, &bootstrap).is_ok());
    assert_eq!(
        admitted(TrafficStage::Bootstrapping, &bootstrap),
        Err("traffic_bootstrap_duplicate")
    );
    let pause = CommandKind::PauseTxDriver { driver_id: driver };
    assert!(admitted(TrafficStage::Provisioned, &pause).is_err());
    assert!(admitted(TrafficStage::Active, &pause).is_ok());
    assert!(admitted(
        TrafficStage::Bootstrapping,
        &CommandKind::StopRun {
            reason: "cancel".into()
        }
    )
    .is_ok());
}
