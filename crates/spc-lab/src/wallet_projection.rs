//! Private controller projection of explicitly registered agent wallets.
//! No SQLite access, key material, RPC credentials or browser-supplied path.
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::access::AccessClaim;
use crate::ids::{FrontId, IdentityId, PoolId};
use crate::scenario::Scenario;

pub const INPUTS_MAX_AGE_MS: u64 = 15_000;

#[derive(Clone)]
pub struct WalletBinding {
    pub identity_id: IdentityId,
    pub pool_id: PoolId,
    pub status_addr: SocketAddr,
    pub datadir: PathBuf,
}

#[derive(Default)]
pub struct WalletRegistry(BTreeMap<IdentityId, WalletBinding>);

/// Supplied only by the scenario/resource registry, never by an HTTP body.
pub struct PersistedWalletBinding {
    pub identity_id: IdentityId,
    pub front_id: FrontId,
    pub status_addr: SocketAddr,
    pub front_addr: String,
    pub front_config_path: PathBuf,
    pub config_path: PathBuf,
}

impl WalletRegistry {
    /// Reconstruct after attach/adopt without spawning, provisioning, querying
    /// SQLite, rewriting config or manufacturing an initialized wallet.
    pub fn bind_persisted(
        &mut self,
        scenario: &Scenario,
        run_dir: &Path,
        declared: PersistedWalletBinding,
    ) -> Result<(), &'static str> {
        let private = run_dir
            .join("private")
            .canonicalize()
            .map_err(|_| "wallet_path_invalid")?;
        let path = &declared.config_path;
        if path
            .canonicalize()
            .map_err(|_| "wallet_config_unavailable")?
            != *path
            || !path.starts_with(&private)
        {
            return Err("wallet_path_invalid");
        }
        let parent = path.parent().ok_or("wallet_path_invalid")?;
        private_dir(parent)?;
        let bytes = private_read(path)?;
        let config: toml::Value =
            toml::from_str(std::str::from_utf8(&bytes).map_err(|_| "wallet_config_invalid")?)
                .map_err(|_| "wallet_config_invalid")?;
        let string = |section: &str, key: &str| {
            config
                .get(section)
                .and_then(|v| v.get(key))
                .and_then(toml::Value::as_str)
        };
        let front_path = &declared.front_config_path;
        if front_path
            .canonicalize()
            .map_err(|_| "wallet_front_config_unavailable")?
            != *front_path
            || !front_path.starts_with(&private)
        {
            return Err("wallet_path_invalid");
        }
        let front_bytes = private_read(front_path)?;
        let front_config: toml::Value = toml::from_str(
            std::str::from_utf8(&front_bytes).map_err(|_| "wallet_front_config_invalid")?,
        )
        .map_err(|_| "wallet_front_config_invalid")?;
        let pool = front_config
            .get("pool")
            .ok_or("wallet_front_config_invalid")?;
        let authority = pool
            .get("authority_public_key")
            .and_then(toml::Value::as_str)
            .filter(|key| !key.is_empty())
            .ok_or("wallet_front_config_invalid")?;
        let datadir = parent.join("state");
        if string("ark", "network") != Some("regtest")
            || string("ark", "datadir") != datadir.to_str()
            || string("ark", "status_listen") != Some(declared.status_addr.to_string().as_str())
            || string("upstream", "address") != Some(declared.front_addr.as_str())
            || pool.get("network").and_then(toml::Value::as_str) != Some("regtest")
            || pool.get("listen").and_then(toml::Value::as_str)
                != Some(declared.front_addr.as_str())
            || string("upstream", "authority_public_key") != Some(authority)
        {
            return Err("wallet_config_binding_mismatch");
        }
        let provision: Value =
            serde_json::from_slice(&private_read(&parent.join("provision.json"))?)
                .map_err(|_| "wallet_provision_invalid")?;
        let address = provision["sp_address"]
            .as_str()
            .ok_or("wallet_provision_invalid")?;
        if provision["schema"] != "spc.agent.provision/v1"
            || provision["network"] != "regtest"
            || string("upstream", "sp_address") != Some(address)
        {
            return Err("wallet_identity_mismatch");
        }
        spc_crypto::parse_sp_address(address, spc_crypto::Network::Regtest)
            .map_err(|_| "wallet_identity_invalid")?;
        let identity = scenario
            .identities
            .iter()
            .find(|i| i.id == declared.identity_id)
            .ok_or("wallet_identity_invalid")?;
        if let crate::scenario::AddressSource::Explicit { address: expected } =
            &identity.address_source
        {
            if expected != address {
                return Err("wallet_identity_mismatch");
            }
        }
        if datadir.canonicalize().map_err(|_| "wallet_uninitialized")? != datadir {
            return Err("wallet_path_invalid");
        }
        private_dir(&datadir)?;
        private_dir(&datadir.join("bark"))?;
        for name in ["state.sqlite", "bark/db.sqlite"] {
            let meta = std::fs::symlink_metadata(datadir.join(name))
                .map_err(|_| "wallet_uninitialized")?;
            let uid = std::fs::metadata("/proc/self")
                .map_err(|_| "wallet_unavailable")?
                .uid();
            if !meta.is_file() || meta.uid() != uid || meta.mode() & 0o777 != 0o600 {
                return Err("wallet_uninitialized");
            }
        }
        self.bind(
            scenario,
            run_dir,
            &declared.identity_id,
            &declared.front_id,
            declared.status_addr,
            datadir,
        )
    }

    /// AP-3b launch hook only, called for each declared Scenario.wallet_agents
    /// entry. This is not a HTTP mutation. All paths/listeners are run-owned.
    pub fn bind(
        &mut self,
        scenario: &Scenario,
        run_dir: &Path,
        identity: &IdentityId,
        front: &FrontId,
        status_addr: SocketAddr,
        datadir: PathBuf,
    ) -> Result<(), &'static str> {
        let bindings =
            crate::access::derive_user_bindings(scenario).map_err(|_| "wallet_mapping_invalid")?;
        let owner = bindings
            .iter()
            .find(|b| &b.identity_id == identity)
            .ok_or("wallet_mapping_invalid")?;
        if !scenario
            .fronts
            .iter()
            .any(|f| &f.id == front && f.pool_id == owner.pool_id)
            || !status_addr.ip().is_loopback()
            || self.0.contains_key(identity)
            || self
                .0
                .values()
                .any(|b| b.datadir == datadir || b.status_addr == status_addr)
        {
            return Err("wallet_mapping_invalid");
        }
        let private = run_dir
            .join("private")
            .canonicalize()
            .map_err(|_| "wallet_path_invalid")?;
        let canonical = datadir.canonicalize().map_err(|_| "wallet_path_invalid")?;
        if canonical != datadir || !canonical.starts_with(&private) {
            return Err("wallet_path_invalid");
        }
        private_dir(&datadir)?;
        self.0.insert(
            identity.clone(),
            WalletBinding {
                identity_id: identity.clone(),
                pool_id: owner.pool_id.clone(),
                status_addr,
                datadir,
            },
        );
        Ok(())
    }

    pub fn get(&self, identity: &IdentityId) -> Option<WalletBinding> {
        self.0.get(identity).cloned()
    }

    pub fn scoped(&self, claim: &AccessClaim) -> Vec<WalletBinding> {
        self.0
            .values()
            .filter(|b| {
                claim.is_operator()
                    || (claim.identity_id.as_ref() == Some(&b.identity_id)
                        && claim.may.iter().any(|m| m == "view_own_status")
                        && claim.pool_id.as_ref() == Some(&b.pool_id))
            })
            .cloned()
            .collect()
    }
}

