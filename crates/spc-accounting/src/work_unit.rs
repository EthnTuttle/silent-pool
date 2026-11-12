//! Pure v2 work-unit arithmetic (LP-1a precursor).
//!
//! Implements the approved decisions
//! `docs/decisions/2026-11-05-lab-share-work-units.md` (reference-target
//! family, consensus compact decode, O(1) issuance selection, fingerprint,
//! checked arithmetic with named errors) and
//! `docs/decisions/2026-11-05-window-network-block-work.md` (the
//! `Q(T) = ceil(N (R + 1)/(T + 1))` network-block window target, the
//! `window_rule` codes, and the `max_window_target_units` hard cap).
//!
//! # Scope (LP-1a precursor only)
//!
//! This module is pure arithmetic: no I/O, no clock, no pool state. It is
//! NOT the full LP-1 packet. Job generations, `job_issued` evidence, slice
//! and distribution persistence, wire fields, restore rejection, and the
//! adoption of these helpers by the aggregation and payout paths are future
//! LP-1 work gated on the L1 SV2 merge. Until that adoption lands, it is
//! NOT claimed that the full v2 accounting path is overflow-safe; only the
//! functions in this module are checked and tested.
//!
//! # Arithmetic contract
//!
//! - No `f64` or `f32` anywhere (a test asserts this at the source level).
//! - Checked arithmetic everywhere: no saturation, no wrapping, no implicit
//!   lossy casts. The only casts are explicit widenings (`u64 -> u128`) and
//!   explicit narrowings behind checked conversions.
//! - A [`Target`] is 256 bits, big-endian. Wide intermediates use private
//!   big-endian `[u64; N]` limb types with auditable restoring-division
//!   algorithms. No `unsafe`, no external arithmetic crates.
//!
//! # Width caveat (decision section 5.5)
//!
//! `d * f` fitting in `u128` does NOT imply that the window-level fee
//! aggregates (`DF_S`, `F_net * sum(d*f)`) fit in `u128`. A small `m` alone
//! guarantees nothing about the fee products. The checked helpers here are
//! building blocks; the SPEC section 4.4 products must use the exact wide
//! multiply/divide ([`checked_mul_div_u128`]) or wider and fail with
//! `payout_product_overflow` once LP-1 wires them into the payout path.

#![deny(unsafe_code)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Default reference exponent: `R = 2^255 - 1`, the recommended lab profile
/// (`E(R) = 2` expected hashes; easier than the regtest pow limit).
pub const DEFAULT_REFERENCE_EXPONENT: u32 = 255;

/// Hard cap on `max_work_multiplier` (work-unit decision section 5.5): a v2
/// configuration MUST reject a larger value.
pub const HARD_CAP_MAX_WORK_MULTIPLIER: u64 = 1 << 32;

/// Hard cap on `max_window_target_units` (window decision section 5): a v2
/// configuration MUST reject a larger value.
pub const HARD_CAP_MAX_WINDOW_TARGET_UNITS: u128 = 1 << 64;

/// Fingerprint preimage prefix (work-unit decision section 7.1):
/// `sha256("spc-lab/work-unit/v2" || 0x00 || R_be32)`.
pub const WORK_UNIT_FINGERPRINT_PREFIX: &[u8] = b"spc-lab/work-unit/v2";

/// Named fail-closed errors for the v2 work-unit arithmetic. Every invalid
/// case is an error; there is no clamp to 1, no saturation and no silent
/// zero (work-unit decision section 5.4).
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum WorkUnitError {
    #[error("overflow in v2 work-unit arithmetic")]
    Overflow,
    #[error("compact target sign bit set")]
    CompactSign,
    #[error("compact target overflows 256 bits")]
    CompactOverflow,
    #[error("compact target decodes to zero")]
    CompactZero,
    #[error("reference_not_power_of_two: R + 1 is not a power of two")]
    ReferenceNotPowerOfTwo,
    #[error("reference_exponent_out_of_range: k = {0} outside the supported bounds 1..=255")]
    ReferenceExponentOutOfRange(u32),
    #[error("reference_below_pow_limit: reference {reference} is below the network pow limit {pow_limit}")]
    ReferenceBelowPowLimit {
        reference: String,
        pow_limit: String,
    },
    #[error(
        "multiplier_not_in_family: {0} is not a member of the power-of-two family [1, 2^(k-1)]"
    )]
    MultiplierNotInFamily(u64),
    #[error("multiplier_above_hard_cap: {0} exceeds max_work_multiplier (hard cap 2^32)")]
    MultiplierAboveHardCap(u64),
    #[error("max_work_multiplier_above_hard_cap: {0} exceeds the 2^32 hard cap")]
    MaxWorkMultiplierAboveHardCap(u64),
    #[error("max_window_target_units_above_hard_cap: {0} exceeds the 2^64 hard cap")]
    MaxWindowTargetAboveHardCap(u128),
    #[error("network target is zero")]
    NetworkTargetZero,
    #[error("share target is zero")]
    ShareTargetZero,
    #[error("share target is below the network target")]
    ShareTargetBelowNetwork,
    #[error("target_profile_unplannable: no family member is easier than the network target (defensive guard, unreachable for a valid profile)")]
    TargetProfileUnplannable,
    #[error("window_n_zero: the window multiplier N must be > 0")]
    WindowNZero,
    #[error("window_target_zero: the window target must be >= 1")]
    WindowTargetZero,
    #[error("window_target_too_large: the computed window target exceeds the hard cap {cap} reference units")]
    WindowTargetTooLarge {
        /// The computed value when it fits `u128`; `None` when it does not.
        computed: Option<u128>,
        cap: u128,
    },
    #[error("window_work_overflow: window work accumulation overflowed u128")]
    WindowWorkOverflow,
    #[error("slice_work_overflow: slice work accumulation overflowed u128")]
    SliceWorkOverflow,
    #[error("slice_fee_work_overflow: d*f or a diff_fee_sum accumulation overflowed u128")]
    SliceFeeWorkOverflow,
    #[error("payout_window_empty: v2 payout requires a non-empty positive-work window")]
    PayoutWindowEmpty,
    #[error("payout_product_overflow: a payout product or its exact quotient does not fit the required width")]
    PayoutProductOverflow,
    #[error("unknown work unit version {0}")]
    UnknownWorkUnitVersion(u8),
    #[error("unknown window rule {0}")]
    UnknownWindowRule(u8),
    #[error("work-unit version {version} and window rule {rule} are incompatible")]
    VersionRuleMismatch { version: u8, rule: u8 },
    #[error("work_unit_restore_mismatch: slice {slice_id} version/fingerprint does not match the configured profile")]
    WorkUnitRestoreMismatch { slice_id: u64 },
}

