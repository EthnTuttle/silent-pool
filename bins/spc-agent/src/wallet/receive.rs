//! Receive-path types and the receive ledger (SPEC 11.5 "Receive",
//! plan 4.4).
//!
//! AP-3a scope: the **persistence and validation** layer. The live
//! mailbox/status RPCs (the validated `spc_ark_client` prototype's
//! `read_arkoor_mailbox` / `query_vtxo_status` / `claim_vtxos`, driven
//! with `u_i` against a stock Ark server) sit behind the
//! [`ReceiveBackend`] trait and are wired by AP-3b when the chain scan
//! and server transport land. What is enforced **here and now**:
//!
//! - a VTXO is recorded only against a `U` the SP keystore owns, and the
//!   stored secret must actually derive that `U` (ownership, fail closed);
//! - expiry comes from the VTXO's actual ancestry or a server receipt -
//!   never from the pool's finding height. An unreported expiry stays
//!   `expiry_unknown`: not refreshed, not offboarded, and never assigned
//!   an invented `H_f + N` deadline (plan 4.4);
//! - records are idempotent per `vtxo_id` and conflict-free: the same id
//!   with different fields is refused, so a restart cannot double-claim
//!   or rewrite history;
//! - the received sum reconciles against the pool-reported `ark_due`;
//!   a mismatch alarms (status `last_error` + reconciliation state).
//!
//! Honest scope labels (Fable rework, should-fix 8) - what the tests
//! here do **not** prove:
//!
//! - the committed fixture and the scenario tests drive this ledger by
//!   **key derivation and ledger replay**, not by real VTXO ancestry
//!   parsing, a live mailbox read, or stock bark VTXO recovery. The
//!   fixture's records are **synthetic**: deterministic derivations
//!   with explicitly provided expiry constants that are unrelated to
//!   the finding height;
//! - `VtxoState::Spendable` in an [`ObservedVtxo`] is
//!   **caller-asserted** (the observation's claim, faithfully
//!   persisted). Nothing here verifies spendability against the chain
//!   or a server, so no caller-asserted `Spendable` becomes a proved
//!   live balance. Live verification is AP-3b's network backend -
//!   deliberately not expanded into here.

use bitcoin::secp256k1::{PublicKey, Secp256k1};

use crate::wallet::state::{WalletStateError, WalletStore};

/// A VTXO's expiry, with its provenance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expiry {
    /// The server did not report an expiry. Not a deadline. The record is
    /// not refreshed, not offboarded, and never given an invented height.
    Unknown,
    /// An actual expiry, from the VTXO's own ancestry (the round/funding
    /// transaction it commits to) or a server receipt.
    At { height: u32, source: ExpirySource },
}

impl Expiry {
    pub fn height(&self) -> Option<u32> {
        match self {
            Expiry::Unknown => None,
            Expiry::At { height, .. } => Some(*height),
        }
    }

    pub fn source(&self) -> Option<ExpirySource> {
        match self {
            Expiry::Unknown => None,
            Expiry::At { source, .. } => Some(*source),
        }
    }
}

/// Where a recorded expiry came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExpirySource {
    /// Computed from the VTXO's actual on-chain ancestry.
    VtxoAncestry,
    /// Reported by the Ark server (receipt / `GetVtxoStatus`).
    ServerReport,
}

impl ExpirySource {
    pub fn as_str(&self) -> &'static str {
        match self {
            ExpirySource::VtxoAncestry => "vtxo_ancestry",
            ExpirySource::ServerReport => "server_report",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "vtxo_ancestry" => Some(ExpirySource::VtxoAncestry),
            "server_report" => Some(ExpirySource::ServerReport),
            _ => None,
        }
    }
}

/// Lifecycle state of an owned VTXO record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VtxoState {
    /// Observed in a mailbox / notice, ownership proven, not yet
    /// validated against the server.
    Pending,
    /// Validated unspent by the server (`GetVtxoStatus` with the `u_i`
    /// attestation).
    Spendable,
    /// Spent (refreshed, offboarded or otherwise consumed).
    Spent,
}

impl VtxoState {
    pub fn as_str(&self) -> &'static str {
        match self {
            VtxoState::Pending => "pending",
            VtxoState::Spendable => "spendable",
            VtxoState::Spent => "spent",
        }
    }

    /// Lifecycle rank: transitions are forward-only.
    pub fn rank(&self) -> u8 {
        match self {
            VtxoState::Pending => 0,
            VtxoState::Spendable => 1,
            VtxoState::Spent => 2,
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(VtxoState::Pending),
            "spendable" => Some(VtxoState::Spendable),
            "spent" => Some(VtxoState::Spent),
            _ => None,
        }
    }
}

