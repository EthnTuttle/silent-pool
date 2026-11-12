//! Controller adapter tests only: private source fixtures, no funded wallet.
mod common;

use std::os::unix::fs::PermissionsExt;

use serde_json::json;
use spc_lab::access::{check_command_scope, AccessClaim, AuthContext};
use spc_lab::command::{now_ms, CommandKind, LabCommand, OutcomeState};
use spc_lab::wallet_control::{apply, operation_id};
use spc_lab::wallet_projection::{inputs, project_status, WalletBinding};

fn private_write(path: &std::path::Path, value: &serde_json::Value) {
    std::fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
}

fn binding(dir: &common::TestDir) -> WalletBinding {
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    WalletBinding {
        identity_id: "alice".parse().unwrap(),
        pool_id: "pool-a".parse().unwrap(),
        status_addr: "127.0.0.1:1".parse().unwrap(),
        datadir: dir.path().to_path_buf(),
    }
}

fn refresh() -> LabCommand {
    LabCommand::new(
        "test-run".parse().unwrap(),
        "user/alice".parse().unwrap(),
        CommandKind::WalletRefresh {
            identity_id: "alice".parse().unwrap(),
            vtxo_ids: vec!["ab".repeat(36)],
        },
    )
}

#[test]
fn exact_status_projection_never_relays_errors_keys_or_proofs() {
    let value = json!({"state":"dormant","registered":true,"updated_at":"2026-11-09T00:00:00Z",
        "spendable_sats":u64::MAX,"pending_sats":null,"needs_refresh_sats":0,
        "last_error":"postgres://secret@host/private/path", "xprv":"secret", "proof":"raw"});
    let projected = project_status(&value).unwrap();
    assert_eq!(projected["spendable_sats"], u64::MAX.to_string());
    assert!(projected["pending_sats"].is_null());
    assert_eq!(projected["needs_refresh_sats"], "0");
    assert_eq!(projected["state"], "dormant");
    let text = projected.to_string();
    for forbidden in ["secret", "postgres", "xprv", "proof", "/private"] {
        assert!(!text.contains(forbidden));
    }
}

#[test]
fn queue_submission_is_not_completion_and_unknown_never_requeues() {
    let dir = common::TestDir::new("wallet-ui-queue");
    let binding = binding(&dir);
    let command = refresh();
    assert_eq!(
        apply(Some(binding.clone()), &command, false).state,
        OutcomeState::RejectedPrecondition
    );
    assert!(!dir.join("commands").exists());
    private_write(
        &dir.join("ui-inputs.json"),
        &json!({"schema":"spc-agent-private-inputs/v1","updated_at_ms":now_ms(),
        "inputs":[{"id":"ab".repeat(36),"amount_sats":"1000","state":"spendable","expiry_height":1000,"rail":"derived"}]}),
    );
    assert_eq!(
        apply(Some(binding.clone()), &command, false).state,
        OutcomeState::Running
    );
    let queue = dir.join(&format!("commands/{}.json", operation_id(&command)));
    let bytes = std::fs::read(&queue).unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(payload["action"], json!({"kind":"refresh"}));
    assert_eq!(payload["vtxo_ids"], json!(["ab".repeat(36)]));
    assert_eq!(payload.as_object().unwrap().len(), 3);
    assert_eq!(
        apply(Some(binding.clone()), &command, true).state,
        OutcomeState::Unknown
    );
    assert_eq!(std::fs::read(&queue).unwrap(), bytes);
    let result = dir.join(&format!("commands/{}.result", operation_id(&command)));
    private_write(
        &result,
        &json!({"operation_id":operation_id(&command),"state":"stock_submitted"}),
    );
    assert_eq!(
        apply(Some(binding.clone()), &command, true).state,
        OutcomeState::Running
    );
    private_write(
        &result,
        &json!({"operation_id":operation_id(&command),"state":"round_confirmed"}),
    );
    assert_eq!(
        apply(Some(binding.clone()), &command, true).state,
        OutcomeState::Succeeded
    );
    assert_eq!(std::fs::read(&queue).unwrap(), bytes);
    for terminal in ["rejected_no_effect", "noop"] {
        private_write(
            &result,
            &json!({"operation_id":operation_id(&command),"state":terminal}),
        );
        assert_eq!(
            apply(Some(binding.clone()), &command, true).state,
            OutcomeState::RejectedPrecondition
        );
    }
}

