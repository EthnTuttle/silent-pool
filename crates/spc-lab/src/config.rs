//! Runtime adapter and capability configuration.
//!
//! A scenario names an implementation; this file supplies the executable and
//! arguments. A name is never evidence that a capability exists: preflight
//! compares the scenario's requirements with declared and effective values.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use num_bigint::BigUint;
use serde::{Deserialize, Serialize};

use crate::ids::NodeId;
use crate::scenario::{Scenario, ScenarioError, WindowRule};
use crate::wide::{ReferenceProfile, Target256};

/// One external process adapter.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterConfig {
    pub program: PathBuf,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// A fixture is explicitly not a real node/pool/miner. Its evidence is
    /// tagged and cannot support real-mining or interoperability claims.
    #[serde(default)]
    pub fixture: bool,
    #[serde(default)]
    pub effective_profile: Option<NodeEffectiveProfile>,
    #[serde(default)]
    pub rpc_auth: Option<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LabConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traffic_node_source_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traffic_probe: Option<AdapterConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tx_driver: Option<AdapterConfig>,
    #[serde(default)]
    pub nodes: BTreeMap<String, AdapterConfig>,
    #[serde(default)]
    pub pools: BTreeMap<String, AdapterConfig>,
    #[serde(default)]
    pub miners: BTreeMap<String, AdapterConfig>,
    #[serde(default)]
    pub ark: BTreeMap<String, AdapterConfig>,
    #[serde(default)]
    pub redis: Option<AdapterConfig>,
    #[serde(default)]
    pub postgres: Option<AdapterConfig>,
    /// Per-node profile override for a node whose implementation is shared.
    #[serde(default)]
    pub node_profiles: BTreeMap<NodeId, NodeEffectiveProfile>,
    /// `fixture` or `unsupported`. LP-4 owns the network-namespace backend.
    #[serde(default)]
    pub link_backend: Option<String>,
}

