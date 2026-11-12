//! Tests for the LP-1a `work_unit` precursor module (`crates/spc-accounting/src/work_unit.rs`).
//!
//! Frozen vectors are the approved ones from
//! `docs/decisions/2026-11-05-lab-share-work-units.md` and
//! `docs/decisions/2026-11-05-window-network-block-work.md` (Fable APPROVE on
//! snapshot `3edcd4e744149667b184f44bdf3833ba465e210671eb90912fb1db9b9b11ba25`).
//!
//! Property tests compare every arithmetic result against an independent
//! arbitrary-precision oracle (base-10^9 bignum, a different representation
//! and algorithm from the module's 64-bit-limb arithmetic). The oracle lives
//! in this file and deliberately never calls module helpers.

use super::*;
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Independent big-integer oracle (base 1e9, little-endian digits)
// ---------------------------------------------------------------------------

mod oracle {
    /// Arbitrary-precision non-negative integer, base 1e9, canonical (no
    /// trailing zero digits; zero is the empty digit vector).
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Big(pub Vec<u32>);

    impl Big {
        pub fn zero() -> Self {
            Big(vec![])
        }
        pub fn from_u64(v: u64) -> Self {
            if v == 0 {
                return Big::zero();
            }
            let mut d = vec![(v % 1_000_000_000) as u32];
            let mut v = v / 1_000_000_000;
            while v > 0 {
                d.push((v % 1_000_000_000) as u32);
                v /= 1_000_000_000;
            }
            Big(d)
        }
        pub fn from_u128(v: u128) -> Self {
            let hi = (v >> 64) as u64;
            let lo = v as u64;
            Big::from_u64(hi)
                .mul(&Big::pow2(64))
                .add(&Big::from_u64(lo))
        }
        /// Horner parse of 32 big-endian bytes.
        pub fn from_be_bytes32(b: &[u8; 32]) -> Self {
            let mut acc = Big::zero();
            for &byte in b.iter() {
                acc = acc.mul_small(256).add_small(byte as u32);
            }
            acc
        }
        /// Canonical 32 big-endian bytes; panics if the value is >= 2^256.
        pub fn to_be_bytes32(&self) -> [u8; 32] {
            assert!(self.cmp_big(&Big::pow2(256)) == std::cmp::Ordering::Less);
            let mut rem = if self.0.is_empty() {
                vec![0u32]
            } else {
                self.0.clone()
            };
            let mut out = [0u8; 32];
            for pos in (0..32).rev() {
                let mut carry = 0u64;
                let mut q = vec![0u32; rem.len()];
                for i in (0..rem.len()).rev() {
                    let cur = carry * 1_000_000_000 + rem[i] as u64;
                    q[i] = (cur / 256) as u32;
                    carry = cur % 256;
                }
                out[pos] = carry as u8;
                while q.len() > 1 && q.last() == Some(&0) {
                    q.pop();
                }
                rem = q;
            }
            assert!(rem == vec![0], "value did not fit 32 bytes");
            out
        }
        pub fn is_zero(&self) -> bool {
            self.0.is_empty()
        }
        pub fn mul_small(&self, m: u32) -> Self {
            let mut carry = 0u64;
            let mut d = Vec::with_capacity(self.0.len() + 1);
            for &x in &self.0 {
                let cur = x as u64 * m as u64 + carry;
                d.push((cur % 1_000_000_000) as u32);
                carry = cur / 1_000_000_000;
            }
            while carry > 0 {
                d.push((carry % 1_000_000_000) as u32);
                carry /= 1_000_000_000;
            }
            while d.last() == Some(&0) {
                d.pop();
            }
            Big(d)
        }
        pub fn add_small(&self, a: u32) -> Self {
            self.add(&Big(vec![a]))
        }
        pub fn sub_small(&self, a: u32) -> Self {
            self.sub(&Big::from_u64(a as u64))
        }
        pub fn add(&self, other: &Self) -> Self {
            let n = self.0.len().max(other.0.len());
            let mut d = Vec::with_capacity(n + 1);
            let mut carry = 0u64;
            for i in 0..n {
                let x = *self.0.get(i).unwrap_or(&0) as u64;
                let y = *other.0.get(i).unwrap_or(&0) as u64;
                let cur = x + y + carry;
                d.push((cur % 1_000_000_000) as u32);
                carry = cur / 1_000_000_000;
            }
            if carry > 0 {
                d.push(carry as u32);
            }
            while d.last() == Some(&0) {
                d.pop();
            }
            Big(d)
        }
        /// `self - other` for `self >= other` (panics otherwise).
        pub fn sub(&self, other: &Self) -> Self {
            assert!(self.cmp_big(other) != std::cmp::Ordering::Less);
            let mut d = Vec::with_capacity(self.0.len());
            let mut borrow = 0i64;
            for i in 0..self.0.len() {
                let x = self.0[i] as i64;
                let y = *other.0.get(i).unwrap_or(&0) as i64;
                let mut cur = x - y - borrow;
                if cur < 0 {
                    cur += 1_000_000_000;
                    borrow = 1;
                } else {
                    borrow = 0;
                }
                d.push(cur as u32);
            }
            while d.last() == Some(&0) {
                d.pop();
            }
            Big(d)
        }
        pub fn mul(&self, other: &Self) -> Self {
            if self.is_zero() || other.is_zero() {
                return Big::zero();
            }
            let mut d = vec![0u32; self.0.len() + other.0.len()];
            for (i, &x) in self.0.iter().enumerate() {
                let mut carry = 0u64;
                for (j, &y) in other.0.iter().enumerate() {
                    let cur = d[i + j] as u64 + x as u64 * y as u64 + carry;
                    d[i + j] = (cur % 1_000_000_000) as u32;
                    carry = cur / 1_000_000_000;
                }
                let mut k = i + other.0.len();
                while carry > 0 {
                    let cur = d[k] as u64 + carry;
                    d[k] = (cur % 1_000_000_000) as u32;
                    carry = cur / 1_000_000_000;
                    k += 1;
                }
            }
            while d.last() == Some(&0) {
                d.pop();
            }
            Big(d)
        }
        /// `2^n` by repeated doubling.
        pub fn pow2(n: u32) -> Self {
            let mut acc = Big(vec![1]);
            for _ in 0..n {
                acc = acc.add(&acc);
            }
            acc
        }
        /// `2^n - 1` for `n >= 1` by repeated `x -> 2x + 1`; `n == 0` is zero.
        pub fn low_ones(n: u32) -> Self {
            let mut acc = Big::zero();
            for _ in 0..n {
                acc = acc.add(&acc).add_small(1);
            }
            acc
        }
        pub fn cmp_big(&self, other: &Self) -> std::cmp::Ordering {
            match self.0.len().cmp(&other.0.len()) {
                std::cmp::Ordering::Equal => {}
                ord => return ord,
            }
            for i in (0..self.0.len()).rev() {
                match self.0[i].cmp(&other.0[i]) {
                    std::cmp::Ordering::Equal => {}
                    ord => return ord,
                }
            }
            std::cmp::Ordering::Equal
        }
    }

