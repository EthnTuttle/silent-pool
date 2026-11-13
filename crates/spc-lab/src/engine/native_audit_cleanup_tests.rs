//! Cleanup-purpose native-audit binding validation
//! (`attach_for_cleanup` / `validate_persisted_native_audit_bindings`): the
//! persisted-only checks that let `reap` reopen a run whose native-audit
//! wallet failed during startup. The scenario is an actually-configured
//! native-audit traffic run (Ark-backed pool, wallet agent with a collector
//! profile), the traffic capability is recorded exactly as the failed AP5
//! Git shared run recorded it, and the wallet process was recorded and
//! stopped before any miner bound - so no wallet instance and no lease
//! projection was ever published. That absence is expected
//! (FAILED/INCOMPLETE provisioning), not a contradiction that blocks
//! resource cleanup: the second actual reaper blocker, pinned end-to-end
//! through `attach_for_cleanup` into the real reaper. A lease projection
//! that IS present is verified exactly as persisted metadata (fail closed
//! on foreign or rewritten records) without any live wallet, socket,
//! instance or ownership proof, and cleanup never signals a process the
//! recorded census does not verify as ours.
use super::*;
use crate::ids::CommandId;
use crate::supervisor::{ControlTransport, ProcessRecord, ProcessState, Readiness};
use crate::traffic::projection::{
    project_traffic_capability, TrafficCapabilityRecord, AP1A_PROBE_ID,
    AP1A_TRAFFIC_CONTRACT_SHA256, TRAFFIC_CONSUMER,
};
use spc_scan::native_activation::{
    PeerBinding, PeerRegistry, ProcessIdentity, WalletInstance, INSTANCE_SCHEMA, REGISTRY_SCHEMA,
};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

/// Two distinct, verified on-curve secp256k1 x coordinates (x^3+7 is a
/// quadratic residue for both, even-y form exists), presented as x-only
/// public keys with no known private keys: the collector profile needs two
/// distinct, canonically-encoded public keys and nothing else.
const POOL_PUBKEY: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f4615b53f16f01";
const COLLECTOR_PUBKEY: &str = "c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5";

