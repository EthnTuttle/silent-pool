//! `payout` role: the only Postgres writer. Consumes `spc:slices`,
//! `spc:blocks` and `spc:arkreg`, settles idempotently and publishes
//! `spc:front:events` (SPEC §5, §7.4).
//!
//! A block is booked only once the `ChainView` shows it `confirmation_depth`
//! deep on the active chain (review 4.1); a block whose distribution
//! fingerprint never arrived is booked `pending` with fingerprint NULL and
//! owes nothing until reconstructed (review 4.5, SPEC §7.4); Ark/Forfeit
//! rails in the block event are re-decided against `ark_delivery` in
//! Postgres at settlement (review 3.1, SPEC §11.1/§4.6(f)).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::json;
use spc_ext::ArkSendOrder;
use spc_store::models::*;
use spc_store::redis::{RedisError, Streams};
use spc_store::wire::{ArkRegEvent, BlockEvent, FrontEvent, Rail, SliceEvent};
use spc_store::Store;
use thiserror::Error;

pub const GROUP: &str = "payout";
pub const CONSUMER: &str = "payout-0";

/// How often the group pending list of `spc:blocks` is retried. A held
/// settlement is waiting on confirmations, so a slow cadence is fine.
const PENDING_RETRY_INTERVAL: Duration = Duration::from_secs(15);

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

/// A chain view failure never books anything; settlement waits and retries.
#[derive(Debug, Error)]
pub enum ChainError {
    #[error("chain view unavailable: {0}")]
    Unavailable(String),
}

/// The slice of chain state settlement needs (SPEC §7.4, review 4.1): is
/// this block the active-chain block at its height, and how deep is it?
#[async_trait::async_trait]
pub trait ChainView: Send + Sync {
    /// Height of the active-chain tip.
    async fn tip_height(&self) -> Result<u32, ChainError>;
    /// The active-chain block hash at `height`, or `None` when the chain
    /// has not reached `height` yet.
    async fn block_hash_at(&self, height: u32) -> Result<Option<[u8; 32]>, ChainError>;
    /// True when `block_hash` is the active-chain block at `height`.
    async fn is_active(&self, block_hash: &[u8; 32], height: u32) -> Result<bool, ChainError> {
        Ok(self
            .block_hash_at(height)
            .await?
            .is_some_and(|h| &h == block_hash))
    }
}

/// No chain source (tests, dev template sources): every block passes the
/// gate. `main` warns loudly whenever the payout role runs on this.
#[derive(Clone, Copy, Debug)]
pub struct AlwaysConfirm;

#[async_trait::async_trait]
impl ChainView for AlwaysConfirm {
    async fn tip_height(&self) -> Result<u32, ChainError> {
        Ok(u32::MAX)
    }

    async fn block_hash_at(&self, _height: u32) -> Result<Option<[u8; 32]>, ChainError> {
        Ok(None)
    }

    async fn is_active(&self, _block_hash: &[u8; 32], _height: u32) -> Result<bool, ChainError> {
        Ok(true)
    }
}

/// Bitcoin Core RPC chain view — the settlement gate when the template
/// source is `rpc` (review 4.1).
#[cfg(feature = "rpc")]
pub struct RpcChainView {
    client: bitcoincore_rpc::Client,
}

#[cfg(feature = "rpc")]
impl RpcChainView {
    pub fn new(url: &str, user: &str, pass: &str) -> Result<Self, bitcoincore_rpc::Error> {
        let auth = if user.is_empty() {
            bitcoincore_rpc::Auth::None
        } else {
            bitcoincore_rpc::Auth::UserPass(user.to_string(), pass.to_string())
        };
        Ok(Self {
            client: bitcoincore_rpc::Client::new(url, auth)?,
        })
    }
}

#[cfg(feature = "rpc")]
#[async_trait::async_trait]
impl ChainView for RpcChainView {
    async fn tip_height(&self) -> Result<u32, ChainError> {
        use bitcoincore_rpc::RpcApi;
        let count = self
            .client
            .get_block_count()
            .map_err(|e| ChainError::Unavailable(e.to_string()))?;
        u32::try_from(count).map_err(|_| ChainError::Unavailable("tip exceeds u32".into()))
    }

