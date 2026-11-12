use serde_json::json;
use spc_lab_txd::actor_config::{
    driver_component, ActorConfig, AllocationActor, AllocationDescription,
};
use spc_lab_txd::control::{
    parse_rpc_response, read_message, write_message, Message, RpcOutcome, UnknownClass, PROTOCOL,
};
use std::io::BufReader;
use std::process::{Command, Stdio};

#[path = "common/destinations.rs"]
mod destinations;
#[path = "common/mod.rs"]
mod independent;

struct Process {
    child: std::process::Child,
    input: std::process::ChildStdin,
    output: std::sync::mpsc::Receiver<std::io::Result<Message>>,
}
impl Process {
    /// T1-DEST offline responder adapted only to the private forwarding envelope.
    fn destination_response(&mut self, id: u64, captured: &mut Vec<String>) -> serde_json::Value {
        let genesis = bitcoin::blockdata::constants::genesis_block(bitcoin::Network::Regtest);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            assert!(std::time::Instant::now() < deadline);
            match self.receive() {
                Message::Response {
                    request_id, result, ..
                } if request_id == id => return result,
                Message::Event { .. } => {}
                Message::Request {
                    request_id,
                    operation,
                    payload,
                    ..
                } if operation == "rpc_forward" => {
                    let request = &payload["request"];
                    let result = match request["method"].as_str().unwrap() {
                        "getbestblockhash" => RpcOutcome::Ok {
                            result: json!(genesis.block_hash().to_string()),
                        },
                        "getblock" => RpcOutcome::Ok {
                            result: json!(hex::encode(bitcoin::consensus::serialize(&genesis))),
                        },
                        "sendrawtransaction" => {
                            captured.push(request["params"][0].as_str().unwrap().to_string());
                            RpcOutcome::unknown(UnknownClass::Transport)
                        }
                        method => panic!("unexpected fixture RPC {method}"),
                    };
                    write_message(
                        &mut self.input,
                        &Message::Response {
                            protocol: PROTOCOL.into(),
                            incarnation: "first".into(),
                            driver_id: "../a".into(),
                            request_id,
                            command_id: None,
                            result: json!({"request":request,"result":result}),
                        },
                    )
                    .unwrap();
                }
                Message::Request {
                    request_id,
                    operation,
                    ..
                } if operation == "acceptance_checkpoint" => {
                    // No delay hook is armed in this offline fixture. This
                    // response cannot select a construction version or grant
                    // core send admission; the version is already Signed.
                    write_message(
                        &mut self.input,
                        &Message::Response {
                            protocol: PROTOCOL.into(),
                            incarnation: "first".into(),
                            driver_id: "../a".into(),
                            request_id,
                            command_id: None,
                            result: json!({"status":"proceed"}),
                        },
                    )
                    .unwrap();
                }
                other => panic!("unexpected actor frame {other:?}"),
            }
        }
    }
    fn start(path: &std::path::Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_spc-lab-txd"))
            .args(["--config", path.to_str().unwrap(), "--incarnation", "first"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let mut output = BufReader::new(child.stdout.take().unwrap());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || loop {
            match read_message(&mut output) {
                Ok(Some(m)) => {
                    if tx.send(Ok(m)).is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    let _ = tx.send(Err(e));
                    break;
                }
            }
        });
        Self {
            child,
            input,
            output: rx,
        }
    }
    fn receive(&mut self) -> Message {
        self.output
            .recv_timeout(std::time::Duration::from_secs(3))
            .expect("bounded actor response")
            .unwrap()
    }
    fn send(
        &mut self,
        id: u64,
        command: Option<&str>,
        operation: &str,
        payload: serde_json::Value,
    ) {
        write_message(
            &mut self.input,
            &Message::Request {
                protocol: PROTOCOL.into(),
                incarnation: "first".into(),
                driver_id: "../a".into(),
                request_id: id,
                command_id: command.map(str::to_string),
                operation: operation.into(),
                payload,
            },
        )
        .unwrap();
    }
    fn response(&mut self, id: u64) -> serde_json::Value {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            assert!(
                std::time::Instant::now() < deadline,
                "bounded actor control loop"
            );
            match self.receive() {
                Message::Response {
                    request_id, result, ..
                } if request_id == id => return result,
                Message::Event { .. } => {}
                Message::Request {
                    request_id,
                    operation,
                    payload,
                    ..
                } if operation == "rpc_forward" => {
                    write_message(&mut self.input,&Message::Response {protocol:PROTOCOL.into(),incarnation:"first".into(),driver_id:"../a".into(),request_id,command_id:None,
                    result:json!({"request":payload["request"],"result":RpcOutcome::unknown(UnknownClass::Transport)})}).unwrap();
                }
                _ => panic!("unexpected actor frame"),
            }
        }
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn config(run: &std::path::Path) -> ActorConfig {
    ActorConfig {
        node_profile_binding: None,
        lab_acceptance: false,
        chain_policy: spc_lab_txd::observer::ChainPolicy {
            genesis: bitcoin::blockdata::constants::genesis_block(bitcoin::Network::Regtest)
                .block_hash(),
            pow_limit_bits: 0x207fffff,
            initial_target_bits: 0x207fffff,
            no_pow_retargeting: true,
            allow_min_difficulty_blocks: false,
            pow_target_spacing: 600,
            difficulty_adjustment_interval: 2016,
            regtest_pow_bits_overlay: 0x207fffff,
            subsidy_initial_sats: 5_000_000_000,
            subsidy_halving_interval: 150,
        },
        size_classes: vec![spc_lab_txd::actor_config::SizeClass {
            vbytes: 220,
            mix: 1.0,
        }],
        protocol: PROTOCOL.into(),
        history_version: 3,
        run_dir: run.into(),
        driver_id: "../a".into(),
        allocation: AllocationDescription {
            run_seed: 42,
            run_id: "run".into(),
            network: bitcoin::Network::Regtest,
            caps: Default::default(),
            users: vec!["alice".into(), "bob".into()],
            actors: vec![
                AllocationActor {
                    driver_id: "../a".into(),
                    network_wallets: 4,
                    user_payment_wallets: 1,
                    hosts_bootstrap_funding: true,
                    lookahead: 8,
                    fee_policy: Default::default(),
                },
                AllocationActor {
                    driver_id: "b/x".into(),
                    network_wallets: 4,
                    user_payment_wallets: 1,
                    hosts_bootstrap_funding: false,
                    lookahead: 8,
                    fee_policy: Default::default(),
                },
            ],
        },
        node_ids: vec!["node".into()],
        authority_node: "node".into(),
        ordinary_interval_ms: 10000,
        max_vbytes: 1000,
        max_mempool_backlog_tx: 100,
    }
}
#[test]
fn complete_allocation_and_roots_do_not_alias() {
    let dir = tempfile::tempdir().unwrap();
    let mut first = config(dir.path());
    let mut second = first.clone();
    second.driver_id = "b/x".into();
    let a = first.validate().unwrap();
    let b = second.validate().unwrap();
    assert_eq!(a.digest(), b.digest());
    assert_eq!(first.allocation.owner_of_user("bob").unwrap(), ("b/x", 1));
    let root_a = first.root().unwrap();
    let root_b = second.root().unwrap();
    assert_ne!(root_a, root_b);
    assert_eq!(driver_component("../a").unwrap(), "d-Li4vYQ");
    for id in ["../a", "/a", "a/../b", "b", "b/x"] {
        let c = driver_component(id).unwrap();
        assert!(!c.contains('/'));
        assert!(!c.contains('.'));
    }
    let path = first.write().unwrap();
    assert!(path.starts_with(dir.path().join("private/traffic")));
    let w1 = spc_lab_txd::wallet::TrafficWalletSet::provision(&a, "../a", &root_a).unwrap();
    second.write().unwrap();
    let w2 = spc_lab_txd::wallet::TrafficWalletSet::provision(&b, "b/x", &root_b).unwrap();
    assert_eq!(
        w1.journal().header().unwrap().allocation_digest,
        w2.journal().header().unwrap().allocation_digest
    );
    first.allocation.actors.pop();
    assert!(first.validate().is_err());
}
#[test]
fn actor_process_handshake_has_no_funding_and_wrong_incarnation_exits() {
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path());
    let path = config.write().unwrap();
    let mut process = Process::start(&path);
    let hello = process.receive();
    assert!(matches!(hello,Message::Event {ref name,..} if name=="hello"));
    let request = |id, op: &str, payload| Message::Request {
        protocol: PROTOCOL.into(),
        incarnation: "first".into(),
        driver_id: "../a".into(),
        request_id: id,
        command_id: None,
        operation: op.into(),
        payload,
    };
    write_message(&mut process.input,&request(1,"hello_ack",json!({"protocol_name":"spc-lab-tx-control","protocol_major":1,"accepted":true,"history_version":3}))).unwrap();
    assert_eq!(process.response(1)["status"], "provisioned");
    process.send(2, None, "snapshot", json!({}));
    assert_eq!(process.response(2)["summary"]["funding_observations"], 0);
    let mut wrong = request(3, "stop", json!({}));
    if let Message::Request {
        ref mut incarnation,
        ..
    } = wrong
    {
        *incarnation = "old".into();
    }
    write_message(&mut process.input, &wrong).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        if let Some(status) = process.child.try_wait().unwrap() {
            assert!(!status.success());
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn actor_destination_admission_uses_existing_signer_and_preserves_rejected_journal() {
    use bitcoin::hashes::Hash;
    use spc_lab_txd::journal::{FundingContext, FundingObservation, FundingSource};
    use spc_lab_txd::keys::{Chain, WalletRole};
    use spc_lab_txd::wallet::TrafficWalletSet;
    for destination in destinations::admitted_addresses() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = config(dir.path());
        config.ordinary_interval_ms = 3_600_000;
        let path = config.write().unwrap();
        let allocation = config.validate().unwrap();
        let set =
            TrafficWalletSet::provision(&allocation, "../a", &config.root().unwrap()).unwrap();
        // Offline canonical-source journal fixture; no RPC supplies funding.
        set.observe_funding(&FundingObservation {
            outpoint: bitcoin::OutPoint::new(bitcoin::Txid::from_byte_array([9; 32]), 0),
            amount_sats: 1_000_000,
            chain: Chain::Receive,
            index: 0,
            owner_role: WalletRole::UserPayment { index: 0 },
            source: FundingSource::CanonicalBlock,
            context: FundingContext::CanonicalBlock {
                block_hash: config.chain_policy.genesis.to_string(),
                height: 0,
                coinbase: false,
            },
        })
        .unwrap();
        set.note_chain_observed(config.chain_policy.genesis, 0, &[])
            .unwrap();
        let journal = set.journal().path().to_path_buf();
        drop(set);
        let mut process = Process::start(&path);
        assert!(matches!(process.receive(),Message::Event{ref name,..} if name=="hello"));
        process.send(1,None,"hello_ack",json!({"protocol_name":"spc-lab-tx-control","protocol_major":1,"accepted":true,"history_version":3}));
        assert_eq!(process.response(1)["status"], "provisioned");
        process.send(2,None,"activate",json!({"monotonic_ms":spc_lab_txd::bootstrap::monotonic_ms().unwrap(),"boot_id":spc_lab_txd::bootstrap::boot_id().unwrap()}));
        assert_eq!(process.response(2)["status"], "active");
        let before = std::fs::read(&journal).unwrap();
        let mut captured = Vec::new();
        let payload = |address: &str| json!({"user":"alice","driver_id":"../a","destination_address":address,"amount_sats":100000,"fee_rate_sats_vb":3});
        let mut id = 3;
        for bad in destinations::rejected_addresses() {
            process.send(id, Some("rejected"), "send_user_payment", payload(&bad));
            assert_eq!(
                process.destination_response(id, &mut captured)["status"],
                "failed",
                "{bad}"
            );
            assert_eq!(std::fs::read(&journal).unwrap(), before);
            assert!(captured.is_empty());
            id += 1;
        }
        process.send(
            id,
            Some("destination"),
            "send_user_payment",
            payload(&destination),
        );
        let response = process.destination_response(id, &mut captured);
        assert_eq!(response["status"], "unknown");
        assert_eq!(captured.len(), 1);
        let tx: bitcoin::Transaction =
            bitcoin::consensus::deserialize(&hex::decode(&captured[0]).unwrap()).unwrap();
        let address: bitcoin::Address<_> = destination.parse().unwrap();
        assert_eq!(
            tx.output[0].script_pubkey,
            address
                .require_network(bitcoin::Network::Regtest)
                .unwrap()
                .script_pubkey()
        );
        assert_eq!(tx.output[0].value.to_sat(), 100000);
        assert!(tx.input[0].witness.is_empty());
        assert_eq!(response["txid"], tx.compute_txid().to_string());
        id += 1;
        process.send(
            id,
            Some("destination"),
            "send_user_payment",
            payload(&destination),
        );
        assert_eq!(
            process.destination_response(id, &mut captured)["status"],
            "stored"
        );
        assert_eq!(captured.len(), 1);
        id += 1;
        process.send(
            id,
            Some("destination"),
            "send_user_payment",
            payload("bcrt1qw508d6qejxtdg4y5r3zarvary0c5xw7kygt080"),
        );
        assert_eq!(
            process.destination_response(id, &mut captured)["status"],
            "failed"
        );
        assert_eq!(captured.len(), 1);
        id += 1;
        process.send(id, None, "stop", json!({}));
        assert_eq!(
            process.destination_response(id, &mut captured)["status"],
            "stopped"
        );
    }
}

#[test]
fn actor_invalid_destination_precedes_unavailable_chain_observation() {
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path());
    let path = config.write().unwrap();
    let mut process = Process::start(&path);
    assert!(matches!(process.receive(),Message::Event{ref name,..} if name=="hello"));
    process.send(1,None,"hello_ack",json!({"protocol_name":"spc-lab-tx-control","protocol_major":1,"accepted":true,"history_version":3}));
    assert_eq!(process.response(1)["status"], "provisioned");
    let journal = spc_lab_txd::journal::TrafficJournal::path_in(&config.root().unwrap());
    let before = std::fs::read(&journal).unwrap();
    for (index, address) in destinations::rejected_addresses().into_iter().enumerate() {
        let id = index as u64 + 2;
        process.send(id,Some("invalid-output"),"send_user_payment",json!({"user":"alice","driver_id":"../a","destination_address":address,"amount_sats":100000,"fee_rate_sats_vb":3}));
        let response = process.response(id);
        assert_eq!(response["status"], "failed");
        assert!(
            response["code"].as_str().unwrap().contains("destination"),
            "{response}"
        );
        assert_eq!(std::fs::read(&journal).unwrap(), before);
    }
}

