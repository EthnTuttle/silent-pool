//! Integration suite 4: named regressions for every counterexample of
//! the 2026-11-08 Fable review (`docs/reviews/2026-11-08-traffic-wallet-fable.md`).
//!
//! Each test here mirrors one probe from the review (`/tmp/opencode/
//! T1a-fable-probe`, P1-P6, P8-P13): the exact scenario that reproduced
//! the invariant violation is driven again, and the post-fix invariant
//! is asserted. A test in this file failing means a review finding has
//! regressed.
//!
//! - H1 (P1, P2): an ever-uncertain (unknown / RPC-accepted /
//!   reorg-resubmitted) payment never releases its inputs on a later
//!   `rejected`; the durable counters decide, so crash replay takes the
//!   same decision.
//! - H2 (P3): displacing a confirmed transaction cascades - its own
//!   funded outputs are invalidated (remembering their prior status)
//!   and every descendant spending them is invalidated; recovery
//!   proceeds generation by generation through `reorg_resubmit`.
//! - H3 (P8, P9): `Reorged` re-enters the network exclusively through
//!   `ReorgResubmit` (proven-unspent, capped); the unknown cap bounds
//!   retry SENDS (N retries + the original = N+1 total), not outcome
//!   recording, and the count persists across restart.
//! - H4 (P6): one journal, one owner - a second live handle fails with
//!   `HeldByAnotherOwner` without mutating, in-process and across
//!   processes; owner exit releases the lock.
//! - H5 (P5, P10): funding observations carry provenance and a
//!   derivation locus, never a caller-supplied script; own-transaction
//!   backing requires the journaled confirmed creator with an exact
//!   vout match; coinbase funding is spendable only at 100
//!   confirmations of the observed tip (no tip = immature).
//! - M1 (P4): the command digest covers the payer role and the
//!   category, before and after signing.
//! - M2 (P12): spending resolves keys by direct derivation at the
//!   recorded index - the script lookahead never bounds the change
//!   chain.
//! - M3 (P13): multi-actor allocations are validated (disjoint index
//!   ranges, exactly one bootstrap host, one run identity) before any
//!   actor provisions.
//! - M4: an injected write fault either rolls back provably or poisons
//!   the handle; replay of the rolled-back (or torn) file reconstructs
//!   exactly the pre-fault state.
//! - M7 (P11): a late confirmation/conflict after a reorg cycle is
//!   journaled without error; only the finish is guarded.
//! - R1 (round 2): a REAL write fault (RLIMIT_FSIZE in a child process,
//!   SIGXFSZ ignored - not an injected fault) takes the same
//!   rollback-or-poison recovery path; no fragment survives, the next
//!   append is clean, the reopen is consistent and no ack is lost.
//! - R2 (round 2): a restart while a transaction is in `BroadcastSent`
//!   is journaled uncertainty (the durable `SessionStarted` record);
//!   a later rejection degrades instead of releasing, crash-looping
//!   reopens do not inflate the attempt counters, and the restart
//!   neither resets nor consumes the retry-send budget.
//! - R12 (round 2): a canonical re-inclusion of a reorged transaction
//!   is recordable from `Reorged` (block evidence, not a send) but must
//!   not name the displaced block; the restore cascade recovers one
//!   generation at a time.
//! - R6 (round 2): production provisioning is type-state enforced - it
//!   consumes an opaque `ValidatedAllocation` (disjoint ranges, exactly
//!   one bootstrap host, unique driver ids) whose canonical digest is
//!   persisted in the journal header and verified on reopen before any
//!   mutation.
//! - S3 (round 3): the displaced-block refusal is STALE-EVIDENCE only -
//!   the journal records the displacement seq and the last committed
//!   `ChainObserved` seq (both derived deterministically from the
//!   record stream), and a same-block re-inclusion becomes admissible
//!   once a newer chain observation exists (the chain reorged back);
//!   the ORIGINAL displacement seq is preserved for cascade-invalidated
//!   descendants promoted back to `Reorged`, so a reorg-back heals the
//!   whole cascade generation by generation.
//! - S4 (round 3): `note_rejected` always records the rejection (the
//!   journal-level transition rules are the sole legality gate) and
//!   guards only the finish - a genuine rejection of a finished command
//!   (e.g. `bad-txns-inputs-missingorspent` after a reorg resubmit) is
//!   journaled without error and degrades the inputs; classifying node
//!   answers ("already known / already in block chain") is T1b's
//!   concern, and the core never parses the code string.

mod common;

use std::collections::HashSet;
use std::ops::RangeInclusive;
use std::path::Path;

use bitcoin::hashes::Hash as _;
use bitcoin::{BlockHash, OutPoint};

use common::{
    conformance_set, decode_hex, fixture_outpoint, fund_payer, reopen_conformance_set,
    request_record_of, standard_test_config,
};
use spc_lab_txd::builder::{DestinationSpec, PaymentCategory};
use spc_lab_txd::error::{Error, JournalError};
use spc_lab_txd::journal::{
    CommandPhase, FundingContext, FundingObservation, FundingSource, JournalCaps, TrafficJournal,
    TxState, WriteFault,
};
use spc_lab_txd::keys::{Chain, WalletRole};
use spc_lab_txd::wallet::{
    PaymentOutcome, PaymentRequest, ProvisionConfig, SignedPaymentView, TrafficWalletSet,
};

const PAYER: WalletRole = WalletRole::BootstrapFunding;
const DEST: WalletRole = WalletRole::NetworkWallet { index: 0 };

fn block_hash(byte: u8) -> BlockHash {
    BlockHash::from_byte_array([byte; 32])
}

/// A payment of `amount_sats` from the payer to the destination owner's
/// next receive address (ordinary category, 3 sat/vB).
fn payment_request(set: &TrafficWalletSet, command_id: &str, amount_sats: u64) -> PaymentRequest {
    let address = set
        .next_receive_address(&DEST)
        .expect("destination receive address");
    PaymentRequest {
        command_id: command_id.to_string(),
        payer_role: PAYER.clone(),
        category: PaymentCategory::Ordinary,
        destinations: vec![DestinationSpec {
            script_pubkey: address.script_pubkey,
            amount_sats,
            owner: Some(DEST.clone()),
        }],
        fee_rate_sats_vb: 3,
    }
}

fn signed_view(set: &TrafficWalletSet, request: &PaymentRequest) -> SignedPaymentView {
    match set.sign_payment(request).expect("sign payment") {
        PaymentOutcome::Signed(view) => view,
        PaymentOutcome::ReplayedStored(_) => panic!("expected a fresh signature"),
    }
}

/// A synthetic fixture observation for the payer at its next receive
/// index (the fixed P5/P10/P12 counterexample shapes).
fn fixture_observation(set: &TrafficWalletSet, byte: u8, amount_sats: u64) -> FundingObservation {
    FundingObservation {
        outpoint: fixture_outpoint(byte),
        amount_sats,
        chain: Chain::Receive,
        index: set.journal().next_receive_index(&PAYER),
        owner_role: PAYER.clone(),
        source: FundingSource::SyntheticFixture,
        context: FundingContext::Fixture,
    }
}

/// Sign, broadcast, RPC-accept and canonically confirm one payment in
/// `block_hash(byte)` at `height`; returns the published view.
fn sign_broadcast_accept_confirm(
    set: &TrafficWalletSet,
    command_id: &str,
    amount_sats: u64,
    block_byte: u8,
    height: u64,
) -> SignedPaymentView {
    let view = signed_view(set, &payment_request(set, command_id, amount_sats));
    set.note_broadcast_sent(command_id)
        .expect("broadcast (original send)");
    set.note_accepted_by_rpc(command_id, "node-1")
        .expect("rpc acceptance");
    set.note_confirmed(command_id, block_hash(block_byte), height)
        .expect("canonical confirmation");
    view
}

/// Observe the change output (vout 1) of `view` as own-transaction
/// funding for the payer, at the change index the signed transaction
/// recorded (H5: the locus is carried, the script is derived).
fn observe_own_change(set: &TrafficWalletSet, view: &SignedPaymentView) {
    let parsed = common::parse_raw_tx(&decode_hex(&view.raw_hex));
    let change_value = parsed.outputs[1].value_sats;
    let change_index = view.change_index.expect("change index published");
    set.observe_funding(&FundingObservation {
        outpoint: OutPoint::new(view.txid, 1),
        amount_sats: change_value,
        chain: Chain::Change,
        index: change_index,
        owner_role: view.payer_role.clone(),
        source: FundingSource::OwnTransaction,
        context: FundingContext::OwnTransaction,
    })
    .expect("observe own change funding");
}

// ---------------------------------------------------------------------
// H1 (P1): reject after an unknown retry send never releases the inputs
// ---------------------------------------------------------------------

#[test]
fn h1_p1_reject_after_unknown_retry_degrades_to_possibly_spent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let set = conformance_set(dir.path());
    fund_payer(&set, &PAYER, fixture_outpoint(0x01), 1_000_000);
    let view = signed_view(&set, &payment_request(&set, "c1", 250_000));
    set.note_broadcast_sent("c1").expect("original send");
    set.note_unknown("c1", "timeout; node may have accepted")
        .expect("record unknown outcome");
    set.note_broadcast_sent("c1")
        .expect("same-bytes retry send");
    // The node answers for the send whose outcome was unknown (e.g.
    // maps "txn-already-in-mempool" to a rejection).
    set.note_rejected("c1", "txn-already-in-mempool")
        .expect("record rejection");
    assert_eq!(set.journal().tx_state(&view.txid), Some(TxState::Rejected));
    // FIXED: the input is NOT released - the payment was ever-unknown,
    // so the claim degrades conservatively to possibly-spent.
    assert_eq!(set.balance_sats(&PAYER), 0);
    let summary = set.journal().summary();
    assert_eq!(summary.possibly_spent_outpoints, 1);
    assert_eq!(summary.available_outpoints, 0);
    // No second payment can double-spend the outpoint.
    let error = set
        .sign_payment(&payment_request(&set, "c2", 250_000))
        .expect_err("ever-unknown input is not reselectable");
    assert!(matches!(
        error,
        Error::Builder(spc_lab_txd::error::BuilderError::InsufficientFunds { .. })
    ));
    // The decision is durable history, not live state: crash replay
    // reconstructs the same possibly-spent claim.
    drop(set);
    let reopened = reopen_conformance_set(dir.path());
    assert_eq!(reopened.balance_sats(&PAYER), 0);
    assert_eq!(reopened.journal().summary().possibly_spent_outpoints, 1);
    assert_eq!(
        reopened.journal().tx_state(&view.txid),
        Some(TxState::Rejected)
    );
}

