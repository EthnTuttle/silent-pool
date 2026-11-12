//! Offline core conformance: mined header fixtures are linkage evidence,
//! not live-node canonicality or native-funding acceptance evidence.
use bitcoin::block::{Header, Version};
use bitcoin::consensus::{deserialize, serialize};
use bitcoin::hashes::Hash;
use bitcoin::{BlockHash, CompactTarget, OutPoint, Transaction, TxMerkleNode, Txid};
use spc_lab_txd::builder::{DestinationSpec, PaymentCategory};
use spc_lab_txd::journal::{
    FundingContext, FundingObservation, FundingSource, JournalRecord, OutpointStatus,
    TrafficJournal, TxState, WriteFault, MAX_ROLLBACK_HEADERS,
};
use spc_lab_txd::keys::{Chain, WalletRole};
use spc_lab_txd::wallet::{
    PaymentOutcome, PaymentRequest, ProvisionConfig, SignedPaymentView, TrafficWalletSet,
};
use spc_lab_txd::{ChainAnchor, RollbackEvidence};

const PAYER: WalletRole = WalletRole::BootstrapFunding;
const DESTINATION: WalletRole = WalletRole::NetworkWallet { index: 0 };

fn config() -> ProvisionConfig {
    ProvisionConfig::standard(0x5452_4146_4649_4321, "history3-fixture", "txd-0")
}

fn fixture(dir: &std::path::Path) -> TrafficWalletSet {
    let allocation = ProvisionConfig::validate_allocation(&[config()]).unwrap();
    TrafficWalletSet::provision(&allocation, "txd-0", dir).unwrap()
}

fn reopen(dir: &std::path::Path) -> Result<TrafficWalletSet, spc_lab_txd::error::Error> {
    let allocation = ProvisionConfig::validate_allocation(&[config()]).unwrap();
    TrafficWalletSet::reopen(&allocation, "txd-0", dir)
}

fn header(previous: BlockHash, marker: u32) -> Header {
    let mut h = Header {
        version: Version::ONE,
        prev_blockhash: previous,
        merkle_root: TxMerkleNode::from_byte_array([marker as u8; 32]),
        time: 1_700_000_000 + marker,
        bits: CompactTarget::from_consensus(0x207fffff),
        nonce: 0,
    };
    while h.validate_pow(h.target()).is_err() {
        h.nonce += 1;
    }
    h
}

fn chain(height: usize) -> Vec<Header> {
    let mut result = vec![header(BlockHash::all_zeros(), 0)];
    for h in 1..=height {
        result.push(header(result[h - 1].block_hash(), h as u32));
    }
    result
}

fn anchor(chain: &[Header], height: usize) -> ChainAnchor {
    ChainAnchor {
        block_hash: chain[height].block_hash(),
        height: height as u64,
    }
}

fn rollback(old: &[Header], new: &[Header], common: usize) -> RollbackEvidence {
    RollbackEvidence {
        version: 1,
        old_anchor: anchor(old, old.len() - 1),
        new_anchor: anchor(new, new.len() - 1),
        common_ancestor: anchor(old, common),
        old_headers: old[common..]
            .iter()
            .rev()
            .map(|h| hex::encode(serialize(h)))
            .collect(),
        new_headers: new[common..]
            .iter()
            .rev()
            .map(|h| hex::encode(serialize(h)))
            .collect(),
        displaced_block_hashes: old[common + 1..]
            .iter()
            .rev()
            .map(Header::block_hash)
            .collect(),
    }
}

fn observe_tip(set: &TrafficWalletSet, chain: &[Header]) {
    set.journal()
        .record_chain_observed(
            chain.last().unwrap().block_hash(),
            (chain.len() - 1) as u64,
            &[],
        )
        .unwrap();
}

fn funding(
    set: &TrafficWalletSet,
    hash: BlockHash,
    height: u64,
    coinbase: bool,
    marker: u8,
) -> FundingObservation {
    let observation = FundingObservation {
        outpoint: OutPoint::new(Txid::from_byte_array([marker; 32]), 0),
        amount_sats: 1_000_000,
        chain: Chain::Receive,
        index: set.journal().next_receive_index(&PAYER),
        owner_role: PAYER,
        source: if coinbase {
            FundingSource::BootstrapCoinbase
        } else {
            FundingSource::CanonicalBlock
        },
        context: FundingContext::CanonicalBlock {
            block_hash: hash.to_string(),
            height,
            coinbase,
        },
    };
    set.observe_funding(&observation).unwrap();
    observation
}

