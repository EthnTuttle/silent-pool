//! Provisioned key material for the wallet role (SPEC 11.5, 11.8; plan
//! 3.3).
//!
//! The agent loads **only** what the provisioner gives it:
//!
//! - the scan branch account xprv `m/352'/0'/0'/1'` (file, xprv string),
//! - the ark branch account xprv `m/352'/0'/0'/2'` (file, xprv string),
//! - the BIP 85 child seed (file, 128 hex chars) - the 64-byte BIP 39
//!   seed of `BIP85(master, m/83696968'/39'/0'/12'/0')`, which the
//!   embedded stock bark wallet runs on,
//! - the public `b_spend`, taken from `sp_address` (never a file on the
//!   mining daemon).
//!
//! The master seed and the spend branch `m/352'/0'/0'/0'` never reach
//! this process; there is no code path that could load them.
//!
//! Fail-closed validation at load time:
//!
//! - key files must be `0600` (and the datadir `0700`, checked by the
//!   state store): loose permissions are an error, not a chmod;
//! - branch xprvs must be depth-4 hardened account keys - the branch
//!   shape the SPc tree defines;
//! - xprv network magic must match the configured network (mainnet
//!   `xprv` vs test networks `tprv`; the magic cannot distinguish
//!   testnet from regtest, which is why `[ark] network` exists);
//! - each branch file must be the **exact** depth-4 hardened account
//!   key for its branch index (`m/352'/0'/0'/1'` scan, `m/352'/0'/0'/2'`
//!   ark) - not merely any depth-4 hardened account;
//! - scan and ark branch accounts must share a parent fingerprint:
//!   they must be siblings of the same `m/352'/0'/0'` tree. This is a
//!   required **provisioning metadata check** - the daemon holds no
//!   master key, and the 4-byte parent fingerprint does *not*
//!   cryptographically prove the full derivation path; it is the
//!   strongest consistency check available without the master, and a
//!   mismatch always fails closed;
//! - the scan branch must derive exactly the `B_scan` in `sp_address`;
//! - `B_ark != B_spend` (`ark-key-is-spend-key` is a terminal server
//!   error; catch it locally before any send);
//! - scan and ark branches must be distinct keys;
//! - the child seed is 64 bytes and its stock bark `WalletSeed`
//!   fingerprint is cross-checked against the in-tree derivation.
//!
//! A failed load regenerates nothing: `spc_crypto::provision` is the
//! provisioner's tool, not the agent's.

use std::path::Path;

use bitcoin::bip32::{Fingerprint, Xpriv};
use bitcoin::secp256k1::{Keypair, PublicKey, Secp256k1, SecretKey};
use spc_crypto::{Network, SpAddress};
use zeroize::Zeroizing;

use crate::wallet::state::StoreIdentity;

/// Errors from loading provisioned wallet keys. All fail closed; none
/// of them ever produces or overwrites key material.
#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    #[error("cannot read key file {0}: {1}")]
    Read(String, String),
    #[error("key file {0} has loose permissions ({1}); expected 0600 - fix the provisioning, refusing to run")]
    Permissions(String, String),
    #[error("key file {0} does not contain a valid xprv: {1}")]
    ParseXprv(String, String),
    #[error("key file {0} does not contain 128 hex chars (64-byte seed): {1}")]
    ParseSeed(String, String),
    #[error("key file {path} is not the m/352'/0'/0'/{branch} branch account - found depth {depth}, child {child}")]
    BranchShape {
        path: String,
        branch: &'static str,
        depth: u8,
        child: String,
    },
    #[error(
        "the scan and ark branch accounts do not share a parent fingerprint ({scan_fp} vs {ark_fp}); \
         they are not siblings of the same m/352'/0'/0' tree - provisioning error (metadata check: \
         the 4-byte parent fingerprint is required consistency, not a cryptographic ancestry proof)"
    )]
    BranchesNotSiblings { scan_fp: String, ark_fp: String },
    #[error("key file {path} is for network {found}, config says {expected}")]
    Network {
        path: String,
        found: String,
        expected: String,
    },
    #[error("the scan branch does not derive the B_scan in sp_address")]
    ScanMismatch,
    #[error("the ark branch derives B_spend (ark-key-is-spend-key); the ark branch must never be the spend branch")]
    ArkKeyIsSpendKey,
    #[error("the scan and ark branch files hold the same key")]
    BranchesNotDistinct,
    #[error("sp_address is not valid for the configured network: {0}")]
    Address(String),
    #[error("key derivation failed: {0}")]
    Derivation(String),
}

