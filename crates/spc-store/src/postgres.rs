//! Postgres settlement store. Only the `payout` role writes these tables
//! (SPEC §1, §5.1, §7.4).

use std::collections::{BTreeMap, BTreeSet};

use sqlx::postgres::{PgPool, PgPoolOptions, PgRow};
use sqlx::Row;
use thiserror::Error;

use crate::models::*;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("database connect timed out after {0:?}")]
    Timeout(std::time::Duration),
    #[error("invalid column value: {0}")]
    Invalid(&'static str),
    #[error("block {0:?} is already settled")]
    AlreadySettled([u8; 32]),
    #[error("block {0:?} was orphaned; it cannot be settled")]
    Orphaned([u8; 32]),
}

impl StoreError {
    /// Whether the error indicates the database schema does not match
    /// what the store expects (missing table, column or type). The
    /// operator observation surface maps this to its closed
    /// `schema_mismatch` code; kept here so consumers never pattern-match
    /// the sqlx error tree themselves.
    pub fn is_schema_mismatch(&self) -> bool {
        match self {
            StoreError::Db(sqlx::Error::ColumnNotFound(_))
            | StoreError::Db(sqlx::Error::ColumnDecode { .. })
            | StoreError::Db(sqlx::Error::TypeNotFound { .. }) => true,
            StoreError::Db(sqlx::Error::Database(d)) => {
                matches!(d.code().as_deref(), Some("42P01" | "42703"))
            }
            _ => false,
        }
    }
}

/// Current UTC time as an RFC3339 string, for DTO timestamps rendered by
/// consumers that do not (and should not) depend on chrono directly.
pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// How many confirmed blocks a bootstrap/republish window carries. Bounds the
/// block-hash dedupe a front seeds and the `LedgerApplied` replay.
pub const APPLIED_BLOCKS_LIMIT: i64 = 1000;

/// One confirmed block's replayable ledger deltas: `(block_hash,
/// [(sp_id, pending_delta)])`.
pub type LedgerEvent = ([u8; 32], Vec<([u8; 32], i64)>);

fn id32(v: &[u8]) -> Result<[u8; 32]> {
    v.try_into()
        .map_err(|_| StoreError::Invalid("expected 32 bytes"))
}

fn sats_i64(v: u64) -> Result<i64> {
    i64::try_from(v).map_err(|_| StoreError::Invalid("sats exceed i64"))
}

fn sats_u64(v: i64) -> Result<u64> {
    u64::try_from(v).map_err(|_| StoreError::Invalid("negative sats"))
}

fn height_i32(v: u32) -> Result<i32> {
    i32::try_from(v).map_err(|_| StoreError::Invalid("height exceeds i32"))
}

fn height_u32(v: i32) -> Result<u32> {
    u32::try_from(v).map_err(|_| StoreError::Invalid("negative height"))
}

/// A Postgres-backed store. Cheap to clone (`PgPool` is an Arc).
#[derive(Clone)]
pub struct Store {
    pool: PgPool,
}

/// One consistent liability snapshot (EXP-OP): pending balance, scheduled
/// Ark component and their sum, from one SQL statement, with the single
/// read timestamp shared by all three fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LiabilitySnapshot {
    pub pending_balance_sats: u64,
    pub ark_due_sats: u64,
    pub total_sats: u64,
    pub as_of: chrono::DateTime<chrono::Utc>,
}

/// The `StoreError::Invalid` message for a liability snapshot whose total
/// cannot be represented (pending + Ark overflows u64). Exported so the
/// operator surface can map it to its closed `snapshot_inconsistent`
/// error code without string matching on private text.
pub const LIABILITY_TOTAL_OVERFLOW: &str = "liability total exceeds u64";

impl Store {
    pub async fn connect(url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(16)
            .connect(url)
            .await?;
        Ok(Self { pool })
    }

    /// Connect with a hard wall-clock bound.
    ///
    /// `PgPoolOptions::connect` retries an unreachable host indefinitely on
    /// some platforms (a TCP `EINPROGRESS` loop), so a front-only process can
    /// hang before it reaches `refresh()`. Startup must turn that into a
    /// terminal error instead; the caller decides whether an absent database
    /// is fatal (`payout`/`api`) or a dev-only empty bootstrap (`front` on
    /// regtest with `allow_unchecked_chain`).
    pub async fn connect_with_timeout(url: &str, timeout: std::time::Duration) -> Result<Self> {
        match tokio::time::timeout(timeout, Self::connect(url)).await {
            Ok(result) => result,
            Err(_) => Err(StoreError::Timeout(timeout)),
        }
    }

    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Run sqlx migrations under an advisory lock (SPEC §9).
    pub async fn migrate(&self) -> Result<()> {
        sqlx::query("SELECT pg_advisory_lock(0x5350434d494752)") // "SPCMIGR"
            .execute(&self.pool)
            .await?;
        let out = sqlx::migrate!("./migrations").run(&self.pool).await;
        let unlock = sqlx::query("SELECT pg_advisory_unlock(0x5350434d494752)")
            .execute(&self.pool)
            .await;
        unlock?;
        out.map_err(|e| StoreError::Db(e.into()))?;
        Ok(())
    }

