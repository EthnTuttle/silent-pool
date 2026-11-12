//! Namespace placement, address publication and the controller RPC.
//!
//! The LP-topology packet places the lab actors in network namespaces that the
//! approved LP-4a namespace broker owns.  This module is the Rust side of
//! that boundary:
//!
//! * it starts the broker (an approved LP-4a primitive) inside an ephemeral
//!   user namespace,
//! * it reads the broker's manifest, which is the *only* source of the
//!   placement (no generated configuration is parsed for placement),
//! * it publishes the addresses and the control/data plane links,
//! * it drives the broker over its Unix socket JSON RPC (the controller RPC).
//!
//! The module is deliberately self contained so it can be merged into the
//! runtime after the runtime's own freeze without depending on the line
//! positions of `runtime.rs` or `engine.rs`.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Errors of the namespace boundary.
#[derive(Debug, thiserror::Error)]
pub enum NamespaceError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("broker: {0}")]
    Broker(String),
    #[error("broker rejected {op} ({code}): {detail}")]
    Rejected {
        op: String,
        code: String,
        detail: String,
    },
    #[error("the namespace broker did not become ready: {0}")]
    NotReady(String),
    #[error("the broker is not running: {0}")]
    Unavailable(String),
    #[error("unknown endpoint {0}")]
    UnknownEndpoint(String),
    #[error("the run directory {run_dir} is not empty: {entries:?}; use a fresh directory")]
    StaleRunDir {
        run_dir: String,
        entries: Vec<String>,
    },
    #[error("unknown link {0}")]
    UnknownLink(String),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

/// One address the broker published for an endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedAddress {
    pub link_id: String,
    pub addr: String,
    pub prefix: u8,
    pub iface: String,
    #[serde(default)]
    pub peer_addr: String,
    #[serde(default)]
    pub family: String,
}

impl PublishedAddress {
    pub fn socket_addr(&self, port: u16) -> String {
        format!("{}:{}", self.addr, port)
    }
}

/// One endpoint (one network namespace) as published by the broker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedEndpoint {
    pub name: String,
    pub role: String,
    pub namespace: PublishedNamespace,
    #[serde(default)]
    pub addresses: Vec<PublishedAddress>,
    #[serde(default)]
    pub ipv6_link_local: Vec<Value>,
    #[serde(default)]
    pub forwarding: Option<PublishedForwarding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedNamespace {
    pub holder_pid: u32,
    #[serde(default)]
    pub start_time_token: String,
    pub netns_path: String,
    #[serde(default)]
    pub alive: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedForwarding {
    pub ipv4: u32,
    pub ipv6: u32,
}

/// One link as published by the broker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedLink {
    pub id: String,
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub subnet: String,
    #[serde(default)]
    pub addresses: BTreeMap<String, String>,
    #[serde(default)]
    pub interfaces: BTreeMap<String, String>,
    #[serde(default)]
    pub nft_tables: BTreeMap<String, String>,
    #[serde(default)]
    pub policy: Value,
}

/// The broker's published manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrokerManifest {
    pub run_id: String,
    pub run_token: String,
    pub backend: String,
    #[serde(default)]
    pub endpoints: Vec<PublishedEndpoint>,
    #[serde(default)]
    pub links: Vec<PublishedLink>,
    #[serde(default)]
    pub guarantees: Vec<String>,
    #[serde(default)]
    pub unsupported_requests: Vec<Value>,
    #[serde(default)]
    pub limits: Value,
}

impl BrokerManifest {
    pub fn endpoint(&self, name: &str) -> Option<&PublishedEndpoint> {
        self.endpoints.iter().find(|item| item.name == name)
    }

    pub fn link(&self, id: &str) -> Option<&PublishedLink> {
        self.links.iter().find(|item| item.id == id)
    }

    /// The address of `endpoint` on `link`, if the broker published one.
    pub fn address(&self, endpoint: &str, link: &str) -> Option<&PublishedAddress> {
        let published = self.endpoint(endpoint)?;
        published.addresses.iter().find(|item| item.link_id == link)
    }
}

/// How the broker process is started.
#[derive(Debug, Clone)]
pub struct BrokerOptions {
    /// The LP-4a broker implementation (the approved primitive).
    pub broker_script: PathBuf,
    /// The topology the broker has to realise.
    pub spec_path: PathBuf,
    /// The run directory of the broker (it owns its manifest and socket).
    pub broker_run_dir: PathBuf,
    /// The run token every resource of the broker carries.
    pub run_token: String,
    /// The `unshare` binary that creates the broker's user + network namespace.
    pub unshare: PathBuf,
    /// `nsenter` is used to join the broker's user namespace.
    pub nsenter: PathBuf,
    /// Safe Rust helper that ties the broker to the controller's lifetime.
    /// Tests that embed the library may leave this unset; the real runtime
    /// always supplies its own `spc-lab` executable.
    pub lifetime_helper: Option<PathBuf>,
    /// The `python3` interpreter that runs the broker.
    pub python: PathBuf,
    /// The `ip` binary, used to enumerate the host addresses.
    pub ip: PathBuf,
    /// Host IPv4 addresses the broker must prove unreachable from the
    /// namespaces.
    pub host_addresses: Vec<String>,
    /// Host IPv6 addresses the broker must prove unreachable from the
    /// namespaces.
    pub host_addresses_v6: Vec<String>,
}

