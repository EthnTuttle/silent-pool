//! Real legacy (P2PKH) traffic key material, fully separate from SPc master,
//! child and Ark keys (traffic plan sections 5.1 and 6.2).
//!
//! **Key mode: `lab_deterministic_run_seed_v1` (regtest-only, declared).**
//! All traffic keys are deterministically derived from the *public* scenario
//! `RunSpec.seed` plus the run id. A public run seed means the keys are
//! **publicly reproducible**: the mode-0600 key file protects nothing the
//! scenario does not already reveal, and no secrecy or privacy claim is made
//! for these keys anywhere in this crate. Production (non-regtest) use is
//! refused outright by the derivation itself.
//!
//! Derivation (version 2, recorded in the key file and the journal header):
//!
//! - per-role master seed:
//!   `HMAC-SHA256(key = run_seed big-endian, msg = run_id || 0x00 || role_label)`
//!   followed by a BIP32 tree. The explicit `run_id` in the HMAC domain means
//!   a repeated seed across runs never reuses wallet keys, while a fixture
//!   that fixes the run id stays byte-reproducible.
//!   (Version 1 - `HMAC(run_seed, label)` without the run id - was the
//!   unapproved candidate scheme and is superseded; it is refused on reopen.)
//! - receive scripts: `m/0'/{index}'`, change scripts: `m/1'/{index}'`
//!   (hardened children, one BIP32 tree per wallet).
//!
//! The three wallet populations are disjoint by construction: the bootstrap
//! funding wallet, the background network wallets and the SPc user payment
//! wallets use different derivation labels (with globally disjoint role
//! index ranges per actor, validated before provisioning), and none of them
//! ever touches `spc-scan` identity material (`b_scan`/`b_spend`) or Ark
//! keys.
//!
//! Private provisioning writes a single mode-0600 file under the run's
//! private directory and reopens it fail-closed: a corrupt file, a network
//! mismatch, a wrong run id or key mode, or a persisted master that does not
//! byte-equal re-derivation from the run seed is a hard error - never a
//! silent regeneration.
//!
//! No secret ever reaches a `Debug`, `Display`, log line or status record:
//! `TrafficPrivateKey` and `ResolvedKey` print only role names, addresses and
//! key fingerprints.

use std::collections::HashMap;
use std::fmt;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use bitcoin::bip32::{ChildNumber, Xpriv};
use bitcoin::hashes::sha256::Hash as Sha256;
use bitcoin::hashes::Hash;
use bitcoin::secp256k1::{Secp256k1, SecretKey};
use bitcoin::{Address, Network, PublicKey, ScriptBuf};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256 as Sha256Engine;
use zeroize::Zeroize;

use crate::error::KeyError;

/// Derivation label for the single bootstrap funding wallet (plan section 5.1).
pub const LABEL_BOOTSTRAP_FUNDING: &str = "spc-lab/traffic/bootstrap-funding";

/// Derivation label prefix for background network wallets (plan section 6.2).
pub const LABEL_NETWORK_WALLET: &str = "spc-lab/traffic/network-wallet";

/// Derivation label prefix for SPc user payment wallets (plan section 6.2).
pub const LABEL_USER_PAYMENT_WALLET: &str = "spc-lab/traffic/user-payment-wallet";

/// How deep `key_for_script` scans the receive/change chains when resolving
/// an observed script back to its private key. This bound only serves
/// destination owner tagging (`checked_destination`); the spending path
/// resolves keys by direct `(chain, index)` derivation (M2) and is not
/// bounded by the lookahead.
pub const DEFAULT_SCRIPT_LOOKAHEAD: u32 = 512;

/// The declared key mode of this core (M5): deterministic from the public
/// run seed plus run id, regtest-only, no secrecy claim.
pub const KEY_MODE: &str = "lab_deterministic_run_seed_v1";

/// The derivation scheme version recorded in the key file and the journal
/// header. Version 2 puts the explicit `run_id` in the HMAC domain.
pub const DERIVATION_VERSION: u32 = 2;