    impl PartialOrd for Big {
        fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp_big(other))
        }
    }

    /// MSB-first restoring long division over a bit vector (structurally
    /// different from the module's 512-bit limb division).
    pub fn div_rem_bits(num_bits_msb_first: &[bool], den: &Big) -> (Big, Big) {
        assert!(!den.is_zero());
        let mut rem = Big::zero();
        let mut quot = Big::zero();
        for &b in num_bits_msb_first {
            rem = rem.add(&rem);
            if b {
                rem = rem.add_small(1);
            }
            if rem.cmp_big(den) != std::cmp::Ordering::Less {
                rem = rem.sub(den);
                quot = quot.add(&quot).add_small(1);
            } else {
                quot = quot.add(&quot);
            }
        }
        (quot, rem)
    }

    /// Bit vector (MSB first) of `n * 2^k` for `n >= 1`.
    pub fn bits_of_n_shifted_k(n: u64, k: u32) -> Vec<bool> {
        let mut bits: Vec<bool> = (0..64).rev().map(|i| (n >> i) & 1 == 1).collect();
        while bits.first() != Some(&true) {
            bits.remove(0);
        }
        bits.extend(std::iter::repeat_n(false, k as usize));
        bits
    }

    /// Bit vector (MSB first) of a bignum.
    pub fn bits_msb(v: &Big) -> Vec<bool> {
        let bytes = v.to_be_bytes32();
        let mut bits: Vec<bool> = Vec::with_capacity(256);
        let mut started = false;
        for &b in bytes.iter() {
            for i in (0..8).rev() {
                let bit = (b >> i) & 1 == 1;
                if bit {
                    started = true;
                }
                if started {
                    bits.push(bit);
                }
            }
        }
        if bits.is_empty() {
            bits.push(false);
        }
        bits
    }
}

use oracle::Big;
use std::cmp::Ordering;

impl Big {
    /// Low 128 bits; panics if the value is >= 2^128.
    fn to_u128_value(&self) -> u128 {
        assert!(self.cmp_big(&Big::pow2(128)) == Ordering::Less);
        let bytes = self.to_be_bytes32();
        let mut v = 0u128;
        for &b in bytes[16..].iter() {
            v = (v << 8) | b as u128;
        }
        v
    }
    fn to_u64_value(&self) -> u64 {
        assert!(self.cmp_big(&Big::pow2(64)) == Ordering::Less);
        let bytes = self.to_be_bytes32();
        let mut v = 0u64;
        for &b in bytes[24..].iter() {
            v = (v << 8) | b as u64;
        }
        v
    }
}

/// Decode-classification oracle, replicating the consensus `SetCompact`
/// rules with independent bignum value computation.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum DecodeClass {
    Ok,
    Negative,
    Overflow,
    Zero,
}

fn oracle_decode(bits: u32) -> (DecodeClass, Big) {
    let size = bits >> 24;
    let word = bits & 0x007f_ffff;
    if word != 0 {
        if bits & 0x0080_0000 != 0 {
            return (DecodeClass::Negative, Big::zero());
        }
        if size > 34 || (word > 0xff && size > 33) || (word > 0xffff && size > 32) {
            return (DecodeClass::Overflow, Big::zero());
        }
    }
    let target = if size <= 3 {
        Big::from_u64((word >> (8 * (3 - size))) as u64)
    } else {
        let mut t = Big::from_u64(word as u64);
        for _ in 0..(size - 3) * 8 {
            t = t.add(&t);
        }
        t
    };
    if target.is_zero() {
        (DecodeClass::Zero, Big::zero())
    } else {
        (DecodeClass::Ok, target)
    }
}

fn classify(err: &WorkUnitError) -> DecodeClass {
    match err {
        WorkUnitError::CompactSign => DecodeClass::Negative,
        WorkUnitError::CompactOverflow => DecodeClass::Overflow,
        WorkUnitError::CompactZero => DecodeClass::Zero,
        other => panic!("unexpected decode error: {other}"),
    }
}

/// Oracle value of `ceil(N * 2^k / (T + 1))` via bit long division plus one
/// ceiling step on a nonzero remainder.
fn oracle_ceil_q(n: u64, k: u32, t: &[u8; 32]) -> Big {
    let num_bits = oracle::bits_of_n_shifted_k(n, k);
    let den = Big::from_be_bytes32(t).add_small(1);
    let (q, r) = oracle::div_rem_bits(&num_bits, &den);
    if r.is_zero() {
        q
    } else {
        q.add_small(1)
    }
}

