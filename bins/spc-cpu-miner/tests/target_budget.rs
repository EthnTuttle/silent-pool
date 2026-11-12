use std::time::{Duration, Instant};

use bitcoin::hashes::Hash;
use bitcoin::pow::{CompactTarget, Target};
use bitcoin::BlockHash;
use spc_cpu_miner::budget::{pacing_delay, saturation_decision, HashBudget, Pacer};
use spc_cpu_miner::hash::{sha256d, SearchCursor};
use spc_cpu_miner::target::{
    decode_compact_target, meets_target, target_from_le_bytes, TargetError,
};

fn hash_with_le_byte(index: usize, byte: u8) -> BlockHash {
    let mut bytes = [0u8; 32];
    bytes[index] = byte;
    BlockHash::from_byte_array(bytes)
}

#[test]
fn independent_sha256d_empty_vector() {
    // bitcoin_hashes' upstream vector. The bytes are SHA256d input order;
    // the RPC/display string is the reverse byte order.
    assert_eq!(
        hex::encode(sha256d(b"")),
        "5df6e0e2761359d30a8275058e299fcc0381534545f55cf43e41983f5d4c9456"
    );
    assert_eq!(
        BlockHash::from_byte_array(sha256d(b"")).to_string(),
        "56944c5d3f98413ef45cf54545538103cc9f298e0575820ad3591376e2e0f65d"
    );
}

#[test]
fn target_comparison_is_little_endian_and_inclusive() {
    let mut target_bytes = [0u8; 32];
    target_bytes[31] = 0x01;
    let target = target_from_le_bytes(&target_bytes).unwrap();
    assert!(meets_target(&hash_with_le_byte(31, 0x01), target));
    assert!(!meets_target(&hash_with_le_byte(31, 0x02), target));
    // This would invert the result with a reversed byte-order comparison.
    assert!(meets_target(&hash_with_le_byte(0, 0xff), target));
}

#[test]
fn compact_decode_matches_consensus_boundaries() {
    assert_eq!(
        decode_compact_target(0x0212_3456).unwrap(),
        Target::from_compact(CompactTarget::from_consensus(0x0212_3456))
    );
    for bits in [0x2100_ffff, 0x2200_00ff, 0x207f_ffff] {
        assert_eq!(
            decode_compact_target(bits).unwrap(),
            Target::from_compact(CompactTarget::from_consensus(bits))
        );
    }
    assert_eq!(decode_compact_target(0x0012_3456), Err(TargetError::Zero));
    assert_eq!(decode_compact_target(0x2080_0000), Err(TargetError::Zero));
    assert_eq!(
        decode_compact_target(0x2080_0001),
        Err(TargetError::Negative)
    );
    for bits in [0x2101_0000, 0x2200_0100, 0x2300_0001] {
        assert_eq!(decode_compact_target(bits), Err(TargetError::Overflow));
    }
}

#[test]
fn rate_pacing_is_derived_from_real_attempt_counts() {
    assert_eq!(pacing_delay(1_000, 0, Duration::ZERO), Duration::ZERO);
    assert_eq!(
        pacing_delay(1_000, 1_000, Duration::from_millis(100)),
        Duration::from_millis(900)
    );
    assert_eq!(
        pacing_delay(1_000, 50, Duration::from_secs(1)),
        Duration::ZERO
    );
}

#[test]
fn total_attempt_budget_has_no_implicit_reset() {
    let budget = HashBudget {
        hashes_per_second: None,
        total_attempts: Some(17),
    };
    assert_eq!(budget.remaining(16), 1);
    assert_eq!(budget.remaining(17), 0);
    assert!(!budget.allows(17));
}

#[test]
fn saturation_requires_sustained_unpaced_cpu_shortfall() {
    assert!(!saturation_decision(0, 100_000, 50_000, false));
    assert!(!saturation_decision(1, 100_000, 50_000, false));
    assert!(saturation_decision(2, 100_000, 50_000, false));
    assert!(!saturation_decision(2, 100_000, 99_000, false));
    assert!(!saturation_decision(2, 100_000, 50_000, true));
}

#[test]
fn reduced_nonce_domain_wraps_to_a_new_extranonce_without_rehashing() {
    // Four values split across two workers. This is deliberately a reduced
    // domain, not a claim to exhaust the production nonce/extranonce space.
    let mut cursor = SearchCursor::with_nonce_domain_size(7, 9, 0, 2, 8, 4).unwrap();
    let first_extra = cursor.extranonce().to_vec();
    assert_eq!(cursor.next_nonce(), Some(0));
    assert_eq!(cursor.next_nonce(), Some(2));
    assert_eq!(cursor.next_nonce(), None);
    assert!(cursor.roll_extranonce());
    assert_ne!(cursor.extranonce(), first_extra.as_slice());
    assert_eq!(cursor.next_nonce(), Some(0));

    let mut other = SearchCursor::with_nonce_domain_size(8, 9, 1, 2, 8, 4).unwrap();
    assert_ne!(other.extranonce(), first_extra.as_slice());
    assert_eq!(other.next_nonce(), Some(1));
    assert_eq!(other.next_nonce(), Some(3));
    assert_eq!(other.next_nonce(), None);
}

#[test]
fn extranonce_counter_exhaustion_fails_closed() {
    let mut cursor = SearchCursor::with_nonce_domain_size(1, 2, 0, 1, 4, 1).unwrap();
    cursor.set_extranonce_counter_for_test(&[0xff]);
    assert_eq!(cursor.next_nonce(), Some(0));
    assert!(!cursor.roll_extranonce());
    assert_eq!(cursor.next_nonce(), None);
}

#[test]
fn live_rate_reduction_rebases_credit_without_taking_attempt_history() {
    let start = Instant::now();
    let mut pacer = Pacer::new(start);
    pacer.note_attempts(6_000_000);
    let lowered = HashBudget {
        hashes_per_second: Some(1_000),
        total_attempts: None,
    };
    let delay = pacer.required_delay(&lowered, start + Duration::from_secs(60));
    assert!(delay <= Duration::from_millis(25), "delay was {delay:?}");
    assert_eq!(
        pacer.attempted(),
        6_000_000,
        "lifetime attempts must not reset"
    );
    let attempts_only = HashBudget {
        hashes_per_second: None,
        total_attempts: Some(6_000_001),
    };
    assert_eq!(attempts_only.remaining(pacer.attempted()), 1);
}
