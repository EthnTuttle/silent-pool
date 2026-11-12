//! `spc-pool` configuration and startup invariants (SPEC §6.3, §9, §11.10).

use std::net::SocketAddr;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use spc_crypto::Network;
use stratum_apps::key_utils::{Secp256k1PublicKey, Secp256k1SecretKey};
use thiserror::Error;

use crate::net::host_is_onion;
use crate::sv1::SharePolicy;
use spc_accounting::work_unit::{
    decode_compact_target, HARD_CAP_MAX_WINDOW_TARGET_UNITS, HARD_CAP_MAX_WORK_MULTIPLIER,
};

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
    pub reach: Reach,
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
    #[serde(default)]
    pub operator: Operator,
}

/// `[reach]` (SPEC §3.8): how the pool is found. The Tor daemon owns onion
/// keys and forwards to the loopback listeners; this binary only advertises.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reach {
    /// Published verbatim as `endpoints` at `GET /api/pool` (§6.1).
    #[serde(default)]
    pub advertise: Vec<String>,
    /// Per-listener advertised "<id>.onion:<port>".
    #[serde(default)]
    pub onion: Onion,
}

/// A non-empty entry means the matching listener is published as a Tor
/// onion service and therefore MUST bind a loopback address (§3.8).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Onion {
    #[serde(default)]
    pub sv2: String,
    #[serde(default)]
    pub sv1: String,
    #[serde(default)]
    pub jd: String,
    #[serde(default)]
    pub api: String,
}

#[derive(Clone, Default, Deserialize)]
#[serde(transparent)]
pub struct RedactedSecret(String);

