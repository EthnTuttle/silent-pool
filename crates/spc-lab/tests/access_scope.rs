//! EXP-ACCESS scope tests (live-lab plan section 5, AC-1).
//!
//! Fixture-only: no captaind, no pool Ark role, no AP-3b, no T1b, no live
//! funded run. These tests prove the closed negative surface of the
//! expanded-access foundation: bearer gating, scope before journal, replay
//! binding by principal, claim-file privacy and path safety, revocation,
//! relay semantics and byte-identical legacy behavior.

mod common;

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use spc_lab::engine::{EngineOptions, RunEngine};
use spc_lab::http::{HttpServer, WebAssets};
use spc_lab::ids::{IdentityId, RunId};
use spc_lab::scenario::Scenario;

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
) -> Arc<RunEngine> {
    prepare_expanded_with_config(dir, scenario, common::fixture_config(), addresses)
}

fn prepare_expanded_with_config(
    dir: &common::TestDir,
    scenario: Scenario,
    config: spc_lab::config::LabConfig,
    addresses: BTreeMap<String, String>,
) -> Arc<RunEngine> {
    RunEngine::prepare(
        scenario,
        EngineOptions {
            run_dir: dir.path().to_path_buf(),
            config,
            base_revision: "test-base".to_string(),
            dirty_tree_hash: "clean".to_string(),
            owner_instance: format!("test-{}", std::process::id()),
            evidence_capacity: 100_000,
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
    token: String,
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
            token: value["token"].as_str().unwrap().to_string(),
            path,
        });
    }
    claims
}

fn token_of(dir: &common::TestDir, principal: &str) -> String {
    read_claims(dir)
        .into_iter()
        .find(|claim| claim.principal == principal)
        .unwrap_or_else(|| panic!("no claim for principal {principal}"))
        .token
}

fn claim_path_of(dir: &common::TestDir, principal: &str) -> PathBuf {
    read_claims(dir)
        .into_iter()
        .find(|claim| claim.principal == principal)
        .unwrap_or_else(|| panic!("no claim for principal {principal}"))
        .path
}

/// Rewrite a claim file in place with 0600 (the mode every load re-checks;
/// `fs::write` on an existing file preserves it, but be explicit so a
/// fresh file also passes hygiene).
fn write_claim_file(path: &std::path::Path, value: &serde_json::Value) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
}

fn set_mode(path: &std::path::Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

/// The effective uid of this test process (Linux: `/proc/self` is owned by
/// the process owner).
fn test_euid() -> u32 {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self").unwrap().uid()
}

struct Response {
    status: u16,
    head: String,
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
        head,
        body: bytes[split + 4..].to_vec(),
    }
}

fn bearer(token: &str) -> Vec<(&'static str, String)> {
    vec![("Authorization", format!("Bearer {token}"))]
}

fn access_command(run_id: &str, actor: &str, command_id: &str, kind: serde_json::Value) -> String {
    serde_json::json!({
        "command_id": command_id,
        "run_id": run_id,
        "issued_at_wall": 1,
        "actor": actor,
        "kind": kind,
    })
    .to_string()
}

fn unique_command_id(label: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("cmd-access-{label}-{nanos:032x}-{count:016x}")
}

fn journal_line_count(engine: &RunEngine) -> usize {
    let text = std::fs::read_to_string(engine.command_log.path()).unwrap();
    text.lines().filter(|line| !line.trim().is_empty()).count()
}

