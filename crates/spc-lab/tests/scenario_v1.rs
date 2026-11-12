mod common;

use serde_json::Value;
use spc_lab::scenario::Scenario;

fn fixture_value(name: &str) -> Value {
    serde_json::from_str(&common::fixture_scenario_text(name)).unwrap()
}

fn parse(value: Value) -> Result<Scenario, spc_lab::scenario::ScenarioError> {
    Scenario::from_json(&value.to_string())
}

#[test]
fn scenario_v1_entities_parse_and_validate() {
    let single = common::fixture_scenario("scenario_v1_one_pool_two_miners.json");
    let report = single.validate().expect("single fixture validates");
    assert_eq!(single.scenario_version, 1);
    assert_eq!(single.nodes.len(), 1);
    assert_eq!(single.pools.len(), 1);
    assert_eq!(single.miners.len(), 2);
    assert_eq!(single.identities.len(), 2);
    assert!(report.required_capabilities.contains("fixture.scripted"));

    let multi = common::fixture_scenario("scenario_v1_two_pools.json");
    multi.validate().expect("multi fixture validates");
    assert_eq!(multi.nodes.len(), 2);
    assert_eq!(multi.pools.len(), 2);
    assert_eq!(multi.miners.len(), 2);
    assert_eq!(multi.resources.redis_instances.len(), 2);
    assert_eq!(multi.resources.postgres_databases.len(), 2);
}

#[test]
fn unknown_version_is_rejected_before_any_process() {
    let mut value = fixture_value("scenario_v1_one_pool_two_miners.json");
    value["scenario_version"] = serde_json::json!(2);
    let error = parse(value).unwrap_err();
    assert_eq!(error.code(), "scenario_version_unsupported");
}

#[test]
fn unknown_top_level_field_is_rejected() {
    let mut value = fixture_value("scenario_v1_one_pool_two_miners.json");
    value["typo_work_profile"] = serde_json::json!(true);
    let error = parse(value).unwrap_err();
    assert_eq!(error.code(), "scenario_schema_invalid");
}

#[test]
fn relationship_errors_name_the_entity() {
    let mut value = fixture_value("scenario_v1_one_pool_two_miners.json");
    value["miners"][0]["workers"][0]["identity_id"] = serde_json::json!("id-nobody");
    let scenario = parse(value).unwrap();
    let error = scenario.validate().unwrap_err();
    assert_eq!(error.code(), "worker_identity_unknown");
    assert!(error.detail().contains("w-alice-1"));
}

#[test]
fn duplicate_entity_and_listener_collisions_are_rejected() {
    let mut value = fixture_value("scenario_v1_one_pool_two_miners.json");
    let node = value["nodes"][0].clone();
    value["nodes"].as_array_mut().unwrap().push(node);
    let scenario = parse(value).unwrap();
    assert_eq!(scenario.validate().unwrap_err().code(), "duplicate_node_id");

    let mut value = fixture_value("scenario_v1_one_pool_two_miners.json");
    value["fronts"][0]["listen"]["sv2"] = serde_json::json!("127.0.0.1:12345");
    value["nodes"][0]["rpc"]["listen"] = serde_json::json!("127.0.0.1:12345");
    let scenario = parse(value).unwrap();
    assert_eq!(
        scenario.validate().unwrap_err().code(),
        "listener_port_collision"
    );
}

#[test]
fn pool_storage_cannot_be_shared_or_borrowed() {
    let mut value = fixture_value("scenario_v1_two_pools.json");
    value["pools"][1]["storage"]["redis_instance_id"] = serde_json::json!("redis-pool-a");
    value["pools"][1]["storage"]["postgres_database_id"] = serde_json::json!("pg-pool-b");
    let scenario = parse(value).unwrap();
    assert_eq!(
        scenario.validate().unwrap_err().code(),
        "pool_redis_owner_mismatch"
    );
}

#[test]
fn work_profile_family_and_pow_limit_fail_closed() {
    let mut value = fixture_value("scenario_v1_one_pool_two_miners.json");
    value["pools"][0]["work_profile"]["share_work_multiplier"] = serde_json::json!(3);
    let scenario = parse(value).unwrap();
    assert_eq!(
        scenario.validate().unwrap_err().code(),
        "multiplier_not_in_family"
    );

    let mut value = fixture_value("scenario_v1_one_pool_two_miners.json");
    value["pools"][0]["work_profile"]["reference_target"] =
        serde_json::json!("0x3fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff");
    let scenario = parse(value).unwrap();
    assert_eq!(
        scenario.validate().unwrap_err().code(),
        "reference_below_pow_limit"
    );

    let mut value = fixture_value("scenario_v1_one_pool_two_miners.json");
    value["pools"][0]["work_profile"]["reference_target"] =
        serde_json::json!("0xfffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe");
    let scenario = parse(value).unwrap();
    assert_eq!(
        scenario.validate().unwrap_err().code(),
        "reference_target_invalid"
    );

    let mut value = fixture_value("scenario_v1_one_pool_two_miners.json");
    value["pools"][0]["work_profile"]["window_rule"] = serde_json::json!("share_work_legacy");
    let scenario = parse(value).unwrap();
    assert_eq!(
        scenario.validate().unwrap_err().code(),
        "work_window_rule_mismatch"
    );
}

#[test]
fn implicit_links_and_overlapping_nonce_ranges_are_rejected() {
    let mut value = fixture_value("scenario_v1_one_pool_two_miners.json");
    value["links"].as_array_mut().unwrap().pop();
    let scenario = parse(value).unwrap();
    assert_eq!(scenario.validate().unwrap_err().code(), "link_missing");

    let mut value = fixture_value("scenario_v1_one_pool_two_miners.json");
    value["miners"][0]["workers"][0]["nonce_start"] = serde_json::json!(0);
    value["miners"][0]["workers"][0]["nonce_len"] = serde_json::json!(100);
    let worker = value["miners"][0]["workers"][0].clone();
    let mut second = worker.clone();
    second["id"] = serde_json::json!("w-alice-2");
    second["nonce_start"] = serde_json::json!(50);
    second["nonce_len"] = serde_json::json!(100);
    value["miners"][0]["workers"]
        .as_array_mut()
        .unwrap()
        .push(second);
    let scenario = parse(value).unwrap();
    assert_eq!(
        scenario.validate().unwrap_err().code(),
        "worker_ranges_overlap"
    );
}