// ---------------------------------------------------------------------
// H1 (P2): reject after an RPC acceptance never releases the inputs
// ---------------------------------------------------------------------

#[test]
fn h1_p2_reject_after_rpc_accept_degrades_to_possibly_spent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let set = conformance_set(dir.path());
    fund_payer(&set, &PAYER, fixture_outpoint(0x02), 1_000_000);
    let view = signed_view(&set, &payment_request(&set, "c1", 250_000));
    set.note_broadcast_sent("c1").expect("broadcast");
    set.note_accepted_by_rpc("c1", "node-1")
        .expect("rpc acceptance");
    set.note_rejected("c1", "node-2 rejected")
        .expect("record rejection");
    assert_eq!(set.journal().tx_state(&view.txid), Some(TxState::Rejected));
    // FIXED: at least one node held the bytes; the inputs degrade to
    // possibly-spent, never back to available.
    assert_eq!(set.balance_sats(&PAYER), 0);
    assert_eq!(set.journal().summary().possibly_spent_outpoints, 1);
    let error = set
        .sign_payment(&payment_request(&set, "c2", 250_000))
        .expect_err("ever-accepted input is not reselectable");
    assert!(matches!(
        error,
        Error::Builder(spc_lab_txd::error::BuilderError::InsufficientFunds { .. })
    ));
}

// ---------------------------------------------------------------------
// H1 (direct variant): a rejection arriving for a send whose outcome
// was recorded unknown (the (Unknown, Rejected) edge)
// ---------------------------------------------------------------------

#[test]
fn h1_unknown_then_direct_rejected_degrades_to_possibly_spent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let set = conformance_set(dir.path());
    fund_payer(&set, &PAYER, fixture_outpoint(0x07), 1_000_000);
    let view = signed_view(&set, &payment_request(&set, "c1", 250_000));
    set.note_broadcast_sent("c1").expect("broadcast");
    set.note_unknown("c1", "transport timeout")
        .expect("record unknown outcome");
    // The node answers the original send late.
    set.note_rejected("c1", "bad-txns-inputs-missingorspent")
        .expect("record rejection");
    assert_eq!(set.journal().tx_state(&view.txid), Some(TxState::Rejected));
    assert_eq!(set.balance_sats(&PAYER), 0);
    assert_eq!(set.journal().summary().possibly_spent_outpoints, 1);
}

// ---------------------------------------------------------------------
// H2 (P3): displacing the parent cascades through three generations
// and recovery proceeds generation by generation
// ---------------------------------------------------------------------

#[test]
fn h2_p3_three_generation_cascade_and_recovery() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = standard_test_config();
    let set = TrafficWalletSet::provision_for_conformance_tests(&config, dir.path())
        .expect("provision conformance set");

    // Generation 1: f0 -> c1 (dest1 + change1), confirmed in B1. The
    // destination index is captured before the request is built (it is
    // consumed by the observation, not by address derivation).
    let f0 = fund_payer(&set, &PAYER, fixture_outpoint(0x03), 1_000_000).0;
    let dest1_index = set.journal().next_receive_index(&DEST);
    let view1 = sign_broadcast_accept_confirm(&set, "c1", 250_000, 0x33, 10);
    set.observe_funding(&FundingObservation {
        outpoint: OutPoint::new(view1.txid, 0),
        amount_sats: view1.destinations[0].amount_sats,
        chain: Chain::Receive,
        index: dest1_index,
        owner_role: DEST.clone(),
        source: FundingSource::OwnTransaction,
        context: FundingContext::OwnTransaction,
    })
    .expect("observe dest1");
    observe_own_change(&set, &view1);
    let change1 = OutPoint::new(view1.txid, 1);

    // Generation 2: change1 -> c2 (dest2 + change2), confirmed in B2.
    let dest2_index = set.journal().next_receive_index(&DEST);
    let view2 = sign_broadcast_accept_confirm(&set, "c2", 100_000, 0x34, 11);
    assert_eq!(view2.spent_outpoints, vec![change1]);
    set.observe_funding(&FundingObservation {
        outpoint: OutPoint::new(view2.txid, 0),
        amount_sats: view2.destinations[0].amount_sats,
        chain: Chain::Receive,
        index: dest2_index,
        owner_role: DEST.clone(),
        source: FundingSource::OwnTransaction,
        context: FundingContext::OwnTransaction,
    })
    .expect("observe dest2");
    observe_own_change(&set, &view2);
    let change2 = OutPoint::new(view2.txid, 1);

    // Generation 3: change2 -> c3 (dest3 + change3), confirmed in B3.
    let dest3_index = set.journal().next_receive_index(&DEST);
    let view3 = sign_broadcast_accept_confirm(&set, "c3", 50_000, 0x35, 12);
    assert_eq!(view3.spent_outpoints, vec![change2]);
    set.observe_funding(&FundingObservation {
        outpoint: OutPoint::new(view3.txid, 0),
        amount_sats: view3.destinations[0].amount_sats,
        chain: Chain::Receive,
        index: dest3_index,
        owner_role: DEST.clone(),
        source: FundingSource::OwnTransaction,
        context: FundingContext::OwnTransaction,
    })
    .expect("observe dest3");
    observe_own_change(&set, &view3);

    // Healthy chain: everything confirmed and spendable.
    let change3_value = set.balance_sats(&PAYER);
    assert!(change3_value > 500_000);
    assert_eq!(set.balance_sats(&DEST), 400_000);
    assert_eq!(set.journal().summary().invalidated_outpoints, 0);

    // Displace B1 (H5 canonical chain observation).
    set.note_chain_observed(block_hash(0x99), 13, &[block_hash(0x33)])
        .expect("chain observation displacing B1");
    // FIXED: c1 is reorged and every own output of it is invalidated;
    // the cascade invalidates c2 and c3 (descendants) and their outputs.
    assert_eq!(set.journal().tx_state(&view1.txid), Some(TxState::Reorged));
    assert_eq!(
        set.journal().tx_state(&view2.txid),
        Some(TxState::Invalidated)
    );
    assert_eq!(
        set.journal().tx_state(&view3.txid),
        Some(TxState::Invalidated)
    );
    assert_eq!(set.balance_sats(&DEST), 0);
    assert_eq!(set.balance_sats(&PAYER), 0);
    let summary = set.journal().summary();
    assert_eq!(summary.possibly_spent_outpoints, 1); // f0
    assert_eq!(summary.invalidated_outpoints, 6); // change1, dest1, change2, dest2, change3, dest3

    // No descendant can be confirmed or spent while invalidated.
    let error = set
        .note_confirmed("c2", block_hash(0x34), 11)
        .expect_err("confirmation over an invalidated input is refused");
    assert!(matches!(
        error,
        Error::Journal(JournalError::IllegalTransition { .. })
    ));

    // Recovery, generation by generation. c1 resubmits with f0 proven
    // unspent (its possibly-spent claim) and reconfirms.
    let raw1 = decode_hex(&view1.raw_hex);
    set.reorg_resubmit("c1", &raw1, &[f0])
        .expect("resubmit c1 with proven-unspent input");
    set.note_accepted_by_rpc("c1", "node-1").expect("accept c1");
    set.note_confirmed("c1", block_hash(0x3a), 10)
        .expect("reconfirm c1");
    assert_eq!(
        set.journal().tx_state(&view1.txid),
        Some(TxState::Confirmed)
    );
    // dest1 is restored available; change1 restores conservatively
    // (possibly-spent, its prior was spent by c2); c2 is reorged again
    // (its prior was confirmed); c3 is still invalidated.
    assert_eq!(set.balance_sats(&DEST), 250_000);
    assert_eq!(set.balance_sats(&PAYER), 0);
    assert_eq!(set.journal().tx_state(&view2.txid), Some(TxState::Reorged));
    assert_eq!(
        set.journal().tx_state(&view3.txid),
        Some(TxState::Invalidated)
    );
    let summary = set.journal().summary();
    assert_eq!(summary.possibly_spent_outpoints, 1); // change1
    assert_eq!(summary.invalidated_outpoints, 4); // change2, dest2, change3, dest3

    // Generation 2 recovers.
    let raw2 = decode_hex(&view2.raw_hex);
    set.reorg_resubmit("c2", &raw2, &[change1])
        .expect("resubmit c2 with proven-unspent input");
    set.note_accepted_by_rpc("c2", "node-1").expect("accept c2");
    set.note_confirmed("c2", block_hash(0x3b), 11)
        .expect("reconfirm c2");
    assert_eq!(
        set.journal().tx_state(&view2.txid),
        Some(TxState::Confirmed)
    );
    assert_eq!(set.balance_sats(&DEST), 350_000);
    assert_eq!(set.journal().tx_state(&view3.txid), Some(TxState::Reorged));
    let summary = set.journal().summary();
    assert_eq!(summary.possibly_spent_outpoints, 1); // change2
    assert_eq!(summary.invalidated_outpoints, 2); // change3, dest3

    // Generation 3 recovers.
    let raw3 = decode_hex(&view3.raw_hex);
    set.reorg_resubmit("c3", &raw3, &[change2])
        .expect("resubmit c3 with proven-unspent input");
    set.note_accepted_by_rpc("c3", "node-1").expect("accept c3");
    set.note_confirmed("c3", block_hash(0x3c), 12)
        .expect("reconfirm c3");
    assert_eq!(
        set.journal().tx_state(&view3.txid),
        Some(TxState::Confirmed)
    );

    // Fully restored: same balances as before the reorg.
    assert_eq!(set.balance_sats(&DEST), 400_000);
    assert_eq!(set.balance_sats(&PAYER), change3_value);
    let summary = set.journal().summary();
    assert_eq!(summary.possibly_spent_outpoints, 0);
    assert_eq!(summary.invalidated_outpoints, 0);

    // The cascade is deterministic from the record stream: replay
    // reconstructs the same recovered world.
    drop(set);
    let reopened = reopen_conformance_set(dir.path());
    assert_eq!(reopened.balance_sats(&DEST), 400_000);
    assert_eq!(reopened.balance_sats(&PAYER), change3_value);
    assert_eq!(
        reopened.journal().tx_state(&view1.txid),
        Some(TxState::Confirmed)
    );
    assert_eq!(
        reopened.journal().tx_state(&view3.txid),
        Some(TxState::Confirmed)
    );
}

