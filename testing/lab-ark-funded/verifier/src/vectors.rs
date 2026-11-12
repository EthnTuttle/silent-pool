//! Frozen public stock proofs only. No vector generation, wallet or signer.
//! These tests are prepared, not executed until the parent's heavy-build slot.
use super::*;

fn fixture(name: &str) -> Request {
    let values: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/stock-public.json")).unwrap();
    let raw = values["public"][name].as_str().unwrap();
    let vtxo = Vtxo::<Full>::deserialize(&hex::decode(raw).unwrap()).unwrap();
    let anchor_name = if name.starts_with("round") || name == "arkoor3_vtxo" {
        "round_tx"
    } else {
        "anchor_tx"
    };
    Request {
        schema: "spc.lab.ap5.vtxo-proof-request/v1".into(),
        raw_vtxo_hex: raw.into(),
        anchor_tx_hex: values["public"][anchor_name].as_str().unwrap().into(),
        expected: Expected {
            user_pubkey: vtxo.user_pubkey().to_string(),
            server_pubkey: "0365a81233741893bbe2461b8d479dadc5880594fe6f7479180d5843820af72b62"
                .into(),
            amount_sats: match name {
                "arkoor2_vtxo" => 8000,
                "arkoor_htlc_out_vtxo" => 9000,
                _ => 10000,
            },
            expiry_height: 101010,
            exit_delta: 2016,
            max_exit_depth: 100,
        },
    }
}

// This mandatory positive is entirely embedded. A missing live archive can
// never skip it, and a dummy/unsigned VTXO cannot satisfy its witness checks.
#[test]
fn synthetic_signed_board_positive_is_unconditional() {
    let mut req = fixture("board_vtxo");
    req.expected.user_pubkey =
        "030a752219f1b94bbdf8994a0a980cdda08c2ad094cb29dd834878db6dee1612ee".into();
    let raw = hex::decode(&req.raw_vtxo_hex).unwrap();
    let vtxo = Vtxo::<Full>::deserialize(&raw).unwrap();
    assert_eq!(vtxo.serialize(), raw);
    assert!(vtxo.has_all_witnesses());
    let transitions: Vec<_> = vtxo.transactions().collect();
    assert!(!transitions.is_empty());
    assert!(transitions.iter().all(|item| item.tx.input[0]
        .witness
        .iter()
        .any(|element| element.len() == 64)));
    let result = verify(req).expect("embedded signed board must pass production crypto validation");
    assert_eq!(result["verified"], true);
    assert_eq!(result["policy_kind"], "pubkey");
    assert_eq!(result["amount_sats"], 10000);
    assert_eq!(
        result["user_pubkey"],
        "030a752219f1b94bbdf8994a0a980cdda08c2ad094cb29dd834878db6dee1612ee"
    );
    assert_eq!(
        result["server_pubkey"],
        "0365a81233741893bbe2461b8d479dadc5880594fe6f7479180d5843820af72b62"
    );
    assert_eq!(
        result["id"],
        "56315e0b35be9ae01700116df4a86ad359fee4e4119ba7ee908601d921a4e924:0"
    );
    assert_eq!(
        result["chain_anchor"],
        "8f76befc98e5bd4dbbd2f811f9dd5eaee62a29710f5bf0c877fe3837dfd01151:0"
    );
    assert_eq!(
        result["raw_sha256"],
        "621bd7065d91b7901fc42ee0ba8e515352f0713c01142c36a175ce74ec8f6f48"
    );
    assert_eq!(
        result["anchor_raw_sha256"],
        "6ee5df26b6f235ece4f316a713db3a84f3fbf432cf20595e146170875e191331"
    );
}

#[test]
fn frozen_board_round_and_arkoor_full_signatures_validate() {
    for name in ["board_vtxo", "arkoor2_vtxo", "round1_vtxo", "arkoor3_vtxo"] {
        let req = fixture(name);
        let expected_amount = req.expected.amount_sats;
        let result = verify(req).unwrap_or_else(|err| panic!("{name}: {err:#}"));
        assert_eq!(result["verified"], true);
        assert_eq!(result["amount_sats"], expected_amount);
    }
}