/// Validate a run id for use in the HMAC domain and the v2 journal
/// header, aligned with the approved T0 id vocabulary (read-only
/// adoption): non-empty, at most 128 bytes, and only ASCII alphanumeric
/// or `-_.:/@` (no control characters, no NUL, no spaces). The same
/// rules apply to the driver id.
pub fn validate_run_id(run_id: &str) -> Result<(), KeyError> {
    let invalid = || KeyError::InvalidRunId(run_id.to_string());
    if run_id.is_empty() || run_id.len() > 128 {
        return Err(invalid());
    }
    if !run_id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':' | b'/' | b'@'))
    {
        return Err(invalid());
    }
    Ok(())
}

/// One traffic wallet role. Roles are unique per driver: a wallet/outpoint
/// always has exactly one owning role, and reservations check ownership.
///
/// Note: internally-tagged serde enums cannot carry `deny_unknown_fields`
/// (same constraint as the engine's `CommandKind`); unknown variants are
/// still rejected by the tag itself.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WalletRole {
    BootstrapFunding,
    NetworkWallet { index: u32 },
    UserPayment { index: u32 },
}

impl WalletRole {
    /// The exact HMAC derivation label for this role.
    pub fn derivation_label(&self) -> String {
        match self {
            Self::BootstrapFunding => LABEL_BOOTSTRAP_FUNDING.to_string(),
            Self::NetworkWallet { index } => format!("{LABEL_NETWORK_WALLET}/{index}"),
            Self::UserPayment { index } => format!("{LABEL_USER_PAYMENT_WALLET}/{index}"),
        }
    }

    /// A stable, human-readable name (contains no secrets).
    pub fn name(&self) -> String {
        match self {
            Self::BootstrapFunding => "bootstrap-funding".to_string(),
            Self::NetworkWallet { index } => format!("network-wallet-{index}"),
            Self::UserPayment { index } => format!("user-payment-wallet-{index}"),
        }
    }

    /// The short kind name used in error messages.
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::BootstrapFunding => "bootstrap_funding",
            Self::NetworkWallet { .. } => "network_wallet",
            Self::UserPayment { .. } => "user_payment",
        }
    }
}

impl fmt::Display for WalletRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name())
    }
}

/// The receive/change chain of a derived script.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Chain {
    Receive,
    Change,
}

/// A shareable derived script: address, script and public key, no secret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddressScript {
    pub chain: Chain,
    pub index: u32,
    pub public_key: PublicKey,
    pub script_pubkey: ScriptBuf,
    pub address: Address,
}

/// A resolved private key. `Debug` prints no secret material.
#[derive(Clone)]
pub struct ResolvedKey {
    pub secret_key: SecretKey,
    pub public_key: PublicKey,
    pub chain: Chain,
    pub index: u32,
}

impl fmt::Debug for ResolvedKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResolvedKey")
            .field("chain", &self.chain)
            .field("index", &self.index)
            .field("public_key", &self.public_key)
            .finish_non_exhaustive()
    }
}

/// A per-role BIP32 master key for traffic spending.
///
/// `Debug` prints the role and fingerprint only. The underlying `Xpriv`
/// cannot be zeroized in bitcoin 0.32 (recorded residual, same as the
/// `PoolAuthority` precedent in `spc-lab`); every derived seed and encoded
/// string this type touches is zeroized on drop.
pub struct TrafficPrivateKey {
    xpriv: Xpriv,
    network: Network,
    role: WalletRole,
}

impl fmt::Debug for TrafficPrivateKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TrafficPrivateKey")
            .field("role", &self.role)
            .field("network", &self.network)
            .finish_non_exhaustive()
    }
}

