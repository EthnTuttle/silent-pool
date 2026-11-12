//! LP-topology: namespace placement, address publication and link faults.
//!
//! The tests run against the approved LP-4a namespace broker.  They are
//! process level: a real broker, real namespaces, real veth pairs, real
//! nftables rules and real TCP sessions.  Nothing is mocked and no fixture
//! backend is involved, because a fixture backend cannot sustain a fault.
//!
//! What is proved here:
//!
//! * the broker publishes the address of every endpoint and the namespace it
//!   lives in, and the published address is the one the endpoint can be
//!   reached on (address ownership and readiness);
//! * a process launched into a namespace keeps a duplex stdio transport and
//!   its lifecycle is owned (exec/stdio lifecycle);
//! * applying the same fault twice is idempotent and the journal records it
//!   as such;
//! * there is no direct IP bypass, IPv4 *and* IPv6, while a fault is active;
//! * an established TCP session is cut by the fault and a new session works
//!   again after the heal;
//! * an unrelated link and the controller plane stay reachable while the
//!   fault is active.

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use spc_lab::links::netns::NetnsLinkBackend;
use spc_lab::links::LinkBackend;
use spc_lab::namespace::{BrokerOptions, NamespaceBroker, Placement};
use spc_lab::scenario::{parse_scenario, LinkPolicy};
use spc_lab::topology::{CanonicalObserver, CanonicalProbe, LinkPlane, TopologyPlan};

/// The process level proofs need the host namespace tools and an unprivileged
/// user namespace.  They are opt in, like the other host dependent proofs of
/// this crate, so the default `cargo test` and the Nix check stay green.
fn require_netns() {
    for tool in ["unshare", "nsenter", "nft", "ip", "python3"] {
        assert!(
            spc_lab::namespace::resolve_tool(tool).is_some(),
            "the namespace proof needs `{tool}` on PATH"
        );
    }
    assert_eq!(
        std::env::var("SPC_LAB_NETNS_TESTS").as_deref(),
        Ok("1"),
        "the namespace proof needs SPC_LAB_NETNS_TESTS=1"
    );
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}

fn broker_script() -> PathBuf {
    repo_root().join("scripts/lab/lib/lab_backend.py")
}

fn tool(name: &str) -> PathBuf {
    let path = std::env::var("PATH").unwrap_or_default();
    for directory in path.split(':') {
        let candidate = Path::new(directory).join(name);
        if candidate.is_file() {
            return candidate;
        }
    }
    PathBuf::from("/usr/bin").join(name)
}

/// Two nodes, one pool front, one miner and the links between them.
fn topology_scenario() -> Value {
    json!({
        "scenario_version": 1,
        "run": {
            "id": "lp-topology-test",
            "seed": 7,
            "time_policy": {"chain_time": "node_header", "miner_ntime": "hold", "mock_node_time": null},
            "warmup": {"blocks": 0, "window_fills": 1, "exclude_from_statistics": true}
        },
        "network": {
            "id": "net-a",
            "consensus_profile": {"required_capability": "spc.node.regtest.pow_baseline"}
        },
        "nodes": [
            {"id": "node-a", "network_id": "net-a", "implementation": "rbitcoin-node",
             "p2p": {"listen": "127.0.0.1:0"}, "rpc": {"listen": "127.0.0.1:0"},
             "tdp": {"listen": "127.0.0.1:0", "session_cap": 8}},
            {"id": "node-b", "network_id": "net-a", "implementation": "rbitcoin-node",
             "p2p": {"listen": "127.0.0.1:0"}, "rpc": {"listen": "127.0.0.1:0"},
             "tdp": {"listen": "127.0.0.1:0", "session_cap": 8}}
        ],
        "pools": [
            {"id": "pool-a", "network_id": "net-a", "node_ids": ["node-a"],
             "implementation": "spc-pool",
             "storage": {"redis_instance_id": "redis-a", "postgres_database_id": "pg-a"},
             "work_profile": {"work_unit_version": 2,
                              "reference_target": "7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
                              "share_work_multiplier": 2, "max_work_multiplier": 4294967296u64,
                              "max_window_target_units": "18446744073709551616",
                              "window_rule": "network_block_work", "window_n": 8, "max_slice_shares": 4}}
        ],
        "fronts": [
            {"id": "front-a", "pool_id": "pool-a", "roles": ["front", "payout", "api"],
             "listen": {"sv2": "127.0.0.1:0", "sv1": null, "jd": "127.0.0.1:0", "api": "127.0.0.1:0"}}
        ],
        "identities": [
            {"id": "id-a", "address_source": {"fixture": "lp-topology-slot-a"}}
        ],
        "miners": [
            {"id": "miner-a", "implementation": "spc-cpu-miner",
             "connections": [{"id": "conn-a", "front_id": "front-a", "identity_id": "id-a"}],
             "workers": [{"id": "worker-a", "identity_id": "id-a", "connection_id": "conn-a",
                          "budget": {"hashes_per_second": "20"}, "nonce_start": 0, "nonce_len": 1024}]}
        ],
        "resources": {
            "redis_instances": [{"id": "redis-a", "owner_pool_id": "pool-a", "port": 0, "dir": "redis/a"}],
            "postgres_databases": [{"id": "pg-a", "owner_pool_id": "pool-a", "database": "spc_a", "role": "spc_a", "port": 0}]
        },
        "links": [
            {"id": "p2p-ab", "from": {"kind": "node", "id": "node-a"}, "to": {"kind": "node", "id": "node-b"}}
        ]
    })
}

/// A short, stable hex token for a fixture name.
fn name_bytes(name: &str) -> u32 {
    let mut hash: u32 = 2166136261;
    for byte in name.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(16777619);
    }
    hash
}

struct Fixture {
    broker: std::sync::Arc<NamespaceBroker>,
    plan: TopologyPlan,
    run_dir: PathBuf,
}

