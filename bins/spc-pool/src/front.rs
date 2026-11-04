//! Front-role state: one `a_send` per height, the open slice, the current
//! distribution and the job the miners are hashing (SPEC §2.2, §3.3, §4.7).
//!
//! ## Share difficulty and the window
//!
//! The front applies the share-difficulty policy from
//! [`crate::sv1::effective_share_target`] and [`crate::sv1::credited_difficulty`]
//! (decision 2026-11-02-share-target-policy): the same credited `d` is used for
//! the `ShareRec`, the window target (`N x d`) and the slice aggregates, while
//! block detection still keys off the consensus target. A share that does not
//! meet the effective share target is rejected here as well as at the ingress
//! (fail closed in depth).
//!
//! ## Pending balances
//!
//! `pending` only moves when the payout role says so: a `LedgerApplied` event,
//! published after the block is confirmed and booked (SPEC §4.6, §7.4). A
//! forecast refresh never moves money, and finding a block on the network
//! target creates no liability before settlement - so an orphaned block leaves
//! no phantom balance behind. The routing of the current distribution is kept
//! for inspection but is not applied to the ledger.

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};

use bitcoin::hashes::Hash;
use bitcoin::pow::Target;
use bitcoin::BlockHash;
use spc_accounting::{
    build_distribution, build_weights, raw_payouts, route_tiers, select_window, ArkCapability,
    Distribution, MinerKeys, Routing, ShareRec, Slice, SliceAgg, SpId, TierConfig, Weights,
};
use spc_crypto::{Network, SendKey};
use spc_store::models::{BootstrapSnapshot, SliceRow};
use spc_store::wire::FrontEvent as WireFrontEvent;
use spc_store::wire::{BlockEvent, Entry as WireEntry, Rail as WireRail, SliceEvent};
use thiserror::Error;

use crate::block::{assemble, coinbase_merkle_branch, merkle_root, AssembledBlock};
use crate::coinbase::{build_pool_template, CoinbaseTemplate};
use crate::sv1::{
    credited_difficulty, effective_share_target, network_target, ntime_allowed, nullifier,
    version_allowed, Job, Share,
};
use crate::template::{template_txids, Template, TemplateError, TemplateSource};
use crate::type32::Type32Store;

/// The 56 low bits of a slice id: `slice_id = front_id << 56 | local_seq`
/// (MULTI-REGION §10, same composition as the JDS distribution id).
const LOCAL_SEQ_MASK: u64 = 0x00ff_ffff_ffff_ffff;

/// The front id encoded in a slice id.
fn slice_front_id(slice_id: u64) -> u8 {
    u8::try_from(slice_id >> 56).unwrap_or(0)
}