impl TrafficPrivateKey {
    /// Derive the role master from the run seed and run id:
    /// `HMAC-SHA256(key = run_seed big-endian,
    ///              msg = run_id || 0x00 || role_label)`
    /// is the BIP32 seed (32 bytes / 256 bits, within BIP32's 128-512 bit
    /// range). The run id is inside the HMAC domain, so the same public
    /// seed replayed in another run derives different wallet keys, and the
    /// keys remain publicly reproducible from (seed, run_id) - by design of
    /// the declared lab key mode (see the module docs).
    pub fn derive_from_run_seed(
        run_seed: u64,
        run_id: &str,
        role: &WalletRole,
        network: Network,
    ) -> Result<Self, KeyError> {
        if network != Network::Regtest {
            return Err(KeyError::UnsupportedNetwork(format!("{network:?}")));
        }
        validate_run_id(run_id)?;
        let label = role.derivation_label();
        let mut mac = Hmac::<Sha256Engine>::new_from_slice(&run_seed.to_be_bytes())
            .expect("hmac accepts any key length");
        mac.update(run_id.as_bytes());
        // 0x00 domain separator: labels never contain NUL, so the split of
        // the message into (run id, label) is unambiguous.
        mac.update(&[0x00]);
        mac.update(label.as_bytes());
        let mut seed = mac.finalize().into_bytes();
        let xpriv =
            Xpriv::new_master(network, &seed).map_err(|e| KeyError::Derivation(e.to_string()))?;
        let key = Self {
            xpriv,
            network,
            role: role.clone(),
        };
        seed.zeroize();
        Ok(key)
    }

    /// The BIP32 fingerprint (hash160 of the master public key), hex-encoded.
    pub fn fingerprint_hex(&self) -> String {
        let secp = Secp256k1::new();
        hex::encode(self.xpriv.fingerprint(&secp).to_bytes())
    }

    /// A stable fingerprint of the run seed itself (for reopen binding).
    pub fn run_seed_fingerprint(run_seed: u64) -> String {
        hex::encode(Sha256::hash(&run_seed.to_be_bytes()).to_byte_array())
    }

    pub fn role(&self) -> &WalletRole {
        &self.role
    }

    pub fn network(&self) -> Network {
        self.network
    }

    /// The persisted private encoding of this master (base58check xprv).
    pub fn master_xpriv_base58(&self) -> String {
        self.xpriv.to_string()
    }

    /// Parse a persisted master encoding, failing closed on any mismatch.
    ///
    /// Testnet and regtest share extended-key version bytes in bitcoin 0.32
    /// (`tprv`, `NetworkKind::Test`), so the byte-level network check accepts
    /// the test kind for both and the regtest binding is enforced by the
    /// persisted file's `network` field plus the full master-encoding
    /// comparison in [`TrafficKeyStore::reopen`].
    pub fn from_persisted(
        encoded: &str,
        role: &WalletRole,
        expected_network: Network,
    ) -> Result<Self, KeyError> {
        let xpriv: Xpriv = encoded
            .parse()
            .map_err(|e| KeyError::CorruptKeyFile(format!("xprv parse: {e}")))?;
        let expected_main = expected_network == Network::Bitcoin;
        let actual_main = xpriv.network == bitcoin::NetworkKind::Main;
        let prefix_ok = if expected_main {
            encoded.starts_with("xprv")
        } else {
            encoded.starts_with("tprv")
        };
        if actual_main != expected_main || !prefix_ok {
            return Err(KeyError::WrongNetwork {
                stored: if actual_main {
                    "bitcoin".to_string()
                } else {
                    "test (testnet/regtest kind)".to_string()
                },
                expected: network_string(expected_network),
            });
        }
        Ok(Self {
            xpriv,
            network: expected_network,
            role: role.clone(),
        })
    }

    fn child(
        &self,
        secp: &Secp256k1<bitcoin::secp256k1::All>,
        chain: Chain,
        index: u32,
    ) -> Result<(SecretKey, PublicKey), KeyError> {
        let chain_number = match chain {
            Chain::Receive => 0,
            Chain::Change => 1,
        };
        let path = vec![
            ChildNumber::from_hardened_idx(chain_number)
                .map_err(|e| KeyError::Derivation(e.to_string()))?,
            ChildNumber::from_hardened_idx(index)
                .map_err(|e| KeyError::Derivation(e.to_string()))?,
        ];
        let child = self
            .xpriv
            .derive_priv(secp, &path)
            .map_err(|e| KeyError::Derivation(e.to_string()))?;
        let secret = child.to_priv().inner;
        let public = PublicKey::new(secp256k1::PublicKey::from_secret_key(secp, &secret));
        Ok((secret, public))
    }
}

/// One traffic wallet: a role, its master key and a script-resolution cache.
pub struct TrafficWallet {
    role: WalletRole,
    network: Network,
    master: TrafficPrivateKey,
    secp: Secp256k1<bitcoin::secp256k1::All>,
    lookahead: u32,
    cache: Mutex<HashMap<ScriptBuf, ResolvedKey>>,
}

