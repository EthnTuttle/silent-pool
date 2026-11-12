//! BIP 141 witness-commitment primitives for pool-template blocks
//! (SPEC §3.3; decision 2026-11-08-pool-witness-commitment).
//!
//! A block whose transaction set carries witness data must commit to the
//! witness merkle root in a coinbase output (BIP 141). This module owns the
//! constants and the pure functions; the front plans the commitment at
//! refresh time (the reserved value is cached per wtxid root, so
//! republicating an unchanged transaction set keeps the same commitment and
//! leaves already-issued shares valid), and `block` injects the witness at
//! assembly time.
//!
//! BIP 141 facts this module relies on: the reserved value is an
//! unconstrained 32-byte value (the protocol places no requirement on it -
//! the pool draws it from the OS CSPRNG once per witness root as its own
//! share-stability choice, not a protocol obligation); a commitment output
//! is an `OP_RETURN` output whose script starts with the 6-byte header and
//! is at least 38 bytes long; and where several outputs match, consensus
//! takes the one with the **highest output index**.
//!
//! `coinbase_commitment` below is the pool's strict parser for coinbases
//! the pool itself constructs and re-verifies: exactly one well-formed
//! 38-byte zero-value commitment output, appended last (pool policy,
//! deliberately stricter than the consensus highest-index rule - a
//! duplicate or malformed commitment-shaped output in our own coinbase is
//! an ambiguity we refuse instead of disambiguating). Parsing arbitrary
//! node-accepted blocks under the lenient consensus rule is a separate
//! concern, crosschecked against a real node in T4b. Everything here is
//! fail-closed: a script that merely starts like a commitment but is not
//! exactly the 38-byte shape is rejected, never guessed at.

use bitcoin::hashes::{sha256d, Hash};
use thiserror::Error;

/// The 6-byte prologue of a witness commitment output script:
/// `OP_RETURN OP_PUSHBYTES_36 0xaa 0x21 0xa9 0xed` (BIP 141).
pub const WITNESS_COMMITMENT_HEADER: [u8; 6] = [0x6a, 0x24, 0xaa, 0x21, 0xa9, 0xed];

/// Length of a full witness commitment script: 6-byte header + 32 bytes.
pub const COMMITMENT_SCRIPT_LEN: usize = 38;

/// Serialized size of the commitment output, in vbytes: 8 (value) + 1
/// (script length prefix) + 38 (script).
pub const COMMITMENT_OUTPUT_VBYTES: u64 = 47;

/// Weight of the commitment output: it sits in the stripped (non-witness)
/// serialization, so it costs 4 WU per vbyte.
pub const COMMITMENT_OUTPUT_WEIGHT_WU: u64 = COMMITMENT_OUTPUT_VBYTES * 4;

/// Weight of the coinbase's segwit overhead once the witness is injected:
/// 2 (marker + flag) + 1 (item count) + 1 (item length) + 32 (reserved
/// value). Witness bytes count 1 WU each under BIP 141.
pub const COINBASE_WITNESS_WEIGHT_WU: u64 = 36;

/// Total weight the witness commitment removes from the payout budget:
/// the commitment output plus the coinbase witness field. Passed to
/// `route_tiers` as extra `overhead_wu` whenever a commitment is present
/// (SPEC §4.8).
///
/// Boundary notes (Fable rework 2026-11-08, R8): this constant does **not**
/// cover the coinbase output-count CompactSize growing from one byte
/// (<= 252 outputs) to three (`0xfd` + 2 bytes, >= 253 outputs) when the
/// commitment output crosses that boundary - an extra +2 vbytes = +8 WU.
/// The whole-block weight model is the preexisting tier-budget
/// approximation: the ladder governs pool-controlled payout outputs, and
/// the +8 WU at the 252/253 boundary stays a documented model limit rather
/// than a redesigned hard limit. Settlement mirrors the same 224 WU
/// (`payout::tier_config`) so the budget crosscheck agrees at the boundary.
pub const WITNESS_COMMITMENT_OVERHEAD_WU: u32 =
    (COMMITMENT_OUTPUT_WEIGHT_WU + COINBASE_WITNESS_WEIGHT_WU) as u32;

