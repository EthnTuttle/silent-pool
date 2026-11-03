//! Store-backed `ApiBackend` for the `api` role.

use async_trait::async_trait;
use serde_json::{json, Value};
use spc_store::Store;

use crate::api::ApiBackend;
use crate::config::Config;
use crate::metrics::{render, MetricsSnapshot};

/// The static, config-sourced part of `GET /api/pool` (SPEC §6.1).
/// `endpoints` republishes `[reach] advertise` verbatim (SPEC §3.8), so a
/// miner can learn the pool's onion addresses from its clearnet API.
pub fn pool_info(config: &Config) -> Value {
    json!({
        "network": config.pool.network,
        "window_n": config.accounting.window_n,
        "coinbase_threshold_sats": config.accounting.coinbase_threshold_sats,
        "extensions": ["0x0001", "0x0003", "0x0020", "0x5350"],
        "endpoints": config.reach.advertise,
    })
}

/// Config-sourced constants for the `/metrics` config families (SPEC §9).
#[derive(Clone, Copy, Debug, Default)]
pub struct MetricsConfig {
    /// `[accounting] window_n`.
    pub window_n: u32,
    /// `[accounting] coinbase_threshold_sats` (the tier threshold `C`).
    pub coinbase_threshold_sats: u64,
    /// `[budget] floor_wu`.
    pub budget_floor_wu: u64,
    /// `[budget] ceiling_wu`.
    pub budget_ceiling_wu: u64,
}

pub struct StoreBackend {
    store: Store,
    pool: Value,
    capacity: Value,
    metrics_cfg: MetricsConfig,
}

impl StoreBackend {
    /// Backend with zeroed config families. Callers that know the config
    /// should use [`StoreBackend::with_metrics_config`]; this constructor
    /// keeps existing call sites compiling.
    pub fn new(store: Store, pool: Value, capacity: Value) -> Self {
        Self::with_metrics_config(store, pool, capacity, MetricsConfig::default())
    }

