//! `spc-lab-tx-control/v1` protocol contract, history-version, lifecycle
//! vocabulary and seed domains (traffic plan Sections 5, 6.1-6.5).
//!
//! T0 owns these typed contracts; T1 implements the runtime behavior
//! (the JSONL control loop, the journal, the actor) against them. Nothing
//! here launches a process or signs anything.
//!
//! Two protocols are named here and never conflated:
//!
//! - the **control protocol** `spc-lab-tx-control/v1` (the controller
//!   channel: hello/ack, capabilities) - still major version 1; and
//! - the **journal protocol** `spc-lab-tx-journal/3` (the on-disk
//!   private history record contract of
//!   `<run_dir>/private/traffic/history.jsonl`) - version 3, aligned with
//!   the Fable-approved T1a core (`crates/spc-lab-txd`,
//!   `JournalRecord::Header`), which mandates the network / key-mode /
//!   derivation-version / caps / allocation-digest binding that the
//!   retired 4-field v1 header could not carry.

use serde::{Deserialize, Serialize};

use crate::ids::{ActorId, RunId};
use crate::traffic::TrafficError;

/// Control protocol name. The scenario's `traffic.protocol.control`
/// string and every hello/ack exchange must carry it.
pub const TX_CONTROL_PROTOCOL_NAME: &str = "spc-lab-tx-control";

/// The only control major version v1 accepts. An unknown major version
/// fails before any broadcast (traffic plan 6.1).
pub const TX_CONTROL_PROTOCOL_MAJOR: u32 = 1;

/// Current control minor version.
pub const TX_CONTROL_PROTOCOL_MINOR: u32 = 0;

/// The canonical control protocol string carried by scenario specs.
pub const TX_CONTROL_PROTOCOL_STRING: &str = "spc-lab-tx-control/v1";

/// Version of the private, append-only, replayable transaction history
/// journal (`<run_dir>/private/traffic/history.jsonl`; traffic plan 6.3).
///
/// Version 3 adds the atomic proof-bearing rollback record. Header fields
/// retain the approved v2 shape; v1 and v2 are refused without migration.
/// The core and T0 both reject unknown fields. Key derivation remains v2.
pub const TRAFFIC_HISTORY_VERSION: u32 = 3;

/// The journal protocol name: the on-disk history record contract,
/// distinct from [`TX_CONTROL_PROTOCOL_NAME`] (the controller channel).
pub const TX_JOURNAL_PROTOCOL_NAME: &str = "spc-lab-tx-journal";

/// The only journal history-contract major version T0 accepts; must
/// equal the T1a core's `SUPPORTED_HISTORY_VERSION`.
pub const TX_JOURNAL_PROTOCOL_MAJOR: u32 = 3;

/// The canonical journal protocol string recorded in the history-3
/// header (and only there - never in a hello/ack exchange).
pub const TX_JOURNAL_PROTOCOL_STRING: &str = "spc-lab-tx-journal/3";

/// The file-format magic of the journal header: identifies the file
/// as the traffic journal and rejects pre-rework candidate files (which
/// have no magic field at all). Must equal the T1a core's
/// `FORMAT_MAGIC`.
pub const TX_JOURNAL_FORMAT_MAGIC: &str = "SPC-LAB-TRAFFIC-JOURNAL";

/// The declared lab key mode recorded in the history header and the
/// private key file: traffic wallet keys are deterministically derived
/// from the PUBLIC scenario seed bound to the run id - publicly
/// reproducible lab keys, regtest-only, with NO secrecy claim (the
/// mode-0600 files protect nothing the scenario does not already
/// reveal). Must equal the T1a core's `KEY_MODE`.
pub const TRAFFIC_KEY_MODE: &str = "lab_deterministic_run_seed_v1";

/// The traffic key derivation scheme version recorded in the history
/// header: version 2 puts the explicit run id in the HMAC domain. Must
/// equal the T1a core's `DERIVATION_VERSION`.
pub const TRAFFIC_KEY_DERIVATION_VERSION: u32 = 2;

/// The only network a traffic journal may name: the lab's single
/// network (the traffic capability itself is
/// `spc.node.regtest.mempool_rpc.v1`, and the core's derivation refuses
/// non-regtest outright).
pub const TRAFFIC_HISTORY_NETWORK: &str = "regtest";

