mod common;

use serde_json::Value;
use spc_lab::evidence::{ActorKind, EvidenceActor, EvidencePayload, UnknownEvidence};
use spc_lab::http::{HttpServer, WebAssets};
use spc_lab::ids::SourceId;
use spc_lab::RunEngine;

fn assets() -> WebAssets {
    WebAssets {
        index_html: include_str!("../../../bins/spc-lab/static/index.html"),
        app_js: include_str!("../../../bins/spc-lab/static/app.js"),
        style_css: include_str!("../../../bins/spc-lab/static/style.css"),
        lab_html: include_str!("../../../bins/spc-lab/static/lab.html"),
        lab_js: include_str!("../../../bins/spc-lab/static/lab.js"),
        lab_css: include_str!("../../../bins/spc-lab/static/lab.css"),
    }
}

fn inventory_json(engine: &RunEngine) -> Value {
    serde_json::to_value(engine.lab_inventory()).unwrap()
}

fn row<'a>(inventory: &'a Value, id: &str) -> &'a Value {
    inventory["groups"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|group| group["rows"].as_array().unwrap())
        .find(|row| row["id"] == id)
        .unwrap_or_else(|| panic!("missing inventory row {id}: {inventory}"))
}

fn integration<'a>(inventory: &'a Value, id: &str) -> &'a Value {
    inventory["integrations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == id)
        .unwrap_or_else(|| panic!("missing integration {id}"))
}

#[test]
fn one_and_two_pool_inventories_show_colocated_roles_not_extra_daemons() {
    for (fixture, pools) in [
        ("scenario_v1_one_pool_two_miners.json", 1),
        ("scenario_v1_two_pools.json", 2),
    ] {
        let dir = common::TestDir::new("hub-inventory");
        let engine = common::prepare_engine(&dir, fixture);
        let inventory = inventory_json(&engine);
        let actual_pool_groups = inventory["groups"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|group| group["id"].as_str().unwrap().starts_with("pool-"))
            .count();
        assert_eq!(actual_pool_groups, pools);
        assert_eq!(inventory["run_id"], engine.run_id().to_string());
        assert_eq!(
            inventory["declared_actor_count"].as_u64().unwrap(),
            if pools == 1 { 6 } else { 10 }
        );
        assert!(!inventory["groups"]
            .as_array()
            .unwrap()
            .iter()
            .any(|group| group["id"] == "declared-ark"));

        for (front, pool) in [("front-a1", "pool-a"), ("front-b1", "pool-b")]
            .into_iter()
            .take(pools)
        {
            let front_row = row(&inventory, &format!("front-{front}"));
            assert_eq!(front_row["process"], format!("fixture-pool {front}"));
            assert_eq!(
                front_row["declared_role"],
                format!("front {front} on {pool}")
            );
            assert_eq!(
                front_row["logical_roles"],
                serde_json::json!(["front", "payout", "api"])
            );
            assert_eq!(front_row["recorded_state"], "not_started");
        }
        assert_eq!(
            row(&inventory, "scanner")["recorded_state"],
            "embedded_no_standalone_process"
        );
        assert_eq!(
            row(&inventory, "scanner")["process"],
            "spc-lab collector thread"
        );
        let agent = integration(&inventory, "spc-agent");
        assert_eq!(agent["current_status"], "not_running");
        let agent_text = agent.to_string();
        assert!(agent_text.contains("standard scenario launches no agent wallet"));
        assert!(!agent_text.contains("wallet remains unimplemented"));
        assert_eq!(
            integration(&inventory, "independent-ark-server")["current_status"],
            "not_running_planned_required"
        );
        assert_eq!(
            integration(&inventory, "spc-user-wallets")["current_status"],
            "not_running_planned_required"
        );
        for pool in ["pool-a", "pool-b"].into_iter().take(pools) {
            let ark = integration(&inventory, &format!("ark-delivery-{pool}"));
            assert_eq!(ark["current_status"], "not_running_planned_required");
            assert_eq!(ark["required_next_profile"], true);
            assert!(ark["details"]
                .as_array()
                .unwrap()
                .iter()
                .any(|line| line.as_str().unwrap().contains("operator Ark wallet")));
        }
        assert!(!inventory.to_string().contains("\"declared-ark\""));
    }
}

