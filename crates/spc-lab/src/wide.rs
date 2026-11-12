//! Exact integer helpers for targets, work units and window thresholds.
//!
//! The lab does not use `f64` to decide whether a hash meets a target or how
//! much a share is worth. Targets are 256-bit values and wide counters are
//! serialized as decimal strings so a JavaScript client cannot round them.

use std::fmt;
use std::str::FromStr;

use num_bigint::BigUint;
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};

/// Errors from exact target/work arithmetic.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WideError {
    #[error("invalid hex target")]
    InvalidHex,
    #[error("target is zero")]
    ZeroTarget,
    #[error("compact target uses the sign bit")]
    CompactSignBit,
    #[error("compact target overflows 256 bits")]
    CompactOverflow,
    #[error("reference target is not of the form 2^k - 1")]
    ReferenceNotPowerOfTwo,
    #[error("reference target must have 1 <= k <= 255, got k={0}")]
    ReferenceOutOfRange(u32),
    #[error("multiplier is not a power of two")]
    MultiplierNotPowerOfTwo,
    #[error("multiplier exponent {j} is outside 0..={max}")]
    MultiplierOutOfRange { j: u32, max: u32 },
    #[error("network target is above the reference target")]
    TargetAboveReference,
    #[error("window target does not fit the supported width")]
    WindowTooLarge,
    #[error("decimal value is malformed or out of range")]
    WideInteger,
}

/// A 256-bit unsigned target, stored big-endian.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Target256([u8; 32]);

impl Target256 {
    pub fn zero() -> Self {
        Self([0u8; 32])
    }

    pub fn from_bytes_be(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn from_slice_be(bytes: &[u8]) -> Result<Self, WideError> {
        if bytes.len() > 32 {
            return Err(WideError::InvalidHex);
        }
        let mut out = [0u8; 32];
        out[32 - bytes.len()..].copy_from_slice(bytes);
        Ok(Self(out))
    }

    pub fn from_biguint(value: &BigUint) -> Result<Self, WideError> {
        if value.bits() > 256 {
            return Err(WideError::InvalidHex);
        }
        Self::from_slice_be(&value.to_bytes_be())
    }

    pub fn from_hex(value: &str) -> Result<Self, WideError> {
        let value = value.trim();
        let value = value
            .strip_prefix("0x")
            .or_else(|| value.strip_prefix("0X"))
            .unwrap_or(value);
        if value.is_empty() || value.len() > 64 {
            return Err(WideError::InvalidHex);
        }
        let padded = format!("{value:0>64}");
        let bytes = hex::decode(padded).map_err(|_| WideError::InvalidHex)?;
        let mut out = [0u8; 32];
        out.copy_from_slice(&bytes);
        Ok(Self(out))
    }

    /// Decode the consensus compact representation (`SetCompact` rules).
    pub fn from_compact(bits: u32) -> Result<Self, WideError> {
        let size = (bits >> 24) as i32;
        let word = bits & 0x007f_ffff;
        if word != 0 && (bits & 0x0080_0000) != 0 {
            return Err(WideError::CompactSignBit);
        }
        if word != 0 && (size > 34 || (word > 0xff && size > 33) || (word > 0xffff && size > 32)) {
            return Err(WideError::CompactOverflow);
        }
        let word = BigUint::from(word);
        let target = if size <= 3 {
            word >> (8 * (3 - size) as u32)
        } else {
            word << (8 * (size - 3) as u32)
        };
        if target.bits() == 0 {
            return Err(WideError::ZeroTarget);
        }
        if target.bits() > 256 {
            return Err(WideError::CompactOverflow);
        }
        Self::from_biguint(&target)
    }

    pub fn to_biguint(self) -> BigUint {
        BigUint::from_bytes_be(&self.0)
    }

    pub fn is_zero(self) -> bool {
        self.0.iter().all(|b| *b == 0)
    }

    pub fn as_bytes_be(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(self) -> String {
        hex::encode(self.0)
    }

    /// Numeric comparison of a SHA256d digest. Bitcoin interprets the digest
    /// as a little-endian number for proof-of-work, which is why a displayed
    /// block hash is the reversed digest.
    pub fn hash_meets(&self, digest: &[u8]) -> bool {
        let hash = BigUint::from_bytes_le(digest);
        hash <= self.to_biguint()
    }
}

impl fmt::Debug for Target256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Target256(0x{})", self.to_hex())
    }
}

