mod common;

use spc_lab::command::{BeginOutcome, CommandKind, CommandLog, LabCommand, OutcomeState};
use spc_lab::ids::{ActorId, CommandId, MinerId, RunId};
use spc_lab::manifest::RunState;

#[test]
fn duplicate_command_id_returns_the_stored_outcome_without_a_second_effect() {
    let dir = common::TestDir::new("command-idempotency");
    let engine = common::prepare_engine(&dir, "scenario_v1_one_pool_two_miners.json");
    engine.start().unwrap();
    let miner = MinerId::new("miner-1").unwrap();
    let command = LabCommand::new(
        engine.run_id().clone(),
        ActorId::new("test").unwrap(),
        CommandKind::PauseWorkers {
            miner_id: miner.clone(),
            workers: Vec::new(),
        },
    );
    let id = command.command_id.clone();
    let first = engine.submit(command.clone());
    assert_eq!(first.state, OutcomeState::Succeeded, "{}", first.detail);
    let second = engine.submit(command);
    assert_eq!(second.state, OutcomeState::Succeeded);
    assert_eq!(first, second);
    assert_eq!(
        engine
            .evidence()
            .events()
            .iter()
            .filter(|event| event.source.as_str() == "command")
            .count(),
        1,
        "the retry must not emit a second command effect"
    );
    assert_eq!(engine.command_log().entries().len(), 1);
    assert_eq!(
        engine.command_log().outcome(&id).unwrap().state,
        OutcomeState::Succeeded
    );

    // Same id, different payload: reject and keep the first effect.
    let conflict = LabCommand {
        command_id: id.clone(),
        run_id: engine.run_id().clone(),
        issued_at_wall: 2,
        actor: ActorId::new("test").unwrap(),
        kind: CommandKind::ResumeWorkers {
            miner_id: miner,
            workers: Vec::new(),
        },
    };
    let outcome = engine.submit(conflict);
    assert_eq!(outcome.state, OutcomeState::RejectedConflict);
    assert_eq!(
        engine.command_log().outcome(&id).unwrap().state,
        OutcomeState::Succeeded
    );
    engine.stop().unwrap();
}

#[test]
fn failed_precondition_is_durable_and_not_a_crash() {
    let dir = common::TestDir::new("command-precondition");
    let engine = common::prepare_engine(&dir, "scenario_v1_one_pool_two_miners.json");
    engine.start().unwrap();
    let command = LabCommand::new(
        engine.run_id().clone(),
        ActorId::new("test").unwrap(),
        CommandKind::RemoveMiner {
            miner_id: MinerId::new("miner-does-not-exist").unwrap(),
        },
    );
    let id = command.command_id.clone();
    let outcome = engine.submit(command);
    assert_eq!(outcome.state, OutcomeState::RejectedPrecondition);
    assert_eq!(
        engine.command_log().outcome(&id).unwrap().state,
        OutcomeState::RejectedPrecondition
    );
    // The run is still alive after a rejected command.
    assert_eq!(engine.manifest_snapshot().state, RunState::Running);
    engine.stop().unwrap();
}

#[test]
fn crash_interrupted_command_is_reconciled_from_the_manifest() {
    let dir = common::TestDir::new("command-reconcile");
    let engine = common::prepare_engine(&dir, "scenario_v1_one_pool_two_miners.json");
    let log_path = dir.join("crash-commands.jsonl");
    let log = CommandLog::open(&log_path).unwrap();
    let run_id = engine.run_id().clone();
    let start = LabCommand::new(
        run_id.clone(),
        ActorId::new("test").unwrap(),
        CommandKind::StartRun { reason: None },
    );
    let stop = LabCommand::new(
        run_id.clone(),
        ActorId::new("test").unwrap(),
        CommandKind::StopRun {
            reason: "interrupted".to_string(),
        },
    );
    assert!(matches!(log.begin(&start).unwrap(), BeginOutcome::New));
    assert!(matches!(log.begin(&stop).unwrap(), BeginOutcome::New));
    drop(log);

    let reopened = CommandLog::open(&log_path).unwrap();
    assert_eq!(reopened.in_flight().len(), 2);
    let mut manifest = engine.manifest_snapshot();
    manifest.state = RunState::Running;
    let outcomes = reopened.reconcile(&manifest).unwrap();
    let start_outcome = outcomes
        .iter()
        .find(|outcome| outcome.command_id == start.command_id)
        .unwrap();
    let stop_outcome = outcomes
        .iter()
        .find(|outcome| outcome.command_id == stop.command_id)
        .unwrap();
    assert_eq!(start_outcome.state, OutcomeState::Succeeded);
    assert_eq!(
        stop_outcome.state,
        OutcomeState::Unknown,
        "an interrupted stop is never assumed successful"
    );
    assert!(reopened.in_flight().is_empty());
}

