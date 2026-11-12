//! Explicit operator-declared public collector bindings for the existing wallet
//! actor. Unix paths never use the TCP/API listener lease or browser input.
use anyhow::{ensure, Context};
use serde::{Deserialize, Serialize};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Component, Path, PathBuf};

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
        engine.manifest_snapshot().placement.is_none(),
        "native wallet collector needs host-local placement"
    );
    let pool = &engine
        .scenario
        .fronts
        .iter()
        .find(|f| f.id == agent.front_id)
        .context("native wallet front")?
        .pool_id;
    render_at(
        &engine.run_dir.join("private"),
        engine.scenario.run.id.as_ref(),
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
    run: &str,
    pool: &str,
    front: &str,
    identity: &str,
    address: &str,
    profile: &NativeAuditProfile,
) -> anyhow::Result<serde_json::Value> {
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
        serde_json::json!({"run_id":run,"pool_id":pool,"front_id":front,"identity_id":identity,
        "collector_socket":socket,"pool_pubkey":keys.pool.to_string(),"collector_pubkey":keys.collector.to_string()}),
    )
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
        let rendered = render_at(&dir, "run", "pool", "front", "alice", &address, &p).unwrap();
        assert_eq!(
            rendered["collector_socket"],
            dir.join("collector.sock").to_str().unwrap()
        );
        assert!(rendered.get("scan_secret_key_file").is_none());
        assert!(rendered.get("wallet_socket").is_none());
        p.collector_socket = "../outside.sock".into();
        assert!(render_at(&dir, "run", "pool", "front", "alice", &address, &p).is_err());
        p.collector_socket = "/outside.sock".into();
        assert!(render_at(&dir, "run", "pool", "front", "alice", &address, &p).is_err());
        std::fs::remove_dir(&dir).unwrap();
    }
}
