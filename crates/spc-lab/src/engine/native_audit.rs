//! Controller-derived native audit leases. No browser-selected socket/key,
//! wallet provisioning, signer, or duplicate wallet service is introduced.
use super::*;
use sha2::{Digest, Sha256};
use spc_scan::native_activation::{
    self as wire, PeerBinding, PeerRegistry, ProcessIdentity, WalletInstance,
};
use std::io::{Read, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt};

static PUBLICATION: Mutex<()> = Mutex::new(());

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Lease {
    instance: WalletInstance,
    registry_path: PathBuf,
    registry_sha256: [u8; 32],
}
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Leases {
    schema: String,
    run_id: String,
    leases: BTreeMap<String, Lease>,
}

fn failure(reason: impl Into<String>) -> EngineError {
    EngineError::new("native_audit_unavailable", reason)
}
fn hash(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
fn file_hash(path: &Path) -> Result<[u8; 32], EngineError> {
    let mut file = std::fs::File::open(path).map_err(|e| failure(e.to_string()))?;
    let mut digest = Sha256::new();
    let mut buf = [0; 65536];
    loop {
        let n = file.read(&mut buf).map_err(|e| failure(e.to_string()))?;
        if n == 0 {
            break;
        }
        digest.update(&buf[..n]);
    }
    Ok(digest.finalize().into())
}
fn private_dir(path: &Path, uid: u32) -> Result<(), EngineError> {
    let m = std::fs::symlink_metadata(path).map_err(|e| failure(e.to_string()))?;
    if !m.is_dir() || m.mode() & 0o777 != 0o700 || m.uid() != uid {
        return Err(failure("unsafe native audit directory"));
    }
    Ok(())
}
fn private_read(path: &Path, uid: u32) -> Result<Vec<u8>, EngineError> {
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags((rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32)
        .open(path)
        .map_err(|e| failure(e.to_string()))?;
    let m = file.metadata().map_err(|e| failure(e.to_string()))?;
    if !m.is_file()
        || m.mode() & 0o777 != 0o600
        || m.uid() != uid
        || m.nlink() != 1
        || m.len() > wire::MAX_REQUEST_BYTES as u64
    {
        return Err(failure("unsafe native audit input"));
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(wire::MAX_REQUEST_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| failure(e.to_string()))?;
    let after = file.metadata().map_err(|e| failure(e.to_string()))?;
    if bytes.len() > wire::MAX_REQUEST_BYTES
        || bytes.len() as u64 != m.len()
        || after.len() != m.len()
        || after.mtime() != m.mtime()
        || after.mtime_nsec() != m.mtime_nsec()
        || after.ctime() != m.ctime()
        || after.ctime_nsec() != m.ctime_nsec()
    {
        return Err(failure("native audit input changed during read"));
    }
    Ok(bytes)
}
fn atomic_private<T: serde::Serialize>(
    path: &Path,
    value: &T,
    uid: u32,
) -> Result<Vec<u8>, EngineError> {
    let parent = path
        .parent()
        .ok_or_else(|| failure("native audit parent missing"))?;
    private_dir(parent, uid)?;
    if path.symlink_metadata().is_ok() {
        private_read(path, uid)?;
    }
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| failure(e.to_string()))?;
    if bytes.len() > wire::MAX_REQUEST_BYTES {
        return Err(failure("native audit projection bound"));
    }
    let temp = parent.join(format!(
        ".native-audit-{}",
        hex::encode(rand::random::<[u8; 16]>())
    ));
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&temp)
        .map_err(|e| failure(e.to_string()))?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| failure(e.to_string()))?;
    std::fs::rename(&temp, path).map_err(|e| failure(e.to_string()))?;
    std::fs::File::open(parent)
        .and_then(|f| f.sync_all())
        .map_err(|e| failure(e.to_string()))?;
    Ok(bytes)
}

