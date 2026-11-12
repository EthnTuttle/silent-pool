#![cfg(feature = "ark")]

use spc_agent::wallet::{RegistrationOutcome, StoreIdentity, WalletStore};

#[test]
fn restart_revalidation_reuses_signed_counter_and_superseded_clears_binding() {
    let dir = tempfile::tempdir().unwrap();
    let identity = StoreIdentity {
        wallet_fingerprint: bitcoin::bip32::Fingerprint::from([1; 4]),
        sp_id: [2; 32],
        network: "regtest".into(),
    };
    let digest = [3; 32];
    {
        let store = WalletStore::open(dir.path(), &identity).unwrap();
        assert_eq!(store.reserve_registration_request_id(&digest).unwrap(), 1);
        store
            .record_registration_outcome(&identity.sp_id, 1, &digest, &RegistrationOutcome::Applied)
            .unwrap();
    }
    let store = WalletStore::open(dir.path(), &identity).unwrap();
    store.reserve_registration_revalidation(&digest).unwrap();
    assert_eq!(store.reserve_registration_request_id(&digest).unwrap(), 1);
    store
        .record_registration_outcome(
            &identity.sp_id,
            1,
            &digest,
            &RegistrationOutcome::Superseded,
        )
        .unwrap();
    assert!(!store.is_registered(&digest).unwrap());
    assert_eq!(store.pending_registration_request_id().unwrap(), None);
    assert_eq!(store.reserve_registration_request_id(&digest).unwrap(), 2);
}

#[test]
fn fake_ack_cannot_change_reserved_payload_or_identity() {
    let dir = tempfile::tempdir().unwrap();
    let identity = StoreIdentity {
        wallet_fingerprint: bitcoin::bip32::Fingerprint::from([1; 4]),
        sp_id: [2; 32],
        network: "regtest".into(),
    };
    let store = WalletStore::open(dir.path(), &identity).unwrap();
    let digest = [3; 32];
    let id = store.reserve_registration_request_id(&digest).unwrap();
    assert!(store
        .record_registration_outcome(&identity.sp_id, id, &[4; 32], &RegistrationOutcome::Applied)
        .is_err());
    assert!(store
        .record_registration_outcome(&[5; 32], id, &digest, &RegistrationOutcome::Applied)
        .is_err());
    assert!(!store.is_registered(&digest).unwrap());
    assert_eq!(store.pending_registration_request_id().unwrap(), Some(id));
}

#[test]
fn receipt_bytes_deduplicate_without_advancing_mailbox_on_failure() {
    let dir = tempfile::tempdir().unwrap();
    let identity = StoreIdentity {
        wallet_fingerprint: bitcoin::bip32::Fingerprint::from([1; 4]),
        sp_id: [2; 32],
        network: "regtest".into(),
    };
    let store = WalletStore::open(dir.path(), &identity).unwrap();
    store.initialize_live_receipts().unwrap();
    let id = [3; 36];
    let mailbox = [4; 33];
    // Persistence-only synthetic bytes, never claimed as a verified VTXO.
    store
        .persist_live_receipt(&id, b"proof", 7, &[8; 32], &mailbox)
        .unwrap();
    store
        .persist_live_receipt(&id, b"proof", 7, &[8; 32], &mailbox)
        .unwrap();
    assert!(store
        .persist_live_receipt(&id, b"other", 7, &[8; 32], &mailbox)
        .is_err());
    assert_eq!(store.mailbox_checkpoint(&mailbox).unwrap(), 0);
    assert!(
        store.vtxo_records().unwrap().is_empty(),
        "proof metadata is not a second funds ledger"
    );
}
