//! Store-backed `ApiBackend` for the `api` role.

use async_trait::async_trait;
use serde_json::{json, Value};
use spc_store::Store;

use crate::api::ApiBackend;

pub struct StoreBackend {
    store: Store,
    pool: Value,
    capacity: Value,
}

impl StoreBackend {
    pub fn new(store: Store, pool: Value, capacity: Value) -> Self {
        Self {
            store,
            pool,
            capacity,
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

    async fn audit(&self, _sp_id: &[u8; 32], _block_hash: &str) -> Option<Value> {
        // The type-32 window service is wired in WP11; returning None here
        // keeps the endpoint honest instead of fabricating a window.
        None
    }
}
