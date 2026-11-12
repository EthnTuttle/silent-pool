//! Scripted ownership/status/atomic-join checks, not funded chain acceptance.
use super::aggregate_receive::*;
use super::aggregate_watch::*;
use super::receive::{Expiry, ExpirySource, VtxoState};
use super::*;
use ark::{ProtocolEncoding, VtxoPolicy};
use bitcoin::hashes::Hash;
use bitcoin::secp256k1::{Keypair, PublicKey, Secp256k1, SecretKey};
use bitcoin::{Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness};

fn identity() -> StoreIdentity {
    StoreIdentity {
        wallet_fingerprint: bitcoin::bip32::Fingerprint::from([1; 4]),
        sp_id: [2; 32],
        network: "regtest".into(),
    }
}
fn key(n: u8) -> Keypair {
    Keypair::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&[n; 32]).unwrap())
}
fn status(state: &str) -> AggregateStatus {
    AggregateStatus {
        schema: "spc.aggregate-status/1".into(),
        wallet_received: None,
        sent_means: "fetch_and_verify_owned_vtxo_before_closing_watch".into(),
        obligations: (1..=2)
            .map(|n| Obligation {
                block_hash: [n; 32],
                height: n as u32,
                sats: 1000,
                due_height: n as u32 + 6,
                payout_state: "scheduled".into(),
                order_id: Some([3; 16]),
                vtxo_id: None,
                expiry_height: None,
            })
            .collect(),
        orders: vec![AggregateOrder {
            order_id: [3; 16],
            anchor_hash: [1; 32],
            anchor_height: 1,
            sats: 2000,
            state: state.into(),
            revision: 3,
            members: (1..=2)
                .map(|n| Member {
                    block_hash: [n; 32],
                    height: n as u32,
                    sats: 1000,
                    orphaned: false,
                })
                .collect(),
            vtxo_id: None,
            expiry_height: None,
            pool_loss_sats: 0,
        }],
    }
}
fn setup(dir: &std::path::Path) -> WalletStore {
    let store = WalletStore::open(dir, &identity()).unwrap();
    store.initialize_live_scan().unwrap();
    store
        .bind_aggregate_api(
            &identity().sp_id,
            "https://pool.invalid/",
            &key(8).x_only_public_key().0,
        )
        .unwrap();
    store
}
fn record() -> VtxoRecord {
    VtxoRecord {
        vtxo_id: [4; 36],
        u_pubkey: key(5).public_key(),
        amount_sats: 2000,
        state: VtxoState::Spendable,
        expiry: Expiry::At {
            height: 2000,
            source: ExpirySource::VtxoAncestry,
        },
    }
}

#[test]
fn aggregate_unknown_unbatched_frozen_sent_and_missing_views_never_close_watch() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup(dir.path());
    let mut unbatched = status("frozen");
    unbatched.orders.clear();
    for o in &mut unbatched.obligations {
        o.order_id = None;
    }
    store
        .ingest_aggregate_status(&identity().sp_id, &unbatched)
        .unwrap();
    let counts = || {
        store
            .lock()
            .query_row(
                "SELECT count(*) FROM aggregate_watches WHERE verified_order IS NULL",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
    };
    assert_eq!(counts(), 2);
    store
        .ingest_aggregate_status(&identity().sp_id, &status("frozen"))
        .unwrap();
    store
        .ingest_aggregate_status(&identity().sp_id, &status("sent"))
        .unwrap();
    assert_eq!(counts(), 2);
    let mut empty = status("sent");
    empty.orders.clear();
    empty.obligations.clear();
    store
        .ingest_aggregate_status(&identity().sp_id, &empty)
        .unwrap();
    assert_eq!(counts(), 2);
    assert!(store.ingest_aggregate_status(&[9; 32], &empty).is_err());
    assert!(
        store.frozen_aggregate_order(&[3; 16]).unwrap().is_none(),
        "no authenticated frozen order has been retained"
    );
    drop(store);
    let store = setup(dir.path());
    assert_eq!(store.aggregate_fetch_work().unwrap().len(), 1);
}

