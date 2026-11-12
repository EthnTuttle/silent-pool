#![cfg(feature = "ark")]
use bitcoin::hashes::Hash;
use bitcoin::BlockHash;
use spc_agent::wallet::{StoreIdentity, WalletStore};

fn identity() -> StoreIdentity {
    StoreIdentity {
        wallet_fingerprint: bitcoin::bip32::Fingerprint::from([1; 4]),
        sp_id: [2; 32],
        network: "regtest".into(),
    }
}

#[test]
fn canonical_cursor_rewind_is_journaled_and_restart_replays_contiguously() {
    let dir = tempfile::tempdir().unwrap();
    let first = BlockHash::from_byte_array([1; 32]);
    {
        let store = WalletStore::open(dir.path(), &identity()).unwrap();
        store.initialize_live_scan().unwrap();
        store.commit_scan_block(1, first, false).unwrap();
        store
            .commit_scan_block(2, BlockHash::from_byte_array([2; 32]), true)
            .unwrap();
        assert!(store
            .rewind_live_scan(1, BlockHash::from_byte_array([9; 32]))
            .is_err());
        assert_eq!(store.last_scan_height().unwrap(), Some(2));
        store.rewind_live_scan(1, first).unwrap();
    }
    let store = WalletStore::open(dir.path(), &identity()).unwrap();
    store.initialize_live_scan().unwrap();
    assert_eq!(store.last_scan_height().unwrap(), Some(1));
    assert!(store
        .commit_scan_block(3, BlockHash::from_byte_array([3; 32]), false)
        .is_err());
    store
        .commit_scan_block(2, BlockHash::from_byte_array([4; 32]), false)
        .unwrap();
    assert_eq!(
        store.live_scan_blocks().unwrap()[1].1,
        BlockHash::from_byte_array([4; 32])
    );
    let db = rusqlite::Connection::open(dir.path().join("state.sqlite")).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM live_scan_rewinds", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn legacy_height_only_cursor_cannot_become_canonical_proof() {
    let dir = tempfile::tempdir().unwrap();
    let store = WalletStore::open(dir.path(), &identity()).unwrap();
    store.set_scan_height(123).unwrap();
    assert!(store.initialize_live_scan().is_err());
    assert_eq!(store.last_scan_height().unwrap(), Some(123));
}

#[test]
fn private_command_queue_rejects_changed_payload_and_duplicate_inputs() {
    use spc_agent::wallet::maintenance::{submit_command, MaintenanceAction, WalletCommand};
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut command = WalletCommand {
        operation_id: hex::encode([1; 32]),
        vtxo_ids: vec![hex::encode([2; 36])],
        action: MaintenanceAction::Refresh,
    };
    submit_command(dir.path(), &command).unwrap();
    submit_command(dir.path(), &command).unwrap();
    command.vtxo_ids = vec![hex::encode([3; 36])];
    assert!(submit_command(dir.path(), &command).is_err());
    command.vtxo_ids.push(command.vtxo_ids[0].clone());
    assert!(command.validate().is_err());
    assert_eq!(
        std::fs::read_dir(dir.path().join("commands"))
            .unwrap()
            .count(),
        1
    );
}
