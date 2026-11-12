//! Shared conformance fixtures and the independent verification path for
//! the T1a integration suites.
//!
//! The verification helpers here are deliberately **independent of the
//! signer's code path**: they contain a hand-written raw-transaction
//! parser, a hand-written compact-size codec, a direct SHA-256d over the
//! sha2 crate, and a raw secp256k1 ECDSA verify. Nothing from
//! `bitcoin::hashes` and no `SighashCache` participates, so a signature
//! produced by the builder is checked by a second, unrelated
//! implementation (traffic plan: "independently verified signatures").

// Each integration binary uses a different subset of these helpers;
// unused-in-this-binary items are expected, not drift.
#![allow(dead_code)]

use std::path::Path;

use bitcoin::hashes::Hash as _;
use bitcoin::{OutPoint, ScriptBuf, Txid};
use secp256k1::ecdsa::Signature;
use secp256k1::{Message, PublicKey, Secp256k1};

use spc_lab_txd::builder::{DestinationSpec, PaymentCategory};
use spc_lab_txd::journal::{
    DestinationRecord, FundingContext, FundingObservation, FundingSource, RequestRecord,
};
use spc_lab_txd::keys::{Chain, WalletRole};
use spc_lab_txd::wallet::{PaymentRequest, ProvisionConfig, TrafficWalletSet};

/// The fixed run seed of every conformance fixture (reproducible keys).
pub const TEST_RUN_SEED: u64 = 0x5452_4146_4649_4321;

/// The fixed fixture run id (HMAC derivation domain; derivation v2).
pub const FIXTURE_RUN_ID: &str = "t1a-fixed-fixture";

/// The fixed fixture driver id (journal header binding).
pub const FIXTURE_DRIVER_ID: &str = "t1a-conformance";

/// The command id of the golden fixture payment.
pub const GOLDEN_COMMAND_ID: &str = "golden-payment-1";

/// The synthetic funding amount of the golden fixture (sats).
pub const GOLDEN_FUNDING_SATS: u64 = 1_000_000;

/// The exact destination amount of the golden fixture (sats).
pub const GOLDEN_DEST_SATS: u64 = 250_000;

/// The requested fee rate of the golden fixture (sats/vB).
pub const GOLDEN_FEE_RATE_SATS_VB: u64 = 3;

/// The payer role of the golden fixture.
pub const GOLDEN_PAYER: WalletRole = WalletRole::BootstrapFunding;

/// The destination-owner role of the golden fixture.
pub const GOLDEN_DEST_OWNER: WalletRole = WalletRole::NetworkWallet { index: 0 };

/// The golden fixture: exact signed raw bytes for the fixed seed
/// (`TEST_RUN_SEED`), the fixture run id (`FIXTURE_RUN_ID`, derivation
/// v2), one synthetic 1,000,000-sat UTXO funding the bootstrap wallet,
/// 250,000 sats to network-wallet-0's first receive address at
/// 3 sats/vB, change at index 0. Regenerating this value with the same
/// inputs must reproduce it byte-for-byte (RFC 6979 nonces, deterministic
/// selection and change index).
pub const GOLDEN_RAW_HEX: &str = "02000000014242424242424242424242424242424242424242424242424242424242424242000000006a4730440220660fc74c68c09794c133648cf779dff710a8bc16c638cad94b9477e23a821c0c02207f59320a097dddded1abc3fd24733666a5e1f824073f529e4a57eda473e30614012102a20b330c32d43193c6e306a617d9cc41b3fc915121f1af0f1927ef2d02258383ffffffff0290d00300000000001976a914ae37f857ff78b68705ba3f8788a16beaf39a35e488ac076f0b00000000001976a914e42ddfc72d5b9c7452a1f439bb89d2a3b6f2642588ac00000000";

/// The golden fixture txid (the SHA-256d of `GOLDEN_RAW_HEX`, in the
/// conventional display byte order).
pub const GOLDEN_TXID: &str = "10636a5abf76021132012a01f34c22c42d2f302507e210ed50514cf60f446211";