#[test]
fn aggregate_old_anchor_and_reorg_keep_whole_set_and_fair_watch() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup(dir.path());
    for n in 1..=3 {
        store
            .commit_scan_block(n, bitcoin::BlockHash::from_byte_array([n as u8; 32]), true)
            .unwrap();
    }
    store
        .ingest_aggregate_status(&identity().sp_id, &status("blocked_unknown"))
        .unwrap();
    store
        .lock()
        .execute("UPDATE live_scan_blocks SET reads=4", [])
        .unwrap();
    let due = store.due_epochs(5000).unwrap();
    assert!(due.iter().any(|(h, _, _)| *h == 1));
    assert!(
        !due.iter().any(|(h, _, _)| *h == 2),
        "nonanchor follows frozen anchor"
    );
    store
        .rewind_live_scan(1, bitcoin::BlockHash::from_byte_array([1; 32]))
        .unwrap();
    let count: i64 = store
        .lock()
        .query_row(
            "SELECT count(*) FROM aggregate_client_members WHERE order_id=?1",
            rusqlite::params![[3u8; 16].as_slice()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 2);
    let mut changed = status("blocked_unknown");
    changed.orders[0].members[1].sats = 999;
    changed.orders[0].sats = 1999;
    assert!(store
        .ingest_aggregate_status(&identity().sp_id, &changed)
        .is_err());
}

#[test]
fn aggregate_two_members_credit_once_atomic_partial_replay_and_foreign_allocation_refuse() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup(dir.path());
    let s = status("sent");
    store
        .ingest_aggregate_status(&identity().sp_id, &s)
        .unwrap();
    let mut partial = record();
    partial.amount_sats = 1000;
    assert!(store
        .commit_verified_aggregate(
            &s.orders[0],
            b"mock verified receipt",
            &[(partial, vec![1])],
            &[2; 33],
            b"mock origin"
        )
        .is_err());
    assert!(store.vtxo_records().unwrap().is_empty());
    store.lock().execute_batch("CREATE TRIGGER fail_second_allocation BEFORE UPDATE ON aggregate_watches WHEN OLD.height=2 BEGIN SELECT RAISE(ABORT,'mock crash'); END;").unwrap();
    assert!(store
        .commit_verified_aggregate(
            &s.orders[0],
            b"mock verified receipt",
            &[(record(), vec![1])],
            &[2; 33],
            b"mock origin"
        )
        .is_err());
    assert!(store.vtxo_records().unwrap().is_empty());
    store
        .lock()
        .execute_batch("DROP TRIGGER fail_second_allocation;")
        .unwrap();
    store
        .commit_verified_aggregate(
            &s.orders[0],
            b"mock verified receipt",
            &[(record(), vec![1])],
            &[2; 33],
            b"mock origin",
        )
        .unwrap();
    store
        .commit_verified_aggregate(
            &s.orders[0],
            b"mock verified receipt",
            &[(record(), vec![1])],
            &[2; 33],
            b"mock origin",
        )
        .unwrap();
    assert_eq!(store.vtxo_records().unwrap().len(), 1);
    assert_eq!(store.vtxo_records().unwrap()[0].amount_sats, 2000);
    assert_eq!(
        store
            .lock()
            .query_row(
                "SELECT count(*) FROM aggregate_watches WHERE verified_order IS NOT NULL",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
    assert!(store
        .commit_verified_aggregate(
            &s.orders[0],
            b"changed receipt",
            &[(record(), vec![1])],
            &[2; 33],
            b"mock origin"
        )
        .is_err());
    let mut foreign = status("sent");
    foreign.orders[0].order_id = [7; 16];
    foreign.orders[0].anchor_hash = [8; 32];
    foreign.orders[0].anchor_height = 8;
    foreign.orders[0].members = vec![Member {
        block_hash: [8; 32],
        height: 8,
        sats: 2000,
        orphaned: false,
    }];
    foreign.obligations.clear();
    store
        .ingest_aggregate_status(&identity().sp_id, &foreign)
        .unwrap();
    assert!(store
        .commit_verified_aggregate(
            &foreign.orders[0],
            b"other order",
            &[(record(), vec![1])],
            &[2; 33],
            b"mock origin"
        )
        .is_err());
}

fn board(owner: Keypair, server: Keypair) -> (Transaction, ark::Vtxo<ark::vtxo::Full>) {
    let builder = ark::board::BoardBuilder::new(
        owner.public_key(),
        2000u32.into(),
        server.public_key(),
        18u16.into(),
    );
    let tx = Transaction {
        version: bitcoin::transaction::Version(3),
        lock_time: bitcoin::absolute::LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::null(),
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ZERO,
            witness: Witness::new(),
        }],
        output: vec![TxOut {
            value: Amount::from_sat(2330),
            script_pubkey: builder.funding_script_pubkey(),
        }],
    };
    let point = OutPoint::new(tx.compute_txid(), 0);
    let builder = builder
        .set_funding_details(Amount::from_sat(2330), Amount::from_sat(330), point)
        .unwrap()
        .generate_user_nonces();
    let signing = ark::board::BoardBuilder::new_for_cosign(
        owner.public_key(),
        2000u32.into(),
        server.public_key(),
        18u16.into(),
        Amount::from_sat(2330),
        Amount::from_sat(330),
        point,
        *builder.user_pub_nonce(),
    );
    let vtxo = builder
        .build_vtxo(&signing.server_cosign(&server), &owner)
        .unwrap();
    vtxo.validate(&tx).unwrap();
    (tx, vtxo)
}

