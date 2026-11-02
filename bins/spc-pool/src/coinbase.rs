//! Coinbase construction for pool-template mode (SPEC §2.4.1, §3.3) and
//! job-declaration mode (SPEC §2.4.2).
//!
//! Both builders emit the SV2 `coinbase_tx_prefix` / `coinbase_tx_suffix`
//! split: the prefix ends where the miner's extranonce begins, and the suffix
//! starts with the input sequence. SV1 uses the same split as `coinb1` /
//! `coinb2`.
//!
//! The two modes differ only in the scriptSig carrier:
//!
//! - pool-template: `<BIP34 height push> OP_PUSHBYTES_32 <ser_x(A_send)>`
//!   followed by the extranonce (§2.4.1);
//! - JD: `<BIP34 height push>` followed by the extranonce — no scriptSig
//!   carrier. `A_send` travels in the `OP_RETURN` output the caller supplies
//!   in `outputs` (§2.4.2).

use bitcoin::script::ScriptBuf;
use spc_crypto::{emit_script_sig, height_push_bytes, script_sig_budget};

/// The scriptSig budget the Template Provider reserves unconditionally
/// (SPEC §2.4.1).
const SCRIPT_SIG_BUDGET: usize = 100;

/// A coinbase could not be built within the scriptSig budget.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CoinbaseError {
    /// The height push (+ carrier, in pool-template mode) + extranonce would
    /// exceed the 100 bytes the Template Provider reserves for the scriptSig.
    #[error(
        "scriptSig budget exceeded: {used} bytes at height {height} with a {extranonce_len}-byte extranonce (max {SCRIPT_SIG_BUDGET})"
    )]
    ScriptSigBudget {
        height: u32,
        extranonce_len: usize,
        used: usize,
    },
}

/// A witness-free coinbase template. The witness commitment is the template
/// provider's responsibility in pool-template mode; JD mode carries the JDC's
/// own commitment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoinbaseTemplate {
    pub prefix: Vec<u8>,
    pub suffix: Vec<u8>,
    /// Height push (pool-template mode appends `push32(A_send)`); the
    /// extranonce follows it.
    pub script_sig_prefix: Vec<u8>,
    pub extranonce_len: usize,
}

impl CoinbaseTemplate {
    /// Assemble a complete coinbase with the miner-supplied extranonce.
    pub fn assemble(&self, extranonce: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.prefix.len() + extranonce.len() + self.suffix.len());
        out.extend_from_slice(&self.prefix);
        out.extend_from_slice(extranonce);
        out.extend_from_slice(&self.suffix);
        out
    }
}

/// Build a pool-template coinbase, enforcing the scriptSig budget.
///
/// This is the infallible legacy entry point: the extranonce length is
/// validated at config/subscribe time, so a budget failure here is an
/// internal invariant break, not attacker input. New call sites (and
/// per-connection builds) should call [`try_build_pool_template`], which
/// returns the error instead of aborting.
///
/// `outputs` MUST already be in payout order: the pool output first, miner
/// outputs sorted by `sp_id`, and (in JD mode only) the carrier last. This
/// function does not inject `OP_RETURN`: pool-template mode must not emit it
/// (SPEC §2.4.2).
pub fn build_pool_template(
    version: u32,
    height: u32,
    a_x: &[u8; 32],
    extranonce_len: usize,
    outputs: &[(ScriptBuf, u64)],
    locktime: u32,
) -> CoinbaseTemplate {
    try_build_pool_template(version, height, a_x, extranonce_len, outputs, locktime)
        .expect("scriptSig budget exceeded (SPEC 2.4.1)")
}

/// Build a pool-template coinbase, failing if the scriptSig would exceed the
/// 100-byte budget reserved by the Template Provider (SPEC §2.4.1):
/// `height push + push32(A_send) + extranonce <= 100`.
pub fn try_build_pool_template(
    version: u32,
    height: u32,
    a_x: &[u8; 32],
    extranonce_len: usize,
    outputs: &[(ScriptBuf, u64)],
    locktime: u32,
) -> Result<CoinbaseTemplate, CoinbaseError> {
    if !script_sig_budget(height, extranonce_len) {
        return Err(CoinbaseError::ScriptSigBudget {
            height,
            extranonce_len,
            used: height_push_bytes(height).len() + 33 + extranonce_len,
        });
    }
    let script_sig_prefix = emit_script_sig(height, a_x);
    let prefix = coinbase_prefix(version, &script_sig_prefix, extranonce_len);
    let suffix = output_suffix(outputs, locktime);
    Ok(CoinbaseTemplate {
        prefix,
        suffix,
        script_sig_prefix,
        extranonce_len,
    })
}

