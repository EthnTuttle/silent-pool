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
//!
//! Settlement never trusts the event's sats. For an authoritative chain view
//! it reconstructs the window from `distribution_snapshot.high_water` and the
//! stored slices (SPEC §4.3, §7.4), reads the mined coinbase (SPEC §4.6:
//! `T_actual = Σ coinbase outputs`, `paid(id)` from `OP_1 ser_x(P_id)`) and
//! derives every ledger row from those two sources. A non-authoritative view
//! (dev template sources, tests) falls back to the event metadata with a
//! loud warning; an authoritative view that cannot supply the coinbase holds
//! settlement rather than guess.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;
use spc_accounting::{
    claim_jd, p2tr_script, raw_payouts, route_tiers, select_window, ArkCapability, RawPayouts,
    SliceAgg, SpId, TierConfig,
};
use spc_ext::{ArkSendOrder, ArkSendReceipt, ReceiptStatus};
use spc_store::models::*;
use spc_store::redis::{RedisError, Streams};
use spc_store::wire::{ArkRegEvent, BlockEvent, Entry, FrontEvent, Rail, SliceEvent};
use spc_store::Store;

use crate::ark::SeenOrders;
use thiserror::Error;

pub const GROUP: &str = "payout";
pub const CONSUMER: &str = "payout-0";

/// How often the group pending list of `spc:blocks` is retried. A held
/// settlement is waiting on confirmations, so a slow cadence is fine.
const PENDING_RETRY_INTERVAL: Duration = Duration::from_secs(15);

/// How often `payout` re-emits due `ark_payout` rows as signed orders. The
/// order id is deterministic from `(block_hash, sp_id)`, so a re-emit after a
/// restart is idempotent at the `ark` dedupe (SPEC §11.3).
const ARK_EMIT_INTERVAL: Duration = Duration::from_secs(30);

/// How often old attribution data is scrubbed (SPEC §5.1). Scrubbing is
/// idempotent; a slow cadence is enough.
const SCRUB_INTERVAL: Duration = Duration::from_secs(600);

/// How often recent `LedgerApplied` events are re-emitted. This heals a lost
/// XADD; the front ignores events for a block hash it already applied.
const REPUBLISH_INTERVAL: Duration = Duration::from_secs(30);

/// Upper bound on the slice walk used to reconstruct one window. Slices are
/// pruned after `retain_windows`; hitting this bound means the walk is
/// ambiguous, so settlement holds (fail closed).
const WINDOW_SLICE_LIMIT: i64 = 4096;

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
    #[error("malformed snapshot {0}")]
    Snapshot(&'static str),
    #[error("value exceeds i64")]
    Overflow,
}

/// A chain view failure never books anything; settlement waits and retries.
#[derive(Debug, Error)]
pub enum ChainError {
    #[error("chain view unavailable: {0}")]
    Unavailable(String),
}

/// One output of the mined coinbase, in script/value order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoinbaseOutput {
    pub script: Vec<u8>,
    pub value: u64,
}

/// The mined block's economics, as the chain reports them. `subsidy + fees`
/// equals `T_actual` (the sum of coinbase outputs) for a valid block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockEconomics {
    pub subsidy: u64,
    pub fees: u64,
}

/// The slice of chain state settlement needs (SPEC §7.4, review 4.1): is
/// this block the active-chain block at its height, how deep is it, what did
/// its coinbase pay, and what were its subsidy and fees?
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

    /// The mined coinbase's outputs for `block_hash` (SPEC §4.6, §7.4).
    /// `None` means this view cannot supply them (a dev view); an `Err`
    /// always holds settlement.
    async fn coinbase_outputs(
        &self,
        _block_hash: &[u8; 32],
    ) -> Result<Option<Vec<CoinbaseOutput>>, ChainError> {
        Ok(None)
    }

    /// The block's subsidy and fees. `None` means this view cannot supply
    /// them; settlement then cannot split `T_actual` into the two SPEC §4.4
    /// terms and holds for an authoritative view.
    async fn block_economics(
        &self,
        _block_hash: &[u8; 32],
    ) -> Result<Option<BlockEconomics>, ChainError> {
        Ok(None)
    }

    /// True when this view is the settlement authority. An authoritative
    /// view that cannot supply the coinbase holds settlement instead of
    /// falling back to the block event's sats.
    fn authoritative(&self) -> bool {
        false
    }
}

/// No chain source (tests, dev template sources): every block passes the
/// gate and settlement uses the event metadata. This is ONLY for dev use
/// on regtest/signet with `accounting.allow_unchecked_chain = true`.
/// Production MUST use `RpcChainView` with `template.source = "rpc"`.
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

/// Bitcoin Core RPC chain view - the settlement gate when the template
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

    fn block_hash(hash: &[u8; 32]) -> bitcoin::BlockHash {
        use bitcoin::hashes::Hash as _;
        bitcoin::BlockHash::from_byte_array(*hash)
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

    async fn coinbase_outputs(
        &self,
        block_hash: &[u8; 32],
    ) -> Result<Option<Vec<CoinbaseOutput>>, ChainError> {
        use bitcoincore_rpc::RpcApi;
        let block = self
            .client
            .get_block(&Self::block_hash(block_hash))
            .map_err(|e| ChainError::Unavailable(e.to_string()))?;
        let coinbase = block
            .txdata
            .first()
            .ok_or_else(|| ChainError::Unavailable("block has no coinbase".into()))?;
        Ok(Some(
            coinbase
                .output
                .iter()
                .map(|o| CoinbaseOutput {
                    script: o.script_pubkey.as_bytes().to_vec(),
                    value: o.value.to_sat(),
                })
                .collect(),
        ))
    }

    async fn block_economics(
        &self,
        block_hash: &[u8; 32],
    ) -> Result<Option<BlockEconomics>, ChainError> {
        use bitcoincore_rpc::RpcApi;
        let hash = Self::block_hash(block_hash);
        let stats: serde_json::Value = self
            .client
            .call("getblockstats", &[json!(hash.to_string())])
            .map_err(|e| ChainError::Unavailable(e.to_string()))?;
        let subsidy = stats
            .get("subsidy")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| ChainError::Unavailable("getblockstats has no subsidy".into()))?;
        let fees = stats
            .get("totalfee")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| ChainError::Unavailable("getblockstats has no totalfee".into()))?;
        Ok(Some(BlockEconomics { subsidy, fees }))
    }

    fn authoritative(&self) -> bool {
        true
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
/// sats are deliverable - `ark_due` when the front derived the per-block
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

/// Settlement accounting parameters (SPEC §4.1). `main` should pass these
/// from `[accounting]`/`[pool]` via [`PayoutService::with_accounting`]; the
/// defaults are the SPEC defaults so an unconfigured service still settles
/// deterministically.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccountingConfig {
    pub fee_bps: u16,
    pub window_n: u32,
    pub dust_limit_sats: u64,
    pub max_payout_slots: usize,
    pub budget_wu: u32,
    pub overhead_wu: u32,
}

impl Default for AccountingConfig {
    fn default() -> Self {
        Self {
            fee_bps: 100,
            window_n: 8,
            dust_limit_sats: 330,
            max_payout_slots: 256,
            budget_wu: 50_000,
            overhead_wu: 0,
        }
    }
}

/// How the mined coinbase was built (SPEC §3.3/§3.5). Detected from the
/// coinbase itself: JD mode always carries the `OP_RETURN "SPc"` additional
/// output (§2.4.2), pool-template mode carries the key in the scriptSig and
/// has no `OP_RETURN` output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplateMode {
    Pool,
    Jd,
}

#[derive(Debug, Error)]
pub enum ReconstructError {
    #[error("high-water vector is empty")]
    EmptyHighWater,
    #[error("multi-front high-water vectors are not supported ({0} fronts)")]
    MultiFrontHighWater(usize),
    #[error("high-water value is not a slice id")]
    HighWaterParse,
    #[error("no sealed slices for the window")]
    NoSlices,
    #[error("mined coinbase is empty")]
    EmptyCoinbase,
    #[error("the slice walk was truncated before the window closed")]
    WindowTruncated,
    #[error("cannot derive the network difficulty from the window slices")]
    NoDifficulty,
    #[error("invalid accounting parameter")]
    Config,
    #[error("accounting: {0}")]
    Accounting(#[from] spc_accounting::Error),
    #[error("snapshot has two entries for one id")]
    DuplicateEntry(SpId),
    #[error("window contains an id the distribution did not publish")]
    WindowMismatch,
    #[error("snapshot entry {0:?} has no payout key")]
    MissingPayoutKey(SpId),
    #[error("snapshot entry {0:?} has no mined coinbase output")]
    MissingCoinbaseOutput(SpId),
    #[error("coinbase output for {0:?} is not an entry on the coinbase rail")]
    PaidMismatch(SpId),
    #[error("mined coinbase does not pay the reconstructed coinbase tier")]
    CoinbaseTierMismatch,
    #[error("coinbase has duplicate payout scripts")]
    DuplicateScript,
    #[error("coinbase total {t_actual} differs from subsidy plus fees {expected}")]
    RevenueMismatch { t_actual: u64, expected: u64 },
    #[error("distribution weight is zero")]
    ZeroWeight,
    #[error("numeric value out of range")]
    Number,
    #[error("value exceeds i64")]
    Overflow,
}

/// Everything settlement needs, with no I/O, so the reconstruction is
/// testable against a known window (SPEC §7.4).
#[derive(Clone, Copy)]
pub struct ReconstructInput<'a> {
    pub height: u32,
    /// `distribution_snapshot.high_water` (§4.3).
    pub high_water: &'a BTreeMap<u8, String>,
    /// `distribution_snapshot.entries` (§4.7): identity metadata, not sats
    /// truth.
    pub entries: &'a [Entry],
    /// Sealed slices at or below the high-water mark, newest first.
    pub slices: &'a [SliceRow],
    /// True when the store returned its safety-limited slice walk, i.e.
    /// there may be older slices below the last row.
    pub window_truncated: bool,
    /// `pending_balance` before this block.
    pub pending: &'a BTreeMap<[u8; 32], u64>,
    /// The mined coinbase (SPEC §4.6).
    pub coinbase: &'a [CoinbaseOutput],
    pub economics: BlockEconomics,
    /// Ids with an `ark_delivery` row in Postgres right now (§4.6(f)).
    pub delivered: &'a BTreeSet<[u8; 32]>,
    pub cfg: AccountingConfig,
    /// `C` from the payout service configuration.
    pub coinbase_threshold_sats: u64,
}

/// The recomputed ledger for one block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reconstructed {
    pub mode: TemplateMode,
    pub ledger: Vec<LedgerRow>,
    pub pending_deltas: Vec<([u8; 32], i64)>,
    pub t_actual: u64,
    /// Slice ids actually selected, newest first (tests and logs).
    pub window: Vec<u64>,
    /// Recomputed `raw` per identity (tests and logs).
    pub raw: BTreeMap<SpId, u64>,
}

/// Reconstruct one block's ledger from the frozen window, the pending ledger
/// and the mined coinbase (SPEC §4.3, §4.6, §7.4). Fails closed: any
/// disagreement between the reconstruction and the mined distribution is an
/// error, never a guess.
///
/// The high-water mark is inclusive (SPEC §4.3). Older in-tree fronts record
/// the still-open finding slice instead of the last window slice; when the
/// inclusive window pays less than the reconstructed raw shares, the
/// reconstruction is retried without the slice the mark names. The mined
/// coinbase still has to agree, so this never books a guess.
pub fn reconstruct_settlement(
    input: &ReconstructInput<'_>,
) -> Result<Reconstructed, ReconstructError> {
    match reconstruct_once(input) {
        Err(ReconstructError::CoinbaseTierMismatch) => {
            let mark = single_front_high_water(input.high_water)?;
            let is_finding_slice = input
                .slices
                .first()
                .is_some_and(|s| s.slice_id == mark && s.height == input.height);
            if !is_finding_slice {
                return Err(ReconstructError::CoinbaseTierMismatch);
            }
            let rest: Vec<SliceRow> = input
                .slices
                .iter()
                .filter(|s| s.slice_id != mark)
                .cloned()
                .collect();
            let mut retry = *input;
            retry.slices = &rest;
            reconstruct_once(&retry)
        }
        other => other,
    }
}