#[test]
fn owned_actor_payment_read_is_scoped_and_never_appends_or_allocates() {
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path());
    let path = config.write().unwrap();
    let mut process = Process::start(&path);
    let hello = process.receive();
    assert!(
        matches!(hello,Message::Event{ref payload,..} if payload["capabilities"].as_array().unwrap().iter().any(|c|c=="payment_wallet_read_v1"))
    );
    process.send(1,None,"hello_ack",json!({"protocol_name":"spc-lab-tx-control","protocol_major":1,"accepted":true,"history_version":3}));
    assert_eq!(process.response(1)["status"], "provisioned");
    let journal = spc_lab_txd::journal::TrafficJournal::path_in(&config.root().unwrap());
    let before = std::fs::read(&journal).unwrap();
    let payload = |user: &str, index| json!({"user_id":user,"wallet_index":index,"allocation_digest":config.validate().unwrap().digest()});
    for id in 2..=3 {
        process.send(id, None, "payment_wallet", payload("alice", 0));
        let reply = process.response(id);
        assert_eq!(reply["view"]["user_id"], "alice");
        assert_eq!(reply["view"]["state"], "not_available");
        assert!(reply["view"]["spendable_sats"].is_null());
        assert!(reply["view"]["unconfirmed_sats"].is_null());
    }
    process.send(4, None, "payment_wallet", payload("bob", 1));
    assert_eq!(process.response(4)["status"], "failed");
    process.send(5, None, "payment_wallet", payload("alice", 1));
    assert_eq!(process.response(5)["status"], "failed");
    assert_eq!(std::fs::read(&journal).unwrap(), before);
    process.send(6, None, "stop", json!({}));
    assert_eq!(process.response(6)["status"], "stopped");
}

