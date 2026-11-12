#![allow(dead_code)]

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use spc_lab::config::{AdapterConfig, LabConfig, NodeEffectiveProfile};
use spc_lab::engine::{EngineOptions, RunEngine};
use spc_lab::scenario::Scenario;
use spc_lab::RunId;

pub fn fixture_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_spc-lab-fixture"))
}

pub struct TestDir {
    path: PathBuf,
}

impl TestDir {
    pub fn new(label: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let count = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::var("SPC_LAB_TEST_ROOT").unwrap_or_else(|_| "/tmp/opencode".to_string());
        let path = Path::new(&root).join(format!(
            "LP3-{}-{}-{count}-{label}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn join(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

pub fn fixture_scenario(name: &str) -> Scenario {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    Scenario::from_json(&text).unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

pub fn fixture_scenario_text(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read_to_string(path).unwrap()
}

fn profile() -> NodeEffectiveProfile {
    NodeEffectiveProfile {
        implementation: Some("fixture-node".to_string()),
        network: Some("regtest".to_string()),
        consensus_profile_id: Some("fixture.regtest.retarget".to_string()),
        profile_fingerprint: None,
        pow_limit_bits: Some("207fffff".to_string()),
        retarget_floor_bits: Some("1e7fffff".to_string()),
        initial_target_bits: Some("1e7fffff".to_string()),
        regtest_pow_bits_overlay: Some("1e7fffff".to_string()),
        target_spacing_secs: Some(10),
        target_timespan_secs: Some(240),
        retarget_interval_blocks: Some(24),
        no_pow_retargeting: Some(false),
        allow_min_difficulty_blocks: Some(false),
        minimum_difficulty_time_gap_secs: None,
        subsidy_initial_sats: Some("5000000000".to_string()),
        subsidy_halving_interval_blocks: Some(150),
        coinbase_maturity_blocks: Some(100),
        witness_commitment_supported: Some(true),
        tdp_session_cap: Some(8),
    }
}

pub fn fixture_config() -> LabConfig {
    let bin = fixture_bin();
    let mut config = LabConfig::default();
    config.nodes.insert(
        "fixture-node".to_string(),
        AdapterConfig {
            program: bin.clone(),
            args: vec![
                "node".to_string(),
                "--port".to_string(),
                "{rpc_port}".to_string(),
                "--height-file".to_string(),
                "{height_file}".to_string(),
            ],
            capabilities: vec!["fixture.scripted".to_string()],
            fixture: true,
            effective_profile: Some(profile()),
            rpc_auth: None,
            env: BTreeMap::new(),
        },
    );
    config.pools.insert(
        "fixture-pool".to_string(),
        AdapterConfig {
            program: bin.clone(),
            args: vec![
                "hold".to_string(),
                "--port".to_string(),
                "{sv2_port}".to_string(),
                "--ready-file".to_string(),
                "{run_dir}/ready-front-{entity}".to_string(),
            ],
            capabilities: vec![
                "fixture.scripted".to_string(),
                "spc.pool.work_unit_v2".to_string(),
                "spc.pool.network_block_work".to_string(),
            ],
            fixture: true,
            effective_profile: None,
            rpc_auth: None,
            env: BTreeMap::new(),
        },
    );
    config.miners.insert(
        "fixture-miner".to_string(),
        AdapterConfig {
            program: bin.clone(),
            args: vec![
                "miner-jsonl".to_string(),
                "--miner-id".to_string(),
                "{entity}".to_string(),
                "--events".to_string(),
                "{run_dir}/miner-{entity}.events.jsonl".to_string(),
            ],
            capabilities: vec!["fixture.scripted".to_string()],
            fixture: true,
            effective_profile: None,
            rpc_auth: None,
            env: BTreeMap::new(),
        },
    );
    let resource = AdapterConfig {
        program: bin,
        args: vec![
            "hold".to_string(),
            "--port".to_string(),
            "{port}".to_string(),
            "--ready-file".to_string(),
            "{run_dir}/ready-{role}-{entity}".to_string(),
        ],
        capabilities: vec!["fixture.scripted".to_string()],
        fixture: true,
        effective_profile: None,
        rpc_auth: None,
        env: BTreeMap::new(),
    };
    config.redis = Some(resource.clone());
    config.postgres = Some(resource);
    config.link_backend = Some("fixture".to_string());
    config
}

pub fn prepare_engine(dir: &TestDir, scenario_name: &str) -> std::sync::Arc<RunEngine> {
    prepare_engine_with(dir, fixture_scenario(scenario_name), fixture_config())
}

pub fn prepare_engine_with(
    dir: &TestDir,
    scenario: Scenario,
    config: LabConfig,
) -> std::sync::Arc<RunEngine> {
    prepare_engine_with_operator_ui(dir, scenario, config, false)
}

/// `prepare_engine_with` with an explicit published-address map, the seam a
/// namespace/topology run uses to wire node P2P through published addresses
/// instead of host loopback.
pub fn prepare_engine_with_addresses(
    dir: &TestDir,
    scenario: Scenario,
    config: LabConfig,
    addresses: BTreeMap<String, String>,
) -> std::sync::Arc<RunEngine> {
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
            link_backend: None,
            operator_ui: false,
        },
    )
    .unwrap()
}

/// `prepare_engine_with` with the EXP-OP `--operator-ui` flag: plans the
/// extra `front.<id>.operator` loopback listener per front.
pub fn prepare_engine_with_operator_ui(
    dir: &TestDir,
    scenario: Scenario,
    config: LabConfig,
    operator_ui: bool,
) -> std::sync::Arc<RunEngine> {
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
            addresses: BTreeMap::new(),
            link_backend: None,
            operator_ui,
        },
    )
    .unwrap()
}

pub fn prepare_engine_with_capacity(
    dir: &TestDir,
    scenario_name: &str,
    capacity: usize,
) -> std::sync::Arc<RunEngine> {
    RunEngine::prepare(
        fixture_scenario(scenario_name),
        EngineOptions {
            run_dir: dir.path().to_path_buf(),
            config: fixture_config(),
            base_revision: "test-base".to_string(),
            dirty_tree_hash: "clean".to_string(),
            owner_instance: format!("test-{}", std::process::id()),
            evidence_capacity: capacity,
            namespaces: BTreeMap::new(),
            nsenter: None,
            user_namespace: None,
            user_namespace_owner: None,
            namespace_owners: BTreeMap::new(),
            namespace_broker: None,
            lifetime_helper: None,
            addresses: BTreeMap::new(),
            link_backend: None,
            operator_ui: false,
        },
    )
    .unwrap()
}

pub fn scenario_with_run_id(mut scenario: Scenario, run_id: &str) -> Scenario {
    scenario.run.id = RunId::new(run_id).unwrap();
    scenario
}

pub fn write_height(dir: &TestDir, node: &str, height: u64) {
    std::fs::write(dir.join(&format!("height-{node}.txt")), height.to_string()).unwrap();
}

pub fn wait_until<F: FnMut() -> bool>(timeout: Duration, mut condition: F) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if condition() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|error| panic!("response is not JSON ({error}): {}", self.text()))
    }
}

