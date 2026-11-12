//! Integration suite 3: the journaled transaction lifecycle - the walk
//! to confirmation, explicit provenance-carrying destination crediting,
//! reorg displacement with preserved possibly-spent claims, bounded
//! same-bytes resubmission, and terminal conflicts.

mod common;

use bitcoin::hashes::Hash as _;
use bitcoin::{BlockHash, OutPoint};

use common::{
    conformance_set, decode_hex, fixture_outpoint, fund_payer, golden_request, GOLDEN_DEST_OWNER,
    GOLDEN_DEST_SATS, GOLDEN_FUNDING_SATS, GOLDEN_PAYER,
};
use spc_lab_txd::journal::{FundingContext, FundingObservation, FundingSource, TxState};
use spc_lab_txd::keys::Chain;
use spc_lab_txd::wallet::{PaymentOutcome, SignedPaymentView, TrafficWalletSet};

fn block_hash(byte: u8) -> BlockHash {
    BlockHash::from_byte_array([byte; 32])
}

/// Sign and broadcast one payment; returns the published view.
fn signed_and_broadcast(
    set: &TrafficWalletSet,
    command_id: &str,
    outpoint_byte: u8,
) -> SignedPaymentView {
    let (outpoint, _script) = fund_payer(
        set,
        &GOLDEN_PAYER,
        fixture_outpoint(outpoint_byte),
        GOLDEN_FUNDING_SATS,
    );
    assert!(!set.available_outpoints(&GOLDEN_PAYER).is_empty());
    let request = golden_request(set, command_id);
    let view = match set.sign_payment(&request).expect("sign") {
        PaymentOutcome::Signed(view) => view,
        PaymentOutcome::ReplayedStored(_) => panic!("fresh command freshly signs"),
    };
    assert_eq!(view.spent_outpoints.len(), 1);
    assert_eq!(view.spent_outpoints[0], outpoint);
    set.note_broadcast_sent(command_id)
        .expect("broadcast sent is journaled");
    view
}

#[test]
fn lifecycle_walks_to_confirmation_and_credits_explicitly() {
    let dir = tempfile::tempdir().expect("tempdir");
    let set = conformance_set(dir.path());
    let view = signed_and_broadcast(&set, "life-1", 0x61);
    let txid = view.txid;

    set.note_accepted_by_rpc("life-1", "node-1")
        .expect("rpc accept");
    set.note_observed_in_mempool("life-1", "node-1")
        .expect("mempool");
    set.note_confirmed("life-1", block_hash(0x77), 101)
        .expect("confirmation");

    assert_eq!(set.journal().tx_state(&txid), Some(TxState::Confirmed));
    // The payer's inputs are spent.
    assert_eq!(set.balance_sats(&GOLDEN_PAYER), 0);
    assert!(set.available_outpoints(&GOLDEN_PAYER).is_empty());
    // The destination owner is NOT auto-credited: funding observations
    // are explicit and carry source provenance (T3 observes canonical
    // blocks); no implicit destination rule exists.
    assert_eq!(set.balance_sats(&GOLDEN_DEST_OWNER), 0);
    // The explicit observation (destination output 0: destinations are
    // serialized before change) credits the owner wallet. H5: the
    // observation carries the owner's derivation locus (receive-0) and
    // NO script - the set derives the script itself.
    set.observe_funding(&FundingObservation {
        outpoint: OutPoint::new(txid, 0),
        amount_sats: GOLDEN_DEST_SATS,
        chain: Chain::Receive,
        index: 0,
        owner_role: GOLDEN_DEST_OWNER.clone(),
        source: FundingSource::OwnTransaction,
        context: FundingContext::OwnTransaction,
    })
    .expect("explicit destination funding observation");
    assert_eq!(set.balance_sats(&GOLDEN_DEST_OWNER), GOLDEN_DEST_SATS);
    // The change output belongs to the payer's change chain and is
    // credited the same explicit way (at the deterministic change index
    // the signed transaction recorded).
    let change_index = view.change_index.expect("change index published");
    let parsed = common::parse_raw_tx(&decode_hex(&view.raw_hex));
    let change_value = parsed.outputs[1].value_sats;
    set.observe_funding(&FundingObservation {
        outpoint: OutPoint::new(txid, 1),
        amount_sats: change_value,
        chain: Chain::Change,
        index: change_index,
        owner_role: GOLDEN_PAYER.clone(),
        source: FundingSource::OwnTransaction,
        context: FundingContext::OwnTransaction,
    })
    .expect("explicit change funding observation");
    assert_eq!(set.balance_sats(&GOLDEN_PAYER), change_value);
}

