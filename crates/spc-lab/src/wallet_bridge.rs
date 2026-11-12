//! Explicit wallet actors and the common initial/dynamic registration barrier.
//! No traffic signer, funding ledger or implicit hashrate lives in this module.
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::engine::RunEngine;
use crate::ids::{FrontId, IdentityId};
use crate::scenario::{ConnectionSpec, Scenario, ScenarioError};

#[path = "wallet_aggregate_binding.rs"]
mod aggregate_binding;
#[path = "wallet_native_render.rs"]
mod native_render;
pub use native_render::NativeAuditProfile;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WalletAgentSpec {
    pub identity_id: IdentityId,
    pub front_id: FrontId,
    #[serde(default)]
    pub native_audit: Option<NativeAuditProfile>,
}

pub fn validate(scenario: &Scenario) -> Result<(), ScenarioError> {
    let bad = |s| ScenarioError::new("wallet_identity_mapping", s);
    let mut seen = BTreeSet::new();
    for agent in &scenario.wallet_agents {
        if let Some(profile) = &agent.native_audit {
            profile
                .validate()
                .map_err(|_| bad("invalid native wallet collector/public-key profile"))?;
        }
        if !seen.insert(&agent.identity_id)
            || !scenario
                .identities
                .iter()
                .any(|i| i.id == agent.identity_id)
        {
            return Err(bad(
                "wallet requires exactly one declared identity and one writer",
            ));
        }
        let front = scenario
            .fronts
            .iter()
            .find(|f| f.id == agent.front_id)
            .ok_or_else(|| bad("wallet front missing"))?;
        if !scenario.pools.iter().any(|p| {
            p.id == front.pool_id
                && (p.ark_backend_id.is_some()
                    || scenario.ark_backends.iter().any(|b| b.local.is_some()))
        }) {
            return Err(bad("wallet front must belong to an Ark pool"));
        }
    }
    for connection in scenario.miners.iter().flat_map(|m| &m.connections) {
        validate_connection(scenario, connection).map_err(|_| {
            bad("Ark mining connection needs one configured wallet in the same pool")
        })?;
    }
    Ok(())
}

fn validate_connection<'a>(
    scenario: &'a Scenario,
    c: &ConnectionSpec,
) -> Result<Option<&'a WalletAgentSpec>> {
    let front = scenario
        .fronts
        .iter()
        .find(|f| f.id == c.front_id)
        .context("unknown mining front")?;
    let pool = scenario
        .pools
        .iter()
        .find(|p| p.id == front.pool_id)
        .context("unknown pool")?;
    if pool.ark_backend_id.is_none() && !scenario.ark_backends.iter().any(|b| b.local.is_some()) {
        return Ok(None);
    }
    let agents: Vec<_> = scenario
        .wallet_agents
        .iter()
        .filter(|a| a.identity_id == c.identity_id)
        .collect();
    ensure!(
        agents.len() == 1,
        "Ark identity needs exactly one wallet writer"
    );
    let agent = agents[0];
    ensure!(
        scenario
            .fronts
            .iter()
            .any(|f| f.id == agent.front_id && f.pool_id == front.pool_id),
        "wallet registration targets another pool"
    );
    Ok(Some(agent))
}

fn private_file(path: &Path, bytes: &[u8]) -> Result<()> {
    if path.exists() {
        ensure!(
            std::fs::read(path)? == bytes,
            "existing wallet runtime input changed"
        );
        return Ok(());
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

/// Cold provisioning runs as a bounded separate process. No master or spend
/// key is loaded into the controller, miner, pool front or Ark role.
pub fn provision(scenario: &Scenario, private: &Path, binary: &Path) -> Result<()> {
    for spec in &scenario.wallet_agents {
        ensure!(
            !private
                .join(format!("identity-{}.json", spec.identity_id))
                .exists(),
            "legacy random identity material is not BIP85 wallet provisioning"
        );
        let online = private.join(format!("wallet-{}", spec.identity_id));
        let identity_spec = scenario
            .identities
            .iter()
            .find(|i| i.id == spec.identity_id)
            .context("wallet identity")?;
        if let crate::scenario::AddressSource::Explicit { address } = &identity_spec.address_source
        {
            let existing = public_identity(private, &spec.identity_id)
                .context("explicit identity needs existing wallet provisioning")?;
            ensure!(
                &existing.address == address,
                "configured wallet identity differs from provisioning"
            );
        }
        let cold = private.join(format!("cold-wallet-{}", spec.identity_id));
        let log = private.join(format!("wallet-provision-{}.log", spec.identity_id));
        let output = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(log)?;
        let pid = std::process::id();
        let mut child = Command::new(std::env::current_exe()?)
            .args([
                "actor-exec",
                "--parent-pid",
                &pid.to_string(),
                "--parent-start-ticks",
                &crate::supervisor::process_start_time(pid)
                    .context("controller identity")?
                    .to_string(),
                "--",
            ])
            .arg(binary)
            .arg("--provision-wallet")
            .arg(&online)
            .arg("--cold-store")
            .arg(&cold)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(output)
            .spawn()?;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    ensure!(status.success(), "cold wallet provisioning failed");
                    break;
                }
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20))
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    bail!("cold wallet provisioning timeout");
                }
            }
        }
    }
    Ok(())
}