/// Build a JD-mode coinbase (SPEC §2.4.2).
///
/// The scriptSig carries only the BIP34 height push followed by the miner's
/// extranonce — there is no `push32(A_send)` carrier, so a JD coinbase has
/// the `OP_RETURN` output as its single carrier. That output is supplied by
/// the caller in `outputs` (typically last). Fails if the height push +
/// extranonce would exceed the 100-byte scriptSig budget.
pub fn build_jd_template(
    version: u32,
    height: u32,
    extranonce_len: usize,
    outputs: &[(ScriptBuf, u64)],
    locktime: u32,
) -> Result<CoinbaseTemplate, CoinbaseError> {
    let h_push = height_push_bytes(height);
    let used = h_push.len() + extranonce_len;
    if used > SCRIPT_SIG_BUDGET {
        return Err(CoinbaseError::ScriptSigBudget {
            height,
            extranonce_len,
            used,
        });
    }
    let prefix = coinbase_prefix(version, &h_push, extranonce_len);
    let suffix = output_suffix(outputs, locktime);
    Ok(CoinbaseTemplate {
        prefix,
        suffix,
        script_sig_prefix: h_push,
        extranonce_len,
    })
}

/// Transaction prefix through the end of the scriptSig carrier; the miner's
/// extranonce is appended between prefix and suffix.
fn coinbase_prefix(version: u32, script_sig_prefix: &[u8], extranonce_len: usize) -> Vec<u8> {
    let mut prefix = Vec::with_capacity(64 + script_sig_prefix.len());
    prefix.extend_from_slice(&version.to_le_bytes());
    prefix.push(1); // one input
    prefix.extend_from_slice(&[0u8; 32]); // null prevout
    prefix.extend_from_slice(&u32::MAX.to_le_bytes());
    write_compact_size(
        &mut prefix,
        (script_sig_prefix.len() + extranonce_len) as u64,
    );
    prefix.extend_from_slice(script_sig_prefix);
    prefix
}

/// Transaction suffix from the input sequence through the locktime.
fn output_suffix(outputs: &[(ScriptBuf, u64)], locktime: u32) -> Vec<u8> {
    let mut suffix = Vec::new();
    suffix.extend_from_slice(&u32::MAX.to_le_bytes()); // sequence
    write_compact_size(&mut suffix, outputs.len() as u64);
    for (script, value) in outputs {
        suffix.extend_from_slice(&value.to_le_bytes());
        write_compact_size(&mut suffix, script.len() as u64);
        suffix.extend_from_slice(script.as_bytes());
    }
    suffix.extend_from_slice(&locktime.to_le_bytes());
    suffix
}

/// P2TR output script: `OP_1 OP_PUSHBYTES_32 <x>`.
pub fn p2tr_script(x: &[u8; 32]) -> ScriptBuf {
    let mut bytes = Vec::with_capacity(34);
    bytes.push(0x51);
    bytes.push(0x20);
    bytes.extend_from_slice(x);
    ScriptBuf::from_bytes(bytes)
}

/// Bitcoin compact size.
pub fn write_compact_size(out: &mut Vec<u8>, n: u64) {
    match n {
        0..=0xfc => out.push(n as u8),
        0xfd..=0xffff => {
            out.push(0xfd);
            out.extend_from_slice(&(n as u16).to_le_bytes());
        }
        0x1_0000..=0xffff_ffff => {
            out.push(0xfe);
            out.extend_from_slice(&(n as u32).to_le_bytes());
        }
        _ => {
            out.push(0xff);
            out.extend_from_slice(&n.to_le_bytes());
        }
    }
}

