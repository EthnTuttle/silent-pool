mod common;

use spc_lab::evidence::{Accounted, EvidenceActor, EvidencePayload, JobActivation};
use spc_lab::ids::{FrontId, IdentityId, JobId, ShareId, SourceId};
use spc_lab::wide::WideU128;

#[test]
fn identity_bearing_evidence_and_secrets_stay_out_of_the_shareable_manifest() {
    let dir = common::TestDir::new("artifacts-privacy");
    let engine = common::prepare_engine(&dir, "scenario_v1_one_pool_two_miners.json");
    engine.start().unwrap();
    engine.emit_job_issued(
        &FrontId::new("front-a1").unwrap(),
        1,
        JobId::new("job-1").unwrap(),
        1,
        0x207f_ffff,
        "7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".to_string(),
        2,
        2,
        "fingerprint".to_string(),
        JobActivation::Active,
    );
    engine.evidence().append(
        SourceId::new("front-a1").unwrap(),
        "accounted-1",
        EvidenceActor::harness(),
        None,
        EvidencePayload::Accounted(Accounted {
            front_id: FrontId::new("front-a1").unwrap(),
            identity_id: IdentityId::new("id-alice").unwrap(),
            share_id: ShareId::new("share-1").unwrap(),
            job_id: JobId::new("job-1").unwrap(),
            generation: 1,
            nbits: 0x207f_ffff,
            share_target: "7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
                .to_string(),
            d: WideU128(2),
            slice_id: "slice-1".to_string(),
            diff_sum: WideU128(2),
        }),
    );
    engine.stop().unwrap();
    let artifacts = engine.manifest_snapshot().artifact_paths;
    let shareable = std::fs::read_to_string(&artifacts.shareable_manifest).unwrap();
    let charts = std::fs::read_to_string(&artifacts.shareable_charts).unwrap();
    let result = std::fs::read_to_string(&artifacts.report).unwrap();
    for (name, body) in [
        ("manifest", &shareable),
        ("charts", &charts),
        ("result", &result),
    ] {
        for forbidden in [
            "identity_id",
            "sp_id",
            "a_send",
            "b_scan",
            "b_spend",
            "b_ark",
            "shared_secret",
            "ecdh",
            "tweak",
            "signing_key",
            "mailbox_secret",
            "private_key",
            "password",
            "credential",
        ] {
            assert!(
                !body.contains(forbidden),
                "{name} contains forbidden key/value {forbidden:?}"
            );
        }
    }
    let private = std::fs::read_to_string(&artifacts.private_evidence).unwrap();
    assert!(
        private.contains("id-alice"),
        "private evidence retains identity"
    );
    assert!(private.contains("\"accounted\""));

    // The chart is a projection; raw evidence is still present in the private
    // store and its event count is not reduced by decimation.
    let manifest: serde_json::Value = serde_json::from_str(&shareable).unwrap();
    assert!(
        manifest["evidence"]["counts"]["accounted"]
            .as_u64()
            .unwrap_or(0)
            >= 1
    );
    assert!(manifest["non_claims"].as_array().unwrap().len() >= 4);
}