#[test]
fn aggregate_genuine_grant_does_not_authorize_same_u_wrong_policy_or_server() {
    use spc_ext::authorization::{
        order_digest, AuthorizationGrant, AuthorizationRequest, ProvedReceipt,
    };
    let owner = key(5);
    let server = key(6);
    let signer = key(8);
    let s = status("sent");
    let destination = ark::Address::new(
        true,
        ark::address::ArkId::from_server_pubkey(server.public_key()),
        VtxoPolicy::new_pubkey(owner.public_key()),
        vec![],
    );
    let mut order = spc_ext::ArkSendOrder {
        order_id: [3; 16],
        ark_address: destination.to_string(),
        sats: 2000,
        not_before_height: 8,
        sig: [0; 64],
    };
    order.sign(&signer).unwrap();
    let mut grant = AuthorizationGrant {
        request: AuthorizationRequest {
            order_id: [3; 16],
            order_digest: order_digest(&order).unwrap(),
            nonce: [7; 16],
        },
        revision: 2,
        expires_at: 100,
        signature: [0; 64],
    };
    grant.sign(&signer);
    let (anchor, input) = board(owner, server);
    let mut receipt = ProvedReceipt {
        grant: grant.clone(),
        sats: 2000,
        vtxos: vec![input.serialize()],
    };
    verify_order_envelope(
        &s.orders[0],
        &order,
        &receipt,
        &signer.x_only_public_key().0,
    )
    .unwrap();
    verify_output_policy(
        &input,
        &VtxoPolicy::new_pubkey(owner.public_key()),
        server.public_key(),
    )
    .unwrap();
    assert_eq!(
        decode_receipt_outputs(&receipt, destination.policy(), server.public_key())
            .unwrap()
            .len(),
        1
    );
    receipt.vtxos.push(input.serialize());
    assert!(
        decode_receipt_outputs(&receipt, destination.policy(), server.public_key()).is_err(),
        "duplicate IDs cannot satisfy a receipt"
    );
    let (wrong_anchor, wrong_server) = board(owner, key(9));
    wrong_server.validate(&wrong_anchor).unwrap();
    receipt.vtxos = vec![wrong_server.serialize()];
    verify_order_envelope(
        &s.orders[0],
        &order,
        &receipt,
        &signer.x_only_public_key().0,
    )
    .unwrap();
    assert!(verify_output_policy(
        &wrong_server,
        &VtxoPolicy::new_pubkey(owner.public_key()),
        server.public_key()
    )
    .is_err());
    assert!(decode_receipt_outputs(&receipt, destination.policy(), server.public_key()).is_err());
    let policy = VtxoPolicy::ServerHtlcSend(ark::vtxo::policy::ServerHtlcSendVtxoPolicy {
        user_pubkey: owner.public_key(),
        payment_hash: bitcoin::hashes::sha256::Hash::from_byte_array([11; 32]).into(),
        htlc_expiry: 1500u32.into(),
    });
    let builder =
        ark::arkoor::package::ArkoorPackageBuilder::new_claim_all_with_checkpoints([input], policy)
            .unwrap()
            .generate_user_nonces(&[owner])
            .unwrap();
    let response =
        ark::arkoor::package::ArkoorPackageBuilder::from_cosign_request(builder.cosign_request())
            .unwrap()
            .server_cosign(&server)
            .unwrap();
    let outputs = builder
        .user_cosign(&[owner], response.cosign_response())
        .unwrap()
        .build_signed_vtxos();
    let wrong_policy = &outputs[0];
    wrong_policy.validate(&anchor).unwrap();
    assert_eq!(wrong_policy.user_pubkey(), owner.public_key());
    receipt.vtxos = vec![wrong_policy.serialize()];
    verify_order_envelope(
        &s.orders[0],
        &order,
        &receipt,
        &signer.x_only_public_key().0,
    )
    .unwrap();
    assert!(verify_output_policy(
        wrong_policy,
        &VtxoPolicy::new_pubkey(owner.public_key()),
        server.public_key()
    )
    .is_err());
    assert!(decode_receipt_outputs(&receipt, destination.policy(), server.public_key()).is_err());
    let encoded = receipt.encode().unwrap();
    assert!(ProvedReceipt::decode(&encoded[..encoded.len() - 1]).is_err());
    let mut bad = encoded;
    bad.push(0);
    assert!(ProvedReceipt::decode(&bad).is_err());
    receipt.sats = 1999;
    assert!(verify_order_envelope(
        &s.orders[0],
        &order,
        &receipt,
        &signer.x_only_public_key().0
    )
    .is_err());
    receipt.sats = 2000;
    receipt.grant.signature[0] ^= 1;
    assert!(verify_order_envelope(
        &s.orders[0],
        &order,
        &receipt,
        &signer.x_only_public_key().0
    )
    .is_err());
}

