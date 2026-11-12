//! Audit W1-W4: real engine/HTTP replay and append-only wallet resolution.
mod common;

use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;

use serde_json::json;
use spc_lab::access::{AccessClaim, AuthContext};
use spc_lab::command::{now_ms, CommandKind, LabCommand, OutcomeState};
use spc_lab::http::{HttpServer, WebAssets};
use spc_lab::wallet_control::{apply, operation_id};
use spc_lab::wallet_projection::{WalletBinding, WalletRegistry};

struct StopGuard(Option<Arc<spc_lab::RunEngine>>);
impl Drop for StopGuard {
    fn drop(&mut self) {
        if let Some(engine) = &self.0 {
            let _ = engine.stop();
        }
    }
}

fn private(path: &std::path::Path, value: &serde_json::Value) {
    std::fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
}

fn auth(claim: &AccessClaim) -> AuthContext {
    AuthContext {
        principal: claim.principal.clone(),
        claim: claim.clone(),
    }
}

fn setup() -> (
    common::TestDir,
    Arc<spc_lab::RunEngine>,
    AccessClaim,
    WalletBinding,
) {
    let dir = common::TestDir::new("wallet-audit-replay");
    let mut scenario = common::fixture_scenario("scenario_v1_one_pool_two_miners.json");
    scenario.expanded_access = true;
    let engine = common::prepare_engine_with(&dir, scenario, common::fixture_config());
    let claim = engine
        .access()
        .unwrap()
        .claims()
        .into_iter()
        .find(|c| c.is_user())
        .unwrap();
    let datadir = dir.join("private/owned-wallet");
    std::fs::create_dir(&datadir).unwrap();
    std::fs::set_permissions(&datadir, std::fs::Permissions::from_mode(0o700)).unwrap();
    private(
        &datadir.join("ui-inputs.json"),
        &json!({"schema":"spc-agent-private-inputs/v1","updated_at_ms":now_ms(),"inputs":[
        {"id":"ab".repeat(36),"amount_sats":"1000","state":"spendable","expiry_height":1000,"rail":"derived"},
        {"id":"cd".repeat(36),"amount_sats":"1000","state":"spendable","expiry_height":1000,"rail":"derived"}]}),
    );
    let binding = WalletBinding {
        identity_id: claim.identity_id.clone().unwrap(),
        pool_id: claim.pool_id.clone().unwrap(),
        status_addr: "127.0.0.1:1".parse().unwrap(),
        datadir,
    };
    bind(&engine, &binding);
    (dir, engine, claim, binding)
}

fn bind(engine: &spc_lab::RunEngine, binding: &WalletBinding) {
    let front = engine
        .scenario
        .fronts
        .iter()
        .find(|f| f.pool_id == binding.pool_id)
        .unwrap();
    engine
        .wallet_registry
        .lock()
        .unwrap()
        .bind(
            &engine.scenario,
            &engine.run_dir,
            &binding.identity_id,
            &front.id,
            binding.status_addr,
            binding.datadir.clone(),
        )
        .unwrap();
}

fn command(engine: &spc_lab::RunEngine, claim: &AccessClaim, id: &str) -> LabCommand {
    LabCommand::new(
        engine.run_id.clone(),
        claim.actor.clone(),
        CommandKind::WalletRefresh {
            identity_id: "id-alice".parse().unwrap(),
            vtxo_ids: vec![id.repeat(36)],
        },
    )
}

fn post(
    server: &HttpServer,
    command: &LabCommand,
    claim: &AccessClaim,
) -> (u16, serde_json::Value) {
    let body = serde_json::to_string(command).unwrap();
    let mut socket = std::net::TcpStream::connect(server.addr()).unwrap();
    socket
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    write!(socket, "POST /api/lab/v1/runs/{}/commands HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nOrigin: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", command.run_id, server.addr(), claim.token, server.url(), body.len(), body).unwrap();
    let mut response = String::new();
    socket.read_to_string(&mut response).unwrap();
    let (head, body) = response.split_once("\r\n\r\n").unwrap();
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    (
        status,
        serde_json::from_str(body).unwrap_or(serde_json::Value::Null),
    )
}