fn unix_mode(path: &std::path::Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn expanded_access_requires_bearer_for_every_private_route() {
    let dir = common::TestDir::new("access-scope-gate");
    let engine = prepare_expanded(&dir, expanded_scenario("run-access-gate"), BTreeMap::new());
    engine.start().unwrap();
    let server = HttpServer::start(engine.clone(), "127.0.0.1:0", WebAssets::fallback()).unwrap();
    let addr = server.addr();
    let run_id = engine.run_id().to_string();
    let operator = token_of(&dir, "operator");
    let alice = token_of(&dir, "id-alice");
    let bob = token_of(&dir, "id-bob");
    let command = access_command(
        &run_id,
        "user/id-alice",
        &unique_command_id("gate"),
        serde_json::json!({"type": "pause_workers", "miner_id": "miner-1", "workers": ["w-alice-1"]}),
    );

    let journal_before = journal_line_count(&engine);

    // The closed public static bootstrap exception: address-bar page loads
    // work without a token and contain no private facts or tokens.
    for path in ["/", "/index.html", "/app.js", "/style.css"] {
        let response = request_raw(addr, "GET", path, &[], None);
        assert_eq!(response.status, 200, "{path}: {}", response.text());
        let text = response.text();
        assert!(
            !text.contains(&operator),
            "{path} leaked the operator token"
        );
        assert!(!text.contains(&alice), "{path} leaked a user token");
        assert!(!text.contains(&bob), "{path} leaked a user token");
        assert!(!text.contains("id-bob"), "{path} leaked an identity id");
    }
    let index = request_raw(addr, "GET", "/", &[], None);
    assert!(index.head.contains("Content-Security-Policy"));
    assert!(index.head.contains("default-src 'self'"));

    // Every private route is 401 without a token.
    let private_paths = [
        "/api/lab/v1/health".to_string(),
        "/api/lab/v1/runs".to_string(),
        format!("/api/lab/v1/runs/{run_id}/snapshot"),
        format!("/api/lab/v1/runs/{run_id}/events?after=0"),
        format!("/api/lab/v1/runs/{run_id}/me"),
        format!("/api/lab/v1/runs/{run_id}/projection"),
        format!("/api/lab/v1/runs/{run_id}/catalog"),
        format!("/api/lab/v1/runs/{run_id}/commands"),
    ];
    for path in &private_paths {
        let method = if path.ends_with("/commands") {
            "POST"
        } else {
            "GET"
        };
        let response = request_raw(
            addr,
            method,
            path,
            &[],
            if method == "POST" {
                Some(&command)
            } else {
                None
            },
        );
        assert_eq!(response.status, 401, "{path}");
    }

    // Wrong, malformed and duplicated credentials all fail closed the same
    // way, with no private echo.
    let wrong = request_raw(
        addr,
        "GET",
        &format!("/api/lab/v1/runs/{run_id}/me"),
        &[("Authorization", "Bearer 00".repeat(32))],
        None,
    );
    assert_eq!(wrong.status, 401);
    let malformed = request_raw(
        addr,
        "GET",
        &format!("/api/lab/v1/runs/{run_id}/me"),
        &[("Authorization", format!("Token {alice}"))],
        None,
    );
    assert_eq!(malformed.status, 401);
    let short = request_raw(
        addr,
        "GET",
        &format!("/api/lab/v1/runs/{run_id}/me"),
        &[("Authorization", "Bearer nothex".to_string())],
        None,
    );
    assert_eq!(short.status, 401);
    let duplicate = request_raw(
        addr,
        "GET",
        &format!("/api/lab/v1/runs/{run_id}/me"),
        &[
            ("Authorization", format!("Bearer {alice}")),
            ("Authorization", format!("Bearer {alice}")),
        ],
        None,
    );
    assert_eq!(duplicate.status, 401);

    // A valid token reaches the private routes.
    let health = request_raw(addr, "GET", "/api/lab/v1/health", &bearer(&operator), None);
    assert_eq!(health.status, 200);
    let me = request_raw(
        addr,
        "GET",
        &format!("/api/lab/v1/runs/{run_id}/me"),
        &bearer(&alice),
        None,
    );
    assert_eq!(me.status, 200);
    let me_json = me.json();
    assert_eq!(me_json["principal"], "id-alice");
    assert_eq!(me_json["actor"], "user/id-alice");
    assert_eq!(me_json["miner_id"], "miner-1");
    // The /me view never echoes the token.
    assert!(!me.text().contains(&alice));

    // None of the rejected requests wrote a journal entry.
    assert_eq!(journal_line_count(&engine), journal_before);
    engine.stop().unwrap();
    server.shutdown();
}

#[test]
fn user_scope_forged_ids_and_unavailable_kinds_are_closed_before_journal() {
    let dir = common::TestDir::new("access-scope-user");
    let engine = prepare_expanded(&dir, expanded_scenario("run-access-user"), BTreeMap::new());
    engine.start().unwrap();
    let server = HttpServer::start(engine.clone(), "127.0.0.1:0", WebAssets::fallback()).unwrap();
    let addr = server.addr();
    let run_id = engine.run_id().to_string();
    let alice = token_of(&dir, "id-alice");
    let bob = token_of(&dir, "id-bob");
    let operator = token_of(&dir, "operator");

    let post = |token: &str, actor: &str, kind: serde_json::Value| -> Response {
        let body = access_command(&run_id, actor, &unique_command_id("scope"), kind);
        request_raw(
            addr,
            "POST",
            &format!("/api/lab/v1/runs/{run_id}/commands"),
            &bearer(token),
            Some(&body),
        )
    };

    let journal_before = journal_line_count(&engine);

    // A user token may not issue operator commands.
    let stop = post(
        &alice,
        "user/id-alice",
        serde_json::json!({"type": "stop_run", "reason": "user attempt"}),
    );
    assert_eq!(stop.status, 200);
    assert_eq!(stop.json()["state"], "rejected_precondition");
    assert!(stop.json()["detail"]
        .as_str()
        .unwrap()
        .contains("not permitted"));

    // Forged actor: the payload actor must equal the claim's bound actor.
    let forged_actor = post(
        &alice,
        "user/id-bob",
        serde_json::json!({"type": "pause_workers", "miner_id": "miner-1", "workers": ["w-alice-1"]}),
    );
    assert_eq!(forged_actor.json()["state"], "rejected_precondition");
    assert!(forged_actor.json()["detail"]
        .as_str()
        .unwrap()
        .contains("actor"));

    // Cross-user miner.
    let cross_miner = post(
        &alice,
        "user/id-alice",
        serde_json::json!({"type": "pause_workers", "miner_id": "miner-2", "workers": ["w-bob-1"]}),
    );
    assert_eq!(cross_miner.json()["state"], "rejected_precondition");
    assert!(cross_miner.json()["detail"]
        .as_str()
        .unwrap()
        .contains("miner"));

    // Cross-user worker.
    let cross_worker = post(
        &alice,
        "user/id-alice",
        serde_json::json!({"type": "set_worker_budget", "miner_id": "miner-1", "worker_id": "w-bob-1", "hashes_per_second": "1", "total_hashes": null}),
    );
    assert_eq!(cross_worker.json()["state"], "rejected_precondition");
    assert!(cross_worker.json()["detail"]
        .as_str()
        .unwrap()
        .contains("worker"));

    // A cross-user read is closed too.
    let catalog = request_raw(
        addr,
        "GET",
        &format!("/api/lab/v1/runs/{run_id}/catalog"),
        &bearer(&alice),
        None,
    );
    assert_eq!(catalog.status, 403);

    // Candidate kinds with no runtime variant: closed not-available outcome,
    // never journaled, never pretended to be T1.
    for kind in [
        serde_json::json!({
            "type": "send_user_payment",
            "user": "id-alice",
            "destination_address": "mipcBbFg9gMiCh81KU8LsJ9pEFGgBByB3s",
            "amount_sats": 1000,
            "fee_rate_sats_vb": 1
        }),
        serde_json::json!({"type": "bootstrap_fund", "node_id": "node-a"}),
    ] {
        let unavailable = post(&alice, "user/id-alice", kind.clone());
        assert_eq!(unavailable.status, 200, "{kind}");
        assert_eq!(unavailable.json()["state"], "rejected_precondition");
        assert!(unavailable.json()["detail"]
            .as_str()
            .unwrap()
            .contains("not available"));
    }

    // No scope failure or unavailable kind wrote a journal entry.
    assert_eq!(journal_line_count(&engine), journal_before);

    // And a user's own scoped command succeeds through the same surface.
    let own_pause = post(
        &bob,
        "user/id-bob",
        serde_json::json!({"type": "pause_workers", "miner_id": "miner-2", "workers": ["w-bob-1"]}),
    );
    assert_eq!(own_pause.status, 200);
    assert_eq!(own_pause.json()["state"], "succeeded");

    // The operator claim gets the real outcome for an operator command.
    // (After the run is stopped, so this comes last: a stopped run rejects
    // every further command with `run is not running`.)
    let operator_stop_body = access_command(
        &run_id,
        "operator",
        &unique_command_id("op-stop"),
        serde_json::json!({"type": "stop_run", "reason": "operator test"}),
    );
    let operator_stop = request_raw(
        addr,
        "POST",
        &format!("/api/lab/v1/runs/{run_id}/commands"),
        &bearer(&operator),
        Some(&operator_stop_body),
    );
    assert_eq!(operator_stop.status, 200);
    assert_eq!(operator_stop.json()["state"], "succeeded");

    // Only the two real commands were journaled.
    assert_eq!(journal_line_count(&engine), journal_before + 4);
    // The run was already stopped by the operator command; reap the rest.
    let _ = engine.stop();
    server.shutdown();
}

#[test]
fn replay_binding_is_per_principal_and_survives_restart() {
    let dir = common::TestDir::new("access-scope-replay");
    let engine = prepare_expanded(
        &dir,
        expanded_scenario("run-access-replay"),
        BTreeMap::new(),
    );
    engine.start().unwrap();
    let server = HttpServer::start(engine.clone(), "127.0.0.1:0", WebAssets::fallback()).unwrap();
    let addr = server.addr();
    let run_id = engine.run_id().to_string();
    let alice = token_of(&dir, "id-alice");
    let bob = token_of(&dir, "id-bob");

    let command_id = unique_command_id("replay");
    let pause_kind = serde_json::json!({
        "type": "pause_workers", "miner_id": "miner-1", "workers": ["w-alice-1"]
    });
    let post_as = |token: &str, actor: &str, id: &str, kind: serde_json::Value| -> Response {
        let body = access_command(&run_id, actor, id, kind);
        request_raw(
            addr,
            "POST",
            &format!("/api/lab/v1/runs/{run_id}/commands"),
            &bearer(token),
            Some(&body),
        )
    };

    // First submit: real effect, stored outcome.
    let first = post_as(&alice, "user/id-alice", &command_id, pause_kind.clone());
    assert_eq!(first.json()["state"], "succeeded");

    // Same principal, same id, same payload: the stored outcome, unchanged.
    let replay = post_as(&alice, "user/id-alice", &command_id, pause_kind.clone());
    assert_eq!(replay.json()["state"], "succeeded");
    assert_eq!(replay.json()["detail"], first.json()["detail"]);

    // Same principal, same id, changed payload: a durable conflict. (The
    // changed payload stays inside alice's scope - an out-of-scope payload
    // would be rejected before the journal, which is a different test.)
    let changed = serde_json::json!({
        "type": "set_worker_budget", "miner_id": "miner-1", "worker_id": "w-alice-1",
        "hashes_per_second": "5", "total_hashes": null
    });
    let conflict = post_as(&alice, "user/id-alice", &command_id, changed);
    assert_eq!(conflict.json()["state"], "rejected_conflict");

    // A different principal presenting the same id fails closed without
    // revealing that an outcome exists.
    let bob_kind = serde_json::json!({
        "type": "pause_workers", "miner_id": "miner-2", "workers": ["w-bob-1"]
    });
    let cross = post_as(&bob, "user/id-bob", &command_id, bob_kind.clone());
    assert_eq!(cross.json()["state"], "rejected_precondition");
    assert_eq!(
        cross.json()["detail"].as_str().unwrap(),
        "command id is not available for this principal"
    );

    // Journal restart: the binding survives attach.
    drop(server);
    drop(engine);
    let attached = RunEngine::attach(dir.path(), "access-replay-reattach").unwrap();
    let server = HttpServer::start(attached.clone(), "127.0.0.1:0", WebAssets::fallback()).unwrap();
    let addr = server.addr();
    let after_restart = {
        let body = access_command(&run_id, "user/id-alice", &command_id, pause_kind.clone());
        request_raw(
            addr,
            "POST",
            &format!("/api/lab/v1/runs/{run_id}/commands"),
            &bearer(&alice),
            Some(&body),
        )
    };
    assert_eq!(after_restart.json()["state"], "succeeded");
    // And the cross-principal denial survives the restart too.
    let cross_after = {
        let body = access_command(&run_id, "user/id-bob", &command_id, bob_kind);
        request_raw(
            addr,
            "POST",
            &format!("/api/lab/v1/runs/{run_id}/commands"),
            &bearer(&bob),
            Some(&body),
        )
    };
    assert_eq!(cross_after.json()["state"], "rejected_precondition");
    assert_eq!(
        cross_after.json()["detail"].as_str().unwrap(),
        "command id is not available for this principal"
    );
    // The first engine was dropped mid-run on purpose (journal restart); the
    // attached engine adopted its processes, so stop and reap both.
    let _ = attached.stop();
    let _ = attached.reap();
    server.shutdown();
}

#[test]
fn legacy_journal_stays_byte_identical_and_replays_unchanged() {
    let dir = common::TestDir::new("access-scope-legacy");
    let engine = common::prepare_engine(&dir, "scenario_v1_one_pool_two_miners.json");
    engine.start().unwrap();
    let server = HttpServer::start(engine.clone(), "127.0.0.1:0", WebAssets::fallback()).unwrap();
    let addr = server.addr();
    let run_id = engine.run_id().to_string();

    // Unauthenticated loopback, exactly as before the access work.
    let snapshot = common::http_request(
        addr,
        "GET",
        &format!("/api/lab/v1/runs/{run_id}/snapshot"),
        None,
    )
    .unwrap();
    assert_eq!(snapshot.status, 200);
    assert!(snapshot.text().contains("id-bob"));

    let command_id = unique_command_id("legacy");
    let pause = serde_json::json!({
        "type": "pause_workers", "miner_id": "miner-1", "workers": []
    });
    let first = common::http_request(
        addr,
        "POST",
        &format!("/api/lab/v1/runs/{run_id}/commands"),
        Some(&access_command(&run_id, "test", &command_id, pause.clone())),
    )
    .unwrap();
    assert_eq!(first.json()["state"], "succeeded");

    // The persisted journal line carries no scheme discriminator and no
    // principal: byte-for-byte the legacy shape.
    let journal = std::fs::read_to_string(engine.command_log.path()).unwrap();
    assert!(!journal.contains("hash_scheme"), "{journal}");
    assert!(!journal.contains("principal"), "{journal}");

    // Legacy replay: same id + payload returns the stored outcome; a changed
    // payload is a conflict. No authentication exists to bind.
    let replay = common::http_request(
        addr,
        "POST",
        &format!("/api/lab/v1/runs/{run_id}/commands"),
        Some(&access_command(&run_id, "test", &command_id, pause.clone())),
    )
    .unwrap();
    assert_eq!(replay.json()["state"], "succeeded");
    let changed = serde_json::json!({
        "type": "pause_workers", "miner_id": "miner-1", "workers": ["w-alice-1"]
    });
    let conflict = common::http_request(
        addr,
        "POST",
        &format!("/api/lab/v1/runs/{run_id}/commands"),
        Some(&access_command(&run_id, "test", &command_id, changed)),
    )
    .unwrap();
    assert_eq!(conflict.json()["state"], "rejected_conflict");
    engine.stop().unwrap();
    server.shutdown();
}

#[test]
fn expanded_run_fails_closed_on_crafted_journal_lines() {
    let dir = common::TestDir::new("access-scope-crafted");
    let engine = prepare_expanded(
        &dir,
        expanded_scenario("run-access-crafted"),
        BTreeMap::new(),
    );
    let run_id = engine.run_id().to_string();
    drop(engine);

    // A pre-change (legacy, kind-only) begin line and a line with an
    // unknown hash scheme, hand-crafted into the expanded journal.
    let journal_path = dir.path().join("private").join("commands.jsonl");
    let legacy_line = serde_json::json!({
        "record": "begin",
        "seq": 500,
        "command": {
            "command_id": "cmd-crafted-legacy",
            "run_id": run_id,
            "issued_at_wall": 1,
            "actor": "user/id-alice",
            "kind": {"type": "pause_workers", "miner_id": "miner-1", "workers": ["w-alice-1"]}
        },
        "payload_hash": "deadbeef"
    });
    let unknown_scheme_line = serde_json::json!({
        "record": "begin",
        "seq": 501,
        "command": {
            "command_id": "cmd-crafted-unknown",
            "run_id": run_id,
            "issued_at_wall": 1,
            "actor": "user/id-alice",
            "kind": {"type": "pause_workers", "miner_id": "miner-1", "workers": ["w-alice-1"]}
        },
        "payload_hash": "deadbeef",
        "hash_scheme": "some_future_v9",
        "principal": "id-alice"
    });
    let mut text = std::fs::read_to_string(&journal_path).unwrap_or_default();
    text.push_str(&legacy_line.to_string());
    text.push('\n');
    text.push_str(&unknown_scheme_line.to_string());
    text.push('\n');
    std::fs::write(&journal_path, text).unwrap();

    let engine = RunEngine::attach(dir.path(), "access-crafted-reattach").unwrap();
    let server = HttpServer::start(engine.clone(), "127.0.0.1:0", WebAssets::fallback()).unwrap();
    let addr = server.addr();
    let alice = token_of(&dir, "id-alice");
    let pause_kind = serde_json::json!({
        "type": "pause_workers", "miner_id": "miner-1", "workers": ["w-alice-1"]
    });

    for command_id in ["cmd-crafted-legacy", "cmd-crafted-unknown"] {
        let body = access_command(&run_id, "user/id-alice", command_id, pause_kind.clone());
        let response = request_raw(
            addr,
            "POST",
            &format!("/api/lab/v1/runs/{run_id}/commands"),
            &bearer(&alice),
            Some(&body),
        );
        assert_eq!(response.status, 200, "{command_id}");
        assert_eq!(
            response.json()["state"],
            "rejected_precondition",
            "{command_id}"
        );
        assert_eq!(
            response.json()["detail"].as_str().unwrap(),
            "command id is not available for this principal",
            "{command_id}"
        );
    }
    server.shutdown();
}

#[test]
fn claim_files_are_private_atomic_and_never_escape_the_access_dir() {
    let dir = common::TestDir::new("access-scope-hostile");
    // A hostile identity id that is a legal identifier but an illegal path
    // component ('/' and '..').
    let hostile = "u0/../escape";
    let mut scenario = common::fixture_scenario("scenario_v1_one_pool_two_miners.json");
    scenario.run.id = RunId::new("run-access-hostile").unwrap();
    scenario.expanded_access = true;
    for identity in &mut scenario.identities {
        if identity.id.as_str() == "id-alice" {
            identity.id = IdentityId::new(hostile).unwrap();
        }
    }
    for miner in &mut scenario.miners {
        for connection in &mut miner.connections {
            if connection.identity_id.as_str() == "id-alice" {
                connection.identity_id = IdentityId::new(hostile).unwrap();
            }
        }
        for worker in &mut miner.workers {
            if worker.identity_id.as_str() == "id-alice" {
                worker.identity_id = IdentityId::new(hostile).unwrap();
            }
        }
    }
    let engine = prepare_expanded(&dir, scenario, BTreeMap::new());
    let server = HttpServer::start(engine.clone(), "127.0.0.1:0", WebAssets::fallback()).unwrap();
    let addr = server.addr();
    let run_id = engine.run_id().to_string();

    let access_dir = dir.path().join("private").join("access");
    assert!(access_dir.is_dir());
    assert_eq!(unix_mode(&access_dir), 0o700);

    // Three claims: operator, the hostile user and id-bob. Every file is a
    // single safe component inside the access directory, mode 0600.
    let claims = read_claims(&dir);
    assert_eq!(claims.len(), 3, "{:?}", claims.len());
    for claim in &claims {
        let name = claim
            .path
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert!(
            name.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')),
            "unsafe claim filename {name}"
        );
        assert!(!name.starts_with('.'), "claim filename {name} is hidden");
        assert!(name.ends_with(".json"), "claim filename {name}");
        assert_eq!(
            claim.path.parent().unwrap(),
            access_dir,
            "claim escaped the access directory: {}",
            claim.path.display()
        );
        assert_eq!(unix_mode(&claim.path), 0o600, "{}", claim.path.display());
    }

    // The atomic write leaves no temp file behind anywhere in the run tree:
    // every file is a finished rename, never a partial `.tmp-claim-*`.
    for entry in walk(dir.path()) {
        let name = entry.file_name().unwrap().to_str().unwrap().to_string();
        assert!(
            !name.starts_with(".tmp-claim-"),
            "leftover claim temp file: {}",
            entry.display()
        );
    }

    // Nothing named after the hostile id's segments exists anywhere under
    // the run directory.
    for entry in walk(dir.path()) {
        let name = entry.file_name().unwrap().to_str().unwrap().to_string();
        assert!(
            name != "escape" && name != "u0" && name != "..",
            "hostile id created a path component: {}",
            entry.display()
        );
    }

    // The hostile principal's claim still authenticates.
    let hostile_token = token_of(&dir, hostile);
    let me = request_raw(
        addr,
        "GET",
        &format!("/api/lab/v1/runs/{run_id}/me"),
        &bearer(&hostile_token),
        None,
    );
    assert_eq!(me.status, 200);
    assert_eq!(me.json()["principal"], hostile);
    assert_eq!(me.json()["miner_id"], "miner-1");
    server.shutdown();
}

fn walk(root: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path.clone());
            }
            out.push(path);
        }
    }
    out
}