/// Versioned work-unit identifier persisted on sealed slices and
/// distribution snapshots (work-unit decision section 7.2). Unknown values
/// fail closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WorkUnitVersion {
    /// Legacy integer share difficulty; saturating arithmetic, frozen.
    V1,
    /// Exact reference-multiplier profile; checked arithmetic.
    V2,
}

impl WorkUnitVersion {
    pub fn as_u8(self) -> u8 {
        match self {
            Self::V1 => 1,
            Self::V2 => 2,
        }
    }

    pub fn from_u8(v: u8) -> Result<Self, WorkUnitError> {
        match v {
            1 => Ok(Self::V1),
            2 => Ok(Self::V2),
            other => Err(WorkUnitError::UnknownWorkUnitVersion(other)),
        }
    }
}

/// Persisted window-rule discriminator (window decision section 3.3), used
/// by both the database `SMALLINT` and the wire `U8`. Unknown codes fail
/// closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WindowRule {
    /// Stored `window_target` is `N * d` (v1); named `share_work_legacy`.
    ShareWorkLegacy,
    /// Stored `window_target` is `Q(T)` in v2 reference units.
    NetworkBlockWork,
}

impl WindowRule {
    pub fn as_u8(self) -> u8 {
        match self {
            Self::ShareWorkLegacy => 0,
            Self::NetworkBlockWork => 1,
        }
    }

    pub fn from_u8(v: u8) -> Result<Self, WorkUnitError> {
        match v {
            0 => Ok(Self::ShareWorkLegacy),
            1 => Ok(Self::NetworkBlockWork),
            other => Err(WorkUnitError::UnknownWindowRule(other)),
        }
    }
}

/// A v1 window may only carry the legacy rule; a v2 window only the
/// network-block rule. Any other combination fails closed at preflight and
/// at restore (window decision section 3.4).
pub fn check_version_rule_compatibility(
    version: WorkUnitVersion,
    rule: WindowRule,
) -> Result<(), WorkUnitError> {
    match (version, rule) {
        (WorkUnitVersion::V1, WindowRule::ShareWorkLegacy)
        | (WorkUnitVersion::V2, WindowRule::NetworkBlockWork) => Ok(()),
        (version, rule) => Err(WorkUnitError::VersionRuleMismatch {
            version: version.as_u8(),
            rule: rule.as_u8(),
        }),
    }
}

/// A 256-bit consensus target (the inclusive threshold `T`, `0 <= T`).
///
/// Stored big-endian, so the derived `Ord` is numeric order: a hash meets the
/// target iff `hash <= target` compared as 256-bit integers (work-unit
/// decision section 5.2). Never compared as decimal strings, never through a
/// float.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Target([u8; 32]);

impl Target {
    pub fn from_be_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn to_be_bytes(self) -> [u8; 32] {
        self.0
    }

    pub fn is_zero(&self) -> bool {
        self.0 == [0u8; 32]
    }

    /// Number of significant bits (`0..=256`).
    pub fn bit_length(&self) -> u32 {
        U256::from_be_bytes(self.0).bit_length()
    }

    fn from_u256(v: U256) -> Self {
        Self(v.to_be_bytes())
    }

    fn to_u256(self) -> U256 {
        U256::from_be_bytes(self.0)
    }
}

