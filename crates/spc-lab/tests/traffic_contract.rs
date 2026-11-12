//! Traffic contract integration tests (traffic plan T0).
//!
//! Covers the frozen `TrafficSpec` schema (valid shapes, every rejection
//! code, defaults, integer extremes, serialization roundtrip), the
//! schema-evolution contract (old scenarios unchanged, an old runner
//! rejecting a traffic-bearing scenario before any effect), the
//! tx-control protocol/history/lifecycle/seed-domain contracts, the
//! durable command payload contracts, and the AP-1a traffic capability
//! projection (as-is state mapping, fail-closed lineage, the r1 evidence
//! shape including the preserved `getmempoolinfo` failure) plus the
//! scenario preflight gate.

mod common;

use serde::Deserialize;
use serde_json::{json, Value};
use sha2::Digest as _;
use spc_lab::config::LabConfig;
use spc_lab::engine::{EngineOptions, RunEngine};
use spc_lab::ids::ActorId;
use spc_lab::manifest::RunManifest;
use spc_lab::scenario::{Scenario, ScenarioError};
use spc_lab::traffic::commands::{
    BootstrapFund, PauseTxDriver, ResumeTxDriver, RpcSignedTxPayload, SendUserPayment, SetTxRate,
};
use spc_lab::traffic::projection::{
    check_traffic_capability, project_traffic_capability, Ap1aCheckState, TrafficCapabilityRecord,
    AP1A_APPROVED_CONTRACT_SHA256S, AP1A_HISTORICAL_R2_CONTRACT_SHA256,
    AP1A_PRE_RPC_POLICY_CONTRACT_SHA256, AP1A_PROBE_ID, AP1A_RECOGNIZED_UNAPPROVED_CANDIDATES,
    AP1A_TRAFFIC_CONTRACT_SHA256, TRAFFIC_CONSUMER, TRAFFIC_EXPECTED_CHECKS,
    TRAFFIC_FOUNDATIONAL_CHECKS, TRAFFIC_METHODS, TRAFFIC_TARGET_KIND_OWNED,
    TRAFFIC_TARGET_KIND_REMOTE,
};
use spc_lab::traffic::protocol::{
    network_wallet_seed_domain, parse_control_protocol, user_payment_wallet_seed_domain,
    TrafficHistoryCaps, TrafficHistoryHeader, TxControlHello, TxControlHelloAck, TxLifecycleState,
    SEED_DOMAIN_BOOTSTRAP_FUNDING, SEED_DOMAIN_PLANNED_SCHEDULE, TRAFFIC_HISTORY_NETWORK,
    TRAFFIC_HISTORY_REORG_RESUBMITS_MAX, TRAFFIC_HISTORY_UNKNOWN_RETRY_SENDS_MAX,
    TRAFFIC_HISTORY_VERSION, TRAFFIC_KEY_DERIVATION_VERSION, TRAFFIC_KEY_MODE,
    TX_CONTROL_CAPABILITIES_V1, TX_CONTROL_PROTOCOL_STRING, TX_JOURNAL_FORMAT_MAGIC,
    TX_JOURNAL_PROTOCOL_STRING,
};
use spc_lab::traffic::spec::{
    BootstrapMiningMode, TrafficPacing, TrafficSpec, TRAFFIC_CAPABILITY, TRAFFIC_SPEC_VERSION,
};
use spc_lab::RunId;
use spc_lab::{IdentityId, NodeId};

/// Current parent/user-authorized contract, paired with its exact table and
/// metadata below. Synthetic rows test shape only, never funded behavior.
const TEST_CONTRACT_HASH: &str = AP1A_TRAFFIC_CONTRACT_SHA256;
const FULL_MODEL_FAILURE: &str = "shape: maxdatacarriersize: expected JSON integer, got null";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn fixture_value(name: &str) -> Value {
    serde_json::from_str(&common::fixture_scenario_text(name)).unwrap()
}

fn parse(json: &str) -> Result<Scenario, ScenarioError> {
    Scenario::from_json(json)
}

fn parse_and_validate(json: &str) -> Result<(), ScenarioError> {
    let scenario = parse(json)?;
    scenario.validate().map(|_| ())
}

fn traffic_value() -> Value {
    json!({
        "version": 1,
        "required_capability": "spc.node.regtest.mempool_rpc.v1",
        "protocol": {
            "control": "spc-lab-tx-control/v1",
            "history_version": 3
        },
        "bootstrap": {
            "enabled": true,
            "mining_mode": "native",
            "funding_blocks": 101,
            "distribution_outputs_per_wallet": 8,
            "excluded_from_statistics": true
        },
        "driver": {
            "actors": [{"id": "txd-0", "network_wallets": 8, "user_payment_wallets": 2, "hosts_bootstrap_funding": true}],
            "user_payment_wallets": 2,
            "history": "traffic/"
        },
        "rate": {
            "ordinary_tx_per_sec": 0.2,
            "pacing": "fixed_jitter"
        },
        "fees": {
            "rate_sats_vb": {"min": 1, "max": 20},
            "max_fee_sats_per_tx": 100000
        },
        "sizes": {
            "classes_vb": [
                {"vb": 220, "mix": 0.70},
                {"vb": 600, "mix": 0.25},
                {"vb": 1500, "mix": 0.05}
            ],
            "max_vb": 100000
        },
        "spend_types": ["p2pkh"],
        "ark": {
            "enabled": true,
            "registration_before_mining": true,
            "chain_effect_budget_tx_per_sec": 0.05
        },
        "caps": {
            "max_inflight_broadcasts": 64,
            "max_unknown_retries": 3,
            "max_mempool_backlog_tx": 2000,
            "reorg_resubmit_max": 3
        },
        "pause_semantics": "no_broadcast_after_ack"
    })
}

fn scenario_json_with_traffic(mutate: impl FnOnce(&mut Value)) -> String {
    let mut scenario = fixture_value("scenario_v1_two_pools.json");
    let mut traffic = traffic_value();
    mutate(&mut traffic);
    scenario["traffic"] = traffic;
    scenario.to_string()
}

fn expect_traffic_code(mutate: impl FnOnce(&mut Value), code: &str) {
    let json = scenario_json_with_traffic(mutate);
    let err = parse_and_validate(&json)
        .expect_err("expected the mutated traffic section to fail validation");
    assert_eq!(err.code(), code, "unexpected detail: {}", err.detail());
}

fn check_row(id: &str, method: Option<&str>, availability: &str, behavior: &str) -> Value {
    json!({
        "id": id,
        "method": method,
        "availability": availability,
        "behavior": behavior,
        "detail": "synthetic test row"
    })
}

/// The 12 five-method rows with everything passed (the shape AP-1a r1
/// reports use, with statuses overridden per test).
fn passing_five_method_checks() -> Vec<Value> {
    vec![
        check_row(
            "sendrawtransaction.recognized",
            Some("sendrawtransaction"),
            "passed",
            "passed",
        ),
        check_row("getrawmempool", Some("getrawmempool"), "passed", "passed"),
        check_row(
            "getrawmempool.bool-form",
            Some("getrawmempool"),
            "passed",
            "passed",
        ),
        check_row(
            "getrawtransaction.verbosity0",
            Some("getrawtransaction"),
            "passed",
            "passed",
        ),
        check_row(
            "getrawtransaction.verbosity1",
            Some("getrawtransaction"),
            "passed",
            "passed",
        ),
        check_row(
            "getrawtransaction.bool-verbose",
            Some("getrawtransaction"),
            "passed",
            "passed",
        ),
        check_row(
            "getrawtransaction.bool-verbose.async",
            Some("getrawtransaction"),
            "passed",
            "passed",
        ),
        check_row(
            "getmempoolinfo.traffic-consumer",
            Some("getmempoolinfo"),
            "passed",
            "passed",
        ),
        check_row("getblock.verbosity0", Some("getblock"), "passed", "passed"),
        check_row(
            "getblock.verbosity0.rehash",
            Some("getblock"),
            "passed",
            "passed",
        ),
        check_row("getblock.verbosity1", Some("getblock"), "passed", "passed"),
        check_row("getblock.singleparam", Some("getblock"), "passed", "passed"),
        check_row(
            "getrawtransaction.blockhash.traffic-consumer",
            Some("getrawtransaction"),
            "passed",
            "passed",
        ),
    ]
}

fn passing_foundational_checks() -> Vec<Value> {
    vec![
        check_row(
            "getbestblockhash",
            Some("getbestblockhash"),
            "passed",
            "passed",
        ),
        check_row(
            "getblockheader.traffic-consumer",
            Some("getblockheader"),
            "passed",
            "passed",
        ),
        check_row("protocol.id-echo", None, "passed", "passed"),
        check_row(
            "protocol.envelope20.getnetworkinfo",
            Some("getnetworkinfo"),
            "passed",
            "passed",
        ),
        check_row(
            "getblockchaininfo.network",
            Some("getblockchaininfo"),
            "passed",
            "passed",
        ),
        check_row(
            "getblockhash.genesis",
            Some("getblockhash"),
            "passed",
            "passed",
        ),
        check_row(
            "getblockheader.verbose.genesis",
            Some("getblockheader"),
            "passed",
            "passed",
        ),
    ]
}

/// Bark version-floor rows exactly as the r1 evidence records them: they
/// must never affect the traffic projection.
fn version_gate_checks() -> Vec<Value> {
    vec![
        check_row(
            "getnetworkinfo.gate.server",
            Some("getnetworkinfo"),
            "passed",
            "failed",
        ),
        check_row(
            "getnetworkinfo.gate.wallet",
            Some("getnetworkinfo"),
            "passed",
            "failed",
        ),
    ]
}

fn full_model_failure() -> Value {
    let mut row = check_row("getmempoolinfo", Some("getmempoolinfo"), "passed", "failed");
    row["detail"] = json!(FULL_MODEL_FAILURE);
    row["fail_kind"] = json!("shape");
    row
}

fn report_json(mut checks: Vec<Value>) -> Value {
    if !checks.iter().any(|row| row["id"] == "getmempoolinfo") {
        checks.push(full_model_failure());
    }
    let failures: Vec<_> = checks
        .iter()
        .filter(|row| {
            row["id"] == "getmempoolinfo"
                && (row["availability"] == "failed" || row["behavior"] == "failed")
        })
        .cloned()
        .collect();
    json!({
        "probe": AP1A_PROBE_ID,
        "probe_version": "1.0.0",
        "schema_version": 2,
        "generated_utc": "2026-11-08T00:00:00Z",
        "run_token": "AP1a-test",
        "target": {"label": "test-node", "kind": "owned-fresh-regtest-node"},
        "profile": {
            "name": "test",
            "server_min_rpc_version": 310000,
            "wallet_min_rpc_version": 290000,
            "expected_network": "regtest",
            "expected_genesis":
                "0f9188f13cb7b2c71f2a335e3a4fc328bf5beb436012afca590b1a11433e0ad2"
        },
        "contract_hash": TEST_CONTRACT_HASH,
        "traffic_consumer": TRAFFIC_CONSUMER,
        "consumer_model_failures": failures,
        "checks": checks,
        "node": {
            "label": "test-node",
            "binary": "/tmp/test/rbitcoin-node",
            "binary_sha256":
                "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
        },
        "observations": {
            "mempool_size": 0,
            "traffic_fee_policy": {
                "minrelaytxfee_btc_kb_lexeme": "0.00001000",
                "minrelaytxfee_sats_kb": 1000,
                "maxmempool": 300000000, "bytes": 0, "usage": 0, "errors": []
            }
        },
        "verdict": {
            "status": "surface_compatible",
            "reasons": ["funded-unproven"],
            "summary": "synthetic test report",
            "funded_state": "unproven",
            "full_acceptance": false
        }
    })
}

fn project(checks: Vec<Value>) -> TrafficCapabilityRecord {
    project_traffic_capability(report_json(checks).to_string().as_bytes()).unwrap()
}

fn project_err(checks: Vec<Value>) -> spc_lab::traffic::TrafficError {
    project_traffic_capability(report_json(checks).to_string().as_bytes())
        .expect_err("expected projection failure")
}

fn satisfied_scenario() -> Scenario {
    parse(&scenario_json_with_traffic(|_| {})).unwrap()
}

// ---------------------------------------------------------------------------
// Scenario schema: acceptance, evolution, defaults
// ---------------------------------------------------------------------------

#[test]
fn traffic_scenario_validates_and_declares_capability() {
    let scenario = parse(&scenario_json_with_traffic(|_| {})).unwrap();
    let report = scenario.validate().expect("traffic scenario validates");
    assert_eq!(
        scenario.traffic.as_ref().unwrap().version,
        TRAFFIC_SPEC_VERSION
    );
    assert!(report.required_capabilities.contains(TRAFFIC_CAPABILITY));
    // The baseline capabilities are still declared alongside traffic
    // (the two-pool fixture's consensus capability).
    assert!(report.required_capabilities.contains("fixture.scripted"));
}