/// Where the provisioned material lives.
#[derive(Clone, Debug)]
pub struct KeySources {
    /// File holding the scan branch xprv `m/352'/0'/0'/1'`.
    pub scan_xprv: std::path::PathBuf,
    /// File holding the ark branch xprv `m/352'/0'/0'/2'`.
    pub ark_xprv: std::path::PathBuf,
    /// File holding the 64-byte BIP 85 child seed (hex).
    pub child_seed: std::path::PathBuf,
    /// The pool's `sp_address` (public `B_scan`/`B_spend`).
    pub sp_address: String,
    pub network: Network,
}

/// The loaded, validated wallet keys. Holds secrets; `Debug` is
/// redacted and the material is overwritten on drop.
pub struct WalletKeys {
    /// `b_scan = m/352'/0'/0'/1'/0`.
    pub b_scan: SecretKey,
    /// `b_ark = m/352'/0'/0'/2'/0`.
    pub b_ark: SecretKey,
    /// Public `B_spend` from `sp_address` (the agent never holds the
    /// secret).
    pub b_spend_public: PublicKey,
    /// `hash_SPc/Id(ser_P(B_scan) || ser_P(B_spend))`.
    pub sp_id: [u8; 32],
    pub network: Network,
    /// The BIP 85 child seed (owns `M` and the normal wallet keys).
    child_seed: Zeroizing<[u8; 64]>,
    /// Stock bark `WalletSeed` fingerprint of the child seed: the
    /// store-identity binding.
    pub wallet_fingerprint: Fingerprint,
    /// `M = m/350'/1'` under the child seed.
    pub mailbox: Keypair,
    /// Normal (non-SPc) wallet key `m/350'/0'/0` under the child seed.
    pub normal_vtxo: Keypair,
}

impl std::fmt::Debug for WalletKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // No secret, no sp_id, no pubkey that is not already public
        // config: fingerprints only.
        f.debug_struct("WalletKeys")
            .field("network", &self.network)
            .field("wallet_fingerprint", &self.wallet_fingerprint)
            .finish_non_exhaustive()
    }
}

impl Drop for WalletKeys {
    fn drop(&mut self) {
        self.zeroize_keys();
    }
}

pub(crate) fn bitcoin_network(network: Network) -> bitcoin::Network {
    match network {
        Network::Mainnet => bitcoin::Network::Bitcoin,
        Network::Testnet => bitcoin::Network::Testnet,
        Network::Signet => bitcoin::Network::Signet,
        Network::Regtest => bitcoin::Network::Regtest,
    }
}

impl WalletKeys {
    pub(crate) fn bark_seed(&self) -> bark::WalletSeed {
        bark::WalletSeed::new_from_seed(bitcoin_network(self.network), &self.child_seed)
    }

