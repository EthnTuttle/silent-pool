//! Frozen lab transaction traffic contract (traffic plan packet T0).
//!
//! This module owns the complete versioned traffic vocabulary:
//!
//! - [`spec`]: the frozen `TrafficSpec` scenario schema, defaults, ranges,
//!   cross-field validation and error codes (traffic plan Section 11).
//!   T1 (runtime) and T2-L (bootstrap) implement behavior *against* this
//!   schema; later packets never create pieces of the same type.
//! - [`protocol`]: the `spc-lab-tx-control/v1` hello/capability exchange,
//!   the versioned private history record contract, the transaction
//!   lifecycle state vocabulary (Section 6.5) and the deterministic seed
//!   derivation domains (Sections 5 and 6.2).
//! - [`commands`]: the typed durable command payloads (Section 12) that T1
//!   wires into `CommandKind` under the existing command-id idempotency
//!   and conflict semantics.
//! - [`projection`]: the AP-1a traffic capability projection (Section 8) -
//!   the mapping of the canonical `ap1a-ark-rpc-probe` report into the
//!   run manifest - and the scenario-side preflight gate. No probe
//!   execution and no second verdict live here.
//!
//! Nothing in this module launches a process, signs, broadcasts or mines;
//! those are T1/T2-L behavior against these frozen types.

pub mod commands;
pub mod completion;
pub mod consumer;
pub mod control;
pub mod launch_profile;
pub mod payment_view;
pub mod projection;
pub mod protocol;
pub mod rpc;
pub mod runtime;
pub mod spec;

pub use projection::{
    check_traffic_capability, expected_ids_for_method, project_traffic_capability, Ap1aCheckState,
    Ap1aReportInput, Ap1aVerdictSummary, ExpectedCheck, TrafficCapabilityRecord,
    TrafficCheckStatus, TrafficMethodStatus, AP1A_APPROVED_CONTRACT_SHA256S, AP1A_PROBE_ID,
    AP1A_RECOGNIZED_UNAPPROVED_CANDIDATES, AP1A_REPORT_SCHEMA_VERSION,
    TRAFFIC_CAPABILITY_RECORD_SCHEMA_VERSION, TRAFFIC_EXPECTED_CHECKS, TRAFFIC_EXPECTED_NETWORK,
    TRAFFIC_FOUNDATIONAL_CHECKS, TRAFFIC_METHODS, TRAFFIC_TARGET_KIND_OWNED,
    TRAFFIC_TARGET_KIND_REMOTE,
};
pub use protocol::{
    network_wallet_seed_domain, parse_control_protocol, user_payment_wallet_seed_domain,
    TrafficHistoryCaps, TrafficHistoryHeader, TxControlHello, TxControlHelloAck, TxLifecycleState,
    SEED_DOMAIN_BOOTSTRAP_FUNDING, SEED_DOMAIN_NETWORK_WALLET_PREFIX, SEED_DOMAIN_PLANNED_SCHEDULE,
    SEED_DOMAIN_USER_PAYMENT_WALLET_PREFIX, TRAFFIC_HISTORY_NETWORK,
    TRAFFIC_HISTORY_REORG_RESUBMITS_MAX, TRAFFIC_HISTORY_REORG_RESUBMITS_MIN,
    TRAFFIC_HISTORY_UNKNOWN_RETRY_SENDS_MAX, TRAFFIC_HISTORY_VERSION,
    TRAFFIC_KEY_DERIVATION_VERSION, TRAFFIC_KEY_MODE, TX_CONTROL_CAPABILITIES_V1,
    TX_CONTROL_PROTOCOL_MAJOR, TX_CONTROL_PROTOCOL_MINOR, TX_CONTROL_PROTOCOL_NAME,
    TX_CONTROL_PROTOCOL_STRING, TX_JOURNAL_FORMAT_MAGIC, TX_JOURNAL_PROTOCOL_MAJOR,
    TX_JOURNAL_PROTOCOL_NAME, TX_JOURNAL_PROTOCOL_STRING,
};
pub use spec::{
    TrafficSpec, FUNDING_BLOCKS_MAX, FUNDING_BLOCKS_MIN, TRAFFIC_CAPABILITY, TRAFFIC_SPEC_VERSION,
};

/// A traffic-contract failure with a stable code, mirroring the
/// [`crate::scenario::ScenarioError`] shape for runtime-facing traffic
/// types (protocol, commands, projection).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {detail}")]
pub struct TrafficError {
    code: &'static str,
    detail: String,
}

impl TrafficError {
    pub fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }

    pub fn code(&self) -> &str {
        self.code
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }
}