#[test]
fn existing_scenarios_have_no_traffic_and_validate_unchanged() {
    for name in [
        "scenario_v1_one_pool_two_miners.json",
        "scenario_v1_two_pools.json",
    ] {
        let scenario = common::fixture_scenario(name);
        scenario
            .validate()
            .expect("pre-traffic fixture still validates");
        assert!(scenario.traffic.is_none());
    }
}

/// The frozen baseline runner (pre-T0 `scenario.rs`) rejects a
/// traffic-bearing scenario at parse time: `deny_unknown_fields` fails on
/// the unknown top-level field before any validation or process start.
/// The mirror struct replicates that exact top-level field set.
#[test]
fn old_runner_rejects_traffic_bearing_scenario_before_any_effect() {
    // The mirror struct replicates the pre-T0 top-level field set purely
    // for its deny_unknown_fields parse behavior; its fields are never
    // read.
    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    #[allow(dead_code)]
    struct PreTrafficScenario {
        scenario_version: u32,
        run: Value,
        network: Value,
        nodes: Vec<Value>,
        pools: Vec<Value>,
        fronts: Vec<Value>,
        identities: Vec<Value>,
        miners: Vec<Value>,
        #[serde(default)]
        ark_backends: Vec<Value>,
        resources: Value,
        #[serde(default)]
        links: Vec<Value>,
        #[serde(default)]
        phases: Vec<Value>,
        #[serde(default)]
        assertions: Vec<Value>,
        #[serde(default)]
        statistics: Vec<Value>,
    }

    let plain = fixture_value("scenario_v1_two_pools.json");
    serde_json::from_value::<PreTrafficScenario>(plain.clone())
        .expect("plain fixture parses under the pre-traffic field set");

    let mut with_traffic = plain;
    with_traffic["traffic"] = traffic_value();
    let err = serde_json::from_value::<PreTrafficScenario>(with_traffic)
        .expect_err("old runner must reject the traffic field");
    assert!(
        err.to_string().contains("unknown field `traffic`"),
        "expected an unknown-field rejection, got: {err}"
    );
}

#[test]
fn traffic_defaults_apply_when_fields_absent() {
    let mut traffic = traffic_value();
    traffic["bootstrap"]
        .as_object_mut()
        .unwrap()
        .remove("mining_mode");
    traffic["rate"].as_object_mut().unwrap().remove("pacing");
    traffic["caps"]
        .as_object_mut()
        .unwrap()
        .remove("reorg_resubmit_max");
    let spec: TrafficSpec = serde_json::from_value(traffic).unwrap();
    assert_eq!(spec.bootstrap.mining_mode, BootstrapMiningMode::Native);
    assert_eq!(spec.rate.pacing, TrafficPacing::FixedJitter);
    assert_eq!(spec.caps.reorg_resubmit_max, 3);
    spec.validate(2, &mut Default::default())
        .expect("defaults satisfy validation");
}

#[test]
fn traffic_spec_serialization_roundtrip() {
    let spec: TrafficSpec = serde_json::from_value(traffic_value()).unwrap();
    let roundtripped: TrafficSpec =
        serde_json::from_str(&serde_json::to_string(&spec).unwrap()).unwrap();
    assert_eq!(spec, roundtripped);
}

#[test]
fn scenario_roundtrip_with_traffic() {
    let scenario = parse(&scenario_json_with_traffic(|_| {})).unwrap();
    let json = serde_json::to_string(&scenario).unwrap();
    let roundtripped = parse(&json).unwrap();
    roundtripped
        .validate()
        .expect("roundtripped scenario validates");
    // Scenario has no PartialEq; compare through the canonical JSON form.
    assert_eq!(serde_json::to_string(&roundtripped).unwrap(), json);
}

// ---------------------------------------------------------------------------
// Scenario schema: rejection codes
// ---------------------------------------------------------------------------

#[test]
fn traffic_version_and_capability_rejections() {
    expect_traffic_code(|t| t["version"] = json!(2), "traffic_version_unsupported");
    expect_traffic_code(
        |t| t["required_capability"] = json!(""),
        "traffic_capability_missing",
    );
    expect_traffic_code(
        |t| t["required_capability"] = json!("spc.node.regtest.something.else"),
        "traffic_capability_unknown",
    );
}

#[test]
fn traffic_protocol_rejections() {
    expect_traffic_code(
        |t| t["protocol"]["control"] = json!("spc-lab-tx-control/v2"),
        "traffic_protocol_unsupported",
    );
    expect_traffic_code(
        |t| t["protocol"]["control"] = json!("spc-lab-other-control/v1"),
        "traffic_protocol_unsupported",
    );
    // The journal history contract is version 3 (atomic rollback proof):
    // version 1 - the retired pre-rework 4-field shape,
    // never written by any runtime - and any later version are refused,
    // never migrated.
    expect_traffic_code(
        |t| t["protocol"]["history_version"] = json!(1),
        "traffic_protocol_unsupported",
    );
    expect_traffic_code(
        |t| t["protocol"]["history_version"] = json!(2),
        "traffic_protocol_unsupported",
    );
    // The protocol string is exact: surrounding whitespace is a
    // rejection, never a trimmed acceptance (F8).
    for bad in [
        " spc-lab-tx-control/v1",
        "spc-lab-tx-control/v1 ",
        "spc-lab-tx-control/v1\t",
    ] {
        expect_traffic_code(
            |t| t["protocol"]["control"] = json!(bad),
            "traffic_protocol_unsupported",
        );
    }
}

#[test]
fn traffic_bootstrap_rejections() {
    expect_traffic_code(
        |t| t["bootstrap"]["mining_mode"] = json!("rpc_generate"),
        "traffic_bootstrap_mining_mode_invalid",
    );
    expect_traffic_code(
        |t| t["bootstrap"]["funding_blocks"] = json!(100),
        "bootstrap_maturity_insufficient",
    );
    expect_traffic_code(
        |t| t["bootstrap"]["funding_blocks"] = json!(1001),
        "traffic_bootstrap_funding_blocks_out_of_range",
    );
    expect_traffic_code(
        |t| t["bootstrap"]["distribution_outputs_per_wallet"] = json!(0),
        "traffic_bootstrap_distribution_invalid",
    );
    expect_traffic_code(
        |t| t["bootstrap"]["distribution_outputs_per_wallet"] = json!(65),
        "traffic_bootstrap_distribution_invalid",
    );
    expect_traffic_code(
        |t| t["bootstrap"]["excluded_from_statistics"] = json!(false),
        "traffic_bootstrap_statistics_not_excluded",
    );
    expect_traffic_code(
        |t| t["bootstrap"]["enabled"] = json!(false),
        "traffic_bootstrap_required",
    );
}

#[test]
fn traffic_driver_rejections() {
    expect_traffic_code(
        |t| t["driver"]["actors"] = json!([]),
        "traffic_actor_count_invalid",
    );
    expect_traffic_code(
        |t| {
            t["driver"]["actors"] = json!([
                {"id": "txd-0", "network_wallets": 8, "user_payment_wallets": 1, "hosts_bootstrap_funding": true},
                {"id": "txd-1", "network_wallets": 8, "user_payment_wallets": 1, "hosts_bootstrap_funding": false},
                {"id": "txd-2", "network_wallets": 8, "user_payment_wallets": 1, "hosts_bootstrap_funding": false}
            ])
        },
        "traffic_actor_count_invalid",
    );
    expect_traffic_code(
        |t| t["driver"]["actors"] = json!([{"id": "", "network_wallets": 8, "user_payment_wallets": 2, "hosts_bootstrap_funding": true}]),
        "traffic_actor_id_invalid",
    );
    // Actor ids obey the shared ActorId charset bound (F6): at most 128
    // bytes, ASCII, no control characters - they end up in private file
    // names, evidence and process specs.
    for bad in ["bad\nid", "bad id", "txd-é", "a".repeat(129).as_str()] {
        expect_traffic_code(
            |t| t["driver"]["actors"] = json!([{"id": bad, "network_wallets": 8, "user_payment_wallets": 2, "hosts_bootstrap_funding": true}]),
            "traffic_actor_id_invalid",
        );
    }
    expect_traffic_code(
        |t| {
            t["driver"]["actors"] = json!([
                {"id": "txd-0", "network_wallets": 4, "user_payment_wallets": 1, "hosts_bootstrap_funding": true},
                {"id": "txd-0", "network_wallets": 4, "user_payment_wallets": 1, "hosts_bootstrap_funding": false}
            ])
        },
        "traffic_actor_id_duplicate",
    );
    expect_traffic_code(
        |t| t["driver"]["actors"] = json!([{"id": "txd-0", "network_wallets": 0, "user_payment_wallets": 2, "hosts_bootstrap_funding": true}]),
        "traffic_actor_wallet_overflow",
    );
    expect_traffic_code(
        |t| t["driver"]["actors"] = json!([{"id": "txd-0", "network_wallets": 257, "user_payment_wallets": 2, "hosts_bootstrap_funding": true}]),
        "traffic_actor_wallet_overflow",
    );
    expect_traffic_code(
        |t| t["driver"]["user_payment_wallets"] = json!(0),
        "traffic_user_payment_wallets_invalid",
    );
    // The two-pool fixture declares 2 identities.
    expect_traffic_code(
        |t| t["driver"]["user_payment_wallets"] = json!(4),
        "traffic_user_payment_wallets_mismatch",
    );
    // The journal is confined to the traffic/ subdirectory: no bare
    // file name, no artifact collision, no escape, no whitespace.
    for bad in [
        "",
        "/abs/path",
        "a/../b",
        "./x",
        "a//b",
        "history.jsonl",
        "scenario.json",
        "traffic",
        "traffic/history.jsonl",
        "traffic/../scenario.json",
        "traffic/history.jsonl ",
        " traffic/history.jsonl",
        "traffic\\history.jsonl",
        "private/traffic/history.jsonl",
    ] {
        expect_traffic_code(
            |t| t["driver"]["history"] = json!(bad),
            "traffic_history_path_invalid",
        );
    }
}

#[test]
fn traffic_rate_pacing_and_fee_rejections() {
    expect_traffic_code(
        |t| t["rate"]["ordinary_tx_per_sec"] = json!(0.001),
        "traffic_rate_bounds_invalid",
    );
    expect_traffic_code(
        |t| t["rate"]["ordinary_tx_per_sec"] = json!(20.5),
        "traffic_rate_exceeds_preset_cap",
    );
    expect_traffic_code(
        |t| t["rate"]["pacing"] = json!("burst"),
        "traffic_pacing_unsupported",
    );
    expect_traffic_code(
        |t| t["fees"]["rate_sats_vb"]["min"] = json!(0),
        "traffic_fee_bounds_invalid",
    );
    expect_traffic_code(
        |t| t["fees"]["rate_sats_vb"]["max"] = json!(101),
        "traffic_fee_bounds_invalid",
    );
    expect_traffic_code(
        |t| {
            t["fees"]["rate_sats_vb"] = json!({"min": 20, "max": 10});
        },
        "traffic_fee_bounds_invalid",
    );
    expect_traffic_code(
        |t| t["fees"]["max_fee_sats_per_tx"] = json!(0),
        "traffic_fee_bounds_invalid",
    );
    expect_traffic_code(
        |t| t["fees"]["max_fee_sats_per_tx"] = json!(100001),
        "traffic_fee_bounds_invalid",
    );
}

#[test]
fn traffic_size_rejections() {
    expect_traffic_code(
        |t| t["sizes"]["classes_vb"] = json!([]),
        "traffic_size_mix_invalid",
    );
    expect_traffic_code(
        |t| t["sizes"]["classes_vb"] = json!([{"vb": 220, "mix": 0.9}]),
        "traffic_size_mix_invalid",
    );
    expect_traffic_code(
        |t| t["sizes"]["classes_vb"] = json!([{"vb": 220, "mix": 0.0}]),
        "traffic_size_mix_invalid",
    );
    expect_traffic_code(
        |t| t["sizes"]["max_vb"] = json!(100001),
        "traffic_size_cap_invalid",
    );
    expect_traffic_code(
        |t| t["sizes"]["classes_vb"] = json!([{"vb": 200000, "mix": 1.0}]),
        "traffic_size_cap_invalid",
    );
}

