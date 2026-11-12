mod common;

use std::collections::BTreeMap;
use std::time::Duration;

use spc_lab::supervisor::{pid_is_alive, Ownership, ProcessSpec, Readiness, Supervisor};

fn spec(dir: &common::TestDir, entity: &str) -> ProcessSpec {
    let ready = dir.join(&format!("ready-{entity}"));
    let mut spec = ProcessSpec::new(
        "fixture",
        entity,
        common::fixture_bin(),
        dir.join(&format!("{entity}.log")),
    );
    spec.args = vec![
        "hold".to_string(),
        "--ready-file".to_string(),
        ready.display().to_string(),
    ];
    spec.readiness = Readiness::File { path: ready };
    spec.ready_timeout_ms = 3_000;
    spec.env = BTreeMap::new();
    spec
}

#[test]
fn a_matching_run_token_is_adopted_and_stopped_safely() {
    let dir = common::TestDir::new("adopt-owned");
    let token = format!("token-{}", std::process::id());
    let mut first = Supervisor::new(&token);
    let launched = first.launch(spec(&dir, "one")).unwrap();
    assert_eq!(first.verify(&launched), Ownership::Owned);
    assert!(pid_is_alive(launched.pid));
    drop(first); // a crash leaves the process for adoption

    let mut second = Supervisor::new(&token);
    let adopted = second.adopt(launched.clone()).unwrap();
    assert!(adopted.adopted);
    assert_eq!(second.verify(&adopted), Ownership::Owned);
    assert!(pid_is_alive(adopted.pid));
    assert!(second
        .stop("fixture", "one", Duration::from_secs(2))
        .unwrap());
    assert!(common::wait_until(Duration::from_secs(2), || {
        !pid_is_alive(adopted.pid)
    }));
}

#[test]
fn pid_reuse_is_never_a_kill_target() {
    let dir = common::TestDir::new("adopt-pid-reuse");
    let token = format!("token-{}", std::process::id());
    let mut supervisor = Supervisor::new(&token);
    let launched = supervisor.launch(spec(&dir, "pid")).unwrap();
    let mut forged = launched.clone();
    forged.start_time_ticks = forged.start_time_ticks.wrapping_add(1);
    let mut adopter = Supervisor::new(&token);
    let error = adopter.adopt(forged.clone()).unwrap_err();
    assert!(error.to_string().contains("not owned"), "{error}");
    assert!(
        pid_is_alive(launched.pid),
        "the unrelated process is untouched"
    );
    let result = adopter.stop("fixture", "pid", Duration::from_millis(100));
    assert!(result.is_err(), "stop must refuse an unverified record");
    assert!(pid_is_alive(launched.pid));

    // The real owner can still stop the process.
    let mut owner = Supervisor::new(&token);
    owner.adopt(launched.clone()).unwrap();
    owner
        .stop("fixture", "pid", Duration::from_secs(2))
        .unwrap();
    assert!(!pid_is_alive(launched.pid));
}

#[test]
fn run_token_or_executable_mismatch_is_unverified() {
    let dir = common::TestDir::new("adopt-mismatch");
    let token = format!("token-{}", std::process::id());
    let mut supervisor = Supervisor::new(&token);
    let launched = supervisor.launch(spec(&dir, "mismatch")).unwrap();

    let mut wrong_token = launched.clone();
    wrong_token.run_token = "another-run".to_string();
    assert_eq!(supervisor.verify(&wrong_token), Ownership::RunTokenMismatch);

    let mut wrong_executable = launched.clone();
    wrong_executable.executable_path = std::path::PathBuf::from("/bin/false");
    assert!(matches!(
        supervisor.verify(&wrong_executable),
        Ownership::ExecutableMismatch { .. }
    ));

    // Clean up with the real record rather than a forged one.
    supervisor
        .stop("fixture", "mismatch", Duration::from_secs(2))
        .unwrap();
    assert!(!pid_is_alive(launched.pid));
}

#[test]
fn missing_process_is_reported_not_signalled() {
    let dir = common::TestDir::new("adopt-missing");
    let mut supervisor = Supervisor::new("token");
    let launched = supervisor.launch(spec(&dir, "missing")).unwrap();
    supervisor
        .stop("fixture", "missing", Duration::from_secs(2))
        .unwrap();
    let ownership = supervisor.verify(&launched);
    assert!(
        !ownership.is_owned(),
        "a stopped PID must not remain owned, got {ownership:?}"
    );
    let mut adopter = Supervisor::new("token");
    let error = adopter.adopt(launched).unwrap_err();
    assert!(error.to_string().contains("not owned"));
}

#[test]
fn an_exited_own_child_is_reaped_and_never_marked_orphan_unverified() {
    let dir = common::TestDir::new("adopt-zombie");
    let mut supervisor = Supervisor::new("zombie-token");
    let mut spec = spec(&dir, "zombie");
    // Use the pipe miner so dropping stdin makes it exit like the real driver.
    spec.program = common::fixture_bin();
    spec.args = vec![
        "miner-jsonl".to_string(),
        "--miner-id".to_string(),
        "miner-zombie".to_string(),
    ];
    spec.pipe_stdin = true;
    spec.pipe_stdout = true;
    spec.control = spc_lab::supervisor::ControlTransport::Pipe;
    spec.readiness = Readiness::Immediate;
    let launched = supervisor.launch_with_pipes(spec).unwrap();
    drop(launched.stdin);
    assert!(common::wait_until(Duration::from_secs(2), || {
        spc_lab::supervisor::process_state_char(launched.record.pid) == Some('Z')
    }));
    let stopped = supervisor
        .stop("fixture", "zombie", Duration::from_secs(1))
        .unwrap();
    assert!(!stopped);
    assert_eq!(
        supervisor.record("fixture", "zombie").unwrap().state,
        spc_lab::supervisor::ProcessState::Exited
    );
    assert_ne!(
        supervisor.record("fixture", "zombie").unwrap().state,
        spc_lab::supervisor::ProcessState::OrphanUnverified
    );
}

#[test]
fn executable_wrapper_scripts_are_refused_before_launch() {
    use std::os::unix::fs::PermissionsExt;
    let dir = common::TestDir::new("adopt-wrapper");
    let script = dir.join("wrapper.sh");
    std::fs::write(&script, "#!/bin/sh\nexec /bin/sleep 60\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut supervisor = Supervisor::new("wrapper-token");
    let mut spec = spec(&dir, "wrapper");
    spec.program = script;
    spec.args.clear();
    spec.readiness = Readiness::Immediate;
    let error = supervisor.launch(spec).unwrap_err();
    assert!(error.to_string().contains("wrapper script"), "{error}");
}

#[test]
fn readiness_rejects_a_foreign_listener_on_the_same_port() {
    let dir = common::TestDir::new("adopt-foreign");
    let foreign = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = foreign.local_addr().unwrap().port();
    let mut supervisor = Supervisor::new("foreign-token");
    let mut spec = spec(&dir, "foreign");
    spec.args = vec!["hold".to_string()];
    spec.readiness = Readiness::Tcp {
        host: "127.0.0.1".to_string(),
        port,
    };
    spec.ready_timeout_ms = 300;
    let error = supervisor.launch(spec).unwrap_err();
    assert!(error.to_string().contains("not ready"), "{error}");
    drop(foreign);
}
