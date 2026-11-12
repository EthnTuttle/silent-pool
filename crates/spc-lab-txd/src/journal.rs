//! The durable reservation/provenance journal (traffic plan sections 6.5,
//! 6.7 and 9.8).
//!
//! This is the **single authoritative funding ledger** for the traffic
//! driver: every satoshi the traffic wallets may spend enters through a
//! `FundingObserved` record with explicit source provenance, and every
//! spend passes `CommandBegin -> Reserved -> SignedTx` before any signed
//! byte can exist outside memory. There is no second ledger (no SQL
//! shadow, no in-memory credit): reopen replays the same file and rebuilds
//! the same state, crash-atomically.
//!
//! Journal shape (mirrors the engine `CommandLog` pattern: append-only
//! JSONL, mode 0600, per-record write + flush + `sync_data`, torn final
//! line tolerated on replay and truncated, strictly monotonic `seq`):
//!
//! - **Version 3 header (history contract)**: the first record (seq 0)
//!   carries the format magic, `history_version = 3`, the journal
//!   protocol string, the network, the declared key mode and derivation
//!   version, the run id, the driver id, the run-seed fingerprint and the
//!   retry caps. Header fields retain the approved v2 shape; history 3
//!   introduces the atomic `ChainRollback` proof record. History 2 and
//!   older files are refused without mutation or migration. Key derivation
//!   remains version 2, independently of the journal format.
//! - **Dedup**: a command id replayed with the same payload digest returns
//!   the stored outcome; the same id with a different payload (including
//!   a different payer role or category - both are digest-covered inside
//!   `RequestRecord`) is journaled as a `CommandConflict` and fails.
//!   Replaying a durable command id can never issue a second payment,
//!   and a retry after transport uncertainty reuses the identical signed
//!   bytes.
//! - **Reservation contract (plan 6.7)**: an outpoint is durably reserved
//!   before signing; concurrent reservations of the same outpoint are
//!   exclusive; a reserved outpoint is only released by an explicit
//!   terminal transition. `unknown` never releases and never reselects,
//!   and a payment that was **ever** unknown, ever accepted by an RPC
//!   node, or ever reorg-resubmitted never releases on a later rejection
//!   - its inputs degrade to possibly-spent instead (H1).
//! - **Lifecycle (plan 6.5)**: every transaction walks the journaled state
//!   machine (`Signed -> BroadcastSent -> AcceptedByRpc -> ...`), with
//!   `unknown` a first-class state under a **retry-send** cap, `conflicted`
//!   terminal, reorg displacement preserving possibly-spent claims with
//!   bounded same-bytes resubmission, and a derived `Invalidated` cascade
//!   for reorg-displaced funding (H2).
//! - **Retry caps**: `JournalCaps` (persisted in the header, revalidated
//!   on reopen) bounds retry **sends** from `unknown`: a cap of N admits
//!   at most N retry sends, i.e. at most N+1 total sends of the same
//!   bytes (the original plus N). Recording unknown **outcomes** is never
//!   capped (an honest reporter can always record what it saw). A
//!   `Reorged` transaction may only re-enter the network through an
//!   explicit `ReorgResubmit` record (H3).
//! - **Pause barrier (plan 6.4)**: broadcasts carry a monotonic send
//!   sequence; a `SendBarrier` records it, and no broadcast may be
//!   journaled after a barrier until an explicit `SendResumed`.
//! - **Integrity**: replay recomputes every txid from the stored raw bytes,
//!   remeasures weight/vbytes, recomputes the fee from the funding ledger,
//!   re-checks every transition, and requires the **decoded** inputs of a
//!   signed transaction to be exactly the reserved set (M6). A mismatch
//!   is a hard error, never a silent repair.
//! - **Ownership (kernel flock)**: the journal file is exclusively locked
//!   (non-blocking `flock`) for the handle's whole lifetime, acquired
//!   **before** replay or truncation in both create and reopen: a second
//!   live owner fails with `HeldByAnotherOwner` and never mutates (or
//!   truncates) a live writer's tail (H4).
//! - **Funding provenance (H5)**: funding enters as a `FundingObservation`
//!   with the owner's derivation `(chain, index)` - the script is derived
//!   by the wallet at that index, never supplied by the caller - and a
//!   source/context pairing. Canonical observations carry their block
//!   context; coinbase funding is reservable only at 100 confirmations of
//!   the chain tip observed through `ChainObserved`; own-transaction
//!   funding requires the journaled creating transaction to be confirmed
//!   with a byte-exact decoded vout match.
//!
//! Money conservation across the cascade is fail-closed: reorg
//! displacement can only make claims **less** available (invalidated /
//! possibly-spent); re-credit happens exclusively through re-confirmation
//! or canonical re-observation, never through inference.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Mutex;

use bitcoin::blockdata::transaction::Transaction;
use bitcoin::consensus::Decodable;
use bitcoin::hashes::sha256::Hash as Sha256;
use bitcoin::hashes::Hash as _;
use bitcoin::{BlockHash, OutPoint, ScriptBuf, Txid};
use serde::{Deserialize, Serialize};

use crate::builder::{PaymentCategory, SignedTrafficTx};
use crate::error::JournalError;
use crate::keys::{validate_run_id, Chain, WalletRole, DERIVATION_VERSION, KEY_MODE};
use crate::money::{checked_sub_sats, checked_sum_sats};

/// The journal history-contract version this core writes and accepts.
///
/// Version 3 activates proof-bearing rollback atomically with the T0
/// header/hello/scenario contract. History 2 is refused, never migrated.
pub const SUPPORTED_HISTORY_VERSION: u32 = 3;

/// The file-format magic in the header: identifies the file as this
/// core's traffic journal (and rejects pre-rework candidate files, which
/// have no magic field at all).
pub const FORMAT_MAGIC: &str = "SPC-LAB-TRAFFIC-JOURNAL";

/// The journal protocol string recorded in the history-3 header. Distinct from
/// the T0 control protocol (`spc-lab-tx-control/v1`): this identifies the
/// on-disk record contract, not the controller channel.
pub const PROTOCOL: &str = "spc-lab-tx-journal/3";

/// Maximum headers per branch, including the common ancestor. A deeper
/// rollback stops observation closed; it is never partially applied.
pub const MAX_ROLLBACK_HEADERS: usize = 4096;

/// Bound the encoded proof before serde's internally-tagged enum buffers
/// its contents. Covers two header branches, displaced hashes and anchors.
pub const MAX_ROLLBACK_RECORD_BYTES: usize = 2 * 1024 * 1024;

/// A hash/height anchor supplied by the canonical node observer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChainAnchor {
    pub block_hash: BlockHash,
    pub height: u64,
}

/// Source evidence, not a chain-selection rule. Both branches are ordered
/// tip-to-ancestor INCLUSIVE. The observer must verify canonicality and the
/// run's consensus profile; the core verifies hashes, links, heights, PoW
/// and the regtest target ceiling, never inferring a longest chain.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RollbackEvidence {
    pub version: u32,
    pub old_anchor: ChainAnchor,
    pub new_anchor: ChainAnchor,
    pub common_ancestor: ChainAnchor,
    /// Exactly 80-byte consensus headers encoded as lowercase hex.
    #[serde(deserialize_with = "bounded_rollback_vec")]
    pub old_headers: Vec<String>,
    #[serde(deserialize_with = "bounded_rollback_vec")]
    pub new_headers: Vec<String>,
    /// Old tip to (excluding) common ancestor, exactly, without duplicates.
    #[serde(deserialize_with = "bounded_rollback_vec")]
    pub displaced_block_hashes: Vec<BlockHash>,
}

// Do not trust a serde size_hint or allocate an unbounded proof on replay.
fn bounded_rollback_vec<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Bounded<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Bounded<T> {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "at most {MAX_ROLLBACK_HEADERS} rollback elements")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> Result<Self::Value, A::Error> {
            let mut result = Vec::new();
            while let Some(value) = seq.next_element()? {
                if result.len() == MAX_ROLLBACK_HEADERS {
                    return Err(serde::de::Error::custom("rollback proof exceeds bound"));
                }
                result.push(value);
            }
            Ok(result)
        }
    }
    deserializer.deserialize_seq(Bounded(std::marker::PhantomData))
}

/// Confirmations required before coinbase funding is reservable (H5).
pub const COINBASE_MATURITY: u64 = 100;

/// Default retry-send cap from `unknown` (see [`JournalCaps`]).
pub const DEFAULT_UNKNOWN_RETRY_SENDS: u32 = 3;

/// Default cap on same-bytes reorg resubmissions (see [`JournalCaps`]).
pub const DEFAULT_REORG_RESUBMITS: u32 = 3;

/// The private subdirectory (under the run's private directory) that owns
/// the journal file.
pub const JOURNAL_SUBDIR: &str = "traffic";

/// The journal file name.
pub const JOURNAL_FILE_NAME: &str = "history.jsonl";

/// The retry caps of one journal, persisted in the header and
/// revalidated on reopen (H3).
///
/// **Semantics (frozen):** `unknown_retry_sends = N` admits at most N
/// retry **sends** from the `unknown` state - i.e. at most N+1 total sends
/// of the same bytes (the original send plus N retries). Recording
/// unknown **outcomes** is not capped: the counter governs admission of a
/// new send (`retry_sends < N`), not the journaling of what was observed.
/// Cap 0 means the original send can never be retried (its outcome may
/// still be recorded any number of times).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalCaps {
    /// Maximum retry sends from `unknown` (0..=3).
    pub unknown_retry_sends: u32,
    /// Maximum same-bytes reorg resubmissions (1..=3).
    pub reorg_resubmits: u32,
}

impl Default for JournalCaps {
    fn default() -> Self {
        Self {
            unknown_retry_sends: DEFAULT_UNKNOWN_RETRY_SENDS,
            reorg_resubmits: DEFAULT_REORG_RESUBMITS,
        }
    }
}

impl JournalCaps {
    /// Validate the cap ranges (fail closed on an unusable policy).
    pub fn validate(&self) -> Result<(), JournalError> {
        if self.unknown_retry_sends > 3 {
            return Err(JournalError::Integrity(format!(
                "invalid journal caps: unknown_retry_sends {} is outside 0..=3",
                self.unknown_retry_sends
            )));
        }
        if self.reorg_resubmits == 0 || self.reorg_resubmits > 3 {
            return Err(JournalError::Integrity(format!(
                "invalid journal caps: reorg_resubmits {} is outside 1..=3",
                self.reorg_resubmits
            )));
        }
        Ok(())
    }
}

/// A one-shot write-path fault for the fault-injection tests (M4). Never
/// set outside `_for_tests_set_write_fault`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteFault {
    /// Write only part of the record, then attempt a rollback.
    PartialWrite,
    /// Write the full record, then fail the durability sync and roll back.
    SyncFailure,
    /// Fail the rollback itself: the handle is poisoned.
    RollbackFailure,
}

/// Where a funding observation came from. Synthetic fixtures are
/// conformance-test inputs only; the production journal constructor
/// refuses them (fail closed).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FundingSource {
    /// Coinbase paid to a traffic address by the bootstrap mining phase.
    BootstrapCoinbase,
    /// An output observed in a canonical block.
    CanonicalBlock,
    /// An output of a traffic wallet's own confirmed transaction.
    OwnTransaction,
    /// Synthetic conformance fixture; never admissible in production.
    SyntheticFixture,
}

impl FundingSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::BootstrapCoinbase => "bootstrap_coinbase",
            Self::CanonicalBlock => "canonical_block",
            Self::OwnTransaction => "own_transaction",
            Self::SyntheticFixture => "synthetic_fixture",
        }
    }
}

/// The provenance evidence carried by a funding observation (H5). The
/// pairing with [`FundingSource`] is enforced: bootstrap coinbase and
/// canonical-block observations must carry their block context;
/// own-transaction observations carry none (the creating transaction is
/// journaled); fixtures carry none.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FundingContext {
    /// The canonical block that carries the observed output.
    CanonicalBlock {
        block_hash: String,
        height: u64,
        coinbase: bool,
    },
    /// An output of a journaled, confirmed, own transaction.
    OwnTransaction,
    /// Synthetic conformance fixture (no chain evidence).
    Fixture,
}

/// One funding observation as handed to the journal (H5). The funding
/// **script is not a field**: the wallet set derives it at
/// `(owner_role, chain, index)` and passes the derived script to the
/// journal, so a foreign script can never be claimed through this API
/// (and the spending path re-derives and re-compares it, M2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FundingObservation {
    pub outpoint: OutPoint,
    pub amount_sats: u64,
    /// The owner's chain (receive/change) the funding was paid to.
    pub chain: Chain,
    /// The owner's derivation index the funding was paid to.
    pub index: u32,
    pub owner_role: WalletRole,
    pub source: FundingSource,
    pub context: FundingContext,
}

/// One payment destination as journaled (public data only, no secret).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DestinationRecord {
    pub script_hex: String,
    pub amount_sats: u64,
    /// The owning traffic wallet when the destination is one of ours.
    #[serde(default)]
    pub owner_role: Option<WalletRole>,
}

/// The canonical, digest-covered form of a payment request. The payload
/// digest is `SHA-256` over the serde-JSON encoding of this value (see
/// [`request_record_digest`]; the journal computes it itself), so the same
/// command id plus the same digest implies byte-identical intent -
/// including the same payer role and the same category (M1: both live in
/// the digest-covered record, so a same-id/different-payer or
/// same-id/different-category retry is a journaled conflict).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub construction_effect: Option<crate::construction::ConstructionEffect>,
    pub payer_role: WalletRole,
    pub category: PaymentCategory,
    pub destinations: Vec<DestinationRecord>,
    pub fee_rate_sats_vb: u64,
}

/// The payload digest of a request record: SHA-256 (hex) over the
/// canonical serde-JSON encoding. Computed by the journal, never a
/// caller-supplied value (M1).
pub fn request_record_digest(record: &RequestRecord) -> String {
    let encoded = serde_json::to_vec(record).expect("request record serializes");
    hex::encode(Sha256::hash(&encoded).to_byte_array())
}

/// The change output of a signed transaction (always an owned script).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeRecord {
    pub script_hex: String,
    pub amount_sats: u64,
    pub index: u32,
}

/// One journal line. `seq` is strictly monotonic from 0; the header is
/// always the first record (seq 0).
///
/// History 3 rejects unknown record fields, including proof fields.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "record", rename_all = "snake_case", deny_unknown_fields)]
#[allow(clippy::large_enum_variant)]
pub enum JournalRecord {
    /// The history-3 header: always the first record.
    Header {
        seq: u64,
        format_magic: String,
        history_version: u32,
        protocol: String,
        network: String,
        key_mode: String,
        derivation_version: u32,
        run_id: String,
        driver_id: String,
        run_seed_fingerprint: String,
        /// The canonical digest of the complete wallet-set allocation
        /// this journal belongs to (R6: SHA-256 over the sorted-key JSON
        /// of the whole allocation, computed by
        /// `ProvisionConfig::validate_allocation`, never caller-supplied
        /// as a raw string through the wallet API). Verified on reopen.
        allocation_digest: String,
        caps: JournalCaps,
        created_unix_ms: u64,
    },
    /// A writer session boundary (R2): appended by `reopen` under the
    /// exclusive lock, after replay and every binding check. Committing
    /// it moves every transaction still in `BroadcastSent` (a send whose
    /// outcome was never journaled before the process went away) to
    /// `Unknown` with the attempt counter advanced and
    /// `ever_unknown = true` - **without** incrementing `retry_sends`
    /// (a mere process restart is not a retry send; the next send from
    /// `unknown` is one, and counts against the cap). Replay commits it
    /// identically, and a later session never moves an already-`Unknown`
    /// transaction again, so crash-looping reopens cannot inflate the
    /// attempt counters.
    SessionStarted {
        seq: u64,
        restarted_unix_ms: u64,
    },
    FundingObserved {
        seq: u64,
        outpoint: String,
        amount_sats: u64,
        script_hex: String,
        chain: Chain,
        index: u32,
        owner_role: WalletRole,
        source: FundingSource,
        context: FundingContext,
        observed_unix_ms: u64,
    },
    CommandBegin {
        seq: u64,
        command_id: String,
        /// The digest-covered request (payer role and category included,
        /// M1). The payload digest is recomputed by the journal on replay.
        request: RequestRecord,
    },
    CommandConflict {
        seq: u64,
        command_id: String,
        existing_digest: String,
        incoming_digest: String,
    },
    Reserved {
        seq: u64,
        command_id: String,
        outpoints: Vec<String>,
        change_index: u32,
    },
    SignedTx {
        seq: u64,
        command_id: String,
        txid: String,
        raw_hex: String,
        vbytes: u64,
        weight_wu: u64,
        fee_sats: u64,
        fee_rate_sats_vb: u64,
        category: String,
        payer_role: WalletRole,
        spent_outpoints: Vec<String>,
        destinations: Vec<DestinationRecord>,
        #[serde(default)]
        change: Option<ChangeRecord>,
        /// Size class label from the scenario profile (T1b maps the T0
        /// `TrafficSpec`; `None` until then).
        #[serde(default)]
        size_class: Option<String>,
    },
    CommandFinished {
        seq: u64,
        command_id: String,
        outcome: String,
        #[serde(default)]
        txid: Option<String>,
    },
    InputsReleased {
        seq: u64,
        command_id: String,
        reason: String,
    },
    BroadcastSent {
        seq: u64,
        command_id: String,
        txid: String,
        send_sequence: u64,
    },
    AcceptedByRpc {
        seq: u64,
        command_id: String,
        txid: String,
        node_id: String,
    },
    ObservedInMempool {
        seq: u64,
        command_id: String,
        txid: String,
        node_id: String,
    },
    Confirmed {
        seq: u64,
        command_id: String,
        txid: String,
        block_hash: String,
        height: u64,
    },
    Rejected {
        seq: u64,
        command_id: String,
        txid: String,
        code: String,
    },
    UnknownOutcome {
        seq: u64,
        command_id: String,
        txid: String,
        reason: String,
        attempt: u32,
    },
    Conflicted {
        seq: u64,
        command_id: String,
        txid: String,
        reason: String,
    },
    Reorged {
        seq: u64,
        command_id: String,
        txid: String,
        from_block_hash: String,
        height: u64,
    },
    ReorgResubmit {
        seq: u64,
        command_id: String,
        txid: String,
        /// The eligibility proof journaled with the resubmission: the
        /// original inputs observed unspent from canonical blocks.
        proven_unspent: Vec<String>,
        send_sequence: u64,
    },
    /// A canonical chain observation (H5): the current tip, and the
    /// blocks displaced by the reorg (empty for a pure tip advance).
    /// Drives coinbase maturity (tip height) and the funding/transaction
    /// invalidation cascade (displaced blocks).
    ChainObserved {
        seq: u64,
        tip_block_hash: String,
        tip_height: u64,
        #[serde(default)]
        displaced_block_hashes: Vec<String>,
        observed_unix_ms: u64,
    },
    /// Proof and lower-tip application are one durable history-3 record.
    ChainRollback {
        seq: u64,
        evidence: RollbackEvidence,
        observed_unix_ms: u64,
    },
    SendBarrier {
        seq: u64,
        command_id: String,
        wall_unix_ms: u64,
        monotonic_ms: u64,
        #[serde(default)]
        last_txid: Option<String>,
        send_sequence: u64,
    },
    SendResumed {
        seq: u64,
        command_id: String,
    },
}

