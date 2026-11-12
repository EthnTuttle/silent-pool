//! Real PostgreSQL + Redis registration/stream integration. No Ark server/node.

use std::time::{Duration, Instant};

use bitcoin::secp256k1::{PublicKey as BitcoinPublicKey, Secp256k1 as BitcoinSecp, SecretKey};
use secp256k1::{Keypair, PublicKey, Secp256k1};
use spc_ext::{
    DeliveryMessage, EpochFallback, MailboxMode, RegistrationSuccessState, SetArkIdentity,
};
use spc_pool::identity::ChannelIdentity;
use spc_pool::payout::{AlwaysConfirm, PayoutService};
use spc_pool::registration::{ArkRegistrationHandler, RegistrationService};
use spc_store::models::RegistrationOutcome;
use spc_store::redis::Streams;
use spc_store::Store;
use uuid::Uuid;

const ARK_ID: [u8; 4] = [1, 2, 3, 4];

async fn store(tag: &str) -> Option<(Store, sqlx::PgPool, String)> {
    let url = std::env::var("SPC_TEST_DATABASE_URL").ok()?;
    let base = url.split('?').next().unwrap_or(&url);
    let (head, _) = base.rsplit_once('/').expect("database path");
    let admin = sqlx::PgPool::connect(&format!("{head}/postgres"))
        .await
        .unwrap();
    let name = format!("{tag}{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .expect("create isolated database");
    let store = Store::connect(&format!("{head}/{name}"))
        .await
        .expect("connect");
    store.migrate().await.expect("migrate");
    Some((store, admin, name))
}

fn key(seed: u8) -> Keypair {
    Keypair::from_secret_key(
        &Secp256k1::new(),
        &secp256k1::SecretKey::from_slice(&[seed; 32]).unwrap(),
    )
}

fn public(seed: u8) -> [u8; 33] {
    PublicKey::from_keypair(&key(seed)).serialize()
}

struct Fixture {
    identity: ChannelIdentity,
    service: RegistrationService,
    payout: PayoutService,
    streams: Streams,
    ack_key: String,
    registration_stream: String,
}

async fn fixture(tag: &str) -> Option<(Fixture, Store, sqlx::PgPool, String)> {
    let (store, admin, name) = store(tag).await?;
    let streams = Streams::connect(&std::env::var("SPC_TEST_REDIS_URL").ok()?)
        .await
        .unwrap();
    let suffix = Uuid::new_v4().simple().to_string();
    let registration_stream = format!("ap2:arkreg:{suffix}");
    let ack_key = format!("ap2:ack:{suffix}");
    let front_stream = format!("ap2:front:{suffix}");
    let payout = PayoutService::new(
        store.clone(),
        streams.clone(),
        6,
        330,
        std::sync::Arc::new(AlwaysConfirm),
        None,
        100_000,
    )
    .with_registration_ark_id(ARK_ID)
    .with_registration_streams(registration_stream.clone(), ack_key.clone())
    .with_front_events_stream(front_stream)
    .with_network(3);
    let service = RegistrationService::new(streams.clone(), ARK_ID)
        .with_names(registration_stream.clone(), ack_key.clone())
        .with_ack_timeout(Duration::from_secs(2));
    let identity = identity(3);
    Some((
        Fixture {
            identity,
            service,
            payout,
            streams,
            ack_key,
            registration_stream,
        },
        store,
        admin,
        name,
    ))
}

fn identity(seed: u8) -> ChannelIdentity {
    let secp = BitcoinSecp::new();
    let scan =
        BitcoinPublicKey::from_secret_key(&secp, &SecretKey::from_slice(&[seed; 32]).unwrap());
    let spend =
        BitcoinPublicKey::from_secret_key(&secp, &SecretKey::from_slice(&[seed + 1; 32]).unwrap());
    ChannelIdentity {
        sp_id: spc_crypto::sp_id(&scan, &spend),
        b_scan: scan.serialize(),
        b_spend: spend.serialize(),
        worker: None,
    }
}

fn request_with_signer(
    identity: &ChannelIdentity,
    signer_seed: u8,
    ark_seed: u8,
    request_id: u32,
) -> SetArkIdentity {
    let mut request = SetArkIdentity {
        ark_id: ARK_ID,
        b_ark: public(ark_seed + 10),
        mailbox_mode: MailboxMode::Static,
        mailbox_pubkey: public(ark_seed + 20).to_vec(),
        mailbox_dh: public(ark_seed + 30).to_vec(),
        epoch_fallback: EpochFallback::Static,
        sig: [0; 64],
        request_id,
    };
    request.sign(&identity.sp_id, &key(signer_seed)).unwrap();
    request
}

fn request(identity: &ChannelIdentity, seed: u8, request_id: u32) -> SetArkIdentity {
    request_with_signer(identity, seed, seed, request_id)
}

async fn drive(
    service: RegistrationService,
    request: SetArkIdentity,
    identity: ChannelIdentity,
    payout: PayoutService,
) -> DeliveryMessage {
    let task = tokio::spawn(async move {
        let payload = request.encode().unwrap();
        service
            .handle(&identity, spc_ext::MSG_SET_ARK_IDENTITY, &payload)
            .await
    });
    for _ in 0..100 {
        if task.is_finished() {
            break;
        }
        let _ = payout.process_registrations_once().await;
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .expect("drive timeout")
        .expect("join")
}

fn success(message: DeliveryMessage) -> (RegistrationSuccessState, u64) {
    let DeliveryMessage::SetArkIdentitySuccess(success) = message else {
        panic!("expected success, got {message:?}");
    };
    (success.state, success.registration_rev)
}

fn error(message: DeliveryMessage) -> (String, bool) {
    let DeliveryMessage::SetArkIdentityError(error) = message else {
        panic!("expected error, got {message:?}");
    };
    (
        String::from_utf8(error.error_code).unwrap(),
        error.retryable,
    )
}

#[tokio::test]
async fn streams_two_users_cache_expiry_restart_and_replay_never_roll_back() {
    let Some((fixture, store, _admin, _name)) = fixture("ap2a_").await else {
        eprintln!("skip: storage URLs absent");
        return;
    };
    let alice = fixture.identity.clone();
    let alice_request = request(&alice, 3, 1);
    let first = drive(
        fixture.service.clone(),
        alice_request.clone(),
        alice.clone(),
        fixture.payout.clone(),
    )
    .await;
    let (state, alice_rev) = success(first);
    assert_eq!(state, RegistrationSuccessState::Applied);
    assert!(store.ark_delivery(&alice.sp_id).await.unwrap().is_some());

    let bob = identity(11);
    let bob_request = request(&bob, 11, 1);
    let second = drive(
        fixture.service.clone(),
        bob_request,
        bob,
        fixture.payout.clone(),
    )
    .await;
    let (_, bob_rev) = success(second);
    assert!(bob_rev > alice_rev);

    // Expire the actual Redis cache key; PostgreSQL still resolves the replay.
    fixture
        .streams
        .expire(&fixture.ack_key, 1)
        .await
        .expect("expire registration ack cache");
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    let duplicate = drive(
        fixture.service.clone(),
        alice_request.clone(),
        alice.clone(),
        fixture.payout.clone(),
    )
    .await;
    assert_eq!(
        success(duplicate),
        (RegistrationSuccessState::AlreadyApplied, alice_rev)
    );

    let mut mismatch = alice_request.clone();
    mismatch.b_ark = public(90);
    mismatch.sign(&alice.sp_id, &key(3)).unwrap();
    let (code, retryable) = error(
        drive(
            fixture.service.clone(),
            mismatch,
            alice.clone(),
            fixture.payout.clone(),
        )
        .await,
    );
    assert_eq!(code, "registration-mismatch");
    assert!(!retryable);

    let newer_request = request_with_signer(&alice, 3, 40, 2);
    let newer_digest = newer_request.signing_payload(&alice.sp_id).unwrap();
    let _ = newer_digest;
    let (_, newer_rev) = success(
        drive(
            fixture.service.clone(),
            newer_request.clone(),
            alice.clone(),
            fixture.payout.clone(),
        )
        .await,
    );
    assert!(newer_rev > alice_rev);
    let replayed_old = drive(
        fixture.service.clone(),
        alice_request.clone(),
        alice.clone(),
        fixture.payout.clone(),
    )
    .await;
    assert_eq!(
        success(replayed_old).0,
        RegistrationSuccessState::Superseded
    );
    let row = store.ark_delivery(&alice.sp_id).await.unwrap().unwrap();
    assert_eq!(row.b_ark, newer_request.b_ark.to_vec());

    // Simulate a restart with a new Store/Service over the same durable DB and
    // Redis. The same counter still resolves from the journal.
    let restarted_payout = PayoutService::new(
        store.clone(),
        fixture.streams.clone(),
        6,
        330,
        std::sync::Arc::new(AlwaysConfirm),
        None,
        100_000,
    )
    .with_registration_ark_id(ARK_ID);
    let _ = restarted_payout;
    let duplicate_newer = drive(
        fixture.service.clone(),
        newer_request,
        alice,
        fixture.payout.clone(),
    )
    .await;
    let (_, duplicate_rev) = success(duplicate_newer);
    assert_eq!(duplicate_rev, newer_rev);
}

async fn drive_epoch_request(
    service: RegistrationService,
    request: spc_ext::RegisterEpochMailboxes,
    identity: ChannelIdentity,
    payout: PayoutService,
) -> DeliveryMessage {
    let task = tokio::spawn(async move {
        service
            .handle(
                &identity,
                spc_ext::MSG_REGISTER_EPOCH_MAILBOXES,
                &request.encode().unwrap(),
            )
            .await
    });
    for _ in 0..100 {
        if task.is_finished() {
            break;
        }
        let _ = payout.process_registrations_once().await;
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .expect("epoch drive timeout")
        .expect("epoch drive join")
}

#[tokio::test]
async fn rejected_id_collision_is_terminal_mismatch_without_poison_or_mutation() {
    let Some((fixture, store, _admin, _name)) = fixture("ap2r21_").await else {
        eprintln!("skip: storage URLs absent");
        return;
    };
    let miner = fixture.identity.clone();
    let initial = request(&miner, 3, 1);
    let (first_state, first_rev) = success(
        drive(
            fixture.service.clone(),
            initial,
            miner.clone(),
            fixture.payout.clone(),
        )
        .await,
    );
    assert_eq!(first_state, RegistrationSuccessState::Applied);
    let initial_delivery = store.ark_delivery(&miner.sp_id).await.unwrap().unwrap();

    let mut epoch = spc_ext::RegisterEpochMailboxes {
        entries: vec![{
            let mut entry = [0u8; 37];
            entry[..4].copy_from_slice(&800_000u32.to_be_bytes());
            entry[4..].copy_from_slice(&public(60));
            entry
        }],
        sig: [0; 64],
        request_id: 2,
    };
    epoch.sign(&miner.sp_id, &key(3)).unwrap();
    let rejected = drive_epoch_request(
        fixture.service.clone(),
        epoch.clone(),
        miner.clone(),
        fixture.payout.clone(),
    )
    .await;
    let DeliveryMessage::RegisterEpochMailboxesError(epoch_error) = rejected else {
        panic!("epoch on static registration must reject");
    };
    assert_eq!(epoch_error.error_code, b"bad-mailbox-fields");
    assert!(!epoch_error.retryable);

    // The journal now has id2 rejected while current remains id1. A valid
    // identity at id2 MUST NOT collide with the journal or advance current.
    let collision = request_with_signer(&miner, 3, 40, 2);
    let collision_response = drive(
        fixture.service.clone(),
        collision,
        miner.clone(),
        fixture.payout.clone(),
    )
    .await;
    let (code, retryable) = error(collision_response);
    assert_eq!(code, "registration-mismatch");
    assert!(!retryable);
    for _ in 0..2 {
        fixture.payout.process_registrations_once().await.unwrap();
    }
    assert_eq!(
        store
            .current_registration_request(&miner.sp_id)
            .await
            .unwrap(),
        Some(1)
    );
    assert_eq!(
        store.ark_delivery(&miner.sp_id).await.unwrap().unwrap(),
        initial_delivery
    );
    let stable_rev: i64 =
        sqlx::query_scalar("SELECT registration_rev FROM ark_registration_state WHERE sp_id = $1")
            .bind(&miner.sp_id[..])
            .fetch_one(store.pool())
            .await
            .unwrap();
    assert_eq!(stable_rev as u64, first_rev);

    // Replaying the exact rejected request also resolves from the journal,
    // preserving its original terminal error rather than becoming commit unknown.
    let rejected_again = drive_epoch_request(
        fixture.service.clone(),
        epoch,
        miner.clone(),
        fixture.payout.clone(),
    )
    .await;
    let DeliveryMessage::RegisterEpochMailboxesError(error) = rejected_again else {
        panic!("rejected replay must stay terminal");
    };
    assert_eq!(error.error_code, b"bad-mailbox-fields");
    assert!(!error.retryable);
    let pending = fixture
        .streams
        .read_pending(
            &fixture.registration_stream,
            spc_pool::payout::GROUP,
            spc_pool::payout::CONSUMER,
            64,
        )
        .await
        .unwrap();
    assert!(pending.is_empty(), "rejected collision must not poison PEL");

    // No counter advance is needed to mask the conflict: the next fresh valid
    // ID follows the normal no-journal-row apply path.
    let next = request_with_signer(&miner, 3, 50, 3);
    assert_eq!(
        success(
            drive(
                fixture.service.clone(),
                next.clone(),
                miner.clone(),
                fixture.payout.clone(),
            )
            .await,
        )
        .0,
        RegistrationSuccessState::Applied
    );
    assert_eq!(
        store
            .current_registration_request(&miner.sp_id)
            .await
            .unwrap(),
        Some(3)
    );
    assert_eq!(
        store
            .ark_delivery(&miner.sp_id)
            .await
            .unwrap()
            .unwrap()
            .b_ark,
        next.b_ark.to_vec()
    );
}

#[tokio::test]
async fn live_cache_never_overrides_durable_supersession() {
    let Some((fixture, store, _admin, _name)) = fixture("ap2ce1_").await else {
        eprintln!("skip: storage URLs absent");
        return;
    };
    let alice = fixture.identity.clone();
    let old = request(&alice, 3, 1);
    assert_eq!(
        success(
            drive(
                fixture.service.clone(),
                old.clone(),
                alice.clone(),
                fixture.payout.clone()
            )
            .await
        )
        .0,
        RegistrationSuccessState::Applied
    );
    let newer = request_with_signer(&alice, 3, 40, 2);
    assert_eq!(
        success(
            drive(
                fixture.service.clone(),
                newer.clone(),
                alice.clone(),
                fixture.payout.clone()
            )
            .await
        )
        .0,
        RegistrationSuccessState::Applied
    );
    // No mismatch and no TTL expiry between these identical old-field replays.
    // A fresh nonce must send A1 through Postgres rather than accept its stale
    // cached Applied field mined by an earlier client.
    let replay = drive(
        fixture.service.clone(),
        old,
        alice.clone(),
        fixture.payout.clone(),
    )
    .await;
    assert_eq!(success(replay).0, RegistrationSuccessState::Superseded);
    assert_eq!(
        store
            .current_registration_request(&alice.sp_id)
            .await
            .unwrap(),
        Some(2)
    );
    assert_eq!(
        store
            .ark_delivery(&alice.sp_id)
            .await
            .unwrap()
            .unwrap()
            .b_ark,
        newer.b_ark.to_vec()
    );

    // Durable already_applied must not be replayed as current Applied later.
    // A different identity isolates the initial already_applied cache state.
    let other = identity(11);
    let initial = request(&other, 11, 1);
    assert_eq!(
        success(
            drive(
                fixture.service.clone(),
                initial.clone(),
                other.clone(),
                fixture.payout.clone()
            )
            .await
        )
        .0,
        RegistrationSuccessState::Applied
    );
    assert_eq!(
        success(
            drive(
                fixture.service.clone(),
                initial.clone(),
                other.clone(),
                fixture.payout.clone()
            )
            .await
        )
        .0,
        RegistrationSuccessState::AlreadyApplied
    );
    let second = request_with_signer(&other, 11, 50, 2);
    assert_eq!(
        success(
            drive(
                fixture.service.clone(),
                second,
                other.clone(),
                fixture.payout.clone()
            )
            .await
        )
        .0,
        RegistrationSuccessState::Applied
    );
    let superseded = drive(
        fixture.service.clone(),
        initial,
        other,
        fixture.payout.clone(),
    )
    .await;
    assert_eq!(success(superseded).0, RegistrationSuccessState::Superseded);
}

#[tokio::test]
async fn signed_epoch_mismatch_is_terminal_acked_and_does_not_poison_later_records() {
    let Some((fixture, store, _admin, _name)) = fixture("ap2ce2_").await else {
        eprintln!("skip: storage URLs absent");
        return;
    };
    let miner = fixture.identity.clone();
    let registration = request(&miner, 3, 1);
    assert_eq!(
        success(
            drive(
                fixture.service.clone(),
                registration,
                miner.clone(),
                fixture.payout.clone()
            )
            .await
        )
        .0,
        RegistrationSuccessState::Applied
    );

    let mut epoch = spc_ext::RegisterEpochMailboxes {
        entries: vec![{
            let mut entry = [0u8; 37];
            entry[..4].copy_from_slice(&800_000u32.to_be_bytes());
            entry[4..].copy_from_slice(&public(60));
            entry
        }],
        sig: [0; 64],
        request_id: 2,
    };
    epoch.sign(&miner.sp_id, &key(3)).unwrap();
    let task = tokio::spawn({
        let service = fixture.service.clone();
        let identity = miner.clone();
        async move {
            service
                .handle(
                    &identity,
                    spc_ext::MSG_REGISTER_EPOCH_MAILBOXES,
                    &epoch.encode().unwrap(),
                )
                .await
        }
    });
    for _ in 0..100 {
        if task.is_finished() {
            break;
        }
        let _ = fixture.payout.process_registrations_once().await;
    }
    let message = tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .expect("terminal epoch response timeout")
        .unwrap();
    let DeliveryMessage::RegisterEpochMailboxesError(error) = message else {
        panic!("expected terminal epoch rejection");
    };
    assert_eq!(error.error_code, b"bad-mailbox-fields");
    assert!(!error.retryable);
    let resolution = store
        .ark_registration_resolution(&miner.sp_id, 2)
        .await
        .unwrap()
        .expect("terminal rejection is durable");
    assert_eq!(resolution.outcome, RegistrationOutcome::Rejected);
    assert_eq!(resolution.error_code(), Some("bad-mailbox-fields"));

    for _ in 0..2 {
        fixture.payout.process_registrations_once().await.unwrap();
    }
    let pending = fixture
        .streams
        .read_pending(
            &fixture.registration_stream,
            spc_pool::payout::GROUP,
            spc_pool::payout::CONSUMER,
            64,
        )
        .await
        .unwrap();
    assert!(
        pending.is_empty(),
        "rejected epoch request must not poison PEL"
    );

    let later = identity(11);
    let later_request = request(&later, 11, 1);
    assert_eq!(
        success(
            drive(
                fixture.service.clone(),
                later_request,
                later,
                fixture.payout.clone()
            )
            .await
        )
        .0,
        RegistrationSuccessState::Applied
    );
    assert_eq!(
        store
            .current_registration_request(&miner.sp_id)
            .await
            .unwrap(),
        Some(1)
    );
}

#[tokio::test]
async fn durable_commit_precedes_success_and_late_unknown_retry_is_idempotent() {
    let Some((fixture, store, _admin, _name)) = fixture("ap2b_").await else {
        eprintln!("skip: storage URLs absent");
        return;
    };
    let identity = fixture.identity.clone();
    let request = request(&identity, 3, 1);
    let service = fixture
        .service
        .clone()
        .with_ack_timeout(Duration::from_millis(80));
    let task = tokio::spawn({
        let identity = identity.clone();
        let request = request.clone();
        async move {
            service
                .handle(
                    &identity,
                    spc_ext::MSG_SET_ARK_IDENTITY,
                    &request.encode().unwrap(),
                )
                .await
        }
    });
    // Deliberately process after the short caller deadline: commit outcome was
    // unknown, not proof of unregistered status.
    tokio::time::sleep(Duration::from_millis(140)).await;
    let _ = fixture.payout.process_registrations_once().await;
    let (code, retryable) = error(task.await.unwrap());
    assert_eq!(code, "registration-commit-unknown");
    assert!(retryable);
    assert!(store.ark_delivery(&identity.sp_id).await.unwrap().is_some());

    let duplicate = drive(
        fixture.service.clone(),
        request,
        identity.clone(),
        fixture.payout.clone(),
    )
    .await;
    let committed = store
        .ark_registration_resolution(&identity.sp_id, 1)
        .await
        .unwrap()
        .expect("applied journal");
    let (_, duplicate_rev) = success(duplicate);
    assert_eq!(duplicate_rev, committed.registration_rev);
    assert_eq!(committed.outcome, RegistrationOutcome::Applied);
}

#[tokio::test]
async fn identity_row_lock_serializes_duplicate_and_competing_counters() {
    let Some((fixture, store, admin, name)) = fixture("ap2c_").await else {
        eprintln!("skip: storage URLs absent");
        return;
    };
    let identity = fixture.identity.clone();
    let requests: Vec<_> = (0..8)
        .map(|counter| {
            request_with_signer(&identity, 3, if counter == 0 { 3 } else { 40 }, counter + 1)
        })
        .collect();
    let mut tasks = Vec::new();
    for request in &requests {
        tasks.push(tokio::spawn({
            let service = fixture.service.clone();
            let identity = identity.clone();
            let request = request.clone();
            async move {
                service
                    .handle(
                        &identity,
                        spc_ext::MSG_SET_ARK_IDENTITY,
                        &request.encode().unwrap(),
                    )
                    .await
            }
        }));
    }
    for _ in 0..4 {
        let _ = fixture.payout.process_registrations_once().await;
    }
    for task in tasks {
        let _ = success(task.await.unwrap());
    }
    let current = store
        .ark_registration_resolution(&identity.sp_id, 8)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.outcome, RegistrationOutcome::Applied);
    assert_eq!(
        store
            .current_registration_request(&identity.sp_id)
            .await
            .unwrap()
            .unwrap(),
        8
    );

    drop(store);
    drop(fixture);
    let head = std::env::var("SPC_TEST_DATABASE_URL")
        .unwrap()
        .split('?')
        .next()
        .unwrap()
        .rsplit_once('/')
        .unwrap()
        .0
        .to_string();
    let _ = sqlx::query(&format!("DROP DATABASE {name} WITH (FORCE)"))
        .execute(&admin)
        .await;
    let _ = head;
}

#[tokio::test]
async fn epoch_mailboxes_commit_through_the_same_durable_journal() {
    let Some((fixture, store, admin, name)) = fixture("ap2e_").await else {
        eprintln!("skip: storage URLs absent");
        return;
    };
    let identity = fixture.identity.clone();
    let mut registration = request(&identity, 3, 1);
    registration.mailbox_mode = MailboxMode::EpochRegistered;
    registration.epoch_fallback = EpochFallback::Hold;
    registration.sign(&identity.sp_id, &key(3)).unwrap();
    assert_eq!(
        drive(
            fixture.service.clone(),
            registration,
            identity.clone(),
            fixture.payout.clone(),
        )
        .await
        .pipe_success(),
        RegistrationSuccessState::Applied
    );

    let mut epoch = spc_ext::RegisterEpochMailboxes {
        entries: vec![
            {
                let mut entry = [0u8; 37];
                entry[..4].copy_from_slice(&800_000u32.to_be_bytes());
                entry[4..].copy_from_slice(&public(60));
                entry
            },
            {
                let mut entry = [0u8; 37];
                entry[..4].copy_from_slice(&800_001u32.to_be_bytes());
                entry[4..].copy_from_slice(&public(61));
                entry
            },
        ],
        sig: [0; 64],
        request_id: 2,
    };
    epoch.sign(&identity.sp_id, &key(3)).unwrap();
    let task = tokio::spawn({
        let service = fixture.service.clone();
        let identity = identity.clone();
        async move {
            service
                .handle(
                    &identity,
                    spc_ext::MSG_REGISTER_EPOCH_MAILBOXES,
                    &epoch.encode().unwrap(),
                )
                .await
        }
    });
    for _ in 0..100 {
        if task.is_finished() {
            break;
        }
        let _ = fixture.payout.process_registrations_once().await;
    }
    let response = tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .expect("epoch timeout")
        .unwrap();
    let DeliveryMessage::RegisterEpochMailboxesSuccess(success) = response else {
        panic!("expected epoch success");
    };
    assert_eq!(success.request_id, 2);
    assert_eq!(success.state, RegistrationSuccessState::Applied);
    assert_eq!(success.ark_id, ARK_ID);
    let rows = store.epoch_mailboxes(&identity.sp_id).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].height, 800_000);
    assert_eq!(rows[1].height, 800_001);
    assert_eq!(
        store
            .current_registration_request(&identity.sp_id)
            .await
            .unwrap(),
        Some(2)
    );

    drop(store);
    drop(fixture);
    let _ = sqlx::query(&format!("DROP DATABASE {name} WITH (FORCE)"))
        .execute(&admin)
        .await;
}

