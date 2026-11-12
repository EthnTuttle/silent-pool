mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use spc_lab::command::{CommandKind, LabCommand, OutcomeState};
use spc_lab::http::{HttpServer, WebAssets};
use spc_lab::ids::ActorId;

#[test]
fn concurrent_snapshots_do_not_deadlock_a_stop_with_live_children() {
    let dir = common::TestDir::new("snapshot-stop-race");
    let engine = common::prepare_engine(&dir, "scenario_v1_one_pool_two_miners.json");
    engine.start().unwrap();
    let server = HttpServer::start(engine.clone(), "127.0.0.1:0", WebAssets::fallback()).unwrap();
    let addr = server.addr();
    let run_id = engine.run_id().to_string();
    let stop = Arc::new(AtomicBool::new(false));
    let mut readers = Vec::new();
    for _ in 0..8 {
        let run_id = run_id.clone();
        let stop = Arc::clone(&stop);
        readers.push(std::thread::spawn(move || {
            let mut count = 0u64;
            while !stop.load(Ordering::Relaxed) {
                let path = format!("/api/lab/v1/runs/{run_id}/snapshot?cursor=0");
                if common::http_request(addr, "GET", &path, None).is_err() {
                    break;
                }
                count += 1;
            }
            count
        }));
    }
    // Let the pollers overlap the running state and evidence emission.
    std::thread::sleep(Duration::from_millis(150));
    let started = Instant::now();
    let outcome = engine.submit(LabCommand::new(
        engine.run_id().clone(),
        ActorId::new("test").unwrap(),
        CommandKind::StopRun {
            reason: "race regression".to_string(),
        },
    ));
    stop.store(true, Ordering::Relaxed);
    let elapsed = started.elapsed();
    assert_eq!(outcome.state, OutcomeState::Succeeded, "{}", outcome.detail);
    assert!(
        elapsed < Duration::from_secs(8),
        "stop took {elapsed:?}; snapshot/evidence lock order deadlocked"
    );
    for reader in readers {
        let count = reader.join().expect("snapshot thread panicked");
        assert!(count > 0, "snapshot poller never completed a request");
    }
    assert!(engine
        .manifest_snapshot()
        .processes
        .iter()
        .all(|record| record.state.as_str() != "running"));
    server.shutdown();
}