#[test]
fn inventory_is_allowlisted_and_observation_age_comes_from_evidence() {
    let dir = common::TestDir::new("hub-privacy");
    let mut scenario = common::fixture_scenario("scenario_v1_one_pool_two_miners.json");
    scenario.resources.redis_instances[0].password = Some("HUB-SECRET-REDIS".to_string());
    scenario.resources.postgres_databases[0].password = Some("HUB-SECRET-PG".to_string());
    let mut config = common::fixture_config();
    for adapter in config.pools.values_mut() {
        adapter
            .env
            .insert("HUB_ENV_MARKER".to_string(), "HUB-SECRET-ENV".to_string());
    }
    let engine = common::prepare_engine_with(&dir, scenario, config);
    engine.evidence().append_at(
        SourceId::trusted("redis-redis-pool-a"),
        "observation-1",
        EvidenceActor::new(ActorKind::Harness, "test-collector"),
        None,
        EvidencePayload::Unknown(UnknownEvidence {
            topic: "redis_observation".to_string(),
            detail: serde_json::json!({
                "ping": "PONG",
                "secret": "HUB-SECRET-DETAIL",
                "path": "/secret/private/path"
            })
            .to_string(),
        }),
        1_000,
    );

    let inventory = serde_json::to_value(spc_lab::inventory::project(&engine, 2_000)).unwrap();
    let observation = row(&inventory, "redis-redis-pool-a")["observation"].clone();
    assert_eq!(observation["availability"], "reachable_when_observed");
    assert_eq!(observation["observed_at_ms"], 1_000);
    assert_eq!(observation["age_ms"], 1_000);
    assert_eq!(observation["freshness"], "recent");
    let older = serde_json::to_value(spc_lab::inventory::project(&engine, 20_000)).unwrap();
    assert_eq!(
        row(&older, "redis-redis-pool-a")["observation"]["freshness"],
        "older"
    );

    let encoded = inventory.to_string().to_lowercase();
    for forbidden in [
        "hub-secret",
        "hub_env_marker",
        "/tmp/",
        "/home/",
        "/secret/",
        "redis://",
        "postgres://",
        "run-manifest",
        "run_dir",
        "executable_path",
        "log_path",
        "\"args\"",
        "\"env\"",
        "cookie",
        "password",
        "scan_key",
        "spend_key",
        "sp_id",
        "b_scan",
        "b_spend",
        "payout_descriptor",
        "tr(tpub",
        "id-alice",
        "id-bob",
    ] {
        assert!(
            !encoded.contains(forbidden),
            "forbidden marker {forbidden:?} in {encoded}"
        );
    }
    assert!(!encoded.contains("processrecord"));
    assert_eq!(
        row(&inventory, "redis-redis-pool-a")["observation"]["source"],
        "authenticated Redis read collector"
    );
}

#[test]
fn namespaced_api_is_internal_not_a_host_browser_link() {
    let dir = common::TestDir::new("hub-namespace");
    let mut scenario = common::fixture_scenario("scenario_v1_one_pool_two_miners.json");
    scenario.fronts[0].listen.api = Some("127.0.0.1:0".to_string());
    let engine = common::prepare_engine_with(&dir, scenario, common::fixture_config());
    engine
        .record_placement(serde_json::json!({"published": "test-only"}))
        .unwrap();
    let inventory = inventory_json(&engine);
    assert_eq!(inventory["topology"], "namespaces");
    let interfaces = row(&inventory, "front-front-a1")["interfaces"]
        .as_array()
        .unwrap();
    let api = interfaces
        .iter()
        .find(|item| item["kind"] == "api_json")
        .unwrap();
    let metrics = interfaces
        .iter()
        .find(|item| item["kind"] == "metrics")
        .unwrap();
    assert!(api["url"].is_null());
    assert!(metrics["url"].is_null());
    assert!(api["note"].as_str().unwrap().contains("Internal namespace"));
    assert!(!inventory.to_string().contains("127.0.0.1:"));
    assert_eq!(
        row(&inventory, "namespace-broker")["recorded_state"],
        "outside_scenario_process_records"
    );
}

