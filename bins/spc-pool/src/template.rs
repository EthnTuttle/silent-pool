//! Template source abstraction (SPEC §7.1). The front never builds a block
//! without a template: the source owns the network difficulty, the transaction
//! set and the coinbase value.

use async_trait::async_trait;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Template {
    pub height: u32,
    pub prev_hash: [u8; 32],
    pub version: u32,
    pub bits: u32,
    pub curtime: u32,
    /// Block subsidy, in satoshis.
    pub subsidy: u64,
    /// Selected transaction fees, in satoshis.
    pub fees: u64,
    /// Subsidy plus selected fees.
    pub coinbase_value: u64,
    /// Non-coinbase transactions in template order.
    pub transactions: Vec<Vec<u8>>,
    /// Merkle branches for the coinbase, if the source supplies them.
    pub merkle_branches: Vec<[u8; 32]>,
    /// The Template Provider's reservation for the coinbase's additional
    /// outputs (SPEC §4.8).
    pub coinbase_output_max_additional_size: u32,
}

#[derive(Debug, Error)]
pub enum TemplateError {
    #[error("template source unavailable: {0}")]
    Unavailable(String),
    #[error("template source rejected the block: {0}")]
    Rejected(String),
}

/// Strictly decode one full transaction: `consensus::deserialize` (which
/// already rejects trailing bytes) plus a byte-exact re-serialization
/// roundtrip.
///
/// The roundtrip rejects ambiguous encodings: a segwit marker/flag with an
/// all-empty witness decodes but re-serializes without the marker, and a
/// zero-input tx serialized without the marker re-serializes with it. A
/// zero-input tx carrying the marker/flag (the reviewer's raw vector
/// `020000000001000101000000000000000000000000`) decodes *and* roundtrips,
/// so it is rejected explicitly: it is not a consensus-valid transaction
/// (a tx needs at least one input), and its wtxid differs from its txid
/// while no input carries witness data - exactly the shape that must never
/// reach the txid/wtxid analysis. Exactly one byte string may represent a
/// decoded transaction here, and it must have inputs (fail closed).
pub fn decode_transaction_strict(raw: &[u8]) -> Result<bitcoin::Transaction, TemplateError> {
    decode_transaction_strict_reason(raw).map_err(TemplateError::Unavailable)
}

/// The strict-decode core with a bare reason string, shared with the block
/// assembler (which reports its own error type).
pub(crate) fn decode_transaction_strict_reason(raw: &[u8]) -> Result<bitcoin::Transaction, String> {
    let tx: bitcoin::Transaction = bitcoin::consensus::deserialize(raw)
        .map_err(|e| format!("transaction does not decode: {e}"))?;
    if tx.input.is_empty() {
        return Err("transaction has no inputs".into());
    }
    let canonical = bitcoin::consensus::serialize(&tx);
    if canonical != raw {
        return Err("transaction does not re-serialize byte-for-byte (ambiguous encoding)".into());
    }
    Ok(tx)
}

/// `txid` (witness-stripped hash) of a serialized transaction.
pub fn txid_of(raw: &[u8]) -> Option<[u8; 32]> {
    use bitcoin::hashes::Hash;
    let tx: bitcoin::Transaction = bitcoin::consensus::deserialize(raw).ok()?;
    Some(tx.compute_txid().to_byte_array())
}

/// The txid/wtxid analysis of a template's non-coinbase transactions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TransactionSet {
    /// Witness-stripped txids in template order: the txid merkle leaves
    /// (and what SV1/SV2 merkle branches fold).
    pub txids: Vec<[u8; 32]>,
    /// Full-serialization wtxids in template order: the BIP 141 witness
    /// merkle leaves. Equal to `txids` for witness-free transactions.
    pub wtxids: Vec<[u8; 32]>,
    /// How many transactions carry witness data.
    pub witness_count: usize,
}

