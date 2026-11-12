//! Audited scheduling/durability regressions. Scripted observations below are
//! explicitly synthetic; they prove local protocol state, not funded delivery.
use super::live_receive::{EpochReadResult, MailboxBurst};
use super::maintenance::{
    queue_stock_renewal, submit_command, MaintenanceAction, NoEffectEvidence, WalletCommand,
};
use super::receive::{Expiry, ExpirySource, ObservedVtxo, VtxoState};
use super::*;
use bitcoin::hashes::Hash;
use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
use bitcoin::BlockHash;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn identity() -> StoreIdentity {
    StoreIdentity {
        wallet_fingerprint: bitcoin::bip32::Fingerprint::from([1; 4]),
        sp_id: [2; 32],
        network: "regtest".into(),
    }
}
fn command(n: u8, inputs: &[u8]) -> WalletCommand {
    WalletCommand {
        operation_id: format!("{n:064x}"),
        vtxo_ids: inputs.iter().map(|n| hex::encode([*n; 36])).collect(),
        action: MaintenanceAction::Refresh,
    }
}
fn queue_files(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|s| s == "json"))
        .collect()
}
fn select_batch(store: &WalletStore, files: &[PathBuf]) -> Vec<PathBuf> {
    let mut visited = BTreeSet::new();
    let mut out = vec![];
    for _ in 0..8 {
        if let Some(p) = store.next_command_file(files, &visited).unwrap() {
            visited.insert(p.clone());
            out.push(p);
        } else {
            break;
        }
    }
    out
}

#[test]
fn audit_r2_completed_commands_are_filtered_before_limit_and_tombstones_remain() {
    let dir = tempfile::tempdir().unwrap();
    let store = WalletStore::open(dir.path(), &identity()).unwrap();
    for n in 1..=9 {
        let c = command(n, &[n]);
        submit_command(dir.path(), &c).unwrap();
        store.reserve_maintenance(&c, Some(b"[]")).unwrap();
        if n <= 8 {
            store
                .set_operation(&c.operation_id, "round_confirmed", None, None)
                .unwrap();
        }
    }
    let files = queue_files(&dir.path().join("commands"));
    let batch = select_batch(&store, &files);
    assert_eq!(batch.len(), 1);
    assert_eq!(
        batch[0].file_stem().unwrap().to_str().unwrap(),
        command(9, &[9]).operation_id
    );
    let rows: i64 = store
        .lock()
        .query_row("SELECT count(*) FROM live_operations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 9);
    let mut changed = command(1, &[1]);
    changed.vtxo_ids = vec![hex::encode([10; 36])];
    assert!(store.reserve_maintenance(&changed, Some(b"[]")).is_err());
}

#[test]
fn audit_r2_eight_unknown_and_bad_files_cannot_starve_ninth_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = WalletStore::open(dir.path(), &identity()).unwrap();
        for n in 1..=9 {
            let c = command(n, &[n]);
            submit_command(dir.path(), &c).unwrap();
            store.reserve_maintenance(&c, Some(b"[]")).unwrap();
            store
                .set_operation(&c.operation_id, "stock_unknown", None, None)
                .unwrap();
        }
        let files = queue_files(&dir.path().join("commands"));
        assert_eq!(select_batch(&store, &files).len(), 8);
    }
    let store = WalletStore::open(dir.path(), &identity()).unwrap();
    let files = queue_files(&dir.path().join("commands"));
    assert_eq!(
        select_batch(&store, &files)[0]
            .file_stem()
            .unwrap()
            .to_str()
            .unwrap(),
        command(9, &[9]).operation_id
    );
    let bad = dir.path().join("commands/!.json");
    std::fs::write(&bad, b"invalid").unwrap();
    let files = queue_files(&dir.path().join("commands"));
    let a = select_batch(&store, &files);
    let b = select_batch(&store, &files);
    assert!(a.contains(&bad) || b.contains(&bad));
    assert!(a
        .iter()
        .chain(&b)
        .any(|p| p.file_stem().unwrap().to_str().unwrap() == command(9, &[9]).operation_id));
}

fn scan_epochs(store: &WalletStore, count: u32) {
    store.initialize_live_scan().unwrap();
    for h in 1..=count {
        store
            .commit_scan_block(h, BlockHash::from_byte_array([h as u8; 32]), true)
            .unwrap();
    }
}