impl fmt::Display for Target256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl FromStr for Target256 {
    type Err = WideError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::from_hex(value)
    }
}

impl Serialize for Target256 {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Target256 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::from_hex(&value).map_err(de::Error::custom)
    }
}

/// A `u128` that serializes as a decimal string.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WideU128(pub u128);

impl WideU128 {
    pub fn get(self) -> u128 {
        self.0
    }
}

impl From<u128> for WideU128 {
    fn from(value: u128) -> Self {
        Self(value)
    }
}

impl From<u64> for WideU128 {
    fn from(value: u64) -> Self {
        Self(u128::from(value))
    }
}

impl fmt::Display for WideU128 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl Serialize for WideU128 {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0.to_string())
    }
}

struct WideVisitor;

impl<'de> Visitor<'de> for WideVisitor {
    type Value = WideU128;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a decimal string containing a u128")
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        value.parse::<u128>().map(WideU128).map_err(E::custom)
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
        Ok(WideU128(u128::from(value)))
    }

    fn visit_u128<E: de::Error>(self, value: u128) -> Result<Self::Value, E> {
        Ok(WideU128(value))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
        if value < 0 {
            return Err(E::custom("wide integer must not be negative"));
        }
        Ok(WideU128(value as u128))
    }
}

impl<'de> Deserialize<'de> for WideU128 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(WideVisitor)
    }
}

/// Error detail used when a profile is checked against a network target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileCheckError {
    pub code: &'static str,
    pub detail: String,
}

/// The exact power-of-two v2 reference family.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReferenceProfile {
    k: u32,
    r_plus_one: BigUint,
    reference: Target256,
}

impl ReferenceProfile {
    pub fn parse_hex(value: &str) -> Result<Self, ProfileCheckError> {
        let reference = Target256::from_hex(value).map_err(|e| ProfileCheckError {
            code: "reference_not_power_of_two",
            detail: e.to_string(),
        })?;
        let r = reference.to_biguint();
        let r_plus_one = &r + BigUint::from(1u32);
        if !is_power_of_two(&r_plus_one) {
            return Err(ProfileCheckError {
                code: "reference_not_power_of_two",
                detail: format!(
                    "R + 1 is not an exact power of two for R=0x{}",
                    reference.to_hex()
                ),
            });
        }
        let k = r_plus_one.bits() - 1;
        if !(1..=255).contains(&k) {
            return Err(ProfileCheckError {
                code: "reference_not_power_of_two",
                detail: format!("k={k} is outside 1..=255"),
            });
        }
        Ok(Self {
            k: k as u32,
            r_plus_one,
            reference,
        })
    }

    pub fn k(&self) -> u32 {
        self.k
    }

    pub fn reference(&self) -> Target256 {
        self.reference
    }

    pub fn r_plus_one(&self) -> &BigUint {
        &self.r_plus_one
    }

    /// `S(2^j) = 2^(k-j) - 1`.
    pub fn share_target(&self, j: u32) -> Result<Target256, WideError> {
        if j > self.k.saturating_sub(1) {
            return Err(WideError::MultiplierOutOfRange {
                j,
                max: self.k.saturating_sub(1),
            });
        }
        let value = (BigUint::from(1u32) << (self.k - j)) - BigUint::from(1u32);
        Target256::from_biguint(&value)
    }

    /// `j_issue = min(j_cfg, k - ceil_log2(T + 1))`.
    pub fn issuance_exponent(
        &self,
        j_cfg: u32,
        network_target: &BigUint,
    ) -> Result<u32, WideError> {
        if network_target.bits() == 0 {
            return Err(WideError::ZeroTarget);
        }
        if j_cfg > self.k.saturating_sub(1) {
            return Err(WideError::MultiplierOutOfRange {
                j: j_cfg,
                max: self.k.saturating_sub(1),
            });
        }
        let t_plus_one = network_target + BigUint::from(1u32);
        let bits = ceil_log2(&t_plus_one);
        if bits > self.k {
            return Err(WideError::TargetAboveReference);
        }
        Ok(j_cfg.min(self.k - bits))
    }

