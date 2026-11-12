//! Offline rule vectors and genuinely hashed raw-header fixtures.
use super::*;
use bitcoin::blockdata::constants::genesis_block;
use bitcoin::consensus::{deserialize, serialize};
use bitcoin::hashes::Hash;
use bitcoin::{Block, Network};
use spc_cpu_miner::hash::HeaderPreimage;
use std::collections::BTreeMap;

fn policy() -> ChainPolicy {
    ChainPolicy {
        genesis: genesis_block(Network::Regtest).block_hash(),
        pow_limit_bits: 0x207fffff,
        initial_target_bits: 0x200fffff,
        regtest_pow_bits_overlay: 0x200fffff,
        no_pow_retargeting: true,
        allow_min_difficulty_blocks: true,
        pow_target_spacing: 600,
        difficulty_adjustment_interval: 2016,
        subsidy_initial_sats: 5_000_000_000,
        subsidy_halving_interval: 150,
    }
}
// Synthetic prefix used ONLY by the private arithmetic vectors below.
fn prefix(bits: &[u32], first_time: u32) -> Vec<Header> {
    bits.iter()
        .enumerate()
        .map(|(i, bits)| {
            let mut h = genesis_block(Network::Regtest).header;
            h.bits = CompactTarget::from_consensus(*bits);
            h.time = first_time + i as u32;
            h
        })
        .collect()
}

#[test]
fn first_gap_and_exact_two_spacing_boundary() {
    let p = policy();
    let chain = prefix(&[0x207fffff], 100);
    assert_eq!(expected_bits(&p, &chain, 1301).unwrap(), 0x207fffff);
    assert_eq!(expected_bits(&p, &chain, 1300).unwrap(), 0x200fffff);
    assert_eq!(expected_bits(&p, &chain, 101).unwrap(), 0x200fffff);
    let mut disabled = p;
    disabled.allow_min_difficulty_blocks = false;
    assert_eq!(expected_bits(&disabled, &chain, 1301).unwrap(), 0x200fffff);
}

#[test]
fn short_gap_after_minimum_restores_overlay_or_last_nonminimum() {
    let p = policy();
    assert_eq!(
        expected_bits(&p, &prefix(&[0x207fffff, 0x207fffff], 100), 102).unwrap(),
        0x200fffff
    );
    assert_eq!(
        expected_bits(
            &p,
            &prefix(
                &[0x207fffff, 0x1f7fffff, 0x207fffff, 0x207fffff, 0x207fffff],
                100
            ),
            105
        )
        .unwrap(),
        0x1f7fffff
    );
}

#[test]
fn no_retarget_boundary_keeps_parent_and_overlay_exemptions() {
    let mut p = policy();
    p.difficulty_adjustment_interval = 4;
    let min_parent = prefix(&[0x207fffff; 4], 100);
    assert_eq!(expected_bits(&p, &min_parent, 104).unwrap(), 0x207fffff);
    let hard_parent = prefix(&[0x207fffff, 0x200fffff, 0x200fffff, 0x200fffff], 100);
    assert_eq!(expected_bits(&p, &hard_parent, 10000).unwrap(), 0x200fffff);
    // Backtracking must stop at the most recent boundary, then the overlay
    // applies to its pow-limit bits for a non-boundary short-gap candidate.
    let boundary_parent = prefix(
        &[
            0x207fffff, 0x1f7fffff, 0x207fffff, 0x207fffff, 0x207fffff, 0x207fffff,
        ],
        100,
    );
    assert_eq!(
        expected_bits(&p, &boundary_parent, 106).unwrap(),
        0x200fffff
    );
}

#[test]
fn harder_inherited_target_is_not_replaced_by_initial_overlay() {
    let mut p = policy();
    let chain = prefix(&[0x207fffff, 0x1f7fffff], 100);
    assert_eq!(expected_bits(&p, &chain, 102).unwrap(), 0x1f7fffff);
    assert_eq!(expected_bits(&p, &chain, 1302).unwrap(), 0x207fffff);
    p.allow_min_difficulty_blocks = false;
    assert_eq!(expected_bits(&p, &chain, 1302).unwrap(), 0x1f7fffff);
}

