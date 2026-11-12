//! Native process supervisor with run-token ownership.
//!
//! A PID alone is never sufficient to stop a process. The persisted record
//! carries the PID start-time token, executable identity and run token; a
//! mismatch makes the process `orphan_unverified` rather than a kill target.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::resource::Endpoint;

pub const RUN_TOKEN_ENV: &str = "SPC_LAB_RUN_TOKEN";

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Readiness {
    #[default]
    Immediate,
    Tcp {
        host: String,
        port: u16,
    },
    File {
        path: PathBuf,
    },
}

/// The identity of the process that owns a namespace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamespaceOwner {
    /// The PID of the holder process.
    pub pid: u32,
    /// The holder's `/proc/<pid>/stat` field 22 start time token.
    pub start_time_ticks: String,
}

#[derive(Debug, Clone)]
pub struct ProcessSpec {
    pub role: String,
    pub entity: String,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub clear_env: bool,
    pub cwd: Option<PathBuf>,
    pub log_path: PathBuf,
    pub readiness: Readiness,
    pub ready_timeout_ms: u64,
    pub pipe_stdin: bool,
    pub pipe_stdout: bool,
    pub control: ControlTransport,
    /// The network namespace the process must run in.
    ///
    /// A topology actor enters both `user_namespace` and this namespace with
    /// one `nsenter --user=... --net=... --preserve-credentials` invocation.
    /// The launcher execs the declared helper/program in the same PID, so the
    /// supervisor retains the real child handle and ordinary owned stdio.
    pub namespace: Option<PathBuf>,
    /// The broker user namespace that owns `namespace`.
    pub user_namespace: Option<PathBuf>,
    /// The `nsenter` binary used to join the namespaces.
    pub nsenter: Option<PathBuf>,
    /// The owner of the namespace the process is placed in.
    ///
    /// The namespace is addressed by the PID of its holder.  A recycled
    /// holder PID would point at an unrelated namespace, so the holder's
    /// start time token is verified before the launch.
    pub namespace_owner: Option<NamespaceOwner>,
    /// Identity of the broker process that owns `user_namespace`.
    pub user_namespace_owner: Option<NamespaceOwner>,
    /// Safe exec helper that sets PR_SET_PDEATHSIG in the child before exec.
    /// No pre-exec callback runs in the controller process.
    pub lifetime_helper: Option<PathBuf>,
    /// A helper binary that probes a TCP endpoint.
    ///
    /// A namespaced listener only exists inside its namespace, so the probe
    /// has to run there as well.  The helper is the `spc-lab` binary with the
    /// `probe-tcp` subcommand; the supervisor runs it through `nsenter`.
    pub readiness_helper: Option<PathBuf>,
}

