//! Pure SLICE/PPLNS-JD accounting (SPEC §4, §11.1).
//!
//! No I/O. Every function is deterministic; floors are applied per term as the
//! spec requires. Proptests pin conservation and the pool-template collapse.

#![deny(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use spc_crypto::{spc_output_pubkey, tier_tweaks, SendKey};
use thiserror::Error;

pub type SpId = [u8; 32];

#[derive(Debug, Error, PartialEq, Eq)]
pub enum Error {
    #[error("window is empty")]
    EmptyWindow,
    #[error("fee_bps exceeds 10000")]
    FeeBps,
    #[error("coinbase threshold is below dust")]
    Threshold,
    #[error("arithmetic overflow")]
    Overflow,
    #[error("derivation failed for {0:?}")]
    Derivation(SpId),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareRec {
    pub sp_id: SpId,
    pub d: u64,
    pub job_fees: u64,
    pub job_id: u64,
    pub nonce: u32,
    pub ntime: u32,
    pub version: u32,
    pub extranonce: Vec<u8>,
    pub share_index: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slice {
    pub slice_id: u64,
    pub height: u32,
    pub prev_hash: [u8; 32],
    pub front_id: u8,
    pub ref_job_id: u64,
    pub ref_fees: u64,
    pub shares: Vec<ShareRec>,
    pub difficulty: u128,
    pub root: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Seal {
    Keep,
    Seal(SealReason),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SealReason {
    NewPrevHash,
    FeeBand,
    MaxShares,
}

impl Slice {
    pub fn new(
        slice_id: u64,
        height: u32,
        prev_hash: [u8; 32],
        front_id: u8,
        ref_job_id: u64,
        ref_fees: u64,
    ) -> Self {
        Self {
            slice_id,
            height,
            prev_hash,
            front_id,
            ref_job_id,
            ref_fees,
            shares: Vec::new(),
            difficulty: 0,
            root: [0u8; 32],
        }
    }

    /// SPEC §4.2 sealing rule.
    pub fn seal_if_needed(&self, incoming: &ShareRec, delta: u64, max_shares: usize) -> Seal {
        // delta = 0 is the pool-template case: all shares carry the same job
        // fees, so the fee band is disabled rather than sealing every share.
        if delta > 0 && incoming.job_fees >= self.ref_fees.saturating_add(delta) {
            return Seal::Seal(SealReason::FeeBand);
        }
        if self.shares.len() >= max_shares {
            return Seal::Seal(SealReason::MaxShares);
        }
        Seal::Keep
    }

    pub fn push(&mut self, share: ShareRec) {
        self.difficulty = self.difficulty.saturating_add(share.d as u128);
        self.shares.push(share);
    }

    /// Compute `root` from the identity-free leaves.
    pub fn seal(&mut self) {
        let leaves: Vec<[u8; 32]> = self.shares.iter().map(share_leaf).collect();
        self.root = merkle_root(&leaves);
    }

    pub fn agg(&self) -> SliceAgg {
        let mut per_id = BTreeMap::new();
        let mut diff_fee_sum = 0u128;
        for s in &self.shares {
            let d = s.d as u128;
            let df = d.saturating_mul(s.job_fees as u128);
            let e = per_id.entry(s.sp_id).or_insert((0u128, 0u128));
            e.0 = e.0.saturating_add(d);
            e.1 = e.1.saturating_add(df);
            diff_fee_sum = diff_fee_sum.saturating_add(df);
        }
        SliceAgg {
            slice_id: self.slice_id,
            height: self.height,
            prev_hash: self.prev_hash,
            front_id: self.front_id,
            difficulty: self.difficulty,
            diff_fee_sum,
            per_id,
            root: self.root,
        }
    }
}

/// Identity-free leaf (SPEC §4.2).
pub fn share_leaf(s: &ShareRec) -> [u8; 32] {
    use bitcoin::hashes::{sha256d, Hash, HashEngine};
    let mut eng = sha256d::Hash::engine();
    eng.input(&s.nonce.to_le_bytes());
    eng.input(&s.ntime.to_le_bytes());
    eng.input(&s.version.to_le_bytes());
    eng.input(&s.extranonce);
    eng.input(&s.job_id.to_le_bytes());
    eng.input(&s.share_index.to_le_bytes());
    sha256d::Hash::from_engine(eng).to_byte_array()
}

pub fn merkle_root(leaves: &[[u8; 32]]) -> [u8; 32] {
    if leaves.is_empty() {
        return [0u8; 32];
    }
    let mut level = leaves.to_vec();
    while level.len() > 1 {
        if !level.len().is_multiple_of(2) {
            if let Some(last) = level.last().copied() {
                level.push(last);
            }
        }
        level = level
            .chunks_exact(2)
            .map(|p| {
                let mut buf = [0u8; 64];
                buf[..32].copy_from_slice(&p[0]);
                buf[32..].copy_from_slice(&p[1]);
                use bitcoin::hashes::{sha256d, Hash};
                sha256d::Hash::hash(&buf).to_byte_array()
            })
            .collect();
    }
    level[0]
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SliceAgg {
    pub slice_id: u64,
    pub height: u32,
    pub prev_hash: [u8; 32],
    pub front_id: u8,
    pub difficulty: u128,
    pub diff_fee_sum: u128,
    pub per_id: BTreeMap<SpId, (u128, u128)>,
    pub root: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Window {
    pub slices: Vec<SliceAgg>,
    pub t: usize,
    pub d_win: u128,
    pub high_water: BTreeMap<u8, String>,
}

/// SPEC §4.3. `slices_desc` is newest first; the finding slice is skipped.
pub fn select_window(
    slices_desc: &[SliceAgg],
    finding_slice_id: u64,
    target: u128,
    high_water: BTreeMap<u8, String>,
) -> Window {
    let mut slices = Vec::new();
    let mut d_win = 0u128;
    for s in slices_desc {
        if s.slice_id == finding_slice_id {
            continue;
        }
        slices.push(s.clone());
        d_win = d_win.saturating_add(s.difficulty);
        if d_win >= target {
            break;
        }
    }
    let t = slices.len();
    Window {
        slices,
        t,
        d_win,
        high_water,
    }
}

/// Raw payouts before pending and tier routing (SPEC §4.4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawPayouts {
    pub raw: BTreeMap<SpId, u64>,
    pub pool_sats: u64,
    pub r_net: u64,
    pub f_net: u64,
}

/// Raw payouts before pending and tier routing (SPEC §4.4).
///
/// Floors follow the spec exactly:
///
/// ```text
/// raw(id) = floor(r_net * Σ_{s in id} d_s / D_win)                     // once per identity
///         + Σ_S floor((F_net / t) * Σ_{s in id, s in S} d_s*f_s / DF_S) // once per slice
/// ```
///
/// The subsidy floor is taken **once per identity over the whole window**; a
/// per-slice floor would lose the accumulated remainder (SPEC §4.4). The fee
/// term keeps its per-slice floor.
pub fn raw_payouts(
    window: &Window,
    subsidy: u64,
    fees: u64,
    fee_bps: u16,
) -> Result<RawPayouts, Error> {
    if window.d_win == 0 || window.slices.is_empty() {
        return Err(Error::EmptyWindow);
    }
    if fee_bps > 10_000 {
        return Err(Error::FeeBps);
    }
    let fee_bps = fee_bps as u128;
    let r_net =
        (subsidy as u128).saturating_sub((subsidy as u128).saturating_mul(fee_bps) / 10_000);
    let f_net = (fees as u128).saturating_sub((fees as u128).saturating_mul(fee_bps) / 10_000);

    // `t` is the number of whole slices in the window; the slice vector is
    // authoritative so a stale `t` field cannot skew the fee split.
    let t = window.slices.len() as u128;

    let mut per_id_diff: BTreeMap<SpId, u128> = BTreeMap::new();
    let mut per_id_fee: BTreeMap<SpId, u128> = BTreeMap::new();
    for s in &window.slices {
        for (id, (d, df)) in &s.per_id {
            let e = per_id_diff.entry(*id).or_insert(0);
            *e = e.saturating_add(*d);

            // fee_term(S, id) = floor((F_net / t) * df / DF_S), one floor per
            // (identity, slice) term.
            let fee_term = if s.diff_fee_sum == 0 {
                0
            } else {
                f_net.saturating_mul(*df) / t.saturating_mul(s.diff_fee_sum)
            };
            let e = per_id_fee.entry(*id).or_insert(0);
            *e = e.saturating_add(fee_term);
        }
    }

    let mut raw: BTreeMap<SpId, u64> = BTreeMap::new();
    for (id, d_window) in per_id_diff {
        let subsidy_term = r_net.saturating_mul(d_window) / window.d_win;
        let fee_term = per_id_fee.get(&id).copied().unwrap_or(0);
        let v = subsidy_term.saturating_add(fee_term);
        raw.insert(id, u64::try_from(v).unwrap_or(u64::MAX));
    }

    let total = raw
        .values()
        .fold(0u128, |acc, v| acc.saturating_add(*v as u128));
    let reward = (subsidy as u128).saturating_add(fees as u128);
    let pool_sats = reward.saturating_sub(total);
    Ok(RawPayouts {
        raw,
        pool_sats: u64::try_from(pool_sats).unwrap_or(u64::MAX),
        r_net: u64::try_from(r_net).unwrap_or(u64::MAX),
        f_net: u64::try_from(f_net).unwrap_or(u64::MAX),
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TierConfig {
    pub fee_bps: u16,
    pub dust_limit_sats: u64,
    pub coinbase_threshold_sats: u64,
    pub max_payout_slots: usize,
    pub budget_wu: u32,
    /// Fixed coinbase overhead in weight units; the P2TR outputs are 172 each.
    pub overhead_wu: u32,
}

impl TierConfig {
    pub fn validate(&self) -> Result<(), Error> {
        if self.coinbase_threshold_sats < self.dust_limit_sats {
            return Err(Error::Threshold);
        }
        if self.fee_bps > 10_000 {
            return Err(Error::FeeBps);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArkCapability {
    pub registered: bool,
    /// The destination `U_i` plus delivery is available to this front.
    pub dest_known: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Routing {
    /// Full `owed(id) = raw(id) + prior_pending(id)` for every routed id,
    /// **before** the pool-template coinbase cap. JD weights are built from
    /// this map (SPEC §4.7 `weight_id = owed(id)`), so a balance that the
    /// coinbase can only partly pay still carries its full weight and O11
    /// scales it at settlement.
    pub owed: BTreeMap<SpId, u64>,
    /// Coinbase outputs: `(sp_id, sats actually paid)`. Sum is at most
    /// `T − pool_sats`, so `pay_p` always covers the pool fee.
    pub kept: Vec<(SpId, u64)>,
    /// Balances that wait in full for a later coinbase (trimmed, over-cap,
    /// pending-only, or registered without a propagated destination).
    pub pending: Vec<(SpId, u64)>,
    /// Balances that become an Ark send due at `H_f + confirmation_depth`.
    pub ark_due: Vec<(SpId, u64)>,
    /// Current-block earnings confiscated from an unregistered miner below
    /// `C`. A prior pending balance is never forfeited, so this is the
    /// block's raw portion only (SPEC §11.1).
    pub forfeited: Vec<(SpId, u64)>,
    /// Delta to apply to the prior pending balance to obtain the new one.
    ///
    /// The `pending` rail lists balances that wait in full; a kept id the
    /// coinbase could only partly cover, and a forfeited id with a prior
    /// balance, carry their remainder on this delta rather than a second rail
    /// entry, because the distribution carries exactly one entry per `sp_id`
    /// (SPEC §4.7; settlement rejects duplicate entries).
    pub pending_deltas: BTreeMap<SpId, i64>,
    pub pay_p: u64,
    pub total: u64,
    pub pool_fee: u64,
}

/// SPEC §4.6 + §11.1. `pending` is the prior balance per id.
///
/// `owed(id) = raw(id) + pending(id)` is computed for every id present in
/// **either** map (SPEC §4.6), so an id that only carries a prior pending
/// balance is still routed. Ids whose owed is zero are dropped (there is
/// nothing to route and no rail should carry a 0-sat entry).
///
/// # Coinbase cap (SPEC §4.6(a))
///
/// `Σ kept + pay_p = T` must hold, and the pool's own fee must stay in the
/// pool output, so `kept` is capped at `T − pool_sats` (`pool_sats` is the fee
/// plus rounding remainder from [`raw_payouts`]):
///
/// 1. Walk `owed` descending (ties by `sp_id`). Keep an id only when its whole
///    owed fits in the remaining capacity; the first id that does not fit
///    closes the coinbase, and it plus every smaller balance goes to its
///    non-coinbase rail (pending above `C`, Ark/retained/forfeited below).
/// 2. If the *largest* owed exceeds the capacity (`Σ kept == 0`), the coinbase
///    pays the whole capacity against it (only when that still clears `C`) and
///    the remainder is carried as pending. The cap therefore never produces a
///    negative or unsigned balance, and `pay_p >= pool_sats` always, so an
///    over-`T` balance cannot hand the pool fee to a miner.
/// 3. If `T <= pool_sats` the coinbase pays nothing and every balance waits.
///
/// `total` is the found block's revenue `T` (subsidy + fees); `pay_p` is
/// exactly `T − Σ kept`.
///
/// # Forfeiture (SPEC §11.1)
///
/// An unregistered id below `C` forfeits **this block's raw portion only**.
/// `pending` arises from the pool's own side (trimming, failed Ark sends, a
/// registration not yet propagated) and §11.1 says it is never forfeited, so a
/// prior balance is left on the pending ledger (`pending_delta = 0`). A
/// pending-only id below `C` therefore routes to the `pending` rail with
/// `pending_delta = 0`.
pub fn route_tiers(
    raw: &RawPayouts,
    pending: &BTreeMap<SpId, u64>,
    ark: &BTreeMap<SpId, ArkCapability>,
    cfg: &TierConfig,
    total: u64,
) -> Result<Routing, Error> {
    cfg.validate()?;

    let mut ids: BTreeSet<SpId> = raw.raw.keys().copied().collect();
    ids.extend(pending.keys().copied());
    let mut owed: Vec<(SpId, u64)> = ids
        .into_iter()
        .map(|id| {
            let r = raw.raw.get(&id).copied().unwrap_or(0);
            let p = pending.get(&id).copied().unwrap_or(0);
            (id, r.saturating_add(p))
        })
        .filter(|(_, amount)| *amount > 0)
        .collect();
    // Largest first; ties break on sp_id so the result is deterministic.
    owed.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let mut kept = Vec::new();
    let mut pending_out = Vec::new();
    let mut ark_due = Vec::new();
    let mut forfeited = Vec::new();
    let mut pending_deltas = BTreeMap::new();
    let mut used_wu = cfg.overhead_wu;
    let mut paid: u64 = 0;
    // The pool keeps at least `pool_sats` (fee + rounding) in `pay_p`: cap the
    // coinbase at `T − pool_sats` so an over-`T` balance cannot pay the fee out.
    let capacity = total.saturating_sub(raw.pool_sats);
    // Set once no further id can be paid by this coinbase (sats, slots or
    // weight exhausted): the walk is a prefix of the owed order.
    let mut coinbase_closed = false;

    for (id, amount) in &owed {
        let raw_part = raw.raw.get(id).copied().unwrap_or(0);
        let old = pending.get(id).copied().unwrap_or(0);

        if !coinbase_closed && *amount >= cfg.coinbase_threshold_sats {
            let next_wu = used_wu.saturating_add(172);
            if kept.len() < cfg.max_payout_slots && next_wu <= cfg.budget_wu {
                let available = capacity.saturating_sub(paid);
                if *amount <= available {
                    used_wu = next_wu;
                    paid = paid.saturating_add(*amount);
                    kept.push((*id, *amount));
                    // Fully paid: the prior balance is consumed.
                    pending_deltas.insert(*id, pending_delta(0, old));
                    continue;
                }
                if paid == 0 && available > 0 && available >= cfg.coinbase_threshold_sats {
                    // A single balance larger than the payable capacity: pay
                    // what the coinbase can carry and keep the rest as pending
                    // (one output per id, SPEC §4.6(a)).
                    used_wu = next_wu;
                    paid = capacity;
                    kept.push((*id, available));
                    pending_deltas
                        .insert(*id, pending_delta(amount.saturating_sub(available), old));
                    coinbase_closed = true;
                    continue;
                }
            }
            // No further coinbase payment is possible this block.
            coinbase_closed = true;
        }

        if *amount >= cfg.coinbase_threshold_sats {
            // Trimmed above C waits for the next coinbase (SPEC §4.8).
            pending_out.push((*id, *amount));
            pending_deltas.insert(*id, pending_delta(*amount, old));
            continue;
        }

        match ark.get(id) {
            Some(cap) if cap.registered && cap.dest_known => {
                ark_due.push((*id, *amount));
                // The balance leaves the pending ledger and becomes an Ark
                // send due; `failed_to_pending` re-credits it if the send
                // fails (§11.3), so it must not sit in both places.
                pending_deltas.insert(*id, pending_delta(0, old));
            }
            Some(cap) if cap.registered => {
                // Committed registration, destination not propagated yet.
                pending_out.push((*id, *amount));
                pending_deltas.insert(*id, pending_delta(*amount, old));
            }
            _ => {
                // Never forfeit a prior pending balance: only this block's
                // raw earnings are confiscated (SPEC §11.1).
                if raw_part > 0 {
                    forfeited.push((*id, raw_part));
                } else if old > 0 {
                    // Pending-only: the balance simply keeps waiting.
                    pending_out.push((*id, old));
                }
                pending_deltas.insert(*id, pending_delta(old, old));
            }
        }
    }

    let pay_p = total.saturating_sub(paid);
    let pool_fee = raw.pool_sats;
    Ok(Routing {
        // Full owed per id, before the cap; JD weights use this.
        owed: owed.iter().copied().collect(),
        kept,
        pending: pending_out,
        ark_due,
        forfeited,
        pending_deltas,
        pay_p,
        total,
        pool_fee,
    })
}

/// Saturating `u64 -> i64` for the pending-delta ledger (no wrap, no panic).
fn to_i64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

/// `new_pending - old_pending` as a saturating `i64` (never wraps).
fn pending_delta(new_pending: u64, old_pending: u64) -> i64 {
    to_i64(new_pending).saturating_sub(to_i64(old_pending))
}

/// O11 claim: `claim(id) = floor(owed * (T_actual - F) / W)`.
///
/// * `t_actual` is `T_actual`, the sum of the **mined** coinbase outputs.
/// * `fixed_outputs_sats` is `F`, the fixed-output total of PR #203 §4 exactly
///   as in the SPEC §3.5 validation (0 when `additional_outputs` is only the
///   0-sat `OP_RETURN` carrier). It is **not** the pool weight: `F` is
///   subtracted before scaling.
/// * `weight_total` is `W = Σ weight_id + weight_P`, i.e. it **includes** the
///   pool weight. Passing only `Σ weight_id` here would over-credit every
///   miner.
///
/// `docs/decisions/2026-10-14-o11-claim-formula.md` is authoritative.
pub fn claim_jd(
    owed: u64,
    t_actual: u64,
    fixed_outputs_sats: u64,
    weight_total: u64,
) -> Result<u64, Error> {
    if weight_total == 0 || fixed_outputs_sats > t_actual {
        return Err(Error::Overflow);
    }
    // u64 × u64 always fits u128, so this cannot saturate or wrap.
    let scaled = (owed as u128) * ((t_actual - fixed_outputs_sats) as u128);
    let v = scaled / weight_total as u128;
    Ok(u64::try_from(v).unwrap_or(u64::MAX))
}

/// JD settlement for one id. `paid` is read from the mined coinbase.
///
/// `pending'(id) = claim(id) - paid(id)` and is unsigned: a miner the JDC
/// overpaid (`paid > claim`, e.g. a distribution the pool did not build) books
/// zero pending rather than a debit. A dust-pruned id (`paid = 0`) books the
/// whole claim as pending. See the parameter docs on [`claim_jd`].
pub fn settle_jd(
    owed: u64,
    paid: u64,
    t_actual: u64,
    fixed_outputs_sats: u64,
    weight_total: u64,
) -> Result<(u64, u64), Error> {
    let claim = claim_jd(owed, t_actual, fixed_outputs_sats, weight_total)?;
    let pending = claim.saturating_sub(paid);
    Ok((claim, pending))
}

/// Weight-budget autoscaler (SPEC §4.8).
#[derive(Clone, Debug, PartialEq)]
pub struct Autoscaler {
    pub budget_wu: u32,
    pub floor_wu: u32,
    pub ceiling_wu: u32,
    pub up_threshold: f64,
    pub down_threshold: f64,
    pub step: f64,
    pub up_debounce: u32,
    pub down_debounce: u32,
    pub cooldown_secs: u64,
    up_count: u32,
    down_count: u32,
    last_change: u64,
}

impl Autoscaler {
    pub fn new(budget_wu: u32, floor_wu: u32, ceiling_wu: u32) -> Self {
        Self {
            budget_wu,
            floor_wu,
            ceiling_wu,
            up_threshold: 0.85,
            down_threshold: 0.50,
            step: 1.15,
            up_debounce: 3,
            down_debounce: 20,
            cooldown_secs: 300,
            up_count: 0,
            down_count: 0,
            last_change: 0,
        }
    }

    /// Feed one template's utilisation. Returns a new budget when it changes.
    pub fn on_template(&mut self, used_wu: u32, now_secs: u64) -> Option<u32> {
        let util = used_wu as f64 / self.budget_wu.max(1) as f64;
        if util >= self.up_threshold {
            self.up_count = self.up_count.saturating_add(1);
            self.down_count = 0;
        } else if util <= self.down_threshold {
            self.down_count = self.down_count.saturating_add(1);
            self.up_count = 0;
        } else {
            self.up_count = 0;
            self.down_count = 0;
            return None;
        }

        if now_secs.saturating_sub(self.last_change) < self.cooldown_secs {
            return None;
        }

        if self.up_count >= self.up_debounce {
            let next = ((self.budget_wu as f64) * self.step).ceil() as u32;
            let next = next.min(self.ceiling_wu);
            if next != self.budget_wu {
                self.budget_wu = next;
                self.up_count = 0;
                self.last_change = now_secs;
                return Some(next);
            }
        }
        if self.down_count >= self.down_debounce {
            let next = ((self.budget_wu as f64) / self.step).floor() as u32;
            let next = next.max(self.floor_wu);
            if next != self.budget_wu {
                self.budget_wu = next;
                self.down_count = 0;
                self.last_change = now_secs;
                return Some(next);
            }
        }
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Rail {
    Coinbase = 0,
    Ark = 1,
    Pending = 2,
    Forfeit = 3,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub sp_id: SpId,
    pub rail: Rail,
    pub p_x: Option<[u8; 32]>,
    pub dest_id: Option<Vec<u8>>,
    pub sats: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Distribution {
    pub height: u32,
    pub a_send_x: [u8; 32],
    pub outputs: Vec<(Vec<u8>, u64)>,
    pub entries: Vec<Entry>,
    pub fingerprint: [u8; 32],
    pub pay_p: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MinerKeys {
    pub sp_id: SpId,
    pub b_scan: bitcoin::secp256k1::PublicKey,
    pub b_spend: bitcoin::secp256k1::PublicKey,
}

/// Build the pool-template (sats) distribution (SPEC §4.7).
///
/// Output order is `[pool output] ++ [miner outputs sorted by sp_id]` as the
/// spec requires, so the fingerprint is independent of the trim order.
pub fn build_distribution(
    height: u32,
    send: &SendKey,
    routing: &Routing,
    miners: &BTreeMap<SpId, MinerKeys>,
    pool_script: &[u8],
    ark_dest: &BTreeMap<SpId, Vec<u8>>,
) -> Result<Distribution, Error> {
    let kept = sorted_by_id(&routing.kept);
    let mut outputs = Vec::with_capacity(kept.len() + 1);
    outputs.push((pool_script.to_vec(), routing.pay_p));
    let mut entries = Vec::new();

    for (id, sats) in &kept {
        let keys = miners.get(id).ok_or(Error::Derivation(*id))?;
        let ecdh = send
            .derive_pool_side(&keys.b_scan)
            .map_err(|_| Error::Derivation(*id))?;
        let tweaks = tier_tweaks(&ecdh, 0).map_err(|_| Error::Derivation(*id))?;
        let p = spc_output_pubkey(&keys.b_spend, &tweaks.t_onchain)
            .map_err(|_| Error::Derivation(*id))?;
        let p_x = p.x_only_public_key().0.serialize();
        outputs.push((p2tr_script(&p_x), *sats));
        entries.push(Entry {
            sp_id: *id,
            rail: Rail::Coinbase,
            p_x: Some(p_x),
            dest_id: ark_dest.get(id).cloned(),
            sats: *sats,
        });
    }
    for (id, sats) in sorted_by_id(&routing.pending) {
        entries.push(Entry {
            sp_id: id,
            rail: Rail::Pending,
            p_x: None,
            dest_id: None,
            sats,
        });
    }
    for (id, sats) in sorted_by_id(&routing.ark_due) {
        entries.push(Entry {
            sp_id: id,
            rail: Rail::Ark,
            p_x: None,
            dest_id: ark_dest.get(&id).cloned(),
            sats,
        });
    }
    for (id, sats) in sorted_by_id(&routing.forfeited) {
        entries.push(Entry {
            sp_id: id,
            rail: Rail::Forfeit,
            p_x: None,
            dest_id: None,
            sats,
        });
    }

    let fingerprint = fingerprint(height, send.x_only(), &outputs);
    Ok(Distribution {
        height,
        a_send_x: *send.x_only(),
        outputs,
        entries,
        fingerprint,
        pay_p: routing.pay_p,
    })
}

/// Copy a routing rail sorted by `sp_id` (deterministic, spec order).
fn sorted_by_id(rail: &[(SpId, u64)]) -> Vec<(SpId, u64)> {
    let mut out = rail.to_vec();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// JD weights: `weight_id = owed(id)` for every kept id,
/// `weight_P = fee_weight + Σ_{not kept} owed(id)` (SPEC §4.7, §11.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Weights {
    pub pool_weight: u64,
    pub weights: Vec<(SpId, u64)>,
    pub entries: Vec<Entry>,
}

/// Build the JD weights from the same routing that produced the payout tiers
/// (SPEC §4.7: "the two modes share everything except the final scaling").
///
/// `weights` carries each kept id's **full owed** (`Routing::owed`) in `sp_id`
/// order; `pool_weight` folds `fee_weight` plus the owed of every id that is
/// not in the coinbase, which is what the pool output is worth in JD mode
/// (§11.2).
///
/// The pool-template coinbase cap (SPEC §4.6(a)) does **not** apply here: JD
/// weights are not sats, and the JDC/O11 scales them by
/// `(T_actual − F) / W` at settlement, so a balance the pool-template coinbase
/// could only partly pay keeps its full weight. The rail lists are still used
/// for `entries`, because they describe what the snapshot actually allocated.
pub fn build_weights(routing: &Routing, fee_weight: u64) -> Weights {
    let kept = sorted_by_id(&routing.kept);
    let kept_ids: BTreeSet<SpId> = kept.iter().map(|(id, _)| *id).collect();
    let held = routing
        .owed
        .iter()
        .filter(|(id, _)| !kept_ids.contains(*id))
        .fold(0u64, |acc, (_, v)| acc.saturating_add(*v));

    // `Routing::owed` is authoritative; the rail amount is a fallback for a
    // hand-built routing that does not populate it.
    let weights: Vec<(SpId, u64)> = kept
        .iter()
        .map(|(id, sats)| (*id, routing.owed.get(id).copied().unwrap_or(*sats)))
        .collect();

    let mut entries = Vec::with_capacity(
        kept.len() + routing.pending.len() + routing.ark_due.len() + routing.forfeited.len(),
    );
    for (id, sats) in &kept {
        entries.push(Entry {
            sp_id: *id,
            rail: Rail::Coinbase,
            p_x: None,
            dest_id: None,
            sats: *sats,
        });
    }
    for (rail, group) in [
        (Rail::Pending, &routing.pending),
        (Rail::Ark, &routing.ark_due),
        (Rail::Forfeit, &routing.forfeited),
    ] {
        for (id, sats) in sorted_by_id(group) {
            entries.push(Entry {
                sp_id: id,
                rail,
                p_x: None,
                dest_id: None,
                sats,
            });
        }
    }

    Weights {
        pool_weight: fee_weight.saturating_add(held),
        weights,
        entries,
    }
}

pub fn p2tr_script(x: &[u8; 32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(34);
    out.push(0x51);
    out.push(0x20);
    out.extend_from_slice(x);
    out
}

pub fn fingerprint(height: u32, a_x: &[u8; 32], outputs: &[(Vec<u8>, u64)]) -> [u8; 32] {
    use bitcoin::hashes::{sha256, Hash, HashEngine};
    let mut eng = sha256::Hash::engine();
    eng.input(&height.to_be_bytes());
    eng.input(a_x);
    for (script, value) in outputs {
        eng.input(&value.to_le_bytes());
        eng.input(&(script.len() as u32).to_le_bytes());
        eng.input(script);
    }
    sha256::Hash::from_engine(eng).to_byte_array()
}

#[cfg(test)]
mod tests;