fn request(set: &TrafficWalletSet, id: &str) -> PaymentRequest {
    PaymentRequest {
        command_id: id.into(),
        payer_role: PAYER,
        category: PaymentCategory::Ordinary,
        destinations: vec![DestinationSpec {
            script_pubkey: set
                .next_receive_address(&DESTINATION)
                .unwrap()
                .script_pubkey,
            amount_sats: 100_000,
            owner: Some(DESTINATION),
        }],
        fee_rate_sats_vb: 3,
    }
}

fn sign(set: &TrafficWalletSet, id: &str) -> SignedPaymentView {
    match set.sign_payment(&request(set, id)).unwrap() {
        PaymentOutcome::Signed(view) => view,
        _ => panic!("expected new signed transaction"),
    }
}

fn confirm(set: &TrafficWalletSet, id: &str, block: &Header, height: u64) {
    set.note_broadcast_sent(id).unwrap();
    set.note_accepted_by_rpc(id, "fixture-node").unwrap();
    set.note_confirmed(id, block.block_hash(), height).unwrap();
}

fn own_change(set: &TrafficWalletSet, view: &SignedPaymentView) -> OutPoint {
    let tx: Transaction = deserialize(&hex::decode(&view.raw_hex).unwrap()).unwrap();
    let outpoint = OutPoint::new(view.txid, 1);
    set.observe_funding(&FundingObservation {
        outpoint,
        amount_sats: tx.output[1].value.to_sat(),
        chain: Chain::Change,
        index: view.change_index.unwrap(),
        owner_role: PAYER,
        source: FundingSource::OwnTransaction,
        context: FundingContext::OwnTransaction,
    })
    .unwrap();
    outpoint
}

#[test]
fn snapshot_is_detached_coherent_and_preserves_valid_invalidated_and_retained_statuses() {
    let dir = tempfile::tempdir().unwrap();
    let set = fixture(dir.path());
    let blocks = chain(100);
    observe_tip(&set, &blocks);
    let retained_script = set.next_receive_address(&PAYER).unwrap().script_pubkey;
    let retained = funding(&set, blocks[0].block_hash(), 0, true, 1);
    let external = funding(&set, blocks[100].block_hash(), 100, false, 2);
    let before_bytes = std::fs::read(set.journal().path()).unwrap();
    let before = set.journal().observer_snapshot();
    assert_eq!(std::fs::read(set.journal().path()).unwrap(), before_bytes);
    assert!(!before.outpoints[&retained.outpoint].coinbase_immature);
    assert_eq!(
        before.outpoints[&external.outpoint].status,
        OutpointStatus::Available
    );
    set.journal()
        .record_chain_rollback(rollback(&blocks, &blocks[..100], 99))
        .unwrap();
    let after = set.journal().observer_snapshot();
    assert_eq!(after.committed_seq, before.committed_seq + 1);
    assert_eq!(before.tip, Some(anchor(&blocks, 100)));
    assert_eq!(after.tip, Some(anchor(&blocks, 99)));
    assert_eq!(
        after.outpoints[&retained.outpoint].status,
        OutpointStatus::Available
    );
    assert!(after.outpoints[&retained.outpoint].coinbase_immature);
    assert!(matches!(
        after.outpoints[&external.outpoint].status,
        OutpointStatus::Invalidated { .. }
    ));
    let mut detached = after.clone();
    detached.outpoints.clear();
    assert_eq!(set.journal().observer_snapshot(), after);
    let keys: Vec<_> = after.outpoints.keys().copied().collect();
    assert!(keys.windows(2).all(|w| w[0] < w[1]));
    let facts = &after.outpoints[&retained.outpoint];
    assert_eq!(facts.amount_sats, retained.amount_sats);
    assert_eq!(facts.owner, PAYER);
    assert_eq!(facts.chain, retained.chain);
    assert_eq!(facts.index, retained.index);
    assert_eq!(facts.source, retained.source);
    assert_eq!(facts.context, retained.context);
    assert_eq!(facts.script, retained_script);
}

