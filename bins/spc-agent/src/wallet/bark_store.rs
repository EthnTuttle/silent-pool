//! Stock bark sqlite persistence foundation (SPEC 11.5 wallet role;
//! Fable rework blocker 1).
//!
//! Scope - deliberately narrow: this module proves the **stock
//! persister foundation**, nothing more. It opens the pinned stock bark
//! `SqliteClient` (bark c419159, `UPSTREAM_DEPS.md`) at
//! `<datadir>/bark/db.sqlite`, initializes it through the stock
//! `BarkPersister::init_wallet` with
//! `WalletProperties(network, fingerprint, None, None)`, and on reopen
//! validates that the persisted properties still name the same network
//! and the same BIP 85 child-seed fingerprint.
//!
//! What this is **not**: a full stock bark `Wallet::open`. That needs a
//! live chain source (bitcoind RPC) and lands in AP-3b together with
//! live balance, claim, refresh and offboard. Nothing here talks to a
//! server, invents a chain, or asserts any balance.
//!
//! Fail-closed rules (same shape as the agent state store):
//!
//! - the sidecar markers are inspected **before any mutating step,
//!   regardless of whether the database file exists**: a completion
//!   marker (`spc-initialized`) alongside a missing or empty database
//!   is partial datadir damage (the deliberate reset destroys the
//!   whole datadir, never `bark/` alone) - refused, never
//!   re-initialized;
//! - a creation marker (`spc-creating`) without a database file cannot
//!   arise from this module's own ordering (the file is created before
//!   the marker) and is refused; a 0-byte database with only the
//!   creation marker is this module's own crash window and the
//!   initialization may complete;
//! - first creation secures the directory (0700) and creates the
//!   database file 0600 **before** sqlite is pointed at it, re-checks
//!   the completion-marker absence immediately before `init_wallet`
//!   (so a marker collision can only ever be a true concurrent race,
//!   never damage discovered after a mutation), and writes the
//!   completion marker (0600) only after `init_wallet` commits;
//! - an existing file is pre-checked through a **read-only** sqlite
//!   connection before `SqliteClient::open` (which runs migrations and
//!   would therefore mutate a foreign file): garbage bytes are
//!   corruption, a non-bark sqlite database is foreign and refused, and
//!   a bark database whose properties name a different child seed or
//!   network is refused with its bytes untouched;
//! - a database that claims initialization (completion marker present,
//!   or a `bark_properties` row) but cannot present complete identity
//!   metadata is **damaged prior state, not fresh**: refused, never
//!   recreated;
//! - a bark-migrated database with no properties row and no data is an
//!   incomplete first initialization (a crash between migration and
//!   `init_wallet`): the initialization is completed, not redone from
//!   scratch.
//!
//! Sidecar lifecycle: two 0600 markers under `<datadir>/bark/`.
//! `spc-creating` is written **before** the stock client first opens a
//! fresh database (creation intent); `spc-initialized` is written only
//! after `init_wallet` commits (completion). On any open, with or
//! without a database file:
//!
//! - database missing or empty + `spc-initialized` present -> damaged
//!   prior initialization: refused, never re-initialized;
//! - database missing + `spc-creating` present (no completion marker)
//!   -> impossible for this module's ordering: refused;
//! - database empty + only `spc-creating` present -> our own crash
//!   window between file creation and the stock client's first open:
//!   the initialization may complete;
//! - properties row present -> validate network + fingerprint against
//!   the provisioned material (normal reopen);
//! - no row + `spc-initialized` present -> damaged prior
//!   initialization: refused, never re-initialized;
//! - no row + `spc-creating` present -> our own crashed first
//!   creation: the initialization is completed (the stock migrations
//!   are idempotent);
//! - no row + no markers + tables present -> a bark-shaped database
//!   this process never claimed: foreign, refused;
//! - no tables at all and no markers -> a fresh empty database:
//!   initialized.

use std::path::{Path, PathBuf};
use std::str::FromStr as _;

