#![cfg(feature = "postgres-integration")]
//! Required owned-DB gate. Never environment-skip; fixtures are accounting
//! transactions, not claims of funded stock-Ark delivery.
use bitcoin::secp256k1::{Keypair, PublicKey, Secp256k1, SecretKey};
use spc_ext::authorization::{order_digest, AuthorizationRequest};
use spc_store::{
    aggregate_store::{bounded_members, AggregateMember, DeliveryLimits},
    models::*,
    wire::{Entry, Rail},
    Store,
};
use sqlx::Row;
use std::collections::BTreeSet;

async fn store() -> Store {
    let url = std::env::var("SPC_LADDER_TEST_DATABASE_URL")
        .expect("owned SPC_LADDER_TEST_DATABASE_URL is required; this test never skips");
    assert!(
        url.contains("ladder_completion"),
        "dedicated ladder DB required"
    );
    let admin = Store::connect(&url).await.unwrap();
    let name = format!(
        "ladder_completion_aggregate_{}",
        uuid::Uuid::new_v4().simple()
    );
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(admin.pool())
        .await
        .unwrap();
    let store = Store::connect(&url.replace("/ladder_completion?", &format!("/{name}?")))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    store
}
fn signer() -> Keypair {
    Keypair::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&[7; 32]).unwrap())
}
async fn member(
    store: &Store,
    id: [u8; 32],
    hash: [u8; 32],
    height: u32,
    sats: u64,
) -> AggregateMember {
    let secp = Secp256k1::new();
    let scan = PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&[1; 32]).unwrap());
    let spend = PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&[2; 32]).unwrap());
    store
        .upsert_identity(&SpIdentityRow {
            sp_id: id,
            b_scan: scan.serialize().to_vec(),
            b_spend: spend.serialize().to_vec(),
            network: 3,
        })
        .await
        .unwrap();
    let fingerprint = hash;
    let entry = Entry {
        sp_id: id,
        rail: Rail::Ark,
        p_x: vec![],
        dest_id: b"tark1unit-destination".to_vec(),
        sats,
        owed_sats: sats,
    };
    store
        .insert_snapshot(&SnapshotRow {
            fingerprint,
            height,
            a_send_x: [3; 32],
            distribution_id: None,
            entries: serde_json::json!([entry]),
            high_water: serde_json::json!({"0":"0"}),
            window_target: Some("1".into()),
            work_unit_version: spc_accounting::WorkUnitVersion::V1,
            work_unit_fingerprint: None,
            window_rule: spc_accounting::WindowRule::ShareWorkLegacy,
        })
        .await
        .unwrap();
    store
        .settle(
            &Settlement {
                block_hash: hash,
                height,
                fingerprint: Some(fingerprint),
                status: BlockStatus::Confirmed,
                ledger: vec![LedgerRow {
                    sp_id: id,
                    rail: LedgerRail::Ark,
                    paid_sats: 0,
                    ark_due_sats: sats,
                    forfeited_sats: 0,
                    pending_delta: 0,
                }],
            },
            6,
        )
        .await
        .unwrap();
    AggregateMember {
        sp_id: id,
        block_hash: hash,
        block_height: height,
        due_height: height + 6,
        sats,
        destination: b"tark1unit-destination".to_vec(),
        is_remainder: true,
    }
}
fn fresh_id() -> [u8; 32] {
    use bitcoin::hashes::{sha256, Hash};
    sha256::Hash::hash(uuid::Uuid::new_v4().as_bytes()).to_byte_array()
}

fn canonical_observation(
    member: &AggregateMember,
    tip: u32,
) -> spc_store::observations::CanonicalRead {
    spc_store::observations::CanonicalRead {
        schema: "spc.canonical-read/1".into(),
        source_session: [7; 16],
        started_unix_ms: Some(1),
        finished_unix_ms: Some(2),
        authoritative: true,
        tip: Some(tip),
        tip_hash: Some([9; 32]),
        tip_after: Some(tip),
        tip_hash_after: Some([9; 32]),
        outcome: "complete".into(),
        members: vec![spc_store::observations::CanonicalMemberRead {
            height: member.block_height,
            expected_hash: member.block_hash,
            observed_hash: Some(member.block_hash),
            read_status: "present".into(),
        }],
    }
}

