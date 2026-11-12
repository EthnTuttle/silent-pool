//! Block assembly and merkle roots. Pure functions used by the front when a
//! share meets the network target.

use thiserror::Error;

use crate::witness::{self, WitnessError};

/// A block header plus the transactions that follow it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssembledBlock {
    pub header: [u8; 80],
    pub block: Vec<u8>,
    pub merkle_root: [u8; 32],
}

/// A block assembled with a BIP 141 witness commitment (transaction sets
/// that may carry witness data). The wire bytes are a superset of
/// [`AssembledBlock`]; the extra roots are for diagnostics and tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssembledWitnessBlock {
    pub header: [u8; 80],
    pub block: Vec<u8>,
    pub merkle_root: [u8; 32],
    /// The witness merkle root the commitment commits to (BIP 141).
    pub witness_root: [u8; 32],
    /// The commitment bytes carried by the coinbase's commitment output.
    pub commitment: [u8; 32],
}

impl From<AssembledWitnessBlock> for AssembledBlock {
    fn from(b: AssembledWitnessBlock) -> AssembledBlock {
        AssembledBlock {
            header: b.header,
            block: b.block,
            merkle_root: b.merkle_root,
        }
    }
}

/// A witness-commitment block could not be assembled. Every variant is a
/// fail-closed rejection: nothing is submitted to the node.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum BlockError {
    #[error("coinbase does not strictly decode: {0}")]
    CoinbaseDecode(String),
    #[error("transaction {index} does not strictly decode: {reason}")]
    TransactionDecode { index: usize, reason: String },
    #[error("witness injection changed the coinbase txid")]
    CoinbaseTxidChanged,
    #[error("witness: {0}")]
    Witness(#[from] WitnessError),
    #[error("coinbase carries no witness commitment output")]
    MissingCommitment,
    #[error("commitment output at index {index} is not zero-value: {value} sats")]
    CommitmentValue { index: usize, value: u64 },
    #[error(
        "commitment mismatch: coinbase carries {found:?}, recomputed {expected:?} from the transaction set"
    )]
    CommitmentMismatch { found: [u8; 32], expected: [u8; 32] },
    #[error("actual coinbase or block weight exceeds the consensus/pool ceiling")]
    Weight,
}

/// Exact serialized transaction weights, including CompactSize counts and
/// the 36-WU injected coinbase witness. Tier-budget estimates are insufficient.
pub fn validate_template_weight(
    stripped_coinbase: &[u8],
    transactions: &[Vec<u8>],
    witness: bool,
) -> Result<(), BlockError> {
    let cb = crate::template::decode_transaction_strict_reason(stripped_coinbase)
        .map_err(BlockError::CoinbaseDecode)?;
    let coinbase_weight = cb.weight().to_wu() + if witness { 36 } else { 0 };
    if coinbase_weight > 1_000_000 {
        return Err(BlockError::Weight);
    }
    let n = transactions
        .len()
        .checked_add(1)
        .ok_or(BlockError::Weight)?;
    let count_bytes = bitcoin::consensus::serialize(&bitcoin::VarInt(n as u64)).len() as u64;
    let mut weight = 320 + 4 * count_bytes + coinbase_weight;
    for (index, bytes) in transactions.iter().enumerate() {
        let tx = crate::template::decode_transaction_strict_reason(bytes)
            .map_err(|reason| BlockError::TransactionDecode { index, reason })?;
        weight = weight
            .checked_add(tx.weight().to_wu())
            .ok_or(BlockError::Weight)?;
    }
    if weight > 4_000_000 {
        return Err(BlockError::Weight);
    }
    Ok(())
}

/// `sha256d`.
pub fn txid(tx: &[u8]) -> [u8; 32] {
    use bitcoin::hashes::{sha256d, Hash};
    sha256d::Hash::hash(tx).to_byte_array()
}

/// `sha256d` of the concatenation, the inner merkle node.
fn hash_pair(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let mut buf = [0u8; 64];
    buf[..32].copy_from_slice(a);
    buf[32..].copy_from_slice(b);
    use bitcoin::hashes::{sha256d, Hash};
    sha256d::Hash::hash(&buf).to_byte_array()
}