impl JournalRecord {
    fn seq(&self) -> u64 {
        match self {
            Self::Header { seq, .. }
            | Self::SessionStarted { seq, .. }
            | Self::FundingObserved { seq, .. }
            | Self::CommandBegin { seq, .. }
            | Self::CommandConflict { seq, .. }
            | Self::Reserved { seq, .. }
            | Self::SignedTx { seq, .. }
            | Self::CommandFinished { seq, .. }
            | Self::InputsReleased { seq, .. }
            | Self::BroadcastSent { seq, .. }
            | Self::AcceptedByRpc { seq, .. }
            | Self::ObservedInMempool { seq, .. }
            | Self::Confirmed { seq, .. }
            | Self::Rejected { seq, .. }
            | Self::UnknownOutcome { seq, .. }
            | Self::Conflicted { seq, .. }
            | Self::Reorged { seq, .. }
            | Self::ReorgResubmit { seq, .. }
            | Self::ChainObserved { seq, .. }
            | Self::ChainRollback { seq, .. }
            | Self::SendBarrier { seq, .. }
            | Self::SendResumed { seq, .. } => *seq,
        }
    }
}

/// The lifecycle state of one journaled transaction (plan 6.5).
///
/// `Invalidated` is **derived, never recorded**: it is the cascade state
/// of a transaction that spends reorg-invalidated funding (H2). No
/// explicit record may transition into or out of it; the cascade restores
/// the remembered prior state (a `Confirmed` prior maps to `Reorged`, the
/// conservative resubmit path) once **all** of the transaction's inputs
/// are non-invalidated again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TxState {
    Signed,
    BroadcastSent,
    AcceptedByRpc,
    ObservedInMempool,
    Confirmed,
    Rejected,
    Unknown,
    Conflicted,
    Reorged,
    Invalidated,
}

impl TxState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Signed => "created_signed",
            Self::BroadcastSent => "broadcast_sent",
            Self::AcceptedByRpc => "accepted_by_rpc",
            Self::ObservedInMempool => "observed_in_mempool",
            Self::Confirmed => "confirmed",
            Self::Rejected => "rejected",
            Self::Unknown => "unknown",
            Self::Conflicted => "conflicted",
            Self::Reorged => "reorged",
            Self::Invalidated => "invalidated",
        }
    }

    /// Terminal states never take another transition (except the reorg
    /// reconcile of a confirmed tx, the canonical re-inclusion of a
    /// reorged tx, and the bounded same-bytes retry of `unknown`, which
    /// [`transition_legal`] encodes separately).
    /// `Invalidated` is not terminal (the cascade can restore it).
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Confirmed | Self::Rejected | Self::Conflicted)
    }

    /// The plan 6.5 transition table for **explicitly recorded**
    /// transitions. A normal broadcast may only leave the initial
    /// `Signed` state or an eligible `Unknown` (retry-send admission is
    /// checked separately against the journal caps, H3); a `Reorged`
    /// transaction re-enters the network **only** through
    /// `ReorgResubmit` (there is deliberately no
    /// `(Reorged, BroadcastSent)` edge) - but a canonical re-inclusion
    /// (the new chain re-mines the displaced transaction from the
    /// mempool without any wallet action) IS recordable as
    /// `(Reorged, Confirmed)`: a confirmation is block evidence, not a
    /// send (R12; the displaced-block refusal is validated separately).
    /// `Invalidated` has no edges at all: explicit records never touch a
    /// cascade-invalidated transaction.
    fn transition_legal(from: Self, to: Self) -> bool {
        use TxState::*;
        matches!(
            (from, to),
            (Signed, BroadcastSent)
                | (BroadcastSent, AcceptedByRpc)
                | (BroadcastSent, Rejected)
                | (BroadcastSent, Unknown)
                | (AcceptedByRpc, ObservedInMempool)
                | (AcceptedByRpc, Confirmed)
                | (AcceptedByRpc, Rejected)
                | (AcceptedByRpc, Unknown)
                | (ObservedInMempool, Confirmed)
                | (ObservedInMempool, Unknown)
                | (ObservedInMempool, ObservedInMempool)
                | (ObservedInMempool, Conflicted)
                | (Unknown, BroadcastSent)
                | (Unknown, AcceptedByRpc)
                | (Unknown, Confirmed)
                | (Unknown, Unknown)
                | (Unknown, Rejected)
                | (Unknown, Conflicted)
                | (Confirmed, Reorged)
                | (Confirmed, Confirmed)
                | (Reorged, Confirmed)
                | (Reorged, Conflicted)
        )
    }
}

/// The reservation status of one funded outpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutpointStatus {
    Available,
    Reserved {
        command_id: String,
    },
    /// A confirmed spend later displaced by a reorg: the coin may still
    /// have been spent (possibly-spent claims are preserved, plan 6.5).
    PossiblySpent,
    Spent,
    /// The funding itself was invalidated by a reorg (its canonical block
    /// was displaced, or its creating own transaction is no longer
    /// confirmed). `prior` is the status to restore to if the funding is
    /// revalidated (a `Spent` prior restores conservatively to
    /// `PossiblySpent`). Derived by the cascade / chain observation -
    /// never recorded directly (H2).
    Invalidated {
        prior: Box<OutpointStatus>,
    },
}

impl OutpointStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Reserved { .. } => "reserved",
            Self::PossiblySpent => "possibly_spent",
            Self::Spent => "spent",
            Self::Invalidated { .. } => "invalidated",
        }
    }
}

/// The phase of one durable command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandPhase {
    /// Begun, nothing reserved yet (a crash here may select fresh).
    Began,
    /// Inputs durably reserved (a crash here must reuse them).
    Reserved,
    /// Signed bytes durably recorded (a retry returns those bytes).
    Signed,
    /// Finished with a terminal outcome.
    Finished,
}

impl CommandPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Began => "began",
            Self::Reserved => "reserved",
            Self::Signed => "signed",
            Self::Finished => "finished",
        }
    }
}

/// The typed header facts as replayed from the header record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeaderFacts {
    pub format_magic: String,
    pub history_version: u32,
    pub protocol: String,
    pub network: String,
    pub key_mode: String,
    pub derivation_version: u32,
    pub run_id: String,
    pub driver_id: String,
    pub run_seed_fingerprint: String,
    /// The canonical whole-allocation digest (see the `Header` record).
    pub allocation_digest: String,
    pub caps: JournalCaps,
    pub created_unix_ms: u64,
}

/// An owned, read-only observation of committed state. It is not an
/// allocation token or a scan-completion cursor. Reuse after an append
/// deliberately sees the old sequence/data; call observer_snapshot again
/// for fresh facts. Editing this detached copy cannot modify the journal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObserverSnapshot {
    pub committed_seq: u64,
    pub tip: Option<ChainAnchor>,
    pub outpoints: BTreeMap<OutPoint, ObserverOutpoint>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObserverOutpoint {
    pub amount_sats: u64,
    pub script: ScriptBuf,
    pub owner: WalletRole,
    pub chain: Chain,
    pub index: u32,
    pub source: FundingSource,
    pub context: FundingContext,
    pub status: OutpointStatus,
    /// Independent of reservation/status: a retained coinbase may become
    /// immature without invalidating its provenance or releasing its claim.
    pub coinbase_immature: bool,
}

/// Opaque identity of an already journaled send, for local queue revalidation.
/// No deserializer or public constructor; not a permit to bypass admission.
#[derive(Clone, Debug)]
pub struct PendingSendIntent {
    header: HeaderFacts,
    txid: Txid,
    send_sequence: u64,
    intent_seq: u64,
}

/// An in-memory funded outpoint.
#[derive(Clone, Debug)]
struct OutpointEntry {
    amount_sats: u64,
    script: ScriptBuf,
    chain: Chain,
    index: u32,
    owner: WalletRole,
    source: FundingSource,
    /// Parsed canonical block context (present for canonical sources).
    context: Option<(BlockHash, u64, bool)>,
    status: OutpointStatus,
}

/// An in-memory command.
#[derive(Clone, Debug)]
struct CommandEntry {
    payload_digest: String,
    category: PaymentCategory,
    wallet_role: WalletRole,
    request: RequestRecord,
    phase: CommandPhase,
    txid: Option<Txid>,
    reserved_outpoints: Vec<OutPoint>,
    change_index: Option<u32>,
    outcome: Option<String>,
}

/// An in-memory transaction.
#[derive(Clone, Debug)]
struct TxEntry {
    command_id: String,
    state: TxState,
    raw_hex: String,
    vbytes: u64,
    weight_wu: u64,
    fee_sats: u64,
    fee_rate_sats_vb: u64,
    category: PaymentCategory,
    spent_outpoints: Vec<OutPoint>,
    change_index: Option<u32>,
    /// Unknown outcomes recorded (never capped; evidence only).
    unknown_retries: u32,
    /// Retry sends committed from `unknown` (admitted against the caps).
    retry_sends: u32,
    reorg_resubmits: u32,
    /// Durable history (H1): set once any unknown outcome or RPC
    /// acceptance was ever journaled; a later `rejected` then degrades
    /// the inputs to possibly-spent instead of releasing them.
    ever_unknown: bool,
    ever_accepted: bool,
    /// The remembered prior state while cascade-invalidated (H2).
    invalidated_prior: Option<TxState>,
    confirmed_block: Option<(BlockHash, u64)>,
    /// The block a `Reorged` transaction was displaced from (R12/S3):
    /// a canonical re-inclusion may not name the displaced block on
    /// STALE evidence, so the displaced identity survives the `Reorged`
    /// transition (which clears `confirmed_block`).
    reorged_from_block: Option<(BlockHash, u64)>,
    /// The journal seq of the record that displaced this transaction
    /// (S3): the explicit `Reorged` record, or the `ChainObserved`
    /// record that displaced it directly or cascaded it to
    /// `Invalidated`. The displaced-block refusal is stale-evidence
    /// only: a same-block re-inclusion is refused while no
    /// `ChainObserved` newer than this seq has been committed (the
    /// displacement is still the latest chain evidence); any newer
    /// chain observation - e.g. the chain reorged back and the block
    /// is canonical again - makes the same-hash confirmation
    /// admissible. The ORIGINAL displacement seq is preserved while
    /// the transaction is cascade-`Invalidated` and through its
    /// promotion back to `Reorged` (never overwritten by the restoring
    /// parent's confirmation), and cleared when the transaction is
    /// confirmed. Derived deterministically from the record stream.
    displaced_at_seq: Option<u64>,
    last_send: Option<(u64, u64)>,
}

/// The shareable, identity-free aggregate of the ledger (counts and totals
/// only; scripts and roles stay in the private journal).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LedgerSummary {
    pub funding_observations: u64,
    pub available_outpoints: u64,
    pub reserved_outpoints: u64,
    pub possibly_spent_outpoints: u64,
    pub spent_outpoints: u64,
    pub invalidated_outpoints: u64,
    pub available_sats: u64,
    pub commands_begun: u64,
    pub commands_finished: u64,
    pub commands_in_flight: u64,
    pub transactions_signed: u64,
    pub transactions_confirmed: u64,
    pub transactions_rejected: u64,
    pub transactions_conflicted: u64,
    pub transactions_unknown: u64,
    pub transactions_reorged: u64,
    pub transactions_invalidated: u64,
    pub send_sequence: u64,
    pub barrier_active: bool,
}

/// The stored state of a command, as returned to retries.
#[derive(Clone, Debug)]
pub struct StoredCommand {
    pub command_id: String,
    pub payload_digest: String,
    pub category: PaymentCategory,
    pub wallet_role: WalletRole,
    pub request: RequestRecord,
    pub phase: CommandPhase,
    pub txid: Option<Txid>,
    pub raw_hex: Option<String>,
    pub reserved_outpoints: Vec<OutPoint>,
    pub change_index: Option<u32>,
    pub outcome: Option<String>,
}

/// Result of `begin_command`.
#[derive(Clone, Debug)]
pub enum CommandBeginOutcome {
    /// A brand-new command id: proceed to selection and reservation.
    New,
    /// A previously recorded command: its durable state (replay/resume
    /// paths consume this; a different payload digest is an error before
    /// this is returned). Boxed: the variant is far larger than `New` and
    /// the outcome is returned once per command, not in hot loops.
    Stored(Box<StoredCommand>),
}

/// The measured facts of one journaled transaction.
#[derive(Clone, Debug)]
pub struct SignedTxFacts {
    pub command_id: String,
    pub state: TxState,
    pub raw_hex: String,
    pub vbytes: u64,
    pub weight_wu: u64,
    pub fee_sats: u64,
    pub fee_rate_sats_vb: u64,
    pub category: PaymentCategory,
    pub spent_outpoints: Vec<OutPoint>,
    pub change_index: Option<u32>,
    pub unknown_retries: u32,
    pub retry_sends: u32,
    pub reorg_resubmits: u32,
    pub ever_unknown: bool,
    pub ever_accepted: bool,
    pub confirmed_block: Option<(BlockHash, u64)>,
}

/// Detached private actor recovery data. This is deliberately not serializable
/// or deserializable: HTTP/evidence must use their allowlisted projections.
/// All commands are included, including finished commands whose transactions
/// subsequently became reorged or uncertain. None of these copies is authority
/// to mutate the journal or construct an opaque PendingSendIntent.
#[derive(Clone, Debug)]
pub struct RecoverySnapshot {
    pub committed_seq: u64,
    pub tip: Option<ChainAnchor>,
    pub barrier_active: bool,
    pub next_send_sequence: u64,
    pub last_broadcast_txid: Option<Txid>,
    pub last_chain_observed_seq: Option<u64>,
    pub caps: JournalCaps,
    pub poisoned: bool,
    pub commands: BTreeMap<String, StoredCommand>,
    pub transactions: BTreeMap<Txid, RecoveryTransaction>,
}

#[derive(Clone, Debug)]
pub struct RecoveryTransaction {
    pub facts: SignedTxFacts,
    /// (send sequence, intent record sequence), as committed by the core.
    pub last_send: Option<(u64, u64)>,
    pub reorged_from_block: Option<(BlockHash, u64)>,
    pub displaced_at_seq: Option<u64>,
    pub invalidated_prior: Option<TxState>,
}

struct JournalState {
    file: File,
    /// The proven-durable file length (rollback point for M4).
    durable_len: u64,
    /// Set when a write fault could not be proven rolled back: every
    /// further mutation fails closed (M4).
    poisoned: bool,
    /// One-shot injected fault (tests only).
    write_fault: Option<WriteFault>,
    next_seq: u64,
    header_seen: bool,
    header_facts: Option<HeaderFacts>,
    funding: BTreeMap<OutPoint, OutpointEntry>,
    commands: BTreeMap<String, CommandEntry>,
    txs: BTreeMap<Txid, TxEntry>,
    next_send_sequence: u64,
    barrier_active: bool,
    last_broadcast_txid: Option<Txid>,
    /// Per-role next change index (deterministic selection input).
    change_index_next: BTreeMap<WalletRole, u32>,
    /// Per-role funding observation count (deterministic receive index).
    receive_index_next: BTreeMap<WalletRole, u32>,
    /// The canonical chain tip as last observed (H5 maturity).
    tip: Option<(u64, BlockHash)>,
    /// The journal seq of the last committed `ChainObserved` record
    /// (S3): the recency witness for the displaced-block refusal. None
    /// until the first chain observation. Derived deterministically
    /// from the record stream (replay reproduces it exactly).
    last_chain_observed_seq: Option<u64>,
    /// The retry caps from the header.
    caps: JournalCaps,
}

impl JournalState {
    fn tx_facts(&self, txid: &Txid) -> Option<SignedTxFacts> {
        let tx = self.txs.get(txid)?;
        Some(SignedTxFacts {
            command_id: tx.command_id.clone(),
            state: tx.state,
            raw_hex: tx.raw_hex.clone(),
            vbytes: tx.vbytes,
            weight_wu: tx.weight_wu,
            fee_sats: tx.fee_sats,
            fee_rate_sats_vb: tx.fee_rate_sats_vb,
            category: tx.category.clone(),
            spent_outpoints: tx.spent_outpoints.clone(),
            change_index: tx.change_index,
            unknown_retries: tx.unknown_retries,
            retry_sends: tx.retry_sends,
            reorg_resubmits: tx.reorg_resubmits,
            ever_unknown: tx.ever_unknown,
            ever_accepted: tx.ever_accepted,
            confirmed_block: tx.confirmed_block,
        })
    }
    fn new(file: File, durable_len: u64, caps: JournalCaps) -> Self {
        Self {
            file,
            durable_len,
            poisoned: false,
            write_fault: None,
            next_seq: 0,
            header_seen: false,
            header_facts: None,
            funding: BTreeMap::new(),
            commands: BTreeMap::new(),
            txs: BTreeMap::new(),
            next_send_sequence: 1,
            barrier_active: false,
            last_broadcast_txid: None,
            change_index_next: BTreeMap::new(),
            receive_index_next: BTreeMap::new(),
            tip: None,
            last_chain_observed_seq: None,
            caps,
        }
    }