#[tokio::test]
async fn observed_grant_consumption_and_working_row_deletion_preserve_exact_history() {
    use spc_ext::authorization::Consumption;
    let store = store().await;
    let member = member(&store, fresh_id(), fresh_id(), 100, 10_000).await;
    let order = store
        .freeze_aggregate(std::slice::from_ref(&member), 110, [9; 32], &signer())
        .await
        .unwrap();
    store.publish_aggregate(&order.order_id).await.unwrap();
    let request = AuthorizationRequest {
        order_id: order.order_id,
        order_digest: order_digest(&order).unwrap(),
        nonce: uuid::Uuid::new_v4().into_bytes(),
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let read = canonical_observation(&member, 110);
    let grant = store
        .authorize_aggregate_observed(&request, &read, now, &signer())
        .await
        .unwrap()
        .unwrap();
    let row=sqlx::query("SELECT transaction_id,body FROM source_decision_observation WHERE kind='authorization_decision' AND subject=$1 AND body->>'outcome'='issued'")
        .bind(&order.order_id[..]).fetch_one(store.pool()).await.unwrap();
    let body: serde_json::Value = row.get("body");
    assert_eq!(body["canonical_read"], serde_json::to_value(&read).unwrap());
    assert_eq!(body["grant_bytes"], hex::encode(grant.encode()));
    assert_eq!(body["queried_members"].as_array().unwrap().len(), 1);
    let issuance_xid:i64=sqlx::query_scalar("SELECT transaction_id FROM source_decision_observation WHERE kind='authorization_row' AND subject=$1 AND body->>'operation'='INSERT'")
        .bind(&order.order_id[..]).fetch_one(store.pool()).await.unwrap();
    assert_eq!(issuance_xid, row.get::<i64, _>("transaction_id"));
    assert!(store
        .authorize_aggregate_observed(&request, &read, now, &signer())
        .await
        .unwrap()
        .is_none());
    let consumption = Consumption {
        grant: grant.clone(),
        observed_unix_ms: now * 1000,
    };
    let ack = store
        .observe_authorization_consumption(&consumption)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ack.digest, consumption.digest());
    assert!(ack.recorded_unix_ms / 1000 < grant.expires_at);
    assert_eq!(
        store
            .observe_authorization_consumption(&consumption)
            .await
            .unwrap()
            .unwrap(),
        ack
    );
    let mut conflict = consumption.clone();
    conflict.observed_unix_ms += 1;
    assert!(store
        .observe_authorization_consumption(&conflict)
        .await
        .is_err());
    sqlx::query("UPDATE ark_send_authorization SET checked_tip=checked_tip WHERE order_id=$1")
        .bind(&order.order_id[..])
        .execute(store.pool())
        .await
        .unwrap();
    sqlx::query("DELETE FROM ark_send_authorization WHERE order_id=$1")
        .bind(&order.order_id[..])
        .execute(store.pool())
        .await
        .unwrap();
    let history:Vec<serde_json::Value>=sqlx::query_scalar("SELECT body FROM source_decision_observation WHERE kind='authorization_row' AND subject=$1 ORDER BY observation_id")
        .bind(&order.order_id[..]).fetch_all(store.pool()).await.unwrap();
    assert_eq!(history.len(), 3);
    assert_eq!(history[2]["operation"], "DELETE");
    assert_eq!(
        history[2]["before"]["grant_bytes"],
        format!("\\x{}", hex::encode(grant.encode()))
    );
    assert_eq!(
        store
            .observe_authorization_consumption(&consumption)
            .await
            .unwrap()
            .unwrap(),
        ack,
        "duplicate returns original evidence even after working-row scrub"
    );
    assert_eq!(
        store
            .aggregate(&order.order_id)
            .await
            .unwrap()
            .unwrap()
            .state,
        "authorized"
    );
    assert!(
        store
            .aggregate_candidates(&member.sp_id)
            .await
            .unwrap()
            .is_empty(),
        "observation never refunds/reissues membership"
    );
}

