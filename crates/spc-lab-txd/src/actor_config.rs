//! Complete allocation rendering and injective private driver roots.
use crate::builder::FeePolicy;
use crate::control::invalid;
use crate::journal::JournalCaps;
use crate::wallet::{ProvisionConfig, ValidatedAllocation};
use serde::{Deserialize, Serialize};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AllocationActor {
    pub driver_id: String,
    pub network_wallets: u32,
    pub user_payment_wallets: u32,
    pub hosts_bootstrap_funding: bool,
    pub lookahead: u32,
    pub fee_policy: FeePolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AllocationDescription {
    pub run_seed: u64,
    pub run_id: String,
    pub network: bitcoin::Network,
    pub caps: JournalCaps,
    pub actors: Vec<AllocationActor>,
    /// Sorted raw identity ids map to cumulative global wallet indices.
    pub users: Vec<String>,
}

impl AllocationDescription {
    pub fn reconstruct(&self) -> io::Result<ValidatedAllocation> {
        if self.network != bitcoin::Network::Regtest
            || !(1..=2).contains(&self.actors.len())
            || self.users.is_empty()
            || self.users.len() > 256
        {
            return Err(invalid("invalid allocation population"));
        }
        for user in &self.users {
            crate::keys::validate_run_id(user).map_err(|_| invalid("invalid user id"))?;
        }
        if !self.users.windows(2).all(|w| w[0] < w[1]) {
            return Err(invalid("users not uniquely sorted"));
        }
        if !self
            .actors
            .windows(2)
            .all(|w| w[0].driver_id < w[1].driver_id)
        {
            return Err(invalid("actors not uniquely sorted"));
        }
        let (mut network, mut users) = (0u32, 0u32);
        let mut configs = Vec::new();
        for actor in &self.actors {
            if actor.network_wallets == 0
                || actor.user_payment_wallets == 0
                || actor.lookahead == 0
                || actor.lookahead > 4096
            {
                return Err(invalid("actor role count or lookahead invalid"));
            }
            let n = network
                .checked_add(actor.network_wallets)
                .filter(|n| *n <= 256)
                .ok_or_else(|| invalid("network count overflow"))?;
            let u = users
                .checked_add(actor.user_payment_wallets)
                .filter(|n| *n <= 256)
                .ok_or_else(|| invalid("user count overflow"))?;
            actor
                .fee_policy
                .validate()
                .map_err(|_| invalid("invalid fee policy"))?;
            configs.push(ProvisionConfig {
                run_seed: self.run_seed,
                run_id: self.run_id.clone(),
                network: self.network,
                driver_id: actor.driver_id.clone(),
                caps: self.caps,
                network_wallets: network..=n - 1,
                user_payment_wallets: users..=u - 1,
                hosts_bootstrap_funding: actor.hosts_bootstrap_funding,
                lookahead: actor.lookahead,
                fee_policy: actor.fee_policy.clone(),
            });
            network = n;
            users = u;
        }
        if users as usize != self.users.len() {
            return Err(invalid("allocation identity count mismatch"));
        }
        ProvisionConfig::validate_allocation(&configs)
            .map_err(|_| invalid("global allocation validation failed"))
    }

    pub fn owner_of_user(&self, user: &str) -> io::Result<(&str, u32)> {
        let index = self
            .users
            .binary_search_by(|s| s.as_str().cmp(user))
            .map_err(|_| invalid("unknown user"))? as u32;
        let mut end = 0u32;
        for actor in &self.actors {
            end = end
                .checked_add(actor.user_payment_wallets)
                .ok_or_else(|| invalid("allocation overflow"))?;
            if index < end {
                return Ok((&actor.driver_id, index));
            }
        }
        Err(invalid("unallocated user"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActorConfig {
    #[serde(default)]
    pub lab_acceptance: bool,
    #[serde(default)]
    pub node_profile_binding: Option<crate::send_outcome::NodeProfileBinding>,
    pub chain_policy: crate::observer::ChainPolicy,
    pub size_classes: Vec<SizeClass>,
    pub protocol: String,
    pub history_version: u32,
    pub run_dir: PathBuf,
    pub driver_id: String,
    pub allocation: AllocationDescription,
    pub node_ids: Vec<String>,
    /// The selected bootstrap node is authority for EVERY driver.
    pub authority_node: String,
    pub ordinary_interval_ms: u64,
    pub max_vbytes: u64,
    pub max_mempool_backlog_tx: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SizeClass {
    pub vbytes: u64,
    pub mix: f64,
}

impl ActorConfig {
    pub fn validate(&self) -> io::Result<ValidatedAllocation> {
        self.chain_policy.pow_limit().map_err(|e| invalid(&e))?;
        if self.size_classes.is_empty()
            || self.size_classes.len() > 64
            || self.size_classes.iter().any(|c| {
                c.vbytes == 0
                    || c.vbytes > self.max_vbytes
                    || !c.mix.is_finite()
                    || c.mix <= 0.0
                    || c.mix > 1.0
            })
            || (self.size_classes.iter().map(|c| c.mix).sum::<f64>() - 1.0).abs() > 1e-9
        {
            return Err(invalid("traffic_size_mix_invalid"));
        }
        if self.protocol != crate::control::PROTOCOL
            || self.history_version != crate::journal::SUPPORTED_HISTORY_VERSION
            || self.ordinary_interval_ms == 0
            || self.max_vbytes == 0
            || self.max_vbytes > 100_000
            || self.max_mempool_backlog_tx == 0
            || self.max_mempool_backlog_tx > 2000
            || self.node_ids.is_empty()
            || self.node_ids.len() > 64
            || !self.node_ids.windows(2).all(|w| w[0] < w[1])
            || !self.node_ids.contains(&self.authority_node)
            || !self
                .allocation
                .actors
                .iter()
                .any(|a| a.driver_id == self.driver_id)
        {
            return Err(invalid("actor config binding invalid"));
        }
        for node in &self.node_ids {
            crate::keys::validate_run_id(node).map_err(|_| invalid("invalid node id"))?;
        }
        self.allocation.reconstruct()
    }

    pub fn root(&self) -> io::Result<PathBuf> {
        driver_root(&self.run_dir, &self.driver_id)
    }

    pub fn write(&self) -> io::Result<PathBuf> {
        self.validate()?;
        let root = self.root()?;
        confine_root(&self.run_dir, &root)?;
        let path = root.join("actor.json");
        let bytes = serde_json::to_vec(self)?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::File::open(&root)?.sync_all()?;
        Ok(path)
    }

    pub fn load(path: &Path) -> io::Result<Self> {
        private_file(path)?;
        if std::fs::metadata(path)?.len() > 1024 * 1024 {
            return Err(invalid("actor config too large"));
        }
        let bytes = std::fs::read(path)?;
        if bytes.len() > 1024 * 1024 {
            return Err(invalid("actor config too large"));
        }
        let config: Self = serde_json::from_value(crate::control::strict_json(&bytes)?)?;
        config.validate()?;
        let root = config.root()?;
        if root.join("actor.json") != path {
            return Err(invalid("actor config path mismatch"));
        }
        confine_root(&config.run_dir, &root)?;
        Ok(config)
    }
}

/// RFC 4648 unpadded base64url. Raw ActorId is never joined to a path.
pub fn driver_component(id: &str) -> io::Result<String> {
    crate::keys::validate_run_id(id).map_err(|_| invalid("invalid driver id"))?;
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut encoded = String::from("d-");
    for c in id.as_bytes().chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        for shift in [18, 12, 6, 0].into_iter().take(c.len() + 1) {
            encoded.push(ALPHABET[((n >> shift) & 63) as usize] as char);
        }
    }
    Ok(encoded)
}

pub fn driver_root(run: &Path, id: &str) -> io::Result<PathBuf> {
    Ok(run.join("private/traffic").join(driver_component(id)?))
}

pub fn confine_root(run: &Path, root: &Path) -> io::Result<()> {
    for dir in [
        run.join("private"),
        run.join("private/traffic"),
        root.to_path_buf(),
        root.join("private"),
        root.join("private/traffic"),
    ] {
        match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
        let meta = std::fs::symlink_metadata(&dir)?;
        if !meta.file_type().is_dir()
            || meta.permissions().mode() & 0o777 != 0o700
            || meta.uid() != std::fs::metadata("/proc/self")?.uid()
        {
            return Err(invalid("unsafe traffic directory"));
        }
    }
    for path in [
        root.join("private/traffic-wallets.json"),
        root.join("private/traffic/history.jsonl"),
        root.join("actor.json"),
    ] {
        match std::fs::symlink_metadata(&path) {
            Ok(_) => private_file(&path)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn private_file(path: &Path) -> io::Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.file_type().is_file()
        || meta.permissions().mode() & 0o777 != 0o600
        || meta.uid() != std::fs::metadata("/proc/self")?.uid()
        || meta.nlink() != 1
    {
        return Err(invalid("unsafe traffic file"));
    }
    Ok(())
}