    /// Load and validate the provisioned material. Fails closed on any
    /// ambiguity; regenerates nothing.
    pub fn load(sources: &KeySources) -> Result<Self, KeyError> {
        let address: SpAddress = spc_crypto::parse_sp_address(&sources.sp_address, sources.network)
            .map_err(|e| KeyError::Address(e.to_string()))?;

        let scan_branch = load_branch_xprv(&sources.scan_xprv, 1, "1'", sources.network)?;
        let ark_branch = load_branch_xprv(&sources.ark_xprv, 2, "2'", sources.network)?;
        // Provisioning metadata check (fail closed): both branch
        // accounts must be siblings - same parent fingerprint, i.e.
        // both cut from the same m/352'/0'/0' account. The daemon
        // deliberately holds no master key, so this 4-byte check is the
        // available consistency proof between the two files; it does
        // NOT cryptographically establish the full derivation path.
        if scan_branch.parent_fingerprint != ark_branch.parent_fingerprint {
            return Err(KeyError::BranchesNotSiblings {
                scan_fp: scan_branch.parent_fingerprint.to_string(),
                ark_fp: ark_branch.parent_fingerprint.to_string(),
            });
        }

        let b_scan = spc_crypto::hd::spc_branch_secret(&scan_branch)
            .map_err(|e| KeyError::Derivation(e.to_string()))?;
        let b_ark = spc_crypto::hd::spc_branch_secret(&ark_branch)
            .map_err(|e| KeyError::Derivation(e.to_string()))?;

        // The scan branch must match sp_address exactly.
        let secp = Secp256k1::new();
        if PublicKey::from_secret_key(&secp, &b_scan) != address.b_scan {
            return Err(KeyError::ScanMismatch);
        }
        // The ark branch must never be the spend key.
        let b_ark_public = PublicKey::from_secret_key(&secp, &b_ark);
        if b_ark_public == address.b_spend {
            return Err(KeyError::ArkKeyIsSpendKey);
        }
        if b_scan == b_ark {
            return Err(KeyError::BranchesNotDistinct);
        }

        let child_seed = load_child_seed(&sources.child_seed)?;
        let network = sources.network;
        let wallet_fingerprint =
            bark_wallet_seed_fingerprint(&child_seed, network).map_err(KeyError::Derivation)?;
        // Cross-check the stock bark derivation against the in-tree one.
        let in_tree_root = spc_crypto::hd::bark_root_xpriv(&child_seed, network)
            .map_err(|e| KeyError::Derivation(e.to_string()))?;
        if in_tree_root.fingerprint(&secp) != wallet_fingerprint {
            return Err(KeyError::Derivation(
                "stock bark WalletSeed fingerprint does not match the in-tree derivation".into(),
            ));
        }

        let mailbox = spc_crypto::hd::bark_mailbox_keypair(&child_seed, network)
            .map_err(|e| KeyError::Derivation(e.to_string()))?;
        let normal_vtxo = spc_crypto::hd::bark_vtxo_keypair(&child_seed, network, 0)
            .map_err(|e| KeyError::Derivation(e.to_string()))?;

        Ok(WalletKeys {
            b_scan,
            b_ark,
            b_spend_public: address.b_spend,
            sp_id: spc_crypto::sp_id(&address.b_scan, &address.b_spend),
            network,
            child_seed,
            wallet_fingerprint,
            mailbox,
            normal_vtxo,
        })
    }

    /// The store-identity binding for the persistent state.
    pub fn store_identity(&self) -> StoreIdentity {
        StoreIdentity {
            wallet_fingerprint: self.wallet_fingerprint,
            sp_id: self.sp_id,
            network: network_name(self.network).to_string(),
        }
    }

    /// The public `B_ark` (`U_i = B_ark + t_ark*G`).
    pub fn b_ark_public(&self) -> PublicKey {
        PublicKey::from_secret_key(&Secp256k1::new(), &self.b_ark)
    }

    /// The static mailbox id `M` (public).
    pub fn mailbox_public(&self) -> PublicKey {
        self.mailbox.public_key()
    }

    /// The normal wallet's first address (public; parity with bark's
    /// derived keys, used by recovery checks).
    pub fn normal_vtxo_public(&self) -> PublicKey {
        self.normal_vtxo.public_key()
    }

