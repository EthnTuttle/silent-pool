//! Cleanup-purpose attach (`reap`).
//!
//! A stopped run has no live processes to adopt, so the strict live-adopt
//! gate refuses it; `attach_for_cleanup` reopens it by validating only the
//! persisted bindings, and `reap` reports every recorded process plus any
//! expected-but-absent traffic node as exited.

mod common;

use common::{fixture_config, fixture_scenario, prepare_engine_with, TestDir};
use spc_lab::engine::RunEngine;
use spc_lab::supervisor::ProcessState;

#[test]
fn attach_for_cleanup_reaps_a_stopped_run() {
    let dir = TestDir::new("reap-cleanup");
    let engine = prepare_engine_with(
        &dir,
        fixture_scenario("scenario_v1_one_pool_two_miners.json"),
        fixture_config(),
    );
    engine.start().unwrap();
    let recorded = engine.manifest_snapshot().processes.len();
    assert!(recorded > 0, "a started run records its processes");
    engine.stop().unwrap();

    // For a stopped non-traffic run even a live adopt is benign (dead PIDs
    // verify as Missing and adopt as Exited). The run mode `reap` actually
    // fixes is the traffic run, whose live attach refuses in
    // `reverify_traffic_binding` (stopped nodes are no longer Owned); that
    // refusal-vs-acceptance distinction is asserted by the internal
    // `reap_cleanup_tests` suite. Here the CLI path is what is under test:
    // cleanup attach must open a stopped run and report every recorded
    // process as exited.
    let reaper = RunEngine::attach_for_cleanup(dir.path(), &format!("reap-{}", std::process::id()))
        .expect("cleanup attach must accept a stopped run");
    let results = reaper.reap().unwrap();
    assert_eq!(
        results.len(),
        recorded,
        "every recorded process must be reported: {results:?}"
    );
    for (role, entity, state) in &results {
        assert_eq!(
            state,
            &ProcessState::Exited,
            "stopped processes report as exited: {role}/{entity}"
        );
    }
    // Reaping twice is stable (release the owner lock first).
    drop(reaper);
    let again =
        RunEngine::attach_for_cleanup(dir.path(), &format!("reap-again-{}", std::process::id()))
            .unwrap();
    let results = again.reap().unwrap();
    assert_eq!(results.len(), recorded);
}

#[test]
fn attach_for_cleanup_opens_an_unstarted_run() {
    let dir = TestDir::new("reap-unstarted");
    let engine = prepare_engine_with(
        &dir,
        fixture_scenario("scenario_v1_one_pool_two_miners.json"),
        fixture_config(),
    );
    // Release the owner lock before reopening for cleanup.
    let run_dir = dir.path().to_path_buf();
    drop(engine);
    let reaper = RunEngine::attach_for_cleanup(&run_dir, &format!("reap-{}", std::process::id()))
        .expect("cleanup attach must accept a never-started run");
    let results = reaper.reap().unwrap();
    assert!(results.is_empty(), "nothing was ever launched: {results:?}");
}
