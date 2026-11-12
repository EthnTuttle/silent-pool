//! Durable SPc-side aggregate membership. Ark has no access to this store.
use crate::{
    postgres::{Result, StoreError},
    wire::{Entry, Rail},
    Store,
};
use bitcoin::hashes::{sha256, Hash, HashEngine};
use bitcoin::secp256k1::Keypair;
use serde::{Deserialize, Serialize};
use spc_ext::{
    authorization::{order_digest, AuthorizationGrant, AuthorizationRequest},
    ArkSendOrder,
};
use sqlx::Row;
use std::collections::BTreeSet;

pub const MAX_MEMBERS: usize = 2048;

pub struct SentDeliveryProof<'a> {
    pub vtxo_id: &'a [u8],
    pub expiry: u32,
    pub proof: &'a [u8],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregateMember {
    pub block_hash: [u8; 32],
    pub sp_id: [u8; 32],
    pub block_height: u32,
    pub due_height: u32,
    pub sats: u64,
    pub destination: Vec<u8>,
    pub is_remainder: bool,
}

#[derive(Clone, Debug)]
pub struct FrozenAggregate {
    pub server_pubkey: Option<[u8; 33]>,
    pub order: ArkSendOrder,
    pub digest: [u8; 32],
    pub revision: u64,
    pub state: String,
    pub members: Vec<AggregateMember>,
}

fn bytes32(bytes: Vec<u8>) -> Result<[u8; 32]> {
    bytes
        .try_into()
        .map_err(|_| StoreError::Invalid("aggregate hash length"))
}
fn number(v: i64) -> Result<u64> {
    u64::try_from(v).map_err(|_| StoreError::Invalid("negative aggregate number"))
}
fn height(v: i32) -> Result<u32> {
    u32::try_from(v).map_err(|_| StoreError::Invalid("negative aggregate height"))
}
fn signed(v: u64) -> Result<i64> {
    i64::try_from(v).map_err(|_| StoreError::Invalid("aggregate amount overflow"))
}
fn h32(v: u32) -> Result<i32> {
    i32::try_from(v).map_err(|_| StoreError::Invalid("aggregate height overflow"))
}

/// Caps are explicit observations of the real provider/order policy. Unknown
/// limits never become guessed defaults. Fees are pool expense, not a deduction.
#[derive(Clone, Copy, Debug)]
pub struct DeliveryLimits {
    pub min_sats: u64,
    pub max_sats: u64,
    pub max_members: usize,
}

pub fn bounded_members(
    mut rows: Vec<AggregateMember>,
    tip: u32,
    limits: DeliveryLimits,
    min: u64,
    delay: u32,
) -> Result<Vec<AggregateMember>> {
    if limits.min_sats == 0
        || limits.max_sats < limits.min_sats
        || limits.max_members == 0
        || limits.max_members > MAX_MEMBERS
    {
        return Err(StoreError::Invalid(
            "delivery policy unavailable or invalid",
        ));
    }
    rows.sort_by_key(|r| (r.block_height, r.block_hash, r.sp_id));
    rows.retain(|r| r.due_height <= tip);
    let Some(first) = rows.first() else {
        return Ok(vec![]);
    };
    let id = first.sp_id;
    let mut seen = BTreeSet::new();
    if rows
        .iter()
        .any(|r| r.sp_id != id || r.sats == 0 || !seen.insert(r.block_hash))
    {
        return Err(StoreError::Invalid("aggregate membership"));
    }
    // Full sub-base Ark obligations keep their one-row cadence.
    let max_members = if first.is_remainder {
        limits.max_members
    } else {
        1
    };
    let remainder = first.is_remainder;
    let mut selected = Vec::new();
    let mut total = 0u64;
    for row in rows {
        if selected.len() == max_members || row.is_remainder != remainder {
            break;
        }
        let next = total
            .checked_add(row.sats)
            .ok_or(StoreError::Invalid("aggregate sum overflow"))?;
        if next > limits.max_sats {
            break;
        }
        selected.push(row);
        total = next;
    }
    let Some(anchor) = selected.first() else {
        return Ok(vec![]);
    };
    if total < limits.min_sats {
        return Ok(vec![]);
    }
    if remainder && total < min && tip - anchor.due_height < delay {
        return Ok(vec![]);
    }
    Ok(selected)
}