use bitcoin::bip32::Fingerprint;
use bitcoin::Network;
use rusqlite::{OpenFlags, OptionalExtension as _};

use bark::persist::sqlite::SqliteClient;
use bark::persist::BarkPersister;
use bark::WalletProperties;

/// Errors from the stock bark persister foundation. All fail closed;
/// none of them ever creates, deletes or rewrites database bytes.
#[derive(Debug, thiserror::Error)]
pub enum BarkStoreError {
    #[error("bark wallet db at {0} is corrupt or unusable: {1}")]
    Corrupt(PathBuf, String),
    #[error(
        "bark wallet db at {0} is a foreign sqlite database (no bark properties table); refusing to adopt"
    )]
    Foreign(PathBuf),
    #[error(
        "bark wallet db at {path} belongs to a different child seed (fingerprint {found}, expected {expected}); refusing to open"
    )]
    FingerprintMismatch {
        path: PathBuf,
        found: String,
        expected: String,
    },
    #[error("bark wallet db at {path} is for network {found}, config says {expected}")]
    NetworkMismatch {
        path: PathBuf,
        found: String,
        expected: String,
    },
    #[error("bark wallet db error: {0}")]
    Bark(#[source] anyhow::Error),
}

/// The stock bark persister handle bound to one child seed and network.
pub struct BarkStore {
    client: std::sync::Arc<SqliteClient>,
    db_path: PathBuf,
    network: Network,
    fingerprint: Fingerprint,
    /// Whether this open performed (or completed) the first
    /// initialization. A reopen of prior state is never `fresh`.
    fresh: bool,
}

impl std::fmt::Debug for BarkStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Path only: the store is keyed by a child-seed fingerprint.
        f.debug_struct("BarkStore")
            .field("db_path", &self.db_path)
            .field("network", &self.network)
            .field("fingerprint", &self.fingerprint)
            .field("fresh", &self.fresh)
            .finish()
    }
}

/// What a read-only pre-check found in an existing database file.
#[derive(Debug)]
enum Prior {
    /// No file at all: the only genuinely-fresh case.
    Absent,
    /// A sqlite file with no tables (including a 0-byte file): fresh.
    Empty,
    /// No properties row, but our creation-intent marker is present:
    /// our own crashed first initialization; safe to complete.
    IncompleteInit,
    /// A properties row naming a prior initialization.
    Initialized {
        network: String,
        fingerprint: String,
    },
}

