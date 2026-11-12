//! Durable run manifest and owner lock.
//!
//! The manifest is written before dependent processes start and updated after
//! every launch/adoption. It is the source of truth for crash reconciliation:
//! a command is only considered successful when this file proves its effect.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::artifacts::ArtifactPaths;
use crate::config::NodeEffectiveProfile;
use crate::ids::{NodeId, PoolId, RunId};
use crate::resource::{Endpoint, PostgresRuntime, RedisRuntime};
use crate::supervisor::ProcessRecord;

pub const MANIFEST_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Created,
    Preparing,
    Running,
    Stopping,
    Stopped,
    Failed,
    CompletedWithErrors,
}

impl RunState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Preparing => "preparing",
            Self::Running => "running",
            Self::Stopping => "stopping",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
            Self::CompletedWithErrors => "completed_with_errors",
        }
    }

    pub fn is_live(self) -> bool {
        matches!(
            self,
            Self::Created | Self::Preparing | Self::Running | Self::Stopping
        )
    }

    pub fn can_start(self) -> bool {
        matches!(self, Self::Created | Self::Preparing)
    }

    pub fn can_stop(self) -> bool {
        matches!(self, Self::Preparing | Self::Running)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BinaryIdentity {
    pub path: PathBuf,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceManifest {
    pub run_dir: PathBuf,
    pub listeners: BTreeMap<String, Endpoint>,
    pub redis_instances: Vec<RedisRuntime>,
    pub postgres_databases: Vec<PostgresRuntime>,
    pub temp_dirs: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockRecord {
    pub path: PathBuf,
    pub owner_instance: String,
    pub pid: u32,
    /// Metadata timestamp, not a TTL/heartbeat: ownership is the kernel flock.
    pub heartbeat_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunManifest {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub traffic_lab_acceptance: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traffic_preflight_state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traffic_stage: Option<crate::traffic::runtime::TrafficStage>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub traffic_launch_profiles: BTreeMap<NodeId, crate::traffic::launch_profile::LaunchProfile>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub traffic_driver_paths: BTreeMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traffic_measured_from: Option<crate::traffic::runtime::MeasuredBoundary>,
    pub schema_version: u32,
    pub run_id: RunId,
    pub owner_instance: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub base_revision: String,
    pub dirty_tree_hash: String,
    pub scenario_sha256: String,
    pub binary_paths_and_hashes: BTreeMap<String, BinaryIdentity>,
    pub resources: ResourceManifest,
    pub effective_profiles: BTreeMap<NodeId, NodeEffectiveProfile>,
    pub processes: Vec<ProcessRecord>,
    pub locks: LockRecord,
    pub state: RunState,
    pub artifact_paths: ArtifactPaths,
    pub warnings: Vec<String>,
    /// The published placement of the actors, when the run is namespaced.
    ///
    /// It is recorded before any configuration is rendered, so the manifest
    /// always carries the addresses the actors were actually given.
    #[serde(default)]
    pub placement: Option<serde_json::Value>,
    /// The recorded traffic capability projection (traffic plan Section
    /// 8), present when the run's scenario declares traffic. It is the
    /// projection of the canonical AP-1a report - never a second verdict -
    /// and the scenario preflight gate consumes it before any process
    /// starts.
    #[serde(default)]
    pub traffic_capability: Option<crate::traffic::projection::TrafficCapabilityRecord>,
}

impl RunManifest {
    pub fn load(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let bytes = std::fs::read(path)?;
        serde_json::from_slice(&bytes).map_err(|e| std::io::Error::other(e.to_string()))
    }

    pub fn save(&self, path: impl AsRef<Path>) -> std::io::Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        let bytes =
            serde_json::to_vec_pretty(self).map_err(|e| std::io::Error::other(e.to_string()))?;
        {
            let mut file = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .mode(0o600)
                .open(&tmp)?;
            file.write_all(&bytes)?;
            file.flush()?;
            file.sync_all()?;
        }
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn process(&self, role: &str, entity: &str) -> Option<&ProcessRecord> {
        self.processes
            .iter()
            .find(|record| record.role == role && record.entity == entity)
    }

    pub fn upsert_process(&mut self, record: ProcessRecord) {
        if let Some(existing) = self
            .processes
            .iter_mut()
            .find(|r| r.role == record.role && r.entity == record.entity)
        {
            *existing = record;
        } else {
            self.processes.push(record);
        }
    }

    pub fn redis(&self, id: &str) -> Option<&RedisRuntime> {
        self.resources.redis_instances.iter().find(|r| r.id == id)
    }

    pub fn postgres(&self, id: &str) -> Option<&PostgresRuntime> {
        self.resources
            .postgres_databases
            .iter()
            .find(|p| p.id == id)
    }

    pub fn storage_for_pool(&self, pool: &PoolId) -> Option<(&RedisRuntime, &PostgresRuntime)> {
        let redis = self
            .resources
            .redis_instances
            .iter()
            .find(|r| &r.owner_pool_id == pool)?;
        let pg = self
            .resources
            .postgres_databases
            .iter()
            .find(|p| &p.owner_pool_id == pool)?;
        Some((redis, pg))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LockError {
    #[error("run {run_id} is already owned by live instance {owner_instance} (pid {pid})")]
    AlreadyLocked {
        run_id: String,
        owner_instance: String,
        pid: u32,
    },
    #[error("lock io error: {0}")]
    Io(String),
}

/// A lock file plus a heartbeat. A live owner prevents a second supervisor; a
/// dead owner leaves a stale lock that adoption may take over.
#[derive(Debug)]
pub struct RunLock {
    path: PathBuf,
    owner_instance: String,
    pid: u32,
    held: bool,
    file: Option<File>,
}

impl RunLock {
    pub fn acquire(
        path: impl AsRef<Path>,
        run_id: &RunId,
        owner_instance: &str,
    ) -> Result<Self, LockError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| LockError::Io(e.to_string()))?;
        }
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&path)
            .map_err(|e| LockError::Io(e.to_string()))?;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        match rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive) {
            Ok(()) => {}
            Err(error)
                if error == rustix::io::Errno::WOULDBLOCK || error == rustix::io::Errno::AGAIN =>
            {
                let mut metadata = String::new();
                let _ = file.read_to_string(&mut metadata);
                let parsed = serde_json::from_str::<serde_json::Value>(&metadata).ok();
                let owner = parsed
                    .as_ref()
                    .and_then(|value| value.get("owner_instance"))
                    .and_then(|value| value.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let pid = parsed
                    .as_ref()
                    .and_then(|value| value.get("pid"))
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0) as u32;
                return Err(LockError::AlreadyLocked {
                    run_id: run_id.to_string(),
                    owner_instance: owner,
                    pid,
                });
            }
            Err(error) => return Err(LockError::Io(std::io::Error::from(error).to_string())),
        }
        let pid = std::process::id();
        let body = serde_json::to_vec(&serde_json::json!({
            "run_id": run_id.as_str(),
            "owner_instance": owner_instance,
            "pid": pid,
            "heartbeat_at": now_ms(),
        }))
        .map_err(|e| LockError::Io(e.to_string()))?;
        file.set_len(0)
            .and_then(|_| file.write_all(&body))
            .and_then(|_| file.sync_all())
            .map_err(|e| LockError::Io(e.to_string()))?;
        Ok(Self {
            path,
            owner_instance: owner_instance.to_string(),
            pid,
            held: true,
            file: Some(file),
        })
    }

    pub fn record(&self) -> LockRecord {
        LockRecord {
            path: self.path.clone(),
            owner_instance: self.owner_instance.clone(),
            pid: self.pid,
            heartbeat_at: now_ms(),
        }
    }

    pub fn release(mut self) {
        if let Some(file) = self.file.take() {
            let _ = rustix::fs::flock(&file, rustix::fs::FlockOperation::Unlock);
        }
        self.held = false;
    }
}

impl Drop for RunLock {
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            let _ = rustix::fs::flock(&file, rustix::fs::FlockOperation::Unlock);
        }
        self.held = false;
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
