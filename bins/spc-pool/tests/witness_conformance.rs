//! Frozen witness-commitment conformance vectors (peer-cleared T4a).
//!
//! Every expected value below was produced by an **independent** reference
//! implementation (`/tmp/opencode/T4a-vectors/gen.py`, hashlib-only Python,
//! hand-assembled transactions - no shared code with this crate), then
//! frozen as hex. The transactions mix legacy, P2WPKH and P2TR key-path
//! spends, exactly as a mainnet template would.
//!
//! R9 (Fable rework 2026-11-08): the P2WPKH/P2TR inputs below carry
//! **structural pattern signatures** - DER/Schnorr-shaped byte strings with
//! the right lengths and script shapes - not signatures that would validly
//! spend the (nonexistent) prevouts. The vectors exercise txid/wtxid
//! separation, witness-root folding and commitment binding only; spend
//! validity is out of scope and is exercised with real keys in T4b.
//!
//! Negative vectors prove the fail-closed paths: wrong reserved value,
//! missing/duplicate/malformed/valued commitment output, transactions that
//! do not strictly decode, and a merkle root that would result from hashing
//! the *full* (witness-carrying) serialization instead of the stripped one.

use bitcoin::hashes::Hash;
use spc_pool::block::{
    assemble_witness, coinbase_merkle_branch, coinbase_with_witness, merkle_root, txid,
    AssembledBlock, BlockError,
};
use spc_pool::coinbase::{commitment_output, try_build_pool_template};
use spc_pool::template::analyze_transactions;
use spc_pool::witness::{
    self, commitment_from_script, WitnessError, WITNESS_COMMITMENT_OVERHEAD_WU,
};

fn hex(s: &str) -> Vec<u8> {
    assert!(s.len().is_multiple_of(2), "odd hex length");
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex digit"))
        .collect()
}

fn hex32(s: &str) -> [u8; 32] {
    hex(s).try_into().expect("32 bytes")
}

/// The four non-coinbase transactions, full serializations, template order:
/// a legacy tx, a P2WPKH, a P2TR key-path spend and a second P2WPKH.
const TRANSACTIONS_FULL: [&str; 4] = [
    "020000000111111111111111111111111111111111111111111111111111111111111111110000000017606162636465666768696a6b6c6d6e6f70717273747576ffffffff01905f0100000000001976a914a0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b388ac00000000",
    "0200000000010122222222222222222222222222222222222222222222222222222222222222220000000000ffffffff018038010000000000160014e1e2e3e4e5e6e7e8e9eaebecedeeeff0f1f2f3f40247304402200102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f2002202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f40012102c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe000000000",
    "0200000000010133333333333333333333333333333333333333333333333333333333333333330000000000ffffffff017011010000000000225120f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff000102030405060708090a0b0c0d0e0f0140101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f00000000",
    "0200000000010144444444444444444444444444444444444444444444444444444444444444440000000000ffffffff0160ea0000000000001600149192939495969798999a9b9c9d9e9fa0a1a2a3a40248304502215152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707102202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f400121036162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f8000000000",
];

/// The same transactions, witness-stripped.
const TRANSACTIONS_STRIPPED: [&str; 4] = [
    "020000000111111111111111111111111111111111111111111111111111111111111111110000000017606162636465666768696a6b6c6d6e6f70717273747576ffffffff01905f0100000000001976a914a0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b388ac00000000",
    "020000000122222222222222222222222222222222222222222222222222222222222222220000000000ffffffff018038010000000000160014e1e2e3e4e5e6e7e8e9eaebecedeeeff0f1f2f3f400000000",
    "020000000133333333333333333333333333333333333333333333333333333333333333330000000000ffffffff017011010000000000225120f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff000102030405060708090a0b0c0d0e0f00000000",
    "020000000144444444444444444444444444444444444444444444444444444444444444440000000000ffffffff0160ea0000000000001600149192939495969798999a9b9c9d9e9fa0a1a2a3a400000000",
];

/// The P2TR spend with a different 64-byte schnorr witness: same txid,
/// different wtxid - the witness-malleation vector.
const TR1_MALLEATED: &str = "0200000000010133333333333333333333333333333333333333333333333333333333333333330000000000ffffffff017011010000000000225120f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff000102030405060708090a0b0c0d0e0f01401112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f304142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f6000000000";