#[test]
fn traffic_spend_type_ark_and_caps_rejections() {
    expect_traffic_code(
        |t| t["spend_types"] = json!(["p2wpkh"]),
        "traffic_spend_type_gated",
    );
    expect_traffic_code(
        |t| t["spend_types"] = json!([]),
        "traffic_spend_type_missing",
    );
    expect_traffic_code(
        |t| t["spend_types"] = json!(["p2pkh", "p2pkh"]),
        "traffic_spend_type_duplicate",
    );
    expect_traffic_code(
        |t| t["ark"]["registration_before_mining"] = json!(false),
        "traffic_ark_registration_invalid",
    );
    expect_traffic_code(
        |t| t["ark"]["chain_effect_budget_tx_per_sec"] = json!(1.5),
        "traffic_ark_budget_invalid",
    );
    expect_traffic_code(
        |t| {
            t["ark"]["enabled"] = json!(false);
            t["ark"]["chain_effect_budget_tx_per_sec"] = json!(0.05);
        },
        "traffic_ark_budget_invalid",
    );
    expect_traffic_code(
        |t| {
            t["rate"]["ordinary_tx_per_sec"] = json!(20.0);
            t["ark"]["chain_effect_budget_tx_per_sec"] = json!(0.05);
        },
        "traffic_admission_overlapping_streams",
    );
    expect_traffic_code(
        |t| t["caps"]["max_inflight_broadcasts"] = json!(65),
        "traffic_caps_invalid",
    );
    expect_traffic_code(
        |t| t["caps"]["max_unknown_retries"] = json!(4),
        "traffic_caps_invalid",
    );
    expect_traffic_code(
        |t| t["caps"]["max_mempool_backlog_tx"] = json!(2001),
        "traffic_caps_invalid",
    );
    expect_traffic_code(
        |t| t["caps"]["reorg_resubmit_max"] = json!(0),
        "traffic_caps_invalid",
    );
    expect_traffic_code(
        |t| t["caps"]["reorg_resubmit_max"] = json!(4),
        "traffic_caps_invalid",
    );
}

/// Integer extremes are rejected by range checks, never by overflow.
#[test]
fn traffic_integer_extremes_rejected_without_panic() {
    expect_traffic_code(
        |t| t["bootstrap"]["funding_blocks"] = json!(u32::MAX),
        "traffic_bootstrap_funding_blocks_out_of_range",
    );
    expect_traffic_code(
        |t| t["fees"]["max_fee_sats_per_tx"] = json!(u64::MAX),
        "traffic_fee_bounds_invalid",
    );
    expect_traffic_code(
        |t| t["sizes"]["max_vb"] = json!(u64::MAX),
        "traffic_size_cap_invalid",
    );
    expect_traffic_code(
        |t| t["caps"]["max_inflight_broadcasts"] = json!(u64::MAX),
        "traffic_caps_invalid",
    );
    expect_traffic_code(
        |t| t["driver"]["actors"] = json!([{"id": "txd-0", "network_wallets": u32::MAX, "user_payment_wallets": 2, "hosts_bootstrap_funding": true}]),
        "traffic_actor_wallet_overflow",
    );
    expect_traffic_code(
        |t| t["driver"]["user_payment_wallets"] = json!(u32::MAX),
        "traffic_user_payment_wallets_mismatch",
    );
}

// ---------------------------------------------------------------------------
// tx-control protocol, history, lifecycle, seed domains
// ---------------------------------------------------------------------------

fn valid_hello() -> TxControlHello {
    TxControlHello {
        protocol_name: "spc-lab-tx-control".to_string(),
        protocol_major: 1,
        protocol_minor: 0,
        driver_id: ActorId::trusted("txd-0"),
        network_wallets: 8,
        user_payment_wallets: 2,
        capabilities: Vec::new(),
    }
}

#[test]
fn hello_validates_and_unknown_major_fails_before_broadcast() {
    valid_hello().validate().unwrap();
    let mut hello = valid_hello();
    hello.protocol_major = 2;
    let err = hello.validate().unwrap_err();
    assert_eq!(err.code(), "traffic_protocol_unsupported");
    let mut hello = valid_hello();
    hello.protocol_name = "spc-lab-other".into();
    assert_eq!(
        hello.validate().unwrap_err().code(),
        "traffic_protocol_unsupported"
    );
    let mut hello = valid_hello();
    hello.driver_id = ActorId::trusted("bad id");
    assert_eq!(
        hello.validate().unwrap_err().code(),
        "traffic_driver_id_invalid"
    );
    let mut hello = valid_hello();
    hello.driver_id = ActorId::trusted("x".repeat(129));
    assert_eq!(
        hello.validate().unwrap_err().code(),
        "traffic_driver_id_invalid"
    );
    let mut hello = valid_hello();
    hello.network_wallets = 0;
    assert_eq!(
        hello.validate().unwrap_err().code(),
        "traffic_hello_wallet_count_invalid"
    );
}

#[test]
fn v1_capability_registry_requires_known_explicit_advertisement() {
    assert_eq!(
        TX_CONTROL_CAPABILITIES_V1,
        [
            "payment_wallet_read_v1",
            "ordinary_churn",
            "user_payment",
            "inject_tx",
            "bootstrap_fund"
        ]
    );
    let mut hello = valid_hello();
    hello.capabilities = vec!["burst".to_string()];
    hello.validate().unwrap();
    // A driver advertising an unregistered capability never makes the
    // controller rely on it.
    assert!(!hello.supports("burst"));
    assert!(!hello.supports("anything"));
    assert!(!hello.supports("payment_wallet_read_v1"));
    hello.capabilities.push("payment_wallet_read_v1".into());
    assert!(hello.supports("payment_wallet_read_v1"));
}

#[test]
fn hello_ack_contract() {
    let ack = TxControlHelloAck {
        protocol_name: "spc-lab-tx-control".into(),
        protocol_major: 1,
        protocol_minor: 0,
        accepted: true,
        history_version: TRAFFIC_HISTORY_VERSION,
        reason: None,
    };
    ack.validate().unwrap();
    // The accepted history contract is exactly version 3: the retired
    // version 1 and any later version are refused before any effect.
    for bad_version in [1u32, 2, 4] {
        let mut bad = ack.clone();
        bad.history_version = bad_version;
        assert_eq!(
            bad.validate().unwrap_err().code(),
            "traffic_protocol_unsupported",
            "history_version {bad_version}"
        );
    }
    let mut rejected = ack;
    rejected.accepted = false;
    assert_eq!(
        rejected.validate().unwrap_err().code(),
        "traffic_hello_ack_invalid"
    );
    rejected.reason = Some("unknown major version".into());
    rejected.validate().unwrap();
}

/// A syntactically valid 64-lowercase-hex digest stand-in for header
/// fixture fields (the conformance suite proves the real T1a digests).
const HEX64: &str = "abababababababababababababababababababababababababababababababab";

fn valid_history_header() -> TrafficHistoryHeader {
    TrafficHistoryHeader {
        record: "header".into(),
        seq: 0,
        format_magic: TX_JOURNAL_FORMAT_MAGIC.into(),
        history_version: TRAFFIC_HISTORY_VERSION,
        protocol: TX_JOURNAL_PROTOCOL_STRING.into(),
        network: TRAFFIC_HISTORY_NETWORK.into(),
        key_mode: TRAFFIC_KEY_MODE.into(),
        derivation_version: TRAFFIC_KEY_DERIVATION_VERSION,
        run_id: RunId::trusted("run-x"),
        driver_id: ActorId::trusted("txd-0"),
        run_seed_fingerprint: HEX64.into(),
        allocation_digest: HEX64.into(),
        caps: TrafficHistoryCaps {
            unknown_retry_sends: TRAFFIC_HISTORY_UNKNOWN_RETRY_SENDS_MAX,
            reorg_resubmits: TRAFFIC_HISTORY_REORG_RESUBMITS_MAX,
        },
        created_unix_ms: 1_762_572_800_000,
    }
}

/// The history-3 header contract: the strict full-record mirror of the
/// Fable-approved T1a `JournalRecord::Header` line. Every field is
/// validated fail-closed, the strict parse rejects unknown and missing
/// fields, and serialization roundtrips byte-identically.
#[test]
fn history_header_contract() {
    let header = valid_history_header();
    header.validate().unwrap();
    TrafficHistoryHeader::from_journal_line(&serde_json::to_string(&header).unwrap()).unwrap();

    // Record kind, seq and format magic: the first line is the header.
    {
        let mut bad = header.clone();
        bad.record = "facts".into();
        assert_eq!(
            bad.validate().unwrap_err().code(),
            "traffic_history_record_invalid"
        );
        let mut bad = header.clone();
        bad.seq = 1;
        assert_eq!(
            bad.validate().unwrap_err().code(),
            "traffic_history_record_invalid"
        );
        let mut bad = header.clone();
        bad.format_magic = "NOT-THE-MAGIC".into();
        assert_eq!(
            bad.validate().unwrap_err().code(),
            "traffic_history_record_invalid"
        );
    }

    // History version and journal protocol: version 1 is refused
    // explicitly, and the CONTROL protocol string never passes for the
    // journal protocol.
    for bad_version in [1u32, 2, 4] {
        let mut bad = header.clone();
        bad.history_version = bad_version;
        assert_eq!(
            bad.validate().unwrap_err().code(),
            "traffic_protocol_unsupported",
            "history_version {bad_version}"
        );
    }
    let mut bad = header.clone();
    bad.protocol = TX_CONTROL_PROTOCOL_STRING.into();
    assert_eq!(
        bad.validate().unwrap_err().code(),
        "traffic_protocol_unsupported"
    );

    // Network / key mode / derivation version binding.
    let mut bad = header.clone();
    bad.network = "mainnet".into();
    assert_eq!(
        bad.validate().unwrap_err().code(),
        "traffic_history_network_invalid"
    );
    let mut bad = header.clone();
    bad.key_mode = "hardware_wallet_v9".into();
    assert_eq!(
        bad.validate().unwrap_err().code(),
        "traffic_key_mode_unsupported"
    );
    let mut bad = header.clone();
    bad.derivation_version = 1;
    assert_eq!(
        bad.validate().unwrap_err().code(),
        "traffic_key_mode_unsupported"
    );

    // Typed ids re-validated (serde never validated them).
    let mut bad = header.clone();
    bad.run_id = RunId::trusted("bad run");
    assert_eq!(bad.validate().unwrap_err().code(), "traffic_run_id_invalid");
    let mut bad = header.clone();
    bad.driver_id = ActorId::trusted("bad\nid");
    assert_eq!(
        bad.validate().unwrap_err().code(),
        "traffic_driver_id_invalid"
    );

    // Digest fields: exactly 64 lowercase hex, never uppercase, never
    // trimmed, never a wrong length.
    for (field, mutate) in [
        (
            "run_seed_fingerprint",
            (&|h: &mut TrafficHistoryHeader| h.run_seed_fingerprint = "AB".repeat(32))
                as &dyn Fn(&mut TrafficHistoryHeader),
        ),
        (
            "run_seed_fingerprint",
            (&|h: &mut TrafficHistoryHeader| h.run_seed_fingerprint = "ab".repeat(31))
                as &dyn Fn(&mut TrafficHistoryHeader),
        ),
        (
            "run_seed_fingerprint",
            (&|h: &mut TrafficHistoryHeader| h.run_seed_fingerprint = "zz".repeat(32))
                as &dyn Fn(&mut TrafficHistoryHeader),
        ),
        (
            "allocation_digest",
            (&|h: &mut TrafficHistoryHeader| h.allocation_digest = "ab ".repeat(32))
                as &dyn Fn(&mut TrafficHistoryHeader),
        ),
    ] {
        let mut bad = header.clone();
        mutate(&mut bad);
        assert_eq!(
            bad.validate().unwrap_err().code(),
            "traffic_history_digest_invalid",
            "field {field}"
        );
    }

    // Caps bounds: out-of-range values reject, never clamp.
    for (unknown_retry_sends, reorg_resubmits) in [(4u32, 3u32), (0, 0), (0, 4), (9, 9)] {
        let mut bad = header.clone();
        bad.caps = TrafficHistoryCaps {
            unknown_retry_sends,
            reorg_resubmits,
        };
        assert_eq!(
            bad.validate().unwrap_err().code(),
            "traffic_caps_invalid",
            "caps {unknown_retry_sends}/{reorg_resubmits}"
        );
    }

    // The strict parse: an unknown field, a missing field and a non-
    // object line all reject with the unparseable code (deny_unknown_
    // fields, no defaults).
    let golden = serde_json::to_string(&header).unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&golden).unwrap();
    value["extra_field"] = json!(1);
    let err = TrafficHistoryHeader::from_journal_line(&value.to_string()).unwrap_err();
    assert_eq!(err.code(), "traffic_history_header_unparseable");
    let mut value: serde_json::Value = serde_json::from_str(&golden).unwrap();
    value.as_object_mut().unwrap().remove("allocation_digest");
    let err = TrafficHistoryHeader::from_journal_line(&value.to_string()).unwrap_err();
    assert_eq!(err.code(), "traffic_history_header_unparseable");
    let err = TrafficHistoryHeader::from_journal_line("not json").unwrap_err();
    assert_eq!(err.code(), "traffic_history_header_unparseable");
    let err = TrafficHistoryHeader::from_journal_line("[1,2,3]").unwrap_err();
    assert_eq!(err.code(), "traffic_history_header_unparseable");

    // Field order is part of the contract: serialization is the exact
    // T1a line shape (record, seq, magic, version, protocol, ...).
    let reparsed = TrafficHistoryHeader::from_journal_line(&golden).unwrap();
    assert_eq!(serde_json::to_string(&reparsed).unwrap(), golden);
    assert_eq!(reparsed, header);
}

