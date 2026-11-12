//! Explicit offline hosts: no node, journal credit, or funded-runtime claim.
//! Header search is the production loop and performs real SHA256d work.
use super::*;
use crate::actor_config::{AllocationActor, AllocationDescription, SizeClass};
use crate::builder::FeePolicy;
use crate::journal::JournalCaps;
use bitcoin::blockdata::constants::genesis_block;
use bitcoin::Network;
use std::sync::atomic::{AtomicUsize, Ordering};

fn policy() -> ChainPolicy {
    ChainPolicy {
        genesis: genesis_block(Network::Regtest).block_hash(),
        pow_limit_bits: 0x207fffff,
        initial_target_bits: 0x207fffff,
        no_pow_retargeting: true,
        allow_min_difficulty_blocks: false,
        pow_target_spacing: 600,
        difficulty_adjustment_interval: 2016,
        regtest_pow_bits_overlay: 0x207fffff,
        subsidy_initial_sats: 5_000_000_000,
        subsidy_halving_interval: 150,
    }
}

#[test]
fn private_evidence_root_refuses_symlinks_and_public_permissions() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("private/traffic/driver");
    std::fs::create_dir_all(&root).unwrap();
    for path in [
        temp.path().join("private"),
        temp.path().join("private/traffic"),
        root.clone(),
    ] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    validate_evidence_root(temp.path(), &root).unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(validate_evidence_root(temp.path(), &root).is_err());
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let link = temp.path().join("private/traffic/linked");
    symlink(&root, &link).unwrap();
    assert!(validate_evidence_root(temp.path(), &link).is_err());
}
fn script() -> ScriptBuf {
    ScriptBuf::from_bytes(
        hex::decode("76a914111111111111111111111111111111111111111188ac").unwrap(),
    )
}
fn template() -> Value {
    json!({"version":536870912,"height":1,"previousblockhash":policy().genesis,
        "bits":"207fffff","target":hex::encode(policy().pow_limit().unwrap().to_be_bytes()),
        "mintime":1296688603,"curtime":1296688603,"coinbasevalue":5_000_000_000u64,"transactions":[]})
}
#[test]
fn native_template_context_supports_min_difficulty_without_trusting_bits() {
    let mut policy = policy();
    policy.allow_min_difficulty_blocks = true;
    policy.initial_target_bits = 0x200fffff;
    policy.regtest_pow_bits_overlay = 0x200fffff;
    let genesis = genesis_block(Network::Regtest).header;
    let context = crate::contextual_pow::VerifiedHeaders::new(&policy, vec![genesis]).unwrap();
    let mut value = template();
    value["curtime"] = json!(genesis.time + 1201);
    Template::parse(&value, &policy)
        .unwrap()
        .verify_context(&context)
        .unwrap();
    value["curtime"] = json!(genesis.time + 1200);
    assert!(Template::parse(&value, &policy)
        .unwrap()
        .verify_context(&context)
        .is_err());
    value["bits"] = json!("200fffff");
    value["target"] = json!(hex::encode(
        Target::from_compact(bitcoin::CompactTarget::from_consensus(0x200fffff)).to_be_bytes()
    ));
    Template::parse(&value, &policy)
        .unwrap()
        .verify_context(&context)
        .unwrap();
}

#[test]
fn chain_policy_requires_explicit_min_difficulty_fact() {
    let mut encoded = serde_json::to_value(policy()).unwrap();
    encoded
        .as_object_mut()
        .unwrap()
        .remove("allow_min_difficulty_blocks");
    assert!(serde_json::from_value::<ChainPolicy>(encoded).is_err());
}

fn config() -> ActorConfig {
    ActorConfig {
        node_profile_binding: None,
        lab_acceptance: false,
        chain_policy: policy(),
        size_classes: vec![SizeClass {
            vbytes: 226,
            mix: 1.0,
        }],
        protocol: crate::control::PROTOCOL.into(),
        history_version: 3,
        run_dir: std::path::PathBuf::from("/unused-offline-fixture"),
        driver_id: "txd-0".into(),
        allocation: AllocationDescription {
            run_seed: 42,
            run_id: "native-bootstrap-fixture".into(),
            network: Network::Regtest,
            caps: JournalCaps {
                unknown_retry_sends: 3,
                reorg_resubmits: 3,
            },
            actors: vec![AllocationActor {
                driver_id: "txd-0".into(),
                network_wallets: 2,
                user_payment_wallets: 1,
                hosts_bootstrap_funding: true,
                lookahead: 64,
                fee_policy: FeePolicy::default(),
            }],
            users: vec!["user-0".into()],
        },
        node_ids: vec!["node-0".into(), "node-1".into()],
        authority_node: "node-0".into(),
        ordinary_interval_ms: 5000,
        max_vbytes: 1000,
        max_mempool_backlog_tx: 100,
    }
}

#[test]
fn bitcoin_genesis_header_endianness_and_real_sha256_vector() {
    let genesis = genesis_block(Network::Bitcoin);
    let cb = serialize(&genesis.txdata[0]);
    let preimage = HeaderPreimage::new(1, [0; 32], &cb, &[], 1231006505, 0x1d00ffff);
    let (raw, hash) = preimage.hash_nonce(2083236893);
    assert_eq!(raw.to_vec(), serialize(&genesis.header));
    assert_eq!(
        hash.to_string(),
        "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f"
    );
    assert!(genesis.header.target().is_met_by(hash));
}

#[test]
fn bip34_coinbase_is_not_a_spend_and_has_exact_value() {
    for (height, prefix) in [
        (1, "51"),
        (16, "60"),
        (17, "0111"),
        (128, "028000"),
        (800000, "0300350c"),
    ] {
        let tx = coinbase(height, 7, &script(), 123456).unwrap();
        assert!(tx.is_coinbase());
        assert!(tx.input[0].script_sig.to_hex_string().starts_with(prefix));
        assert_eq!(tx.input[0].sequence, bitcoin::Sequence::MAX);
        assert_eq!(tx.output[0].value.to_sat(), 123456);
        assert_eq!(tx.output[0].script_pubkey, script());
        assert_eq!(
            strict_transaction(&serialize(&tx), tx.compute_txid()).unwrap(),
            tx
        );
    }
}