#[test]
fn revocation_closes_access_without_touching_other_principals() {
    let dir = common::TestDir::new("access-scope-revoke");
    let engine = prepare_expanded(
        &dir,
        expanded_scenario("run-access-revoke"),
        BTreeMap::new(),
    );
    engine.start().unwrap();
    let server = HttpServer::start(engine.clone(), "127.0.0.1:0", WebAssets::fallback()).unwrap();
    let addr = server.addr();
    let run_id = engine.run_id().to_string();
    let alice = token_of(&dir, "id-alice");
    let operator = token_of(&dir, "operator");

    let me_path = format!("/api/lab/v1/runs/{run_id}/me");
    let alive = request_raw(addr, "GET", &me_path, &bearer(&alice), None);
    assert_eq!(alive.status, 200);

    let journal_before = journal_line_count(&engine);

    // Revoke alice by deleting the claim file.
    let alice_claim = read_claims(&dir)
        .into_iter()
        .find(|claim| claim.principal == "id-alice")
        .unwrap();
    std::fs::remove_file(&alice_claim.path).unwrap();

    let revoked = request_raw(addr, "GET", &me_path, &bearer(&alice), None);
    assert_eq!(revoked.status, 401);

    // A revoked token cannot submit either, and nothing is journaled.
    let command = access_command(
        &run_id,
        "user/id-alice",
        &unique_command_id("revoked"),
        serde_json::json!({"type": "pause_workers", "miner_id": "miner-1", "workers": ["w-alice-1"]}),
    );
    let revoked_post = request_raw(
        addr,
        "POST",
        &format!("/api/lab/v1/runs/{run_id}/commands"),
        &bearer(&alice),
        Some(&command),
    );
    assert_eq!(revoked_post.status, 401);
    assert_eq!(journal_line_count(&engine), journal_before);

    // Deleting a principal's claim file does not touch the other principals.
    let operator_me = request_raw(addr, "GET", &me_path, &bearer(&operator), None);
    assert_eq!(operator_me.status, 200);
    engine.stop().unwrap();
    server.shutdown();
}