// ---------------------------------------------------------------------
// M1 (P4): the command digest covers the payer role and the category
// ---------------------------------------------------------------------

#[test]
fn m1_p4_payer_or_category_change_is_a_command_conflict() {
    let dir = tempfile::tempdir().expect("tempdir");
    let set = conformance_set(dir.path());
    fund_payer(&set, &PAYER, fixture_outpoint(0x04), 1_000_000);
    fund_payer(
        &set,
        &WalletRole::NetworkWallet { index: 3 },
        fixture_outpoint(0x05),
        1_000_000,
    );

    let request = payment_request(&set, "c1", 250_000);
    signed_view(&set, &request);

    // FIXED: same command id + same payload but a different payer role
    // is a journaled conflict - the bootstrap tx is never replayed for
    // another wallet.
    let mut other_payer = request.clone();
    other_payer.payer_role = WalletRole::NetworkWallet { index: 3 };
    let error = set
        .sign_payment(&other_payer)
        .expect_err("payer change must conflict");
    assert!(matches!(
        error,
        Error::Journal(JournalError::CommandConflict { .. })
    ));

    // Same command id + different category is a conflict too.
    let mut other_category = request.clone();
    other_category.category = PaymentCategory::UserPayment;
    let error = set
        .sign_payment(&other_category)
        .expect_err("category change must conflict");
    assert!(matches!(
        error,
        Error::Journal(JournalError::CommandConflict { .. })
    ));

    // The same conflict is detected before any signature exists
    // (crash between begin and sign): the digest is computed over the
    // full request record at begin time.
    let request_b = payment_request(&set, "c2", 100_000);
    set.journal()
        .begin_command("c2", request_record_of(&request_b))
        .expect("begin c2");
    let mut tampered = request_b.clone();
    tampered.payer_role = WalletRole::NetworkWallet { index: 3 };
    let error = set
        .journal()
        .begin_command("c2", request_record_of(&tampered))
        .expect_err("begin with a different payer must conflict");
    assert!(matches!(error, JournalError::CommandConflict { .. }));

    // The original request still replays its stored bytes.
    match set.sign_payment(&request).expect("replay") {
        PaymentOutcome::ReplayedStored(view) => {
            assert_eq!(view.payer_role, PAYER);
        }
        PaymentOutcome::Signed(_) => panic!("expected the stored transaction"),
    }
}

// ---------------------------------------------------------------------
// H5 (P5): funding observations are verified claims, not caller input
// ---------------------------------------------------------------------

#[test]
fn h5_p5_funding_backing_and_derivation_are_verified() {
    let dir = tempfile::tempdir().expect("tempdir");
    let set = conformance_set(dir.path());

    // FIXED: own-transaction funding for an outpoint whose creating
    // transaction is not journaled is refused (the journal has never
    // seen this txid).
    let error = set
        .observe_funding(&FundingObservation {
            outpoint: fixture_outpoint(0x50),
            amount_sats: 123_456_789,
            chain: Chain::Receive,
            index: 0,
            owner_role: PAYER.clone(),
            source: FundingSource::OwnTransaction,
            context: FundingContext::OwnTransaction,
        })
        .expect_err("unjournaled creator must be refused");
    assert!(matches!(
        error,
        Error::Journal(JournalError::FundingNotVerifiable { .. })
    ));
    assert_eq!(set.balance_sats(&PAYER), 0);

    // A real own-tx output, but observed at the wrong derivation locus:
    // FIXED: the wallet derives the script at the claimed locus and the
    // journal compares it with the decoded creating-tx output - a
    // foreign script can never be claimed through this API (there is no
    // script field to claim with).
    fund_payer(&set, &PAYER, fixture_outpoint(0x51), 1_000_000);
    let dest_index = set.journal().next_receive_index(&DEST);
    let view = sign_broadcast_accept_confirm(&set, "c1", 250_000, 0x52, 5);
    let error = set
        .observe_funding(&FundingObservation {
            outpoint: OutPoint::new(view.txid, 0),
            amount_sats: 250_000,
            chain: Chain::Receive,
            index: dest_index + 1, // wrong locus
            owner_role: DEST.clone(),
            source: FundingSource::OwnTransaction,
            context: FundingContext::OwnTransaction,
        })
        .expect_err("wrong derivation locus must be refused");
    assert!(matches!(
        error,
        Error::Journal(JournalError::FundingNotVerifiable { .. })
    ));

    // Wrong amount is refused too (byte-exact vout match).
    let error = set
        .observe_funding(&FundingObservation {
            outpoint: OutPoint::new(view.txid, 0),
            amount_sats: 249_999,
            chain: Chain::Receive,
            index: dest_index,
            owner_role: DEST.clone(),
            source: FundingSource::OwnTransaction,
            context: FundingContext::OwnTransaction,
        })
        .expect_err("wrong amount must be refused");
    assert!(matches!(
        error,
        Error::Journal(JournalError::FundingNotVerifiable { .. })
    ));

    // The correct observation (locus + amount + journaled confirmed
    // creator) credits the owner.
    set.observe_funding(&FundingObservation {
        outpoint: OutPoint::new(view.txid, 0),
        amount_sats: 250_000,
        chain: Chain::Receive,
        index: dest_index,
        owner_role: DEST.clone(),
        source: FundingSource::OwnTransaction,
        context: FundingContext::OwnTransaction,
    })
    .expect("correct own-transaction observation");
    assert_eq!(set.balance_sats(&DEST), 250_000);
}

// ---------------------------------------------------------------------
// H4 (P6): a second live owner fails closed without mutating
// ---------------------------------------------------------------------

/// Reopen the journal of a conformance set at the journal level (the
/// digest and driver id come from the validated allocation - the same
/// token the production wallet API consumes, R6).
fn reopen_conformance_journal(
    dir: &Path,
    config: &ProvisionConfig,
) -> Result<TrafficJournal, JournalError> {
    let allocation = ProvisionConfig::validate_allocation(std::slice::from_ref(config))
        .expect("the standard single-config allocation is valid");
    TrafficJournal::reopen_for_conformance_tests(
        dir,
        &config.caps,
        allocation.digest(),
        &config.driver_id,
    )
}

#[test]
fn h4_p6_second_live_owner_fails_without_mutation() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = standard_test_config();
    let set = TrafficWalletSet::provision_for_conformance_tests(&config, dir.path()).expect("set");
    fund_payer(&set, &PAYER, fixture_outpoint(0x06), 1_000_000);
    let seq_before = set.journal().last_seq();

    // FIXED: the second live handle is refused by the kernel flock
    // before any replay or truncation.
    let error =
        reopen_conformance_journal(dir.path(), &config).expect_err("second live owner must fail");
    assert!(matches!(error, JournalError::HeldByAnotherOwner { .. }));
    // Nothing was mutated and the live owner still works.
    assert_eq!(set.journal().last_seq(), seq_before);
    signed_view(&set, &payment_request(&set, "a", 250_000));
}

#[test]
fn h4_owner_exit_releases_the_lock() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = standard_test_config();
    {
        let set = TrafficWalletSet::provision_for_conformance_tests(&config, dir.path())
            .expect("provision");
        fund_payer(&set, &PAYER, fixture_outpoint(0x60), 1_000_000);
        // The set is dropped here: the exclusive flock is released.
    }
    let reopened =
        reopen_conformance_journal(dir.path(), &config).expect("reopen after owner exit");
    assert_eq!(reopened.balance_sats(&PAYER), 1_000_000);
}