/// A traffic run over the one-pool fixture whose pool-a carries an Ark
/// backend and whose alice wallet agent declares the native-audit collector
/// profile - the actual configured shape the failed shared run used. The
/// scenario validates as-is (wallet/front declarations, Ark pool, miner
/// connections, traffic section).
fn native_audit_traffic_engine() -> Arc<RunEngine> {
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
    s["phases"] = serde_json::json!([{"id":"bootstrap","commands":[{"kind":{"type":"bootstrap_fund","driver_id":"txd-0","node_id":s["nodes"][0]["id"]}}]}]);
    s["pools"][0]["ark_backend_id"] = serde_json::json!("ark-a");
    s["ark_backends"] =
        serde_json::json!([{"id":"ark-a","implementation":"fixture-ark","listen":"127.0.0.1:0"}]);
    s["wallet_agents"] = serde_json::json!([
        {"identity_id":"id-alice","front_id":"front-a1","native_audit":{
            "collector_socket":"native-audit/alice/collector.sock",
            "pool_pubkey":POOL_PUBKEY,"collector_pubkey":COLLECTOR_PUBKEY}},
        {"identity_id":"id-bob","front_id":"front-a1"}
    ]);
    let scenario: Scenario = serde_json::from_value(s).unwrap();
    scenario.validate().unwrap();
    let adapter: crate::config::AdapterConfig = serde_json::from_value(serde_json::json!({"program":"/bin/true","args":[],"fixture":true,"capabilities":["fixture.scripted","spc.node.regtest.mempool_rpc.v1","spc.pool.work_unit_v2","spc.pool.network_block_work"],"effective_profile":{
        "implementation":"fixture-node","network":"regtest","consensus_profile_id":"fixture.regtest.retarget","profile_fingerprint":null,"pow_limit_bits":"207fffff","retarget_floor_bits":"1e7fffff","initial_target_bits":"1e7fffff","regtest_pow_bits_overlay":"1e7fffff","target_spacing_secs":10,"target_timespan_secs":240,"retarget_interval_blocks":24,"no_pow_retargeting":false,"allow_min_difficulty_blocks":false,"minimum_difficulty_time_gap_secs":null,"subsidy_initial_sats":"5000000000","subsidy_halving_interval_blocks":150,"coinbase_maturity_blocks":100,"witness_commitment_supported":true,"tdp_session_cap":8},"rpc_auth":null,"env":{}})).unwrap();
    let mut config = LabConfig::default();
    config.nodes.insert("fixture-node".into(), adapter.clone());
    config.pools.insert("fixture-pool".into(), adapter.clone());
    config
        .miners
        .insert("fixture-miner".into(), adapter.clone());
    config.redis = Some(adapter.clone());
    config.postgres = Some(adapter.clone());
    config.ark.insert("fixture-ark".into(), adapter.clone());
    let mut native = adapter;
    native.fixture = false;
    config.traffic_probe = Some(native.clone());
    config.tx_driver = Some(native);
    let root =
        PathBuf::from("/tmp/opencode").join(format!("native-audit-reap-{}", CommandId::generate()));
    std::fs::create_dir(&root).unwrap();
    RunEngine::prepare(
        scenario,
        EngineOptions {
            run_dir: root.clone(),
            config,
            base_revision: "native-audit-reap-test".into(),
            dirty_tree_hash: "fixture".into(),
            owner_instance: "native-audit-reap-test".into(),
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
    .unwrap()
}

// ---------------------------------------------------------------------------
// Traffic capability fixture: the projection of a full passing AP-1a report
// over the production-approved contract, exactly the state the failed
// shared run recorded (`traffic_capability_recorded: true`).
// ---------------------------------------------------------------------------

fn check_row(id: &str, method: Option<&str>, availability: &str, behavior: &str) -> Value {
    serde_json::json!({
        "id": id,
        "method": method,
        "availability": availability,
        "behavior": behavior,
        "detail": "synthetic test row"
    })
}

fn passing_five_method_checks() -> Vec<Value> {
    vec![
        check_row(
            "sendrawtransaction.recognized",
            Some("sendrawtransaction"),
            "passed",
            "passed",
        ),
        check_row("getrawmempool", Some("getrawmempool"), "passed", "passed"),
        check_row(
            "getrawmempool.bool-form",
            Some("getrawmempool"),
            "passed",
            "passed",
        ),
        check_row(
            "getrawtransaction.verbosity0",
            Some("getrawtransaction"),
            "passed",
            "passed",
        ),
        check_row(
            "getrawtransaction.verbosity1",
            Some("getrawtransaction"),
            "passed",
            "passed",
        ),
        check_row(
            "getrawtransaction.bool-verbose",
            Some("getrawtransaction"),
            "passed",
            "passed",
        ),
        check_row(
            "getrawtransaction.bool-verbose.async",
            Some("getrawtransaction"),
            "passed",
            "passed",
        ),
        check_row(
            "getmempoolinfo.traffic-consumer",
            Some("getmempoolinfo"),
            "passed",
            "passed",
        ),
        check_row("getblock.verbosity0", Some("getblock"), "passed", "passed"),
        check_row(
            "getblock.verbosity0.rehash",
            Some("getblock"),
            "passed",
            "passed",
        ),
        check_row("getblock.verbosity1", Some("getblock"), "passed", "passed"),
        check_row("getblock.singleparam", Some("getblock"), "passed", "passed"),
        check_row(
            "getrawtransaction.blockhash.traffic-consumer",
            Some("getrawtransaction"),
            "passed",
            "passed",
        ),
    ]
}

fn passing_foundational_checks() -> Vec<Value> {
    vec![
        check_row(
            "getbestblockhash",
            Some("getbestblockhash"),
            "passed",
            "passed",
        ),
        check_row(
            "getblockheader.traffic-consumer",
            Some("getblockheader"),
            "passed",
            "passed",
        ),
        check_row("protocol.id-echo", None, "passed", "passed"),
        check_row(
            "protocol.envelope20.getnetworkinfo",
            Some("getnetworkinfo"),
            "passed",
            "passed",
        ),
        check_row(
            "getblockchaininfo.network",
            Some("getblockchaininfo"),
            "passed",
            "passed",
        ),
        check_row(
            "getblockhash.genesis",
            Some("getblockhash"),
            "passed",
            "passed",
        ),
        check_row(
            "getblockheader.verbose.genesis",
            Some("getblockheader"),
            "passed",
            "passed",
        ),
    ]
}

fn version_gate_checks() -> Vec<Value> {
    vec![
        check_row(
            "getnetworkinfo.gate.server",
            Some("getnetworkinfo"),
            "passed",
            "failed",
        ),
        check_row(
            "getnetworkinfo.gate.wallet",
            Some("getnetworkinfo"),
            "passed",
            "failed",
        ),
    ]
}

fn full_model_failure() -> Value {
    let mut row = check_row("getmempoolinfo", Some("getmempoolinfo"), "passed", "failed");
    row["detail"] = serde_json::json!("shape: maxdatacarriersize: expected JSON integer, got null");
    row["fail_kind"] = serde_json::json!("shape");
    row
}

fn report_json(mut checks: Vec<Value>) -> Value {
    if !checks.iter().any(|row| row["id"] == "getmempoolinfo") {
        checks.push(full_model_failure());
    }
    let failures: Vec<_> = checks
        .iter()
        .filter(|row| {
            row["id"] == "getmempoolinfo"
                && (row["availability"] == "failed" || row["behavior"] == "failed")
        })
        .cloned()
        .collect();
    serde_json::json!({
        "probe": AP1A_PROBE_ID,
        "probe_version": "1.0.0",
        "schema_version": 2,
        "generated_utc": "2026-11-08T00:00:00Z",
        "run_token": "AP1a-test",
        "target": {"label": "test-node", "kind": "owned-fresh-regtest-node"},
        "profile": {
            "name": "test",
            "server_min_rpc_version": 310000,
            "wallet_min_rpc_version": 290000,
            "expected_network": "regtest",
            "expected_genesis":
                "0f9188f13cb7b2c71f2a335e3a4fc328bf5beb436012afca590b1a11433e0ad2"
        },
        "contract_hash": AP1A_TRAFFIC_CONTRACT_SHA256,
        "traffic_consumer": TRAFFIC_CONSUMER,
        "consumer_model_failures": failures,
        "checks": checks,
        "node": {
            "label": "test-node",
            "binary": "/tmp/test/rbitcoin-node",
            "binary_sha256":
                "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
        },
        "observations": {
            "mempool_size": 0,
            "traffic_fee_policy": {
                "minrelaytxfee_btc_kb_lexeme": "0.00001000",
                "minrelaytxfee_sats_kb": 1000,
                "maxmempool": 300000000, "bytes": 0, "usage": 0, "errors": []
            }
        },
        "verdict": {
            "status": "surface_compatible",
            "reasons": ["funded-unproven"],
            "summary": "synthetic test report",
            "funded_state": "unproven",
            "full_acceptance": false
        }
    })
}

fn recorded_traffic_capability() -> TrafficCapabilityRecord {
    let mut checks = passing_five_method_checks();
    checks.extend(passing_foundational_checks());
    checks.extend(version_gate_checks());
    project_traffic_capability(report_json(checks).to_string().as_bytes()).unwrap()
}

// ---------------------------------------------------------------------------
// Persisted wallet/miner record and lease-projection fixtures.
// ---------------------------------------------------------------------------

fn current_uid() -> u32 {
    std::fs::metadata("/proc/self").unwrap().uid()
}

fn current_boot_id() -> String {
    std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .unwrap()
        .trim()
        .to_string()
}

fn write_private(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .unwrap();
    file.write_all(bytes).unwrap();
}

/// The PID is impossible on purpose: cleanup validation must not touch live
/// processes, and `reap` reports the stopped record without signalling it.
fn native_record(
    engine: &RunEngine,
    role: &str,
    entity: &str,
    config_path: &Path,
    state: ProcessState,
) -> ProcessRecord {
    ProcessRecord {
        role: role.into(),
        entity: entity.into(),
        pid: u32::MAX,
        pgid: u32::MAX,
        start_time_ticks: 1,
        executable_path: PathBuf::from("/bin/true"),
        executable_sha256: crate::supervisor::hash_file(Path::new("/bin/true")).unwrap(),
        run_token: "native-audit-reap-test".into(),
        args: vec![
            "/bin/true".into(),
            "--config".into(),
            config_path.to_string_lossy().into_owned(),
        ],
        log_path: engine.run_dir.join(format!("{role}-{entity}.log")),
        readiness: Readiness::Immediate,
        state,
        adopted: false,
        control: ControlTransport::None,
    }
}

fn process_identity(record: &ProcessRecord, config_bytes: &[u8]) -> ProcessIdentity {
    ProcessIdentity {
        pid: record.pid,
        uid: current_uid(),
        start_time_ticks: record.start_time_ticks,
        boot_id: current_boot_id(),
        executable_sha256: hex::decode(&record.executable_sha256)
            .unwrap()
            .try_into()
            .unwrap(),
        run_token_sha256: hash(record.run_token.as_bytes()),
        config_sha256: hash(config_bytes),
    }
}

fn record_config_path(record: &ProcessRecord) -> PathBuf {
    record
        .args
        .windows(2)
        .find(|w| w[0] == "--config")
        .map(|w| PathBuf::from(&w[1]))
        .unwrap()
}

/// Write the wallet and miner config files a launched native-audit run
/// persists, and return their bytes for identity construction.
fn install_native_configs(engine: &RunEngine) -> (PathBuf, Vec<u8>, PathBuf, Vec<u8>) {
    let wallet_config = engine
        .run_dir
        .join("private/native-audit/alice/wallet.toml");
    let miner_config = engine.run_dir.join("private/native-audit/miner-1.toml");
    let wallet_bytes = b"[upstream]\n[native_evidence]\n".to_vec();
    let miner_bytes = b"[native_evidence]\n".to_vec();
    write_private(&wallet_config, &wallet_bytes);
    write_private(&miner_config, &miner_bytes);
    (wallet_config, wallet_bytes, miner_config, miner_bytes)
}

/// A regtest SP address for the fixture identity, from the same well-known
/// in-tree test secrets `wallet_aggregate_binding_tests` uses (not run
/// secrets; the instance validation parses the address and requires its
/// b_scan x-only key to be distinct from the collector/pool keys).
fn fixture_sp_address() -> String {
    use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
    let secp = Secp256k1::new();
    let scan = PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&[3; 32]).unwrap());
    let spend = PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&[4; 32]).unwrap());
    spc_crypto::address::encode_sp_address(spc_crypto::Network::Regtest, &scan, &spend).unwrap()
}

