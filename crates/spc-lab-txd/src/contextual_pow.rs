//! Regtest contextual work verification, shared by the bootstrap and observer.
//!
//! Source: rbitcoin-consensus/header.rs:217-365 and
//! rbitcoin-query/header_read.rs:173-202 at the source-bound node semantics
//! handoff (2026-11-09). Base min-difficulty/backtracking runs BEFORE the
//! regtest overlay. No-retargeting does not disable min-difficulty.
use crate::observer::ChainPolicy;
use bitcoin::{block::Header, BlockHash, CompactTarget, Target};

/// Only complete, linked, PoW- and schedule-verified genesis-inclusive
/// headers can construct this context. Height/fork identity is not supplied
/// by a verbose RPC claim or a cached height counter.
#[derive(Clone, Debug)]
pub struct VerifiedHeaders {
    policy: ChainPolicy,
    headers: Vec<Header>,
}
impl VerifiedHeaders {
    pub fn new(policy: &ChainPolicy, headers: Vec<Header>) -> Result<Self, String> {
        validate_policy(policy)?;
        if headers.is_empty() || headers.len() > 4096 || headers[0].block_hash() != policy.genesis {
            return Err("contextual_pow_genesis_or_coverage".into());
        }
        let limit = policy.pow_limit()?;
        for (height, header) in headers.iter().enumerate() {
            let target = checked_target(header.bits.to_consensus(), limit)?;
            header
                .validate_pow(target)
                .map_err(|_| "contextual_pow_invalid_work")?;
            if height == 0 {
                continue;
            }
            if header.prev_blockhash != headers[height - 1].block_hash() {
                return Err("contextual_pow_sparse_or_foreign_parent".into());
            }
            let expected = expected_bits(policy, &headers[..height], header.time)?;
            if header.bits.to_consensus() != expected {
                return Err(format!("contextual_pow_bits_mismatch: height={height} expected={expected:08x} actual={:08x}", header.bits.to_consensus()));
            }
        }
        Ok(Self {
            policy: policy.clone(),
            headers,
        })
    }

    pub fn tip(&self) -> BlockHash {
        self.headers
            .last()
            .expect("nonempty verified headers")
            .block_hash()
    }

    pub fn next_height(&self) -> u64 {
        self.headers.len() as u64
    }

    pub fn expected_next_bits(
        &self,
        height: u64,
        parent: BlockHash,
        time: u32,
    ) -> Result<u32, String> {
        if height != self.next_height() || parent != self.tip() {
            return Err("contextual_pow_candidate_parent_mismatch".into());
        }
        expected_bits(&self.policy, &self.headers, time)
    }
}

pub fn validate_policy(policy: &ChainPolicy) -> Result<(), String> {
    let limit = policy.pow_limit()?;
    checked_target(policy.initial_target_bits, limit)?;
    checked_target(policy.regtest_pow_bits_overlay, limit)?;
    if !policy.no_pow_retargeting {
        return Err("bootstrap_retarget_schedule_unsupported".into());
    }
    if policy.pow_target_spacing == 0
        || policy.difficulty_adjustment_interval == 0
        || policy
            .pow_target_spacing
            .checked_mul(2)
            .and_then(|gap| gap.checked_add(u64::from(u32::MAX)))
            .is_none()
        || policy.initial_target_bits != policy.regtest_pow_bits_overlay
    {
        return Err("contextual_pow_policy_invalid".into());
    }
    Ok(())
}

pub(crate) fn checked_target(bits: u32, limit: Target) -> Result<Target, String> {
    let size = bits >> 24;
    let word = bits & 0x007f_ffff;
    let target = Target::from_compact(CompactTarget::from_consensus(bits));
    if bits & 0x0080_0000 != 0
        || word == 0
        || size > 34
        || (word > 0xff && size > 33)
        || (word > 0xffff && size > 32)
        || target == Target::ZERO
        || target > limit
    {
        return Err("contextual_pow_compact_invalid".into());
    }
    Ok(target)
}

// `ancestors` is the complete genesis-inclusive prefix. Only VerifiedHeaders
// exposes this calculation to consumers; synthetic rule vectors below never
// construct a production verified context without PoW/ancestry checks.
fn expected_bits(policy: &ChainPolicy, ancestors: &[Header], time: u32) -> Result<u32, String> {
    let parent = ancestors.last().ok_or("contextual_pow_parent_missing")?;
    let height = u64::try_from(ancestors.len()).map_err(|_| "contextual_pow_height_overflow")?;
    let interval = policy.difficulty_adjustment_interval;
    let limit = policy.pow_limit()?.to_compact_lossy().to_consensus();
    let boundary = height.is_multiple_of(interval);
    let threshold = u64::from(parent.time)
        .checked_add(
            policy
                .pow_target_spacing
                .checked_mul(2)
                .ok_or("contextual_pow_spacing_overflow")?,
        )
        .ok_or("contextual_pow_time_overflow")?;
    let gap = u64::from(time) > threshold;
    // A no-retarget boundary inherits the parent's bits, even on a long gap.
    let mut bits = parent.bits.to_consensus();
    if !boundary && policy.allow_min_difficulty_blocks {
        if gap {
            bits = limit;
        } else {
            let mut h = ancestors.len() - 1;
            while !(h as u64).is_multiple_of(interval) && bits == limit {
                h = h
                    .checked_sub(1)
                    .ok_or("contextual_pow_ancestry_underflow")?;
                bits = ancestors
                    .get(h)
                    .ok_or("contextual_pow_ancestor_missing")?
                    .bits
                    .to_consensus();
            }
        }
    }
    // Overlay exceptions are intentional in this node: boundary, gap (when
    // enabled), and every inherited target other than the pow-limit compact.
    if bits == limit && !boundary && !(policy.allow_min_difficulty_blocks && gap) {
        bits = policy.regtest_pow_bits_overlay;
    }
    checked_target(bits, policy.pow_limit()?)?;
    Ok(bits)
}

#[cfg(test)]
mod tests;
