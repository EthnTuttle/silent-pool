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
use spc_accounting::work_unit::check_version_rule_compatibility;
use spc_accounting::{
    claim_jd, claim_jd_v2, p2tr_script, raw_payouts, raw_payouts_v2, select_window,
    select_window_v2, ArkCapability, RawPayouts, SliceAgg, SpId, TierConfig, WorkUnitError,
    WorkUnitVersion,
};
use spc_ext::{ArkSendOrder, ArkSendReceipt, ReceiptStatus};
use spc_store::models::*;
use spc_store::redis::{RedisError, Streams};
use spc_store::wire::{
    ArkEpochRegEvent, ArkRegEvent, BlockEvent, Entry, FrontEvent, Rail, SliceEvent,
};
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
    #[error("registration rejected: {0}")]
    RegistrationCode(&'static str),
    #[error("malformed snapshot {0}")]
    Snapshot(&'static str),
    #[error("value exceeds i64")]
    Overflow,
    #[error("ledger event: {0}")]
    LedgerEvent(String),
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
#[derive(Clone, Debug)]
pub struct ChainEconomicEvidence {
    pub parent_hash: [u8; 32],
    pub coinbase_digest: [u8; 32],
    pub pool_script: Vec<u8>,
    pub spc: bool,
}

/// The slice of chain state settlement needs (SPEC §7.4, review 4.1): is
/// this block the active-chain block at its height, how deep is it, what did
/// its coinbase pay, and what were its subsidy and fees?
#[async_trait::async_trait]
pub trait ChainView: Send + Sync {
    async fn economic_evidence(
        &self,
        _block: &[u8; 32],
    ) -> Result<Option<ChainEconomicEvidence>, ChainError> {
        Ok(None)
    }
    async fn transaction(
        &self,
        _txid: &[u8; 32],
    ) -> Result<Option<bitcoin::Transaction>, ChainError> {
        Ok(None)
    }
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
    async fn economic_evidence(
        &self,
        hash: &[u8; 32],
    ) -> Result<Option<ChainEconomicEvidence>, ChainError> {
        use bitcoin::hashes::{sha256, Hash};
        use bitcoincore_rpc::RpcApi;
        let block = self
            .client
            .get_block(&bitcoin::BlockHash::from_byte_array(*hash))
            .map_err(|e| ChainError::Unavailable(e.to_string()))?;
        let cb = block
            .txdata
            .first()
            .ok_or_else(|| ChainError::Unavailable("empty block".into()))?;
        let outputs: Vec<_> = cb
            .output
            .iter()
            .map(|o| spc_crypto::carriers::TxOut {
                value: o.value.to_sat(),
                script_pubkey: o.script_pubkey.as_bytes(),
            })
            .collect();
        Ok(Some(ChainEconomicEvidence {
            parent_hash: block.header.prev_blockhash.to_byte_array(),
            coinbase_digest: sha256::Hash::hash(&bitcoin::consensus::serialize(cb)).to_byte_array(),
            pool_script: cb
                .output
                .first()
                .map(|o| o.script_pubkey.as_bytes().to_vec())
                .unwrap_or_default(),
            spc: cb.input.first().is_some_and(|i| {
                spc_crypto::parse_carrier(i.script_sig.as_bytes(), &outputs).is_some()
            }),
        }))
    }
    async fn transaction(
        &self,
        txid: &[u8; 32],
    ) -> Result<Option<bitcoin::Transaction>, ChainError> {
        use bitcoin::hashes::Hash;
        use bitcoincore_rpc::RpcApi;
        self.client
            .get_raw_transaction(&bitcoin::Txid::from_byte_array(*txid), None)
            .map(Some)
            .map_err(|e| ChainError::Unavailable(e.to_string()))
    }
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
            max_payout_slots: 1024,
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
    #[error("work unit: {0}")]
    WorkUnit(#[from] WorkUnitError),
    #[error("v2 distribution has no frozen window target")]
    MissingFrozenV2Target,
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
    #[error("v2 stored difficulty {stored} on slice {slice_id} differs from credit sum {credits}")]
    StoredDifficultyMismatch {
        slice_id: u64,
        stored: u128,
        credits: u128,
    },
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
    /// Frozen work-unit metadata selecting the window and payout arithmetic.
    pub work_unit_version: WorkUnitVersion,
    pub work_unit_fingerprint: Option<[u8; 32]>,
    pub window_rule: WindowRule,
    /// The exact window target (`N x d`, SPEC §4.3) frozen in
    /// `distribution_snapshot.window_target` when the front built this
    /// distribution. `None` only for rows written before that column
    /// existed: settlement then falls back to the warned slice estimate.
    pub window_target: Option<u128>,
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
    reconstruct_once(input)
}

fn reconstruct_once(input: &ReconstructInput<'_>) -> Result<Reconstructed, ReconstructError> {
    // The vector is inclusive: `high_water = {front: last slice IN the
    // window}` (SPEC §4.3), so every stored slice at or below it is a
    // candidate. The finding share lives in the still-open slice, whose id
    // is above the mark and is therefore absent.
    let high_water = single_front_high_water(input.high_water)?;
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

    // Keep historical JD empty-window behavior separate from this pool-template
    // correction. O11 claims and gross JD weights are unchanged.
    if input.slices.is_empty() && mode == TemplateMode::Jd {
        if !input.entries.is_empty() {
            return Err(ReconstructError::NoSlices);
        }
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
    if aggs.is_empty() && high_water != 0 {
        return Err(ReconstructError::NoSlices);
    }
    check_version_rule_compatibility(input.work_unit_version, input.window_rule)?;
    if input.work_unit_version == WorkUnitVersion::V2 && input.window_target.is_none() {
        return Err(ReconstructError::MissingFrozenV2Target);
    }
    // The persisted target is the one the front selected this window with;
    // only a pre-migration row (no stored value) falls back to the estimate.
    let target = match input.window_target {
        Some(target) => target,
        // A legacy known-empty window has no target estimate to perform. V2
        // above always requires its actual frozen Q, even when no slice exists.
        None if aggs.is_empty() => 0,
        None => window_target(input.slices, input.height, input.cfg.window_n)?,
    };
    // `u64::MAX`: nothing is skipped. The stored slices at or below the
    // high-water mark already are the window's slice set (SPEC §4.3).
    let window = match input.work_unit_version {
        WorkUnitVersion::V1 => select_window(&aggs, u64::MAX, target, input.high_water.clone()),
        WorkUnitVersion::V2 => select_window_v2(
            &aggs,
            u64::MAX,
            target,
            input.high_water.clone(),
            input.work_unit_fingerprint,
        )?,
    };
    // A capped walk is only ambiguous when the window itself consumed every
    // returned slice; then there may be older rows it needed.
    if input.window_truncated && window.slices.len() == aggs.len() {
        return Err(ReconstructError::WindowTruncated);
    }
    let raw = if window.slices.is_empty() {
        // Same raw adapter as front: no new earnings, but frozen prior P still
        // participates in ordinary routing, mined-output checks and E-P below.
        spc_accounting::RawPayouts {
            raw: BTreeMap::new(),
            pool_sats: t_actual,
            r_net: input.economics.subsidy,
            f_net: input.economics.fees,
        }
    } else {
        match input.work_unit_version {
            WorkUnitVersion::V1 => raw_payouts(
                &window,
                input.economics.subsidy,
                input.economics.fees,
                input.cfg.fee_bps,
            )?,
            WorkUnitVersion::V2 => raw_payouts_v2(
                &window,
                input.economics.subsidy,
                input.economics.fees,
                input.cfg.fee_bps,
            )?,
        }
    };

    let mut entry_by_id: BTreeMap<SpId, &Entry> = BTreeMap::new();
    for e in input.entries {
        e.validate()
            .map_err(|_| ReconstructError::MissingPayoutKey(e.sp_id))?;
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
    if mode == TemplateMode::Pool
        && input
            .pending
            .iter()
            .any(|(id, p)| *p > 0 && !entry_by_id.contains_key(id))
    {
        return Err(ReconstructError::WindowMismatch);
    }

    let mut script_value: BTreeMap<&[u8], u64> = BTreeMap::new();
    for o in input.coinbase {
        if script_value.insert(o.script.as_slice(), o.value).is_some() {
            return Err(ReconstructError::DuplicateScript);
        }
    }
    let mut paid: BTreeMap<SpId, u64> = BTreeMap::new();
    let mut all_keys = BTreeSet::new();
    for e in input.entries {
        if e.p_x.is_empty() {
            if e.rail == Rail::Coinbase {
                return Err(ReconstructError::MissingPayoutKey(e.sp_id));
            }
            continue;
        }
        if mode == TemplateMode::Jd && e.p_x.len() != 1 {
            return Err(ReconstructError::PaidMismatch(e.sp_id));
        }
        for p_x in &e.p_x {
            if !all_keys.insert(*p_x) {
                return Err(ReconstructError::DuplicateScript);
            }
            let script = p2tr_script(p_x);
            if let Some(value) = script_value.get(script.as_slice()).copied() {
                if e.rail != Rail::Coinbase {
                    return Err(ReconstructError::PaidMismatch(e.sp_id));
                }
                let sum = paid.entry(e.sp_id).or_insert(0);
                *sum = sum.checked_add(value).ok_or(ReconstructError::Overflow)?;
            }
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
                // Reconstruct from this job's frozen P, not the later booked B.
                // Full owed and every actual k-associated amount must agree.
                let expected_sats = if e.rail == Rail::Coinbase {
                    spc_accounting::band_amount(owed).0
                } else if e.rail == Rail::Forfeit {
                    raw_sats
                } else {
                    owed
                };
                if e.sats != expected_sats || e.owed_sats != owed {
                    return Err(ReconstructError::CoinbaseTierMismatch);
                }
                amounts.insert(e.sp_id, owed);
            }
            let mut caps = BTreeMap::new();
            for e in input.entries {
                caps.insert(
                    e.sp_id,
                    ArkCapability {
                        registered: input.delivered.contains(&e.sp_id),
                        registration_committed: input.delivered.contains(&e.sp_id),
                        dest_known: !e.dest_id.is_empty(),
                    },
                );
            }
            let tier = tier_config(input);
            let known = input
                .entries
                .iter()
                .filter(|e| e.rail == Rail::Coinbase)
                .map(|e| e.sp_id)
                .collect();
            let routing = spc_accounting::route_tiers_with_keys(
                &raw,
                input.pending,
                &caps,
                &tier,
                t_actual,
                Some(&known),
            )?;
            if input
                .coinbase
                .first()
                .is_none_or(|output| output.value != routing.pay_p)
            {
                return Err(ReconstructError::CoinbaseTierMismatch);
            }
            forfeited_now = routing.forfeited.iter().copied().collect();
            let expected_kept: BTreeSet<SpId> = routing.kept.iter().map(|(id, _)| *id).collect();
            let paid_ids: BTreeSet<SpId> = paid.keys().copied().collect();
            // The mined coinbase is the truth. The routing set can differ
            // when pending moved between the distribution build and this
            // booking, so only a reconstruction that would over-credit the
            // window is fatal.
            // Unknown delivery keys are the sole legitimate large-balance
            // hold. Every represented coinbase row still pays full entitlement.
            if expected_kept
                .difference(&paid_ids)
                .any(|id| entry_by_id.get(id).is_none_or(|e| e.rail != Rail::Pending))
                || !paid_ids.is_subset(&expected_kept)
            {
                return Err(ReconstructError::CoinbaseTierMismatch);
            }
            for e in input.entries {
                if e.rail == Rail::Coinbase {
                    let value = paid
                        .get(&e.sp_id)
                        .ok_or(ReconstructError::MissingCoinbaseOutput(e.sp_id))?;
                    let owed = amounts[&e.sp_id];
                    let shape = spc_accounting::decompose(owed, input.cfg.max_payout_slots)?;
                    // Every represented coinbase row pays its complete greedy
                    // decomposition. No partial entitlement or trimming.
                    if *value != shape.paid || e.p_x.len() != shape.outputs.len() {
                        return Err(ReconstructError::CoinbaseTierMismatch);
                    }
                    for (key, expected) in e.p_x.iter().zip(&shape.outputs) {
                        if script_value.get(p2tr_script(key).as_slice()) != Some(expected) {
                            return Err(ReconstructError::CoinbaseTierMismatch);
                        }
                    }
                } else if paid.contains_key(&e.sp_id) {
                    return Err(ReconstructError::PaidMismatch(e.sp_id));
                }
            }
            // No unrepresented miner output may masquerade as pool rounding.
            for output in input.coinbase.iter().skip(1) {
                if output.script.len() == 34 && output.script[..2] == [0x51, 0x20] {
                    let key: [u8; 32] = output.script[2..]
                        .try_into()
                        .map_err(|_| ReconstructError::DuplicateScript)?;
                    if !all_keys.contains(&key) {
                        return Err(ReconstructError::CoinbaseTierMismatch);
                    }
                }
            }
        }
        TemplateMode::Jd => {
            if input.entries.iter().any(|e| e.owed_sats != e.sats) {
                return Err(ReconstructError::CoinbaseTierMismatch);
            }
            let weight = jd_weight(input.entries, &raw)?;
            let mut payout_scripts = BTreeSet::new();
            for e in input.entries {
                for p_x in &e.p_x {
                    payout_scripts.insert(p2tr_script(p_x));
                }
            }
            let fixed = fixed_outputs(input.coinbase, &paid, t_actual, &payout_scripts)?;
            for e in input.entries {
                let claim = match input.work_unit_version {
                    WorkUnitVersion::V1 => claim_jd(e.sats, t_actual, fixed, weight)?,
                    WorkUnitVersion::V2 => claim_jd_v2(e.sats, t_actual, fixed, weight)?,
                };
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
            let remainder = amount
                .checked_sub(*value)
                .ok_or(ReconstructError::PaidMismatch(id))?;
            let ark_due = if mode == TemplateMode::Pool
                && input.delivered.contains(&id)
                && !e.dest_id.is_empty()
            {
                remainder
            } else {
                0
            };
            let after = remainder - ark_due;
            LedgerRow {
                sp_id: id,
                rail: LedgerRail::Coinbase,
                paid_sats: *value,
                ark_due_sats: ark_due,
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
                if mode == TemplateMode::Pool {
                    spc_accounting::LADDER_BASE_SATS
                } else {
                    input.coinbase_threshold_sats
                },
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

    if mode == TemplateMode::Pool {
        let mut net_pool = i128::from(raw.pool_sats);
        for row in &ledger {
            let prior = i128::from(input.pending.get(&row.sp_id).copied().unwrap_or(0));
            let after = prior + i128::from(row.pending_delta);
            let fresh = i128::from(raw.raw.get(&row.sp_id).copied().unwrap_or(0));
            let noncoinbase = i128::from(row.ark_due_sats) + i128::from(row.forfeited_sats);
            if after < 0 || i128::from(row.paid_sats) + noncoinbase + after != fresh + prior {
                return Err(ReconstructError::CoinbaseTierMismatch);
            }
            net_pool = net_pool
                .checked_add(noncoinbase + i128::from(row.pending_delta))
                .ok_or(ReconstructError::Overflow)?;
        }
        if input
            .coinbase
            .first()
            .is_none_or(|output| i128::from(output.value) != net_pool)
        {
            return Err(ReconstructError::CoinbaseTierMismatch);
        }
    }

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
    // A pool-template witness commitment output is also an OP_RETURN
    // output (BIP 141, decision 2026-11-08-pool-witness-commitment); it is
    // not a JD carrier and must not flip a pool-template block into JD
    // mode. The real JD carrier is `OP_RETURN OP_PUSHBYTES_35 "SPc"
    // <ser_x(A_send)>` (`6a23535063…`, SPEC §2.4.2,
    // `spc_crypto::carriers::emit_op_return`), whose `6a23` push cannot
    // collide with the commitment's `6a24` push. Any other (unknown)
    // OP_RETURN output still selects JD mode: settlement then assumes the
    // least about the coinbase.
    if coinbase.iter().any(|o| {
        o.script.first() == Some(&0x6a) && !crate::witness::looks_like_commitment(&o.script)
    }) {
        TemplateMode::Jd
    } else {
        TemplateMode::Pool
    }
}

/// Estimated network difficulty at the finding block, as `Σd / n_shares`
/// from a sealed slice (every share under one template carries the same `d`,
/// so the division is exact). A slice sealed at the block's own height is the
/// finding template and is preferred; the newest stored slice is the fallback
/// when the finding slice has not sealed yet.
///
/// This is the **pre-migration fallback only** (decision
/// 2026-11-03-settlement-window-target): a difficulty retarget between the
/// reference slice and the finding block makes it wrong, so the exact
/// `N x d` is persisted in `distribution_snapshot.window_target` and used
/// whenever present. The caller warns when it has to estimate.
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
    let mut credit_difficulty = 0u128;
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
        match row.work_unit_version {
            WorkUnitVersion::V1 => {
                credit_difficulty = credit_difficulty.saturating_add(d);
                diff_fee_sum = diff_fee_sum.saturating_add(df);
                let slot = per_id.entry(c.sp_id).or_insert((0, 0));
                slot.0 = slot.0.saturating_add(d);
                slot.1 = slot.1.saturating_add(df);
            }
            WorkUnitVersion::V2 => {
                credit_difficulty =
                    spc_accounting::work_unit::checked_add_window_work(credit_difficulty, d)?;
                diff_fee_sum = spc_accounting::work_unit::checked_add_fee_work(diff_fee_sum, df)?;
                let slot = per_id.entry(c.sp_id).or_insert((0, 0));
                slot.0 = spc_accounting::work_unit::checked_add_window_work(slot.0, d)?;
                slot.1 = spc_accounting::work_unit::checked_add_fee_work(slot.1, df)?;
            }
        }
    }
    if row.work_unit_version == WorkUnitVersion::V2 && difficulty != credit_difficulty {
        return Err(ReconstructError::StoredDifficultyMismatch {
            slice_id: row.slice_id,
            stored: difficulty,
            credits: credit_difficulty,
        });
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
        work_unit_version: row.work_unit_version,
        work_unit_fingerprint: row.work_unit_fingerprint,
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
        // Mirror the front (SPEC §4.8, Fable rework 2026-11-08 R5): a mined
        // coinbase carrying a BIP 141 witness commitment spent 224 WU the
        // distribution could not pay out, and the front's routing budgeted
        // exactly that. Settlement must charge the same overhead - detected
        // from the mined coinbase, the truth this reconstruction runs on -
        // or the budget crosscheck at the boundary would keep more ids than
        // the coinbase pays and warn (or fail) spuriously. The front
        // constructs exactly one 38-byte zero-value commitment output last
        // (`assemble_witness` verified it before submission); the 6-byte
        // structural prefix identifies it here.
        overhead_wu: input.cfg.overhead_wu.saturating_add(
            if input
                .coinbase
                .iter()
                .any(|o| crate::witness::looks_like_commitment(&o.script))
            {
                crate::witness::WITNESS_COMMITMENT_OVERHEAD_WU
            } else {
                0
            },
        ),
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

#[derive(Clone)]
pub struct PayoutService {
    economic_pool: Option<crate::front::FrontConfig>,
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
    registration_stream: String,
    registration_ack_key: String,
    registration_ack_ttl_secs: u64,
    registration_ark_id: Option<[u8; 4]>,
    aggregate_server: Option<(String, bitcoin::Network)>,
}

/// How one group entry ends up after handling.
enum Disposition {
    /// Handled (or settle-orphaned): ack it.
    Done,
    /// Waiting on the chain: leave unacked; the pending pass retries it.
    Retry,
}
struct AbortTask(tokio::task::JoinHandle<()>);
type RecomputedLedger = (Vec<LedgerRow>, Vec<([u8; 32], i64)>, Option<i64>);
#[cfg(all(test, feature = "postgres-integration"))]
#[path = "aggregate_boundary_tests.rs"]
mod aggregate_boundary_tests;
impl Drop for AbortTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl PayoutService {
    async fn authorization_dispatcher(&self) -> Result<Option<AbortTask>, PayoutError> {
        if self.order_key.is_none() {
            return Ok(None);
        }
        let mut authorizer = self.clone();
        authorizer.streams = self.streams.independent().await?;
        let mut consumer = self.clone();
        consumer.streams = self.streams.independent().await?;
        consumer
            .streams
            .ensure_group_from(spc_ext::authorization::CONSUMPTION_STREAM, GROUP, "0")
            .await?;
        Ok(Some(AbortTask(tokio::spawn(async move {
            let requests = async {
                loop {
                    if let Err(error) = authorizer.process_authorizations().await {
                        tracing::warn!(error=%error,"authorization dispatcher unavailable");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
            };
            let consumptions = async {
                loop {
                    if let Err(error) = consumer.process_consumptions().await {
                        tracing::warn!(error=%error,"consumption observation dispatcher unavailable");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
            };
            tokio::join!(requests, consumptions);
        }))))
    }

    async fn process_consumptions(&self) -> Result<(), PayoutError> {
        use spc_ext::authorization::{Consumption, CONSUMED_KEY, CONSUMPTION_STREAM};
        let mut entries = self
            .streams
            .read_pending(CONSUMPTION_STREAM, GROUP, "consumption-observer", 64)
            .await?;
        if entries.is_empty() {
            entries = self
                .streams
                .read_group(CONSUMPTION_STREAM, GROUP, "consumption-observer", 16)
                .await?;
        }
        for (id, bytes) in entries {
            if let Ok(consumption) = Consumption::decode(&bytes) {
                let ack = match self
                    .store
                    .observe_authorization_consumption(&consumption)
                    .await
                {
                    Ok(ack) => ack,
                    Err(spc_store::postgres::StoreError::Invalid(reason)) => {
                        tracing::warn!(reason, "conflicting opaque consumption; no receipt");
                        None
                    }
                    Err(error) => return Err(error.into()),
                };
                if let Some(ack) = ack {
                    self.streams
                        .hset(
                            CONSUMED_KEY,
                            &consumption.field(),
                            &hex::encode(ack.encode()),
                        )
                        .await?;
                }
            }
            self.streams.ack(CONSUMPTION_STREAM, GROUP, &id).await?;
        }
        Ok(())
    }
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
            economic_pool: None,
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
            registration_stream: "spc:arkreg".to_string(),
            registration_ack_key: "spc:arkreg:ack".to_string(),
            registration_ack_ttl_secs: 3_600,
            registration_ark_id: None,
            aggregate_server: None,
        }
    }

    /// Supply the configured accounting parameters (SPEC §4.1). Settlement
    /// fails closed if the window/fee parameters differ from the front's.
    pub fn with_accounting(mut self, accounting: AccountingConfig) -> Self {
        self.accounting = accounting;
        self
    }
    pub fn with_economic_pool(mut self, pool: crate::front::FrontConfig) -> Self {
        self.economic_pool = Some(pool);
        self
    }
    pub fn with_aggregate_server(mut self, address: String, network: bitcoin::Network) -> Self {
        self.aggregate_server = Some((address, network));
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

    /// Bind registration validation to the pool's actual Ark server id.
    pub fn with_registration_ark_id(mut self, ark_id: [u8; 4]) -> Self {
        self.registration_ark_id = Some(ark_id);
        self
    }

    /// Override registration stream/cache names for an isolated integration
    /// resource. Production keeps the exact SPEC names.
    pub fn with_registration_streams(
        mut self,
        registration_stream: impl Into<String>,
        registration_ack_key: impl Into<String>,
    ) -> Self {
        self.registration_stream = registration_stream.into();
        self.registration_ack_key = registration_ack_key.into();
        self
    }

    pub fn with_registration_ack_ttl(mut self, ttl_secs: u64) -> Self {
        self.registration_ack_ttl_secs = ttl_secs.max(1);
        self
    }

    pub async fn run(&self) -> Result<(), PayoutError> {
        self.store.migrate().await?;
        let _observation_task = AbortTask(
            crate::payout_observation::spawn(self.store.clone(), self.streams.independent().await?)
                .await?,
        );
        self.streams
            .ensure_group_from(spc_ext::authorization::REQUESTS_STREAM, GROUP, "0")
            .await?;
        let _authorization_task = self.authorization_dispatcher().await?;
        for s in ["spc:slices", "spc:blocks"] {
            self.streams.ensure_group(s, GROUP).await?;
        }
        self.streams
            .ensure_group_from(&self.registration_stream, GROUP, "0")
            .await?;
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
            if let Err(e) = self.reconcile_economics().await {
                let _ = self
                    .store
                    .economic_hold("chain/economic reconciliation unavailable")
                    .await;
                tracing::warn!(error=%e,"economic readiness unavailable");
            }
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
                if let Err(e) = self.republish_unpublished_ledger_events().await {
                    tracing::error!(error = %e, "ledger republish failed");
                }
            }
            let mut did = false;
            for (stream, kind) in [("spc:slices", Kind::Slice), ("spc:blocks", Kind::Block)] {
                let entries = self.streams.read_group(stream, GROUP, CONSUMER, 16).await?;
                for (id, payload) in entries {
                    let res: Result<Disposition, PayoutError> = match kind {
                        Kind::Slice => self.on_slice(&payload).await.map(|_| Disposition::Done),
                        Kind::Block => self.on_block(&payload).await,
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
            if let Err(error) = self.process_registrations_once().await {
                tracing::warn!(error = %error, "registration processing failed");
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
                work_unit_version: e.work_unit_version,
                work_unit_fingerprint: e.work_unit_fingerprint,
                credits,
            })
            .await?;
        Ok(())
    }

    async fn on_block(&self, payload: &[u8]) -> Result<Disposition, PayoutError> {
        let e = BlockEvent::decode(payload)?;
        if let Some(old) = self.store.economic_event_digest(&e.block_hash).await? {
            if old != e.snapshot_id()? {
                return Err(PayoutError::Snapshot("conflicting found payload"));
            }
        }

        // Terminal states are absorbing (§7.4 idempotency); a redelivery
        // then changes nothing. The one exception is a confirmed block whose
        // `LedgerApplied` XADD never succeeded: that block stays unpublished
        // and is retried here (and by the periodic bounded republish).
        match self.store.block_status(&e.block_hash).await? {
            Some(BlockStatus::Confirmed) => {
                if self.store.ledger_published(&e.block_hash).await? == Some(false) {
                    self.republish_unpublished_ledger_events().await?;
                    if self.store.ledger_published(&e.block_hash).await? == Some(false) {
                        return Ok(Disposition::Retry);
                    }
                }
                return Ok(Disposition::Done);
            }
            Some(BlockStatus::Orphaned) => return Ok(Disposition::Done),
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
        if self.economic_pool.is_some() && active {
            let Some(job) = &e.economic else {
                self.store
                    .economic_hold("pool event omitted frozen P")
                    .await?;
                return Ok(Disposition::Retry);
            };
            let proof = self
                .chain
                .economic_evidence(&e.block_hash)
                .await
                .map_err(|e| PayoutError::LedgerEvent(e.to_string()))?
                .ok_or(PayoutError::Snapshot("missing raw ancestry"))?;
            if proof.parent_hash != job.parent_hash {
                return Err(PayoutError::Snapshot(
                    "frozen parent differs from mined parent",
                ));
            }
            let fingerprint = self.persist_economic_snapshot(&e).await?;
            let Some((ledger, _, observation)) = self.recompute(&e, fingerprint).await? else {
                return Ok(Disposition::Retry);
            };
            if self
                .store
                .economic_found_observed(&e, &ledger, false, observation)
                .await
                .is_err()
            {
                return Ok(Disposition::Retry);
            }
            let view = self.store.economic_view().await?;
            if view.anchor_height + 1 == e.height {
                self.store
                    .economic_advance(
                        e.height,
                        e.block_hash,
                        proof.parent_hash,
                        true,
                        proof.coinbase_digest,
                    )
                    .await?;
                let current = self.store.economic_view().await?;
                let hint = FrontEvent::EconomicChanged {
                    rev: self.store.bump_front_rev().await?,
                    economic_revision: current.revision,
                    anchor_hash: current.anchor_hash,
                };
                self.streams
                    .add(&self.front_events_stream, &hint.encode()?)
                    .await?;
            }
        }
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
                if e.economic.is_some() {
                    self.store.orphan(&e.block_hash).await?;
                }
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
            Some(e.snapshot_id()?)
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
                        // The exact target the front selected this window
                        // with; settlement must never re-derive it.
                        work_unit_version: e.work_unit_version,
                        work_unit_fingerprint: e.work_unit_fingerprint,
                        window_rule: e.window_rule,
                        window_target: Some(e.window_target.to_string()),
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
        let (ledger, deltas, _) = booked_ledger.unwrap_or_default();

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
        let bytes = event
            .encode()
            .map_err(|e| PayoutError::LedgerEvent(e.to_string()))?;
        if let Err(err) = self.streams.add(&self.front_events_stream, &bytes).await {
            tracing::warn!(
                error = %err,
                block = %display_hash(&e.block_hash),
                "LedgerApplied XADD failed; leaving the block unpublished for retry"
            );
            return Ok(Disposition::Retry);
        }
        self.store.mark_ledger_published(&e.block_hash).await?;
        // The Ark rows are now in Postgres. Emit from the store, not from
        // this in-memory ledger, so a payout restart and a duplicate booking
        // take the same idempotent path (SPEC §11.3).
        if let Err(e) = self.emit_due_ark_orders().await {
            tracing::warn!(error = %e, "ark order emission failed; the timer retries");
        }
        Ok(Disposition::Done)
    }

    async fn persist_economic_snapshot(&self, e: &BlockEvent) -> Result<[u8; 32], PayoutError> {
        let fingerprint = e.snapshot_id()?;
        self.store
            .insert_snapshot(&SnapshotRow {
                fingerprint,
                height: e.height,
                a_send_x: e.a_send_x,
                distribution_id: None,
                entries: json!(e.entries),
                high_water: json!(e.high_water),
                window_target: Some(e.window_target.to_string()),
                work_unit_version: e.work_unit_version,
                work_unit_fingerprint: e.work_unit_fingerprint,
                window_rule: e.window_rule,
            })
            .await?;
        Ok(fingerprint)
    }

    async fn reconcile_economics(&self) -> Result<(), PayoutError> {
        let Some(pool) = &self.economic_pool else {
            return Ok(());
        };
        if !self.chain.authoritative() {
            return Err(PayoutError::Snapshot(
                "economic chain authority unavailable",
            ));
        }
        let tip = self
            .chain
            .tip_height()
            .await
            .map_err(|e| PayoutError::LedgerEvent(e.to_string()))?;
        let genesis = self
            .chain
            .block_hash_at(0)
            .await
            .map_err(|e| PayoutError::LedgerEvent(e.to_string()))?
            .ok_or(PayoutError::Snapshot("genesis unavailable"))?;
        self.store.economic_initialize(genesis).await?;
        let mut view = self.store.economic_view().await?;
        if view.anchor_hash == [0; 32] {
            return Err(PayoutError::Snapshot("economic history checkpoint missing"));
        }
        let actual = self
            .chain
            .block_hash_at(view.anchor_height)
            .await
            .map_err(|e| PayoutError::LedgerEvent(e.to_string()))?;
        if actual != Some(view.anchor_hash) {
            self.store
                .economic_hold("canonical fork reconciliation")
                .await?;
            let mut fork = view.anchor_height.min(tip);
            loop {
                let node = self
                    .chain
                    .block_hash_at(fork)
                    .await
                    .map_err(|e| PayoutError::LedgerEvent(e.to_string()))?;
                if node.is_some() && node == self.store.economic_chain_hash(fork).await? {
                    break;
                }
                fork = fork
                    .checked_sub(1)
                    .ok_or(PayoutError::Snapshot("economic genesis mismatch"))?;
            }
            for hash in self.store.economic_displaced(fork).await? {
                self.store.orphan(&hash).await?;
            }
            let hash = self
                .store
                .economic_chain_hash(fork)
                .await?
                .ok_or(PayoutError::Snapshot("fork anchor missing"))?;
            self.store.economic_rewind(fork, hash).await?;
            view = self.store.economic_view().await?;
        }
        for height in
            view.anchor_height.saturating_add(1)..=tip.min(view.anchor_height.saturating_add(64))
        {
            let hash = self
                .chain
                .block_hash_at(height)
                .await
                .map_err(|e| PayoutError::LedgerEvent(e.to_string()))?
                .ok_or(PayoutError::Snapshot("canonical block missing"))?;
            let proof = self
                .chain
                .economic_evidence(&hash)
                .await
                .map_err(|e| PayoutError::LedgerEvent(e.to_string()))?
                .ok_or(PayoutError::Snapshot("raw coinbase evidence missing"))?;
            let relevant = proof.spc
                && proof.pool_script
                    == pool
                        .pool_script_at(height)
                        .map_err(|_| PayoutError::Snapshot("pool script derivation"))?;
            if relevant {
                let Some(event) = self.store.economic_saved_event(&hash).await? else {
                    self.store
                        .economic_hold("canonical pool event unresolved")
                        .await?;
                    return Ok(());
                };
                let fp = self.persist_economic_snapshot(&event).await?;
                let Some((ledger, _, observation)) = self.recompute(&event, fp).await? else {
                    return Err(PayoutError::Snapshot("recanonical ledger unavailable"));
                };
                if self.store.economic_lifecycle(&hash).await?.as_deref() == Some("loss_held") {
                    if self
                        .chain
                        .tip_height()
                        .await
                        .map_err(|e| PayoutError::LedgerEvent(e.to_string()))?
                        != tip
                        || self
                            .chain
                            .block_hash_at(height)
                            .await
                            .map_err(|e| PayoutError::LedgerEvent(e.to_string()))?
                            != Some(hash)
                    {
                        return Err(PayoutError::Snapshot(
                            "recanonical chain observation changed",
                        ));
                    }
                    self.store
                        .economic_restore_loss_held(&event, self.confirmation_depth, tip)
                        .await?;
                } else {
                    self.store
                        .economic_found_observed(&event, &ledger, true, observation)
                        .await?;
                }
            }
            self.store
                .economic_advance(
                    height,
                    hash,
                    proof.parent_hash,
                    relevant,
                    proof.coinbase_digest,
                )
                .await?;
        }
        let view = self.store.economic_view().await?;
        if view.anchor_height == tip {
            self.store
                .economic_confirm_anchor(tip, view.anchor_hash)
                .await?;
        }
        Ok(())
    }

    /// Recompute one block's ledger (SPEC §7.4). `None` means hold: the
    /// reconstruction cannot be trusted yet and the event stays unacked.
    async fn recompute(
        &self,
        e: &BlockEvent,
        fingerprint: [u8; 32],
    ) -> Result<Option<RecomputedLedger>, PayoutError> {
        if let Some(ledger) = self.store.economic_ledger(&e.block_hash).await? {
            let deltas = ledger
                .iter()
                .filter(|r| r.pending_delta != 0)
                .map(|r| (r.sp_id, r.pending_delta))
                .collect();
            return Ok(Some((
                ledger,
                deltas,
                self.store.reconstruction_observation(&e.block_hash).await?,
            )));
        }
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
                // The persisted window target is the exact `N x d` the front
                // used. A stored value that is not a u128 decimal string is a
                // corrupt row: fail closed, never guess. A missing value means
                // the row predates the column, and only then is the target
                // estimated (decision 2026-11-03-settlement-window-target).
                let window_target = match snapshot.window_target.as_deref() {
                    Some(text) => Some(
                        text.parse::<u128>()
                            .map_err(|_| PayoutError::Snapshot("window_target is not a u128"))?,
                    ),
                    None => {
                        tracing::warn!(
                            block = %display_hash(&e.block_hash),
                            height = e.height,
                            "distribution snapshot predates the persisted window target; \
                             N x d is estimated from the stored slices"
                        );
                        None
                    }
                };
                let high_water_mark = match single_front_high_water(&high_water) {
                    Ok(id) => id,
                    Err(err) => {
                        tracing::error!(error = %err, "cannot reconstruct the paid window; holding settlement");
                        return Ok(None);
                    }
                };
                let mut slices = self
                    .store
                    .window_slices_to(high_water_mark, WINDOW_SLICE_LIMIT)
                    .await?;
                // `slice` rows are never pruned, so the walk is capped. That
                // is only ambiguous when the window itself consumes every
                // returned row (checked by `reconstruct_settlement`).
                let window_truncated = e.economic.is_none()
                    && i64::try_from(slices.len()).unwrap_or(i64::MAX) >= WINDOW_SLICE_LIMIT;
                let pending = if let Some(job) = &e.economic {
                    let frozen = job
                        .prior()
                        .map_err(|_| PayoutError::Snapshot("frozen economic P"))?;
                    let mut selected = Vec::new();
                    for id in &job.slice_ids {
                        let row = slices
                            .iter()
                            .find(|s| s.slice_id == *id)
                            .ok_or(PayoutError::Snapshot("frozen visible slice missing"))?;
                        selected.push(row.clone());
                    }
                    if selected.windows(2).any(|w| w[0].slice_id <= w[1].slice_id) {
                        return Err(PayoutError::Snapshot("frozen slice ordering"));
                    }
                    slices = selected;
                    frozen
                } else if self.economic_pool.is_some() {
                    return Ok(None);
                } else {
                    self.store.pending_map().await?
                };
                let ids: Vec<[u8; 32]> = entries.iter().map(|entry| entry.sp_id).collect();
                let attempt = rand::random();
                let delivery_read = self
                    .store
                    .observe_settlement_delivery_read(e, &ids, &attempt)
                    .await?;
                let delivered = delivery_read.delivered;
                let input = ReconstructInput {
                    height: e.height,
                    high_water: &high_water,
                    entries: &entries,
                    slices: &slices,
                    work_unit_version: snapshot.work_unit_version,
                    work_unit_fingerprint: snapshot.work_unit_fingerprint,
                    window_rule: snapshot.window_rule,
                    window_target,
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
                        let body = serde_json::json!({"schema":"spc.settlement-reconstruction/1",
                            "binding":spc_store::observations::block_binding(e)?,"delivery_read_observation":delivery_read.observation_id,
                            "status":"complete","ledger":reconstructed.ledger,"pending_deltas":reconstructed.pending_deltas});
                        let observation = self
                            .store
                            .record_observation(
                                "settlement_reconstruction",
                                &e.block_hash,
                                Some(&attempt),
                                &body,
                            )
                            .await?;
                        Ok(Some((
                            reconstructed.ledger,
                            reconstructed.pending_deltas,
                            Some(observation),
                        )))
                    }
                    Err(err) => {
                        self.store.record_observation("settlement_reconstruction",&e.block_hash,Some(&attempt),&serde_json::json!({
                            "schema":"spc.settlement-reconstruction/1","binding":spc_store::observations::block_binding(e)?,
                            "delivery_read_observation":delivery_read.observation_id,"status":"unknown","error":err.to_string()})).await?;
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
                self.legacy_ledger(e)
                    .await
                    .map(|(ledger, deltas)| Some((ledger, deltas, None)))
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
        let kind = payload
            .first()
            .copied()
            .ok_or(PayoutError::RegistrationCode("invalid-message"))?;
        let result = match kind {
            ArkRegEvent::KIND => self.on_identity_reg(payload).await,
            ArkEpochRegEvent::KIND => self.on_epoch_reg(payload).await,
            _ => Err(PayoutError::RegistrationCode("invalid-message")),
        };
        match result {
            Err(PayoutError::RegistrationCode(code)) => match kind {
                ArkRegEvent::KIND => self.reject_identity(payload, code).await,
                ArkEpochRegEvent::KIND => self.reject_epoch(payload, code).await,
                _ => Err(PayoutError::RegistrationCode(code)),
            },
            Err(PayoutError::Codec(_)) => match kind {
                ArkRegEvent::KIND => self.reject_identity(payload, "invalid-message").await,
                ArkEpochRegEvent::KIND => self.reject_epoch(payload, "invalid-message").await,
                _ => Err(PayoutError::RegistrationCode("invalid-message")),
            },
            Err(PayoutError::Registration) => match kind {
                ArkRegEvent::KIND => self.reject_identity(payload, "invalid-message").await,
                ArkEpochRegEvent::KIND => self.reject_epoch(payload, "invalid-message").await,
                _ => Err(PayoutError::RegistrationCode("invalid-message")),
            },
            Err(PayoutError::Store(spc_store::postgres::StoreError::Invalid(_))) => match kind {
                ArkRegEvent::KIND => self.reject_identity(payload, "invalid-message").await,
                ArkEpochRegEvent::KIND => self.reject_epoch(payload, "invalid-message").await,
                _ => Err(PayoutError::RegistrationCode("invalid-message")),
            },
            other => other,
        }
    }

    async fn on_identity_reg(&self, payload: &[u8]) -> Result<(), PayoutError> {
        let e = ArkRegEvent::decode(payload)?;
        let scan = bitcoin::secp256k1::PublicKey::from_slice(&e.b_scan)
            .map_err(|_| PayoutError::RegistrationCode("invalid-message"))?;
        let spend = bitcoin::secp256k1::PublicKey::from_slice(&e.b_spend)
            .map_err(|_| PayoutError::RegistrationCode("invalid-message"))?;
        if spc_crypto::sp_id(&scan, &spend) != e.sp_id {
            return Err(PayoutError::RegistrationCode("bad-signature"));
        }
        let mailbox_mode = match e.mailbox_mode {
            0 => spc_ext::MailboxMode::None,
            1 => spc_ext::MailboxMode::Static,
            2 => spc_ext::MailboxMode::EpochDerived,
            3 => spc_ext::MailboxMode::EpochRegistered,
            _ => return Err(PayoutError::RegistrationCode("invalid-message")),
        };
        let epoch_fallback = match e.epoch_fallback {
            0 => spc_ext::EpochFallback::Static,
            1 => spc_ext::EpochFallback::Hold,
            _ => return Err(PayoutError::RegistrationCode("invalid-message")),
        };
        let request = spc_ext::SetArkIdentity {
            ark_id: e.ark_id,
            b_ark: e.b_ark,
            mailbox_mode,
            mailbox_pubkey: e.mailbox_pubkey.clone(),
            mailbox_dh: e.mailbox_dh.clone(),
            epoch_fallback,
            sig: e.sig,
            request_id: e.request_id,
        };
        let ext_scan = secp256k1::PublicKey::from_slice(&e.b_scan)
            .map_err(|_| PayoutError::RegistrationCode("invalid-message"))?
            .x_only_public_key()
            .0;
        let configured_ark = self
            .registration_ark_id
            .ok_or(PayoutError::RegistrationCode("wrong-ark-server"))?;
        if let Some(code) = request.error_code(&e.sp_id, &ext_scan, &e.b_spend, &configured_ark) {
            return Err(PayoutError::RegistrationCode(code));
        }
        let digest = request
            .signed_fields_digest(&e.sp_id)
            .map_err(|_| PayoutError::RegistrationCode("invalid-message"))?;
        let stored_mode = match mailbox_mode {
            spc_ext::MailboxMode::None => MailboxMode::None,
            spc_ext::MailboxMode::Static => MailboxMode::Static,
            spc_ext::MailboxMode::EpochDerived => MailboxMode::EpochDerived,
            spc_ext::MailboxMode::EpochRegistered => MailboxMode::EpochRegistered,
        };
        let stored_fallback = match epoch_fallback {
            spc_ext::EpochFallback::Static => EpochFallback::Static,
            spc_ext::EpochFallback::Hold => EpochFallback::Hold,
        };
        let effect = RegistrationEffect::Delivery {
            identity: SpIdentityRow {
                sp_id: e.sp_id,
                b_scan: e.b_scan.to_vec(),
                b_spend: e.b_spend.to_vec(),
                network: identity_network(self.network, 1),
            },
            delivery: ArkDeliveryRow {
                registration_commit: None,
                sp_id: e.sp_id,
                ark_id: e.ark_id,
                b_ark: e.b_ark.to_vec(),
                mailbox_mode: stored_mode,
                mailbox_m: (!e.mailbox_pubkey.is_empty()).then_some(e.mailbox_pubkey),
                mailbox_d: (!e.mailbox_dh.is_empty()).then_some(e.mailbox_dh),
                epoch_fallback: stored_fallback,
            },
        };
        let resolution = self
            .store
            .apply_registration(effect, e.request_id, digest)
            .await?;
        self.complete_registration(&resolution, e.attempt, Vec::new())
            .await
    }

    async fn on_epoch_reg(&self, payload: &[u8]) -> Result<(), PayoutError> {
        let e = ArkEpochRegEvent::decode(payload)?;
        let identity = self
            .store
            .identity(&e.sp_id)
            .await?
            .ok_or(PayoutError::RegistrationCode("invalid-message"))?;
        let scan = secp256k1::PublicKey::from_slice(&identity.b_scan)
            .map_err(|_| PayoutError::RegistrationCode("invalid-message"))?
            .x_only_public_key()
            .0;
        let request = spc_ext::RegisterEpochMailboxes {
            entries: e.entries.clone(),
            sig: e.sig,
            request_id: e.request_id,
        };
        request
            .verify(&e.sp_id, &scan)
            .map_err(|_| PayoutError::RegistrationCode("bad-signature"))?;
        let digest = request
            .signed_fields_digest(&e.sp_id)
            .map_err(|_| PayoutError::RegistrationCode("invalid-message"))?;
        let delivery = self.store.ark_delivery(&e.sp_id).await?;
        let Some(delivery) = delivery else {
            return Err(PayoutError::RegistrationCode("invalid-message"));
        };
        if delivery.mailbox_mode != MailboxMode::EpochRegistered {
            return Err(PayoutError::RegistrationCode("bad-mailbox-fields"));
        }

        let mut entries = Vec::with_capacity(e.entries.len());
        for entry in &e.entries {
            let height = u32::from_be_bytes(
                entry[..4]
                    .try_into()
                    .map_err(|_| PayoutError::RegistrationCode("invalid-message"))?,
            );
            let mailbox_mi: [u8; 33] = entry[4..]
                .try_into()
                .map_err(|_| PayoutError::RegistrationCode("invalid-message"))?;
            secp256k1::PublicKey::from_slice(&mailbox_mi)
                .map_err(|_| PayoutError::RegistrationCode("invalid-point"))?;
            entries.push(ArkEpochMailboxRow {
                sp_id: e.sp_id,
                height,
                mailbox_mi,
            });
        }
        let resolution = self
            .store
            .apply_registration(
                RegistrationEffect::EpochMailboxes {
                    sp_id: e.sp_id,
                    entries: entries.clone(),
                },
                e.request_id,
                digest,
            )
            .await?;
        self.complete_registration(&resolution, e.attempt, entries)
            .await
    }

    async fn reject_identity(&self, payload: &[u8], code: &'static str) -> Result<(), PayoutError> {
        let e = ArkRegEvent::decode(payload)
            .map_err(|_| PayoutError::RegistrationCode("invalid-message"))?;
        let request = identity_request(&e)?;
        let digest = request
            .signed_fields_digest(&e.sp_id)
            .map_err(|_| PayoutError::RegistrationCode("invalid-message"))?;
        let resolution = self
            .store
            .reject_registration(
                &identity_effect(&e, identity_network(self.network, 1)),
                e.request_id,
                digest,
                code,
            )
            .await?;
        self.complete_registration(&resolution, e.attempt, Vec::new())
            .await
    }

    async fn reject_epoch(&self, payload: &[u8], code: &'static str) -> Result<(), PayoutError> {
        let e = ArkEpochRegEvent::decode(payload)
            .map_err(|_| PayoutError::RegistrationCode("invalid-message"))?;
        let request = spc_ext::RegisterEpochMailboxes {
            entries: e.entries.clone(),
            sig: e.sig,
            request_id: e.request_id,
        };
        let digest = request
            .signed_fields_digest(&e.sp_id)
            .map_err(|_| PayoutError::RegistrationCode("invalid-message"))?;
        let entries = epoch_rows(&e)?;
        let resolution = self
            .store
            .reject_registration(
                &RegistrationEffect::EpochMailboxes {
                    sp_id: e.sp_id,
                    entries,
                },
                e.request_id,
                digest,
                code,
            )
            .await?;
        self.complete_registration(&resolution, e.attempt, Vec::new())
            .await
    }

    /// Cache the already-durable resolution, then propagate the current full
    /// capability to every front. A duplicate after Redis cache expiry follows
    /// exactly this path again; a superseded replay never restores old fields.
    async fn complete_registration(
        &self,
        resolution: &RegistrationResolution,
        attempt: [u8; 16],
        epoch_mailboxes: Vec<ArkEpochMailboxRow>,
    ) -> Result<(), PayoutError> {
        let expires_at = unix_secs().saturating_add(self.registration_ack_ttl_secs);
        let ack = ArkRegistrationAck::from_resolution(resolution, &attempt, expires_at);
        let bytes = serde_json::to_string(&ack)
            .map_err(|e| PayoutError::LedgerEvent(format!("registration ack json: {e}")))?;
        self.streams
            .hset(
                &self.registration_ack_key,
                &registration_ack_field(&resolution.sp_id, resolution.request_id, &attempt),
                &bytes,
            )
            .await?;
        self.streams
            .expire(&self.registration_ack_key, self.registration_ack_ttl_secs)
            .await?;
        if !resolution.is_success() {
            return Ok(());
        }

        let identity = self
            .store
            .identity(&resolution.sp_id)
            .await?
            .ok_or(PayoutError::Registration)?;
        let delivery = self
            .store
            .ark_delivery(&resolution.sp_id)
            .await?
            .ok_or(PayoutError::Registration)?;
        let state = match resolution.outcome {
            RegistrationOutcome::Applied => spc_ext::RegistrationSuccessState::Applied,
            RegistrationOutcome::AlreadyApplied => {
                spc_ext::RegistrationSuccessState::AlreadyApplied
            }
            RegistrationOutcome::Superseded => spc_ext::RegistrationSuccessState::Superseded,
            _ => return Err(PayoutError::Registration),
        };
        let event_rev = self.store.bump_front_rev().await?;
        let event = FrontEvent::ArkRegApplied {
            rev: event_rev,
            sp_id: resolution.sp_id,
            request_id: resolution.request_id,
            registration_rev: resolution.registration_rev,
            state,
            identity,
            delivery,
            epoch_mailboxes,
        };
        let bytes = event
            .encode()
            .map_err(|e| PayoutError::LedgerEvent(e.to_string()))?;
        self.streams.add(&self.front_events_stream, &bytes).await?;
        Ok(())
    }

    /// One deterministic registration-stream pass, also used by real
    /// PostgreSQL/Redis integration tests. Postgres failures leave entries
    /// unacknowledged; structurally invalid entries are acknowledged and logged
    /// because no retry can make them valid.
    pub async fn process_registrations_once(&self) -> Result<usize, PayoutError> {
        self.streams
            .ensure_group_from(&self.registration_stream, GROUP, "0")
            .await?;
        let new = self
            .streams
            .read_group(&self.registration_stream, GROUP, CONSUMER, 32)
            .await?;
        let pending = self
            .streams
            .read_pending(&self.registration_stream, GROUP, CONSUMER, 64)
            .await?;
        let mut seen = std::collections::BTreeSet::new();
        let mut handled = 0usize;
        for (id, payload) in new.into_iter().chain(pending) {
            if !seen.insert(id.clone()) {
                continue;
            }
            match self.on_reg(&payload).await {
                Ok(()) => {
                    self.streams
                        .ack(&self.registration_stream, GROUP, &id)
                        .await?;
                    handled += 1;
                }
                Err(PayoutError::RegistrationCode(code)) => {
                    tracing::warn!(id, code, "invalid registration stream entry rejected");
                    self.streams
                        .ack(&self.registration_stream, GROUP, &id)
                        .await?;
                }
                Err(error) => return Err(error),
            }
        }
        Ok(handled)
    }

    /// Re-emit `LedgerApplied` only for confirmed blocks whose event has not
    /// been published yet (SPEC §5.2, §7.4).
    ///
    /// `block_found.ledger_published` is set only after a successful XADD, so
    /// a dead front cursor cannot make this timer grow the stream: a healthy
    /// front leaves the set empty. A failed XADD leaves the block unpublished
    /// and the next pass retries it; the front's block-hash dedupe makes a
    /// duplicate harmless.
    async fn republish_unpublished_ledger_events(&self) -> Result<(), PayoutError> {
        let unpublished = self
            .store
            .unpublished_ledger_events(spc_store::postgres::APPLIED_BLOCKS_LIMIT)
            .await?;
        for (block_hash, pending_deltas) in unpublished {
            let rev = self.store.bump_front_rev().await?;
            let event = FrontEvent::LedgerApplied {
                rev,
                block_hash,
                republish: true,
                pending_deltas,
            };
            let Ok(bytes) = event.encode() else {
                return Err(PayoutError::LedgerEvent(format!(
                    "cannot encode LedgerApplied for {block_hash:?}"
                )));
            };
            if let Err(e) = self.streams.add(&self.front_events_stream, &bytes).await {
                tracing::warn!(
                    error = %e,
                    block = %display_hash(&block_hash),
                    "ledger republish XADD failed; the next pass retries"
                );
                return Err(PayoutError::Redis(e));
            }
            self.store.mark_ledger_published(&block_hash).await?;
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
        if self.aggregate_server.is_some() {
            return self.emit_aggregate_orders().await;
        }
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
            // A due row is not active-chain evidence. Never publish while
            // canonical membership is unknown or its block has been removed.
            if !matches!(
                self.chain
                    .is_active(&row.block_hash, row.block_height)
                    .await,
                Ok(true)
            ) {
                tracing::warn!("ark_emission_noncanonical: holding for reorg reconciliation");
                continue;
            }
            if row.is_remainder && row.sats < spc_store::aggregation::MIN_OVERAGE_SATS {
                // SPEC requires aggregation, but AGGREGATEDEST is unresolved.
                // Keep the booked liability visible instead of creating a
                // forbidden micro-VTXO or guessing another epoch's destination.
                tracing::warn!("aggregate_destination_unbound: remainder remains scheduled");
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

    async fn canonical_members(
        &self,
        members: &[spc_store::aggregate_store::AggregateMember],
    ) -> Option<(u32, [u8; 32], BTreeSet<[u8; 32]>)> {
        self.observe_canonical_members(members).await.canonical()
    }

    async fn observe_canonical_members(
        &self,
        members: &[spc_store::aggregate_store::AggregateMember],
    ) -> spc_store::observations::CanonicalRead {
        use spc_store::observations::{CanonicalMemberRead, CanonicalRead};
        let clock = || {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .and_then(|d| u64::try_from(d.as_millis()).ok())
        };
        let mut read = CanonicalRead {
            schema: "spc.canonical-read/1".into(),
            source_session: crate::evidence::session_id(),
            started_unix_ms: clock(),
            finished_unix_ms: None,
            authoritative: self.chain.authoritative(),
            tip: None,
            tip_hash: None,
            tip_after: None,
            tip_hash_after: None,
            members: members
                .iter()
                .map(|m| CanonicalMemberRead {
                    height: m.block_height,
                    expected_hash: m.block_hash,
                    observed_hash: None,
                    read_status: "not_read".into(),
                })
                .collect(),
            outcome: String::new(),
        };
        let outcome: Result<(), &str> = async {
            if !read.authoritative {
                return Err("not_authoritative");
            }
            let tip = self
                .chain
                .tip_height()
                .await
                .map_err(|_| "tip_read_failed")?;
            read.tip = Some(tip);
            read.tip_hash = self
                .chain
                .block_hash_at(tip)
                .await
                .map_err(|_| "tip_hash_read_failed")?;
            if read.tip_hash.is_none() {
                return Err("tip_hash_unavailable");
            }
            for member in &mut read.members {
                member.observed_hash = match self.chain.block_hash_at(member.height).await {
                    Ok(hash) => hash,
                    Err(_) => {
                        member.read_status = "read_failed".into();
                        return Err("member_hash_read_failed");
                    }
                };
                member.read_status = if member.observed_hash.is_some() {
                    "present"
                } else {
                    "unavailable"
                }
                .into();
                if member.observed_hash.is_none() {
                    return Err("member_hash_unavailable");
                }
            }
            read.tip_after = Some(
                self.chain
                    .tip_height()
                    .await
                    .map_err(|_| "tip_after_read_failed")?,
            );
            read.tip_hash_after = self
                .chain
                .block_hash_at(tip)
                .await
                .map_err(|_| "tip_hash_after_read_failed")?;
            if read.tip != read.tip_after || read.tip_hash != read.tip_hash_after {
                return Err("tip_changed");
            }
            Ok(())
        }
        .await;
        read.finished_unix_ms = clock();
        read.outcome = match outcome {
            Ok(()) if read.started_unix_ms.is_some() && read.finished_unix_ms.is_some() => {
                "complete"
            }
            Ok(()) => "clock_unavailable",
            Err(reason) => reason,
        }
        .into();
        read
    }

    async fn process_authorizations(&self) -> Result<(), PayoutError> {
        use spc_ext::authorization::{AuthorizationRequest, GRANTS_KEY, REQUESTS_STREAM};
        let Some(signer) = &self.order_key else {
            return Ok(());
        };
        let requests = self
            .streams
            .read_group(REQUESTS_STREAM, GROUP, CONSUMER, 16)
            .await?;
        for (stream_id, bytes) in requests {
            if let Ok(request) = AuthorizationRequest::decode(&bytes) {
                if let Some(frozen) = self.store.aggregate(&request.order_id).await? {
                    let canonical_read = self.observe_canonical_members(&frozen.members).await;
                    {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map_err(|_| PayoutError::Overflow)?
                            .as_secs();
                        if let Some(grant) = self
                            .store
                            .authorize_aggregate_observed(&request, &canonical_read, now, signer)
                            .await?
                        {
                            let field = format!(
                                "{}:{}",
                                hex::encode(request.order_id),
                                hex::encode(request.nonce)
                            );
                            self.streams
                                .hset(GRANTS_KEY, &field, &hex::encode(grant.encode()))
                                .await?;
                            self.streams.expire(GRANTS_KEY, 60).await?; // cleanup only, never send authority
                        }
                    }
                }
            }
            self.streams.ack(REQUESTS_STREAM, GROUP, &stream_id).await?;
        }
        Ok(())
    }

    #[cfg(feature = "ark")]
    async fn emit_aggregate_orders(&self) -> Result<(), PayoutError> {
        use spc_store::aggregate_store::{bounded_members, DeliveryLimits};
        let Some((address, network)) = &self.aggregate_server else {
            return Ok(());
        };
        let Some(signer) = &self.order_key else {
            return Ok(());
        };
        let server = match server_rpc::ServerConnection::builder()
            .address(address.clone())
            .network(*network)
            .connect()
            .await
        {
            Ok(server) => server,
            Err(error) => {
                tracing::warn!(error=%error,"aggregate delivery policy unavailable");
                return Ok(());
            }
        };
        let info = server.ark_info().await;
        let actual_id =
            <[u8; 4]>::from(ark::address::ArkId::from_server_pubkey(info.server_pubkey));
        if self.registration_ark_id.is_some_and(|id| id != actual_id)
            || info.fees.validate().is_err()
        {
            return Err(PayoutError::Snapshot("Ark provider policy mismatch"));
        }
        // Pinned stock arkoor supports positive subdust outputs and has no
        // arkoor fee entry; max_vtxo_amount is discovered from actual ArkInfo.
        // Board/refresh/offboard fees remain provider-discovered pool expense.
        let circuit = if self.max_order_sats == 0 {
            5_000_000
        } else {
            self.max_order_sats
        };
        let limits = DeliveryLimits {
            min_sats: 1,
            max_sats: info
                .max_vtxo_amount
                .map_or(circuit, |v| v.to_sat().min(circuit)),
            max_members: 2048,
        };
        let emissions = self.store.scheduled_ark_emissions().await?;
        let identities: BTreeSet<_> = emissions.iter().map(|e| e.sp_id).collect();
        for identity in identities {
            let candidates = self.store.aggregate_candidates(&identity).await?;
            let Some((tip, tip_hash, canonical)) = self.canonical_members(&candidates).await else {
                continue;
            };
            let candidates = candidates
                .into_iter()
                .filter(|r| canonical.contains(&r.block_hash))
                .collect();
            let selected = bounded_members(
                candidates,
                tip,
                limits,
                spc_store::aggregation::MIN_OVERAGE_SATS,
                spc_store::aggregation::OVERAGE_MAX_DELAY_BLOCKS,
            )?;
            if selected.is_empty() {
                continue;
            }
            self.store
                .freeze_aggregate_bound(
                    &selected,
                    tip,
                    tip_hash,
                    signer,
                    info.server_pubkey.serialize(),
                )
                .await?;
        }
        for id in self.store.publishable_aggregate_ids().await? {
            let Some(frozen) = self.store.aggregate(&id).await? else {
                continue;
            };
            let Some((_, _, canonical)) = self.canonical_members(&frozen.members).await else {
                continue;
            };
            if canonical.len() != frozen.members.len() {
                if !self
                    .store
                    .cancel_aggregate_without_authorization(&id, &canonical)
                    .await?
                {
                    self.store.block_aggregate_unknown(&id).await?;
                }
                continue;
            }
            self.store.publish_aggregate(&id).await?;
            let bytes = frozen
                .order
                .encode()
                .map_err(|_| PayoutError::Snapshot("aggregate order"))?;
            self.streams.add(&self.orders_stream, &bytes).await?;
        }
        Ok(())
    }
    #[cfg(not(feature = "ark"))]
    async fn emit_aggregate_orders(&self) -> Result<(), PayoutError> {
        tracing::warn!("aggregate delivery requires the stock Ark policy/proof adapter");
        Ok(())
    }

    async fn apply_proved_receipt(&self, bytes: &[u8]) -> Result<(), PayoutError> {
        use ark::ProtocolEncoding;
        use bitcoin::hashes::Hash;
        let receipt = spc_ext::authorization::ProvedReceipt::decode(bytes)
            .map_err(|_| PayoutError::Snapshot("aggregate receipt encoding"))?;
        let grant = &receipt.grant;
        let Some(signer) = &self.order_key else {
            return Err(PayoutError::Snapshot("receipt signer unavailable"));
        };
        grant
            .verify_signature(&signer.x_only_public_key().0)
            .map_err(|_| PayoutError::Snapshot("receipt grant signature"))?;
        let frozen = self
            .store
            .aggregate(&grant.request.order_id)
            .await?
            .ok_or(PayoutError::Snapshot("unknown aggregate receipt"))?;
        let address: ark::Address = frozen
            .order
            .ark_address
            .parse()
            .map_err(|_| PayoutError::Snapshot("aggregate address"))?;
        let server = frozen
            .server_pubkey
            .ok_or(PayoutError::Snapshot("frozen server proof unavailable"))?;
        let mut total = 0u64;
        let mut expiry = u32::MAX;
        let mut ids = BTreeSet::new();
        for proof in &receipt.vtxos {
            let vtxo = ark::Vtxo::<ark::vtxo::Full>::deserialize(proof)
                .map_err(|_| PayoutError::Snapshot("VTXO proof encoding"))?;
            if vtxo.serialize() != *proof
                || vtxo.policy() != address.policy()
                || vtxo.server_pubkey().serialize() != server
            {
                return Err(PayoutError::Snapshot("VTXO destination/proof mismatch"));
            }
            let anchor = self
                .chain
                .transaction(&vtxo.chain_anchor().txid.to_byte_array())
                .await
                .map_err(|e| PayoutError::LedgerEvent(e.to_string()))?
                .ok_or(PayoutError::Snapshot(
                    "VTXO anchor unavailable; outcome unknown",
                ))?;
            vtxo.validate(&anchor)
                .map_err(|_| PayoutError::Snapshot("invalid VTXO signature/ancestry proof"))?;
            total = total
                .checked_add(vtxo.amount().to_sat())
                .ok_or(PayoutError::Overflow)?;
            expiry = expiry.min(vtxo.expiry_height().to_u32());
            if !ids.insert(vtxo.id().to_bytes()) {
                return Err(PayoutError::Snapshot("duplicate VTXO proof"));
            }
        }
        if total != receipt.sats || total != frozen.order.sats {
            return Err(PayoutError::Snapshot("partial aggregate delivery"));
        }
        let Some((_, _, canonical)) = self.canonical_members(&frozen.members).await else {
            return Err(PayoutError::Snapshot(
                "aggregate receipt canonicality unknown",
            ));
        };
        let first = ids
            .first()
            .ok_or(PayoutError::Snapshot("empty aggregate delivery"))?;
        self.store
            .resolve_aggregate_sent(
                &grant.request,
                grant.revision,
                total,
                spc_store::aggregate_store::SentDeliveryProof {
                    vtxo_id: first,
                    expiry,
                    proof: bytes,
                },
                &canonical,
            )
            .await?;
        Ok(())
    }

    /// Consume `spc:ark:receipts` and close out scheduled payouts
    /// (SPEC §11.3). `Sent` records the VTXO and never credits pending;
    /// `Failed` moves the sats to pending, never to forfeit.
    async fn process_receipts_once(&self) -> Result<(), PayoutError> {
        let mut entries = self
            .streams
            .read_group(&self.receipts_stream, GROUP, CONSUMER, 64)
            .await?;
        let cursor = self.store.receipt_cursor(&self.receipts_stream).await?;
        let pending = self
            .streams
            .read_pending_after(&self.receipts_stream, GROUP, CONSUMER, 64, &cursor)
            .await?;
        let next_cursor = pending
            .last()
            .map(|(id, _)| id.clone())
            .unwrap_or_else(|| "0".into());
        entries.extend(pending);
        let mut seen = BTreeSet::new();
        let mut terminal_count = 0usize;
        let mut retry_count = 0usize;
        for (id, payload) in entries {
            if !seen.insert(id.clone()) {
                continue;
            }
            let result = if payload.starts_with(b"SPRP") {
                self.apply_proved_receipt(&payload).await
            } else if self.aggregate_server.is_some() {
                Err(PayoutError::Snapshot(
                    "legacy receipt forbidden for aggregate mode",
                ))
            } else {
                match ArkSendReceipt::decode(&payload) {
                    Ok(receipt) => self.apply_receipt(&receipt).await,
                    Err(_) => Err(PayoutError::Snapshot("aggregate receipt encoding")),
                }
            };
            match result {
                Ok(()) => {
                    self.store
                        .record_receipt_disposition(
                            &self.receipts_stream,
                            &id,
                            "applied",
                            "verified",
                            &payload,
                        )
                        .await?;
                    self.streams.ack(&self.receipts_stream, GROUP, &id).await?;
                }
                Err(error) => {
                    let terminal = matches!(
                        &error,
                        PayoutError::Snapshot(
                            "legacy receipt forbidden for aggregate mode"
                                | "aggregate receipt encoding"
                                | "receipt grant signature"
                                | "aggregate address"
                                | "VTXO proof encoding"
                                | "VTXO destination/proof mismatch"
                                | "invalid VTXO signature/ancestry proof"
                                | "duplicate VTXO proof"
                                | "partial aggregate delivery"
                                | "empty aggregate delivery"
                        )
                    ) || matches!(
                        &error,
                        PayoutError::Store(spc_store::postgres::StoreError::Invalid(
                            "aggregate sent proof"
                                | "aggregate partial or mismatched receipt"
                                | "aggregate receipt authorization mismatch"
                                | "aggregate receipt replay mismatch"
                                | "aggregate member cannot accept a legacy Sent receipt"
                        ))
                    );
                    if terminal {
                        terminal_count += 1;
                    } else {
                        retry_count += 1;
                    }
                    self.store
                        .record_receipt_disposition(
                            &self.receipts_stream,
                            &id,
                            if terminal {
                                "terminal_invalid"
                            } else {
                                "retry_unknown"
                            },
                            if terminal {
                                "invalid proof/binding"
                            } else {
                                "proof source unavailable"
                            },
                            &payload,
                        )
                        .await?;
                    if terminal {
                        self.streams.ack(&self.receipts_stream, GROUP, &id).await?;
                    }
                }
            }
        }
        self.store
            .set_receipt_cursor(&self.receipts_stream, &next_cursor)
            .await?;
        if terminal_count + retry_count > 0 {
            tracing::warn!(
                terminal_count,
                retry_count,
                "receipt batch retained evidence without releasing uncertain reservations"
            );
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
        // Store chooses the actual deletion horizon per stored row version;
        // this legacy calculation never expands v1 retention to the v2 limit.
        let retain = scrub_retain_windows(
            self.scrub.retain_windows,
            self.confirmation_depth,
            self.accounting.window_n,
        );
        self.store
            .scrub(
                tip,
                retain,
                V2_RETENTION_HEIGHT_BLOCKS,
                self.scrub.scrub_after_blocks,
            )
            .await?;
        Ok(())
    }
}

fn identity_request(e: &ArkRegEvent) -> Result<spc_ext::SetArkIdentity, PayoutError> {
    let mailbox_mode = match e.mailbox_mode {
        0 => spc_ext::MailboxMode::None,
        1 => spc_ext::MailboxMode::Static,
        2 => spc_ext::MailboxMode::EpochDerived,
        3 => spc_ext::MailboxMode::EpochRegistered,
        _ => return Err(PayoutError::RegistrationCode("invalid-message")),
    };
    let epoch_fallback = match e.epoch_fallback {
        0 => spc_ext::EpochFallback::Static,
        1 => spc_ext::EpochFallback::Hold,
        _ => return Err(PayoutError::RegistrationCode("invalid-message")),
    };
    Ok(spc_ext::SetArkIdentity {
        ark_id: e.ark_id,
        b_ark: e.b_ark,
        mailbox_mode,
        mailbox_pubkey: e.mailbox_pubkey.clone(),
        mailbox_dh: e.mailbox_dh.clone(),
        epoch_fallback,
        sig: e.sig,
        request_id: e.request_id,
    })
}

fn identity_effect(e: &ArkRegEvent, network: i16) -> RegistrationEffect {
    let mailbox_mode = match e.mailbox_mode {
        0 => MailboxMode::None,
        1 => MailboxMode::Static,
        2 => MailboxMode::EpochDerived,
        3 => MailboxMode::EpochRegistered,
        _ => MailboxMode::None,
    };
    let epoch_fallback = match e.epoch_fallback {
        0 => EpochFallback::Static,
        1 => EpochFallback::Hold,
        _ => EpochFallback::Static,
    };
    RegistrationEffect::Delivery {
        identity: SpIdentityRow {
            sp_id: e.sp_id,
            b_scan: e.b_scan.to_vec(),
            b_spend: e.b_spend.to_vec(),
            network,
        },
        delivery: ArkDeliveryRow {
            registration_commit: None,
            sp_id: e.sp_id,
            ark_id: e.ark_id,
            b_ark: e.b_ark.to_vec(),
            mailbox_mode,
            mailbox_m: (!e.mailbox_pubkey.is_empty()).then_some(e.mailbox_pubkey.clone()),
            mailbox_d: (!e.mailbox_dh.is_empty()).then_some(e.mailbox_dh.clone()),
            epoch_fallback,
        },
    }
}

fn epoch_rows(e: &ArkEpochRegEvent) -> Result<Vec<ArkEpochMailboxRow>, PayoutError> {
    e.entries
        .iter()
        .map(|entry| {
            let height = u32::from_be_bytes(
                entry[..4]
                    .try_into()
                    .map_err(|_| PayoutError::RegistrationCode("invalid-message"))?,
            );
            let mailbox_mi: [u8; 33] = entry[4..]
                .try_into()
                .map_err(|_| PayoutError::RegistrationCode("invalid-message"))?;
            Ok(ArkEpochMailboxRow {
                sp_id: e.sp_id,
                height,
                mailbox_mi,
            })
        })
        .collect()
}

pub fn registration_ack_field(sp_id: &[u8; 32], request_id: u32, attempt: &[u8; 16]) -> String {
    format!(
        "reg:{}:{request_id}:{}",
        hex::encode(sp_id),
        hex::encode(attempt)
    )
}

fn unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
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
            work_unit_version: WorkUnitVersion::V1,
            work_unit_fingerprint: None,
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
            owed_sats: sats,
            rail,
            p_x: p_x.into_iter().collect(),
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

    /// Explicit current-ladder settlement fixture, with independently allocated
    /// valid output points. Crypto ownership is tested by the scanner vectors.
    fn ladder_fixture(owed: &[(SpId, u64)], pool: u64) -> (Vec<Entry>, Vec<CoinbaseOutput>) {
        use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
        let secp = Secp256k1::new();
        let mut entries = Vec::new();
        let mut outputs = vec![output(pool_script(), pool)];
        for (id, owed) in owed {
            let shape = spc_accounting::decompose(*owed, 1024).unwrap();
            let mut keys = Vec::new();
            for (k, sats) in shape.outputs.into_iter().enumerate() {
                let mut secret = [0u8; 32];
                secret[0] = 1;
                secret[1..28].copy_from_slice(&id[..27]);
                secret[28..].copy_from_slice(&(k as u32).to_be_bytes());
                let key =
                    PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&secret).unwrap())
                        .x_only_public_key()
                        .0
                        .serialize();
                outputs.push(output(p2tr_script(&key), sats));
                keys.push(key);
            }
            entries.push(Entry {
                sp_id: *id,
                owed_sats: *owed,
                rail: Rail::Coinbase,
                p_x: keys,
                dest_id: vec![],
                sats: shape.paid,
            });
        }
        outputs[1..].sort_by(|a, b| a.script.cmp(&b.script));
        (entries, outputs)
    }

    #[test]
    fn ladder_grouped_settlement_binds_full_owed_and_witness_shape() {
        use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
        let id = id(7);
        let secp = Secp256k1::new();
        let keys: Vec<_> = (1..=4)
            .map(|i| {
                PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&[i; 32]).unwrap())
                    .x_only_public_key()
                    .0
                    .serialize()
            })
            .collect();
        let entries = vec![Entry {
            sp_id: id,
            owed_sats: 9_500_123,
            rail: Rail::Coinbase,
            p_x: keys.clone(),
            dest_id: vec![1],
            sats: 9_500_000,
        }];
        let mut coinbase = vec![output(pool_script(), 123)];
        for (key, amount) in keys.iter().zip([5_000_000, 3_000_000, 1_000_000, 500_000]) {
            coinbase.push(output(p2tr_script(key), amount));
        }
        coinbase[1..].sort_by(|a, b| a.script.cmp(&b.script));
        coinbase.push(output(crate::witness::commitment_script(&[0x42; 32]), 0));
        let mut slices = vec![slice_row(1, 100, 1, 1, &[(id, 1, 0)])];
        slices[0].work_unit_version = WorkUnitVersion::V2;
        slices[0].work_unit_fingerprint = Some([0x55; 32]);
        let hw = high_water(0, "1");
        let pending = BTreeMap::new();
        let delivered = BTreeSet::from([id]);
        let input = ReconstructInput {
            height: 101,
            high_water: &hw,
            entries: &entries,
            slices: &slices,
            work_unit_version: WorkUnitVersion::V2,
            work_unit_fingerprint: Some([0x55; 32]),
            window_rule: WindowRule::NetworkBlockWork,
            window_target: Some(1),
            window_truncated: false,
            pending: &pending,
            coinbase: &coinbase,
            economics: BlockEconomics {
                subsidy: 9_500_123,
                fees: 0,
            },
            delivered: &delivered,
            cfg: AccountingConfig {
                fee_bps: 0,
                ..AccountingConfig::default()
            },
            coinbase_threshold_sats: 330,
        };
        let result = reconstruct_settlement(&input).unwrap();
        assert_eq!(result.mode, TemplateMode::Pool);
        assert_eq!(result.ledger.len(), 1);
        assert_eq!(result.ledger[0].paid_sats, 9_500_000);
        assert_eq!(result.ledger[0].ark_due_sats, 123);
        assert_eq!(result.ledger[0].pending_delta, 0);
        let mut changed = entries.clone();
        changed[0].owed_sats += 1;
        assert!(reconstruct_settlement(&ReconstructInput {
            entries: &changed,
            ..input
        })
        .is_err());
        let mut changed = entries.clone();
        changed[0].p_x.swap(0, 1);
        assert!(reconstruct_settlement(&ReconstructInput {
            entries: &changed,
            ..input
        })
        .is_err());
        let mut changed = entries.clone();
        changed[0].p_x.pop();
        assert!(reconstruct_settlement(&ReconstructInput {
            entries: &changed,
            ..input
        })
        .is_err());
        let mut changed = entries.clone();
        changed.push(changed[0].clone());
        assert!(matches!(
            reconstruct_settlement(&ReconstructInput {
                entries: &changed,
                ..input
            }),
            Err(ReconstructError::DuplicateEntry(_))
        ));
        assert!(matches!(
            reconstruct_settlement(&ReconstructInput {
                window_target: None,
                ..input
            }),
            Err(ReconstructError::MissingFrozenV2Target)
        ));
    }

    #[test]
    fn empty_partial_and_full_settlement_repay_frozen_prior_with_signed_net_pool_value() {
        let who = id(7);
        let prior = BTreeMap::from([(who, 1_000_123)]);
        let delivered = BTreeSet::new();
        for work in [0, 1, 2] {
            let raw = if work == 0 { 0 } else { 4_950_000_000 };
            let owed = raw + 1_000_123;
            let paid = owed - owed % 500_000;
            let (entries, coinbase) = ladder_fixture(&[(who, owed)], 5_000_000_000 - paid);
            let mut slices = Vec::new();
            if work > 0 {
                let mut row = slice_row(1, 100, work as u128, work, &[(who, work as u128, 0)]);
                row.work_unit_version = WorkUnitVersion::V2;
                row.work_unit_fingerprint = Some([0x55; 32]);
                slices.push(row);
            }
            let hw = high_water(0, if work == 0 { "0" } else { "1" });
            let input = ReconstructInput {
                height: 101,
                high_water: &hw,
                entries: &entries,
                slices: &slices,
                work_unit_version: WorkUnitVersion::V2,
                work_unit_fingerprint: Some([0x55; 32]),
                window_rule: WindowRule::NetworkBlockWork,
                window_target: Some(2),
                window_truncated: false,
                pending: &prior,
                coinbase: &coinbase,
                economics: BlockEconomics {
                    subsidy: 5_000_000_000,
                    fees: 0,
                },
                delivered: &delivered,
                cfg: AccountingConfig {
                    fee_bps: 100,
                    max_payout_slots: 2048,
                    budget_wu: 1_000_000,
                    ..AccountingConfig::default()
                },
                coinbase_threshold_sats: 330,
            };
            let result = reconstruct_settlement(&input).unwrap();
            assert_eq!(result.raw.get(&who).copied().unwrap_or(0), raw);
            assert_eq!(result.ledger.len(), 1);
            let row = &result.ledger[0];
            assert_eq!(row.rail, LedgerRail::Coinbase);
            assert_eq!(row.paid_sats, paid);
            assert_eq!(row.ark_due_sats, 0);
            assert_eq!(row.forfeited_sats, 0);
            assert_eq!(row.pending_delta, -1_000_000);
            assert_eq!(
                i128::from(coinbase[0].value),
                i128::from(5_000_000_000 - raw) + i128::from(row.pending_delta)
            );
            let mut wrong_prior = prior.clone();
            *wrong_prior.get_mut(&who).unwrap() += 1;
            assert!(reconstruct_settlement(&ReconstructInput {
                pending: &wrong_prior,
                ..input
            })
            .is_err());
            assert!(reconstruct_settlement(&ReconstructInput {
                window_truncated: true,
                ..input
            })
            .is_err());
            if work == 0 {
                let missing = high_water(0, "1");
                assert!(matches!(
                    reconstruct_settlement(&ReconstructInput {
                        high_water: &missing,
                        ..input
                    }),
                    Err(ReconstructError::NoSlices)
                ));
                assert!(matches!(
                    reconstruct_settlement(&ReconstructInput {
                        window_target: None,
                        ..input
                    }),
                    Err(ReconstructError::MissingFrozenV2Target)
                ));
                assert!(matches!(
                    reconstruct_settlement(&ReconstructInput {
                        entries: &[],
                        ..input
                    }),
                    Err(ReconstructError::WindowMismatch)
                ));
            }
        }
    }

    #[test]
    fn empty_settlement_can_hold_total_owed_above_t_and_repay_a_known_identity() {
        let held = id(1);
        let paid_id = id(2);
        let prior = BTreeMap::from([(held, 2_000_000), (paid_id, 600_123)]);
        let (mut entries, coinbase) = ladder_fixture(&[(paid_id, 600_123)], 500_000);
        entries.push(entry(held, Rail::Pending, None, 2_000_000));
        let hw = high_water(0, "0");
        let delivered = BTreeSet::new();
        let input = ReconstructInput {
            height: 101,
            high_water: &hw,
            entries: &entries,
            slices: &[],
            work_unit_version: WorkUnitVersion::V2,
            work_unit_fingerprint: Some([0x55; 32]),
            window_rule: WindowRule::NetworkBlockWork,
            window_target: Some(2),
            window_truncated: false,
            pending: &prior,
            coinbase: &coinbase,
            economics: BlockEconomics {
                subsidy: 1_000_000,
                fees: 0,
            },
            delivered: &delivered,
            cfg: AccountingConfig {
                max_payout_slots: 2048,
                budget_wu: 1_000_000,
                ..AccountingConfig::default()
            },
            coinbase_threshold_sats: 330,
        };
        let result = reconstruct_settlement(&input).unwrap();
        let held_row = result.ledger.iter().find(|r| r.sp_id == held).unwrap();
        assert_eq!(held_row.rail, LedgerRail::Pending);
        assert_eq!(held_row.pending_delta, 0);
        let paid = result.ledger.iter().find(|r| r.sp_id == paid_id).unwrap();
        assert_eq!(paid.paid_sats, 500_000);
        assert_eq!(paid.pending_delta, -500_000);
        assert_eq!(paid.forfeited_sats, 0);
    }

    #[test]
    fn native_r4_failed_candidate_reconstructs_exact_net_pending_correction() {
        let priors = BTreeMap::from([(id(1), 386_759), (id(2), 566_201), (id(3), 547_037)]);
        let owed = [
            (id(1), 1_699_640_490),
            (id(2), 1_625_939_335),
            (id(3), 1_625_920_171),
        ];
        let (entries, coinbase) = ladder_fixture(&owed, 49_500_000);
        let mut row = slice_row(
            1,
            11,
            67,
            67,
            &[(id(1), 23, 0), (id(2), 22, 0), (id(3), 22, 0)],
        );
        row.work_unit_version = WorkUnitVersion::V2;
        row.work_unit_fingerprint = Some([0x55; 32]);
        let slices = [row];
        let hw = high_water(0, "1");
        let delivered = BTreeSet::new();
        let input = ReconstructInput {
            height: 12,
            high_water: &hw,
            entries: &entries,
            slices: &slices,
            work_unit_version: WorkUnitVersion::V2,
            work_unit_fingerprint: Some([0x55; 32]),
            window_rule: WindowRule::NetworkBlockWork,
            window_target: Some(65),
            window_truncated: false,
            pending: &priors,
            coinbase: &coinbase,
            economics: BlockEconomics {
                subsidy: 5_000_000_000,
                fees: 0,
            },
            delivered: &delivered,
            cfg: AccountingConfig {
                fee_bps: 100,
                max_payout_slots: 2048,
                budget_wu: 1_000_000,
                ..AccountingConfig::default()
            },
            coinbase_threshold_sats: 330,
        };
        let result = reconstruct_settlement(&input).unwrap();
        assert_eq!(result.raw.values().sum::<u64>(), 4_949_999_999);
        assert_eq!(
            result.ledger.iter().map(|r| r.paid_sats).sum::<u64>(),
            4_950_500_000
        );
        assert_eq!(
            result.pending_deltas,
            vec![(id(1), -246_269), (id(2), -126_866), (id(3), -126_866)]
        );
        assert_eq!(
            result.ledger.iter().map(|r| r.pending_delta).sum::<i64>(),
            -500_001
        );
        assert!(result
            .ledger
            .iter()
            .all(|r| r.ark_due_sats == 0 && r.forfeited_sats == 0));
    }

    /// A BIP 141 witness commitment output is an OP_RETURN output but not a
    /// JD carrier: pool-template blocks with witness data must settle in
    /// pool mode (decision 2026-11-08-pool-witness-commitment). The real JD
    /// carrier is `OP_RETURN OP_PUSHBYTES_35 "SPc" <ser_x(A_send)>`
    /// (`6a23535063…`, SPEC §2.4.2, `spc_crypto::carriers::emit_op_return`)
    /// and still selects JD mode - alone or alongside a commitment - and an
    /// unknown `OP_RETURN` output keeps selecting JD mode.
    #[test]
    fn a_witness_commitment_output_is_not_a_jd_carrier() {
        let commitment = output(crate::witness::commitment_script(&[0x42; 32]), 0);
        let pool = output(pool_script(), 100);
        assert_eq!(detect_mode(std::slice::from_ref(&pool)), TemplateMode::Pool);
        assert_eq!(
            detect_mode(&[pool, commitment.clone()]),
            TemplateMode::Pool,
            "a pool-template commitment output must not flip mode to Jd"
        );
        // The actual JD carrier (spc_crypto's own emitter), alone...
        let carrier = spc_crypto::carriers::emit_op_return(&[0x77; 32]);
        assert_eq!(
            carrier.as_bytes()[..3],
            [0x6a, 0x23, 0x53][..],
            "the real JD carrier is a 6a23 SPc push"
        );
        assert_eq!(
            detect_mode(&[output(carrier.as_bytes().to_vec(), 0)]),
            TemplateMode::Jd,
            "the actual JD carrier selects Jd mode"
        );
        // ...and alongside a witness commitment: the commitment does not
        // suppress JD detection.
        assert_eq!(
            detect_mode(&[output(carrier.as_bytes().to_vec(), 0), commitment]),
            TemplateMode::Jd,
            "a commitment output must not mask a JD carrier"
        );
        // An unknown OP_RETURN shape still selects JD mode.
        let mut unknown = vec![0x6a, 0x20];
        unknown.extend_from_slice(&[0u8; 32]);
        assert_eq!(
            detect_mode(&[output(unknown, 0)]),
            TemplateMode::Jd,
            "an unknown OP_RETURN still selects Jd mode"
        );
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

    /// R5 (Fable rework 2026-11-08): settlement's tier crosscheck mirrors the
    /// front's 224 WU witness overhead, detected from the mined coinbase. At
    /// the exact budget boundary (224 WU overhead + two 172 WU payout slots)
    /// the settlement routing keeps precisely the ids the coinbase pays;
    /// without the mirror the same budget would keep a third id the coinbase
    /// does not carry and the routing crosscheck would diverge. A legacy
    /// (witness-free) coinbase keeps the configured overhead unchanged.
    #[test]
    fn settlement_mirrors_the_witness_overhead_at_the_budget_boundary() {
        let alice = id(0xa1);
        let bob = id(0xb2);
        let carol = id(0xc3);
        let slices = vec![
            slice_row(3, 100, 100, 1, &[(alice, 60, 0), (carol, 30, 0)]),
            slice_row(2, 100, 100, 1, &[(bob, 40, 0)]),
        ];
        // The front's distribution: alice and bob paid by the coinbase,
        // carol trimmed at the weight boundary to pending.
        let (entries, mut coinbase) = ladder_fixture(
            &[(alice, 6_000_000), (bob, 4_000_000), (carol, 3_000_000)],
            7_000_000,
        );
        // The mined coinbase: pool output first, the two paid payouts, and
        // the BIP 141 commitment output last (zero value).
        let commitment = crate::witness::commitment_script(&[0x42; 32]);
        coinbase.push(output(commitment, 0));
        let pending = BTreeMap::new();
        let delivered = BTreeSet::new();
        let hw = high_water(0, "3");
        // 224 WU commitment overhead + exactly two 172 WU payout slots.
        let mut cfg = base_cfg(2);
        cfg.budget_wu = 224 + 5 * 172;
        let input = ReconstructInput {
            height: 100,
            high_water: &hw,
            entries: &entries,
            slices: &slices,
            // Pre-migration shape: the warned slice-estimate fallback, like
            // the neighbouring reconstruction tests.
            work_unit_version: WorkUnitVersion::V1,
            work_unit_fingerprint: None,
            window_rule: WindowRule::ShareWorkLegacy,
            window_target: None,
            window_truncated: false,
            pending: &pending,
            coinbase: &coinbase,
            economics: BlockEconomics {
                subsidy: 20_000_000,
                fees: 0,
            },
            delivered: &delivered,
            cfg,
            coinbase_threshold_sats: 30,
        };

        // The mirror: the commitment output in the mined coinbase costs the
        // settlement routing the same 224 WU the front budgeted.
        assert_eq!(tier_config(&input).overhead_wu, 224);

        let reconstructed = reconstruct_settlement(&input).unwrap();
        // Exactly the paid ids settle on the coinbase rail with their paid
        // values; carol stays beyond the boundary.
        for row in &reconstructed.ledger {
            if row.sp_id == alice {
                assert_eq!(row.rail, LedgerRail::Coinbase);
                assert_eq!(row.paid_sats, 6_000_000);
            } else if row.sp_id == bob {
                assert_eq!(row.rail, LedgerRail::Coinbase);
                assert_eq!(row.paid_sats, 4_000_000);
            } else if row.sp_id == carol {
                assert_eq!(row.paid_sats, 3_000_000, "no capacity deferral");
            } else {
                panic!("unexpected ledger row");
            }
        }

        // The counterfactual: the same routing with the overhead unmirrored
        // (what settlement charged before the rework) keeps carol too, at
        // the very same budget - the spurious divergence the mirror removes.
        let raw = spc_accounting::RawPayouts {
            raw: BTreeMap::from([(alice, 6_000_000), (bob, 4_000_000), (carol, 3_000_000)]),
            pool_sats: 0,
            r_net: 130,
            f_net: 0,
        };
        let caps = BTreeMap::from([
            (
                alice,
                spc_accounting::ArkCapability {
                    // AP-2 reconciliation: `ArkCapability` gained
                    // `registration_committed`; these T4-era counterfactual
                    // caps stay fully unregistered.
                    registered: false,
                    registration_committed: false,
                    dest_known: false,
                },
            ),
            (
                bob,
                spc_accounting::ArkCapability {
                    // AP-2 reconciliation: `ArkCapability` gained
                    // `registration_committed`; these T4-era counterfactual
                    // caps stay fully unregistered.
                    registered: false,
                    registration_committed: false,
                    dest_known: false,
                },
            ),
            (
                carol,
                spc_accounting::ArkCapability {
                    // AP-2 reconciliation: `ArkCapability` gained
                    // `registration_committed`; these T4-era counterfactual
                    // caps stay fully unregistered.
                    registered: false,
                    registration_committed: false,
                    dest_known: false,
                },
            ),
        ]);
        let mirrored = tier_config(&input);
        let routing =
            spc_accounting::route_tiers(&raw, &pending, &caps, &mirrored, 20_000_000).unwrap();
        let kept: BTreeSet<[u8; 32]> = routing.kept.iter().map(|(id, _)| *id).collect();
        assert_eq!(
            kept,
            BTreeSet::from([alice, bob, carol]),
            "the mirrored boundary keeps exactly what the coinbase pays"
        );
        let unmirrored = spc_accounting::TierConfig {
            overhead_wu: 0,
            budget_wu: 5 * 172,
            ..mirrored
        };
        let wrong =
            spc_accounting::route_tiers(&raw, &pending, &caps, &unmirrored, 20_000_000).unwrap();
        let too_small = spc_accounting::TierConfig {
            overhead_wu: 224,
            ..unmirrored
        };
        assert_eq!(
            spc_accounting::route_tiers(&raw, &pending, &caps, &too_small, 20_000_000),
            Err(spc_accounting::Error::PayoutWeight)
        );
        assert!(
            wrong.kept.iter().any(|(id, _)| *id == carol),
            "without the mirror the same budget keeps an unpaid id"
        );

        // Legacy control: a witness-free coinbase with the same payouts, at
        // its own boundary (two slots, no overhead), settles identically -
        // the mirror is commitment-driven, never a default.
        let legacy_coinbase = coinbase[..coinbase.len() - 1].to_vec();
        let mut legacy_input = input;
        legacy_input.coinbase = &legacy_coinbase;
        let mut legacy_cfg = cfg;
        legacy_cfg.budget_wu = 5 * 172;
        legacy_input.cfg = legacy_cfg;
        assert_eq!(tier_config(&legacy_input).overhead_wu, 0);
        let legacy = reconstruct_settlement(&legacy_input).unwrap();
        assert_eq!(
            legacy
                .ledger
                .iter()
                .filter(|r| r.rail == LedgerRail::Coinbase)
                .count(),
            3
        );
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
        let (entries, coinbase) =
            ladder_fixture(&[(alice, 6_000_000), (bob, 4_000_000)], 10_000_000);
        let pending = BTreeMap::new();
        let delivered = BTreeSet::new();
        let high_water = high_water(0, "4");
        let input = ReconstructInput {
            height: 100,
            high_water: &high_water,
            entries: &entries,
            slices: &slices,
            // Pre-migration shape: no persisted target, so these tests
            // exercise the warned slice-estimate fallback.
            work_unit_version: WorkUnitVersion::V1,
            work_unit_fingerprint: None,
            window_rule: WindowRule::ShareWorkLegacy,
            window_target: None,
            window_truncated: false,
            pending: &pending,
            coinbase: &coinbase,
            economics: BlockEconomics {
                subsidy: 20_000_000,
                fees: 0,
            },
            delivered: &delivered,
            cfg: base_cfg(2),
            coinbase_threshold_sats: 30,
        };

        let reconstructed = reconstruct_settlement(&input).unwrap();
        assert_eq!(reconstructed.mode, TemplateMode::Pool);
        assert_eq!(reconstructed.window, vec![3, 2]);
        assert_eq!(reconstructed.raw.get(&alice), Some(&6_000_000));
        assert_eq!(reconstructed.raw.get(&bob), Some(&4_000_000));
        assert_eq!(reconstructed.raw.get(&carol), None);
        assert_eq!(reconstructed.t_actual, 20_000_000);
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
            // Pre-migration shape: no persisted target, so these tests
            // exercise the warned slice-estimate fallback.
            work_unit_version: WorkUnitVersion::V1,
            work_unit_fingerprint: None,
            window_rule: WindowRule::ShareWorkLegacy,
            window_target: None,
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
            // Pre-migration shape: no persisted target, so these tests
            // exercise the warned slice-estimate fallback.
            work_unit_version: WorkUnitVersion::V1,
            work_unit_fingerprint: None,
            window_rule: WindowRule::ShareWorkLegacy,
            window_target: None,
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
        let (mut entries, coinbase) = ladder_fixture(&[(alice, 20_000_000)], 0);
        entries.push(carol_entry);
        let pending = BTreeMap::from([(carol, 100)]);
        let delivered = BTreeSet::from([carol]);
        let high_water = high_water(0, "3");
        let input = ReconstructInput {
            height: 100,
            high_water: &high_water,
            entries: &entries,
            slices: &slices,
            // Pre-migration shape: no persisted target, so these tests
            // exercise the warned slice-estimate fallback.
            work_unit_version: WorkUnitVersion::V1,
            work_unit_fingerprint: None,
            window_rule: WindowRule::ShareWorkLegacy,
            window_target: None,
            window_truncated: false,
            pending: &pending,
            coinbase: &coinbase,
            economics: BlockEconomics {
                subsidy: 20_000_000,
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
    fn high_water_naming_the_open_slice_is_rejected_without_alternative_window() {
        let alice = id(0xa1);
        let bob = id(0xb2);
        let slices = vec![
            // The open finding slice: credited here only if wrongly taken.
            slice_row(4, 100, 100, 1, &[(alice, 100, 0)]),
            slice_row(3, 100, 100, 1, &[(alice, 60, 0)]),
            slice_row(2, 100, 100, 1, &[(bob, 40, 0)]),
        ];
        let (entries, coinbase) =
            ladder_fixture(&[(alice, 6_000_000), (bob, 4_000_000)], 10_000_000);
        let pending = BTreeMap::new();
        let delivered = BTreeSet::new();
        let hw = high_water(0, "4");
        let input = ReconstructInput {
            height: 100,
            high_water: &hw,
            entries: &entries,
            slices: &slices,
            // Pre-migration shape: no persisted target, so these tests
            // exercise the warned slice-estimate fallback.
            work_unit_version: WorkUnitVersion::V1,
            work_unit_fingerprint: None,
            window_rule: WindowRule::ShareWorkLegacy,
            window_target: None,
            window_truncated: false,
            pending: &pending,
            coinbase: &coinbase,
            economics: BlockEconomics {
                subsidy: 20_000_000,
                fees: 0,
            },
            delivered: &delivered,
            cfg: base_cfg(2),
            coinbase_threshold_sats: 30,
        };

        assert!(matches!(
            reconstruct_settlement(&input),
            Err(ReconstructError::CoinbaseTierMismatch)
        ));
    }

    #[test]
    fn multi_front_high_water_fails_closed() {
        let hw = BTreeMap::from([(0u8, "1".to_string()), (1, "2".to_string())]);
        let input = ReconstructInput {
            height: 1,
            high_water: &hw,
            entries: &[],
            slices: &[],
            work_unit_version: WorkUnitVersion::V1,
            work_unit_fingerprint: None,
            window_rule: WindowRule::ShareWorkLegacy,
            window_target: None,
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
            // Pre-migration shape: no persisted target, so these tests
            // exercise the warned slice-estimate fallback.
            work_unit_version: WorkUnitVersion::V1,
            work_unit_fingerprint: None,
            window_rule: WindowRule::ShareWorkLegacy,
            window_target: None,
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
            // Pre-migration shape: no persisted target, so these tests
            // exercise the warned slice-estimate fallback.
            work_unit_version: WorkUnitVersion::V1,
            work_unit_fingerprint: None,
            window_rule: WindowRule::ShareWorkLegacy,
            window_target: None,
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
    fn v2_slice_restoration_rejects_stored_sum_mismatch_and_saturating_overflow() {
        let id = id(0xa1);
        let mut mismatch = slice_row(3, 100, 100, 1, &[(id, 99, 2)]);
        mismatch.work_unit_version = WorkUnitVersion::V2;
        mismatch.work_unit_fingerprint = Some([7; 32]);
        let err = slice_agg(&mismatch).unwrap_err();
        assert!(format!("{err}").contains("stored difficulty"));

        let mut overflow = slice_row(4, 100, u128::MAX, 1, &[(id, u128::MAX, 1)]);
        overflow.credits.push(SliceCreditRow {
            sp_id: id,
            diff_sum: "1".into(),
            diff_fee_sum: "1".into(),
        });
        overflow.work_unit_version = WorkUnitVersion::V2;
        overflow.work_unit_fingerprint = Some([7; 32]);
        assert!(matches!(
            slice_agg(&overflow),
            Err(ReconstructError::WorkUnit(
                spc_accounting::WorkUnitError::WindowWorkOverflow
            ))
        ));
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

    /// Decision 2026-11-03-settlement-window-target (test (a)): the window
    /// was selected with the finding template's `d`, a retarget has since
    /// changed what the stored slices imply, and settlement must use the
    /// persisted target, not the slice estimate.
    ///
    /// `window_n = 1`; the finding template credited `d = 600` (persisted
    /// target 600) while every stored slice was sealed under the pre-retarget
    /// `d = 300`, so the slice estimate would be 300.
    #[test]
    fn settlement_uses_the_persisted_window_target_across_a_retarget() {
        let alice = id(0xa1);
        let bob = id(0xb2);
        // No slice at the finding height 200: the newest slice is from the
        // pre-retarget epoch, so the estimate path would pick 300.
        let slices = vec![
            slice_row(3, 199, 300, 1, &[(alice, 300, 0)]),
            slice_row(2, 198, 300, 1, &[(bob, 300, 0)]),
        ];
        let (entries, coinbase) = ladder_fixture(&[(alice, 3_000_000), (bob, 3_000_000)], 0);
        let hw = high_water(0, "3");
        let input = ReconstructInput {
            height: 200,
            high_water: &hw,
            entries: &entries,
            slices: &slices,
            // The exact target the front froze for this distribution.
            work_unit_version: WorkUnitVersion::V1,
            work_unit_fingerprint: None,
            window_rule: WindowRule::ShareWorkLegacy,
            window_target: Some(600),
            window_truncated: false,
            pending: &BTreeMap::new(),
            coinbase: &coinbase,
            economics: BlockEconomics {
                subsidy: 6_000_000,
                fees: 0,
            },
            delivered: &BTreeSet::new(),
            cfg: base_cfg(1),
            coinbase_threshold_sats: 30,
        };

        let reconstructed = reconstruct_settlement(&input).unwrap();
        // The persisted 600 target takes both slices (Σ 600); the 300
        // estimate would have taken only slice 3 and paid Bob nothing.
        assert_eq!(reconstructed.window, vec![3, 2]);
        assert_eq!(reconstructed.raw.get(&alice), Some(&3_000_000));
        assert_eq!(reconstructed.raw.get(&bob), Some(&3_000_000));
        // Replay determinism: the same frozen inputs book the same ledger.
        assert_eq!(reconstruct_settlement(&input).unwrap(), reconstructed);
    }

    #[test]
    fn v2_settlement_uses_frozen_reference_target_not_current_generation() {
        let alice = id(0xa1);
        let fp = [0x55; 32];
        let mut slices = vec![
            slice_row(3, 199, 300, 1, &[(alice, 300, 0)]),
            slice_row(2, 198, 300, 1, &[(id(0xb2), 300, 0)]),
        ];
        for row in &mut slices {
            row.work_unit_version = WorkUnitVersion::V2;
            row.work_unit_fingerprint = Some(fp);
        }
        // Only Alice's output belongs to this Q=65 distribution. The other
        // slice exists in history but is not part of the selected window.
        let (entries, coinbase) = ladder_fixture(&[(alice, 3_000_000)], 0);
        let hw = high_water(0, "3");
        let input = ReconstructInput {
            height: 200,
            high_water: &hw,
            entries: &entries,
            slices: &slices,
            work_unit_version: WorkUnitVersion::V2,
            work_unit_fingerprint: Some(fp),
            window_rule: WindowRule::NetworkBlockWork,
            // Frozen Q at build time, not a later target or network difficulty.
            window_target: Some(65),
            window_truncated: false,
            pending: &BTreeMap::new(),
            coinbase: &coinbase,
            economics: BlockEconomics {
                subsidy: 3_000_000,
                fees: 0,
            },
            delivered: &BTreeSet::new(),
            cfg: base_cfg(1),
            coinbase_threshold_sats: 30,
        };
        let settled = reconstruct_settlement(&input).unwrap();
        assert_eq!(settled.window, vec![3]);
        assert_eq!(settled.raw.get(&alice), Some(&3_000_000));
        assert!(!input.work_unit_version.eq(&WorkUnitVersion::V1));
        assert!(settled.raw.len() == 1);
    }

    /// Test (b): a pre-migration snapshot row carries no target, so
    /// settlement falls back to the (warned, at the service layer) slice
    /// estimate. The same slices as the retarget test estimate 300 from the
    /// newest slice and select only slice 3.
    #[test]
    fn pre_migration_snapshot_falls_back_to_the_slice_estimate() {
        let alice = id(0xa1);
        let slices = vec![
            slice_row(3, 199, 300, 1, &[(alice, 300, 0)]),
            slice_row(2, 198, 300, 1, &[(id(0xb2), 300, 0)]),
        ];
        let (entries, coinbase) = ladder_fixture(&[(alice, 3_000_000)], 0);
        let hw = high_water(0, "3");
        let input = ReconstructInput {
            height: 200,
            high_water: &hw,
            entries: &entries,
            slices: &slices,
            work_unit_version: WorkUnitVersion::V1,
            work_unit_fingerprint: None,
            window_rule: WindowRule::ShareWorkLegacy,
            window_target: None,
            window_truncated: false,
            pending: &BTreeMap::new(),
            coinbase: &coinbase,
            economics: BlockEconomics {
                subsidy: 3_000_000,
                fees: 0,
            },
            delivered: &BTreeSet::new(),
            cfg: base_cfg(1),
            coinbase_threshold_sats: 30,
        };

        let reconstructed = reconstruct_settlement(&input).unwrap();
        // The estimate (300 from the newest slice) closes on slice 3 alone -
        // a smaller window than the persisted 600 target would select.
        assert_eq!(reconstructed.window, vec![3]);
        assert_eq!(reconstructed.raw.get(&alice), Some(&3_000_000));
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
            owed_sats: sats,
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
                work_unit_version: WorkUnitVersion::V1,
                work_unit_fingerprint: None,
                window_rule: WindowRule::ShareWorkLegacy,
                window_target: Some("8".into()),
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
                work_unit_version: WorkUnitVersion::V1,
                work_unit_fingerprint: None,
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

    /// Authoritative view with a fixed coinbase and economics: lets the
    /// env-gated tests run the exact `recompute` path against a real store.
    struct ReconstructChain {
        coinbase: Vec<CoinbaseOutput>,
        economics: BlockEconomics,
    }

    #[async_trait::async_trait]
    impl ChainView for ReconstructChain {
        async fn tip_height(&self) -> Result<u32, ChainError> {
            Ok(900_000)
        }

        async fn block_hash_at(&self, _height: u32) -> Result<Option<[u8; 32]>, ChainError> {
            Ok(None)
        }

        async fn is_active(&self, _block: &[u8; 32], _height: u32) -> Result<bool, ChainError> {
            Ok(true)
        }

        async fn coinbase_outputs(
            &self,
            _block_hash: &[u8; 32],
        ) -> Result<Option<Vec<CoinbaseOutput>>, ChainError> {
            Ok(Some(self.coinbase.clone()))
        }

        async fn block_economics(
            &self,
            _block_hash: &[u8; 32],
        ) -> Result<Option<BlockEconomics>, ChainError> {
            Ok(Some(self.economics))
        }

        fn authoritative(&self) -> bool {
            true
        }
    }

    /// Store fixture for the reconstruction tests: two miners, two sealed
    /// pre-retarget slices below the finding height, and a snapshot whose
    /// `window_target` the caller chooses (None = pre-migration row).
    /// Env-gated settlement fixture: two identities, two sealed slices, and
    /// a distribution snapshot at a finding height.
    ///
    /// The high-water walk (`window_slices_to`) returns EVERY stored slice
    /// at or below the mark, so this fixture must be deterministic against
    /// a shared test database that other fixtures - and earlier runs of
    /// these very tests - have filled:
    ///
    /// - The slice ids sit just below `i64::MAX` and the heights at
    ///   `i32::MAX`, so no foreign row can sort between (or before) the
    ///   fixture's two slices in the walk's `height DESC, slice_id DESC`
    ///   order; the window closes on exactly these two rows.
    /// - The ids, heights, difficulties, and identity keys are FIXED, and
    ///   `insert_slice` is `ON CONFLICT DO NOTHING`: a re-run writes
    ///   byte-identical rows, so the fixture is idempotent and never
    ///   accumulates stale credits the window would reject.
    ///
    /// The finding height equals the newest slice's height, so the
    /// pre-migration estimate's reference slice is the fixture's own, not
    /// whatever foreign slice happens to be stored.
    async fn reconstruction_env(
        window_target: Option<&str>,
    ) -> Option<(Store, Streams, [u8; 32], [u8; 32], u64, u32)> {
        let (store, streams) = ark_test_env().await?;
        let alice = [
            0x53, 0x50, 0x63, 0x74, 0x65, 0x73, 0x74, 0x61, 0x6c, 0x69, 0x63, 0x65, 0x31, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x01,
        ];
        let bob = [
            0x53, 0x50, 0x63, 0x74, 0x65, 0x73, 0x74, 0x62, 0x6f, 0x62, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x02,
        ];
        for sp_id in [alice, bob] {
            store
                .upsert_identity(&SpIdentityRow {
                    sp_id,
                    b_scan: vec![2; 33],
                    b_spend: vec![3; 33],
                    network: 3,
                })
                .await
                .unwrap();
        }
        let older = 0x7fff_ffff_ffff_f000u64;
        let newer = 0x7fff_ffff_ffff_f001u64;
        let finding_height = i32::MAX as u32;
        for (slice_id, height, sp_id) in [
            (newer, finding_height, alice),
            (older, finding_height - 1, bob),
        ] {
            store
                .insert_slice(&SliceRow {
                    slice_id,
                    height,
                    prev_hash: [1; 32],
                    ref_job_id: 1,
                    ref_fees: 0,
                    n_shares: 1,
                    difficulty: "100".into(),
                    root: [2; 32],
                    shares: vec![3],
                    work_unit_version: WorkUnitVersion::V1,
                    work_unit_fingerprint: None,
                    credits: vec![SliceCreditRow {
                        sp_id,
                        diff_sum: "100".into(),
                        diff_fee_sum: "0".into(),
                    }],
                })
                .await
                .unwrap();
        }
        let fingerprint: [u8; 32] = rand::random();
        let entries = vec![
            entry(alice, Rail::Coinbase, Some(id(0x01)), 100),
            entry(bob, Rail::Coinbase, Some(id(0x02)), 100),
        ];
        store
            .insert_snapshot(&SnapshotRow {
                fingerprint,
                height: finding_height,
                a_send_x: [4; 32],
                distribution_id: None,
                entries: serde_json::to_value(entries).unwrap(),
                high_water: serde_json::json!({ "0": newer.to_string() }),
                work_unit_version: WorkUnitVersion::V1,
                work_unit_fingerprint: None,
                window_rule: WindowRule::ShareWorkLegacy,
                window_target: window_target.map(str::to_string),
            })
            .await
            .unwrap();
        Some((store, streams, fingerprint, alice, newer, finding_height))
    }

    /// Test (b), service level: a pre-migration snapshot row (no stored
    /// window target) settles through the warned slice estimate - the
    /// reconstruction is not held, and the estimate path (which logs the
    /// warning naming the block) runs.
    #[tokio::test]
    async fn pre_migration_row_settles_through_the_warned_estimate() {
        let Some((store, streams, fingerprint, alice, newer, finding_height)) =
            reconstruction_env(None).await
        else {
            return;
        };
        let bob_entry_p_x = id(0x02);
        let svc = PayoutService::new(
            store.clone(),
            streams,
            RETRY_DEPTH,
            30,
            Arc::new(ReconstructChain {
                coinbase: vec![
                    output(pool_script(), 0),
                    output(spc_accounting::p2tr_script(&id(0x01)), 100),
                    output(spc_accounting::p2tr_script(&bob_entry_p_x), 100),
                ],
                economics: BlockEconomics {
                    subsidy: 200,
                    fees: 0,
                },
            }),
            None,
            0,
        )
        .with_accounting(base_cfg(2));
        let event = BlockEvent {
            economic: None,
            block_hash: rand::random(),
            height: finding_height,
            front_id: 0,
            fingerprint,
            a_send_x: [4; 32],
            high_water: BTreeMap::from([(0u8, newer.to_string())]),
            work_unit_version: WorkUnitVersion::V1,
            work_unit_fingerprint: None,
            window_rule: WindowRule::ShareWorkLegacy,
            window_target: 0,
            entries: Vec::new(),
        };
        let Some((ledger, deltas, _)) = svc.recompute(&event, fingerprint).await.unwrap() else {
            panic!("a pre-migration row must settle, not hold");
        };
        // The estimate (d = 100 from the newest slice, window_n = 2 -> 200)
        // takes both slices: both miners are paid their raw share.
        assert_eq!(ledger.len(), 2);
        assert!(ledger.iter().all(|row| row.rail == LedgerRail::Coinbase));
        let alice_row = ledger.iter().find(|row| row.sp_id == alice).unwrap();
        assert_eq!(alice_row.paid_sats, 100);
        assert!(deltas.is_empty());
    }

    /// A persisted window target that is not a u128 is a corrupt row:
    /// settlement fails closed (an error the consumer group retries and
    /// alarms on), never a silent fall-back to the estimate.
    #[tokio::test]
    async fn corrupt_persisted_window_target_fails_closed() {
        // 2^128: a valid NUMERIC(40,0) but beyond u128.
        let Some((store, streams, fingerprint, _alice, newer, finding_height)) =
            reconstruction_env(Some("340282366920938463463374607431768211456")).await
        else {
            return;
        };
        let svc = PayoutService::new(
            store,
            streams,
            RETRY_DEPTH,
            30,
            Arc::new(ReconstructChain {
                coinbase: vec![output(pool_script(), 200)],
                economics: BlockEconomics {
                    subsidy: 200,
                    fees: 0,
                },
            }),
            None,
            0,
        )
        .with_accounting(base_cfg(2));
        let event = BlockEvent {
            economic: None,
            block_hash: rand::random(),
            height: finding_height,
            front_id: 0,
            fingerprint,
            a_send_x: [4; 32],
            high_water: BTreeMap::from([(0u8, newer.to_string())]),
            work_unit_version: WorkUnitVersion::V1,
            work_unit_fingerprint: None,
            window_rule: WindowRule::ShareWorkLegacy,
            window_target: 0,
            entries: Vec::new(),
        };
        assert!(matches!(
            svc.recompute(&event, fingerprint).await,
            Err(PayoutError::Snapshot(_))
        ));
    }

    /// Regression for the accounting fix: an unregistered id below `C`
    /// forfeits only this block's raw earnings. A prior pending balance is
    /// retained, so the ledger delta is zero and conservation holds.
    #[test]
    fn forfeit_confiscates_raw_and_keeps_prior_pending() {
        let alice = id(0xa1);
        // raw = 40 (the window's only credit), pending = 100, owed = 140.
        let slices = vec![slice_row(2, 400, 100, 1, &[(alice, 100, 0)])];
        let mut owned = entry(alice, Rail::Forfeit, None, 40);
        owned.owed_sats = 140;
        let entries = vec![owned];
        // T = 40: nothing is kept, so the whole coinbase is the pool output.
        let coinbase = vec![output(pool_script(), 40)];
        let pending = BTreeMap::from([(alice, 100u64)]);
        let hw = high_water(0, "3");
        let input = ReconstructInput {
            height: 400,
            high_water: &hw,
            entries: &entries,
            slices: &slices,
            // Pre-migration shape: no persisted target, so these tests
            // exercise the warned slice-estimate fallback.
            work_unit_version: WorkUnitVersion::V1,
            work_unit_fingerprint: None,
            window_rule: WindowRule::ShareWorkLegacy,
            window_target: None,
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
    fn raw_at_base_rung_pays_full_canonical_amount_and_retains_only_remainder() {
        let alice = id(0xa1);
        // raw = 330 (>= C), pending = 100: owed = 430, T = 330.
        let slices = vec![slice_row(2, 500, 100, 1, &[(alice, 100, 0)])];
        let (entries, coinbase) = ladder_fixture(&[(alice, 500_100)], 0);
        let pending = BTreeMap::from([(alice, 100u64)]);
        let hw = high_water(0, "3");
        let input = ReconstructInput {
            height: 500,
            high_water: &hw,
            entries: &entries,
            slices: &slices,
            // Pre-migration shape: no persisted target, so these tests
            // exercise the warned slice-estimate fallback.
            work_unit_version: WorkUnitVersion::V1,
            work_unit_fingerprint: None,
            window_rule: WindowRule::ShareWorkLegacy,
            window_target: None,
            window_truncated: false,
            pending: &pending,
            coinbase: &coinbase,
            economics: BlockEconomics {
                subsidy: 500_000,
                fees: 0,
            },
            delivered: &BTreeSet::new(),
            cfg: base_cfg(2),
            coinbase_threshold_sats: 330,
        };

        let reconstructed = reconstruct_settlement(&input).unwrap();
        assert_eq!(reconstructed.raw.get(&alice), Some(&500_000));
        let row = reconstructed
            .ledger
            .iter()
            .find(|row| row.sp_id == alice)
            .expect("coinbase row");
        assert_eq!(row.rail, LedgerRail::Coinbase);
        assert_eq!(row.paid_sats, 500_000);
        assert_eq!(row.pending_delta, 0, "the carried 100 is retained");
        assert!(reconstructed
            .pending_deltas
            .iter()
            .all(|(id, _)| *id != alice));
        // Conservation: paid + retained pending == owed.
        let after = 100i64 + row.pending_delta;
        assert_eq!(i64::try_from(row.paid_sats).unwrap() + after, 500_100);
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
        front
            .apply_bootstrap(&BootstrapSnapshot {
                economic: None,
                rev: 42,
                applied_blocks: Vec::new(),
                pending_balance: Vec::new(),
                ark_delivery: Vec::new(),
                ark_identities: vec![],
                ark_epoch_mailboxes: vec![],
                window_tail: Vec::new(),
                type32_blobs: Vec::new(),
                slices_cursor: None,
                front_events_cursor: None,
            })
            .unwrap();

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
            economic: None,
            block_hash: block,
            height: 800_000,
            front_id: 0,
            fingerprint: rand::random(),
            a_send_x: [4; 32],
            high_water: BTreeMap::from([(0u8, "1".to_string())]),
            work_unit_version: WorkUnitVersion::V1,
            work_unit_fingerprint: None,
            window_rule: WindowRule::ShareWorkLegacy,
            window_target: 8,
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
        front
            .apply_bootstrap(&BootstrapSnapshot {
                economic: None,
                rev: 7,
                applied_blocks: vec![block],
                pending_balance: vec![(sp_id, 40)],
                ark_delivery: Vec::new(),
                ark_identities: vec![],
                ark_epoch_mailboxes: vec![],
                window_tail: Vec::new(),
                type32_blobs: Vec::new(),
                slices_cursor: None,
                front_events_cursor: None,
            })
            .unwrap();

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
                work_unit_version: WorkUnitVersion::V1,
                work_unit_fingerprint: None,
                window_rule: WindowRule::ShareWorkLegacy,
                window_target: Some("8".into()),
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
        svc.republish_unpublished_ledger_events().await.unwrap();

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
            economic: None,
            block_hash: block,
            height: 800_000,
            front_id: 0,
            fingerprint: rand::random(),
            a_send_x: [4; 32],
            high_water: BTreeMap::from([(0u8, "1".to_string())]),
            work_unit_version: WorkUnitVersion::V1,
            work_unit_fingerprint: None,
            window_rule: WindowRule::ShareWorkLegacy,
            window_target: 8,
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

    /// The republish set is bounded to blocks whose XADD has not succeeded:
    /// once a block is marked published, a stalled front cursor cannot make
    /// the timer append it again.
    #[tokio::test]
    async fn republish_is_bounded_to_unpublished_blocks() {
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
                height: 800_001,
                a_send_x: [4; 32],
                distribution_id: None,
                entries: serde_json::json!([]),
                high_water: serde_json::json!({"0": "1"}),
                work_unit_version: WorkUnitVersion::V1,
                work_unit_fingerprint: None,
                window_rule: WindowRule::ShareWorkLegacy,
                window_target: Some("8".into()),
            })
            .await
            .unwrap();
        store
            .settle(
                &Settlement {
                    block_hash: block,
                    height: 800_001,
                    fingerprint: Some(fingerprint),
                    status: BlockStatus::Confirmed,
                    ledger: vec![LedgerRow {
                        sp_id,
                        rail: LedgerRail::Pending,
                        paid_sats: 0,
                        ark_due_sats: 0,
                        forfeited_sats: 0,
                        pending_delta: 11,
                    }],
                },
                RETRY_DEPTH,
            )
            .await
            .unwrap();

        assert_eq!(
            store.unpublished_ledger_events(1_000).await.unwrap().len(),
            1
        );
        store.mark_ledger_published(&block).await.unwrap();
        assert!(store
            .unpublished_ledger_events(1_000)
            .await
            .unwrap()
            .is_empty());

        let events = unique_stream("front:events");
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
        svc.republish_unpublished_ledger_events().await.unwrap();
        assert_eq!(streams.len(&events).await.unwrap(), 0);
    }
}