/// Capabilities a v1 controller may rely on.
///
/// Runtime operations understood by control v1. Bootstrap execution still
/// requires a configured T2 provider; a capability is not a funding proof.
pub const TX_CONTROL_CAPABILITIES_V1: [&str; 5] = [
    "payment_wallet_read_v1",
    "ordinary_churn",
    "user_payment",
    "inject_tx",
    "bootstrap_fund",
];

/// Deterministic seed derivation domains (traffic plan Sections 5 and
/// 6.2). The lab deterministic key mode derives every traffic wallet
/// key from the PUBLIC scenario seed (`RunSpec.seed`) bound to the run
/// id (`RunSpec.id`) through these HMAC-SHA256 domains: lab traffic
/// keys are publicly reproducible by construction and make no secrecy
/// claim. The exact per-domain input tuple (domain string, run binding,
/// actor/global index) is T1b's deterministic disjoint allocation to
/// define over these frozen domain strings - never a new string, never
/// a forked type. SPc identity keys and Ark wallet keys never come
/// from them.
pub const SEED_DOMAIN_BOOTSTRAP_FUNDING: &str = "spc-lab/traffic/bootstrap-funding";
pub const SEED_DOMAIN_NETWORK_WALLET_PREFIX: &str = "spc-lab/traffic/network-wallet/";
pub const SEED_DOMAIN_USER_PAYMENT_WALLET_PREFIX: &str = "spc-lab/traffic/user-payment-wallet/";
/// The planned-schedule domain (traffic plan 6.3): the reproducible
/// arrival schedule derives from `(run_seed, protocol_version,
/// driver_id, sequence)` in integer/rational time units with bounded
/// seed-derived jitter. Frozen here so T1 never invents the string; the
/// four inputs are the HMAC key material in exactly that order.
pub const SEED_DOMAIN_PLANNED_SCHEDULE: &str = "spc-lab/traffic/planned-schedule";

/// Seed domain of background network wallet `index`.
pub fn network_wallet_seed_domain(index: u64) -> String {
    format!("{SEED_DOMAIN_NETWORK_WALLET_PREFIX}{index}")
}

/// Seed domain of SPc user payment wallet `index`.
pub fn user_payment_wallet_seed_domain(index: u64) -> String {
    format!("{SEED_DOMAIN_USER_PAYMENT_WALLET_PREFIX}{index}")
}

/// Parses a control protocol string `<name>/v<major>`. Returns the major
/// version when the name matches and the form is exact; anything else
/// (wrong name, wrong form, minor suffixes) is unsupported in the frozen
/// v1 vocabulary.
pub fn parse_control_protocol(value: &str) -> Option<u32> {
    let rest = value.strip_prefix(TX_CONTROL_PROTOCOL_NAME)?;
    let version = rest.strip_prefix('/')?;
    let major = version.strip_prefix('v')?;
    if major.is_empty() || !major.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    major.parse::<u32>().ok()
}

/// The driver's opening hello/capability exchange (traffic plan 6.1).
///
/// The controller validates it before any broadcast: an unknown protocol
/// name or major version fails the exchange outright.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TxControlHello {
    pub protocol_name: String,
    pub protocol_major: u32,
    pub protocol_minor: u32,
    /// The driver actor's id (the scenario's `driver.actors[].id`
    /// namespace, validated through the shared [`ActorId`] rules:
    /// at most 128 bytes, ASCII, no control characters).
    pub driver_id: ActorId,
    /// Background network wallets hosted by this actor.
    pub network_wallets: u32,
    /// SPc user payment wallets hosted by this actor.
    pub user_payment_wallets: u32,
    /// Capabilities the driver advertises. Advertising a capability the
    /// frozen v1 registry does not know never makes the controller rely
    /// on it (see [`TX_CONTROL_CAPABILITIES_V1`]).
    pub capabilities: Vec<String>,
}