fn reconstruct_once(input: &ReconstructInput<'_>) -> Result<Reconstructed, ReconstructError> {
    // The vector is inclusive: `high_water = {front: last slice IN the
    // window}` (SPEC §4.3), so every stored slice at or below it is a
    // candidate. The finding share lives in the still-open slice, whose id
    // is above the mark and is therefore absent.
    single_front_high_water(input.high_water)?;
    let expected = input
        .economics
        .subsidy
        .checked_add(input.economics.fees)
        .ok_or(ReconstructError::Overflow)?;
    let t_actual = input
        .coinbase
        .iter()
        .try_fold(0u64, |sum, o| sum.checked_add(o.value))
        .ok_or(ReconstructError::Overflow)?;
    if t_actual != expected {
        return Err(ReconstructError::RevenueMismatch { t_actual, expected });
    }
    let mode = detect_mode(input.coinbase);

    if input.slices.is_empty() {
        if !input.entries.is_empty() {
            return Err(ReconstructError::NoSlices);
        }
        // A block found before any slice sealed: the whole coinbase is the
        // pool output, nothing is owed.
        return Ok(Reconstructed {
            mode,
            ledger: Vec::new(),
            pending_deltas: Vec::new(),
            t_actual,
            window: Vec::new(),
            raw: BTreeMap::new(),
        });
    }

    let aggs = input
        .slices
        .iter()
        .map(slice_agg)
        .collect::<Result<Vec<_>, _>>()?;
    let target = window_target(input.slices, input.height, input.cfg.window_n)?;
    // `u64::MAX`: nothing is skipped. The stored slices at or below the
    // high-water mark already are the window's slice set (SPEC §4.3).
    let window = select_window(&aggs, u64::MAX, target, input.high_water.clone());
    if window.t == 0 {
        return Err(ReconstructError::NoSlices);
    }
    // A capped walk is only ambiguous when the window itself consumed every
    // returned slice; then there may be older rows it needed.
    if input.window_truncated && window.slices.len() == aggs.len() {
        return Err(ReconstructError::WindowTruncated);
    }
    let raw = raw_payouts(
        &window,
        input.economics.subsidy,
        input.economics.fees,
        input.cfg.fee_bps,
    )?;

    let mut entry_by_id: BTreeMap<SpId, &Entry> = BTreeMap::new();
    for e in input.entries {
        if entry_by_id.insert(e.sp_id, e).is_some() {
            return Err(ReconstructError::DuplicateEntry(e.sp_id));
        }
    }
    // The window was frozen into the distribution: every identity it credits
    // must have been published, or the reconstruction is not this block's.
    for id in raw.raw.keys() {
        if !entry_by_id.contains_key(id) {
            return Err(ReconstructError::WindowMismatch);
        }
    }

    let mut script_value: BTreeMap<&[u8], u64> = BTreeMap::new();
    for o in input.coinbase {
        if script_value.insert(o.script.as_slice(), o.value).is_some() {
            return Err(ReconstructError::DuplicateScript);
        }
    }
    let mut paid: BTreeMap<SpId, u64> = BTreeMap::new();
    for e in input.entries {
        if e.p_x.is_empty() {
            if e.rail == Rail::Coinbase {
                return Err(ReconstructError::MissingPayoutKey(e.sp_id));
            }
            continue;
        }
        let p_x: [u8; 32] = e
            .p_x
            .as_slice()
            .try_into()
            .map_err(|_| ReconstructError::MissingPayoutKey(e.sp_id))?;
        let script = p2tr_script(&p_x);
        if let Some(value) = script_value.get(script.as_slice()).copied() {
            if e.rail != Rail::Coinbase {
                return Err(ReconstructError::PaidMismatch(e.sp_id));
            }
            paid.insert(e.sp_id, value);
        }
    }

    // Per-identity amount owed: pool-template mode recomputes `raw + pending`
    // from the window; JD mode uses the published weight and rescales it by
    // the mined revenue (O11, SPEC §4.6).
    let mut amounts: BTreeMap<SpId, u64> = BTreeMap::new();
    // The forfeitable (this block's raw) portion per id, from the accounting
    // routing. Prior pending is never forfeited (SPEC §11.1).
    let mut forfeited_now: BTreeMap<SpId, u64> = BTreeMap::new();
    match mode {
        TemplateMode::Pool => {
            for e in input.entries {
                let before = input.pending.get(&e.sp_id).copied().unwrap_or(0);
                let raw_sats = raw.raw.get(&e.sp_id).copied().unwrap_or(0);
                let owed = raw_sats
                    .checked_add(before)
                    .ok_or(ReconstructError::Overflow)?;
                // The distribution's `sats` is metadata: it was frozen from
                // the front's pending forecast, which normally equals the
                // booked ledger by the time this block is `confirmation_depth`
                // deep. A difference is not fatal (the mined coinbase still
                // says what was actually paid), but it is worth a line.
                if e.sats != owed {
                    tracing::warn!(
                        sp_id = %hex::encode(e.sp_id),
                        event_sats = e.sats,
                        reconstructed = owed,
                        "distribution entry differs from slices + booked pending"
                    );
                }
                amounts.insert(e.sp_id, owed);
            }
            let mut caps = BTreeMap::new();
            for e in input.entries {
                caps.insert(
                    e.sp_id,
                    ArkCapability {
                        registered: input.delivered.contains(&e.sp_id),
                        dest_known: !e.dest_id.is_empty(),
                    },
                );
            }
            let tier = tier_config(input);
            let routing = route_tiers(&raw, input.pending, &caps, &tier, t_actual)?;
            forfeited_now = routing.forfeited.iter().copied().collect();
            let expected_kept: BTreeSet<SpId> = routing.kept.iter().map(|(id, _)| *id).collect();
            let paid_ids: BTreeSet<SpId> = paid.keys().copied().collect();
            // The mined coinbase is the truth. The routing set can differ
            // when pending moved between the distribution build and this
            // booking, so only a reconstruction that would over-credit the
            // window is fatal.
            if expected_kept != paid_ids {
                tracing::warn!(
                    expected = expected_kept.len(),
                    mined = paid_ids.len(),
                    "tier routing differs from the mined coinbase"
                );
            }
            for e in input.entries {
                if e.rail == Rail::Coinbase {
                    let value = paid
                        .get(&e.sp_id)
                        .ok_or(ReconstructError::MissingCoinbaseOutput(e.sp_id))?;
                    let raw_sats = raw.raw.get(&e.sp_id).copied().unwrap_or(0);
                    // `route_tiers` can pay only what the coinbase carries
                    // when one balance exceeds the block reward: `paid` is
                    // then `raw` plus part of the carried pending, and the
                    // rest stays pending. Fail closed on a coinbase that
                    // pays less than this block's raw share or more than
                    // the frozen distribution entry.
                    if *value < raw_sats || *value > e.sats {
                        return Err(ReconstructError::CoinbaseTierMismatch);
                    }
                } else if paid.contains_key(&e.sp_id) {
                    return Err(ReconstructError::PaidMismatch(e.sp_id));
                }
            }
        }
        TemplateMode::Jd => {
            let weight = jd_weight(input.entries, &raw)?;
            let mut payout_scripts = BTreeSet::new();
            for e in input.entries {
                if let Ok(p_x) = <[u8; 32]>::try_from(e.p_x.as_slice()) {
                    payout_scripts.insert(p2tr_script(&p_x));
                }
            }
            let fixed = fixed_outputs(input.coinbase, &paid, t_actual, &payout_scripts)?;
            for e in input.entries {
                let claim = claim_jd(e.sats, t_actual, fixed, weight)?;
                amounts.insert(e.sp_id, claim);
            }
        }
    }

    let mut ledger = Vec::with_capacity(input.entries.len());
    let mut deltas = Vec::new();
    for e in input.entries {
        let id = e.sp_id;
        let before = input.pending.get(&id).copied().unwrap_or(0);
        let amount = amounts.get(&id).copied().unwrap_or(0);
        let row = if let Some(value) = paid.get(&id) {
            // The mined coinbase paid this identity.
            let after = amount.saturating_sub(*value);
            LedgerRow {
                sp_id: id,
                rail: LedgerRail::Coinbase,
                paid_sats: *value,
                ark_due_sats: 0,
                forfeited_sats: 0,
                pending_delta: delta_of(after, before)?,
            }
        } else if e.rail == Rail::Coinbase {
            match mode {
                // Pool-template mode builds the coinbase from the same
                // routing: a kept id with no output is a broken distribution.
                TemplateMode::Pool => return Err(ReconstructError::MissingCoinbaseOutput(id)),
                // JD mode may dust-prune a kept id; the whole claim is
                // carried pending (unsigned, SPEC §4.6 O11).
                TemplateMode::Jd => LedgerRow {
                    sp_id: id,
                    rail: LedgerRail::Pending,
                    paid_sats: 0,
                    ark_due_sats: 0,
                    forfeited_sats: 0,
                    pending_delta: delta_of(amount, before)?,
                },
            }
        } else {
            let rail = settlement_rail(
                amount,
                input.delivered.contains(&id),
                !e.dest_id.is_empty(),
                input.coinbase_threshold_sats,
            );
            let (ark_due, forfeited, after) = match rail {
                LedgerRail::Ark => (amount, 0, 0),
                LedgerRail::Pending => (0, 0, amount),
                LedgerRail::Forfeit => {
                    // SPEC §11.1: only this block's raw earnings are
                    // confiscated; a prior pending balance keeps waiting.
                    // Prefer the accounting routing's forfeit split, and
                    // otherwise derive the raw portion for this id.
                    let forfeitable = forfeited_now
                        .get(&id)
                        .copied()
                        .unwrap_or_else(|| raw.raw.get(&id).copied().unwrap_or(0));
                    (0, forfeitable, before)
                }
                LedgerRail::Coinbase => return Err(ReconstructError::PaidMismatch(id)),
            };
            LedgerRow {
                sp_id: id,
                rail,
                paid_sats: 0,
                ark_due_sats: ark_due,
                forfeited_sats: forfeited,
                pending_delta: delta_of(after, before)?,
            }
        };
        if row.pending_delta != 0 {
            deltas.push((id, row.pending_delta));
        }
        ledger.push(row);
    }

    // Kept ids are the ones the mined coinbase paid; the routing set is a
    // cross-check for pool-template mode (done above).

    Ok(Reconstructed {
        mode,
        ledger,
        pending_deltas: deltas,
        t_actual,
        window: window.slices.iter().map(|s| s.slice_id).collect(),
        raw: raw.raw,
    })
}

/// Single-front high-water: the only vector the `slice` table can resolve
/// exactly (it has no `front_id` column). A multi-front vector fails closed
/// instead of mixing per-front `slice_id` sequences.
fn single_front_high_water(high_water: &BTreeMap<u8, String>) -> Result<u64, ReconstructError> {
    match high_water.len() {
        0 => Err(ReconstructError::EmptyHighWater),
        1 => high_water
            .values()
            .next()
            .ok_or(ReconstructError::EmptyHighWater)?
            .parse::<u64>()
            .map_err(|_| ReconstructError::HighWaterParse),
        n => Err(ReconstructError::MultiFrontHighWater(n)),
    }
}

fn detect_mode(coinbase: &[CoinbaseOutput]) -> TemplateMode {
    if coinbase.iter().any(|o| o.script.first() == Some(&0x6a)) {
        TemplateMode::Jd
    } else {
        TemplateMode::Pool
    }
}

