//! Shared Bitcoin endpoint identity check. Use on the actual authenticated
//! client before consuming chain data; reconnect/refresh adapters recheck it.

#[cfg(feature = "rpc")]
pub fn validate_rpc_genesis(
    client: &impl bitcoincore_rpc::RpcApi,
    expected: spc_crypto::ChainNetwork,
) -> anyhow::Result<()> {
    // `getblockhash` avoids the pinned dependency's closed chain-name parser.
    // No RPC value is rewritten; the endpoint must return the real genesis.
    expected.validate_genesis(client.get_block_hash(0)?)?;
    Ok(())
}

/// Admission of the actual BIP54 packet's private activation marker. A known
/// Testnet5 endpoint that omits active cleanup facts is not a legacy source.
pub fn validate_template_chain(
    template: &crate::template::Template,
    chain: spc_crypto::ChainNetwork,
) -> Result<(), crate::template::TemplateError> {
    template.validate_context()?;
    if chain == spc_crypto::ChainNetwork::Testnet5 && !template.coinbase.consensus_cleanup() {
        return Err(crate::template::TemplateError::Unavailable(
            "Testnet5 template is missing active BIP54 consensuscleanup facts".into(),
        ));
    }
    Ok(())
}

/// The source and genesis client MUST be constructed from the same immutable
/// URL/credentials (the main dispatcher does this). No upstream RPC rewriting.
#[cfg(feature = "rpc")]
pub struct BoundTemplateSource<S, C> {
    inner: S,
    client: C,
    chain: spc_crypto::ChainNetwork,
}

#[cfg(feature = "rpc")]
impl<S, C: bitcoincore_rpc::RpcApi> BoundTemplateSource<S, C> {
    pub fn new(
        inner: S,
        client: C,
        chain: spc_crypto::ChainNetwork,
    ) -> Result<Self, crate::template::TemplateError> {
        let source = Self {
            inner,
            client,
            chain,
        };
        source.check()?;
        Ok(source)
    }
    fn check(&self) -> Result<(), crate::template::TemplateError> {
        validate_rpc_genesis(&self.client, self.chain)
            .map_err(|e| crate::template::TemplateError::Unavailable(e.to_string()))
    }
}

#[cfg(feature = "rpc")]
#[async_trait::async_trait]
impl<S: crate::template::TemplateSource, C: bitcoincore_rpc::RpcApi + Send + Sync + 'static>
    crate::template::TemplateSource for BoundTemplateSource<S, C>
{
    async fn get_template(
        &self,
    ) -> Result<crate::template::Template, crate::template::TemplateError> {
        self.check()?;
        let template = self.inner.get_template().await?;
        self.check()?;
        validate_template_chain(&template, self.chain)?;
        Ok(template)
    }
    async fn submit_block(&self, block: &[u8]) -> Result<(), crate::template::TemplateError> {
        self.check()?;
        self.inner.submit_block(block).await
    }
}

/// The genesis bracket also guards submission: a found block must never be
/// handed to an endpoint that stopped being the configured chain.
#[cfg(feature = "rpc")]
#[async_trait::async_trait]
impl<S, C> crate::template::BlockSubmitter for BoundTemplateSource<S, C>
where
    S: crate::template::BlockSubmitter,
    C: bitcoincore_rpc::RpcApi + Send + Sync + 'static,
{
    async fn submit_assembled(
        &self,
        block: &crate::block::AssembledBlock,
    ) -> Result<(), crate::template::TemplateError> {
        self.check()?;
        self.inner.submit_assembled(block).await
    }
}

/// Bind settlement's existing view without changing its economics interface.
/// Every authoritative read is bracketed by genesis checks on that same view.
pub struct BoundChainView<T> {
    inner: T,
    chain: spc_crypto::ChainNetwork,
}

