//! The self-hosted wrapper's default block cadence (`nix/lab/ark-runtime.nix`,
//! docs/LAB-RUNTIME.md section 4) against the scenario it launches: the
//! compact target is canonical, the hash budget yields a watchable block
//! interval and a long session before regtest halvings exhaust the subsidy,
//! and the scenario's v2 work profile still issues its configured shares.

use num_bigint::BigUint;
use spc_lab::scenario::Scenario;
use spc_lab::{ReferenceProfile, Target256};

const REGTEST_SUBSIDY_SATS: u64 = 5_000_000_000;
const REGTEST_HALVING_INTERVAL_BLOCKS: u64 = 150;
const TOP_LADDER_RUNG_SATS: u64 = 5_000_000;

fn scenario() -> Scenario {
    serde_json::from_str(include_str!("../scenarios/lab-ark-wallets.json")).unwrap()
}

fn wrapper_default_pow_bits() -> u32 {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../nix/lab/ark-runtime.nix");
    let text = std::fs::read_to_string(path).expect("wrapper source is readable");
    let flag = "--pow-bits 0x";
    let mut defaults = text
        .match_indices(flag)
        .map(|(at, _)| &text[at + flag.len()..at + flag.len() + 8]);
    let bits = defaults.next().expect("wrapper declares a default cadence");
    assert!(defaults.next().is_none(), "wrapper declares one default");
    u32::from_str_radix(bits, 16).expect("default cadence is 8 hex digits")
}

/// Consensus `GetCompact`: the one canonical encoding of a target.
fn to_compact(target: &BigUint) -> u32 {
    let mut size = u32::try_from(target.bits().div_ceil(8)).unwrap();
    let mut word = if size <= 3 {
        u32::try_from(target << (8 * (3 - size))).unwrap()
    } else {
        u32::try_from(target >> (8 * (size - 3))).unwrap()
    };
    if word & 0x0080_0000 != 0 {
        word >>= 8;
        size += 1;
    }
    (size << 24) | word
}

fn total_hashes_per_second(scenario: &Scenario) -> u64 {
    scenario
        .miners
        .iter()
        .flat_map(|miner| &miner.workers)
        .map(|worker| u64::try_from(worker.budget.hashes_per_second.unwrap().get()).unwrap())
        .sum()
}

#[test]
fn default_cadence_is_a_canonical_target_within_the_pow_limit() {
    let bits = wrapper_default_pow_bits();
    let target = Target256::from_compact(bits).unwrap();
    assert_eq!(to_compact(&target.to_biguint()), bits);

    let scenario = scenario();
    let limit = scenario
        .network
        .consensus_profile
        .requirements
        .pow_limit_bits
        .as_deref()
        .unwrap();
    let limit = Target256::from_compact(u32::from_str_radix(limit, 16).unwrap()).unwrap();
    assert!(target <= limit);
}

#[test]
fn default_cadence_gives_a_watchable_interval_and_a_long_session() {
    let scenario = scenario();
    let target = Target256::from_compact(wrapper_default_pow_bits())
        .unwrap()
        .to_biguint();
    // Expected interval = 2^256 / ((T + 1) * H) seconds.
    let space = BigUint::from(1u32) << 256;
    let hashes_per_second = (target + 1u32) * total_hashes_per_second(&scenario);
    assert!(
        &hashes_per_second * 30u32 <= space,
        "blocks faster than 30 s"
    );
    assert!(
        &hashes_per_second * 60u32 >= space,
        "blocks slower than 60 s"
    );

    let blocks_at_top_rung = (0u64..)
        .find(|height| {
            REGTEST_SUBSIDY_SATS >> (height / REGTEST_HALVING_INTERVAL_BLOCKS)
                < TOP_LADDER_RUNG_SATS
        })
        .unwrap();
    assert_eq!(blocks_at_top_rung, 1500);
    assert!(
        space * blocks_at_top_rung >= hashes_per_second * (12u32 * 3600),
        "subsidy falls below the top ladder rung within 12 hours"
    );
}

#[test]
fn default_cadence_keeps_the_scenario_work_profile_issuable() {
    let scenario = scenario();
    let target = Target256::from_compact(wrapper_default_pow_bits())
        .unwrap()
        .to_biguint();
    for pool in &scenario.pools {
        let profile = &pool.work_profile;
        let reference =
            ReferenceProfile::parse_hex(profile.reference_target.as_deref().unwrap()).unwrap();
        let multiplier = profile.share_work_multiplier.unwrap();
        assert!(multiplier.is_power_of_two());
        let j_cfg = multiplier.trailing_zeros();
        // No step-down: shares are issued at the configured multiplier.
        assert_eq!(reference.issuance_exponent(j_cfg, &target).unwrap(), j_cfg);
        let window = reference.window_target(profile.window_n, &target).unwrap();
        assert!(u128::try_from(&window).unwrap() <= profile.max_window_target_units.unwrap().get());
        // Several shares per block: the network target is much harder than
        // the share target.
        let share = reference.share_target(j_cfg).unwrap().to_biguint();
        assert!((share + 1u32) >= (&target + 1u32) * 8u32);
    }
}
