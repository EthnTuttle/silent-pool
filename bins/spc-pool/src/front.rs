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

use crate::registration::{self, ArkServerMetadata};
use bitcoin::hashes::{sha256, Hash, HashEngine};
use bitcoin::pow::Target;
use bitcoin::BlockHash;
use spc_accounting::{
    build_distribution, build_weights, raw_payouts, raw_payouts_v2, select_window,
    select_window_v2, ArkCapability, Distribution, MinerKeys, Routing, ShareRec, Slice, SliceAgg,
    SpId, TierConfig, Weights, WorkUnitVersion,
};
use spc_crypto::{Network, SendKey};
use spc_store::models::{
    ArkDeliveryRow, ArkEpochMailboxRow, BootstrapSnapshot, SliceRow, V2_RETENTION_HEIGHT_BLOCKS,
};
use spc_store::wire::FrontEvent as WireFrontEvent;
use spc_store::wire::{BlockEvent, Entry as WireEntry, Rail as WireRail, SliceEvent};
use thiserror::Error;

use crate::block::{
    assemble, assemble_witness, coinbase_merkle_branch, merkle_root, AssembledBlock,
};
use crate::coinbase::{build_pool_template, commitment_output, CoinbaseTemplate};
use crate::sv1::{
    network_target, ntime_allowed, nullifier, version_allowed, Job, Share, SharePolicy,
};
use crate::template::{analyze_transactions, Template, TemplateError, TemplateSource};
use crate::type32::Type32Store;
use crate::witness;

/// The 56 low bits of a slice id: `slice_id = front_id << 56 | local_seq`
/// (MULTI-REGION §10, same composition as the JDS distribution id).
const LOCAL_SEQ_MASK: u64 = 0x00ff_ffff_ffff_ffff;

/// Explicit lab-v2 bound for the live work-aware tail. The legacy v1
/// `retain_windows` slice count is not changed. Within this bound v2 retains
/// enough whole slices to reach the current Q; if the bound is exhausted,
/// `window_underfilled` remains true instead of claiming full coverage.
pub const V2_RETAINED_SLICES_LIMIT: usize = 4096;

/// The front id encoded in a slice id.
fn slice_front_id(slice_id: u64) -> u8 {
    u8::try_from(slice_id >> 56).unwrap_or(0)
}