#[test]
fn compact_consensus_sign_zero_overflow_and_powlimit_vectors() {
    let max = Target::from_be_bytes([255; 32]);
    for bits in [
        0, 0x20800001, 0x23000001, 0x22000100, 0x21010000, 0x01000001,
    ] {
        assert!(compact_target(bits, max).is_err(), "{bits:x}");
    }
    for bits in [0x207fffff, 0x2100ffff, 0x220000ff] {
        assert!(compact_target(bits, max).is_ok(), "{bits:x}");
    }
    assert!(compact_target(
        0x207fffff,
        Target::from_compact(bitcoin::CompactTarget::from_consensus(0x200fffff))
    )
    .is_err());
}

#[test]
fn gbt_requires_integer_lexemes_and_separate_profile_conformance() {
    for key in ["version", "height", "coinbasevalue", "mintime", "curtime"] {
        for bad in [json!(1.0), json!(-1), json!("1"), Value::Null, json!(true)] {
            let mut t = template();
            t[key] = bad;
            assert!(Template::parse(&t, &policy()).is_err(), "{key}");
        }
    }
    let mut t = template();
    t["version"] = json!(u64::from(u32::MAX) + 1);
    assert!(Template::parse(&t, &policy()).is_err());
    let mut t = template();
    t["bits"] = json!("200fffff");
    t["target"] = json!(hex::encode(
        Target::from_compact(bitcoin::CompactTarget::from_consensus(0x200fffff)).to_be_bytes()
    ));
    let context = crate::contextual_pow::VerifiedHeaders::new(
        &policy(),
        vec![genesis_block(Network::Regtest).header],
    )
    .unwrap();
    assert!(Template::parse(&t, &policy())
        .unwrap()
        .verify_context(&context)
        .is_err()); // matching fields, wrong contextual policy
    let mut t = template();
    t["target"] = json!("00".repeat(32));
    assert!(Template::parse(&t, &policy()).is_err());
    let mut p = policy();
    p.no_pow_retargeting = false;
    assert!(Template::parse(&template(), &p).is_err());
}

fn legacy_tx() -> Transaction {
    let mut tx = coinbase(1, 0, &script(), 1000).unwrap();
    tx.input[0].previous_output = OutPoint {
        txid: Txid::from_byte_array([7; 32]),
        vout: 0,
    };
    tx
}
fn with_tx(tx: &Transaction, fee: Value) -> Value {
    let mut value = template();
    value["transactions"] =
        json!([{"data":hex::encode(serialize(tx)),"txid":tx.compute_txid(),"fee":fee}]);
    value
}
#[test]
fn raw_transaction_roundtrip_trailing_data_txid_and_witness_refusal() {
    let tx = legacy_tx();
    let mut raw = serialize(&tx);
    assert!(strict_transaction(&raw, tx.compute_txid()).is_ok());
    raw.push(0);
    assert!(strict_transaction(&raw, tx.compute_txid()).is_err());
    assert!(strict_transaction(&serialize(&tx), Txid::all_zeros()).is_err());
    let mut witness = tx;
    witness.input[0].witness.push([42]);
    let t = with_tx(&witness, json!(0));
    assert_eq!(
        Template::parse(&t, &policy()).unwrap_err(),
        "bootstrap_witness_template_unsupported"
    );
}

#[test]
fn fees_subsidy_halving_and_overflow_are_exact() {
    let tx = legacy_tx();
    let mut t = with_tx(&tx, json!(123));
    t["coinbasevalue"] = json!(5_000_000_123u64);
    assert_eq!(
        Template::parse(&t, &policy()).unwrap().revenue,
        5_000_000_123
    );
    t["coinbasevalue"] = json!(5_000_000_122u64);
    assert!(Template::parse(&t, &policy()).is_err());
    for fee in [json!(1.0), json!(-1), Value::Null, json!(u64::MAX)] {
        assert!(Template::parse(&with_tx(&tx, fee), &policy()).is_err());
    }
    assert_eq!(subsidy(&policy(), 149).unwrap(), 5_000_000_000);
    assert_eq!(subsidy(&policy(), 150).unwrap(), 2_500_000_000);
    assert_eq!(subsidy(&policy(), 150 * 64).unwrap(), 0);
    let mut p = policy();
    p.subsidy_halving_interval = 0;
    assert!(subsidy(&p, 1).is_err());
}

#[test]
fn oldest_of_101_coinbases_only_is_mature() {
    assert!(!is_mature(1, 100).unwrap());
    assert!(is_mature(1, 101).unwrap());
    for height in 2..=101 {
        assert!(!is_mature(height, 101).unwrap());
    }
    assert!(!is_mature(0, 1000).unwrap());
    assert!(is_mature(u64::MAX, 100).is_err());
}

#[test]
fn merkle_paths_match_consensus_for_odd_and_even_transaction_counts() {
    let cb = coinbase(17, 3, &script(), 5_000_000_000).unwrap();
    for count in 0..15 {
        let transactions: Vec<_> = (0..count)
            .map(|i| {
                let mut tx = legacy_tx();
                tx.output[0].value = bitcoin::Amount::from_sat(1000 + i);
                tx
            })
            .collect();
        let path = coinbase_path(&transactions);
        let root = spc_cpu_miner::hash::merkle_root_for_coinbase(&serialize(&cb), &path);
        let mut block = genesis_block(Network::Regtest);
        block.txdata = vec![cb.clone()];
        block.txdata.extend(transactions);
        assert_eq!(root, block.compute_merkle_root().unwrap().to_byte_array());
    }
}

