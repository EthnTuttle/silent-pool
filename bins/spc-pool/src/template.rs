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

/// `Some(true)` when any input of the serialized transaction carries witness
/// data, `Some(false)` when it is witness-free, `None` when it cannot be
/// decoded.
///
/// A witness transaction needs a witness commitment output in the coinbase
/// (BIP 141), which this pool does not build yet, so a template containing one
/// is refused rather than mined as an invalid block.
pub fn tx_has_witness(raw: &[u8]) -> Option<bool> {
    let tx: bitcoin::Transaction = bitcoin::consensus::deserialize(raw).ok()?;
    Some(tx.input.iter().any(|input| !input.witness.is_empty()))
}

/// `txid` (witness-stripped hash) of a serialized transaction.
pub fn txid_of(raw: &[u8]) -> Option<[u8; 32]> {
    use bitcoin::hashes::Hash;
    let tx: bitcoin::Transaction = bitcoin::consensus::deserialize(raw).ok()?;
    Some(tx.compute_txid().to_byte_array())
}

/// Validate the non-coinbase transactions of a template: every one must decode
/// and be witness-free. Returns the txids in template order.
pub fn template_txids(transactions: &[Vec<u8>]) -> Result<Vec<[u8; 32]>, TemplateError> {
    let mut txids = Vec::with_capacity(transactions.len());
    for (i, raw) in transactions.iter().enumerate() {
        match tx_has_witness(raw) {
            Some(false) => {}
            Some(true) => {
                return Err(TemplateError::Unavailable(format!(
                    "template transaction {i} carries witness data (no witness commitment support)"
                )))
            }
            None => {
                return Err(TemplateError::Unavailable(format!(
                    "template transaction {i} does not decode"
                )))
            }
        }
        let txid = txid_of(raw).ok_or_else(|| {
            TemplateError::Unavailable(format!("template transaction {i} has no txid"))
        })?;
        txids.push(txid);
    }
    Ok(txids)
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

    #[test]
    fn witness_transactions_are_refused() {
        assert_eq!(tx_has_witness(&raw_tx(false)), Some(false));
        assert_eq!(tx_has_witness(&raw_tx(true)), Some(true));
        assert_eq!(tx_has_witness(&[0x00, 0x01]), None);

        assert!(template_txids(&[raw_tx(false)]).is_ok());
        let err = template_txids(&[raw_tx(false), raw_tx(true)]).unwrap_err();
        assert!(err.to_string().contains("witness"), "{err}");
        assert!(template_txids(&[vec![0u8; 3]]).is_err());
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