/// Read a compact size, returning the value and the new offset.
pub fn read_compact_size(buf: &[u8], offset: usize) -> Option<(u64, usize)> {
    let first = *buf.get(offset)?;
    match first {
        0..=0xfc => Some((first as u64, offset + 1)),
        0xfd => {
            let b = buf.get(offset + 1..offset + 3)?;
            Some((u16::from_le_bytes([b[0], b[1]]) as u64, offset + 3))
        }
        0xfe => {
            let b = buf.get(offset + 1..offset + 5)?;
            Some((
                u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as u64,
                offset + 5,
            ))
        }
        0xff => {
            let b = buf.get(offset + 1..offset + 9)?;
            Some((u64::from_le_bytes(b.try_into().ok()?), offset + 9))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_size_boundaries() {
        for (n, expected) in [
            (0u64, vec![0x00]),
            (0xfc, vec![0xfc]),
            (0xfd, vec![0xfd, 0xfd, 0x00]),
            (0xffff, vec![0xfd, 0xff, 0xff]),
            (0x1_0000, vec![0xfe, 0x00, 0x00, 0x01, 0x00]),
        ] {
            let mut out = Vec::new();
            write_compact_size(&mut out, n);
            assert_eq!(out, expected);
            assert_eq!(read_compact_size(&out, 0), Some((n, out.len())));
        }
    }

    #[test]
    fn prefix_has_the_spc_carrier_and_the_right_script_len() {
        let a_x = [0x42; 32];
        let outputs = vec![(p2tr_script(&[0x11; 32]), 1000)];
        let t = build_pool_template(2, 800_000, &a_x, 8, &outputs, 0);

        // version, input count
        assert_eq!(&t.prefix[..5], &[0x02, 0x00, 0x00, 0x00, 0x01]);
        // null prevout + index
        assert_eq!(&t.prefix[5..37], &[0u8; 32]);
        assert_eq!(&t.prefix[37..41], &u32::MAX.to_le_bytes());
        // scriptSig length = height push(4) + push32(33) + extranonce(8) = 45
        assert_eq!(t.prefix[41], 45);
        // height push 800000 = 03 00 35 0c, then 0x20 + 32 bytes
        assert_eq!(&t.prefix[42..46], &[0x03, 0x00, 0x35, 0x0c]);
        assert_eq!(t.prefix[46], 0x20);
        assert_eq!(&t.prefix[47..], &a_x);

        // suffix: sequence, 1 output, value, script len 34, script, locktime
        assert_eq!(&t.suffix[..4], &u32::MAX.to_le_bytes());
        assert_eq!(t.suffix[4], 1);
        assert_eq!(&t.suffix[5..13], &1000u64.to_le_bytes());
        assert_eq!(t.suffix[13], 34);
        assert_eq!(t.suffix[14], 0x51);
        assert_eq!(t.suffix[15], 0x20);

        let full = t.assemble(&[0xaa; 8]);
        assert_eq!(full.len(), t.prefix.len() + 8 + t.suffix.len());
    }

    #[test]
    fn assemble_places_extranonce_in_the_script_sig() {
        let t = build_pool_template(1, 100, &[1; 32], 4, &[], 0);
        let full = t.assemble(&[0xde, 0xad, 0xbe, 0xef]);
        let script_len = t.script_sig_prefix.len() + 4;
        // The four extranonce bytes sit immediately after the carrier.
        let start = t.prefix.len();
        assert_eq!(&full[start..start + 4], &[0xde, 0xad, 0xbe, 0xef]);
        assert_eq!(t.prefix[41], script_len as u8);
    }

    #[test]
    fn p2tr_script_is_34_bytes() {
        let s = p2tr_script(&[9; 32]);
        assert_eq!(s.len(), 34);
        assert_eq!(&s.as_bytes()[..2], &[0x51, 0x20]);
    }

    #[test]
    fn pool_template_budget_is_enforced() {
        let a_x = [0x42; 32];
        let outputs = vec![(p2tr_script(&[0x11; 32]), 1000)];
        // Height 800000 uses a 4-byte push: 4 + 33 + 63 = exactly 100.
        assert!(try_build_pool_template(2, 800_000, &a_x, 63, &outputs, 0).is_ok());
        // One byte over is rejected.
        assert_eq!(
            try_build_pool_template(2, 800_000, &a_x, 64, &outputs, 0),
            Err(CoinbaseError::ScriptSigBudget {
                height: 800_000,
                extranonce_len: 64,
                used: 4 + 33 + 64,
            })
        );
    }

    #[test]
    #[should_panic(expected = "scriptSig budget exceeded")]
    fn pool_template_legacy_entry_panics_on_budget_excess() {
        let outputs = vec![(p2tr_script(&[0x11; 32]), 1000)];
        let _ = build_pool_template(2, 800_000, &[0x42; 32], 64, &outputs, 0);
    }

    #[test]
    fn jd_template_carries_no_script_sig_carrier() {
        let a_x = [0x42; 32];
        let outputs = vec![
            (p2tr_script(&[0x11; 32]), 1000),
            (spc_crypto::emit_op_return(&a_x), 0),
        ];
        let t = build_jd_template(2, 800_000, 8, &outputs, 0).unwrap();

        // scriptSig length = height push (4) + extranonce (8) = 12.
        assert_eq!(t.prefix[41], 12);
        // The scriptSig is the height push only; the prefix ends right after
        // it, so there is no push32(A_send).
        assert_eq!(&t.prefix[42..], &[0x03, 0x00, 0x35, 0x0c]);
        assert_eq!(t.script_sig_prefix, vec![0x03, 0x00, 0x35, 0x0c]);
        // A_send appears only in the OP_RETURN output (suffix), never in the
        // scriptSig (prefix).
        assert!(!t.prefix.windows(32).any(|w| w == a_x));
        assert!(t
            .suffix
            .windows(37)
            .any(|w| w == spc_crypto::emit_op_return(&a_x).as_bytes()));

        // The assembled scriptSig does not parse as an SPc scriptSig carrier.
        let mut script_sig = t.prefix[42..].to_vec();
        script_sig.extend_from_slice(&[0xaa; 8]);
        assert_eq!(spc_crypto::parse_script_sig(&script_sig).unwrap(), None);

        let full = t.assemble(&[0xaa; 8]);
        assert_eq!(full.len(), t.prefix.len() + 8 + t.suffix.len());
    }

    #[test]
    fn jd_template_budget_is_enforced() {
        // Height 16777216 uses a 5-byte push: 5 + 95 = exactly 100.
        assert!(build_jd_template(2, 16_777_216, 95, &[], 0).is_ok());
        assert_eq!(
            build_jd_template(2, 16_777_216, 96, &[], 0),
            Err(CoinbaseError::ScriptSigBudget {
                height: 16_777_216,
                extranonce_len: 96,
                used: 5 + 96,
            })
        );
    }
}