/// The effective profile a node exposes. Fields a node does not support are
/// absent rather than guessed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeEffectiveProfile {
    #[serde(default)]
    pub implementation: Option<String>,
    #[serde(default)]
    pub network: Option<String>,
    #[serde(default)]
    pub consensus_profile_id: Option<String>,
    #[serde(default)]
    pub profile_fingerprint: Option<String>,
    #[serde(default)]
    pub pow_limit_bits: Option<String>,
    #[serde(default)]
    pub retarget_floor_bits: Option<String>,
    #[serde(default)]
    pub initial_target_bits: Option<String>,
    #[serde(default)]
    pub regtest_pow_bits_overlay: Option<String>,
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
    pub subsidy_initial_sats: Option<String>,
    #[serde(default)]
    pub subsidy_halving_interval_blocks: Option<u64>,
    #[serde(default)]
    pub coinbase_maturity_blocks: Option<u64>,
    #[serde(default)]
    pub witness_commitment_supported: Option<bool>,
    #[serde(default)]
    pub tdp_session_cap: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreflightReport {
    pub warnings: Vec<String>,
    pub capabilities: BTreeSet<String>,
    pub effective_profiles: BTreeMap<NodeId, NodeEffectiveProfile>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {detail}")]
pub struct PreflightError {
    code: &'static str,
    detail: String,
}

impl PreflightError {
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

impl LabConfig {
    pub fn from_json(json: &str) -> Result<Self, PreflightError> {
        serde_json::from_str(json)
            .map_err(|e| PreflightError::new("config_schema_invalid", e.to_string()))
    }

    pub fn node(&self, implementation: &str) -> Option<&AdapterConfig> {
        self.nodes.get(implementation)
    }

    pub fn pool(&self, implementation: &str) -> Option<&AdapterConfig> {
        self.pools.get(implementation)
    }

    pub fn miner(&self, implementation: &str) -> Option<&AdapterConfig> {
        self.miners.get(implementation)
    }

    pub fn adapter_for_node(&self, _node: &NodeId, implementation: &str) -> Option<&AdapterConfig> {
        self.nodes.get(implementation)
    }
}

/// Validate a scenario against the configured adapters. This runs before any
/// process starts. A missing executable, capability or effective field is a
/// failure, never a silent fixture fallback.
pub fn preflight(
    scenario: &Scenario,
    config: &LabConfig,
) -> Result<PreflightReport, PreflightError> {
    let validation = scenario
        .validate()
        .map_err(|e: ScenarioError| PreflightError::new(e.code(), e.detail()))?;
    let mut report = PreflightReport {
        warnings: validation.warnings.clone(),
        capabilities: validation.required_capabilities.clone(),
        effective_profiles: BTreeMap::new(),
    };

    // Required capability set includes the network requirement and the pool
    // work version. A fixture declares its own weaker capability.
    if scenario
        .pools
        .iter()
        .any(|pool| pool.work_profile.work_unit_version == 2)
    {
        report
            .capabilities
            .insert("spc.pool.work_unit_v2".to_string());
    }
    for pool in &scenario.pools {
        if pool.work_profile.window_rule == WindowRule::NetworkBlockWork {
            report
                .capabilities
                .insert("spc.pool.network_block_work".to_string());
        }
    }

    for node in &scenario.nodes {
        let adapter = config.node(&node.implementation).ok_or_else(|| {
            PreflightError::new(
                "adapter_missing",
                format!(
                    "node {} implementation {:?} has no configured node adapter",
                    node.id, node.implementation
                ),
            )
        })?;
        check_program(&adapter.program)?;
        let declared: BTreeSet<_> = adapter.capabilities.iter().cloned().collect();
        if !declared.contains(&scenario.network.consensus_profile.required_capability) {
            return Err(PreflightError::new(
                "capability_unavailable",
                format!(
                    "node {} implementation {:?} does not declare required capability {:?} (declared: {:?})",
                    node.id, node.implementation,
                    scenario.network.consensus_profile.required_capability,
                    declared
                ),
            ));
        }
        for capability in &report.capabilities {
            if capability.starts_with("spc.pool.") || capability == "ark.stub" {
                continue;
            }
            if !declared.contains(capability) {
                return Err(PreflightError::new(
                    "capability_unavailable",
                    format!(
                        "node {} implementation {:?} does not declare required capability {:?}",
                        node.id, node.implementation, capability
                    ),
                ));
            }
        }
        let profile = config
            .node_profiles
            .get(&node.id)
            .cloned()
            .or_else(|| adapter.effective_profile.clone());
        if let Some(profile) = profile {
            report.effective_profiles.insert(node.id.clone(), profile);
        } else {
            report.warnings.push(format!(
                "node {} has no preflight effective profile; the adapter must expose one at readiness",
                node.id
            ));
        }
        if adapter.fixture {
            report.warnings.push(format!(
                "node {} uses fixture adapter {:?}; real mining/consensus claims are not available",
                node.id, node.implementation
            ));
        }
    }

    for pool in &scenario.pools {
        let adapter = config.pool(&pool.implementation).ok_or_else(|| {
            PreflightError::new(
                "adapter_missing",
                format!(
                    "pool {} implementation {:?} has no configured pool adapter",
                    pool.id, pool.implementation
                ),
            )
        })?;
        check_program(&adapter.program)?;
        let declared: BTreeSet<_> = adapter.capabilities.iter().cloned().collect();
        for capability in report
            .capabilities
            .iter()
            .filter(|c| c.starts_with("spc.pool."))
        {
            if !declared.contains(capability) {
                return Err(PreflightError::new(
                    "capability_unavailable",
                    format!(
                        "pool {} implementation {:?} does not declare required capability {:?}",
                        pool.id, pool.implementation, capability
                    ),
                ));
            }
        }
        if adapter.fixture {
            report.warnings.push(format!(
                "pool {} uses fixture adapter {:?}; accounting/payout claims are not available",
                pool.id, pool.implementation
            ));
        }
    }

    for miner in &scenario.miners {
        let adapter = config.miner(&miner.implementation).ok_or_else(|| {
            PreflightError::new(
                "adapter_missing",
                format!(
                    "miner {} implementation {:?} has no configured miner adapter",
                    miner.id, miner.implementation
                ),
            )
        })?;
        check_program(&adapter.program)?;
        if adapter.fixture {
            report.warnings.push(format!(
                "miner {} uses fixture adapter {:?}; hashes are not real PoW",
                miner.id, miner.implementation
            ));
        }
    }

    for ark in &scenario.ark_backends {
        let adapter = config.ark.get(&ark.implementation).ok_or_else(|| {
            PreflightError::new(
                "adapter_missing",
                format!(
                    "Ark backend {} implementation {:?} has no configured adapter",
                    ark.id, ark.implementation
                ),
            )
        })?;
        check_program(&adapter.program)?;
    }

    if !scenario.resources.redis_instances.is_empty() {
        let adapter = config.redis.as_ref().ok_or_else(|| {
            PreflightError::new(
                "adapter_missing",
                "scenario declares Redis instances but no Redis adapter is configured",
            )
        })?;
        check_program(&adapter.program)?;
    }
    if !scenario.resources.postgres_databases.is_empty() {
        let adapter = config.postgres.as_ref().ok_or_else(|| {
            PreflightError::new(
                "adapter_missing",
                "scenario declares Postgres databases but no Postgres adapter is configured",
            )
        })?;
        check_program(&adapter.program)?;
    }

    let link_backend = crate::links::backend_for(config.link_backend.as_deref());
    link_backend
        .preflight(scenario)
        .map_err(|e| PreflightError::new("link_backend_unavailable", e.to_string()))?;
    report.capabilities.extend(link_backend.capabilities());

    check_cross_node_profiles(scenario, &report)?;
    check_profile_requirements(scenario, &mut report)?;
    check_work_profiles(scenario, &mut report)?;
    check_tdp_capacity(scenario, &report)?;

    // Every configured binary must be executable, not merely present.
    for adapter in config
        .nodes
        .values()
        .chain(config.pools.values())
        .chain(config.miners.values())
        .chain(config.ark.values())
        .chain(config.redis.iter())
        .chain(config.postgres.iter())
    {
        check_program(&adapter.program)?;
    }
    Ok(report)
}

fn check_program(path: &Path) -> Result<(), PreflightError> {
    let metadata = std::fs::metadata(path).map_err(|e| {
        PreflightError::new(
            "adapter_binary_missing",
            format!("adapter binary {} is not readable: {e}", path.display()),
        )
    })?;
    if !metadata.is_file() {
        return Err(PreflightError::new(
            "adapter_binary_not_file",
            format!("adapter binary {} is not a regular file", path.display()),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(PreflightError::new(
                "adapter_binary_not_executable",
                format!("adapter binary {} is not executable", path.display()),
            ));
        }
    }
    Ok(())
}

fn check_cross_node_profiles(
    scenario: &Scenario,
    report: &PreflightReport,
) -> Result<(), PreflightError> {
    let profiles: Vec<_> = report.effective_profiles.values().collect();
    if profiles.len() < 2 {
        return Ok(());
    }
    for node in &scenario.nodes {
        let Some(left) = report.effective_profiles.get(&node.id) else {
            continue;
        };
        for other in &scenario.nodes {
            if node.id == other.id {
                continue;
            }
            let Some(right) = report.effective_profiles.get(&other.id) else {
                continue;
            };
            if !same_consensus(left, right) {
                return Err(PreflightError::new(
                    "cross_node_profile_mismatch",
                    format!(
                        "nodes {} and {} do not expose the same effective consensus profile",
                        node.id, other.id
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn same_consensus(left: &NodeEffectiveProfile, right: &NodeEffectiveProfile) -> bool {
    left.pow_limit_bits == right.pow_limit_bits
        && left.target_spacing_secs == right.target_spacing_secs
        && left.target_timespan_secs == right.target_timespan_secs
        && left.retarget_interval_blocks == right.retarget_interval_blocks
        && left.no_pow_retargeting == right.no_pow_retargeting
        && left.allow_min_difficulty_blocks == right.allow_min_difficulty_blocks
        && left.minimum_difficulty_time_gap_secs == right.minimum_difficulty_time_gap_secs
        && left.retarget_floor_bits == right.retarget_floor_bits
        && left.subsidy_initial_sats == right.subsidy_initial_sats
        && left.subsidy_halving_interval_blocks == right.subsidy_halving_interval_blocks
        && left.coinbase_maturity_blocks == right.coinbase_maturity_blocks
        && left.witness_commitment_supported == right.witness_commitment_supported
        && left.tdp_session_cap == right.tdp_session_cap
}

fn check_profile_requirements(
    scenario: &Scenario,
    report: &mut PreflightReport,
) -> Result<(), PreflightError> {
    let requirement = &scenario.network.consensus_profile;
    let req = &requirement.requirements;
    for node in &scenario.nodes {
        let Some(profile) = report.effective_profiles.get(&node.id) else {
            continue;
        };
        compare_option(
            "pow_limit_bits",
            req.pow_limit_bits.as_deref(),
            profile.pow_limit_bits.as_deref(),
            &node.id,
        )?;
        compare_option(
            "retarget_floor_bits",
            req.retarget_floor_bits.as_deref(),
            profile.retarget_floor_bits.as_deref(),
            &node.id,
        )?;
        compare_option(
            "initial_target_bits",
            req.initial_target_bits.as_deref(),
            profile.initial_target_bits.as_deref(),
            &node.id,
        )?;
        compare_option(
            "target_spacing_secs",
            req.target_spacing_secs,
            profile.target_spacing_secs,
            &node.id,
        )?;
        compare_option(
            "target_timespan_secs",
            req.target_timespan_secs,
            profile.target_timespan_secs,
            &node.id,
        )?;
        compare_option(
            "retarget_interval_blocks",
            req.retarget_interval_blocks,
            profile.retarget_interval_blocks,
            &node.id,
        )?;
        compare_option(
            "no_pow_retargeting",
            req.no_pow_retargeting,
            profile.no_pow_retargeting,
            &node.id,
        )?;
        compare_option(
            "allow_min_difficulty_blocks",
            req.allow_min_difficulty_blocks,
            profile.allow_min_difficulty_blocks,
            &node.id,
        )?;
        if let Some(required) = req.minimum_difficulty_time_gap_secs {
            match profile.minimum_difficulty_time_gap_secs {
                Some(actual) if actual == required => {}
                Some(actual) => {
                    return Err(PreflightError::new(
                        "profile_requirement_mismatch",
                        format!(
                            "node {} minimum_difficulty_time_gap_secs={actual}, scenario requires {required}",
                            node.id
                        ),
                    ));
                }
                None => {
                    return Err(PreflightError::new(
                        "profile_requirement_missing",
                        format!(
                            "node {} does not expose minimum_difficulty_time_gap_secs",
                            node.id
                        ),
                    ));
                }
            }
        }
        compare_option(
            "subsidy_halving_interval_blocks",
            req.subsidy_halving_interval_blocks,
            profile.subsidy_halving_interval_blocks,
            &node.id,
        )?;
        compare_option(
            "coinbase_maturity_blocks",
            req.coinbase_maturity_blocks,
            profile.coinbase_maturity_blocks,
            &node.id,
        )?;
        compare_option(
            "witness_commitment_supported",
            req.witness_commitment_supported,
            profile.witness_commitment_supported,
            &node.id,
        )?;
        compare_option(
            "tdp_session_cap",
            req.tdp_session_cap,
            profile.tdp_session_cap,
            &node.id,
        )?;
        if let Some(required) = req.subsidy_initial_sats.as_ref() {
            match profile.subsidy_initial_sats.as_deref() {
                Some(actual) if required.to_string() == actual => {}
                Some(actual) => {
                    return Err(PreflightError::new(
                        "subsidy_profile_mismatch",
                        format!(
                            "node {} subsidy_initial_sats is {actual}, scenario requires {}",
                            node.id, required
                        ),
                    ));
                }
                None => {
                    return Err(PreflightError::new(
                        "profile_requirement_missing",
                        format!(
                            "node {} does not expose required subsidy_initial_sats",
                            node.id
                        ),
                    ));
                }
            }
        }
    }

    if let Some(fingerprint) = &requirement.profile_fingerprint {
        let hex = fingerprint.trim_start_matches("sha256:");
        for node in &scenario.nodes {
            let Some(profile) = report.effective_profiles.get(&node.id) else {
                continue;
            };
            match profile.profile_fingerprint.as_deref() {
                Some(actual) => {
                    let actual = actual.trim_start_matches("sha256:");
                    if actual != hex {
                        return Err(PreflightError::new(
                            "profile_fingerprint_mismatch",
                            format!(
                                "node {} fingerprint {actual} does not match scenario {hex}",
                                node.id
                            ),
                        ));
                    }
                }
                None => report.warnings.push(format!(
                    "node {} does not report a profile fingerprint; effective fields remain authoritative",
                    node.id
                )),
            }
        }
    }
    if requirement.required_capability.contains("retarget") {
        for node in &scenario.nodes {
            if let Some(profile) = report.effective_profiles.get(&node.id) {
                if profile.no_pow_retargeting == Some(true) {
                    return Err(PreflightError::new(
                        "retarget_profile_unavailable",
                        format!("node {} reports no_pow_retargeting=true", node.id),
                    ));
                }
            }
        }
    }
    if requirement.required_capability.contains("baseline") {
        for node in &scenario.nodes {
            if let Some(profile) = report.effective_profiles.get(&node.id) {
                if profile.no_pow_retargeting == Some(false) {
                    return Err(PreflightError::new(
                        "baseline_profile_mismatch",
                        format!(
                            "node {} reports retargeting on while the scenario requests the baseline",
                            node.id
                        ),
                    ));
                }
            }
        }
    }
    Ok(())
}

fn compare_option<T: PartialEq + std::fmt::Debug>(
    name: &str,
    required: Option<T>,
    actual: Option<T>,
    node: &NodeId,
) -> Result<(), PreflightError> {
    if let Some(required) = required {
        match actual {
            Some(actual) if actual == required => Ok(()),
            Some(actual) => Err(PreflightError::new(
                "profile_requirement_mismatch",
                format!("node {node} {name}={actual:?}, scenario requires {required:?}"),
            )),
            None => Err(PreflightError::new(
                "profile_requirement_missing",
                format!("node {node} does not expose {name} (scenario requires {required:?})"),
            )),
        }
    } else {
        Ok(())
    }
}

fn check_work_profiles(
    scenario: &Scenario,
    report: &mut PreflightReport,
) -> Result<(), PreflightError> {
    for pool in &scenario.pools {
        if pool.work_profile.work_unit_version != 2 {
            continue;
        }
        let reference_hex = pool
            .work_profile
            .reference_target
            .as_deref()
            .ok_or_else(|| {
                PreflightError::new("reference_target_missing", format!("pool {}", pool.id))
            })?;
        let reference = ReferenceProfile::parse_hex(reference_hex)
            .map_err(|e| PreflightError::new("reference_target_invalid", e.detail))?;
        let m_cfg = pool.work_profile.share_work_multiplier.unwrap_or(0);
        let j_cfg = m_cfg.trailing_zeros();
        let max_window = pool
            .work_profile
            .max_window_target_units
            .unwrap_or(crate::wide::WideU128(1u128 << 64));
        let mut seen_bits: Option<u32> = None;
        for node_id in &pool.node_ids {
            let Some(profile) = report.effective_profiles.get(node_id) else {
                continue;
            };
            let Some(bits) = profile.pow_limit_bits.as_deref() else {
                continue;
            };
            let bits = u32::from_str_radix(bits.trim_start_matches("0x"), 16).map_err(|_| {
                PreflightError::new(
                    "pow_limit_bits_invalid",
                    format!("node {node_id} reports malformed pow_limit_bits {bits:?}"),
                )
            })?;
            if let Some(previous) = seen_bits {
                if previous != bits {
                    return Err(PreflightError::new(
                        "cross_node_profile_mismatch",
                        format!("pool {} nodes disagree on pow_limit_bits", pool.id),
                    ));
                }
            }
            seen_bits = Some(bits);
            let target = Target256::from_compact(bits).map_err(|e| {
                PreflightError::new("pow_limit_bits_invalid", format!("node {node_id}: {e}"))
            })?;
            if reference.reference() < target {
                return Err(PreflightError::new(
                    "reference_below_pow_limit",
                    format!(
                        "pool {} reference 0x{} is harder than node {} pow limit 0x{}",
                        pool.id,
                        reference.reference().to_hex(),
                        node_id,
                        target.to_hex()
                    ),
                ));
            }
            let target = target.to_biguint();
            reference.issuance_exponent(j_cfg, &target).map_err(|e| {
                PreflightError::new("issuance_unplannable", format!("pool {}: {e}", pool.id))
            })?;
            let q = reference
                .window_target(pool.work_profile.window_n, &target)
                .map_err(|e| {
                    PreflightError::new("window_target_invalid", format!("pool {}: {e}", pool.id))
                })?;
            let q = u128::try_from(q).map_err(|_| {
                PreflightError::new(
                    "window_target_too_large",
                    format!("pool {} window target exceeds u128", pool.id),
                )
            })?;
            if q > max_window.get() {
                return Err(PreflightError::new(
                    "window_target_too_large",
                    format!(
                        "pool {} window target {q} exceeds max_window_target_units {}",
                        pool.id,
                        max_window.get()
                    ),
                ));
            }
        }
        if seen_bits.is_none() {
            report.warnings.push(format!(
                "pool {} has no effective pow_limit_bits at preflight; readiness must supply it",
                pool.id
            ));
        }
    }
    Ok(())
}

fn check_tdp_capacity(scenario: &Scenario, report: &PreflightReport) -> Result<(), PreflightError> {
    for node in &scenario.nodes {
        let cap = match report
            .effective_profiles
            .get(&node.id)
            .and_then(|p| p.tdp_session_cap)
        {
            Some(cap) => cap,
            None => {
                if scenario
                    .network
                    .consensus_profile
                    .requirements
                    .tdp_session_cap
                    .is_some()
                {
                    return Err(PreflightError::new(
                        "profile_requirement_missing",
                        format!("node {} does not expose required tdp_session_cap", node.id),
                    ));
                }
                continue;
            }
        };
        let fronts = scenario
            .pools
            .iter()
            .filter(|pool| pool.node_ids.contains(&node.id))
            .flat_map(|pool| {
                scenario
                    .fronts
                    .iter()
                    .filter(move |front| front.pool_id == pool.id)
            })
            .count();
        if fronts as u64 > cap {
            return Err(PreflightError::new(
                "tdp_session_capacity",
                format!(
                    "node {} has {fronts} fronts but a TDP session cap of {cap}",
                    node.id
                ),
            ));
        }
    }
    Ok(())
}

/// Re-check effective profiles obtained at readiness. This is the same field
/// comparison used by preflight, never a fingerprint-only trust decision.
pub fn validate_effective_profiles(
    scenario: &Scenario,
    profiles: &BTreeMap<NodeId, NodeEffectiveProfile>,
) -> Result<Vec<String>, PreflightError> {
    let mut report = PreflightReport {
        effective_profiles: profiles.clone(),
        ..PreflightReport::default()
    };
    check_cross_node_profiles(scenario, &report)?;
    check_profile_requirements(scenario, &mut report)?;
    check_work_profiles(scenario, &mut report)?;
    check_tdp_capacity(scenario, &report)?;
    Ok(report.warnings)
}

/// Expand `{run_dir}`, `{entity}`, `{port}` and friends in an adapter argument.
pub fn expand_template(
    template: &str,
    context: &BTreeMap<&str, String>,
) -> Result<String, PreflightError> {
    let mut output = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        output.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else {
            output.push_str(&rest[start..]);
            return Ok(output);
        };
        let key = &after[..end];
        if key.is_empty() {
            return Err(PreflightError::new(
                "template_placeholder_invalid",
                format!("empty placeholder in {template:?}"),
            ));
        }
        let Some(value) = context.get(key) else {
            return Err(PreflightError::new(
                "template_placeholder_unknown",
                format!("unknown placeholder {{{key}}} in {template:?}"),
            ));
        };
        output.push_str(value);
        rest = &after[end + 1..];
    }
    output.push_str(rest);
    Ok(output)
}

/// Ensure each pool's storage is an independent endpoint.
pub fn isolated_storage(scenario: &Scenario) -> Result<(), PreflightError> {
    let mut redis = BTreeSet::new();
    let mut postgres = BTreeSet::new();
    for pool in &scenario.pools {
        let r = scenario
            .resources
            .redis_instances
            .iter()
            .find(|r| r.id == pool.storage.redis_instance_id)
            .ok_or_else(|| {
                PreflightError::new(
                    "pool_redis_unknown",
                    format!("pool {} storage is not declared", pool.id),
                )
            })?;
        let p = scenario
            .resources
            .postgres_databases
            .iter()
            .find(|p| p.id == pool.storage.postgres_database_id)
            .ok_or_else(|| {
                PreflightError::new(
                    "pool_postgres_unknown",
                    format!("pool {} storage is not declared", pool.id),
                )
            })?;
        if r.owner_pool_id != pool.id || p.owner_pool_id != pool.id {
            return Err(PreflightError::new(
                "pool_storage_owner_mismatch",
                format!("pool {} storage is owned by another pool", pool.id),
            ));
        }
        if !redis.insert(r.id.clone()) || !postgres.insert(p.id.clone()) {
            return Err(PreflightError::new(
                "pool_storage_shared",
                format!("pool {} shares a storage resource", pool.id),
            ));
        }
    }
    Ok(())
}

/// Convert a numeric profile field into a target for a node, when present.
pub fn profile_pow_limit(profile: &NodeEffectiveProfile) -> Option<Target256> {
    let bits = profile.pow_limit_bits.as_deref()?;
    let bits = u32::from_str_radix(bits.trim_start_matches("0x"), 16).ok()?;
    Target256::from_compact(bits).ok()
}

pub fn profile_target_big(profile: &NodeEffectiveProfile) -> Option<BigUint> {
    profile_pow_limit(profile).map(Target256::to_biguint)
}
