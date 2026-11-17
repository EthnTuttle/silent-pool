//! Role-metrics plumbing integration tests (SPEC §9, OPERATIONS §7).
//! Skipped unless `SPC_TEST_REDIS_URL` points at a throwaway server (the
//! e2e harness sets it).

use spc_pool::metrics::{render, MetricsSnapshot};
use spc_pool::role_metrics::{
    gather, merge_into, publish_ark, publish_front, publish_payout, ArkSample, FrontSample,
    PayoutSample,
};
use spc_store::redis::Streams;

async fn streams() -> Option<Streams> {
    let url = std::env::var("SPC_TEST_REDIS_URL").ok()?;
    Some(Streams::connect(&url).await.expect("connect"))
}

/// The full pipeline: three roles publish their snapshots, the api gathers
/// them from Redis and the exporter renders every role-sourced family.
#[tokio::test]
async fn published_role_snapshots_are_gathered_and_rendered() {
    let Some(streams) = streams().await else {
        return;
    };

    let front = FrontSample {
        front_id: 7,
        a_send_age_seconds: Some(12.5),
        a_send_repeat_total: 0,
        open_slice_shares: 3,
        slices_sealed_total: [("max_shares".to_string(), 2)].into(),
        distribution_slots: Some(
            [
                ("kept".to_string(), 1),
                ("ark".to_string(), 2),
                ("error".to_string(), 0),
            ]
            .into(),
        ),
        distribution_outputs_total: Some(2),
        budget_wu: Some(400_000),
        budget_utilisation: Some(0.01),
        channel_reject_total: [("invalid-user-identity".to_string(), 1)].into(),
        jd_declaration_reject_total: [("invalid-job".to_string(), 4)].into(),
        ext32_requests_total: [("get_window".to_string(), 9)].into(),
        applied_rev: 41,
    };
    publish_front(&streams, &front).await.unwrap();
    publish_ark(
        &streams,
        &ArkSample {
            float_sats: Some(1_234_567),
            surplus_sats: None,
            orders_rejected_total: [("bad-signature".to_string(), 1)].into(),
        },
    )
    .await
    .unwrap();
    publish_payout(
        &streams,
        &PayoutSample {
            committed_rev: Some(44),
            ark_overdue_total: Some(0),
            orphaned_settlement_total: [("competing_branch".to_string(), 1)].into(),
        },
    )
    .await
    .unwrap();

    let gathered = gather(&streams).await;
    assert_eq!(gathered.errors, 0);
    assert!(gathered.fronts.iter().any(|f| f.front_id == 7));
    assert!(gathered.ark.is_some());
    assert!(gathered.payout.is_some());

    let mut snap = MetricsSnapshot::default();
    merge_into(&mut snap, &gathered);
    let text = render(&snap);
    assert!(text.contains("spc_a_send_age_seconds 12.500\n"));
    assert!(text.contains("spc_a_send_repeat_total{front_id=\"7\"} 0\n"));
    assert!(text.contains("spc_open_slice_shares 3\n"));
    assert!(text.contains("spc_slices_sealed_total{reason=\"max_shares\"} 2\n"));
    assert!(text.contains("spc_distribution_slots{class=\"ark\"} 2\n"));
    assert!(text.contains("spc_distribution_outputs_total 2\n"));
    assert!(text.contains("spc_budget_wu 400000\n"));
    assert!(text.contains("spc_ark_float_sats 1234567\n"));
    assert!(text.contains("spc_ark_orders_rejected_total{reason=\"bad-signature\"} 1\n"));
    assert!(text.contains("spc_channel_reject_total{reason=\"invalid-user-identity\"} 1\n"));
    assert!(text.contains("spc_jd_declaration_reject_total{code=\"invalid-job\"} 4\n"));
    assert!(text.contains("spc_ext32_requests_total{msg=\"get_window\"} 9\n"));
    assert!(text.contains("spc_ark_overdue_total 0\n"));
    assert!(text.contains("spc_orphaned_settlement_total{reason=\"competing_branch\"} 1\n"));
    // committed 44, slowest applied 41.
    assert!(text.contains("spc_registration_lag_revs 3\n"));
    // No source for surplus: absent, not zero.
    assert!(!text.contains("spc_ark_surplus_sats"));

    // A role that stops publishing expires: its families become unknown.
    streams
        .kv_set_ex(
            "spc:metrics:ark",
            &serde_json::to_string(&ArkSample {
                float_sats: Some(1),
                surplus_sats: None,
                orders_rejected_total: Default::default(),
            })
            .unwrap(),
            1,
        )
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    let gathered = gather(&streams).await;
    assert!(gathered.ark.is_none());
    let mut snap = MetricsSnapshot::default();
    merge_into(&mut snap, &gathered);
    assert!(!render(&snap).contains("spc_ark_float_sats"));

    // An undecodable snapshot is a failed source: it counts in the scrape
    // errors and its families stay absent, never guessed at.
    streams
        .kv_set_ex("spc:metrics:payout", "not json", 30)
        .await
        .unwrap();
    let gathered = gather(&streams).await;
    assert!(gathered.payout.is_none());
    assert!(gathered.errors >= 1);
    let mut snap = MetricsSnapshot::default();
    merge_into(&mut snap, &gathered);
    assert!(snap.scrape_errors >= 1);
    assert!(!render(&snap).contains("spc_orphaned_settlement_total"));
    // Leave a decodable snapshot behind for any later test on the shared
    // throwaway server.
    publish_payout(&streams, &PayoutSample::default())
        .await
        .unwrap();
}