// ---------------------------------------------------------------------
// Conformance fixture setup
// ---------------------------------------------------------------------

/// A deterministic synthetic funding outpoint (arbitrary fixture txid).
pub fn fixture_outpoint(byte: u8) -> OutPoint {
    OutPoint::new(Txid::from_byte_array([byte; 32]), 0)
}

/// Provision a conformance wallet set (synthetic fixtures admitted).
pub fn conformance_set(dir: &Path) -> TrafficWalletSet {
    TrafficWalletSet::provision_for_conformance_tests(&standard_test_config(), dir)
        .expect("provision conformance wallet set")
}

/// Reopen a conformance wallet set.
pub fn reopen_conformance_set(dir: &Path) -> TrafficWalletSet {
    TrafficWalletSet::reopen_for_conformance_tests(&standard_test_config(), dir)
        .expect("reopen conformance wallet set")
}

pub fn standard_test_config() -> ProvisionConfig {
    ProvisionConfig::standard(TEST_RUN_SEED, FIXTURE_RUN_ID, FIXTURE_DRIVER_ID)
}

/// One synthetic UTXO of `amount_sats` paid to the payer's next receive
/// address. Returns (outpoint, funding script).
pub fn fund_payer(
    set: &TrafficWalletSet,
    payer: &WalletRole,
    outpoint: OutPoint,
    amount_sats: u64,
) -> (OutPoint, ScriptBuf) {
    let index = set.journal().next_receive_index(payer);
    let address = set
        .next_receive_address(payer)
        .expect("payer receive address");
    let script = address.script_pubkey.clone();
    set.observe_funding(&FundingObservation {
        outpoint,
        amount_sats,
        chain: Chain::Receive,
        index,
        owner_role: payer.clone(),
        source: FundingSource::SyntheticFixture,
        context: FundingContext::Fixture,
    })
    .expect("observe synthetic fixture funding");
    (outpoint, script)
}

/// The golden payment request: `GOLDEN_DEST_SATS` to the destination
/// owner's first receive address at `GOLDEN_FEE_RATE_SATS_VB`.
pub fn golden_request(set: &TrafficWalletSet, command_id: &str) -> PaymentRequest {
    let address = set
        .next_receive_address(&GOLDEN_DEST_OWNER)
        .expect("destination receive address");
    PaymentRequest {
        command_id: command_id.to_string(),
        payer_role: GOLDEN_PAYER.clone(),
        category: PaymentCategory::Ordinary,
        destinations: vec![DestinationSpec {
            script_pubkey: address.script_pubkey,
            amount_sats: GOLDEN_DEST_SATS,
            owner: Some(GOLDEN_DEST_OWNER.clone()),
        }],
        fee_rate_sats_vb: GOLDEN_FEE_RATE_SATS_VB,
    }
}

// ---------------------------------------------------------------------
// Independent verification path (no bitcoin::hashes, no SighashCache)
// ---------------------------------------------------------------------

/// SHA-256d (`SHA256(SHA256(x))`) over the sha2 crate directly.
pub fn sha256d(data: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    let once = sha2::Sha256::digest(data);
    sha2::Sha256::digest(once).into()
}

