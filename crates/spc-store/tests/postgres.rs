//! Postgres integration tests. Skipped unless `SPC_TEST_DATABASE_URL` points
//! at a throwaway database (the e2e harness sets it).
//!
//! These are the tests that prove SPEC §7.4's idempotency, orphan and
//! liability rules at the real storage boundary.

use sqlx::Row;

use spc_store::models::*;
use spc_store::postgres::APPLIED_BLOCKS_LIMIT;
use spc_store::Store;
use uuid::Uuid;

async fn store() -> Option<Store> {
    let url = std::env::var("SPC_TEST_DATABASE_URL").ok()?;
    let store = Store::connect(&url).await.expect("connect");
    store.migrate().await.expect("migrate");
    Some(store)
}

/// Create a uniquely named database for tests whose metric assertions must
/// describe only their own fixture. This avoids both shared-database counts
/// and racy before/after deltas when the suite runs with parallel test threads.
async fn isolated_store(prefix: &str) -> Option<(Store, sqlx::PgPool, String)> {
    let url = std::env::var("SPC_TEST_DATABASE_URL").ok()?;
    let base = url.split('?').next().unwrap_or(&url);
    let (head, _) = base
        .rsplit_once('/')
        .expect("SPC_TEST_DATABASE_URL has a database path");
    let admin_url = format!("{head}/postgres");
    let name = format!("{prefix}{}", hex::encode(uuid::Uuid::new_v4().as_bytes()));
    let admin = sqlx::PgPool::connect(&admin_url).await.unwrap();
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
    out[..16].copy_from_slice(Uuid::new_v4().as_bytes());
    out[16..].copy_from_slice(Uuid::new_v4().as_bytes());
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

async fn snapshot(store: &Store, fingerprint: [u8; 32]) {
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

#[tokio::test]
async fn settle_is_idempotent_and_orphan_reverses() {
    let Some(store) = store().await else { return };

    let sp_id = id();
    let fingerprint = id();
    let block_hash = id();
    identity(&store, sp_id).await;
    snapshot(&store, fingerprint).await;

    let settlement = Settlement {
        block_hash,
        height: 800_000,
        fingerprint: Some(fingerprint),
        status: BlockStatus::Confirmed,
        ledger: vec![LedgerRow {
            sp_id,
            rail: LedgerRail::Coinbase,
            paid_sats: 1_000,
            ark_due_sats: 0,
            forfeited_sats: 0,
            pending_delta: 0,
        }],
    };

    store.settle(&settlement, 6).await.unwrap();
    assert_eq!(
        store.block_status(&block_hash).await.unwrap(),
        Some(BlockStatus::Confirmed)
    );

    // A redelivered block event changes nothing.
    assert!(store.settle(&settlement, 6).await.is_err());
    assert_eq!(
        store.block_status(&block_hash).await.unwrap(),
        Some(BlockStatus::Confirmed)
    );

    store.orphan(&block_hash).await.unwrap();
    assert_eq!(
        store.block_status(&block_hash).await.unwrap(),
        Some(BlockStatus::Orphaned)
    );
    // Scoped to this test's ids: the suite shares one database and runs in
    // parallel, so a global liability assertion would race.
    assert!(store.ledger_for(&sp_id, 10).await.unwrap().is_empty());
    assert_eq!(store.pending_for(&sp_id).await.unwrap(), 0);
}

#[tokio::test]
async fn ark_rail_is_scheduled_and_leaves_liability_when_sent() {
    let Some(store) = store().await else { return };

    let sp_id = id();
    let fingerprint = id();
    let block_hash = id();
    identity(&store, sp_id).await;
    snapshot(&store, fingerprint).await;

    let settlement = Settlement {
        block_hash,
        height: 800_000,
        fingerprint: Some(fingerprint),
        status: BlockStatus::Confirmed,
        ledger: vec![LedgerRow {
            sp_id,
            rail: LedgerRail::Ark,
            paid_sats: 0,
            ark_due_sats: 250,
            forfeited_sats: 0,
            pending_delta: 0,
        }],
    };
    store.settle(&settlement, 6).await.unwrap();

    let payouts = store.scheduled_ark_payouts().await.unwrap();
    let payout = payouts
        .iter()
        .find(|p| p.block_hash == block_hash)
        .expect("scheduled ark payout");
    assert_eq!(payout.sats, 250);
    assert_eq!(payout.due_height, 800_006);
    // Scoped to this miner: global liability races other tests.
    let scheduled_for = |rows: Vec<ArkPayoutRow>| -> u64 {
        rows.iter()
            .filter(|row| row.sp_id == sp_id)
            .map(|row| row.sats)
            .sum()
    };
    assert_eq!(
        scheduled_for(store.scheduled_ark_payouts().await.unwrap()),
        250
    );

    store
        .mark_ark_sent(&block_hash, &sp_id, &[9; 34], 900_000)
        .await
        .unwrap();
    assert_eq!(
        scheduled_for(store.scheduled_ark_payouts().await.unwrap()),
        0
    );
    assert_eq!(store.pending_for(&sp_id).await.unwrap(), 0);
}

/// SPEC §7.4 + review 4.5: a block booked without a distribution
/// fingerprint is `pending`, never `confirmed` (the CHECK constraint
/// forbids `confirmed` with NULL); it owes nothing until the event is
/// reconstructed, after which it confirms and books.
#[tokio::test]
async fn pending_settlement_accepts_null_fingerprint() {
    let Some(store) = store().await else { return };

    let sp_id = id();
    let fingerprint = id();
    let block_hash = id();
    identity(&store, sp_id).await;

    // Lost block event: booked pending with fingerprint NULL, no liability.
    store
        .settle(
            &Settlement {
                block_hash,
                height: 800_000,
                fingerprint: None,
                status: BlockStatus::Pending,
                ledger: vec![],
            },
            6,
        )
        .await
        .unwrap();
    assert_eq!(
        store.block_status(&block_hash).await.unwrap(),
        Some(BlockStatus::Pending)
    );
    assert_eq!(store.pending_for(&sp_id).await.unwrap(), 0);

    // A caller asking for `confirmed` with a NULL fingerprint is coerced to
    // `pending`, and no ledger rows are written either way.
    let stray = id();
    store
        .settle(
            &Settlement {
                block_hash: stray,
                height: 800_001,
                fingerprint: None,
                status: BlockStatus::Confirmed,
                ledger: vec![LedgerRow {
                    sp_id,
                    rail: LedgerRail::Forfeit,
                    paid_sats: 0,
                    ark_due_sats: 0,
                    forfeited_sats: 40,
                    pending_delta: 0,
                }],
            },
            6,
        )
        .await
        .unwrap();
    assert_eq!(
        store.block_status(&stray).await.unwrap(),
        Some(BlockStatus::Pending)
    );
    let rows = store.ledger_for(&sp_id, 10).await.unwrap();
    assert!(rows.iter().all(|(hash, ..)| *hash != stray));

    // Reconstruction sets the fingerprint; the block then confirms and books.
    snapshot(&store, fingerprint).await;
    store
        .settle(
            &Settlement {
                block_hash,
                height: 800_000,
                fingerprint: Some(fingerprint),
                status: BlockStatus::Confirmed,
                ledger: vec![LedgerRow {
                    sp_id,
                    rail: LedgerRail::Coinbase,
                    paid_sats: 1_000,
                    ark_due_sats: 0,
                    forfeited_sats: 0,
                    pending_delta: 0,
                }],
            },
            6,
        )
        .await
        .unwrap();
    assert_eq!(
        store.block_status(&block_hash).await.unwrap(),
        Some(BlockStatus::Confirmed)
    );
}

/// SPEC §5.1: a miner appears in a slice before registering Ark delivery.
/// `payout` books a clearly-marked placeholder identity so the
/// `slice_credit` FK holds; the later registration upsert replaces the
/// placeholder keys and network, and a late placeholder never clobbers a
/// registered identity.
#[tokio::test]
async fn placeholder_identity_lets_slices_book_and_registration_replaces_it() {
    let Some(store) = store().await else { return };

    let sp_id = id();
    let placeholder = SpIdentityRow {
        sp_id,
        b_scan: vec![0u8; 33],
        b_spend: vec![0u8; 33],
        network: 3,
    };
    store.insert_identity_if_absent(&placeholder).await.unwrap();
    assert_eq!(
        store.identity(&sp_id).await.unwrap(),
        Some(placeholder.clone())
    );

    // A slice credit for the not-yet-registered miner inserts cleanly.
    // `slice_id` is a BIGINT, so keep the random id inside i64's range.
    let slice_id = u64::from_be_bytes(id()[..8].try_into().unwrap()) & 0x7fff_ffff_ffff_ffff;
    store
        .insert_slice(&SliceRow {
            slice_id,
            height: 800_000,
            prev_hash: id(),
            ref_job_id: 1,
            ref_fees: 0,
            n_shares: 2,
            difficulty: "1".into(),
            root: id(),
            shares: vec![0u8; 8],
            work_unit_version: WorkUnitVersion::V1,
            work_unit_fingerprint: None,
            credits: vec![SliceCreditRow {
                sp_id,
                diff_sum: "12".into(),
                diff_fee_sum: "6".into(),
            }],
        })
        .await
        .unwrap();

    // The registration upsert replaces the placeholder keys and network.
    let real = SpIdentityRow {
        sp_id,
        b_scan: vec![2; 33],
        b_spend: vec![3; 33],
        network: 1,
    };
    store.upsert_identity(&real).await.unwrap();
    assert_eq!(store.identity(&sp_id).await.unwrap(), Some(real.clone()));

    // A late placeholder (a later slice before scrub) changes nothing.
    store.insert_identity_if_absent(&placeholder).await.unwrap();
    assert_eq!(store.identity(&sp_id).await.unwrap(), Some(real));
}

#[tokio::test]
async fn failed_ark_send_becomes_pending_never_forfeit() {
    let Some(store) = store().await else { return };

    let sp_id = id();
    let fingerprint = id();
    let block_hash = id();
    identity(&store, sp_id).await;
    snapshot(&store, fingerprint).await;

    store
        .settle(
            &Settlement {
                block_hash,
                height: 800_000,
                fingerprint: Some(fingerprint),
                status: BlockStatus::Confirmed,
                ledger: vec![LedgerRow {
                    sp_id,
                    rail: LedgerRail::Ark,
                    paid_sats: 0,
                    ark_due_sats: 75,
                    forfeited_sats: 0,
                    pending_delta: 0,
                }],
            },
            6,
        )
        .await
        .unwrap();

    store
        .mark_ark_failed_to_pending(&block_hash, &sp_id)
        .await
        .unwrap();

    let pending = store.pending_balances().await.unwrap();
    assert_eq!(
        pending
            .into_iter()
            .find(|(id, _)| *id == sp_id)
            .map(|(_, s)| s),
        Some(75)
    );
}

/// A slice id that fits the `BIGINT` column.
fn slice_id() -> u64 {
    u64::from_be_bytes(id()[..8].try_into().unwrap()) & 0x7fff_ffff_ffff_ffff
}

/// SPEC §4.3/§7.4: settlement reconstructs the window from the high-water
/// mark, so the store must return exactly the slices at or below it, newest
/// first, with their identity aggregates.
#[tokio::test]
async fn window_slices_to_returns_credits_up_to_the_high_water() {
    let Some(store) = store().await else { return };

    let alice = id();
    let bob = id();
    identity(&store, alice).await;
    identity(&store, bob).await;

    let older = slice_id();
    let newer = slice_id().max(older + 1);
    store
        .insert_slice(&SliceRow {
            slice_id: older,
            height: 800_000,
            prev_hash: id(),
            ref_job_id: 1,
            ref_fees: 0,
            n_shares: 3,
            difficulty: "300".into(),
            root: id(),
            shares: vec![1],
            work_unit_version: WorkUnitVersion::V1,
            work_unit_fingerprint: None,
            credits: vec![SliceCreditRow {
                sp_id: alice,
                diff_sum: "100".into(),
                diff_fee_sum: "5".into(),
            }],
        })
        .await
        .unwrap();
    store
        .insert_slice(&SliceRow {
            slice_id: newer,
            height: 800_000,
            prev_hash: id(),
            ref_job_id: 2,
            ref_fees: 0,
            n_shares: 2,
            difficulty: "200".into(),
            root: id(),
            shares: vec![2],
            work_unit_version: WorkUnitVersion::V1,
            work_unit_fingerprint: None,
            credits: vec![SliceCreditRow {
                sp_id: bob,
                diff_sum: "200".into(),
                diff_fee_sum: "7".into(),
            }],
        })
        .await
        .unwrap();

    // The mark includes both slices; check them by id (other tests may have
    // inserted slices concurrently) and verify the newest-first ordering of
    // our two rows at the same height.
    let both = store.window_slices_to(newer, 100).await.unwrap();
    assert!(both.iter().all(|s| s.slice_id <= newer));
    let newer_idx = both
        .iter()
        .position(|s| s.slice_id == newer)
        .expect("newer slice");
    let older_idx = both
        .iter()
        .position(|s| s.slice_id == older)
        .expect("older slice");
    assert!(newer_idx < older_idx);
    let newer_row = &both[newer_idx];
    assert_eq!(newer_row.credits.len(), 1);
    assert_eq!(newer_row.credits[0].sp_id, bob);
    assert_eq!(newer_row.credits[0].diff_sum, "200");
    assert_eq!(both[older_idx].credits[0].sp_id, alice);

    // A mark at the older slice excludes the newer one.
    let only_older = store.window_slices_to(older, 100).await.unwrap();
    assert!(only_older.iter().all(|s| s.slice_id <= older));
    assert!(only_older.iter().any(|s| s.slice_id == older));
    assert!(!only_older.iter().any(|s| s.slice_id == newer));
}

/// SPEC §4.7/§5.1: the snapshot's `entries` and `high_water` are the frozen
/// window metadata settlement reads back.
#[tokio::test]
async fn snapshot_round_trips_entries_and_high_water() {
    let Some(store) = store().await else { return };

    let fingerprint = id();
    let entries = serde_json::json!([{
        "sp_id": id(),
        "rail": "Coinbase",
        "p_x": vec![2u8; 32],
        "dest_id": [],
        "sats": 1_000u64,
    }]);
    let high_water = serde_json::json!({"0": "17"});
    store
        .insert_snapshot(&SnapshotRow {
            fingerprint,
            height: 800_000,
            a_send_x: [4; 32],
            distribution_id: Some(42),
            entries: entries.clone(),
            high_water: high_water.clone(),
            work_unit_version: WorkUnitVersion::V1,
            work_unit_fingerprint: None,
            window_rule: WindowRule::ShareWorkLegacy,
            window_target: Some("340282366920938463463374607431768211455".into()),
        })
        .await
        .unwrap();

    let loaded = store.snapshot(&fingerprint).await.unwrap().unwrap();
    assert_eq!(loaded.fingerprint, fingerprint);
    assert_eq!(loaded.height, 800_000);
    assert_eq!(loaded.distribution_id, Some(42));
    assert_eq!(loaded.entries, entries);
    assert_eq!(loaded.high_water, high_water);
    // u128::MAX round-trips exactly: N x d can exceed i64, and the
    // NUMERIC(40,0) column must never truncate it.
    assert_eq!(
        loaded.window_target.as_deref(),
        Some("340282366920938463463374607431768211455")
    );
    assert!(store.snapshot(&id()).await.unwrap().is_none());
}

/// SPEC §5.1: `window_target` is nullable, so rows written before the
/// migration (which carry no value) keep reading back as `None`.
#[tokio::test]
async fn snapshot_window_target_is_nullable_for_pre_migration_rows() {
    let Some(store) = store().await else { return };

    let fingerprint = id();
    store
        .insert_snapshot(&SnapshotRow {
            fingerprint,
            height: 800_000,
            a_send_x: [4; 32],
            distribution_id: None,
            entries: serde_json::json!([]),
            high_water: serde_json::json!({"0": "1"}),
            work_unit_version: WorkUnitVersion::V1,
            work_unit_fingerprint: None,
            window_rule: WindowRule::ShareWorkLegacy,
            window_target: None,
        })
        .await
        .unwrap();
    let loaded = store.snapshot(&fingerprint).await.unwrap().unwrap();
    assert_eq!(loaded.window_target, None);
}

/// SPEC §4.6/§7.4: pending deltas are the settlement-time surface, and a
/// replay writes nothing twice.
#[tokio::test]
async fn replayed_settlement_with_pending_delta_is_idempotent() {
    let Some(store) = store().await else { return };

    let sp_id = id();
    let fingerprint = id();
    let block_hash = id();
    identity(&store, sp_id).await;
    snapshot(&store, fingerprint).await;

    let settlement = Settlement {
        block_hash,
        height: 800_000,
        fingerprint: Some(fingerprint),
        status: BlockStatus::Confirmed,
        ledger: vec![LedgerRow {
            sp_id,
            rail: LedgerRail::Pending,
            paid_sats: 0,
            ark_due_sats: 0,
            forfeited_sats: 0,
            pending_delta: 50,
        }],
    };
    store.settle(&settlement, 6).await.unwrap();
    assert_eq!(store.pending_for(&sp_id).await.unwrap(), 50);

    assert!(store.settle(&settlement, 6).await.is_err());
    assert_eq!(store.pending_for(&sp_id).await.unwrap(), 50);
    let rows = store.ledger_for(&sp_id, 10).await.unwrap();
    assert_eq!(
        rows.iter().filter(|(hash, ..)| *hash == block_hash).count(),
        1
    );
}

/// SPEC §7.4: an orphaned block writes no ledger rows and reverses a pending
/// delta that was already applied.
#[tokio::test]
async fn orphan_books_no_ledger_and_reverses_pending() {
    let Some(store) = store().await else { return };

    let sp_id = id();
    let fingerprint = id();
    let block_hash = id();
    identity(&store, sp_id).await;
    snapshot(&store, fingerprint).await;

    store
        .settle(
            &Settlement {
                block_hash,
                height: 800_000,
                fingerprint: Some(fingerprint),
                status: BlockStatus::Confirmed,
                ledger: vec![LedgerRow {
                    sp_id,
                    rail: LedgerRail::Pending,
                    paid_sats: 0,
                    ark_due_sats: 0,
                    forfeited_sats: 0,
                    pending_delta: 80,
                }],
            },
            6,
        )
        .await
        .unwrap();
    assert_eq!(store.pending_for(&sp_id).await.unwrap(), 80);

    store.orphan(&block_hash).await.unwrap();
    assert_eq!(
        store.block_status(&block_hash).await.unwrap(),
        Some(BlockStatus::Orphaned)
    );
    assert!(store.ledger_for(&sp_id, 10).await.unwrap().is_empty());
    assert_eq!(store.pending_for(&sp_id).await.unwrap(), 0);
}

/// SPEC §7.4: a block with no entry for an id leaves that id's carried
/// pending balance untouched (pending-only ids are not silently dropped).
#[tokio::test]
async fn pending_only_balance_survives_a_later_block() {
    let Some(store) = store().await else { return };

    let carried = id();
    let other = id();
    identity(&store, carried).await;
    identity(&store, other).await;

    let first_fingerprint = id();
    let second_fingerprint = id();
    snapshot(&store, first_fingerprint).await;
    snapshot(&store, second_fingerprint).await;

    store
        .settle(
            &Settlement {
                block_hash: id(),
                height: 800_000,
                fingerprint: Some(first_fingerprint),
                status: BlockStatus::Confirmed,
                ledger: vec![LedgerRow {
                    sp_id: carried,
                    rail: LedgerRail::Pending,
                    paid_sats: 0,
                    ark_due_sats: 0,
                    forfeited_sats: 0,
                    pending_delta: 120,
                }],
            },
            6,
        )
        .await
        .unwrap();
    assert_eq!(store.pending_for(&carried).await.unwrap(), 120);

    // A later block pays someone else; the carried balance is unchanged.
    store
        .settle(
            &Settlement {
                block_hash: id(),
                height: 800_001,
                fingerprint: Some(second_fingerprint),
                status: BlockStatus::Confirmed,
                ledger: vec![LedgerRow {
                    sp_id: other,
                    rail: LedgerRail::Coinbase,
                    paid_sats: 1_000,
                    ark_due_sats: 0,
                    forfeited_sats: 0,
                    pending_delta: 0,
                }],
            },
            6,
        )
        .await
        .unwrap();
    assert_eq!(store.pending_for(&carried).await.unwrap(), 120);
    let pending = store.pending_map().await.unwrap();
    assert_eq!(pending.get(&carried), Some(&120));
}

/// The settlement-time rail re-decision needs the set of ids with an
/// `ark_delivery` row, not just a per-id lookup.
#[tokio::test]
async fn ark_delivery_ids_reports_registered_miners() {
    let Some(store) = store().await else { return };

    let registered = id();
    let plain = id();
    identity(&store, registered).await;
    identity(&store, plain).await;
    store
        .upsert_ark_delivery(&ArkDeliveryRow {
            registration_commit: None,
            sp_id: registered,
            ark_id: [7; 4],
            b_ark: vec![5; 33],
            mailbox_mode: MailboxMode::None,
            mailbox_m: None,
            mailbox_d: None,
            epoch_fallback: EpochFallback::Static,
        })
        .await
        .unwrap();

    let ids = store.ark_delivery_ids(&[registered, plain]).await.unwrap();
    assert!(ids.contains(&registered));
    assert!(!ids.contains(&plain));
}

/// SPEC §5.3 / WIRE-FORMATS §3.5: a front that restarts builds its window
/// from `bootstrap().window_tail`, so the tail must carry the same
/// per-identity credits as `window_slices_to`, not just the slice headers.
#[tokio::test]
async fn bootstrap_window_tail_carries_slice_credits() {
    let Some(store) = store().await else { return };

    let alice = id();
    identity(&store, alice).await;

    let sid = slice_id();
    store
        .insert_slice(&SliceRow {
            slice_id: sid,
            height: 800_000,
            prev_hash: id(),
            ref_job_id: 3,
            ref_fees: 0,
            n_shares: 4,
            difficulty: "400".into(),
            root: id(),
            shares: vec![9, 9, 9],
            work_unit_version: WorkUnitVersion::V1,
            work_unit_fingerprint: None,
            credits: vec![SliceCreditRow {
                sp_id: alice,
                diff_sum: "44".into(),
                diff_fee_sum: "9".into(),
            }],
        })
        .await
        .unwrap();

    // `retain_windows` is huge so a concurrently inserted slice cannot push
    // this row out of the tail.
    let snapshot = store.bootstrap(i64::MAX).await.unwrap();
    let tail_row = snapshot
        .window_tail
        .iter()
        .find(|s| s.slice_id == sid)
        .expect("slice is in the bootstrap window tail");
    assert_eq!(tail_row.credits.len(), 1);
    assert_eq!(tail_row.credits[0].sp_id, alice);
    assert_eq!(tail_row.credits[0].diff_sum, "44");
    assert_eq!(tail_row.credits[0].diff_fee_sum, "9");
    assert_eq!(tail_row.shares, vec![9, 9, 9]);

    // The tail feeds the same window builder as `window_slices_to`.
    let direct = store
        .window_slices_to(sid, i64::MAX)
        .await
        .unwrap()
        .into_iter()
        .find(|s| s.slice_id == sid)
        .expect("slice is in the high-water walk");
    assert_eq!(tail_row.credits, direct.credits);
}

/// SPEC §5.1: scrub deletes old `slice_credit`, clears `entries` only for
/// fully settled blocks, and keeps the snapshot while an Ark payout is still
/// scheduled. It also drops a matured sent payout's `vtxo_id`.
#[tokio::test]
async fn scrub_preserves_scheduled_ark_snapshots_and_clears_settled_ones() {
    let Some(store) = store().await else { return };

    let scheduled_miner = id();
    let plain_miner = id();
    let sent_miner = id();
    for sp in [scheduled_miner, plain_miner, sent_miner] {
        identity(&store, sp).await;
    }

    // Old slice credit (height far below the retention window).
    let old_slice = slice_id();
    store
        .insert_slice(&SliceRow {
            slice_id: old_slice,
            height: 1_000,
            prev_hash: id(),
            ref_job_id: 1,
            ref_fees: 0,
            n_shares: 1,
            difficulty: "10".into(),
            root: id(),
            shares: vec![1],
            work_unit_version: WorkUnitVersion::V1,
            work_unit_fingerprint: None,
            credits: vec![SliceCreditRow {
                sp_id: scheduled_miner,
                diff_sum: "10".into(),
                diff_fee_sum: "1".into(),
            }],
        })
        .await
        .unwrap();

    // A confirmed block with an Ark row still scheduled: entries must stay.
    let scheduled_fp = id();
    let scheduled_block = id();
    store
        .insert_snapshot(&SnapshotRow {
            fingerprint: scheduled_fp,
            height: 1_000,
            a_send_x: [4; 32],
            distribution_id: None,
            entries: serde_json::json!([{
                "sp_id": scheduled_miner,
                "rail": "Ark",
                "p_x": [],
                "dest_id": b"tark1destination",
                "sats": 250u64,
            }]),
            high_water: serde_json::json!({"0": "1"}),
            work_unit_version: WorkUnitVersion::V1,
            work_unit_fingerprint: None,
            window_rule: WindowRule::ShareWorkLegacy,
            window_target: None,
        })
        .await
        .unwrap();
    store
        .settle(
            &Settlement {
                block_hash: scheduled_block,
                height: 1_000,
                fingerprint: Some(scheduled_fp),
                status: BlockStatus::Confirmed,
                ledger: vec![LedgerRow {
                    sp_id: scheduled_miner,
                    rail: LedgerRail::Ark,
                    paid_sats: 0,
                    ark_due_sats: 250,
                    forfeited_sats: 0,
                    pending_delta: 0,
                }],
            },
            6,
        )
        .await
        .unwrap();

    // The emission view carries the frozen destination.
    let emissions = store.scheduled_ark_emissions().await.unwrap();
    let emission = emissions
        .iter()
        .find(|e| e.block_hash == scheduled_block)
        .expect("scheduled emission");
    assert_eq!(emission.dest_id, b"tark1destination");
    assert_eq!(emission.due_height, 1_006);

    // A confirmed block with nothing scheduled: entries are scrubbed.
    let plain_fp = id();
    store
        .insert_snapshot(&SnapshotRow {
            fingerprint: plain_fp,
            height: 1_000,
            a_send_x: [4; 32],
            distribution_id: None,
            entries: serde_json::json!([{
                "sp_id": plain_miner,
                "rail": "Coinbase",
                "p_x": vec![2u8; 32],
                "dest_id": [],
                "sats": 1_000u64,
            }]),
            high_water: serde_json::json!({"0": "1"}),
            work_unit_version: WorkUnitVersion::V1,
            work_unit_fingerprint: None,
            window_rule: WindowRule::ShareWorkLegacy,
            window_target: None,
        })
        .await
        .unwrap();
    store
        .settle(
            &Settlement {
                block_hash: id(),
                height: 1_000,
                fingerprint: Some(plain_fp),
                status: BlockStatus::Confirmed,
                ledger: vec![LedgerRow {
                    sp_id: plain_miner,
                    rail: LedgerRail::Coinbase,
                    paid_sats: 1_000,
                    ark_due_sats: 0,
                    forfeited_sats: 0,
                    pending_delta: 0,
                }],
            },
            6,
        )
        .await
        .unwrap();

    // An old, already sent payout whose VTXO id must be dropped.
    let sent_fp = id();
    let sent_block = id();
    store
        .insert_snapshot(&SnapshotRow {
            fingerprint: sent_fp,
            height: 1_000,
            a_send_x: [4; 32],
            distribution_id: None,
            entries: serde_json::json!([]),
            high_water: serde_json::json!({"0": "1"}),
            work_unit_version: WorkUnitVersion::V1,
            work_unit_fingerprint: None,
            window_rule: WindowRule::ShareWorkLegacy,
            window_target: None,
        })
        .await
        .unwrap();
    store
        .settle(
            &Settlement {
                block_hash: sent_block,
                height: 1_000,
                fingerprint: Some(sent_fp),
                status: BlockStatus::Confirmed,
                ledger: vec![LedgerRow {
                    sp_id: sent_miner,
                    rail: LedgerRail::Ark,
                    paid_sats: 0,
                    ark_due_sats: 250,
                    forfeited_sats: 0,
                    pending_delta: 0,
                }],
            },
            6,
        )
        .await
        .unwrap();
    store
        .mark_ark_sent(&sent_block, &sent_miner, &[9; 34], 900_000)
        .await
        .unwrap();

    // tip 10_000: retain = 9_984, scrub_after = 9_900.
    store.scrub(10_000, 16, 4_096, 100).await.unwrap();

    let slices = store.window_slices_to(old_slice, i64::MAX).await.unwrap();
    let old = slices
        .iter()
        .find(|s| s.slice_id == old_slice)
        .expect("slice survives");
    assert!(old.credits.is_empty(), "old slice credits are scrubbed");

    let scheduled = store.snapshot(&scheduled_fp).await.unwrap().unwrap();
    assert_ne!(
        scheduled.entries,
        serde_json::json!([]),
        "scheduled Ark payouts keep their entries"
    );
    let plain = store.snapshot(&plain_fp).await.unwrap().unwrap();
    assert_eq!(plain.entries, serde_json::json!([]));

    let row = sqlx::query("SELECT vtxo_id FROM ark_payout WHERE block_hash = $1")
        .bind(&sent_block[..])
        .fetch_one(store.pool())
        .await
        .unwrap();
    let vtxo: Option<Vec<u8>> = row.try_get("vtxo_id").unwrap();
    assert!(vtxo.is_none(), "matured sent VTXO ids are scrubbed");
}

/// SPEC §5.2/§7.4: the `spc:front:events` watermark is persisted, strictly
/// increasing, and returned to a restarting front by `bootstrap`.
#[tokio::test]
async fn front_event_rev_is_strictly_increasing_and_bootstrapped() {
    let Some(store) = store().await else { return };

    let first = store.bump_front_rev().await.unwrap();
    let second = store.bump_front_rev().await.unwrap();
    assert!(second > first, "the watermark is strictly increasing");
    assert!(store.front_rev().await.unwrap() >= second);

    let snapshot = store.bootstrap(16).await.unwrap();
    assert!(
        snapshot.rev >= second,
        "bootstrap must carry at least the last published rev"
    );
    // A second bootstrap (a front restart) sees the same persisted value.
    let again = store.bootstrap(16).await.unwrap();
    assert!(again.rev >= snapshot.rev);
}

/// SPEC §5.2/§7.4: a bootstrap reads pending balances, the confirmed-block
/// list and the event watermark from one repeatable-read snapshot, so a
/// front's pending projection and block-hash dedupe cannot disagree.
#[tokio::test]
async fn bootstrap_reports_pending_rev_and_applied_blocks_together() {
    let Some(store) = store().await else { return };

    let sp_id = id();
    let fingerprint = id();
    let block = id();
    identity(&store, sp_id).await;
    snapshot(&store, fingerprint).await;

    let rev = store
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
                    pending_delta: 40,
                }],
            },
            6,
        )
        .await
        .unwrap()
        .expect("a confirmed booking returns its rev");

    let boot = store.bootstrap(16).await.unwrap();
    assert!(boot.rev >= rev);
    assert!(boot.applied_blocks.contains(&block));
    assert!(boot
        .pending_balance
        .iter()
        .any(|(id, sats)| *id == sp_id && *sats == 40));
    assert!(boot.applied_blocks.len() <= usize::try_from(APPLIED_BLOCKS_LIMIT).unwrap());
}