#[tokio::test]
async fn observation_failure_rolls_back_grant_and_unknown_canonical_read_never_issues() {
    let store = store().await;
    let member = member(&store, fresh_id(), fresh_id(), 100, 10_000).await;
    let order = store
        .freeze_aggregate(std::slice::from_ref(&member), 110, [9; 32], &signer())
        .await
        .unwrap();
    store.publish_aggregate(&order.order_id).await.unwrap();
    let request = AuthorizationRequest {
        order_id: order.order_id,
        order_digest: order_digest(&order).unwrap(),
        nonce: uuid::Uuid::new_v4().into_bytes(),
    };
    let mut read = canonical_observation(&member, 110);
    read.tip_after = Some(111);
    assert!(store
        .authorize_aggregate_observed(&request, &read, 100, &signer())
        .await
        .unwrap()
        .is_none());
    read.tip_after = Some(110);
    sqlx::query("CREATE FUNCTION reject_test_observation() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.kind='authorization_decision' AND NEW.body->>'outcome'='issued' THEN RAISE EXCEPTION 'fixture observation persistence failure'; END IF; RETURN NEW; END $$")
        .execute(store.pool()).await.unwrap();
    sqlx::query("CREATE TRIGGER reject_test_observation BEFORE INSERT ON source_decision_observation FOR EACH ROW EXECUTE FUNCTION reject_test_observation()")
        .execute(store.pool()).await.unwrap();
    assert!(store
        .authorize_aggregate_observed(&request, &read, 100, &signer())
        .await
        .is_err());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM ark_send_authorization")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(count, 0);
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM source_decision_observation WHERE kind='authorization_row'",
    )
    .fetch_one(store.pool())
    .await
    .unwrap();
    assert_eq!(count, 0);
    assert_eq!(
        store
            .aggregate(&order.order_id)
            .await
            .unwrap()
            .unwrap()
            .state,
        "published"
    );
}

#[tokio::test]
async fn expired_consumption_is_unknown_and_does_not_release_the_order() {
    let store = store().await;
    let member = member(&store, fresh_id(), fresh_id(), 100, 10_000).await;
    let order = store
        .freeze_aggregate(std::slice::from_ref(&member), 110, [9; 32], &signer())
        .await
        .unwrap();
    store.publish_aggregate(&order.order_id).await.unwrap();
    let request = AuthorizationRequest {
        order_id: order.order_id,
        order_digest: order_digest(&order).unwrap(),
        nonce: uuid::Uuid::new_v4().into_bytes(),
    };
    let grant = store
        .authorize_aggregate_observed(
            &request,
            &canonical_observation(&member, 110),
            100,
            &signer(),
        )
        .await
        .unwrap()
        .unwrap();
    let consumption = spc_ext::authorization::Consumption {
        grant,
        observed_unix_ms: 101_000,
    };
    assert!(store
        .observe_authorization_consumption(&consumption)
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        store
            .aggregate(&order.order_id)
            .await
            .unwrap()
            .unwrap()
            .state,
        "authorized"
    );
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM source_decision_observation WHERE kind='authorization_consumption' AND body->>'status'='recorded'",
    )
    .fetch_one(store.pool())
    .await
    .unwrap();
    assert_eq!(count, 0);
    let body:serde_json::Value=sqlx::query_scalar("SELECT body FROM source_decision_observation WHERE kind='authorization_consumption' AND body->>'status'='unknown'")
        .fetch_one(store.pool()).await.unwrap();
    assert_eq!(body["reason"], "grant_mismatch_or_expired");
}