/// A hand-written RIPEMD-160 (the second half of Bitcoin's HASH160),
/// so the independent verification path never touches `bitcoin::hashes`.
/// Validated against the published test vectors in
/// `ripemd160_matches_published_vectors` below.
pub fn ripemd160(data: &[u8]) -> [u8; 20] {
    const RL: [u32; 5] = [
        0x0000_0000,
        0x5A82_7999,
        0x6ED9_EBA1,
        0x8F1B_BCDC,
        0xA953_FD4E,
    ];
    const RR: [u32; 5] = [
        0x50A2_8BE6,
        0x5C4D_D124,
        0x6D70_3EF3,
        0x7A6D_76E9,
        0x0000_0000,
    ];
    const ROT_L: [u32; 80] = [
        11, 14, 15, 12, 5, 8, 7, 9, 11, 13, 14, 15, 6, 7, 9, 8, 7, 6, 8, 13, 11, 9, 7, 15, 7, 12,
        15, 9, 11, 7, 13, 12, 11, 13, 6, 7, 14, 9, 13, 15, 14, 8, 13, 6, 5, 12, 7, 5, 11, 12, 14,
        15, 14, 15, 9, 8, 9, 14, 5, 6, 8, 6, 5, 12, 9, 15, 5, 11, 6, 8, 13, 12, 5, 12, 13, 14, 11,
        8, 5, 6,
    ];
    const ROT_R: [u32; 80] = [
        8, 9, 9, 11, 13, 15, 15, 5, 7, 7, 8, 11, 14, 14, 12, 6, 9, 13, 15, 7, 12, 8, 9, 11, 7, 7,
        12, 7, 6, 15, 13, 11, 9, 7, 15, 11, 8, 6, 6, 14, 12, 13, 5, 14, 13, 13, 7, 5, 15, 5, 8, 11,
        14, 14, 6, 14, 6, 9, 12, 9, 12, 5, 15, 8, 8, 5, 12, 9, 12, 5, 14, 6, 8, 13, 6, 5, 15, 13,
        11, 11,
    ];
    const WORD_L: [usize; 80] = [
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 7, 4, 13, 1, 10, 6, 15, 3, 12, 0, 9,
        5, 2, 14, 11, 8, 3, 10, 14, 4, 9, 15, 8, 1, 2, 7, 0, 6, 13, 11, 5, 12, 1, 9, 11, 10, 0, 8,
        12, 4, 13, 3, 7, 15, 14, 5, 6, 2, 4, 0, 5, 9, 7, 12, 2, 10, 14, 1, 3, 8, 11, 6, 15, 13,
    ];
    const WORD_R: [usize; 80] = [
        5, 14, 7, 0, 9, 2, 11, 4, 13, 6, 15, 8, 1, 10, 3, 12, 6, 11, 3, 7, 0, 13, 5, 10, 14, 15, 8,
        12, 4, 9, 1, 2, 15, 5, 1, 3, 7, 14, 6, 9, 11, 8, 12, 2, 10, 0, 4, 13, 8, 6, 4, 1, 3, 11,
        15, 0, 5, 12, 2, 13, 9, 7, 10, 14, 12, 15, 10, 4, 1, 5, 8, 7, 6, 2, 13, 14, 0, 3, 9, 11,
    ];
    fn f_left(j: usize, x: u32, y: u32, z: u32) -> u32 {
        match j / 16 {
            0 => x ^ y ^ z,
            1 => (x & y) | (!x & z),
            2 => (x | !y) ^ z,
            3 => (x & z) | (y & !z),
            _ => x ^ (y | !z),
        }
    }
    fn f_right(j: usize, x: u32, y: u32, z: u32) -> u32 {
        match j / 16 {
            0 => x ^ (y | !z),
            1 => (x & z) | (y & !z),
            2 => (x | !y) ^ z,
            3 => (x & y) | (!x & z),
            _ => x ^ y ^ z,
        }
    }
    let rol = |value: u32, amount: u32| value.rotate_left(amount);

    // MD4-style little-endian padding: 0x80, zeros, 64-bit bit length.
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&((data.len() as u64) * 8).to_le_bytes());

    let mut h: [u32; 5] = [
        0x6745_2301,
        0xEFCD_AB89,
        0x98BA_DCFE,
        0x1032_5476,
        0xC3D2_E1F0,
    ];
    for block in message.chunks_exact(64) {
        let mut x = [0u32; 16];
        for (i, word) in x.iter_mut().enumerate() {
            *word = u32::from_le_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ]);
        }
        let (mut al, mut bl, mut cl, mut dl, mut el) = (h[0], h[1], h[2], h[3], h[4]);
        let (mut ar, mut br, mut cr, mut dr, mut er) = (h[0], h[1], h[2], h[3], h[4]);
        for j in 0..80 {
            let round = j / 16;
            let tl = rol(
                al.wrapping_add(f_left(j, bl, cl, dl))
                    .wrapping_add(x[WORD_L[j]])
                    .wrapping_add(RL[round]),
                ROT_L[j],
            )
            .wrapping_add(el);
            al = el;
            el = dl;
            dl = rol(cl, 10);
            cl = bl;
            bl = tl;
            let tr = rol(
                ar.wrapping_add(f_right(j, br, cr, dr))
                    .wrapping_add(x[WORD_R[j]])
                    .wrapping_add(RR[round]),
                ROT_R[j],
            )
            .wrapping_add(er);
            ar = er;
            er = dr;
            dr = rol(cr, 10);
            cr = br;
            br = tr;
        }
        let t = h[1].wrapping_add(cl).wrapping_add(dr);
        h[1] = h[2].wrapping_add(dl).wrapping_add(er);
        h[2] = h[3].wrapping_add(el).wrapping_add(ar);
        h[3] = h[4].wrapping_add(al).wrapping_add(br);
        h[4] = h[0].wrapping_add(bl).wrapping_add(cr);
        h[0] = t;
    }
    let mut out = [0u8; 20];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    out
}

