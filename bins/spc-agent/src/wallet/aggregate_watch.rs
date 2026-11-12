//! Authenticated aggregate obligations are WATCH metadata, never credits.
//! Credit/allocation happens only after complete stock proofs are verified.
use super::WalletStore;
use anyhow::{ensure, Context};
use bitcoin::secp256k1::XOnlyPublicKey;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AggregateStatus {
    pub schema: String,
    pub obligations: Vec<Obligation>,
    pub orders: Vec<AggregateOrder>,
    pub wallet_received: Option<bool>,
    pub sent_means: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Obligation {
    pub block_hash: [u8; 32],
    pub height: u32,
    pub sats: u64,
    pub due_height: u32,
    pub payout_state: String,
    pub order_id: Option<[u8; 16]>,
    pub vtxo_id: Option<Vec<u8>>,
    pub expiry_height: Option<u32>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Member {
    pub block_hash: [u8; 32],
    pub height: u32,
    pub sats: u64,
    pub orphaned: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AggregateOrder {
    pub order_id: [u8; 16],
    pub anchor_hash: [u8; 32],
    pub anchor_height: u32,
    pub sats: u64,
    pub state: String,
    pub revision: u64,
    pub members: Vec<Member>,
    pub vtxo_id: Option<Vec<u8>>,
    pub expiry_height: Option<u32>,
    pub pool_loss_sats: u64,
}
impl AggregateOrder {
    fn immutable(&self) -> anyhow::Result<Vec<u8>> {
        Ok(serde_json::to_vec(&(
            self.order_id,
            self.anchor_hash,
            self.anchor_height,
            self.sats,
            self.members
                .iter()
                .map(|m| (m.block_hash, m.height, m.sats))
                .collect::<Vec<_>>(),
        ))?)
    }
}
impl AggregateStatus {
    pub(super) fn validate(&self) -> anyhow::Result<()> {
        ensure!(
            self.schema == "spc.aggregate-status/1"
                && self.wallet_received.is_none()
                && self.sent_means == "fetch_and_verify_owned_vtxo_before_closing_watch",
            "unsupported aggregate status"
        );
        ensure!(
            self.obligations.len() <= 2048 && self.orders.len() <= 2048,
            "aggregate status limit"
        );
        let mut obligations = BTreeSet::new();
        let mut orders = BTreeSet::new();
        for row in &self.obligations {
            ensure!(
                row.sats > 0
                    && row.height > 0
                    && row.due_height >= row.height
                    && obligations.insert(row.block_hash),
                "invalid aggregate obligation"
            );
            ensure!(
                row.vtxo_id.as_ref().is_none_or(|v| v.len() == 36) && row.expiry_height != Some(0),
                "invalid obligation receipt hint"
            );
        }
        for order in &self.orders {
            ensure!(
                orders.insert(order.order_id)
                    && order.revision > 0
                    && order.sats > 0
                    && order.pool_loss_sats <= order.sats
                    && !order.members.is_empty()
                    && order.members.len() <= 2048,
                "invalid aggregate order"
            );
            ensure!(
                matches!(
                    order.state.as_str(),
                    "frozen"
                        | "published"
                        | "authorized"
                        | "blocked_unknown"
                        | "sent"
                        | "cancelled"
                ),
                "unknown aggregate state"
            );
            ensure!(
                order.members[0].block_hash == order.anchor_hash
                    && order.members[0].height == order.anchor_height,
                "aggregate is not oldest stored anchor"
            );
            let mut total = 0u64;
            let mut hashes = BTreeSet::new();
            let mut previous = None;
            for member in &order.members {
                ensure!(
                    member.sats > 0 && member.height > 0 && hashes.insert(member.block_hash),
                    "invalid aggregate membership"
                );
                let key = (member.height, member.block_hash);
                ensure!(
                    previous.is_none_or(|p| p < key),
                    "aggregate membership order"
                );
                previous = Some(key);
                total = total
                    .checked_add(member.sats)
                    .context("aggregate member amount overflow")?;
            }
            ensure!(
                total == order.sats
                    && order.vtxo_id.as_ref().is_none_or(|v| v.len() == 36)
                    && order.expiry_height != Some(0),
                "aggregate total or hint mismatch"
            );
        }
        Ok(())
    }
}

impl WalletStore {
    pub(super) fn init_aggregate_watches(&self) -> anyhow::Result<()> {
        self.lock().execute_batch("CREATE TABLE IF NOT EXISTS aggregate_client_binding (
            id INTEGER PRIMARY KEY CHECK(id=1),owner BLOB NOT NULL,origin TEXT NOT NULL,order_key BLOB NOT NULL);
            CREATE TABLE IF NOT EXISTS aggregate_watches (
            block_hash BLOB PRIMARY KEY CHECK(length(block_hash)=32),height INTEGER NOT NULL,owed_sats TEXT,
            due_height INTEGER,current_order BLOB,verified_order BLOB);
            CREATE TABLE IF NOT EXISTS aggregate_client_orders (
            order_id BLOB PRIMARY KEY CHECK(length(order_id)=16),immutable BLOB NOT NULL,status BLOB NOT NULL,
            revision INTEGER NOT NULL,state TEXT NOT NULL,anchor_hash BLOB NOT NULL,anchor_height INTEGER NOT NULL,frozen_order BLOB,verified_receipt BLOB);
            CREATE TABLE IF NOT EXISTS aggregate_client_members (
            order_id BLOB NOT NULL REFERENCES aggregate_client_orders(order_id),ordinal INTEGER NOT NULL,
            block_hash BLOB NOT NULL REFERENCES aggregate_watches(block_hash),sats TEXT NOT NULL,
            PRIMARY KEY(order_id,ordinal),UNIQUE(order_id,block_hash));
            CREATE TABLE IF NOT EXISTS aggregate_receipt_candidates (
            order_id BLOB PRIMARY KEY REFERENCES aggregate_client_orders(order_id),receipt BLOB NOT NULL);
            CREATE TABLE IF NOT EXISTS aggregate_vtxo_allocation (
            vtxo_id BLOB PRIMARY KEY CHECK(length(vtxo_id)=36),order_id BLOB NOT NULL REFERENCES aggregate_client_orders(order_id));
            CREATE TABLE IF NOT EXISTS aggregate_poll_cursor(id INTEGER PRIMARY KEY CHECK(id=1),last_order BLOB NOT NULL);
            CREATE TABLE IF NOT EXISTS aggregate_origin_proofs(block_hash BLOB PRIMARY KEY CHECK(length(block_hash)=32),block BLOB NOT NULL);
            CREATE TABLE IF NOT EXISTS owned_coinbase_outputs(block_hash BLOB NOT NULL,height INTEGER NOT NULL,derivation_height INTEGER NOT NULL,k INTEGER NOT NULL,vout INTEGER NOT NULL,
                sats TEXT NOT NULL,tweak BLOB NOT NULL,canonical INTEGER NOT NULL DEFAULT 1,PRIMARY KEY(block_hash,derivation_height,k),UNIQUE(block_hash,vout));
            INSERT OR IGNORE INTO aggregate_poll_cursor(id,last_order) VALUES(1,X'');")?;
        Ok(())
    }

    pub(super) fn bind_aggregate_api(
        &self,
        owner: &[u8; 32],
        origin: &str,
        key: &XOnlyPublicKey,
    ) -> anyhow::Result<()> {
        self.init_aggregate_watches()?;
        let mut guard = self.lock();
        let tx = guard.transaction()?;
        let own: Vec<u8> =
            tx.query_row("SELECT value FROM meta WHERE key='sp_id'", [], |r| r.get(0))?;
        ensure!(own == owner, "aggregate API owner mismatch");
        let previous: Option<(Vec<u8>, String, Vec<u8>)> = tx
            .query_row(
                "SELECT owner,origin,order_key FROM aggregate_client_binding WHERE id=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        if let Some((old_owner, old_origin, old_key)) = previous {
            ensure!(
                old_owner == owner && old_origin == origin && old_key == key.serialize(),
                "aggregate API identity/key/origin changed"
            );
        } else {
            tx.execute("INSERT INTO aggregate_client_binding(id,owner,origin,order_key) VALUES(1,?1,?2,?3)",params![owner.as_slice(),origin,key.serialize().as_slice()])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Only the fresh-challenge transport calls this in production. Absence,
    /// empty views and errors never delete or close any existing watch.
    pub(super) fn ingest_aggregate_status(
        &self,
        owner: &[u8; 32],
        status: &AggregateStatus,
    ) -> anyhow::Result<()> {
        status.validate()?;
        self.init_aggregate_watches()?;
        let mut guard = self.lock();
        let tx = guard.transaction()?;
        let bound: Vec<u8> = tx.query_row(
            "SELECT owner FROM aggregate_client_binding WHERE id=1",
            [],
            |r| r.get(0),
        )?;
        ensure!(bound == owner, "aggregate status wrong owner");
        for row in &status.obligations {
            merge_watch(
                &tx,
                &row.block_hash,
                row.height,
                row.sats,
                Some(row.due_height),
            )?;
        }
        for order in &status.orders {
            let revision = i64::try_from(order.revision)?;
            let old:Option<(Vec<u8>,i64,String)>=tx.query_row("SELECT immutable,revision,state FROM aggregate_client_orders WHERE order_id=?1",params![order.order_id.as_slice()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
            let immutable = order.immutable()?;
            if let Some((prior, rev, state)) = &old {
                ensure!(
                    *prior == immutable && revision >= *rev,
                    "aggregate frozen membership changed or status regressed"
                );
                ensure!(
                    !matches!(state.as_str(), "sent" | "cancelled") || *state == order.state,
                    "aggregate terminal outcome regressed"
                );
            }
            tx.execute("INSERT INTO aggregate_client_orders(order_id,immutable,status,revision,state,anchor_hash,anchor_height) VALUES(?1,?2,?3,?4,?5,?6,?7)
                ON CONFLICT(order_id) DO UPDATE SET status=excluded.status,revision=excluded.revision,state=excluded.state",
                params![order.order_id.as_slice(),immutable,serde_json::to_vec(order)?,revision,order.state,order.anchor_hash.as_slice(),order.anchor_height])?;
            for (ordinal, member) in order.members.iter().enumerate() {
                merge_watch(&tx, &member.block_hash, member.height, member.sats, None)?;
                if old.is_none() {
                    tx.execute("INSERT INTO aggregate_client_members(order_id,ordinal,block_hash,sats) VALUES(?1,?2,?3,?4)",params![order.order_id.as_slice(),ordinal,member.block_hash.as_slice(),member.sats.to_string()])?;
                }
            }
        }
        // Apply associations only after every authenticated order transition
        // has been validated. The producer may return newest orders first.
        for order in &status.orders {
            for member in &order.members {
                if order.state != "cancelled" {
                    let prior: Option<Vec<u8>> = tx.query_row(
                        "SELECT current_order FROM aggregate_watches WHERE block_hash=?1",
                        params![member.block_hash.as_slice()],
                        |r| r.get(0),
                    )?;
                    if let Some(prior) = prior.filter(|p| p.as_slice() != order.order_id) {
                        let state: String = tx.query_row(
                            "SELECT state FROM aggregate_client_orders WHERE order_id=?1",
                            params![prior],
                            |r| r.get(0),
                        )?;
                        ensure!(
                            state == "cancelled",
                            "possibly sent aggregate member reassigned"
                        );
                    }
                    tx.execute(
                        "UPDATE aggregate_watches SET current_order=?2 WHERE block_hash=?1",
                        params![member.block_hash.as_slice(), order.order_id.as_slice()],
                    )?;
                }
            }
        }
        // Every association advertised by the obligation must have the same
        // owner-scoped frozen order/member in this authenticated snapshot.
        for row in &status.obligations {
            if let Some(id) = row.order_id {
                let order = status
                    .orders
                    .iter()
                    .find(|o| o.order_id == id)
                    .context("incomplete aggregate order view")?;
                ensure!(
                    order.members.iter().any(|m| m.block_hash == row.block_hash
                        && m.height == row.height
                        && m.sats == row.sats),
                    "obligation/member association mismatch"
                );
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub(super) fn aggregate_fetch_work(&self) -> anyhow::Result<Vec<AggregateOrder>> {
        let mut guard = self.lock();
        let tx = guard.transaction()?;
        let mut stmt=tx.prepare("SELECT status FROM aggregate_client_orders WHERE state='sent' AND verified_receipt IS NULL
            ORDER BY order_id <= (SELECT last_order FROM aggregate_poll_cursor WHERE id=1),order_id LIMIT 8")?;
        let values = stmt
            .query_map([], |r| r.get::<_, Vec<u8>>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        drop(stmt);
        let orders = values
            .iter()
            .map(|v| serde_json::from_slice::<AggregateOrder>(v))
            .collect::<Result<Vec<_>, _>>()?;
        tx.commit()?;
        Ok(orders)
    }
    pub(super) fn begin_aggregate_fetch(&self, id: &[u8; 16]) -> anyhow::Result<()> {
        self.lock().execute(
            "UPDATE aggregate_poll_cursor SET last_order=?1 WHERE id=1",
            params![id.as_slice()],
        )?;
        Ok(())
    }

    pub(super) fn retain_candidate_receipt(
        &self,
        id: &[u8; 16],
        receipt: &spc_ext::authorization::ProvedReceipt,
    ) -> anyhow::Result<()> {
        let bytes = receipt.encode()?;
        ensure!(
            receipt.grant.request.order_id == *id,
            "receipt order mismatch"
        );
        let mut guard = self.lock();
        let tx = guard.transaction()?;
        let previous: Option<Vec<u8>> = tx
            .query_row(
                "SELECT receipt FROM aggregate_receipt_candidates WHERE order_id=?1",
                params![id.as_slice()],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(previous) = previous {
            ensure!(previous == bytes, "aggregate receipt candidate changed");
        } else {
            tx.execute(
                "INSERT INTO aggregate_receipt_candidates(order_id,receipt) VALUES(?1,?2)",
                params![id.as_slice(), bytes],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(super) fn frozen_aggregate_order(
        &self,
        id: &[u8; 16],
    ) -> anyhow::Result<Option<spc_ext::ArkSendOrder>> {
        let bytes: Option<Vec<u8>> = self.lock().query_row(
            "SELECT frozen_order FROM aggregate_client_orders WHERE order_id=?1",
            params![id.as_slice()],
            |r| r.get(0),
        )?;
        bytes
            .map(|b| spc_ext::ArkSendOrder::decode(&b).map_err(anyhow::Error::from))
            .transpose()
    }

    pub(super) fn retain_frozen_aggregate_order(
        &self,
        id: &[u8; 16],
        bytes: &[u8],
    ) -> anyhow::Result<()> {
        let order = spc_ext::ArkSendOrder::decode(bytes)?;
        ensure!(
            order.encode()? == bytes && order.order_id == *id,
            "frozen order association/encoding mismatch"
        );
        let mut guard = self.lock();
        let tx = guard.transaction()?;
        let key: Vec<u8> = tx.query_row(
            "SELECT order_key FROM aggregate_client_binding WHERE id=1",
            [],
            |r| r.get(0),
        )?;
        order.verify(&XOnlyPublicKey::from_slice(&key)?)?;
        let (status, previous): (Vec<u8>, Option<Vec<u8>>) = tx.query_row(
            "SELECT status,frozen_order FROM aggregate_client_orders WHERE order_id=?1",
            params![id.as_slice()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let status: AggregateOrder = serde_json::from_slice(&status)?;
        ensure!(
            order.sats == status.sats,
            "frozen order amount differs from covered set"
        );
        if let Some(previous) = previous {
            ensure!(previous == bytes, "stored frozen order changed");
        } else {
            tx.execute(
                "UPDATE aggregate_client_orders SET frozen_order=?2 WHERE order_id=?1",
                params![id.as_slice(), bytes],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(super) fn verified_aggregate_origin(
        &self,
        hash: &[u8; 32],
    ) -> anyhow::Result<Option<bitcoin::Block>> {
        self.init_aggregate_watches()?;
        let bytes:Option<Vec<u8>>=self.lock().query_row("SELECT p.block FROM aggregate_origin_proofs p WHERE p.block_hash=?1
            AND EXISTS(SELECT 1 FROM aggregate_client_orders o WHERE o.verified_receipt IS NOT NULL AND o.order_id IN
            (SELECT order_id FROM aggregate_client_members WHERE block_hash=?1))",params![hash.as_slice()],|r|r.get(0)).optional()?;
        bytes
            .map(|b| bitcoin::consensus::deserialize(&b).map_err(anyhow::Error::from))
            .transpose()
    }
}

fn merge_watch(
    tx: &rusqlite::Transaction<'_>,
    hash: &[u8; 32],
    height: u32,
    sats: u64,
    due: Option<u32>,
) -> anyhow::Result<()> {
    let prior: Option<(u32, Option<String>)> = tx
        .query_row(
            "SELECT height,owed_sats FROM aggregate_watches WHERE block_hash=?1",
            params![hash.as_slice()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if let Some((h, amount)) = prior {
        ensure!(
            h == height && amount.is_none_or(|a| a == sats.to_string()),
            "watched obligation changed"
        );
    }
    tx.execute("INSERT INTO aggregate_watches(block_hash,height,owed_sats,due_height) VALUES(?1,?2,?3,?4)
        ON CONFLICT(block_hash) DO UPDATE SET owed_sats=excluded.owed_sats,due_height=COALESCE(excluded.due_height,due_height)",params![hash.as_slice(),height,sats.to_string(),due])?;
    Ok(())
}