#[test]
fn host_pool_api_and_metrics_are_typed_actual_routes() {
    let dir = common::TestDir::new("hub-links");
    let mut scenario = common::fixture_scenario("scenario_v1_one_pool_two_miners.json");
    scenario.fronts[0].listen.api = Some("127.0.0.1:0".to_string());
    let engine = common::prepare_engine_with(&dir, scenario, common::fixture_config());
    let inventory = inventory_json(&engine);
    let interfaces = row(&inventory, "front-front-a1")["interfaces"]
        .as_array()
        .unwrap();
    let api = interfaces
        .iter()
        .find(|item| item["kind"] == "api_json")
        .unwrap();
    let metrics = interfaces
        .iter()
        .find(|item| item["kind"] == "metrics")
        .unwrap();
    assert!(api["url"]
        .as_str()
        .unwrap()
        .starts_with("http://127.0.0.1:"));
    assert!(api["url"].as_str().unwrap().ends_with("/api/pool"));
    assert!(metrics["url"].as_str().unwrap().ends_with("/metrics"));
    assert_ne!(api["kind"], "ui");
    assert_ne!(metrics["kind"], "ui");
    for role in ["node-node-a", "redis-redis-pool-a", "postgres-pg-pool-a"] {
        assert!(row(&inventory, role)["interfaces"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["url"].is_null()));
    }
}

#[test]
fn get_hub_and_inventory_do_not_mutate_run_or_commands() {
    let dir = common::TestDir::new("hub-read-only-get");
    let engine = common::prepare_engine(&dir, "scenario_v1_one_pool_two_miners.json");
    let commands_before = engine.command_log().entries().len();
    let events_before = engine.evidence().last_event_id();
    let manifest_before = engine.manifest_snapshot();
    let server = HttpServer::start(engine.clone(), "127.0.0.1:0", assets()).unwrap();
    for path in [
        "/lab",
        "/lab.js",
        "/lab.css",
        &format!("/api/lab/v1/runs/{}/inventory", engine.run_id()),
    ] {
        let response = common::http_request(server.addr(), "GET", path, None).unwrap();
        assert_eq!(response.status, 200, "GET {path}");
    }
    assert_eq!(engine.command_log().entries().len(), commands_before);
    assert_eq!(engine.evidence().last_event_id(), events_before);
    let manifest_after = engine.manifest_snapshot();
    assert_eq!(manifest_after.state, manifest_before.state);
    assert_eq!(manifest_after.updated_at, manifest_before.updated_at);
    assert_eq!(
        manifest_after.processes.len(),
        manifest_before.processes.len()
    );
    server.shutdown();
}

#[test]
fn refresh_uses_new_process_state_and_recorded_serve_is_explicit() {
    let dir = common::TestDir::new("hub-refresh");
    let engine = common::prepare_engine(&dir, "scenario_v1_one_pool_two_miners.json");
    engine.start().unwrap();
    let server = HttpServer::start(engine.clone(), "127.0.0.1:0", assets()).unwrap();
    let path = format!("/api/lab/v1/runs/{}/inventory", engine.run_id());
    let running = common::http_request(server.addr(), "GET", &path, None)
        .unwrap()
        .json();
    assert_eq!(running["view"], "live");
    assert_eq!(running["lifecycle"], "running");
    assert!(running["recorded_live_actor_count"].as_u64().unwrap() >= 5);
    engine.stop().unwrap();
    let stopped = common::http_request(server.addr(), "GET", &path, None)
        .unwrap()
        .json();
    assert_eq!(stopped["lifecycle"], "stopped");
    assert_eq!(stopped["recorded_live_actor_count"], 0);
    server.shutdown();

    let recorded_engine = RunEngine::attach_read_only(dir.path(), "recorded-hub-test").unwrap();
    let recorded_server =
        HttpServer::start(recorded_engine.clone(), "127.0.0.1:0", assets()).unwrap();
    let recorded = common::http_request(recorded_server.addr(), "GET", &path, None)
        .unwrap()
        .json();
    assert_eq!(recorded["view"], "recorded_read_only");
    assert_eq!(recorded["latest_evidence"]["freshness"], "recorded");
    let page = common::http_request(recorded_server.addr(), "GET", "/lab", None).unwrap();
    assert!(page.text().contains("id=\"view-banner\""));
    let lab_js = common::http_request(recorded_server.addr(), "GET", "/lab.js", None).unwrap();
    assert!(lab_js.text().contains("Recorded-run serve"));
    let root = common::http_request(recorded_server.addr(), "GET", "/", None).unwrap();
    assert!(root.text().contains("recorded-banner"));
    assert!(root.text().contains("href=\"/lab\""));
    recorded_server.shutdown();
}