impl TxControlHello {
    /// Fail-closed hello validation: exact protocol name, known major
    /// version, non-empty driver id, at least one wallet of each kind,
    /// non-empty capability names.
    pub fn validate(&self) -> Result<(), TrafficError> {
        if self.protocol_name != TX_CONTROL_PROTOCOL_NAME {
            return Err(TrafficError::new(
                "traffic_protocol_unsupported",
                format!(
                    "hello protocol_name {:?} is not {TX_CONTROL_PROTOCOL_NAME}",
                    self.protocol_name
                ),
            ));
        }
        if self.protocol_major != TX_CONTROL_PROTOCOL_MAJOR {
            return Err(TrafficError::new(
                "traffic_protocol_unsupported",
                format!(
                    "hello protocol major {} is unsupported (expected \
                     {TX_CONTROL_PROTOCOL_MAJOR}); failing before any broadcast",
                    self.protocol_major
                ),
            ));
        }
        if let Err(e) = ActorId::new(self.driver_id.as_str()) {
            return Err(TrafficError::new(
                "traffic_driver_id_invalid",
                format!("hello driver_id is not a valid actor id: {e}"),
            ));
        }
        if self.network_wallets == 0 || self.user_payment_wallets == 0 {
            return Err(TrafficError::new(
                "traffic_hello_wallet_count_invalid",
                format!(
                    "hello wallet counts must be at least 1 (network {}, user payment {})",
                    self.network_wallets, self.user_payment_wallets
                ),
            ));
        }
        if self.capabilities.iter().any(|c| c.trim().is_empty()) {
            return Err(TrafficError::new(
                "traffic_hello_capability_invalid",
                "hello advertises an empty capability name",
            ));
        }
        Ok(())
    }

    /// True only when the frozen v1 registry knows the capability AND this
    /// driver advertised it. Never trusts an unregistered advertisement.
    pub fn supports(&self, capability: &str) -> bool {
        TX_CONTROL_CAPABILITIES_V1.contains(&capability)
            && self.capabilities.iter().any(|c| c == capability)
    }
}

/// The controller's hello acknowledgment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TxControlHelloAck {
    pub protocol_name: String,
    pub protocol_major: u32,
    pub protocol_minor: u32,
    pub accepted: bool,
    /// The history record contract version the controller will journal
    /// under; must equal [`TRAFFIC_HISTORY_VERSION`] on acceptance.
    pub history_version: u32,
    /// Rejection reason when `accepted` is false.
    #[serde(default)]
    pub reason: Option<String>,
}

impl TxControlHelloAck {
    pub fn validate(&self) -> Result<(), TrafficError> {
        if self.protocol_name != TX_CONTROL_PROTOCOL_NAME {
            return Err(TrafficError::new(
                "traffic_protocol_unsupported",
                format!(
                    "hello-ack protocol_name {:?} is not {TX_CONTROL_PROTOCOL_NAME}",
                    self.protocol_name
                ),
            ));
        }
        if self.protocol_major != TX_CONTROL_PROTOCOL_MAJOR {
            return Err(TrafficError::new(
                "traffic_protocol_unsupported",
                format!(
                    "hello-ack protocol major {} is unsupported (expected \
                     {TX_CONTROL_PROTOCOL_MAJOR})",
                    self.protocol_major
                ),
            ));
        }
        if self.accepted && self.history_version != TRAFFIC_HISTORY_VERSION {
            return Err(TrafficError::new(
                "traffic_protocol_unsupported",
                format!(
                    "accepted hello-ack carries history_version {} (expected \
                     {TRAFFIC_HISTORY_VERSION}); traffic-history versions 1 and 2 \
                     are refused before any effect, never migrated",
                    self.history_version
                ),
            ));
        }
        if !self.accepted
            && self
                .reason
                .as_deref()
                .map(str::trim)
                .unwrap_or("")
                .is_empty()
        {
            return Err(TrafficError::new(
                "traffic_hello_ack_invalid",
                "rejected hello-ack carries no reason",
            ));
        }
        Ok(())
    }
}

/// The retry caps persisted in the history header (traffic plan 6.5 /
/// T1a `JournalCaps`): `unknown_retry_sends = N` admits at most N retry
/// **sends** from the `unknown` state (N plus the original = at most N+1
/// total sends of the same bytes) while unknown **outcomes** are always
/// recordable; `reorg_resubmits` bounds same-bytes reorg resubmission.
///
/// These are the same numeric policy as the scenario `traffic.caps`
/// fields (`max_unknown_retries`, `reorg_resubmit_max`): the scenario
/// declares them, the header persists them, and the journal revalidates
/// them on every reopen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrafficHistoryCaps {
    /// Maximum retry sends from `unknown`: 0..=3.
    pub unknown_retry_sends: u32,
    /// Maximum same-bytes reorg resubmissions: 1..=3.
    pub reorg_resubmits: u32,
}

