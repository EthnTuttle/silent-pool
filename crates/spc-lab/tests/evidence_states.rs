mod common;

use std::time::{Duration, Instant};

use spc_lab::evidence::{
    Accounted, CanonicalAt, CanonicalResult, EvidenceActor, EvidencePayload, EvidenceStore,
    JobActivation, JobIssued, NodeRejected, ShareHeader,
};
use spc_lab::ids::{FrontId, IdentityId, JobId, NodeId, RunId, ShareId, SourceId};
use spc_lab::scenario::{AssertionKind, AssertionSpec};
use spc_lab::wide::WideU128;

fn run_id() -> RunId {
    RunId::new("run-evidence").unwrap()
}

fn source(name: &str) -> SourceId {
    SourceId::new(name).unwrap()
}

fn assertion(predicate: &str) -> AssertionSpec {
    AssertionSpec {
        id: "a".to_string(),
        kind: AssertionKind::Evidence,
        predicate: predicate.to_string(),
        deadline_secs: 10,
    }
}

fn job() -> JobIssued {
    JobIssued {
        front_id: FrontId::new("front-a").unwrap(),
        channel_id: 1,
        job_id: JobId::new("job-1").unwrap(),
        generation: 3,
        nbits: 0x207f_ffff,
        share_target: "7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
            .to_string(),
        m_issue: 2,
        work_unit_version: 2,
        work_unit_fingerprint: "fingerprint".to_string(),
        issued_at: 1,
        activation: JobActivation::Active,
    }
}

#[test]
fn absence_is_unknown_and_only_node_rejection_supports_invalid() {
    let store = EvidenceStore::new(run_id(), 100);
    store.append(
        source("front-a"),
        "job-1",
        EvidenceActor::new(spc_lab::evidence::ActorKind::Front, "front-a"),
        None,
        EvidencePayload::JobIssued(job()),
    );
    let block = "11".repeat(32);
    let status = store.current_status(&NodeId::new("node-a").unwrap(), &block);
    assert_eq!(status.result, CanonicalResult::Unknown);
    assert!(status.history.is_empty());
    assert!(store.has_node_rejection(&block).is_none());

    store.append(
        source("node-a"),
        "reject-1",
        EvidenceActor::new(spc_lab::evidence::ActorKind::Node, "node-a"),
        None,
        EvidencePayload::NodeRejected(NodeRejected {
            node_id: NodeId::new("node-a").unwrap(),
            block_hash: block.clone(),
            height: 5,
            reason: "bad-pow".to_string(),
            observed_at: 10,
        }),
    );
    assert!(store.has_node_rejection(&block).is_some());
    // A rejection does not create a canonical observation.
    assert_eq!(
        store
            .current_status(&NodeId::new("node-a").unwrap(), &block)
            .result,
        CanonicalResult::Unknown
    );
}

#[test]
fn canonical_observations_are_node_and_tip_scoped_and_reorgs_keep_history() {
    let store = EvidenceStore::new(run_id(), 100);
    let node_a = NodeId::new("node-a").unwrap();
    let node_b = NodeId::new("node-b").unwrap();
    let block = "22".repeat(32);
    let tip_a = "aa".repeat(32);
    let tip_b = "bb".repeat(32);
    store.append_at(
        source("node-a"),
        "c1",
        EvidenceActor::new(spc_lab::evidence::ActorKind::Node, "node-a"),
        None,
        EvidencePayload::CanonicalAt(CanonicalAt {
            node_id: node_a.clone(),
            block_hash: block.clone(),
            height: 5,
            anchored_tip: tip_a.clone(),
            observed_at: 100,
            source: source("node-a"),
            result: CanonicalResult::Yes,
            detail: "active at tip A".to_string(),
        }),
        100,
    );
    assert_eq!(
        store.current_status(&node_a, &block).result,
        CanonicalResult::Yes
    );
    assert_eq!(
        store.current_status(&node_b, &block).result,
        CanonicalResult::Unknown
    );

    store.append_at(
        source("node-a"),
        "c2",
        EvidenceActor::new(spc_lab::evidence::ActorKind::Node, "node-a"),
        None,
        EvidencePayload::CanonicalAt(CanonicalAt {
            node_id: node_a.clone(),
            block_hash: block.clone(),
            height: 5,
            anchored_tip: tip_b.clone(),
            observed_at: 200,
            source: source("node-a"),
            result: CanonicalResult::No,
            detail: "tip B has a different block at height 5".to_string(),
        }),
        200,
    );
    let status = store.current_status(&node_a, &block);
    assert_eq!(status.result, CanonicalResult::No);
    assert_eq!(status.anchored_tip, tip_b);
    assert_eq!(status.history.len(), 2);
    assert_eq!(status.history[0].result, CanonicalResult::Yes);
    assert_eq!(status.history[0].anchored_tip, tip_a);
    // The historical observation remains auditable; "no" is not "invalid".
    assert!(store.has_node_rejection(&block).is_none());
}

