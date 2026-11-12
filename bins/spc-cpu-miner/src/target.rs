use bitcoin::pow::{CompactTarget, Target};
use bitcoin::BlockHash;
use thiserror::Error;

/// Errors from the consensus compact-target rules or a channel target.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum TargetError {
    #[error("zero target")]
    Zero,
    #[error("negative compact target")]
    Negative,
    #[error("compact target overflows 256 bits")]
    Overflow,
    #[error("target must contain exactly 32 bytes, got {0}")]
    InvalidLength(usize),
}

/// Decode Bitcoin's consensus SetCompact representation and reject values that
/// cannot be a real target. The node still owns the pow-limit check.
pub fn decode_compact_target(bits: u32) -> Result<Target, TargetError> {
    let size = bits >> 24;
    let word = bits & 0x007f_ffff;
    if word != 0 && bits & 0x0080_0000 != 0 {
        return Err(TargetError::Negative);
    }
    if word != 0 && (size > 34 || (word > 0xff && size > 33) || (word > 0xffff && size > 32)) {
        return Err(TargetError::Overflow);
    }
    let target = Target::from_compact(CompactTarget::from_consensus(bits));
    if target == Target::ZERO {
        Err(TargetError::Zero)
    } else {
        Ok(target)
    }
}

/// SV2 U256 values are little-endian byte strings on the wire.
pub fn target_from_le_bytes(bytes: &[u8]) -> Result<Target, TargetError> {
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| TargetError::InvalidLength(bytes.len()))?;
    let target = Target::from_le_bytes(bytes);
    if target == Target::ZERO {
        Err(TargetError::Zero)
    } else {
        Ok(target)
    }
}

/// Inclusive Bitcoin proof-of-work comparison, with both values interpreted as
/// little-endian 256-bit integers.
pub fn meets_target(hash: &BlockHash, target: Target) -> bool {
    target.is_met_by(*hash)
}

pub fn target_le_hex(target: Target) -> String {
    hex::encode(target.to_le_bytes())
}

pub fn target_be_hex(target: Target) -> String {
    hex::encode(target.to_be_bytes())
}