impl BarkStore {
    /// Open (or first-create) the stock bark wallet database under
    /// `<datadir>/bark/`, bound to `network` and the BIP 85 child
    /// seed's `fingerprint`.
    ///
    /// Fail-closed on every ambiguity (see the module docs); a failed
    /// open never deletes, recreates or rewrites anything.
    pub async fn open(
        datadir: &Path,
        network: Network,
        fingerprint: Fingerprint,
    ) -> Result<Self, BarkStoreError> {
        let bark_dir = datadir.join("bark");
        prepare_bark_dir(&bark_dir)?;
        let db_path = bark_dir.join("db.sqlite");
        let creating_marker = bark_dir.join("spc-creating");
        let initialized_marker = bark_dir.join("spc-initialized");

        let prior = if db_path.exists() {
            read_prior(&db_path, &creating_marker, &initialized_marker)?
        } else {
            Prior::Absent
        };

        // Marker consistency BEFORE any mutating step (fix-r2): the
        // sidecar markers are consulted regardless of whether the
        // database file exists - `SqliteClient::open` runs migrations
        // and must not touch a directory we are about to refuse.
        if matches!(prior, Prior::Absent | Prior::Empty) {
            if initialized_marker.exists() {
                return Err(BarkStoreError::Corrupt(
                    db_path.clone(),
                    "initialization marker present but database missing/empty: \
                     damaged prior initialization, refusing to re-initialize"
                        .into(),
                ));
            }
            if matches!(prior, Prior::Absent) && creating_marker.exists() {
                // This module creates the database file before the
                // creation marker, so a marker without a file cannot
                // arise from its own ordering and is not adopted.
                return Err(BarkStoreError::Corrupt(
                    db_path.clone(),
                    "creation marker present but database file absent (impossible \
                     for this module's ordering); refusing"
                        .into(),
                ));
            }
            // `Empty` + only `spc-creating` is this module's own crash
            // window (file created 0-byte, marker written, crash
            // before the stock client first opened it): the
            // initialization may complete below.
        }

        // Validate any prior initialization BEFORE any mutating open:
        // `SqliteClient::open` runs migrations and must not touch a
        // database we are about to refuse.
        if let Prior::Initialized {
            network: found_network,
            fingerprint: found_fingerprint,
        } = &prior
        {
            check_prior_identity(
                &db_path,
                network,
                fingerprint,
                found_network,
                found_fingerprint,
            )?;
        }

        // First creation: the file and the creation-intent marker are
        // created 0600 by us before sqlite ever opens the file (sqlite
        // would apply the process umask).
        let fresh = !matches!(prior, Prior::Initialized { .. });
        if fresh && initialized_marker.exists() {
            // Final marker-existence check immediately before the
            // mutating path: the consistency block above and
            // `read_prior` already refuse every marker-present prior,
            // so reaching here means the marker appeared concurrently.
            // Refusing here guarantees `create_new` on the completion
            // marker below can only fail on a true race, never after
            // this process has already mutated the database.
            return Err(BarkStoreError::Corrupt(
                db_path.clone(),
                "completion marker appeared while opening for first \
                 initialization; refusing to re-initialize"
                    .into(),
            ));
        }
        if matches!(prior, Prior::Absent) {
            create_private_file(&db_path)?;
            create_private_file(&creating_marker)?;
        }

        // The pinned stock client (runs the stock migrations).
        let client = SqliteClient::open(&db_path).map_err(BarkStoreError::Bark)?;
        chmod_0600(&db_path)?;

        if fresh {
            // The pinned stock initialization path.
            let properties = WalletProperties {
                network,
                fingerprint,
                server_pubkey: None,
                server_mailbox_pubkey: None,
            };
            client
                .init_wallet(&properties)
                .await
                .map_err(BarkStoreError::Bark)?;
            create_private_file(&initialized_marker)?;
            let _ = std::fs::remove_file(&creating_marker);
        } else {
            // Authoritative stock-API re-read of the persisted
            // properties, validated against the provisioned material.
            let properties = client
                .read_properties()
                .await
                .map_err(BarkStoreError::Bark)?
                .ok_or_else(|| {
                    BarkStoreError::Corrupt(
                        db_path.clone(),
                        "initialization is claimed but the stock read_properties returned \
                         None (damaged prior initialization); refusing to re-initialize"
                            .into(),
                    )
                })?;
            if properties.fingerprint != fingerprint {
                return Err(BarkStoreError::FingerprintMismatch {
                    path: db_path,
                    found: properties.fingerprint.to_string(),
                    expected: fingerprint.to_string(),
                });
            }
            if properties.network != network {
                return Err(BarkStoreError::NetworkMismatch {
                    path: db_path,
                    found: properties.network.to_string(),
                    expected: network.to_string(),
                });
            }
            // A validated prior init makes any stale creation-intent
            // marker obsolete.
            let _ = std::fs::remove_file(&creating_marker);
        }

        tracing::info!(
            db = %db_path.display(),
            fresh,
            fingerprint = %fingerprint,
            "wallet role: stock bark persister open (foundation only; full Wallet::open is AP-3b)"
        );
        Ok(BarkStore {
            client: std::sync::Arc::new(client),
            db_path,
            network,
            fingerprint,
            fresh,
        })
    }

    /// The pinned stock client (for AP-3b, which builds the full
    /// `Wallet` on top of this foundation).
    pub fn client(&self) -> &SqliteClient {
        &self.client
    }