impl fmt::Debug for TrafficWallet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TrafficWallet")
            .field("role", &self.role)
            .field("network", &self.network)
            .field("lookahead", &self.lookahead)
            .finish_non_exhaustive()
    }
}

impl TrafficWallet {
    pub fn new(
        run_seed: u64,
        run_id: &str,
        role: WalletRole,
        network: Network,
        lookahead: u32,
    ) -> Result<Self, KeyError> {
        let master = TrafficPrivateKey::derive_from_run_seed(run_seed, run_id, &role, network)?;
        Ok(Self {
            role,
            network,
            master,
            secp: Secp256k1::new(),
            lookahead,
            cache: Mutex::new(HashMap::new()),
        })
    }

    pub fn role(&self) -> &WalletRole {
        &self.role
    }

    pub fn network(&self) -> Network {
        self.network
    }

    pub fn master_fingerprint_hex(&self) -> String {
        self.master.fingerprint_hex()
    }

    /// The address/script at a chain index (no secret material returned).
    pub fn script_at(&self, chain: Chain, index: u32) -> Result<AddressScript, KeyError> {
        let (_secret, public) = self.master.child(&self.secp, chain, index)?;
        let pubkey_hash = public.pubkey_hash();
        let address = Address::p2pkh(pubkey_hash, self.network);
        Ok(AddressScript {
            chain,
            index,
            public_key: public,
            script_pubkey: address.script_pubkey(),
            address,
        })
    }

    /// Resolve the private key at an exact `(chain, index)` by direct
    /// derivation - no scan. This is the **spending path** (M2): an owned
    /// outpoint carries its `(chain, index)` in the journal, so the key is
    /// resolved by construction, and the caller additionally requires the
    /// derived script to equal the recorded script before signing.
    pub fn resolved_key_at(&self, chain: Chain, index: u32) -> Result<ResolvedKey, KeyError> {
        let (secret_key, public_key) = self.master.child(&self.secp, chain, index)?;
        Ok(ResolvedKey {
            secret_key,
            public_key,
            chain,
            index,
        })
    }

    /// Resolve an observed script back to its private key, scanning the
    /// receive and change chains up to the lookahead bound. This serves
    /// **destination owner tagging only** (crediting an inbound payment to
    /// a role); the spending path never scans - it resolves keys directly
    /// by `(chain, index)` via [`TrafficWallet::resolved_key_at`]. Foreign
    /// scripts and scripts beyond the bound fail closed.
    pub fn key_for_script(&self, script: &ScriptBuf) -> Result<ResolvedKey, KeyError> {
        if let Some(resolved) = self.cache.lock().unwrap().get(script) {
            return Ok(resolved.clone());
        }
        for chain in [Chain::Receive, Chain::Change] {
            for index in 0..self.lookahead {
                let (secret, public) = self.master.child(&self.secp, chain, index)?;
                let address = Address::p2pkh(public.pubkey_hash(), self.network);
                let candidate = address.script_pubkey();
                if &candidate == script {
                    let resolved = ResolvedKey {
                        secret_key: secret,
                        public_key: public,
                        chain,
                        index,
                    };
                    self.cache
                        .lock()
                        .unwrap()
                        .insert(script.clone(), resolved.clone());
                    return Ok(resolved);
                }
            }
        }
        Err(KeyError::ScriptNotOwned {
            lookahead: self.lookahead,
        })
    }

    /// Whether `script` is owned by this wallet (key resolution succeeds).
    pub fn owns_script(&self, script: &ScriptBuf) -> bool {
        self.key_for_script(script).is_ok()
    }
}

/// The persisted private wallet-set record (mode 0600, fsynced). The
/// `key_mode`/`derivation_version`/`run_id` fields bind the file to the
/// declared lab key mode and the exact HMAC domain of the derivation
/// (M5): a file written under any other mode, version or run id is
/// refused on reopen, never re-derived around.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrivateWalletFile {
    history_version: u32,
    key_mode: String,
    derivation_version: u32,
    network: String,
    run_id: String,
    run_seed_fingerprint: String,
    wallets: Vec<PersistedWallet>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedWallet {
    role: WalletRole,
    master_xpriv: String,
    fingerprint: String,
}

