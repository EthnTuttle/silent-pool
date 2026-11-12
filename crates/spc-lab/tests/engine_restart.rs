mod common;

use std::time::Duration;

use spc_lab::engine::RunEngine;
use spc_lab::supervisor::{pid_is_alive, ProcessState};

#[test]
fn restart_adopts_surviving_processes_and_reap_cleans_only_verified_ones() {
    let dir = common::TestDir::new("engine-restart");
    let engine = common::prepare_engine(&dir, "scenario_v1_one_pool_two_miners.json");
    engine.start().unwrap();
    let run_dir = engine.run_dir.clone();
    let pids: Vec<u32> = engine
        .manifest_snapshot()
        .processes
        .iter()
        .map(|record| record.pid)
        .collect();
    assert!(!pids.is_empty());
    assert!(pids.iter().all(|pid| pid_is_alive(*pid)));
    drop(engine); // crash: no stop, no kill

    let adopted = RunEngine::attach(&run_dir, "restart-test").unwrap();
    let manifest = adopted.manifest_snapshot();
    // Storage/front/node processes are adopted. Pipe-backed miners must not
    // be promised as live hash workers: the real driver closes on control
    // EOF, so their state is exited/unverified and the run is stale.
    assert!(
        manifest
            .processes
            .iter()
            .filter(|record| record.role != "miner")
            .all(|record| matches!(
                record.state,
                ProcessState::Adopted | ProcessState::AdoptedControlLost
            )),
        "{:?}",
        manifest
            .processes
            .iter()
            .map(|record| (record.role.as_str(), record.state))
            .collect::<Vec<_>>()
    );
    assert!(manifest
        .processes
        .iter()
        .filter(|record| record.role == "miner")
        .all(|record| !matches!(record.state, ProcessState::Adopted)));
    assert!(manifest
        .warnings
        .iter()
        .any(|warning| warning.contains("stale_resources")));

    let reaped = adopted.reap().unwrap();
    assert!(!reaped.is_empty());
    assert!(common::wait_until(Duration::from_secs(3), || {
        pids.iter().all(|pid| !pid_is_alive(*pid))
    }));
}

#[test]
fn attach_reports_unverified_records_instead_of_killing_them() {
    let dir = common::TestDir::new("engine-stale");
    let engine = common::prepare_engine(&dir, "scenario_v1_one_pool_two_miners.json");
    engine.start().unwrap();
    let run_dir = engine.run_dir.clone();
    let pids: Vec<u32> = engine
        .manifest_snapshot()
        .processes
        .iter()
        .map(|record| record.pid)
        .collect();
    drop(engine);

    // Forge one record's start token, simulating a recycled PID.
    let manifest_path = run_dir.join("run-manifest.json");
    let mut manifest = spc_lab::RunManifest::load(&manifest_path).unwrap();
    manifest.processes[0].start_time_ticks = manifest.processes[0].start_time_ticks.wrapping_add(1);
    manifest.save(&manifest_path).unwrap();

    let attached = RunEngine::attach(&run_dir, "stale-test").unwrap();
    assert!(attached
        .manifest_snapshot()
        .processes
        .iter()
        .any(|record| matches!(record.state, ProcessState::OrphanUnverified)));
    // The injected record's PID is still referenced by another genuine record
    // only if the first record was a different entity; the unrelated processes
    // are alive and are not killed by attach.
    assert!(pids.iter().any(|pid| pid_is_alive(*pid)));
    let reaped = attached.reap().unwrap();
    assert!(reaped
        .iter()
        .any(|(_, _, state)| matches!(state, ProcessState::OrphanUnverified)));
    // Clean up genuine processes with a fresh attach of a repaired manifest is
    // not possible here because the forged record belongs to a live pool. Stop
    // the remaining real records through a new supervisor with explicit
    // records; the test process reaps them at exit otherwise.
    for pid in pids {
        if let Some(pid) = rustix::process::Pid::from_raw(pid as i32) {
            let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::TERM);
        }
    }
}

#[test]
fn read_only_attach_does_not_mutate_the_manifest() {
    let dir = common::TestDir::new("engine-read-only");
    let engine = common::prepare_engine(&dir, "scenario_v1_one_pool_two_miners.json");
    engine.start().unwrap();
    let run_dir = engine.run_dir.clone();
    let manifest_path = run_dir.join("run-manifest.json");
    drop(engine);
    let before = std::fs::read(&manifest_path).unwrap();
    {
        let read_only = RunEngine::attach_read_only(&run_dir, "read-only-test").unwrap();
        let snapshot = read_only.snapshot(None);
        assert!(snapshot
            .entities
            .iter()
            .any(|entity| entity.kind == "node" && entity.status == "running"));
        assert!(read_only
            .entity_states()
            .iter()
            .filter(|entity| entity.kind == "worker")
            .all(|entity| entity.status == "unknown"));
    }
    let after = std::fs::read(&manifest_path).unwrap();
    assert_eq!(
        before, after,
        "snapshot review must not rewrite the manifest"
    );
    let adopted = RunEngine::attach(&run_dir, "cleanup-test").unwrap();
    adopted.reap().unwrap();
}
