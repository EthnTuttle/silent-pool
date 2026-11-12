mod common;

use std::time::Duration;

use spc_lab::http::{HttpServer, WebAssets};

struct StopOnDrop(std::sync::Arc<spc_lab::RunEngine>);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        let _ = self.0.stop();
    }
}

#[test]
fn http_snapshot_command_api_and_sse_reattach_share_one_contract() {
    let dir = common::TestDir::new("http-sse");
    let engine = common::prepare_engine(&dir, "scenario_v1_one_pool_two_miners.json");
    let _cleanup = StopOnDrop(engine.clone());
    engine.start().unwrap();
    let server = HttpServer::start(engine.clone(), "127.0.0.1:0", WebAssets::fallback()).unwrap();
    let addr = server.addr();
    let run_id = engine.run_id().to_string();

    let index = common::http_request(addr, "GET", "/", None).unwrap();
    assert_eq!(index.status, 200);
    assert!(index.text().contains("spc-lab"));

    // The API serves whichever assets it was started with; with the real
    // embedded module the browser code (generic SSE listener and the
    // id-mapping controls) is reachable, and the fallback stays a fallback.
    let real_js: &'static str = include_str!("../../../bins/spc-lab/static/app.js");
    let real_html: &'static str = include_str!("../../../bins/spc-lab/static/index.html");
    let server_with_app = HttpServer::start(
        engine.clone(),
        "127.0.0.1:0",
        WebAssets {
            index_html: real_html,
            app_js: real_js,
            style_css: "/* unused here */",
            lab_html: "<!doctype html><title>lab</title>",
            lab_js: "/* unused here */",
            lab_css: "/* unused here */",
        },
    )
    .unwrap();
    let app_addr = server_with_app.addr();
    let app_js = common::http_request(app_addr, "GET", "/app.js", None).unwrap();
    assert_eq!(app_js.status, 200);
    assert!(app_js.text().contains("controlCommand"));
    assert_eq!(app_js.text(), real_js);
    assert!(!app_js.text().contains("new EventSource"));
    let access_js = common::http_request(app_addr, "GET", "/access.js", None).unwrap();
    assert_eq!(access_js.status, 200);
    assert_eq!(
        access_js.text(),
        include_str!("../../../bins/spc-lab/static/access.js")
    );
    assert!(app_js.text().contains("miner_id"));
    let served_html = common::http_request(app_addr, "GET", "/", None).unwrap();
    assert!(served_html.text().contains("control-error"));
    server_with_app.shutdown();

    let snapshot = common::http_request(
        addr,
        "GET",
        &format!("/api/lab/v1/runs/{run_id}/snapshot"),
        None,
    )
    .unwrap();
    assert_eq!(snapshot.status, 200);
    let snapshot_json = snapshot.json();
    assert_eq!(snapshot_json["run_id"], run_id);
    let cursor = snapshot_json["snapshot_cursor"].as_u64().unwrap();
    assert!(cursor > 0);
    assert!(snapshot_json["entities"].as_array().unwrap().len() >= 4);

    let command = common::command(
        &run_id,
        serde_json::json!({
            "type": "pause_workers",
            "miner_id": "miner-1",
            "workers": []
        }),
    );
    let response = common::http_request(
        addr,
        "POST",
        &format!("/api/lab/v1/runs/{run_id}/commands"),
        Some(&command.to_string()),
    )
    .unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(response.json()["state"], "succeeded");

    // Duplicate command through HTTP returns the stored outcome.
    let duplicate = common::http_request(
        addr,
        "POST",
        &format!("/api/lab/v1/runs/{run_id}/commands"),
        Some(&command.to_string()),
    )
    .unwrap();
    assert_eq!(duplicate.status, 200);
    assert_eq!(duplicate.json()["state"], "succeeded");
    assert_eq!(engine.command_log().entries().len(), 1);

    let mut sse = common::SseReader::open(addr, &run_id, cursor).unwrap();
    let resume = common::command(
        &run_id,
        serde_json::json!({
            "type": "resume_workers",
            "miner_id": "miner-1",
            "workers": []
        }),
    );
    let response = common::http_request(
        addr,
        "POST",
        &format!("/api/lab/v1/runs/{run_id}/commands"),
        Some(&resume.to_string()),
    )
    .unwrap();
    assert_eq!(response.json()["state"], "succeeded");
    let (kind, event) = sse
        .next_event()
        .unwrap()
        .expect("SSE delivered the command observation");
    // Ordinary evidence events use the generic SSE type `message`; the
    // specific kind rides in the payload so any additive kind is delivered
    // without a client update.
    assert_eq!(kind, "message");
    assert_eq!(event["payload"]["kind"], "command");
    let last_event_id = event["event_id"].as_u64().unwrap();
    drop(sse);
    // A dropped client socket is noticed on the next failed write, so post
    // assert commands (unique ids) until the evidence store has reclaimed the
    // subscriber slot, within a deadline. Observable condition, no sleep.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut probe = 0;
    while engine.evidence().subscriber_count() > 0 && std::time::Instant::now() < deadline {
        let disconnect = common::command(
            &run_id,
            serde_json::json!({
                "type": "assert",
                "assertion_id": format!("assert-disconnect-{probe}")
            }),
        );
        common::http_request(
            addr,
            "POST",
            &format!("/api/lab/v1/runs/{run_id}/commands"),
            Some(&disconnect.to_string()),
        )
        .unwrap();
        probe += 1;
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(engine.evidence().subscriber_count(), 0);
    // The disconnect does not stop the run or the HTTP view.
    assert_eq!(engine.manifest_snapshot().state, spc_lab::RunState::Running);
    let still = common::http_request(
        addr,
        "GET",
        &format!("/api/lab/v1/runs/{run_id}/snapshot?cursor={cursor}"),
        None,
    )
    .unwrap();
    assert_eq!(still.status, 200);

    // Reattach after the last event: no duplicate delivery.
    let mut again = common::SseReader::open(addr, &run_id, last_event_id).unwrap();
    let next = common::command(
        &run_id,
        serde_json::json!({
            "type": "assert",
            "assertion_id": "assert-started"
        }),
    );
    common::http_request(
        addr,
        "POST",
        &format!("/api/lab/v1/runs/{run_id}/commands"),
        Some(&next.to_string()),
    )
    .unwrap();
    let (_, event) = again.next_event().unwrap().unwrap();
    assert!(event["event_id"].as_u64().unwrap() > last_event_id);

    // A cursor older than the retained tail asks for a resync, never a skip.
    let resync_dir = common::TestDir::new("http-resync");
    let resync_engine = common::prepare_engine_with_capacity(
        &resync_dir,
        "scenario_v1_one_pool_two_miners.json",
        6,
    );
    resync_engine.start().unwrap();
    let resync_server =
        HttpServer::start(resync_engine.clone(), "127.0.0.1:0", WebAssets::fallback()).unwrap();
    let resync_run = resync_engine.run_id().to_string();
    let mut stale = common::SseReader::open(resync_server.addr(), &resync_run, 1).unwrap();
    let (kind, resync) = stale.next_event().unwrap().unwrap();
    assert_eq!(kind, "resync");
    assert!(resync["snapshot_cursor"].as_u64().unwrap() > 1);
    resync_engine.stop().unwrap();
    resync_server.shutdown();

    engine.stop().unwrap();
    let manifest = common::http_request(
        addr,
        "GET",
        &format!("/api/lab/v1/runs/{run_id}/artifacts/shareable/manifest.json"),
        None,
    )
    .unwrap();
    assert_eq!(manifest.status, 200);
    let private = common::http_request(
        addr,
        "GET",
        &format!("/api/lab/v1/runs/{run_id}/artifacts/private/evidence.jsonl"),
        None,
    )
    .unwrap();
    assert_eq!(private.status, 403);
    assert!(engine
        .manifest_snapshot()
        .artifact_paths
        .shareable_manifest
        .exists());
    server.shutdown();
}

#[test]
fn http_refuses_a_non_loopback_bind() {
    let dir = common::TestDir::new("http-nonloopback");
    let engine = common::prepare_engine(&dir, "scenario_v1_one_pool_two_miners.json");
    let error = HttpServer::start(engine, "0.0.0.0:0", WebAssets::fallback()).unwrap_err();
    assert!(error.to_string().contains("loopback"));
}

#[test]
fn typed_command_precondition_is_reported_over_http() {
    let dir = common::TestDir::new("http-precondition");
    let engine = common::prepare_engine(&dir, "scenario_v1_one_pool_two_miners.json");
    engine.start().unwrap();
    let server = HttpServer::start(engine.clone(), "127.0.0.1:0", WebAssets::fallback()).unwrap();
    let run_id = engine.run_id().to_string();
    let command = common::command(
        &run_id,
        serde_json::json!({
            "type": "remove_miner",
            "miner_id": "not-a-miner"
        }),
    );
    let response = common::http_request(
        server.addr(),
        "POST",
        &format!("/api/lab/v1/runs/{run_id}/commands"),
        Some(&command.to_string()),
    )
    .unwrap();
    assert_eq!(response.json()["state"], "rejected_precondition");
    engine.stop().unwrap();
    server.shutdown();
}