pub fn http_request(
    addr: SocketAddr,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> std::io::Result<HttpResponse> {
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let body = body.unwrap_or("");
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    stream.flush()?;
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes)?;
    let split = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap_or(0);
    let head = String::from_utf8_lossy(&bytes[..split]);
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    Ok(HttpResponse {
        status,
        body: bytes[split.saturating_add(4)..].to_vec(),
    })
}

pub struct SseReader {
    reader: BufReader<TcpStream>,
}

impl SseReader {
    pub fn open(addr: SocketAddr, run_id: &str, after: u64) -> std::io::Result<Self> {
        let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        write!(
            stream,
            "GET /api/lab/v1/runs/{run_id}/events?after={after} HTTP/1.1\r\nHost: {addr}\r\nConnection: keep-alive\r\n\r\n"
        )?;
        stream.flush()?;
        let mut reader = BufReader::new(stream);
        let mut status = String::new();
        reader.read_line(&mut status)?;
        assert!(status.contains("200"), "SSE status: {status}");
        loop {
            let mut line = String::new();
            reader.read_line(&mut line)?;
            if line == "\r\n" || line == "\n" {
                break;
            }
        }
        Ok(Self { reader })
    }

    pub fn next_data(&mut self) -> std::io::Result<Option<serde_json::Value>> {
        loop {
            let mut line = String::new();
            if self.reader.read_line(&mut line)? == 0 {
                return Ok(None);
            }
            if let Some(data) = line.strip_prefix("data: ") {
                return Ok(Some(serde_json::from_str(data.trim()).unwrap()));
            }
        }
    }

    pub fn next_event(&mut self) -> std::io::Result<Option<(String, serde_json::Value)>> {
        let mut event = "message".to_string();
        loop {
            let mut line = String::new();
            if self.reader.read_line(&mut line)? == 0 {
                return Ok(None);
            }
            if let Some(value) = line.strip_prefix("event: ") {
                event = value.trim().to_string();
            }
            if let Some(data) = line.strip_prefix("data: ") {
                return Ok(Some((event, serde_json::from_str(data.trim()).unwrap())));
            }
        }
    }
}

pub fn command(run_id: &str, kind: serde_json::Value) -> serde_json::Value {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    serde_json::json!({
        "command_id": format!("cmd-test-{nanos:032x}-{count:016x}"),
        "run_id": run_id,
        "issued_at_wall": 1,
        "actor": "test",
        "kind": kind,
    })
}