#[test]
fn observer_overflow_records_a_gap_without_blocking_the_producer() {
    let store = EvidenceStore::new(run_id(), 1_000);
    store.append(
        source("harness"),
        "run-state",
        EvidenceActor::harness(),
        None,
        EvidencePayload::RunState {
            state: "running".to_string(),
        },
    );
    let (subscription, _) = store.subscribe(0).unwrap();
    let started = Instant::now();
    for index in 0..400 {
        store.append(
            source("front-a"),
            format!("event-{index}"),
            EvidenceActor::harness(),
            None,
            EvidencePayload::Unknown(spc_lab::evidence::UnknownEvidence {
                topic: format!("event-{index}"),
                detail: String::new(),
            }),
        );
    }
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(2),
        "producer was blocked for {elapsed:?}"
    );
    let gaps = store.gaps();
    assert!(!gaps.is_empty());
    assert!(gaps.iter().all(|gap| gap.subscriber > 0));
    let event_lost = store
        .events()
        .iter()
        .any(|event| matches!(event.payload, EvidencePayload::EventLost(_)));
    assert!(event_lost);
    assert!(gaps
        .iter()
        .all(|gap| gap.scope == spc_lab::evidence::GapScope::Subscriber));
    // A slow subscriber is subscriber-scoped; the durable store still has the
    // run_state event, so the assertion passes.
    assert_eq!(
        store.evaluate_assertion(&assertion("exists(run_state)")),
        spc_lab::evidence::AssertionEvaluation::Passed
    );
    // A store-level loss is different and makes assertions inconclusive.
    store.record_gap(spc_lab::evidence::Gap {
        scope: spc_lab::evidence::GapScope::Store,
        subscriber: 0,
        from_event_id: 1,
        to_event_id: 1,
        reason: "injected durable write failure".to_string(),
    });
    assert!(matches!(
        store.evaluate_assertion(&assertion("exists(run_state)")),
        spc_lab::evidence::AssertionEvaluation::Inconclusive(_)
    ));
    // The stalled observer is closed rather than continuing with a silent
    // skip; its marker is in the durable gap list and HTTP sends resync.
    assert_eq!(store.subscriber_count(), 0);
    drop(subscription);
    assert!(store.last_event_id() > 400);
}

#[test]
fn typed_evidence_states_and_wide_credit_are_lossless() {
    let store = EvidenceStore::new(run_id(), 100);
    let identity = IdentityId::new("id-alice").unwrap();
    let target = "7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
    store.append(
        source("front-a"),
        "job-1",
        EvidenceActor::new(spc_lab::evidence::ActorKind::Front, "front-a"),
        None,
        EvidencePayload::JobIssued(job()),
    );
    store.append(
        source("miner-1"),
        "submit-1",
        EvidenceActor::new(spc_lab::evidence::ActorKind::Miner, "miner-1"),
        None,
        EvidencePayload::Submitted(spc_lab::evidence::Submitted {
            miner_id: spc_lab::ids::MinerId::new("miner-1").unwrap(),
            identity_id: identity.clone(),
            share_id: ShareId::new("share-1").unwrap(),
            job_id: JobId::new("job-1").unwrap(),
            generation: 3,
            header: ShareHeader {
                version: 1,
                prev_hash: "00".repeat(32),
                merkle_root: "00".repeat(32),
                ntime: 1,
                nbits: 0x207f_ffff,
                nonce: 1,
            },
            hash: "00".repeat(32),
        }),
    );
    let id = store.append(
        source("front-a"),
        "accounted-1",
        EvidenceActor::new(spc_lab::evidence::ActorKind::Front, "front-a"),
        None,
        EvidencePayload::Accounted(Accounted {
            front_id: FrontId::new("front-a").unwrap(),
            identity_id: identity,
            share_id: ShareId::new("share-1").unwrap(),
            job_id: JobId::new("job-1").unwrap(),
            generation: 3,
            nbits: 0x207f_ffff,
            share_target: target.to_string(),
            d: WideU128(2),
            slice_id: "slice-1".to_string(),
            diff_sum: WideU128(u128::MAX),
        }),
    );
    assert!(id > 0);
    assert_eq!(
        store.evaluate_assertion(&assertion("exists(accounted)")),
        spc_lab::evidence::AssertionEvaluation::Passed
    );
    // Candidate is absent: absence is not invalid.
    assert_eq!(
        store.evaluate_assertion(&assertion("exists(candidate)")),
        spc_lab::evidence::AssertionEvaluation::Inconclusive(
            "candidate was not observed; absence is not invalid".to_string()
        )
    );
    let accounted = store
        .events()
        .iter()
        .find_map(|event| match &event.payload {
            EvidencePayload::Accounted(value) => Some(value.clone()),
            _ => None,
        })
        .unwrap();
    let json = serde_json::to_string(&accounted).unwrap();
    assert!(json.contains(&u128::MAX.to_string()));
    let back: Accounted = serde_json::from_str(&json).unwrap();
    assert_eq!(back, accounted);

    let snapshot = store.snapshot(None, Vec::new());
    assert!(snapshot
        .watermarks
        .iter()
        .any(|w| w.source.as_str() == "front-a"));
    assert!(snapshot
        .watermarks
        .iter()
        .any(|w| w.source.as_str() == "miner-1"));
    assert_eq!(snapshot.evidence.counts.get("job_issued"), Some(&1));
    assert_eq!(snapshot.evidence.counts.get("submitted"), Some(&1));
    assert_eq!(snapshot.evidence.counts.get("accounted"), Some(&1));
}

#[test]
fn a_durable_write_failure_is_a_store_gap_not_silently_ignored() {
    if !std::path::Path::new("/dev/full").exists() {
        return;
    }
    let store =
        EvidenceStore::open(RunId::new("run-write-failure").unwrap(), "/dev/full", 10).unwrap();
    store.append(
        source("harness"),
        "run-state",
        EvidenceActor::harness(),
        None,
        EvidencePayload::RunState {
            state: "running".to_string(),
        },
    );
    assert!(!store.durable_gaps().is_empty());
    assert!(matches!(
        store.evaluate_assertion(&assertion("exists(run_state)")),
        spc_lab::evidence::AssertionEvaluation::Inconclusive(_)
    ));
}
