//! Runtime-only join from owned SPc listener/public order-key configuration.
//! No URL from a browser, private order key, wallet secret, or wallet DB is read.
use std::io::Read;
use std::net::{SocketAddr, SocketAddrV4};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use anyhow::{ensure, Context, Result};
use bitcoin::secp256k1::XOnlyPublicKey;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::WalletAgentSpec;
use crate::engine::RunEngine;
use crate::manifest::RunManifest;
use crate::scenario::{AddressSource, FrontRole, Scenario};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Binding {
    pub origin: String,
    pub order_pubkey: String,
}

struct Resolved {
    binding: Binding,
    front_path: PathBuf,
    wallet_dir: PathBuf,
    api: SocketAddrV4,
    sv2: SocketAddrV4,
    status: SocketAddrV4,
    address: String,
    authority: String,
    ark: toml::Value,
}

fn private_read(root: &Path, path: &Path) -> Result<Vec<u8>> {
    ensure!(
        root.canonicalize()? == root && path.canonicalize()? == path && path.starts_with(root),
        "wallet binding path is not canonical/run-owned"
    );
    let uid = std::fs::metadata("/proc/self")?.uid();
    let mut parent = path.parent().context("wallet binding parent")?;
    loop {
        let meta = std::fs::symlink_metadata(parent)?;
        ensure!(
            meta.is_dir() && meta.uid() == uid && meta.mode() & 0o777 == 0o700,
            "wallet binding directory is not private"
        );
        if parent == root {
            break;
        }
        parent = parent
            .parent()
            .context("wallet binding escaped private root")?;
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags((rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32)
        .open(path)?;
    let meta = file.metadata()?;
    ensure!(
        meta.is_file()
            && meta.uid() == uid
            && meta.mode() & 0o777 == 0o600
            && meta.len() <= 1024 * 1024,
        "wallet binding file is not bounded/private"
    );
    let mut bytes = Vec::new();
    file.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 1024 * 1024, "wallet binding file too large");
    Ok(bytes)
}

fn config(root: &Path, path: &Path) -> Result<toml::Value> {
    Ok(toml::from_str(std::str::from_utf8(&private_read(
        root, path,
    )?)?)?)
}

fn field<'a>(value: &'a toml::Value, section: &str, key: &str) -> Result<&'a str> {
    value
        .get(section)
        .and_then(|v| v.get(key))
        .and_then(toml::Value::as_str)
        .context("wallet binding configuration field missing")
}

fn leased(manifest: &RunManifest, name: &str) -> Result<SocketAddrV4> {
    let endpoint = manifest
        .resources
        .listeners
        .get(name)
        .context("wallet binding listener not leased")?;
    let SocketAddr::V4(addr) = endpoint.socket_addr()? else {
        anyhow::bail!("wallet binding requires actual host IPv4 placement");
    };
    ensure!(
        addr.ip().is_loopback() && addr.port() != 0,
        "wallet binding listener must be host loopback"
    );
    ensure!(
        manifest
            .resources
            .listeners
            .iter()
            .all(|(key, other)| key == name || other.port != addr.port()),
        "wallet binding listener lease is ambiguous"
    );
    Ok(addr)
}