#[test]
fn user_and_operator_http_replay_after_attach_without_registry_stays_unknown() {
    let (_dir, engine, user, binding) = setup();
    let operator = engine
        .access()
        .unwrap()
        .claims()
        .into_iter()
        .find(|c| c.is_operator())
        .unwrap();
    let mut guard = StopGuard(Some(engine.clone()));
    engine.start().unwrap();
    let initial_server =
        HttpServer::start(engine.clone(), "127.0.0.1:0", WebAssets::fallback()).unwrap();
    let commands = [
        command(&engine, &user, "ab"),
        command(&engine, &operator, "cd"),
    ];
    for (claim, command) in [&user, &operator].into_iter().zip(&commands) {
        let (status, outcome) = post(&initial_server, command, claim);
        assert_eq!(status, 200);
        assert_eq!(outcome["state"], "running");
    }
    let path = engine.run_dir.clone();
    let queues: Vec<_> = commands
        .iter()
        .map(|c| {
            let p = binding
                .datadir
                .join(format!("commands/{}.json", operation_id(c)));
            let b = std::fs::read(&p).unwrap();
            (p, b)
        })
        .collect();
    engine.stop().unwrap();
    initial_server.shutdown();
    guard.0.take();
    drop(engine);
    let attached = spc_lab::RunEngine::attach(&path, "wallet-audit-adopt").unwrap();
    let server = HttpServer::start(attached.clone(), "127.0.0.1:0", WebAssets::fallback()).unwrap();
    for (claim, command) in [&user, &operator].into_iter().zip(&commands) {
        let before_seq = attached.command_log.last_seq();
        let (status, response) = post(&server, command, claim);
        assert_eq!(status, 200);
        assert_eq!(response["state"], "unknown");
        assert_eq!(response["command_id"], command.command_id.as_str());
        assert!(response["detail"]
            .as_str()
            .unwrap()
            .contains("retain original"));
        assert_eq!(attached.command_log.last_seq(), before_seq);
        let seq = attached.command_log.last_seq();
        assert_eq!(post(&server, command, claim).1["state"], "unknown");
        assert_eq!(attached.command_log.last_seq(), seq);
    }
    let seq = attached.command_log.last_seq();
    let mut forged = commands[0].clone();
    forged.actor = operator.actor.clone();
    assert_eq!(post(&server, &forged, &operator).1["journal_seq"], 0);
    assert_eq!(attached.command_log.last_seq(), seq);
    let mut wrong_pool = user.clone();
    wrong_pool.pool_id = Some("wrong-pool".parse().unwrap());
    let claim_path = path.join("private/access/id-alice.json");
    private(&claim_path, &serde_json::to_value(&wrong_pool).unwrap());
    assert_eq!(post(&server, &commands[0], &user).1["journal_seq"], 0);
    assert_eq!(attached.command_log.last_seq(), seq);
    private(&claim_path, &serde_json::to_value(&user).unwrap());
    let mut wrong_identity = commands[0].clone();
    wrong_identity.kind = CommandKind::WalletReadInputs {
        identity_id: "id-bob".parse().unwrap(),
    };
    assert_eq!(post(&server, &wrong_identity, &user).1["journal_seq"], 0);
    assert_eq!(attached.command_log.last_seq(), seq);
    let mut altered = commands[0].clone();
    altered.kind = CommandKind::WalletRefresh {
        identity_id: "id-alice".parse().unwrap(),
        vtxo_ids: vec!["ef".repeat(36)],
    };
    assert_eq!(
        post(&server, &altered, &user).1["state"],
        "rejected_conflict"
    );
    let read_only = spc_lab::RunEngine::attach_read_only(&path, "wallet-audit-read").unwrap();
    let before = std::fs::read(path.join("private/commands.jsonl")).unwrap();
    assert_eq!(
        read_only
            .submit_authenticated(&auth(&user), commands[0].clone())
            .state,
        OutcomeState::Unknown
    );
    assert_eq!(
        std::fs::read(path.join("private/commands.jsonl")).unwrap(),
        before
    );
    attached.access().unwrap().revoke(&user.principal);
    assert_eq!(post(&server, &commands[0], &user).0, 401);
    for (p, bytes) in queues {
        assert_eq!(std::fs::read(p).unwrap(), bytes);
    }
    assert_eq!(
        apply(None, &commands[0], false).state,
        OutcomeState::RejectedPrecondition
    );
    server.shutdown();
}

