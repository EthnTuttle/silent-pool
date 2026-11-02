//! SPc carrier parsing and scanning (SPEC §2.4-§2.5).
//!
//! The scanner is stateless: it takes a block (or a light proof) and a scan
//! key, derives the receiver's output for `H` and `H-1`, and returns what it
//! found. It never talks to the pool.

#![deny(unsafe_code)]

use bitcoin::block::Header;
use bitcoin::consensus::{deserialize, serialize};
use bitcoin::hashes::Hash;
use bitcoin::secp256k1::{PublicKey, Scalar, SecretKey};
use bitcoin::{Block, Transaction, Txid};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use spc_crypto::{input_hash, tier_tweaks, SharedSecret};

pub mod audit;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Error)]
pub enum Error {
    #[error("not a coinbase transaction")]
    NotCoinbase,
    #[error("block has no transactions")]
    EmptyBlock,
    #[error("bad light proof")]
    Proof,
    #[error("derivation failed")]
    Derivation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Found {
    pub height: u32,
    pub vout: u32,
    pub amount: u64,
    /// `ser_x(P)`.
    pub output_key: [u8; 32],
    /// The tier tweak as a 32-byte big-endian scalar.
    pub tweak: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanOutcome {
    pub is_spc: bool,
    pub a_x: Option<[u8; 32]>,
    pub found: Vec<Found>,
}

/// Parse the BIP 34 height from the coinbase scriptSig.
pub fn parse_bip34_height(script_sig: &[u8]) -> Option<u32> {
    let (value, _) = read_first_push(script_sig)?;
    decode_script_num(value)
}

fn read_first_push(script: &[u8]) -> Option<(&[u8], usize)> {
    let op = *script.first()?;
    match op {
        0x00 => Some((&[], 1)),
        0x51..=0x60 => Some((&script[..1], 1)), // handled below
        0x01..=0x4b => {
            let n = op as usize;
            let data = script.get(1..1 + n)?;
            Some((data, 1 + n))
        }
        _ => None,
    }
}

fn decode_script_num(bytes: &[u8]) -> Option<u32> {
    if bytes.len() == 1 && (0x51..=0x60).contains(&bytes[0]) {
        return Some((bytes[0] - 0x50) as u32);
    }
    if bytes.is_empty() {
        return Some(0);
    }
    if bytes.len() > 4 {
        return None;
    }
    let mut v: i64 = 0;
    for (i, b) in bytes.iter().enumerate() {
        v |= (*b as i64) << (8 * i);
    }
    if bytes.last().is_some_and(|b| b & 0x80 != 0) {
        return None; // negative height
    }
    u32::try_from(v).ok()
}

/// Parse `A_send` from a coinbase, scriptSig first then OP_RETURN.
pub fn carrier_from_tx(tx: &Transaction) -> Option<[u8; 32]> {
    if !tx.is_coinbase() {
        return None;
    }
    let script_sig = tx.input.first()?.script_sig.as_bytes();
    let vouts: Vec<spc_crypto::carriers::TxOut<'_>> = tx
        .output
        .iter()
        .map(|o| spc_crypto::carriers::TxOut {
            value: o.value.to_sat(),
            script_pubkey: o.script_pubkey.as_bytes(),
        })
        .collect();
    spc_crypto::parse_carrier(script_sig, &vouts)
}

/// Scan one coinbase for the miner's output at `H` and `H-1`.
pub fn scan_coinbase(
    height: u32,
    a_x: &[u8; 32],
    tx: &Transaction,
    b_scan: &SecretKey,
    b_spend: &PublicKey,
) -> Result<Vec<Found>, Error> {
    if !tx.is_coinbase() {
        return Err(Error::NotCoinbase);
    }
    let mut a_bytes = [0u8; 33];
    a_bytes[0] = 0x02;
    a_bytes[1..].copy_from_slice(a_x);
    let a_send = PublicKey::from_slice(&a_bytes).map_err(|_| Error::Derivation)?;

    let mut out = Vec::new();
    for h in [height, height.saturating_sub(1)] {
        if h == 0 {
            continue;
        }
        let ecdh = SharedSecret::miner_side(h, b_scan, &a_send).map_err(|_| Error::Derivation)?;
        let tweaks = tier_tweaks(&ecdh, 0).map_err(|_| Error::Derivation)?;
        let p = spc_crypto::spc_output_pubkey(b_spend, &tweaks.t_onchain)
            .map_err(|_| Error::Derivation)?;
        let p_x = p.x_only_public_key().0.serialize();
        for (i, o) in tx.output.iter().enumerate() {
            let s = o.script_pubkey.as_bytes();
            if s.len() == 34 && s[0] == 0x51 && s[1] == 0x20 && s[2..] == p_x {
                out.push(Found {
                    height: h,
                    vout: i as u32,
                    amount: o.value.to_sat(),
                    output_key: p_x,
                    tweak: tweaks.t_onchain.to_be_bytes(),
                });
            }
        }
    }
    Ok(out)
}

/// Scan a full block.
pub fn scan_block(
    block: &Block,
    b_scan: &SecretKey,
    b_spend: &PublicKey,
) -> Result<ScanOutcome, Error> {
    let coinbase = block.txdata.first().ok_or(Error::EmptyBlock)?;
    let a_x = carrier_from_tx(coinbase);
    let Some(a_x) = a_x else {
        return Ok(ScanOutcome {
            is_spc: false,
            a_x: None,
            found: vec![],
        });
    };
    let height = parse_bip34_height(coinbase.input[0].script_sig.as_bytes()).ok_or(Error::Proof)?;
    let found = scan_coinbase(height, &a_x, coinbase, b_scan, b_spend)?;
    Ok(ScanOutcome {
        is_spc: true,
        a_x: Some(a_x),
        found,
    })
}

/// A header plus the coinbase and its merkle branch (~500 bytes/block).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LightProof {
    pub header: Header,
    pub branch: Vec<Txid>,
    pub coinbase: Transaction,
}

impl LightProof {
    pub fn encode(&self) -> Vec<u8> {
        let header = serialize(&self.header);
        let mut out = Vec::with_capacity(80 + 1 + self.branch.len() * 32 + 4);
        out.extend_from_slice(&header);
        out.push(self.branch.len() as u8);
        for b in &self.branch {
            out.extend_from_slice(b.as_ref());
        }
        let cb = serialize(&self.coinbase);
        out.extend_from_slice(&(cb.len() as u32).to_le_bytes());
        out.extend_from_slice(&cb);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < 80 {
            return Err(Error::Proof);
        }
        let header: Header = deserialize(&bytes[..80]).map_err(|_| Error::Proof)?;
        let n = *bytes.get(80).ok_or(Error::Proof)? as usize;
        let mut off = 81;
        let mut branch = Vec::with_capacity(n);
        for _ in 0..n {
            let b = bytes.get(off..off + 32).ok_or(Error::Proof)?;
            branch.push(Txid::from_slice(b).map_err(|_| Error::Proof)?);
            off += 32;
        }
        let len_bytes = bytes.get(off..off + 4).ok_or(Error::Proof)?;
        let len =
            u32::from_le_bytes([len_bytes[0], len_bytes[1], len_bytes[2], len_bytes[3]]) as usize;
        off += 4;
        let end = off.checked_add(len).ok_or(Error::Proof)?;
        let cb = bytes.get(off..end).ok_or(Error::Proof)?;
        let coinbase = deserialize(cb).map_err(|_| Error::Proof)?;
        Ok(Self {
            header,
            branch,
            coinbase,
        })
    }