fn resolve(
    scenario: &Scenario,
    manifest: &RunManifest,
    run_dir: &Path,
    agent: &WalletAgentSpec,
) -> Result<Resolved> {
    super::validate(scenario).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    ensure!(
        manifest.run_id == scenario.run.id && manifest.resources.run_dir == run_dir,
        "wallet binding run mismatch"
    );
    // ark_service::validate_runtime currently permits only host-local Ark.
    // Do not use a host loopback URL for an actor placed in another netns.
    ensure!(
        manifest.placement.is_none(),
        "wallet aggregate binding requires the current host-local Ark placement"
    );
    ensure!(
        scenario
            .wallet_agents
            .iter()
            .filter(|a| a.identity_id == agent.identity_id && a.front_id == agent.front_id)
            .count()
            == 1,
        "wallet binding agent is not uniquely declared"
    );
    let (ordinal, front) = scenario
        .fronts
        .iter()
        .enumerate()
        .find(|(_, f)| f.id == agent.front_id)
        .context("wallet binding front missing")?;
    ensure!(
        front.roles.contains(&FrontRole::Api),
        "wallet front has no SPc API role"
    );
    let pool = scenario
        .pools
        .iter()
        .find(|p| p.id == front.pool_id)
        .context("wallet binding pool missing")?;
    ensure!(
        scenario.ark_backends.len() == 1
            && scenario.ark_backends[0].local.is_some()
            && pool
                .ark_backend_id
                .as_ref()
                .is_none_or(|id| id == &scenario.ark_backends[0].id),
        "wallet binding Ark backend placement mismatch"
    );
    let api = leased(manifest, &format!("front.{}.api", front.id))?;
    let sv2 = leased(manifest, &format!("front.{}.sv2", front.id))?;
    let status = leased(manifest, &format!("wallet.{}.status", agent.identity_id))?;
    let private = run_dir.join("private");
    let front_path = private.join(format!("pool-{}.toml", front.id));
    let front_config = config(&private, &front_path)?;
    ensure!(
        field(&front_config, "api", "listen")? == api.to_string()
            && field(&front_config, "pool", "listen")? == sv2.to_string()
            && field(&front_config, "pool", "network")? == "regtest"
            && front_config["pool"]
                .get("front_id")
                .and_then(toml::Value::as_integer)
                == Some(i64::try_from((ordinal + 1).min(255))?),
        "wallet front configuration/lease mismatch"
    );
    ensure!(
        front_config["pool"]
            .get("roles")
            .and_then(toml::Value::as_array)
            .is_some_and(|roles| roles.iter().any(|v| v.as_str() == Some("api"))),
        "configured front lacks API role"
    );
    let redis = manifest
        .resources
        .redis_instances
        .iter()
        .find(|r| r.id == pool.storage.redis_instance_id && r.owner_pool_id == pool.id)
        .context("wallet pool Redis ownership")?;
    let pg = manifest
        .resources
        .postgres_databases
        .iter()
        .find(|p| p.id == pool.storage.postgres_database_id && p.owner_pool_id == pool.id)
        .context("wallet pool Postgres ownership")?;
    ensure!(
        field(&front_config, "storage", "redis")? == redis.url
            && field(&front_config, "storage", "postgres")? == pg.url,
        "wallet front storage belongs to another pool"
    );
    let role_root = private.join(format!("pool-ark-{}", pool.id));
    let role_bytes = private_read(&private, &role_root.join("pool.toml"))?;
    let role: toml::Value = toml::from_str(std::str::from_utf8(&role_bytes)?)?;
    ensure!(
        front_config
            .get("ark")
            .and_then(|v| v.get("enabled"))
            .and_then(toml::Value::as_bool)
            == Some(true)
            && role
                .get("ark")
                .and_then(|v| v.get("enabled"))
                .and_then(toml::Value::as_bool)
                == Some(true),
        "wallet aggregate pool is not Ark-enabled"
    );
    let provision: serde_json::Value =
        serde_json::from_slice(&private_read(&private, &role_root.join("provision.json"))?)?;
    ensure!(
        provision["schema"] == "spc.lab.pool-ark-provision/v1"
            && provision["pool_id"].as_str() == Some(pool.id.as_str())
            && provision["config_sha256"].as_str()
                == Some(hex::encode(Sha256::digest(&role_bytes)).as_str()),
        "wallet pool public order-key provenance mismatch"
    );
    ensure!(
        field(&role, "pool", "network")? == "regtest"
            && field(&role, "pool", "pool_payout_descriptor")?
                == field(&front_config, "pool", "pool_payout_descriptor")?,
        "wallet payout configuration belongs to another pool"
    );
    for name in [
        "order_pubkey",
        "ark_id",
        "server_pubkey",
        "mailbox_pubkey",
        "server",
    ] {
        ensure!(
            field(&front_config, "ark", name)? == field(&role, "ark", name)?,
            "wallet front/Ark-role public configuration mismatch"
        );
    }
    let order_pubkey = field(&front_config, "ark", "order_pubkey")?;
    let parsed: XOnlyPublicKey = order_pubkey.parse()?;
    ensure!(
        order_pubkey == parsed.to_string(),
        "wallet order verification key must be canonical"
    );
    let wallet_dir = private.join(format!("wallet-{}", agent.identity_id));
    let provision: serde_json::Value =
        serde_json::from_slice(&private_read(&private, &wallet_dir.join("provision.json"))?)?;
    ensure!(
        provision["schema"] == "spc.agent.provision/v1" && provision["network"] == "regtest",
        "wallet public provisioning mismatch"
    );
    let address = provision["sp_address"]
        .as_str()
        .context("wallet provisioned SP address missing")?
        .to_owned();
    spc_crypto::parse_sp_address(&address, spc_crypto::Network::Regtest)?;
    let identity = scenario
        .identities
        .iter()
        .find(|i| i.id == agent.identity_id)
        .context("wallet scenario identity missing")?;
    if let AddressSource::Explicit { address: expected } = &identity.address_source {
        ensure!(
            expected == &address,
            "wallet cold-provisioned identity differs from scenario"
        );
    }
    let authority = field(&front_config, "pool", "authority_public_key")?.to_owned();
    ensure!(!authority.is_empty(), "wallet front authority missing");
    Ok(Resolved {
        binding: Binding {
            origin: format!("http://{api}/"),
            order_pubkey: parsed.to_string(),
        },
        front_path,
        wallet_dir,
        api,
        sv2,
        status,
        address,
        authority,
        ark: front_config["ark"].clone(),
    })
}