// ---------------------------------------------------------------------------
// Frozen approved vectors
// ---------------------------------------------------------------------------

const REGTEST_POW_LIMIT_BITS: u32 = 0x207f_ffff;
const OVERLAY_BITS: u32 = 0x200f_ffff;

/// `2^n - 1` as 32 big-endian bytes via the oracle.
fn low_ones_bytes(n: u32) -> [u8; 32] {
    Big::low_ones(n).to_be_bytes32()
}

#[test]
fn fingerprint_frozen_vectors() {
    // R = 2^255 - 1 (the recommended lab reference).
    let r255 = low_ones_bytes(255);
    assert_eq!(
        hex::encode(work_unit_fingerprint(&r255)),
        "7fe9ed1a4e2d4279b302cbb0bcf709758f03dc44db6dab82ac40c04eb0d77c6b"
    );
    // R = 2^254 - 1 (the old negative vector).
    let r254 = low_ones_bytes(254);
    assert_eq!(
        hex::encode(work_unit_fingerprint(&r254)),
        "2fc4270b351c943cc49909e369f72df47e38ed48ee0f69bdf0bc1b2cb76a1859"
    );
    // The profile method agrees with the free function and does not depend
    // on the multiplier cap.
    let profile = WorkUnitProfile::lab_default();
    assert_eq!(profile.fingerprint(), work_unit_fingerprint(&r255));
    let other = WorkUnitProfile::from_exponent(255, 1).unwrap();
    assert_eq!(other.fingerprint(), profile.fingerprint());
}

#[test]
fn compact_decode_frozen_vectors() {
    // Accepted (decision test 5 and the Fable B3 vectors).
    for bits in [0x2100_ffffu32, 0x2200_00ff, 0x0212_3456, 0x207f_ffff] {
        let (class, expect) = oracle_decode(bits);
        assert_eq!(class, DecodeClass::Ok, "bits = {bits:#010x}");
        let t = decode_compact_target(bits).unwrap_or_else(|e| panic!("bits {bits:#010x}: {e}"));
        assert_eq!(
            t.to_be_bytes(),
            expect.to_be_bytes32(),
            "bits = {bits:#010x}"
        );
    }
    // 0x02123456 is the 13-bit target 0x1234.
    assert_eq!(
        decode_compact_target(0x0212_3456).unwrap().to_be_bytes()[30..],
        [0x12, 0x34]
    );
    // 0x207fffff is exactly the regtest pow limit 2^255 - 2^232.
    let pow_limit = decode_compact_target(REGTEST_POW_LIMIT_BITS).unwrap();
    assert_eq!(
        pow_limit.to_be_bytes(),
        Big::low_ones(255).sub(&Big::low_ones(232)).to_be_bytes32()
    );
    // Overflow rejection.
    assert_eq!(
        decode_compact_target(0x2101_0000),
        Err(WorkUnitError::CompactOverflow)
    );
    assert_eq!(
        decode_compact_target(0x2200_0100),
        Err(WorkUnitError::CompactOverflow)
    );
    // Zero rejection.
    assert_eq!(
        decode_compact_target(0x0012_3456),
        Err(WorkUnitError::CompactZero)
    );
    assert_eq!(
        decode_compact_target(0x0180_0000),
        Err(WorkUnitError::CompactZero)
    );
    // Sign rejection (word != 0 with the sign bit set).
    assert_eq!(
        decode_compact_target(0x2080_0001),
        Err(WorkUnitError::CompactSign)
    );
    // 0x20800000 has word == 0, so consensus rejects it via the zero rule,
    // not the sign rule; either way it fails closed.
    assert_eq!(
        decode_compact_target(0x2080_0000),
        Err(WorkUnitError::CompactZero)
    );
}

#[test]
fn family_frozen_vectors_for_reference_2_255_minus_1() {
    let profile = WorkUnitProfile::lab_default();
    assert_eq!(profile.exponent(), 255);
    assert_eq!(profile.reference_bytes(), low_ones_bytes(255));
    // Decision vector table: m = 1, 2, 4, 8, 16.
    for (j, expect_exp, expect_e) in [
        (0u32, 255u32, 2u128),
        (1, 254, 4),
        (2, 253, 8),
        (3, 252, 16),
        (4, 251, 32),
    ] {
        let s = profile.share_target(j).unwrap();
        assert_eq!(s.to_be_bytes(), low_ones_bytes(expect_exp), "j = {j}");
        // Exact family equation m * (S + 1) == R + 1 (oracle, no floats).
        let m = Big::pow2(j);
        let s1 = Big::from_be_bytes32(&s.to_be_bytes()).add_small(1);
        assert_eq!(m.mul(&s1), Big::pow2(255), "exact E ratio for j = {j}");
        // E(S) = 2^(256 - (k - j)) is an exact integer here.
        assert_eq!(expected_hashes_exact(&s), Some(expect_e));
    }
    // m = 16 (S = 2^251 - 1) is harder than 0x200fffff and must be rejected
    // for that generation.
    let t_overlay = decode_compact_target(OVERLAY_BITS).unwrap();
    let s16 = profile.share_target(4).unwrap();
    assert_eq!(
        share_target_is_acceptable(&s16, &t_overlay),
        Err(WorkUnitError::ShareTargetBelowNetwork)
    );
}

