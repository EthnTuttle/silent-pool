//! EXP-OP operator runtime wiring: the hub inventory exposes a typed
//! `operator_ui` interface for a front only when the run planned the
//! `front.<id>.operator` listener (`--operator-ui`), the runtime's
//! start-time probe recorded `page_served: true`, and the endpoint is a
//! host loopback address. The projection itself performs no network
//! request; everything comes from the recorded evidence.

mod common;

use serde_json::Value;
use spc_lab::evidence::{ActorKind, EvidenceActor, EvidencePayload, UnknownEvidence};
use spc_lab::ids::SourceId;
use spc_lab::RunEngine;

/// Record the same evidence fact `Runtime::publish_operator_urls` records
/// after its bounded page probe.
fn operator_fact(engine: &RunEngine, front: &str, page_served: bool) {
    engine.evidence().append_at(
        SourceId::trusted("operator"),
        "operator-readiness-1",
        EvidenceActor::new(ActorKind::Observer, "runtime-operator"),
        None,
        EvidencePayload::Unknown(UnknownEvidence {
            topic: "operator_observation".to_string(),
            detail: serde_json::json!({
                "front": front,
                "listen": "127.0.0.1:48181",
                "loopback": true,
                "page_served": page_served,
                "observation_route": null,
            })
            .to_string(),
        }),
        1_000,
    );
}

fn inventory_json(engine: &RunEngine) -> Value {
    serde_json::to_value(engine.lab_inventory()).unwrap()
}

fn operator_interface(inventory: &Value) -> Option<&Value> {
    inventory["groups"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|group| group["rows"].as_array().unwrap())
        .find(|row| row["id"] == "front-front-a1")
        .unwrap()["interfaces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["kind"] == "operator_ui")
}

#[test]
fn operator_interface_is_typed_and_matches_the_planned_listener() {
    let dir = common::TestDir::new("op-ui-live");
    let mut scenario = common::fixture_scenario("scenario_v1_one_pool_two_miners.json");
    scenario.fronts[0].listen.api = Some("127.0.0.1:0".to_string());
    let engine =
        common::prepare_engine_with_operator_ui(&dir, scenario, common::fixture_config(), true);
    let listener = engine
        .resources_manifest()
        .listeners
        .get("front.front-a1.operator")
        .expect("operator listener is planned when opted in")
        .clone();
    operator_fact(&engine, "front-a1", true);

    let inventory = inventory_json(&engine);
    let interface = operator_interface(&inventory).expect("served operator interface exists");
    assert_eq!(interface["label"], "Pool operator page");
    assert_eq!(
        interface["url"].as_str().unwrap(),
        format!("http://{}:{}/", listener.host, listener.port)
    );
    // The interface is typed; it is never relabeled as the generic ui kind.
    assert_ne!(interface["kind"], "ui");
}

#[test]
fn operator_interface_requires_a_served_proof_not_a_process_state() {
    let dir = common::TestDir::new("op-ui-not-served");
    let mut scenario = common::fixture_scenario("scenario_v1_one_pool_two_miners.json");
    scenario.fronts[0].listen.api = Some("127.0.0.1:0".to_string());
    let engine =
        common::prepare_engine_with_operator_ui(&dir, scenario, common::fixture_config(), true);
    // The listener exists but the runtime's probe recorded page_served:
    // false - no interface may be published on configuration alone.
    operator_fact(&engine, "front-a1", false);
    assert!(operator_interface(&inventory_json(&engine)).is_none());
}

#[test]
fn operator_interface_requires_the_opted_in_listener() {
    let dir = common::TestDir::new("op-ui-not-planned");
    let mut scenario = common::fixture_scenario("scenario_v1_one_pool_two_miners.json");
    scenario.fronts[0].listen.api = Some("127.0.0.1:0".to_string());
    let engine =
        common::prepare_engine_with_operator_ui(&dir, scenario, common::fixture_config(), false);
    // A stray fact without the planned listener (impossible in a real run)
    // still publishes nothing: evidence alone is not a configuration.
    operator_fact(&engine, "front-a1", true);
    assert!(operator_interface(&inventory_json(&engine)).is_none());
}

#[test]
fn namespaced_operator_url_is_never_a_host_browser_link() {
    let dir = common::TestDir::new("op-ui-namespaced");
    let mut scenario = common::fixture_scenario("scenario_v1_one_pool_two_miners.json");
    scenario.fronts[0].listen.api = Some("127.0.0.1:0".to_string());
    let engine =
        common::prepare_engine_with_operator_ui(&dir, scenario, common::fixture_config(), true);
    operator_fact(&engine, "front-a1", true);
    engine
        .record_placement(serde_json::json!({"published": "test-only"}))
        .unwrap();
    let inventory = inventory_json(&engine);
    assert_eq!(inventory["topology"], "namespaces");
    assert!(operator_interface(&inventory).is_none());
    assert!(!inventory.to_string().contains("http://127.0.0.1:"));
}

#[test]
fn recorded_serve_keeps_the_interface_and_the_page_unlinks_it_client_side() {
    // Parity with the api_json/metrics behavior: a recorded (read-only)
    // serve still reports the interface, and the lab.js link guard is what
    // removes the stale historical listener link (covered by the JS guard
    // tests); the JSON itself makes no reachability claim.
    let dir = common::TestDir::new("op-ui-recorded");
    let mut scenario = common::fixture_scenario("scenario_v1_one_pool_two_miners.json");
    scenario.fronts[0].listen.api = Some("127.0.0.1:0".to_string());
    let engine =
        common::prepare_engine_with_operator_ui(&dir, scenario, common::fixture_config(), true);
    operator_fact(&engine, "front-a1", true);
    engine.start().unwrap();
    engine.stop().unwrap();
    let recorded = RunEngine::attach_read_only(dir.path(), "op-ui-recorded-test").unwrap();
    let inventory = inventory_json(&recorded);
    assert_eq!(inventory["view"], "recorded_read_only");
    assert!(operator_interface(&inventory).is_some());
}