impl BrokerOptions {
    pub fn new(
        broker_script: impl Into<PathBuf>,
        spec_path: impl Into<PathBuf>,
        broker_run_dir: impl Into<PathBuf>,
        run_token: impl Into<String>,
    ) -> Self {
        Self {
            broker_script: broker_script.into(),
            spec_path: spec_path.into(),
            broker_run_dir: broker_run_dir.into(),
            run_token: run_token.into(),
            unshare: PathBuf::from("unshare"),
            nsenter: PathBuf::from("nsenter"),
            lifetime_helper: None,
            python: PathBuf::from("python3"),
            ip: PathBuf::from("ip"),
            host_addresses: Vec::new(),
            host_addresses_v6: Vec::new(),
        }
    }

    /// Resolve the tools from `PATH`.
    ///
    /// Nothing is hard coded to a host prefix: under Nix the tools live in
    /// the store and are on the `PATH` of the development shell.
    pub fn resolve_tools(mut self) -> Self {
        self.unshare = resolve_tool("unshare").unwrap_or(self.unshare);
        self.nsenter = resolve_tool("nsenter").unwrap_or(self.nsenter);
        self.python = resolve_tool("python3").unwrap_or(self.python);
        self.ip = resolve_tool("ip").unwrap_or(self.ip);
        self
    }

    /// The run token has to be the short hex form the broker derives its
    /// interface and table names from.
    pub fn validate(&self) -> Result<(), NamespaceError> {
        let token = self.run_token.as_str();
        if token.len() != 8 || !token.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(NamespaceError::Broker(format!(
                "the run token {token:?} is not eight hex characters"
            )));
        }
        Ok(())
    }
}

/// Execute an actor after installing a parent-death signal in this process.
///
/// This function runs in the new helper process, never in a controller
/// `pre_exec` callback. The parent identity is checked after PR_SET_PDEATHSIG
/// to close the race where the controller died before the signal was set.
pub fn actor_exec(parent_pid: u32, parent_start_ticks: u64, argv: &[String]) -> Result<(), String> {
    if argv.is_empty() {
        return Err("actor-exec requires a program after `--`".to_string());
    }
    rustix::process::set_parent_process_death_signal(Some(rustix::process::Signal::KILL))
        .map_err(|error| format!("set actor parent-death signal: {error}"))?;
    if crate::supervisor::process_start_time(parent_pid) != Some(parent_start_ticks)
        || std::os::unix::process::parent_id() != parent_pid
    {
        return Err(format!(
            "actor parent {parent_pid} exited or was replaced before exec"
        ));
    }
    use std::os::unix::process::CommandExt;
    let error = Command::new(&argv[0]).args(&argv[1..]).exec();
    Err(format!("exec actor {}: {error}", argv[0]))
}

/// Parse a JSON-RPC params value. Invalid JSON and non-array values are not
/// silently replaced with `[]`.
pub fn parse_rpc_params(text: &str) -> Result<Value, String> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| format!("--params must be valid JSON: {error}"))?;
    if !value.is_array() {
        return Err("--params must be a JSON array".to_string());
    }
    Ok(value)
}

/// Read a private HTTP Basic cookie for the short-lived helper. The cookie's
/// contents are returned only to the helper process and never placed in argv
/// or an environment variable. Parsing does not echo credentials in errors.
pub fn basic_auth_file(path: &Path) -> Result<(String, String), String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("read auth file {}: {error}", path.display()))?;
    if !metadata.file_type().is_file() {
        return Err(format!(
            "auth file {} is not a regular file",
            path.display()
        ));
    }
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(format!(
            "auth file {} must not be readable by group/other",
            path.display()
        ));
    }
    const MAX_AUTH_BYTES: u64 = 2_048;
    if metadata.len() > MAX_AUTH_BYTES {
        return Err(format!("auth file {} is too large", path.display()));
    }
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("read auth file {}: {error}", path.display()))?;
    if text.is_empty() || text.contains(['\n', '\r', '\0']) {
        return Err(format!("auth file {} is malformed", path.display()));
    }
    let (user, password) = text
        .split_once(':')
        .ok_or_else(|| format!("auth file {} must be username:password", path.display()))?;
    if user.is_empty()
        || password.is_empty()
        || user.chars().any(|c| c.is_whitespace() || c == ':')
        || password.chars().any(char::is_whitespace)
    {
        return Err(format!("auth file {} is malformed", path.display()));
    }
    Ok((user.to_string(), password.to_string()))
}

