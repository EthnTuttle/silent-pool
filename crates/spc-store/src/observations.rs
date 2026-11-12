//! Immutable SPc decision evidence. No identity-bearing DTO is an Ark message.
use crate::{
    postgres::{Result, StoreError},
    wire::BlockEvent,
    Store,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{PgConnection, Row};
use std::collections::BTreeSet;

pub const FRONT_OBSERVATIONS_STREAM: &str = "spc:source:decisions";
pub const FRONT_OBSERVATIONS_ACK: &str = "spc:source:decision-acks";

/// SPc-only private transport; never authorized by the Ark-role ACL.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrontObservation {
    pub schema_version: u8,
    pub session: [u8; 16],
    pub attempt: [u8; 16],
    pub body: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrontObservationAck {
    pub digest: String,
    pub observation_id: i64,
}
impl FrontObservation {
    fn validate(&self) -> Result<()> {
        if self.schema_version != 1
            || self.body["schema"] != "spc.front-derivation/1"
            || self.body["source_session"] != hex::encode(self.session)
        {
            return Err(StoreError::Invalid("front observation binding"));
        }
        public_fields(
            &self.body,
            &[
                "schema",
                "source_session",
                "front_id",
                "a_send_x",
                "queried_identity_census",
                "census_complete",
                "status",
                "job_id",
                "generation",
                "height",
                "economic_revision",
                "distribution_fingerprint",
                "rows",
            ],
        )?;
        let rows = self.body["rows"]
            .as_array()
            .ok_or(StoreError::Invalid("public derivation rows"))?;
        let census = self.body["queried_identity_census"]
            .as_array()
            .ok_or(StoreError::Invalid("derivation identity census missing"))?;
        if census.len() != rows.len()
            || rows.iter().zip(census).any(|(row, id)| &row["sp_id"] != id)
        {
            return Err(StoreError::Invalid("derivation identity census mismatch"));
        }
        match self.body["status"].as_str() {
            Some("frozen_job")
                if self.body["job_id"].as_u64().is_some()
                    && self.body["generation"].as_u64().is_some()
                    && self.body["height"]
                        .as_u64()
                        .is_some_and(|h| h <= u64::from(u32::MAX))
                    && self.body["census_complete"] == true
                    && self.body["a_send_x"]
                        .as_str()
                        .is_some_and(|s| s.len() == 64)
                    && self.body["distribution_fingerprint"]
                        .as_str()
                        .is_some_and(|s| s.len() == 64) => {}
            Some("failed_unadvertised_attempt")
                if self.body["job_id"].is_null()
                    && self.body["generation"].is_null()
                    && self.body["census_complete"] == false => {}
            _ => {
                return Err(StoreError::Invalid(
                    "front observation decision context missing",
                ))
            }
        }
        for row in rows {
            public_fields(
                row,
                &[
                    "sp_id",
                    "input_scope",
                    "observed_unix_ms",
                    "scan_public",
                    "spend_public",
                    "coinbase_key_status",
                    "registration_presence",
                    "registration",
                    "server",
                    "epoch_mailbox",
                    "mailbox_selection",
                    "commit_provenance",
                    "destination_bytes",
                    "outcome",
                    "error_stage",
                    "error_reason",
                ],
            )?;
            if !row["registration"].is_null() {
                public_fields(
                    &row["registration"],
                    &[
                        "ark_id",
                        "b_ark",
                        "mailbox_mode",
                        "mailbox_m",
                        "mailbox_d",
                        "epoch_fallback",
                        "commit",
                    ],
                )?;
                if !row["registration"]["commit"].is_null() {
                    public_fields(
                        &row["registration"]["commit"],
                        &["request_id", "registration_rev", "signed_fields_digest"],
                    )?;
                }
            }
            if !row["server"].is_null() {
                public_fields(
                    &row["server"],
                    &["ark_id", "server_pubkey", "mailbox_pubkey", "testnet"],
                )?;
            }
            if !row["epoch_mailbox"].is_null() {
                public_fields(&row["epoch_mailbox"], &["height", "mailbox_mi"])?;
            }
        }
        Ok(())
    }
    pub fn field(&self) -> String {
        format!(
            "{}:{}",
            hex::encode(self.session),
            hex::encode(self.attempt)
        )
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let bytes =
            serde_json::to_vec(self).map_err(|_| StoreError::Invalid("front observation JSON"))?;
        if bytes.len() > 8 * 1024 * 1024 {
            return Err(StoreError::Invalid("front observation bound"));
        }
        Ok(bytes)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > 8 * 1024 * 1024 {
            return Err(StoreError::Invalid("front observation bound"));
        }
        let value: Self = serde_json::from_slice(bytes)
            .map_err(|_| StoreError::Invalid("front observation JSON"))?;
        value.validate()?;
        Ok(value)
    }
}

fn public_fields(value: &Value, allowed: &[&str]) -> Result<()> {
    let object = value
        .as_object()
        .ok_or(StoreError::Invalid("public observation object"))?;
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(StoreError::Invalid(
            "non-public or unknown observation field",
        ));
    }
    Ok(())
}