/// Install a consistent persisted lease projection: wallet instance bound to
/// the recorded wallet process, a peer registry whose single miner peer
/// matches the recorded miner process, and the lease file pointing at the
/// registry with its exact hash. Returns the leases path.
fn install_lease_projection(
    engine: &RunEngine,
    wallet_record: &ProcessRecord,
    wallet_config_bytes: &[u8],
    miner_record: &ProcessRecord,
    miner_config_bytes: &[u8],
) -> PathBuf {
    let identity = fixture_sp_address();
    let instance = WalletInstance {
        schema: INSTANCE_SCHEMA.into(),
        instance_id: [7; 16],
        run_id: engine.run_id.as_str().to_string(),
        identity_id: "id-alice".into(),
        identity: identity.clone(),
        pool_id: "pool-a".into(),
        front_id: "front-a1".into(),
        process: process_identity(wallet_record, wallet_config_bytes),
        config_path: record_config_path(wallet_record)
            .to_string_lossy()
            .into_owned(),
        socket_path: engine
            .run_dir
            .join("private/native-audit/alice/wallet.sock")
            .to_string_lossy()
            .into_owned(),
        socket_device: 0,
        socket_inode: 1,
        collector_pubkey: COLLECTOR_PUBKEY.into(),
        pool_pubkey: POOL_PUBKEY.into(),
    };
    let registry = PeerRegistry {
        schema: REGISTRY_SCHEMA.into(),
        run_id: engine.run_id.as_str().to_string(),
        wallet_instance: instance.instance_id,
        generation: 1,
        peers: vec![PeerBinding {
            miner_id: "miner-1".into(),
            connection_id: "conn-alice".into(),
            identity,
            pool_id: "pool-a".into(),
            front_id: "front-a1".into(),
            generation: 1,
            process: process_identity(miner_record, miner_config_bytes),
            config_path: record_config_path(miner_record)
                .to_string_lossy()
                .into_owned(),
            connection_config_digest: [0; 32],
        }],
    };
    let registry_path = engine
        .run_dir
        .join("private/native-audit/alice/registry.json");
    let registry_bytes = serde_json::to_vec_pretty(&registry).unwrap();
    write_private(&registry_path, &registry_bytes);
    let leases_path = engine.run_dir.join("private/native-audit-leases.json");
    let leases = Leases {
        schema: "spc.native-audit-leases/1".into(),
        run_id: engine.run_id.as_str().to_string(),
        leases: BTreeMap::from([(
            "wallet.id-alice.native_audit".to_string(),
            Lease {
                instance,
                registry_path,
                registry_sha256: hash(&registry_bytes),
            },
        )]),
    };
    write_private(&leases_path, &serde_json::to_vec_pretty(&leases).unwrap());
    leases_path
}

