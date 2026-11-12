mod common;

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;

use spc_lab::evidence::{CanonicalResult, EvidenceStore};
use spc_lab::ids::{FrontId, IdentityId, SourceId};
use spc_lab::spc_evidence::{SpcEvidenceAdapter, SpcEvidenceError, SpcEvidenceMappings};
use spc_lab::RunId;

fn mappings() -> SpcEvidenceMappings {
    let mut front_ids = BTreeMap::new();
    front_ids.insert(FrontId::new("front-a1").unwrap(), 1);
    let mut identity_sp_ids = BTreeMap::new();
    identity_sp_ids.insert(IdentityId::new("id-alice").unwrap(), "aa".repeat(32));
    SpcEvidenceMappings {
        front_ids,
        identity_sp_ids,
    }
}

fn line(event: &str, evidence: serde_json::Value) -> String {
    serde_json::json!({
        "timestamp": "2026-11-05T00:00:00Z",
        "level": "INFO",
        "fields": {
            "message": "evidence",
            "event": event,
            "evidence": evidence.to_string(),
        },
        "target": "spc_evidence",
    })
    .to_string()
}

fn job_evidence(front_id: u64) -> serde_json::Value {
    serde_json::json!({
        "generation": "7",
        "nbits": 0x207f_ffffu32,
        "share_target": "7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        "m_issue": 2,
        "work_unit_version": 2,
        "work_unit_fingerprint": "fingerprint",
        "front_id": front_id,
        "channel_id": 5,
        "job_id": "job-1",
        "issued_at": 1234,
        "activation": "active",
        "prev_hash": "00".repeat(32),
    })
}

#[test]
fn job_issued_is_mapped_truthfully_and_raw_stays_private() {
    let dir = common::TestDir::new("spc-evidence");
    let store = EvidenceStore::new(RunId::new("run-spc").unwrap(), 100);
    let private = dir.join("private-spc-evidence.jsonl");
    let mut adapter =
        SpcEvidenceAdapter::new(SourceId::new("front-log").unwrap(), &private, mappings()).unwrap();
    let event_id = adapter
        .ingest_line(&store, &line("job_issued", job_evidence(1)))
        .unwrap();
    assert!(event_id > 0);
    let events = store.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].payload.kind(), "job_issued");
    match &events[0].payload {
        spc_lab::evidence::EvidencePayload::JobIssued(job) => {
            assert_eq!(job.front_id, FrontId::new("front-a1").unwrap());
            assert_eq!(job.generation, 7);
            assert_eq!(job.nbits, 0x207f_ffff);
            assert_eq!(job.m_issue, 2);
            assert_eq!(job.share_target.len(), 64);
        }
        other => panic!("expected mapped job_issued, got {other:?}"),
    }
    let metadata = std::fs::metadata(&private).unwrap();
    assert_eq!(metadata.permissions().mode() & 0o077, 0);
    let raw = std::fs::read_to_string(&private).unwrap();
    assert!(raw.contains("spc_evidence"));
    assert!(raw.contains("job-1"));
    assert_eq!(adapter.cursor(), 1);
}

