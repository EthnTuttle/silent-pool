//! Store-boundary tests for `Store::liability_snapshot()` (EXP-OP / OP-3).
//!
//! These run only when `SPC_TEST_DATABASE_URL` names a throwaway server the
//! test may create and drop databases on (the e2e harness sets it). Every
//! fixture value here is a tiny TESTFIXTURES label, not earnings: pending
//! balance 7 sats, scheduled Ark 3 sats, and 64 one-sat Ark rows moved to
//! pending by the racing settle. No acceptance claim is derived from these
//! numbers.

use spc_store::models::*;
use spc_store::Store;

/// Create an isolated fixture database (name carries the TESTFIXTURES
/// label) so parallel tests never share rows. Skips when the env var is
/// absent; the EXP-OP harness runs it against a dedicated throwaway
/// Postgres, so in that mode it always executes.
async fn isolated_fixture() -> Option<(Store, sqlx::PgPool, String)> {
    let url = std::env::var("SPC_TEST_DATABASE_URL").ok()?;
    let base = url.split('?').next().unwrap_or(&url);
    let (head, _) = base.rsplit_once('/').expect("database path in URL");
    let name = format!(
        "expop_testfixtures_{}",
        hex::encode(uuid::Uuid::new_v4().as_bytes())
    );
    let admin = sqlx::PgPool::connect(&format!("{head}/postgres"))
        .await
        .unwrap();
    if sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .is_err()
    {
        eprintln!("skip: no CREATE DATABASE privilege on the test server");
        return None;
    }
    let store = Store::connect(&format!("{head}/{name}"))
        .await
        .expect("connect isolated database");
    store.migrate().await.expect("migrate isolated database");
    Some((store, admin, name))
}