#[test]
fn profile_validity_vectors() {
    // R = 2^254 - 1 is below the regtest pow limit -> named error naming both.
    let err = WorkUnitProfile::from_exponent(254, 1 << 32)
        .unwrap()
        .validate_pow_limit(REGTEST_POW_LIMIT_BITS)
        .unwrap_err();
    match &err {
        WorkUnitError::ReferenceBelowPowLimit {
            reference,
            pow_limit,
        } => {
            assert_eq!(reference, &hex::encode(low_ones_bytes(254)));
            let pl = decode_compact_target(REGTEST_POW_LIMIT_BITS).unwrap();
            assert_eq!(pow_limit, &hex::encode(pl.to_be_bytes()));
        }
        other => panic!("expected ReferenceBelowPowLimit, got {other}"),
    }
    assert!(err.to_string().contains("reference_below_pow_limit"));
    // R = 2^255 - 1 passes.
    WorkUnitProfile::lab_default()
        .validate_pow_limit(REGTEST_POW_LIMIT_BITS)
        .unwrap();
    // R + 1 with two bits set is not in the family (R = 2^128 here).
    let mut non_power = [0u8; 32];
    non_power[15] = 1; // R = 2^128, so R + 1 = 2^128 + 1.
    assert_eq!(
        WorkUnitProfile::from_reference(non_power, 1 << 32),
        Err(WorkUnitError::ReferenceNotPowerOfTwo)
    );
    // R = 2^256 - 1: the all-ones sentinel; R + 1 overflows 256 bits.
    assert_eq!(
        WorkUnitProfile::from_reference([0xffu8; 32], 1 << 32),
        Err(WorkUnitError::ReferenceExponentOutOfRange(256))
    );
    // R = 0: k = 0 is outside the supported bounds.
    assert_eq!(
        WorkUnitProfile::from_reference([0u8; 32], 1 << 32),
        Err(WorkUnitError::ReferenceExponentOutOfRange(0))
    );
    // R = 2^255: R + 1 = 2^255 + 1, not a power of two.
    let mut k256 = [0u8; 32];
    k256[0] = 1;
    assert_eq!(
        WorkUnitProfile::from_reference(k256, 1 << 32),
        Err(WorkUnitError::ReferenceNotPowerOfTwo)
    );
    // from_exponent bounds: k = 0 and k = 256 rejected.
    assert!(WorkUnitProfile::from_exponent(0, 1 << 32).is_err());
    assert!(WorkUnitProfile::from_exponent(256, 1 << 32).is_err());
    assert!(WorkUnitProfile::from_exponent(1, 1 << 32).is_ok());
}

#[test]
fn multiplier_rejects_non_power_and_m3() {
    let profile = WorkUnitProfile::lab_default();
    // m_cfg = 3 (decision vector) -> multiplier_not_in_family.
    assert_eq!(
        profile.validate_multiplier(3),
        Err(WorkUnitError::MultiplierNotInFamily(3))
    );
    // m = 0.
    assert_eq!(
        profile.validate_multiplier(0),
        Err(WorkUnitError::MultiplierNotInFamily(0))
    );
    // m above the hard cap 2^32.
    assert_eq!(
        profile.validate_multiplier(1 << 33),
        Err(WorkUnitError::MultiplierAboveHardCap(1 << 33))
    );
    // m_cfg = 2^32 is allowed (exactly at the cap) when k - 1 >= 32.
    assert_eq!(profile.validate_multiplier(1 << 32), Ok(32));
    // j > k - 1 is outside the family (decision 5.6).
    let small = WorkUnitProfile::from_exponent(8, 1 << 32).unwrap();
    assert_eq!(
        small.validate_multiplier(1 << 8),
        Err(WorkUnitError::MultiplierNotInFamily(1 << 8))
    );
    assert_eq!(small.validate_multiplier(1 << 7), Ok(7));
    // The cap itself is bounded: max_work_multiplier > 2^32 is rejected.
    assert_eq!(
        WorkUnitProfile::from_exponent(255, (1 << 32) + 1),
        Err(WorkUnitError::MaxWorkMultiplierAboveHardCap((1 << 32) + 1))
    );
}

#[test]
fn issuance_stepdown_and_stepup_data() {
    let profile = WorkUnitProfile::lab_default();
    let t_overlay = decode_compact_target(OVERLAY_BITS).unwrap();
    let t_pow_limit = decode_compact_target(REGTEST_POW_LIMIT_BITS).unwrap();

    // m_cfg = 8 at 0x200fffff: no stepdown, m_issue = 8.
    let a = profile.issuance(3, t_overlay).unwrap();
    assert_eq!((a.j_cfg, a.j_issue, a.m_issue), (3, 3, 8));
    assert!(!a.stepped_down);
    assert_eq!(a.share_target.to_be_bytes(), low_ones_bytes(252));

    // A generation at the pow limit steps down to m_issue = 1.
    let b = profile.issuance(3, t_pow_limit).unwrap();
    assert_eq!((b.j_cfg, b.j_issue, b.m_issue), (3, 0, 1));
    assert!(b.stepped_down);
    assert_eq!(b.share_target.to_be_bytes(), low_ones_bytes(255));

    // The next generation at 0x200fffff uses m_cfg again (step-up): the
    // step-up event belongs to the front's generation state machine; the
    // precursor provides the data (stepped_down == false again).
    let c = profile.issuance(3, t_overlay).unwrap();
    assert_eq!(c.m_issue, 8);
    assert!(!c.stepped_down);

    // The old generation is untouched: `a` still carries m_issue = 8.
    assert_eq!(a.m_issue, 8);

    // The ceiling binds: j_cfg = 32 on 0x200fffff still gives j_issue = 3.
    let d = profile.issuance(32, t_overlay).unwrap();
    assert_eq!(d.j_issue, 3);
    assert!(d.stepped_down);
    // j_cfg = 0 at the pow limit stays 0.
    let e = profile.issuance(0, t_pow_limit).unwrap();
    assert_eq!(e.j_issue, 0);
    assert!(!e.stepped_down);
}

