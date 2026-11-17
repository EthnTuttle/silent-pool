//! Bitcoin Core RPC template source (dev/regtest). The production path is
//! the Template Distribution Protocol; this source exists so the pool can be
//! run end-to-end against a plain `bitcoind` without the IPC patch.

use async_trait::async_trait;
use bitcoin::blockdata::block::Block;
use bitcoin::hashes::Hash;
use bitcoin::BlockHash;
use bitcoincore_rpc::{Auth, Client, RpcApi};
use serde::Deserialize;

use crate::template::{BlockSubmitter, CoinbaseFields, Template, TemplateError, TemplateSource};

// The pinned RPC crate has closed rule enums predating consensuscleanup.
// Decode the actual result, without deleting unknown rules to satisfy that
// enum: required rules must be checked before any work is published.
#[derive(Deserialize)]
struct GbtTemplate {
    height: u32,
    previousblockhash: BlockHash,
    version: u32,
    bits: String,
    target: String,
    curtime: u32,
    mintime: u32,
    maxtime: Option<u32>,
    rules: Vec<String>,
    coinbasevalue: u64,
    transactions: Vec<GbtTransaction>,
}

#[derive(Deserialize)]
struct GbtTransaction {
    data: String,
    fee: u64,
}

fn gbt_request() -> serde_json::Value {
    serde_json::json!({"mode": "template", "rules": ["segwit", "consensuscleanup"]})
}

fn parse_template(value: serde_json::Value, reservation: u32) -> Result<Template, TemplateError> {
    let unavailable = TemplateError::Unavailable;
    let t: GbtTemplate = serde_json::from_value(value).map_err(|e| unavailable(e.to_string()))?;
    let coinbase = CoinbaseFields::from_gbt_rules(t.height, &t.rules)?;
    let bits_bytes = hex::decode(&t.bits).map_err(|e| unavailable(e.to_string()))?;
    let bits = u32::from_be_bytes(
        bits_bytes
            .try_into()
            .map_err(|_| unavailable("bad bits length".into()))?,
    );
    let target = spc_accounting::work_unit::decode_compact_target(bits)
        .map_err(|e| unavailable(e.to_string()))?;
    let stated_target = hex::decode(&t.target).map_err(|e| unavailable(e.to_string()))?;
    // Bind the RPC's target to the exact nBits that miners receive. The
    // checked compact decoder above rejects zero, negative and overflow.
    if stated_target != target.to_be_bytes() {
        return Err(unavailable(
            "GBT target/nbits mismatch or invalid target".into(),
        ));
    }
    let mut transactions = Vec::with_capacity(t.transactions.len());
    let mut fees = 0u64;
    for entry in t.transactions {
        fees = fees
            .checked_add(entry.fee)
            .ok_or_else(|| unavailable("GBT fee overflow".into()))?;
        let raw = hex::decode(entry.data).map_err(|e| unavailable(e.to_string()))?;
        let tx = crate::template::decode_transaction_strict(&raw)?;
        coinbase.validate_transaction(&tx)?;
        // The trusted full node owns prevout-dependent consensus checks,
        // including BIP54's legacy input sigops limit. Do not approximate it
        // using GBT's (different) aggregate sigops-cost field.
        transactions.push(raw);
    }
    let subsidy = t
        .coinbasevalue
        .checked_sub(fees)
        .ok_or_else(|| unavailable("GBT fees exceed coinbase value".into()))?;
    let template = Template {
        height: t.height,
        prev_hash: t.previousblockhash.to_byte_array(),
        version: t.version,
        bits,
        curtime: t.curtime,
        min_time: t.mintime,
        max_time: t.maxtime,
        coinbase,
        subsidy,
        fees,
        coinbase_value: t.coinbasevalue,
        transactions,
        merkle_branches: Vec::new(),
        coinbase_output_max_additional_size: reservation,
    };
    template.validate_context()?;
    Ok(template)
}

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
        let value = self
            .client
            .call("getblocktemplate", &[gbt_request()])
            .map_err(|e| TemplateError::Unavailable(e.to_string()))?;
        parse_template(value, self.coinbase_output_max_additional_size)
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