/// An owned VTXO, as observed by the receive path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservedVtxo {
    /// Stock Bark outpoint (txid plus little-endian vout), exactly 36 bytes.
    pub vtxo_id: [u8; 36],
    /// The SP-derived policy key this VTXO pays.
    pub u_pubkey: PublicKey,
    pub amount_sats: u64,
    pub expiry: Expiry,
    pub state: VtxoState,
}

/// The persisted form of an owned VTXO.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VtxoRecord {
    pub vtxo_id: [u8; 36],
    pub u_pubkey: PublicKey,
    pub amount_sats: u64,
    pub state: VtxoState,
    pub expiry: Expiry,
}

/// Receive-path validation failures. All fail closed; nothing is written
/// to the store when one of these fires.
#[derive(Debug, thiserror::Error)]
pub enum ReceiveError {
    #[error("VTXO {0} has a non-positive amount")]
    InvalidAmount(String),
    #[error("VTXO {0} pays U {1}, which the SP keystore does not own")]
    NotOwned(String, String),
    #[error("VTXO {0}: keystore secret does not derive its U (corrupt keystore)")]
    KeystoreMismatch(String),
    #[error("VTXO {0} claims an expiry without a source; an expiry needs its provenance")]
    ExpiryNeedsSource(String),
    #[error(transparent)]
    State(#[from] WalletStateError),
}

/// The live receive backend: reads mailboxes by mode and VTXO status from
/// a real Ark server, exactly as the `spc_ark_client` prototype does
/// (`read_arkoor_mailbox`, `claim_vtxos`, `query_vtxo_status`).
///
/// AP-3b wires the stock-server implementation (ark-lib +
/// bark-server-rpc). AP-3a consumes it only through fixtures in tests;
/// the trait is the seam that keeps the persistence/validation layer
/// honest about what is live and what is not.
pub trait ReceiveBackend {
    /// Poll the mailbox identity (`M` or the epoch-derived `M_i`) for raw
    /// VTXO observations. Unknown/not-found is **not** invalid: the
    /// caller keeps the record pending and retries inside the actual
    /// window (plan 4.4).
    fn poll_mailbox(
        &mut self,
        mailbox_epoch_height: Option<u32>,
    ) -> anyhow::Result<Vec<ObservedVtxo>>;
}

/// Balances and expiry aggregates for the status snapshot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReceiveSummary {
    /// Sum of `spendable` records. `None` until a receive path has run at
    /// least once - unknown is not zero.
    pub spendable_sats: Option<u64>,
    pub pending_sats: Option<u64>,
    /// Sum of spendable records whose known expiry is within the refresh
    /// threshold of the tip. `None` while the tip (scan cursor) is
    /// unknown.
    pub needs_refresh_sats: Option<u64>,
    /// Minimum known expiry height across unspent records.
    pub min_expiry_height: Option<u32>,
    /// Minimum expiry source among those records.
    pub min_expiry_source: Option<ExpirySource>,
    /// Earliest `expiry - refresh_threshold` across spendable records
    /// with a known expiry: the next height at which a refresh is due.
    pub next_refresh_height: Option<u32>,
    /// Count of unspent records whose expiry is unknown.
    pub expiry_unknown_count: usize,
}

/// The receive ledger: validates and persists owned VTXOs over the SP
/// keystore, and computes the status aggregates.
pub struct ReceiveLedger {
    store: std::sync::Arc<WalletStore>,
    /// Blocks before expiry at which a refresh is due (bark default 144;
    /// the lab profile uses 96).
    refresh_threshold: u32,
}

impl ReceiveLedger {
    pub fn new(store: std::sync::Arc<WalletStore>, refresh_threshold: u32) -> Self {
        ReceiveLedger {
            store,
            refresh_threshold: refresh_threshold.max(1),
        }
    }