/// SPEC §5.2/§7.4: a lost `LedgerApplied` is recovered from the durable
/// ledger; only confirmed, non-zero deltas are replayed.
#[tokio::test]
async fn recent_ledger_events_returns_nonzero_confirmed_deltas() {
    let Some(store) = store().await else { return };

    let pending_id = id();
    let paid_id = id();
    let fingerprint = id();
    let block = id();
    identity(&store, pending_id).await;
    identity(&store, paid_id).await;
    snapshot(&store, fingerprint).await;

    store
        .settle(
            &Settlement {
                block_hash: block,
                height: 800_000,
                fingerprint: Some(fingerprint),
                status: BlockStatus::Confirmed,
                ledger: vec![
                    LedgerRow {
                        sp_id: pending_id,
                        rail: LedgerRail::Pending,
                        paid_sats: 0,
                        ark_due_sats: 0,
                        forfeited_sats: 0,
                        pending_delta: 50,
                    },
                    LedgerRow {
                        sp_id: paid_id,
                        rail: LedgerRail::Coinbase,
                        paid_sats: 1_000,
                        ark_due_sats: 0,
                        forfeited_sats: 0,
                        pending_delta: 0,
                    },
                ],
            },
            6,
        )
        .await
        .unwrap();

    let events = store.recent_ledger_events(1_000).await.unwrap();
    let (_, deltas) = events
        .iter()
        .find(|(hash, _)| *hash == block)
        .expect("confirmed block is replayed");
    assert_eq!(deltas, &vec![(pending_id, 50)]);
    assert!(
        deltas.iter().all(|(id, _)| *id != paid_id),
        "zero-delta rows need no replay"
    );
}

