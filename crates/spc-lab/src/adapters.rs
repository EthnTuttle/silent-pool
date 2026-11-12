//! Explicit node and miner adapter contracts.
//!
//! The runner never guesses a node flag or a miner CLI. A configured adapter
//! supplies the executable, arguments and capability facts. A fixture adapter
//! is tagged as such and its evidence cannot support a real-mining claim.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::{ChildStdin, ChildStdout};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::NodeEffectiveProfile;
use crate::ids::{CommandId, MinerId, NodeId};
use crate::resource::Endpoint;

pub trait NodeControl: Send + Sync {
    fn node_id(&self) -> &NodeId;
    fn fixture(&self) -> bool;
    fn height(&self) -> Result<u64, String>;
    fn effective_profile(&self) -> Result<NodeEffectiveProfile, String>;
}

#[derive(Debug, Clone)]
pub struct FixtureNodeControl {
    node_id: NodeId,
    profile: NodeEffectiveProfile,
    height_file: PathBuf,
}

impl FixtureNodeControl {
    pub fn new(node_id: NodeId, profile: NodeEffectiveProfile, height_file: PathBuf) -> Self {
        Self {
            node_id,
            profile,
            height_file,
        }
    }
}

impl NodeControl for FixtureNodeControl {
    fn node_id(&self) -> &NodeId {
        &self.node_id
    }

    fn fixture(&self) -> bool {
        true
    }

    fn height(&self) -> Result<u64, String> {
        let text = std::fs::read_to_string(&self.height_file)
            .map_err(|e| format!("fixture height file: {e}"))?;
        text.trim()
            .parse::<u64>()
            .map_err(|e| format!("fixture height is malformed: {e}"))
    }

    fn effective_profile(&self) -> Result<NodeEffectiveProfile, String> {
        Ok(self.profile.clone())
    }
}

#[derive(Debug, Clone)]
/// The namespace a node control talks to.
///
/// A namespaced node only listens inside its namespace, so the call has to be
/// executed there.  The broker runs the helper binary in the namespace and
/// returns the answer.
pub struct NamespaceClient {
    pub broker: std::sync::Arc<crate::namespace::NamespaceBroker>,
    pub endpoint: String,
}

pub struct RpcNodeControl {
    node_id: NodeId,
    endpoint: Endpoint,
    auth: Option<String>,
    auth_file: Option<PathBuf>,
    namespace: Option<NamespaceClient>,
}

impl RpcNodeControl {
    /// Typed traffic route. The namespace helper returns this enum itself;
    /// no English error string is ever interpreted as node validation.
    pub fn call_traffic(
        &self,
        request_id: u64,
        method: &str,
        params: Value,
    ) -> crate::traffic::rpc::RpcOutcome {
        use crate::traffic::rpc::{RpcOutcome, UnknownClass};
        if let Some(namespace) = &self.namespace {
            let Some(auth_file) = self.auth_file.as_deref() else {
                return RpcOutcome::unknown(UnknownClass::Broker);
            };
            return namespace.broker.exec_rpc_typed(
                &namespace.endpoint,
                &format!("http://{}:{}/", self.endpoint.host, self.endpoint.port),
                request_id,
                method,
                &params,
                auth_file,
            );
        }
        crate::traffic::rpc::call_direct(
            &self.endpoint,
            self.auth_header().as_deref(),
            request_id,
            method,
            params,
        )
    }

    pub fn new(node_id: NodeId, endpoint: Endpoint, auth: Option<String>) -> Self {
        Self {
            node_id,
            endpoint,
            auth,
            auth_file: None,
            namespace: None,
        }
    }

    /// A control that reaches the node through the namespace broker.
    pub fn in_namespace(mut self, namespace: NamespaceClient) -> Self {
        self.namespace = Some(namespace);
        self
    }