/// Bitcoin's HASH160: RIPEMD160(SHA256(x)), both over this module's own
/// implementations.
pub fn hash160(data: &[u8]) -> [u8; 20] {
    use sha2::Digest;
    let once: [u8; 32] = sha2::Sha256::digest(data).into();
    ripemd160(&once)
}

/// A hand-written compact-size (varint) encoder.
pub fn encode_varint(out: &mut Vec<u8>, value: u64) {
    if value < 0xFD {
        out.push(value as u8);
    } else if value <= 0xFFFF {
        out.push(0xFD);
        out.extend_from_slice(&(value as u16).to_le_bytes());
    } else if value <= 0xFFFF_FFFF {
        out.push(0xFE);
        out.extend_from_slice(&(value as u32).to_le_bytes());
    } else {
        out.push(0xFF);
        out.extend_from_slice(&value.to_le_bytes());
    }
}

/// A hand-written compact-size (varint) decoder (fail-closed cursor).
struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Cursor<'_> {
    fn take(&mut self, len: usize) -> &[u8] {
        assert!(
            self.pos + len <= self.data.len(),
            "raw tx truncated at byte {} wanting {} more",
            self.pos,
            len
        );
        let slice = &self.data[self.pos..self.pos + len];
        self.pos += len;
        slice
    }

    fn u32_le(&mut self) -> u32 {
        let bytes = self.take(4);
        u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    }

    fn u64_le(&mut self) -> u64 {
        let bytes = self.take(8);
        u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ])
    }

    fn varint(&mut self) -> u64 {
        let prefix = self.take(1)[0];
        match prefix {
            0xFD => {
                let bytes = self.take(2);
                u64::from(u16::from_le_bytes([bytes[0], bytes[1]]))
            }
            0xFE => {
                let bytes = self.take(4);
                u64::from(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
            }
            0xFF => self.u64_le(),
            n => u64::from(n),
        }
    }

    fn script(&mut self) -> Vec<u8> {
        let len = self.varint() as usize;
        self.take(len).to_vec()
    }
}

/// A raw legacy transaction parsed by the hand-written decoder above.
pub struct ParsedTx {
    pub version: u32,
    pub inputs: Vec<ParsedInput>,
    pub outputs: Vec<ParsedOutput>,
    pub locktime: u32,
}

pub struct ParsedInput {
    pub prevout_txid: [u8; 32],
    pub prevout_vout: u32,
    pub script_sig: Vec<u8>,
    pub sequence: u32,
}

pub struct ParsedOutput {
    pub value_sats: u64,
    pub script_pubkey: Vec<u8>,
}

