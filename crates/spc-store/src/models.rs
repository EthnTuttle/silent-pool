//! Durable row types. The schema is `migrations/0001_initial.sql` (SPEC §5.1).

use serde::{Deserialize, Serialize};

fn hex_bytes(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub use spc_accounting::work_unit::{WindowRule, WorkUnitVersion};

/// V2 slice-credit retention horizon in network heights. This applies to the
/// row's stored work-unit version, never the process's configured profile.
/// V1 rows continue to use `[accounting] retain_windows` and its legacy guards.
pub const V2_RETENTION_HEIGHT_BLOCKS: u32 = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpIdentityRow {
    pub sp_id: [u8; 32],
    pub b_scan: Vec<u8>,
    pub b_spend: Vec<u8>,
    /// 0 mainnet, 1 testnet4, 2 signet, 3 regtest.
    pub network: i16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MailboxMode {
    None = 0,
    Static = 1,
    EpochDerived = 2,
    EpochRegistered = 3,
}

impl MailboxMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Static => "static",
            Self::EpochDerived => "epoch_derived",
            Self::EpochRegistered => "epoch_registered",
        }
    }

    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "none" => Some(Self::None),
            "static" => Some(Self::Static),
            "epoch_derived" => Some(Self::EpochDerived),
            "epoch_registered" => Some(Self::EpochRegistered),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArkDeliveryRow {
    /// Observation metadata only; never part of 0x5350 or Ark-server RPCs.
    pub registration_commit: Option<Box<RegistrationCommit>>,
    pub sp_id: [u8; 32],
    pub ark_id: [u8; 4],
    pub b_ark: Vec<u8>,
    pub mailbox_mode: MailboxMode,
    pub mailbox_m: Option<Vec<u8>>,
    pub mailbox_d: Option<Vec<u8>>,
    pub epoch_fallback: EpochFallback,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistrationCommit {
    pub request_id: u32,
    pub registration_rev: u64,
    pub signed_fields_digest: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EpochFallback {
    Static = 0,
    Hold = 1,
}

impl EpochFallback {
    pub fn from_i16(value: i16) -> Option<Self> {
        match value {
            0 => Some(Self::Static),
            1 => Some(Self::Hold),
            _ => None,
        }
    }

    pub fn as_i16(self) -> i16 {
        self as i16
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArkEpochMailboxRow {
    pub sp_id: [u8; 32],
    pub height: u32,
    pub mailbox_mi: [u8; 33],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegistrationEffect {
    Delivery {
        identity: SpIdentityRow,
        delivery: ArkDeliveryRow,
    },
    EpochMailboxes {
        sp_id: [u8; 32],
        entries: Vec<ArkEpochMailboxRow>,
    },
}

impl RegistrationEffect {
    pub fn sp_id(&self) -> [u8; 32] {
        match self {
            Self::Delivery { identity, .. } => identity.sp_id,
            Self::EpochMailboxes { sp_id, .. } => *sp_id,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistrationOutcome {
    Applied,
    AlreadyApplied,
    Superseded,
    Mismatch,
    Stale,
    /// A validly parsed/signed request rejected before any delivery effect.
    Rejected,
    RequestIdExhausted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistrationResolution {
    pub sp_id: [u8; 32],
    pub request_id: u32,
    pub signed_fields_digest: [u8; 32],
    pub outcome: RegistrationOutcome,
    pub registration_rev: u64,
    /// Exact terminal code for `Rejected`; named replay outcomes map to their
    /// canonical wire errors instead.
    pub rejection_code: Option<&'static str>,
}

impl RegistrationResolution {
    pub fn error_code(&self) -> Option<&'static str> {
        if let Some(code) = self.rejection_code {
            return Some(code);
        }
        match self.outcome {
            RegistrationOutcome::Mismatch => Some("registration-mismatch"),
            RegistrationOutcome::Stale => Some("stale-request"),
            RegistrationOutcome::RequestIdExhausted => Some("request-id-exhausted"),
            _ => None,
        }
    }

    pub fn is_success(&self) -> bool {
        matches!(
            self.outcome,
            RegistrationOutcome::Applied
                | RegistrationOutcome::AlreadyApplied
                | RegistrationOutcome::Superseded
        )
    }
}

/// JSON value stored in the SPc-side `spc:arkreg:ack` Redis hash.
/// This is an expiring notification cache, never registration authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArkRegistrationAck {
    pub sp_id: String,
    pub request_id: u32,
    pub signed_fields_digest: String,
    pub registration_rev: u64,
    pub state: String,
    /// Freshly generated on each front handle; cache correlation only, never a
    /// durable operation ID or an additional registration effect.
    pub attempt: String,
    pub error_code: Option<String>,
    pub expires_at: u64,
}

impl ArkRegistrationAck {
    pub fn from_resolution(
        resolution: &RegistrationResolution,
        attempt: &[u8; 16],
        expires_at: u64,
    ) -> Self {
        let state = match resolution.outcome {
            RegistrationOutcome::Applied => "applied",
            RegistrationOutcome::AlreadyApplied => "already_applied",
            RegistrationOutcome::Superseded => "superseded",
            RegistrationOutcome::Mismatch => "mismatch",
            RegistrationOutcome::Stale => "stale",
            RegistrationOutcome::Rejected => "rejected",
            RegistrationOutcome::RequestIdExhausted => "request_id_exhausted",
        };
        Self {
            sp_id: hex_bytes(resolution.sp_id),
            request_id: resolution.request_id,
            signed_fields_digest: hex_bytes(resolution.signed_fields_digest),
            registration_rev: resolution.registration_rev,
            state: state.to_string(),
            attempt: hex_bytes(attempt),
            error_code: resolution.error_code().map(str::to_string),
            expires_at,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SliceRow {
    pub slice_id: u64,
    pub height: u32,
    pub prev_hash: [u8; 32],
    pub ref_job_id: u64,
    pub ref_fees: u64,
    pub n_shares: u32,
    pub work_unit_version: WorkUnitVersion,
    /// NULL in Postgres for v1; v2 carries the exact reference fingerprint.
    pub work_unit_fingerprint: Option<[u8; 32]>,
    /// u128 as a decimal string for `NUMERIC(40,0)`.
    pub difficulty: String,
    pub root: [u8; 32],
    pub shares: Vec<u8>,
    pub credits: Vec<SliceCreditRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SliceCreditRow {
    pub sp_id: [u8; 32],
    /// u128 decimal.
    pub diff_sum: String,
    /// u128 decimal.
    pub diff_fee_sum: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotRow {
    pub fingerprint: [u8; 32],
    pub height: u32,
    pub a_send_x: [u8; 32],
    pub distribution_id: Option<u64>,
    pub entries: serde_json::Value,
    pub high_water: serde_json::Value,
    pub work_unit_version: WorkUnitVersion,
    pub work_unit_fingerprint: Option<[u8; 32]>,
    pub window_rule: WindowRule,
    /// The exact window target (`N x d`, SPEC §4.3) the distribution was
    /// built against, as a u128 decimal string for `NUMERIC(40,0)`. `None`
    /// on rows written before the column existed; settlement then falls
    /// back to the warned slice estimate.
    pub window_target: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BlockStatus {
    Pending,
    Confirmed,
    Orphaned,
}

impl BlockStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Confirmed => "confirmed",
            Self::Orphaned => "orphaned",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LedgerRail {
    Coinbase,
    Ark,
    Pending,
    Forfeit,
}

impl LedgerRail {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Coinbase => "coinbase",
            Self::Ark => "ark",
            Self::Pending => "pending",
            Self::Forfeit => "forfeit",
        }
    }

    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "coinbase" => Some(Self::Coinbase),
            "ark" => Some(Self::Ark),
            "pending" => Some(Self::Pending),
            "forfeit" => Some(Self::Forfeit),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerRow {
    pub sp_id: [u8; 32],
    pub rail: LedgerRail,
    pub paid_sats: u64,
    pub ark_due_sats: u64,
    pub forfeited_sats: u64,
    pub pending_delta: i64,
}

/// One settlement transaction: block identity plus every ledger row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settlement {
    pub block_hash: [u8; 32],
    pub height: u32,
    pub fingerprint: Option<[u8; 32]>,
    pub status: BlockStatus,
    pub ledger: Vec<LedgerRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArkPayoutRow {
    pub block_hash: [u8; 32],
    pub sp_id: [u8; 32],
    pub sats: u64,
    pub due_height: u32,
    pub status: ArkPayoutStatus,
    pub vtxo_id: Option<Vec<u8>>,
    pub expiry_height: Option<u32>,
}

/// One scheduled `ark_payout` row plus the destination frozen in its block's
/// distribution snapshot. This is everything `payout` needs to (re)build and
/// sign an `ArkSendOrder` after a restart (SPEC §11.3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArkPayoutEmission {
    pub block_hash: [u8; 32],
    pub block_height: u32,
    pub is_remainder: bool,
    pub sp_id: [u8; 32],
    pub sats: u64,
    pub due_height: u32,
    /// `dest_id` from the snapshot entry; empty when it was not derived.
    pub dest_id: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArkPayoutStatus {
    Scheduled,
    Sent,
    FailedToPending,
}

impl ArkPayoutStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Scheduled => "scheduled",
            Self::Sent => "sent",
            Self::FailedToPending => "failed_to_pending",
        }
    }

    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "scheduled" => Some(Self::Scheduled),
            "sent" => Some(Self::Sent),
            "failed_to_pending" => Some(Self::FailedToPending),
            _ => None,
        }
    }
}

/// Bootstrap snapshot returned to a front at start (WIRE-FORMATS §3.5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BootstrapSnapshot {
    pub economic: Option<spc_accounting::economic::View>,
    pub rev: u64,
    /// Confirmed blocks whose `LedgerApplied` is reflected in
    /// `pending_balance` (newest first, bounded). A front seeds its
    /// block-hash dedupe from this so a republished event is applied once.
    pub applied_blocks: Vec<[u8; 32]>,
    pub pending_balance: Vec<([u8; 32], u64)>,
    pub ark_delivery: Vec<ArkDeliveryRow>,
    /// Identity keys needed to rebuild per-block destinations after restart.
    pub ark_identities: Vec<SpIdentityRow>,
    /// Registered variant-A epoch mailboxes used by `epoch_registered` mode.
    pub ark_epoch_mailboxes: Vec<ArkEpochMailboxRow>,
    pub window_tail: Vec<SliceRow>,
    pub type32_blobs: Vec<(u64, Vec<u8>)>,
    pub slices_cursor: Option<String>,
    pub front_events_cursor: Option<String>,
}