#[test]
fn in_place_claim_edits_take_effect_on_the_next_request() {
    let dir = common::TestDir::new("access-scope-rewrite");
    let engine = prepare_expanded(
        &dir,
        expanded_scenario("run-access-rewrite"),
        BTreeMap::new(),
    );
    engine.start().unwrap();
    let server = HttpServer::start(engine.clone(), "127.0.0.1:0", WebAssets::fallback()).unwrap();
    let addr = server.addr();
    let run_id = engine.run_id().to_string();
    let me_path = format!("/api/lab/v1/runs/{run_id}/me");
    let snapshot_path = format!("/api/lab/v1/runs/{run_id}/snapshot");

    let alice_claim = claim_path_of(&dir, "id-alice");
    let alice_value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&alice_claim).unwrap()).unwrap();
    let alice = token_of(&dir, "id-alice");
    let operator = token_of(&dir, "operator");
    let bob_claim = claim_path_of(&dir, "id-bob");
    let bob_text = std::fs::read_to_string(&bob_claim).unwrap();

    let journal_before = journal_line_count(&engine);

    // Token rotation: a same-size in-place overwrite of alice's claim with
    // a fresh token. No directory entry is added or removed, so no
    // directory signal exists - the old token must die on the very next
    // request and the new one must work.
    let mut rotated = alice_value.clone();
    let new_token = "ab".repeat(32);
    rotated["token"] = serde_json::json!(new_token);
    write_claim_file(&alice_claim, &rotated);
    let old_token = request_raw(addr, "GET", &me_path, &bearer(&alice), None);
    assert_eq!(old_token.status, 401);
    let rotated_ok = request_raw(addr, "GET", &me_path, &bearer(&new_token), None);
    assert_eq!(rotated_ok.status, 200);

    // Scope reduction: removing alice's worker from her claim closes her
    // worker command and her worker read on the next request.
    let mut reduced = alice_value.clone();
    reduced["token"] = serde_json::json!(new_token);
    reduced["worker_ids"] = serde_json::json!([]);
    write_claim_file(&alice_claim, &reduced);
    let command = access_command(
        &run_id,
        "user/id-alice",
        &unique_command_id("rescope"),
        serde_json::json!({"type": "pause_workers", "miner_id": "miner-1", "workers": ["w-alice-1"]}),
    );
    let rescoped = request_raw(
        addr,
        "POST",
        &format!("/api/lab/v1/runs/{run_id}/commands"),
        &bearer(&new_token),
        Some(&command),
    );
    assert_eq!(rescoped.json()["state"], "rejected_precondition");
    let snapshot = request_raw(addr, "GET", &snapshot_path, &bearer(&new_token), None);
    assert_eq!(snapshot.status, 200);
    assert_eq!(snapshot.json()["workers"].as_array().unwrap().len(), 0);
    assert_eq!(journal_line_count(&engine), journal_before);

    // Corruption: in-place garbage in bob's claim denies EVERY token on the
    // very next request - all-or-nothing, the operator included, with no
    // directory change to trigger it.
    std::fs::write(&bob_claim, b"not json").unwrap();
    let operator_denied = request_raw(addr, "GET", &me_path, &bearer(&operator), None);
    assert_eq!(operator_denied.status, 401);
    let alice_denied = request_raw(addr, "GET", &me_path, &bearer(&new_token), None);
    assert_eq!(alice_denied.status, 401);

    // Repair: restoring a valid claim file recovers the whole store.
    std::fs::write(&bob_claim, bob_text.as_bytes()).unwrap();
    set_mode(&bob_claim, 0o600);
    let repaired = request_raw(addr, "GET", &me_path, &bearer(&operator), None);
    assert_eq!(repaired.status, 200);
    let bob = token_of(&dir, "id-bob");
    let bob_ok = request_raw(addr, "GET", &me_path, &bearer(&bob), None);
    assert_eq!(bob_ok.status, 200);
    engine.stop().unwrap();
    server.shutdown();
}

#[test]
fn claim_files_with_bad_mode_type_or_owner_fail_closed() {
    let dir = common::TestDir::new("access-scope-hygiene");
    let engine = prepare_expanded(
        &dir,
        expanded_scenario("run-access-hygiene"),
        BTreeMap::new(),
    );
    engine.start().unwrap();
    let server = HttpServer::start(engine.clone(), "127.0.0.1:0", WebAssets::fallback()).unwrap();
    let addr = server.addr();
    let run_id = engine.run_id().to_string();
    let me_path = format!("/api/lab/v1/runs/{run_id}/me");
    let alice = token_of(&dir, "id-alice");
    let operator = token_of(&dir, "operator");
    let alice_claim = claim_path_of(&dir, "id-alice");
    let access_dir = dir.path().join("private").join("access");
    let both_alive = |what: &str| {
        for token in [&alice, &operator] {
            let response = request_raw(addr, "GET", &me_path, &bearer(token), None);
            assert_eq!(response.status, 200, "{what}: token should work");
        }
    };
    let all_denied = |what: &str| {
        for token in [&alice, &operator] {
            let response = request_raw(addr, "GET", &me_path, &bearer(token), None);
            assert_eq!(response.status, 401, "{what}: everyone must be denied");
        }
    };

    // A group-readable claim file denies everyone until it is restored
    // (never silently chmod'ed back into approval).
    set_mode(&alice_claim, 0o644);
    all_denied("claim 0644");
    set_mode(&alice_claim, 0o600);
    both_alive("claim 0600 restored");

    // A loosened access directory denies everyone until restored.
    set_mode(&access_dir, 0o755);
    all_denied("access dir 0755");
    set_mode(&access_dir, 0o700);
    both_alive("access dir 0700 restored");

    // A symlink standing in for a claim file is rejected before it is ever
    // opened: bounded, no read of the target.
    let bob_claim = claim_path_of(&dir, "id-bob");
    let bob_text = std::fs::read_to_string(&bob_claim).unwrap();
    std::fs::remove_file(&bob_claim).unwrap();
    std::os::unix::fs::symlink(&alice_claim, &bob_claim).unwrap();
    all_denied("symlinked claim");
    std::fs::remove_file(&bob_claim).unwrap();
    std::fs::write(&bob_claim, bob_text.as_bytes()).unwrap();
    set_mode(&bob_claim, 0o600);
    both_alive("symlink removed");

    // A non-regular, non-symlink entry (a directory named like a claim) is
    // rejected the same bounded way - it is never opened or read.
    let trap = access_dir.join("trap.json");
    std::fs::create_dir(&trap).unwrap();
    all_denied("directory named like a claim");
    std::fs::remove_dir(&trap).unwrap();
    both_alive("trap removed");

    // Foreign ownership is only observable as root (chown needs privileges
    // this test user does not have); the case is exercised when possible and
    // explicitly omitted otherwise.
    if test_euid() == 0 {
        use std::os::unix::fs::chown;
        chown(&alice_claim, Some(1u32), None).unwrap();
        all_denied("foreign-owned claim");
        chown(&alice_claim, Some(0u32), None).unwrap();
        both_alive("ownership restored");
    }
    engine.stop().unwrap();
    server.shutdown();
}