impl RedactedSecret {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for RedactedSecret {
    fn from(value: &str) -> Self {
        Self(value.into())
    }
}

impl std::fmt::Debug for RedactedSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[redacted]")
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pool {
    pub network: String,
    #[serde(default = "default_roles")]
    pub roles: Vec<String>,
    pub listen: String,
    pub authority_public_key: String,
    /// Base58check key-utils secret for the miner-facing SV2 Noise authority.
    /// Required when the front role is selected; matched against the public pin.
    #[serde(default)]
    pub authority_secret_key: RedactedSecret,
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
    #[serde(default)]
    pub share_difficulty: Option<u64>,
    /// 1 = frozen integer difficulty; 2 = opt-in exact reference multiplier.
    #[serde(default = "default_work_unit_version")]
    pub work_unit_version: u8,
    /// Required for v2, forbidden for v1. 32-byte big-endian target as 64 hex.
    #[serde(default)]
    pub work_reference_target: Option<String>,
    /// Required for v2, forbidden as a legacy-difficulty alias.
    #[serde(default)]
    pub share_work_multiplier: Option<u64>,
    #[serde(default = "default_max_work_multiplier")]
    pub max_work_multiplier: u64,
    /// Decimal u128 string (2^64 by default, avoids TOML's signed-64 limit).
    #[serde(default = "default_max_window_target_units")]
    pub max_window_target_units: String,
    /// Effective node pow limit. v2 is explicitly regtest-only in this packet.
    #[serde(default = "default_pow_limit_bits")]
    pub pow_limit_bits: u32,
    /// Optional explicit discriminator; the correct rule is required when set.
    #[serde(default)]
    pub window_rule: Option<String>,
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

/// `[operator]` (EXP-OP, contract §4): the optional, off-by-default
/// loopback-only listener that serves the read-only operator page and its
/// observation DTO inside the same API process. Absent section = listener
/// off; a non-loopback `listen` value is a configuration validation
/// error; the listener never mounts on the public `[api]` router.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Operator {
    /// Loopback listen address, e.g. `"127.0.0.1:8081"`. `None` (section
    /// absent or key absent) keeps the operator listener off.
    pub listen: Option<String>,
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
    /// Hex compressed ArkInfo server public key (front-only public metadata).
    #[serde(default)]
    pub server_pubkey: String,
    /// Hex compressed ArkInfo mailbox public key (front-only public metadata).
    #[serde(default)]
    pub mailbox_pubkey: String,
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
fn default_work_unit_version() -> u8 {
    1
}
fn default_max_work_multiplier() -> u64 {
    HARD_CAP_MAX_WORK_MULTIPLIER
}
fn default_max_window_target_units() -> String {
    HARD_CAP_MAX_WINDOW_TARGET_UNITS.to_string()
}
fn default_pow_limit_bits() -> u32 {
    0x207f_ffff
}
fn default_max_slice_shares() -> usize {
    65_000
}
fn default_dust() -> u64 {
    330
}
fn default_max_slots() -> usize {
    1024
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
    1_000_000
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
            share_difficulty: None,
            work_unit_version: default_work_unit_version(),
            work_reference_target: None,
            share_work_multiplier: None,
            max_work_multiplier: default_max_work_multiplier(),
            max_window_target_units: default_max_window_target_units(),
            pow_limit_bits: default_pow_limit_bits(),
            window_rule: None,
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
            server_pubkey: String::new(),
            mailbox_pubkey: String::new(),
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
    #[error("max_payout_slots must be in 1..=2048")]
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
    #[error("invalid miner-facing Noise authority: {0}")]
    Authority(String),
    #[error("invalid work-unit profile: {0}")]
    WorkUnit(String),
    #[error("SV1 extranonce does not fit the scriptSig budget")]
    Sv1Budget,
    #[error("invalid listen address: {0}")]
    Listen(String),
    #[error("invalid [net] proxy: {0}")]
    Proxy(String),
    #[error("invalid [reach] onion.{0} entry '{1}' (expected \"<id>.onion:<port>\")")]
    OnionAdvertised(&'static str, String),
    #[error("[reach] onion.{0} is advertised but [{1}] listen = {2} is not a loopback address (SPEC §3.8)")]
    OnionNotLoopback(&'static str, &'static str, String),
    #[error("invalid [ark] server URL: {0}")]
    ArkServer(String),
    #[error("ark enabled without {0}")]
    ArkMissing(&'static str),
    #[error("invalid ark transport config: {0}")]
    ArkTransport(String),
    #[error("ark must not be self-hosted on mainnet")]
    ArkSelfMainnet,
    #[error("invalid [operator] listen address: {0}")]
    OperatorListen(String),
    #[error("[operator] listen = {0} is not a loopback address; the operator page is loopback-only (EXP-OP)")]
    OperatorNotLoopback(String),
    #[error("[operator] listen requires the api role; the operator listener runs inside the pool API process")]
    OperatorRequiresApiRole,
}

impl Pool {
    pub fn authority_secret(&self) -> Result<Secp256k1SecretKey, ConfigError> {
        if self.authority_secret_key.is_empty() {
            return Err(ConfigError::Authority(
                "pool.authority_secret_key is required for the front role".into(),
            ));
        }
        self.authority_secret_key
            .as_str()
            .parse()
            .map_err(|_| ConfigError::Authority("invalid authority_secret_key encoding".into()))
    }

    pub fn authority_public(&self) -> Result<Secp256k1PublicKey, ConfigError> {
        self.authority_public_key
            .parse()
            .map_err(|_| ConfigError::Authority("invalid authority_public_key encoding".into()))
    }
}

impl Accounting {
    pub fn legacy_share_difficulty(&self) -> u64 {
        self.share_difficulty.unwrap_or(1)
    }

    pub fn profile_json(&self) -> serde_json::Value {
        let (rule, fingerprint) = match self.work_unit_version {
            2 => (
                self.window_rule.as_deref().unwrap_or("network_block_work"),
                self.work_reference_target.as_deref().and_then(|reference| {
                    let bytes = hex::decode(reference).ok()?.try_into().ok()?;
                    Some(hex::encode(
                        spc_accounting::work_unit::work_unit_fingerprint(&bytes),
                    ))
                }),
            ),
            _ => (
                self.window_rule.as_deref().unwrap_or("share_work_legacy"),
                None,
            ),
        };
        serde_json::json!({
            "work_unit_version": self.work_unit_version,
            "work_reference_target": self.work_reference_target,
            "share_work_multiplier": self.share_work_multiplier,
            "share_difficulty": self.share_difficulty,
            "work_unit_fingerprint": fingerprint,
            "max_work_multiplier": self.max_work_multiplier.to_string(),
            "max_window_target_units": self.max_window_target_units,
            "window_n": self.window_n,
            "window_rule": rule,
            // This is a v2-only regtest authority. Never attribute the
            // regtest default to a legacy pool on another network.
            "pow_limit_bits": (self.work_unit_version == 2).then_some(self.pow_limit_bits),
            "max_slice_shares": self.max_slice_shares,
        })
    }

    pub fn share_policy(&self, network: Network) -> Result<SharePolicy, ConfigError> {
        match self.work_unit_version {
            1 => {
                if self.work_reference_target.is_some() || self.share_work_multiplier.is_some() {
                    return Err(ConfigError::WorkUnit(
                        "v2 reference/multiplier fields are forbidden with work_unit_version = 1"
                            .into(),
                    ));
                }
                if self
                    .window_rule
                    .as_deref()
                    .is_some_and(|rule| rule != "share_work_legacy")
                {
                    return Err(ConfigError::WorkUnit(
                        "work_unit_version = 1 requires window_rule = share_work_legacy".into(),
                    ));
                }
                if self.legacy_share_difficulty() == 0 {
                    return Err(ConfigError::ShareDifficulty);
                }
                Ok(SharePolicy::v1(self.legacy_share_difficulty()))
            }
            2 => {
                if network != Network::Regtest {
                    return Err(ConfigError::WorkUnit(
                        "v2 is supported only with pool.network = regtest".into(),
                    ));
                }
                if self.share_difficulty.is_some() {
                    return Err(ConfigError::WorkUnit(
                        "share_difficulty is a legacy v1 field and cannot be combined with v2"
                            .into(),
                    ));
                }
                if self.max_slice_shares > 65_000 {
                    return Err(ConfigError::WorkUnit(
                        "v2 requires max_slice_shares <= 65000".into(),
                    ));
                }
                if self.max_work_multiplier > HARD_CAP_MAX_WORK_MULTIPLIER {
                    return Err(ConfigError::WorkUnit(
                        "max_work_multiplier_above_hard_cap".into(),
                    ));
                }
                let max_window_target_units =
                    self.max_window_target_units.parse::<u128>().map_err(|_| {
                        ConfigError::WorkUnit(
                            "max_window_target_units is not a u128 decimal string".into(),
                        )
                    })?;
                if max_window_target_units > HARD_CAP_MAX_WINDOW_TARGET_UNITS {
                    return Err(ConfigError::WorkUnit(
                        "max_window_target_units_above_hard_cap".into(),
                    ));
                }
                if self
                    .window_rule
                    .as_deref()
                    .is_some_and(|rule| rule != "network_block_work")
                {
                    return Err(ConfigError::WorkUnit(
                        "work_unit_version = 2 requires window_rule = network_block_work".into(),
                    ));
                }
                if self.pow_limit_bits != 0x207f_ffff {
                    // Regtest's consensus powLimit is exactly 0x207fffff. An
                    // operator override must not turn the reference-family
                    // check into a fictitious alternate network authority.
                    return Err(ConfigError::WorkUnit(format!(
                        "pow_limit_bits must equal regtest consensus 0x207fffff, got 0x{:08x}",
                        self.pow_limit_bits
                    )));
                }
                let reference = self.work_reference_target.as_deref().ok_or_else(|| {
                    ConfigError::WorkUnit("work_reference_target is required with v2".into())
                })?;
                let reference = hex::decode(reference).map_err(|_| {
                    ConfigError::WorkUnit("work_reference_target must be 32-byte hex".into())
                })?;
                let reference: [u8; 32] = reference.try_into().map_err(|_| {
                    ConfigError::WorkUnit("work_reference_target must be 32-byte hex".into())
                })?;
                let profile = spc_accounting::WorkUnitProfile::from_reference(
                    reference,
                    self.max_work_multiplier,
                )
                .map_err(|e| ConfigError::WorkUnit(e.to_string()))?;
                let multiplier = self.share_work_multiplier.ok_or_else(|| {
                    ConfigError::WorkUnit("share_work_multiplier is required with v2".into())
                })?;
                let j_cfg = profile
                    .validate_multiplier(multiplier)
                    .map_err(|e| ConfigError::WorkUnit(e.to_string()))?;
                let _ = decode_compact_target(self.pow_limit_bits)
                    .map_err(|e| ConfigError::WorkUnit(e.to_string()))?;
                profile
                    .validate_pow_limit(self.pow_limit_bits)
                    .map_err(|e| ConfigError::WorkUnit(e.to_string()))?;
                Ok(SharePolicy::V2 {
                    profile,
                    j_cfg,
                    max_window_target_units,
                })
            }
            other => Err(ConfigError::WorkUnit(format!(
                "unknown work_unit_version {other}"
            ))),
        }
    }
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
        let network = self.network()?;
        let roles = self.roles()?;
        if roles.contains(&Role::Front) {
            let secret = self.pool.authority_secret()?;
            let configured_public = self.pool.authority_public()?;
            if Secp256k1PublicKey::from(secret).into_bytes() != configured_public.into_bytes() {
                return Err(ConfigError::Authority(
                    "authority_public_key does not match authority_secret_key".into(),
                ));
            }
        }
        if self.pool.front_id > MAX_FRONT_ID {
            return Err(ConfigError::FrontId);
        }
        if self.pool.fee_bps > 10_000 {
            return Err(ConfigError::FeeBps);
        }
        if self.accounting.window_n == 0 {
            return Err(ConfigError::WindowN);
        }
        let _ = self.accounting.share_policy(network)?;
        if self.accounting.work_unit_version == 2 && self.sv1.enabled {
            return Err(ConfigError::WorkUnit(
                "v2 requires native SV2: set sv1.enabled = false".into(),
            ));
        }
        if self.accounting.work_unit_version == 2 && self.extensions.share_accounting {
            return Err(ConfigError::WorkUnit(
                "v2 type-32 auditing is unsupported: set extensions.share_accounting = false"
                    .into(),
            ));
        }
        if self.accounting.max_payout_slots == 0
            || self.accounting.max_payout_slots > spc_accounting::ladder::MAX_LADDER_OUTPUTS
        {
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

        // EXP-OP (contract §4): the operator listener is optional,
        // off-by-default and loopback-only. A non-loopback value is a
        // configuration error, not a silent public bind; it also requires
        // the api role because it runs inside the same API process.
        if let Some(listen) = &self.operator.listen {
            let addr = listen
                .parse::<SocketAddr>()
                .map_err(|_| ConfigError::OperatorListen(listen.clone()))?;
            if !addr.ip().is_loopback() {
                return Err(ConfigError::OperatorNotLoopback(listen.clone()));
            }
            if !roles.contains(&Role::Api) {
                return Err(ConfigError::OperatorRequiresApiRole);
            }
        }

        // SPEC §3.8: the egress proxy must do remote name resolution
        // (validated here so a bad scheme is a startup error, not a first-use
        // surprise), and an onion-published listener must bind loopback - an
        // onion service on a clearnet interface silently bypasses the privacy
        // it is published for.
        //
        // The loopback rule is deliberately NOT conditioned on the listener
        // being enabled or its role being selected: the onion entry is the
        // operator's statement "this address is published", and a published
        // clearnet-reachable listener is the bypass the rule exists to
        // forbid. Publishing and binding are config facts, not runtime state.
        for (name, section, listen, entry) in [
            ("sv2", "pool", &self.pool.listen, &self.reach.onion.sv2),
            ("sv1", "sv1", &self.sv1.listen, &self.reach.onion.sv1),
            ("jd", "jd", &self.jd.listen, &self.reach.onion.jd),
            ("api", "api", &self.api.listen, &self.reach.onion.api),
        ] {
            validate_onion_entry(name, entry)?;
            if entry.is_empty() {
                continue;
            }
            let addr = listen
                .parse::<SocketAddr>()
                .map_err(|_| ConfigError::Listen(listen.clone()))?;
            if !addr.ip().is_loopback() {
                return Err(ConfigError::OnionNotLoopback(
                    name,
                    section,
                    addr.to_string(),
                ));
            }
        }

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
                if roles.contains(&Role::Front) {
                    if ark.server_pubkey.is_empty() {
                        return Err(ConfigError::ArkMissing("server_pubkey"));
                    }
                    if ark.mailbox_pubkey.is_empty() {
                        return Err(ConfigError::ArkMissing("mailbox_pubkey"));
                    }
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

/// `[reach] onion` entries are advertised "<id>.onion:<port>" strings;
/// anything else is rejected at startup (fail closed, §3.8).
fn validate_onion_entry(name: &'static str, entry: &str) -> Result<(), ConfigError> {
    if entry.is_empty() {
        return Ok(());
    }
    let ok = entry
        .rsplit_once(':')
        .filter(|(host, port)| host_is_onion(host) && port.parse::<u16>().is_ok())
        .is_some();
    if !ok {
        return Err(ConfigError::OnionAdvertised(name, entry.to_string()));
    }
    Ok(())
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
authority_public_key = "9auqWEzQDVyd2oe1JVGFLMLHZtCo2FFqZwtKA5gd9xbuEu7PH72"
authority_secret_key = "mkDLTBBRxdBv998612qipDYoTK3YUrqLe8uWw7gu3iXbSrn2n"
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
    fn front_authority_is_provisioned_and_matches_its_pin() {
        let cfg = sample();
        let debug = format!("{:?}", cfg.pool);
        assert!(debug.contains("[redacted]"));
        assert!(!debug.contains("mkDLTBBRxdBv998612qipDYoTK3YUrqLe8uWw7gu3iXbSrn2n"));

        let mut missing = sample();
        missing.pool.authority_secret_key.clear();
        assert!(matches!(missing.validate(), Err(ConfigError::Authority(_))));

        let mut mismatched = sample();
        mismatched.pool.authority_secret_key =
            "mkDLTBBRxdBv998612qipDYoTK3YUrqLe8uWw7gu3iXbSrn2p".into();
        assert!(matches!(
            mismatched.validate(),
            Err(ConfigError::Authority(_))
        ));

        let mut payout_only = sample();
        payout_only.pool.roles = vec!["payout".into()];
        payout_only.pool.authority_secret_key.clear();
        payout_only.pool.authority_public_key.clear();
        payout_only.validate().unwrap();
    }

    fn with_v2() -> Config {
        let mut cfg = sample();
        cfg.accounting.work_unit_version = 2;
        cfg.accounting.work_reference_target = Some(format!("7f{}", "ff".repeat(31)));
        cfg.accounting.share_work_multiplier = Some(2);
        cfg.sv1.enabled = false;
        cfg.extensions.share_accounting = false;
        cfg
    }

    #[test]
    fn v2_profile_uses_exact_generation_targets_and_network_work_window() {
        let cfg = with_v2();
        cfg.validate().unwrap();
        let policy = cfg.accounting.share_policy(Network::Regtest).unwrap();
        assert_eq!(policy.window_target(0x200f_ffff, 8).unwrap(), 65);
        let initial = policy.job_work(0x200f_ffff, 1).unwrap();
        assert_eq!(initial.m_issue, 2);
        let mut target = [0u8; 32];
        target[0] = 0x3f;
        target[1..].fill(0xff);
        assert_eq!(initial.share_target.to_be_bytes(), target);

        let easier = policy.job_work(0x207f_ffff, 2).unwrap();
        assert_eq!(easier.m_issue, 1);
        assert!(easier.stepped_down);
        assert_eq!(initial.m_issue, 2, "old generation is immutable");
        let stepped_up = policy.job_work(0x200f_ffff, 3).unwrap();
        assert_eq!(stepped_up.m_issue, 2);
        assert!(!stepped_up.stepped_down);
    }

    #[test]
    fn v2_rejects_conflicting_unsupported_and_out_of_family_config() {
        let mut legacy_alias = with_v2();
        legacy_alias.accounting.share_difficulty = Some(1);
        assert!(matches!(
            legacy_alias.validate(),
            Err(ConfigError::WorkUnit(_))
        ));

        let mut network = with_v2();
        network.pool.network = "testnet4".into();
        assert!(matches!(network.validate(), Err(ConfigError::WorkUnit(_))));

        let mut multiplier = with_v2();
        multiplier.accounting.share_work_multiplier = Some(3);
        assert!(matches!(
            multiplier.validate(),
            Err(ConfigError::WorkUnit(_))
        ));

        let mut rule = with_v2();
        rule.accounting.window_rule = Some("share_work_legacy".into());
        assert!(matches!(rule.validate(), Err(ConfigError::WorkUnit(_))));

        let mut cap = with_v2();
        cap.accounting.max_work_multiplier = (1u64 << 32) + 1;
        assert!(matches!(cap.validate(), Err(ConfigError::WorkUnit(_))));

        let mut sv1 = with_v2();
        sv1.sv1.enabled = true;
        assert!(matches!(sv1.validate(), Err(ConfigError::WorkUnit(_))));
        let mut type32 = with_v2();
        type32.extensions.share_accounting = true;
        assert!(matches!(type32.validate(), Err(ConfigError::WorkUnit(_))));

        let mut bad_reference = with_v2();
        bad_reference.accounting.work_reference_target = Some("3f".repeat(32));
        assert!(matches!(
            bad_reference.validate(),
            Err(ConfigError::WorkUnit(_))
        ));

        // R = 2^254 - 1 is valid family but below actual regtest pow limit.
        // A mainnet-style 0x1d00ffff override must not make it look valid.
        let mut wrong_authority = with_v2();
        wrong_authority.accounting.work_reference_target = Some(format!("003f{}", "ff".repeat(30)));
        wrong_authority.accounting.pow_limit_bits = 0x1d00ffff;
        assert!(matches!(
            wrong_authority.validate(),
            Err(ConfigError::WorkUnit(message))
                if message.contains("regtest consensus 0x207fffff")
        ));
    }

    #[test]
    fn v1_profile_does_not_claim_the_regtest_pow_limit() {
        let mut cfg = sample();
        cfg.pool.network = "testnet4".into();
        assert!(cfg.accounting.profile_json()["pow_limit_bits"].is_null());
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
        assert_eq!(cfg.accounting.legacy_share_difficulty(), 1);
        let mut cfg = sample();
        cfg.accounting.share_difficulty = Some(0);
        assert_eq!(cfg.validate(), Err(ConfigError::ShareDifficulty));
        let mut cfg = sample();
        cfg.accounting.share_difficulty = Some(16_384);
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

    fn with_extra(extra: &str) -> Config {
        toml::from_str(&format!("{SAMPLE}\n{extra}")).unwrap()
    }

    #[test]
    fn reach_defaults_to_empty_and_validates() {
        let cfg = sample();
        cfg.validate().unwrap();
        assert!(cfg.reach.advertise.is_empty());
    }

    #[test]
    fn pool_config_rejects_net() {
        // [net] is agent-only (mining connections). A pool config that still
        // carries it must fail to parse, not silently ignore it.
        let text = format!("{SAMPLE}\n[net]\nproxy = \"socks5h://127.0.0.1:9050\"\n");
        assert!(toml::from_str::<Config>(&text).is_err());
    }

    #[test]
    fn onion_listeners_validate_when_all_loopback() {
        // jd.listen defaults to 0.0.0.0:34264, so pin it loopback here.
        let cfg = with_extra(
            r#"
[reach]
advertise = ["sv2://pool.example:34254", "sv2://abc.onion:34254"]
onion = { sv2 = "abc.onion:34254", sv1 = "def.onion:3333", jd = "ghi.onion:34264", api = "jkl.onion:8080" }

[jd]
listen = "127.0.0.1:34264"
"#,
        );
        cfg.validate().unwrap();
        assert_eq!(cfg.reach.advertise.len(), 2);
    }

    #[test]
    fn onion_listeners_off_loopback_are_refused_by_name() {
        for name in ["sv2", "sv1", "jd", "api"] {
            let mut cfg = sample();
            match name {
                "sv2" => {
                    cfg.reach.onion.sv2 = "abc.onion:34254".into();
                    cfg.pool.listen = "0.0.0.0:34254".into();
                }
                "sv1" => {
                    cfg.reach.onion.sv1 = "abc.onion:3333".into();
                    cfg.sv1.listen = "0.0.0.0:3333".into();
                }
                "jd" => {
                    cfg.reach.onion.jd = "abc.onion:34264".into();
                    cfg.jd.listen = "0.0.0.0:34264".into();
                }
                _ => {
                    cfg.reach.onion.api = "abc.onion:8080".into();
                    cfg.api.listen = "0.0.0.0:8080".into();
                }
            }
            let err = cfg.validate().unwrap_err();
            assert!(
                matches!(err, ConfigError::OnionNotLoopback(n, _, _) if n == name),
                "{name}: {err:?}"
            );
            assert!(err.to_string().contains("loopback"), "{err}");
        }
    }

    #[test]
    fn onion_entries_must_be_id_dot_onion_colon_port() {
        for bad in [
            "not-an-address",
            "abc.example:1",
            "abc.onion",
            "abc.onion:notaport",
        ] {
            let mut cfg = sample();
            cfg.reach.onion.sv2 = bad.into();
            assert!(
                matches!(cfg.validate(), Err(ConfigError::OnionAdvertised("sv2", _))),
                "{bad}"
            );
        }
    }

    #[test]
    fn onion_loopback_applies_even_when_the_listener_is_disabled() {
        // sv1 disabled, its onion entry set, listen off loopback: refuse.
        // The §3.8 rule binds the published address, not the runtime role.
        let mut cfg = sample();
        cfg.sv1.enabled = false;
        cfg.sv1.listen = "0.0.0.0:3333".into();
        cfg.reach.onion.sv1 = "abc.onion:3333".into();
        assert!(
            matches!(
                cfg.validate(),
                Err(ConfigError::OnionNotLoopback("sv1", "sv1", _))
            ),
            "disabled SV1 with an onion entry and a clearnet bind must not pass"
        );
        // Same shape bound to loopback is fine (e.g. preparing to enable).
        let mut cfg = sample();
        cfg.sv1.enabled = false;
        cfg.sv1.listen = "127.0.0.1:3333".into();
        cfg.reach.onion.sv1 = "abc.onion:3333".into();
        cfg.validate().unwrap();
    }

    fn ark_enabled() -> Config {
        let mut cfg = sample();
        cfg.ark = Some(Ark {
            enabled: true,
            server: "https://ark.example.tech".into(),
            ark_id: "00000000".into(),
            server_pubkey: "02".repeat(33),
            mailbox_pubkey: "03".repeat(33),
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
