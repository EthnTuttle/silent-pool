mod common;

use std::sync::Arc;

use spc_lab::command::{CommandKind, LabCommand, OutcomeState};
use spc_lab::ids::{ActorId, ConnectionId, MinerId, WorkerId};
use spc_lab::scenario::ConnectionSpec;

#[test]
fn concurrent_add_miner_is_log_serialized_and_creates_one_process() {
    let dir = common::TestDir::new("command-concurrency");
    let engine = common::prepare_engine(&dir, "scenario_v1_one_pool_two_miners.json");
    engine.start().unwrap();
    let base_connection = engine.scenario.miners[0].connections[0].clone();
    let base_worker = engine.scenario.miners[0].workers[0].clone();
    let mut commands = Vec::new();
    for index in 0..2 {
        let mut connection = ConnectionSpec {
            id: ConnectionId::new(format!("conn-new-{index}")).unwrap(),
            ..base_connection.clone()
        };
        connection.front_id = base_connection.front_id.clone();
        let worker = spc_lab::scenario::WorkerSpec {
            id: WorkerId::new(format!("w-new-{index}")).unwrap(),
            connection_id: connection.id.clone(),
            ..base_worker.clone()
        };
        commands.push(LabCommand::new(
            engine.run_id().clone(),
            ActorId::new("test").unwrap(),
            CommandKind::AddMiner {
                miner_id: MinerId::new("miner-new").unwrap(),
                implementation: "fixture-miner".to_string(),
                connections: vec![connection],
                workers: vec![worker],
            },
        ));
    }
    let engine: Arc<_> = engine;
    let handles: Vec<_> = commands
        .into_iter()
        .map(|command| {
            let engine = Arc::clone(&engine);
            std::thread::spawn(move || engine.submit(command))
        })
        .collect();
    let outcomes: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    let succeeded = outcomes
        .iter()
        .filter(|outcome| outcome.state == OutcomeState::Succeeded)
        .count();
    let rejected = outcomes
        .iter()
        .filter(|outcome| outcome.state == OutcomeState::RejectedPrecondition)
        .count();
    assert_eq!(succeeded, 1, "{outcomes:?}");
    assert_eq!(rejected, 1, "{outcomes:?}");
    assert!(outcomes.iter().all(|outcome| outcome.journal_seq > 0));
    let records: Vec<_> = engine
        .manifest_snapshot()
        .processes
        .into_iter()
        .filter(|record| record.role == "miner" && record.entity == "miner-new")
        .collect();
    assert_eq!(
        records.len(),
        1,
        "one process record; no untracked duplicate"
    );
    assert_eq!(engine.command_log().entries().len(), 2);
    engine.stop().unwrap();
}