    /// Registration upsert (SPEC §5.1): replaces `b_scan`, `b_spend` and
    /// `network`, so a real registration overwrites a placeholder row
    /// booked earlier by `insert_identity_if_absent`.
    pub async fn upsert_identity(&self, row: &SpIdentityRow) -> Result<()> {
        sqlx::query(
            "INSERT INTO sp_identity (sp_id, b_scan, b_spend, network)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (sp_id) DO UPDATE SET
                b_scan = EXCLUDED.b_scan,
                b_spend = EXCLUDED.b_spend,
                network = EXCLUDED.network,
                last_seen = now()",
        )
        .bind(&row.sp_id[..])
        .bind(&row.b_scan)
        .bind(&row.b_spend)
        .bind(row.network)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Insert an identity row only when none exists. `payout` books a
    /// clearly-marked placeholder for a miner that appears in a slice
    /// before registering Ark delivery (the slice stream carries only
    /// `sp_id`, but `slice_credit` references `sp_identity`); unlike
    /// `upsert_identity`, this never overwrites a real registration.
    pub async fn insert_identity_if_absent(&self, row: &SpIdentityRow) -> Result<()> {
        sqlx::query(
            "INSERT INTO sp_identity (sp_id, b_scan, b_spend, network)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (sp_id) DO NOTHING",
        )
        .bind(&row.sp_id[..])
        .bind(&row.b_scan)
        .bind(&row.b_spend)
        .bind(row.network)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn identity(&self, sp_id: &[u8; 32]) -> Result<Option<SpIdentityRow>> {
        let row =
            sqlx::query("SELECT sp_id, b_scan, b_spend, network FROM sp_identity WHERE sp_id = $1")
                .bind(&sp_id[..])
                .fetch_optional(&self.pool)
                .await?;
        row.map(|r| {
            let id: Vec<u8> = r.try_get("sp_id")?;
            Ok(SpIdentityRow {
                sp_id: id32(&id)?,
                b_scan: r.try_get("b_scan")?,
                b_spend: r.try_get("b_spend")?,
                network: r.try_get("network")?,
            })
        })
        .transpose()
    }

    pub async fn upsert_ark_delivery(&self, row: &ArkDeliveryRow) -> Result<()> {
        sqlx::query(
            "INSERT INTO ark_delivery
                (sp_id, ark_id, b_ark, mailbox_mode, mailbox_m, mailbox_d,
                 epoch_fallback, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, now())
             ON CONFLICT (sp_id) DO UPDATE SET
                ark_id = EXCLUDED.ark_id,
                b_ark = EXCLUDED.b_ark,
                mailbox_mode = EXCLUDED.mailbox_mode,
                mailbox_m = EXCLUDED.mailbox_m,
                mailbox_d = EXCLUDED.mailbox_d,
                updated_at = now()",
        )
        .bind(&row.sp_id[..])
        .bind(&row.ark_id[..])
        .bind(&row.b_ark)
        .bind(row.mailbox_mode.as_str())
        .bind(&row.mailbox_m)
        .bind(&row.mailbox_d)
        .bind(row.epoch_fallback.as_i16())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn ark_registration_resolution(
        &self,
        sp_id: &[u8; 32],
        request_id: u32,
    ) -> Result<Option<RegistrationResolution>> {
        let row = sqlx::query(
            "SELECT signed_fields_digest, outcome, registration_rev, error_code
             FROM ark_registration_request WHERE sp_id = $1 AND request_id = $2",
        )
        .bind(&sp_id[..])
        .bind(i64::from(request_id))
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            let digest: Vec<u8> = row.try_get("signed_fields_digest")?;
            let outcome: String = row.try_get("outcome")?;
            let rev: i64 = row.try_get("registration_rev")?;
            let outcome = match outcome.as_str() {
                "applied" => RegistrationOutcome::Applied,
                "superseded" => RegistrationOutcome::Superseded,
                "rejected" => RegistrationOutcome::Rejected,
                "mismatch" => RegistrationOutcome::Mismatch,
                "stale" => RegistrationOutcome::Stale,
                _ => return Err(StoreError::Invalid("registration outcome")),
            };
            Ok(RegistrationResolution {
                sp_id: *sp_id,
                request_id,
                signed_fields_digest: id32(&digest)?,
                outcome,
                registration_rev: sats_u64(rev)?,
                rejection_code: if outcome == RegistrationOutcome::Rejected {
                    Some(
                        match row.try_get::<Option<String>, _>("error_code")?.as_deref() {
                            Some("bad-mailbox-fields") => "bad-mailbox-fields",
                            Some("invalid-point") => "invalid-point",
                            Some("bad-signature") => "bad-signature",
                            Some("wrong-ark-server") => "wrong-ark-server",
                            Some("ark-key-is-spend-key") => "ark-key-is-spend-key",
                            _ => "invalid-message",
                        },
                    )
                } else {
                    None
                },
            })
        })
        .transpose()
    }

    pub async fn current_registration_request(&self, sp_id: &[u8; 32]) -> Result<Option<u32>> {
        let row =
            sqlx::query("SELECT last_request_id FROM ark_registration_state WHERE sp_id = $1")
                .bind(&sp_id[..])
                .fetch_optional(&self.pool)
                .await?;
        row.map(|row| {
            let request_id: i64 = row.try_get("last_request_id")?;
            u32::try_from(request_id).map_err(|_| StoreError::Invalid("registration request id"))
        })
        .transpose()
    }

    /// Resolve a validly parsed/signed request that must not change delivery.
    /// Existing journals govern replay fidelity. A first rejection journals
    /// its exact error code without changing current request/delivery state.
    pub async fn reject_registration(
        &self,
        effect: &RegistrationEffect,
        request_id: u32,
        signed_fields_digest: [u8; 32],
        code: &'static str,
    ) -> Result<RegistrationResolution> {
        let sp_id = effect.sp_id();
        if request_id == 0 {
            return Ok(RegistrationResolution {
                sp_id,
                request_id,
                signed_fields_digest,
                outcome: RegistrationOutcome::RequestIdExhausted,
                registration_rev: 0,
                rejection_code: Some("request-id-exhausted"),
            });
        }
        let request_i64 = i64::from(request_id);
        let mut tx = self.pool.begin().await?;
        if let RegistrationEffect::Delivery { identity, .. } = effect {
            sqlx::query(
                "INSERT INTO sp_identity (sp_id, b_scan, b_spend, network)
                 VALUES ($1, $2, $3, $4) ON CONFLICT (sp_id) DO NOTHING",
            )
            .bind(&identity.sp_id[..])
            .bind(&identity.b_scan)
            .bind(&identity.b_spend)
            .bind(identity.network)
            .execute(&mut *tx)
            .await?;
        }
        let identity_exists =
            sqlx::query("SELECT sp_id FROM sp_identity WHERE sp_id = $1 FOR UPDATE")
                .bind(&sp_id[..])
                .fetch_optional(&mut *tx)
                .await?
                .is_some();
        let existing = sqlx::query(
            "SELECT signed_fields_digest, outcome, registration_rev, error_code
             FROM ark_registration_request WHERE sp_id = $1 AND request_id = $2",
        )
        .bind(&sp_id[..])
        .bind(request_i64)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(row) = existing {
            let digest: Vec<u8> = row.try_get("signed_fields_digest")?;
            let rev: i64 = row.try_get("registration_rev")?;
            let outcome: String = row.try_get("outcome")?;
            let stored_code: Option<String> = row.try_get("error_code")?;
            let outcome = if id32(&digest)? != signed_fields_digest {
                RegistrationOutcome::Mismatch
            } else {
                match outcome.as_str() {
                    "applied" => {
                        let last = sqlx::query(
                            "SELECT last_request_id FROM ark_registration_state WHERE sp_id = $1",
                        )
                        .bind(&sp_id[..])
                        .fetch_optional(&mut *tx)
                        .await?;
                        match last {
                            Some(row)
                                if row.try_get::<i64, _>("last_request_id")? == request_i64 =>
                            {
                                RegistrationOutcome::AlreadyApplied
                            }
                            _ => RegistrationOutcome::Superseded,
                        }
                    }
                    "superseded" => RegistrationOutcome::Superseded,
                    "mismatch" => RegistrationOutcome::Mismatch,
                    "stale" => RegistrationOutcome::Stale,
                    "rejected" => RegistrationOutcome::Rejected,
                    _ => return Err(StoreError::Invalid("registration outcome")),
                }
            };
            let rejection_code = match outcome {
                RegistrationOutcome::Rejected => Some(match stored_code.as_deref() {
                    Some("bad-mailbox-fields") => "bad-mailbox-fields",
                    Some("invalid-point") => "invalid-point",
                    Some("invalid-message") => "invalid-message",
                    Some("bad-signature") => "bad-signature",
                    Some("wrong-ark-server") => "wrong-ark-server",
                    Some("ark-key-is-spend-key") => "ark-key-is-spend-key",
                    _ => "invalid-message",
                }),
                RegistrationOutcome::Mismatch => Some("registration-mismatch"),
                RegistrationOutcome::Stale => Some("stale-request"),
                RegistrationOutcome::AlreadyApplied | RegistrationOutcome::Superseded => None,
                RegistrationOutcome::Applied => None,
                RegistrationOutcome::RequestIdExhausted => Some("request-id-exhausted"),
            };
            tx.commit().await?;
            return Ok(RegistrationResolution {
                sp_id,
                request_id,
                signed_fields_digest,
                outcome,
                registration_rev: sats_u64(rev)?,
                rejection_code,
            });
        }

        let state = sqlx::query(
            "SELECT last_request_id, signed_fields_digest, registration_rev
             FROM ark_registration_state WHERE sp_id = $1 FOR UPDATE",
        )
        .bind(&sp_id[..])
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(row) = state {
            let last: i64 = row.try_get("last_request_id")?;
            let digest: Vec<u8> = row.try_get("signed_fields_digest")?;
            let rev: i64 = row.try_get("registration_rev")?;
            if request_i64 < last {
                tx.commit().await?;
                return Ok(RegistrationResolution {
                    sp_id,
                    request_id,
                    signed_fields_digest,
                    outcome: RegistrationOutcome::Stale,
                    registration_rev: sats_u64(rev)?,
                    rejection_code: Some("stale-request"),
                });
            }
            if request_i64 == last && id32(&digest)? != signed_fields_digest {
                tx.commit().await?;
                return Ok(RegistrationResolution {
                    sp_id,
                    request_id,
                    signed_fields_digest,
                    outcome: RegistrationOutcome::Mismatch,
                    registration_rev: sats_u64(rev)?,
                    rejection_code: Some("registration-mismatch"),
                });
            }
        }

        let registration_rev = bump_front_rev_conn(&mut tx).await?;
        if identity_exists {
            sqlx::query(
                "INSERT INTO ark_registration_request
                    (sp_id, request_id, signed_fields_digest, outcome, registration_rev,
                     error_code, created_at)
                 VALUES ($1, $2, $3, 'rejected', $4, $5, now())",
            )
            .bind(&sp_id[..])
            .bind(request_i64)
            .bind(&signed_fields_digest[..])
            .bind(sats_i64(registration_rev)?)
            .bind(code)
            .execute(&mut *tx)
            .await?;
        }
        // Without an identity row no FK-safe journal row exists. Deterministic
        // invalid-message remains terminal on replay; no delivery is created.
        tx.commit().await?;
        Ok(RegistrationResolution {
            sp_id,
            request_id,
            signed_fields_digest,
            outcome: RegistrationOutcome::Rejected,
            registration_rev,
            rejection_code: Some(code),
        })
    }

    /// Resolve and, when newer, apply one v1 registration atomically. The
    /// `sp_identity` row lock serializes first registration and every later
    /// counter for this identity. Terminal replay/stale/mismatch outcomes never
    /// mutate the current delivery, digest, counter or revision.
    pub async fn apply_registration(
        &self,
        effect: RegistrationEffect,
        request_id: u32,
        signed_fields_digest: [u8; 32],
    ) -> Result<RegistrationResolution> {
        let sp_id = effect.sp_id();
        if request_id == 0 {
            return Ok(RegistrationResolution {
                sp_id,
                request_id,
                signed_fields_digest,
                outcome: RegistrationOutcome::RequestIdExhausted,
                registration_rev: 0,
                rejection_code: Some("request-id-exhausted"),
            });
        }

        let request_i64 = i64::from(request_id);
        let mut tx = self.pool.begin().await?;
        if let RegistrationEffect::Delivery { identity, .. } = &effect {
            // Required for FOR UPDATE serialization on the first request. DO
            // NOTHING makes replays/read resolutions side-effect free.
            sqlx::query(
                "INSERT INTO sp_identity (sp_id, b_scan, b_spend, network)
                 VALUES ($1, $2, $3, $4) ON CONFLICT (sp_id) DO NOTHING",
            )
            .bind(&identity.sp_id[..])
            .bind(&identity.b_scan)
            .bind(&identity.b_spend)
            .bind(identity.network)
            .execute(&mut *tx)
            .await?;
        }
        let identity_row = sqlx::query("SELECT sp_id FROM sp_identity WHERE sp_id = $1 FOR UPDATE")
            .bind(&sp_id[..])
            .fetch_optional(&mut *tx)
            .await?;
        if identity_row.is_none() {
            return Err(StoreError::Invalid(
                "epoch registration has no prior identity registration",
            ));
        }

        let state = sqlx::query(
            "SELECT last_request_id, signed_fields_digest, registration_rev
             FROM ark_registration_state WHERE sp_id = $1 FOR UPDATE",
        )
        .bind(&sp_id[..])
        .fetch_optional(&mut *tx)
        .await?;
        let mut current = None;
        if let Some(row) = state {
            let last: i64 = row.try_get("last_request_id")?;
            let digest: Vec<u8> = row.try_get("signed_fields_digest")?;
            let rev: i64 = row.try_get("registration_rev")?;
            current = Some((last, id32(&digest)?, sats_u64(rev)?));
        }

        // A rejection does not advance the current counter. Check the journal
        // first or a later valid request with the same ID collides on its PK.
        // The identity lock remains held, so this lookup and any subsequent
        // insert/new decision are one serial resolution, not two races.
        let existing = sqlx::query(
            "SELECT outcome, signed_fields_digest, registration_rev, error_code
             FROM ark_registration_request WHERE sp_id = $1 AND request_id = $2
             FOR UPDATE",
        )
        .bind(&sp_id[..])
        .bind(request_i64)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(row) = existing {
            let outcome: String = row.try_get("outcome")?;
            let stored_digest: Vec<u8> = row.try_get("signed_fields_digest")?;
            let registration_rev: i64 = row.try_get("registration_rev")?;
            let stored_code: Option<String> = row.try_get("error_code")?;
            let outcome = if id32(&stored_digest)? != signed_fields_digest {
                RegistrationOutcome::Mismatch
            } else {
                match outcome.as_str() {
                    "applied" => match current.as_ref() {
                        Some((last, _, _)) if *last == request_i64 => {
                            RegistrationOutcome::AlreadyApplied
                        }
                        _ => RegistrationOutcome::Superseded,
                    },
                    "superseded" => RegistrationOutcome::Superseded,
                    "rejected" => RegistrationOutcome::Rejected,
                    "mismatch" => RegistrationOutcome::Mismatch,
                    "stale" => RegistrationOutcome::Stale,
                    _ => return Err(StoreError::Invalid("registration outcome")),
                }
            };
            let rejection_code = match outcome {
                RegistrationOutcome::Rejected => Some(match stored_code.as_deref() {
                    Some("bad-mailbox-fields") => "bad-mailbox-fields",
                    Some("invalid-point") => "invalid-point",
                    Some("invalid-message") => "invalid-message",
                    Some("bad-signature") => "bad-signature",
                    Some("wrong-ark-server") => "wrong-ark-server",
                    Some("ark-key-is-spend-key") => "ark-key-is-spend-key",
                    _ => "invalid-message",
                }),
                RegistrationOutcome::Mismatch => Some("registration-mismatch"),
                RegistrationOutcome::Stale => Some("stale-request"),
                RegistrationOutcome::AlreadyApplied | RegistrationOutcome::Superseded => None,
                RegistrationOutcome::Applied => None,
                RegistrationOutcome::RequestIdExhausted => Some("request-id-exhausted"),
            };
            tx.commit().await?;
            return Ok(RegistrationResolution {
                sp_id,
                request_id,
                signed_fields_digest,
                outcome,
                registration_rev: sats_u64(registration_rev)?,
                rejection_code,
            });
        }

        let is_new = current
            .as_ref()
            .is_none_or(|(last, _, _)| request_i64 > *last);
        if is_new {
            let registration_rev = bump_front_rev_conn(&mut tx).await?;
            match &effect {
                RegistrationEffect::Delivery { identity, delivery } => {
                    sqlx::query(
                        "UPDATE sp_identity
                         SET b_scan = $2, b_spend = $3, network = $4, last_seen = now()
                         WHERE sp_id = $1",
                    )
                    .bind(&identity.sp_id[..])
                    .bind(&identity.b_scan)
                    .bind(&identity.b_spend)
                    .bind(identity.network)
                    .execute(&mut *tx)
                    .await?;
                    sqlx::query(
                        "INSERT INTO ark_delivery
                            (sp_id, ark_id, b_ark, mailbox_mode, mailbox_m, mailbox_d,
                             epoch_fallback, updated_at)
                         VALUES ($1, $2, $3, $4, $5, $6, $7, now())
                         ON CONFLICT (sp_id) DO UPDATE SET
                            ark_id = EXCLUDED.ark_id,
                            b_ark = EXCLUDED.b_ark,
                            mailbox_mode = EXCLUDED.mailbox_mode,
                            mailbox_m = EXCLUDED.mailbox_m,
                            mailbox_d = EXCLUDED.mailbox_d,
                            epoch_fallback = EXCLUDED.epoch_fallback,
                            updated_at = now()",
                    )
                    .bind(&delivery.sp_id[..])
                    .bind(&delivery.ark_id[..])
                    .bind(&delivery.b_ark)
                    .bind(delivery.mailbox_mode.as_str())
                    .bind(&delivery.mailbox_m)
                    .bind(&delivery.mailbox_d)
                    .bind(delivery.epoch_fallback.as_i16())
                    .execute(&mut *tx)
                    .await?;
                }
                RegistrationEffect::EpochMailboxes { sp_id, entries } => {
                    let mode =
                        sqlx::query("SELECT mailbox_mode FROM ark_delivery WHERE sp_id = $1")
                            .bind(&sp_id[..])
                            .fetch_optional(&mut *tx)
                            .await?;
                    let Some(mode) = mode else {
                        return Err(StoreError::Invalid(
                            "epoch registration has no delivery capability",
                        ));
                    };
                    let mode: String = mode.try_get("mailbox_mode")?;
                    if mode != "epoch_registered" {
                        return Err(StoreError::Invalid(
                            "epoch mailboxes require epoch_registered mode",
                        ));
                    }
                    for entry in entries {
                        if &entry.sp_id != sp_id {
                            return Err(StoreError::Invalid("epoch mailbox identity mismatch"));
                        }
                        sqlx::query(
                            "INSERT INTO ark_epoch_mailbox (sp_id, height, mailbox_mi)
                             VALUES ($1, $2, $3)
                             ON CONFLICT (sp_id, height) DO UPDATE
                                SET mailbox_mi = EXCLUDED.mailbox_mi",
                        )
                        .bind(&entry.sp_id[..])
                        .bind(height_i32(entry.height)?)
                        .bind(&entry.mailbox_mi[..])
                        .execute(&mut *tx)
                        .await?;
                    }
                }
            }
            sqlx::query(
                "INSERT INTO ark_registration_state
                    (sp_id, last_request_id, signed_fields_digest, registration_rev, updated_at)
                 VALUES ($1, $2, $3, $4, now())
                 ON CONFLICT (sp_id) DO UPDATE SET
                    last_request_id = EXCLUDED.last_request_id,
                    signed_fields_digest = EXCLUDED.signed_fields_digest,
                    registration_rev = EXCLUDED.registration_rev,
                    updated_at = now()",
            )
            .bind(&sp_id[..])
            .bind(request_i64)
            .bind(&signed_fields_digest[..])
            .bind(sats_i64(registration_rev)?)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO ark_registration_request
                    (sp_id, request_id, signed_fields_digest, outcome, registration_rev,
                     error_code, created_at)
                 VALUES ($1, $2, $3, 'applied', $4, NULL, now())",
            )
            .bind(&sp_id[..])
            .bind(request_i64)
            .bind(&signed_fields_digest[..])
            .bind(sats_i64(registration_rev)?)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            return Ok(RegistrationResolution {
                sp_id,
                request_id,
                signed_fields_digest,
                outcome: RegistrationOutcome::Applied,
                registration_rev,
                rejection_code: None,
            });
        }

        let Some((last, current_digest, current_rev)) = current else {
            return Err(StoreError::Invalid("registration state after new check"));
        };
        let outcome = if request_i64 == last {
            if current_digest == signed_fields_digest {
                RegistrationOutcome::AlreadyApplied
            } else {
                RegistrationOutcome::Mismatch
            }
        } else if request_i64 < last {
            let journal = sqlx::query(
                "SELECT signed_fields_digest, registration_rev
                 FROM ark_registration_request WHERE sp_id = $1 AND request_id = $2",
            )
            .bind(&sp_id[..])
            .bind(request_i64)
            .fetch_optional(&mut *tx)
            .await?;
            match journal {
                Some(row) => {
                    let digest: Vec<u8> = row.try_get("signed_fields_digest")?;
                    if id32(&digest)? == signed_fields_digest {
                        let rev: i64 = row.try_get("registration_rev")?;
                        tx.commit().await?;
                        return Ok(RegistrationResolution {
                            sp_id,
                            request_id,
                            signed_fields_digest,
                            outcome: RegistrationOutcome::Superseded,
                            registration_rev: sats_u64(rev)?,
                            rejection_code: None,
                        });
                    }
                    RegistrationOutcome::Mismatch
                }
                None => RegistrationOutcome::Stale,
            }
        } else {
            return Err(StoreError::Invalid("unreachable registration ordering"));
        };
        tx.commit().await?;
        Ok(RegistrationResolution {
            sp_id,
            request_id,
            signed_fields_digest,
            outcome,
            registration_rev: current_rev,
            rejection_code: None,
        })
    }