#[test]
fn torn_wallet_tail_on_reopen_cannot_authorize_a_new_queue_effect() {
    let (_dir, engine, user, binding) = setup();
    let old = command(&engine, &user, "ab");
    engine
        .command_log
        .begin_scoped(&old, &user.principal)
        .unwrap();
    let initial = apply(Some(binding.clone()), &old, false);
    engine
        .command_log
        .finish(&old.command_id, &initial)
        .unwrap();
    let path = engine.run_dir.clone();
    drop(engine);
    let journal = path.join("private/commands.jsonl");
    std::fs::OpenOptions::new()
        .append(true)
        .open(&journal)
        .unwrap()
        .write_all(b"{\"record\":\"finish\"")
        .unwrap();
    let bytes = std::fs::read(&journal).unwrap();
    let reopened = spc_lab::RunEngine::attach(&path, "wallet-torn-reopen").unwrap();
    assert!(!reopened.command_log.wallet_resolutions_available());
    let fresh = command(&reopened, &user, "cd");
    assert_eq!(
        reopened
            .submit_authenticated(&auth(&user), fresh.clone())
            .state,
        OutcomeState::Unknown
    );
    assert!(!binding
        .datadir
        .join(format!("commands/{}.json", operation_id(&fresh)))
        .exists());
    assert_eq!(
        reopened.submit_authenticated(&auth(&user), old).state,
        OutcomeState::Unknown
    );
    assert_eq!(std::fs::read(journal).unwrap(), bytes);
}

#[test]
fn confirmed_resolution_is_append_only_concurrent_and_survives_missing_provider_after_restart() {
    let (_dir, engine, user, binding) = setup();
    let command = command(&engine, &user, "ab");
    engine
        .command_log
        .begin_scoped(&command, &user.principal)
        .unwrap();
    let first = apply(Some(binding.clone()), &command, false);
    engine
        .command_log
        .finish(&command.command_id, &first)
        .unwrap();
    let queue = binding
        .datadir
        .join(format!("commands/{}.json", operation_id(&command)));
    let original = std::fs::read(&queue).unwrap();
    let result = binding
        .datadir
        .join(format!("commands/{}.result", operation_id(&command)));
    private(
        &result,
        &json!({"operation_id":operation_id(&command),"state":"round_confirmed"}),
    );
    let seq = engine.command_log.last_seq();
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let (engine, context, command) = (engine.clone(), auth(&user), command.clone());
            std::thread::spawn(move || engine.submit_authenticated(&context, command))
        })
        .collect();
    for thread in threads {
        let out = thread.join().unwrap();
        assert_eq!(out.state, OutcomeState::Succeeded);
        assert_eq!(out.journal_seq, seq + 1);
    }
    assert_eq!(engine.command_log.last_seq(), seq + 1);
    let different = spc_lab::CommandOutcome::new(
        command.command_id.clone(),
        OutcomeState::Failed,
        "cannot overwrite",
    );
    assert_eq!(
        engine
            .command_log
            .supersede_nonterminal(&command, Some(&user.principal), &different)
            .unwrap()
            .0
            .state,
        OutcomeState::Succeeded
    );
    assert_eq!(engine.command_log.last_seq(), seq + 1);
    assert_eq!(std::fs::read(&queue).unwrap(), original);
    std::fs::remove_file(result).unwrap();
    *engine.wallet_registry.lock().unwrap() = WalletRegistry::default();
    let path = engine.run_dir.clone();
    drop(engine);
    let reopened = spc_lab::RunEngine::attach(&path, "wallet-terminal-reopen").unwrap();
    assert_eq!(
        reopened
            .submit_authenticated(&auth(&user), command.clone())
            .state,
        OutcomeState::Succeeded
    );
    let readonly = spc_lab::RunEngine::attach_read_only(&path, "wallet-terminal-read").unwrap();
    assert_eq!(
        readonly.submit_authenticated(&auth(&user), command).state,
        OutcomeState::Succeeded
    );
    assert_eq!(reopened.command_log.last_seq(), seq + 1);
}

