#![cfg(feature = "postgres-integration")]
//! Owned SQL fixtures; none of these rows is live mined/funded evidence.
use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
use serde_json::{json, Value};
use spc_accounting::{
    economic::{Balance, FrozenJob},
    WindowRule, WorkUnitVersion,
};
use spc_store::{
    models::*,
    observations,
    wire::{BlockEvent, Entry, Rail},
    Store,
};
use sqlx::Row;

async fn fixture() -> (Store, BlockEvent) {
    let url = std::env::var("SPC_LADDER_TEST_DATABASE_URL").expect("owned PG required; no skip");
    assert!(url.contains("ladder_completion"));
    let admin = Store::connect(&url).await.unwrap();
    let name = format!(
        "ladder_completion_observation_{}",
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
    store.economic_initialize([9; 32]).await.unwrap();
    let view = store.economic_view().await.unwrap();
    let event = BlockEvent {
        block_hash: [4; 32],
        height: 1,
        front_id: 0,
        fingerprint: [5; 32],
        a_send_x: [6; 32],
        high_water: Default::default(),
        window_target: 1,
        entries: [[1; 32], [2; 32]]
            .into_iter()
            .map(|sp_id| Entry {
                sp_id,
                owed_sats: 1,
                rail: Rail::Pending,
                p_x: vec![],
                dest_id: vec![],
                sats: 1,
            })
            .collect(),
        work_unit_version: WorkUnitVersion::V2,
        work_unit_fingerprint: Some([7; 32]),
        window_rule: WindowRule::NetworkBlockWork,
        economic: Some(FrozenJob {
            version: 1,
            revision: view.revision,
            parent_hash: [9; 32],
            job_id: 1,
            generation: 2,
            slice_ids: vec![],
            balances: vec![
                Balance {
                    sp_id: [1; 32],
                    booked: 0,
                    reserved: 0,
                    available: 0,
                },
                Balance {
                    sp_id: [2; 32],
                    booked: 0,
                    reserved: 0,
                    available: 0,
                },
            ],
        }),
    };
    (store, event)
}

#[tokio::test]
async fn delivery_read_preserves_census_rows_absence_commit_and_acceptance_order() {
    let (store, event) = fixture().await;
    let public = PublicKey::from_secret_key(
        &Secp256k1::new(),
        &SecretKey::from_slice(&[0x51; 32]).unwrap(),
    )
    .serialize()
    .to_vec();
    let registration = store
        .apply_registration(
            RegistrationEffect::Delivery {
                identity: SpIdentityRow {
                    sp_id: [1; 32],
                    b_scan: public.clone(),
                    b_spend: public.clone(),
                    network: 3,
                },
                delivery: ArkDeliveryRow {
                    registration_commit: None,
                    sp_id: [1; 32],
                    ark_id: [3; 4],
                    b_ark: public.clone(),
                    mailbox_mode: MailboxMode::Static,
                    mailbox_m: Some(public.clone()),
                    mailbox_d: Some(public),
                    epoch_fallback: EpochFallback::Hold,
                },
            },
            1,
            [8; 32],
        )
        .await
        .unwrap();
    let read = store
        .observe_settlement_delivery_read(&event, &[[1; 32], [2; 32]], &[1; 16])
        .await
        .unwrap();
    assert_eq!(read.delivered, std::collections::BTreeSet::from([[1; 32]]));
    let row = sqlx::query("SELECT * FROM source_decision_observation WHERE observation_id=$1")
        .bind(read.observation_id)
        .fetch_one(store.pool())
        .await
        .unwrap();
    let body: Value = row.get("body");
    assert_eq!(body["status"], "complete");
    assert_eq!(body["results"].as_array().unwrap().len(), 2);
    assert!(body["results"][1]["delivery"].is_null());
    assert!(body["results"][1]["registration_state"].is_null());
    assert_eq!(
        body["results"][0]["registration_state"]["registration_rev"],
        registration.registration_rev
    );
    assert_eq!(
        body["results"][0]["registration_request"]["registration_rev"],
        registration.registration_rev
    );
    assert_eq!(
        body["results"][0]["registration_request"]["signed_fields_digest"],
        format!("\\x{}", hex::encode([8; 32]))
    );
    assert!(
        !body.to_string().contains(&hex::encode([0x51; 32])),
        "private scalar was not observed"
    );
    assert!(!row.get::<String, _>("snapshot_boundary").is_empty());
    let history_xid:i64=sqlx::query_scalar("SELECT transaction_id FROM source_accounting_history WHERE table_name='source_decision_observation' AND (after_row->>'observation_id')::bigint=$1")
        .bind(read.observation_id).fetch_one(store.pool()).await.unwrap();
    assert_eq!(history_xid, row.get::<i64, _>("transaction_id"));
    let ledger: Vec<LedgerRow> = [[1; 32], [2; 32]]
        .into_iter()
        .map(|sp_id| LedgerRow {
            sp_id,
            rail: LedgerRail::Pending,
            paid_sats: 0,
            ark_due_sats: 0,
            forfeited_sats: 0,
            pending_delta: 1,
        })
        .collect();
    let before_revision = store.economic_view().await.unwrap().revision;
    let missing=store.record_observation("settlement_reconstruction",&event.block_hash,Some(&[9;16]),&json!({
        "schema":"spc.settlement-reconstruction/1","binding":observations::block_binding(&event).unwrap(),
        "status":"complete","ledger":ledger})).await.unwrap();
    assert!(store
        .economic_found_observed(&event, &ledger, false, Some(missing))
        .await
        .is_err());
    assert_eq!(
        store.economic_view().await.unwrap().revision,
        before_revision
    );
    assert!(store
        .economic_lifecycle(&event.block_hash)
        .await
        .unwrap()
        .is_none());
    let reconstruction=store.record_observation("settlement_reconstruction",&event.block_hash,Some(&[1;16]),&json!({
        "schema":"spc.settlement-reconstruction/1","binding":observations::block_binding(&event).unwrap(),"status":"complete",
        "delivery_read_observation":read.observation_id,"ledger":ledger})).await.unwrap();
    assert!(store
        .economic_found_observed(&event, &ledger, false, Some(reconstruction))
        .await
        .unwrap());
    assert_eq!(
        store
            .reconstruction_observation(&event.block_hash)
            .await
            .unwrap(),
        Some(reconstruction)
    );
    let accept_event:i64=sqlx::query_scalar("SELECT event_id FROM source_accounting_history WHERE table_name='economic_block' AND operation='INSERT'")
        .fetch_one(store.pool()).await.unwrap();
    let read_event:i64=sqlx::query_scalar("SELECT event_id FROM source_accounting_history WHERE table_name='source_decision_observation' AND (after_row->>'observation_id')::bigint=$1")
        .bind(read.observation_id).fetch_one(store.pool()).await.unwrap();
    assert!(read_event < accept_event);
    sqlx::query("DELETE FROM ark_delivery WHERE sp_id=$1")
        .bind(&[1u8; 32][..])
        .execute(store.pool())
        .await
        .unwrap();
    let later = store
        .observe_settlement_delivery_read(&event, &[[1; 32], [2; 32]], &[2; 16])
        .await
        .unwrap();
    assert!(later.delivered.is_empty());
    let retained: Value =
        sqlx::query_scalar("SELECT body FROM source_decision_observation WHERE observation_id=$1")
            .bind(read.observation_id)
            .fetch_one(store.pool())
            .await
            .unwrap();
    assert_eq!(retained, body);
    assert!(
        sqlx::query("UPDATE economic_block SET reconstruction_observation=NULL")
            .execute(store.pool())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn failed_read_is_unknown_and_append_replay_rollback_and_scrub_are_safe() {
    let (store, event) = fixture().await;
    sqlx::query("ALTER TABLE ark_delivery RENAME TO temporarily_unavailable_delivery")
        .execute(store.pool())
        .await
        .unwrap();
    assert!(store
        .observe_settlement_delivery_read(&event, &[[1; 32]], &[3; 16])
        .await
        .is_err());
    sqlx::query("ALTER TABLE temporarily_unavailable_delivery RENAME TO ark_delivery")
        .execute(store.pool())
        .await
        .unwrap();
    let failed: Value =
        sqlx::query_scalar("SELECT body FROM source_decision_observation WHERE attempt=$1")
            .bind(&[3u8; 16][..])
            .fetch_one(store.pool())
            .await
            .unwrap();
    assert_eq!(failed["status"], "unknown");
    assert!(failed["results"].is_null());
    let value = json!({"schema":"spc.front-derivation/1","status":"fixture"});
    let mut tx = store.pool().begin().await.unwrap();
    observations::append(
        &mut tx,
        "front_derivation",
        &[1; 16],
        Some(&[4; 16]),
        &value,
    )
    .await
    .unwrap();
    tx.rollback().await.unwrap();
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM source_decision_observation WHERE attempt=$1")
            .bind(&[4u8; 16][..])
            .fetch_one(store.pool())
            .await
            .unwrap();
    assert_eq!(count, 0);
    let count:i64=sqlx::query_scalar("SELECT count(*) FROM source_accounting_history WHERE table_name='source_decision_observation' AND after_row->>'attempt'=$1")
        .bind(format!("\\x{}",hex::encode([4;16]))).fetch_one(store.pool()).await.unwrap();
    assert_eq!(count, 0);
    let first = store
        .record_observation("front_derivation", &[1; 16], Some(&[4; 16]), &value)
        .await
        .unwrap();
    assert_eq!(
        first,
        store
            .record_observation("front_derivation", &[1; 16], Some(&[4; 16]), &value)
            .await
            .unwrap()
    );
    assert!(store
        .record_observation(
            "front_derivation",
            &[1; 16],
            Some(&[4; 16]),
            &json!({"changed":true})
        )
        .await
        .is_err());
    assert!(
        sqlx::query("DELETE FROM source_decision_observation WHERE observation_id=$1")
            .bind(first)
            .execute(store.pool())
            .await
            .is_err()
    );
    assert!(sqlx::query(
        "UPDATE source_decision_observation SET body='{}' WHERE observation_id=$1"
    )
    .bind(first)
    .execute(store.pool())
    .await
    .is_err());
}