pub fn observation_digest(bytes: &[u8]) -> String {
    use bitcoin::hashes::{sha256, Hash};
    hex::encode(sha256::Hash::hash(bytes).to_byte_array())
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CanonicalMemberRead {
    pub height: u32,
    pub expected_hash: [u8; 32],
    pub observed_hash: Option<[u8; 32]>,
    pub read_status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CanonicalRead {
    pub schema: String,
    pub source_session: [u8; 16],
    pub started_unix_ms: Option<u64>,
    pub finished_unix_ms: Option<u64>,
    pub authoritative: bool,
    pub tip: Option<u32>,
    pub tip_hash: Option<[u8; 32]>,
    pub tip_after: Option<u32>,
    pub tip_hash_after: Option<[u8; 32]>,
    pub members: Vec<CanonicalMemberRead>,
    pub outcome: String,
}

pub struct DeliveryRead {
    pub observation_id: i64,
    pub delivered: BTreeSet<[u8; 32]>,
}

impl CanonicalRead {
    pub fn canonical(&self) -> Option<(u32, [u8; 32], BTreeSet<[u8; 32]>)> {
        if self.schema != "spc.canonical-read/1"
            || self.outcome != "complete"
            || !self.authoritative
            || self.started_unix_ms.is_none()
            || self.finished_unix_ms.is_none()
            || self.finished_unix_ms < self.started_unix_ms
            || self.tip != self.tip_after
            || self.tip_hash != self.tip_hash_after
        {
            return None;
        }
        let mut hashes = BTreeSet::new();
        for member in &self.members {
            if member.read_status != "present" {
                return None;
            }
            let actual = member.observed_hash?;
            if actual == member.expected_hash {
                hashes.insert(actual);
            }
        }
        Some((self.tip?, self.tip_hash?, hashes))
    }
}

/// Atomic append and exact-attempt dedupe. Conflicts never rewrite evidence.
pub async fn append(
    connection: &mut PgConnection,
    kind: &str,
    subject: &[u8],
    attempt: Option<&[u8; 16]>,
    body: &Value,
) -> Result<i64> {
    let row = sqlx::query(
        "INSERT INTO source_decision_observation(kind,subject,attempt,body) VALUES($1,$2,$3,$4)
        ON CONFLICT(kind,subject,attempt) DO NOTHING RETURNING observation_id",
    )
    .bind(kind)
    .bind(subject)
    .bind(attempt.map(|a| &a[..]))
    .bind(body)
    .fetch_optional(&mut *connection)
    .await?;
    if let Some(row) = row {
        return Ok(row.try_get("observation_id")?);
    }
    let row=sqlx::query("SELECT observation_id,body FROM source_decision_observation WHERE kind=$1 AND subject=$2 AND attempt=$3")
        .bind(kind).bind(subject).bind(attempt.map(|a| &a[..])).fetch_one(connection).await?;
    if row.get::<Value, _>("body") != *body {
        return Err(StoreError::Invalid("conflicting observation replay"));
    }
    Ok(row.try_get("observation_id")?)
}

pub fn block_binding(event: &BlockEvent) -> Result<Value> {
    Ok(
        json!({"block_hash":hex::encode(event.block_hash),"height":event.height,
        "frozen_job":event.economic,"payload_digest":hex::encode(event.snapshot_id().map_err(|_|StoreError::Invalid("observation payload"))?)}),
    )
}

impl Store {
    /// Resolve the exact cached request/revision, never the current delivery
    /// row. Missing immutable request provenance remains explicitly unknown.
    pub async fn registration_commit_digest(
        &self,
        id: &[u8; 32],
        commit: &crate::models::RegistrationCommit,
    ) -> Result<Option<String>> {
        Ok(sqlx::query_scalar("SELECT encode(signed_fields_digest,'hex') FROM ark_registration_request
            WHERE sp_id=$1 AND request_id=$2 AND registration_rev=$3 AND outcome IN ('applied','superseded')")
            .bind(&id[..]).bind(i64::from(commit.request_id)).bind(i64::try_from(commit.registration_rev).map_err(|_|StoreError::Invalid("registration revision"))?)
            .fetch_optional(self.pool()).await?)
    }
    /// Payout-only persistence. The Ark role receives only the opaque ACK.
    pub async fn observe_authorization_consumption(
        &self,
        consumed: &spc_ext::authorization::Consumption,
    ) -> Result<Option<spc_ext::authorization::ConsumptionAck>> {
        use spc_ext::authorization::ConsumptionAck;
        let request = &consumed.grant.request;
        let mut tx = self.pool().begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(1397769025)")
            .execute(&mut *tx)
            .await?;
        let body = json!({"schema":"spc.authorization-consumption/1","status":"recorded","request_bytes":hex::encode(consumed.encode()),
            "request_digest":hex::encode(consumed.digest()),"grant_bytes":hex::encode(consumed.grant.encode()),
            "consumed_at_unix_ms":consumed.observed_unix_ms,"meaning":"lease_consumed_before_possible_cosign_not_delivery"});
        let existing=sqlx::query("SELECT observation_id,body,(extract(epoch FROM observed_at)*1000)::bigint AS recorded_ms
            FROM source_decision_observation WHERE kind='authorization_consumption' AND subject=$1 AND attempt=$2")
            .bind(&request.order_id[..]).bind(&request.nonce[..]).fetch_optional(&mut *tx).await?;
        if let Some(row) = existing {
            if row.get::<Value, _>("body") != body {
                return Err(StoreError::Invalid("conflicting consumption replay"));
            }
            return Ok(Some(ConsumptionAck {
                digest: consumed.digest(),
                observation_id: row.get::<i64, _>("observation_id") as u64,
                recorded_unix_ms: row.get::<i64, _>("recorded_ms") as u64,
            }));
        }
        let Some(row)=sqlx::query("SELECT grant_bytes,expires_at FROM ark_send_authorization WHERE order_id=$1 AND nonce=$2 FOR UPDATE")
            .bind(&request.order_id[..]).bind(&request.nonce[..]).fetch_optional(&mut *tx).await? else {
                let mut unknown=body.clone();unknown["status"]="unknown".into();unknown["reason"]="grant_row_absent".into();
                append(&mut tx,"authorization_consumption",&request.order_id,None,&unknown).await?;
                tx.commit().await?;return Ok(None)
            };
        let now: i64 =
            sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp())*1000)::bigint")
                .fetch_one(&mut *tx)
                .await?;
        let expires = row.get::<i64, _>("expires_at") as u64;
        let actor = consumed.observed_unix_ms / 1000;
        if row.get::<Vec<u8>, _>("grant_bytes") != consumed.grant.encode()
            || now < 0
            || now as u64 / 1000 >= expires
            || (now as u64 / 1000)
                < expires.saturating_sub(spc_ext::authorization::GRANT_LIFETIME_SECS)
            || actor >= expires
            || actor < expires.saturating_sub(spc_ext::authorization::GRANT_LIFETIME_SECS)
        {
            let mut unknown = body.clone();
            unknown["status"] = "unknown".into();
            unknown["reason"] = "grant_mismatch_or_expired".into();
            unknown["observer_unix_ms"] = now.into();
            append(
                &mut tx,
                "authorization_consumption",
                &request.order_id,
                None,
                &unknown,
            )
            .await?;
            tx.commit().await?;
            return Ok(None);
        }
        let id = append(
            &mut tx,
            "authorization_consumption",
            &request.order_id,
            Some(&request.nonce),
            &body,
        )
        .await?;
        let recorded:i64=sqlx::query_scalar("SELECT (extract(epoch FROM observed_at)*1000)::bigint FROM source_decision_observation WHERE observation_id=$1")
            .bind(id).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        Ok(Some(ConsumptionAck {
            digest: consumed.digest(),
            observation_id: id as u64,
            recorded_unix_ms: recorded as u64,
        }))
    }
    pub async fn record_observation(
        &self,
        kind: &str,
        subject: &[u8],
        attempt: Option<&[u8; 16]>,
        body: &Value,
    ) -> Result<i64> {
        let mut tx = self.pool().begin().await?;
        let id = append(&mut tx, kind, subject, attempt, body).await?;
        tx.commit().await?;
        Ok(id)
    }

    /// The returned set is exactly the legacy delivery-row existence decision.
    /// Census, returned rows, committed metadata and explicit absences share
    /// the repeatable-read transaction that durably records this actual read.
    pub async fn observe_settlement_delivery_read(
        &self,
        event: &BlockEvent,
        ids: &[[u8; 32]],
        attempt: &[u8; 16],
    ) -> Result<DeliveryRead> {
        let binding = block_binding(event)?;
        let queried: Vec<_> = ids.iter().map(hex::encode).collect();
        let read:Result<DeliveryRead>=async {
            let mut tx=self.pool().begin().await?;
            sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ").execute(&mut *tx).await?;
            let keys:Vec<Vec<u8>>=ids.iter().map(|id|id.to_vec()).collect();
            let rows=sqlx::query("SELECT q.id,q.ordinal,CASE WHEN d.sp_id IS NULL THEN NULL ELSE to_jsonb(d) END AS delivery,
                CASE WHEN s.sp_id IS NULL THEN NULL ELSE to_jsonb(s) END AS registration_state,
                CASE WHEN r.sp_id IS NULL THEN NULL ELSE to_jsonb(r) END AS registration_request,
                CASE WHEN m.sp_id IS NULL THEN NULL ELSE to_jsonb(m) END AS epoch_mailbox
                FROM unnest($1::bytea[]) WITH ORDINALITY AS q(id,ordinal)
                LEFT JOIN ark_delivery d ON d.sp_id=q.id
                LEFT JOIN ark_registration_state s ON s.sp_id=q.id
                LEFT JOIN ark_registration_request r ON r.sp_id=s.sp_id AND r.request_id=s.last_request_id
                LEFT JOIN ark_epoch_mailbox m ON m.sp_id=q.id AND m.height=$2 ORDER BY q.ordinal")
                .bind(&keys).bind(i32::try_from(event.height).map_err(|_|StoreError::Invalid("read height"))?)
                .fetch_all(&mut *tx).await?;
            if rows.len()!=ids.len(){return Err(StoreError::Invalid("incomplete delivery census"));}
            let mut results=Vec::new();let mut delivered=BTreeSet::new();
            for (row,expected) in rows.into_iter().zip(ids) {
                let id:Vec<u8>=row.try_get("id")?;
                if id!=expected[..]{return Err(StoreError::Invalid("delivery census ordering"));}
                let delivery:Option<Value>=row.try_get("delivery")?;
                if delivery.is_some(){delivered.insert(*expected);}
                results.push(json!({"sp_id":hex::encode(expected),"delivery":delivery,
                    "registration_state":row.try_get::<Option<Value>,_>("registration_state")?,
                    "registration_request":row.try_get::<Option<Value>,_>("registration_request")?,
                    "epoch_mailbox":row.try_get::<Option<Value>,_>("epoch_mailbox")?}));
            }
            let body=json!({"schema":"spc.settlement-delivery-read/1","binding":binding,"queried":queried,
                "status":"complete","results":results});
            let observation_id=append(&mut tx,"settlement_delivery_read",&event.block_hash,Some(attempt),&body).await?;
            tx.commit().await?;
            Ok(DeliveryRead{observation_id,delivered})
        }.await;
        if read.is_err() {
            // A failed/partial query cannot establish absence. If storage itself
            // is unavailable this append also fails, and reconstruction holds.
            let body = json!({"schema":"spc.settlement-delivery-read/1","binding":binding,"queried":queried,
                "status":"unknown","results":null,"error_class":"read_or_persistence_failed"});
            let _ = self
                .record_observation(
                    "settlement_delivery_read",
                    &event.block_hash,
                    Some(attempt),
                    &body,
                )
                .await;
        }
        read
    }

    pub async fn reconstruction_observation(&self, block: &[u8; 32]) -> Result<Option<i64>> {
        Ok(sqlx::query_scalar(
            "SELECT reconstruction_observation FROM economic_block WHERE block_hash=$1",
        )
        .bind(&block[..])
        .fetch_optional(self.pool())
        .await?
        .flatten())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn front_observation_public_contract_rejects_private_and_unbound_fields() {
        let request = FrontObservation {
            schema_version: 1,
            session: [1; 16],
            attempt: [2; 16],
            body: json!({
            "schema":"spc.front-derivation/1","source_session":hex::encode([1;16]),"status":"frozen_job",
            "job_id":1,"generation":1,"height":1,"a_send_x":hex::encode([6;32]),"distribution_fingerprint":hex::encode([7;32]),
            "queried_identity_census":[hex::encode([3;32])],"census_complete":true,
            "rows":[{"sp_id":hex::encode([3;32]),"registration":null,"server":null,"epoch_mailbox":null,"outcome":"missing_miner_keys"}]}),
        };
        let bytes = request.encode().unwrap();
        assert_eq!(
            FrontObservation::decode(&bytes).unwrap().field(),
            request.field()
        );
        let mut missing = request.clone();
        missing.body.as_object_mut().unwrap().remove("job_id");
        assert!(missing.encode().is_err());
        let mut private = request.clone();
        private.body["rows"][0]["scan_secret"] = hex::encode([4; 32]).into();
        assert!(private.encode().is_err());
        let mut private = request.clone();
        private.body["a_send"] = hex::encode([4; 32]).into();
        assert!(private.encode().is_err());
        let mut wrong = request.clone();
        wrong.body["source_session"] = hex::encode([9; 16]).into();
        assert!(wrong.encode().is_err());
        let mut extra = serde_json::to_value(&request).unwrap();
        extra["private_key"] = "bad".into();
        assert!(FrontObservation::decode(&serde_json::to_vec(&extra).unwrap()).is_err());
        assert!(serde_json::from_value::<FrontObservationAck>(
            json!({"digest":"x","observation_id":1,"unexpected":true})
        )
        .is_err());
    }
}
