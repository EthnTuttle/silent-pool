//! Locally owned stock Ark infrastructure. A responding RPC is not a funded float.
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use bitcoin::bip32::{DerivationPath, Xpriv, Xpub};
use rand::RngCore;
use secp256k1::{Keypair, PublicKey, Secp256k1, SecretKey};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::config::AdapterConfig;
use crate::engine::RunEngine;
use crate::ids::NodeId;
use crate::runtime::{RuntimeBinaries, RuntimeOptions};
use crate::scenario::{ArkBackendSpec, Scenario};
use crate::supervisor::{hash_file, process_start_time, Readiness};

pub const BARK_REVISION: &str = "c419159fa54085572f8ca72d2ba1716556a9d3eb";

/// Closed operator-only inventory projection. Never copies wallet addresses,
/// private paths, raw RPC bodies or user identity fields into the hub.
pub fn inventory_details(engine: &RunEngine, id: &str) -> Vec<String> {
    let mut details = vec![
        "Locally owned stock captaind; independent provider Postgres and mnemonic.".into(),
        "RPC availability is not funded end-to-end readiness. User wallets are not provisioned in this infrastructure profile.".into(),
    ];
    if let Some(endpoint) = engine
        .resources_manifest()
        .listeners
        .get(&format!("ark.{id}"))
    {
        details.push(format!("Local gRPC endpoint: {endpoint}"));
    }
    let metadata = fs::read(engine.run_dir.join("private/ark-info.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
    if let Some(metadata) =
        metadata.filter(|m| m["schema"] == "spc.lab.ark-info/v1" && m["backend_id"] == id)
    {
        let state = match metadata["state"].as_str() {
            Some("initialized_unfunded") => "initialized_unfunded",
            Some("funding_observed_not_validated") => "funding_observed_not_validated",
            _ => "unknown",
        };
        if let Some(at) = metadata["observed_at"].as_u64() {
            details.push(format!("Last startup observation: {state} at unix_ms={at}; process state above is independent."));
        }
        if let Some(hash) = metadata["bound"]["fees_sha256"]
            .as_str()
            .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            details.push(format!("Startup ArkInfo fee-schedule SHA256: {hash}"));
        }
    } else {
        details.push("ArkInfo startup observation unavailable.".into());
    }
    details
}

#[derive(Debug, Clone)]
pub struct ArkBinaries {
    pub captaind: PathBuf,
    pub grpcurl: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Deployment {
    SelfHosted,
}

/// Required profile values; none silently inherit a hosted provider policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalArkSpec {
    pub deployment: Deployment,
    pub node_id: NodeId,
    pub vtxo_lifetime: u32,
    pub max_vtxo_exit_depth: u16,
    pub required_board_confirmations: u32,
    pub min_board_amount_sats: u64,
    pub round_interval_secs: u32,
    pub round_submit_secs: u32,
    pub round_sign_secs: u32,
}

impl LocalArkSpec {
    pub fn validate(&self, scenario: &Scenario, backend: &ArkBackendSpec) -> Result<()> {
        if backend.implementation != "captaind"
            || !scenario.nodes.iter().any(|n| n.id == self.node_id)
            || !backend.listen.starts_with("127.0.0.1:")
        {
            bail!("self-hosted captaind requires a declared local node and loopback listener");
        }
        if self.vtxo_lifetime != 432
            || self.max_vtxo_exit_depth != 100
            || self.required_board_confirmations != 3
            || self.min_board_amount_sats != 20_000
            || self.round_interval_secs != 10
            || self.round_submit_secs != 2
            || self.round_sign_secs != 2
        {
            bail!("unsupported self-hosted Ark lab policy; use the explicit AP-4 profile");
        }
        Ok(())
    }
}

pub fn validate_runtime(scenario: &Scenario, options: &RuntimeOptions) -> Result<()> {
    if scenario.ark_backends.is_empty() {
        if options.ark.is_some() {
            bail!("Ark tools require a declared self-hosted backend");
        }
        return Ok(());
    }
    if scenario.ark_backends.len() != 1 || options.topology.is_some() {
        bail!("self-hosted Ark runtime requires one host-local backend");
    }
    let backend = &scenario.ark_backends[0];
    backend
        .local
        .as_ref()
        .context("hosted/external Ark backends are not a lab deployment")?
        .validate(scenario, backend)?;
    crate::wallet_bridge::validate(scenario)?;
    if !scenario.miners.is_empty() && options.wallet_agent.is_none() {
        bail!("Ark miners require the explicit registered wallet-agent runtime");
    }
    if std::env::vars_os().any(|(k, _)| k.to_string_lossy().starts_with("BARK_SERVER_")) {
        bail!("BARK_SERVER_ environment overrides conflict with run-bound local configuration");
    }
    let tools = options
        .ark
        .as_ref()
        .context("local Ark requires --captaind and --grpcurl")?;
    for path in [&tools.captaind, &tools.grpcurl] {
        if !fs::canonicalize(path)?.starts_with("/nix/store") || !path.is_file() {
            bail!("Ark tools must be Nix store executables");
        }
    }
    let share = tools.share()?;
    if fs::read_to_string(share.join("source-revision"))?.trim() != BARK_REVISION {
        bail!("captaind source revision differs from UPSTREAM_DEPS.md");
    }
    Ok(())
}

impl ArkBinaries {
    fn share(&self) -> Result<PathBuf> {
        Ok(fs::canonicalize(&self.captaind)?
            .parent()
            .context("captaind bin")?
            .parent()
            .context("captaind package")?
            .join("share/ark"))
    }
}

fn private_dir(path: &Path) -> Result<()> {
    fs::create_dir(path)
        .with_context(|| format!("create fresh owned directory {}", path.display()))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

fn private_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

/// Short provisioning commands use the same safe parent-death exec helper as
/// actors. Output goes to private files (never pipes that can deadlock), and
/// timeout/error always reaps the child. Long-lived services use the supervisor.
fn command(engine: &RunEngine, program: &Path, args: &[String], tag: &str) -> Result<Vec<u8>> {
    let dir = engine.run_dir.join("private/ark-commands");
    if !dir.exists() {
        private_dir(&dir)?;
    }
    let nonce = random_hex(8);
    let stdout = dir.join(format!("{tag}-{nonce}.out"));
    let stderr = dir.join(format!("{tag}-{nonce}.err"));
    let out = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&stdout)?;
    let err = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&stderr)?;
    let pid = std::process::id();
    let mut child = Command::new(std::env::current_exe()?)
        .args([
            "actor-exec",
            "--parent-pid",
            &pid.to_string(),
            "--parent-start-ticks",
            &process_start_time(pid)
                .context("controller identity")?
                .to_string(),
            "--",
        ])
        .arg(program)
        .args(args)
        .env_clear()
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    bail!(
                        "{tag} failed ({status}); private diagnostic {}",
                        stderr.display()
                    );
                }
                return Ok(fs::read(stdout)?);
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            other => {
                let _ = child.kill();
                let _ = child.wait();
                bail!("{tag} timed out or could not be waited: {other:?}");
            }
        }
    }
}