const COINBASE_STRIPPED: &str = "02000000010000000000000000000000000000000000000000000000000000000000000000ffffffff2d0300350c20404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5faa11bb22cc33dd44ffffffff02598fa01200000000225120c0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedf0000000000000000266a24aa21a9ed91959ba5b529dcc96825a62b66e67862cec627160ce3f2dcf44e736a076f910b00000000";
const COINBASE_FULL: &str = "020000000001010000000000000000000000000000000000000000000000000000000000000000ffffffff2d0300350c20404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5faa11bb22cc33dd44ffffffff02598fa01200000000225120c0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedf0000000000000000266a24aa21a9ed91959ba5b529dcc96825a62b66e67862cec627160ce3f2dcf44e736a076f910b0120808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f00000000";

const EXTRANONCE: &str = "aa11bb22cc33dd44";
const RESERVED: &str = "808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f";

const TXIDS: [&str; 4] = [
    "3ada0dbdde8c5897b2c62b607d6b2734ec8c430016ff4c720b093f605c76a22d",
    "e76a1e8ccb0f7a3dd838c19289e53c683c2d51bee88d23624e694d524240e0ed",
    "bd26582e536f6ad83ddb6cc4d3a410c62c6d1e2e20478b35962319cfb214dc48",
    "b640bb62c2aca4a89392b94ba816db81ababfb9d7e6f4206cff9fe11cef02987",
];
const WTXIDS: [&str; 4] = [
    "3ada0dbdde8c5897b2c62b607d6b2734ec8c430016ff4c720b093f605c76a22d",
    "5b609b2a8076c1f3c13fa599c149c13bf78e9a166f19101d4ac901024fc91df3",
    "1b9bf445dd8d6d345b44ec7daa51f556a8bf65a3d372e5041f2c3d22dd0204ff",
    "81095888ec4045b218570f78c5ba57592fe7d92d241bf77ffde545eec022d628",
];

const COINBASE_TXID: &str = "8a51bac906638fde5715c4d046ee0bc4e03b1538cdc26453360604a2032d0dc1";
const WITNESS_ROOT: &str = "074a69eca6c311a8d7dec43231be8be5dc89800e592969345f577e7d21e5d836";
const COMMITMENT: &str = "91959ba5b529dcc96825a62b66e67862cec627160ce3f2dcf44e736a076f910b";
const COMMITMENT_SCRIPT: &str =
    "6a24aa21a9ed91959ba5b529dcc96825a62b66e67862cec627160ce3f2dcf44e736a076f910b";
const MERKLE_ROOT: &str = "c1c5d9e383219ba93c39f245df9003d55606d0e9200da01662181fbed13783b4";
const MERKLE_BRANCH: [&str; 3] = [
    "3ada0dbdde8c5897b2c62b607d6b2734ec8c430016ff4c720b093f605c76a22d",
    "97d394a119ed4e1901048953532ab472138af73233a20d4e776480f4e2485163",
    "379af345e2876a2de114eda126284d89addfd890f338b8abfda41da5b9412908",
];
const HEADER: &str = "020000001111111111111111111111111111111111111111111111111111111111111111c1c5d9e383219ba93c39f245df9003d55606d0e9200da01662181fbed13783b400f15365ffff7f2007000000";
const HEADER_HASH: &str = "d653c17dc4f81550e06b906d37cb0f30fc8c49062c371a39e84340e780fbf068";
const BLOCK: &str = "020000001111111111111111111111111111111111111111111111111111111111111111c1c5d9e383219ba93c39f245df9003d55606d0e9200da01662181fbed13783b400f15365ffff7f200700000005020000000001010000000000000000000000000000000000000000000000000000000000000000ffffffff2d0300350c20404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5faa11bb22cc33dd44ffffffff02598fa01200000000225120c0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedf0000000000000000266a24aa21a9ed91959ba5b529dcc96825a62b66e67862cec627160ce3f2dcf44e736a076f910b0120808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f00000000020000000111111111111111111111111111111111111111111111111111111111111111110000000017606162636465666768696a6b6c6d6e6f70717273747576ffffffff01905f0100000000001976a914a0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b388ac000000000200000000010122222222222222222222222222222222222222222222222222222222222222220000000000ffffffff018038010000000000160014e1e2e3e4e5e6e7e8e9eaebecedeeeff0f1f2f3f40247304402200102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f2002202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f40012102c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0000000000200000000010133333333333333333333333333333333333333333333333333333333333333330000000000ffffffff017011010000000000225120f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff000102030405060708090a0b0c0d0e0f0140101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f000000000200000000010144444444444444444444444444444444444444444444444444444444444444440000000000ffffffff0160ea0000000000001600149192939495969798999a9b9c9d9e9fa0a1a2a3a40248304502215152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707102202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f400121036162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f8000000000";