#[test]
fn control_protocol_parser_and_seed_domains_are_exact() {
    assert_eq!(parse_control_protocol("spc-lab-tx-control/v1"), Some(1));
    assert_eq!(parse_control_protocol("spc-lab-tx-control/v2"), Some(2));
    assert_eq!(parse_control_protocol("spc-lab-tx-control/v1.2"), None);
    assert_eq!(parse_control_protocol("spc-lab-tx-control/1"), None);
    assert_eq!(parse_control_protocol("other/v1"), None);
    assert_eq!(parse_control_protocol("spc-lab-tx-control"), None);

    assert_eq!(
        SEED_DOMAIN_BOOTSTRAP_FUNDING,
        "spc-lab/traffic/bootstrap-funding"
    );
    assert_eq!(
        network_wallet_seed_domain(0),
        "spc-lab/traffic/network-wallet/0"
    );
    assert_eq!(
        network_wallet_seed_domain(7),
        "spc-lab/traffic/network-wallet/7"
    );
    assert_eq!(
        user_payment_wallet_seed_domain(3),
        "spc-lab/traffic/user-payment-wallet/3"
    );
    // The planned-schedule domain is frozen (F8) so T1 never invents
    // the string: (run_seed, protocol_version, driver_id, sequence).
    assert_eq!(
        SEED_DOMAIN_PLANNED_SCHEDULE,
        "spc-lab/traffic/planned-schedule"
    );
}

#[test]
fn lifecycle_state_vocabulary_and_terminality() {
    assert_eq!(TxLifecycleState::Created.as_str(), "created");
    assert_eq!(TxLifecycleState::BroadcastSent.as_str(), "broadcast_sent");
    assert_eq!(TxLifecycleState::AcceptedByRpc.as_str(), "accepted_by_rpc");
    assert_eq!(
        TxLifecycleState::ObservedInMempool.as_str(),
        "observed_in_mempool"
    );
    assert_eq!(TxLifecycleState::Confirmed.as_str(), "confirmed");
    assert_eq!(TxLifecycleState::Rejected.as_str(), "rejected");
    assert_eq!(TxLifecycleState::Conflicted.as_str(), "conflicted");
    assert_eq!(TxLifecycleState::Unknown.as_str(), "unknown");
    assert_eq!(TxLifecycleState::Reorged.as_str(), "reorged");
    assert_eq!(TxLifecycleState::Invalidated.as_str(), "invalidated");
    assert_eq!(TxLifecycleState::Dropped.as_str(), "dropped");
    assert_eq!(TxLifecycleState::Expired.as_str(), "expired");
    assert!(TxLifecycleState::Rejected.is_terminal());
    assert!(TxLifecycleState::Conflicted.is_terminal());
    // confirmed is not terminal: canonical displacement reconciles it.
    assert!(!TxLifecycleState::Confirmed.is_terminal());
    let state: TxLifecycleState = serde_json::from_value(json!("observed_in_mempool")).unwrap();
    assert_eq!(state, TxLifecycleState::ObservedInMempool);
}

// ---------------------------------------------------------------------------
// Durable command payloads
// ---------------------------------------------------------------------------

/// A valid regtest p2pkh destination address (the classic testnet-form
/// example; regtest shares the address prefixes).
const REGTEST_P2PKH_ADDRESS: &str = "mipcBbFg9gMiCh81Kj8tqqdgoZub1ZJRfn";

#[test]
fn command_payloads_validate_and_reject_out_of_bounds() {
    PauseTxDriver {
        driver_id: ActorId::trusted("txd-0"),
    }
    .validate()
    .unwrap();
    ResumeTxDriver {
        driver_id: ActorId::trusted("txd-0"),
    }
    .validate()
    .unwrap();
    for bad in ["", "bad id", "x".repeat(129).as_str()] {
        assert_eq!(
            PauseTxDriver {
                driver_id: ActorId::trusted(bad)
            }
            .validate()
            .unwrap_err()
            .code(),
            "traffic_driver_id_invalid"
        );
    }

    SetTxRate {
        driver_id: ActorId::trusted("txd-0"),
        ordinary_tx_per_sec: 1.0,
    }
    .validate()
    .unwrap();
    assert_eq!(
        SetTxRate {
            driver_id: ActorId::trusted("txd-0"),
            ordinary_tx_per_sec: 20.5,
        }
        .validate()
        .unwrap_err()
        .code(),
        "traffic_rate_exceeds_preset_cap"
    );
    assert_eq!(
        SetTxRate {
            driver_id: ActorId::trusted("txd-0"),
            ordinary_tx_per_sec: 0.001,
        }
        .validate()
        .unwrap_err()
        .code(),
        "traffic_rate_bounds_invalid"
    );

    SendUserPayment {
        user: IdentityId::trusted("id-a"),
        destination_address: REGTEST_P2PKH_ADDRESS.to_string(),
        amount_sats: 1000,
        fee_rate_sats_vb: 5,
    }
    .validate()
    .unwrap();
    assert_eq!(
        SendUserPayment {
            user: IdentityId::trusted("id-a"),
            destination_address: REGTEST_P2PKH_ADDRESS.to_string(),
            amount_sats: 0,
            fee_rate_sats_vb: 5,
        }
        .validate()
        .unwrap_err()
        .code(),
        "traffic_payment_amount_invalid"
    );
    assert_eq!(
        SendUserPayment {
            user: IdentityId::trusted("id-a"),
            destination_address: REGTEST_P2PKH_ADDRESS.to_string(),
            amount_sats: 1000,
            fee_rate_sats_vb: 101,
        }
        .validate()
        .unwrap_err()
        .code(),
        "traffic_fee_bounds_invalid"
    );

    BootstrapFund {
        driver_id: ActorId::trusted("txd-0"),
        node_id: NodeId::trusted("node-a"),
    }
    .validate()
    .unwrap();
    assert_eq!(
        BootstrapFund {
            driver_id: ActorId::trusted("txd-0"),
            node_id: NodeId::trusted(""),
        }
        .validate()
        .unwrap_err()
        .code(),
        "traffic_node_id_invalid"
    );

    RpcSignedTxPayload {
        raw_tx_hex: "01000000".into(),
    }
    .validate()
    .unwrap();
    for bad in ["", "abc", "zz"] {
        assert_eq!(
            RpcSignedTxPayload {
                raw_tx_hex: bad.into()
            }
            .validate()
            .unwrap_err()
            .code(),
            "traffic_payload_invalid"
        );
    }
}

/// F5: the destination is an explicit, bounded, fail-closed address
/// field - format, network and script class validated with the shared
/// bitcoin types; no freeform endpoint, no default, no silent ignore.
#[test]
fn send_user_payment_destination_is_validated_fail_closed() {
    let valid = SendUserPayment {
        user: IdentityId::trusted("id-a"),
        destination_address: REGTEST_P2PKH_ADDRESS.to_string(),
        amount_sats: 1000,
        fee_rate_sats_vb: 5,
    };
    valid.validate().unwrap();

    // Garbage, empty, unbounded or control-character destinations.
    for bad in [
        "",
        "not-an-address",
        "x".repeat(200).as_str(),
        "bad\naddress",
    ] {
        assert_eq!(
            SendUserPayment {
                destination_address: bad.to_string(),
                ..valid.clone()
            }
            .validate()
            .unwrap_err()
            .code(),
            "traffic_payment_destination_invalid"
        );
    }

    // A mainnet address is not valid for the lab's regtest network.
    assert_eq!(
        SendUserPayment {
            destination_address: "1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2".to_string(),
            ..valid.clone()
        }
        .validate()
        .unwrap_err()
        .code(),
        "traffic_payment_destination_network_invalid"
    );

    // A witness OUTPUT is admitted without enabling witness inputs.
    assert!(SendUserPayment {
        destination_address: "bcrt1qw508d6qejxtdg4y5r3zarvary0c5xw7kygt080".to_string(),
        ..valid
    }
    .validate()
    .is_ok());

    // An invalid user id is rejected with the user code.
    assert_eq!(
        SendUserPayment {
            user: IdentityId::trusted("bad id"),
            destination_address: REGTEST_P2PKH_ADDRESS.to_string(),
            amount_sats: 1000,
            fee_rate_sats_vb: 5,
        }
        .validate()
        .unwrap_err()
        .code(),
        "traffic_payment_user_invalid"
    );
}

