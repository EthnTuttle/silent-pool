use std::path::Path;
use std::str::FromStr;

use secp256k1_legacy::{PublicKey, XOnlyPublicKey};
use serde::{Deserialize, Serialize};
use stratum_apps::key_utils::Secp256k1PublicKey;
use thiserror::Error;

use crate::budget::{BudgetError, HashBudget};
use crate::hash::NtimePolicy;
use crate::ids::{ConnectionId, IdentityId, MinerId, WorkerId};

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("unsupported config_version {0}; expected 1")]
    Version(u16),
    #[error("invalid id {0:?}: use 1-128 ASCII alphanumeric, '.', '_', ':', or '-' characters")]
    Id(String),
    #[error("identity must be 1-255 UTF-8 bytes")]
    Identity,
    #[error("endpoint must not be empty")]
    Endpoint,
    #[error("authority public key must be x-only/compressed-SEC hex or key-utils base58check")]
    AuthorityFormat,
    #[error("authority public key is not a valid secp256k1 point")]
    AuthorityPoint,
    #[error("extranonce size must be in 4..=32")]
    ExtranonceSize,
    #[error("a connection supports at most 256 workers")]
    WorkerCount,
    #[error("worker batch_size must be in 1..=4096")]
    BatchSize,
    #[error("max_ntime_roll_seconds must be <= 7200")]
    NtimeRoll,
    #[error("worker id {0} is duplicated on this connection")]
    DuplicateWorker(String),
    #[error("connection id {0} is duplicated")]
    DuplicateConnection(String),
    #[error("worker id {0} is duplicated across the miner")]
    GlobalDuplicateWorker(String),
    #[error("reconnect initial_backoff_ms must be greater than zero")]
    ReconnectInitial,
    #[error("reconnect max_backoff_ms must be >= initial_backoff_ms")]
    ReconnectMax,
    #[error("handshake_timeout_ms must be >= 100")]
    HandshakeTimeout,
    #[error("event_queue_capacity must be in 1..=65536")]
    EventQueue,
    #[error("invalid native source-audit configuration: {0}")]
    SourceAudit(String),
    #[error(transparent)]
    Budget(#[from] BudgetError),
    #[error("config I/O: {0}")]
    Io(String),
    #[error("config TOML: {0}")]
    Toml(String),
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReconnectConfig {
    pub enabled: bool,
    pub initial_backoff_ms: u64,
    pub max_backoff_ms: u64,
    pub handshake_timeout_ms: u64,
}

impl Default for ReconnectConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            initial_backoff_ms: 100,
            max_backoff_ms: 5_000,
            handshake_timeout_ms: 10_000,
        }
    }
}

