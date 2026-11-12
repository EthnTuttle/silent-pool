//! Bounded, incarnation-bound native JSONL control. Durable command identity
//! is independent of the monotonically increasing per-pipe request identity.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{self, BufRead, Write};

pub const PROTOCOL: &str = "spc-lab-tx-control/v1";
pub const MAX_BODY: usize = 16 * 1024 * 1024;
pub const MAX_LINE: usize = MAX_BODY + 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrafficEventKind {
    Planned,
    Deferred,
    Signed,
    AcceptedByRpc,
    Unknown,
    Confirmed,
    Reorged,
    SizeRefused,
    ObserverHealth,
    Rejected,
    RateMeasured,
}

/// Closed evidence projection. Deliberately excludes raw bytes, scripts,
/// outpoints, command payloads, allocation users and all recovery maps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrafficObservation {
    pub requested_interval_ms: Option<u64>,
    pub measured_elapsed_ms: Option<u64>,
    pub ordinary_broadcast_count: Option<u64>,
    pub event: TrafficEventKind,
    pub journal_seq: u64,
    pub observed_wall_ms: u64,
    pub observed_monotonic_ms: u64,
    pub planned_monotonic_ms: Option<u64>,
    pub schedule_sequence: Option<u64>,
    pub requested_vbytes: Option<u64>,
    pub txid: Option<String>,
    pub vbytes: Option<u64>,
    pub weight_wu: Option<u64>,
    pub fee_sats: Option<u64>,
    pub fee_rate_sats_vb: Option<u64>,
    pub category: Option<String>,
    pub wallet: Option<String>,
    pub healthy: Option<bool>,
}
impl TrafficObservation {
    pub fn empty(event: TrafficEventKind) -> Self {
        Self {
            requested_interval_ms: None,
            measured_elapsed_ms: None,
            ordinary_broadcast_count: None,
            event,
            journal_seq: 0,
            observed_wall_ms: 0,
            observed_monotonic_ms: 0,
            planned_monotonic_ms: None,
            schedule_sequence: None,
            requested_vbytes: None,
            txid: None,
            vbytes: None,
            weight_wu: None,
            fee_sats: None,
            fee_rate_sats_vb: None,
            category: None,
            wallet: None,
            healthy: None,
        }
    }
    pub fn health(healthy: bool) -> Self {
        let mut event = Self::empty(TrafficEventKind::ObserverHealth);
        event.healthy = Some(healthy);
        event
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnknownClass {
    Transport,
    Broker,
    Timeout,
    Protocol,
    Shape,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum RpcOutcome {
    Ok { result: Value },
    NodeRejected { code: i64, message: String },
    Unknown { class: UnknownClass },
    GateClosed,
}

impl RpcOutcome {
    pub fn unknown(class: UnknownClass) -> Self {
        Self::Unknown { class }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RpcRequest {
    pub node_id: String,
    pub method: String,
    pub params: Value,
    /// Required only for a send: binds to the committed journal intent.
    pub send_sequence: Option<u64>,
    pub txid: Option<String>,
}

/// Private, observation-only binding of an actor handoff to its durable core.
/// These numbers cannot authorize a send; the core recheck precedes this wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForwardContext {
    pub command_id: Option<String>,
    pub journal_seq: u64,
    pub chain_revision: Option<u64>,
    pub intent_seq: Option<u64>,
    pub raw_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForwardRequest {
    pub request: RpcRequest,
    pub context: ForwardContext,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedCheckpoint {
    pub command_id: String,
    pub raw_sha256: String,
    pub journal_seq: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Message {
    Request {
        protocol: String,
        incarnation: String,
        driver_id: String,
        request_id: u64,
        command_id: Option<String>,
        operation: String,
        payload: Value,
    },
    Response {
        protocol: String,
        incarnation: String,
        driver_id: String,
        request_id: u64,
        command_id: Option<String>,
        result: Value,
    },
    Event {
        protocol: String,
        incarnation: String,
        driver_id: String,
        event_id: u64,
        name: String,
        payload: Value,
    },
}

impl Message {
    pub fn validate_binding(&self, driver: &str, incarnation: &str) -> io::Result<()> {
        let (p, d, i) = match self {
            Self::Request {
                protocol,
                driver_id,
                incarnation,
                ..
            }
            | Self::Response {
                protocol,
                driver_id,
                incarnation,
                ..
            }
            | Self::Event {
                protocol,
                driver_id,
                incarnation,
                ..
            } => (protocol, driver_id, incarnation),
        };
        if p != PROTOCOL || d != driver || i != incarnation {
            return Err(invalid("control protocol/driver/incarnation mismatch"));
        }
        Ok(())
    }
}

pub fn invalid(reason: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason)
}

/// Bounded before allocation; a partial EOF line is a protocol failure.
pub fn read_message(reader: &mut impl BufRead) -> io::Result<Option<Message>> {
    let mut bytes = Vec::new();
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            return if bytes.is_empty() {
                Ok(None)
            } else {
                Err(invalid("partial control line"))
            };
        }
        let end = buffer.iter().position(|b| *b == b'\n').map(|n| n + 1);
        let take = end.unwrap_or(buffer.len());
        if bytes.len().checked_add(take).is_none_or(|n| n > MAX_LINE) {
            return Err(invalid("control line too large"));
        }
        bytes.extend_from_slice(&buffer[..take]);
        reader.consume(take);
        if end.is_some() {
            let value = strict_json(&bytes).map_err(|_| invalid("malformed control JSON"))?;
            return serde_json::from_value(value)
                .map(Some)
                .map_err(|_| invalid("malformed control message"));
        }
    }
}

pub fn write_message(writer: &mut impl Write, message: &Message) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(message)?;
    if bytes.len() >= MAX_LINE {
        return Err(invalid("control line too large"));
    }
    bytes.push(b'\n');
    writer.write_all(&bytes)?;
    writer.flush()
}

/// A single pipe admits exactly the next wire request. Duplicate wire
/// delivery is refused; durable command retries use a fresh wire id and the
/// original command id, and are resolved by the one existing journal.
pub struct RequestSequence {
    next: u64,
}
impl Default for RequestSequence {
    fn default() -> Self {
        Self { next: 1 }
    }
}
impl RequestSequence {
    pub fn accept(&mut self, id: u64) -> io::Result<()> {
        if id != self.next {
            return Err(invalid("duplicate or out-of-order request"));
        }
        self.next = self
            .next
            .checked_add(1)
            .ok_or_else(|| invalid("request sequence exhausted"))?;
        Ok(())
    }
}

/// Response-id validation happens before inspecting an error. A parsed node
/// rejection is data, not a definitive send outcome. Money decimals are never
/// consumed by this lane.
pub fn parse_rpc_response(bytes: &[u8], request_id: u64) -> RpcOutcome {
    if bytes.len() > MAX_BODY {
        return RpcOutcome::unknown(UnknownClass::Shape);
    }
    #[derive(Deserialize)]
    struct Response {
        id: Value,
        #[serde(default)]
        result: Value,
        #[serde(default)]
        error: Value,
    }
    let value = match strict_json(bytes) {
        Ok(value) => value,
        Err(_) => return RpcOutcome::unknown(UnknownClass::Protocol),
    };
    let has_result = value
        .as_object()
        .is_some_and(|object| object.contains_key("result"));
    let response = match serde_json::from_value::<Response>(value) {
        Ok(response) => response,
        Err(_) => return RpcOutcome::unknown(UnknownClass::Protocol),
    };
    if response.id.as_u64() != Some(request_id) {
        return RpcOutcome::unknown(UnknownClass::Protocol);
    }
    if response.error.is_null() {
        if !has_result {
            return RpcOutcome::unknown(UnknownClass::Shape);
        }
        return RpcOutcome::Ok {
            result: response.result,
        };
    }
    if !response.result.is_null() {
        return RpcOutcome::unknown(UnknownClass::Shape);
    }
    let Some(code) = response.error.get("code").and_then(Value::as_i64) else {
        return RpcOutcome::unknown(UnknownClass::Shape);
    };
    let Some(message) = response.error.get("message").and_then(Value::as_str) else {
        return RpcOutcome::unknown(UnknownClass::Shape);
    };
    if message.len() > 4096 {
        return RpcOutcome::unknown(UnknownClass::Shape);
    }
    RpcOutcome::NodeRejected {
        code,
        message: message.to_owned(),
    }
}

/// Reject duplicate object keys at every nesting level before typed parsing.
/// serde_json::Value alone would silently replace an earlier field.
pub fn strict_json(bytes: &[u8]) -> Result<Value, serde_json::Error> {
    struct Strict(Value);
    impl<'de> Deserialize<'de> for Strict {
        fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
            struct Visitor;
            impl<'de> serde::de::Visitor<'de> for Visitor {
                type Value = Strict;
                fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    f.write_str("JSON with unique keys")
                }
                fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Strict, E> {
                    Ok(Strict(Value::Bool(v)))
                }
                fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Strict, E> {
                    Ok(Strict(v.into()))
                }
                fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Strict, E> {
                    Ok(Strict(v.into()))
                }
                fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Strict, E> {
                    serde_json::Number::from_f64(v)
                        .map(|n| Strict(Value::Number(n)))
                        .ok_or_else(|| E::custom("nonfinite number"))
                }
                fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Strict, E> {
                    Ok(Strict(v.into()))
                }
                fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Strict, E> {
                    Ok(Strict(v.into()))
                }
                fn visit_unit<E: serde::de::Error>(self) -> Result<Strict, E> {
                    Ok(Strict(Value::Null))
                }
                fn visit_seq<A: serde::de::SeqAccess<'de>>(
                    self,
                    mut seq: A,
                ) -> Result<Strict, A::Error> {
                    let mut out = Vec::new();
                    while let Some(Strict(v)) = seq.next_element()? {
                        out.push(v);
                    }
                    Ok(Strict(Value::Array(out)))
                }
                fn visit_map<A: serde::de::MapAccess<'de>>(
                    self,
                    mut map: A,
                ) -> Result<Strict, A::Error> {
                    let mut out = serde_json::Map::new();
                    while let Some((k, Strict(v))) = map.next_entry::<String, Strict>()? {
                        if out.insert(k, v).is_some() {
                            return Err(serde::de::Error::custom("duplicate JSON key"));
                        }
                    }
                    Ok(Strict(Value::Object(out)))
                }
            }
            de.deserialize_any(Visitor)
        }
    }
    serde_json::from_slice::<Strict>(bytes).map(|v| v.0)
}