impl RunEngine {
    fn validate_native_evidence(
        &self,
        miner: &str,
        process: &ProcessIdentity,
        config_path: &Path,
        advertised: Option<&Value>,
    ) -> Result<(), EngineError> {
        use spc_scan::native_evidence as evidence;
        let raw = private_read(config_path, process.uid)?;
        if hash(&raw) != process.config_sha256 {
            return Err(failure("native evidence config changed"));
        }
        let config: toml::Value =
            toml::from_str(std::str::from_utf8(&raw).map_err(|_| failure("native config UTF8"))?)
                .map_err(|e| failure(e.to_string()))?;
        let capture: evidence::Config = config
            .get("native_evidence")
            .cloned()
            .ok_or_else(|| failure("durable native evidence capability required"))?
            .try_into()
            .map_err(|e: toml::de::Error| failure(e.to_string()))?;
        capture.validate(miner).map_err(failure)?;
        let run = self
            .run_dir
            .canonicalize()
            .map_err(|e| failure(e.to_string()))?;
        if capture.run_id != self.run_id.as_str()
            || capture.run_manifest != run.join("run-manifest.json")
            || capture.directory != run.join("private/native-evidence").join(miner)
        {
            return Err(failure("native evidence path/run binding"));
        }
        private_dir(&capture.directory, process.uid)?;
        let session = capture.directory.join(evidence::session_name(process));
        private_dir(&session, process.uid)?;
        let manifest_bytes = private_read(&session.join("manifest.json"), process.uid)?;
        let manifest = evidence::decode_manifest(&manifest_bytes).map_err(failure)?;
        if manifest.config != capture
            || manifest.process != *process
            || manifest.miner_id != miner
            || manifest.config_path != config_path
        {
            return Err(failure("native durable evidence process binding"));
        }
        if let Some(advertised) = advertised {
            let expected = serde_json::json!({"schema":evidence::LOCATOR_SCHEMA,"capability":evidence::CAPABILITY,
                "directory":capture.directory,"manifest_relative_path":format!("{}/manifest.json",evidence::session_name(process)),
                "manifest_sha256":evidence::sha256_hex(&manifest_bytes)});
            if advertised != &expected {
                return Err(failure("native evidence capability mismatch"));
            }
        }
        Ok(())
    }
    fn native_process(
        &self,
        role: &str,
        id: &str,
    ) -> Result<(ProcessIdentity, PathBuf), EngineError> {
        let record = self
            .manifest
            .lock()
            .unwrap()
            .process(role, id)
            .cloned()
            .ok_or_else(|| failure("native peer not supervised"))?;
        if !self.supervisor.lock().unwrap().verify(&record).is_owned() {
            return Err(failure("native peer ownership mismatch"));
        }
        if role == "wallet" {
            let agent = self
                .scenario
                .wallet_agents
                .iter()
                .find(|agent| agent.identity_id.as_str() == id)
                .ok_or_else(|| failure("native wallet profile missing"))?;
            // Includes persisted expected hash vs recorded supervisor marker.
            // Shared by bind and live attach; never rewrites config on adopt.
            crate::wallet_bridge::validate_native_run_binding(self, agent)
                .map_err(|e| failure(e.to_string()))?;
        }
        let paths: Vec<_> = record
            .args
            .windows(2)
            .filter(|w| w[0] == "--config")
            .map(|w| PathBuf::from(&w[1]))
            .collect();
        if paths.len() != 1 {
            return Err(failure("native peer config ambiguous"));
        }
        let path = paths[0].clone();
        let private = self
            .run_dir
            .join("private")
            .canonicalize()
            .map_err(|e| failure(e.to_string()))?;
        if !path.starts_with(&private)
            || path.canonicalize().map_err(|e| failure(e.to_string()))? != path
        {
            return Err(failure("native peer config not confined"));
        }
        let proc = PathBuf::from(format!("/proc/{}", record.pid));
        let uid = std::fs::metadata(&proc)
            .map_err(|e| failure(e.to_string()))?
            .uid();
        private_dir(
            path.parent()
                .ok_or_else(|| failure("native config parent"))?,
            uid,
        )?;
        let bytes = private_read(&path, uid)?;
        let cmdline = std::fs::read(proc.join("cmdline")).map_err(|e| failure(e.to_string()))?;
        let actual: Vec<_> = cmdline
            .split(|b| *b == 0)
            .filter(|b| !b.is_empty())
            .collect();
        let actual_configs: Vec<_> = actual.windows(2).filter(|w| w[0] == b"--config").collect();
        if actual_configs.len() != 1 || actual_configs[0][1] != path.as_os_str().as_encoded_bytes()
        {
            return Err(failure("native process argv/config mismatch"));
        }
        let env = std::fs::read(proc.join("environ")).map_err(|e| failure(e.to_string()))?;
        let tokens: Vec<_> = env
            .split(|b| *b == 0)
            .filter_map(|e| e.strip_prefix(b"SPC_LAB_RUN_TOKEN="))
            .collect();
        if tokens.len() != 1 || tokens[0] != record.run_token.as_bytes() {
            return Err(failure("native process run token mismatch"));
        }
        let executable = file_hash(&proc.join("exe"))?;
        if hex::encode(executable) != record.executable_sha256 {
            return Err(failure("native executable mismatch"));
        }
        let process = ProcessIdentity {
            pid: record.pid,
            uid,
            start_time_ticks: record.start_time_ticks,
            boot_id: std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
                .map_err(|e| failure(e.to_string()))?
                .trim()
                .into(),
            executable_sha256: executable,
            run_token_sha256: hash(tokens[0]),
            config_sha256: hash(&bytes),
        };
        process.validate().map_err(failure)?;
        if !self.supervisor.lock().unwrap().verify(&record).is_owned() {
            return Err(failure("native process changed during capture"));
        }
        Ok((process, path))
    }

