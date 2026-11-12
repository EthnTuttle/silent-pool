//! Postgres-authoritative registration journal integration tests.

use spc_store::models::*;
use spc_store::Store;
use sqlx::Row;
use uuid::Uuid;

async fn isolated_store() -> Option<(Store, sqlx::PgPool, String)> {
    let url = std::env::var("SPC_TEST_DATABASE_URL").ok()?;
    let base = url.split('?').next().unwrap_or(&url);
    let (head, _) = base.rsplit_once('/').expect("database path");
    let admin = sqlx::PgPool::connect(&format!("{head}/postgres"))
        .await
        .unwrap();
    let name = format!("ap2_{}", Uuid::new_v4().simple());
    if sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .is_err()
    {
        eprintln!("skip: no CREATE DATABASE privilege");
        return None;
    }
    let store = Store::connect(&format!("{head}/{name}"))
        .await
        .expect("connect");
    store.migrate().await.expect("migrate");
    Some((store, admin, name))
}

fn identity(seed: u8) -> SpIdentityRow {
    SpIdentityRow {
        sp_id: [seed; 32],
        b_scan: vec![2; 33],
        b_spend: vec![3; 33],
        network: 3,
    }
}

fn delivery(sp_id: [u8; 32], ark_id: u8) -> ArkDeliveryRow {
    ArkDeliveryRow {
        registration_commit: None,
        sp_id,
        ark_id: [ark_id; 4],
        b_ark: vec![4; 33],
        mailbox_mode: MailboxMode::EpochDerived,
        mailbox_m: Some(vec![5; 33]),
        mailbox_d: Some(vec![6; 33]),
        epoch_fallback: EpochFallback::Hold,
    }
}

fn effect(seed: u8, ark_id: u8) -> RegistrationEffect {
    let identity = identity(seed);
    let delivery = delivery(identity.sp_id, ark_id);
    RegistrationEffect::Delivery { identity, delivery }
}

#[tokio::test]
async fn journal_scopes_idempotency_by_identity_and_never_rolls_back() {
    let Some((store, _admin, _name)) = isolated_store().await else {
        return;
    };

    let alice = effect(0xa1, 1);
    let bob = effect(0xb2, 2);
    let digest_a1 = [0x11; 32];
    let digest_b1 = [0x22; 32];
    let first = store
        .apply_registration(alice.clone(), 1, digest_a1)
        .await
        .unwrap();
    let second = store
        .apply_registration(bob.clone(), 1, digest_b1)
        .await
        .unwrap();
    assert_eq!(first.outcome, RegistrationOutcome::Applied);
    assert_eq!(second.outcome, RegistrationOutcome::Applied);
    assert!(second.registration_rev > first.registration_rev);

    // Both users can safely choose request ID 1.
    assert_eq!(
        store
            .ark_delivery(&[0xa1; 32])
            .await
            .unwrap()
            .unwrap()
            .ark_id,
        [1; 4]
    );
    assert_eq!(
        store
            .ark_delivery(&[0xb2; 32])
            .await
            .unwrap()
            .unwrap()
            .ark_id,
        [2; 4]
    );

    let duplicate = store
        .apply_registration(alice.clone(), 1, digest_a1)
        .await
        .unwrap();
    assert_eq!(duplicate.outcome, RegistrationOutcome::AlreadyApplied);
    assert_eq!(duplicate.registration_rev, first.registration_rev);

    let mismatch = store
        .apply_registration(effect(0xa1, 3), 1, [0x33; 32])
        .await
        .unwrap();
    assert_eq!(mismatch.outcome, RegistrationOutcome::Mismatch);
    assert_eq!(mismatch.error_code(), Some("registration-mismatch"));
    assert_eq!(
        store
            .ark_delivery(&[0xa1; 32])
            .await
            .unwrap()
            .unwrap()
            .ark_id,
        [1; 4]
    );

    let newer = store
        .apply_registration(effect(0xa1, 4), 2, [0x44; 32])
        .await
        .unwrap();
    assert_eq!(newer.outcome, RegistrationOutcome::Applied);
    assert_eq!(
        store
            .ark_delivery(&[0xa1; 32])
            .await
            .unwrap()
            .unwrap()
            .ark_id,
        [4; 4]
    );

    let replay = store
        .apply_registration(alice.clone(), 1, digest_a1)
        .await
        .unwrap();
    assert_eq!(replay.outcome, RegistrationOutcome::Superseded);
    assert_eq!(replay.registration_rev, first.registration_rev);
    assert_eq!(
        store
            .ark_delivery(&[0xa1; 32])
            .await
            .unwrap()
            .unwrap()
            .ark_id,
        [4; 4]
    );

    let stale = store
        .apply_registration(effect(0xa1, 5), 0, [0x55; 32])
        .await
        .unwrap();
    assert_eq!(stale.outcome, RegistrationOutcome::RequestIdExhausted);
    assert_eq!(stale.error_code(), Some("request-id-exhausted"));

    let state: (i64, Vec<u8>, i64) = sqlx::query_as(
        "SELECT last_request_id, signed_fields_digest, registration_rev
         FROM ark_registration_state WHERE sp_id = $1::bytea",
    )
    .bind(&[0xa1u8; 32][..])
    .fetch_one(store.pool())
    .await
    .unwrap();
    assert_eq!(state.0, 2);
    assert_eq!(state.1, vec![0x44; 32]);
    assert_eq!(state.2 as u64, newer.registration_rev);
}

#[tokio::test]
async fn durable_resolution_survives_reconnect_and_cache_loss() {
    let Some((store, _admin, name)) = isolated_store().await else {
        return;
    };
    let original = store
        .apply_registration(effect(0xc1, 7), 1, [0x77; 32])
        .await
        .unwrap();
    drop(store);

    let url = std::env::var("SPC_TEST_DATABASE_URL").unwrap();
    let head = url.split('?').next().unwrap().rsplit_once('/').unwrap().0;
    let restarted = Store::connect(&format!("{head}/{name}")).await.unwrap();
    let replay = restarted
        .apply_registration(effect(0xc1, 7), 1, [0x77; 32])
        .await
        .unwrap();
    assert_eq!(replay.outcome, RegistrationOutcome::AlreadyApplied);
    assert_eq!(replay.registration_rev, original.registration_rev);

    let row = sqlx::query(
        "SELECT outcome, registration_rev FROM ark_registration_request
         WHERE sp_id = $1::bytea AND request_id = 1::bigint",
    )
    .bind(&[0xc1u8; 32][..])
    .fetch_one(restarted.pool())
    .await
    .unwrap();
    let outcome: String = row.try_get("outcome").unwrap();
    let rev: i64 = row.try_get("registration_rev").unwrap();
    assert_eq!(outcome, "applied");
    assert_eq!(rev as u64, original.registration_rev);
}
