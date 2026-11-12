//! Adapter for LP-1's `spc_evidence` tracing records.
//!
//! The front emits tracing JSON with `target = "spc_evidence"`, a fixed
//! `event` name and an `evidence` JSON object. This adapter captures the exact
//! raw payload to a private mode-0600 artifact and maps only fields it can
//! resolve through explicit front/identity mappings. It never turns a miner
//! ACK into an `accounted` observation.

use std::collections::BTreeMap;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::evidence::EventId;
use crate::evidence::{EvidenceActor, EvidencePayload, EvidenceStore, JobActivation, JobIssued};
use crate::ids::{FrontId, IdentityId, JobId, SourceId};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpcEvidenceError {
    #[error("not an spc_evidence record")]
    NotEvidence,
    #[error("spc_evidence record is malformed: {0}")]
    Malformed(String),
    #[error("spc_evidence io: {0}")]
    Io(String),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpcEvidenceMappings {
    /// Explicit scenario front ID -> numeric LP-1 front_id.
    #[serde(default)]
    pub front_ids: BTreeMap<FrontId, u8>,
    /// Explicit scenario identity -> 32-byte `sp_id` hex.
    #[serde(default)]
    pub identity_sp_ids: BTreeMap<IdentityId, String>,
}

impl SpcEvidenceMappings {
    fn front_for(&self, numeric: u8) -> Option<FrontId> {
        self.front_ids
            .iter()
            .find(|(_, value)| **value == numeric)
            .map(|(id, _)| id.clone())
    }

    fn identity_for(&self, sp_id: &str) -> Option<IdentityId> {
        self.identity_sp_ids
            .iter()
            .find(|(_, value)| value.eq_ignore_ascii_case(sp_id))
            .map(|(id, _)| id.clone())
    }
}

/// Exact captured record, including private identity material. This type is
/// only stored in the private evidence stream; the shareable manifest never
/// receives `raw`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpcEvidenceRecord {
    pub event: String,
    pub raw: Value,
    pub front_id: Option<FrontId>,
    pub front_numeric: Option<u8>,
    pub channel_id: Option<u64>,
    pub job_id: Option<JobId>,
    pub generation: Option<u64>,
    pub mapped_identity: Option<IdentityId>,
    pub share_target_endianness: String,
    pub share_target_byte_order: Option<String>,
    pub share_hash: Option<String>,
    pub share_hash_byte_order: Option<String>,
    pub extranonce: Option<String>,
    pub extranonce_byte_order: Option<String>,
    pub note: String,
}

#[derive(Debug, Clone)]
pub struct SpcEvidenceAdapter {
    source: SourceId,
    private_path: PathBuf,
    mappings: SpcEvidenceMappings,
    cursor: u64,
}