    /// Best-effort in-place erasure of the held key material: the scan
    /// and ark branch secrets, the BIP 85 child seed, and the mailbox
    /// and normal wallet keypairs (mailbox + normal key are the
    /// secp-supported surface). secp256k1 secrets cannot be zeroized
    /// byte-for-byte through the safe API; they are replaced with a
    /// dummy valid key so the real bytes do not survive in process
    /// memory. Called on drop; exposed for explicit early scrubbing.
    pub fn zeroize_keys(&mut self) {
        let dummy = SecretKey::from_slice(&[1u8; 32]).expect("dummy key is valid");
        let dummy_pair = Keypair::from_secret_key(&Secp256k1::new(), &dummy);
        self.b_scan = dummy;
        self.b_ark = dummy;
        self.child_seed = Zeroizing::new([0u8; 64]);
        self.mailbox = dummy_pair;
        self.normal_vtxo = dummy_pair;
    }
}

/// The store's network label (regtest is distinct from testnet even
/// though the xprv magic cannot tell them apart).
pub fn network_name(network: Network) -> &'static str {
    match network {
        Network::Mainnet => "mainnet",
        Network::Testnet => "testnet",
        Network::Signet => "signet",
        Network::Regtest => "regtest",
    }
}

/// Parse a network name from `[ark] network`.
pub fn parse_network_name(s: &str) -> Option<Network> {
    match s {
        "mainnet" => Some(Network::Mainnet),
        "testnet" => Some(Network::Testnet),
        "signet" => Some(Network::Signet),
        "regtest" => Some(Network::Regtest),
        _ => None,
    }
}

