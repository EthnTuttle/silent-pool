use secp256k1::{Keypair, PublicKey, Secp256k1, SecretKey};
use spc_ext::{
    correspondent_error_code, DeliveryMessage, EpochFallback, MailboxMode,
    RegistrationSuccessState, SetArkIdentity, SetArkIdentityError, SetArkIdentitySuccess,
    DELIVERY_VERSION, MSG_SET_ARK_IDENTITY, MSG_SET_ARK_IDENTITY_ERROR,
    MSG_SET_ARK_IDENTITY_SUCCESS,
};

fn key(seed: u8) -> Keypair {
    let secp = Secp256k1::new();
    Keypair::from_secret_key(&secp, &SecretKey::from_slice(&[seed; 32]).unwrap())
}

fn request(seed: u8, request_id: u32) -> SetArkIdentity {
    let mut request = SetArkIdentity {
        ark_id: [1, 2, 3, 4],
        b_ark: PublicKey::from_keypair(&key(seed + 10)).serialize(),
        mailbox_mode: MailboxMode::Static,
        mailbox_pubkey: PublicKey::from_keypair(&key(seed + 20))
            .serialize()
            .to_vec(),
        mailbox_dh: PublicKey::from_keypair(&key(seed + 30))
            .serialize()
            .to_vec(),
        epoch_fallback: EpochFallback::Static,
        sig: [0; 64],
        request_id,
    };
    request.sign(&[seed; 32], &key(seed)).unwrap();
    request
}

#[test]
fn set_identity_v1_has_exact_layout_and_rejects_legacy_bytes() {
    let request = request(3, 1);
    let bytes = request.encode().unwrap();
    assert_eq!(bytes.len(), 176);
    assert_eq!(bytes[0], DELIVERY_VERSION);
    assert_eq!(&bytes[1..5], &[1, 2, 3, 4]);
    assert_eq!(&bytes[bytes.len() - 4..], &1u32.to_le_bytes());
    assert_eq!(SetArkIdentity::decode(&bytes).unwrap(), request);

    // The unversioned legacy encoding is not a v1 alias.
    assert!(SetArkIdentity::decode(&bytes[1..]).is_err());
    let mut trailing = bytes;
    trailing.push(0);
    assert!(SetArkIdentity::decode(&trailing).is_err());
}

#[test]
fn request_id_is_correlation_and_an_authorization_field() {
    let request = request(4, 1);
    let scan = key(4).x_only_public_key().0;
    request.verify(&[4; 32], &scan).unwrap();

    // Moving an old valid signature to a greater counter must not authorize a
    // rollback. The wire position stays after sig; the signature preimage
    // commits to the counter as the AP-2 contract amendment requires.
    let mut replay = request.clone();
    replay.request_id = 2;
    assert!(replay.verify(&[4; 32], &scan).is_err());
    assert_ne!(
        request.signed_fields_digest(&[4; 32]).unwrap(),
        replay.signed_fields_digest(&[4; 32]).unwrap()
    );

    let encoded = replay.encode().unwrap();
    assert_eq!(SetArkIdentity::decode(&encoded).unwrap(), replay);
}

#[test]
fn raw_unknown_mode_has_the_new_explicit_error_code() {
    let bytes = request(5, 9).encode().unwrap();
    let mut unknown = bytes;
    // version + ark_id + b_ark
    unknown[38] = 4;
    assert_eq!(
        correspondent_error_code(MSG_SET_ARK_IDENTITY, &unknown),
        "unsupported-mailbox-mode"
    );
    assert_eq!(
        spc_ext::correlated_request_id(MSG_SET_ARK_IDENTITY, &unknown),
        9
    );
}

#[test]
fn success_and_error_frames_are_versioned_and_correlated() {
    let success = SetArkIdentitySuccess {
        request_id: 41,
        registration_rev: 9_000,
        ark_id: [1, 2, 3, 4],
        state: RegistrationSuccessState::AlreadyApplied,
    };
    let bytes = success.encode().unwrap();
    assert_eq!(bytes.len(), 1 + 4 + 8 + 4 + 1);
    assert_eq!(bytes[0], DELIVERY_VERSION);
    let decoded = DeliveryMessage::decode(MSG_SET_ARK_IDENTITY_SUCCESS, &bytes).unwrap();
    assert_eq!(decoded, DeliveryMessage::SetArkIdentitySuccess(success));

    let error = SetArkIdentityError {
        request_id: 41,
        retryable: true,
        error_code: b"registration-commit-unknown".to_vec(),
        detail: b"commit outcome is unknown; retry the same request".to_vec(),
    };
    let bytes = error.encode().unwrap();
    assert_eq!(
        DeliveryMessage::decode(MSG_SET_ARK_IDENTITY_ERROR, &bytes).unwrap(),
        DeliveryMessage::SetArkIdentityError(error)
    );
}

#[test]
fn validation_order_preserves_signer_identity_server_and_mailbox_checks() {
    let request = request(6, 1);
    let scan = key(6).x_only_public_key().0;
    let spend = PublicKey::from_keypair(&key(7)).serialize();
    assert_eq!(
        request.error_code(&[6; 32], &scan, &spend, &[1, 2, 3, 4]),
        None
    );
    assert_eq!(
        request.error_code(&[6; 32], &scan, &spend, &[9, 9, 9, 9]),
        Some("wrong-ark-server")
    );
    assert_eq!(
        request.error_code(&[7; 32], &scan, &spend, &[1, 2, 3, 4]),
        Some("bad-signature")
    );

    let mut same_spend = request.clone();
    same_spend.b_ark = spend;
    same_spend.sign(&[6; 32], &key(6)).unwrap();
    assert_eq!(
        same_spend.error_code(&[6; 32], &scan, &spend, &[1, 2, 3, 4]),
        Some("ark-key-is-spend-key")
    );

    let mut invalid = request.clone();
    invalid.b_ark = [0; 33];
    invalid.sign(&[6; 32], &key(6)).unwrap();
    assert_eq!(
        invalid.error_code(&[6; 32], &scan, &spend, &[1, 2, 3, 4]),
        Some("invalid-point")
    );

    let mut missing = request.clone();
    missing.mailbox_pubkey.clear();
    missing.sign(&[6; 32], &key(6)).unwrap();
    assert_eq!(
        missing.error_code(&[6; 32], &scan, &spend, &[1, 2, 3, 4]),
        Some("bad-mailbox-fields")
    );
}

#[test]
fn hold_is_rejected_outside_epoch_registered_and_epoch_domain_differs() {
    let mut invalid = request(7, 1);
    invalid.epoch_fallback = EpochFallback::Hold;
    invalid.sign(&[7; 32], &key(7)).unwrap();
    assert_eq!(
        invalid.error_code(
            &[7; 32],
            &key(7).x_only_public_key().0,
            &PublicKey::from_keypair(&key(8)).serialize(),
            &[1, 2, 3, 4]
        ),
        Some("bad-mailbox-fields")
    );

    let sp_id = [7; 32];
    let identity = request(7, 1);
    let epoch = spc_ext::RegisterEpochMailboxes {
        entries: vec![],
        sig: [0; 64],
        request_id: 1,
    };
    assert_ne!(
        identity.signed_fields_digest(&sp_id).unwrap(),
        epoch.signed_fields_digest(&sp_id).unwrap()
    );
}
