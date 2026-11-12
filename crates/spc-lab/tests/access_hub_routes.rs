//! EXP-ACCESS R4 merge-handoff integration tests.
//!
//! Authored by the merge owner, required by the Fable review's R4
//! conditions - these are NOT part of the candidate's frozen test files.
//! The candidate delta was written against the pre-OP baseline; primary
//! additionally carries the live-lab hub routes. These tests pin the R4
//! classification of those hub routes inside the expanded-access profile:
//!
//! - the hub inventory DTO is operator-only in expanded runs: a user claim
//!   is a closed 403 before any inventory build or I/O, and a tokenless
//!   request is a 401 at the Bearer gate;
//! - a legacy run (no expanded access) keeps the Hub's public inventory
//!   behavior unchanged;
//! - the lab pages join the closed pre-auth static bootstrap list only as
//!   byte-exact Hub assets carrying no inline private fact, status,
//!   address or token.

mod common;

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use spc_lab::engine::{EngineOptions, RunEngine};
use spc_lab::http::{HttpServer, WebAssets};
use spc_lab::ids::RunId;
use spc_lab::scenario::Scenario;

// The real embedded hub assets (the same bytes `spc-lab` embeds via
// include_str!): the served bootstrap pages must be byte-equal to these.
const LAB_HTML: &str = include_str!("../../../bins/spc-lab/static/lab.html");
const LAB_JS: &str = include_str!("../../../bins/spc-lab/static/lab.js");
const LAB_CSS: &str = include_str!("../../../bins/spc-lab/static/lab.css");

fn scenario(run_id: &str, expanded: bool) -> Scenario {
    let mut scenario = common::fixture_scenario("scenario_v1_one_pool_two_miners.json");
    scenario.run.id = RunId::new(run_id).unwrap();
    scenario.expanded_access = expanded;
    scenario
}

fn prepare(
    dir: &common::TestDir,
    scenario: Scenario,
    addresses: BTreeMap<String, String>,
) -> Arc<RunEngine> {
    RunEngine::prepare(
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
            addresses,
            // EXP-OP wiring: these tests never plan the operator page
            // listener (host-only opt-in flag).
            operator_ui: false,
            link_backend: None,
        },
    )
    .unwrap()
}