fn adapter(program: &Path) -> AdapterConfig {
    AdapterConfig {
        program: program.into(),
        args: vec![],
        env: BTreeMap::new(),
        capabilities: vec![],
        fixture: false,
        effective_profile: None,
        rpc_auth: None,
    }
}

fn launch(
    engine: &RunEngine,
    role: &str,
    id: &str,
    program: &Path,
    args: Vec<String>,
    readiness: Readiness,
) -> Result<()> {
    engine.launch_process(
        role,
        id,
        &adapter(program),
        args,
        BTreeMap::new(),
        readiness,
        &engine
            .run_dir
            .join("private")
            .join(format!("{role}-{id}.log")),
        false,
    )?;
    Ok(())
}

/// Called after the lab nodes and pool stores are live, before fronts start.
pub fn start(engine: &RunEngine, binaries: &RuntimeBinaries, tools: &ArkBinaries) -> Result<()> {
    let backend = engine
        .scenario()
        .ark_backends
        .first()
        .context("local backend")?;
    let profile = backend.local.as_ref().context("self-hosted profile")?;
    let id = backend.id.as_str();
    let listeners = engine.resources_manifest().listeners;
    let endpoint = &listeners[&format!("ark.{id}")];
    let pg = &listeners[&format!("ark.{id}.postgres")];
    let admin = &listeners[&format!("ark.{id}.admin")];
    let node = &listeners[&format!("node.{}.rpc", profile.node_id)];
    let root = engine
        .run_dir
        .join("private")
        .join(format!("ark-server-{id}"));
    private_dir(&root)?;
    let pgdir = root.join("postgres");
    let socket = root.join("socket");
    private_dir(&socket)?;
    let password = random_hex(32);
    let passfile = root.join("postgres.password");
    private_file(&passfile, password.as_bytes())?;
    command(
        engine,
        &binaries.postgres_tool("initdb"),
        &[
            "--pgdata".into(),
            pgdir.display().to_string(),
            "--username=ark".into(),
            format!("--pwfile={}", passfile.display()),
            "--auth-host=scram-sha-256".into(),
            "--auth-local=trust".into(),
            "--encoding=UTF8".into(),
            "--locale=C".into(),
            "--no-instructions".into(),
        ],
        "ark-initdb",
    )?;
    launch(
        engine,
        "ark-postgres",
        id,
        &binaries.postgres_tool("postgres"),
        vec![
            "-D".into(),
            pgdir.display().to_string(),
            "-p".into(),
            pg.port.to_string(),
            "-h".into(),
            "127.0.0.1".into(),
            "-k".into(),
            socket.display().to_string(),
        ],
        Readiness::Tcp {
            host: pg.host.clone(),
            port: pg.port,
        },
    )?;
    // Stock Server::create creates its own database and migrations.
    let share = tools.share()?;
    let template = fs::read_to_string(share.join("captaind.default.toml"))?;
    let mut cfg: toml::Value = toml::from_str(&template)?;
    cfg["data_dir"] = root.join("data").display().to_string().into();
    cfg["network"] = "regtest".into();
    cfg["vtxo_lifetime"] = i64::from(profile.vtxo_lifetime).into();
    cfg["max_vtxo_exit_depth"] = i64::from(profile.max_vtxo_exit_depth).into();
    cfg["required_board_confirmations"] = i64::from(profile.required_board_confirmations).into();
    cfg["min_board_amount"] = format!("{} sat", profile.min_board_amount_sats).into();
    cfg["round_interval"] = format!("{}s", profile.round_interval_secs).into();
    cfg["round_submit_time"] = format!("{}s", profile.round_submit_secs).into();
    cfg["round_sign_time"] = format!("{}s", profile.round_sign_secs).into();
    cfg["rpc"] = toml::Value::try_from(
        json!({"public_address": endpoint.display(), "admin_address": admin.display()}),
    )?;
    cfg["postgres"] = toml::Value::try_from(json!({"host":"127.0.0.1", "port":pg.port,
        "name":"ark", "user":"ark", "password":password, "max_connections":10}))?;
    cfg["bitcoind"] = toml::Value::try_from(json!({"url":format!("http://{node}"),
        "cookie":engine.run_dir.join("private").join(format!("node-rpc-{}.cookie",profile.node_id))}))?;
    let config = root.join("captaind.toml");
    private_file(&config, toml::to_string(&cfg)?.as_bytes())?;
    command(
        engine,
        &tools.captaind,
        &["check-config".into(), config.display().to_string()],
        "ark-check-config",
    )?;
    command(
        engine,
        &tools.captaind,
        &[
            "--config".into(),
            config.display().to_string(),
            "create".into(),
        ],
        "ark-create",
    )?;
    private_file(
        &root.join("initialized.json"),
        &serde_json::to_vec_pretty(&json!({
            "state":"initialized", "funding":"unproven", "source_revision":BARK_REVISION,
            "captaind":fs::canonicalize(&tools.captaind)?, "binary_sha256":hash_file(&tools.captaind)?,
            "grpcurl":fs::canonicalize(&tools.grpcurl)?, "grpcurl_sha256":hash_file(&tools.grpcurl)?,
            "node_binary":fs::canonicalize(&binaries.node)?, "node_binary_sha256":hash_file(&binaries.node)?,
            "node_profile_binding":"not_validated_by_canonical_probe",
            "config_sha256":hash_file(&config)?, "database_dir":pgdir,
            "pool_databases_shared":false, "observed_at":crate::command::now_ms()
        }))?,
    )?;
    launch(
        engine,
        "ark",
        id,
        &tools.captaind,
        vec![
            "--config".into(),
            config.display().to_string(),
            "start".into(),
        ],
        Readiness::Tcp {
            host: endpoint.host.clone(),
            port: endpoint.port,
        },
    )?;
    let rpc = command(
        engine,
        &tools.grpcurl,
        &[
            "-plaintext".into(),
            "-max-time".into(),
            "10".into(),
            "-emit-defaults".into(),
            "-import-path".into(),
            share.join("protos").display().to_string(),
            "-proto".into(),
            "bark_server.proto".into(),
            endpoint.display(),
            "bark_server.ArkService/GetArkInfo".into(),
        ],
        "ark-info",
    )?;
    let info: Value = serde_json::from_slice(&rpc)?;
    let bound = bind_info(&info, profile)?;
    let wallet = command(
        engine,
        &tools.captaind,
        &[
            "--config".into(),
            config.display().to_string(),
            "rpc".into(),
            "wallet".into(),
        ],
        "ark-wallet",
    )?;
    let wallet: Value = serde_json::from_slice(&wallet)?;
    let state = match wallet["rounds"]["total_balance"].as_u64() {
        Some(0) => "initialized_unfunded",
        Some(_) => "funding_observed_not_validated",
        None => bail!("stock server wallet returned no total_balance"),
    };
    // Wallet addresses/balances are the native funder's input, not a fabricated
    // funding receipt. This file is private and never a user inventory payload.
    private_file(
        &root.join("native-funding-handoff.json"),
        &serde_json::to_vec_pretty(&json!({
            "schema":"spc.lab.ark.native-funding/v1", "server_wallet":wallet,
            "pool_float_target_sats":115_000_000u64, "funded_e2e":false,
            "required_next":"native T1b/T2/DEST funding, pool boarding, AP-3b receive/refresh/offboard"
        }))?,
    )?;
    private_file(
        &engine.run_dir.join("private/ark-info.json"),
        &serde_json::to_vec_pretty(&json!({
            "schema":"spc.lab.ark-info/v1", "backend_id":id, "endpoint":format!("http://{endpoint}"),
            "source_revision":BARK_REVISION, "state":state,
            "observed_at":crate::command::now_ms(), "bound":bound, "ark_info":info
        }))?,
    )?;
    provision_pool_roles(engine, binaries, &format!("http://{endpoint}"), &bound)?;
    Ok(())
}

