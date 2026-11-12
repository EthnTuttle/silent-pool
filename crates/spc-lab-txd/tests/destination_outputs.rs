//! Offline, genuinely signed legacy inputs to stock Bark-style witness outputs.
//! Synthetic prevouts are explicit fixtures; no node funding/acceptance is claimed.
mod common;
#[path = "common/destinations.rs"]
mod destinations;

use bitcoin::consensus::{deserialize, serialize};
use bitcoin::hashes::Hash;
use bitcoin::{Address, Network, ScriptBuf, Transaction};
use common::{conformance_set, fixture_outpoint, fund_payer, verify_signed_traffic, VerifyUtxo};
use spc_lab_txd::admit::admit_declared_tx;
use spc_lab_txd::builder::{
    build_and_sign, selection_fee_upper_bound, DestinationSpec, PaymentCategory, SignableUtxo,
};
use spc_lab_txd::keys::{Chain, TrafficWallet, WalletRole};
use spc_lab_txd::wallet::{PaymentOutcome, PaymentRequest, TrafficWalletSet};

const FUNDING: u64 = 1_000_000;
const AMOUNT: u64 = 100_000;

fn funded(dir: &std::path::Path) -> (TrafficWalletSet, ScriptBuf) {
    let set = conformance_set(dir);
    let (_, script) = fund_payer(
        &set,
        &WalletRole::BootstrapFunding,
        fixture_outpoint(0x42),
        FUNDING,
    );
    (set, script)
}

fn request(destination: DestinationSpec) -> PaymentRequest {
    PaymentRequest {
        command_id: "destination-payment".into(),
        payer_role: WalletRole::BootstrapFunding,
        category: PaymentCategory::UserPayment,
        destinations: vec![destination],
        fee_rate_sats_vb: 3,
    }
}

fn offline_input(script: &ScriptBuf) -> (TrafficWallet, SignableUtxo) {
    let c = common::standard_test_config();
    let wallet = TrafficWallet::new(
        c.run_seed,
        &c.run_id,
        WalletRole::BootstrapFunding,
        c.network,
        c.lookahead,
    )
    .unwrap();
    let input = SignableUtxo::resolve_indexed(
        &wallet,
        fixture_outpoint(0x42),
        Chain::Receive,
        0,
        script,
        FUNDING,
    )
    .unwrap();
    (wallet, input)
}