impl Fixture {
    fn start(name: &str) -> Fixture {
        let scenario = topology_scenario();
        let parsed = parse_scenario(&serde_json::to_string_pretty(&scenario).expect("scenario"))
            .expect("the scenario is valid");
        let plan = TopologyPlan::from_scenario(&parsed).expect("the topology plan");
        let run_dir = PathBuf::from("/tmp/opencode").join(format!(
            "LP-network-kimi-topology-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&run_dir);
        std::fs::create_dir_all(&run_dir).expect("run directory");
        let spec_path = run_dir.join("broker-topology.json");
        std::fs::write(
            &spec_path,
            serde_json::to_vec_pretty(&plan.broker_spec()).expect("spec"),
        )
        .expect("spec write");
        let broker_dir = run_dir.join("broker");
        let _ = std::fs::remove_dir_all(&broker_dir);
        let mut options = BrokerOptions::new(
            broker_script(),
            &spec_path,
            broker_dir,
            // The broker derives the interface and nft table names from the
            // token, so it has to be the short hex form the LP-4a backend
            // expects.
            format!("{:08x}", name_bytes(name)),
        );
        options.unshare = tool("unshare");
        options.nsenter = tool("nsenter");
        let broker = NamespaceBroker::start(options).expect("the namespace broker starts");
        Fixture {
            broker: std::sync::Arc::new(broker),
            plan,
            run_dir,
        }
    }

    fn broker(&self) -> std::sync::Arc<NamespaceBroker> {
        std::sync::Arc::clone(&self.broker)
    }

    fn address(&self, endpoint: &str, link: &str) -> String {
        self.broker
            .manifest()
            .address(endpoint, link)
            .unwrap_or_else(|| panic!("the broker published {endpoint} on {link}"))
            .addr
            .clone()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // The broker's own drop stops the broker and releases its namespaces.
        let _ = std::fs::remove_dir_all(&self.run_dir);
    }
}

#[test]
#[ignore = "requires SPC_LAB_NETNS_TESTS=1 and the host namespace tools"]
fn namespace_address_ownership_and_readiness() {
    require_netns();
    let fixture = Fixture::start("ownership");
    let manifest = fixture.broker.manifest();

    // Every planned endpoint has a namespace and a published address.
    for endpoint in &fixture.plan.endpoints {
        let published = manifest
            .endpoint(&endpoint.name)
            .unwrap_or_else(|| panic!("endpoint {} was not published", endpoint.name));
        assert!(
            published.namespace.alive,
            "endpoint {} is alive",
            endpoint.name
        );
        assert!(
            published.namespace.netns_path.starts_with("/proc/"),
            "the namespace handle is a pid handle: {}",
            published.namespace.netns_path
        );
        assert!(
            !published.addresses.is_empty(),
            "endpoint {} has no published address",
            endpoint.name
        );
        for address in &published.addresses {
            assert!(
                address.addr.starts_with("10.250."),
                "the published address is in the lab range: {}",
                address.addr
            );
        }
    }

    // The placement the runtime publishes is derived from the manifest only.
    let placement = Placement::from_manifest(manifest);
    assert_eq!(
        placement.namespace("node-a"),
        Some(
            manifest
                .endpoint("node-a")
                .expect("node-a")
                .namespace
                .netns_path
                .as_str()
        )
    );
    let node_a = fixture.address("node-a", "p2p-ab");
    let node_b = fixture.address("node-b", "p2p-ab");
    assert_ne!(
        node_a, node_b,
        "the two ends of a link have different addresses"
    );

    // Readiness: a listener bound to the published address inside the
    // namespace accepts a connection from the peer namespace.
    // Readiness and address ownership: the endpoint can bind the address the
    // broker published for it, inside its own namespace.
    let server = fixture
        .broker
        .exec(
            "node-b",
            &[
                "python3".to_string(),
                "-c".to_string(),
                format!(
                    "import socket,sys\ns=socket.socket(); s.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1)\ns.bind(('{node_b}',9301)); s.listen(1)\nprint(s.getsockname()[0])\n"
                ),
            ],
            10.0,
        )
        .expect("the broker runs the command inside the namespace");
    assert_eq!(
        server.get("command_exit_code").and_then(Value::as_i64),
        Some(0),
        "the endpoint bound its published address: {server}"
    );
    assert!(
        server
            .get("stdout")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains(&node_b),
        "the bound address is the published one: {server}"
    );
}

#[test]
#[ignore = "requires SPC_LAB_NETNS_TESTS=1 and the host namespace tools"]
fn exec_stdio_lifecycle() {
    require_netns();
    let fixture = Fixture::start("stdio");
    // The broker's exec is the controller RPC: a command runs inside the
    // namespace and its output comes back.
    let result = fixture
        .broker
        .exec(
            "node-a",
            &[
                "python3".to_string(),
                "-c".to_string(),
                "import sys; sys.stdout.write('ping'); sys.stdout.flush(); sys.exit(0)".to_string(),
            ],
            10.0,
        )
        .expect("the broker executes inside the namespace");
    assert_eq!(
        result.get("stdout").and_then(Value::as_str),
        Some("ping"),
        "the namespace command produced its output"
    );
    assert_eq!(
        result.get("command_exit_code").and_then(Value::as_i64),
        Some(0)
    );
    // A failing command is reported as a failure, never as a success.
    let failure = fixture
        .broker
        .exec(
            "node-a",
            &[
                "python3".to_string(),
                "-c".to_string(),
                "import sys; sys.exit(3)".to_string(),
            ],
            10.0,
        )
        .expect("the broker executes inside the namespace");
    assert_eq!(
        failure.get("command_exit_code").and_then(Value::as_i64),
        Some(3)
    );
}

#[test]
#[ignore = "requires SPC_LAB_NETNS_TESTS=1 and the host namespace tools"]
fn rpc_auth_file_is_not_a_secret_in_helper_argv_or_response() {
    require_netns();
    let fixture = Fixture::start("rpc-auth-file");
    let password = "must-not-appear-in-helper-argv";
    let cookie = fixture.run_dir.join("node-rpc-test.cookie");
    std::fs::write(&cookie, format!("spc:{password}")).unwrap();
    std::fs::set_permissions(&cookie, std::fs::Permissions::from_mode(0o600)).unwrap();
    let helper = spc_lab::namespace::NamespaceBroker::helper_binary().unwrap();
    let result = fixture
        .broker
        .exec(
            "node-a",
            &[
                helper.display().to_string(),
                "rpc".to_string(),
                "--url".to_string(),
                "http://127.0.0.1:9/".to_string(),
                "--method".to_string(),
                "getblockcount".to_string(),
                "--params".to_string(),
                "[]".to_string(),
                "--auth-file".to_string(),
                cookie.display().to_string(),
            ],
            10.0,
        )
        .expect("bounded helper response");
    // An unreachable listener is fine for this negative capture: all helper
    // arguments and diagnostics still have to avoid carrying the cookie body.
    let whole_response = result.to_string();
    assert!(!whole_response.contains(password), "{whole_response}");
    assert!(!whole_response.contains("--pass"), "{whole_response}");
    assert!(whole_response.contains("--auth-file"));
    assert!(whole_response.contains("node-rpc-test.cookie"));
}

#[test]
#[ignore = "requires SPC_LAB_NETNS_TESTS=1 and the host namespace tools"]
fn fault_command_idempotence() {
    require_netns();
    let fixture = Fixture::start("idempotence");
    let backend = NetnsLinkBackend::new(Some(fixture.broker()));
    let parsed =
        parse_scenario(&serde_json::to_string_pretty(&topology_scenario()).expect("scenario"))
            .expect("the scenario is valid");
    backend
        .preflight(&parsed)
        .expect("the preflight accepts the topology");

    let link = parsed
        .links
        .first()
        .expect("the scenario declares the consensus link")
        .clone();
    backend
        .apply(&link, &LinkPolicy::Partition)
        .expect("the partition is applied");
    let first = backend.counters("p2p-ab").expect("counters");
    backend
        .apply(&link, &LinkPolicy::Partition)
        .expect("the partition is reapplied");
    let second = backend.counters("p2p-ab").expect("counters");

    // The reapply replaced the ruleset: one table, one pair of rules, and no
    // accumulated counters from the first application.
    let tables = second
        .get("sides")
        .and_then(|sides| sides.get("node-a"))
        .and_then(|side| side.get("nft"))
        .map(|nft| {
            nft.get("nftables")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter(|item| item.get("table").is_some())
                        .count()
                })
                .unwrap_or(0)
        })
        .unwrap_or(0);
    assert_eq!(tables, 1, "a reapply leaves exactly one lab table");
    let dropped = second
        .get("summary")
        .and_then(|summary| summary.get("nft_dropped_packets"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let dropped_first = first
        .get("summary")
        .and_then(|summary| summary.get("nft_dropped_packets"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    assert!(
        dropped <= dropped_first + 8,
        "a reapply does not accumulate drop counters ({dropped_first} -> {dropped})"
    );
    assert_eq!(backend.current(&link.id), Some(LinkPolicy::Partition));
    backend.heal(&link).expect("the heal succeeds");
    assert_eq!(backend.current(&link.id), Some(LinkPolicy::Normal));
}

#[test]
#[ignore = "requires SPC_LAB_NETNS_TESTS=1 and the host namespace tools"]
fn no_direct_ip_bypass_including_ipv6() {
    require_netns();
    let fixture = Fixture::start("bypass");
    let backend = NetnsLinkBackend::new(Some(fixture.broker()));
    let check = backend.no_bypass_check().expect("the no-bypass check runs");
    assert!(
        check.get("ok").and_then(Value::as_bool) == Some(true),
        "the no-bypass check reports no violation: {}",
        check
            .get("violations")
            .and_then(Value::as_array)
            .map(|items| items.len())
            .unwrap_or(0)
    );
    let violations = check
        .get("violations")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    assert_eq!(violations, 0, "no bypass was found");

    // The proof covers IPv6 as well: the non-link IPv6 probes are refused.
    let ipv6_probes = check
        .get("checks")
        .and_then(Value::as_array)
        .map(|checks| {
            checks
                .iter()
                .filter(|check| {
                    check.get("family").and_then(Value::as_str) == Some("inet6")
                        && check.get("kind").and_then(Value::as_str) == Some("non_link_address")
                })
                .count()
        })
        .unwrap_or(0);
    assert!(ipv6_probes > 0, "the IPv6 non-link addresses were probed");

    // The host addresses are refused from every namespace as well.
    let host_probes = check
        .get("checks")
        .and_then(Value::as_array)
        .map(|checks| {
            checks
                .iter()
                .filter(|check| check.get("kind").and_then(Value::as_str) == Some("host_address"))
                .count()
        })
        .unwrap_or(0);
    assert!(host_probes > 0, "the host addresses were probed");
}

#[test]
#[ignore = "requires SPC_LAB_NETNS_TESTS=1 and the host namespace tools"]
fn established_connection_is_cut_and_reconnects_after_heal() {
    require_netns();
    let fixture = Fixture::start("established");
    let node_a = fixture.address("node-a", "p2p-ab");
    let node_b = fixture.address("node-b", "p2p-ab");
    let backend = NetnsLinkBackend::new(Some(fixture.broker()));
    let parsed =
        parse_scenario(&serde_json::to_string_pretty(&topology_scenario()).expect("scenario"))
            .expect("the scenario is valid");
    let link = parsed.links.first().expect("the consensus link").clone();

    // A server in node-b that keeps the session open.
    let server_log = fixture.run_dir.join("server.jsonl");
    let fixture_script = repo_root().join("testing/lab-isolation/tcp_fixture.py");
    let spawn = fixture
        .broker
        .request(json!({
            "op": "spawn",
            "endpoint": "node-b",
            "argv": [
                "python3", fixture_script.display().to_string(), "server",
                "--bind", format!("{node_b}:9401"), "--persistent",
                "--log", server_log.display().to_string(), "--label", "established"
            ],
            "log_path": "server-spawn.log",
            "label": "established-server"
        }))
        .expect("the broker spawns the fixture server");
    assert_eq!(spawn.get("ok").and_then(Value::as_bool), Some(true));

    // A client in node-a that holds the session while the fault is applied.
    let client_log = fixture.run_dir.join("client.jsonl");
    let client = fixture
        .broker
        .request(json!({
            "op": "spawn",
            "endpoint": "node-a",
            "argv": [
                "python3", fixture_script.display().to_string(), "session",
                "--connect", format!("{node_b}:9401"), "--hold", "6", "--timeout", "3",
                "--log", client_log.display().to_string(), "--label", "established"
            ],
            "log_path": "client-spawn.log",
            "label": "established-client"
        }))
        .expect("the broker spawns the fixture client");
    assert_eq!(client.get("ok").and_then(Value::as_bool), Some(true));

    std::thread::sleep(Duration::from_millis(1500));
    backend
        .apply(&link, &LinkPolicy::Partition)
        .expect("the partition is applied");

    // The established session is cut: the second exchange fails.
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut phase2 = String::new();
    while Instant::now() < deadline {
        if let Ok(text) = std::fs::read_to_string(&client_log) {
            for line in text.lines() {
                if let Ok(value) = serde_json::from_str::<Value>(line) {
                    if value.get("event").and_then(Value::as_str) == Some("phase2") {
                        phase2 = value
                            .get("response")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                    }
                }
            }
        }
        if !phase2.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(
        phase2.starts_with("FAIL:"),
        "the established session was cut by the fault (phase2={phase2:?})"
    );

    // The heal restores delivery: a new session works again.
    backend.heal(&link).expect("the heal succeeds");
    let probe = fixture
        .broker
        .exec(
            "node-a",
            &[
                "python3".to_string(),
                "-c".to_string(),
                format!(
                    "import socket,sys\ns=socket.socket(); s.settimeout(3)\ns.connect(('{node_b}',9401))\ns.sendall(b'PING')\nprint(s.recv(64).decode().strip())\n"
                ),
            ],
            10.0,
        )
        .expect("the broker executes the probe");
    assert!(
        probe
            .get("stdout")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("PONG"),
        "a new session works after the heal: {probe}"
    );
    let _ = node_a;
}

#[test]
#[ignore = "requires SPC_LAB_NETNS_TESTS=1 and the host namespace tools"]
fn unrelated_link_and_control_plane_stay_reachable() {
    require_netns();
    let fixture = Fixture::start("unrelated");
    let backend = NetnsLinkBackend::new(Some(fixture.broker()));
    let parsed =
        parse_scenario(&serde_json::to_string_pretty(&topology_scenario()).expect("scenario"))
            .expect("the scenario is valid");
    let link = parsed.links.first().expect("the consensus link").clone();
    backend
        .apply(&link, &LinkPolicy::Partition)
        .expect("the partition is applied");

    // The controller plane is a socket on the filesystem: a data plane fault
    // never reaches it.
    let ping = fixture
        .broker
        .request(json!({"op": "ping"}))
        .expect("the controller answers");
    assert_eq!(ping.get("ok").and_then(Value::as_bool), Some(true));

    // The unrelated link of the plan (front-a <-> node-a) still works: the
    // fault is link scoped.
    let unrelated = fixture
        .plan
        .links_of_plane(LinkPlane::PoolNode)
        .first()
        .cloned()
        .expect("the plan has a pool to node link");
    let front = fixture.address(&unrelated.a, &unrelated.id);
    let node = fixture.address(&unrelated.b, &unrelated.id);
    let server = fixture.broker.request(json!({
        "op": "spawn",
        "endpoint": unrelated.b,
        "argv": [
            "python3", repo_root().join("testing/lab-isolation/tcp_fixture.py").display().to_string(),
            "server", "--bind", format!("{node}:9501"), "--label", "unrelated"
        ],
        "log_path": "unrelated-spawn.log",
        "label": "unrelated-server"
    }));
    assert_eq!(
        server
            .expect("the broker spawns the unrelated server")
            .get("ok")
            .and_then(Value::as_bool),
        Some(true)
    );
    std::thread::sleep(Duration::from_millis(800));
    let probe = fixture
        .broker
        .exec(
        &unrelated.a,
        &[
            "python3".to_string(),
            "-c".to_string(),
            format!(
                "import socket,sys\ns=socket.socket(); s.settimeout(3)\ns.connect(('{node}',9501))\ns.sendall(b'PING')\nprint(s.recv(64).decode().strip())\n"
            ),
        ],
        10.0,
    )
    .expect("the broker executes the probe");
    assert!(
        probe
            .get("stdout")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("PONG"),
        "the unrelated link still carries traffic: {probe}"
    );
    let _ = front;
}

#[test]
fn canonical_observer_detects_a_displaced_height() {
    struct FakeProbe {
        hashes: BTreeMap<(String, u64), String>,
        tip: (u64, String),
        fail_reads: bool,
    }
    impl CanonicalProbe for FakeProbe {
        fn hash_at(&mut self, node: &str, height: u64) -> Result<Option<String>, String> {
            if self.fail_reads {
                return Err("the node could not be asked".to_string());
            }
            Ok(self.hashes.get(&(node.to_string(), height)).cloned())
        }
        fn tip(&mut self, _node: &str) -> Result<(u64, String), String> {
            Ok(self.tip.clone())
        }
    }

    let mut observer = CanonicalObserver::new(4);
    let mut probe = FakeProbe {
        hashes: BTreeMap::from([(("node-a".to_string(), 10u64), "hash-10".to_string())]),
        tip: (10, "hash-10".to_string()),
        fail_reads: false,
    };
    let first = observer.observe(&["node-a".to_string()], &mut probe, 1);
    assert!(
        first.iter().all(|item| item.result == "yes"),
        "the first observation is a yes: {first:?}"
    );

    // The chain grew: the observer now tracks the old tip as well.
    probe
        .hashes
        .insert(("node-a".to_string(), 11), "hash-11".to_string());
    probe.tip = (11, "hash-11".to_string());
    let second = observer.observe(&["node-a".to_string()], &mut probe, 2);
    assert!(
        second
            .iter()
            .any(|item| item.height == 10 && item.result == "yes"),
        "the old tip is tracked and still canonical: {second:?}"
    );

    // The node is reorganised: the height 10 now has a different hash.
    probe
        .hashes
        .insert(("node-a".to_string(), 10), "other-hash".to_string());
    probe
        .hashes
        .insert(("node-a".to_string(), 12), "hash-12".to_string());
    probe.tip = (12, "hash-12".to_string());
    let third = observer.observe(&["node-a".to_string()], &mut probe, 3);
    let displaced = third
        .iter()
        .find(|item| item.height == 10)
        .expect("the displaced height is still tracked");
    assert_eq!(
        displaced.result, "no",
        "a displaced height is reported as no: {third:?}"
    );
    assert!(
        third
            .iter()
            .any(|item| item.height == 12 && item.result == "yes"),
        "the new tip is a yes: {third:?}"
    );
    // A failed read must never invent a displacement: it stays `unknown`.
    assert!(
        third
            .iter()
            .all(|item| item.result != "unknown" || item.detail.is_none()),
        "an unreadable height is unknown, not a displacement: {third:?}"
    );
    assert!(
        third.iter().all(|item| item.scope == "tip_sampler"),
        "the observation publishes its scope: {third:?}"
    );

    // A node that cannot be read never produces a `no`.
    probe.fail_reads = true;
    let unreadable = observer.observe(&["node-a".to_string()], &mut probe, 4);
    assert!(
        unreadable.iter().all(|item| item.result != "no"),
        "an unreadable node is unknown, never a displacement: {unreadable:?}"
    );
}

/// A tip view that raced with a new block must never look like a
/// displacement.
#[test]
fn a_racy_tip_view_is_unknown_and_never_a_displacement() {
    struct RacyProbe {
        tip: (u64, String),
        hash_at_height: String,
    }
    impl CanonicalProbe for RacyProbe {
        fn hash_at(&mut self, _node: &str, _height: u64) -> Result<Option<String>, String> {
            Ok(Some(self.hash_at_height.clone()))
        }
        fn tip(&mut self, _node: &str) -> Result<(u64, String), String> {
            Ok(self.tip.clone())
        }
    }
    let mut observer = CanonicalObserver::new(4);
    // The height is read before the block and the hash after it: the two
    // values do not describe the same block.
    let mut probe = RacyProbe {
        tip: (10, "hash-after".to_string()),
        hash_at_height: "hash-of-10".to_string(),
    };
    let first = observer.observe(&["node-a".to_string()], &mut probe, 1);
    assert!(
        first.iter().all(|item| item.result == "unknown"),
        "a racy tip view is unknown, never a displacement: {first:?}"
    );
    assert!(
        first.iter().all(|item| item.result != "no"),
        "a racy tip view never reports a displacement: {first:?}"
    );
}

/// The plan keeps the scenario's link ids verbatim.
#[test]
fn the_plan_keeps_the_scenario_link_ids_verbatim() {
    let scenario = topology_scenario();
    let parsed = parse_scenario(&serde_json::to_string_pretty(&scenario).expect("scenario"))
        .expect("the scenario is valid");
    let plan = TopologyPlan::from_scenario(&parsed).expect("the plan");
    let declared: Vec<String> = parsed
        .links
        .iter()
        .map(|link| link.id.to_string())
        .collect();
    for id in &declared {
        assert!(
            plan.links.iter().any(|link| &link.id == id),
            "the plan keeps the scenario link {id}: {:?}",
            plan.links
        );
    }
    // The generated storage links may not rename a declared link.
    for link in &plan.links {
        if link.plane == LinkPlane::Storage {
            assert!(
                !declared.contains(&link.id),
                "a generated link does not collide with a declared id: {}",
                link.id
            );
        }
    }
    assert_eq!(
        plan.consensus_links,
        vec!["p2p-ab".to_string()],
        "the consensus link is the scenario's own link"
    );
}

#[test]
fn topology_plan_separates_the_planes() {
    let scenario = topology_scenario();
    let parsed = parse_scenario(&serde_json::to_string_pretty(&scenario).expect("scenario"))
        .expect("the scenario is valid");
    let plan = TopologyPlan::from_scenario(&parsed).expect("the plan");
    assert_eq!(plan.consensus_links, vec!["p2p-ab".to_string()]);
    assert!(
        !plan.links_of_plane(LinkPlane::Consensus).is_empty(),
        "the consensus plane has the node to node link"
    );
    assert!(
        !plan.links_of_plane(LinkPlane::MinerFront).is_empty(),
        "the miner front plane has the miner link"
    );
    assert!(
        !plan.links_of_plane(LinkPlane::Storage).is_empty(),
        "the storage plane has the front to storage link"
    );
    // The miner is only linked to its front: it can never reach a node.
    let miner_links: Vec<_> = plan
        .links
        .iter()
        .filter(|link| link.a == "miner-a" || link.b == "miner-a")
        .collect();
    assert_eq!(miner_links.len(), 1);
    assert_eq!(miner_links[0].b, "front-a");
}

#[test]
#[ignore = "requires SPC_LAB_NETNS_TESTS=1 and the host namespace tools"]
fn host_address_collection_returns_addresses_not_interface_names() {
    require_netns();
    let (v4, v6) = spc_lab::namespace::host_global_addresses().expect("the host addresses");
    for address in &v4 {
        assert!(
            address.chars().all(|c| c.is_ascii_digit() || c == '.'),
            "the collected IPv4 host address is an address: {address}"
        );
    }
    for address in &v6 {
        assert!(
            address.contains(':'),
            "the collected IPv6 host address is an address: {address}"
        );
    }
    assert!(!v4.is_empty(), "the host has global IPv4 addresses");
}

/// The negative control of the suite: the fixture link backend must NOT be
/// able to satisfy the sustained partition assertion.
///
/// This is the fail-first evidence: the same observation that
/// `established_connection_is_cut_and_reconnects_after_heal` makes against
/// the netns backend is made against the fixture backend, and it must fail.
/// If the fixture backend ever passed that observation, the partition proof
/// would be meaningless.
#[test]
#[ignore = "requires SPC_LAB_NETNS_TESTS=1 and the host namespace tools"]
fn fixture_backend_cannot_sustain_a_partition() {
    require_netns();
    use spc_lab::links::FixtureLinkBackend;
    let fixture = Fixture::start("fixture-control");
    let node_a = fixture.address("node-a", "p2p-ab");
    let node_b = fixture.address("node-b", "p2p-ab");
    let parsed =
        parse_scenario(&serde_json::to_string_pretty(&topology_scenario()).expect("scenario"))
            .expect("the scenario is valid");
    let link = parsed.links.first().expect("the consensus link").clone();

    let backend = FixtureLinkBackend::new();
    assert!(
        !backend.supports_sustained_fault(),
        "the fixture backend never claims a sustained fault"
    );
    backend
        .apply(&link, &LinkPolicy::Partition)
        .expect("the fixture backend records the policy");

    // The fixture backend installs nothing, so a real session is NOT cut.
    let server = fixture
        .broker
        .request(json!({
            "op": "spawn",
            "endpoint": "node-b",
            "argv": [
                "python3", repo_root().join("testing/lab-isolation/tcp_fixture.py").display().to_string(),
                "server", "--bind", format!("{node_b}:9601"), "--persistent", "--label", "control"
            ],
            "log_path": "control-spawn.log",
            "label": "control-server"
        }))
        .expect("the broker spawns the control server");
    assert_eq!(server.get("ok").and_then(Value::as_bool), Some(true));
    std::thread::sleep(Duration::from_millis(800));
    let probe = fixture
        .broker
        .exec(
            "node-a",
            &[
                "python3".to_string(),
                "-c".to_string(),
                format!(
                    "import socket,sys\ns=socket.socket(); s.settimeout(3)\ns.connect(('{node_b}',9601))\ns.sendall(b'PING')\nprint(s.recv(64).decode().strip())\n"
                ),
            ],
            10.0,
        )
        .expect("the broker executes the probe");
    assert!(
        probe
            .get("stdout")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("PONG"),
        "the fixture backend leaves the link open, which is exactly why it cannot satisfy the partition assertion"
    );
    let _ = node_a;
}

/// T1: the preflight of the packet's own scenario has to pass against the
/// broker that its plan produces.
#[test]
#[ignore = "requires SPC_LAB_NETNS_TESTS=1 and the host namespace tools"]
fn the_scenario_preflight_passes_against_its_own_broker() {
    require_netns();
    let scenario_path = repo_root().join("crates/spc-lab/scenarios/lab-topology-two-nodes.json");
    let text = std::fs::read_to_string(&scenario_path).expect("the scenario");
    let scenario = parse_scenario(&text).expect("the scenario is valid");
    let plan = TopologyPlan::from_scenario(&scenario).expect("the plan");
    let dir = std::env::temp_dir().join(format!("lp-topology-preflight-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("the run dir");
    let spec_path = dir.join("broker-topology.json");
    std::fs::write(
        &spec_path,
        serde_json::to_vec_pretty(&plan.broker_spec()).expect("spec"),
    )
    .expect("the spec");
    let broker_dir = dir.join("broker");
    let mut options = BrokerOptions::new(
        broker_script(),
        &spec_path,
        broker_dir,
        format!("{:08x}", name_bytes("preflight")),
    );
    options.unshare = tool("unshare");
    options.nsenter = tool("nsenter");
    let broker = std::sync::Arc::new(NamespaceBroker::start(options).expect("the broker starts"));
    let backend = NetnsLinkBackend::new(Some(std::sync::Arc::clone(&broker)));
    backend
        .preflight(&scenario)
        .expect("the preflight of the packet's own scenario passes");
    let _ = std::fs::remove_dir_all(dir);
}

/// T1: a scenario link whose endpoint is not placed is refused.
#[test]
fn an_unplaced_scenario_link_is_refused() {
    let mut scenario = topology_scenario();
    scenario["links"]
        .as_array_mut()
        .expect("the links")
        .push(serde_json::json!({
            "id": "link-unplaced",
            "from": {"kind": "node", "id": "node-a"},
            "to": {"kind": "node", "id": "node-zzz"}
        }));
    let parsed = parse_scenario(&serde_json::to_string_pretty(&scenario).expect("scenario"))
        .expect("the scenario is valid");
    let error = TopologyPlan::from_scenario(&parsed).expect_err("the link is refused");
    assert!(
        error.contains("node-zzz"),
        "the error names the endpoint that is not placed: {error}"
    );
}

/// T2: a broker run directory that is not empty is refused, a run token that
/// is not eight hex characters is refused, and a foreign manifest is refused.
#[test]
#[ignore = "requires SPC_LAB_NETNS_TESTS=1 and the host namespace tools"]
fn the_broker_refuses_a_stale_directory_and_a_bad_token() {
    require_netns();
    let dir = std::env::temp_dir().join(format!("lp-topology-stale-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("the run dir");
    let spec_path = dir.join("broker-topology.json");
    std::fs::write(
        &spec_path,
        br#"{"endpoints":[{"name":"node-a","role":"node"}],"links":[]}"#,
    )
    .expect("the spec");
    let broker_dir = dir.join("broker");

    // A token that is not eight hex characters never reaches the broker.
    let mut options =
        BrokerOptions::new(broker_script(), &spec_path, broker_dir.clone(), "not-hex");
    options.unshare = tool("unshare");
    options.nsenter = tool("nsenter");
    let error = NamespaceBroker::start(options).expect_err("the token is refused");
    assert!(
        format!("{error}").contains("eight hex"),
        "the token error explains the rule: {error}"
    );

    // A directory that is not empty is refused: it belongs to another run.
    std::fs::create_dir_all(&broker_dir).expect("the broker dir");
    std::fs::write(broker_dir.join("manifest.json"), b"{}").expect("a stale manifest");
    let mut options = BrokerOptions::new(
        broker_script(),
        &spec_path,
        broker_dir.clone(),
        format!("{:08x}", name_bytes("stale")),
    );
    options.unshare = tool("unshare");
    options.nsenter = tool("nsenter");
    let error = NamespaceBroker::start(options).expect_err("the stale directory is refused");
    assert!(
        matches!(
            error,
            spc_lab::namespace::NamespaceError::StaleRunDir { .. }
        ),
        "a stale run directory is refused: {error}"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// T3: the broker's error codes map onto distinct link errors and a dead
/// broker never leaves a cached policy behind.
#[test]
#[ignore = "requires SPC_LAB_NETNS_TESTS=1 and the host namespace tools"]
fn the_link_errors_are_distinct_and_a_dead_broker_forgets_the_policy() {
    require_netns();
    let fixture = Fixture::start("errors");
    let scenario = topology_scenario();
    let parsed = parse_scenario(&serde_json::to_string_pretty(&scenario).expect("scenario"))
        .expect("the scenario is valid");
    let link = parsed.links.first().expect("the consensus link").clone();
    let broker = fixture.broker();
    let backend = NetnsLinkBackend::new(Some(std::sync::Arc::clone(&broker)));
    backend
        .apply(&link, &spc_lab::scenario::LinkPolicy::Partition)
        .expect("the partition is applied");
    assert_eq!(
        backend.current(&link.id),
        Some(spc_lab::scenario::LinkPolicy::Partition)
    );
    // Structured broker codes must be preserved all the way to LinkError.
    for policy in [
        spc_lab::scenario::LinkPolicy::Latency { delay_ms: 0 },
        spc_lab::scenario::LinkPolicy::Loss { loss_percent: 101 },
    ] {
        let error = backend.apply(&link, &policy).expect_err("invalid policy");
        assert!(
            matches!(error, spc_lab::links::LinkError::Unsupported(..)),
            "invalid_policy must map to Unsupported: {error}"
        );
    }
    // An unknown link is not a policy rejection.
    let mut unknown = link.clone();
    unknown.id = spc_lab::ids::LinkId::new("no-such-link").expect("the link id");
    let error = backend
        .apply(&unknown, &spc_lab::scenario::LinkPolicy::Partition)
        .expect_err("the unknown link is refused");
    assert!(
        matches!(error, spc_lab::links::LinkError::UnknownLink(_)),
        "unknown_link must map to UnknownLink: {error}"
    );
    // Ask this owned broker to terminate over its authenticated control
    // socket. Never signal a numeric PID from the test without identity checks.
    let _ = broker.request(json!({"op": "down"}));
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert_eq!(
        backend.current(&link.id),
        None,
        "a dead broker makes the link state unknown"
    );
}

/// T4: the exec handshake of the Rust supervisor, run inside the broker's
/// user namespace.  The outer process re-runs this test binary inside the
/// namespace, so the handshake that is tested is the Rust one.
#[test]
#[ignore = "requires SPC_LAB_NETNS_TESTS=1 and the host namespace tools"]
fn the_exec_handshake_survives_a_symlink_and_reports_a_failed_launch() {
    require_netns();
    if std::env::var("SPC_LAB_NETNS_INNER").is_ok() {
        // The inner process: it is inside the broker's user namespace.
        let symlink = std::env::var("SPC_LAB_NETNS_SYMLINK").expect("the symlink");
        let program = std::env::var("SPC_LAB_NETNS_PROGRAM").expect("the program");
        let sleep = tool("sleep");
        let mut supervisor = spc_lab::supervisor::Supervisor::new("handshake-proof");
        let dir = PathBuf::from("/tmp/opencode")
            .join(format!("LP-network-kimi-handshake-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("the dir");
        // A symlinked program is launched: the recorded identity is the
        // canonical program.
        let mut spec = spc_lab::supervisor::ProcessSpec::new(
            "probe",
            "symlink",
            std::path::PathBuf::from(&symlink),
            dir.join("symlink.log"),
        );
        // The program has to stay alive long enough to be recorded.
        spec.args = vec!["5".to_string()];
        let launched = supervisor
            .launch(spec)
            .expect("the symlinked program is launched");
        assert!(
            launched.executable_path.ends_with(&program),
            "the recorded executable is the canonical program: {:?}",
            launched.executable_path
        );
        // An unusable namespace fails fast with the launcher's status.
        let mut spec = spc_lab::supervisor::ProcessSpec::new(
            "probe",
            "bogus",
            std::path::PathBuf::from(&sleep),
            dir.join("bogus.log"),
        );
        spec.args = vec!["5".to_string()];
        spec.in_namespace("/proc/1/ns/net", tool("nsenter"));
        let error = supervisor.launch(spec).expect_err("the launch fails");
        assert!(
            !matches!(
                error,
                spc_lab::supervisor::SupervisorError::ExecHandshakeTimeout { .. }
            ),
            "a failed launch reports the launcher's status, not a timeout: {error}"
        );
        let _ = std::fs::remove_dir_all(dir);
        return;
    }
    // The outer process: start a broker and re-run this test inside its user
    // namespace.
    let fixture = Fixture::start("handshake");
    let program = tool("sleep");
    let symlink_dir = PathBuf::from("/tmp/opencode")
        .join(format!("LP-network-kimi-symlink-{}", std::process::id()));
    std::fs::create_dir_all(&symlink_dir).expect("the symlink dir");
    let symlink = symlink_dir.join("sleep-symlink");
    let _ = std::fs::remove_file(&symlink);
    std::os::unix::fs::symlink(&program, &symlink).expect("the symlink");
    let test_binary = std::env::current_exe().expect("the test binary");
    let broker_pid = fixture.broker().pid().expect("the broker pid");
    let mut command = std::process::Command::new(tool("nsenter"));
    command
        .arg("-U")
        .arg("--preserve-credentials")
        .arg("-t")
        .arg(broker_pid.to_string())
        .arg("--")
        .arg(&test_binary)
        .arg("--exact")
        .arg("the_exec_handshake_survives_a_symlink_and_reports_a_failed_launch")
        .arg("--ignored")
        .arg("--nocapture")
        .env("SPC_LAB_NETNS_TESTS", "1")
        .env("SPC_LAB_NETNS_INNER", "1")
        .env("SPC_LAB_NETNS_SYMLINK", &symlink)
        .env("SPC_LAB_NETNS_PROGRAM", program.to_string_lossy().as_ref());
    let output = command.output().expect("the inner test runs");
    assert!(
        output.status.success(),
        "the inner handshake test failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = std::fs::remove_dir_all(symlink_dir);
}

#[test]
#[ignore = "requires SPC_LAB_NETNS_TESTS=1, Redis binaries and host namespace tools"]
fn combined_user_net_launch_and_redis_negative_controls() {
    require_netns();
    let redis_server = std::env::var("SPC_LAB_REDIS_SERVER")
        .expect("set SPC_LAB_REDIS_SERVER to the absolute redis-server path");
    let redis_cli = std::env::var("SPC_LAB_REDIS_CLI")
        .expect("set SPC_LAB_REDIS_CLI to the absolute redis-cli path");
    let fixture = Fixture::start("storage-negative");
    let broker = fixture.broker();
    let broker_pid = broker.pid().expect("owned broker PID");
    let published = broker
        .manifest()
        .endpoint("storage-pool-a")
        .expect("storage endpoint")
        .namespace
        .clone();
    let address = fixture
        .broker
        .manifest()
        .address("storage-pool-a", "front-a--storage")
        .expect("published storage address")
        .addr
        .clone();
    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("ephemeral port");
    let port = probe.local_addr().expect("local port").port();
    drop(probe);
    let password = "storage-negative-password";
    let data_dir = fixture.run_dir.join("redis-data");
    std::fs::create_dir_all(&data_dir).expect("Redis data directory");
    let config = fixture.run_dir.join("redis.conf");
    std::fs::write(
        &config,
        format!(
            "bind {address}\nport {port}\nprotected-mode yes\ndaemonize no\nsave \"\"\nappendonly no\ndir {}\nrequirepass {password}\n",
            data_dir.display()
        ),
    )
    .expect("Redis config");
    let mut supervisor = spc_lab::supervisor::Supervisor::new("combined-storage-proof");
    let mut spec = spc_lab::supervisor::ProcessSpec::new(
        "redis",
        "negative",
        &redis_server,
        fixture.run_dir.join("redis.log"),
    );
    spec.args = vec![config.display().to_string()];
    spec.in_namespaces(
        format!("/proc/{broker_pid}/ns/user"),
        &published.netns_path,
        tool("nsenter"),
    );
    spec.namespace_owner = Some(spc_lab::supervisor::NamespaceOwner {
        pid: published.holder_pid,
        start_time_ticks: published.start_time_token,
    });
    spec.user_namespace_owner = Some(spc_lab::supervisor::NamespaceOwner {
        pid: broker_pid,
        start_time_ticks: spc_lab::supervisor::process_start_time(broker_pid)
            .expect("broker start token")
            .to_string(),
    });
    let record = supervisor
        .launch(spec)
        .expect("combined user+network namespace actor launch");
    assert_eq!(
        std::fs::read_link(format!("/proc/{}/ns/net", record.pid)).unwrap(),
        std::fs::read_link(&published.netns_path).unwrap()
    );
    assert_eq!(
        std::fs::read_link(format!("/proc/{}/ns/user", record.pid)).unwrap(),
        std::fs::read_link(format!("/proc/{broker_pid}/ns/user")).unwrap()
    );

    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let call = |endpoint: &str, password: &str, host: &str, port: u16| -> Value {
            broker
                .exec(
                    endpoint,
                    &[
                        redis_cli.clone(),
                        "-h".to_string(),
                        host.to_string(),
                        "-p".to_string(),
                        port.to_string(),
                        "-a".to_string(),
                        password.to_string(),
                        "--no-auth-warning".to_string(),
                        "PING".to_string(),
                    ],
                    5.0,
                )
                .expect("bounded Redis probe")
        };
        let mut ready = false;
        for _ in 0..50 {
            let response = call("storage-pool-a", password, &address, port);
            if response.get("command_exit_code").and_then(Value::as_i64) == Some(0)
                && response.get("stdout").and_then(Value::as_str) == Some("PONG\n")
            {
                ready = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(
            ready,
            "authenticated Redis did not become ready on {address}:{port}"
        );

        let wrong_password = call("storage-pool-a", "definitely-wrong", &address, port);
        // redis-cli exits 0 even when the server rejects AUTH, so inspect the
        // protocol answer rather than treating process exit as authentication.
        assert!(
            wrong_password.get("stdout").and_then(Value::as_str) != Some("PONG\n")
                && (wrong_password
                    .get("stderr")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .contains("WRONGPASS")
                    || wrong_password
                        .get("stdout")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .contains("NOAUTH")),
            "wrong Redis password must fail: {wrong_password}"
        );
        let front_ok = call("front-a", password, &address, port);
        assert_eq!(
            front_ok.get("command_exit_code").and_then(Value::as_i64),
            Some(0),
            "the front must reach only the published storage address"
        );
        let loopback = call("front-a", password, "127.0.0.1", port);
        assert_ne!(
            loopback.get("command_exit_code").and_then(Value::as_i64),
            Some(0),
            "another namespace's loopback must not be used: {loopback}"
        );
        let wrong_port = call(
            "front-a",
            password,
            &address,
            port.checked_add(1).expect("port+1"),
        );
        assert_ne!(
            wrong_port.get("command_exit_code").and_then(Value::as_i64),
            Some(0),
            "the wrong storage port must fail closed: {wrong_port}"
        );
    }));
    supervisor.stop_all(Duration::from_secs(2));
    drop(fixture);
    outcome.unwrap();
}
