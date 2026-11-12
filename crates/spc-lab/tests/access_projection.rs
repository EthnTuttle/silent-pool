//! EXP-ACCESS projection tests (live-lab plan section 5).
//!
//! Fixture-only: no captaind, no pool Ark role, no AP-3b, no T1b, no live
//! funded run. These tests prove the positive read surface of the
//! expanded-access foundation: the filtered user snapshot and SSE stream
//! (initial replay, live events, Last-Event-ID, resync) never carry another
//! user's ids, revocation closes an open stream at the next event or
//! heartbeat boundary, and the miner-poll worker projection serves real rows
//! or explicit nulls with reasons - never zeros.

mod common;

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use spc_lab::config::LabConfig;
use spc_lab::engine::{EngineOptions, RunEngine};
use spc_lab::evidence::{
    Accounted, CommandObservation, Delivered, EvidenceActor, EvidencePayload, Gap, GapScope,
    ShareHeader, Submitted,
};
use spc_lab::http::{HttpServer, WebAssets};
use spc_lab::ids::{CommandId, FrontId, IdentityId, JobId, MinerId, RunId, ShareId, SourceId};
use spc_lab::scenario::Scenario;
use spc_lab::wide::WideU128;

// ---------------------------------------------------------------------------
// Local helpers (this packet does not edit the shared harness)
// ---------------------------------------------------------------------------

fn expanded_scenario(run_id: &str) -> Scenario {
    let mut scenario = common::fixture_scenario("scenario_v1_one_pool_two_miners.json");
    scenario.run.id = RunId::new(run_id).unwrap();
    scenario.expanded_access = true;
    scenario
}

fn prepare_expanded(
    dir: &common::TestDir,
    scenario: Scenario,
    addresses: BTreeMap<String, String>,
    evidence_capacity: usize,
) -> Arc<RunEngine> {
    prepare_expanded_with_config(
        dir,
        scenario,
        common::fixture_config(),
        addresses,
        evidence_capacity,
    )
}

fn prepare_expanded_with_config(
    dir: &common::TestDir,
    scenario: Scenario,
    config: LabConfig,
    addresses: BTreeMap<String, String>,
    evidence_capacity: usize,
) -> Arc<RunEngine> {
    RunEngine::prepare(
        scenario,
        EngineOptions {
            run_dir: dir.path().to_path_buf(),
            config,
            base_revision: "test-base".to_string(),
            dirty_tree_hash: "clean".to_string(),
            owner_instance: format!("test-{}", std::process::id()),
            evidence_capacity,
            namespaces: BTreeMap::new(),
            nsenter: None,
            user_namespace: None,
            user_namespace_owner: None,
            namespace_owners: BTreeMap::new(),
            namespace_broker: None,
            lifetime_helper: None,
            addresses,
            // EXP-OP wiring: the access suite never plans the operator
            // page listener (host-only opt-in flag).
            operator_ui: false,
            link_backend: None,
        },
    )
    .unwrap()
}

struct ClaimInfo {
    principal: String,
    path: PathBuf,
}

fn read_claims(dir: &common::TestDir) -> Vec<ClaimInfo> {
    let access_dir = dir.path().join("private").join("access");
    let mut claims = Vec::new();
    for entry in std::fs::read_dir(&access_dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        claims.push(ClaimInfo {
            principal: value["principal"].as_str().unwrap().to_string(),
            path,
        });
    }
    claims
}

fn token_of(dir: &common::TestDir, principal: &str) -> String {
    let claim = read_claims(dir)
        .into_iter()
        .find(|claim| claim.principal == principal)
        .unwrap_or_else(|| panic!("no claim for principal {principal}"));
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&claim.path).unwrap()).unwrap();
    value["token"].as_str().unwrap().to_string()
}

/// Revoke a principal by deleting its claim file, the way an operator does.
fn revoke(dir: &common::TestDir, principal: &str) {
    let claim = read_claims(dir)
        .into_iter()
        .find(|claim| claim.principal == principal)
        .unwrap_or_else(|| panic!("no claim for principal {principal}"));
    std::fs::remove_file(&claim.path).unwrap();
}

struct Response {
    status: u16,
    body: Vec<u8>,
}

impl Response {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
    }

    fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|error| panic!("response is not JSON ({error}): {}", self.text()))
    }
}

fn request_raw(
    addr: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, String)],
    body: Option<&str>,
) -> Response {
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let body_text = body.unwrap_or("");
    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        body_text.len()
    );
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).unwrap();
    stream.write_all(body_text.as_bytes()).unwrap();
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).unwrap();
    let split = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap_or(0);
    let head = String::from_utf8_lossy(&bytes[..split]).to_string();
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    Response {
        status,
        body: bytes[split + 4..].to_vec(),
    }
}