fn id() -> [u8; 32] {
    let mut out = [0u8; 32];
    out[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    out[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    out
}

async fn identity(store: &Store, sp_id: [u8; 32]) {
    store
        .upsert_identity(&SpIdentityRow {
            sp_id,
            b_scan: vec![2; 33],
            b_spend: vec![3; 33],
            network: 3,
        })
        .await
        .unwrap();
}

async fn snapshot_row(store: &Store, fingerprint: [u8; 32]) {
    store
        .insert_snapshot(&SnapshotRow {
            fingerprint,
            height: 800_000,
            a_send_x: [4; 32],
            distribution_id: None,
            entries: serde_json::json!([]),
            high_water: serde_json::json!({}),
            work_unit_version: WorkUnitVersion::V1,
            work_unit_fingerprint: None,
            window_rule: WindowRule::ShareWorkLegacy,
            window_target: Some("16".into()),
        })
        .await
        .unwrap();
}

/// Seed the contract's single-snapshot fixture: pending balance 7 sats
/// (4 + 3 across two identities) and scheduled Ark 3 sats (one block).
async fn seed_7_3(store: &Store) {
    let fingerprint = id();
    snapshot_row(store, fingerprint).await;
    for (delta, block) in [(4i64, id()), (3, id())] {
        let sp_id = id();
        identity(store, sp_id).await;
        store
            .settle(
                &Settlement {
                    block_hash: block,
                    height: 800_000,
                    fingerprint: Some(fingerprint),
                    status: BlockStatus::Confirmed,
                    ledger: vec![LedgerRow {
                        sp_id,
                        rail: LedgerRail::Pending,
                        paid_sats: 0,
                        ark_due_sats: 0,
                        forfeited_sats: 0,
                        pending_delta: delta,
                    }],
                },
                6,
            )
            .await
            .unwrap();
    }
    let sp_id = id();
    identity(store, sp_id).await;
    store
        .settle(
            &Settlement {
                block_hash: id(),
                height: 800_001,
                fingerprint: Some(fingerprint),
                status: BlockStatus::Confirmed,
                ledger: vec![LedgerRow {
                    sp_id,
                    rail: LedgerRail::Ark,
                    paid_sats: 0,
                    ark_due_sats: 3,
                    forfeited_sats: 0,
                    pending_delta: 0,
                }],
            },
            6,
        )
        .await
        .unwrap();
}

/// OP-2: one snapshot returns pending 7, Ark 3 and the enforced total 10,
/// with a single `as_of` shared by all three fields.
#[tokio::test]
async fn snapshot_returns_the_contract_fixture_7_plus_3() {
    let Some((store, admin, name)) = isolated_fixture().await else {
        return;
    };
    seed_7_3(&store).await;
    let snap = store.liability_snapshot().await.expect("snapshot");
    assert_eq!(snap.pending_balance_sats, 7);
    assert_eq!(snap.ark_due_sats, 3);
    assert_eq!(snap.total_sats, 10);
    assert_eq!(
        snap.total_sats,
        snap.pending_balance_sats + snap.ark_due_sats,
        "the invariant must hold on the seeded fixture"
    );
    assert!(snap.as_of <= chrono::Utc::now());
    sqlx::query(&format!("DROP DATABASE {name} WITH (FORCE)"))
        .execute(&admin)
        .await
        .unwrap();
}

/// OP-2: a settle racing the snapshot cannot yield a torn or partial pair.
///
/// Conserved-sum oracle (Fable R1 rework): N one-sat scheduled Ark rows
/// are moved to pending one at a time by the real
/// `mark_ark_failed_to_pending` transaction (scheduled -> failed_to_pending
/// plus `pending += sats`, atomically). The moved sat never leaves the
/// pool, so every single-statement snapshot must observe the conserved
/// total N; the pending and ark halves are checked against the seeded
/// constant, not against each other (the old form asserted the Rust
/// `checked_add` identity, which holds for any two numbers the SQL
/// returns, torn or not). A two-statement implementation reading pending
/// and ark in separate statements observes N-1 or N+1 whenever a move
/// commits between its reads - that counterfactual is executed
/// deterministically in `torn_two_statement_counterfactual_is_detectable`
/// below, and the Fable review measured the live version at 40 tears in
/// 1236 two-statement reads against 0 for this single statement.
#[tokio::test]
async fn settle_racing_the_snapshot_never_tears_the_pair() {
    let Some((store, admin, name)) = isolated_fixture().await else {
        return;
    };
    const N: u64 = 64;
    let fingerprint = id();
    snapshot_row(&store, fingerprint).await;
    let sp_id = id();
    identity(&store, sp_id).await;
    let mut blocks = Vec::new();
    for i in 0..N as u32 {
        let block = id();
        blocks.push(block);
        store
            .settle(
                &Settlement {
                    block_hash: block,
                    height: 900_000 + i,
                    fingerprint: Some(fingerprint),
                    status: BlockStatus::Confirmed,
                    ledger: vec![LedgerRow {
                        sp_id,
                        rail: LedgerRail::Ark,
                        paid_sats: 0,
                        ark_due_sats: 1,
                        forfeited_sats: 0,
                        pending_delta: 0,
                    }],
                },
                6,
            )
            .await
            .unwrap();
    }
    let snap0 = store.liability_snapshot().await.unwrap();
    assert_eq!(snap0.pending_balance_sats, 0);
    assert_eq!(snap0.ark_due_sats, N);
    assert_eq!(snap0.total_sats, N);

    // One move at a time via the real production path; the small sleep
    // only spreads the moves so the snapshot loop observes the middle of
    // the transition (coverage, not correctness - conservation below is
    // guaranteed by the single MVCC statement, not by timing).
    let sender = {
        let store = store.clone();
        tokio::spawn(async move {
            for block in &blocks {
                store
                    .mark_ark_failed_to_pending(block, &sp_id)
                    .await
                    .unwrap();
                tokio::time::sleep(std::time::Duration::from_millis(2)).await;
            }
        })
    };
    let mut observed = Vec::new();
    while !sender.is_finished() {
        let snap = store.liability_snapshot().await.expect("snapshot");
        assert_eq!(
            snap.total_sats, N,
            "torn snapshot: total must stay at the conserved N, got {snap:?}"
        );
        assert_eq!(
            snap.pending_balance_sats + snap.ark_due_sats,
            N,
            "torn snapshot: halves must sum to the seeded N, got {snap:?}"
        );
        observed.push((snap.pending_balance_sats, snap.ark_due_sats));
    }
    sender.await.unwrap();
    let last = store.liability_snapshot().await.unwrap();
    assert_eq!(last.pending_balance_sats, N);
    assert_eq!(last.ark_due_sats, 0);
    assert_eq!(last.total_sats, N);
    observed.push((last.pending_balance_sats, last.ark_due_sats));
    // The race must actually observe the middle of the transition, not
    // just the endpoints (with N moves spread over >= 2 ms each, the
    // snapshot loop runs throughout).
    assert!(
        observed
            .iter()
            .any(|(pending, _)| *pending > 0 && *pending < N),
        "the race must observe a mid-transition state, got {observed:?}"
    );
    // Real secondary properties of this scenario: moves only add to
    // pending and only remove from scheduled Ark.
    for pair in observed.windows(2) {
        assert!(
            pair[1].0 >= pair[0].0 && pair[1].1 <= pair[0].1,
            "pending must be non-decreasing and ark non-increasing while \
             moves run: {observed:?}"
        );
    }
    sqlx::query(&format!("DROP DATABASE {name} WITH (FORCE)"))
        .execute(&admin)
        .await
        .unwrap();
}

/// R1 sensitivity proof (forced interleave): demonstrates that the
/// conserved-sum oracle above actually discriminates. We read the pending
/// half with one statement, let exactly one full
/// `mark_ark_failed_to_pending` commit, then read the ark half with a
/// second statement - the exact shape of a hypothetical two-statement
/// `liability_snapshot`. The torn pair sums to N-1, which the
/// `total == N` oracle rejects; the real single-statement snapshot at the
/// same logical point is exact. Deterministic: the interleave point is
/// program order, no sleep, no probability.
#[tokio::test]
async fn torn_two_statement_counterfactual_is_detectable() {
    let Some((store, admin, name)) = isolated_fixture().await else {
        return;
    };
    const N: i64 = 3;
    let fingerprint = id();
    snapshot_row(&store, fingerprint).await;
    let sp_id = id();
    identity(&store, sp_id).await;
    let mut blocks = Vec::new();
    for i in 0..N as u32 {
        let block = id();
        blocks.push(block);
        store
            .settle(
                &Settlement {
                    block_hash: block,
                    height: 900_000 + i,
                    fingerprint: Some(fingerprint),
                    status: BlockStatus::Confirmed,
                    ledger: vec![LedgerRow {
                        sp_id,
                        rail: LedgerRail::Ark,
                        paid_sats: 0,
                        ark_due_sats: 1,
                        forfeited_sats: 0,
                        pending_delta: 0,
                    }],
                },
                6,
            )
            .await
            .unwrap();
    }
    // Half-read one: pending before any move.
    let half_pending: i64 =
        sqlx::query_scalar("SELECT COALESCE(SUM(sats), 0)::bigint FROM pending_balance")
            .fetch_one(store.pool())
            .await
            .unwrap();
    assert_eq!(half_pending, 0);
    // Exactly one atomic move commits between the two half-reads.
    store
        .mark_ark_failed_to_pending(&blocks[0], &sp_id)
        .await
        .unwrap();
    // Half-read two: scheduled ark after the move.
    let half_ark: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(sats), 0)::bigint \
         FROM ark_payout WHERE status = 'scheduled'",
    )
    .fetch_one(store.pool())
    .await
    .unwrap();
    assert_eq!(half_ark, N - 1);
    // The torn two-statement total: the moved sat is counted on neither
    // side. The conserved-sum oracle rejects exactly this.
    let torn_total = half_pending + half_ark;
    assert_eq!(torn_total, N - 1);
    assert_ne!(
        torn_total, N,
        "the oracle must be able to fail: a torn read must not equal N"
    );
    // The real single-statement snapshot at the same logical point is
    // exact: pending 1, scheduled ark N-1, total N.
    let snap = store.liability_snapshot().await.unwrap();
    assert_eq!(snap.pending_balance_sats, 1);
    assert_eq!(snap.ark_due_sats, (N - 1) as u64);
    assert_eq!(snap.total_sats, N as u64);
    sqlx::query(&format!("DROP DATABASE {name} WITH (FORCE)"))
        .execute(&admin)
        .await
        .unwrap();
}