    pub fn with_metrics_config(
        store: Store,
        pool: Value,
        capacity: Value,
        metrics_cfg: MetricsConfig,
    ) -> Self {
        Self {
            store,
            pool,
            capacity,
            metrics_cfg,
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[async_trait]
impl ApiBackend for StoreBackend {
    async fn pool(&self) -> Value {
        self.pool.clone()
    }

    async fn window(&self) -> Value {
        match self.store.window_tail(16).await {
            Ok(slices) => json!({
                "slices": slices.iter().map(|s| json!({
                    "slice_id": s.slice_id,
                    "height": s.height,
                    "difficulty": s.difficulty,
                    "n_shares": s.n_shares,
                })).collect::<Vec<_>>(),
            }),
            Err(e) => json!({ "error": e.to_string() }),
        }
    }

    async fn blocks(&self, limit: usize) -> Vec<Value> {
        match self.store.recent_blocks(limit as i64).await {
            Ok(rows) => rows
                .into_iter()
                .map(|(hash, height, status, settled)| {
                    json!({
                        "block_hash": hex(&hash),
                        "height": height,
                        "status": status,
                        "settled_at": settled.map(|t| t.to_rfc3339()),
                    })
                })
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    async fn liability_sats(&self) -> u64 {
        self.store.liability_sats().await.unwrap_or(0)
    }

    async fn capacity(&self) -> Value {
        self.capacity.clone()
    }

    async fn me(&self, sp_id: &[u8; 32]) -> Option<Value> {
        let pending = self.store.pending_for(sp_id).await.ok()?;
        let ledger = self.store.ledger_for(sp_id, 50).await.ok()?;
        Some(json!({
            "pending_sats": pending,
            "ledger": ledger.into_iter().map(|(hash, height, rail, paid, ark, forfeit, delta)| json!({
                "block_hash": hex(&hash),
                "height": height,
                "rail": rail,
                "paid_sats": paid,
                "ark_due_sats": ark,
                "forfeited_sats": forfeit,
                "pending_delta": delta,
            })).collect::<Vec<_>>(),
        }))
    }

    async fn audit(&self, _sp_id: &[u8; 32], block_hash: &str) -> Option<Value> {
        let bytes = (0..block_hash.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&block_hash[i..i + 2], 16).ok())
            .collect::<Option<Vec<u8>>>()?;
        let hash: [u8; 32] = bytes.try_into().ok()?;
        let height = self.store.block_height(&hash).await.ok()??;
        let slices = self.store.slices_for_height(height).await.ok()?;
        Some(json!({
            "height": height,
            "slices": slices.into_iter().map(|s| {
                let shares: Value = serde_json::from_slice(&s.shares).unwrap_or(Value::Null);
                json!({
                    "slice_id": s.slice_id,
                    "difficulty": s.difficulty,
                    "root": hex(&s.root),
                    "shares": shares,
                })
            }).collect::<Vec<_>>(),
        }))
    }

    /// Gather one scrape from the live store and render it. A failed query
    /// omits its family and counts in `spc_metrics_scrape_errors_total`;
    /// the config-sourced families cannot fail.
    async fn metrics(&self) -> String {
        let mut snap = MetricsSnapshot {
            uptime_secs: crate::metrics::process_uptime_secs(),
            window_n: Some(self.metrics_cfg.window_n),
            coinbase_threshold_sats: Some(self.metrics_cfg.coinbase_threshold_sats),
            budget_floor_wu: Some(self.metrics_cfg.budget_floor_wu),
            budget_ceiling_wu: Some(self.metrics_cfg.budget_ceiling_wu),
            ..Default::default()
        };
        let mut failed = 0u32;
        match self.store.blocks_by_status().await {
            Ok(v) => snap.blocks_by_status = Some(v),
            Err(e) => {
                tracing::warn!(error = %e, "metrics: blocks_by_status failed");
                failed += 1;
            }
        }
        match self.store.liability_sats().await {
            Ok(v) => snap.pending_liability_sats = Some(v),
            Err(e) => {
                tracing::warn!(error = %e, "metrics: liability_sats failed");
                failed += 1;
            }
        }
        match self.store.ark_due_sats().await {
            Ok(v) => snap.ark_due_sats = Some(v),
            Err(e) => {
                tracing::warn!(error = %e, "metrics: ark_due_sats failed");
                failed += 1;
            }
        }
        match self.store.ark_payouts_by_status().await {
            Ok(v) => snap.ark_payouts_by_status = Some(v),
            Err(e) => {
                tracing::warn!(error = %e, "metrics: ark_payouts_by_status failed");
                failed += 1;
            }
        }
        match self.store.forfeited_sats_total().await {
            Ok(v) => snap.forfeited_sats_total = Some(v),
            Err(e) => {
                tracing::warn!(error = %e, "metrics: forfeited_sats_total failed");
                failed += 1;
            }
        }
        snap.scrape_errors = failed;
        render(&snap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `GET /api/pool` carries `[reach] advertise` verbatim (SPEC §3.8, §6.1):
    /// order preserved, no filtering, no rewriting.
    #[test]
    fn pool_info_publishes_advertise_verbatim_as_endpoints() {
        let text = r#"
[pool]
network = "regtest"
listen = "127.0.0.1:34254"
authority_public_key = "02aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
pool_payout_descriptor = "tr(tpubD6NzVbkrYhZ4YTest/0/*)"

[reach]
advertise = ["sv2://pool.example:34254", "sv2://abc.onion:34254", "sv1://abc.onion:3333"]
"#;
        let config: Config = toml::from_str(text).unwrap();
        config.validate().unwrap();
        let info = pool_info(&config);
        assert_eq!(
            info["endpoints"],
            json!([
                "sv2://pool.example:34254",
                "sv2://abc.onion:34254",
                "sv1://abc.onion:3333"
            ])
        );
    }

    #[test]
    fn pool_info_defaults_endpoints_to_empty() {
        let text = r#"
[pool]
network = "regtest"
listen = "127.0.0.1:34254"
authority_public_key = "02aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
pool_payout_descriptor = "tr(tpubD6NzVbkrYhZ4YTest/0/*)"
"#;
        let config: Config = toml::from_str(text).unwrap();
        config.validate().unwrap();
        assert_eq!(pool_info(&config)["endpoints"], json!([]));
    }
}