/// Bitcoin-style merkle root; duplicate the last node on an odd level.
pub fn merkle_root(txids: &[[u8; 32]]) -> [u8; 32] {
    if txids.is_empty() {
        return [0u8; 32];
    }
    let mut level = txids.to_vec();
    while level.len() > 1 {
        if !level.len().is_multiple_of(2) {
            let last = *level.last().expect("non-empty");
            level.push(last);
        }
        level = level
            .chunks_exact(2)
            .map(|pair| hash_pair(&pair[0], &pair[1]))
            .collect();
    }
    level[0]
}

/// Merkle branch for the coinbase (leaf 0) over
/// `leaves = [coinbase] ++ txids`, in SV1/SV2 wire order: deepest sibling
/// first, so a miner reconstructs the root with
/// `root = H(coinbase_txid); for b in branch { root = H(root || b) }`.
///
/// The branch never depends on the coinbase txid, only on the non-coinbase
/// txs, which is why it can be built before the miner rolls the extranonce.
pub fn coinbase_merkle_branch(txids: &[[u8; 32]]) -> Vec<[u8; 32]> {
    if txids.is_empty() {
        return Vec::new();
    }
    let mut level = Vec::with_capacity(txids.len() + 1);
    // Placeholder for the coinbase: index 0 is never anyone's sibling, so its
    // value never reaches the path.
    level.push([0u8; 32]);
    level.extend_from_slice(txids);
    let mut branch = Vec::new();
    while level.len() > 1 {
        if !level.len().is_multiple_of(2) {
            let last = *level.last().expect("non-empty");
            level.push(last);
        }
        // The node we follow is always index 0, so its sibling is index 1.
        branch.push(level[1]);
        level = level
            .chunks_exact(2)
            .map(|pair| hash_pair(&pair[0], &pair[1]))
            .collect();
    }
    branch
}

/// Assemble a block. `version` is the version the miner actually hashed (it may
/// be rolled within the BIP320 mask); `coinbase` is the fully serialized
/// coinbase (no witness); `transactions` are the non-coinbase txs in template
/// order (witness-free; transaction sets with witness data go through
/// [`assemble_witness`]).
pub fn assemble(
    version: u32,
    prev_hash: [u8; 32],
    time: u32,
    bits: u32,
    nonce: u32,
    coinbase: &[u8],
    transactions: &[Vec<u8>],
) -> AssembledBlock {
    let mut ids = Vec::with_capacity(transactions.len() + 1);
    ids.push(txid(coinbase));
    ids.extend(transactions.iter().map(|tx| txid(tx)));
    let merkle_root = merkle_root(&ids);

    let mut header = [0u8; 80];
    header[..4].copy_from_slice(&version.to_le_bytes());
    header[4..36].copy_from_slice(&prev_hash);
    header[36..68].copy_from_slice(&merkle_root);
    header[68..72].copy_from_slice(&time.to_le_bytes());
    header[72..76].copy_from_slice(&bits.to_le_bytes());
    header[76..80].copy_from_slice(&nonce.to_le_bytes());

    let mut block = Vec::with_capacity(
        80 + 9 + coinbase.len() + transactions.iter().map(Vec::len).sum::<usize>(),
    );
    block.extend_from_slice(&header);
    write_compact_size(&mut block, (transactions.len() + 1) as u64);
    // Bitcoin block serialization is a raw transaction list, not a list of
    // length-prefixed blobs.
    block.extend_from_slice(coinbase);
    for tx in transactions {
        block.extend_from_slice(tx);
    }

    AssembledBlock {
        header,
        block,
        merkle_root,
    }
}