    /// Use the run-private cookie file for namespace-local helper calls. This
    /// path is the only credential material passed through broker argv.
    pub fn with_auth_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.auth_file = Some(path.into());
        self
    }

    pub fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        if let Some(namespace) = &self.namespace {
            let auth_file = self.auth_file.as_deref().ok_or_else(|| {
                format!(
                    "namespaced node {} has no private RPC auth file; refusing an argv credential",
                    self.node_id
                )
            })?;
            let url = format!("http://{}:{}/", self.endpoint.host, self.endpoint.port);
            let response = namespace
                .broker
                .exec_rpc(&namespace.endpoint, &url, method, &params, Some(auth_file))
                .map_err(|error| error.to_string())?;
            if let Some(error) = response.get("error").filter(|value| !value.is_null()) {
                return Err(format!("RPC {method} error: {error}"));
            }
            return Ok(response.get("result").cloned().unwrap_or(Value::Null));
        }
        self.call_direct(method, params)
    }

    fn call_direct(&self, method: &str, params: Value) -> Result<Value, String> {
        let body = serde_json::json!({
            "jsonrpc": "1.0",
            "id": 1,
            "method": method,
            "params": params,
        })
        .to_string();
        let auth = self.auth_header();
        let response = http_post(
            &self.endpoint,
            auth.as_deref(),
            "application/json",
            body.as_bytes(),
        )?;
        let value: Value =
            serde_json::from_slice(&response).map_err(|e| format!("RPC JSON: {e}"))?;
        if let Some(error) = value.get("error").filter(|v| !v.is_null()) {
            return Err(format!("RPC {method} error: {error}"));
        }
        Ok(value.get("result").cloned().unwrap_or(Value::Null))
    }
}

impl RpcNodeControl {
    fn auth_header(&self) -> Option<String> {
        let auth = self.auth.as_deref()?;
        if let Some(token) = auth.strip_prefix("bearer:") {
            return Some(format!("Bearer {token}"));
        }
        if let Some(path) = auth.strip_prefix("bearer-file:") {
            let token = std::fs::read_to_string(path).ok()?;
            return Some(format!("Bearer {}", token.trim()));
        }
        if let Some(credentials) = auth.strip_prefix("basic:") {
            return Some(format!("Basic {}", base64(credentials.as_bytes())));
        }
        Some(format!("Basic {}", base64(auth.as_bytes())))
    }
}

impl NodeControl for RpcNodeControl {
    fn node_id(&self) -> &NodeId {
        &self.node_id
    }

    fn fixture(&self) -> bool {
        false
    }

    fn height(&self) -> Result<u64, String> {
        self.call("getblockcount", Value::Array(Vec::new()))?
            .as_u64()
            .ok_or_else(|| "getblockcount did not return an integer".to_string())
    }

    fn effective_profile(&self) -> Result<NodeEffectiveProfile, String> {
        let info = self.call("getblockchaininfo", Value::Array(Vec::new()))?;
        let consensus = info
            .get("consensus")
            .ok_or_else(|| "getblockchaininfo has no effective consensus object".to_string())?;
        let profile = NodeEffectiveProfile {
            implementation: None,
            network: consensus
                .get("network")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            consensus_profile_id: consensus
                .get("consensus_profile_id")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            profile_fingerprint: consensus
                .get("profile_fingerprint")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            pow_limit_bits: consensus
                .get("pow_limit_bits")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            retarget_floor_bits: consensus
                .get("retarget_floor_bits")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            initial_target_bits: consensus
                .get("initial_target_bits")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            regtest_pow_bits_overlay: consensus
                .get("regtest_pow_bits_overlay")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            target_spacing_secs: consensus.get("pow_target_spacing").and_then(|v| v.as_u64()),
            target_timespan_secs: consensus
                .get("pow_target_timespan")
                .and_then(|v| v.as_u64()),
            retarget_interval_blocks: consensus
                .get("difficulty_adjustment_interval")
                .and_then(|v| v.as_u64()),
            no_pow_retargeting: consensus
                .get("no_pow_retargeting")
                .and_then(|v| v.as_bool()),
            allow_min_difficulty_blocks: consensus
                .get("allow_min_difficulty_blocks")
                .and_then(|v| v.as_bool()),
            minimum_difficulty_time_gap_secs: None,
            subsidy_initial_sats: consensus
                .get("subsidy_initial_sats")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            subsidy_halving_interval_blocks: consensus
                .get("subsidy_halving_interval")
                .and_then(|v| v.as_u64()),
            coinbase_maturity_blocks: consensus.get("coinbase_maturity").and_then(|v| v.as_u64()),
            witness_commitment_supported: consensus
                .get("witness_commitment_supported")
                .and_then(|v| v.as_bool()),
            tdp_session_cap: consensus.get("tdp_session_cap").and_then(|v| v.as_u64()),
        };
        Ok(profile)
    }
}