impl ProcessSpec {
    pub fn new(
        role: impl Into<String>,
        entity: impl Into<String>,
        program: impl Into<PathBuf>,
        log_path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            role: role.into(),
            entity: entity.into(),
            program: program.into(),
            args: Vec::new(),
            env: BTreeMap::new(),
            clear_env: false,
            cwd: None,
            log_path: log_path.into(),
            readiness: Readiness::Immediate,
            ready_timeout_ms: 10_000,
            pipe_stdin: false,
            pipe_stdout: false,
            control: ControlTransport::None,
            namespace: None,
            user_namespace: None,
            nsenter: None,
            namespace_owner: None,
            user_namespace_owner: None,
            lifetime_helper: None,
            readiness_helper: None,
        }
    }

    /// Place the process in `netns` (a published namespace path).
    pub fn in_namespace(&mut self, netns: impl Into<PathBuf>, nsenter: impl Into<PathBuf>) {
        self.namespace = Some(netns.into());
        self.nsenter = Some(nsenter.into());
    }

    /// Place a topology actor in the broker user namespace and its endpoint
    /// network namespace with one launch.
    pub fn in_namespaces(
        &mut self,
        user_namespace: impl Into<PathBuf>,
        netns: impl Into<PathBuf>,
        nsenter: impl Into<PathBuf>,
    ) {
        self.user_namespace = Some(user_namespace.into());
        self.in_namespace(netns, nsenter);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessState {
    Starting,
    Running,
    Adopted,
    AdoptedControlLost,
    Exited,
    Stopped,
    OrphanUnverified,
}

impl ProcessState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Adopted => "adopted",
            Self::AdoptedControlLost => "adopted_control_lost",
            Self::Exited => "exited",
            Self::Stopped => "stopped",
            Self::OrphanUnverified => "orphan_unverified",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlTransport {
    #[default]
    None,
    /// stdin/stdout JSONL. The pipe cannot be reopened after the supervisor
    /// dies; a live process is adopted but its control is stale.
    Pipe,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessRecord {
    pub role: String,
    pub entity: String,
    pub pid: u32,
    pub pgid: u32,
    pub start_time_ticks: u64,
    pub executable_path: PathBuf,
    pub executable_sha256: String,
    pub run_token: String,
    pub args: Vec<String>,
    pub log_path: PathBuf,
    pub readiness: Readiness,
    pub state: ProcessState,
    pub adopted: bool,
    #[serde(default)]
    pub control: ControlTransport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ownership {
    Owned,
    Missing,
    PidReused,
    RunTokenMismatch,
    ExecutableMismatch { expected: String, actual: String },
}

impl Ownership {
    pub fn is_owned(&self) -> bool {
        matches!(self, Self::Owned)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SupervisorError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("process {role}/{entity} exited during readiness with {status}")]
    ExitedDuringReadiness {
        role: String,
        entity: String,
        status: String,
    },
    #[error("process {role}/{entity} was not ready before the deadline")]
    ReadinessTimeout { role: String, entity: String },
    #[error("process {role}/{entity} is not owned: {ownership:?}")]
    NotOwned {
        role: String,
        entity: String,
        ownership: Ownership,
    },
    #[error("process record {0}/{1} is unknown")]
    Unknown(String, String),
    #[error("adapter program {0} is an executable wrapper script; refusing to launch an unverified exec target")]
    UnsupportedWrapper(String),
    #[error("a namespace was requested for {0} but no namespace launcher was configured")]
    NamespaceLauncherMissing(String),
    #[error("process {role}/{entity} did not exec {expected} before the deadline")]
    ExecHandshakeTimeout {
        role: String,
        entity: String,
        expected: String,
    },
    #[error("the namespace owner {pid} is gone or was replaced")]
    NamespaceOwnerChanged { pid: u32 },
    #[error("the launch of {role}/{entity} failed: {status}")]
    LaunchFailed {
        role: String,
        entity: String,
        status: String,
    },
    #[error("process {role}/{entity} listener ownership could not be verified")]
    ForeignListener { role: String, entity: String },
}

#[derive(Debug)]
pub struct Launched {
    pub record: ProcessRecord,
    pub stdin: Option<std::process::ChildStdin>,
    pub stdout: Option<std::process::ChildStdout>,
}

pub struct Supervisor {
    records: BTreeMap<String, ProcessRecord>,
    children: BTreeMap<u32, Child>,
    run_token: String,
}

impl Supervisor {
    pub fn new(run_token: impl Into<String>) -> Self {
        Self {
            records: BTreeMap::new(),
            children: BTreeMap::new(),
            run_token: run_token.into(),
        }
    }

    pub fn run_token(&self) -> &str {
        &self.run_token
    }

    pub fn records(&self) -> Vec<ProcessRecord> {
        self.records.values().cloned().collect()
    }

    /// Load persisted records verbatim for cleanup (`reap`). No
    /// verification and no adoption happen here: the census is exactly what
    /// the manifest recorded, so processes that already exited - or node
    /// records that were never written - are still reported honestly by
    /// `reap_owned` and the engine's expected-node diff.
    pub fn load_census(&mut self, records: Vec<ProcessRecord>) {
        self.records = records
            .into_iter()
            .map(|record| (key(&record.role, &record.entity), record))
            .collect();
    }

    /// Observe the exit of an owned startup helper without losing its status.
    pub fn try_wait_child(
        &mut self,
        pid: u32,
    ) -> std::io::Result<Option<std::process::ExitStatus>> {
        self.children
            .get_mut(&pid)
            .ok_or_else(|| std::io::Error::other("owned child handle unavailable"))?
            .try_wait()
    }

    pub fn record(&self, role: &str, entity: &str) -> Option<&ProcessRecord> {
        self.records.get(&key(role, entity))
    }

    pub fn launch(&mut self, spec: ProcessSpec) -> Result<ProcessRecord, SupervisorError> {
        self.launch_with_pipes(spec).map(|launched| launched.record)
    }

    pub fn launch_with_pipes(&mut self, spec: ProcessSpec) -> Result<Launched, SupervisorError> {
        if is_wrapper_script(&spec.program)? {
            return Err(SupervisorError::UnsupportedWrapper(
                spec.program.display().to_string(),
            ));
        }
        if let Some(parent) = spec.log_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&spec.log_path)?;
        let _ = std::fs::set_permissions(&spec.log_path, std::fs::Permissions::from_mode(0o600));
        let mut launch_argv: Vec<PathBuf> = match (&spec.namespace, &spec.nsenter) {
            (Some(netns), Some(nsenter)) => {
                let mut argv = vec![nsenter.clone()];
                if let Some(userns) = &spec.user_namespace {
                    argv.push(PathBuf::from(format!("--user={}", userns.display())));
                }
                argv.push(PathBuf::from(format!("--net={}", netns.display())));
                argv.push(PathBuf::from("--preserve-credentials"));
                argv.push(PathBuf::from("--"));
                argv.push(spec.program.clone());
                argv
            }
            (Some(_), None) => {
                return Err(SupervisorError::NamespaceLauncherMissing(
                    spec.program.display().to_string(),
                ));
            }
            (None, _) => vec![spec.program.clone()],
        };
        if launch_argv.len() > 1 && spec.user_namespace.is_some() && spec.namespace.is_none() {
            return Err(SupervisorError::NamespaceLauncherMissing(
                spec.program.display().to_string(),
            ));
        }
        if let Some(helper) = &spec.lifetime_helper {
            let parent_pid = std::process::id();
            let parent_start = process_start_time(parent_pid).ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "the controller start-time token is unavailable",
                )
            })?;
            let mut wrapped = vec![
                helper.clone(),
                PathBuf::from("actor-exec"),
                PathBuf::from("--parent-pid"),
                PathBuf::from(parent_pid.to_string()),
                PathBuf::from("--parent-start-ticks"),
                PathBuf::from(parent_start.to_string()),
                PathBuf::from("--"),
            ];
            wrapped.append(&mut launch_argv);
            launch_argv = wrapped;
        }
        let mut launch_iter = launch_argv.iter();
        let launch_program = launch_iter.next().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "empty actor launch")
        })?;
        let mut command = Command::new(launch_program);
        if spec.clear_env {
            command.env_clear();
        }
        command.args(launch_iter);
        command
            .args(&spec.args)
            .env(RUN_TOKEN_ENV, &self.run_token)
            .envs(&spec.env)
            .stdin(if spec.pipe_stdin {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(if spec.pipe_stdout {
                Stdio::piped()
            } else {
                Stdio::from(log.try_clone()?)
            })
            .stderr(Stdio::from(log));
        if let Some(cwd) = &spec.cwd {
            command.current_dir(cwd);
        }
        // Own process group so a graceful stop can reach helpers without
        // signalling an unrelated PID.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        for owner in [&spec.namespace_owner, &spec.user_namespace_owner]
            .into_iter()
            .flatten()
        {
            if !namespace_owner_is_current(owner) {
                return Err(SupervisorError::NamespaceOwnerChanged { pid: owner.pid });
            }
        }
        let mut child = command.spawn()?;
        let pid = child.id();
        let pgid = pid;
        let start_time_ticks = process_start_time(pid).unwrap_or(0);
        // A namespaced launch goes through a launcher that execs the program.
        // Wait for that exec handshake so the recorded executable identity is
        // the actor and never the launcher.
        let executable_path = if spec.namespace.is_some() || spec.lifetime_helper.is_some() {
            match wait_for_exec_handshake(pid, &spec.program, spec.ready_timeout_ms, &mut child) {
                Ok(path) => path,
                Err(HandshakeFailure::Exited(status)) => {
                    let _ = child.wait();
                    let _ = self.children.remove(&pid);
                    return Err(SupervisorError::LaunchFailed {
                        role: spec.role,
                        entity: spec.entity,
                        status,
                    });
                }
                Err(HandshakeFailure::Timeout) => {
                    // The child handle is still ours: kill the launch that
                    // never became the declared program and reap it, so it does
                    // not stay a zombie.
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = self.children.remove(&pid);
                    return Err(SupervisorError::ExecHandshakeTimeout {
                        role: spec.role,
                        entity: spec.entity,
                        expected: spec.program.display().to_string(),
                    });
                }
            }
        } else {
            process_executable(pid).unwrap_or_else(|| spec.program.clone())
        };
        let executable_sha256 = hash_file(&executable_path).unwrap_or_default();
        let record = ProcessRecord {
            role: spec.role.clone(),
            entity: spec.entity.clone(),
            pid,
            pgid,
            start_time_ticks,
            executable_path,
            executable_sha256,
            run_token: self.run_token.clone(),
            args: spec.args.clone(),
            log_path: spec.log_path.clone(),
            readiness: spec.readiness.clone(),
            state: ProcessState::Starting,
            adopted: false,
            control: spec.control,
        };
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        self.children.insert(pid, child);

        let deadline = Instant::now() + Duration::from_millis(spec.ready_timeout_ms);
        loop {
            if let Some(child) = self.children.get_mut(&pid) {
                if let Some(status) = child.try_wait()? {
                    self.children.remove(&pid);
                    return Err(SupervisorError::ExitedDuringReadiness {
                        role: spec.role,
                        entity: spec.entity,
                        status: status.to_string(),
                    });
                }
            }
            if readiness_satisfied(
                pid,
                &spec.readiness,
                spec.namespace.as_deref(),
                spec.user_namespace.as_deref(),
                spec.nsenter.as_deref(),
                spec.readiness_helper.as_deref(),
            ) {
                break;
            }
            if Instant::now() >= deadline {
                let _ = self.signal_verified(&record, "KILL");
                if let Some(mut child) = self.children.remove(&pid) {
                    let _ = child.wait();
                }
                return Err(SupervisorError::ReadinessTimeout {
                    role: spec.role,
                    entity: spec.entity,
                });
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let mut record = record;
        record.state = ProcessState::Running;
        self.records
            .insert(key(&record.role, &record.entity), record.clone());
        Ok(Launched {
            record,
            stdin,
            stdout,
        })
    }

    /// Verify that a persisted record still identifies the live process.
    pub fn verify(&self, record: &ProcessRecord) -> Ownership {
        if !Path::new(&format!("/proc/{}", record.pid)).exists() {
            return Ownership::Missing;
        }
        // A zombie is our exited child (or an exited process awaiting its
        // parent). It has no readable environ/exe but is never an unrelated
        // PID to be killed.
        if process_state_char(record.pid) == Some('Z') {
            return Ownership::Missing;
        }
        let Some(start) = process_start_time(record.pid) else {
            return Ownership::Missing;
        };
        if start != record.start_time_ticks {
            return Ownership::PidReused;
        }
        match process_run_token(record.pid) {
            Some(token) if token == record.run_token => {}
            // redis-server deliberately clears its environment and rewrites
            // its process title after startup, which removes the run token and
            // makes the recorded argv unusable. For that role only, the
            // kernel start-time tick, executable path and executable hash below
            // remain the identity checks. Every other role still requires the
            // run token exactly.
            None if record.role == "redis" => {}
            _ => return Ownership::RunTokenMismatch,
        }
        let actual_path = process_executable(record.pid).unwrap_or_default();
        if actual_path != record.executable_path {
            return Ownership::ExecutableMismatch {
                expected: record.executable_path.display().to_string(),
                actual: actual_path.display().to_string(),
            };
        }
        let actual_hash = hash_file(&actual_path).unwrap_or_default();
        if !record.executable_sha256.is_empty()
            && !actual_hash.is_empty()
            && record.executable_sha256 != actual_hash
        {
            return Ownership::ExecutableMismatch {
                expected: record.executable_sha256.clone(),
                actual: actual_hash,
            };
        }
        Ownership::Owned
    }

    pub fn adopt(&mut self, mut record: ProcessRecord) -> Result<ProcessRecord, SupervisorError> {
        match self.verify(&record) {
            Ownership::Owned => {
                record.state = ProcessState::Adopted;
                record.adopted = true;
                self.records
                    .insert(key(&record.role, &record.entity), record.clone());
                Ok(record)
            }
            Ownership::Missing => {
                record.state = ProcessState::Exited;
                self.records
                    .insert(key(&record.role, &record.entity), record.clone());
                Err(SupervisorError::NotOwned {
                    role: record.role,
                    entity: record.entity,
                    ownership: Ownership::Missing,
                })
            }
            ownership => {
                record.state = ProcessState::OrphanUnverified;
                self.records
                    .insert(key(&record.role, &record.entity), record.clone());
                Err(SupervisorError::NotOwned {
                    role: record.role,
                    entity: record.entity,
                    ownership,
                })
            }
        }
    }

    pub fn set_state(&mut self, role: &str, entity: &str, state: ProcessState) {
        self.mark_state(role, entity, state);
    }

    /// Stop a process only after verifying its identity. Returns `Ok(false)` if
    /// the record was missing; returns `NotOwned` without signalling otherwise.
    pub fn stop(
        &mut self,
        role: &str,
        entity: &str,
        deadline: Duration,
    ) -> Result<bool, SupervisorError> {
        let Some(record) = self.records.get(&key(role, entity)).cloned() else {
            return Ok(false);
        };
        if matches!(record.state, ProcessState::Exited | ProcessState::Stopped) {
            return Ok(false);
        }
        if self.process_gone(record.pid) {
            self.mark_state(role, entity, ProcessState::Exited);
            return Ok(false);
        }
        match self.verify(&record) {
            Ownership::Owned => {}
            Ownership::Missing => {
                self.mark_state(role, entity, ProcessState::Exited);
                return Ok(false);
            }
            ownership => {
                self.mark_state(role, entity, ProcessState::OrphanUnverified);
                return Err(SupervisorError::NotOwned {
                    role: role.to_string(),
                    entity: entity.to_string(),
                    ownership,
                });
            }
        }

        self.signal_verified(&record, "TERM")?;
        let wait_until = Instant::now() + deadline;
        loop {
            if self.process_gone(record.pid) {
                self.mark_state(role, entity, ProcessState::Stopped);
                return Ok(true);
            }
            if Instant::now() >= wait_until {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        // Re-verify immediately before the force kill. A recycled PID must not
        // receive this signal.
        match self.verify(&record) {
            Ownership::Owned => {
                self.signal_verified(&record, "KILL")?;
            }
            _ => {
                self.mark_state(role, entity, ProcessState::OrphanUnverified);
                return Err(SupervisorError::NotOwned {
                    role: role.to_string(),
                    entity: entity.to_string(),
                    ownership: self.verify(&record),
                });
            }
        }
        let force_until = Instant::now() + Duration::from_millis(500);
        while Instant::now() < force_until {
            if self.process_gone(record.pid) {
                self.mark_state(role, entity, ProcessState::Stopped);
                return Ok(true);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        self.mark_state(role, entity, ProcessState::OrphanUnverified);
        Err(SupervisorError::NotOwned {
            role: role.to_string(),
            entity: entity.to_string(),
            ownership: Ownership::Missing,
        })
    }

    pub fn stop_all(&mut self, deadline: Duration) {
        let records = self.records();
        for record in records {
            let _ = self.stop(&record.role, &record.entity, deadline);
        }
        self.reap_children();
    }

    /// Stop only records verified as ours; mark everything else for operator
    /// review. Used by an explicit `reap` command, never by accidental drop.
    pub fn reap_owned(&mut self, deadline: Duration) -> Vec<(String, String, ProcessState)> {
        let mut results = Vec::new();
        let records = self.records();
        for record in records {
            let result = self.stop(&record.role, &record.entity, deadline);
            let state = match result {
                Ok(true) => ProcessState::Stopped,
                Ok(false) => ProcessState::Exited,
                Err(SupervisorError::NotOwned { .. }) => ProcessState::OrphanUnverified,
                Err(_) => ProcessState::OrphanUnverified,
            };
            results.push((record.role, record.entity, state));
        }
        self.reap_children();
        results
    }

    pub fn reap_children(&mut self) {
        let pids: Vec<u32> = self.children.keys().copied().collect();
        for pid in pids {
            if let Some(child) = self.children.get_mut(&pid) {
                let _ = child.try_wait();
                if process_state_char(pid).is_none() {
                    self.children.remove(&pid);
                }
            }
        }
    }

    fn process_gone(&mut self, pid: u32) -> bool {
        if let Some(child) = self.children.get_mut(&pid) {
            match child.try_wait() {
                Ok(Some(_)) | Err(_) => return true,
                Ok(None) => {}
            }
        }
        match process_state_char(pid) {
            None => true,
            Some('Z') => true,
            Some(_) => false,
        }
    }

    fn signal_verified(&self, record: &ProcessRecord, signal: &str) -> Result<(), SupervisorError> {
        match self.verify(record) {
            Ownership::Owned => {}
            Ownership::Missing => return Ok(()),
            ownership => {
                return Err(SupervisorError::NotOwned {
                    role: record.role.clone(),
                    entity: record.entity.clone(),
                    ownership,
                });
            }
        }
        let signal = match signal {
            "TERM" => rustix::process::Signal::TERM,
            "KILL" => rustix::process::Signal::KILL,
            other => {
                return Err(SupervisorError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("unsupported signal {other}"),
                )));
            }
        };
        let pgid = i32::try_from(record.pgid).map_err(|_| {
            SupervisorError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "process group does not fit pid_t",
            ))
        })?;
        let pid = rustix::process::Pid::from_raw(pgid).ok_or_else(|| {
            SupervisorError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "process group zero is not a target",
            ))
        })?;
        match rustix::process::kill_process_group(pid, signal) {
            Ok(()) => Ok(()),
            Err(error) if error == rustix::io::Errno::SRCH => Ok(()),
            Err(error) => Err(SupervisorError::Io(std::io::Error::from(error))),
        }
    }

    fn mark_state(&mut self, role: &str, entity: &str, state: ProcessState) {
        if let Some(record) = self.records.get_mut(&key(role, entity)) {
            record.state = state;
        }
    }
}