/// Upper bound of [`TrafficHistoryCaps::unknown_retry_sends`] (equal to
/// the scenario `max_unknown_retries` ceiling).
pub const TRAFFIC_HISTORY_UNKNOWN_RETRY_SENDS_MAX: u32 = 3;
/// Lower bound of [`TrafficHistoryCaps::reorg_resubmits`].
pub const TRAFFIC_HISTORY_REORG_RESUBMITS_MIN: u32 = 1;
/// Upper bound of [`TrafficHistoryCaps::reorg_resubmits`] (equal to the
/// scenario `reorg_resubmit_max` ceiling).
pub const TRAFFIC_HISTORY_REORG_RESUBMITS_MAX: u32 = 3;

impl TrafficHistoryCaps {
    /// Fail-closed cap range validation (mirrors the T1a core's
    /// `JournalCaps::validate` exactly: out-of-range values reject,
    /// never clamp).
    pub fn validate(&self) -> Result<(), TrafficError> {
        if self.unknown_retry_sends > TRAFFIC_HISTORY_UNKNOWN_RETRY_SENDS_MAX {
            return Err(TrafficError::new(
                "traffic_caps_invalid",
                format!(
                    "history caps unknown_retry_sends {} is outside 0..={} (retry sends bound, \
                     never outcome recording)",
                    self.unknown_retry_sends, TRAFFIC_HISTORY_UNKNOWN_RETRY_SENDS_MAX
                ),
            ));
        }
        if self.reorg_resubmits < TRAFFIC_HISTORY_REORG_RESUBMITS_MIN
            || self.reorg_resubmits > TRAFFIC_HISTORY_REORG_RESUBMITS_MAX
        {
            return Err(TrafficError::new(
                "traffic_caps_invalid",
                format!(
                    "history caps reorg_resubmits {} is outside {}..={}",
                    self.reorg_resubmits,
                    TRAFFIC_HISTORY_REORG_RESUBMITS_MIN,
                    TRAFFIC_HISTORY_REORG_RESUBMITS_MAX
                ),
            ));
        }
        Ok(())
    }
}

/// The exact first line of the private driver history journal (traffic
/// plan 6.3): the FULL history-3 header record as the core
/// writes it, mirrored field-for-field - including the `record` tag and
/// the `seq` - so this type IS the first line, not a projection of it.
///
/// Serialization is byte-compatible with the T1a `JournalRecord::Header`
/// line (same tag, same field order, transparent `RunId`/`ActorId`
/// strings), which the conformance suite proves against a real journal
/// written by the core. Both this strict struct and the history-3 core
/// reject unknown fields and missing required fields at parse time.
///
/// Field semantics (frozen):
///
/// - `record`/`seq`: the record kind `"header"` at sequence 0 - always
///   the first line; any other tag or a nonzero `seq` is not a header.
/// - `format_magic`/`history_version`/`protocol`: the journal file
///   identity and history contract version 3
///   ([`TX_JOURNAL_PROTOCOL_STRING`], NOT the control protocol).
/// - `network`/`key_mode`/`derivation_version`: the declared binding -
///   regtest only, the public-seed lab key mode with no secrecy claim.
/// - `run_id`/`driver_id`: shared typed ids, re-validated (serde
///   deserialization of the transparent newtypes does not validate).
/// - `run_seed_fingerprint`: 64-hex SHA-256 of the run seed - a
///   provenance binding for reopen, NOT a cryptographic identity of
///   anything (do not confuse it with a 4-byte BIP32 fingerprint, and
///   do not read secrecy into it: the seed is public scenario data).
/// - `allocation_digest`: 64-hex SHA-256 over the canonical sorted-key
///   JSON of the complete validated wallet-set allocation, exactly as
///   computed by the T1a `ProvisionConfig::validate_allocation` - there
///   is no second hash and no subset derivation in T0.
/// - `caps`: the persisted retry caps ([`TrafficHistoryCaps`]).
/// - `created_unix_ms`: wall-clock creation time as u64 milliseconds
///   since the Unix epoch.
///
/// The header fields are **context and binding metadata, not approval
/// and not proven funding**: a valid header never proves the capability
/// gate, the funded state, or any mempool/chain outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrafficHistoryHeader {
    /// Record kind: exactly `"header"`.
    pub record: String,
    /// Journal sequence: the header is always the first record (seq 0).
    pub seq: u64,
    pub format_magic: String,
    pub history_version: u32,
    pub protocol: String,
    pub network: String,
    pub key_mode: String,
    pub derivation_version: u32,
    pub run_id: RunId,
    /// The driver actor's id (shared [`ActorId`] rules).
    pub driver_id: ActorId,
    pub run_seed_fingerprint: String,
    pub allocation_digest: String,
    pub caps: TrafficHistoryCaps,
    /// Creation time: u64 milliseconds since the Unix epoch.
    pub created_unix_ms: u64,
}