    fn command(&self, command_id: &str) -> Result<&CommandEntry, JournalError> {
        self.commands
            .get(command_id)
            .ok_or_else(|| JournalError::UnknownCommand(command_id.to_string()))
    }

    fn tx_of_command(&self, command_id: &str) -> Result<(&TxEntry, &CommandEntry), JournalError> {
        let command = self.command(command_id)?;
        let txid = command
            .txid
            .ok_or_else(|| JournalError::UnknownTx(command_id.to_string()))?;
        let tx = self
            .txs
            .get(&txid)
            .ok_or_else(|| JournalError::UnknownTx(txid.to_string()))?;
        Ok((tx, command))
    }

    fn stored_view(&self, command_id: &str) -> Option<StoredCommand> {
        let entry = self.commands.get(command_id)?;
        let raw_hex = entry
            .txid
            .and_then(|txid| self.txs.get(&txid))
            .map(|tx| tx.raw_hex.clone());
        Some(StoredCommand {
            command_id: command_id.to_string(),
            payload_digest: entry.payload_digest.clone(),
            category: entry.category.clone(),
            wallet_role: entry.wallet_role.clone(),
            request: entry.request.clone(),
            phase: entry.phase,
            txid: entry.txid,
            raw_hex,
            reserved_outpoints: entry.reserved_outpoints.clone(),
            change_index: entry.change_index,
            outcome: entry.outcome.clone(),
        })
    }
}

/// The append-only, crash-atomic traffic journal (see the module docs).
pub struct TrafficJournal {
    path: PathBuf,
    allow_fixture_funding: bool,
    state: Mutex<JournalState>,
}

impl std::fmt::Debug for TrafficJournal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrafficJournal")
            .field("path", &self.path)
            .field("allow_fixture_funding", &self.allow_fixture_funding)
            .finish_non_exhaustive()
    }
}

impl TrafficJournal {
    /// The journal path under a run directory:
    /// `<run_dir>/private/traffic/history.jsonl`.
    pub fn path_in(run_dir: &Path) -> PathBuf {
        run_dir
            .join("private")
            .join(JOURNAL_SUBDIR)
            .join(JOURNAL_FILE_NAME)
    }

