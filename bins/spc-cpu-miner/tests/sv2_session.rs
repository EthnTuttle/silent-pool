use std::time::{Duration, Instant};

use bitcoin::consensus::{deserialize, serialize};
use bitcoin::hashes::Hash;
use bitcoin::pow::Target;
use bitcoin::{Block, Txid};
use spc_cpu_miner::hash::{assemble_coinbase, assemble_header, NtimePolicy};
use spc_cpu_miner::session::{ProtocolError, ProtocolState};
use spc_cpu_miner::target::target_from_le_bytes;
use stratum_core::binary_sv2::{Seq0255, Sv2Option, B064K, U256};
use stratum_core::mining_sv2::{NewExtendedMiningJob, SetNewPrevHash, SetTarget};

fn target(high_byte: u8) -> Target {
    let mut bytes = [0u8; 32];
    bytes[31] = high_byte;
    target_from_le_bytes(&bytes).unwrap()
}

fn set_target(state: &mut ProtocolState, new_target: Target) {
    let bytes = new_target.to_le_bytes();
    state
        .on_set_target(SetTarget {
            channel_id: 41,
            maximum_target: U256::from(&bytes),
        })
        .unwrap();
}

fn job(channel_id: u32, job_id: u32, min_ntime: Option<u32>) -> NewExtendedMiningJob<'static> {
    NewExtendedMiningJob {
        channel_id,
        job_id,
        min_ntime: Sv2Option::new(min_ntime),
        version: 0x2000_0000,
        version_rolling_allowed: false,
        merkle_path: Seq0255::new(Vec::<U256>::new()).unwrap(),
        coinbase_tx_prefix: B064K::try_from(&[0x01, 0x02][..]).unwrap(),
        coinbase_tx_suffix: B064K::try_from(&[0x03, 0x04][..]).unwrap(),
    }
}

fn prev_hash(channel_id: u32, job_id: u32) -> SetNewPrevHash<'static> {
    SetNewPrevHash {
        channel_id,
        job_id,
        prev_hash: U256::from(&[0x11; 32]),
        min_ntime: 1_700_000_000,
        nbits: 0x200f_ffff,
    }
}

fn state() -> ProtocolState {
    ProtocolState::new(41, 9, target(0x20), vec![0xaa], 8).unwrap()
}

#[test]
fn future_job_uses_target_at_activation_not_at_receipt() {
    let mut state = state();
    let changes = state.on_new_extended(job(41, 7, None)).unwrap();
    assert!(changes.activated.is_empty());
    assert!(state.active_job().is_none());

    set_target(&mut state, target(0x08));
    let changes = state.on_set_new_prev_hash(prev_hash(41, 7)).unwrap();
    assert_eq!(changes.activated.len(), 1);
    let active = &changes.activated[0];
    assert_eq!(active.share_target, target(0x08));
    assert_eq!(active.target_generation, 1);
    assert_eq!(active.job_id, 7);
}

#[test]
fn active_job_target_remains_stable_when_channel_target_changes() {
    let mut state = state();
    state.on_new_extended(job(41, 7, None)).unwrap();
    state.on_set_new_prev_hash(prev_hash(41, 7)).unwrap();
    let before = state.active_job().unwrap().share_target;
    set_target(&mut state, target(0x01));
    assert_eq!(state.active_job().unwrap().share_target, before);

    // An immediate job on the same tip picks up the new channel target.
    let changes = state
        .on_new_extended(job(41, 8, Some(1_700_000_000)))
        .unwrap();
    assert_eq!(changes.activated[0].share_target, target(0x01));
    assert_eq!(changes.invalidated, vec![7]);
}

#[test]
fn prev_hash_rejects_unknown_active_job_and_wrong_channel() {
    let mut state = state();
    assert_eq!(
        state.on_set_new_prev_hash(prev_hash(41, 99)).unwrap_err(),
        ProtocolError::UnknownFutureJob(99.into())
    );
    assert_eq!(
        state.on_new_extended(job(99, 7, None)).unwrap_err(),
        ProtocolError::WrongChannel {
            expected: 41,
            group: 9,
            actual: 99,
        }
    );
    state.on_new_extended(job(41, 7, Some(1))).unwrap_err();

    state.on_new_extended(job(41, 7, None)).unwrap();
    state.on_set_new_prev_hash(prev_hash(41, 7)).unwrap();

    // L1 advances curtime on a same-tip transaction update. The job minimum is
    // authoritative; local elapsed time is not a validity ceiling.
    let job_plus_second = state
        .on_new_extended(job(41, 8, Some(1_700_000_001)))
        .unwrap();
    assert_eq!(job_plus_second.activated.len(), 1);
    let job_plus_minutes = state
        .on_new_extended(job(41, 9, Some(1_700_000_600)))
        .unwrap();
    assert_eq!(job_plus_minutes.activated[0].min_ntime, 1_700_000_600);
}