/// The settle transaction commits the ledger and the event revision
/// together; a pending (no-ledger) booking returns `None` and bumps nothing.
#[tokio::test]
async fn settle_bumps_the_front_rev_atomically() {
    let Some(store) = store().await else { return };

    let sp_id = id();
    let fingerprint = id();
    identity(&store, sp_id).await;
    snapshot(&store, fingerprint).await;

    let before = store.front_rev().await.unwrap();
    let rev = store
        .settle(
            &Settlement {
                block_hash: id(),
                height: 800_000,
                fingerprint: Some(fingerprint),
                status: BlockStatus::Confirmed,
                ledger: vec![LedgerRow {
                    sp_id,
                    rail: LedgerRail::Pending,
                    paid_sats: 0,
                    ark_due_sats: 0,
                    forfeited_sats: 0,
                    pending_delta: 10,
                }],
            },
            6,
        )
        .await
        .unwrap()
        .expect("rev");
    assert!(rev > before);
    assert!(store.front_rev().await.unwrap() >= rev);

    let pending = store
        .settle(
            &Settlement {
                block_hash: id(),
                height: 800_001,
                fingerprint: None,
                status: BlockStatus::Pending,
                ledger: Vec::new(),
            },
            6,
        )
        .await
        .unwrap();
    assert!(
        pending.is_none(),
        "pending bookings publish no LedgerApplied"
    );
}