impl<T: crate::payout::ChainView> BoundChainView<T> {
    pub async fn new(
        inner: T,
        chain: spc_crypto::ChainNetwork,
    ) -> Result<Self, crate::payout::ChainError> {
        let view = Self { inner, chain };
        view.check().await?;
        Ok(view)
    }
    async fn check(&self) -> Result<(), crate::payout::ChainError> {
        if self.inner.block_hash_at(0).await? != Some(self.chain.genesis_bytes()) {
            return Err(crate::payout::ChainError::Unavailable(
                "wrong or unavailable genesis".into(),
            ));
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl<T: crate::payout::ChainView> crate::payout::ChainView for BoundChainView<T> {
    async fn economic_evidence(
        &self,
        hash: &[u8; 32],
    ) -> Result<Option<crate::payout::ChainEconomicEvidence>, crate::payout::ChainError> {
        self.check().await?;
        let result = self.inner.economic_evidence(hash).await?;
        self.check().await?;
        Ok(result)
    }
    async fn transaction(
        &self,
        txid: &[u8; 32],
    ) -> Result<Option<bitcoin::Transaction>, crate::payout::ChainError> {
        self.check().await?;
        let result = self.inner.transaction(txid).await?;
        self.check().await?;
        Ok(result)
    }
    async fn tip_height(&self) -> Result<u32, crate::payout::ChainError> {
        self.check().await?;
        let result = self.inner.tip_height().await?;
        self.check().await?;
        Ok(result)
    }
    async fn block_hash_at(
        &self,
        height: u32,
    ) -> Result<Option<[u8; 32]>, crate::payout::ChainError> {
        self.check().await?;
        let result = self.inner.block_hash_at(height).await?;
        self.check().await?;
        Ok(result)
    }
    async fn coinbase_outputs(
        &self,
        hash: &[u8; 32],
    ) -> Result<Option<Vec<crate::payout::CoinbaseOutput>>, crate::payout::ChainError> {
        self.check().await?;
        let result = self.inner.coinbase_outputs(hash).await?;
        self.check().await?;
        Ok(result)
    }
    async fn block_economics(
        &self,
        hash: &[u8; 32],
    ) -> Result<Option<crate::payout::BlockEconomics>, crate::payout::ChainError> {
        self.check().await?;
        let result = self.inner.block_economics(hash).await?;
        self.check().await?;
        Ok(result)
    }
    fn authoritative(&self) -> bool {
        self.inner.authoritative()
    }
}

#[cfg(all(test, feature = "rpc"))]
mod tests {
    use super::*;
    use spc_crypto::ChainNetwork;
    struct Endpoint(serde_json::Value);

    fn template(coinbase: crate::template::CoinbaseFields) -> crate::template::Template {
        crate::template::Template {
            height: 1,
            prev_hash: ChainNetwork::Testnet5.genesis_bytes(),
            version: 0x2000_0000,
            bits: 0x1a0f_ffff,
            curtime: 1,
            min_time: 1,
            max_time: None,
            coinbase,
            subsidy: 5_000_000_000,
            fees: 0,
            coinbase_value: 5_000_000_000,
            transactions: vec![],
            merkle_branches: vec![],
            coinbase_output_max_additional_size: 1024,
        }
    }

    #[tokio::test]
    async fn known_testnet5_genesis_cannot_admit_missing_bip54_facts() {
        use crate::template::{CoinbaseFields, MockTemplateSource, TemplateSource};
        let endpoint = || {
            Endpoint(serde_json::json!(ChainNetwork::Testnet5
                .genesis()
                .to_string()))
        };
        let source = BoundTemplateSource::new(
            MockTemplateSource::new(template(CoinbaseFields::legacy(0))),
            endpoint(),
            ChainNetwork::Testnet5,
        )
        .unwrap();
        assert!(source.get_template().await.is_err());
        let active = CoinbaseFields::from_gbt_rules(1, &["!consensuscleanup".into()]).unwrap();
        let source = BoundTemplateSource::new(
            MockTemplateSource::new(template(active)),
            endpoint(),
            ChainNetwork::Testnet5,
        )
        .unwrap();
        assert!(source
            .get_template()
            .await
            .unwrap()
            .coinbase
            .consensus_cleanup());
        let mut invalid = active;
        invalid.sequence = u32::MAX;
        assert!(validate_template_chain(&template(invalid), ChainNetwork::Testnet5).is_err());
    }
    impl bitcoincore_rpc::RpcApi for Endpoint {
        fn call<T: for<'a> serde::Deserialize<'a>>(
            &self,
            cmd: &str,
            args: &[serde_json::Value],
        ) -> bitcoincore_rpc::Result<T> {
            assert_eq!(cmd, "getblockhash");
            assert_eq!(args, &[serde_json::json!(0)]);
            serde_json::from_value(self.0.clone()).map_err(bitcoincore_rpc::Error::Json)
        }
    }
    #[test]
    fn endpoint_requires_actual_genesis_not_compatible_address_encoding() {
        let wrong = Endpoint(serde_json::json!(ChainNetwork::Testnet4
            .genesis()
            .to_string()));
        assert!(validate_rpc_genesis(&wrong, ChainNetwork::Testnet5).is_err());
        let right = Endpoint(serde_json::json!(ChainNetwork::Testnet5
            .genesis()
            .to_string()));
        assert!(validate_rpc_genesis(&right, ChainNetwork::Testnet5).is_ok());
        assert!(validate_rpc_genesis(
            &Endpoint(serde_json::json!("testnet5")),
            ChainNetwork::Testnet5
        )
        .is_err());
    }

    struct SwitchingView(std::sync::atomic::AtomicBool);
    #[async_trait::async_trait]
    impl crate::payout::ChainView for SwitchingView {
        async fn tip_height(&self) -> Result<u32, crate::payout::ChainError> {
            self.0.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(100)
        }
        async fn block_hash_at(
            &self,
            _: u32,
        ) -> Result<Option<[u8; 32]>, crate::payout::ChainError> {
            Ok(Some(
                if self.0.load(std::sync::atomic::Ordering::SeqCst) {
                    ChainNetwork::Testnet4
                } else {
                    ChainNetwork::Testnet5
                }
                .genesis_bytes(),
            ))
        }
        fn authoritative(&self) -> bool {
            true
        }
    }
    #[tokio::test]
    async fn endpoint_switch_during_read_does_not_publish_foreign_result() {
        use crate::payout::ChainView;
        let view = BoundChainView::new(SwitchingView(false.into()), ChainNetwork::Testnet5)
            .await
            .unwrap();
        assert!(view.authoritative());
        assert!(view.tip_height().await.is_err());
        assert!(view.block_hash_at(0).await.is_err());
    }
}