#[test]
fn activation_invalidates_all_other_jobs() {
    let mut state = state();
    state.on_new_extended(job(41, 7, None)).unwrap();
    state.on_new_extended(job(41, 8, None)).unwrap();
    state.on_new_extended(job(41, 9, None)).unwrap();
    let changes = state.on_set_new_prev_hash(prev_hash(41, 8)).unwrap();
    assert_eq!(changes.activated[0].job_id, 8);
    assert_eq!(changes.invalidated, vec![7, 9]);
    assert_eq!(state.future_job_count(), 0);
}

#[test]
fn ntime_hold_and_roll_obey_protocol_upper_bound() {
    let activated = Instant::now();
    let hold = NtimePolicy::Hold;
    assert_eq!(
        hold.ntime(100, activated, activated + Duration::from_secs(90), 600),
        100
    );
    let roll = NtimePolicy::Roll;
    assert_eq!(
        roll.ntime(100, activated, activated + Duration::from_secs(30), 600),
        130
    );
    assert_eq!(
        roll.ntime(100, activated, activated + Duration::from_secs(900), 600),
        700
    );
    assert_eq!(
        roll.ntime(100, activated, activated + Duration::from_millis(999), 600),
        100
    );
}

#[test]
fn merkle_path_is_deepest_first_against_upstream_bitcoin_tree() {
    let coinbase = b"real coinbase fixture";
    let tx_a = b"transaction A";
    let tx_b = b"transaction B";
    let tx_c = b"transaction C";
    let coinbase_id = Txid::from_byte_array(sha256d_local(coinbase));
    let a = Txid::from_byte_array(sha256d_local(tx_a));
    let b = Txid::from_byte_array(sha256d_local(tx_b));
    let c = Txid::from_byte_array(sha256d_local(tx_c));

    // Deepest-first path for [coinbase, A, B, C]: A, then H(B||C).
    // Use the upstream tree calculation as the independent root oracle.
    let expected =
        bitcoin::merkle_tree::calculate_root([coinbase_id, a, b, c].into_iter()).unwrap();
    let bc = bitcoin::hashes::sha256d::Hash::hash(&[b.to_byte_array(), c.to_byte_array()].concat());
    let path = vec![a.to_byte_array(), bc.to_byte_array()];
    let root = spc_cpu_miner::hash::merkle_root(coinbase, &[], &[], &[], &path);
    assert_eq!(root, expected.to_byte_array());

    let reversed = vec![bc.to_byte_array(), a.to_byte_array()];
    assert_ne!(
        spc_cpu_miner::hash::merkle_root(coinbase, &[], &[], &[], &reversed),
        expected.to_byte_array()
    );
}

fn sha256d_local(bytes: &[u8]) -> [u8; 32] {
    bitcoin::hashes::sha256d::Hash::hash(bytes).to_byte_array()
}

// Mainnet block 100,000 from rust-bitcoin's upstream block vector. It checks
// real coinbase serialization and an actual header hash independently of the
// synthetic multi-level Merkle-path vector above.
const BLOCK_100000: &str = "0200000035ab154183570282ce9afc0b494c9fc6a3cfea05aa8c1add2ecc56490000000038ba3d78e4500a5a7570dbe61960398add4410d278b21cd9708e6d9743f374d544fc055227f1001c29c1ea3b0101000000010000000000000000000000000000000000000000000000000000000000000000ffffffff3703a08601000427f1001c046a510100522cfabe6d6d0000000000000000000068692066726f6d20706f6f6c7365727665726aac1eeeed88ffffffff0100f2052a010000001976a914912e2b234f941f30b18afbb4fa46171214bf66c888ac00000000";

#[test]
fn upstream_block_100000_header_and_coinbase_round_trip() {
    let bytes = hex::decode(BLOCK_100000).unwrap();
    let block: Block = deserialize(&bytes).unwrap();
    let coinbase = serialize(&block.txdata[0]);
    assert!(coinbase.len() > 100);
    let split = 72;
    let prefix = &coinbase[..split];
    let extra = &coinbase[split..split + 8];
    let suffix = &coinbase[split + 8..];
    assert_eq!(assemble_coinbase(prefix, &[], extra, suffix), coinbase);

    let path = Vec::new();
    let header = assemble_header(
        block.header.version.to_consensus() as u32,
        block.header.prev_blockhash.to_byte_array(),
        &coinbase,
        &path,
        block.header.time,
        block.header.bits.to_consensus(),
        block.header.nonce,
    );
    assert_eq!(&header[..], &bytes[..80]);
    assert_eq!(
        bitcoin::BlockHash::from_byte_array(sha256d_local(&header)),
        block.block_hash()
    );

    let root = spc_cpu_miner::hash::merkle_root(prefix, &[], extra, suffix, &path);
    assert_eq!(root, block.header.merkle_root.to_byte_array());
}

#[test]
fn target_is_not_guessed_from_difficulty() {
    let state = state();
    assert_eq!(state.channel_target(), target(0x20));
}