#[test]
fn owned_actor_version_case_selects_before_signing_and_replay_does_not_send_again() {
    use bitcoin::hashes::Hash;
    use spc_lab_txd::journal::{FundingContext, FundingObservation, FundingSource};
    use spc_lab_txd::keys::{Chain, WalletRole};
    use spc_lab_txd::wallet::TrafficWalletSet;
    let dir = tempfile::tempdir().unwrap();
    let mut config = config(dir.path());
    config.lab_acceptance = true;
    config.ordinary_interval_ms = 3_600_000;
    let path = config.write().unwrap();
    let allocation = config.validate().unwrap();
    let set = TrafficWalletSet::provision(&allocation, "../a", &config.root().unwrap()).unwrap();
    let script = set
        .next_receive_address(&WalletRole::UserPayment { index: 0 })
        .unwrap()
        .script_pubkey;
    // Explicit offline canonical-source fixture, not native/node funding proof.
    set.observe_funding(&FundingObservation {
        outpoint: bitcoin::OutPoint::new(bitcoin::Txid::from_byte_array([9; 32]), 0),
        amount_sats: 1_000_000,
        chain: Chain::Receive,
        index: 0,
        owner_role: WalletRole::UserPayment { index: 0 },
        source: FundingSource::CanonicalBlock,
        context: FundingContext::CanonicalBlock {
            block_hash: config.chain_policy.genesis.to_string(),
            height: 0,
            coinbase: false,
        },
    })
    .unwrap();
    set.note_chain_observed(config.chain_policy.genesis, 0, &[])
        .unwrap();
    drop(set);
    let mut process = Process::start(&path);
    process.receive();
    process.send(1,None,"hello_ack",json!({"protocol_name":"spc-lab-tx-control","protocol_major":1,"accepted":true,"history_version":3}));
    assert_eq!(process.response(1)["status"], "provisioned");
    process.send(2,None,"activate",json!({"boot_id":spc_lab_txd::bootstrap::boot_id().unwrap(),"monotonic_ms":spc_lab_txd::bootstrap::monotonic_ms().unwrap()}));
    assert_eq!(process.response(2)["status"], "active");
    let destination = destinations::generated_pair(bitcoin::Network::Regtest)[0].to_string();
    let payload = json!({"user":"alice","driver_id":"../a","destination_address":destination,"amount_sats":100000,"fee_rate_sats_vb":3});
    let mut captured = Vec::new();
    process.send(
        3,
        Some("selected-original"),
        "acceptance_version_rejection_v1",
        payload.clone(),
    );
    // The responder returns transport Unknown, not a forged node rejection.
    // Independently verify every final legacy signature before any live claim.
    let result = process.destination_response(3, &mut captured);
    assert_eq!(result["status"], "unknown");
    assert_eq!(captured.len(), 1);
    independent::verify_version_rejection_traffic(
        &hex::decode(&captured[0]).unwrap(),
        &[independent::VerifyUtxo {
            prevout_txid: [9; 32],
            prevout_vout: 0,
            amount_sats: 1_000_000,
            script_pubkey: script.to_bytes(),
        }],
    );
    process.send(
        4,
        Some("selected-original"),
        "acceptance_version_rejection_v1",
        payload.clone(),
    );
    assert_eq!(
        process.destination_response(4, &mut captured)["status"],
        "stored"
    );
    assert_eq!(captured.len(), 1);
    process.send(5, Some("selected-original"), "send_user_payment", payload);
    assert_eq!(
        process.destination_response(5, &mut captured)["status"],
        "failed"
    );
    assert_eq!(captured.len(), 1);
    process.send(6, None, "stop", json!({}));
    assert_eq!(
        process.destination_response(6, &mut captured)["status"],
        "stopped"
    );
    drop(process);
    let reopened = TrafficWalletSet::reopen(&allocation, "../a", &config.root().unwrap()).unwrap();
    let command = reopened.journal().command("selected-original").unwrap();
    assert_eq!(command.raw_hex.as_deref(), Some(captured[0].as_str()));
    assert_eq!(
        command.request.construction_effect,
        Some(spc_lab_txd::construction::ConstructionEffect::VersionRejectionV1)
    );
}