#[derive(Debug, Error)]
pub enum FrontError {
    #[error("template: {0}")]
    Template(#[from] TemplateError),
    #[error("accounting: {0}")]
    Accounting(#[from] spc_accounting::Error),
    #[error("crypto: {0}")]
    Crypto(#[from] spc_crypto::Error),
    #[error("pool payout derivation: {0}")]
    PoolPayout(String),
    #[error("no active template")]
    NoTemplate,
    #[error("no active slice")]
    NoSlice,
    #[error("no send key")]
    NoSendKey,
    #[error("share_difficulty must be > 0")]
    ShareDifficulty,
    #[error("share does not meet the pool share target")]
    LowDifficulty,
    #[error("stale job")]
    StaleJob,
    #[error("no active job")]
    NoJob,
    #[error("ntime out of range")]
    StaleTime,
    #[error("version is not the job version under the BIP 320 mask")]
    InvalidVersion,
    #[error("duplicate share")]
    DuplicateShare,
}

#[derive(Clone, Debug)]
pub struct FrontConfig {
    pub network: Network,
    pub fee_bps: u16,
    pub window_n: u32,
    pub slice_fee_delta_sats: u64,
    pub max_slice_shares: usize,
    pub dust_limit_sats: u64,
    pub coinbase_threshold_sats: u64,
    pub max_payout_slots: usize,
    pub budget_wu: u32,
    pub overhead_wu: u32,
    /// Parsed `[pool] pool_payout_descriptor` (SPEC §6.3, §11.3). The pool
    /// output for height `H` is derived from it at `H`; the SPc side holds
    /// only this public key material.
    pub pool_payout_descriptor: crate::coinbase::PoolPayoutDescriptor,
    /// Explicit dev/test override of the pool output script (hex in
    /// `[pool] pool_payout_script`). When set it is used verbatim and no
    /// derivation happens; there is no fallback when it is absent.
    pub pool_payout_script: Option<Vec<u8>>,
    pub extranonce_len: usize,
    pub front_id: u8,
    pub keep_slices: usize,
}

pub struct Front {
    cfg: FrontConfig,
    source: Arc<dyn TemplateSource>,
    send: Option<SendKey>,
    height: u32,
    template: Option<Template>,
    job: Option<Job>,
    distribution: Option<Distribution>,
    /// JD weights for the current distribution (SPEC §4.7, §11.2), built from
    /// the same routing as the coinbase distribution.
    weights: Option<Weights>,
    open: Option<Slice>,
    slices: Vec<SliceAgg>,
    /// Last local slice sequence (56 bits). The composed id is
    /// `front_id << 56 | local_seq`; `local_seq = max(last + 1, unix_ms)` so a
    /// restart never reuses an id (MULTI-REGION §10, SPEC §5.1).
    slice_seq: u64,
    /// Newest sealed slice actually included in the current distribution
    /// window, `0` when the window holds no sealed slice. This is the
    /// inclusive high-water mark settlement uses (SPEC §4.3): the probing
    /// share's own slice is still open and therefore above the mark.
    high_water: u64,
    /// The exact window target (`N x d`, SPEC §4.3) the current
    /// distribution's window was selected with. Frozen into the block event
    /// and then `distribution_snapshot` so settlement reconstructs the same
    /// window even when a retarget has since changed the stored slices'
    /// implied `d` (decision 2026-11-03-settlement-window-target).
    window_target: u128,
    miners: BTreeMap<SpId, MinerKeys>,
    /// The pending ledger, and the frozen base every forecast routes against:
    /// it only changes when a block is actually found (or from a bootstrap /
    /// `LedgerApplied`), never on a template refresh (SPEC §4.6).
    pending: BTreeMap<SpId, u64>,
    ark: BTreeMap<SpId, ArkCapability>,
    ark_dest: BTreeMap<SpId, Vec<u8>>,
    /// Pool share difficulty (`[accounting] share_difficulty`).
    share_difficulty: u64,
    /// Policy applied for the current template: the credited `d` and the
    /// target every share must meet.
    policy: Option<AppliedPolicy>,
    /// The routing of the current distribution: the frozen view of the
    /// distribution the miners are hashing. `pending` only moves when a block
    /// is actually found (never on a forecast refresh), using these deltas.
    routing: Option<Routing>,
    /// Process-wide, identity-free replay set (SPEC §3.4):
    /// `sha256(channel_id || job_id || nonce || ntime || version || extranonce)`.
    /// Cleared when the prev-hash changes.
    seen_shares: HashSet<[u8; 32]>,
    seen_prev_hash: Option<[u8; 32]>,
    /// Monotonic per-template job sequence; the wire job id is its hex form.
    job_seq: u32,
    /// Merkle identity of the template's non-coinbase tx set, used to detect a
    /// same-height tx-set change.
    template_id: [u8; 32],
    /// Non-coinbase txids of the current template, in template order (type-32
    /// `GetTransactionsInJob`).
    txids: Vec<[u8; 32]>,
    /// Fingerprint of the distribution last published, to detect a
    /// distribution-only change that still needs a new job id.
    published_fingerprint: Option<[u8; 32]>,
    /// Highest `spc:front:events` revision applied (bootstrap or live).
    last_rev: u64,
    /// Confirmed blocks whose `LedgerApplied` is already reflected in
    /// `pending`: bootstrapped from the snapshot and extended as events are
    /// applied, so a replayed/republished event is a no-op regardless of `rev`.
    applied_blocks: HashSet<[u8; 32]>,
    last_block: Option<AssembledBlock>,
    events: Vec<FrontEvent>,
    type32: Arc<Mutex<Type32Store>>,
}

/// The share policy in force for one template (decision
/// 2026-11-02-share-target-policy).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AppliedPolicy {
    /// `d` booked for every share: never 0, never above network difficulty.
    credited_difficulty: u64,
    /// target a share must meet: never harder than the network target.
    share_target: Target,
}

#[derive(Clone, Debug)]
pub enum FrontEvent {
    Slice(SliceEvent),
    Block(BlockEvent),
}

impl FrontConfig {
    pub fn from_config(c: &crate::config::Config) -> Result<Self, String> {
        let network = c.network().map_err(|e| e.to_string())?;
        // Fail-closed descriptor parse (SPEC §6.3): the three xpub-spec
        // assertions plus network binding. There is no zero-key fallback.
        let descriptor =
            crate::coinbase::PoolPayoutDescriptor::parse(&c.pool.pool_payout_descriptor, network)?;
        let pool_payout_script = match &c.pool.pool_payout_script {
            Some(h) => {
                let script = hex::decode(h).map_err(|e| format!("pool_payout_script: {e}"))?;
                // A dev override that is not an unambiguous 34-byte P2TR
                // is rejected, not guessed at.
                if script.len() != 34 || script[..2] != [0x51, 0x20] {
                    return Err(
                        "pool_payout_script must be a 34-byte P2TR script (hex)".to_string()
                    );
                }
                Some(script)
            }
            None => None,
        };
        Ok(Self {
            network,
            fee_bps: c.pool.fee_bps,
            window_n: c.accounting.window_n,
            slice_fee_delta_sats: c.accounting.slice_fee_delta_sats,
            max_slice_shares: c.accounting.max_slice_shares,
            dust_limit_sats: c.accounting.dust_limit_sats,
            coinbase_threshold_sats: c.accounting.coinbase_threshold_sats,
            max_payout_slots: c.accounting.max_payout_slots,
            budget_wu: c.budget.floor_wu,
            overhead_wu: 0,
            pool_payout_descriptor: descriptor,
            pool_payout_script,
            extranonce_len: c.sv1.extranonce1_bytes + c.sv1.extranonce2_bytes,
            front_id: c.pool.front_id,
            keep_slices: c.accounting.retain_windows as usize,
        })
    }

    /// The pool output script for height `H` (SPEC §11.3): the explicit
    /// dev/test override when present, else the descriptor derived at `H`.
    pub fn pool_script_at(&self, height: u32) -> Result<Vec<u8>, FrontError> {
        if let Some(script) = &self.pool_payout_script {
            return Ok(script.clone());
        }
        Ok(self
            .pool_payout_descriptor
            .script_at(height)
            .map_err(FrontError::PoolPayout)?
            .as_bytes()
            .to_vec())
    }
}

impl Front {
    pub fn new(
        cfg: FrontConfig,
        source: Arc<dyn TemplateSource>,
        type32: Arc<Mutex<Type32Store>>,
    ) -> Self {
        Self {
            cfg,
            source,
            send: None,
            height: 0,
            template: None,
            job: None,
            distribution: None,
            weights: None,
            open: None,
            slices: Vec::new(),
            slice_seq: 0,
            high_water: 0,
            window_target: 0,
            miners: BTreeMap::new(),
            pending: BTreeMap::new(),
            ark: BTreeMap::new(),
            ark_dest: BTreeMap::new(),
            share_difficulty: 1,
            policy: None,
            routing: None,
            seen_shares: HashSet::new(),
            seen_prev_hash: None,
            job_seq: (unix_ms() as u32) | 1,
            template_id: [0u8; 32],
            txids: Vec::new(),
            published_fingerprint: None,
            last_rev: 0,
            applied_blocks: HashSet::new(),
            last_block: None,
            events: Vec::new(),
            type32,
        }
    }

    /// Set the pool share difficulty (validated `> 0`; 0 is rejected rather
    /// than silently clamped so a bad config cannot weaken share validation).
    pub fn set_share_difficulty(&mut self, share_difficulty: u64) -> Result<(), FrontError> {
        if share_difficulty == 0 {
            return Err(FrontError::ShareDifficulty);
        }
        self.share_difficulty = share_difficulty;
        Ok(())
    }

    pub fn share_difficulty(&self) -> u64 {
        self.share_difficulty
    }

    pub fn job(&self) -> Option<&Job> {
        self.job.as_ref()
    }

    pub fn distribution(&self) -> Option<&Distribution> {
        self.distribution.as_ref()
    }

    pub fn last_block(&self) -> Option<&AssembledBlock> {
        self.last_block.as_ref()
    }

    pub fn template_revenue(&self) -> Option<u64> {
        self.template.as_ref().map(|t| t.coinbase_value)
    }

    pub fn pending(&self) -> &BTreeMap<SpId, u64> {
        &self.pending
    }

    /// JD weights for the current distribution, built from the same routing as
    /// the coinbase distribution (SPEC §4.7). `None` before the first
    /// `refresh`.
    pub fn weights(&self) -> Option<&Weights> {
        self.weights.as_ref()
    }

    /// P2TR script of the pool output in the current distribution, the
    /// `pool_payout` of the JD distribution. Derived per height from the
    /// descriptor (SPEC §11.3); `None` before the first `refresh`.
    pub fn pool_payout_script(&self) -> Option<&[u8]> {
        self.distribution
            .as_ref()?
            .outputs
            .first()
            .map(|(script, _)| script.as_slice())
    }

    /// The inclusive high-water mark of the current window (SPEC §4.3).
    pub fn high_water(&self) -> u64 {
        self.high_water
    }

    /// Merkle identity of the current template's non-coinbase tx set.
    pub fn template_id(&self) -> [u8; 32] {
        self.template_id
    }

    /// Non-coinbase txids of the current template, in template order.
    pub fn template_txids(&self) -> &[[u8; 32]] {
        &self.txids
    }

    /// `(slice_id, ref_job_id)` of the open slice, for diagnostics and tests.
    pub fn open_slice_refs(&self) -> Option<(u64, u64)> {
        self.open.as_ref().map(|s| (s.slice_id, s.ref_job_id))
    }

    /// The routing snapshot the current distribution was built from. Kept for
    /// inspection only: it never mutates `pending`.
    pub fn routing(&self) -> Option<&Routing> {
        self.routing.as_ref()
    }

    /// Last assigned local slice sequence (tests and logs).
    pub fn local_slice_seq(&self) -> u64 {
        self.slice_seq
    }

    /// Learn a miner's keys without a share, e.g. from a registration message
    /// that carries `B_scan`/`B_spend`. Only key learning: no accounting side
    /// effect.
    pub fn learn_identity(
        &mut self,
        sp_id: SpId,
        b_scan: [u8; 33],
        b_spend: [u8; 33],
    ) -> Result<(), FrontError> {
        self.miners.insert(
            sp_id,
            MinerKeys {
                sp_id,
                b_scan: bitcoin::secp256k1::PublicKey::from_slice(&b_scan)
                    .map_err(|_| FrontError::NoSendKey)?,
                b_spend: bitcoin::secp256k1::PublicKey::from_slice(&b_spend)
                    .map_err(|_| FrontError::NoSendKey)?,
            },
        );
        Ok(())
    }

    pub fn take_events(&mut self) -> Vec<FrontEvent> {
        std::mem::take(&mut self.events)
    }

    pub fn set_ark(&mut self, sp_id: SpId, cap: ArkCapability) {
        self.ark.insert(sp_id, cap);
    }

    /// Record the payable Ark destination for `sp_id` (a bark address, the
    /// `dest_id` of the block entry). Also marks the destination as known, so
    /// the next routing sends the Ark-tier balance instead of holding it.
    pub fn set_ark_dest(&mut self, sp_id: SpId, dest: Vec<u8>) {
        if dest.is_empty() {
            self.ark_dest.remove(&sp_id);
            if let Some(cap) = self.ark.get_mut(&sp_id) {
                cap.dest_known = false;
            }
            return;
        }
        self.ark_dest.insert(sp_id, dest);
        self.ark
            .entry(sp_id)
            .or_insert(ArkCapability {
                registered: false,
                dest_known: false,
            })
            .dest_known = true;
    }

    /// Apply a bootstrap snapshot (WIRE-FORMATS §3.5) before serving: pending
    /// balances, Ark registrations and the sealed window tail survive a front
    /// restart.
    ///
    /// The snapshot carries `b_ark` rather than a payable bark address, so a
    /// registration sets `registered` but not `dest_known`: an Ark-tier
    /// balance stays `pending` instead of being routed to a destination the
    /// front cannot describe.
    ///
    /// `window_tail` is replayed into the live window (SPEC §5.3): each
    /// `SliceRow` becomes a `SliceAgg`, newest first, truncated to
    /// `keep_slices`, and the local slice sequence plus the high-water mark are
    /// seeded from the maximum restored id so a restarted front never reuses
    /// or collides with an id that settlement already saw. Rows without
    /// per-identity credits are skipped: inventing zero credits would silently
    /// shrink the window. `type32_blobs` are not replayed here; the type-32
    /// service is served from the payout/bootstrap path.
    pub fn apply_bootstrap(&mut self, snapshot: &BootstrapSnapshot) {
        self.last_rev = self.last_rev.max(snapshot.rev);
        // The snapshot's pending already includes these blocks' ledger deltas;
        // remembering them makes a republished event a no-op even when its
        // `rev` is newer than the snapshot's.
        self.applied_blocks = snapshot.applied_blocks.iter().copied().collect();
        self.pending = snapshot
            .pending_balance
            .iter()
            .filter(|(_, sats)| *sats > 0)
            .map(|(id, sats)| (*id, *sats))
            .collect();
        for row in &snapshot.ark_delivery {
            self.ark
                .entry(row.sp_id)
                .or_insert(ArkCapability {
                    registered: false,
                    dest_known: false,
                })
                .registered = true;
        }

        let mut restored: Vec<SliceAgg> = snapshot
            .window_tail
            .iter()
            .filter(|row| slice_front_id(row.slice_id) == self.cfg.front_id)
            .filter_map(slice_agg_from_row)
            .collect();
        restored.sort_by(|a, b| b.slice_id.cmp(&a.slice_id));
        if let Some(newest) = restored.first().map(|s| s.slice_id) {
            // Ids continue above everything settlement has already seen.
            self.high_water = newest;
            self.slice_seq = newest & LOCAL_SEQ_MASK;
        }
        restored.truncate(self.cfg.keep_slices.max(1));
        self.slices = restored;
    }

    /// Apply one `spc:front:events` payload (WIRE-FORMATS §3.2). Returns true
    /// when the event changed front state. Events older than or equal to the
    /// last applied revision are ignored, as are events this front has no use
    /// for (`BlockFound`, `TipHeartbeat`) and `LedgerApplied` for a block this
    /// front settled itself.
    pub fn apply_front_event(&mut self, event: &WireFrontEvent) -> bool {
        let rev = event.rev();
        if rev != 0 && rev <= self.last_rev {
            return false;
        }
        self.last_rev = self.last_rev.max(rev);
        match event {
            // The payout role is the only writer of the pending ledger: the
            // delta is applied after the block is confirmed and booked, for
            // this front's own blocks too. A block already applied - from the
            // bootstrap snapshot or an earlier delivery of this event - is a
            // no-op, so replay/lost-XADD recovery is safe independently of
            // `rev`.
            WireFrontEvent::LedgerApplied {
                block_hash,
                pending_deltas,
                ..
            } => {
                if self.applied_blocks.contains(block_hash) {
                    return false;
                }
                let deltas: BTreeMap<SpId, i64> =
                    pending_deltas.iter().map(|(id, d)| (*id, *d)).collect();
                apply_pending_deltas(&mut self.pending, &deltas);
                self.applied_blocks.insert(*block_hash);
                true
            }
            WireFrontEvent::ArkRegApplied { sp_id, .. } => {
                self.ark
                    .entry(*sp_id)
                    .or_insert(ArkCapability {
                        registered: false,
                        dest_known: false,
                    })
                    .registered = true;
                true
            }
            WireFrontEvent::BlockFound { .. } | WireFrontEvent::TipHeartbeat { .. } => false,
        }
    }

    /// Fetch a template and build the job miners will hash.
    ///
    /// A new height, a different prev-hash (including a same-height reorg) or a
    /// changed tx set republishes the job with a new id; a new prev-hash also
    /// seals the open slice, because the shares in it were mined on the old
    /// fork (SPEC §3.3, §4.2).
    pub async fn refresh(&mut self) -> Result<Job, FrontError> {
        let template = self.source.get_template().await?;
        // Fail closed on transactions we cannot hash correctly: witness txs
        // need a BIP 141 commitment this pool does not build, and an
        // undecodable tx has no txid to put in the merkle branch.
        let txids = template_txids(&template.transactions)?;
        let merkle_branches = coinbase_merkle_branch(&txids);
        let template_id = merkle_root(&txids);

        let chain_moved = self.send.is_none()
            || self.height != template.height
            || self.open.as_ref().map(|s| s.prev_hash) != Some(template.prev_hash);
        let fees_moved = self.template.as_ref().map(|t| t.fees) != Some(template.fees);
        let job_changed = chain_moved || fees_moved || self.template_id != template_id;
        // The job id must exist before the slice is opened: the slice records
        // `(H << 32) | job_id` of the job it belongs to (SPEC §3.4).
        let mut bumped = false;
        if job_changed || self.job.is_none() {
            self.job_seq = self.job_seq.wrapping_add(1);
            bumped = true;
        }
        if chain_moved {
            self.roll_slice();
            if self.send.is_none() || self.height != template.height {
                self.send = Some(SendKey::generate(template.height)?);
            }
            self.height = template.height;
            self.open_slice(template.height, template.prev_hash, template.fees);
        }
        if self.seen_prev_hash != Some(template.prev_hash) {
            // Replay protection is scoped to one prev-hash (SPEC §3.4).
            self.seen_shares.clear();
            self.seen_prev_hash = Some(template.prev_hash);
        }
        self.template_id = template_id;
        self.txids = txids;

        // Decision 2026-11-02-share-target-policy: the credited difficulty is
        // the same `d` used for the ShareRec and for the window target. It is
        // never 0, so a regtest-style template still fills a window.
        let policy = AppliedPolicy {
            credited_difficulty: credited_difficulty(self.share_difficulty, template.bits),
            share_target: effective_share_target(self.share_difficulty, template.bits),
        };
        // Decision 2026-11-03-settlement-window-target: the exact target is
        // remembered with the distribution it selected, so settlement never
        // has to re-derive it from slices a retarget may have left behind.
        let window_target =
            (self.cfg.window_n as u128).saturating_mul(u128::from(policy.credited_difficulty));
        self.window_target = window_target;
        let window = select_window(&self.slices, u64::MAX, window_target, BTreeMap::new());
        // The mark is the newest *sealed* slice the window actually consumed,
        // not the still-open slice the probing share will land in (SPEC §4.3).
        // `0` means "no sealed slice in this window": no stored slice is at or
        // below it.
        self.high_water = window.slices.first().map(|s| s.slice_id).unwrap_or(0);
        let raw = if window.t == 0 {
            spc_accounting::RawPayouts {
                raw: BTreeMap::new(),
                pool_sats: template.coinbase_value,
                r_net: template.subsidy,
                f_net: template.fees,
            }
        } else {
            raw_payouts(&window, template.subsidy, template.fees, self.cfg.fee_bps)?
        };
        let tier_cfg = TierConfig {
            fee_bps: self.cfg.fee_bps,
            dust_limit_sats: self.cfg.dust_limit_sats,
            coinbase_threshold_sats: self.cfg.coinbase_threshold_sats,
            max_payout_slots: self.cfg.max_payout_slots,
            budget_wu: self.cfg.budget_wu,
            overhead_wu: self.cfg.overhead_wu,
        };
        let routing = route_tiers(
            &raw,
            &self.pending,
            &self.ark,
            &tier_cfg,
            template.coinbase_value,
        )?;
        // A restarted front can hold a restored window whose miners have not
        // sent a share yet, so their `P_i` cannot be derived. Hold those
        // balances as `pending` rather than paying them to a guessed output
        // (fail closed); the miner's next share restores the keys and the
        // balance is paid from the next coinbase.
        let mut routing = routing;
        hold_undeliverable(&mut routing, &self.miners, &self.pending);

        // JD weights come from the same routing (SPEC §4.7): kept ids carry
        // their owed amount, everything not kept folds into `pool_weight`.
        let weights = build_weights(&routing, raw.pool_sats);

        // Copy the carrier key so the distribution build does not hold a
        // borrow of `self` across the slice roll below.
        let a_send_x = *self.send.as_ref().ok_or(FrontError::NoSendKey)?.x_only();
        // The pool output is derived per height from the descriptor (SPEC
        // §11.3): a fresh key-path P2TR at index H, unless the explicit
        // dev/test override is set.
        let pool_script = self.cfg.pool_script_at(template.height)?;
        let distribution = {
            let send = self.send.as_ref().ok_or(FrontError::NoSendKey)?;
            build_distribution(
                template.height,
                send,
                &routing,
                &self.miners,
                &pool_script,
                &self.ark_dest,
            )?
        };

        // A distribution-only change (a slice sealed, a `LedgerApplied` moved
        // pending, a fee band sealed) is still a new coinbase: shares hashed
        // against the previous one must be stale, so the job id moves. The open
        // slice is sealed and reopened as well, so no slice ever mixes shares
        // minted under two different jobs.
        if !bumped && self.published_fingerprint != Some(distribution.fingerprint) {
            self.job_seq = self.job_seq.wrapping_add(1);
            self.roll_slice();
            self.open_slice(template.height, template.prev_hash, template.fees);
        }
        self.published_fingerprint = Some(distribution.fingerprint);

        let outputs: Vec<(bitcoin::script::ScriptBuf, u64)> = distribution
            .outputs
            .iter()
            .map(|(s, v)| (bitcoin::script::ScriptBuf::from_bytes(s.clone()), *v))
            .collect();
        let coinbase = build_pool_template(
            template.version,
            template.height,
            &a_send_x,
            self.cfg.extranonce_len,
            &outputs,
            0,
        );

        let job = Job {
            // The wire job id is the template sequence, not the height: an
            // equal-height reorg or tx-set change must invalidate stale work.
            job_id: format!("{:08x}", self.job_seq),
            prev_hash: template.prev_hash,
            height: template.height,
            version: template.version,
            nbits: template.bits,
            ntime: template.curtime,
            coinb1: coinbase.prefix,
            coinb2: coinbase.suffix,
            merkle_branches,
            clean_jobs: true,
        };
        self.job = Some(job.clone());
        self.distribution = Some(distribution);
        self.weights = Some(weights);
        self.routing = Some(routing);
        self.policy = Some(policy);
        self.template = Some(template);
        Ok(job)
    }

    /// Forward a validated share into the open slice and, if it meets the
    /// network target, assemble the block for submission.
    ///
    /// Every share is re-validated against the effective share target here (the
    /// front is the accounting authority; the ingresses check too). The credited
    /// `d` is the policy's, so the `ShareRec`, the window target and the slice
    /// aggregates all agree.
    pub fn on_share(&mut self, share: Share) -> Result<Option<AssembledBlock>, FrontError> {
        let template = self
            .template
            .as_ref()
            .ok_or(FrontError::NoTemplate)?
            .clone();
        let policy = self.policy.ok_or(FrontError::NoTemplate)?;
        let job = self.job.as_ref().ok_or(FrontError::NoJob)?;

        // Stale work is rejected before anything is credited: the job id
        // changes with the height, the prev-hash (including a same-height
        // reorg), the tx set and the job-fee level (SPEC §3.4).
        if share.job_id != job.job_id {
            return Err(FrontError::StaleJob);
        }
        // The submitted version must be the job's version outside the BIP 320
        // rollable bits, and `ntime` must be in the job's roll window: a header
        // outside them is not the template the pool can submit.
        if !version_allowed(template.version, share.version) {
            return Err(FrontError::InvalidVersion);
        }
        if !ntime_allowed(job.ntime, share.ntime) {
            return Err(FrontError::StaleTime);
        }

        // Fail closed: a share below the pool share target is never credited,
        // whatever the ingress did.
        if !policy
            .share_target
            .is_met_by(BlockHash::from_byte_array(share.share_hash))
        {
            return Err(FrontError::LowDifficulty);
        }

        // Process-wide replay guard, keyed exactly as SPEC §3.4 says. Both
        // ingresses keep their own per-connection set as defence in depth; this
        // one is authoritative, because it sees every ingress.
        let key = nullifier(
            share.channel_id,
            &share.job_id,
            share.nonce,
            share.ntime,
            share.version,
            &share.extranonce,
        );
        if !self.seen_shares.insert(key) {
            return Err(FrontError::DuplicateShare);
        }

        self.miners.insert(
            share.sp_id,
            MinerKeys {
                sp_id: share.sp_id,
                b_scan: bitcoin::secp256k1::PublicKey::from_slice(&share.b_scan)
                    .map_err(|_| FrontError::NoSendKey)?,
                b_spend: bitcoin::secp256k1::PublicKey::from_slice(&share.b_spend)
                    .map_err(|_| FrontError::NoSendKey)?,
            },
        );

        let rec = ShareRec {
            sp_id: share.sp_id,
            d: policy.credited_difficulty,
            // A job-declared share carries its own job fees; a pool-template
            // share leaves them at 0 and inherits the template's (SPEC §3.4).
            job_fees: if share.job_fees != 0 {
                share.job_fees
            } else {
                template.fees
            },
            // `(H << 32) | mining_job_id` (SPEC §3.4).
            job_id: (u64::from(template.height) << 32) | parse_job_id(&share.job_id),
            nonce: share.nonce,
            ntime: share.ntime,
            version: share.version,
            extranonce: share.extranonce.clone(),
            share_index: share.share_index,
        };
        let slice = self.open.as_mut().ok_or(FrontError::NoSlice)?;
        match slice.seal_if_needed(
            &rec,
            self.cfg.slice_fee_delta_sats,
            self.cfg.max_slice_shares,
        ) {
            spc_accounting::Seal::Seal(_) => {
                self.roll_slice();
                self.open_slice(self.height, template.prev_hash, template.fees);
                let slice = self.open.as_mut().ok_or(FrontError::NoSlice)?;
                slice.push(rec);
            }
            spc_accounting::Seal::Keep => slice.push(rec),
        }

        // Block detection is consensus work, not pool work: it always uses the
        // network target, never the (easier) share target.
        let hash = BlockHash::from_byte_array(share.share_hash);
        if !network_target(template.bits).is_met_by(hash) {
            return Ok(None);
        }

        // The miner hashed its own version (rolled within the BIP 320 mask),
        // so the block must carry that exact version.
        let block = assemble(
            share.version,
            template.prev_hash,
            share.ntime,
            template.bits,
            share.nonce,
            &share.coinbase,
            &template.transactions,
        );
        if let Some(dist) = &self.distribution {
            let block_hash = bitcoin::hashes::sha256d::Hash::hash(&block.header).to_byte_array();
            let entries = dist
                .entries
                .iter()
                .map(|e| WireEntry {
                    sp_id: e.sp_id,
                    rail: match e.rail {
                        spc_accounting::Rail::Coinbase => WireRail::Coinbase,
                        spc_accounting::Rail::Ark => WireRail::Ark,
                        spc_accounting::Rail::Pending => WireRail::Pending,
                        spc_accounting::Rail::Forfeit => WireRail::Forfeit,
                    },
                    p_x: e.p_x.map(|x| x.to_vec()).unwrap_or_default(),
                    dest_id: e.dest_id.clone().unwrap_or_default(),
                    sats: e.sats,
                })
                .collect();
            let high_water = BTreeMap::from([(self.cfg.front_id, self.high_water.to_string())]);
            self.events.push(FrontEvent::Block(BlockEvent {
                block_hash,
                height: template.height,
                front_id: self.cfg.front_id,
                fingerprint: dist.fingerprint,
                a_send_x: dist.a_send_x,
                high_water,
                // The exact target this distribution's window was selected
                // with (frozen at `refresh` time), not a re-derivation.
                window_target: self.window_target,
                entries,
            }));
        }
        self.last_block = Some(block.clone());
        Ok(Some(block))
    }

    /// Sealed slices, newest first, for the window and type-32 service.
    pub fn sealed_slices(&self) -> &[SliceAgg] {
        &self.slices
    }

    fn roll_slice(&mut self) {
        if let Some(mut open) = self.open.take() {
            if !open.shares.is_empty() {
                open.seal();
                let credits = open
                    .agg()
                    .per_id
                    .iter()
                    .map(|(id, (d, df))| spc_store::wire::SliceCredit {
                        sp_id: *id,
                        diff_sum: *d,
                        diff_fee_sum: *df,
                    })
                    .collect();
                if let Ok(mut store) = self.type32.lock() {
                    store.add(&open);
                }
                self.events.push(FrontEvent::Slice(SliceEvent {
                    slice_id: open.slice_id,
                    height: open.height,
                    prev_hash: open.prev_hash,
                    ref_job_id: open.ref_job_id,
                    ref_fees: open.ref_fees,
                    n_shares: open.shares.len() as u32,
                    difficulty: open.difficulty,
                    root: open.root,
                    shares_blob: serde_json::to_vec(
                        &open
                            .shares
                            .iter()
                            .map(|s| spc_share_ext::ShareLeaf {
                                nonce: s.nonce,
                                ntime: s.ntime,
                                version: s.version,
                                extranonce: s.extranonce.clone(),
                                job_id: s.job_id,
                                share_index: s.share_index,
                            })
                            .collect::<Vec<_>>(),
                    )
                    .unwrap_or_default(),
                    credits,
                }));
                self.slices.insert(0, open.agg());
                self.slices.truncate(self.cfg.keep_slices.max(1));
            }
        }
    }

    /// Open a fresh slice for `height`, assigning the next restart-safe id and
    /// recording the job it was opened under as `(H << 32) | mining_job_id`
    /// (SPEC §3.4).
    fn open_slice(&mut self, height: u32, prev_hash: [u8; 32], fees: u64) {
        let slice_id = self.next_slice_id();
        let ref_job_id = (u64::from(height) << 32) | u64::from(self.job_seq);
        self.open = Some(Slice::new(
            slice_id,
            height,
            prev_hash,
            self.cfg.front_id,
            ref_job_id,
            fees,
        ));
    }

    /// Next slice id: `front_id << 56 | local_seq`, where
    /// `local_seq = max(last + 1, unix_ms)` masked to 56 bits (MULTI-REGION
    /// §10, SPEC §5.1). Monotonic across restarts because the clock only moves
    /// forward and `apply_bootstrap` seeds the sequence above the tail.
    fn next_slice_id(&mut self) -> u64 {
        let n = self.slice_seq.saturating_add(1).max(unix_ms()) & LOCAL_SEQ_MASK;
        self.slice_seq = n;
        (u64::from(self.cfg.front_id) << 56) | n
    }

    pub fn coinbase_template(&self) -> Option<CoinbaseTemplate> {
        let _ = &self.template;
        None
    }

    /// Route helper for callers that need the canonical values.
    pub fn route_for_test(&self, raw: &spc_accounting::RawPayouts, total: u64) -> Routing {
        let tier = TierConfig {
            fee_bps: self.cfg.fee_bps,
            dust_limit_sats: self.cfg.dust_limit_sats,
            coinbase_threshold_sats: self.cfg.coinbase_threshold_sats,
            max_payout_slots: self.cfg.max_payout_slots,
            budget_wu: self.cfg.budget_wu,
            overhead_wu: self.cfg.overhead_wu,
        };
        route_tiers(raw, &self.pending, &self.ark, &tier, total).expect("valid tier config")
    }
}

/// Apply payout-assigned pending deltas. A balance that reaches zero or below
/// is removed, never stored as a negative or wrapped value.
fn apply_pending_deltas(pending: &mut BTreeMap<SpId, u64>, deltas: &BTreeMap<SpId, i64>) {
    for (id, delta) in deltas {
        let base = i128::from(pending.get(id).copied().unwrap_or(0));
        let next = base + i128::from(*delta);
        if next > 0 {
            pending.insert(*id, u64::try_from(next).unwrap_or(u64::MAX));
        } else {
            pending.remove(id);
        }
    }
}

/// Move kept balances whose `P_i` cannot be derived (keys unknown, e.g. after
/// a restart restored the window but the miner has not reconnected) onto the
/// `pending` rail, keeping the coinbase whole: the sats stay in the pool output
/// and the balance is owed from a later coinbase.
fn hold_undeliverable(
    routing: &mut Routing,
    miners: &BTreeMap<SpId, MinerKeys>,
    pending: &BTreeMap<SpId, u64>,
) {
    let mut held: Vec<(SpId, u64)> = Vec::new();
    let mut held_paid: u64 = 0;
    routing.kept.retain(|(id, paid)| {
        if miners.contains_key(id) {
            true
        } else {
            held_paid = held_paid.saturating_add(*paid);
            // The liability is the whole `owed`, not the coinbase-capped
            // amount: a partially paid id keeps its residual too (SPEC §4.6).
            let owed = routing.owed.get(id).copied().unwrap_or(*paid);
            held.push((*id, owed));
            false
        }
    });
    for (id, owed) in held {
        tracing::warn!(
            ?id,
            owed,
            "no keys for a kept identity; holding the full owed balance as pending"
        );
        let old = i64::try_from(pending.get(&id).copied().unwrap_or(0)).unwrap_or(i64::MAX);
        let owed_i64 = i64::try_from(owed).unwrap_or(i64::MAX);
        routing
            .pending_deltas
            .insert(id, owed_i64.saturating_sub(old));
        routing.pending.push((id, owed));
    }
    // The sats the coinbase no longer pays stay in the pool output.
    routing.pay_p = routing.pay_p.saturating_add(held_paid);
}

/// Milliseconds since the Unix epoch, saturating at `u64::MAX` for a clock
/// before 1970 (never panics).
fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// Rebuild one window slice from a bootstrap row (WIRE-FORMATS §3.5).
///
/// Returns `None` when the row cannot be replayed faithfully: no per-identity
/// credits (the window would silently shrink), an unparseable numeric, or a
/// stored difficulty that disagrees with the credits. Failing closed is the
/// only safe option: a window that does not match what was mined cannot be
/// reconstructed for settlement.
fn slice_agg_from_row(row: &SliceRow) -> Option<SliceAgg> {
    if row.credits.is_empty() {
        tracing::warn!(
            slice_id = row.slice_id,
            "bootstrap slice has no credits; not restored into the window"
        );
        return None;
    }
    let mut per_id: BTreeMap<SpId, (u128, u128)> = BTreeMap::new();
    let mut difficulty = 0u128;
    let mut diff_fee_sum = 0u128;
    for credit in &row.credits {
        let d: u128 = credit.diff_sum.parse().ok()?;
        let df: u128 = credit.diff_fee_sum.parse().ok()?;
        // A duplicate identity would double count the slice.
        if per_id.insert(credit.sp_id, (d, df)).is_some() {
            tracing::warn!(
                slice_id = row.slice_id,
                "bootstrap slice has duplicate credits; not restored"
            );
            return None;
        }
        difficulty = difficulty.saturating_add(d);
        diff_fee_sum = diff_fee_sum.saturating_add(df);
    }
    if let Ok(stored) = row.difficulty.parse::<u128>() {
        if stored != difficulty {
            tracing::warn!(
                slice_id = row.slice_id,
                stored,
                credits = difficulty,
                "bootstrap slice difficulty disagrees with its credits; not restored"
            );
            return None;
        }
    }
    Some(SliceAgg {
        slice_id: row.slice_id,
        height: row.height,
        prev_hash: row.prev_hash,
        front_id: slice_front_id(row.slice_id),
        difficulty,
        diff_fee_sum,
        per_id,
        root: row.root,
    })
}

fn parse_job_id(s: &str) -> u64 {
    u64::from_str_radix(s, 16).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template::MockTemplateSource;
    use spc_crypto::SendKey;

    fn template() -> Template {
        Template {
            height: 800_000,
            prev_hash: [0x11; 32],
            version: 0x2000_0000,
            bits: 0x207f_ffff, // easy regtest target
            curtime: 1_700_000_000,
            subsidy: 312_500_000,
            fees: 0,
            coinbase_value: 312_500_000,
            transactions: vec![],
            merkle_branches: vec![],
            coinbase_output_max_additional_size: 50_000,
        }
    }

    fn cfg() -> FrontConfig {
        FrontConfig {
            network: Network::Regtest,
            fee_bps: 100,
            window_n: 8,
            slice_fee_delta_sats: 0,
            max_slice_shares: 65_000,
            dust_limit_sats: 330,
            coinbase_threshold_sats: 330,
            max_payout_slots: 256,
            budget_wu: 100_000,
            overhead_wu: 0,
            pool_payout_descriptor: test_descriptor(),
            pool_payout_script: Some(
                crate::coinbase::p2tr_script(&[0x99; 32])
                    .to_bytes()
                    .to_vec(),
            ),
            extranonce_len: 8,
            front_id: 0,
            keep_slices: 16,
        }
    }

    /// A parsed `[pool] pool_payout_descriptor` for the in-process tests:
    /// a real watch-only testnet-family key, fail-closed parsed.
    fn test_descriptor() -> crate::coinbase::PoolPayoutDescriptor {
        let secp = bitcoin::secp256k1::Secp256k1::new();
        let xprv =
            bitcoin::bip32::Xpriv::new_master(bitcoin::Network::Testnet, &[0x5c; 32]).unwrap();
        let xpub = bitcoin::bip32::Xpub::from_priv(&secp, &xprv);
        crate::coinbase::PoolPayoutDescriptor::parse(&format!("tr({xpub}/0/*)"), Network::Regtest)
            .unwrap()
    }

    /// SPEC §6.3/§11.3: with no dev override, the coinbase pool output is
    /// derived from the descriptor at the template's height, so two
    /// heights build different pool outputs and neither is ever a zero
    /// key.
    #[tokio::test]
    async fn the_pool_output_is_derived_per_height() {
        let source = Arc::new(MutableSource::new(template()));
        let mut cfg = cfg();
        cfg.pool_payout_script = None;
        let mut front = Front::new(
            cfg,
            source.clone(),
            std::sync::Arc::new(std::sync::Mutex::new(crate::type32::Type32Store::default())),
        );
        front.refresh().await.unwrap();
        let d1 = front.distribution().unwrap().clone();
        let pool_1 = d1.outputs.first().cloned().unwrap().0;
        assert_eq!(pool_1.len(), 34);
        assert_ne!(&pool_1[2..], &[0u8; 32], "never a zero-key output");

        let mut t2 = template();
        t2.height = 800_001;
        source.set(t2);
        front.refresh().await.unwrap();
        let d2 = front.distribution().unwrap().clone();
        let pool_2 = d2.outputs.first().cloned().unwrap().0;
        assert_ne!(pool_1, pool_2, "each height derives a fresh pool output");

        // The built script equals the descriptor's own derivation at H.
        assert_eq!(
            pool_1,
            test_descriptor()
                .script_at(800_000)
                .unwrap()
                .as_bytes()
                .to_vec()
        );
    }

    /// The explicit dev/test override wins verbatim and no derivation
    /// happens (the e2e harnesses rely on this).
    #[tokio::test]
    async fn the_dev_override_replaces_derivation() {
        let source = Arc::new(MockTemplateSource::new(template()));
        let mut front = Front::new(
            cfg(),
            source,
            std::sync::Arc::new(std::sync::Mutex::new(crate::type32::Type32Store::default())),
        );
        front.refresh().await.unwrap();
        let pool = front
            .distribution()
            .unwrap()
            .outputs
            .first()
            .cloned()
            .unwrap()
            .0;
        assert_eq!(
            pool,
            crate::coinbase::p2tr_script(&[0x99; 32])
                .to_bytes()
                .to_vec()
        );
    }

    /// `FrontConfig::from_config` carries the parsed descriptor and rejects
    /// a bad one (and a non-P2TR override) fail-closed.
    #[test]
    fn from_config_parses_the_descriptor_fail_closed() {
        let toml_text = r#"
[pool]
network = "regtest"
listen = "127.0.0.1:34254"
authority_public_key = "02aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
pool_payout_descriptor = "DESCRIPTOR"
"#;
        let secp = bitcoin::secp256k1::Secp256k1::new();
        let xprv =
            bitcoin::bip32::Xpriv::new_master(bitcoin::Network::Testnet, &[0x5c; 32]).unwrap();
        let xpub = bitcoin::bip32::Xpub::from_priv(&secp, &xprv);

        // Valid: the parsed descriptor is carried, no override, no
        // zero-key fallback.
        let text = toml_text.replace("DESCRIPTOR", &format!("tr({xpub}/0/*)"));
        let cfg: crate::config::Config = toml::from_str(&text).unwrap();
        let front = FrontConfig::from_config(&cfg).unwrap();
        assert!(front.pool_payout_script.is_none());
        let s = front.pool_script_at(800_000).unwrap();
        assert_eq!(s.len(), 34);
        assert_ne!(&s[2..], &[0u8; 32]);

        // Malformed descriptor (hardened step): fail closed.
        let bad = toml_text.replace("DESCRIPTOR", "tr(tpubD6NzVbkrYhZ4YTest/0'/*)");
        let cfg: crate::config::Config = toml::from_str(&bad).unwrap();
        assert!(FrontConfig::from_config(&cfg).is_err());

        // Non-P2TR dev override: fail closed.
        let with_override = format!(
            "{text}pool_payout_script = \"76a914000000000000000000000000000000000000000088ac\"\n"
        );
        let cfg: crate::config::Config = toml::from_str(&with_override).unwrap();
        assert!(FrontConfig::from_config(&cfg).is_err());
    }

    #[tokio::test]
    async fn refresh_builds_a_job_with_the_carrier() {
        let source = Arc::new(MockTemplateSource::new(template()));
        let mut front = Front::new(
            cfg(),
            source,
            std::sync::Arc::new(std::sync::Mutex::new(crate::type32::Type32Store::default())),
        );
        let job = front.refresh().await.unwrap();
        assert_eq!(job.height, 800_000);
        // coinb1 ends with the 32-byte key; coinb2 starts with the sequence.
        assert!(job.coinb1.len() > 33);
        assert!(job.coinb2.len() >= 4);
        let d = front.distribution().unwrap();
        assert_eq!(d.height, 800_000);
        assert_eq!(d.outputs.len(), 1); // pool only before any shares
    }

    #[tokio::test]
    async fn a_share_rolls_into_the_slice_and_can_find_a_block() {
        let source = Arc::new(MockTemplateSource::new(template()));
        let mut front = Front::new(
            cfg(),
            source.clone(),
            std::sync::Arc::new(std::sync::Mutex::new(crate::type32::Type32Store::default())),
        );
        front.refresh().await.unwrap();

        // A valid identity derived from a scan/spend key pair.
        let secp = bitcoin::secp256k1::Secp256k1::new();
        let scan_sk = bitcoin::secp256k1::SecretKey::from_slice(&[0x11; 32]).unwrap();
        let spend_sk = bitcoin::secp256k1::SecretKey::from_slice(&[0x12; 32]).unwrap();
        let scan = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &scan_sk);
        let spend = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &spend_sk);
        let id = spc_crypto::sp_id(&scan, &spend);
        let send = SendKey::generate(800_000).unwrap();
        let _ = send;

        let share = crate::sv1::Share {
            sp_id: id,
            b_scan: scan.serialize(),
            b_spend: spend.serialize(),
            channel_id: 1,
            job_id: front.job().unwrap().job_id.clone(),
            nonce: 1,
            ntime: 1_700_000_000,
            version: 0x2000_0000,
            extranonce: vec![0xaa; 8],
            share_index: 0,
            job_fees: 0,
            coinbase: front.job().unwrap().coinb1.clone(),
            merkle_root: [0u8; 32],
            share_hash: [0u8; 32], // easy target is met
        };
        let block = front.on_share(share).unwrap().expect("block");
        assert_eq!(block.header.len(), 80);
        assert_eq!(front.sealed_slices().len(), 0);
    }

