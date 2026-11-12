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

use bitcoin::bip32::{ChildNumber, Xpub};
use bitcoin::script::ScriptBuf;
use bitcoin::NetworkKind;
use spc_crypto::{emit_script_sig, height_push_bytes, script_sig_budget, Network};

/// The scriptSig budget the Template Provider reserves unconditionally
/// (SPEC §2.4.1).
const SCRIPT_SIG_BUDGET: usize = 100;

/// Extended-public-key version bytes (BIP 32 / SLIP 132): mainnet `xpub`
/// and the testnet-family `tpub`, `upub`, `vpub`.
const XPUB_VERSION_MAINNET: [u8; 4] = [0x04, 0x88, 0xB2, 0x1E];
const TPUB_VERSION: [u8; 4] = [0x04, 0x35, 0x87, 0xCF];
const UPUB_VERSION: [u8; 4] = [0x04, 0x4A, 0x52, 0x62];
const VPUB_VERSION: [u8; 4] = [0x04, 0x5F, 0x1C, 0xF6];

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

/// A witness-free coinbase template (the witness field is injected at block
/// assembly, never hashed by miners). In pool-template mode the commitment
/// output - when the transaction set carries witness data - is appended by
/// the caller as the last output; JD mode carries the JDC's own commitment.
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

    /// Assemble the coinbase and inject the BIP 141 witness field
    /// (`01 20 <reserved>` behind the segwit marker/flag). The result
    /// serializes the same *stripped* transaction as [`Self::assemble`]
    /// plus the witness, so the txid - what miners hashed - is unchanged.
    pub fn assemble_with_witness(
        &self,
        extranonce: &[u8],
        reserved: &[u8; 32],
    ) -> Result<Vec<u8>, crate::block::BlockError> {
        crate::block::coinbase_with_witness(&self.assemble(extranonce), reserved)
    }
}

