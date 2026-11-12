//! JOINed producer+client aggregate revision proof (real PostgreSQL).
//!
//! The wallet-side envelope predicate (`verify_order_envelope`) requires a
//! signed grant whose revision strictly precedes the `sent` order's own
//! revision. That predicate alone is not producer semantics: this module
//! drives the actual `spc-store` transition chain (freeze -> publish ->
//! grant INSERT -> Sent UPDATE) against a real owned database and joins the
//! resulting store output with the client verifier, so the revision
//! relation is proved from the producer's own INSERT/UPDATE behavior.
//!
//! Gate: `cargo test -p spc-agent --features postgres-integration` with
//! `SPC_LADDER_TEST_DATABASE_URL` pointing at a dedicated `ladder_completion`
//! database (same never-skip contract as `spc-store`'s integration suite).
//! Normal `cargo test` runs do not compile this module.
use super::aggregate_receive::verify_order_envelope;
use super::aggregate_watch::AggregateStatus;
use bitcoin::hashes::{sha256, Hash};
use bitcoin::secp256k1::{Keypair, PublicKey, Secp256k1, SecretKey};
use spc_ext::authorization::{order_digest, AuthorizationRequest, ProvedReceipt};
use spc_store::{
    aggregate_store::{bounded_members, AggregateMember, DeliveryLimits, SentDeliveryProof},
    models::*,
    wire::{Entry, Rail},
    Store,
};
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
        "ladder_completion_agent_join_{}",
        rand::random::<u64>().to_string()
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
    Keypair::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&[9; 32]).unwrap())
}

fn fresh_id() -> [u8; 32] {
    sha256::Hash::hash(rand::random::<[u8; 32]>().as_slice()).to_byte_array()
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

/// The producer's actual grant INSERT -> Sent UPDATE chain must yield the
/// revision relation the client join assumes, and the client verifier must
/// accept the store's own grant plus the store's own sent status.
#[tokio::test]
async fn store_grant_insert_then_sent_transition_satisfies_client_envelope_join() {
    let store = store().await;
    let id = fresh_id();
    let a = member(&store, id, fresh_id(), 300, 4_000).await;
    let b = member(&store, id, fresh_id(), 301, 6_000).await;
    let limits = DeliveryLimits {
        min_sats: 1,
        max_sats: 5_000_000,
        max_members: 2048,
    };
    let members = bounded_members(vec![b.clone(), a.clone()], 307, limits, 10_000, 144).unwrap();
    let signer = signer();
    let order = store
        .freeze_aggregate(&members, 307, [9; 32], &signer)
        .await
        .unwrap();
    assert_eq!(order.sats, 10_000);
    store.publish_aggregate(&order.order_id).await.unwrap();
    let request = AuthorizationRequest {
        order_id: order.order_id,
        order_digest: order_digest(&order).unwrap(),
        nonce: [0xa7; 16],
    };
    let canonical = BTreeSet::from([a.block_hash, b.block_hash]);
    let grant = store
        .authorize_aggregate(&request, 307, [9; 32], &canonical, 1000, &signer)
        .await
        .unwrap()
        .expect("fresh published aggregate with canonical confirmed members grants once");
    // Producer fact 1: the durable grant is signed by the same authority as
    // the frozen order, and its revision is the aggregate's own revision
    // at authorization time (INSERT at revision = current + 1).
    assert!(grant.revision > 0);
    // Producer fact 2: the Sent resolution bumps the aggregate revision past
    // the grant's, so the stored relation grant.revision < sent revision is
    // an actual UPDATE consequence, not a client assumption.
    store
        .resolve_aggregate_sent(
            &request,
            grant.revision,
            10_000,
            SentDeliveryProof {
                vtxo_id: &[7; 36],
                expiry: 9000,
                proof: b"agent-join validated sender fixture",
            },
            &canonical,
        )
        .await
        .unwrap();
    let status_value = store.aggregate_status(&id).await.unwrap();
    assert_eq!(status_value["orders"][0]["state"], "sent");
    let sent_revision = status_value["orders"][0]["revision"].as_u64().unwrap();
    assert!(
        grant.revision < sent_revision,
        "store Sent transition must strictly increase revision past the grant: grant={} sent={}",
        grant.revision,
        sent_revision
    );
    // JOIN: feed the producer's own status document and the producer's own
    // signed grant into the client envelope verifier. The client parses the
    // same wire document the pool serves; the grant is the exact signed
    // AuthorizationGrant the store INSERTed.
    let status: AggregateStatus = serde_json::from_value(status_value).unwrap();
    assert_eq!(status.orders.len(), 1);
    let receipt = ProvedReceipt {
        grant: grant.clone(),
        sats: order.sats,
        vtxos: vec![b"agent-join vtxo proof fixture".to_vec()],
    };
    verify_order_envelope(
        &status.orders[0],
        &order,
        &receipt,
        &signer.x_only_public_key().0,
    )
    .expect(
        "actual store grant INSERT -> Sent transition output satisfies the client envelope join",
    );
    // A grant re-signed at the post-Sent revision must still refuse on the
    // client side, proving the join predicate is live against the producer's
    // actual revision, not just against hand-built fixtures.
    let mut stale = grant.clone();
    stale.revision = sent_revision;
    stale.sign(&signer);
    let replay = ProvedReceipt {
        grant: stale,
        sats: order.sats,
        vtxos: vec![b"agent-join vtxo proof fixture".to_vec()],
    };
    assert!(
        verify_order_envelope(
            &status.orders[0],
            &order,
            &replay,
            &signer.x_only_public_key().0
        )
        .is_err(),
        "a grant at or past the Sent revision must never join"
    );
}