/// Decode a compact target with the consensus `SetCompact` rule (work-unit
/// decision section 5.1): for `size = bits >> 24` and `word = bits & 0x007fffff`,
///
/// - reject when `word != 0` and the sign bit `0x00800000` is set;
/// - reject when `word != 0` and (`size > 34` or (`word > 0xff` and
///   `size > 33`) or (`word > 0xffff` and `size > 32`));
/// - shift right `8 * (3 - size)` when `size <= 3`, otherwise left
///   `8 * (size - 3)`;
/// - reject a zero result.
///
/// Exponent 33 with `word <= 0xffff` and exponent 34 with `word <= 0xff` are
/// valid 256-bit targets; there is no exponent-32 cutoff. The node, not the
/// pool, enforces `target <= pow_limit`.
pub fn decode_compact_target(bits: u32) -> Result<Target, WorkUnitError> {
    let size = bits >> 24;
    let word = bits & 0x007f_ffff;
    if word != 0 {
        if bits & 0x0080_0000 != 0 {
            return Err(WorkUnitError::CompactSign);
        }
        if size > 34 || (word > 0xff && size > 33) || (word > 0xffff && size > 32) {
            return Err(WorkUnitError::CompactOverflow);
        }
    }
    let value = if size <= 3 {
        U256::from_u64(u64::from(word >> (8 * (3 - size))))
    } else {
        U256::from_u64(u64::from(word)).checked_shl(8 * (size - 3))?
    };
    if value.is_zero() {
        return Err(WorkUnitError::CompactZero);
    }
    Ok(Target::from_u256(value))
}

/// A validated v2 reference-target profile: `R = 2^k - 1` with
/// `1 <= k <= 255`, so `R + 1 = 2^k` is an exact power of two and the exact
/// multipliers are `m = 2^j`, `0 <= j <= k - 1` (work-unit decision
/// section 3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkUnitProfile {
    k: u32,
    reference: Target,
    max_work_multiplier: u64,
}

impl WorkUnitProfile {
    /// Build a profile from the reference exponent `k` (R = `2^k - 1`) and
    /// the configured hard cap `max_work_multiplier <= 2^32`.
    pub fn from_exponent(k: u32, max_work_multiplier: u64) -> Result<Self, WorkUnitError> {
        if k == 0 || k > 255 {
            return Err(WorkUnitError::ReferenceExponentOutOfRange(k));
        }
        if max_work_multiplier > HARD_CAP_MAX_WORK_MULTIPLIER {
            return Err(WorkUnitError::MaxWorkMultiplierAboveHardCap(
                max_work_multiplier,
            ));
        }
        Ok(Self {
            k,
            reference: Target::from_u256(U256::low_ones(k)),
            max_work_multiplier,
        })
    }

    /// Build a profile from the 32-byte big-endian reference target. `R + 1`
    /// must be an exact power of two (`reference_not_power_of_two`
    /// otherwise; it would be a new version) and the derived `k` must lie in
    /// the supported bounds.
    pub fn from_reference(
        reference_be32: [u8; 32],
        max_work_multiplier: u64,
    ) -> Result<Self, WorkUnitError> {
        if max_work_multiplier > HARD_CAP_MAX_WORK_MULTIPLIER {
            return Err(WorkUnitError::MaxWorkMultiplierAboveHardCap(
                max_work_multiplier,
            ));
        }
        let r = U256::from_be_bytes(reference_be32);
        // R + 1 = 2^k: exactly one bit set. The all-ones sentinel has
        // R + 1 = 2^256, which overflows 256 bits and yields k = 256.
        let k = match r.add1_checked() {
            Err(_) => 256,
            Ok(r1) => {
                if !r1.is_power_of_two() {
                    return Err(WorkUnitError::ReferenceNotPowerOfTwo);
                }
                // A single bit at position k - 1 has bit length k.
                r1.bit_length() - 1
            }
        };
        Self::from_exponent(k, max_work_multiplier)
    }

    /// The recommended regtest profile: `R = 2^255 - 1` (`E(R) = 2` expected
    /// hashes) with the default `max_work_multiplier = 2^32` hard cap. It is
    /// easier than the regtest pow limit `0x207fffff`, so profile validity
    /// holds and `m = 1` is always issuable.
    pub fn lab_default() -> Self {
        Self {
            k: DEFAULT_REFERENCE_EXPONENT,
            reference: Target::from_u256(U256::low_ones(DEFAULT_REFERENCE_EXPONENT)),
            max_work_multiplier: HARD_CAP_MAX_WORK_MULTIPLIER,
        }
    }

    pub fn exponent(&self) -> u32 {
        self.k
    }

    pub fn reference_target(&self) -> Target {
        self.reference
    }

    pub fn reference_bytes(&self) -> [u8; 32] {
        self.reference.to_be_bytes()
    }

    pub fn max_work_multiplier(&self) -> u64 {
        self.max_work_multiplier
    }