/// Child mode of `h4_lock_held_across_processes`: opens the journal
/// (taking the exclusive flock), signals readiness and parks until the
/// parent kills the process. Never run directly.
#[test]
#[ignore = "child mode: spawned by h4_lock_held_across_processes"]
fn h4_lock_child() {
    let run_dir = std::env::var_os("SPC_TXD_FABLE_LOCK_RUN_DIR")
        .expect("SPC_TXD_FABLE_LOCK_RUN_DIR is set by the parent test");
    let ready = std::env::var_os("SPC_TXD_FABLE_LOCK_READY")
        .expect("SPC_TXD_FABLE_LOCK_READY is set by the parent test");
    let allocation_digest = std::env::var("SPC_TXD_FABLE_LOCK_ALLOCATION_DIGEST")
        .expect("SPC_TXD_FABLE_LOCK_ALLOCATION_DIGEST is set by the parent test");
    let driver_id = std::env::var("SPC_TXD_FABLE_LOCK_DRIVER_ID")
        .expect("SPC_TXD_FABLE_LOCK_DRIVER_ID is set by the parent test");
    let journal = TrafficJournal::reopen_for_conformance_tests(
        Path::new(&run_dir),
        &JournalCaps::default(),
        &allocation_digest,
        &driver_id,
    )
    .expect("child opens the journal and takes the flock");
    std::fs::write(&ready, b"ready").expect("child signals readiness");
    // Park (bounded) until the parent kills us.
    for _ in 0..300 {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    drop(journal);
}

#[test]
fn h4_lock_held_across_processes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = standard_test_config();
    {
        let _set = TrafficWalletSet::provision_for_conformance_tests(&config, dir.path())
            .expect("provision");
        // Dropped: the lock is free for the child to take.
    }
    let allocation = ProvisionConfig::validate_allocation(std::slice::from_ref(&config))
        .expect("the standard single-config allocation is valid");
    let ready = dir.path().join("child-ready");
    let mut child = std::process::Command::new(std::env::current_exe().expect("test binary"))
        .args(["--exact", "h4_lock_child", "--ignored", "--nocapture"])
        .env("SPC_TXD_FABLE_LOCK_RUN_DIR", dir.path())
        .env("SPC_TXD_FABLE_LOCK_READY", &ready)
        .env("SPC_TXD_FABLE_LOCK_ALLOCATION_DIGEST", allocation.digest())
        .env("SPC_TXD_FABLE_LOCK_DRIVER_ID", &config.driver_id)
        .spawn()
        .expect("spawn the child-mode lock holder");

    // Wait for the child to hold the lock (bounded).
    let mut held = false;
    for _ in 0..400 {
        if ready.exists() {
            held = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(held, "child did not take the journal lock in time");

    // FIXED: while another process holds the journal, reopening fails
    // with HeldByAnotherOwner (the flock is a real process lock).
    let error = reopen_conformance_journal(dir.path(), &config)
        .expect_err("cross-process second owner must fail");
    assert!(matches!(error, JournalError::HeldByAnotherOwner { .. }));

    child.kill().expect("kill the child holder");
    child.wait().expect("reap the child holder");
    // The kernel released the flock at process exit.
    let reopened = reopen_conformance_journal(dir.path(), &config)
        .expect("reopen after the holder process exited");
    drop(reopened);
}

// ---------------------------------------------------------------------
// H3 (P8): a reorged transaction re-enters only via ReorgResubmit
// ---------------------------------------------------------------------

#[test]
fn h3_p8_reorged_reenters_only_via_resubmit() {
    let dir = tempfile::tempdir().expect("tempdir");
    let set = conformance_set(dir.path());
    fund_payer(&set, &PAYER, fixture_outpoint(0x08), 1_000_000);
    let view = signed_view(&set, &payment_request(&set, "c1", 250_000));
    set.note_broadcast_sent("c1").expect("broadcast");
    set.note_accepted_by_rpc("c1", "node-1").expect("accept");
    // Crash between record_confirmed and finish_command: the command
    // phase stays Signed (the exact P8 setup).
    set.journal()
        .record_confirmed("c1", block_hash(0x88), 5)
        .expect("confirm without finish");
    set.note_reorged("c1", block_hash(0x88), 5).expect("reorg");
    assert_eq!(set.journal().tx_state(&view.txid), Some(TxState::Reorged));

    // FIXED: a plain broadcast from Reorged is refused - there is no
    // (Reorged, BroadcastSent) edge, so the proven-unspent check and
    // the resubmit cap cannot be bypassed.
    let error = set
        .note_broadcast_sent("c1")
        .expect_err("broadcast from reorged must be refused");
    assert!(matches!(
        error,
        Error::Journal(JournalError::IllegalTransition { .. })
    ));

    // Resubmission must prove every original input unspent.
    let raw = decode_hex(&view.raw_hex);
    let error = set
        .reorg_resubmit("c1", &raw, &[])
        .expect_err("unproven resubmission must be refused");
    assert!(matches!(
        error,
        Error::Journal(JournalError::ReorgRetryNotEligible { .. })
    ));

    // With the proof, resubmission is admitted exactly up to the cap.
    let cap = set.journal().caps().reorg_resubmits;
    for round in 0..cap {
        set.reorg_resubmit("c1", &raw, &view.spent_outpoints)
            .unwrap_or_else(|e| panic!("resubmit round {round} within cap: {e}"));
        set.note_accepted_by_rpc("c1", "node-1").expect("accept");
        set.note_confirmed("c1", block_hash(0x88), 5)
            .expect("reconfirm (guarded finish)");
        set.note_reorged("c1", block_hash(0x88), 5)
            .expect("reorg again");
    }
    let facts = set.journal().tx_facts(&view.txid).expect("tx facts");
    assert_eq!(facts.reorg_resubmits, cap);
    let error = set
        .reorg_resubmit("c1", &raw, &view.spent_outpoints)
        .expect_err("resubmit cap exceeded");
    assert!(matches!(
        error,
        Error::Journal(JournalError::ReorgResubmitCapExceeded { .. })
    ));
    // Every resubmission was itself a journaled send.
    assert_eq!(set.journal().next_send_sequence() - 1, 1 + u64::from(cap));
}

// ---------------------------------------------------------------------
// H3 (P9): the unknown cap bounds retry SENDS exactly (N+1 total),
// across restart; outcome recording is never capped
// ---------------------------------------------------------------------

#[test]
fn h3_p9_retry_send_counts_are_exact_across_restart() {
    for cap in [0u32, 1, 3] {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut config = standard_test_config();
        config.caps = JournalCaps {
            unknown_retry_sends: cap,
            reorg_resubmits: 1,
        };
        let set = TrafficWalletSet::provision_for_conformance_tests(&config, dir.path())
            .expect("provision with custom caps");
        fund_payer(&set, &PAYER, fixture_outpoint(0x09), 1_000_000);
        signed_view(&set, &payment_request(&set, "c1", 250_000));

        // The original send.
        set.note_broadcast_sent("c1").expect("original send");
        // Cap N admits exactly N retry sends.
        for _ in 0..cap {
            set.note_unknown("c1", "timeout")
                .expect("outcome recording is never capped");
            set.note_broadcast_sent("c1")
                .expect("retry send within the cap");
        }
        // Outcome recording is still not capped...
        set.note_unknown("c1", "timeout")
            .expect("outcome recording is never capped");
        // ...but the next retry send is refused.
        let error = set
            .note_broadcast_sent("c1")
            .expect_err("retry-send cap exceeded");
        assert!(
            matches!(
                &error,
                Error::Journal(JournalError::UnknownRetryCapExceeded {
                    cap: found,
                    ..
                }) if *found == cap
            ),
            "cap {cap}: {error}"
        );
        // Exactly N+1 total sends were journaled.
        let expected_total = 1 + u64::from(cap);
        assert_eq!(set.journal().next_send_sequence() - 1, expected_total);

        // The counts and the cap are durable: after a restart the same
        // command is still at its send budget.
        drop(set);
        let reopened = TrafficWalletSet::reopen_for_conformance_tests(&config, dir.path())
            .expect("reopen with the same caps");
        assert_eq!(
            reopened.journal().next_send_sequence() - 1,
            expected_total,
            "send count must survive the restart"
        );
        let error = reopened
            .note_broadcast_sent("c1")
            .expect_err("retry-send cap persists across restart");
        assert!(matches!(
            error,
            Error::Journal(JournalError::UnknownRetryCapExceeded { .. })
        ));
    }
}

// ---------------------------------------------------------------------
// H5 (P10): coinbase funding is gated on maturity at the observed tip
// ---------------------------------------------------------------------

#[test]
fn h5_p10_coinbase_maturity_gates_availability() {
    let dir = tempfile::tempdir().expect("tempdir");
    let set = conformance_set(dir.path());

    // A bootstrap coinbase output in a canonical block at height 200,
    // and a regular canonical-block output at the same height.
    let coinbase = FundingObservation {
        outpoint: fixture_outpoint(0x0a),
        amount_sats: 500_000_000,
        chain: Chain::Receive,
        index: set.journal().next_receive_index(&PAYER),
        owner_role: PAYER.clone(),
        source: FundingSource::BootstrapCoinbase,
        context: FundingContext::CanonicalBlock {
            block_hash: block_hash(0xa1).to_string(),
            height: 200,
            coinbase: true,
        },
    };
    set.observe_funding(&coinbase).expect("observe coinbase");
    let regular = FundingObservation {
        outpoint: fixture_outpoint(0x0b),
        amount_sats: 100_000,
        chain: Chain::Receive,
        index: set.journal().next_receive_index(&PAYER),
        owner_role: PAYER.clone(),
        source: FundingSource::CanonicalBlock,
        context: FundingContext::CanonicalBlock {
            block_hash: block_hash(0xa2).to_string(),
            height: 200,
            coinbase: false,
        },
    };
    set.observe_funding(&regular).expect("observe regular");

    // FIXED: with no observed tip, coinbase maturity is unprovable, so
    // the coinbase output is NOT available (the regular one is).
    assert_eq!(set.balance_sats(&PAYER), 100_000);
    // A payment only the coinbase could fund is refused.
    let error = set
        .sign_payment(&payment_request(&set, "c1", 200_000))
        .expect_err("immature coinbase must not be selectable");
    assert!(matches!(
        error,
        Error::Builder(spc_lab_txd::error::BuilderError::InsufficientFunds { .. })
    ));

    // Tip at height 299 = 200 + 99 confirmations: still immature.
    set.note_chain_observed(block_hash(0xb1), 299, &[])
        .expect("tip advance to 299");
    assert_eq!(set.balance_sats(&PAYER), 100_000);
    let error = set
        .sign_payment(&payment_request(&set, "c1", 200_000))
        .expect_err("99 confirmations is still immature");
    assert!(matches!(
        error,
        Error::Builder(spc_lab_txd::error::BuilderError::InsufficientFunds { .. })
    ));

    // Tip at height 300 = 200 + 100 confirmations: mature.
    set.note_chain_observed(block_hash(0xb2), 300, &[])
        .expect("tip advance to 300");
    assert_eq!(set.balance_sats(&PAYER), 500_100_000);
    // And the payment signs.
    signed_view(&set, &payment_request(&set, "c1", 200_000));
}

// ---------------------------------------------------------------------
// M7 (P11): a late confirmation after a reorg cycle is journaled
// ---------------------------------------------------------------------

#[test]
fn m7_p11_late_confirmation_after_reorg_cycle_is_journaled() {
    let dir = tempfile::tempdir().expect("tempdir");
    let set = conformance_set(dir.path());
    fund_payer(&set, &PAYER, fixture_outpoint(0x0b), 1_000_000);
    let view = signed_view(&set, &payment_request(&set, "c1", 250_000));
    set.note_broadcast_sent("c1").expect("broadcast");
    set.note_accepted_by_rpc("c1", "node-1").expect("accept");
    set.note_confirmed("c1", block_hash(0x01), 1)
        .expect("confirm");
    set.note_reorged("c1", block_hash(0x01), 1).expect("reorg");
    let raw = decode_hex(&view.raw_hex);
    set.reorg_resubmit("c1", &raw, &view.spent_outpoints)
        .expect("resubmit");
    set.note_accepted_by_rpc("c1", "node-1").expect("accept");

    // FIXED: the late confirmation is journaled and the guarded finish
    // is a no-op for the already-finished command - no error after the
    // record has committed.
    set.note_confirmed("c1", block_hash(0x02), 2)
        .expect("late confirmation must not error");
    assert_eq!(set.journal().tx_state(&view.txid), Some(TxState::Confirmed));
    assert_eq!(
        set.journal().command_phase("c1"),
        Some(CommandPhase::Finished)
    );
    // The raw journal API stays non-idempotent; the guard lives in the
    // wrapper (M7).
    assert!(set.journal().finish_command("c1", "again").is_err());

    // A late conflict note after another reorg of the already-finished
    // command journals the same way (record always, finish guarded).
    // (Reorged, Conflicted) is a legal edge; AcceptedByRpc is not - a
    // conflict of an RPC-accepted tx is discovered through mempool
    // observation or a reorg, which is exactly this path.
    set.note_reorged("c1", block_hash(0x02), 2)
        .expect("reorg again");
    set.note_conflicted("c1", "replaced-by-higher-fee")
        .expect("late conflict is journaled without a second finish");
    assert_eq!(
        set.journal().tx_state(&view.txid),
        Some(TxState::Conflicted)
    );
}

// ---------------------------------------------------------------------
// M2 (P12): the change chain is unbounded by the script lookahead
// ---------------------------------------------------------------------

#[test]
fn m2_p12_small_lookahead_does_not_brick_the_role() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut config = standard_test_config();
    config.lookahead = 2;
    let set = TrafficWalletSet::provision_for_conformance_tests(&config, dir.path())
        .expect("provision with lookahead 2");
    fund_payer(&set, &PAYER, fixture_outpoint(0x0c), 100_000_000);

    // Five consecutive payments chain the change outputs through
    // change indexes 0..=4 - indexes 2, 3 and 4 are at or beyond the
    // lookahead of 2. FIXED: spending resolves keys by direct
    // derivation at the recorded index, so the role keeps paying.
    for i in 0..5u8 {
        let view = sign_broadcast_accept_confirm(
            &set,
            &format!("c{i}"),
            1_000_000,
            0x40 + i,
            u64::from(i) + 1,
        );
        observe_own_change(&set, &view);
        assert!(
            set.balance_sats(&PAYER) > 90_000_000,
            "payment {i}: the change chain must keep funding the role"
        );
    }

    // And a sixth payment still signs.
    signed_view(&set, &payment_request(&set, "c5", 1_000_000));
}

// ---------------------------------------------------------------------
// M3 (P13): multi-actor allocations are validated before provisioning
// ---------------------------------------------------------------------

#[test]
fn m3_p13_allocation_validation_and_disjoint_provisioning() {
    // Empty allocation refused.
    assert!(ProvisionConfig::validate_allocation(&[]).is_err());

    // Two bootstrap-funding hosts refused.
    let error =
        ProvisionConfig::validate_allocation(&[standard_test_config(), standard_test_config()])
            .expect_err("two bootstrap hosts must be refused");
    assert!(matches!(
        error,
        spc_lab_txd::error::WalletError::AllocationConflict(_)
    ));

    // A disjoint two-actor allocation of the standard population.
    let mut host = standard_test_config();
    host.network_wallets = 0..=3;
    host.user_payment_wallets = 0..=1;
    let mut other = standard_test_config();
    other.network_wallets = 4..=7;
    other.user_payment_wallets = 2..=3;
    other.hosts_bootstrap_funding = false;
    other.driver_id = "t1a-conformance-b".to_string();
    ProvisionConfig::validate_allocation(&[host.clone(), other.clone()])
        .expect("disjoint allocation is valid");

    // Overlapping network-wallet ranges refused.
    let mut overlapping = other.clone();
    overlapping.network_wallets = 3..=7; // overlaps host's 0..=3
    assert!(
        ProvisionConfig::validate_allocation(&[host.clone(), overlapping]).is_err(),
        "overlapping network-wallet ranges must be refused"
    );

    // A second bootstrap host in an otherwise disjoint allocation.
    let mut second_host = other.clone();
    second_host.hosts_bootstrap_funding = true;
    assert!(ProvisionConfig::validate_allocation(&[host.clone(), second_host]).is_err());

    // Divergent run identities refused.
    let mut wrong_run = other.clone();
    wrong_run.run_id = "a-different-run".to_string();
    assert!(
        ProvisionConfig::validate_allocation(&[host.clone(), wrong_run]).is_err(),
        "divergent run ids must be refused"
    );

    // Invalid caps refused.
    let mut bad_caps = other.clone();
    bad_caps.caps.unknown_retry_sends = 9;
    assert!(ProvisionConfig::validate_allocation(&[host.clone(), bad_caps]).is_err());

    // An empty role set (no ranges, no bootstrap host) refused. The
    // constructor form expresses an intentionally empty range (clippy
    // denies the reversed `1..=0` literal).
    let mut empty = other.clone();
    empty.network_wallets = RangeInclusive::new(1, 0);
    empty.user_payment_wallets = RangeInclusive::new(1, 0);
    empty.hosts_bootstrap_funding = false;
    assert!(
        ProvisionConfig::validate_allocation(&[host.clone(), empty]).is_err(),
        "a config owning no role must be refused"
    );

    // The validated allocation provisions into two disjoint sets.
    let dir_host = tempfile::tempdir().expect("tempdir");
    let dir_other = tempfile::tempdir().expect("tempdir");
    let set_host = TrafficWalletSet::provision_for_conformance_tests(&host, dir_host.path())
        .expect("provision host set");
    let set_other = TrafficWalletSet::provision_for_conformance_tests(&other, dir_other.path())
        .expect("provision other set");

    // The role sets are disjoint: no role is ever derived by two sets
    // (a double signer is structurally impossible).
    let roles_host: HashSet<WalletRole> = set_host.roles().into_iter().collect();
    let roles_other: HashSet<WalletRole> = set_other.roles().into_iter().collect();
    assert!(roles_host.is_disjoint(&roles_other));
    assert!(roles_host.contains(&WalletRole::BootstrapFunding));
    assert!(!roles_other.contains(&WalletRole::BootstrapFunding));

    // Different loci derive different scripts (the collision the probe
    // exhibited was two standard configs deriving the same loci).
    let host_script = set_host
        .next_receive_address(&WalletRole::NetworkWallet { index: 0 })
        .expect("host script")
        .script_pubkey;
    let other_script = set_other
        .next_receive_address(&WalletRole::NetworkWallet { index: 4 })
        .expect("other script")
        .script_pubkey;
    assert_ne!(host_script, other_script);

    // Both journals bind the same run identity (one derivation domain).
    assert_eq!(set_host.run_id(), set_other.run_id());
    assert_eq!(
        set_host.journal().header().expect("header").run_id,
        host.run_id
    );
}

// ---------------------------------------------------------------------
// M4: injected write faults roll back provably or poison the handle
// ---------------------------------------------------------------------

#[test]
fn m4_write_faults_roll_back_or_poison() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = standard_test_config();
    let set =
        TrafficWalletSet::provision_for_conformance_tests(&config, dir.path()).expect("provision");
    fund_payer(&set, &PAYER, fixture_outpoint(0xd0), 1_000_000);
    let seq_before = set.journal().last_seq();

    // PartialWrite: the append fails, the partial bytes are rolled back,
    // nothing commits.
    set.journal()
        ._for_tests_set_write_fault(WriteFault::PartialWrite)
        .expect("arm partial-write fault");
    let error = set
        .observe_funding(&fixture_observation(&set, 0xd1, 500_000))
        .expect_err("partial write must fail");
    assert!(matches!(
        error,
        Error::Journal(JournalError::Io(ref reason)) if reason.contains("rolled back")
    ));
    assert_eq!(set.journal().last_seq(), seq_before);
    // The fault is consumed and the handle is healthy: the same
    // observation now commits.
    set.observe_funding(&fixture_observation(&set, 0xd1, 500_000))
        .expect("append succeeds after the rollback");
    assert_eq!(set.journal().last_seq(), seq_before + 1);

    // SyncFailure: the line never became durable; it is rolled back so
    // no later flush can resurrect it.
    set.journal()
        ._for_tests_set_write_fault(WriteFault::SyncFailure)
        .expect("arm sync-failure fault");
    let error = set
        .observe_funding(&fixture_observation(&set, 0xd2, 500_000))
        .expect_err("sync failure must fail");
    assert!(matches!(
        error,
        Error::Journal(JournalError::Io(ref reason)) if reason.contains("not made durable")
    ));
    assert_eq!(set.journal().last_seq(), seq_before + 1);
    set.observe_funding(&fixture_observation(&set, 0xd2, 500_000))
        .expect("append succeeds after the sync-failure rollback");
    assert_eq!(set.journal().last_seq(), seq_before + 2);

    // RollbackFailure: the rollback cannot be proven, so the handle is
    // poisoned and every further mutation fails closed.
    set.journal()
        ._for_tests_set_write_fault(WriteFault::RollbackFailure)
        .expect("arm rollback-failure fault");
    let error = set
        .observe_funding(&fixture_observation(&set, 0xd3, 500_000))
        .expect_err("poisoned handle must fail");
    assert!(matches!(error, Error::Journal(JournalError::Poisoned)));
    let error = set
        .observe_funding(&fixture_observation(&set, 0xd4, 500_000))
        .expect_err("poisoned handle refuses every mutation");
    assert!(matches!(error, Error::Journal(JournalError::Poisoned)));
    let error = set
        .journal()
        ._for_tests_set_write_fault(WriteFault::PartialWrite)
        .expect_err("even fault arming is refused");
    assert!(matches!(error, JournalError::Poisoned));
    // The durable world is exactly the two committed observations.
    assert_eq!(set.balance_sats(&PAYER), 2_000_000);

    // Replay: the poisoned handle left a torn tail, which the reopen
    // truncates; the reconstructed state is exactly the pre-fault
    // state, and the reopened handle is healthy. The reopen appends one
    // SessionStarted record (R2), hence +3.
    drop(set);
    let reopened = reopen_conformance_set(dir.path());
    assert_eq!(reopened.balance_sats(&PAYER), 2_000_000);
    assert_eq!(reopened.journal().last_seq(), seq_before + 3);
    reopened
        .observe_funding(&fixture_observation(&reopened, 0xd4, 123_000))
        .expect("reopened handle is healthy");
    assert_eq!(reopened.balance_sats(&PAYER), 2_123_000);
}

