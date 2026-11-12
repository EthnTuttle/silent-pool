//! Public durable native evidence DTOs. This is a retention envelope, not a
//! new authorization/signature protocol. Inner bytes use the frozen codec.
use crate::native_activation::{
    self as wire, CandidateRequest, ProcessIdentity, Response, VerificationKeys,
};
use bitcoin::hashes::{sha256, Hash};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

pub const SCHEMA: &str = "spc.native-evidence/1";
pub const CONFIG_SCHEMA: &str = "spc.native-evidence-config/1";
pub const MANIFEST_SCHEMA: &str = "spc.native-evidence-manifest/1";
pub const HANDLE_SCHEMA: &str = "spc.native-evidence-handle/1";
pub const LOCATOR_SCHEMA: &str = "spc.native-evidence-locator/1";
pub const CAPABILITY: &str = "native_public_evidence_durable_v1";
pub const MAX_RECORD_BYTES: usize = 2 * (wire::MAX_REQUEST_BYTES + wire::MAX_RESPONSE_BYTES) + 8192;
pub const MAX_MANIFEST_BYTES: usize = 16 * 1024;

pub fn sha256_hex(bytes: &[u8]) -> String {
    sha256::Hash::hash(bytes).to_string()
}
fn hex_bytes(value: &str, max: usize) -> Result<Vec<u8>, &'static str> {
    if value.len() > 2 * max
        || value.len() % 2 != 0
        || !value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err("native_evidence_encoding");
    }
    hex::decode(value).map_err(|_| "native_evidence_encoding")
}
fn absolute(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|c| matches!(c, Component::RootDir | Component::Normal(_)))
}
pub fn session_name(process: &ProcessIdentity) -> String {
    format!("process-{}-{}", process.pid, process.start_time_ticks)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub schema: String,
    pub run_id: String,
    pub directory: PathBuf,
    pub run_manifest: PathBuf,
    pub max_records: u64,
    pub max_bytes: u64,
    pub max_sessions: u32,
}
impl Config {
    pub fn validate(&self, miner: &str) -> Result<(), &'static str> {
        let id = |s: &str| {
            !s.is_empty()
                && s.len() <= 128
                && s.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"._:-".contains(&c))
        };
        let run = self.run_manifest.parent().ok_or("native_evidence_path")?;
        if self.schema != CONFIG_SCHEMA
            || !id(&self.run_id)
            || !id(miner)
            || !absolute(&self.directory)
            || !absolute(&self.run_manifest)
            || self.run_manifest.file_name().and_then(|s| s.to_str()) != Some("run-manifest.json")
            || self.directory != run.join("private/native-evidence").join(miner)
            || !(1..=65536).contains(&self.max_records)
            || !(MAX_RECORD_BYTES as u64..=16 * 1024 * 1024 * 1024).contains(&self.max_bytes)
            || !(1..=256).contains(&self.max_sessions)
        {
            return Err("native_evidence_config");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: String,
    pub kind: String,
    pub capability: String,
    pub capture_id: String,
    pub miner_id: String,
    pub config: Config,
    pub process: ProcessIdentity,
    pub config_path: PathBuf,
}
impl Manifest {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.config.validate(&self.miner_id)?;
        self.process.validate()?;
        if self.schema != MANIFEST_SCHEMA
            || self.kind != "capture_session"
            || self.capability != CAPABILITY
            || hex_bytes(&self.capture_id, 16)?.len() != 16
            || self.capture_id == "0".repeat(32)
            || !absolute(&self.config_path)
            || !self
                .config_path
                .starts_with(self.config.run_manifest.parent().unwrap().join("private"))
        {
            return Err("native_evidence_manifest");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub run_id: String,
    pub miner_id: String,
    pub connection_id: String,
    pub session_epoch: u64,
    pub candidate_epoch: u64,
    pub wallet_instance: [u8; 16],
    pub peer_generation: u64,
    pub connection_config_digest: [u8; 32],
    pub nonce: [u8; 16],
}
impl From<&CandidateRequest> for Binding {
    fn from(r: &CandidateRequest) -> Self {
        Self {
            run_id: r.run_id.clone(),
            miner_id: r.miner_id.clone(),
            connection_id: r.connection_id.clone(),
            session_epoch: r.session_epoch,
            candidate_epoch: r.candidate_epoch,
            wallet_instance: r.wallet_instance,
            peer_generation: r.peer_generation,
            connection_config_digest: r.connection_config_digest,
            nonce: r.nonce,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub schema: String,
    pub kind: String,
    pub version: u16,
    /// Always not_asserted. Only the activation/first-hash event join proves use.
    pub activation: String,
    pub capture_id: String,
    pub receipt_sequence: u64,
    pub manifest_sha256: String,
    pub request_digest: String,
    pub request_sha256: String,
    pub response_sha256: String,
    pub checkpoint_digest: String,
    pub binding: Binding,
    pub verified_at_unix_ms: u64,
    /// Exact transmitted/received UTF8 JSON payload bytes, excluding U32LE length.
    pub request_json_hex: String,
    pub response_json_hex: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DurableHandle {
    pub schema: String,
    pub capability: String,
    pub capture_id: String,
    pub receipt_sequence: u64,
    pub manifest_relative_path: String,
    pub manifest_sha256: String,
    pub record_relative_path: String,
    pub record_sha256: String,
    pub request_digest: String,
}

pub fn decode_manifest(bytes: &[u8]) -> Result<Manifest, &'static str> {
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err("native_evidence_bound");
    }
    let m: Manifest = serde_json::from_slice(bytes).map_err(|_| "native_evidence_manifest")?;
    m.validate()?;
    Ok(m)
}

/// Read-only validation before exposing the real outer pair to the existing
/// verifier/observer. Expected keys come from the controller binding, not record.
pub fn decode_record(
    bytes: &[u8],
    manifest_bytes: &[u8],
    expected: &DurableHandle,
    keys: &VerificationKeys,
) -> Result<(Record, CandidateRequest, Response), &'static str> {
    if bytes.len() > MAX_RECORD_BYTES || sha256_hex(bytes) != expected.record_sha256 {
        return Err("native_evidence_record_hash");
    }
    let manifest = decode_manifest(manifest_bytes)?;
    let record: Record = serde_json::from_slice(bytes).map_err(|_| "native_evidence_record")?;
    let root = session_name(&manifest.process);
    if record.schema != SCHEMA
        || record.kind != "approved_exchange"
        || record.version != 1
        || record.activation != "not_asserted"
        || expected.schema != HANDLE_SCHEMA
        || expected.capability != CAPABILITY
        || record.capture_id != manifest.capture_id
        || expected.capture_id != record.capture_id
        || record.manifest_sha256 != sha256_hex(manifest_bytes)
        || expected.manifest_sha256 != record.manifest_sha256
        || record.receipt_sequence == 0
        || record.receipt_sequence > manifest.config.max_records
        || expected.receipt_sequence != record.receipt_sequence
        || expected.request_digest != record.request_digest
        || expected.manifest_relative_path != format!("{root}/manifest.json")
        || expected.record_relative_path != format!("{root}/{}.json", record.request_digest)
        || hex_bytes(&record.request_digest, 32)?.len() != 32
    {
        return Err("native_evidence_binding");
    }
    let request_bytes = hex_bytes(&record.request_json_hex, wire::MAX_REQUEST_BYTES)?;
    let response_bytes = hex_bytes(&record.response_json_hex, wire::MAX_RESPONSE_BYTES)?;
    if sha256_hex(&request_bytes) != record.request_sha256
        || sha256_hex(&response_bytes) != record.response_sha256
    {
        return Err("native_evidence_payload_hash");
    }
    let request = wire::decode_request(&request_bytes)?;
    let response = wire::decode_response(&response_bytes)?;
    if record.binding != Binding::from(&request)
        || record.binding.run_id != manifest.config.run_id
        || record.binding.miner_id != manifest.miner_id
        || hex::encode(request.signing_digest()?) != record.request_digest
    {
        return Err("native_evidence_request_binding");
    }
    let proof = wire::verify_response(&request, &response, keys, record.verified_at_unix_ms)?;
    if record.checkpoint_digest != hex::encode(proof.checkpoint_digest) {
        return Err("native_evidence_checkpoint");
    }
    Ok((record, request, response))
}