#[test]
fn command_payload_serde_is_deterministic() {
    let rate = SetTxRate {
        driver_id: ActorId::trusted("txd-0"),
        ordinary_tx_per_sec: 0.2,
    };
    let json = serde_json::to_string(&rate).unwrap();
    assert_eq!(json, r#"{"driver_id":"txd-0","ordinary_tx_per_sec":0.2}"#);
    assert_eq!(serde_json::to_string(&rate).unwrap(), json);
    assert_eq!(serde_json::from_str::<SetTxRate>(&json).unwrap(), rate);

    let payment = SendUserPayment {
        user: IdentityId::trusted("id-a"),
        destination_address: REGTEST_P2PKH_ADDRESS.to_string(),
        amount_sats: 1500,
        fee_rate_sats_vb: 3,
    };
    let json = serde_json::to_string(&payment).unwrap();
    assert_eq!(
        json,
        format!(
            r#"{{"user":"id-a","destination_address":"{REGTEST_P2PKH_ADDRESS}","amount_sats":1500,"fee_rate_sats_vb":3}}"#
        )
    );
    assert_eq!(
        serde_json::from_str::<SendUserPayment>(&json).unwrap(),
        payment
    );
}

// ---------------------------------------------------------------------------
// AP-1a traffic capability projection
// ---------------------------------------------------------------------------

// Consumed from frozen producer r4's focused integration tests, with ONLY
// activation outcomes changed by the parent/user atomic J1 decision.
#[test]
fn selected_read_only_recognition_preserves_model_failure_and_unfunded_rows() {
    let mut checks = passing_five_method_checks();
    checks.extend(passing_foundational_checks());
    for row in &mut checks {
        if row["id"] == "sendrawtransaction.recognized" {
            row["behavior"] = json!("not_run");
        } else if row["method"] == "getrawtransaction" {
            row["behavior"] = json!("unknown");
        }
    }
    let report = report_json(checks);
    let bytes = serde_json::to_vec(&report).unwrap();
    let record = project_traffic_capability(&bytes).unwrap();
    assert!(record.recognized);
    assert!(!record.behavior_proven);
    assert_eq!(record.traffic_consumer.as_deref(), Some(TRAFFIC_CONSUMER));
    assert_eq!(record.consumer_model_failures, vec![full_model_failure()]);
    assert_eq!(
        record.check_states["getmempoolinfo"].behavior,
        Ap1aCheckState::Failed
    );
    assert_eq!(
        record.methods["getmempoolinfo"].state,
        Ap1aCheckState::Passed
    );
    assert_eq!(
        record.methods["getmempoolinfo"].check_ids,
        vec!["getmempoolinfo.traffic-consumer"]
    );
    assert_eq!(
        record.report_sha256,
        hex::encode(sha2::Sha256::digest(&bytes))
    );
    assert_eq!(
        record.node_binary_sha256.as_deref(),
        report["node"]["binary_sha256"].as_str()
    );
    assert_eq!(
        record.expected_genesis,
        report["profile"]["expected_genesis"].as_str().unwrap()
    );
    assert_eq!(
        record.observations["traffic_fee_policy"]["minrelaytxfee_btc_kb_lexeme"],
        "0.00001000"
    );
    assert_eq!(
        record.observations["traffic_fee_policy"]["minrelaytxfee_sats_kb"].as_u64(),
        Some(1000)
    );
    assert_eq!(
        record.unproven_check_ids,
        vec![
            "getrawtransaction.blockhash.traffic-consumer",
            "getrawtransaction.bool-verbose",
            "getrawtransaction.bool-verbose.async",
            "getrawtransaction.verbosity0",
            "getrawtransaction.verbosity1",
            "sendrawtransaction.recognized"
        ]
    );
    check_traffic_capability(&satisfied_scenario(), Some(&record)).unwrap();
    let pretty = serde_json::to_vec_pretty(&report).unwrap();
    let pretty_record = project_traffic_capability(&pretty).unwrap();
    assert_ne!(record.report_sha256, pretty_record.report_sha256);
    assert_eq!(record.check_states, pretty_record.check_states);
}

#[test]
fn selected_report_metadata_and_diagnostic_omissions_are_refused() {
    let fixture = || {
        let mut checks = passing_five_method_checks();
        checks.extend(passing_foundational_checks());
        report_json(checks)
    };
    for field in ["probe", "traffic_consumer", "consumer_model_failures"] {
        let mut report = fixture();
        report.as_object_mut().unwrap().remove(field);
        assert!(
            project_traffic_capability(report.to_string().as_bytes()).is_err(),
            "{field}"
        );
    }
    for value in [Value::Null, json!("corepc-v30"), json!(true)] {
        let mut report = fixture();
        report["traffic_consumer"] = value;
        assert!(project_traffic_capability(report.to_string().as_bytes()).is_err());
    }
    for field in ["detail", "method", "availability", "behavior", "fail_kind"] {
        let mut report = fixture();
        report["consumer_model_failures"][0][field] = json!("tampered");
        let error = project_traffic_capability(report.to_string().as_bytes()).unwrap_err();
        assert_eq!(error.code(), "traffic_projection_report_invalid");
        assert!(error.detail().contains("not preserved verbatim"));
    }
    let mut report = fixture();
    report["checks"]
        .as_array_mut()
        .unwrap()
        .retain(|row| row["id"] != "getmempoolinfo");
    assert!(project_traffic_capability(report.to_string().as_bytes()).is_err());
}

#[test]
fn selected_rows_require_exact_methods_presence_and_availability() {
    for id in [
        "getmempoolinfo.traffic-consumer",
        "getrawtransaction.blockhash.traffic-consumer",
        "getblockheader.traffic-consumer",
        "getbestblockhash",
        "getblock.verbosity0",
    ] {
        let mut checks = passing_five_method_checks();
        checks.extend(passing_foundational_checks());
        let index = checks.iter().position(|row| row["id"] == id).unwrap();
        let mut missing = checks.clone();
        missing.remove(index);
        let record = project(missing);
        assert!(!record.recognized, "missing {id}");
        assert!(record.unproven_check_ids.iter().any(|entry| entry == id));
        for availability in ["failed", "unknown"] {
            let mut failed = checks.clone();
            failed[index]["availability"] = json!(availability);
            failed[index]["detail"] =
                json!("not evaluated: oversized: response body exceeds 1048576 bytes");
            let record = project(failed);
            assert!(!record.recognized, "{id}: {availability}");
            assert_eq!(record.consumer_model_failures, vec![full_model_failure()]);
        }
        checks[index]["method"] = json!("other-method");
        assert_eq!(
            project_err(checks).code(),
            "traffic_projection_report_invalid"
        );
    }
    for id in ["getmempoolinfo.traffic-consumer", "getblock.verbosity0"] {
        let mut checks = passing_five_method_checks();
        checks.extend(passing_foundational_checks());
        checks.iter_mut().find(|row| row["id"] == id).unwrap()["behavior"] = json!("unknown");
        assert!(!project(checks).recognized);
    }
}

#[test]
fn immutable_r1_and_r2_reports_remain_non_authorizing_history() {
    let fixtures: &[(&[u8], &str, &str)] = &[
        (
            include_bytes!(
                "../../../testing/ark-rpc/reports/2026-11-08-r1-l2-eb257af7/report.json"
            ),
            "e1151d3e32294d875ae968c5c1066bc0e3104e55653118d0cbab2486698e95a4",
            "11bdb1007e130a749720f87482f287c1fb65d1e743c142fc4b1ca15a782d89e7",
        ),
        (
            include_bytes!(
                "../../../testing/ark-rpc/reports/2026-11-08-r2-l2-eb257af7/report.json"
            ),
            "e834cba45cf3417711e8c0084e5792b2b18dec044782b22c00ffa507282c6d04",
            AP1A_HISTORICAL_R2_CONTRACT_SHA256,
        ),
    ];
    for (bytes, digest, lineage) in fixtures {
        assert_eq!(hex::encode(sha2::Sha256::digest(bytes)), *digest);
        let report: Value = serde_json::from_slice(bytes).unwrap();
        let model = report["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == "getmempoolinfo")
            .unwrap();
        assert_eq!(model, &full_model_failure());
        let record = project_traffic_capability(bytes).unwrap();
        assert_eq!(record.contract_hash, *lineage);
        assert_eq!(record.report_sha256, *digest);
        assert_eq!(
            record.check_states["getmempoolinfo"].behavior,
            Ap1aCheckState::Failed
        );
        assert!(!record.recognized);
        assert_eq!(
            record.methods["getmempoolinfo"].state,
            Ap1aCheckState::NotRun
        );
        assert!(record.traffic_consumer.is_none());
        assert!(check_traffic_capability(&satisfied_scenario(), Some(&record)).is_err());
        let mut borrowed = report;
        borrowed["contract_hash"] = json!(TEST_CONTRACT_HASH);
        assert!(project_traffic_capability(borrowed.to_string().as_bytes()).is_err());
    }
}

#[test]
fn selected_rpc_fixture_keeps_raw_and_verbose_forms_distinct() {
    let bytes = include_bytes!("fixtures/j1/traffic_selected_consumer.json");
    assert_eq!(
        hex::encode(sha2::Sha256::digest(bytes)),
        "05222f30c3aab326310485efc66b8c4b102601aef88049cfc3c60f16cb2176a7"
    );
    let fixture: Value = serde_json::from_slice(bytes).unwrap();
    let overrides = fixture["overrides"].as_array().unwrap();
    let header = overrides
        .iter()
        .find(|row| row["method"] == "getblockheader")
        .unwrap();
    let raw_tx = overrides
        .iter()
        .find(|row| row["method"] == "getrawtransaction")
        .unwrap();
    let genesis = "0f9188f13cb7b2c71f2a335e3a4fc328bf5beb436012afca590b1a11466e2206";
    let txid = "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b";
    assert_eq!(header["response"]["params"], json!([genesis]));
    assert_eq!(raw_tx["response"]["params"], json!([txid, 0, genesis]));
    assert!(raw_tx["response"]["params"][1].is_u64());
    assert_eq!(raw_tx["response"]["error"]["code"], -5);
    let base: Value = serde_json::from_str(include_str!(
        "../../../testing/ark-rpc/fixtures/base_responses.json"
    ))
    .unwrap();
    let entries = base["entries"].as_array().unwrap();
    let raw_block = entries
        .iter()
        .find(|row| row["method"] == "getblock" && row["params"] == json!([genesis, 0]))
        .unwrap();
    let verbose = entries
        .iter()
        .find(|row| row["method"] == "getblock" && row["params"] == json!([genesis]))
        .unwrap();
    assert!(raw_block["result"].is_string());
    assert!(verbose["result"].is_object());
    assert_eq!(verbose["result"]["hash"], genesis);
}

#[test]
fn rpc_policy_contract_history_never_authorizes_the_new_grammar() {
    assert_eq!(
        AP1A_PRE_RPC_POLICY_CONTRACT_SHA256,
        "da87e2b665f4c3427098e308fd87d5946c8fc78d1ffa64c93a1ad0f84b3b5ca8"
    );
    assert!(AP1A_RECOGNIZED_UNAPPROVED_CANDIDATES.contains(&AP1A_PRE_RPC_POLICY_CONTRACT_SHA256));
    let mut record = all_passed_record();
    record.contract_hash = AP1A_PRE_RPC_POLICY_CONTRACT_SHA256.into();
    assert!(record.recognized);
    assert!(!record.behavior_proven);
    assert_eq!(
        check_traffic_capability(&satisfied_scenario(), Some(&record))
            .unwrap_err()
            .code(),
        "traffic_capability_lineage_unapproved"
    );
    assert!(!AP1A_RECOGNIZED_UNAPPROVED_CANDIDATES.contains(&AP1A_TRAFFIC_CONTRACT_SHA256));
    check_traffic_capability(&satisfied_scenario(), Some(&all_passed_record())).unwrap();
}

#[test]
fn rpc_policy_golden_bytes_bind_literals_and_preserve_default_profile() {
    let base_bytes = include_bytes!("fixtures/j1/launch-profile-goldens.json");
    let policy_bytes = include_bytes!("fixtures/j1/launch-profile-rpc-policy-goldens.json");
    assert_eq!(
        hex::encode(sha2::Sha256::digest(policy_bytes)),
        "7d87ad97d2937412a62f5ac4edd8d156a9ea02e600a3a6011ad469da569885ca"
    );
    let base: Value = serde_json::from_slice(base_bytes).unwrap();
    let policy: Value = serde_json::from_slice(policy_bytes).unwrap();
    assert_eq!(
        hex::encode(sha2::Sha256::digest(base_bytes)),
        policy["base_fixture_sha256"]
    );
    assert_eq!(
        base["comparison_sha256"],
        policy["default_comparison_sha256"]
    );
    let default: Value = serde_json::from_str(base["canonical_utf8"].as_str().unwrap()).unwrap();
    let mut digests = std::collections::BTreeSet::new();
    for case in policy["cases"].as_array().unwrap() {
        let tokens = match case["name"].as_str().unwrap() {
            "split" => json!(["--rpc-compat-profile", "bark-v31"]),
            "equals" => json!(["--rpc-compat-profile=bark-v31"]),
            other => panic!("unknown policy form {other}"),
        };
        assert_eq!(case["append_argv"], tokens);
        let bytes = case["canonical_utf8"].as_str().unwrap().as_bytes();
        assert_eq!(
            hex::encode(sha2::Sha256::digest(bytes)),
            case["comparison_sha256"]
        );
        assert_ne!(
            case["comparison_sha256"],
            policy["default_comparison_sha256"]
        );
        assert!(digests.insert(case["comparison_sha256"].as_str().unwrap()));
        let comparison: Value = serde_json::from_slice(bytes).unwrap();
        let mut expected = default.clone();
        for token in tokens.as_array().unwrap() {
            expected["argv"]
                .as_array_mut()
                .unwrap()
                .push(json!({"literal":token}));
        }
        assert_eq!(comparison, expected);
    }
    assert_eq!(digests.len(), 2);
}

#[test]
fn projection_maps_states_as_is_and_recognizes_on_full_pass() {
    let mut checks = passing_five_method_checks();
    checks.extend(passing_foundational_checks());
    checks.extend(version_gate_checks());
    let record = project(checks);
    assert!(record.recognized);
    // Behavior is never proven from a read-only report, by construction.
    assert!(!record.behavior_proven);
    assert!(record.unproven_check_ids.is_empty());
    assert_eq!(record.probe, AP1A_PROBE_ID);
    assert_eq!(record.report_schema_version, 2);
    assert_eq!(record.contract_hash, TEST_CONTRACT_HASH);
    assert_eq!(record.report_sha256.len(), 64);
    for method in TRAFFIC_METHODS {
        let status = record.methods.get(method).unwrap();
        assert_eq!(status.state, Ap1aCheckState::Passed, "method {method}");
        assert!(!status.check_ids.is_empty());
    }
    for id in TRAFFIC_FOUNDATIONAL_CHECKS {
        assert_eq!(
            record.foundational.get(id),
            Some(&Ap1aCheckState::Passed),
            "foundational {id}"
        );
    }
    // Observations, binding metadata and the bark verdict are carried
    // as-is.
    assert_eq!(record.observations["mempool_size"], json!(0));
    assert_eq!(record.target_kind, TRAFFIC_TARGET_KIND_OWNED);
    assert_eq!(
        record.node_binary_sha256.as_deref(),
        Some("dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd")
    );
    assert_eq!(record.verdict.status, "surface_compatible");
    assert_eq!(record.verdict.funded_state, "unproven");
    assert!(!record.verdict.full_acceptance);
}

/// The exact r1 evidence shape on both approved binaries: recognition
/// passed/not_run, getrawtransaction honest unknowns, getmempoolinfo a
/// real shape failure (`maxdatacarriersize` null), version gates
/// failed. The capability is NOT recognized with every state preserved
/// as-is - no coercion, no fabricated funded pass, and no funded
/// broadcast requirement (the funded row is tolerated as not_run).
#[test]
fn projection_preserves_the_r1_evidence_shape() {
    let mut checks = version_gate_checks();
    checks.extend(passing_foundational_checks());
    checks.push(check_row(
        "sendrawtransaction.recognized",
        Some("sendrawtransaction"),
        "passed",
        "not_run",
    ));
    checks.push(check_row(
        "getrawmempool",
        Some("getrawmempool"),
        "passed",
        "passed",
    ));
    checks.push(check_row(
        "getrawmempool.bool-form",
        Some("getrawmempool"),
        "passed",
        "passed",
    ));
    for id in [
        "getrawtransaction.verbosity0",
        "getrawtransaction.verbosity1",
        "getrawtransaction.bool-verbose",
        "getrawtransaction.bool-verbose.async",
    ] {
        checks.push(check_row(
            id,
            Some("getrawtransaction"),
            "passed",
            "unknown",
        ));
    }
    checks.push(full_model_failure());
    for id in [
        "getblock.verbosity0",
        "getblock.verbosity0.rehash",
        "getblock.verbosity1",
        "getblock.singleparam",
    ] {
        checks.push(check_row(id, Some("getblock"), "passed", "passed"));
    }
    let record = project(checks);
    // Missing selected rows block recognition; the full model remains a
    // verbatim diagnostic and does not stand in for the selected size check.
    assert!(!record.recognized);
    assert!(!record.behavior_proven);
    assert_eq!(
        record.methods["sendrawtransaction"].state,
        Ap1aCheckState::NotRun
    );
    assert_eq!(
        record.methods["getrawtransaction"].state,
        Ap1aCheckState::Unknown
    );
    // The failure is preserved, never null-to-zero laundered.
    assert_eq!(
        record.check_states["getmempoolinfo"].behavior,
        Ap1aCheckState::Failed
    );
    assert_eq!(record.consumer_model_failures, vec![full_model_failure()]);
    assert_eq!(
        record.methods["getmempoolinfo"].state,
        Ap1aCheckState::NotRun
    );
    assert_eq!(
        record.methods["getrawmempool"].state,
        Ap1aCheckState::Passed
    );
    assert_eq!(record.methods["getblock"].state, Ap1aCheckState::Passed);
    // The unproven-id list records exactly what remains to be
    // exercised.
    assert_eq!(
        record.unproven_check_ids,
        vec![
            "getmempoolinfo.traffic-consumer".to_string(),
            "getrawtransaction.blockhash.traffic-consumer".to_string(),
            "getrawtransaction.bool-verbose".to_string(),
            "getrawtransaction.bool-verbose.async".to_string(),
            "getrawtransaction.verbosity0".to_string(),
            "getrawtransaction.verbosity1".to_string(),
            "sendrawtransaction.recognized".to_string(),
        ]
    );
}

#[test]
fn projection_availability_not_passed_is_blocking() {
    // availability != passed blocks recognition even when behavior
    // passed: recognition itself is unproven.
    let mut checks = passing_five_method_checks();
    checks.extend(passing_foundational_checks());
    checks.retain(|row| row["id"].as_str() != Some("getrawmempool"));
    checks.push(check_row(
        "getrawmempool",
        Some("getrawmempool"),
        "unknown",
        "passed",
    ));
    let record = project(checks);
    assert!(!record.recognized);
    assert_eq!(
        record.methods["getrawmempool"].state,
        Ap1aCheckState::Unknown
    );
}

#[test]
fn projection_version_gate_failures_are_not_traffic_failures() {
    // Version gates fail exactly as in r1 while the whole traffic
    // surface passes: the projection stays recognized (no Bark version
    // floor).
    let mut checks = passing_five_method_checks();
    checks.extend(passing_foundational_checks());
    checks.extend(version_gate_checks());
    let record = project(checks);
    assert!(record.recognized);
}

#[test]
fn projection_foundational_failures_are_load_bearing() {
    for (id, method, availability, behavior) in [
        ("protocol.id-echo", None, "failed", "passed"),
        (
            "protocol.envelope20.getnetworkinfo",
            Some("getnetworkinfo"),
            "unknown",
            "passed",
        ),
        (
            "getblockchaininfo.network",
            Some("getblockchaininfo"),
            "passed",
            "failed",
        ),
        (
            "getblockhash.genesis",
            Some("getblockhash"),
            "failed",
            "failed",
        ),
        (
            "getblockheader.verbose.genesis",
            Some("getblockheader"),
            "unknown",
            "unknown",
        ),
    ] {
        let mut checks = passing_five_method_checks();
        checks.extend(passing_foundational_checks());
        checks.retain(|row| row["id"].as_str() != Some(id));
        checks.push(check_row(id, method, availability, behavior));
        let record = project(checks);
        assert!(
            !record.recognized,
            "foundational failure on {id} must fail closed"
        );
        assert!(record
            .methods
            .values()
            .all(|status| status.state == Ap1aCheckState::Passed));
    }
}

#[test]
fn projection_foundational_missing_fails_closed() {
    let mut checks = passing_five_method_checks();
    checks.extend(passing_foundational_checks());
    checks.retain(|row| row["id"].as_str() != Some("getblockhash.genesis"));
    let record = project(checks);
    assert!(!record.recognized);
    assert_eq!(
        record.foundational["getblockhash.genesis"],
        Ap1aCheckState::NotRun
    );
}

/// A missing expected id is never tolerated just because the id's
/// flags would tolerate a non-passed behavior axis (F4): the row must
/// exist and be consulted.
#[test]
fn projection_missing_expected_id_blocks_even_when_flagged() {
    let mut checks = passing_five_method_checks();
    checks.extend(passing_foundational_checks());
    checks.retain(|row| row["id"].as_str() != Some("sendrawtransaction.recognized"));
    let record = project(checks);
    assert!(!record.recognized);
    assert_eq!(
        record.methods["sendrawtransaction"].state,
        Ap1aCheckState::NotRun
    );
    assert!(record
        .unproven_check_ids
        .contains(&"sendrawtransaction.recognized".to_string()));
}

/// CE1 (review F3): one forged row per method ("<method>.whatever"),
/// observations null, foreign target/profile - the report itself is
/// rejected: unknown consulted ids and a non-object observations never
/// project.
#[test]
fn projection_rejects_forged_method_rows_and_null_observations() {
    let mut checks = passing_foundational_checks();
    for method in TRAFFIC_METHODS {
        checks.push(check_row(
            &format!("{method}.whatever"),
            Some(method),
            "passed",
            "passed",
        ));
    }
    let err = project_err(checks);
    assert_eq!(err.code(), "traffic_projection_report_invalid");
    assert!(err
        .detail()
        .contains("not in the frozen expected-check table"));

    // observations must be a JSON object, never null or an array.
    for observations in [serde_json::Value::Null, json!([]), json!("x")] {
        let mut report = report_json({
            let mut checks = passing_five_method_checks();
            checks.extend(passing_foundational_checks());
            checks
        });
        report["observations"] = observations;
        let err = project_traffic_capability(report.to_string().as_bytes()).unwrap_err();
        assert_eq!(err.code(), "traffic_projection_report_invalid");
    }
}

/// CE2 (review F3): a row id attributed to the wrong method is
/// rejected by the exact id-method binding.
#[test]
fn projection_rejects_wrong_id_method_binding() {
    let mut checks = passing_five_method_checks();
    checks.extend(passing_foundational_checks());
    checks.retain(|row| row["id"].as_str() != Some("getblock.verbosity1"));
    checks.push(check_row(
        "getblock.verbosity1",
        Some("getrawtransaction"),
        "passed",
        "passed",
    ));
    let err = project_err(checks);
    assert_eq!(err.code(), "traffic_projection_report_invalid");
    assert!(err
        .detail()
        .contains("frozen expected-check table binds it"));

    // Same for a foundational id claiming a traffic method.
    let mut checks = passing_five_method_checks();
    checks.extend(passing_foundational_checks());
    checks.retain(|row| row["id"].as_str() != Some("protocol.id-echo"));
    checks.push(check_row(
        "protocol.id-echo",
        Some("getblock"),
        "passed",
        "passed",
    ));
    assert_eq!(
        project_err(checks).code(),
        "traffic_projection_report_invalid"
    );
}

/// An extra check on a traffic method is NOT a projection extension:
/// unknown consulted ids fail closed (the frozen table is the
/// contract; extensions arrive only through a recorded decision).
#[test]
fn projection_unknown_consulted_id_fails_closed() {
    let mut checks = passing_five_method_checks();
    checks.extend(passing_foundational_checks());
    checks.push(check_row(
        "getmempoolinfo.extra-form",
        Some("getmempoolinfo"),
        "passed",
        "unknown",
    ));
    let err = project_err(checks);
    assert_eq!(err.code(), "traffic_projection_report_invalid");
    assert!(err.detail().contains("getmempoolinfo.extra-form"));
}

#[test]
fn projection_fail_closed_on_lineage_and_shape() {
    let full_checks = || {
        let mut checks = passing_five_method_checks();
        checks.extend(passing_foundational_checks());
        checks
    };

    // Wrong probe id.
    let mut report = report_json(full_checks());
    report["probe"] = json!("some-other-probe");
    let err = project_traffic_capability(report.to_string().as_bytes()).unwrap_err();
    assert_eq!(err.code(), "traffic_projection_provenance_invalid");

    // Wrong report schema version (v1 frozen evidence is not consumable).
    for version in [1, 3] {
        let mut report = report_json(full_checks());
        report["schema_version"] = json!(version);
        let err = project_traffic_capability(report.to_string().as_bytes()).unwrap_err();
        assert_eq!(err.code(), "traffic_projection_schema_unsupported");
    }

    // Duplicate check id.
    let mut checks = full_checks();
    checks.push(check_row(
        "getrawmempool",
        Some("getrawmempool"),
        "passed",
        "passed",
    ));
    assert_eq!(
        project_err(checks).code(),
        "traffic_projection_duplicate_check"
    );

    // Malformed contract hash.
    let mut report = report_json(full_checks());
    report["contract_hash"] = json!("not-a-hash");
    let err = project_traffic_capability(report.to_string().as_bytes()).unwrap_err();
    assert_eq!(err.code(), "traffic_projection_report_invalid");

    // Strict provenance: empty/whitespace probe version and run token,
    // and a generated_utc that is not a strict UTC timestamp.
    for (field, value) in [
        ("probe_version", json!("")),
        ("probe_version", json!(" ")),
        ("run_token", json!("")),
        ("generated_utc", json!("2026-11-08")),
        ("generated_utc", json!("2026-11-08T00:00:00")),
        ("generated_utc", json!("2026-11-08T00:00:00+00:00")),
        ("generated_utc", json!("")),
    ] {
        let mut report = report_json(full_checks());
        report[field] = value.clone();
        let err = project_traffic_capability(report.to_string().as_bytes()).unwrap_err();
        assert_eq!(
            err.code(),
            "traffic_projection_report_invalid",
            "{field} = {value}"
        );
    }

    // Target/profile/node shape: unknown target kind, malformed
    // genesis, malformed node binary hash, missing profile field.
    let mut report = report_json(full_checks());
    report["target"]["kind"] = json!("somewhere-else");
    assert_eq!(
        project_traffic_capability(report.to_string().as_bytes())
            .unwrap_err()
            .code(),
        "traffic_projection_report_invalid"
    );
    let mut report = report_json(full_checks());
    report["profile"]["expected_genesis"] = json!("00");
    assert_eq!(
        project_traffic_capability(report.to_string().as_bytes())
            .unwrap_err()
            .code(),
        "traffic_projection_report_invalid"
    );
    let mut report = report_json(full_checks());
    report["node"]["binary_sha256"] = json!("zz");
    assert_eq!(
        project_traffic_capability(report.to_string().as_bytes())
            .unwrap_err()
            .code(),
        "traffic_projection_report_invalid"
    );
    let mut report = report_json(full_checks());
    report["profile"]
        .as_object_mut()
        .unwrap()
        .remove("expected_network");
    assert_eq!(
        project_traffic_capability(report.to_string().as_bytes())
            .unwrap_err()
            .code(),
        "traffic_projection_report_invalid"
    );

    // Unknown status values on consulted rows.
    for (availability, behavior) in [
        ("maybe", "passed"),
        ("passed", "banana"),
        ("not_run", "passed"),
    ] {
        let mut checks = full_checks();
        checks.retain(|row| row["id"].as_str() != Some("getrawmempool"));
        checks.push(check_row(
            "getrawmempool",
            Some("getrawmempool"),
            availability,
            behavior,
        ));
        assert_eq!(
            project_err(checks).code(),
            "traffic_projection_state_invalid"
        );
    }

    // Read-only verdict invariants.
    for (funded_state, full_acceptance) in [("proven", false), ("unproven", true)] {
        let mut report = report_json(full_checks());
        report["verdict"]["funded_state"] = json!(funded_state);
        report["verdict"]["full_acceptance"] = json!(full_acceptance);
        let err = project_traffic_capability(report.to_string().as_bytes()).unwrap_err();
        assert_eq!(err.code(), "traffic_projection_report_invalid");
    }

    // Unparseable / incomplete report.
    let err = project_traffic_capability(b"{ not json").unwrap_err();
    assert_eq!(err.code(), "traffic_projection_report_invalid");
    let mut report = report_json(full_checks());
    report.as_object_mut().unwrap().remove("observations");
    let err = project_traffic_capability(report.to_string().as_bytes()).unwrap_err();
    assert_eq!(err.code(), "traffic_projection_report_invalid");
}

#[test]
fn projection_record_roundtrips_through_the_manifest_field() {
    let mut checks = passing_five_method_checks();
    checks.extend(passing_foundational_checks());
    let record = project(checks);
    let json = serde_json::to_value(&record).unwrap();
    let roundtripped: TrafficCapabilityRecord = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(record, roundtripped);
    // deny_unknown_fields: an extra key inside the record is rejected.
    let mut tampered = json;
    tampered["extra"] = json!(1);
    assert!(serde_json::from_value::<TrafficCapabilityRecord>(tampered).is_err());
}

// ---------------------------------------------------------------------------
// Scenario preflight gate (production: approved lineage allowlist empty)
// ---------------------------------------------------------------------------

fn all_passed_record() -> TrafficCapabilityRecord {
    let mut checks = passing_five_method_checks();
    checks.extend(passing_foundational_checks());
    project(checks)
}

fn r1_shaped_record() -> TrafficCapabilityRecord {
    let mut checks = version_gate_checks();
    checks.extend(passing_foundational_checks());
    checks.push(check_row(
        "sendrawtransaction.recognized",
        Some("sendrawtransaction"),
        "passed",
        "not_run",
    ));
    checks.push(check_row(
        "getrawmempool",
        Some("getrawmempool"),
        "passed",
        "passed",
    ));
    checks.push(check_row(
        "getrawmempool.bool-form",
        Some("getrawmempool"),
        "passed",
        "passed",
    ));
    for id in [
        "getrawtransaction.verbosity0",
        "getrawtransaction.verbosity1",
        "getrawtransaction.bool-verbose",
        "getrawtransaction.bool-verbose.async",
    ] {
        checks.push(check_row(
            id,
            Some("getrawtransaction"),
            "passed",
            "unknown",
        ));
    }
    checks.push(check_row(
        "getmempoolinfo",
        Some("getmempoolinfo"),
        "passed",
        "failed",
    ));
    for id in [
        "getblock.verbosity0",
        "getblock.verbosity0.rehash",
        "getblock.verbosity1",
        "getblock.singleparam",
    ] {
        checks.push(check_row(id, Some("getblock"), "passed", "passed"));
    }
    project(checks)
}

#[test]
fn check_traffic_capability_gates_scenarios() {
    let traffic_scenario = satisfied_scenario();

    // No traffic declared: no gate, with or without a record.
    let plain = common::fixture_scenario("scenario_v1_two_pools.json");
    check_traffic_capability(&plain, None).unwrap();
    check_traffic_capability(&plain, Some(&all_passed_record())).unwrap();

    // Traffic declared without any recorded capability: rejected with
    // the dedicated unrecorded code, before any process starts.
    let err = check_traffic_capability(&traffic_scenario, None).unwrap_err();
    assert_eq!(err.code(), "traffic_capability_unrecorded");

    // Activation-specific expectation: the complete r4 fixture now passes
    // the authorized production lineage gate, still without funded proof.
    check_traffic_capability(&traffic_scenario, Some(&all_passed_record())).unwrap();
    assert!(!all_passed_record().behavior_proven);
    let mut unknown = all_passed_record();
    unknown.contract_hash = "cc".repeat(32);
    assert_eq!(
        check_traffic_capability(&traffic_scenario, Some(&unknown))
            .unwrap_err()
            .code(),
        "traffic_capability_lineage_unknown"
    );

    // Missing selected rows fail recognition under the current lineage.
    let err = check_traffic_capability(&traffic_scenario, Some(&r1_shaped_record())).unwrap_err();
    assert_eq!(err.code(), "traffic_capability_unrecognized");

    // A recognized-but-unapproved candidate hash (the superseded r1
    // contract) is named as such in the diagnostics and still never
    // accepted.
    let mut candidate = all_passed_record();
    candidate.contract_hash = AP1A_RECOGNIZED_UNAPPROVED_CANDIDATES[0].to_string();
    let err = check_traffic_capability(&traffic_scenario, Some(&candidate)).unwrap_err();
    assert_eq!(err.code(), "traffic_capability_lineage_unapproved");
    assert!(err.detail().contains("recognized but unapproved"));

    // The production accept path: the authorized r4 lineage plus a fully
    // passing, recognized record passes the gate.
    let mut approved = all_passed_record();
    approved.contract_hash = AP1A_APPROVED_CONTRACT_SHA256S[0].to_string();
    check_traffic_capability(&traffic_scenario, Some(&approved)).unwrap();

    // The approved lineage does not rescue an unrecognized record: the
    // r1 evidence shape (getmempoolinfo failed) still blocks at the
    // recognized stage - the honest current outcome, never laundered.
    let mut approved_r1_shape = r1_shaped_record();
    approved_r1_shape.contract_hash = AP1A_APPROVED_CONTRACT_SHA256S[0].to_string();
    let err = check_traffic_capability(&traffic_scenario, Some(&approved_r1_shape)).unwrap_err();
    assert_eq!(err.code(), "traffic_capability_unrecognized");
    assert!(err.detail().contains("getmempoolinfo=not_run"));
    let mut historical = all_passed_record();
    historical.contract_hash = AP1A_HISTORICAL_R2_CONTRACT_SHA256.into();
    assert_eq!(
        check_traffic_capability(&traffic_scenario, Some(&historical))
            .unwrap_err()
            .code(),
        "traffic_capability_contract_historical"
    );
}

#[test]
fn check_traffic_capability_fails_closed_on_tampered_records() {
    let traffic_scenario = satisfied_scenario();

    // CE3 (review F3): a hand-built record with empty check_ids, a
    // wrong report schema version and empty provenance strings is
    // rejected - the gate re-validates the record, not just key sets.
    let hand_built: TrafficCapabilityRecord = serde_json::from_value(json!({
        "schema_version": 1,
        "probe": AP1A_PROBE_ID,
        "probe_version": "",
        "report_schema_version": 99,
        "report_sha256": "",
        "contract_hash": TEST_CONTRACT_HASH,
        "generated_utc": "",
        "run_token": "",
        "target_label": "x",
        "target_kind": TRAFFIC_TARGET_KIND_OWNED,
        "profile_name": "x",
        "expected_network": "regtest",
        "expected_genesis":
            "0f9188f13cb7b2c71f2a335e3a4fc328bf5beb436012afca590b1a11433e0ad2",
        "node_binary_sha256": null,
        "traffic_consumer": TRAFFIC_CONSUMER,
        "consumer_model_failures": [],
        "methods": {
            "sendrawtransaction": {"state": "passed", "check_ids": []},
            "getrawmempool": {"state": "passed", "check_ids": []},
            "getrawtransaction": {"state": "passed", "check_ids": []},
            "getmempoolinfo": {"state": "passed", "check_ids": []},
            "getblock": {"state": "passed", "check_ids": []}
        },
        "foundational": {
            "protocol.id-echo": "passed",
            "protocol.envelope20.getnetworkinfo": "passed",
            "getblockchaininfo.network": "passed",
            "getblockhash.genesis": "passed",
            "getblockheader.verbose.genesis": "passed"
        },
        "check_states": {},
        "observations": {},
        "verdict": {
            "status": "surface_compatible",
            "funded_state": "unproven",
            "full_acceptance": false
        },
        "recognized": true,
        "behavior_proven": false,
        "unproven_check_ids": []
    }))
    .unwrap();
    let err = check_traffic_capability(&traffic_scenario, Some(&hand_built)).unwrap_err();
    assert_eq!(err.code(), "traffic_capability_record_invalid");

    // A record claiming recognized while its rows recompute otherwise.
    let mut tampered = r1_shaped_record();
    tampered.recognized = true;
    let err = check_traffic_capability(&traffic_scenario, Some(&tampered)).unwrap_err();
    assert_eq!(err.code(), "traffic_capability_record_invalid");

    // A record claiming behavior_proven (never constructible from a
    // read-only projection).
    let mut proven = all_passed_record();
    proven.behavior_proven = true;
    let err = check_traffic_capability(&traffic_scenario, Some(&proven)).unwrap_err();
    assert_eq!(err.code(), "traffic_capability_record_invalid");

    // Wrong probe and wrong record schema version.
    let mut probe = all_passed_record();
    probe.probe = "other".into();
    assert_eq!(
        check_traffic_capability(&traffic_scenario, Some(&probe))
            .unwrap_err()
            .code(),
        "traffic_capability_record_invalid"
    );
    let mut schema = all_passed_record();
    schema.schema_version = 2;
    assert_eq!(
        check_traffic_capability(&traffic_scenario, Some(&schema))
            .unwrap_err()
            .code(),
        "traffic_capability_record_invalid"
    );
    let mut report_schema = all_passed_record();
    report_schema.report_schema_version = 99;
    assert_eq!(
        check_traffic_capability(&traffic_scenario, Some(&report_schema))
            .unwrap_err()
            .code(),
        "traffic_capability_record_invalid"
    );

    // Empty/short provenance and malformed hashes.
    let mut short_sha = all_passed_record();
    short_sha.report_sha256 = "abc".into();
    assert_eq!(
        check_traffic_capability(&traffic_scenario, Some(&short_sha))
            .unwrap_err()
            .code(),
        "traffic_capability_record_invalid"
    );
    let mut bad_time = all_passed_record();
    bad_time.generated_utc = "yesterday".into();
    assert_eq!(
        check_traffic_capability(&traffic_scenario, Some(&bad_time))
            .unwrap_err()
            .code(),
        "traffic_capability_record_invalid"
    );
    let mut bad_hash = all_passed_record();
    bad_hash.contract_hash = "not-hex".into();
    assert_eq!(
        check_traffic_capability(&traffic_scenario, Some(&bad_hash))
            .unwrap_err()
            .code(),
        "traffic_capability_record_invalid"
    );

    // Method check_ids emptied (CE3 aggregate forgery).
    let mut methods = all_passed_record();
    methods
        .methods
        .get_mut("getblock")
        .unwrap()
        .check_ids
        .clear();
    assert_eq!(
        check_traffic_capability(&traffic_scenario, Some(&methods))
            .unwrap_err()
            .code(),
        "traffic_capability_record_invalid"
    );

    // An unknown id smuggled into the full check rows.
    let mut extra_row = all_passed_record();
    extra_row.check_states.insert(
        "getblock.whatever".to_string(),
        spc_lab::traffic::projection::TrafficCheckStatus {
            availability: Ap1aCheckState::Passed,
            behavior: Ap1aCheckState::Passed,
        },
    );
    // Keep the derived views consistent so the id check is what fires.
    extra_row.recognized = extra_row.recompute_recognized();
    extra_row.unproven_check_ids = extra_row.recompute_unproven_ids();
    assert_eq!(
        check_traffic_capability(&traffic_scenario, Some(&extra_row))
            .unwrap_err()
            .code(),
        "traffic_capability_record_invalid"
    );

    // A merged method state that disagrees with the full rows.
    let mut merged = all_passed_record();
    merged.methods.get_mut("getblock").unwrap().state = Ap1aCheckState::Failed;
    assert_eq!(
        check_traffic_capability(&traffic_scenario, Some(&merged))
            .unwrap_err()
            .code(),
        "traffic_capability_record_invalid"
    );

    // A tampered unproven-id list.
    let mut unproven = all_passed_record();
    unproven.unproven_check_ids = vec!["getmempoolinfo".to_string()];
    assert_eq!(
        check_traffic_capability(&traffic_scenario, Some(&unproven))
            .unwrap_err()
            .code(),
        "traffic_capability_record_invalid"
    );
}

#[test]
fn check_traffic_capability_rejects_unbound_targets() {
    let traffic_scenario = satisfied_scenario();

    // A remote-url target says nothing about the run's own nodes.
    let mut remote = all_passed_record();
    remote.target_kind = TRAFFIC_TARGET_KIND_REMOTE.to_string();
    let err = check_traffic_capability(&traffic_scenario, Some(&remote)).unwrap_err();
    assert_eq!(err.code(), "traffic_capability_target_unbound");

    // A non-regtest network cannot be bound to the lab's capability.
    let mut mainnet = all_passed_record();
    mainnet.expected_network = "mainnet".to_string();
    let err = check_traffic_capability(&traffic_scenario, Some(&mainnet)).unwrap_err();
    assert_eq!(err.code(), "traffic_capability_target_unbound");
}

#[test]
fn check_traffic_capability_rejects_hand_built_unknown_capability() {
    // A scenario object built directly (not via parse) with an unknown
    // capability is still rejected by the gate.
    let mut scenario = satisfied_scenario();
    scenario.traffic.as_mut().unwrap().required_capability = "spc.node.regtest.other.v9".into();
    let err = check_traffic_capability(&scenario, Some(&all_passed_record())).unwrap_err();
    assert_eq!(err.code(), "traffic_capability_unknown");
}

/// The frozen expected-check table is EXACTLY the r4 canonical CHECKS for
/// its 20 selected/foundational rows, consumed from the hash-verified producer.
/// This is the atomicity guard: the parent/user-authorized hash and this
/// table change together through a recorded decision or not at all -
/// a string-only hash substitution that leaves the table behind (or
/// vice versa) fails here.
#[test]
fn frozen_expected_checks_table_is_the_authorized_r4_contract() {
    assert_eq!(TRAFFIC_EXPECTED_CHECKS.len(), 20);
    let table: &[(&str, Option<&str>, &str, bool)] = &[
        // Five traffic methods.
        (
            "sendrawtransaction.recognized",
            Some("sendrawtransaction"),
            "funded",
            false,
        ),
        ("getrawmempool", Some("getrawmempool"), "required", false),
        (
            "getrawmempool.bool-form",
            Some("getrawmempool"),
            "encoding",
            false,
        ),
        (
            "getrawtransaction.verbosity0",
            Some("getrawtransaction"),
            "required",
            true,
        ),
        (
            "getrawtransaction.verbosity1",
            Some("getrawtransaction"),
            "required",
            true,
        ),
        (
            "getrawtransaction.bool-verbose",
            Some("getrawtransaction"),
            "encoding",
            true,
        ),
        (
            "getrawtransaction.bool-verbose.async",
            Some("getrawtransaction"),
            "encoding",
            true,
        ),
        (
            "getmempoolinfo.traffic-consumer",
            Some("getmempoolinfo"),
            "traffic",
            false,
        ),
        ("getblock.verbosity0", Some("getblock"), "required", false),
        (
            "getblock.verbosity0.rehash",
            Some("getblock"),
            "required",
            false,
        ),
        ("getblock.verbosity1", Some("getblock"), "required", false),
        ("getblock.singleparam", Some("getblock"), "encoding", false),
        // Foundational rows.
        (
            "getrawtransaction.blockhash.traffic-consumer",
            Some("getrawtransaction"),
            "traffic",
            true,
        ),
        (
            "getblockheader.traffic-consumer",
            Some("getblockheader"),
            "traffic",
            false,
        ),
        (
            "getbestblockhash",
            Some("getbestblockhash"),
            "observational",
            false,
        ),
        ("protocol.id-echo", None, "protocol", false),
        (
            "protocol.envelope20.getnetworkinfo",
            Some("getnetworkinfo"),
            "encoding",
            false,
        ),
        (
            "getblockchaininfo.network",
            Some("getblockchaininfo"),
            "required",
            false,
        ),
        (
            "getblockhash.genesis",
            Some("getblockhash"),
            "required",
            false,
        ),
        (
            "getblockheader.verbose.genesis",
            Some("getblockheader"),
            "required",
            false,
        ),
    ];
    assert_eq!(table.len(), 20);
    for (entry, (id, method, coverage, unknown_behavior_ok)) in
        TRAFFIC_EXPECTED_CHECKS.iter().zip(table)
    {
        assert_eq!(entry.id, *id);
        assert_eq!(entry.method, *method);
        assert_eq!(entry.coverage, *coverage);
        assert_eq!(entry.unknown_behavior_ok, *unknown_behavior_ok);
    }
    // Atomic parent/user activation, not a claim of a new Fable verdict.
    assert_eq!(
        AP1A_APPROVED_CONTRACT_SHA256S,
        &["754e1e6131955f9e5bbce7ef81b6f9aabaa2f1379ec6409f1c3a7a9e88be91f9"]
    );
    // The only behavior-tolerated rows are exactly the canonical flags:
    // the four getrawtransaction unknown_behavior_ok rows and the funded
    // sendrawtransaction.recognized row.
    let tolerated: Vec<&str> = TRAFFIC_EXPECTED_CHECKS
        .iter()
        .filter(|entry| entry.behavior_tolerated())
        .map(|entry| entry.id)
        .collect();
    assert_eq!(
        tolerated,
        vec![
            "sendrawtransaction.recognized",
            "getrawtransaction.verbosity0",
            "getrawtransaction.verbosity1",
            "getrawtransaction.bool-verbose",
            "getrawtransaction.bool-verbose.async",
            "getrawtransaction.blockhash.traffic-consumer",
        ]
    );
}

// ---------------------------------------------------------------------------
// Engine wiring (F1): the gate runs before any effect
// ---------------------------------------------------------------------------

fn engine_options(dir: &common::TestDir, config: LabConfig) -> EngineOptions {
    EngineOptions {
        run_dir: dir.path().to_path_buf(),
        config,
        base_revision: "test-base".to_string(),
        dirty_tree_hash: "clean".to_string(),
        owner_instance: format!("test-{}", std::process::id()),
        evidence_capacity: 100_000,
        namespaces: std::collections::BTreeMap::new(),
        nsenter: None,
        user_namespace: None,
        user_namespace_owner: None,
        namespace_owners: std::collections::BTreeMap::new(),
        namespace_broker: None,
        lifetime_helper: None,
        addresses: std::collections::BTreeMap::new(),
        link_backend: None,
        operator_ui: false,
    }
}

/// A fixture config whose node adapter DECLAREDS the traffic capability
/// name - the review's fail-open reproduction setup. The name-based
/// preflight passes; the traffic gate must still reject.
fn name_declaring_config() -> LabConfig {
    let mut config = common::fixture_config();
    config
        .nodes
        .get_mut("fixture-node")
        .unwrap()
        .capabilities
        .push(TRAFFIC_CAPABILITY.to_string());
    config
}

#[test]
fn engine_prepare_rejects_traffic_before_any_run_artifact() {
    let dir = common::TestDir::new("traffic-gate-prepare");

    // Baseline: the fixture adapter does not declare the traffic
    // capability name, so preflight rejects first (capability names,
    // by coincidence of the list) - also before any effect.
    let err = RunEngine::prepare(
        parse(&scenario_json_with_traffic(|_| {})).unwrap(),
        engine_options(&dir, common::fixture_config()),
    )
    .unwrap_err();
    assert_eq!(err.code(), "capability_unavailable");
    assert!(!dir.path().join("run-manifest.json").exists());

    // The fail-open reproduction: the adapter declares the name, the
    // name-based preflight passes - and the traffic gate rejects
    // before create_dir_all, so the run directory stays empty.
    let err = RunEngine::prepare(
        parse(&scenario_json_with_traffic(|_| {})).unwrap(),
        engine_options(&dir, name_declaring_config()),
    )
    .unwrap_err();
    assert_eq!(err.code(), "traffic_probe_unconfigured");
    assert!(!dir.path().join("run-manifest.json").exists());
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        0,
        "no filesystem, lock or authority effect may happen before the gate"
    );

    // No-traffic scenarios prepare exactly as before.
    let engine = common::prepare_engine(&dir, "scenario_v1_two_pools.json");
    assert!(engine.run_dir.join("run-manifest.json").exists());
}

#[test]
fn engine_attach_gates_traffic_on_the_adopt_path() {
    let dir = common::TestDir::new("traffic-gate-adopt");
    // Prepare a plain run: full run-dir skeleton, no traffic.
    let engine = common::prepare_engine(&dir, "scenario_v1_two_pools.json");
    let run_dir = engine.run_dir.clone();
    let manifest_path = run_dir.join("run-manifest.json");
    let mut manifest = RunManifest::load(&manifest_path).unwrap();

    // Rewrite the private scenario with a traffic-bearing scenario
    // (same run id) and the runtime so the node adapter declares the
    // capability name: simulating a run a T1 runtime would have
    // recorded a capability against.
    let traffic_json = scenario_json_with_traffic(|_| {});
    std::fs::write(&manifest.artifact_paths.private_scenario, &traffic_json).unwrap();
    manifest.scenario_sha256 = hex::encode(sha2::Sha256::digest(traffic_json.as_bytes()));
    std::fs::write(
        &manifest.artifact_paths.private_runtime,
        serde_json::to_vec_pretty(&name_declaring_config()).unwrap(),
    )
    .unwrap();
    manifest.save(&manifest_path).unwrap();
    drop(engine); // release the owner lock

    // Adopt with no recorded capability: rejected before the lock.
    let err = RunEngine::attach(&run_dir, "adopt-test").unwrap_err();
    assert_eq!(err.code(), "traffic_capability_unrecorded");

    // Adopt with a production (unknown-lineage) record: rejected on
    // lineage, even though every row passed.
    let mut manifest = RunManifest::load(&manifest_path).unwrap();
    let mut unknown_lineage = all_passed_record();
    unknown_lineage.contract_hash = "cc".repeat(32);
    manifest.traffic_capability = Some(unknown_lineage);
    manifest.save(&manifest_path).unwrap();
    let err = RunEngine::attach(&run_dir, "adopt-test").unwrap_err();
    assert_eq!(err.code(), "traffic_capability_lineage_unknown");

    // Adopt with a forged (CE3-shaped) record: rejected as invalid.
    let forged: TrafficCapabilityRecord = serde_json::from_value(json!({
        "schema_version": 1,
        "probe": AP1A_PROBE_ID,
        "probe_version": "1.0.0",
        "report_schema_version": 2,
        "report_sha256":
            "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        "contract_hash": TEST_CONTRACT_HASH,
        "generated_utc": "2026-11-08T00:00:00Z",
        "run_token": "AP1a-test",
        "target_label": "x",
        "target_kind": TRAFFIC_TARGET_KIND_OWNED,
        "profile_name": "x",
        "expected_network": "regtest",
        "expected_genesis":
            "0f9188f13cb7b2c71f2a335e3a4fc328bf5beb436012afca590b1a11433e0ad2",
        "node_binary_sha256": null,
        "traffic_consumer": TRAFFIC_CONSUMER,
        "consumer_model_failures": [],
        "methods": {
            "sendrawtransaction": {"state": "passed", "check_ids": ["x"]},
            "getrawmempool": {"state": "passed", "check_ids": ["x"]},
            "getrawtransaction": {"state": "passed", "check_ids": ["x"]},
            "getmempoolinfo": {"state": "passed", "check_ids": ["x"]},
            "getblock": {"state": "passed", "check_ids": ["x"]}
        },
        "foundational": {
            "protocol.id-echo": "passed",
            "protocol.envelope20.getnetworkinfo": "passed",
            "getblockchaininfo.network": "passed",
            "getblockhash.genesis": "passed",
            "getblockheader.verbose.genesis": "passed"
        },
        "check_states": {},
        "observations": {},
        "verdict": {
            "status": "surface_compatible",
            "funded_state": "unproven",
            "full_acceptance": false
        },
        "recognized": true,
        "behavior_proven": false,
        "unproven_check_ids": []
    }))
    .unwrap();
    let mut manifest = RunManifest::load(&manifest_path).unwrap();
    manifest.traffic_capability = Some(forged);
    manifest.save(&manifest_path).unwrap();
    let err = RunEngine::attach(&run_dir, "adopt-test").unwrap_err();
    assert_eq!(err.code(), "traffic_capability_record_invalid");

    // Approved lineage and a recognized projection satisfy the T0 gate, but
    // cannot replace J2's actual probe/profile binding on adoption. This
    // fixture has no owned probe report or verified live node launch.
    let mut manifest = RunManifest::load(&manifest_path).unwrap();
    let mut approved = all_passed_record();
    approved.contract_hash = AP1A_APPROVED_CONTRACT_SHA256S[0].to_string();
    manifest.traffic_capability = Some(approved);
    manifest.save(&manifest_path).unwrap();
    check_traffic_capability(
        &parse(&traffic_json).unwrap(),
        manifest.traffic_capability.as_ref(),
    )
    .unwrap();
    let before_manifest = std::fs::read(&manifest_path).unwrap();
    let before_commands = std::fs::read(&manifest.artifact_paths.private_commands).unwrap();
    let err = RunEngine::attach(&run_dir, "adopt-test").unwrap_err();
    assert_eq!(err.code(), "traffic_probe_binding");
    assert_eq!(std::fs::read(&manifest_path).unwrap(), before_manifest);
    assert_eq!(
        std::fs::read(&manifest.artifact_paths.private_commands).unwrap(),
        before_commands
    );

    // Read-only attach (review path) never runs the gate and keeps
    // working on the same run directory.
    RunEngine::attach_read_only(&run_dir, "review-test").unwrap();
}