    /// `branch` is the sibling path for coinbase index 0, bottom-up.
    pub fn verify(&self) -> bool {
        let mut cur = self.coinbase.compute_txid();
        for sib in &self.branch {
            let mut buf = Vec::with_capacity(64);
            buf.extend_from_slice(cur.as_ref());
            buf.extend_from_slice(sib.as_ref());
            cur = Txid::from_raw_hash(bitcoin::hashes::sha256d::Hash::hash(&buf));
        }
        cur.to_raw_hash() == self.header.merkle_root.to_raw_hash()
    }
}

pub fn scan_light(
    proof: &LightProof,
    b_scan: &SecretKey,
    b_spend: &PublicKey,
) -> Result<ScanOutcome, Error> {
    if !proof.verify() {
        return Err(Error::Proof);
    }
    let a_x = carrier_from_tx(&proof.coinbase);
    let Some(a_x) = a_x else {
        return Ok(ScanOutcome {
            is_spc: false,
            a_x: None,
            found: vec![],
        });
    };
    let height =
        parse_bip34_height(proof.coinbase.input[0].script_sig.as_bytes()).ok_or(Error::Proof)?;
    let found = scan_coinbase(height, &a_x, &proof.coinbase, b_scan, b_spend)?;
    Ok(ScanOutcome {
        is_spc: true,
        a_x: Some(a_x),
        found,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TweakFormat {
    Json,
    Sparrow,
}

pub fn export_tweaks(found: &[Found], txid: &Txid, format: TweakFormat) -> String {
    match format {
        TweakFormat::Json => serde_json::to_string_pretty(
            &found
                .iter()
                .map(|f| {
                    serde_json::json!({
                        "outpoint": format!("{}:{}", txid, f.vout),
                        "tweak": hex(&f.tweak),
                        "amount": f.amount,
                    })
                })
                .collect::<Vec<serde_json::Value>>(),
        )
        .unwrap_or_else(|_| "[]".into()),
        TweakFormat::Sparrow => found
            .iter()
            .map(|f| format!("{}:{},{}", txid, f.vout, hex(&f.tweak)))
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// Block source for the CLI.
pub trait BlockSource {
    fn block_at(&self, height: u32) -> Result<Block, String>;
    fn tip_height(&self) -> Result<u32, String>;
}

/// A directory of raw block files, named in ascending height order.
pub struct FileSource {
    blocks: Vec<Block>,
}

impl FileSource {
    pub fn open(dir: &std::path::Path) -> Result<Self, String> {
        let mut paths: Vec<_> = std::fs::read_dir(dir)
            .map_err(|e| e.to_string())?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_file())
            .collect();
        paths.sort();
        let mut blocks = Vec::new();
        for p in paths {
            let bytes = std::fs::read(&p).map_err(|e| e.to_string())?;
            blocks.push(deserialize(&bytes).map_err(|e| format!("{}: {e}", p.display()))?);
        }
        Ok(Self { blocks })
    }
}

impl BlockSource for FileSource {
    fn block_at(&self, height: u32) -> Result<Block, String> {
        self.blocks
            .get(height as usize)
            .cloned()
            .ok_or_else(|| "height out of range".into())
    }
    fn tip_height(&self) -> Result<u32, String> {
        Ok(self.blocks.len().saturating_sub(1) as u32)
    }
}

#[cfg(feature = "rpc")]
pub struct RpcSource {
    client: bitcoincore_rpc::Client,
}

#[cfg(feature = "rpc")]
impl RpcSource {
    pub fn new(url: &str, auth: bitcoincore_rpc::Auth) -> Result<Self, String> {
        let client = bitcoincore_rpc::Client::new(url, auth).map_err(|e| e.to_string())?;
        Ok(Self { client })
    }
}

#[cfg(feature = "rpc")]
impl BlockSource for RpcSource {
    fn block_at(&self, height: u32) -> Result<Block, String> {
        use bitcoincore_rpc::RpcApi;
        let hash = self
            .client
            .get_block_hash(height as u64)
            .map_err(|e| e.to_string())?;
        self.client.get_block(&hash).map_err(|e| e.to_string())
    }
    fn tip_height(&self) -> Result<u32, String> {
        use bitcoincore_rpc::RpcApi;
        Ok(self.client.get_block_count().map_err(|e| e.to_string())? as u32)
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Parse a 32-byte secret key from hex.
pub fn parse_scan_key(s: &str) -> Result<SecretKey, String> {
    let bytes = hex_decode(s).ok_or("scan key is not hex")?;
    SecretKey::from_slice(&bytes).map_err(|e| e.to_string())
}

pub fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

/// Derive `P` for a scan key at `(height, A_send)` (used by tests and tools).
pub fn derive_p(
    height: u32,
    a_x: &[u8; 32],
    b_scan: &SecretKey,
    b_spend: &PublicKey,
) -> Result<(PublicKey, Scalar), Error> {
    let mut a_bytes = [0u8; 33];
    a_bytes[0] = 0x02;
    a_bytes[1..].copy_from_slice(a_x);
    let a_send = PublicKey::from_slice(&a_bytes).map_err(|_| Error::Derivation)?;
    let ecdh = SharedSecret::miner_side(height, b_scan, &a_send).map_err(|_| Error::Derivation)?;
    let tweaks = tier_tweaks(&ecdh, 0).map_err(|_| Error::Derivation)?;
    let p =
        spc_crypto::spc_output_pubkey(b_spend, &tweaks.t_onchain).map_err(|_| Error::Derivation)?;
    let _ = input_hash(height, &a_send);
    Ok((p, tweaks.t_onchain))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::absolute::LockTime;
    use bitcoin::script::ScriptBuf;
    use bitcoin::transaction::Version;
    use bitcoin::{Amount, OutPoint, TxIn, TxOut, Witness};
    use spc_crypto::SendKey;

    fn keys() -> (SecretKey, PublicKey, PublicKey) {
        let secp = bitcoin::secp256k1::Secp256k1::new();
        let scan_sk = SecretKey::from_slice(&[0x11; 32]).unwrap();
        let spend_sk = SecretKey::from_slice(&[0x12; 32]).unwrap();
        let scan = PublicKey::from_secret_key(&secp, &scan_sk);
        let spend = PublicKey::from_secret_key(&secp, &spend_sk);
        (scan_sk, scan, spend)
    }

    fn coinbase(
        height: u32,
        a_x: &[u8; 32],
        outputs: Vec<(Vec<u8>, u64)>,
        carrier: bool,
    ) -> Transaction {
        let mut script = spc_crypto::emit_script_sig(height, a_x);
        script.extend_from_slice(&[0xaa; 8]); // extranonce
        let mut outs = outputs
            .into_iter()
            .map(|(s, v)| TxOut {
                value: Amount::from_sat(v),
                script_pubkey: ScriptBuf::from_bytes(s),
            })
            .collect::<Vec<_>>();
        if carrier {
            outs.push(TxOut {
                value: Amount::ZERO,
                script_pubkey: spc_crypto::emit_op_return(a_x),
            });
        }
        Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::null(),
                script_sig: ScriptBuf::from_bytes(script),
                sequence: bitcoin::Sequence::MAX,
                witness: Witness::new(),
            }],
            output: outs,
        }
    }

    #[test]
    fn scans_both_carriers_and_tolerates_h_minus_one() {
        let (scan_sk, _scan, spend) = keys();
        let send = SendKey::generate(800_000).unwrap();
        let (p, _) = derive_p(800_000, send.x_only(), &scan_sk, &spend).unwrap();
        let p_x = p.x_only_public_key().0.serialize();
        let out = spc_crypto::carriers::TxOut {
            value: 500,
            script_pubkey: &[0x51, 0x20],
        };
        let _ = out;

        let script = {
            let mut s = vec![0x51, 0x20];
            s.extend_from_slice(&p_x);
            s
        };
        let cb = coinbase(800_000, send.x_only(), vec![(script, 500)], false);
        let found = scan_coinbase(800_000, send.x_only(), &cb, &scan_sk, &spend).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].vout, 0);
        assert_eq!(found[0].amount, 500);

        // At H+1 the H-1 tolerance finds the same output.
        let found = scan_coinbase(800_001, send.x_only(), &cb, &scan_sk, &spend).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].height, 800_000);

        // Two heights away is not found.
        let found = scan_coinbase(800_002, send.x_only(), &cb, &scan_sk, &spend).unwrap();
        assert!(found.is_empty());
    }