    /// Called on the actual AddConnection send path, after CPU spawn and
    /// before any wire write. The connection came from the typed controller
    /// command/scenario, not from an arbitrary wallet path in an HTTP body.
    pub(super) fn bind_native_audit(
        &self,
        miner: &MinerId,
        payload: &mut Value,
        advertised: Option<&Value>,
    ) -> Result<(), EngineError> {
        let process = self
            .manifest
            .lock()
            .unwrap()
            .process("miner", miner.as_str())
            .cloned()
            .ok_or_else(|| failure("miner process missing"))?;
        let profiles: Vec<_> = self
            .config
            .miners
            .values()
            .filter(|a| a.program == process.executable_path)
            .collect();
        let unaudited = !profiles.is_empty()
            && profiles.iter().all(|a| {
                a.fixture
                    || a.capabilities
                        .iter()
                        .any(|c| c == "spc.miner.native.unaudited")
            });
        let connection = payload
            .get_mut("connection")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| failure("native connection shape"))?;
        if unaudited {
            connection.insert(
                "source_audit".into(),
                serde_json::json!({"mode":"unaudited"}),
            );
            return Ok(());
        }
        if !self.namespaces.is_empty() {
            return Err(failure(
                "native audit currently requires host-local wallet/CPU",
            ));
        }
        let id = connection
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| failure("native connection id"))?
            .to_owned();
        let identity = connection
            .get("identity")
            .and_then(Value::as_str)
            .ok_or_else(|| failure("native connection identity"))?
            .to_owned();
        let matches: Vec<_> = self
            .scenario
            .identities
            .iter()
            .filter(|i| self.identity_address(&i.id).is_ok_and(|v| v == identity))
            .collect();
        if matches.len() != 1 {
            return Err(failure("native wallet identity mapping ambiguous"));
        }
        let identity_id = matches[0].id.to_string();
        let fronts: Vec<_> = self
            .scenario
            .fronts
            .iter()
            .filter(|f| {
                self.front_endpoint(&f.id).is_ok_and(|v| {
                    Some(v.as_str()) == connection.get("endpoint").and_then(Value::as_str)
                }) && self.authority_public(&f.id).is_ok_and(|v| {
                    Some(v.as_str())
                        == connection
                            .get("authority_public_key")
                            .and_then(Value::as_str)
                })
            })
            .collect();
        if fronts.len() != 1 {
            return Err(failure("native front binding ambiguous"));
        }
        let front = fronts[0];
        // Reflect only this named field so the focused helper compiles against
        // the frozen base and joins the current AP3-owned Scenario additively.
        let scenario = serde_json::to_value(&self.scenario).map_err(|e| failure(e.to_string()))?;
        let wallets = scenario
            .get("wallet_agents")
            .and_then(Value::as_array)
            .ok_or_else(|| failure("declared wallet service required"))?;
        if wallets
            .iter()
            .filter(|w| {
                w["identity_id"].as_str() == Some(identity_id.as_str())
                    && w["front_id"].as_str() == Some(front.id.as_str())
            })
            .count()
            != 1
        {
            return Err(failure("native wallet/front declaration missing"));
        }
        let (wallet_process, wallet_config) = self.native_process("wallet", &identity_id)?;
        let text = private_read(&wallet_config, wallet_process.uid)?;
        let config: toml::Value =
            toml::from_str(std::str::from_utf8(&text).map_err(|_| failure("wallet config UTF8"))?)
                .map_err(|e| failure(e.to_string()))?;
        let upstream = config
            .get("upstream")
            .and_then(toml::Value::as_table)
            .ok_or_else(|| failure("wallet upstream config"))?;
        if upstream.get("sp_address").and_then(toml::Value::as_str) != Some(identity.as_str())
            || upstream.get("address").and_then(toml::Value::as_str)
                != connection.get("endpoint").and_then(Value::as_str)
            || upstream
                .get("authority_public_key")
                .and_then(toml::Value::as_str)
                != connection
                    .get("authority_public_key")
                    .and_then(Value::as_str)
        {
            return Err(failure(
                "wallet/native public identity or endpoint mismatch",
            ));
        }
        let ark = config
            .get("ark")
            .and_then(toml::Value::as_table)
            .ok_or_else(|| failure("wallet ark config"))?;
        let audit = ark
            .get("native_audit")
            .and_then(toml::Value::as_table)
            .ok_or_else(|| failure("wallet native audit service unconfigured"))?;
        let datadir = PathBuf::from(
            ark.get("datadir")
                .and_then(toml::Value::as_str)
                .ok_or_else(|| failure("wallet datadir"))?,
        );
        let private = self
            .run_dir
            .join("private")
            .canonicalize()
            .map_err(|e| failure(e.to_string()))?;
        if !datadir.starts_with(&private)
            || datadir.canonicalize().map_err(|e| failure(e.to_string()))? != datadir
        {
            return Err(failure("wallet datadir unconfined"));
        }
        private_dir(&datadir, wallet_process.uid)?;
        let instance = wire::decode_instance(&private_read(
            &datadir.join(wire::INSTANCE_FILENAME),
            wallet_process.uid,
        )?)
        .map_err(failure)?;
        if instance.run_id != self.run_id.as_str()
            || instance.identity_id != identity_id
            || instance.identity != identity
            || instance.pool_id != front.pool_id.as_str()
            || instance.front_id != front.id.as_str()
            || instance.process != wallet_process
            || Path::new(&instance.config_path) != wallet_config
            || Path::new(&instance.socket_path) != datadir.join(wire::SOCKET_FILENAME)
            || audit.get("collector_pubkey").and_then(toml::Value::as_str)
                != Some(instance.collector_pubkey.as_str())
            || audit.get("pool_pubkey").and_then(toml::Value::as_str)
                != Some(instance.pool_pubkey.as_str())
        {
            return Err(failure("wallet native instance/config binding mismatch"));
        }
        let socket =
            std::fs::symlink_metadata(&instance.socket_path).map_err(|e| failure(e.to_string()))?;
        if !socket.file_type().is_socket()
            || socket.uid() != wallet_process.uid
            || socket.mode() & 0o777 != 0o600
            || socket.dev() != instance.socket_device
            || socket.ino() != instance.socket_inode
        {
            return Err(failure("wallet native socket identity mismatch"));
        }
        let (peer, peer_config) = self.native_process("miner", miner.as_str())?;
        let advertised =
            advertised.ok_or_else(|| failure("native durable evidence capability absent"))?;
        self.validate_native_evidence(miner.as_str(), &peer, &peer_config, Some(advertised))?;
        if peer.uid != wallet_process.uid || peer.boot_id != wallet_process.boot_id {
            return Err(failure("native host/UID binding mismatch"));
        }
        let _publication = PUBLICATION
            .lock()
            .map_err(|_| failure("native publication poisoned"))?;
        let leases_path = private.join("native-audit-leases.json");
        let mut leases: Leases = if leases_path.exists() {
            wire::decode_private_json(&private_read(&leases_path, peer.uid)?).map_err(failure)?
        } else {
            Leases {
                schema: "spc.native-audit-leases/1".into(),
                run_id: self.run_id.to_string(),
                leases: BTreeMap::new(),
            }
        };
        if leases.schema != "spc.native-audit-leases/1" || leases.run_id != self.run_id.as_str() {
            return Err(failure("native lease projection foreign"));
        }
        let registry_path = datadir.join(wire::REGISTRY_FILENAME);
        let lease_key = format!("wallet.{identity_id}.native_audit");
        if let Some(previous) = leases.leases.get(&lease_key) {
            let old = private_read(&registry_path, peer.uid)?;
            if previous.registry_path != registry_path || hash(&old) != previous.registry_sha256 {
                return Err(failure("native registry history unavailable"));
            }
        } else if registry_path.exists() {
            return Err(failure("native registry publication incomplete"));
        }
        let mut registry = if registry_path.exists() {
            wire::decode_registry(&private_read(&registry_path, peer.uid)?).map_err(failure)?
        } else {
            PeerRegistry {
                schema: wire::REGISTRY_SCHEMA.into(),
                run_id: self.run_id.to_string(),
                wallet_instance: instance.instance_id,
                generation: 0,
                peers: vec![],
            }
        };
        if registry.schema != wire::REGISTRY_SCHEMA || registry.run_id != self.run_id.as_str() {
            return Err(failure("native peer registry foreign"));
        }
        if registry.wallet_instance != instance.instance_id {
            registry.wallet_instance = instance.instance_id;
            registry.peers.clear();
        }
        registry.generation = registry
            .generation
            .checked_add(1)
            .ok_or_else(|| failure("native peer generation exhausted"))?;
        connection.insert("source_audit".into(),serde_json::json!({"mode":"required","run_id":self.run_id,"miner_id":miner,
            "wallet_socket":instance.socket_path,"wallet_instance":instance.instance_id,"wallet_process":instance.process,
            "wallet_socket_device":instance.socket_device,"wallet_socket_inode":instance.socket_inode,
            "peer_generation":registry.generation,"connection_config_digest":vec![0u8;32],
            "collector_pubkey":instance.collector_pubkey,"pool_pubkey":instance.pool_pubkey}));
        let digest =
            wire::connection_config_digest(&Value::Object(connection.clone())).map_err(failure)?;
        connection.get_mut("source_audit").unwrap()["connection_config_digest"] =
            serde_json::json!(digest);
        registry
            .peers
            .retain(|p| p.miner_id != miner.as_str() || p.connection_id != id);
        registry.peers.push(PeerBinding {
            miner_id: miner.to_string(),
            connection_id: id,
            identity,
            pool_id: front.pool_id.to_string(),
            front_id: front.id.to_string(),
            generation: registry.generation,
            process: peer.clone(),
            config_path: peer_config.to_string_lossy().into_owned(),
            connection_config_digest: digest,
        });
        if registry.peers.len() > 256 {
            return Err(failure("native peer registry capacity"));
        }
        let bytes = atomic_private(&registry_path, &registry, peer.uid)?;
        leases.leases.insert(
            format!("wallet.{identity_id}.native_audit"),
            Lease {
                instance,
                registry_path,
                registry_sha256: hash(&bytes),
            },
        );
        atomic_private(&leases_path, &leases, peer.uid)?;
        Ok(())
    }

    /// Attach/adoption validation only: never creates a wallet/registry or
    /// advances generations. A changed socket or process requires explicit
    /// lifecycle/rebind, not a synthesized allowance.
    pub(super) fn validate_native_audit_attach(&self) -> Result<(), EngineError> {
        let path = self.run_dir.join("private/native-audit-leases.json");
        if !path.exists() {
            let scenario =
                serde_json::to_value(&self.scenario).map_err(|e| failure(e.to_string()))?;
            if scenario
                .get("wallet_agents")
                .and_then(Value::as_array)
                .is_some_and(|a| !a.is_empty())
            {
                return Err(failure("native audit lease projection missing on adopt"));
            }
            return Ok(());
        }
        let uid = std::fs::metadata("/proc/self")
            .map_err(|e| failure(e.to_string()))?
            .uid();
        let leases: Leases =
            wire::decode_private_json(&private_read(&path, uid)?).map_err(failure)?;
        if leases.run_id != self.run_id.as_str() || leases.schema != "spc.native-audit-leases/1" {
            return Err(failure("native lease projection foreign"));
        }
        for lease in leases.leases.values() {
            let (wallet, config) = self.native_process("wallet", &lease.instance.identity_id)?;
            if wallet != lease.instance.process || config != Path::new(&lease.instance.config_path)
            {
                return Err(failure("native wallet replaced"));
            }
            let current = wire::decode_instance(&private_read(
                &Path::new(&lease.instance.socket_path)
                    .parent()
                    .ok_or_else(|| failure("native socket parent"))?
                    .join(wire::INSTANCE_FILENAME),
                uid,
            )?)
            .map_err(failure)?;
            if current != lease.instance {
                return Err(failure("native wallet instance changed"));
            }
            let socket = std::fs::symlink_metadata(&current.socket_path)
                .map_err(|e| failure(e.to_string()))?;
            if !socket.file_type().is_socket()
                || socket.uid() != uid
                || socket.mode() & 0o777 != 0o600
                || socket.dev() != current.socket_device
                || socket.ino() != current.socket_inode
            {
                return Err(failure("native socket replaced"));
            }
            let bytes = private_read(&lease.registry_path, uid)?;
            if hash(&bytes) != lease.registry_sha256 {
                return Err(failure("native registry projection mismatch"));
            }
            let registry = wire::decode_registry(&bytes).map_err(failure)?;
            if registry.wallet_instance != current.instance_id
                || registry.run_id != self.run_id.as_str()
            {
                return Err(failure("native registry foreign"));
            }
            for peer in &registry.peers {
                let (actual, path) = self.native_process("miner", &peer.miner_id)?;
                self.validate_native_evidence(&peer.miner_id, &actual, &path, None)?;
                if actual != peer.process || path != Path::new(&peer.config_path) {
                    return Err(failure("native CPU replaced"));
                }
            }
        }
        Ok(())
    }

    /// Cleanup-purpose validation of the persisted native-audit bindings.
    /// The live adopt path ([`Self::validate_native_audit_attach`]) demands
    /// a live wallet socket, instance file and ownership proof; cleanup
    /// must not. A wallet that failed before any miner bound never
    /// publishes a lease projection, and that absence on a never-started
    /// wallet is expected (FAILED/INCOMPLETE provisioning), not a
    /// contradiction that blocks resource cleanup. A lease projection that
    /// IS present is verified exactly as persisted metadata: the recorded
    /// process identity and config path must still match the persisted
    /// supervisor records and the recorded config bytes, and the recorded
    /// registry must still hash and decode to the same run and wallet
    /// instance - but no live process, socket, instance or ownership proof
    /// is required, because cleanup only reports and signals processes the
    /// recorded census actually identifies as ours. Nothing here creates a
    /// wallet, lease, capability or command, and no key, signer or hash
    /// authority is exercised.
    pub(super) fn validate_persisted_native_audit_bindings(&self) -> Result<(), EngineError> {
        let path = self.run_dir.join("private/native-audit-leases.json");
        if !path.exists() {
            // Expected-not-launched: the wallet never reached the bind that
            // publishes a lease. Never fabricated, never a failure.
            return Ok(());
        }
        let uid = std::fs::metadata("/proc/self")
            .map_err(|e| failure(e.to_string()))?
            .uid();
        let leases: Leases =
            wire::decode_private_json(&private_read(&path, uid)?).map_err(failure)?;
        if leases.run_id != self.run_id.as_str() || leases.schema != "spc.native-audit-leases/1" {
            return Err(failure("native lease projection foreign"));
        }
        for lease in leases.leases.values() {
            lease.instance.validate().map_err(failure)?;
            lease.instance.process.validate().map_err(failure)?;
            let (record, config) =
                self.persisted_native_record("wallet", &lease.instance.identity_id)?;
            if lease.instance.process.pid != record.pid
                || lease.instance.process.start_time_ticks != record.start_time_ticks
                || hex::encode(lease.instance.process.executable_sha256) != record.executable_sha256
                || hash(record.run_token.as_bytes()) != lease.instance.process.run_token_sha256
                || config != Path::new(&lease.instance.config_path)
                || hash(&private_read(&config, lease.instance.process.uid)?)
                    != lease.instance.process.config_sha256
            {
                return Err(failure("native wallet record replaced"));
            }
            let bytes = private_read(&lease.registry_path, uid)?;
            if hash(&bytes) != lease.registry_sha256 {
                return Err(failure("native registry projection mismatch"));
            }
            let registry = wire::decode_registry(&bytes).map_err(failure)?;
            if registry.wallet_instance != lease.instance.instance_id
                || registry.run_id != self.run_id.as_str()
            {
                return Err(failure("native registry foreign"));
            }
            for peer in &registry.peers {
                peer.process.validate().map_err(failure)?;
                let (record, config) = self.persisted_native_record("miner", &peer.miner_id)?;
                if peer.process.pid != record.pid
                    || peer.process.start_time_ticks != record.start_time_ticks
                    || hex::encode(peer.process.executable_sha256) != record.executable_sha256
                    || hash(record.run_token.as_bytes()) != peer.process.run_token_sha256
                    || config != Path::new(&peer.config_path)
                    || hash(&private_read(&config, peer.process.uid)?) != peer.process.config_sha256
                {
                    return Err(failure("native peer record replaced"));
                }
            }
        }
        Ok(())
    }

    /// Persisted-only manifest lookup for the cleanup validation above: the
    /// recorded process fields and the single recorded `--config`
    /// argument, with no live `/proc`, ownership, socket or token check.
    fn persisted_native_record(
        &self,
        role: &str,
        id: &str,
    ) -> Result<(ProcessRecord, PathBuf), EngineError> {
        let record = self
            .manifest
            .lock()
            .unwrap()
            .process(role, id)
            .cloned()
            .ok_or_else(|| failure("native record missing from manifest"))?;
        let paths: Vec<_> = record
            .args
            .windows(2)
            .filter(|w| w[0] == "--config")
            .map(|w| PathBuf::from(&w[1]))
            .collect();
        if paths.len() != 1 {
            return Err(failure("native record config ambiguous"));
        }
        Ok((record, paths[0].clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, DirBuilderExt, PermissionsExt};
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let path = PathBuf::from("/tmp/opencode").join(format!(
                "native-peer-test-{}",
                hex::encode(rand::random::<[u8; 16]>())
            ));
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&path)
                .unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn private_publication_is_complete_and_rejects_symlink_or_public_parent() {
        let dir = Temp::new();
        let uid = std::fs::metadata("/proc/self").unwrap().uid();
        let path = dir.0.join("peers.json");
        let first = atomic_private(&path, &serde_json::json!({"generation":1}), uid).unwrap();
        assert_eq!(private_read(&path, uid).unwrap(), first);
        let second = atomic_private(&path, &serde_json::json!({"generation":2}), uid).unwrap();
        assert_ne!(hash(&first), hash(&second));
        assert_eq!(private_read(&path, uid).unwrap(), second);
        let link = dir.0.join("link.json");
        symlink(&path, &link).unwrap();
        assert!(atomic_private(&link, &serde_json::json!({"generation":3}), uid).is_err());
        let dangling = dir.0.join("dangling.json");
        symlink(dir.0.join("missing.json"), &dangling).unwrap();
        assert!(atomic_private(&dangling, &serde_json::json!({"generation":3}), uid).is_err());
        assert_eq!(private_read(&path, uid).unwrap(), second);
        std::fs::set_permissions(&dir.0, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(atomic_private(&path, &serde_json::json!({"generation":4}), uid).is_err());
    }
}

#[cfg(test)]
#[path = "native_audit_cleanup_tests.rs"]
mod native_audit_cleanup_tests;