#[test]
fn issuance_exact_power_boundary_selector_proof() {
    let profile = WorkUnitProfile::lab_default();
    // T = 2^252 - 1: T + 1 = 2^252, ceil_log2(T + 1) = 252, j_issue = 3,
    // m_issue = 8, and S == T exactly (the power-of-two edge).
    let t_minus = low_ones_bytes(252);
    let a = profile.issuance(3, Target::from_be_bytes(t_minus)).unwrap();
    assert_eq!((a.j_issue, a.m_issue), (3, 8));
    assert_eq!(a.share_target.to_be_bytes(), t_minus);

    // T = 2^252: ceil_log2(T + 1) = 253, j_issue = 2, m_issue = 4, S > T.
    let mut t_exact = [0u8; 32];
    t_exact[0] = 0x10; // byte 0 covers bits 248..255; 0x10 sets bit 252.
    let b = profile.issuance(3, Target::from_be_bytes(t_exact)).unwrap();
    assert_eq!((b.j_issue, b.m_issue), (2, 4));
    assert_eq!(b.share_target.to_be_bytes(), low_ones_bytes(253));
    assert!(b.share_target > Target::from_be_bytes(t_exact));
}

#[test]
fn issuance_defensive_unplannable_and_zero_target() {
    // A network target harder than every family member is a defensive
    // fail-closed error (unreachable for a validated profile).
    let small = WorkUnitProfile::from_exponent(8, 1 << 32).unwrap();
    let mut t_hard = [0u8; 32];
    t_hard[22] = 0x04; // byte 22 covers bits 72..79; 0x04 sets bit 74.
    assert_eq!(
        small.issuance(7, Target::from_be_bytes(t_hard)),
        Err(WorkUnitError::TargetProfileUnplannable)
    );
    // Zero network target.
    assert_eq!(
        small.issuance(7, Target::from_be_bytes([0u8; 32])),
        Err(WorkUnitError::NetworkTargetZero)
    );
}

#[test]
fn share_target_acceptability() {
    let profile = WorkUnitProfile::lab_default();
    let t_overlay = decode_compact_target(OVERLAY_BITS).unwrap();
    // S(m = 8) = 2^252 - 1 >= T is acceptable.
    let s8 = profile.share_target(3).unwrap();
    share_target_is_acceptable(&s8, &t_overlay).unwrap();
    // S(m = 16) = 2^251 - 1 < T is rejected, never clamped.
    let s16 = profile.share_target(4).unwrap();
    assert_eq!(
        share_target_is_acceptable(&s16, &t_overlay),
        Err(WorkUnitError::ShareTargetBelowNetwork)
    );
    // A zero share target is invalid.
    assert_eq!(
        share_target_is_acceptable(&Target::from_be_bytes([0u8; 32]), &t_overlay),
        Err(WorkUnitError::ShareTargetZero)
    );
    // A share target exactly equal to the network target is acceptable.
    share_target_is_acceptable(&t_overlay, &t_overlay).unwrap();
}

#[test]
fn multiplier_for_share_target_exact_ratios() {
    let profile = WorkUnitProfile::lab_default();
    for j in 0..8u32 {
        let s = profile.share_target(j).unwrap();
        assert_eq!(profile.multiplier_for_share_target(&s).unwrap(), 1u64 << j);
    }
    // A non-family target is refused, never rounded.
    let t_overlay = decode_compact_target(OVERLAY_BITS).unwrap();
    assert!(matches!(
        profile.multiplier_for_share_target(&t_overlay),
        Err(WorkUnitError::MultiplierNotInFamily(_))
    ));
    // A zero target has no multiplier.
    assert_eq!(
        profile.multiplier_for_share_target(&Target::from_be_bytes([0u8; 32])),
        Err(WorkUnitError::ShareTargetZero)
    );
}

#[test]
fn window_network_target_frozen_vectors() {
    let profile = WorkUnitProfile::lab_default();
    let t_overlay = decode_compact_target(OVERLAY_BITS).unwrap();
    let t_pow_limit = decode_compact_target(REGTEST_POW_LIMIT_BITS).unwrap();

    // Q(0x200fffff) = 65 (floor 64 must not be used).
    let q = window_target_network_work(&profile, t_overlay, 8, 1u128 << 64).unwrap();
    assert_eq!(q, 65);
    assert_ne!(q, 64);
    // Q(0x207fffff) = 9 (floor 8 must not be used).
    let q2 = window_target_network_work(&profile, t_pow_limit, 8, 1u128 << 64).unwrap();
    assert_eq!(q2, 9);
    assert_ne!(q2, 8);

    // Ceiling error bound 0 <= Q*E(R) - N*E_block(T) < E(R), computed
    // exactly: with R + 1 = 2^255 and E(R) = 2 hashes this is
    // 0 <= 2Q(T + 1) - 8*2^256 < 2(T + 1).
    for (t_bytes, q) in [
        (t_overlay.to_be_bytes(), 65u128),
        (t_pow_limit.to_be_bytes(), 9),
    ] {
        let den = Big::from_be_bytes32(&t_bytes).add_small(1);
        let err_num = Big::from_u128(2 * q)
            .mul(&den)
            .sub(&Big::pow2(256).mul_small(8));
        assert!(
            err_num.cmp_big(&Big::zero()) != Ordering::Less,
            "error bound not >= 0"
        );
        let two_den = den.add(&den);
        assert!(
            err_num.cmp_big(&two_den) == Ordering::Less,
            "error bound not < E(R)"
        );
    }
}