    /// `sha256("spc-lab/work-unit/v2" || 0x00 || R_be32)`. The fingerprint
    /// does not depend on `m_cfg`, `m_issue`, `N`, the pool, the node or the
    /// scenario (work-unit decision section 7.1).
    pub fn fingerprint(&self) -> [u8; 32] {
        work_unit_fingerprint(&self.reference.to_be_bytes())
    }

    /// Profile validity for a network (work-unit decision section 3):
    /// `R >= T_powlimit` for the network's effective `pow_limit_bits`. With
    /// that, `m = 1` is issuable for every target consensus can produce,
    /// because `T <= T_powlimit <= R = S(1)` always.
    pub fn validate_pow_limit(&self, pow_limit_bits: u32) -> Result<(), WorkUnitError> {
        let pow_limit = decode_compact_target(pow_limit_bits)?;
        if self.reference >= pow_limit {
            Ok(())
        } else {
            Err(WorkUnitError::ReferenceBelowPowLimit {
                reference: hex32(&self.reference.to_be_bytes()),
                pow_limit: hex32(&pow_limit.to_be_bytes()),
            })
        }
    }

    /// Validate the configured multiplier `m_cfg = 2^j_cfg` and return
    /// `j_cfg`. Rejects `m = 0`, a non-power (`multiplier_not_in_family`),
    /// `m_cfg > max_work_multiplier` (`multiplier_above_hard_cap`), and
    /// `j_cfg > k - 1` (outside `[1, 2^(k-1)]`, also
    /// `multiplier_not_in_family`).
    pub fn validate_multiplier(&self, m_cfg: u64) -> Result<u32, WorkUnitError> {
        if m_cfg == 0 || m_cfg & (m_cfg - 1) != 0 {
            return Err(WorkUnitError::MultiplierNotInFamily(m_cfg));
        }
        if m_cfg > self.max_work_multiplier {
            return Err(WorkUnitError::MultiplierAboveHardCap(m_cfg));
        }
        let j_cfg = m_cfg.trailing_zeros();
        if j_cfg > self.k - 1 {
            return Err(WorkUnitError::MultiplierNotInFamily(m_cfg));
        }
        Ok(j_cfg)
    }

    /// The share target for multiplier exponent `j`:
    /// `S(2^j) = 2^(k - j) - 1` with `0 <= j <= k - 1`, so `S >= 1` (a zero
    /// target is exactly `j = k` and is not in the family).
    pub fn share_target(&self, j: u32) -> Result<Target, WorkUnitError> {
        if j > self.k - 1 {
            let m = if j < 64 { 1u64 << j } else { 0 };
            return Err(WorkUnitError::MultiplierNotInFamily(m));
        }
        Ok(Target::from_u256(U256::low_ones(self.k - j)))
    }

    /// The exact expected-work ratio `E(S) / E(R) = 2^j` for a share target
    /// `S` of this family, as a `u64` multiplier. A target outside the
    /// power-of-two family is refused, never rounded to the nearest member.
    pub fn multiplier_for_share_target(&self, s: &Target) -> Result<u64, WorkUnitError> {
        let s = s.to_u256();
        if s.is_zero() {
            return Err(WorkUnitError::ShareTargetZero);
        }
        // S + 1 must be the power of two 2^p with 1 <= p <= k; m = 2^(k - p).
        let s1 = match s.add1_checked() {
            // S = 2^256 - 1: S + 1 = 2^256 > R + 1 for every supported k.
            Err(_) => return Err(WorkUnitError::MultiplierNotInFamily(0)),
            Ok(v) => v,
        };
        if !s1.is_power_of_two() {
            return Err(WorkUnitError::MultiplierNotInFamily(0));
        }
        let p = s1.bit_length() - 1;
        if p > self.k {
            return Err(WorkUnitError::MultiplierNotInFamily(0));
        }
        // Fail closed if the multiplier does not fit u64 (it can only exceed
        // the cap for a foreign target, never for an issued one).
        1u64.checked_shl(self.k - p).ok_or(WorkUnitError::Overflow)
    }

