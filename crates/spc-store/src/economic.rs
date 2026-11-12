//! Canonical, durable Rule-B reservation boundary, written only by payout.
use crate::{
    models::{LedgerRow, Settlement},
    postgres::{Result, StoreError},
    wire::BlockEvent,
    Store,
};
use spc_accounting::economic::{Consumption, FrozenJob, Lifecycle, Reservation, View};
use sqlx::{PgConnection, Row};
use std::collections::BTreeSet;

fn hash(bytes: Vec<u8>) -> Result<[u8; 32]> {
    bytes
        .try_into()
        .map_err(|_| StoreError::Invalid("economic hash"))
}
fn num(n: i64) -> Result<u64> {
    u64::try_from(n).map_err(|_| StoreError::Invalid("economic negative number"))
}
pub(crate) async fn bump(c: &mut PgConnection) -> Result<u64> {
    let rev: i64 = sqlx::query_scalar(
        "UPDATE economic_state SET revision=revision+1 WHERE id=1 RETURNING revision",
    )
    .fetch_one(c)
    .await?;
    num(rev)
}
pub(crate) async fn load(c: &mut PgConnection) -> Result<View> {
    check_seed(c).await?;
    let state = sqlx::query(
        "SELECT revision,anchor_height,anchor_hash,complete FROM economic_state WHERE id=1",
    )
    .fetch_one(&mut *c)
    .await?;
    let balances = sqlx::query("SELECT sp_id,sats FROM pending_balance ORDER BY sp_id")
        .fetch_all(&mut *c)
        .await?;
    let booked = balances
        .into_iter()
        .map(|r| Ok((hash(r.try_get("sp_id")?)?, num(r.try_get("sats")?)?)))
        .collect::<Result<Vec<_>>>()?;
    let blocks=sqlx::query("SELECT * FROM economic_block WHERE lifecycle IN ('unbooked','loss_held') ORDER BY height,block_hash LIMIT 4097").fetch_all(c).await?;
    if blocks.len() > 4096 {
        return Err(StoreError::Invalid("economic active-set bound"));
    }
    let mut reservations = Vec::new();
    for row in blocks {
        let job: FrozenJob = serde_json::from_value(row.try_get("frozen_job")?)
            .map_err(|_| StoreError::Invalid("economic frozen job"))?;
        reservations.push(Reservation {
            block_hash: hash(row.try_get("block_hash")?)?,
            parent_hash: hash(row.try_get("parent_hash")?)?,
            height: u32::try_from(row.get::<i32, _>("height"))
                .map_err(|_| StoreError::Invalid("economic height"))?,
            payload_digest: hash(row.try_get("payload_digest")?)?,
            job_id: job.job_id,
            generation: job.generation,
            lifecycle: if row.get::<String, _>("lifecycle") == "unbooked" {
                Lifecycle::Unbooked
            } else {
                Lifecycle::LossHeld
            },
            transition: num(row.try_get("transition")?)?,
            revision: num(row.try_get("revision")?)?,
            rows: serde_json::from_value(row.try_get("consumption")?)
                .map_err(|_| StoreError::Invalid("economic consumption"))?,
        });
    }
    let anchor: Option<Vec<u8>> = state.try_get("anchor_hash")?;
    Ok(View {
        version: 1,
        revision: num(state.try_get("revision")?)?,
        anchor_height: u32::try_from(state.get::<i32, _>("anchor_height"))
            .map_err(|_| StoreError::Invalid("economic anchor height"))?,
        anchor_hash: anchor.clone().map(hash).transpose()?.unwrap_or([0; 32]),
        complete: state.get::<bool, _>("complete") && anchor.is_some(),
        booked,
        reservations,
    })
}
fn validate_seed(
    classification: &str,
    revision: i64,
    watermark: i64,
    state: &[u8],
    snapshot: &[u8],
    digest: &[u8],
) -> Result<()> {
    use bitcoin::hashes::{sha256, Hash};
    if classification != "fresh_empty"
        || revision != 0
        || watermark <= 0
        || snapshot.len() > 8 * 1024 * 1024
        || sha256::Hash::hash(snapshot).as_byte_array().as_slice() != digest
    {
        return Err(StoreError::Invalid(
            "economic history checkpoint unverified",
        ));
    }
    let state: serde_json::Value = serde_json::from_slice(state)
        .map_err(|_| StoreError::Invalid("economic seed state encoding"))?;
    let snapshot: serde_json::Value = serde_json::from_slice(snapshot)
        .map_err(|_| StoreError::Invalid("economic seed snapshot encoding"))?;
    if snapshot["schema"] != "spc.economic-checkpoint/1"
        || snapshot["migration_version"] != 11
        || snapshot["complete_from_empty"] != true
        || snapshot["history_start"]["event_id"].as_i64() != Some(watermark)
        || snapshot["history_start"]["after_row"]["complete_from_empty"] != true
        || snapshot["history_start"]["after_row"]["external_checkpoint_required"] != false
        || snapshot["state"] != state
        || state
            != serde_json::json!({"id":1,"revision":0,"anchor_height":0,"anchor_hash":null,"complete":false,"reason":"unreconciled"})
        || [
            "booked_balances",
            "economic_blocks",
            "chain",
            "found",
            "ledger",
            "ark_payouts",
        ]
        .iter()
        .any(|key| snapshot[*key] != serde_json::json!([]))
    {
        return Err(StoreError::Invalid(
            "economic seed does not prove empty monetary history",
        ));
    }
    Ok(())
}
pub(crate) async fn check_seed(c: &mut PgConnection) -> Result<()> {
    let row=sqlx::query("SELECT classification,economic_revision,history_watermark,state_bytes,snapshot_bytes,snapshot_sha256 FROM economic_checkpoint c
        WHERE checkpoint_id=1 AND schema_version=1 AND migration_version=11 AND octet_length(snapshot_bytes)<=8388608
        AND EXISTS(SELECT 1 FROM source_accounting_history h WHERE h.table_name='economic_checkpoint' AND h.operation='INSERT' AND h.after_row-'recorded_at'=to_jsonb(c)-'recorded_at')")
        .fetch_optional(c).await?.ok_or(StoreError::Invalid("economic history checkpoint missing"))?;
    validate_seed(
        &row.get::<String, _>("classification"),
        row.get("economic_revision"),
        row.get("history_watermark"),
        &row.get::<Vec<u8>, _>("state_bytes"),
        &row.get::<Vec<u8>, _>("snapshot_bytes"),
        &row.get::<Vec<u8>, _>("snapshot_sha256"),
    )
}
pub(crate) async fn check_balances(c: &mut PgConnection) -> Result<()> {
    let mut view = load(c).await?;
    view.complete = true;
    view.balances()
        .map_err(|_| StoreError::Invalid("economic reserved exceeds booked"))?;
    Ok(())
}
pub(crate) async fn book(c: &mut PgConnection, s: &Settlement) -> Result<()> {
    let record = sqlx::query(
        "SELECT lifecycle,validated_ledger FROM economic_block WHERE block_hash=$1 FOR UPDATE",
    )
    .bind(&s.block_hash[..])
    .fetch_optional(&mut *c)
    .await?;
    if let Some(record) = record {
        if record.get::<String, _>("lifecycle") != "unbooked" {
            return Err(StoreError::Invalid("economic booking lifecycle"));
        }
        let ledger: Vec<LedgerRow> = serde_json::from_value(record.try_get("validated_ledger")?)
            .map_err(|_| StoreError::Invalid("economic ledger"))?;
        if ledger != s.ledger {
            return Err(StoreError::Invalid(
                "booking differs from validated frozen ledger",
            ));
        }
        let rev = bump(c).await?;
        sqlx::query("UPDATE economic_block SET lifecycle='booked',revision=$2 WHERE block_hash=$1")
            .bind(&s.block_hash[..])
            .bind(rev as i64)
            .execute(c)
            .await?;
    }
    Ok(())
}
impl Store {
    pub async fn economic_lifecycle(&self, block: &[u8; 32]) -> Result<Option<String>> {
        Ok(
            sqlx::query_scalar("SELECT lifecycle FROM economic_block WHERE block_hash=$1")
                .bind(&block[..])
                .fetch_optional(self.pool())
                .await?,
        )
    }
    /// Explicit, freshly chain-verified recanonicalization of a previously
    /// booked mixed block. Restore the COMPLETE original ledger atomically;
    /// retained irreversible rows are verified and never applied a second time.
    pub async fn economic_restore_loss_held(
        &self,
        event: &BlockEvent,
        depth: u32,
        verified_tip: u32,
    ) -> Result<bool> {
        let due = event
            .height
            .checked_add(depth)
            .ok_or(StoreError::Invalid("recanonical depth overflow"))?;
        if verified_tip < due {
            return Err(StoreError::Invalid(
                "loss-held recanonical block not confirmed",
            ));
        }
        let job = event
            .economic
            .as_ref()
            .ok_or(StoreError::Invalid("recanonical frozen provenance missing"))?;
        let digest = event
            .snapshot_id()
            .map_err(|_| StoreError::Invalid("recanonical event encoding"))?;
        let mut tx = self.pool().begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(1397769025)")
            .execute(&mut *tx)
            .await?;
        let old=sqlx::query("SELECT lifecycle,payload_digest,validated_ledger FROM economic_block WHERE block_hash=$1 FOR UPDATE")
            .bind(&event.block_hash[..]).fetch_one(&mut *tx).await?;
        if hash(old.get("payload_digest"))? != digest {
            return Err(StoreError::Invalid("recanonical event conflict"));
        }
        if old.get::<String, _>("lifecycle") != "loss_held" {
            return Ok(false);
        }
        let mut view = load(&mut tx).await?;
        view.complete = true;
        view.at_parent(
            event
                .height
                .checked_sub(1)
                .ok_or(StoreError::Invalid("recanonical genesis"))?,
            job.parent_hash,
        )
        .map_err(|_| StoreError::Invalid("recanonical parent unresolved"))?;
        let ledger: Vec<LedgerRow> = serde_json::from_value(old.get("validated_ledger"))
            .map_err(|_| StoreError::Invalid("recanonical validated ledger"))?;
        let block_status: Option<String> =
            sqlx::query_scalar("SELECT status FROM block_found WHERE block_hash=$1 FOR UPDATE")
                .bind(&event.block_hash[..])
                .fetch_optional(&mut *tx)
                .await?;
        if block_status.as_deref() != Some("orphaned") {
            return Err(StoreError::Invalid("loss-held block status mismatch"));
        }
        let signed =
            |n: u64| i64::try_from(n).map_err(|_| StoreError::Invalid("recanonical amount range"));
        for row in &ledger {
            let retained=sqlx::query("SELECT rail,paid_sats,ark_due_sats,forfeited_sats,pending_delta FROM ledger_history WHERE block_hash=$1 AND sp_id=$2 FOR UPDATE")
                .bind(&event.block_hash[..]).bind(&row.sp_id[..]).fetch_optional(&mut *tx).await?;
            if let Some(retained) = retained {
                if retained.get::<String, _>("rail") != row.rail.as_str()
                    || retained.get::<i64, _>("paid_sats") != signed(row.paid_sats)?
                    || retained.get::<i64, _>("ark_due_sats") != signed(row.ark_due_sats)?
                    || retained.get::<i64, _>("forfeited_sats") != signed(row.forfeited_sats)?
                    || retained.get::<i64, _>("pending_delta") != row.pending_delta
                {
                    return Err(StoreError::Invalid(
                        "retained irreversible ledger differs from frozen effects",
                    ));
                }
                if row.ark_due_sats > 0 {
                    let preserved:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM ark_payout ap WHERE ap.block_hash=$1 AND ap.sp_id=$2 AND ap.sats=$3 AND ap.due_height=$4 AND
                        (ap.status='sent' OR (ap.status='scheduled' AND EXISTS(SELECT 1 FROM ark_aggregate_member m WHERE m.block_hash=ap.block_hash AND m.sp_id=ap.sp_id AND m.reserved))))")
                        .bind(&event.block_hash[..]).bind(&row.sp_id[..]).bind(signed(row.ark_due_sats)?).bind(due as i32).fetch_one(&mut *tx).await?;
                    if !preserved {
                        return Err(StoreError::Invalid(
                            "irreversible Ark provenance unresolved",
                        ));
                    }
                } else {
                    return Err(StoreError::Invalid(
                        "retained row lacks irreversible provenance",
                    ));
                }
                continue;
            }
            sqlx::query("INSERT INTO ledger_history(block_hash,sp_id,rail,paid_sats,ark_due_sats,forfeited_sats,pending_delta) VALUES($1,$2,$3,$4,$5,$6,$7)")
                .bind(&event.block_hash[..]).bind(&row.sp_id[..]).bind(row.rail.as_str()).bind(signed(row.paid_sats)?).bind(signed(row.ark_due_sats)?)
                .bind(signed(row.forfeited_sats)?).bind(row.pending_delta).execute(&mut *tx).await?;
            if row.pending_delta != 0 {
                sqlx::query("INSERT INTO pending_balance(sp_id,sats) VALUES($1,0) ON CONFLICT(sp_id) DO NOTHING")
                    .bind(&row.sp_id[..]).execute(&mut *tx).await?;
                sqlx::query(
                    "UPDATE pending_balance SET sats=sats+$2,updated_at=now() WHERE sp_id=$1",
                )
                .bind(&row.sp_id[..])
                .bind(row.pending_delta)
                .execute(&mut *tx)
                .await?;
            }
            if row.ark_due_sats > 0 {
                sqlx::query("INSERT INTO ark_payout(block_hash,sp_id,sats,due_height,status) VALUES($1,$2,$3,$4,'scheduled')")
                    .bind(&event.block_hash[..]).bind(&row.sp_id[..]).bind(signed(row.ark_due_sats)?).bind(due as i32).execute(&mut *tx).await?;
            }
        }
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM ledger_history WHERE block_hash=$1")
                .bind(&event.block_hash[..])
                .fetch_one(&mut *tx)
                .await?;
        if usize::try_from(count).ok() != Some(ledger.len()) {
            return Err(StoreError::Invalid("recanonical complete ledger coverage"));
        }
        sqlx::query("UPDATE block_found SET status='confirmed',settled_at=now(),ledger_published=false WHERE block_hash=$1").bind(&event.block_hash[..]).execute(&mut *tx).await?;
        sqlx::query("UPDATE ark_aggregate_member SET orphaned=false WHERE block_hash=$1")
            .bind(&event.block_hash[..])
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE ark_aggregate a SET pool_loss_sats=(SELECT COALESCE(sum(m.sats),0) FROM ark_aggregate_member m WHERE m.order_id=a.order_id AND m.orphaned),revision=revision+1
            WHERE a.state='sent' AND EXISTS(SELECT 1 FROM ark_aggregate_member m WHERE m.order_id=a.order_id AND m.block_hash=$1)")
            .bind(&event.block_hash[..]).execute(&mut *tx).await?;
        let rev = bump(&mut tx).await?;
        sqlx::query("UPDATE economic_block SET lifecycle='booked',transition=transition+1,revision=$2 WHERE block_hash=$1")
            .bind(&event.block_hash[..]).bind(rev as i64).execute(&mut *tx).await?;
        check_balances(&mut tx).await?;
        tx.commit().await?;
        Ok(true)
    }
    /// Private collector read: exact checkpoint bytes and snapshot coordinates.
    pub async fn economic_checkpoint_record(&self) -> Result<Option<serde_json::Value>> {
        Ok(sqlx::query_scalar("SELECT to_jsonb(c) FROM economic_checkpoint c WHERE checkpoint_id=1 AND octet_length(snapshot_bytes)<=8388608").fetch_optional(self.pool()).await?)
    }
    pub async fn economic_event_digest(&self, block: &[u8; 32]) -> Result<Option<[u8; 32]>> {
        let value: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT payload_digest FROM economic_block WHERE block_hash=$1")
                .bind(&block[..])
                .fetch_optional(self.pool())
                .await?;
        value.map(hash).transpose()
    }
    pub async fn economic_confirm_anchor(&self, height: u32, hash: [u8; 32]) -> Result<()> {
        let mut tx = self.pool().begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(1397769025)")
            .execute(&mut *tx)
            .await?;
        check_balances(&mut tx).await?;
        let updated=sqlx::query("UPDATE economic_state SET complete=true,reason='' WHERE id=1 AND anchor_height=$1 AND anchor_hash=$2")
            .bind(height as i32).bind(&hash[..]).execute(&mut *tx).await?;
        if updated.rows_affected() != 1 {
            return Err(StoreError::Invalid("economic anchor changed"));
        }
        tx.commit().await?;
        Ok(())
    }
    pub async fn economic_ledger(&self, block: &[u8; 32]) -> Result<Option<Vec<LedgerRow>>> {
        // Recanonicalization reuses the originally validated mined effects too;
        // current registration or window state must never rewrite them.
        let value: Option<serde_json::Value> =
            sqlx::query_scalar("SELECT validated_ledger FROM economic_block WHERE block_hash=$1")
                .bind(&block[..])
                .fetch_optional(self.pool())
                .await?;
        value
            .map(|v| {
                serde_json::from_value(v).map_err(|_| StoreError::Invalid("frozen economic ledger"))
            })
            .transpose()
    }
    pub async fn economic_view(&self) -> Result<View> {
        let mut tx = self.pool().begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        let view = load(&mut tx).await?;
        tx.commit().await?;
        Ok(view)
    }
    pub async fn economic_initialize(&self, genesis: [u8; 32]) -> Result<()> {
        let mut tx = self.pool().begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(1397769025)")
            .execute(&mut *tx)
            .await?;
        check_seed(&mut tx).await?;
        let any: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM block_found) OR EXISTS(SELECT 1 FROM economic_block) OR EXISTS(SELECT 1 FROM pending_balance) OR EXISTS(SELECT 1 FROM ledger_history)",
        )
        .fetch_one(&mut *tx)
        .await?;
        let anchored: bool =
            sqlx::query_scalar("SELECT anchor_hash IS NOT NULL FROM economic_state WHERE id=1")
                .fetch_one(&mut *tx)
                .await?;
        if !anchored && !any {
            let rev = bump(&mut tx).await?;
            sqlx::query("INSERT INTO economic_chain(height,block_hash,parent_hash,pool_block,coinbase_digest,revision) VALUES(0,$1,$2,false,$2,$3)")
                .bind(&genesis[..]).bind(&[0u8;32][..]).bind(rev as i64).execute(&mut *tx).await?;
            sqlx::query(
                "UPDATE economic_state SET anchor_hash=$1,complete=true,reason='' WHERE id=1",
            )
            .bind(&genesis[..])
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }
    pub async fn economic_hold(&self, reason: &str) -> Result<()> {
        sqlx::query("UPDATE economic_state SET complete=false,reason=$1 WHERE id=1")
            .bind(reason)
            .execute(self.pool())
            .await?;
        Ok(())
    }
    /// Fresh canonical validation is a caller obligation. Replay never revives a
    /// tombstone; only reconcile may explicitly request a recanonical transition.
    pub async fn economic_found(
        &self,
        event: &BlockEvent,
        ledger: &[LedgerRow],
        recanonical: bool,
    ) -> Result<bool> {
        let job = event
            .economic
            .as_ref()
            .ok_or(StoreError::Invalid("missing frozen economic provenance"))?;
        let prior = job
            .prior()
            .map_err(|_| StoreError::Invalid("frozen prior invalid"))?;
        let digest = event
            .snapshot_id()
            .map_err(|_| StoreError::Invalid("economic event encoding"))?;
        let mut rows = Vec::new();
        let mut ids = BTreeSet::new();
        for row in ledger {
            if !ids.insert(row.sp_id) {
                return Err(StoreError::Invalid("duplicate economic ledger identity"));
            }
            rows.push(
                Consumption::from_delta(
                    row.sp_id,
                    *prior
                        .get(&row.sp_id)
                        .ok_or(StoreError::Invalid("frozen P missing"))?,
                    row.pending_delta,
                )
                .map_err(|_| StoreError::Invalid("economic delta"))?,
            );
        }
        if rows.len() != prior.len() {
            return Err(StoreError::Invalid("economic prior coverage"));
        }
        let mut tx = self.pool().begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(1397769025)")
            .execute(&mut *tx)
            .await?;
        let old = sqlx::query(
            "SELECT payload_digest,lifecycle,validated_ledger FROM economic_block WHERE block_hash=$1 FOR UPDATE",
        )
        .bind(&event.block_hash[..])
        .fetch_optional(&mut *tx)
        .await?;
        let exists = old.is_some();
        if let Some(old) = old {
            if hash(old.try_get("payload_digest")?)? != digest {
                return Err(StoreError::Invalid("conflicting economic found payload"));
            }
            let stored: Vec<LedgerRow> = serde_json::from_value(old.try_get("validated_ledger")?)
                .map_err(|_| StoreError::Invalid("economic ledger"))?;
            if stored != ledger {
                return Err(StoreError::Invalid("conflicting economic ledger"));
            }
            if !recanonical || old.get::<String, _>("lifecycle") != "cancelled" {
                return Ok(false);
            }
        }
        let mut view = load(&mut tx).await?;
        // A missing next pool event holds descendant readiness, not the
        // already verified parent anchor. Caller proved this child's ancestry.
        view.complete = true;
        view.at_parent(
            event
                .height
                .checked_sub(1)
                .ok_or(StoreError::Invalid("economic block height"))?,
            job.parent_hash,
        )
        .map_err(|_| StoreError::Invalid("economic parent incomplete"))?;
        if job.revision > view.revision {
            return Err(StoreError::Invalid("economic job revision ahead"));
        }
        let balances = view
            .balances()
            .map_err(|_| StoreError::Invalid("economic balances"))?;
        for row in &rows {
            if row.consumed > balances.get(&row.sp_id).map_or(0, |b| b.available) {
                return Err(StoreError::Invalid(
                    "insufficient available pending for reservation",
                ));
            }
        }
        let rev = bump(&mut tx).await?;
        let bytes = event
            .encode()
            .map_err(|_| StoreError::Invalid("economic event"))?;
        if exists {
            sqlx::query("UPDATE economic_block SET lifecycle='unbooked',transition=transition+1,revision=$2 WHERE block_hash=$1").bind(&event.block_hash[..]).bind(rev as i64).execute(&mut *tx).await?;
            sqlx::query("UPDATE block_found SET status='pending',settled_at=NULL WHERE block_hash=$1 AND status='orphaned'").bind(&event.block_hash[..]).execute(&mut *tx).await?;
        } else {
            sqlx::query("INSERT INTO economic_block(block_hash,parent_hash,height,payload_digest,raw_event,frozen_job,validated_ledger,consumption,lifecycle,revision) VALUES($1,$2,$3,$4,$5,$6,$7,$8,'unbooked',$9)")
                .bind(&event.block_hash[..]).bind(&job.parent_hash[..]).bind(event.height as i32).bind(&digest[..]).bind(bytes)
                .bind(serde_json::to_value(job).map_err(|_|StoreError::Invalid("economic job JSON"))?)
                .bind(serde_json::to_value(ledger).map_err(|_|StoreError::Invalid("economic ledger JSON"))?)
                .bind(serde_json::to_value(rows).map_err(|_|StoreError::Invalid("economic consumption JSON"))?).bind(rev as i64).execute(&mut *tx).await?;
        }
        check_balances(&mut tx).await?;
        tx.commit().await?;
        Ok(true)
    }
    pub async fn economic_advance(
        &self,
        height: u32,
        block: [u8; 32],
        parent: [u8; 32],
        pool_block: bool,
        coinbase_digest: [u8; 32],
    ) -> Result<()> {
        let mut tx = self.pool().begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(1397769025)")
            .execute(&mut *tx)
            .await?;
        let mut view = load(&mut tx).await?;
        view.complete = true;
        view.at_parent(
            height
                .checked_sub(1)
                .ok_or(StoreError::Invalid("advance genesis"))?,
            parent,
        )
        .map_err(|_| StoreError::Invalid("economic ancestry gap"))?;
        if pool_block {
            let found:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM economic_block WHERE block_hash=$1 AND parent_hash=$2 AND height=$3 AND lifecycle IN ('unbooked','booked'))")
                .bind(&block[..]).bind(&parent[..]).bind(height as i32).fetch_one(&mut *tx).await?;
            if !found {
                return Err(StoreError::Invalid(
                    "canonical pool block has no validated reservation",
                ));
            }
        }
        let rev = bump(&mut tx).await?;
        sqlx::query("INSERT INTO economic_chain(height,block_hash,parent_hash,pool_block,coinbase_digest,revision) VALUES($1,$2,$3,$4,$5,$6)")
            .bind(height as i32).bind(&block[..]).bind(&parent[..]).bind(pool_block).bind(&coinbase_digest[..]).bind(rev as i64).execute(&mut *tx).await?;
        sqlx::query("UPDATE economic_state SET anchor_height=$1,anchor_hash=$2,complete=true,reason='' WHERE id=1").bind(height as i32).bind(&block[..]).execute(&mut *tx).await?;
        check_balances(&mut tx).await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn economic_chain_hash(&self, height: u32) -> Result<Option<[u8; 32]>> {
        let h: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT block_hash FROM economic_chain WHERE height=$1")
                .bind(height as i32)
                .fetch_optional(self.pool())
                .await?;
        h.map(hash).transpose()
    }
    pub async fn economic_saved_event(&self, block: &[u8; 32]) -> Result<Option<BlockEvent>> {
        let data: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT raw_event FROM economic_block WHERE block_hash=$1")
                .bind(&block[..])
                .fetch_optional(self.pool())
                .await?
                .flatten();
        data.map(|v| {
            BlockEvent::decode(&v).map_err(|_| StoreError::Invalid("saved economic event"))
        })
        .transpose()
    }
    pub async fn economic_rewind(&self, height: u32, block: [u8; 32]) -> Result<()> {
        let mut tx = self.pool().begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(1397769025)")
            .execute(&mut *tx)
            .await?;
        let anchor: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT block_hash FROM economic_chain WHERE height=$1")
                .bind(height as i32)
                .fetch_optional(&mut *tx)
                .await?;
        if anchor.as_deref() != Some(&block[..]) {
            return Err(StoreError::Invalid("unverified economic rewind anchor"));
        }
        let active:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM economic_block WHERE height>$1 AND lifecycle IN ('unbooked','booked'))").bind(height as i32).fetch_one(&mut *tx).await?;
        if active {
            return Err(StoreError::Invalid(
                "dependent economic records not reconciled",
            ));
        }
        sqlx::query("DELETE FROM economic_chain WHERE height>$1")
            .bind(height as i32)
            .execute(&mut *tx)
            .await?;
        bump(&mut tx).await?;
        sqlx::query("UPDATE economic_state SET anchor_height=$1,anchor_hash=$2,complete=true,reason='' WHERE id=1").bind(height as i32).bind(&block[..]).execute(&mut *tx).await?;
        check_balances(&mut tx).await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn economic_displaced(&self, height: u32) -> Result<Vec<[u8; 32]>> {
        let rows:Vec<Vec<u8>>=sqlx::query_scalar("SELECT block_hash FROM economic_block WHERE height>$1 AND lifecycle IN ('unbooked','booked') ORDER BY height DESC,block_hash")
            .bind(height as i32).fetch_all(self.pool()).await?;
        rows.into_iter().map(hash).collect()
    }
}