/// Resolve a tool from `PATH`.
pub fn resolve_tool(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
}

/// A live namespace broker.
#[derive(Debug)]
pub struct NamespaceBroker {
    options: BrokerOptions,
    socket_path: PathBuf,
    manifest: BrokerManifest,
    child: Option<Child>,
}

impl NamespaceBroker {
    pub fn exec_rpc_typed(
        &self,
        endpoint: &str,
        url: &str,
        request_id: u64,
        method: &str,
        params: &Value,
        auth_file: &Path,
    ) -> crate::traffic::rpc::RpcOutcome {
        use crate::traffic::rpc::{RpcOutcome, UnknownClass};
        let Ok(helper) = Self::helper_binary() else {
            return RpcOutcome::unknown(UnknownClass::Broker);
        };
        let argv = vec![
            helper.display().to_string(),
            "rpc".into(),
            "--url".into(),
            url.into(),
            "--method".into(),
            method.into(),
            "--params".into(),
            params.to_string(),
            "--auth-file".into(),
            auth_file.display().to_string(),
            "--typed".into(),
            "--request-id".into(),
            request_id.to_string(),
        ];
        let response = match self.traffic_broker_request(
            json!({"op":"exec","endpoint":endpoint,"argv":argv,"timeout_s":10.0}),
        ) {
            Ok(response) => response,
            Err(class) => return RpcOutcome::unknown(class),
        };
        if response.get("command_exit_code").and_then(Value::as_i64) != Some(0) {
            return RpcOutcome::unknown(UnknownClass::Transport);
        }
        let Some(stdout) = response.get("stdout").and_then(Value::as_str) else {
            return RpcOutcome::unknown(UnknownClass::Shape);
        };
        if stdout.len() > spc_lab_txd::control::MAX_LINE {
            return RpcOutcome::unknown(UnknownClass::Shape);
        }
        serde_json::from_str(stdout).unwrap_or_else(|_| RpcOutcome::unknown(UnknownClass::Protocol))
    }