#[test]
fn audit_r3_not_due_old_epochs_cannot_hide_epoch_129() {
    let dir = tempfile::tempdir().unwrap();
    let store = WalletStore::open(dir.path(), &identity()).unwrap();
    scan_epochs(&store, 129);
    store
        .lock()
        .execute("UPDATE live_scan_blocks SET reads=4 WHERE height<=128", [])
        .unwrap();
    // Last fourth read was possible at 128+6+144=278, well before tip 500.
    let due = store.due_epochs(500).unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].0, 129);
    assert!(store.due_epochs(134).unwrap().is_empty());
}

#[test]
fn audit_r3_continuations_rotate_before_network_work_and_survive_restart() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = WalletStore::open(dir.path(), &identity()).unwrap();
        scan_epochs(&store, 129);
        for (h, _, reads) in store.due_epochs(500).unwrap() {
            store.begin_epoch_read(h, reads).unwrap();
        }
    }
    let store = WalletStore::open(dir.path(), &identity()).unwrap();
    store.initialize_live_scan().unwrap();
    assert_eq!(store.due_epochs(500).unwrap()[0].0, 129);
}

fn observation(store: &WalletStore, id: u8, amount: u64) -> ObservedVtxo {
    let key = SecretKey::from_slice(&[7; 32]).unwrap();
    let public = PublicKey::from_secret_key(&Secp256k1::new(), &key);
    store.insert_u(&public, &key, 1).unwrap();
    ObservedVtxo {
        vtxo_id: [id; 36],
        u_pubkey: public,
        amount_sats: amount,
        expiry: Expiry::At {
            height: 1000,
            source: ExpirySource::VtxoAncestry,
        },
        state: VtxoState::Spendable,
    }
}

#[test]
fn audit_r5_local_dust_rejection_releases_only_own_inputs_atomically() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = std::sync::Arc::new(WalletStore::open(dir.path(), &identity()).unwrap());
        let ledger = ReceiveLedger::new(store.clone(), 96);
        for n in 1..=3 {
            ledger.record(&observation(&store, n, 100)).unwrap();
        }
        let c = command(1, &[1]);
        store.reserve_maintenance(&c, None).unwrap();
        let other = command(2, &[3]);
        store.reserve_maintenance(&other, None).unwrap();
        let dust = ark::fees::validate_and_subtract_fee_min_dust(
            bitcoin::Amount::from_sat(100),
            bitcoin::Amount::from_sat(200),
            bitcoin::Amount::from_sat(330),
        )
        .map_err(anyhow::Error::from);
        assert!(store.local_fee_result(&c.operation_id, dust).is_err());
        assert_eq!(
            store.operation_stage(&c.operation_id).unwrap(),
            "rejected_no_effect"
        );
        assert!(!store.has_uncertain_operations().unwrap());
        assert_eq!(
            store.reserved_inputs().unwrap(),
            BTreeSet::from([vec![3; 36]])
        );
    }
    let store = WalletStore::open(dir.path(), &identity()).unwrap();
    store
        .reserve_maintenance(&command(3, &[1, 2]), None)
        .unwrap();
    let original = command(1, &[1]);
    store.reserve_maintenance(&original, None).unwrap();
    assert_eq!(
        store.operation_stage(&original.operation_id).unwrap(),
        "rejected_no_effect"
    );
    assert!(store
        .reserve_maintenance(&command(1, &[1, 2]), None)
        .is_err());
}

