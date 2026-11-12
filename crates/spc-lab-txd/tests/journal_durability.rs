//! Integration suite 2: durable crash recovery, replay and conflict
//! semantics, reservation exclusivity, and run binding.
//!
//! Every test drives the journal to an exact crash phase (or a real
//! crash point in the payment flow), drops the wallet set entirely, and
//! reopens from disk: the journal replay plus the fail-closed key-file
//! verification must reconstruct the same world with no double payment,
//! no reselection and no regeneration.

mod common;

use bitcoin::hashes::Hash as _;

use common::{
    conformance_set, fixture_outpoint, fund_payer, golden_request, reopen_conformance_set,
    request_record_of, standard_test_config, GOLDEN_PAYER, GOLDEN_RAW_HEX, GOLDEN_TXID,
};
use spc_lab_txd::error::{Error, JournalError};
use spc_lab_txd::wallet::{PaymentOutcome, PaymentRequest, TrafficWalletSet};

/// Drive the journal of `set` to exactly the post-crash `Reserved` phase
/// of `request`: the command is begun and its inputs durably reserved,
/// but no signed bytes exist.
fn drive_to_reserved(set: &TrafficWalletSet, request: &PaymentRequest) {
    set.journal()
        .begin_command(&request.command_id, request_record_of(request))
        .expect("begin command");
    let outpoints: Vec<bitcoin::OutPoint> = set
        .available_outpoints(&request.payer_role)
        .into_iter()
        .map(|(outpoint, _, _, _, _)| outpoint)
        .collect();
    assert!(!outpoints.is_empty(), "funding exists before reservation");
    set.journal()
        .reserve(&request.command_id, &outpoints, 0)
        .expect("reserve inputs");
}

#[test]
fn crash_after_reservation_resumes_same_inputs_identically() {
    let dir = tempfile::tempdir().expect("tempdir");
    let set = conformance_set(dir.path());
    let (outpoint, _script) = fund_payer(&set, &GOLDEN_PAYER, fixture_outpoint(0x42), 1_000_000);
    let request = golden_request(&set, "crash-reserved");
    drive_to_reserved(&set, &request);
    // The inputs are locked: no other command can take them.
    assert!(set.available_outpoints(&GOLDEN_PAYER).is_empty());

    // Crash: the process dies here; everything must come from disk.
    drop(set);
    let set = reopen_conformance_set(dir.path());

    let outcome = set
        .sign_payment(&request)
        .expect("reserved-phase resume signs");
    let view = match outcome {
        PaymentOutcome::Signed(view) => view,
        PaymentOutcome::ReplayedStored(_) => {
            panic!("a Reserved-phase command resumes by signing, not replay")
        }
    };
    // The SAME outpoints and the SAME change index were reused - never
    // reselected - and the result is byte-identical to an uncrashed sign
    // of the same request (the golden fixture): the crash is invisible.
    assert_eq!(view.spent_outpoints, vec![outpoint]);
    assert_eq!(view.raw_hex, GOLDEN_RAW_HEX);
    assert_eq!(view.txid.to_string(), GOLDEN_TXID);
    // The payment is complete and durable; the funding is locked to this
    // command (not spendable by any other).
    assert_eq!(set.balance_sats(&GOLDEN_PAYER), 0);
    assert_eq!(
        set.journal().tx_raw_hex(&view.txid),
        Some(GOLDEN_RAW_HEX.to_string())
    );
}

#[test]
fn crash_after_signed_replays_stored_bytes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let set = conformance_set(dir.path());
    let (outpoint, _script) = fund_payer(&set, &GOLDEN_PAYER, fixture_outpoint(0x42), 1_000_000);
    let request = golden_request(&set, "crash-signed");
    let first = match set.sign_payment(&request).expect("sign") {
        PaymentOutcome::Signed(view) => view,
        PaymentOutcome::ReplayedStored(_) => panic!("fresh command freshly signs"),
    };
    assert_eq!(first.spent_outpoints, vec![outpoint]);

    // Crash after the signed bytes (and txid) became durable.
    drop(set);
    let set = reopen_conformance_set(dir.path());

    // A same-id retry returns the STORED bytes - never a second payment,
    // never a re-signature, even across a crash.
    let second = match set.sign_payment(&request).expect("replay") {
        PaymentOutcome::ReplayedStored(view) => view,
        PaymentOutcome::Signed(_) => panic!("stored command must replay, not re-sign"),
    };
    assert_eq!(second.txid, first.txid);
    assert_eq!(second.raw_hex, GOLDEN_RAW_HEX);
    assert_eq!(second.txid.to_string(), GOLDEN_TXID);
    // The stored raw bytes and txid are mutually consistent.
    let stored = set
        .journal()
        .tx_raw_hex(&second.txid)
        .expect("stored raw hex");
    let txid_from_bytes = common::sha256d(&common::decode_hex(&stored));
    assert_eq!(
        bitcoin::Txid::from_byte_array(txid_from_bytes).to_string(),
        GOLDEN_TXID,
        "stored raw bytes hash to the stored txid"
    );
}

