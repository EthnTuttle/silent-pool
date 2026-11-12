mod common;

use std::collections::BTreeMap;
use std::time::Duration;

use spc_lab::adapters::{MinerControl, MinerOperation};
use spc_lab::ids::MinerId;
use spc_lab::supervisor::{ControlTransport, ProcessSpec, Readiness, Supervisor};

/// Real LP-2 driver proof against the published `spc-cpu-miner-control`
/// version 1 protocol. This is a control-only process (`connections = []`) and
/// does not claim mining, SV2 acceptance or accounting.
#[test]
#[ignore = "requires SPC_LAB_CPU_MINER and SPC_LAB_CPU_MINER_CONFIG"]
fn real_cpu_miner_control_and_stop() {
    let program = std::env::var("SPC_LAB_CPU_MINER")
        .expect("set SPC_LAB_CPU_MINER to the spc-cpu-miner binary");
    let config = std::env::var("SPC_LAB_CPU_MINER_CONFIG")
        .expect("set SPC_LAB_CPU_MINER_CONFIG to a control-only miner.toml");

    let check = std::process::Command::new(&program)
        .arg("--config")
        .arg(&config)
        .arg("--check-config")
        .status()
        .expect("run --check-config");
    assert!(check.success(), "spc-cpu-miner --check-config failed");

    eprintln!(
        "proof program={} sha256={}",
        program,
        spc_lab::supervisor::hash_file(std::path::Path::new(&program)).unwrap_or_default()
    );
    let dir = common::TestDir::new("real-cpu-miner");
    let mut supervisor = Supervisor::new("real-cpu-miner-proof");
    let mut spec = ProcessSpec::new("miner", "miner-real", &program, dir.join("miner.log"));
    spec.args = vec!["--config".to_string(), config];
    spec.pipe_stdin = true;
    spec.pipe_stdout = true;
    spec.control = ControlTransport::Pipe;
    spec.readiness = Readiness::Immediate;
    spec.ready_timeout_ms = 5_000;
    spec.env = BTreeMap::new();
    let launched = supervisor.launch_with_pipes(spec).unwrap();
    let control = MinerControl::spawn(
        MinerId::new("miner-real").unwrap(),
        "real-incarnation".to_string(),
        launched.stdin.unwrap(),
        launched.stdout.unwrap(),
        dir.join("miner.log"),
    )
    .unwrap();
    let snapshot = control
        .request(MinerOperation::Snapshot, serde_json::json!({}))
        .unwrap();
    assert!(snapshot.is_success(), "{snapshot:?}");
    assert_eq!(snapshot.result.unwrap()["snapshot_version"], 1);
    let stopped = control.stop().unwrap();
    assert!(stopped.is_success(), "{stopped:?}");
    assert!(common::wait_until(Duration::from_secs(3), || {
        !spc_lab::supervisor::pid_is_alive(launched.record.pid)
    }));
    let stop = supervisor
        .stop("miner", "miner-real", Duration::from_secs(1))
        .expect("an exited own child is not an ownership error");
    assert!(!stop || !spc_lab::supervisor::pid_is_alive(launched.record.pid));
    assert_eq!(
        supervisor.record("miner", "miner-real").unwrap().state,
        spc_lab::supervisor::ProcessState::Exited
    );
}