#[test]
fn accounted_requires_an_explicit_identity_mapping_and_never_becomes_accounting_evidence_from_an_ack(
) {
    let dir = common::TestDir::new("spc-evidence-accounted");
    let store = EvidenceStore::new(RunId::new("run-spc").unwrap(), 100);
    let mut adapter = SpcEvidenceAdapter::new(
        SourceId::new("front-log").unwrap(),
        dir.join("private.jsonl"),
        mappings(),
    )
    .unwrap();
    let mut accounted = job_evidence(1);
    accounted["sp_id"] = serde_json::json!("aa".repeat(32));
    accounted["d"] = serde_json::json!(2);
    accounted["share_hash"] = serde_json::json!("11".repeat(32));
    accounted["share_hash_byte_order"] =
        serde_json::json!("raw SHA256d digest bytes, not RPC display order");
    accounted["extranonce"] = serde_json::json!("01020304aabbccdd");
    accounted["extranonce_byte_order"] =
        serde_json::json!("coinbase order: pool prefix then miner extranonce");
    accounted["share_target_byte_order"] = serde_json::json!("big-endian unsigned 256-bit target");
    adapter
        .ingest_line(&store, &line("accounted", accounted))
        .unwrap();
    let events = store.events();
    assert_eq!(events[0].payload.kind(), "spc_evidence");
    match &events[0].payload {
        spc_lab::evidence::EvidencePayload::SpcEvidence(record) => {
            assert_eq!(record.event, "accounted");
            assert_eq!(
                record.mapped_identity,
                Some(IdentityId::new("id-alice").unwrap())
            );
            assert!(record.raw.get("sp_id").is_some());
            assert_eq!(
                record.share_hash.as_deref(),
                Some("1111111111111111111111111111111111111111111111111111111111111111")
            );
            assert_eq!(record.extranonce.as_deref(), Some("01020304aabbccdd"));
            assert!(record
                .share_hash_byte_order
                .as_deref()
                .unwrap()
                .contains("not RPC display"));
            assert!(record
                .extranonce_byte_order
                .as_deref()
                .unwrap()
                .contains("coinbase order"));
            assert!(record.note.contains("miner ACK is not used"));
        }
        other => panic!("expected spc_evidence, got {other:?}"),
    }
    assert!(!store
        .events()
        .iter()
        .any(|event| event.payload.kind() == "accounted"));
    assert_eq!(
        store
            .snapshot(None, Vec::new())
            .evidence
            .counts
            .get("accounted"),
        None
    );
    // Source cursor is local and monotonic.
    adapter
        .ingest_line(&store, &line("accounted", job_evidence(1)))
        .unwrap();
    assert_eq!(adapter.cursor(), 2);
}

#[test]
fn unmapped_front_and_unknown_events_are_retained_as_private_raw_records() {
    let dir = common::TestDir::new("spc-evidence-unmapped");
    let store = EvidenceStore::new(RunId::new("run-spc").unwrap(), 100);
    let mut adapter = SpcEvidenceAdapter::new(
        SourceId::new("front-log").unwrap(),
        dir.join("private.jsonl"),
        mappings(),
    )
    .unwrap();
    adapter
        .ingest_line(&store, &line("job_issued", job_evidence(99)))
        .unwrap();
    adapter
        .ingest_line(
            &store,
            &line(
                "share_outcome_unknown",
                serde_json::json!({
                    "front_id": 1,
                    "channel_id": 5,
                    "job_id": "job-1",
                    "sequence_number": 9,
                    "reason": "lost",
                }),
            ),
        )
        .unwrap();
    assert!(store
        .events()
        .iter()
        .all(|event| event.payload.kind() == "spc_evidence"));
    assert!(store.events().iter().any(|event| matches!(
        &event.payload,
        spc_lab::evidence::EvidencePayload::SpcEvidence(record)
            if record.event == "job_issued" && record.front_id.is_none()
    )));
    assert!(store.events().iter().any(|event| matches!(
        &event.payload,
        spc_lab::evidence::EvidencePayload::SpcEvidence(record)
            if record.event == "share_outcome_unknown"
    )));
    assert!(!store.events().iter().any(|event| matches!(
        event.payload,
        spc_lab::evidence::EvidencePayload::Accounted(_)
    )));
    assert!(matches!(
        adapter.ingest_line(&store, "not json"),
        Err(SpcEvidenceError::Malformed(_))
    ));
    assert!(matches!(
        adapter.ingest_line(&store, "{\"target\":\"other\",\"fields\":{}}"),
        Err(SpcEvidenceError::NotEvidence)
    ));
    // A canonical observation is still independent of these records.
    assert_eq!(
        store
            .current_status(
                &spc_lab::ids::NodeId::new("node-a").unwrap(),
                &"00".repeat(32)
            )
            .result,
        CanonicalResult::Unknown
    );
}