#[tokio::test]
async fn threshold_timeout_freeze_grant_and_all_covered_atomic_receipt() {
    let store = store().await;
    let id = fresh_id();
    let a = member(&store, id, fresh_id(), 100, 4_532).await;
    let b = member(&store, id, fresh_id(), 101, 6_000).await;
    let limits = DeliveryLimits {
        min_sats: 1,
        max_sats: 5_000_000,
        max_members: 2048,
    };
    assert!(bounded_members(vec![a.clone()], 106, limits, 10_000, 144)
        .unwrap()
        .is_empty());
    assert_eq!(
        bounded_members(vec![a.clone()], 250, limits, 10_000, 144)
            .unwrap()
            .len(),
        1
    );
    let members = bounded_members(vec![b.clone(), a.clone()], 107, limits, 10_000, 144).unwrap();
    let order = store
        .freeze_aggregate(&members, 107, [9; 32], &signer())
        .await
        .unwrap();
    assert_eq!(order.sats, 10_532);
    assert_eq!(order.ark_address, "tark1unit-destination");
    assert!(store.aggregate_candidates(&id).await.unwrap().is_empty());
    store.publish_aggregate(&order.order_id).await.unwrap();
    let request = AuthorizationRequest {
        order_id: order.order_id,
        order_digest: order_digest(&order).unwrap(),
        nonce: uuid::Uuid::new_v4().into_bytes(),
    };
    let canonical = BTreeSet::from([a.block_hash, b.block_hash]);
    let grant = store
        .authorize_aggregate(&request, 107, [9; 32], &canonical, 1000, &signer())
        .await
        .unwrap()
        .unwrap();
    let other = AuthorizationRequest {
        nonce: uuid::Uuid::new_v4().into_bytes(),
        ..request.clone()
    };
    assert!(store
        .authorize_aggregate(&other, 108, [9; 32], &canonical, 2000, &signer())
        .await
        .unwrap()
        .is_none());
    assert!(!store
        .cancel_aggregate_without_authorization(&order.order_id, &canonical)
        .await
        .unwrap());
    assert!(store
        .resolve_aggregate_sent(
            &request,
            grant.revision,
            4532,
            spc_store::aggregate_store::SentDeliveryProof {
                vtxo_id: &[8; 36],
                expiry: 9000,
                proof: b"validated sender fixture"
            },
            &canonical
        )
        .await
        .is_err());
    let before = store.scheduled_ark_payouts().await.unwrap();
    assert_eq!(
        before.iter().filter(|r| r.sp_id == id).count(),
        2,
        "partial receipt freed no member"
    );
    let one = store.clone();
    let two = store.clone();
    let (a_result, b_result) = tokio::join!(
        one.resolve_aggregate_sent(
            &request,
            grant.revision,
            10_532,
            spc_store::aggregate_store::SentDeliveryProof {
                vtxo_id: &[8; 36],
                expiry: 9000,
                proof: b"validated sender fixture"
            },
            &canonical
        ),
        two.resolve_aggregate_sent(
            &request,
            grant.revision,
            10_532,
            spc_store::aggregate_store::SentDeliveryProof {
                vtxo_id: &[8; 36],
                expiry: 9000,
                proof: b"validated sender fixture"
            },
            &canonical
        )
    );
    a_result.unwrap();
    b_result.unwrap();
    assert_eq!(
        store
            .scheduled_ark_payouts()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.sp_id == id)
            .count(),
        0
    );
    assert_eq!(store.pending_for(&id).await.unwrap(), 0);
    let status = store.aggregate_status(&id).await.unwrap();
    assert_eq!(status["orders"][0]["members"].as_array().unwrap().len(), 2);
    assert_eq!(status["orders"][0]["state"], "sent");
    assert!(status["wallet_received"].is_null());
    assert!(store.aggregate_status(&fresh_id()).await.unwrap()["orders"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn authorized_orphan_races_receipt_without_partial_reversal_or_reissue() {
    let store = store().await;
    let id = fresh_id();
    let a = member(&store, id, fresh_id(), 200, 5_000).await;
    let b = member(&store, id, fresh_id(), 201, 5_000).await;
    let order = store
        .freeze_aggregate(&[a.clone(), b.clone()], 207, [9; 32], &signer())
        .await
        .unwrap();
    store.publish_aggregate(&order.order_id).await.unwrap();
    let request = AuthorizationRequest {
        order_id: order.order_id,
        order_digest: order_digest(&order).unwrap(),
        nonce: uuid::Uuid::new_v4().into_bytes(),
    };
    let grant = store
        .authorize_aggregate(
            &request,
            207,
            [9; 32],
            &BTreeSet::from([a.block_hash, b.block_hash]),
            1000,
            &signer(),
        )
        .await
        .unwrap()
        .unwrap();
    let canonical = BTreeSet::from([b.block_hash]);
    let one = store.clone();
    let two = store.clone();
    let (orphan, sent) = tokio::join!(
        one.orphan(&a.block_hash),
        two.resolve_aggregate_sent(
            &request,
            grant.revision,
            10_000,
            spc_store::aggregate_store::SentDeliveryProof {
                vtxo_id: &[9; 36],
                expiry: 9000,
                proof: b"validated sender race fixture"
            },
            &canonical
        )
    );
    orphan.unwrap();
    sent.unwrap();
    let status = store.aggregate_status(&id).await.unwrap();
    assert_eq!(status["orders"][0]["state"], "sent");
    assert_eq!(status["orders"][0]["pool_loss_sats"], 5_000);
    assert!(store.aggregate_candidates(&id).await.unwrap().is_empty());
    assert_eq!(store.pending_for(&id).await.unwrap(), 0);
    assert!(store
        .mark_ark_failed_to_pending(&a.block_hash, &id)
        .await
        .is_err());
}

#[tokio::test]
async fn lost_grant_stays_pinned_and_never_authorized_reorg_requeues_only_canonical_members() {
    let store = store().await;
    let id = fresh_id();
    let a = member(&store, id, fresh_id(), 300, 5_000).await;
    let b = member(&store, id, fresh_id(), 301, 5_000).await;
    let order = store
        .freeze_aggregate(&[a.clone(), b.clone()], 307, [9; 32], &signer())
        .await
        .unwrap();
    store.publish_aggregate(&order.order_id).await.unwrap();
    // Before any authorization, cancel under durable row-lock proof. The old
    // order tombstone survives while only the canonical member re-enters queue.
    store.orphan(&a.block_hash).await.unwrap();
    assert!(store
        .cancel_aggregate_without_authorization(&order.order_id, &BTreeSet::from([b.block_hash]))
        .await
        .unwrap());
    assert_eq!(
        store
            .aggregate(&order.order_id)
            .await
            .unwrap()
            .unwrap()
            .state,
        "cancelled"
    );
    let queued = store.aggregate_candidates(&id).await.unwrap();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].block_hash, b.block_hash);
    let replacement = store
        .freeze_aggregate(&queued, 307, [9; 32], &signer())
        .await
        .unwrap();
    assert_ne!(replacement.order_id, order.order_id);
    store
        .publish_aggregate(&replacement.order_id)
        .await
        .unwrap();
    let request = AuthorizationRequest {
        order_id: replacement.order_id,
        order_digest: order_digest(&replacement).unwrap(),
        nonce: uuid::Uuid::new_v4().into_bytes(),
    };
    store
        .authorize_aggregate(
            &request,
            307,
            [9; 32],
            &BTreeSet::from([b.block_hash]),
            1000,
            &signer(),
        )
        .await
        .unwrap()
        .unwrap();
    // Simulated lost grant/transport response: no receipt is available.
    store
        .block_aggregate_unknown(&replacement.order_id)
        .await
        .unwrap();
    assert!(!store
        .cancel_aggregate_without_authorization(&replacement.order_id, &BTreeSet::new())
        .await
        .unwrap());
    assert!(store.aggregate_candidates(&id).await.unwrap().is_empty());
    let rows = sqlx::query("SELECT reserved FROM ark_aggregate_member WHERE order_id=$1")
        .bind(&replacement.order_id[..])
        .fetch_all(store.pool())
        .await
        .unwrap();
    assert!(rows.iter().all(|r| r.get::<bool, _>("reserved")));
}