    /// `Q(T) = ceil(N * (R + 1) / (T + 1))`.
    pub fn window_target(&self, n: u64, network_target: &BigUint) -> Result<BigUint, WideError> {
        if network_target.bits() == 0 {
            return Err(WideError::ZeroTarget);
        }
        if n == 0 {
            return Err(WideError::WindowTooLarge);
        }
        let numerator = BigUint::from(n) * &self.r_plus_one;
        let denominator = network_target + BigUint::from(1u32);
        let result = (numerator + &denominator - BigUint::from(1u32)) / denominator;
        if result.bits() > 128 {
            return Err(WideError::WindowTooLarge);
        }
        Ok(result)
    }

    /// `sha256("spc-lab/work-unit/v2" || 0x00 || R_be32)`.
    pub fn fingerprint(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"spc-lab/work-unit/v2");
        hasher.update([0u8]);
        hasher.update(self.reference.as_bytes_be());
        hex::encode(hasher.finalize())
    }
}

pub fn is_power_of_two(value: &BigUint) -> bool {
    let bits = value.bits();
    if bits == 0 {
        return false;
    }
    let one = BigUint::from(1u32);
    (value & (value - &one)).bits() == 0
}

/// `(n - 1).bit_length()` for `n >= 1`.
pub fn ceil_log2(n: &BigUint) -> u32 {
    if n.bits() == 0 {
        return 0;
    }
    let one = BigUint::from(1u32);
    (n - &one).bits() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_decode_matches_the_consensus_rule() {
        assert_eq!(
            Target256::from_compact(0x2100ffff)
                .unwrap()
                .to_biguint()
                .bits(),
            256
        );
        assert_eq!(
            Target256::from_compact(0x220000ff)
                .unwrap()
                .to_biguint()
                .bits(),
            256
        );
        assert_eq!(
            Target256::from_compact(0x21010000),
            Err(WideError::CompactOverflow)
        );
        assert_eq!(
            Target256::from_compact(0x22000100),
            Err(WideError::CompactOverflow)
        );
        assert_eq!(
            Target256::from_compact(0x00123456),
            Err(WideError::ZeroTarget)
        );
        assert_eq!(
            Target256::from_compact(0x20800000),
            Err(WideError::ZeroTarget)
        );
        assert_eq!(
            Target256::from_compact(0x20800001),
            Err(WideError::CompactSignBit)
        );
        assert_eq!(
            Target256::from_compact(0x02123456).unwrap().to_biguint(),
            BigUint::from(0x1234u32)
        );
        assert_eq!(
            Target256::from_compact(0x207fffff).unwrap(),
            Target256::from_hex("7fffff0000000000000000000000000000000000000000000000000000000000")
                .unwrap()
        );
    }

    #[test]
    fn reference_profile_vectors() {
        let r = ReferenceProfile::parse_hex(
            "7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        )
        .unwrap();
        assert_eq!(r.k(), 255);
        assert_eq!(r.share_target(8).unwrap().to_biguint().bits(), 247);
        let t = Target256::from_compact(0x200fffff).unwrap().to_biguint();
        assert_eq!(r.issuance_exponent(3, &t).unwrap(), 3);
        assert_eq!(r.window_target(8, &t).unwrap(), BigUint::from(65u32));
        let pl = Target256::from_compact(0x207fffff).unwrap().to_biguint();
        assert_eq!(r.issuance_exponent(3, &pl).unwrap(), 0);
        assert_eq!(r.window_target(8, &pl).unwrap(), BigUint::from(9u32));
        assert_eq!(
            r.fingerprint(),
            "7fe9ed1a4e2d4279b302cbb0bcf709758f03dc44db6dab82ac40c04eb0d77c6b"
        );
    }

    #[test]
    fn non_power_reference_is_rejected() {
        assert!(ReferenceProfile::parse_hex(
            "fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe"
        )
        .is_err());
    }

    #[test]
    fn wide_u128_round_trips_as_a_string() {
        let value = WideU128(u128::MAX);
        let json = serde_json::to_string(&value).unwrap();
        assert_eq!(json, format!("\"{}\"", u128::MAX));
        let back: WideU128 = serde_json::from_str(&json).unwrap();
        assert_eq!(back, value);
        assert!(serde_json::from_str::<WideU128>("\"-1\"").is_err());
    }
}