    /// Share the already validated stock persister; never open a second wallet DB.
    pub fn persister(&self) -> std::sync::Arc<dyn BarkPersister> {
        self.client.clone()
    }

    /// The database path (diagnostics and tests).
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// Whether this open performed (or completed) the first
    /// initialization.
    pub fn is_fresh(&self) -> bool {
        self.fresh
    }

    /// Read the persisted stock `WalletProperties` through the pinned
    /// persister API.
    pub async fn properties(&self) -> Result<Option<WalletProperties>, BarkStoreError> {
        self.client
            .read_properties()
            .await
            .map_err(BarkStoreError::Bark)
    }
}

fn check_prior_identity(
    db_path: &Path,
    network: Network,
    fingerprint: Fingerprint,
    found_network: &str,
    found_fingerprint: &str,
) -> Result<(), BarkStoreError> {
    let found_fingerprint = Fingerprint::from_str(found_fingerprint).map_err(|_| {
        BarkStoreError::Corrupt(
            db_path.to_path_buf(),
            format!("stored fingerprint '{found_fingerprint}' is not a valid fingerprint"),
        )
    })?;
    if found_fingerprint != fingerprint {
        return Err(BarkStoreError::FingerprintMismatch {
            path: db_path.to_path_buf(),
            found: found_fingerprint.to_string(),
            expected: fingerprint.to_string(),
        });
    }
    if found_network != network.to_string() {
        return Err(BarkStoreError::NetworkMismatch {
            path: db_path.to_path_buf(),
            found: found_network.to_string(),
            expected: network.to_string(),
        });
    }
    Ok(())
}

/// Read-only pre-check of an existing database file. Never writes.
fn read_prior(
    db_path: &Path,
    creating_marker: &Path,
    initialized_marker: &Path,
) -> Result<Prior, BarkStoreError> {
    let corrupt = |msg: String| BarkStoreError::Corrupt(db_path.to_path_buf(), msg);
    // A 0-byte file is an empty sqlite database (fresh).
    if std::fs::metadata(db_path)
        .map_err(|e| corrupt(format!("cannot stat: {e}")))?
        .len()
        == 0
    {
        return Ok(Prior::Empty);
    }
    let conn = rusqlite::Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| {
            corrupt(format!(
                "cannot open read-only (not a sqlite database?): {e}"
            ))
        })?;
    let has_props_table: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'bark_properties'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map_err(|e| corrupt(format!("cannot read schema: {e}")))?
        > 0;
    let n_tables: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
            [],
            |r| r.get(0),
        )
        .map_err(|e| corrupt(format!("cannot read schema: {e}")))?;
    if n_tables == 0 {
        return Ok(Prior::Empty);
    }
    if !has_props_table {
        return Err(BarkStoreError::Foreign(db_path.to_path_buf()));
    }
    // The stock properties row (single row, id '1').
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT network, fingerprint FROM bark_properties WHERE id = '1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| corrupt(format!("cannot read bark properties: {e}")))?;
    match row {
        Some((network, fingerprint)) => Ok(Prior::Initialized {
            network,
            fingerprint,
        }),
        None => {
            if initialized_marker.exists() {
                return Err(corrupt(
                    "initialization marker present but no bark properties row: damaged \
                     prior initialization, refusing to re-initialize"
                        .into(),
                ));
            }
            if creating_marker.exists() {
                // Our own crashed first creation (marker written before
                // the stock client ever opened the file).
                return Ok(Prior::IncompleteInit);
            }
            // A bark-shaped database this process never claimed: no
            // properties row, no markers. Fail closed.
            Err(corrupt(
                "bark tables present but no wallet properties and no creation marker: \
                 foreign or damaged database, refusing to adopt"
                    .into(),
            ))
        }
    }
}

