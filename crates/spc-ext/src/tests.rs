use super::*;
use secp256k1::{Keypair, Secp256k1, SecretKey};

fn key(seed: u8) -> Keypair {
    let secp = Secp256k1::new();
    Keypair::from_secret_key(&secp, &SecretKey::from_slice(&[seed; 32]).unwrap())
}

#[test]
fn txout_consensus_round_trip() {
    let o = TxOut {
        script: vec![0x51, 0x20, 7, 7],
        value: 330,
    };
    let bytes = o.to_consensus().unwrap();
    assert_eq!(TxOut::from_consensus(&bytes).unwrap(), o);
    // value LE then compact-size script length.
    assert_eq!(&bytes[..8], &330u64.to_le_bytes());
    assert_eq!(bytes[8], 4);
}

#[test]
fn distribution_round_trip_and_tlv() {
    let d = SetPayoutDistribution {
        distribution_id: 0x0100_0000_0000_0042,
        pool_payout: TxOut {
            script: vec![0x51, 0x20, 1, 1],
            value: 100,
        },
        payouts: vec![TxOut {
            script: vec![0x51, 0x20, 2, 2],
            value: 300,
        }],
        dust_limits: vec![330],
        additional_outputs: vec![TxOut {
            script: vec![0x6a, 1, 2, 3],
            value: 0,
        }],
    };
    let bytes = d.encode().unwrap();
    assert_eq!(SetPayoutDistribution::decode(&bytes).unwrap(), d);

    let tlv = DistributionIdTlv { id: 42 }.encode();
    assert_eq!(tlv.len(), 13);
    assert_eq!(&tlv[..2], &[0x03, 0x00]);
    assert_eq!(tlv[2], 0x01);
    assert_eq!(&tlv[3..5], &[0x08, 0x00]);
    assert_eq!(DistributionIdTlv::decode(&tlv).unwrap().id, 42);
}

#[test]
fn payout_formula_matches_pr203() {
    let a = compute_payout_amounts(1_000, 100, &[300, 600], &[0, 0], 0).unwrap();
    assert_eq!(a.pays, vec![Some(300), Some(600)]);
    assert_eq!(a.pay_p, 100);
    assert_eq!(a.w, 1_000);
    // Dust prune flows to pay_P.
    let a = compute_payout_amounts(1_000, 100, &[300, 600], &[0, 700], 0).unwrap();
    assert_eq!(a.pays, vec![Some(300), None]);
    assert_eq!(a.pay_p, 700);
    // Fixed outputs are funded off the top.
    let a = compute_payout_amounts(1_000, 100, &[300, 600], &[0, 0], 100).unwrap();
    assert_eq!(a.r, 900);
    assert_eq!(a.pay_p, 1_000 - 270 - 540 - 100);
    assert!(compute_payout_amounts(100, 1, &[], &[], 100).is_err());
}

#[test]
fn ark_identity_signs_and_verifies() {
    let kp = key(3);
    let sp_id = [9u8; 32];
    let mut m = SetArkIdentity {
        ark_id: [1, 2, 3, 4],
        b_ark: PublicKey::from_keypair(&key(4)).serialize(),
        mailbox_mode: MailboxMode::None,
        mailbox_pubkey: vec![],
        mailbox_dh: vec![],
        epoch_fallback: EpochFallback::Static,
        sig: [0; 64],
    };
    m.sign(&sp_id, &kp).unwrap();
    m.verify(&sp_id, &kp.x_only_public_key().0).unwrap();
    let bytes = m.encode().unwrap();
    assert_eq!(SetArkIdentity::decode(&bytes).unwrap(), m);
    // Tampering fails.
    let mut bad = m.clone();
    bad.ark_id = [9, 9, 9, 9];
    assert!(bad.verify(&sp_id, &kp.x_only_public_key().0).is_err());
}

#[test]
fn ark_identity_error_matrix() {
    let kp = key(3);
    let sp_id = [9u8; 32];
    let b_spend = PublicKey::from_keypair(&key(7)).serialize();
    let mut m = SetArkIdentity {
        ark_id: [1, 2, 3, 4],
        b_ark: b_spend, // same as spend key
        mailbox_mode: MailboxMode::None,
        mailbox_pubkey: vec![],
        mailbox_dh: vec![],
        epoch_fallback: EpochFallback::Static,
        sig: [0; 64],
    };
    m.sign(&sp_id, &kp).unwrap();
    let x = kp.x_only_public_key().0;
    assert_eq!(
        m.error_code(&sp_id, &x, &b_spend, &[1, 2, 3, 4]),
        Some("ark-key-is-spend-key")
    );
    assert_eq!(
        m.error_code(&sp_id, &x, &b_spend, &[9, 9, 9, 9]),
        Some("wrong-ark-server")
    );

    let mut none_with_m = m.clone();
    none_with_m.mailbox_pubkey = vec![2; 33];
    assert_eq!(
        none_with_m.error_code(&sp_id, &x, &[3; 33], &[1, 2, 3, 4]),
        Some("bad-mailbox-fields")
    );

    let mut bad_sig = m.clone();
    bad_sig.sig[0] ^= 1;
    assert_eq!(
        bad_sig.error_code(&sp_id, &x, &[3; 33], &[1, 2, 3, 4]),
        Some("bad-signature")
    );
}

#[test]
fn ark_order_signs_and_verifies() {
    let kp = key(5);
    let mut o = ArkSendOrder {
        order_id: [1; 16],
        ark_address: "ark1qqqq".into(),
        sats: 330,
        not_before_height: 800_006,
        sig: [0; 64],
    };
    o.sign(&kp).unwrap();
    o.verify(&kp.x_only_public_key().0).unwrap();
    let bytes = o.encode().unwrap();
    assert_eq!(ArkSendOrder::decode(&bytes).unwrap(), o);
    let mut tampered = o.clone();
    tampered.sats += 1;
    assert!(tampered.verify(&kp.x_only_public_key().0).is_err());
}

#[test]
fn receipt_round_trip() {
    let r = ArkSendReceipt {
        order_id: [2; 16],
        status: ReceiptStatus::Sent,
        vtxo_id: vec![3; 34],
        expiry_height: 900_000,
    };
    let b = r.encode().unwrap();
    assert_eq!(ArkSendReceipt::decode(&b).unwrap(), r);
}

#[test]
fn epoch_mailboxes_sign_and_verify() {
    let kp = key(8);
    let mut m = RegisterEpochMailboxes {
        entries: vec![[1u8; 37], [2u8; 37]],
        sig: [0; 64],
    };
    m.sign(&[3u8; 32], &kp).unwrap();
    m.verify(&[3u8; 32], &kp.x_only_public_key().0).unwrap();
    let b = m.encode().unwrap();
    assert_eq!(RegisterEpochMailboxes::decode(&b).unwrap(), m);
    assert!(m.verify(&[4u8; 32], &kp.x_only_public_key().0).is_err());
}
