//! Explicit operator-declared public collector bindings for the existing wallet
//! actor. Unix paths never use the TCP/API listener lease or browser input.
use anyhow::{ensure, Context};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};

#[cfg(test)]
#[path = "wallet_native_run_token_tests.rs"]
mod run_token_tests;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeAuditProfile {
    /// Absolute under this run's private root, or relative to that root.
    pub collector_socket: PathBuf,
    pub pool_pubkey: String,
    pub collector_pubkey: String,
}
impl NativeAuditProfile {
    pub fn validate(&self) -> anyhow::Result<()> {
        ensure!(
            !self.collector_socket.as_os_str().is_empty()
                && !self
                    .collector_socket
                    .components()
                    .any(|c| matches!(c, Component::ParentDir | Component::CurDir)),
            "native collector path"
        );
        let pool: bitcoin::secp256k1::XOnlyPublicKey = self.pool_pubkey.parse()?;
        let collector: bitcoin::secp256k1::XOnlyPublicKey = self.collector_pubkey.parse()?;
        ensure!(
            pool != collector
                && pool.to_string() == self.pool_pubkey
                && collector.to_string() == self.collector_pubkey,
            "native public source authority mismatch"
        );
        Ok(())
    }
}
pub(super) fn render(
    engine: &crate::engine::RunEngine,
    agent: &super::WalletAgentSpec,
    address: &str,
) -> anyhow::Result<Option<serde_json::Value>> {
    let Some(profile) = &agent.native_audit else {
        return Ok(None);
    };
    ensure!(
        engine.run_id == engine.scenario.run.id,
        "native renderer run mismatch"
    );
    ensure!(
        engine
            .manifest
            .lock()
            .map_err(|_| anyhow::anyhow!("manifest lock"))?
            .placement
            .is_none(),
        "native wallet collector needs host-local placement"
    );
    let pool = &engine
        .scenario
        .fronts
        .iter()
        .find(|f| f.id == agent.front_id)
        .context("native wallet front")?
        .pool_id;
    let token_hash = {
        let supervisor = engine
            .supervisor
            .lock()
            .map_err(|_| anyhow::anyhow!("supervisor lock"))?;
        let token = supervisor.run_token();
        ensure!(
            !token.is_empty() && token.len() <= 1024,
            "native supervisor token bound"
        );
        // Borrow the supervisor's private token; only its digest leaves the lock.
        <[u8; 32]>::from(Sha256::digest(token.as_bytes()))
    };
    render_at(
        &engine.run_dir.join("private"),
        (engine.scenario.run.id.as_ref(), token_hash),
        pool.as_str(),
        agent.front_id.as_str(),
        agent.identity_id.as_str(),
        address,
        profile,
    )
    .map(Some)
}
fn render_at(
    private: &Path,
    scope: (&str, [u8; 32]),
    pool: &str,
    front: &str,
    identity: &str,
    address: &str,
    profile: &NativeAuditProfile,
) -> anyhow::Result<serde_json::Value> {
    let (run, token_hash) = scope;
    ensure!(token_hash != [0; 32], "native supervisor token digest");
    profile.validate()?;
    let keys = spc_scan::native_activation::VerificationKeys::for_regtest_identity(
        address,
        &profile.collector_pubkey,
        &profile.pool_pubkey,
    )
    .map_err(anyhow::Error::msg)?;
    ensure!(
        private.canonicalize()? == private,
        "native runtime root is not canonical"
    );
    let socket = if profile.collector_socket.is_absolute() {
        profile.collector_socket.clone()
    } else {
        private.join(&profile.collector_socket)
    };
    ensure!(
        socket.starts_with(private) && socket.file_name().is_some(),
        "native collector outside run"
    );
    let uid = std::fs::metadata("/proc/self")?.uid();
    let mut parent = socket.parent().context("native collector parent")?;
    loop {
        let m = std::fs::symlink_metadata(parent)?;
        ensure!(
            parent.canonicalize()? == parent
                && m.is_dir()
                && m.uid() == uid
                && m.mode() & 0o777 == 0o700,
            "native collector directory ownership"
        );
        if parent == private {
            break;
        }
        parent = parent.parent().context("native collector escaped root")?;
    }
    if let Ok(m) = socket.symlink_metadata() {
        ensure!(
            m.file_type().is_socket() && m.uid() == uid && m.mode() & 0o777 == 0o600,
            "native collector socket ownership"
        );
    }
    Ok(
        serde_json::json!({"run_id":run,"run_token_hash":hex::encode(token_hash),"pool_id":pool,"front_id":front,"identity_id":identity,
        "collector_socket":socket,"pool_pubkey":keys.pool.to_string(),"collector_pubkey":keys.collector.to_string()}),
    )
}