/// Inject the BIP 141 witness field into a witness-free coinbase: the
/// segwit marker and flag after the version, and one 32-byte witness item
/// (the witness reserved value) right before the trailing 4-byte locktime.
/// The txid is unchanged - witness data is excluded from the stripped hash -
/// which is what lets miners hash the coinbase before the witness exists.
///
/// `stripped` must be a canonically-encoded, witness-free coinbase (strict
/// decode plus a byte-exact roundtrip); anything else fails closed. The
/// result is itself strict-decoded and checked to keep the txid.
pub fn coinbase_with_witness(stripped: &[u8], reserved: &[u8; 32]) -> Result<Vec<u8>, BlockError> {
    let tx = crate::template::decode_transaction_strict_reason(stripped)
        .map_err(BlockError::CoinbaseDecode)?;
    if tx.input.iter().any(|input| !input.witness.is_empty()) {
        return Err(BlockError::CoinbaseDecode(
            "coinbase already carries witness data".into(),
        ));
    }
    let mut full = Vec::with_capacity(stripped.len() + 36);
    // version ‖ marker ‖ flag
    full.extend_from_slice(&stripped[..4]);
    full.extend_from_slice(&[0x00, 0x01]);
    // inputs and outputs, up to (but excluding) the trailing locktime
    full.extend_from_slice(&stripped[4..stripped.len() - 4]);
    // one witness item: the 32-byte reserved value
    full.push(0x01);
    full.push(0x20);
    full.extend_from_slice(reserved);
    // locktime
    full.extend_from_slice(&stripped[stripped.len() - 4..]);
    // Self-check: canonical encoding and unchanged txid.
    let decoded = crate::template::decode_transaction_strict_reason(&full)
        .map_err(BlockError::CoinbaseDecode)?;
    use bitcoin::hashes::Hash;
    if decoded.compute_txid().to_byte_array() != tx.compute_txid().to_byte_array() {
        return Err(BlockError::CoinbaseTxidChanged);
    }
    Ok(full)
}

/// Assemble a block whose transaction set may carry witness data (BIP 141).
/// Like [`assemble`], `coinbase` is the fully serialized *stripped* coinbase
/// the miner hashed (commitment output included, witness absent); the
/// witness field is injected here from `reserved`. `transactions` are the
/// full serializations, in template order.
///
/// Fail-closed checks before the block is returned:
///
/// - every transaction strictly decodes (no trailing bytes, no ambiguous
///   marker/flag encoding - otherwise there is no unambiguous txid/wtxid);
/// - the coinbase carries exactly one well-formed commitment output
///   ([`witness::coinbase_commitment`] rejects duplicates and malformed
///   shapes) and it is zero-value;
/// - that output's commitment equals `SHA256d(witness_root ‖ reserved)`
///   recomputed from the *actual* transaction set - a coinbase committing
///   to anything else is rejected, never submitted;
/// - the injected witness leaves the coinbase txid unchanged.
#[allow(clippy::too_many_arguments)]
pub fn assemble_witness(
    version: u32,
    prev_hash: [u8; 32],
    time: u32,
    bits: u32,
    nonce: u32,
    stripped_coinbase: &[u8],
    reserved: &[u8; 32],
    transactions: &[Vec<u8>],
) -> Result<AssembledWitnessBlock, BlockError> {
    use bitcoin::hashes::Hash;
    let mut txids = Vec::with_capacity(transactions.len() + 1);
    let mut wtxids = Vec::with_capacity(transactions.len());
    for (index, raw) in transactions.iter().enumerate() {
        let tx = crate::template::decode_transaction_strict_reason(raw)
            .map_err(|reason| BlockError::TransactionDecode { index, reason })?;
        txids.push(tx.compute_txid().to_byte_array());
        wtxids.push(tx.compute_wtxid().to_byte_array());
    }

    let coinbase_tx = crate::template::decode_transaction_strict_reason(stripped_coinbase)
        .map_err(BlockError::CoinbaseDecode)?;
    let (index, found) = witness::coinbase_commitment(
        coinbase_tx
            .output
            .iter()
            .map(|o| o.script_pubkey.as_bytes()),
    )?
    .ok_or(BlockError::MissingCommitment)?;
    let value = coinbase_tx.output[index].value.to_sat();
    if value != 0 {
        return Err(BlockError::CommitmentValue { index, value });
    }

    let coinbase = coinbase_with_witness(stripped_coinbase, reserved)?;
    txids.insert(0, coinbase_tx.compute_txid().to_byte_array());
    let root = merkle_root(&txids);

    let witness_root = witness::witness_root(&wtxids);
    let commitment = witness::witness_commitment(&witness_root, reserved);
    if found != commitment {
        return Err(BlockError::CommitmentMismatch {
            found,
            expected: commitment,
        });
    }

    let mut header = [0u8; 80];
    header[..4].copy_from_slice(&version.to_le_bytes());
    header[4..36].copy_from_slice(&prev_hash);
    header[36..68].copy_from_slice(&root);
    header[68..72].copy_from_slice(&time.to_le_bytes());
    header[72..76].copy_from_slice(&bits.to_le_bytes());
    header[76..80].copy_from_slice(&nonce.to_le_bytes());

    let mut block = Vec::with_capacity(
        80 + 9 + coinbase.len() + transactions.iter().map(Vec::len).sum::<usize>(),
    );
    block.extend_from_slice(&header);
    write_compact_size(&mut block, (transactions.len() + 1) as u64);
    block.extend_from_slice(&coinbase);
    for tx in transactions {
        block.extend_from_slice(tx);
    }

    Ok(AssembledWitnessBlock {
        header,
        block,
        merkle_root: root,
        witness_root,
        commitment,
    })
}