#[test]
fn same_command_conflicting_payload_is_a_durable_conflict() {
    let dir = tempfile::tempdir().expect("tempdir");
    let set = conformance_set(dir.path());
    let (_outpoint, _script) = fund_payer(&set, &GOLDEN_PAYER, fixture_outpoint(0x43), 1_000_000);
    let request = golden_request(&set, "dup-cmd");
    let view = match set.sign_payment(&request).expect("sign") {
        PaymentOutcome::Signed(view) => view,
        PaymentOutcome::ReplayedStored(_) => panic!("fresh command freshly signs"),
    };

    // The same command id with a DIFFERENT payload (different amount to
    // the same destination) is a durable conflict, not a second payment.
    let mut conflicting = golden_request(&set, "dup-cmd");
    conflicting.destinations = vec![view.destinations[0].clone()];
    conflicting.destinations[0].amount_sats = 200_000;
    let error = set
        .sign_payment(&conflicting)
        .expect_err("different payload under the same id must conflict");
    assert!(matches!(
        error,
        Error::Journal(JournalError::CommandConflict { .. })
    ));

    // The conflict is durable: after a crash+reopen, the conflicting
    // payload is still refused and the original still replays.
    drop(set);
    let set = reopen_conformance_set(dir.path());
    let error = set
        .sign_payment(&conflicting)
        .expect_err("conflict survives the crash");
    assert!(matches!(
        error,
        Error::Journal(JournalError::CommandConflict { .. })
    ));
    let replay = match set.sign_payment(&request).expect("original replays") {
        PaymentOutcome::ReplayedStored(view) => view,
        PaymentOutcome::Signed(_) => panic!("stored command must replay"),
    };
    assert_eq!(replay.raw_hex, view.raw_hex);
    assert_eq!(replay.txid, view.txid);
    assert_eq!(set.balance_sats(&GOLDEN_PAYER), 0);
}

#[test]
fn same_outpoint_reservation_is_exclusive_across_commands() {
    let dir = tempfile::tempdir().expect("tempdir");
    let set = conformance_set(dir.path());
    let (outpoint, _script) = fund_payer(&set, &GOLDEN_PAYER, fixture_outpoint(0x44), 1_000_000);

    // Command A reaches the Reserved phase on the only funded outpoint.
    let request_a = golden_request(&set, "cmd-a");
    drive_to_reserved(&set, &request_a);

    // Command B (a concurrent same-outpoint reservation) is refused.
    // Its change index must be the deterministic next (A consumed 0) to
    // reach the exclusivity check at all.
    let request_b = golden_request(&set, "cmd-b");
    set.journal()
        .begin_command("cmd-b", request_record_of(&request_b))
        .expect("begin command b");
    let error = set
        .journal()
        .reserve("cmd-b", std::slice::from_ref(&outpoint), 1)
        .expect_err("an already-reserved outpoint is exclusive");
    assert!(matches!(error, JournalError::AlreadyReserved { .. }));

    // And B cannot pay either: it resumes as a Began command (same id,
    // same digest), finds no available funding (A holds the only
    // outpoint) and fails closed - B never spends anything.
    let error = set
        .sign_payment(&request_b)
        .expect_err("command b cannot pay");
    assert!(matches!(
        error,
        Error::Builder(spc_lab_txd::error::BuilderError::InsufficientFunds { .. })
    ));

    // A completes normally on its reserved inputs.
    let view = match set.sign_payment(&request_a).expect("a signs") {
        PaymentOutcome::Signed(view) => view,
        PaymentOutcome::ReplayedStored(_) => panic!("reserved-phase resume signs"),
    };
    assert_eq!(view.spent_outpoints, vec![outpoint]);
}

#[test]
fn wallet_set_from_another_run_or_network_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    {
        let set = conformance_set(dir.path());
        let (_outpoint, _script) =
            fund_payer(&set, &GOLDEN_PAYER, fixture_outpoint(0x45), 1_000_000);
    }
    // A different run seed cannot reopen the private artifacts: the
    // key-file fingerprint fails closed and nothing is regenerated.
    let mut wrong_seed = standard_test_config();
    wrong_seed.run_seed += 1;
    let error = TrafficWalletSet::reopen_for_conformance_tests(&wrong_seed, dir.path())
        .expect_err("wrong seed must fail closed");
    assert!(matches!(
        error,
        Error::Key(spc_lab_txd::error::KeyError::FingerprintMismatch { .. })
    ));
    // A different network cannot reopen either.
    let mut wrong_network = standard_test_config();
    wrong_network.network = bitcoin::Network::Testnet;
    let error = TrafficWalletSet::reopen_for_conformance_tests(&wrong_network, dir.path())
        .expect_err("wrong network must fail closed");
    assert!(matches!(
        error,
        Error::Key(spc_lab_txd::error::KeyError::WrongNetwork { .. })
    ));
    // The correct run still reopens (nothing was overwritten).
    let set = reopen_conformance_set(dir.path());
    assert_eq!(set.balance_sats(&GOLDEN_PAYER), 1_000_000);
}