pub fn public_identity(private: &Path, id: &IdentityId) -> Result<crate::runtime::PrivateIdentity> {
    let record: Value = serde_json::from_slice(&std::fs::read(
        private.join(format!("wallet-{id}/provision.json")),
    )?)?;
    ensure!(
        record["schema"] == "spc.agent.provision/v1" && record["network"] == "regtest",
        "foreign wallet provisioning"
    );
    let address = record["sp_address"]
        .as_str()
        .context("wallet SP address missing")?;
    let parsed = spc_crypto::parse_sp_address(address, spc_crypto::Network::Regtest)?;
    Ok(crate::runtime::PrivateIdentity {
        id: id.to_string(),
        address: address.into(),
        sp_id: hex::encode(spc_crypto::sp_id(&parsed.b_scan, &parsed.b_spend)),
        b_scan: hex::encode(parsed.b_scan.serialize()),
        b_spend: hex::encode(parsed.b_spend.serialize()),
        scan_key: String::new(),
        spend_key: String::new(),
    })
}

/// All agents are supervised before any initial miner starts. Funding remains
/// the separate T1 startup prerequisite; this hook never activates a miner.
pub fn launch(engine: &RunEngine, binary: &Path) -> Result<()> {
    if engine.scenario.wallet_agents.is_empty() {
        return Ok(());
    }
    let metadata: Value = serde_json::from_slice(&std::fs::read(
        engine.run_dir.join("private/ark-info.json"),
    )?)?;
    ensure!(
        metadata["schema"] == "spc.lab.ark-info/v1",
        "missing actual ArkInfo binding"
    );
    let resources = engine.resources_manifest();
    for agent in &engine.scenario.wallet_agents {
        let private = engine.run_dir.join("private");
        let aggregate_api = aggregate_binding::for_launch(engine, agent)?;
        let dir = private.join(format!("wallet-{}", agent.identity_id));
        let identity = public_identity(&private, &agent.identity_id)?;
        let front = engine
            .scenario
            .fronts
            .iter()
            .find(|f| f.id == agent.front_id)
            .context("wallet front")?;
        let pool = engine
            .scenario
            .pools
            .iter()
            .find(|p| p.id == front.pool_id)
            .context("wallet pool")?;
        let backend = engine
            .scenario
            .ark_backends
            .iter()
            .find(|b| {
                Some(&b.id) == pool.ark_backend_id.as_ref()
                    || (pool.ark_backend_id.is_none() && b.local.is_some())
            })
            .context("wallet backend")?;
        ensure!(
            metadata["backend_id"].as_str() == Some(backend.id.as_str()),
            "ArkInfo backend mismatch"
        );
        let node = &backend
            .local
            .as_ref()
            .context("local Ark backend required")?
            .node_id;
        let rpc = resources
            .listeners
            .get(&format!("node.{node}.rpc"))
            .context("node RPC lease")?;
        let front_endpoint = resources
            .listeners
            .get(&format!("front.{}.sv2", agent.front_id))
            .context("front SV2 lease")?;
        let status = resources
            .listeners
            .get(&format!("wallet.{}.status", agent.identity_id))
            .context("wallet status lease")?;
        let key = |name: &str| -> Result<String> {
            Ok(metadata["bound"][name]
                .as_str()
                .context("ArkInfo bound key missing")?
                .into())
        };
        let mut config = serde_json::json!({"upstream": {"address": front_endpoint.to_string(),
            "authority_public_key": engine.authority_public(&agent.front_id)?, "sp_address": identity.address,
            "request_share_accounting": false}, "ark": {"enabled": true, "mailbox_mode": "epoch_derived",
            "epoch_fallback": "static", "network": "regtest", "ark_id": key("ark_id")?,
            "server_pubkey": key("server_pubkey")?, "mailbox_pubkey": key("mailbox_pubkey")?,
            "server_address": metadata["endpoint"].as_str().context("Ark endpoint missing")?,
            "chain_rpc_url": format!("http://{rpc}"), "chain_cookie": private.join(format!("node-rpc-{node}.cookie")),
            "scan_xprv": dir.join("scan.xprv"), "ark_xprv": dir.join("ark.xprv"), "seed_child": dir.join("child.seed"),
            "datadir": dir.join("state"), "status_listen": status.to_string(), "aggregate_api": aggregate_api}});
        if let Some(native) = native_render::render(engine, agent, &identity.address)? {
            config["ark"]["native_audit"] = native;
        }
        let config_path = dir.join("agent.toml");
        private_file(&config_path, toml::to_string(&config)?.as_bytes())?;
        aggregate_binding::validate_persisted(engine, agent)?;
        let adapter = crate::config::AdapterConfig {
            program: binary.into(),
            args: vec![],
            capabilities: vec![],
            fixture: false,
            effective_profile: None,
            rpc_auth: None,
            env: BTreeMap::new(),
        };
        engine.launch_process(
            "wallet",
            agent.identity_id.as_str(),
            &adapter,
            vec![
                "--config".into(),
                config_path.display().to_string(),
                "--roles".into(),
                "wallet".into(),
            ],
            BTreeMap::new(),
            crate::supervisor::Readiness::Tcp {
                host: "127.0.0.1".into(),
                port: status.port,
            },
            &private.join(format!("wallet-agent-{}.log", agent.identity_id)),
            false,
        )?;
    }
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let connections: Vec<_> = engine
            .scenario
            .miners
            .iter()
            .flat_map(|m| m.connections.iter().cloned())
            .collect();
        if require_ready(engine, &connections).is_ok() {
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "wallet registration startup timeout; miners remain inactive"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

pub fn require_ready(engine: &RunEngine, connections: &[ConnectionSpec]) -> Result<(), String> {
    let resources = engine.resources_manifest();
    for c in connections {
        if let Some(agent) = validate_connection(&engine.scenario, c).map_err(|e| e.to_string())? {
            aggregate_binding::validate_persisted(engine, agent).map_err(|e| e.to_string())?;
            {
                let supervisor = engine.supervisor.lock().map_err(|_| "supervisor lock")?;
                let records = supervisor.records();
                let record = records
                    .iter()
                    .find(|r| r.role == "wallet" && r.entity == agent.identity_id.as_str())
                    .ok_or("wallet actor not launched")?;
                if !supervisor.verify(record).is_owned() {
                    return Err("wallet actor ownership unavailable".into());
                }
            }
            let endpoint = resources
                .listeners
                .get(&format!("wallet.{}.status", agent.identity_id))
                .ok_or("wallet status not leased")?;
            let status = read_status(endpoint.port).map_err(|_| "wallet status unavailable")?;
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| "clock unavailable")?
                .as_millis() as u64;
            if !status_ready(&status, now) {
                return Err(
                    "wallet has no current committed registration and live connection".into(),
                );
            }
        }
    }
    Ok(())
}

pub fn status_ready(status: &Value, now: u64) -> bool {
    status["state"] == "connected"
        && status["registered"] == true
        && status["registration_live_until_unix_ms"]
            .as_u64()
            .is_some_and(|until| until > now && until - now <= 30_000)
}

fn read_status(port: u16) -> Result<Value> {
    let mut stream = TcpStream::connect_timeout(
        &format!("127.0.0.1:{port}").parse()?,
        Duration::from_secs(2),
    )?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    stream.write_all(b"GET /status HTTP/1.0\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;
    let mut bytes = Vec::new();
    stream.take(16_385).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 16_384, "oversized wallet status");
    let text = std::str::from_utf8(&bytes)?;
    ensure!(
        text.starts_with("HTTP/1.0 200 ") || text.starts_with("HTTP/1.1 200 "),
        "wallet status HTTP failure"
    );
    Ok(serde_json::from_str(
        text.split_once("\r\n\r\n")
            .context("wallet HTTP framing")?
            .1,
    )?)
}
