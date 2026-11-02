//! Coinbase carriers for `A_send` (SPEC §2.4).
//!
//! Two carriers are defined:
//!
//! 1. **scriptSig** (pool-template mode, §2.4.1):
//!    `<BIP34 height push> OP_PUSHBYTES_32 <ser_x(A_send)> <extranonce push>`
//!
//! 2. **OP_RETURN** (JD mode, §2.4.2):
//!    `OP_RETURN OP_PUSHBYTES_35 0x53 0x50 0x63 <ser_x(A_send)>`
//!
//! Parser precedence (SPEC §2.5): scriptSig wins, then OP_RETURN.
//! Neither parser panics; both return `Option` or `Result`.

use bitcoin::script::ScriptBuf;

use crate::Error;

// OP codes as raw bytes.
const OP_RETURN: u8 = 0x6a;
const OP_PUSHBYTES_32: u8 = 0x20; // pushes exactly 32 bytes
const OP_PUSHBYTES_35: u8 = 0x23; // pushes exactly 35 bytes
const SPC_MAGIC: [u8; 3] = [0x53, 0x50, 0x63]; // "SPc"

// ---------------------------------------------------------------------------
// BIP 34 height push (minimal CScript number)
// ---------------------------------------------------------------------------

/// Encode a block height as the minimal BIP 34 scriptSig number push.
///
/// Heights 1–16 encode as a single opcode (`OP_1`..`OP_16`); 0 is `OP_0`.
/// Heights 17–255 encode as `OP_PUSHBYTES_1 <byte>`.
/// Heights 256–65535 encode as `OP_PUSHBYTES_2 <le16>`.
/// Heights 65536–16777215 encode as `OP_PUSHBYTES_3 <le24>`.
/// Heights >= 16777216 require 4 bytes: `OP_PUSHBYTES_4 <le32>` (or 5 bytes
/// if the high bit of the 4th byte is set, i.e. >= 2^31, but block heights
/// are u32 so the maximum is 4 294 967 295 which needs 5 bytes with the sign
/// byte, but BIP 34 specifies the minimal push so 4 bytes is fine for
/// heights < 2^31; for heights >= 2^31 we add a zero sign byte).
///
/// Returns a `Vec<u8>` including the push opcode and the value bytes.
/// This is the value that goes **directly** in the scriptSig, including the
/// length-prefix opcode.
pub fn height_push_bytes(height: u32) -> Vec<u8> {
    // CScript minimal number encoding (little-endian, sign-magnitude).
    // For a positive number we need enough bytes to hold it without the high
    // bit being set (which would indicate the sign).
    if height == 0 {
        // OP_0 / OP_FALSE
        return vec![0x00];
    }

    // Compute minimal little-endian encoding.
    let mut bytes = height.to_le_bytes().to_vec();
    // Trim trailing zero bytes, but keep at least one.
    while bytes.len() > 1 && *bytes.last().unwrap() == 0 {
        bytes.pop();
    }
    // If the high bit of the last byte is set, we need a zero sign byte to
    // indicate positive (CScript sign-magnitude).
    if bytes.last().unwrap() & 0x80 != 0 {
        bytes.push(0x00);
    }

    let n = bytes.len();
    // For 1 byte: if value 1..=16, use OP_1..OP_16 (0x51..0x60).
    if n == 1 && bytes[0] >= 1 && bytes[0] <= 16 {
        return vec![0x50 + bytes[0]]; // OP_1 = 0x51, ..., OP_16 = 0x60
    }

    // Otherwise: length prefix + bytes.
    // n <= 4 bytes for heights < 2^31; n = 5 for heights that need the sign byte.
    debug_assert!(n <= 5, "height push too long");
    let mut out = Vec::with_capacity(1 + n);
    out.push(n as u8); // OP_PUSHBYTES_n
    out.extend_from_slice(&bytes);
    out
}

/// Compute the length of the BIP 34 height push for a given height.
pub fn height_push_len(height: u32) -> usize {
    height_push_bytes(height).len()
}

// ---------------------------------------------------------------------------
// scriptSig (pool-template carrier, §2.4.1)
// ---------------------------------------------------------------------------