// ---------------------------------------------------------------------
// R1 (round 2): a REAL write fault takes the shared recovery path
// ---------------------------------------------------------------------

/// Child mode of `r1_real_write_fault_rolls_back_or_poisons`: reopens
/// the journal, ignores SIGXFSZ, sets `RLIMIT_FSIZE` to the current
/// durable length plus 40 bytes and attempts one append through the
/// REAL production write path (no injected fault). The probe's exact
/// counterexample shape: the append used to fail with `Io` and leave a
/// 40-byte fragment on disk, the next append landed behind it and the
/// reopen failed `Integrity`. Never run directly.
#[test]
#[ignore = "child mode: spawned by r1_real_write_fault_rolls_back_or_poisons"]
fn r1_real_write_fault_child() {
    let run_dir = std::env::var_os("SPC_TXD_FABLE_R1_RUN_DIR")
        .expect("SPC_TXD_FABLE_R1_RUN_DIR is set by the parent test");
    let result = std::env::var_os("SPC_TXD_FABLE_R1_RESULT")
        .expect("SPC_TXD_FABLE_R1_RESULT is set by the parent test");
    let allocation_digest = std::env::var("SPC_TXD_FABLE_R1_ALLOCATION_DIGEST")
        .expect("SPC_TXD_FABLE_R1_ALLOCATION_DIGEST is set by the parent test");
    let driver_id = std::env::var("SPC_TXD_FABLE_R1_DRIVER_ID")
        .expect("SPC_TXD_FABLE_R1_DRIVER_ID is set by the parent test");
    let run_dir = Path::new(&run_dir);

    let journal = TrafficJournal::reopen_for_conformance_tests(
        run_dir,
        &JournalCaps::default(),
        &allocation_digest,
        &driver_id,
    )
    .expect("child reopens the journal");
    let path = TrafficJournal::path_in(run_dir);
    let len_before = std::fs::metadata(&path).expect("stat journal").len();
    let seq_before = journal.last_seq();

    // A REAL write fault, in this child process only: the file-size
    // limit stops the append 40 bytes in (SIGXFSZ ignored, so write_all
    // returns EFBIG instead of killing the process).
    unsafe {
        libc::signal(libc::SIGXFSZ, libc::SIG_IGN);
        let limit = libc::rlimit {
            rlim_cur: len_before + 40,
            rlim_max: libc::RLIM_INFINITY,
        };
        assert_eq!(libc::setrlimit(libc::RLIMIT_FSIZE, &limit), 0);
    }
    let error = journal
        .record_chain_observed(block_hash(0x71), 71, &[])
        .expect_err("the real write fault must fail the append");
    assert!(
        matches!(error, JournalError::Io(ref reason) if reason.contains("rolled back")),
        "the real-path Io must state the rollback: {error}"
    );
    // FIXED: rolled back durably to len_before - no fragment survives.
    assert_eq!(
        std::fs::metadata(&path).expect("stat journal").len(),
        len_before,
        "the 40-byte fragment must be rolled back"
    );
    assert_eq!(journal.last_seq(), seq_before, "in-memory state unchanged");
    // Lift the limit: the next append lands cleanly (nothing was left
    // behind for it to land behind).
    unsafe {
        let limit = libc::rlimit {
            rlim_cur: libc::RLIM_INFINITY,
            rlim_max: libc::RLIM_INFINITY,
        };
        assert_eq!(libc::setrlimit(libc::RLIMIT_FSIZE, &limit), 0);
    }
    journal
        .record_chain_observed(block_hash(0x72), 72, &[])
        .expect("append after the rollback is clean");
    assert_eq!(journal.last_seq(), seq_before + 1);
    drop(journal);

    // Reopen: consistent, and the observation (the ack that could have
    // been lost to the fault) is durably present exactly once - the
    // reopen's own session record makes +2.
    let reopened = TrafficJournal::reopen_for_conformance_tests(
        run_dir,
        &JournalCaps::default(),
        &allocation_digest,
        &driver_id,
    )
    .expect("reopen after the real write fault");
    assert_eq!(reopened.last_seq(), seq_before + 2);
    std::fs::write(&result, b"ok").expect("child reports success");
}