#[test]
fn malformed_operator_is_rejected_before_journal_or_private_queue_access() {
    let (_dir, engine, _, binding) = setup();
    let operator = engine
        .access()
        .unwrap()
        .claims()
        .into_iter()
        .find(|c| c.is_operator())
        .unwrap();
    let command = command(&engine, &operator, "zz");
    let seq = engine.command_log.last_seq();
    assert_eq!(
        engine.submit_authenticated(&auth(&operator), command).state,
        OutcomeState::RejectedPrecondition
    );
    assert_eq!(engine.command_log.last_seq(), seq);
    assert!(!binding.datadir.join("commands").exists());
}

#[test]
fn failed_link_removes_only_our_created_ui_temp() {
    let (_dir, engine, user, binding) = setup();
    let command = command(&engine, &user, "ab");
    let dir = binding.datadir.join("commands");
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let stem = operation_id(&command);
    std::os::unix::fs::symlink(dir.join("absent"), dir.join(format!("{stem}.json"))).unwrap();
    let result = dir.join(format!("{stem}.result"));
    std::fs::write(&result, b"preserve").unwrap();
    assert_eq!(
        apply(Some(binding), &command, false).state,
        OutcomeState::Unknown
    );
    assert!(!dir.join(format!("{stem}.ui.tmp")).exists());
    assert!(std::fs::symlink_metadata(dir.join(format!("{stem}.json")))
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(std::fs::read(result).unwrap(), b"preserve");
}

#[test]
fn persisted_registry_rebind_is_read_only_and_refuses_resource_or_config_mismatch() {
    use spc_lab::wallet_projection::PersistedWalletBinding;
    let (dir, engine, user, _) = setup();
    let parent = dir.join("private/wallet-id-alice");
    let state = parent.join("state");
    std::fs::create_dir_all(state.join("bark")).unwrap();
    for path in [&parent, &state, &state.join("bark")] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    for file in ["state.sqlite", "bark/db.sqlite"] {
        private(
            &state.join(file),
            &json!("TEST ONLY opaque database; registry must not parse or rewrite"),
        );
    }
    let secp = bitcoin::secp256k1::Secp256k1::new();
    let key = |byte| {
        bitcoin::secp256k1::PublicKey::from_secret_key(
            &secp,
            &bitcoin::secp256k1::SecretKey::from_slice(&[byte; 32]).unwrap(),
        )
    };
    let address =
        spc_crypto::address::encode_sp_address(spc_crypto::Network::Regtest, &key(1), &key(2))
            .unwrap();
    private(
        &parent.join("provision.json"),
        &json!({"schema":"spc.agent.provision/v1","network":"regtest","sp_address":address}),
    );
    let config = json!({"upstream":{"address":"127.0.0.1:1234","authority_public_key":"fixture-pinned-key","sp_address":address},
        "ark":{"network":"regtest","datadir":state,"status_listen":"127.0.0.1:4321"}});
    let config_path = parent.join("agent.toml");
    std::fs::write(&config_path, toml::to_string(&config).unwrap()).unwrap();
    std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let front_config_path = dir.join("private/pool-front-a1.toml");
    std::fs::write(&front_config_path, "[pool]\nnetwork = 'regtest'\nlisten = '127.0.0.1:1234'\nauthority_public_key = 'fixture-pinned-key'\n").unwrap();
    std::fs::set_permissions(&front_config_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let declared = |port| PersistedWalletBinding {
        identity_id: user.identity_id.clone().unwrap(),
        front_id: "front-a1".parse().unwrap(),
        status_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        front_addr: "127.0.0.1:1234".into(),
        front_config_path: front_config_path.clone(),
        config_path: config_path.clone(),
    };
    let original = std::fs::read(state.join("state.sqlite")).unwrap();
    let mut registry = WalletRegistry::default();
    assert!(registry
        .bind_persisted(&engine.scenario, &engine.run_dir, declared(4322))
        .is_err());
    assert!(registry.scoped(&user).is_empty());
    registry
        .bind_persisted(&engine.scenario, &engine.run_dir, declared(4321))
        .unwrap();
    assert_eq!(registry.scoped(&user).len(), 1);
    assert_eq!(std::fs::read(state.join("state.sqlite")).unwrap(), original);
    assert!(!state.join("commands").exists());
}