#[test]
fn required_facts_and_unsupported_or_overflowing_policy_refuse() {
    for field in [
        "allow_min_difficulty_blocks",
        "pow_target_spacing",
        "difficulty_adjustment_interval",
        "regtest_pow_bits_overlay",
    ] {
        let mut v = serde_json::to_value(policy()).unwrap();
        v.as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<ChainPolicy>(v).is_err(), "{field}");
    }
    let mut p = policy();
    p.pow_target_spacing = u64::MAX;
    assert!(validate_policy(&p).is_err());
    p = policy();
    p.difficulty_adjustment_interval = 0;
    assert!(validate_policy(&p).is_err());
    p = policy();
    p.no_pow_retargeting = false;
    assert_eq!(
        validate_policy(&p).unwrap_err(),
        "bootstrap_retarget_schedule_unsupported"
    );
    p = policy();
    p.regtest_pow_bits_overlay = 0x20800001;
    assert!(validate_policy(&p).is_err());
    assert!(VerifiedHeaders::new(&policy(), vec![]).is_err());
}

fn mined(parent: &Block, height: u32, time: u32, bits: u32) -> Block {
    let mut coinbase = genesis_block(Network::Regtest).txdata.remove(0);
    coinbase.input[0].script_sig = bitcoin::script::Builder::new()
        .push_int(i64::from(height))
        .push_slice([0; 8])
        .into_script();
    let bytes = serialize(&coinbase);
    let preimage = HeaderPreimage::new(
        1,
        parent.block_hash().to_byte_array(),
        &bytes,
        &[],
        time,
        bits,
    );
    let target = checked_target(bits, policy().pow_limit().unwrap()).unwrap();
    for nonce in 0..1_000_000 {
        let (raw, hash) = preimage.hash_nonce(nonce);
        if target.is_met_by(hash) {
            return Block {
                header: deserialize(&raw).unwrap(),
                txdata: vec![coinbase],
            };
        }
    }
    panic!("bounded offline PoW fixture exhausted")
}

#[test]
fn real_headers_observer_and_context_agree_on_gap_then_overlay() {
    let p = policy();
    let genesis = genesis_block(Network::Regtest);
    let first = mined(&genesis, 1, genesis.header.time + 1201, 0x207fffff);
    let second = mined(&first, 2, first.header.time + 1, 0x200fffff);
    let blocks = vec![genesis, first, second];
    let context = VerifiedHeaders::new(&p, blocks.iter().map(|b| b.header).collect()).unwrap();
    assert_eq!(
        context
            .expected_next_bits(3, context.tip(), blocks[2].header.time + 1)
            .unwrap(),
        0x200fffff
    );
    let index = crate::observer::CanonicalIndex {
        blocks,
        transactions: BTreeMap::new(),
        unspent: BTreeMap::new(),
        fees: BTreeMap::new(),
    };
    index.verify_policy(&p).unwrap();
}

#[test]
fn unexpected_minimum_sparse_ancestry_and_equal_height_fork_refuse() {
    let p = policy();
    let genesis = genesis_block(Network::Regtest);
    let first = mined(&genesis, 1, genesis.header.time + 1201, 0x207fffff);
    let bad = mined(&first, 2, first.header.time + 1, 0x207fffff);
    assert!(VerifiedHeaders::new(&p, vec![genesis.header, first.header, bad.header]).is_err());
    let second = mined(&first, 2, first.header.time + 1, 0x200fffff);
    assert!(VerifiedHeaders::new(&p, vec![genesis.header, second.header]).is_err());
    let fork = mined(&genesis, 1, genesis.header.time + 1202, 0x207fffff);
    let context = VerifiedHeaders::new(&p, vec![genesis.header, first.header]).unwrap();
    assert!(context
        .expected_next_bits(2, fork.block_hash(), fork.header.time + 1)
        .is_err());
    assert!(context
        .expected_next_bits(1, first.block_hash(), first.header.time + 1)
        .is_err());
    let wrong_first = mined(&genesis, 1, genesis.header.time + 1201, 0x200fffff);
    assert!(VerifiedHeaders::new(&p, vec![genesis.header, wrong_first.header]).is_err());
}
