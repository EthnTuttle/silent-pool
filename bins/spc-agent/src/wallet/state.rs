//! Persistent agent wallet state (SPEC 11.5; plan 3.3, 4.1.1).
//!
//! One sqlite database per identity under the wallet datadir, separate
//! from the embedded bark wallet's own database (which the child seed
//! owns). It holds:
//!
//! - the `U_i -> u_i` SP-aware keystore (the thing stock bark cannot do,
//!   SPEC 11.8 item 2),
//! - owned VTXO records with their **actual** expiry (from the VTXO's
//!   ancestry or a server report - never invented from a finding height),
//! - the per-identity monotonic registration counter and pending-request
//!   journal (plan 4.1.1 request-ID policy),
//! - the chain scan cursor.
//!
//! Fail-closed rules:
//!
//! - The store is bound to one `(wallet_fingerprint, sp_id, network)` at
//!   creation. A reopen with different material is an error; state is
//!   never wiped or re-created to make a mismatch go away, and a failed
//!   restore never regenerates seed material.
//! - `schema_version` must match exactly; anything else is corruption.
//! - The registration counter is reserved durably (committed) **before**
//!   the registration frame is sent, and never wraps (`u32::MAX` is
//!   refused with [`WalletStateError::RequestIdExhausted`]).
//! - Registration liveness is the **effective binding**: the single
//!   most recent `applied`/`already_applied` outcome, stored with the
//!   payload digest it acknowledged. A payload is "registered" only
//!   while it matches that binding - never because *some historical*
//!   journal row matches (a payload that was applied, replaced, and
//!   restored is not live again without a fresh acknowledgment).
//!
//! Scan cursor caveat (AP-3b dependency, not implemented here): the
//! cursor is **monotonic-only**. A chain reorg that invalidates already
//! scanned heights requires a *journaled rewind* - record the fork
//! point, replay forward, and only then advance - before the wallet may
//! scan live. Silently rewinding (or ignoring a reorg) could hide or
//! double-count VTXO observations; the rewind journal is AP-3b's to
//! build, and `set_scan_height` refuses any backward move until it
//! exists.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use bitcoin::bip32::Fingerprint;
use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
use rusqlite::{params, Connection, OptionalExtension, Transaction};

use crate::wallet::receive::{Expiry, ExpirySource, VtxoRecord, VtxoState};
use crate::wallet::registration::RegistrationOutcome;

/// A stored VTXO row: `(u_pubkey, amount_sats, state, expiry_height,
/// expiry_source)` as sqlite returns it.
type VtxoRow = (Vec<u8>, i64, String, Option<i64>, Option<String>);

/// The only state schema version this build understands.
///
/// v2 adds the **effective registration binding**
/// (`effective_digest`/`effective_request_id` in `registration_state`):
/// registration liveness is "the current payload matches the most
/// recent acknowledged `applied`/`already_applied` state", not "any
/// historical journal row matches". A v1 database is rejected
/// explicitly with [`WalletStateError::SchemaVersion`] - there is no
/// silent adoption of an old DB (the old journal cannot answer the
/// effective-binding question).
pub const STATE_SCHEMA_VERSION: u32 = 2;

/// Errors from the wallet state store. All fail closed.
#[derive(Debug, thiserror::Error)]
pub enum WalletStateError {
    #[error("wallet state db error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("wallet state at {0} is corrupt: {1}")]
    Corrupt(PathBuf, String),
    #[error(
        "wallet state schema version mismatch: found {found}, this build only supports {expected}"
    )]
    SchemaVersion { found: u32, expected: u32 },
    #[error("wallet state at {0} belongs to a different child seed (fingerprint mismatch); refusing to open")]
    FingerprintMismatch(PathBuf),
    #[error(
        "wallet state at {0} belongs to a different SP identity (sp_id mismatch); refusing to open"
    )]
    SpIdMismatch(PathBuf),
    #[error("wallet state at {path} is for network {found}, config says {expected}")]
    NetworkMismatch {
        path: PathBuf,
        found: String,
        expected: String,
    },
    #[error("registration counter is exhausted (last_request_id == u32::MAX); operator/seed-reset action required (no wrap)")]
    RequestIdExhausted,
    #[error("keystore entry for U already exists with a different secret: {0}")]
    KeystoreMismatch(String),
    #[error("VTXO record {0} already exists with different fields; refusing to rewrite history")]
    RecordConflict(String),
    #[error("scan cursor must be monotonic: last {last}, tried {tried}")]
    ScanCursorNotMonotonic { last: u32, tried: u32 },
}

/// The persistent wallet state.
pub struct WalletStore {
    db: Arc<Mutex<Connection>>,
    path: PathBuf,
}

type LiveReceiptRow = (Vec<u8>, i64, Vec<u8>, Vec<u8>);

// The sqlite connection is not `Sync` on its own; every access goes
// through the mutex, so the store as a whole is. Checked in tests.

impl std::fmt::Debug for WalletStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Path only: the store holds keystore secrets.
        f.debug_struct("WalletStore")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

/// Material the store is bound to at first creation. A later `open` with
/// different material fails closed instead of re-initializing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreIdentity {
    /// Stock bark `WalletSeed` fingerprint of the BIP 85 child seed.
    pub wallet_fingerprint: Fingerprint,
    /// `hash_SPc/Id(ser_P(B_scan) || ser_P(B_spend))`.
    pub sp_id: [u8; 32],
    /// Chain network name (e.g. `regtest`).
    pub network: String,
}

impl WalletStore {
    /// Open (or first-create) the agent wallet state at `dir/state.sqlite`.
    ///
    /// The directory is created 0700 if absent (and tightened if loose);
    /// the database file is kept 0600. Validation happens before any
    /// write, so a failed restore leaves state untouched and never
    /// regenerates anything.
    pub fn open(dir: &Path, identity: &StoreIdentity) -> Result<Self, WalletStateError> {
        Self::prepare_datadir(dir)?;
        let path = dir.join("state.sqlite");
        // First creation is private **before** sqlite is pointed at the
        // file: `Connection::open` would create it with the process
        // umask, leaving a window where spendable-state bytes are
        // group/world readable. An existing file is never recreated.
        #[cfg(unix)]
        if !path.exists() {
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
                .map_err(|e| {
                    WalletStateError::Corrupt(path.clone(), format!("cannot create: {e}"))
                })?;
        }
        let db = Connection::open(&path)
            .map_err(|e| WalletStateError::Corrupt(path.clone(), format!("cannot open: {e}")))?;
        // Any failure this early means the file cannot act as a state
        // database (garbage bytes, foreign db, unreadable disk): that
        // is corruption, and it fails closed.
        db.pragma_update(None, "synchronous", "FULL")
            .map_err(|e| WalletStateError::Corrupt(path.clone(), format!("cannot open: {e}")))?;
        db.pragma_update(None, "foreign_keys", "ON")
            .map_err(|e| WalletStateError::Corrupt(path.clone(), format!("cannot open: {e}")))?;
        Self::chmod_0600(&path)?;

        let store = WalletStore {
            db: Arc::new(Mutex::new(db)),
            path: path.clone(),
        };
        {
            let mut guard = store.lock();
            let tx = guard.transaction()?;
            store.init_or_validate(&tx, identity)?;
            tx.commit()?;
        }
        Ok(store)
    }