/// Parse a raw legacy transaction byte-for-byte. Fails closed on any
/// trailing garbage (the serialization must be exactly consumed).
pub fn parse_raw_tx(raw: &[u8]) -> ParsedTx {
    let mut cursor = Cursor { data: raw, pos: 0 };
    let version = cursor.u32_le();
    let input_count = cursor.varint();
    assert!(input_count > 0, "a transaction must have inputs");
    assert!(input_count <= 100_000, "input count out of range");
    let mut inputs = Vec::with_capacity(input_count as usize);
    for _ in 0..input_count {
        let prevout_txid: [u8; 32] = cursor.take(32).try_into().expect("32 txid bytes");
        let prevout_vout = cursor.u32_le();
        let script_sig = cursor.script();
        let sequence = cursor.u32_le();
        inputs.push(ParsedInput {
            prevout_txid,
            prevout_vout,
            script_sig,
            sequence,
        });
    }
    let output_count = cursor.varint();
    assert!(output_count > 0, "a transaction must have outputs");
    assert!(output_count <= 100_000, "output count out of range");
    let mut outputs = Vec::with_capacity(output_count as usize);
    for _ in 0..output_count {
        let value_sats = cursor.u64_le();
        let script_pubkey = cursor.script();
        outputs.push(ParsedOutput {
            value_sats,
            script_pubkey,
        });
    }
    let locktime = cursor.u32_le();
    assert_eq!(
        cursor.pos,
        raw.len(),
        "raw tx has trailing bytes after locktime"
    );
    ParsedTx {
        version,
        inputs,
        outputs,
        locktime,
    }
}

/// One input's funding facts for independent verification.
pub struct VerifyUtxo {
    pub prevout_txid: [u8; 32],
    pub prevout_vout: u32,
    pub amount_sats: u64,
    pub script_pubkey: Vec<u8>,
}

/// The result of a successful independent verification.
pub struct VerifiedFacts {
    /// The SHA-256d of the exact raw bytes (internal byte order), from
    /// this module's own hashing - not the builder's claim.
    pub txid: [u8; 32],
    pub fee_sats: u64,
    pub output_total_sats: u64,
    pub output_values: Vec<u64>,
    pub output_scripts: Vec<Vec<u8>>,
}

/// The legacy SIGHASH_ALL preimage for `input_index`, built by hand from
/// the parsed transaction (the signed input carries its funding script,
/// all others the empty script), hashed with the sha2 crate directly.
fn legacy_sighash_all(tx: &ParsedTx, input_index: usize, funding_script: &[u8]) -> [u8; 32] {
    let mut preimage = Vec::with_capacity(512);
    preimage.extend_from_slice(&tx.version.to_le_bytes());
    encode_varint(&mut preimage, tx.inputs.len() as u64);
    for (index, input) in tx.inputs.iter().enumerate() {
        preimage.extend_from_slice(&input.prevout_txid);
        preimage.extend_from_slice(&input.prevout_vout.to_le_bytes());
        let script: &[u8] = if index == input_index {
            funding_script
        } else {
            &[]
        };
        encode_varint(&mut preimage, script.len() as u64);
        preimage.extend_from_slice(script);
        preimage.extend_from_slice(&input.sequence.to_le_bytes());
    }
    encode_varint(&mut preimage, tx.outputs.len() as u64);
    for output in &tx.outputs {
        preimage.extend_from_slice(&output.value_sats.to_le_bytes());
        encode_varint(&mut preimage, output.script_pubkey.len() as u64);
        preimage.extend_from_slice(&output.script_pubkey);
    }
    preimage.extend_from_slice(&tx.locktime.to_le_bytes());
    // SIGHASH_ALL (0x01) as the 4-byte little-endian hashtype suffix.
    preimage.extend_from_slice(&1u32.to_le_bytes());
    sha256d(&preimage)
}