#[test]
fn window_network_target_rejections() {
    let profile = WorkUnitProfile::lab_default();
    let t_overlay = decode_compact_target(OVERLAY_BITS).unwrap();
    // N = 0 is a configuration error.
    assert_eq!(
        window_target_network_work(&profile, t_overlay, 0, 1u128 << 64),
        Err(WorkUnitError::WindowNZero)
    );
    // Zero network target.
    assert_eq!(
        window_target_network_work(&profile, Target::from_be_bytes([0u8; 32]), 8, 1u128 << 64),
        Err(WorkUnitError::NetworkTargetZero)
    );
    // Above the hard cap.
    assert_eq!(
        window_target_network_work(&profile, t_overlay, 8, 64),
        Err(WorkUnitError::WindowTargetTooLarge {
            computed: Some(65),
            cap: 64
        })
    );
    // A cap above 2^64 is rejected at the call boundary.
    assert_eq!(
        window_target_network_work(&profile, t_overlay, 8, (1u128 << 64) + 1),
        Err(WorkUnitError::MaxWindowTargetAboveHardCap(
            (1u128 << 64) + 1
        ))
    );
    // Q above u128::MAX fails closed (never saturates): T = 1, N = 2^63.
    let mut t_one = [0u8; 32];
    t_one[31] = 1;
    let err =
        window_target_network_work(&profile, Target::from_be_bytes(t_one), 1 << 63, 1u128 << 64)
            .unwrap_err();
    assert!(err.to_string().contains("window_target_too_large"));
}

#[test]
fn window_network_selection_crossing_overshoot_underfill() {
    // Decision vector: slices with difficulty 10, 10, 10 and Q = 25 take all
    // three slices, actual = 30, overshoot = 5.
    let slices = [(1u64, 10u128), (2, 10), (3, 10)];
    let sel = accumulate_slice_work(&slices, 25).unwrap();
    assert_eq!(sel.slice_ids, vec![1, 2, 3]);
    assert_eq!(sel.actual_sum, 30);
    assert_eq!(sel.overshoot, 5);
    assert_eq!(sel.underfill, 0);
    assert!(!sel.underfilled);

    // A slice exactly at the threshold is included; the walk stops there.
    let sel = accumulate_slice_work(&slices, 20).unwrap();
    assert_eq!(sel.slice_ids, vec![1, 2]);
    assert_eq!(sel.actual_sum, 20);
    assert_eq!(sel.overshoot, 0);
    assert!(!sel.underfilled);

    // Underfill: the tail ends before Q.
    let sel = accumulate_slice_work(&[(7u64, 10u128), (8, 5)], 25).unwrap();
    assert_eq!(sel.slice_ids, vec![7, 8]);
    assert_eq!(sel.actual_sum, 15);
    assert_eq!(sel.underfill, 10);
    assert_eq!(sel.overshoot, 0);
    assert!(sel.underfilled);

    // Empty tail with Q >= 1 is underfilled with zero work.
    let sel = accumulate_slice_work(&[], 25).unwrap();
    assert!(sel.slice_ids.is_empty());
    assert_eq!(sel.actual_sum, 0);
    assert_eq!(sel.underfill, 25);
    assert!(sel.underfilled);

    // Zero window target fails closed (Q >= 1 by construction).
    assert_eq!(
        accumulate_slice_work(&slices, 0),
        Err(WorkUnitError::WindowTargetZero)
    );
    // Window work accumulation overflow fails closed.
    let huge = [(1u64, u128::MAX - 1), (2, 2)];
    assert_eq!(
        accumulate_slice_work(&huge, u128::MAX),
        Err(WorkUnitError::WindowWorkOverflow)
    );
}

#[test]
fn accounting_overflow_checked_helper_set() {
    // slice_work_overflow.
    assert_eq!(
        checked_add_slice_work(u128::MAX, 1),
        Err(WorkUnitError::SliceWorkOverflow)
    );
    assert_eq!(checked_add_slice_work(1, 2), Ok(3));
    // slice_fee_work_overflow (d * f). Note `u64 * u64` always fits u128,
    // so the checked multiply takes u128 operands (building block for the
    // wider fee aggregates).
    assert_eq!(
        checked_fee_work(1u128 << 100, 1u128 << 100),
        Err(WorkUnitError::SliceFeeWorkOverflow)
    );
    // (2^64) * (2^64 - 1) = 2^128 - 2^64 fits exactly.
    assert_eq!(
        checked_fee_work(1u128 << 64, (1u128 << 64) - 1),
        Ok(u128::MAX - ((1u128 << 64) - 1))
    );
    // diff_fee_sum accumulation.
    assert_eq!(
        checked_add_fee_work(u128::MAX, 1),
        Err(WorkUnitError::SliceFeeWorkOverflow)
    );
    // window_work_overflow.
    assert_eq!(
        checked_add_window_work(u128::MAX, 1),
        Err(WorkUnitError::WindowWorkOverflow)
    );
}

#[test]
fn accounting_overflow_mul_div_exact_against_oracle() {
    // Small values agree with native arithmetic.
    assert_eq!(checked_mul_div_u128(10, 7, 3), Ok(23));
    // Exact wide product: (2^64 + 1) * (2^64 + 3) / 7 against the oracle.
    let a = (1u128 << 64) + 1;
    let b = (1u128 << 64) + 3;
    let product = Big::from_u128(a).mul(&Big::from_u128(b));
    let (q_big, r_big) = oracle::div_rem_bits(&oracle::bits_msb(&product), &Big::from_u128(7));
    assert_eq!(
        checked_mul_div_u128(a, b, 7).unwrap(),
        q_big.to_u128_value()
    );
    assert!(
        !r_big.is_zero(),
        "the quotient is floored; remainder nonzero here"
    );
    // Quotient overflow fails closed.
    assert_eq!(
        checked_mul_div_u128(u128::MAX, u128::MAX, 1),
        Err(WorkUnitError::PayoutProductOverflow)
    );
    // Max product with max denominator: (2^128-1)^2 / (2^128-1) = 2^128-1.
    assert_eq!(
        checked_mul_div_u128(u128::MAX, u128::MAX, u128::MAX),
        Ok(u128::MAX)
    );
    // Zero denominator fails closed.
    assert_eq!(
        checked_mul_div_u128(1, 1, 0),
        Err(WorkUnitError::PayoutProductOverflow)
    );
    // Zero numerator is zero.
    assert_eq!(checked_mul_div_u128(0, u128::MAX, 5), Ok(0));
}