impl NodeControl for Arc<RpcNodeControl> {
    fn node_id(&self) -> &NodeId {
        self.as_ref().node_id()
    }
    fn fixture(&self) -> bool {
        false
    }
    fn height(&self) -> Result<u64, String> {
        self.as_ref().height()
    }
    fn effective_profile(&self) -> Result<NodeEffectiveProfile, String> {
        self.as_ref().effective_profile()
    }
}

/// Exact LP-2 control protocol, version 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MinerOperation {
    AddConnection,
    RemoveConnection,
    SetWorkerBudget,
    PauseWorkers,
    ResumeWorkers,
    SwitchPool,
    Snapshot,
    Stop,
}

impl MinerOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AddConnection => "add_connection",
            Self::RemoveConnection => "remove_connection",
            Self::SetWorkerBudget => "set_worker_budget",
            Self::PauseWorkers => "pause_workers",
            Self::ResumeWorkers => "resume_workers",
            Self::SwitchPool => "switch_pool",
            Self::Snapshot => "snapshot",
            Self::Stop => "stop",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MinerWireCommand {
    pub protocol: String,
    pub version: u8,
    pub command_id: CommandId,
    pub operation: MinerOperation,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MinerWireError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MinerWireResponse {
    pub protocol: String,
    pub version: u8,
    pub kind: String,
    pub command_id: CommandId,
    pub outcome: String,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<MinerWireError>,
}

impl MinerWireResponse {
    pub fn is_success(&self) -> bool {
        matches!(self.outcome.as_str(), "running" | "succeeded")
    }

    pub fn is_rejected(&self) -> bool {
        self.outcome == "rejected_precondition"
    }

    pub fn error_text(&self) -> String {
        match &self.error {
            Some(error) => format!("{}: {}", error.code, error.message),
            None => self.outcome.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MinerEventBody {
    pub name: String,
    pub payload: Value,
}

/// Event envelopes tolerate additive fields so a protocol revision can add
/// correlation metadata without the runner silently dropping observations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MinerWireEvent {
    pub protocol: String,
    pub version: u8,
    pub kind: String,
    pub source: String,
    pub event_id: u64,
    pub source_cursor: u64,
    pub miner_id: MinerId,
    pub occurred_at_wall_ms: u64,
    pub occurred_at_mono_ms: u64,
    pub dropped_before: u64,
    pub event: MinerEventBody,
}

/// JSONL miner control wrapper for `spc-cpu-miner-control` version 1.
///
/// Two SEPARATE transports, two separate rules:
///
/// 1. This control stream (JSONL over the miner's stdin) is command/response
///    plumbing only. Its EOF semantics are teardown semantics: dropping or
///    closing this handle delivers stdin EOF, which the native CPU miner
///    treats as an orderly shutdown. This stream is NOT the native-audit
///    exchange; a test on this stream cannot prove anything about the
///    wallet-audit write half.
///
/// 2. The native source-audit exchange runs over the wallet instance's
///    private Unix request stream (`spc.native-job-gate/1` bound socket,
///    source-feed frames). THAT stream must stay open for the whole audit
///    interaction - `Proof`, then the wallet's `Consumed` ack, then the
///    miner's `Committed` frame, all on one connection - including while an
///    `add_connection` carrying `source_audit` has returned `running` and the
///    audit completes asynchronously. The real-binary proof for that
///    transport is the agent's joint test (`spc-agent`
///    `wallet::native_audit::joint_test`, which drives the corrected CPU
///    binary 7b62234f and the actual Unix socket through the full
///    Proof/Consumed/Committed exchange on one open stream).
///
/// See `miner_control_stream_eof_is_orderly_teardown` in the tests below for
/// what the control-stream test does and does not cover.
pub struct MinerControl {
    miner_id: MinerId,
    incarnation: String,
    stdin: Mutex<ChildStdin>,
    pending: Arc<Mutex<BTreeMap<String, Sender<MinerWireResponse>>>>,
    events: Arc<Mutex<Receiver<MinerWireEvent>>>,
    timeout: Duration,
}

impl MinerControl {
    pub fn spawn(
        miner_id: MinerId,
        incarnation: String,
        mut stdin: ChildStdin,
        stdout: ChildStdout,
        log_path: PathBuf,
    ) -> Result<Self, String> {
        stdin
            .write_all(b"")
            .map_err(|e| format!("miner stdin is not writable: {e}"))?;
        let pending: Arc<Mutex<BTreeMap<String, Sender<MinerWireResponse>>>> =
            Arc::new(Mutex::new(BTreeMap::new()));
        let pending_reader = Arc::clone(&pending);
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        let reader_miner = miner_id.clone();
        std::thread::Builder::new()
            .name(format!("miner-{miner_id}-reader"))
            .spawn(move || {
                let log = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&log_path)
                    .ok();
                let mut log = log;
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    if let Some(file) = log.as_mut() {
                        let _ = writeln!(file, "{line}");
                    }
                    let Ok(value) = serde_json::from_str::<Value>(&line) else {
                        let _ = event_tx.send(parse_error_event(
                            &reader_miner,
                            "invalid JSON from miner stdout",
                            &line,
                        ));
                        continue;
                    };
                    match value.get("kind").and_then(|value| value.as_str()) {
                        Some("command_result") => {
                            let Ok(response) = serde_json::from_value::<MinerWireResponse>(value)
                            else {
                                let _ = event_tx.send(parse_error_event(
                                    &reader_miner,
                                    "invalid command_result envelope",
                                    &line,
                                ));
                                continue;
                            };
                            if let Some(sender) = pending_reader
                                .lock()
                                .unwrap()
                                .remove(response.command_id.as_str())
                            {
                                let _ = sender.send(response);
                            }
                        }
                        Some("event") => match serde_json::from_value::<MinerWireEvent>(value) {
                            Ok(event) => {
                                let _ = event_tx.send(event);
                            }
                            Err(error) => {
                                let _ = event_tx.send(parse_error_event(
                                    &reader_miner,
                                    &format!("invalid event envelope: {error}"),
                                    &line,
                                ));
                            }
                        },
                        _ => {
                            let _ = event_tx.send(parse_error_event(
                                &reader_miner,
                                "unknown miner stdout record kind",
                                &line,
                            ));
                        }
                    }
                }
                // Closing the reader drops the event sender and times out every
                // pending request; the process is treated as stale, not
                // successful.
            })
            .map_err(|e| format!("miner reader thread: {e}"))?;
        Ok(Self {
            miner_id,
            incarnation,
            stdin: Mutex::new(stdin),
            pending,
            events: Arc::new(Mutex::new(event_rx)),
            timeout: Duration::from_secs(10),
        })
    }

    pub fn miner_id(&self) -> &MinerId {
        &self.miner_id
    }

    pub fn incarnation(&self) -> &str {
        &self.incarnation
    }

    pub fn event_receiver(&self) -> Arc<Mutex<Receiver<MinerWireEvent>>> {
        Arc::clone(&self.events)
    }

    pub fn request(
        &self,
        operation: MinerOperation,
        payload: Value,
    ) -> Result<MinerWireResponse, String> {
        self.request_traced(operation, payload, |_, _| Ok(()))
    }

    /// Callback runs under the pipe write mutex immediately before dispatch.
    /// A durable evidence failure prevents every byte of the write.
    pub fn request_traced(
        &self,
        operation: MinerOperation,
        payload: Value,
        before: impl FnOnce(&CommandId, &[u8]) -> Result<(), String>,
    ) -> Result<MinerWireResponse, String> {
        let command_id = CommandId::generate();
        let wire = MinerWireCommand {
            protocol: "spc-cpu-miner-control".to_string(),
            version: 1,
            command_id: command_id.clone(),
            operation,
            payload,
        };
        let mut line = serde_json::to_vec(&wire).map_err(|e| e.to_string())?;
        line.push(b'\n');
        let (tx, rx) = std::sync::mpsc::channel();
        self.pending
            .lock()
            .unwrap()
            .insert(command_id.as_str().to_string(), tx);
        {
            let mut stdin = self.stdin.lock().unwrap();
            if let Err(error) = before(&command_id, &line) {
                self.pending.lock().unwrap().remove(command_id.as_str());
                return Err(error);
            }
            stdin
                .write_all(&line)
                .and_then(|_| stdin.flush())
                .map_err(|e| format!("miner control write: {e}"))?;
        }
        let response = rx
            .recv_timeout(self.timeout)
            .map_err(|_| format!("miner control response timed out for {command_id}"))?;
        if response.command_id != command_id {
            return Err(format!(
                "miner response id {} does not match request {command_id}",
                response.command_id
            ));
        }
        if response.protocol != "spc-cpu-miner-control" || response.version != 1 {
            return Err(format!(
                "miner response protocol/version mismatch: {}/{}",
                response.protocol, response.version
            ));
        }
        if response.kind != "command_result" {
            return Err(format!("miner response kind is {:?}", response.kind));
        }
        Ok(response)
    }

    pub fn stop(&self) -> Result<MinerWireResponse, String> {
        self.request(MinerOperation::Stop, serde_json::json!({}))
    }
}

fn parse_error_event(miner_id: &MinerId, error: &str, line: &str) -> MinerWireEvent {
    MinerWireEvent {
        protocol: "spc-cpu-miner-events".to_string(),
        version: 1,
        kind: "event".to_string(),
        source: "process".to_string(),
        event_id: 0,
        source_cursor: 0,
        miner_id: miner_id.clone(),
        occurred_at_wall_ms: 0,
        occurred_at_mono_ms: 0,
        dropped_before: 1,
        event: MinerEventBody {
            name: "driver_parse_error".to_string(),
            payload: serde_json::json!({"error": error, "line": line}),
        },
    }
}

fn http_post(
    endpoint: &Endpoint,
    auth: Option<&str>,
    content_type: &str,
    body: &[u8],
) -> Result<Vec<u8>, String> {
    let addr: SocketAddr = endpoint
        .socket_addr()
        .map_err(|e| format!("RPC endpoint: {e}"))?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(3))
        .map_err(|e| format!("RPC connect: {e}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;
    let mut request = format!(
        "POST / HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n",
        endpoint.display(),
        body.len()
    );
    if let Some(auth) = auth {
        request.push_str(&format!("Authorization: {auth}\r\n"));
    }
    request.push_str("\r\n");
    stream
        .write_all(request.as_bytes())
        .and_then(|_| stream.write_all(body))
        .map_err(|e| format!("RPC write: {e}"))?;
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .map_err(|e| format!("RPC read: {e}"))?;
    let split = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| "RPC response has no header terminator".to_string())?;
    let headers = String::from_utf8_lossy(&response[..split]);
    let body = &response[split + 4..];
    if headers
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        return decode_chunked(body);
    }
    Ok(body.to_vec())
}

