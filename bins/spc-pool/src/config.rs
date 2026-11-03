//! `spc-pool` configuration and startup invariants (SPEC §6.3, §9, §11.10).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use spc_crypto::Network;
use thiserror::Error;

pub const MAX_FRONT_ID: u8 = 127;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Front,
    Payout,
    Api,
    Ark,
}

impl Role {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "front" => Some(Self::Front),
            "payout" => Some(Self::Payout),
            "api" => Some(Self::Api),
            "ark" => Some(Self::Ark),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub pool: Pool,
    #[serde(default)]
    pub template: Template,
    #[serde(default)]
    pub sv1: Sv1,
    #[serde(default)]
    pub accounting: Accounting,
    #[serde(default)]
    pub budget: Budget,
    #[serde(default)]
    pub jd: Jd,
    #[serde(default)]
    pub extensions: Extensions,
    #[serde(default)]
    pub storage: Storage,
    #[serde(default)]
    pub ark: Option<Ark>,
    #[serde(default)]
    pub api: Api,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pool {
    pub network: String,
    #[serde(default = "default_roles")]
    pub roles: Vec<String>,
    pub listen: String,
    pub authority_public_key: String,
    pub pool_payout_descriptor: String,
    #[serde(default = "default_fee_bps")]
    pub fee_bps: u16,
    #[serde(default)]
    pub front_id: u8,
    #[serde(default = "default_outbox_max")]
    pub outbox_max: usize,
    #[serde(default = "default_tip_divergence")]
    pub tip_divergence_secs: u64,
    /// Dev-only override for the pool P2TR script (hex). Production derives it
    /// from `pool_payout_descriptor`.
    #[serde(default)]
    pub pool_payout_script: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Template {
    #[serde(default = "default_template_source")]
    pub source: String,
    #[serde(default = "default_socket")]
    pub socket_path: PathBuf,
    #[serde(default = "default_extranonce_bytes")]
    pub extranonce_bytes: usize,
    /// How often the front polls the template source, in milliseconds. Bounds
    /// how long an external new block leaves the JDS and miner jobs stale
    /// (SPEC §2.2/§3.5).
    #[serde(default = "default_refresh_interval_ms")]
    pub refresh_interval_ms: u64,
    #[serde(default = "default_rpc_url")]
    pub rpc_url: String,
    #[serde(default)]
    pub rpc_user: String,
    #[serde(default)]
    pub rpc_pass: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sv1 {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_sv1_listen")]
    pub listen: String,
    #[serde(default = "default_extranonce1_bytes")]
    pub extranonce1_bytes: usize,
    #[serde(default = "default_extranonce2_bytes")]
    pub extranonce2_bytes: usize,
    #[serde(default = "default_true")]
    pub vardiff: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Accounting {
    #[serde(default = "default_window_n")]
    pub window_n: u32,
    /// Pool share difficulty (SPEC §3.4, decision 2026-11-02-share-target-policy).
    /// Every ingress validates shares against this difficulty's target (never
    /// harder than the network target) and the front credits
    /// `min(share_difficulty, max(1, round(network_difficulty)))`.
    #[serde(default = "default_share_difficulty")]
    pub share_difficulty: u64,
    #[serde(default)]
    pub slice_fee_delta_sats: u64,
    #[serde(default = "default_max_slice_shares")]
    pub max_slice_shares: usize,
    #[serde(default = "default_dust")]
    pub dust_limit_sats: u64,
    #[serde(default = "default_dust")]
    pub coinbase_threshold_sats: u64,
    #[serde(default = "default_max_slots")]
    pub max_payout_slots: usize,
    #[serde(default = "default_retain")]
    pub retain_windows: u32,
    #[serde(default = "default_scrub")]
    pub scrub_after_blocks: u32,
    #[serde(default = "default_confirmation_depth")]
    pub confirmation_depth: u32,
    /// Dev-only: allow the payout chain gate to pass without an RPC chain view.
    /// MUST only be used on regtest/signet. Production requires `template.source = "rpc"`.
    #[serde(default)]
    pub allow_unchecked_chain: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Budget {
    #[serde(default = "default_budget_floor")]
    pub floor_wu: u32,
    #[serde(default = "default_budget_ceiling")]
    pub ceiling_wu: u32,
    #[serde(default = "default_up_threshold")]
    pub up_threshold: f64,
    #[serde(default = "default_down_threshold")]
    pub down_threshold: f64,
    #[serde(default = "default_step_factor")]
    pub step_factor: f64,
    #[serde(default = "default_up_debounce")]
    pub up_debounce: u32,
    #[serde(default = "default_down_debounce")]
    pub down_debounce: u32,
    #[serde(default = "default_cooldown")]
    pub cooldown_secs: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Jd {
    #[serde(default = "default_dist_interval")]
    pub distribution_interval_secs: u64,
    #[serde(default = "default_rebalance_bps")]
    pub distribution_rebalance_bps: u16,
    /// MUST stay true (SPEC §3.5). Validation rejects false.
    #[serde(default = "default_true")]
    pub grace_same_height_only: bool,
    /// JDP listener (always on, matching the bundled JDS decision).
    #[serde(default = "default_jd_listen")]
    pub listen: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Extensions {
    #[serde(default = "default_true")]
    pub share_accounting: bool,
    #[serde(default = "default_window_hot_blocks")]
    pub window_hot_blocks: u32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Storage {
    #[serde(default = "default_postgres")]
    pub postgres: String,
    #[serde(default = "default_redis")]
    pub redis: String,
    #[serde(default = "default_cursor_heartbeat")]
    pub front_cursor_heartbeat_secs: u64,
    #[serde(default = "default_cursor_ttl")]
    pub front_cursor_ttl_secs: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Api {
    #[serde(default = "default_api_listen")]
    pub listen: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ark {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub server: String,
    #[serde(default)]
    pub ark_id: String,
    #[serde(default)]
    pub float_target_sats: u64,
    #[serde(default)]
    pub max_order_sats: u64,
    #[serde(default)]
    pub max_daily_sats: u64,
    #[serde(default)]
    pub order_pubkey: String,
    #[serde(default = "default_order_key_path")]
    pub order_key_path: PathBuf,
    #[serde(default = "default_seed_path")]
    pub seed_path: PathBuf,
    #[serde(default = "default_pool_account")]
    pub pool_output_account: String,
    #[serde(default = "default_min_remaining")]
    pub min_remaining_blocks: u32,
    #[serde(default = "default_retry_blocks")]
    pub retry_blocks: u32,
    #[serde(default)]
    pub server_is_self: bool,
    /// Ark transport selector (SPEC §11.10): `"bark"` (default, the
    /// stock bark wallet at the UPSTREAM_DEPS.md pin) or `"command"`
    /// (the dev/test external-command transport, refused outside
    /// regtest/signet).
    #[serde(default = "default_transport")]
    pub transport: String,
    /// Bark wallet datadir (sqlite persister, on-chain wallet state,
    /// lock files); created 0o700 at first start.
    #[serde(default = "default_datadir")]
    pub datadir: PathBuf,
    /// External command to execute Ark sends. The command receives the address
    /// and sats as JSON on stdin: `{"address": "<ark1...>", "sats": 12345}`.
    /// It MUST return `{"vtxo_id": "<hex>", "expiry_height": <u32>}` on stdout.
    /// Non-zero exit, timeout, invalid JSON, or wrong types are treated as
    /// transport failures. This is a real operator integration point, not a mock.
    /// Required only when `transport = "command"`.
    #[serde(default)]
    pub send_command: Option<String>,
}

fn default_roles() -> Vec<String> {
    vec!["front".into(), "payout".into(), "api".into()]
}
fn default_fee_bps() -> u16 {
    100
}
fn default_outbox_max() -> usize {
    4096
}
fn default_tip_divergence() -> u64 {
    30
}
fn default_template_source() -> String {
    "ipc".into()
}
fn default_socket() -> PathBuf {
    "/run/bitcoin/node.sock".into()
}
fn default_extranonce_bytes() -> usize {
    16
}
fn default_refresh_interval_ms() -> u64 {
    1_000
}
fn default_rpc_url() -> String {
    "http://127.0.0.1:18443".into()
}
fn default_true() -> bool {
    true
}
fn default_sv1_listen() -> String {
    "0.0.0.0:3333".into()
}
fn default_extranonce1_bytes() -> usize {
    4
}
fn default_extranonce2_bytes() -> usize {
    4
}
fn default_window_n() -> u32 {
    8
}
fn default_share_difficulty() -> u64 {
    1
}
fn default_max_slice_shares() -> usize {
    65_000
}
fn default_dust() -> u64 {
    330
}
fn default_max_slots() -> usize {
    256
}
fn default_retain() -> u32 {
    16
}
fn default_scrub() -> u32 {
    100
}
fn default_confirmation_depth() -> u32 {
    6
}
fn default_budget_floor() -> u32 {
    50_000
}
fn default_budget_ceiling() -> u32 {
    400_000
}
fn default_up_threshold() -> f64 {
    0.85
}
fn default_down_threshold() -> f64 {
    0.50
}
fn default_step_factor() -> f64 {
    1.15
}
fn default_up_debounce() -> u32 {
    3
}
fn default_down_debounce() -> u32 {
    20
}
fn default_cooldown() -> u64 {
    300
}
fn default_dist_interval() -> u64 {
    60
}
fn default_rebalance_bps() -> u16 {
    100
}
fn default_jd_listen() -> String {
    "0.0.0.0:34264".into()
}
fn default_window_hot_blocks() -> u32 {
    6
}
fn default_postgres() -> String {
    "postgres://spc:spc@localhost:5432/spc".into()
}
fn default_redis() -> String {
    "redis://localhost:6379".into()
}
fn default_cursor_heartbeat() -> u64 {
    30
}
fn default_cursor_ttl() -> u64 {
    120
}
fn default_api_listen() -> String {
    "0.0.0.0:8080".into()
}
fn default_order_key_path() -> PathBuf {
    "/run/secrets/spc/ark-order-key".into()
}
fn default_seed_path() -> PathBuf {
    "/run/secrets/ark/seed".into()
}
fn default_pool_account() -> String {
    "m/86'/1'/1'".into()
}
fn default_min_remaining() -> u32 {
    3_024
}
fn default_retry_blocks() -> u32 {
    6
}
fn default_transport() -> String {
    "bark".into()
}
fn default_datadir() -> PathBuf {
    "/var/lib/spc/ark".into()
}

impl Default for Template {
    fn default() -> Self {
        Self {
            source: default_template_source(),
            socket_path: default_socket(),
            extranonce_bytes: default_extranonce_bytes(),
            refresh_interval_ms: default_refresh_interval_ms(),
            rpc_url: default_rpc_url(),
            rpc_user: String::new(),
            rpc_pass: String::new(),
        }
    }
}
impl Default for Sv1 {
    fn default() -> Self {
        Self {
            enabled: true,
            listen: default_sv1_listen(),
            extranonce1_bytes: default_extranonce1_bytes(),
            extranonce2_bytes: default_extranonce2_bytes(),
            vardiff: true,
        }
    }
}
impl Default for Accounting {
    fn default() -> Self {
        Self {
            window_n: default_window_n(),
            share_difficulty: default_share_difficulty(),
            slice_fee_delta_sats: 0,
            max_slice_shares: default_max_slice_shares(),
            dust_limit_sats: default_dust(),
            coinbase_threshold_sats: default_dust(),
            max_payout_slots: default_max_slots(),
            retain_windows: default_retain(),
            scrub_after_blocks: default_scrub(),
            confirmation_depth: default_confirmation_depth(),
            allow_unchecked_chain: false,
        }
    }
}
impl Default for Budget {
    fn default() -> Self {
        Self {
            floor_wu: default_budget_floor(),
            ceiling_wu: default_budget_ceiling(),
            up_threshold: default_up_threshold(),
            down_threshold: default_down_threshold(),
            step_factor: default_step_factor(),
            up_debounce: default_up_debounce(),
            down_debounce: default_down_debounce(),
            cooldown_secs: default_cooldown(),
        }
    }
}
impl Default for Jd {
    fn default() -> Self {
        Self {
            distribution_interval_secs: default_dist_interval(),
            distribution_rebalance_bps: default_rebalance_bps(),
            grace_same_height_only: true,
            listen: default_jd_listen(),
        }
    }
}
impl Default for Extensions {
    fn default() -> Self {
        Self {
            share_accounting: true,
            window_hot_blocks: default_window_hot_blocks(),
        }
    }
}
impl Default for Storage {
    fn default() -> Self {
        Self {
            postgres: default_postgres(),
            redis: default_redis(),
            front_cursor_heartbeat_secs: default_cursor_heartbeat(),
            front_cursor_ttl_secs: default_cursor_ttl(),
        }
    }
}
impl Default for Api {
    fn default() -> Self {
        Self {
            listen: default_api_listen(),
        }
    }
}
impl Default for Ark {
    fn default() -> Self {
        Self {
            enabled: false,
            server: String::new(),
            ark_id: String::new(),
            float_target_sats: 0,
            max_order_sats: 0,
            max_daily_sats: 0,
            order_pubkey: String::new(),
            order_key_path: default_order_key_path(),
            seed_path: default_seed_path(),
            pool_output_account: default_pool_account(),
            min_remaining_blocks: default_min_remaining(),
            retry_blocks: default_retry_blocks(),
            server_is_self: false,
            transport: default_transport(),
            datadir: default_datadir(),
            send_command: None,
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("unknown network '{0}' (expected mainnet, testnet4, signet, regtest)")]
    Network(String),
    #[error("mainnet is out of scope for v0.1")]
    Mainnet,
    #[error("unknown role '{0}'")]
    Role(String),
    #[error("front_id must be 0..=127")]
    FrontId,
    #[error("fee_bps must be <= 10000")]
    FeeBps,
    #[error("window_n must be > 0")]
    WindowN,
    #[error("share_difficulty must be > 0")]
    ShareDifficulty,
    #[error("max_payout_slots must be > 0")]
    Slots,
    #[error("coinbase_threshold_sats must be >= dust_limit_sats")]
    Threshold,
    #[error("confirmation_depth must be > 0")]
    ConfirmationDepth,
    #[error("template.refresh_interval_ms must be > 0")]
    RefreshInterval,
    #[error("grace_same_height_only MUST be true (SPEC §3.5)")]
    Grace,
    #[error("invalid pool_payout_descriptor: {0}")]
    Descriptor(String),
    #[error("SV1 extranonce does not fit the scriptSig budget")]
    Sv1Budget,
    #[error("invalid listen address: {0}")]
    Listen(String),
    #[error("ark enabled without {0}")]
    ArkMissing(&'static str),
    #[error("invalid ark transport config: {0}")]
    ArkTransport(String),
    #[error("ark must not be self-hosted on mainnet")]
    ArkSelfMainnet,
}

impl Config {
    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let cfg: Self = toml::from_str(&text)?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn network(&self) -> Result<Network, ConfigError> {
        match self.pool.network.as_str() {
            "mainnet" => Err(ConfigError::Mainnet),
            "testnet4" | "testnet" => Ok(Network::Testnet),
            "signet" => Ok(Network::Signet),
            "regtest" => Ok(Network::Regtest),
            other => Err(ConfigError::Network(other.to_string())),
        }
    }

    pub fn roles(&self) -> Result<Vec<Role>, ConfigError> {
        self.pool
            .roles
            .iter()
            .map(|r| Role::parse(r).ok_or_else(|| ConfigError::Role(r.clone())))
            .collect()
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        let _ = self.network()?;
        let _ = self.roles()?;
        if self.pool.front_id > MAX_FRONT_ID {
            return Err(ConfigError::FrontId);
        }
        if self.pool.fee_bps > 10_000 {
            return Err(ConfigError::FeeBps);
        }
        if self.accounting.window_n == 0 {
            return Err(ConfigError::WindowN);
        }
        if self.accounting.share_difficulty == 0 {
            return Err(ConfigError::ShareDifficulty);
        }
        if self.accounting.max_payout_slots == 0 {
            return Err(ConfigError::Slots);
        }
        if self.accounting.coinbase_threshold_sats < self.accounting.dust_limit_sats {
            return Err(ConfigError::Threshold);
        }
        if self.accounting.confirmation_depth == 0 {
            return Err(ConfigError::ConfirmationDepth);
        }
        if self.template.refresh_interval_ms == 0 {
            return Err(ConfigError::RefreshInterval);
        }
        if !self.jd.grace_same_height_only {
            return Err(ConfigError::Grace);
        }
        validate_pool_descriptor(&self.pool.pool_payout_descriptor)?;
        match self.template.source.as_str() {
            "ipc" | "sv2-tp" | "rpc" | "mock" => {}
            other => {
                return Err(ConfigError::Listen(format!(
                    "unknown template source '{other}'"
                )))
            }
        }

        // The height push is at most 5 bytes and push32 is 33; reserve the
        // worst case at the current height (height 0 uses 1, but the check is
        // intentionally conservative).
        let used = 5 + 33 + self.sv1.extranonce1_bytes + self.sv1.extranonce2_bytes;
        if self.sv1.enabled && used > 100 {
            return Err(ConfigError::Sv1Budget);
        }
        if self.sv1.enabled && (self.sv1.extranonce1_bytes == 0 || self.sv1.extranonce2_bytes == 0)
        {
            return Err(ConfigError::Sv1Budget);
        }

        parse_listen(&self.pool.listen)?;
        if self.sv1.enabled {
            parse_listen(&self.sv1.listen)?;
        }
        parse_listen(&self.api.listen)?;
        parse_listen(&self.jd.listen)?;

        if let Some(ark) = &self.ark {
            if ark.enabled {
                if ark.server.is_empty() {
                    return Err(ConfigError::ArkMissing("server"));
                }
                if ark.ark_id.is_empty() {
                    return Err(ConfigError::ArkMissing("ark_id"));
                }
                if ark.order_pubkey.is_empty() {
                    return Err(ConfigError::ArkMissing("order_pubkey"));
                }
                if ark.float_target_sats == 0 {
                    return Err(ConfigError::ArkMissing("float_target_sats"));
                }
                if ark.retry_blocks == 0 {
                    return Err(ConfigError::ArkMissing("retry_blocks"));
                }
                // Transport selection (SPEC §11.10): "bark" is the
                // production stock-wallet transport; "command" is
                // dev/test only and is refused outside regtest/signet
                // (SPEC §11.9 negative matrix).
                let is_dev_network = matches!(self.pool.network.as_str(), "regtest" | "signet");
                match ark.transport.as_str() {
                    "bark" => {}
                    "command" => {
                        if !is_dev_network {
                            return Err(ConfigError::ArkTransport(
                                "transport = \"command\" is dev/test only; it requires regtest or signet"
                                    .into(),
                            ));
                        }
                        if ark.send_command.is_none() {
                            return Err(ConfigError::ArkMissing("send_command"));
                        }
                    }
                    other => {
                        return Err(ConfigError::ArkTransport(format!(
                            "unknown transport '{other}' (expected \"bark\" or \"command\")"
                        )));
                    }
                }
                if ark.server_is_self && self.pool.network == "mainnet" {
                    return Err(ConfigError::ArkSelfMainnet);
                }
                // server_is_self is only permitted on regtest/signet (SPEC §11.3, §11.9)
                if ark.server_is_self && !matches!(self.pool.network.as_str(), "regtest" | "signet")
                {
                    return Err(ConfigError::ArkSelfMainnet);
                }
            }
        }

        // Dev-only guards: mock template source and unchecked chain gate are only
        // permitted on regtest/signet with explicit opt-in (SPEC §7.4, §11.10).
        let is_dev_network = matches!(self.pool.network.as_str(), "regtest" | "signet");
        if self.template.source == "mock"
            && (!is_dev_network || !self.accounting.allow_unchecked_chain)
        {
            return Err(ConfigError::Listen(
                "template source 'mock' requires regtest/signet AND accounting.allow_unchecked_chain = true"
                    .into(),
            ));
        }
        if self.accounting.allow_unchecked_chain && !is_dev_network {
            return Err(ConfigError::Listen(
                "accounting.allow_unchecked_chain is only permitted on regtest/signet".into(),
            ));
        }
        Ok(())
    }
}

fn parse_listen(s: &str) -> Result<(), ConfigError> {
    s.parse::<std::net::SocketAddr>()
        .map(|_| ())
        .map_err(|_| ConfigError::Listen(s.to_string()))
}

/// The three assertions from the xpub spec (SPEC §6.3):
/// single-key `tr(xpub/.../*)`, exactly one unhardened wildcard, no hardened
/// step after the xpub, no multipath, and no private key material.
pub fn validate_pool_descriptor(d: &str) -> Result<(), ConfigError> {
    if !d.starts_with("tr(") || !d.ends_with(')') {
        return Err(ConfigError::Descriptor(
            "must be a tr(...) descriptor".into(),
        ));
    }
    let inner = &d[3..d.len() - 1];
    if inner.contains("xprv") || inner.contains("tprv") || inner.contains("prv") {
        return Err(ConfigError::Descriptor(
            "must not contain a private key".into(),
        ));
    }
    if !(inner.contains("xpub")
        || inner.contains("tpub")
        || inner.contains("upub")
        || inner.contains("vpub"))
    {
        return Err(ConfigError::Descriptor(
            "must contain an extended public key".into(),
        ));
    }
    if inner.contains('<') || inner.contains(';') {
        return Err(ConfigError::Descriptor("multipath is not supported".into()));
    }
    if inner.matches("/*").count() != 1 {
        return Err(ConfigError::Descriptor(
            "exactly one unhardened wildcard is required".into(),
        ));
    }
    // No hardened derivation after the xpub.
    if inner.split('/').any(|part| part.ends_with('\'')) {
        return Err(ConfigError::Descriptor(
            "hardened steps are not allowed".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
[pool]
network = "regtest"
roles = ["front", "payout", "api"]
listen = "127.0.0.1:34254"
authority_public_key = "02aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
pool_payout_descriptor = "tr(tpubD6NzVbkrYhZ4YTest/0/*)"
front_id = 0

[template]
source = "sv2-tp"
extranonce_bytes = 16

[sv1]
enabled = true
listen = "127.0.0.1:3333"
extranonce1_bytes = 4
extranonce2_bytes = 4

[api]
listen = "127.0.0.1:8080"
"#;

    fn sample() -> Config {
        toml::from_str(SAMPLE).unwrap()
    }

    #[test]
    fn parses_the_spec_sample() {
        let cfg = sample();
        cfg.validate().unwrap();
        assert_eq!(cfg.network().unwrap(), Network::Regtest);
        assert_eq!(cfg.accounting.window_n, 8);
        assert_eq!(cfg.accounting.coinbase_threshold_sats, 330);
        assert!(cfg.jd.grace_same_height_only);
    }

    #[test]
    fn rejects_mainnet() {
        let mut cfg = sample();
        cfg.pool.network = "mainnet".into();
        assert_eq!(cfg.validate(), Err(ConfigError::Mainnet));
    }

    #[test]
    fn rejects_bad_descriptors() {
        for d in [
            "tr(xprv.../0/*)",
            "tr(tpub..../0'/0/*)",
            "tr(tpub..../<0;1>/*)",
            "tr(tpub..../0)",
            "wpkh(tpub..../0/*)",
        ] {
            assert!(validate_pool_descriptor(d).is_err(), "{d} should fail");
        }
        assert!(validate_pool_descriptor("tr(tpubD6NzVbkrYhZ4YTest/0/*)").is_ok());
    }

    #[test]
    fn share_difficulty_defaults_to_one_and_must_be_positive() {
        let cfg = sample();
        assert_eq!(cfg.accounting.share_difficulty, 1);
        let mut cfg = sample();
        cfg.accounting.share_difficulty = 0;
        assert_eq!(cfg.validate(), Err(ConfigError::ShareDifficulty));
        let mut cfg = sample();
        cfg.accounting.share_difficulty = 16_384;
        cfg.validate().unwrap();
    }

    #[test]
    fn refresh_interval_defaults_and_must_be_positive() {
        let cfg = sample();
        assert_eq!(cfg.template.refresh_interval_ms, 1_000);
        let mut cfg = sample();
        cfg.template.refresh_interval_ms = 0;
        assert_eq!(cfg.validate(), Err(ConfigError::RefreshInterval));
    }

    #[test]
    fn rejects_threshold_below_dust() {
        let mut cfg = sample();
        cfg.accounting.coinbase_threshold_sats = 329;
        assert_eq!(cfg.validate(), Err(ConfigError::Threshold));
    }

    #[test]
    fn rejects_sv1_budget() {
        let mut cfg = sample();
        cfg.sv1.extranonce1_bytes = 40;
        cfg.sv1.extranonce2_bytes = 40;
        assert_eq!(cfg.validate(), Err(ConfigError::Sv1Budget));
    }

    #[test]
    fn rejects_front_id_too_large() {
        let mut cfg = sample();
        cfg.pool.front_id = 128;
        assert_eq!(cfg.validate(), Err(ConfigError::FrontId));
    }

    #[test]
    fn rejects_grace_off() {
        let mut cfg = sample();
        cfg.jd.grace_same_height_only = false;
        assert_eq!(cfg.validate(), Err(ConfigError::Grace));
    }

    fn ark_enabled() -> Config {
        let mut cfg = sample();
        cfg.ark = Some(Ark {
            enabled: true,
            server: "https://ark.example.tech".into(),
            ark_id: "00000000".into(),
            float_target_sats: 1_000_000,
            max_order_sats: 330,
            max_daily_sats: 0,
            order_pubkey: "01".repeat(32),
            order_key_path: default_order_key_path(),
            seed_path: default_seed_path(),
            pool_output_account: default_pool_account(),
            min_remaining_blocks: 3_024,
            retry_blocks: 6,
            server_is_self: false,
            transport: default_transport(),
            datadir: default_datadir(),
            send_command: None,
        });
        cfg
    }

    /// `transport = "bark"` is the default and needs no send_command
    /// (SPEC §11.10); `"command"` is dev/test only and requires both a
    /// dev network and the command itself (SPEC §11.9).
    #[test]
    fn ark_transport_selection_is_enforced() {
        // Default: bark, no send_command, any (non-mainnet) network.
        let cfg = ark_enabled();
        assert_eq!(cfg.ark.as_ref().unwrap().transport, "bark");
        cfg.validate().unwrap();

        // command on regtest with the command set: fine.
        let mut cfg = ark_enabled();
        cfg.ark.as_mut().unwrap().transport = "command".into();
        cfg.ark.as_mut().unwrap().send_command = Some("true".into());
        cfg.validate().unwrap();

        // command on testnet4: refused (not a dev network).
        let mut cfg = ark_enabled();
        cfg.pool.network = "testnet4".into();
        cfg.ark.as_mut().unwrap().transport = "command".into();
        cfg.ark.as_mut().unwrap().send_command = Some("true".into());
        assert!(matches!(cfg.validate(), Err(ConfigError::ArkTransport(_))));

        // command without the command: refused.
        let mut cfg = ark_enabled();
        cfg.ark.as_mut().unwrap().transport = "command".into();
        assert_eq!(cfg.validate(), Err(ConfigError::ArkMissing("send_command")));

        // Unknown transport: refused.
        let mut cfg = ark_enabled();
        cfg.ark.as_mut().unwrap().transport = "carrier-pigeon".into();
        assert!(matches!(cfg.validate(), Err(ConfigError::ArkTransport(_))));
    }
}
