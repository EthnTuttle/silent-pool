//! Integration suite 1: independently verified signing and the golden
//! known-raw-transaction fixture.
//!
//! Every signature, txid, fee and output in this suite is checked by the
//! independent path in `common` (hand-written raw parser, own varint
//! codec, sha2 SHA-256d, raw secp256k1 verify) - never by the builder's
//! own code. The golden fixture is a fixed, embedded raw transaction
//! regenerated from the fixed seed; any change in derivation, selection,
//! fee convergence or serialization breaks the constant.

mod common;

use bitcoin::hashes::Hash as _;

use common::{
    conformance_set, decode_hex, fixture_outpoint, fund_payer, golden_request, parse_raw_tx,
    verify_signed_traffic, VerifyUtxo, GOLDEN_COMMAND_ID, GOLDEN_DEST_OWNER, GOLDEN_DEST_SATS,
    GOLDEN_FEE_RATE_SATS_VB, GOLDEN_FUNDING_SATS, GOLDEN_PAYER, GOLDEN_RAW_HEX, GOLDEN_TXID,
};
use spc_lab_txd::wallet::PaymentOutcome;

/// Sign the golden fixture in a fresh directory and return the published
/// view plus the raw bytes.
fn sign_golden(dir: &std::path::Path) -> (spc_lab_txd::wallet::SignedPaymentView, Vec<u8>) {
    let set = conformance_set(dir);
    let (_outpoint, _funding_script) = fund_payer(
        &set,
        &GOLDEN_PAYER,
        fixture_outpoint(0x42),
        GOLDEN_FUNDING_SATS,
    );
    let request = golden_request(&set, GOLDEN_COMMAND_ID);
    let outcome = set.sign_payment(&request).expect("golden payment signs");
    let view = match outcome {
        PaymentOutcome::Signed(view) => view,
        PaymentOutcome::ReplayedStored(_) => panic!("fresh command must freshly sign"),
    };
    let raw = decode_hex(&view.raw_hex);
    (view, raw)
}

fn golden_utxos(funding_script: &[u8]) -> Vec<VerifyUtxo> {
    vec![VerifyUtxo {
        prevout_txid: [0x42; 32],
        prevout_vout: 0,
        amount_sats: GOLDEN_FUNDING_SATS,
        script_pubkey: funding_script.to_vec(),
    }]
}

#[test]
fn golden_fixture_is_known_and_independently_verified() {
    let dir = tempfile::tempdir().expect("tempdir");
    let set = conformance_set(dir.path());
    let (outpoint, funding_script) = fund_payer(
        &set,
        &GOLDEN_PAYER,
        fixture_outpoint(0x42),
        GOLDEN_FUNDING_SATS,
    );
    assert_eq!(
        outpoint,
        bitcoin::OutPoint::new(bitcoin::Txid::from_byte_array([0x42; 32]), 0)
    );
    let request = golden_request(&set, GOLDEN_COMMAND_ID);
    let destination_script = request.destinations[0].script_pubkey.as_bytes().to_vec();
    // The destination is owner-tagged for the destination wallet (the
    // explicit `observe_funding` on confirmation credits it; there is no
    // implicit destination rule).
    assert_eq!(
        request.destinations[0].owner,
        Some(GOLDEN_DEST_OWNER.clone()),
        "golden destination is owner-tagged"
    );

    let outcome = set.sign_payment(&request).expect("golden payment signs");
    let view = match outcome {
        PaymentOutcome::Signed(view) => view,
        PaymentOutcome::ReplayedStored(_) => panic!("fresh command must freshly sign"),
    };

    // The known raw transaction and its txid are exactly the constants.
    assert_eq!(view.raw_hex, GOLDEN_RAW_HEX, "golden raw hex");
    assert_eq!(view.txid.to_string(), GOLDEN_TXID, "golden txid");

    // Independent verification of every signature and the txid.
    let raw = decode_hex(&view.raw_hex);
    let facts = verify_signed_traffic(&raw, &golden_utxos(funding_script.as_bytes()));

    // Exact destination, fee and change (checked against the parsed raw
    // bytes, not the builder's claims).
    let parsed = parse_raw_tx(&raw);
    assert_eq!(parsed.outputs.len(), 2, "destination + change");
    let mut change_sats = 0;
    let mut saw_destination = false;
    for (script, value) in facts.output_scripts.iter().zip(facts.output_values.iter()) {
        if script == &destination_script {
            assert_eq!(*value, GOLDEN_DEST_SATS, "exact destination amount");
            saw_destination = true;
        } else {
            change_sats = *value;
        }
    }
    assert!(saw_destination, "destination output present");
    assert!(change_sats > 0, "change output present");
    assert_eq!(
        facts.output_total_sats,
        change_sats + GOLDEN_DEST_SATS,
        "independently summed outputs == destination + change"
    );
    assert_eq!(
        change_sats + GOLDEN_DEST_SATS + facts.fee_sats,
        GOLDEN_FUNDING_SATS,
        "change + destination + fee == funding (no creation, no donation)"
    );

    // Fee band on the final signed size: never below the requested rate,
    // inside the policy ceiling, expressed as exact integer products.
    let vbytes = raw.len() as u64; // legacy: vsize == size
    assert_eq!(view.vbytes, vbytes, "published vbytes is the final size");
    assert_eq!(view.weight_wu, 4 * vbytes, "legacy weight is 4x size");
    assert!(
        facts.fee_sats >= GOLDEN_FEE_RATE_SATS_VB * vbytes,
        "realized rate never below the requested rate"
    );
    assert!(
        facts.fee_sats <= 20 * vbytes,
        "realized rate inside the policy band"
    );
    assert_eq!(view.fee_sats, facts.fee_sats, "published fee is exact");

    // The published txid is the SHA-256d of the published raw bytes
    // (from the independent verification, not recomputed here).
    assert_eq!(
        view.txid.to_string(),
        bitcoin::Txid::from_byte_array(facts.txid).to_string(),
        "raw bytes hash to the published txid"
    );

    // The destination owner is credited by the journal on confirmation
    // only; for now the journal knows the tx and its facts.
    let journal_facts = set
        .journal()
        .tx_facts(&view.txid)
        .expect("journal knows the signed tx");
    assert_eq!(journal_facts.fee_sats, facts.fee_sats);
    assert_eq!(journal_facts.vbytes, vbytes);
}

#[test]
fn signing_is_deterministic_across_fresh_processes() {
    // The same fixture signed in two independent directories (fresh key
    // files, fresh journals) produces byte-identical transactions: the
    // RFC 6979 nonce, the deterministic selection and the change index
    // are all functions of the request alone.
    let dir_a = tempfile::tempdir().expect("tempdir a");
    let (view_a, raw_a) = sign_golden(dir_a.path());
    let dir_b = tempfile::tempdir().expect("tempdir b");
    let (view_b, raw_b) = sign_golden(dir_b.path());
    assert_eq!(raw_a, raw_b, "byte-identical transactions");
    assert_eq!(view_a.txid, view_b.txid);
    assert_eq!(view_a.fee_sats, view_b.fee_sats);
}
