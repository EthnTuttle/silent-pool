//! Chain binding for one pool database and its bootstrap consumer.
//! Existing unbound economic data is not automatically assigned a chain.

use serde::{Deserialize, Serialize};
use spc_crypto::ChainNetwork;
use sqlx::Row;

use crate::{
    postgres::{Result, StoreError},
    Store,
};

/// Public metadata; genesis uses canonical display-order lowercase hex.
/// Versioned independently from economic/job/Ark wire formats.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChainBinding {
    pub version: u8,
    pub network: String,
    pub network_code: i16,
    pub genesis_hash: String,
}

impl ChainBinding {
    pub fn new(chain: ChainNetwork) -> Self {
        Self {
            version: 1,
            network: chain.name().into(),
            network_code: chain.code(),
            genesis_hash: chain.genesis().to_string(),
        }
    }

    pub fn validate(&self, expected: ChainNetwork) -> Result<()> {
        if self != &Self::new(expected) {
            return Err(StoreError::Invalid("chain binding mismatch"));
        }
        Ok(())
    }
}

/// Bind the dedicated SPc-side Redis DB before any consumer/group/lease.
/// Payout alone may initialize an entirely empty DB. Old streams have no chain
/// envelope, so silently adopting a nonempty namespace would defeat SQL binding.
/// This is never called by the Ark role and grants it no identity-key access.
pub async fn bind_redis_namespace(
    url: &str,
    chain: ChainNetwork,
    initialize: bool,
) -> anyhow::Result<()> {
    let client = redis::Client::open(url)?;
    let mut conn = client.get_multiplexed_async_connection().await?;
    let expected = serde_json::to_string(&ChainBinding::new(chain))?;
    // One atomic decision, no scan, no automatic expiration or overwrite.
    let script = r#"
        local old = redis.call('GET', KEYS[1])
        if old then
            if old == ARGV[1] then return 1 else return 0 end
        end
        if ARGV[2] ~= '1' or redis.call('DBSIZE') ~= 0 then return 0 end
        redis.call('SET', KEYS[1], ARGV[1])
        return 1
    "#;
    let accepted: i64 = redis::cmd("EVAL")
        .arg(script)
        .arg(1)
        .arg("spc:chain-identity")
        .arg(expected)
        .arg(if initialize { "1" } else { "0" })
        .query_async(&mut conn)
        .await?;
    anyhow::ensure!(
        accepted == 1,
        "Redis chain identity mismatch or unbound nonempty namespace"
    );
    Ok(())
}

impl Store {
    /// Read-only preflight, BEFORE migrations. A bound foreign DB must not be
    /// changed even by an additive migration. Missing metadata is not approval
    /// to adopt existing data: `initialize_chain` handles only an empty DB.
    pub async fn preflight_chain(&self, expected: ChainNetwork) -> Result<()> {
        let exists: bool =
            sqlx::query_scalar("SELECT to_regclass('pool_chain_identity') IS NOT NULL")
                .fetch_one(self.pool())
                .await?;
        if exists {
            let rows: Vec<serde_json::Value> =
                sqlx::query_scalar("SELECT binding FROM pool_chain_identity")
                    .fetch_all(self.pool())
                    .await?;
            if rows.len() > 1 {
                return Err(StoreError::Invalid("multiple chain bindings"));
            }
            for value in rows {
                let binding: ChainBinding = serde_json::from_value(value)
                    .map_err(|_| StoreError::Invalid("invalid chain binding"))?;
                binding.validate(expected)?;
            }
        }
        Ok(())
    }

    /// Payout-only initial binding, serialized with other initializers.
    /// Run after migrations and genesis validation, before economic state or
    /// stream consumers start. No old row, descriptor or checksum is rewritten.
    pub async fn initialize_chain(&self, expected: ChainNetwork) -> Result<()> {
        let mut tx = self.pool().begin().await?;
        sqlx::query("LOCK TABLE pool_chain_identity IN EXCLUSIVE MODE")
            .execute(&mut *tx)
            .await?;
        let rows = sqlx::query("SELECT binding FROM pool_chain_identity")
            .fetch_all(&mut *tx)
            .await?;
        if rows.len() > 1 {
            return Err(StoreError::Invalid("multiple chain bindings"));
        }
        if let Some(row) = rows.first() {
            let binding: ChainBinding = serde_json::from_value(row.try_get("binding")?)
                .map_err(|_| StoreError::Invalid("invalid chain binding"))?;
            binding.validate(expected)?;
        } else {
            // Stop writes to chain-bearing tables while deciding freshness.
            sqlx::query("LOCK TABLE sp_identity, slice, block_found, pending_balance, economic_chain, economic_block, distribution_snapshot, ark_aggregate IN SHARE MODE")
                .execute(&mut *tx).await?;
            let used: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM sp_identity) OR EXISTS(SELECT 1 FROM slice)
                 OR EXISTS(SELECT 1 FROM block_found) OR EXISTS(SELECT 1 FROM pending_balance)
                 OR EXISTS(SELECT 1 FROM economic_chain) OR EXISTS(SELECT 1 FROM economic_block)
                 OR EXISTS(SELECT 1 FROM distribution_snapshot) OR EXISTS(SELECT 1 FROM ark_aggregate)
                 OR EXISTS(SELECT 1 FROM economic_state WHERE anchor_hash IS NOT NULL OR revision <> 0)")
                .fetch_one(&mut *tx).await?;
            if used {
                return Err(StoreError::Invalid(
                    "unbound existing chain data requires explicit migration",
                ));
            }
            sqlx::query("INSERT INTO pool_chain_identity(id,binding) VALUES(1,$1)")
                .bind(
                    serde_json::to_value(ChainBinding::new(expected))
                        .map_err(|_| StoreError::Invalid("invalid chain binding"))?,
                )
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Reader/startup gate; absence is unknown, never a default chain.
    pub async fn require_chain(&self, expected: ChainNetwork) -> Result<ChainBinding> {
        let rows: Vec<serde_json::Value> =
            sqlx::query_scalar("SELECT binding FROM pool_chain_identity")
                .fetch_all(self.pool())
                .await?;
        if rows.len() != 1 {
            return Err(StoreError::Invalid("missing chain binding"));
        }
        let binding: ChainBinding = serde_json::from_value(rows[0].clone())
            .map_err(|_| StoreError::Invalid("invalid chain binding"))?;
        binding.validate(expected)?;
        Ok(binding)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_rejects_same_prefix_chain_wrong_genesis_and_unknown_version() {
        let mut binding = ChainBinding::new(ChainNetwork::Testnet5);
        assert!(binding.validate(ChainNetwork::Testnet4).is_err());
        binding.genesis_hash = ChainNetwork::Testnet4.genesis().to_string();
        assert!(binding.validate(ChainNetwork::Testnet5).is_err());
        binding = ChainBinding::new(ChainNetwork::Testnet5);
        binding.version = 2;
        assert!(binding.validate(ChainNetwork::Testnet5).is_err());
        assert!(serde_json::from_str::<ChainBinding>("{}").is_err());
    }
}