#[test]
fn reorg_displacement_and_bounded_same_bytes_resubmission() {
    let dir = tempfile::tempdir().expect("tempdir");
    let set = conformance_set(dir.path());
    let view = signed_and_broadcast(&set, "reorg-1", 0x62);
    let txid = view.txid;
    let raw = decode_hex(&view.raw_hex);
    let spent = view.spent_outpoints.clone();

    set.note_accepted_by_rpc("reorg-1", "node-1")
        .expect("rpc accept");
    // A resubmission while merely accepted (not reorged) is refused.
    let error = set
        .reorg_resubmit("reorg-1", &raw, &spent)
        .expect_err("resubmission requires the reorged state");
    assert!(matches!(
        error,
        spc_lab_txd::error::Error::Journal(
            spc_lab_txd::error::JournalError::ReorgRetryNotEligible { .. }
        )
    ));

    // The first confirmation finishes the command; later confirmation
    // cycles in this test drive the journal's tx-state API directly
    // (finish is once-only by design).
    set.note_confirmed("reorg-1", block_hash(0x88), 42)
        .expect("first confirmation");
    assert_eq!(set.journal().tx_state(&txid), Some(TxState::Confirmed));

    // Canonical displacement: the confirmed inputs degrade to
    // possibly-spent claims - preserved, never silently released, never
    // spendable again by this signer.
    set.note_reorged("reorg-1", block_hash(0x88), 42)
        .expect("reorg displacement");
    assert_eq!(set.journal().tx_state(&txid), Some(TxState::Reorged));
    assert!(set.available_outpoints(&GOLDEN_PAYER).is_empty());
    assert_eq!(set.balance_sats(&GOLDEN_PAYER), 0);

    // Resubmission eligibility is fail-closed: different bytes refused.
    let mut tampered = raw.clone();
    tampered[0] ^= 0x01;
    let error = set
        .reorg_resubmit("reorg-1", &tampered, &spent)
        .expect_err("byte-different resubmission refused");
    assert!(matches!(
        error,
        spc_lab_txd::error::Error::Journal(
            spc_lab_txd::error::JournalError::ReorgRetryNotEligible { .. }
        )
    ));
    // ... and an unproven original input refused.
    let error = set
        .reorg_resubmit("reorg-1", &raw, &[])
        .expect_err("unproven input refused");
    assert!(matches!(
        error,
        spc_lab_txd::error::Error::Journal(
            spc_lab_txd::error::JournalError::ReorgRetryNotEligible { .. }
        )
    ));

    // The correct same-bytes resubmission (all original inputs proven
    // unspent from canonical blocks) succeeds and consumes a send slot.
    let sequence = set
        .reorg_resubmit("reorg-1", &raw, &spent)
        .expect("eligible same-bytes resubmission");
    assert!(
        sequence >= 2,
        "monotonic send sequence after the first broadcast"
    );
    assert_eq!(set.journal().tx_state(&txid), Some(TxState::BroadcastSent));

    // The retry budget is exactly three same-bytes resubmissions.
    for cycle in 2..=3u32 {
        set.note_accepted_by_rpc("reorg-1", "node-1")
            .expect("rpc accept");
        set.journal()
            .record_confirmed("reorg-1", block_hash(0x88), 42 + u64::from(cycle))
            .expect("re-confirmation");
        set.note_reorged("reorg-1", block_hash(0x88), 42 + u64::from(cycle))
            .expect("re-displacement");
        set.reorg_resubmit("reorg-1", &raw, &spent)
            .expect("resubmission within the cap");
    }
    // The fourth displacement exhausts the budget: terminal refusal.
    set.note_accepted_by_rpc("reorg-1", "node-1")
        .expect("rpc accept");
    set.journal()
        .record_confirmed("reorg-1", block_hash(0x88), 99)
        .expect("final confirmation");
    set.note_reorged("reorg-1", block_hash(0x88), 99)
        .expect("final displacement");
    let error = set
        .reorg_resubmit("reorg-1", &raw, &spent)
        .expect_err("retry cap is bounded");
    assert!(matches!(
        error,
        spc_lab_txd::error::Error::Journal(
            spc_lab_txd::error::JournalError::ReorgResubmitCapExceeded { .. }
        )
    ));
}

#[test]
fn conflicted_is_terminal_and_consumes_claims() {
    let dir = tempfile::tempdir().expect("tempdir");
    let set = conformance_set(dir.path());
    let view = signed_and_broadcast(&set, "conf-1", 0x63);
    let txid = view.txid;
    set.note_accepted_by_rpc("conf-1", "node-1")
        .expect("rpc accept");
    set.note_observed_in_mempool("conf-1", "node-1")
        .expect("mempool");

    // A conflicting spend was admitted: terminal.
    set.note_conflicted("conf-1", "conflicting spend admitted")
        .expect("conflict recorded");
    assert_eq!(set.journal().tx_state(&txid), Some(TxState::Conflicted));

    // No further transition leaves the terminal state - not observation,
    // not unknown, not confirmation.
    assert!(set.note_observed_in_mempool("conf-1", "node-1").is_err());
    assert!(set.note_unknown("conf-1", "timeout").is_err());
    assert!(set
        .journal()
        .record_confirmed("conf-1", block_hash(0x99), 50)
        .is_err());
    // The inputs were consumed by the conflicting spend: gone for good.
    assert!(set.available_outpoints(&GOLDEN_PAYER).is_empty());
    assert_eq!(set.balance_sats(&GOLDEN_PAYER), 0);
}

#[test]
fn pause_barrier_blocks_broadcasts_until_resumed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let set = conformance_set(dir.path());
    let (outpoint, _script) = fund_payer(&set, &GOLDEN_PAYER, fixture_outpoint(0x64), 1_000_000);
    let request = golden_request(&set, "pause-1");
    let view = match set.sign_payment(&request).expect("sign") {
        PaymentOutcome::Signed(view) => view,
        PaymentOutcome::ReplayedStored(_) => panic!("fresh command freshly signs"),
    };
    assert_eq!(view.spent_outpoints, vec![outpoint]);

    // The quiesce barrier: after the pause acknowledgment, no broadcast
    // may be journaled until resume.
    set.pause_sends("pause-1", 12_345)
        .expect("barrier journaled");
    assert!(set.note_broadcast_sent("pause-1").is_err());
    set.resume_sends("pause-1").expect("resume journaled");
    let sequence = set
        .note_broadcast_sent("pause-1")
        .expect("broadcast allowed after resume");
    assert_eq!(sequence, 1, "the first send slot after resume");
}