/// Decode the protobuf JSON representation of a compressed point. Its length
/// is exactly 33 bytes, hence exactly 44 unpadded base64 characters.
fn proto_point(value: &Value) -> Result<PublicKey> {
    let text = value.as_str().context("ArkInfo point missing")?;
    if text.len() != 44 {
        bail!("ArkInfo compressed point length");
    }
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut bytes = Vec::with_capacity(33);
    for chunk in text.as_bytes().chunks_exact(4) {
        let mut word = 0u32;
        for c in chunk {
            word = (word << 6)
                | u32::try_from(
                    alphabet
                        .iter()
                        .position(|v| v == c)
                        .context("ArkInfo point base64")?,
                )?;
        }
        bytes.extend_from_slice(&word.to_be_bytes()[1..]);
    }
    Ok(PublicKey::from_slice(&bytes)?)
}

pub fn bind_info(info: &Value, profile: &LocalArkSpec) -> Result<Value> {
    if info["network"] != "regtest" {
        bail!("ArkInfo is not regtest");
    }
    for (name, expected) in [
        ("vtxoLifetime", u64::from(profile.vtxo_lifetime)),
        ("maxVtxoExitDepth", u64::from(profile.max_vtxo_exit_depth)),
        (
            "requiredBoardConfirmations",
            u64::from(profile.required_board_confirmations),
        ),
        ("roundIntervalSecs", u64::from(profile.round_interval_secs)),
    ] {
        if info[name].as_u64() != Some(expected) {
            bail!("ArkInfo policy mismatch: {name}");
        }
    }
    if info["minBoardAmount"].as_str() != Some(&profile.min_board_amount_sats.to_string()) {
        bail!("ArkInfo minimum board mismatch");
    }
    let server = proto_point(&info["serverPubkey"])?;
    let mailbox = proto_point(&info["mailboxPubkey"])?;
    let fees = info
        .get("fees")
        .filter(|f| f.is_object())
        .context("ArkInfo fee schedule missing")?;
    Ok(
        json!({"ark_id":hex::encode(&Sha256::digest(server.serialize())[..4]),
        "server_pubkey":server.to_string(), "mailbox_pubkey":mailbox.to_string(),
        "fees_sha256":hex::encode(Sha256::digest(serde_json::to_vec(fees)?))}),
    )
}

