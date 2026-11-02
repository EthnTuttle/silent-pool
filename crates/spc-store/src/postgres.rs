//! Postgres settlement store. Only the `payout` role writes these tables
//! (SPEC §1, §5.1, §7.4).

use sqlx::postgres::{PgPool, PgPoolOptions, PgRow};
use sqlx::Row;
use thiserror::Error;

use crate::models::*;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("invalid column value: {0}")]
    Invalid(&'static str),
    #[error("block {0:?} is already settled")]
    AlreadySettled([u8; 32]),
    #[error("block {0:?} was orphaned; it cannot be settled")]
    Orphaned([u8; 32]),
}

pub type Result<T> = std::result::Result<T, StoreError>;

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

    pub async fn upsert_identity(&self, row: &SpIdentityRow) -> Result<()> {
        sqlx::query(
            "INSERT INTO sp_identity (sp_id, b_scan, b_spend, network)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (sp_id) DO UPDATE SET last_seen = now()",
        )
        .bind(&row.sp_id[..])
        .bind(&row.b_scan)
        .bind(&row.b_spend)
        .bind(row.network)
        .execute(&self.pool)
        .await?;
        Ok(())
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

    /// Apply one settlement transaction. Re-delivery is a no-op.
    pub async fn settle(&self, s: &Settlement, confirmation_depth: u32) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        let existing = sqlx::query("SELECT status FROM block_found WHERE block_hash = $1")
            .bind(&s.block_hash[..])
            .fetch_optional(&mut *tx)
            .await?;

        match existing {
            Some(row) => {
                let status: String = row.try_get("status")?;
                match status.as_str() {
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
                                 settled_at = now()
                             WHERE block_hash = $1",
                        )
                        .bind(&s.block_hash[..])
                        .bind(s.fingerprint.as_ref().map(|f| &f[..]))
                        .bind(s.status.as_str())
                        .execute(&mut *tx)
                        .await?;
                    }
                }
            }
            None => {
                sqlx::query(
                    "INSERT INTO block_found
                        (block_hash, height, fingerprint, status, settled_at)
                     VALUES ($1, $2, $3, $4, now())",
                )
                .bind(&s.block_hash[..])
                .bind(height_i32(s.height)?)
                .bind(s.fingerprint.as_ref().map(|f| &f[..]))
                .bind(s.status.as_str())
                .execute(&mut *tx)
                .await?;
            }
        }

        let due_height = s.height.saturating_add(confirmation_depth);
        for row in &s.ledger {
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

        tx.commit().await?;
        Ok(())
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

    /// Scrub attribution data per SPEC §5.1.
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
                   WHERE bf.fingerprint = ds.fingerprint AND ap.status = 'scheduled')",
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

    /// Recent sealed slices for the live window tail and type-32 service.
    pub async fn window_tail(&self, limit: i64) -> Result<Vec<SliceRow>> {
        let rows = sqlx::query(
            "SELECT slice_id, height, prev_hash, ref_job_id, ref_fees, n_shares,
                    difficulty::text AS difficulty, root, shares
             FROM slice ORDER BY height DESC, slice_id DESC LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(slice_from_row).collect()
    }

    /// Bootstrap snapshot. `rev` is 0 until the stream supplies a newer one;
    /// fronts apply only newer revs, so this is safe.
    pub async fn bootstrap(&self, retain_windows: i64) -> Result<BootstrapSnapshot> {
        let pending_balance = self.pending_balances().await?;
        let delivery_rows = sqlx::query(
            "SELECT sp_id, ark_id, b_ark, mailbox_mode, mailbox_m, mailbox_d FROM ark_delivery",
        )
        .fetch_all(&self.pool)
        .await?;
        let ark_delivery = delivery_rows
            .into_iter()
            .map(ark_delivery_from_row)
            .collect::<Result<Vec<_>>>()?;
        let window_tail = self.window_tail(retain_windows).await?;
        let type32_blobs = window_tail
            .iter()
            .map(|s| (s.slice_id, s.shares.clone()))
            .collect();

        Ok(BootstrapSnapshot {
            rev: 0,
            pending_balance,
            ark_delivery,
            window_tail,
            type32_blobs,
            slices_cursor: None,
            front_events_cursor: None,
        })
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