#[test]
fn lower_tip_is_one_atomic_record_and_replays_exact_cascade() {
    let dir = tempfile::tempdir().unwrap();
    let set = fixture(dir.path());
    let blocks = chain(4);
    observe_tip(&set, &blocks);
    funding(&set, blocks[0].block_hash(), 0, false, 3);
    let parent = sign(&set, "parent");
    confirm(&set, "parent", &blocks[3], 3);
    let change = own_change(&set, &parent);
    let child = sign(&set, "child");
    let before = set.journal().last_seq();
    let evidence = rollback(&blocks, &blocks[..3], 2);
    set.journal()
        ._for_tests_set_write_fault(WriteFault::PartialWrite)
        .unwrap();
    let bytes = std::fs::read(set.journal().path()).unwrap();
    assert!(set
        .journal()
        .record_chain_rollback(evidence.clone())
        .is_err());
    assert_eq!(std::fs::read(set.journal().path()).unwrap(), bytes);
    assert_eq!(set.journal().last_seq(), before);
    set.journal().record_chain_rollback(evidence).unwrap();
    let snapshot = set.journal().observer_snapshot();
    assert_eq!(snapshot.committed_seq, before + 1);
    assert_eq!(set.journal().tx_state(&parent.txid), Some(TxState::Reorged));
    assert_eq!(
        set.journal().tx_state(&child.txid),
        Some(TxState::Invalidated)
    );
    assert!(matches!(
        snapshot.outpoints[&change].status,
        OutpointStatus::Invalidated { .. }
    ));
    let text = std::fs::read_to_string(set.journal().path()).unwrap();
    assert!(matches!(
        serde_json::from_str::<JournalRecord>(text.lines().last().unwrap()).unwrap(),
        JournalRecord::ChainRollback { .. }
    ));
    drop(set);
    let reopened = reopen(dir.path()).unwrap();
    let mut expected = snapshot;
    expected.committed_seq += 1; // reopen's one explicit SessionStarted
    assert_eq!(reopened.journal().observer_snapshot(), expected);
    assert_eq!(
        reopened.journal().tx_state(&parent.txid),
        Some(TxState::Reorged)
    );
    assert_eq!(
        reopened.journal().tx_state(&child.txid),
        Some(TxState::Invalidated)
    );
    // S3: same-block evidence stays stale until a later chain observation.
    assert!(reopened
        .note_confirmed("parent", blocks[3].block_hash(), 3)
        .is_err());
    observe_tip(&reopened, &blocks);
    reopened
        .note_confirmed("parent", blocks[3].block_hash(), 3)
        .unwrap();
    assert_eq!(
        reopened.journal().tx_state(&child.txid),
        Some(TxState::Signed)
    );
}

