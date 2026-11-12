//! Real HTTP/claim/engine/log paths with explicitly inert owned pipe peers.
//! These are stage/authorization tests, not native bootstrap evidence.
use super::*;
use crate::command::traffic_audit::{HookAction, HookKey, HookMode, Session};
use crate::command::traffic_budget::BudgetOperation;
use crate::http::{HttpServer, WebAssets};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};

struct Peer(std::process::Child);
impl Drop for Peer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn prepared(enabled: bool) -> (Arc<RunEngine>, Peer, PathBuf) {
    let mut s: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/scenario_v1_one_pool_two_miners.json"
    ))
    .unwrap();
    s["run"]["id"] = serde_json::json!(CommandId::generate().to_string());
    s["expanded_access"] = serde_json::json!(true);
    s["traffic"] = serde_json::json!({"version":1,"required_capability":"spc.node.regtest.mempool_rpc.v1","protocol":{"control":"spc-lab-tx-control/v1","history_version":3},
        "bootstrap":{"enabled":true,"mining_mode":"native","funding_blocks":101,"distribution_outputs_per_wallet":8,"excluded_from_statistics":true},
        "driver":{"actors":[{"id":"txd-0","network_wallets":8,"user_payment_wallets":2,"hosts_bootstrap_funding":true}],"user_payment_wallets":2,"history":"traffic/"},
        "rate":{"ordinary_tx_per_sec":0.2,"pacing":"fixed_jitter"},"fees":{"rate_sats_vb":{"min":1,"max":20},"max_fee_sats_per_tx":100000},"sizes":{"classes_vb":[{"vb":220,"mix":1.0}],"max_vb":1000},
        "spend_types":["p2pkh"],"ark":{"enabled":false,"registration_before_mining":true,"chain_effect_budget_tx_per_sec":0.0},"caps":{"max_inflight_broadcasts":1,"max_unknown_retries":3,"max_mempool_backlog_tx":1000,"reorg_resubmit_max":3},"pause_semantics":"no_broadcast_after_ack"});
    if enabled {
        s["traffic"]["lab_acceptance"] = serde_json::json!(true);
    }
    s["phases"] = serde_json::json!([{"id":"bootstrap","commands":[{"kind":{"type":"bootstrap_fund","driver_id":"txd-0","node_id":s["nodes"][0]["id"]}}]}]);
    let scenario: Scenario = serde_json::from_value(s).unwrap();
    let adapter:crate::config::AdapterConfig=serde_json::from_value(serde_json::json!({"program":"/bin/true","args":[],"fixture":true,"capabilities":["fixture.scripted","spc.node.regtest.mempool_rpc.v1","spc.pool.work_unit_v2","spc.pool.network_block_work"],"effective_profile":{
        "implementation":"fixture-node","network":"regtest","consensus_profile_id":"fixture.regtest.retarget","profile_fingerprint":null,"pow_limit_bits":"207fffff","retarget_floor_bits":"1e7fffff","initial_target_bits":"1e7fffff","regtest_pow_bits_overlay":"1e7fffff","target_spacing_secs":10,"target_timespan_secs":240,"retarget_interval_blocks":24,"no_pow_retargeting":false,"allow_min_difficulty_blocks":false,"minimum_difficulty_time_gap_secs":null,"subsidy_initial_sats":"5000000000","subsidy_halving_interval_blocks":150,"coinbase_maturity_blocks":100,"witness_commitment_supported":true,"tdp_session_cap":8},"rpc_auth":null,"env":{}})).unwrap();
    let mut config = LabConfig::default();
    config.nodes.insert("fixture-node".into(), adapter.clone());
    config.pools.insert("fixture-pool".into(), adapter.clone());
    config
        .miners
        .insert("fixture-miner".into(), adapter.clone());
    config.redis = Some(adapter.clone());
    config.postgres = Some(adapter.clone());
    let mut native = adapter;
    native.fixture = false;
    config.traffic_probe = Some(native.clone());
    config.tx_driver = Some(native);
    let root =
        PathBuf::from("/tmp/opencode").join(format!("acceptance-stage-{}", CommandId::generate()));
    std::fs::create_dir(&root).unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let engine = RunEngine::prepare(
        scenario,
        EngineOptions {
            run_dir: root.clone(),
            config,
            base_revision: "stage-test".into(),
            dirty_tree_hash: "fixture".into(),
            owner_instance: "stage-test".into(),
            evidence_capacity: 1024,
            namespaces: BTreeMap::new(),
            nsenter: None,
            user_namespace: None,
            user_namespace_owner: None,
            namespace_owners: BTreeMap::new(),
            namespace_broker: None,
            lifetime_helper: None,
            addresses: BTreeMap::new(),
            link_backend: None,
            operator_ui: false,
        },
    )
    .unwrap();
    let mut peer = Peer(
        Command::new("/usr/bin/python3")
            .args(["-c", "import time; time.sleep(60)"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let gate = RpcGate::new(BTreeMap::new(), 0)
        .with_audit(
            engine.command_log.clone(),
            Session {
                run_id: engine.run_id.to_string(),
                driver_id: "txd-0".into(),
                incarnation: "stage-peer".into(),
                controller_sha256: crate::supervisor::hash_file(&std::env::current_exe().unwrap())
                    .unwrap(),
                actor_sha256: crate::supervisor::hash_file(Path::new("/usr/bin/python3")).unwrap(),
                acceptance_enabled: enabled,
            },
        )
        .unwrap();
    let control = TxDriverControl::spawn(
        ActorId::trusted("txd-0"),
        "stage-peer".into(),
        peer.0.stdin.take().unwrap(),
        peer.0.stdout.take().unwrap(),
        gate,
    )
    .unwrap();
    engine
        .traffic
        .lock()
        .unwrap()
        .drivers
        .insert(ActorId::trusted("txd-0"), Arc::new(control));
    engine.state.lock().unwrap().started = true;
    (engine, peer, root)
}
fn submit_http(
    server: &HttpServer,
    engine: &RunEngine,
    claim: &crate::access::AccessClaim,
    kind: CommandKind,
) -> Value {
    let cmd = LabCommand::new(engine.run_id.clone(), claim.actor.clone(), kind);
    let body = serde_json::to_vec(&cmd).unwrap();
    let mut socket = std::net::TcpStream::connect(server.addr()).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    write!(socket,"POST /api/lab/v1/runs/{}/commands HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",engine.run_id,claim.token,body.len()).unwrap();
    socket.write_all(&body).unwrap();
    let mut bytes = Vec::new();
    socket.read_to_end(&mut bytes).unwrap();
    let split = bytes.windows(4).position(|b| b == b"\r\n\r\n").unwrap();
    serde_json::from_slice(&bytes[split + 4..]).unwrap()
}

#[test]
fn early_hook_http_scope_default_off_and_original_paused_budget_are_preserved() {
    for enabled in [false, true] {
        for stage in [TrafficStage::Provisioned, TrafficStage::Bootstrapping] {
            let (engine, peer, root) = prepared(enabled);
            {
                let mut m = engine.manifest.lock().unwrap();
                m.traffic_stage = Some(stage);
                m.state = crate::manifest::RunState::Running;
                m.save(&engine.manifest_path).unwrap();
            }
            let server =
                HttpServer::start(engine.clone(), "127.0.0.1:0", WebAssets::fallback()).unwrap();
            let claims = engine.access.as_ref().unwrap().claims();
            let operator = claims.iter().find(|c| c.is_operator()).unwrap();
            let user = claims.iter().find(|c| c.is_user()).unwrap();
            let original = CommandId::trusted("bootstrap-original");
            let bootstrap = LabCommand::new(
                engine.run_id.clone(),
                ActorId::trusted("controller"),
                CommandKind::BootstrapFund {
                    driver_id: ActorId::trusted("txd-0"),
                    node_id: engine.scenario.nodes[0].id.clone(),
                },
            )
            .with_id(original.clone());
            engine.command_log.begin(&bootstrap).unwrap();
            let now = spc_lab_txd::bootstrap::monotonic_ms().unwrap();
            let boot = spc_lab_txd::bootstrap::boot_id().unwrap();
            engine
                .command_log
                .start_hash_budget(BudgetOperation {
                    operation_id: original.clone(),
                    driver_id: "txd-0".into(),
                    boot_id: boot.clone(),
                    deadline_monotonic_ms: now + 240000,
                    total_attempts: 1000,
                    hashes_per_second: 40,
                    cancel_grace_secs: 10,
                })
                .unwrap();
            engine
                .command_log
                .reserve_hashes(&original, "stage-peer", 1, 1, &boot, now)
                .unwrap();
            let native = OperationControl::default();
            native.pause();
            engine.traffic.lock().unwrap().operation = Some((original.clone(), native.clone()));
            let before =
                serde_json::to_value(engine.command_log.hash_budget_snapshot(&original).unwrap())
                    .unwrap();
            let target = format!(
                "bootstrap/{}/00000000",
                crate::command::traffic_audit::hash(original.as_str().as_bytes())
            );
            let kind = CommandKind::TrafficAcceptance {
                key: HookKey {
                    driver_id: "txd-0".into(),
                    incarnation: "stage-peer".into(),
                    command_id: target,
                },
                control: HookAction::Arm {
                    mode: HookMode::HoldBeforeIntent,
                    timeout_ms: 120000,
                },
            };
            let seq = engine.command_log.last_seq();
            let denied = submit_http(&server, &engine, user, kind.clone());
            assert_ne!(denied["state"], "succeeded");
            assert_eq!(engine.command_log.last_seq(), seq);
            // The real HTTP command path must bypass this mutex while retaining
            // its narrow authorization; otherwise the bounded request fails.
            let guard = engine.command_mutex.lock().unwrap();
            let outcome = submit_http(&server, &engine, operator, kind);
            drop(guard);
            if enabled {
                assert_eq!(outcome["state"], "succeeded", "{outcome}");
            } else {
                assert!(
                    outcome["detail"]
                        .as_str()
                        .unwrap()
                        .contains("traffic_acceptance_disabled"),
                    "{outcome}"
                );
            }
            assert!(native.paused());
            assert_eq!(
                before,
                serde_json::to_value(engine.command_log.hash_budget_snapshot(&original).unwrap())
                    .unwrap()
            );
            let normal = submit_http(
                &server,
                &engine,
                operator,
                CommandKind::ResumeTxDriver {
                    driver_id: ActorId::trusted("txd-0"),
                },
            );
            assert!(normal["detail"]
                .as_str()
                .unwrap()
                .contains("traffic_not_active"));
            let version_case = CommandKind::AcceptanceVersionRejectionV1 {
                user: engine.scenario.identities[0].id.clone(),
                destination_address: "bcrt1qw508d6qejxtdg4y5r3zarvary0c5xw7kygt080".into(),
                amount_sats: 100000,
                fee_rate_sats_vb: 3,
            };
            let early = submit_http(&server, &engine, operator, version_case.clone());
            assert!(early["detail"]
                .as_str()
                .unwrap()
                .contains("traffic_not_active"));
            engine.manifest.lock().unwrap().traffic_stage = Some(TrafficStage::Active);
            let seq = engine.command_log.last_seq();
            let denied = submit_http(&server, &engine, user, version_case.clone());
            assert_ne!(denied["state"], "succeeded");
            assert_eq!(engine.command_log.last_seq(), seq);
            // In-process/controller ownership cannot impersonate the explicit
            // operator-owned constructor command, even when the lab flag is on.
            let denied = engine.submit(LabCommand::new(
                engine.run_id.clone(),
                ActorId::trusted("controller"),
                version_case.clone(),
            ));
            assert!(denied.detail.contains("operator_or_enablement"));
            if !enabled {
                let denied = submit_http(&server, &engine, operator, version_case);
                assert!(denied["detail"]
                    .as_str()
                    .unwrap()
                    .contains("operator_or_enablement"));
            }
            drop(server);
            drop(peer);
            drop(engine);
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}

#[test]
fn constructor_wire_is_closed_and_selection_changes_original_command_digest() {
    let user = crate::ids::IdentityId::trusted("id-alice");
    let selected = CommandKind::AcceptanceVersionRejectionV1 {
        user: user.clone(),
        destination_address: "address".into(),
        amount_sats: 100000,
        fee_rate_sats_vb: 3,
    };
    let normal = CommandKind::SendUserPayment {
        user,
        destination_address: "address".into(),
        amount_sats: 100000,
        fee_rate_sats_vb: 3,
    };
    let run = crate::ids::RunId::trusted("run");
    let actor = ActorId::trusted("operator");
    let id = CommandId::trusted("same-original");
    let selected_cmd =
        LabCommand::new(run.clone(), actor.clone(), selected.clone()).with_id(id.clone());
    let normal_cmd = LabCommand::new(run, actor, normal).with_id(id);
    assert_ne!(
        crate::command::CommandLog::payload_hash(&selected_cmd),
        crate::command::CommandLog::payload_hash(&normal_cmd)
    );
    let mut raw = serde_json::to_value(selected).unwrap();
    raw["version"] = serde_json::json!(4);
    assert!(serde_json::from_value::<CommandKind>(raw).is_err());
}
