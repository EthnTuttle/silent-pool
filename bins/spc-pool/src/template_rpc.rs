//! Bitcoin Core RPC template source (dev/regtest). The production path is
//! the Template Distribution Protocol; this source exists so the pool can be
//! run end-to-end against a plain `bitcoind` without the IPC patch.

use async_trait::async_trait;
use bitcoin::blockdata::block::Block;
use bitcoin::hashes::Hash;
use bitcoin::BlockHash;
use bitcoincore_rpc::{Auth, Client, RpcApi};

use crate::template::{Template, TemplateError, TemplateSource};

pub struct RpcTemplateSource {
    client: Client,
    coinbase_output_max_additional_size: u32,
}

impl RpcTemplateSource {
    pub fn new(
        url: &str,
        user: &str,
        pass: &str,
        coinbase_output_max_additional_size: u32,
    ) -> Result<Self, TemplateError> {
        let auth = if user.is_empty() {
            Auth::None
        } else {
            Auth::UserPass(user.to_string(), pass.to_string())
        };
        let client =
            Client::new(url, auth).map_err(|e| TemplateError::Unavailable(e.to_string()))?;
        Ok(Self {
            client,
            coinbase_output_max_additional_size,
        })
    }
}

#[async_trait]
impl TemplateSource for RpcTemplateSource {
    async fn get_template(&self) -> Result<Template, TemplateError> {
        let t = self
            .client
            .get_block_template(
                bitcoincore_rpc::json::GetBlockTemplateModes::Template,
                &[bitcoincore_rpc::json::GetBlockTemplateRules::SegWit],
                &[],
            )
            .map_err(|e| TemplateError::Unavailable(e.to_string()))?;
        let mut transactions = Vec::with_capacity(t.transactions.len());
        let mut kept_fees = 0u64;
        let mut total_fees = 0u64;
        let mut dropped_witness = 0usize;
        // A witness transaction needs a BIP 141 witness commitment output in
        // the coinbase, which this pool does not build yet. Excluding it keeps
        // the block consensus-valid: the template provider selects which
        // transactions to include, so leaving one out only lowers the
        // claimable fees.
        for tx in &t.transactions {
            total_fees = total_fees.saturating_add(tx.fee.to_sat());
            match crate::template::tx_has_witness(&tx.raw_tx) {
                Some(false) => {
                    kept_fees = kept_fees.saturating_add(tx.fee.to_sat());
                    transactions.push(tx.raw_tx.clone());
                }
                Some(true) => dropped_witness = dropped_witness.saturating_add(1),
                None => {
                    return Err(TemplateError::Unavailable(
                        "template transaction does not decode".into(),
                    ))
                }
            }
        }
        if dropped_witness > 0 {
            tracing::warn!(
                dropped = dropped_witness,
                "template contained witness transactions; excluded until the coinbase carries a witness commitment"
            );
        }
        // GBT reports `subsidy + fees` for the selected set; dropping a tx
        // removes exactly its fee from what the coinbase may claim.
        let subsidy = t.coinbase_value.to_sat().saturating_sub(total_fees);
        let coinbase_value = subsidy.saturating_add(kept_fees);
        let fees = kept_fees;
        let prev_hash: BlockHash = t.previous_block_hash;
        if t.bits.len() != 4 {
            return Err(TemplateError::Unavailable("bad bits length".into()));
        }
        let bits = u32::from_be_bytes([t.bits[0], t.bits[1], t.bits[2], t.bits[3]]);
        Ok(Template {
            height: u32::try_from(t.height)
                .map_err(|_| TemplateError::Unavailable("height".into()))?,
            prev_hash: prev_hash.to_byte_array(),
            version: t.version,
            bits,
            curtime: u32::try_from(t.current_time)
                .map_err(|_| TemplateError::Unavailable("curtime".into()))?,
            subsidy,
            fees,
            coinbase_value,
            transactions,
            merkle_branches: Vec::new(),
            coinbase_output_max_additional_size: self.coinbase_output_max_additional_size,
        })
    }

    async fn submit_block(&self, block: &[u8]) -> Result<(), TemplateError> {
        let block: Block = bitcoin::consensus::deserialize(block)
            .map_err(|e| TemplateError::Rejected(format!("block decode: {e}")))?;
        self.client
            .submit_block(&block)
            .map_err(|e| TemplateError::Rejected(e.to_string()))?;
        Ok(())
    }
}