#[test]
fn r1_real_write_fault_rolls_back_or_poisons() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = standard_test_config();
    {
        let set = TrafficWalletSet::provision_for_conformance_tests(&config, dir.path())
            .expect("provision");
        fund_payer(&set, &PAYER, fixture_outpoint(0x12), 1_000_000);
    }
    let allocation = ProvisionConfig::validate_allocation(std::slice::from_ref(&config))
        .expect("the standard single-config allocation is valid");
    let result = dir.path().join("r1-result");
    let mut child = std::process::Command::new(std::env::current_exe().expect("test binary"))
        .args([
            "--exact",
            "r1_real_write_fault_child",
            "--ignored",
            "--nocapture",
        ])
        .env("SPC_TXD_FABLE_R1_RUN_DIR", dir.path())
        .env("SPC_TXD_FABLE_R1_RESULT", &result)
        .env("SPC_TXD_FABLE_R1_ALLOCATION_DIGEST", allocation.digest())
        .env("SPC_TXD_FABLE_R1_DRIVER_ID", &config.driver_id)
        .spawn()
        .expect("spawn the child-mode real-fault prober");
    let mut done = false;
    for _ in 0..600 {
        if result.exists() {
            done = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(done, "the real-fault child did not report in time");
    let status = child.wait().expect("reap the real-fault child");
    assert_eq!(
        std::fs::read_to_string(&result).expect("child result"),
        "ok",
        "the child's own assertions all held"
    );
    assert!(status.success(), "child exit status: {status}");
}

// ---------------------------------------------------------------------
// R2 (round 2): a restart while BroadcastSent is journaled uncertainty
// ---------------------------------------------------------------------

#[test]
fn r2_restart_in_broadcast_sent_then_rejected_degrades() {
    let dir = tempfile::tempdir().expect("tempdir");
    let set = conformance_set(dir.path());
    fund_payer(&set, &PAYER, fixture_outpoint(0x13), 1_000_000);
    let view = signed_view(&set, &payment_request(&set, "c1", 250_000));
    set.note_broadcast_sent("c1").expect("broadcast");
    // Crash before any outcome record: the fate of this send is not
    // accounted for by anything in the journal.
    drop(set);

    // FIXED: the reopen appends a durable SessionStarted record under
    // the exclusive lock; the in-flight send becomes unknown (the
    // restart itself is uncertainty), with ever_unknown set durably.
    let set = reopen_conformance_set(dir.path());
    assert_eq!(
        set.journal().tx_state(&view.txid),
        Some(TxState::Unknown),
        "the restart moved the in-flight send to unknown"
    );
    set.note_rejected("c1", "txn-already-in-mempool")
        .expect("the rejection itself is recordable");
    // A rejection after the restart degrades to possibly-spent: the
    // first send may have been accepted.
    assert_eq!(set.balance_sats(&PAYER), 0);
    let error = set
        .sign_payment(&payment_request(&set, "c2", 250_000))
        .expect_err("possibly-spent inputs are not reselectable");
    assert!(matches!(
        error,
        Error::Builder(spc_lab_txd::error::BuilderError::InsufficientFunds { .. })
    ));
    // And the whole decision replays identically.
    drop(set);
    let set = reopen_conformance_set(dir.path());
    assert_eq!(set.journal().tx_state(&view.txid), Some(TxState::Rejected));
    assert_eq!(set.balance_sats(&PAYER), 0);
}

#[test]
fn r2_crash_loop_reopens_do_not_inflate_attempts() {
    let dir = tempfile::tempdir().expect("tempdir");
    let set = conformance_set(dir.path());
    fund_payer(&set, &PAYER, fixture_outpoint(0x14), 1_000_000);
    let view = signed_view(&set, &payment_request(&set, "c1", 250_000));
    set.note_broadcast_sent("c1").expect("broadcast");
    drop(set);
    // Crash-loop: several reopens in a row, never recording an outcome.
    for _ in 0..3 {
        drop(reopen_conformance_set(dir.path()));
    }
    let set = reopen_conformance_set(dir.path());
    assert_eq!(set.journal().tx_state(&view.txid), Some(TxState::Unknown));
    // The session records did not inflate the attempt counter: the
    // next unknown outcome is attempt 2 (one restart-induced unknown),
    // not 2 + the number of sessions.
    assert_eq!(
        set.note_unknown("c1", "timeout").expect("record unknown"),
        2
    );
}

#[test]
fn r2_restart_does_not_reset_or_consume_the_retry_budget() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut config = standard_test_config();
    config.caps = JournalCaps {
        unknown_retry_sends: 1,
        reorg_resubmits: 3,
    };
    let set = TrafficWalletSet::provision_for_conformance_tests(&config, dir.path())
        .expect("provision with retry-send cap 1");
    fund_payer(&set, &PAYER, fixture_outpoint(0x15), 1_000_000);
    signed_view(&set, &payment_request(&set, "c1", 250_000));
    set.note_broadcast_sent("c1").expect("original send");
    drop(set);
    let set = TrafficWalletSet::reopen_for_conformance_tests(&config, dir.path())
        .expect("reopen (the session record is appended)");
    // The restart moved the send to unknown but consumed NO retry send
    // (a process restart is not a retry send): the single admitted
    // retry is still available...
    set.note_broadcast_sent("c1")
        .expect("the retry send is admitted (the restart was not one)");
    // ...and once used, the cap binds across the restart: recording the
    // outcome is never capped, the next retry send is refused.
    set.note_unknown("c1", "timeout")
        .expect("outcome recording is never capped");
    let error = set
        .note_broadcast_sent("c1")
        .expect_err("retry cap 1 is exhausted and the restart did not reset it");
    assert!(matches!(
        error,
        Error::Journal(JournalError::UnknownRetryCapExceeded { .. })
    ));
}

