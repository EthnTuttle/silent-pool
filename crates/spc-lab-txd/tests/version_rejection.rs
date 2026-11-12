//! Source-only production-builder fixtures. No native funding or node outcome
//! is fabricated/claimed by this offline suite; the live case remains QA-owned.
mod common;
use bitcoin::consensus::{deserialize, serialize};
use bitcoin::hashes::Hash;
use spc_lab_txd::{
    builder::{DestinationSpec, PaymentCategory},
    construction::ConstructionEffect,
    journal::{CommandPhase, OutpointStatus},
    keys::WalletRole,
    wallet::{PaymentOutcome, PaymentRequest, TrafficWalletSet},
};
const AMOUNT: u64 = 1_000_000;
fn setup(dir: &std::path::Path) -> (TrafficWalletSet, PaymentRequest, Vec<common::VerifyUtxo>) {
    let set = common::conformance_set(dir);
    let role = WalletRole::UserPayment { index: 0 };
    let (_, script) = common::fund_payer(&set, &role, common::fixture_outpoint(0x42), AMOUNT);
    let genesis =
        bitcoin::blockdata::constants::genesis_block(bitcoin::Network::Regtest).block_hash();
    set.note_chain_observed(genesis, 0, &[]).unwrap();
    let destination = set
        .next_receive_address(&WalletRole::NetworkWallet { index: 0 })
        .unwrap();
    let request = PaymentRequest {
        command_id: "version-case".into(),
        payer_role: role,
        category: PaymentCategory::UserPayment,
        destinations: vec![DestinationSpec {
            script_pubkey: destination.script_pubkey,
            amount_sats: 100000,
            owner: Some(WalletRole::NetworkWallet { index: 0 }),
        }],
        fee_rate_sats_vb: 3,
    };
    (
        set,
        request,
        vec![common::VerifyUtxo {
            prevout_txid: [0x42; 32],
            prevout_vout: 0,
            amount_sats: AMOUNT,
            script_pubkey: script.to_bytes(),
        }],
    )
}
fn signed(outcome: PaymentOutcome) -> spc_lab_txd::wallet::SignedPaymentView {
    match outcome {
        PaymentOutcome::Signed(v) | PaymentOutcome::ReplayedStored(v) => v,
    }
}
#[test]
fn defaults_identical_selected_v3_signatures_and_after_sign_mutation_reject() {
    let a = tempfile::tempdir().unwrap();
    let (normal, request, utxos) = setup(a.path());
    let before = std::fs::read(normal.journal().path()).unwrap();
    assert!(normal
        .sign_version_rejection_for_acceptance(&request, false)
        .is_err());
    assert_eq!(std::fs::read(normal.journal().path()).unwrap(), before);
    let v2 = signed(normal.sign_payment(&request).unwrap());
    common::verify_signed_traffic(&hex::decode(&v2.raw_hex).unwrap(), &utxos);
    let b = tempfile::tempdir().unwrap();
    let (other, r, _) = setup(b.path());
    assert_eq!(signed(other.sign_payment(&r).unwrap()).raw_hex, v2.raw_hex);
    // Late selection cannot rewrite an already signed ordinary transaction.
    assert!(normal
        .sign_version_rejection_for_acceptance(&request, true)
        .is_err());
    assert_eq!(
        normal
            .journal()
            .command(&request.command_id)
            .unwrap()
            .raw_hex
            .as_deref(),
        Some(v2.raw_hex.as_str())
    );
    let c = tempfile::tempdir().unwrap();
    let (selected, r, inputs) = setup(c.path());
    let v3 = signed(
        selected
            .sign_version_rejection_for_acceptance(&r, true)
            .unwrap(),
    );
    let raw = hex::decode(&v3.raw_hex).unwrap();
    let facts = common::verify_version_rejection_traffic(&raw, &inputs);
    assert_eq!(facts.txid, v3.txid.to_byte_array());
    assert_eq!(facts.fee_sats, v3.fee_sats);
    let tx: bitcoin::Transaction = deserialize(&raw).unwrap();
    assert_eq!(tx.version.0, 3);
    assert!(tx.input.iter().all(|i| i.witness.is_empty()));
    assert!(v3.fee_sats >= 3 * v3.vbytes && v3.fee_sats <= 20 * v3.vbytes);
    assert_eq!(
        selected
            .journal()
            .command(&r.command_id)
            .unwrap()
            .request
            .construction_effect,
        Some(ConstructionEffect::VersionRejectionV1)
    );
    assert_eq!(
        spc_lab_txd::admit::admit_declared_tx(&selected, &r.command_id, &v3.raw_hex, 1000)
            .unwrap()
            .raw_hex,
        v3.raw_hex
    );
    let d = tempfile::tempdir().unwrap();
    let (fresh, _, _) = setup(d.path());
    assert_eq!(
        spc_lab_txd::admit::admit_declared_tx(&fresh, "unselected-external", &v3.raw_hex, 1000)
            .unwrap_err(),
        "traffic_declared_version_not_selected"
    );
    let mut mutated: bitcoin::Transaction = deserialize(&hex::decode(v2.raw_hex).unwrap()).unwrap();
    mutated.version = bitcoin::transaction::Version(3);
    assert!(
        std::panic::catch_unwind(|| common::verify_version_rejection_traffic(
            &serialize(&mutated),
            &utxos
        ))
        .is_err()
    );
    if let Some(path) = std::env::var_os("TRAFFIC_CONSTRUCTION_FIXTURE_OUT") {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let sample = serde_json::json!({
            "scope":"synthetic conformance prevouts; valid production-builder signatures, NOT native-funded proof",
            "constructor":"acceptance_version_rejection_v1","construction_effect":ConstructionEffect::VersionRejectionV1,
            "command_id":r.command_id,"raw_hex":v3.raw_hex,"txid":v3.txid.to_string(),"fee_sats":v3.fee_sats,"vbytes":v3.vbytes,"version":3,
            "inputs":inputs.iter().map(|u|serde_json::json!({"txid":bitcoin::Txid::from_byte_array(u.prevout_txid).to_string(),"vout":u.prevout_vout,"amount_sats":u.amount_sats,"script_hex":hex::encode(&u.script_pubkey)})).collect::<Vec<_>>(),
            "independent_verifier":"tests/common/mod.rs: independent raw parser, SIGHASH_ALL encoder, SHA256d and secp verify",
            "every_signature_verified":true,"native_funded_acceptance":false
        });
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .unwrap();
        file.write_all(&serde_json::to_vec_pretty(&sample).unwrap())
            .unwrap();
        file.sync_all().unwrap();
    }
}
#[test]
fn immutable_selector_replay_reserved_recovery_and_lost_enablement_never_resign_v2() {
    let dir = tempfile::tempdir().unwrap();
    let (set, request, utxos) = setup(dir.path());
    let mut record = common::request_record_of(&request);
    record.construction_effect = Some(ConstructionEffect::VersionRejectionV1);
    assert_ne!(
        spc_lab_txd::journal::request_record_digest(&record),
        TrafficWalletSet::payload_digest(&request)
    );
    set.journal()
        .begin_command(&request.command_id, record)
        .unwrap();
    set.journal()
        .reserve(&request.command_id, &[common::fixture_outpoint(0x42)], 0)
        .unwrap();
    drop(set);
    let set = common::reopen_conformance_set(dir.path());
    let before = std::fs::read(set.journal().path()).unwrap();
    assert!(set
        .sign_version_rejection_for_acceptance(&request, false)
        .is_err());
    assert_eq!(
        set.journal().command_phase(&request.command_id),
        Some(CommandPhase::Reserved)
    );
    assert_eq!(std::fs::read(set.journal().path()).unwrap(), before);
    let v3 = signed(
        set.sign_version_rejection_for_acceptance(&request, true)
            .unwrap(),
    );
    common::verify_version_rejection_traffic(&hex::decode(&v3.raw_hex).unwrap(), &utxos);
    drop(set);
    let set = common::reopen_conformance_set(dir.path());
    let before = std::fs::read(set.journal().path()).unwrap();
    let replay = set
        .sign_version_rejection_for_acceptance(&request, true)
        .unwrap();
    assert!(matches!(replay, PaymentOutcome::ReplayedStored(_)));
    assert_eq!(signed(replay).raw_hex, v3.raw_hex);
    assert_eq!(std::fs::read(set.journal().path()).unwrap(), before);
    assert!(set.sign_payment(&request).is_err());
    assert_eq!(
        set.journal()
            .command(&request.command_id)
            .unwrap()
            .raw_hex
            .as_deref(),
        Some(v3.raw_hex.as_str())
    );
    assert!(serde_json::from_value::<ConstructionEffect>(
        serde_json::json!({"schema":"spc-lab-version-rejection/v2"})
    )
    .is_err());
    assert!(serde_json::from_value::<ConstructionEffect>(
        serde_json::json!({"schema":"spc-lab-version-rejection/v1","version":4})
    )
    .is_err());
}
#[test]
fn exact_rejection_releases_only_fresh_inputs_prior_unknown_remains_possibly_spent() {
    for ever_unknown in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let (set, request, _) = setup(dir.path());
        let v3 = signed(
            set.sign_version_rejection_for_acceptance(&request, true)
                .unwrap(),
        );
        set.note_broadcast_sent(&request.command_id).unwrap();
        if ever_unknown {
            set.note_unknown(
                &request.command_id,
                "actual transport uncertainty remains unresolved",
            )
            .unwrap();
            set.note_broadcast_sent(&request.command_id).unwrap();
        }
        let binding = spc_lab_txd::send_outcome::reviewed_binding(
            spc_lab_txd::send_outcome::REVIEWED_BINARY,
            spc_lab_txd::send_outcome::REVIEWED_CONSENSUS,
        )
        .unwrap();
        let profile = spc_lab_txd::send_outcome::bind(&binding).unwrap();
        let code = spc_lab_txd::send_outcome::definitive_rejection(
            &spc_lab_txd::control::RpcOutcome::NodeRejected {
                code: -26,
                message: "version".into(),
            },
            Some(&profile),
            &v3.raw_hex,
        )
        .unwrap();
        set.note_rejected(&request.command_id, code).unwrap();
        let snapshot = set.journal().observer_snapshot();
        let state = &snapshot.outpoints[&common::fixture_outpoint(0x42)].status;
        if ever_unknown {
            assert_eq!(*state, OutpointStatus::PossiblySpent);
            assert_eq!(set.balance_sats(&request.payer_role), 0);
        } else {
            assert_eq!(*state, OutpointStatus::Available);
            assert_eq!(set.balance_sats(&request.payer_role), AMOUNT);
        }
    }
}
#[test]
fn fee_convergence_preserves_selected_version_on_resigning() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = common::standard_test_config();
    config.fee_policy.min_rate_sats_vb = 3;
    config.fee_policy.max_rate_sats_vb = 3;
    let set = TrafficWalletSet::provision_for_conformance_tests(&config, dir.path()).unwrap();
    let role = WalletRole::UserPayment { index: 0 };
    let (_, script) = common::fund_payer(&set, &role, common::fixture_outpoint(0x42), AMOUNT);
    let destination = set
        .next_receive_address(&WalletRole::NetworkWallet { index: 0 })
        .unwrap();
    let request = PaymentRequest {
        command_id: "convergence".into(),
        payer_role: role,
        category: PaymentCategory::UserPayment,
        destinations: vec![DestinationSpec {
            script_pubkey: destination.script_pubkey,
            amount_sats: 100000,
            owner: Some(WalletRole::NetworkWallet { index: 0 }),
        }],
        fee_rate_sats_vb: 3,
    };
    let v3 = signed(
        set.sign_version_rejection_for_acceptance(&request, true)
            .unwrap(),
    );
    assert_eq!(v3.fee_sats, 3 * v3.vbytes);
    common::verify_version_rejection_traffic(
        &hex::decode(&v3.raw_hex).unwrap(),
        &[common::VerifyUtxo {
            prevout_txid: [0x42; 32],
            prevout_vout: 0,
            amount_sats: AMOUNT,
            script_pubkey: script.to_bytes(),
        }],
    );
}