/// The RPC submission path is unchanged: `submitblock` with the assembled
/// block. Kept behind the submitter seam so `main.rs` treats every template
/// source uniformly.
#[async_trait]
impl BlockSubmitter for RpcTemplateSource {
    async fn submit_assembled(
        &self,
        block: &crate::block::AssembledBlock,
    ) -> Result<(), TemplateError> {
        TemplateSource::submit_block(self, &block.block).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Public BIP95 target, genesis parent and BIP54 active from height 1.
    // A synthetic GBT envelope, not a claim of observing a live node.
    fn gbt() -> serde_json::Value {
        serde_json::json!({
            "height": 1,
            "previousblockhash": "00000000000008c1637e7d3252dc1c333d4546fd6fff6a474b200d674a91d497",
            "version": 536870912, "bits": "1a0fffff",
            "target": "0000000000000fffff0000000000000000000000000000000000000000000000",
            "curtime": 1794632594u32, "mintime": 1794632594u32,
            "rules": ["csv", "!segwit", "taproot", "!consensuscleanup"],
            "coinbasevalue": 5000000000u64, "transactions": []
        })
    }

    #[test]
    fn negotiation_reaches_the_serialized_coinbase() {
        assert_eq!(
            gbt_request()["rules"],
            serde_json::json!(["segwit", "consensuscleanup"])
        );
        let t = parse_template(gbt(), 50_000).unwrap();
        let cb = crate::coinbase::try_build_pool_template_with_fields(
            2,
            t.height,
            &[0x42; 32],
            8,
            &[(crate::coinbase::p2tr_script(&[0x11; 32]), t.coinbase_value)],
            t.coinbase,
        )
        .unwrap();
        let tx: bitcoin::Transaction =
            bitcoin::consensus::deserialize(&cb.assemble(&[0; 8])).unwrap();
        assert_eq!(tx.input[0].sequence.0, 0xffff_fffe);
        assert_eq!(tx.lock_time.to_consensus_u32(), 0);
        assert_eq!(t.bits, 0x1a0fffff);
    }

    #[test]
    fn rules_fail_closed_and_legacy_is_explicit() {
        for rule in [
            "!future-softfork",
            "!signet",
            "signet",
            "!!consensuscleanup",
        ] {
            let mut value = gbt();
            value["rules"].as_array_mut().unwrap().push(rule.into());
            assert!(parse_template(value, 50_000).is_err(), "{rule}");
        }
        let mut value = gbt();
        value["rules"] = serde_json::json!([
            "csv",
            "!segwit",
            "taproot",
            "testdummy",
            "optional-future-rule"
        ]);
        assert_eq!(
            parse_template(value.clone(), 50_000).unwrap().coinbase,
            CoinbaseFields::legacy(0)
        );
        value.as_object_mut().unwrap().remove("rules");
        assert!(parse_template(value, 50_000).is_err());
    }

    #[test]
    fn rejects_bip54_size_and_malformed_fee_totals_at_rpc_admission() {
        let vectors: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/bip54-txsize.json")).unwrap();
        let mut value = gbt();
        value["transactions"] = serde_json::json!([{"data": vectors[2]["tx"], "fee": 0}]);
        assert!(parse_template(value.clone(), 50_000).is_err());
        value["rules"] = serde_json::json!(["!segwit"]);
        assert!(parse_template(value.clone(), 50_000).is_ok());
        value["transactions"][0]["fee"] = 5_000_000_001u64.into();
        assert!(parse_template(value.clone(), 50_000).is_err());
        value["coinbasevalue"] = u64::MAX.into();
        value["transactions"] = serde_json::json!([
            {"data": vectors[0]["tx"], "fee": u64::MAX},
            {"data": vectors[0]["tx"], "fee": 1}
        ]);
        assert!(parse_template(value, 50_000).is_err());
    }

    #[test]
    fn node_time_bounds_and_target_are_not_discarded() {
        // GBT mintime is node-derived, including the BIP54 boundaries at
        // N % 2016 == 0/2015. It is not reconstructed from a local clock.
        for height in [2015, 2016, 4031, 4032] {
            let mut value = gbt();
            value["height"] = height.into();
            value["maxtime"] = value["curtime"].clone();
            let t = parse_template(value.clone(), 50_000).unwrap();
            assert!(t.time_allowed(t.min_time));
            assert!(!t.time_allowed(t.min_time - 1));
            assert!(!t.time_allowed(t.curtime + 1));
            value["mintime"] = (t.curtime + 1).into();
            assert!(parse_template(value, 50_000).is_err());
        }
        for bits in [
            "00000000", "1a8fffff", "21010000", "22000100", "23000001", "1a0ffffe",
        ] {
            let mut value = gbt();
            value["bits"] = bits.into();
            assert!(parse_template(value, 50_000).is_err(), "{bits}");
        }
        let mut value = gbt();
        value["curtime"] = (u64::from(u32::MAX) + 1).into();
        assert!(parse_template(value, 50_000).is_err());
    }
}
