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
}

impl Streams {
    pub async fn connect(url: &str) -> Result<Self> {
        let client = redis::Client::open(url)?;
        let conn = client.get_multiplexed_async_connection().await?;
        Ok(Self { conn })
    }

    pub async fn add(&self, stream: &str, payload: &[u8]) -> Result<String> {
        let mut conn = self.conn.clone();
        let id: String = conn.xadd(stream, "*", &[("d", payload)]).await?;
        Ok(id)
    }

    pub async fn ensure_group(&self, stream: &str, group: &str) -> Result<()> {
        let mut conn = self.conn.clone();
        let res: redis::RedisResult<()> = conn.xgroup_create_mkstream(stream, group, "$").await;
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