fn key(role: &str, entity: &str) -> String {
    format!("{role}\u{1f}{entity}")
}

/// Wait until the launched process really is `expected`.
///
/// The namespace launcher execs the program, so `/proc/<pid>/exe` changes from
/// the launcher to the program.  Waiting for that transition is the exec
/// handshake: it removes the race between the spawn and the identity
/// capture, and it refuses a launch that never became the declared program.
/// Why the exec handshake did not complete.
enum HandshakeFailure {
    /// The launch exited before it became the declared program.  The status
    /// is the launcher's own error, for example a namespace the caller may
    /// not enter.
    Exited(String),
    /// The launch is still the wrong program after the deadline.
    Timeout,
}

fn wait_for_exec_handshake(
    pid: u32,
    expected: &Path,
    timeout_ms: u64,
    child: &mut std::process::Child,
) -> Result<PathBuf, HandshakeFailure> {
    // The program may be reached through a symlink, so both sides are
    // canonicalised before they are compared.
    let expected = std::fs::canonicalize(expected).unwrap_or_else(|_| expected.to_path_buf());
    let deadline = Instant::now() + Duration::from_millis(timeout_ms.max(1_000));
    loop {
        if let Some(path) = process_executable(pid) {
            let path = std::fs::canonicalize(&path).unwrap_or(path);
            if path == expected {
                return Ok(path);
            }
        }
        // A launch that failed (a namespace the caller may not enter, a
        // missing binary) exits at once: the status is reported instead of a
        // timeout that hides the reason.
        match child.try_wait() {
            Ok(Some(status)) => return Err(HandshakeFailure::Exited(status.to_string())),
            Ok(None) => {}
            Err(error) => return Err(HandshakeFailure::Exited(error.to_string())),
        }
        if Instant::now() >= deadline {
            return Err(HandshakeFailure::Timeout);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// True when the namespace holder is still the process that owns it.
fn namespace_owner_is_current(owner: &NamespaceOwner) -> bool {
    match process_start_time(owner.pid) {
        Some(start) => start.to_string() == owner.start_time_ticks,
        None => false,
    }
}

fn readiness_satisfied(
    pid: u32,
    readiness: &Readiness,
    namespace: Option<&Path>,
    user_namespace: Option<&Path>,
    nsenter: Option<&Path>,
    helper: Option<&Path>,
) -> bool {
    match readiness {
        Readiness::Immediate => true,
        Readiness::File { path } => path.exists(),
        Readiness::Tcp { host, port } => {
            // A listener inside a namespace only exists there, so the probe
            // has to run inside the namespace too.  The ownership check stays
            // here: it reads the child's own file descriptors.
            let connected = match (namespace, nsenter, helper) {
                (Some(netns), Some(nsenter), Some(helper)) => {
                    // The helper runs inside the namespace: it connects and it
                    // proves that the listener belongs to this process, using
                    // the namespace's own socket table.
                    namespace_tcp_probe(nsenter, user_namespace, netns, helper, host, *port, pid)
                }
                _ => Endpoint {
                    host: host.clone(),
                    port: *port,
                }
                .socket_addr()
                .ok()
                .and_then(|addr| {
                    std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(100)).ok()
                })
                .is_some(),
            };
            // For a namespaced actor the helper already proved the
            // ownership inside the namespace; for a host actor the check is
            // done here.
            let owned = match (namespace, nsenter, helper) {
                (Some(_), Some(_), Some(_)) => true,
                _ => listener_owned_by(pid, *port),
            };
            connected && owned
        }
    }
}

/// Probe a TCP endpoint from inside a namespace.
///
/// The helper is a short lived process: it either connects or it does not,
/// and it never touches anything but the endpoint it was given.
fn namespace_tcp_probe(
    nsenter: &Path,
    user_namespace: Option<&Path>,
    netns: &Path,
    helper: &Path,
    host: &str,
    port: u16,
    pid: u32,
) -> bool {
    let mut command = std::process::Command::new(nsenter);
    if let Some(userns) = user_namespace {
        command.arg(format!("--user={}", userns.display()));
    }
    let output = command
        .arg(format!("--net={}", netns.display()))
        .arg("--preserve-credentials")
        .arg("--")
        .arg(helper)
        .arg("probe-tcp")
        .arg("--host")
        .arg(host)
        .arg("--port")
        .arg(port.to_string())
        .arg("--pid")
        .arg(pid.to_string())
        .stdin(std::process::Stdio::null())
        .output();
    matches!(output, Ok(result) if result.status.success())
}

/// A `#!` wrapper can fork/exec and change its own `/proc/pid/exe` after this
/// record is captured, making a safe stop impossible. Refuse it before launch
/// rather than creating an unkillable process.
fn is_wrapper_script(path: &Path) -> std::io::Result<bool> {
    use std::io::Read as _;
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    let mut head = [0u8; 2];
    let read = file.read(&mut head)?;
    Ok(read == 2 && &head == b"#!")
}

/// Verify a LISTEN socket port belongs to a file descriptor of the child.
/// This rejects a readiness probe that connected to an unrelated listener.
fn listener_owned_by(pid: u32, port: u16) -> bool {
    let Ok(inode) = listener_inode(port) else {
        return false;
    };
    let fd_dir = format!("/proc/{pid}/fd");
    let Ok(entries) = std::fs::read_dir(fd_dir) else {
        return false;
    };
    let needle = format!("socket:[{inode}]");
    entries.flatten().any(|entry| {
        std::fs::read_link(entry.path())
            .map(|target| target.to_string_lossy() == needle)
            .unwrap_or(false)
    })
}

fn listener_inode(port: u16) -> Result<u64, ()> {
    let local = format!(":{port:04X}");
    for path in ["/proc/net/tcp", "/proc/net/tcp6"] {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        for line in text.lines().skip(1) {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() < 10 {
                continue;
            }
            if fields[1].ends_with(&local) && fields[3] == "0A" {
                if let Ok(inode) = fields[9].parse::<u64>() {
                    return Ok(inode);
                }
            }
        }
    }
    Err(())
}

pub fn pid_is_alive(pid: u32) -> bool {
    match process_state_char(pid) {
        Some('Z') | None => false,
        Some(_) => true,
    }
}

pub fn process_state_char(pid: u32) -> Option<char> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let close = stat.rfind(')')?;
    stat[close + 1..].split_whitespace().next()?.chars().next()
}

pub fn process_start_time(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let close = stat.rfind(')')?;
    // Fields after the comm field start at field 3 (state). starttime is field
    // 22, so it is the 20th token after the closing parenthesis.
    stat[close + 1..]
        .split_whitespace()
        .nth(19)?
        .parse::<u64>()
        .ok()
}

pub fn process_run_token(pid: u32) -> Option<String> {
    let mut bytes = Vec::new();
    File::open(format!("/proc/{pid}/environ"))
        .ok()?
        .read_to_end(&mut bytes)
        .ok()?;
    for entry in bytes.split(|b| *b == 0) {
        if let Some(rest) = entry.strip_prefix(format!("{RUN_TOKEN_ENV}=").as_bytes()) {
            return String::from_utf8(rest.to_vec()).ok();
        }
    }
    None
}

pub fn process_executable(pid: u32) -> Option<PathBuf> {
    let path = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    let text = path.to_string_lossy();
    let cleaned = text.strip_suffix(" (deleted)").unwrap_or(&text);
    Some(PathBuf::from(cleaned))
}

pub fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}