    fn prepare_datadir(dir: &Path) -> Result<(), WalletStateError> {
        if !dir.exists() {
            std::fs::create_dir_all(dir).map_err(|e| {
                WalletStateError::Corrupt(dir.to_path_buf(), format!("cannot create: {e}"))
            })?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir)
                .map_err(|e| WalletStateError::Corrupt(dir.to_path_buf(), format!("stat: {e}")))?
                .permissions()
                .mode()
                & 0o777;
            if mode & 0o077 != 0 {
                // The datadir holds spendable VTXO secrets: tighten, and
                // fail if we cannot.
                std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(
                    |e| {
                        WalletStateError::Corrupt(
                            dir.to_path_buf(),
                            format!("cannot tighten to 0700: {e}"),
                        )
                    },
                )?;
            }
        }
        #[cfg(not(unix))]
        {
            return Err(WalletStateError::Corrupt(
                dir.to_path_buf(),
                "the wallet role requires unix file permissions".into(),
            ));
        }
        Ok(())
    }

    #[cfg(unix)]
    fn chmod_0600(path: &Path) -> Result<(), WalletStateError> {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path)
            .map_err(|e| WalletStateError::Corrupt(path.to_path_buf(), format!("stat: {e}")))?
            .permissions()
            .mode()
            & 0o777;
        if mode & 0o077 != 0 {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(
                |e| WalletStateError::Corrupt(path.to_path_buf(), format!("chmod 0600: {e}")),
            )?;
        }
        Ok(())
    }

    pub(super) fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.db.lock().expect("wallet state mutex poisoned")
    }

    /// Test-only direct handle for corrupting state on purpose.
    #[cfg(test)]
    pub(crate) fn lock_for_tests(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.lock()
    }

    fn init_or_validate(
        &self,
        tx: &Transaction<'_>,
        identity: &StoreIdentity,
    ) -> Result<(), WalletStateError> {
        // Does this file already carry OUR schema? The `meta` table is
        // created by this code and always carries `schema_version`.
        let has_meta: bool =
            tx.query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'table' AND name = 'meta'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .map_err(|e| {
                WalletStateError::Corrupt(self.path.clone(), format!("cannot read schema: {e}"))
            })? > 0;

        if !has_meta {
            // No `meta` table. If the file carries ANY table, it is a
            // foreign sqlite database (or damaged state): refuse to
            // adopt it. "First creation" means a genuinely empty
            // database - the schema below is only created then.
            let n_tables: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master
                     WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
                    [],
                    |r| r.get(0),
                )
                .map_err(|e| {
                    WalletStateError::Corrupt(self.path.clone(), format!("cannot read schema: {e}"))
                })?;
            if n_tables > 0 {
                return Err(WalletStateError::Corrupt(
                    self.path.clone(),
                    format!(
                        "foreign sqlite database ({n_tables} table(s) present, no \
                         schema metadata); refusing to adopt"
                    ),
                ));
            }
            tx.execute_batch(
                "CREATE TABLE IF NOT EXISTS meta (
                     key   TEXT PRIMARY KEY,
                     value BLOB NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS sp_keystore (
                     u_pubkey  BLOB PRIMARY KEY,
                     u_secret  BLOB NOT NULL,
                     height    INTEGER NOT NULL,
                     created_at TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS vtxo_records (
                     vtxo_id      BLOB PRIMARY KEY,
                     u_pubkey     BLOB NOT NULL,
                     amount_sats  INTEGER NOT NULL,
                     state        TEXT NOT NULL,
                     expiry_height INTEGER,
                     expiry_source TEXT,
                     received_at  TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS registration_state (
                     id INTEGER PRIMARY KEY CHECK (id = 1),
                     last_request_id  INTEGER NOT NULL,
                     pending_request_id INTEGER,
                     pending_digest   BLOB,
                     effective_digest BLOB,
                     effective_request_id INTEGER,
                     updated_at TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS registration_journal (
                     sp_id      BLOB NOT NULL,
                     request_id INTEGER NOT NULL,
                     outcome    TEXT NOT NULL,
                     error_code TEXT,
                     created_at TEXT NOT NULL,
                     PRIMARY KEY (sp_id, request_id)
                 );
                 CREATE TABLE IF NOT EXISTS scan_cursor (
                     id INTEGER PRIMARY KEY CHECK (id = 1),
                     last_height INTEGER NOT NULL
                 );",
            )
            .map_err(|e| {
                // A file that cannot carry the schema (garbage bytes, an
                // unreadable disk) is corruption.
                WalletStateError::Corrupt(
                    self.path.clone(),
                    format!("cannot initialize schema: {e}"),
                )
            })?;
            // First creation: bind the store to this identity.
            tx.execute(
                "INSERT INTO meta (key, value) VALUES ('schema_version', ?1)",
                params![STATE_SCHEMA_VERSION.to_be_bytes().to_vec()],
            )?;
            tx.execute(
                "INSERT INTO meta (key, value) VALUES ('wallet_fingerprint', ?1)",
                params![identity.wallet_fingerprint.to_bytes().to_vec()],
            )?;
            tx.execute(
                "INSERT INTO meta (key, value) VALUES ('sp_id', ?1)",
                params![identity.sp_id.to_vec()],
            )?;
            tx.execute(
                "INSERT INTO meta (key, value) VALUES ('network', ?1)",
                params![identity.network.as_bytes().to_vec()],
            )?;
            tx.execute(
                "INSERT INTO meta (key, value) VALUES ('created_at', ?1)",
                params![now_rfc3339()],
            )?;
            return Ok(());
        }

        // `meta` exists: this file claims to be our state. It must
        // carry a known schema version and the bound identity, or it is
        // damaged/refused - never silently re-bound.
        let bytes: Option<Vec<u8>> = tx
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let bytes = bytes.ok_or_else(|| {
            WalletStateError::Corrupt(
                self.path.clone(),
                "meta table exists but schema_version is missing (damaged state); \
                 refusing to adopt"
                    .into(),
            )
        })?;
        let found = decode_u32(&bytes).ok_or_else(|| {
            WalletStateError::Corrupt(
                self.path.clone(),
                "schema_version is not a 4-byte big-endian integer".into(),
            )
        })?;
        if found != STATE_SCHEMA_VERSION {
            return Err(WalletStateError::SchemaVersion {
                found,
                expected: STATE_SCHEMA_VERSION,
            });
        }
        self.validate_meta(
            tx,
            "wallet_fingerprint",
            &identity.wallet_fingerprint.to_bytes(),
            |v| {
                if v != identity.wallet_fingerprint.to_bytes() {
                    Err(WalletStateError::FingerprintMismatch(self.path.clone()))
                } else {
                    Ok(())
                }
            },
        )?;
        self.validate_meta(tx, "sp_id", &identity.sp_id[..], |v| {
            if v.as_slice() != &identity.sp_id[..] {
                Err(WalletStateError::SpIdMismatch(self.path.clone()))
            } else {
                Ok(())
            }
        })?;
        self.validate_meta(tx, "network", identity.network.as_bytes(), |v| {
            let found = String::from_utf8_lossy(&v).to_string();
            if found != identity.network {
                Err(WalletStateError::NetworkMismatch {
                    path: self.path.clone(),
                    found,
                    expected: identity.network.clone(),
                })
            } else {
                Ok(())
            }
        })?;
        Ok(())
    }

    fn validate_meta(
        &self,
        tx: &Transaction<'_>,
        key: &str,
        _expected: &[u8],
        check: impl FnOnce(Vec<u8>) -> Result<(), WalletStateError>,
    ) -> Result<(), WalletStateError> {
        let value: Option<Vec<u8>> = tx
            .query_row(
                "SELECT value FROM meta WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()?;
        let value = value.ok_or_else(|| {
            WalletStateError::Corrupt(self.path.clone(), format!("meta key '{key}' missing"))
        })?;
        check(value)
    }

    /// Reserve the next registration request id, durably, before any send.
    ///
    /// Plan 4.1.1: a new registration attempt increments `last_request_id`;
    /// retries of the same attempt reuse it. The reservation (including
    /// the pending digest) is committed before this returns, so a crash
    /// between reservation and send still recovers the same id. A pending
    /// reservation with the same digest is reused; one with a different
    /// digest means the signed fields changed, which requires a fresh id
    /// (reusing it would self-wedge into `registration-mismatch`).
    /// `u32::MAX` is refused: no wrap.
    pub fn reserve_registration_request_id(
        &self,
        payload_digest: &[u8; 32],
    ) -> Result<u32, WalletStateError> {
        let mut guard = self.lock();
        let tx = guard.transaction()?;
        let row: Option<(i64, Option<i64>, Option<Vec<u8>>)> = tx
            .query_row(
                "SELECT last_request_id, pending_request_id, pending_digest
                 FROM registration_state WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let had_row = row.is_some();
        // No row yet: the first reservation creates it (last = 0).
        let (last, pending_id, pending_digest) = row.unwrap_or_default();

        // Retry of the same attempt: reuse the pending id.
        if let (Some(id), Some(digest)) = (&pending_id, &pending_digest) {
            if digest.as_slice() == payload_digest.as_slice() {
                return u32::try_from(*id).map_err(|_| {
                    WalletStateError::Corrupt(
                        self.path.clone(),
                        format!("pending request id {id} out of range"),
                    )
                });
            }
        }

        let last_u32 = u32::try_from(last).map_err(|_| {
            WalletStateError::Corrupt(
                self.path.clone(),
                format!("last_request_id {last} out of range"),
            )
        })?;
        if last_u32 == u32::MAX {
            return Err(WalletStateError::RequestIdExhausted);
        }
        let next = last_u32
            .checked_add(1)
            .ok_or(WalletStateError::RequestIdExhausted)?;
        let now = now_rfc3339();
        if had_row {
            tx.execute(
                "UPDATE registration_state
                 SET last_request_id = ?1, pending_request_id = ?1, pending_digest = ?2,
                     updated_at = ?3
                 WHERE id = 1",
                params![next as i64, payload_digest.to_vec(), now],
            )?;
        } else {
            tx.execute(
                "INSERT INTO registration_state
                     (id, last_request_id, pending_request_id, pending_digest, updated_at)
                 VALUES (1, ?1, ?1, ?2, ?3)",
                params![next as i64, payload_digest.to_vec(), now],
            )?;
        }
        tx.commit()?;
        Ok(next)
    }

    /// Record the outcome of a registration attempt and clear the
    /// pending reservation. `Applied`/`AlreadyApplied` outcomes are
    /// journaled **and** become the effective registration binding for
    /// `payload_digest`; a `Superseded` verdict aimed at the currently
    /// effective payload clears the binding (the server just said a
    /// newer registration is live). The journal keeps every row; the
    /// binding is the single most recent acknowledgment.
    ///
    /// Only the attempt that still owns the pending reservation may
    /// move the binding: a replayed old `request_id` journals but
    /// cannot rewrite the effective state.
    pub fn record_registration_outcome(
        &self,
        sp_id: &[u8; 32],
        request_id: u32,
        payload_digest: &[u8; 32],
        outcome: &RegistrationOutcome,
    ) -> Result<(), WalletStateError> {
        let mut guard = self.lock();
        let tx = guard.transaction()?;
        let outcome_text = outcome.as_str();
        let bound_identity: Vec<u8> =
            tx.query_row("SELECT value FROM meta WHERE key = 'sp_id'", [], |r| {
                r.get(0)
            })?;
        if bound_identity != sp_id {
            return Err(WalletStateError::RecordConflict(
                "registration identity mismatch".into(),
            ));
        }
        let pending: Option<(Option<i64>, Option<Vec<u8>>)> = tx
            .query_row(
                "SELECT pending_request_id, pending_digest FROM registration_state WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let is_pending = pending
            .as_ref()
            .is_some_and(|(id, _)| *id == Some(request_id as i64));
        if is_pending
            && pending.as_ref().and_then(|(_, digest)| digest.as_deref())
                != Some(payload_digest.as_slice())
        {
            return Err(WalletStateError::RecordConflict(
                "registration payload mismatch".into(),
            ));
        }
        tx.execute(
            "INSERT OR REPLACE INTO registration_journal
                 (sp_id, request_id, outcome, error_code, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                sp_id.to_vec(),
                request_id as i64,
                outcome_text,
                outcome.error_code(),
                now_rfc3339()
            ],
        )?;
        if is_pending {
            let now = now_rfc3339();
            match outcome {
                RegistrationOutcome::Applied | RegistrationOutcome::AlreadyApplied => {
                    tx.execute(
                        "UPDATE registration_state
                         SET pending_request_id = NULL, pending_digest = NULL,
                             effective_digest = ?1, effective_request_id = ?2, updated_at = ?3
                         WHERE id = 1",
                        params![payload_digest.to_vec(), request_id as i64, now],
                    )?;
                }
                RegistrationOutcome::Superseded | RegistrationOutcome::Mismatch => {
                    // If the superseded verdict names the currently
                    // effective payload, that binding is no longer the
                    // live destination.
                    tx.execute(
                        "UPDATE registration_state
                         SET pending_request_id = NULL, pending_digest = NULL,
                              effective_request_id = CASE WHEN effective_digest = ?2 THEN NULL ELSE effective_request_id END,
                              effective_digest = CASE WHEN effective_digest = ?2 THEN NULL ELSE effective_digest END,
                              updated_at = ?1
                          WHERE id = 1",
                        params![now, payload_digest.to_vec()],
                    )?;
                }
                _ => {
                    tx.execute(
                        "UPDATE registration_state
                         SET pending_request_id = NULL, pending_digest = NULL, updated_at = ?1
                         WHERE id = 1 AND pending_request_id = ?2",
                        params![now, request_id as i64],
                    )?;
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// The pending (reserved but not resolved) request id, if any.
    pub fn pending_registration_request_id(&self) -> Result<Option<u32>, WalletStateError> {
        let guard = self.lock();
        let id: Option<Option<i64>> = guard
            .query_row(
                "SELECT pending_request_id FROM registration_state WHERE id = 1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        Ok(id.flatten().and_then(|i| u32::try_from(i).ok()))
    }

    /// The last reserved request id (0 if never registered).
    pub fn last_request_id(&self) -> Result<u32, WalletStateError> {
        let guard = self.lock();
        let row: Option<i64> = guard
            .query_row(
                "SELECT last_request_id FROM registration_state WHERE id = 1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        Ok(row.and_then(|i| u32::try_from(i).ok()).unwrap_or(0))
    }

    /// Whether a prior successful bark-persister initialization is
    /// recorded in this store (the `bark/` cross-binding, fix-r2).
    /// Absent for stores created before fix-r2; when present, a wholly
    /// missing `bark/` directory at role start is partial datadir
    /// damage and is refused (the deliberate reset removes the whole
    /// datadir, so both databases disappear together).
    pub fn bark_initialized(&self) -> Result<bool, WalletStateError> {
        let guard = self.lock();
        let value: Option<Vec<u8>> = guard
            .query_row(
                "SELECT value FROM meta WHERE key = 'bark_initialized'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        Ok(value.as_deref() == Some(b"1"))
    }

    /// Record that the bark persister under this datadir initialized
    /// successfully (idempotent; called after every successful
    /// `BarkStore::open`, fresh or reopen, so pre-fix-r2 stores heal
    /// the binding on their next validated open).
    pub fn set_bark_initialized(&self) -> Result<(), WalletStateError> {
        let guard = self.lock();
        guard.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES ('bark_initialized', ?1)",
            params![b"1".to_vec()],
        )?;
        Ok(())
    }

    /// Whether the **current** payload is the live registration: the
    /// payload digest must match the effective binding (the most
    /// recent `applied`/`already_applied` acknowledgment). A historical
    /// journal row with the same digest is *not* proof - if a newer
    /// registration superseded it, only a fresh acknowledgment
    /// re-registers the payload.
    pub fn is_registered(&self, payload_digest: &[u8; 32]) -> Result<bool, WalletStateError> {
        let guard = self.lock();
        let effective: Option<Option<Vec<u8>>> = guard
            .query_row(
                "SELECT effective_digest FROM registration_state WHERE id = 1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        Ok(effective.flatten().as_deref() == Some(payload_digest.as_slice()))
    }

    /// The effective registration binding, if any: `(payload digest,
    /// request id)` of the most recent acknowledged
    /// `applied`/`already_applied` outcome.
    pub fn effective_registration(&self) -> Result<Option<([u8; 32], u32)>, WalletStateError> {
        let guard = self.lock();
        let row: Option<(Option<Vec<u8>>, Option<i64>)> = guard
            .query_row(
                "SELECT effective_digest, effective_request_id
                 FROM registration_state WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        Ok(row.and_then(|(digest, id)| {
            let digest: [u8; 32] = digest?.as_slice().try_into().ok()?;
            let id = u32::try_from(id?).ok()?;
            Some((digest, id))
        }))
    }

    /// Revalidate the last acknowledged payload on a new/live connection with
    /// the SAME signed operation id. A newer pending request always wins.
    pub fn reserve_registration_revalidation(
        &self,
        digest: &[u8; 32],
    ) -> Result<(), WalletStateError> {
        let guard = self.lock();
        guard.execute(
            "UPDATE registration_state SET pending_request_id = effective_request_id,
             pending_digest = effective_digest WHERE id = 1 AND pending_request_id IS NULL
             AND effective_digest = ?1 AND effective_request_id = last_request_id",
            params![digest.to_vec()],
        )?;
        Ok(())
    }

    /// Live transport metadata in the same authoritative SP wallet database.
    /// No amount or second funds ledger is kept here. Raw proof bytes are
    /// retained before the ledger commits; checkpoints advance only afterwards.
    pub fn initialize_live_receipts(&self) -> Result<(), WalletStateError> {
        self.vtxo_records()?;
        self.lock().execute_batch(
            "CREATE TABLE IF NOT EXISTS live_receipts (
                vtxo_id BLOB PRIMARY KEY CHECK(length(vtxo_id) = 36),
                raw BLOB NOT NULL, epoch_height INTEGER NOT NULL,
                epoch_hash BLOB NOT NULL CHECK(length(epoch_hash) = 32),
                mailbox BLOB NOT NULL CHECK(length(mailbox) = 33));
             CREATE TABLE IF NOT EXISTS live_mailboxes (
                mailbox BLOB PRIMARY KEY CHECK(length(mailbox) = 33),
                checkpoint INTEGER NOT NULL CHECK(checkpoint >= 0));",
        )?;
        Ok(())
    }

    pub fn mailbox_checkpoint(&self, mailbox: &[u8; 33]) -> Result<i64, WalletStateError> {
        Ok(self
            .lock()
            .query_row(
                "SELECT checkpoint FROM live_mailboxes WHERE mailbox = ?1",
                params![mailbox.as_slice()],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0))
    }

    pub fn advance_mailbox_checkpoint(
        &self,
        mailbox: &[u8; 33],
        checkpoint: i64,
    ) -> Result<(), WalletStateError> {
        self.lock().execute(
            "INSERT INTO live_mailboxes (mailbox, checkpoint) VALUES (?1, ?2)
            ON CONFLICT(mailbox) DO UPDATE SET checkpoint = MAX(checkpoint, excluded.checkpoint)",
            params![mailbox.as_slice(), checkpoint],
        )?;
        Ok(())
    }

    pub fn persist_live_receipt(
        &self,
        id: &[u8; 36],
        raw: &[u8],
        height: u32,
        hash: &[u8; 32],
        mailbox: &[u8; 33],
    ) -> Result<(), WalletStateError> {
        let mut guard = self.lock();
        let tx = guard.transaction()?;
        let previous: Option<LiveReceiptRow> = tx.query_row(
            "SELECT raw, epoch_height, epoch_hash, mailbox FROM live_receipts WHERE vtxo_id = ?1",
            params![id.as_slice()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).optional()?;
        if let Some((old_raw, old_height, old_hash, old_mailbox)) = previous {
            if old_raw != raw
                || old_height != i64::from(height)
                || old_hash != hash
                || old_mailbox != mailbox
            {
                return Err(WalletStateError::RecordConflict(hex::encode(id)));
            }
        } else {
            tx.execute(
                "INSERT INTO live_receipts (vtxo_id, raw, epoch_height, epoch_hash, mailbox)
                VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    id.as_slice(),
                    raw,
                    i64::from(height),
                    hash.as_slice(),
                    mailbox.as_slice()
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Insert a `U_i -> u_i` mapping. Idempotent only when the secret
    /// matches; a different secret for the same `U` is a fail-closed
    /// mismatch (the pool's derivation changed or the file is corrupt).
    pub fn insert_u(
        &self,
        u_pubkey: &PublicKey,
        u_secret: &SecretKey,
        height: u32,
    ) -> Result<(), WalletStateError> {
        let mut guard = self.lock();
        let tx = guard.transaction()?;
        let existing: Option<Vec<u8>> = tx
            .query_row(
                "SELECT u_secret FROM sp_keystore WHERE u_pubkey = ?1",
                params![u_pubkey.serialize().to_vec()],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(secret) = existing {
            if secret.as_slice() != &u_secret.secret_bytes()[..] {
                return Err(WalletStateError::KeystoreMismatch(hex::encode(
                    u_pubkey.serialize(),
                )));
            }
            return Ok(());
        }
        tx.execute(
            "INSERT INTO sp_keystore (u_pubkey, u_secret, height, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                u_pubkey.serialize().to_vec(),
                u_secret.secret_bytes().to_vec(),
                height as i64,
                now_rfc3339()
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// The secret for `U` if the keystore owns it.
    pub fn lookup_u(&self, u_pubkey: &PublicKey) -> Result<Option<SecretKey>, WalletStateError> {
        let guard = self.lock();
        let secret: Option<Vec<u8>> = guard
            .query_row(
                "SELECT u_secret FROM sp_keystore WHERE u_pubkey = ?1",
                params![u_pubkey.serialize().to_vec()],
                |r| r.get(0),
            )
            .optional()?;
        secret
            .map(|bytes| {
                SecretKey::from_slice(&bytes).map_err(|_| {
                    WalletStateError::Corrupt(
                        self.path.clone(),
                        "keystore secret is not a valid secp256k1 key".into(),
                    )
                })
            })
            .transpose()
    }

    /// Whether the keystore owns `U` (watch-only check without exposing
    /// the secret).
    pub fn owns_u(&self, u_pubkey: &PublicKey) -> Result<bool, WalletStateError> {
        Ok(self.lookup_u(u_pubkey)?.is_some())
    }

    /// Number of `U -> u` entries.
    pub fn u_count(&self) -> Result<usize, WalletStateError> {
        let guard = self.lock();
        let n: i64 = guard.query_row("SELECT COUNT(*) FROM sp_keystore", [], |r| r.get(0))?;
        Ok(n as usize)
    }

    /// The set of owned `U` pubkeys (public watch keys; used by recovery
    /// checks and the receive filter).
    pub fn owned_u_pubkeys(&self) -> Result<Vec<PublicKey>, WalletStateError> {
        let guard = self.lock();
        let mut stmt =
            guard.prepare("SELECT u_pubkey FROM sp_keystore ORDER BY height, u_pubkey")?;
        let rows = stmt.query_map([], |r| {
            let bytes: Vec<u8> = r.get(0)?;
            Ok(bytes)
        })?;
        let mut out = Vec::new();
        for row in rows {
            let bytes = row?;
            let pk = PublicKey::from_slice(&bytes).map_err(|_| {
                WalletStateError::Corrupt(
                    self.path.clone(),
                    "keystore entry is not a valid pubkey".into(),
                )
            })?;
            out.push(pk);
        }
        Ok(out)
    }

    /// Record an owned VTXO observation. Idempotent per `vtxo_id` when
    /// the fields match; legitimate forward transitions are allowed
    /// (`pending -> spendable -> spent`, and learning an expiry that
    /// was `Unknown`); any other rewrite of history is refused.
    pub fn upsert_vtxo(&self, record: &VtxoRecord) -> Result<RecordWrite, WalletStateError> {
        let mut guard = self.lock();
        let tx = guard.transaction()?;
        let result = self.upsert_vtxo_in_transaction(&tx, record)?;
        tx.commit()?;
        Ok(result)
    }

    /// Same authoritative ledger write, shared with atomic aggregate allocation.
    pub(super) fn upsert_vtxo_in_transaction(
        &self,
        tx: &rusqlite::Transaction<'_>,
        record: &VtxoRecord,
    ) -> Result<RecordWrite, WalletStateError> {
        let existing: Option<VtxoRow> = tx
            .query_row(
                "SELECT u_pubkey, amount_sats, state, expiry_height, expiry_source
                 FROM vtxo_records WHERE vtxo_id = ?1",
                params![record.vtxo_id.to_vec()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        if let Some((u, amount, state, expiry_height, expiry_source)) = existing {
            // The identity of a record is immutable.
            let u_same = u.as_slice() == &record.u_pubkey.serialize()[..];
            let amount_same = amount == record.amount_sats as i64;
            if !u_same || !amount_same {
                return Err(WalletStateError::RecordConflict(hex::encode(
                    record.vtxo_id,
                )));
            }
            // State moves forward only (pending -> spendable -> spent).
            let old_state = VtxoState::parse(&state).ok_or_else(|| {
                WalletStateError::Corrupt(
                    self.path.clone(),
                    format!("vtxo record has an unknown state '{state}'"),
                )
            })?;
            if record.state.rank() < old_state.rank() {
                return Err(WalletStateError::RecordConflict(hex::encode(
                    record.vtxo_id,
                )));
            }
            // Expiry: identical is fine; Unknown -> actual is learning
            // the real expiry (server report / ancestry); anything else
            // (including forgetting one, or changing it) is a conflict.
            let old_expiry = match (expiry_height, expiry_source) {
                (Some(h), Some(source)) => Some((
                    u32::try_from(h).map_err(|_| {
                        WalletStateError::Corrupt(
                            self.path.clone(),
                            "vtxo record has an invalid expiry height".into(),
                        )
                    })?,
                    source,
                )),
                (None, None) => None,
                _ => {
                    return Err(WalletStateError::Corrupt(
                        self.path.clone(),
                        "vtxo record has a half-specified expiry".into(),
                    ))
                }
            };
            let expiry_ok = match (old_expiry.clone(), record.expiry) {
                (None, Expiry::Unknown) => true,
                (None, Expiry::At { .. }) => true, // learned the actual expiry
                (
                    Some((h, source)),
                    Expiry::At {
                        height,
                        source: new_source,
                    },
                ) => h == height && ExpirySource::parse(&source) == Some(new_source),
                (Some(_), Expiry::Unknown) => false, // never forget an expiry
            };
            if !expiry_ok {
                return Err(WalletStateError::RecordConflict(hex::encode(
                    record.vtxo_id,
                )));
            }
            let unchanged = old_state == record.state
                && old_expiry.is_some() == record.expiry.height().is_some();
            if unchanged {
                return Ok(RecordWrite::AlreadyPresent);
            }
            tx.execute(
                "UPDATE vtxo_records
                 SET state = ?1, expiry_height = ?2, expiry_source = ?3
                 WHERE vtxo_id = ?4",
                params![
                    record.state.as_str(),
                    record.expiry.height().map(|h| h as i64),
                    record.expiry.source().map(|s| s.as_str()),
                    record.vtxo_id.to_vec()
                ],
            )?;
            return Ok(RecordWrite::Updated);
        }
        tx.execute(
            "INSERT INTO vtxo_records
                 (vtxo_id, u_pubkey, amount_sats, state, expiry_height, expiry_source, received_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                record.vtxo_id.to_vec(),
                record.u_pubkey.serialize().to_vec(),
                record.amount_sats as i64,
                record.state.as_str(),
                record.expiry.height().map(|h| h as i64),
                record.expiry.source().map(|s| s.as_str()),
                now_rfc3339()
            ],
        )?;
        Ok(RecordWrite::Inserted)
    }

    /// All owned VTXO records, ordered by id.
    pub fn vtxo_records(&self) -> Result<Vec<VtxoRecord>, WalletStateError> {
        let guard = self.lock();
        let mut stmt = guard.prepare(
            "SELECT vtxo_id, u_pubkey, amount_sats, state, expiry_height, expiry_source
             FROM vtxo_records ORDER BY vtxo_id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, Vec<u8>>(0)?,
                r.get::<_, Vec<u8>>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<i64>>(4)?,
                r.get::<_, Option<String>>(5)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, u, amount, state, expiry_height, expiry_source) = row?;
            let u_pubkey = PublicKey::from_slice(&u).map_err(|_| {
                WalletStateError::Corrupt(
                    self.path.clone(),
                    "vtxo record has an invalid u_pubkey".into(),
                )
            })?;
            let expiry = match (expiry_height, expiry_source) {
                (Some(h), Some(source)) => Expiry::At {
                    height: u32::try_from(h).map_err(|_| {
                        WalletStateError::Corrupt(
                            self.path.clone(),
                            "vtxo record has an invalid expiry height".into(),
                        )
                    })?,
                    source: ExpirySource::parse(&source).ok_or_else(|| {
                        WalletStateError::Corrupt(
                            self.path.clone(),
                            format!("vtxo record has an unknown expiry source '{source}'"),
                        )
                    })?,
                },
                (None, None) => Expiry::Unknown,
                _ => return Err(WalletStateError::Corrupt(
                    self.path.clone(),
                    "vtxo record has a half-specified expiry (height without source or vice versa)"
                        .into(),
                )),
            };
            out.push(VtxoRecord {
                vtxo_id: id.as_slice().try_into().map_err(|_| {
                    WalletStateError::Corrupt(
                        self.path.clone(),
                         "vtxo record id is not a 36-byte stock Bark outpoint; synthetic AP-3a records cannot be adopted as live receipts".into(),
                    )
                })?,
                u_pubkey,
                amount_sats: amount.try_into().map_err(|_| {
                    WalletStateError::Corrupt(
                        self.path.clone(),
                        "vtxo record amount is negative".into(),
                    )
                })?,
                state: VtxoState::parse(&state).ok_or_else(|| {
                    WalletStateError::Corrupt(
                        self.path.clone(),
                        format!("vtxo record has an unknown state '{state}'"),
                    )
                })?,
                expiry,
            });
        }
        Ok(out)
    }

    /// Advance the chain scan cursor. Monotonic only.
    pub fn set_scan_height(&self, height: u32) -> Result<(), WalletStateError> {
        let mut guard = self.lock();
        let tx = guard.transaction()?;
        let existing: Option<i64> = tx
            .query_row(
                "SELECT last_height FROM scan_cursor WHERE id = 1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        match existing {
            Some(last) => {
                let last = u32::try_from(last).map_err(|_| {
                    WalletStateError::Corrupt(
                        self.path.clone(),
                        "scan cursor height out of range".into(),
                    )
                })?;
                if height < last {
                    return Err(WalletStateError::ScanCursorNotMonotonic {
                        last,
                        tried: height,
                    });
                }
                tx.execute(
                    "UPDATE scan_cursor SET last_height = ?1 WHERE id = 1",
                    params![height as i64],
                )?;
            }
            None => {
                tx.execute(
                    "INSERT INTO scan_cursor (id, last_height) VALUES (1, ?1)",
                    params![height as i64],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// The last scanned height, if a scan has run.
    pub fn last_scan_height(&self) -> Result<Option<u32>, WalletStateError> {
        let guard = self.lock();
        let row: Option<i64> = guard
            .query_row(
                "SELECT last_height FROM scan_cursor WHERE id = 1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        Ok(row.and_then(|h| u32::try_from(h).ok()))
    }
}

/// Result of a VTXO record write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordWrite {
    Inserted,
    /// Same record, already persisted: no duplicate claim.
    AlreadyPresent,
    /// A legitimate forward transition (state advanced, or an unknown
    /// expiry was learned from its actual ancestry / a server report).
    Updated,
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn decode_u32(bytes: &[u8]) -> Option<u32> {
    let array: [u8; 4] = bytes.try_into().ok()?;
    Some(u32::from_be_bytes(array))
}

/// Convenience: derive the `U -> u` pair for a pool block output the way
/// the agent's receive path does (`k = 0` in v0.1), and insert it.
pub fn insert_derived_u(
    store: &WalletStore,
    b_scan: &SecretKey,
    b_ark: &SecretKey,
    a_send: &PublicKey,
    height: u32,
    k: u32,
) -> Result<Option<PublicKey>, WalletStateError> {
    let ecdh = match spc_crypto::SharedSecret::miner_side(height, b_scan, a_send) {
        Ok(ecdh) => ecdh,
        // Hash-to-scalar failure: the pool skips this output too.
        Err(_) => return Ok(None),
    };
    let tweaks = match spc_crypto::tier_tweaks(&ecdh, k) {
        Ok(tweaks) => tweaks,
        Err(_) => return Ok(None),
    };
    let secp = Secp256k1::new();
    let b_ark_pub = PublicKey::from_secret_key(&secp, b_ark);
    let u_pub = spc_crypto::ark_user_pubkey(&b_ark_pub, &tweaks.t_ark)
        .map_err(|e| WalletStateError::Corrupt(store.path.clone(), format!("U derivation: {e}")))?;
    let u = spc_crypto::ark_user_secret(b_ark, &tweaks.t_ark)
        .map_err(|e| WalletStateError::Corrupt(store.path.clone(), format!("u derivation: {e}")))?;
    store.insert_u(&u_pub, &u, height)?;
    Ok(Some(u_pub))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(seed: u8) -> StoreIdentity {
        let secp = Secp256k1::new();
        let scan = SecretKey::from_slice(&[seed; 32]).unwrap();
        let spend = SecretKey::from_slice(&[seed + 1; 32]).unwrap();
        let scan_pub = PublicKey::from_secret_key(&secp, &scan);
        let spend_pub = PublicKey::from_secret_key(&secp, &spend);
        StoreIdentity {
            wallet_fingerprint: Fingerprint::from([seed; 4]),
            sp_id: spc_crypto::sp_id(&scan_pub, &spend_pub),
            network: "regtest".into(),
        }
    }

    #[test]
    fn wallet_store_is_send_and_sync() {
        fn check<T: Send + Sync>() {}
        check::<WalletStore>();
        check::<Arc<WalletStore>>();
    }

    #[test]
    fn creates_binds_and_reopens() {
        let dir = tempfile::tempdir().unwrap();
        let id = identity(1);
        let store = WalletStore::open(dir.path(), &id).unwrap();
        assert_eq!(store.last_request_id().unwrap(), 0);
        assert!(!store.is_registered(&[7u8; 32]).unwrap());
        drop(store);
        // Reopen with the same identity: fine, still empty.
        let store = WalletStore::open(dir.path(), &id).unwrap();
        assert_eq!(store.last_request_id().unwrap(), 0);
        drop(store);
        // Different identity: fail closed, do not wipe.
        let err = WalletStore::open(dir.path(), &identity(2)).unwrap_err();
        assert!(matches!(err, WalletStateError::FingerprintMismatch(_)));
        // ...and the original identity still opens (state untouched).
        WalletStore::open(dir.path(), &id).unwrap();
    }

    /// A v1 database (pre effective-binding schema) is rejected
    /// explicitly - never silently adopted.
    #[test]
    fn v1_schema_is_rejected_not_adopted() {
        let dir = tempfile::tempdir().unwrap();
        let id = identity(1);
        {
            let store = WalletStore::open(dir.path(), &id).unwrap();
            let guard = store.lock();
            guard
                .execute(
                    "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
                    params![1u32.to_be_bytes().to_vec()],
                )
                .unwrap();
        }
        let err = WalletStore::open(dir.path(), &id).unwrap_err();
        assert!(
            matches!(
                err,
                WalletStateError::SchemaVersion {
                    found: 1,
                    expected: STATE_SCHEMA_VERSION
                }
            ),
            "v1 must be rejected explicitly: {err}"
        );
    }

    /// A foreign nonempty sqlite file (tables, no schema metadata) is
    /// refused, not adopted as a "first creation".
    #[test]
    fn foreign_sqlite_is_refused_not_adopted() {
        let dir = tempfile::tempdir().unwrap();
        let id = identity(1);
        let path = dir.path().join("state.sqlite");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE someone_elses (x INTEGER);
                 INSERT INTO someone_elses VALUES (42);",
            )
            .unwrap();
        }
        let before = std::fs::read(&path).unwrap();
        let err = WalletStore::open(dir.path(), &id).unwrap_err();
        assert!(
            matches!(err, WalletStateError::Corrupt(_, ref msg) if msg.contains("foreign")),
            "foreign db must be refused: {err}"
        );
        // Bytes untouched: nothing was adopted or rewritten.
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn schema_version_mismatch_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let id = identity(1);
        {
            let store = WalletStore::open(dir.path(), &id).unwrap();
            let guard = store.lock();
            guard
                .execute(
                    "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
                    params![99u32.to_be_bytes().to_vec()],
                )
                .unwrap();
        }
        let err = WalletStore::open(dir.path(), &id).unwrap_err();
        assert!(matches!(
            err,
            WalletStateError::SchemaVersion {
                found: 99,
                expected: STATE_SCHEMA_VERSION
            }
        ));
    }

    #[test]
    fn corrupt_database_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let id = identity(1);
        {
            let store = WalletStore::open(dir.path(), &id).unwrap();
            drop(store);
        }
        let path = dir.path().join("state.sqlite");
        std::fs::write(&path, b"this is not a sqlite database").unwrap();
        let err = WalletStore::open(dir.path(), &id).unwrap_err();
        assert!(matches!(err, WalletStateError::Corrupt(_, _)));
    }

    #[test]
    fn counter_reserves_durably_and_reuses_pending() {
        let dir = tempfile::tempdir().unwrap();
        let id = identity(1);
        let store = WalletStore::open(dir.path(), &id).unwrap();
        let digest_a = [1u8; 32];
        let digest_b = [2u8; 32];

        // New attempt: 1. Retry of the same attempt: still 1.
        assert_eq!(store.reserve_registration_request_id(&digest_a).unwrap(), 1);
        assert_eq!(store.reserve_registration_request_id(&digest_a).unwrap(), 1);
        // Changed signed fields: fresh id (self-wedge avoidance).
        assert_eq!(store.reserve_registration_request_id(&digest_b).unwrap(), 2);
        // Resolve 2, then the next attempt increments (never wraps back).
        store
            .record_registration_outcome(&id.sp_id, 2, &digest_b, &RegistrationOutcome::Applied)
            .unwrap();
        assert_eq!(store.reserve_registration_request_id(&digest_b).unwrap(), 3);

        // Durability across "restart": a fresh handle on the same file.
        drop(store);
        let store = WalletStore::open(dir.path(), &id).unwrap();
        assert_eq!(store.last_request_id().unwrap(), 3);
        assert!(store.is_registered(&digest_b).unwrap());
        assert!(!store.is_registered(&digest_a).unwrap());
        assert_eq!(
            store.effective_registration().unwrap(),
            Some((digest_b, 2)),
            "the binding is the acknowledged request, not the pending one"
        );
        // The pending 3 is still reserved with digest_b and is reused.
        assert_eq!(store.reserve_registration_request_id(&digest_b).unwrap(), 3);
    }

    /// The effective-binding semantics (blocker 4): applied A, then
    /// applied B, then back to A - the historical A row is NOT proof;
    /// only the current acknowledgment counts. An interrupted B leaves
    /// B unregistered (A stays live until something replaces it).
    #[test]
    fn effective_binding_is_the_latest_acknowledgment_only() {
        let dir = tempfile::tempdir().unwrap();
        let id = identity(1);
        let store = WalletStore::open(dir.path(), &id).unwrap();
        let digest_a = [1u8; 32];
        let digest_b = [2u8; 32];

        // A applied: A is live.
        assert_eq!(store.reserve_registration_request_id(&digest_a).unwrap(), 1);
        store
            .record_registration_outcome(&id.sp_id, 1, &digest_a, &RegistrationOutcome::Applied)
            .unwrap();
        assert!(store.is_registered(&digest_a).unwrap());

        // Payload changes to B: a fresh id is reserved (2) and applied.
        assert_eq!(store.reserve_registration_request_id(&digest_b).unwrap(), 2);
        store
            .record_registration_outcome(&id.sp_id, 2, &digest_b, &RegistrationOutcome::Applied)
            .unwrap();
        assert!(store.is_registered(&digest_b).unwrap());
        // ...and A is no longer live, despite its historical row.
        assert!(
            !store.is_registered(&digest_a).unwrap(),
            "a historical applied row is not proof of liveness"
        );

        // Back to A: the historical A row must NOT re-register it. The
        // next reservation is a fresh id (3), not a replay of 1.
        assert_eq!(store.reserve_registration_request_id(&digest_a).unwrap(), 3);
        assert!(!store.is_registered(&digest_a).unwrap());
        store
            .record_registration_outcome(&id.sp_id, 3, &digest_a, &RegistrationOutcome::Applied)
            .unwrap();
        assert!(store.is_registered(&digest_a).unwrap());
        assert_eq!(store.effective_registration().unwrap(), Some((digest_a, 3)));
    }

    /// An interrupted (retryable/unknown) attempt for B leaves B
    /// unregistered; the previously acknowledged A stays the effective
    /// state until a fresh acknowledgment replaces it.
    #[test]
    fn interrupted_attempt_does_not_flip_the_binding() {
        let dir = tempfile::tempdir().unwrap();
        let id = identity(1);
        let store = WalletStore::open(dir.path(), &id).unwrap();
        let digest_a = [1u8; 32];
        let digest_b = [2u8; 32];

        assert_eq!(store.reserve_registration_request_id(&digest_a).unwrap(), 1);
        store
            .record_registration_outcome(&id.sp_id, 1, &digest_a, &RegistrationOutcome::Applied)
            .unwrap();
        assert!(store.is_registered(&digest_a).unwrap());

        // B is reserved but the outcome is never resolved (retryable /
        // unknown / crash): no journal row, no binding change.
        assert_eq!(store.reserve_registration_request_id(&digest_b).unwrap(), 2);
        assert!(!store.is_registered(&digest_b).unwrap());
        assert!(store.is_registered(&digest_a).unwrap());
        assert_eq!(store.effective_registration().unwrap(), Some((digest_a, 1)));
    }

    /// A `superseded` verdict aimed at the effective payload clears the
    /// binding: the server said a newer registration is live.
    #[test]
    fn superseded_clears_the_effective_binding_it_names() {
        let dir = tempfile::tempdir().unwrap();
        let id = identity(1);
        let store = WalletStore::open(dir.path(), &id).unwrap();
        let digest_a = [1u8; 32];

        assert_eq!(store.reserve_registration_request_id(&digest_a).unwrap(), 1);
        store
            .record_registration_outcome(&id.sp_id, 1, &digest_a, &RegistrationOutcome::Applied)
            .unwrap();
        assert!(store.is_registered(&digest_a).unwrap());

        // The same payload, retried, comes back superseded.
        assert_eq!(store.reserve_registration_request_id(&digest_a).unwrap(), 2);
        store
            .record_registration_outcome(&id.sp_id, 2, &digest_a, &RegistrationOutcome::Superseded)
            .unwrap();
        assert!(
            !store.is_registered(&digest_a).unwrap(),
            "superseded means a newer registration is live"
        );
        assert_eq!(store.effective_registration().unwrap(), None);
    }

    /// A replayed old request id journals its row but cannot rewrite
    /// the effective binding or steal the pending reservation.
    #[test]
    fn replayed_request_id_cannot_rewrite_the_binding() {
        let dir = tempfile::tempdir().unwrap();
        let id = identity(1);
        let store = WalletStore::open(dir.path(), &id).unwrap();
        let digest_a = [1u8; 32];
        let digest_b = [2u8; 32];

        assert_eq!(store.reserve_registration_request_id(&digest_a).unwrap(), 1);
        store
            .record_registration_outcome(&id.sp_id, 1, &digest_a, &RegistrationOutcome::Applied)
            .unwrap();
        // A new pending attempt (id 2, digest B) is in flight...
        assert_eq!(store.reserve_registration_request_id(&digest_b).unwrap(), 2);
        // ...when a stale delivery of the old outcome for id 1 arrives
        // again. It journals, but must not touch anything live.
        store
            .record_registration_outcome(&id.sp_id, 1, &digest_a, &RegistrationOutcome::Applied)
            .unwrap();
        assert_eq!(store.effective_registration().unwrap(), Some((digest_a, 1)));
        assert_eq!(
            store.pending_registration_request_id().unwrap(),
            Some(2),
            "the in-flight reservation survives the replay"
        );
    }

    #[test]
    fn counter_wrap_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let id = identity(1);
        let store = WalletStore::open(dir.path(), &id).unwrap();
        {
            let guard = store.lock();
            guard
                .execute(
                    "INSERT INTO registration_state
                         (id, last_request_id, updated_at) VALUES (1, ?1, 'x')",
                    params![u32::MAX as i64],
                )
                .unwrap();
        }
        let err = store
            .reserve_registration_request_id(&[9u8; 32])
            .unwrap_err();
        assert!(matches!(err, WalletStateError::RequestIdExhausted));
    }

    #[test]
    fn counter_concurrent_reservations_are_unique_and_gapless() {
        let dir = tempfile::tempdir().unwrap();
        let id = identity(1);
        let store = Arc::new(WalletStore::open(dir.path(), &id).unwrap());
        let mut handles = Vec::new();
        for t in 0..8u8 {
            let store = Arc::clone(&store);
            handles.push(std::thread::spawn(move || {
                let mut got = Vec::new();
                for i in 0..8u8 {
                    // Unique digest per (thread, iteration): a shared
                    // digest would be (correctly) treated as the same
                    // attempt and reuse the pending id.
                    let mut digest = [0u8; 32];
                    digest[0] = t;
                    digest[1] = i;
                    got.push(store.reserve_registration_request_id(&digest).unwrap());
                }
                got
            }));
        }
        let mut all = Vec::new();
        for handle in handles {
            all.extend(handle.join().unwrap());
        }
        all.sort_unstable();
        let expected: Vec<u32> = (1..=64).collect();
        assert_eq!(all, expected, "unique and gapless reservations");
        assert_eq!(store.last_request_id().unwrap(), 64);
    }

    #[test]
    fn keystore_is_idempotent_but_conflict_fails() {
        let dir = tempfile::tempdir().unwrap();
        let store = WalletStore::open(dir.path(), &identity(1)).unwrap();
        let secp = Secp256k1::new();
        let u = SecretKey::from_slice(&[7u8; 32]).unwrap();
        let u_pub = PublicKey::from_secret_key(&secp, &u);
        store.insert_u(&u_pub, &u, 100).unwrap();
        store.insert_u(&u_pub, &u, 100).unwrap();
        let other = SecretKey::from_slice(&[8u8; 32]).unwrap();
        let err = store.insert_u(&u_pub, &other, 100).unwrap_err();
        assert!(matches!(err, WalletStateError::KeystoreMismatch(_)));
        assert!(store.owns_u(&u_pub).unwrap());
        assert_eq!(store.lookup_u(&u_pub).unwrap().unwrap(), u);
        assert_eq!(store.u_count().unwrap(), 1);
        assert_eq!(store.owned_u_pubkeys().unwrap(), vec![u_pub]);
    }

    #[test]
    fn scan_cursor_is_monotonic() {
        let dir = tempfile::tempdir().unwrap();
        let store = WalletStore::open(dir.path(), &identity(1)).unwrap();
        assert_eq!(store.last_scan_height().unwrap(), None);
        store.set_scan_height(100).unwrap();
        store.set_scan_height(100).unwrap();
        store.set_scan_height(101).unwrap();
        let err = store.set_scan_height(99).unwrap_err();
        assert!(matches!(
            err,
            WalletStateError::ScanCursorNotMonotonic {
                last: 101,
                tried: 99
            }
        ));
        assert_eq!(store.last_scan_height().unwrap(), Some(101));
    }
}
