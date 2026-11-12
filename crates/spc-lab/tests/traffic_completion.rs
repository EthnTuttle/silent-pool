//! Explicit offline raw-node fixtures. Passing this verifier is not native T2
//! funding acceptance: the fixture builds headers locally and supplies no server.
use bitcoin::consensus::{deserialize, serialize};
use bitcoin::{Amount, Block, BlockHash, OutPoint, Sequence, Transaction, TxIn, TxOut, Witness};
use serde_json::{json, Value};
use spc_lab_txd::actor_config::{AllocationActor, AllocationDescription};
use spc_lab_txd::builder::{DestinationSpec, PaymentCategory};
use spc_lab_txd::journal::{FundingContext, FundingObservation, FundingSource};
use spc_lab_txd::keys::{Chain, TrafficWallet, WalletRole};
use spc_lab_txd::observer::{CanonicalIndex, ChainPolicy, RawRpc};
use spc_lab_txd::wallet::{PaymentOutcome, PaymentRequest, TrafficWalletSet};

struct FixtureNode {
    blocks: Vec<Block>,
}
impl RawRpc for FixtureNode {
    fn read(&mut self, _node: &str, method: &str, params: Value) -> Result<Value, String> {
        match method {
            "getbestblockhash" => Ok(json!(self.blocks.last().unwrap().block_hash().to_string())),
            "getblock" => self
                .blocks
                .iter()
                .find(|b| Some(b.block_hash().to_string().as_str()) == params[0].as_str())
                .map(|b| json!(hex::encode(serialize(b))))
                .ok_or("fixture block missing".into()),
            _ => Err("fixture method unsupported".into()),
        }
    }
}
fn block(
    previous: &Block,
    height: u32,
    script: bitcoin::ScriptBuf,
    extra: Vec<Transaction>,
    fees: u64,
) -> Block {
    let coinbase = Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: bitcoin::absolute::LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::null(),
            script_sig: bitcoin::script::Builder::new()
                .push_int(height as i64)
                .push_int(0)
                .into_script(),
            sequence: Sequence::MAX,
            witness: Witness::default(),
        }],
        output: vec![TxOut {
            value: Amount::from_sat(5_000_000_000 + fees),
            script_pubkey: script,
        }],
    };
    let mut txdata = vec![coinbase];
    txdata.extend(extra);
    let mut result = Block {
        header: bitcoin::block::Header {
            version: bitcoin::block::Version::ONE,
            prev_blockhash: previous.block_hash(),
            merkle_root: previous.header.merkle_root,
            time: previous.header.time + 1,
            bits: bitcoin::CompactTarget::from_consensus(0x207fffff),
            nonce: 0,
        },
        txdata,
    };
    result.header.merkle_root = result.compute_merkle_root().unwrap();
    while result.header.validate_pow(result.header.target()).is_err() {
        result.header.nonce += 1;
    }
    result
}
#[test]
fn real_raw_fixture_distribution_requires_all_routes_and_common_nodes() {
    let allocation = AllocationDescription {
        run_seed: 42,
        run_id: "completion-fixture".into(),
        network: bitcoin::Network::Regtest,
        caps: Default::default(),
        users: vec!["alice".into(), "bob".into()],
        actors: vec![
            AllocationActor {
                driver_id: "a".into(),
                network_wallets: 1,
                user_payment_wallets: 1,
                hosts_bootstrap_funding: true,
                lookahead: 2,
                fee_policy: Default::default(),
            },
            AllocationActor {
                driver_id: "b".into(),
                network_wallets: 1,
                user_payment_wallets: 1,
                hosts_bootstrap_funding: false,
                lookahead: 2,
                fee_policy: Default::default(),
            },
        ],
    };
    let dir = std::env::temp_dir().join(format!(
        "traffic-completion-{}",
        spc_lab::ids::CommandId::generate()
    ));
    std::fs::create_dir(&dir).unwrap();
    let token = allocation.reconstruct().unwrap();
    let wallet = TrafficWalletSet::provision(&token, "a", &dir).unwrap();
    let funding = wallet
        .next_receive_address(&WalletRole::BootstrapFunding)
        .unwrap()
        .script_pubkey;
    let genesis = bitcoin::blockdata::constants::genesis_block(bitcoin::Network::Regtest);
    let policy = ChainPolicy {
        genesis: genesis.block_hash(),
        pow_limit_bits: 0x207fffff,
        initial_target_bits: 0x207fffff,
        no_pow_retargeting: true,
        allow_min_difficulty_blocks: false,
        pow_target_spacing: 600,
        difficulty_adjustment_interval: 2016,
        regtest_pow_bits_overlay: 0x207fffff,
        subsidy_initial_sats: 5_000_000_000,
        subsidy_halving_interval: 150,
    };
    let mut blocks = vec![genesis];
    for height in 1..=101 {
        blocks.push(block(
            blocks.last().unwrap(),
            height,
            funding.clone(),
            Vec::new(),
            0,
        ));
    }
    wallet
        .note_chain_observed(blocks[101].block_hash(), 101, &[])
        .unwrap();
    wallet
        .observe_funding(&FundingObservation {
            outpoint: OutPoint {
                txid: blocks[1].txdata[0].compute_txid(),
                vout: 0,
            },
            amount_sats: 5_000_000_000,
            chain: Chain::Receive,
            index: 0,
            owner_role: WalletRole::BootstrapFunding,
            source: FundingSource::BootstrapCoinbase,
            context: FundingContext::CanonicalBlock {
                block_hash: blocks[1].block_hash().to_string(),
                height: 1,
                coinbase: true,
            },
        })
        .unwrap();
    let mut destinations = Vec::new();
    for role in [
        WalletRole::NetworkWallet { index: 0 },
        WalletRole::NetworkWallet { index: 1 },
        WalletRole::UserPayment { index: 0 },
        WalletRole::UserPayment { index: 1 },
    ] {
        let script = TrafficWallet::new(42, &allocation.run_id, role, allocation.network, 2)
            .unwrap()
            .script_at(Chain::Receive, 0)
            .unwrap()
            .script_pubkey;
        destinations.push(DestinationSpec {
            script_pubkey: script,
            amount_sats: 100_000,
            owner: None,
        });
    }
    let view = match wallet
        .sign_payment(&PaymentRequest {
            command_id: "distribution".into(),
            payer_role: WalletRole::BootstrapFunding,
            category: PaymentCategory::BootstrapDistribution,
            destinations,
            fee_rate_sats_vb: 3,
        })
        .unwrap()
    {
        PaymentOutcome::Signed(view) => view,
        _ => panic!("fresh signer fixture"),
    };
    let tx = deserialize(&hex::decode(&view.raw_hex).unwrap()).unwrap();
    blocks.push(block(
        blocks.last().unwrap(),
        102,
        funding,
        vec![tx],
        view.fee_sats,
    ));
    let mut node = FixtureNode { blocks };
    let index = CanonicalIndex::scan(
        &mut node,
        "fixture-a",
        policy.genesis,
        policy.pow_limit().unwrap(),
    )
    .unwrap();
    let second = CanonicalIndex::scan(
        &mut node,
        "fixture-b",
        policy.genesis,
        policy.pow_limit().unwrap(),
    )
    .unwrap();
    let completion = spc_lab_txd::bootstrap::BootstrapCompletion {
        distribution_txids: vec![view.txid.to_string()],
        common_tip_hash: index.anchor().block_hash.to_string(),
        common_tip_height: 102,
    };
    spc_lab::traffic::completion::verify_distribution(
        &completion,
        &[index, second],
        &allocation,
        &policy,
        101,
        1,
    )
    .unwrap();
    // Discovery is inclusive after a crash between ChainObserved and the
    // remaining funding outputs. Valid outpoints are skipped, not re-credited.
    let receiver_dir = dir.join("receiver");
    std::fs::create_dir(&receiver_dir).unwrap();
    let receiver = TrafficWalletSet::provision(&token, "b", &receiver_dir).unwrap();
    receiver
        .note_chain_observed(node.blocks[102].block_hash(), 102, &[])
        .unwrap();
    receiver
        .observe_funding(&FundingObservation {
            outpoint: OutPoint {
                txid: view.txid,
                vout: 1,
            },
            amount_sats: 100_000,
            chain: Chain::Receive,
            index: 0,
            owner_role: WalletRole::NetworkWallet { index: 1 },
            source: FundingSource::CanonicalBlock,
            context: FundingContext::CanonicalBlock {
                block_hash: node.blocks[102].block_hash().to_string(),
                height: 102,
                coinbase: false,
            },
        })
        .unwrap();
    drop(receiver);
    let receiver = TrafficWalletSet::reopen(&token, "b", &receiver_dir).unwrap();
    let scan = CanonicalIndex::scan(
        &mut node,
        "fixture-a",
        policy.genesis,
        policy.pow_limit().unwrap(),
    )
    .unwrap();
    scan.observe(
        &receiver,
        &mut node,
        "fixture-a",
        policy.genesis,
        policy.pow_limit().unwrap(),
        2,
    )
    .unwrap();
    assert_eq!(receiver.journal().summary().funding_observations, 2);
    let seq = receiver.journal().last_seq();
    scan.observe(
        &receiver,
        &mut node,
        "fixture-a",
        policy.genesis,
        policy.pow_limit().unwrap(),
        2,
    )
    .unwrap();
    assert_eq!(receiver.journal().last_seq(), seq);
    // Own change is inserted only after the creator's raw canonical inclusion.
    wallet.note_broadcast_sent("distribution").unwrap();
    wallet
        .note_accepted_by_rpc("distribution", "fixture-a")
        .unwrap();
    scan.observe(
        &wallet,
        &mut node,
        "fixture-a",
        policy.genesis,
        policy.pow_limit().unwrap(),
        2,
    )
    .unwrap();
    assert!(wallet
        .journal()
        .observer_snapshot()
        .outpoints
        .values()
        .any(|o| o.chain == Chain::Change && o.source == FundingSource::OwnTransaction));
    let mut bad = completion.clone();
    bad.distribution_txids.clear();
    let index = CanonicalIndex::scan(
        &mut node,
        "fixture-a",
        policy.genesis,
        policy.pow_limit().unwrap(),
    )
    .unwrap();
    assert!(spc_lab::traffic::completion::verify_distribution(
        &bad,
        &[index],
        &allocation,
        &policy,
        101,
        1
    )
    .is_err());
    let index = CanonicalIndex::scan(
        &mut node,
        "fixture-a",
        policy.genesis,
        policy.pow_limit().unwrap(),
    )
    .unwrap();
    assert!(spc_lab::traffic::completion::verify_distribution(
        &completion,
        &[index],
        &allocation,
        &policy,
        101,
        2
    )
    .is_err());
    let index = CanonicalIndex::scan(
        &mut node,
        "fixture-a",
        policy.genesis,
        policy.pow_limit().unwrap(),
    )
    .unwrap();
    node.blocks.pop();
    let lagging = CanonicalIndex::scan(
        &mut node,
        "fixture-b",
        policy.genesis,
        policy.pow_limit().unwrap(),
    )
    .unwrap();
    assert!(spc_lab::traffic::completion::verify_distribution(
        &completion,
        &[index, lagging],
        &allocation,
        &policy,
        101,
        1
    )
    .is_err());
    let _hash: BlockHash = policy.genesis;
}