    /// Create a fresh journal (the file must not exist) with its history-3
    /// header. Fixture funding is refused in this production
    /// constructor. `allocation_digest` is the canonical digest of the
    /// complete wallet-set allocation (computed by
    /// `ProvisionConfig::validate_allocation`; the wallet API never
    /// accepts a caller-supplied digest string).
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        run_dir: &Path,
        network: &str,
        run_seed_fingerprint: &str,
        run_id: &str,
        driver_id: &str,
        allocation_digest: &str,
        caps: &JournalCaps,
    ) -> Result<Self, JournalError> {
        Self::create_inner(
            run_dir,
            network,
            run_seed_fingerprint,
            run_id,
            driver_id,
            allocation_digest,
            caps,
            false,
        )
    }

    /// Create a journal that admits `SyntheticFixture` funding. This is a
    /// conformance-test constructor: the production constructor refuses
    /// synthetic UTXOs, so a fixture can never fund a production run.
    #[allow(clippy::too_many_arguments)]
    pub fn create_for_conformance_tests(
        run_dir: &Path,
        network: &str,
        run_seed_fingerprint: &str,
        run_id: &str,
        driver_id: &str,
        allocation_digest: &str,
        caps: &JournalCaps,
    ) -> Result<Self, JournalError> {
        Self::create_inner(
            run_dir,
            network,
            run_seed_fingerprint,
            run_id,
            driver_id,
            allocation_digest,
            caps,
            true,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn create_inner(
        run_dir: &Path,
        network: &str,
        run_seed_fingerprint: &str,
        run_id: &str,
        driver_id: &str,
        allocation_digest: &str,
        caps: &JournalCaps,
        allow_fixture_funding: bool,
    ) -> Result<Self, JournalError> {
        caps.validate()?;
        validate_run_id(run_id)
            .map_err(|e| JournalError::Integrity(format!("journal run id: {e}")))?;
        validate_run_id(driver_id)
            .map_err(|e| JournalError::Integrity(format!("journal driver id: {e}")))?;
        let path = Self::path_in(run_dir);
        if path.exists() {
            return Err(JournalError::Io(format!(
                "journal {} already exists; journals are created exactly once",
                path.display()
            )));
        }
        let parent = path
            .parent()
            .expect("journal path has a parent")
            .to_path_buf();
        std::fs::create_dir_all(&parent)
            .map_err(|e| JournalError::Io(format!("create journal dir: {e}")))?;
        let file = OpenOptions::new()
            .create_new(true)
            .append(true)
            .read(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| JournalError::Io(format!("create {}: {e}", path.display())))?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| JournalError::Io(format!("chmod {}: {e}", path.display())))?;
        // H4: the exclusive lifetime lock is acquired BEFORE any write.
        lock_or_fail(&file, &path)?;
        // M4: the directory entry itself must be durable (parent fsync).
        File::open(&parent)
            .and_then(|dir| dir.sync_all())
            .map_err(|e| JournalError::Io(format!("fsync {}: {e}", parent.display())))?;
        let file_len = file
            .metadata()
            .map_err(|e| JournalError::Io(format!("stat {}: {e}", path.display())))?
            .len();
        let mut state = JournalState::new(file, file_len, *caps);
        let record = JournalRecord::Header {
            seq: 0,
            format_magic: FORMAT_MAGIC.to_string(),
            history_version: SUPPORTED_HISTORY_VERSION,
            protocol: PROTOCOL.to_string(),
            network: network.to_string(),
            key_mode: KEY_MODE.to_string(),
            derivation_version: DERIVATION_VERSION,
            run_id: run_id.to_string(),
            driver_id: driver_id.to_string(),
            run_seed_fingerprint: run_seed_fingerprint.to_string(),
            allocation_digest: allocation_digest.to_string(),
            caps: *caps,
            created_unix_ms: crate::now_unix_ms(),
        };
        validate_record(&state, &record, allow_fixture_funding)?;
        write_line(&mut state, &record)?;
        commit_record(&mut state, record);
        Ok(Self {
            path,
            allow_fixture_funding,
            state: Mutex::new(state),
        })
    }

    /// Reopen an existing journal, replaying and re-verifying every line.
    /// The final line may be torn by a crash; it is truncated (a torn
    /// record cannot have authorized an effect) - but only after the
    /// exclusive lock is acquired, so a **live** writer's tail is never
    /// touched (a second live owner fails with `HeldByAnotherOwner`).
    /// Any other inconsistency is a hard integrity error - never a silent
    /// repair. The header caps must equal `expected_caps`, the header
    /// allocation digest must equal `expected_allocation_digest` and the
    /// header driver id must equal `expected_driver_id` - every check
    /// runs BEFORE the session record below, so a mismatching reopen
    /// never mutates the file.
    ///
    /// **Reopen is a writer operation** (R2); use `observer_snapshot` for
    /// an existing owner's read-only view. After replay and every
    /// binding check, a durable `SessionStarted` record is appended;
    /// committing it moves every still-in-flight `BroadcastSent`
    /// transaction to `Unknown` (`ever_unknown = true`, the attempt
    /// counter advanced, `retry_sends` untouched). If that append fails,
    /// the reopen fails (or the handle is poisoned) - a silently
    /// uncertain handle is never returned.
    pub fn reopen(
        run_dir: &Path,
        expected_caps: &JournalCaps,
        expected_allocation_digest: &str,
        expected_driver_id: &str,
    ) -> Result<Self, JournalError> {
        Self::reopen_inner(
            run_dir,
            expected_caps,
            expected_allocation_digest,
            expected_driver_id,
            false,
            None,
        )
    }

    /// Reopen a conformance-test journal (synthetic funding admitted).
    pub fn reopen_for_conformance_tests(
        run_dir: &Path,
        expected_caps: &JournalCaps,
        expected_allocation_digest: &str,
        expected_driver_id: &str,
    ) -> Result<Self, JournalError> {
        Self::reopen_inner(
            run_dir,
            expected_caps,
            expected_allocation_digest,
            expected_driver_id,
            true,
            None,
        )
    }

    pub(crate) fn reopen_bound(
        run_dir: &Path,
        caps: &JournalCaps,
        allocation_digest: &str,
        driver_id: &str,
        identity: (&str, &str, &str),
        allow_fixtures: bool,
    ) -> Result<Self, JournalError> {
        Self::reopen_inner(
            run_dir,
            caps,
            allocation_digest,
            driver_id,
            allow_fixtures,
            Some(identity),
        )
    }

    fn reopen_inner(
        run_dir: &Path,
        expected_caps: &JournalCaps,
        expected_allocation_digest: &str,
        expected_driver_id: &str,
        allow_fixture_funding: bool,
        expected_identity: Option<(&str, &str, &str)>,
    ) -> Result<Self, JournalError> {
        expected_caps.validate()?;
        let path = Self::path_in(run_dir);
        let mut file = OpenOptions::new()
            .append(true)
            .read(true)
            .open(&path)
            .map_err(|e| JournalError::Io(format!("open {}: {e}", path.display())))?;
        // H4: exclusive lifetime lock BEFORE replay or truncation.
        lock_or_fail(&file, &path)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|e| JournalError::Io(format!("read {}: {e}", path.display())))?;
        // Torn-final-line tolerance: a record is only durable once its
        // terminating newline is on disk. Truncate the torn tail.
        let durable_len = match bytes.iter().rposition(|&b| b == b'\n') {
            Some(pos) => pos + 1,
            None => {
                if bytes.is_empty() {
                    0
                } else {
                    return Err(JournalError::Integrity(format!(
                        "journal {} has no complete record and no header",
                        path.display()
                    )));
                }
            }
        };
        // No disk repair until the complete prefix and all bindings pass.
        // In particular a v2 file with a torn tail is refused byte-for-byte.
        let tail = bytes.split_off(durable_len);
        let mut state = JournalState::new(file, durable_len as u64, JournalCaps::default());
        let text = String::from_utf8(bytes)
            .map_err(|_| JournalError::Integrity("journal is not valid UTF-8".to_string()))?;
        for (line_number, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let record: JournalRecord = parse_record(line).map_err(|e| {
                let mut message = format!(
                    "journal line {} is not a parseable record: {e}",
                    line_number + 1
                );
                if line_number == 0 {
                    message.push_str(
                        " (the first line must be the history-3 header; older journals \
                          are refused without migration)",
                    );
                }
                JournalError::Integrity(message)
            })?;
            validate_record(&state, &record, allow_fixture_funding).map_err(|e| {
                JournalError::Integrity(format!(
                    "journal line {} fails replay validation: {e}",
                    line_number + 1,
                ))
            })?;
            commit_record(&mut state, record);
        }
        if !state.header_seen {
            return Err(JournalError::Integrity(
                "journal has no header record".to_string(),
            ));
        }
        // Binding checks, in order, all BEFORE the session record below
        // (R6): a mismatching reopen must never mutate the file. The caps
        // fire first so a cap drift is reported as `CapsMismatch` even
        // when the drifted config also implies a different digest.
        if state.caps != *expected_caps {
            return Err(JournalError::CapsMismatch {
                found: state.caps,
                expected: *expected_caps,
            });
        }
        let header = state
            .header_facts
            .as_ref()
            .expect("header_seen implies header facts");
        if header.allocation_digest != expected_allocation_digest {
            return Err(JournalError::AllocationMismatch {
                found: header.allocation_digest.clone(),
                expected: expected_allocation_digest.to_string(),
            });
        }
        if header.driver_id != expected_driver_id {
            return Err(JournalError::DriverIdMismatch {
                stored: header.driver_id.clone(),
                expected: expected_driver_id.to_string(),
            });
        }
        if let Some((run_id, fingerprint, network)) = expected_identity {
            if header.run_id != run_id
                || header.run_seed_fingerprint != fingerprint
                || header.network != network
            {
                return Err(integrity(
                    "journal run/seed/network binding mismatch".to_string(),
                ));
            }
        }
        // A complete JSON object without its newline is still uncommitted,
        // but cannot hide a malformed/unsupported proof by omitting newline.
        if serde_json::from_slice::<serde::de::IgnoredAny>(&tail).is_ok() {
            let line = std::str::from_utf8(&tail)
                .map_err(|e| JournalError::Integrity(format!("invalid uncommitted record: {e}")))?;
            let record = parse_record(line)?;
            validate_record(&state, &record, allow_fixture_funding)?;
        } else if tail_names_rollback(&tail) {
            return Err(integrity(
                "truncated rollback proof record; file preserved".to_string(),
            ));
        }
        // An incomplete tail has never been a committed record. Recover only
        // the validated prefix; never apply any part of its proof/transition.
        if !tail.is_empty() {
            state
                .file
                .set_len(durable_len as u64)
                .and_then(|_| state.file.sync_data())
                .map_err(|e| JournalError::Io(format!("truncate torn tail: {e}")))?;
        }
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| JournalError::Io(format!("chmod {}: {e}", path.display())))?;
        // R2: under the exclusive lock, after replay and every binding
        // check, append the durable session record. A failure here fails
        // the reopen (the write path rolls back or poisons) - a usable
        // but silently-uncertain handle is never returned. Replay commits
        // this record identically (deterministic).
        let session = JournalRecord::SessionStarted {
            seq: state.next_seq,
            restarted_unix_ms: crate::now_unix_ms(),
        };
        validate_record(&state, &session, allow_fixture_funding)?;
        write_line(&mut state, &session)?;
        commit_record(&mut state, session);
        Ok(Self {
            path,
            allow_fixture_funding,
            state: Mutex::new(state),
        })
    }

    /// The journal's header facts as replayed from the header record.
    pub fn header(&self) -> Result<HeaderFacts, JournalError> {
        let state = self.state.lock().unwrap();
        state
            .header_facts
            .clone()
            .ok_or_else(|| JournalError::Integrity("no header".to_string()))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn last_seq(&self) -> u64 {
        self.state.lock().unwrap().next_seq.saturating_sub(1)
    }

    /// Coherent committed copy under the same mutex used for all appends.
    /// Does no I/O and lends no mutable internal state or writer handle.
    pub fn observer_snapshot(&self) -> ObserverSnapshot {
        let state = self.state.lock().unwrap();
        ObserverSnapshot {
            committed_seq: state.next_seq - 1,
            tip: state
                .tip
                .map(|(height, block_hash)| ChainAnchor { block_hash, height }),
            outpoints: state
                .funding
                .iter()
                .map(|(outpoint, entry)| {
                    let context = match entry.context {
                        Some((block_hash, height, coinbase)) => FundingContext::CanonicalBlock {
                            block_hash: block_hash.to_string(),
                            height,
                            coinbase,
                        },
                        None if entry.source == FundingSource::OwnTransaction => {
                            FundingContext::OwnTransaction
                        }
                        None => FundingContext::Fixture,
                    };
                    (
                        *outpoint,
                        ObserverOutpoint {
                            amount_sats: entry.amount_sats,
                            script: entry.script.clone(),
                            owner: entry.owner.clone(),
                            chain: entry.chain,
                            index: entry.index,
                            source: entry.source.clone(),
                            context,
                            status: entry.status.clone(),
                            coinbase_immature: coinbase_immature(&state, entry),
                        },
                    )
                })
                .collect(),
        }
    }

    /// Capture the exact last durable send of this command for a local queue.
    pub fn pending_send_intent(&self, command_id: &str) -> Result<PendingSendIntent, JournalError> {
        let state = self.state.lock().unwrap();
        let (tx, command) = state.tx_of_command(command_id)?;
        if state.poisoned || tx.state != TxState::BroadcastSent {
            return Err(integrity("no live committed send intent".to_string()));
        }
        let (send_sequence, intent_seq) = tx.last_send.expect("broadcast has a send record");
        Ok(PendingSendIntent {
            header: state.header_facts.clone().expect("header"),
            txid: command.txid.expect("signed command"),
            send_sequence,
            intent_seq,
        })
    }

    /// Check immediately before LOCAL forwarding under the controller's
    /// serialization of queue + chain updates + forward. The mutex held here
    /// ends on return: the integrator MUST serialize that whole boundary.
    /// No remote chain atomicity is claimed. Already forwarded results must
    /// still be recorded honestly, including Unknown; refusal writes nothing.
    pub fn check_send_before_forward(
        &self,
        intent: &PendingSendIntent,
        expected_committed_seq: u64,
    ) -> Result<(), JournalError> {
        let state = self.state.lock().unwrap();
        if state.poisoned
            || state.barrier_active
            || state.next_seq - 1 != expected_committed_seq
            || state.header_facts.as_ref() != Some(&intent.header)
        {
            return Err(integrity(
                "stale, foreign or blocked queued send".to_string(),
            ));
        }
        let tx = state
            .txs
            .get(&intent.txid)
            .ok_or_else(|| integrity("unknown queued transaction".to_string()))?;
        if tx.state != TxState::BroadcastSent
            || tx.last_send != Some((intent.send_sequence, intent.intent_seq))
        {
            return Err(integrity(
                "queued send identity is no longer current".to_string(),
            ));
        }
        validate_send_funding(&state, tx)
    }

    pub fn barrier_active(&self) -> bool {
        self.state.lock().unwrap().barrier_active
    }

    pub fn next_send_sequence(&self) -> u64 {
        self.state.lock().unwrap().next_send_sequence
    }

    /// The journal's retry caps (from the header).
    pub fn caps(&self) -> JournalCaps {
        self.state.lock().unwrap().caps
    }

    /// The canonical chain tip height as last observed (`None` before any
    /// `ChainObserved` record; coinbase funding is not reservable then).
    pub fn tip_height(&self) -> Option<u64> {
        self.state.lock().unwrap().tip.map(|(height, _)| height)
    }

    /// The phase of a journaled command, if it exists.
    pub fn command_phase(&self, command_id: &str) -> Option<CommandPhase> {
        self.state
            .lock()
            .unwrap()
            .commands
            .get(command_id)
            .map(|entry| entry.phase)
    }

    /// Inject a one-shot write fault (fault-injection tests only, M4).
    /// The fault is consumed by the next append; if its rollback cannot
    /// be proven, the handle is poisoned and every further mutation fails
    /// with [`JournalError::Poisoned`].
    pub fn _for_tests_set_write_fault(&self, fault: WriteFault) -> Result<(), JournalError> {
        let mut state = self.state.lock().unwrap();
        if state.poisoned {
            return Err(JournalError::Poisoned);
        }
        state.write_fault = Some(fault);
        Ok(())
    }

    /// Observe funding with explicit provenance (H5). The observation
    /// carries the owner's derivation `(chain, index)`; `owner_script`
    /// must be the owner's script derived at exactly that index - the
    /// wallet set enforces this (the caller of this lower-level API is
    /// trusted to derive, and the spending path re-derives and compares,
    /// so a foreign script can never be spent). The outpoint must be new,
    /// unless it re-validates a previously invalidated canonical entry.
    /// Synthetic sources are refused unless the journal was created by
    /// the conformance-test constructor.
    pub fn observe_funding(
        &self,
        observation: &FundingObservation,
        owner_script: &ScriptBuf,
    ) -> Result<(), JournalError> {
        let mut state = self.state.lock().unwrap();
        let record = JournalRecord::FundingObserved {
            seq: state.next_seq,
            outpoint: outpoint_to_string(&observation.outpoint),
            amount_sats: observation.amount_sats,
            script_hex: owner_script.to_hex_string(),
            chain: observation.chain,
            index: observation.index,
            owner_role: observation.owner_role.clone(),
            source: observation.source.clone(),
            context: observation.context.clone(),
            observed_unix_ms: crate::now_unix_ms(),
        };
        validate_record(&state, &record, self.allow_fixture_funding)?;
        write_line(&mut state, &record)?;
        commit_record(&mut state, record);
        Ok(())
    }

    /// Begin a command (dedup by id + payload digest; the digest is
    /// computed by the journal over the full `RequestRecord`, including
    /// the payer role and category - M1). A different record for a known
    /// id is journaled as a conflict and fails.
    pub fn begin_command(
        &self,
        command_id: &str,
        request: RequestRecord,
    ) -> Result<CommandBeginOutcome, JournalError> {
        let mut state = self.state.lock().unwrap();
        let payload_digest = request_record_digest(&request);
        let existing = state
            .commands
            .get(command_id)
            .map(|entry| entry.payload_digest.clone());
        if let Some(existing_digest) = existing {
            if existing_digest != payload_digest {
                let record = JournalRecord::CommandConflict {
                    seq: state.next_seq,
                    command_id: command_id.to_string(),
                    existing_digest: existing_digest.clone(),
                    incoming_digest: payload_digest.clone(),
                };
                validate_record(&state, &record, self.allow_fixture_funding)?;
                write_line(&mut state, &record)?;
                commit_record(&mut state, record);
                return Err(JournalError::CommandConflict {
                    command: command_id.to_string(),
                    existing: existing_digest,
                    incoming: payload_digest,
                });
            }
            return Ok(CommandBeginOutcome::Stored(Box::new(
                state
                    .stored_view(command_id)
                    .expect("entry was just checked"),
            )));
        }
        let record = JournalRecord::CommandBegin {
            seq: state.next_seq,
            command_id: command_id.to_string(),
            request,
        };
        validate_record(&state, &record, self.allow_fixture_funding)?;
        write_line(&mut state, &record)?;
        commit_record(&mut state, record);
        Ok(CommandBeginOutcome::New)
    }

    /// Durably reserve outpoints for a command **before** signing. The
    /// reservations are exclusive: an unavailable, already-reserved,
    /// possibly-spent, spent, invalidated or immature-coinbase outpoint
    /// fails; a foreign owner fails.
    pub fn reserve(
        &self,
        command_id: &str,
        outpoints: &[OutPoint],
        change_index: u32,
    ) -> Result<(), JournalError> {
        let mut state = self.state.lock().unwrap();
        let record = JournalRecord::Reserved {
            seq: state.next_seq,
            command_id: command_id.to_string(),
            outpoints: outpoints.iter().map(outpoint_to_string).collect(),
            change_index,
        };
        validate_record(&state, &record, self.allow_fixture_funding)?;
        write_line(&mut state, &record)?;
        commit_record(&mut state, record);
        Ok(())
    }

    /// Record the signed transaction of a reserved command. The raw bytes
    /// are verified against the txid, the weight/vbytes are remeasured,
    /// the fee is recomputed from the funding ledger, and the **decoded**
    /// inputs must be exactly the reserved set (M6) - all before the
    /// record becomes durable.
    pub fn record_signed(
        &self,
        command_id: &str,
        signed: &SignedTrafficTx,
        fee_rate_sats_vb: u64,
    ) -> Result<(), JournalError> {
        let mut state = self.state.lock().unwrap();
        let payer_role = state.command(command_id)?.wallet_role.clone();
        let record = JournalRecord::SignedTx {
            seq: state.next_seq,
            command_id: command_id.to_string(),
            txid: signed.txid.to_string(),
            raw_hex: hex::encode(&signed.raw_bytes),
            vbytes: signed.vbytes,
            weight_wu: signed.weight_wu,
            fee_sats: signed.fee_sats,
            fee_rate_sats_vb,
            category: signed.category.as_str(),
            payer_role,
            spent_outpoints: signed
                .spent_outpoints
                .iter()
                .map(outpoint_to_string)
                .collect(),
            destinations: signed
                .destinations
                .iter()
                .map(|d| DestinationRecord {
                    script_hex: d.script_pubkey.to_hex_string(),
                    amount_sats: d.amount_sats,
                    owner_role: d.owner.clone(),
                })
                .collect(),
            change: signed.change.as_ref().map(|c| ChangeRecord {
                script_hex: c.script_pubkey.to_hex_string(),
                amount_sats: c.amount_sats,
                index: c.index,
            }),
            size_class: None,
        };
        validate_record(&state, &record, self.allow_fixture_funding)?;
        write_line(&mut state, &record)?;
        commit_record(&mut state, record);
        Ok(())
    }

    /// Finish a command with a terminal outcome string.
    pub fn finish_command(&self, command_id: &str, outcome: &str) -> Result<(), JournalError> {
        let mut state = self.state.lock().unwrap();
        let txid = state
            .commands
            .get(command_id)
            .and_then(|entry| entry.txid)
            .map(|txid| txid.to_string());
        let record = JournalRecord::CommandFinished {
            seq: state.next_seq,
            command_id: command_id.to_string(),
            outcome: outcome.to_string(),
            txid,
        };
        validate_record(&state, &record, self.allow_fixture_funding)?;
        write_line(&mut state, &record)?;
        commit_record(&mut state, record);
        Ok(())
    }

    /// Release the reservations of a command that failed construction
    /// **before** any signed bytes were recorded. Legal only from the
    /// `Reserved` phase; a signed command must walk the lifecycle instead
    /// (an `unknown` result never releases inputs). Reservations on
    /// funding that was invalidated in the meantime stay invalidated
    /// (only the reservation is dropped, never the reorg evidence).
    pub fn release_failed_command(
        &self,
        command_id: &str,
        reason: &str,
    ) -> Result<(), JournalError> {
        let mut state = self.state.lock().unwrap();
        let record = JournalRecord::InputsReleased {
            seq: state.next_seq,
            command_id: command_id.to_string(),
            reason: reason.to_string(),
        };
        validate_record(&state, &record, self.allow_fixture_funding)?;
        write_line(&mut state, &record)?;
        commit_record(&mut state, record);
        Ok(())
    }

    /// Record that the signed bytes left the process (monotonic send
    /// sequence; refused after a pause barrier until `SendResumed`).
    /// Retry-send admission (H3): a send from the `unknown` state is
    /// admitted only while `retry_sends < caps.unknown_retry_sends`;
    /// a cap of N therefore bounds the journal to N+1 total sends.
    pub fn record_broadcast_sent(&self, command_id: &str) -> Result<u64, JournalError> {
        let mut state = self.state.lock().unwrap();
        let txid = txid_of(&state, command_id)?;
        let record = JournalRecord::BroadcastSent {
            seq: state.next_seq,
            command_id: command_id.to_string(),
            txid: txid.to_string(),
            send_sequence: state.next_send_sequence,
        };
        validate_record(&state, &record, self.allow_fixture_funding)?;
        write_line(&mut state, &record)?;
        let send_sequence = match &record {
            JournalRecord::BroadcastSent { send_sequence, .. } => *send_sequence,
            _ => unreachable!(),
        };
        commit_record(&mut state, record);
        Ok(send_sequence)
    }

    /// Record RPC acceptance (RPC success only - not canonical).
    pub fn record_accepted_by_rpc(
        &self,
        command_id: &str,
        node_id: &str,
    ) -> Result<(), JournalError> {
        let mut state = self.state.lock().unwrap();
        let txid = txid_of(&state, command_id)?;
        let record = JournalRecord::AcceptedByRpc {
            seq: state.next_seq,
            command_id: command_id.to_string(),
            txid: txid.to_string(),
            node_id: node_id.to_string(),
        };
        validate_record(&state, &record, self.allow_fixture_funding)?;
        write_line(&mut state, &record)?;
        commit_record(&mut state, record);
        Ok(())
    }

    /// Record mempool membership observed on a declared node.
    pub fn record_observed_in_mempool(
        &self,
        command_id: &str,
        node_id: &str,
    ) -> Result<(), JournalError> {
        let mut state = self.state.lock().unwrap();
        let txid = txid_of(&state, command_id)?;
        let record = JournalRecord::ObservedInMempool {
            seq: state.next_seq,
            command_id: command_id.to_string(),
            txid: txid.to_string(),
            node_id: node_id.to_string(),
        };
        validate_record(&state, &record, self.allow_fixture_funding)?;
        write_line(&mut state, &record)?;
        commit_record(&mut state, record);
        Ok(())
    }

    /// Record canonical confirmation. Marks the spent outpoints spent and
    /// revalidates any funding this transaction's outputs back (H2
    /// cascade). Refused while any spent input is invalidated. From
    /// `Reorged`, a re-inclusion naming the displaced block itself is
    /// refused on STALE evidence only: it becomes admissible once a
    /// `ChainObserved` newer than the displacement has been committed
    /// (S3 - the chain reorged back and the block is canonical again).
    pub fn record_confirmed(
        &self,
        command_id: &str,
        block_hash: BlockHash,
        height: u64,
    ) -> Result<(), JournalError> {
        let mut state = self.state.lock().unwrap();
        let txid = txid_of(&state, command_id)?;
        let record = JournalRecord::Confirmed {
            seq: state.next_seq,
            command_id: command_id.to_string(),
            txid: txid.to_string(),
            block_hash: block_hash.to_string(),
            height,
        };
        validate_record(&state, &record, self.allow_fixture_funding)?;
        write_line(&mut state, &record)?;
        commit_record(&mut state, record);
        Ok(())
    }

    /// Record an explicit node rejection (never inferred from silence).
    /// Releases the reserved inputs back to available **only** if the
    /// payment was never uncertain (never unknown, never accepted by an
    /// RPC node, never reorg-resubmitted - H1); otherwise the inputs
    /// degrade to possibly-spent, because at least one send's outcome is
    /// not accounted for by this rejection.
    pub fn record_rejected(&self, command_id: &str, code: &str) -> Result<(), JournalError> {
        let mut state = self.state.lock().unwrap();
        let txid = txid_of(&state, command_id)?;
        let record = JournalRecord::Rejected {
            seq: state.next_seq,
            command_id: command_id.to_string(),
            txid: txid.to_string(),
            code: code.to_string(),
        };
        validate_record(&state, &record, self.allow_fixture_funding)?;
        write_line(&mut state, &record)?;
        commit_record(&mut state, record);
        Ok(())
    }

    /// Record an unknown outcome (timeout/transport). Inputs stay
    /// reserved. **Recording outcomes is never capped** (H3): the caps
    /// govern retry-send admission, not honest reporting. Returns the
    /// attempt number.
    pub fn record_unknown(&self, command_id: &str, reason: &str) -> Result<u32, JournalError> {
        let mut state = self.state.lock().unwrap();
        let txid = txid_of(&state, command_id)?;
        let attempt = state
            .txs
            .get(&txid)
            .map(|tx| tx.unknown_retries + 1)
            .unwrap_or(1);
        let record = JournalRecord::UnknownOutcome {
            seq: state.next_seq,
            command_id: command_id.to_string(),
            txid: txid.to_string(),
            reason: reason.to_string(),
            attempt,
        };
        validate_record(&state, &record, self.allow_fixture_funding)?;
        write_line(&mut state, &record)?;
        commit_record(&mut state, record);
        Ok(attempt)
    }

    /// Record a terminal conflict (a conflicting spend was admitted).
    pub fn record_conflicted(&self, command_id: &str, reason: &str) -> Result<(), JournalError> {
        let mut state = self.state.lock().unwrap();
        let txid = txid_of(&state, command_id)?;
        let record = JournalRecord::Conflicted {
            seq: state.next_seq,
            command_id: command_id.to_string(),
            txid: txid.to_string(),
            reason: reason.to_string(),
        };
        validate_record(&state, &record, self.allow_fixture_funding)?;
        write_line(&mut state, &record)?;
        commit_record(&mut state, record);
        Ok(())
    }

    /// Record canonical displacement of a confirmed transaction. The
    /// spent-outpoint claims degrade to possibly-spent (preserved, never
    /// silently released) and the transaction's own outputs lose their
    /// funding backing (the H2 cascade invalidates them and every
    /// descendant that spends them).
    pub fn record_reorged(
        &self,
        command_id: &str,
        from_block_hash: BlockHash,
        height: u64,
    ) -> Result<(), JournalError> {
        let mut state = self.state.lock().unwrap();
        let txid = txid_of(&state, command_id)?;
        let record = JournalRecord::Reorged {
            seq: state.next_seq,
            command_id: command_id.to_string(),
            txid: txid.to_string(),
            from_block_hash: from_block_hash.to_string(),
            height,
        };
        validate_record(&state, &record, self.allow_fixture_funding)?;
        write_line(&mut state, &record)?;
        commit_record(&mut state, record);
        Ok(())
    }

    /// Perform a bounded same-bytes reorg resubmission. Eligibility is
    /// fail-closed: the state must be `Reorged`, the retry cap must not
    /// be exceeded, the supplied bytes must be byte-identical to the
    /// stored signed transaction, every original input must be proven
    /// unspent from canonical blocks, and no input may be invalidated.
    pub fn record_reorg_resubmit(
        &self,
        command_id: &str,
        raw_bytes: &[u8],
        proven_unspent: &[OutPoint],
    ) -> Result<u64, JournalError> {
        let mut state = self.state.lock().unwrap();
        let txid = txid_of(&state, command_id)?;
        let stored_hex = state
            .txs
            .get(&txid)
            .map(|tx| tx.raw_hex.clone())
            .ok_or_else(|| JournalError::UnknownTx(txid.to_string()))?;
        if hex::encode(raw_bytes) != stored_hex {
            return Err(JournalError::ReorgRetryNotEligible {
                txid: txid.to_string(),
                reason: "resubmission bytes differ from the stored signed transaction".to_string(),
            });
        }
        let record = JournalRecord::ReorgResubmit {
            seq: state.next_seq,
            command_id: command_id.to_string(),
            txid: txid.to_string(),
            proven_unspent: proven_unspent.iter().map(outpoint_to_string).collect(),
            send_sequence: state.next_send_sequence,
        };
        validate_record(&state, &record, self.allow_fixture_funding)?;
        write_line(&mut state, &record)?;
        let send_sequence = match &record {
            JournalRecord::ReorgResubmit { send_sequence, .. } => *send_sequence,
            _ => unreachable!(),
        };
        commit_record(&mut state, record);
        Ok(send_sequence)
    }

    /// Record a canonical chain observation (H5): the tip drives coinbase
    /// maturity, and every own confirmed transaction or canonical
    /// funding entry in a displaced block takes the reorg transition
    /// (txs -> `Reorged` with possibly-spent claims; funding ->
    /// `Invalidated`), after which the derived cascade invalidates
    /// descendants. An empty displaced list is a pure tip advance.
    pub fn record_chain_observed(
        &self,
        tip_block_hash: BlockHash,
        tip_height: u64,
        displaced_block_hashes: &[BlockHash],
    ) -> Result<(), JournalError> {
        let mut state = self.state.lock().unwrap();
        let record = JournalRecord::ChainObserved {
            seq: state.next_seq,
            tip_block_hash: tip_block_hash.to_string(),
            tip_height,
            displaced_block_hashes: displaced_block_hashes
                .iter()
                .map(|hash| hash.to_string())
                .collect(),
            observed_unix_ms: crate::now_unix_ms(),
        };
        validate_record(&state, &record, self.allow_fixture_funding)?;
        write_line(&mut state, &record)?;
        commit_record(&mut state, record);
        Ok(())
    }

    /// Admit a lower canonical tip only with complete, checked evidence.
    /// Validation, durable append and cascade commit share the owner mutex.
    pub fn record_chain_rollback(&self, evidence: RollbackEvidence) -> Result<(), JournalError> {
        let mut state = self.state.lock().unwrap();
        let record = JournalRecord::ChainRollback {
            seq: state.next_seq,
            evidence,
            observed_unix_ms: crate::now_unix_ms(),
        };
        validate_record(&state, &record, self.allow_fixture_funding)?;
        write_line(&mut state, &record)?;
        commit_record(&mut state, record);
        Ok(())
    }

    /// Record the pause acknowledgment barrier (plan 6.4): wall time,
    /// monotonic time, the last journaled txid, and the monotonic send
    /// sequence at the barrier. No broadcast may be journaled afterwards
    /// until `record_send_resumed`.
    pub fn record_send_barrier(
        &self,
        command_id: &str,
        monotonic_ms: u64,
    ) -> Result<(), JournalError> {
        let mut state = self.state.lock().unwrap();
        let last_txid = state.last_broadcast_txid.map(|txid| txid.to_string());
        let record = JournalRecord::SendBarrier {
            seq: state.next_seq,
            command_id: command_id.to_string(),
            wall_unix_ms: crate::now_unix_ms(),
            monotonic_ms,
            last_txid,
            send_sequence: state.next_send_sequence.saturating_sub(1),
        };
        validate_record(&state, &record, self.allow_fixture_funding)?;
        write_line(&mut state, &record)?;
        commit_record(&mut state, record);
        Ok(())
    }

    /// Record that sends resumed after a barrier.
    pub fn record_send_resumed(&self, command_id: &str) -> Result<(), JournalError> {
        let mut state = self.state.lock().unwrap();
        let record = JournalRecord::SendResumed {
            seq: state.next_seq,
            command_id: command_id.to_string(),
        };
        validate_record(&state, &record, self.allow_fixture_funding)?;
        write_line(&mut state, &record)?;
        commit_record(&mut state, record);
        Ok(())
    }

    /// The stored command view, if the command exists.
    pub fn command(&self, command_id: &str) -> Option<StoredCommand> {
        self.state.lock().unwrap().stored_view(command_id)
    }

    /// One coherent clone under the existing journal mutex, with no file I/O,
    /// second replay or public-method relock. Authoritative caps, maturity and
    /// identity validation remain in the existing mutation paths.
    pub fn recovery_snapshot(&self) -> RecoverySnapshot {
        let state = self.state.lock().unwrap();
        RecoverySnapshot {
            committed_seq: state.next_seq.saturating_sub(1),
            tip: state
                .tip
                .map(|(height, block_hash)| ChainAnchor { block_hash, height }),
            barrier_active: state.barrier_active,
            next_send_sequence: state.next_send_sequence,
            last_broadcast_txid: state.last_broadcast_txid,
            last_chain_observed_seq: state.last_chain_observed_seq,
            caps: state.caps,
            poisoned: state.poisoned,
            commands: state
                .commands
                .keys()
                .map(|id| (id.clone(), state.stored_view(id).expect("existing command")))
                .collect(),
            transactions: state
                .txs
                .iter()
                .map(|(txid, tx)| {
                    (
                        *txid,
                        RecoveryTransaction {
                            facts: state.tx_facts(txid).expect("existing transaction"),
                            last_send: tx.last_send,
                            reorged_from_block: tx.reorged_from_block,
                            displaced_at_seq: tx.displaced_at_seq,
                            invalidated_prior: tx.invalidated_prior,
                        },
                    )
                })
                .collect(),
        }
    }

    /// The lifecycle state of a journaled transaction.
    pub fn tx_state(&self, txid: &Txid) -> Option<TxState> {
        self.state.lock().unwrap().txs.get(txid).map(|tx| tx.state)
    }

    /// The stored raw bytes of a journaled transaction.
    pub fn tx_raw_hex(&self, txid: &Txid) -> Option<String> {
        self.state
            .lock()
            .unwrap()
            .txs
            .get(txid)
            .map(|tx| tx.raw_hex.clone())
    }

    /// The measured facts of a journaled transaction (for replay views).
    pub fn tx_facts(&self, txid: &Txid) -> Option<SignedTxFacts> {
        self.state.lock().unwrap().tx_facts(txid)
    }

    /// The funding-ledger facts of one outpoint (value, script,
    /// derivation locus, owner).
    pub fn outpoint_facts(
        &self,
        outpoint: &OutPoint,
    ) -> Option<(u64, ScriptBuf, Chain, u32, WalletRole)> {
        let state = self.state.lock().unwrap();
        state.funding.get(outpoint).map(|entry| {
            (
                entry.amount_sats,
                entry.script.clone(),
                entry.chain,
                entry.index,
                entry.owner.clone(),
            )
        })
    }

    /// The recorded provenance of one funded outpoint.
    pub fn funding_source(&self, outpoint: &OutPoint) -> Option<FundingSource> {
        let state = self.state.lock().unwrap();
        state
            .funding
            .get(outpoint)
            .map(|entry| entry.source.clone())
    }

    /// All currently available (unreserved, unspent, non-invalidated,
    /// mature) outpoints owned by a role, in deterministic order.
    /// Coinbase funding counts as available only at
    /// [`COINBASE_MATURITY`] confirmations of the observed tip (H5); with
    /// no observed tip it is never available.
    pub fn available_outpoints(
        &self,
        role: &WalletRole,
    ) -> Vec<(OutPoint, u64, Chain, u32, ScriptBuf)> {
        let state = self.state.lock().unwrap();
        let mut result: Vec<(OutPoint, u64, Chain, u32, ScriptBuf)> = state
            .funding
            .iter()
            .filter(|(_, entry)| {
                entry.owner == *role
                    && matches!(entry.status, OutpointStatus::Available)
                    && !coinbase_immature(&state, entry)
            })
            .map(|(outpoint, entry)| {
                (
                    *outpoint,
                    entry.amount_sats,
                    entry.chain,
                    entry.index,
                    entry.script.clone(),
                )
            })
            .collect();
        result.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then(a.0.txid.cmp(&b.0.txid))
                .then(a.0.vout.cmp(&b.0.vout))
        });
        result
    }

    /// The available balance of a role (sats).
    pub fn balance_sats(&self, role: &WalletRole) -> u64 {
        self.available_outpoints(role)
            .iter()
            .map(|(_, amount, _, _, _)| *amount)
            .sum()
    }

    /// The deterministic next change index for a role (one index is
    /// consumed by every reservation).
    pub fn next_change_index(&self, role: &WalletRole) -> u32 {
        self.state
            .lock()
            .unwrap()
            .change_index_next
            .get(role)
            .copied()
            .unwrap_or(0)
    }

    /// The deterministic next receive index for a role (one index is
    /// consumed by every funding observation).
    pub fn next_receive_index(&self, role: &WalletRole) -> u32 {
        self.state
            .lock()
            .unwrap()
            .receive_index_next
            .get(role)
            .copied()
            .unwrap_or(0)
    }

    /// The identity-free ledger summary.
    pub fn summary(&self) -> LedgerSummary {
        let state = self.state.lock().unwrap();
        let mut summary = LedgerSummary {
            funding_observations: state.funding.len() as u64,
            commands_begun: state.commands.len() as u64,
            transactions_signed: state.txs.len() as u64,
            send_sequence: state.next_send_sequence.saturating_sub(1),
            barrier_active: state.barrier_active,
            ..LedgerSummary::default()
        };
        for entry in state.funding.values() {
            match &entry.status {
                OutpointStatus::Available => {
                    summary.available_outpoints += 1;
                    summary.available_sats += entry.amount_sats;
                }
                OutpointStatus::Reserved { .. } => summary.reserved_outpoints += 1,
                OutpointStatus::PossiblySpent => summary.possibly_spent_outpoints += 1,
                OutpointStatus::Spent => summary.spent_outpoints += 1,
                OutpointStatus::Invalidated { .. } => summary.invalidated_outpoints += 1,
            }
        }
        for command in state.commands.values() {
            if command.phase == CommandPhase::Finished {
                summary.commands_finished += 1;
            } else {
                summary.commands_in_flight += 1;
            }
        }
        for tx in state.txs.values() {
            match tx.state {
                TxState::Confirmed => summary.transactions_confirmed += 1,
                TxState::Rejected => summary.transactions_rejected += 1,
                TxState::Conflicted => summary.transactions_conflicted += 1,
                TxState::Unknown => summary.transactions_unknown += 1,
                TxState::Reorged => summary.transactions_reorged += 1,
                TxState::Invalidated => summary.transactions_invalidated += 1,
                _ => {}
            }
        }
        summary
    }
}

