mod common;

use std::time::{Duration, Instant};

use spc_lab::command::{CommandKind, LabCommand, OutcomeState};
use spc_lab::ids::{ActorId, MinerId, NodeId};

fn wait_command(engine: &spc_lab::engine::RunEngine, height: u64, timeout_secs: u64) -> LabCommand {
    LabCommand::new(
        engine.run_id().clone(),
        ActorId::new("test").unwrap(),
        CommandKind::WaitHeight {
            node_id: NodeId::new("node-a").unwrap(),
            height,
            timeout_secs,
        },
    )
}

fn pause(engine: &spc_lab::engine::RunEngine, miner: &str, pause: bool) -> OutcomeState {
    let kind = if pause {
        CommandKind::PauseWorkers {
            miner_id: MinerId::new(miner).unwrap(),
            workers: Vec::new(),
        }
    } else {
        CommandKind::ResumeWorkers {
            miner_id: MinerId::new(miner).unwrap(),
            workers: Vec::new(),
        }
    };
    engine
        .submit(LabCommand::new(
            engine.run_id().clone(),
            ActorId::new("test").unwrap(),
            kind,
        ))
        .state
}

#[test]
fn paused_producers_make_a_height_wait_blocked_not_a_timeout() {
    let dir = common::TestDir::new("height-blocked");
    let engine = common::prepare_engine(&dir, "scenario_v1_one_pool_two_miners.json");
    engine.start().unwrap();
    assert_eq!(pause(&engine, "miner-1", true), OutcomeState::Succeeded);
    assert_eq!(pause(&engine, "miner-2", true), OutcomeState::Succeeded);
    let started = Instant::now();
    let outcome = engine.submit(wait_command(&engine, 5, 5));
    assert_eq!(
        outcome.state,
        OutcomeState::BlockedOnHeight,
        "{}",
        outcome.detail
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "blocked_on_height must be reported immediately, not await the timeout"
    );

    // With the producers resumed, the same wait is a bounded timeout while the
    // chain cannot advance.
    assert_eq!(pause(&engine, "miner-1", false), OutcomeState::Succeeded);
    assert_eq!(pause(&engine, "miner-2", false), OutcomeState::Succeeded);
    let outcome = engine.submit(wait_command(&engine, 5, 1));
    assert_eq!(outcome.state, OutcomeState::Failed);
    assert!(outcome.detail.contains("timed out"), "{}", outcome.detail);

    // A node that actually advances succeeds through the same adapter.
    common::write_height(&dir, "node-a", 5);
    let outcome = engine.submit(wait_command(&engine, 5, 2));
    assert_eq!(outcome.state, OutcomeState::Succeeded, "{}", outcome.detail);
    engine.stop().unwrap();
}
