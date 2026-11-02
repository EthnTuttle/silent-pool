//! Postgres integration tests. Skipped unless `SPC_TEST_DATABASE_URL` points
//! at a throwaway database (the e2e harness sets it).
//!
//! These are the tests that prove SPEC §7.4's idempotency, orphan and
//! liability rules at the real storage boundary.

use spc_store::models::*;
use spc_store::Store;
use uuid::Uuid;

async fn store() -> Option<Store> {
    let url = std::env::var("SPC_TEST_DATABASE_URL").ok()?;
    let store = Store::connect(&url).await.expect("connect");
    store.migrate().await.expect("migrate");
    Some(store)
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
    assert_eq!(store.liability_sats().await.unwrap(), 0);
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
    assert_eq!(store.liability_sats().await.unwrap(), 250);

    store
        .mark_ark_sent(&block_hash, &sp_id, &[9; 34], 900_000)
        .await
        .unwrap();
    assert_eq!(store.liability_sats().await.unwrap(), 0);
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
    assert_eq!(store.liability_sats().await.unwrap(), 0);

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