fn write_compact_size(out: &mut Vec<u8>, n: u64) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merkle_root_matches_a_known_shape() {
        let a = [1u8; 32];
        let b = [2u8; 32];
        let c = [3u8; 32];
        // Two leaves: H(a || b).
        let ab = {
            let mut buf = [0u8; 64];
            buf[..32].copy_from_slice(&a);
            buf[32..].copy_from_slice(&b);
            use bitcoin::hashes::{sha256d, Hash};
            sha256d::Hash::hash(&buf).to_byte_array()
        };
        assert_eq!(merkle_root(&[a, b]), ab);
        // Three leaves duplicate c: H(H(a||b) || H(c||c)).
        let cc = {
            let mut buf = [0u8; 64];
            buf[..32].copy_from_slice(&c);
            buf[32..].copy_from_slice(&c);
            use bitcoin::hashes::{sha256d, Hash};
            sha256d::Hash::hash(&buf).to_byte_array()
        };
        let root = {
            let mut buf = [0u8; 64];
            buf[..32].copy_from_slice(&ab);
            buf[32..].copy_from_slice(&cc);
            use bitcoin::hashes::{sha256d, Hash};
            sha256d::Hash::hash(&buf).to_byte_array()
        };
        assert_eq!(merkle_root(&[a, b, c]), root);
    }

    #[test]
    fn assembles_a_header_and_transaction_count() {
        let coinbase = vec![0xaa; 100];
        let b = assemble(2, [0x11; 32], 1_700_000_000, 0x1d00_ffff, 7, &coinbase, &[]);
        assert_eq!(b.block.len(), 80 + 1 + 100);
        assert_eq!(&b.block[..4], &2u32.to_le_bytes());
        assert_eq!(&b.block[4..36], &[0x11; 32]);
        assert_eq!(&b.block[36..68], &b.merkle_root);
        assert_eq!(&b.block[68..72], &1_700_000_000u32.to_le_bytes());
        assert_eq!(&b.block[72..76], &0x1d00_ffffu32.to_le_bytes());
        assert_eq!(&b.block[76..80], &7u32.to_le_bytes());
        assert_eq!(b.block[80], 1); // one tx
        assert_eq!(&b.block[81..], &coinbase);
    }

    #[test]
    fn empty_merkle_root_is_zero() {
        assert_eq!(merkle_root(&[]), [0u8; 32]);
    }

    /// The branch a miner gets must rebuild the block's merkle root for any
    /// number of non-coinbase transactions, using the same fold the SV1/SV2
    /// listeners use.
    #[test]
    fn coinbase_branch_rebuilds_the_block_root() {
        let coinbase_txid = [0xc0u8; 32];
        for txids in [
            vec![[1u8; 32]],
            vec![[1u8; 32], [2u8; 32]],
            vec![[1u8; 32], [2u8; 32], [3u8; 32]],
            vec![[1u8; 32], [2u8; 32], [3u8; 32], [4u8; 32], [5u8; 32]],
        ] {
            let branch = coinbase_merkle_branch(&txids);
            let mut all = vec![coinbase_txid];
            all.extend_from_slice(&txids);
            let expected = merkle_root(&all);

            // Miner-side fold: coinbase txid, then the branch bottom-up.
            let mut root = coinbase_txid;
            for sibling in &branch {
                root = hash_pair(&root, sibling);
            }
            assert_eq!(root, expected, "{} txs", txids.len());
        }
    }

    #[test]
    fn coinbase_branch_is_deepest_first_upstream_vector() {
        let coinbase = [0xc0u8; 32];
        let txids = vec![[1u8; 32], [2u8; 32], [3u8; 32]];
        let deepest = txids[0];
        let next = hash_pair(&txids[1], &txids[2]);
        let branch = coinbase_merkle_branch(&txids);

        assert_eq!(branch, vec![deepest, next]);
        let expected = hash_pair(&hash_pair(&coinbase, &deepest), &next);
        assert_eq!(
            channels_sv2::merkle_root::merkle_root_from_path_(coinbase, &branch),
            expected,
            "upstream channels_sv2 consumes the path deepest first"
        );
    }

    #[test]
    fn assemble_uses_the_submitted_version() {
        let coinbase = vec![0xaa; 10];
        let rolled = 0x2000_0000u32 ^ 0x0000_2000;
        let b = assemble(rolled, [0x11; 32], 1, 0x207f_ffff, 3, &coinbase, &[]);
        assert_eq!(&b.block[..4], &rolled.to_le_bytes());
    }

    /// R8 (Fable rework 2026-11-08): the 224 WU witness overhead covers the
    /// commitment output and the coinbase witness field, but not the coinbase
    /// output-count CompactSize crossing the 252->253 boundary - the count
    /// grows from one byte to `0xfd` + 2 little-endian bytes, i.e. +2
    /// vbytes = +8 WU. This boundary vector pins the documented model limit
    /// of the preexisting whole-block weight approximation (the tier ladder
    /// governs pool-controlled payout outputs); it is deliberately not a
    /// redesigned hard weight limit.
    #[test]
    fn compact_size_growth_at_the_253rd_output_is_8_wu() {
        let many = |n: usize| {
            let outputs: Vec<bitcoin::TxOut> = (0..n).map(|_| txout(Vec::new(), 1000)).collect();
            coinbase_with_outputs(&outputs)
        };
        let at_252 = many(252);
        let at_253 = many(253);
        // Each empty-script output is 9 bytes (8 value + 1 script length).
        // version(4) + in-count(1) + one input(42) = 47 bytes before the
        // output count.
        let count_at = 4 + 1 + 42;
        // 252 outputs still encode the count in a single byte...
        assert_eq!(at_252[count_at], 252);
        // ...while the 253rd forces the three-byte `0xfd` form.
        assert_eq!(&at_253[count_at..count_at + 3], &[0xfd, 0xfd, 0x00]);
        // One more 9-byte output, plus the 2-byte CompactSize growth: the
        // +2 vbytes = +8 WU the 224 WU overhead does not account for.
        assert_eq!(at_253.len(), at_252.len() + 9 + 2);
        // Both shapes remain strictly canonical serializations.
        assert!(crate::template::decode_transaction_strict(&at_252).is_ok());
        assert!(crate::template::decode_transaction_strict(&at_253).is_ok());
    }

    fn txout(script: Vec<u8>, value: u64) -> bitcoin::TxOut {
        bitcoin::TxOut {
            value: bitcoin::Amount::from_sat(value),
            script_pubkey: bitcoin::ScriptBuf::from_bytes(script),
        }
    }

    /// A witness-free coinbase with the given outputs.
    fn coinbase_with_outputs(outputs: &[bitcoin::TxOut]) -> Vec<u8> {
        use bitcoin::absolute::LockTime;
        use bitcoin::transaction::Version;
        use bitcoin::{OutPoint, ScriptBuf, Sequence, Transaction, TxIn, Witness};
        let tx = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::null(),
                script_sig: ScriptBuf::from_bytes(vec![0x51]),
                sequence: Sequence::MAX,
                witness: Witness::default(),
            }],
            output: outputs.to_vec(),
        };
        bitcoin::consensus::serialize(&tx)
    }

    /// A witness-free coinbase with a pool output and an optional
    /// commitment output (last).
    fn stripped_coinbase(commitment: Option<(Vec<u8>, u64)>) -> Vec<u8> {
        let mut outputs = vec![txout(Vec::new(), 1000)];
        if let Some((script, value)) = commitment {
            outputs.push(txout(script, value));
        }
        coinbase_with_outputs(&outputs)
    }

    fn witness_tx(seed: u8) -> Vec<u8> {
        use bitcoin::absolute::LockTime;
        use bitcoin::hashes::Hash;
        use bitcoin::transaction::Version;
        use bitcoin::{Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness};
        let tx = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint {
                    txid: bitcoin::Txid::from_byte_array([seed; 32]),
                    vout: u32::from(seed),
                },
                script_sig: ScriptBuf::new(),
                sequence: Sequence::MAX,
                witness: Witness::from_slice(&[vec![seed; 4]]),
            }],
            output: vec![TxOut {
                value: Amount::from_sat(1),
                script_pubkey: ScriptBuf::new(),
            }],
        };
        bitcoin::consensus::serialize(&tx)
    }

    #[test]
    fn witness_injection_splices_marker_flag_and_reserved() {
        let stripped = stripped_coinbase(None);
        let reserved = [0x42u8; 32];
        let full = coinbase_with_witness(&stripped, &reserved).unwrap();
        // 2 marker/flag bytes + 34 witness-field bytes.
        assert_eq!(full.len(), stripped.len() + 36);
        assert_eq!(&full[..4], &stripped[..4]); // version
        assert_eq!(&full[4..6], &[0x00, 0x01]); // marker, flag
                                                // inputs and outputs, excluding the trailing locktime
        assert_eq!(&full[6..full.len() - 38], &stripped[4..stripped.len() - 4]);
        // one 32-byte witness item...
        assert_eq!(full[full.len() - 38], 0x01);
        assert_eq!(full[full.len() - 37], 0x20);
        assert_eq!(&full[full.len() - 36..full.len() - 4], &reserved);
        // ...then the locktime, unchanged.
        assert_eq!(&full[full.len() - 4..], &stripped[stripped.len() - 4..]);
        // The txid is unchanged: the witness is not hashed. (`txid()` on raw
        // bytes would be the wtxid for the full serialization; the txid
        // comes from the decoded transaction.)
        let full_tx: bitcoin::Transaction = bitcoin::consensus::deserialize(&full).unwrap();
        let stripped_tx: bitcoin::Transaction = bitcoin::consensus::deserialize(&stripped).unwrap();
        assert_eq!(full_tx.compute_txid(), stripped_tx.compute_txid());
    }

    #[test]
    fn witness_injection_fails_closed_on_bad_coinbases() {
        let reserved = [0x42u8; 32];
        // A coinbase that already carries witness data.
        let with_witness = witness_tx(9);
        assert!(matches!(
            coinbase_with_witness(&with_witness, &reserved),
            Err(BlockError::CoinbaseDecode(_))
        ));
        // Trailing bytes.
        let mut trailing = stripped_coinbase(None);
        trailing.push(0x00);
        assert!(matches!(
            coinbase_with_witness(&trailing, &reserved),
            Err(BlockError::CoinbaseDecode(_))
        ));
    }

    #[test]
    fn assemble_witness_builds_a_checkable_block() {
        let txs = vec![witness_tx(1), witness_tx(2)];
        let set = crate::template::analyze_transactions(&txs).unwrap();
        let root = witness::witness_root(&set.wtxids);
        let reserved = [0x42u8; 32];
        let commitment = witness::witness_commitment(&root, &reserved);
        let coinbase = stripped_coinbase(Some((witness::commitment_script(&commitment), 0)));

        let b = assemble_witness(
            2,
            [0x11; 32],
            1_700_000_000,
            0x207f_ffff,
            7,
            &coinbase,
            &reserved,
            &txs,
        )
        .unwrap();
        // The merkle root folds the coinbase txid over the stripped txids.
        let mut ids = vec![txid(&coinbase)];
        ids.extend(set.txids);
        assert_eq!(b.merkle_root, merkle_root(&ids));
        assert_eq!(b.witness_root, root);
        assert_eq!(b.commitment, commitment);
        // The whole block decodes as a consensus block: the coinbase
        // carries the witness, the rest keep theirs.
        let block: bitcoin::Block = bitcoin::consensus::deserialize(&b.block).unwrap();
        assert_eq!(block.txdata.len(), 3);
        assert!(!block.txdata[0].input[0].witness.is_empty());
        let coinbase_witness: Vec<&[u8]> = block.txdata[0].input[0].witness.iter().collect();
        assert_eq!(coinbase_witness, vec![reserved.as_slice()]);
        assert!(block.txdata[1..]
            .iter()
            .all(|tx| !tx.input[0].witness.is_empty()));
        // ... and the header carries the recomputed merkle root.
        assert_eq!(&b.header[36..68], &b.merkle_root);
        let as_legacy: AssembledBlock = b.into();
        assert_eq!(as_legacy.block[..80], as_legacy.header);
    }

    #[test]
    fn assemble_witness_fails_closed_on_every_commitment_defect() {
        let txs = vec![witness_tx(1)];
        let set = crate::template::analyze_transactions(&txs).unwrap();
        let root = witness::witness_root(&set.wtxids);
        let reserved = [0x42u8; 32];
        let commitment = witness::witness_commitment(&root, &reserved);
        let script = witness::commitment_script(&commitment);
        let assemble = |coinbase: &[u8], reserved: &[u8; 32]| {
            assemble_witness(2, [0x11; 32], 1, 0x207f_ffff, 7, coinbase, reserved, &txs)
        };

        // Missing commitment output.
        assert!(matches!(
            assemble(&stripped_coinbase(None), &reserved),
            Err(BlockError::MissingCommitment)
        ));
        // Wrong reserved value: the coinbase commits to something else.
        assert!(matches!(
            assemble(&stripped_coinbase(Some((script.clone(), 0))), &[0x43u8; 32]),
            Err(BlockError::CommitmentMismatch { .. })
        ));
        // A commitment that matches nothing in this transaction set.
        assert!(matches!(
            assemble(
                &stripped_coinbase(Some((witness::commitment_script(&[9u8; 32]), 0))),
                &reserved
            ),
            Err(BlockError::CommitmentMismatch { .. })
        ));
        // Duplicate commitment outputs: consensus takes the highest index
        // (BIP 141), but the pool's own-coinbase policy is stricter and
        // rejects the ambiguity outright.
        assert!(matches!(
            assemble(
                &coinbase_with_outputs(&[
                    txout(Vec::new(), 1000),
                    txout(script.clone(), 0),
                    txout(script.clone(), 0),
                ]),
                &reserved
            ),
            Err(BlockError::Witness(
                WitnessError::DuplicateCommitment { .. }
            ))
        ));
        // Malformed commitment shape: header with the wrong length.
        let mut malformed = witness::WITNESS_COMMITMENT_HEADER.to_vec();
        malformed.extend_from_slice(&[0u8; 31]);
        assert!(matches!(
            assemble(&stripped_coinbase(Some((malformed, 0))), &reserved),
            Err(BlockError::Witness(
                WitnessError::MalformedCommitment { .. }
            ))
        ));
        // Non-zero-value commitment output.
        assert!(matches!(
            assemble(&stripped_coinbase(Some((script, 1000))), &reserved),
            Err(BlockError::CommitmentValue { .. })
        ));
    }

    #[test]
    fn assemble_witness_fails_closed_on_bad_transactions() {
        let txs = vec![witness_tx(1)];
        let set = crate::template::analyze_transactions(&txs).unwrap();
        let root = witness::witness_root(&set.wtxids);
        let reserved = [0x42u8; 32];
        let commitment = witness::witness_commitment(&root, &reserved);
        let coinbase = stripped_coinbase(Some((witness::commitment_script(&commitment), 0)));
        // Trailing bytes on a non-coinbase transaction.
        let mut bad = vec![witness_tx(1), witness_tx(2)];
        bad[1].push(0x00);
        assert!(matches!(
            assemble_witness(2, [0x11; 32], 1, 0x207f_ffff, 7, &coinbase, &reserved, &bad),
            Err(BlockError::TransactionDecode { index: 1, .. })
        ));
    }
}
