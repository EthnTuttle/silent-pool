//! Durable row types. The schema is `migrations/0001_initial.sql` (SPEC §5.1).

use serde::{Deserialize, Serialize};

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
    pub sp_id: [u8; 32],
    pub ark_id: [u8; 4],
    pub b_ark: Vec<u8>,
    pub mailbox_mode: MailboxMode,
    pub mailbox_m: Option<Vec<u8>>,
    pub mailbox_d: Option<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SliceRow {
    pub slice_id: u64,
    pub height: u32,
    pub prev_hash: [u8; 32],
    pub ref_job_id: u64,
    pub ref_fees: u64,
    pub n_shares: u32,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

#[derive(Clone, Debug, PartialEq, Eq)]
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
    pub rev: u64,
    pub pending_balance: Vec<([u8; 32], u64)>,
    pub ark_delivery: Vec<ArkDeliveryRow>,
    pub window_tail: Vec<SliceRow>,
    pub type32_blobs: Vec<(u64, Vec<u8>)>,
    pub slices_cursor: Option<String>,
    pub front_events_cursor: Option<String>,
}
