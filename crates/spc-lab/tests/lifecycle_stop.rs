mod common;

use std::time::Duration;

use spc_lab::command::{CommandKind, LabCommand, OutcomeState};
use spc_lab::engine::RunEngine;
use spc_lab::ids::{ActorId, MinerId};
use spc_lab::manifest::RunState;
use spc_lab::supervisor::{pid_is_alive, ProcessState};

fn stop_command(engine: &RunEngine) -> LabCommand {
    LabCommand::new(
        engine.run_id().clone(),
        ActorId::new("test").unwrap(),
        CommandKind::StopRun {
            reason: "test".to_string(),
        },
    )
}

#[test]
fn stop_after_crash_stops_adopted_processes_and_records_are_terminal() {
    let dir = common::TestDir::new("lifecycle-stop");
    let engine = common::prepare_engine(&dir, "scenario_v1_one_pool_two_miners.json");
    engine.start().unwrap();
    let run_dir = engine.run_dir.clone();
    let pids: Vec<u32> = engine
        .manifest_snapshot()
        .processes
        .iter()
        .filter(|record| record.role != "miner")
        .map(|record| record.pid)
        .collect();
    drop(engine);

    let attached = RunEngine::attach(&run_dir, "stop-test").unwrap();
    assert_eq!(
        attached.manifest_snapshot().state,
        RunState::CompletedWithErrors
    );
    let outcome = attached.submit(stop_command(&attached));
    assert_eq!(outcome.state, OutcomeState::Succeeded, "{}", outcome.detail);
    assert!(common::wait_until(Duration::from_secs(5), || {
        pids.iter().all(|pid| !pid_is_alive(*pid))
    }));
    let manifest = attached.manifest_snapshot();
    assert!(matches!(
        manifest.state,
        RunState::Stopped | RunState::CompletedWithErrors
    ));
    assert!(manifest.processes.iter().all(|record| matches!(
        record.state,
        ProcessState::Stopped | ProcessState::Exited | ProcessState::OrphanUnverified
    )));
}

#[test]
fn stop_on_created_run_is_a_rejected_precondition_not_success() {
    let dir = common::TestDir::new("lifecycle-created");
    let engine = common::prepare_engine(&dir, "scenario_v1_one_pool_two_miners.json");
    let outcome = engine.submit(stop_command(&engine));
    assert_eq!(outcome.state, OutcomeState::RejectedPrecondition);
    assert_eq!(engine.manifest_snapshot().state, RunState::Created);
}

#[test]
fn pause_state_is_not_inferred_after_adoption() {
    let dir = common::TestDir::new("lifecycle-pause");
    let engine = common::prepare_engine(&dir, "scenario_v1_one_pool_two_miners.json");
    engine.start().unwrap();
    let pause = LabCommand::new(
        engine.run_id().clone(),
        ActorId::new("test").unwrap(),
        CommandKind::PauseWorkers {
            miner_id: MinerId::new("miner-1").unwrap(),
            workers: Vec::new(),
        },
    );
    assert_eq!(engine.submit(pause).state, OutcomeState::Succeeded);
    let run_dir = engine.run_dir.clone();
    drop(engine);

    let attached = RunEngine::attach(&run_dir, "pause-test").unwrap();
    // Pause desired state is rebuilt from the finished journal, but the
    // pipe-backed control is gone: workers are not reported as live/running.
    let entities = attached.entity_states();
    assert!(entities
        .iter()
        .filter(|entity| entity.kind == "worker")
        .all(|entity| entity.status != "running"));
    assert!(entities.iter().any(|entity| {
        entity.kind == "miner"
            && matches!(
                entity.status.as_str(),
                "control_lost" | "exited" | "unknown"
            )
    }));
    let outcome = attached.submit(stop_command(&attached));
    assert_eq!(outcome.state, OutcomeState::Succeeded);
}