fn decode_chunked(mut body: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    loop {
        let Some(line_end) = body.windows(2).position(|w| w == b"\r\n") else {
            return Err("chunked body has no size line".to_string());
        };
        let size_text = std::str::from_utf8(&body[..line_end]).map_err(|e| e.to_string())?;
        let size = usize::from_str_radix(size_text.trim(), 16).map_err(|e| e.to_string())?;
        body = &body[line_end + 2..];
        if size == 0 {
            return Ok(out);
        }
        if body.len() < size + 2 {
            return Err("chunked body is truncated".to_string());
        }
        out.extend_from_slice(&body[..size]);
        body = &body[size + 2..];
    }
}

fn base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let value = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((value >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((value >> 12) & 0x3f) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((value >> 6) & 0x3f) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(value & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod miner_control_stream_tests {
    use super::*;
    use std::os::unix::fs::DirBuilderExt;
    use std::process::{Command, Stdio};

    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let path = PathBuf::from("/tmp/opencode").join(format!(
                "miner-control-stream-test-{}",
                CommandId::generate()
            ));
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&path)
                .unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A protocol peer that delays its FIRST snapshot response (simulating a
    /// pending native source audit) and records stdin EOF in a marker file.
    const PEER: &str = r#"
import json, sys, time
marker = sys.argv[1]
delayed = False
while True:
    line = sys.stdin.readline()
    if not line:
        break
    line = line.strip()
    if not line:
        continue
    message = json.loads(line)
    if message.get("operation") == "snapshot" and not delayed:
        delayed = True
        time.sleep(3.0)
    sys.stdout.write(json.dumps({
        "protocol": "spc-cpu-miner-control",
        "version": 1,
        "kind": "command_result",
        "command_id": message["command_id"],
        "outcome": "succeeded",
        "result": {},
    }) + "\n")
    sys.stdout.flush()
with open(marker, "w") as f:
    f.write("EOF-OBSERVED\n")
"#;

    /// Bounded control-transport check (NOT the native-audit seam): while a
    /// control request is outstanding on the JSONL stdin stream, further
    /// requests are written and answered on the same stream, and dropping
    /// the control delivers stdin EOF, which the peer observes as an
    /// orderly shutdown.
    ///
    /// Scope: this proves the CONTROL stream's multiplexing and
    /// EOF-to-orderly-shutdown semantics only. The native source-audit
    /// request write half - the wallet instance's private Unix stream
    /// staying open across the full Proof/Consumed/Committed exchange,
    /// including while an audit is awaited - is exercised on its actual
    /// transport by the agent's joint test (`spc-agent`
    /// `wallet::native_audit::joint_test`, real CPU binary and real Unix
    /// socket), not here.
    #[test]
    fn miner_control_stream_eof_is_orderly_teardown() {
        let dir = Temp::new();
        let marker = dir.0.join("eof.marker");
        let log = dir.0.join("peer.log");
        let mut peer = Command::new("/usr/bin/python3")
            .arg("-c")
            .arg(PEER)
            .arg(&marker)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("python3 peer");
        let control = MinerControl::spawn(
            MinerId::trusted("m-stream-half-open"),
            "stream-test".into(),
            peer.stdin.take().unwrap(),
            peer.stdout.take().unwrap(),
            log,
        )
        .expect("miner control");
        let control = Arc::new(control);

        // The first snapshot stays outstanding for ~3 s (pending audit
        // window) on its own thread.
        let awaiting = Arc::clone(&control);
        let outstanding = std::thread::spawn(move || {
            awaiting
                .request(MinerOperation::Snapshot, Value::Null)
                .expect("delayed request is answered while the write side stays open")
        });

        // While that request is awaited, the same stream accepts and answers
        // a second request: no EOF, no broken pipe, no teardown.
        std::thread::sleep(Duration::from_millis(300));
        let concurrent = control
            .request(MinerOperation::Snapshot, Value::Null)
            .expect("stream accepts requests while another is awaited");
        assert!(concurrent.is_success());

        let delayed = outstanding.join().expect("awaiting thread");
        assert!(delayed.is_success());
        assert_ne!(delayed.command_id, concurrent.command_id);

        // Intentional teardown: dropping the control drops the stdin write
        // half, the peer observes EOF, records it and exits orderly. The
        // EOF-to-shutdown semantics remain intact - never weakened.
        Arc::try_unwrap(control)
            .map(|control| drop(control))
            .ok()
            .expect("sole control handle");
        let mut waited = 0;
        while !marker.exists() && waited < 5_000 {
            std::thread::sleep(Duration::from_millis(100));
            waited += 100;
        }
        assert!(
            marker.exists(),
            "peer did not observe the intentional stdin EOF within 5 s"
        );
        let status = peer.wait().expect("peer exit");
        assert!(status.success(), "peer exits orderly after EOF: {status}");
    }
}