    /// O(1) issuance selection for one job generation (work-unit decision
    /// section 4): `j_issue = min(j_cfg, k - ceil_log2(T + 1))` where
    /// `ceil_log2(n) = (n - 1).bit_length()`, i.e. exactly
    /// `ceil_log2(T + 1) = T.bit_length()`. No scan of `[1, m_cfg]`, no
    /// divisor search.
    ///
    /// The generation records `m_issue` and validates submissions at
    /// `S(m_issue)`. When `j_issue < j_cfg` the caller emits a
    /// `multiplier_stepdown` event ([`Issuance::stepped_down`]); a later
    /// generation with `j_issue == j_cfg` steps back up (front state).
    /// `target_profile_unplannable` is a defensive guard only: unreachable
    /// for a valid profile because `m = 1` is always in the family.
    pub fn issuance(&self, j_cfg: u32, network_target: Target) -> Result<Issuance, WorkUnitError> {
        if network_target.is_zero() {
            return Err(WorkUnitError::NetworkTargetZero);
        }
        let m_cfg = 1u64.checked_shl(j_cfg).ok_or(WorkUnitError::Overflow)?;
        if m_cfg > self.max_work_multiplier {
            return Err(WorkUnitError::MultiplierAboveHardCap(m_cfg));
        }
        if j_cfg > self.k - 1 {
            return Err(WorkUnitError::MultiplierNotInFamily(m_cfg));
        }
        // ceil_log2(T + 1) = T.bit_length(); T >= 1 so this is >= 1.
        let t_bits = network_target.to_u256().bit_length();
        let j_max = self.k as i32 - i32::try_from(t_bits).unwrap_or(i32::MAX);
        if j_max < 0 {
            // Defensive: T > R. Unreachable for a validated profile because
            // T <= T_powlimit <= R.
            return Err(WorkUnitError::TargetProfileUnplannable);
        }
        let j_issue = j_cfg.min(u32::try_from(j_max).unwrap_or(0));
        let share_target = self.share_target(j_issue)?;
        Ok(Issuance {
            j_cfg,
            j_issue,
            m_issue: 1u64 << j_issue,
            share_target,
            network_target,
            stepped_down: j_issue < j_cfg,
        })
    }
}

/// One job generation's issuance selection ([`WorkUnitProfile::issuance`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Issuance {
    pub j_cfg: u32,
    pub j_issue: u32,
    /// `2^j_issue`; the credited `d` for shares of this generation.
    pub m_issue: u64,
    /// `S(m_issue) = 2^(k - j_issue) - 1`; the submission-validation target.
    pub share_target: Target,
    pub network_target: Target,
    /// `j_issue < j_cfg`: the caller emits `multiplier_stepdown` for this
    /// generation. A step-up is the next generation with `stepped_down ==
    /// false` after a stepdown; the event belongs to the front's generation
    /// state machine, not to this pure selection.
    pub stepped_down: bool,
}

/// A target is acceptable for share issuance when `S >= 1` and
/// `S >= T` for the network target current at issuance. A target harder than
/// the network target is rejected for that generation, never silently
/// clamped (work-unit decision section 3).
pub fn share_target_is_acceptable(
    share_target: &Target,
    network_target: &Target,
) -> Result<(), WorkUnitError> {
    if share_target.is_zero() {
        return Err(WorkUnitError::ShareTargetZero);
    }
    if network_target.is_zero() {
        return Err(WorkUnitError::NetworkTargetZero);
    }
    if *share_target < *network_target {
        return Err(WorkUnitError::ShareTargetBelowNetwork);
    }
    Ok(())
}

/// The exact expected number of hashes for a target `T` as an integer,
/// `E(T) = 2^256 / (T + 1)`, when `T + 1` is a power of two (so the quotient
/// is exact) and fits `u128`. `None` otherwise: no float approximation is
/// ever produced.
pub fn expected_hashes_exact(target: &Target) -> Option<u128> {
    let t1 = match target.to_u256().add1_checked() {
        // T = 2^256 - 1 (all-ones): T + 1 = 2^256, E = 1 exactly.
        Err(_) => return Some(1),
        Ok(v) => v,
    };
    if !t1.is_power_of_two() {
        return None;
    }
    let p = t1.bit_length() - 1; // T + 1 = 2^p, E = 2^(256 - p)
    let e = 256u32.checked_sub(p)?;
    if e > 127 {
        return None; // 2^e does not fit u128
    }
    Some(1u128 << e)
}

/// `Q(T) = ceil(N * (R + 1) / (T + 1))` in v2 reference units (window
/// decision sections 1 and 6): the window threshold for `N` blocks of
/// network work.
///
/// The division is exact wide arithmetic (`N * 2^k` fits a 512-bit
/// intermediate because `N < 2^64` and `k <= 255`) followed by one ceiling.
/// A computed `Q` above the hard cap `max_window_target_units <= 2^64`
/// (or above `u128`) fails with `window_target_too_large`; it is never
/// saturated or truncated. `N = 0` and a zero network target are
/// configuration errors.
pub fn window_target_network_work(
    profile: &WorkUnitProfile,
    network_target: Target,
    n_blocks: u64,
    max_window_target_units: u128,
) -> Result<u128, WorkUnitError> {
    if n_blocks == 0 {
        return Err(WorkUnitError::WindowNZero);
    }
    if max_window_target_units > HARD_CAP_MAX_WINDOW_TARGET_UNITS {
        return Err(WorkUnitError::MaxWindowTargetAboveHardCap(
            max_window_target_units,
        ));
    }
    let t = network_target.to_u256();
    if t.is_zero() {
        return Err(WorkUnitError::NetworkTargetZero);
    }
    let den = t.add1_checked()?; // T + 1; T < 2^256 - 1 here, never zero
    let num = U512::from_u64_shifted(n_blocks, profile.exponent());
    let (quot, rem) = div_rem_u512_u256(num, den)?;
    let q = if rem.is_zero() {
        quot
    } else {
        quot.add1_checked()?
    };
    let cap_w = U512::from_u128(max_window_target_units);
    if q > cap_w {
        return Err(WorkUnitError::WindowTargetTooLarge {
            computed: q.to_u128(),
            cap: max_window_target_units,
        });
    }
    // q <= cap <= 2^64 fits u128.
    q.to_u128().ok_or(WorkUnitError::Overflow)
}