impl SpcEvidenceAdapter {
    pub fn new(
        source: SourceId,
        private_path: impl Into<PathBuf>,
        mappings: SpcEvidenceMappings,
    ) -> Result<Self, SpcEvidenceError> {
        let private_path = private_path.into();
        if let Some(parent) = private_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| SpcEvidenceError::Io(e.to_string()))?;
        }
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&private_path)
            .map_err(|e| SpcEvidenceError::Io(e.to_string()))?;
        std::fs::set_permissions(
            &private_path,
            std::os::unix::fs::PermissionsExt::from_mode(0o600),
        )
        .map_err(|e| SpcEvidenceError::Io(e.to_string()))?;
        Ok(Self {
            source,
            private_path,
            mappings,
            cursor: 0,
        })
    }

    pub fn cursor(&self) -> u64 {
        self.cursor
    }

    /// Ingest one tracing JSON line. The line cursor is a source-local line
    /// count, not a global causal order.
    pub fn ingest_line(
        &mut self,
        store: &EvidenceStore,
        line: &str,
    ) -> Result<EventId, SpcEvidenceError> {
        let (event, raw) = parse_tracing_line(line)?;
        self.cursor += 1;
        self.append_private(line)?;
        let front_numeric = raw.get("front_id").and_then(value_u64).map(|v| v as u8);
        let front_id = front_numeric.and_then(|value| self.mappings.front_for(value));
        let channel_id = raw.get("channel_id").and_then(value_u64);
        let job_id = raw
            .get("job_id")
            .and_then(Value::as_str)
            .and_then(|value| JobId::new(value).ok());
        let generation = raw.get("generation").and_then(value_u64);
        let sp_id = raw.get("sp_id").and_then(Value::as_str);
        let mapped_identity = sp_id.and_then(|value| self.mappings.identity_for(value));
        let share_target_endianness = "be256_hex".to_string();
        let share_target_byte_order = raw
            .get("share_target_byte_order")
            .and_then(Value::as_str)
            .map(str::to_string);
        let share_hash = raw
            .get("share_hash")
            .and_then(Value::as_str)
            .map(str::to_string);
        let share_hash_byte_order = raw
            .get("share_hash_byte_order")
            .and_then(Value::as_str)
            .map(str::to_string);
        let extranonce = raw
            .get("extranonce")
            .and_then(Value::as_str)
            .map(str::to_string);
        let extranonce_byte_order = raw
            .get("extranonce_byte_order")
            .and_then(Value::as_str)
            .map(str::to_string);
        let mut note = String::new();

        if event == "job_issued" {
            if let (Some(front_id), Some(channel_id), Some(job_id), Some(generation)) =
                (front_id.clone(), channel_id, job_id.clone(), generation)
            {
                let nbits = raw.get("nbits").and_then(value_u64).map(|v| v as u32);
                let m_issue = raw.get("m_issue").and_then(value_u64);
                let version = raw
                    .get("work_unit_version")
                    .and_then(value_u64)
                    .map(|v| v as u16);
                let share_target = raw
                    .get("share_target")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let fingerprint = raw
                    .get("work_unit_fingerprint")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let activation = match raw.get("activation").and_then(Value::as_str) {
                    Some("future") => Some(JobActivation::Future),
                    Some("active") => Some(JobActivation::Active),
                    _ => None,
                };
                if let (
                    Some(nbits),
                    Some(m_issue),
                    Some(version),
                    Some(share_target),
                    Some(activation),
                ) = (nbits, m_issue, version, share_target, activation)
                {
                    let issued_at = raw.get("issued_at").and_then(value_u64).unwrap_or(0);
                    let job = JobIssued {
                        front_id: front_id.clone(),
                        channel_id,
                        job_id: job_id.clone(),
                        generation,
                        nbits,
                        share_target,
                        m_issue,
                        work_unit_version: version,
                        work_unit_fingerprint: fingerprint.to_string(),
                        issued_at,
                        activation,
                    };
                    return Ok(store.append(
                        self.source.clone(),
                        format!("{}:{}", self.cursor, event),
                        EvidenceActor::new(crate::evidence::ActorKind::Front, front_id.to_string()),
                        None,
                        EvidencePayload::JobIssued(job),
                    ));
                }
                note =
                    "job_issued is missing a required typed field; raw record retained privately"
                        .to_string();
            } else {
                note = "job_issued front_id is not mapped to a scenario front; raw record retained privately"
                    .to_string();
            }
        } else if event == "accounted" && mapped_identity.is_none() {
            note = "accounted sp_id is not mapped to a scenario identity; raw record retained privately"
                .to_string();
        } else if event == "accounted" {
            note = "accounted parsed from the front evidence record; miner ACK is not used"
                .to_string();
        }

        Ok(store.append(
            self.source.clone(),
            format!("{}:{}", self.cursor, event),
            EvidenceActor::new(crate::evidence::ActorKind::Front, "spc_evidence"),
            None,
            EvidencePayload::SpcEvidence(SpcEvidenceRecord {
                event,
                raw,
                front_id,
                front_numeric,
                channel_id,
                job_id,
                generation,
                mapped_identity,
                share_target_endianness,
                share_target_byte_order,
                share_hash,
                share_hash_byte_order,
                extranonce,
                extranonce_byte_order,
                note,
            }),
        ))
    }

    fn append_private(&self, line: &str) -> Result<(), SpcEvidenceError> {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&self.private_path)
            .map_err(|e| SpcEvidenceError::Io(e.to_string()))?;
        file.write_all(line.as_bytes())
            .and_then(|_| file.write_all(b"\n"))
            .and_then(|_| file.sync_data())
            .map_err(|e| SpcEvidenceError::Io(e.to_string()))
    }

    pub fn private_path(&self) -> &Path {
        &self.private_path
    }
}

fn parse_tracing_line(line: &str) -> Result<(String, Value), SpcEvidenceError> {
    let outer: Value =
        serde_json::from_str(line).map_err(|e| SpcEvidenceError::Malformed(e.to_string()))?;
    let target = outer.get("target").and_then(Value::as_str).unwrap_or("");
    if target != "spc_evidence" {
        return Err(SpcEvidenceError::NotEvidence);
    }
    let fields = outer
        .get("fields")
        .and_then(Value::as_object)
        .ok_or_else(|| SpcEvidenceError::Malformed("missing fields object".to_string()))?;
    let event = fields
        .get("event")
        .and_then(Value::as_str)
        .or_else(|| outer.get("event").and_then(Value::as_str))
        .ok_or_else(|| SpcEvidenceError::Malformed("missing event name".to_string()))?
        .to_string();
    let evidence = fields
        .get("evidence")
        .or_else(|| outer.get("evidence"))
        .ok_or_else(|| SpcEvidenceError::Malformed("missing evidence payload".to_string()))?;
    let raw = match evidence {
        Value::String(text) => serde_json::from_str(text)
            .map_err(|e| SpcEvidenceError::Malformed(format!("evidence JSON: {e}")))?,
        Value::Object(_) => evidence.clone(),
        _ => {
            return Err(SpcEvidenceError::Malformed(
                "evidence payload is not JSON".to_string(),
            ));
        }
    };
    Ok((event, raw))
}

fn value_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|text| text.parse::<u64>().ok()))
}