#[test]
fn malformed_proofs_and_unproved_lower_tips_never_mutate() {
    let dir = tempfile::tempdir().unwrap();
    let set = fixture(dir.path());
    let blocks = chain(4);
    let mut fork = blocks[..2].to_vec();
    fork.push(header(fork[1].block_hash(), 200));
    observe_tip(&set, &blocks);
    let proof = rollback(&blocks, &fork, 1);
    let mut bad = Vec::new();
    let mut p = proof.clone();
    p.version = 2;
    bad.push(p);
    let mut p = proof.clone();
    p.old_anchor.height -= 1;
    bad.push(p);
    let mut p = proof.clone();
    p.old_anchor.block_hash = blocks[0].block_hash();
    bad.push(p);
    let mut p = proof.clone();
    p.new_anchor.block_hash = blocks[0].block_hash();
    bad.push(p);
    let mut p = proof.clone();
    p.common_ancestor.height = 0;
    bad.push(p);
    let mut p = proof.clone();
    p.common_ancestor.block_hash = blocks[0].block_hash();
    bad.push(p);
    let mut p = proof.clone();
    p.old_headers.pop();
    bad.push(p);
    let mut p = proof.clone();
    p.new_headers[0] = hex::encode(serialize(&blocks[2]));
    bad.push(p);
    let mut p = proof.clone();
    p.displaced_block_hashes.pop();
    bad.push(p);
    let mut p = proof.clone();
    p.displaced_block_hashes.push(p.displaced_block_hashes[0]);
    bad.push(p);
    let mut p = proof.clone();
    p.new_headers[0].truncate(158);
    bad.push(p);
    let mut p = proof.clone();
    p.old_anchor.height = u64::MAX;
    bad.push(p);
    let unlinked = header(BlockHash::all_zeros(), 201);
    let mut p = proof.clone();
    p.new_headers[0] = hex::encode(serialize(&unlinked));
    p.new_anchor.block_hash = unlinked.block_hash();
    bad.push(p);
    let mut easy = fork[2];
    easy.bits = CompactTarget::from_consensus(0x2100ffff);
    let mut p = proof.clone();
    p.new_headers[0] = hex::encode(serialize(&easy));
    p.new_anchor.block_hash = easy.block_hash();
    bad.push(p);
    let mut invalid_pow = fork[2];
    while invalid_pow.validate_pow(invalid_pow.target()).is_ok() {
        invalid_pow.nonce += 1;
    }
    let mut p = proof.clone();
    p.new_headers[0] = hex::encode(serialize(&invalid_pow));
    p.new_anchor.block_hash = invalid_pow.block_hash();
    bad.push(p);
    let bytes = std::fs::read(set.journal().path()).unwrap();
    let snapshot = set.journal().observer_snapshot();
    for p in bad {
        assert!(set.journal().record_chain_rollback(p).is_err());
        assert_eq!(set.journal().observer_snapshot(), snapshot);
        assert_eq!(std::fs::read(set.journal().path()).unwrap(), bytes);
    }
    assert!(set
        .journal()
        .record_chain_observed(fork[2].block_hash(), 2, &proof.displaced_block_hashes)
        .is_err());
    set.journal().record_chain_rollback(proof).unwrap();
    assert_eq!(set.journal().tip_height(), Some(2));
}

#[test]
fn retained_coinbase_blocks_signed_unknown_retry_and_queued_intent_until_real_maturity() {
    let dir = tempfile::tempdir().unwrap();
    let set = fixture(dir.path());
    let blocks = chain(100);
    observe_tip(&set, &blocks);
    let coin = funding(&set, blocks[0].block_hash(), 0, true, 4);
    let view = sign(&set, "send");
    let reservation = set.journal().observer_snapshot().outpoints[&coin.outpoint]
        .status
        .clone();
    let proof = rollback(&blocks, &blocks[..100], 99);
    set.journal().record_chain_rollback(proof.clone()).unwrap();
    let seq = set.journal().last_seq();
    assert!(set.note_broadcast_sent("send").is_err());
    assert_eq!(set.journal().last_seq(), seq);
    assert_eq!(
        set.journal().observer_snapshot().outpoints[&coin.outpoint].status,
        reservation
    );
    assert_eq!(set.journal().tx_state(&view.txid), Some(TxState::Signed));
    observe_tip(&set, &blocks);
    set.note_broadcast_sent("send").unwrap();
    let intent = set.journal().pending_send_intent("send").unwrap();
    let permitted_seq = set.journal().last_seq();
    set.journal()
        .check_send_before_forward(&intent, permitted_seq)
        .unwrap();
    set.journal().record_chain_rollback(proof.clone()).unwrap();
    assert!(set
        .journal()
        .check_send_before_forward(&intent, permitted_seq)
        .is_err());
    assert!(set
        .journal()
        .check_send_before_forward(&intent, set.journal().last_seq())
        .is_err());
    set.note_unknown("send", "withheld locally; no node outcome known")
        .unwrap();
    assert!(set.note_broadcast_sent("send").is_err());
    observe_tip(&set, &blocks);
    set.note_broadcast_sent("send").unwrap();
    assert!(set
        .journal()
        .check_send_before_forward(&intent, set.journal().last_seq())
        .is_err());
    assert_eq!(set.journal().tx_facts(&view.txid).unwrap().retry_sends, 1);
    // A detached intent from another journal/run cannot pass binding.
    let other_dir = tempfile::tempdir().unwrap();
    let other = fixture(other_dir.path());
    assert!(other
        .journal()
        .check_send_before_forward(&intent, other.journal().last_seq())
        .is_err());
}