/// A coinbase output set failed the witness-commitment scan.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum WitnessError {
    /// More than one output starts with the commitment header. Consensus
    /// would disambiguate by taking the highest output index (BIP 141);
    /// pool policy for our own coinbases is stricter - a duplicate is an
    /// ambiguity we refuse rather than disambiguate.
    #[error("duplicate witness commitment output at index {index}")]
    DuplicateCommitment { index: usize },
    /// An output starts with the commitment header but is not the exact
    /// 38-byte script: ambiguous, so it is rejected rather than guessed at.
    #[error("malformed witness commitment output at index {index}")]
    MalformedCommitment { index: usize },
}

/// The witness commitment output script for `commitment`:
/// `6a24aa21a9ed` ‖ 32 bytes (BIP 141).
pub fn commitment_script(commitment: &[u8; 32]) -> Vec<u8> {
    let mut script = Vec::with_capacity(COMMITMENT_SCRIPT_LEN);
    script.extend_from_slice(&WITNESS_COMMITMENT_HEADER);
    script.extend_from_slice(commitment);
    script
}

/// Parse a witness commitment script. `Some` only for the exact 38-byte
/// shape; anything else - including a script that starts like a commitment
/// but has the wrong length - is `None` (fail closed).
pub fn commitment_from_script(script: &[u8]) -> Option<[u8; 32]> {
    if script.len() != COMMITMENT_SCRIPT_LEN || !script.starts_with(&WITNESS_COMMITMENT_HEADER) {
        return None;
    }
    Some(script[6..].try_into().expect("checked length"))
}

/// Whether the script merely *starts* like a witness commitment output
/// (the 6-byte header), regardless of length. Used to reject ambiguous
/// commitment-shaped scripts and to keep a pool-template commitment out
/// of JD-mode detection: the JD carrier is
/// `OP_RETURN OP_PUSHBYTES_35 "SPc" <ser_x(A_send)>` (`6a23535063…`,
/// `spc_crypto::carriers::emit_op_return`), whose `6a23` push can never
/// collide with the commitment's `6a24` push.
pub fn looks_like_commitment(script: &[u8]) -> bool {
    script.starts_with(&WITNESS_COMMITMENT_HEADER)
}

/// The BIP 141 witness merkle root. The coinbase's leaf position holds the
/// zero hash (the coinbase carries no witness commitment of its own), so
/// the leaves are `[0u8; 32] ++ wtxids` folded with the same
/// Bitcoin-style last-node duplication rule as the txid merkle tree.
pub fn witness_root(wtxids: &[[u8; 32]]) -> [u8; 32] {
    let mut leaves = Vec::with_capacity(wtxids.len() + 1);
    leaves.push([0u8; 32]);
    leaves.extend_from_slice(wtxids);
    crate::block::merkle_root(&leaves)
}

/// The BIP 141 witness commitment: `SHA256d(witness_root ‖ reserved)`.
pub fn witness_commitment(root: &[u8; 32], reserved: &[u8; 32]) -> [u8; 32] {
    let mut buf = [0u8; 64];
    buf[..32].copy_from_slice(root);
    buf[32..].copy_from_slice(reserved);
    sha256d::Hash::hash(&buf).to_byte_array()
}