#[test]
fn private_input_inventory_rejects_stale_unknown_fields_symlinks_and_public_mode() {
    let dir = common::TestDir::new("wallet-ui-inputs");
    let binding = binding(&dir);
    let path = dir.join("ui-inputs.json");
    private_write(
        &path,
        &json!({"schema":"spc-agent-private-inputs/v1","updated_at_ms":0,"inputs":[]}),
    );
    assert!(inputs(&binding).is_err());
    private_write(
        &path,
        &json!({"schema":"spc-agent-private-inputs/v1","updated_at_ms":now_ms(),"inputs":[],"xprv":"never"}),
    );
    assert!(inputs(&binding).is_err());
    private_write(
        &path,
        &json!({"schema":"spc-agent-private-inputs/v1","updated_at_ms":now_ms(),"inputs":[]}),
    );
    assert!(inputs(&binding).unwrap().inputs.is_empty());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(inputs(&binding).is_err());
    std::fs::rename(&path, dir.join("target.json")).unwrap();
    std::os::unix::fs::symlink(dir.join("target.json"), &path).unwrap();
    assert!(inputs(&binding).is_err());
}

#[test]
fn every_wallet_kind_is_scoped_before_journal_and_replay() {
    let dir = common::TestDir::new("wallet-ui-scope");
    let mut scenario = common::fixture_scenario("scenario_v1_one_pool_two_miners.json");
    scenario.expanded_access = true;
    let engine = common::prepare_engine_with(&dir, scenario, common::fixture_config());
    let claims = engine.access().unwrap().claims();
    let own: AccessClaim = claims.iter().find(|c| c.is_user()).unwrap().clone();
    let other = claims
        .iter()
        .find(|c| c.is_user() && c.principal != own.principal)
        .unwrap();
    let auth = AuthContext {
        principal: own.principal.clone(),
        claim: own.clone(),
    };
    for kind in [
        CommandKind::WalletRefresh {
            identity_id: other.identity_id.clone().unwrap(),
            vtxo_ids: vec!["ab".repeat(36)],
        },
        CommandKind::WalletOffboard {
            identity_id: other.identity_id.clone().unwrap(),
            vtxo_ids: vec!["ab".repeat(36)],
            address: "invalid".into(),
        },
        CommandKind::WalletReadInputs {
            identity_id: other.identity_id.clone().unwrap(),
        },
    ] {
        let command = LabCommand::new(engine.run_id.clone(), own.actor.clone(), kind);
        // Existing secret outcome under a foreign principal cannot be replayed.
        engine
            .command_log
            .begin_scoped(&command, &other.principal)
            .unwrap();
        let seq = engine.command_log.last_seq();
        assert!(check_command_scope(&own, &command).is_err());
        let outcome = engine.submit_authenticated(&auth, command);
        assert_eq!(outcome.state, OutcomeState::RejectedPrecondition);
        assert_eq!(outcome.journal_seq, 0);
        assert_eq!(engine.command_log.last_seq(), seq);
    }
}

