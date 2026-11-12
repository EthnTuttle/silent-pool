//! Versioned lab scenario vocabulary.
//!
//! Version 1 deliberately separates the network, node, pool, front, identity,
//! worker and miner concepts. Validation is structural and semantic, and an
//! unknown version or unknown field is rejected before any process starts.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::ids::{
    ArkBackendId, ConnectionId, FrontId, IdentityId, LinkId, MinerId, NetworkId, NodeId, PoolId,
    RunId, WorkerId,
};
use crate::wide::{ReferenceProfile, Target256, WideU128};

pub const SCENARIO_VERSION: u32 = 1;

/// A scenario validation failure with a stable code for tests and reports.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {detail}")]
pub struct ScenarioError {
    code: &'static str,
    detail: String,
}

impl ScenarioError {
    pub fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }

    pub fn code(&self) -> &'static str {
        self.code
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }
}

/// Additional facts a successful structural/semantic validation observed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ValidationReport {
    pub warnings: Vec<String>,
    pub required_capabilities: BTreeSet<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub scenario_version: u32,
    pub run: RunSpec,
    pub network: NetworkSpec,
    pub nodes: Vec<NodeSpec>,
    pub pools: Vec<PoolSpec>,
    pub fronts: Vec<FrontSpec>,
    pub identities: Vec<IdentitySpec>,
    pub miners: Vec<MinerSpec>,
    /// One explicitly configured wallet writer per Ark mining identity.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub wallet_agents: Vec<crate::wallet_bridge::WalletAgentSpec>,
    #[serde(default)]
    pub ark_backends: Vec<ArkBackendSpec>,
    pub resources: ResourceSpec,
    #[serde(default)]
    pub links: Vec<LinkSpec>,
    #[serde(default)]
    pub phases: Vec<PhaseSpec>,
    #[serde(default)]
    pub assertions: Vec<AssertionSpec>,
    #[serde(default)]
    pub statistics: Vec<StatisticSpec>,
    /// Optional transaction traffic contract (traffic plan Section 11).
    ///
    /// Absent in every pre-traffic scenario: those parse and validate
    /// unchanged. The field is additive at `scenario_version` 1, so an
    /// old runner rejects a traffic-bearing scenario as an unknown field
    /// (fail closed, before any validation or process start) while a new
    /// runner accepts both.
    #[serde(default)]
    pub traffic: Option<crate::traffic::spec::TrafficSpec>,
    /// Opt-in expanded-access profile (live-lab plan section 5).
    ///
    /// Absent in every pre-access scenario: those parse, validate and serve
    /// byte-for-byte as today (unauthenticated loopback). When true, the run
    /// generates per-user and operator access claims under
    /// `<run>/private/access/`, every controller route except the closed
    /// public static bootstrap list requires a Bearer token, command replay
    /// is bound to the authenticated principal, and reads are served through
    /// a server-side user projection.
    #[serde(default)]
    pub expanded_access: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunSpec {
    pub id: RunId,
    pub seed: u64,
    pub time_policy: TimePolicy,
    pub warmup: WarmupSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimePolicy {
    pub chain_time: ChainTime,
    pub miner_ntime: MinerNtime,
    #[serde(default)]
    pub mock_node_time: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChainTime {
    NodeHeader,
    Mock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MinerNtime {
    Hold,
    Roll,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WarmupSpec {
    pub blocks: u64,
    pub window_fills: u64,
    pub exclude_from_statistics: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkSpec {
    pub id: NetworkId,
    pub consensus_profile: ConsensusProfileSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsensusProfileSpec {
    pub required_capability: String,
    /// Advisory only. Effective field comparison is authoritative; a matching
    /// fingerprint never substitutes for checking the fields.
    #[serde(default)]
    pub profile_fingerprint: Option<String>,
    #[serde(default)]
    pub requirements: ConsensusRequirements,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsensusRequirements {
    #[serde(default)]
    pub pow_limit_bits: Option<String>,
    #[serde(default)]
    pub retarget_floor_bits: Option<String>,
    #[serde(default)]
    pub initial_target_bits: Option<String>,
    #[serde(default)]
    pub target_spacing_secs: Option<u64>,
    #[serde(default)]
    pub target_timespan_secs: Option<u64>,
    #[serde(default)]
    pub retarget_interval_blocks: Option<u64>,
    #[serde(default)]
    pub no_pow_retargeting: Option<bool>,
    #[serde(default)]
    pub allow_min_difficulty_blocks: Option<bool>,
    #[serde(default)]
    pub minimum_difficulty_time_gap_secs: Option<u64>,
    #[serde(default)]
    pub subsidy_halving_interval_blocks: Option<u64>,
    #[serde(default)]
    pub subsidy_initial_sats: Option<WideU128>,
    #[serde(default)]
    pub coinbase_maturity_blocks: Option<u64>,
    #[serde(default)]
    pub witness_commitment_supported: Option<bool>,
    #[serde(default)]
    pub tdp_session_cap: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeSpec {
    pub id: NodeId,
    pub network_id: NetworkId,
    pub implementation: String,
    pub p2p: ListenSpec,
    pub rpc: ListenSpec,
    pub tdp: TdpSpec,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListenSpec {
    pub listen: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TdpSpec {
    pub listen: String,
    pub session_cap: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PoolSpec {
    pub id: PoolId,
    pub network_id: NetworkId,
    pub node_ids: Vec<NodeId>,
    pub implementation: String,
    pub work_profile: WorkProfileSpec,
    pub storage: PoolStorageSpec,
    #[serde(default)]
    pub ark_backend_id: Option<ArkBackendId>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PoolStorageSpec {
    pub redis_instance_id: String,
    pub postgres_database_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkProfileSpec {
    pub work_unit_version: u16,
    #[serde(default)]
    pub reference_target: Option<String>,
    #[serde(default)]
    pub share_work_multiplier: Option<u64>,
    #[serde(default)]
    pub max_work_multiplier: Option<u64>,
    #[serde(default)]
    pub max_window_target_units: Option<WideU128>,
    pub window_rule: WindowRule,
    pub window_n: u64,
    #[serde(default)]
    pub max_slice_shares: Option<u64>,
    #[serde(default)]
    pub legacy_share_difficulty: Option<WideU128>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowRule {
    ShareWorkLegacy,
    NetworkBlockWork,
}

impl WindowRule {
    pub fn code(self) -> u16 {
        match self {
            Self::ShareWorkLegacy => 0,
            Self::NetworkBlockWork => 1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrontSpec {
    pub id: FrontId,
    pub pool_id: PoolId,
    pub roles: Vec<FrontRole>,
    pub listen: FrontListen,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrontRole {
    Front,
    Payout,
    Api,
    Ark,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrontListen {
    pub sv2: Option<String>,
    #[serde(default)]
    pub sv1: Option<String>,
    #[serde(default)]
    pub jd: Option<String>,
    /// Pool API/HTTP listener. Optional for fixture fronts; the built-in real
    /// runtime declares it so a rendered pool TOML can bind the real API.
    #[serde(default)]
    pub api: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentitySpec {
    pub id: IdentityId,
    pub address_source: AddressSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AddressSource {
    Fixture { fixture: String },
    Explicit { address: String },
}

impl AddressSource {
    pub fn is_nonempty(&self) -> bool {
        match self {
            Self::Fixture { fixture } => !fixture.trim().is_empty(),
            Self::Explicit { address } => !address.trim().is_empty(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MinerSpec {
    pub id: MinerId,
    pub implementation: String,
    pub connections: Vec<ConnectionSpec>,
    pub workers: Vec<WorkerSpec>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionSpec {
    pub id: ConnectionId,
    pub front_id: FrontId,
    pub identity_id: IdentityId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerSpec {
    pub id: WorkerId,
    pub identity_id: IdentityId,
    pub connection_id: ConnectionId,
    pub budget: BudgetSpec,
    #[serde(default)]
    pub nonce_start: Option<u64>,
    #[serde(default)]
    pub nonce_len: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetSpec {
    #[serde(default)]
    pub hashes_per_second: Option<WideU128>,
    #[serde(default)]
    pub total_hashes: Option<WideU128>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArkBackendSpec {
    pub id: ArkBackendId,
    pub implementation: String,
    pub listen: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local: Option<crate::ark_service::LocalArkSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceSpec {
    #[serde(default)]
    pub redis_instances: Vec<RedisInstanceSpec>,
    #[serde(default)]
    pub postgres_databases: Vec<PostgresDatabaseSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RedisInstanceSpec {
    pub id: String,
    pub owner_pool_id: PoolId,
    pub port: u16,
    pub dir: String,
    #[serde(default)]
    pub password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PostgresDatabaseSpec {
    pub id: String,
    pub owner_pool_id: PoolId,
    pub database: String,
    pub role: String,
    /// `0` asks the allocator for a per-pool cluster port.
    #[serde(default)]
    pub port: u16,
    /// Private per-run role password; omitted in structural fixtures.
    #[serde(default)]
    pub password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkSpec {
    pub id: LinkId,
    #[serde(rename = "from")]
    pub from: LinkEndpoint,
    #[serde(rename = "to")]
    pub to: LinkEndpoint,
    #[serde(default)]
    pub policy: LinkPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkEndpoint {
    pub kind: EndpointKind,
    pub id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndpointKind {
    Node,
    Pool,
    Front,
    Miner,
    Identity,
    Ark,
    Redis,
    Postgres,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkPolicy {
    #[default]
    Normal,
    Partition,
    Latency {
        delay_ms: u64,
    },
    Loss {
        loss_percent: u8,
    },
}

impl LinkPolicy {
    pub fn is_normal(&self) -> bool {
        matches!(self, Self::Normal)
    }
}

/// Versioned post-phase convergence action. It is opt-in per phase: existing
/// scenarios never resume miners after their declared commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConvergenceSpec {
    /// Native convergence DSL version. Only version 1 is executable.
    pub version: u16,
    /// Cooperative convergence budget, checked between potentially blocking
    /// observations. It is not a hard deadline for one helper/RPC round trip.
    pub timeout_secs: u64,
    /// Maximum number of atomic view assertions (including revalidation).
    pub max_observations: u32,
    /// Whether real SV2 workers may be resumed once to work out of a fork.
    #[serde(default)]
    pub allow_resume: bool,
    /// Hard cap on resume commands per miner, independent of observation cap.
    #[serde(default)]
    pub max_resume_attempts: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhaseSpec {
    pub id: String,
    #[serde(default)]
    pub commands: Vec<serde_json::Value>,
    /// Optional explicit convergence action, executed once after all declared
    /// phases regardless of the declaring phase's position.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub convergence: Option<ConvergenceSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssertionSpec {
    pub id: String,
    pub kind: AssertionKind,
    pub predicate: String,
    pub deadline_secs: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssertionKind {
    Evidence,
    Canonical,
    Accounting,
    Statistics,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatisticSpec {
    pub metric: String,
    pub estimator: String,
    pub planned_n: u64,
    pub stopping_rule: serde_json::Value,
    #[serde(default)]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub exclusions: Vec<String>,
    #[serde(default)]
    pub unit: Option<String>,
}

/// Parse a scenario, checking the version before strict deserialization.
pub fn parse_scenario(json: &str) -> Result<Scenario, ScenarioError> {
    let raw: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| ScenarioError::new("scenario_json_invalid", e.to_string()))?;
    let version = raw
        .get("scenario_version")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| {
            ScenarioError::new(
                "scenario_version_missing",
                "scenario_version must be an integer",
            )
        })?;
    if version != u64::from(SCENARIO_VERSION) {
        return Err(ScenarioError::new(
            "scenario_version_unsupported",
            format!("scenario_version {version} is not supported (expected {SCENARIO_VERSION})"),
        ));
    }
    serde_json::from_value(raw)
        .map_err(|e| ScenarioError::new("scenario_schema_invalid", e.to_string()))
}

impl Scenario {
    pub fn from_json(json: &str) -> Result<Self, ScenarioError> {
        parse_scenario(json)
    }

    pub fn validate(&self) -> Result<ValidationReport, ScenarioError> {
        if self.scenario_version != SCENARIO_VERSION {
            return Err(ScenarioError::new(
                "scenario_version_unsupported",
                format!("scenario_version {} is not 1", self.scenario_version),
            ));
        }

        let mut report = ValidationReport::default();
        self.validate_run(&mut report)?;
        self.validate_network(&mut report)?;
        self.validate_nodes()?;
        self.validate_pools(&mut report)?;
        self.validate_fronts()?;
        self.validate_identities()?;
        self.validate_miners()?;
        self.validate_ark()?;
        crate::wallet_bridge::validate(self)?;
        self.validate_resources()?;
        self.validate_links()?;
        self.validate_phases()?;
        self.validate_assertions()?;
        self.validate_statistics()?;
        self.validate_traffic(&mut report)?;
        self.validate_observers(&mut report)?;

        Ok(report)
    }

    fn validate_run(&self, report: &mut ValidationReport) -> Result<(), ScenarioError> {
        if self.run.id.as_str().is_empty() {
            return Err(ScenarioError::new("run_id_empty", "run.id is empty"));
        }
        match (
            self.run.time_policy.chain_time,
            self.run.time_policy.mock_node_time,
        ) {
            (ChainTime::Mock, None) => {
                return Err(ScenarioError::new(
                    "time_policy_mock_missing",
                    "chain_time=mock requires mock_node_time",
                ));
            }
            (ChainTime::NodeHeader, Some(_)) => {
                report.warnings.push(
                    "mock_node_time is set while chain_time=node_header; node header time wins"
                        .to_string(),
                );
            }
            _ => {}
        }
        if self.run.warmup.exclude_from_statistics
            && self.run.warmup.blocks == 0
            && self.run.warmup.window_fills == 0
        {
            report.warnings.push(
                "warmup exclude_from_statistics=true but no warmup blocks or window fills are declared"
                    .to_string(),
            );
        }
        Ok(())
    }

    fn validate_network(&self, report: &mut ValidationReport) -> Result<(), ScenarioError> {
        if self.network.id.as_str().is_empty() {
            return Err(ScenarioError::new(
                "network_id_empty",
                "network.id is empty",
            ));
        }
        let profile = &self.network.consensus_profile;
        if profile.required_capability.trim().is_empty() {
            return Err(ScenarioError::new(
                "capability_missing",
                "network.consensus_profile.required_capability is empty",
            ));
        }
        report
            .required_capabilities
            .insert(profile.required_capability.clone());
        if let Some(fingerprint) = &profile.profile_fingerprint {
            validate_fingerprint(fingerprint)?;
            report.warnings.push(
                "profile_fingerprint is advisory; effective consensus fields are authoritative"
                    .to_string(),
            );
        }
        let req = &profile.requirements;
        for (name, bits) in [
            ("pow_limit_bits", req.pow_limit_bits.as_deref()),
            ("retarget_floor_bits", req.retarget_floor_bits.as_deref()),
            ("initial_target_bits", req.initial_target_bits.as_deref()),
        ] {
            if let Some(bits) = bits {
                decode_bits(bits).map_err(|e| {
                    ScenarioError::new("consensus_bits_invalid", format!("{name}: {e}"))
                })?;
            }
        }
        if let Some(spacing) = req.target_spacing_secs {
            if spacing == 0 {
                return Err(ScenarioError::new(
                    "consensus_spacing_invalid",
                    "target_spacing_secs must be > 0",
                ));
            }
        }
        if let Some(timespan) = req.target_timespan_secs {
            if timespan == 0 {
                return Err(ScenarioError::new(
                    "consensus_timespan_invalid",
                    "target_timespan_secs must be > 0",
                ));
            }
        }
        if let Some(interval) = req.retarget_interval_blocks {
            if interval == 0 {
                return Err(ScenarioError::new(
                    "consensus_interval_invalid",
                    "retarget_interval_blocks must be > 0",
                ));
            }
        }
        if let Some(cap) = req.tdp_session_cap {
            if cap == 0 {
                return Err(ScenarioError::new(
                    "tdp_session_cap_invalid",
                    "tdp_session_cap must be > 0",
                ));
            }
        }
        Ok(())
    }

    fn validate_nodes(&self) -> Result<(), ScenarioError> {
        let mut seen = BTreeSet::new();
        for node in &self.nodes {
            if !seen.insert(node.id.clone()) {
                return Err(ScenarioError::new(
                    "duplicate_node_id",
                    format!("duplicate node id {}", node.id),
                ));
            }
            if node.network_id != self.network.id {
                return Err(ScenarioError::new(
                    "node_network_mismatch",
                    format!("node {} is not on network {}", node.id, self.network.id),
                ));
            }
            if node.implementation.trim().is_empty() {
                return Err(ScenarioError::new(
                    "node_implementation_empty",
                    format!("node {} has no implementation", node.id),
                ));
            }
            parse_listen(&node.p2p.listen)?;
            parse_listen(&node.rpc.listen)?;
            parse_listen(&node.tdp.listen)?;
            if node.tdp.session_cap == 0 {
                return Err(ScenarioError::new(
                    "tdp_session_cap_invalid",
                    format!("node {} has zero tdp session_cap", node.id),
                ));
            }
        }
        if self.nodes.is_empty() {
            return Err(ScenarioError::new("nodes_empty", "scenario has no nodes"));
        }
        Ok(())
    }

    fn validate_pools(&self, report: &mut ValidationReport) -> Result<(), ScenarioError> {
        if self.pools.is_empty() {
            return Err(ScenarioError::new("pools_empty", "scenario has no pools"));
        }
        let nodes: BTreeSet<_> = self.nodes.iter().map(|n| &n.id).collect();
        let mut seen = BTreeSet::new();
        for pool in &self.pools {
            if !seen.insert(pool.id.clone()) {
                return Err(ScenarioError::new(
                    "duplicate_pool_id",
                    format!("duplicate pool id {}", pool.id),
                ));
            }
            if pool.network_id != self.network.id {
                return Err(ScenarioError::new(
                    "pool_network_mismatch",
                    format!("pool {} is not on network {}", pool.id, self.network.id),
                ));
            }
            if pool.node_ids.is_empty() {
                return Err(ScenarioError::new(
                    "pool_nodes_empty",
                    format!("pool {} names no node", pool.id),
                ));
            }
            let mut pool_nodes = BTreeSet::new();
            for node_id in &pool.node_ids {
                if !nodes.contains(node_id) {
                    return Err(ScenarioError::new(
                        "pool_node_unknown",
                        format!("pool {} names unknown node {}", pool.id, node_id),
                    ));
                }
                if !pool_nodes.insert(node_id) {
                    return Err(ScenarioError::new(
                        "pool_node_duplicate",
                        format!("pool {} names node {} twice", pool.id, node_id),
                    ));
                }
            }
            if pool.implementation.trim().is_empty() {
                return Err(ScenarioError::new(
                    "pool_implementation_empty",
                    format!("pool {} has no implementation", pool.id),
                ));
            }
            if pool.storage.redis_instance_id.trim().is_empty()
                || pool.storage.postgres_database_id.trim().is_empty()
            {
                return Err(ScenarioError::new(
                    "pool_storage_missing",
                    format!("pool {} has an empty storage id", pool.id),
                ));
            }
            validate_work_profile(pool, &self.network, report)?;
            if let Some(ark) = &pool.ark_backend_id {
                if !self.ark_backends.iter().any(|a| &a.id == ark) {
                    return Err(ScenarioError::new(
                        "pool_ark_unknown",
                        format!("pool {} names unknown Ark backend {}", pool.id, ark),
                    ));
                }
            }
            // Requirement relation: m_cfg <= max_work_multiplier <= 2^32 and
            // Q <= max_window_target_units <= 2^64 are enforced in
            // validate_work_profile.
            if let Some(profile) = &pool.work_profile.reference_target {
                if let Ok(reference) = ReferenceProfile::parse_hex(profile) {
                    report.warnings.push(format!(
                        "pool {} work reference fingerprint is {}",
                        pool.id,
                        reference.fingerprint()
                    ));
                }
            }
        }
        Ok(())
    }

    fn validate_fronts(&self) -> Result<(), ScenarioError> {
        let pools: BTreeSet<_> = self.pools.iter().map(|p| &p.id).collect();
        let mut seen = BTreeSet::new();
        for front in &self.fronts {
            if !seen.insert(front.id.clone()) {
                return Err(ScenarioError::new(
                    "duplicate_front_id",
                    format!("duplicate front id {}", front.id),
                ));
            }
            if !pools.contains(&front.pool_id) {
                return Err(ScenarioError::new(
                    "front_pool_unknown",
                    format!("front {} names unknown pool {}", front.id, front.pool_id),
                ));
            }
            if front.roles.is_empty() {
                return Err(ScenarioError::new(
                    "front_roles_empty",
                    format!("front {} has no roles", front.id),
                ));
            }
            let mut roles = BTreeSet::new();
            for role in &front.roles {
                if !roles.insert(*role) {
                    return Err(ScenarioError::new(
                        "front_role_duplicate",
                        format!("front {} repeats role {role:?}", front.id),
                    ));
                }
            }
            let sv2 = front.listen.sv2.as_deref().ok_or_else(|| {
                ScenarioError::new(
                    "front_sv2_missing",
                    format!("front {} has no SV2 listener", front.id),
                )
            })?;
            parse_listen(sv2)?;
            for listener in [&front.listen.sv1, &front.listen.jd, &front.listen.api]
                .into_iter()
                .flatten()
            {
                parse_listen(listener)?;
            }
        }
        Ok(())
    }

    fn validate_identities(&self) -> Result<(), ScenarioError> {
        let mut seen = BTreeSet::new();
        for identity in &self.identities {
            if !seen.insert(identity.id.clone()) {
                return Err(ScenarioError::new(
                    "duplicate_identity_id",
                    format!("duplicate identity id {}", identity.id),
                ));
            }
            if !identity.address_source.is_nonempty() {
                return Err(ScenarioError::new(
                    "identity_address_empty",
                    format!("identity {} has an empty address source", identity.id),
                ));
            }
        }
        Ok(())
    }

    fn validate_miners(&self) -> Result<(), ScenarioError> {
        if self.miners.is_empty() && !self.ark_backends.iter().any(|a| a.local.is_some()) {
            return Err(ScenarioError::new(
                "miners_empty",
                "scenario has no miner processes",
            ));
        }
        let fronts: BTreeSet<_> = self.fronts.iter().map(|f| &f.id).collect();
        let identities: BTreeSet<_> = self.identities.iter().map(|i| &i.id).collect();
        let mut miner_ids = BTreeSet::new();
        let mut connection_ids = BTreeSet::new();
        let mut worker_ids = BTreeSet::new();
        for miner in &self.miners {
            if !miner_ids.insert(miner.id.clone()) {
                return Err(ScenarioError::new(
                    "duplicate_miner_id",
                    format!("duplicate miner id {}", miner.id),
                ));
            }
            if miner.implementation.trim().is_empty() {
                return Err(ScenarioError::new(
                    "miner_implementation_empty",
                    format!("miner {} has no implementation", miner.id),
                ));
            }
            if miner.connections.is_empty() {
                return Err(ScenarioError::new(
                    "miner_connections_empty",
                    format!("miner {} has no connections", miner.id),
                ));
            }
            let mut own_connections = BTreeMap::new();
            for connection in &miner.connections {
                if !connection_ids.insert(connection.id.clone()) {
                    return Err(ScenarioError::new(
                        "duplicate_connection_id",
                        format!("duplicate connection id {}", connection.id),
                    ));
                }
                if !fronts.contains(&connection.front_id) {
                    return Err(ScenarioError::new(
                        "connection_front_unknown",
                        format!(
                            "connection {} names unknown front {}",
                            connection.id, connection.front_id
                        ),
                    ));
                }
                if !identities.contains(&connection.identity_id) {
                    return Err(ScenarioError::new(
                        "connection_identity_unknown",
                        format!(
                            "connection {} names unknown identity {}",
                            connection.id, connection.identity_id
                        ),
                    ));
                }
                own_connections.insert(connection.id.clone(), connection.identity_id.clone());
            }
            if miner.workers.is_empty() {
                return Err(ScenarioError::new(
                    "miner_workers_empty",
                    format!("miner {} has no workers", miner.id),
                ));
            }
            let mut ranges: BTreeMap<ConnectionId, Vec<(u64, u64)>> = BTreeMap::new();
            for worker in &miner.workers {
                if !worker_ids.insert(worker.id.clone()) {
                    return Err(ScenarioError::new(
                        "duplicate_worker_id",
                        format!("duplicate worker id {}", worker.id),
                    ));
                }
                if !identities.contains(&worker.identity_id) {
                    return Err(ScenarioError::new(
                        "worker_identity_unknown",
                        format!(
                            "worker {} names unknown identity {}",
                            worker.id, worker.identity_id
                        ),
                    ));
                }
                let Some(connection_identity) = own_connections.get(&worker.connection_id) else {
                    return Err(ScenarioError::new(
                        "worker_connection_unknown",
                        format!(
                            "worker {} names connection {} outside miner {}",
                            worker.id, worker.connection_id, miner.id
                        ),
                    ));
                };
                if connection_identity != &worker.identity_id {
                    return Err(ScenarioError::new(
                        "worker_identity_mismatch",
                        format!(
                            "worker {} identity {} does not match connection {} identity {}",
                            worker.id,
                            worker.identity_id,
                            worker.connection_id,
                            connection_identity
                        ),
                    ));
                }
                if worker.budget.hashes_per_second.is_none() && worker.budget.total_hashes.is_none()
                {
                    return Err(ScenarioError::new(
                        "worker_budget_missing",
                        format!("worker {} declares no budget", worker.id),
                    ));
                }
                if let (Some(start), Some(len)) = (worker.nonce_start, worker.nonce_len) {
                    if len == 0 {
                        return Err(ScenarioError::new(
                            "worker_range_empty",
                            format!("worker {} has a zero nonce range", worker.id),
                        ));
                    }
                    let end = start.checked_add(len).ok_or_else(|| {
                        ScenarioError::new(
                            "worker_range_overflow",
                            format!("worker {} nonce range overflows", worker.id),
                        )
                    })?;
                    let existing = ranges.entry(worker.connection_id.clone()).or_default();
                    for (other_start, other_end) in existing.iter() {
                        if start < *other_end && *other_start < end {
                            return Err(ScenarioError::new(
                                "worker_ranges_overlap",
                                format!(
                                    "worker {} nonce range overlaps another worker on connection {}",
                                    worker.id, worker.connection_id
                                ),
                            ));
                        }
                    }
                    existing.push((start, end));
                }
            }
        }
        Ok(())
    }

    fn validate_ark(&self) -> Result<(), ScenarioError> {
        let mut seen = BTreeSet::new();
        for ark in &self.ark_backends {
            if !seen.insert(ark.id.clone()) {
                return Err(ScenarioError::new(
                    "duplicate_ark_id",
                    format!("duplicate Ark backend id {}", ark.id),
                ));
            }
            if ark.implementation.trim().is_empty() {
                return Err(ScenarioError::new(
                    "ark_implementation_empty",
                    format!("Ark backend {} has no implementation", ark.id),
                ));
            }
            parse_listen(&ark.listen)?;
            if let Some(local) = &ark.local {
                local
                    .validate(self, ark)
                    .map_err(|e| ScenarioError::new("local_ark_invalid", e.to_string()))?;
            }
        }
        Ok(())
    }

    fn validate_resources(&self) -> Result<(), ScenarioError> {
        let pools: BTreeSet<_> = self.pools.iter().map(|p| &p.id).collect();

        let mut redis_ids = BTreeSet::new();
        let mut redis_ports = BTreeSet::new();
        let mut redis_owners: BTreeMap<PoolId, String> = BTreeMap::new();
        for redis in &self.resources.redis_instances {
            if redis.id.trim().is_empty() {
                return Err(ScenarioError::new(
                    "redis_id_empty",
                    "a Redis instance has an empty id",
                ));
            }
            if !redis_ids.insert(redis.id.clone()) {
                return Err(ScenarioError::new(
                    "duplicate_redis_id",
                    format!("duplicate Redis instance id {}", redis.id),
                ));
            }
            if !pools.contains(&redis.owner_pool_id) {
                return Err(ScenarioError::new(
                    "redis_owner_unknown",
                    format!(
                        "Redis instance {} names unknown owner pool {}",
                        redis.id, redis.owner_pool_id
                    ),
                ));
            }
            if redis.port != 0 && !redis_ports.insert(redis.port) {
                return Err(ScenarioError::new(
                    "redis_port_collision",
                    format!("Redis port {} is requested twice", redis.port),
                ));
            }
            if redis.dir.trim().is_empty()
                || redis.dir.contains("..")
                || std::path::Path::new(&redis.dir).is_absolute()
            {
                return Err(ScenarioError::new(
                    "redis_dir_invalid",
                    format!(
                        "Redis instance {} dir must be a relative path without ..",
                        redis.id
                    ),
                ));
            }
            if let Some(previous) =
                redis_owners.insert(redis.owner_pool_id.clone(), redis.id.clone())
            {
                return Err(ScenarioError::new(
                    "redis_pool_shared",
                    format!(
                        "pool {} owns two Redis instances ({} and {})",
                        redis.owner_pool_id, previous, redis.id
                    ),
                ));
            }
        }

        let mut pg_ids = BTreeSet::new();
        let mut pg_databases = BTreeSet::new();
        let mut pg_roles = BTreeSet::new();
        let mut pg_ports = BTreeSet::new();
        let mut pg_owners: BTreeMap<PoolId, String> = BTreeMap::new();
        for pg in &self.resources.postgres_databases {
            if pg.id.trim().is_empty() {
                return Err(ScenarioError::new(
                    "postgres_id_empty",
                    "a Postgres database has an empty id",
                ));
            }
            if !pg_ids.insert(pg.id.clone()) {
                return Err(ScenarioError::new(
                    "duplicate_postgres_id",
                    format!("duplicate Postgres database id {}", pg.id),
                ));
            }
            if !pools.contains(&pg.owner_pool_id) {
                return Err(ScenarioError::new(
                    "postgres_owner_unknown",
                    format!(
                        "Postgres database {} names unknown owner pool {}",
                        pg.id, pg.owner_pool_id
                    ),
                ));
            }
            if pg.database.trim().is_empty() || pg.role.trim().is_empty() {
                return Err(ScenarioError::new(
                    "postgres_name_empty",
                    format!("Postgres database {} has an empty database or role", pg.id),
                ));
            }
            if !valid_sql_identifier(&pg.database) || !valid_sql_identifier(&pg.role) {
                return Err(ScenarioError::new(
                    "postgres_identifier_invalid",
                    format!(
                        "Postgres database {} must use [a-z_][a-z0-9_]* identifiers (<=63 bytes)",
                        pg.id
                    ),
                ));
            }
            if !pg_databases.insert(pg.database.clone()) {
                return Err(ScenarioError::new(
                    "postgres_database_collision",
                    format!("Postgres database {} is declared twice", pg.database),
                ));
            }
            if !pg_roles.insert(pg.role.clone()) {
                return Err(ScenarioError::new(
                    "postgres_role_collision",
                    format!("Postgres role {} is declared twice", pg.role),
                ));
            }
            if pg.port != 0 && !pg_ports.insert(pg.port) {
                return Err(ScenarioError::new(
                    "postgres_port_collision",
                    format!("Postgres port {} is requested twice", pg.port),
                ));
            }
            if let Some(previous) = pg_owners.insert(pg.owner_pool_id.clone(), pg.id.clone()) {
                return Err(ScenarioError::new(
                    "postgres_pool_shared",
                    format!(
                        "pool {} owns two Postgres databases ({} and {})",
                        pg.owner_pool_id, previous, pg.id
                    ),
                ));
            }
        }

        for pool in &self.pools {
            let redis = self
                .resources
                .redis_instances
                .iter()
                .find(|r| r.id == pool.storage.redis_instance_id)
                .ok_or_else(|| {
                    ScenarioError::new(
                        "pool_redis_unknown",
                        format!(
                            "pool {} names unknown Redis instance {}",
                            pool.id, pool.storage.redis_instance_id
                        ),
                    )
                })?;
            if redis.owner_pool_id != pool.id {
                return Err(ScenarioError::new(
                    "pool_redis_owner_mismatch",
                    format!(
                        "pool {} names Redis {} owned by {}",
                        pool.id, redis.id, redis.owner_pool_id
                    ),
                ));
            }
            let pg = self
                .resources
                .postgres_databases
                .iter()
                .find(|p| p.id == pool.storage.postgres_database_id)
                .ok_or_else(|| {
                    ScenarioError::new(
                        "pool_postgres_unknown",
                        format!(
                            "pool {} names unknown Postgres database {}",
                            pool.id, pool.storage.postgres_database_id
                        ),
                    )
                })?;
            if pg.owner_pool_id != pool.id {
                return Err(ScenarioError::new(
                    "pool_postgres_owner_mismatch",
                    format!(
                        "pool {} names Postgres {} owned by {}",
                        pool.id, pg.id, pg.owner_pool_id
                    ),
                ));
            }
        }

        // Every listener declared in the scenario must have a unique endpoint
        // on the loopback namespace, except port 0 which the allocator fills.
        let mut ports: BTreeMap<(String, u16), String> = BTreeMap::new();
        for node in &self.nodes {
            for (kind, listener) in [
                ("node.p2p", &node.p2p.listen),
                ("node.rpc", &node.rpc.listen),
                ("node.tdp", &node.tdp.listen),
            ] {
                record_listener(&mut ports, kind, node.id.as_ref(), listener)?;
            }
        }
        for front in &self.fronts {
            if let Some(listener) = &front.listen.sv2 {
                record_listener(&mut ports, "front.sv2", front.id.as_ref(), listener)?;
            }
            if let Some(listener) = &front.listen.sv1 {
                record_listener(&mut ports, "front.sv1", front.id.as_ref(), listener)?;
            }
            if let Some(listener) = &front.listen.jd {
                record_listener(&mut ports, "front.jd", front.id.as_ref(), listener)?;
            }
            if let Some(listener) = &front.listen.api {
                record_listener(&mut ports, "front.api", front.id.as_ref(), listener)?;
            }
        }
        for ark in &self.ark_backends {
            record_listener(&mut ports, "ark", ark.id.as_ref(), &ark.listen)?;
        }
        Ok(())
    }

    fn validate_links(&self) -> Result<(), ScenarioError> {
        let mut seen = BTreeSet::new();
        for link in &self.links {
            if !seen.insert(link.id.clone()) {
                return Err(ScenarioError::new(
                    "duplicate_link_id",
                    format!("duplicate link id {}", link.id),
                ));
            }
            self.endpoint_exists(&link.from)?;
            self.endpoint_exists(&link.to)?;
            if link.from.kind == link.to.kind && link.from.id == link.to.id {
                return Err(ScenarioError::new(
                    "link_self_loop",
                    format!("link {} connects an endpoint to itself", link.id),
                ));
            }
            match &link.policy {
                LinkPolicy::Loss { loss_percent } if *loss_percent > 100 => {
                    return Err(ScenarioError::new(
                        "link_loss_invalid",
                        format!("link {} loss_percent exceeds 100", link.id),
                    ));
                }
                _ => {}
            }
        }
        // No link is implicit: every connection and every pool->node relation
        // must be named.
        for miner in &self.miners {
            for connection in &miner.connections {
                let present = self.links.iter().any(|link| {
                    link.from.kind == EndpointKind::Miner
                        && link.from.id == miner.id.as_str()
                        && link.to.kind == EndpointKind::Front
                        && link.to.id == connection.front_id.as_str()
                });
                if !present {
                    return Err(ScenarioError::new(
                        "link_missing",
                        format!(
                            "connection {} has no miner {} -> front {} link",
                            connection.id, miner.id, connection.front_id
                        ),
                    ));
                }
            }
        }
        for pool in &self.pools {
            for node_id in &pool.node_ids {
                let present = self.links.iter().any(|link| {
                    link.from.kind == EndpointKind::Pool
                        && link.from.id == pool.id.as_str()
                        && link.to.kind == EndpointKind::Node
                        && link.to.id == node_id.as_str()
                });
                if !present {
                    return Err(ScenarioError::new(
                        "link_missing",
                        format!("pool {} -> node {} link is missing", pool.id, node_id),
                    ));
                }
            }
        }
        Ok(())
    }

    fn endpoint_exists(&self, endpoint: &LinkEndpoint) -> Result<(), ScenarioError> {
        let exists = match endpoint.kind {
            EndpointKind::Node => self.nodes.iter().any(|n| n.id.as_str() == endpoint.id),
            EndpointKind::Pool => self.pools.iter().any(|p| p.id.as_str() == endpoint.id),
            EndpointKind::Front => self.fronts.iter().any(|f| f.id.as_str() == endpoint.id),
            EndpointKind::Miner => self.miners.iter().any(|m| m.id.as_str() == endpoint.id),
            EndpointKind::Identity => self.identities.iter().any(|i| i.id.as_str() == endpoint.id),
            EndpointKind::Ark => self
                .ark_backends
                .iter()
                .any(|a| a.id.as_str() == endpoint.id),
            EndpointKind::Redis => self
                .resources
                .redis_instances
                .iter()
                .any(|r| r.id == endpoint.id),
            EndpointKind::Postgres => self
                .resources
                .postgres_databases
                .iter()
                .any(|p| p.id == endpoint.id),
        };
        if exists {
            Ok(())
        } else {
            Err(ScenarioError::new(
                "link_endpoint_unknown",
                format!("unknown link endpoint {:?}:{}", endpoint.kind, endpoint.id),
            ))
        }
    }

    fn validate_phases(&self) -> Result<(), ScenarioError> {
        let mut seen = BTreeSet::new();
        for phase in &self.phases {
            if phase.id.trim().is_empty() {
                return Err(ScenarioError::new(
                    "phase_id_empty",
                    "a phase has an empty id",
                ));
            }
            if !seen.insert(phase.id.clone()) {
                return Err(ScenarioError::new(
                    "duplicate_phase_id",
                    format!("duplicate phase id {}", phase.id),
                ));
            }
            for command in &phase.commands {
                if !command.is_object() {
                    return Err(ScenarioError::new(
                        "phase_command_invalid",
                        format!("phase {} has a non-object command", phase.id),
                    ));
                }
            }
            if let Some(convergence) = &phase.convergence {
                if convergence.version != 1 {
                    return Err(ScenarioError::new(
                        "convergence_version_unsupported",
                        format!(
                            "phase {} has unsupported convergence version {}",
                            phase.id, convergence.version
                        ),
                    ));
                }
                if !(1..=90).contains(&convergence.timeout_secs) {
                    return Err(ScenarioError::new(
                        "convergence_timeout_invalid",
                        format!(
                            "phase {} convergence timeout must be 1..=90 seconds",
                            phase.id
                        ),
                    ));
                }
                if !(1..=90).contains(&convergence.max_observations) {
                    return Err(ScenarioError::new(
                        "convergence_observations_invalid",
                        format!("phase {} convergence observations must be 1..=90", phase.id),
                    ));
                }
                if convergence.max_resume_attempts > 1 {
                    return Err(ScenarioError::new(
                        "convergence_resume_bound_invalid",
                        format!("phase {} convergence permits at most one resume", phase.id),
                    ));
                }
                if !convergence.allow_resume && convergence.max_resume_attempts != 0 {
                    return Err(ScenarioError::new(
                        "convergence_resume_disabled_with_budget",
                        format!(
                            "phase {} disables resume but budgets resume attempts",
                            phase.id
                        ),
                    ));
                }
            }
        }
        Ok(())
    }

    fn validate_assertions(&self) -> Result<(), ScenarioError> {
        let mut seen = BTreeSet::new();
        for assertion in &self.assertions {
            if assertion.id.trim().is_empty() {
                return Err(ScenarioError::new(
                    "assertion_id_empty",
                    "an assertion has an empty id",
                ));
            }
            if !seen.insert(assertion.id.clone()) {
                return Err(ScenarioError::new(
                    "duplicate_assertion_id",
                    format!("duplicate assertion id {}", assertion.id),
                ));
            }
            if assertion.deadline_secs == 0 {
                return Err(ScenarioError::new(
                    "assertion_deadline_invalid",
                    format!("assertion {} has a zero deadline", assertion.id),
                ));
            }
            let predicate = assertion.predicate.trim();
            if predicate.is_empty() {
                return Err(ScenarioError::new(
                    "assertion_predicate_empty",
                    format!("assertion {} has an empty predicate", assertion.id),
                ));
            }
            if predicate == "is_canonical"
                || (predicate.contains("is_canonical")
                    && !predicate.contains("canonical_at")
                    && !predicate.contains("at="))
            {
                return Err(ScenarioError::new(
                    "assertion_time_scope_ambiguous",
                    format!(
                        "assertion {} says is_canonical without a named observation time or canonical_at",
                        assertion.id
                    ),
                ));
            }
        }
        Ok(())
    }

    fn validate_statistics(&self) -> Result<(), ScenarioError> {
        for statistic in &self.statistics {
            if statistic.metric.trim().is_empty() {
                return Err(ScenarioError::new(
                    "statistic_metric_empty",
                    "a statistic has an empty metric",
                ));
            }
            if statistic.estimator.trim().is_empty() {
                return Err(ScenarioError::new(
                    "statistic_estimator_empty",
                    format!("statistic {} has no estimator", statistic.metric),
                ));
            }
            if statistic.planned_n == 0 {
                return Err(ScenarioError::new(
                    "statistic_planned_n_invalid",
                    format!("statistic {} has planned_n=0", statistic.metric),
                ));
            }
            if statistic.stopping_rule.is_null() {
                return Err(ScenarioError::new(
                    "statistic_stopping_rule_missing",
                    format!("statistic {} has no stopping rule", statistic.metric),
                ));
            }
            if let Some(confidence) = statistic.confidence {
                if !(0.0..=1.0).contains(&confidence) {
                    return Err(ScenarioError::new(
                        "statistic_confidence_invalid",
                        format!("statistic {} confidence is outside [0,1]", statistic.metric),
                    ));
                }
            }
        }
        Ok(())
    }

    /// Validates the optional traffic section (traffic plan Section 11)
    /// against the frozen T0 schema, including the cross-field rule that
    /// `user_payment_wallets` equals the scenario identity count. On
    /// success the recorded traffic capability joins
    /// `report.required_capabilities` so preflight gates traffic exactly
    /// like `spc.node.regtest.pow_baseline` today.
    fn validate_traffic(&self, report: &mut ValidationReport) -> Result<(), ScenarioError> {
        if let Some(traffic) = self.traffic.as_ref() {
            traffic.validate(self.identities.len(), report)?;
        }
        Ok(())
    }

    fn validate_observers(&self, report: &mut ValidationReport) -> Result<(), ScenarioError> {
        let node_count = self.nodes.len();
        if node_count > 1 {
            let profiles: BTreeSet<_> =
                self.nodes.iter().map(|node| &node.implementation).collect();
            if profiles.len() > 1 {
                report.warnings.push(
                    "network nodes use different implementations; cross-node profile equality is checked at preflight"
                        .to_string(),
                );
            }
        }
        if self
            .ark_backends
            .iter()
            .any(|ark| ark.implementation.contains("stub"))
        {
            report.required_capabilities.insert("ark.stub".to_string());
        }
        Ok(())
    }

    /// Work profiles of every pool, keyed by pool.
    pub fn work_profiles(&self) -> BTreeMap<PoolId, &WorkProfileSpec> {
        self.pools
            .iter()
            .map(|pool| (pool.id.clone(), &pool.work_profile))
            .collect()
    }

    pub fn pool_for_front(&self, front_id: &FrontId) -> Option<&PoolSpec> {
        let front = self.fronts.iter().find(|f| &f.id == front_id)?;
        self.pools.iter().find(|p| p.id == front.pool_id)
    }
}

fn validate_work_profile(
    pool: &PoolSpec,
    network: &NetworkSpec,
    report: &mut ValidationReport,
) -> Result<(), ScenarioError> {
    let profile = &pool.work_profile;
    match profile.work_unit_version {
        1 => {
            if profile.window_rule != WindowRule::ShareWorkLegacy {
                return Err(ScenarioError::new(
                    "work_window_rule_mismatch",
                    format!(
                        "pool {} v1 work requires window_rule=share_work_legacy",
                        pool.id
                    ),
                ));
            }
            match profile.legacy_share_difficulty {
                Some(d) if d.get() > 0 => {}
                _ => {
                    return Err(ScenarioError::new(
                        "legacy_difficulty_invalid",
                        format!(
                            "pool {} v1 work needs a positive legacy_share_difficulty",
                            pool.id
                        ),
                    ));
                }
            }
            return Ok(());
        }
        2 => {}
        other => {
            return Err(ScenarioError::new(
                "work_unit_version_unknown",
                format!("pool {} has unknown work_unit_version {other}", pool.id),
            ));
        }
    }

    if profile.window_rule != WindowRule::NetworkBlockWork {
        return Err(ScenarioError::new(
            "work_window_rule_mismatch",
            format!(
                "pool {} v2 work requires window_rule=network_block_work",
                pool.id
            ),
        ));
    }
    if profile.window_n == 0 {
        return Err(ScenarioError::new(
            "window_n_invalid",
            format!("pool {} window_n must be > 0", pool.id),
        ));
    }
    let reference_hex = profile.reference_target.as_deref().ok_or_else(|| {
        ScenarioError::new(
            "reference_target_missing",
            format!("pool {} has no reference_target", pool.id),
        )
    })?;
    let reference = ReferenceProfile::parse_hex(reference_hex).map_err(|e| {
        ScenarioError::new(
            "reference_target_invalid",
            format!("pool {}: {}", pool.id, e.detail),
        )
    })?;
    let m_cfg = profile.share_work_multiplier.ok_or_else(|| {
        ScenarioError::new(
            "multiplier_missing",
            format!("pool {} has no share_work_multiplier", pool.id),
        )
    })?;
    let max_m = profile.max_work_multiplier.ok_or_else(|| {
        ScenarioError::new(
            "max_work_multiplier_missing",
            format!("pool {} has no max_work_multiplier", pool.id),
        )
    })?;
    if max_m > (1u64 << 32) {
        return Err(ScenarioError::new(
            "max_work_multiplier_too_large",
            format!("pool {} max_work_multiplier exceeds 2^32", pool.id),
        ));
    }
    if !m_cfg.is_power_of_two() {
        return Err(ScenarioError::new(
            "multiplier_not_in_family",
            format!(
                "pool {} share_work_multiplier {m_cfg} is not a power of two",
                pool.id
            ),
        ));
    }
    let j_cfg = m_cfg.trailing_zeros();
    if j_cfg > reference.k() - 1 {
        return Err(ScenarioError::new(
            "multiplier_not_in_family",
            format!(
                "pool {} multiplier exponent {j_cfg} exceeds k-1={}",
                pool.id,
                reference.k() - 1
            ),
        ));
    }
    if m_cfg > max_m {
        return Err(ScenarioError::new(
            "multiplier_exceeds_max",
            format!(
                "pool {} share_work_multiplier {m_cfg} exceeds max_work_multiplier {max_m}",
                pool.id
            ),
        ));
    }
    let max_window = profile
        .max_window_target_units
        .unwrap_or(WideU128(1u128 << 64));
    if max_window.get() > (1u128 << 64) {
        return Err(ScenarioError::new(
            "max_window_target_units_too_large",
            format!("pool {} max_window_target_units exceeds 2^64", pool.id),
        ));
    }
    if let Some(slice_shares) = profile.max_slice_shares {
        if slice_shares > 65_000 {
            return Err(ScenarioError::new(
                "max_slice_shares_too_large",
                format!("pool {} max_slice_shares exceeds 65 000", pool.id),
            ));
        }
    }

    if let Some(bits) = network
        .consensus_profile
        .requirements
        .pow_limit_bits
        .as_deref()
        .and_then(|bits| decode_bits(bits).ok())
    {
        let target = Target256::from_compact(bits).map_err(|e| {
            ScenarioError::new(
                "pow_limit_bits_invalid",
                format!("pool {} network pow limit: {e}", pool.id),
            )
        })?;
        if reference.reference() < target {
            return Err(ScenarioError::new(
                "reference_below_pow_limit",
                format!(
                    "pool {} reference 0x{} is harder than pow limit 0x{}",
                    pool.id,
                    reference.reference().to_hex(),
                    target.to_hex()
                ),
            ));
        }
        let network_target = target.to_biguint();
        let j_issue = reference
            .issuance_exponent(j_cfg, &network_target)
            .map_err(|e| {
                ScenarioError::new("issuance_unplannable", format!("pool {}: {e}", pool.id))
            })?;
        debug_assert!(j_issue <= j_cfg);
        let q = reference
            .window_target(profile.window_n, &network_target)
            .map_err(|e| {
                ScenarioError::new("window_target_invalid", format!("pool {}: {e}", pool.id))
            })?;
        let q_u128 = u128::try_from(q).map_err(|_| {
            ScenarioError::new(
                "window_target_too_large",
                format!("pool {} window target does not fit u128", pool.id),
            )
        })?;
        if q_u128 > max_window.get() {
            return Err(ScenarioError::new(
                "window_target_too_large",
                format!(
                    "pool {} window target {} exceeds max_window_target_units {}",
                    pool.id,
                    q_u128,
                    max_window.get()
                ),
            ));
        }
    } else {
        report.warnings.push(format!(
            "pool {} declares no network pow_limit_bits; preflight must compare the effective profile",
            pool.id
        ));
    }
    Ok(())
}

fn decode_bits(value: &str) -> Result<u32, String> {
    let value = value
        .trim()
        .trim_start_matches("0x")
        .trim_start_matches("0X");
    if value.is_empty() || value.len() > 8 {
        return Err("compact bits must be 1..=8 hex digits".to_string());
    }
    u32::from_str_radix(value, 16).map_err(|_| "compact bits are not hexadecimal".to_string())
}

fn validate_fingerprint(value: &str) -> Result<(), ScenarioError> {
    let value = value.trim();
    let hex = value.strip_prefix("sha256:").unwrap_or(value);
    if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ScenarioError::new(
            "profile_fingerprint_invalid",
            format!("profile fingerprint {value:?} is not sha256:hex"),
        ));
    }
    Ok(())
}

fn parse_listen(value: &str) -> Result<(String, u16), ScenarioError> {
    let (host, port) = value.rsplit_once(':').ok_or_else(|| {
        ScenarioError::new("listen_invalid", format!("listener {value:?} has no port"))
    })?;
    if !matches!(host, "127.0.0.1" | "::1" | "localhost") {
        return Err(ScenarioError::new(
            "listen_not_loopback",
            format!("listener {value:?} is not loopback"),
        ));
    }
    let port = port.parse::<u16>().map_err(|_| {
        ScenarioError::new(
            "listen_port_invalid",
            format!("listener {value:?} has a bad port"),
        )
    })?;
    Ok((host.to_string(), port))
}

fn valid_sql_identifier(value: &str) -> bool {
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    value.len() <= 63
        && (first.is_ascii_lowercase() || first == b'_')
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn record_listener(
    ports: &mut BTreeMap<(String, u16), String>,
    kind: &'static str,
    owner: &str,
    listener: &str,
) -> Result<(), ScenarioError> {
    let (host, port) = parse_listen(listener)?;
    if port == 0 {
        return Ok(());
    }
    let key = (host, port);
    if let Some(previous) = ports.insert(key, owner.to_string()) {
        return Err(ScenarioError::new(
            "listener_port_collision",
            format!("{kind} for {owner} collides with {previous} on port {port}"),
        ));
    }
    Ok(())
}

impl fmt::Display for LinkPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Normal => f.write_str("normal"),
            Self::Partition => f.write_str("partition"),
            Self::Latency { delay_ms } => write!(f, "latency({delay_ms}ms)"),
            Self::Loss { loss_percent } => write!(f, "loss({loss_percent}%)"),
        }
    }
}