/// Scan coinbase output scripts for the witness commitment.
///
/// - `Ok(None)`: no output starts like a commitment;
/// - `Ok(Some((index, bytes)))`: exactly one well-formed commitment;
/// - `Err`: a duplicate or a commitment-shaped script with the wrong
///   length - both make the block ambiguous or invalid, so both fail
///   closed.
pub fn coinbase_commitment<'a>(
    scripts: impl Iterator<Item = &'a [u8]>,
) -> Result<Option<(usize, [u8; 32])>, WitnessError> {
    let mut found: Option<(usize, [u8; 32])> = None;
    for (index, script) in scripts.enumerate() {
        if !looks_like_commitment(script) {
            continue;
        }
        let commitment =
            commitment_from_script(script).ok_or(WitnessError::MalformedCommitment { index })?;
        if found.is_some() {
            return Err(WitnessError::DuplicateCommitment { index });
        }
        found = Some((index, commitment));
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commitment_script_roundtrips_exactly() {
        let commitment = [0x42u8; 32];
        let script = commitment_script(&commitment);
        assert_eq!(script.len(), COMMITMENT_SCRIPT_LEN);
        assert_eq!(&script[..6], &WITNESS_COMMITMENT_HEADER);
        assert_eq!(commitment_from_script(&script), Some(commitment));
    }

    #[test]
    fn commitment_parsing_fails_closed_on_every_shape() {
        let script = commitment_script(&[0x42; 32]);
        // Truncated and extended.
        assert_eq!(commitment_from_script(&script[..37]), None);
        assert_eq!(
            commitment_from_script(&{
                let mut s = script.clone();
                s.push(0x00);
                s
            }),
            None
        );
        // Right length, wrong content: the header is where it must be, so
        // only a mutated header/length pair reaches this.
        let mut wrong = script.clone();
        wrong[2] = 0x20; // OP_PUSHBYTES_32 instead of 36
        assert_eq!(commitment_from_script(&wrong), None);
        // An OP_RETURN carrier (`6a20…`) is not a commitment.
        assert!(!looks_like_commitment(&{
            let mut carrier = vec![0x6a, 0x20];
            carrier.extend_from_slice(&[0u8; 32]);
            carrier
        }));
    }

    #[test]
    fn commitment_scan_rejects_duplicates_and_malformed_shapes() {
        let good = commitment_script(&[1u8; 32]);
        let other = commitment_script(&[2u8; 32]);
        let pool = [0x51, 0x20]
            .iter()
            .copied()
            .chain([0u8; 32])
            .collect::<Vec<u8>>();

        // No commitment at all.
        assert_eq!(coinbase_commitment([pool.as_slice()].into_iter()), Ok(None));
        // Exactly one.
        assert_eq!(
            coinbase_commitment([pool.as_slice(), good.as_slice()].into_iter()),
            Ok(Some((1, [1u8; 32])))
        );
        // Two commitment outputs: consensus takes the highest index, but
        // the pool's own-coinbase policy is stricter and rejects the
        // ambiguity outright.
        assert_eq!(
            coinbase_commitment([good.as_slice(), other.as_slice()].into_iter()),
            Err(WitnessError::DuplicateCommitment { index: 1 })
        );
        // Header with the wrong length: ambiguous, not a commitment.
        let mut malformed = WITNESS_COMMITMENT_HEADER.to_vec();
        malformed.extend_from_slice(&[0u8; 31]);
        assert_eq!(
            coinbase_commitment([malformed.as_slice()].into_iter()),
            Err(WitnessError::MalformedCommitment { index: 0 })
        );
    }

    #[test]
    fn witness_root_folds_the_zero_coinbase_leaf() {
        use bitcoin::hashes::{sha256d, Hash};
        let hash = |b: &[u8]| sha256d::Hash::hash(b).to_byte_array();
        let pair = |a: &[u8; 32], b: &[u8; 32]| {
            let mut buf = [0u8; 64];
            buf[..32].copy_from_slice(a);
            buf[32..].copy_from_slice(b);
            hash(&buf)
        };
        // No non-coinbase tx: the single zero leaf is the root.
        assert_eq!(witness_root(&[]), [0u8; 32]);
        // One wtxid: H(0 ‖ w).
        let w = [7u8; 32];
        assert_eq!(witness_root(&[w]), pair(&[0u8; 32], &w));
        // Two wtxids: H(H(0 ‖ a) ‖ H(b ‖ b)) - the odd level duplicates b.
        let a = [1u8; 32];
        let b = [2u8; 32];
        assert_eq!(
            witness_root(&[a, b]),
            pair(&pair(&[0u8; 32], &a), &pair(&b, &b))
        );
    }

    #[test]
    fn weight_constants_match_the_serialized_shapes() {
        // Output: value(8) + script length(1) + script(38) = 47 vbytes.
        assert_eq!(COMMITMENT_OUTPUT_VBYTES, 47);
        assert_eq!(COMMITMENT_OUTPUT_WEIGHT_WU, 188);
        // Witness field: marker(1) + flag(1) + count(1) + length(1) +
        // reserved(32) = 36 bytes.
        assert_eq!(COINBASE_WITNESS_WEIGHT_WU, 36);
        assert_eq!(WITNESS_COMMITMENT_OVERHEAD_WU, 224);
    }
}