impl ReconnectConfig {
    fn validate(&self) -> Result<(), ConfigError> {
        if self.initial_backoff_ms == 0 {
            return Err(ConfigError::ReconnectInitial);
        }
        if self.max_backoff_ms < self.initial_backoff_ms {
            return Err(ConfigError::ReconnectMax);
        }
        if self.handshake_timeout_ms < 100 {
            return Err(ConfigError::HandshakeTimeout);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfig {
    pub id: WorkerId,
    pub budget: HashBudget,
    #[serde(default)]
    pub ntime_policy: NtimePolicy,
    #[serde(default = "default_max_roll")]
    pub max_ntime_roll_seconds: u32,
    #[serde(default = "default_batch_size")]
    pub batch_size: u32,
}

const fn default_max_roll() -> u32 {
    600
}
const fn default_batch_size() -> u32 {
    256
}

impl WorkerConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        validate_id(&self.id)?;
        self.budget.validate()?;
        if !(1..=4096).contains(&self.batch_size) {
            return Err(ConfigError::BatchSize);
        }
        if self.max_ntime_roll_seconds > 7_200 {
            return Err(ConfigError::NtimeRoll);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConnectionConfig {
    pub id: ConnectionId,
    pub endpoint: String,
    pub identity: IdentityId,
    pub authority_public_key: String,
    /// Deliberately REQUIRED in serialized configs. Old missing fields do not
    /// silently turn a full lab connection into unaudited work.
    pub source_audit: SourceAuditConfig,
    #[serde(default = "default_extranonce_size")]
    pub min_extranonce_size: u16,
    #[serde(default)]
    pub reconnect: ReconnectConfig,
    pub workers: Vec<WorkerConfig>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum SourceAuditConfig {
    Required {
        run_id: String,
        miner_id: String,
        wallet_socket: std::path::PathBuf,
        wallet_instance: [u8; 16],
        wallet_process: spc_scan::native_activation::ProcessIdentity,
        wallet_socket_device: u64,
        wallet_socket_inode: u64,
        peer_generation: u64,
        connection_config_digest: [u8; 32],
        collector_pubkey: String,
        pool_pubkey: String,
    },
    /// Explicit smoke/compatibility profile; its events never claim auditing.
    Unaudited {},
}

impl SourceAuditConfig {
    pub fn required(&self) -> bool {
        matches!(self, Self::Required { .. })
    }
    pub fn validate(&self, identity: &str) -> Result<(), ConfigError> {
        if let Self::Required {
            run_id,
            miner_id,
            wallet_socket,
            wallet_instance,
            wallet_process,
            wallet_socket_inode,
            peer_generation,
            connection_config_digest,
            collector_pubkey,
            pool_pubkey,
            ..
        } = self
        {
            validate_id(run_id)?;
            validate_id(miner_id)?;
            if wallet_instance == &[0; 16]
                || *peer_generation == 0
                || connection_config_digest == &[0; 32]
            {
                return Err(ConfigError::SourceAudit(
                    "missing lifecycle/process/config binding".into(),
                ));
            }
            wallet_process
                .validate()
                .map_err(|e| ConfigError::SourceAudit(e.into()))?;
            if *wallet_socket_inode == 0 {
                return Err(ConfigError::SourceAudit(
                    "missing wallet socket inode".into(),
                ));
            }
            if !wallet_socket.is_absolute()
                || wallet_socket
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                return Err(ConfigError::SourceAudit(
                    "socket must be absolute and confined".into(),
                ));
            }
            spc_scan::native_activation::VerificationKeys::for_regtest_identity(
                identity,
                collector_pubkey,
                pool_pubkey,
            )
            .map_err(|e| ConfigError::SourceAudit(e.into()))?;
        }
        Ok(())
    }
}

const fn default_extranonce_size() -> u16 {
    4
}

impl ConnectionConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        validate_id(&self.id)?;
        if self.identity.is_empty() || self.identity.len() > 255 {
            return Err(ConfigError::Identity);
        }
        if self.endpoint.trim().is_empty() {
            return Err(ConfigError::Endpoint);
        }
        self.authority()?;
        self.source_audit.validate(&self.identity)?;
        if let SourceAuditConfig::Required {
            connection_config_digest,
            ..
        } = &self.source_audit
        {
            let value =
                serde_json::to_value(self).map_err(|e| ConfigError::SourceAudit(e.to_string()))?;
            if spc_scan::native_activation::connection_config_digest(&value)
                .map_err(|e| ConfigError::SourceAudit(e.into()))?
                != *connection_config_digest
            {
                return Err(ConfigError::SourceAudit(
                    "connection configuration digest mismatch".into(),
                ));
            }
        }
        if self.source_audit.required()
            && self
                .workers
                .iter()
                .any(|w| w.ntime_policy != NtimePolicy::Hold)
        {
            return Err(ConfigError::SourceAudit(
                "audited candidate ntime is immutable; use hold".into(),
            ));
        }
        if !(4..=32).contains(&self.min_extranonce_size) {
            return Err(ConfigError::ExtranonceSize);
        }
        if self.workers.len() > 256 {
            return Err(ConfigError::WorkerCount);
        }
        self.reconnect.validate()?;
        let mut workers = std::collections::BTreeSet::new();
        for worker in &self.workers {
            worker.validate()?;
            if !workers.insert(worker.id.as_str()) {
                return Err(ConfigError::DuplicateWorker(worker.id.to_string()));
            }
        }
        Ok(())
    }

    /// Parse an x-only key or its 33-byte compressed equivalent. Authentication
    /// always supplies Some(key) to the Noise initiator; there is no bypass.
    pub fn authority(&self) -> Result<Secp256k1PublicKey, ConfigError> {
        parse_authority_public_key(&self.authority_public_key)
    }

    pub fn nominal_hash_rate(&self) -> f32 {
        let rate = self.workers.iter().fold(0u128, |sum, worker| {
            sum.saturating_add(u128::from(worker.budget.hashes_per_second.unwrap_or(0)))
        });
        rate.min(f32::MAX as u128) as f32
    }
}

pub fn parse_authority_public_key(value: &str) -> Result<Secp256k1PublicKey, ConfigError> {
    if let Ok(bytes) = hex::decode(value) {
        match bytes.len() {
            32 => {
                let key =
                    XOnlyPublicKey::from_str(value).map_err(|_| ConfigError::AuthorityPoint)?;
                return Ok(Secp256k1PublicKey(key));
            }
            33 => {
                let key = PublicKey::from_slice(&bytes).map_err(|_| ConfigError::AuthorityPoint)?;
                return Ok(Secp256k1PublicKey(key.x_only_public_key().0));
            }
            _ => return Err(ConfigError::AuthorityFormat),
        }
    }
    Secp256k1PublicKey::from_str(value).map_err(|_| ConfigError::AuthorityFormat)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MinerConfig {
    pub config_version: u16,
    pub miner_id: MinerId,
    #[serde(default = "default_event_queue")]
    pub event_queue_capacity: usize,
    pub connections: Vec<ConnectionConfig>,
}

const fn default_event_queue() -> usize {
    4096
}

impl MinerConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.config_version != 1 {
            return Err(ConfigError::Version(self.config_version));
        }
        validate_id(&self.miner_id)?;
        if !(1..=65_536).contains(&self.event_queue_capacity) {
            return Err(ConfigError::EventQueue);
        }
        let mut connections = std::collections::BTreeSet::new();
        let mut workers = std::collections::BTreeSet::new();
        for connection in &self.connections {
            connection.validate()?;
            if !connections.insert(connection.id.as_str()) {
                return Err(ConfigError::DuplicateConnection(connection.id.to_string()));
            }
            for worker in &connection.workers {
                if !workers.insert(worker.id.as_str()) {
                    return Err(ConfigError::GlobalDuplicateWorker(worker.id.to_string()));
                }
            }
        }
        if workers.len() > usize::from(u16::MAX) {
            return Err(ConfigError::GlobalDuplicateWorker(
                "worker slot space exhausted".into(),
            ));
        }
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|e| ConfigError::Io(e.to_string()))?;
        let config: Self = toml::from_str(&text).map_err(|e| ConfigError::Toml(e.to_string()))?;
        config.validate()?;
        Ok(config)
    }
}

pub fn validate_id(id: &str) -> Result<(), ConfigError> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
    {
        Err(ConfigError::Id(id.to_string()))
    } else {
        Ok(())
    }
}