/// Create the bark directory 0700 (tightened if loose).
fn prepare_bark_dir(dir: &Path) -> Result<(), BarkStoreError> {
    if !dir.exists() {
        std::fs::create_dir_all(dir).map_err(|e| {
            BarkStoreError::Corrupt(dir.to_path_buf(), format!("cannot create: {e}"))
        })?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir)
            .map_err(|e| BarkStoreError::Corrupt(dir.to_path_buf(), format!("stat: {e}")))?
            .permissions()
            .mode()
            & 0o777;
        if mode & 0o077 != 0 {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(|e| {
                BarkStoreError::Corrupt(dir.to_path_buf(), format!("cannot tighten to 0700: {e}"))
            })?;
        }
    }
    #[cfg(not(unix))]
    {
        return Err(BarkStoreError::Corrupt(
            dir.to_path_buf(),
            "the wallet role requires unix file permissions".into(),
        ));
    }
    Ok(())
}

/// Create a fresh private (0600) file; fails if it already exists.
#[cfg(unix)]
fn create_private_file(path: &Path) -> Result<(), BarkStoreError> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| BarkStoreError::Corrupt(path.to_path_buf(), format!("cannot create: {e}")))?;
    Ok(())
}

#[cfg(not(unix))]
fn create_private_file(path: &Path) -> Result<(), BarkStoreError> {
    Err(BarkStoreError::Corrupt(
        path.to_path_buf(),
        "the wallet role requires unix file permissions".into(),
    ))
}