/// The derived window-selection metadata (window decision section 2).
/// `actual_sum`, `overshoot` and `underfill` are derived from the taken
/// slices and the persisted `window_target`; nothing here is a persisted
/// column. `underfilled` windows pay from the available slices with the
/// normal score normalisation and are excluded from window-length claims.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowSelection {
    /// Taken slice ids, newest first; the crossing slice is included.
    pub slice_ids: Vec<u64>,
    /// `sum(difficulty)` of the taken slices (checked `u128`).
    pub actual_sum: u128,
    /// `max(0, actual_sum - window_target)`.
    pub overshoot: u128,
    /// `max(0, window_target - actual_sum)`.
    pub underfill: u128,
    /// `underfill > 0`.
    pub underfilled: bool,
}

/// Accumulate whole slices newest-first until the window target is reached
/// (the crossing slice is included). The caller must already have excluded
/// the finding slice. Every addition is checked; overflow is
/// `window_work_overflow`, a fail-closed error, not a wrap or a clamp.
/// A zero window target fails closed (`Q >= 1` by construction).
pub fn accumulate_slice_work(
    slices_desc: &[(u64, u128)],
    window_target: u128,
) -> Result<WindowSelection, WorkUnitError> {
    if window_target == 0 {
        return Err(WorkUnitError::WindowTargetZero);
    }
    let mut slice_ids = Vec::new();
    let mut actual = 0u128;
    for (id, work) in slices_desc {
        slice_ids.push(*id);
        actual = checked_add_window_work(actual, *work)?;
        if actual >= window_target {
            break;
        }
    }
    // max(0, actual_sum - window_target): the decision's definition is a
    // clamped difference by specification, so saturating_sub is the exact
    // semantics here, not silent overflow handling.
    let overshoot = actual.saturating_sub(window_target);
    let underfill = window_target.saturating_sub(actual);
    Ok(WindowSelection {
        slice_ids,
        actual_sum: actual,
        overshoot,
        underfill,
        underfilled: actual < window_target,
    })
}

/// The stable v2 work-unit fingerprint (work-unit decision section 7.1):
/// `sha256("spc-lab/work-unit/v2" || 0x00 || R_be32)`.
pub fn work_unit_fingerprint(reference_be32: &[u8; 32]) -> [u8; 32] {
    use bitcoin::hashes::{sha256, Hash, HashEngine};
    let mut eng = sha256::Hash::engine();
    eng.input(WORK_UNIT_FINGERPRINT_PREFIX);
    eng.input(&[0x00]);
    eng.input(reference_be32);
    sha256::Hash::from_engine(eng).to_byte_array()
}

// ---------------------------------------------------------------------------
// Checked accounting helpers for later LP-1 integration
// ---------------------------------------------------------------------------

/// Checked slice-work accumulation: `acc + d` with `slice_work_overflow`.
pub fn checked_add_slice_work(acc: u128, d: u64) -> Result<u128, WorkUnitError> {
    acc.checked_add(u128::from(d))
        .ok_or(WorkUnitError::SliceWorkOverflow)
}

/// Checked per-share fee work `d * f` with `slice_fee_work_overflow`.
///
/// `u64 * u64` always fits `u128`, so this takes `u128` operands to remain a
/// meaningful checked multiply for the wider fee aggregates. Even so, the
/// product fitting `u128` does NOT imply the window-level fee aggregates fit
/// `u128` (decision section 5.5); those must go through
/// [`checked_mul_div_u128`] or wider in the payout path.
pub fn checked_fee_work(d: u128, f: u128) -> Result<u128, WorkUnitError> {
    d.checked_mul(f).ok_or(WorkUnitError::SliceFeeWorkOverflow)
}

/// Checked `diff_fee_sum` accumulation with `slice_fee_work_overflow`.
pub fn checked_add_fee_work(acc: u128, df: u128) -> Result<u128, WorkUnitError> {
    acc.checked_add(df)
        .ok_or(WorkUnitError::SliceFeeWorkOverflow)
}

/// Checked window-work accumulation with `window_work_overflow`.
pub fn checked_add_window_work(acc: u128, slice_work: u128) -> Result<u128, WorkUnitError> {
    acc.checked_add(slice_work)
        .ok_or(WorkUnitError::WindowWorkOverflow)
}

/// Exact wide multiply/divide for the SPEC section 4.4 payout products:
/// `floor(a * b / denominator)` computed through a 256-bit product and a
/// 512-bit restoring division, returning `payout_product_overflow` when the
/// product, the quotient or the denominator is unusable. Never saturates.
pub fn checked_mul_div_u128(a: u128, b: u128, denominator: u128) -> Result<u128, WorkUnitError> {
    if denominator == 0 {
        // A zero denominator is a caller bug; fail closed rather than divide
        // by zero or guess.
        return Err(WorkUnitError::PayoutProductOverflow);
    }
    let product = U256::mul_u128(a, b);
    let (quot, _) = div_rem_u512_u256(U512::from_u256(product), U256::from_u128(denominator))?;
    quot.to_u128().ok_or(WorkUnitError::PayoutProductOverflow)
}