pub fn private_dir(path: &Path) -> Result<(), &'static str> {
    let meta = std::fs::symlink_metadata(path).map_err(|_| "wallet_unavailable")?;
    let uid = std::fs::metadata("/proc/self")
        .map_err(|_| "wallet_unavailable")?
        .uid();
    if !meta.is_dir() || meta.mode() & 0o777 != 0o700 || meta.uid() != uid {
        return Err("wallet_private_path_invalid");
    }
    Ok(())
}

pub fn private_read(path: &Path) -> Result<Vec<u8>, &'static str> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags((rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32)
        .open(path)
        .map_err(|_| "wallet_provider_unavailable")?;
    let meta = file.metadata().map_err(|_| "wallet_provider_unavailable")?;
    let uid = std::fs::metadata("/proc/self")
        .map_err(|_| "wallet_provider_unavailable")?
        .uid();
    if !meta.is_file()
        || meta.mode() & 0o777 != 0o600
        || meta.uid() != uid
        || meta.len() > 1024 * 1024
    {
        return Err("wallet_private_file_invalid");
    }
    let mut bytes = Vec::new();
    file.take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "wallet_provider_unavailable")?;
    if bytes.len() > 1024 * 1024 {
        return Err("wallet_provider_invalid");
    }
    Ok(bytes)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VtxoSummary {
    pub id: String,
    pub amount_sats: String,
    pub state: String,
    pub expiry_height: Option<u32>,
    pub rail: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PrivateInputs {
    pub schema: String,
    pub updated_at_ms: u64,
    pub inputs: Vec<VtxoSummary>,
}

pub fn inputs(binding: &WalletBinding) -> Result<PrivateInputs, &'static str> {
    input_snapshot(binding).map(|(dto, _)| dto)
}

fn input_snapshot(binding: &WalletBinding) -> Result<(PrivateInputs, String), &'static str> {
    private_dir(&binding.datadir)?;
    let bytes = private_read(&binding.datadir.join("ui-inputs.json"))?;
    let dto: PrivateInputs = serde_json::from_slice(&bytes).map_err(|_| "wallet_inputs_invalid")?;
    let now = crate::command::now_ms();
    if dto.schema != "spc-agent-private-inputs/v1"
        || dto.inputs.len() > 4096
        || dto.updated_at_ms > now
        || now - dto.updated_at_ms > INPUTS_MAX_AGE_MS
    {
        return Err("wallet_inputs_stale");
    }
    let mut seen = std::collections::BTreeSet::new();
    for row in &dto.inputs {
        if row.id.len() != 72
            || !row.id.bytes().all(|c| c.is_ascii_hexdigit())
            || !seen.insert(row.id.to_ascii_lowercase())
            || !matches!(row.rail.as_str(), "derived" | "normal")
            || !matches!(
                row.state.as_str(),
                "spendable" | "pending" | "reserved" | "spent" | "unknown"
            )
            || row.amount_sats.parse::<u64>().ok().map(|n| n.to_string())
                != Some(row.amount_sats.clone())
        {
            return Err("wallet_inputs_invalid");
        }
    }
    Ok((dto, hex::encode(Sha256::digest(&bytes))))
}