#[tokio::test]
async fn mixed_identity_orphan_reverses_unreserved_rows_once_and_legacy_sent_cannot_split_the_set()
{
    for authorized in [false, true] {
        for (delta, unreserved_ark) in [
            (100i64, None),
            (-100, None),
            (100, Some("scheduled")),
            (-100, Some("failed_to_pending")),
        ] {
            let store = store().await;
            let id = fresh_id();
            let other = fresh_id();
            let a = member(&store, id, fresh_id(), 400, 5_000).await;
            let b = member(&store, id, fresh_id(), 401, 5_000).await;
            store
                .upsert_identity(&SpIdentityRow {
                    sp_id: other,
                    b_scan: vec![2; 33],
                    b_spend: vec![3; 33],
                    network: 3,
                })
                .await
                .unwrap();
            // Explicit booked-state fixture: other starts at 200. Exercise both
            // delta signs and unreserved Ark rows, including the failure credit.
            let failure_credit = if unreserved_ark == Some("failed_to_pending") {
                25
            } else {
                0
            };
            sqlx::query("INSERT INTO pending_balance(sp_id,sats) VALUES($1,$2)")
                .bind(&other[..])
                .bind(200 + delta + failure_credit)
                .execute(store.pool())
                .await
                .unwrap();
            sqlx::query("INSERT INTO ledger_history(block_hash,sp_id,rail,paid_sats,ark_due_sats,forfeited_sats,pending_delta) VALUES($1,$2,$3,0,$4,0,$5)")
            .bind(&a.block_hash[..]).bind(&other[..]).bind(if unreserved_ark.is_some(){"ark"}else{"pending"})
            .bind(if unreserved_ark.is_some(){25i64}else{0}).bind(delta).execute(store.pool()).await.unwrap();
            if let Some(status) = unreserved_ark {
                sqlx::query("INSERT INTO ark_payout(block_hash,sp_id,sats,due_height,status) VALUES($1,$2,25,406,$3)")
                .bind(&a.block_hash[..]).bind(&other[..]).bind(status).execute(store.pool()).await.unwrap();
            }
            let order = store
                .freeze_aggregate(&[a.clone(), b.clone()], 407, [9; 32], &signer())
                .await
                .unwrap();
            store.publish_aggregate(&order.order_id).await.unwrap();
            assert!(store
                .mark_ark_sent(&a.block_hash, &id, &[8; 36], 9000)
                .await
                .is_err());
            let request = AuthorizationRequest {
                order_id: order.order_id,
                order_digest: order_digest(&order).unwrap(),
                nonce: uuid::Uuid::new_v4().into_bytes(),
            };
            let grant = if authorized {
                store
                    .authorize_aggregate(
                        &request,
                        407,
                        [9; 32],
                        &BTreeSet::from([a.block_hash, b.block_hash]),
                        1000,
                        &signer(),
                    )
                    .await
                    .unwrap()
            } else {
                None
            };
            let canonical = BTreeSet::from([b.block_hash]);
            if let Some(grant) = &grant {
                let (orphan, sent) = tokio::join!(
                    store.orphan(&a.block_hash),
                    store.resolve_aggregate_sent(
                        &request,
                        grant.revision,
                        10_000,
                        spc_store::aggregate_store::SentDeliveryProof {
                            vtxo_id: &[9; 36],
                            expiry: 9000,
                            proof: b"validated fixture",
                        },
                        &canonical
                    )
                );
                orphan.unwrap();
                sent.unwrap();
            } else {
                let (orphan, cancel) = tokio::join!(
                    store.orphan(&a.block_hash),
                    store.cancel_aggregate_without_authorization(&order.order_id, &canonical)
                );
                orphan.unwrap();
                assert!(cancel.unwrap());
            }
            store.orphan(&a.block_hash).await.unwrap();
            assert_eq!(store.pending_for(&other).await.unwrap(), 200);
            let remaining: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM ledger_history WHERE block_hash=$1 AND sp_id=$2",
            )
            .bind(&a.block_hash[..])
            .bind(&other[..])
            .fetch_one(store.pool())
            .await
            .unwrap();
            assert_eq!(remaining, 0);
            if let Some(grant) = grant {
                assert!(!store
                    .cancel_aggregate_without_authorization(
                        &order.order_id,
                        &BTreeSet::from([b.block_hash])
                    )
                    .await
                    .unwrap());
                store
                    .resolve_aggregate_sent(
                        &request,
                        grant.revision,
                        10_000,
                        spc_store::aggregate_store::SentDeliveryProof {
                            vtxo_id: &[9; 36],
                            expiry: 9000,
                            proof: b"validated fixture",
                        },
                        &BTreeSet::from([b.block_hash]),
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    store.aggregate_status(&id).await.unwrap()["orders"][0]["pool_loss_sats"],
                    5_000
                );
            } else {
                assert!(store
                    .cancel_aggregate_without_authorization(
                        &order.order_id,
                        &BTreeSet::from([b.block_hash])
                    )
                    .await
                    .unwrap());
                assert_eq!(store.aggregate_candidates(&id).await.unwrap().len(), 1);
            }
            assert_eq!(
                store.pending_for(&other).await.unwrap(),
                200,
                "later aggregate resolution never reverses the unrelated row twice"
            );
        }
    }
}

#[tokio::test]
async fn concurrent_freeze_and_legacy_sent_have_only_one_winner() {
    let store = store().await;
    let id = fresh_id();
    let a = member(&store, id, fresh_id(), 500, 5_000).await;
    let freeze = store.clone();
    let sent = store.clone();
    let key = signer();
    let members = [a.clone()];
    let (f, s) = tokio::join!(
        freeze.freeze_aggregate(&members, 506, [9; 32], &key),
        sent.mark_ark_sent(&a.block_hash, &id, &[9; 36], 9000)
    );
    assert_ne!(
        f.is_ok(),
        s.is_ok(),
        "serialization forbids a reserved member already marked sent via legacy path"
    );
}