/// Strictly decode every non-coinbase transaction and derive its txid (from
/// the witness-stripped serialization) and its wtxid (from the full
/// serialization, BIP 141). Witness transactions are allowed: the coinbase
/// carries the witness commitment (`crate::witness`).
pub fn analyze_transactions(transactions: &[Vec<u8>]) -> Result<TransactionSet, TemplateError> {
    use bitcoin::hashes::Hash;
    let mut set = TransactionSet {
        txids: Vec::with_capacity(transactions.len()),
        wtxids: Vec::with_capacity(transactions.len()),
        witness_count: 0,
    };
    for (i, raw) in transactions.iter().enumerate() {
        let tx = decode_transaction_strict(raw)
            .map_err(|e| TemplateError::Unavailable(format!("template transaction {i}: {e}")))?;
        set.txids.push(tx.compute_txid().to_byte_array());
        set.wtxids.push(tx.compute_wtxid().to_byte_array());
        if tx.input.iter().any(|input| !input.witness.is_empty()) {
            set.witness_count += 1;
        }
    }
    Ok(set)
}

/// Validate the non-coinbase transactions of a template (strict decode) and
/// return their txids in template order. Witness transactions are allowed;
/// the coinbase carries the BIP 141 commitment.
pub fn template_txids(transactions: &[Vec<u8>]) -> Result<Vec<[u8; 32]>, TemplateError> {
    Ok(analyze_transactions(transactions)?.txids)
}

#[async_trait]
pub trait TemplateSource: Send + Sync + 'static {
    async fn get_template(&self) -> Result<Template, TemplateError>;
    async fn submit_block(&self, block: &[u8]) -> Result<(), TemplateError>;
}

/// A deterministic source for tests and the regtest harness.
pub struct MockTemplateSource {
    template: Template,
    submitted: std::sync::Mutex<Vec<Vec<u8>>>,
}

impl MockTemplateSource {
    pub fn new(template: Template) -> Self {
        Self {
            template,
            submitted: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub fn submitted(&self) -> Vec<Vec<u8>> {
        self.submitted.lock().expect("poisoned").clone()
    }
}

#[async_trait]
impl TemplateSource for MockTemplateSource {
    async fn get_template(&self) -> Result<Template, TemplateError> {
        Ok(self.template.clone())
    }

    async fn submit_block(&self, block: &[u8]) -> Result<(), TemplateError> {
        self.submitted
            .lock()
            .map_err(|_| TemplateError::Unavailable("lock poisoned".into()))?
            .push(block.to_vec());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::absolute::LockTime;
    use bitcoin::hashes::Hash;
    use bitcoin::transaction::Version;
    use bitcoin::{Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness};

    fn raw_tx(witness: bool) -> Vec<u8> {
        let tx = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::null(),
                script_sig: ScriptBuf::new(),
                sequence: Sequence::MAX,
                witness: if witness {
                    Witness::from_slice(&[vec![0u8; 4]])
                } else {
                    Witness::default()
                },
            }],
            output: vec![TxOut {
                value: Amount::from_sat(1),
                script_pubkey: ScriptBuf::new(),
            }],
        };
        bitcoin::consensus::serialize(&tx)
    }

    /// A segwit marker/flag with an all-empty witness: decodes, but
    /// re-serializes without the marker, so it is ambiguous.
    fn marker_with_empty_witness(mut raw: Vec<u8>) -> Vec<u8> {
        raw.insert(4, 0x00); // marker
        raw.insert(5, 0x01); // flag
                             // One input: a witness field with zero items before the locktime.
        raw.insert(raw.len() - 4, 0x00);
        raw
    }

    #[test]
    fn strict_decode_accepts_witness_and_legacy_txs() {
        assert!(decode_transaction_strict(&raw_tx(false)).is_ok());
        assert!(decode_transaction_strict(&raw_tx(true)).is_ok());
    }