/// Network difficulty at the finding block, as `Σd / n_shares` from a sealed
/// slice (every share under one template carries the same `d`, so the
/// division is exact). A slice sealed at the block's own height is the
/// finding template and is preferred; the newest stored slice is the fallback
/// when the finding slice has not sealed yet.
///
/// A difficulty retarget between the fallback slice and the finding block
/// makes this estimate wrong (reviewer #12). The exact fix is to persist the
/// window target (or the finding `d`) in `distribution_snapshot`; see the
/// migration noted in the report.
fn window_target(
    slices: &[SliceRow],
    finding_height: u32,
    window_n: u32,
) -> Result<u128, ReconstructError> {
    if window_n == 0 {
        return Err(ReconstructError::Config);
    }
    let reference = slices
        .iter()
        .find(|s| s.height == finding_height)
        .or_else(|| slices.first())
        .ok_or(ReconstructError::NoSlices)?;
    if reference.n_shares == 0 {
        return Err(ReconstructError::NoDifficulty);
    }
    let difficulty = reference
        .difficulty
        .parse::<u128>()
        .map_err(|_| ReconstructError::Number)?;
    let share_difficulty = difficulty / u128::from(reference.n_shares);
    if share_difficulty == 0 {
        return Err(ReconstructError::NoDifficulty);
    }
    Ok(u128::from(window_n).saturating_mul(share_difficulty))
}

fn slice_agg(row: &SliceRow) -> Result<SliceAgg, ReconstructError> {
    let difficulty = row
        .difficulty
        .parse::<u128>()
        .map_err(|_| ReconstructError::Number)?;
    let mut per_id: BTreeMap<SpId, (u128, u128)> = BTreeMap::new();
    let mut diff_fee_sum = 0u128;
    for c in &row.credits {
        let d = c
            .diff_sum
            .parse::<u128>()
            .map_err(|_| ReconstructError::Number)?;
        let df = c
            .diff_fee_sum
            .parse::<u128>()
            .map_err(|_| ReconstructError::Number)?;
        diff_fee_sum = diff_fee_sum.saturating_add(df);
        let slot = per_id.entry(c.sp_id).or_insert((0, 0));
        slot.0 = slot.0.saturating_add(d);
        slot.1 = slot.1.saturating_add(df);
    }
    Ok(SliceAgg {
        slice_id: row.slice_id,
        height: row.height,
        prev_hash: row.prev_hash,
        // The slice table has no front column; single-front only.
        front_id: 0,
        difficulty,
        diff_fee_sum,
        per_id,
        root: row.root,
    })
}

/// O11: `W = Σ weight_id + weight_P`. `weight_P` is the pool fee weight.
/// The fee weight is reconstructed at the mined revenue (the pool template's
/// own fee split); exact when the JDC's template revenue equals the pool's.
fn jd_weight(entries: &[Entry], raw: &RawPayouts) -> Result<u64, ReconstructError> {
    let owed: u128 = entries.iter().map(|e| u128::from(e.sats)).sum();
    let weight = owed.saturating_add(u128::from(raw.pool_sats));
    if weight == 0 {
        return Err(ReconstructError::ZeroWeight);
    }
    u64::try_from(weight).map_err(|_| ReconstructError::Overflow)
}

/// PR #203 §4 fixed-output total as it appears on chain (SPEC §3.5).
///
/// The mandated order is `[pool_payout, payouts…, additional_outputs,
/// JDC/TP outputs]`, so the pool payout is the first mined output and every
/// other output that is not a snapshot payout is fixed. Summing only
/// `OP_RETURN` scripts would miss a non-OP_RETURN `additional_outputs` entry
/// and the JDC's own outputs; derive the total by subtraction instead:
/// `F = T_actual - Σ paid - pool_output`.
fn fixed_outputs(
    coinbase: &[CoinbaseOutput],
    paid: &BTreeMap<SpId, u64>,
    t_actual: u64,
    payout_scripts: &BTreeSet<Vec<u8>>,
) -> Result<u64, ReconstructError> {
    let pool = coinbase.first().ok_or(ReconstructError::EmptyCoinbase)?;
    if payout_scripts.contains(&pool.script) {
        // Output[0] must be the pool payout, never a miner payout.
        return Err(ReconstructError::CoinbaseTierMismatch);
    }
    let paid_sum = paid
        .values()
        .try_fold(0u64, |sum, value| sum.checked_add(*value))
        .ok_or(ReconstructError::Overflow)?;
    t_actual
        .checked_sub(paid_sum)
        .and_then(|rest| rest.checked_sub(pool.value))
        .ok_or(ReconstructError::CoinbaseTierMismatch)
}

fn tier_config(input: &ReconstructInput<'_>) -> TierConfig {
    TierConfig {
        fee_bps: input.cfg.fee_bps,
        dust_limit_sats: input.cfg.dust_limit_sats,
        coinbase_threshold_sats: input.coinbase_threshold_sats,
        max_payout_slots: input.cfg.max_payout_slots,
        budget_wu: input.cfg.budget_wu,
        overhead_wu: input.cfg.overhead_wu,
    }
}

fn delta_of(after: u64, before: u64) -> Result<i64, ReconstructError> {
    let delta = i128::from(after) - i128::from(before);
    i64::try_from(delta).map_err(|_| ReconstructError::Overflow)
}

/// Old-attribution scrub knobs (SPEC §4.1, §5.1). Defaults are the SPEC
/// defaults; `main` should pass `[accounting] retain_windows` and
/// `scrub_after_blocks` via [`PayoutService::with_scrub`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScrubConfig {
    pub retain_windows: u32,
    pub scrub_after_blocks: u32,
}