    /// Template with explicit bits and fees for the accounting tests.
    fn template_with(bits: u32, fees: u64) -> Template {
        template_at(800_000, bits, fees)
    }

    /// Template at an explicit height, used to roll the open slice.
    fn template_at(height: u32, bits: u32, fees: u64) -> Template {
        Template {
            height,
            prev_hash: [0x11; 32],
            version: 0x2000_0000,
            bits,
            curtime: 1_700_000_000,
            subsidy: 312_500_000,
            fees,
            coinbase_value: 312_500_000 + fees,
            transactions: vec![],
            merkle_branches: vec![],
            coinbase_output_max_additional_size: 50_000,
        }
    }

    fn type32() -> std::sync::Arc<std::sync::Mutex<crate::type32::Type32Store>> {
        std::sync::Arc::new(std::sync::Mutex::new(crate::type32::Type32Store::default()))
    }

    /// Identity for the scan/spend key pair derived from `seed`.
    fn id_for(seed: u8) -> [u8; 32] {
        let secp = bitcoin::secp256k1::Secp256k1::new();
        let scan = bitcoin::secp256k1::SecretKey::from_slice(&[seed; 32]).unwrap();
        let spend = bitcoin::secp256k1::SecretKey::from_slice(&[seed.wrapping_add(1); 32]).unwrap();
        spc_crypto::sp_id(
            &bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &scan),
            &bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &spend),
        )
    }

    /// `(B_scan, B_spend)` for the identity derived from `seed`.
    fn key_pair(seed: u8) -> ([u8; 33], [u8; 33]) {
        let secp = bitcoin::secp256k1::Secp256k1::new();
        let scan = bitcoin::secp256k1::SecretKey::from_slice(&[seed; 32]).unwrap();
        let spend = bitcoin::secp256k1::SecretKey::from_slice(&[seed.wrapping_add(1); 32]).unwrap();
        (
            bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &scan).serialize(),
            bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &spend).serialize(),
        )
    }

    /// A source whose template the test can change between refreshes.
    struct MutableSource {
        t: std::sync::Mutex<Template>,
    }

    impl MutableSource {
        fn new(t: Template) -> Self {
            Self {
                t: std::sync::Mutex::new(t),
            }
        }
        fn set(&self, t: Template) {
            *self.t.lock().expect("poisoned") = t;
        }
    }

    #[async_trait::async_trait]
    impl TemplateSource for MutableSource {
        async fn get_template(&self) -> Result<Template, TemplateError> {
            Ok(self.t.lock().expect("poisoned").clone())
        }
        async fn submit_block(&self, _block: &[u8]) -> Result<(), TemplateError> {
            Ok(())
        }
    }

    /// Build a share against the front's current job.
    ///
    /// `share_hash` is `2^216`: it meets the difficulty-1 pool share target
    /// (`0x00000000ffff0000...`) but not a network target 256x harder
    /// (`bits = 0x1c00ffff`), so no block is assembled.
    fn share(front: &Front, nonce: u32) -> crate::sv1::Share {
        let secp = bitcoin::secp256k1::Secp256k1::new();
        let scan_sk = bitcoin::secp256k1::SecretKey::from_slice(&[0x11; 32]).unwrap();
        let spend_sk = bitcoin::secp256k1::SecretKey::from_slice(&[0x12; 32]).unwrap();
        let scan = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &scan_sk);
        let spend = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &spend_sk);
        crate::sv1::Share {
            sp_id: spc_crypto::sp_id(&scan, &spend),
            b_scan: scan.serialize(),
            b_spend: spend.serialize(),
            channel_id: 1,
            job_id: front.job().unwrap().job_id.clone(),
            nonce,
            ntime: 1_700_000_000,
            version: 0x2000_0000,
            extranonce: vec![0xaa; 8],
            share_index: 0,
            // The listeners leave this at 0 for pool-template mode; the front
            // substitutes the template's fees (REVIEW 8.5/8.6).
            job_fees: 0,
            coinbase: front.job().unwrap().coinb1.clone(),
            merkle_root: [0u8; 32],
            share_hash: diff1_share_hash(),
        }
    }

    /// `2^216`, which meets the difficulty-1 target but not a target 256x
    /// harder.
    fn diff1_share_hash() -> [u8; 32] {
        let mut hash = [0u8; 32];
        hash[4] = 0x01;
        hash
    }

    /// A share from a different identity so two ids can be owed at once.
    fn share_for(front: &Front, seed: u8, nonce: u32) -> crate::sv1::Share {
        let secp = bitcoin::secp256k1::Secp256k1::new();
        let scan_sk = bitcoin::secp256k1::SecretKey::from_slice(&[seed; 32]).unwrap();
        let spend_sk =
            bitcoin::secp256k1::SecretKey::from_slice(&[seed.wrapping_add(1); 32]).unwrap();
        let scan = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &scan_sk);
        let spend = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &spend_sk);
        crate::sv1::Share {
            sp_id: spc_crypto::sp_id(&scan, &spend),
            b_scan: scan.serialize(),
            b_spend: spend.serialize(),
            channel_id: 1,
            job_id: front.job().unwrap().job_id.clone(),
            nonce,
            ntime: 1_700_000_000,
            version: 0x2000_0000,
            extranonce: vec![0xbb; 8],
            share_index: nonce,
            job_fees: 0,
            coinbase: front.job().unwrap().coinb1.clone(),
            merkle_root: [0u8; 32],
            share_hash: diff1_share_hash(),
        }
    }

    fn cfg_with(mut c: FrontConfig, f: impl Fn(&mut FrontConfig)) -> FrontConfig {
        f(&mut c);
        c
    }

    fn slice_events(front: &mut Front) -> Vec<SliceEvent> {
        front
            .take_events()
            .into_iter()
            .filter_map(|e| match e {
                FrontEvent::Slice(s) => Some(s),
                _ => None,
            })
            .collect()
    }

    fn block_events(front: &mut Front) -> Vec<BlockEvent> {
        front
            .take_events()
            .into_iter()
            .filter_map(|e| match e {
                FrontEvent::Block(b) => Some(b),
                _ => None,
            })
            .collect()
    }

    /// One bootstrap window row with the credits that produced it.
    fn slice_row(slice_id: u64, height: u32, credits: &[([u8; 32], u128, u128)]) -> SliceRow {
        let difficulty: u128 = credits.iter().map(|(_, d, _)| *d).sum();
        SliceRow {
            slice_id,
            height,
            prev_hash: [0x11; 32],
            ref_job_id: u64::from(height),
            ref_fees: 0,
            n_shares: u32::try_from(credits.len()).unwrap(),
            difficulty: difficulty.to_string(),
            root: [0xaa; 32],
            shares: Vec::new(),
            credits: credits
                .iter()
                .map(|(id, d, df)| spc_store::models::SliceCreditRow {
                    sp_id: *id,
                    diff_sum: d.to_string(),
                    diff_fee_sum: df.to_string(),
                })
                .collect(),
        }
    }

    /// SPEC §4.3: the mark is the newest *sealed* slice the window consumed,
    /// never the still-open slice the finding share lands in.
    #[tokio::test]
    async fn block_events_use_the_newest_sealed_slice_as_high_water() {
        let source = Arc::new(MutableSource::new(template_at(800_000, 0x207f_ffff, 0)));
        let cfg = cfg_with(cfg(), |c| c.window_n = 1);
        let mut front = Front::new(cfg, source.clone(), type32());
        front.refresh().await.unwrap();

        // No sealed slice yet: the mark is 0, so settlement selects nothing.
        let mut first = share(&front, 1);
        first.share_hash = [0u8; 32]; // meets the regtest network target
        front.on_share(first).unwrap().expect("block");
        let events = block_events(&mut front);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].high_water[&0], "0");
        assert_eq!(front.high_water(), 0);

        // New height seals the slice holding that share; the window now holds
        // it, so the mark is its id, not the open slice's.
        source.set(template_at(800_001, 0x207f_ffff, 0));
        front.refresh().await.unwrap();
        let sealed_id = front.sealed_slices()[0].slice_id;
        assert_eq!(front.high_water(), sealed_id);
        let open_id = (u64::from(front.cfg.front_id) << 56) | front.local_slice_seq();
        assert_ne!(front.high_water(), open_id);

        let mut second = share(&front, 2);
        second.share_hash = [0u8; 32];
        front.on_share(second).unwrap().expect("block");
        let events = block_events(&mut front);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].high_water[&0], sealed_id.to_string());
    }

    /// SPEC §4.3 / decision 2026-11-03-settlement-window-target: the block
    /// event carries the exact `N x d` the front selected the window with, so
    /// settlement can freeze it instead of re-deriving it from slices.
    #[tokio::test]
    async fn block_events_carry_the_exact_window_target() {
        // bits 0x1c00ffff is difficulty 256 against the difficulty-1 target;
        // with share_difficulty 1000 the credited `d` caps at 256.
        let source = Arc::new(MutableSource::new(template_at(800_000, 0x1c00_ffff, 0)));
        let cfg = cfg_with(cfg(), |c| c.window_n = 2);
        let mut front = Front::new(cfg, source, type32());
        front.set_share_difficulty(1000).unwrap();
        front.refresh().await.unwrap();

        let mut finding = share(&front, 1);
        finding.share_hash = [0u8; 32]; // meets the network target
        front.on_share(finding).unwrap().expect("block");
        let events = block_events(&mut front);
        assert_eq!(events.len(), 1);
        // N x d = 2 x 256: the value settlement will persist, not an
        // estimate from whatever the stored slices imply later.
        assert_eq!(events[0].window_target, 512);
    }

    #[tokio::test]
    async fn slice_ids_encode_the_front_and_are_monotonic() {
        let source = Arc::new(MutableSource::new(template_at(800_000, 0x1c00_ffff, 0)));
        let cfg = cfg_with(cfg(), |c| {
            c.front_id = 3;
            c.max_slice_shares = 1;
        });
        let mut front = Front::new(cfg, source, type32());
        front.refresh().await.unwrap();

        front.on_share(share(&front, 1)).unwrap();
        front.on_share(share(&front, 2)).unwrap(); // seals the first slice
        let events = slice_events(&mut front);
        assert_eq!(events.len(), 1);
        let first = events[0].slice_id;
        assert_eq!(first >> 56, 3);
        assert_ne!(first & LOCAL_SEQ_MASK, 0);

        front.on_share(share(&front, 3)).unwrap();
        front.on_share(share(&front, 4)).unwrap();
        let events = slice_events(&mut front);
        assert_eq!(events.len(), 2);
        let newest = events.iter().map(|e| e.slice_id).max().unwrap();
        assert_eq!(newest >> 56, 3);
        assert!(newest > first);
        assert!(events.iter().all(|e| e.slice_id > first));
    }

    /// SPEC §5.3 / WIRE-FORMATS §3.5: a restarted front rebuilds its window
    /// from the bootstrap tail and pays it in the next distribution.
    #[tokio::test]
    async fn bootstrap_restores_the_window_into_the_next_distribution() {
        let source = Arc::new(MockTemplateSource::new(template_at(
            800_000,
            0x1c00_ffff,
            0,
        )));
        let cfg = cfg_with(cfg(), |c| c.window_n = 2);
        let mut front = Front::new(cfg, source, type32());
        let miner = id_for(0x11);
        let other = id_for(0x31);

        // Two sealed slices at difficulty 1 each, one identity apiece, plus a
        // row with no credits that must be skipped rather than zero-filled.
        let snapshot = BootstrapSnapshot {
            rev: 3,
            applied_blocks: vec![],
            pending_balance: vec![],
            ark_delivery: vec![],
            window_tail: vec![
                slice_row(9_000, 799_999, &[(miner, 1, 0)]),
                slice_row(8_000, 799_998, &[(other, 1, 0)]),
                slice_row(7_000, 799_997, &[]),
            ],
            type32_blobs: vec![],
            slices_cursor: None,
            front_events_cursor: None,
        };
        front.apply_bootstrap(&snapshot);

        // Newest first, truncated, credits parsed, empty row skipped.
        let ids: Vec<u64> = front.sealed_slices().iter().map(|s| s.slice_id).collect();
        assert_eq!(ids, vec![9_000, 8_000]);
        assert_eq!(front.high_water(), 9_000);
        assert_eq!(front.local_slice_seq(), 9_000);

        // The restarting front has no keys for the restored identities yet, so
        // the balances are held as pending instead of being paid to a guessed
        // output.
        let job = front.refresh().await.unwrap();
        assert_eq!(job.height, 800_000);
        let dist = front.distribution().unwrap();
        assert!(dist
            .entries
            .iter()
            .all(|e| e.rail == spc_accounting::Rail::Pending));
        // `pending` only moves when a block is found, so the held rail is
        // visible in the distribution, not in the ledger yet.
        assert!(front.pending().is_empty());

        // Once the miner's keys are known (next share or a registration that
        // carries them) the restored window is paid from the coinbase.
        front
            .learn_identity(miner, key_pair(0x11).0, key_pair(0x11).1)
            .unwrap();
        front
            .learn_identity(other, key_pair(0x31).0, key_pair(0x31).1)
            .unwrap();
        front.refresh().await.unwrap();
        let dist = front.distribution().unwrap();
        let entry = dist
            .entries
            .iter()
            .find(|e| e.sp_id == miner && e.rail == spc_accounting::Rail::Coinbase)
            .expect("restored window pays the miner");
        assert!(entry.sats > 0);
        assert!(entry.p_x.is_some());
    }

    /// A legacy (witness-free) transaction the merkle tests can put in a
    /// template.
    fn raw_legacy_tx(seed: u8) -> Vec<u8> {
        use bitcoin::absolute::LockTime;
        use bitcoin::transaction::Version;
        use bitcoin::{Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness};
        let tx = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint {
                    txid: bitcoin::Txid::from_byte_array([seed; 32]),
                    vout: u32::from(seed),
                },
                script_sig: ScriptBuf::new(),
                sequence: Sequence::MAX,
                witness: Witness::default(),
            }],
            output: vec![TxOut {
                value: Amount::from_sat(u64::from(seed) + 1),
                script_pubkey: ScriptBuf::new(),
            }],
        };
        bitcoin::consensus::serialize(&tx)
    }

    /// A transaction with a witness stack: needs a coinbase commitment this
    /// pool does not build, so templates carrying one are refused.
    fn raw_witness_tx(seed: u8) -> Vec<u8> {
        use bitcoin::absolute::LockTime;
        use bitcoin::transaction::Version;
        use bitcoin::{Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness};
        let tx = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint {
                    txid: bitcoin::Txid::from_byte_array([seed; 32]),
                    vout: u32::from(seed),
                },
                script_sig: ScriptBuf::new(),
                sequence: Sequence::MAX,
                witness: Witness::from_slice(&[vec![0u8; 4]]),
            }],
            output: vec![TxOut {
                value: Amount::from_sat(1),
                script_pubkey: ScriptBuf::new(),
            }],
        };
        bitcoin::consensus::serialize(&tx)
    }

    /// Miner-side merkle fold: coinbase txid, then the branch bottom-up.
    fn miner_merkle_root(coinbase: &[u8], branch: &[[u8; 32]]) -> [u8; 32] {
        let mut root = crate::block::txid(coinbase);
        for sibling in branch.iter().rev() {
            let mut buf = [0u8; 64];
            buf[..32].copy_from_slice(&root);
            buf[32..].copy_from_slice(sibling);
            root = crate::block::txid(&buf);
        }
        root
    }

    fn coinbase_of(job: &Job, extranonce: &[u8]) -> Vec<u8> {
        let mut coinbase = job.coinb1.clone();
        coinbase.extend_from_slice(extranonce);
        coinbase.extend_from_slice(&job.coinb2);
        coinbase
    }

    /// SPEC §3.4: the same share submitted twice is credited once.
    #[tokio::test]
    async fn a_replayed_share_is_credited_once() {
        let source = Arc::new(MockTemplateSource::new(template_at(
            800_000,
            0x1c00_ffff,
            0,
        )));
        let cfg = cfg_with(cfg(), |c| c.max_slice_shares = 1);
        let mut front = Front::new(cfg, source, type32());
        front.refresh().await.unwrap();

        let first = share(&front, 1);
        front.on_share(first.clone()).unwrap();
        assert!(matches!(
            front.on_share(first),
            Err(FrontError::DuplicateShare)
        ));

        // Only one share reached the slice: the next distinct share seals a
        // one-share slice.
        front.on_share(share(&front, 2)).unwrap();
        let events = slice_events(&mut front);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].n_shares, 1);
        assert_eq!(
            events[0].credits.iter().map(|c| c.diff_sum).sum::<u128>(),
            1
        );
    }

    /// A forecast refresh must not move money: two refreshes at one height
    /// route against the same frozen `pending`.
    #[tokio::test]
    async fn two_refreshes_at_one_height_keep_the_same_distribution() {
        let bits = 0x1c00_ffff;
        let source = Arc::new(MutableSource::new(template_at(800_000, bits, 0)));
        let cfg = cfg_with(cfg(), |c| {
            c.fee_bps = 0;
            c.window_n = 1;
            c.max_payout_slots = 1; // one trimmed id: the double-count vector
        });
        let mut front = Front::new(cfg, source.clone(), type32());
        front.refresh().await.unwrap();
        front.on_share(share_for(&front, 0x21, 1)).unwrap();
        front.on_share(share_for(&front, 0x31, 2)).unwrap();

        source.set(template_at(800_001, bits, 0));
        front.refresh().await.unwrap();
        let first = front.distribution().unwrap().clone();
        assert!(front.pending().is_empty());

        // Same height, same template: the distribution must be byte-identical,
        // not "raw + pending" again.
        front.refresh().await.unwrap();
        let second = front.distribution().unwrap().clone();
        assert_eq!(first.fingerprint, second.fingerprint);
        assert_eq!(first.outputs, second.outputs);
        assert_eq!(first.entries, second.entries);
        assert!(front.pending().is_empty());
    }

    /// Heights advance with no found block: no liability may appear.
    #[tokio::test]
    async fn pending_does_not_grow_across_heights_without_a_block() {
        let bits = 0x1c00_ffff;
        let source = Arc::new(MutableSource::new(template_at(800_000, bits, 0)));
        let cfg = cfg_with(cfg(), |c| {
            c.fee_bps = 0;
            c.window_n = 1;
            c.max_payout_slots = 1;
        });
        let mut front = Front::new(cfg, source.clone(), type32());
        front.refresh().await.unwrap();
        front.on_share(share_for(&front, 0x21, 1)).unwrap();
        front.on_share(share_for(&front, 0x31, 2)).unwrap();

        let mut kept_amounts = Vec::new();
        for height in 800_001..=800_003 {
            source.set(template_at(height, bits, 0));
            front.refresh().await.unwrap();
            assert!(
                front.pending().is_empty(),
                "no block found at {height}, so no pending"
            );
            let dist = front.distribution().unwrap();
            let kept: Vec<u64> = dist
                .entries
                .iter()
                .filter(|e| e.rail == spc_accounting::Rail::Coinbase)
                .map(|e| e.sats)
                .collect();
            assert_eq!(kept.len(), 1);
            kept_amounts.push(kept[0]);
        }
        // The window is the same two-share slice every time: one kept id gets
        // `r_net / d_win = r_net / 2`, unchanged by the passing heights.
        assert_eq!(kept_amounts[0], 312_500_000 / 2);
        assert!(kept_amounts.windows(2).all(|w| w[0] == w[1]));
    }

    /// A found block creates no liability: `pending` moves only when the payout
    /// role publishes `LedgerApplied` after confirmation, so an orphan leaves
    /// nothing behind.
    #[tokio::test]
    async fn a_found_block_creates_no_pending_until_the_ledger_says_so() {
        let bits = 0x1c00_ffff;
        let source = Arc::new(MutableSource::new(template_at(800_000, bits, 0)));
        let cfg = cfg_with(cfg(), |c| {
            c.fee_bps = 0;
            c.window_n = 1;
            c.max_payout_slots = 1;
        });
        let mut front = Front::new(cfg, source.clone(), type32());
        front.refresh().await.unwrap();
        front.on_share(share_for(&front, 0x21, 1)).unwrap();
        front.on_share(share_for(&front, 0x31, 2)).unwrap();
        source.set(template_at(800_001, bits, 0));
        front.refresh().await.unwrap();
        assert!(front.pending().is_empty());

        // The routing knows one id is trimmed, but the front does not book it.
        let routing = front.routing().expect("routing").clone();
        assert_eq!(routing.pending.len(), 1);
        let trimmed = routing.pending[0];

        let mut block = share_for(&front, 0x21, 3);
        block.share_hash = [0u8; 32];
        front.on_share(block).unwrap().expect("block");
        assert!(
            front.pending().is_empty(),
            "found-but-unconfirmed block must not create pending"
        );

        // The payout role books it: its delta applies, including for our own
        // block (no local advance to duplicate).
        let deltas: Vec<([u8; 32], i64)> = routing
            .pending_deltas
            .iter()
            .map(|(id, delta)| (*id, *delta))
            .collect();
        assert!(front.apply_front_event(&WireFrontEvent::LedgerApplied {
            rev: 11,
            block_hash: [0x33; 32],
            republish: true,
            pending_deltas: deltas,
        }));
        assert_eq!(front.pending().get(&trimmed.0), Some(&trimmed.1));
    }

    /// SPEC §3.3/§3.4: the job's merkle path must rebuild the block root from
    /// the miner's coinbase, and the block must carry the submitted version.
    #[tokio::test]
    async fn merkle_path_rebuilds_the_block_and_uses_the_submitted_version() {
        let mut t = template_at(800_000, 0x207f_ffff, 0);
        t.transactions = vec![raw_legacy_tx(1), raw_legacy_tx(2)];
        let source = Arc::new(MockTemplateSource::new(t.clone()));
        let mut front = Front::new(cfg(), source, type32());
        let job = front.refresh().await.unwrap();
        // 3 leaves (coinbase + 2 txs) give a 2-level branch.
        assert_eq!(job.merkle_branches.len(), 2);

        let rolled = job.version ^ 0x0000_2000; // legal BIP 320 roll
        let mut share = share(&front, 1);
        share.version = rolled;
        share.share_hash = [0u8; 32]; // meets the network target
        let coinbase = coinbase_of(&job, &share.extranonce);
        let miner_root = miner_merkle_root(&coinbase, &job.merkle_branches);
        share.coinbase = coinbase.clone();
        share.merkle_root = miner_root;

        let block = front.on_share(share).unwrap().expect("block");
        // The assembled block carries the miner's root and version.
        assert_eq!(block.merkle_root, miner_root);
        assert_eq!(&block.header[..4], &rolled.to_le_bytes());
        let expected = {
            let mut leaves = vec![crate::block::txid(&coinbase)];
            leaves.extend(t.transactions.iter().map(|tx| crate::block::txid(tx)));
            crate::block::merkle_root(&leaves)
        };
        assert_eq!(block.merkle_root, expected);
    }

    /// A version rolled outside the BIP 320 mask is not the template.
    #[tokio::test]
    async fn a_version_outside_the_mask_is_rejected() {
        let source = Arc::new(MockTemplateSource::new(template()));
        let mut front = Front::new(cfg(), source, type32());
        front.refresh().await.unwrap();
        let mut bad = share(&front, 1);
        bad.version ^= 0x0000_0001;
        assert!(matches!(
            front.on_share(bad),
            Err(FrontError::InvalidVersion)
        ));
    }

    /// SPEC §3.3/§4.2: a same-height reorg seals the old fork's slice, issues a
    /// new job id and rejects work on the old head.
    #[tokio::test]
    async fn a_same_height_reorg_seals_the_slice_and_rejects_stale_work() {
        let bits = 0x1c00_ffff;
        let source = Arc::new(MutableSource::new(template_at(800_000, bits, 0)));
        let mut front = Front::new(cfg(), source.clone(), type32());
        front.refresh().await.unwrap();
        front.on_share(share(&front, 1)).unwrap();
        let old_job = front.job().unwrap().job_id.clone();

        let mut reorged = template_at(800_000, bits, 0);
        reorged.prev_hash = [0x22; 32];
        source.set(reorged);
        front.refresh().await.unwrap();

        // The slice mined on the old head was sealed, not merged into the new
        // one.
        let events = slice_events(&mut front);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].prev_hash, [0x11; 32]);
        assert_eq!(events[0].n_shares, 1);

        // New job id for the new head; work on the old one is stale.
        let new_job = front.job().unwrap().job_id.clone();
        assert_ne!(new_job, old_job);
        let mut stale = share(&front, 2);
        stale.job_id = old_job;
        assert!(matches!(front.on_share(stale), Err(FrontError::StaleJob)));

        // Work on the new head is accepted into the new slice.
        front.on_share(share(&front, 3)).unwrap();
        assert_eq!(front.sealed_slices().len(), 1);
    }

    /// Witness templates are refused rather than mined without a BIP 141
    /// commitment.
    #[tokio::test]
    async fn witness_templates_are_refused() {
        let mut t = template();
        t.transactions = vec![raw_witness_tx(9)];
        let mut front = Front::new(cfg(), Arc::new(MockTemplateSource::new(t)), type32());
        let err = front.refresh().await.unwrap_err();
        assert!(err.to_string().contains("witness"), "{err}");
        assert!(front.job().is_none());
    }

    #[tokio::test]
    async fn jd_distribution_is_built_from_weights_and_p_x() {
        let fees = 0;
        let source = Arc::new(MutableSource::new(template_at(800_000, 0x1c00_ffff, fees)));
        let cfg = cfg_with(cfg(), |c| {
            c.fee_bps = 0;
            c.window_n = 1;
        });
        let mut front = Front::new(cfg, source.clone(), type32());
        assert!(front.weights().is_none());
        front.refresh().await.unwrap();
        front.on_share(share(&front, 1)).unwrap();

        source.set(template_at(800_001, 0x1c00_ffff, fees));
        front.refresh().await.unwrap();

        let weights = front.weights().expect("weights after refresh").clone();
        assert_eq!(weights.pool_weight, 0); // no fees, single identity paid
        assert_eq!(weights.weights.len(), 1);
        assert_eq!(weights.weights[0].0, id_for(0x11));
        assert_eq!(weights.weights[0].1, 312_500_000);

        let dist = front.distribution().unwrap();
        let p_x_by_id: BTreeMap<[u8; 32], [u8; 32]> = dist
            .entries
            .iter()
            .filter_map(|e| e.p_x.map(|x| (e.sp_id, x)))
            .collect();
        let jd = crate::jds::distribution_from_weights(
            &weights,
            &p_x_by_id,
            front.pool_payout_script().unwrap(),
            &dist.a_send_x,
            330,
        )
        .unwrap();
        assert_eq!(
            jd.pool_payout.script,
            front.pool_payout_script().unwrap().to_vec()
        );
        assert_eq!(jd.pool_payout.value, weights.pool_weight);
        assert_eq!(jd.payouts.len(), 1);
        assert_eq!(jd.payouts[0].value, 312_500_000);
        assert_eq!(jd.additional_outputs.len(), 1);
        assert_eq!(jd.additional_outputs[0].script.first(), Some(&0x6a));
    }

    #[tokio::test]
    async fn share_credit_uses_pool_difficulty_capped_by_the_network() {
        let fees = 12_345;
        // Bits 0x1c00ffff put network difficulty at 256, so a pool difficulty
        // of 4 is fully creditable and the share target is the pool's.
        let source = Arc::new(MockTemplateSource::new(template_with(0x1c00_ffff, fees)));
        // Seal on the second share so the first share's record is observable.
        let cfg = cfg_with(cfg(), |c| c.max_slice_shares = 1);
        let mut front = Front::new(cfg, source, type32());
        front.set_share_difficulty(4).unwrap();
        front.refresh().await.unwrap();

        front.on_share(share(&front, 1)).unwrap();
        front.on_share(share(&front, 2)).unwrap();

        let events = slice_events(&mut front);
        assert_eq!(events.len(), 1);
        let sealed = &events[0];
        // The slice references the template it was opened from as
        // `(H << 32) | mining_job_id` (SPEC §3.4).
        assert_eq!(sealed.ref_job_id >> 32, 800_000);
        assert_eq!(
            sealed.ref_job_id & 0xffff_ffff,
            u64::from(u32::from_str_radix(front.job().unwrap().job_id.as_str(), 16).unwrap())
        );
        assert_eq!(sealed.ref_fees, fees);
        assert_eq!(sealed.n_shares, 1);
        let credit = sealed
            .credits
            .iter()
            .find(|c| c.sp_id == id_for(0x11))
            .expect("credit for the share's identity");
        // d is the pool share difficulty, not the network difficulty (REVIEW
        // 8.4) and not hardcoded to 1.
        assert_eq!(credit.diff_sum, 4);
        // job_fees comes from the template in pool-template mode (REVIEW 8.5).
        assert_eq!(credit.diff_fee_sum, 4 * fees as u128);
    }

    #[tokio::test]
    async fn regtest_share_credit_is_one_never_zero() {
        // Regtest-style bits: network difficulty rounds below 1, so the credit
        // is 1. A zero credit would leave the window empty forever.
        let source = Arc::new(MockTemplateSource::new(template_with(0x207f_ffff, 0)));
        let cfg = cfg_with(cfg(), |c| {
            c.max_slice_shares = 1;
            c.window_n = 1;
        });
        let mut front = Front::new(cfg, source, type32());
        front.set_share_difficulty(4_096).unwrap();
        front.refresh().await.unwrap();

        front.on_share(share(&front, 1)).unwrap();
        front.on_share(share(&front, 2)).unwrap();
        let events = slice_events(&mut front);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].credits[0].diff_sum, 1);
        assert_eq!(events[0].difficulty, 1);
    }

    #[tokio::test]
    async fn a_share_below_the_pool_target_is_rejected() {
        let source = Arc::new(MockTemplateSource::new(template()));
        let mut front = Front::new(cfg(), source, type32());
        front.refresh().await.unwrap();
        let mut bad = share(&front, 1);
        bad.share_hash = [0xff; 32];
        assert!(matches!(
            front.on_share(bad),
            Err(FrontError::LowDifficulty)
        ));
        assert_eq!(front.sealed_slices().len(), 0);
        // Zero is rejected outright rather than silently clamped.
        assert!(matches!(
            front.set_share_difficulty(0),
            Err(FrontError::ShareDifficulty)
        ));
    }

    #[tokio::test]
    async fn a_jd_share_carries_its_own_job_fees() {
        let fees = 12_345;
        let source = Arc::new(MockTemplateSource::new(template_with(0x1c00_ffff, fees)));
        let cfg = cfg_with(cfg(), |c| c.max_slice_shares = 1);
        let mut front = Front::new(cfg, source, type32());
        front.refresh().await.unwrap();

        // JD mode: the share declares 500 sats of job fees, which must win over
        // the template's 12 345 (SPEC §3.4, REVIEW 8.5).
        let mut jd = share(&front, 1);
        jd.job_fees = 500;
        front.on_share(jd).unwrap();
        front.on_share(share(&front, 2)).unwrap();

        let events = slice_events(&mut front);
        assert_eq!(events.len(), 1);
        let credit = events[0]
            .credits
            .iter()
            .find(|c| c.sp_id == id_for(0x11))
            .expect("credit");
        assert_eq!(credit.diff_fee_sum, 500);
    }

    /// SPEC §4.6: a balance trimmed from one coinbase is owed in the next
    /// distribution. The liability comes from the payout role's `LedgerApplied`
    /// delta, not from the front's forecast.
    #[tokio::test]
    async fn the_ledger_carries_a_trimmed_balance_into_the_next_distribution() {
        let fees = 0;
        let bits = 0x1c00_ffff; // network difficulty 256: shares are not blocks
        let source = Arc::new(MutableSource::new(template_at(800_000, bits, fees)));
        let cfg = cfg_with(cfg(), |c| {
            c.fee_bps = 0;
            c.window_n = 1;
            c.max_payout_slots = 1; // exactly one id can be paid per coinbase
            c.max_slice_shares = 65_000;
        });
        let mut front = Front::new(cfg, source.clone(), type32());
        front.refresh().await.unwrap();

        // Two identities, one share each at difficulty 1.
        front.on_share(share_for(&front, 0x21, 1)).unwrap();
        front.on_share(share_for(&front, 0x31, 2)).unwrap();

        // Next height seals the slice: r_net split two ways, one id paid, one
        // trimmed. No block has been found, so nothing is booked yet.
        source.set(template_at(800_001, bits, fees));
        front.refresh().await.unwrap();
        let raw_share = 312_500_000 / 2;
        assert!(front.pending().is_empty(), "a forecast books nothing");

        let routing = front.routing().expect("routing").clone();
        let trimmed = *routing
            .pending
            .first()
            .expect("one trimmed balance in the routing");
        assert_eq!(trimmed.1, raw_share);

        // The payout role books the block: the delta becomes the liability.
        let deltas: Vec<([u8; 32], i64)> = routing
            .pending_deltas
            .iter()
            .map(|(id, delta)| (*id, *delta))
            .collect();
        assert!(front.apply_front_event(&WireFrontEvent::LedgerApplied {
            rev: 1,
            block_hash: [0x55; 32],
            republish: true,
            pending_deltas: deltas,
        }));
        assert_eq!(front.pending().get(&trimmed.0), Some(&raw_share));

        // The next distribution owes the trimmed balance on top of its raw
        // share, and the largest owed id takes the single coinbase slot.
        source.set(template_at(800_002, bits, fees));
        front.refresh().await.unwrap();
        let dist = front.distribution().expect("distribution");
        let entry = dist
            .entries
            .iter()
            .find(|e| e.sp_id == trimmed.0)
            .expect("the trimmed id appears in the next distribution");
        assert_eq!(entry.rail, spc_accounting::Rail::Coinbase);
        assert_eq!(entry.sats, raw_share + raw_share);
        assert!(entry.sats > raw_share, "the pending balance was carried");
    }

    /// SPEC §3.4: a distribution-only change (a slice sealed, pending moved by
    /// the ledger) is a new coinbase, so the job id must move and work on the
    /// old job must be stale.
    #[tokio::test]
    async fn a_distribution_change_issues_a_new_job_id() {
        let bits = 0x1c00_ffff;
        let source = Arc::new(MutableSource::new(template_at(800_000, bits, 0)));
        let cfg = cfg_with(cfg(), |c| {
            c.fee_bps = 0;
            c.window_n = 1;
            c.max_payout_slots = 1;
            c.max_slice_shares = 1; // every other share seals a slice
        });
        let mut front = Front::new(cfg, source.clone(), type32());
        front.refresh().await.unwrap();
        let first = front.job().unwrap().job_id.clone();

        // An unchanged refresh keeps the id (no needless job churn).
        front.refresh().await.unwrap();
        assert_eq!(front.job().unwrap().job_id, first);

        // Sealing a slice changes the window, hence the coinbase.
        front.on_share(share_for(&front, 0x21, 1)).unwrap();
        front.on_share(share_for(&front, 0x31, 2)).unwrap(); // seals the first
        assert_eq!(front.sealed_slices().len(), 1);
        front.refresh().await.unwrap();
        let second = front.job().unwrap().job_id.clone();
        assert_ne!(second, first, "a new distribution needs a new job id");

        // Work on the old job is stale.
        let mut stale = share_for(&front, 0x21, 9);
        stale.job_id = first;
        assert!(matches!(front.on_share(stale), Err(FrontError::StaleJob)));

        // A ledger-only change at the same height also moves the id: a pending
        // balance big enough to flip the single paid slot changes the coinbase
        // outputs (different script and value).
        assert!(front.apply_front_event(&WireFrontEvent::LedgerApplied {
            rev: 3,
            block_hash: [0x66; 32],
            republish: true,
            pending_deltas: vec![(id_for(0x31), 400_000_000)],
        }));
        front.refresh().await.unwrap();
        let third = front.job().unwrap().job_id.clone();
        assert_ne!(third, second, "a ledger change is a new distribution");

        // And another unchanged refresh keeps it.
        front.refresh().await.unwrap();
        assert_eq!(front.job().unwrap().job_id, third);
    }

    /// SPEC §4.6: a held (unknown-keys) balance is the full `owed`, so a
    /// partially kept id keeps its residual in the pending rail.
    #[tokio::test]
    async fn a_held_balance_keeps_the_full_owed_residual() {
        let source = Arc::new(MockTemplateSource::new(template_at(
            800_000,
            0x1c00_ffff,
            0,
        )));
        let cfg = cfg_with(cfg(), |c| {
            c.fee_bps = 0;
            c.window_n = 1;
        });
        let mut front = Front::new(cfg, source, type32());
        let miner = id_for(0x11);

        // A restored window slice credits an identity whose keys we do not
        // know (a restarted front before the miner reconnects).
        front.apply_bootstrap(&BootstrapSnapshot {
            rev: 1,
            applied_blocks: vec![],
            pending_balance: vec![],
            ark_delivery: vec![],
            window_tail: vec![slice_row(9_000, 799_999, &[(miner, 1, 0)])],
            type32_blobs: vec![],
            slices_cursor: None,
            front_events_cursor: None,
        });
        // A held balance pushes owed above the coinbase capacity, so the
        // routed `kept` amount is only the payable part.
        assert!(front.apply_front_event(&WireFrontEvent::LedgerApplied {
            rev: 2,
            block_hash: [0x21; 32],
            republish: true,
            pending_deltas: vec![(miner, 100_000_000)],
        }));

        front.refresh().await.unwrap();
        let routing = front.routing().expect("routing").clone();
        // raw 312 500 000 + prior pending 100 000 000 = 412 500 000 owed.
        assert_eq!(routing.owed.get(&miner).copied(), Some(412_500_000));
        let held = routing
            .pending
            .iter()
            .find(|(id, _)| *id == miner)
            .copied()
            .expect("held on the pending rail");
        assert_eq!(held.1, 412_500_000, "the full owed is held, not the cap");
        assert_eq!(
            routing.pending_deltas.get(&miner).copied(),
            Some(412_500_000 - 100_000_000),
            "delta is owed - old"
        );
        let dist = front.distribution().expect("distribution");
        let entry = dist
            .entries
            .iter()
            .find(|e| e.sp_id == miner)
            .expect("held entry");
        assert_eq!(entry.rail, spc_accounting::Rail::Pending);
        assert_eq!(entry.sats, 412_500_000);
    }

    /// A distribution-only job change seals the open slice, so a slice never
    /// mixes shares minted under two jobs.
    #[tokio::test]
    async fn a_distribution_only_job_change_seals_the_open_slice() {
        let source = Arc::new(MockTemplateSource::new(template_at(
            800_000,
            0x1c00_ffff,
            0,
        )));
        let cfg = cfg_with(cfg(), |c| {
            c.fee_bps = 0;
            c.window_n = 1;
            c.max_payout_slots = 1;
            c.max_slice_shares = 1; // every other share seals a slice
        });
        let mut front = Front::new(cfg, source, type32());
        front.refresh().await.unwrap();

        // Two shares: the second seals a one-share slice, so the next refresh
        // publishes a different coinbase (job J2) and reopens the slice.
        front.on_share(share_for(&front, 0x21, 1)).unwrap();
        front.on_share(share_for(&front, 0x31, 2)).unwrap();
        front.refresh().await.unwrap();
        let _ = slice_events(&mut front);
        let j2 = front.job().unwrap().job_id.clone();

        // One share on the J2 coinbase, then a ledger-only change that flips
        // which id the single paid slot serves: same height, same prev-hash,
        // different coinbase (job J3).
        front.on_share(share_for(&front, 0x21, 3)).unwrap();
        assert!(front.apply_front_event(&WireFrontEvent::LedgerApplied {
            rev: 2,
            block_hash: [0x31; 32],
            republish: true,
            pending_deltas: vec![(id_for(0x31), 400_000_000)],
        }));
        front.refresh().await.unwrap();

        let j3 = front.job().unwrap().job_id.clone();
        assert_ne!(j3, j2, "a ledger change is a new coinbase");
        let events = slice_events(&mut front);
        assert_eq!(events.len(), 1, "the open slice was sealed");
        assert_eq!(events[0].n_shares, 1);
        assert_eq!(events[0].ref_job_id >> 32, 800_000);
        // The sealed slice was minted under the previous job ...
        assert_eq!(
            events[0].ref_job_id & 0xffff_ffff,
            u64::from(u32::from_str_radix(j2.as_str(), 16).unwrap()),
            "the sealed slice keeps the job it was minted under"
        );
        // ... and the reopened slice carries the new one.
        let (_, open_ref_job) = front.open_slice_refs().expect("open slice");
        assert_eq!(open_ref_job >> 32, 800_000);
        assert_eq!(
            open_ref_job & 0xffff_ffff,
            u64::from(u32::from_str_radix(j3.as_str(), 16).unwrap()),
            "the new slice names the new job"
        );

        // Old-job work is stale; new-job work lands in the new slice.
        let mut stale = share_for(&front, 0x21, 4);
        stale.job_id = j2;
        assert!(matches!(front.on_share(stale), Err(FrontError::StaleJob)));
        front.on_share(share_for(&front, 0x21, 5)).unwrap();
    }

    /// SPEC §4.7: JD weights are the full `owed`, not the coinbase-capped
    /// amount.
    #[tokio::test]
    async fn jd_weights_are_not_capped_by_the_coinbase() {
        let bits = 0x1c00_ffff;
        let source = Arc::new(MutableSource::new(template_at(800_000, bits, 0)));
        let cfg = cfg_with(cfg(), |c| {
            c.fee_bps = 0;
            c.window_n = 1;
            c.max_payout_slots = 1;
        });
        let mut front = Front::new(cfg, source.clone(), type32());
        front.refresh().await.unwrap();
        front.on_share(share_for(&front, 0x21, 1)).unwrap();
        source.set(template_at(800_001, bits, 0));
        front.refresh().await.unwrap();

        // A held balance pushes `owed` above the coinbase capacity.
        let id = id_for(0x21);
        assert!(front.apply_front_event(&WireFrontEvent::LedgerApplied {
            rev: 3,
            block_hash: [0x44; 32],
            republish: true,
            pending_deltas: vec![(id, 100_000_000)],
        }));
        front.refresh().await.unwrap();

        let dist = front.distribution().unwrap();
        let paid = dist
            .entries
            .iter()
            .find(|e| e.sp_id == id && e.rail == spc_accounting::Rail::Coinbase)
            .expect("coinbase entry")
            .sats;
        assert_eq!(paid, 312_500_000, "the coinbase pays its capacity");
        let weights = front.weights().expect("weights");
        assert_eq!(
            weights.weights[0].1, 412_500_000,
            "JD weight is the full owed, not the capped payout"
        );
    }

    #[tokio::test]
    async fn fee_band_seals_on_job_fee_increase() {
        let source = Arc::new(MutableSource::new(template_with(0x207f_ffff, 100)));
        let cfg = cfg_with(cfg(), |c| {
            c.slice_fee_delta_sats = 50;
            c.max_slice_shares = 65_000;
        });
        let mut front = Front::new(
            cfg,
            source.clone(),
            std::sync::Arc::new(std::sync::Mutex::new(crate::type32::Type32Store::default())),
        );
        front.refresh().await.unwrap();

        front.on_share(share(&front, 1)).unwrap();
        assert_eq!(slice_events(&mut front).len(), 0);

        // JD-style fee increase on the same height: the open slice keeps
        // ref_fees = 100, so the share at 200 seals it.
        source.set(template_with(0x207f_ffff, 200));
        front.refresh().await.unwrap();
        assert!(slice_events(&mut front).is_empty());

        front.on_share(share(&front, 2)).unwrap();
        let events = slice_events(&mut front);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].ref_fees, 100);

        // The successor slice pinned the new fee level: another share at 200
        // stays within the band and does not re-seal.
        front.on_share(share(&front, 3)).unwrap();
        assert!(slice_events(&mut front).is_empty());
    }

    #[tokio::test]
    async fn pool_template_delta_zero_never_seals_on_fee_band() {
        let source = Arc::new(MutableSource::new(template_with(0x207f_ffff, 100)));
        let mut front = Front::new(
            cfg(), // slice_fee_delta_sats = 0: pool-template mode
            source.clone(),
            std::sync::Arc::new(std::sync::Mutex::new(crate::type32::Type32Store::default())),
        );
        front.refresh().await.unwrap();

        front.on_share(share(&front, 1)).unwrap();
        source.set(template_with(0x207f_ffff, 10_000));
        front.refresh().await.unwrap();
        front.on_share(share(&front, 2)).unwrap();

        assert!(slice_events(&mut front).is_empty());
    }

    #[tokio::test]
    async fn bootstrap_snapshot_restores_pending_and_ark_registrations() {
        let mut front = Front::new(
            cfg(),
            Arc::new(MockTemplateSource::new(template())),
            type32(),
        );
        let id = id_for(0x21);
        let snapshot = BootstrapSnapshot {
            rev: 7,
            applied_blocks: vec![],
            pending_balance: vec![(id, 5_000)],
            ark_delivery: vec![spc_store::models::ArkDeliveryRow {
                sp_id: id,
                ark_id: [1, 2, 3, 4],
                b_ark: vec![2u8; 33],
                mailbox_mode: spc_store::models::MailboxMode::None,
                mailbox_m: None,
                mailbox_d: None,
            }],
            window_tail: vec![],
            type32_blobs: vec![],
            slices_cursor: None,
            front_events_cursor: None,
        };
        front.apply_bootstrap(&snapshot);
        assert_eq!(front.pending().get(&id), Some(&5_000));
        // Registered, but the snapshot carries `b_ark`, not a payable bark
        // address: the destination stays unknown and the balance is held.
        let cap = front.ark.get(&id).expect("registration applied");
        assert!(cap.registered);
        assert!(!cap.dest_known);

        // A live registration for a new id is applied; a stale revision is not.
        let other = id_for(0x31);
        assert!(front.apply_front_event(&WireFrontEvent::ArkRegApplied {
            rev: 8,
            sp_id: other,
        }));
        assert!(front.ark.get(&other).expect("registered").registered);
        assert!(!front.apply_front_event(&WireFrontEvent::ArkRegApplied {
            rev: 8,
            sp_id: other,
        }));
    }

    #[tokio::test]
    async fn ledger_applied_is_authoritative_including_for_this_fronts_blocks() {
        let source = Arc::new(MockTemplateSource::new(template()));
        let mut front = Front::new(cfg(), source, type32());
        front.refresh().await.unwrap();
        let id = id_for(0x21);

        // Find a block ourselves: no local liability is booked.
        let mut own = share_for(&front, 0x21, 1);
        own.share_hash = [0u8; 32]; // meets the regtest network target
        front.on_share(own).unwrap().expect("block");
        assert!(front.pending().is_empty());
        let block_hash = bitcoin::hashes::sha256d::Hash::hash(
            &front.last_block().expect("assembled block").header,
        )
        .to_byte_array();

        // The payout role's delta for that very block is applied: it is the
        // only writer of the ledger.
        assert!(front.apply_front_event(&WireFrontEvent::LedgerApplied {
            rev: 6,
            block_hash,
            republish: true,
            pending_deltas: vec![(id, 1_500)],
        }));
        assert_eq!(front.pending().get(&id), Some(&1_500));

        // A balance that reaches zero is removed, never stored.
        assert!(front.apply_front_event(&WireFrontEvent::LedgerApplied {
            rev: 7,
            block_hash: [0x88; 32],
            republish: true,
            pending_deltas: vec![(id, -1_500)],
        }));
        assert!(!front.pending().contains_key(&id));
    }

    /// A `LedgerApplied` for a block already reflected in the ledger is a
    /// no-op even with a newer `rev`: replay/lost-XADD recovery is block-keyed.
    #[tokio::test]
    async fn a_republished_ledger_event_is_applied_once() {
        let mut front = Front::new(
            cfg(),
            Arc::new(MockTemplateSource::new(template())),
            type32(),
        );
        let id = id_for(0x21);
        let block = [0x91; 32];

        assert!(front.apply_front_event(&WireFrontEvent::LedgerApplied {
            rev: 5,
            block_hash: block,
            republish: true,
            pending_deltas: vec![(id, 1_000)],
        }));
        assert_eq!(front.pending().get(&id), Some(&1_000));

        // The payout role republishes the same block: newer rev, same block.
        assert!(!front.apply_front_event(&WireFrontEvent::LedgerApplied {
            rev: 9,
            block_hash: block,
            republish: true,
            pending_deltas: vec![(id, 1_000)],
        }));
        assert_eq!(front.pending().get(&id), Some(&1_000), "not double counted");

        // A different block still applies.
        assert!(front.apply_front_event(&WireFrontEvent::LedgerApplied {
            rev: 10,
            block_hash: [0x92; 32],
            republish: true,
            pending_deltas: vec![(id, 500)],
        }));
        assert_eq!(front.pending().get(&id), Some(&1_500));
    }

    /// Blocks named in the bootstrap snapshot are already in its pending
    /// balance: a republished event for one of them must not apply again.
    #[tokio::test]
    async fn bootstrapped_applied_blocks_are_not_reapplied() {
        let mut front = Front::new(
            cfg(),
            Arc::new(MockTemplateSource::new(template())),
            type32(),
        );
        let id = id_for(0x21);
        let block = [0x93; 32];
        front.apply_bootstrap(&BootstrapSnapshot {
            rev: 1,
            applied_blocks: vec![block],
            pending_balance: vec![(id, 7_000)],
            ark_delivery: vec![],
            window_tail: vec![],
            type32_blobs: vec![],
            slices_cursor: None,
            front_events_cursor: None,
        });
        assert!(!front.apply_front_event(&WireFrontEvent::LedgerApplied {
            rev: 2,
            block_hash: block,
            republish: true,
            pending_deltas: vec![(id, 7_000)],
        }));
        assert_eq!(front.pending().get(&id), Some(&7_000));
    }

    /// A `spc:front:events` entry at or below the bootstrapped rev is a replay
    /// of settlement the snapshot already contains: it must not be applied.
    #[tokio::test]
    async fn events_at_or_below_the_bootstrap_rev_are_ignored() {
        let mut front = Front::new(
            cfg(),
            Arc::new(MockTemplateSource::new(template())),
            type32(),
        );
        let id = id_for(0x21);
        let snapshot = BootstrapSnapshot {
            rev: 42,
            applied_blocks: vec![],
            pending_balance: vec![(id, 5_000)],
            ark_delivery: vec![],
            window_tail: vec![],
            type32_blobs: vec![],
            slices_cursor: None,
            front_events_cursor: None,
        };
        front.apply_bootstrap(&snapshot);
        assert_eq!(front.pending().get(&id), Some(&5_000));

        // Same rev as the snapshot: already reflected in `pending_balance`.
        assert!(!front.apply_front_event(&WireFrontEvent::LedgerApplied {
            rev: 42,
            block_hash: [0x01; 32],
            republish: true,
            pending_deltas: vec![(id, 1_000)],
        }));
        assert_eq!(front.pending().get(&id), Some(&5_000));
        // Older rev: also ignored.
        assert!(!front.apply_front_event(&WireFrontEvent::LedgerApplied {
            rev: 41,
            block_hash: [0x02; 32],
            republish: true,
            pending_deltas: vec![(id, 1_000)],
        }));
        assert_eq!(front.pending().get(&id), Some(&5_000));
        // Strictly newer: applied.
        assert!(front.apply_front_event(&WireFrontEvent::LedgerApplied {
            rev: 43,
            block_hash: [0x03; 32],
            republish: true,
            pending_deltas: vec![(id, 1_000)],
        }));
        assert_eq!(front.pending().get(&id), Some(&6_000));
    }
}