#[test]
fn http_wallets_are_private_and_user_filtered_before_serialization() {
    use spc_lab::http::{HttpServer, WebAssets};
    use std::io::{Read, Write};

    let dir = common::TestDir::new("wallet-ui-http");
    let mut scenario = common::fixture_scenario("scenario_v1_one_pool_two_miners.json");
    scenario.expanded_access = true;
    let engine = common::prepare_engine_with(&dir, scenario, common::fixture_config());
    for (index, binding) in spc_lab::access::derive_user_bindings(&engine.scenario)
        .unwrap()
        .into_iter()
        .enumerate()
    {
        let datadir = dir.join(&format!("private/test-wallet-{}", binding.identity_id));
        std::fs::create_dir(&datadir).unwrap();
        std::fs::set_permissions(&datadir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let front = &engine
            .scenario
            .fronts
            .iter()
            .find(|f| f.pool_id == binding.pool_id)
            .unwrap()
            .id;
        engine
            .wallet_registry
            .lock()
            .unwrap()
            .bind(
                &engine.scenario,
                &engine.run_dir,
                &binding.identity_id,
                front,
                format!("127.0.0.1:{}", index + 1).parse().unwrap(),
                datadir,
            )
            .unwrap();
    }
    let server = HttpServer::start(engine.clone(), "127.0.0.1:0", WebAssets::fallback()).unwrap();
    let get = |path: &str, token: Option<&str>| {
        let mut socket = std::net::TcpStream::connect(server.addr()).unwrap();
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let auth = token
            .map(|t| format!("Authorization: Bearer {t}\r\n"))
            .unwrap_or_default();
        write!(
            socket,
            "GET {path} HTTP/1.1\r\nHost: {}\r\n{auth}Connection: close\r\n\r\n",
            server.addr()
        )
        .unwrap();
        let mut body = String::new();
        socket.read_to_string(&mut body).unwrap();
        body
    };
    let claims = engine.access().unwrap().claims();
    let operator = claims.iter().find(|c| c.is_operator()).unwrap();
    let user = claims.iter().find(|c| c.is_user()).unwrap();
    let other = claims
        .iter()
        .find(|c| c.is_user() && c.principal != user.principal)
        .unwrap();
    let prefix = format!("/api/lab/v1/runs/{}", engine.run_id);
    assert!(get("/access.js", None).starts_with("HTTP/1.1 200"));
    assert!(get(&format!("{prefix}/wallets"), None).starts_with("HTTP/1.1 401"));
    assert!(get(&format!("{prefix}/inventory"), Some(&user.token)).starts_with("HTTP/1.1 403"));
    assert!(get(&format!("{prefix}/inventory"), Some(&operator.token)).starts_with("HTTP/1.1 200"));
    let own = get(&format!("{prefix}/wallets"), Some(&user.token));
    assert!(own.starts_with("HTTP/1.1 200"));
    assert!(own.contains(user.identity_id.as_ref().unwrap().as_str()));
    assert!(!own.contains(other.identity_id.as_ref().unwrap().as_str()));
    assert!(!own.contains(&user.token));
    engine.access().unwrap().revoke(&user.principal);
    assert!(get(&format!("{prefix}/wallets"), Some(&user.token)).starts_with("HTTP/1.1 401"));
}

#[test]
fn derived_offboard_uses_homogeneous_owned_inputs_and_strict_network() {
    let dir = common::TestDir::new("wallet-ui-derived-offboard");
    let binding = binding(&dir);
    let secp = bitcoin::secp256k1::Secp256k1::new();
    let secret = bitcoin::secp256k1::SecretKey::from_slice(&[1; 32]).unwrap();
    let key = bitcoin::secp256k1::Keypair::from_secret_key(&secp, &secret);
    let address = |network| {
        bitcoin::Address::p2tr(&secp, key.x_only_public_key().0, None, network).to_string()
    };
    let mut command = refresh();
    command.kind = CommandKind::WalletOffboard {
        identity_id: binding.identity_id.clone(),
        vtxo_ids: vec!["ab".repeat(36)],
        address: address(bitcoin::Network::Bitcoin),
    };
    assert_eq!(
        apply(Some(binding.clone()), &command, false).state,
        OutcomeState::RejectedPrecondition
    );
    assert!(!dir.join("commands").exists());
    private_write(
        &dir.join("ui-inputs.json"),
        &json!({"schema":"spc-agent-private-inputs/v1","updated_at_ms":now_ms(),
        "inputs":[{"id":"ab".repeat(36),"amount_sats":"1000","state":"spendable","expiry_height":1000,"rail":"derived"},
                  {"id":"cd".repeat(36),"amount_sats":"1000","state":"spendable","expiry_height":1000,"rail":"normal"}]}),
    );
    command.kind = CommandKind::WalletOffboard {
        identity_id: binding.identity_id.clone(),
        vtxo_ids: vec!["ab".repeat(36), "cd".repeat(36)],
        address: address(bitcoin::Network::Regtest),
    };
    assert_eq!(
        apply(Some(binding.clone()), &command, false).state,
        OutcomeState::RejectedPrecondition
    );
    assert!(!dir.join("commands").exists());
    if let CommandKind::WalletOffboard { vtxo_ids, .. } = &mut command.kind {
        vtxo_ids.pop();
    }
    assert_eq!(
        apply(Some(binding.clone()), &command, false).state,
        OutcomeState::Running
    );
    let result = dir.join(&format!("commands/{}.result", operation_id(&command)));
    private_write(
        &result,
        &json!({"operation_id":operation_id(&command),"state":"round_confirmed"}),
    );
    assert_eq!(
        apply(Some(binding.clone()), &command, true).state,
        OutcomeState::Unknown
    );
    private_write(
        &result,
        &json!({"operation_id":operation_id(&command),"state":"offboard_confirmed"}),
    );
    assert_eq!(
        apply(Some(binding), &command, true).state,
        OutcomeState::Succeeded
    );
}