#[test]
fn resubmit_checks_retained_coinbase_maturity_and_pause_without_consuming_caps() {
    let dir = tempfile::tempdir().unwrap();
    let set = fixture(dir.path());
    let blocks = chain(101);
    observe_tip(&set, &blocks);
    let coin = funding(&set, blocks[0].block_hash(), 0, true, 5);
    let view = sign(&set, "send");
    confirm(&set, "send", &blocks[101], 101);
    set.journal()
        .record_chain_rollback(rollback(&blocks, &blocks[..100], 99))
        .unwrap();
    let raw = hex::decode(&view.raw_hex).unwrap();
    let seq = set.journal().last_seq();
    assert!(set
        .journal()
        .record_reorg_resubmit("send", &raw, &[coin.outpoint])
        .is_err());
    assert_eq!(set.journal().last_seq(), seq);
    observe_tip(&set, &blocks[..101]);
    set.journal().record_send_barrier("pause", 1).unwrap();
    assert!(set
        .journal()
        .record_reorg_resubmit("send", &raw, &[coin.outpoint])
        .is_err());
    set.journal().record_send_resumed("resume").unwrap();
    set.journal()
        .record_reorg_resubmit("send", &raw, &[coin.outpoint])
        .unwrap();
    assert_eq!(
        set.journal().tx_facts(&view.txid).unwrap().reorg_resubmits,
        1
    );
    set.note_unknown("send", "already forwarded timeout")
        .unwrap();
    assert_eq!(set.journal().tx_state(&view.txid), Some(TxState::Unknown));
}

#[test]
fn displaced_external_reobserves_but_own_change_restores_only_through_creator() {
    let dir = tempfile::tempdir().unwrap();
    let set = fixture(dir.path());
    let blocks = chain(3);
    observe_tip(&set, &blocks);
    let mut external = funding(&set, blocks[2].block_hash(), 2, false, 6);
    let parent = sign(&set, "parent");
    confirm(&set, "parent", &blocks[3], 3);
    let change = own_change(&set, &parent);
    set.journal()
        .record_chain_rollback(rollback(&blocks, &blocks[..2], 1))
        .unwrap();
    external.context = FundingContext::CanonicalBlock {
        block_hash: blocks[1].block_hash().to_string(),
        height: 1,
        coinbase: false,
    };
    set.observe_funding(&external).unwrap();
    assert_eq!(set.journal().tx_state(&parent.txid), Some(TxState::Reorged));
    assert!(matches!(
        set.journal().observer_snapshot().outpoints[&change].status,
        OutpointStatus::Invalidated { .. }
    ));
    assert!(set.observe_funding(&external).is_err()); // valid duplicate
    observe_tip(&set, &blocks);
    set.note_confirmed("parent", blocks[3].block_hash(), 3)
        .unwrap();
    assert_eq!(
        set.journal().observer_snapshot().outpoints[&change].status,
        OutpointStatus::Available
    );
}