#[test]
fn distribution_count_disjoint_roles_fees_and_rounding_conserve() {
    let mut cfg = config();
    cfg.allocation.actors.push(AllocationActor {
        driver_id: "txd-1".into(),
        network_wallets: 3,
        user_payment_wallets: 1,
        hosts_bootstrap_funding: false,
        lookahead: 64,
        fee_policy: FeePolicy::default(),
    });
    cfg.allocation.users.push("user-1".into());
    cfg.validate().unwrap();
    let available = 5_000_000_000;
    let plan = DistributionPlan::new(&cfg, 8, available).unwrap();
    assert_eq!(plan.count, 56);
    let mut loci = BTreeSet::new();
    for output in plan.parts.iter().flatten() {
        assert!(loci.insert((output.role.clone(), output.receive_index)));
    }
    let sent = plan.count * plan.amount;
    let residual = available - sent - plan.fee_reserve;
    assert_eq!(sent + plan.fee_reserve + residual, available);
    assert!(residual >= 546 && residual < 546 + plan.count);
    assert!(DistributionPlan::new(&cfg, 64, 1000).is_err());
}

#[derive(Clone, Copy)]
enum Permit {
    Normal,
    NoAck,
    Duplicate,
    OldIncarnation,
    Cancel,
}
#[derive(Clone, Copy)]
enum Submission {
    Accept,
    Unknown,
    Reject,
}
struct FixtureHost {
    permit: Permit,
    submission: Submission,
    control: OperationControl,
    chain: Mutex<Vec<Block>>,
    reservations: Mutex<Vec<HashAuthorization>>,
    reconciled: Mutex<Vec<u64>>,
    observed: AtomicUsize,
}
impl FixtureHost {
    fn new(permit: Permit, submission: Submission) -> Self {
        Self {
            permit,
            submission,
            control: OperationControl::default(),
            chain: Mutex::new(vec![genesis_block(Network::Regtest)]),
            reservations: Mutex::new(vec![]),
            reconciled: Mutex::new(vec![]),
            observed: AtomicUsize::new(0),
        }
    }
}
impl BootstrapHost for FixtureHost {
    fn actor_request(&self, operation: &str, payload: Value) -> Result<Value, String> {
        match operation {
            "funding_address" => {
                Ok(json!({"status":"address","index":0,"script_hex":script().to_hex_string()}))
            }
            "reserve_hashes" => {
                if matches!(self.permit, Permit::NoAck) {
                    return Err("fixture_lost_ack".into());
                }
                let mut reservations = self.reservations.lock().unwrap();
                let a = HashAuthorization {
                    operation_id: payload["operation_id"].as_str().unwrap().into(),
                    driver_incarnation: if matches!(self.permit, Permit::OldIncarnation) {
                        "old"
                    } else {
                        "fixture"
                    }
                    .into(),
                    batch_id: payload["batch_id"].as_u64().unwrap(),
                    reserved_attempts: payload["attempts"].as_u64().unwrap(),
                    cumulative_reserved: reservations.last().map_or(0, |a| a.cumulative_reserved)
                        + payload["attempts"].as_u64().unwrap(),
                };
                reservations.push(a.clone());
                if matches!(self.permit, Permit::Cancel) {
                    self.control.cancel();
                }
                Ok(
                    json!({"status":"authorized","execute":true,"reservation":HashReservationStatus{
                    authorization:a,duplicate:matches!(self.permit,Permit::Duplicate),reconciled:false,measured_attempts:None}}),
                )
            }
            "reconcile_hashes" => {
                self.reconciled
                    .lock()
                    .unwrap()
                    .push(payload["measured_attempts"].as_u64().unwrap());
                Ok(json!({"status":"reconciled"}))
            }
            "observe_bootstrap_coinbase" => {
                // Records that production attempted observation, NOT credit.
                self.observed.fetch_add(1, Ordering::SeqCst);
                Ok(json!({"status":"funding_observed"}))
            }
            "bootstrap_rpc" => {
                let mut chain = self.chain.lock().unwrap();
                let params = &payload["params"];
                let value = match payload["method"].as_str().unwrap() {
                    "getbestblockhash" => json!(chain.last().unwrap().block_hash()),
                    "getblockheader" => {
                        let h = chain
                            .iter()
                            .position(|b| {
                                Some(b.block_hash().to_string().as_str()) == params[0].as_str()
                            })
                            .ok_or("fixture_header_missing")?;
                        json!({"height":h,"confirmations":chain.len()-h})
                    }
                    "getblockhash" => {
                        json!(chain[params[0].as_u64().unwrap() as usize].block_hash())
                    }
                    "getblock" => {
                        assert_eq!(params[1], json!(0));
                        let b = chain
                            .iter()
                            .find(|b| {
                                Some(b.block_hash().to_string().as_str()) == params[0].as_str()
                            })
                            .ok_or("fixture_block_missing")?;
                        json!(hex::encode(serialize(b)))
                    }
                    "getblocktemplate" => {
                        assert_eq!(*params, json!([{"mode":"template","rules":["segwit"]}]));
                        let mut t = template();
                        t["height"] = json!(chain.len());
                        t["previousblockhash"] = json!(chain.last().unwrap().block_hash());
                        t["curtime"] = json!(1296688602 + chain.len() as u64);
                        t
                    }
                    "submitblock" => {
                        let raw = hex::decode(params[0].as_str().unwrap()).unwrap();
                        let block: Block = deserialize(&raw).unwrap();
                        verify_raw_block(&raw, block.block_hash(), &policy()).unwrap();
                        match self.submission {
                            Submission::Accept => {
                                chain.push(block);
                                Value::Null
                            }
                            Submission::Reject => json!("fixture_rejected"),
                            Submission::Unknown => {
                                return Ok(
                                    json!({"status":"rpc_result","result":RpcOutcome::Unknown{class:crate::control::UnknownClass::Timeout}}),
                                )
                            }
                        }
                    }
                    other => return Err(format!("unexpected fixture RPC {other}")),
                };
                Ok(json!({"status":"rpc_result","result":RpcOutcome::Ok{result:value}}))
            }
            other => Err(format!("unexpected fixture operation {other}")),
        }
    }
}
fn fixture_run<'a>(config: &'a ActorConfig, host: Arc<FixtureHost>, total: u64) -> Run<'a> {
    let request = BootstrapRequest {
        command_id: "operation".into(),
        driver_id: "txd-0".into(),
        node_id: "node-0".into(),
        boot_id: boot_id().unwrap(),
        deadline_monotonic_ms: monotonic_ms().unwrap() + 10_000,
        cancel_grace_secs: 30,
    };
    let control = host.control.clone();
    Run {
        rpc: HostRpc {
            host,
            request,
            control,
        },
        config,
        evidence: tempfile::tempfile().unwrap(),
        budget: HashBudget {
            hashes_per_second: Some(1_000_000),
            total_attempts: Some(total),
        },
        cumulative_reserved: 0,
        next_batch: 1,
        consumption: BatchConsumption::new("operation".into(), "fixture".into()),
        pacer: Pacer::new(Instant::now()),
        entropy: 0,
        parent_context: None,
    }
}