impl TrafficHistoryHeader {
    /// Parse and validate one line of `traffic/history.jsonl` as the history-3
    /// header. Only the FIRST line of the journal may be handed here;
    /// the parse is strict (`deny_unknown_fields`, no defaults, so a
    /// missing field or an unknown extra field rejects), and every
    /// constant, id, digest and cap bound is then re-checked by
    /// [`Self::validate`]. History versions 1 and 2 are refused, never migrated.
    pub fn from_journal_line(line: &str) -> Result<Self, TrafficError> {
        let header: Self = serde_json::from_str(line).map_err(|e| {
            TrafficError::new(
                "traffic_history_header_unparseable",
                format!(
                    "the history journal line is not a strict history-3 header record (unknown or \
                     missing field, wrong type, or not an object): {e}"
                ),
            )
        })?;
        header.validate()?;
        Ok(header)
    }

    /// Fail-closed header validation: record kind, seq 0, format magic,
    /// history version 3, journal protocol string, regtest network,
    /// declared key mode and derivation version, typed id rules,
    /// 64-hex digests, and cap bounds. Binding the header against an
    /// expected run/driver/allocation/caps is the consumer's reopen
    /// concern (the T1a core enforces it on every reopen before any
    /// mutation): this method validates the contract, not the
    /// possession.
    pub fn validate(&self) -> Result<(), TrafficError> {
        if self.record != "header" {
            return Err(TrafficError::new(
                "traffic_history_record_invalid",
                format!(
                    "history record kind {:?} is not \"header\" (the first journal line must be \
                     the history-3 header)",
                    self.record
                ),
            ));
        }
        if self.seq != 0 {
            return Err(TrafficError::new(
                "traffic_history_record_invalid",
                format!(
                    "history header seq {} is not 0 (the header is always the first journal \
                     record)",
                    self.seq
                ),
            ));
        }
        if self.format_magic != TX_JOURNAL_FORMAT_MAGIC {
            return Err(TrafficError::new(
                "traffic_history_record_invalid",
                format!(
                    "history header format_magic {:?} is not {TX_JOURNAL_FORMAT_MAGIC} (a \
                     pre-rework candidate journal without the magic is refused, never \
                     reinterpreted)",
                    self.format_magic
                ),
            ));
        }
        if self.history_version != TRAFFIC_HISTORY_VERSION {
            return Err(TrafficError::new(
                "traffic_protocol_unsupported",
                format!(
                    "history_version {} is unsupported (expected {TRAFFIC_HISTORY_VERSION}); the \
                     traffic-history versions 1 and 2 are refused, never migrated",
                    self.history_version
                ),
            ));
        }
        if self.protocol != TX_JOURNAL_PROTOCOL_STRING {
            return Err(TrafficError::new(
                "traffic_protocol_unsupported",
                format!(
                    "history header protocol {:?} is not the journal protocol \
                     {TX_JOURNAL_PROTOCOL_STRING} (the control protocol {TX_CONTROL_PROTOCOL_STRING} \
                     belongs to the hello/ack channel, not the journal)",
                    self.protocol
                ),
            ));
        }
        if self.network != TRAFFIC_HISTORY_NETWORK {
            return Err(TrafficError::new(
                "traffic_history_network_invalid",
                format!(
                    "history header network {:?} is not the lab network \
                     {TRAFFIC_HISTORY_NETWORK:?}",
                    self.network
                ),
            ));
        }
        if self.key_mode != TRAFFIC_KEY_MODE {
            return Err(TrafficError::new(
                "traffic_key_mode_unsupported",
                format!(
                    "history header key_mode {:?} is not the declared lab key mode \
                     {TRAFFIC_KEY_MODE} (public run seed bound to the run id - no secrecy claim)",
                    self.key_mode
                ),
            ));
        }
        if self.derivation_version != TRAFFIC_KEY_DERIVATION_VERSION {
            return Err(TrafficError::new(
                "traffic_key_mode_unsupported",
                format!(
                    "history header derivation_version {} is unsupported (expected \
                     {TRAFFIC_KEY_DERIVATION_VERSION}: the run id is in the HMAC domain)",
                    self.derivation_version
                ),
            ));
        }
        if let Err(e) = RunId::new(self.run_id.as_str()) {
            return Err(TrafficError::new(
                "traffic_run_id_invalid",
                format!("history header run_id is not a valid run id: {e}"),
            ));
        }
        if let Err(e) = ActorId::new(self.driver_id.as_str()) {
            return Err(TrafficError::new(
                "traffic_driver_id_invalid",
                format!("history header driver_id is not a valid actor id: {e}"),
            ));
        }
        validate_digest_hex64("run_seed_fingerprint", &self.run_seed_fingerprint)?;
        validate_digest_hex64("allocation_digest", &self.allocation_digest)?;
        self.caps.validate()?;
        Ok(())
    }
}

