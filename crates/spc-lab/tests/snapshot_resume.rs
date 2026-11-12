mod common;

use spc_lab::artifacts::decimate_timeline;
use spc_lab::evidence::{EvidenceActor, EvidencePayload, EvidenceStore, UnknownEvidence};
use spc_lab::ids::{RunId, SourceId};
use spc_lab::wide::WideU128;

fn run_id() -> RunId {
    RunId::new("run-snapshot").unwrap()
}

fn source() -> SourceId {
    SourceId::new("front-a").unwrap()
}

fn append(store: &EvidenceStore, index: u64) {
    store.append(
        source(),
        format!("event-{index}"),
        EvidenceActor::harness(),
        None,
        EvidencePayload::Unknown(UnknownEvidence {
            topic: format!("event-{index}"),
            detail: String::new(),
        }),
    );
}

#[test]
fn snapshot_cursor_and_subscription_do_not_skip_a_gap() {
    let store = EvidenceStore::new(run_id(), 100);
    for index in 0..3 {
        append(&store, index);
    }
    let snapshot = store.snapshot(Some(0), Vec::new());
    assert_eq!(snapshot.snapshot_cursor, 3);
    assert!(!snapshot.resync_required);
    assert_eq!(snapshot.evidence.total_events, 3);

    let (subscription, result) = store.subscribe(3).unwrap();
    assert!(matches!(
        result,
        spc_lab::evidence::SubscribeResult::Replay { cursor: 3, .. }
    ));
    assert!(subscription.replay.is_empty());
    append(&store, 3);
    let event = subscription
        .receiver
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap();
    assert_eq!(event.event_id, 4);
    assert_eq!(event.payload.kind(), "unknown");

    // A reconnect at the new cursor does not replay the received event.
    let (again, _) = store.subscribe(4).unwrap();
    assert!(again.replay.is_empty());

    // A future cursor is a silent-skip hazard, not an empty replay.
    let (_, future) = store.subscribe(10_000).unwrap();
    assert!(matches!(
        future,
        spc_lab::evidence::SubscribeResult::ResyncRequired { .. }
    ));
    assert!(store.snapshot(Some(10_000), Vec::new()).resync_required);
}

#[test]
fn cursor_older_than_the_retained_tail_requires_resync() {
    let store = EvidenceStore::new(run_id(), 4);
    for index in 0..12 {
        append(&store, index);
    }
    assert_eq!(store.retained_from(), 9);
    let snapshot = store.snapshot(Some(1), Vec::new());
    assert!(snapshot.resync_required);
    assert_eq!(snapshot.retained_from, 9);
    assert_eq!(snapshot.snapshot_cursor, 12);
    let (_, result) = store.subscribe(1).unwrap();
    assert!(matches!(
        result,
        spc_lab::evidence::SubscribeResult::ResyncRequired { .. }
    ));
    // The resync snapshot carries the cursor the client must use.
    let fresh = store.snapshot(Some(snapshot.snapshot_cursor), Vec::new());
    assert!(!fresh.resync_required);
}

#[test]
fn snapshots_keep_wide_values_as_decimal_strings() {
    let store = EvidenceStore::new(run_id(), 100);
    store.append(
        source(),
        "wide",
        EvidenceActor::harness(),
        None,
        EvidencePayload::Delivered(spc_lab::evidence::Delivered {
            identity_id: spc_lab::ids::IdentityId::new("id-alice").unwrap(),
            block_hash: "00".repeat(32),
            amount_sats: WideU128(u128::MAX),
            detail: "wide".to_string(),
        }),
    );
    let events = store.events();
    let event_json = serde_json::to_string(&*events[0]).unwrap();
    assert!(event_json.contains(&format!("\"{}\"", u128::MAX)));
    assert!(!event_json.contains(&format!(":{}", u128::MAX)));
    let snapshot_json = serde_json::to_string(&store.snapshot(None, Vec::new())).unwrap();
    assert!(!snapshot_json.contains(&format!(":{}", u128::MAX)));
}

#[test]
fn chart_decimation_never_removes_raw_evidence() {
    let store = EvidenceStore::new(run_id(), 100);
    for index in 0..40 {
        store.append_at(
            source(),
            format!("event-{index}"),
            EvidenceActor::harness(),
            None,
            EvidencePayload::Unknown(UnknownEvidence {
                topic: format!("event-{index}"),
                detail: String::new(),
            }),
            1_000 + index,
        );
    }
    let raw = store.events();
    let owned: Vec<_> = raw.iter().map(|event| (**event).clone()).collect();
    let series = decimate_timeline(&owned, 10);
    assert_eq!(series.source_event_count, 40);
    assert_eq!(series.point_count, 4);
    assert!(series.dropped_points > 0);
    assert_eq!(series.algorithm, "sum_event_count_per_fixed_bucket");
    assert_eq!(store.events().len(), 40, "the raw store is untouched");
}