    /// Validate and persist one observation. Idempotent per `vtxo_id`
    /// with identical fields; conflicting fields are refused.
    pub fn record(
        &self,
        observed: &ObservedVtxo,
    ) -> Result<crate::wallet::state::RecordWrite, ReceiveError> {
        let id_hex = hex::encode(observed.vtxo_id);
        if observed.amount_sats == 0 {
            return Err(ReceiveError::InvalidAmount(id_hex));
        }
        // An expiry always carries its provenance; Unknown carries none.
        match observed.expiry {
            Expiry::Unknown => {}
            Expiry::At { height, source } => {
                if height == 0 {
                    return Err(ReceiveError::ExpiryNeedsSource(id_hex));
                }
                let _ = source;
            }
        }
        // Ownership: the keystore must hold U, and the held secret must
        // actually derive U (a mismatch means corrupt state, not a payout).
        let secret = self
            .store
            .lookup_u(&observed.u_pubkey)?
            .ok_or_else(|| ReceiveError::NotOwned(id_hex.clone(), observed.u_pubkey.to_string()))?;
        let derived = PublicKey::from_secret_key(&Secp256k1::new(), &secret);
        if derived != observed.u_pubkey {
            return Err(ReceiveError::KeystoreMismatch(id_hex));
        }
        let record = VtxoRecord {
            vtxo_id: observed.vtxo_id,
            u_pubkey: observed.u_pubkey,
            amount_sats: observed.amount_sats,
            state: observed.state,
            expiry: observed.expiry,
        };
        Ok(self.store.upsert_vtxo(&record)?)
    }

    /// All owned records (restart-stable, ordered by id).
    pub fn records(&self) -> Result<Vec<VtxoRecord>, WalletStateError> {
        self.store.vtxo_records()
    }

    /// Compute the status aggregates. `tip` is the scan cursor; while it
    /// is unknown, `needs_refresh_sats` stays unknown rather than zero.
    pub fn summary(&self, tip: Option<u32>) -> Result<ReceiveSummary, WalletStateError> {
        let records = self.store.vtxo_records()?;
        let mut summary = ReceiveSummary::default();
        let mut spendable_seen = false;
        let mut pending_seen = false;
        for record in &records {
            if record.state == VtxoState::Spent {
                continue;
            }
            match record.expiry.height() {
                None => summary.expiry_unknown_count += 1,
                Some(height) => {
                    if summary.min_expiry_height.is_none_or(|m| height < m) {
                        summary.min_expiry_height = Some(height);
                        summary.min_expiry_source = record.expiry.source();
                    }
                }
            }
            match record.state {
                VtxoState::Spendable => {
                    spendable_seen = true;
                    summary.spendable_sats =
                        Some(summary.spendable_sats.unwrap_or(0) + record.amount_sats);
                    if let Some(expiry) = record.expiry.height() {
                        let due = expiry.saturating_sub(self.refresh_threshold);
                        if summary.next_refresh_height.is_none_or(|n| due < n) {
                            summary.next_refresh_height = Some(due);
                        }
                        if let Some(tip) = tip {
                            if expiry <= tip.saturating_add(self.refresh_threshold) {
                                summary.needs_refresh_sats = Some(
                                    summary.needs_refresh_sats.unwrap_or(0) + record.amount_sats,
                                );
                            }
                        }
                    }
                }
                VtxoState::Pending => {
                    pending_seen = true;
                    summary.pending_sats =
                        Some(summary.pending_sats.unwrap_or(0) + record.amount_sats);
                }
                VtxoState::Spent => unreachable!("filtered above"),
            }
        }
        if !spendable_seen {
            summary.spendable_sats = if records.is_empty() { None } else { Some(0) };
        }
        if !pending_seen {
            summary.pending_sats = if records.is_empty() { None } else { Some(0) };
        }
        // needs_refresh_sats stays None while the tip is unknown.
        Ok(summary)
    }

    /// Reconcile the owned unspent sum against the pool-reported total
    /// (`ark_due` or an `ArkPayoutNotice` sum). A mismatch is an alarm,
    /// never a silent adjustment (plan 4.4, SPEC 11.6).
    pub fn reconcile(&self, pool_reported_sats: u64) -> Result<Reconciliation, WalletStateError> {
        let owned: u64 = self
            .store
            .vtxo_records()?
            .iter()
            .filter(|r| r.state != VtxoState::Spent)
            .map(|r| r.amount_sats)
            .sum();
        if owned == pool_reported_sats {
            Ok(Reconciliation::Ok)
        } else {
            Ok(Reconciliation::Mismatch {
                pool_reported_sats,
                owned_sats: owned,
            })
        }
    }
}