impl Default for ScrubConfig {
    fn default() -> Self {
        Self {
            retain_windows: 16,
            scrub_after_blocks: 100,
        }
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
    accounting: AccountingConfig,
    scrub: ScrubConfig,
    /// `sp_id -> identity.network` code to book; `None` keeps the historical
    /// fallbacks until `main` passes the configured network.
    network: Option<i16>,
    orders_stream: String,
    receipts_stream: String,
    front_events_stream: String,
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
            accounting: AccountingConfig::default(),
            scrub: ScrubConfig::default(),
            network: None,
            orders_stream: "spc:ark:orders".to_string(),
            receipts_stream: "spc:ark:receipts".to_string(),
            front_events_stream: "spc:front:events".to_string(),
        }
    }

    /// Supply the configured accounting parameters (SPEC §4.1). Settlement
    /// fails closed if the window/fee parameters differ from the front's.
    pub fn with_accounting(mut self, accounting: AccountingConfig) -> Self {
        self.accounting = accounting;
        self
    }

    /// Supply the scrub retention knobs (SPEC §4.1, §5.1).
    pub fn with_scrub(mut self, scrub: ScrubConfig) -> Self {
        self.scrub = scrub;
        self
    }

    /// Supply the configured network code (0 mainnet, 1 testnet4, 2 signet,
    /// 3 regtest) used for identity rows. Without it the historical
    /// registration/placeholder fallbacks are kept.
    pub fn with_network(mut self, network: i16) -> Self {
        self.network = Some(network);
        self
    }

    /// Override the Ark stream names (tests; production uses SPEC §5.2 names).
    pub fn with_ark_streams(
        mut self,
        orders_stream: impl Into<String>,
        receipts_stream: impl Into<String>,
    ) -> Self {
        self.orders_stream = orders_stream.into();
        self.receipts_stream = receipts_stream.into();
        self
    }

    /// Override the `spc:front:events` stream name (tests).
    pub fn with_front_events_stream(mut self, stream: impl Into<String>) -> Self {
        self.front_events_stream = stream.into();
        self
    }

    pub async fn run(&self) -> Result<(), PayoutError> {
        self.store.migrate().await?;
        for s in ["spc:slices", "spc:blocks", "spc:arkreg"] {
            self.streams.ensure_group(s, GROUP).await?;
        }
        // Receipts are idempotent, so start a first-created group at 0:
        // a receipt emitted before this payout process started must not be
        // lost.
        self.streams
            .ensure_group_from(&self.receipts_stream, GROUP, "0")
            .await?;
        // Held block events wait in the consumer group's pending list; run
        // the retry pass once at startup so restarts pick them up, then on
        // the interval.
        let mut last_pending = Instant::now()
            .checked_sub(PENDING_RETRY_INTERVAL)
            .unwrap_or_else(Instant::now);
        // Startup passes: pick up scheduled Ark sends and scrub old
        // attribution after a restart, before waiting for the first interval.
        let mut last_emit = Instant::now()
            .checked_sub(ARK_EMIT_INTERVAL)
            .unwrap_or_else(Instant::now);
        let mut last_scrub = Instant::now()
            .checked_sub(SCRUB_INTERVAL)
            .unwrap_or_else(Instant::now);
        let mut last_republish = Instant::now()
            .checked_sub(REPUBLISH_INTERVAL)
            .unwrap_or_else(Instant::now);
        loop {
            // Receipts are the only path that resolves a scheduled Ark send.
            if let Err(e) = self.process_receipts_once().await {
                tracing::error!(error = %e, "ark receipt processing failed");
            }
            if last_emit.elapsed() >= ARK_EMIT_INTERVAL {
                last_emit = Instant::now();
                if let Err(e) = self.emit_due_ark_orders().await {
                    tracing::error!(error = %e, "ark order emission failed");
                }
            }
            if last_scrub.elapsed() >= SCRUB_INTERVAL {
                last_scrub = Instant::now();
                if let Err(e) = self.scrub_once().await {
                    tracing::error!(error = %e, "attribution scrub failed");
                }
            }
            if last_republish.elapsed() >= REPUBLISH_INTERVAL {
                last_republish = Instant::now();
                if let Err(e) = self.republish_ledger_events().await {
                    tracing::error!(error = %e, "ledger republish failed");
                }
            }
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
        // stream carries only `sp_id` - no keys. A miner that has not
        // registered Ark delivery yet gets a placeholder identity row here;
        // the registration upsert replaces it once keys arrive.
        let mut seen = std::collections::BTreeSet::new();
        for c in &e.credits {
            if seen.insert(c.sp_id) {
                self.store
                    .insert_identity_if_absent(&placeholder_identity(
                        c.sp_id,
                        identity_network(self.network, 3),
                    ))
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
        // `confirmation_depth` deep on the active chain. Any doubt - chain
        // view down, node not yet at the height, too shallow - leaves the
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
                    Ok(_)
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

        let booked_ledger = match (status, fingerprint) {
            (BlockStatus::Confirmed, Some(fingerprint)) => {
                match self.recompute(&e, fingerprint).await? {
                    Some(ledger) => Some(ledger),
                    None => return Ok(Disposition::Retry),
                }
            }
            _ => None,
        };
        let (ledger, deltas) = booked_ledger.unwrap_or_default();

        let settlement = Settlement {
            block_hash: e.block_hash,
            height: e.height,
            fingerprint,
            status,
            ledger,
        };
        let booked_rev = match self
            .store
            .settle(&settlement, self.confirmation_depth)
            .await
        {
            Ok(rev) => rev,
            Err(spc_store::postgres::StoreError::AlreadySettled(_)) => None,
            Err(err) => return Err(err.into()),
        };
        if status != BlockStatus::Confirmed {
            return Ok(Disposition::Done);
        }
        // The ledger booking and its revision committed atomically inside
        // `settle` (SPEC §7.4). A crash before the XADD leaves the DB state
        // that bootstrap reflects, and the republish timer re-emits the
        // delta; the front dedupes by block hash.
        let Some(rev) = booked_rev else {
            return Ok(Disposition::Done);
        };
        let event = FrontEvent::LedgerApplied {
            rev,
            block_hash: e.block_hash,
            republish: true,
            pending_deltas: deltas,
        };
        if let Ok(bytes) = event.encode() {
            let _ = self.streams.add(&self.front_events_stream, &bytes).await;
        }
        // The Ark rows are now in Postgres. Emit from the store, not from
        // this in-memory ledger, so a payout restart and a duplicate booking
        // take the same idempotent path (SPEC §11.3).
        if let Err(e) = self.emit_due_ark_orders().await {
            tracing::warn!(error = %e, "ark order emission failed; the timer retries");
        }
        Ok(Disposition::Done)
    }

    /// Recompute one block's ledger (SPEC §7.4). `None` means hold: the
    /// reconstruction cannot be trusted yet and the event stays unacked.
    async fn recompute(
        &self,
        e: &BlockEvent,
        fingerprint: [u8; 32],
    ) -> Result<Option<(Vec<LedgerRow>, Vec<([u8; 32], i64)>)>, PayoutError> {
        let coinbase = match self.chain.coinbase_outputs(&e.block_hash).await {
            Ok(outputs) => outputs,
            Err(err) => {
                tracing::warn!(error = %err, block = %display_hash(&e.block_hash), "cannot read the mined coinbase; holding settlement");
                return Ok(None);
            }
        };
        let economics = match self.chain.block_economics(&e.block_hash).await {
            Ok(economics) => economics,
            Err(err) => {
                tracing::warn!(error = %err, block = %display_hash(&e.block_hash), "cannot read block economics; holding settlement");
                return Ok(None);
            }
        };

        match (coinbase, economics) {
            (Some(coinbase), Some(economics)) => {
                let Some(snapshot) = self.store.snapshot(&fingerprint).await? else {
                    tracing::error!("distribution snapshot vanished; holding settlement");
                    return Ok(None);
                };
                let high_water: BTreeMap<u8, String> = serde_json::from_value(snapshot.high_water)
                    .map_err(|_| {
                        PayoutError::Snapshot("high_water is not a front -> slice id map")
                    })?;
                let entries: Vec<Entry> = serde_json::from_value(snapshot.entries)
                    .map_err(|_| PayoutError::Snapshot("entries is not an entry list"))?;
                let high_water_mark = match single_front_high_water(&high_water) {
                    Ok(id) => id,
                    Err(err) => {
                        tracing::error!(error = %err, "cannot reconstruct the paid window; holding settlement");
                        return Ok(None);
                    }
                };
                let slices = self
                    .store
                    .window_slices_to(high_water_mark, WINDOW_SLICE_LIMIT)
                    .await?;
                // `slice` rows are never pruned, so the walk is capped. That
                // is only ambiguous when the window itself consumes every
                // returned row (checked by `reconstruct_settlement`).
                let window_truncated =
                    i64::try_from(slices.len()).unwrap_or(i64::MAX) >= WINDOW_SLICE_LIMIT;
                let pending = self.store.pending_map().await?;
                let ids: Vec<[u8; 32]> = entries.iter().map(|entry| entry.sp_id).collect();
                let delivered = self.store.ark_delivery_ids(&ids).await?;
                let input = ReconstructInput {
                    height: e.height,
                    high_water: &high_water,
                    entries: &entries,
                    slices: &slices,
                    window_truncated,
                    pending: &pending,
                    coinbase: &coinbase,
                    economics,
                    delivered: &delivered,
                    cfg: self.accounting,
                    coinbase_threshold_sats: self.coinbase_threshold_sats,
                };
                match reconstruct_settlement(&input) {
                    Ok(reconstructed) => {
                        Ok(Some((reconstructed.ledger, reconstructed.pending_deltas)))
                    }
                    Err(err) => {
                        tracing::error!(
                            error = %err,
                            block = %display_hash(&e.block_hash),
                            height = e.height,
                            "settlement reconstruction failed; holding"
                        );
                        Ok(None)
                    }
                }
            }
            _ if self.chain.authoritative() => {
                tracing::error!(
                    block = %display_hash(&e.block_hash),
                    "authoritative chain view has no mined coinbase; holding settlement"
                );
                Ok(None)
            }
            _ => {
                tracing::warn!(
                    block = %display_hash(&e.block_hash),
                    "no authoritative chain view: booking from the block event's entries"
                );
                self.legacy_ledger(e).await.map(Some)
            }
        }
    }

    /// Dev/test fallback: book from the event's entries when no chain view
    /// can supply the coinbase. Never used by an authoritative view.
    async fn legacy_ledger(
        &self,
        e: &BlockEvent,
    ) -> Result<(Vec<LedgerRow>, Vec<([u8; 32], i64)>), PayoutError> {
        let mut ledger = Vec::with_capacity(e.entries.len());
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
                    let delta = i64::try_from(entry.sats).map_err(|_| PayoutError::Overflow)?;
                    deltas.push((entry.sp_id, delta));
                    LedgerRow {
                        sp_id: entry.sp_id,
                        rail: LedgerRail::Pending,
                        paid_sats: 0,
                        ark_due_sats: 0,
                        forfeited_sats: 0,
                        pending_delta: delta,
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
        Ok((ledger, deltas))
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
                network: identity_network(self.network, 1),
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
        // Registration is durable above; bump then publish.
        let rev = self.store.bump_front_rev().await?;
        let event = FrontEvent::ArkRegApplied {
            rev,
            sp_id: e.sp_id,
        };
        if let Ok(bytes) = event.encode() {
            let _ = self.streams.add(&self.front_events_stream, &bytes).await;
        }
        Ok(())
    }

    /// Re-emit `LedgerApplied` for the most recent confirmed blocks
    /// (SPEC §5.2, §7.4).
    ///
    /// `on_block` emits once after the atomic settle/rev commit, but the XADD
    /// can fail or the event can be lost. This periodic replay is the
    /// self-healing path; the front seeds its block-hash dedupe from
    /// `bootstrap.applied_blocks` and ignores a block it already applied, so
    /// a duplicate republish is harmless.
    async fn republish_ledger_events(&self) -> Result<(), PayoutError> {
        let recent = self
            .store
            .recent_ledger_events(spc_store::postgres::APPLIED_BLOCKS_LIMIT)
            .await?;
        for (block_hash, pending_deltas) in recent {
            let rev = self.store.bump_front_rev().await?;
            let event = FrontEvent::LedgerApplied {
                rev,
                block_hash,
                republish: true,
                pending_deltas,
            };
            let Ok(bytes) = event.encode() else {
                continue;
            };
            if let Err(e) = self.streams.add(&self.front_events_stream, &bytes).await {
                tracing::warn!(
                    error = %e,
                    block = %display_hash(&block_hash),
                    "ledger republish XADD failed; the next pass retries"
                );
            }
        }
        Ok(())
    }

    /// Re-emit every due `ark_payout` row as a signed order (SPEC §11.3).
    ///
    /// Store-driven, so it is safe to call after a fresh booking, after a
    /// restart, or from the timer: the order id is deterministic per
    /// `(block_hash, sp_id)` and `ark` dedupes it. A row that is not due yet
    /// is left alone; a row with no destination or over the cap is skipped
    /// with a warning (the row stays scheduled).
    async fn emit_due_ark_orders(&self) -> Result<(), PayoutError> {
        let emissions = self.store.scheduled_ark_emissions().await?;
        if emissions.is_empty() {
            return Ok(());
        }
        let Some(key) = &self.order_key else {
            tracing::warn!(
                count = emissions.len(),
                "scheduled Ark payouts exist but no order-signing key is configured"
            );
            return Ok(());
        };
        let tip = match self.chain.tip_height().await {
            Ok(tip) => tip,
            Err(e) => {
                tracing::warn!(error = %e, "chain view unavailable; not emitting ark orders yet");
                return Ok(());
            }
        };
        for row in emissions {
            // `due_height` is H_f + confirmation_depth (settle(), SPEC §7.4).
            // The chain gate books at tip >= H_f + depth - 1, so the first
            // emission can be one confirmation early; `ark` holds it until
            // the tip reaches `not_before_height`.
            if row.due_height > tip {
                continue;
            }
            if row.dest_id.is_empty() {
                tracing::warn!(
                    block = %display_hash(&row.block_hash),
                    "Ark payout has no destination; not emitted"
                );
                continue;
            }
            if self.max_order_sats > 0 && row.sats > self.max_order_sats {
                tracing::warn!(
                    sats = row.sats,
                    "Ark order exceeds max_order_sats; not emitted"
                );
                continue;
            }
            let Ok(address) = String::from_utf8(row.dest_id) else {
                tracing::warn!("Ark destination is not UTF-8; not emitted");
                continue;
            };
            let mut order = ArkSendOrder {
                order_id: ark_order_id(&row.block_hash, &row.sp_id),
                ark_address: address,
                sats: row.sats,
                not_before_height: row.due_height,
                sig: [0u8; 64],
            };
            if let Err(e) = order.sign(key) {
                tracing::error!(error = %e, "Ark order signing failed");
                continue;
            }
            let bytes = match order.encode() {
                Ok(bytes) => bytes,
                Err(e) => {
                    tracing::error!(error = %e, "Ark order encode failed");
                    continue;
                }
            };
            self.streams.add(&self.orders_stream, &bytes).await?;
        }
        Ok(())
    }

    /// Consume `spc:ark:receipts` and close out scheduled payouts
    /// (SPEC §11.3). `Sent` records the VTXO and never credits pending;
    /// `Failed` moves the sats to pending, never to forfeit.
    async fn process_receipts_once(&self) -> Result<(), PayoutError> {
        let entries = self
            .streams
            .read_group(&self.receipts_stream, GROUP, CONSUMER, 64)
            .await?;
        for (id, payload) in entries {
            let receipt = match ArkSendReceipt::decode(&payload) {
                Ok(receipt) => receipt,
                Err(e) => {
                    tracing::warn!(error = %e, "undecodable ark receipt dropped");
                    self.streams.ack(&self.receipts_stream, GROUP, &id).await?;
                    continue;
                }
            };
            if let Err(e) = self.apply_receipt(&receipt).await {
                // Leave the receipt unacked: the store update failed and is
                // retried on the next pass.
                tracing::error!(error = %e, "ark receipt not applied");
                continue;
            }
            self.streams.ack(&self.receipts_stream, GROUP, &id).await?;
        }
        Ok(())
    }

    /// Apply one receipt to `ark_payout`. Matching is by the deterministic
    /// order id, so a receipt for an already-resolved row is a no-op.
    async fn apply_receipt(&self, receipt: &ArkSendReceipt) -> Result<(), PayoutError> {
        let emissions = self.store.scheduled_ark_emissions().await?;
        let Some(row) = emissions
            .iter()
            .find(|row| ark_order_id(&row.block_hash, &row.sp_id) == receipt.order_id)
        else {
            tracing::debug!(
                order = %hex::encode(receipt.order_id),
                "ark receipt for an order that is no longer scheduled"
            );
            return Ok(());
        };
        match receipt.status {
            ReceiptStatus::Sent => {
                self.store
                    .mark_ark_sent(
                        &row.block_hash,
                        &row.sp_id,
                        &receipt.vtxo_id,
                        receipt.expiry_height,
                    )
                    .await?;
            }
            ReceiptStatus::Failed => {
                self.store
                    .mark_ark_failed_to_pending(&row.block_hash, &row.sp_id)
                    .await?;
            }
        }
        // The receipt resolves the ambiguous state; allow a future emit of
        // this order id to be checked normally again.
        if let Err(e) = self
            .streams
            .srem(SeenOrders::AMBIGUOUS_KEY, &receipt.order_id)
            .await
        {
            tracing::warn!(error = %e, "could not clear the ambiguous ark order pin");
        }
        Ok(())
    }

    /// Scrub old attribution data (SPEC §5.1). Only an authoritative chain
    /// view has a meaningful tip; `AlwaysConfirm` reports `u32::MAX` and
    /// would scrub everything.
    async fn scrub_once(&self) -> Result<(), PayoutError> {
        if !self.chain.authoritative() {
            return Ok(());
        }
        let tip = match self.chain.tip_height().await {
            Ok(tip) => tip,
            Err(e) => {
                tracing::warn!(error = %e, "chain view unavailable; not scrubbing");
                return Ok(());
            }
        };
        // `retain_windows` is in blocks, but an unbooked block at the
        // confirmation edge can still need the `N` blocks of window behind
        // it. Keep enough credits for `confirmation_depth + window_n` blocks
        // (rounded up) even if the configured retention is smaller: under-
        // retention is a silent settlement hold, over-retention only costs
        // storage.
        let retain = scrub_retain_windows(
            self.scrub.retain_windows,
            self.confirmation_depth,
            self.accounting.window_n,
        );
        self.store
            .scrub(tip, retain, self.scrub.scrub_after_blocks)
            .await?;
        Ok(())
    }
}

/// Deterministic `order_id` for an `ark_payout` row. The same
/// `(block_hash, sp_id)` always yields the same id, so a payout restart or a
/// timer re-emit is rejected by `ark`'s dedupe instead of paying twice
/// (SPEC §11.3).
pub fn ark_order_id(block_hash: &[u8; 32], sp_id: &[u8; 32]) -> [u8; 16] {
    use bitcoin::hashes::{sha256, Hash, HashEngine};
    let mut engine = sha256::Hash::engine();
    engine.input(b"SPc/ArkOrderId");
    engine.input(block_hash);
    engine.input(sp_id);
    let digest = sha256::Hash::from_engine(engine).to_byte_array();
    let mut id = [0u8; 16];
    id.copy_from_slice(&digest[..16]);
    id
}

#[derive(Clone, Copy)]
enum Kind {
    Slice,
    Block,
    Reg,
}

/// Conservative `slice_credit` retention in blocks. A block at the
/// confirmation edge (`tip - confirmation_depth + 2`) can still be unbooked
/// and needs a window of `window_n` blocks behind it; keep a few extra.
fn scrub_retain_windows(configured: u32, confirmation_depth: u32, window_n: u32) -> u32 {
    configured.max(
        confirmation_depth
            .saturating_add(window_n)
            .saturating_add(1),
    )
}

#[cfg(test)]
fn now_rev() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
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
/// Ark delivery (SPEC §5.1). The keys are all-zero 33-byte blobs - a
/// compressed secp256k1 public key never serializes to all zeros, so no
/// real registration can collide with or be mistaken for a placeholder.
/// The real `b_scan`/`b_spend` (and network) arrive later via the
/// registration path (`on_reg`), whose upsert replaces this row.
///
/// The network code is provisional: the slice stream carries no network,
/// and the value is overwritten at registration. 3 (regtest) keeps the
/// placeholder distinguishable from the testnet4 code the registration
/// path currently books.
fn placeholder_identity(sp_id: [u8; 32], network: i16) -> SpIdentityRow {
    SpIdentityRow {
        sp_id,
        b_scan: vec![0u8; 33],
        b_spend: vec![0u8; 33],
        network,
    }
}

/// The configured identity network code, or the historical fallback when the
/// service was not given one (main.rs hook).
fn identity_network(configured: Option<i16>, fallback: i16) -> i16 {
    configured.unwrap_or(fallback)
}

/// Convenience for tests: a pending map as the `LedgerApplied` payload.
pub fn deltas_map(deltas: Vec<([u8; 32], i64)>) -> BTreeMap<[u8; 32], i64> {
    deltas.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(byte: u8) -> [u8; 32] {
        [byte; 32]
    }

    fn slice_row(
        slice_id: u64,
        height: u32,
        difficulty: u128,
        n_shares: u32,
        credits: &[([u8; 32], u128, u128)],
    ) -> SliceRow {
        SliceRow {
            slice_id,
            height,
            prev_hash: [0x11; 32],
            ref_job_id: 0,
            ref_fees: 0,
            n_shares,
            difficulty: difficulty.to_string(),
            root: [0u8; 32],
            shares: Vec::new(),
            credits: credits
                .iter()
                .map(|(sp_id, d, df)| SliceCreditRow {
                    sp_id: *sp_id,
                    diff_sum: d.to_string(),
                    diff_fee_sum: df.to_string(),
                })
                .collect(),
        }
    }

    fn entry(sp_id: [u8; 32], rail: Rail, p_x: Option<[u8; 32]>, sats: u64) -> Entry {
        Entry {
            sp_id,
            rail,
            p_x: p_x.map(|x| x.to_vec()).unwrap_or_default(),
            dest_id: Vec::new(),
            sats,
        }
    }

    fn output(script: Vec<u8>, value: u64) -> CoinbaseOutput {
        CoinbaseOutput { script, value }
    }

    fn pool_script() -> Vec<u8> {
        spc_accounting::p2tr_script(&[0x99; 32])
    }

    fn base_cfg(window_n: u32) -> AccountingConfig {
        AccountingConfig {
            fee_bps: 0,
            window_n,
            dust_limit_sats: 30,
            max_payout_slots: 256,
            budget_wu: 100_000,
            overhead_wu: 0,
        }
    }

    fn high_water(front: u8, slice_id: &str) -> BTreeMap<u8, String> {
        BTreeMap::from([(front, slice_id.to_string())])
    }

    #[test]
    fn settlement_rail_rechecks_delivery_at_settlement() {
        // Delivery committed and the destination reached the front: the Ark
        // send can be built (§11.1 branch 3).
        assert_eq!(settlement_rail(75, true, true, 330), LedgerRail::Ark);
        assert_eq!(settlement_rail(250_000, true, true, 330), LedgerRail::Ark);
        // Delivery committed but the destination never reached the building
        // front: §11.1 branch 4 - pending, never forfeit (the destination is
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
        // The node has not reached the height: hold - never orphan on an
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
        assert_eq!(view.coinbase_outputs(&hash).await.unwrap(), None);
        assert_eq!(view.block_economics(&hash).await.unwrap(), None);
        assert!(!view.authoritative());
    }

    /// SPEC §4.3/§7.4: exactly the slices at or below the high-water mark
    /// that `select_window` takes, never the whole slice history.
    #[test]
    fn reconstruction_selects_exactly_the_high_water_window() {
        let alice = id(0xa1);
        let bob = id(0xb2);
        let carol = id(0xc3);
        let slices = vec![
            slice_row(3, 100, 100, 1, &[(alice, 60, 0)]),
            slice_row(2, 100, 100, 1, &[(bob, 40, 0)]),
            // Outside the N x D window even though it is below the mark.
            slice_row(1, 99, 100, 1, &[(carol, 100, 0)]),
        ];
        let entries = vec![
            entry(alice, Rail::Coinbase, Some(id(0x01)), 60),
            entry(bob, Rail::Coinbase, Some(id(0x02)), 40),
        ];
        let coinbase = vec![
            output(pool_script(), 100),
            output(spc_accounting::p2tr_script(&id(0x01)), 60),
            output(spc_accounting::p2tr_script(&id(0x02)), 40),
        ];
        let pending = BTreeMap::new();
        let delivered = BTreeSet::new();
        let high_water = high_water(0, "4");
        let input = ReconstructInput {
            height: 100,
            high_water: &high_water,
            entries: &entries,
            slices: &slices,
            window_truncated: false,
            pending: &pending,
            coinbase: &coinbase,
            economics: BlockEconomics {
                subsidy: 200,
                fees: 0,
            },
            delivered: &delivered,
            cfg: base_cfg(2),
            coinbase_threshold_sats: 30,
        };

        let reconstructed = reconstruct_settlement(&input).unwrap();
        assert_eq!(reconstructed.mode, TemplateMode::Pool);
        assert_eq!(reconstructed.window, vec![3, 2]);
        assert_eq!(reconstructed.raw.get(&alice), Some(&60));
        assert_eq!(reconstructed.raw.get(&bob), Some(&40));
        assert_eq!(reconstructed.raw.get(&carol), None);
        assert_eq!(reconstructed.t_actual, 200);
        assert_eq!(reconstructed.ledger.len(), 2);
        assert!(reconstructed
            .ledger
            .iter()
            .all(|row| row.rail == LedgerRail::Coinbase));
        assert!(reconstructed.pending_deltas.is_empty());
    }

    /// SPEC §4.6 (O11): the JDC paid at its own template revenue. The ledger
    /// books `claim - paid` from the mined coinbase, unsigned.
    #[test]
    fn jd_revenue_mismatch_books_claim_minus_paid_pending() {
        let alice = id(0xa1);
        let bob = id(0xb2);
        let slices = vec![slice_row(2, 200, 100, 1, &[(alice, 50, 0), (bob, 50, 0)])];
        // Weights were built at the pool's T = 400; the mined coinbase is
        // T_actual = 200, and the JDC paid 70/80 rather than the weights.
        let entries = vec![
            entry(alice, Rail::Coinbase, Some(id(0x01)), 200),
            entry(bob, Rail::Coinbase, Some(id(0x02)), 200),
        ];
        // SPEC §3.5 order: pool payout, then payouts, then additional outputs.
        let coinbase = vec![
            output(pool_script(), 50),
            output(spc_accounting::p2tr_script(&id(0x01)), 70),
            output(spc_accounting::p2tr_script(&id(0x02)), 80),
            output(vec![0x6a, 0x04, b'S', b'P', b'c'], 0),
        ];
        let pending = BTreeMap::new();
        let delivered = BTreeSet::new();
        let high_water = high_water(0, "3");
        let input = ReconstructInput {
            height: 200,
            high_water: &high_water,
            entries: &entries,
            slices: &slices,
            window_truncated: false,
            pending: &pending,
            coinbase: &coinbase,
            economics: BlockEconomics {
                subsidy: 200,
                fees: 0,
            },
            delivered: &delivered,
            cfg: base_cfg(2),
            coinbase_threshold_sats: 30,
        };

        let reconstructed = reconstruct_settlement(&input).unwrap();
        assert_eq!(reconstructed.mode, TemplateMode::Jd);
        assert_eq!(reconstructed.t_actual, 200);
        // The snapshot's weights sum to the pool's T, not the mined T.
        let weights: u64 = entries.iter().map(|e| e.sats).sum();
        assert_eq!(weights, 400);
        let deltas = deltas_map(reconstructed.pending_deltas);
        assert_eq!(deltas.get(&alice), Some(&30));
        assert_eq!(deltas.get(&bob), Some(&20));
        for row in &reconstructed.ledger {
            assert_eq!(row.rail, LedgerRail::Coinbase);
            // `claim - paid` is never negative (unsigned ledger).
            assert!(row.pending_delta >= 0);
        }
        let alice_row = reconstructed
            .ledger
            .iter()
            .find(|row| row.sp_id == alice)
            .unwrap();
        assert_eq!(alice_row.paid_sats, 70);
    }

    /// SPEC §4.6/§7.4: a JD id the mined coinbase dust-pruned carries its
    /// whole claim as pending, never a coinbase row with zero paid.
    #[test]
    fn jd_dust_pruned_id_is_carried_pending() {
        let alice = id(0xa1);
        let bob = id(0xb2);
        let slices = vec![slice_row(2, 100, 100, 1, &[(alice, 50, 0), (bob, 50, 0)])];
        let entries = vec![
            entry(alice, Rail::Coinbase, Some(id(0x01)), 200),
            entry(bob, Rail::Coinbase, Some(id(0x02)), 200),
        ];
        // Alice was pruned; only Bob appears in the mined coinbase, after
        // the pool payout (SPEC §3.5 order).
        let coinbase = vec![
            output(pool_script(), 60),
            output(spc_accounting::p2tr_script(&id(0x02)), 40),
            output(vec![0x6a, 0x04, b'S', b'P', b'c'], 0),
        ];
        let pending = BTreeMap::new();
        let delivered = BTreeSet::new();
        let high_water = high_water(0, "3");
        let input = ReconstructInput {
            height: 100,
            high_water: &high_water,
            entries: &entries,
            slices: &slices,
            window_truncated: false,
            pending: &pending,
            coinbase: &coinbase,
            economics: BlockEconomics {
                subsidy: 100,
                fees: 0,
            },
            delivered: &delivered,
            cfg: base_cfg(2),
            coinbase_threshold_sats: 30,
        };

        let reconstructed = reconstruct_settlement(&input).unwrap();
        let alice_row = reconstructed
            .ledger
            .iter()
            .find(|row| row.sp_id == alice)
            .unwrap();
        assert_eq!(alice_row.rail, LedgerRail::Pending);
        assert_eq!(alice_row.paid_sats, 0);
        assert_eq!(alice_row.forfeited_sats, 0);
        assert_eq!(alice_row.pending_delta, 50);
        let bob_row = reconstructed
            .ledger
            .iter()
            .find(|row| row.sp_id == bob)
            .unwrap();
        assert_eq!(bob_row.rail, LedgerRail::Coinbase);
        assert_eq!(bob_row.paid_sats, 40);
        assert_eq!(bob_row.pending_delta, 10);
    }

    /// SPEC §7.4: an id whose only contribution is a carried pending balance
    /// still gets a ledger row, and the delta moves that balance to the rail
    /// the settlement re-decision picks (Ark here), never silently dropping
    /// it.
    #[test]
    fn pending_only_entry_keeps_its_balance() {
        let alice = id(0xa1);
        let carol = id(0xc3);
        let slices = vec![slice_row(2, 100, 200, 1, &[(alice, 200, 0)])];
        let mut carol_entry = entry(carol, Rail::Pending, None, 100);
        carol_entry.dest_id = b"ark-destination".to_vec();
        let entries = vec![
            entry(alice, Rail::Coinbase, Some(id(0x01)), 200),
            carol_entry,
        ];
        let coinbase = vec![
            output(pool_script(), 0),
            output(spc_accounting::p2tr_script(&id(0x01)), 200),
        ];
        let pending = BTreeMap::from([(carol, 100)]);
        let delivered = BTreeSet::from([carol]);
        let high_water = high_water(0, "3");
        let input = ReconstructInput {
            height: 100,
            high_water: &high_water,
            entries: &entries,
            slices: &slices,
            window_truncated: false,
            pending: &pending,
            coinbase: &coinbase,
            economics: BlockEconomics {
                subsidy: 200,
                fees: 0,
            },
            delivered: &delivered,
            cfg: base_cfg(2),
            coinbase_threshold_sats: 150,
        };

        let reconstructed = reconstruct_settlement(&input).unwrap();
        let carol_row = reconstructed
            .ledger
            .iter()
            .find(|row| row.sp_id == carol)
            .unwrap();
        assert_eq!(carol_row.rail, LedgerRail::Ark);
        assert_eq!(carol_row.ark_due_sats, 100);
        assert_eq!(carol_row.pending_delta, -100);
        assert_eq!(
            deltas_map(reconstructed.pending_deltas.clone()).get(&carol),
            Some(&-100)
        );
    }

    /// An older front records the still-open finding slice as the mark.
    /// When the inclusive window disagrees with the mined coinbase, the
    /// reconstruction retries without that slice and matches the known
    /// window (SPEC §4.3, §7.4).
    #[test]
    fn high_water_naming_the_open_slice_is_excluded_on_retry() {
        let alice = id(0xa1);
        let bob = id(0xb2);
        let slices = vec![
            // The open finding slice: credited here only if wrongly taken.
            slice_row(4, 100, 100, 1, &[(alice, 100, 0)]),
            slice_row(3, 100, 100, 1, &[(alice, 60, 0)]),
            slice_row(2, 100, 100, 1, &[(bob, 40, 0)]),
        ];
        let entries = vec![
            entry(alice, Rail::Coinbase, Some(id(0x01)), 60),
            entry(bob, Rail::Coinbase, Some(id(0x02)), 40),
        ];
        let coinbase = vec![
            output(pool_script(), 100),
            output(spc_accounting::p2tr_script(&id(0x01)), 60),
            output(spc_accounting::p2tr_script(&id(0x02)), 40),
        ];
        let pending = BTreeMap::new();
        let delivered = BTreeSet::new();
        let hw = high_water(0, "4");
        let input = ReconstructInput {
            height: 100,
            high_water: &hw,
            entries: &entries,
            slices: &slices,
            window_truncated: false,
            pending: &pending,
            coinbase: &coinbase,
            economics: BlockEconomics {
                subsidy: 200,
                fees: 0,
            },
            delivered: &delivered,
            cfg: base_cfg(2),
            coinbase_threshold_sats: 30,
        };

        let reconstructed = reconstruct_settlement(&input).unwrap();
        assert_eq!(reconstructed.window, vec![3, 2]);
        assert_eq!(reconstructed.raw.get(&alice), Some(&60));
        assert_eq!(reconstructed.raw.get(&bob), Some(&40));
    }

    #[test]
    fn multi_front_high_water_fails_closed() {
        let hw = BTreeMap::from([(0u8, "1".to_string()), (1, "2".to_string())]);
        let input = ReconstructInput {
            height: 1,
            high_water: &hw,
            entries: &[],
            slices: &[],
            window_truncated: false,
            pending: &BTreeMap::new(),
            coinbase: &[],
            economics: BlockEconomics {
                subsidy: 0,
                fees: 0,
            },
            delivered: &BTreeSet::new(),
            cfg: base_cfg(8),
            coinbase_threshold_sats: 330,
        };
        assert!(matches!(
            reconstruct_settlement(&input),
            Err(ReconstructError::MultiFrontHighWater(2))
        ));
    }

    /// SPEC §4.6: `T_actual` is the sum of the mined coinbase outputs. A
    /// coinbase that does not match the block's subsidy plus fees is not
    /// this block's and settlement holds.
    #[test]
    fn coinbase_total_must_match_subsidy_plus_fees() {
        let alice = id(0xa1);
        let slices = vec![slice_row(2, 100, 100, 1, &[(alice, 100, 0)])];
        let entries = vec![entry(alice, Rail::Coinbase, Some(id(0x01)), 100)];
        let coinbase = vec![output(spc_accounting::p2tr_script(&id(0x01)), 100)];
        let hw = high_water(0, "3");
        let input = ReconstructInput {
            height: 100,
            high_water: &hw,
            entries: &entries,
            slices: &slices,
            window_truncated: false,
            pending: &BTreeMap::new(),
            coinbase: &coinbase,
            economics: BlockEconomics {
                subsidy: 90,
                fees: 0,
            },
            delivered: &BTreeSet::new(),
            cfg: base_cfg(2),
            coinbase_threshold_sats: 30,
        };
        assert!(matches!(
            reconstruct_settlement(&input),
            Err(ReconstructError::RevenueMismatch {
                t_actual: 100,
                expected: 90
            })
        ));
    }

    /// O11 `F` is the fixed-output total, not just `OP_RETURN` outputs. A
    /// non-OP_RETURN additional output must reduce the claim by exactly its
    /// value.
    #[test]
    fn jd_fixed_outputs_include_non_op_return_outputs() {
        let alice = id(0xa1);
        let bob = id(0xb2);
        let slices = vec![slice_row(2, 200, 100, 1, &[(alice, 50, 0), (bob, 50, 0)])];
        let entries = vec![
            entry(alice, Rail::Coinbase, Some(id(0x01)), 200),
            entry(bob, Rail::Coinbase, Some(id(0x02)), 200),
        ];
        // T_actual = 200: pool 43 + alice 70 + bob 80 + carrier 0 + fixed 7.
        let fixed_script = vec![0x00, 0x14, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66];
        let coinbase = vec![
            output(pool_script(), 43),
            output(spc_accounting::p2tr_script(&id(0x01)), 70),
            output(spc_accounting::p2tr_script(&id(0x02)), 80),
            output(vec![0x6a, 0x04, b'S', b'P', b'c'], 0),
            output(fixed_script, 7),
        ];
        let pending = BTreeMap::new();
        let delivered = BTreeSet::new();
        let hw = high_water(0, "3");
        let input = ReconstructInput {
            height: 200,
            high_water: &hw,
            entries: &entries,
            slices: &slices,
            window_truncated: false,
            pending: &pending,
            coinbase: &coinbase,
            economics: BlockEconomics {
                subsidy: 200,
                fees: 0,
            },
            delivered: &delivered,
            cfg: base_cfg(2),
            coinbase_threshold_sats: 30,
        };

        let reconstructed = reconstruct_settlement(&input).unwrap();
        // W = 400; F = 7; claim = floor(200 * 193 / 400) = 96.
        let deltas = deltas_map(reconstructed.pending_deltas);
        assert_eq!(deltas.get(&alice), Some(&26)); // 96 - 70
        assert_eq!(deltas.get(&bob), Some(&16)); // 96 - 80
    }

    #[test]
    fn configured_network_overrides_the_hardcoded_fallbacks() {
        assert_eq!(identity_network(Some(2), 3), 2);
        assert_eq!(identity_network(Some(2), 1), 2);
        assert_eq!(identity_network(None, 3), 3);
        assert_eq!(identity_network(None, 1), 1);
    }

    /// Reviewer #12: when the finding slice is stored, its `d` is the
    /// template's; do not take the newest older slice across a retarget.
    #[test]
    fn window_target_prefers_the_finding_height() {
        let slices = vec![
            slice_row(9, 99, 100, 1, &[]),
            slice_row(10, 100, 200, 1, &[]),
        ];
        assert_eq!(window_target(&slices, 100, 4).unwrap(), 800);
        // No slice at the finding height: fall back to the newest.
        assert_eq!(window_target(&slices, 101, 4).unwrap(), 400);
    }

    #[test]
    fn ark_order_id_is_deterministic() {
        let a = ark_order_id(&[1; 32], &[2; 32]);
        assert_eq!(a, ark_order_id(&[1; 32], &[2; 32]));
        assert_ne!(a, ark_order_id(&[1; 32], &[3; 32]));
        assert_ne!(a, ark_order_id(&[2; 32], &[2; 32]));
    }

    // -- env-gated Ark lifecycle tests -------------------------------------

    const RETRY_DEPTH: u32 = 6;

    struct FixedChain {
        tip: u32,
    }

    /// Non-authoritative view (legacy ledger path) whose gate is open.
    struct LegacyChain;

    #[async_trait::async_trait]
    impl ChainView for LegacyChain {
        async fn tip_height(&self) -> Result<u32, ChainError> {
            Ok(u32::MAX)
        }

        async fn block_hash_at(&self, _height: u32) -> Result<Option<[u8; 32]>, ChainError> {
            Ok(None)
        }

        async fn is_active(&self, _block: &[u8; 32], _height: u32) -> Result<bool, ChainError> {
            Ok(true)
        }
    }

    #[async_trait::async_trait]
    impl ChainView for FixedChain {
        async fn tip_height(&self) -> Result<u32, ChainError> {
            Ok(self.tip)
        }

        async fn block_hash_at(&self, _height: u32) -> Result<Option<[u8; 32]>, ChainError> {
            Ok(None)
        }

        async fn is_active(&self, _block: &[u8; 32], _height: u32) -> Result<bool, ChainError> {
            Ok(true)
        }

        fn authoritative(&self) -> bool {
            true
        }
    }

    async fn ark_test_env() -> Option<(Store, Streams)> {
        let db = std::env::var("SPC_TEST_DATABASE_URL").ok()?;
        let redis = std::env::var("SPC_TEST_REDIS_URL").ok()?;
        let store = Store::connect(&db).await.expect("SPC_TEST_DATABASE_URL");
        store.migrate().await.expect("migrate");
        let streams = Streams::connect(&redis).await.expect("SPC_TEST_REDIS_URL");
        Some((store, streams))
    }

    fn unique_stream(prefix: &str) -> String {
        format!("spc:test:{prefix}:{}{}", now_rev(), rand::random::<u64>())
    }

    fn test_key() -> secp256k1::Keypair {
        let secp = secp256k1::Secp256k1::new();
        secp256k1::Keypair::from_secret_key(
            &secp,
            &secp256k1::SecretKey::from_slice(&[7; 32]).unwrap(),
        )
    }

    fn ark_service(
        store: Store,
        streams: Streams,
        chain: Arc<dyn ChainView>,
        key: Option<secp256k1::Keypair>,
        orders: &str,
        receipts: &str,
    ) -> PayoutService {
        PayoutService::new(store, streams, RETRY_DEPTH, 330, chain, key, 0)
            .with_ark_streams(orders, receipts)
    }

    /// Insert a confirmed block whose ledger puts `sats` on the Ark rail for
    /// `sp_id`, with `dest` in the frozen distribution entries.
    async fn insert_scheduled_ark(
        store: &Store,
        sp_id: [u8; 32],
        block_hash: [u8; 32],
        dest: &str,
        sats: u64,
    ) {
        store
            .upsert_identity(&SpIdentityRow {
                sp_id,
                b_scan: vec![2; 33],
                b_spend: vec![3; 33],
                network: 3,
            })
            .await
            .unwrap();
        let fingerprint: [u8; 32] = rand::random();
        let entries = vec![Entry {
            sp_id,
            rail: Rail::Ark,
            p_x: Vec::new(),
            dest_id: dest.as_bytes().to_vec(),
            sats,
        }];
        store
            .insert_snapshot(&SnapshotRow {
                fingerprint,
                height: 800_000,
                a_send_x: [4; 32],
                distribution_id: None,
                entries: serde_json::to_value(entries).unwrap(),
                high_water: serde_json::json!({"0": "1"}),
            })
            .await
            .unwrap();
        store
            .settle(
                &Settlement {
                    block_hash,
                    height: 800_000,
                    fingerprint: Some(fingerprint),
                    status: BlockStatus::Confirmed,
                    ledger: vec![LedgerRow {
                        sp_id,
                        rail: LedgerRail::Ark,
                        paid_sats: 0,
                        ark_due_sats: sats,
                        forfeited_sats: 0,
                        pending_delta: 0,
                    }],
                },
                RETRY_DEPTH,
            )
            .await
            .unwrap();
    }

    /// A payout restart must re-emit the same order from `ark_payout`, with a
    /// deterministic `order_id` so `ark` dedupes instead of paying twice.
    #[tokio::test]
    async fn restart_reemits_a_scheduled_ark_order() {
        let Some((store, streams)) = ark_test_env().await else {
            return;
        };
        let sp_id: [u8; 32] = rand::random();
        let block: [u8; 32] = rand::random();
        insert_scheduled_ark(&store, sp_id, block, "tark1destination", 250).await;

        let orders = unique_stream("ark:orders");
        let receipts = unique_stream("ark:receipts");
        streams.ensure_group(&orders, "test").await.unwrap();
        let key = test_key();
        let chain: Arc<dyn ChainView> = Arc::new(FixedChain { tip: 900_000 });
        let order_id = ark_order_id(&block, &sp_id);

        // First process instance.
        ark_service(
            store.clone(),
            streams.clone(),
            chain.clone(),
            Some(key),
            &orders,
            &receipts,
        )
        .emit_due_ark_orders()
        .await
        .unwrap();
        // "Restart": a fresh instance, same database.
        ark_service(
            store.clone(),
            streams.clone(),
            chain,
            Some(key),
            &orders,
            &receipts,
        )
        .emit_due_ark_orders()
        .await
        .unwrap();

        let entries = streams
            .read_group(&orders, "test", "c1", 256)
            .await
            .unwrap();
        let ours: Vec<spc_ext::ArkSendOrder> = entries
            .iter()
            .map(|(_, payload)| spc_ext::ArkSendOrder::decode(payload).unwrap())
            .filter(|order| order.order_id == order_id)
            .collect();
        assert_eq!(ours.len(), 2, "one order per emit");
        assert_eq!(ours[0].order_id, ours[1].order_id);
        assert_eq!(ours[0].sats, 250);
        assert_eq!(ours[0].not_before_height, 800_000 + RETRY_DEPTH);
        ours[0]
            .verify(&key.x_only_public_key().0)
            .expect("signed by the payout key");
    }

    /// A `Sent` receipt records the VTXO and never credits `pending_balance`
    /// (a replayed receipt is a no-op, not a second payment).
    #[tokio::test]
    async fn sent_receipt_does_not_credit_pending() {
        let Some((store, streams)) = ark_test_env().await else {
            return;
        };
        let sp_id: [u8; 32] = rand::random();
        let block: [u8; 32] = rand::random();
        insert_scheduled_ark(&store, sp_id, block, "tark1destination", 250).await;

        let orders = unique_stream("ark:orders");
        let receipts = unique_stream("ark:receipts");
        streams.ensure_group(&receipts, GROUP).await.unwrap();
        let svc = ark_service(
            store.clone(),
            streams.clone(),
            Arc::new(FixedChain { tip: 900_000 }),
            Some(test_key()),
            &orders,
            &receipts,
        );
        let receipt = ArkSendReceipt {
            order_id: ark_order_id(&block, &sp_id),
            status: ReceiptStatus::Sent,
            vtxo_id: vec![9; 34],
            expiry_height: 900_100,
        };
        streams
            .add(&receipts, &receipt.encode().unwrap())
            .await
            .unwrap();
        svc.process_receipts_once().await.unwrap();

        assert!(store
            .scheduled_ark_payouts()
            .await
            .unwrap()
            .iter()
            .all(|row| row.block_hash != block));
        assert_eq!(store.pending_for(&sp_id).await.unwrap(), 0);

        // Replay: still no pending credit.
        streams
            .add(&receipts, &receipt.encode().unwrap())
            .await
            .unwrap();
        svc.process_receipts_once().await.unwrap();
        assert_eq!(store.pending_for(&sp_id).await.unwrap(), 0);
    }

    /// A `Failed` receipt moves the sats to `pending_balance` once, never to
    /// forfeit and never twice.
    #[tokio::test]
    async fn failed_receipt_credits_pending_once() {
        let Some((store, streams)) = ark_test_env().await else {
            return;
        };
        let sp_id: [u8; 32] = rand::random();
        let block: [u8; 32] = rand::random();
        insert_scheduled_ark(&store, sp_id, block, "tark1destination", 250).await;

        let orders = unique_stream("ark:orders");
        let receipts = unique_stream("ark:receipts");
        streams.ensure_group(&receipts, GROUP).await.unwrap();
        let svc = ark_service(
            store.clone(),
            streams.clone(),
            Arc::new(FixedChain { tip: 900_000 }),
            Some(test_key()),
            &orders,
            &receipts,
        );
        let receipt = ArkSendReceipt {
            order_id: ark_order_id(&block, &sp_id),
            status: ReceiptStatus::Failed,
            vtxo_id: Vec::new(),
            expiry_height: 0,
        };
        streams
            .add(&receipts, &receipt.encode().unwrap())
            .await
            .unwrap();
        svc.process_receipts_once().await.unwrap();
        assert_eq!(store.pending_for(&sp_id).await.unwrap(), 250);
        assert!(store
            .scheduled_ark_payouts()
            .await
            .unwrap()
            .iter()
            .all(|row| row.block_hash != block));

        // A replay is idempotent: the balance is credited once.
        streams
            .add(&receipts, &receipt.encode().unwrap())
            .await
            .unwrap();
        svc.process_receipts_once().await.unwrap();
        assert_eq!(store.pending_for(&sp_id).await.unwrap(), 250);
    }

    /// The scrub caller: `scrub_once` deletes `slice_credit` below the
    /// retention window.
    #[tokio::test]
    async fn scrub_once_deletes_old_slice_credits() {
        let Some((store, streams)) = ark_test_env().await else {
            return;
        };
        let sp_id: [u8; 32] = rand::random();
        store
            .upsert_identity(&SpIdentityRow {
                sp_id,
                b_scan: vec![2; 33],
                b_spend: vec![3; 33],
                network: 3,
            })
            .await
            .unwrap();
        let slice_id = rand::random::<u64>() & 0x7fff_ffff_ffff_ffff;
        store
            .insert_slice(&SliceRow {
                slice_id,
                // Far below the other tests' 800_000-height rows, so this
                // scrub cannot disturb them.
                height: 1_000,
                prev_hash: [1; 32],
                ref_job_id: 1,
                ref_fees: 0,
                n_shares: 1,
                difficulty: "10".into(),
                root: [2; 32],
                shares: vec![3],
                credits: vec![SliceCreditRow {
                    sp_id,
                    diff_sum: "10".into(),
                    diff_fee_sum: "1".into(),
                }],
            })
            .await
            .unwrap();

        let svc = ark_service(
            store.clone(),
            streams,
            Arc::new(FixedChain { tip: 10_000 }),
            None,
            &unique_stream("ark:orders"),
            &unique_stream("ark:receipts"),
        )
        .with_scrub(ScrubConfig {
            retain_windows: 16,
            scrub_after_blocks: 100,
        });
        svc.scrub_once().await.unwrap();

        let slices = store.window_slices_to(slice_id, i64::MAX).await.unwrap();
        let row = slices
            .iter()
            .find(|s| s.slice_id == slice_id)
            .expect("slice row");
        assert!(row.credits.is_empty(), "old credits are scrubbed");
    }

    /// Regression for the accounting fix: an unregistered id below `C`
    /// forfeits only this block's raw earnings. A prior pending balance is
    /// retained, so the ledger delta is zero and conservation holds.
    #[test]
    fn forfeit_confiscates_raw_and_keeps_prior_pending() {
        let alice = id(0xa1);
        // raw = 40 (the window's only credit), pending = 100, owed = 140.
        let slices = vec![slice_row(2, 400, 100, 1, &[(alice, 100, 0)])];
        let entries = vec![entry(alice, Rail::Forfeit, None, 140)];
        // T = 40: nothing is kept, so the whole coinbase is the pool output.
        let coinbase = vec![output(pool_script(), 40)];
        let pending = BTreeMap::from([(alice, 100u64)]);
        let hw = high_water(0, "3");
        let input = ReconstructInput {
            height: 400,
            high_water: &hw,
            entries: &entries,
            slices: &slices,
            window_truncated: false,
            pending: &pending,
            coinbase: &coinbase,
            economics: BlockEconomics {
                subsidy: 40,
                fees: 0,
            },
            delivered: &BTreeSet::new(),
            cfg: base_cfg(2),
            coinbase_threshold_sats: 330,
        };

        let reconstructed = reconstruct_settlement(&input).unwrap();
        assert_eq!(reconstructed.raw.get(&alice), Some(&40));
        assert_eq!(reconstructed.t_actual, 40);
        let row = reconstructed
            .ledger
            .iter()
            .find(|row| row.sp_id == alice)
            .expect("forfeit row");
        assert_eq!(row.rail, LedgerRail::Forfeit);
        assert_eq!(row.forfeited_sats, 40, "only raw is forfeited");
        assert_eq!(row.pending_delta, 0, "prior pending is retained");
        assert!(
            !reconstructed
                .pending_deltas
                .iter()
                .any(|(id, _)| *id == alice),
            "no pending release for a raw-only forfeit"
        );
        // Conservation: forfeited raw + retained pending == owed.
        let after = 100i64 + row.pending_delta;
        assert_eq!(i64::try_from(row.forfeited_sats).unwrap() + after, 140);
    }

    /// With raw >= C the same balance routes to the coinbase. When the block
    /// reward cannot cover the full owed total, the routing pays what the
    /// coinbase carries and retains the rest as pending (partial keep).
    #[test]
    fn raw_at_threshold_routes_to_coinbase_and_retains_the_rest() {
        let alice = id(0xa1);
        // raw = 330 (>= C), pending = 100: owed = 430, T = 330.
        let slices = vec![slice_row(2, 500, 100, 1, &[(alice, 100, 0)])];
        let entries = vec![entry(alice, Rail::Coinbase, Some(id(0x01)), 430)];
        let coinbase = vec![
            output(pool_script(), 0),
            output(spc_accounting::p2tr_script(&id(0x01)), 330),
        ];
        let pending = BTreeMap::from([(alice, 100u64)]);
        let hw = high_water(0, "3");
        let input = ReconstructInput {
            height: 500,
            high_water: &hw,
            entries: &entries,
            slices: &slices,
            window_truncated: false,
            pending: &pending,
            coinbase: &coinbase,
            economics: BlockEconomics {
                subsidy: 330,
                fees: 0,
            },
            delivered: &BTreeSet::new(),
            cfg: base_cfg(2),
            coinbase_threshold_sats: 330,
        };

        let reconstructed = reconstruct_settlement(&input).unwrap();
        assert_eq!(reconstructed.raw.get(&alice), Some(&330));
        let row = reconstructed
            .ledger
            .iter()
            .find(|row| row.sp_id == alice)
            .expect("coinbase row");
        assert_eq!(row.rail, LedgerRail::Coinbase);
        assert_eq!(row.paid_sats, 330);
        assert_eq!(row.pending_delta, 0, "the carried 100 is retained");
        assert!(reconstructed
            .pending_deltas
            .iter()
            .all(|(id, _)| *id != alice));
        // Conservation: paid + retained pending == owed.
        let after = 100i64 + row.pending_delta;
        assert_eq!(i64::try_from(row.paid_sats).unwrap() + after, 430);
    }

    fn test_front() -> crate::front::Front {
        use crate::coinbase::PoolPayoutDescriptor;
        use crate::front::{Front, FrontConfig};
        use crate::template::{MockTemplateSource, Template};
        use bitcoin::bip32::Xpriv;

        let secp = bitcoin::secp256k1::Secp256k1::new();
        let xprv = Xpriv::new_master(bitcoin::Network::Testnet, &[0x5c; 32]).unwrap();
        let xpub = bitcoin::bip32::Xpub::from_priv(&secp, &xprv);

        let cfg = FrontConfig {
            network: spc_crypto::Network::Regtest,
            fee_bps: 100,
            window_n: 8,
            slice_fee_delta_sats: 0,
            max_slice_shares: 65_000,
            dust_limit_sats: 330,
            coinbase_threshold_sats: 330,
            max_payout_slots: 256,
            budget_wu: 100_000,
            overhead_wu: 0,
            pool_payout_descriptor: PoolPayoutDescriptor::parse(
                &format!("tr({xpub}/0/*)"),
                spc_crypto::Network::Regtest,
            )
            .unwrap(),
            pool_payout_script: Some(vec![0x51; 34]),
            extranonce_len: 8,
            front_id: 0,
            keep_slices: 16,
        };
        let template = Template {
            height: 1,
            prev_hash: [0; 32],
            version: 0x2000_0000,
            bits: 0x207f_ffff,
            curtime: 0,
            subsidy: 0,
            fees: 0,
            coinbase_value: 0,
            transactions: vec![],
            merkle_branches: vec![],
            coinbase_output_max_additional_size: 0,
        };
        Front::new(
            cfg,
            Arc::new(MockTemplateSource::new(template)),
            Arc::new(std::sync::Mutex::new(crate::type32::Type32Store::default())),
        )
    }

    /// The front's rev filter (front.rs) drops an event at or below the
    /// bootstrap watermark; a fresh one is applied.
    #[test]
    fn front_ignores_events_at_or_below_the_bootstrap_rev() {
        let mut front = test_front();
        front.apply_bootstrap(&BootstrapSnapshot {
            rev: 42,
            applied_blocks: Vec::new(),
            pending_balance: Vec::new(),
            ark_delivery: Vec::new(),
            window_tail: Vec::new(),
            type32_blobs: Vec::new(),
            slices_cursor: None,
            front_events_cursor: None,
        });

        let stale = FrontEvent::LedgerApplied {
            rev: 42,
            block_hash: [1; 32],
            republish: true,
            pending_deltas: vec![([2; 32], 5)],
        };
        assert!(!front.apply_front_event(&stale));
        let fresh = FrontEvent::LedgerApplied {
            rev: 43,
            block_hash: [1; 32],
            republish: true,
            pending_deltas: vec![([2; 32], 5)],
        };
        assert!(front.apply_front_event(&fresh));
    }

    /// Reviewer 3: `retain_windows` is blocks, but an unbooked block at the
    /// confirmation edge still needs `window_n` blocks of credits behind it.
    #[test]
    fn scrub_retention_covers_an_unbooked_window() {
        // Defaults: configured 16 already exceeds 6 + 8 + 1.
        assert_eq!(scrub_retain_windows(16, 6, 8), 16);
        // A too-small configured window is raised to depth + N + 1.
        assert_eq!(scrub_retain_windows(2, 6, 8), 15);
        assert_eq!(scrub_retain_windows(0, 1, 1), 3);
    }

    /// The `LedgerApplied` revision is bumped (and persisted) after the
    /// settle transaction and before the XADD; bootstrap carries it.
    #[tokio::test]
    async fn ledger_event_rev_is_persisted_before_xadd() {
        let Some((store, streams)) = ark_test_env().await else {
            return;
        };
        let events = unique_stream("front:events");
        streams
            .ensure_group_from(&events, GROUP, "0")
            .await
            .unwrap();

        let sp_id: [u8; 32] = rand::random();
        store
            .upsert_identity(&SpIdentityRow {
                sp_id,
                b_scan: vec![2; 33],
                b_spend: vec![3; 33],
                network: 3,
            })
            .await
            .unwrap();
        let block: [u8; 32] = rand::random();
        let event = BlockEvent {
            block_hash: block,
            height: 800_000,
            front_id: 0,
            fingerprint: rand::random(),
            a_send_x: [4; 32],
            high_water: BTreeMap::from([(0u8, "1".to_string())]),
            entries: vec![entry(sp_id, Rail::Coinbase, Some(id(0x01)), 100)],
        };
        // Non-authoritative chain: the legacy ledger path books the entry,
        // which is enough to exercise the settle -> bump -> XADD order.
        let svc = PayoutService::new(
            store.clone(),
            streams.clone(),
            RETRY_DEPTH,
            330,
            Arc::new(LegacyChain),
            None,
            0,
        )
        .with_front_events_stream(&events);
        assert!(svc.on_block(&event.encode().unwrap()).await.is_ok());

        let entries = streams.read_group(&events, GROUP, "c", 16).await.unwrap();
        let mut revs = Vec::new();
        for (_, payload) in entries {
            if let Ok(FrontEvent::LedgerApplied {
                rev, block_hash, ..
            }) = FrontEvent::decode(&payload)
            {
                assert_eq!(block_hash, block);
                revs.push(rev);
            }
        }
        assert_eq!(revs.len(), 1, "one ledger event");
        assert!(revs[0] > 0);
        let boot = store.bootstrap(16).await.unwrap();
        assert!(
            boot.rev >= revs[0],
            "bootstrap must cover the emitted event rev"
        );
    }

    /// A front that was down for the original settlement gets the pending
    /// delta from bootstrap; a later republished event for the same block is
    /// ignored (block-hash dedupe), so it is applied exactly once.
    #[test]
    fn bootstrap_applied_blocks_suppress_a_republished_ledger_event() {
        let block = [0x0b; 32];
        let sp_id = [0x21; 32];
        let mut front = test_front();
        front.apply_bootstrap(&BootstrapSnapshot {
            rev: 7,
            applied_blocks: vec![block],
            pending_balance: vec![(sp_id, 40)],
            ark_delivery: Vec::new(),
            window_tail: Vec::new(),
            type32_blobs: Vec::new(),
            slices_cursor: None,
            front_events_cursor: None,
        });

        let republished = FrontEvent::LedgerApplied {
            rev: 8,
            block_hash: block,
            republish: true,
            pending_deltas: vec![(sp_id, 40)],
        };
        assert!(!front.apply_front_event(&republished));
        assert_eq!(front.pending().get(&sp_id), Some(&40));

        // A genuinely new block still applies.
        let fresh = FrontEvent::LedgerApplied {
            rev: 9,
            block_hash: [0x0c; 32],
            republish: true,
            pending_deltas: vec![(sp_id, 5)],
        };
        assert!(front.apply_front_event(&fresh));
        assert_eq!(front.pending().get(&sp_id), Some(&45));
    }

    /// A settlement whose XADD failed (the durable booking exists, no event)
    /// is healed by the periodic republish.
    #[tokio::test]
    async fn dropped_xadd_is_recovered_by_republish() {
        let Some((store, streams)) = ark_test_env().await else {
            return;
        };
        let sp_id: [u8; 32] = rand::random();
        let block: [u8; 32] = rand::random();
        let fingerprint: [u8; 32] = rand::random();
        store
            .upsert_identity(&SpIdentityRow {
                sp_id,
                b_scan: vec![2; 33],
                b_spend: vec![3; 33],
                network: 3,
            })
            .await
            .unwrap();
        store
            .insert_snapshot(&SnapshotRow {
                fingerprint,
                height: 800_000,
                a_send_x: [4; 32],
                distribution_id: None,
                entries: serde_json::json!([]),
                high_water: serde_json::json!({"0": "1"}),
            })
            .await
            .unwrap();
        // The durable booking whose XADD was dropped: settle directly.
        let settle_rev = store
            .settle(
                &Settlement {
                    block_hash: block,
                    height: 800_000,
                    fingerprint: Some(fingerprint),
                    status: BlockStatus::Confirmed,
                    ledger: vec![LedgerRow {
                        sp_id,
                        rail: LedgerRail::Pending,
                        paid_sats: 0,
                        ark_due_sats: 0,
                        forfeited_sats: 0,
                        pending_delta: 40,
                    }],
                },
                RETRY_DEPTH,
            )
            .await
            .unwrap()
            .expect("rev");

        let events = unique_stream("front:events");
        streams
            .ensure_group_from(&events, GROUP, "0")
            .await
            .unwrap();
        let svc = PayoutService::new(
            store.clone(),
            streams.clone(),
            RETRY_DEPTH,
            330,
            Arc::new(LegacyChain),
            None,
            0,
        )
        .with_front_events_stream(&events);
        svc.republish_ledger_events().await.unwrap();

        let entries = streams.read_group(&events, GROUP, "c", 256).await.unwrap();
        let (event_rev, deltas) = entries
            .iter()
            .filter_map(|(_, payload)| FrontEvent::decode(payload).ok())
            .find_map(|event| match event {
                FrontEvent::LedgerApplied {
                    rev,
                    block_hash,
                    pending_deltas,
                    ..
                } if block_hash == block => Some((rev, pending_deltas)),
                _ => None,
            })
            .expect("republished ledger event");
        assert!(event_rev >= settle_rev);
        assert_eq!(deltas, vec![(sp_id, 40)]);
        assert!(store.bootstrap(16).await.unwrap().rev >= event_rev);
    }

    /// Re-delivering a confirmed block event is a no-op and must not emit a
    /// second delta (the settle short-circuit plus block-hash dedupe).
    #[tokio::test]
    async fn confirmed_retry_does_not_emit_a_duplicate_delta() {
        let Some((store, streams)) = ark_test_env().await else {
            return;
        };
        let sp_id: [u8; 32] = rand::random();
        let block: [u8; 32] = rand::random();
        store
            .upsert_identity(&SpIdentityRow {
                sp_id,
                b_scan: vec![2; 33],
                b_spend: vec![3; 33],
                network: 3,
            })
            .await
            .unwrap();

        let events = unique_stream("front:events");
        streams
            .ensure_group_from(&events, GROUP, "0")
            .await
            .unwrap();
        let svc = PayoutService::new(
            store.clone(),
            streams.clone(),
            RETRY_DEPTH,
            330,
            Arc::new(LegacyChain),
            None,
            0,
        )
        .with_front_events_stream(&events);
        let event = BlockEvent {
            block_hash: block,
            height: 800_000,
            front_id: 0,
            fingerprint: rand::random(),
            a_send_x: [4; 32],
            high_water: BTreeMap::from([(0u8, "1".to_string())]),
            entries: vec![entry(sp_id, Rail::Coinbase, None, 100)],
        };
        let payload = event.encode().unwrap();
        assert!(svc.on_block(&payload).await.is_ok());
        let first = streams.len(&events).await.unwrap();
        assert_eq!(first, 1);

        assert!(svc.on_block(&payload).await.is_ok());
        assert_eq!(
            streams.len(&events).await.unwrap(),
            first,
            "a confirmed retry emits no duplicate delta"
        );
    }
}