// ---------------------------------------------------------------------
// R12 (round 2): canonical re-inclusion of a reorged transaction
// ---------------------------------------------------------------------

#[test]
fn r12_reorged_reinclusion_is_canonical_evidence() {
    let dir = tempfile::tempdir().expect("tempdir");
    let set = conformance_set(dir.path());
    fund_payer(&set, &PAYER, fixture_outpoint(0x16), 2_000_000);
    // Parent confirmed in block 0x10 (destination output funds DEST,
    // change returns to the payer); child confirmed in block 0x11
    // spending the parent's change.
    let dest_index = set.journal().next_receive_index(&DEST);
    let parent = sign_broadcast_accept_confirm(&set, "p", 600_000, 0x10, 10);
    set.observe_funding(&FundingObservation {
        outpoint: OutPoint::new(parent.txid, 0),
        amount_sats: parent.destinations[0].amount_sats,
        chain: Chain::Receive,
        index: dest_index,
        owner_role: DEST.clone(),
        source: FundingSource::OwnTransaction,
        context: FundingContext::OwnTransaction,
    })
    .expect("observe the parent's destination output");
    observe_own_change(&set, &parent);
    let child = sign_broadcast_accept_confirm(&set, "c1", 100_000, 0x11, 11);
    assert_eq!(child.spent_outpoints, vec![OutPoint::new(parent.txid, 1)]);
    assert_eq!(set.balance_sats(&DEST), 600_000);

    // Displace the parent: the child cascades to invalidated.
    set.note_reorged("p", block_hash(0x10), 10).expect("reorg");
    assert_eq!(set.journal().tx_state(&parent.txid), Some(TxState::Reorged));
    assert_eq!(
        set.journal().tx_state(&child.txid),
        Some(TxState::Invalidated)
    );
    assert_eq!(set.balance_sats(&DEST), 0);

    // FIXED (R12): the new chain re-mined the parent from the mempool
    // WITHOUT any wallet action; the canonical re-inclusion is
    // recordable from Reorged (a confirmation is block evidence, not a
    // send; only BroadcastSent stays ReorgResubmit-only).
    set.journal()
        .record_confirmed("p", block_hash(0x20), 20)
        .expect("canonical re-inclusion is recordable");
    assert_eq!(
        set.journal().tx_state(&parent.txid),
        Some(TxState::Confirmed)
    );
    // The restore cascade recovers one generation at a time: the child
    // is Reorged (the conservative resubmit-or-reconfirm path), and
    // DEST's funding is restored (its creator is confirmed again).
    assert_eq!(set.journal().tx_state(&child.txid), Some(TxState::Reorged));
    assert_eq!(set.balance_sats(&DEST), 600_000);
    // Re-including the child completes the recovery.
    set.journal()
        .record_confirmed("c1", block_hash(0x21), 21)
        .expect("child re-inclusion");
    assert_eq!(
        set.journal().tx_state(&child.txid),
        Some(TxState::Confirmed)
    );
    // Replay reconstructs the same recovered world.
    drop(set);
    let set = reopen_conformance_set(dir.path());
    assert_eq!(
        set.journal().tx_state(&parent.txid),
        Some(TxState::Confirmed)
    );
    assert_eq!(
        set.journal().tx_state(&child.txid),
        Some(TxState::Confirmed)
    );
    assert_eq!(set.balance_sats(&DEST), 600_000);
}

#[test]
fn r12_reinclusion_naming_the_displaced_block_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let set = conformance_set(dir.path());
    fund_payer(&set, &PAYER, fixture_outpoint(0x17), 1_000_000);
    let view = signed_view(&set, &payment_request(&set, "c1", 250_000));
    set.note_broadcast_sent("c1").expect("send");
    set.note_accepted_by_rpc("c1", "node-1").expect("accept");
    set.note_confirmed("c1", block_hash(0x60), 60)
        .expect("confirm");
    set.note_reorged("c1", block_hash(0x60), 60).expect("reorg");
    // FIXED (fail closed): the re-inclusion must not name the displaced
    // block itself - the same block cannot be both the displaced and
    // the re-including evidence.
    let error = set
        .journal()
        .record_confirmed("c1", block_hash(0x60), 60)
        .expect_err("naming the displaced block must fail closed");
    assert!(
        matches!(error, JournalError::IllegalTransition { .. }),
        "{error}"
    );
    // A different block is the canonical re-inclusion edge.
    set.journal()
        .record_confirmed("c1", block_hash(0x61), 61)
        .expect("re-inclusion in the new chain");
    assert_eq!(set.journal().tx_state(&view.txid), Some(TxState::Confirmed));
}

// ---------------------------------------------------------------------
// S3 (round 3): the displaced-block refusal is stale-evidence only -
// a reorg BACK to the original block is canonical again
// ---------------------------------------------------------------------

