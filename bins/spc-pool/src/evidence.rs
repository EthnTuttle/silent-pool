//! Machine-readable job/accounting evidence for the independent lab oracle.
//!
//! This module is only linked into the SPc front. It is not linked by or sent
//! to the Ark role. Identity-bearing accounted logs are private fixture data;
//! slice/Ark stream shapes are unchanged.

use crate::sv1::{Job, JobWork, Share};
use serde_json::json;
use std::sync::{Mutex, OnceLock};

struct Trace {
    session: [u8; 16],
    seq: u64,
    previous: [u8; 32],
}
static TRACE: OnceLock<Mutex<Trace>> = OnceLock::new();
static SOURCE_KEY: OnceLock<bitcoin::secp256k1::Keypair> = OnceLock::new();
/// Front-only startup hook; never loaded into the Ark process.
pub fn configure_source_signing_key(path: &std::path::Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    let parent = path.parent().ok_or("source key parent")?;
    if !path.is_absolute()
        || !meta.is_file()
        || meta.permissions().mode() & 0o077 != 0
        || meta.len() > 128
        || std::fs::metadata(parent)
            .map_err(|e| e.to_string())?
            .permissions()
            .mode()
            & 0o077
            != 0
    {
        return Err("private source key permissions/path".into());
    }
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let sk: bitcoin::secp256k1::SecretKey =
        text.trim().parse().map_err(|_| "source key encoding")?;
    SOURCE_KEY
        .set(bitcoin::secp256k1::Keypair::from_secret_key(
            &bitcoin::secp256k1::Secp256k1::new(),
            &sk,
        ))
        .map_err(|_| "source key already configured".into())
}
const MAX_EVENT_BYTES: usize = 4 * 1024 * 1024;

fn canonical(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            map.sort_keys();
            for (_, value) in map.iter_mut() {
                canonical(value);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                canonical(value);
            }
        }
        _ => {}
    }
}