#[test]
fn no_ack_duplicate_old_incarnation_and_cancel_execute_zero_hashes() {
    let cfg = config();
    for permit in [
        Permit::NoAck,
        Permit::Duplicate,
        Permit::OldIncarnation,
        Permit::Cancel,
    ] {
        let host = Arc::new(FixtureHost::new(permit, Submission::Accept));
        let mut run = fixture_run(&cfg, host.clone(), 128);
        assert!(run.mine_block().is_err());
        assert_eq!(run.pacer.attempted(), 0);
        assert_eq!(host.observed.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn native_fixture_executes_real_pow_then_raw_canonical_observation() {
    let cfg = config();
    let host = Arc::new(FixtureHost::new(Permit::Normal, Submission::Accept));
    let mut run = fixture_run(&cfg, host.clone(), 128);
    let (block, height, _) = run.mine_block().unwrap();
    assert_eq!(height, 1);
    assert!(block.header.validate_pow(block.header.target()).is_ok());
    assert!(run.pacer.attempted() > 0);
    assert_eq!(
        host.reconciled.lock().unwrap().iter().sum::<u64>(),
        run.pacer.attempted()
    );
    assert!(run.cumulative_reserved >= run.pacer.attempted());
    assert_eq!(host.observed.load(Ordering::SeqCst), 1);
    let before = run.pacer.attempted();
    let reserved = run.cumulative_reserved;
    run.mine_block().unwrap();
    assert!(run.pacer.attempted() > before && run.cumulative_reserved > reserved);
}

#[test]
fn unknown_submission_never_credits_and_rejected_work_exhausts_shared_budget() {
    let cfg = config();
    for submission in [Submission::Unknown, Submission::Reject] {
        let host = Arc::new(FixtureHost::new(Permit::Normal, submission));
        let mut run = fixture_run(&cfg, host.clone(), 64);
        let error = run.mine_block().unwrap_err();
        assert!(
            matches!(
                error.as_str(),
                "bootstrap_submission_uncertain" | "bootstrap_hash_budget_exhausted"
            ),
            "{error}"
        );
        assert_eq!(host.observed.load(Ordering::SeqCst), 0);
        assert!(run.pacer.attempted() > 0 && run.pacer.attempted() <= run.cumulative_reserved);
        assert!(run.cumulative_reserved <= 64);
    }
}

#[test]
fn expired_or_wrong_boot_requests_never_reach_host() {
    let cfg = config();
    let host = Arc::new(FixtureHost::new(Permit::Normal, Submission::Accept));
    let mut run = fixture_run(&cfg, host.clone(), 64);
    run.rpc.request.deadline_monotonic_ms = 0;
    assert!(run.mine_block().is_err());
    assert_eq!(run.pacer.attempted(), 0);
    assert!(host.reservations.lock().unwrap().is_empty());
    run.rpc.request.deadline_monotonic_ms = monotonic_ms().unwrap() + 1000;
    run.rpc.request.boot_id = "another-boot".into();
    assert!(run.mine_block().is_err());
    assert_eq!(run.pacer.attempted(), 0);
}

#[test]
fn paused_operation_deadline_keeps_running_without_hashes() {
    let cfg = config();
    let host = Arc::new(FixtureHost::new(Permit::Normal, Submission::Accept));
    host.control.pause();
    let mut run = fixture_run(&cfg, host.clone(), 64);
    run.rpc.request.deadline_monotonic_ms = monotonic_ms().unwrap() + 40;
    assert!(run.mine_block().is_err());
    assert_eq!(run.pacer.attempted(), 0);
    assert!(host.reservations.lock().unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// Real two-node propagation (env-gated): SPC_LAB_RBITCOIN_NODE=/path/to/rbitcoin
//
// Proves the P2P wiring the engine plans for host runs (ring peering over
// leased loopback ports, --no-seeds/--no-discover/--max-outbound 1) actually
// propagates a mined block between two real nodes: the bootstrap submits
// only to the authority node and every configured node observes the common
// chain before funding observation is credited.
// ---------------------------------------------------------------------------
struct NodeChild(std::process::Child);
impl Drop for NodeChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Minimal JSON-RPC over HTTP/1.1, like the engine's RpcNodeControl
/// transport. `auth_header` is the complete Authorization header value
/// (`Bearer <token>` or `Basic base64(user:password)`).
fn rpc_call(port: u16, auth_header: &str, method: &str, params: Value) -> Result<Value, String> {
    use std::io::{Read, Write};
    let text = serde_json::to_string(&json!({
        "jsonrpc": "2.0", "id": 1, "method": method, "params": params
    }))
    .unwrap();
    let mut stream =
        std::net::TcpStream::connect(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    stream
        .set_write_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    let request = format!(
        "POST / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: {auth_header}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
        text.len()
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&bytes);
    let start = text.find("\r\n\r\n").ok_or("no http body")? + 4;
    let value: Value = serde_json::from_str(&text[start..]).map_err(|e| e.to_string())?;
    value
        .get("result")
        .cloned()
        .ok_or_else(|| format!("rpc error: {value}"))
}

/// Bearer-authenticated RPC call (the node's own generated rpc.token).
fn real_rpc_call(port: u16, token: &str, method: &str, params: Value) -> Result<Value, String> {
    rpc_call(port, &format!("Bearer {token}"), method, params)
}

/// Minimal standard base64 (test-only; the engine's adapter does the
/// same encoding for `--rpc-cookie-file` credentials).
fn base64_credentials(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let value = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((value >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((value >> 12) & 0x3f) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((value >> 6) & 0x3f) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(value & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    out
}

struct RealTwoNodeHost {
    control: OperationControl,
    submit_nodes: Mutex<Vec<String>>,
    /// Number of `reserve_hashes` host calls (hash-budget reservations).
    reservations: AtomicUsize,
    observed: AtomicUsize,
    nodes: Vec<(String, u16, String)>,
}

impl BootstrapHost for RealTwoNodeHost {
    fn actor_request(&self, operation: &str, payload: Value) -> Result<Value, String> {
        match operation {
            "funding_address" => {
                Ok(json!({"status":"address","index":0,"script_hex":script().to_hex_string()}))
            }
            "reserve_hashes" => {
                self.reservations.fetch_add(1, Ordering::SeqCst);
                let attempts = payload["attempts"].as_u64().unwrap();
                let a = HashAuthorization {
                    operation_id: payload["operation_id"].as_str().unwrap().into(),
                    driver_incarnation: "real".into(),
                    batch_id: payload["batch_id"].as_u64().unwrap(),
                    reserved_attempts: attempts,
                    cumulative_reserved: attempts,
                };
                Ok(
                    json!({"status":"authorized","execute":true,"reservation":HashReservationStatus{
                    authorization:a,duplicate:false,reconciled:false,measured_attempts:None}}),
                )
            }
            "reconcile_hashes" => Ok(json!({"status":"reconciled"})),
            "observe_bootstrap_coinbase" => {
                self.observed.fetch_add(1, Ordering::SeqCst);
                Ok(json!({"status":"funding_observed"}))
            }
            "bootstrap_rpc" => {
                let node_id = payload["node_id"]
                    .as_str()
                    .ok_or("node_id missing")?
                    .to_string();
                let method = payload["method"].as_str().ok_or("method missing")?;
                let params = payload["params"].clone();
                if method == "submitblock" {
                    self.submit_nodes.lock().unwrap().push(node_id.clone());
                }
                let (port, auth) = self
                    .nodes
                    .iter()
                    .find(|(id, _, _)| *id == node_id)
                    .map(|(_, port, auth)| (*port, auth.clone()))
                    .ok_or_else(|| format!("unknown node {node_id}"))?;
                let result = rpc_call(port, &auth, method, params)?;
                Ok(json!({"status":"rpc_result","result":RpcOutcome::Ok{result}}))
            }
            other => Err(format!("unexpected real operation {other}")),
        }
    }
}

fn real_run<'a>(
    config: &'a ActorConfig,
    host: Arc<RealTwoNodeHost>,
    total: u64,
    command_id: &str,
    node_id: &str,
) -> Run<'a> {
    let request = BootstrapRequest {
        command_id: command_id.into(),
        driver_id: "txd-0".into(),
        node_id: node_id.into(),
        boot_id: boot_id().unwrap(),
        deadline_monotonic_ms: monotonic_ms().unwrap() + 60_000,
        cancel_grace_secs: 30,
    };
    let control = host.control.clone();
    Run {
        rpc: HostRpc {
            host,
            request,
            control,
        },
        config,
        evidence: tempfile::tempfile().unwrap(),
        budget: HashBudget {
            hashes_per_second: Some(1_000_000),
            total_attempts: Some(total),
        },
        cumulative_reserved: 0,
        next_batch: 1,
        consumption: BatchConsumption::new(command_id.into(), "real".into()),
        pacer: Pacer::new(Instant::now()),
        entropy: 0,
        parent_context: None,
    }
}

#[test]
fn real_two_node_propagation_submits_to_authority_and_observes_both() {
    let Ok(program) = std::env::var("SPC_LAB_RBITCOIN_NODE") else {
        eprintln!("skipping: SPC_LAB_RBITCOIN_NODE is not set");
        return;
    };
    let deadline = Instant::now() + std::time::Duration::from_secs(60);
    let dir = tempfile::tempdir().unwrap();
    let rpc0 = free_port();
    let rpc1 = free_port();
    let p2p0 = free_port();
    let p2p1 = free_port();

    // Ring peering, exactly the argv the engine plans for a two-node host
    // run: each node listens on its own leased loopback P2P port and dials
    // its ring successor.
    let mut children = Vec::new();
    for (id, rpc_port, listen_port, connect_port) in
        [("node-0", rpc0, p2p0, p2p1), ("node-1", rpc1, p2p1, p2p0)]
    {
        let datadir = dir.path().join(id);
        let mut child = std::process::Command::new(&program)
            .args([
                "--network",
                "regtest",
                "--datadir",
                datadir.to_str().unwrap(),
                "--rpc",
                "--rpc-listen",
                &format!("127.0.0.1:{rpc_port}"),
                "--no-seeds",
                "--no-discover",
                "--max-outbound",
                "1",
                "--listen",
                &format!("127.0.0.1:{listen_port}"),
                "--connect",
                &format!("127.0.0.1:{connect_port}"),
                "--log-level",
                "error",
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap_or_else(|e| panic!("spawn {program}: {e}"));
        // Fail fast when the node cannot start at all.
        if let Some(status) = child.try_wait().unwrap() {
            panic!("node {id} exited immediately: {status}");
        }
        children.push(NodeChild(child));
    }

    // Wait for the RPC listeners, then read the Bearer tokens.
    let mut tokens = Vec::new();
    for (id, rpc_port, _) in [("node-0", rpc0, 0u16), ("node-1", rpc1, 0u16)] {
        loop {
            if Instant::now() > deadline {
                panic!("node {id} RPC not ready before deadline");
            }
            if std::net::TcpStream::connect_timeout(
                &format!("127.0.0.1:{rpc_port}").parse().unwrap(),
                std::time::Duration::from_millis(200),
            )
            .is_ok()
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let token_path = dir.path().join(id).join("rpc.token");
        let token = std::fs::read_to_string(&token_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", token_path.display()))
            .trim()
            .to_string();
        assert!(!token.is_empty(), "empty rpc token for {id}");
        tokens.push(token);
    }

    // Wait for observed P2P connectivity on both nodes.
    for (index, id) in ["node-0", "node-1"].iter().enumerate() {
        loop {
            if Instant::now() > deadline {
                panic!("node {id} never reported a peer before deadline");
            }
            if let Ok(peers) = real_rpc_call(
                [rpc0, rpc1][index],
                &tokens[index],
                "getpeerinfo",
                json!([]),
            ) {
                if peers.as_array().is_some_and(|entries| !entries.is_empty()) {
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
    }

    let host = Arc::new(RealTwoNodeHost {
        control: OperationControl::default(),
        submit_nodes: Mutex::new(Vec::new()),
        reservations: AtomicUsize::new(0),
        observed: AtomicUsize::new(0),
        nodes: vec![
            ("node-0".into(), rpc0, format!("Bearer {}", tokens[0])),
            ("node-1".into(), rpc1, format!("Bearer {}", tokens[1])),
        ],
    });
    let cfg = config();
    let mut run = real_run(&cfg, host.clone(), 128, "operation", "node-0");
    let (block, height, _) = run.mine_block().unwrap();
    assert_eq!(height, 1);
    assert!(block.header.validate_pow(block.header.target()).is_ok());
    assert!(run.pacer.attempted() > 0);

    // The bootstrap observed funding exactly once, after every configured
    // node corroborated the common chain.
    assert_eq!(host.observed.load(Ordering::SeqCst), 1);
    // The block was submitted ONLY to the authority node.
    assert_eq!(
        *host.submit_nodes.lock().unwrap(),
        vec!["node-0".to_string()],
        "submitblock must target the authority node only"
    );

    // Both real nodes have the mined block at height 1.
    let hash = block.block_hash().to_string();
    for (index, id) in ["node-0", "node-1"].iter().enumerate() {
        let count = real_rpc_call(
            [rpc0, rpc1][index],
            &tokens[index],
            "getblockcount",
            json!([]),
        )
        .unwrap();
        assert_eq!(count, json!(1), "node {id} block count");
        let tip = real_rpc_call(
            [rpc0, rpc1][index],
            &tokens[index],
            "getblockhash",
            json!([1]),
        )
        .unwrap();
        assert_eq!(
            tip,
            json!(hash),
            "node {id} must have propagated the mined block"
        );
    }
}

// ---------------------------------------------------------------------------
// Exact AP5-profile strict readiness regression (env-gated):
// SPC_LAB_RBITCOIN_NODE=/path/to/rbitcoin-node
//
// Launches the two nodes with the exact AP5 diagnostic argv profile
// (leased loopback ports, --rpc-cookie-file, --regtest-pow-bits
// 0x200fffff, --api-log, --rpc-compat-profile bark-v31, ring
// --listen/--connect) and proves, with the shared
// receiver_handshake_with_owned_reverse_tcp observer, that BOTH planned
// directed dials are live at genesis BEFORE the sole hash budget of the
// operation is reserved, then that an authority-only submission
// propagates the first real block to the second node. The propagation
// test above only waited for nonempty peer arrays; it does not exercise
// this gate.
// ---------------------------------------------------------------------------

/// The AP5 chain policy, mirroring exactly what the node's consensus
/// profile reports for `--network regtest --regtest-pow-bits 0x200fffff`:
/// the overlay and initial target are 0x200fffff inside the unchanged
/// regtest pow limit, no retargeting, and default regtest min-difficulty
/// blocks are ON (the node's own profile reports
/// allow_min_difficulty_blocks=true). That matters at height 1: the
/// first block is far past the genesis-time gap, so its expected bits
/// are the min-difficulty pow limit 0x207fffff - the overlay steps the
/// chain onto 0x200fffff only from the next non-boundary height without
/// a gap, exactly as `contextual_pow::expected_bits` encodes.
fn ap5_policy() -> ChainPolicy {
    ChainPolicy {
        genesis: genesis_block(Network::Regtest).block_hash(),
        pow_limit_bits: 0x207fffff,
        initial_target_bits: 0x200fffff,
        no_pow_retargeting: true,
        allow_min_difficulty_blocks: true,
        pow_target_spacing: 600,
        difficulty_adjustment_interval: 2016,
        regtest_pow_bits_overlay: 0x200fffff,
        subsidy_initial_sats: 5_000_000_000,
        subsidy_halving_interval: 150,
    }
}

fn ap5_config() -> ActorConfig {
    let mut cfg = config();
    cfg.chain_policy = ap5_policy();
    cfg.node_ids = vec!["node-a".into(), "node-b".into()];
    cfg.authority_node = "node-a".into();
    cfg
}

#[test]
fn real_ap5_profile_strict_readiness_gate_precedes_sole_hash_start() {
    use crate::net_readiness::{
        capture_bracket, observe_peer_graph, LinkProofFailure, NodeProcessIdentity,
        PeerHandshakeRow, PeerDualSample, PeerSample, PlannedPeers, ReadinessAcquisitionBounds,
        EVIDENCE_FORM, EXPECTED_PROTOCOL_VERSION, EXPECTED_SUBVER, READINESS_CONTRACT_VERSION,
        READINESS_MAX_FD_ENTRIES, READINESS_MAX_PEER_ROWS, READINESS_MAX_RPC_CALL_MS,
        READINESS_MAX_TCP_ROWS,
    };
    let Ok(program) = std::env::var("SPC_LAB_RBITCOIN_NODE") else {
        eprintln!("skipping: SPC_LAB_RBITCOIN_NODE is not set");
        return;
    };
    let deadline = Instant::now() + std::time::Duration::from_secs(120);
    let dir = tempfile::tempdir().unwrap();
    let rpc_a = free_port();
    let rpc_b = free_port();
    let p2p_a = free_port();
    let p2p_b = free_port();

    // Exact AP5 argv profile (archived run shape) on freshly leased
    // loopback ports: node-a listens on p2p_a and dials p2p_b, node-b
    // listens on p2p_b and dials p2p_a.
    let mut children = Vec::new();
    let mut starts = Vec::new();
    for (id, rpc_port, listen_port, connect_port) in [
        ("node-a", rpc_a, p2p_a, p2p_b),
        ("node-b", rpc_b, p2p_b, p2p_a),
    ] {
        let datadir = dir.path().join(format!("node-{id}"));
        let cookie = dir.path().join(format!("node-rpc-{id}.cookie"));
        let api_log = dir.path().join(format!("node-api-{id}.jsonl"));
        // The engine writes the RPC cookie (user:password, HTTP Basic
        // credentials) BEFORE launching the node, owner-only: the pinned
        // node reads the file (it does not generate one) and refuses a
        // cookie whose permissions are wider than owner-only.
        let credentials = format!("{id}-user:{id}-pass");
        std::fs::write(&cookie, &credentials).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&cookie, std::fs::Permissions::from_mode(0o600)).unwrap();
        let mut child = std::process::Command::new(&program)
            .args([
                "--network",
                "regtest",
                "--datadir",
                datadir.to_str().unwrap(),
                "--rpc",
                "--rpc-listen",
                &format!("127.0.0.1:{rpc_port}"),
                "--rpc-cookie-file",
                cookie.to_str().unwrap(),
                "--regtest-pow-bits",
                "0x200fffff",
                "--no-seeds",
                "--no-discover",
                "--max-outbound",
                "1",
                "--api-log",
                api_log.to_str().unwrap(),
                "--log-level",
                "info",
                "--rpc-compat-profile",
                "bark-v31",
                "--listen",
                &format!("127.0.0.1:{listen_port}"),
                "--connect",
                &format!("127.0.0.1:{connect_port}"),
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap_or_else(|e| panic!("spawn {program}: {e}"));
        if let Some(status) = child.try_wait().unwrap() {
            panic!("node {id} exited immediately: {status}");
        }
        starts.push(
            crate::net_readiness::process_start_ticks(child.id())
                .unwrap_or_else(|| panic!("start ticks of {id} unreadable")),
        );
        children.push(NodeChild(child));
    }

    // Wait for the RPC listeners. The credentials were written to the
    // cookie files before launch (the node reads, not writes, them) and
    // authenticate as HTTP Basic, exactly like the engine's host-mode
    // adapter for `--rpc-cookie-file` nodes.
    let mut auth = Vec::new();
    for (id, rpc_port) in [("node-a", rpc_a), ("node-b", rpc_b)] {
        loop {
            if Instant::now() > deadline {
                panic!("node {id} RPC not ready before deadline");
            }
            if std::net::TcpStream::connect_timeout(
                &format!("127.0.0.1:{rpc_port}").parse().unwrap(),
                std::time::Duration::from_millis(200),
            )
            .is_ok()
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let credentials = format!("{id}-user:{id}-pass");
        auth.push(format!(
            "Basic {}",
            base64_credentials(credentials.as_bytes())
        ));
    }

    // The strict readiness gate, using the same shared proof layer the
    // engine gate calls: every planned directed dial must carry a
    // complete receiver-handshake/owned-reverse-TCP proof.
    let planned = PlannedPeers::new(
        std::collections::BTreeMap::from([
            ("node-a".to_string(), format!("127.0.0.1:{p2p_a}")),
            ("node-b".to_string(), format!("127.0.0.1:{p2p_b}")),
        ]),
        std::collections::BTreeMap::from([
            ("node-a".to_string(), format!("127.0.0.1:{p2p_b}")),
            ("node-b".to_string(), format!("127.0.0.1:{p2p_a}")),
        ]),
    )
    .expect("AP5 planned graph");
    let identities = std::collections::BTreeMap::from([
        (
            "node-a".to_string(),
            NodeProcessIdentity {
                node: "node-a".to_string(),
                pid: children[0].0.id(),
                start_time_ticks: starts[0],
                network_namespace: None,
            },
        ),
        (
            "node-b".to_string(),
            NodeProcessIdentity {
                node: "node-b".to_string(),
                pid: children[1].0.id(),
                start_time_ticks: starts[1],
                network_namespace: None,
            },
        ),
    ]);
    // The strict gate loop: panic on refuse, retry on transient states,
    // and yield the complete proofs once every planned directed dial is
    // proven. Contract version 3: each pass takes TWO peer samples (A
    // and B) with stamps taken after their reads completed, inside the
    // bracket interval, and the brackets are bounded by the
    // source-declared cardinalities.
    let bounds = ReadinessAcquisitionBounds {
        max_acquisition_ms: 120_000,
        max_rpc_call_ms: READINESS_MAX_RPC_CALL_MS,
        max_peer_rows: READINESS_MAX_PEER_ROWS,
        max_tcp_rows: READINESS_MAX_TCP_ROWS,
        max_fd_entries: READINESS_MAX_FD_ENTRIES,
    };
    let proofs = loop {
        if Instant::now() > deadline {
            panic!("strict readiness gate never passed before deadline");
        }
        let before = capture_bracket(&identities, &bounds).expect("bracket before sampling");
        // Peer sample A (right after the before bracket): the rows and
        // the stamp taken after the reads completed.
        let mut rows_a = std::collections::BTreeMap::new();
        for (index, id) in ["node-a", "node-b"].iter().enumerate() {
            let peers = rpc_call(
                [rpc_a, rpc_b][index],
                &auth[index],
                "getpeerinfo",
                json!([]),
            )
            .expect("getpeerinfo");
            let mut rows = Vec::new();
            for entry in peers.as_array().expect("getpeerinfo array") {
                rows.push(
                    PeerHandshakeRow::parse(entry)
                        .unwrap_or_else(|e| panic!("malformed getpeerinfo row on {id}: {e}")),
                );
            }
            rows_a.insert(id.to_string(), rows);
        }
        let stamp_a = crate::bootstrap::monotonic_ms().expect("monotonic stamp A");
        // Peer sample B (right before the after bracket).
        let mut rows_b = std::collections::BTreeMap::new();
        for (index, id) in ["node-a", "node-b"].iter().enumerate() {
            let peers = rpc_call(
                [rpc_a, rpc_b][index],
                &auth[index],
                "getpeerinfo",
                json!([]),
            )
            .expect("getpeerinfo");
            let mut rows = Vec::new();
            for entry in peers.as_array().expect("getpeerinfo array") {
                rows.push(
                    PeerHandshakeRow::parse(entry)
                        .unwrap_or_else(|e| panic!("malformed getpeerinfo row on {id}: {e}")),
                );
            }
            rows_b.insert(id.to_string(), rows);
        }
        let stamp_b = crate::bootstrap::monotonic_ms().expect("monotonic stamp B");
        let after = capture_bracket(&identities, &bounds).expect("bracket after sampling");
        let mut samples_by_node = std::collections::BTreeMap::new();
        for (id, a_rows) in rows_a {
            let b_rows = rows_b.remove(&id).expect("both samples cover both nodes");
            samples_by_node.insert(
                id,
                PeerDualSample {
                    a: PeerSample {
                        rows: a_rows,
                        sampled_at_mono_ms: stamp_a,
                    },
                    b: PeerSample {
                        rows: b_rows,
                        sampled_at_mono_ms: stamp_b,
                    },
                },
            );
        }
        let pass = observe_peer_graph(&planned, &samples_by_node, &identities, &before, &after);
        if let Some(LinkProofFailure::Refuse { detail }) = &pass.failure {
            panic!("strict readiness gate refused: {detail}");
        }
        if pass.failure.is_none() && pass.covered_dials.len() == 2 {
            break pass.proofs;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    };

    // Both nodes are still at genesis when the gate passes: the proof
    // precedes any hashing.
    for (index, id) in ["node-a", "node-b"].iter().enumerate() {
        let count = rpc_call(
            [rpc_a, rpc_b][index],
            &auth[index],
            "getblockcount",
            json!([]),
        )
        .unwrap();
        assert_eq!(
            count,
            json!(0),
            "node {id} must still be at genesis when the gate passes"
        );
    }

    // Each directed dial carries a complete typed proof.
    assert_eq!(proofs.len(), 2);
    let mut directions = std::collections::BTreeSet::new();
    for proof in &proofs {
        assert_eq!(proof.contract_version, READINESS_CONTRACT_VERSION);
        assert_eq!(proof.evidence_form, EVIDENCE_FORM);
        // The receiver's completed inbound handshake, exactly as reported.
        assert!(proof.handshake.inbound);
        assert_eq!(proof.handshake.version, EXPECTED_PROTOCOL_VERSION);
        assert_eq!(proof.handshake.subver, EXPECTED_SUBVER);
        assert!(proof.handshake.bytesrecv_version.is_some_and(|n| n > 0));
        assert!(proof.handshake.bytesrecv_verack.is_some_and(|n| n > 0));
        // The exact reversed four-tuple, both sides process-owned, and
        // the acceptor side is the planned listener endpoint.
        assert_eq!(proof.dialer_socket.local, proof.acceptor_socket.remote);
        assert_eq!(proof.dialer_socket.remote, proof.acceptor_socket.local);
        assert_eq!(proof.dialer_socket.remote, proof.planned_target);
        assert_eq!(proof.acceptor_socket.local, proof.planned_target);
        assert!(proof.dialer_socket.fd_owned);
        assert!(proof.acceptor_socket.fd_owned);
        assert_ne!(proof.dialer_socket.inode, 0);
        assert_ne!(proof.acceptor_socket.inode, 0);
        assert!(proof.snapshot_confirmed);
        assert!(proof.start_verified);
        assert!(proof.boot_id.is_some());
        assert!(!proof.getheaders_corroboration_required);
        directions.insert((proof.dialer.clone(), proof.acceptor.clone()));
    }
    assert!(
        directions.contains(&("node-a".to_string(), "node-b".to_string())),
        "the node-a -> node-b directed dial must be proven"
    );
    assert!(
        directions.contains(&("node-b".to_string(), "node-a".to_string())),
        "the node-b -> node-a directed dial must be proven"
    );

    // Optional producer-record dump: with SPC_LAB_AP5_PROOF_OUT set, the
    // real-run proofs are serialized to that path (one compact JSON
    // record per line) so consumers can bind their parser to bytes a
    // real run produced. The test never depends on the file existing.
    if let Ok(path) = std::env::var("SPC_LAB_AP5_PROOF_OUT") {
        use std::io::Write;
        let mut out = std::fs::File::create(&path).unwrap_or_else(|e| panic!("create {path}: {e}"));
        for proof in &proofs {
            let record = serde_json::to_string(proof).expect("proof serializes");
            writeln!(out, "{record}").expect("write proof record");
        }
    }

    let host = Arc::new(RealTwoNodeHost {
        control: OperationControl::default(),
        submit_nodes: Mutex::new(Vec::new()),
        reservations: AtomicUsize::new(0),
        observed: AtomicUsize::new(0),
        nodes: vec![
            ("node-a".into(), rpc_a, auth[0].clone()),
            ("node-b".into(), rpc_b, auth[1].clone()),
        ],
    });
    // No hash budget was reserved before the gate passed.
    assert_eq!(host.reservations.load(Ordering::SeqCst), 0);
    let cfg = ap5_config();
    let mut run = real_run(&cfg, host.clone(), 128, "ap5-strict-readiness", "node-a");
    let (block, height, _) = run.mine_block().unwrap();
    assert_eq!(height, 1);
    assert!(block.header.validate_pow(block.header.target()).is_ok());
    assert!(run.pacer.attempted() > 0);
    // The sole hash budget was reserved only after the gate passed.
    assert!(host.reservations.load(Ordering::SeqCst) >= 1);
    // Funding was observed exactly once, after both nodes corroborated
    // the common chain.
    assert_eq!(host.observed.load(Ordering::SeqCst), 1);
    // The block was submitted ONLY to the authority node.
    assert_eq!(
        *host.submit_nodes.lock().unwrap(),
        vec!["node-a".to_string()],
        "submitblock must target the authority node only"
    );

    // Both real nodes have the mined block at height 1.
    let hash = block.block_hash().to_string();
    for (index, id) in ["node-a", "node-b"].iter().enumerate() {
        let count = rpc_call(
            [rpc_a, rpc_b][index],
            &auth[index],
            "getblockcount",
            json!([]),
        )
        .unwrap();
        assert_eq!(count, json!(1), "node {id} block count");
        let tip = rpc_call(
            [rpc_a, rpc_b][index],
            &auth[index],
            "getblockhash",
            json!([1]),
        )
        .unwrap();
        assert_eq!(
            tip,
            json!(hash),
            "node {id} must have propagated the mined block"
        );
    }
}
