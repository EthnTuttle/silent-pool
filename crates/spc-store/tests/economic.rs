#![cfg(feature = "postgres-integration")]
use spc_accounting::{economic::FrozenJob, WindowRule, WorkUnitVersion};
use spc_store::{
    models::*,
    wire::{BlockEvent, Entry, Rail},
    Store,
};
use std::collections::BTreeMap;

async fn database() -> Store {
    let url = std::env::var("SPC_LADDER_TEST_DATABASE_URL")
        .expect("owned PostgreSQL required; never skipped");
    assert!(url.contains("ladder_completion"));
    let admin = Store::connect(&url).await.unwrap();
    let name = format!("ladder_completion_b_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(admin.pool())
        .await
        .unwrap();
    let store = Store::connect(&url.replace("/ladder_completion?", &format!("/{name}?")))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    store.economic_initialize([9; 32]).await.unwrap();
    store
        .upsert_identity(&SpIdentityRow {
            sp_id: [1; 32],
            b_scan: vec![2; 33],
            b_spend: vec![3; 33],
            network: 3,
        })
        .await
        .unwrap();
    store
}
async fn seed(store: &Store, sats: i64) {
    sqlx::query("INSERT INTO pending_balance(sp_id,sats) VALUES($1,$2)")
        .bind(&[1u8; 32][..])
        .bind(sats)
        .execute(store.pool())
        .await
        .unwrap();
}
async fn found(store: &Store, height: u32, delta: i64) -> (BlockEvent, Settlement) {
    let view = store.economic_view().await.unwrap();
    let b =
        view.balances()
            .unwrap()
            .remove(&[1; 32])
            .unwrap_or(spc_accounting::economic::Balance {
                sp_id: [1; 32],
                booked: 0,
                reserved: 0,
                available: 0,
            });
    let prior = b.available;
    let e = BlockEvent {
        economic: Some(FrozenJob {
            version: 1,
            revision: view.revision,
            parent_hash: view.anchor_hash,
            job_id: u64::from(height),
            generation: u64::from(height),
            balances: vec![b],
            slice_ids: vec![],
        }),
        block_hash: [height as u8; 32],
        height,
        front_id: 0,
        fingerprint: [8; 32],
        a_send_x: [4; 32],
        high_water: BTreeMap::new(),
        window_target: 9,
        entries: vec![Entry {
            sp_id: [1; 32],
            owed_sats: prior + 1_100_000,
            rail: Rail::Pending,
            p_x: vec![],
            dest_id: vec![],
            sats: prior + 1_100_000,
        }],
        work_unit_version: WorkUnitVersion::V2,
        work_unit_fingerprint: Some([5; 32]),
        window_rule: WindowRule::NetworkBlockWork,
    };
    let rows = vec![LedgerRow {
        sp_id: [1; 32],
        rail: LedgerRail::Pending,
        paid_sats: 0,
        ark_due_sats: 0,
        forfeited_sats: 0,
        pending_delta: delta,
    }];
    let fp = e.snapshot_id().unwrap();
    store
        .insert_snapshot(&SnapshotRow {
            fingerprint: fp,
            height,
            a_send_x: e.a_send_x,
            distribution_id: None,
            entries: serde_json::json!(e.entries),
            high_water: serde_json::json!({}),
            window_target: Some("9".into()),
            work_unit_version: e.work_unit_version,
            work_unit_fingerprint: e.work_unit_fingerprint,
            window_rule: e.window_rule,
        })
        .await
        .unwrap();
    store.economic_found(&e, &rows, false).await.unwrap();
    store
        .economic_advance(height, e.block_hash, view.anchor_hash, true, [0; 32])
        .await
        .unwrap();
    let s = Settlement {
        block_hash: e.block_hash,
        height,
        fingerprint: Some(fp),
        status: BlockStatus::Confirmed,
        ledger: rows,
    };
    (e, s)
}

#[tokio::test]
async fn consecutive_net_debits_book_in_either_order_and_frozen_p_is_preserved() {
    for reverse in [false, true] {
        let store = database().await;
        seed(&store, 500_000).await;
        let (e1, s1) = found(&store, 1, -400_000).await;
        assert_eq!(
            store.economic_view().await.unwrap().balances().unwrap()[&[1; 32]].available,
            100_000
        );
        let (e2, s2) = found(&store, 2, -100_000).await;
        assert_eq!(
            e2.economic.as_ref().unwrap().prior().unwrap()[&[1; 32]],
            100_000
        );
        let (a, b) = if reverse { (&s2, &s1) } else { (&s1, &s2) };
        let one = store.clone();
        let two = store.clone();
        let (ra, rb) = tokio::join!(one.settle(a, 6), two.settle(b, 6));
        ra.unwrap();
        rb.unwrap();
        let v = store.economic_view().await.unwrap();
        let balance = &v.balances().unwrap()[&[1; 32]];
        assert_eq!(
            (balance.booked, balance.reserved, balance.available),
            (0, 0, 0)
        );
        assert!(!store.economic_found(&e1, &s1.ledger, false).await.unwrap());
        let mut bad = e1.clone();
        bad.entries[0].owed_sats += 1;
        assert!(store.economic_found(&bad, &s1.ledger, false).await.is_err());
        assert_eq!(e1.economic.unwrap().prior().unwrap()[&[1; 32]], 500_000);
    }
}
#[tokio::test]
async fn unbooked_positive_credit_is_unavailable_and_orphan_never_reverses_it() {
    let store = database().await;
    let (event, s) = found(&store, 1, 1_000_000).await;
    let v = store.economic_view().await.unwrap();
    assert_eq!(v.balances().unwrap()[&[1; 32]].available, 0);
    store.orphan(&event.block_hash).await.unwrap();
    assert_eq!(store.pending_for(&[1; 32]).await.unwrap(), 0);
    assert!(!store
        .economic_found(&event, &s.ledger, false)
        .await
        .unwrap());
    store.economic_rewind(0, [9; 32]).await.unwrap();
    assert!(store.economic_found(&event, &s.ledger, true).await.unwrap());
    store
        .economic_advance(1, event.block_hash, [9; 32], true, [0; 32])
        .await
        .unwrap();
    store.settle(&s, 6).await.unwrap();
    assert_eq!(store.pending_for(&[1; 32]).await.unwrap(), 1_000_000);
    let boot = store.bootstrap(16).await.unwrap();
    let v = boot.economic.unwrap();
    assert!(v.complete);
    assert_eq!(v.balances().unwrap()[&[1; 32]].available, 1_000_000);
}
#[tokio::test]
async fn bootstrap_gaps_and_unknown_pool_ancestors_never_default_to_zero() {
    let store = database().await;
    seed(&store, 500_000).await;
    assert!(store
        .economic_advance(1, [1; 32], [9; 32], true, [0; 32])
        .await
        .is_err());
    store.economic_hold("unknown pool event").await.unwrap();
    let view = store.bootstrap(16).await.unwrap().economic.unwrap();
    assert!(view.balances().is_err());
    assert!(view.at_parent(0, [9; 32]).is_err());
    store
        .economic_advance(1, [2; 32], [9; 32], false, [7; 32])
        .await
        .unwrap(); // caller supplied real non-pool evidence in production
    assert_eq!(
        store.economic_view().await.unwrap().balances().unwrap()[&[1; 32]].available,
        500_000
    );
}

#[tokio::test]
async fn booked_reorg_reconciles_dependent_reservations_before_exposing_balances() {
    for delta in [-400_000, 100_000] {
        let store = database().await;
        seed(&store, 500_000).await;
        let (parent, settlement) = found(&store, 1, delta).await;
        store.settle(&settlement, 6).await.unwrap();
        let booked = (500_000i64 + delta) as u64;
        let (child, _) = found(&store, 2, -(booked as i64)).await;
        if delta > 0 {
            assert!(
                store.orphan(&parent.block_hash).await.is_err(),
                "cannot expose B<R by reversing a funding ancestor first"
            );
            assert_eq!(store.pending_for(&[1; 32]).await.unwrap(), booked);
        }
        store.orphan(&child.block_hash).await.unwrap();
        store.orphan(&parent.block_hash).await.unwrap();
        store.orphan(&parent.block_hash).await.unwrap();
        assert_eq!(store.pending_for(&[1; 32]).await.unwrap(), 500_000);
        assert!(
            !store
                .bootstrap(16)
                .await
                .unwrap()
                .economic
                .unwrap()
                .complete
        );
        store.economic_rewind(0, [9; 32]).await.unwrap();
        assert_eq!(
            store.economic_view().await.unwrap().balances().unwrap()[&[1; 32]].available,
            500_000
        );
    }
}

#[tokio::test]
async fn recanonicalization_rejects_changed_validated_effects() {
    let store = database().await;
    seed(&store, 500_000).await;
    let (event, settlement) = found(&store, 1, -400_000).await;
    store.orphan(&event.block_hash).await.unwrap();
    store.economic_rewind(0, [9; 32]).await.unwrap();
    let mut changed = settlement.ledger.clone();
    changed[0].pending_delta = -100_000;
    assert!(store.economic_found(&event, &changed, true).await.is_err());
    assert_eq!(
        store
            .economic_ledger(&event.block_hash)
            .await
            .unwrap()
            .unwrap(),
        settlement.ledger
    );
    assert!(store
        .economic_found(&event, &settlement.ledger, true)
        .await
        .unwrap());
}

#[tokio::test]
async fn fresh_initial_row_checkpoint_replays_state_history_and_rejects_altered_or_missing_seed() {
    use sqlx::Row;
    let store = database().await;
    let record = store.economic_checkpoint_record().await.unwrap().unwrap();
    assert_eq!(record["classification"], "fresh_empty");
    assert_eq!(record["economic_revision"], 0);
    let row = sqlx::query(
        "SELECT state_bytes,history_watermark FROM economic_checkpoint WHERE checkpoint_id=1",
    )
    .fetch_one(store.pool())
    .await
    .unwrap();
    let mut state: serde_json::Value =
        serde_json::from_slice(&row.get::<Vec<u8>, _>("state_bytes")).unwrap();
    assert!(state["anchor_hash"].is_null());
    assert_eq!(state["revision"], 0);
    let events=sqlx::query("SELECT before_row,after_row FROM source_accounting_history WHERE table_name='economic_state' AND event_id>$1 ORDER BY event_id")
        .bind(row.get::<i64,_>("history_watermark")).fetch_all(store.pool()).await.unwrap();
    assert!(!events.is_empty());
    for event in events {
        assert_eq!(event.get::<serde_json::Value, _>("before_row"), state);
        state = event.get("after_row");
    }
    let actual: serde_json::Value =
        sqlx::query_scalar("SELECT to_jsonb(s) FROM economic_state s WHERE id=1")
            .fetch_one(store.pool())
            .await
            .unwrap();
    assert_eq!(
        state, actual,
        "first row plus actual committed transitions reconstruct the anchor"
    );
    assert!(store.economic_view().await.unwrap().complete);
    sqlx::query("UPDATE economic_checkpoint SET state_bytes=$1 WHERE checkpoint_id=1")
        .bind(b"{}".as_slice())
        .execute(store.pool())
        .await
        .unwrap();
    assert!(store.economic_view().await.is_err());
    sqlx::query("DELETE FROM economic_checkpoint WHERE checkpoint_id=1")
        .execute(store.pool())
        .await
        .unwrap();
    assert!(store.economic_view().await.is_err());
    assert!(store.economic_initialize([9; 32]).await.is_err());
}

#[tokio::test]
async fn compatibility_checkpoint_preserves_nonempty_pre_11_data_as_unknown() {
    let url = std::env::var("SPC_LADDER_TEST_DATABASE_URL").expect("owned PG required");
    assert!(url.contains("ladder_completion"));
    let admin = Store::connect(&url).await.unwrap();
    let name = format!("ladder_completion_seed_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(admin.pool())
        .await
        .unwrap();
    let store = Store::connect(&url.replace("/ladder_completion?", &format!("/{name}?")))
        .await
        .unwrap();
    let mut before = sqlx::migrate!("./migrations");
    before.migrations =
        std::borrow::Cow::Owned(before.iter().filter(|m| m.version < 11).cloned().collect());
    before.run(store.pool()).await.unwrap();
    store
        .upsert_identity(&SpIdentityRow {
            sp_id: [1; 32],
            b_scan: vec![2; 33],
            b_spend: vec![3; 33],
            network: 3,
        })
        .await
        .unwrap();
    seed(&store, 125).await; // Explicit pre-existing ledger fixture, never funded acceptance.
    store.migrate().await.unwrap();
    let checkpoint = store.economic_checkpoint_record().await.unwrap().unwrap();
    assert_eq!(checkpoint["classification"], "unverified_existing");
    assert_eq!(store.pending_for(&[1; 32]).await.unwrap(), 125);
    assert!(store.economic_view().await.is_err());
    assert!(store.economic_initialize([9; 32]).await.is_err());
}

#[tokio::test]
async fn confirmed_loss_held_recanonicalization_restores_full_ledger_once_with_parent_reservation()
{
    use bitcoin::secp256k1::{Keypair, Secp256k1, SecretKey};
    use spc_ext::authorization::{order_digest, AuthorizationRequest};
    use spc_store::aggregate_store::{AggregateMember, SentDeliveryProof};
    for delta in [-100_000, 100_000] {
        let store = database().await;
        seed(&store, 500_000).await;
        let (parent, _) = found(&store, 1, -100_000).await; // remains canonical/unbooked: R=100k
        let protected = [2; 32];
        store
            .upsert_identity(&SpIdentityRow {
                sp_id: protected,
                b_scan: vec![2; 33],
                b_spend: vec![3; 33],
                network: 3,
            })
            .await
            .unwrap();
        let view = store.economic_view().await.unwrap();
        let mut balances = view.balances().unwrap().into_values().collect::<Vec<_>>();
        balances.push(spc_accounting::economic::Balance {
            sp_id: protected,
            booked: 0,
            reserved: 0,
            available: 0,
        });
        let event = BlockEvent {
            block_hash: [2; 32],
            height: 2,
            front_id: 0,
            fingerprint: [8; 32],
            a_send_x: [4; 32],
            high_water: BTreeMap::new(),
            window_target: 9,
            economic: Some(FrozenJob {
                version: 1,
                revision: view.revision,
                parent_hash: parent.block_hash,
                job_id: 2,
                generation: 2,
                balances,
                slice_ids: vec![],
            }),
            entries: vec![
                Entry {
                    sp_id: [1; 32],
                    owed_sats: 500_000,
                    rail: Rail::Pending,
                    p_x: vec![],
                    dest_id: vec![],
                    sats: 500_000,
                },
                Entry {
                    sp_id: protected,
                    owed_sats: 5_000,
                    rail: Rail::Ark,
                    p_x: vec![],
                    dest_id: b"tark1store-fixture".to_vec(),
                    sats: 5_000,
                },
            ],
            work_unit_version: WorkUnitVersion::V2,
            work_unit_fingerprint: Some([5; 32]),
            window_rule: WindowRule::NetworkBlockWork,
        };
        let ledger = vec![
            LedgerRow {
                sp_id: [1; 32],
                rail: LedgerRail::Pending,
                paid_sats: 0,
                ark_due_sats: 0,
                forfeited_sats: 0,
                pending_delta: delta,
            },
            LedgerRow {
                sp_id: protected,
                rail: LedgerRail::Ark,
                paid_sats: 0,
                ark_due_sats: 5_000,
                forfeited_sats: 0,
                pending_delta: 0,
            },
        ];
        let fp = event.snapshot_id().unwrap();
        store
            .insert_snapshot(&SnapshotRow {
                fingerprint: fp,
                height: 2,
                a_send_x: event.a_send_x,
                distribution_id: None,
                entries: serde_json::json!(event.entries),
                high_water: serde_json::json!({}),
                window_target: Some("9".into()),
                work_unit_version: event.work_unit_version,
                work_unit_fingerprint: event.work_unit_fingerprint,
                window_rule: event.window_rule,
            })
            .await
            .unwrap();
        store.economic_found(&event, &ledger, false).await.unwrap();
        store
            .economic_advance(2, event.block_hash, parent.block_hash, true, [0; 32])
            .await
            .unwrap();
        store
            .settle(
                &Settlement {
                    block_hash: event.block_hash,
                    height: 2,
                    fingerprint: Some(fp),
                    status: BlockStatus::Confirmed,
                    ledger,
                },
                6,
            )
            .await
            .unwrap();
        let signer =
            Keypair::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&[7; 32]).unwrap());
        let order = store
            .freeze_aggregate(
                &[AggregateMember {
                    block_hash: event.block_hash,
                    sp_id: protected,
                    block_height: 2,
                    due_height: 8,
                    sats: 5_000,
                    destination: b"tark1store-fixture".to_vec(),
                    is_remainder: true,
                }],
                8,
                [9; 32],
                &signer,
            )
            .await
            .unwrap();
        store.publish_aggregate(&order.order_id).await.unwrap();
        let canonical = std::collections::BTreeSet::from([event.block_hash]);
        let request = AuthorizationRequest {
            order_id: order.order_id,
            order_digest: order_digest(&order).unwrap(),
            nonce: [3; 16],
        };
        let grant = store
            .authorize_aggregate(&request, 8, [9; 32], &canonical, 1000, &signer)
            .await
            .unwrap()
            .unwrap();
        store
            .resolve_aggregate_sent(
                &request,
                grant.revision,
                5000,
                SentDeliveryProof {
                    vtxo_id: &[9; 36],
                    expiry: 9000,
                    proof: b"validated internal-store fixture",
                },
                &canonical,
            )
            .await
            .unwrap();
        store.orphan(&event.block_hash).await.unwrap();
        store.economic_rewind(1, parent.block_hash).await.unwrap();
        assert_eq!(store.pending_for(&[1; 32]).await.unwrap(), 500_000);
        assert_eq!(
            store.aggregate_status(&protected).await.unwrap()["orders"][0]["pool_loss_sats"],
            5000
        );
        assert!(store
            .economic_restore_loss_held(&event, 6, 7)
            .await
            .is_err());
        assert_eq!(store.pending_for(&[1; 32]).await.unwrap(), 500_000);
        assert!(store
            .economic_restore_loss_held(&event, 6, 8)
            .await
            .unwrap());
        assert!(!store
            .economic_restore_loss_held(&event, 6, 8)
            .await
            .unwrap());
        store
            .economic_advance(2, event.block_hash, parent.block_hash, true, [0; 32])
            .await
            .unwrap();
        let balance = store.economic_view().await.unwrap().balances().unwrap()[&[1; 32]].clone();
        assert_eq!(balance.booked, (500_000i64 + delta) as u64);
        assert_eq!(balance.reserved, 100_000);
        assert_eq!(balance.available, (400_000i64 + delta) as u64);
        assert_eq!(
            store.aggregate_status(&protected).await.unwrap()["orders"][0]["pool_loss_sats"],
            0
        );
        assert!(
            store.scheduled_ark_payouts().await.unwrap().is_empty(),
            "retained Sent is not reissued"
        );
    }
}