    /// Traffic-specific bounded transport. Legacy broker limits and timeout
    /// behavior remain unchanged; this path preserves typed transport failures.
    fn traffic_broker_request(
        &self,
        request: Value,
    ) -> Result<Value, crate::traffic::rpc::UnknownClass> {
        use crate::traffic::rpc::UnknownClass;
        let deadline = Instant::now() + Duration::from_secs(12);
        let classify = |error: std::io::Error| match error.kind() {
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => UnknownClass::Timeout,
            _ => UnknownClass::Broker,
        };
        let mut stream = UnixStream::connect(&self.socket_path).map_err(classify)?;
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .map_err(classify)?;
        let mut bytes = serde_json::to_vec(&request).map_err(|_| UnknownClass::Shape)?;
        bytes.push(b'\n');
        if bytes.len() > spc_lab_txd::control::MAX_BODY + 16 * 1024 {
            return Err(UnknownClass::Shape);
        }
        stream.write_all(&bytes).map_err(classify)?;
        let mut response = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .filter(|d| !d.is_zero())
                .ok_or(UnknownClass::Timeout)?;
            stream.set_read_timeout(Some(remaining)).map_err(classify)?;
            let n = stream.read(&mut chunk).map_err(classify)?;
            if n == 0 {
                return Err(UnknownClass::Broker);
            }
            if response.len() + n > spc_lab_txd::control::MAX_BODY + 16 * 1024 {
                return Err(UnknownClass::Shape);
            }
            if let Some(end) = chunk[..n].iter().position(|b| *b == b'\n') {
                if end + 1 != n {
                    return Err(UnknownClass::Protocol);
                }
                response.extend_from_slice(&chunk[..end]);
                break;
            }
            response.extend_from_slice(&chunk[..n]);
        }
        let response =
            spc_lab_txd::control::strict_json(&response).map_err(|_| UnknownClass::Protocol)?;
        if response.get("error").is_some() {
            return Err(UnknownClass::Broker);
        }
        Ok(response)
    }

    /// Start the broker and wait for its published manifest.
    ///
    /// The broker runs inside `unshare -U --map-root-user -rn`: its own user
    /// namespace owns its own network namespace, so it can create veth pairs
    /// and nftables rules without any host privilege, and every namespace it
    /// creates is owned by that user namespace.
    pub fn start(mut options: BrokerOptions) -> Result<Self, NamespaceError> {
        options.validate()?;
        options = options.resolve_tools();
        // A run directory belongs to exactly one broker.  A directory that is
        // not empty is either owned by a live broker or is the debris of a
        // crashed one; both are refused, because reusing it could make a
        // stale manifest look like a fresh start.
        if options.broker_run_dir.exists() {
            let mut entries = std::fs::read_dir(&options.broker_run_dir)
                .map_err(|error| {
                    NamespaceError::Broker(format!(
                        "the broker run directory cannot be read: {error}"
                    ))
                })?
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().to_string())
                .collect::<Vec<_>>();
            entries.sort();
            if !entries.is_empty() {
                return Err(NamespaceError::StaleRunDir {
                    run_dir: options.broker_run_dir.display().to_string(),
                    entries,
                });
            }
        }
        std::fs::create_dir_all(&options.broker_run_dir).map_err(|error| {
            NamespaceError::Broker(format!(
                "the broker run directory cannot be created: {error}"
            ))
        })?;
        let _ = std::fs::set_permissions(
            &options.broker_run_dir,
            std::fs::Permissions::from_mode(0o700),
        );
        if options.host_addresses.is_empty() && options.host_addresses_v6.is_empty() {
            let (v4, v6) = host_global_addresses().map_err(NamespaceError::Broker)?;
            options.host_addresses = v4;
            options.host_addresses_v6 = v6;
        }
        let mut command = if let Some(helper) = &options.lifetime_helper {
            let parent_pid = std::process::id();
            let parent_start =
                crate::supervisor::process_start_time(parent_pid).ok_or_else(|| {
                    NamespaceError::Broker(
                        "the controller start-time token is unavailable".to_string(),
                    )
                })?;
            let mut command = Command::new(helper);
            command
                .arg("actor-exec")
                .arg("--parent-pid")
                .arg(parent_pid.to_string())
                .arg("--parent-start-ticks")
                .arg(parent_start.to_string())
                .arg("--")
                .arg(&options.unshare);
            command
        } else {
            Command::new(&options.unshare)
        };
        command
            .arg("-U")
            .arg("--map-root-user")
            .arg("-r")
            .arg("-n")
            .arg("--")
            .arg(&options.python)
            .arg(&options.broker_script)
            .arg("--run-dir")
            .arg(&options.broker_run_dir)
            .arg("--spec")
            .arg(&options.spec_path)
            .arg("--run-token")
            .arg(&options.run_token);
        for address in &options.host_addresses {
            command.arg("--host-address").arg(address);
        }
        for address in &options.host_addresses_v6 {
            command.arg("--host-address-v6").arg(address);
        }
        // The broker's diagnostics are kept: a failed start must be
        // explainable, not silent.
        let broker_log = options.broker_run_dir.join("broker.log");
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&broker_log)
            .map_err(|error| {
                NamespaceError::Broker(format!("the broker log cannot be opened: {error}"))
            })?;
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(log));
        let mut child = command.spawn().map_err(|error| {
            NamespaceError::Broker(format!("the broker process cannot be started: {error}"))
        })?;
        let socket_path = options.broker_run_dir.join("control.sock");
        let manifest_path = options.broker_run_dir.join("manifest.json");
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if manifest_path.exists() && socket_path.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if !manifest_path.exists() {
            let _ = child.kill();
            let _ = child.wait();
            // A broker that refused to start records why.
            let error_path = options
                .broker_run_dir
                .join(format!("up-error-{}.json", options.run_token));
            if let Ok(payload) = std::fs::read_to_string(&error_path) {
                let detail = serde_json::from_str::<Value>(&payload)
                    .ok()
                    .and_then(|value| {
                        value
                            .get("error")
                            .and_then(|error| error.get("detail"))
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                    .unwrap_or_else(|| payload.trim().to_string());
                return Err(NamespaceError::Broker(detail));
            }
            let detail = std::fs::read_to_string(&broker_log).unwrap_or_default();
            return Err(NamespaceError::NotReady(format!(
                "the broker wrote no manifest at {}: {}",
                manifest_path.display(),
                detail.trim()
            )));
        }
        let raw = std::fs::read(&manifest_path).map_err(|error| {
            NamespaceError::Broker(format!("the broker manifest cannot be read: {error}"))
        })?;
        let manifest: BrokerManifest = serde_json::from_slice(&raw)?;
        // The manifest must be this run's.  A stale manifest from a crashed
        // broker is never accepted as a fresh start.
        if manifest.run_token != options.run_token {
            let _ = child.kill();
            let _ = child.wait();
            return Err(NamespaceError::Broker(format!(
                "the manifest in {} belongs to the run token {:?}, not {:?}",
                options.broker_run_dir.display(),
                manifest.run_token,
                options.run_token
            )));
        }
        let broker = Self {
            options,
            socket_path,
            manifest,
            child: Some(child),
        };
        // The broker has to answer before the start is a success.
        let ping = broker.request(json!({"op": "ping"}))?;
        let token = ping
            .get("token")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if token != broker.options.run_token {
            return Err(NamespaceError::Broker(format!(
                "the broker answered with the run token {token:?}, not {:?}",
                broker.options.run_token
            )));
        }
        Ok(broker)
    }

    pub fn manifest(&self) -> &BrokerManifest {
        &self.manifest
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// The run directory the broker owns.
    pub fn run_dir(&self) -> &Path {
        &self.options.broker_run_dir
    }

    /// The absolute path of a tool the broker uses.
    pub fn tool_path(&self, name: &str) -> Option<PathBuf> {
        match name {
            "unshare" => Some(self.options.unshare.clone()),
            "nsenter" => Some(self.options.nsenter.clone()),
            _ => None,
        }
    }

    pub fn run_token(&self) -> &str {
        &self.options.run_token
    }

    /// The broker's process id, if it is still ours.
    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().map(|child| child.id())
    }

    /// Send one JSON request to the broker and return its JSON response.
    pub fn request(&self, request: Value) -> Result<Value, NamespaceError> {
        let mut stream = UnixStream::connect(&self.socket_path).map_err(|error| {
            NamespaceError::Unavailable(format!(
                "the namespace broker is not reachable at {}: {error}",
                self.socket_path.display()
            ))
        })?;
        stream.set_read_timeout(Some(Duration::from_secs(120)))?;
        stream.set_write_timeout(Some(Duration::from_secs(30)))?;
        let mut line = serde_json::to_string(&request)?;
        line.push('\n');
        stream.write_all(line.as_bytes())?;
        stream.flush()?;
        // The response is bounded and it is one line: the broker answers with
        // a single JSON line and keeps the connection open, so the read has to
        // stop at the newline instead of waiting for an end of file.
        let reader = BufReader::new(stream);
        let mut limited = reader.take(MAX_RESPONSE_BYTES as u64 + 1);
        let mut response = String::new();
        limited.read_line(&mut response)?;
        if response.len() > MAX_RESPONSE_BYTES {
            return Err(NamespaceError::Broker(
                "the broker response exceeds the response limit".to_string(),
            ));
        }
        if response.trim().is_empty() {
            return Err(NamespaceError::Unavailable(
                "the namespace broker closed the control socket".to_string(),
            ));
        }
        let value: Value = serde_json::from_str(&response)?;
        // A response that carries an `error` object is a rejected request.
        // A response without one is a served request: its `ok` field is the
        // outcome of the request (an `exec` reports the command's own exit
        // status, and the caller has to look at it).
        if let Some(error) = value.get("error") {
            let detail = error
                .get("detail")
                .and_then(Value::as_str)
                .unwrap_or("unknown broker error")
                .to_string();
            let code = error
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            let op = request
                .get("op")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            return Err(NamespaceError::Rejected { op, code, detail });
        }
        Ok(value)
    }

    /// Install a link policy (partition, latency or loss).
    pub fn apply(&self, link: &str, policy: Value) -> Result<Value, NamespaceError> {
        self.request(json!({"op": "apply", "link": link, "policy": policy}))
    }

    /// Remove a link fault.
    pub fn heal(&self, link: &str) -> Result<Value, NamespaceError> {
        self.request(json!({"op": "heal", "link": link}))
    }

    /// The raw per-link counters of the broker.
    pub fn counters(&self, link: &str) -> Result<Value, NamespaceError> {
        self.request(json!({"op": "counters", "link": link}))
    }

    /// The broker's no-bypass proof for the current topology.
    pub fn no_bypass_check(&self) -> Result<Value, NamespaceError> {
        self.request(json!({"op": "no-bypass-check"}))
    }

    /// Run a command inside an endpoint namespace and return its output.
    pub fn exec(
        &self,
        endpoint: &str,
        argv: &[String],
        timeout_s: f64,
    ) -> Result<Value, NamespaceError> {
        self.request(json!({
            "op": "exec",
            "endpoint": endpoint,
            "argv": argv,
            "timeout_s": timeout_s,
        }))
    }

    /// The broker's view of the topology, as the controller sees it.
    pub fn status(&self) -> Result<Value, NamespaceError> {
        self.request(json!({"op": "status"}))
    }

    /// Stop the broker and release its namespaces.
    pub fn stop(&mut self) -> Result<(), NamespaceError> {
        if self.socket_path.exists() {
            let _ = self.request(json!({"op": "down"}));
        }
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        Ok(())
    }
}