/// Read one key file and enforce 0600 before parsing anything.
fn read_key_file(path: &Path) -> Result<String, KeyError> {
    let meta = std::fs::metadata(path)
        .map_err(|e| KeyError::Read(path.display().to_string(), e.to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = meta.permissions().mode() & 0o777;
        if mode != 0o600 {
            return Err(KeyError::Permissions(
                path.display().to_string(),
                format!("{mode:o}"),
            ));
        }
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        return Err(KeyError::Read(
            path.display().to_string(),
            "the wallet role requires unix file permissions".into(),
        ));
    }
    #[allow(unreachable_code)]
    std::fs::read_to_string(path)
        .map_err(|e| KeyError::Read(path.display().to_string(), e.to_string()))
}

/// Load a branch account xprv and validate its exact shape: the
/// depth-4 hardened account key `m/352'/0'/0'/{expected_index}'` (depth
/// 4, that exact hardened child index) and correct network magic. The
/// agent cannot verify the full path without the master (which it must
/// not hold); depth + the exact child index + the shared parent
/// fingerprint checked in [`WalletKeys::load`] + the `B_scan` match
/// against `sp_address` are the enforceable invariants.
fn load_branch_xprv(
    path: &Path,
    expected_index: u32,
    branch: &'static str,
    network: Network,
) -> Result<Xpriv, KeyError> {
    let text = read_key_file(path)?;
    let xprv: Xpriv = text
        .trim()
        .parse::<Xpriv>()
        .map_err(|e| KeyError::ParseXprv(path.display().to_string(), e.to_string()))?;
    let expected_child = bitcoin::bip32::ChildNumber::from_hardened_idx(expected_index)
        .expect("hardened branch indices are valid child numbers");
    if xprv.depth != 4 || xprv.child_number != expected_child {
        return Err(KeyError::BranchShape {
            path: path.display().to_string(),
            branch,
            depth: xprv.depth,
            child: xprv.child_number.to_string(),
        });
    }
    // The magic only distinguishes mainnet (xprv) from test networks
    // (tprv, covering testnet/signet/regtest); `[ark] network` carries
    // the rest.
    let expected = bitcoin_network(network);
    let expected_mainnet = matches!(expected, bitcoin::Network::Bitcoin);
    let found_mainnet = xprv.network.is_mainnet();
    if expected_mainnet != found_mainnet {
        return Err(KeyError::Network {
            path: path.display().to_string(),
            found: if found_mainnet {
                "mainnet (xprv)".into()
            } else {
                "test networks (tprv)".into()
            },
            expected: expected.to_string(),
        });
    }
    Ok(xprv)
}

/// Load the 64-byte BIP 85 child seed (hex file, 0600).
fn load_child_seed(path: &Path) -> Result<Zeroizing<[u8; 64]>, KeyError> {
    let text = read_key_file(path)?;
    let trimmed = text.trim();
    let bytes = hex::decode(trimmed)
        .map_err(|e| KeyError::ParseSeed(path.display().to_string(), e.to_string()))?;
    let seed: [u8; 64] = bytes
        .try_into()
        .map_err(|_| KeyError::ParseSeed(path.display().to_string(), "not 64 bytes".into()))?;
    Ok(Zeroizing::new(seed))
}

/// The stock bark `WalletSeed` fingerprint of the child seed
/// (`WalletSeed::new_from_seed`, bark c419159). AP-3a persists this
/// fingerprint (and derives keys itself); opening the full bark
/// `Wallet` needs a live chain source and lands in AP-3b.
fn bark_wallet_seed_fingerprint(
    child_seed: &[u8; 64],
    network: Network,
) -> Result<Fingerprint, String> {
    let seed = bark::WalletSeed::new_from_seed(bitcoin_network(network), child_seed);
    Ok(seed.fingerprint())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::bip32::ChildNumber;
    use spc_crypto::address::encode_sp_address;

    fn write_keys(dir: &Path, master_seed: &[u8; 64], network: Network) -> KeySources {
        let keys = spc_crypto::provision(master_seed, network).unwrap();
        let scan_path = dir.join("scan.xprv");
        let ark_path = dir.join("ark.xprv");
        let seed_path = dir.join("child.seed");
        write_secret(&scan_path, &keys.scan_branch.to_string());
        write_secret(&ark_path, &keys.ark_branch.to_string());
        write_secret(&seed_path, &hex::encode(keys.child.seed));
        let spend_pub = {
            let secp = Secp256k1::new();
            let spend = spc_crypto::hd::spc_branch_secret(&keys.spend_branch).unwrap();
            PublicKey::from_secret_key(&secp, &spend)
        };
        let scan_pub = {
            let secp = Secp256k1::new();
            let scan = spc_crypto::hd::spc_branch_secret(&keys.scan_branch).unwrap();
            PublicKey::from_secret_key(&secp, &scan)
        };
        KeySources {
            scan_xprv: scan_path,
            ark_xprv: ark_path,
            child_seed: seed_path,
            sp_address: encode_sp_address(network, &scan_pub, &spend_pub).unwrap(),
            network,
        }
    }

    fn write_secret(path: &Path, content: &str) {
        std::fs::write(path, content).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
    }

    fn master() -> [u8; 64] {
        [0x42u8; 64]
    }

    #[test]
    fn loads_and_validates_provisioned_keys() {
        let dir = tempfile::tempdir().unwrap();
        let sources = write_keys(dir.path(), &master(), Network::Regtest);
        let keys = WalletKeys::load(&sources).unwrap();

        let secp = Secp256k1::new();
        // b_scan matches sp_address; b_ark is the ark branch; the spend
        // key is public-only.
        assert_eq!(
            PublicKey::from_secret_key(&secp, &keys.b_scan),
            spc_crypto::parse_sp_address(&sources.sp_address, Network::Regtest)
                .unwrap()
                .b_scan
        );
        assert_ne!(keys.b_ark, keys.b_scan);
        assert_ne!(keys.b_ark_public(), keys.b_spend_public);

        // sp_id over the public pair.
        let parsed = spc_crypto::parse_sp_address(&sources.sp_address, Network::Regtest).unwrap();
        assert_eq!(
            keys.sp_id,
            spc_crypto::sp_id(&parsed.b_scan, &parsed.b_spend)
        );

        // The stock bark fingerprint matches the in-tree derivation.
        let expected =
            spc_crypto::hd::bark_root_xpriv(&keys_provisioned_seed(&sources), Network::Regtest)
                .unwrap()
                .fingerprint(&secp);
        assert_eq!(keys.wallet_fingerprint, expected);

        // M and the normal wallet key are distinct bark keys.
        assert_ne!(keys.mailbox_public(), keys.normal_vtxo_public());

        // The store identity binds all three.
        let identity = keys.store_identity();
        assert_eq!(identity.sp_id, keys.sp_id);
        assert_eq!(identity.wallet_fingerprint, keys.wallet_fingerprint);
        assert_eq!(identity.network, "regtest");

        // Debug is redacted.
        let text = format!("{keys:?}");
        assert!(!text.contains("SecretKey"));
        assert!(!text.contains("child"));
    }

    /// Re-derive the child seed from the same master for cross-checks.
    fn keys_provisioned_seed(_sources: &KeySources) -> [u8; 64] {
        spc_crypto::provision(&master(), Network::Regtest)
            .unwrap()
            .child
            .seed
    }

    #[test]
    fn deterministic_rematerialization_gives_the_same_keys() {
        let dir = tempfile::tempdir().unwrap();
        let sources = write_keys(dir.path(), &master(), Network::Regtest);
        let first = WalletKeys::load(&sources).unwrap();
        let identity = first.store_identity();
        let mailbox = first.mailbox_public();
        let normal = first.normal_vtxo_public();
        let b_ark = first.b_ark_public();
        drop(first);
        // "Restart": load again from the same files.
        let second = WalletKeys::load(&sources).unwrap();
        assert_eq!(second.store_identity(), identity);
        assert_eq!(second.mailbox_public(), mailbox);
        assert_eq!(second.normal_vtxo_public(), normal);
        assert_eq!(second.b_ark_public(), b_ark);
    }

    #[test]
    fn loose_key_file_permissions_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let sources = write_keys(dir.path(), &master(), Network::Regtest);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&sources.scan_xprv, std::fs::Permissions::from_mode(0o644))
                .unwrap();
        }
        let err = WalletKeys::load(&sources).unwrap_err();
        assert!(matches!(err, KeyError::Permissions(_, _)), "{err}");
        // The file is not silently tightened or rewritten.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&sources.scan_xprv)
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o644);
        }
    }

    #[test]
    fn scan_branch_mismatch_with_sp_address_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let mut sources = write_keys(dir.path(), &master(), Network::Regtest);
        // sp_address built from a *different* scan key.
        let secp = Secp256k1::new();
        let other_scan =
            PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&[9u8; 32]).unwrap());
        let parsed = spc_crypto::parse_sp_address(&sources.sp_address, Network::Regtest).unwrap();
        sources.sp_address =
            encode_sp_address(Network::Regtest, &other_scan, &parsed.b_spend).unwrap();
        let err = WalletKeys::load(&sources).unwrap_err();
        assert!(matches!(err, KeyError::ScanMismatch), "{err}");
    }

    #[test]
    fn ark_branch_equal_to_spend_key_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let sources = write_keys(dir.path(), &master(), Network::Regtest);
        // Writing the *spend* branch (m/352'/0'/0'/0') into the ark file
        // is now caught even earlier: the exact child-index check
        // rejects the 0' account before anything else.
        let keys = spc_crypto::provision(&master(), Network::Regtest).unwrap();
        write_secret(&sources.ark_xprv, &keys.spend_branch.to_string());
        let err = WalletKeys::load(&sources).unwrap_err();
        assert!(
            matches!(err, KeyError::BranchShape { .. }),
            "the spend branch is refused at load (wrong index): {err}"
        );

        // The `ark-key-is-spend-key` defense-in-depth check itself: an
        // sp_address whose B_spend IS the ark branch public key (a
        // mis-provisioned pool would have to send exactly this).
        // Restore the real ark branch to the file first - the crafted
        // address is the corruption under test, not the key files.
        write_secret(&sources.ark_xprv, &keys.ark_branch.to_string());
        let ark_pub = {
            let secp = Secp256k1::new();
            let ark = spc_crypto::hd::spc_branch_secret(&keys.ark_branch).unwrap();
            PublicKey::from_secret_key(&secp, &ark)
        };
        let parsed = spc_crypto::parse_sp_address(&sources.sp_address, Network::Regtest).unwrap();
        let bad_address = encode_sp_address(Network::Regtest, &parsed.b_scan, &ark_pub).unwrap();
        let broken = KeySources {
            sp_address: bad_address,
            ..sources.clone()
        };
        let err = WalletKeys::load(&broken).unwrap_err();
        assert!(matches!(err, KeyError::ArkKeyIsSpendKey), "{err}");
    }

    #[test]
    fn wrong_network_magic_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        // Provision on regtest (tprv), but the file claims mainnet
        // (xprv) - i.e. load with network = mainnet against a regtest
        // sp_address HRP mismatch is caught by address parsing first,
        // so check the magic check directly via a mainnet xprv file
        // loaded as regtest.
        let keys = spc_crypto::provision(&master(), Network::Mainnet).unwrap();
        let scan_path = dir.path().join("scan.xprv");
        write_secret(&scan_path, &keys.scan_branch.to_string());
        let err = load_branch_xprv(&scan_path, 1, "1'", Network::Regtest).unwrap_err();
        assert!(matches!(err, KeyError::Network { .. }), "{err}");
    }

    #[test]
    fn non_branch_shaped_xprv_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        // A master xprv (depth 0) is not a branch account.
        let root = Xpriv::new_master(bitcoin::Network::Regtest, &master()).unwrap();
        let path = dir.path().join("root.xprv");
        write_secret(&path, &root.to_string());
        let err = load_branch_xprv(&path, 1, "1'", Network::Regtest).unwrap_err();
        assert!(
            matches!(err, KeyError::BranchShape { depth: 0, .. }),
            "{err}"
        );

        // A depth-4 *unhardened* child is wrong too.
        let secp = Secp256k1::new();
        let deep = root
            .derive_priv(
                &secp,
                &[
                    ChildNumber::from_hardened_idx(352).unwrap(),
                    ChildNumber::from_hardened_idx(0).unwrap(),
                    ChildNumber::from_hardened_idx(0).unwrap(),
                    ChildNumber::from_normal_idx(1).unwrap(),
                ],
            )
            .unwrap();
        write_secret(&path, &deep.to_string());
        let err = load_branch_xprv(&path, 1, "1'", Network::Regtest).unwrap_err();
        assert!(
            matches!(err, KeyError::BranchShape { depth: 4, .. }),
            "{err}"
        );
    }

    /// The branch files must be the exact account indices 1'/2': a 3'
    /// account (depth 4, hardened, wrong index) is refused even though
    /// it is shape-plausible, while the valid fixture's real branches
    /// (1' scan, 2' ark) still load unchanged.
    #[test]
    fn wrong_branch_child_index_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let secp = Secp256k1::new();
        let root = Xpriv::new_master(bitcoin::Network::Regtest, &master()).unwrap();
        // m/352'/0'/0'/3' - the wrong branch (depth 4, hardened).
        let branch3 = root
            .derive_priv(
                &secp,
                &[
                    ChildNumber::from_hardened_idx(352).unwrap(),
                    ChildNumber::from_hardened_idx(0).unwrap(),
                    ChildNumber::from_hardened_idx(0).unwrap(),
                    ChildNumber::from_hardened_idx(3).unwrap(),
                ],
            )
            .unwrap();
        assert_eq!(branch3.depth, 4);
        assert!(branch3.child_number.is_hardened());
        let path = dir.path().join("branch3.xprv");
        write_secret(&path, &branch3.to_string());
        let err = load_branch_xprv(&path, 1, "1'", Network::Regtest).unwrap_err();
        assert!(
            matches!(err, KeyError::BranchShape { ref child, .. } if child.contains("3'")),
            "the 3' account must be refused for the scan branch: {err}"
        );
        let err = load_branch_xprv(&path, 2, "2'", Network::Regtest).unwrap_err();
        assert!(
            matches!(err, KeyError::BranchShape { ref child, .. } if child.contains("3'")),
            "the 3' account must be refused for the ark branch: {err}"
        );

        // The unchanged valid fixture: 1' for scan, 2' for ark.
        let sources = write_keys(dir.path(), &master(), Network::Regtest);
        load_branch_xprv(&sources.scan_xprv, 1, "1'", Network::Regtest).unwrap();
        load_branch_xprv(&sources.ark_xprv, 2, "2'", Network::Regtest).unwrap();
    }

    /// Scan and ark branch files from *different* masters are not
    /// siblings: the shared-parent-fingerprint metadata check fails
    /// closed (required provisioning consistency, not a cryptographic
    /// ancestry proof).
    #[test]
    fn branches_from_different_masters_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let sources = write_keys(dir.path(), &master(), Network::Regtest);
        // Replace the ark file with the ark branch of a *foreign*
        // master: depth 4, index 2', correct network - only the parent
        // fingerprint gives it away.
        let foreign_master = [0x77u8; 64];
        let foreign = spc_crypto::provision(&foreign_master, Network::Regtest).unwrap();
        write_secret(&sources.ark_xprv, &foreign.ark_branch.to_string());
        let err = WalletKeys::load(&sources).unwrap_err();
        assert!(
            matches!(err, KeyError::BranchesNotSiblings { .. }),
            "foreign-master branches must fail the sibling check: {err}"
        );
    }

    /// Best-effort key erasure: the branch secrets, the child seed and
    /// the mailbox/normal keypairs no longer derive the real keys after
    /// `zeroize_keys` (drop does the same).
    #[test]
    fn zeroize_scrubs_the_supported_key_surface() {
        let dir = tempfile::tempdir().unwrap();
        let sources = write_keys(dir.path(), &master(), Network::Regtest);
        let mut keys = WalletKeys::load(&sources).unwrap();
        let b_ark = keys.b_ark_public();
        let mailbox = keys.mailbox_public();
        let normal = keys.normal_vtxo_public();
        let fingerprint = keys.wallet_fingerprint;

        keys.zeroize_keys();
        assert_ne!(keys.b_ark_public(), b_ark, "ark branch secret scrubbed");
        assert_ne!(keys.mailbox_public(), mailbox, "mailbox keypair scrubbed");
        assert_ne!(keys.normal_vtxo_public(), normal, "normal keypair scrubbed");
        // Public metadata survives (it is not secret).
        assert_eq!(keys.wallet_fingerprint, fingerprint);
        // Idempotent, and drop after explicit zeroize is safe.
        keys.zeroize_keys();
    }

    #[test]
    fn child_seed_length_is_enforced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("short.seed");
        write_secret(&path, &hex::encode([0u8; 32]));
        let err = load_child_seed(&path).unwrap_err();
        assert!(matches!(err, KeyError::ParseSeed(_, _)), "{err}");
    }

    #[test]
    fn missing_files_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let sources = write_keys(dir.path(), &master(), Network::Regtest);
        let mut broken = sources.clone();
        broken.scan_xprv = dir.path().join("does-not-exist.xprv");
        let err = WalletKeys::load(&broken).unwrap_err();
        assert!(matches!(err, KeyError::Read(_, _)), "{err}");
        // Nothing was created by the failed load.
        assert!(!broken.scan_xprv.exists());
    }

    #[test]
    fn network_names_round_trip() {
        for name in ["mainnet", "testnet", "signet", "regtest"] {
            let network = parse_network_name(name).unwrap();
            assert_eq!(network_name(network), name);
        }
        assert!(parse_network_name("dogenet").is_none());
    }
}