    async fn block_hash_at(&self, height: u32) -> Result<Option<[u8; 32]>, ChainError> {
        use bitcoin::hashes::Hash as _;
        use bitcoincore_rpc::RpcApi;
        match self.client.get_block_hash(u64::from(height)) {
            Ok(hash) => Ok(Some(hash.to_byte_array())),
            // RPC_INVALID_PARAMETER ("Block height out of range"): the chain
            // has not reached `height`.
            Err(bitcoincore_rpc::Error::JsonRpc(bitcoincore_rpc::jsonrpc::error::Error::Rpc(
                e,
            ))) if e.code == -8 => Ok(None),
            Err(e) => Err(ChainError::Unavailable(e.to_string())),
        }
    }
}

/// The §7.4 pre-booking chain check (review 4.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChainGate {
    /// Active-chain block at `height` with at least `depth` confirmations.
    Confirm,
    /// A different block is confirmed at `height`: orphan, no ledger rows.
    Orphan,
    /// Cannot confirm yet (too shallow, node behind, view flaky): retry.
    Wait,
}

fn chain_gate(
    active: bool,
    hash_at_height: Option<[u8; 32]>,
    block_hash: &[u8; 32],
    height: u32,
    tip: u32,
    depth: u32,
) -> ChainGate {
    let on_chain = active || matches!(hash_at_height, Some(h) if &h == block_hash);
    if !on_chain {
        // Only a confirmed competitor orphans the block now; "node behind"
        // is indistinguishable from "reorged out" on a partial view.
        return match hash_at_height {
            Some(_) => ChainGate::Orphan,
            None => ChainGate::Wait,
        };
    }
    if u64::from(tip) + 1 >= u64::from(height) + u64::from(depth) {
        ChainGate::Confirm
    } else {
        ChainGate::Wait
    }
}

/// SPEC §11.1/§4.6(f), applied at settlement (review 3.1). For entries the
/// front marked Ark or Forfeit, `payout` re-decides against the
/// `ark_delivery` row in Postgres NOW: a committed registration means the
/// sats are deliverable — `ark_due` when the front derived the per-block
/// destination, `pending` when it had not propagated yet (§11.1 branch 4;
/// the destination cannot be re-derived once `a_send` is gone). No row:
/// forfeit strictly below `C`; at or above `C` the balance is `pending`,
/// never silently forfeited.
fn settlement_rail(sats: u64, delivery: bool, dest_known: bool, c: u64) -> LedgerRail {
    match (delivery, dest_known) {
        (true, true) => LedgerRail::Ark,
        (true, false) => LedgerRail::Pending,
        (false, _) if sats < c => LedgerRail::Forfeit,
        (false, _) => LedgerRail::Pending,
    }
}

pub struct PayoutService {
    store: Store,
    streams: Streams,
    confirmation_depth: u32,
    /// `C` (SPEC §4.1, §11.1): the settlement-time forfeit boundary.
    coinbase_threshold_sats: u64,
    chain: Arc<dyn ChainView>,
    order_key: Option<secp256k1::Keypair>,
    max_order_sats: u64,
}

/// How one group entry ends up after handling.
enum Disposition {
    /// Handled (or settle-orphaned): ack it.
    Done,
    /// Waiting on the chain: leave unacked; the pending pass retries it.
    Retry,
}

impl PayoutService {
    pub fn new(
        store: Store,
        streams: Streams,
        confirmation_depth: u32,
        coinbase_threshold_sats: u64,
        chain: Arc<dyn ChainView>,
        order_key: Option<secp256k1::Keypair>,
        max_order_sats: u64,
    ) -> Self {
        Self {
            store,
            streams,
            confirmation_depth,
            coinbase_threshold_sats,
            chain,
            order_key,
            max_order_sats,
        }
    }