impl Store {
    /// Select by authenticated owner AND opaque order ID before reading bytes.
    /// All lifecycle states are readable; this is evidence, not send authority.
    pub async fn aggregate_order_for(
        &self,
        sp_id: &[u8; 32],
        order_id: &[u8; 16],
        pubkey: Option<&bitcoin::secp256k1::XOnlyPublicKey>,
    ) -> Result<Option<Vec<u8>>> {
        let row=sqlx::query("SELECT CASE WHEN octet_length(a.order_bytes) BETWEEN $3 AND $4 THEN a.order_bytes END AS order_bytes,
            a.order_digest,a.destination,a.sats,a.not_before_height,g.order_digest AS grant_digest
            FROM ark_aggregate a LEFT JOIN ark_send_authorization g USING(order_id)
            WHERE a.sp_id=$1 AND a.order_id=$2")
            .bind(&sp_id[..]).bind(&order_id[..]).bind(ArkSendOrder::MIN_ENCODED_LEN as i32).bind(ArkSendOrder::MAX_ENCODED_LEN as i32)
            .fetch_optional(self.pool()).await?;
        let Some(row) = row else { return Ok(None) };
        let bytes: Vec<u8> = row
            .try_get::<Option<Vec<u8>>, _>("order_bytes")?
            .ok_or(StoreError::Invalid("frozen order byte bound"))?;
        let order = ArkSendOrder::decode(&bytes)
            .map_err(|_| StoreError::Invalid("frozen order encoding"))?;
        let digest =
            order_digest(&order).map_err(|_| StoreError::Invalid("frozen order digest"))?;
        if order.order_id != *order_id
            || order
                .encode()
                .map_err(|_| StoreError::Invalid("frozen order encoding"))?
                != bytes
            || digest != bytes32(row.try_get("order_digest")?)?
            || order.ark_address.as_bytes() != row.get::<Vec<u8>, _>("destination")
            || order.sats != number(row.try_get("sats")?)?
            || order.not_before_height != height(row.try_get("not_before_height")?)?
            || row
                .get::<Option<Vec<u8>>, _>("grant_digest")
                .is_some_and(|g| g != digest)
        {
            return Err(StoreError::Invalid("frozen order binding"));
        }
        order
            .verify(pubkey.ok_or(StoreError::Invalid("order verification key unavailable"))?)
            .map_err(|_| StoreError::Invalid("frozen order signature"))?;
        Ok(Some(bytes))
    }
    pub async fn receipt_cursor(&self, stream: &str) -> Result<String> {
        Ok(
            sqlx::query_scalar("SELECT last_id FROM ark_receipt_cursor WHERE stream=$1")
                .bind(stream)
                .fetch_optional(self.pool())
                .await?
                .unwrap_or_else(|| "0".into()),
        )
    }
    pub async fn set_receipt_cursor(&self, stream: &str, id: &str) -> Result<()> {
        sqlx::query("INSERT INTO ark_receipt_cursor(stream,last_id) VALUES($1,$2) ON CONFLICT(stream) DO UPDATE SET last_id=EXCLUDED.last_id")
            .bind(stream).bind(id).execute(self.pool()).await?;
        Ok(())
    }
    pub async fn record_receipt_disposition(
        &self,
        stream: &str,
        id: &str,
        status: &str,
        reason: &str,
        payload: &[u8],
    ) -> Result<()> {
        let digest = sha256::Hash::hash(payload).to_byte_array();
        let saved=sqlx::query("INSERT INTO ark_receipt_disposition(stream,stream_id,status,reason,payload,payload_sha256,original_len) VALUES($1,$2,$3,$4,$5,$6,$7)
            ON CONFLICT(stream,stream_id) DO UPDATE SET status=EXCLUDED.status,reason=EXCLUDED.reason,attempts=ark_receipt_disposition.attempts+1
            WHERE ark_receipt_disposition.payload_sha256=EXCLUDED.payload_sha256")
            .bind(stream).bind(id).bind(status).bind(reason).bind(&payload[..payload.len().min(4*1024*1024)])
            .bind(&digest[..]).bind(payload.len() as i64).execute(self.pool()).await?;
        if saved.rows_affected() != 1 {
            return Err(StoreError::Invalid(
                "receipt stream ID reused with different payload",
            ));
        }
        Ok(())
    }
    pub async fn aggregate_receipt_for(
        &self,
        sp_id: &[u8; 32],
        order_id: &[u8; 16],
    ) -> Result<Option<Vec<u8>>> {
        let proof:Option<Vec<u8>>=sqlx::query_scalar("SELECT resolution_proof FROM ark_aggregate WHERE sp_id=$1 AND order_id=$2 AND state='sent'")
            .bind(&sp_id[..]).bind(&order_id[..]).fetch_optional(self.pool()).await?.flatten();
        if proof.as_ref().is_some_and(|p| p.len() > 4 * 1024 * 1024) {
            return Err(StoreError::Invalid("delivery proof bound"));
        }
        Ok(proof)
    }
    pub async fn publishable_aggregate_ids(&self) -> Result<Vec<[u8; 16]>> {
        let rows=sqlx::query("SELECT order_id FROM ark_aggregate WHERE state IN ('frozen','published') ORDER BY batch_seq LIMIT 64")
            .fetch_all(self.pool()).await?;
        rows.into_iter()
            .map(|row| {
                row.get::<Vec<u8>, _>("order_id")
                    .try_into()
                    .map_err(|_| StoreError::Invalid("aggregate ID"))
            })
            .collect()
    }
    /// A definitive receipt from the authorized sender. Payout validates the
    /// supported VTXO/wallet evidence before calling; opaque ID alone is never
    /// evidence. The complete receipt bytes are retained for idempotent replay.
    pub async fn resolve_aggregate_sent(
        &self,
        request: &AuthorizationRequest,
        revision: u64,
        sats: u64,
        delivery: SentDeliveryProof<'_>,
        canonical: &BTreeSet<[u8; 32]>,
    ) -> Result<()> {
        let SentDeliveryProof {
            vtxo_id,
            expiry,
            proof,
        } = delivery;
        if vtxo_id.len() != 36 || proof.is_empty() || proof.len() > 4 * 1024 * 1024 {
            return Err(StoreError::Invalid("aggregate sent proof"));
        }
        let mut tx = self.pool().begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(1397769025)")
            .execute(&mut *tx)
            .await?;
        let row=sqlx::query("SELECT state,sats,order_digest,resolution_proof,vtxo_id,expiry_height FROM ark_aggregate WHERE order_id=$1 FOR UPDATE")
            .bind(&request.order_id[..]).fetch_one(&mut *tx).await?;
        if number(row.try_get("sats")?)? != sats
            || bytes32(row.try_get("order_digest")?)? != request.order_digest
        {
            return Err(StoreError::Invalid(
                "aggregate partial or mismatched receipt",
            ));
        }
        let auth = sqlx::query(
            "SELECT nonce,revision,order_digest FROM ark_send_authorization WHERE order_id=$1",
        )
        .bind(&request.order_id[..])
        .fetch_one(&mut *tx)
        .await?;
        if auth.get::<Vec<u8>, _>("nonce") != request.nonce
            || number(auth.try_get("revision")?)? != revision
            || bytes32(auth.try_get("order_digest")?)? != request.order_digest
        {
            return Err(StoreError::Invalid(
                "aggregate receipt authorization mismatch",
            ));
        }
        let state: String = row.try_get("state")?;
        if state == "sent" {
            if row.get::<Option<Vec<u8>>, _>("resolution_proof").as_deref() != Some(proof)
                || row.get::<Option<Vec<u8>>, _>("vtxo_id").as_deref() != Some(vtxo_id)
                || row.get::<Option<i32>, _>("expiry_height") != Some(h32(expiry)?)
            {
                return Err(StoreError::Invalid("aggregate receipt replay mismatch"));
            }
            return Ok(());
        }
        if !matches!(state.as_str(), "authorized" | "blocked_unknown") {
            return Err(StoreError::Invalid("aggregate cannot accept sent receipt"));
        }
        let members=sqlx::query("SELECT m.block_hash,m.sp_id,m.sats,bf.status FROM ark_aggregate_member m JOIN block_found bf USING(block_hash)
            WHERE m.order_id=$1 ORDER BY m.ordinal FOR UPDATE OF bf,m")
            .bind(&request.order_id[..]).fetch_all(&mut *tx).await?;
        let mut total = 0u64;
        let mut loss = 0u64;
        for member in members {
            let amount = number(member.try_get("sats")?)?;
            total = total
                .checked_add(amount)
                .ok_or(StoreError::Invalid("aggregate member sum"))?;
            let hash = bytes32(member.try_get("block_hash")?)?;
            let id: Vec<u8> = member.try_get("sp_id")?;
            let orphan =
                member.get::<String, _>("status") == "orphaned" || !canonical.contains(&hash);
            if orphan {
                loss = loss
                    .checked_add(amount)
                    .ok_or(StoreError::Invalid("aggregate loss overflow"))?;
            }
            sqlx::query(
                "UPDATE ark_aggregate_member SET orphaned=$3 WHERE order_id=$1 AND block_hash=$2",
            )
            .bind(&request.order_id[..])
            .bind(&hash[..])
            .bind(orphan)
            .execute(&mut *tx)
            .await?;
            let updated=sqlx::query("UPDATE ark_payout SET status='sent',vtxo_id=$3,expiry_height=$4 WHERE block_hash=$1 AND sp_id=$2 AND status='scheduled' AND sats=$5")
                .bind(&hash[..]).bind(&id).bind(vtxo_id).bind(h32(expiry)?).bind(signed(amount)?).execute(&mut *tx).await?;
            if updated.rows_affected() != 1 {
                return Err(StoreError::Invalid(
                    "aggregate member missing or already reassigned",
                ));
            }
        }
        if total != sats {
            return Err(StoreError::Invalid(
                "aggregate incomplete receipt membership",
            ));
        }
        sqlx::query("UPDATE ark_aggregate SET state='sent',revision=revision+1,vtxo_id=$2,expiry_height=$3,resolution_proof=$4,pool_loss_sats=$5 WHERE order_id=$1")
            .bind(&request.order_id[..]).bind(vtxo_id).bind(h32(expiry)?).bind(proof).bind(signed(loss)?).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Safe local NoEffect proof: no grant ever existed, and cancellation
    /// atomically tombstones the order before any delayed request can grant it.
    /// Authorized/unknown orders require an external definitive proof path and
    /// cannot be released by this operation, timeout, or elapsed block count.
    pub async fn cancel_aggregate_without_authorization(
        &self,
        id: &[u8; 16],
        canonical: &BTreeSet<[u8; 32]>,
    ) -> Result<bool> {
        let mut tx = self.pool().begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(1397769025)")
            .execute(&mut *tx)
            .await?;
        let row = sqlx::query("SELECT state FROM ark_aggregate WHERE order_id=$1 FOR UPDATE")
            .bind(&id[..])
            .fetch_one(&mut *tx)
            .await?;
        let state: String = row.try_get("state")?;
        if state == "cancelled" {
            return Ok(true);
        }
        let authorized: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM ark_send_authorization WHERE order_id=$1)",
        )
        .bind(&id[..])
        .fetch_one(&mut *tx)
        .await?;
        if authorized || !matches!(state.as_str(), "frozen" | "published" | "blocked_unknown") {
            return Ok(false);
        }
        let members = sqlx::query(
            "SELECT block_hash,sp_id FROM ark_aggregate_member WHERE order_id=$1 ORDER BY ordinal",
        )
        .bind(&id[..])
        .fetch_all(&mut *tx)
        .await?;
        let mut orphaned_blocks = BTreeSet::new();
        for member in members {
            let hash = bytes32(member.try_get("block_hash")?)?;
            if canonical.contains(&hash) {
                continue;
            }
            orphaned_blocks.insert(hash);
            sqlx::query(
                "UPDATE ark_aggregate_member SET orphaned=true WHERE order_id=$1 AND block_hash=$2",
            )
            .bind(&id[..])
            .bind(&hash[..])
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query("UPDATE ark_aggregate_member SET reserved=false WHERE order_id=$1")
            .bind(&id[..])
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE ark_aggregate SET state='cancelled',revision=revision+1,resolution_proof=$2 WHERE order_id=$1")
            .bind(&id[..]).bind(b"durable_no_authorization_tombstone/v1".as_slice()).execute(&mut *tx).await?;
        for hash in orphaned_blocks {
            Self::orphan_conn(&mut tx, &hash).await?;
        }
        tx.commit().await?;
        Ok(true)
    }

    pub async fn block_aggregate_unknown(&self, id: &[u8; 16]) -> Result<()> {
        sqlx::query("UPDATE ark_aggregate SET state='blocked_unknown',revision=revision+1 WHERE order_id=$1 AND state IN ('published','authorized')")
            .bind(&id[..]).execute(self.pool()).await?;
        Ok(())
    }
    pub async fn aggregate_candidates(&self, sp_id: &[u8; 32]) -> Result<Vec<AggregateMember>> {
        let rows=sqlx::query("SELECT ap.block_hash,bf.height,ap.sp_id,ap.sats,ap.due_height,ds.entries
            FROM ark_payout ap JOIN block_found bf USING(block_hash)
            JOIN distribution_snapshot ds ON ds.fingerprint=bf.fingerprint
            WHERE ap.sp_id=$1 AND ap.status='scheduled' AND bf.status='confirmed'
              AND NOT EXISTS(SELECT 1 FROM ark_aggregate_member m WHERE m.block_hash=ap.block_hash AND m.sp_id=ap.sp_id AND m.reserved)
            ORDER BY bf.height,ap.block_hash,ap.sp_id LIMIT 2048")
            .bind(&sp_id[..]).fetch_all(self.pool()).await?;
        rows.into_iter()
            .map(|row| {
                let entries: Vec<Entry> = serde_json::from_value(row.try_get("entries")?)
                    .map_err(|_| StoreError::Invalid("aggregate snapshot entries"))?;
                let entry = entries
                    .into_iter()
                    .find(|e| &e.sp_id == sp_id)
                    .ok_or(StoreError::Invalid("aggregate snapshot identity"))?;
                Ok(AggregateMember {
                    block_hash: bytes32(row.try_get("block_hash")?)?,
                    sp_id: *sp_id,
                    block_height: height(row.try_get("height")?)?,
                    due_height: height(row.try_get("due_height")?)?,
                    sats: number(row.try_get("sats")?)?,
                    destination: entry.dest_id,
                    is_remainder: entry.rail == Rail::Coinbase,
                })
            })
            .collect()
    }

    /// Caller has freshly validated every candidate block at (tip,tip_hash).
    /// Row locks recheck durable amounts/status/destination before freezing.
    pub async fn freeze_aggregate(
        &self,
        members: &[AggregateMember],
        tip: u32,
        tip_hash: [u8; 32],
        signer: &Keypair,
    ) -> Result<ArkSendOrder> {
        self.freeze_aggregate_inner(members, tip, tip_hash, signer, None)
            .await
    }
    pub async fn freeze_aggregate_bound(
        &self,
        members: &[AggregateMember],
        tip: u32,
        tip_hash: [u8; 32],
        signer: &Keypair,
        server_pubkey: [u8; 33],
    ) -> Result<ArkSendOrder> {
        self.freeze_aggregate_inner(members, tip, tip_hash, signer, Some(server_pubkey))
            .await
    }
    async fn freeze_aggregate_inner(
        &self,
        members: &[AggregateMember],
        tip: u32,
        tip_hash: [u8; 32],
        signer: &Keypair,
        server_pubkey: Option<[u8; 33]>,
    ) -> Result<ArkSendOrder> {
        if members.is_empty() || members.len() > MAX_MEMBERS {
            return Err(StoreError::Invalid("aggregate member count"));
        }
        let anchor = &members[0];
        let mut seen = BTreeSet::new();
        let mut sats = 0u64;
        let mut due = 0;
        for member in members {
            if member.sp_id != anchor.sp_id
                || member.sats == 0
                || member.due_height > tip
                || !seen.insert(member.block_hash)
            {
                return Err(StoreError::Invalid("aggregate membership"));
            }
            sats = sats
                .checked_add(member.sats)
                .ok_or(StoreError::Invalid("aggregate sum overflow"))?;
            due = due.max(member.due_height);
        }
        if members.windows(2).any(|w| {
            (w[0].block_height, w[0].block_hash, w[0].sp_id)
                >= (w[1].block_height, w[1].block_hash, w[1].sp_id)
        }) {
            return Err(StoreError::Invalid("aggregate member ordering"));
        }
        let address = String::from_utf8(anchor.destination.clone())
            .map_err(|_| StoreError::Invalid("aggregate destination encoding"))?;
        if address.is_empty() {
            return Err(StoreError::Invalid("aggregate destination missing"));
        }
        let mut tx = self.pool().begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(1397769025)")
            .execute(&mut *tx)
            .await?;
        crate::economic::check_seed(&mut tx).await?;
        sqlx::query("SELECT sp_id FROM sp_identity WHERE sp_id=$1 FOR UPDATE")
            .bind(&anchor.sp_id[..])
            .fetch_one(&mut *tx)
            .await?;
        for member in members {
            let row=sqlx::query("SELECT ap.sats,ap.status,bf.status AS block_status,bf.height,ap.due_height,ds.entries
                FROM ark_payout ap JOIN block_found bf USING(block_hash)
                JOIN distribution_snapshot ds ON ds.fingerprint=bf.fingerprint
                WHERE ap.block_hash=$1 AND ap.sp_id=$2 FOR UPDATE OF bf,ap")
                .bind(&member.block_hash[..]).bind(&member.sp_id[..]).fetch_one(&mut *tx).await?;
            let entries: Vec<Entry> = serde_json::from_value(row.try_get("entries")?)
                .map_err(|_| StoreError::Invalid("aggregate snapshot"))?;
            let entry = entries
                .iter()
                .find(|e| e.sp_id == member.sp_id)
                .ok_or(StoreError::Invalid("aggregate identity"))?;
            if row.get::<String, _>("status") != "scheduled"
                || row.get::<String, _>("block_status") != "confirmed"
                || number(row.try_get("sats")?)? != member.sats
                || height(row.try_get("height")?)? != member.block_height
                || height(row.try_get("due_height")?)? != member.due_height
                || entry.dest_id != member.destination
            {
                return Err(StoreError::Invalid(
                    "aggregate member changed before freeze",
                ));
            }
        }
        let seq: i64 = sqlx::query_scalar(
            "SELECT nextval(pg_get_serial_sequence('ark_aggregate','batch_seq'))",
        )
        .fetch_one(&mut *tx)
        .await?;
        let mut hash = sha256::Hash::engine();
        hash.input(b"SPc/ArkAggregateV1");
        hash.input(&anchor.sp_id);
        hash.input(&anchor.block_hash);
        hash.input(&seq.to_le_bytes());
        let order_id = sha256::Hash::from_engine(hash).to_byte_array()[..16]
            .try_into()
            .map_err(|_| StoreError::Invalid("aggregate ID"))?;
        let mut order = ArkSendOrder {
            order_id,
            ark_address: address,
            sats,
            not_before_height: due,
            sig: [0; 64],
        };
        order
            .sign(signer)
            .map_err(|_| StoreError::Invalid("aggregate order signature"))?;
        let bytes = order
            .encode()
            .map_err(|_| StoreError::Invalid("aggregate order encoding"))?;
        let digest =
            order_digest(&order).map_err(|_| StoreError::Invalid("aggregate order digest"))?;
        sqlx::query("INSERT INTO ark_aggregate(batch_seq,order_id,sp_id,anchor_hash,anchor_height,destination,sats,not_before_height,selected_tip,selected_tip_hash,order_bytes,order_digest,state,server_pubkey)
            VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,'frozen',$13)")
            .bind(seq).bind(&order_id[..]).bind(&anchor.sp_id[..]).bind(&anchor.block_hash[..]).bind(h32(anchor.block_height)?)
            .bind(&anchor.destination).bind(signed(sats)?).bind(h32(due)?).bind(h32(tip)?).bind(&tip_hash[..]).bind(bytes).bind(&digest[..]).bind(server_pubkey.as_ref().map(|v|v.as_slice()))
            .execute(&mut *tx).await?;
        for (ordinal, member) in members.iter().enumerate() {
            sqlx::query("INSERT INTO ark_aggregate_member(order_id,ordinal,block_hash,sp_id,block_height,due_height,sats) VALUES($1,$2,$3,$4,$5,$6,$7)")
                .bind(&order_id[..]).bind(ordinal as i32).bind(&member.block_hash[..]).bind(&member.sp_id[..])
                .bind(h32(member.block_height)?).bind(h32(member.due_height)?).bind(signed(member.sats)?).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(order)
    }

    pub async fn aggregate(&self, id: &[u8; 16]) -> Result<Option<FrozenAggregate>> {
        let Some(row)=sqlx::query("SELECT order_bytes,order_digest,state,revision,destination,server_pubkey FROM ark_aggregate WHERE order_id=$1")
            .bind(&id[..]).fetch_optional(self.pool()).await? else {return Ok(None)};
        let bytes: Vec<u8> = row.try_get("order_bytes")?;
        let order = ArkSendOrder::decode(&bytes)
            .map_err(|_| StoreError::Invalid("frozen order encoding"))?;
        let digest = bytes32(row.try_get("order_digest")?)?;
        if order_digest(&order).map_err(|_| StoreError::Invalid("frozen order digest"))? != digest {
            return Err(StoreError::Invalid("frozen order corruption"));
        }
        let rows=sqlx::query("SELECT block_hash,sp_id,block_height,due_height,sats FROM ark_aggregate_member WHERE order_id=$1 ORDER BY ordinal")
            .bind(&id[..]).fetch_all(self.pool()).await?;
        let members = rows
            .into_iter()
            .map(|r| {
                Ok(AggregateMember {
                    block_hash: bytes32(r.try_get("block_hash")?)?,
                    sp_id: bytes32(r.try_get("sp_id")?)?,
                    block_height: height(r.try_get("block_height")?)?,
                    due_height: height(r.try_get("due_height")?)?,
                    sats: number(r.try_get("sats")?)?,
                    destination: vec![],
                    is_remainder: true,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Some(FrozenAggregate {
            server_pubkey: row
                .get::<Option<Vec<u8>>, _>("server_pubkey")
                .map(|v| {
                    v.try_into()
                        .map_err(|_| StoreError::Invalid("frozen Ark server key"))
                })
                .transpose()?,
            order,
            digest,
            state: row.try_get("state")?,
            revision: number(row.try_get("revision")?)?,
            members,
        }))
    }

    /// Commit publication intent BEFORE XADD. Failure/lost XADD retains the set.
    pub async fn publish_aggregate(&self, id: &[u8; 16]) -> Result<()> {
        sqlx::query("UPDATE ark_aggregate SET state='published',revision=revision+1 WHERE order_id=$1 AND state='frozen'")
            .bind(&id[..]).execute(self.pool()).await?;
        Ok(())
    }

    /// Fresh observations supplied by payout for this nonce. One durable grant
    /// maximum until definitive resolution; lost response cannot mint a retry.
    pub async fn authorize_aggregate(
        &self,
        request: &AuthorizationRequest,
        tip: u32,
        tip_hash: [u8; 32],
        canonical: &BTreeSet<[u8; 32]>,
        now: u64,
        signer: &Keypair,
    ) -> Result<Option<AuthorizationGrant>> {
        self.authorize_aggregate_inner(request, tip, tip_hash, canonical, now, signer, None)
            .await
    }

    pub async fn authorize_aggregate_observed(
        &self,
        request: &AuthorizationRequest,
        read: &crate::observations::CanonicalRead,
        now: u64,
        signer: &Keypair,
    ) -> Result<Option<AuthorizationGrant>> {
        let Some((tip, hash, canonical)) = read.canonical() else {
            self.record_observation("authorization_decision",&request.order_id,None,&serde_json::json!({
                "schema":"spc.authorization-decision/1","request_bytes":hex::encode(request.encode()),
                "now":now,"canonical_read":read,"outcome":"canonical_read_unknown","grant_bytes":null})).await?;
            return Ok(None);
        };
        let result = self
            .authorize_aggregate_inner(request, tip, hash, &canonical, now, signer, Some(read))
            .await;
        if result.is_err() {
            let _=self.record_observation("authorization_decision",&request.order_id,None,&serde_json::json!({
                "schema":"spc.authorization-decision/1","request_bytes":hex::encode(request.encode()),"now":now,
                "canonical_read":read,"outcome":"issuance_unknown","grant_bytes":null,
                "reason":"transaction_or_persistence_failed"})).await;
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    async fn authorize_aggregate_inner(
        &self,
        request: &AuthorizationRequest,
        tip: u32,
        tip_hash: [u8; 32],
        canonical: &BTreeSet<[u8; 32]>,
        now: u64,
        signer: &Keypair,
        read: Option<&crate::observations::CanonicalRead>,
    ) -> Result<Option<AuthorizationGrant>> {
        let mut tx = self.pool().begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SELECT pg_advisory_xact_lock(1397769025)")
            .execute(&mut *tx)
            .await?;
        crate::economic::check_seed(&mut tx).await?;
        let mut observation = serde_json::json!({"schema":"spc.authorization-decision/1","request_bytes":hex::encode(request.encode()),
            "request_nonce":hex::encode(request.nonce),"now":now,"checked_tip":tip,"checked_tip_hash":hex::encode(tip_hash),
            "canonical_set":canonical.iter().map(hex::encode).collect::<Vec<_>>(),"canonical_read":read,
            "sql_members":null,"grant_bytes":null});
        let Some(row)=sqlx::query("SELECT state,revision,order_digest,not_before_height,sats FROM ark_aggregate WHERE order_id=$1 FOR UPDATE")
            .bind(&request.order_id[..]).fetch_optional(&mut *tx).await? else {
                observation["outcome"]="aggregate_absent".into();
                crate::observations::append(&mut tx,"authorization_decision",&request.order_id,None,&observation).await?;
                tx.commit().await?;return Ok(None)
            };
        let state: String = row.try_get("state")?;
        if state != "published"
            || request.nonce == [0; 16]
            || bytes32(row.try_get("order_digest")?)? != request.order_digest
            || height(row.try_get("not_before_height")?)? > tip
        {
            observation["outcome"] = "request_not_issuable".into();
            crate::observations::append(
                &mut tx,
                "authorization_decision",
                &request.order_id,
                None,
                &observation,
            )
            .await?;
            tx.commit().await?;
            return Ok(None);
        }
        let census: Vec<serde_json::Value> = sqlx::query_scalar(
            "SELECT to_jsonb(m) FROM ark_aggregate_member m WHERE order_id=$1 ORDER BY ordinal",
        )
        .bind(&request.order_id[..])
        .fetch_all(&mut *tx)
        .await?;
        observation["queried_members"] = serde_json::Value::Array(census);
        let rows=sqlx::query("SELECT m.block_hash,bf.status,m.sats,m.reserved,ap.sats AS payout_sats,ap.status AS payout_status,
            jsonb_build_object('member',to_jsonb(m),'block_status',bf.status,'payout_status',ap.status,'payout_sats',ap.sats) AS observed_member
            FROM ark_aggregate_member m JOIN block_found bf USING(block_hash)
            JOIN ark_payout ap ON ap.block_hash=m.block_hash AND ap.sp_id=m.sp_id
            WHERE m.order_id=$1 ORDER BY m.ordinal FOR UPDATE OF bf,ap,m")
            .bind(&request.order_id[..]).fetch_all(&mut *tx).await?;
        let mut valid = !rows.is_empty();
        observation["sql_members"] =
            serde_json::Value::Array(rows.iter().map(|row| row.get("observed_member")).collect());
        let mut member_total = 0u64;
        for row in rows {
            member_total = member_total
                .checked_add(number(row.try_get("sats")?)?)
                .ok_or(StoreError::Invalid("authorization member total"))?;
            if row.get::<String, _>("status") != "confirmed"
                || !canonical.contains(&bytes32(row.try_get("block_hash")?)?)
                || !row.get::<bool, _>("reserved")
                || row.get::<String, _>("payout_status") != "scheduled"
                || row.get::<i64, _>("sats") != row.get::<i64, _>("payout_sats")
            {
                valid = false;
            }
        }
        valid &= member_total == number(row.try_get("sats")?)?;
        if !valid {
            sqlx::query("UPDATE ark_aggregate SET state='blocked_unknown',revision=revision+1 WHERE order_id=$1")
                .bind(&request.order_id[..]).execute(&mut *tx).await?;
            observation["outcome"] = "members_not_canonical_or_reserved".into();
            crate::observations::append(
                &mut tx,
                "authorization_decision",
                &request.order_id,
                None,
                &observation,
            )
            .await?;
            tx.commit().await?;
            return Ok(None);
        }
        let revision = number(row.try_get("revision")?)?
            .checked_add(1)
            .ok_or(StoreError::Invalid("aggregate revision overflow"))?;
        let expires_at = now
            .checked_add(spc_ext::authorization::GRANT_LIFETIME_SECS)
            .ok_or(StoreError::Invalid("grant expiry overflow"))?;
        let mut grant = AuthorizationGrant {
            request: request.clone(),
            revision,
            expires_at,
            signature: [0; 64],
        };
        grant.sign(signer);
        observation["outcome"] = "issued".into();
        observation["grant_bytes"] = hex::encode(grant.encode()).into();
        observation["aggregate_revision"] = revision.into();
        sqlx::query("INSERT INTO ark_send_authorization(order_id,nonce,revision,order_digest,checked_tip,checked_tip_hash,expires_at,grant_bytes) VALUES($1,$2,$3,$4,$5,$6,$7,$8)")
            .bind(&request.order_id[..]).bind(&request.nonce[..]).bind(signed(revision)?).bind(&request.order_digest[..])
            .bind(h32(tip)?).bind(&tip_hash[..]).bind(signed(expires_at)?).bind(grant.encode()).execute(&mut *tx).await?;
        sqlx::query("UPDATE ark_aggregate SET state='authorized',revision=$2 WHERE order_id=$1")
            .bind(&request.order_id[..])
            .bind(signed(revision)?)
            .execute(&mut *tx)
            .await?;
        crate::observations::append(
            &mut tx,
            "authorization_decision",
            &request.order_id,
            None,
            &observation,
        )
        .await?;
        tx.commit().await?;
        Ok(Some(grant))
    }

    /// Per-owner projection; pool Sent is explicitly NOT wallet-received proof.
    pub async fn aggregate_status(&self, id: &[u8; 32]) -> Result<serde_json::Value> {
        let mut tx = self.pool().begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        let rows=sqlx::query("SELECT ap.block_hash,bf.height,ap.sats,ap.due_height,ap.status,ap.vtxo_id,ap.expiry_height,m.order_id
            FROM ark_payout ap JOIN block_found bf USING(block_hash)
            LEFT JOIN ark_aggregate_member m ON m.block_hash=ap.block_hash AND m.sp_id=ap.sp_id AND m.reserved
            WHERE ap.sp_id=$1 AND ap.sats>0 ORDER BY bf.height,ap.block_hash LIMIT 2049")
            .bind(&id[..]).fetch_all(&mut *tx).await?;
        if rows.len() > 2048 {
            return Err(StoreError::Invalid("aggregate status requires pagination"));
        }
        let mut obligations = Vec::new();
        for row in rows {
            let order: Option<Vec<u8>> = row.try_get("order_id")?;
            obligations.push(serde_json::json!({"block_hash":row.get::<Vec<u8>,_>("block_hash"),"height":row.get::<i32,_>("height"),
                "sats":row.get::<i64,_>("sats"),"due_height":row.get::<i32,_>("due_height"),"payout_state":row.get::<String,_>("status"),
                "order_id":order,"vtxo_id":row.get::<Option<Vec<u8>>,_>("vtxo_id"),"expiry_height":row.get::<Option<i32>,_>("expiry_height")}));
        }
        let rows=sqlx::query("SELECT order_id,anchor_hash,anchor_height,sats,state,revision,vtxo_id,expiry_height,pool_loss_sats FROM ark_aggregate WHERE sp_id=$1 ORDER BY batch_seq DESC LIMIT 2049")
            .bind(&id[..]).fetch_all(&mut *tx).await?;
        if rows.len() > 2048 {
            return Err(StoreError::Invalid("aggregate status requires pagination"));
        }
        let mut orders = Vec::new();
        for row in rows {
            let order: Vec<u8> = row.try_get("order_id")?;
            let members=sqlx::query("SELECT block_hash,block_height,sats,orphaned FROM ark_aggregate_member WHERE order_id=$1 AND sp_id=$2 ORDER BY ordinal")
                .bind(&order).bind(&id[..]).fetch_all(&mut *tx).await?;
            let members:Vec<_>=members.into_iter().map(|r|serde_json::json!({"block_hash":r.get::<Vec<u8>,_>("block_hash"),"height":r.get::<i32,_>("block_height"),"sats":r.get::<i64,_>("sats"),"orphaned":r.get::<bool,_>("orphaned")})).collect();
            orders.push(serde_json::json!({"order_id":order,"anchor_hash":row.get::<Vec<u8>,_>("anchor_hash"),"anchor_height":row.get::<i32,_>("anchor_height"),
                "sats":row.get::<i64,_>("sats"),"state":row.get::<String,_>("state"),"revision":row.get::<i64,_>("revision"),"members":members,
                "vtxo_id":row.get::<Option<Vec<u8>>,_>("vtxo_id"),"expiry_height":row.get::<Option<i32>,_>("expiry_height"),"pool_loss_sats":row.get::<i64,_>("pool_loss_sats")}));
        }
        tx.commit().await?;
        Ok(
            serde_json::json!({"schema":"spc.aggregate-status/1","obligations":obligations,"orders":orders,
            "wallet_received":null,"sent_means":"fetch_and_verify_owned_vtxo_before_closing_watch"}),
        )
    }
}