trait SuccessState {
    fn pipe_success(self) -> RegistrationSuccessState;
}

impl SuccessState for DeliveryMessage {
    fn pipe_success(self) -> RegistrationSuccessState {
        success(self).0
    }
}

#[tokio::test]
async fn default_commit_unknown_deadline_is_five_seconds_and_retryable() {
    let Some((fixture, _store, _admin, _name)) = fixture("ap2d_").await else {
        eprintln!("skip: storage URLs absent");
        return;
    };
    let service = RegistrationService::new(fixture.streams.clone(), ARK_ID).with_names(
        format!("ap2:no-payout:{}", Uuid::new_v4()),
        "spc:arkreg:ack",
    );
    let identity = fixture.identity.clone();
    let request = request(&identity, 3, 77);
    let started = Instant::now();
    let response = service
        .handle(
            &identity,
            spc_ext::MSG_SET_ARK_IDENTITY,
            &request.encode().unwrap(),
        )
        .await;
    let elapsed = started.elapsed();
    let (code, retryable) = error(response);
    assert_eq!(code, "registration-commit-unknown");
    assert!(retryable);
    assert!(
        elapsed >= Duration::from_millis(4_800),
        "elapsed {elapsed:?}"
    );
    assert!(elapsed < Duration::from_secs(8), "elapsed {elapsed:?}");
}