    #[test]
    fn strict_decode_fails_closed_on_every_ambiguity() {
        let raw = raw_tx(true);
        // Trailing bytes.
        let mut trailing = raw.clone();
        trailing.push(0x00);
        assert!(decode_transaction_strict(&trailing).is_err());
        // Truncated.
        assert!(decode_transaction_strict(&raw[..raw.len() - 1]).is_err());
        // Marker/flag with an all-empty witness.
        assert!(decode_transaction_strict(&marker_with_empty_witness(raw_tx(false))).is_err());
        // Garbage and empty blobs.
        assert!(decode_transaction_strict(&[0x00, 0x01]).is_err());
        assert!(decode_transaction_strict(&[]).is_err());
        // The reviewer's raw zero-input vector (Fable rework 2026-11-08):
        // version 2, marker/flag, 0 inputs, 1 output of 1 sat, locktime 0.
        // It decodes and roundtrips byte-for-byte, so only the explicit
        // no-input rejection catches it - a txid!=wtxid shape with no input
        // witness must never pass as a "legacy" transaction.
        const ZERO_INPUT: [u8; 21] = [
            0x02, 0x00, 0x00, 0x00, // version
            0x00, 0x01, // marker, flag
            0x00, // 0 inputs
            0x01, // 1 output
            0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // 1 sat
            0x00, // empty script
            0x00, 0x00, 0x00, 0x00, // locktime
        ];
        assert_eq!(
            hex::encode(ZERO_INPUT),
            "020000000001000101000000000000000000000000",
            "the raw vector is transcribed exactly"
        );
        assert!(decode_transaction_strict(&ZERO_INPUT).is_err());
        assert!(analyze_transactions(&[ZERO_INPUT.to_vec()]).is_err());
    }

    #[test]
    fn analyze_separates_txids_from_wtxids() {
        let legacy = raw_tx(false);
        let segwit = raw_tx(true);
        let set = analyze_transactions(&[legacy.clone(), segwit.clone()]).unwrap();
        assert_eq!(set.witness_count, 1);
        // The legacy tx's wtxid equals its txid (BIP 141).
        assert_eq!(set.txids[0], set.wtxids[0]);
        // The witness tx's wtxid differs from its txid...
        assert_ne!(set.txids[1], set.wtxids[1]);
        // ...while its txid still hashes the stripped serialization.
        let stripped: Transaction = {
            let mut tx: Transaction = bitcoin::consensus::deserialize(&segwit).unwrap();
            for input in &mut tx.input {
                input.witness = Witness::default();
            }
            tx
        };
        use bitcoin::hashes::Hash;
        assert_eq!(set.txids[1], stripped.compute_txid().to_byte_array());
    }

    #[test]
    fn analyze_reports_the_index_of_a_bad_transaction() {
        let err = analyze_transactions(&[raw_tx(false), vec![0u8; 3]]).unwrap_err();
        assert!(err.to_string().contains("transaction 1"), "{err}");
    }

    #[test]
    fn template_txids_still_returns_the_stripped_hashes() {
        let txs = vec![raw_tx(false), raw_tx(true), raw_tx(false)];
        assert_eq!(
            template_txids(&txs).unwrap(),
            analyze_transactions(&txs).unwrap().txids
        );
    }

    #[test]
    fn txids_match_the_witness_stripped_hash() {
        let raw = raw_tx(false);
        let tx: Transaction = bitcoin::consensus::deserialize(&raw).unwrap();
        assert_eq!(txid_of(&raw), Some(tx.compute_txid().to_byte_array()));
    }

    fn template() -> Template {
        Template {
            height: 800_000,
            prev_hash: [0x11; 32],
            version: 0x2000_0000,
            bits: 0x1d00_ffff,
            curtime: 1_700_000_000,
            subsidy: 312_500_000,
            fees: 0,
            coinbase_value: 312_500_000,
            transactions: vec![vec![0xaa; 10]],
            merkle_branches: vec![],
            coinbase_output_max_additional_size: 50_000,
        }
    }

    #[tokio::test]
    async fn mock_returns_and_records() {
        let src = MockTemplateSource::new(template());
        assert_eq!(src.get_template().await.unwrap().height, 800_000);
        src.submit_block(&[1, 2, 3]).await.unwrap();
        assert_eq!(src.submitted(), vec![vec![1, 2, 3]]);
    }
}
