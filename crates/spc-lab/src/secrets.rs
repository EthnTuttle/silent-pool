//! Per-pool Noise authority material.
//!
//! L1 requires a front to start with a matching base58check
//! `authority_secret_key`/`authority_public_key` pair. The lab generates a
//! distinct pair for every pool and writes it only to a private, mode-0600
//! artifact. The shareable manifest never receives the secret.

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use bs58;
use secp256k1::{PublicKey, Secp256k1, SecretKey, XOnlyPublicKey};
use serde::{Deserialize, Serialize};

use crate::ids::PoolId;

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("secret io: {0}")]
    Io(String),
    #[error("secret key generation failed: {0}")]
    Key(String),
}

/// One pool's authority pair. The secret is zeroed on drop.
pub struct PoolAuthority {
    secret: SecretKey,
    public: XOnlyPublicKey,
    public_key_base58: String,
    secret_file: PathBuf,
}

impl PoolAuthority {
    pub fn generate(pool: &PoolId, run_dir: &Path) -> Result<Self, SecretError> {
        let secp = Secp256k1::new();
        let mut rng = rand::thread_rng();
        let (secret, _public) = secp.generate_keypair(&mut rng);
        let public =
            XOnlyPublicKey::from_keypair(&secp256k1::Keypair::from_secret_key(&secp, &secret)).0;
        let public_key_base58 = encode_public(public);
        let secret_file = run_dir
            .join("private")
            .join(format!("pool-authority-{pool}.json"));
        let mut authority = Self {
            secret,
            public,
            public_key_base58,
            secret_file,
        };
        authority.write_private()?;
        Ok(authority)
    }

    pub fn secret_base58(&self) -> String {
        encode_secret(self.secret)
    }

    pub fn public_base58(&self) -> &str {
        &self.public_key_base58
    }

    pub fn public_hex(&self) -> String {
        hex::encode(self.public.serialize())
    }

    pub fn secret_file(&self) -> &Path {
        &self.secret_file
    }

    /// A fragment that the real pool config can merge; the runner does not
    /// invent the rest of the pool's TOML.
    pub fn config_fragment(&self) -> String {
        format!(
            "[pool]\nauthority_secret_key = \"{}\"\nauthority_public_key = \"{}\"\n",
            self.secret_base58(),
            self.public_key_base58
        )
    }

    fn write_private(&mut self) -> Result<(), SecretError> {
        if let Some(parent) = self.secret_file.parent() {
            std::fs::create_dir_all(parent).map_err(|e| SecretError::Io(e.to_string()))?;
        }
        let body = serde_json::to_vec_pretty(&PrivateAuthority {
            pool: self.secret_file.display().to_string(),
            secret_base58: self.secret_base58(),
            public_base58: self.public_key_base58.clone(),
            public_hex: self.public_hex(),
        })
        .map_err(|e| SecretError::Io(e.to_string()))?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&self.secret_file)
            .map_err(|e| SecretError::Io(e.to_string()))?;
        file.write_all(&body)
            .and_then(|_| file.sync_all())
            .map_err(|e| SecretError::Io(e.to_string()))?;
        // The companion fragment is also private and mode-0600.
        let fragment_path = self.secret_file.with_extension("toml.fragment");
        let mut fragment = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&fragment_path)
            .map_err(|e| SecretError::Io(e.to_string()))?;
        fragment
            .write_all(self.config_fragment().as_bytes())
            .and_then(|_| fragment.sync_all())
            .map_err(|e| SecretError::Io(e.to_string()))?;
        Ok(())
    }
}

impl Drop for PoolAuthority {
    fn drop(&mut self) {
        self.secret.non_secure_erase();
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrivateAuthority {
    pool: String,
    secret_base58: String,
    public_base58: String,
    public_hex: String,
}

fn encode_secret(secret: SecretKey) -> String {
    bs58::encode(secret.secret_bytes())
        .with_check()
        .into_string()
}

fn encode_public(public: XOnlyPublicKey) -> String {
    let mut payload = [0u8; 34];
    payload[0] = 1;
    payload[2..].copy_from_slice(&public.serialize());
    bs58::encode(payload).with_check().into_string()
}

/// A public summary safe for a shareable manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicAuthority {
    pub pool_id: PoolId,
    pub authority_public_key: String,
    pub public_hex: String,
}

impl From<(&PoolId, &PoolAuthority)> for PublicAuthority {
    fn from((pool, authority): (&PoolId, &PoolAuthority)) -> Self {
        Self {
            pool_id: pool.clone(),
            authority_public_key: authority.public_base58().to_string(),
            public_hex: authority.public_hex(),
        }
    }
}

pub fn decode_public_for_test(value: &str) -> Option<PublicKey> {
    let decoded = bs58::decode(value).with_check(None).into_vec().ok()?;
    if decoded.len() < 34 {
        return None;
    }
    let xonly = XOnlyPublicKey::from_slice(&decoded[2..]).ok()?;
    Some(PublicKey::from_x_only_public_key(
        xonly,
        secp256k1::Parity::Even,
    ))
}