fn transactions_full() -> Vec<Vec<u8>> {
    TRANSACTIONS_FULL.iter().map(|s| hex(s)).collect()
}

fn transactions_stripped() -> Vec<Vec<u8>> {
    TRANSACTIONS_STRIPPED.iter().map(|s| hex(s)).collect()
}

fn frozen_set() -> spc_pool::template::TransactionSet {
    analyze_transactions(&transactions_full()).expect("the frozen txs strictly decode")
}

#[test]
fn txids_wtxids_and_witness_count_match_the_reference() {
    let set = frozen_set();
    let expected_txids: Vec<[u8; 32]> = TXIDS.iter().map(|s| hex32(s)).collect();
    let expected_wtxids: Vec<[u8; 32]> = WTXIDS.iter().map(|s| hex32(s)).collect();
    assert_eq!(set.txids, expected_txids);
    assert_eq!(set.wtxids, expected_wtxids);
    // One legacy tx plus three segwit txs.
    assert_eq!(set.witness_count, 3);
    // The legacy tx's wtxid equals its txid (BIP 141).
    assert_eq!(set.txids[0], set.wtxids[0]);
    assert_ne!(set.txids[1], set.wtxids[1]);
}

#[test]
fn strict_decode_rejects_every_ambiguity() {
    // Trailing bytes after a well-formed witness tx.
    let mut trailing = hex(TRANSACTIONS_FULL[1]);
    trailing.push(0x00);
    assert!(analyze_transactions(&[trailing]).is_err());
    // Truncated.
    let truncated = hex(TRANSACTIONS_FULL[1]);
    assert!(analyze_transactions(&[truncated[..truncated.len() - 1].to_vec()]).is_err());
    // A segwit marker/flag with an all-empty witness: decodes, but
    // re-serializes without the marker, so it is ambiguous.
    let mut marker_empty = hex(TRANSACTIONS_STRIPPED[1]);
    marker_empty.insert(4, 0x00);
    marker_empty.insert(5, 0x01);
    let at = marker_empty.len() - 4;
    marker_empty.insert(at, 0x00); // one input, zero witness items
    assert!(analyze_transactions(&[marker_empty]).is_err());
    // Garbage.
    assert!(analyze_transactions(&[vec![0x00, 0x01]]).is_err());
    assert!(analyze_transactions(&[Vec::new()]).is_err());
    // The reviewer's raw zero-input vector (Fable rework 2026-11-08): a
    // marker/flag transaction with zero inputs that decodes and roundtrips
    // byte-for-byte. It is not consensus-valid (no inputs), and its wtxid
    // differs from its txid while no input carries witness data - the exact
    // shape that must never fall through to the legacy txid path.
    let zero_input = hex("020000000001000101000000000000000000000000");
    assert!(analyze_transactions(&[zero_input]).is_err());
}

#[test]
fn witness_root_and_commitment_match_the_reference() {
    let set = frozen_set();
    let root = witness::witness_root(&set.wtxids);
    assert_eq!(root, hex32(WITNESS_ROOT));
    let reserved = hex32(RESERVED);
    let commitment = witness::witness_commitment(&root, &reserved);
    assert_eq!(commitment, hex32(COMMITMENT));
    // The commitment script and its parse.
    let script = witness::commitment_script(&commitment);
    assert_eq!(script, hex(COMMITMENT_SCRIPT));
    assert_eq!(commitment_from_script(&script), Some(commitment));
    // A wrong reserved value yields a different commitment: the
    // commitment is binding on both inputs.
    let wrong = witness::witness_commitment(&root, &[0x00; 32]);
    assert_ne!(wrong, commitment);
    let wrong_root = witness::witness_root(&[set.wtxids[0], set.wtxids[1]]);
    assert_ne!(
        witness::witness_commitment(&wrong_root, &reserved),
        commitment
    );
}

