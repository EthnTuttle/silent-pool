//! Frozen traffic scenario schema (traffic plan Section 11).
//!
//! The schema is frozen complete by T0: every field, default, range,
//! cross-field rule and error code lives here. T1 implements runtime
//! behavior and T2-L implements bootstrap behavior against these types;
//! later packets never create pieces of the same type in dependency
//! order. Validation is structural and semantic in the scenario style:
//! unknown fields are rejected at parse time (`deny_unknown_fields`) and
//! every semantic violation has a stable `traffic_*` (or plan-named)
//! error code, rejected before any process starts.
//!
//! Existing scenarios (no `traffic` field) are unchanged and still
//! validate byte-identically: `scenario_version` stays 1 for the additive
//! field, so an old runner rejects a traffic-bearing scenario as an
//! unknown field while a new runner accepts both.

use serde::{Deserialize, Serialize};

use crate::ids::ActorId;
use crate::scenario::{ScenarioError, ValidationReport};
use crate::traffic::protocol::{
    TRAFFIC_HISTORY_VERSION, TX_CONTROL_PROTOCOL_MAJOR, TX_CONTROL_PROTOCOL_NAME,
};

/// The only accepted `traffic.version` in the frozen v1 schema.
pub const TRAFFIC_SPEC_VERSION: u32 = 1;

/// The single traffic capability name v1 knows (the AP-1a projection of
/// the canonical report; traffic plan Section 8).
pub const TRAFFIC_CAPABILITY: &str = "spc.node.regtest.mempool_rpc.v1";

/// Real 100-block regtest coinbase maturity requires at least 101 funded
/// blocks (block-1 coinbases must be spendable at the distribution step).
pub const FUNDING_BLOCKS_MIN: u32 = 101;
pub const FUNDING_BLOCKS_MAX: u32 = 1000;

pub const DISTRIBUTION_OUTPUTS_PER_WALLET_MIN: u32 = 1;
pub const DISTRIBUTION_OUTPUTS_PER_WALLET_MAX: u32 = 64;

/// v1 hosts one or two driver actors; more requires a new packet.
pub const TRAFFIC_ACTORS_MIN: usize = 1;
pub const TRAFFIC_ACTORS_MAX: usize = 2;

/// Defensive wallet-count ceiling above the stress preset (100 network
/// wallets across at most two actors); reaching it is a schema rejection,
/// never a silent truncation.
pub const NETWORK_WALLETS_MAX: u32 = 256;

pub const ORDINARY_TX_PER_SEC_MIN: f64 = 0.01;
pub const ORDINARY_TX_PER_SEC_MAX: f64 = 20.0;

/// Aggregate admission budget for ordinary plus Ark chain-effect streams
/// (user payments are manual and counted at runtime, not declared here).
pub const ADMISSION_BUDGET_TX_PER_SEC_MAX: f64 = 20.0;

/// Ark chain-effect budget ceiling (stress preset cap; Ark agent owns
/// realization).
pub const ARK_CHAIN_EFFECT_BUDGET_TX_PER_SEC_MAX: f64 = 1.0;

pub const FEE_RATE_SATS_VB_MIN: u64 = 1;
pub const FEE_RATE_SATS_VB_MAX: u64 = 100;
pub const MAX_FEE_SATS_PER_TX_MAX: u64 = 100_000;

pub const MAX_VB_MIN: u64 = 1;
pub const MAX_VB_MAX: u64 = 100_000;
/// Defensive ceiling on the number of declared size classes.
pub const SIZE_CLASSES_MAX: usize = 64;