#[test]
fn proof_replay_rejects_complete_bad_records_even_without_newline_and_bounds_decode() {
    let dir = tempfile::tempdir().unwrap();
    let set = fixture(dir.path());
    let blocks = chain(3);
    observe_tip(&set, &blocks);
    let prefix = std::fs::read(set.journal().path()).unwrap();
    let record = JournalRecord::ChainRollback {
        seq: set.journal().last_seq() + 1,
        evidence: rollback(&blocks, &blocks[..2], 1),
        observed_unix_ms: 0,
    };
    let value = serde_json::to_value(&record).unwrap();
    drop(set);
    let path = TrafficJournal::path_in(dir.path());
    for mutation in 0..10 {
        let mut bad = value.clone();
        match mutation {
            0 => {
                bad.as_object_mut().unwrap().remove("evidence");
            }
            1 => bad["evidence"]["version"] = 2.into(),
            2 => bad["evidence"]["extra"] = true.into(),
            3 => bad["extra"] = true.into(),
            4 => bad["evidence"]["new_headers"][0] = "00".into(),
            5 => bad["evidence"]["displaced_block_hashes"] = serde_json::json!([]),
            6 => bad["evidence"]["old_anchor"]["height"] = 2.into(),
            7 => {
                bad["evidence"]["common_ancestor"]["block_hash"] =
                    blocks[0].block_hash().to_string().into()
            }
            8 => {
                bad["evidence"]["old_headers"].as_array_mut().unwrap().pop();
            }
            _ => {
                bad["evidence"]["old_anchor"]["block_hash"] =
                    blocks[0].block_hash().to_string().into()
            }
        }
        for terminated in [true, false] {
            let mut bytes = prefix.clone();
            bytes.extend(serde_json::to_vec(&bad).unwrap());
            if terminated {
                bytes.push(b'\n');
            }
            std::fs::write(&path, &bytes).unwrap();
            assert!(
                reopen(dir.path()).is_err(),
                "mutation {mutation}, terminated {terminated}"
            );
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
    }
    let mut excessive = value.clone();
    excessive["evidence"]["new_headers"] = serde_json::json!(vec!["00"; MAX_ROLLBACK_HEADERS + 1]);
    assert!(serde_json::from_value::<JournalRecord>(excessive).is_err());
    let duplicate = serde_json::to_string(&record)
        .unwrap()
        .replace("\"version\":1", "\"version\":1,\"version\":1");
    assert!(serde_json::from_str::<JournalRecord>(&duplicate).is_err());
    // A recognizable truncated proof is refused, never repaired away.
    let encoded = serde_json::to_vec(&record).unwrap();
    let mut torn = prefix.clone();
    torn.extend(&encoded[..encoded.len() / 2]);
    std::fs::write(&path, &torn).unwrap();
    assert!(reopen(dir.path()).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), torn);
    // An incomplete, unidentifiable non-record prefix never authorizes a
    // rollback; the existing torn-tail rule recovers the committed tip.
    let mut torn = prefix.clone();
    torn.extend_from_slice(b"{\"rec");
    std::fs::write(&path, torn).unwrap();
    let recovered = reopen(dir.path()).unwrap();
    assert_eq!(recovered.journal().tip_height(), Some(3));
    assert_eq!(std::fs::read_to_string(path).unwrap().lines().count(), 3);
}