#[test]
fn coinbase_builder_reproduces_the_reference_coinbase() {
    // The frozen coinbase is exactly what `try_build_pool_template`
    // produces for this height, carrier key, extranonce length and
    // outputs - so the prefix/suffix split a miner hashes (coinb1/ex2/
    // coinb2) is the reference's byte string.
    let stripped = hex(COINBASE_STRIPPED);
    let decoded: bitcoin::Transaction =
        bitcoin::consensus::deserialize(&stripped).expect("frozen coinbase decodes");
    let outputs: Vec<(bitcoin::script::ScriptBuf, u64)> = decoded
        .output
        .iter()
        .map(|o| (o.script_pubkey.clone(), o.value.to_sat()))
        .collect();
    let mut a_x = [0u8; 32];
    for (i, b) in a_x.iter_mut().enumerate() {
        *b = 0x40 + i as u8;
    }
    let template = try_build_pool_template(2, 800_000, &a_x, 8, &outputs, 0)
        .expect("the frozen coinbase fits the scriptSig budget");
    assert_eq!(template.assemble(&hex(EXTRANONCE)), stripped);
    // ... and the witness injection reproduces the full coinbase.
    let full = template
        .assemble_with_witness(&hex(EXTRANONCE), &hex32(RESERVED))
        .expect("witness injection");
    assert_eq!(full, hex(COINBASE_FULL));
    // The commitment output helper produces the reference output.
    let (script, value) = commitment_output(&hex32(COMMITMENT));
    assert_eq!(
        script.as_bytes(),
        decoded.output[1].script_pubkey.as_bytes()
    );
    assert_eq!(value, decoded.output[1].value.to_sat());
    assert_eq!(value, 0);
}

#[test]
fn witness_injection_matches_the_reference() {
    let full = coinbase_with_witness(&hex(COINBASE_STRIPPED), &hex32(RESERVED)).unwrap();
    assert_eq!(full, hex(COINBASE_FULL));
    // The txid is preserved (the coinbase txid miners folded into the
    // merkle root).
    use bitcoin::hashes::Hash;
    let decoded: bitcoin::Transaction = bitcoin::consensus::deserialize(&full).unwrap();
    assert_eq!(decoded.compute_txid().to_byte_array(), hex32(COINBASE_TXID));
    // The witness is exactly one 32-byte item.
    let items: Vec<&[u8]> = decoded.input[0].witness.iter().collect();
    assert_eq!(items, vec![hex(RESERVED)]);
}

#[test]
fn assemble_witness_reproduces_the_reference_block() {
    let b = assemble_witness(
        2,
        [0x11; 32],
        1_700_000_000,
        0x207f_ffff,
        7,
        &hex(COINBASE_STRIPPED),
        &hex32(RESERVED),
        &transactions_full(),
    )
    .expect("the frozen vectors assemble");
    let header: [u8; 80] = hex(HEADER).try_into().unwrap();
    assert_eq!(b.header, header);
    assert_eq!(b.block, hex(BLOCK));
    assert_eq!(b.merkle_root, hex32(MERKLE_ROOT));
    assert_eq!(b.witness_root, hex32(WITNESS_ROOT));
    assert_eq!(b.commitment, hex32(COMMITMENT));
    // The header hash (what the network target is checked against).
    assert_eq!(txid(&b.header), hex32(HEADER_HASH));
    // The merkle branch a miner folds the coinbase txid with.
    let branch = coinbase_merkle_branch(&TXIDS.iter().map(|s| hex32(s)).collect::<Vec<_>>());
    assert_eq!(
        branch,
        MERKLE_BRANCH.iter().map(|s| hex32(s)).collect::<Vec<_>>()
    );
    // The whole block decodes as a consensus block.
    let decoded: bitcoin::Block = bitcoin::consensus::deserialize(&b.block).unwrap();
    assert_eq!(decoded.txdata.len(), 5);
    assert_eq!(
        decoded.txdata[0].compute_txid().to_byte_array(),
        hex32(COINBASE_TXID)
    );
    // The AssembledBlock conversion keeps the wire bytes.
    let as_legacy: AssembledBlock = b.into();
    assert_eq!(as_legacy.block, hex(BLOCK));
}

