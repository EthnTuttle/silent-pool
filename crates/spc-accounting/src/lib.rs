//! Pure SLICE/PPLNS-JD accounting (SPEC §4, §11.1).
//!
//! No I/O. Every function is deterministic; floors are applied per term as the
//! spec requires. Proptests pin conservation and the pool-template collapse.

#![deny(unsafe_code)]

use std::collections::BTreeMap;

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
        for s in &self.shares {
            let e = per_id.entry(s.sp_id).or_insert((0u128, 0u128));
            e.0 = e.0.saturating_add(s.d as u128);
            e.1 =
                e.1.saturating_add((s.d as u128).saturating_mul(s.job_fees as u128));
        }
        SliceAgg {
            slice_id: self.slice_id,
            height: self.height,
            prev_hash: self.prev_hash,
            front_id: self.front_id,
            difficulty: self.difficulty,
            diff_fee_sum: self
                .shares
                .iter()
                .map(|s| (s.d as u128).saturating_mul(s.job_fees as u128))
                .sum(),
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
            level.push(*level.last().expect("non-empty"));
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

pub fn raw_payouts(
    window: &Window,
    subsidy: u64,
    fees: u64,
    fee_bps: u16,
) -> Result<RawPayouts, Error> {
    if window.t == 0 || window.d_win == 0 {
        return Err(Error::EmptyWindow);
    }
    if fee_bps > 10_000 {
        return Err(Error::FeeBps);
    }
    let fee_bps = fee_bps as u128;
    let r_net =
        (subsidy as u128).saturating_sub((subsidy as u128).saturating_mul(fee_bps) / 10_000);
    let f_net = (fees as u128).saturating_sub((fees as u128).saturating_mul(fee_bps) / 10_000);

    let mut raw: BTreeMap<SpId, u128> = BTreeMap::new();
    for s in &window.slices {
        for (id, (d, df)) in &s.per_id {
            let subsidy_term = r_net.saturating_mul(*d) / window.d_win;
            let fee_term = if s.diff_fee_sum == 0 {
                0
            } else {
                f_net.saturating_mul(*df) / ((window.t as u128).saturating_mul(s.diff_fee_sum))
            };
            let e = raw.entry(*id).or_insert(0);
            *e = e.saturating_add(subsidy_term).saturating_add(fee_term);
        }
    }
    let raw: BTreeMap<SpId, u64> = raw
        .into_iter()
        .map(|(id, v)| (id, u64::try_from(v).unwrap_or(u64::MAX)))
        .collect();
    let total: u128 = raw.values().map(|v| *v as u128).sum();
    let reward = subsidy as u128 + fees as u128;
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
    pub kept: Vec<(SpId, u64)>,
    pub pending: Vec<(SpId, u64)>,
    pub ark_due: Vec<(SpId, u64)>,
    pub forfeited: Vec<(SpId, u64)>,
    pub pending_deltas: BTreeMap<SpId, i64>,
    pub pay_p: u64,
    pub total: u64,
    pub pool_fee: u64,
}

/// SPEC §4.6 + §11.1. `pending` is the prior balance per id.
pub fn route_tiers(
    raw: &RawPayouts,
    pending: &BTreeMap<SpId, u64>,
    ark: &BTreeMap<SpId, ArkCapability>,
    cfg: &TierConfig,
    total: u64,
) -> Result<Routing, Error> {
    cfg.validate()?;

    let mut owed: Vec<(SpId, u64)> = raw
        .raw
        .iter()
        .map(|(id, r)| {
            let p = pending.get(id).copied().unwrap_or(0);
            (*id, r.saturating_add(p))
        })
        .collect();
    owed.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let mut kept = Vec::new();
    let mut pending_out = Vec::new();
    let mut ark_due = Vec::new();
    let mut forfeited = Vec::new();
    let mut pending_deltas = BTreeMap::new();
    let mut used_wu = cfg.overhead_wu;
    let mut kept_ids = Vec::new();

    for (id, amount) in &owed {
        let old = pending.get(id).copied().unwrap_or(0) as i64;
        if *amount >= cfg.coinbase_threshold_sats {
            let next_wu = used_wu.saturating_add(172);
            if kept.len() < cfg.max_payout_slots && next_wu <= cfg.budget_wu {
                used_wu = next_wu;
                kept.push((*id, *amount));
                kept_ids.push(*id);
                pending_deltas.insert(*id, -old);
                continue;
            }
            // Trimmed above C waits for the next coinbase.
            pending_out.push((*id, *amount));
            pending_deltas.insert(*id, *amount as i64 - old);
            continue;
        }

        match ark.get(id) {
            Some(cap) if cap.registered && cap.dest_known => {
                ark_due.push((*id, *amount));
                pending_deltas.insert(*id, *amount as i64 - old);
            }
            Some(cap) if cap.registered => {
                pending_out.push((*id, *amount));
                pending_deltas.insert(*id, *amount as i64 - old);
            }
            _ => {
                forfeited.push((*id, *amount));
                // A forfeited balance is not owed; its prior pending, if any,
                // is released so the ledger stays consistent.
                pending_deltas.insert(*id, -old);
            }
        }
    }

    let paid: u128 = kept.iter().map(|(_, v)| *v as u128).sum();
    let pay_p = (total as u128).saturating_sub(paid);
    let pool_fee = raw.pool_sats;
    Ok(Routing {
        kept,
        pending: pending_out,
        ark_due,
        forfeited,
        pending_deltas,
        pay_p: u64::try_from(pay_p).unwrap_or(u64::MAX),
        total,
        pool_fee,
    })
}

/// O11: `claim(id) = floor(owed * (T_actual - F) / W)`.
pub fn claim_jd(owed: u64, t_actual: u64, fixed: u64, weight_total: u64) -> Result<u64, Error> {
    if weight_total == 0 || fixed > t_actual {
        return Err(Error::Overflow);
    }
    let v = (owed as u128).saturating_mul((t_actual - fixed) as u128) / weight_total as u128;
    Ok(u64::try_from(v).unwrap_or(u64::MAX))
}

/// JD settlement for one id. `paid` is read from the mined coinbase.
pub fn settle_jd(
    owed: u64,
    paid: u64,
    t_actual: u64,
    fixed: u64,
    weight_total: u64,
) -> Result<(u64, u64), Error> {
    let claim = claim_jd(owed, t_actual, fixed, weight_total)?;
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
pub fn build_distribution(
    height: u32,
    send: &SendKey,
    routing: &Routing,
    miners: &BTreeMap<SpId, MinerKeys>,
    pool_script: &[u8],
    ark_dest: &BTreeMap<SpId, Vec<u8>>,
) -> Result<Distribution, Error> {
    let mut outputs = Vec::with_capacity(routing.kept.len() + 1);
    outputs.push((pool_script.to_vec(), routing.pay_p));
    let mut entries = Vec::new();

    for (id, sats) in &routing.kept {
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
    for (id, sats) in &routing.pending {
        entries.push(Entry {
            sp_id: *id,
            rail: Rail::Pending,
            p_x: None,
            dest_id: None,
            sats: *sats,
        });
    }
    for (id, sats) in &routing.ark_due {
        entries.push(Entry {
            sp_id: *id,
            rail: Rail::Ark,
            p_x: None,
            dest_id: ark_dest.get(id).cloned(),
            sats: *sats,
        });
    }
    for (id, sats) in &routing.forfeited {
        entries.push(Entry {
            sp_id: *id,
            rail: Rail::Forfeit,
            p_x: None,
            dest_id: None,
            sats: *sats,
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

/// JD weights: `weight_id = owed`, `weight_P = fee + sum(not kept owed)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Weights {
    pub pool_weight: u64,
    pub weights: Vec<(SpId, u64)>,
    pub entries: Vec<Entry>,
}

pub fn build_weights(routing: &Routing, fee_weight: u64) -> Weights {
    let held: u64 = routing
        .pending
        .iter()
        .chain(routing.ark_due.iter())
        .chain(routing.forfeited.iter())
        .map(|(_, v)| *v)
        .sum();
    let mut entries = Vec::new();
    for (id, sats) in &routing.kept {
        entries.push(Entry {
            sp_id: *id,
            rail: Rail::Coinbase,
            p_x: None,
            dest_id: None,
            sats: *sats,
        });
    }
    Weights {
        pool_weight: fee_weight.saturating_add(held),
        weights: routing.kept.clone(),
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
