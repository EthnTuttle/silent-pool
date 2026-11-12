mod common;

use spc_lab::config::{preflight, LabConfig};

#[test]
fn missing_adapter_capability_and_profile_fail_before_launch() {
    let scenario = common::fixture_scenario("scenario_v1_one_pool_two_miners.json");

    let mut config = common::fixture_config();
    config.miners.remove("fixture-miner");
    let error = preflight(&scenario, &config).unwrap_err();
    assert_eq!(error.code(), "adapter_missing");

    let mut config = common::fixture_config();
    config
        .nodes
        .get_mut("fixture-node")
        .unwrap()
        .capabilities
        .clear();
    let error = preflight(&scenario, &config).unwrap_err();
    assert_eq!(error.code(), "capability_unavailable");

    let mut config = common::fixture_config();
    let profile = config
        .nodes
        .get_mut("fixture-node")
        .unwrap()
        .effective_profile
        .as_mut()
        .unwrap();
    profile.pow_limit_bits = Some("1d00ffff".to_string());
    let error = preflight(&scenario, &config).unwrap_err();
    assert_eq!(error.code(), "profile_requirement_mismatch");

    let mut config = common::fixture_config();
    config.link_backend = Some("netns".to_string());
    let error = preflight(&scenario, &config).unwrap_err();
    assert_eq!(error.code(), "link_backend_unavailable");

    let mut required_subsidy = scenario.clone();
    required_subsidy
        .network
        .consensus_profile
        .requirements
        .subsidy_initial_sats = Some(spc_lab::WideU128(5_000_000_000));
    let mut config = common::fixture_config();
    config
        .nodes
        .get_mut("fixture-node")
        .unwrap()
        .effective_profile
        .as_mut()
        .unwrap()
        .subsidy_initial_sats = None;
    let error = preflight(&required_subsidy, &config).unwrap_err();
    assert_eq!(error.code(), "profile_requirement_missing");

    let mut config = common::fixture_config();
    config
        .nodes
        .get_mut("fixture-node")
        .unwrap()
        .effective_profile
        .as_mut()
        .unwrap()
        .tdp_session_cap = None;
    let error = preflight(&scenario, &config).unwrap_err();
    assert_eq!(error.code(), "profile_requirement_missing");
}

#[test]
fn a_scenario_requiring_a_real_capability_cannot_use_a_fixture_adapter() {
    let mut scenario = common::fixture_scenario("scenario_v1_one_pool_two_miners.json");
    scenario.network.consensus_profile.required_capability = "rbitcoin.retarget".to_string();
    let config = common::fixture_config();
    let error = preflight(&scenario, &config).unwrap_err();
    assert_eq!(error.code(), "capability_unavailable");
}

#[test]
fn config_schema_rejects_unknown_fields() {
    let error = LabConfig::from_json("{\"nodes\": {}, \"typo\": true}").unwrap_err();
    assert_eq!(error.code(), "config_schema_invalid");
}