/// On restart/adopt the existing config must agree with recorded run markers.
/// No marker is reconstructed from owner_instance, run ID, or token shape.
pub(super) fn validate_persisted(
    engine: &crate::engine::RunEngine,
    agent: &super::WalletAgentSpec,
    address: &str,
    path: &Path,
) -> anyhow::Result<()> {
    let Some(expected) = render(engine, agent, address)? else {
        return Ok(());
    };
    {
        let manifest = engine
            .manifest
            .lock()
            .map_err(|_| anyhow::anyhow!("manifest lock"))?;
        ensure!(
            manifest.run_id == engine.scenario.run.id && !manifest.processes.is_empty(),
            "native recorded run marker missing"
        );
        for record in &manifest.processes {
            ensure!(
                !record.run_token.is_empty()
                    && record.run_token.len() <= 1024
                    && expected["run_token_hash"].as_str()
                        == Some(hex::encode(Sha256::digest(record.run_token.as_bytes())).as_str()),
                "native recorded run marker mismatch"
            );
        }
    }
    let private = engine.run_dir.join("private");
    let bytes = private_config(&private, path)?;
    let value: toml::Value = toml::from_str(std::str::from_utf8(&bytes)?)?;
    let actual = value
        .get("ark")
        .and_then(|v| v.get("native_audit"))
        .context("native persisted run binding missing")?;
    ensure!(
        serde_json::to_value(actual)? == expected,
        "native persisted run binding mismatch"
    );
    ensure!(
        value
            .get("upstream")
            .and_then(|v| v.get("sp_address"))
            .and_then(toml::Value::as_str)
            == Some(address),
        "native persisted identity mismatch"
    );
    Ok(())
}

pub(super) fn validate_before_write(
    engine: &crate::engine::RunEngine,
    agent: &super::WalletAgentSpec,
    address: &str,
    path: &Path,
) -> anyhow::Result<()> {
    if agent.native_audit.is_none() {
        return Ok(());
    }
    match path.symlink_metadata() {
        Ok(_) => validate_persisted(engine, agent, address, path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

fn private_config(root: &Path, path: &Path) -> anyhow::Result<Vec<u8>> {
    ensure!(
        path.starts_with(root) && path.canonicalize()? == path,
        "native config path"
    );
    let uid = std::fs::metadata("/proc/self")?.uid();
    let mut parent = path.parent().context("native config parent")?;
    loop {
        let m = parent.symlink_metadata()?;
        ensure!(
            parent.canonicalize()? == parent
                && m.is_dir()
                && m.uid() == uid
                && m.mode() & 0o777 == 0o700,
            "native config directory ownership"
        );
        if parent == root {
            break;
        }
        parent = parent.parent().context("native config root")?;
    }
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags((rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32)
        .open(path)?;
    let before = file.metadata()?;
    ensure!(
        before.is_file()
            && before.uid() == uid
            && before.mode() & 0o777 == 0o600
            && before.nlink() == 1
            && before.len() <= 1024 * 1024,
        "native config ownership/bound"
    );
    let mut bytes = Vec::new();
    (&mut file).take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
    let after = file.metadata()?;
    let current = path.symlink_metadata()?;
    ensure!(
        bytes.len() as u64 == before.len()
            && after.len() == before.len()
            && after.mtime() == before.mtime()
            && after.mtime_nsec() == before.mtime_nsec()
            && after.ctime() == before.ctime()
            && after.ctime_nsec() == before.ctime_nsec()
            && current.dev() == before.dev()
            && current.ino() == before.ino(),
        "native config changed"
    );
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::secp256k1::{Keypair, Secp256k1, SecretKey};
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn native_render_confines_declared_collector_and_keeps_only_public_binding() {
        let key = |n| {
            Keypair::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&[n; 32]).unwrap())
        };
        let dir = std::env::temp_dir().join(format!("native-render-{}", rand::random::<u64>()));
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let address = spc_crypto::address::encode_sp_address(
            spc_crypto::Network::Regtest,
            &key(1).public_key(),
            &key(2).public_key(),
        )
        .unwrap();
        let mut p = NativeAuditProfile {
            collector_socket: "collector.sock".into(),
            pool_pubkey: key(3).x_only_public_key().0.to_string(),
            collector_pubkey: key(4).x_only_public_key().0.to_string(),
        };
        let token_hash = Sha256::digest(b"run:fixture").into();
        let rendered = render_at(
            &dir,
            ("run", token_hash),
            "pool",
            "front",
            "alice",
            &address,
            &p,
        )
        .unwrap();
        assert_eq!(rendered["run_token_hash"], hex::encode(token_hash));
        assert_eq!(
            rendered["collector_socket"],
            dir.join("collector.sock").to_str().unwrap()
        );
        assert!(rendered.get("scan_secret_key_file").is_none());
        assert!(rendered.get("wallet_socket").is_none());
        p.collector_socket = "../outside.sock".into();
        assert!(render_at(
            &dir,
            ("run", token_hash),
            "pool",
            "front",
            "alice",
            &address,
            &p
        )
        .is_err());
        p.collector_socket = "/outside.sock".into();
        assert!(render_at(
            &dir,
            ("run", token_hash),
            "pool",
            "front",
            "alice",
            &address,
            &p
        )
        .is_err());
        std::fs::remove_dir(&dir).unwrap();
    }
}