#[test]
fn s3_reorg_back_to_same_block_is_canonical_after_a_fresh_chain_observation() {
    let dir = tempfile::tempdir().expect("tempdir");
    let set = conformance_set(dir.path());
    fund_payer(&set, &PAYER, fixture_outpoint(0x19), 2_000_000);
    // The original chain: parent in B(0x10)@10, child in B(0x11)@11
    // spending the parent's change, grandchild in B(0x13)@12 spending
    // the child's change.
    let parent = sign_broadcast_accept_confirm(&set, "p", 600_000, 0x10, 10);
    observe_own_change(&set, &parent);
    let child = sign_broadcast_accept_confirm(&set, "c", 100_000, 0x11, 11);
    observe_own_change(&set, &child);
    let grandchild = sign_broadcast_accept_confirm(&set, "g", 50_000, 0x13, 12);
    assert_eq!(
        grandchild.spent_outpoints,
        vec![OutPoint::new(child.txid, 1)]
    );
    observe_own_change(&set, &grandchild);
    let payer_balance_before = set.balance_sats(&PAYER);
    assert!(payer_balance_before > 0);

    // Reorg away: a competing chain to tip 0x20@11 displaces B(0x10),
    // B(0x11) and B(0x13): the parent is Reorged, the child and the
    // grandchild cascade to Invalidated.
    set.note_chain_observed(
        block_hash(0x20),
        11,
        &[block_hash(0x10), block_hash(0x11), block_hash(0x13)],
    )
    .expect("reorg away");
    assert_eq!(set.journal().tx_state(&parent.txid), Some(TxState::Reorged));
    assert_eq!(
        set.journal().tx_state(&child.txid),
        Some(TxState::Invalidated)
    );
    assert_eq!(
        set.journal().tx_state(&grandchild.txid),
        Some(TxState::Invalidated)
    );
    assert_eq!(set.balance_sats(&PAYER), 0);

    // STALE evidence: while the displacement is the LATEST chain
    // observation, the same-block re-inclusion is still refused (the
    // r12 rule; no ChainObserved newer than the displacement exists).
    let error = set
        .journal()
        .record_confirmed("p", block_hash(0x10), 10)
        .expect_err("stale same-block re-inclusion must fail closed");
    assert!(
        matches!(error, JournalError::IllegalTransition { .. }),
        "{error}"
    );

    // Reorg back: the original branch extends to 0x12@12, displacing
    // the competing tip; B(0x10)/B(0x11)/B(0x13) are canonical again.
    set.note_chain_observed(block_hash(0x12), 12, &[block_hash(0x20)])
        .expect("reorg back");

    // FIXED (S3): the fresh chain observation makes the same-block
    // confirmation admissible as canonical evidence - the displaced
    // identity is invalid only while the displacement is the latest
    // chain evidence.
    set.journal()
        .record_confirmed("p", block_hash(0x10), 10)
        .expect("the reorg-back re-inclusion of the parent is canonical");
    assert_eq!(
        set.journal().tx_state(&parent.txid),
        Some(TxState::Confirmed)
    );
    // The restore cascade recovers ONE generation: the child is
    // Reorged - with its ORIGINAL displacement identity preserved, not
    // overwritten by the parent's restoring confirmation - and the
    // grandchild stays Invalidated.
    assert_eq!(set.journal().tx_state(&child.txid), Some(TxState::Reorged));
    assert_eq!(
        set.journal().tx_state(&grandchild.txid),
        Some(TxState::Invalidated)
    );

    // The child re-includes in its own original block B(0x11): its
    // preserved displacement seq is OLDER than the reorg-back
    // observation, so this is admissible (a parent-reconfirmation
    // stamp would have wrongly refused it here).
    set.journal()
        .record_confirmed("c", block_hash(0x11), 11)
        .expect("the child re-includes in its original block");
    assert_eq!(
        set.journal().tx_state(&child.txid),
        Some(TxState::Confirmed)
    );
    assert_eq!(
        set.journal().tx_state(&grandchild.txid),
        Some(TxState::Reorged)
    );

    // The grandchild completes the recovery in its own original block.
    set.journal()
        .record_confirmed("g", block_hash(0x13), 12)
        .expect("the grandchild re-includes in its original block");
    assert_eq!(
        set.journal().tx_state(&grandchild.txid),
        Some(TxState::Confirmed)
    );
    assert_eq!(set.balance_sats(&PAYER), payer_balance_before);

    // Replay reconstructs the identical healed world (the displacement
    // seqs and the last chain-observed seq derive from the record
    // stream alone - no timestamps, no external state).
    drop(set);
    let set = reopen_conformance_set(dir.path());
    assert_eq!(
        set.journal().tx_state(&parent.txid),
        Some(TxState::Confirmed)
    );
    assert_eq!(
        set.journal().tx_state(&child.txid),
        Some(TxState::Confirmed)
    );
    assert_eq!(
        set.journal().tx_state(&grandchild.txid),
        Some(TxState::Confirmed)
    );
    assert_eq!(set.balance_sats(&PAYER), payer_balance_before);
}

#[test]
fn s4_note_rejected_records_genuine_rejection_and_guards_only_the_finish() {
    let dir = tempfile::tempdir().expect("tempdir");
    let set = conformance_set(dir.path());
    fund_payer(&set, &PAYER, fixture_outpoint(0x18), 1_000_000);
    let view = signed_view(&set, &payment_request(&set, "c1", 250_000));
    set.note_broadcast_sent("c1").expect("send");
    set.note_accepted_by_rpc("c1", "node-1").expect("accept");
    set.note_confirmed("c1", block_hash(0x62), 62)
        .expect("confirm (finishes the command)");
    set.note_reorged("c1", block_hash(0x62), 62).expect("reorg");
    // The exact probe shape: after the reorg resubmission the node
    // GENUINELY rejects the bytes - the inputs were spent in the
    // reorged-out chain ("bad-txns-inputs-missingorspent"). The command
    // is already finished; only a rejection of the funding claim would
    // be a misclassification, and classifying node answers is T1b's
    // concern, never the core's (it does not parse the code string).
    set.reorg_resubmit("c1", &decode_hex(&view.raw_hex), &view.spent_outpoints)
        .expect("resubmit");
    let seq_before = set.journal().last_seq();
    // FIXED (S4): the wrapper always records the rejection; only the
    // finish is guarded. The genuine post-resubmit rejection of a
    // finished command is journaled without error.
    set.note_rejected("c1", "bad-txns-inputs-missingorspent")
        .expect("the genuine rejection is recorded through the wrapper");
    // Exactly ONE record advanced (the Rejected record - no second
    // finish was written for the already-finished command).
    assert_eq!(set.journal().last_seq(), seq_before + 1);
    assert_eq!(set.journal().tx_state(&view.txid), Some(TxState::Rejected));
    assert_eq!(
        set.journal().command_phase("c1"),
        Some(CommandPhase::Finished)
    );
    // The command keeps its original terminal outcome; the inputs
    // degrade (reorg-resubmitted payments never release them, H1).
    assert_eq!(set.balance_sats(&PAYER), 0);
    assert_eq!(set.journal().summary().possibly_spent_outpoints, 1);
    // Replay reconstructs the same world.
    drop(set);
    let set = reopen_conformance_set(dir.path());
    assert_eq!(set.journal().tx_state(&view.txid), Some(TxState::Rejected));
    assert_eq!(
        set.journal().command_phase("c1"),
        Some(CommandPhase::Finished)
    );
    assert_eq!(set.balance_sats(&PAYER), 0);
}

// ---------------------------------------------------------------------
// R6 (round 2): production provisioning is type-state enforced
// ---------------------------------------------------------------------

#[test]
fn r6_production_provision_requires_a_validated_allocation() {
    // The probe's overlapping two-actor allocation (overlapping network
    // ranges AND two bootstrap hosts) cannot produce a token, so it can
    // no longer provision at all through the production path (the
    // probe's `TrafficWalletSet::provision(&config, dir)` call does not
    // even compile against the token API).
    let mut a = standard_test_config();
    a.driver_id = "t1a-r6-a".to_string();
    let mut b = standard_test_config();
    b.driver_id = "t1a-r6-b".to_string();
    b.network_wallets = RangeInclusive::new(4, 11); // overlaps 4..=7
    assert!(
        ProvisionConfig::validate_allocation(&[a.clone(), b.clone()]).is_err(),
        "the overlapping allocation cannot produce a token"
    );

    // A valid two-actor allocation provisions two disjoint sets (the
    // token selects the actor by driver id).
    let mut host = standard_test_config();
    host.network_wallets = 0..=3;
    host.user_payment_wallets = 0..=1;
    let mut other = standard_test_config();
    other.network_wallets = 4..=7;
    other.user_payment_wallets = 2..=3;
    other.hosts_bootstrap_funding = false;
    other.driver_id = "t1a-r6-other".to_string();
    let allocation = ProvisionConfig::validate_allocation(&[host.clone(), other.clone()])
        .expect("the disjoint allocation is valid");

    let dir_host = tempfile::tempdir().expect("tempdir");
    let dir_other = tempfile::tempdir().expect("tempdir");
    let set_host = TrafficWalletSet::provision(&allocation, &host.driver_id, dir_host.path())
        .expect("provision the host actor");
    let set_other = TrafficWalletSet::provision(&allocation, &other.driver_id, dir_other.path())
        .expect("provision the other actor");
    // Valid two-actor ranges are distinct: no role is derived by both.
    let roles_host: HashSet<WalletRole> = set_host.roles().into_iter().collect();
    let roles_other: HashSet<WalletRole> = set_other.roles().into_iter().collect();
    assert!(roles_host.is_disjoint(&roles_other));
    assert!(roles_host.contains(&WalletRole::BootstrapFunding));
    assert!(!roles_other.contains(&WalletRole::BootstrapFunding));

    // A driver id outside the allocation cannot provision at all (no
    // artifact is created).
    let dir_stranger = tempfile::tempdir().expect("tempdir");
    let error =
        TrafficWalletSet::provision(&allocation, "not-in-the-allocation", dir_stranger.path())
            .expect_err("an unknown driver id cannot provision");
    assert!(matches!(
        error,
        Error::Wallet(spc_lab_txd::error::WalletError::AllocationConflict(_))
    ));
    assert!(!TrafficJournal::path_in(dir_stranger.path()).exists());

    // Reopen binds the allocation digest: a drifted allocation (here a
    // different host lookahead - same roles, so the key file still
    // verifies) fails closed BEFORE any mutation.
    drop(set_host);
    let journal_path = TrafficJournal::path_in(dir_host.path());
    let len_before = std::fs::metadata(&journal_path)
        .expect("the host journal exists")
        .len();
    let mut drifted = host.clone();
    drifted.lookahead += 1;
    let drifted_allocation = ProvisionConfig::validate_allocation(&[drifted, other.clone()])
        .expect("the drifted allocation is itself valid");
    let error = TrafficWalletSet::reopen(&drifted_allocation, &host.driver_id, dir_host.path())
        .expect_err("a drifted allocation must fail closed");
    assert!(matches!(
        error,
        Error::Journal(JournalError::AllocationMismatch { .. })
    ));
    // Before mutation: no session record was appended.
    assert_eq!(
        std::fs::metadata(&journal_path)
            .expect("stat journal")
            .len(),
        len_before
    );

    // The journal-level driver binding fails closed the same way
    // (defense in depth for direct journal consumers).
    let error = TrafficJournal::reopen_for_conformance_tests(
        dir_host.path(),
        &host.caps,
        allocation.digest(),
        "a-wrong-driver",
    )
    .expect_err("a wrong driver id must fail closed");
    assert!(matches!(error, JournalError::DriverIdMismatch { .. }));
    assert_eq!(
        std::fs::metadata(&journal_path)
            .expect("stat journal")
            .len(),
        len_before
    );

    // The correct allocation and driver still reopen.
    let set = TrafficWalletSet::reopen(&allocation, &host.driver_id, dir_host.path())
        .expect("the matching allocation reopens");
    assert_eq!(set.driver_id(), host.driver_id);
}