#[test]
fn same_u_in_valid_htlc_is_not_full_pubkey_policy() {
    // Stock round2 is an HTLC too, not a Pubkey-policy positive. Keep both
    // signed fixtures covered without weakening production full-policy equality.
    for name in ["arkoor_htlc_out_vtxo", "round2_vtxo"] {
        let req = fixture(name);
        let vtxo = Vtxo::<Full>::deserialize(&hex::decode(&req.raw_vtxo_hex).unwrap()).unwrap();
        let anchor =
            bitcoin::consensus::deserialize(&hex::decode(&req.anchor_tx_hex).unwrap()).unwrap();
        vtxo.validate(&anchor)
            .expect("the alternate full policy must itself be cryptographically valid");
        assert_eq!(vtxo.user_pubkey().to_string(), req.expected.user_pubkey);
        assert!(verify(req)
            .unwrap_err()
            .to_string()
            .contains("full VTXO policy mismatch"));
    }
}

#[test]
fn foreign_owner_and_server_are_rejected() {
    let req = fixture("board_vtxo");
    let other = fixture("arkoor2_vtxo").expected.user_pubkey;
    let mut wrong_owner = req.clone();
    wrong_owner.expected.user_pubkey = other.clone();
    assert!(verify(wrong_owner)
        .unwrap_err()
        .to_string()
        .contains("policy mismatch"));
    let mut wrong_server = req;
    wrong_server.expected.server_pubkey = other;
    assert!(verify(wrong_server)
        .unwrap_err()
        .to_string()
        .contains("provider mismatch"));
}

#[test]
fn first_and_terminal_input_signature_corruption_is_not_just_decode_failure() {
    for name in ["board_vtxo", "arkoor2_vtxo", "round1_vtxo", "arkoor3_vtxo"] {
        let req = fixture(name);
        let bytes = hex::decode(&req.raw_vtxo_hex).unwrap();
        let good = Vtxo::<Full>::deserialize(&bytes).unwrap();
        let txs: Vec<_> = good.transactions().collect();
        for index in [0, txs.len() - 1] {
            // Extract a real Schnorr input signature from the reconstructed
            // transition, then mutate only its scalar half in the public proof.
            let signature = txs[index].tx.input[0]
                .witness
                .iter()
                .find(|item| item.len() == 64)
                .unwrap();
            let offsets: Vec<_> = bytes
                .windows(64)
                .enumerate()
                .filter_map(|(i, b)| (b == signature).then_some(i))
                .collect();
            assert_eq!(
                offsets.len(),
                1,
                "signature must identify a unique encoded transition"
            );
            let mut changed = bytes.clone();
            changed[offsets[0] + 48] ^= 1;
            let bad = Vtxo::<Full>::deserialize(&changed).expect("canonical proof still decodes");
            let anchor =
                bitcoin::consensus::deserialize(&hex::decode(&req.anchor_tx_hex).unwrap()).unwrap();
            assert_eq!(bad.id(), good.id());
            assert_eq!(bad.serialize(), changed);
            bad.validate_unsigned(&anchor)
                .expect("all non-signature structure remains valid");
            assert!(
                bad.validate(&anchor).is_err(),
                "full stock validation must reject the signature"
            );
            let mut changed_req = req.clone();
            changed_req.raw_vtxo_hex = hex::encode(changed);
            assert!(
                verify(changed_req).is_err(),
                "production verifier must use full validation"
            );
        }
    }
}

#[test]
fn wrong_anchor_input_and_output_and_truncated_proof_reject() {
    let req = fixture("board_vtxo");
    for change_output in [false, true] {
        let mut anchor: bitcoin::Transaction =
            bitcoin::consensus::deserialize(&hex::decode(&req.anchor_tx_hex).unwrap()).unwrap();
        if change_output {
            anchor.output[0].value = bitcoin::Amount::from_sat(anchor.output[0].value.to_sat() + 1);
        } else {
            anchor.input[0].previous_output.vout = 1;
        }
        let mut changed = req.clone();
        changed.anchor_tx_hex = hex::encode(bitcoin::consensus::serialize(&anchor));
        assert!(verify(changed)
            .unwrap_err()
            .to_string()
            .contains("anchor transaction identity"));
    }
    let mut changed = req;
    changed
        .raw_vtxo_hex
        .truncate(changed.raw_vtxo_hex.len() - 2);
    assert!(verify(changed).is_err());
}