fn token_of(dir: &common::TestDir, principal: &str) -> String {
    let access_dir = dir.path().join("private").join("access");
    for entry in std::fs::read_dir(&access_dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        if value["principal"].as_str() == Some(principal) {
            return value["token"].as_str().unwrap().to_string();
        }
    }
    panic!("no claim for principal {principal}");
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

fn get(addr: std::net::SocketAddr, path: &str, headers: &[(&str, String)]) -> Response {
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut head = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n");
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).unwrap();
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

#[test]
fn expanded_inventory_is_operator_only() {
    let dir = common::TestDir::new("access-hub-inventory");
    let engine = prepare(
        &dir,
        scenario("run-access-hub-inventory", true),
        BTreeMap::new(),
    );
    engine.start().unwrap();
    let server = HttpServer::start(
        engine.clone(),
        "127.0.0.1:0",
        WebAssets {
            lab_html: LAB_HTML,
            lab_js: LAB_JS,
            lab_css: LAB_CSS,
            ..WebAssets::fallback()
        },
    )
    .unwrap();
    let addr = server.addr();
    let run_id = engine.run_id().to_string();
    let inventory_path = format!("/api/lab/v1/runs/{run_id}/inventory");

    // Tokenless: the Bearer gate answers 401 before any route work.
    let tokenless = get(addr, &inventory_path, &[]);
    assert_eq!(tokenless.status, 401, "{}", tokenless.text());

    // User claim: closed 403 before any inventory build or file I/O.
    let alice = token_of(&dir, "id-alice");
    let user = get(addr, &inventory_path, &bearer(&alice));
    assert_eq!(user.status, 403, "{}", user.text());
    assert!(
        user.text().contains("operator claim"),
        "user 403 must name the operator claim: {}",
        user.text()
    );

    // Operator claim: the Hub-approved inventory DTO.
    let operator = token_of(&dir, "operator");
    let op = get(addr, &inventory_path, &bearer(&operator));
    assert_eq!(op.status, 200, "{}", op.text());
    let dto = op.json();
    assert_eq!(
        dto["schema_version"].as_str(),
        Some("spc.lab.inventory.v1"),
        "operator inventory must be the hub DTO: {dto}"
    );
    engine.stop().unwrap();
    server.shutdown();
}

#[test]
fn legacy_inventory_stays_public_and_unchanged() {
    let dir = common::TestDir::new("access-hub-legacy-inventory");
    let engine = prepare(
        &dir,
        scenario("run-access-hub-legacy", false),
        BTreeMap::new(),
    );
    engine.start().unwrap();
    let server = HttpServer::start(engine.clone(), "127.0.0.1:0", WebAssets::fallback()).unwrap();
    let addr = server.addr();
    let run_id = engine.run_id().to_string();
    let inventory_path = format!("/api/lab/v1/runs/{run_id}/inventory");

    // A legacy run never enters the expanded path: the Hub's public
    // inventory route answers 200 with the same DTO, no Bearer gate.
    let response = get(addr, &inventory_path, &[]);
    assert_eq!(response.status, 200, "{}", response.text());
    let dto = response.json();
    assert_eq!(
        dto["schema_version"].as_str(),
        Some("spc.lab.inventory.v1"),
        "legacy inventory must stay the hub DTO: {dto}"
    );
    engine.stop().unwrap();
    server.shutdown();
}

#[test]
fn expanded_lab_pages_are_public_static_bootstrap() {
    let dir = common::TestDir::new("access-hub-lab-pages");
    let engine = prepare(
        &dir,
        scenario("run-access-hub-lab-pages", true),
        BTreeMap::new(),
    );
    engine.start().unwrap();
    let server = HttpServer::start(
        engine.clone(),
        "127.0.0.1:0",
        WebAssets {
            lab_html: LAB_HTML,
            lab_js: LAB_JS,
            lab_css: LAB_CSS,
            ..WebAssets::fallback()
        },
    )
    .unwrap();
    let addr = server.addr();
    let operator = token_of(&dir, "operator");
    let alice = token_of(&dir, "id-alice");

    // Pre-auth static bootstrap: 200 without a token, byte-equal to the
    // embedded Hub assets, correct content types, CSP on the HTML page,
    // and no token or identity leak in any served byte.
    let page = get(addr, "/lab", &[]);
    assert_eq!(page.status, 200, "{}", page.text());
    assert!(page.head.contains("Content-Type: text/html; charset=utf-8"));
    assert!(page.head.contains("Content-Security-Policy"));
    assert_eq!(page.body, LAB_HTML.as_bytes());

    let script = get(addr, "/lab.js", &[]);
    assert_eq!(script.status, 200, "{}", script.text());
    assert!(script
        .head
        .contains("Content-Type: text/javascript; charset=utf-8"));
    assert_eq!(script.body, LAB_JS.as_bytes());

    let style = get(addr, "/lab.css", &[]);
    assert_eq!(style.status, 200, "{}", style.text());
    assert!(style.head.contains("Content-Type: text/css; charset=utf-8"));
    assert_eq!(style.body, LAB_CSS.as_bytes());

    for (label, response) in [("lab", page), ("lab.js", script), ("lab.css", style)] {
        let text = response.text();
        assert!(
            !text.contains(&operator),
            "{label} leaked the operator token"
        );
        assert!(!text.contains(&alice), "{label} leaked a user token");
        assert!(!text.contains("id-alice"), "{label} leaked an identity id");
    }
    engine.stop().unwrap();
    server.shutdown();
}