/// OP-1: a broken schema is an error, never a zero snapshot.
#[tokio::test]
async fn broken_schema_is_an_error_not_zeros() {
    let Some((store, admin, name)) = isolated_fixture().await else {
        return;
    };
    seed_7_3(&store).await;
    sqlx::query("DROP TABLE pending_balance, ark_payout")
        .execute(store.pool())
        .await
        .unwrap();
    let err = store.liability_snapshot().await.expect_err("must fail");
    assert!(
        !matches!(err, spc_store::postgres::StoreError::Timeout(_)),
        "a broken schema is not a timeout: {err:?}"
    );
    sqlx::query(&format!("DROP DATABASE {name} WITH (FORCE)"))
        .execute(&admin)
        .await
        .unwrap();
}

/// OP-1: a stopped store (here: the database is force-dropped under the
/// pool) is an error, never a zero snapshot.
#[tokio::test]
async fn stopped_store_is_an_error_not_zeros() {
    let Some((store, admin, name)) = isolated_fixture().await else {
        return;
    };
    seed_7_3(&store).await;
    let before = store.liability_snapshot().await.unwrap();
    assert_eq!(before.total_sats, 10);
    sqlx::query(&format!("DROP DATABASE {name} WITH (FORCE)"))
        .execute(&admin)
        .await
        .unwrap();
    let err = store.liability_snapshot().await.expect_err("must fail");
    assert!(
        !matches!(err, spc_store::postgres::StoreError::Invalid(_)),
        "a dead database is not an invalid value: {err:?}"
    );
}
