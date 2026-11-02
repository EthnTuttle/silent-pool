//! Coinbase construction for pool-template mode (SPEC §2.4.1, §3.3).
//!
//! The builder emits the SV2 `coinbase_tx_prefix` / `coinbase_tx_suffix`
//! split: the prefix ends immediately after `push32(A_send)`, the miner's
//! extranonce sits between the two, and the suffix starts with the input
//! sequence. SV1 uses the same split as `coinb1` / `coinb2`.

use bitcoin::script::ScriptBuf;
use spc_crypto::emit_script_sig;

/// A witness-free coinbase template. The witness commitment is the template
/// provider's responsibility in pool-template mode; JD mode carries the JDC's
/// own commitment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoinbaseTemplate {
    pub prefix: Vec<u8>,
    pub suffix: Vec<u8>,
    /// Height push + `push32(A_send)`; the extranonce follows it.
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

/// Build a pool-template coinbase.
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
    let script_sig_prefix = emit_script_sig(height, a_x);
    let script_sig_len = script_sig_prefix.len() + extranonce_len;

    let mut prefix = Vec::with_capacity(64 + script_sig_prefix.len());
    prefix.extend_from_slice(&version.to_le_bytes());
    prefix.push(1); // one input
    prefix.extend_from_slice(&[0u8; 32]); // null prevout
    prefix.extend_from_slice(&u32::MAX.to_le_bytes());
    write_compact_size(&mut prefix, script_sig_len as u64);
    prefix.extend_from_slice(&script_sig_prefix);

    let mut suffix = Vec::new();
    suffix.extend_from_slice(&u32::MAX.to_le_bytes()); // sequence
    write_compact_size(&mut suffix, outputs.len() as u64);
    for (script, value) in outputs {
        suffix.extend_from_slice(&value.to_le_bytes());
        write_compact_size(&mut suffix, script.len() as u64);
        suffix.extend_from_slice(script.as_bytes());
    }
    suffix.extend_from_slice(&locktime.to_le_bytes());

    CoinbaseTemplate {
        prefix,
        suffix,
        script_sig_prefix,
        extranonce_len,
    }
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
}