fn bearer(token: &str) -> Vec<(&'static str, String)> {
    vec![("Authorization", format!("Bearer {token}"))]
}

// ---------------------------------------------------------------------------
// Synthetic evidence: harness-appended events for both users
// ---------------------------------------------------------------------------

fn append_event(engine: &RunEngine, payload: EvidencePayload) -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    engine.evidence().append(
        SourceId::trusted("test-projection"),
        format!("projection-{count}"),
        EvidenceActor::harness(),
        None,
        payload,
    )
}

fn submitted_event(identity: &str, miner: &str, share: &str) -> EvidencePayload {
    EvidencePayload::Submitted(Submitted {
        miner_id: MinerId::trusted(miner),
        identity_id: IdentityId::trusted(identity),
        share_id: ShareId::trusted(share),
        job_id: JobId::trusted("job-fixture-1"),
        generation: 1,
        header: ShareHeader {
            version: 1,
            prev_hash: "00".repeat(32),
            merkle_root: "11".repeat(32),
            ntime: 1,
            nbits: 0x207fffff,
            nonce: 1,
        },
        hash: "ab".repeat(32),
    })
}

fn accounted_event(identity: &str, share: &str) -> EvidencePayload {
    EvidencePayload::Accounted(Accounted {
        front_id: FrontId::trusted("front-a1"),
        identity_id: IdentityId::trusted(identity),
        share_id: ShareId::trusted(share),
        job_id: JobId::trusted("job-fixture-1"),
        generation: 1,
        nbits: 0x207fffff,
        share_target: "ff".repeat(32),
        d: WideU128::from(1u64),
        slice_id: "slice-1".to_string(),
        diff_sum: WideU128::from(2u64),
    })
}

fn delivered_event(identity: &str, block_hash: &str) -> EvidencePayload {
    EvidencePayload::Delivered(Delivered {
        identity_id: IdentityId::trusted(identity),
        block_hash: block_hash.to_string(),
        amount_sats: WideU128::from(1_500_000_000u64),
        detail: "fixture payout".to_string(),
    })
}

fn miner_event(miner: &str, name: &str) -> EvidencePayload {
    EvidencePayload::MinerEvent {
        miner_id: MinerId::trusted(miner),
        incarnation: "inc-fixture".to_string(),
        driver_event_id: 1,
        name: name.to_string(),
        // The raw driver payload must never reach a user response.
        payload: serde_json::json!({"internal": "driver-detail-not-for-users"}),
        dropped_before: 0,
    }
}

fn command_event(command_id: &str) -> EvidencePayload {
    EvidencePayload::Command(CommandObservation {
        command_id: CommandId::trusted(command_id),
        state: "succeeded".to_string(),
        detail: "projection test".to_string(),
        journal_seq: 1,
    })
}

// ---------------------------------------------------------------------------
// A raw SSE client with header support (the shared harness reader has none)
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct SseFrame {
    id: Option<u64>,
    event: String,
    data: String,
}

enum SseItem {
    Frame(SseFrame),
    Comment,
}

/// Reads the filtered (and unfiltered) SSE streams at the byte level: full
/// multiline frames (`id:`/`event:`/`data:` lines in any order, comment
/// lines skipped), frames split across TCP segments, and EOF.
struct SseClient {
    reader: BufReader<TcpStream>,
}