/// The read-only `/metrics` queries (SPEC §9) agree with what settlement
/// wrote: one confirmed block with an Ark payout and a forfeit.
#[tokio::test]
async fn metrics_queries_read_back_settlement() {
    let Some((store, admin, database)) = isolated_store("spc_metrics_").await else {
        return;
    };

    let sp_id = id();
    let fingerprint = id();
    let block_hash = id();
    identity(&store, sp_id).await;
    snapshot(&store, fingerprint).await;

    store
        .settle(
            &Settlement {
                block_hash,
                height: 800_000,
                fingerprint: Some(fingerprint),
                status: BlockStatus::Confirmed,
                ledger: vec![LedgerRow {
                    sp_id,
                    rail: LedgerRail::Ark,
                    paid_sats: 0,
                    ark_due_sats: 250,
                    forfeited_sats: 40,
                    pending_delta: 0,
                }],
            },
            6,
        )
        .await
        .unwrap();

    // Exact totals for an isolated fixture, not global assertions or deltas
    // that another test could change concurrently.
    assert_eq!(
        store.blocks_by_status().await.unwrap(),
        vec![("confirmed".to_string(), 1)]
    );
    assert_eq!(
        store.ark_payouts_by_status().await.unwrap(),
        vec![("scheduled".to_string(), 1)]
    );
    assert_eq!(store.ark_due_sats().await.unwrap(), 250);
    assert_eq!(store.forfeited_sats_total().await.unwrap(), 40);

    store.pool().close().await;
    sqlx::query(&format!("DROP DATABASE {database} WITH (FORCE)"))
        .execute(&admin)
        .await
        .unwrap();
}