fn leases_path(engine: &RunEngine) -> PathBuf {
    engine.run_dir.join("private/native-audit-leases.json")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn cleanup_attaches_a_partial_native_wallet_startup_run() {
    // The AP5 Git shared-run reap blocker, pinned end-to-end at the engine
    // boundary: the traffic capability IS recorded (the Git fix worked),
    // the wallet/alice process was recorded and then failed during
    // readiness (the token contract bug), so no miner ever bound and no
    // native-audit lease projection was published. The post-attach
    // validate_native_audit_attach call made cleanup demand the missing
    // lease and `reap` exited 1 with
    // `native_audit_unavailable: native audit lease projection missing on
    // adopt`. Cleanup must attach, reap the owned record, report the
    // never-launched node, and invent no lease, command or capability.
    let engine = native_audit_traffic_engine();
    let (wallet_config, _wallet_bytes, _miner_config, _miner_bytes) =
        install_native_configs(&engine);
    let wallet_record = native_record(
        &engine,
        "wallet",
        "id-alice",
        &wallet_config,
        ProcessState::Stopped,
    );
    {
        let mut manifest = engine.manifest.lock().unwrap();
        manifest.traffic_capability = Some(recorded_traffic_capability());
        manifest.upsert_process(wallet_record);
        manifest.save(&engine.manifest_path).unwrap();
    }
    let commands = engine.run_dir.join("private/commands.jsonl");
    let commands_before = std::fs::read(&commands).unwrap();
    assert!(!leases_path(&engine).exists());

    // The Live gate stays strict: no lease projection, no live adopt of a
    // native-audit wallet run.
    let live_native = engine.validate_native_audit_attach().unwrap_err();
    assert_eq!(live_native.code(), "native_audit_unavailable");
    assert!(
        live_native
            .to_string()
            .contains("native audit lease projection missing on adopt"),
        "{live_native}"
    );

    let run_dir = engine.run_dir.clone();
    drop(engine); // Release the owner lock.
                  // Live attach refuses the partial run (the direct native gate call
                  // above pins the native strictness; the adopt path refuses earlier on
                  // the missing probe report/launch profiles).
    assert!(RunEngine::attach(&run_dir, "native-reap-live").is_err());

    // Cleanup attaches the partial run - this was the blocker.
    let reaper = RunEngine::attach_for_cleanup(&run_dir, "native-reap-cleanup").unwrap();
    let results = reaper.reap().unwrap();
    assert_eq!(
        results,
        vec![
            (
                "wallet".to_string(),
                "id-alice".to_string(),
                ProcessState::Exited
            ),
            (
                "node".to_string(),
                "node-a".to_string(),
                ProcessState::Exited
            ),
        ],
        "the recorded wallet reports from its record; the never-launched node reports expected-not-launched"
    );

    // Cleanup invented nothing: no lease projection, no command, and the
    // recorded capability is kept, not erased or re-granted.
    assert!(
        !leases_path(&reaper).exists(),
        "cleanup must not fabricate a native-audit lease projection"
    );
    assert_eq!(std::fs::read(&commands).unwrap(), commands_before);
    let after = reaper.manifest_snapshot();
    assert!(after.traffic_capability.is_some());
    assert_eq!(after.state, RunState::Created);
    drop(reaper); // Release the owner lock again.

    // Idempotent: a second cleanup reports the same thing.
    let again = RunEngine::attach_for_cleanup(&run_dir, "native-reap-cleanup-2").unwrap();
    assert_eq!(
        again.reap().unwrap(),
        vec![
            (
                "wallet".to_string(),
                "id-alice".to_string(),
                ProcessState::Exited
            ),
            (
                "node".to_string(),
                "node-a".to_string(),
                ProcessState::Exited
            ),
        ]
    );
}

#[test]
fn cleanup_rejects_a_foreign_native_lease_projection() {
    // A lease projection that IS present is verified: a foreign run id
    // (even with an empty lease map) fails closed - cleanup never ignores
    // an existing foreign record.
    let engine = native_audit_traffic_engine();
    let foreign = Leases {
        schema: "spc.native-audit-leases/1".into(),
        run_id: "another-run".into(),
        leases: BTreeMap::new(),
    };
    write_private(
        &leases_path(&engine),
        &serde_json::to_vec_pretty(&foreign).unwrap(),
    );
    let run_dir = engine.run_dir.clone();
    drop(engine); // Release the owner lock.
    let error = RunEngine::attach_for_cleanup(&run_dir, "foreign-lease-reap").unwrap_err();
    assert_eq!(error.code(), "native_audit_unavailable");
    assert!(
        error
            .to_string()
            .contains("native lease projection foreign"),
        "{error}"
    );
}

#[test]
fn a_present_lease_projection_is_validated_as_persisted_metadata() {
    // The happy persisted path: a lease projection published while the run
    // was live is accepted by cleanup as persisted metadata alone - the
    // recorded processes are long gone (impossible PIDs, stopped states),
    // no live socket, instance file or ownership proof exists, and cleanup
    // still reaps every recorded process plus the never-launched node.
    let engine = native_audit_traffic_engine();
    let (wallet_config, wallet_bytes, miner_config, miner_bytes) = install_native_configs(&engine);
    let wallet_record = native_record(
        &engine,
        "wallet",
        "id-alice",
        &wallet_config,
        ProcessState::Stopped,
    );
    let miner_record = native_record(
        &engine,
        "miner",
        "miner-1",
        &miner_config,
        ProcessState::Stopped,
    );
    {
        let mut manifest = engine.manifest.lock().unwrap();
        manifest.traffic_capability = Some(recorded_traffic_capability());
        manifest.upsert_process(wallet_record.clone());
        manifest.upsert_process(miner_record.clone());
        manifest.save(&engine.manifest_path).unwrap();
    }
    let leases = install_lease_projection(
        &engine,
        &wallet_record,
        &wallet_bytes,
        &miner_record,
        &miner_bytes,
    );
    let leases_before = std::fs::read(&leases).unwrap();
    let commands = engine.run_dir.join("private/commands.jsonl");
    let commands_before = std::fs::read(&commands).unwrap();
    let run_dir = engine.run_dir.clone();
    drop(engine); // Release the owner lock.

    // The live gate still refuses: the recorded processes are not live.
    assert!(RunEngine::attach(&run_dir, "persisted-lease-live").is_err());

    let reaper = RunEngine::attach_for_cleanup(&run_dir, "persisted-lease-reap").unwrap();
    assert_eq!(
        reaper.reap().unwrap(),
        vec![
            (
                "miner".to_string(),
                "miner-1".to_string(),
                ProcessState::Exited
            ),
            (
                "wallet".to_string(),
                "id-alice".to_string(),
                ProcessState::Exited
            ),
            (
                "node".to_string(),
                "node-a".to_string(),
                ProcessState::Exited
            ),
        ]
    );
    // Cleanup rewrote neither the lease projection nor the command log.
    assert_eq!(std::fs::read(&leases).unwrap(), leases_before);
    assert_eq!(std::fs::read(&commands).unwrap(), commands_before);
}

#[test]
fn a_present_lease_with_a_rewritten_registry_rejects() {
    let engine = native_audit_traffic_engine();
    let (wallet_config, wallet_bytes, miner_config, miner_bytes) = install_native_configs(&engine);
    let wallet_record = native_record(
        &engine,
        "wallet",
        "id-alice",
        &wallet_config,
        ProcessState::Stopped,
    );
    let miner_record = native_record(
        &engine,
        "miner",
        "miner-1",
        &miner_config,
        ProcessState::Stopped,
    );
    {
        let mut manifest = engine.manifest.lock().unwrap();
        manifest.upsert_process(wallet_record.clone());
        manifest.upsert_process(miner_record.clone());
        manifest.save(&engine.manifest_path).unwrap();
    }
    install_lease_projection(
        &engine,
        &wallet_record,
        &wallet_bytes,
        &miner_record,
        &miner_bytes,
    );
    // Rewrite the registry the lease points at: the recorded hash no
    // longer matches, so cleanup fails closed on rewritten history.
    let registry_path = engine
        .run_dir
        .join("private/native-audit/alice/registry.json");
    std::fs::write(&registry_path, b"{\"rewritten\":true}").unwrap();
    std::fs::set_permissions(&registry_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let run_dir = engine.run_dir.clone();
    drop(engine); // Release the owner lock.
    let error = RunEngine::attach_for_cleanup(&run_dir, "rewritten-registry-reap").unwrap_err();
    assert_eq!(error.code(), "native_audit_unavailable");
    assert!(
        error
            .to_string()
            .contains("native registry projection mismatch"),
        "{error}"
    );
}

#[test]
fn a_present_lease_with_a_replaced_wallet_record_rejects() {
    let engine = native_audit_traffic_engine();
    let (wallet_config, wallet_bytes, miner_config, miner_bytes) = install_native_configs(&engine);
    let wallet_record = native_record(
        &engine,
        "wallet",
        "id-alice",
        &wallet_config,
        ProcessState::Stopped,
    );
    let miner_record = native_record(
        &engine,
        "miner",
        "miner-1",
        &miner_config,
        ProcessState::Stopped,
    );
    {
        let mut manifest = engine.manifest.lock().unwrap();
        manifest.upsert_process(wallet_record.clone());
        manifest.upsert_process(miner_record.clone());
        manifest.save(&engine.manifest_path).unwrap();
    }
    install_lease_projection(
        &engine,
        &wallet_record,
        &wallet_bytes,
        &miner_record,
        &miner_bytes,
    );
    // Replace the persisted wallet record AFTER the lease was published:
    // the lease's recorded identity no longer matches the manifest.
    let mut replaced = wallet_record;
    replaced.pid = u32::MAX - 1;
    {
        let mut manifest = engine.manifest.lock().unwrap();
        manifest.upsert_process(replaced);
        manifest.save(&engine.manifest_path).unwrap();
    }
    let run_dir = engine.run_dir.clone();
    drop(engine); // Release the owner lock.
    let error = RunEngine::attach_for_cleanup(&run_dir, "replaced-wallet-reap").unwrap_err();
    assert_eq!(error.code(), "native_audit_unavailable");
    assert!(
        error.to_string().contains("native wallet record replaced"),
        "{error}"
    );
}

#[test]
fn a_present_lease_with_a_replaced_miner_peer_record_rejects() {
    let engine = native_audit_traffic_engine();
    let (wallet_config, wallet_bytes, miner_config, miner_bytes) = install_native_configs(&engine);
    let wallet_record = native_record(
        &engine,
        "wallet",
        "id-alice",
        &wallet_config,
        ProcessState::Stopped,
    );
    let miner_record = native_record(
        &engine,
        "miner",
        "miner-1",
        &miner_config,
        ProcessState::Stopped,
    );
    {
        let mut manifest = engine.manifest.lock().unwrap();
        manifest.upsert_process(wallet_record.clone());
        manifest.upsert_process(miner_record.clone());
        manifest.save(&engine.manifest_path).unwrap();
    }
    install_lease_projection(
        &engine,
        &wallet_record,
        &wallet_bytes,
        &miner_record,
        &miner_bytes,
    );
    // Replace the persisted miner record the registry peer binds to.
    let mut replaced = miner_record;
    replaced.start_time_ticks = 2;
    {
        let mut manifest = engine.manifest.lock().unwrap();
        manifest.upsert_process(replaced);
        manifest.save(&engine.manifest_path).unwrap();
    }
    let run_dir = engine.run_dir.clone();
    drop(engine); // Release the owner lock.
    let error = RunEngine::attach_for_cleanup(&run_dir, "replaced-peer-reap").unwrap_err();
    assert_eq!(error.code(), "native_audit_unavailable");
    assert!(
        error.to_string().contains("native peer record replaced"),
        "{error}"
    );
}

#[test]
fn a_present_lease_without_a_wallet_record_rejects() {
    // A published lease whose wallet process record is gone from the
    // manifest is a persisted contradiction: cleanup fails closed instead
    // of trusting the lease alone.
    let engine = native_audit_traffic_engine();
    let (wallet_config, wallet_bytes, miner_config, miner_bytes) = install_native_configs(&engine);
    let wallet_record = native_record(
        &engine,
        "wallet",
        "id-alice",
        &wallet_config,
        ProcessState::Stopped,
    );
    let miner_record = native_record(
        &engine,
        "miner",
        "miner-1",
        &miner_config,
        ProcessState::Stopped,
    );
    // Only the miner is recorded; the lease still references the wallet.
    {
        let mut manifest = engine.manifest.lock().unwrap();
        manifest.upsert_process(miner_record.clone());
        manifest.save(&engine.manifest_path).unwrap();
    }
    install_lease_projection(
        &engine,
        &wallet_record,
        &wallet_bytes,
        &miner_record,
        &miner_bytes,
    );
    let run_dir = engine.run_dir.clone();
    drop(engine); // Release the owner lock.
    let error = RunEngine::attach_for_cleanup(&run_dir, "missing-wallet-record-reap").unwrap_err();
    assert_eq!(error.code(), "native_audit_unavailable");
    assert!(
        error
            .to_string()
            .contains("native record missing from manifest"),
        "{error}"
    );
}

#[test]
fn cleanup_never_signals_a_foreign_wallet_pid() {
    // Safe PID identity on the native-audit path: a live foreign process
    // recorded in the wallet record with a stale start-time tick fails
    // verification as PidReused. Cleanup reports OrphanUnverified without
    // signalling the foreign process, which survives untouched - even
    // though the run has a configured native-audit wallet agent.
    let engine = native_audit_traffic_engine();
    let (wallet_config, _wallet_bytes, _miner_config, _miner_bytes) =
        install_native_configs(&engine);
    let mut wallet_record = native_record(
        &engine,
        "wallet",
        "id-alice",
        &wallet_config,
        ProcessState::Running,
    );
    let mut foreign = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    wallet_record.pid = foreign.id();
    wallet_record.pgid = foreign.id();
    wallet_record.start_time_ticks = 1; // Never this process's real start time.
    {
        let mut manifest = engine.manifest.lock().unwrap();
        manifest.upsert_process(wallet_record);
        manifest.save(&engine.manifest_path).unwrap();
    }
    let run_dir = engine.run_dir.clone();
    drop(engine); // Release the owner lock.
    let reaper = RunEngine::attach_for_cleanup(&run_dir, "foreign-wallet-pid-reap").unwrap();
    let results = reaper.reap().unwrap();
    assert_eq!(
        results,
        vec![
            (
                "wallet".to_string(),
                "id-alice".to_string(),
                ProcessState::OrphanUnverified
            ),
            (
                "node".to_string(),
                "node-a".to_string(),
                ProcessState::Exited
            ),
        ],
        "a reused or foreign wallet PID is never signalled"
    );
    assert!(
        foreign.try_wait().unwrap().is_none(),
        "the foreign process must survive cleanup untouched"
    );
    let _ = foreign.kill();
    let _ = foreign.wait();
}