impl SseClient {
    fn open(
        addr: SocketAddr,
        run_id: &str,
        query: &str,
        headers: &[(&str, String)],
        read_timeout: Duration,
    ) -> std::io::Result<Self> {
        let path = if query.is_empty() {
            format!("/api/lab/v1/runs/{run_id}/events")
        } else {
            format!("/api/lab/v1/runs/{run_id}/events?{query}")
        };
        let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))?;
        stream.set_read_timeout(Some(read_timeout))?;
        let mut head = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: keep-alive\r\n");
        for (name, value) in headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str("\r\n");
        stream.write_all(head.as_bytes())?;
        stream.flush()?;
        let mut reader = BufReader::new(stream);
        let mut status = String::new();
        reader.read_line(&mut status)?;
        assert!(status.contains("200"), "SSE status was not 200: {status}");
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line)? == 0 {
                break;
            }
            if line == "\r\n" || line == "\n" {
                break;
            }
        }
        Ok(Self { reader })
    }

    fn set_read_timeout(&mut self, timeout: Duration) {
        self.reader
            .get_ref()
            .set_read_timeout(Some(timeout))
            .unwrap();
    }

    fn next_item(&mut self) -> std::io::Result<Option<SseItem>> {
        let mut id = None;
        let mut event = None;
        let mut data = None;
        let mut saw_comment = false;
        loop {
            let mut line = String::new();
            if self.reader.read_line(&mut line)? == 0 {
                return Ok(None);
            }
            let line = line.trim_end();
            if line.is_empty() {
                if let Some(data) = data {
                    return Ok(Some(SseItem::Frame(SseFrame {
                        id,
                        event: event.unwrap_or_else(|| "message".to_string()),
                        data,
                    })));
                }
                if saw_comment {
                    return Ok(Some(SseItem::Comment));
                }
                continue;
            }
            if let Some(rest) = line.strip_prefix("id: ") {
                id = rest.parse().ok();
            } else if let Some(rest) = line.strip_prefix("event: ") {
                event = Some(rest.to_string());
            } else if let Some(rest) = line.strip_prefix("data: ") {
                data = Some(rest.to_string());
            } else if line.starts_with(':') {
                saw_comment = true;
            }
        }
    }

    /// Read frames until the stream goes idle for `idle`. The stream is
    /// expected to stay open; an unexpected close is a failure.
    fn drain_until_idle(&mut self, idle: Duration) -> Vec<SseFrame> {
        let mut frames = Vec::new();
        self.set_read_timeout(idle);
        loop {
            match self.next_item() {
                Ok(Some(SseItem::Frame(frame))) => frames.push(frame),
                Ok(Some(SseItem::Comment)) => {}
                Ok(None) => panic!("the SSE stream closed while it was expected to stay open"),
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        || error.kind() == std::io::ErrorKind::TimedOut =>
                {
                    break;
                }
                Err(error) => panic!("SSE read failed: {error}"),
            }
        }
        frames
    }

    /// Read until EOF or `wait`. Returns whether the stream closed and every
    /// frame delivered along the way.
    fn read_until_eof(&mut self, wait: Duration) -> (bool, Vec<SseFrame>) {
        let mut frames = Vec::new();
        self.set_read_timeout(wait);
        let deadline = Instant::now() + wait;
        loop {
            if Instant::now() >= deadline {
                return (false, frames);
            }
            match self.next_item() {
                Ok(Some(SseItem::Frame(frame))) => frames.push(frame),
                Ok(Some(SseItem::Comment)) => {}
                Ok(None) => return (true, frames),
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        || error.kind() == std::io::ErrorKind::TimedOut =>
                {
                    return (false, frames);
                }
                Err(error) => panic!("SSE read failed: {error}"),
            }
        }
    }
}