/// Check the exact IPv4 address AND port, not another socket on the same port.
fn owns_listener(pid: u32, address: SocketAddrV4) -> Result<bool> {
    if std::fs::read_link(format!("/proc/{pid}/ns/net"))?
        != std::fs::read_link("/proc/self/ns/net")?
    {
        return Ok(false);
    }
    let wanted = format!(
        "{:08X}:{:04X}",
        u32::from_le_bytes(address.ip().octets()),
        address.port()
    );
    let table = std::fs::read_to_string("/proc/net/tcp")?;
    let inodes: Vec<_> = table
        .lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            (fields.len() >= 10 && fields[1] == wanted && fields[3] == "0A")
                .then(|| fields[9].parse::<u64>().ok())
                .flatten()
        })
        .collect();
    if inodes.len() != 1 {
        return Ok(false);
    }
    let wanted = format!("socket:[{}]", inodes[0]);
    Ok(std::fs::read_dir(format!("/proc/{pid}/fd"))?
        .filter_map(|e| e.ok())
        .any(|entry| std::fs::read_link(entry.path()).is_ok_and(|p| p.to_string_lossy() == wanted)))
}

fn owned_front(engine: &RunEngine, agent: &WalletAgentSpec, resolved: &Resolved) -> Result<()> {
    let manifest = engine.manifest_snapshot();
    let records: Vec<_> = manifest
        .processes
        .iter()
        .filter(|p| p.role == "front" && p.entity == agent.front_id.as_str())
        .collect();
    ensure!(
        records.len() == 1,
        "wallet API front process is not uniquely recorded"
    );
    let record = records[0];
    let configs: Vec<_> = record
        .args
        .windows(2)
        .filter(|w| w[0] == "--config")
        .collect();
    ensure!(
        configs.len() == 1 && Path::new(&configs[0][1]) == resolved.front_path,
        "wallet API process uses another configuration"
    );
    let supervisor = engine
        .supervisor
        .lock()
        .map_err(|_| anyhow::anyhow!("wallet supervisor unavailable"))?;
    ensure!(
        supervisor.verify(record).is_owned()
            && owns_listener(record.pid, resolved.api)?
            && owns_listener(record.pid, resolved.sv2)?
            && supervisor.verify(record).is_owned(),
        "wallet API listener ownership unavailable"
    );
    Ok(())
}

pub(super) fn for_launch(engine: &RunEngine, agent: &WalletAgentSpec) -> Result<Binding> {
    let resolved = resolve(
        &engine.scenario,
        &engine.manifest_snapshot(),
        &engine.run_dir,
        agent,
    )?;
    owned_front(engine, agent, &resolved)?;
    Ok(resolved.binding)
}

fn persisted_config(resolved: &Resolved, value: &toml::Value) -> Result<()> {
    let actual: Binding = value
        .get("ark")
        .and_then(|v| v.get("aggregate_api"))
        .context("wallet aggregate API unconfigured; watches remain unknown")?
        .clone()
        .try_into()?;
    ensure!(
        actual == resolved.binding,
        "persisted wallet aggregate origin/order key changed"
    );
    ensure!(
        field(value, "upstream", "sp_address")? == resolved.address
            && field(value, "upstream", "address")? == resolved.sv2.to_string()
            && field(value, "upstream", "authority_public_key")? == resolved.authority
            && field(value, "ark", "network")? == "regtest"
            && field(value, "ark", "status_listen")? == resolved.status.to_string(),
        "persisted wallet identity/front binding mismatch"
    );
    for (field_name, filename) in [
        ("scan_xprv", "scan.xprv"),
        ("ark_xprv", "ark.xprv"),
        ("seed_child", "child.seed"),
        ("datadir", "state"),
    ] {
        ensure!(
            Path::new(field(value, "ark", field_name)?) == resolved.wallet_dir.join(filename),
            "persisted wallet key/datadir path changed"
        );
    }
    for (wallet_key, pool_key) in [
        ("ark_id", "ark_id"),
        ("server_pubkey", "server_pubkey"),
        ("mailbox_pubkey", "mailbox_pubkey"),
        ("server_address", "server"),
    ] {
        ensure!(
            field(value, "ark", wallet_key)?
                == resolved
                    .ark
                    .get(pool_key)
                    .and_then(toml::Value::as_str)
                    .context("wallet Ark public binding missing")?,
            "persisted wallet Ark binding mismatch"
        );
    }
    // WalletKeys::load validates the cold-derived B_scan against this same
    // address and both branch/child bindings before the existing wallet opens.
    Ok(())
}

/// Read-only attach/adopt and pre-miner gate: never provision or rewrite.
pub(super) fn validate_persisted(engine: &RunEngine, agent: &WalletAgentSpec) -> Result<()> {
    let resolved = resolve(
        &engine.scenario,
        &engine.manifest_snapshot(),
        &engine.run_dir,
        agent,
    )?;
    let value = config(
        &engine.run_dir.join("private"),
        &resolved.wallet_dir.join("agent.toml"),
    )?;
    persisted_config(&resolved, &value)?;
    let expected_native = super::native_render::render(engine, agent, &resolved.address)?;
    let stored_native = value
        .get("ark")
        .and_then(|v| v.get("native_audit"))
        .map(serde_json::to_value)
        .transpose()?;
    ensure!(
        stored_native == expected_native,
        "persisted native audit profile differs from declared runtime"
    );
    owned_front(engine, agent, &resolved)
}

#[cfg(test)]
#[path = "wallet_aggregate_binding_tests.rs"]
mod tests;
