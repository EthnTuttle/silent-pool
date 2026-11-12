mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use spc_lab::command::{CommandKind, LabCommand, OutcomeState};
use spc_lab::ids::{ActorId, NodeId};

#[test]
fn long_height_wait_does_not_block_stop_and_is_cancelled() {
    let dir = common::TestDir::new("long-wait-stop");
    let engine = common::prepare_engine(&dir, "scenario_v1_one_pool_two_miners.json");
    engine.start().unwrap();
    let wait = LabCommand::new(
        engine.run_id().clone(),
        ActorId::new("test").unwrap(),
        CommandKind::WaitHeight {
            node_id: NodeId::new("node-a").unwrap(),
            height: 100,
            timeout_secs: 30,
        },
    );
    let waiter = Arc::clone(&engine);
    let handle = std::thread::spawn(move || waiter.submit(wait));
    std::thread::sleep(Duration::from_millis(150));
    let start = Instant::now();
    let stop = engine.submit(LabCommand::new(
        engine.run_id().clone(),
        ActorId::new("test").unwrap(),
        CommandKind::StopRun {
            reason: "cancel wait".to_string(),
        },
    ));
    assert_eq!(stop.state, OutcomeState::Succeeded, "{}", stop.detail);
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "stop waited behind the long height observation"
    );
    let wait_outcome = handle.join().unwrap();
    assert_eq!(
        wait_outcome.state,
        OutcomeState::Cancelled,
        "{wait_outcome:?}"
    );
    assert!(engine
        .manifest_snapshot()
        .processes
        .iter()
        .all(|record| matches!(
            record.state,
            spc_lab::supervisor::ProcessState::Stopped | spc_lab::supervisor::ProcessState::Exited
        )));
}
