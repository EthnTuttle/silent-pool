//! Traffic stage admission and preflight binding, independent of the legacy lane.
use crate::command::CommandKind;
use crate::ids::NodeId;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrafficStage {
    Provisioned,
    Bootstrapping,
    Active,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasuredBoundary {
    pub boot_id: String,
    pub monotonic_ms: u64,
    pub wall_ms: u64,
    pub tip_hash: String,
    pub tip_height: u64,
}

pub fn admitted(stage: TrafficStage, command: &CommandKind) -> Result<(), &'static str> {
    use CommandKind::*;
    match command {
        StopRun { .. } | WaitHeight { .. } | RestartTxDriver { .. } => Ok(()),
        // This only admits the closed hook vocabulary to the engine handler.
        // Authentication, default-off scenario flag, exact driver incarnation
        // and hook transition checks still precede any hook effect there.
        TrafficAcceptance { .. } => Ok(()),
        // Wallet commands are not traffic effects: their own claim scope,
        // journal durability and registry binding gates apply in every stage,
        // exactly as in the wallet-only lineage this runtime was joined with.
        WalletRefresh { .. } | WalletOffboard { .. } | WalletReadInputs { .. } => Ok(()),
        BootstrapFund { .. } if stage == TrafficStage::Provisioned => Ok(()),
        BootstrapFund { .. } => Err("traffic_bootstrap_duplicate"),
        BootstrapControl { .. } if stage == TrafficStage::Bootstrapping => Ok(()),
        _ if stage == TrafficStage::Active => Ok(()),
        _ => Err("traffic_not_active"),
    }
}

#[derive(Debug, Clone)]
pub struct NodeBinding {
    pub binary_sha256: String,
    pub genesis: String,
    pub comparison_sha256: String,
    pub consensus: Value,
}

/// Match the producer's closed, required live consensus subset. All keys are
/// required and exact; a partially upgraded node cannot be silently accepted.
pub fn consensus_facts(info: &Value) -> Result<Value, &'static str> {
    let c = info
        .get("consensus")
        .and_then(Value::as_object)
        .ok_or("traffic_consensus_unavailable")?;
    let mut out = serde_json::Map::new();
    for key in [
        "network",
        "consensus_profile_id",
        "profile_fingerprint",
        "initial_target_bits",
        "regtest_pow_bits_overlay",
        "pow_limit_bits",
        "retarget_floor_bits",
        "subsidy_initial_sats",
    ] {
        let value = c
            .get(key)
            .filter(|v| v.is_string())
            .ok_or("traffic_consensus_unavailable")?;
        out.insert(key.into(), value.clone());
    }
    for key in [
        "pow_target_spacing",
        "pow_target_timespan",
        "difficulty_adjustment_interval",
        "subsidy_halving_interval",
        "coinbase_maturity",
    ] {
        let value = c
            .get(key)
            .filter(|v| v.as_u64().is_some())
            .ok_or("traffic_consensus_unavailable")?;
        out.insert(key.into(), value.clone());
    }
    for key in ["no_pow_retargeting", "allow_min_difficulty_blocks"] {
        let value = c
            .get(key)
            .filter(|v| v.is_boolean())
            .ok_or("traffic_consensus_unavailable")?;
        out.insert(key.into(), value.clone());
    }
    if out["network"].as_str() != Some("regtest") {
        return Err("traffic_launch_profile_mismatch");
    }
    Ok(Value::Object(out))
}

pub fn bind_report(report: &Value, actual: &BTreeMap<NodeId, NodeBinding>) -> Result<(), String> {
    if actual.is_empty() {
        return Err("traffic_launch_profile_mismatch:no_actual_nodes".into());
    }
    let node = &report["node"];
    for (id, binding) in actual {
        for (field, expected, observed) in [
            (
                "binary_sha256",
                Value::String(binding.binary_sha256.clone()),
                node["binary_sha256"].clone(),
            ),
            (
                "comparison_sha256",
                Value::String(binding.comparison_sha256.clone()),
                node["launch_profile"]["comparison_sha256"].clone(),
            ),
            (
                "effective_consensus",
                binding.consensus.clone(),
                node["effective_consensus"].clone(),
            ),
        ] {
            if expected != observed {
                return Err(format!("traffic_launch_profile_mismatch:{id}:{field}"));
            }
        }
        if node["launch_profile"]["parent_death_levels"].as_u64() != Some(2)
            || node["launch_profile"]["environment"].as_str() != Some("finalized-no-inheritance")
            || node["launch_profile"]["transport"].as_str() != Some("validated_lab_transport")
        {
            return Err(format!(
                "traffic_launch_profile_mismatch:{id}:lifetime_or_environment"
            ));
        }
    }
    Ok(())
}