/// Emit the pool-template coinbase scriptSig prefix through the `A_send` push.
///
/// Layout: `<BIP34 height push> OP_PUSHBYTES_32 <a_x[32]>`
///
/// The extranonce bytes are NOT included here; they follow in the
/// extranonce/suffix fields of the SV2 job.
pub fn emit_script_sig(height: u32, a_x: &[u8; 32]) -> Vec<u8> {
    let h_push = height_push_bytes(height);
    let mut out = Vec::with_capacity(h_push.len() + 1 + 32);
    out.extend_from_slice(&h_push);
    out.push(OP_PUSHBYTES_32);
    out.extend_from_slice(a_x);
    out
}

/// Parse `ser_x(A_send)` from a coinbase scriptSig.
///
/// Expects the layout from `emit_script_sig` (height push then `OP_PUSHBYTES_32`
/// followed by 32 bytes). Returns `Ok(Some(a_x))` on success, `Ok(None)` if
/// the script does not match the SPc pattern (not an error — it just isn't an
/// SPc coinbase), or `Err` on a malformed push.
///
/// The function is deliberately permissive about the height push: it skips
/// the BIP 34 height field by reading the first push opcode and its data, then
/// looks for `OP_PUSHBYTES_32`.
pub fn parse_script_sig(script: &[u8]) -> Result<Option<[u8; 32]>, Error> {
    if script.is_empty() {
        return Ok(None);
    }

    // Step 1: skip the BIP34 height push.
    let after_height = skip_bip34_push(script)?;
    if after_height.is_none() {
        return Ok(None);
    }
    let rest = after_height.unwrap();

    // Step 2: must be OP_PUSHBYTES_32 followed by 32 bytes.
    if rest.len() < 1 + 32 {
        return Ok(None);
    }
    if rest[0] != OP_PUSHBYTES_32 {
        return Ok(None);
    }
    let mut a_x = [0u8; 32];
    a_x.copy_from_slice(&rest[1..33]);
    Ok(Some(a_x))
}