#[test]
fn reserved_principals_and_filename_collisions_fail_prepare_closed() {
    for reserved in ["operator", "controller", "sha256-deadbeef"] {
        let dir = common::TestDir::new("access-scope-reserved");
        let mut scenario = common::fixture_scenario("scenario_v1_one_pool_two_miners.json");
        scenario.run.id = RunId::new("run-access-reserved").unwrap();
        scenario.expanded_access = true;
        for identity in &mut scenario.identities {
            if identity.id.as_str() == "id-alice" {
                identity.id = IdentityId::new(reserved).unwrap();
            }
        }
        for miner in &mut scenario.miners {
            for connection in &mut miner.connections {
                if connection.identity_id.as_str() == "id-alice" {
                    connection.identity_id = IdentityId::new(reserved).unwrap();
                }
            }
            for worker in &mut miner.workers {
                if worker.identity_id.as_str() == "id-alice" {
                    worker.identity_id = IdentityId::new(reserved).unwrap();
                }
            }
        }
        let error = RunEngine::prepare(
            scenario,
            EngineOptions {
                run_dir: dir.path().to_path_buf(),
                config: common::fixture_config(),
                base_revision: "test-base".to_string(),
                dirty_tree_hash: "clean".to_string(),
                owner_instance: format!("test-{}", std::process::id()),
                evidence_capacity: 100_000,
                namespaces: BTreeMap::new(),
                nsenter: None,
                user_namespace: None,
                user_namespace_owner: None,
                namespace_owners: BTreeMap::new(),
                namespace_broker: None,
                lifetime_helper: None,
                addresses: BTreeMap::new(),
                operator_ui: false,
                link_backend: None,
            },
        )
        .unwrap_err();
        assert_eq!(error.code(), "access_bindings", "{reserved}: {error}");
        // The rejection happens before any filesystem effect: no claim
        // directory (and nothing under private/) is ever created.
        assert!(
            !dir.path().join("private").join("access").exists(),
            "{reserved}: the access directory must not exist"
        );
    }
}

#[test]
fn forged_claim_files_with_duplicate_authority_fail_closed() {
    let dir = common::TestDir::new("access-scope-forged");
    let engine = prepare_expanded(
        &dir,
        expanded_scenario("run-access-forged"),
        BTreeMap::new(),
    );
    let server = HttpServer::start(engine.clone(), "127.0.0.1:0", WebAssets::fallback()).unwrap();
    let addr = server.addr();
    let run_id = engine.run_id().to_string();
    let me_path = format!("/api/lab/v1/runs/{run_id}/me");
    let alice = token_of(&dir, "id-alice");
    let operator = token_of(&dir, "operator");
    let alice_claim = claim_path_of(&dir, "id-alice");
    let alice_value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&alice_claim).unwrap()).unwrap();
    let access_dir = dir.path().join("private").join("access");
    let all_denied = || {
        for token in [&alice, &operator] {
            let response = request_raw(addr, "GET", &me_path, &bearer(token), None);
            assert_eq!(response.status, 401, "duplicate authority must deny all");
        }
    };

    // A forged file duplicating alice's principal under another name (with a
    // distinct token): duplicate principal is a closed error, never
    // last-wins.
    let mut duplicate_principal = alice_value.clone();
    duplicate_principal["token"] = serde_json::json!("cd".repeat(32));
    let duplicate_path = access_dir.join("forged-principal.json");
    write_claim_file(&duplicate_path, &duplicate_principal);
    all_denied();
    std::fs::remove_file(&duplicate_path).unwrap();

    // A forged file with a fresh principal but alice's token: a duplicated
    // secret is a closed error too.
    let mut duplicate_token = alice_value.clone();
    duplicate_token["principal"] = serde_json::json!("id-carol");
    let token_path = access_dir.join("forged-token.json");
    write_claim_file(&token_path, &duplicate_token);
    all_denied();
    std::fs::remove_file(&token_path).unwrap();

    // A forged file claiming the reserved controller principal.
    let mut controller = alice_value.clone();
    controller["principal"] = serde_json::json!("controller");
    let controller_path = access_dir.join("forged-controller.json");
    write_claim_file(&controller_path, &controller);
    all_denied();
    std::fs::remove_file(&controller_path).unwrap();

    // A user claim carrying operator authority under a user principal.
    let mut mixed = alice_value.clone();
    mixed["may"] = serde_json::json!(["operator"]);
    let mixed_path = access_dir.join("forged-authority.json");
    write_claim_file(&mixed_path, &mixed);
    all_denied();
    std::fs::remove_file(&mixed_path).unwrap();

    // With the forged files gone the store recovers.
    let recovered = request_raw(addr, "GET", &me_path, &bearer(&operator), None);
    assert_eq!(recovered.status, 200);
    server.shutdown();
}

