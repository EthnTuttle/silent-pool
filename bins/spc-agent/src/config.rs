//! Miner-side configuration (SPEC §6.3, §11.10).

use std::path::PathBuf;

use serde::Deserialize;
use spc_crypto::{parse_sp_address, Network};
use thiserror::Error;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentConfig {
    pub upstream: Upstream,
    #[serde(default)]
    pub downstream: Downstream,
    #[serde(default)]
    pub jd: Option<Jd>,
    #[serde(default)]
    pub ark: Option<Ark>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Upstream {
    pub address: String,
    pub authority_public_key: String,
    pub sp_address: String,
    #[serde(default = "default_require")]
    pub require_own_payout: String,
    #[serde(default = "default_true")]
    pub request_share_accounting: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Downstream {
    #[serde(default = "default_downstream")]
    pub listen: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Jd {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub tp_address: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ark {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_mailbox")]
    pub mailbox_mode: String,
    #[serde(default = "default_fallback")]
    pub epoch_fallback: String,
    #[serde(default = "default_signer")]
    pub signer: String,
    #[serde(default)]
    pub refresh_on_receive_above_sats: u64,
    #[serde(default)]
    pub offboard_on_exit: String,
    #[serde(default)]
    pub seed_child: Option<PathBuf>,
}

fn default_require() -> String {
    "warn".into()
}
fn default_true() -> bool {
    true
}
fn default_downstream() -> String {
    "0.0.0.0:34255".into()
}

impl Default for Downstream {
    fn default() -> Self {
        Self {
            listen: default_downstream(),
        }
    }
}
fn default_mailbox() -> String {
    "epoch_derived".into()
}
fn default_fallback() -> String {
    "static".into()
}
fn default_signer() -> String {
    "local".into()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnPayoutPolicy {
    Warn,
    Refuse,
}

impl OwnPayoutPolicy {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "warn" => Some(Self::Warn),
            "refuse" => Some(Self::Refuse),
            _ => None,
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("sp_address is not a valid address for the pool network")]
    InvalidAddress,
    #[error("require_own_payout must be 'warn' or 'refuse'")]
    Policy,
    #[error("unknown mailbox mode '{0}'")]
    Mailbox(String),
    #[error("unknown epoch fallback '{0}'")]
    Fallback(String),
    #[error("unknown signer '{0}'")]
    Signer(String),
    #[error("invalid listen address: {0}")]
    Listen(String),
}

impl AgentConfig {
    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let cfg: Self = toml::from_str(&text)?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn network(&self) -> Network {
        // v0.1 miners are testnet/signet/regtest; the HRP only distinguishes
        // mainnet (`sp`) from test networks (`tsp`).
        if self.upstream.sp_address.starts_with("sp1") {
            Network::Mainnet
        } else {
            Network::Testnet
        }
    }

    pub fn own_payout_policy(&self) -> Result<OwnPayoutPolicy, ConfigError> {
        OwnPayoutPolicy::parse(&self.upstream.require_own_payout).ok_or(ConfigError::Policy)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        let network = self.network();
        parse_sp_address(&self.upstream.sp_address, network)
            .map_err(|_| ConfigError::InvalidAddress)?;
        let _ = self.own_payout_policy()?;
        self.upstream
            .address
            .parse::<std::net::SocketAddr>()
            .map(|_| ())
            .map_err(|_| ConfigError::Listen(self.upstream.address.clone()))?;
        self.downstream
            .listen
            .parse::<std::net::SocketAddr>()
            .map(|_| ())
            .map_err(|_| ConfigError::Listen(self.downstream.listen.clone()))?;
        if let Some(ark) = &self.ark {
            if ark.enabled {
                match ark.mailbox_mode.as_str() {
                    "none" | "static" | "epoch_derived" | "epoch_registered" => {}
                    other => return Err(ConfigError::Mailbox(other.to_string())),
                }
                match ark.epoch_fallback.as_str() {
                    "static" | "hold" => {}
                    other => return Err(ConfigError::Fallback(other.to_string())),
                }
                match ark.signer.as_str() {
                    "local" | "external" => {}
                    other => return Err(ConfigError::Signer(other.to_string())),
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};

    fn address(network: Network) -> String {
        let secp = Secp256k1::new();
        let a = SecretKey::from_slice(&[3u8; 32]).unwrap();
        let b = SecretKey::from_slice(&[4u8; 32]).unwrap();
        spc_crypto::address::encode_sp_address(
            network,
            &PublicKey::from_secret_key(&secp, &a),
            &PublicKey::from_secret_key(&secp, &b),
        )
        .unwrap()
    }

    fn sample() -> AgentConfig {
        toml::from_str(&format!(
            r#"
[upstream]
address = "127.0.0.1:34254"
authority_public_key = "02aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
sp_address = "{}"
require_own_payout = "refuse"

[downstream]
listen = "127.0.0.1:34255"

[ark]
enabled = true
mailbox_mode = "epoch_derived"
"#,
            address(Network::Testnet)
        ))
        .unwrap()
    }

    #[test]
    fn validates_a_miner_config() {
        let cfg = sample();
        cfg.validate().unwrap();
        assert_eq!(cfg.own_payout_policy().unwrap(), OwnPayoutPolicy::Refuse);
    }

    #[test]
    fn rejects_bare_address() {
        let mut cfg = sample();
        cfg.upstream.sp_address = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq".into();
        assert_eq!(cfg.validate(), Err(ConfigError::InvalidAddress));
    }

    #[test]
    fn rejects_bad_policy_and_mailbox() {
        let mut cfg = sample();
        cfg.upstream.require_own_payout = "silent".into();
        assert_eq!(cfg.validate(), Err(ConfigError::Policy));

        let mut cfg = sample();
        cfg.ark.as_mut().unwrap().mailbox_mode = "per-minute".into();
        assert_eq!(
            cfg.validate(),
            Err(ConfigError::Mailbox("per-minute".into()))
        );
    }
}