    #[test]
    fn op_return_carrier_is_read_when_scriptsig_is_absent() {
        let (scan_sk, _scan, spend) = keys();
        let send = SendKey::generate(100).unwrap();
        let (p, _) = derive_p(100, send.x_only(), &scan_sk, &spend).unwrap();
        let p_x = p.x_only_public_key().0.serialize();
        let mut script = vec![0x51, 0x20];
        script.extend_from_slice(&p_x);
        // Build a coinbase whose scriptSig is only the height, then an OP_RETURN.
        let mut tx = coinbase(100, send.x_only(), vec![(script, 330)], true);
        tx.input[0].script_sig = ScriptBuf::from_bytes(spc_crypto::height_push_bytes(100));
        let outcome = scan_block(
            &Block {
                header: Header {
                    version: bitcoin::block::Version::TWO,
                    prev_blockhash: bitcoin::BlockHash::all_zeros(),
                    merkle_root: bitcoin::TxMerkleNode::all_zeros(),
                    time: 0,
                    bits: bitcoin::CompactTarget::from_consensus(0x1d00ffff),
                    nonce: 0,
                },
                txdata: vec![tx],
            },
            &scan_sk,
            &spend,
        )
        .unwrap();
        // merkle_root does not match, but the carrier scan is independent.
        assert!(outcome.is_spc);
        assert_eq!(outcome.found.len(), 1);
    }