/// Skip a BIP34 minimal-push at the start of `script`. Returns the slice
/// after the push, or `None` if the script doesn't start with a recognizable
/// push opcode.
fn skip_bip34_push(script: &[u8]) -> Result<Option<&[u8]>, Error> {
    if script.is_empty() {
        return Ok(None);
    }
    let op = script[0];
    match op {
        // OP_0
        0x00 => Ok(Some(&script[1..])),
        // OP_1..OP_16 (0x51..0x60) — single opcode encodes value 1..16.
        0x51..=0x60 => Ok(Some(&script[1..])),
        // OP_PUSHBYTES_1..OP_PUSHBYTES_4 (0x01..0x04) for heights 1..2^32-1.
        // BIP34 uses up to 4 bytes (heights < 2^31) or 5 bytes (with sign byte).
        0x01..=0x05 => {
            let n = op as usize;
            if script.len() < 1 + n {
                return Err(Error::Carrier("BIP34 height push truncated".to_string()));
            }
            Ok(Some(&script[1 + n..]))
        }
        // OP_1NEGATE (0x4f) — rare but valid CScript -1
        0x4f => Ok(Some(&script[1..])),
        // Anything else is not a recognized height push for SPc purposes.
        _ => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// OP_RETURN (JD carrier, §2.4.2)
// ---------------------------------------------------------------------------

/// Emit the JD-mode `OP_RETURN` output script for `A_send`.
///
/// Layout: `OP_RETURN OP_PUSHBYTES_35 0x53 0x50 0x63 <a_x[32]>`
///
/// Total script: 37 bytes. Output value MUST be 0.
pub fn emit_op_return(a_x: &[u8; 32]) -> ScriptBuf {
    let mut data = Vec::with_capacity(37);
    data.push(OP_RETURN);
    data.push(OP_PUSHBYTES_35);
    data.extend_from_slice(&SPC_MAGIC);
    data.extend_from_slice(a_x);
    ScriptBuf::from_bytes(data)
}

/// A minimal `TxOut` view for `parse_op_return`.
pub struct TxOut<'a> {
    pub value: u64,
    pub script_pubkey: &'a [u8],
}

/// Parse `ser_x(A_send)` from the coinbase outputs, looking for the SPc
/// `OP_RETURN` carrier.
///
/// Scans `vouts` for the first output whose script matches:
/// `OP_RETURN OP_PUSHBYTES_35 0x53 0x50 0x63 <32 bytes>`.
///
/// Returns `Some(a_x)` on the first match; `None` if no matching output exists.
/// Rejects wrong lengths or wrong magic silently (not an error).
pub fn parse_op_return(vouts: &[TxOut<'_>]) -> Option<[u8; 32]> {
    for out in vouts {
        let s = out.script_pubkey;
        if s.len() != 37 {
            continue;
        }
        if s[0] != OP_RETURN {
            continue;
        }
        if s[1] != OP_PUSHBYTES_35 {
            continue;
        }
        if s[2..5] != SPC_MAGIC {
            continue;
        }
        let mut a_x = [0u8; 32];
        a_x.copy_from_slice(&s[5..]);
        return Some(a_x);
    }
    None
}

// ---------------------------------------------------------------------------
// Carrier precedence (SPEC §2.5)
// ---------------------------------------------------------------------------

/// Parse `ser_x(A_send)` with scriptSig precedence, then OP_RETURN fallback.
///
/// - If `script_sig` contains a valid SPc `push32` after the BIP34 height,
///   return it (scriptSig takes precedence per SPEC §2.5).
/// - Otherwise look for the `OP_RETURN` carrier in `vouts`.
/// - Returns `None` if neither is present (block is not SPc).
pub fn parse_carrier(script_sig: &[u8], vouts: &[TxOut<'_>]) -> Option<[u8; 32]> {
    // scriptSig first.
    if let Ok(Some(a_x)) = parse_script_sig(script_sig) {
        return Some(a_x);
    }
    // OP_RETURN fallback.
    parse_op_return(vouts)
}

// ---------------------------------------------------------------------------
// Budget check (SPEC §2.4.1)
// ---------------------------------------------------------------------------

/// Check whether the extranonce fits within the scriptSig budget.
///
/// The budget is 100 bytes (reserved by the Template Provider unconditionally).
///
/// Layout: `height_push (4 or 5 B) + OP_PUSHBYTES_32 + 32 B + extranonce`
///
/// Heights <= 16,777,215 (0xFFFFFF) use a 4-byte push (opcode + 3 data bytes);
/// heights above 16,777,215 use a 5-byte push (opcode + 4 data bytes, or
/// opcode + 4 + sign byte if needed).
///
/// Returns `true` if `100 - height_push_len - 33 - extranonce_len >= 0`.
pub fn script_sig_budget(height: u32, extranonce_len: usize) -> bool {
    let h_len = height_push_len(height);
    // push32 = 1 opcode + 32 data bytes = 33
    let used = h_len.saturating_add(33).saturating_add(extranonce_len);
    used <= 100
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn height_push_small_values() {
        // 0 -> OP_0
        assert_eq!(height_push_bytes(0), vec![0x00]);
        // 1..16 -> OP_1..OP_16
        for v in 1u32..=16 {
            let push = height_push_bytes(v);
            assert_eq!(push.len(), 1, "height {v} should be single opcode");
            assert_eq!(push[0], 0x50 + v as u8);
        }
    }

    #[test]
    fn height_push_17() {
        // 17 = 0x11, no high bit, 1 byte.
        let push = height_push_bytes(17);
        assert_eq!(push, vec![0x01, 0x11]);
    }

    #[test]
    fn height_push_255() {
        // 255 = 0xff; high bit set, needs sign byte.
        let push = height_push_bytes(255);
        assert_eq!(push, vec![0x02, 0xff, 0x00]);
    }

    #[test]
    fn height_push_800000() {
        // 800000 = 0x0c3500 in little-endian = [0x00, 0x35, 0x0c].
        // After trim: [0x00, 0x35, 0x0c].
        // High bit of 0x0c = 0, no sign byte needed.
        let push = height_push_bytes(800_000);
        // le bytes: 0x00, 0x35, 0x0c, 0x00 -> trimmed -> [0x00, 0x35, 0x0c]
        assert_eq!(push, vec![0x03, 0x00, 0x35, 0x0c]);
    }

    #[test]
    fn height_push_16777216() {
        // 16777216 = 0x01_00_00_00.
        // le bytes: [0x00, 0x00, 0x00, 0x01] -> trim trailing zeros -> [0x00, 0x00, 0x00, 0x01]
        // High bit of 0x01 = 0, no sign byte. 4 bytes.
        let push = height_push_bytes(16_777_216);
        assert_eq!(push.len(), 5); // 1 opcode + 4 data bytes
        assert_eq!(push[0], 0x04);
        assert_eq!(&push[1..], &[0x00, 0x00, 0x00, 0x01]);
    }

    #[test]
    fn script_sig_roundtrip() {
        let a_x = [0x42u8; 32];
        let script = emit_script_sig(800_000, &a_x);
        let parsed = parse_script_sig(&script).unwrap().unwrap();
        assert_eq!(parsed, a_x);
    }

    #[test]
    fn script_sig_roundtrip_low_height() {
        let a_x = [0xabu8; 32];
        for h in [0u32, 1, 16, 17, 255, 65535] {
            let script = emit_script_sig(h, &a_x);
            let parsed = parse_script_sig(&script).unwrap().unwrap();
            assert_eq!(parsed, a_x, "height {h}");
        }
    }

    #[test]
    fn script_sig_roundtrip_16777216() {
        let a_x = [0x11u8; 32];
        let script = emit_script_sig(16_777_216, &a_x);
        let parsed = parse_script_sig(&script).unwrap().unwrap();
        assert_eq!(parsed, a_x);
    }

    #[test]
    fn script_sig_wrong_op_returns_none() {
        // Build a script where after the height there's no OP_PUSHBYTES_32.
        let h_push = height_push_bytes(800_000);
        let mut script = h_push;
        script.push(0x76); // OP_DUP instead of push32
        script.extend_from_slice(&[0u8; 32]);
        assert_eq!(parse_script_sig(&script).unwrap(), None);
    }

    #[test]
    fn script_sig_empty_returns_none() {
        assert_eq!(parse_script_sig(&[]).unwrap(), None);
    }

    #[test]
    fn op_return_roundtrip() {
        let a_x = [0x42u8; 32];
        let script = emit_op_return(&a_x);
        let vouts = vec![TxOut {
            value: 0,
            script_pubkey: script.as_bytes(),
        }];
        let parsed = parse_op_return(&vouts).unwrap();
        assert_eq!(parsed, a_x);
    }

    #[test]
    fn op_return_wrong_magic_returns_none() {
        let a_x = [0x42u8; 32];
        let script = emit_op_return(&a_x);
        let mut s = script.as_bytes().to_vec();
        s[3] = 0xff; // corrupt magic
        let vouts = vec![TxOut {
            value: 0,
            script_pubkey: &s,
        }];
        assert_eq!(parse_op_return(&vouts), None);
    }

    #[test]
    fn op_return_wrong_length_returns_none() {
        let s = vec![OP_RETURN, OP_PUSHBYTES_35, 0x53, 0x50, 0x63];
        let vouts = vec![TxOut {
            value: 0,
            script_pubkey: &s,
        }];
        assert_eq!(parse_op_return(&vouts), None);
    }

    #[test]
    fn op_return_empty_vouts_returns_none() {
        assert_eq!(parse_op_return(&[]), None);
    }

    #[test]
    fn carrier_precedence_scriptsig_wins() {
        let a_x_sig = [0x01u8; 32];
        let a_x_ret = [0x02u8; 32];
        let script = emit_script_sig(800_000, &a_x_sig);
        let or_script = emit_op_return(&a_x_ret);
        let vouts = vec![TxOut {
            value: 0,
            script_pubkey: or_script.as_bytes(),
        }];
        let parsed = parse_carrier(&script, &vouts).unwrap();
        assert_eq!(parsed, a_x_sig, "scriptSig must take precedence");
    }

    #[test]
    fn carrier_falls_back_to_op_return() {
        let a_x = [0x42u8; 32];
        let or_script = emit_op_return(&a_x);
        let vouts = vec![TxOut {
            value: 0,
            script_pubkey: or_script.as_bytes(),
        }];
        // Use an empty scriptSig (no SPc data).
        let parsed = parse_carrier(&[], &vouts).unwrap();
        assert_eq!(parsed, a_x);
    }

    #[test]
    fn carrier_none_when_neither_present() {
        assert_eq!(parse_carrier(&[], &[]), None);
    }

    #[test]
    fn budget_800000_fits() {
        // Height 800000 uses 4-byte push; 4 + 33 + 16 = 53 <= 100.
        assert!(script_sig_budget(800_000, 16));
    }

    #[test]
    fn budget_16777216_fits() {
        // Height 16777216 uses 5-byte push; 5 + 33 + 16 = 54 <= 100.
        assert!(script_sig_budget(16_777_216, 16));
    }

    #[test]
    fn budget_too_large() {
        // Height 800000 (4 B) + 33 + 64 = 101 > 100.
        assert!(!script_sig_budget(800_000, 64));
    }

    #[test]
    fn budget_exact_boundary() {
        // Height 800000 (4 B) + 33 + extranonce = exactly 100.
        let extranonce = 100 - 4 - 33;
        assert!(script_sig_budget(800_000, extranonce));
        assert!(!script_sig_budget(800_000, extranonce + 1));
    }
}