/// Bounded JOIN check (client side): a signed grant only authorizes the
/// aggregate join while it strictly precedes the `sent` order's own
/// revision. A replayed/equal/future grant revision, or revision 0, must
/// refuse before any ledger or watch mutation.
///
/// This fixture exercises the client predicate in isolation. The producer
/// side of the same relation (the store's actual grant INSERT at
/// revision R followed by the Sent UPDATE at R+1, joined back into this
/// verifier) is proved against real PostgreSQL in
/// `aggregate_join_proof_tests.rs` (feature `postgres-integration`).
#[test]
fn aggregate_grant_revision_must_strictly_precede_sent_status_revision() {
    use spc_ext::authorization::{
        order_digest, AuthorizationGrant, AuthorizationRequest, ProvedReceipt,
    };
    let owner = key(5);
    let server = key(6);
    let signer = key(8);
    let s = status("sent");
    let destination = ark::Address::new(
        true,
        ark::address::ArkId::from_server_pubkey(server.public_key()),
        VtxoPolicy::new_pubkey(owner.public_key()),
        vec![],
    );
    let mut order = spc_ext::ArkSendOrder {
        order_id: [3; 16],
        ark_address: destination.to_string(),
        sats: 2000,
        not_before_height: 8,
        sig: [0; 64],
    };
    order.sign(&signer).unwrap();
    let (_, input) = board(owner, server);
    let build = |grant_revision: u64| {
        let mut grant = AuthorizationGrant {
            request: AuthorizationRequest {
                order_id: [3; 16],
                order_digest: order_digest(&order).unwrap(),
                nonce: [7; 16],
            },
            revision: grant_revision,
            expires_at: 100,
            signature: [0; 64],
        };
        grant.sign(&signer);
        ProvedReceipt {
            grant,
            sats: 2000,
            vtxos: vec![input.serialize()],
        }
    };
    // The status fixture carries order revision 3: a grant at revision 3
    // (equal, not strictly preceding) and one at revision 4 (future) both
    // refuse, as does the unsigned-meaning revision 0. Revision 2 (strictly
    // preceding) is accepted by the envelope join under the true signer,
    // so the ordering check is exactly what refused 3 and 4.
    assert!(verify_order_envelope(
        &s.orders[0],
        &order,
        &build(3),
        &signer.x_only_public_key().0
    )
    .is_err());
    assert!(verify_order_envelope(
        &s.orders[0],
        &order,
        &build(4),
        &signer.x_only_public_key().0
    )
    .is_err());
    assert!(verify_order_envelope(
        &s.orders[0],
        &order,
        &build(0),
        &signer.x_only_public_key().0
    )
    .is_err());
    verify_order_envelope(
        &s.orders[0],
        &order,
        &build(2),
        &signer.x_only_public_key().0,
    )
    .unwrap();
    // A non-sent order state never joins even with a strictly preceding
    // grant revision.
    let pending = status("pending");
    assert!(verify_order_envelope(
        &pending.orders[0],
        &order,
        &build(2),
        &signer.x_only_public_key().0
    )
    .is_err());
}