/// The BIP 141 witness commitment output: a zero-value `OP_RETURN` output
/// carrying `commitment_script(commitment)` (decision
/// 2026-11-08-pool-witness-commitment). Callers append it *last*, after
/// every payout output, so the pool output stays index 0 (settlement's
/// `fixed_outputs` takes it) and payout slots keep their order.
pub fn commitment_output(commitment: &[u8; 32]) -> (ScriptBuf, u64) {
    (
        ScriptBuf::from_bytes(crate::witness::commitment_script(commitment)),
        0,
    )
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

/// The parsed `[pool] pool_payout_descriptor` (SPEC §6.3, §11.3): a
/// single-key `tr(<xpub>/.../*)` whose wildcard index is the block height.
///
/// Fail-closed by construction: every ambiguity a descriptor can carry
/// (private key, hardened step after the xpub, multipath, missing or
/// repeated wildcard, wrong network, malformed key) is a parse error, never
/// a fallback. The SPc side holds only this public key material.
#[derive(Clone, Debug)]
pub struct PoolPayoutDescriptor {
    xpub: Xpub,
    /// Unhardened derivation steps between the xpub and the wildcard.
    steps: Vec<ChildNumber>,
    network: Network,
}

impl PoolPayoutDescriptor {
    /// Parse fail-closed per the three assertions of the xpub spec (SPEC
    /// §6.3): single-key `tr()` wrapper, one extended public key
    /// (`xpub`/`tpub`/`upub`/`vpub`) matching `network`, unhardened steps
    /// only, exactly one trailing `*`, no private key, no multipath.
    pub fn parse(desc: &str, network: Network) -> Result<Self, String> {
        let inner = desc
            .strip_prefix("tr(")
            .and_then(|d| d.strip_suffix(')'))
            .ok_or_else(|| "must be a tr(...) descriptor".to_string())?;
        if inner.is_empty() {
            return Err("empty descriptor".into());
        }
        if inner.contains("prv") {
            return Err("must not contain a private key".into());
        }
        if inner.contains('<') || inner.contains(';') {
            return Err("multipath is not supported".into());
        }
        let parts: Vec<&str> = inner.split('/').collect();
        let xpub_str = parts[0];
        if xpub_str.contains('*') {
            return Err("the wildcard must be the last step".into());
        }
        let xpub = parse_xpub(xpub_str, network)?;
        let mut steps = Vec::new();
        for (i, part) in parts.iter().enumerate().skip(1) {
            if *part == "*" {
                // A wildcard is only legal as the final step; the trailing
                // position is re-asserted below.
                if i != parts.len() - 1 {
                    return Err("the wildcard must be the last step".into());
                }
                continue;
            }
            let index = part
                .parse::<u32>()
                .map_err(|_| format!("invalid derivation step '{part}'"))?;
            steps.push(normal_child(index)?);
        }
        // Exactly one unhardened wildcard at the end (the index is the
        // block height, SPEC §11.3).
        if !desc.ends_with("/*)") {
            return Err("exactly one trailing unhardened wildcard is required".into());
        }
        Ok(Self {
            xpub,
            steps,
            network,
        })
    }

    /// The 34-byte P2TR output script for the pool output at `height`
    /// (SPEC §11.3): `pool_output(H) = descriptor.at_derivation_index(H)`,
    /// a fresh key-path taproot output per height.
    pub fn script_at(&self, height: u32) -> Result<ScriptBuf, String> {
        let index = normal_child(height)?;
        let mut path = self.steps.clone();
        path.push(index);
        let secp = bitcoin::secp256k1::Secp256k1::verification_only();
        let child = self
            .xpub
            .derive_pub(&secp, &path)
            .map_err(|e| format!("public derivation failed: {e}"))?;
        // Key-path P2TR (no script tree): tweak the x-only internal key.
        let address = bitcoin::Address::p2tr(
            &secp,
            child.to_x_only_pub(),
            None,
            bitcoin_network(self.network),
        );
        let script = address.script_pubkey();
        if script.len() != 34 {
            return Err("derived script is not a 34-byte P2TR".into());
        }
        Ok(script)
    }
}

/// An unhardened child number, rejecting the hardened half of the space.
fn normal_child(index: u32) -> Result<ChildNumber, String> {
    ChildNumber::from_normal_idx(index).map_err(|_| "hardened indexes are not allowed".to_string())
}

fn bitcoin_network(n: Network) -> bitcoin::Network {
    match n {
        Network::Mainnet => bitcoin::Network::Bitcoin,
        Network::Testnet | Network::Signet => bitcoin::Network::Testnet,
        Network::Regtest => bitcoin::Network::Regtest,
    }
}

/// Base58check-decode an extended public key and bind its version bytes to
/// the configured network: `xpub` is mainnet, `tpub`/`upub`/`vpub` are the
/// testnet family. Anything else fails closed.
fn parse_xpub(s: &str, network: Network) -> Result<Xpub, String> {
    let data = bitcoin::base58::decode_check(s).map_err(|_| "invalid extended key".to_string())?;
    if data.len() != 78 {
        return Err("invalid extended key length".into());
    }
    let version: [u8; 4] = data[0..4]
        .try_into()
        .expect("slice of length 4 fits [u8; 4]");
    let is_mainnet = version == XPUB_VERSION_MAINNET;
    let is_testnet = version == TPUB_VERSION || version == UPUB_VERSION || version == VPUB_VERSION;
    if !is_mainnet && !is_testnet {
        return Err("unknown extended-key version bytes".into());
    }
    let network_ok = match network {
        Network::Mainnet => is_mainnet,
        Network::Testnet | Network::Signet | Network::Regtest => is_testnet,
    };
    if !network_ok {
        return Err("extended key does not match the configured network".into());
    }
    let chain_code: [u8; 32] = data[13..45]
        .try_into()
        .expect("slice of length 32 fits [u8; 32]");
    Ok(Xpub {
        network: if is_mainnet {
            NetworkKind::Main
        } else {
            NetworkKind::Test
        },
        depth: data[4],
        parent_fingerprint: data[5..9]
            .try_into()
            .expect("slice of length 4 fits [u8; 4]"),
        child_number: u32::from_be_bytes(
            data[9..13]
                .try_into()
                .expect("slice of length 4 fits [u8; 4]"),
        )
        .into(),
        chain_code: chain_code.into(),
        public_key: bitcoin::secp256k1::PublicKey::from_slice(&data[45..78])
            .map_err(|_| "invalid public key point".to_string())?,
    })
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
    use bitcoin::bip32::Xpriv;

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
    fn commitment_output_is_a_zero_value_op_return() {
        let (script, value) = commitment_output(&[0x42; 32]);
        assert_eq!(value, 0);
        assert_eq!(script.len(), 38);
        assert_eq!(
            &script.as_bytes()[..6],
            &[0x6a, 0x24, 0xaa, 0x21, 0xa9, 0xed]
        );
        assert_eq!(
            crate::witness::commitment_from_script(script.as_bytes()),
            Some([0x42; 32])
        );
    }

    #[test]
    fn assemble_with_witness_keeps_the_stripped_txid() {
        let a_x = [0x42; 32];
        let outputs = vec![
            (p2tr_script(&[0x11; 32]), 1000),
            commitment_output(&[0x99; 32]),
        ];
        let t = build_pool_template(2, 800_000, &a_x, 8, &outputs, 0);
        let stripped = t.assemble(&[0xaa; 8]);
        let full = t.assemble_with_witness(&[0xaa; 8], &[0x42; 32]).unwrap();
        // Marker/flag plus the 34-byte witness field.
        assert_eq!(full.len(), stripped.len() + 36);
        assert_eq!(&full[4..6], &[0x00, 0x01]);
        // The txid is the hash of the stripped serialization: unchanged.
        // (The raw-byte hash of the full serialization is the wtxid, so
        // compare via the decoded transactions.)
        let full_tx: bitcoin::Transaction = bitcoin::consensus::deserialize(&full).unwrap();
        let stripped_tx: bitcoin::Transaction = bitcoin::consensus::deserialize(&stripped).unwrap();
        assert_eq!(full_tx.compute_txid(), stripped_tx.compute_txid());
        // A second call with a different reserved value still keeps the
        // txid: only the commitment changes.
        let other = t.assemble_with_witness(&[0xaa; 8], &[0x43; 32]).unwrap();
        let other_tx: bitcoin::Transaction = bitcoin::consensus::deserialize(&other).unwrap();
        assert_eq!(other_tx.compute_txid(), stripped_tx.compute_txid());
        assert_ne!(other, full);
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

    /// A real testnet-family xpub (watch-only nonsense, structurally valid),
    /// so the parse path exercises real base58check and point validation.
    fn test_xpub() -> Xpub {
        let secp = bitcoin::secp256k1::Secp256k1::new();
        let xprv = Xpriv::new_master(bitcoin::Network::Testnet, &[0x5c; 32]).expect("master key");
        Xpub::from_priv(&secp, &xprv)
    }

    fn test_descriptor(network: Network) -> PoolPayoutDescriptor {
        PoolPayoutDescriptor::parse(&format!("tr({}/0/*)", test_xpub()), network)
            .expect("valid descriptor")
    }

    /// The watch-only tpub documented in `config/spc-pool.example.toml`
    /// and the regtest harnesses: structurally valid (real base58check
    /// checksum, real point), so it must parse and derive.
    #[test]
    fn the_documented_example_tpub_parses() {
        const DOC_TPUB: &str = "tpubD6NzVbkrYhZ4WUjXBVxZPvGhWxTRE5veAscTgokX65xcfCoDF5nyNdtaS4xGoJ7xnZuDTxCAEAnGpxJbWo6wtTAJsESmeJWpZzybZ6dvCia";
        let d = PoolPayoutDescriptor::parse(&format!("tr({DOC_TPUB}/0/*)"), Network::Regtest)
            .expect("the documented tpub must parse");
        let script = d.script_at(800_000).unwrap();
        assert_eq!(script.len(), 34);
        assert_ne!(&script.as_bytes()[2..], &[0u8; 32]);
    }

    #[test]
    fn descriptor_derives_a_distinct_p2tr_per_height() {
        let d = test_descriptor(Network::Regtest);
        let at = |h: u32| d.script_at(h).unwrap();
        let s0 = at(800_000);
        let s1 = at(800_001);
        let s0_again = at(800_000);
        assert_eq!(s0.len(), 34);
        assert_eq!(&s0.as_bytes()[..2], &[0x51, 0x20]);
        assert_ne!(s0, s1, "two heights must derive different scripts");
        assert_eq!(s0, s0_again, "derivation is deterministic");
        // No zero-key output: the fallback is gone.
        assert_ne!(&s0.as_bytes()[2..], &[0u8; 32]);
    }

    #[test]
    fn descriptor_derivation_matches_the_reference_p2tr_tweak() {
        // Cross-check against bitcoin's own address derivation: the derived
        // child's x-only key, tweaked with no script tree, must produce the
        // same script as `script_at`.
        let d = test_descriptor(Network::Regtest);
        let secp = bitcoin::secp256k1::Secp256k1::verification_only();
        let path = vec![
            ChildNumber::Normal { index: 0 },
            ChildNumber::Normal { index: 800_012 },
        ];
        let child = d.xpub.derive_pub(&secp, &path).unwrap();
        let expected = bitcoin::Address::p2tr(
            &secp,
            child.to_x_only_pub(),
            None,
            bitcoin::Network::Regtest,
        )
        .script_pubkey();
        assert_eq!(d.script_at(800_012).unwrap(), expected);
    }

    #[test]
    fn descriptor_rejects_network_mismatch() {
        // A tpub on mainnet is refused; an xpub on regtest is refused.
        assert!(
            PoolPayoutDescriptor::parse(&format!("tr({}/0/*)", test_xpub()), Network::Mainnet)
                .is_err()
        );
        let secp = bitcoin::secp256k1::Secp256k1::new();
        let xprv = Xpriv::new_master(bitcoin::Network::Bitcoin, &[0x5c; 32]).unwrap();
        let xpub = Xpub::from_priv(&secp, &xprv);
        assert!(PoolPayoutDescriptor::parse(&format!("tr({xpub}/0/*)"), Network::Regtest).is_err());
        assert!(PoolPayoutDescriptor::parse(&format!("tr({xpub}/0/*)"), Network::Mainnet).is_ok());
    }

    #[test]
    fn descriptor_fails_closed_on_every_ambiguity() {
        let xpub = test_xpub().to_string();
        let xprv_like = xpub.replacen("tpub", "tprv", 1);
        for bad in [
            format!("tr({xpub}/0)"),              // no wildcard
            format!("tr({xpub}/0/*/1)"),          // wildcard not last
            format!("tr({xpub}/0/*/*)"),          // two wildcards
            format!("tr({xpub}/0'/ *)"),          // malformed step
            format!("tr({xpub}/0'/*)"),           // hardened step
            format!("tr({xpub}/0h/*)"),           // hardened step (h)
            format!("tr({xpub}/0/2147483648/*)"), // hardened index value
            format!("tr({xpub}/<0;1>/*)"),        // multipath
            format!("wpkh({xpub}/0/*)"),          // wrong wrapper
            format!("tr({xprv_like}/0/*)"),       // private key
            format!("tr({xpub}/0/*) extra"),      // trailing junk
            format!("tr({xpub}/0/*"),             // unbalanced
        ] {
            assert!(
                PoolPayoutDescriptor::parse(&bad, Network::Regtest).is_err(),
                "{bad} should fail closed"
            );
        }
        // Malformed key material.
        assert!(PoolPayoutDescriptor::parse("tr(notakey/0/*)", Network::Regtest).is_err());
        assert!(PoolPayoutDescriptor::parse("tr()", Network::Regtest).is_err());
        // A height in the hardened half of the child space is refused too.
        let d = test_descriptor(Network::Regtest);
        assert!(d.script_at(1 << 31).is_err());
    }

    #[test]
    fn descriptor_accepts_upub_and_vpub_on_the_testnet_family() {
        // SLIP-132 public-key versions: structurally valid testnet-family
        // keys must parse and derive (the network binding is the version
        // bytes, not the prefix characters).
        let secp = bitcoin::secp256k1::Secp256k1::new();
        let xprv = Xpriv::new_master(bitcoin::Network::Testnet, &[0x5c; 32]).unwrap();
        let tpub = Xpub::from_priv(&secp, &xprv);
        for version in [UPUB_VERSION, VPUB_VERSION] {
            let mut raw = tpub.encode().to_vec();
            raw[..4].copy_from_slice(&version);
            let encoded = bitcoin::base58::encode_check(&raw);
            let d = PoolPayoutDescriptor::parse(&format!("tr({encoded}/0/*)"), Network::Regtest)
                .expect("upub/vpub parse on regtest");
            assert!(d.script_at(1).is_ok());
            // ... and are still refused on mainnet.
            assert!(
                PoolPayoutDescriptor::parse(&format!("tr({encoded}/0/*)"), Network::Mainnet)
                    .is_err()
            );
        }
    }
}