fn frame_json(frame: &SseFrame) -> serde_json::Value {
    serde_json::from_str(&frame.data)
        .unwrap_or_else(|error| panic!("frame data is not JSON ({error}): {}", frame.data))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn user_snapshot_is_a_filtered_projection_with_honest_watermark_and_gaps() {
    let dir = common::TestDir::new("access-proj-snapshot");
    let engine = prepare_expanded(
        &dir,
        expanded_scenario("run-access-proj-snapshot"),
        BTreeMap::new(),
        100_000,
    );
    let server = HttpServer::start(engine.clone(), "127.0.0.1:0", WebAssets::fallback()).unwrap();
    let addr = server.addr();
    let run_id = engine.run_id().to_string();
    let alice = token_of(&dir, "id-alice");
    let operator = token_of(&dir, "operator");
    let snapshot_path = format!("/api/lab/v1/runs/{run_id}/snapshot");

    // Seed both users' facts and one durable gap.
    append_event(
        &engine,
        submitted_event("id-alice", "miner-1", "share-alice-1"),
    );
    append_event(&engine, submitted_event("id-bob", "miner-2", "share-bob-1"));
    append_event(&engine, accounted_event("id-alice", "share-alice-1"));
    append_event(&engine, delivered_event("id-alice", "block-hash-alice"));
    let watermark = engine.evidence().last_event_id();
    engine.evidence().record_gap(Gap {
        scope: GapScope::Store,
        subscriber: 0,
        from_event_id: 1,
        to_event_id: 2,
        reason: "projection gap honesty".to_string(),
    });

    // Alice's snapshot: her own worker row, the global watermark, explicit
    // gaps and not_available wallet facts.
    let snapshot = request_raw(addr, "GET", &snapshot_path, &bearer(&alice), None);
    assert_eq!(snapshot.status, 200, "{}", snapshot.text());
    let body = snapshot.json();
    assert_eq!(body["schema"], "spc-user-snapshot/v1");
    assert_eq!(body["filtered"], true);
    assert_eq!(body["principal"], "id-alice");
    assert_eq!(body["user_id"], "id-alice");
    assert_eq!(body["miner"], "miner-1");
    let workers = body["workers"].as_array().unwrap();
    assert_eq!(workers.len(), 1, "{}", snapshot.text());
    assert_eq!(workers[0]["id"], "w-alice-1");
    assert_eq!(workers[0]["miner_id"], "miner-1");
    // Watermark honesty: the cursor and evidence watermark count every
    // event, including bob's, so a filtered-empty view can never masquerade
    // as up to date.
    assert_eq!(body["cursor"].as_u64().unwrap(), watermark);
    assert_eq!(
        body["evidence"]["last_event_id"].as_u64().unwrap(),
        watermark
    );
    // Per-kind counts aggregate every user's events and are not included.
    assert!(body["evidence"].get("counts").is_none());
    assert!(body.get("watermarks").is_none());
    // A gap stays an explicit gap fact.
    assert_eq!(body["evidence"]["gap_count"].as_u64().unwrap(), 1);
    assert!(
        body["gaps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|gap| gap["reason"] == "projection gap honesty"),
        "{}",
        snapshot.text()
    );
    // Wallet facts are closed not_available facts, never zeros.
    assert_eq!(body["wallet"]["state"], "not_available");
    assert!(body["wallet"]["reason"].as_str().unwrap().contains("AP-3b"));
    assert_eq!(body["payment_wallet"]["state"], "not_available");
    assert!(body["payment_wallet"]["reason"]
        .as_str()
        .unwrap()
        .contains("T1b"));
    // Zero cross-user ids anywhere in the payload.
    for forbidden in [
        "id-bob",
        "miner-2",
        "w-bob-1",
        "share-bob-1",
        "block-hash-bob",
    ] {
        assert!(
            !snapshot.text().contains(forbidden),
            "{forbidden} leaked into the user snapshot"
        );
    }

    // The operator sees the whole-run snapshot, unfiltered.
    let operator_snapshot = request_raw(addr, "GET", &snapshot_path, &bearer(&operator), None);
    assert_eq!(operator_snapshot.status, 200);
    assert!(operator_snapshot.text().contains("w-bob-1"));
    assert!(operator_snapshot.text().contains("id-bob"));
    assert!(operator_snapshot.json().get("filtered").is_none());

    // After revocation there is no view at all.
    revoke(&dir, "id-alice");
    let revoked = request_raw(addr, "GET", &snapshot_path, &bearer(&alice), None);
    assert_eq!(revoked.status, 401);
    server.shutdown();
}

#[test]
fn user_sse_filters_replay_and_live_events_and_honors_last_event_id() {
    let dir = common::TestDir::new("access-proj-sse");
    let engine = prepare_expanded(
        &dir,
        expanded_scenario("run-access-proj-sse"),
        BTreeMap::new(),
        100_000,
    );
    let server = HttpServer::start(engine.clone(), "127.0.0.1:0", WebAssets::fallback()).unwrap();
    let addr = server.addr();
    let run_id = engine.run_id().to_string();
    let alice = token_of(&dir, "id-alice");
    let operator = token_of(&dir, "operator");

    // No token, no stream.
    let tokenless = request_raw(
        addr,
        "GET",
        &format!("/api/lab/v1/runs/{run_id}/events?after=0"),
        &[],
        None,
    );
    assert_eq!(tokenless.status, 401);
    // A malformed cursor is a client bug even with a valid token.
    let bad_cursor = request_raw(
        addr,
        "GET",
        &format!("/api/lab/v1/runs/{run_id}/events?after=abc"),
        &bearer(&alice),
        None,
    );
    assert_eq!(bad_cursor.status, 400);

    // Seed: alice-visible kinds, bob's events, and a kind with no user
    // visibility at all.
    append_event(
        &engine,
        submitted_event("id-alice", "miner-1", "share-alice-1"),
    );
    append_event(&engine, submitted_event("id-bob", "miner-2", "share-bob-1"));
    append_event(&engine, accounted_event("id-alice", "share-alice-1"));
    append_event(&engine, delivered_event("id-alice", "block-hash-alice"));
    append_event(&engine, miner_event("miner-1", "connection_state"));
    append_event(&engine, miner_event("miner-2", "connection_state"));
    append_event(&engine, command_event("cmd-proj-invisible"));
    append_event(
        &engine,
        EvidencePayload::EventLost(Gap {
            scope: GapScope::Store,
            subscriber: 0,
            from_event_id: 1,
            to_event_id: 2,
            reason: "projection event lost".to_string(),
        }),
    );
    let seeded = engine.evidence().last_event_id();

    // Alice's replay: complete multiline SSE frames (id/event/data lines),
    // and only her own facts.
    let mut alice_stream = SseClient::open(
        addr,
        &run_id,
        "after=0",
        &bearer(&alice),
        Duration::from_secs(5),
    )
    .unwrap();
    let alice_frames = alice_stream.drain_until_idle(Duration::from_millis(750));
    assert!(!alice_frames.is_empty());
    let mut kinds = Vec::new();
    for frame in &alice_frames {
        assert!(frame.id.unwrap_or(0) > 0, "frame without an id line");
        assert_eq!(frame.event, "message");
        let value = frame_json(frame);
        assert_eq!(value["schema"], "spc-user-event/v1");
        let kind = value["kind"].as_str().unwrap().to_string();
        kinds.push(kind.clone());
        match kind.as_str() {
            "submitted" => {
                assert_eq!(value["data"]["share_id"], "share-alice-1");
                assert_eq!(value["data"]["miner_id"], "miner-1");
            }
            "accounted" => assert_eq!(value["data"]["share_id"], "share-alice-1"),
            "delivered" => {
                assert_eq!(value["data"]["block_hash"], "block-hash-alice");
                assert_eq!(value["data"]["amount_sats"], "1500000000");
            }
            "miner_event" => {
                // The raw miner payload is stripped to a typed projection.
                assert_eq!(value["data"]["miner_id"], "miner-1");
                assert_eq!(value["data"].as_object().unwrap().len(), 2);
                assert!(value["data"].get("payload").is_none());
                assert!(!frame.data.contains("driver-detail-not-for-users"));
            }
            "event_lost" => assert_eq!(value["data"]["reason"], "projection event lost"),
            other => panic!("a user stream delivered kind {other}"),
        }
        assert!(
            !frame.data.contains("share-bob-1"),
            "cross-user id: {}",
            frame.data
        );
        assert!(
            !frame.data.contains("id-bob"),
            "cross-user id: {}",
            frame.data
        );
    }
    for expected in [
        "submitted",
        "accounted",
        "delivered",
        "miner_event",
        "event_lost",
    ] {
        assert!(
            kinds.contains(&expected.to_string()),
            "missing kind {expected} in {kinds:?}"
        );
    }
    assert!(!kinds.contains(&"command".to_string()));

    // The operator stream is the unfiltered whole-run stream.
    let mut operator_stream = SseClient::open(
        addr,
        &run_id,
        "after=0",
        &bearer(&operator),
        Duration::from_secs(5),
    )
    .unwrap();
    let operator_frames = operator_stream.drain_until_idle(Duration::from_millis(750));
    let operator_text = operator_frames
        .iter()
        .map(|frame| frame.data.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(operator_text.contains("share-bob-1"));
    assert!(operator_text.contains("\"kind\":\"command\""));
    drop(operator_stream);

    // Live delivery: only alice's new facts arrive, never bob's.
    append_event(
        &engine,
        submitted_event("id-bob", "miner-2", "share-bob-live"),
    );
    append_event(&engine, miner_event("miner-1", "worker_state"));
    let live_frames = alice_stream.drain_until_idle(Duration::from_millis(750));
    assert_eq!(
        live_frames.len(),
        1,
        "unexpected live frames: {:?}",
        live_frames
    );
    let live = frame_json(&live_frames[0]);
    assert_eq!(live["kind"], "miner_event");
    assert_eq!(live["data"]["miner_id"], "miner-1");
    assert!(live_frames[0].id.unwrap() > seeded);
    drop(alice_stream);

    // Last-Event-ID wins over the URL cursor and resumes without duplicates.
    let resume_from = engine.evidence().last_event_id();
    let mut resume_headers = bearer(&alice);
    resume_headers.push(("Last-Event-ID", resume_from.to_string()));
    let mut resumed = SseClient::open(
        addr,
        &run_id,
        // A future cursor in the URL would force a resync if it were used.
        "after=999999",
        &resume_headers,
        Duration::from_secs(5),
    )
    .unwrap();
    append_event(&engine, miner_event("miner-1", "worker_budget"));
    let resumed_frames = resumed.drain_until_idle(Duration::from_millis(750));
    assert_eq!(resumed_frames.len(), 1, "{resumed_frames:?}");
    assert!(resumed_frames[0].id.unwrap() > resume_from);
    assert_eq!(frame_json(&resumed_frames[0])["kind"], "miner_event");
    drop(resumed);

    // A malformed Last-Event-ID is a client bug.
    let mut bad_headers = bearer(&alice);
    bad_headers.push(("Last-Event-ID", "not-a-cursor".to_string()));
    let bad = request_raw(
        addr,
        "GET",
        &format!("/api/lab/v1/runs/{run_id}/events?after=0"),
        &bad_headers,
        None,
    );
    assert_eq!(bad.status, 400, "{}", bad.text());

    // After revocation there is no new stream.
    revoke(&dir, "id-alice");
    let revoked = request_raw(
        addr,
        "GET",
        &format!("/api/lab/v1/runs/{run_id}/events?after=0"),
        &bearer(&alice),
        None,
    );
    assert_eq!(revoked.status, 401);
    server.shutdown();
}

#[test]
fn a_stale_cursor_gets_a_filtered_resync_snapshot_event() {
    let dir = common::TestDir::new("access-proj-resync");
    // A small retention ring: cursor 1 goes stale once the ring overflows.
    let engine = prepare_expanded(
        &dir,
        expanded_scenario("run-access-proj-resync"),
        BTreeMap::new(),
        6,
    );
    let server = HttpServer::start(engine.clone(), "127.0.0.1:0", WebAssets::fallback()).unwrap();
    let addr = server.addr();
    let run_id = engine.run_id().to_string();
    let alice = token_of(&dir, "id-alice");

    for index in 0..30 {
        let (identity, miner) = if index % 2 == 0 {
            ("id-alice", "miner-1")
        } else {
            ("id-bob", "miner-2")
        };
        append_event(
            &engine,
            submitted_event(identity, miner, &format!("share-resync-{index}")),
        );
    }
    let last = engine.evidence().last_event_id();

    let mut stream = SseClient::open(
        addr,
        &run_id,
        "after=1",
        &bearer(&alice),
        Duration::from_secs(5),
    )
    .unwrap();
    let (closed, frames) = stream.read_until_eof(Duration::from_secs(5));
    assert!(closed, "the resync stream should close after the snapshot");
    assert_eq!(frames.len(), 1, "{frames:?}");
    let frame = &frames[0];
    assert_eq!(frame.event, "resync");
    assert_eq!(frame.id, Some(last));
    let value = frame_json(frame);
    assert_eq!(value["schema"], "spc-user-snapshot/v1");
    assert_eq!(value["filtered"], true);
    assert_eq!(value["resync_required"], true);
    assert_eq!(value["evidence"]["last_event_id"].as_u64().unwrap(), last);
    let workers = value["workers"].as_array().unwrap();
    assert_eq!(workers.len(), 1);
    assert_eq!(workers[0]["id"], "w-alice-1");
    // The resync snapshot carries no raw event data at all.
    assert!(!frame.data.contains("id-bob"));
    assert!(!frame.data.contains("share-resync-"));
    server.shutdown();
}

#[test]
fn a_revoked_stream_closes_at_the_next_boundary_or_heartbeat() {
    let dir = common::TestDir::new("access-proj-revoke-stream");
    let engine = prepare_expanded(
        &dir,
        expanded_scenario("run-access-proj-revoke-stream"),
        BTreeMap::new(),
        100_000,
    );
    let server = HttpServer::start(engine.clone(), "127.0.0.1:0", WebAssets::fallback()).unwrap();
    let addr = server.addr();
    let run_id = engine.run_id().to_string();
    let alice = token_of(&dir, "id-alice");
    let bob = token_of(&dir, "id-bob");
    append_event(&engine, miner_event("miner-1", "seed"));

    // Event-boundary closure: revoke, then append an event the user would
    // otherwise receive. The stream re-authenticates before delivering it,
    // closes, and nothing leaks.
    let after = engine.evidence().last_event_id();
    let mut alice_stream = SseClient::open(
        addr,
        &run_id,
        &format!("after={after}"),
        &bearer(&alice),
        Duration::from_secs(5),
    )
    .unwrap();
    revoke(&dir, "id-alice");
    append_event(
        &engine,
        submitted_event("id-alice", "miner-1", "share-after-revoke"),
    );
    let (closed, frames) = alice_stream.read_until_eof(Duration::from_secs(5));
    assert!(closed, "a revoked stream must close");
    assert!(
        frames.is_empty(),
        "frames delivered after revocation: {frames:?}"
    );

    // Idle closure: nothing is appended, so the 15s heartbeat boundary is
    // where the revocation lands. Bounded well inside 20s.
    let after_bob = engine.evidence().last_event_id();
    let mut bob_stream = SseClient::open(
        addr,
        &run_id,
        &format!("after={after_bob}"),
        &bearer(&bob),
        Duration::from_secs(20),
    )
    .unwrap();
    // Let the server enter the live loop before revoking.
    std::thread::sleep(Duration::from_millis(300));
    let revoked_at = Instant::now();
    revoke(&dir, "id-bob");
    let (closed, frames) = bob_stream.read_until_eof(Duration::from_secs(20));
    assert!(closed, "an idle revoked stream must close at the heartbeat");
    assert!(
        frames.is_empty(),
        "frames delivered after revocation: {frames:?}"
    );
    let elapsed = revoked_at.elapsed();
    assert!(
        elapsed < Duration::from_secs(20),
        "the stream outlived the heartbeat bound: {elapsed:?}"
    );
    server.shutdown();
}

#[test]
fn miner_poll_projection_serves_real_rows_or_nulls_with_reasons_never_zeros() {
    let dir = common::TestDir::new("access-proj-miner");
    // The fixture miner reports a snapshot row for w-bob-1 only: bob's
    // claim maps to a real row, alice's does not.
    let mut config = common::fixture_config();
    let miner = config.miners.get_mut("fixture-miner").unwrap();
    miner.args.push("--snapshot-worker".to_string());
    miner.args.push("w-bob-1".to_string());
    let engine = prepare_expanded_with_config(
        &dir,
        expanded_scenario("run-access-proj-miner"),
        config,
        BTreeMap::new(),
        100_000,
    );
    engine.start().unwrap();
    let server = HttpServer::start(engine.clone(), "127.0.0.1:0", WebAssets::fallback()).unwrap();
    let addr = server.addr();
    let run_id = engine.run_id().to_string();
    let alice = token_of(&dir, "id-alice");
    let bob = token_of(&dir, "id-bob");
    let operator = token_of(&dir, "operator");
    let projection_path = format!("/api/lab/v1/runs/{run_id}/projection");

    // Bob's projection is a real miner-poll row.
    let bob_projection = request_raw(addr, "GET", &projection_path, &bearer(&bob), None);
    assert_eq!(bob_projection.status, 200, "{}", bob_projection.text());
    let bob_body = bob_projection.json();
    assert_eq!(bob_body["schema"], "spc-user-projection/v1");
    assert_eq!(bob_body["user_id"], "id-bob");
    assert!(bob_body["observed_at"].as_u64().unwrap() > 0);
    let bob_workers = bob_body["workers"].as_array().unwrap();
    assert_eq!(bob_workers.len(), 1, "{}", bob_projection.text());
    assert_eq!(bob_workers[0]["worker_id"], "w-bob-1");
    assert_eq!(bob_workers[0]["requested_hps"].as_u64().unwrap(), 100_000);
    assert_eq!(bob_workers[0]["achieved_hps"].as_u64().unwrap(), 4242);
    assert_eq!(bob_workers[0]["active_job_id"], "job-fixture-1");
    assert!(bob_workers[0].get("error").is_none());

    // Alice's worker has no row in the miner snapshot: null fields with a
    // reason, never zeros.
    let alice_projection = request_raw(addr, "GET", &projection_path, &bearer(&alice), None);
    assert_eq!(alice_projection.status, 200, "{}", alice_projection.text());
    let alice_body = alice_projection.json();
    assert_eq!(alice_body["user_id"], "id-alice");
    let alice_workers = alice_body["workers"].as_array().unwrap();
    assert_eq!(alice_workers.len(), 1);
    assert_eq!(alice_workers[0]["worker_id"], "w-alice-1");
    assert!(alice_workers[0]["requested_hps"].is_null());
    assert!(alice_workers[0]["achieved_hps"].is_null());
    assert!(alice_workers[0]["active_job_id"].is_null());
    assert_eq!(
        alice_workers[0]["error"],
        "worker row is missing from the miner snapshot"
    );
    assert!(!alice_projection.text().contains("\"requested_hps\":0"));
    assert!(!alice_projection.text().contains("\"achieved_hps\":0"));

    // The operator sees every user's projection.
    let operator_projection = request_raw(addr, "GET", &projection_path, &bearer(&operator), None);
    assert_eq!(operator_projection.status, 200);
    let operator_body = operator_projection.json();
    let array = operator_body.as_array().unwrap();
    assert_eq!(array.len(), 2, "{}", operator_projection.text());
    for entry in array {
        assert_eq!(entry["schema"], "spc-user-projection/v1");
    }
    let user_ids: Vec<&str> = array
        .iter()
        .map(|entry| entry["user_id"].as_str().unwrap())
        .collect();
    assert!(user_ids.contains(&"id-alice") && user_ids.contains(&"id-bob"));
    engine.stop().unwrap();
    server.shutdown();

    // A miner with no control pipe (prepared but not started) reports the
    // closed reason, still never zeros.
    let stale_dir = common::TestDir::new("access-proj-stale");
    let stale_engine = prepare_expanded(
        &stale_dir,
        expanded_scenario("run-access-proj-stale"),
        BTreeMap::new(),
        100_000,
    );
    let stale_server =
        HttpServer::start(stale_engine.clone(), "127.0.0.1:0", WebAssets::fallback()).unwrap();
    let stale_alice = token_of(&stale_dir, "id-alice");
    let stale_run = stale_engine.run_id().to_string();
    let stale = request_raw(
        stale_server.addr(),
        "GET",
        &format!("/api/lab/v1/runs/{stale_run}/projection"),
        &bearer(&stale_alice),
        None,
    );
    assert_eq!(stale.status, 200, "{}", stale.text());
    let stale_body = stale.json();
    let stale_workers = stale_body["workers"].as_array().unwrap();
    assert_eq!(stale_workers.len(), 1);
    assert_eq!(stale_workers[0]["worker_id"], "w-alice-1");
    assert!(stale_workers[0]["requested_hps"].is_null());
    assert!(stale_workers[0]["achieved_hps"].is_null());
    assert!(stale_workers[0]["active_job_id"].is_null());
    assert_eq!(
        stale_workers[0]["error"],
        "miner control pipe is unavailable (not started, adopted or stopped)"
    );
    assert!(!stale.text().contains("\"requested_hps\":0"));
    stale_server.shutdown();
}

/// Rewrite the operator claim in place with a rotated token (0600, same
/// principal): no directory entry is added or removed, only the file bytes
/// change - exactly the case a directory-change signal cannot see.
fn rotate_operator_token(dir: &common::TestDir, new_token: &str) {
    let claim = read_claims(dir)
        .into_iter()
        .find(|claim| claim.principal == "operator")
        .unwrap();
    let mut value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&claim.path).unwrap()).unwrap();
    value["token"] = serde_json::json!(new_token);
    std::fs::write(&claim.path, serde_json::to_vec(&value).unwrap()).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&claim.path, std::fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn an_operator_stream_reauthenticates_and_closes_on_rotation_or_revocation() {
    let dir = common::TestDir::new("access-proj-revoke-operator");
    let engine = prepare_expanded(
        &dir,
        expanded_scenario("run-access-proj-revoke-operator"),
        BTreeMap::new(),
        100_000,
    );
    let server = HttpServer::start(engine.clone(), "127.0.0.1:0", WebAssets::fallback()).unwrap();
    let addr = server.addr();
    let run_id = engine.run_id().to_string();
    let operator = token_of(&dir, "operator");
    let rotated_token = "ef".repeat(32);
    append_event(&engine, miner_event("miner-1", "seed"));

    // Rotation while idle: an in-place token rewrite changes no directory
    // entry, so only the per-request re-validation can observe it. The open
    // stream - authenticated with the old token - closes at the heartbeat
    // boundary, bounded well inside 20s, with nothing further delivered.
    let after = engine.evidence().last_event_id();
    let mut operator_stream = SseClient::open(
        addr,
        &run_id,
        &format!("after={after}"),
        &bearer(&operator),
        Duration::from_secs(5),
    )
    .unwrap();
    // Let the server enter the live loop before rotating.
    std::thread::sleep(Duration::from_millis(300));
    let rotated_at = Instant::now();
    rotate_operator_token(&dir, &rotated_token);
    let (closed, frames) = operator_stream.read_until_eof(Duration::from_secs(20));
    assert!(closed, "a rotated-away operator stream must close");
    assert!(
        frames.is_empty(),
        "frames delivered after rotation: {frames:?}"
    );
    assert!(
        rotated_at.elapsed() < Duration::from_secs(20),
        "the stream outlived the heartbeat bound: {:?}",
        rotated_at.elapsed()
    );

    // The rotated credential itself still works: a fresh operator stream
    // opens (200 asserted in the client) and receives live events. The
    // closure above was the old token losing authority, not a broken store.
    let after_rotation = engine.evidence().last_event_id();
    append_event(&engine, miner_event("miner-1", "after-rotation"));
    let mut fresh_stream = SseClient::open(
        addr,
        &run_id,
        &format!("after={after_rotation}"),
        &bearer(&rotated_token),
        Duration::from_secs(5),
    )
    .unwrap();
    let frames = fresh_stream.drain_until_idle(Duration::from_secs(5));
    assert!(
        frames
            .iter()
            .any(|frame| frame.data.contains("after-rotation")),
        "the rotated operator token must still stream: {frames:?}"
    );

    // Revocation at an event boundary: delete the operator claim, then
    // append an event the operator would otherwise receive (a driver
    // payload with internal detail). The stream re-authenticates before
    // delivering it, closes, and nothing leaks.
    let before_delete = engine.evidence().last_event_id();
    let mut revoked_stream = SseClient::open(
        addr,
        &run_id,
        &format!("after={before_delete}"),
        &bearer(&rotated_token),
        Duration::from_secs(5),
    )
    .unwrap();
    revoke(&dir, "operator");
    append_event(&engine, miner_event("miner-1", "after-operator-revoke"));
    let (closed, frames) = revoked_stream.read_until_eof(Duration::from_secs(5));
    assert!(closed, "a revoked operator stream must close");
    assert!(
        frames.is_empty(),
        "frames delivered after revocation: {frames:?}"
    );
    server.shutdown();
}
