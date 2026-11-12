mod common;

use std::time::Duration;

use spc_lab::evidence::GapScope;

#[test]
fn cumulative_driver_dropped_total_is_recorded_once_per_incarnation_and_parse_errors_are_scoped() {
    let dir = common::TestDir::new("driver-loss");
    let mut config = common::fixture_config();
    let miner = config.miners.get_mut("fixture-miner").unwrap();
    miner.args.push("--dropped-before".to_string());
    miner.args.push("3".to_string());
    miner.args.push("--emit-invalid".to_string());
    let engine = common::prepare_engine_with(
        &dir,
        common::fixture_scenario("scenario_v1_one_pool_two_miners.json"),
        config,
    );
    engine.start().unwrap();
    assert!(common::wait_until(Duration::from_secs(3), || {
        engine
            .evidence()
            .gaps()
            .iter()
            .any(|gap| gap.scope == GapScope::Store)
    }));
    // Give the second cumulative event a chance to arrive; it must not create a
    // second global gap for the same cumulative dropped total.
    std::thread::sleep(Duration::from_millis(150));
    let gaps = engine.evidence().gaps();
    let store_gaps: Vec<_> = gaps
        .iter()
        .filter(|gap| gap.scope == GapScope::Store)
        .collect();
    assert_eq!(
        store_gaps.len(),
        4,
        "two miners x (one cumulative delta + one parse error); the repeated cumulative frame adds none"
    );
    assert_eq!(
        store_gaps
            .iter()
            .filter(|gap| gap.reason.contains("delta=3"))
            .count(),
        2
    );
    assert_eq!(
        store_gaps
            .iter()
            .filter(|gap| gap.reason.contains("parse failure"))
            .count(),
        2
    );
    engine.stop().unwrap();
}
