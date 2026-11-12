//! Recovery API conformance using the existing signer and journal transitions.
//! Canonical hashes here are offline core fixtures, not funded runtime proof.
use bitcoin::hashes::Hash;
use bitcoin::{BlockHash, OutPoint, Txid};
use spc_lab_txd::builder::{DestinationSpec, PaymentCategory};
use spc_lab_txd::journal::{
    CommandPhase, FundingContext, FundingObservation, FundingSource, TxState,
};
use spc_lab_txd::keys::{Chain, WalletRole};
use spc_lab_txd::wallet::{PaymentOutcome, PaymentRequest, ProvisionConfig, TrafficWalletSet};

fn config() -> ProvisionConfig {
    ProvisionConfig::standard(0x5452_4146_4649_4321, "history3-fixture", "txd-0")
}
fn open(path: &std::path::Path, reopen: bool) -> TrafficWalletSet {
    let allocation = ProvisionConfig::validate_allocation(&[config()]).unwrap();
    if reopen {
        TrafficWalletSet::reopen(&allocation, "txd-0", path)
    } else {
        TrafficWalletSet::provision(&allocation, "txd-0", path)
    }
    .unwrap()
}
fn sign(wallet: &TrafficWalletSet, id: &str, marker: u8) -> Txid {
    wallet
        .observe_funding(&FundingObservation {
            outpoint: OutPoint::new(Txid::from_byte_array([marker; 32]), 0),
            amount_sats: 1_000_000,
            chain: Chain::Receive,
            index: wallet
                .journal()
                .next_receive_index(&WalletRole::BootstrapFunding),
            owner_role: WalletRole::BootstrapFunding,
            source: FundingSource::CanonicalBlock,
            context: FundingContext::CanonicalBlock {
                block_hash: BlockHash::from_byte_array([1; 32]).to_string(),
                height: 1,
                coinbase: false,
            },
        })
        .unwrap();
    let request = PaymentRequest {
        command_id: id.into(),
        payer_role: WalletRole::BootstrapFunding,
        category: PaymentCategory::Ordinary,
        destinations: vec![DestinationSpec {
            script_pubkey: wallet
                .next_receive_address(&WalletRole::NetworkWallet { index: 0 })
                .unwrap()
                .script_pubkey,
            amount_sats: 100_000,
            owner: Some(WalletRole::NetworkWallet { index: 0 }),
        }],
        fee_rate_sats_vb: 3,
    };
    match wallet.sign_payment(&request).unwrap() {
        PaymentOutcome::Signed(v) => v.txid,
        _ => panic!("fresh signature expected"),
    }
}
#[test]
fn empty_detached_binding_and_session_unknown_preserve_last_send_and_barrier() {
    let dir = tempfile::tempdir().unwrap();
    let wallet = open(dir.path(), false);
    let empty = wallet.journal().recovery_snapshot();
    assert!(empty.commands.is_empty());
    assert!(empty.transactions.is_empty());
    assert_eq!(empty.next_send_sequence, 1);
    assert_eq!(empty.last_broadcast_txid, None);
    let txid = sign(&wallet, "payment", 9);
    let signed = wallet.journal().recovery_snapshot();
    assert_eq!(signed.commands["payment"].txid, Some(txid));
    assert_eq!(
        signed.commands["payment"].raw_hex.as_deref(),
        Some(signed.transactions[&txid].facts.raw_hex.as_str())
    );
    wallet.note_broadcast_sent("payment").unwrap();
    wallet.pause_sends("pause", 10).unwrap();
    let paused = wallet.journal().recovery_snapshot();
    assert!(paused.barrier_active);
    assert_eq!(paused.last_broadcast_txid, Some(txid));
    assert_eq!(paused.transactions[&txid].last_send.unwrap().0, 1);
    assert_eq!(paused.next_send_sequence, 2);
    drop(wallet);
    let wallet = open(dir.path(), true);
    let reopened = wallet.journal().recovery_snapshot();
    assert_eq!(reopened.committed_seq, paused.committed_seq + 1);
    assert!(reopened.barrier_active);
    assert_eq!(reopened.transactions[&txid].facts.state, TxState::Unknown);
    assert!(reopened.transactions[&txid].facts.ever_unknown);
    assert_eq!(
        reopened.transactions[&txid].last_send,
        paused.transactions[&txid].last_send
    );
    assert_eq!(reopened.last_broadcast_txid, Some(txid));
    assert_eq!(reopened.transactions[&txid].facts.retry_sends, 0);
    let mut detached = reopened.clone();
    detached.commands.clear();
    detached
        .transactions
        .get_mut(&txid)
        .unwrap()
        .facts
        .raw_hex
        .clear();
    assert!(!wallet.journal().recovery_snapshot().commands.is_empty());
    assert!(!wallet.journal().recovery_snapshot().transactions[&txid]
        .facts
        .raw_hex
        .is_empty());
    assert_eq!(signed.transactions[&txid].facts.state, TxState::Signed);
    assert!(wallet.note_broadcast_sent("payment").is_err());
    wallet.resume_sends("resume").unwrap();
    assert_eq!(wallet.note_broadcast_sent("payment").unwrap(), 2);
    assert_eq!(
        wallet.journal().recovery_snapshot().transactions[&txid]
            .facts
            .retry_sends,
        1
    );
}
#[test]
fn finished_reorged_commands_and_resubmission_identity_remain_enumerable() {
    let dir = tempfile::tempdir().unwrap();
    let wallet = open(dir.path(), false);
    let txid = sign(&wallet, "parent", 10);
    let a = BlockHash::from_byte_array([2; 32]);
    let b = BlockHash::from_byte_array([3; 32]);
    wallet.note_chain_observed(a, 2, &[]).unwrap();
    wallet.note_broadcast_sent("parent").unwrap();
    wallet
        .note_accepted_by_rpc("parent", "fixture-node")
        .unwrap();
    wallet.note_confirmed("parent", a, 2).unwrap();
    assert_eq!(
        wallet.journal().recovery_snapshot().commands["parent"].phase,
        CommandPhase::Finished
    );
    wallet.note_chain_observed(b, 2, &[a]).unwrap();
    let displaced = wallet.journal().recovery_snapshot();
    assert_eq!(displaced.transactions[&txid].facts.state, TxState::Reorged);
    assert!(displaced.commands.contains_key("parent"));
    assert_eq!(
        displaced.transactions[&txid].displaced_at_seq,
        Some(displaced.committed_seq)
    );
    let facts = &displaced.transactions[&txid].facts;
    wallet
        .journal()
        .record_reorg_resubmit(
            "parent",
            &hex::decode(&facts.raw_hex).unwrap(),
            &facts.spent_outpoints,
        )
        .unwrap();
    let sent = wallet.journal().recovery_snapshot();
    assert_eq!(
        sent.transactions[&txid].last_send,
        Some((2, sent.committed_seq))
    );
    assert_eq!(sent.transactions[&txid].facts.reorg_resubmits, 1);
    assert_eq!(sent.next_send_sequence, 3);
}
#[test]
fn concurrent_barrier_updates_cannot_tear_the_recovery_view() {
    let dir = tempfile::tempdir().unwrap();
    let wallet = open(dir.path(), false);
    let txid = sign(&wallet, "payment", 11);
    wallet.note_broadcast_sent("payment").unwrap();
    wallet
        .note_unknown("payment", "fixture response lost")
        .unwrap();
    std::thread::scope(|scope| {
        let writer = scope.spawn(|| {
            for i in 0..30 {
                wallet.pause_sends(&format!("pause-{i}"), i).unwrap();
                wallet.resume_sends(&format!("resume-{i}")).unwrap();
            }
        });
        for _ in 0..200 {
            let snap = wallet.journal().recovery_snapshot();
            let tx = &snap.transactions[&txid];
            assert_eq!(snap.commands[&tx.facts.command_id].txid, Some(txid));
            assert_eq!(snap.last_broadcast_txid, Some(txid));
            assert_eq!(tx.last_send.unwrap().0 + 1, snap.next_send_sequence);
            assert!(tx.last_send.unwrap().1 <= snap.committed_seq);
            assert_eq!(tx.facts.state, TxState::Unknown);
            assert_eq!(
                snap.commands["payment"].raw_hex.as_deref(),
                Some(tx.facts.raw_hex.as_str())
            );
        }
        writer.join().unwrap();
    });
}