/// A digest field of the header: exactly 64 lowercase hex characters
/// (the canonical SHA-256 hex form the T1a core writes). Wrong length,
/// uppercase, or non-hex characters reject - never a trimmed or
/// case-folded acceptance.
fn validate_digest_hex64(field: &'static str, value: &str) -> Result<(), TrafficError> {
    let valid = value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if !valid {
        return Err(TrafficError::new(
            "traffic_history_digest_invalid",
            format!(
                "history header {field} is not a canonical 64-character lowercase hex SHA-256 \
                 digest (got {} characters)",
                value.len()
            ),
        ));
    }
    Ok(())
}

/// The journaled transaction lifecycle state vocabulary (traffic plan
/// 6.5). Each transition names its observing node and time; `unknown` is
/// a first-class state, never silently resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TxLifecycleState {
    /// Signed, txid and raw bytes recorded.
    Created,
    BroadcastSent,
    /// RPC success only - not canonical, not even cross-node admission.
    AcceptedByRpc,
    /// `getrawmempool` membership on at least one declared node.
    ObservedInMempool,
    /// Txid found in a canonical block (time-scoped, node-scoped).
    Confirmed,
    /// Only ever set from the node's explicit error.
    Rejected,
    /// Conflicting spend admitted; terminal - not another retry loop.
    Conflicted,
    /// Timeout/transport/shape failures; bounded identical-bytes retry.
    Unknown,
    /// Canonical displacement; reconciled from canonical blocks.
    Reorged,
    /// Canonical displacement (alias row of the same transition family).
    Invalidated,
    /// Later profile (T6). Its semantics - including whether it is
    /// terminal - are **UNDEFINED until T6 specifies them**: no
    /// implementation may set this state before that, and nothing may
    /// infer a non-terminal (or terminal) reading from this enum. T0
    /// freezes the vocabulary slot only.
    Dropped,
    /// Later profile (T6). Same as [`TxLifecycleState::Dropped`]:
    /// terminality and expiry semantics are undefined until T6; never
    /// set, never silently resolved.
    Expired,
}

impl TxLifecycleState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::BroadcastSent => "broadcast_sent",
            Self::AcceptedByRpc => "accepted_by_rpc",
            Self::ObservedInMempool => "observed_in_mempool",
            Self::Confirmed => "confirmed",
            Self::Rejected => "rejected",
            Self::Conflicted => "conflicted",
            Self::Unknown => "unknown",
            Self::Reorged => "reorged",
            Self::Invalidated => "invalidated",
            Self::Dropped => "dropped",
            Self::Expired => "expired",
        }
    }

    /// Terminal states never leave the state machine: `rejected` is a
    /// definitive node answer (retries are for `unknown` only) and
    /// `conflicted` is terminal by contract. `confirmed` is not terminal:
    /// canonical displacement moves it to `reorged`/`invalidated` for
    /// bounded same-bytes reconciliation.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Rejected | Self::Conflicted)
    }
}