/// The 0004 migration must apply to a database last migrated at 0003, and a
/// row written before it (no `window_target`) must keep reading back as
/// `None` - never a zero, never a guess. Proven on an isolated scratch
/// database because the suite's shared database is used in parallel and this
/// test rewinds the schema.
#[tokio::test]
async fn window_target_migration_applies_to_an_existing_database() {
    let Some(url) = std::env::var("SPC_TEST_DATABASE_URL").ok() else {
        return;
    };
    // Same server, maintenance database: the harness runs a throwaway
    // cluster where CREATE DATABASE is permitted. When it is not (a
    // restricted CI user), this test cannot isolate itself and skips.
    let admin_url = {
        let base = url.split('?').next().unwrap_or(&url);
        let (head, _) = base
            .rsplit_once('/')
            .expect("SPC_TEST_DATABASE_URL has a database path");
        format!("{head}/postgres")
    };
    let name = format!(
        "spc_migrate_{}",
        hex::encode(uuid::Uuid::new_v4().as_bytes())
    );
    let admin = sqlx::PgPool::connect(&admin_url).await.unwrap();
    if sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .is_err()
    {
        eprintln!("skip: no CREATE DATABASE privilege on the test server");
        return;
    }
    let scratch_url = {
        let base = url.split('?').next().unwrap_or(&url);
        let (head, _) = base
            .rsplit_once('/')
            .expect("SPC_TEST_DATABASE_URL has a database path");
        format!("{head}/{name}")
    };

    // Build the database as it was before 0004: apply 0001-0003 only, with
    // the migrator's own bookkeeping so the later full run applies 0004.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let all = sqlx::migrate::Migrator::new(dir.as_path()).await.unwrap();
    let pre_0004 = sqlx::migrate::Migrator {
        migrations: std::borrow::Cow::Owned(
            all.migrations
                .iter()
                .filter(|m| m.version <= 3)
                .cloned()
                .collect(),
        ),
        ignore_missing: false,
        locking: true,
        no_tx: false,
    };
    {
        let scratch = sqlx::PgPool::connect(&scratch_url).await.unwrap();
        pre_0004.run(&scratch).await.unwrap();

        // A snapshot row in the pre-migration shape (no window_target).
        sqlx::query(
            "INSERT INTO distribution_snapshot
                (fingerprint, height, a_send_x, distribution_id, entries, high_water)
             VALUES ($1, 800000, $2, NULL, '[]'::jsonb, $3::jsonb)",
        )
        .bind(id())
        .bind([4u8; 32])
        .bind(r#"{"0": "1"}"#)
        .execute(&scratch)
        .await
        .unwrap();
        scratch.close().await;
    }

    // The production path: Store::connect + migrate() applies 0004 on top.
    let store = Store::connect(&scratch_url).await.unwrap();
    store.migrate().await.unwrap();

    // The pre-migration row is readable and reports no target.
    let row = sqlx::query(
        "SELECT count(*) AS n,
                bool_and(window_target IS NULL) AS all_null
         FROM distribution_snapshot",
    )
    .fetch_one(store.pool())
    .await
    .unwrap();
    let n: i64 = row.try_get("n").unwrap();
    let all_null: bool = row.try_get("all_null").unwrap();
    assert_eq!(n, 1);
    assert!(all_null, "pre-migration row keeps NULL window_target");

    // New rows carry the exact u128 target.
    let fingerprint = id();
    store
        .insert_snapshot(&SnapshotRow {
            fingerprint,
            height: 800_000,
            a_send_x: [4; 32],
            distribution_id: None,
            entries: serde_json::json!([]),
            high_water: serde_json::json!({"0": "1"}),
            work_unit_version: WorkUnitVersion::V1,
            work_unit_fingerprint: None,
            window_rule: WindowRule::ShareWorkLegacy,
            window_target: Some("340282366920938463463374607431768211455".into()),
        })
        .await
        .unwrap();
    let loaded = store.snapshot(&fingerprint).await.unwrap().unwrap();
    assert_eq!(
        loaded.window_target.as_deref(),
        Some("340282366920938463463374607431768211455")
    );
    store.pool().close().await;
    sqlx::query(&format!("DROP DATABASE {name} WITH (FORCE)"))
        .execute(&admin)
        .await
        .unwrap();
}