fn random_hex(n: usize) -> String {
    let mut bytes = vec![0; n];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn provision_pool_roles(
    engine: &RunEngine,
    binaries: &RuntimeBinaries,
    endpoint: &str,
    info: &Value,
) -> Result<()> {
    for pool in &engine.scenario().pools {
        let root = engine
            .run_dir
            .join("private")
            .join(format!("pool-ark-{}", pool.id));
        private_dir(&root)?;
        private_dir(&root.join("wallet"))?;
        private_file(&root.join("wallet/wallet.sqlite"), &[])?;
        let seed_hex = random_hex(64);
        let seed = hex::decode(&seed_hex)?;
        private_file(&root.join("seed.hex"), seed_hex.as_bytes())?;
        let secp = Secp256k1::new();
        let account = Xpriv::new_master(bitcoin::Network::Regtest, &seed)?
            .derive_priv(&secp, &"m/86'/1'/1'".parse::<DerivationPath>()?)?;
        let descriptor = format!("tr({}/0/*)", Xpub::from_priv(&secp, &account));
        let order_key = SecretKey::new(&mut rand::thread_rng());
        let order_pub = Keypair::from_secret_key(&secp, &order_key)
            .x_only_public_key()
            .0;
        let order_path = engine
            .run_dir
            .join("private")
            .join(format!("order-{}.hex", pool.id));
        private_file(
            &order_path,
            hex::encode(order_key.secret_bytes()).as_bytes(),
        )?;
        let mut ark = json!({"enabled":true,"transport":"bark","server":endpoint,
            "server_is_self":true,"ark_id":info["ark_id"],"server_pubkey":info["server_pubkey"],
            "mailbox_pubkey":info["mailbox_pubkey"],"float_target_sats":115_000_000u64,
            "max_order_sats":500_000,"max_daily_sats":0,"min_remaining_blocks":64,"retry_blocks":6,
            "order_pubkey":order_pub.to_string(),"order_key_path":order_path,
            "seed_path":root.join("seed.hex"),"datadir":root.join("wallet"),"pool_output_account":"m/86'/1'/1'"});
        let front = engine
            .scenario()
            .fronts
            .iter()
            .find(|f| f.pool_id == pool.id)
            .context("pool front missing")?;
        let front_path = engine
            .run_dir
            .join("private")
            .join(format!("pool-{}.toml", front.id));
        let mut front_cfg: toml::Value = toml::from_str(&fs::read_to_string(&front_path)?)?;
        let node_cfg = front_cfg["template"].clone();
        let resources = engine.resources_manifest();
        let redis = resources
            .redis_instances
            .iter()
            .find(|r| r.id == pool.storage.redis_instance_id)
            .context("pool Redis missing")?;
        let acl_password = random_hex(32);
        // Set the ACL through a private redis-cli command file would still
        // expose credentials in argv. Use its stdin command mode instead via
        // the existing node-free HTTP-independent RESP exchange below.
        install_ark_acl(&redis.url, &acl_password)?;
        let role_redis = format!("redis://ark:{acl_password}@127.0.0.1:{}/0", redis.port);
        for front in engine
            .scenario()
            .fronts
            .iter()
            .filter(|f| f.pool_id == pool.id)
        {
            let path = engine
                .run_dir
                .join("private")
                .join(format!("pool-{}.toml", front.id));
            front_cfg = toml::from_str(&fs::read_to_string(&path)?)?;
            front_cfg["pool"]["pool_payout_descriptor"] = descriptor.clone().into();
            front_cfg["pool"]
                .as_table_mut()
                .context("pool section")?
                .remove("pool_payout_script");
            let mut public_ark = ark.clone();
            public_ark.as_object_mut().unwrap().remove("seed_path");
            public_ark.as_object_mut().unwrap().remove("datadir");
            front_cfg
                .as_table_mut()
                .context("front config table")?
                .insert("ark".into(), toml::Value::try_from(public_ark)?);
            // Already-private runtime config; atomic replacement prevents a
            // future concurrent reader observing half of the new metadata.
            let pending = path.with_extension("ark-pending");
            private_file(&pending, toml::to_string(&front_cfg)?.as_bytes())?;
            fs::rename(pending, path)?;
        }
        ark.as_object_mut().unwrap().remove("order_key_path");
        let role = pool_role_config(&descriptor, node_cfg, &role_redis, ark)?;
        let role_path = root.join("pool.toml");
        private_file(&role_path, toml::to_string(&role)?.as_bytes())?;
        private_file(
            &root.join("provision.json"),
            &serde_json::to_vec_pretty(&json!({
                "schema":"spc.lab.pool-ark-provision/v1", "pool_id":pool.id,"server":endpoint,
                "pool_payout_descriptor":descriptor,"float_target_sats":115_000_000u64,
                "wallet_state":"not_yet_observed", "database_access":false,
                "config_sha256":hash_file(&role_path)?,"funded_e2e":false
            }))?,
        )?;
        launch(
            engine,
            "pool-ark",
            pool.id.as_str(),
            &binaries.pool,
            vec!["--config".into(), role_path.display().to_string()],
            Readiness::File {
                path: root.join("wallet/provision.json"),
            },
        )?;
    }
    Ok(())
}

/// The shared pool parser requires a listen string and public-authority field
/// even without a front. Port zero and an empty public pin are unused by the
/// ark-only role; no listener or Noise secret is provisioned for it.
pub fn pool_role_config(
    descriptor: &str,
    node: toml::Value,
    redis: &str,
    ark: Value,
) -> Result<toml::Value> {
    Ok(toml::Value::try_from(json!({
        "pool":{"network":"regtest", "roles":["ark"], "listen":"127.0.0.1:0",
            "authority_public_key":"", "pool_payout_descriptor":descriptor},
        "template":node, "sv1":{"enabled":false}, "storage":{"redis":redis}, "ark":ark
    }))?)
}

/// Minimal RESP commands for local provisioning, with credentials off argv.
/// The role can access only identity-free Ark operational keys, not arkreg.
fn install_ark_acl(url: &str, password: &str) -> Result<()> {
    use std::io::{BufRead, BufReader};
    use std::net::TcpStream;
    let rest = url
        .strip_prefix("redis://:")
        .context("private runtime Redis URL")?;
    let (auth, host) = rest
        .split_once('@')
        .context("private Redis authentication")?;
    let addr = host.strip_suffix("/0").context("private Redis db")?;
    let addr: std::net::SocketAddr = addr.parse().context("local Redis address")?;
    if !addr.ip().is_loopback() {
        bail!("Ark role Redis must be local");
    }
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    for words in [vec!["AUTH".to_string(),auth.to_string()], vec![
        "ACL".into(),"SETUSER".into(),"ark".into(),"reset".into(),"on".into(),format!(">{password}"),
        "+ping".into(),"+client|setinfo".into(),"+select".into(),
        "(+xgroup +xreadgroup +xack +xautoclaim ~spc:ark:orders)".into(),
        "(+xadd ~spc:ark:receipts)".into(),
        "(+xadd ~spc:ark:authorization:requests)".into(),
        "(+hget ~spc:ark:authorization:grants)".into(),
        "(+get +set ~spc:ark:pending-cursor:*)".into(),
        "(+hget +hset +hgetall +hdel +get +set +sadd +srem +smembers +sismember +expire ~spc:ark:seen ~spc:ark:ambiguous ~spc:ark:board:*)".into()]] {
        write!(stream,"*{}\r\n",words.len())?;
        for word in words { write!(stream,"${}\r\n{}\r\n",word.len(),word)?; }
        stream.flush()?;
        let mut response=String::new();
        BufReader::new(&mut stream).read_line(&mut response)?;
        if response != "+OK\r\n" { bail!("local Ark Redis ACL provisioning rejected"); }
    }
    Ok(())
}

#[cfg(test)]
#[path = "ark_acl_tests.rs"]
mod acl_tests;