#[test]
fn audit_r5_stock_none_is_noop_but_ambiguous_prepare_forfeit_finish_never_release() {
    let dir = tempfile::tempdir().unwrap();
    let store = WalletStore::open(dir.path(), &identity()).unwrap();
    let c = command(1, &[1]);
    store.reserve_maintenance(&c, Some(b"[]")).unwrap();
    store
        .set_operation(&c.operation_id, "stock_unknown", None, None)
        .unwrap();
    store
        .mark_stock_refresh_invocation(&c.operation_id)
        .unwrap();
    assert_eq!(
        store.finish_stock_refresh(&c.operation_id, None).unwrap(),
        "noop"
    );
    assert!(store.reserved_inputs().unwrap().is_empty());
    for (n, stage) in [
        (2, "offboard_requested"),
        (3, "round_forfeiting"),
        (4, "offboard_finishing"),
    ] {
        let c = command(n, &[n]);
        store.reserve_maintenance(&c, Some(b"[]")).unwrap();
        store
            .set_operation(&c.operation_id, stage, None, None)
            .unwrap();
        assert!(store
            .finish_no_effect(&c.operation_id, NoEffectEvidence::LocalFeeOrDust)
            .is_err());
        assert!(store
            .finish_no_effect(&c.operation_id, NoEffectEvidence::StockRefreshReturnedNone)
            .is_err());
    }
    assert_eq!(store.reserved_inputs().unwrap().len(), 3);
    assert!(store.has_uncertain_operations().unwrap());
}

#[test]
fn audit_r5_failed_release_transaction_keeps_reservations_and_no_terminal_success() {
    let dir = tempfile::tempdir().unwrap();
    let store = WalletStore::open(dir.path(), &identity()).unwrap();
    let c = command(1, &[1]);
    store.reserve_maintenance(&c, Some(b"[]")).unwrap();
    store
        .set_operation(&c.operation_id, "stock_unknown", None, None)
        .unwrap();
    store
        .mark_stock_refresh_invocation(&c.operation_id)
        .unwrap();
    store.lock().execute_batch("CREATE TRIGGER reject_release BEFORE DELETE ON live_operation_inputs BEGIN SELECT RAISE(ABORT,'mock disk failure'); END;").unwrap();
    assert!(store.finish_stock_refresh(&c.operation_id, None).is_err());
    assert_eq!(
        store.operation_stage(&c.operation_id).unwrap(),
        "stock_unknown"
    );
    assert!(store.has_uncertain_operations().unwrap());
    assert_eq!(
        store.reserved_inputs().unwrap(),
        BTreeSet::from([vec![1; 36]])
    );
    drop(store);
    let store = WalletStore::open(dir.path(), &identity()).unwrap();
    assert_eq!(
        store.operation_stage(&c.operation_id).unwrap(),
        "stock_unknown"
    );
    assert!(store
        .reserve_maintenance(&command(2, &[1]), Some(b"[]"))
        .is_err());
}

