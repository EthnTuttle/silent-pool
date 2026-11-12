//! Cold, one-shot provisioner. The runtime invokes this in a separate process;
//! the online role receives only the two account branches and BIP85 child seed.

use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

use anyhow::{ensure, Context};
use bitcoin::secp256k1::{PublicKey, Secp256k1};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvisionedWallet {
    pub schema: String,
    pub network: String,
    pub sp_address: String,
    pub child_fingerprint: String,
}

fn private_dir(path: &Path) -> anyhow::Result<()> {
    if !path.exists() {
        std::fs::DirBuilder::new().mode(0o700).create(path)?;
    }
    let meta = std::fs::symlink_metadata(path)?;
    ensure!(
        meta.is_dir() && meta.permissions().mode() & 0o777 == 0o700,
        "provisioning directory must be private and not a symlink"
    );
    Ok(())
}

fn write_new(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

/// Regtest lab provisioning. No seed or xprv is accepted in argv or returned.
/// Existing complete provisioning is validated; partial state is refused.
pub fn provision(wallet_dir: &Path, cold_dir: &Path) -> anyhow::Result<ProvisionedWallet> {
    private_dir(wallet_dir)?;
    private_dir(cold_dir)?;
    let wallet_path = wallet_dir.canonicalize()?;
    let cold_path = cold_dir.canonicalize()?;
    ensure!(
        !wallet_path.starts_with(&cold_path) && !cold_path.starts_with(&wallet_path),
        "cold and online provisioning directories must be separate"
    );
    let manifest = wallet_dir.join("provision.json");
    if manifest.exists() {
        let record: ProvisionedWallet = serde_json::from_slice(&std::fs::read(&manifest)?)?;
        ensure!(
            record.schema == "spc.agent.provision/v1" && record.network == "regtest",
            "foreign wallet provisioning"
        );
        let keys = super::WalletKeys::load(&super::KeySources {
            scan_xprv: wallet_dir.join("scan.xprv"),
            ark_xprv: wallet_dir.join("ark.xprv"),
            child_seed: wallet_dir.join("child.seed"),
            sp_address: record.sp_address.clone(),
            network: spc_crypto::Network::Regtest,
        })?;
        ensure!(
            keys.wallet_fingerprint.to_string() == record.child_fingerprint,
            "provisioned child fingerprint mismatch"
        );
        return Ok(record);
    }
    ensure!(
        std::fs::read_dir(wallet_dir)?.next().is_none()
            && std::fs::read_dir(cold_dir)?.next().is_none(),
        "partial provisioning refused; key material is never regenerated"
    );
    let mut master = zeroize::Zeroizing::new([0u8; 64]);
    std::fs::File::open("/dev/urandom")?.read_exact(&mut *master)?;
    let keys = spc_crypto::provision(&master, spc_crypto::Network::Regtest)?;
    let secp = Secp256k1::new();
    let scan = spc_crypto::hd::spc_branch_secret(&keys.scan_branch)?;
    let spend = spc_crypto::hd::spc_branch_secret(&keys.spend_branch)?;
    let address = spc_crypto::address::encode_sp_address(
        spc_crypto::Network::Regtest,
        &PublicKey::from_secret_key(&secp, &scan),
        &PublicKey::from_secret_key(&secp, &spend),
    )?;
    // Cold backup commits first. A crash leaves explicit partial provisioning,
    // never a silent replacement seed on the next invocation.
    write_new(
        &cold_dir.join("master.seed"),
        zeroize::Zeroizing::new(hex::encode(*master)).as_bytes(),
    )?;
    write_new(
        &wallet_dir.join("scan.xprv"),
        zeroize::Zeroizing::new(keys.scan_branch.to_string()).as_bytes(),
    )?;
    write_new(
        &wallet_dir.join("ark.xprv"),
        zeroize::Zeroizing::new(keys.ark_branch.to_string()).as_bytes(),
    )?;
    write_new(
        &wallet_dir.join("child.seed"),
        zeroize::Zeroizing::new(hex::encode(keys.child.seed)).as_bytes(),
    )?;
    let record = ProvisionedWallet {
        schema: "spc.agent.provision/v1".into(),
        network: "regtest".into(),
        sp_address: address,
        child_fingerprint: bark::WalletSeed::new_from_seed(
            bitcoin::Network::Regtest,
            &keys.child.seed,
        )
        .fingerprint()
        .to_string(),
    };
    write_new(&manifest, &serde_json::to_vec(&record)?)?;
    std::fs::File::open(wallet_dir)?
        .sync_all()
        .context("sync wallet provisioning directory")?;
    std::fs::File::open(cold_dir)?.sync_all()?;
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_validates_same_child_and_tamper_never_regenerates() {
        let root = tempfile::tempdir().unwrap();
        let wallet = root.path().join("online");
        let cold = root.path().join("cold");
        let first = provision(&wallet, &cold).unwrap();
        let master = std::fs::read(cold.join("master.seed")).unwrap();
        let child = std::fs::read(wallet.join("child.seed")).unwrap();
        let second = provision(&wallet, &cold).unwrap();
        assert_eq!(first.sp_address, second.sp_address);
        assert_eq!(first.child_fingerprint, second.child_fingerprint);
        for name in ["scan.xprv", "ark.xprv", "child.seed", "provision.json"] {
            assert_eq!(
                std::fs::metadata(wallet.join(name))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        std::fs::write(wallet.join("child.seed"), "11".repeat(64)).unwrap();
        assert!(provision(&wallet, &cold).is_err());
        assert_eq!(std::fs::read(cold.join("master.seed")).unwrap(), master);
        assert_ne!(std::fs::read(wallet.join("child.seed")).unwrap(), child);
        assert!(!wallet.join("master.seed").exists());
    }

    #[test]
    fn partial_material_and_nested_cold_store_are_refused() {
        let root = tempfile::tempdir().unwrap();
        let wallet = root.path().join("online");
        let cold = root.path().join("cold");
        private_dir(&wallet).unwrap();
        write_new(&wallet.join("child.seed"), b"partial").unwrap();
        assert!(provision(&wallet, &cold).is_err());
        assert!(!cold.join("master.seed").exists());
        assert!(provision(&wallet, &wallet.join("cold")).is_err());
    }
}
