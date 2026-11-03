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
                (sp_id, ark_id, b_ark, mailbox_mode, mailbox_m, mailbox_d, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, now())
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
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn ark_delivery(&self, sp_id: &[u8; 32]) -> Result<Option<ArkDeliveryRow>> {
        let row = sqlx::query(
            "SELECT sp_id, ark_id, b_ark, mailbox_mode, mailbox_m, mailbox_d
             FROM ark_delivery WHERE sp_id = $1",
        )
        .bind(&sp_id[..])
        .fetch_optional(&self.pool)
        .await?;
        row.map(ark_delivery_from_row).transpose()
    }

    pub async fn insert_slice(&self, row: &SliceRow) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO slice
                (slice_id, height, prev_hash, ref_job_id, ref_fees, n_shares,
                 difficulty, root, shares, sealed_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7::numeric, $8, $9, now())
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
                (fingerprint, height, a_send_x, distribution_id, entries, high_water)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (fingerprint) DO NOTHING",
        )
        .bind(&row.fingerprint[..])
        .bind(height_i32(row.height)?)
        .bind(&row.a_send_x[..])
        .bind(row.distribution_id.map(sats_i64).transpose()?)
        .bind(&row.entries)
        .bind(&row.high_water)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Load one distribution snapshot by fingerprint. `entries` and
    /// `high_water` are the frozen window metadata settlement reconstructs
    /// from (SPEC §4.3, §4.7, §7.4).
    pub async fn snapshot(&self, fingerprint: &[u8; 32]) -> Result<Option<SnapshotRow>> {
        let row = sqlx::query(
            "SELECT fingerprint, height, a_send_x, distribution_id, entries, high_water
             FROM distribution_snapshot WHERE fingerprint = $1",
        )
        .bind(&fingerprint[..])
        .fetch_optional(&self.pool)
        .await?;
        row.map(|r| {
            let fingerprint: Vec<u8> = r.try_get("fingerprint")?;
            let height: i32 = r.try_get("height")?;
            let a_send_x: Vec<u8> = r.try_get("a_send_x")?;
            let distribution_id: Option<i64> = r.try_get("distribution_id")?;
            Ok(SnapshotRow {
                fingerprint: id32(&fingerprint)?,
                height: height_u32(height)?,
                a_send_x: id32(&a_send_x)?,
                distribution_id: distribution_id.map(sats_u64).transpose()?,
                entries: r.try_get("entries")?,
                high_water: r.try_get("high_water")?,
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
                    difficulty::text AS difficulty, root, shares
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
        let rows = sqlx::query("SELECT sp_id FROM ark_delivery WHERE sp_id = ANY($1)")
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

        let existing = sqlx::query("SELECT status FROM block_found WHERE block_hash = $1")
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
                     VALUES ($1, $2, now())
                     ON CONFLICT (sp_id) DO UPDATE SET
                        sats = pending_balance.sats + EXCLUDED.sats,
                        updated_at = now()",
                )
                .bind(&row.sp_id[..])
                .bind(row.pending_delta)
                .execute(&mut *tx)
                .await?;
            }

            if row.rail == LedgerRail::Ark {
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
        let rows = sqlx::query(
            "SELECT sp_id, rail, pending_delta FROM ledger_history WHERE block_hash = $1",
        )
        .bind(&block_hash[..])
        .fetch_all(&mut *tx)
        .await?;

        for row in rows {
            let delta: i64 = row.try_get("pending_delta")?;
            if delta != 0 {
                let sp_id: Vec<u8> = row.try_get("sp_id")?;
                sqlx::query(
                    "UPDATE pending_balance SET sats = sats - $2, updated_at = now()
                     WHERE sp_id = $1",
                )
                .bind(&sp_id)
                .bind(delta)
                .execute(&mut *tx)
                .await?;
            }
        }

        sqlx::query("DELETE FROM ark_payout WHERE block_hash = $1")
            .bind(&block_hash[..])
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM ledger_history WHERE block_hash = $1")
            .bind(&block_hash[..])
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE block_found SET status = 'orphaned' WHERE block_hash = $1")
            .bind(&block_hash[..])
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Scrub attribution data per SPEC §5.1. Snapshot entries are kept while
    /// any `ark_payout` of the block is `scheduled` or `failed_to_pending`
    /// (review 4.3: a failed send can still be retried and re-sent).
    pub async fn scrub(
        &self,
        tip_height: u32,
        retain_windows: u32,
        scrub_after: u32,
    ) -> Result<()> {
        let retain_below = tip_height.saturating_sub(retain_windows);
        let scrub_below = tip_height.saturating_sub(scrub_after);

        sqlx::query(
            "DELETE FROM slice_credit sc USING slice s
             WHERE sc.slice_id = s.slice_id AND s.height < $1",
        )
        .bind(height_i32(retain_below)?)
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
            "SELECT ap.block_hash, ap.sp_id, ap.sats, ap.due_height, ds.entries
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
            let dest_id = entries
                .into_iter()
                .find(|e| e.sp_id == sp_id)
                .map(|e| e.dest_id)
                .unwrap_or_default();
            out.push(ArkPayoutEmission {
                block_hash: id32(&block_hash)?,
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
        sqlx::query(
            "UPDATE ark_payout
             SET status = 'sent', vtxo_id = $3, expiry_height = $4
             WHERE block_hash = $1 AND sp_id = $2 AND status = 'scheduled'",
        )
        .bind(&block_hash[..])
        .bind(&sp_id[..])
        .bind(vtxo_id)
        .bind(height_i32(expiry_height)?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// A failed Ark send becomes pending, never forfeit (SPEC §11.3).
    pub async fn mark_ark_failed_to_pending(
        &self,
        block_hash: &[u8; 32],
        sp_id: &[u8; 32],
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
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
                    difficulty::text AS difficulty, root, shares
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
                    difficulty::text AS difficulty, root, shares
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
            "SELECT sp_id, ark_id, b_ark, mailbox_mode, mailbox_m, mailbox_d FROM ark_delivery",
        )
        .fetch_all(&mut *tx)
        .await?;
        let ark_delivery = delivery_rows
            .into_iter()
            .map(ark_delivery_from_row)
            .collect::<Result<Vec<_>>>()?;

        let slice_rows = sqlx::query(
            "SELECT slice_id, height, prev_hash, ref_job_id, ref_fees, n_shares,
                    difficulty::text AS difficulty, root, shares
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
        tx.commit().await?;

        Ok(BootstrapSnapshot {
            rev,
            applied_blocks,
            pending_balance,
            ark_delivery,
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