pub const MAX_INFLIGHT_BROADCASTS_MIN: u64 = 1;
pub const MAX_INFLIGHT_BROADCASTS_MAX: u64 = 64;
pub const MAX_UNKNOWN_RETRIES_MIN: u64 = 0;
pub const MAX_UNKNOWN_RETRIES_MAX: u64 = 3;
pub const MAX_MEMPOOL_BACKLOG_TX_MIN: u64 = 1;
pub const MAX_MEMPOOL_BACKLOG_TX_MAX: u64 = 2000;
pub const REORG_RESUBMIT_MAX_MIN: u64 = 1;
pub const REORG_RESUBMIT_MAX_MAX: u64 = 3;
/// Declared default for `caps.reorg_resubmit_max` (traffic plan 6.5).
pub const REORG_RESUBMIT_MAX_DEFAULT: u64 = 3;

/// Tolerance for decimal fractions (`mix` sums, admission sums) compared
/// as f64. Rates and mixes are configuration, not money: the plan's exact
/// decimal arithmetic requirement applies to BTC amounts (T1), while these
/// bounded comparisons only need to reject real violations.
pub const FRACTION_EPSILON: f64 = 1e-9;

/// The frozen traffic scenario section (traffic plan Section 11).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrafficSpec {
    /// Test-only interposition, disabled unless explicitly opted in. Requires
    /// expanded operator authentication at runtime.
    #[serde(default)]
    pub lab_acceptance: bool,
    /// Only 1 is accepted; anything else is `traffic_version_unsupported`.
    pub version: u32,
    /// Must name the recorded traffic capability object
    /// (`spc.node.regtest.mempool_rpc.v1`).
    pub required_capability: String,
    pub protocol: TrafficProtocolSpec,
    pub bootstrap: TrafficBootstrapSpec,
    pub driver: TrafficDriverSpec,
    pub rate: TrafficRateSpec,
    pub fees: TrafficFeesSpec,
    pub sizes: TrafficSizesSpec,
    /// v1: `p2pkh` only; `p2wpkh` is rejected until T4.
    pub spend_types: Vec<TrafficSpendType>,
    pub ark: TrafficArkSpec,
    pub caps: TrafficCapsSpec,
    /// v1 accepts only `no_broadcast_after_ack`.
    pub pause_semantics: TrafficPauseSemantics,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrafficProtocolSpec {
    /// Control protocol string `<name>/v<major>`; an unknown major fails
    /// before any broadcast.
    pub control: String,
    /// Versioned private history record contract (traffic plan 6.3):
    /// the JOURNAL protocol version (3), not the control protocol
    /// version - the two are never conflated.
    pub history_version: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BootstrapMiningMode {
    /// Declared default: real PoW on the approved custom profile by the
    /// named bootstrap block-production module (T2-L).
    #[default]
    Native,
    /// Separately scoped and proven; never assumed on the approved custom
    /// profile. Rejected in v1 (no proof record mechanism exists yet).
    RpcGenerate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrafficBootstrapSpec {
    pub enabled: bool,
    /// Defaults to `native` (the declared default mining mode).
    #[serde(default)]
    pub mining_mode: BootstrapMiningMode,
    /// At least 101 (real 100-block coinbase maturity), at most 1000.
    pub funding_blocks: u32,
    /// Distribution outputs seeded per wallet: 1..=64.
    pub distribution_outputs_per_wallet: u32,
    /// MUST be true in v1.
    pub excluded_from_statistics: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrafficDriverSpec {
    #[serde(default)]
    pub bootstrap: BootstrapExecutionLimits,
    /// 1..=2 actors in v1.
    pub actors: Vec<TrafficActorSpec>,
    /// Must equal the scenario's identity (SPc user) count.
    pub user_payment_wallets: u32,
    /// Private history journal path, relative under the run directory.
    pub history: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrafficActorSpec {
    /// The driver actor's id: a shared [`ActorId`] (at most 128 bytes,
    /// ASCII, no control characters), enforced by [`TrafficSpec`]'s
    /// validation so both parsed and programmatically built scenarios
    /// obey the same charset bound. Rendered into private file names,
    /// evidence and process specs - never a free-form string.
    pub id: ActorId,
    /// Network (background churn) wallets hosted by this actor: 1..=256.
    pub network_wallets: u32,
    pub user_payment_wallets: u32,
    pub hosts_bootstrap_funding: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapExecutionLimits {
    pub hashes_per_second: u64,
    pub total_attempts: u64,
    pub deadline_secs: u64,
    pub cancel_grace_secs: u64,
}
impl Default for BootstrapExecutionLimits {
    fn default() -> Self {
        Self {
            hashes_per_second: 40,
            total_attempts: 4_194_304,
            deadline_secs: 3600,
            cancel_grace_secs: 30,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TrafficPacing {
    /// Fixed interval with bounded seed-derived jitter (the v1 default).
    #[default]
    FixedJitter,
    /// Bounded burst profile: gated behind T6, rejected in v1.
    Burst,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrafficRateSpec {
    /// 0.01..=20.0 ordinary transactions per second.
    pub ordinary_tx_per_sec: f64,
    /// Defaults to `fixed_jitter`; `burst` is T6-gated.
    #[serde(default)]
    pub pacing: TrafficPacing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrafficFeesSpec {
    pub rate_sats_vb: TrafficFeeRateBand,
    /// Hard per-transaction fee ceiling: 1..=100000 sats.
    pub max_fee_sats_per_tx: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrafficFeeRateBand {
    pub min: u64,
    pub max: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrafficSizesSpec {
    /// Declared size classes; `sum(mix)` must be 1.
    pub classes_vb: Vec<TrafficSizeClass>,
    /// Hard per-transaction virtual size cap: 1..=100000 vB, and every
    /// declared class must satisfy `vb <= max_vb`.
    pub max_vb: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrafficSizeClass {
    pub vb: u64,
    pub mix: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrafficSpendType {
    /// The only v1 input spend type: non-witness P2PKH (T1-DEST also permits
    /// standard P2WPKH/P2TR destination outputs).
    P2pkh,
    /// Rejected until T4 (witness gating); see `traffic_spend_type_gated`.
    P2wpkh,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrafficArkSpec {
    pub enabled: bool,
    /// MUST be true in v1 (the registration-before-mining phase barrier).
    pub registration_before_mining: bool,
    /// Ark chain-effect budget in tx/s, owned by the Ark agent. Zero when
    /// disabled; at most 1.0 when enabled.
    pub chain_effect_budget_tx_per_sec: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrafficCapsSpec {
    pub max_inflight_broadcasts: u64,
    /// Unknown-outcome retry cap: one initial send plus AT MOST this
    /// many retry sends. The cap bounds sends, never evidence - an
    /// outcome (accepted/rejected/expired-unknown) is always recordable
    /// after the cap is exhausted. Bounded 0..=3; out-of-range values
    /// are rejected, never silently clamped or ignored.
    pub max_unknown_retries: u64,
    pub max_mempool_backlog_tx: u64,
    /// Bounded same-bytes reorg resubmission ceiling (traffic plan 6.5).
    /// Defaults to 3.
    #[serde(default = "default_reorg_resubmit_max")]
    pub reorg_resubmit_max: u64,
}

fn default_reorg_resubmit_max() -> u64 {
    REORG_RESUBMIT_MAX_DEFAULT
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrafficPauseSemantics {
    /// The only v1 pause semantics: the quiesce barrier acknowledges only
    /// when no `sendrawtransaction` call is in flight and none will be
    /// started (traffic plan 6.4).
    NoBroadcastAfterAck,
}

impl TrafficSpec {
    /// Structural and semantic validation in the scenario style.
    ///
    /// `identities_count` is the scenario's identity (SPc user) count:
    /// `user_payment_wallets` must equal it. On success the recorded
    /// traffic capability is inserted into `report.required_capabilities`
    /// so preflight gates traffic exactly like `spc.node.regtest.pow_baseline`
    /// today.
    pub fn validate(
        &self,
        identities_count: usize,
        report: &mut ValidationReport,
    ) -> Result<(), ScenarioError> {
        if self.version != TRAFFIC_SPEC_VERSION {
            return Err(ScenarioError::new(
                "traffic_version_unsupported",
                format!(
                    "traffic.version {} is not supported (expected {TRAFFIC_SPEC_VERSION})",
                    self.version
                ),
            ));
        }
        if self.required_capability.trim().is_empty() {
            return Err(ScenarioError::new(
                "traffic_capability_missing",
                "traffic.required_capability is empty",
            ));
        }
        if self.required_capability != TRAFFIC_CAPABILITY {
            return Err(ScenarioError::new(
                "traffic_capability_unknown",
                format!(
                    "traffic.required_capability {} is not a known traffic capability (v1 knows \
                     only {TRAFFIC_CAPABILITY})",
                    self.required_capability
                ),
            ));
        }
        self.validate_protocol()?;
        self.validate_bootstrap()?;
        self.validate_driver(identities_count)?;
        self.validate_rate()?;
        self.validate_fees()?;
        self.validate_sizes()?;
        self.validate_spend_types()?;
        self.validate_ark()?;
        self.validate_caps()?;
        report
            .required_capabilities
            .insert(TRAFFIC_CAPABILITY.to_string());
        Ok(())
    }

    fn validate_protocol(&self) -> Result<(), ScenarioError> {
        // Exact string, no trim: the scenario must carry the protocol
        // string byte-for-byte, so any surrounding whitespace is a
        // rejection, never a normalized acceptance.
        let control = self.protocol.control.as_str();
        if !control.starts_with(TX_CONTROL_PROTOCOL_NAME)
            || !control[TX_CONTROL_PROTOCOL_NAME.len()..].starts_with('/')
        {
            return Err(ScenarioError::new(
                "traffic_protocol_unsupported",
                format!(
                    "traffic.protocol.control {control:?} is not the tx-control protocol \
                     ({TX_CONTROL_PROTOCOL_NAME}/v<major>)"
                ),
            ));
        }
        let version_part = &control[TX_CONTROL_PROTOCOL_NAME.len() + 1..];
        let expected_version = format!("v{TX_CONTROL_PROTOCOL_MAJOR}");
        if version_part != expected_version {
            return Err(ScenarioError::new(
                "traffic_protocol_unsupported",
                format!(
                    "traffic.protocol.control {control:?} is an unsupported tx-control version \
                     (expected major {TX_CONTROL_PROTOCOL_MAJOR})"
                ),
            ));
        }
        if self.protocol.history_version != TRAFFIC_HISTORY_VERSION {
            return Err(ScenarioError::new(
                "traffic_protocol_unsupported",
                format!(
                    "traffic.protocol.history_version {} is not supported (expected \
                     {TRAFFIC_HISTORY_VERSION}); traffic-history versions 1 and 2 \
                     are refused before any process starts, never migrated",
                    self.protocol.history_version
                ),
            ));
        }
        Ok(())
    }

    fn validate_bootstrap(&self) -> Result<(), ScenarioError> {
        let bootstrap = &self.bootstrap;
        if !bootstrap.enabled {
            return Err(ScenarioError::new(
                "traffic_bootstrap_required",
                "traffic.bootstrap.enabled must be true in v1: no alternative funding source is \
                 declared (a later packet may add one explicitly)",
            ));
        }
        if bootstrap.funding_blocks < FUNDING_BLOCKS_MIN {
            return Err(ScenarioError::new(
                "bootstrap_maturity_insufficient",
                format!(
                    "traffic.bootstrap.funding_blocks {} is below the real 100-block coinbase \
                     maturity minimum of {FUNDING_BLOCKS_MIN}",
                    bootstrap.funding_blocks
                ),
            ));
        }
        if bootstrap.funding_blocks > FUNDING_BLOCKS_MAX {
            return Err(ScenarioError::new(
                "traffic_bootstrap_funding_blocks_out_of_range",
                format!(
                    "traffic.bootstrap.funding_blocks {} exceeds the maximum of \
                     {FUNDING_BLOCKS_MAX}",
                    bootstrap.funding_blocks
                ),
            ));
        }
        if !(DISTRIBUTION_OUTPUTS_PER_WALLET_MIN..=DISTRIBUTION_OUTPUTS_PER_WALLET_MAX)
            .contains(&bootstrap.distribution_outputs_per_wallet)
        {
            return Err(ScenarioError::new(
                "traffic_bootstrap_distribution_invalid",
                format!(
                    "traffic.bootstrap.distribution_outputs_per_wallet {} is outside \
                     {DISTRIBUTION_OUTPUTS_PER_WALLET_MIN}..={DISTRIBUTION_OUTPUTS_PER_WALLET_MAX}",
                    bootstrap.distribution_outputs_per_wallet
                ),
            ));
        }
        if !bootstrap.excluded_from_statistics {
            return Err(ScenarioError::new(
                "traffic_bootstrap_statistics_not_excluded",
                "traffic.bootstrap.excluded_from_statistics must be true in v1",
            ));
        }
        if matches!(bootstrap.mining_mode, BootstrapMiningMode::RpcGenerate) {
            return Err(ScenarioError::new(
                "traffic_bootstrap_mining_mode_invalid",
                "traffic.bootstrap.mining_mode rpc_generate requires a separately recorded proof \
                 on the exact node/profile; no proof record mechanism exists in v1 (declared \
                 native bootstrap is the only accepted mode)",
            ));
        }
        Ok(())
    }

    fn validate_driver(&self, identities_count: usize) -> Result<(), ScenarioError> {
        let driver = &self.driver;
        if !(TRAFFIC_ACTORS_MIN..=TRAFFIC_ACTORS_MAX).contains(&driver.actors.len()) {
            return Err(ScenarioError::new(
                "traffic_actor_count_invalid",
                format!(
                    "traffic.driver.actors count {} is outside \
                     {TRAFFIC_ACTORS_MIN}..={TRAFFIC_ACTORS_MAX}",
                    driver.actors.len()
                ),
            ));
        }
        let mut seen_ids = std::collections::BTreeSet::new();
        let mut total_network_wallets: u64 = 0;
        let mut total_user_wallets = 0u64;
        let mut hosts = 0u32;
        for actor in &driver.actors {
            if let Err(e) = ActorId::new(actor.id.as_str()) {
                return Err(ScenarioError::new(
                    "traffic_actor_id_invalid",
                    format!(
                        "traffic.driver.actors id is not a valid actor id (at most 128 \
                         bytes, ASCII, no control characters): {e}"
                    ),
                ));
            }
            if !seen_ids.insert(actor.id.clone()) {
                return Err(ScenarioError::new(
                    "traffic_actor_id_duplicate",
                    format!("traffic.driver.actors id {} is declared twice", actor.id),
                ));
            }
            if actor.network_wallets == 0 || actor.network_wallets > NETWORK_WALLETS_MAX {
                return Err(ScenarioError::new(
                    "traffic_actor_wallet_overflow",
                    format!(
                        "actor {} declares network_wallets {} outside 1..={NETWORK_WALLETS_MAX}",
                        actor.id, actor.network_wallets
                    ),
                ));
            }
            total_network_wallets += u64::from(actor.network_wallets);
            if actor.user_payment_wallets == 0 {
                return Err(ScenarioError::new(
                    "traffic_actor_role_count_invalid",
                    "every actor must own both wallet roles",
                ));
            }
            total_user_wallets = total_user_wallets
                .checked_add(u64::from(actor.user_payment_wallets))
                .ok_or_else(|| {
                    ScenarioError::new("traffic_actor_wallet_overflow", "user wallet sum overflow")
                })?;
            hosts += u32::from(actor.hosts_bootstrap_funding);
        }
        if total_network_wallets > u64::from(NETWORK_WALLETS_MAX) {
            return Err(ScenarioError::new(
                "traffic_actor_wallet_overflow",
                format!(
                    "total network wallets {total_network_wallets} across actors exceeds \
                     {NETWORK_WALLETS_MAX}"
                ),
            ));
        }
        if driver.user_payment_wallets == 0 {
            return Err(ScenarioError::new(
                "traffic_user_payment_wallets_invalid",
                "traffic.driver.user_payment_wallets must be at least 1",
            ));
        }
        if usize::try_from(driver.user_payment_wallets).unwrap_or(usize::MAX) != identities_count {
            return Err(ScenarioError::new(
                "traffic_user_payment_wallets_mismatch",
                format!(
                    "traffic.driver.user_payment_wallets {} does not equal the scenario \
                     identity count {identities_count}",
                    driver.user_payment_wallets
                ),
            ));
        }
        if total_user_wallets != u64::from(driver.user_payment_wallets) || total_user_wallets > 256
        {
            return Err(ScenarioError::new(
                "traffic_user_payment_wallets_mismatch",
                "per-actor user counts must sum to the identity count within 256",
            ));
        }
        if hosts != 1 {
            return Err(ScenarioError::new(
                "traffic_bootstrap_host_invalid",
                "exactly one bootstrap host is required",
            ));
        }
        let limits = &driver.bootstrap;
        if [
            limits.hashes_per_second,
            limits.total_attempts,
            limits.deadline_secs,
            limits.cancel_grace_secs,
        ]
        .contains(&0)
            || limits.deadline_secs.checked_mul(1000).is_none()
            || limits.cancel_grace_secs.checked_mul(1000).is_none()
        {
            return Err(ScenarioError::new(
                "traffic_bootstrap_limits_invalid",
                "positive checked bootstrap execution limits required",
            ));
        }
        let history = driver.history.as_str();
        if history != "traffic/" {
            return Err(ScenarioError::new(
                "traffic_history_path_invalid",
                format!("traffic.driver.history {history:?} must equal traffic/ exactly"),
            ));
        }
        Ok(())
    }

    fn validate_rate(&self) -> Result<(), ScenarioError> {
        let rate = &self.rate;
        if !rate.ordinary_tx_per_sec.is_finite()
            || rate.ordinary_tx_per_sec < ORDINARY_TX_PER_SEC_MIN
        {
            return Err(ScenarioError::new(
                "traffic_rate_bounds_invalid",
                format!(
                    "traffic.rate.ordinary_tx_per_sec {} is below the minimum of \
                     {ORDINARY_TX_PER_SEC_MIN}",
                    rate.ordinary_tx_per_sec
                ),
            ));
        }
        if rate.ordinary_tx_per_sec > ORDINARY_TX_PER_SEC_MAX {
            return Err(ScenarioError::new(
                "traffic_rate_exceeds_preset_cap",
                format!(
                    "traffic.rate.ordinary_tx_per_sec {} exceeds the hard cap of \
                     {ORDINARY_TX_PER_SEC_MAX} (a new packet is required to raise it)",
                    rate.ordinary_tx_per_sec
                ),
            ));
        }
        if matches!(rate.pacing, TrafficPacing::Burst) {
            return Err(ScenarioError::new(
                "traffic_pacing_unsupported",
                "traffic.rate.pacing burst is a T6 profile and is not accepted in v1 (only \
                 fixed_jitter)",
            ));
        }
        Ok(())
    }

    fn validate_fees(&self) -> Result<(), ScenarioError> {
        let fees = &self.fees;
        if fees.rate_sats_vb.min < FEE_RATE_SATS_VB_MIN
            || fees.rate_sats_vb.max > FEE_RATE_SATS_VB_MAX
            || fees.rate_sats_vb.min > fees.rate_sats_vb.max
        {
            return Err(ScenarioError::new(
                "traffic_fee_bounds_invalid",
                format!(
                    "traffic.fees.rate_sats_vb min {} / max {} violate the band \
                     {FEE_RATE_SATS_VB_MIN}..={FEE_RATE_SATS_VB_MAX} with min <= max",
                    fees.rate_sats_vb.min, fees.rate_sats_vb.max
                ),
            ));
        }
        if fees.max_fee_sats_per_tx == 0 || fees.max_fee_sats_per_tx > MAX_FEE_SATS_PER_TX_MAX {
            return Err(ScenarioError::new(
                "traffic_fee_bounds_invalid",
                format!(
                    "traffic.fees.max_fee_sats_per_tx {} is outside \
                     1..={MAX_FEE_SATS_PER_TX_MAX}",
                    fees.max_fee_sats_per_tx
                ),
            ));
        }
        Ok(())
    }

    fn validate_sizes(&self) -> Result<(), ScenarioError> {
        let sizes = &self.sizes;
        if sizes.max_vb < MAX_VB_MIN || sizes.max_vb > MAX_VB_MAX {
            return Err(ScenarioError::new(
                "traffic_size_cap_invalid",
                format!(
                    "traffic.sizes.max_vb {} is outside {MAX_VB_MIN}..={MAX_VB_MAX}",
                    sizes.max_vb
                ),
            ));
        }
        if sizes.classes_vb.is_empty() || sizes.classes_vb.len() > SIZE_CLASSES_MAX {
            return Err(ScenarioError::new(
                "traffic_size_mix_invalid",
                format!(
                    "traffic.sizes.classes_vb must declare 1..={SIZE_CLASSES_MAX} classes (got \
                     {})",
                    sizes.classes_vb.len()
                ),
            ));
        }
        let mut mix_sum = 0.0_f64;
        for class in &sizes.classes_vb {
            if class.vb == 0 || class.vb > sizes.max_vb {
                return Err(ScenarioError::new(
                    "traffic_size_cap_invalid",
                    format!(
                        "size class vb {} is outside 1..=max_vb {}",
                        class.vb, sizes.max_vb
                    ),
                ));
            }
            if !class.mix.is_finite() || class.mix <= 0.0 || class.mix > 1.0 {
                return Err(ScenarioError::new(
                    "traffic_size_mix_invalid",
                    format!(
                        "size class vb {} mix {} is outside (0, 1]",
                        class.vb, class.mix
                    ),
                ));
            }
            mix_sum += class.mix;
        }
        if (mix_sum - 1.0).abs() > FRACTION_EPSILON {
            return Err(ScenarioError::new(
                "traffic_size_mix_invalid",
                format!(
                    "traffic.sizes.classes_vb mix sums to {mix_sum}, not 1 (tolerance \
                     {FRACTION_EPSILON})"
                ),
            ));
        }
        Ok(())
    }

    fn validate_spend_types(&self) -> Result<(), ScenarioError> {
        if self.spend_types.is_empty() {
            return Err(ScenarioError::new(
                "traffic_spend_type_missing",
                "traffic.spend_types must declare at least one spend type",
            ));
        }
        let mut seen = std::collections::BTreeSet::new();
        for spend_type in &self.spend_types {
            if !seen.insert(spend_type) {
                return Err(ScenarioError::new(
                    "traffic_spend_type_duplicate",
                    "traffic.spend_types declares the same spend type twice",
                ));
            }
            if matches!(spend_type, TrafficSpendType::P2wpkh) {
                return Err(ScenarioError::new(
                    "traffic_spend_type_gated",
                    "traffic.spend_types p2wpkh is rejected until T4 lands the witness packet \
                     (v1 accepts only p2pkh)",
                ));
            }
        }
        Ok(())
    }

    fn validate_ark(&self) -> Result<(), ScenarioError> {
        let ark = &self.ark;
        if !ark.registration_before_mining {
            return Err(ScenarioError::new(
                "traffic_ark_registration_invalid",
                "traffic.ark.registration_before_mining must be true in v1 (it is the only \
                 accepted value)",
            ));
        }
        let budget = ark.chain_effect_budget_tx_per_sec;
        if !budget.is_finite() || budget < 0.0 {
            return Err(ScenarioError::new(
                "traffic_ark_budget_invalid",
                format!("traffic.ark.chain_effect_budget_tx_per_sec {budget} is invalid"),
            ));
        }
        if ark.enabled {
            if budget <= 0.0 || budget > ARK_CHAIN_EFFECT_BUDGET_TX_PER_SEC_MAX {
                return Err(ScenarioError::new(
                    "traffic_ark_budget_invalid",
                    format!(
                        "traffic.ark.chain_effect_budget_tx_per_sec {budget} is outside \
                         (0, {ARK_CHAIN_EFFECT_BUDGET_TX_PER_SEC_MAX}] while ark is enabled"
                    ),
                ));
            }
        } else if budget > FRACTION_EPSILON {
            return Err(ScenarioError::new(
                "traffic_ark_budget_invalid",
                format!(
                    "traffic.ark.chain_effect_budget_tx_per_sec {budget} must be 0 while ark is \
                     disabled"
                ),
            ));
        }
        let admission = self.rate.ordinary_tx_per_sec + budget;
        if admission > ADMISSION_BUDGET_TX_PER_SEC_MAX + FRACTION_EPSILON {
            return Err(ScenarioError::new(
                "traffic_admission_overlapping_streams",
                format!(
                    "ordinary rate {} plus ark chain-effect budget {budget} is {admission}, \
                     above the aggregate admission cap of \
                     {ADMISSION_BUDGET_TX_PER_SEC_MAX}",
                    self.rate.ordinary_tx_per_sec
                ),
            ));
        }
        Ok(())
    }

    fn validate_caps(&self) -> Result<(), ScenarioError> {
        let caps = &self.caps;
        if !(MAX_INFLIGHT_BROADCASTS_MIN..=MAX_INFLIGHT_BROADCASTS_MAX)
            .contains(&caps.max_inflight_broadcasts)
        {
            return Err(ScenarioError::new(
                "traffic_caps_invalid",
                format!(
                    "traffic.caps.max_inflight_broadcasts {} is outside \
                     {MAX_INFLIGHT_BROADCASTS_MIN}..={MAX_INFLIGHT_BROADCASTS_MAX}",
                    caps.max_inflight_broadcasts
                ),
            ));
        }
        if !(MAX_UNKNOWN_RETRIES_MIN..=MAX_UNKNOWN_RETRIES_MAX).contains(&caps.max_unknown_retries)
        {
            return Err(ScenarioError::new(
                "traffic_caps_invalid",
                format!(
                    "traffic.caps.max_unknown_retries {} is outside \
                     {MAX_UNKNOWN_RETRIES_MIN}..={MAX_UNKNOWN_RETRIES_MAX}",
                    caps.max_unknown_retries
                ),
            ));
        }
        if !(MAX_MEMPOOL_BACKLOG_TX_MIN..=MAX_MEMPOOL_BACKLOG_TX_MAX)
            .contains(&caps.max_mempool_backlog_tx)
        {
            return Err(ScenarioError::new(
                "traffic_caps_invalid",
                format!(
                    "traffic.caps.max_mempool_backlog_tx {} is outside \
                     {MAX_MEMPOOL_BACKLOG_TX_MIN}..={MAX_MEMPOOL_BACKLOG_TX_MAX}",
                    caps.max_mempool_backlog_tx
                ),
            ));
        }
        if !(REORG_RESUBMIT_MAX_MIN..=REORG_RESUBMIT_MAX_MAX).contains(&caps.reorg_resubmit_max) {
            return Err(ScenarioError::new(
                "traffic_caps_invalid",
                format!(
                    "traffic.caps.reorg_resubmit_max {} is outside \
                     {REORG_RESUBMIT_MAX_MIN}..={REORG_RESUBMIT_MAX_MAX}",
                    caps.reorg_resubmit_max
                ),
            ));
        }
        Ok(())
    }
}
