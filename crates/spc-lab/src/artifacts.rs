//! Artifact layout, shareable/private separation and chart projections.
//!
//! Raw evidence is append-only. Charts are a derived projection and never
//! replace the raw events an assertion reads. Identity-bearing evidence and
//! process arguments stay in the private store; the shareable manifest is
//! scanned for secret-bearing keys before it is written.

use std::collections::BTreeMap;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::evidence::{EvidenceEvent, EvidenceSummary, Gap};
use crate::ids::{CommandId, NodeId, RunId};
use crate::manifest::BinaryIdentity;
use crate::secrets::PublicAuthority;
use crate::wide::WideU128;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactPaths {
    pub root: PathBuf,
    pub run_manifest: PathBuf,
    pub shareable_manifest: PathBuf,
    pub shareable_charts: PathBuf,
    pub private_evidence: PathBuf,
    pub private_commands: PathBuf,
    pub private_scenario: PathBuf,
    pub private_runtime: PathBuf,
    pub logs: PathBuf,
    pub report: PathBuf,
}

impl ArtifactPaths {
    pub fn under(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            run_manifest: root.join("run-manifest.json"),
            shareable_manifest: root.join("shareable").join("manifest.json"),
            shareable_charts: root.join("shareable").join("charts.json"),
            private_evidence: root.join("private").join("evidence.jsonl"),
            private_commands: root.join("private").join("commands.jsonl"),
            private_scenario: root.join("private").join("scenario.json"),
            private_runtime: root.join("private").join("runtime.json"),
            logs: root.join("logs"),
            report: root.join("shareable").join("result.json"),
            root,
        }
    }

    pub fn create(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.root)?;
        let shareable = self.shareable_manifest.parent().unwrap_or(&self.root);
        std::fs::create_dir_all(shareable)?;
        let private = self.private_evidence.parent().unwrap_or(&self.root);
        std::fs::create_dir_all(private)?;
        std::fs::create_dir_all(&self.logs)?;
        std::fs::set_permissions(private, std::fs::Permissions::from_mode(0o700))?;
        std::fs::set_permissions(&self.logs, std::fs::Permissions::from_mode(0o700))?;
        std::fs::set_permissions(shareable, std::fs::Permissions::from_mode(0o755))?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceSummary {
    pub kind: String,
    pub id: String,
    pub owner: String,
    pub port: u16,
    #[serde(default)]
    pub database: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkProfileSummary {
    pub pool_id: String,
    pub work_unit_version: u16,
    pub window_rule: String,
    pub reference_fingerprint: Option<String>,
    pub share_work_multiplier: Option<u64>,
    pub window_n: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandSummary {
    pub command_id: CommandId,
    pub kind: String,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioSummary {
    pub scenario_version: u32,
    pub node_count: u64,
    pub pool_count: u64,
    pub front_count: u64,
    pub miner_count: u64,
    pub identity_count: u64,
    pub worker_count: u64,
    pub link_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareableManifest {
    pub schema_version: u32,
    pub run_id: RunId,
    pub base_revision: String,
    pub dirty_tree_hash: String,
    pub scenario_sha256: String,
    pub generated_at: u64,
    pub state: String,
    pub scenario: ScenarioSummary,
    pub binary_hashes: BTreeMap<String, BinaryIdentity>,
    pub resources: Vec<ResourceSummary>,
    pub effective_profiles: BTreeMap<NodeId, serde_json::Value>,
    pub pool_authorities: Vec<PublicAuthority>,
    pub work_profiles: Vec<WorkProfileSummary>,
    pub commands: Vec<CommandSummary>,
    pub evidence: EvidenceSummary,
    pub gaps: Vec<Gap>,
    pub assertions: Vec<AssertionSummary>,
    pub warnings: Vec<String>,
    pub non_claims: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssertionSummary {
    pub id: String,
    pub state: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChartPoint {
    pub bucket_start_ms: u64,
    pub event_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChartSeries {
    pub algorithm: String,
    pub bucket_ms: u64,
    pub source_first_event: u64,
    pub source_last_event: u64,
    pub source_event_count: u64,
    pub point_count: u64,
    pub dropped_points: u64,
    pub points: Vec<ChartPoint>,
}

/// Derive a chart timeline while leaving the raw evidence untouched.
pub fn decimate_timeline(events: &[EvidenceEvent], bucket_ms: u64) -> ChartSeries {
    let bucket_ms = bucket_ms.max(1);
    let first = events.first().map(|event| event.event_id).unwrap_or(0);
    let last = events.last().map(|event| event.event_id).unwrap_or(0);
    let mut buckets: BTreeMap<u64, u64> = BTreeMap::new();
    for event in events {
        let bucket = event.occurred_at_wall / bucket_ms * bucket_ms;
        *buckets.entry(bucket).or_insert(0) += 1;
    }
    let points: Vec<_> = buckets
        .into_iter()
        .map(|(bucket_start_ms, event_count)| ChartPoint {
            bucket_start_ms,
            event_count,
        })
        .collect();
    ChartSeries {
        algorithm: "sum_event_count_per_fixed_bucket".to_string(),
        bucket_ms,
        source_first_event: first,
        source_last_event: last,
        source_event_count: events.len() as u64,
        point_count: points.len() as u64,
        dropped_points: (events.len() as u64).saturating_sub(points.len() as u64),
        points,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ArtifactError {
    #[error("artifact io: {0}")]
    Io(String),
    #[error("shareable artifact contains forbidden key {0:?}")]
    ForbiddenKey(String),
    #[error("artifact encoding: {0}")]
    Encoding(String),
}

pub struct ArtifactWriter {
    paths: ArtifactPaths,
}

impl ArtifactWriter {
    pub fn new(paths: ArtifactPaths) -> Result<Self, ArtifactError> {
        paths
            .create()
            .map_err(|e| ArtifactError::Io(e.to_string()))?;
        Ok(Self { paths })
    }

    pub fn paths(&self) -> &ArtifactPaths {
        &self.paths
    }

    pub fn write_shareable(&self, manifest: &ShareableManifest) -> Result<(), ArtifactError> {
        assert_shareable_safe(manifest)?;
        write_json(&self.paths.shareable_manifest, manifest)
    }

    pub fn write_charts(&self, series: &ChartSeries) -> Result<(), ArtifactError> {
        write_json(&self.paths.shareable_charts, series)
    }

    pub fn write_result<T: Serialize>(&self, value: &T) -> Result<(), ArtifactError> {
        write_json(&self.paths.report, value)
    }

    pub fn write_private<T: Serialize>(
        &self,
        name: &str,
        value: &T,
    ) -> Result<PathBuf, ArtifactError> {
        let path = self.paths.root.join("private").join(name);
        write_json(&path, value)?;
        Ok(path)
    }

    pub fn copy_private_evidence(&self, events: &[EvidenceEvent]) -> Result<(), ArtifactError> {
        if let Some(parent) = self.paths.private_evidence.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ArtifactError::Io(e.to_string()))?;
        }
        let mut body = String::new();
        for event in events {
            body.push_str(
                &serde_json::to_string(event)
                    .map_err(|e| ArtifactError::Encoding(e.to_string()))?,
            );
            body.push('\n');
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&self.paths.private_evidence)
            .map_err(|e| ArtifactError::Io(e.to_string()))?;
        std::io::Write::write_all(&mut file, body.as_bytes())
            .map_err(|e| ArtifactError::Io(e.to_string()))?;
        std::fs::set_permissions(
            &self.paths.private_evidence,
            std::fs::Permissions::from_mode(0o600),
        )
        .map_err(|e| ArtifactError::Io(e.to_string()))
    }
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), ArtifactError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ArtifactError::Io(e.to_string()))?;
    }
    let private = path
        .components()
        .any(|component| component.as_os_str() == "private");
    let mode = if private { 0o600 } else { 0o644 };
    let bytes =
        serde_json::to_vec_pretty(value).map_err(|e| ArtifactError::Encoding(e.to_string()))?;
    let tmp = path.with_extension("tmp");
    {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(mode)
            .open(&tmp)
            .map_err(|e| ArtifactError::Io(e.to_string()))?;
        std::io::Write::write_all(&mut file, &bytes)
            .map_err(|e| ArtifactError::Io(e.to_string()))?;
        file.sync_all()
            .map_err(|e| ArtifactError::Io(e.to_string()))?;
    }
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode))
        .map_err(|e| ArtifactError::Io(e.to_string()))?;
    std::fs::rename(tmp, path).map_err(|e| ArtifactError::Io(e.to_string()))
}

/// Fail closed if a shareable projection contains secret-bearing keys.
pub fn assert_shareable_safe(value: &ShareableManifest) -> Result<(), ArtifactError> {
    let json = serde_json::to_value(value).map_err(|e| ArtifactError::Encoding(e.to_string()))?;
    scan(&json)
}

fn scan(value: &serde_json::Value) -> Result<(), ArtifactError> {
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                let normalized = key.to_ascii_lowercase();
                if is_forbidden_key(&normalized) {
                    return Err(ArtifactError::ForbiddenKey(key.clone()));
                }
                scan(value)?;
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                scan(value)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn is_forbidden_key(key: &str) -> bool {
    matches!(
        key,
        "sp_id"
            | "a_send"
            | "b_scan"
            | "b_spend"
            | "b_ark"
            | "shared_secret"
            | "ecdh"
            | "tweak"
            | "signing_key"
            | "mailbox_secret"
            | "private_key"
            | "password"
            | "credential"
            | "credentials"
            | "identity_id"
    ) || key.contains("secret")
}

/// Aggregate proof marker used in reports: identity-bearing evidence belongs
/// in the private store, not in the shareable manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactVisibility {
    Shareable,
    Private,
}

pub fn wide(value: u128) -> WideU128 {
    WideU128(value)
}
