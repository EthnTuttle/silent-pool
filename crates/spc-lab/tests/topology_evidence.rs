use std::time::Duration;

use serde_json::{json, Value};
use spc_lab::convergence::{drive_convergence, ConvergenceDriver};
use spc_lab::evidence::{AtomicTipObservation, EvidencePayload};
use spc_lab::scenario::{ConvergenceSpec, Scenario};

fn spec() -> ConvergenceSpec {
    ConvergenceSpec {
        version: 1,
        timeout_secs: 90,
        max_observations: 4,
        allow_resume: false,
        max_resume_attempts: 0,
    }
}

#[test]
fn convergence_is_typed_with_a_version_and_old_unknown_records_still_load() {
    struct Shared;
    impl ConvergenceDriver for Shared {
        fn observe(&mut self) -> Vec<AtomicTipObservation> {
            ["node-a", "node-b"]
                .map(|node| AtomicTipObservation {
                    node: node.to_string(),
                    height: Some(42),
                    hash: Some("anchor".to_string()),
                    parent_hash: Some("parent".to_string()),
                    atomic: true,
                    peers: Some(2),
                    error: None,
                })
                .to_vec()
        }
        fn worker_control(&mut self, resume: bool) -> Result<Vec<Value>, String> {
            Ok(vec![json!({"resume": resume})])
        }
        fn stop_requested(&self) -> bool {
            false
        }
        fn wait(&mut self, _: Duration) {}
    }
    let evidence = drive_convergence("after-heal", &spec(), &mut Shared).unwrap();
    assert_eq!(evidence.status, "converged");
    let mut evidence = evidence;
    evidence.pre_partition_common_anchor = Some(spc_lab::evidence::SharedChainAnchor {
        height: 41,
        hash: "anchor-parent".to_string(),
    });
    let encoded =
        serde_json::to_value(EvidencePayload::TopologyConvergence(evidence.clone())).unwrap();
    assert_eq!(encoded["kind"], "topology_convergence");
    assert_eq!(encoded["data"]["schema_version"], 1);
    assert_eq!(encoded["data"]["views"][0]["hash"], "anchor");
    assert_eq!(encoded["data"]["views"][0]["parent_hash"], "parent");
    assert_eq!(encoded["data"]["pre_partition_common_anchor"]["height"], 41);
    assert_eq!(encoded["data"]["policy"]["max_resume_attempts"], 0);
    let decoded: EvidencePayload = serde_json::from_value(encoded.clone()).unwrap();
    assert_eq!(decoded.kind(), "topology_convergence");
    let mut unknown = encoded;
    unknown["data"]["unexpected_field"] = json!(true);
    assert!(
        serde_json::from_value::<EvidencePayload>(unknown).is_err(),
        "strict typed payloads reject unknown fields"
    );
    let legacy: EvidencePayload = serde_json::from_value(json!({
        "kind": "unknown",
        "data": {"topic": "topology_phase", "detail": "{\"old\":\"record\"}"}
    }))
    .unwrap();
    assert_eq!(legacy.kind(), "unknown", "old recorded runs stay readable");
}

#[test]
fn convergence_dsl_is_optional_and_validates_version_and_bounds() {
    let legacy = Scenario::from_json(include_str!(
        "fixtures/scenario_v1_one_pool_two_miners.json"
    ))
    .unwrap();
    assert!(
        legacy
            .phases
            .iter()
            .all(|phase| phase.convergence.is_none()),
        "legacy scenarios have no implicit convergence/hash action"
    );
    let scenario = Scenario::from_json(include_str!(
        "../scenarios/lab-network-runtime-two-pools.json"
    ))
    .unwrap();
    let convergence = scenario
        .phases
        .iter()
        .find(|phase| phase.id == "converge-after-heal")
        .and_then(|phase| phase.convergence.as_ref())
        .unwrap();
    assert_eq!(convergence.version, 1);
    assert_equality_checks_fail(scenario);
}

fn assert_equality_checks_fail(mut scenario: Scenario) {
    if let Some(phase) = scenario
        .phases
        .iter_mut()
        .find(|phase| phase.convergence.is_some())
    {
        phase.convergence.as_mut().unwrap().timeout_secs = 0;
    }
    assert!(
        scenario.validate().is_err(),
        "a zero convergence timeout must fail closed"
    );
}