/// Split a P2PKH scriptSig into (DER signature, hashtype, pubkey) using
/// the push-opcode layout directly: `<push> <DER+hashtype> <33> <pubkey>`.
fn split_p2pkh_script_sig(script_sig: &[u8]) -> (Vec<u8>, u8, Vec<u8>) {
    // Minimum: push byte + smallest DER (8) + hashtype + push + 33 bytes.
    assert!(script_sig.len() >= 1 + 9 + 1 + 33, "scriptSig too short");
    let sig_push = script_sig[0] as usize;
    assert!(sig_push < 0xFD, "one-byte signature push expected");
    assert_eq!(
        script_sig.len(),
        1 + sig_push + 1 + 33,
        "scriptSig is not exactly two pushes"
    );
    let der = script_sig[1..sig_push].to_vec();
    let hashtype = script_sig[sig_push];
    assert_eq!(script_sig[sig_push + 1], 33, "33-byte pubkey push");
    let pubkey = script_sig[sig_push + 2..script_sig.len()].to_vec();
    (der, hashtype, pubkey)
}

/// Independently verify a signed traffic transaction against its funding
/// facts: own txid hash, own raw parser, own sighash serialization, raw
/// secp256k1 verification, low-S and SIGHASH_ALL checks, exact fee
/// arithmetic, and the fixed v1 shape (version 2, locktime 0, max
/// sequence). Returns the output facts for caller-side assertions.
pub fn verify_signed_traffic(raw: &[u8], utxos: &[VerifyUtxo]) -> VerifiedFacts {
    verify_signed_traffic_version(raw, utxos, 2)
}

/// Only the explicit before-sign acceptance fixture uses this verifier. The
/// normal verifier above continues to require exact version 2.
pub fn verify_version_rejection_traffic(raw: &[u8], utxos: &[VerifyUtxo]) -> VerifiedFacts {
    verify_signed_traffic_version(raw, utxos, 3)
}
fn verify_signed_traffic_version(raw: &[u8], utxos: &[VerifyUtxo], version: u32) -> VerifiedFacts {
    assert!(!raw.is_empty(), "non-empty raw bytes");

    // Own txid: SHA-256d over the exact raw bytes.
    let txid = sha256d(raw);

    let tx = parse_raw_tx(raw);
    assert_eq!(tx.version, version, "exact selected construction version");
    assert_eq!(tx.locktime, 0, "traffic v1 profile uses locktime 0");
    assert_eq!(tx.inputs.len(), utxos.len(), "one funding fact per input");

    let secp = Secp256k1::verification_only();
    let mut input_total: u64 = 0;
    for (index, input) in tx.inputs.iter().enumerate() {
        let utxo = &utxos[index];
        assert_eq!(
            input.prevout_txid, utxo.prevout_txid,
            "prevout txid matches"
        );
        assert_eq!(
            input.prevout_vout, utxo.prevout_vout,
            "prevout vout matches"
        );
        assert_eq!(
            input.sequence, 0xFFFF_FFFF,
            "traffic v1 profile uses max sequence"
        );
        input_total = input_total
            .checked_add(utxo.amount_sats)
            .expect("input total fits");

        let (der, hashtype, pubkey_bytes) = split_p2pkh_script_sig(&input.script_sig);
        assert_eq!(hashtype, 0x01, "SIGHASH_ALL");
        let mut signature = Signature::from_der(&der).expect("DER signature parses");
        let der_before = signature.serialize_der();
        signature.normalize_s();
        assert!(
            der_before.as_ref() == signature.serialize_der().as_ref(),
            "signature must be low-S"
        );

        let sighash = legacy_sighash_all(&tx, index, &utxo.script_pubkey);
        let message = Message::from_digest(sighash);
        let public_key = PublicKey::from_slice(&pubkey_bytes).expect("pubkey parses");
        secp.verify_ecdsa(&message, &signature, &public_key)
            .expect("independent secp256k1 verification succeeds");

        // The pubkey push must HASH160 to the P2PKH funding script it
        // spends (hand-written RIPEMD-160 over the sha2 SHA-256).
        let expected_script = {
            let hash = hash160(&pubkey_bytes);
            let mut script = vec![0x76, 0xa9, 0x14]; // OP_DUP OP_HASH160 PUSH20
            script.extend_from_slice(&hash);
            script.extend_from_slice(&[0x88, 0xac]); // OP_EQUALVERIFY OP_CHECKSIG
            script
        };
        assert_eq!(
            utxo.script_pubkey, expected_script,
            "funding script is P2PKH of the pushed pubkey"
        );
    }

    let mut output_total: u64 = 0;
    let mut output_values = Vec::with_capacity(tx.outputs.len());
    for output in &tx.outputs {
        output_total = output_total
            .checked_add(output.value_sats)
            .expect("output total fits");
        output_values.push(output.value_sats);
    }
    let fee_sats = input_total
        .checked_sub(output_total)
        .expect("fee is inputs minus outputs, no underflow");
    assert!(fee_sats > 0, "positive fee");

    VerifiedFacts {
        txid,
        fee_sats,
        output_total_sats: output_total,
        output_values,
        output_scripts: tx.outputs.iter().map(|o| o.script_pubkey.clone()).collect(),
    }
}