    pub async fn ark_delivery(&self, sp_id: &[u8; 32]) -> Result<Option<ArkDeliveryRow>> {
        let row = sqlx::query(
            "SELECT sp_id, ark_id, b_ark, mailbox_mode, mailbox_m, mailbox_d,
                    epoch_fallback
             FROM ark_delivery WHERE sp_id = $1",
        )
        .bind(&sp_id[..])
        .fetch_optional(&self.pool)
        .await?;
        row.map(ark_delivery_from_row).transpose()
    }

    pub async fn epoch_mailboxes(&self, sp_id: &[u8; 32]) -> Result<Vec<ArkEpochMailboxRow>> {
        let rows = sqlx::query(
            "SELECT sp_id, height, mailbox_mi FROM ark_epoch_mailbox
             WHERE sp_id = $1 ORDER BY height",
        )
        .bind(&sp_id[..])
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|r| {
                let id: Vec<u8> = r.try_get("sp_id")?;
                let height: i32 = r.try_get("height")?;
                let mailbox_mi: Vec<u8> = r.try_get("mailbox_mi")?;
                Ok(ArkEpochMailboxRow {
                    sp_id: id32(&id)?,
                    height: height_u32(height)?,
                    mailbox_mi: mailbox_mi
                        .as_slice()
                        .try_into()
                        .map_err(|_| StoreError::Invalid("epoch mailbox key length"))?,
                })
            })
            .collect()
    }

    pub async fn insert_slice(&self, row: &SliceRow) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO slice
                (slice_id, height, prev_hash, ref_job_id, ref_fees, n_shares,
                 difficulty, root, shares, sealed_at, work_unit_version, work_unit_fingerprint)
             VALUES ($1, $2, $3, $4, $5, $6, $7::numeric, $8, $9, now(), $10, $11)
             ON CONFLICT (slice_id) DO NOTHING",
        )
        .bind(sats_i64(row.slice_id)?)
        .bind(height_i32(row.height)?)
        .bind(&row.prev_hash[..])
        .bind(sats_i64(row.ref_job_id)?)
        .bind(sats_i64(row.ref_fees)?)
        .bind(i32::try_from(row.n_shares).map_err(|_| StoreError::Invalid("n_shares"))?)
        .bind(&row.difficulty)
        .bind(&row.root[..])
        .bind(&row.shares)
        .bind(i16::from(row.work_unit_version.as_u8()))
        .bind(row.work_unit_fingerprint.as_ref().map(|fp| fp.as_slice()))
        .execute(&mut *tx)
        .await?;

        for c in &row.credits {
            sqlx::query(
                "INSERT INTO slice_credit (slice_id, sp_id, diff_sum, diff_fee_sum)
                 VALUES ($1, $2, $3::numeric, $4::numeric)
                 ON CONFLICT (slice_id, sp_id) DO NOTHING",
            )
            .bind(sats_i64(row.slice_id)?)
            .bind(&c.sp_id[..])
            .bind(&c.diff_sum)
            .bind(&c.diff_fee_sum)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn insert_snapshot(&self, row: &SnapshotRow) -> Result<()> {
        sqlx::query(
            "INSERT INTO distribution_snapshot
                (fingerprint, height, a_send_x, distribution_id, entries, high_water,
                 window_target, work_unit_version, work_unit_fingerprint, window_rule)
             VALUES ($1, $2, $3, $4, $5, $6, $7::numeric, $8, $9, $10)
             ON CONFLICT (fingerprint) DO NOTHING",
        )
        .bind(&row.fingerprint[..])
        .bind(height_i32(row.height)?)
        .bind(&row.a_send_x[..])
        .bind(row.distribution_id.map(sats_i64).transpose()?)
        .bind(&row.entries)
        .bind(&row.high_water)
        .bind(row.window_target.as_deref())
        .bind(i16::from(row.work_unit_version.as_u8()))
        .bind(row.work_unit_fingerprint.as_ref().map(|fp| fp.as_slice()))
        .bind(i16::from(row.window_rule.as_u8()))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Load one distribution snapshot by fingerprint. `entries`, `high_water`
    /// and `window_target` are the frozen window metadata settlement
    /// reconstructs from (SPEC §4.3, §4.7, §7.4).
    pub async fn snapshot(&self, fingerprint: &[u8; 32]) -> Result<Option<SnapshotRow>> {
        let row = sqlx::query(
            "SELECT fingerprint, height, a_send_x, distribution_id, entries, high_water,
                    work_unit_version, work_unit_fingerprint, window_rule,
                    window_target::text AS window_target
             FROM distribution_snapshot WHERE fingerprint = $1",
        )
        .bind(&fingerprint[..])
        .fetch_optional(&self.pool)
        .await?;
        row.map(|r| {
            let fingerprint: Vec<u8> = r.try_get("fingerprint")?;
            let height: i32 = r.try_get("height")?;
            let a_send_x: Vec<u8> = r.try_get("a_send_x")?;
            let work_unit_version: i16 = r.try_get("work_unit_version")?;
            let work_unit_fingerprint: Option<Vec<u8>> = r.try_get("work_unit_fingerprint")?;
            let window_rule: i16 = r.try_get("window_rule")?;
            let distribution_id: Option<i64> = r.try_get("distribution_id")?;
            Ok(SnapshotRow {
                fingerprint: id32(&fingerprint)?,
                height: height_u32(height)?,
                a_send_x: id32(&a_send_x)?,
                distribution_id: distribution_id.map(sats_u64).transpose()?,
                entries: r.try_get("entries")?,
                high_water: r.try_get("high_water")?,
                work_unit_version: work_version(work_unit_version)?,
                work_unit_fingerprint: opt_id32(work_unit_fingerprint)?,
                window_rule: window_rule_from_i16(window_rule)?,
                window_target: r.try_get("window_target")?,
            })
        })
        .transpose()
    }

    /// Every sealed slice at or below a single-front high-water mark that is
    /// still stored, newest first, with its identity aggregates (SPEC §4.3,
    /// §7.4).
    ///
    /// The `slice` table has no `front_id` column, so only a single-front
    /// high-water vector can be reconstructed exactly. Callers must reject a
    /// multi-front vector before calling this; passing the finding front's
    /// value against a table that also holds other fronts' `slice_id`
    /// sequences would silently mix windows.
    ///
    /// `limit` bounds the walk; a caller that receives exactly `limit` rows
    /// MUST treat the window as ambiguous unless its selection closed before
    /// the last row.
    pub async fn window_slices_to(&self, high_water: u64, limit: i64) -> Result<Vec<SliceRow>> {
        let rows = sqlx::query(
            "SELECT slice_id, height, prev_hash, ref_job_id, ref_fees, n_shares,
                    difficulty::text AS difficulty, root, shares,
                    work_unit_version, work_unit_fingerprint
             FROM slice WHERE slice_id <= $1
             ORDER BY height DESC, slice_id DESC LIMIT $2",
        )
        .bind(sats_i64(high_water)?)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        let slices = rows
            .into_iter()
            .map(slice_from_row)
            .collect::<Result<Vec<_>>>()?;
        self.attach_credits(slices).await
    }

    /// Attach the `slice_credit` aggregates to already-selected slice rows.
    ///
    /// `slice_from_row` deliberately returns no credits (the header query is
    /// shared and cheap); every caller that hands slices to a window builder
    /// must go through this helper, or the window has no per-identity
    /// difficulty and cannot be reconstructed (SPEC §4.3, §4.7). One query
    /// covers the whole list.
    async fn attach_credits(&self, mut slices: Vec<SliceRow>) -> Result<Vec<SliceRow>> {
        if slices.is_empty() {
            return Ok(slices);
        }

        let ids: Vec<i64> = slices
            .iter()
            .map(|s| {
                i64::try_from(s.slice_id).map_err(|_| StoreError::Invalid("slice_id exceeds i64"))
            })
            .collect::<Result<_>>()?;
        let credit_rows = sqlx::query(
            "SELECT slice_id, sp_id, diff_sum::text AS diff_sum,
                    diff_fee_sum::text AS diff_fee_sum
             FROM slice_credit WHERE slice_id = ANY($1)",
        )
        .bind(ids.as_slice())
        .fetch_all(&self.pool)
        .await?;

        let mut by_slice = credits_by_slice(credit_rows)?;
        for s in &mut slices {
            s.credits = by_slice.remove(&s.slice_id).unwrap_or_default();
        }
        Ok(slices)
    }

    /// The whole pending ledger, keyed by `sp_id` (SPEC §4.6, §7.4).
    pub async fn pending_map(&self) -> Result<BTreeMap<[u8; 32], u64>> {
        let rows = sqlx::query("SELECT sp_id, sats FROM pending_balance")
            .fetch_all(&self.pool)
            .await?;
        let mut out = BTreeMap::new();
        for r in rows {
            let id: Vec<u8> = r.try_get("sp_id")?;
            let sats: i64 = r.try_get("sats")?;
            out.insert(id32(&id)?, sats_u64(sats)?);
        }
        Ok(out)
    }

    /// Which of `sp_ids` have an `ark_delivery` row, i.e. are Ark-capable at
    /// this instant (SPEC §4.6(f), §11.1). Settlement re-decides the rail
    /// from this set, never from the block event's rail.
    pub async fn ark_delivery_ids(&self, sp_ids: &[[u8; 32]]) -> Result<BTreeSet<[u8; 32]>> {
        if sp_ids.is_empty() {
            return Ok(BTreeSet::new());
        }
        let ids: Vec<Vec<u8>> = sp_ids.iter().map(|id| id.to_vec()).collect();
        let rows = sqlx::query("SELECT sp_id FROM ark_delivery WHERE sp_id = ANY($1::bytea[])")
            .bind(ids.as_slice())
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(|r| {
                let id: Vec<u8> = r.try_get("sp_id")?;
                id32(&id)
            })
            .collect()
    }

    /// Apply one settlement transaction. Re-delivery is a no-op.
    ///
    /// Fail-closed on the schema invariant (SPEC §5.1/§7.4, review 4.5): a
    /// block without a distribution fingerprint can never be booked
    /// `confirmed` — the caller is expected to pass `Pending`, and the store
    /// coerces a stray `Confirmed`+NULL request to `Pending` here as well.
    /// Ledger rows are written only for a confirmed booking; `pending` and
    /// `orphaned` bookings record just the `block_found` row.
    pub async fn settle(&self, s: &Settlement, confirmation_depth: u32) -> Result<Option<u64>> {
        let status = match (s.status, s.fingerprint) {
            (BlockStatus::Confirmed, None) => BlockStatus::Pending,
            (st, _) => st,
        };
        let book_ledger = status == BlockStatus::Confirmed;
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(1397769025)")
            .execute(&mut *tx)
            .await?;

        let existing =
            sqlx::query("SELECT status FROM block_found WHERE block_hash = $1 FOR UPDATE")
                .bind(&s.block_hash[..])
                .fetch_optional(&mut *tx)
                .await?;

        match existing {
            Some(row) => {
                let existing_status: String = row.try_get("status")?;
                match existing_status.as_str() {
                    "confirmed" => {
                        tx.rollback().await?;
                        return Err(StoreError::AlreadySettled(s.block_hash));
                    }
                    "orphaned" => {
                        tx.rollback().await?;
                        return Err(StoreError::Orphaned(s.block_hash));
                    }
                    _ => {
                        sqlx::query(
                            "UPDATE block_found
                             SET fingerprint = COALESCE($2, fingerprint),
                                 status = $3,
                                 settled_at = now(),
                                 ledger_published = CASE
                                     WHEN $3 = 'confirmed' THEN false
                                     ELSE ledger_published
                                 END
                             WHERE block_hash = $1",
                        )
                        .bind(&s.block_hash[..])
                        .bind(s.fingerprint.as_ref().map(|f| &f[..]))
                        .bind(status.as_str())
                        .execute(&mut *tx)
                        .await?;
                    }
                }
            }
            None => {
                sqlx::query(
                    "INSERT INTO block_found
                        (block_hash, height, fingerprint, status, settled_at,
                         ledger_published)
                     VALUES ($1, $2, $3, $4, now(), $5)",
                )
                .bind(&s.block_hash[..])
                .bind(height_i32(s.height)?)
                .bind(s.fingerprint.as_ref().map(|f| &f[..]))
                .bind(status.as_str())
                // A confirmed booking is unpublished until the XADD succeeds.
                .bind(status != BlockStatus::Confirmed)
                .execute(&mut *tx)
                .await?;
            }
        }

        let due_height = s.height.saturating_add(confirmation_depth);
        if book_ledger {
            crate::economic::book(&mut tx, s).await?;
        }
        for row in if book_ledger { &s.ledger[..] } else { &[][..] } {
            let res = sqlx::query(
                "INSERT INTO ledger_history
                    (block_hash, sp_id, rail, paid_sats, ark_due_sats, forfeited_sats,
                     pending_delta)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)
                 ON CONFLICT (block_hash, sp_id) DO NOTHING",
            )
            .bind(&s.block_hash[..])
            .bind(&row.sp_id[..])
            .bind(row.rail.as_str())
            .bind(sats_i64(row.paid_sats)?)
            .bind(sats_i64(row.ark_due_sats)?)
            .bind(sats_i64(row.forfeited_sats)?)
            .bind(row.pending_delta)
            .execute(&mut *tx)
            .await?;

            if res.rows_affected() == 0 {
                continue;
            }

            if row.pending_delta != 0 {
                sqlx::query(
                    "INSERT INTO pending_balance (sp_id, sats, updated_at)
                     VALUES ($1, 0, now()) ON CONFLICT (sp_id) DO NOTHING",
                )
                .bind(&row.sp_id[..])
                .execute(&mut *tx)
                .await?;
                // A negative INSERT fails CHECK(sats >= 0) before ON CONFLICT
                // can consume prior pending. Update the locked balance instead.
                sqlx::query("UPDATE pending_balance SET sats = sats + $2, updated_at = now() WHERE sp_id = $1")
                .bind(&row.sp_id[..])
                .bind(row.pending_delta)
                .execute(&mut *tx)
                .await?;
            }

            if row.ark_due_sats > 0 {
                sqlx::query(
                    "INSERT INTO ark_payout
                        (block_hash, sp_id, sats, due_height, status)
                     VALUES ($1, $2, $3, $4, 'scheduled')
                     ON CONFLICT (block_hash, sp_id) DO NOTHING",
                )
                .bind(&s.block_hash[..])
                .bind(&row.sp_id[..])
                .bind(sats_i64(row.ark_due_sats)?)
                .bind(height_i32(due_height)?)
                .execute(&mut *tx)
                .await?;
            }
        }

        // The revision commits with the ledger booking, so a crash cannot
        // leave `pending_balance` advanced while the watermark still points
        // at an older event (SPEC §7.4). Orphan/pending bookings publish no
        // `LedgerApplied` and need no rev.
        let rev = if book_ledger {
            crate::economic::check_balances(&mut tx).await?;
            Some(bump_front_rev_conn(&mut tx).await?)
        } else {
            None
        };
        tx.commit().await?;
        Ok(rev)
    }

    /// Mark a block orphaned and reverse anything already booked.
    pub async fn orphan(&self, block_hash: &[u8; 32]) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(1397769025)")
            .execute(&mut *tx)
            .await?;
        Self::orphan_conn(&mut tx, block_hash).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Caller holds the accounting advisory lock. Aggregate cancellation uses
    /// this same reversal after releasing its reservation, in one transaction.
    pub(crate) async fn orphan_conn(
        tx: &mut sqlx::PgConnection,
        block_hash: &[u8; 32],
    ) -> Result<()> {
        let economic: Option<String> = sqlx::query_scalar(
            "SELECT lifecycle FROM economic_block WHERE block_hash=$1 FOR UPDATE",
        )
        .bind(&block_hash[..])
        .fetch_optional(&mut *tx)
        .await?;
        if economic.as_deref() == Some("cancelled") {
            return Ok(());
        }
        if economic.as_deref() == Some("unbooked") {
            let rev = crate::economic::bump(&mut *tx).await?;
            sqlx::query(
                "UPDATE economic_block SET lifecycle='cancelled',revision=$2 WHERE block_hash=$1",
            )
            .bind(&block_hash[..])
            .bind(rev as i64)
            .execute(&mut *tx)
            .await?;
            sqlx::query("UPDATE block_found SET status='orphaned' WHERE block_hash=$1")
                .bind(&block_hash[..])
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE economic_state SET complete=false,reason='unbooked reorg reconciliation' WHERE id=1").execute(&mut *tx).await?;
            crate::economic::check_balances(&mut *tx).await?;
            return Ok(());
        }
        let aggregates=sqlx::query("SELECT DISTINCT a.order_id,a.state FROM ark_aggregate a JOIN ark_aggregate_member m USING(order_id) WHERE m.block_hash=$1 AND m.reserved")
            .bind(&block_hash[..]).fetch_all(&mut *tx).await?;
        if !aggregates.is_empty() {
            sqlx::query(
                "UPDATE ark_aggregate_member SET orphaned=true WHERE block_hash=$1 AND reserved",
            )
            .bind(&block_hash[..])
            .execute(&mut *tx)
            .await?;
            for aggregate in aggregates {
                let id: Vec<u8> = aggregate.try_get("order_id")?;
                if aggregate.get::<String, _>("state") == "sent" {
                    sqlx::query("UPDATE ark_aggregate SET pool_loss_sats=(SELECT COALESCE(sum(sats),0) FROM ark_aggregate_member WHERE order_id=$1 AND orphaned),revision=revision+1 WHERE order_id=$1")
                        .bind(&id).execute(&mut *tx).await?;
                } else {
                    sqlx::query("UPDATE ark_aggregate SET state='blocked_unknown',revision=revision+1 WHERE order_id=$1 AND state<>'cancelled'")
                        .bind(&id).execute(&mut *tx).await?;
                }
            }
            // Preserve only these members; unrelated reversible identities
            // below must still be reversed atomically in this transaction.
        }
        let block = sqlx::query("SELECT status FROM block_found WHERE block_hash = $1 FOR UPDATE")
            .bind(&block_hash[..])
            .fetch_optional(&mut *tx)
            .await?;
        if block.is_none() {
            return Ok(());
        }
        // Sent Ark value is irreversible and must remain as evidence of the
        // pool's loss, never deleted as if the liability had never been sent.
        let rows = sqlx::query(
            "SELECT lh.sp_id, lh.rail,
                    lh.pending_delta + CASE WHEN ap.status = 'failed_to_pending' THEN ap.sats ELSE 0 END AS pending_delta
             FROM ledger_history lh LEFT JOIN ark_payout ap
               ON ap.block_hash = lh.block_hash AND ap.sp_id = lh.sp_id
              WHERE lh.block_hash = $1
                AND COALESCE(ap.status,'') <> 'sent'
                AND NOT EXISTS(SELECT 1 FROM ark_aggregate_member m WHERE m.block_hash=lh.block_hash AND m.sp_id=lh.sp_id AND m.reserved)",
        )
        .bind(&block_hash[..])
        .fetch_all(&mut *tx)
        .await?;

        for row in rows {
            let sp_id: Vec<u8> = row.try_get("sp_id")?;
            let delta: i64 = row.try_get("pending_delta")?;
            if delta != 0 {
                sqlx::query(
                    "UPDATE pending_balance SET sats = sats - $2, updated_at = now()
                     WHERE sp_id = $1",
                )
                .bind(&sp_id)
                .bind(delta)
                .execute(&mut *tx)
                .await?;
            }
            sqlx::query("DELETE FROM ark_payout WHERE block_hash=$1 AND sp_id=$2")
                .bind(&block_hash[..])
                .bind(&sp_id)
                .execute(&mut *tx)
                .await?;
            sqlx::query("DELETE FROM ledger_history WHERE block_hash=$1 AND sp_id=$2")
                .bind(&block_hash[..])
                .bind(&sp_id)
                .execute(&mut *tx)
                .await?;
        }

        sqlx::query("UPDATE block_found SET status = 'orphaned' WHERE block_hash = $1")
            .bind(&block_hash[..])
            .execute(&mut *tx)
            .await?;
        if economic.is_some() {
            let rev = crate::economic::bump(&mut *tx).await?;
            let held: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM ledger_history WHERE block_hash=$1)",
            )
            .bind(&block_hash[..])
            .fetch_one(&mut *tx)
            .await?;
            sqlx::query("UPDATE economic_block SET lifecycle=$3,revision=$2 WHERE block_hash=$1")
                .bind(&block_hash[..])
                .bind(rev as i64)
                .bind(if held { "loss_held" } else { "cancelled" })
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE economic_state SET complete=false,reason='booked reorg reconciliation' WHERE id=1").execute(&mut *tx).await?;
            crate::economic::check_balances(&mut *tx).await?;
        }
        Ok(())
    }

    /// Scrub attribution data per SPEC §5.1. Snapshot entries are kept while
    /// any `ark_payout` of the block is `scheduled` or `failed_to_pending`
    /// (review 4.3: a failed send can still be retried and re-sent).
    pub async fn scrub(
        &self,
        tip_height: u32,
        retain_windows_v1: u32,
        retain_windows_v2: u32,
        scrub_after: u32,
    ) -> Result<()> {
        let retain_below_v1 = tip_height.saturating_sub(retain_windows_v1);
        let retain_below_v2 = tip_height.saturating_sub(retain_windows_v2);
        let scrub_below = tip_height.saturating_sub(scrub_after);
        sqlx::query("UPDATE economic_block eb SET raw_event=NULL WHERE eb.height<$1 AND eb.lifecycle='booked'
            AND NOT EXISTS(SELECT 1 FROM ark_payout ap WHERE ap.block_hash=eb.block_hash AND ap.status IN ('scheduled','failed_to_pending'))")
            .bind(height_i32(scrub_below)?).execute(&self.pool).await?;

        // Select each row's horizon by its stored version. In particular, a
        // payout started under v1 must not erase v2 credits early, and a payout
        // started under v2 must not extend identity-bearing v1 retention.
        sqlx::query(
            "DELETE FROM slice_credit sc USING slice s
             WHERE sc.slice_id = s.slice_id
               AND ((s.work_unit_version = 1 AND s.height < $1)
                 OR (s.work_unit_version = 2 AND s.height < $2))",
        )
        .bind(height_i32(retain_below_v1)?)
        .bind(height_i32(retain_below_v2)?)
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "UPDATE distribution_snapshot ds SET entries = '[]'::jsonb
             WHERE ds.height < $1
               AND EXISTS (
                   SELECT 1 FROM block_found bf
                   WHERE bf.fingerprint = ds.fingerprint AND bf.status = 'confirmed')
               AND NOT EXISTS (
                   SELECT 1 FROM block_found bf
                   JOIN ark_payout ap ON ap.block_hash = bf.block_hash
                   WHERE bf.fingerprint = ds.fingerprint
                     AND ap.status IN ('scheduled', 'failed_to_pending'))",
        )
        .bind(height_i32(scrub_below)?)
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "UPDATE ark_payout SET vtxo_id = NULL
             WHERE status = 'sent' AND due_height < $1",
        )
        .bind(height_i32(scrub_below)?)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Everything the pool holds for miners (SPEC §6.1).
    pub async fn liability_sats(&self) -> Result<u64> {
        let row = sqlx::query(
            "SELECT
                COALESCE((SELECT SUM(sats) FROM pending_balance), 0)::bigint AS pending,
                COALESCE((SELECT SUM(sats) FROM ark_payout WHERE status = 'scheduled'), 0)::bigint
                    AS ark",
        )
        .fetch_one(&self.pool)
        .await?;
        let pending: i64 = row.try_get("pending")?;
        let ark: i64 = row.try_get("ark")?;
        Ok(sats_u64(pending)?.saturating_add(sats_u64(ark)?))
    }

    /// One point-in-time liability snapshot for the operator observation
    /// DTO (EXP-OP, contract §4.2): the pending balance, the scheduled-Ark
    /// component and their sum, read by a single SQL statement so a settle
    /// racing the read can never yield a torn pair. The invariant
    /// `total_sats == pending_balance_sats + ark_due_sats` holds by
    /// construction; a sum that cannot be represented as u64 fails closed
    /// with [`LIABILITY_TOTAL_OVERFLOW`] instead of saturating.
    pub async fn liability_snapshot(&self) -> Result<LiabilitySnapshot> {
        let row = sqlx::query(
            "SELECT
                COALESCE((SELECT SUM(sats) FROM pending_balance), 0)::bigint AS pending,
                COALESCE((SELECT SUM(sats) FROM ark_payout WHERE status = 'scheduled'), 0)::bigint
                    AS ark,
                now() AS as_of",
        )
        .fetch_one(&self.pool)
        .await?;
        let pending: i64 = row.try_get("pending")?;
        let ark: i64 = row.try_get("ark")?;
        let as_of: chrono::DateTime<chrono::Utc> = row.try_get("as_of")?;
        let pending_balance_sats = sats_u64(pending)?;
        let ark_due_sats = sats_u64(ark)?;
        let total_sats = pending_balance_sats
            .checked_add(ark_due_sats)
            .ok_or(StoreError::Invalid(LIABILITY_TOTAL_OVERFLOW))?;
        Ok(LiabilitySnapshot {
            pending_balance_sats,
            ark_due_sats,
            total_sats,
            as_of,
        })
    }

    pub async fn pending_balances(&self) -> Result<Vec<([u8; 32], u64)>> {
        let rows = sqlx::query("SELECT sp_id, sats FROM pending_balance ORDER BY sp_id")
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(|r| {
                let id: Vec<u8> = r.try_get("sp_id")?;
                let sats: i64 = r.try_get("sats")?;
                Ok((id32(&id)?, sats_u64(sats)?))
            })
            .collect()
    }

    pub async fn scheduled_ark_payouts(&self) -> Result<Vec<ArkPayoutRow>> {
        let rows = sqlx::query(
            "SELECT block_hash, sp_id, sats, due_height, status, vtxo_id, expiry_height
             FROM ark_payout WHERE status = 'scheduled' ORDER BY due_height, block_hash",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(ark_payout_from_row).collect()
    }

    /// Scheduled `ark_payout` rows joined with their block's frozen
    /// distribution snapshot, so `payout` can rebuild a signed order from the
    /// database alone (SPEC §11.3). Entries are retained while a payout is
    /// `scheduled` (see [`Self::scrub`]), so an empty `dest_id` means the
    /// snapshot never derived one and the row cannot be emitted.
    pub async fn scheduled_ark_emissions(&self) -> Result<Vec<ArkPayoutEmission>> {
        let rows = sqlx::query(
            "SELECT ap.block_hash, bf.height, ap.sp_id, ap.sats, ap.due_height, ds.entries
             FROM ark_payout ap
             JOIN block_found bf ON bf.block_hash = ap.block_hash
             JOIN distribution_snapshot ds ON ds.fingerprint = bf.fingerprint
             WHERE ap.status = 'scheduled'
             ORDER BY ap.due_height, ap.block_hash, ap.sp_id",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let block_hash: Vec<u8> = r.try_get("block_hash")?;
            let sp_id: Vec<u8> = r.try_get("sp_id")?;
            let sats: i64 = r.try_get("sats")?;
            let due_height: i32 = r.try_get("due_height")?;
            let sp_id = id32(&sp_id)?;
            let entries: serde_json::Value = r.try_get("entries")?;
            let entries: Vec<crate::wire::Entry> = serde_json::from_value(entries)
                .map_err(|_| StoreError::Invalid("snapshot entries"))?;
            let entry = entries
                .into_iter()
                .find(|e| e.sp_id == sp_id)
                .ok_or(StoreError::Invalid("missing Ark emission entry"))?;
            let is_remainder = entry.rail == crate::wire::Rail::Coinbase;
            let dest_id = entry.dest_id;
            out.push(ArkPayoutEmission {
                block_hash: id32(&block_hash)?,
                block_height: height_u32(r.try_get("height")?)?,
                is_remainder,
                sp_id,
                sats: sats_u64(sats)?,
                due_height: height_u32(due_height)?,
                dest_id,
            });
        }
        Ok(out)
    }

    pub async fn mark_ark_sent(
        &self,
        block_hash: &[u8; 32],
        sp_id: &[u8; 32],
        vtxo_id: &[u8],
        expiry_height: u32,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(1397769025)")
            .execute(&mut *tx)
            .await?;
        let reserved:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM ark_aggregate_member WHERE block_hash=$1 AND sp_id=$2 AND reserved)")
            .bind(&block_hash[..]).bind(&sp_id[..]).fetch_one(&mut *tx).await?;
        if reserved {
            return Err(StoreError::Invalid(
                "aggregate member cannot accept a legacy Sent receipt",
            ));
        }
        let status: Option<String> =
            sqlx::query_scalar("SELECT status FROM block_found WHERE block_hash = $1 FOR UPDATE")
                .bind(&block_hash[..])
                .fetch_optional(&mut *tx)
                .await?;
        if status.as_deref() != Some("confirmed") {
            return Err(StoreError::Invalid("Ark receipt block is not confirmed"));
        }
        sqlx::query(
            "UPDATE ark_payout
             SET status = 'sent', vtxo_id = $3, expiry_height = $4
             WHERE block_hash = $1 AND sp_id = $2 AND status = 'scheduled'",
        )
        .bind(&block_hash[..])
        .bind(&sp_id[..])
        .bind(vtxo_id)
        .bind(height_i32(expiry_height)?)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// A failed Ark send becomes pending, never forfeit (SPEC §11.3).
    pub async fn mark_ark_failed_to_pending(
        &self,
        block_hash: &[u8; 32],
        sp_id: &[u8; 32],
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(1397769025)")
            .execute(&mut *tx)
            .await?;
        let reserved:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM ark_aggregate_member WHERE block_hash=$1 AND sp_id=$2 AND reserved)")
            .bind(&block_hash[..]).bind(&sp_id[..]).fetch_one(&mut *tx).await?;
        if reserved {
            return Err(StoreError::Invalid(
                "aggregate member cannot be refunded individually",
            ));
        }
        let status: Option<String> =
            sqlx::query_scalar("SELECT status FROM block_found WHERE block_hash = $1 FOR UPDATE")
                .bind(&block_hash[..])
                .fetch_optional(&mut *tx)
                .await?;
        if status.as_deref() != Some("confirmed") {
            return Err(StoreError::Invalid("Ark receipt block is not confirmed"));
        }
        let row = sqlx::query(
            "UPDATE ark_payout SET status = 'failed_to_pending'
             WHERE block_hash = $1 AND sp_id = $2 AND status = 'scheduled'
             RETURNING sats",
        )
        .bind(&block_hash[..])
        .bind(&sp_id[..])
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(row) = row {
            let sats: i64 = row.try_get("sats")?;
            sqlx::query(
                "INSERT INTO pending_balance (sp_id, sats, updated_at)
                 VALUES ($1, $2, now())
                 ON CONFLICT (sp_id) DO UPDATE SET
                    sats = pending_balance.sats + EXCLUDED.sats,
                    updated_at = now()",
            )
            .bind(&sp_id[..])
            .bind(sats)
            .execute(&mut *tx)
            .await?;
            crate::economic::bump(&mut tx).await?;
            crate::economic::check_balances(&mut tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn block_status(&self, block_hash: &[u8; 32]) -> Result<Option<BlockStatus>> {
        let row = sqlx::query("SELECT status FROM block_found WHERE block_hash = $1")
            .bind(&block_hash[..])
            .fetch_optional(&self.pool)
            .await?;
        let Some(row) = row else { return Ok(None) };
        let status: String = row.try_get("status")?;
        Ok(Some(match status.as_str() {
            "pending" => BlockStatus::Pending,
            "confirmed" => BlockStatus::Confirmed,
            "orphaned" => BlockStatus::Orphaned,
            _ => return Err(StoreError::Invalid("block status")),
        }))
    }

    pub async fn block_height(&self, block_hash: &[u8; 32]) -> Result<Option<u32>> {
        let row = sqlx::query("SELECT height FROM block_found WHERE block_hash = $1")
            .bind(&block_hash[..])
            .fetch_optional(&self.pool)
            .await?;
        row.map(|r| {
            let h: i32 = r.try_get("height")?;
            height_u32(h)
        })
        .transpose()
    }

    pub async fn slices_for_height(&self, height: u32) -> Result<Vec<SliceRow>> {
        let rows = sqlx::query(
            "SELECT slice_id, height, prev_hash, ref_job_id, ref_fees, n_shares,
                    difficulty::text AS difficulty, root, shares,
                    work_unit_version, work_unit_fingerprint
             FROM slice WHERE height = $1 ORDER BY slice_id",
        )
        .bind(height_i32(height)?)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(slice_from_row).collect()
    }

    pub async fn recent_blocks(
        &self,
        limit: i64,
    ) -> Result<Vec<(Vec<u8>, i32, String, Option<chrono::DateTime<chrono::Utc>>)>> {
        let rows = sqlx::query(
            "SELECT block_hash, height, status, settled_at
             FROM block_found ORDER BY height DESC LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|r| {
                Ok((
                    r.try_get("block_hash")?,
                    r.try_get("height")?,
                    r.try_get("status")?,
                    r.try_get("settled_at")?,
                ))
            })
            .collect()
    }

    pub async fn pending_for(&self, sp_id: &[u8; 32]) -> Result<u64> {
        let row = sqlx::query("SELECT sats FROM pending_balance WHERE sp_id = $1")
            .bind(&sp_id[..])
            .fetch_optional(&self.pool)
            .await?;
        match row {
            Some(r) => {
                let sats: i64 = r.try_get("sats")?;
                sats_u64(sats)
            }
            None => Ok(0),
        }
    }

    pub async fn ledger_for(
        &self,
        sp_id: &[u8; 32],
        limit: i64,
    ) -> Result<Vec<(Vec<u8>, i32, String, i64, i64, i64, i64)>> {
        let rows = sqlx::query(
            "SELECT l.block_hash, b.height, l.rail, l.paid_sats, l.ark_due_sats,
                    l.forfeited_sats, l.pending_delta
             FROM ledger_history l JOIN block_found b ON b.block_hash = l.block_hash
             WHERE l.sp_id = $1 ORDER BY b.height DESC LIMIT $2",
        )
        .bind(&sp_id[..])
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|r| {
                Ok((
                    r.try_get("block_hash")?,
                    r.try_get("height")?,
                    r.try_get("rail")?,
                    r.try_get("paid_sats")?,
                    r.try_get("ark_due_sats")?,
                    r.try_get("forfeited_sats")?,
                    r.try_get("pending_delta")?,
                ))
            })
            .collect()
    }

    /// Recent sealed slices for the live window tail and type-32 service,
    /// with the same per-identity credits as [`Self::window_slices_to`] so a
    /// restarting front can rebuild its window from `bootstrap` (SPEC §5.3,
    /// WIRE-FORMATS §3.5).
    pub async fn window_tail(&self, limit: i64) -> Result<Vec<SliceRow>> {
        let rows = sqlx::query(
            "SELECT slice_id, height, prev_hash, ref_job_id, ref_fees, n_shares,
                    difficulty::text AS difficulty, root, shares,
                    work_unit_version, work_unit_fingerprint
             FROM slice ORDER BY height DESC, slice_id DESC LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        let slices = rows
            .into_iter()
            .map(slice_from_row)
            .collect::<Result<Vec<_>>>()?;
        self.attach_credits(slices).await
    }

    /// Bump the persisted `spc:front:events` watermark and return the value
    /// to put in the event (SPEC §5.2, §7.4).
    ///
    /// `now_ms` is the candidate; the row is strictly increasing even when
    /// several events are published in the same millisecond or the wall
    /// clock moves backwards.
    pub async fn bump_front_rev(&self) -> Result<u64> {
        let mut tx = self.pool.begin().await?;
        let rev = bump_front_rev_conn(&mut tx).await?;
        tx.commit().await?;
        Ok(rev)
    }

    /// The persisted `spc:front:events` watermark, or 0 when no event has
    /// been published yet (or the migration has not run: a bootstrap-only
    /// front may start before `payout`).
    pub async fn front_rev(&self) -> Result<u64> {
        let mut tx = self.pool.begin().await?;
        let rev = front_rev_conn(&mut tx).await;
        tx.rollback().await?;
        rev
    }

    /// Ledger deltas for the most recent confirmed blocks, oldest first, for
    /// republishing `LedgerApplied` after a lost XADD (SPEC §5.2, §7.4).
    ///
    /// Only non-zero `pending_delta` rows are returned; a block whose ledger
    /// changed no balance needs no delta replay (the front seeds its
    /// block-hash dedupe from `bootstrap.applied_blocks`).
    pub async fn recent_ledger_events(&self, limit: i64) -> Result<Vec<LedgerEvent>> {
        let rows = sqlx::query(
            "WITH recent AS (
                 SELECT block_hash, height FROM block_found
                 WHERE status = 'confirmed'
                 ORDER BY height DESC, block_hash
                 LIMIT $1
             )
             SELECT recent.block_hash, lh.sp_id, lh.pending_delta
             FROM recent
             JOIN ledger_history lh ON lh.block_hash = recent.block_hash
             ORDER BY recent.height ASC, recent.block_hash, lh.sp_id",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        let mut out: Vec<LedgerEvent> = Vec::new();
        for r in rows {
            let delta: i64 = r.try_get("pending_delta")?;
            if delta == 0 {
                continue;
            }
            let block: Vec<u8> = r.try_get("block_hash")?;
            let sp_id: Vec<u8> = r.try_get("sp_id")?;
            let block = id32(&block)?;
            match out.last_mut() {
                Some((last, deltas)) if *last == block => {
                    deltas.push((id32(&sp_id)?, delta));
                }
                _ => out.push((block, vec![(id32(&sp_id)?, delta)])),
            }
        }
        Ok(out)
    }

    /// Whether a confirmed block's `LedgerApplied` has been published.
    pub async fn ledger_published(&self, block_hash: &[u8; 32]) -> Result<Option<bool>> {
        let row = sqlx::query("SELECT ledger_published FROM block_found WHERE block_hash = $1")
            .bind(&block_hash[..])
            .fetch_optional(&self.pool)
            .await?;
        row.map(|r| r.try_get("ledger_published"))
            .transpose()
            .map_err(StoreError::from)
    }

    /// Mark a block's `LedgerApplied` as published. Called only after the
    /// XADD succeeded, so the republish path is bounded to the unseen set.
    pub async fn mark_ledger_published(&self, block_hash: &[u8; 32]) -> Result<()> {
        sqlx::query("UPDATE block_found SET ledger_published = true WHERE block_hash = $1")
            .bind(&block_hash[..])
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Confirmed blocks whose `LedgerApplied` has not yet been published,
    /// oldest first. This is the bounded republish set: a healthy front marks
    /// every block, so the timer only sees events a failed XADD left behind.
    pub async fn unpublished_ledger_events(&self, limit: i64) -> Result<Vec<LedgerEvent>> {
        let rows = sqlx::query(
            "WITH recent AS (
                 SELECT block_hash, height FROM block_found
                 WHERE status = 'confirmed' AND ledger_published = false
                 ORDER BY height ASC, block_hash
                 LIMIT $1
             )
             SELECT recent.block_hash, lh.sp_id, lh.pending_delta
             FROM recent
             JOIN ledger_history lh ON lh.block_hash = recent.block_hash
             ORDER BY recent.height ASC, recent.block_hash, lh.sp_id",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        let mut out: Vec<LedgerEvent> = Vec::new();
        for r in rows {
            let delta: i64 = r.try_get("pending_delta")?;
            if delta == 0 {
                continue;
            }
            let block = id32(&r.try_get::<Vec<u8>, _>("block_hash")?)?;
            let sp_id = id32(&r.try_get::<Vec<u8>, _>("sp_id")?)?;
            match out.last_mut() {
                Some((last, deltas)) if *last == block => deltas.push((sp_id, delta)),
                _ => out.push((block, vec![(sp_id, delta)])),
            }
        }
        Ok(out)
    }

    /// Run a query that returns exactly one BIGINT column in exactly one
    /// row (the `/metrics` scalar counters).
    async fn count_u64(&self, sql: &'static str) -> Result<u64> {
        let row = sqlx::query(sql).fetch_one(&self.pool).await?;
        let n: i64 = row.try_get(0)?;
        sats_u64(n)
    }

    /// Found blocks per settlement status, for `spc_blocks_total{status}`
    /// (read-only, `api` role; SPEC §9).
    pub async fn blocks_by_status(&self) -> Result<Vec<(String, u64)>> {
        let rows = sqlx::query(
            "SELECT status, COUNT(*)::bigint AS n FROM block_found
             GROUP BY status ORDER BY status",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|r| {
                let status: String = r.try_get("status")?;
                let n: i64 = r.try_get("n")?;
                Ok((status, sats_u64(n)?))
            })
            .collect()
    }

    /// Sats booked to the Ark rail and not yet sent, for
    /// `spc_ark_due_sats` (read-only, `api` role; SPEC §9).
    pub async fn ark_due_sats(&self) -> Result<u64> {
        self.count_u64(
            "SELECT COALESCE(SUM(sats), 0)::bigint AS n FROM ark_payout
             WHERE status = 'scheduled'",
        )
        .await
    }

    /// Ark payouts per status, for `spc_ark_payouts_total{status}`
    /// (read-only, `api` role; SPEC §9).
    pub async fn ark_payouts_by_status(&self) -> Result<Vec<(String, u64)>> {
        let rows = sqlx::query(
            "SELECT status, COUNT(*)::bigint AS n FROM ark_payout
             GROUP BY status ORDER BY status",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|r| {
                let status: String = r.try_get("status")?;
                let n: i64 = r.try_get("n")?;
                Ok((status, sats_u64(n)?))
            })
            .collect()
    }

    /// Cumulative forfeited sats, for `spc_forfeited_sats_total`
    /// (read-only, `api` role; SPEC §9, §11.1).
    pub async fn forfeited_sats_total(&self) -> Result<u64> {
        self.count_u64("SELECT COALESCE(SUM(forfeited_sats), 0)::bigint AS n FROM ledger_history")
            .await
    }

    /// Bootstrap snapshot. Every field is read in one `REPEATABLE READ`
    /// transaction, so the pending projection, the applied-block list and
    /// the `front_event_state.rev` watermark describe the same instant
    /// (SPEC §5.2, §7.4). `rev` is 0 when no event has been published.
    pub async fn bootstrap(&self, retain_windows: i64) -> Result<BootstrapSnapshot> {
        let mut tx = self.pool.begin().await?;
        // Must be the first statement of the transaction.
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
            .execute(&mut *tx)
            .await?;

        let pending_rows = sqlx::query("SELECT sp_id, sats FROM pending_balance ORDER BY sp_id")
            .fetch_all(&mut *tx)
            .await?;
        let pending_balance = pending_rows
            .into_iter()
            .map(|r| {
                let id: Vec<u8> = r.try_get("sp_id")?;
                let sats: i64 = r.try_get("sats")?;
                Ok((id32(&id)?, sats_u64(sats)?))
            })
            .collect::<Result<Vec<_>>>()?;

        let delivery_rows = sqlx::query(
            "SELECT sp_id, ark_id, b_ark, mailbox_mode, mailbox_m, mailbox_d,
                    epoch_fallback FROM ark_delivery",
        )
        .fetch_all(&mut *tx)
        .await?;
        let ark_delivery = delivery_rows
            .into_iter()
            .map(ark_delivery_from_row)
            .collect::<Result<Vec<_>>>()?;

        let identity_rows = sqlx::query(
            "SELECT i.sp_id, i.b_scan, i.b_spend, i.network
             FROM sp_identity i JOIN ark_delivery d ON d.sp_id = i.sp_id ORDER BY i.sp_id",
        )
        .fetch_all(&mut *tx)
        .await?;
        let ark_identities = identity_rows
            .into_iter()
            .map(|r| {
                let sp_id: Vec<u8> = r.try_get("sp_id")?;
                Ok(SpIdentityRow {
                    sp_id: id32(&sp_id)?,
                    b_scan: r.try_get("b_scan")?,
                    b_spend: r.try_get("b_spend")?,
                    network: r.try_get("network")?,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        let epoch_rows = sqlx::query(
            "SELECT sp_id, height, mailbox_mi FROM ark_epoch_mailbox ORDER BY sp_id, height",
        )
        .fetch_all(&mut *tx)
        .await?;
        let ark_epoch_mailboxes = epoch_rows
            .into_iter()
            .map(|r| {
                let sp_id: Vec<u8> = r.try_get("sp_id")?;
                let height: i32 = r.try_get("height")?;
                let mailbox_mi: Vec<u8> = r.try_get("mailbox_mi")?;
                Ok(ArkEpochMailboxRow {
                    sp_id: id32(&sp_id)?,
                    height: height_u32(height)?,
                    mailbox_mi: mailbox_mi
                        .as_slice()
                        .try_into()
                        .map_err(|_| StoreError::Invalid("epoch mailbox key length"))?,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        let slice_rows = sqlx::query(
            "SELECT slice_id, height, prev_hash, ref_job_id, ref_fees, n_shares,
                    difficulty::text AS difficulty, root, shares,
                    work_unit_version, work_unit_fingerprint
             FROM slice ORDER BY height DESC, slice_id DESC LIMIT $1",
        )
        .bind(retain_windows)
        .fetch_all(&mut *tx)
        .await?;
        let mut window_tail: Vec<SliceRow> = slice_rows
            .into_iter()
            .map(slice_from_row)
            .collect::<Result<_>>()?;
        if !window_tail.is_empty() {
            let ids: Vec<i64> = window_tail
                .iter()
                .map(|s| {
                    i64::try_from(s.slice_id)
                        .map_err(|_| StoreError::Invalid("slice_id exceeds i64"))
                })
                .collect::<Result<_>>()?;
            let credit_rows = sqlx::query(
                "SELECT slice_id, sp_id, diff_sum::text AS diff_sum,
                        diff_fee_sum::text AS diff_fee_sum
                 FROM slice_credit WHERE slice_id = ANY($1)",
            )
            .bind(ids.as_slice())
            .fetch_all(&mut *tx)
            .await?;
            let mut by_slice = credits_by_slice(credit_rows)?;
            for s in &mut window_tail {
                s.credits = by_slice.remove(&s.slice_id).unwrap_or_default();
            }
        }
        let type32_blobs = window_tail
            .iter()
            .map(|s| (s.slice_id, s.shares.clone()))
            .collect();

        let block_rows = sqlx::query(
            "SELECT block_hash FROM block_found
             WHERE status = 'confirmed'
             ORDER BY height DESC, block_hash
             LIMIT $1",
        )
        .bind(APPLIED_BLOCKS_LIMIT)
        .fetch_all(&mut *tx)
        .await?;
        let applied_blocks = block_rows
            .into_iter()
            .map(|r| {
                let hash: Vec<u8> = r.try_get("block_hash")?;
                id32(&hash)
            })
            .collect::<Result<Vec<_>>>()?;

        let rev = front_rev_conn(&mut tx).await?;
        let economic = Some(crate::economic::load(&mut tx).await?);
        tx.commit().await?;

        Ok(BootstrapSnapshot {
            economic,
            rev,
            applied_blocks,
            pending_balance,
            ark_delivery,
            ark_identities,
            ark_epoch_mailboxes,
            window_tail,
            type32_blobs,
            slices_cursor: None,
            front_events_cursor: None,
        })
    }
}

/// Group `slice_credit` rows by slice id. Shared by the pool-backed
/// `attach_credits` and the repeatable-read `bootstrap` transaction.
fn credits_by_slice(rows: Vec<PgRow>) -> Result<BTreeMap<u64, Vec<SliceCreditRow>>> {
    let mut by_slice: BTreeMap<u64, Vec<SliceCreditRow>> = BTreeMap::new();
    for r in rows {
        let slice_id: i64 = r.try_get("slice_id")?;
        let sp_id: Vec<u8> = r.try_get("sp_id")?;
        by_slice
            .entry(sats_u64(slice_id)?)
            .or_default()
            .push(SliceCreditRow {
                sp_id: id32(&sp_id)?,
                diff_sum: r.try_get("diff_sum")?,
                diff_fee_sum: r.try_get("diff_fee_sum")?,
            });
    }
    Ok(by_slice)
}

/// The watermark upsert, usable inside `settle`'s transaction so the ledger
/// booking and its revision commit together (SPEC §7.4).
async fn bump_front_rev_conn(tx: &mut sqlx::PgConnection) -> Result<u64> {
    let row = sqlx::query(
        "INSERT INTO front_event_state (id, rev)
         VALUES (1, (EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::bigint)
         ON CONFLICT (id) DO UPDATE
            SET rev = GREATEST(front_event_state.rev + 1, EXCLUDED.rev)
         RETURNING rev",
    )
    .fetch_one(&mut *tx)
    .await?;
    let rev: i64 = row.try_get("rev")?;
    u64::try_from(rev).map_err(|_| StoreError::Invalid("negative front rev"))
}

/// Read the watermark inside a transaction. A database that predates the
/// 0002 migration has published no event, so rev 0 is correct; the
/// `to_regclass` probe avoids aborting the surrounding transaction with an
/// undefined-table error.
async fn front_rev_conn(tx: &mut sqlx::PgConnection) -> Result<u64> {
    let exists: bool =
        sqlx::query_scalar("SELECT to_regclass('public.front_event_state') IS NOT NULL")
            .fetch_one(&mut *tx)
            .await?;
    if !exists {
        return Ok(0);
    }
    let row = sqlx::query("SELECT rev FROM front_event_state WHERE id = 1")
        .fetch_optional(&mut *tx)
        .await?;
    match row {
        Some(row) => {
            let rev: i64 = row.try_get("rev")?;
            u64::try_from(rev).map_err(|_| StoreError::Invalid("negative front rev"))
        }
        None => Ok(0),
    }
}

fn ark_delivery_from_row(row: PgRow) -> Result<ArkDeliveryRow> {
    let id: Vec<u8> = row.try_get("sp_id")?;
    let ark: Vec<u8> = row.try_get("ark_id")?;
    let mode: String = row.try_get("mailbox_mode")?;
    Ok(ArkDeliveryRow {
        sp_id: id32(&id)?,
        ark_id: ark
            .as_slice()
            .try_into()
            .map_err(|_| StoreError::Invalid("ark_id length"))?,
        b_ark: row.try_get("b_ark")?,
        mailbox_mode: MailboxMode::parse_str(&mode).ok_or(StoreError::Invalid("mailbox mode"))?,
        mailbox_m: row.try_get("mailbox_m")?,
        mailbox_d: row.try_get("mailbox_d")?,
        epoch_fallback: EpochFallback::from_i16(row.try_get("epoch_fallback")?)
            .ok_or(StoreError::Invalid("epoch fallback"))?,
    })
}

fn slice_from_row(row: PgRow) -> Result<SliceRow> {
    let slice_id: i64 = row.try_get("slice_id")?;
    let height: i32 = row.try_get("height")?;
    let prev_hash: Vec<u8> = row.try_get("prev_hash")?;
    let ref_job_id: i64 = row.try_get("ref_job_id")?;
    let ref_fees: i64 = row.try_get("ref_fees")?;
    let n_shares: i32 = row.try_get("n_shares")?;
    let difficulty: String = row.try_get("difficulty")?;
    let root: Vec<u8> = row.try_get("root")?;
    let shares: Vec<u8> = row.try_get("shares")?;
    let work_unit_version: i16 = row.try_get("work_unit_version")?;
    let work_unit_fingerprint: Option<Vec<u8>> = row.try_get("work_unit_fingerprint")?;
    Ok(SliceRow {
        slice_id: sats_u64(slice_id)?,
        height: height_u32(height)?,
        prev_hash: id32(&prev_hash)?,
        ref_job_id: sats_u64(ref_job_id)?,
        ref_fees: sats_u64(ref_fees)?,
        n_shares: u32::try_from(n_shares).map_err(|_| StoreError::Invalid("n_shares"))?,
        difficulty,
        root: id32(&root)?,
        shares,
        work_unit_version: work_version(work_unit_version)?,
        work_unit_fingerprint: opt_id32(work_unit_fingerprint)?,
        credits: Vec::new(),
    })
}

fn ark_payout_from_row(row: PgRow) -> Result<ArkPayoutRow> {
    let block_hash: Vec<u8> = row.try_get("block_hash")?;
    let sp_id: Vec<u8> = row.try_get("sp_id")?;
    let sats: i64 = row.try_get("sats")?;
    let due_height: i32 = row.try_get("due_height")?;
    let status: String = row.try_get("status")?;
    let expiry_height: Option<i32> = row.try_get("expiry_height")?;
    Ok(ArkPayoutRow {
        block_hash: id32(&block_hash)?,
        sp_id: id32(&sp_id)?,
        sats: sats_u64(sats)?,
        due_height: height_u32(due_height)?,
        status: ArkPayoutStatus::parse_str(&status).ok_or(StoreError::Invalid("ark status"))?,
        vtxo_id: row.try_get("vtxo_id")?,
        expiry_height: expiry_height.map(height_u32).transpose()?,
    })
}

fn work_version(value: i16) -> Result<WorkUnitVersion> {
    let code = u8::try_from(value).map_err(|_| StoreError::Invalid("work_unit_version"))?;
    WorkUnitVersion::from_u8(code).map_err(|_| StoreError::Invalid("work_unit_version"))
}

fn window_rule_from_i16(value: i16) -> Result<WindowRule> {
    let code = u8::try_from(value).map_err(|_| StoreError::Invalid("window_rule"))?;
    WindowRule::from_u8(code).map_err(|_| StoreError::Invalid("window_rule"))
}

fn opt_id32(value: Option<Vec<u8>>) -> Result<Option<[u8; 32]>> {
    value.map(|bytes| id32(&bytes)).transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    use tokio::net::TcpListener;

    /// A TCP endpoint that accepts but never speaks Postgres. The old
    /// `PgPoolOptions::connect` path can retry this indefinitely; startup must
    /// turn it into a bounded error so a front-only process reaches `refresh`.
    #[tokio::test]
    async fn connect_with_timeout_bounds_a_silent_database() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((socket, _)) = listener.accept().await {
                held.push(socket);
            }
        });

        let url = format!("postgres://spc:spc@{addr}/spc");
        let started = Instant::now();
        let result = Store::connect_with_timeout(&url, Duration::from_millis(250)).await;
        let Err(err) = result else {
            panic!("a silent database must not produce a Store");
        };
        assert!(matches!(err, StoreError::Timeout(_)));
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "bounded connect took {:?}",
            started.elapsed()
        );
    }
}
