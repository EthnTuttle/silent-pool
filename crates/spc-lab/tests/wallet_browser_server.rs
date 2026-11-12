//! Opt-in CDP host using the actual Rust controller, auth, SSE and queue adapter.
//! Only node/miner/agent inputs are synthetic and explicitly marked fixtures.
mod common;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use spc_lab::http::{HttpServer, WebAssets};

struct FixtureGuard {
    engines: Vec<Arc<spc_lab::RunEngine>>,
    stop: Arc<AtomicBool>,
}

impl Drop for FixtureGuard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        for engine in &self.engines {
            let _ = engine.stop();
        }
    }
}

fn private_json(path: &Path, value: &Value) {
    let temp = path.with_extension("tmp");
    std::fs::write(&temp, serde_json::to_vec(value).unwrap()).unwrap();
    std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::rename(temp, path).unwrap();
}

fn control(root: &Path) -> Value {
    std::fs::read(root.join("control.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| json!({}))
}

fn config() -> spc_lab::LabConfig {
    let mut config = common::fixture_config();
    let bin = PathBuf::from(std::env::var("SPC_LAB_UI_ACTOR_BIN").unwrap());
    for adapter in config
        .nodes
        .values_mut()
        .chain(config.pools.values_mut())
        .chain(config.miners.values_mut())
    {
        adapter.program = bin.clone();
    }
    for adapter in [&mut config.redis, &mut config.postgres]
        .into_iter()
        .flatten()
    {
        adapter.program = bin.clone();
    }
    config
        .miners
        .get_mut("fixture-miner")
        .unwrap()
        .args
        .extend([
            "--snapshot-worker".into(),
            "w-alice-1".into(),
            "--snapshot-worker".into(),
            "w-bob-1".into(),
        ]);
    config
}

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

#[test]
#[ignore = "owned bounded CDP fixture; requires SPC_LAB_UI_ROOT and actor binary"]
fn serve_ui() {
    let root = PathBuf::from(std::env::var("SPC_LAB_UI_ROOT").unwrap());
    std::fs::create_dir_all(&root).unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let dir = common::TestDir::new("ui-compiled-expanded");
    let legacy_dir = common::TestDir::new("ui-compiled-legacy");
    let mut scenario = common::fixture_scenario("scenario_v1_one_pool_two_miners.json");
    scenario.expanded_access = true;
    let engine = common::prepare_engine_with(&dir, scenario, config());
    let legacy = common::prepare_engine_with(
        &legacy_dir,
        common::fixture_scenario("scenario_v1_one_pool_two_miners.json"),
        config(),
    );
    let stop = Arc::new(AtomicBool::new(false));
    let _cleanup = FixtureGuard {
        engines: vec![Arc::clone(&engine), Arc::clone(&legacy)],
        stop: Arc::clone(&stop),
    };
    engine.start().unwrap();
    legacy.start().unwrap();
    let mut providers = Vec::new();
    let mut datadirs = Vec::new();
    let mut bindings = Vec::new();
    for (index, owner) in spc_lab::access::derive_user_bindings(&engine.scenario)
        .unwrap()
        .into_iter()
        .enumerate()
    {
        let datadir = dir.join(&format!("private/ui-wallet-{}", owner.identity_id));
        std::fs::create_dir(&datadir).unwrap();
        std::fs::set_permissions(&datadir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let front = engine
            .scenario
            .fronts
            .iter()
            .find(|f| f.pool_id == owner.pool_id)
            .unwrap();
        engine
            .wallet_registry
            .lock()
            .unwrap()
            .bind(
                &engine.scenario,
                &engine.run_dir,
                &owner.identity_id,
                &front.id,
                listener.local_addr().unwrap(),
                datadir.clone(),
            )
            .unwrap();
        datadirs.push(json!({"identity_id":owner.identity_id,"datadir":datadir}));
        bindings.push((
            owner.identity_id.clone(),
            front.id.clone(),
            listener.local_addr().unwrap(),
            datadir.clone(),
        ));
        let stop = Arc::clone(&stop);
        let root = root.clone();
        providers.push(std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                let control = control(&root);
                let timestamp = spc_lab::command::now_ms();
                let rows = if control["empty_inputs"] == true { json!([]) } else {
                    json!([{"id": if index == 0 { "ab".repeat(36) } else { "cd".repeat(36) },"amount_sats":"1000","state":"spendable","expiry_height":1000,"rail":"derived"}])
                };
                private_json(&datadir.join("ui-inputs.json"), &json!({"schema":"spc-agent-private-inputs/v1","updated_at_ms":timestamp,"inputs":rows}));
                if let Ok((mut socket, _)) = listener.accept() {
                    socket.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
                    let mut request = [0u8; 4096];
                    let _ = socket.read(&mut request);
                    let status = json!({"state":"connected","registered":true,"registration_live_until_unix_ms":timestamp+30000,
                        "updated_at": control["updated_at"].as_str().unwrap_or("2026-11-09T00:00:00Z"),
                        "spendable_sats":9007199254740993u64,"pending_sats":null,"needs_refresh_sats":1000,
                        "last_error":"<img src=x onerror=alert(1)> TEST-ONLY private source error",
                        "next_refresh_height":null,"min_expiry_height":1000,"last_scan_height":100});
                    let body = status.to_string();
                    let _ = write!(socket, "HTTP/1.0 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}", body.len(), body);
                }
                if let Some(result) = control["result"].as_str() {
                    if let Ok(files) = std::fs::read_dir(datadir.join("commands")) {
                        for file in files.flatten() {
                            let path = file.path();
                            if path.extension().is_some_and(|e| e == "json") {
                                let command: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                                private_json(&path.with_extension("result"), &json!({"operation_id":command["operation_id"],"state":result}));
                            }
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }));
    }
    let server = HttpServer::start(Arc::clone(&engine), "127.0.0.1:0", assets()).unwrap();
    let legacy_server = HttpServer::start(Arc::clone(&legacy), "127.0.0.1:0", assets()).unwrap();
    private_json(
        &root.join("ready.json"),
        &json!({"url":server.url(),"legacy_url":legacy_server.url(),"run_id":engine.run_id(),"run_dir":dir.path(),"wallets":datadirs}),
    );
    println!("compiled UI fixture ready");
    let deadline = Instant::now() + Duration::from_secs(180);
    let mut registry_missing = false;
    while Instant::now() < deadline && !root.join("stop").exists() {
        let missing = control(&root)["registry_unavailable"] == true;
        if missing && !registry_missing {
            *engine.wallet_registry.lock().unwrap() =
                spc_lab::wallet_projection::WalletRegistry::default();
        } else if !missing && registry_missing {
            for (identity, front, status, datadir) in &bindings {
                engine
                    .wallet_registry
                    .lock()
                    .unwrap()
                    .bind(
                        &engine.scenario,
                        &engine.run_dir,
                        identity,
                        front,
                        *status,
                        datadir.clone(),
                    )
                    .unwrap();
            }
        }
        registry_missing = missing;
        std::thread::sleep(Duration::from_millis(100));
    }
    stop.store(true, Ordering::SeqCst);
    for provider in providers {
        provider.join().unwrap();
    }
    engine.stop().unwrap();
    legacy.stop().unwrap();
    server.shutdown();
    legacy_server.shutdown();
    // Preserve private proof/journal inputs for this owned fixture only.
    std::mem::forget(dir);
    std::mem::forget(legacy_dir);
}