    #[test]
    fn bip34_height_parses() {
        assert_eq!(
            parse_bip34_height(&spc_crypto::height_push_bytes(800_000)),
            Some(800_000)
        );
        assert_eq!(
            parse_bip34_height(&spc_crypto::height_push_bytes(16_777_216)),
            Some(16_777_216)
        );
        assert_eq!(parse_bip34_height(&[]), None);
    }

    #[test]
    fn light_proof_verifies_and_scans() {
        let (scan_sk, _scan, spend) = keys();
        let send = SendKey::generate(500).unwrap();
        let (p, _) = derive_p(500, send.x_only(), &scan_sk, &spend).unwrap();
        let p_x = p.x_only_public_key().0.serialize();
        let mut script = vec![0x51, 0x20];
        script.extend_from_slice(&p_x);
        let cb = coinbase(500, send.x_only(), vec![(script, 330)], false);
        let dummy = coinbase(500, send.x_only(), vec![], false);
        let txids = [cb.compute_txid(), dummy.compute_txid()];
        let root = {
            let mut buf = Vec::new();
            buf.extend_from_slice(txids[0].as_ref());
            buf.extend_from_slice(txids[1].as_ref());
            bitcoin::hashes::sha256d::Hash::hash(&buf)
        };
        let header = Header {
            version: bitcoin::block::Version::TWO,
            prev_blockhash: bitcoin::BlockHash::all_zeros(),
            merkle_root: bitcoin::TxMerkleNode::from_raw_hash(root),
            time: 0,
            bits: bitcoin::CompactTarget::from_consensus(0x1d00ffff),
            nonce: 0,
        };
        let proof = LightProof {
            header,
            branch: vec![txids[1]],
            coinbase: cb,
        };
        assert!(proof.verify());
        let encoded = proof.encode();
        let decoded = LightProof::decode(&encoded).unwrap();
        assert_eq!(decoded, proof);
        let outcome = scan_light(&decoded, &scan_sk, &spend).unwrap();
        assert_eq!(outcome.found.len(), 1);
    }

    #[test]
    fn wrong_branch_fails() {
        let (_scan_sk, _scan, _spend) = keys();
        let send = SendKey::generate(500).unwrap();
        let cb = coinbase(500, send.x_only(), vec![], false);
        let bad = LightProof {
            header: Header {
                version: bitcoin::block::Version::TWO,
                prev_blockhash: bitcoin::BlockHash::all_zeros(),
                merkle_root: bitcoin::TxMerkleNode::all_zeros(),
                time: 0,
                bits: bitcoin::CompactTarget::from_consensus(0x1d00ffff),
                nonce: 0,
            },
            branch: vec![],
            coinbase: cb,
        };
        assert!(!bad.verify());
    }
}