#[test]
fn version_and_window_rule_enums() {
    assert_eq!(WorkUnitVersion::V1.as_u8(), 1);
    assert_eq!(WorkUnitVersion::V2.as_u8(), 2);
    assert_eq!(WorkUnitVersion::from_u8(1), Ok(WorkUnitVersion::V1));
    assert_eq!(WorkUnitVersion::from_u8(2), Ok(WorkUnitVersion::V2));
    assert!(WorkUnitVersion::from_u8(0).is_err());
    assert!(WorkUnitVersion::from_u8(3).is_err());
    assert_eq!(WindowRule::ShareWorkLegacy.as_u8(), 0);
    assert_eq!(WindowRule::NetworkBlockWork.as_u8(), 1);
    assert_eq!(WindowRule::from_u8(0), Ok(WindowRule::ShareWorkLegacy));
    assert_eq!(WindowRule::from_u8(1), Ok(WindowRule::NetworkBlockWork));
    assert!(WindowRule::from_u8(2).is_err());
    // Valid and invalid version/rule combinations fail closed.
    assert!(
        check_version_rule_compatibility(WorkUnitVersion::V1, WindowRule::ShareWorkLegacy).is_ok()
    );
    assert!(
        check_version_rule_compatibility(WorkUnitVersion::V2, WindowRule::NetworkBlockWork).is_ok()
    );
    assert!(
        check_version_rule_compatibility(WorkUnitVersion::V2, WindowRule::ShareWorkLegacy).is_err()
    );
    assert!(
        check_version_rule_compatibility(WorkUnitVersion::V1, WindowRule::NetworkBlockWork)
            .is_err()
    );
}

#[test]
fn expected_hashes_exact_only_for_power_of_two_plus_one() {
    let profile = WorkUnitProfile::lab_default();
    assert_eq!(expected_hashes_exact(&profile.reference_target()), Some(2));
    let t_overlay = decode_compact_target(OVERLAY_BITS).unwrap();
    // T + 1 = 0xfffff * 2^232 + 1 is not a power of two: no exact integer.
    assert_eq!(expected_hashes_exact(&t_overlay), None);
    // The all-ones target has E = 1.
    assert_eq!(
        expected_hashes_exact(&Target::from_be_bytes([0xffu8; 32])),
        Some(1)
    );
}

#[test]
fn target_ordering_and_bytes() {
    let profile = WorkUnitProfile::lab_default();
    assert!(profile.reference_target() > profile.share_target(1).unwrap());
    assert!(profile.share_target(1).unwrap() > profile.share_target(2).unwrap());
    assert!(Target::from_be_bytes([0u8; 32]) < profile.share_target(254).unwrap());
    // Round trip.
    let t = decode_compact_target(REGTEST_POW_LIMIT_BITS).unwrap();
    assert_eq!(Target::from_be_bytes(t.to_be_bytes()), t);
}

/// The v2 module has no floating point in code: the decision requires that
/// target decode, family arithmetic and target comparison never touch a
/// float type. Comment text is stripped before scanning so documentation
/// cannot trip the check.
#[test]
fn work_unit_module_has_no_floating_point() {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let path = std::path::Path::new(manifest).join("src/work_unit.rs");
    let src = std::fs::read_to_string(path).expect("work_unit.rs exists");
    let code: String = src
        .lines()
        .map(|line| match line.find("//") {
            Some(pos) => &line[..pos],
            None => line,
        })
        .collect();
    for token in ["f64", "f32", "as f", "difficulty_float"] {
        assert!(
            !code.contains(token),
            "v2 module code must not contain `{token}`"
        );
    }
}

// ---------------------------------------------------------------------------
// Property tests against the independent oracle
// ---------------------------------------------------------------------------