fn txid_of(state: &JournalState, command_id: &str) -> Result<Txid, JournalError> {
    state
        .command(command_id)?
        .txid
        .ok_or_else(|| JournalError::UnknownTx(command_id.to_string()))
}

fn integrity(reason: impl Into<String>) -> JournalError {
    JournalError::Integrity(reason.into())
}

/// Acquire the exclusive non-blocking lifetime flock (H4). A live second
/// owner fails with [`JournalError::HeldByAnotherOwner`]; the kernel
/// releases the lock when the owning process exits or the handle drops.
fn lock_or_fail(file: &File, path: &Path) -> Result<(), JournalError> {
    match rustix::fs::flock(file, rustix::fs::FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => Ok(()),
        Err(error)
            if error == rustix::io::Errno::WOULDBLOCK || error == rustix::io::Errno::AGAIN =>
        {
            Err(JournalError::HeldByAnotherOwner {
                path: path.display().to_string(),
            })
        }
        Err(error) => Err(JournalError::Io(format!(
            "flock {}: {error}",
            path.display()
        ))),
    }
}

fn outpoint_to_string(outpoint: &OutPoint) -> String {
    format!("{}:{}", outpoint.txid, outpoint.vout)
}

fn parse_outpoint(value: &str) -> Result<OutPoint, JournalError> {
    let (txid, vout) = value
        .rsplit_once(':')
        .ok_or_else(|| integrity(format!("malformed outpoint {value:?}")))?;
    let txid = Txid::from_str(txid)
        .map_err(|_| integrity(format!("malformed txid in outpoint {value:?}")))?;
    let vout: u32 = vout
        .parse()
        .map_err(|_| integrity(format!("malformed vout in outpoint {value:?}")))?;
    Ok(OutPoint { txid, vout })
}

fn parse_script(value: &str) -> Result<ScriptBuf, JournalError> {
    ScriptBuf::from_hex(value).map_err(|_| integrity(format!("malformed script hex {value:?}")))
}

fn decode_raw_tx(raw_hex: &str, expected_txid: &str) -> Result<Transaction, JournalError> {
    let bytes = hex::decode(raw_hex)
        .map_err(|_| integrity("signed raw hex is not valid hex".to_string()))?;
    let tx = Transaction::consensus_decode(&mut bytes.as_slice())
        .map_err(|e| integrity(format!("signed raw bytes do not decode: {e}")))?;
    let txid = tx.compute_txid();
    if txid.to_string() != expected_txid {
        return Err(JournalError::RawTxidMismatch {
            txid: expected_txid.to_string(),
        });
    }
    Ok(tx)
}

/// Whether a canonical coinbase entry is immature at the observed tip
/// (H5). With no observed tip, coinbase maturity is unprovable, so the
/// entry is treated as immature (fail closed).
fn coinbase_immature(state: &JournalState, entry: &OutpointEntry) -> bool {
    match entry.context {
        Some((_, height, true)) => match state.tip {
            Some((tip_height, _)) => height
                .checked_add(COINBASE_MATURITY)
                .is_none_or(|mature_at| tip_height < mature_at),
            None => true,
        },
        _ => false,
    }
}

/// Follow owned creation ancestry iteratively (bounded by the committed
/// funding DAG, no recursive stack growth). Reservations and spent ancestor
/// inputs remain valid claims, but invalidated/missing/immature ones block.
fn validate_send_funding(state: &JournalState, tx: &TxEntry) -> Result<(), JournalError> {
    let mut pending = tx.spent_outpoints.clone();
    let mut visited = std::collections::BTreeSet::new();
    while let Some(outpoint) = pending.pop() {
        if !visited.insert(outpoint) {
            continue;
        }
        let entry = state
            .funding
            .get(&outpoint)
            .ok_or_else(|| integrity("send ancestry missing funding".to_string()))?;
        if matches!(entry.status, OutpointStatus::Invalidated { .. })
            || coinbase_immature(state, entry)
        {
            return Err(JournalError::IllegalTransition {
                txid: tx.command_id.clone(),
                reason: format!(
                    "send ancestry {outpoint} is invalidated or immature at committed tip"
                ),
            });
        }
        if entry.source == FundingSource::OwnTransaction {
            let creator = state
                .txs
                .get(&outpoint.txid)
                .ok_or_else(|| integrity("send ancestry missing creator".to_string()))?;
            if creator.state != TxState::Confirmed {
                return Err(integrity(
                    "send ancestry creator is not confirmed".to_string(),
                ));
            }
            pending.extend(creator.spent_outpoints.iter().copied());
        }
    }
    Ok(())
}

fn validate_rollback(state: &JournalState, proof: &RollbackEvidence) -> Result<(), JournalError> {
    if proof.version != 1 {
        return Err(integrity("unsupported rollback proof version".to_string()));
    }
    if state.tip != Some((proof.old_anchor.height, proof.old_anchor.block_hash))
        || proof.new_anchor.height >= proof.old_anchor.height
        || proof.common_ancestor.height > proof.new_anchor.height
    {
        return Err(integrity("inconsistent rollback anchors".to_string()));
    }
    let branch =
        |anchor: &ChainAnchor, headers: &[String]| -> Result<Vec<BlockHash>, JournalError> {
            let count = anchor
                .height
                .checked_sub(proof.common_ancestor.height)
                .and_then(|n| n.checked_add(1))
                .ok_or_else(|| integrity("rollback height overflow".to_string()))?;
            if count > MAX_ROLLBACK_HEADERS as u64 || headers.len() as u64 != count {
                return Err(integrity(
                    "rollback branch length/bound mismatch".to_string(),
                ));
            }
            let mut expected = anchor.block_hash;
            let mut hashes = Vec::new();
            let mut seen = std::collections::BTreeSet::new();
            for raw in headers {
                if raw.len() != 160
                    || !raw
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                {
                    return Err(integrity(
                        "rollback header must be 80-byte lowercase hex".to_string(),
                    ));
                }
                let bytes = hex::decode(raw).expect("checked hex");
                let header: bitcoin::block::Header = bitcoin::consensus::deserialize(&bytes)
                    .map_err(|_| integrity("malformed rollback header".to_string()))?;
                let hash = header.block_hash();
                let target = header.target();
                // Consensus SetCompact validity (including exponents 33/34),
                // before using rust-bitcoin's unsigned Target representation.
                let bits = header.bits.to_consensus();
                let size = bits >> 24;
                let word = bits & 0x007f_ffff;
                let negative = word != 0 && bits & 0x0080_0000 != 0;
                let overflow = word != 0
                    && (size > 34 || (word > 0xff && size > 33) || (word > 0xffff && size > 32));
                if hash != expected
                    || !seen.insert(hash)
                    || negative
                    || overflow
                    || target == bitcoin::Target::ZERO
                    || target > bitcoin::Target::MAX_ATTAINABLE_REGTEST
                    || header.validate_pow(target).is_err()
                {
                    return Err(integrity(
                        "rollback header hash/link/PoW/target mismatch".to_string(),
                    ));
                }
                hashes.push(hash);
                expected = header.prev_blockhash;
            }
            if hashes.last() != Some(&proof.common_ancestor.block_hash) {
                return Err(integrity("rollback common ancestor mismatch".to_string()));
            }
            Ok(hashes)
        };
    let old = branch(&proof.old_anchor, &proof.old_headers)?;
    let new = branch(&proof.new_anchor, &proof.new_headers)?;
    // An earlier shared header would contradict the claimed common ancestor.
    let new_set: std::collections::BTreeSet<_> = new.iter().copied().collect();
    if old[..old.len() - 1].iter().any(|h| new_set.contains(h))
        || proof.old_headers.last() != proof.new_headers.last()
        || proof.displaced_block_hashes != old[..old.len() - 1]
    {
        return Err(integrity(
            "rollback displacement/common-ancestor linkage mismatch".to_string(),
        ));
    }
    Ok(())
}

fn parse_record(line: &str) -> Result<JournalRecord, JournalError> {
    // This first pass borrows the tag and ignores every other value without
    // constructing a Value tree. In particular a giant proof cannot cause
    // unbounded enum-content/vector/header allocations before its byte cap.
    #[derive(Deserialize)]
    struct Tag<'a> {
        #[serde(borrow)]
        record: &'a str,
    }
    let tag: Tag<'_> =
        serde_json::from_str(line).map_err(|e| integrity(format!("record tag: {e}")))?;
    if tag.record == "chain_rollback" && line.len() > MAX_ROLLBACK_RECORD_BYTES {
        return Err(integrity("rollback record exceeds byte bound".to_string()));
    }
    serde_json::from_str(line).map_err(|e| integrity(format!("record decode: {e}")))
}

// Observe the tag even if a later field is truncated. This is a streaming
// syntax pass, not a Value tree or a substring heuristic. A recognizable
// rollback proof is never repaired away; an unidentifiable torn prefix
// remains subject to the existing uncommitted-tail recovery rule.
fn tail_names_rollback(bytes: &[u8]) -> bool {
    use serde::de::{DeserializeSeed, MapAccess, Visitor};
    struct Probe<'a>(&'a std::cell::Cell<bool>);
    impl<'de> Visitor<'de> for Probe<'_> {
        type Value = ();
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("journal record")
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
            while let Some(key) = map.next_key::<&str>()? {
                if key == "record" {
                    let value = map.next_value::<&str>()?;
                    if value == "chain_rollback" {
                        self.0.set(true);
                    }
                } else {
                    map.next_value::<serde::de::IgnoredAny>()?;
                }
            }
            Ok(())
        }
    }
    impl<'de> DeserializeSeed<'de> for Probe<'_> {
        type Value = ();
        fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
            d.deserialize_map(self)
        }
    }
    let found = std::cell::Cell::new(false);
    let _ = Probe(&found).deserialize(&mut serde_json::Deserializer::from_slice(bytes));
    found.get()
}

/// Whether any of the outpoints is currently invalidated.
fn any_input_invalidated(state: &JournalState, outpoints: &[OutPoint]) -> bool {
    outpoints.iter().any(|outpoint| {
        matches!(
            state.funding.get(outpoint).map(|entry| &entry.status),
            Some(OutpointStatus::Invalidated { .. })
        )
    })
}

