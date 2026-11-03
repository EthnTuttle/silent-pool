//! Miner-side configuration (SPEC §6.3, §11.10).

use std::net::SocketAddr;
use std::path::PathBuf;

use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
use serde::Deserialize;
use spc_crypto::{parse_sp_address, Network};
use spc_pool::net::{Net, Target};
use thiserror::Error;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentConfig {
    pub upstream: Upstream,
    #[serde(default)]
    pub net: Net,
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
    /// JDS listener, for example `127.0.0.1:34264`.
    #[serde(default)]
    pub jds_address: String,
    /// Hex-encoded 32-byte `b_scan` secret key. Required by the JDC role for
    /// own-payout verification (SPEC §3.6); it must match `sp_address`.
    #[serde(default)]
    pub scan_secret_key: Option<String>,
    /// Template height. Set by a live Template Provider once one is wired;
    /// the current build requires it in the config to declare a job.
    #[serde(default)]
    pub height: Option<u32>,
    /// `coinbase_tx_value_remaining` (`T`) for the declared template.
    #[serde(default)]
    pub coinbase_value: Option<u64>,
    /// Bytes the miner rolls in the scriptSig after the BIP 34 height push.
    /// Bytes the miner rolls in the scriptSig after the BIP 34 height push.
    #[serde(default = "default_extranonce_len")]
    pub extranonce_len: usize,
    /// Bounded wait for the declaration answer (SPEC §3.5 "JDC fallback"),
    /// seconds. Default 10; minimum 1 - 0 is a startup error, never a clamp.
    #[serde(default = "default_declare_timeout_secs")]
    pub declare_timeout_secs: u64,
    #[serde(default = "default_user_identifier")]
    pub user_identifier: String,
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
fn default_extranonce_len() -> usize {
    8
}
fn default_declare_timeout_secs() -> u64 {
    10
}
fn default_user_identifier() -> String {
    "spc-agent".into()
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
    #[error("invalid upstream address: {0} (expected host:port or IP:port)")]
    UpstreamAddress(String),
    #[error("invalid [net] proxy: {0}")]
    Proxy(String),
    #[error("[{0}] resolves to a .onion host but [net] proxy is not set; there is no direct route (SPEC §3.8)")]
    OnionWithoutProxy(&'static str),
    #[error("invalid jds_address: {0}")]
    JdsAddress(String),
    #[error("invalid tp_address: {0}")]
    TpAddress(String),
    #[error("scan_secret_key must be a 32-byte hex secp256k1 secret key")]
    ScanKey,
    #[error("scan_secret_key does not match the b_scan in sp_address")]
    ScanKeyMismatch,
    #[error("[jd] needs height and coinbase_value until a Template Provider is wired")]
    JdTemplate,
    #[error("[jd] declare_timeout_secs must be at least 1 (fail-closed minimum, SPEC §3.5)")]
    DeclareTimeout,
}

impl Jd {
    /// The JDS dial target. Names stay unresolved so a proxied dial never
    /// resolves locally (SPEC §3.8).
    pub fn jds_target(&self) -> Result<Target, ConfigError> {
        Target::parse(&self.jds_address)
            .ok_or_else(|| ConfigError::JdsAddress(self.jds_address.clone()))
    }

    pub fn scan_secret(&self) -> Result<SecretKey, ConfigError> {
        let text = self
            .scan_secret_key
            .as_deref()
            .ok_or(ConfigError::ScanKey)?
            .trim();
        let text = text.strip_prefix("0x").unwrap_or(text);
        let bytes = hex::decode(text).map_err(|_| ConfigError::ScanKey)?;
        SecretKey::from_slice(&bytes).map_err(|_| ConfigError::ScanKey)
    }

    /// `(height, coinbase_tx_value_remaining, extranonce_len)`.
    pub fn template(&self) -> Result<(u32, u64, usize), ConfigError> {
        let height = self.height.ok_or(ConfigError::JdTemplate)?;
        let value = self.coinbase_value.ok_or(ConfigError::JdTemplate)?;
        Ok((height, value, self.extranonce_len))
    }
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

    /// The upstream SV2 dial target; a hostname stays unresolved so the
    /// SOCKS proxy can resolve it remotely (SPEC §3.8).
    pub fn upstream_target(&self) -> Result<Target, ConfigError> {
        Target::parse(&self.upstream.address)
            .ok_or_else(|| ConfigError::UpstreamAddress(self.upstream.address.clone()))
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        let network = self.network();
        let parsed = parse_sp_address(&self.upstream.sp_address, network)
            .map_err(|_| ConfigError::InvalidAddress)?;
        let _ = self.own_payout_policy()?;
        let egress = self
            .net
            .egress()
            .map_err(|e| ConfigError::Proxy(e.to_string()))?;
        if self.upstream_target()?.is_onion() && egress.proxy().is_none() {
            return Err(ConfigError::OnionWithoutProxy("upstream.address"));
        }
        self.downstream
            .listen
            .parse::<std::net::SocketAddr>()
            .map(|_| ())
            .map_err(|_| ConfigError::Listen(self.downstream.listen.clone()))?;
        if let Some(jd) = &self.jd {
            if jd.enabled {
                let jds = jd.jds_target()?;
                if jds.is_onion() && egress.proxy().is_none() {
                    return Err(ConfigError::OnionWithoutProxy("jd.jds_address"));
                }
                // The miner's own Template Provider is on the trusted side of
                // the boundary (like the pool's template source): never proxied,
                // and stays an IP literal.
                if !jd.tp_address.is_empty() {
                    jd.tp_address
                        .parse::<SocketAddr>()
                        .map(|_| ())
                        .map_err(|_| ConfigError::TpAddress(jd.tp_address.clone()))?;
                }
                let b_scan = jd.scan_secret()?;
                let derived = PublicKey::from_secret_key(&Secp256k1::new(), &b_scan);
                if derived != parsed.b_scan {
                    return Err(ConfigError::ScanKeyMismatch);
                }
                let _ = jd.template()?;
                if jd.declare_timeout_secs == 0 {
                    return Err(ConfigError::DeclareTimeout);
                }
            }
        }
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

    fn with_jd() -> AgentConfig {
        let mut cfg = sample();
        cfg.jd = Some(Jd {
            enabled: true,
            tp_address: "127.0.0.1:48333".into(),
            jds_address: "127.0.0.1:34264".into(),
            // `address()` uses `[3; 32]` as the scan key.
            scan_secret_key: Some(hex::encode([3u8; 32])),
            height: Some(800_000),
            coinbase_value: Some(5_000_000_000),
            extranonce_len: 8,
            declare_timeout_secs: 10,
            user_identifier: "spc-agent".into(),
        });
        cfg
    }

    #[test]
    fn validates_a_jdc_config_and_its_scan_key() {
        let cfg = with_jd();
        cfg.validate().unwrap();
        let jd = cfg.jd.as_ref().unwrap();
        assert_eq!(jd.template().unwrap(), (800_000, 5_000_000_000, 8));
        assert!(matches!(
            jd.jds_target().unwrap(),
            Target::Addr(a) if a.port() == 34264
        ));
    }

    #[test]
    fn rejects_a_scan_key_that_does_not_match_sp_address() {
        let mut cfg = with_jd();
        cfg.jd.as_mut().unwrap().scan_secret_key = Some(hex::encode([5u8; 32]));
        assert_eq!(cfg.validate(), Err(ConfigError::ScanKeyMismatch));
    }

    #[test]
    fn rejects_jd_without_a_template_or_jds_address() {
        let mut cfg = with_jd();
        cfg.jd.as_mut().unwrap().height = None;
        assert_eq!(cfg.validate(), Err(ConfigError::JdTemplate));

        let mut cfg = with_jd();
        cfg.jd.as_mut().unwrap().jds_address = "not-an-address".into();
        assert_eq!(
            cfg.validate(),
            Err(ConfigError::JdsAddress("not-an-address".into()))
        );
    }

    #[test]
    fn upstream_accepts_hostnames_and_onion_targets() {
        // Hostnames and .onion names are dial targets, not listen addresses.
        let mut cfg = sample();
        cfg.upstream.address = "pool.example:34254".into();
        cfg.validate().unwrap();
        assert!(matches!(
            cfg.upstream_target().unwrap(),
            Target::Name { ref host, .. } if host == "pool.example"
        ));
        let mut cfg = sample();
        cfg.upstream.address = "abc.onion:34254".into();
        cfg.net.proxy = "socks5h://127.0.0.1:9050".into();
        cfg.validate().unwrap();
    }

    #[test]
    fn onion_upstream_or_jds_without_proxy_is_a_startup_error() {
        let mut cfg = sample();
        cfg.upstream.address = "abc.onion:34254".into();
        assert_eq!(
            cfg.validate(),
            Err(ConfigError::OnionWithoutProxy("upstream.address"))
        );

        let mut cfg = with_jd();
        cfg.jd.as_mut().unwrap().jds_address = "abc.onion:34264".into();
        assert_eq!(
            cfg.validate(),
            Err(ConfigError::OnionWithoutProxy("jd.jds_address"))
        );

        // ...and both pass with a socks5h proxy set.
        let mut cfg = with_jd();
        cfg.jd.as_mut().unwrap().jds_address = "abc.onion:34264".into();
        cfg.net.proxy = "socks5h://127.0.0.1:9050".into();
        cfg.validate().unwrap();
    }

    #[test]
    fn proxy_schemes_other_than_socks5h_are_rejected() {
        for scheme in ["socks5", "http", "https"] {
            let mut cfg = sample();
            cfg.net.proxy = format!("{scheme}://127.0.0.1:9050");
            assert!(
                matches!(cfg.validate(), Err(ConfigError::Proxy(_))),
                "{scheme}"
            );
        }
    }

    #[test]
    fn rejects_a_zero_declare_timeout() {
        let mut cfg = with_jd();
        cfg.jd.as_mut().unwrap().declare_timeout_secs = 0;
        assert_eq!(cfg.validate(), Err(ConfigError::DeclareTimeout));
        // The documented default applies when the key is absent entirely.
        let parsed: Jd = toml::from_str(
            r#"
                enabled = true
                jds_address = "127.0.0.1:34264"
            "#,
        )
        .unwrap();
        assert_eq!(parsed.declare_timeout_secs, 10);
    }

    #[test]
    fn stream_isolation_defaults_true_only_when_proxied() {
        let mut cfg = sample();
        cfg.net.proxy = "socks5h://127.0.0.1:9050".into();
        cfg.validate().unwrap();
        let egress = cfg.net.egress().unwrap();
        assert!(egress.proxy().is_some());

        // Isolation explicitly off is honoured (operator's explicit choice).
        let mut cfg = sample();
        cfg.net.proxy = "socks5h://127.0.0.1:9050".into();
        cfg.net.stream_isolation = Some(false);
        cfg.validate().unwrap();
        assert_eq!(cfg.net.stream_isolation, Some(false));
    }
}
