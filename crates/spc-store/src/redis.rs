//! Thin Redis stream helpers. Payloads are the byte formats in
//! `docs/WIRE-FORMATS.md`; every entry has one field, `d`.

use redis::aio::MultiplexedConnection;
use redis::{AsyncCommands, Value};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RedisError {
    #[error("redis: {0}")]
    Redis(#[from] redis::RedisError),
    #[error("malformed stream entry")]
    Malformed,
}

pub type Result<T> = std::result::Result<T, RedisError>;

#[derive(Clone)]
pub struct Streams {
    conn: MultiplexedConnection,
    client: redis::Client,
}

impl Streams {
    pub async fn connect(url: &str) -> Result<Self> {
        let client = redis::Client::open(url)?;
        let conn = client.get_multiplexed_async_connection().await?;
        Ok(Self { conn, client })
    }
    pub async fn independent(&self) -> Result<Self> {
        Ok(Self {
            conn: self.client.get_multiplexed_async_connection().await?,
            client: self.client.clone(),
        })
    }

    pub async fn add(&self, stream: &str, payload: &[u8]) -> Result<String> {
        let mut conn = self.conn.clone();
        let id: String = conn.xadd(stream, "*", &[("d", payload)]).await?;
        Ok(id)
    }

    pub async fn ensure_group(&self, stream: &str, group: &str) -> Result<()> {
        self.ensure_group_from(stream, group, "$").await
    }

    /// Like [`Self::ensure_group`], but a first-created group starts at
    /// `start_id` instead of `$`. Use `"0"` for a stream whose pre-existing
    /// entries must not be lost (e.g. `spc:ark:receipts` across a payout
    /// restart). An existing group is left untouched (`BUSYGROUP`).
    pub async fn ensure_group_from(&self, stream: &str, group: &str, start_id: &str) -> Result<()> {
        let mut conn = self.conn.clone();
        let res: redis::RedisResult<()> =
            conn.xgroup_create_mkstream(stream, group, start_id).await;
        match res {
            Ok(()) => Ok(()),
            Err(e) if e.to_string().contains("BUSYGROUP") => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    /// Read new entries for `group`/`consumer`. Returns `(entry_id, payload)`.
    pub async fn read_group(
        &self,
        stream: &str,
        group: &str,
        consumer: &str,
        count: usize,
    ) -> Result<Vec<(String, Vec<u8>)>> {
        let mut conn = self.conn.clone();
        let value: Value = redis::cmd("XREADGROUP")
            .arg("GROUP")
            .arg(group)
            .arg(consumer)
            .arg("COUNT")
            .arg(count)
            .arg("BLOCK")
            .arg(1000)
            .arg("STREAMS")
            .arg(stream)
            .arg(">")
            .query_async(&mut conn)
            .await?;
        parse_xread(value)
    }

    pub async fn ack(&self, stream: &str, group: &str, id: &str) -> Result<()> {
        let mut conn = self.conn.clone();
        let _: i64 = conn.xack(stream, group, &[id]).await?;
        Ok(())
    }

    /// Re-read this consumer's pending (unacknowledged) entries. This is the
    /// retry surface for held settlements: entries that were not acked are
    /// returned here, oldest first, without blocking (SPEC §7.4).
    pub async fn read_pending(
        &self,
        stream: &str,
        group: &str,
        consumer: &str,
        count: usize,
    ) -> Result<Vec<(String, Vec<u8>)>> {
        self.read_pending_after(stream, group, consumer, count, "0")
            .await
    }
    pub async fn read_pending_after(
        &self,
        stream: &str,
        group: &str,
        consumer: &str,
        count: usize,
        after: &str,
    ) -> Result<Vec<(String, Vec<u8>)>> {
        let mut conn = self.conn.clone();
        let value: Value = redis::cmd("XREADGROUP")
            .arg("GROUP")
            .arg(group)
            .arg(consumer)
            .arg("COUNT")
            .arg(count)
            .arg("STREAMS")
            .arg(stream)
            .arg(after)
            .query_async(&mut conn)
            .await?;
        parse_xread(value)
    }

    /// Add `member` to the set at `key`, returning true when it was not
    /// already present (atomic mark-if-absent).
    pub async fn sadd(&self, key: &str, member: &[u8]) -> Result<bool> {
        let mut conn = self.conn.clone();
        let added: i64 = conn.sadd(key, member).await?;
        Ok(added > 0)
    }

    pub async fn sismember(&self, key: &str, member: &[u8]) -> Result<bool> {
        let mut conn = self.conn.clone();
        Ok(conn.sismember(key, member).await?)
    }

    /// Remove `member` from the set at `key`; returns true when it was
    /// present. Used to clear the Ark "ambiguous order" pin once a receipt
    /// resolves it.
    pub async fn srem(&self, key: &str, member: &[u8]) -> Result<bool> {
        let mut conn = self.conn.clone();
        let removed: i64 = conn.srem(key, member).await?;
        Ok(removed > 0)
    }

    /// Write one field of an SPc-side hash (used by `spc:arkreg:ack`).
    pub async fn hset(&self, key: &str, field: &str, value: &str) -> Result<()> {
        let mut conn = self.conn.clone();
        let _: () = conn.hset(key, field, value).await?;
        Ok(())
    }

    /// Read one field of an SPc-side hash. A missing key/field is a normal
    /// cache miss and must be resolved from Postgres by the payout path.
    pub async fn hget(&self, key: &str, field: &str) -> Result<Option<String>> {
        let mut conn = self.conn.clone();
        Ok(conn.hget(key, field).await?)
    }

    /// Refresh `key`'s TTL to `secs`; a missing key is a no-op.
    pub async fn expire(&self, key: &str, secs: u64) -> Result<()> {
        let mut conn = self.conn.clone();
        let secs = i64::try_from(secs).unwrap_or(i64::MAX);
        let _: bool = conn.expire(key, secs).await?;
        Ok(())
    }

    /// Trim behind the slowest live reader. The caller owns the policy
    /// (SPEC §5.2); this is the mechanical `XTRIM MINID`.
    pub async fn trim_minid(&self, stream: &str, min_id: &str) -> Result<()> {
        let mut conn = self.conn.clone();
        let _: i64 = redis::cmd("XTRIM")
            .arg(stream)
            .arg("MINID")
            .arg(min_id)
            .query_async(&mut conn)
            .await?;
        Ok(())
    }

    pub async fn len(&self, stream: &str) -> Result<u64> {
        let mut conn = self.conn.clone();
        Ok(conn.xlen(stream).await?)
    }

    /// Every member of a set (the raw bytes written by `sadd`).
    pub async fn smembers(&self, key: &str) -> Result<Vec<Vec<u8>>> {
        let mut conn = self.conn.clone();
        Ok(conn.smembers(key).await?)
    }

    /// Read a plain string key.
    pub async fn kv_get(&self, key: &str) -> Result<Option<String>> {
        let mut conn = self.conn.clone();
        Ok(conn.get(key).await?)
    }

    /// Write a plain string key.
    pub async fn kv_set(&self, key: &str, value: &str) -> Result<()> {
        let mut conn = self.conn.clone();
        let () = conn.set(key, value).await?;
        Ok(())
    }

    /// Write a plain string key with a TTL (`SET ... EX`), atomically.
    /// Used by the role metric snapshots (`spc:metrics:*`): a role that
    /// stops publishing lets its snapshot expire, so a stale value is never
    /// served as current.
    pub async fn kv_set_ex(&self, key: &str, value: &str, ttl_secs: u64) -> Result<()> {
        let mut conn = self.conn.clone();
        let () = conn.set_ex(key, value, ttl_secs).await?;
        Ok(())
    }

    /// Keys matching `pattern` (cursor-driven `SCAN ... MATCH`). For small
    /// operational namespaces such as `spc:metrics:front:*`; never a
    /// data-plane primitive.
    pub async fn scan_keys(&self, pattern: &str) -> Result<Vec<String>> {
        let mut conn = self.conn.clone();
        let mut cursor: u64 = 0;
        let mut out = Vec::new();
        loop {
            let (next, batch): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(pattern)
                .arg("COUNT")
                .arg(100)
                .query_async(&mut conn)
                .await?;
            out.extend(batch);
            cursor = next;
            if cursor == 0 {
                break;
            }
        }
        out.sort();
        Ok(out)
    }

    /// Worst consumer-group delivery lag of `stream`, in milliseconds: the
    /// age of the oldest entry not yet delivered to each group (entry ids
    /// are millisecond timestamps), maxed over the stream's groups. A group
    /// whose `last-delivered-id` has reached `last-generated-id` is caught
    /// up (0).
    ///
    /// `Ok(None)` when the stream does not exist or has no consumer group:
    /// absent means unknown, never "caught up". A delivered-but-unacked
    /// entry does not count as lag here; this measures how far behind the
    /// stream's newest entries delivery has fallen
    /// (`spc_stream_lag_seconds`, SPEC §9).
    pub async fn group_lag_ms(&self, stream: &str) -> Result<Option<u64>> {
        let mut conn = self.conn.clone();
        let info: Value = match redis::cmd("XINFO")
            .arg("STREAM")
            .arg(stream)
            .query_async(&mut conn)
            .await
        {
            Ok(v) => v,
            // "ERR no such key": the stream was never written.
            Err(e)
                if e.kind() == redis::ErrorKind::ResponseError
                    && e.to_string().contains("no such key") =>
            {
                return Ok(None)
            }
            Err(e) => return Err(e.into()),
        };
        let Some(last_id) = reply_field(&info, "last-generated-id") else {
            return Err(RedisError::Malformed);
        };
        let Some(last_ms) = id_ms(&last_id) else {
            return Err(RedisError::Malformed);
        };
        let groups: Value = redis::cmd("XINFO")
            .arg("GROUPS")
            .arg(stream)
            .query_async(&mut conn)
            .await?;
        let Value::Array(groups) = groups else {
            return Err(RedisError::Malformed);
        };
        let now_ms = u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0),
        )
        .unwrap_or(u64::MAX);
        let mut worst: Option<u64> = None;
        for group in &groups {
            let Some(delivered_id) = reply_field(group, "last-delivered-id") else {
                return Err(RedisError::Malformed);
            };
            let Some(delivered_ms) = id_ms(&delivered_id) else {
                return Err(RedisError::Malformed);
            };
            let lag = if delivered_ms >= last_ms && delivered_id != "0-0" {
                // At (or past) the newest entry. Sub-millisecond sequence
                // distance is not an operational lag.
                0
            } else {
                // The oldest undelivered entry, exclusive of what was
                // already delivered.
                let range: Value = redis::cmd("XRANGE")
                    .arg(stream)
                    .arg(format!("({delivered_id}"))
                    .arg("+")
                    .arg("COUNT")
                    .arg(1)
                    .query_async(&mut conn)
                    .await?;
                match first_entry_id(&range) {
                    Some(id) => now_ms.saturating_sub(id_ms(&id).unwrap_or(now_ms)),
                    // Everything after `delivered_id` was trimmed: caught up.
                    None => 0,
                }
            };
            worst = Some(worst.unwrap_or(0).max(lag));
        }
        Ok(worst)
    }
}

/// Look up `field` in an `XINFO` reply, which is a flat key/value array on
/// RESP2 and a map on RESP3.
fn reply_field(value: &Value, field: &str) -> Option<String> {
    let as_string = |v: &Value| match v {
        Value::BulkString(b) => Some(String::from_utf8_lossy(b).to_string()),
        Value::SimpleString(s) => Some(s.clone()),
        _ => None,
    };
    match value {
        Value::Array(items) => {
            let mut i = 0;
            while i + 1 < items.len() {
                if as_string(&items[i]).as_deref() == Some(field) {
                    return as_string(&items[i + 1]);
                }
                i += 2;
            }
            None
        }
        Value::Map(pairs) => pairs
            .iter()
            .find(|(k, _)| as_string(k).as_deref() == Some(field))
            .and_then(|(_, v)| as_string(v)),
        _ => None,
    }
}

/// The millisecond prefix of a stream entry id (`<ms>-<seq>`).
fn id_ms(id: &str) -> Option<u64> {
    id.split('-').next()?.parse().ok()
}

/// The id of the first entry in an `XRANGE` reply, if any.
fn first_entry_id(value: &Value) -> Option<String> {
    let Value::Array(entries) = value else {
        return None;
    };
    let Value::Array(pair) = entries.first()? else {
        return None;
    };
    match pair.first()? {
        Value::BulkString(b) => Some(String::from_utf8_lossy(b).to_string()),
        Value::SimpleString(s) => Some(s.clone()),
        _ => None,
    }
}

fn parse_xread(value: Value) -> Result<Vec<(String, Vec<u8>)>> {
    let Value::Array(streams) = value else {
        return Ok(vec![]);
    };
    let mut out = Vec::new();
    for stream in streams {
        let Value::Array(parts) = stream else {
            continue;
        };
        if parts.len() != 2 {
            continue;
        }
        let Value::Array(entries) = &parts[1] else {
            continue;
        };
        for entry in entries {
            let Value::Array(pair) = entry else { continue };
            if pair.len() != 2 {
                continue;
            }
            let id = match &pair[0] {
                Value::BulkString(b) => String::from_utf8_lossy(b).to_string(),
                Value::SimpleString(s) => s.clone(),
                _ => continue,
            };
            let Value::Array(fields) = &pair[1] else {
                continue;
            };
            let mut payload = None;
            let mut i = 0;
            while i + 1 < fields.len() {
                if matches!(&fields[i], Value::BulkString(k) if k == b"d") {
                    if let Value::BulkString(v) = &fields[i + 1] {
                        payload = Some(v.clone());
                    }
                }
                i += 2;
            }
            if let Some(p) = payload {
                out.push((id, p));
            }
        }
    }
    Ok(out)
}