/// BIP 141 weight accounting: 3 x stripped size + total size, both
/// including the 80-byte header and the transaction count. The commitment
/// output and the coinbase witness field are the overhead the front
/// charges the payout budget for.
#[test]
fn weights_match_the_reference() {
    let stripped_txs = transactions_stripped();
    let full_txs = transactions_full();
    let stripped_block =
        80 + 1 + hex(COINBASE_STRIPPED).len() + stripped_txs.iter().map(Vec::len).sum::<usize>();
    let full_block =
        80 + 1 + hex(COINBASE_FULL).len() + full_txs.iter().map(Vec::len).sum::<usize>();
    assert_eq!(hex(COINBASE_STRIPPED).len(), 186);
    assert_eq!(hex(COINBASE_FULL).len(), 222);
    assert_eq!(stripped_block, 633);
    assert_eq!(full_block, 956);
    assert_eq!(full_block, hex(BLOCK).len());
    // Block weight: 3 x 633 + 956 = 2855.
    assert_eq!(3 * stripped_block as u64 + full_block as u64, 2855);
    // Coinbase weight: 3 x 186 + 222 = 780.
    assert_eq!(3 * 186 + 222, 780);
    // Commitment output: 47 vbytes = 188 WU; coinbase witness field +
    // marker/flag: 36 WU; total charged overhead: 224 WU.
    assert_eq!(hex(COMMITMENT_SCRIPT).len(), 38);
    assert_eq!(8 + 1 + 38, 47);
    assert_eq!(witness::COMMITMENT_OUTPUT_WEIGHT_WU, 188);
    assert_eq!(witness::COINBASE_WITNESS_WEIGHT_WU, 36);
    assert_eq!(WITNESS_COMMITMENT_OVERHEAD_WU, 224);
    assert_eq!(hex(COINBASE_FULL).len() - hex(COINBASE_STRIPPED).len(), 36);
}

/// Negative vectors: the assembler must reject, never submit, a coinbase
/// whose commitment does not bind the actual transaction set.
#[test]
fn assemble_witness_rejects_wrong_reserved_and_missing_commitment() {
    let assemble = |coinbase: &[u8], reserved: &[u8; 32]| {
        assemble_witness(
            2,
            [0x11; 32],
            1_700_000_000,
            0x207f_ffff,
            7,
            coinbase,
            reserved,
            &transactions_full(),
        )
    };
    let stripped = hex(COINBASE_STRIPPED);

    // Wrong reserved value: the coinbase commits to something else.
    let wrong_reserved = [0x00u8; 32];
    assert!(matches!(
        assemble(&stripped, &wrong_reserved),
        Err(BlockError::CommitmentMismatch { .. })
    ));

    // Missing commitment output: rebuild the coinbase without output 1.
    let mut decoded: bitcoin::Transaction = bitcoin::consensus::deserialize(&stripped).unwrap();
    decoded.output.truncate(1);
    let no_commitment = bitcoin::consensus::serialize(&decoded);
    assert!(matches!(
        assemble(&no_commitment, &hex32(RESERVED)),
        Err(BlockError::MissingCommitment)
    ));

    // Duplicate commitment output.
    let mut twice: bitcoin::Transaction = bitcoin::consensus::deserialize(&stripped).unwrap();
    twice.output.push(twice.output[1].clone());
    let duplicate = bitcoin::consensus::serialize(&twice);
    assert!(matches!(
        assemble(&duplicate, &hex32(RESERVED)),
        Err(BlockError::Witness(
            WitnessError::DuplicateCommitment { .. }
        ))
    ));

    // Malformed commitment shape: the header with a 31-byte tail.
    let mut malformed: bitcoin::Transaction = bitcoin::consensus::deserialize(&stripped).unwrap();
    let mut script = witness::WITNESS_COMMITMENT_HEADER.to_vec();
    script.extend_from_slice(&[0u8; 31]);
    malformed.output[1].script_pubkey = bitcoin::ScriptBuf::from_bytes(script);
    let malformed = bitcoin::consensus::serialize(&malformed);
    assert!(matches!(
        assemble(&malformed, &hex32(RESERVED)),
        Err(BlockError::Witness(
            WitnessError::MalformedCommitment { .. }
        ))
    ));

    // Non-zero-value commitment output.
    let mut valued: bitcoin::Transaction = bitcoin::consensus::deserialize(&stripped).unwrap();
    valued.output[1].value = bitcoin::Amount::from_sat(1000);
    let valued = bitcoin::consensus::serialize(&valued);
    assert!(matches!(
        assemble(&valued, &hex32(RESERVED)),
        Err(BlockError::CommitmentValue { .. })
    ));

    // A transaction that does not strictly decode.
    let mut bad_txs = transactions_full();
    bad_txs[2].push(0x00);
    let err = assemble_witness(
        2,
        [0x11; 32],
        1_700_000_000,
        0x207f_ffff,
        7,
        &stripped,
        &hex32(RESERVED),
        &bad_txs,
    )
    .unwrap_err();
    assert!(
        matches!(err, BlockError::TransactionDecode { index: 2, .. }),
        "{err}"
    );
}