    pub async fn run(&self) -> Result<(), PayoutError> {
        self.store.migrate().await?;
        for s in ["spc:slices", "spc:blocks", "spc:arkreg"] {
            self.streams.ensure_group(s, GROUP).await?;
        }
        // Held block events wait in the consumer group's pending list; run
        // the retry pass once at startup so restarts pick them up, then on
        // the interval.
        let mut last_pending = Instant::now()
            .checked_sub(PENDING_RETRY_INTERVAL)
            .unwrap_or_else(Instant::now);
        loop {
            let mut did = false;
            for (stream, kind) in [
                ("spc:slices", Kind::Slice),
                ("spc:blocks", Kind::Block),
                ("spc:arkreg", Kind::Reg),
            ] {
                let entries = self.streams.read_group(stream, GROUP, CONSUMER, 16).await?;
                for (id, payload) in entries {
                    let res: Result<Disposition, PayoutError> = match kind {
                        Kind::Slice => self.on_slice(&payload).await.map(|_| Disposition::Done),
                        Kind::Block => self.on_block(&payload).await,
                        Kind::Reg => self.on_reg(&payload).await.map(|_| Disposition::Done),
                    };
                    match res {
                        Ok(Disposition::Done) => {
                            self.streams.ack(stream, GROUP, &id).await?;
                            did = true;
                        }
                        Ok(Disposition::Retry) => {
                            did = true;
                        }
                        Err(e) => {
                            tracing::error!(error = %e, stream, id, "payout entry failed");
                        }
                    }
                }
            }
            if last_pending.elapsed() >= PENDING_RETRY_INTERVAL {
                last_pending = Instant::now();
                match self
                    .streams
                    .read_pending("spc:blocks", GROUP, CONSUMER, 64)
                    .await
                {
                    Ok(pending) => {
                        for (id, payload) in pending {
                            match self.on_block(&payload).await {
                                Ok(Disposition::Done) => {
                                    self.streams.ack("spc:blocks", GROUP, &id).await?;
                                    did = true;
                                }
                                Ok(Disposition::Retry) => {}
                                Err(e) => {
                                    tracing::error!(error = %e, id, "pending block retry failed")
                                }
                            }
                        }
                    }
                    Err(e) => tracing::error!(error = %e, "pending block scan failed"),
                }
            }
            if !did {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    }

    async fn on_slice(&self, payload: &[u8]) -> Result<(), PayoutError> {
        let e = SliceEvent::decode(payload)?;
        // `slice_credit` references `sp_identity` (SPEC §5.1), but the slice
        // stream carries only `sp_id` — no keys. A miner that has not
        // registered Ark delivery yet gets a placeholder identity row here;
        // the registration upsert replaces it once keys arrive.
        let mut seen = std::collections::BTreeSet::new();
        for c in &e.credits {
            if seen.insert(c.sp_id) {
                self.store
                    .insert_identity_if_absent(&placeholder_identity(c.sp_id))
                    .await?;
            }
        }
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

    async fn on_block(&self, payload: &[u8]) -> Result<Disposition, PayoutError> {
        let e = BlockEvent::decode(payload)?;

        // Terminal states are absorbing (§7.4 idempotency); a redelivery
        // then changes nothing and, unlike a fresh pass, emits nothing.
        match self.store.block_status(&e.block_hash).await? {
            Some(BlockStatus::Confirmed) | Some(BlockStatus::Orphaned) => {
                return Ok(Disposition::Done)
            }
            Some(BlockStatus::Pending) | None => {}
        }

        // Review 4.1 / SPEC §7.4: book only a block that sits
        // `confirmation_depth` deep on the active chain. Any doubt — chain
        // view down, node not yet at the height, too shallow — leaves the
        // event pending for a later pass (fail closed); a competitor
        // confirmed at the same height orphans the block, no ledger rows.
        let active = match self.chain.is_active(&e.block_hash, e.height).await {
            Ok(active) => active,
            Err(err) => {
                tracing::warn!(error = %err, height = e.height, "chain view unavailable; holding settlement");
                return Ok(Disposition::Retry);
            }
        };
        let hash_at_height = if active {
            Some(e.block_hash)
        } else {
            match self.chain.block_hash_at(e.height).await {
                Ok(hash) => hash,
                Err(err) => {
                    tracing::warn!(error = %err, height = e.height, "chain view unavailable; holding settlement");
                    return Ok(Disposition::Retry);
                }
            }
        };
        let tip = match self.chain.tip_height().await {
            Ok(tip) => tip,
            Err(err) => {
                tracing::warn!(error = %err, "chain view unavailable; holding settlement");
                return Ok(Disposition::Retry);
            }
        };
        match chain_gate(
            active,
            hash_at_height,
            &e.block_hash,
            e.height,
            tip,
            self.confirmation_depth,
        ) {
            ChainGate::Wait => return Ok(Disposition::Retry),
            ChainGate::Orphan => {
                tracing::warn!(
                    block = %display_hash(&e.block_hash),
                    height = e.height,
                    "found block is not on the active chain; booking orphaned with no ledger rows"
                );
                let orphaning = Settlement {
                    block_hash: e.block_hash,
                    height: e.height,
                    fingerprint: None,
                    status: BlockStatus::Orphaned,
                    ledger: Vec::new(),
                };
                match self.store.settle(&orphaning, self.confirmation_depth).await {
                    Ok(())
                    | Err(spc_store::postgres::StoreError::AlreadySettled(_))
                    | Err(spc_store::postgres::StoreError::Orphaned(_)) => {}
                    Err(err) => return Err(err.into()),
                }
                return Ok(Disposition::Done);
            }
            ChainGate::Confirm => {}
        }

        // Review 4.5 / SPEC §7.4 / §11.1: an event that lost its
        // distribution fingerprint is booked `pending` with fingerprint
        // NULL; the schema CHECK forbids `confirmed` there, the block owes
        // nothing until an operator reconstructs the event, and this alarm
        // is the signal to do so.
        let fingerprint = if e.fingerprint == [0u8; 32] {
            None
        } else {
            Some(e.fingerprint)
        };
        let status = match fingerprint {
            Some(fingerprint) => {
                self.store
                    .insert_snapshot(&SnapshotRow {
                        fingerprint,
                        height: e.height,
                        a_send_x: e.a_send_x,
                        distribution_id: None,
                        entries: json!(e.entries),
                        high_water: json!(e.high_water),
                    })
                    .await?;
                BlockStatus::Confirmed
            }
            None => {
                tracing::error!(
                    block = %display_hash(&e.block_hash),
                    height = e.height,
                    "block event carries no distribution fingerprint; booking pending, nothing owed"
                );
                BlockStatus::Pending
            }
        };

        let mut ledger = Vec::with_capacity(e.entries.len());
        let mut rails = Vec::with_capacity(e.entries.len());
        let mut deltas = Vec::new();
        for entry in &e.entries {
            let rail = match entry.rail {
                Rail::Coinbase => LedgerRail::Coinbase,
                Rail::Pending => LedgerRail::Pending,
                Rail::Ark | Rail::Forfeit => {
                    let delivery = self.store.ark_delivery(&entry.sp_id).await?.is_some();
                    settlement_rail(
                        entry.sats,
                        delivery,
                        !entry.dest_id.is_empty(),
                        self.coinbase_threshold_sats,
                    )
                }
            };
            rails.push(rail);
            let row = match rail {
                LedgerRail::Coinbase => LedgerRow {
                    sp_id: entry.sp_id,
                    rail: LedgerRail::Coinbase,
                    paid_sats: entry.sats,
                    ark_due_sats: 0,
                    forfeited_sats: 0,
                    pending_delta: 0,
                },
                LedgerRail::Pending => {
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
                LedgerRail::Ark => LedgerRow {
                    sp_id: entry.sp_id,
                    rail: LedgerRail::Ark,
                    paid_sats: 0,
                    ark_due_sats: entry.sats,
                    forfeited_sats: 0,
                    pending_delta: 0,
                },
                LedgerRail::Forfeit => LedgerRow {
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
            fingerprint,
            status,
            ledger,
        };
        let booked = match self
            .store
            .settle(&settlement, self.confirmation_depth)
            .await
        {
            Ok(()) => true,
            Err(spc_store::postgres::StoreError::AlreadySettled(_)) => false,
            Err(err) => return Err(err.into()),
        };
        if !booked || status != BlockStatus::Confirmed {
            return Ok(Disposition::Done);
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
        if let Some(key) = &self.order_key {
            for (entry, rail) in e.entries.iter().zip(&rails) {
                if *rail != LedgerRail::Ark || entry.dest_id.is_empty() {
                    continue;
                }
                if self.max_order_sats > 0 && entry.sats > self.max_order_sats {
                    tracing::warn!(
                        sats = entry.sats,
                        "Ark order exceeds max_order_sats; not emitted"
                    );
                    continue;
                }
                let Ok(address) = String::from_utf8(entry.dest_id.clone()) else {
                    tracing::warn!("Ark destination is not UTF-8; not emitted");
                    continue;
                };
                let mut order_id = [0u8; 16];
                rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut order_id);
                let mut order = ArkSendOrder {
                    order_id,
                    ark_address: address,
                    sats: entry.sats,
                    not_before_height: e.height.saturating_add(self.confirmation_depth),
                    sig: [0u8; 64],
                };
                if order.sign(key).is_ok() {
                    if let Ok(bytes) = order.encode() {
                        let _ = self.streams.add("spc:ark:orders", &bytes).await;
                    }
                }
            }
        }
        Ok(Disposition::Done)
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

/// Display byte order (reversed from the internal sha256d bytes) for logs.
fn display_hash(block_hash: &[u8; 32]) -> String {
    let mut bytes = *block_hash;
    bytes.reverse();
    hex::encode(bytes)
}

/// Placeholder identity for a miner seen in a slice before it registers
/// Ark delivery (SPEC §5.1). The keys are all-zero 33-byte blobs — a
/// compressed secp256k1 public key never serializes to all zeros, so no
/// real registration can collide with or be mistaken for a placeholder.
/// The real `b_scan`/`b_spend` (and network) arrive later via the
/// registration path (`on_reg`), whose upsert replaces this row.
///
/// The network code is provisional: the slice stream carries no network,
/// and the value is overwritten at registration. 3 (regtest) keeps the
/// placeholder distinguishable from the testnet4 code the registration
/// path currently books.
fn placeholder_identity(sp_id: [u8; 32]) -> SpIdentityRow {
    SpIdentityRow {
        sp_id,
        b_scan: vec![0u8; 33],
        b_spend: vec![0u8; 33],
        network: 3,
    }
}

/// Convenience for tests: a pending map as the `LedgerApplied` payload.
pub fn deltas_map(deltas: Vec<([u8; 32], i64)>) -> BTreeMap<[u8; 32], i64> {
    deltas.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settlement_rail_rechecks_delivery_at_settlement() {
        // Delivery committed and the destination reached the front: the Ark
        // send can be built (§11.1 branch 3).
        assert_eq!(settlement_rail(75, true, true, 330), LedgerRail::Ark);
        assert_eq!(settlement_rail(250_000, true, true, 330), LedgerRail::Ark);
        // Delivery committed but the destination never reached the building
        // front: §11.1 branch 4 — pending, never forfeit (the destination is
        // gone with `a_send` and cannot be re-derived).
        assert_eq!(settlement_rail(75, true, false, 330), LedgerRail::Pending);
        assert_eq!(
            settlement_rail(250_000, true, false, 330),
            LedgerRail::Pending
        );
        // No delivery row at settlement: forfeit strictly below C (§4.6(f))...
        assert_eq!(settlement_rail(329, false, false, 330), LedgerRail::Forfeit);
        assert_eq!(settlement_rail(1, false, true, 330), LedgerRail::Forfeit);
        // ...and never at or above C, whatever the front marked.
        assert_eq!(settlement_rail(330, false, false, 330), LedgerRail::Pending);
        assert_eq!(
            settlement_rail(250_000, false, false, 330),
            LedgerRail::Pending
        );
        assert_eq!(settlement_rail(330, false, true, 330), LedgerRail::Pending);
    }

    #[test]
    fn chain_gate_confirms_waits_and_orphans() {
        let block = [9u8; 32];
        let depth = 6;
        // Active-chain block, exactly confirmation_depth deep: book.
        assert_eq!(
            chain_gate(true, Some(block), &block, 800_000, 800_005, depth),
            ChainGate::Confirm
        );
        // One confirmation short: hold and retry, nothing is booked.
        assert_eq!(
            chain_gate(true, Some(block), &block, 800_000, 800_004, depth),
            ChainGate::Wait
        );
        // Local tip behind the found block: hold.
        assert_eq!(
            chain_gate(true, Some(block), &block, 800_000, 799_999, depth),
            ChainGate::Wait
        );
        // A competitor holds the height: orphaned, no ledger rows.
        assert_eq!(
            chain_gate(false, Some([7u8; 32]), &block, 800_000, 800_100, depth),
            ChainGate::Orphan
        );
        // The node has not reached the height: hold — never orphan on an
        // incomplete view.
        assert_eq!(
            chain_gate(false, None, &block, 800_000, 799_999, depth),
            ChainGate::Wait
        );
        // Inconsistent view (not active, but the same hash at the height):
        // treat as active and apply the depth rule.
        assert_eq!(
            chain_gate(false, Some(block), &block, 800_000, 800_005, depth),
            ChainGate::Confirm
        );
    }

    #[tokio::test]
    async fn always_confirm_passes_everything() {
        let view = AlwaysConfirm;
        let hash = [3u8; 32];
        assert!(view.is_active(&hash, 800_000).await.unwrap());
        assert_eq!(view.tip_height().await.unwrap(), u32::MAX);
        assert_eq!(view.block_hash_at(800_000).await.unwrap(), None);
    }
}
