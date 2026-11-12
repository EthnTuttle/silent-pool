use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

use spc_cpu_miner::budget::HashBudget;
use spc_cpu_miner::config::{ConnectionConfig, WorkerConfig};
use spc_cpu_miner::control::{Command, CommandLedger, CommandResult, Operation};
use spc_cpu_miner::report::{EventEnvelope, EventSink, MinerEvent, OutputMessage};

fn connection() -> ConnectionConfig {
    ConnectionConfig {
        source_audit: spc_cpu_miner::config::SourceAuditConfig::Unaudited {},
        id: "conn-a".into(),
        endpoint: "127.0.0.1:33445".into(),
        identity: "tsp1q-fixture".into(),
        authority_public_key: "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
            .into(),
        min_extranonce_size: 4,
        reconnect: Default::default(),
        workers: vec![WorkerConfig {
            id: "worker-a".into(),
            budget: HashBudget {
                hashes_per_second: Some(100),
                total_attempts: Some(1_000),
            },
            ntime_policy: Default::default(),
            max_ntime_roll_seconds: 600,
            batch_size: 64,
        }],
    }
}

#[test]
fn command_fixture_is_typed_and_versioned() {
    let command = Command {
        protocol: "spc-cpu-miner-control".into(),
        version: 1,
        command_id: "cmd-1".into(),
        operation: Operation::AddConnection {
            connection: connection(),
        },
    };
    let encoded = serde_json::to_string(&command).unwrap();
    let decoded: Command = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, command);
    assert!(encoded.contains("\"operation\":\"add_connection\""));
    assert!(encoded.contains("\"command_id\":\"cmd-1\""));
}

#[test]
fn duplicate_command_replays_outcome_and_conflict_does_not_overwrite() {
    let command = Command {
        protocol: "spc-cpu-miner-control".into(),
        version: 1,
        command_id: "cmd-1".into(),
        operation: Operation::Snapshot {},
    };
    let mut ledger = CommandLedger::default();
    let first =
        CommandResult::succeeded(command.command_id.clone(), serde_json::json!({"ok": true}));
    assert!(ledger.begin(&command).is_none());
    ledger.complete(&command, first.clone());

    let replay = ledger.begin(&command).unwrap();
    assert_eq!(replay, first);
    let mut different = command.clone();
    different.operation = Operation::PauseWorkers {
        worker_ids: vec!["w".into()],
    };
    let conflict = ledger.begin(&different).unwrap();
    assert_eq!(conflict.outcome, "rejected_precondition");
    assert_eq!(conflict.error.unwrap().code, "command_id_conflict");
}

#[test]
fn fixture_file_matches_the_schema() {
    let text = include_str!("../fixtures/control.jsonl");
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let command: Command = serde_json::from_str(line).unwrap();
        command.validate().unwrap();
    }
}

#[test]
fn event_fixture_matches_envelopes() {
    let text = include_str!("../fixtures/events.jsonl");
    let lines: Vec<_> = text.lines().collect();
    for line in &lines[..5] {
        let event: EventEnvelope = serde_json::from_str(line).unwrap();
        assert_eq!(event.protocol, "spc-cpu-miner-events");
        assert_eq!(event.version, 1);
        assert_eq!(event.event_id, event.source_cursor);
    }
    let response: CommandResult = serde_json::from_str(lines[5]).unwrap();
    assert_eq!(response.kind, "command_result");
}

#[test]
fn key_utils_base58_authority_is_accepted() {
    let mut config = connection();
    config.authority_public_key = "9auqWEzQDVyd2oe1JVGFLMLHZtCo2FFqZwtKA5gd9xbuEu7PH72".into();
    let key = config.authority().unwrap();
    assert_eq!(key.into_bytes().len(), 32);
}

fn simple_event() -> MinerEvent {
    MinerEvent::ConnectionState {
        connection_id: "ordering".into(),
        session_slot: None,
        state: "testing".into(),
        detail: None,
    }
}

fn drain_events(rx: &mut tokio::sync::mpsc::Receiver<OutputMessage>) -> Vec<EventEnvelope> {
    let mut events = Vec::new();
    while let Ok(message) = rx.try_recv() {
        if let OutputMessage::Event(event) = message {
            events.push(*event);
        }
    }
    events
}

#[test]
fn concurrent_publishers_have_one_strict_cursor_order() {
    const THREADS: usize = 8;
    const PER_THREAD: usize = 256;
    let (tx, mut rx) = tokio::sync::mpsc::channel(THREADS * PER_THREAD);
    let sink = EventSink::new(tx, "concurrent".into(), Instant::now());
    let barrier = Arc::new(Barrier::new(THREADS));
    let mut threads = Vec::new();
    for _ in 0..THREADS {
        let sink = sink.clone();
        let barrier = Arc::clone(&barrier);
        threads.push(std::thread::spawn(move || {
            barrier.wait();
            for _ in 0..PER_THREAD {
                sink.emit(simple_event());
            }
        }));
    }
    for thread in threads {
        thread.join().unwrap();
    }
    let events = drain_events(&mut rx);
    assert_eq!(events.len(), THREADS * PER_THREAD);
    assert_eq!(sink.dropped(), 0);
    for (index, event) in events.iter().enumerate() {
        assert_eq!(event.event_id, index as u64 + 1);
        assert_eq!(event.source_cursor, event.event_id);
        assert_eq!(event.dropped_before, 0);
    }
}

#[test]
fn full_queue_counts_loss_without_waiting_for_consumer() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(2);
    let sink = EventSink::new(tx, "small-queue".into(), Instant::now());
    let slow_reader = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        let events = drain_events(&mut rx);
        (rx, events)
    });
    let started = Instant::now();
    for _ in 0..8 {
        sink.emit(simple_event());
    }
    assert!(started.elapsed() < Duration::from_millis(90));
    assert_eq!(sink.dropped(), 6);
    let (mut rx, mut delivered) = slow_reader.join().unwrap();
    sink.emit(simple_event()); // trailing evidence for the six failed sends
    delivered.extend(drain_events(&mut rx));

    let ids: Vec<_> = delivered.iter().map(|event| event.event_id).collect();
    assert_eq!(ids, vec![1, 2, 9]);
    let dropped_before: Vec<_> = delivered.iter().map(|event| event.dropped_before).collect();
    assert_eq!(dropped_before, vec![0, 0, 6]);
    assert_eq!(delivered.len() + sink.dropped() as usize, 9);
    assert!(delivered
        .iter()
        .all(|event| event.source_cursor == event.event_id));
    assert_eq!(
        delivered
            .iter()
            .map(|event| event.event_id)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        3
    );
}