/// The canonical `RequestRecord` of a payment request (the digest-covered
/// form `TrafficWalletSet::payload_digest` hashes), for driving the
/// journal to an exact crash phase in durability tests.
pub fn request_record_of(request: &PaymentRequest) -> RequestRecord {
    RequestRecord {
        construction_effect: None,
        payer_role: request.payer_role.clone(),
        category: request.category.clone(),
        destinations: request
            .destinations
            .iter()
            .map(|destination| DestinationRecord {
                script_hex: destination.script_pubkey.to_hex_string(),
                amount_sats: destination.amount_sats,
                owner_role: destination.owner.clone(),
            })
            .collect(),
        fee_rate_sats_vb: request.fee_rate_sats_vb,
    }
}

/// Decode a hex string with the hex crate (the same encoding the journal
/// stores) into raw bytes.
pub fn decode_hex(hex_str: &str) -> Vec<u8> {
    hex::decode(hex_str).expect("valid hex")
}

#[cfg(test)]
mod ripemd160_vectors {
    use super::ripemd160;

    fn hex_str(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// The published RIPEMD-160 test vectors; if this fails, the
    /// hand-written implementation (and every HASH160 check built on it)
    /// is wrong.
    #[test]
    fn ripemd160_matches_published_vectors() {
        assert_eq!(
            hex_str(&ripemd160(b"")),
            "9c1185a5c5e9fc54612808977ee8f548b2258d31"
        );
        assert_eq!(
            hex_str(&ripemd160(b"abc")),
            "8eb208f7e05d987a9b044a8e98c6b087f15a0bfc"
        );
        assert_eq!(
            hex_str(&ripemd160(b"message digest")),
            "5d0689ef49d2fae572b881b123a85ffa21595f36"
        );
        assert_eq!(
            hex_str(&ripemd160(b"a")),
            "0bdc9d2d256b3ee9daae347be6f4dc835a467ffe"
        );
        assert_eq!(
            hex_str(&ripemd160(b"abcdefghijklmnopqrstuvwxyz")),
            "f71c27109c692c1b56bbdceb5b9d2865b3708dbc"
        );
        // A 56-byte message (the paper's 448-bit vector).
        assert_eq!(
            hex_str(&ripemd160(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "12a053384a9c0c88e405a06c27dcf49ada62eb2b"
        );
        // The paper's million-'a' vector exercises multi-block chunking
        // (confirmed against an independent OpenSSL RIPEMD-160).
        assert_eq!(
            hex_str(&ripemd160(b"a".repeat(1_000_000).as_slice())),
            "52783243c1697bdbe16d37f97f68f08325dc1528"
        );
    }
}