proptest! {
    /// Compact decode matches the independent consensus oracle for every
    /// classification (accept / sign / overflow / zero) and value.
    #[test]
    fn prop_decode_matches_oracle(bits: u32) {
        let (class, expect) = oracle_decode(bits);
        match decode_compact_target(bits) {
            Ok(t) => {
                prop_assert_eq!(class, DecodeClass::Ok);
                prop_assert_eq!(t.to_be_bytes(), expect.to_be_bytes32());
            }
            Err(e) => {
                prop_assert_eq!(class, classify(&e));
                prop_assert!(expect.is_zero());
            }
        }
    }

    /// Family arithmetic and the O(1) issuance selector match the oracle:
    /// S(k, j) == 2^(k-j) - 1, m*(S+1) == R+1 exactly, and j_issue equals
    /// the brute-force largest feasible family exponent (computed by direct
    /// bignum target comparison, not by bit-length arithmetic).
    #[test]
    fn prop_family_and_issuance_match_oracle(
        k in 1u32..=255,
        j_cfg in 0u32..=32,
        t in proptest::array::uniform32(any::<u8>()),
    ) {
        let profile = WorkUnitProfile::from_exponent(k, 1u64 << 32).unwrap();
        let j_eff = j_cfg.min(k - 1);
        let s = profile.share_target(j_eff).unwrap();
        prop_assert_eq!(s.to_be_bytes(), Big::low_ones(k - j_eff).to_be_bytes32());
        let m = Big::pow2(j_eff);
        let s1 = Big::from_be_bytes32(&s.to_be_bytes()).add_small(1);
        prop_assert_eq!(m.mul(&s1), Big::pow2(k));

        let t_target = Target::from_be_bytes(t);
        let t_big = Big::from_be_bytes32(&t);
        let issuance = profile.issuance(j_eff, t_target);
        if t_big.is_zero() {
            prop_assert_eq!(issuance, Err(WorkUnitError::NetworkTargetZero));
        } else {
            // Brute-force oracle: the largest j' in 0..=j_eff with S(j') >= T.
            let oracle_j = (0..=j_eff)
                .filter(|&jp| Big::low_ones(k - jp).cmp_big(&t_big) != Ordering::Less)
                .max();
            match (issuance, oracle_j) {
                (Ok(is), Some(oj)) => {
                    prop_assert_eq!(is.j_issue, oj);
                    prop_assert_eq!(is.m_issue, Big::pow2(oj).to_u64_value());
                    prop_assert_eq!(
                        is.share_target.to_be_bytes(),
                        Big::low_ones(k - oj).to_be_bytes32()
                    );
                    prop_assert_eq!(is.stepped_down, oj < j_eff);
                    // The recorded target is acceptable for this generation.
                    prop_assert!(share_target_is_acceptable(&is.share_target, &t_target).is_ok());
                }
                (Err(WorkUnitError::TargetProfileUnplannable), None) => {}
                (other, oj) => prop_assert!(false, "issuance {other:?} vs oracle {oj:?}"),
            }
        }
    }

    /// Q(T) = ceil(N*(R+1)/(T+1)) matches the independent oracle, including
    /// the fail-closed rejections above the cap and above u128.
    #[test]
    fn prop_window_target_matches_oracle(
        k in 1u32..=255,
        n in 1u64..=1_000_000,
        t in proptest::array::uniform32(any::<u8>()),
    ) {
        let profile = WorkUnitProfile::from_exponent(k, 1u64 << 32).unwrap();
        let cap = 1u128 << 64;
        let result = window_target_network_work(&profile, Target::from_be_bytes(t), n, cap);
        if t == [0u8; 32] {
            prop_assert_eq!(result, Err(WorkUnitError::NetworkTargetZero));
            return Ok(());
        }
        let q = oracle_ceil_q(n, k, &t);
        let over_u128 = q.cmp_big(&Big::pow2(128)) != Ordering::Less;
        let over_cap = q.cmp_big(&Big::from_u128(cap)) == Ordering::Greater;
        if over_u128 || over_cap {
            prop_assert!(result.is_err(), "expected window_target_too_large");
            prop_assert!(result.unwrap_err().to_string().contains("window_target_too_large"));
        } else {
            let got = result.unwrap();
            prop_assert_eq!(got, q.to_u128_value());
            // Independent characterization of the ceiling: q*den >= num and
            // (q-1)*den < num, checked entirely in the oracle.
            let num = big_from_bits(&oracle::bits_of_n_shifted_k(n, k));
            let den = Big::from_be_bytes32(&t).add_small(1);
            let q_big = Big::from_u128(got);
            prop_assert!(q_big.mul(&den).cmp_big(&num) != Ordering::Less);
            prop_assert!(q_big.sub_small(1).mul(&den).cmp_big(&num) == Ordering::Less);
        }
    }

    /// Exact wide multiply/divide matches the oracle for the floor quotient.
    #[test]
    fn prop_mul_div_matches_oracle(
        a in any::<u128>(),
        b in any::<u128>(),
        den in 1u128..,
    ) {
        let result = checked_mul_div_u128(a, b, den);
        let product = Big::from_u128(a).mul(&Big::from_u128(b));
        let d = Big::from_u128(den);
        let (q, _r) = oracle::div_rem_bits(&oracle::bits_msb(&product), &d);
        if q.cmp_big(&Big::pow2(128)) != Ordering::Less {
            prop_assert_eq!(result, Err(WorkUnitError::PayoutProductOverflow));
        } else {
            prop_assert_eq!(result.unwrap(), q.to_u128_value());
        }
    }

    /// Every family member S(k, j) equals the oracle's 2^(k-j)-1 and the
    /// expected-work ratio E(S)/E(R) is exactly 2^j, failing closed with
    /// `Overflow` when the exact ratio does not fit `u64` (only possible
    /// for a foreign target, never for an issued multiplier at or below
    /// the 2^32 cap).
    #[test]
    fn prop_expected_work_ratio_exact(k in 1u32..=255, j in 0u32..=254) {
        let j_eff = j.min(k - 1);
        let profile = WorkUnitProfile::from_exponent(k, 1u64 << 32).unwrap();
        let s = profile.share_target(j_eff).unwrap();
        let ratio = profile.multiplier_for_share_target(&s);
        prop_assert_eq!(
            Big::pow2(j_eff).mul(&Big::from_be_bytes32(&s.to_be_bytes()).add_small(1)),
            Big::pow2(k)
        );
        if j_eff > 63 {
            prop_assert_eq!(ratio, Err(WorkUnitError::Overflow));
        } else {
            prop_assert_eq!(ratio.unwrap(), Big::pow2(j_eff).to_u64_value());
        }
    }
}

fn big_from_bits(bits: &[bool]) -> Big {
    let mut acc = Big::zero();
    for &b in bits {
        acc = acc.add(&acc);
        if b {
            acc = acc.add_small(1);
        }
    }
    acc
}