#[test]
fn witness_outputs_preserve_exact_bytes_fee_sighash_replay_and_external_ownership() {
    for address in destinations::admitted_addresses() {
        let dir = tempfile::tempdir().unwrap();
        let (set, funding_script) = funded(dir.path());
        let address: Address<_> = address.parse().unwrap();
        let expected_script = address
            .clone()
            .require_network(Network::Regtest)
            .unwrap()
            .script_pubkey();
        let destination = set.checked_destination(address, AMOUNT).unwrap();
        assert_eq!(destination.owner, None);
        let request = request(destination);
        let PaymentOutcome::Signed(view) = set.sign_payment(&request).unwrap() else {
            panic!("fresh")
        };
        let raw = hex::decode(&view.raw_hex).unwrap();
        let tx: Transaction = deserialize(&raw).unwrap();
        assert_eq!(serialize(&tx), raw);
        assert_eq!(tx.output.len(), 2);
        assert_eq!(tx.output[0].script_pubkey, expected_script);
        assert_eq!(tx.output[0].value.to_sat(), AMOUNT);
        assert!(tx.output[1].script_pubkey.is_p2pkh());
        assert!(tx.input.iter().all(|i| i.witness.is_empty()));
        // Existing independent raw parser, own Bitcoin SIGHASH_ALL encoder,
        // SHA256d and raw secp verification: no builder or SighashCache reuse.
        let facts = verify_signed_traffic(
            &raw,
            &[VerifyUtxo {
                prevout_txid: [0x42; 32],
                prevout_vout: 0,
                amount_sats: FUNDING,
                script_pubkey: funding_script.to_bytes(),
            }],
        );
        assert_eq!(facts.txid, view.txid.to_byte_array());
        assert_eq!(tx.compute_txid(), view.txid);
        assert_eq!(facts.fee_sats, view.fee_sats);
        assert_eq!(
            FUNDING - tx.output.iter().map(|o| o.value.to_sat()).sum::<u64>(),
            view.fee_sats
        );
        assert_eq!(view.vbytes, raw.len() as u64);
        assert_eq!(view.vbytes, tx.vsize() as u64);
        assert_eq!(view.weight_wu, 4 * view.vbytes);
        assert!(view.fee_sats >= 3 * view.vbytes);
        assert!(view.fee_sats <= 20 * view.vbytes);
        assert_eq!(
            view.fee_sats,
            selection_fee_upper_bound(&request.destinations, 3, 1, true)
        );
        assert_eq!(
            set.journal()
                .command(&request.command_id)
                .unwrap()
                .request
                .destinations[0]
                .owner_role,
            None
        );
        let before = std::fs::read(set.journal().path()).unwrap();
        let PaymentOutcome::ReplayedStored(replay) = set.sign_payment(&request).unwrap() else {
            panic!("replay")
        };
        assert_eq!(replay.raw_hex, view.raw_hex);
        let mut conflict = request.clone();
        let pair = destinations::generated_pair(Network::Regtest);
        conflict.destinations[0].script_pubkey = pair
            .iter()
            .find(|a| a.script_pubkey() != expected_script)
            .unwrap()
            .script_pubkey();
        assert_ne!(
            TrafficWalletSet::payload_digest(&request),
            TrafficWalletSet::payload_digest(&conflict)
        );
        assert!(set.sign_payment(&conflict).is_err());
        // Existing contract journals the conflict without changing the bound
        // request, signature or reservation. It is not a second payment.
        let after_conflict = std::fs::read(set.journal().path()).unwrap();
        assert!(after_conflict.starts_with(&before));
        let appended: serde_json::Value =
            serde_json::from_slice(&after_conflict[before.len()..]).unwrap();
        assert_eq!(appended["record"], "command_conflict");
        let stored = set.journal().command(&request.command_id).unwrap();
        assert_eq!(stored.raw_hex.as_deref(), Some(view.raw_hex.as_str()));
        assert_eq!(
            stored.request.destinations[0].script_hex,
            expected_script.to_hex_string()
        );
        assert_eq!(stored.reserved_outpoints, vec![fixture_outpoint(0x42)]);

        // Raw admission independently verifies signatures and exact fee before
        // reserving the same fixture prevout in its own fresh journal.
        let raw_dir = tempfile::tempdir().unwrap();
        let (raw_set, _) = funded(raw_dir.path());
        let admitted = admit_declared_tx(&raw_set, "injected", &view.raw_hex, 1000).unwrap();
        assert_eq!(admitted.raw_hex, view.raw_hex);
        assert_eq!(admitted.fee_sats, view.fee_sats);
        assert_eq!(admitted.vbytes, view.vbytes);
        assert!(raw_set
            .journal()
            .command("injected")
            .unwrap()
            .request
            .destinations
            .iter()
            .all(|d| d.owner_role.is_none()));
        let committed = std::fs::read(raw_set.journal().path()).unwrap();
        assert_eq!(
            admit_declared_tx(&raw_set, "injected", &view.raw_hex, 1000)
                .unwrap()
                .raw_hex,
            view.raw_hex
        );
        let mut changed = tx.clone();
        changed.output[0].script_pubkey = conflict.destinations[0].script_pubkey.clone();
        assert!(admit_declared_tx(
            &raw_set,
            "injected",
            &hex::encode(serialize(&changed)),
            1000
        )
        .is_err());
        assert_eq!(std::fs::read(raw_set.journal().path()).unwrap(), committed);

        set.note_broadcast_sent(&request.command_id).unwrap();
        set.note_unknown(&request.command_id, "offline uncertainty")
            .unwrap();
        set.note_confirmed(
            &request.command_id,
            bitcoin::BlockHash::from_byte_array([0x77; 32]),
            101,
        )
        .unwrap();
        assert!(set
            .journal()
            .observer_snapshot()
            .outpoints
            .values()
            .all(|o| o.script.is_p2pkh()));
        assert!(set
            .available_outpoints(&WalletRole::BootstrapFunding)
            .iter()
            .all(|(_, _, _, _, s)| s.is_p2pkh()));
        drop(set);
        let reopened = common::reopen_conformance_set(dir.path());
        assert_eq!(
            reopened.journal().tx_facts(&view.txid).unwrap().raw_hex,
            view.raw_hex
        );
    }
}

#[test]
fn rejected_addresses_scripts_and_false_ownership_never_mutate_or_reserve() {
    let dir = tempfile::tempdir().unwrap();
    let (set, script) = funded(dir.path());
    let before = std::fs::read(set.journal().path()).unwrap();
    for address in destinations::rejected_addresses() {
        if let Ok(address) = address.parse() {
            assert!(set.checked_destination(address, AMOUNT).is_err());
        }
        assert_eq!(std::fs::read(set.journal().path()).unwrap(), before);
    }
    let (wallet, input) = offline_input(&script);
    let secp = bitcoin::secp256k1::Secp256k1::new();
    let mut bad_destinations: Vec<_> = destinations::unsupported_scripts()
        .into_iter()
        .map(|script_pubkey| DestinationSpec {
            script_pubkey,
            amount_sats: AMOUNT,
            owner: None,
        })
        .collect();
    for address in destinations::generated_pair(Network::Regtest) {
        bad_destinations.push(DestinationSpec {
            script_pubkey: address.script_pubkey(),
            amount_sats: AMOUNT,
            owner: Some(WalletRole::BootstrapFunding),
        });
    }
    for destination in bad_destinations {
        assert!(set.sign_payment(&request(destination.clone())).is_err());
        assert!(build_and_sign(
            &secp,
            &wallet,
            std::slice::from_ref(&input),
            &[destination],
            3,
            set.fee_policy(),
            0,
            PaymentCategory::UserPayment
        )
        .is_err());
        assert_eq!(std::fs::read(set.journal().path()).unwrap(), before);
        assert_eq!(set.balance_sats(&WalletRole::BootstrapFunding), FUNDING);
    }
    for address in destinations::generated_pair(Network::Regtest) {
        assert!(set
            .checked_destination(address.as_unchecked().clone(), 545)
            .is_err());
        assert!(SignableUtxo::resolve_indexed(
            &wallet,
            input.outpoint,
            Chain::Receive,
            0,
            &address.script_pubkey(),
            FUNDING
        )
        .is_err());
        let mut false_input = input.clone();
        false_input.script_pubkey = address.script_pubkey();
        let destination = set
            .checked_destination(address.as_unchecked().clone(), AMOUNT)
            .unwrap();
        assert!(build_and_sign(
            &secp,
            &wallet,
            &[false_input],
            &[destination],
            3,
            set.fee_policy(),
            0,
            PaymentCategory::UserPayment
        )
        .is_err());
    }
    assert_eq!(std::fs::read(set.journal().path()).unwrap(), before);
}