/// Private key store: provisions the wallet set once and reopens it
/// fail-closed. The store is bound to the exact `run_id` it verified on
/// open; wallets it loads can only be derived under that run id.
pub struct TrafficKeyStore {
    path: PathBuf,
    network: Network,
    run_id: String,
    run_seed_fingerprint: String,
    roles: Vec<WalletRole>,
}

impl fmt::Debug for TrafficKeyStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TrafficKeyStore")
            .field("path", &self.path)
            .field("network", &self.network)
            .field("run_id", &self.run_id)
            .field("roles", &self.roles)
            .finish_non_exhaustive()
    }
}

impl TrafficKeyStore {
    /// The private artifact file name under the run private directory.
    pub const FILE_NAME: &'static str = "traffic-wallets.json";

    /// Provision the wallet-set key file exactly once, bound to the run
    /// id. An existing file is never overwritten.
    pub fn provision(
        run_seed: u64,
        run_id: &str,
        network: Network,
        roles: &[WalletRole],
        private_dir: &Path,
    ) -> Result<Self, KeyError> {
        validate_run_id(run_id)?;
        let path = private_dir.join(Self::FILE_NAME);
        if path.exists() {
            return Err(KeyError::Io(format!(
                "private key file {} already exists; traffic wallets are provisioned exactly once",
                path.display()
            )));
        }
        std::fs::create_dir_all(private_dir)
            .map_err(|e| KeyError::Io(format!("create {}: {e}", private_dir.display())))?;
        let mut wallets = Vec::new();
        for role in roles {
            let master = TrafficPrivateKey::derive_from_run_seed(run_seed, run_id, role, network)?;
            let mut encoded = master.master_xpriv_base58();
            wallets.push(PersistedWallet {
                role: role.clone(),
                master_xpriv: encoded.clone(),
                fingerprint: master.fingerprint_hex(),
            });
            encoded.zeroize();
        }
        let file_body = PrivateWalletFile {
            history_version: 1,
            key_mode: KEY_MODE.to_string(),
            derivation_version: DERIVATION_VERSION,
            network: network_string(network),
            run_id: run_id.to_string(),
            run_seed_fingerprint: TrafficPrivateKey::run_seed_fingerprint(run_seed),
            wallets,
        };
        let mut body = serde_json::to_vec_pretty(&file_body)
            .map_err(|e| KeyError::Io(format!("serialize key file: {e}")))?;
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| KeyError::Io(format!("create {}: {e}", path.display())))?;
        file.write_all(&body)
            .and_then(|_| file.sync_all())
            .map_err(|e| KeyError::Io(format!("write {}: {e}", path.display())))?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| KeyError::Io(format!("chmod {}: {e}", path.display())))?;
        body.zeroize();
        Ok(Self {
            path,
            network,
            run_id: run_id.to_string(),
            run_seed_fingerprint: file_body.run_seed_fingerprint,
            roles: roles.to_vec(),
        })
    }

    /// Reopen a provisioned key file, verifying it against the same run
    /// seed, run id, declared key mode and network. Corrupt files, run-id,
    /// key-mode, derivation-version, network and seed-fingerprint
    /// mismatches fail closed, and every persisted master encoding must
    /// **string-equal** re-derivation from the run seed (L4: a 32-bit
    /// BIP32 fingerprint alone is not proof of possession); a seed is
    /// never regenerated.
    pub fn reopen(
        run_seed: u64,
        run_id: &str,
        network: Network,
        private_dir: &Path,
    ) -> Result<Self, KeyError> {
        validate_run_id(run_id)?;
        let path = private_dir.join(Self::FILE_NAME);
        let body = std::fs::read(&path)
            .map_err(|e| KeyError::CorruptKeyFile(format!("read {}: {e}", path.display())))?;
        let file: PrivateWalletFile = serde_json::from_slice(&body)
            .map_err(|e| KeyError::CorruptKeyFile(format!("parse {}: {e}", path.display())))?;
        if file.history_version != 1 {
            return Err(KeyError::CorruptKeyFile(format!(
                "unsupported key file history_version {}",
                file.history_version
            )));
        }
        if file.key_mode != KEY_MODE {
            return Err(KeyError::KeyModeMismatch {
                stored: file.key_mode,
                expected: KEY_MODE.to_string(),
            });
        }
        if file.derivation_version != DERIVATION_VERSION {
            return Err(KeyError::BindingMismatch {
                field: "derivation_version".to_string(),
                stored: file.derivation_version.to_string(),
                expected: DERIVATION_VERSION.to_string(),
            });
        }
        if file.run_id != run_id {
            return Err(KeyError::BindingMismatch {
                field: "run_id".to_string(),
                stored: file.run_id,
                expected: run_id.to_string(),
            });
        }
        if file.network != network_string(network) {
            return Err(KeyError::WrongNetwork {
                stored: file.network,
                expected: network_string(network),
            });
        }
        let expected_fingerprint = TrafficPrivateKey::run_seed_fingerprint(run_seed);
        if file.run_seed_fingerprint != expected_fingerprint {
            return Err(KeyError::FingerprintMismatch {
                role: "run_seed".to_string(),
                stored: file.run_seed_fingerprint,
                derived: expected_fingerprint,
            });
        }
        let mut roles = Vec::new();
        for wallet in &file.wallets {
            let master =
                TrafficPrivateKey::derive_from_run_seed(run_seed, run_id, &wallet.role, network)?;
            if master.fingerprint_hex() != wallet.fingerprint {
                return Err(KeyError::FingerprintMismatch {
                    role: wallet.role.name(),
                    stored: wallet.fingerprint.clone(),
                    derived: master.fingerprint_hex(),
                });
            }
            // The persisted xprv must parse and match the network.
            let parsed =
                TrafficPrivateKey::from_persisted(&wallet.master_xpriv, &wallet.role, network)?;
            if parsed.fingerprint_hex() != wallet.fingerprint {
                return Err(KeyError::FingerprintMismatch {
                    role: wallet.role.name(),
                    stored: wallet.fingerprint.clone(),
                    derived: parsed.fingerprint_hex(),
                });
            }
            // L4: full-encoding comparison. The re-derived xprv string
            // must equal the persisted string exactly; only SHA-256
            // digests of the two encodings enter the error, never the
            // encodings themselves.
            let mut derived_encoding = master.master_xpriv_base58();
            let encoding_matches = derived_encoding == wallet.master_xpriv;
            let stored_digest = Sha256::hash(wallet.master_xpriv.as_bytes()).to_string();
            let derived_digest = Sha256::hash(derived_encoding.as_bytes()).to_string();
            derived_encoding.zeroize();
            if !encoding_matches {
                return Err(KeyError::FingerprintMismatch {
                    role: format!("{} (master encoding)", wallet.role.name()),
                    stored: stored_digest,
                    derived: derived_digest,
                });
            }
            roles.push(wallet.role.clone());
        }
        Ok(Self {
            path,
            network,
            run_id: run_id.to_string(),
            run_seed_fingerprint: expected_fingerprint,
            roles,
        })
    }

    /// Build the live in-memory wallets for the reopened set, derived
    /// under the run id this store was verified against.
    pub fn load_wallets(
        &self,
        run_seed: u64,
        lookahead: u32,
    ) -> Result<HashMap<WalletRole, TrafficWallet>, KeyError> {
        let mut wallets = HashMap::new();
        for role in &self.roles {
            let wallet = TrafficWallet::new(
                run_seed,
                &self.run_id,
                role.clone(),
                self.network,
                lookahead,
            )?;
            wallets.insert(role.clone(), wallet);
        }
        Ok(wallets)
    }

    pub fn roles(&self) -> &[WalletRole] {
        &self.roles
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn run_seed_fingerprint(&self) -> &str {
        &self.run_seed_fingerprint
    }
}

fn network_string(network: Network) -> String {
    match network {
        Network::Regtest => "regtest".to_string(),
        Network::Testnet => "testnet".to_string(),
        Network::Testnet4 => "testnet4".to_string(),
        Network::Signet => "signet".to_string(),
        Network::Bitcoin => "bitcoin".to_string(),
    }
}