#[test]
fn whole_run_artifacts_are_operator_only() {
    let dir = common::TestDir::new("access-scope-artifacts");
    let engine = prepare_expanded(
        &dir,
        expanded_scenario("run-access-artifacts"),
        BTreeMap::new(),
    );
    engine.start().unwrap();
    let server = HttpServer::start(engine.clone(), "127.0.0.1:0", WebAssets::fallback()).unwrap();
    let addr = server.addr();
    let run_id = engine.run_id().to_string();
    let alice = token_of(&dir, "id-alice");
    let bob = token_of(&dir, "id-bob");
    let operator = token_of(&dir, "operator");

    // Two principals run real commands; the shareable manifest written on
    // refresh (and on stop) carries every principal's command ids and
    // outcome states.
    for (token, actor, miner, worker) in [
        (&alice, "user/id-alice", "miner-1", "w-alice-1"),
        (&bob, "user/id-bob", "miner-2", "w-bob-1"),
    ] {
        let command = access_command(
            &run_id,
            actor,
            &unique_command_id("artifact"),
            serde_json::json!({"type": "pause_workers", "miner_id": miner, "workers": [worker]}),
        );
        let response = request_raw(
            addr,
            "POST",
            &format!("/api/lab/v1/runs/{run_id}/commands"),
            &bearer(token),
            Some(&command),
        );
        assert_eq!(response.json()["state"], "succeeded");
    }
    engine.refresh_artifacts().unwrap();
    let artifact = format!("/api/lab/v1/runs/{run_id}/artifacts/shareable/manifest.json");

    // Tokenless is 401.
    let tokenless = request_raw(addr, "GET", &artifact, &[], None);
    assert_eq!(tokenless.status, 401);

    // A user token is a closed 403 before any file I/O, with no command id
    // or outcome leak in the body.
    let user = request_raw(addr, "GET", &artifact, &bearer(&alice), None);
    assert_eq!(user.status, 403);
    assert_eq!(
        user.text(),
        "403 Forbidden\nartifacts require the operator claim\n"
    );
    assert!(!user.text().contains("cmd-access-artifact"));

    // The operator reads the real whole-run manifest, which does carry the
    // principals' command outcome ids (that is exactly why it is
    // operator-only).
    let operator_view = request_raw(addr, "GET", &artifact, &bearer(&operator), None);
    assert_eq!(operator_view.status, 200);
    assert!(operator_view.text().contains("cmd-access-artifact"));

    // Alias-encoded and traversal paths never bypass the operator gate or
    // the artifact sanitizer: every form is a closed 403 for a user.
    for alias in [
        format!("/api/lab/v1/runs/{run_id}/artifacts/%73hareable/manifest.json"),
        format!("/api/lab/v1/runs/{run_id}/artifacts/shareable/../shareable/manifest.json"),
        format!("/api/lab/v1/runs/{run_id}/artifacts/%2e%2e/private/access/id-alice.json"),
        format!("/api/lab/v1/runs/{run_id}/artifacts/"),
    ] {
        let response = request_raw(addr, "GET", &alias, &bearer(&alice), None);
        assert_eq!(response.status, 403, "{alias}: {}", response.text());
    }

    // A user cannot get another principal's stored outcome by replaying its
    // command id either (the generic denial, no leak that it exists).
    let bob_command_id = {
        let text = std::fs::read_to_string(engine.command_log.path()).unwrap();
        text.lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .find(|value| value["command"]["actor"] == "user/id-bob")
            .map(|value| value["command"]["command_id"].as_str().unwrap().to_string())
            .unwrap()
    };
    let replay = request_raw(
        addr,
        "POST",
        &format!("/api/lab/v1/runs/{run_id}/commands"),
        &bearer(&alice),
        Some(&access_command(
            &run_id,
            "user/id-alice",
            &bob_command_id,
            serde_json::json!({"type": "pause_workers", "miner_id": "miner-1", "workers": ["w-alice-1"]}),
        )),
    );
    assert_eq!(replay.json()["state"], "rejected_precondition");
    assert_eq!(
        replay.json()["detail"].as_str().unwrap(),
        "command id is not available for this principal"
    );
    engine.stop().unwrap();
    server.shutdown();
}

#[test]
fn user_pause_resume_require_explicit_own_workers() {
    let dir = common::TestDir::new("access-scope-workers");
    let engine = prepare_expanded(
        &dir,
        expanded_scenario("run-access-workers"),
        BTreeMap::new(),
    );
    engine.start().unwrap();
    let server = HttpServer::start(engine.clone(), "127.0.0.1:0", WebAssets::fallback()).unwrap();
    let addr = server.addr();
    let run_id = engine.run_id().to_string();
    let alice = token_of(&dir, "id-alice");
    let operator = token_of(&dir, "operator");

    let post = |token: &str, actor: &str, kind: serde_json::Value| -> Response {
        let body = access_command(&run_id, actor, &unique_command_id("workers"), kind);
        request_raw(
            addr,
            "POST",
            &format!("/api/lab/v1/runs/{run_id}/commands"),
            &bearer(token),
            Some(&body),
        )
    };
    let journal_before = journal_line_count(&engine);

    // An empty worker list means "the whole miner" and is operator-only: a
    // user's empty list is a closed scope failure with no journal effect,
    // never a reliance on the prepare-time one-identity-per-miner snapshot.
    let empty = post(
        &alice,
        "user/id-alice",
        serde_json::json!({"type": "pause_workers", "miner_id": "miner-1", "workers": []}),
    );
    assert_eq!(empty.json()["state"], "rejected_precondition");
    assert!(empty.json()["detail"]
        .as_str()
        .unwrap()
        .contains("operator-only"));

    // A foreign worker id on the own miner is denied the same way.
    let foreign = post(
        &alice,
        "user/id-alice",
        serde_json::json!({"type": "pause_workers", "miner_id": "miner-1", "workers": ["w-bob-1"]}),
    );
    assert_eq!(foreign.json()["state"], "rejected_precondition");
    assert_eq!(journal_line_count(&engine), journal_before);

    // Own explicit ids work through the same surface.
    let own = post(
        &alice,
        "user/id-alice",
        serde_json::json!({"type": "pause_workers", "miner_id": "miner-1", "workers": ["w-alice-1"]}),
    );
    assert_eq!(own.json()["state"], "succeeded");
    let resume = post(
        &alice,
        "user/id-alice",
        serde_json::json!({"type": "resume_workers", "miner_id": "miner-1", "workers": ["w-alice-1"]}),
    );
    assert_eq!(resume.json()["state"], "succeeded");

    // The operator's empty list still pauses the whole miner.
    let operator_empty = post(
        &operator,
        "operator",
        serde_json::json!({"type": "pause_workers", "miner_id": "miner-2", "workers": []}),
    );
    assert_eq!(operator_empty.json()["state"], "succeeded");

    // Today no runtime command can add a worker to a live miner (control v1
    // has no add_worker operation), so the prepare-time binding cannot be
    // invalidated through the command surface; the explicit-subset rule
    // above does not lean on that fact.
    let add_worker = post(
        &operator,
        "operator",
        serde_json::json!({
            "type": "add_worker", "miner_id": "miner-1",
            "worker": {
                "id": "w-foreign-1", "identity_id": "id-bob", "connection_id": "conn-alice",
                "budget": {"hashes_per_second": null, "total_hashes": null}
            }
        }),
    );
    assert_eq!(add_worker.json()["state"], "rejected_precondition");
    engine.stop().unwrap();
    server.shutdown();
}

// ---------------------------------------------------------------------------
// Relay and same-origin boundary
// ---------------------------------------------------------------------------

struct UpstreamFixture {
    addr: SocketAddr,
    shutdown: Arc<AtomicBool>,
}

impl UpstreamFixture {
    fn start() -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let shutdown = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&shutdown);
        std::thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            while !flag.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = handle_upstream(stream);
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(10)),
                }
            }
        });
        Self { addr, shutdown }
    }
}

impl Drop for UpstreamFixture {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }
}

fn handle_upstream(mut stream: TcpStream) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            content_length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body)?;
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");
    let (status, reason, content_type, body): (u16, &str, &str, String) = if method != "GET" {
        (
            405,
            "Method Not Allowed",
            "text/plain",
            "method not allowed".to_string(),
        )
    } else {
        match path {
            "/index.html" => (
                200,
                "OK",
                "text/html",
                "<html>agent workbench bootstrap</html>".to_string(),
            ),
            "/workbench.css" => (
                200,
                "OK",
                "text/css",
                "body{font-family:system-ui}".to_string(),
            ),
            "/workbench.js" => (
                200,
                "OK",
                "text/javascript",
                "console.log('workbench')".to_string(),
            ),
            "/status" => (
                200,
                "OK",
                "application/json",
                "{\"agent\":\"status-ok\"}".to_string(),
            ),
            _ => (404, "Not Found", "text/plain", "not found".to_string()),
        }
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes())?;
    stream.flush()
}