fn status(binding: &WalletBinding) -> Result<Value, &'static str> {
    let mut socket = TcpStream::connect_timeout(&binding.status_addr, Duration::from_secs(1))
        .map_err(|_| "agent_unavailable")?;
    socket
        .set_read_timeout(Some(Duration::from_secs(1)))
        .map_err(|_| "agent_unavailable")?;
    socket
        .set_write_timeout(Some(Duration::from_secs(1)))
        .map_err(|_| "agent_unavailable")?;
    write!(
        socket,
        "GET /status HTTP/1.0\r\nHost: {}\r\nConnection: close\r\n\r\n",
        binding.status_addr
    )
    .map_err(|_| "agent_unavailable")?;
    let mut bytes = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut chunk = [0u8; 4096];
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("agent_unavailable")?;
        socket
            .set_read_timeout(Some(remaining))
            .map_err(|_| "agent_unavailable")?;
        let count = socket.read(&mut chunk).map_err(|_| "agent_unavailable")?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..count]);
        if bytes.len() > 65536 {
            return Err("agent_status_invalid");
        }
    }
    if bytes.len() > 65536 {
        return Err("agent_status_invalid");
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| "agent_status_invalid")?;
    let (head, body) = text.split_once("\r\n\r\n").ok_or("agent_status_invalid")?;
    if !head
        .lines()
        .next()
        .is_some_and(|s| s.starts_with("HTTP/1.0 200 ") || s.starts_with("HTTP/1.1 200 "))
    {
        return Err("agent_unavailable");
    }
    let raw: Value = serde_json::from_str(body).map_err(|_| "agent_status_invalid")?;
    project_status(&raw)
}

/// The source's raw last_error is deliberately never relayed. Agent v1 money
/// is u64 JSON; turn it into decimal strings *before* browser serialization.
pub fn project_status(raw: &Value) -> Result<Value, &'static str> {
    let state = raw["state"]
        .as_str()
        .filter(|s| {
            matches!(
                *s,
                "initializing"
                    | "awaiting_registration"
                    | "connected"
                    | "dormant"
                    | "error"
                    | "stopped"
            )
        })
        .ok_or("agent_status_invalid")?;
    let registered = raw["registered"].as_bool().ok_or("agent_status_invalid")?;
    let updated = raw["updated_at"]
        .as_str()
        .filter(|s| s.len() <= 64)
        .ok_or("agent_status_invalid")?;
    let mut out = json!({"state":state, "registered":registered, "updated_at":updated,
        "last_error": if raw["last_error"].is_null() { Value::Null } else { json!("agent_reported_error") }});
    for key in ["spendable_sats", "pending_sats", "needs_refresh_sats"] {
        out[key] = match raw.get(key) {
            Some(Value::Null) => Value::Null,
            Some(v) => json!(v.as_u64().ok_or("agent_status_invalid")?.to_string()),
            None => return Err("agent_status_invalid"),
        };
    }
    for key in [
        "min_expiry_height",
        "next_refresh_height",
        "last_scan_height",
        "registration_live_until_unix_ms",
    ] {
        out[key] = match raw.get(key) {
            None | Some(Value::Null) => Value::Null,
            Some(v) => json!(v.as_u64().ok_or("agent_status_invalid")?),
        };
    }
    Ok(out)
}

pub fn view(binding: &WalletBinding) -> Value {
    let mut out = json!({"identity_id":binding.identity_id, "pool_id":binding.pool_id,
        "source":"spc-agent", "observed_at_ms":crate::command::now_ms(),
        "status":null, "inputs":null, "inputs_updated_at_ms":null,
        "inputs_source":null, "inputs_snapshot_sha256":null,
        "watch_only":{"state":"not_available","reason":"canonical scan summary provider absent"},
        "payment_wallet":{"state":"not_available","reason":"T1 per-user balance/address provider absent"}});
    match status(binding) {
        Ok(s) => out["status"] = s,
        Err(code) => out["status_error"] = json!(code),
    }
    match input_snapshot(binding) {
        Ok((dto, hash)) => {
            out["inputs"] = json!(dto.inputs);
            out["inputs_updated_at_ms"] = json!(dto.updated_at_ms);
            out["inputs_source"] = json!("spc-agent-private-vtxos");
            out["inputs_snapshot_sha256"] = json!(hash);
        }
        Err(code) => out["inputs_error"] = json!(code),
    }
    out
}