impl Drop for NamespaceBroker {
    fn drop(&mut self) {
        // A graceful stop first: the broker releases its namespaces and its
        // control socket.  Only then is the process reaped.
        if self.socket_path.exists() {
            let _ = self.request(json!({"op": "down"}));
        }
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// The host's global addresses.
///
/// The broker proves that no namespace can reach the host, so it needs the
/// host addresses.  They are read with the `ip` binary: no host state is
/// changed, only read.
pub fn host_global_addresses() -> Result<(Vec<String>, Vec<String>), String> {
    let ip = resolve_tool("ip").ok_or_else(|| {
        "the `ip` binary is not on PATH, so the host addresses cannot be enumerated".to_string()
    })?;
    let mut v4 = Vec::new();
    let mut v6 = Vec::new();
    for family in ["-4", "-6"] {
        let output = Command::new(&ip)
            .args([family, "-o", "addr", "show", "scope", "global"])
            .output()
            .map_err(|error| format!("the host address enumeration failed: {error}"))?;
        if !output.status.success() {
            // A failed enumeration is an error, not an empty list: an empty
            // list would make the host unreachability check pass vacuously.
            return Err(format!(
                "`ip {} -o addr show scope global` failed: {}",
                family,
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            // `ip -o addr show` prints: "<index>: <ifname> <family> <address>/<prefix> ..."
            let mut address = None;
            let mut fields = line.split_whitespace().skip(2);
            while let Some(field) = fields.next() {
                if field == "inet" || field == "inet6" {
                    address = fields
                        .next()
                        .map(|value| value.split('/').next().unwrap_or_default().to_string());
                    break;
                }
            }
            if let Some(address) = address {
                if address.is_empty() {
                    continue;
                }
                if family == "-4" {
                    v4.push(address);
                } else {
                    v6.push(address);
                }
            }
        }
    }
    Ok((v4, v6))
}

/// Minimal base64 for the HTTP basic authorization header.
fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((triple >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((triple >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[((triple >> 6) & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(triple & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// One plain HTTP request.  Only `http://` is supported, which is all the lab
/// uses: the node RPC and the front API are namespace local loopback
/// endpoints.
pub fn http_request(
    url: &str,
    method: &str,
    body: Option<&str>,
    user: Option<&str>,
    pass: Option<&str>,
) -> Result<String, String> {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| format!("only http:// urls are supported: {url}"))?;
    let (authority, path) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, "/"),
    };
    let mut stream = TcpStream::connect(authority).map_err(|error| error.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .map_err(|error| error.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(30)))
        .map_err(|error| error.to_string())?;
    let mut request =
        format!("{method} {path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n");
    if let (Some(user), Some(pass)) = (user, pass) {
        let token = base64_encode(format!("{user}:{pass}").as_bytes());
        request.push_str(&format!("Authorization: Basic {token}\r\n"));
    }
    match body {
        Some(body) => {
            request.push_str("Content-Type: application/json\r\n");
            request.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
            request.push_str(body);
        }
        None => request.push_str("\r\n"),
    }
    stream
        .write_all(request.as_bytes())
        .map_err(|error| error.to_string())?;
    stream.flush().map_err(|error| error.to_string())?;
    const MAX_HTTP_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;
    const MAX_HTTP_HEAD_BYTES: usize = 64 * 1024;
    let mut raw = Vec::new();
    stream
        .take(MAX_HTTP_RESPONSE_BYTES + 1)
        .read_to_end(&mut raw)
        .map_err(|error| error.to_string())?;
    if raw.len() as u64 > MAX_HTTP_RESPONSE_BYTES {
        return Err("the HTTP response exceeds 8 MiB".to_string());
    }
    let text = String::from_utf8_lossy(&raw).to_string();
    let (head, body) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| "the HTTP response has no complete header/body".to_string())?;
    if head.len() > MAX_HTTP_HEAD_BYTES {
        return Err("the HTTP response header is too large".to_string());
    }
    let status = head.lines().next().unwrap_or_default();
    let mut fields = status.split_whitespace();
    let version = fields.next().unwrap_or_default();
    let code = fields.next().unwrap_or_default();
    if !matches!(version, "HTTP/1.0" | "HTTP/1.1")
        || code.len() != 3
        || !code.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(format!("malformed HTTP status line {status}"));
    }
    let code = code
        .parse::<u16>()
        .map_err(|error| format!("malformed HTTP status code {code}: {error}"))?;
    if !(200..=299).contains(&code) {
        return Err(format!("the HTTP response was {status}: {body}"));
    }
    Ok(body.to_string())
}

/// The inode of the listening socket of `port` in *this* namespace.
///
/// `/proc/net/tcp` is bound to the network namespace of the reading process,
/// so this has to run inside the namespace that owns the listener.
fn listener_inode(port: u16) -> Option<u64> {
    let wanted = format!(":{:04X}", port);
    for table in ["/proc/net/tcp", "/proc/net/tcp6"] {
        let Ok(text) = std::fs::read_to_string(table) else {
            continue;
        };
        for line in text.lines().skip(1) {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() < 10 || !fields[1].ends_with(&wanted) || fields[3] != "0A"
            // TCP_LISTEN
            {
                continue;
            }
            if let Ok(inode) = fields[9].parse::<u64>() {
                return Some(inode);
            }
        }
    }
    None
}

/// True when the listening socket of `port` belongs to `pid`.
///
/// This is the ownership proof of the supervisor: the listener that answered
/// the readiness probe has to be a file descriptor of the process the
/// supervisor started.
pub fn listener_owned_by(pid: u32, port: u16) -> bool {
    let Some(inode) = listener_inode(port) else {
        return false;
    };
    let Ok(entries) = std::fs::read_dir(format!("/proc/{pid}/fd")) else {
        return false;
    };
    let needle = format!("socket:[{inode}]");
    entries.flatten().any(|entry| {
        std::fs::read_link(entry.path())
            .map(|target| target.to_string_lossy() == needle)
            .unwrap_or(false)
    })
}

/// Probe a TCP endpoint: `Ok(true)` when a connection was accepted.
pub fn probe_tcp(host: &str, port: u16, timeout_ms: u64) -> Result<bool, String> {
    use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
    use std::time::Duration;

    let target: SocketAddr = format!("{host}:{port}")
        .to_socket_addrs()
        .map_err(|error| error.to_string())?
        .next()
        .ok_or_else(|| format!("{host}:{port} does not resolve"))?;
    Ok(TcpStream::connect_timeout(&target, Duration::from_millis(timeout_ms)).is_ok())
}

impl NamespaceBroker {
    /// The path of the `spc-lab` binary that runs the namespace local helpers.
    pub fn helper_binary() -> Result<PathBuf, NamespaceError> {
        std::env::current_exe().map_err(|error| {
            NamespaceError::Broker(format!("the helper binary path is unknown: {error}"))
        })
    }

    /// One JSON-RPC call to a namespace local endpoint.
    ///
    /// The call is executed by the `spc-lab rpc` helper **inside** the
    /// endpoint namespace, because the endpoint's listener only exists there.
    pub fn exec_rpc(
        &self,
        endpoint: &str,
        url: &str,
        method: &str,
        params: &Value,
        auth_file: Option<&Path>,
    ) -> Result<Value, NamespaceError> {
        let helper = Self::helper_binary()?;
        let mut argv = vec![
            helper.display().to_string(),
            "rpc".to_string(),
            "--url".to_string(),
            url.to_string(),
            "--method".to_string(),
            method.to_string(),
            "--params".to_string(),
            params.to_string(),
        ];
        if let Some(path) = auth_file {
            argv.push("--auth-file".to_string());
            argv.push(path.display().to_string());
        }
        let response = self.exec(endpoint, &argv, 30.0)?;
        if response.get("command_exit_code").and_then(Value::as_i64) != Some(0) {
            return Err(NamespaceError::Broker(format!(
                "the namespace RPC helper failed: {}",
                response
                    .get("stderr")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
            )));
        }
        let stdout = response
            .get("stdout")
            .and_then(Value::as_str)
            .unwrap_or_default();
        serde_json::from_str(stdout.trim()).map_err(|error| {
            NamespaceError::Broker(format!("the namespace RPC returned invalid JSON: {error}"))
        })
    }

    /// One HTTP request to a namespace local endpoint.
    pub fn exec_http(
        &self,
        endpoint: &str,
        url: &str,
        method: &str,
        body: Option<&str>,
        user: Option<&str>,
        pass: Option<&str>,
    ) -> Result<String, NamespaceError> {
        let helper = Self::helper_binary()?;
        let mut argv = vec![
            helper.display().to_string(),
            "http".to_string(),
            "--url".to_string(),
            url.to_string(),
            "--method".to_string(),
            method.to_string(),
        ];
        if let Some(body) = body {
            argv.push("--body".to_string());
            argv.push(body.to_string());
        }
        if let Some(user) = user {
            argv.push("--user".to_string());
            argv.push(user.to_string());
        }
        if let Some(pass) = pass {
            argv.push("--pass".to_string());
            argv.push(pass.to_string());
        }
        let response = self.exec(endpoint, &argv, 30.0)?;
        if response.get("command_exit_code").and_then(Value::as_i64) != Some(0) {
            return Err(NamespaceError::Broker(format!(
                "the namespace HTTP request failed: {}",
                response
                    .get("stderr")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
            )));
        }
        Ok(response
            .get("stdout")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string())
    }
}

/// The largest broker response the client accepts.
pub const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

/// The published placement of the actors of a run.
///
/// The placement is derived from the broker's manifest only.  A generated
/// configuration is never parsed to learn an address.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Placement {
    /// Endpoint name -> the namespace it lives in.
    #[serde(default)]
    pub namespaces: BTreeMap<String, String>,
    /// Endpoint name -> the addresses the broker published for it.
    #[serde(default)]
    pub addresses: BTreeMap<String, Vec<PublishedAddress>>,
    /// Link id -> the endpoints it connects.
    #[serde(default)]
    pub links: BTreeMap<String, (String, String)>,
    /// Link id -> the per-side addresses.
    #[serde(default)]
    pub link_addresses: BTreeMap<String, BTreeMap<String, String>>,
}

impl Placement {
    pub fn from_manifest(manifest: &BrokerManifest) -> Self {
        let mut placement = Placement::default();
        for endpoint in &manifest.endpoints {
            placement
                .namespaces
                .insert(endpoint.name.clone(), endpoint.namespace.netns_path.clone());
            placement
                .addresses
                .insert(endpoint.name.clone(), endpoint.addresses.clone());
        }
        for link in &manifest.links {
            placement
                .links
                .insert(link.id.clone(), (link.from.clone(), link.to.clone()));
            placement
                .link_addresses
                .insert(link.id.clone(), link.addresses.clone());
        }
        placement
    }

    pub fn namespace(&self, endpoint: &str) -> Option<&str> {
        self.namespaces.get(endpoint).map(String::as_str)
    }

    pub fn address(&self, endpoint: &str, link: &str) -> Option<&PublishedAddress> {
        self.addresses
            .get(endpoint)?
            .iter()
            .find(|item| item.link_id == link)
    }

    /// The shareable projection of the placement: addresses and links, no
    /// identity material and no secrets.
    pub fn shareable(&self) -> Value {
        json!({
            "namespaces": self.namespaces,
            "addresses": self.addresses,
            "links": self.links,
            "link_addresses": self.link_addresses,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listener_ownership_reads_the_real_kernel_socket_columns() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(listener_inode(port).is_some());
        assert!(listener_owned_by(std::process::id(), port));
        assert!(probe_tcp("127.0.0.1", port, 250).unwrap());
        drop(listener);
        assert!(listener_inode(port).is_none());
        assert!(!listener_owned_by(std::process::id(), port));
    }

    #[test]
    fn private_cookie_parsing_and_rpc_params_fail_closed() {
        assert!(parse_rpc_params("[]").is_ok());
        assert!(parse_rpc_params(r#"[1, {"x": 2}]"#).is_ok());
        for invalid in ["not-json", "null", "{}", "1"] {
            assert!(parse_rpc_params(invalid).is_err(), "{invalid} was accepted");
        }
        let dir = std::env::temp_dir().join(format!(
            "LP-final-auth-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let cookie = dir.join("cookie");
        std::fs::write(&cookie, b"spc:correct-secret").unwrap();
        std::fs::set_permissions(&cookie, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            basic_auth_file(&cookie).unwrap(),
            ("spc".to_string(), "correct-secret".to_string())
        );
        assert_eq!(
            base64_encode(b"spc:correct-secret"),
            "c3BjOmNvcnJlY3Qtc2VjcmV0"
        );
        std::fs::set_permissions(&cookie, std::fs::Permissions::from_mode(0o644)).unwrap();
        let error = basic_auth_file(&cookie).unwrap_err();
        assert!(
            !error.contains("correct-secret"),
            "error leaked cookie: {error}"
        );
        std::fs::set_permissions(&cookie, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::write(&cookie, b"spc:bad password").unwrap();
        assert!(basic_auth_file(&cookie).is_err());
        assert!(basic_auth_file(&dir.join("missing-cookie")).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn http_status_is_exact_2xx_not_a_substring() {
        fn serve(response: &'static str) -> String {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            std::thread::spawn(move || {
                if let Ok((mut stream, _)) = listener.accept() {
                    use std::io::{Read, Write};
                    let mut request = [0u8; 4096];
                    let _ = stream.read(&mut request);
                    let _ = stream.write_all(format!("{response}\r\n\r\n").as_bytes());
                    let _ = stream.flush();
                }
            });
            address.to_string()
        }
        for accepted in ["HTTP/1.1 200 OK", "HTTP/1.1 204 No Content"] {
            let url = format!("http://{}/", serve(accepted));
            assert_eq!(http_request(&url, "GET", None, None, None).unwrap(), "");
        }
        for rejected in [
            "HTTP/1.1 2001 OK",
            "HTTP/1.1 199 Weird",
            "HTTP/1.1 302 Found",
            "HTTP/1.1 404 Not Found",
        ] {
            let url = format!("http://{}/", serve(rejected));
            assert!(
                http_request(&url, "GET", None, None, None).is_err(),
                "{rejected}"
            );
        }
    }
}