#[test]
fn restarted_actor_pause_ack_uses_closed_journal_metadata_and_wire_duplicate_cannot_send() {
    use bitcoin::hashes::Hash;
    use spc_lab_txd::builder::{DestinationSpec, PaymentCategory};
    use spc_lab_txd::journal::{FundingContext, FundingObservation, FundingSource};
    use spc_lab_txd::keys::{Chain, WalletRole};
    use spc_lab_txd::wallet::{PaymentOutcome, PaymentRequest, TrafficWalletSet};
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path());
    let path = config.write().unwrap();
    let token = config.validate().unwrap();
    let wallet = TrafficWalletSet::provision(&token, "../a", &config.root().unwrap()).unwrap();
    // Offline core fixture, using the same approved canonical-source unit-test
    // boundary as history3.rs. No RPC or funded runtime proof is claimed.
    wallet
        .observe_funding(&FundingObservation {
            outpoint: bitcoin::OutPoint::new(bitcoin::Txid::from_byte_array([9; 32]), 0),
            amount_sats: 1_000_000,
            chain: Chain::Receive,
            index: 0,
            owner_role: WalletRole::UserPayment { index: 0 },
            source: FundingSource::CanonicalBlock,
            context: FundingContext::CanonicalBlock {
                block_hash: config.chain_policy.genesis.to_string(),
                height: 0,
                coinbase: false,
            },
        })
        .unwrap();
    wallet
        .note_chain_observed(config.chain_policy.genesis, 0, &[])
        .unwrap();
    let request = PaymentRequest {
        command_id: "payment".into(),
        payer_role: WalletRole::UserPayment { index: 0 },
        category: PaymentCategory::UserPayment,
        destinations: vec![DestinationSpec {
            script_pubkey: wallet
                .next_receive_address(&WalletRole::NetworkWallet { index: 0 })
                .unwrap()
                .script_pubkey,
            amount_sats: 100_000,
            owner: Some(WalletRole::NetworkWallet { index: 0 }),
        }],
        fee_rate_sats_vb: 3,
    };
    let txid = match wallet.sign_payment(&request).unwrap() {
        PaymentOutcome::Signed(v) => v.txid,
        _ => panic!("fresh signature"),
    };
    wallet.note_broadcast_sent("payment").unwrap();
    wallet
        .note_unknown("payment", "fixture lost response")
        .unwrap();
    wallet.pause_sends("pause-original", 10).unwrap();
    drop(wallet);
    let mut process = Process::start(&path);
    assert!(matches!(process.receive(),Message::Event {ref name,..} if name=="hello"));
    process.send(1,None,"hello_ack",json!({"protocol_name":"spc-lab-tx-control","protocol_major":1,"accepted":true,"history_version":3}));
    assert_eq!(process.response(1)["paused"], true);
    process.send(2,None,"activate",json!({"boot_id":spc_lab_txd::bootstrap::boot_id().unwrap(),"monotonic_ms":spc_lab_txd::bootstrap::monotonic_ms().unwrap()}));
    assert_eq!(process.response(2)["status"], "active");
    process.send(3, Some("resume"), "resume", json!({}));
    assert_eq!(process.response(3)["status"], "resumed");
    process.send(4, Some("pause-restarted"), "pause", json!({}));
    let ack = process.response(4);
    assert_eq!(ack["last_used_send_sequence"], 1);
    assert_eq!(ack["last_txid"], txid.to_string());
    process.send(4, Some("duplicate"), "resume", json!({}));
    drop(process);
    let reopened = TrafficWalletSet::reopen(&token, "../a", &config.root().unwrap()).unwrap();
    assert!(reopened.journal().barrier_active());
    assert_eq!(reopened.journal().next_send_sequence(), 2);
    assert_eq!(
        reopened.journal().recovery_snapshot().transactions[&txid]
            .facts
            .retry_sends,
        0
    );
}
#[test]
fn rpc_id_code_type_and_bounded_json_fail_closed() {
    assert_eq!(
        parse_rpc_response(
            br#"{"id":2,"result":null,"error":{"code":-26,"message":"version"}}"#,
            1
        ),
        RpcOutcome::unknown(UnknownClass::Protocol)
    );
    assert_eq!(
        parse_rpc_response(
            br#"{"id":1,"result":null,"error":{"code":"-26","message":"version"}}"#,
            1
        ),
        RpcOutcome::unknown(UnknownClass::Shape)
    );
    assert_eq!(
        parse_rpc_response(
            br#"{"id":1,"result":null,"error":{"code":-26,"message":"version"}}"#,
            1
        ),
        RpcOutcome::NodeRejected {
            code: -26,
            message: "version".into()
        }
    );
    assert!(matches!(
        parse_rpc_response(&vec![b' '; spc_lab_txd::control::MAX_BODY + 1], 1),
        RpcOutcome::Unknown { .. }
    ));
    let mut reader = BufReader::new(&b"{\"kind\":\"response\"}"[..]);
    assert!(read_message(&mut reader).is_err());
    let mut reader = BufReader::new(&b"{}\n"[..]);
    assert!(read_message(&mut reader).is_err());
    let mut seq = spc_lab_txd::control::RequestSequence::default();
    seq.accept(1).unwrap();
    assert!(seq.accept(1).is_err());
    assert!(seq.accept(3).is_err());
    assert!(spc_lab_txd::control::strict_json(br#"{"error":{"code":-26,"code":-8}}"#).is_err());
}

#[test]
fn owned_actor_signed_hold_observes_retained_coinbase_rollback_before_intent() {
    // Actual owned txd process; explicitly synthetic raw-chain responses test
    // the adapter/core boundary only. This is NOT funded live acceptance.
    use bitcoin::consensus::serialize;
    use bitcoin::{Amount, Block, OutPoint, Sequence, Transaction, TxIn, TxOut, Witness};
    use spc_lab_txd::journal::{FundingContext, FundingObservation, FundingSource};
    use spc_lab_txd::keys::{Chain, WalletRole};
    use spc_lab_txd::wallet::TrafficWalletSet;
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(dir.path());
    cfg.lab_acceptance = true;
    let path = cfg.write().unwrap();
    let allocation = cfg.validate().unwrap();
    let wallet = TrafficWalletSet::provision(&allocation, "../a", &cfg.root().unwrap()).unwrap();
    let funding = wallet
        .next_receive_address(&WalletRole::UserPayment { index: 0 })
        .unwrap()
        .script_pubkey;
    let destination = wallet
        .next_receive_address(&WalletRole::NetworkWallet { index: 0 })
        .unwrap()
        .script_pubkey;
    let address = bitcoin::Address::from_script(&destination, bitcoin::Network::Regtest)
        .unwrap()
        .to_string();
    let mut blocks = vec![bitcoin::blockdata::constants::genesis_block(
        bitcoin::Network::Regtest,
    )];
    for height in 1..=101 {
        let parent = blocks.last().unwrap();
        let coinbase = Transaction {
            version: bitcoin::transaction::Version::TWO,
            lock_time: bitcoin::absolute::LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::null(),
                script_sig: bitcoin::script::Builder::new()
                    .push_int(height)
                    .push_int(0)
                    .into_script(),
                sequence: Sequence::MAX,
                witness: Witness::default(),
            }],
            output: vec![TxOut {
                value: Amount::from_sat(5_000_000_000),
                script_pubkey: if height == 1 {
                    funding.clone()
                } else {
                    bitcoin::ScriptBuf::new_op_return([1])
                },
            }],
        };
        let mut block = Block {
            header: bitcoin::block::Header {
                version: bitcoin::block::Version::ONE,
                prev_blockhash: parent.block_hash(),
                merkle_root: parent.header.merkle_root,
                time: parent.header.time + 1,
                bits: bitcoin::CompactTarget::from_consensus(0x207fffff),
                nonce: 0,
            },
            txdata: vec![coinbase],
        };
        block.header.merkle_root = block.compute_merkle_root().unwrap();
        while block.header.validate_pow(block.header.target()).is_err() {
            block.header.nonce += 1;
            assert!(block.header.nonce < 1000);
        }
        blocks.push(block);
    }
    wallet
        .note_chain_observed(blocks[101].block_hash(), 101, &[])
        .unwrap();
    wallet
        .observe_funding(&FundingObservation {
            outpoint: OutPoint::new(blocks[1].txdata[0].compute_txid(), 0),
            amount_sats: 5_000_000_000,
            chain: Chain::Receive,
            index: 0,
            owner_role: WalletRole::UserPayment { index: 0 },
            source: FundingSource::BootstrapCoinbase,
            context: FundingContext::CanonicalBlock {
                block_hash: blocks[1].block_hash().to_string(),
                height: 1,
                coinbase: true,
            },
        })
        .unwrap();
    drop(wallet);
    let mut process = Process::start(&path);
    process.receive();
    process.send(1,None,"hello_ack",json!({"protocol_name":"spc-lab-tx-control","protocol_major":1,"accepted":true,"history_version":3}));
    assert_eq!(process.response(1)["status"], "provisioned");
    process.send(2,None,"activate",json!({"boot_id":spc_lab_txd::bootstrap::boot_id().unwrap(),"monotonic_ms":spc_lab_txd::bootstrap::monotonic_ms().unwrap()}));
    assert_eq!(process.response(2)["status"], "active");
    process.send(3,Some("held-payment"),"send_user_payment",json!({"user":"alice","driver_id":"../a","destination_address":address,"amount_sats":100000,"fee_rate_sats_vb":3}));
    let mut tip = 101usize;
    let mut checkpoints = 0;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        assert!(std::time::Instant::now() < deadline, "bounded held actor");
        match process.receive() {
            Message::Event { .. } => {}
            Message::Response {
                request_id: 3,
                result,
                ..
            } => {
                assert_eq!(result["status"], "failed");
                break;
            }
            Message::Request {
                request_id,
                operation,
                payload,
                ..
            } => {
                let result = if operation == "acceptance_checkpoint" {
                    assert_eq!(payload["command_id"], "held-payment");
                    checkpoints += 1;
                    // First checkpoint is after durable Signed and before any
                    // intent; no successful command response has been received.
                    let history = std::fs::read_to_string(
                        spc_lab_txd::journal::TrafficJournal::path_in(&cfg.root().unwrap()),
                    )
                    .unwrap();
                    assert!(history.contains("signed_tx"));
                    assert!(!history.contains("broadcast_sent"));
                    tip = 99;
                    json!({"status":if checkpoints==1{"hold"}else{"proceed"}})
                } else {
                    assert_eq!(operation, "rpc_forward");
                    let request = &payload["request"];
                    let value = match request["method"].as_str().unwrap() {
                        "getbestblockhash" => json!(blocks[tip].block_hash().to_string()),
                        "getblock" => json!(hex::encode(serialize(
                            blocks
                                .iter()
                                .find(|b| Some(b.block_hash().to_string().as_str())
                                    == request["params"][0].as_str())
                                .unwrap()
                        ))),
                        other => panic!("unexpected RPC (must not send): {other}"),
                    };
                    json!({"request":request,"result":RpcOutcome::Ok{result:value}})
                };
                write_message(
                    &mut process.input,
                    &Message::Response {
                        protocol: PROTOCOL.into(),
                        incarnation: "first".into(),
                        driver_id: "../a".into(),
                        request_id,
                        command_id: None,
                        result,
                    },
                )
                .unwrap();
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(checkpoints, 2);
    drop(process);
    let wallet = TrafficWalletSet::reopen(&allocation, "../a", &cfg.root().unwrap()).unwrap();
    assert_eq!(wallet.journal().next_send_sequence(), 1);
    let snapshot = wallet.journal().recovery_snapshot();
    let txid = snapshot.commands["held-payment"].txid.unwrap();
    assert_eq!(
        snapshot.transactions[&txid].facts.state,
        spc_lab_txd::journal::TxState::Signed
    );
    assert_eq!(snapshot.tip.unwrap().height, 99);
    assert!(wallet
        .journal()
        .observer_snapshot()
        .outpoints
        .values()
        .any(|o| o.coinbase_immature));
}