fn hex32(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn emit(kind: &str, mut payload: serde_json::Value) {
    // JSON is a value, not a trusted format string. `kind` is one of the fixed
    // evidence event names below, and an external consumer must anchor by it.
    use bitcoin::hashes::{sha256, Hash, HashEngine};
    canonical(&mut payload);
    let Ok(bytes) = serde_json::to_vec(&payload) else {
        return;
    };
    // Reserve bounded room for source metadata and its pool signature.
    if bytes.len().saturating_add(1024) > MAX_EVENT_BYTES {
        emit(
            "proof_unavailable",
            json!({"event":kind,"reason":"event_bound_exceeded"}),
        );
        return;
    }
    let mut trace = TRACE
        .get_or_init(|| {
            Mutex::new(Trace {
                session: rand::random(),
                seq: 0,
                previous: [0; 32],
            })
        })
        .lock()
        .expect("source trace lock");
    let Some(seq) = trace.seq.checked_add(1) else {
        return;
    };
    let mut hash = sha256::Hash::engine();
    hash.input(b"SPc/SourceEvidenceV2");
    hash.input(&trace.session);
    hash.input(&seq.to_le_bytes());
    hash.input(&trace.previous);
    hash.input(&(kind.len() as u32).to_le_bytes());
    hash.input(kind.as_bytes());
    hash.input(&bytes);
    let digest = sha256::Hash::from_engine(hash).to_byte_array();
    payload["source"] = json!({"schema":"spc.source-evidence/2","session":hex::encode(trace.session),"seq":seq.to_string(),
        "previous":hex::encode(trace.previous),"digest":hex::encode(digest),"complete_prefix_required":true});
    if let Some(key) = SOURCE_KEY.get() {
        let mut seal = spc_scan::source_feed::TraceSeal {
            session: trace.session,
            seq,
            digest,
            signature: String::new(),
        };
        seal.signature = spc_scan::source_feed::sign(seal.signing_digest(), key);
        payload["source"]["pool_seal"] = json!(seal);
    }
    tracing::info!(target: "spc_evidence", event = kind, evidence = %payload);
    trace.seq = seq;
    trace.previous = digest;
}

fn common(work: &JobWork) -> serde_json::Value {
    json!({
        "generation": work.generation.to_string(),
        "nbits": work.nbits,
        "share_target": hex32(&work.share_target.to_be_bytes()),
        "share_target_byte_order": "big-endian unsigned 256-bit",
        "m_issue": work.m_issue,
        "work_unit_version": work.work_unit_version.as_u8(),
        "work_unit_fingerprint": work.work_unit_fingerprint.as_ref().map(hex32),
    })
}

fn merge(work: &JobWork, extra: serde_json::Value) -> serde_json::Value {
    let mut base = common(work);
    if let Some(fields) = base.as_object_mut() {
        for (key, value) in extra.as_object().into_iter().flatten() {
            fields.insert(key.clone(), value.clone());
        }
    }
    base
}

pub fn job_issued(front_id: u8, channel_id: u32, job: &Job, activation: &str) {
    let now = unix_ms();
    emit(
        "job_issued",
        merge(
            &job.work,
            json!({
                "front_id": front_id,
                "channel_id": channel_id,
                "job_id": &job.job_id,
                "issued_at": now,
                "activation": activation,
                "prev_hash": hex32(&job.prev_hash),
                "height":job.height,"version":job.version,"ntime":job.ntime,
                "coinbase_prefix":hex::encode(&job.coinb1),"coinbase_suffix":hex::encode(&job.coinb2),
                "merkle_branch":job.merkle_branches.iter().map(hex32).collect::<Vec<_>>(),
                "clean_jobs":job.clean_jobs,
            }),
        ),
    );
}

pub fn accounted(
    front_id: u8,
    channel_id: u32,
    job: &Job,
    share: &Share,
    slice_id: u64,
    credited_job_fees: u64,
) {
    emit(
        "accounted",
        merge(
            &job.work,
            json!({
                "front_id": front_id,
                "channel_id": channel_id,
                "job_id": &job.job_id,
                "sp_id": hex32(&share.sp_id),
                "d": job.work.m_issue,
                "nonce": share.nonce,
                "ntime": share.ntime,
                "version": share.version,
                "share_index": share.share_index,
                "slice_id":slice_id.to_string(),"credited_job_fees":credited_job_fees.to_string(),
                "job_fees":share.job_fees,
                "coinbase":hex::encode(&share.coinbase),"merkle_root":hex32(&share.merkle_root),
                // Raw SHA256d digest bytes, not reversed RPC display hex.
                // The oracle compares this integer against the explicitly
                // big-endian share_target above.
                "share_hash": hex32(&share.share_hash),
                "share_hash_byte_order": "raw SHA256d digest bytes, not RPC display order",
                // Full pool-prefix || miner extranonce, in coinbase order.
                "extranonce": hex::encode(&share.extranonce),
                "extranonce_byte_order": "coinbase order: pool prefix then miner extranonce",
            }),
        ),
    );
}

/// Actual front generation record, not a fixture-derived payout assertion.
/// This private SPc trace contains attribution and must be collected only by
/// the operator/owned lab broker; it is never an Ark message or public DTO.
pub(crate) struct FrozenEvidence<'a> {
    pub finding_slice_id: u64,
    pub front_id: u8,
    pub job: &'a Job,
    pub superseded_job_id: Option<&'a str>,
    pub economic: Option<&'a spc_accounting::economic::FrozenJob>,
    pub distribution: &'a spc_accounting::Distribution,
    pub window: &'a spc_accounting::Window,
    pub pending: &'a std::collections::BTreeMap<[u8; 32], u64>,
    pub ark: &'a std::collections::BTreeMap<[u8; 32], spc_accounting::ArkCapability>,
    pub window_target: u128,
    pub window_rule: u8,
    pub fee_bps: u16,
    pub subsidy: u64,
    pub fees: u64,
    pub extranonce_len: usize,
    pub entries_digest: [u8; 32],
    pub witness: Option<(&'a [u8; 32], &'a [u8; 32], &'a [u8; 32])>,
    pub transactions: &'a [Vec<u8>],
}
pub(crate) fn frozen_distribution(evidence: FrozenEvidence<'_>) {
    let FrozenEvidence {
        finding_slice_id,
        front_id,
        job,
        superseded_job_id,
        economic,
        distribution,
        window,
        pending,
        ark,
        window_target,
        window_rule,
        fee_bps,
        subsidy,
        fees,
        extranonce_len,
        entries_digest,
        witness,
        transactions,
    } = evidence;
    let slices:Vec<_>=window.slices.iter().map(|s|json!({"slice_id":s.slice_id.to_string(),"height":s.height,"front_id":s.front_id,
        "difficulty":s.difficulty.to_string(),"diff_fee_sum":s.diff_fee_sum.to_string(),"root":hex32(&s.root),
        "credits":s.per_id.iter().map(|(id,(d,df))|json!({"sp_id":hex32(id),"diff_sum":d.to_string(),"diff_fee_sum":df.to_string()})).collect::<Vec<_>>()})).collect();
    emit(
        "distribution_frozen",
        merge(
            &job.work,
            json!({"front_id":front_id,"job_id":job.job_id,"superseded_job_id":superseded_job_id,"economic":economic,"height":job.height,
        "finding_slice_id":finding_slice_id.to_string(),"finding_slice_excluded":true,
        "fingerprint":hex32(&distribution.fingerprint),"entries_digest":hex32(&entries_digest),"a_send_x":hex32(&distribution.a_send_x),
        "entries":distribution.entries.iter().map(|e|json!({"format_version":2,"sp_id":hex32(&e.sp_id),"rail":e.rail as u8,
            "p_x":e.p_x.iter().map(hex32).collect::<Vec<_>>(),"sats":e.sats.to_string(),"owed_sats":e.owed_sats.to_string(),
             "destination_digest":e.dest_id.as_ref().map(|d|{use bitcoin::hashes::{sha256,Hash};hex32(&sha256::Hash::hash(d).to_byte_array())}),
             "delivery_state":match ark.get(&e.sp_id) {Some(cap) if cap.registration_committed && cap.dest_known=>"registered_derived",Some(cap) if cap.registration_committed || cap.registered=>"registered_underived",_=>"unregistered"}})).collect::<Vec<_>>(),
        "window_target":window_target.to_string(),"window_rule":window_rule,"high_water":std::collections::BTreeMap::from([(front_id,window.slices.first().map_or(0,|s|s.slice_id).to_string())]),
        "window_underfilled":window.underfilled(window_target),"slices":slices,"subsidy":subsidy.to_string(),"fees":fees.to_string(),"fee_bps":fee_bps,
        "prior_pending":pending.iter().map(|(id,sats)|json!({"sp_id":hex32(id),"sats":sats.to_string()})).collect::<Vec<_>>(),
        "extranonce_len":extranonce_len,"coinbase_prefix":hex::encode(&job.coinb1),"coinbase_suffix":hex::encode(&job.coinb2),
        "merkle_branch":job.merkle_branches.iter().map(hex32).collect::<Vec<_>>(),"version":job.version,"ntime":job.ntime,"prev_hash":hex32(&job.prev_hash),
        "transactions":transactions.iter().map(hex::encode).collect::<Vec<_>>(),"fee_prevout_transactions_required":true,
        "witness_plan":witness.map(|(root,reserved,commitment)|json!({"wtxid_root":hex32(root),"reserved":hex32(reserved),"commitment":hex32(commitment)})),
        "attribution_trust":"pool_authenticated_ingress_assignment; compare every accounted share and independently verify raw headers/work/arithmetic",
        "pending_history_required":true}),
        ),
    );
}

pub fn multiplier_step(kind: &str, front_id: u8, work: &JobWork) {
    assert!(matches!(kind, "multiplier_stepdown" | "multiplier_stepup"));
    emit(kind, merge(work, json!({"front_id": front_id})));
}

/// Actual Redis publication result and exact bytes for the private collector.
pub fn source_publication(stream: &str, payload: &[u8], stream_id: Option<&str>) {
    let mut value = json!({"stream":stream,"stream_id":stream_id,"published_at_unix_ms":unix_ms(),
        "raw_d_hex":hex::encode(payload),"publication_complete":stream_id.is_some()});
    match stream {
        "spc:blocks" => {
            if let Ok(event) = spc_store::wire::BlockEvent::decode(payload) {
                value["front_id"] = json!(event.front_id);
                value["block_hash"] = json!(hex::encode(event.block_hash));
                value["coinbase_fingerprint"] = json!(hex::encode(event.fingerprint));
                value["sql_snapshot_key"] = json!(event.snapshot_id().ok().map(hex::encode));
            }
        }
        "spc:slices" => {
            if let Ok(event) = spc_store::wire::SliceEvent::decode(payload) {
                value["slice_id"] = json!(event.slice_id.to_string());
            }
        }
        _ => return,
    }
    emit("source_publication", value);
}

pub fn share_outcome_unknown(
    front_id: u8,
    channel_id: u32,
    job_id: &str,
    sequence: u32,
    reason: &str,
) {
    emit(
        "share_outcome_unknown",
        json!({
            "front_id": front_id,
            "channel_id": channel_id,
            "job_id": job_id,
            "sequence_number": sequence,
            "reason": reason,
        }),
    );
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}