/// The txid merkle tree must hash the *stripped* serializations. Hashing
/// the full serializations instead would produce the wtxids and a
/// different merkle root - this vector proves the frozen values would
/// catch that bug.
#[test]
fn the_merkle_root_uses_the_stripped_serializations() {
    let full_txs = transactions_full();
    // sha256d over the full serialization is the wtxid...
    assert_eq!(txid(&full_txs[1]), hex32(WTXIDS[1]));
    assert_eq!(txid(&full_txs[2]), hex32(WTXIDS[2]));
    assert_ne!(txid(&full_txs[1]), hex32(TXIDS[1]));
    // ... so a merkle tree over full-byte hashes is not the block's
    // merkle root.
    let full_ids: Vec<[u8; 32]> = full_txs.iter().map(|t| txid(t)).collect();
    let mut leaves = vec![hex32(COINBASE_TXID)];
    leaves.extend(full_ids);
    assert_ne!(merkle_root(&leaves), hex32(MERKLE_ROOT));
    // The stripped serializations do reproduce it.
    let mut leaves = vec![hex32(COINBASE_TXID)];
    leaves.extend(frozen_set().txids);
    assert_eq!(merkle_root(&leaves), hex32(MERKLE_ROOT));
    // Each frozen txid is the hash of its frozen stripped serialization.
    for (i, stripped) in transactions_stripped().iter().enumerate() {
        assert_eq!(txid(stripped), hex32(TXIDS[i]));
    }
}

/// Witness malleation: the same txid with a different witness is a
/// different wtxid, a different witness root and a different commitment -
/// but the txid merkle root is unchanged. This is why the front keys the
/// job on the commitment, not on the txid merkle root alone.
#[test]
fn witness_malleation_changes_the_commitment_but_not_the_txids() {
    let mut malleated = transactions_full();
    malleated[2] = hex(TR1_MALLEATED);
    let set = analyze_transactions(&malleated).expect("the malleated tx decodes");

    // Same txids...
    assert_eq!(set.txids, frozen_set().txids);
    // ...different wtxid at the swapped position.
    assert_ne!(set.wtxids[2], hex32(WTXIDS[2]));
    // A different witness root, and thus a different commitment under the
    // same reserved value.
    let root = witness::witness_root(&set.wtxids);
    assert_ne!(root, hex32(WITNESS_ROOT));
    let commitment = witness::witness_commitment(&root, &hex32(RESERVED));
    assert_ne!(commitment, hex32(COMMITMENT));

    // The assembled block carries the malleated transaction and commits
    // to the new witness root: rebuild the coinbase for it.
    let new_script = witness::commitment_script(&commitment);
    let mut decoded: bitcoin::Transaction =
        bitcoin::consensus::deserialize(&hex(COINBASE_STRIPPED)).unwrap();
    decoded.output[1].script_pubkey = bitcoin::ScriptBuf::from_bytes(new_script);
    let coinbase = bitcoin::consensus::serialize(&decoded);
    let b = assemble_witness(
        2,
        [0x11; 32],
        1_700_000_000,
        0x207f_ffff,
        7,
        &coinbase,
        &hex32(RESERVED),
        &malleated,
    )
    .expect("the malleated set assembles under the new commitment");
    assert_eq!(b.commitment, commitment);
    assert_eq!(b.witness_root, root);
    // The non-coinbase txids - and therefore the merkle *branch* a miner
    // folds the coinbase txid with - are untouched by the malleation. The
    // merkle *root* moves with the new coinbase (the commitment output is
    // part of the stripped serialization), which is exactly why the job
    // must be republished.
    assert_eq!(
        coinbase_merkle_branch(&set.txids),
        MERKLE_BRANCH.iter().map(|s| hex32(s)).collect::<Vec<_>>()
    );
    let mut leaves = vec![txid(&coinbase)];
    leaves.extend(set.txids.iter().copied());
    assert_eq!(b.merkle_root, merkle_root(&leaves));
    assert_ne!(b.merkle_root, hex32(MERKLE_ROOT));
    // ... and the old commitment no longer binds this set.
    assert!(matches!(
        assemble_witness(
            2,
            [0x11; 32],
            1_700_000_000,
            0x207f_ffff,
            7,
            &hex(COINBASE_STRIPPED),
            &hex32(RESERVED),
            &malleated,
        ),
        Err(BlockError::CommitmentMismatch { .. })
    ));
}