#[test]
fn relay_is_a_closed_get_only_declared_role_surface() {
    let dir = common::TestDir::new("access-scope-relay");
    let upstream = UpstreamFixture::start();
    // A dead declared endpoint proves the closed upstream-failure mapping.
    let mut addresses = BTreeMap::new();
    addresses.insert(
        "agent.u0.api".to_string(),
        format!("127.0.0.1:{}", upstream.addr.port()),
    );
    addresses.insert("dead.role.api".to_string(), "127.0.0.1:1".to_string());
    let engine = prepare_expanded(&dir, expanded_scenario("run-access-relay"), addresses);
    let server = HttpServer::start(engine.clone(), "127.0.0.1:0", WebAssets::fallback()).unwrap();
    let addr = server.addr();
    let run_id = engine.run_id().to_string();
    let relay = format!("/api/lab/v1/runs/{run_id}/ui");
    let operator = token_of(&dir, "operator");
    let alice = token_of(&dir, "id-alice");

    // The relayed bootstrap assets are part of the public static exception:
    // address-bar load without a token, no private facts, CSP on the HTML.
    let page = request_raw(
        addr,
        "GET",
        &format!("{relay}/agent.u0/index.html"),
        &[],
        None,
    );
    assert_eq!(page.status, 200);
    assert_eq!(page.text(), "<html>agent workbench bootstrap</html>");
    assert!(page.head.contains("Content-Security-Policy"));
    let css = request_raw(
        addr,
        "GET",
        &format!("{relay}/agent.u0/workbench.css"),
        &[],
        None,
    );
    assert_eq!(css.status, 200);
    assert_eq!(css.text(), "body{font-family:system-ui}");
    let js = request_raw(
        addr,
        "GET",
        &format!("{relay}/agent.u0/workbench.js"),
        &[],
        None,
    );
    assert_eq!(js.status, 200);

    // The relayed status endpoint is private: operator fetches it, a user
    // token is a closed scope failure, no token is 401.
    let status = request_raw(
        addr,
        "GET",
        &format!("{relay}/agent.u0/status"),
        &bearer(&operator),
        None,
    );
    assert_eq!(status.status, 200);
    assert_eq!(status.text(), "{\"agent\":\"status-ok\"}");
    let user_status = request_raw(
        addr,
        "GET",
        &format!("{relay}/agent.u0/status"),
        &bearer(&alice),
        None,
    );
    assert_eq!(user_status.status, 403);
    let tokenless_status = request_raw(addr, "GET", &format!("{relay}/agent.u0/status"), &[], None);
    assert_eq!(tokenless_status.status, 401);

    // The relay is GET-only.
    let relay_post = request_raw(
        addr,
        "POST",
        &format!("{relay}/agent.u0/index.html"),
        &bearer(&operator),
        Some("{}"),
    );
    assert_eq!(relay_post.status, 405);

    // Unknown role, unknown asset and traversal attempts are closed 404s. A
    // bare role path (no asset segment) is not a relay route; it falls to
    // the artifact sanitizer's closed 403, the same closed answer the
    // legacy surface gives for an unshareable path.
    for (path, expected) in [
        (format!("{relay}/unknown-role/index.html"), 404),
        (format!("{relay}/agent.u0/secret.png"), 404),
        (format!("{relay}/agent.u0/../../etc/passwd"), 404),
        (format!("{relay}/agent.u0"), 403),
    ] {
        let response = request_raw(addr, "GET", &path, &bearer(&operator), None);
        assert_eq!(response.status, expected, "{path}");
    }

    // An upstream failure becomes the relay's own closed error with no body
    // detail from the upstream.
    let dead = request_raw(
        addr,
        "GET",
        &format!("{relay}/dead.role/index.html"),
        &bearer(&operator),
        None,
    );
    assert_eq!(dead.status, 502);
    assert_eq!(
        dead.text(),
        "502 Bad Gateway\nrole upstream is unavailable\n"
    );

    // Same-origin boundary on native controller POSTs: a cross-origin or
    // cross-site POST is 403, and no CORS headers exist anywhere.
    let command = access_command(
        &run_id,
        "operator",
        &unique_command_id("origin"),
        serde_json::json!({"type": "assert", "assertion_id": "assert-started"}),
    );
    let cross_origin = request_raw(
        addr,
        "POST",
        &format!("/api/lab/v1/runs/{run_id}/commands"),
        &[
            ("Authorization", format!("Bearer {operator}")),
            ("Origin", "http://evil.example".to_string()),
        ],
        Some(&command),
    );
    assert_eq!(cross_origin.status, 403);
    let cross_site = request_raw(
        addr,
        "POST",
        &format!("/api/lab/v1/runs/{run_id}/commands"),
        &[
            ("Authorization", format!("Bearer {operator}")),
            ("Sec-Fetch-Site", "cross-site".to_string()),
        ],
        Some(&command),
    );
    assert_eq!(cross_site.status, 403);
    assert!(!cross_site
        .head
        .to_ascii_lowercase()
        .contains("access-control-allow-origin"));

    // No token appears in any served page source.
    for path in [
        format!("{relay}/agent.u0/index.html"),
        format!("{relay}/agent.u0/workbench.js"),
        "/".to_string(),
    ] {
        let response = request_raw(addr, "GET", &path, &[], None);
        let text = response.text();
        assert!(
            !text.contains(&operator),
            "{path} leaked the operator token"
        );
        assert!(!text.contains(&alice), "{path} leaked a user token");
    }
    server.shutdown();
    drop(upstream);
}

#[test]
fn legacy_loopback_behavior_is_unchanged() {
    let dir = common::TestDir::new("access-scope-legacy-loopback");
    let engine = common::prepare_engine(&dir, "scenario_v1_one_pool_two_miners.json");
    engine.start().unwrap();
    let server = HttpServer::start(engine.clone(), "127.0.0.1:0", WebAssets::fallback()).unwrap();
    let addr = server.addr();
    let run_id = engine.run_id().to_string();

    // No claims exist for a legacy run.
    assert!(!dir.path().join("private").join("access").exists());

    // Unauthenticated POST with an arbitrary actor works exactly as today.
    let command = access_command(
        &run_id,
        "test",
        &unique_command_id("legacy-loopback"),
        serde_json::json!({"type": "pause_workers", "miner_id": "miner-1", "workers": []}),
    );
    let response = common::http_request(
        addr,
        "POST",
        &format!("/api/lab/v1/runs/{run_id}/commands"),
        Some(&command),
    )
    .unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(response.json()["state"], "succeeded");

    // The whole-run snapshot, the catalog and the SSE stream stay
    // unauthenticated.
    let snapshot = common::http_request(
        addr,
        "GET",
        &format!("/api/lab/v1/runs/{run_id}/snapshot"),
        None,
    )
    .unwrap();
    assert_eq!(snapshot.status, 200);
    let catalog = common::http_request(
        addr,
        "GET",
        &format!("/api/lab/v1/runs/{run_id}/catalog"),
        None,
    )
    .unwrap();
    assert_eq!(catalog.status, 200);
    let mut events = common::SseReader::open(addr, &run_id, 0).unwrap();
    assert!(events.next_data().unwrap().is_some());
    engine.stop().unwrap();
    server.shutdown();
}

/// A synthetic deterministic command whose legacy begin line is asserted
/// byte-for-byte against a frozen literal: this proves today's legacy writer
/// still emits the exact pre-access on-disk FORMAT (field order, absent
/// scheme/principal fields, kind-only hash
/// `152f875193c6eea17ded428dab59d1b6f501688bf0d58f47774a670a4db53c39` =
/// sha256 over `{"type":"pause_workers","miner_id":"miner-1","workers":[]}`).
/// It is NOT a baseline capture; the real preserved pre-access bytes are in
/// `preserved_pre_access_journal_round_trips_byte_for_byte` below.
const FROZEN_LEGACY_BEGIN_LINE: &str = concat!(
    "{\"record\":\"begin\",\"seq\":1,",
    "\"command\":{\"command_id\":\"cmd-frozen-legacy\",",
    "\"run_id\":\"run-frozen-legacy\",\"issued_at_wall\":1,\"actor\":\"test\",",
    "\"kind\":{\"type\":\"pause_workers\",\"miner_id\":\"miner-1\",\"workers\":[]}},",
    "\"payload_hash\":\"152f875193c6eea17ded428dab59d1b6f501688bf0d58f47774a670a4db53c39\"}\n"
);

#[test]
fn frozen_legacy_begin_line_round_trips_byte_for_byte() {
    use spc_lab::command::{BeginOutcome, CommandKind, CommandLog, CommandOutcome, LabCommand};
    use spc_lab::ids::{ActorId, CommandId, MinerId};

    let command = || LabCommand {
        command_id: CommandId::trusted("cmd-frozen-legacy"),
        run_id: RunId::trusted("run-frozen-legacy"),
        issued_at_wall: 1,
        actor: ActorId::trusted("test"),
        kind: CommandKind::PauseWorkers {
            miner_id: MinerId::trusted("miner-1"),
            workers: vec![],
        },
    };

    // Today's legacy writer (expanded access off) emits byte-for-byte the
    // frozen format literal for the same deterministic command.
    let dir = common::TestDir::new("access-scope-frozen");
    let path = dir.path().join("command.log");
    let log = CommandLog::open(&path).unwrap();
    assert!(matches!(log.begin(&command()).unwrap(), BeginOutcome::New));
    drop(log);
    let produced = std::fs::read_to_string(&path).unwrap();
    assert_eq!(produced, FROZEN_LEGACY_BEGIN_LINE);

    // A journal holding only the frozen baseline line round-trips through a
    // fresh open: the entry parses, a legacy begin of the same command is
    // recognized as recorded, a finish makes the outcome durable, and the
    // next legacy begin returns Existing (restart included).
    let dir = common::TestDir::new("access-scope-frozen-replay");
    let path = dir.path().join("command.log");
    std::fs::write(&path, FROZEN_LEGACY_BEGIN_LINE).unwrap();
    let log = CommandLog::open(&path).unwrap();
    match log.begin(&command()).unwrap() {
        BeginOutcome::InFlight(outcome) => {
            assert_eq!(outcome.state, spc_lab::OutcomeState::Running)
        }
        other => panic!("expected InFlight over the frozen line, got {other:?}"),
    }
    let outcome = CommandOutcome {
        command_id: CommandId::trusted("cmd-frozen-legacy"),
        state: spc_lab::OutcomeState::Succeeded,
        detail: "frozen replay".to_string(),
        resolved_at: 2,
        journal_seq: 0,
    };
    log.finish(&outcome.command_id, &outcome).unwrap();
    match log.begin(&command()).unwrap() {
        BeginOutcome::Existing(returned) => {
            assert_eq!(returned.state, spc_lab::OutcomeState::Succeeded)
        }
        other => panic!("expected Existing after finish, got {other:?}"),
    }
    drop(log);
    let log = CommandLog::open(&path).unwrap();
    assert!(matches!(
        log.begin(&command()).unwrap(),
        BeginOutcome::Existing(_)
    ));
}

