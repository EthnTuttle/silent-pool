use serde_json::json;
use spc_lab::ark_service::bind_info;
use spc_lab::resource::plan_resources;
use spc_lab::scenario::Scenario;

fn scenario() -> Scenario {
    Scenario::from_json(include_str!("../scenarios/lab-ark-infrastructure.json")).unwrap()
}

#[test]
fn ark_only_render_passes_the_real_pool_parser_without_front_or_database_secrets() {
    let ark = json!({"enabled":true,"server":"http://127.0.0.1:30000", "ark_id":"0f715baf",
        "float_target_sats":115000000u64,"max_order_sats":500000,
        "order_pubkey":"79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
        "server_is_self":true,"pool_output_account":"m/86'/1'/1'"});
    let node = toml::from_str("source = 'rpc'\nrpc_url = 'http://127.0.0.1:30001'\n").unwrap();
    let value = spc_lab::ark_service::pool_role_config(
        spc_lab::runtime::POOL_PAYOUT_DESCRIPTOR,
        node,
        "redis://ark:private@127.0.0.1:30002/0",
        ark,
    )
    .unwrap();
    let config: spc_pool::config::Config = value.clone().try_into().unwrap();
    config.validate().unwrap();
    assert!(value["storage"].get("postgres").is_none());
    assert!(value["pool"].get("authority_secret_key").is_none());
    assert_eq!(config.pool.roles, ["ark"]);
}

#[test]
fn stock_protobuf_metadata_binds_keys_and_rejects_policy_or_key_corruption() {
    let scenario = scenario();
    let profile = scenario.ark_backends[0].local.as_ref().unwrap();
    // SEC1 generator point and independently SHA256-derived ArkId.
    let info = json!({"network":"regtest", "vtxoLifetime":432,
        "maxVtxoExitDepth":100, "requiredBoardConfirmations":3,
        "roundIntervalSecs":10, "minBoardAmount":"20000",
        "serverPubkey":"Anm+Zn753LusVaBilc6HCwcCm/zbLc4o2VnygVsW+BeY",
        "mailboxPubkey":"Anm+Zn753LusVaBilc6HCwcCm/zbLc4o2VnygVsW+BeY",
        "fees":{"board":{"minFeeSat":"330","baseFeeSat":"100"}}});
    let bound = bind_info(&info, profile).unwrap();
    assert_eq!(bound["ark_id"], "0f715baf");
    assert_eq!(
        bound["server_pubkey"],
        "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
    );
    for (field, value) in [
        ("network", json!("bitcoin")),
        ("vtxoLifetime", json!(433)),
        ("minBoardAmount", json!(20000)),
        ("fees", json!(null)),
        (
            "serverPubkey",
            json!("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
        ),
        (
            "mailboxPubkey",
            json!("Anm+Zn753LusVaBilc6HCwcCm/zbLc4o2VnygVsW+Be="),
        ),
    ] {
        let mut corrupt = info.clone();
        corrupt[field] = value;
        assert!(bind_info(&corrupt, profile).is_err(), "accepted {field}");
    }
    let mut changed_fee = info;
    changed_fee["fees"]["board"]["baseFeeSat"] = json!("101");
    assert_ne!(
        bind_info(&changed_fee, profile).unwrap()["fees_sha256"],
        bound["fees_sha256"]
    );
}

#[test]
fn provider_storage_is_not_a_pool_database_and_all_ports_are_leased() {
    let scenario = scenario();
    scenario.validate().unwrap();
    let temporary = if std::path::Path::new("/tmp/opencode").is_dir() {
        std::path::PathBuf::from("/tmp/opencode")
    } else {
        std::env::temp_dir()
    };
    let root = temporary.join(format!("ark-resources-test-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let resources = plan_resources(&scenario, &scenario.run.id, &root, true).unwrap();
    assert_eq!(resources.postgres_databases.len(), 2);
    let provider = resources.listeners["ark.local-ark.postgres"].port;
    assert!(resources
        .postgres_databases
        .iter()
        .all(|p| p.port != provider));
    let mut ports = std::collections::BTreeSet::new();
    for endpoint in resources.listeners.values() {
        assert!(ports.insert(endpoint.port));
    }
    assert!(resources.listeners.contains_key("ark.local-ark.admin"));
    assert!(resources.listeners.contains_key("front.front-a1.operator"));
    assert!(spc_lab::resource::PortAllocator::new()
        .allocate(provider)
        .is_err());
    drop(resources);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn local_deployment_schema_refuses_hosted_endpoints_and_implicit_policy() {
    let mut scenario = scenario();
    scenario.ark_backends[0].listen = "ark.example:3535".into();
    assert!(scenario.validate().is_err());
    let mut value = serde_json::to_value(scenario).unwrap();
    value["ark_backends"][0]["local"]["deployment"] = json!("hosted");
    assert!(Scenario::from_json(&value.to_string()).is_err());
    value["ark_backends"][0]["local"]["deployment"] = json!("self_hosted");
    value["ark_backends"][0]["local"]
        .as_object_mut()
        .unwrap()
        .remove("vtxo_lifetime");
    assert!(Scenario::from_json(&value.to_string()).is_err());
}