/// Tighten an existing file to 0600 (fail if it cannot be tightened).
#[cfg(unix)]
fn chmod_0600(path: &Path) -> Result<(), BarkStoreError> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path)
        .map_err(|e| BarkStoreError::Corrupt(path.to_path_buf(), format!("stat: {e}")))?
        .permissions()
        .mode()
        & 0o777;
    if mode & 0o077 != 0 {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| BarkStoreError::Corrupt(path.to_path_buf(), format!("chmod 0600: {e}")))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn chmod_0600(path: &Path) -> Result<(), BarkStoreError> {
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fingerprint(seed: u8) -> Fingerprint {
        Fingerprint::from([seed; 4])
    }

    fn sha256_of(path: &Path) -> Vec<u8> {
        use bitcoin::hashes::{sha256, Hash as _};
        sha256::Hash::hash(&std::fs::read(path).unwrap())
            .to_byte_array()
            .to_vec()
    }

    #[tokio::test]
    async fn stock_db_initializes_with_expected_properties_and_reopens() {
        let dir = tempfile::tempdir().unwrap();
        let fp = fingerprint(1);
        let store = BarkStore::open(dir.path(), Network::Regtest, fp)
            .await
            .unwrap();
        assert!(store.is_fresh(), "the first open initializes");
        // The stock properties, exactly as specified: network,
        // fingerprint, no server keys (no server was contacted).
        let props = store.properties().await.unwrap().expect("initialized");
        assert_eq!(props.network, Network::Regtest);
        assert_eq!(props.fingerprint, fp);
        assert_eq!(props.server_pubkey, None);
        assert_eq!(props.server_mailbox_pubkey, None);

        // Reopen with the same child seed: a normal reopen, not fresh.
        drop(store);
        let store = BarkStore::open(dir.path(), Network::Regtest, fp)
            .await
            .unwrap();
        assert!(!store.is_fresh(), "a reopen must not re-initialize");
        let props = store.properties().await.unwrap().unwrap();
        assert_eq!(props.fingerprint, fp);
        assert_eq!(props.network, Network::Regtest);
    }

    #[tokio::test]
    async fn first_creation_secures_directory_file_and_markers() {
        let dir = tempfile::tempdir().unwrap();
        let bark_dir = dir.path().join("bark");
        assert!(!bark_dir.exists());
        BarkStore::open(dir.path(), Network::Regtest, fingerprint(2))
            .await
            .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dir_mode = std::fs::metadata(&bark_dir).unwrap().permissions().mode() & 0o777;
            assert_eq!(dir_mode, 0o700, "bark dir is 0700");
            let db_mode = std::fs::metadata(bark_dir.join("db.sqlite"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(db_mode, 0o600, "db file is 0600");
            let marker_mode = std::fs::metadata(bark_dir.join("spc-initialized"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(marker_mode, 0o600, "completion marker is 0600");
        }
        // The creation-intent marker is cleaned up on completion.
        assert!(
            !bark_dir.join("spc-creating").exists(),
            "the creation marker is removed after a completed initialization"
        );
    }

    #[tokio::test]
    async fn different_child_seed_is_refused_with_bytes_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let first = BarkStore::open(dir.path(), Network::Regtest, fingerprint(3))
            .await
            .unwrap();
        let db = first.db_path().to_path_buf();
        drop(first);
        let before = sha256_of(&db);

        let err = BarkStore::open(dir.path(), Network::Regtest, fingerprint(4))
            .await
            .unwrap_err();
        assert!(
            matches!(err, BarkStoreError::FingerprintMismatch { .. }),
            "different child seed must be refused: {err}"
        );
        assert_eq!(
            sha256_of(&db),
            before,
            "a refused open leaves the database bytes untouched"
        );
        // ...and the original child seed still opens.
        BarkStore::open(dir.path(), Network::Regtest, fingerprint(3))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn different_network_is_refused_with_bytes_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let first = BarkStore::open(dir.path(), Network::Regtest, fingerprint(5))
            .await
            .unwrap();
        let db = first.db_path().to_path_buf();
        drop(first);
        let before = sha256_of(&db);

        let err = BarkStore::open(dir.path(), Network::Bitcoin, fingerprint(5))
            .await
            .unwrap_err();
        assert!(
            matches!(err, BarkStoreError::NetworkMismatch { .. }),
            "different network must be refused: {err}"
        );
        assert_eq!(sha256_of(&db), before, "bytes untouched on refusal");
    }

    #[tokio::test]
    async fn datadir_loss_deterministically_reinitializes_the_same_child() {
        let dir = tempfile::tempdir().unwrap();
        let fp = fingerprint(6);
        BarkStore::open(dir.path(), Network::Regtest, fp)
            .await
            .unwrap();
        // "Datadir loss": the whole bark directory is gone.
        std::fs::remove_dir_all(dir.path().join("bark")).unwrap();
        let store = BarkStore::open(dir.path(), Network::Regtest, fp)
            .await
            .unwrap();
        assert!(store.is_fresh(), "a lost datadir is a fresh state");
        // The same child seed deterministically reinitializes the same
        // fingerprint (nothing was regenerated - the seed is the input).
        let props = store.properties().await.unwrap().unwrap();
        assert_eq!(props.fingerprint, fp);
        assert_eq!(props.network, Network::Regtest);
    }

    #[tokio::test]
    async fn foreign_nonempty_sqlite_is_refused_not_adopted() {
        let dir = tempfile::tempdir().unwrap();
        let bark_dir = dir.path().join("bark");
        std::fs::create_dir_all(&bark_dir).unwrap();
        let db = bark_dir.join("db.sqlite");
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            conn.execute_batch(
                "CREATE TABLE someone_elses (x INTEGER);
                 INSERT INTO someone_elses VALUES (42);",
            )
            .unwrap();
        }
        let before = sha256_of(&db);
        let err = BarkStore::open(dir.path(), Network::Regtest, fingerprint(7))
            .await
            .unwrap_err();
        assert!(
            matches!(err, BarkStoreError::Foreign(_)),
            "a foreign sqlite db must be refused, not migrated: {err}"
        );
        assert_eq!(sha256_of(&db), before, "the foreign db is not mutated");
    }

    #[tokio::test]
    async fn garbage_file_is_corruption_not_fresh() {
        let dir = tempfile::tempdir().unwrap();
        let bark_dir = dir.path().join("bark");
        std::fs::create_dir_all(&bark_dir).unwrap();
        std::fs::write(bark_dir.join("db.sqlite"), b"this is not a sqlite database").unwrap();
        let err = BarkStore::open(dir.path(), Network::Regtest, fingerprint(8))
            .await
            .unwrap_err();
        assert!(
            matches!(err, BarkStoreError::Corrupt(_, _)),
            "garbage bytes are damage, not a fresh state: {err}"
        );
    }

    #[tokio::test]
    async fn marker_without_properties_row_is_damaged_not_fresh() {
        let dir = tempfile::tempdir().unwrap();
        let fp = fingerprint(9);
        BarkStore::open(dir.path(), Network::Regtest, fp)
            .await
            .unwrap();
        // Damage: the properties row is deleted but the marker (and the
        // migrated tables) remain.
        {
            let conn = rusqlite::Connection::open(dir.path().join("bark/db.sqlite")).unwrap();
            conn.execute("DELETE FROM bark_properties", []).unwrap();
        }
        let err = BarkStore::open(dir.path(), Network::Regtest, fp)
            .await
            .unwrap_err();
        assert!(
            matches!(err, BarkStoreError::Corrupt(_, ref msg) if msg.contains("damaged")),
            "a claimed-but-broken initialization is damage, not fresh: {err}"
        );
    }

    #[tokio::test]
    async fn incomplete_first_initialization_is_completed_not_redone() {
        let dir = tempfile::tempdir().unwrap();
        let bark_dir = dir.path().join("bark");
        std::fs::create_dir_all(&bark_dir).unwrap();
        let db = bark_dir.join("db.sqlite");
        // Simulate a crash in our own first-creation flow: the intent
        // marker is written, the stock client ran the migrations, but
        // `init_wallet` never committed.
        {
            let client = SqliteClient::open(&db).unwrap();
            drop(client);
        }
        std::fs::write(bark_dir.join("spc-creating"), b"creating").unwrap();
        let store = BarkStore::open(dir.path(), Network::Regtest, fingerprint(10))
            .await
            .unwrap();
        assert!(store.is_fresh(), "the incomplete initialization completes");
        let props = store.properties().await.unwrap().unwrap();
        assert_eq!(props.fingerprint, fingerprint(10));
        assert!(
            !bark_dir.join("spc-creating").exists(),
            "completion cleans up the intent marker"
        );
        assert!(bark_dir.join("spc-initialized").exists());
    }

    #[tokio::test]
    async fn data_without_identity_metadata_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let bark_dir = dir.path().join("bark");
        std::fs::create_dir_all(&bark_dir).unwrap();
        let db = bark_dir.join("db.sqlite");
        // Migrated tables plus data, but no properties row and no
        // markers: a bark-shaped database this process never claimed.
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            conn.execute_batch(
                "CREATE TABLE bark_vtxo (x INTEGER);
                 CREATE TABLE bark_properties (id TEXT PRIMARY KEY, network TEXT NOT NULL, fingerprint TEXT NOT NULL);
                 INSERT INTO bark_vtxo VALUES (1);",
            )
            .unwrap();
        }
        let err = BarkStore::open(dir.path(), Network::Regtest, fingerprint(11))
            .await
            .unwrap_err();
        assert!(
            matches!(err, BarkStoreError::Corrupt(_, ref msg) if msg.contains("refusing")),
            "data without identity metadata must be refused: {err}"
        );
    }

    /// Fix-r2 (Fable A, regression 1): a destroyed database with the
    /// completion marker still present is partial damage, never fresh.
    /// The refusal happens before any mutating step: no new database
    /// file is created and both markers are left untouched.
    #[tokio::test]
    async fn deleted_db_with_completion_marker_is_corrupt_not_recreated() {
        let dir = tempfile::tempdir().unwrap();
        let fp = fingerprint(12);
        BarkStore::open(dir.path(), Network::Regtest, fp)
            .await
            .unwrap();
        // Partial damage: only the database file is deleted.
        std::fs::remove_file(dir.path().join("bark/db.sqlite")).unwrap();

        let err = BarkStore::open(dir.path(), Network::Regtest, fp)
            .await
            .unwrap_err();
        assert!(
            matches!(err, BarkStoreError::Corrupt(_, ref msg)
                if msg.contains("database missing/empty")),
            "a missing db with the completion marker present is damage: {err}"
        );
        // Nothing was created or mutated by the refusal.
        assert!(
            !dir.path().join("bark/db.sqlite").exists(),
            "no new database file may be created by a refused open"
        );
        assert!(
            dir.path().join("bark/spc-initialized").exists(),
            "the completion marker is untouched"
        );
        assert!(
            !dir.path().join("bark/spc-creating").exists(),
            "no creation-intent marker is written by a refused open"
        );
    }

    /// Fix-r2 (Fable A, regression 2): a truncated (0-byte) database
    /// with the completion marker still present is the same damage.
    /// The file stays 0 bytes and the markers untouched, and a repeated
    /// attempt fails identically (no state was changed by the refusal).
    #[tokio::test]
    async fn truncated_db_with_completion_marker_is_corrupt_and_stays_zero_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let fp = fingerprint(13);
        BarkStore::open(dir.path(), Network::Regtest, fp)
            .await
            .unwrap();
        // Partial damage: the database is truncated to 0 bytes.
        std::fs::write(dir.path().join("bark/db.sqlite"), b"").unwrap();

        for attempt in 1..=2 {
            let err = BarkStore::open(dir.path(), Network::Regtest, fp)
                .await
                .unwrap_err();
            assert!(
                matches!(err, BarkStoreError::Corrupt(_, ref msg)
                    if msg.contains("database missing/empty")),
                "attempt {attempt}: a 0-byte db with the completion marker is damage: {err}"
            );
            assert_eq!(
                std::fs::metadata(dir.path().join("bark/db.sqlite"))
                    .unwrap()
                    .len(),
                0,
                "attempt {attempt}: the file stays 0 bytes"
            );
            assert!(
                dir.path().join("bark/spc-initialized").exists(),
                "attempt {attempt}: the completion marker is untouched"
            );
        }
    }

    /// Fix-r2 (Fable A, negative): a creation-intent marker without a
    /// database file cannot arise from this module's own ordering (the
    /// file is created before the marker) and is refused - no database
    /// is created.
    #[tokio::test]
    async fn creating_marker_without_db_file_is_impossible_and_refused() {
        let dir = tempfile::tempdir().unwrap();
        let bark_dir = dir.path().join("bark");
        std::fs::create_dir_all(&bark_dir).unwrap();
        std::fs::write(bark_dir.join("spc-creating"), b"creating").unwrap();

        let err = BarkStore::open(dir.path(), Network::Regtest, fingerprint(14))
            .await
            .unwrap_err();
        assert!(
            matches!(err, BarkStoreError::Corrupt(_, ref msg)
                if msg.contains("impossible")),
            "a creation marker without a db file is refused: {err}"
        );
        assert!(
            !bark_dir.join("db.sqlite").exists(),
            "no database file may be created by a refused open"
        );
    }

    /// Fix-r2 (Fable A): a 0-byte database with only the creation
    /// marker is this module's own crash window (file created 0-byte,
    /// marker written, crash before the stock client first opened it):
    /// the initialization completes.
    #[tokio::test]
    async fn empty_db_with_creating_marker_is_our_crash_window_and_completes() {
        let dir = tempfile::tempdir().unwrap();
        let bark_dir = dir.path().join("bark");
        std::fs::create_dir_all(&bark_dir).unwrap();
        std::fs::write(bark_dir.join("db.sqlite"), b"").unwrap();
        std::fs::write(bark_dir.join("spc-creating"), b"creating").unwrap();

        let store = BarkStore::open(dir.path(), Network::Regtest, fingerprint(15))
            .await
            .unwrap();
        assert!(store.is_fresh(), "the crashed creation completes");
        let props = store.properties().await.unwrap().unwrap();
        assert_eq!(props.fingerprint, fingerprint(15));
        assert!(!bark_dir.join("spc-creating").exists());
        assert!(bark_dir.join("spc-initialized").exists());
    }
}
