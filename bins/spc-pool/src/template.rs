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