/// Result of a reconciliation check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reconciliation {
    Ok,
    Mismatch {
        pool_reported_sats: u64,
        owned_sats: u64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallet::state::RecordWrite;
    use bitcoin::secp256k1::SecretKey;

    fn fixture_store() -> (std::sync::Arc<WalletStore>, SecretKey, PublicKey) {
        let dir = tempfile::tempdir().unwrap();
        let secp = Secp256k1::new();
        let scan = SecretKey::from_slice(&[1u8; 32]).unwrap();
        let spend = SecretKey::from_slice(&[2u8; 32]).unwrap();
        let identity = crate::wallet::state::StoreIdentity {
            wallet_fingerprint: bitcoin::bip32::Fingerprint::from([9u8; 4]),
            sp_id: spc_crypto::sp_id(
                &PublicKey::from_secret_key(&secp, &scan),
                &PublicKey::from_secret_key(&secp, &spend),
            ),
            network: "regtest".into(),
        };
        let store = std::sync::Arc::new(WalletStore::open(dir.path(), &identity).unwrap());
        let u = SecretKey::from_slice(&[7u8; 32]).unwrap();
        let u_pub = PublicKey::from_secret_key(&secp, &u);
        store.insert_u(&u_pub, &u, 800_000).unwrap();
        // Leak the tempdir: this fixture is per-test and short-lived.
        std::mem::forget(dir);
        (store, u, u_pub)
    }

    fn observation(u_pub: &PublicKey, amount: u64, expiry: Expiry) -> ObservedVtxo {
        ObservedVtxo {
            vtxo_id: [3u8; 36],
            u_pubkey: *u_pub,
            amount_sats: amount,
            expiry,
            state: VtxoState::Spendable,
        }
    }

    #[test]
    fn records_only_owned_vtxos() {
        let (store, _u, u_pub) = fixture_store();
        let ledger = ReceiveLedger::new(store, 144);
        let secp = Secp256k1::new();
        let stranger =
            PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&[99u8; 32]).unwrap());

        // Not owned: refused, nothing written.
        let err = ledger
            .record(&observation(&stranger, 1000, Expiry::Unknown))
            .unwrap_err();
        assert!(matches!(err, ReceiveError::NotOwned(_, _)));
        assert_eq!(ledger.records().unwrap().len(), 0);

        // Zero amount: refused.
        let err = ledger
            .record(&observation(&u_pub, 0, Expiry::Unknown))
            .unwrap_err();
        assert!(matches!(err, ReceiveError::InvalidAmount(_)));

        // Owned: recorded.
        let expiry = Expiry::At {
            height: 800_432,
            source: ExpirySource::VtxoAncestry,
        };
        assert_eq!(
            ledger.record(&observation(&u_pub, 1000, expiry)).unwrap(),
            crate::wallet::state::RecordWrite::Inserted
        );
        // Same fields: idempotent, no duplicate claim.
        assert_eq!(
            ledger.record(&observation(&u_pub, 1000, expiry)).unwrap(),
            crate::wallet::state::RecordWrite::AlreadyPresent
        );
        // Different fields, same id: conflict.
        let err = ledger
            .record(&observation(&u_pub, 1001, expiry))
            .unwrap_err();
        assert!(matches!(
            err,
            ReceiveError::State(WalletStateError::RecordConflict(_))
        ));
        assert_eq!(ledger.records().unwrap().len(), 1);
    }

    #[test]
    fn expiry_semantics() {
        let (store, _u, u_pub) = fixture_store();
        let ledger = ReceiveLedger::new(store, 96);

        // Unknown expiry: recorded, counted, never a deadline.
        ledger
            .record(&observation(&u_pub, 500, Expiry::Unknown))
            .unwrap();
        let summary = ledger.summary(None).unwrap();
        assert_eq!(summary.expiry_unknown_count, 1);
        assert_eq!(summary.min_expiry_height, None);
        assert_eq!(summary.next_refresh_height, None);
        assert_eq!(summary.spendable_sats, Some(500));

        // Known expiry from ancestry: min expiry and next refresh appear.
        let mut obs = observation(&u_pub, 500, Expiry::Unknown);
        obs.vtxo_id = [4u8; 36];
        obs.expiry = Expiry::At {
            height: 800_432,
            source: ExpirySource::VtxoAncestry,
        };
        ledger.record(&obs).unwrap();
        let summary = ledger.summary(None).unwrap();
        assert_eq!(summary.min_expiry_height, Some(800_432));
        assert_eq!(summary.min_expiry_source, Some(ExpirySource::VtxoAncestry));
        assert_eq!(summary.next_refresh_height, Some(800_432 - 96));
        // Tip unknown: needs_refresh stays unknown, not zero.
        assert_eq!(summary.needs_refresh_sats, None);

        // Tip close to expiry: needs_refresh counts it.
        let summary = ledger.summary(Some(800_432 - 96)).unwrap();
        assert_eq!(summary.needs_refresh_sats, Some(500));

        // Zero-height expiry is refused (needs provenance/sanity).
        let mut obs = observation(&u_pub, 500, Expiry::Unknown);
        obs.vtxo_id = [5u8; 36];
        obs.expiry = Expiry::At {
            height: 0,
            source: ExpirySource::ServerReport,
        };
        let err = ledger.record(&obs).unwrap_err();
        assert!(matches!(err, ReceiveError::ExpiryNeedsSource(_)));
    }

    #[test]
    fn lifecycle_transitions_are_forward_only() {
        let (store, _u, u_pub) = fixture_store();
        let ledger = ReceiveLedger::new(store, 96);

        // Observed pending with unknown expiry.
        let mut obs = observation(&u_pub, 700, Expiry::Unknown);
        obs.vtxo_id = [7u8; 36];
        obs.state = VtxoState::Pending;
        assert_eq!(ledger.record(&obs).unwrap(), RecordWrite::Inserted);

        // Server validation: pending -> spendable, and the actual
        // expiry is learned (Unknown -> At). Both are legitimate.
        obs.state = VtxoState::Spendable;
        obs.expiry = Expiry::At {
            height: 800_500,
            source: ExpirySource::ServerReport,
        };
        assert_eq!(ledger.record(&obs).unwrap(), RecordWrite::Updated);

        // Same again: idempotent.
        assert_eq!(ledger.record(&obs).unwrap(), RecordWrite::AlreadyPresent);

        // Backwards (spendable -> pending): refused.
        obs.state = VtxoState::Pending;
        let err = ledger.record(&obs).unwrap_err();
        assert!(matches!(
            err,
            ReceiveError::State(WalletStateError::RecordConflict(_))
        ));

        // Changing a learned expiry: refused.
        obs.state = VtxoState::Spendable;
        obs.expiry = Expiry::At {
            height: 800_501,
            source: ExpirySource::ServerReport,
        };
        let err = ledger.record(&obs).unwrap_err();
        assert!(matches!(
            err,
            ReceiveError::State(WalletStateError::RecordConflict(_))
        ));

        // Forgetting an expiry: refused.
        obs.expiry = Expiry::Unknown;
        let err = ledger.record(&obs).unwrap_err();
        assert!(matches!(
            err,
            ReceiveError::State(WalletStateError::RecordConflict(_))
        ));

        // Spent is a legal end state.
        obs.expiry = Expiry::At {
            height: 800_500,
            source: ExpirySource::ServerReport,
        };
        obs.state = VtxoState::Spent;
        assert_eq!(ledger.record(&obs).unwrap(), RecordWrite::Updated);
        let records = ledger.records().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].state, VtxoState::Spent);
        // Spent records do not count towards balances.
        let summary = ledger.summary(None).unwrap();
        assert_eq!(summary.spendable_sats, Some(0));
        assert_eq!(summary.pending_sats, Some(0));
    }

    #[test]
    fn reconciliation_alarms_on_mismatch() {
        let (store, _u, u_pub) = fixture_store();
        let ledger = ReceiveLedger::new(store, 144);
        ledger
            .record(&observation(&u_pub, 1234, Expiry::Unknown))
            .unwrap();
        assert_eq!(ledger.reconcile(1234).unwrap(), Reconciliation::Ok);
        assert_eq!(
            ledger.reconcile(1300).unwrap(),
            Reconciliation::Mismatch {
                pool_reported_sats: 1300,
                owned_sats: 1234
            }
        );
    }

    #[test]
    fn empty_ledger_is_unknown_not_zero() {
        let (store, _u, _u_pub) = fixture_store();
        let ledger = ReceiveLedger::new(store, 144);
        let summary = ledger.summary(None).unwrap();
        assert_eq!(summary.spendable_sats, None);
        assert_eq!(summary.pending_sats, None);
        assert_eq!(summary.needs_refresh_sats, None);
        assert_eq!(summary.min_expiry_height, None);
    }
}