/// The derived invalidation cascade (H2), run to a fixpoint after every
/// committed record. Deterministic from the record stream alone (replay
/// reproduces it exactly); the transaction DAG is acyclic, and every rule
/// only moves monotonically within one pass, so the loop terminates:
///
/// (a) **Own-transaction funding is backed only while its creating
///     transaction is `Confirmed`**: a reorged/rejected/conflicted
///     creator invalidates the funding (remembering the prior status);
///     a re-confirmed creator restores it (a `Spent` prior restores
///     conservatively to `PossiblySpent`).
/// (b) **A transaction with any invalidated input is `Invalidated`**,
///     remembering its prior state (a `Confirmed` prior also loses its
///     block, keeps its displaced-block identity and is stamped with
///     the displacing record's seq - S3 - and degrades its
///     confirmed-spent inputs to possibly-spent).
/// (c) **A transaction is restored only when ALL of its inputs are
///     non-invalidated**, to its remembered prior state - except that a
///     `Confirmed` prior restores to `Reorged` (the conservative
///     resubmit-and-reconfirm path). The restoration only sets the
///     state: the displaced-block identity and displacement seq
///     preserved from the ORIGINAL displacement survive it (S3).
///
/// Claims can only become *less* available here; re-credit is exclusively
/// through re-confirmation (a) / re-observation.
fn cascade_invalidation(state: &mut JournalState, record_seq: u64) {
    loop {
        let mut changed = false;
        // (a) own-transaction funding backing.
        let funding_outpoints: Vec<OutPoint> = state.funding.keys().copied().collect();
        for outpoint in funding_outpoints {
            let source = state
                .funding
                .get(&outpoint)
                .map(|entry| entry.source.clone());
            if source != Some(FundingSource::OwnTransaction) {
                continue;
            }
            let backed = state
                .txs
                .get(&outpoint.txid)
                .map(|tx| tx.state == TxState::Confirmed)
                .unwrap_or(false);
            let status = state
                .funding
                .get(&outpoint)
                .map(|entry| entry.status.clone());
            match (status, backed) {
                (Some(OutpointStatus::Invalidated { prior }), true) => {
                    let restored = match *prior {
                        OutpointStatus::Spent => OutpointStatus::PossiblySpent,
                        prior => prior,
                    };
                    let entry = state.funding.get_mut(&outpoint).expect("just read");
                    entry.status = restored;
                    changed = true;
                }
                (Some(status), false) if !matches!(status, OutpointStatus::Invalidated { .. }) => {
                    let entry = state.funding.get_mut(&outpoint).expect("just read");
                    entry.status = OutpointStatus::Invalidated {
                        prior: Box::new(status),
                    };
                    changed = true;
                }
                _ => {}
            }
        }
        // (b)/(c) transaction invalidation and restoration.
        let txids: Vec<Txid> = state.txs.keys().copied().collect();
        for txid in txids {
            let any_invalid_input = state
                .txs
                .get(&txid)
                .map(|tx| any_input_invalidated(state, &tx.spent_outpoints))
                .unwrap_or(false);
            if any_invalid_input {
                let Some(tx) = state.txs.get_mut(&txid) else {
                    continue;
                };
                if tx.state != TxState::Invalidated {
                    tx.invalidated_prior = Some(tx.state);
                    if tx.state == TxState::Confirmed {
                        // S3: a cascade-invalidated confirmed transaction
                        // keeps its displacement identity - its own
                        // confirmed block and THIS record's seq (the
                        // original displacement moment) - so the
                        // stale-evidence rule binds its promotion back to
                        // `Reorged` exactly like an explicit displacement.
                        // The restoring parent's later confirmation never
                        // overwrites these (rule (c) only sets the state).
                        tx.reorged_from_block = tx.confirmed_block;
                        tx.displaced_at_seq = Some(record_seq);
                        tx.confirmed_block = None;
                        // Degrade this transaction's confirmed-spent
                        // inputs to possibly-spent (the same claim
                        // preservation as an explicit reorg).
                        let spent = tx.spent_outpoints.clone();
                        for outpoint in spent {
                            if let Some(entry) = state.funding.get_mut(&outpoint) {
                                if matches!(entry.status, OutpointStatus::Spent) {
                                    entry.status = OutpointStatus::PossiblySpent;
                                }
                            }
                        }
                    }
                    tx.state = TxState::Invalidated;
                    changed = true;
                }
            } else if let Some(tx) = state.txs.get_mut(&txid) {
                if tx.state == TxState::Invalidated {
                    if let Some(prior) = tx.invalidated_prior.take() {
                        tx.state = if prior == TxState::Confirmed {
                            TxState::Reorged
                        } else {
                            prior
                        };
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
}

/// Validate one record against the current state. Everything a replay
/// re-checks, the runtime path checks before the record becomes durable:
/// the history-3 header contract, raw-bytes-to-txid consistency, weight/vbytes
/// remeasurement, fee recomputation from the funding ledger, decoded
/// inputs equal to the reserved set (M6), reservation exclusivity and
/// maturity, change index determinism, send-sequence monotonicity,
/// retry-send admission against the caps (H3), barrier semantics, the
/// lifecycle transition table, and the H1/H2 invalidation gates.
fn validate_record(
    state: &JournalState,
    record: &JournalRecord,
    allow_fixture_funding: bool,
) -> Result<(), JournalError> {
    if record.seq() != state.next_seq {
        return Err(integrity(format!(
            "seq {} does not continue the journal (expected {})",
            record.seq(),
            state.next_seq
        )));
    }
    if !state.header_seen && !matches!(record, JournalRecord::Header { .. }) {
        return Err(integrity(
            "journal must begin with the header record (seq 0)".to_string(),
        ));
    }
    match record {
        JournalRecord::Header {
            format_magic,
            history_version,
            protocol,
            network,
            key_mode,
            derivation_version,
            run_id,
            driver_id,
            run_seed_fingerprint,
            allocation_digest,
            caps,
            ..
        } => {
            if state.header_seen {
                return Err(integrity("duplicate header".to_string()));
            }
            if format_magic != FORMAT_MAGIC {
                return Err(integrity(format!(
                    "journal format magic {format_magic:?} is not {FORMAT_MAGIC:?} \
                     (pre-rework candidate journals are refused)"
                )));
            }
            if *history_version != SUPPORTED_HISTORY_VERSION {
                return Err(JournalError::UnsupportedHistoryVersion {
                    found: *history_version,
                    supported: SUPPORTED_HISTORY_VERSION,
                });
            }
            if protocol != PROTOCOL {
                return Err(integrity(format!(
                    "journal protocol {protocol:?} is not {PROTOCOL:?}"
                )));
            }
            if key_mode != KEY_MODE {
                return Err(JournalError::KeyModeMismatch {
                    stored: key_mode.clone(),
                    expected: KEY_MODE.to_string(),
                });
            }
            if *derivation_version != DERIVATION_VERSION {
                return Err(JournalError::DerivationVersionMismatch {
                    found: *derivation_version,
                    expected: DERIVATION_VERSION,
                });
            }
            validate_run_id(run_id).map_err(|e| integrity(format!("header run id: {e}")))?;
            validate_run_id(driver_id).map_err(|e| integrity(format!("header driver id: {e}")))?;
            caps.validate()?;
            if network.trim().is_empty() {
                return Err(integrity("header network is empty".to_string()));
            }
            if run_seed_fingerprint.trim().is_empty() {
                return Err(integrity(
                    "header run seed fingerprint is empty".to_string(),
                ));
            }
            // R6: the allocation digest is a SHA-256 hex string (64
            // lowercase hex chars); anything else is a corrupt header.
            if hex::decode(allocation_digest).map(|bytes| bytes.len()) != Ok(32) {
                return Err(integrity(
                    "header allocation digest is not a sha256 hex string".to_string(),
                ));
            }
        }
        // R2: a session record carries no command or funding semantics
        // of its own; the global seq and header checks above are its
        // whole validation (the commit arm applies the in-flight
        // degradation deterministically).
        JournalRecord::SessionStarted { .. } => {}
        JournalRecord::FundingObserved {
            outpoint,
            amount_sats,
            script_hex,
            chain,
            index,
            owner_role,
            source,
            context,
            ..
        } => {
            let outpoint = parse_outpoint(outpoint)?;
            if *amount_sats == 0 {
                return Err(integrity(
                    "funding observation with zero amount".to_string(),
                ));
            }
            parse_script(script_hex)?;
            // The v1 traffic profile is legacy-only: a funded script that
            // is not P2PKH could never be spent by this signer.
            if !parse_script(script_hex)?.is_p2pkh() {
                return Err(integrity(
                    "funding observation script is not P2PKH (v1 traffic profile is legacy-only)"
                        .to_string(),
                ));
            }
            if matches!(source, FundingSource::SyntheticFixture) && !allow_fixture_funding {
                return Err(JournalError::FixtureFundingRefused);
            }
            // Source/context pairing (H5).
            match (source, context) {
                (
                    FundingSource::BootstrapCoinbase,
                    FundingContext::CanonicalBlock {
                        block_hash,
                        coinbase: true,
                        ..
                    },
                )
                | (
                    FundingSource::CanonicalBlock,
                    FundingContext::CanonicalBlock {
                        block_hash,
                        coinbase: false,
                        ..
                    },
                ) => {
                    if BlockHash::from_str(block_hash).is_err() {
                        return Err(integrity(
                            "malformed block hash in funding context".to_string(),
                        ));
                    }
                }
                (FundingSource::OwnTransaction, FundingContext::OwnTransaction) => {}
                (FundingSource::SyntheticFixture, FundingContext::Fixture) => {}
                _ => {
                    return Err(JournalError::FundingNotVerifiable {
                        outpoint: outpoint_to_string(&outpoint),
                        reason: "source/context pairing is not admissible".to_string(),
                    });
                }
            }
            // Duplicate funding: only the canonical re-observation of an
            // invalidated entry (same identity, fresh block evidence) is
            // admissible (H5).
            if let Some(existing) = state.funding.get(&outpoint) {
                let revalidatable = matches!(existing.status, OutpointStatus::Invalidated { .. })
                    && matches!(
                        source,
                        FundingSource::BootstrapCoinbase | FundingSource::CanonicalBlock
                    )
                    && existing.owner == *owner_role
                    && existing.amount_sats == *amount_sats
                    && existing.chain == *chain
                    && existing.index == *index
                    && existing.script.to_hex_string() == *script_hex
                    && existing.source == *source;
                if !revalidatable {
                    return Err(JournalError::DuplicateFunding {
                        outpoint: outpoint_to_string(&outpoint),
                    });
                }
            }
            // Own-transaction funding requires the journaled creating
            // transaction to be confirmed, with a byte-exact decoded
            // vout match (H5).
            if *source == FundingSource::OwnTransaction {
                let creating = state.txs.get(&outpoint.txid).ok_or_else(|| {
                    JournalError::FundingNotVerifiable {
                        outpoint: outpoint_to_string(&outpoint),
                        reason: "creating transaction is not journaled".to_string(),
                    }
                })?;
                if creating.state != TxState::Confirmed {
                    return Err(JournalError::FundingNotVerifiable {
                        outpoint: outpoint_to_string(&outpoint),
                        reason: format!(
                            "creating transaction is in state {}, expected confirmed",
                            creating.state.as_str()
                        ),
                    });
                }
                let decoded = decode_raw_tx(&creating.raw_hex, &outpoint.txid.to_string())?;
                let vout = decoded.output.get(outpoint.vout as usize).ok_or_else(|| {
                    JournalError::FundingNotVerifiable {
                        outpoint: outpoint_to_string(&outpoint),
                        reason: "vout is out of range of the creating transaction".to_string(),
                    }
                })?;
                if vout.script_pubkey.to_hex_string() != *script_hex
                    || vout.value.to_sat() != *amount_sats
                {
                    return Err(JournalError::FundingNotVerifiable {
                        outpoint: outpoint_to_string(&outpoint),
                        reason: "decoded creating-tx output does not match the observation"
                            .to_string(),
                    });
                }
            }
        }
        JournalRecord::CommandBegin {
            command_id,
            request,
            ..
        } => {
            if request.construction_effect.is_some()
                && (request.category != PaymentCategory::UserPayment
                    || !matches!(request.payer_role, WalletRole::UserPayment { .. }))
            {
                return Err(integrity(
                    "acceptance construction requires a user-payment request",
                ));
            }
            if command_id.trim().is_empty() {
                return Err(integrity("empty command id".to_string()));
            }
            if state.commands.contains_key(command_id) {
                return Err(integrity(format!(
                    "duplicate command begin for {command_id}"
                )));
            }
            if request.destinations.is_empty() {
                return Err(integrity("command with no destinations".to_string()));
            }
            for destination in &request.destinations {
                if destination.amount_sats == 0 {
                    return Err(integrity("zero-amount destination".to_string()));
                }
                parse_script(&destination.script_hex)?;
            }
            if request.fee_rate_sats_vb == 0 {
                return Err(integrity("zero fee rate in command request".to_string()));
            }
        }
        JournalRecord::CommandConflict { command_id, .. } => {
            state.command(command_id)?;
        }
        JournalRecord::Reserved {
            command_id,
            outpoints,
            change_index,
            ..
        } => {
            let entry = state.command(command_id)?;
            if entry.phase != CommandPhase::Began {
                return Err(JournalError::IllegalTransition {
                    txid: command_id.clone(),
                    reason: format!(
                        "reserve on a command in phase {} (expected began)",
                        entry.phase.as_str()
                    ),
                });
            }
            if outpoints.is_empty() {
                return Err(integrity("reservation with no outpoints".to_string()));
            }
            let wallet_role = entry.wallet_role.clone();
            let expected_change_index = state
                .change_index_next
                .get(&wallet_role)
                .copied()
                .unwrap_or(0);
            if *change_index != expected_change_index {
                return Err(integrity(format!(
                    "change index {change_index} is not the deterministic next index \
                     {expected_change_index} for {wallet_role}"
                )));
            }
            let mut seen = std::collections::BTreeSet::new();
            for outpoint_str in outpoints {
                let outpoint = parse_outpoint(outpoint_str)?;
                if !seen.insert(outpoint) {
                    return Err(integrity(format!(
                        "duplicate outpoint {outpoint_str} in reservation"
                    )));
                }
                let funding =
                    state
                        .funding
                        .get(&outpoint)
                        .ok_or_else(|| JournalError::NotReservable {
                            outpoint: outpoint_str.clone(),
                            reason: "not present in the funding ledger".to_string(),
                        })?;
                match &funding.status {
                    OutpointStatus::Available => {}
                    OutpointStatus::Reserved { command_id } => {
                        return Err(JournalError::AlreadyReserved {
                            outpoint: outpoint_str.clone(),
                            command: command_id.clone(),
                        });
                    }
                    OutpointStatus::PossiblySpent => {
                        return Err(JournalError::NotReservable {
                            outpoint: outpoint_str.clone(),
                            reason: "possibly spent (reorg-displaced claim)".to_string(),
                        });
                    }
                    OutpointStatus::Spent => {
                        return Err(JournalError::NotReservable {
                            outpoint: outpoint_str.clone(),
                            reason: "already spent".to_string(),
                        });
                    }
                    OutpointStatus::Invalidated { .. } => {
                        return Err(JournalError::NotReservable {
                            outpoint: outpoint_str.clone(),
                            reason: "invalidated (reorg-displaced funding)".to_string(),
                        });
                    }
                }
                // H5: coinbase funding is reservable only at maturity of
                // the observed tip (fail closed without an observed tip).
                if coinbase_immature(state, funding) {
                    return Err(JournalError::NotReservable {
                        outpoint: outpoint_str.clone(),
                        reason: format!(
                            "coinbase funding not yet mature (needs {COINBASE_MATURITY} \
                             confirmations of the observed tip)"
                        ),
                    });
                }
                if funding.owner != wallet_role {
                    return Err(JournalError::OutpointOwnerMismatch {
                        outpoint: outpoint_str.clone(),
                        owner: funding.owner.name(),
                        wallet: wallet_role.name(),
                    });
                }
            }
        }
        JournalRecord::SignedTx {
            command_id,
            txid,
            raw_hex,
            vbytes,
            weight_wu,
            fee_sats,
            fee_rate_sats_vb,
            category,
            spent_outpoints,
            destinations,
            change,
            ..
        } => {
            let entry = state.command(command_id)?;
            if entry.phase != CommandPhase::Reserved {
                return Err(JournalError::IllegalTransition {
                    txid: command_id.clone(),
                    reason: format!(
                        "signed record on a command in phase {} (expected reserved)",
                        entry.phase.as_str()
                    ),
                });
            }
            let tx = decode_raw_tx(raw_hex, txid)?;
            let version_ok = match entry.request.construction_effect {
                Some(crate::construction::ConstructionEffect::VersionRejectionV1) => {
                    tx.version.0 == 3
                        && entry.request.category == PaymentCategory::UserPayment
                        && matches!(entry.request.payer_role, WalletRole::UserPayment { .. })
                }
                None => matches!(tx.version.0, 1 | 2),
            };
            if !version_ok {
                return Err(integrity(
                    "signed version does not match the immutable construction effect",
                ));
            }
            let measured_vbytes = tx.vsize() as u64;
            let measured_weight = tx.weight().to_wu();
            if measured_vbytes != *vbytes || measured_weight != *weight_wu {
                return Err(integrity(format!(
                    "recorded size ({vbytes} vB, {weight_wu} WU) does not match the \
                     signed bytes ({measured_vbytes} vB, {measured_weight} WU)"
                )));
            }
            let parsed_category =
                PaymentCategory::parse(category).map_err(|e| integrity(e.to_string()))?;
            if parsed_category != entry.category {
                return Err(integrity(format!(
                    "signed category {category} does not match the command category {}",
                    entry.category.as_str()
                )));
            }
            let mut parsed_spent = Vec::with_capacity(spent_outpoints.len());
            for outpoint_str in spent_outpoints {
                parsed_spent.push(parse_outpoint(outpoint_str)?);
            }
            let mut reserved = entry.reserved_outpoints.clone();
            let mut spent_sorted = parsed_spent.clone();
            reserved.sort();
            spent_sorted.sort();
            if spent_sorted != reserved {
                return Err(integrity(format!(
                    "signed inputs are not the reserved set for {command_id}"
                )));
            }
            // M6: the decoded inputs of the signed bytes must also be
            // exactly the reserved set (a record whose claimed inputs
            // match but whose bytes spend something else is a tamper).
            let decoded_inputs: Vec<OutPoint> =
                tx.input.iter().map(|input| input.previous_output).collect();
            let mut decoded_sorted = decoded_inputs.clone();
            decoded_sorted.sort();
            decoded_sorted.dedup();
            if decoded_sorted.len() != decoded_inputs.len() {
                return Err(integrity(
                    "signed transaction spends a duplicate input".to_string(),
                ));
            }
            if decoded_sorted != reserved {
                return Err(integrity(
                    "decoded signed inputs are not the reserved set".to_string(),
                ));
            }
            // An invalidated input can never back a new signed record.
            if any_input_invalidated(state, &parsed_spent) {
                return Err(JournalError::IllegalTransition {
                    txid: command_id.clone(),
                    reason: "signed record over an invalidated input".to_string(),
                });
            }
            // Fee recomputed from the funding ledger (the ledger is the
            // single source of input values).
            let inputs_total =
                checked_sum_sats(state.funding.iter().filter_map(|(outpoint, entry)| {
                    if parsed_spent.contains(outpoint) {
                        Some(entry.amount_sats)
                    } else {
                        None
                    }
                }))
                .map_err(|_| integrity("input sum overflows".to_string()))?;
            let outputs_total = checked_sum_sats(tx.output.iter().map(|o| o.value.to_sat()))
                .map_err(|_| integrity("output sum overflows".to_string()))?;
            let recomputed_fee = checked_sub_sats(inputs_total, outputs_total)
                .map_err(|_| integrity("outputs exceed inputs".to_string()))?;
            if recomputed_fee != *fee_sats {
                return Err(integrity(format!(
                    "recorded fee {fee_sats} != ledger-recomputed fee {recomputed_fee}"
                )));
            }
            if *fee_sats == 0 {
                return Err(integrity("zero fee".to_string()));
            }
            // The requested rate must match the command's request; the
            // realized rate (fee_sats / vbytes, both recorded above) is
            // re-derivable by any oracle and was band-checked by the
            // builder against the measured final size.
            if *fee_rate_sats_vb != entry.request.fee_rate_sats_vb {
                return Err(integrity(format!(
                    "signed fee rate {fee_rate_sats_vb} does not match the request rate {}",
                    entry.request.fee_rate_sats_vb
                )));
            }
            // Output shape: destinations in request order, then the single
            // change output if present (exactly what the builder assembles).
            let request_destinations = &entry.request.destinations;
            if destinations.len() != request_destinations.len() {
                return Err(integrity(format!(
                    "signed destination count {} != request count {}",
                    destinations.len(),
                    request_destinations.len()
                )));
            }
            for (signed_destination, request_destination) in
                destinations.iter().zip(request_destinations.iter())
            {
                if signed_destination != request_destination {
                    return Err(integrity(format!(
                        "signed destination does not match the request for {command_id}"
                    )));
                }
            }
            let expected_outputs = request_destinations.len() + usize::from(change.is_some());
            if tx.output.len() != expected_outputs {
                return Err(integrity(format!(
                    "signed tx has {} outputs, expected {expected_outputs}",
                    tx.output.len()
                )));
            }
            for (index, destination) in request_destinations.iter().enumerate() {
                let output = &tx.output[index];
                if output.script_pubkey.to_hex_string() != destination.script_hex
                    || output.value.to_sat() != destination.amount_sats
                {
                    return Err(integrity(format!(
                        "output {index} does not pay the exact requested destination"
                    )));
                }
            }
            if let Some(change) = change {
                let output = tx.output.last().expect("change output exists");
                if output.script_pubkey.to_hex_string() != change.script_hex
                    || output.value.to_sat() != change.amount_sats
                {
                    return Err(integrity(
                        "last output does not match the recorded change".to_string(),
                    ));
                }
                if change.amount_sats == 0 {
                    return Err(integrity("zero-amount change output".to_string()));
                }
            }
        }
        JournalRecord::CommandFinished { command_id, .. } => {
            let entry = state.command(command_id)?;
            if entry.phase == CommandPhase::Finished {
                return Err(integrity(format!("command {command_id} finished twice")));
            }
        }
        JournalRecord::InputsReleased { command_id, .. } => {
            let entry = state.command(command_id)?;
            if entry.phase != CommandPhase::Reserved {
                return Err(JournalError::IllegalTransition {
                    txid: command_id.clone(),
                    reason: format!(
                        "input release on a command in phase {} (only pre-signing failures \
                         may release; unknown never releases)",
                        entry.phase.as_str()
                    ),
                });
            }
        }
        JournalRecord::BroadcastSent {
            command_id,
            send_sequence,
            ..
        } => {
            let (tx, command) = state.tx_of_command(command_id)?;
            validate_send_funding(state, tx)?;
            if command.phase != CommandPhase::Signed {
                return Err(JournalError::IllegalTransition {
                    txid: command_id.clone(),
                    reason: format!(
                        "broadcast on a command in phase {} (expected signed)",
                        command.phase.as_str()
                    ),
                });
            }
            // H3: a normal broadcast leaves only the initial `Signed`
            // state or an eligible `Unknown`; a retry send is admitted
            // only while retry_sends < the cap. (`Reorged` has no edge:
            // re-entry after a reorg is exclusively `ReorgResubmit`.)
            match tx.state {
                TxState::Signed => {}
                TxState::Unknown => {
                    if tx.retry_sends >= state.caps.unknown_retry_sends {
                        return Err(JournalError::UnknownRetryCapExceeded {
                            cap: state.caps.unknown_retry_sends,
                            txid: tx.command_id.clone(),
                        });
                    }
                }
                other => {
                    return Err(JournalError::IllegalTransition {
                        txid: command_id.clone(),
                        reason: format!("broadcast from state {}", other.as_str()),
                    });
                }
            }
            // Defense in depth: an invalidated input can never leave the
            // process again (the cascade would have made the tx
            // `Invalidated`, which the state match above already refuses).
            if any_input_invalidated(state, &tx.spent_outpoints) {
                return Err(JournalError::IllegalTransition {
                    txid: command_id.clone(),
                    reason: "broadcast of a transaction with an invalidated input".to_string(),
                });
            }
            if state.barrier_active {
                return Err(JournalError::IllegalTransition {
                    txid: command_id.clone(),
                    reason: "broadcast after the pause barrier (sends must be resumed first)"
                        .to_string(),
                });
            }
            if *send_sequence != state.next_send_sequence {
                return Err(integrity(format!(
                    "send sequence {send_sequence} is not the monotonic next {}",
                    state.next_send_sequence
                )));
            }
        }
        JournalRecord::AcceptedByRpc { command_id, .. } => {
            let (tx, _) = state.tx_of_command(command_id)?;
            if !TxState::transition_legal(tx.state, TxState::AcceptedByRpc) {
                return Err(illegal(tx.state, TxState::AcceptedByRpc, command_id));
            }
        }
        JournalRecord::ObservedInMempool { command_id, .. } => {
            let (tx, _) = state.tx_of_command(command_id)?;
            if !TxState::transition_legal(tx.state, TxState::ObservedInMempool) {
                return Err(illegal(tx.state, TxState::ObservedInMempool, command_id));
            }
        }
        JournalRecord::Confirmed {
            command_id,
            block_hash,
            height,
            ..
        } => {
            let (tx, _) = state.tx_of_command(command_id)?;
            // An invalidated input can never back a confirmation.
            if any_input_invalidated(state, &tx.spent_outpoints) {
                return Err(JournalError::IllegalTransition {
                    txid: command_id.clone(),
                    reason: "confirmation of a transaction with an invalidated input".to_string(),
                });
            }
            if tx.state == TxState::Confirmed {
                // Idempotent re-confirmation must name the same block.
                if let Some((confirmed_hash, confirmed_height)) = tx.confirmed_block {
                    if confirmed_hash.to_string() != *block_hash || confirmed_height != *height {
                        return Err(JournalError::IllegalTransition {
                            txid: command_id.clone(),
                            reason: format!(
                                "re-confirmation names block {block_hash}/{height}, but the \
                                 transaction is confirmed in {confirmed_hash}/{confirmed_height}"
                            ),
                        });
                    }
                }
            } else if !TxState::transition_legal(tx.state, TxState::Confirmed) {
                return Err(illegal(tx.state, TxState::Confirmed, command_id));
            }
            // R12/S3: a canonical re-inclusion from `Reorged` (the new
            // chain re-mined the displaced transaction) must not name the
            // displaced block itself on STALE evidence - the same block
            // cannot be both the displaced and the re-including evidence
            // while the displacement is the latest chain observation. A
            // `ChainObserved` committed AFTER the displacement (the chain
            // reorged back and the block is canonical again) makes the
            // same-hash confirmation admissible as canonical evidence;
            // the explicit `record_reorged` path with no newer chain
            // observation stays refused. The original displacement seq
            // binds here even for a cascade-invalidated descendant
            // promoted back to `Reorged` (the parent's restoring
            // confirmation never overwrites it).
            if tx.state == TxState::Reorged {
                if let Some((displaced_hash, _)) = tx.reorged_from_block {
                    if displaced_hash.to_string() == *block_hash {
                        let fresh_chain_evidence = state
                            .last_chain_observed_seq
                            .zip(tx.displaced_at_seq)
                            .is_some_and(|(last_observed, displaced_at)| {
                                last_observed > displaced_at
                            });
                        if !fresh_chain_evidence {
                            return Err(JournalError::IllegalTransition {
                                txid: command_id.clone(),
                                reason: format!(
                                    "re-inclusion names the displaced block {block_hash} on \
                                     stale evidence: no chain observation is newer than the \
                                     displacement"
                                ),
                            });
                        }
                    }
                }
            }
        }
        JournalRecord::Rejected { command_id, .. } => {
            let (tx, _) = state.tx_of_command(command_id)?;
            if !TxState::transition_legal(tx.state, TxState::Rejected) {
                return Err(illegal(tx.state, TxState::Rejected, command_id));
            }
        }
        JournalRecord::UnknownOutcome {
            command_id,
            attempt,
            ..
        } => {
            let (tx, _) = state.tx_of_command(command_id)?;
            if !TxState::transition_legal(tx.state, TxState::Unknown) {
                return Err(illegal(tx.state, TxState::Unknown, command_id));
            }
            if *attempt != tx.unknown_retries + 1 {
                return Err(integrity(format!(
                    "unknown attempt {attempt} is not the next retry after {}",
                    tx.unknown_retries
                )));
            }
            // H3: recording unknown outcomes is never capped; the caps
            // govern retry-send admission only.
        }
        JournalRecord::Conflicted { command_id, .. } => {
            let (tx, _) = state.tx_of_command(command_id)?;
            if !TxState::transition_legal(tx.state, TxState::Conflicted) {
                return Err(illegal(tx.state, TxState::Conflicted, command_id));
            }
        }
        JournalRecord::Reorged {
            command_id,
            from_block_hash,
            ..
        } => {
            let (tx, _) = state.tx_of_command(command_id)?;
            if !TxState::transition_legal(tx.state, TxState::Reorged) {
                return Err(illegal(tx.state, TxState::Reorged, command_id));
            }
            if let Some((confirmed_hash, _)) = tx.confirmed_block {
                if confirmed_hash.to_string() != *from_block_hash {
                    return Err(integrity(
                        "reorg names a block the tx was not confirmed in".to_string(),
                    ));
                }
            }
        }
        JournalRecord::ReorgResubmit {
            command_id,
            proven_unspent,
            send_sequence,
            ..
        } => {
            let (tx, _) = state.tx_of_command(command_id)?;
            validate_send_funding(state, tx)?;
            if tx.state != TxState::Reorged {
                return Err(JournalError::ReorgRetryNotEligible {
                    txid: command_id.clone(),
                    reason: format!("state is {}, expected reorged", tx.state.as_str()),
                });
            }
            if tx.reorg_resubmits + 1 > state.caps.reorg_resubmits {
                return Err(JournalError::ReorgResubmitCapExceeded {
                    cap: state.caps.reorg_resubmits,
                    txid: command_id.clone(),
                });
            }
            if any_input_invalidated(state, &tx.spent_outpoints) {
                return Err(JournalError::ReorgRetryNotEligible {
                    txid: command_id.clone(),
                    reason: "an original input is invalidated by the reorg".to_string(),
                });
            }
            let mut proven = std::collections::BTreeSet::new();
            for outpoint_str in proven_unspent {
                proven.insert(parse_outpoint(outpoint_str)?);
            }
            for outpoint in &tx.spent_outpoints {
                if !proven.contains(outpoint) {
                    return Err(JournalError::ReorgRetryNotEligible {
                        txid: command_id.clone(),
                        reason: format!(
                            "original input {} was not proven unspent from canonical blocks",
                            outpoint_to_string(outpoint)
                        ),
                    });
                }
                match state.funding.get(outpoint).map(|entry| &entry.status) {
                    Some(OutpointStatus::Spent) => {
                        return Err(JournalError::ReorgRetryNotEligible {
                            txid: command_id.clone(),
                            reason: format!(
                                "original input {} is spent by the conflicting chain",
                                outpoint_to_string(outpoint)
                            ),
                        });
                    }
                    Some(OutpointStatus::PossiblySpent) | Some(OutpointStatus::Available) => {}
                    Some(OutpointStatus::Invalidated { .. })
                    | Some(OutpointStatus::Reserved { .. })
                    | None => {
                        return Err(JournalError::ReorgRetryNotEligible {
                            txid: command_id.clone(),
                            reason: format!(
                                "original input {} is in an unexpected ledger state",
                                outpoint_to_string(outpoint)
                            ),
                        });
                    }
                }
            }
            if state.barrier_active {
                return Err(JournalError::IllegalTransition {
                    txid: command_id.clone(),
                    reason: "reorg resubmission after the pause barrier".to_string(),
                });
            }
            if *send_sequence != state.next_send_sequence {
                return Err(integrity(format!(
                    "resubmit send sequence {send_sequence} is not the monotonic next {}",
                    state.next_send_sequence
                )));
            }
        }
        JournalRecord::ChainRollback { evidence, .. } => validate_rollback(state, evidence)?,
        JournalRecord::ChainObserved {
            tip_block_hash,
            tip_height,
            displaced_block_hashes,
            ..
        } => {
            let tip_hash = BlockHash::from_str(tip_block_hash)
                .map_err(|_| integrity("malformed tip block hash".to_string()))?;
            // The tip never moves backwards (fail closed).
            if let Some((previous_height, _)) = state.tip {
                if *tip_height < previous_height {
                    return Err(integrity(format!(
                        "observed tip height {tip_height} is below the previous tip \
                         {previous_height}"
                    )));
                }
            }
            let mut seen = std::collections::BTreeSet::new();
            for hash in displaced_block_hashes {
                let parsed = BlockHash::from_str(hash)
                    .map_err(|_| integrity("malformed displaced block hash".to_string()))?;
                if parsed == tip_hash {
                    return Err(integrity(
                        "displaced block list contains the observed tip".to_string(),
                    ));
                }
                if !seen.insert(parsed) {
                    return Err(integrity("duplicate displaced block hash".to_string()));
                }
            }
        }
        JournalRecord::SendBarrier { send_sequence, .. } => {
            if state.barrier_active {
                return Err(JournalError::IllegalTransition {
                    txid: "send-barrier".to_string(),
                    reason: "barrier recorded while sends are already paused".to_string(),
                });
            }
            if *send_sequence != state.next_send_sequence.saturating_sub(1) {
                return Err(integrity(
                    "barrier send sequence is not the last used sequence".to_string(),
                ));
            }
        }
        JournalRecord::SendResumed { .. } => {
            if !state.barrier_active {
                return Err(JournalError::IllegalTransition {
                    txid: "send-resume".to_string(),
                    reason: "resume recorded while sends are not paused".to_string(),
                });
            }
        }
    }
    Ok(())
}

fn illegal(from: TxState, to: TxState, command_id: &str) -> JournalError {
    JournalError::IllegalTransition {
        txid: command_id.to_string(),
        reason: format!("{} -> {}", from.as_str(), to.as_str()),
    }
}

/// Commit a validated record to the in-memory state. Infallible by
/// construction: every check lives in [`validate_record`]. Every commit
/// finishes by reconciling the derived invalidation cascade (H2).
fn commit_record(state: &mut JournalState, record: JournalRecord) {
    // Only after proof validation and durable write: reuse the exact ordinary
    // displacement/cascade implementation, without an extra record or seq.
    let record = match record {
        JournalRecord::ChainRollback {
            seq,
            evidence,
            observed_unix_ms,
        } => JournalRecord::ChainObserved {
            seq,
            tip_block_hash: evidence.new_anchor.block_hash.to_string(),
            tip_height: evidence.new_anchor.height,
            displaced_block_hashes: evidence
                .displaced_block_hashes
                .iter()
                .map(ToString::to_string)
                .collect(),
            observed_unix_ms,
        },
        record => record,
    };
    state.next_seq += 1;
    // S3: the committing record's seq is the displacement stamp for any
    // transaction this commit displaces (directly or through the
    // cascade below) - derived from the record stream, so replay
    // reproduces the identical stamps.
    let record_seq = record.seq();
    match record {
        JournalRecord::Header {
            format_magic,
            history_version,
            protocol,
            network,
            key_mode,
            derivation_version,
            run_id,
            driver_id,
            run_seed_fingerprint,
            allocation_digest,
            caps,
            created_unix_ms,
            ..
        } => {
            state.header_seen = true;
            state.caps = caps;
            state.header_facts = Some(HeaderFacts {
                format_magic,
                history_version,
                protocol,
                network,
                key_mode,
                derivation_version,
                run_id,
                driver_id,
                run_seed_fingerprint,
                allocation_digest,
                caps,
                created_unix_ms,
            });
        }
        JournalRecord::SessionStarted { .. } => {
            // R2: every still-in-flight send (a transaction whose outcome
            // was never journaled before the process went away) becomes
            // unknown: the fate of the pre-restart send is not accounted
            // for by any record in this journal. `retry_sends` is
            // deliberately untouched - a process restart is not a retry
            // send. Already-`Unknown` transactions are not moved again, so
            // a crash-looping reopen cannot inflate the attempt counter.
            for tx in state.txs.values_mut() {
                if tx.state == TxState::BroadcastSent {
                    tx.state = TxState::Unknown;
                    tx.unknown_retries += 1;
                    tx.ever_unknown = true;
                }
            }
        }
        JournalRecord::FundingObserved {
            outpoint,
            amount_sats,
            script_hex,
            chain,
            index,
            owner_role,
            source,
            context,
            ..
        } => {
            let outpoint = parse_outpoint(&outpoint).expect("validated");
            let script = parse_script(&script_hex).expect("validated");
            let parsed_context = match &context {
                FundingContext::CanonicalBlock {
                    block_hash,
                    height,
                    coinbase,
                } => Some((
                    BlockHash::from_str(block_hash).expect("validated"),
                    *height,
                    *coinbase,
                )),
                _ => None,
            };
            if let Some(entry) = state.funding.get_mut(&outpoint) {
                // Canonical re-observation of an invalidated entry (H5):
                // refresh the block evidence and restore conservatively.
                entry.context = parsed_context;
                if let OutpointStatus::Invalidated { prior } = entry.status.clone() {
                    entry.status = match *prior {
                        OutpointStatus::Spent => OutpointStatus::PossiblySpent,
                        prior => prior,
                    };
                }
            } else {
                *state
                    .receive_index_next
                    .entry(owner_role.clone())
                    .or_insert(0) += 1;
                state.funding.insert(
                    outpoint,
                    OutpointEntry {
                        amount_sats,
                        script,
                        chain,
                        index,
                        owner: owner_role,
                        source,
                        context: parsed_context,
                        status: OutpointStatus::Available,
                    },
                );
            }
        }
        JournalRecord::CommandBegin {
            command_id,
            request,
            ..
        } => {
            let payload_digest = request_record_digest(&request);
            let category = request.category.clone();
            let wallet_role = request.payer_role.clone();
            state.commands.insert(
                command_id,
                CommandEntry {
                    payload_digest,
                    category,
                    wallet_role,
                    request,
                    phase: CommandPhase::Began,
                    txid: None,
                    reserved_outpoints: Vec::new(),
                    change_index: None,
                    outcome: None,
                },
            );
        }
        JournalRecord::CommandConflict { .. } => {}
        JournalRecord::Reserved {
            command_id,
            outpoints,
            change_index,
            ..
        } => {
            let wallet_role = state
                .commands
                .get(&command_id)
                .expect("validated")
                .wallet_role
                .clone();
            *state.change_index_next.entry(wallet_role).or_insert(0) += 1;
            let mut reserved = Vec::with_capacity(outpoints.len());
            for outpoint_str in &outpoints {
                let outpoint = parse_outpoint(outpoint_str).expect("validated");
                if let Some(entry) = state.funding.get_mut(&outpoint) {
                    entry.status = OutpointStatus::Reserved {
                        command_id: command_id.clone(),
                    };
                }
                reserved.push(outpoint);
            }
            if let Some(entry) = state.commands.get_mut(&command_id) {
                entry.phase = CommandPhase::Reserved;
                entry.reserved_outpoints = reserved;
                entry.change_index = Some(change_index);
            }
        }
        JournalRecord::SignedTx {
            command_id,
            txid,
            raw_hex,
            vbytes,
            weight_wu,
            fee_sats,
            fee_rate_sats_vb,
            category,
            spent_outpoints,
            change,
            ..
        } => {
            let txid = Txid::from_str(&txid).expect("validated");
            let mut spent = Vec::with_capacity(spent_outpoints.len());
            for outpoint_str in &spent_outpoints {
                spent.push(parse_outpoint(outpoint_str).expect("validated"));
            }
            let category = PaymentCategory::parse(&category).expect("validated");
            if let Some(entry) = state.commands.get_mut(&command_id) {
                entry.phase = CommandPhase::Signed;
                entry.txid = Some(txid);
            }
            state.txs.insert(
                txid,
                TxEntry {
                    command_id,
                    state: TxState::Signed,
                    raw_hex,
                    vbytes,
                    weight_wu,
                    fee_sats,
                    fee_rate_sats_vb,
                    category,
                    spent_outpoints: spent,
                    change_index: change.map(|change| change.index),
                    unknown_retries: 0,
                    retry_sends: 0,
                    reorg_resubmits: 0,
                    ever_unknown: false,
                    ever_accepted: false,
                    invalidated_prior: None,
                    confirmed_block: None,
                    reorged_from_block: None,
                    displaced_at_seq: None,
                    last_send: None,
                },
            );
        }
        JournalRecord::CommandFinished {
            command_id,
            outcome,
            ..
        } => {
            if let Some(entry) = state.commands.get_mut(&command_id) {
                entry.phase = CommandPhase::Finished;
                entry.outcome = Some(outcome);
            }
        }
        JournalRecord::InputsReleased { command_id, .. } => {
            let reserved = state
                .commands
                .get(&command_id)
                .map(|entry| entry.reserved_outpoints.clone())
                .unwrap_or_default();
            for outpoint in reserved {
                if let Some(entry) = state.funding.get_mut(&outpoint) {
                    match entry.status.clone() {
                        OutpointStatus::Reserved { .. } => {
                            entry.status = OutpointStatus::Available;
                        }
                        OutpointStatus::Invalidated { prior } => {
                            // The reservation is dropped, but the reorg
                            // evidence is preserved: remember
                            // availability, not the dead reservation.
                            entry.status = OutpointStatus::Invalidated {
                                prior: Box::new(match *prior {
                                    OutpointStatus::Reserved { .. } => OutpointStatus::Available,
                                    prior => prior,
                                }),
                            };
                        }
                        _ => {}
                    }
                }
            }
            if let Some(entry) = state.commands.get_mut(&command_id) {
                // The command is finished by the release.
                entry.phase = CommandPhase::Finished;
                entry.outcome = Some("failed_construction".to_string());
            }
        }
        JournalRecord::BroadcastSent {
            txid,
            send_sequence,
            ..
        } => {
            let txid = Txid::from_str(&txid).expect("validated");
            state.last_broadcast_txid = Some(txid);
            if let Some(tx) = state.txs.get_mut(&txid) {
                tx.last_send = Some((send_sequence, record_seq));
                // H3: a send committed from `unknown` is a retry send.
                let was_retry = tx.state == TxState::Unknown;
                tx.state = TxState::BroadcastSent;
                if was_retry {
                    tx.retry_sends += 1;
                }
            }
            state.next_send_sequence += 1;
        }
        JournalRecord::AcceptedByRpc { command_id, .. } => {
            let txid = state
                .commands
                .get(&command_id)
                .and_then(|entry| entry.txid)
                .expect("validated");
            if let Some(tx) = state.txs.get_mut(&txid) {
                tx.state = TxState::AcceptedByRpc;
                // H1: durable history - an RPC acceptance means at least
                // one node holds the bytes; a later rejection can never
                // release the inputs.
                tx.ever_accepted = true;
            }
        }
        JournalRecord::ObservedInMempool { command_id, .. } => {
            let txid = state
                .commands
                .get(&command_id)
                .and_then(|entry| entry.txid)
                .expect("validated");
            if let Some(tx) = state.txs.get_mut(&txid) {
                tx.state = TxState::ObservedInMempool;
            }
        }
        JournalRecord::Confirmed {
            command_id,
            block_hash,
            height,
            ..
        } => {
            let txid = state
                .commands
                .get(&command_id)
                .and_then(|entry| entry.txid)
                .expect("validated");
            let block_hash = BlockHash::from_str(&block_hash).expect("validated");
            if let Some(tx) = state.txs.get_mut(&txid) {
                tx.state = TxState::Confirmed;
                tx.confirmed_block = Some((block_hash, height));
                tx.reorged_from_block = None;
                tx.displaced_at_seq = None;
                let spent = tx.spent_outpoints.clone();
                for outpoint in spent {
                    if let Some(entry) = state.funding.get_mut(&outpoint) {
                        entry.status = OutpointStatus::Spent;
                    }
                }
            }
        }
        JournalRecord::Rejected { command_id, .. } => {
            let txid = state
                .commands
                .get(&command_id)
                .and_then(|entry| entry.txid)
                .expect("validated");
            if let Some(tx) = state.txs.get_mut(&txid) {
                tx.state = TxState::Rejected;
                // H1: release on rejection only if the payment was never
                // uncertain (never unknown, never RPC-accepted, never
                // reorg-resubmitted). The durable history decides, not
                // the current state: a crash-replayed journal reconstructs
                // the same flags and takes the same decision.
                let ever_uncertain = tx.ever_unknown || tx.ever_accepted || tx.reorg_resubmits > 0;
                let spent = tx.spent_outpoints.clone();
                for outpoint in spent {
                    if let Some(entry) = state.funding.get_mut(&outpoint) {
                        if matches!(entry.status, OutpointStatus::Reserved { .. }) {
                            entry.status = if ever_uncertain {
                                OutpointStatus::PossiblySpent
                            } else {
                                OutpointStatus::Available
                            };
                        }
                    }
                }
            }
        }
        JournalRecord::UnknownOutcome { command_id, .. } => {
            let txid = state
                .commands
                .get(&command_id)
                .and_then(|entry| entry.txid)
                .expect("validated");
            if let Some(tx) = state.txs.get_mut(&txid) {
                tx.state = TxState::Unknown;
                tx.unknown_retries += 1;
                // H1: durable history - one unknown send can never be
                // released by a later rejection.
                tx.ever_unknown = true;
            }
        }
        JournalRecord::Conflicted { command_id, .. } => {
            let txid = state
                .commands
                .get(&command_id)
                .and_then(|entry| entry.txid)
                .expect("validated");
            if let Some(tx) = state.txs.get_mut(&txid) {
                tx.state = TxState::Conflicted;
                let spent = tx.spent_outpoints.clone();
                for outpoint in spent {
                    if let Some(entry) = state.funding.get_mut(&outpoint) {
                        // The conflicting spend consumed the coin.
                        entry.status = OutpointStatus::Spent;
                    }
                }
            }
        }
        JournalRecord::Reorged {
            command_id,
            from_block_hash,
            height,
            ..
        } => {
            let txid = state
                .commands
                .get(&command_id)
                .and_then(|entry| entry.txid)
                .expect("validated");
            if let Some(tx) = state.txs.get_mut(&txid) {
                tx.state = TxState::Reorged;
                tx.confirmed_block = None;
                // R12/S3: remember the displaced block and the
                // displacement moment (this record's seq) so a later
                // canonical re-inclusion can be refused if it names the
                // block on stale evidence (no chain observation newer
                // than this displacement).
                tx.reorged_from_block = Some((
                    BlockHash::from_str(&from_block_hash).expect("validated"),
                    height,
                ));
                tx.displaced_at_seq = Some(record_seq);
                let spent = tx.spent_outpoints.clone();
                for outpoint in spent {
                    if let Some(entry) = state.funding.get_mut(&outpoint) {
                        if matches!(entry.status, OutpointStatus::Spent) {
                            // Preserve the possibly-spent claim.
                            entry.status = OutpointStatus::PossiblySpent;
                        }
                    }
                }
            }
        }
        JournalRecord::ReorgResubmit {
            command_id,
            send_sequence,
            ..
        } => {
            let txid = state
                .commands
                .get(&command_id)
                .and_then(|entry| entry.txid)
                .expect("validated");
            if let Some(tx) = state.txs.get_mut(&txid) {
                tx.last_send = Some((send_sequence, record_seq));
                tx.state = TxState::BroadcastSent;
                tx.reorg_resubmits += 1;
            }
            state.next_send_sequence += 1;
        }
        JournalRecord::ChainRollback { .. } => unreachable!("normalized above"),
        JournalRecord::ChainObserved {
            tip_block_hash,
            tip_height,
            displaced_block_hashes,
            ..
        } => {
            let tip_hash = BlockHash::from_str(&tip_block_hash).expect("validated");
            state.tip = Some((tip_height, tip_hash));
            // S3: this record is now the latest chain evidence - the
            // recency witness for the displaced-block refusal.
            state.last_chain_observed_seq = Some(record_seq);
            let displaced: Vec<BlockHash> = displaced_block_hashes
                .iter()
                .map(|hash| BlockHash::from_str(hash).expect("validated"))
                .collect();
            // Own confirmed transactions in displaced blocks take the
            // reorg transition (identical to the explicit Reorged record).
            let txids: Vec<Txid> = state.txs.keys().copied().collect();
            for txid in txids {
                let confirmed = state.txs.get(&txid).and_then(|tx| tx.confirmed_block);
                let in_displaced = confirmed
                    .map(|(hash, _)| displaced.contains(&hash))
                    .unwrap_or(false);
                if in_displaced {
                    if let Some(tx) = state.txs.get_mut(&txid) {
                        tx.state = TxState::Reorged;
                        tx.confirmed_block = None;
                        // R12/S3: the displaced block identity and the
                        // displacement moment (this record's seq - see
                        // the explicit `Reorged` commit arm). Both
                        // survive a later cascade `Invalidated` and the
                        // promotion back to `Reorged` unchanged.
                        tx.reorged_from_block = confirmed;
                        tx.displaced_at_seq = Some(record_seq);
                        let spent = tx.spent_outpoints.clone();
                        for outpoint in spent {
                            if let Some(entry) = state.funding.get_mut(&outpoint) {
                                if matches!(entry.status, OutpointStatus::Spent) {
                                    entry.status = OutpointStatus::PossiblySpent;
                                }
                            }
                        }
                    }
                }
            }
            // Canonical funding in displaced blocks is invalidated (H2).
            let outpoints: Vec<OutPoint> = state.funding.keys().copied().collect();
            for outpoint in outpoints {
                let in_displaced = state
                    .funding
                    .get(&outpoint)
                    .and_then(|entry| entry.context)
                    .map(|(hash, _, _)| displaced.contains(&hash))
                    .unwrap_or(false);
                if in_displaced {
                    if let Some(entry) = state.funding.get_mut(&outpoint) {
                        if !matches!(entry.status, OutpointStatus::Invalidated { .. }) {
                            entry.status = OutpointStatus::Invalidated {
                                prior: Box::new(entry.status.clone()),
                            };
                        }
                    }
                }
            }
            // The derived cascade (own-tx outputs of the reorged txs and
            // every descendant spending invalidated funding) runs below,
            // at the end of every commit.
        }
        JournalRecord::SendBarrier { .. } => {
            state.barrier_active = true;
        }
        JournalRecord::SendResumed { .. } => {
            state.barrier_active = false;
        }
    }
    cascade_invalidation(state, record_seq);
}

/// Append one record with full durability, or roll it back (M4/R1):
///
/// - the record is serialized, written, flushed and `sync_data`d, and the
///   proven-durable length advances only after the sync succeeds;
/// - an injected fault (tests only) exercises the failure paths: a
///   partial write or an unsynced full write is rolled back with
///   `set_len(durable_len)` + `sync_data` and the fault is consumed;
/// - **every** append failure - an injected fault or a REAL `write_all` /
///   `flush` / `sync_data` error (ENOSPC, EDQUOT, EFBIG under
///   `RLIMIT_FSIZE`, ...) - takes the SAME recovery path
///   ([`rollback_or_poison`]): the file is provably rolled back to the
///   durable length, or the handle is **poisoned** and every further
///   mutation fails closed with [`JournalError::Poisoned`]. A failed
///   append never leaves a fragment behind and never advances the
///   durable length, so the next append cannot corrupt the file and a
///   reopen is always consistent.
fn write_line(state: &mut JournalState, record: &JournalRecord) -> Result<(), JournalError> {
    if state.poisoned {
        return Err(JournalError::Poisoned);
    }
    let mut line = serde_json::to_vec(record)
        .map_err(|e| JournalError::Io(format!("serialize journal record: {e}")))?;
    line.push(b'\n');
    let len_before = state.durable_len;
    // The append stage: the injected faults (tests only) decide WHERE the
    // append fails; the production path writes the whole line and proves
    // durability. Every resulting failure is recovered identically below.
    let append: Result<(), JournalError> = match state.write_fault.take() {
        Some(WriteFault::PartialWrite) => {
            let half = (line.len() / 2).max(1);
            state
                .file
                .write_all(&line[..half])
                .map_err(|e| JournalError::Io(format!("append journal record: {e}")))?;
            Err(JournalError::Io(
                "injected partial write fault: the record was not appended".to_string(),
            ))
        }
        Some(WriteFault::SyncFailure) => {
            // The line is written but its durability sync is skipped: it
            // lives only in the page cache.
            state
                .file
                .write_all(&line)
                .map_err(|e| JournalError::Io(format!("append journal record: {e}")))?;
            Err(JournalError::Io(
                "injected sync failure: the record was not made durable".to_string(),
            ))
        }
        Some(WriteFault::RollbackFailure) => {
            // The append writes a prefix and "fails"; the recovery below
            // is the injected failure point, so re-arm the fault for
            // `rollback_or_poison` (which consumes it).
            let half = (line.len() / 2).max(1);
            let _ = state.file.write_all(&line[..half]);
            state.write_fault = Some(WriteFault::RollbackFailure);
            Err(JournalError::Io("injected rollback failure".to_string()))
        }
        None => state
            .file
            .write_all(&line)
            .and_then(|_| state.file.flush())
            .and_then(|_| state.file.sync_data())
            .map_err(|e| JournalError::Io(format!("append journal record: {e}"))),
    };
    match append {
        Ok(()) => {
            state.durable_len += line.len() as u64;
            Ok(())
        }
        Err(failure) => {
            // R1: the shared recovery path, identical for real faults and
            // injected faults. Roll back to the proven-durable length or
            // poison the handle (the `?` propagates `Poisoned`).
            rollback_or_poison(state, len_before)?;
            Err(JournalError::Io(format!(
                "{failure}; the record was rolled back to {len_before} durable bytes \
                 and not appended"
            )))
        }
    }
}

/// Roll the file back to `len_before` durable bytes (M4). On success the
/// durable length is resynced; on failure the handle is poisoned. The
/// injected rollback failure (tests only) exercises the poison arm of
/// this shared recovery path.
fn rollback_or_poison(state: &mut JournalState, len_before: u64) -> Result<(), JournalError> {
    let rollback = if matches!(state.write_fault, Some(WriteFault::RollbackFailure)) {
        state.write_fault = None;
        Err(std::io::Error::from(std::io::ErrorKind::Other))
    } else {
        state
            .file
            .set_len(len_before)
            .and_then(|_| state.file.sync_data())
    };
    match rollback {
        Ok(()) => {
            state.durable_len = len_before;
            Ok(())
        }
        Err(_) => {
            state.poisoned = true;
            Err(JournalError::Poisoned)
        }
    }
}