#[tokio::test]
async fn audit_r4_stock_output_of_sp_round_gets_second_unattended_refresh() {
    let dir = tempfile::tempdir().unwrap();
    let store = WalletStore::open(dir.path(), &identity()).unwrap();
    std::fs::create_dir(dir.path().join("commands")).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(
        dir.path().join("commands"),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let first = command(1, &[1]);
    store.reserve_maintenance(&first, Some(b"[]")).unwrap();
    store
        .set_operation(&first.operation_id, "round_confirmed", None, None)
        .unwrap();
    let stock_output = ark::VtxoId::from_slice(&[2; 36]).unwrap();
    // Scripted stock selector: the first SP round has imported output 2 into
    // the stock persister, not the SP ledger. Its own expiry policy becomes due.
    assert_eq!(
        queue_stock_renewal(&store, &dir.path().join("commands"), 500, |_| async {
            Ok(vec![])
        })
        .await
        .unwrap(),
        0
    );
    assert_eq!(
        queue_stock_renewal(
            &store,
            &dir.path().join("commands"),
            900,
            |excluded| async move {
                assert!(!excluded.contains(&stock_output));
                Ok(vec![stock_output])
            }
        )
        .await
        .unwrap(),
        1
    );
    let files = queue_files(&dir.path().join("commands"));
    let c: WalletCommand = serde_json::from_slice(&std::fs::read(&files[0]).unwrap()).unwrap();
    assert_eq!(c.vtxo_ids, [hex::encode([2; 36])]);
    store.reserve_maintenance(&c, Some(b"[]")).unwrap();
    store
        .set_operation(&c.operation_id, "stock_unknown", None, None)
        .unwrap();
    store
        .mark_stock_refresh_invocation(&c.operation_id)
        .unwrap();
    store
        .finish_stock_refresh(&c.operation_id, Some("mock_second_round".into()))
        .unwrap();
    assert_eq!(
        queue_stock_renewal(
            &store,
            &dir.path().join("commands"),
            901,
            |excluded| async move {
                assert!(excluded.contains(&stock_output));
                Ok(vec![stock_output])
            }
        )
        .await
        .unwrap(),
        0
    );
    let manual = command(10, &[3]);
    submit_command(dir.path(), &manual).unwrap();
    let manual_id = ark::VtxoId::from_slice(&[3; 36]).unwrap();
    assert_eq!(
        queue_stock_renewal(
            &store,
            &dir.path().join("commands"),
            902,
            |excluded| async move {
                assert!(excluded.contains(&manual_id));
                Ok(vec![manual_id])
            }
        )
        .await
        .unwrap(),
        0
    );
}

#[test]
fn audit_r6_eleven_pages_resume_after_restart_without_retry_or_epoch_loss() {
    let dir = tempfile::tempdir().unwrap();
    let mailbox = [8; 33];
    let hash = BlockHash::from_byte_array([1; 32]);
    {
        let store = std::sync::Arc::new(WalletStore::open(dir.path(), &identity()).unwrap());
        scan_epochs(&store, 1);
        store.begin_epoch_read(1, 0).unwrap();
        let ledger = ReceiveLedger::new(store.clone(), 96);
        let mut burst = MailboxBurst::default();
        for page in 1..=10 {
            let v = observation(&store, page, 100);
            store
                .persist_live_receipt(
                    &v.vtxo_id,
                    b"synthetic verified-page proof",
                    1,
                    &hash.to_byte_array(),
                    &mailbox,
                )
                .unwrap();
            ledger.record(&v).unwrap();
            store
                .advance_mailbox_checkpoint(&mailbox, i64::from(page))
                .unwrap();
            let result = burst.committed_page(1, true).unwrap();
            if page == 10 {
                let result = result.unwrap();
                assert!(result.continuation);
                store.record_epoch_read(1, 0, result).unwrap();
            } else {
                assert!(result.is_none());
            }
        }
        assert_eq!(
            store
                .lock()
                .query_row(
                    "SELECT reads,closed FROM live_scan_blocks WHERE height=1",
                    [],
                    |r| Ok((r.get::<_, u32>(0)?, r.get::<_, bool>(1)?))
                )
                .unwrap(),
            (0, false)
        );
    }
    let store = std::sync::Arc::new(WalletStore::open(dir.path(), &identity()).unwrap());
    store.initialize_live_scan().unwrap();
    assert_eq!(store.mailbox_checkpoint(&mailbox).unwrap(), 10);
    assert_eq!(store.due_epochs(7).unwrap()[0].0, 1);
    let ledger = ReceiveLedger::new(store.clone(), 96);
    let v = observation(&store, 11, 100);
    store
        .persist_live_receipt(
            &v.vtxo_id,
            b"synthetic verified-page proof",
            1,
            &hash.to_byte_array(),
            &mailbox,
        )
        .unwrap();
    ledger.record(&v).unwrap();
    // Crash/replay after ledger but before checkpoint: duplicate is not another credit.
    assert_eq!(
        ledger.record(&v).unwrap(),
        super::state::RecordWrite::AlreadyPresent
    );
    let mut invalid = v.clone();
    invalid.u_pubkey =
        PublicKey::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&[9; 32]).unwrap());
    assert!(ledger.record(&invalid).is_err());
    assert_eq!(store.mailbox_checkpoint(&mailbox).unwrap(), 10);
    store.advance_mailbox_checkpoint(&mailbox, 11).unwrap();
    let result = MailboxBurst::default()
        .committed_page(0, false)
        .unwrap()
        .unwrap();
    store.record_epoch_read(1, 0, result).unwrap();
    // Exhaustion consumes one scheduled read; neither candidate proof pages
    // nor old ledger observations prove the entire aggregate was allocated.
    let due = store.due_epochs(500).unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!((due[0].0, due[0].2), (1, 1));
    assert_eq!(store.mailbox_checkpoint(&mailbox).unwrap(), 11);
    assert_eq!(
        store
            .lock()
            .query_row("SELECT count(*) FROM live_epoch_continuations", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        0
    );
    assert_eq!(store.vtxo_records().unwrap().len(), 11);
    let bounded = EpochReadResult {
        received: 0,
        continuation: true,
    };
    assert!(bounded.continuation);
}