#[test]
fn command_ids_are_nominal_and_payload_hash_is_stable() {
    let run = RunId::new("run-a").unwrap();
    let actor = ActorId::new("test").unwrap();
    let command = LabCommand::new(
        run.clone(),
        actor,
        CommandKind::StopRun {
            reason: "x".to_string(),
        },
    );
    assert!(command.command_id.as_str().starts_with("cmd-"));
    assert_eq!(
        CommandLog::payload_hash(&command),
        CommandLog::payload_hash(&command)
    );
    let other = LabCommand {
        command_id: CommandId::generate(),
        run_id: run,
        issued_at_wall: 1,
        actor: ActorId::new("test").unwrap(),
        kind: CommandKind::StopRun {
            reason: "y".to_string(),
        },
    };
    assert_ne!(
        CommandLog::payload_hash(&command),
        CommandLog::payload_hash(&other)
    );
}

#[test]
fn pipe_control_commands_are_unknown_after_a_crash() {
    let dir = common::TestDir::new("command-pipe-unknown");
    let engine = common::prepare_engine(&dir, "scenario_v1_one_pool_two_miners.json");
    let path = dir.join("pipe-unknown.jsonl");
    let log = CommandLog::open(&path).unwrap();
    let run = engine.run_id().clone();
    let miner = MinerId::new("miner-1").unwrap();
    let commands = [
        LabCommand::new(
            run.clone(),
            ActorId::new("test").unwrap(),
            CommandKind::ResumeWorkers {
                miner_id: miner.clone(),
                workers: Vec::new(),
            },
        ),
        LabCommand::new(
            run.clone(),
            ActorId::new("test").unwrap(),
            CommandKind::PauseWorkers {
                miner_id: miner.clone(),
                workers: Vec::new(),
            },
        ),
        LabCommand::new(
            run,
            ActorId::new("test").unwrap(),
            CommandKind::SetWorkerBudget {
                miner_id: miner,
                worker_id: spc_lab::ids::WorkerId::new("w-alice-1").unwrap(),
                hashes_per_second: Some(spc_lab::WideU128(1)),
                total_hashes: None,
            },
        ),
    ];
    for command in &commands {
        assert!(matches!(log.begin(command).unwrap(), BeginOutcome::New));
    }
    drop(log);
    let reopened = CommandLog::open(&path).unwrap();
    let outcomes = reopened.reconcile(&engine.manifest_snapshot()).unwrap();
    assert_eq!(outcomes.len(), 3);
    assert!(outcomes
        .iter()
        .all(|outcome| outcome.state == OutcomeState::Unknown));
    assert!(outcomes
        .iter()
        .all(|outcome| outcome.detail.contains("pipe-backed control")));
}

#[test]
fn journal_replay_is_ordered_by_sequence_not_command_id() {
    let dir = common::TestDir::new("command-seq-order");
    let path = dir.join("ordered.jsonl");
    let log = CommandLog::open(&path).unwrap();
    let run = RunId::new("run-seq").unwrap();
    let add = LabCommand {
        command_id: CommandId::new("z-add").unwrap(),
        run_id: run.clone(),
        issued_at_wall: 1,
        actor: ActorId::new("test").unwrap(),
        kind: CommandKind::AddMiner {
            miner_id: MinerId::new("miner-z").unwrap(),
            implementation: "fixture-miner".to_string(),
            connections: Vec::new(),
            workers: Vec::new(),
        },
    };
    let remove = LabCommand {
        command_id: CommandId::new("a-remove").unwrap(),
        run_id: run,
        issued_at_wall: 2,
        actor: ActorId::new("test").unwrap(),
        kind: CommandKind::RemoveMiner {
            miner_id: MinerId::new("miner-z").unwrap(),
        },
    };
    assert!(matches!(log.begin(&add).unwrap(), BeginOutcome::New));
    let add_seq = log.last_seq();
    let mut add_outcome =
        spc_lab::CommandOutcome::new(add.command_id.clone(), OutcomeState::Succeeded, "add");
    add_outcome.journal_seq = add_seq;
    log.finish(&add.command_id, &add_outcome).unwrap();
    assert!(matches!(log.begin(&remove).unwrap(), BeginOutcome::New));
    log.finish(
        &remove.command_id,
        &spc_lab::CommandOutcome::new(remove.command_id.clone(), OutcomeState::Succeeded, "remove"),
    )
    .unwrap();
    let entries = log.entries();
    assert_eq!(entries[0].0.command_id.as_str(), "z-add");
    assert_eq!(entries[1].0.command_id.as_str(), "a-remove");
    assert_eq!(entries[0].1.as_ref().unwrap().journal_seq, add_seq);
}
