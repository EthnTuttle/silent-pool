//! `payout` role: the only Postgres writer. Consumes `spc:slices`,
//! `spc:blocks` and `spc:arkreg`, settles idempotently and publishes
//! `spc:front:events` (SPEC §5, §7.4).

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;
use spc_store::models::*;
use spc_store::redis::{RedisError, Streams};
use spc_store::wire::{ArkRegEvent, BlockEvent, FrontEvent, SliceEvent};
use spc_store::Store;
use thiserror::Error;

pub const GROUP: &str = "payout";
pub const CONSUMER: &str = "payout-0";

#[derive(Debug, Error)]
pub enum PayoutError {
    #[error("store: {0}")]
    Store(#[from] spc_store::postgres::StoreError),
    #[error("redis: {0}")]
    Redis(#[from] RedisError),
    #[error("codec: {0}")]
    Codec(#[from] spc_store::codec::CodecError),
    #[error("invalid registration")]
    Registration,
}

pub struct PayoutService {
    store: Store,
    streams: Streams,
    confirmation_depth: u32,
}

impl PayoutService {
    pub fn new(store: Store, streams: Streams, confirmation_depth: u32) -> Self {
        Self {
            store,
            streams,
            confirmation_depth,
        }
    }

    pub async fn run(&self) -> Result<(), PayoutError> {
        self.store.migrate().await?;
        for s in ["spc:slices", "spc:blocks", "spc:arkreg"] {
            self.streams.ensure_group(s, GROUP).await?;
        }
        loop {
            let mut did = false;
            for (stream, kind) in [
                ("spc:slices", Kind::Slice),
                ("spc:blocks", Kind::Block),
                ("spc:arkreg", Kind::Reg),
            ] {
                let entries = self.streams.read_group(stream, GROUP, CONSUMER, 16).await?;
                for (id, payload) in entries {
                    let res = match kind {
                        Kind::Slice => self.on_slice(&payload).await,
                        Kind::Block => self.on_block(&payload).await,
                        Kind::Reg => self.on_reg(&payload).await,
                    };
                    match res {
                        Ok(()) => {
                            self.streams.ack(stream, GROUP, &id).await?;
                            did = true;
                        }
                        Err(e) => {
                            tracing::error!(error = %e, stream, id, "payout entry failed");
                        }
                    }
                }
            }
            if !did {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
        }
    }

    async fn on_slice(&self, payload: &[u8]) -> Result<(), PayoutError> {
        let e = SliceEvent::decode(payload)?;
        let credits = e
            .credits
            .iter()
            .map(|c| SliceCreditRow {
                sp_id: c.sp_id,
                diff_sum: c.diff_sum.to_string(),
                diff_fee_sum: c.diff_fee_sum.to_string(),
            })
            .collect();
        self.store
            .insert_slice(&SliceRow {
                slice_id: e.slice_id,
                height: e.height,
                prev_hash: e.prev_hash,
                ref_job_id: e.ref_job_id,
                ref_fees: e.ref_fees,
                n_shares: e.n_shares,
                difficulty: e.difficulty.to_string(),
                root: e.root,
                shares: e.shares_blob,
                credits,
            })
            .await?;
        Ok(())
    }

    async fn on_block(&self, payload: &[u8]) -> Result<(), PayoutError> {
        let e = BlockEvent::decode(payload)?;
        let entries_json = json!(e.entries);
        let high_water = json!(e.high_water);
        self.store
            .insert_snapshot(&SnapshotRow {
                fingerprint: e.fingerprint,
                height: e.height,
                a_send_x: e.a_send_x,
                distribution_id: None,
                entries: entries_json,
                high_water,
            })
            .await?;

        let mut ledger = Vec::with_capacity(e.entries.len());
        let mut deltas = Vec::new();
        for entry in &e.entries {
            let row = match entry.rail {
                spc_store::wire::Rail::Coinbase => LedgerRow {
                    sp_id: entry.sp_id,
                    rail: LedgerRail::Coinbase,
                    paid_sats: entry.sats,
                    ark_due_sats: 0,
                    forfeited_sats: 0,
                    pending_delta: 0,
                },
                spc_store::wire::Rail::Pending => {
                    deltas.push((entry.sp_id, entry.sats as i64));
                    LedgerRow {
                        sp_id: entry.sp_id,
                        rail: LedgerRail::Pending,
                        paid_sats: 0,
                        ark_due_sats: 0,
                        forfeited_sats: 0,
                        pending_delta: entry.sats as i64,
                    }
                }
                spc_store::wire::Rail::Ark => LedgerRow {
                    sp_id: entry.sp_id,
                    rail: LedgerRail::Ark,
                    paid_sats: 0,
                    ark_due_sats: entry.sats,
                    forfeited_sats: 0,
                    pending_delta: 0,
                },
                spc_store::wire::Rail::Forfeit => LedgerRow {
                    sp_id: entry.sp_id,
                    rail: LedgerRail::Forfeit,
                    paid_sats: 0,
                    ark_due_sats: 0,
                    forfeited_sats: entry.sats,
                    pending_delta: 0,
                },
            };
            ledger.push(row);
        }

        let settlement = Settlement {
            block_hash: e.block_hash,
            height: e.height,
            fingerprint: Some(e.fingerprint),
            status: BlockStatus::Confirmed,
            ledger,
        };
        match self
            .store
            .settle(&settlement, self.confirmation_depth)
            .await
        {
            Ok(()) | Err(spc_store::postgres::StoreError::AlreadySettled(_)) => {}
            Err(err) => return Err(err.into()),
        }

        let event = FrontEvent::LedgerApplied {
            rev: now_rev(),
            block_hash: e.block_hash,
            republish: true,
            pending_deltas: deltas,
        };
        if let Ok(bytes) = event.encode() {
            let _ = self.streams.add("spc:front:events", &bytes).await;
        }
        Ok(())
    }

    async fn on_reg(&self, payload: &[u8]) -> Result<(), PayoutError> {
        let e = ArkRegEvent::decode(payload)?;
        // The pool recomputes sp_id; a mismatch is a rejected registration.
        let scan = bitcoin::secp256k1::PublicKey::from_slice(&e.b_scan)
            .map_err(|_| PayoutError::Registration)?;
        let spend = bitcoin::secp256k1::PublicKey::from_slice(&e.b_spend)
            .map_err(|_| PayoutError::Registration)?;
        if spc_crypto::sp_id(&scan, &spend) != e.sp_id {
            return Err(PayoutError::Registration);
        }
        self.store
            .upsert_identity(&SpIdentityRow {
                sp_id: e.sp_id,
                b_scan: e.b_scan.to_vec(),
                b_spend: e.b_spend.to_vec(),
                network: 1,
            })
            .await?;
        let mode = match e.mailbox_mode {
            0 => MailboxMode::None,
            1 => MailboxMode::Static,
            2 => MailboxMode::EpochDerived,
            3 => MailboxMode::EpochRegistered,
            _ => return Err(PayoutError::Registration),
        };
        self.store
            .upsert_ark_delivery(&ArkDeliveryRow {
                sp_id: e.sp_id,
                ark_id: e.ark_id,
                b_ark: e.b_ark.to_vec(),
                mailbox_mode: mode,
                mailbox_m: if e.mailbox_pubkey.is_empty() {
                    None
                } else {
                    Some(e.mailbox_pubkey)
                },
                mailbox_d: if e.mailbox_dh.is_empty() {
                    None
                } else {
                    Some(e.mailbox_dh)
                },
            })
            .await?;
        let mut event = FrontEvent::ArkRegApplied {
            rev: now_rev(),
            sp_id: e.sp_id,
        };
        if let Ok(bytes) = event.encode() {
            let _ = self.streams.add("spc:front:events", &bytes).await;
        }
        let _ = &mut event;
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Slice,
    Block,
    Reg,
}

fn now_rev() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Convenience for tests: a pending map as the `LedgerApplied` payload.
pub fn deltas_map(deltas: Vec<([u8; 32], i64)>) -> BTreeMap<[u8; 32], i64> {
    deltas.into_iter().collect()
}