#[test]
fn raw_admission_rejects_unsupported_outputs_with_valid_legacy_signatures_and_witness_inputs() {
    let dir = tempfile::tempdir().unwrap();
    let (set, script) = funded(dir.path());
    let (wallet, input) = offline_input(&script);
    let secp = bitcoin::secp256k1::Secp256k1::new();
    let destination = set
        .checked_destination(
            destinations::generated_pair(Network::Regtest)[1]
                .as_unchecked()
                .clone(),
            AMOUNT,
        )
        .unwrap();
    let signed = build_and_sign(
        &secp,
        &wallet,
        std::slice::from_ref(&input),
        &[destination],
        3,
        set.fee_policy(),
        0,
        PaymentCategory::UserPayment,
    )
    .unwrap();
    let before = std::fs::read(set.journal().path()).unwrap();
    for output in destinations::unsupported_scripts() {
        let mut tx = signed.tx.clone();
        tx.output[0].script_pubkey = output;
        // Test-only re-signing ensures rejection reaches the OUTPUT guard,
        // rather than merely failing the earlier SIGHASH_ALL verification.
        let hash = bitcoin::sighash::SighashCache::new(&tx)
            .legacy_signature_hash(0, &script, 1)
            .unwrap();
        let signature = bitcoin::ecdsa::Signature::sighash_all(secp.sign_ecdsa(
            &bitcoin::secp256k1::Message::from_digest(hash.to_byte_array()),
            &input.secret_key,
        ));
        tx.input[0].script_sig = bitcoin::script::Builder::new()
            .push_slice(signature.serialize())
            .push_key(&input.public_key)
            .into_script();
        assert_eq!(
            admit_declared_tx(&set, "bad", &hex::encode(serialize(&tx)), 1000).unwrap_err(),
            "traffic_declared_destination_invalid"
        );
        assert_eq!(std::fs::read(set.journal().path()).unwrap(), before);
    }
    let mut tx = signed.tx;
    tx.input[0].witness.push([1; 32]);
    assert_eq!(
        admit_declared_tx(&set, "false-witness", &hex::encode(serialize(&tx)), 1000).unwrap_err(),
        "traffic_declared_profile_invalid"
    );
    assert_eq!(std::fs::read(set.journal().path()).unwrap(), before);
}

#[test]
fn selection_prices_actual_witness_output_lengths_before_reserving() {
    let pair = destinations::generated_pair(Network::Regtest);
    let fee = |script_pubkey| {
        selection_fee_upper_bound(
            &[DestinationSpec {
                script_pubkey,
                amount_sats: AMOUNT,
                owner: None,
            }],
            3,
            1,
            true,
        )
    };
    let legacy = Address::p2pkh(
        bitcoin::PubkeyHash::from_byte_array([7; 20]),
        Network::Regtest,
    )
    .script_pubkey();
    assert_eq!(fee(pair[0].script_pubkey()) + 3 * 3, fee(legacy.clone()));
    assert_eq!(fee(pair[1].script_pubkey()), fee(legacy) + 3 * 9);
    // A P2TR payment needs nine more bytes than the legacy destination.
    // Exactly the correct selection bound + dust succeeds with one input.
    let dir = tempfile::tempdir().unwrap();
    let set = conformance_set(dir.path());
    let destination = set
        .checked_destination(pair[1].as_unchecked().clone(), AMOUNT)
        .unwrap();
    let bound = selection_fee_upper_bound(std::slice::from_ref(&destination), 3, 1, true);
    fund_payer(
        &set,
        &WalletRole::BootstrapFunding,
        fixture_outpoint(0x42),
        AMOUNT + bound + 546,
    );
    let PaymentOutcome::Signed(view) = set.sign_payment(&request(destination)).unwrap() else {
        panic!("fresh")
    };
    let tx: Transaction = deserialize(&hex::decode(view.raw_hex).unwrap()).unwrap();
    assert_eq!(tx.output[1].value.to_sat(), 546);
    assert_eq!(view.fee_sats, bound);
}