#[derive(Debug, Error)]
pub enum FrontError {
    #[error("decision observation unavailable: {0}")]
    Observation(#[from] spc_store::postgres::StoreError),
    #[error("accounting unavailable: {0}")]
    AccountingUnavailable(String),
    #[error("template: {0}")]
    Template(#[from] TemplateError),
    #[error("accounting: {0}")]
    Accounting(#[from] spc_accounting::Error),
    #[error("crypto: {0}")]
    Crypto(#[from] spc_crypto::Error),
    #[error("block assembly: {0}")]
    Block(#[from] crate::block::BlockError),
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
    #[error("invalid extranonce length")]
    ExtranonceLength,
    #[error("share header/coinbase/hash mismatch")]
    InvalidShare,
    #[error("work-unit policy: {0}")]
    WorkUnit(#[from] spc_accounting::WorkUnitError),
    #[error(
        "work_unit_restore_mismatch on slice {slice_id}: expected {expected:?}, found {found:?}"
    )]
    WorkUnitRestoreMismatch {
        slice_id: u64,
        expected: Option<[u8; 32]>,
        found: Option<[u8; 32]>,
    },
    #[error("invalid restored slice {0}")]
    InvalidRestoredSlice(u64),
    #[error("invalid Ark registration: {0}")]
    InvalidRegistration(String),
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
    economic_source: Option<spc_store::Store>,
    observation_streams: Option<spc_store::redis::Streams>,
    economic_view: Option<spc_accounting::economic::View>,
    frozen_economic: Option<spc_accounting::economic::FrozenJob>,
    published_economic: Option<[u8; 32]>,
    awaiting_found: Option<u32>,
    accounting_fault: bool,
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
    /// Full committed registrations used to derive each generation's fresh
    /// Ark destination while `a_send` is available.
    ark_registrations: BTreeMap<SpId, ArkDeliveryRow>,
    ark_epoch_mailboxes: BTreeMap<(SpId, u32), ArkEpochMailboxRow>,
    ark_server: Option<ArkServerMetadata>,
    ark_dest: BTreeMap<SpId, Vec<u8>>,
    /// Versioned work-unit/target policy (`[accounting]`).
    share_policy: SharePolicy,
    /// Policy applied for the current template: the credited `d` and the
    /// target every share must meet.
    policy: Option<AppliedPolicy>,
    /// Whether the selected v2 tail did not reach Q (derived, not persisted).
    window_underfilled: bool,
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
    /// Monotonic target/credit generation used by the evidence oracle.
    generation: u64,
    /// Whether the preceding v2 generation selected a lower multiplier.
    previous_stepped_down: bool,
    /// Merkle identity of the template's non-coinbase tx set, used to detect a
    /// same-height tx-set change.
    template_id: [u8; 32],
    /// Non-coinbase txids of the current template, in template order (type-32
    /// `GetTransactionsInJob`).
    txids: Vec<[u8; 32]>,
    /// BIP 141 witness plan for the current job: present exactly when the
    /// template's transaction set carries witness data (decision
    /// 2026-11-08-pool-witness-commitment). Cleared with every other piece
    /// of current work on a failed refresh.
    witness_plan: Option<WitnessPlan>,
    /// One-entry witness-reserved cache, keyed by the wtxid root. Not
    /// cleared with the work: an unchanged transaction set must keep the
    /// same reserved value (and thus the same commitment) across job
    /// republications, or shares already hashed against the coinbase
    /// would silently stop meeting it.
    reserved_cache: Option<([u8; 32], [u8; 32])>,
    /// Fingerprint of the distribution last published, to detect a
    /// distribution-only change that still needs a new job id.
    published_fingerprint: Option<[u8; 32]>,
    /// Registration-sensitive entries beside the coinbase fingerprint: rail and
    /// per-output Ark destination must invalidate work even when outputs match.
    published_entries: Option<[u8; 32]>,
    published_window: Option<[u8; 32]>,
    /// Highest `spc:front:events` revision applied (bootstrap or live).
    last_rev: u64,
    /// Confirmed blocks whose `LedgerApplied` is already reflected in
    /// `pending`: bootstrapped from the snapshot and extended as events are
    /// applied, so a replayed/republished event is a no-op regardless of `rev`.
    applied_blocks: HashSet<[u8; 32]>,
    last_block: Option<AssembledBlock>,
    events: Vec<FrontEvent>,
    type32: Arc<Mutex<Type32Store>>,
    type32_enabled: bool,
    // Legacy unit fixtures model accounting with synthetic hashes. Production
    // and integration-test libraries always revalidate the full header.
    #[cfg(test)]
    revalidate_headers: bool,
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

/// The BIP 141 witness-commitment plan for one job (decision
/// 2026-11-08-pool-witness-commitment). The commitment output is baked into
/// the coinbase at refresh time; the reserved value is injected as the
/// coinbase's single witness item only at block-assembly time, because the
/// witness is not part of what miners hash.
#[derive(Clone, Debug, PartialEq, Eq)]
struct WitnessPlan {
    /// Witness merkle root of the non-coinbase transactions (zero leaf for
    /// the coinbase).
    wtxid_root: [u8; 32],
    /// The witness reserved value (OS CSPRNG per wtxid root), injected as
    /// the coinbase's single 32-byte witness item at assembly time.
    reserved: [u8; 32],
    /// `SHA256d(wtxid_root ‖ reserved)`, carried by the coinbase's last
    /// output.
    commitment: [u8; 32],
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
            // The current front reserves against the configured hard ceiling;
            // the autoscaler floor is not permission to trim a denomination set.
            budget_wu: c.budget.ceiling_wu,
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
            economic_source: None,
            observation_streams: None,
            economic_view: None,
            frozen_economic: None,
            published_economic: None,
            awaiting_found: None,
            accounting_fault: false,
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
            window_underfilled: false,
            miners: BTreeMap::new(),
            pending: BTreeMap::new(),
            ark: BTreeMap::new(),
            ark_registrations: BTreeMap::new(),
            ark_epoch_mailboxes: BTreeMap::new(),
            ark_server: None,
            ark_dest: BTreeMap::new(),
            share_policy: SharePolicy::v1(1),
            generation: unix_ms(),
            previous_stepped_down: false,
            policy: None,
            routing: None,
            seen_shares: HashSet::new(),
            seen_prev_hash: None,
            job_seq: (unix_ms() as u32) | 1,
            template_id: [0u8; 32],
            txids: Vec::new(),
            witness_plan: None,
            reserved_cache: None,
            published_fingerprint: None,
            published_entries: None,
            published_window: None,
            last_rev: 0,
            applied_blocks: HashSet::new(),
            last_block: None,
            events: Vec::new(),
            type32,
            // Library fixtures default to legacy v1. Production passes the
            // configured flag; roll_slice also refuses v2 unconditionally.
            type32_enabled: true,
            #[cfg(test)]
            revalidate_headers: false,
        }
    }

    /// Set the pool share difficulty (validated `> 0`; 0 is rejected rather
    /// than silently clamped so a bad config cannot weaken share validation).
    pub fn set_share_difficulty(&mut self, share_difficulty: u64) -> Result<(), FrontError> {
        if share_difficulty == 0 {
            return Err(FrontError::ShareDifficulty);
        }
        self.share_policy = SharePolicy::v1(share_difficulty);
        Ok(())
    }

    pub fn set_share_policy(&mut self, policy: SharePolicy) {
        self.share_policy = policy;
    }
    /// Read-only economic and committed-registration provenance access.
    pub fn set_economic_source(&mut self, store: spc_store::Store) {
        self.economic_source = Some(store);
    }
    pub fn set_observation_streams(&mut self, streams: spc_store::redis::Streams) {
        self.observation_streams = Some(streams);
    }

    /// Configure whether sealed legacy slices may feed the type-32 store.
    /// v2 slices are refused independently of this flag.
    pub fn set_type32_enabled(&mut self, enabled: bool) {
        self.type32_enabled = enabled;
    }

    pub fn share_policy(&self) -> &SharePolicy {
        &self.share_policy
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

    /// Whether the selected v2 tail did not reach its frozen Q. Derived at
    /// selection time, never stored as another column or fact.
    pub fn window_underfilled(&self) -> bool {
        self.window_underfilled
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
                registration_committed: false,
                dest_known: false,
            })
            .dest_known = true;
    }

    /// Supply public ArkInfo metadata. No wallet/order secret enters Front.
    pub fn set_ark_server(&mut self, metadata: ArkServerMetadata) {
        self.ark_server = Some(metadata);
    }

    async fn persist_derivation_observation(
        &self,
        attempt: &[u8; 16],
        mut body: serde_json::Value,
    ) -> Result<(), FrontError> {
        body["schema"] = "spc.front-derivation/1".into();
        let session = crate::evidence::session_id();
        body["source_session"] = hex::encode(session).into();
        body["front_id"] = self.cfg.front_id.into();
        body["queried_identity_census"] = body["rows"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .map(|row| row["sp_id"].clone())
                    .collect::<Vec<_>>()
            })
            .into();
        body["census_complete"] = (body["status"] == "frozen_job").into();
        body["a_send_x"] = self.send.as_ref().map(|s| hex::encode(s.x_only())).into();
        use spc_store::observations::{
            observation_digest, FrontObservation, FrontObservationAck, FRONT_OBSERVATIONS_ACK,
            FRONT_OBSERVATIONS_STREAM,
        };
        let request = FrontObservation {
            schema_version: 1,
            session,
            attempt: *attempt,
            body: body.clone(),
        };
        let bytes = request.encode()?;
        let observation = if let Some(streams) = &self.observation_streams {
            crate::evidence::emit(
                "derivation_prepared",
                serde_json::json!({"attempt":hex::encode(attempt),"record":body}),
            );
            streams
                .add(FRONT_OBSERVATIONS_STREAM, &bytes)
                .await
                .map_err(|e| {
                    FrontError::AccountingUnavailable(format!("observation publish: {e}"))
                })?;
            let ack = tokio::time::timeout(std::time::Duration::from_secs(2), async {
                loop {
                    if let Some(value) = streams
                        .hget(FRONT_OBSERVATIONS_ACK, &request.field())
                        .await?
                    {
                        return Ok::<_, spc_store::redis::RedisError>(value);
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .map_err(|_| {
                FrontError::AccountingUnavailable("observation persistence deadline".into())
            })?
            .map_err(|e| FrontError::AccountingUnavailable(format!("observation receipt: {e}")))?;
            let ack: FrontObservationAck = serde_json::from_str(&ack).map_err(|_| {
                FrontError::AccountingUnavailable("invalid observation receipt".into())
            })?;
            if ack.digest != observation_digest(&bytes) {
                return Err(FrontError::AccountingUnavailable(
                    "observation receipt digest".into(),
                ));
            }
            if ack.observation_id <= 0 {
                return Err(FrontError::AccountingUnavailable(
                    "observation receipt id".into(),
                ));
            }
            Some(ack.observation_id)
        } else {
            None
        };
        crate::evidence::emit(
            "derivation_observed",
            serde_json::json!({"observation_id":observation,"attempt":hex::encode(attempt),"record":body}),
        );
        Ok(())
    }

    async fn observed_delivery(
        &self,
        id: &SpId,
        mut delivery: ArkDeliveryRow,
    ) -> (ArkDeliveryRow, &'static str) {
        let mut provenance = if delivery.registration_commit.is_some() {
            "cached"
        } else {
            "missing_commit_provenance"
        };
        if let (Some(commit), Some(store)) = (
            delivery.registration_commit.as_mut(),
            self.economic_source.as_ref(),
        ) {
            if commit.signed_fields_digest.is_none() {
                match store.registration_commit_digest(id, commit).await {
                    Ok(digest) => {
                        provenance = if digest.is_some() {
                            "exact_request_row"
                        } else {
                            "missing_request_provenance"
                        };
                        commit.signed_fields_digest = digest;
                    }
                    Err(_) => provenance = "request_lookup_failed_unknown",
                }
            }
        }
        (delivery, provenance)
    }

    async fn refresh_ark_destinations(
        &mut self,
        attempt: &[u8; 16],
    ) -> Result<Vec<serde_json::Value>, FrontError> {
        let mut observations = Vec::new();
        let Some(metadata) = self.ark_server.clone() else {
            let registrations: Vec<_> = self
                .ark_registrations
                .iter()
                .map(|(id, d)| (*id, d.clone()))
                .collect();
            for (id, delivery) in registrations {
                let (delivery, provenance) = self.observed_delivery(&id, delivery).await;
                if let Some(cached) = self.ark_registrations.get_mut(&id) {
                    cached.registration_commit = delivery.registration_commit.clone();
                }
                let mut row = crate::derivation_observation::inputs(
                    &id,
                    self.miners.get(&id),
                    Some(&delivery),
                    None,
                    None,
                );
                row["commit_provenance"] = provenance.into();
                row["outcome"] = "server_metadata_unavailable".into();
                observations.push(row);
            }
            for cap in self.ark.values_mut() {
                cap.dest_known = false;
            }
            self.ark_dest.clear();
            return Ok(observations);
        };
        let registrations: Vec<(SpId, ArkDeliveryRow)> = self
            .ark_registrations
            .iter()
            .map(|(sp_id, delivery)| (*sp_id, delivery.clone()))
            .collect();
        let mut derived = Vec::new();
        for (sp_id, delivery) in registrations {
            let (delivery, commit_lookup) = self.observed_delivery(&sp_id, delivery).await;
            if let Some(cached) = self.ark_registrations.get_mut(&sp_id) {
                cached.registration_commit = delivery.registration_commit.clone();
            }
            let epoch = self
                .send
                .as_ref()
                .and_then(|s| self.ark_epoch_mailboxes.get(&(sp_id, s.h)))
                .cloned();
            let mut row = crate::derivation_observation::inputs(
                &sp_id,
                self.miners.get(&sp_id),
                Some(&delivery),
                Some(&metadata),
                epoch.as_ref(),
            );
            row["commit_provenance"] = commit_lookup.into();
            let mut stage = "miner_keys";
            let result: Result<Option<Vec<u8>>, FrontError> = (|| {
                if let Some(keys) = self.miners.get(&sp_id) {
                    stage = "send_context";
                    let send = self.send.as_ref().ok_or(FrontError::NoSendKey)?;
                    stage = "ecdh";
                    let ecdh = send.derive_pool_side(&keys.b_scan)?;
                    stage = "ark_destination";
                    Ok(registration::derive_ark_destination(
                        &metadata,
                        send.h,
                        &ecdh,
                        &delivery,
                        epoch.as_ref(),
                    )?)
                } else {
                    Ok(None)
                }
            })();
            let destination = match result {
                Ok(destination) => destination,
                Err(error) => {
                    row["outcome"] = "derivation_error".into();
                    row["error_stage"] = stage.into();
                    row["error_reason"] = error.to_string().into();
                    observations.push(row);
                    self.persist_derivation_observation(attempt,serde_json::json!({"status":"failed_unadvertised_attempt", "job_id":null,"generation":null,
                        "height":self.send.as_ref().map(|s|s.h),"rows":observations})).await?;
                    return Err(error);
                }
            };
            row["destination_bytes"] = destination.as_ref().map(hex::encode).into();
            row["outcome"] = if destination.is_some() {
                "derived"
            } else if self.miners.contains_key(&sp_id) {
                "epoch_mailbox_held"
            } else {
                "missing_miner_keys"
            }
            .into();
            observations.push(row);
            derived.push((sp_id, destination));
        }
        for (sp_id, destination) in derived {
            self.set_ark_dest(sp_id, destination.unwrap_or_default());
        }
        Ok(observations)
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
    pub fn apply_bootstrap(&mut self, snapshot: &BootstrapSnapshot) -> Result<(), FrontError> {
        if self.economic_source.is_some() && snapshot.economic.is_none() {
            self.accounting_fault = true;
            return Err(FrontError::AccountingUnavailable(
                "bootstrap omitted reservations".into(),
            ));
        }
        self.economic_view = snapshot.economic.clone();
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
        for row in &snapshot.ark_identities {
            let scan: [u8; 33] = row
                .b_scan
                .as_slice()
                .try_into()
                .map_err(|_| FrontError::InvalidRegistration("b_scan length".into()))?;
            let spend: [u8; 33] = row
                .b_spend
                .as_slice()
                .try_into()
                .map_err(|_| FrontError::InvalidRegistration("b_spend length".into()))?;
            self.learn_identity(row.sp_id, scan, spend)?;
        }
        for row in &snapshot.ark_delivery {
            self.ark_registrations.insert(row.sp_id, row.clone());
            self.ark.insert(
                row.sp_id,
                ArkCapability {
                    registered: true,
                    registration_committed: true,
                    dest_known: false,
                },
            );
        }
        for row in &snapshot.ark_epoch_mailboxes {
            self.ark_epoch_mailboxes
                .insert((row.sp_id, row.height), row.clone());
        }

        let mut restored: Vec<SliceAgg> = snapshot
            .window_tail
            .iter()
            .filter(|row| slice_front_id(row.slice_id) == self.cfg.front_id)
            .map(|row| {
                let found = row.work_unit_fingerprint;
                if row.work_unit_version != self.share_policy.version()
                    || found != self.share_policy.fingerprint()
                {
                    return Err(FrontError::WorkUnitRestoreMismatch {
                        slice_id: row.slice_id,
                        expected: self.share_policy.fingerprint(),
                        found,
                    });
                }
                let decoded = slice_agg_from_row(row);
                if decoded.is_none() && row.work_unit_version == WorkUnitVersion::V2 {
                    return Err(FrontError::InvalidRestoredSlice(row.slice_id));
                }
                Ok(decoded)
            })
            .collect::<Result<Vec<Option<_>>, FrontError>>()?
            .into_iter()
            .flatten()
            .collect();
        restored.sort_by(|a, b| b.slice_id.cmp(&a.slice_id));
        if let Some(newest) = restored.first().map(|s| s.slice_id) {
            // Ids continue above everything settlement has already seen.
            self.high_water = newest;
            self.slice_seq = newest & LOCAL_SEQ_MASK;
        }
        match self.share_policy.version() {
            WorkUnitVersion::V1 => restored.truncate(self.cfg.keep_slices.max(1)),
            // Q is not known until the next template. Preserve the bounded
            // restored tail until refresh can apply the work-aware budget.
            WorkUnitVersion::V2 => restored.truncate(V2_RETAINED_SLICES_LIMIT),
        }
        self.slices = restored;
        crate::evidence::emit(
            "bootstrap_applied",
            serde_json::json!({"front_id":self.cfg.front_id,"revision":snapshot.rev.to_string(),
            "pending":self.pending.iter().map(|(id,sats)|serde_json::json!({"sp_id":hex::encode(id),"sats":sats.to_string()})).collect::<Vec<_>>(),
            "applied_blocks":snapshot.applied_blocks.iter().map(hex::encode).collect::<Vec<_>>(),
            "restored_slice_ids":self.slices.iter().map(|s|s.slice_id.to_string()).collect::<Vec<_>>(),
            "external_history_checkpoint_required":true}),
        );
        Ok(())
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
            WireFrontEvent::EconomicChanged {
                economic_revision,
                anchor_hash,
                ..
            } => {
                crate::evidence::emit(
                    "economic_change_observed",
                    serde_json::json!({"economic_revision":economic_revision.to_string(),"anchor_hash":hex::encode(anchor_hash),"coherent_snapshot_reload_required":true}),
                );
                self.economic_source.is_some()
            }
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
                if self.economic_source.is_none() {
                    if self.share_policy.version() == WorkUnitVersion::V2 {
                        let mut updated = self.pending.clone();
                        for (id, delta) in &deltas {
                            let value = i128::from(updated.get(id).copied().unwrap_or(0))
                                + i128::from(*delta);
                            let Ok(value) = u64::try_from(value) else {
                                self.accounting_fault = true;
                                self.invalidate_current_work();
                                return false;
                            };
                            if value == 0 {
                                updated.remove(id);
                            } else {
                                updated.insert(*id, value);
                            }
                        }
                        self.pending = updated;
                    } else {
                        apply_pending_deltas(&mut self.pending, &deltas);
                    }
                }
                self.applied_blocks.insert(*block_hash);
                crate::evidence::emit(
                    "pending_applied",
                    serde_json::json!({"front_id":self.cfg.front_id,"revision":rev.to_string(),
                    "block_hash":hex::encode(block_hash),"pending_deltas":deltas.iter().map(|(id,delta)|serde_json::json!({"sp_id":hex::encode(id),"delta":delta.to_string()})).collect::<Vec<_>>(),
                    "pending_after":self.pending.iter().map(|(id,sats)|serde_json::json!({"sp_id":hex::encode(id),"sats":sats.to_string()})).collect::<Vec<_>>()}),
                );
                true
            }
            WireFrontEvent::ArkRegApplied {
                sp_id,
                request_id,
                registration_rev,
                state,
                identity,
                delivery,
                epoch_mailboxes,
                ..
            } => {
                if *state != spc_ext::RegistrationSuccessState::Superseded {
                    let (Ok(scan), Ok(spend)) = (
                        <[u8; 33]>::try_from(identity.b_scan.as_slice()),
                        <[u8; 33]>::try_from(identity.b_spend.as_slice()),
                    ) else {
                        tracing::error!(
                            ?sp_id,
                            "invalid identity key length in ArkRegApplied; skipping"
                        );
                        return false;
                    };
                    if let Err(error) = self.learn_identity(*sp_id, scan, spend) {
                        tracing::error!(?sp_id, %error, "invalid identity point in ArkRegApplied; skipping");
                        return false;
                    }
                    let mut observed_delivery = delivery.clone();
                    observed_delivery.registration_commit =
                        Some(Box::new(spc_store::models::RegistrationCommit {
                            request_id: *request_id,
                            registration_rev: *registration_rev,
                            signed_fields_digest: None,
                        }));
                    self.ark_registrations.insert(*sp_id, observed_delivery);
                    for entry in epoch_mailboxes {
                        self.ark_epoch_mailboxes
                            .insert((entry.sp_id, entry.height), entry.clone());
                    }
                    self.ark.insert(
                        *sp_id,
                        ArkCapability {
                            registered: true,
                            registration_committed: true,
                            // Recomputed from this generation's a_send below.
                            dest_known: false,
                        },
                    );
                }
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
        let result = self.refresh_inner().await;
        if result.is_err() {
            // Any failed refresh invalidates work, including failures after a
            // roll/open/height transition. No share from the previous
            // generation can be booked under a partially refreshed tip.
            self.invalidate_current_work();
        }
        result
    }

    fn invalidate_current_work(&mut self) {
        self.job = None;
        self.policy = None;
        self.template = None;
        self.distribution = None;
        self.weights = None;
        self.routing = None;
        self.published_fingerprint = None;
        self.witness_plan = None;
        self.published_entries = None;
        self.published_window = None;
        self.window_underfilled = true;
    }

    async fn refresh_inner(&mut self) -> Result<Job, FrontError> {
        let template = self.source.get_template().await?;
        if let Some(store) = &self.economic_source {
            let view = store
                .economic_view()
                .await
                .map_err(|e| FrontError::AccountingUnavailable(e.to_string()))?;
            view.at_parent(
                template
                    .height
                    .checked_sub(1)
                    .ok_or_else(|| FrontError::AccountingUnavailable("template genesis".into()))?,
                template.prev_hash,
            )
            .map_err(|e| FrontError::AccountingUnavailable(e.to_string()))?;
            if self
                .awaiting_found
                .is_some_and(|height| view.anchor_height < height)
            {
                return Err(FrontError::AccountingUnavailable(
                    "found block reservation awaits canonical acknowledgement".into(),
                ));
            }
            self.pending = view
                .balances()
                .map_err(|e| FrontError::AccountingUnavailable(e.to_string()))?
                .into_iter()
                .filter(|(_, b)| b.available > 0)
                .map(|(id, b)| (id, b.available))
                .collect();
            self.economic_view = Some(view);
            self.awaiting_found = None;
            self.accounting_fault = false;
        } else if self.accounting_fault {
            return Err(FrontError::AccountingUnavailable(
                "invalid pending transition".into(),
            ));
        }
        // Fail closed on transactions we cannot hash correctly: every
        // non-coinbase tx must strictly decode. Witness data is fine - the
        // coinbase carries the BIP 141 commitment - but a tx with an
        // ambiguous encoding has no unambiguous txid or wtxid.
        let txset = analyze_transactions(&template.transactions)?;
        let txids = txset.txids;
        let merkle_branches = coinbase_merkle_branch(&txids);
        let template_id = merkle_root(&txids);

        // BIP 141: a transaction set with witness data needs a coinbase
        // commitment output. The plan is keyed by the wtxid root, so a
        // same-txid witness malleation republishes the job (shares on the
        // old commitment are stale), while an unchanged set keeps the same
        // reserved value and commitment.
        let witness_plan = if txset.witness_count > 0 {
            let wtxid_root = witness::witness_root(&txset.wtxids);
            let reserved = self.reserved_for(wtxid_root);
            let commitment = witness::witness_commitment(&wtxid_root, &reserved);
            Some(WitnessPlan {
                wtxid_root,
                reserved,
                commitment,
            })
        } else {
            None
        };

        // Hoist the deterministic fallible v2 planning ahead of every roll,
        // key, height, open-slice and replay-set transition. The single
        // preview value is also the next generation's immutable work value.
        let preview_work = self
            .share_policy
            .job_work(template.bits, self.generation.saturating_add(1))?;
        let window_target = self
            .share_policy
            .window_target(template.bits, self.cfg.window_n)?;
        // Freeze this template's target before the first roll. Live history
        // retention itself is Q-independent, so an older Q cannot prune it.
        self.window_target = window_target;

        let chain_moved = self.send.is_none()
            || self.height != template.height
            || self.open.as_ref().map(|s| s.prev_hash) != Some(template.prev_hash);
        let fees_moved = self.template.as_ref().map(|t| t.fees) != Some(template.fees);
        // A commitment change is a coinbase change even when every txid is
        // identical (witness malleation): shares hashed against the old
        // commitment are stale.
        let commitment_moved = self.witness_plan.as_ref().map(|p| p.commitment)
            != witness_plan.as_ref().map(|p| p.commitment);
        let job_changed = chain_moved
            || fees_moved
            || commitment_moved
            || self.template_id != template_id
            || self.template.as_ref().is_some_and(|previous| {
                previous.bits != template.bits || previous.version != template.version
            });
        // The job id must exist before the slice is opened: the slice records
        // `(H << 32) | job_id` of the job it belongs to (SPEC §3.4).
        let mut bumped = false;
        if job_changed || self.job.is_none() {
            self.job_seq = self.job_seq.wrapping_add(1);
            bumped = true;
        }
        if chain_moved {
            self.roll_slice()?;
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
        // the same `d` used for the ShareRec and for the window target.
        let policy = AppliedPolicy {
            credited_difficulty: preview_work.m_issue,
            share_target: preview_work.bitcoin_share_target(),
        };
        // Decision 2026-11-03-settlement-window-target: the exact target is
        // remembered with the distribution it selected. Selection is
        // work-aware at current Q; retained history is bounded only by the
        // independent height horizon and slice hard cap.
        self.trim_retained_slices(template.height);
        let window = match &self.share_policy {
            SharePolicy::V1 { .. } => {
                select_window(&self.slices, u64::MAX, window_target, BTreeMap::new())
            }
            SharePolicy::V2 { .. } => select_window_v2(
                &self.slices,
                u64::MAX,
                window_target,
                BTreeMap::new(),
                self.share_policy.fingerprint(),
            )?,
        };
        self.window_underfilled = window.underfilled(window_target);
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
            match &self.share_policy {
                SharePolicy::V1 { .. } => {
                    raw_payouts(&window, template.subsidy, template.fees, self.cfg.fee_bps)?
                }
                SharePolicy::V2 { .. } => {
                    raw_payouts_v2(&window, template.subsidy, template.fees, self.cfg.fee_bps)?
                }
            }
        };
        let derivation_attempt = rand::random();
        let mut derivation_observations =
            self.refresh_ark_destinations(&derivation_attempt).await?;

        let tier_cfg = TierConfig {
            fee_bps: self.cfg.fee_bps,
            dust_limit_sats: self.cfg.dust_limit_sats,
            coinbase_threshold_sats: self.cfg.coinbase_threshold_sats,
            max_payout_slots: self.cfg.max_payout_slots,
            budget_wu: self.cfg.budget_wu,
            // The BIP 141 commitment output and the coinbase witness field
            // consume block weight the distribution cannot spend on payout
            // outputs (SPEC §4.8): 188 WU + 36 WU when a commitment is
            // present.
            overhead_wu: self
                .cfg
                .overhead_wu
                .saturating_add(if witness_plan.is_some() {
                    witness::WITNESS_COMMITMENT_OVERHEAD_WU
                } else {
                    0
                }),
        };
        let known = self.miners.keys().copied().collect();
        let routing = spc_accounting::route_tiers_with_keys(
            &raw,
            &self.pending,
            &self.ark,
            &tier_cfg,
            template.coinbase_value,
            Some(&known),
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
        let mut jd_routing = spc_accounting::route_jd_tiers(
            &raw,
            &self.pending,
            &self.ark,
            &tier_cfg,
            template.coinbase_value,
        )?;
        hold_undeliverable(&mut jd_routing, &self.miners, &self.pending);
        let mut weights = build_weights(&jd_routing, raw.pool_sats);
        let send = self.send.as_ref().ok_or(FrontError::NoSendKey)?;
        for entry in &mut weights.entries {
            entry.dest_id = self.ark_dest.get(&entry.sp_id).cloned();
            if entry.rail == spc_accounting::Rail::Coinbase {
                let keys = self
                    .miners
                    .get(&entry.sp_id)
                    .ok_or(spc_accounting::Error::Derivation(entry.sp_id))?;
                let ecdh = send
                    .derive_pool_side(&keys.b_scan)
                    .map_err(|_| spc_accounting::Error::Derivation(entry.sp_id))?;
                let tweaks = spc_crypto::tier_tweaks(&ecdh, 0)
                    .map_err(|_| spc_accounting::Error::Derivation(entry.sp_id))?;
                let key = spc_crypto::spc_output_pubkey(&keys.b_spend, &tweaks.t_onchain)
                    .map_err(|_| spc_accounting::Error::Derivation(entry.sp_id))?;
                entry.p_x = vec![key.x_only_public_key().0.serialize()];
            }
        }

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
        let entries_fingerprint = entries_fingerprint(&distribution);
        let economic_fingerprint = self.economic_view.as_ref().map(|view| {
            sha256::Hash::hash(&serde_json::to_vec(view).expect("economic serialization"))
                .to_byte_array()
        });
        let window_fingerprint = window_fingerprint(
            &window,
            window_target,
            self.share_policy.window_rule().as_u8(),
        );
        if !bumped
            && (self.published_fingerprint != Some(distribution.fingerprint)
                || self.published_entries != Some(entries_fingerprint)
                || self.published_window != Some(window_fingerprint)
                || self.published_economic != economic_fingerprint)
        {
            self.job_seq = self.job_seq.wrapping_add(1);
            bumped = true;
            self.roll_slice()?;
            self.open_slice(template.height, template.prev_hash, template.fees);
        }
        self.published_fingerprint = Some(distribution.fingerprint);
        self.published_entries = Some(entries_fingerprint);
        self.published_window = Some(window_fingerprint);
        self.published_economic = economic_fingerprint;

        let mut outputs: Vec<(bitcoin::script::ScriptBuf, u64)> = distribution
            .outputs
            .iter()
            .map(|(s, v)| (bitcoin::script::ScriptBuf::from_bytes(s.clone()), *v))
            .collect();
        // The BIP 141 commitment output goes last: the pool output stays
        // index 0 (settlement's `fixed_outputs` takes it), every payout
        // script keeps its slot, and it is zero-value and unspendable.
        if let Some(plan) = &witness_plan {
            outputs.push(commitment_output(&plan.commitment));
        }
        let controlled_bytes = outputs
            .iter()
            .try_fold(0usize, |sum, (script, value)| {
                sum.checked_add(
                    bitcoin::consensus::serialize(&bitcoin::TxOut {
                        value: bitcoin::Amount::from_sat(*value),
                        script_pubkey: script.clone(),
                    })
                    .len(),
                )
            })
            .ok_or(spc_accounting::Error::Overflow)?;
        if controlled_bytes > template.coinbase_output_max_additional_size as usize {
            return Err(spc_accounting::Error::PayoutWeight.into());
        }
        let coinbase = build_pool_template(
            template.version,
            template.height,
            &a_send_x,
            self.cfg.extranonce_len,
            &outputs,
            0,
        );
        let stripped = coinbase.assemble(&vec![0; self.cfg.extranonce_len]);
        crate::block::validate_template_weight(
            &stripped,
            &template.transactions,
            witness_plan.is_some(),
        )?;

        let work = if bumped {
            self.generation = self.generation.saturating_add(1);
            let work = preview_work.clone();
            if work.stepped_down {
                crate::evidence::multiplier_step("multiplier_stepdown", self.cfg.front_id, &work);
            } else if self.previous_stepped_down {
                crate::evidence::multiplier_step("multiplier_stepup", self.cfg.front_id, &work);
            }
            self.previous_stepped_down = work.stepped_down;
            work
        } else {
            self.job
                .as_ref()
                .map(|previous| previous.work.clone())
                .unwrap_or(preview_work)
        };

        let job = Job {
            // The wire job id is the template sequence, not the height: an
            // equal-height reorg or tx-set change must invalidate stale work.
            job_id: format!("{:08x}", self.job_seq),
            prev_hash: template.prev_hash,
            height: template.height,
            version: template.version,
            nbits: template.bits,
            // A normal GBT curtime advance does not republish a job. Keep
            // the issued min_ntime so validation uses exactly what miners saw.
            ntime: if bumped {
                template.curtime
            } else {
                self.job
                    .as_ref()
                    .map_or(template.curtime, |issued| issued.ntime)
            },
            coinb1: coinbase.prefix,
            coinb2: coinbase.suffix,
            merkle_branches,
            clean_jobs: true,
            work,
        };
        self.frozen_economic = if let Some(view) = &self.economic_view {
            let all = view
                .balances()
                .map_err(|e| FrontError::AccountingUnavailable(e.to_string()))?;
            Some(spc_accounting::economic::FrozenJob {
                version: 1,
                revision: view.revision,
                parent_hash: template.prev_hash,
                job_id: (u64::from(template.height) << 32) | u64::from(self.job_seq),
                generation: job.work.generation,
                slice_ids: window.slices.iter().map(|slice| slice.slice_id).collect(),
                balances: distribution
                    .entries
                    .iter()
                    .map(|e| {
                        all.get(&e.sp_id)
                            .cloned()
                            .unwrap_or(spc_accounting::economic::Balance {
                                sp_id: e.sp_id,
                                booked: 0,
                                reserved: 0,
                                available: 0,
                            })
                    })
                    .collect(),
            })
        } else {
            None
        };
        if bumped {
            for id in routing
                .owed
                .keys()
                .filter(|id| !self.ark_registrations.contains_key(*id))
            {
                let mut row = crate::derivation_observation::inputs(
                    id,
                    self.miners.get(id),
                    None,
                    self.ark_server.as_ref(),
                    None,
                );
                row["outcome"] = "unregistered_in_front_cache".into();
                derivation_observations.push(row);
            }
            self.persist_derivation_observation(&derivation_attempt,serde_json::json!({"status":"frozen_job",
                "job_id":(u64::from(job.height)<<32)|u64::from(self.job_seq),"generation":job.work.generation,"height":job.height,
                "economic_revision":self.frozen_economic.as_ref().map(|e|e.revision),"distribution_fingerprint":hex::encode(distribution.fingerprint),
                "rows":derivation_observations})).await?;
            crate::evidence::frozen_distribution(crate::evidence::FrozenEvidence {
                finding_slice_id: self.open.as_ref().ok_or(FrontError::NoSlice)?.slice_id,
                front_id: self.cfg.front_id,
                job: &job,
                superseded_job_id: self.job.as_ref().map(|previous| previous.job_id.as_str()),
                economic: self.frozen_economic.as_ref(),
                distribution: &distribution,
                window: &window,
                pending: &self.pending,
                ark: &self.ark,
                window_target,
                window_rule: self.share_policy.window_rule().as_u8(),
                fee_bps: self.cfg.fee_bps,
                subsidy: template.subsidy,
                fees: template.fees,
                extranonce_len: self.cfg.extranonce_len,
                entries_digest: entries_fingerprint,
                witness: witness_plan
                    .as_ref()
                    .map(|p| (&p.wtxid_root, &p.reserved, &p.commitment)),
                transactions: &template.transactions,
            });
        }
        self.job = Some(job.clone());
        self.distribution = Some(distribution);
        self.weights = Some(weights);
        self.routing = Some(routing);
        self.policy = Some(policy);
        self.witness_plan = witness_plan;
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
        let job = self.job.clone().ok_or(FrontError::NoJob)?;
        let template = self
            .template
            .as_ref()
            .ok_or(FrontError::NoTemplate)?
            .clone();
        let policy = self.policy.ok_or(FrontError::NoTemplate)?;

        // Stale work is rejected before anything is credited: the job id
        // changes with the height, the prev-hash (including a same-height
        // reorg), the tx set and the job-fee level (SPEC §3.4).
        if share.job_id != job.job_id {
            crate::evidence::emit(
                "share_rejected",
                serde_json::json!({"front_id":self.cfg.front_id,
                "channel_id":share.channel_id,"job_id":share.job_id,"active_job_id":job.job_id,"reason":"stale-share",
                "generation":job.work.generation.to_string(),"nonce":share.nonce,"ntime":share.ntime,"share_index":share.share_index}),
            );
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
        if share.extranonce.len() != self.cfg.extranonce_len {
            return Err(FrontError::ExtranonceLength);
        }
        #[cfg(test)]
        let revalidate_headers = self.revalidate_headers;
        #[cfg(not(test))]
        let revalidate_headers = true;
        if revalidate_headers {
            revalidate_share(&job, &share)?;
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
            d: job.work.m_issue,
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
                self.roll_slice()?;
                self.open_slice(self.height, template.prev_hash, template.fees);
                let slice = self.open.as_mut().ok_or(FrontError::NoSlice)?;
                if self.share_policy.version() == WorkUnitVersion::V2 {
                    slice.push_v2(rec)?;
                } else {
                    slice.push(rec);
                }
            }
            spc_accounting::Seal::Keep => {
                if self.share_policy.version() == WorkUnitVersion::V2 {
                    slice.push_v2(rec)?;
                } else {
                    slice.push(rec);
                }
            }
        }
        let accepted_slice = self.open.as_ref().ok_or(FrontError::NoSlice)?;
        let credited = accepted_slice.shares.last().ok_or(FrontError::NoSlice)?;
        crate::evidence::accounted(
            self.cfg.front_id,
            share.channel_id,
            &job,
            &share,
            accepted_slice.slice_id,
            credited.job_fees,
        );

        // Block detection is consensus work, not pool work: it always uses the
        // network target, never the (easier) share target.
        let hash = BlockHash::from_byte_array(share.share_hash);
        if !network_target(template.bits).is_met_by(hash) {
            return Ok(None);
        }

        // The miner hashed its own version (rolled within the BIP 320 mask),
        // so the block must carry that exact version. A witness-commitment
        // template injects the reserved value into the coinbase here;
        // `assemble_witness` strict-decodes everything and re-verifies the
        // commitment against the actual transaction set before returning.
        let block = match &self.witness_plan {
            Some(plan) => match assemble_witness(
                share.version,
                template.prev_hash,
                share.ntime,
                template.bits,
                share.nonce,
                &share.coinbase,
                &plan.reserved,
                &template.transactions,
            ) {
                Ok(assembled) => assembled.into(),
                Err(error) => {
                    // The share above is already credited and its evidence
                    // already recorded: the work was real and the accounting
                    // truth keeps it. The block, however, cannot be assembled
                    // (and is therefore never submitted) - that must be loud,
                    // never silent. Bounded context only: no identity field
                    // beyond the front's own ids.
                    tracing::error!(
                        front_id = self.cfg.front_id,
                        height = template.height,
                        job_id = %job.job_id,
                        error = %error,
                        "witness block assembly failed after the share was \
                         credited; the credit stands in the slice and the \
                         evidence, no block event is emitted and no block is \
                         submitted"
                    );
                    return Err(FrontError::Block(error));
                }
            },
            None => assemble(
                share.version,
                template.prev_hash,
                share.ntime,
                template.bits,
                share.nonce,
                &share.coinbase,
                &template.transactions,
            ),
        };
        if let Some(dist) = &self.distribution {
            let block_hash = bitcoin::hashes::sha256d::Hash::hash(&block.header).to_byte_array();
            let entries = dist
                .entries
                .iter()
                .map(|e| WireEntry {
                    owed_sats: e.owed_sats,
                    sp_id: e.sp_id,
                    rail: match e.rail {
                        spc_accounting::Rail::Coinbase => WireRail::Coinbase,
                        spc_accounting::Rail::Ark => WireRail::Ark,
                        spc_accounting::Rail::Pending => WireRail::Pending,
                        spc_accounting::Rail::Forfeit => WireRail::Forfeit,
                    },
                    p_x: e.p_x.clone(),
                    dest_id: e.dest_id.clone().unwrap_or_default(),
                    sats: e.sats,
                })
                .collect();
            let high_water = BTreeMap::from([(self.cfg.front_id, self.high_water.to_string())]);
            self.events.push(FrontEvent::Block(BlockEvent {
                economic: self.frozen_economic.clone(),
                block_hash,
                height: template.height,
                front_id: self.cfg.front_id,
                fingerprint: dist.fingerprint,
                a_send_x: dist.a_send_x,
                high_water,
                // The exact target this distribution's window was selected
                // with (frozen at `refresh` time), not a re-derivation.
                work_unit_version: job.work.work_unit_version,
                work_unit_fingerprint: job.work.work_unit_fingerprint,
                window_rule: self.share_policy.window_rule(),
                window_target: self.window_target,
                entries,
            }));
        }
        self.last_block = Some(block.clone());
        if self.economic_source.is_some() {
            self.awaiting_found = Some(template.height);
            // Tentative same-front reservation after exact issued-coinbase
            // validation. No descendant work until payout validates canonicality
            // and acknowledges the complete durable set at the new parent.
            if let Some(routing) = &self.routing {
                let mut consumes = Vec::new();
                for (id, delta) in &routing.pending_deltas {
                    let consumed = if *delta < 0 { delta.unsigned_abs() } else { 0 };
                    if consumed == 0 {
                        continue;
                    }
                    let available = self
                        .pending
                        .get(id)
                        .copied()
                        .unwrap_or(0)
                        .checked_sub(consumed)
                        .ok_or_else(|| {
                            FrontError::AccountingUnavailable(
                                "local reservation exceeds available pending".into(),
                            )
                        })?;
                    self.pending.insert(*id, available);
                    consumes.push((hex::encode(id), consumed));
                }
                crate::evidence::emit(
                    "provisional_candidate",
                    serde_json::json!({"height":template.height,"job_id":job.job_id,
                    "generation":job.work.generation.to_string(),"net_debits":consumes,"canonical_ack_required":true}),
                );
            }
            if let Some(view) = &mut self.economic_view {
                view.complete = false;
            }
        }
        Ok(Some(block))
    }

    /// Sealed slices, newest first, for the window and type-32 service.
    pub fn sealed_slices(&self) -> &[SliceAgg] {
        &self.slices
    }

    fn roll_slice(&mut self) -> Result<(), FrontError> {
        if let Some(mut open) = self.open.take() {
            if !open.shares.is_empty() {
                open.seal();
                let aggregate = match self.share_policy.version() {
                    WorkUnitVersion::V1 => open.agg(),
                    WorkUnitVersion::V2 => open.agg_v2()?,
                };
                let credits = aggregate
                    .per_id
                    .iter()
                    .map(|(id, (d, df))| spc_store::wire::SliceCredit {
                        sp_id: *id,
                        diff_sum: *d,
                        diff_fee_sum: *df,
                    })
                    .collect();
                if self.type32_enabled && aggregate.work_unit_version == WorkUnitVersion::V1 {
                    if let Ok(mut store) = self.type32.lock() {
                        store.add(&open);
                    }
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
                    work_unit_version: aggregate.work_unit_version,
                    work_unit_fingerprint: aggregate.work_unit_fingerprint,
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
                self.slices.insert(0, aggregate);
                self.trim_retained_slices(self.height);
            }
        }
        Ok(())
    }

    /// Keep every eligible v2 slice, independently of current or previous Q.
    /// The slice hard cap and the same age cutoff as durable scrub (`height <
    /// tip - 4096`) are the only limits. This headroom lets a later, harder
    /// window use previously sealed work rather than discarding it at an
    /// earlier, smaller Q. `select_window_v2` still selects at current Q and
    /// truthfully derives underfill when either real bound leaves too little.
    fn trim_retained_slices(&mut self, tip_height: u32) {
        if self.share_policy.version() == WorkUnitVersion::V1 {
            self.slices.truncate(self.cfg.keep_slices.max(1));
            return;
        }
        let oldest_eligible = tip_height.saturating_sub(V2_RETENTION_HEIGHT_BLOCKS);
        self.slices.retain(|slice| slice.height >= oldest_eligible);
        self.slices.truncate(V2_RETAINED_SLICES_LIMIT);
    }

    /// Open a fresh slice for `height`, assigning the next restart-safe id and
    /// recording the job it was opened under as `(H << 32) | mining_job_id`
    /// (SPEC §3.4).
    fn open_slice(&mut self, height: u32, prev_hash: [u8; 32], fees: u64) {
        let slice_id = self.next_slice_id();
        let ref_job_id = (u64::from(height) << 32) | u64::from(self.job_seq);
        self.open = Some(match &self.share_policy {
            SharePolicy::V1 { .. } => Slice::new(
                slice_id,
                height,
                prev_hash,
                self.cfg.front_id,
                ref_job_id,
                fees,
            ),
            SharePolicy::V2 { .. } => Slice::new_v2(
                slice_id,
                height,
                prev_hash,
                self.cfg.front_id,
                ref_job_id,
                fees,
                self.share_policy.fingerprint().expect("v2 fingerprint"),
            ),
        });
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

    /// The BIP 141 witness reserved value for `wtxid_root`. BIP 141 leaves
    /// the reserved value unconstrained (any 32 bytes commit fine); the
    /// pool draws it from the OS CSPRNG once per witness root and caches
    /// it, a pool-side share-stability choice rather than a protocol
    /// obligation. Republicating an unchanged transaction set keeps the
    /// commitment, so the shares already hashed against that coinbase stay
    /// valid, while a new witness set gets fresh randomness.
    fn reserved_for(&mut self, wtxid_root: [u8; 32]) -> [u8; 32] {
        if let Some((root, reserved)) = self.reserved_cache {
            if root == wtxid_root {
                return reserved;
            }
        }
        use rand::RngCore;
        let mut reserved = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut reserved);
        self.reserved_cache = Some((wtxid_root, reserved));
        reserved
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
        spc_accounting::route_tiers(raw, &self.pending, &self.ark, &tier, total)
            .expect("valid tier config")
    }
}

/// Hash registration-sensitive entry data in production order. Coinbase
/// scripts/amounts remain covered by `Distribution::fingerprint` independently.
fn entries_fingerprint(distribution: &Distribution) -> [u8; 32] {
    let mut engine = sha256::Hash::engine();
    engine.input(b"SPc/JobEntriesV2");
    for entry in &distribution.entries {
        engine.input(&entry.sp_id);
        engine.input(&[entry.rail as u8]);
        engine.input(&(entry.p_x.len() as u16).to_le_bytes());
        for key in &entry.p_x {
            engine.input(key);
        }
        engine.input(&entry.sats.to_le_bytes());
        engine.input(&entry.owed_sats.to_le_bytes());
        let dest = entry.dest_id.as_deref().unwrap_or_default();
        engine.input(&(dest.len() as u32).to_le_bytes());
        engine.input(dest);
    }
    sha256::Hash::from_engine(engine).to_byte_array()
}

/// A different observed window is a different immutable job even when one
/// miner's unchanged relative score produces identical outputs and full owed.
fn window_fingerprint(window: &spc_accounting::Window, target: u128, rule: u8) -> [u8; 32] {
    let mut hash = sha256::Hash::engine();
    hash.input(b"SPc/JobWindowV1");
    hash.input(&target.to_le_bytes());
    hash.input(&[rule]);
    hash.input(&(window.slices.len() as u32).to_le_bytes());
    for slice in &window.slices {
        hash.input(&slice.slice_id.to_le_bytes());
        hash.input(&[slice.front_id]);
        hash.input(&slice.root);
        hash.input(&slice.difficulty.to_le_bytes());
        hash.input(&slice.diff_fee_sum.to_le_bytes());
        hash.input(&[slice.work_unit_version.as_u8()]);
        hash.input(&slice.work_unit_fingerprint.unwrap_or([0; 32]));
        hash.input(&(slice.per_id.len() as u32).to_le_bytes());
        for (id, (work, fee_work)) in &slice.per_id {
            hash.input(id);
            hash.input(&work.to_le_bytes());
            hash.input(&fee_work.to_le_bytes());
        }
    }
    sha256::Hash::from_engine(hash).to_byte_array()
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
        routing.ark_due.retain(|(ark_id, _)| *ark_id != id);
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
        if row.work_unit_version == WorkUnitVersion::V2 {
            difficulty = spc_accounting::work_unit::checked_add_window_work(difficulty, d).ok()?;
            diff_fee_sum =
                spc_accounting::work_unit::checked_add_fee_work(diff_fee_sum, df).ok()?;
        } else {
            difficulty = difficulty.saturating_add(d);
            diff_fee_sum = diff_fee_sum.saturating_add(df);
        }
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
        work_unit_version: row.work_unit_version,
        work_unit_fingerprint: row.work_unit_fingerprint,
    })
}

fn revalidate_share(job: &Job, share: &Share) -> Result<(), FrontError> {
    let mut coinbase = job.coinb1.clone();
    coinbase.extend_from_slice(&share.extranonce);
    coinbase.extend_from_slice(&job.coinb2);
    if coinbase != share.coinbase {
        return Err(FrontError::InvalidShare);
    }
    let mut merkle_root = crate::block::txid(&coinbase);
    for sibling in &job.merkle_branches {
        let mut pair = Vec::with_capacity(64);
        pair.extend_from_slice(&merkle_root);
        pair.extend_from_slice(sibling);
        merkle_root = crate::block::txid(&pair);
    }
    if merkle_root != share.merkle_root {
        return Err(FrontError::InvalidShare);
    }
    let mut header = Vec::with_capacity(80);
    header.extend_from_slice(&share.version.to_le_bytes());
    header.extend_from_slice(&job.prev_hash);
    header.extend_from_slice(&merkle_root);
    header.extend_from_slice(&share.ntime.to_le_bytes());
    header.extend_from_slice(&job.nbits.to_le_bytes());
    header.extend_from_slice(&share.nonce.to_le_bytes());
    if crate::block::txid(&header) != share.share_hash {
        return Err(FrontError::InvalidShare);
    }
    Ok(())
}

fn parse_job_id(s: &str) -> u64 {
    u64::from_str_radix(s, 16).unwrap_or(0)
}

#[cfg(test)]
use spc_accounting::WindowRule;

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
authority_public_key = "9auqWEzQDVyd2oe1JVGFLMLHZtCo2FFqZwtKA5gd9xbuEu7PH72"
authority_secret_key = "mkDLTBBRxdBv998612qipDYoTK3YUrqLe8uWw7gu3iXbSrn2n"
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

    fn observation_registration(front: &mut Front, seed: u8) -> SpId {
        let id = id_for(seed);
        let (scan, spend) = key_pair(seed);
        let server = bitcoin::secp256k1::PublicKey::from_slice(&key_pair(9).0).unwrap();
        front.set_ark_server(ArkServerMetadata {
            ark_id: <[u8; 4]>::from(ark::address::ArkId::from_server_pubkey(server)),
            server_pubkey: server,
            mailbox_pubkey: bitcoin::secp256k1::PublicKey::from_slice(&key_pair(10).0).unwrap(),
            testnet: true,
        });
        front.learn_identity(id, scan, spend).unwrap();
        front.ark_registrations.insert(
            id,
            ArkDeliveryRow {
                registration_commit: Some(Box::new(spc_store::models::RegistrationCommit {
                    request_id: 1,
                    registration_rev: 2,
                    signed_fields_digest: Some(hex::encode([8; 32])),
                })),
                sp_id: id,
                ark_id: [3; 4],
                b_ark: spend.to_vec(),
                mailbox_mode: spc_store::models::MailboxMode::EpochRegistered,
                mailbox_m: Some(scan.to_vec()),
                mailbox_d: Some(spend.to_vec()),
                epoch_fallback: spc_store::models::EpochFallback::Hold,
            },
        );
        front.set_ark(
            id,
            ArkCapability {
                registered: true,
                registration_committed: true,
                dest_known: false,
            },
        );
        id
    }

    #[tokio::test]
    async fn actual_derivation_observes_epoch_hold_missing_keys_and_public_destination() {
        let mut front = Front::new(
            cfg(),
            Arc::new(MockTemplateSource::new(template())),
            type32(),
        );
        front.refresh().await.unwrap();
        let id = observation_registration(&mut front, 0x51);
        let rows = front.refresh_ark_destinations(&[1; 16]).await.unwrap();
        assert_eq!(rows[0]["outcome"], "epoch_mailbox_held");
        assert_eq!(rows[0]["mailbox_selection"], "hold_missing_epoch");
        assert!(rows[0]["destination_bytes"].is_null());
        front.ark_registrations.get_mut(&id).unwrap().epoch_fallback =
            spc_store::models::EpochFallback::Static;
        let rows = front.refresh_ark_destinations(&[2; 16]).await.unwrap();
        assert_eq!(rows[0]["outcome"], "derived");
        assert_eq!(rows[0]["mailbox_selection"], "static_fallback");
        assert_eq!(
            hex::decode(rows[0]["destination_bytes"].as_str().unwrap()).unwrap(),
            front.ark_dest[&id]
        );
        assert!(
            !serde_json::to_string(&rows)
                .unwrap()
                .contains(&hex::encode([0x51; 32])),
            "private scan scalar never serialized"
        );
        front.miners.remove(&id);
        let rows = front.refresh_ark_destinations(&[3; 16]).await.unwrap();
        assert_eq!(rows[0]["outcome"], "missing_miner_keys");
        assert_eq!(rows[0]["coinbase_key_status"], "missing_miner_keys");
        assert!(!front.ark_dest.contains_key(&id));
    }

    #[cfg(feature = "postgres-integration")]
    #[tokio::test]
    async fn actual_derivation_failure_persists_public_inputs_and_classification() {
        let url =
            std::env::var("SPC_LADDER_TEST_DATABASE_URL").expect("owned PG required; no skip");
        assert!(url.contains("ladder_completion"));
        let admin = spc_store::Store::connect(&url).await.unwrap();
        let name = format!("ladder_completion_derive_{}", uuid::Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE DATABASE {name}"))
            .execute(admin.pool())
            .await
            .unwrap();
        let store =
            spc_store::Store::connect(&url.replace("/ladder_completion?", &format!("/{name}?")))
                .await
                .unwrap();
        store.migrate().await.unwrap();
        store.economic_initialize([0x11; 32]).await.unwrap();
        let mut front = Front::new(
            cfg(),
            Arc::new(MockTemplateSource::new(template_at(1, 0x1c00_ffff, 0))),
            type32(),
        );
        front.refresh().await.unwrap();
        let id = observation_registration(&mut front, 0x51);
        front.set_economic_source(store.clone());
        let redis =
            std::env::var("SPC_LADDER_TEST_REDIS_URL").expect("owned Redis required; no skip");
        let streams = spc_store::redis::Streams::connect(&redis).await.unwrap();
        let writer =
            crate::payout_observation::spawn(store.clone(), streams.independent().await.unwrap())
                .await
                .unwrap();
        front.set_observation_streams(streams.independent().await.unwrap());
        front.ark_registrations.get_mut(&id).unwrap().b_ark = vec![0; 33];
        assert!(front.refresh_ark_destinations(&[4; 16]).await.is_err());
        let body: serde_json::Value = sqlx::query_scalar(
            "SELECT body FROM source_decision_observation WHERE kind='front_derivation'",
        )
        .fetch_one(store.pool())
        .await
        .unwrap();
        assert_eq!(body["status"], "failed_unadvertised_attempt");
        assert_eq!(body["census_complete"], false);
        assert_eq!(body["rows"][0]["error_stage"], "ark_destination");
        assert!(body["rows"][0]["error_reason"]
            .as_str()
            .unwrap()
            .contains("invalid B_ark registration"));
        assert!(body["job_id"].is_null());
        assert!(body["generation"].is_null());
        assert!(!body.to_string().contains(&hex::encode([0x51; 32])));
        front.ark_registrations.get_mut(&id).unwrap().b_ark = key_pair(0x51).1.to_vec();
        let job = front.refresh().await.unwrap();
        let accepted:serde_json::Value=sqlx::query_scalar("SELECT body FROM source_decision_observation WHERE kind='front_derivation' AND body->>'status'='frozen_job'")
            .fetch_one(store.pool()).await.unwrap();
        assert_eq!(accepted["generation"], job.work.generation);
        assert_eq!(accepted["height"], job.height);
        assert_eq!(
            accepted["economic_revision"],
            store.economic_view().await.unwrap().revision
        );
        assert_eq!(accepted["census_complete"], true);
        writer.abort();
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
            work_unit_version: WorkUnitVersion::V1,
            work_unit_fingerprint: None,
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
            economic: None,
            rev: 3,
            applied_blocks: vec![],
            pending_balance: vec![],
            ark_delivery: vec![],
            ark_identities: vec![],
            ark_epoch_mailboxes: vec![],
            window_tail: vec![
                slice_row(9_000, 799_999, &[(miner, 1, 0)]),
                slice_row(8_000, 799_998, &[(other, 1, 0)]),
                slice_row(7_000, 799_997, &[]),
            ],
            type32_blobs: vec![],
            slices_cursor: None,
            front_events_cursor: None,
        };
        front.apply_bootstrap(&snapshot).unwrap();

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
        assert!(!entry.p_x.is_empty());
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

    /// A transaction with a witness stack: the coinbase must carry a BIP 141
    /// commitment output for it.
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

    /// The same transaction as `raw_witness_tx(seed)` with a different
    /// witness stack: same txid (the witness is not hashed), different
    /// wtxid - the witness-malleation vector.
    fn malleated_witness_tx(seed: u8) -> Vec<u8> {
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
                witness: Witness::from_slice(&[vec![seed; 4], vec![seed; 8]]),
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
        for sibling in branch {
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
            c.max_payout_slots = 1024; // both logical rows, complete denomination sets
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
            c.max_payout_slots = 1024;
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
            assert_eq!(kept.len(), 2);
            kept_amounts.push(kept[0]);
        }
        // The window is the same two-share slice every time: one kept id gets
        // `r_net / d_win = r_net / 2`, unchanged by the passing heights.
        assert_eq!(kept_amounts[0], 156_000_000);
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
            c.max_payout_slots = 1024;
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
        assert!(routing.pending.is_empty());
        let (&id, &delta) = routing
            .pending_deltas
            .iter()
            .find(|(_, delta)| **delta > 0)
            .unwrap();
        let trimmed = (id, delta as u64);

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

    #[tokio::test]
    async fn bits_and_version_changes_bump_clean_latest_only_jobs() {
        let source = Arc::new(MutableSource::new(template()));
        let mut front = Front::new(cfg(), source.clone(), type32());
        let initial = front.refresh().await.unwrap();

        for change in ["bits", "version"] {
            let previous = front.job().unwrap().job_id.clone();
            let mut next = template();
            if change == "bits" {
                next.bits = 0x207e_ffff;
            } else {
                next.version = 0x2000_2000;
            }
            source.set(next);
            let republished = front.refresh().await.unwrap();
            assert_ne!(republished.job_id, previous, "{change}-only update");
            assert_eq!(republished.prev_hash, initial.prev_hash);
            assert!(republished.clean_jobs, "latest-only means clean SV1 jobs");
        }
    }

    #[tokio::test]
    async fn ordinary_curtime_refresh_keeps_the_issued_job_immutable() {
        let source = Arc::new(MutableSource::new(template()));
        let mut front = Front::new(cfg(), source.clone(), type32());
        let issued = front.refresh().await.unwrap();
        let mut advanced = template();
        advanced.curtime += 1;
        source.set(advanced);

        let refreshed = front.refresh().await.unwrap();
        assert_eq!(
            refreshed, issued,
            "ordinary GBT time advancement must not replace or reinterpret the issued job"
        );
        assert!(ntime_allowed(refreshed.ntime, refreshed.ntime + 30));
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

    /// A witness transaction is mined, not refused: the job's coinbase
    /// carries the BIP 141 commitment output (last, zero-value), the pool
    /// output stays first, and a block-finding share assembles a full
    /// witness block whose commitment the assembler itself re-verified.
    #[tokio::test]
    async fn witness_templates_get_a_commitment_and_assemble() {
        let mut t = template_at(800_000, 0x207f_ffff, 0);
        t.transactions = vec![raw_legacy_tx(4), raw_witness_tx(9), raw_legacy_tx(6)];
        let source = Arc::new(MutableSource::new(t));
        let mut front = Front::new(cfg(), source, type32());
        let job = front.refresh().await.unwrap();
        assert!(front.job().is_some());

        // The assembled coinbase: pool output first, commitment last.
        let coinbase = coinbase_of(&job, &[0xaa; 8]);
        let tx: bitcoin::Transaction = bitcoin::consensus::deserialize(&coinbase).unwrap();
        assert_eq!(tx.output.len(), 2, "pool output + commitment output");
        assert!(tx.output[0].value.to_sat() > 0);
        assert_eq!(
            tx.output[0].script_pubkey.as_bytes(),
            &[0x51, 0x20]
                .iter()
                .copied()
                .chain([0x99; 32])
                .collect::<Vec<u8>>()
        );
        let (index, found) = crate::witness::coinbase_commitment(
            tx.output.iter().map(|o| o.script_pubkey.as_bytes()),
        )
        .unwrap()
        .expect("exactly one commitment output");
        assert_eq!(index, 1, "the commitment output is last");
        assert_eq!(tx.output[1].value.to_sat(), 0);
        let _ = found;

        // A block-finding share assembles a witness block.
        let mut finding = share(&front, 1);
        finding.coinbase = coinbase;
        finding.share_hash = [0u8; 32]; // meets the regtest network target
        let block = front.on_share(finding).unwrap().expect("block");
        let decoded: bitcoin::Block = bitcoin::consensus::deserialize(&block.block).unwrap();
        assert_eq!(decoded.txdata.len(), 4);
        assert!(!decoded.txdata[0].input[0].witness.is_empty());
        assert_eq!(
            decoded.txdata[0].input[0]
                .witness
                .iter()
                .next()
                .map(|item| item.len()),
            Some(32)
        );
        assert!(!decoded.txdata[2].input[0].witness.is_empty());
        assert!(decoded.txdata[1].input[0].witness.is_empty());
        assert!(decoded.txdata[3].input[0].witness.is_empty());
    }

    /// A witness-free template is byte-for-byte the legacy shape: no
    /// commitment output, no witness in the assembled block.
    #[tokio::test]
    async fn witness_free_templates_carry_no_commitment() {
        let mut t = template_at(800_000, 0x207f_ffff, 0);
        t.transactions = vec![raw_legacy_tx(4)];
        let source = Arc::new(MutableSource::new(t));
        let mut front = Front::new(cfg(), source, type32());
        let job = front.refresh().await.unwrap();

        let coinbase = coinbase_of(&job, &[0xaa; 8]);
        let tx: bitcoin::Transaction = bitcoin::consensus::deserialize(&coinbase).unwrap();
        assert_eq!(tx.output.len(), 1, "no commitment output");
        assert_eq!(
            crate::witness::coinbase_commitment(
                tx.output.iter().map(|o| o.script_pubkey.as_bytes())
            )
            .unwrap(),
            None
        );

        let mut finding = share(&front, 1);
        finding.coinbase = coinbase;
        finding.share_hash = [0u8; 32];
        let block = front.on_share(finding).unwrap().expect("block");
        let decoded: bitcoin::Block = bitcoin::consensus::deserialize(&block.block).unwrap();
        assert!(decoded.txdata[0].input[0].witness.is_empty());
    }

    /// A same-txid witness malleation changes the commitment, so the job id
    /// moves and shares on the old job are stale - even though the txid
    /// merkle root, fees, height, prev-hash and bits are all unchanged.
    #[tokio::test]
    async fn a_same_txid_witness_malleation_republishes_the_job() {
        let mut t = template_at(800_000, 0x1c00_ffff, 0);
        t.transactions = vec![raw_witness_tx(9)];
        let source = Arc::new(MutableSource::new(t));
        let mut front = Front::new(cfg(), source.clone(), type32());
        front.refresh().await.unwrap();
        let old_job = front.job().unwrap().job_id.clone();

        // Same txid (the witness is not hashed), same fees, same
        // height/prev/bits/version: only the witness bytes differ.
        let mut malleated = template_at(800_000, 0x1c00_ffff, 0);
        malleated.transactions = vec![malleated_witness_tx(9)];
        assert_eq!(
            crate::template::txid_of(&malleated.transactions[0]),
            crate::template::txid_of(&raw_witness_tx(9)),
            "the malleated tx keeps the same txid"
        );
        source.set(malleated);
        let refreshed = front.refresh().await.unwrap();
        assert_ne!(refreshed.job_id, old_job, "the job id must move");
        let new_job = front.job().unwrap().job_id.clone();

        let mut stale = share(&front, 2);
        stale.job_id = old_job;
        assert!(matches!(front.on_share(stale), Err(FrontError::StaleJob)));
        // ... while work on the new commitment is accepted.
        let mut fresh = share(&front, 3);
        fresh.coinbase = coinbase_of(front.job().unwrap(), &[0xaa; 8]);
        front.on_share(fresh).unwrap();
        let _ = new_job;
    }

    /// An unchanged witness transaction set keeps the same commitment across
    /// republications: the reserved value is cached per wtxid root, so a
    /// share hashed before the refresh still meets the coinbase.
    #[tokio::test]
    async fn an_unchanged_witness_set_keeps_the_commitment() {
        let mut t = template_at(800_000, 0x1c00_ffff, 5);
        t.transactions = vec![raw_witness_tx(9)];
        let source = Arc::new(MutableSource::new(t.clone()));
        let mut front = Front::new(cfg(), source, type32());
        let first = front.refresh().await.unwrap();
        // Same template again: no job bump, byte-identical coinbase.
        let second = front.refresh().await.unwrap();
        assert_eq!(first.job_id, second.job_id);
        assert_eq!(first.coinb1, second.coinb1);
        assert_eq!(first.coinb2, second.coinb2);
    }

    /// A template transaction that does not strictly decode fails the whole
    /// refresh (fail closed), leaving no job.
    #[tokio::test]
    async fn a_non_canonical_template_transaction_fails_the_refresh() {
        let mut t = template_at(800_000, 0x1c00_ffff, 0);
        let mut bad = raw_witness_tx(9);
        bad.push(0x00); // trailing bytes
        t.transactions = vec![raw_legacy_tx(4), bad];
        let mut front = Front::new(cfg(), Arc::new(MockTemplateSource::new(t)), type32());
        let err = front.refresh().await.unwrap_err();
        assert!(err.to_string().contains("transaction 1"), "{err}");
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
            .filter_map(|e| e.p_x.first().map(|x| (e.sp_id, *x)))
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
    async fn forged_header_fields_and_extranonce_lengths_are_rejected() {
        let source = Arc::new(MockTemplateSource::new(template()));
        let mut front = Front::new(cfg(), source, type32());
        front.refresh().await.unwrap();
        front.revalidate_headers = true;
        let mut forged = share(&front, 1);
        forged.share_hash = [0u8; 32];
        assert!(matches!(
            front.on_share(forged),
            Err(FrontError::InvalidShare)
        ));

        let mut wrong_length = share(&front, 2);
        wrong_length.extranonce.pop();
        assert!(matches!(
            front.on_share(wrong_length),
            Err(FrontError::ExtranonceLength)
        ));
    }

    #[tokio::test]
    async fn a_fully_reconstructed_valid_header_is_accepted() {
        let source = Arc::new(MockTemplateSource::new(template()));
        let mut front = Front::new(cfg(), source, type32());
        front.refresh().await.unwrap();
        front.revalidate_headers = true;
        let job = front.job().unwrap().clone();

        let mut accepted = None;
        for nonce in 0..64 {
            let mut candidate = share(&front, nonce);
            let coinbase = coinbase_of(&job, &candidate.extranonce);
            let root = miner_merkle_root(&coinbase, &job.merkle_branches);
            let mut header = Vec::with_capacity(80);
            header.extend_from_slice(&job.version.to_le_bytes());
            header.extend_from_slice(&job.prev_hash);
            header.extend_from_slice(&root);
            header.extend_from_slice(&job.ntime.to_le_bytes());
            header.extend_from_slice(&job.nbits.to_le_bytes());
            header.extend_from_slice(&nonce.to_le_bytes());
            let hash = crate::block::txid(&header);
            candidate.coinbase = coinbase;
            candidate.merkle_root = root;
            candidate.share_hash = hash;
            if network_target(job.nbits).is_met_by(BlockHash::from_byte_array(hash)) {
                accepted = Some(candidate);
                break;
            }
        }
        front.on_share(accepted.expect("regtest nonce")).unwrap();
    }

    fn fully_hashed_v2_share(
        front: &Front,
        start_nonce: u32,
        block_qualified: bool,
    ) -> crate::sv1::Share {
        let job = front.job().unwrap().clone();
        let share_target = job.work.bitcoin_share_target();
        let network = network_target(job.nbits);
        for nonce in start_nonce..start_nonce + 4_096 {
            let extranonce = vec![0xaa; 8];
            let coinbase = coinbase_of(&job, &extranonce);
            let root = miner_merkle_root(&coinbase, &job.merkle_branches);
            let mut header = Vec::with_capacity(80);
            header.extend_from_slice(&job.version.to_le_bytes());
            header.extend_from_slice(&job.prev_hash);
            header.extend_from_slice(&root);
            header.extend_from_slice(&job.ntime.to_le_bytes());
            header.extend_from_slice(&job.nbits.to_le_bytes());
            header.extend_from_slice(&nonce.to_le_bytes());
            let hash = crate::block::txid(&header);
            let is_block = network.is_met_by(BlockHash::from_byte_array(hash));
            if share_target.is_met_by(BlockHash::from_byte_array(hash))
                && is_block == block_qualified
            {
                let mut share = share_for(front, 0x11, nonce);
                share.job_id = job.job_id.clone();
                share.nonce = nonce;
                share.ntime = job.ntime;
                share.version = job.version;
                share.extranonce = extranonce;
                share.job_fees = 0;
                share.coinbase = coinbase;
                share.merkle_root = root;
                share.share_hash = hash;
                return share;
            }
        }
        panic!(
            "no {} nonce in 4096 attempts",
            if block_qualified {
                "block"
            } else {
                "non-block"
            }
        );
    }

    #[tokio::test]
    async fn v2_genuine_non_block_shares_credit_their_generation_and_freeze_q() {
        let source = Arc::new(MutableSource::new(template_at(800_000, 0x200f_ffff, 0)));
        let cfg = cfg_with(cfg(), |c| {
            c.window_n = 8;
            c.max_slice_shares = 1;
        });
        let mut front = Front::new(cfg, source, type32());
        front.set_share_policy(SharePolicy::V2 {
            profile: spc_accounting::WorkUnitProfile::lab_default(),
            j_cfg: 1,
            max_window_target_units: 1u128 << 64,
        });
        front.refresh().await.unwrap();
        front.revalidate_headers = true;
        assert_eq!(front.job().unwrap().work.m_issue, 2);

        let first = fully_hashed_v2_share(&front, 0, false);
        assert!(
            front.on_share(first).unwrap().is_none(),
            "ordinary non-block share"
        );
        let second = fully_hashed_v2_share(&front, 100, false);
        assert!(front.on_share(second).unwrap().is_none());
        let slices = slice_events(&mut front);
        assert_eq!(slices.len(), 1);
        assert_eq!(slices[0].credits[0].diff_sum, 2);
        assert_eq!(slices[0].work_unit_version, WorkUnitVersion::V2);
        assert_eq!(
            slices[0].work_unit_fingerprint,
            front.share_policy().fingerprint()
        );

        let block = fully_hashed_v2_share(&front, 200, true);
        front
            .on_share(block)
            .unwrap()
            .expect("block-qualified share");
        let blocks = block_events(&mut front);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].window_target, 65);
        assert_eq!(blocks[0].window_rule, WindowRule::NetworkBlockWork);
        assert_eq!(blocks[0].work_unit_version, WorkUnitVersion::V2);
        assert!(
            front.window_underfilled(),
            "two m=2 shares cannot reach Q=65"
        );
    }

    #[tokio::test]
    async fn failed_retarget_never_admits_old_generation_work_on_the_new_tip() {
        let source = Arc::new(MutableSource::new(template_at(800_000, 0x200f_ffff, 0)));
        let mut front = Front::new(cfg(), source.clone(), type32());
        front.set_share_policy(SharePolicy::V2 {
            profile: spc_accounting::WorkUnitProfile::from_exponent(254, 1u64 << 32).unwrap(),
            j_cfg: 1,
            max_window_target_units: 1u128 << 64,
        });
        front.refresh().await.unwrap();
        let old_share = share_for(&front, 0x11, 1);

        let mut invalid = template_at(800_001, 0x207f_ffff, 0);
        invalid.prev_hash = [0x22; 32];
        source.set(invalid);
        let error = front.refresh().await.unwrap_err();
        assert!(matches!(
            error,
            FrontError::WorkUnit(spc_accounting::WorkUnitError::TargetProfileUnplannable)
        ));
        assert!(
            front.job().is_none(),
            "failed refresh must invalidate the old generation"
        );
        assert!(matches!(front.on_share(old_share), Err(FrontError::NoJob)));
        assert!(slice_events(&mut front)
            .iter()
            .all(|slice| slice.height != 800_001 || slice.n_shares == 0));
    }

    #[tokio::test]
    async fn v2_roll_retention_keeps_enough_low_work_slices_for_q() {
        let source = Arc::new(MutableSource::new(template_at(800_000, 0x200f_ffff, 0)));
        let cfg = cfg_with(cfg(), |c| {
            c.max_slice_shares = 1;
            c.keep_slices = 16;
        });
        let audit = type32();
        let mut front = Front::new(cfg, source.clone(), audit.clone());
        front.set_share_policy(SharePolicy::V2 {
            profile: spc_accounting::WorkUnitProfile::lab_default(),
            j_cfg: 1,
            max_window_target_units: 1u128 << 64,
        });
        front.set_type32_enabled(false);
        front.refresh().await.unwrap();
        for i in 0..40u32 {
            let share = share_for(&front, 0x11, i + 1);
            front.on_share(share).unwrap();
            source.set(template_at(800_001 + i, 0x200f_ffff, 0));
            front.refresh().await.unwrap();
        }
        let work: u128 = front.sealed_slices().iter().map(|s| s.difficulty).sum();
        assert!(
            front.sealed_slices().len() > 16,
            "v2 retained only {} slices with {work} work",
            front.sealed_slices().len()
        );
        assert!(
            work >= 65,
            "Q=65 requires more than the legacy 16-slice tail"
        );
        assert!(
            !front.window_underfilled(),
            "accepted historical work must not be hidden by retention"
        );
        assert!(
            audit.lock().unwrap().window().is_empty(),
            "v2 never populates Type32Store"
        );
    }

    #[tokio::test]
    async fn v2_bootstrap_retention_restores_enough_low_work_slices_for_q() {
        let fingerprint = spc_accounting::WorkUnitProfile::lab_default().fingerprint();
        let window_tail: Vec<SliceRow> = (1..=40u64)
            .map(|slice_id| {
                let mut row = slice_row(slice_id, 799_000, &[(id_for(0x11), 2, 0)]);
                row.work_unit_version = WorkUnitVersion::V2;
                row.work_unit_fingerprint = Some(fingerprint);
                row
            })
            .collect();
        let snapshot = BootstrapSnapshot {
            economic: None,
            rev: 1,
            applied_blocks: vec![],
            pending_balance: vec![],
            ark_delivery: vec![],
            ark_identities: vec![],
            ark_epoch_mailboxes: vec![],
            window_tail,
            type32_blobs: vec![],
            slices_cursor: None,
            front_events_cursor: None,
        };
        let source = Arc::new(MutableSource::new(template_at(800_000, 0x200f_ffff, 0)));
        let mut front = Front::new(cfg(), source, type32());
        front.set_share_policy(SharePolicy::V2 {
            profile: spc_accounting::WorkUnitProfile::lab_default(),
            j_cfg: 1,
            max_window_target_units: 1u128 << 64,
        });
        front.apply_bootstrap(&snapshot).unwrap();
        front.refresh().await.unwrap();
        let work: u128 = front.sealed_slices().iter().map(|s| s.difficulty).sum();
        assert!(front.sealed_slices().len() > 16);
        assert!(work >= 65);
        assert!(!front.window_underfilled());
    }

    fn retarget_snapshot(rows: Vec<SliceRow>) -> BootstrapSnapshot {
        BootstrapSnapshot {
            economic: None,
            rev: 1,
            applied_blocks: vec![],
            pending_balance: vec![],
            ark_delivery: vec![],
            ark_identities: vec![],
            ark_epoch_mailboxes: vec![],
            window_tail: rows,
            type32_blobs: vec![],
            slices_cursor: None,
            front_events_cursor: None,
        }
    }

    fn set_lab_v2(front: &mut Front) {
        front.set_share_policy(SharePolicy::V2 {
            profile: spc_accounting::WorkUnitProfile::lab_default(),
            j_cfg: 0,
            max_window_target_units: 1u128 << 64,
        });
    }

    #[tokio::test]
    async fn v2_roll_retention_preserves_headroom_for_q9_to_q65_retarget() {
        let source = Arc::new(MutableSource::new(template_at(800_000, 0x207f_ffff, 0)));
        let mut front = Front::new(cfg(), source.clone(), type32());
        set_lab_v2(&mut front);
        front.refresh().await.unwrap();
        for i in 0..70u32 {
            front.on_share(share_for(&front, 0x11, i + 1)).unwrap();
            source.set(template_at(800_001 + i, 0x207f_ffff, 0));
            front.refresh().await.unwrap();
        }
        source.set(template_at(800_070, 0x200f_ffff, 0));
        front.refresh().await.unwrap();
        let work: u128 = front.sealed_slices().iter().map(|s| s.difficulty).sum();
        assert!(
            front.sealed_slices().len() >= 65,
            "only {} slices remained",
            front.sealed_slices().len()
        );
        assert!(
            work >= 65,
            "the Q=65 retarget needs d=1 history beyond the small Q=9 budget"
        );
        assert!(
            !front.window_underfilled(),
            "future harder windows need the retained headroom"
        );
    }

    #[tokio::test]
    async fn v2_bootstrap_retention_preserves_headroom_for_q9_to_q65_retarget() {
        let fp = spc_accounting::WorkUnitProfile::lab_default().fingerprint();
        let rows = (1..=70u64)
            .map(|slice_id| {
                let mut row = slice_row(slice_id, 799_999, &[(id_for(0x11), 1, 0)]);
                row.work_unit_version = WorkUnitVersion::V2;
                row.work_unit_fingerprint = Some(fp);
                row
            })
            .collect();
        let source = Arc::new(MutableSource::new(template_at(800_000, 0x207f_ffff, 0)));
        let mut front = Front::new(cfg(), source.clone(), type32());
        set_lab_v2(&mut front);
        front.apply_bootstrap(&retarget_snapshot(rows)).unwrap();
        front.refresh().await.unwrap();
        source.set(template_at(800_001, 0x200f_ffff, 0));
        front.refresh().await.unwrap();
        let work: u128 = front.sealed_slices().iter().map(|s| s.difficulty).sum();
        assert!(front.sealed_slices().len() >= 65);
        assert!(work >= 65);
        assert!(!front.window_underfilled());
    }

    #[tokio::test]
    async fn v2_age_horizon_excludes_expired_credits_and_reports_underfill() {
        let fp = spc_accounting::WorkUnitProfile::lab_default().fingerprint();
        let mut rows: Vec<SliceRow> = (1..=70u64)
            .map(|slice_id| {
                let mut row = slice_row(slice_id, 100, &[(id_for(0x11), 1, 0)]);
                row.work_unit_version = WorkUnitVersion::V2;
                row.work_unit_fingerprint = Some(fp);
                row
            })
            .collect();
        let mut recent = slice_row(71, 4_999, &[(id_for(0x11), 1, 0)]);
        recent.work_unit_version = WorkUnitVersion::V2;
        recent.work_unit_fingerprint = Some(fp);
        rows.push(recent);
        let source = Arc::new(MutableSource::new(template_at(5_000, 0x207f_ffff, 0)));
        let mut front = Front::new(cfg(), source, type32());
        set_lab_v2(&mut front);
        front.apply_bootstrap(&retarget_snapshot(rows)).unwrap();
        front.refresh().await.unwrap();
        let work: u128 = front.sealed_slices().iter().map(|s| s.difficulty).sum();
        assert_eq!(
            front.sealed_slices().len(),
            1,
            "history older than tip-4096 must be unusable in RAM too"
        );
        assert_eq!(work, 1);
        assert!(
            front.window_underfilled(),
            "expired history cannot make a q=9 window look full"
        );
    }

    #[tokio::test]
    async fn v2_slice_cap_is_bounded_and_history_exhaustion_is_truthful() {
        let fp = spc_accounting::WorkUnitProfile::lab_default().fingerprint();
        let rows = (1..=4_097u64)
            .map(|slice_id| {
                let mut row = slice_row(slice_id, 799_999, &[(id_for(0x11), 1, 0)]);
                row.work_unit_version = WorkUnitVersion::V2;
                row.work_unit_fingerprint = Some(fp);
                row
            })
            .collect();
        let source = Arc::new(MutableSource::new(template_at(800_000, 0x207f_ffff, 0)));
        let mut front = Front::new(cfg(), source, type32());
        set_lab_v2(&mut front);
        front.apply_bootstrap(&retarget_snapshot(rows)).unwrap();
        front.refresh().await.unwrap();
        assert_eq!(front.sealed_slices().len(), V2_RETAINED_SLICES_LIMIT);
        let beyond_cap = select_window_v2(
            front.sealed_slices(),
            u64::MAX,
            4_097,
            BTreeMap::new(),
            Some(fp),
        )
        .unwrap();
        assert!(
            beyond_cap.underfilled(4_097),
            "a real cap/age limit must not be labelled full coverage"
        );
    }

    #[tokio::test]
    async fn v2_bootstrap_mismatch_fails_closed_without_zero_fill() {
        let source = Arc::new(MockTemplateSource::new(template()));
        let mut front = Front::new(cfg(), source, type32());
        front.set_share_policy(SharePolicy::V2 {
            profile: spc_accounting::WorkUnitProfile::lab_default(),
            j_cfg: 1,
            max_window_target_units: 1u128 << 64,
        });
        let snapshot = BootstrapSnapshot {
            economic: None,
            rev: 1,
            applied_blocks: vec![],
            pending_balance: vec![],
            ark_delivery: vec![],
            ark_identities: vec![],
            ark_epoch_mailboxes: vec![],
            window_tail: vec![slice_row(1, 800_000, &[(id_for(0x11), 1, 0)])],
            type32_blobs: vec![],
            slices_cursor: None,
            front_events_cursor: None,
        };
        assert!(matches!(
            front.apply_bootstrap(&snapshot),
            Err(FrontError::WorkUnitRestoreMismatch { slice_id: 1, .. })
        ));
        assert!(front.sealed_slices().is_empty());
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

    /// SPEC §4.6: an unregistered kept remainder is owed in the next
    /// distribution. The liability comes from the payout role's `LedgerApplied`
    /// delta, not from the front's forecast.
    #[tokio::test]
    async fn ledger_remainders_carry_full_entitlement_and_reject_over_revenue() {
        let fees = 0;
        let bits = 0x1c00_ffff; // network difficulty 256: shares are not blocks
        let source = Arc::new(MutableSource::new(template_at(800_000, bits, fees)));
        let cfg = cfg_with(cfg(), |c| {
            c.fee_bps = 0;
            c.window_n = 1;
            c.max_payout_slots = 1024;
            c.max_slice_shares = 65_000;
        });
        let mut front = Front::new(cfg, source.clone(), type32());
        front.refresh().await.unwrap();

        // Two identities, one share each at difficulty 1.
        front.on_share(share_for(&front, 0x21, 1)).unwrap();
        front.on_share(share_for(&front, 0x31, 2)).unwrap();

        // Next height seals the slice: both ids receive their full rungs and
        // keep their remainders. No block has been found, so nothing is booked.
        source.set(template_at(800_001, bits, fees));
        front.refresh().await.unwrap();
        let raw_share = 312_500_000 / 2;
        assert!(front.pending().is_empty(), "a forecast books nothing");

        let routing = front.routing().expect("routing").clone();
        let (&id, &delta) = routing
            .pending_deltas
            .iter()
            .find(|(_, delta)| **delta > 0)
            .unwrap();
        let trimmed = (id, delta as u64);
        assert_eq!(trimmed.1, raw_share % 500_000);

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
        assert_eq!(front.pending().get(&trimmed.0), Some(&trimmed.1));

        // Both next full rung sets together exceed actual T. This is a real
        // coinbase limit, independent of any gross fee/rounding floor.
        source.set(template_at(800_002, bits, fees));
        assert!(matches!(
            front.refresh().await,
            Err(FrontError::Accounting(
                spc_accounting::Error::EntitlementExceedsRevenue
            ))
        ));
        assert_eq!(
            front.pending()[&trimmed.0],
            trimmed.1,
            "failed forecast never consumes liability"
        );
    }

    #[tokio::test]
    async fn positive_prior_empty_partial_full_front_keeps_q_and_repays_without_gross_floor() {
        let who = id_for(0x21);
        let (scan, spend) = key_pair(0x21);
        let profile = spc_accounting::WorkUnitProfile::lab_default();
        for work in [0, 2, 66] {
            let mut template = template_at(101, 0x200f_ffff, 0);
            template.subsidy = 5_000_000_000;
            template.coinbase_value = template.subsidy;
            let mut front = Front::new(
                cfg_with(cfg(), |c| {
                    c.max_payout_slots = 2048;
                    c.budget_wu = 1_000_000;
                }),
                Arc::new(MockTemplateSource::new(template)),
                type32(),
            );
            front.set_share_policy(SharePolicy::V2 {
                profile: profile.clone(),
                j_cfg: 1,
                max_window_target_units: 1u128 << 64,
            });
            let mut window_tail = Vec::new();
            if work > 0 {
                let mut row = slice_row(1, 100, &[(who, work, 0)]);
                row.work_unit_version = WorkUnitVersion::V2;
                row.work_unit_fingerprint = Some(profile.fingerprint());
                window_tail.push(row);
            }
            // Unit fixture of a previously booked liability, not live SQL credit.
            front
                .apply_bootstrap(&BootstrapSnapshot {
                    economic: None,
                    rev: 1,
                    applied_blocks: vec![],
                    pending_balance: vec![(who, 1_000_123)],
                    ark_delivery: vec![],
                    ark_identities: vec![],
                    ark_epoch_mailboxes: vec![],
                    window_tail,
                    type32_blobs: vec![],
                    slices_cursor: None,
                    front_events_cursor: None,
                })
                .unwrap();
            front.learn_identity(who, scan, spend).unwrap();
            front.refresh().await.unwrap();
            let routing = front.routing().unwrap();
            let raw = if work == 0 { 0 } else { 4_950_000_000 };
            assert_eq!(routing.owed[&who], raw + 1_000_123);
            assert_eq!(routing.kept, vec![(who, raw + 1_000_000)]);
            assert_eq!(routing.pending_deltas[&who], -1_000_000);
            assert_eq!(routing.pool_fee, 5_000_000_000 - raw);
            assert_eq!(routing.pay_p, routing.pool_fee - 1_000_000);
            assert_eq!(front.distribution().unwrap().outputs[0].1, routing.pay_p);
            assert_eq!(front.window_underfilled(), work < 65);
            assert_eq!(front.window_target, 65);
            assert_eq!(
                front.pending()[&who],
                1_000_123,
                "forecast never books pending"
            );
        }
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
            c.max_payout_slots = 1024;
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
            pending_deltas: vec![(id_for(0x31), 1)],
        }));
        front.refresh().await.unwrap();
        let third = front.job().unwrap().job_id.clone();
        assert_ne!(third, second, "a ledger change is a new distribution");

        // And another unchanged refresh keeps it.
        front.refresh().await.unwrap();
        assert_eq!(front.job().unwrap().job_id, third);
    }

    #[tokio::test]
    async fn same_outputs_with_a_new_observed_window_get_a_new_immutable_job() {
        let source = Arc::new(MockTemplateSource::new(template_at(
            800_000,
            0x1c00_ffff,
            0,
        )));
        let config = cfg_with(cfg(), |c| {
            c.window_n = 1;
            c.max_slice_shares = 1;
            c.max_payout_slots = 1024;
        });
        let mut front = Front::new(config, source, type32());
        front.refresh().await.unwrap();
        front.on_share(share_for(&front, 0x21, 1)).unwrap();
        front.on_share(share_for(&front, 0x21, 2)).unwrap();
        front.refresh().await.unwrap();
        let first_job = front.job().unwrap().job_id.clone();
        let first_outputs = front.distribution().unwrap().outputs.clone();
        let first_window = front.published_window;
        // The preceding generation transition sealed the second actual share.
        // Its identical relative score must not hide a different high-water mark.
        front.refresh().await.unwrap();
        assert_eq!(front.distribution().unwrap().outputs, first_outputs);
        assert_ne!(front.published_window, first_window);
        assert_ne!(front.job().unwrap().job_id, first_job);
        let mut stale = share_for(&front, 0x21, 3);
        stale.job_id = first_job;
        assert!(matches!(front.on_share(stale), Err(FrontError::StaleJob)));
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
        front
            .apply_bootstrap(&BootstrapSnapshot {
                economic: None,
                rev: 1,
                applied_blocks: vec![],
                pending_balance: vec![],
                ark_delivery: vec![],
                ark_identities: vec![],
                ark_epoch_mailboxes: vec![],
                window_tail: vec![slice_row(9_000, 799_999, &[(miner, 1, 0)])],
                type32_blobs: vec![],
                slices_cursor: None,
                front_events_cursor: None,
            })
            .unwrap();
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
            c.max_payout_slots = 1024;
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
            pending_deltas: vec![(id_for(0x31), 1)],
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
    async fn jd_weights_stay_exact_while_pool_over_revenue_fails_closed() {
        let bits = 0x1c00_ffff;
        let source = Arc::new(MutableSource::new(template_at(800_000, bits, 0)));
        let cfg = cfg_with(cfg(), |c| {
            c.fee_bps = 0;
            c.window_n = 1;
            c.max_payout_slots = 1024;
            c.slice_fee_delta_sats = 50_000; // valid JD fee band does not disable ladder construction
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
        assert!(matches!(
            front.refresh().await,
            Err(FrontError::Accounting(
                spc_accounting::Error::EntitlementExceedsRevenue
            ))
        ));
        let raw = spc_accounting::RawPayouts {
            raw: BTreeMap::from([(id, 312_500_000)]),
            pool_sats: 0,
            r_net: 312_500_000,
            f_net: 0,
        };
        let routing = spc_accounting::route_jd_tiers(
            &raw,
            front.pending(),
            &BTreeMap::new(),
            &TierConfig {
                fee_bps: 0,
                dust_limit_sats: 330,
                coinbase_threshold_sats: 330,
                max_payout_slots: 1024,
                budget_wu: 100_000,
                overhead_wu: 0,
            },
            312_500_000,
        )
        .unwrap();
        assert_eq!(build_weights(&routing, 0).weights[0].1, 412_500_000);
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
            economic: None,
            rev: 7,
            applied_blocks: vec![],
            pending_balance: vec![(id, 5_000)],
            ark_identities: vec![],
            ark_epoch_mailboxes: vec![],
            ark_delivery: vec![spc_store::models::ArkDeliveryRow {
                registration_commit: None,
                sp_id: id,
                ark_id: [1, 2, 3, 4],
                b_ark: vec![2u8; 33],
                mailbox_mode: spc_store::models::MailboxMode::None,
                mailbox_m: None,
                mailbox_d: None,
                epoch_fallback: spc_store::models::EpochFallback::Static,
            }],
            window_tail: vec![],
            type32_blobs: vec![],
            slices_cursor: None,
            front_events_cursor: None,
        };
        front.apply_bootstrap(&snapshot).unwrap();
        assert_eq!(front.pending().get(&id), Some(&5_000));
        // Registered, but the snapshot carries `b_ark`, not a payable bark
        // address: the destination stays unknown and the balance is held.
        let cap = front.ark.get(&id).expect("registration applied");
        assert!(cap.registered);
        assert!(!cap.dest_known);

        // A live registration for a new id is applied; a stale revision is not.
        let other = id_for(0x31);
        let secp = bitcoin::secp256k1::Secp256k1::new();
        let scan = bitcoin::secp256k1::PublicKey::from_secret_key(
            &secp,
            &bitcoin::secp256k1::SecretKey::from_slice(&[0x11; 32]).unwrap(),
        )
        .serialize();
        let spend = bitcoin::secp256k1::PublicKey::from_secret_key(
            &secp,
            &bitcoin::secp256k1::SecretKey::from_slice(&[0x12; 32]).unwrap(),
        )
        .serialize();
        let b_ark = bitcoin::secp256k1::PublicKey::from_secret_key(
            &secp,
            &bitcoin::secp256k1::SecretKey::from_slice(&[0x13; 32]).unwrap(),
        )
        .serialize();
        let identity = spc_store::models::SpIdentityRow {
            sp_id: other,
            b_scan: scan.to_vec(),
            b_spend: spend.to_vec(),
            network: 3,
        };
        let delivery = spc_store::models::ArkDeliveryRow {
            registration_commit: None,
            sp_id: other,
            ark_id: [1, 2, 3, 4],
            b_ark: b_ark.to_vec(),
            mailbox_mode: spc_store::models::MailboxMode::None,
            mailbox_m: None,
            mailbox_d: None,
            epoch_fallback: spc_store::models::EpochFallback::Static,
        };
        assert!(front.apply_front_event(&WireFrontEvent::ArkRegApplied {
            rev: 8,
            sp_id: other,
            request_id: 1,
            registration_rev: 8,
            state: spc_ext::RegistrationSuccessState::Applied,
            identity: identity.clone(),
            delivery: delivery.clone(),
            epoch_mailboxes: vec![],
        }));
        assert!(front.ark.get(&other).expect("registered").registered);
        assert!(!front.apply_front_event(&WireFrontEvent::ArkRegApplied {
            rev: 8,
            sp_id: other,
            request_id: 1,
            registration_rev: 8,
            state: spc_ext::RegistrationSuccessState::Applied,
            identity: identity.clone(),
            delivery: delivery.clone(),
            epoch_mailboxes: vec![],
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
        front
            .apply_bootstrap(&BootstrapSnapshot {
                economic: None,
                rev: 1,
                applied_blocks: vec![block],
                pending_balance: vec![(id, 7_000)],
                ark_delivery: vec![],
                ark_identities: vec![],
                ark_epoch_mailboxes: vec![],
                window_tail: vec![],
                type32_blobs: vec![],
                slices_cursor: None,
                front_events_cursor: None,
            })
            .unwrap();
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
            economic: None,
            rev: 42,
            applied_blocks: vec![],
            pending_balance: vec![(id, 5_000)],
            ark_delivery: vec![],
            ark_identities: vec![],
            ark_epoch_mailboxes: vec![],
            window_tail: vec![],
            type32_blobs: vec![],
            slices_cursor: None,
            front_events_cursor: None,
        };
        front.apply_bootstrap(&snapshot).unwrap();
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

    #[tokio::test]
    async fn registration_destinations_and_rails_roll_job_even_when_coinbase_does_not_change() {
        let mut front = Front::new(
            cfg(),
            Arc::new(MockTemplateSource::new(template())),
            type32(),
        );
        let server_key = bitcoin::secp256k1::SecretKey::from_slice(&[9; 32]).unwrap();
        let secp = bitcoin::secp256k1::Secp256k1::new();
        let server = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &server_key);
        front.set_ark_server(ArkServerMetadata {
            ark_id: <[u8; 4]>::from(ark::address::ArkId::from_server_pubkey(server)),
            server_pubkey: server,
            mailbox_pubkey: bitcoin::secp256k1::PublicKey::from_secret_key(
                &secp,
                &bitcoin::secp256k1::SecretKey::from_slice(&[10; 32]).unwrap(),
            ),
            testnet: true,
        });
        let (identity_scan, identity_spend) = {
            let secp = bitcoin::secp256k1::Secp256k1::new();
            let scan = bitcoin::secp256k1::PublicKey::from_secret_key(
                &secp,
                &bitcoin::secp256k1::SecretKey::from_slice(&[0x21; 32]).unwrap(),
            );
            let spend = bitcoin::secp256k1::PublicKey::from_secret_key(
                &secp,
                &bitcoin::secp256k1::SecretKey::from_slice(&[0x22; 32]).unwrap(),
            );
            (scan, spend)
        };
        let identity_sp_id = spc_crypto::sp_id(&identity_scan, &identity_spend);
        let identity = spc_store::models::SpIdentityRow {
            sp_id: identity_sp_id,
            b_scan: identity_scan.serialize().to_vec(),
            b_spend: identity_spend.serialize().to_vec(),
            network: 3,
        };
        let snapshot = BootstrapSnapshot {
            economic: None,
            rev: 0,
            applied_blocks: vec![],
            pending_balance: vec![(identity_sp_id, 100)],
            ark_identities: vec![],
            ark_epoch_mailboxes: vec![],
            ark_delivery: vec![],
            window_tail: vec![],
            type32_blobs: vec![],
            slices_cursor: None,
            front_events_cursor: None,
        };
        front.apply_bootstrap(&snapshot).unwrap();
        let delivery_for = |seed: u8, mode| spc_store::models::ArkDeliveryRow {
            registration_commit: None,
            sp_id: identity_sp_id,
            ark_id: <[u8; 4]>::from(ark::address::ArkId::from_server_pubkey(server)),
            b_ark: bitcoin::secp256k1::PublicKey::from_secret_key(
                &secp,
                &bitcoin::secp256k1::SecretKey::from_slice(&[seed; 32]).unwrap(),
            )
            .serialize()
            .to_vec(),
            mailbox_mode: mode,
            mailbox_m: if mode == spc_store::models::MailboxMode::None {
                None
            } else {
                Some(
                    bitcoin::secp256k1::PublicKey::from_secret_key(
                        &secp,
                        &bitcoin::secp256k1::SecretKey::from_slice(&[40; 32]).unwrap(),
                    )
                    .serialize()
                    .to_vec(),
                )
            },
            mailbox_d: if mode == spc_store::models::MailboxMode::None {
                None
            } else {
                Some(
                    bitcoin::secp256k1::PublicKey::from_secret_key(
                        &secp,
                        &bitcoin::secp256k1::SecretKey::from_slice(&[41; 32]).unwrap(),
                    )
                    .serialize()
                    .to_vec(),
                )
            },
            epoch_fallback: spc_store::models::EpochFallback::Hold,
        };
        let event = |rev, state, delivery: spc_store::models::ArkDeliveryRow| {
            WireFrontEvent::ArkRegApplied {
                rev,
                sp_id: identity_sp_id,
                request_id: rev as u32,
                registration_rev: rev,
                state,
                identity: identity.clone(),
                delivery,
                epoch_mailboxes: vec![],
            }
        };
        assert!(front.apply_front_event(&event(
            1,
            spc_ext::RegistrationSuccessState::Applied,
            delivery_for(11, spc_store::models::MailboxMode::EpochRegistered),
        )));
        let first_job = front.refresh().await.unwrap();
        let old_share = share_for(&front, 0x21, 1);
        let first_distribution = front.distribution().unwrap().clone();
        let first_entry = first_distribution
            .entries
            .iter()
            .find(|entry| entry.sp_id == identity_sp_id)
            .unwrap();
        assert_eq!(first_entry.rail, spc_accounting::Rail::Pending);
        assert!(first_entry.dest_id.is_none());

        let second_job = front.apply_front_event(&event(
            2,
            spc_ext::RegistrationSuccessState::Applied,
            delivery_for(12, spc_store::models::MailboxMode::Static),
        ));
        assert!(second_job);
        let second_job = front.refresh().await.unwrap();
        let second_distribution = front.distribution().unwrap().clone();
        assert_ne!(second_job.job_id, first_job.job_id);
        assert_eq!(
            second_distribution.fingerprint,
            first_distribution.fingerprint
        );
        let second_entry = second_distribution
            .entries
            .iter()
            .find(|entry| entry.sp_id == identity_sp_id)
            .unwrap();
        assert_eq!(second_entry.rail, spc_accounting::Rail::Ark);
        let second_dest = second_entry.dest_id.clone().expect("derived destination");
        assert!(matches!(
            front.on_share(old_share),
            Err(FrontError::StaleJob)
        ));

        assert!(front.apply_front_event(&event(
            3,
            spc_ext::RegistrationSuccessState::Applied,
            delivery_for(13, spc_store::models::MailboxMode::Static),
        )));
        let third_job = front.refresh().await.unwrap();
        let third_distribution = front.distribution().unwrap().clone();
        assert_ne!(third_job.job_id, second_job.job_id);
        assert_eq!(
            third_distribution.fingerprint,
            second_distribution.fingerprint
        );
        let third_dest = third_distribution
            .entries
            .iter()
            .find(|entry| entry.sp_id == identity_sp_id)
            .and_then(|entry| entry.dest_id.clone())
            .expect("changed destination");
        assert_ne!(third_dest, second_dest);

        let mut invalid = event(
            4,
            spc_ext::RegistrationSuccessState::Applied,
            delivery_for(14, spc_store::models::MailboxMode::Static),
        );
        if let WireFrontEvent::ArkRegApplied { identity, .. } = &mut invalid {
            identity.b_scan = vec![2; 31];
        }
        assert!(
            !front.apply_front_event(&invalid),
            "bad event is skipped, never a panic"
        );
    }
}