// ---------------------------------------------------------------------------
// Private wide-arithmetic core (big-endian u64 limbs, no unsafe)
// ---------------------------------------------------------------------------

fn hex32(bytes: &[u8; 32]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(64);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// 256-bit unsigned integer, big-endian 64-bit limbs (limb 0 is the most
/// significant, so the derived `Ord` is numeric order).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct U256([u64; 4]);

impl U256 {
    const ZERO: Self = Self([0; 4]);

    fn from_u64(v: u64) -> Self {
        let mut limbs = [0u64; 4];
        limbs[3] = v;
        Self(limbs)
    }

    fn from_u128(v: u128) -> Self {
        let mut limbs = [0u64; 4];
        limbs[2] = (v >> 64) as u64;
        limbs[3] = v as u64;
        Self(limbs)
    }

    fn from_be_bytes(bytes: [u8; 32]) -> Self {
        let mut limbs = [0u64; 4];
        for (i, limb) in limbs.iter_mut().enumerate() {
            let mut buf = [0u8; 8];
            buf.copy_from_slice(&bytes[8 * i..8 * i + 8]);
            *limb = u64::from_be_bytes(buf);
        }
        Self(limbs)
    }

    fn to_be_bytes(self) -> [u8; 32] {
        let mut out = [0u8; 32];
        for (i, limb) in self.0.iter().enumerate() {
            out[8 * i..8 * i + 8].copy_from_slice(&limb.to_be_bytes());
        }
        out
    }

    fn is_zero(&self) -> bool {
        self.0 == [0u64; 4]
    }

    /// Exactly one bit set.
    fn is_power_of_two(&self) -> bool {
        let mut ones = 0u32;
        for limb in &self.0 {
            ones += limb.count_ones();
            if ones > 1 {
                return false;
            }
        }
        ones == 1
    }

    /// Number of significant bits (`0..=256`).
    fn bit_length(&self) -> u32 {
        for (i, limb) in self.0.iter().enumerate() {
            if *limb != 0 {
                return 64 * (3 - i) as u32 + (64 - limb.leading_zeros());
            }
        }
        0
    }

    /// `2^n - 1`: the low `n` bits set (`n <= 256`; `n == 0` is zero).
    fn low_ones(n: u32) -> Self {
        debug_assert!(n <= 256);
        let mut limbs = [0u64; 4];
        for (i, limb) in limbs.iter_mut().enumerate() {
            let bottom = 64 * (3 - i) as u32;
            let cnt = n.saturating_sub(bottom).min(64);
            *limb = match cnt {
                0 => 0,
                64 => u64::MAX,
                c => (1u64 << c) - 1,
            };
        }
        Self(limbs)
    }

    /// `self + 1`, `Err(Overflow)` on the all-ones value.
    fn add1_checked(&self) -> Result<Self, WorkUnitError> {
        let mut out = self.0;
        for limb in out.iter_mut().rev() {
            let (v, carry) = limb.overflowing_add(1);
            *limb = v;
            if !carry {
                return Ok(Self(out));
            }
        }
        Err(WorkUnitError::Overflow)
    }

    /// `self << shift`, `Err(Overflow)` if any set bit is shifted out.
    fn checked_shl(&self, shift: u32) -> Result<Self, WorkUnitError> {
        if shift >= 256 {
            return if self.is_zero() {
                Ok(Self::ZERO)
            } else {
                Err(WorkUnitError::Overflow)
            };
        }
        let full = (shift / 64) as usize;
        let rem = shift % 64;
        // The top `shift` bits must be zero.
        if self.0[..full].iter().any(|limb| *limb != 0) {
            return Err(WorkUnitError::Overflow);
        }
        if rem > 0 && self.0[full] >> (64 - rem) != 0 {
            return Err(WorkUnitError::Overflow);
        }
        let mut out = [0u64; 4];
        for i in full..4 {
            let mut v = self.0[i] << rem;
            if rem > 0 && i + 1 < 4 {
                v |= self.0[i + 1] >> (64 - rem);
            }
            out[i - full] = v;
        }
        Ok(Self(out))
    }

    /// Schoolbook product of two `u128` values; always fits 256 bits.
    fn mul_u128(a: u128, b: u128) -> Self {
        let a_limbs = [(a >> 64) as u64, a as u64];
        let b_limbs = [(b >> 64) as u64, b as u64];
        let mut out = [0u64; 4];
        for i in 0..2 {
            let ai = u128::from(a_limbs[1 - i]);
            let mut carry = 0u128;
            for j in 0..2 {
                let bj = u128::from(b_limbs[1 - j]);
                let idx = 3 - (i + j);
                let sum = u128::from(out[idx]) + ai * bj + carry;
                out[idx] = sum as u64;
                carry = sum >> 64;
            }
            let mut pos = i + 2;
            while carry != 0 {
                let idx = 3 - pos;
                let sum = u128::from(out[idx]) + carry;
                out[idx] = sum as u64;
                carry = sum >> 64;
                pos += 1;
            }
        }
        Self(out)
    }
}

/// 512-bit unsigned integer, big-endian 64-bit limbs; workspace for the
/// wide division (`N * 2^k` is at most 319 bits).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct U512([u64; 8]);

impl U512 {
    const ZERO: Self = Self([0; 8]);

    fn from_u256(v: U256) -> Self {
        let mut limbs = [0u64; 8];
        limbs[4..].copy_from_slice(&v.0);
        Self(limbs)
    }

    fn from_u128(v: u128) -> Self {
        let mut limbs = [0u64; 8];
        limbs[6] = (v >> 64) as u64;
        limbs[7] = v as u64;
        Self(limbs)
    }

    /// `n * 2^k` for `k <= 255` (`n` placed at bit offset `k`).
    fn from_u64_shifted(n: u64, k: u32) -> Self {
        debug_assert!(k <= 255);
        let full = (k / 64) as usize;
        let rem = k % 64;
        let mut limbs = [0u64; 8];
        limbs[7 - full] = if rem == 0 { n } else { n << rem };
        if rem > 0 {
            let carry = n >> (64 - rem);
            if carry != 0 {
                debug_assert!(full < 7);
                limbs[7 - full - 1] = carry;
            }
        }
        Self(limbs)
    }

    /// Global bit `i` (`0` = least significant).
    fn bit(&self, i: usize) -> u64 {
        (self.0[7 - i / 64] >> (i % 64)) & 1
    }

    fn set_bit(&mut self, i: usize) {
        self.0[7 - i / 64] |= 1u64 << (i % 64);
    }

    /// `self * 2`, `Err(Overflow)` if bit 511 is set.
    fn shl1_checked(&self) -> Result<Self, WorkUnitError> {
        if self.0[0] >> 63 != 0 {
            return Err(WorkUnitError::Overflow);
        }
        let mut out = [0u64; 8];
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = self.0[i] << 1;
            if i + 1 < 8 {
                *slot |= self.0[i + 1] >> 63;
            }
        }
        Ok(Self(out))
    }

    /// `self + 1`, `Err(Overflow)` on the all-ones value.
    fn add1_checked(&self) -> Result<Self, WorkUnitError> {
        let mut out = self.0;
        for limb in out.iter_mut().rev() {
            let (v, carry) = limb.overflowing_add(1);
            *limb = v;
            if !carry {
                return Ok(Self(out));
            }
        }
        Err(WorkUnitError::Overflow)
    }

    /// `self -= rhs` for `self >= rhs` (caller-maintained invariant).
    fn sub_assign(&mut self, rhs: &Self) {
        let mut borrow = 0u64;
        for i in (0..8).rev() {
            let (d1, b1) = self.0[i].overflowing_sub(rhs.0[i]);
            let (d2, b2) = d1.overflowing_sub(borrow);
            self.0[i] = d2;
            borrow = u64::from(b1 | b2);
        }
        debug_assert_eq!(borrow, 0, "sub_assign precondition violated");
    }

    /// The low 256 bits; the caller maintains the invariant that the high
    /// limbs are zero (division remainders are `< divisor <= 2^256 - 1`).
    fn to_u256_low(self) -> U256 {
        debug_assert!(self.0[..4].iter().all(|limb| *limb == 0));
        U256([self.0[4], self.0[5], self.0[6], self.0[7]])
    }

    /// The value as `u128` when it fits.
    fn to_u128(self) -> Option<u128> {
        if self.0[..6].iter().any(|limb| *limb != 0) {
            return None;
        }
        Some((u128::from(self.0[6]) << 64) | u128::from(self.0[7]))
    }
}

/// Restoring long division of a 512-bit numerator by a 256-bit divisor:
/// one bit per step with a 512-bit remainder register (the remainder stays
/// below the divisor, so the register never actually overflows, but every
/// step is checked anyway). Returns `(quotient, remainder)`.
fn div_rem_u512_u256(num: U512, den: U256) -> Result<(U512, U256), WorkUnitError> {
    if den.is_zero() {
        return Err(WorkUnitError::Overflow);
    }
    let den_w = U512::from_u256(den);
    let mut rem = U512::ZERO;
    let mut quot = U512::ZERO;
    for i in (0..512).rev() {
        rem = rem.shl1_checked()?;
        if num.bit(i) == 1 {
            // rem is even after the shift, so this cannot carry.
            rem.0[7] |= 1;
        }
        if rem >= den_w {
            rem.sub_assign(&den_w);
            quot.set_bit(i);
        }
    }
    Ok((quot, rem.to_u256_low()))
}

#[cfg(test)]
#[path = "work_unit_tests.rs"]
mod work_unit_tests;