#[test]
fn aggregate_scanner_records_all_990_k_outputs_and_only_one_epoch() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup(dir.path());
    let scan = key(3).secret_key();
    let spend = key(4).public_key();
    let sender = key(7).public_key();
    let ax = sender.x_only_public_key().0.serialize();
    let even = PublicKey::from_x_only_public_key(
        sender.x_only_public_key().0,
        bitcoin::secp256k1::Parity::Even,
    );
    let shared = spc_crypto::SharedSecret::miner_side(1, &scan, &even).unwrap();
    // Recovery observations only; the imported ladder's own vectors check
    // decomposition. This fixture checks all-k scanning and persistence.
    let values = vec![5_000_000; 990];
    let scripts: Vec<_> = values
        .iter()
        .enumerate()
        .map(|(k, _)| {
            let p = spc_crypto::spc_output_pubkey(
                &spend,
                &spc_crypto::tier_tweaks(&shared, k as u32)
                    .unwrap()
                    .t_onchain,
            )
            .unwrap();
            let mut s = vec![0x51, 0x20];
            s.extend_from_slice(&p.x_only_public_key().0.serialize());
            s
        })
        .collect();
    let outputs: Vec<_> = scripts
        .iter()
        .zip(&values)
        .map(|(script, value)| spc_scan::OutputView {
            script_pubkey: script,
            value: *value,
        })
        .collect();
    let found = spc_scan::scan_outputs(1, &ax, &outputs, &scan, &spend).unwrap();
    assert_eq!(found.len(), 990);
    assert_eq!(found[989].k, 989);
    store
        .commit_scan_block_with_outputs(
            1,
            bitcoin::BlockHash::from_byte_array([1; 32]),
            true,
            &found,
        )
        .unwrap();
    assert_eq!(
        store
            .lock()
            .query_row("SELECT count(*) FROM owned_coinbase_outputs", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        990
    );
    assert_eq!(
        store
            .lock()
            .query_row(
                "SELECT count(*) FROM live_scan_blocks WHERE epoch=1",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    store
        .rewind_live_scan(0, bitcoin::BlockHash::all_zeros())
        .unwrap();
    assert_eq!(
        store
            .lock()
            .query_row(
                "SELECT count(*) FROM owned_coinbase_outputs WHERE canonical=0",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        990
    );
    store
        .commit_scan_block_with_outputs(
            1,
            bitcoin::BlockHash::from_byte_array([1; 32]),
            true,
            &found,
        )
        .unwrap();
    assert_eq!(
        store
            .lock()
            .query_row(
                "SELECT count(*) FROM owned_coinbase_outputs WHERE canonical=1",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        990
    );
    store
        .rewind_live_scan(0, bitcoin::BlockHash::all_zeros())
        .unwrap();
    let mut changed = found;
    changed[0].amount += 1;
    assert!(store
        .commit_scan_block_with_outputs(
            1,
            bitcoin::BlockHash::from_byte_array([1; 32]),
            true,
            &changed
        )
        .is_err());
    assert!(store.live_scan_blocks().unwrap().is_empty());
}

#[test]
fn aggregate_newest_first_cancellation_and_replacement_are_atomic() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup(dir.path());
    store
        .ingest_aggregate_status(&identity().sp_id, &status("frozen"))
        .unwrap();
    let mut next = status("frozen");
    let mut cancelled = next.orders[0].clone();
    cancelled.state = "cancelled".into();
    cancelled.revision += 1;
    next.orders[0].order_id = [4; 16];
    for row in &mut next.obligations {
        row.order_id = Some([4; 16]);
    }
    // Replacement precedes its cancelled predecessor, as in a newest-first API.
    next.orders.push(cancelled);
    store
        .ingest_aggregate_status(&identity().sp_id, &next)
        .unwrap();
    assert_eq!(store.lock().query_row("SELECT count(*) FROM aggregate_watches WHERE current_order=?1 AND verified_order IS NULL",rusqlite::params![[4u8;16].as_slice()],|r|r.get::<_,i64>(0)).unwrap(),2);
    let mut overlap = next.clone();
    overlap.orders[0].order_id = [5; 16];
    overlap.obligations.clear();
    assert!(store
        .ingest_aggregate_status(&identity().sp_id, &overlap)
        .is_err());
    assert_eq!(
        store
            .lock()
            .query_row("SELECT count(*) FROM aggregate_client_orders", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn aggregate_failed_receipts_rotate_past_eight_across_restart() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup(dir.path());
    let mut snapshot = status("sent");
    snapshot.obligations.clear();
    snapshot.orders.clear();
    for n in 1..=10u8 {
        let mut order = status("sent").orders.remove(0);
        order.order_id = [n; 16];
        order.anchor_hash = [n; 32];
        order.anchor_height = n as u32;
        order.members = vec![Member {
            block_hash: [n; 32],
            height: n as u32,
            sats: 2000,
            orphaned: false,
        }];
        snapshot.orders.push(order);
    }
    store
        .ingest_aggregate_status(&identity().sp_id, &snapshot)
        .unwrap();
    let first = store.aggregate_fetch_work().unwrap();
    assert_eq!(first.len(), 8);
    for order in first {
        store.begin_aggregate_fetch(&order.order_id).unwrap();
    }
    drop(store);
    let store = setup(dir.path());
    let next = store.aggregate_fetch_work().unwrap();
    assert_eq!(next[0].order_id, [9; 16]);
    assert_eq!(next[1].order_id, [10; 16]);
    assert_eq!(
        store
            .lock()
            .query_row(
                "SELECT count(*) FROM aggregate_watches WHERE verified_order IS NULL",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        10
    );
}

#[test]
fn aggregate_candidate_pages_resume_without_partial_credit_or_watch_closure() {
    use super::live_receive::MailboxBurst;
    let dir = tempfile::tempdir().unwrap();
    let store = setup(dir.path());
    let hash = bitcoin::BlockHash::from_byte_array([1; 32]);
    let mailbox = [2; 33];
    store.commit_scan_block(1, hash, true).unwrap();
    store.begin_epoch_read(1, 0).unwrap();
    let mut burst = MailboxBurst::default();
    for page in 1..=10u8 {
        store
            .persist_live_receipt(
                &[page; 36],
                b"scripted owned candidate",
                1,
                &hash.to_byte_array(),
                &mailbox,
            )
            .unwrap();
        store
            .advance_mailbox_checkpoint(&mailbox, i64::from(page))
            .unwrap();
        if let Some(result) = burst.committed_page(0, true).unwrap() {
            assert_eq!(page, 10);
            assert!(result.continuation);
            store.record_epoch_read(1, 0, result).unwrap();
        }
    }
    assert!(store.vtxo_records().unwrap().is_empty());
    drop(store);
    let store = setup(dir.path());
    assert_eq!(store.mailbox_checkpoint(&mailbox).unwrap(), 10);
    assert_eq!(store.due_epochs(7).unwrap(), vec![(1, hash, 0)]);
    store
        .persist_live_receipt(
            &[11; 36],
            b"scripted owned candidate",
            1,
            &hash.to_byte_array(),
            &mailbox,
        )
        .unwrap();
    // Durable candidate replay stays idempotent before cursor advancement.
    store
        .persist_live_receipt(
            &[11; 36],
            b"scripted owned candidate",
            1,
            &hash.to_byte_array(),
            &mailbox,
        )
        .unwrap();
    store.advance_mailbox_checkpoint(&mailbox, 11).unwrap();
    store
        .record_epoch_read(
            1,
            0,
            MailboxBurst::default()
                .committed_page(0, false)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
    assert_eq!(store.due_epochs(5000).unwrap(), vec![(1, hash, 1)]);
    assert!(store.vtxo_records().unwrap().is_empty());
    assert_eq!(
        store
            .lock()
            .query_row("SELECT count(*) FROM live_receipts", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        11
    );
}

#[test]
fn aggregate_frozen_order_is_exact_signed_immutable_bytes_not_reconstructed_status() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup(dir.path());
    store
        .ingest_aggregate_status(&identity().sp_id, &status("sent"))
        .unwrap();
    let destination = ark::Address::new(
        true,
        ark::address::ArkId::from_server_pubkey(key(6).public_key()),
        VtxoPolicy::new_pubkey(key(5).public_key()),
        vec![],
    );
    let mut order = spc_ext::ArkSendOrder {
        order_id: [3; 16],
        ark_address: destination.to_string(),
        sats: 2000,
        not_before_height: 8,
        sig: [0; 64],
    };
    assert!(store
        .retain_frozen_aggregate_order(&[3; 16], &order.encode().unwrap())
        .is_err());
    order.sign(&key(8)).unwrap();
    let bytes = order.encode().unwrap();
    store
        .retain_frozen_aggregate_order(&[3; 16], &bytes)
        .unwrap();
    store
        .retain_frozen_aggregate_order(&[3; 16], &bytes)
        .unwrap();
    let mut changed = order.clone();
    changed.not_before_height += 1;
    changed.sign(&key(8)).unwrap();
    assert!(store
        .retain_frozen_aggregate_order(&[3; 16], &changed.encode().unwrap())
        .is_err());
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(store
        .retain_frozen_aggregate_order(&[3; 16], &trailing)
        .is_err());
    changed = order.clone();
    changed.sign(&key(9)).unwrap();
    assert!(store
        .retain_frozen_aggregate_order(&[3; 16], &changed.encode().unwrap())
        .is_err());
    assert!(store
        .retain_frozen_aggregate_order(&[7; 16], &bytes)
        .is_err());
    assert!(store.vtxo_records().unwrap().is_empty());
    drop(store);
    let store = setup(dir.path());
    assert_eq!(
        store
            .frozen_aggregate_order(&[3; 16])
            .unwrap()
            .unwrap()
            .encode()
            .unwrap(),
        bytes
    );
    assert_eq!(
        store
            .lock()
            .query_row(
                "SELECT count(*) FROM aggregate_watches WHERE verified_order IS NULL",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
}
