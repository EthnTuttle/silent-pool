mod common;

use std::collections::BTreeSet;

use spc_lab::command::{CommandKind, LabCommand};
use spc_lab::ids::ActorId;

#[test]
fn two_live_runs_do_not_share_ports_directories_or_storage() {
    let dir_a = common::TestDir::new("isolation-a");
    let dir_b = common::TestDir::new("isolation-b");
    let scenario_a = common::scenario_with_run_id(
        common::fixture_scenario("scenario_v1_two_pools.json"),
        "run-isolation-a",
    );
    let scenario_b = common::scenario_with_run_id(
        common::fixture_scenario("scenario_v1_two_pools.json"),
        "run-isolation-b",
    );
    let engine_a = common::prepare_engine_with(&dir_a, scenario_a, common::fixture_config());
    let engine_b = common::prepare_engine_with(&dir_b, scenario_b, common::fixture_config());
    engine_a.start().unwrap();
    engine_b.start().unwrap();

    let manifest_a = engine_a.manifest_snapshot();
    let manifest_b = engine_b.manifest_snapshot();
    assert_eq!(manifest_a.resources.redis_instances.len(), 2);
    assert_eq!(manifest_b.resources.redis_instances.len(), 2);

    let ports_a: BTreeSet<u16> = manifest_a
        .resources
        .redis_instances
        .iter()
        .chain(manifest_b.resources.redis_instances.iter())
        .map(|redis| redis.port)
        .collect();
    assert_eq!(ports_a.len(), 4, "all Redis instances are unique");
    let redis_dirs: BTreeSet<_> = manifest_a
        .resources
        .redis_instances
        .iter()
        .chain(manifest_b.resources.redis_instances.iter())
        .map(|redis| redis.dir.clone())
        .collect();
    assert_eq!(redis_dirs.len(), 4, "all Redis data directories are unique");

    let pg_ports: BTreeSet<u16> = manifest_a
        .resources
        .postgres_databases
        .iter()
        .chain(manifest_b.resources.postgres_databases.iter())
        .map(|pg| pg.port)
        .collect();
    assert_eq!(pg_ports.len(), 4, "all Postgres clusters are unique");
    let databases_a: BTreeSet<_> = manifest_a
        .resources
        .postgres_databases
        .iter()
        .map(|pg| (pg.database.clone(), pg.role.clone()))
        .collect();
    let databases_b: BTreeSet<_> = manifest_b
        .resources
        .postgres_databases
        .iter()
        .map(|pg| (pg.database.clone(), pg.role.clone()))
        .collect();
    assert_eq!(
        databases_a.len(),
        2,
        "each pool has its own database in run A"
    );
    assert_eq!(
        databases_b.len(),
        2,
        "each pool has its own database in run B"
    );
    // The run-owned clusters keep even identical declared names isolated.
    assert_eq!(pg_ports.len(), 4);
    assert_ne!(manifest_a.resources.run_dir, manifest_b.resources.run_dir);

    // Within one run, each pool has its own Redis and Postgres endpoint.
    let pool_a_redis = manifest_a
        .storage_for_pool(&spc_lab::ids::PoolId::new("pool-a").unwrap())
        .unwrap();
    let pool_b_redis = manifest_a
        .storage_for_pool(&spc_lab::ids::PoolId::new("pool-b").unwrap())
        .unwrap();
    assert_ne!(pool_a_redis.0.port, pool_b_redis.0.port);
    assert_ne!(pool_a_redis.1.database, pool_b_redis.1.database);
    assert_ne!(pool_a_redis.1.role, pool_b_redis.1.role);

    // A pause command on run A does not touch run B.
    let pause = LabCommand::new(
        engine_a.run_id().clone(),
        ActorId::new("test").unwrap(),
        CommandKind::PauseWorkers {
            miner_id: spc_lab::ids::MinerId::new("miner-1").unwrap(),
            workers: Vec::new(),
        },
    );
    assert_eq!(
        engine_a.submit(pause).state,
        spc_lab::command::OutcomeState::Succeeded
    );
    let states_a = engine_a.entity_states();
    let states_b = engine_b.entity_states();
    assert!(states_a
        .iter()
        .any(|entity| entity.kind == "miner" && entity.status == "paused"));
    assert!(states_b
        .iter()
        .any(|entity| entity.kind == "miner" && entity.status != "paused"));

    engine_a.stop().unwrap();
    engine_b.stop().unwrap();
}