#[test]
fn run_caps_allocation_and_history2_refusals_precede_any_tail_repair_or_session() {
    let dir = tempfile::tempdir().unwrap();
    let set = fixture(dir.path());
    let mut header: serde_json::Value =
        serde_json::from_slice(&std::fs::read(set.journal().path()).unwrap()).unwrap();
    drop(set);
    let path = TrafficJournal::path_in(dir.path());
    let original = header.clone();
    for field in [
        "run_id",
        "run_seed_fingerprint",
        "network",
        "allocation_digest",
        "caps",
        "history_version",
    ] {
        header = original.clone();
        match field {
            "run_id" => header[field] = "foreign-run".into(),
            "network" => header[field] = "bitcoin".into(),
            "run_seed_fingerprint" => header[field] = "ff".repeat(32).into(),
            "allocation_digest" => header[field] = "00".repeat(32).into(),
            "caps" => header[field]["unknown_retry_sends"] = 0.into(),
            _ => {
                header[field] = 2.into();
                header["protocol"] = "spc-lab-tx-journal/2".into();
            }
        }
        let mut bytes = serde_json::to_vec(&header).unwrap();
        bytes.extend_from_slice(b"\n{\"record\":");
        std::fs::write(&path, &bytes).unwrap();
        assert!(reopen(dir.path()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes, "{field}");
    }
}

#[test]
fn reservation_survives_lower_tip_and_resumed_signing_cannot_send_immature_inputs() {
    use spc_lab_txd::journal::{DestinationRecord, RequestRecord};
    let dir = tempfile::tempdir().unwrap();
    let set = fixture(dir.path());
    let blocks = chain(100);
    observe_tip(&set, &blocks);
    let coin = funding(&set, blocks[0].block_hash(), 0, true, 7);
    let req = request(&set, "reserved");
    set.journal()
        .begin_command(
            "reserved",
            RequestRecord {
                construction_effect: None,
                payer_role: req.payer_role.clone(),
                category: req.category.clone(),
                fee_rate_sats_vb: req.fee_rate_sats_vb,
                destinations: req
                    .destinations
                    .iter()
                    .map(|d| DestinationRecord {
                        script_hex: hex::encode(d.script_pubkey.as_bytes()),
                        amount_sats: d.amount_sats,
                        owner_role: d.owner.clone(),
                    })
                    .collect(),
            },
        )
        .unwrap();
    set.journal()
        .reserve("reserved", &[coin.outpoint], 0)
        .unwrap();
    set.journal()
        .record_chain_rollback(rollback(&blocks, &blocks[..100], 99))
        .unwrap();
    assert!(matches!(
        set.journal().observer_snapshot().outpoints[&coin.outpoint].status,
        OutpointStatus::Reserved { .. }
    ));
    // Signing a durable reservation does not authorize a send. Whether a
    // builder elects to defer signing or returns bytes, send admission blocks.
    let _ = set.sign_payment(&req);
    assert!(set.note_broadcast_sent("reserved").is_err());
    assert!(matches!(
        set.journal().observer_snapshot().outpoints[&coin.outpoint].status,
        OutpointStatus::Reserved { .. }
    ));
    observe_tip(&set, &blocks);
    set.sign_payment(&req).unwrap();
    set.note_broadcast_sent("reserved").unwrap();
}

#[test]
fn own_change_ancestry_maturity_is_rechecked_not_only_direct_input() {
    let dir = tempfile::tempdir().unwrap();
    let set = fixture(dir.path());
    let blocks = chain(100);
    observe_tip(&set, &blocks);
    funding(&set, blocks[0].block_hash(), 0, true, 8);
    let parent = sign(&set, "parent");
    // Adversarial observer fact: the existing confirmation API trusts the
    // observer's block inclusion (no raw block argument). This artificially
    // early inclusion isolates the ancestry guard from displacement. It is
    // not claimed as a valid Bitcoin block or live canonical evidence.
    confirm(&set, "parent", &blocks[99], 99);
    own_change(&set, &parent);
    let child = sign(&set, "child");
    set.journal()
        .record_chain_rollback(rollback(&blocks, &blocks[..100], 99))
        .unwrap();
    assert_eq!(
        set.journal().tx_state(&parent.txid),
        Some(TxState::Confirmed)
    );
    assert_eq!(set.journal().tx_state(&child.txid), Some(TxState::Signed));
    assert!(set.note_broadcast_sent("child").is_err());
    observe_tip(&set, &blocks);
    set.note_broadcast_sent("child").unwrap();
}

#[test]
fn concurrent_observer_copies_never_mix_tip_and_sequence() {
    let dir = tempfile::tempdir().unwrap();
    let set = fixture(dir.path());
    let blocks = chain(16);
    observe_tip(&set, &blocks[..1]);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            for height in 1..blocks.len() {
                observe_tip(&set, &blocks[..=height]);
            }
        });
        for _ in 0..100 {
            let snapshot = set.journal().observer_snapshot();
            let tip = snapshot.tip.unwrap();
            assert_eq!(snapshot.committed_seq, tip.height + 1);
            assert_eq!(tip.block_hash, blocks[tip.height as usize].block_hash());
        }
    });
}

#[test]
fn maturity_overflow_cannot_make_a_coinbase_available() {
    let dir = tempfile::tempdir().unwrap();
    let set = fixture(dir.path());
    let hash = header(BlockHash::all_zeros(), 1000).block_hash();
    set.journal()
        .record_chain_observed(hash, u64::MAX, &[])
        .unwrap();
    let coin = funding(&set, hash, u64::MAX - 1, true, 9);
    assert!(set.journal().observer_snapshot().outpoints[&coin.outpoint].coinbase_immature);
    assert_eq!(set.balance_sats(&PAYER), 0);
    assert!(set.sign_payment(&request(&set, "overflow")).is_err());
}