#[test]
fn legacy_begin_denies_a_scheme_bearing_entry() {
    use spc_lab::command::{BeginOutcome, CommandKind, CommandLog, LabCommand};
    use spc_lab::ids::{ActorId, CommandId, MinerId, WorkerId};

    let dir = common::TestDir::new("access-scope-scheme-denied");
    let path = dir.path().join("command.log");
    let command = LabCommand {
        command_id: CommandId::trusted("cmd-scheme-denied"),
        run_id: RunId::trusted("run-scheme-denied"),
        issued_at_wall: 1,
        actor: ActorId::trusted("user/id-alice"),
        kind: CommandKind::PauseWorkers {
            miner_id: MinerId::trusted("miner-1"),
            workers: vec![WorkerId::trusted("w-alice-1")],
        },
    };

    // An expanded-access begin writes a scheme-bearing entry...
    let log = CommandLog::open(&path).unwrap();
    assert!(matches!(
        log.begin_scoped(&command, "id-alice").unwrap(),
        BeginOutcome::New
    ));
    drop(log);

    // ...and a legacy begin over the same journal (fresh open, the
    // engine-restart shape) is denied: no journal write, no leak that an
    // outcome exists.
    let log = CommandLog::open(&path).unwrap();
    match log.begin(&command).unwrap() {
        BeginOutcome::Denied(outcome) => {
            assert_eq!(outcome.state, spc_lab::OutcomeState::RejectedPrecondition);
            assert_eq!(
                outcome.detail,
                "command id is not available for this principal"
            );
        }
        other => {
            panic!("expected Denied for a legacy begin on a scheme-bearing entry, got {other:?}")
        }
    }
    drop(log);
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text.lines().count(), 1, "the denial must not append a line");
    assert!(text.contains("expanded_v1"));
}

/// The REAL preserved pre-access command journal of a finished live hub run,
/// copied verbatim (read-only source, never modified by this test):
/// `/tmp/opencode/LP-hub-run-1/private/commands.jsonl`
/// (sha256 `ac67e403edf1cd7a7db67da99ac43a098a01e0d27f889e29c455d9ff4a5b5866`,
/// frozen 2026-11-08 before any EXP-ACCESS change, exactly one StopRun
/// begin/finish pair, no scheme/principal fields - a pure legacy journal).
/// The records carry only public lab facts (command/run ids, wall timestamps,
/// the `hub-validation` actor, a stop reason and the stored outcome): no
/// token, credential, address or identity payload, so copying them into this
/// owned fixture test leaks nothing the source run kept private.
const PRESERVED_PRE_ACCESS_BEGIN: &str = "{\"record\":\"begin\",\"seq\":1,\"command\":{\"command_id\":\"cmd-hub-validation-000000000000000018e60223df7db22a\",\"run_id\":\"lab-runtime-v2-two-pools\",\"issued_at_wall\":1794123854650,\"actor\":\"hub-validation\",\"kind\":{\"type\":\"stop_run\",\"reason\":\"hub validation complete\"}},\"payload_hash\":\"24a8e64d76bd87718dcaedf661c0c802052be02de1298bcf36424b83c810e505\"}\n";
const PRESERVED_PRE_ACCESS_FINISH: &str = "{\"record\":\"finish\",\"seq\":2,\"command_id\":\"cmd-hub-validation-000000000000000018e60223df7db22a\",\"outcome\":{\"command_id\":\"cmd-hub-validation-000000000000000018e60223df7db22a\",\"state\":\"succeeded\",\"detail\":\"run stopped (final state stopped)\",\"resolved_at\":1794123855285,\"journal_seq\":1}}\n";

#[test]
fn preserved_pre_access_journal_round_trips_byte_for_byte() {
    use spc_lab::command::{BeginOutcome, CommandLog, LabCommand};

    // Reconstruct the command from the preserved bytes themselves.
    let begin_value: serde_json::Value =
        serde_json::from_str(PRESERVED_PRE_ACCESS_BEGIN.trim_end()).unwrap();
    let command: LabCommand = serde_json::from_value(begin_value["command"].clone()).unwrap();

    // 1. The SHA kind matches the baseline: today's kind-only payload hash
    //    over the captured command equals the payload_hash the pre-access
    //    writer stored in the preserved line.
    assert_eq!(
        CommandLog::payload_hash(&command),
        begin_value["payload_hash"].as_str().unwrap(),
        "today's kind-only hash must equal the preserved baseline hash"
    );

    // 2. Read path: a journal holding exactly the preserved bytes parses
    //    through a fresh open, and a legacy begin of the same command
    //    returns the stored outcome (Existing) - never a conflict, never a
    //    rewrite.
    let dir = common::TestDir::new("access-scope-preserved");
    let path = dir.path().join("command.log");
    std::fs::write(
        &path,
        format!("{PRESERVED_PRE_ACCESS_BEGIN}{PRESERVED_PRE_ACCESS_FINISH}"),
    )
    .unwrap();
    let log = CommandLog::open(&path).unwrap();
    match log.begin(&command).unwrap() {
        BeginOutcome::Existing(outcome) => {
            assert_eq!(outcome.command_id, command.command_id);
            assert_eq!(outcome.state, spc_lab::OutcomeState::Succeeded);
            assert_eq!(outcome.detail, "run stopped (final state stopped)");
        }
        other => panic!("expected Existing over the preserved journal, got {other:?}"),
    }
    // No journal write happened: the bytes are untouched.
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        format!("{PRESERVED_PRE_ACCESS_BEGIN}{PRESERVED_PRE_ACCESS_FINISH}")
    );

    // 3. An authenticated (expanded) begin never adopts the legacy unbound
    //    entry: the real old journal is not expanded authority.
    match log.begin_scoped(&command, "operator").unwrap() {
        BeginOutcome::Denied(outcome) => {
            assert_eq!(outcome.state, spc_lab::OutcomeState::RejectedPrecondition);
            assert_eq!(
                outcome.detail,
                "command id is not available for this principal"
            );
        }
        other => panic!("expected Denied for begin_scoped over a legacy entry, got {other:?}"),
    }
    drop(log);

    // 4. Write path, byte-exact: today's legacy writer, given the same
    //    command and outcome, reproduces the preserved journal
    //    byte-for-byte from scratch.
    let dir = common::TestDir::new("access-scope-preserved-rewrite");
    let path = dir.path().join("command.log");
    let log = CommandLog::open(&path).unwrap();
    assert!(matches!(log.begin(&command).unwrap(), BeginOutcome::New));
    let finish_value: serde_json::Value =
        serde_json::from_str(PRESERVED_PRE_ACCESS_FINISH.trim_end()).unwrap();
    let outcome: spc_lab::CommandOutcome =
        serde_json::from_value(finish_value["outcome"].clone()).unwrap();
    log.finish(&command.command_id, &outcome).unwrap();
    drop(log);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        format!("{PRESERVED_PRE_ACCESS_BEGIN}{PRESERVED_PRE_ACCESS_FINISH}"),
        "today's legacy writer must reproduce the preserved journal bytes"
    );
}
