//! Unit tests for the receiver-handshake/owned-reverse-TCP readiness
//! proof. The positive fixtures replicate the exact join table of the
//! blocked AP5 diagnostic run (node A listener 127.0.0.1:44919, node B
//! listener 127.0.0.1:35167, B's dial socket 127.0.0.1:33620 -> 44919,
//! A's dial socket 127.0.0.1:47756 -> 35167, both inbound rows on the
//! acceptors) so the regression pins the real observed shape, not a
//! simplified one.

use super::*;
use serde_json::json;

// -- The exact AP5 planned endpoints and observed ephemeral ports -------

const A_LISTEN: &str = "127.0.0.1:44919";
const B_LISTEN: &str = "127.0.0.1:35167";
/// B's ephemeral dial endpoint as A's inbound row reported it.
const B_EPHEMERAL: &str = "127.0.0.1:33620";
/// A's ephemeral dial endpoint as B's inbound row reported it.
const A_EPHEMERAL: &str = "127.0.0.1:47756";

const PID_A: u32 = 4101;
const PID_B: u32 = 4102;
const START_A: u64 = 91_234;
const START_B: u64 = 91_235;
const BOOT_ID: &str = "b7f0c2d4-6a55-4e3a-9d1b-8c2f0a1e5d67";
/// The observed host-controller netns identity `(st_dev, st_ino)`; in
/// the AP5 profile both node processes share it (host network).
const HOST_NETNS: (u64, u64) = (0, 4_026_531_841);
/// The observed executable of both node processes (the pinned node).
const NODE_EXE: &str = "/nix/store/03pw2rf9wrxxnd9rl0l8ln0mnw9gcpx2-rbitcoin-node-ark-lab-0.8.99-profile-facts/bin/rbitcoin-node";

// Kernel socket inodes. The two endpoints of one connection have
// different inodes; the reversed four-tuples are the match.
const INODE_A_LISTEN: u64 = 82_006;
const INODE_A_ACCEPT: u64 = 82_007;
const INODE_A_DIAL: u64 = 82_008;
const INODE_B_LISTEN: u64 = 82_010;
const INODE_B_ACCEPT: u64 = 82_011;
const INODE_B_DIAL: u64 = 82_012;

fn lo(port: u16) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], port))
}

fn addr(text: &str) -> SocketAddr {
    text.parse().expect("test endpoint parses")
}

fn tcp(local: SocketAddr, remote: SocketAddr, state: &str, inode: u64) -> TcpRow {
    TcpRow {
        local,
        remote,
        state: state.to_string(),
        inode,
    }
}

// -- Fixture builders ---------------------------------------------------

fn ap5_planned() -> PlannedPeers {
    PlannedPeers::new(
        BTreeMap::from([
            ("node-a".to_string(), A_LISTEN.to_string()),
            ("node-b".to_string(), B_LISTEN.to_string()),
        ]),
        BTreeMap::from([
            ("node-a".to_string(), B_LISTEN.to_string()),
            ("node-b".to_string(), A_LISTEN.to_string()),
        ]),
    )
    .expect("AP5 planned graph is valid")
}

fn ap5_identities() -> BTreeMap<String, NodeProcessIdentity> {
    BTreeMap::from([
        (
            "node-a".to_string(),
            NodeProcessIdentity {
                node: "node-a".to_string(),
                pid: PID_A,
                start_time_ticks: START_A,
                network_namespace: None,
            },
        ),
        (
            "node-b".to_string(),
            NodeProcessIdentity {
                node: "node-b".to_string(),
                pid: PID_B,
                start_time_ticks: START_B,
                network_namespace: None,
            },
        ),
    ])
}

/// A completed inbound handshake row exactly as the pinned rbitcoin
/// acceptor reports it (services as the `{:016x}` string, received
/// VERSION/VERACK accounting markers, getheaders corroboration).
fn completed_row(id: i64, peer: &str, listener: &str) -> PeerHandshakeRow {
    PeerHandshakeRow {
        id,
        addr: peer.to_string(),
        addrbind: listener.to_string(),
        inbound: true,
        version: EXPECTED_PROTOCOL_VERSION,
        subver: EXPECTED_SUBVER.to_string(),
        services: format!("{EXPECTED_SERVICE_BASE_MASK:016x}"),
        startingheight: 0,
        bytesrecv_version: Some(110),
        bytesrecv_verack: Some(24),
        bytesrecv_getheaders: Some(29),
        transport_protocol_type: Some("v2".to_string()),
        connection_type: Some("inbound".to_string()),
    }
}

/// The namespace-wide TCP view both planned pids see (same host network
/// namespace): both listeners and both directions of both connections.
/// Ownership is decided by each pid's fd table, not by table presence.
fn ap5_tcp_rows() -> Vec<TcpRow> {
    vec![
        tcp(
            lo(44919),
            addr("0.0.0.0:0"),
            TCP_STATE_LISTEN,
            INODE_A_LISTEN,
        ),
        tcp(
            lo(35167),
            addr("0.0.0.0:0"),
            TCP_STATE_LISTEN,
            INODE_B_LISTEN,
        ),
        // B -> A connection.
        tcp(lo(33620), lo(44919), TCP_STATE_ESTABLISHED, INODE_B_DIAL),
        tcp(lo(44919), lo(33620), TCP_STATE_ESTABLISHED, INODE_A_ACCEPT),
        // A -> B connection.
        tcp(lo(47756), lo(35167), TCP_STATE_ESTABLISHED, INODE_A_DIAL),
        tcp(lo(35167), lo(47756), TCP_STATE_ESTABLISHED, INODE_B_ACCEPT),
    ]
}

/// A's fd socket inodes. The dial socket is listed twice: duplicate fd
/// aliases of one inode are legitimate and deduplicate in the set.
fn fds_a() -> BTreeSet<u64> {
    [INODE_A_LISTEN, INODE_A_ACCEPT, INODE_A_DIAL, INODE_A_DIAL]
        .into_iter()
        .collect()
}

/// B's fd socket inodes: B owns its listener, its accepted socket (from
/// A) and its dial socket (to A) - and NOT A's sockets, even though the
/// namespace-wide table shows them.
fn fds_b() -> BTreeSet<u64> {
    [INODE_B_LISTEN, INODE_B_ACCEPT, INODE_B_DIAL]
        .into_iter()
        .collect()
}

fn process_sample(pid: u32, start: u64, fds: BTreeSet<u64>, rows: &[TcpRow]) -> ProcessSample {
    ProcessSample {
        pid,
        start_ticks: Some(start),
        socket_inodes: Some(fds),
        tcp_rows: rows.to_vec(),
        netns_identity: Some(HOST_NETNS),
        exe: Some(NODE_EXE.to_string()),
    }
}

fn bracket_at(mono_ms: u64, rows: &[TcpRow]) -> BracketSample {
    BracketSample {
        boot_id: Some(BOOT_ID.to_string()),
        host_netns_identity: Some(HOST_NETNS),
        processes: BTreeMap::from([
            (
                "node-a".to_string(),
                process_sample(PID_A, START_A, fds_a(), rows),
            ),
            (
                "node-b".to_string(),
                process_sample(PID_B, START_B, fds_b(), rows),
            ),
        ]),
        sampled_at_mono_ms: mono_ms,
    }
}

/// The default dual peer sample for one node: the same rows in sample A
/// (stamped 5_100 ms, right after the 5_000 ms before bracket) and
/// sample B (stamped 5_300 ms, right before the 5_400 ms after
/// bracket).
fn dual_of(rows: Vec<PeerHandshakeRow>) -> PeerDualSample {
    PeerDualSample {
        a: PeerSample {
            rows: rows.clone(),
            sampled_at_mono_ms: 5_100,
        },
        b: PeerSample {
            rows,
            sampled_at_mono_ms: 5_300,
        },
    }
}

fn ap5_duals() -> BTreeMap<String, PeerDualSample> {
    BTreeMap::from([
        (
            "node-a".to_string(),
            dual_of(vec![completed_row(3, B_EPHEMERAL, A_LISTEN)]),
        ),
        (
            "node-b".to_string(),
            dual_of(vec![completed_row(7, A_EPHEMERAL, B_LISTEN)]),
        ),
    ])
}

fn dial_of(planned: &PlannedPeers, dialer: &str, acceptor: &str) -> PlannedDial {
    planned
        .dials()
        .into_iter()
        .find(|dial| dial.dialer == dialer && dial.acceptor == acceptor)
        .expect("planned dial exists")
}

fn assert_retry(failure: &Option<LinkProofFailure>, needle: &str) {
    match failure {
        Some(LinkProofFailure::Retry { detail }) => {
            assert!(detail.contains(needle), "retry detail: {detail}");
        }
        other => panic!("expected retry, got {other:?}"),
    }
}

fn assert_refuse(failure: &Option<LinkProofFailure>, needle: &str) {
    match failure {
        Some(LinkProofFailure::Refuse { detail }) => {
            assert!(detail.contains(needle), "refuse detail: {detail}");
        }
        other => panic!("expected refuse, got {other:?}"),
    }
}

// -- The positive: the exact AP5 join table proves both directed dials --

#[test]
fn ap5_join_table_proves_both_directed_dials() {
    let planned = ap5_planned();
    let rows = ap5_tcp_rows();
    let before = bracket_at(5_000, &rows);
    let after = bracket_at(5_400, &rows);
    let pass = observe_peer_graph(&planned, &ap5_duals(), &ap5_identities(), &before, &after);
    assert!(
        pass.failure.is_none(),
        "unexpected failure: {:?}",
        pass.failure
    );
    let expected_dials: BTreeSet<(String, String)> = BTreeSet::from([
        ("node-a".to_string(), "node-b".to_string()),
        ("node-b".to_string(), "node-a".to_string()),
    ]);
    assert_eq!(pass.covered_dials, expected_dials);
    // Undirected edges are derived only after a typed directed proof.
    let expected_edges: BTreeSet<(String, String)> =
        BTreeSet::from([("node-a".to_string(), "node-b".to_string())]);
    assert_eq!(pass.covered_edges, expected_edges);
    assert_eq!(pass.proofs.len(), 2);
}

#[test]
fn ap5_proof_records_raw_handshake_and_both_endpoint_joins() {
    let planned = ap5_planned();
    let identities = ap5_identities();
    let rows = ap5_tcp_rows();
    let before = bracket_at(5_000, &rows);
    let after = bracket_at(5_400, &rows);
    let proof = decide_directed_dial(
        &dial_of(&planned, "node-b", "node-a"),
        &identities,
        &dual_of(vec![completed_row(3, B_EPHEMERAL, A_LISTEN)]),
        &completed_row(3, B_EPHEMERAL, A_LISTEN),
        &before,
        &after,
        &planned.listener_ports(),
    )
    .expect("the AP5 B -> A join table proves the dial");
    // Versioned evidence form.
    assert_eq!(proof.contract_version, READINESS_CONTRACT_VERSION);
    assert_eq!(proof.evidence_form, EVIDENCE_FORM);
    assert_eq!(proof.command_id, None);
    assert_eq!(proof.dialer, "node-b");
    assert_eq!(proof.acceptor, "node-a");
    assert_eq!(proof.planned_target, A_LISTEN);
    // Raw handshake fields, exactly as reported.
    assert_eq!(proof.handshake.id, 3);
    assert_eq!(proof.handshake.addr, B_EPHEMERAL);
    assert_eq!(proof.handshake.addrbind, A_LISTEN);
    assert!(proof.handshake.inbound);
    assert_eq!(proof.handshake.version, EXPECTED_PROTOCOL_VERSION);
    assert_eq!(proof.handshake.subver, EXPECTED_SUBVER);
    assert_eq!(proof.handshake.services, "0000000000000809");
    assert_eq!(proof.handshake.bytesrecv_version, Some(110));
    assert_eq!(proof.handshake.bytesrecv_verack, Some(24));
    assert_eq!(proof.handshake.bytesrecv_getheaders, Some(29));
    // getheaders is recorded corroboration and explicitly NOT required.
    assert!(!proof.getheaders_corroboration_required);
    // Process identities: PID and start ticks only.
    assert_eq!(proof.dialer_pid, PID_B);
    assert_eq!(proof.dialer_start_time_ticks, START_B);
    assert_eq!(proof.acceptor_pid, PID_A);
    assert_eq!(proof.acceptor_start_time_ticks, START_A);
    assert_eq!(proof.boot_id.as_deref(), Some(BOOT_ID));
    // Both endpoint joins: exact reversed four-tuples, different inodes.
    assert_eq!(proof.dialer_socket.local, B_EPHEMERAL);
    assert_eq!(proof.dialer_socket.remote, A_LISTEN);
    assert_eq!(proof.dialer_socket.state, TCP_STATE_ESTABLISHED);
    assert_eq!(proof.dialer_socket.inode, INODE_B_DIAL);
    assert!(proof.dialer_socket.fd_owned);
    assert_eq!(proof.acceptor_socket.local, A_LISTEN);
    assert_eq!(proof.acceptor_socket.remote, B_EPHEMERAL);
    assert_eq!(proof.acceptor_socket.state, TCP_STATE_ESTABLISHED);
    assert_eq!(proof.acceptor_socket.inode, INODE_A_ACCEPT);
    assert!(proof.acceptor_socket.fd_owned);
    assert!(proof.endpoint_inodes_differ);
    // Bracket timing.
    assert_eq!(proof.bracket_before_mono_ms, 5_000);
    assert_eq!(proof.bracket_after_mono_ms, 5_400);
    assert_eq!(proof.bracket_interval_ms, 400);
    assert!(proof.snapshot_confirmed);
    assert!(proof.start_verified);
}

#[test]
fn ap5_proof_schema_round_trips_with_stable_field_names() {
    let planned = ap5_planned();
    let rows = ap5_tcp_rows();
    let before = bracket_at(5_000, &rows);
    let after = bracket_at(5_400, &rows);
    let proof = decide_directed_dial(
        &dial_of(&planned, "node-b", "node-a"),
        &ap5_identities(),
        &dual_of(vec![completed_row(3, B_EPHEMERAL, A_LISTEN)]),
        &completed_row(3, B_EPHEMERAL, A_LISTEN),
        &before,
        &after,
        &planned.listener_ports(),
    )
    .expect("proof for the schema fixture");
    let text = serde_json::to_string(&proof).expect("proof serializes");
    let back: DirectedDialProof = serde_json::from_str(&text).expect("proof deserializes");
    assert_eq!(proof, back);
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    // Field names are the frozen public schema the consumer binds to.
    assert_eq!(value["contract_version"], json!(READINESS_CONTRACT_VERSION));
    assert_eq!(value["evidence_form"], json!(EVIDENCE_FORM));
    assert_eq!(value["getheaders_corroboration_required"], json!(false));
    assert_eq!(value["endpoint_inodes_differ"], json!(true));
    assert_eq!(value["handshake"]["addr"], json!(B_EPHEMERAL));
    assert_eq!(value["handshake"]["addrbind"], json!(A_LISTEN));
    assert_eq!(value["handshake"]["bytesrecv_verack"], json!(24));
    assert_eq!(value["dialer_socket"]["inode"], json!(INODE_B_DIAL));
    assert_eq!(value["acceptor_socket"]["inode"], json!(INODE_A_ACCEPT));
    assert_eq!(value["dialer_start_time_ticks"], json!(START_B));
    assert_eq!(value["acceptor_start_time_ticks"], json!(START_A));
    // Version 3 fields: the dual peer sample, the observed namespace
    // identities, the observed executables and the raw brackets.
    assert_eq!(value["handshake_sample_a"]["id"], json!(3));
    assert_eq!(value["peer_sample_a_at_mono_ms"], json!(5_100));
    assert_eq!(value["peer_sample_b_at_mono_ms"], json!(5_300));
    assert_eq!(
        value["host_netns_identity"],
        json!([HOST_NETNS.0, HOST_NETNS.1])
    );
    assert_eq!(
        value["dialer_netns_identity"],
        json!([HOST_NETNS.0, HOST_NETNS.1])
    );
    assert_eq!(value["dialer_exe"], json!(NODE_EXE));
    assert_eq!(value["acceptor_exe"], json!(NODE_EXE));
    assert_eq!(
        value["raw_bracket_before"]["sampled_at_mono_ms"],
        json!(5_000)
    );
    assert_eq!(
        value["raw_bracket_after"]["sampled_at_mono_ms"],
        json!(5_400)
    );
    // The raw bracket records the dialer's fd membership and its own
    // TCP rows (state + four-tuples), not just summary booleans.
    let dialer_raw = &value["raw_bracket_before"]["processes"]["node-b"];
    assert!(dialer_raw["fd_socket_inodes"]
        .as_array()
        .expect("fd socket inodes")
        .contains(&json!(INODE_B_DIAL)));
    assert_eq!(
        dialer_raw["owned_tcp_rows"]
            .as_array()
            .expect("owned tcp rows")
            .len(),
        3
    );
    assert_eq!(dialer_raw["netns_identity"], json!([HOST_NETNS.0, HOST_NETNS.1]));
    assert_eq!(dialer_raw["exe"], json!(NODE_EXE));
}

/// The canonical public fixture record: the exact serialized bytes the
/// producer emits for the AP5 join-table B -> A proof, frozen in-tree so
/// the AP5/QA consumer parser can bind to known bytes. Any schema or
/// serialization change that alters these bytes is a contract change.
/// Regenerate with `emit_canonical_fixture_bytes` after a deliberate
/// contract change, then freeze the new bytes here.
#[test]
fn canonical_fixture_bytes_are_frozen() {
    let planned = ap5_planned();
    let rows = ap5_tcp_rows();
    let before = bracket_at(5_000, &rows);
    let after = bracket_at(5_400, &rows);
    let proof = decide_directed_dial(
        &dial_of(&planned, "node-b", "node-a"),
        &ap5_identities(),
        &dual_of(vec![completed_row(3, B_EPHEMERAL, A_LISTEN)]),
        &completed_row(3, B_EPHEMERAL, A_LISTEN),
        &before,
        &after,
        &planned.listener_ports(),
    )
    .expect("the AP5 B -> A join table proves the dial");
    let serialized = serde_json::to_string(&proof).expect("proof serializes");
    let frozen = include_str!("fixture_directed_dial_proof_v3.json").trim();
    assert_eq!(
        serialized, frozen,
        "serialized proof bytes changed: the public fixture is frozen"
    );
    let reparsed: DirectedDialProof = serde_json::from_str(frozen).expect("frozen fixture parses");
    assert_eq!(reparsed, proof);
}

/// Emits the canonical fixture bytes to stdout (not a check; run with
/// `--ignored --exact --nocapture` and freeze the output into
/// `fixture_directed_dial_proof_v3.json` after a deliberate contract
/// change).
#[test]
#[ignore]
fn emit_canonical_fixture_bytes() {
    let planned = ap5_planned();
    let rows = ap5_tcp_rows();
    let before = bracket_at(5_000, &rows);
    let after = bracket_at(5_400, &rows);
    let proof = decide_directed_dial(
        &dial_of(&planned, "node-b", "node-a"),
        &ap5_identities(),
        &dual_of(vec![completed_row(3, B_EPHEMERAL, A_LISTEN)]),
        &completed_row(3, B_EPHEMERAL, A_LISTEN),
        &before,
        &after,
        &planned.listener_ports(),
    )
    .expect("the AP5 B -> A join table proves the dial");
    println!("{}", serde_json::to_string(&proof).expect("proof serializes"));
}

/// The version 2 fixture (the superseded failed-to-certify contract) no
/// longer parses as a version 3 record: the new mandatory raw-evidence
/// fields are absent, so a v2 proof can never be silently accepted.
#[test]
fn v2_fixture_bytes_no_longer_parse_as_v3() {
    let v2 = include_str!("fixture_directed_dial_proof.json").trim();
    assert!(serde_json::from_str::<DirectedDialProof>(v2).is_err());
}

// -- Handshake-row classification negatives ------------------------------

#[test]
fn connecting_row_with_version_zero_retries() {
    let mut row = completed_row(3, B_EPHEMERAL, A_LISTEN);
    row.version = 0;
    row.subver = String::new();
    row.services = String::new();
    row.bytesrecv_version = None;
    row.bytesrecv_verack = None;
    row.startingheight = -1;
    assert!(row.is_connecting());
    let planned = ap5_planned();
    let rows_table = ap5_tcp_rows();
    let before = bracket_at(5_000, &rows_table);
    let after = bracket_at(5_400, &rows_table);
    let rows = BTreeMap::from([("node-a".to_string(), dual_of(vec![row]))]);
    let pass = observe_peer_graph(&planned, &rows, &ap5_identities(), &before, &after);
    // The other dial is unproven too; the connecting row itself is the
    // recorded reason on node-a's side.
    assert!(matches!(pass.failure, Some(LinkProofFailure::Retry { .. })));
}

#[test]
fn missing_verack_marker_is_incomplete_and_retries() {
    let mut row = completed_row(3, B_EPHEMERAL, A_LISTEN);
    row.bytesrecv_verack = None;
    assert!(row.is_connecting());
    assert!(matches!(
        row.completed_inbound_markers(),
        Err(HandshakeMarkerFailure::Incomplete(_))
    ));
}

#[test]
fn zero_verack_accounting_contradicts_the_pinned_node_and_refuses() {
    let mut row = completed_row(3, B_EPHEMERAL, A_LISTEN);
    row.bytesrecv_verack = Some(0);
    assert!(matches!(
        row.completed_inbound_markers(),
        Err(HandshakeMarkerFailure::Foreign(_))
    ));
}

#[test]
fn wrong_protocol_version_refuses() {
    let mut row = completed_row(3, B_EPHEMERAL, A_LISTEN);
    row.version = 70015;
    assert!(matches!(
        row.completed_inbound_markers(),
        Err(HandshakeMarkerFailure::Foreign(_))
    ));
}

#[test]
fn wrong_subver_refuses() {
    let mut row = completed_row(3, B_EPHEMERAL, A_LISTEN);
    row.subver = "/Satoshi:28.0/".to_string();
    assert!(matches!(
        row.completed_inbound_markers(),
        Err(HandshakeMarkerFailure::Foreign(_))
    ));
}

#[test]
fn services_without_the_pinned_bits_refuse() {
    let mut row = completed_row(3, B_EPHEMERAL, A_LISTEN);
    // NETWORK|WITNESS only: the P2P_V2 bit is missing.
    row.services = format!("{:016x}", 0x009);
    assert!(matches!(
        row.completed_inbound_markers(),
        Err(HandshakeMarkerFailure::Foreign(_))
    ));
}

#[test]
fn non_hex_services_field_refuses() {
    let mut row = completed_row(3, B_EPHEMERAL, A_LISTEN);
    row.services = "network".to_string();
    assert!(matches!(
        row.completed_inbound_markers(),
        Err(HandshakeMarkerFailure::Foreign(_))
    ));
}

#[test]
fn completed_row_with_absent_getheaders_still_proves_the_dial() {
    // getheaders is corroboration only: the AP5 profile may gate before
    // the peer ever sends one.
    let mut row = completed_row(3, B_EPHEMERAL, A_LISTEN);
    row.bytesrecv_getheaders = None;
    let planned = ap5_planned();
    let rows_table = ap5_tcp_rows();
    let before = bracket_at(5_000, &rows_table);
    let after = bracket_at(5_400, &rows_table);
    let proof = decide_directed_dial(
        &dial_of(&planned, "node-b", "node-a"),
        &ap5_identities(),
        &dual_of(vec![row.clone()]),
        &row,
        &before,
        &after,
        &planned.listener_ports(),
    )
    .expect("a completed handshake without getheaders still proves");
    assert!(!proof.getheaders_corroboration_required);
    assert_eq!(proof.handshake.bytesrecv_getheaders, None);
}

// -- Direction, plan and endpoint negatives -----------------------------

#[test]
fn outbound_row_cannot_carry_the_receiver_handshake_proof() {
    // Direction is preserved exactly as reported: relabelling the
    // acceptor's row as outbound must refuse, not pass.
    let mut row = completed_row(3, B_EPHEMERAL, A_LISTEN);
    row.inbound = false;
    let planned = ap5_planned();
    let rows_table = ap5_tcp_rows();
    let before = bracket_at(5_000, &rows_table);
    let after = bracket_at(5_400, &rows_table);
    let result = decide_directed_dial(
        &dial_of(&planned, "node-b", "node-a"),
        &ap5_identities(),
        &dual_of(vec![row.clone()]),
        &row,
        &before,
        &after,
        &planned.listener_ports(),
    );
    let failure = result.expect_err("outbound row must refuse");
    match &failure {
        LinkProofFailure::Refuse { detail } => {
            assert!(detail.contains("not inbound"), "refuse detail: {detail}");
        }
        other => panic!("expected refuse, got {other:?}"),
    }
}

#[test]
fn addrbind_off_the_planned_target_refuses() {
    let row = completed_row(3, B_EPHEMERAL, "127.0.0.1:45000");
    let planned = ap5_planned();
    let rows_table = ap5_tcp_rows();
    let before = bracket_at(5_000, &rows_table);
    let after = bracket_at(5_400, &rows_table);
    let result = decide_directed_dial(
        &dial_of(&planned, "node-b", "node-a"),
        &ap5_identities(),
        &dual_of(vec![row.clone()]),
        &row,
        &before,
        &after,
        &planned.listener_ports(),
    );
    let failure = result.expect_err("wrong addrbind must refuse");
    assert!(matches!(failure, LinkProofFailure::Refuse { .. }));
}

#[test]
fn planned_listener_port_as_dialer_endpoint_refuses() {
    // B's listen port is 35167, not its outgoing source port 33620: a
    // row whose peer endpoint is a planned listener port is not an
    // ephemeral dial endpoint.
    let row = completed_row(3, "127.0.0.1:35167", A_LISTEN);
    let planned = ap5_planned();
    let rows_table = ap5_tcp_rows();
    let before = bracket_at(5_000, &rows_table);
    let after = bracket_at(5_400, &rows_table);
    let result = decide_directed_dial(
        &dial_of(&planned, "node-b", "node-a"),
        &ap5_identities(),
        &dual_of(vec![row.clone()]),
        &row,
        &before,
        &after,
        &planned.listener_ports(),
    );
    let failure = result.expect_err("listener port as dial endpoint must refuse");
    assert!(matches!(failure, LinkProofFailure::Refuse { .. }));
}

#[test]
fn ipv6_endpoints_fail_closed() {
    let planned = PlannedPeers::new(
        BTreeMap::from([
            ("node-a".to_string(), "[::1]:44919".to_string()),
            ("node-b".to_string(), "[::1]:35167".to_string()),
        ]),
        BTreeMap::from([
            ("node-a".to_string(), "[::1]:35167".to_string()),
            ("node-b".to_string(), "[::1]:44919".to_string()),
        ]),
    )
    .expect("IPv6 plan is structurally valid");
    let row = completed_row(3, "[::1]:33620", "[::1]:44919");
    let rows_table = ap5_tcp_rows();
    let before = bracket_at(5_000, &rows_table);
    let after = bracket_at(5_400, &rows_table);
    let result = decide_directed_dial(
        &dial_of(&planned, "node-b", "node-a"),
        &ap5_identities(),
        &dual_of(vec![row.clone()]),
        &row,
        &before,
        &after,
        &planned.listener_ports(),
    );
    let failure = result.expect_err("IPv6 endpoints must fail closed");
    assert!(matches!(failure, LinkProofFailure::Refuse { .. }));
}

#[test]
fn unplanned_inbound_peer_refuses() {
    // An inbound row on an endpoint nobody planned to be dialed on.
    let rows = BTreeMap::from([(
        "node-a".to_string(),
        dual_of(vec![completed_row(3, "127.0.0.1:33620", "127.0.0.1:9999")]),
    )]);
    let planned = ap5_planned();
    let rows_table = ap5_tcp_rows();
    let before = bracket_at(5_000, &rows_table);
    let after = bracket_at(5_400, &rows_table);
    let pass = observe_peer_graph(&planned, &rows, &ap5_identities(), &before, &after);
    assert_refuse(&pass.failure, "unplanned endpoint");
}

#[test]
fn duplicate_inbound_peers_exceeding_the_plan_refuse() {
    let rows = BTreeMap::from([(
        "node-a".to_string(),
        dual_of(vec![
            completed_row(3, B_EPHEMERAL, A_LISTEN),
            completed_row(4, "127.0.0.1:33621", A_LISTEN),
        ]),
    )]);
    let planned = ap5_planned();
    let rows_table = ap5_tcp_rows();
    let before = bracket_at(5_000, &rows_table);
    let after = bracket_at(5_400, &rows_table);
    let pass = observe_peer_graph(&planned, &rows, &ap5_identities(), &before, &after);
    assert_refuse(&pass.failure, "accepted 2 peers but only 1 are planned");
}

#[test]
fn unplanned_outbound_peer_refuses() {
    let mut row = completed_row(3, "127.0.0.1:33620", A_LISTEN);
    row.inbound = false;
    let rows = BTreeMap::from([("node-a".to_string(), dual_of(vec![row]))]);
    let planned = ap5_planned();
    let rows_table = ap5_tcp_rows();
    let before = bracket_at(5_000, &rows_table);
    let after = bracket_at(5_400, &rows_table);
    let pass = observe_peer_graph(&planned, &rows, &ap5_identities(), &before, &after);
    assert_refuse(&pass.failure, "peers outside the plan");
}

#[test]
fn planned_outbound_row_is_accounted_but_never_proves() {
    // The pinned node's IBD dials stay outside PeerHub, so a planned
    // outbound row (if one ever appears) is plan-conformant but cannot
    // satisfy the receiver-handshake proof.
    let mut outbound = completed_row(9, B_LISTEN, A_EPHEMERAL);
    outbound.inbound = false;
    // addr must equal node-a's planned connect target (B_LISTEN).
    outbound.addr = B_LISTEN.to_string();
    let mut duals = ap5_duals();
    // The extra row is observed in BOTH samples (identity stability).
    let node_a = duals.get_mut("node-a").unwrap();
    node_a.a.rows.push(outbound.clone());
    node_a.b.rows.push(outbound);
    let planned = ap5_planned();
    let rows_table = ap5_tcp_rows();
    let before = bracket_at(5_000, &rows_table);
    let after = bracket_at(5_400, &rows_table);
    let pass = observe_peer_graph(&planned, &duals, &ap5_identities(), &before, &after);
    assert!(pass.failure.is_none(), "{:?}", pass.failure);
    // The outbound row covered nothing by itself: both dials were proven
    // by the acceptors' inbound rows.
    assert_eq!(pass.proofs.len(), 2);
}

#[test]
fn only_one_directed_dial_proven_is_not_ready() {
    // B -> A proven, A -> B missing: the undirected edge alone must not
    // pass the gate.
    let rows = BTreeMap::from([(
        "node-a".to_string(),
        dual_of(vec![completed_row(3, B_EPHEMERAL, A_LISTEN)]),
    )]);
    let planned = ap5_planned();
    let rows_table = ap5_tcp_rows();
    let before = bracket_at(5_000, &rows_table);
    let after = bracket_at(5_400, &rows_table);
    let pass = observe_peer_graph(&planned, &rows, &ap5_identities(), &before, &after);
    assert_retry(
        &pass.failure,
        "directed dial node-a -> node-b is not yet proven",
    );
    assert_eq!(pass.covered_dials.len(), 1);
}

// -- Kernel-socket join negatives ---------------------------------------

#[test]
fn socket_owned_by_another_process_refuses() {
    // The row is in node-b's namespace-wide table but its inode is not
    // in node-b's fd set: the connection belongs to another process.
    let planned = ap5_planned();
    let rows_table = ap5_tcp_rows();
    let before = bracket_at(5_000, &rows_table);
    let after = bracket_at(5_400, &rows_table);
    let mut starved = before.clone();
    let b_sample = ProcessSample {
        pid: PID_B,
        start_ticks: Some(START_B),
        // B keeps its listener and accepted sockets but NOT its dial
        // socket: the tuple's inode is owned by another process.
        socket_inodes: Some([INODE_B_LISTEN, INODE_B_ACCEPT].into_iter().collect()),
        tcp_rows: rows_table.clone(),
        netns_identity: Some(HOST_NETNS),
        exe: Some(NODE_EXE.to_string()),
    };
    starved.processes.insert("node-b".to_string(), b_sample);
    let result = decide_directed_dial(
        &dial_of(&planned, "node-b", "node-a"),
        &ap5_identities(),
        &dual_of(vec![completed_row(3, B_EPHEMERAL, A_LISTEN)]),
        &completed_row(3, B_EPHEMERAL, A_LISTEN),
        &starved,
        &after,
        &planned.listener_ports(),
    );
    let failure = result.expect_err("unowned socket must refuse");
    assert!(matches!(failure, LinkProofFailure::Refuse { .. }));
}

#[test]
fn absent_socket_retries() {
    let planned = ap5_planned();
    let mut rows_table = ap5_tcp_rows();
    rows_table.retain(|row| !(row.local == lo(33620) && row.remote == lo(44919)));
    let before = bracket_at(5_000, &rows_table);
    let after = bracket_at(5_400, &rows_table);
    let result = decide_directed_dial(
        &dial_of(&planned, "node-b", "node-a"),
        &ap5_identities(),
        &dual_of(vec![completed_row(3, B_EPHEMERAL, A_LISTEN)]),
        &completed_row(3, B_EPHEMERAL, A_LISTEN),
        &before,
        &after,
        &planned.listener_ports(),
    );
    let failure = result.expect_err("absent socket must retry");
    assert!(matches!(failure, LinkProofFailure::Retry { .. }));
}

#[test]
fn wrong_port_on_the_dialer_socket_retries() {
    // Same IP, one port off: exact tuple matching, never IP-only or
    // suffix matching.
    let planned = ap5_planned();
    let mut rows_table = ap5_tcp_rows();
    rows_table.retain(|row| !(row.local == lo(33620) && row.remote == lo(44919)));
    rows_table.push(tcp(
        lo(33621),
        lo(44919),
        TCP_STATE_ESTABLISHED,
        INODE_B_DIAL,
    ));
    let before = bracket_at(5_000, &rows_table);
    let after = bracket_at(5_400, &rows_table);
    let result = decide_directed_dial(
        &dial_of(&planned, "node-b", "node-a"),
        &ap5_identities(),
        &dual_of(vec![completed_row(3, B_EPHEMERAL, A_LISTEN)]),
        &completed_row(3, B_EPHEMERAL, A_LISTEN),
        &before,
        &after,
        &planned.listener_ports(),
    );
    let failure = result.expect_err("wrong-port socket must retry");
    assert!(matches!(failure, LinkProofFailure::Retry { .. }));
}

#[test]
fn syn_sent_and_time_wait_states_retry() {
    for state in ["03", "06"] {
        let planned = ap5_planned();
        let mut rows_table = ap5_tcp_rows();
        for row in &mut rows_table {
            if row.local == lo(33620) && row.remote == lo(44919) {
                row.state = state.to_string();
            }
        }
        let before = bracket_at(5_000, &rows_table);
        let after = bracket_at(5_400, &rows_table);
        let result = decide_directed_dial(
            &dial_of(&planned, "node-b", "node-a"),
            &ap5_identities(),
            &dual_of(vec![completed_row(3, B_EPHEMERAL, A_LISTEN)]),
            &completed_row(3, B_EPHEMERAL, A_LISTEN),
            &before,
            &after,
            &planned.listener_ports(),
        );
        let failure = result
            .expect_err("non-established state must retry")
            .detail()
            .to_string();
        assert!(failure.contains("not established"), "{failure}");
    }
}

#[test]
fn missing_acceptor_side_socket_retries() {
    // Only one endpoint of the connection is visible: the exact reverse
    // tuple is required.
    let planned = ap5_planned();
    let mut rows_table = ap5_tcp_rows();
    rows_table.retain(|row| !(row.local == lo(44919) && row.remote == lo(33620)));
    let before = bracket_at(5_000, &rows_table);
    let after = bracket_at(5_400, &rows_table);
    let result = decide_directed_dial(
        &dial_of(&planned, "node-b", "node-a"),
        &ap5_identities(),
        &dual_of(vec![completed_row(3, B_EPHEMERAL, A_LISTEN)]),
        &completed_row(3, B_EPHEMERAL, A_LISTEN),
        &before,
        &after,
        &planned.listener_ports(),
    );
    let failure = result.expect_err("single-sided socket must retry");
    assert!(matches!(failure, LinkProofFailure::Retry { .. }));
}

#[test]
fn inode_replaced_between_brackets_retries() {
    let planned = ap5_planned();
    let rows_table = ap5_tcp_rows();
    let before = bracket_at(5_000, &rows_table);
    // The connection was replaced between the bracket samples: the same
    // tuple exists but with a new inode, still fd-owned by B (as a
    // re-dial would be). The stale first socket must not be accepted.
    let mut replaced = ap5_tcp_rows();
    for row in &mut replaced {
        if row.local == lo(33620) && row.remote == lo(44919) {
            row.inode = INODE_B_DIAL + 1;
        }
    }
    let mut after = bracket_at(5_400, &replaced);
    let b_sample = ProcessSample {
        pid: PID_B,
        start_ticks: Some(START_B),
        socket_inodes: Some(
            [INODE_B_LISTEN, INODE_B_ACCEPT, INODE_B_DIAL + 1]
                .into_iter()
                .collect(),
        ),
        tcp_rows: replaced.clone(),
        netns_identity: Some(HOST_NETNS),
        exe: Some(NODE_EXE.to_string()),
    };
    after.processes.insert("node-b".to_string(), b_sample);
    let result = decide_directed_dial(
        &dial_of(&planned, "node-b", "node-a"),
        &ap5_identities(),
        &dual_of(vec![completed_row(3, B_EPHEMERAL, A_LISTEN)]),
        &completed_row(3, B_EPHEMERAL, A_LISTEN),
        &before,
        &after,
        &planned.listener_ports(),
    );
    let failure = result.expect_err("replaced socket must retry");
    assert!(matches!(failure, LinkProofFailure::Retry { .. }));
}

// -- Identity and bracket negatives --------------------------------------

#[test]
fn pid_reuse_start_tick_change_refuses() {
    let planned = ap5_planned();
    let rows_table = ap5_tcp_rows();
    let before = bracket_at(5_000, &rows_table);
    let mut after = bracket_at(5_400, &rows_table);
    let reused = process_sample(PID_B, START_B + 7, fds_b(), &rows_table);
    after.processes.insert("node-b".to_string(), reused);
    let result = decide_directed_dial(
        &dial_of(&planned, "node-b", "node-a"),
        &ap5_identities(),
        &dual_of(vec![completed_row(3, B_EPHEMERAL, A_LISTEN)]),
        &completed_row(3, B_EPHEMERAL, A_LISTEN),
        &before,
        &after,
        &planned.listener_ports(),
    );
    let failure = result.expect_err("start-tick change must refuse");
    assert!(matches!(failure, LinkProofFailure::Refuse { .. }));
}

#[test]
fn unreadable_start_ticks_refuse() {
    let planned = ap5_planned();
    let rows_table = ap5_tcp_rows();
    let before = bracket_at(5_000, &rows_table);
    let mut after = bracket_at(5_400, &rows_table);
    let mut unreadable = process_sample(PID_B, START_B, fds_b(), &rows_table);
    unreadable.start_ticks = None;
    after.processes.insert("node-b".to_string(), unreadable);
    let result = decide_directed_dial(
        &dial_of(&planned, "node-b", "node-a"),
        &ap5_identities(),
        &dual_of(vec![completed_row(3, B_EPHEMERAL, A_LISTEN)]),
        &completed_row(3, B_EPHEMERAL, A_LISTEN),
        &before,
        &after,
        &planned.listener_ports(),
    );
    let failure = result.expect_err("unreadable start ticks must refuse");
    assert!(matches!(failure, LinkProofFailure::Refuse { .. }));
}

#[test]
fn unreadable_fd_table_refuses() {
    let planned = ap5_planned();
    let rows_table = ap5_tcp_rows();
    let before = bracket_at(5_000, &rows_table);
    let mut after = bracket_at(5_400, &rows_table);
    let mut unreadable = process_sample(PID_B, START_B, fds_b(), &rows_table);
    unreadable.socket_inodes = None;
    after.processes.insert("node-b".to_string(), unreadable);
    let result = decide_directed_dial(
        &dial_of(&planned, "node-b", "node-a"),
        &ap5_identities(),
        &dual_of(vec![completed_row(3, B_EPHEMERAL, A_LISTEN)]),
        &completed_row(3, B_EPHEMERAL, A_LISTEN),
        &before,
        &after,
        &planned.listener_ports(),
    );
    let failure = result.expect_err("unreadable fd table must refuse");
    assert!(matches!(failure, LinkProofFailure::Refuse { .. }));
}

#[test]
fn boot_id_change_refuses() {
    let planned = ap5_planned();
    let rows_table = ap5_tcp_rows();
    let before = bracket_at(5_000, &rows_table);
    let mut after = bracket_at(5_400, &rows_table);
    after.boot_id = Some("00000000-0000-4000-8000-000000000000".to_string());
    let result = decide_directed_dial(
        &dial_of(&planned, "node-b", "node-a"),
        &ap5_identities(),
        &dual_of(vec![completed_row(3, B_EPHEMERAL, A_LISTEN)]),
        &completed_row(3, B_EPHEMERAL, A_LISTEN),
        &before,
        &after,
        &planned.listener_ports(),
    );
    let failure = result.expect_err("boot id change must refuse");
    assert!(matches!(failure, LinkProofFailure::Refuse { .. }));
}

#[test]
fn unreadable_boot_id_refuses() {
    let planned = ap5_planned();
    let rows_table = ap5_tcp_rows();
    let before = bracket_at(5_000, &rows_table);
    let mut after = bracket_at(5_400, &rows_table);
    after.boot_id = None;
    let result = decide_directed_dial(
        &dial_of(&planned, "node-b", "node-a"),
        &ap5_identities(),
        &dual_of(vec![completed_row(3, B_EPHEMERAL, A_LISTEN)]),
        &completed_row(3, B_EPHEMERAL, A_LISTEN),
        &before,
        &after,
        &planned.listener_ports(),
    );
    let failure = result.expect_err("unreadable boot id must refuse");
    assert!(matches!(failure, LinkProofFailure::Refuse { .. }));
}

#[test]
fn missing_dialer_identity_refuses() {
    let planned = ap5_planned();
    let rows_table = ap5_tcp_rows();
    let before = bracket_at(5_000, &rows_table);
    let after = bracket_at(5_400, &rows_table);
    let mut identities = ap5_identities();
    identities.remove("node-b");
    let result = decide_directed_dial(
        &dial_of(&planned, "node-b", "node-a"),
        &identities,
        &dual_of(vec![completed_row(3, B_EPHEMERAL, A_LISTEN)]),
        &completed_row(3, B_EPHEMERAL, A_LISTEN),
        &before,
        &after,
        &planned.listener_ports(),
    );
    let failure = result.expect_err("missing identity must refuse");
    assert!(matches!(failure, LinkProofFailure::Refuse { .. }));
}

// -- Raw getpeerinfo row parsing -----------------------------------------

fn raw_completed_entry(id: i64, peer: &str, listener: &str) -> serde_json::Value {
    json!({
        "id": id,
        "addr": peer,
        "addrbind": listener,
        "subver": EXPECTED_SUBVER,
        "inbound": true,
        "services": format!("{EXPECTED_SERVICE_BASE_MASK:016x}"),
        "servicesnames": "NETWORK, WITNESS, P2P_V2",
        "version": EXPECTED_PROTOCOL_VERSION,
        "startingheight": 0,
        "lastsend": 0,
        "lastrecv": 0,
        "conntime": 1_762_000_000,
        "bytesrecv_per_msg": {"version": 110, "verack": 24, "getheaders": 29},
        "bytessent_per_msg": {"version": 110, "verack": 24},
        "connection_type": "inbound",
        "relaytxes": true,
        "transport_protocol_type": "v2",
        "network": "ipv4",
        "synced_headers": -1,
        "synced_blocks": -1,
    })
}

#[test]
fn raw_entry_parses_strictly_with_all_typed_fields() {
    let row = PeerHandshakeRow::parse(&raw_completed_entry(3, B_EPHEMERAL, A_LISTEN))
        .expect("a real pinned-node row parses");
    assert_eq!(row.id, 3);
    assert_eq!(row.addr, B_EPHEMERAL);
    assert_eq!(row.addrbind, A_LISTEN);
    assert!(row.inbound);
    assert_eq!(row.version, EXPECTED_PROTOCOL_VERSION);
    assert_eq!(row.subver, EXPECTED_SUBVER);
    assert_eq!(row.services, "0000000000000809");
    assert_eq!(row.startingheight, 0);
    assert_eq!(row.bytesrecv_version, Some(110));
    assert_eq!(row.bytesrecv_verack, Some(24));
    assert_eq!(row.bytesrecv_getheaders, Some(29));
    assert_eq!(row.transport_protocol_type.as_deref(), Some("v2"));
    assert_eq!(row.connection_type.as_deref(), Some("inbound"));
    assert!(!row.is_connecting());
}

#[test]
fn raw_entry_without_getheaders_parses_with_none() {
    let mut entry = raw_completed_entry(3, B_EPHEMERAL, A_LISTEN);
    entry["bytesrecv_per_msg"] = json!({"version": 110, "verack": 24});
    let row = PeerHandshakeRow::parse(&entry).expect("parses");
    assert_eq!(row.bytesrecv_getheaders, None);
}

#[test]
fn raw_entry_missing_required_fields_fails_closed() {
    for missing in [
        "id",
        "addr",
        "addrbind",
        "inbound",
        "version",
        "subver",
        "services",
        "startingheight",
        "bytesrecv_per_msg",
    ] {
        let mut entry = raw_completed_entry(3, B_EPHEMERAL, A_LISTEN);
        entry.as_object_mut().unwrap().remove(missing);
        assert!(
            PeerHandshakeRow::parse(&entry).is_err(),
            "row without {missing} must fail closed"
        );
    }
}

#[test]
fn raw_entry_with_malformed_types_fails_closed() {
    let mut entry = raw_completed_entry(3, B_EPHEMERAL, A_LISTEN);
    entry["inbound"] = json!("yes");
    assert!(PeerHandshakeRow::parse(&entry).is_err());
    let mut entry = raw_completed_entry(3, B_EPHEMERAL, A_LISTEN);
    entry["id"] = json!("3");
    assert!(PeerHandshakeRow::parse(&entry).is_err());
    let mut entry = raw_completed_entry(3, B_EPHEMERAL, A_LISTEN);
    entry["bytesrecv_per_msg"]["verack"] = json!("24");
    assert!(PeerHandshakeRow::parse(&entry).is_err());
}

// -- Kernel table parsing -------------------------------------------------

#[test]
fn hex_endpoints_decode_little_endian_v4_and_v6() {
    assert_eq!(
        parse_hex_endpoint("0100007F:8354"),
        Some(SocketAddr::from(([127, 0, 0, 1], 0x8354)))
    );
    assert_eq!(
        parse_hex_endpoint("00000000000000000000000001000000:0050"),
        Some(SocketAddr::from((
            [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
            80
        )))
    );
    assert_eq!(parse_hex_endpoint("0100007:8354"), None);
    assert_eq!(parse_hex_endpoint("0100007F:ZZZZ"), None);
    assert_eq!(parse_hex_endpoint("no-colon"), None);
}

#[test]
fn tcp_table_parses_real_shaped_lines_and_skips_malformed() {
    let text = concat!(
        "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n",
        "   0: 0100007F:8354 0100007F:AF77 01 00000000:00000000 00:00000000 00000000     0        0 82012 1 0000000000000000 24 4 30 10 -1\n",
        "   1: 0100007F:AF77 0100007F:8354 01 00000000:00000000 00:00000000 00000000     0        0 82007 1 0000000000000000 24 4 30 10 -1\n",
        "   2: 0100007F:AF77 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 82006 1 0000000000000000 24 4 30 10 -1\n",
        "   3: malformed line\n",
    );
    let rows = parse_tcp_table(text);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].local, lo(0x8354));
    assert_eq!(rows[0].remote, lo(0xAF77));
    assert_eq!(rows[0].state, "01");
    assert_eq!(rows[0].inode, 82_012);
    assert_eq!(rows[1].local, lo(0xAF77));
    assert_eq!(rows[1].inode, 82_007);
    assert_eq!(rows[2].state, "0A");
    assert_eq!(rows[2].inode, 82_006);
}

// -- Planned graph validation ---------------------------------------------

#[test]
fn planned_peers_reject_half_wired_and_ambiguous_graphs() {
    let empty = BTreeMap::new();
    // A listen without a dial.
    assert!(PlannedPeers::new(
        BTreeMap::from([("a".to_string(), "127.0.0.1:1".to_string())]),
        empty
    )
    .is_err());
    // Two nodes sharing one listen endpoint.
    assert!(PlannedPeers::new(
        BTreeMap::from([
            ("a".to_string(), "127.0.0.1:1".to_string()),
            ("b".to_string(), "127.0.0.1:1".to_string()),
        ]),
        BTreeMap::from([
            ("a".to_string(), "127.0.0.1:2".to_string()),
            ("b".to_string(), "127.0.0.1:1".to_string()),
        ]),
    )
    .is_err());
    // A dial of an endpoint nobody plans to listen on.
    assert!(PlannedPeers::new(
        BTreeMap::from([("a".to_string(), "127.0.0.1:1".to_string())]),
        BTreeMap::from([("a".to_string(), "127.0.0.1:2".to_string())]),
    )
    .is_err());
}

#[test]
fn planned_peers_expose_dials_edges_and_listener_ports() {
    let planned = ap5_planned();
    assert_eq!(planned.listener_ports(), BTreeSet::from([44919, 35167]));
    assert_eq!(planned.dials().len(), 2);
    assert_eq!(planned.edges().len(), 1);
    assert_eq!(planned.dialers_of(A_LISTEN), vec!["node-b".to_string()]);
    assert_eq!(
        planned.listen_owner(B_LISTEN, "node-a"),
        Some("node-b".to_string())
    );
}

// -- Live /proc checks (hermetic: only this test process) ----------------

#[test]
fn live_listener_ownership_uses_exact_addresses() {
    let pid = std::process::id();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let bound = listener.local_addr().expect("local addr");
    // Exact planned-endpoint check: address AND port.
    assert_eq!(listener_owned_at(pid, bound), ListenerOwnership::Owned);
    // Same IP, one port off: never suffix-only matching.
    let mut neighbor = bound;
    neighbor.set_port((bound.port() % 65_534) + 1);
    assert_eq!(
        listener_owned_at(pid, neighbor),
        ListenerOwnership::NotOwned
    );
    // The listener's socket is in our own fd table (deduplicated view).
    let inodes = process_socket_inodes(pid).expect("own fd table");
    let rows = pid_tcp_rows(pid);
    let listen_inode = rows
        .iter()
        .find(|row| row.state == TCP_STATE_LISTEN && row.local == bound)
        .expect("own listener row")
        .inode;
    assert!(inodes.contains(&listen_inode));
    // Our own identity is readable and stable across reads.
    assert_eq!(process_start_ticks(pid), process_start_ticks(pid));
}

// -- Contract version 3: acquisition clock and raw evidence ---------------

/// Reversed bracket stamps refuse at the pass level (checked timestamp
/// order; never a saturating accept).
#[test]
fn reversed_bracket_timestamps_refuse() {
    let planned = ap5_planned();
    let rows_table = ap5_tcp_rows();
    let before = bracket_at(5_000, &rows_table);
    // The after bracket is stamped BEFORE the before bracket.
    let after = bracket_at(4_600, &rows_table);
    let pass = observe_peer_graph(&planned, &ap5_duals(), &ap5_identities(), &before, &after);
    assert_refuse(&pass.failure, "bracket timestamps are reversed");
}

/// A peer sample stamped outside the bracket interval refuses: a stamp
/// can never precede the reads it claims to enclose (read-time-after-
/// stamp inversions are detectable offline and rejected).
#[test]
fn peer_sample_stamps_outside_the_bracket_interval_refuse() {
    let planned = ap5_planned();
    let identities = ap5_identities();
    let rows_table = ap5_tcp_rows();
    let before = bracket_at(5_000, &rows_table);
    let after = bracket_at(5_400, &rows_table);
    // Sample B stamped after the after bracket (5_500 > 5_400).
    let late = PeerDualSample {
        a: PeerSample {
            rows: vec![completed_row(3, B_EPHEMERAL, A_LISTEN)],
            sampled_at_mono_ms: 5_100,
        },
        b: PeerSample {
            rows: vec![completed_row(3, B_EPHEMERAL, A_LISTEN)],
            sampled_at_mono_ms: 5_500,
        },
    };
    let result = decide_directed_dial(
        &dial_of(&planned, "node-b", "node-a"),
        &identities,
        &late,
        &late.b.rows[0],
        &before,
        &after,
        &planned.listener_ports(),
    );
    assert!(result
        .expect_err("a sample stamped after the after bracket must refuse")
        .detail()
        .contains("do not lie inside the bracket interval"));
    // Sample A stamped before the before bracket (4_900 < 5_000).
    let early = PeerDualSample {
        a: PeerSample {
            rows: vec![completed_row(3, B_EPHEMERAL, A_LISTEN)],
            sampled_at_mono_ms: 4_900,
        },
        b: PeerSample {
            rows: vec![completed_row(3, B_EPHEMERAL, A_LISTEN)],
            sampled_at_mono_ms: 5_300,
        },
    };
    let result = decide_directed_dial(
        &dial_of(&planned, "node-b", "node-a"),
        &identities,
        &early,
        &early.b.rows[0],
        &before,
        &after,
        &planned.listener_ports(),
    );
    assert!(result
        .expect_err("a sample stamped before the before bracket must refuse")
        .detail()
        .contains("do not lie inside the bracket interval"));
}

/// The observed peer set must be exactly stable across the two samples
/// of one acquisition: an identity change on the same peer id retries
/// (replaced connection).
#[test]
fn peer_identity_changed_between_samples_retries() {
    let planned = ap5_planned();
    let rows_table = ap5_tcp_rows();
    let before = bracket_at(5_000, &rows_table);
    let after = bracket_at(5_400, &rows_table);
    let mut changed = dual_of(vec![completed_row(3, B_EPHEMERAL, A_LISTEN)]);
    changed.b.rows[0].addr = "127.0.0.1:33699".to_string();
    let duals = BTreeMap::from([("node-a".to_string(), changed)]);
    let pass = observe_peer_graph(&planned, &duals, &ap5_identities(), &before, &after);
    assert_retry(&pass.failure, "peer identities changed between the resampled");
}

/// A peer present in sample B but absent from sample A retries.
#[test]
fn peer_id_absent_from_sample_a_retries() {
    let planned = ap5_planned();
    let identities = ap5_identities();
    let rows_table = ap5_tcp_rows();
    let before = bracket_at(5_000, &rows_table);
    let after = bracket_at(5_400, &rows_table);
    let mut absent = dual_of(vec![completed_row(3, B_EPHEMERAL, A_LISTEN)]);
    absent.a.rows.clear();
    let result = decide_directed_dial(
        &dial_of(&planned, "node-b", "node-a"),
        &identities,
        &absent,
        &absent.b.rows[0],
        &before,
        &after,
        &planned.listener_ports(),
    );
    assert!(result
        .expect_err("a peer absent from sample A must retry")
        .detail()
        .contains("absent from the acceptor's sample A"));
}

/// The handshake must be completed in BOTH samples: a connecting row in
/// sample A (handshake finished between the samples) retries.
#[test]
fn connecting_sample_a_row_retries() {
    let planned = ap5_planned();
    let identities = ap5_identities();
    let rows_table = ap5_tcp_rows();
    let before = bracket_at(5_000, &rows_table);
    let after = bracket_at(5_400, &rows_table);
    let mut connecting_a = dual_of(vec![completed_row(3, B_EPHEMERAL, A_LISTEN)]);
    connecting_a.a.rows[0].version = 0;
    connecting_a.a.rows[0].bytesrecv_version = None;
    connecting_a.a.rows[0].bytesrecv_verack = None;
    let result = decide_directed_dial(
        &dial_of(&planned, "node-b", "node-a"),
        &identities,
        &connecting_a,
        &connecting_a.b.rows[0],
        &before,
        &after,
        &planned.listener_ports(),
    );
    assert!(result
        .expect_err("a connecting sample A row must retry")
        .detail()
        .contains("sample A handshake not completed"));
}

/// Byte counters may grow between the two samples: only the identity
/// tuple must be stable (counters are recorded raw in both samples).
#[test]
fn counters_grown_between_samples_still_prove() {
    let planned = ap5_planned();
    let identities = ap5_identities();
    let rows_table = ap5_tcp_rows();
    let before = bracket_at(5_000, &rows_table);
    let after = bracket_at(5_400, &rows_table);
    let mut grown = dual_of(vec![completed_row(3, B_EPHEMERAL, A_LISTEN)]);
    grown.a.rows[0].bytesrecv_version = Some(100);
    grown.a.rows[0].bytesrecv_verack = Some(20);
    grown.a.rows[0].bytesrecv_getheaders = None;
    let proof = decide_directed_dial(
        &dial_of(&planned, "node-b", "node-a"),
        &identities,
        &grown,
        &grown.b.rows[0],
        &before,
        &after,
        &planned.listener_ports(),
    )
    .expect("grown counters between stable identities still prove");
    // Both raw samples are persisted with their own counters.
    assert_eq!(proof.handshake_sample_a.bytesrecv_version, Some(100));
    assert_eq!(proof.handshake.bytesrecv_version, Some(110));
    assert_eq!(proof.handshake_sample_a.bytesrecv_getheaders, None);
}

/// A node process in a different network namespace than the host
/// controller refuses: this host-network proof form does not apply, and
/// the observed (actual) identity - not a declared name - decides.
#[test]
fn node_netns_differs_from_host_controller_refuses() {
    let planned = ap5_planned();
    let identities = ap5_identities();
    let rows_table = ap5_tcp_rows();
    let before = bracket_at(5_000, &rows_table);
    let after = bracket_at(5_400, &rows_table);
    let mut namespaced_before = before.clone();
    let mut namespaced_after = after.clone();
    let mut sample = process_sample(PID_B, START_B, fds_b(), &rows_table);
    sample.netns_identity = Some((9, 4_026_532_001));
    namespaced_before.processes.insert("node-b".to_string(), sample.clone());
    namespaced_after.processes.insert("node-b".to_string(), sample);
    let dual = dual_of(vec![completed_row(3, B_EPHEMERAL, A_LISTEN)]);
    let result = decide_directed_dial(
        &dial_of(&planned, "node-b", "node-a"),
        &identities,
        &dual,
        &dual.b.rows[0],
        &namespaced_before,
        &namespaced_after,
        &planned.listener_ports(),
    );
    assert!(result
        .expect_err("a namespaced node process must refuse")
        .detail()
        .contains("another network namespace"));
}

/// An unreadable node netns identity refuses (an actual value is
/// required, never a None claim).
#[test]
fn node_netns_unreadable_refuses() {
    let planned = ap5_planned();
    let identities = ap5_identities();
    let rows_table = ap5_tcp_rows();
    let before = bracket_at(5_000, &rows_table);
    let after = bracket_at(5_400, &rows_table);
    let mut unreadable = before.clone();
    let mut sample = process_sample(PID_B, START_B, fds_b(), &rows_table);
    sample.netns_identity = None;
    unreadable.processes.insert("node-b".to_string(), sample);
    let dual = dual_of(vec![completed_row(3, B_EPHEMERAL, A_LISTEN)]);
    let result = decide_directed_dial(
        &dial_of(&planned, "node-b", "node-a"),
        &identities,
        &dual,
        &dual.b.rows[0],
        &unreadable,
        &after,
        &planned.listener_ports(),
    );
    assert!(result
        .expect_err("an unreadable node netns identity must refuse")
        .detail()
        .contains("network namespace identity of process node-b is unreadable"));
}

/// An unreadable or changing host-controller netns identity refuses.
#[test]
fn host_netns_unreadable_or_changed_refuses() {
    let planned = ap5_planned();
    let identities = ap5_identities();
    let rows_table = ap5_tcp_rows();
    let before = bracket_at(5_000, &rows_table);
    let after = bracket_at(5_400, &rows_table);
    let dual = dual_of(vec![completed_row(3, B_EPHEMERAL, A_LISTEN)]);
    let mut no_host = before.clone();
    no_host.host_netns_identity = None;
    let result = decide_directed_dial(
        &dial_of(&planned, "node-b", "node-a"),
        &identities,
        &dual,
        &dual.b.rows[0],
        &no_host,
        &after,
        &planned.listener_ports(),
    );
    assert!(result
        .expect_err("an unreadable host netns identity must refuse")
        .detail()
        .contains("host controller network namespace identity is unreadable"));
    let mut changed_host = after.clone();
    changed_host.host_netns_identity = Some((9, 4_026_532_001));
    let result = decide_directed_dial(
        &dial_of(&planned, "node-b", "node-a"),
        &identities,
        &dual,
        &dual.b.rows[0],
        &before,
        &changed_host,
        &planned.listener_ports(),
    );
    assert!(result
        .expect_err("a changed host netns identity must refuse")
        .detail()
        .contains("host controller network namespace identity changed"));
}

/// An unreadable or changing observed executable refuses.
#[test]
fn exe_unreadable_or_changed_refuses() {
    let planned = ap5_planned();
    let identities = ap5_identities();
    let rows_table = ap5_tcp_rows();
    let before = bracket_at(5_000, &rows_table);
    let after = bracket_at(5_400, &rows_table);
    let dual = dual_of(vec![completed_row(3, B_EPHEMERAL, A_LISTEN)]);
    let mut unreadable = before.clone();
    let mut sample = process_sample(PID_B, START_B, fds_b(), &rows_table);
    sample.exe = None;
    unreadable.processes.insert("node-b".to_string(), sample);
    let result = decide_directed_dial(
        &dial_of(&planned, "node-b", "node-a"),
        &identities,
        &dual,
        &dual.b.rows[0],
        &unreadable,
        &after,
        &planned.listener_ports(),
    );
    assert!(result
        .expect_err("an unreadable exe must refuse")
        .detail()
        .contains("executable of process node-b is unreadable"));
    let mut changed = after.clone();
    let mut sample = process_sample(PID_B, START_B, fds_b(), &rows_table);
    sample.exe = Some("/usr/bin/other-node".to_string());
    changed.processes.insert("node-b".to_string(), sample);
    let result = decide_directed_dial(
        &dial_of(&planned, "node-b", "node-a"),
        &identities,
        &dual,
        &dual.b.rows[0],
        &before,
        &changed,
        &planned.listener_ports(),
    );
    assert!(result
        .expect_err("a changed exe must refuse")
        .detail()
        .contains("executable of process node-b changed"));
}

/// The source-declared bounds serialize with all five fields and round
/// trip; a consumer cross-checks the acquisition against exactly these
/// trusted numbers.
#[test]
fn acquisition_bounds_serialize_with_all_source_declared_fields() {
    let bounds = ReadinessAcquisitionBounds {
        max_acquisition_ms: 90_000,
        max_rpc_call_ms: READINESS_MAX_RPC_CALL_MS,
        max_peer_rows: READINESS_MAX_PEER_ROWS,
        max_tcp_rows: READINESS_MAX_TCP_ROWS,
        max_fd_entries: READINESS_MAX_FD_ENTRIES,
    };
    let text = serde_json::to_string(&bounds).expect("bounds serialize");
    assert!(text.contains("\"max_acquisition_ms\":90000"));
    assert!(text.contains("\"max_rpc_call_ms\":10000"));
    assert!(text.contains("\"max_peer_rows\":256"));
    assert!(text.contains("\"max_tcp_rows\":8192"));
    assert!(text.contains("\"max_fd_entries\":8192"));
    let back: ReadinessAcquisitionBounds =
        serde_json::from_str(&text).expect("bounds deserialize");
    assert_eq!(back, bounds);
    // The whole-acquisition record round trips with its clock stamps.
    let record = ReadinessAcquisitionRecord {
        started_at_mono_ms: 1_000,
        deadline_mono_ms: 91_000,
        completed_at_mono_ms: 90_500,
        bounds: bounds.clone(),
    };
    let text = serde_json::to_string(&record).expect("record serializes");
    let back: ReadinessAcquisitionRecord =
        serde_json::from_str(&text).expect("record deserializes");
    assert_eq!(back, record);
}

/// The bounded kernel readers refuse (never truncate) when the
/// source-declared cardinality is exceeded (hermetic: this process's
/// own tables).
#[test]
fn bounded_kernel_readers_refuse_over_cardinality() {
    let pid = std::process::id();
    // This process holds more than one fd.
    assert!(process_socket_inodes_bounded(pid, 1).is_err());
    // Its namespace table holds at least one row.
    assert!(pid_tcp_rows_bounded(pid, 1).is_err());
    // Generous bounds read fine (and the own listener row is present).
    let bounds = ReadinessAcquisitionBounds {
        max_acquisition_ms: 90_000,
        max_rpc_call_ms: READINESS_MAX_RPC_CALL_MS,
        max_peer_rows: READINESS_MAX_PEER_ROWS,
        max_tcp_rows: READINESS_MAX_TCP_ROWS,
        max_fd_entries: READINESS_MAX_FD_ENTRIES,
    };
    let inodes = process_socket_inodes_bounded(pid, bounds.max_fd_entries)
        .expect("own fd table within bounds")
        .expect("own fd table readable");
    let rows = pid_tcp_rows_bounded(pid, bounds.max_tcp_rows).expect("own table within bounds");
    let _ = (inodes, rows);
}

/// Live bracket capture stamps after its reads and observes the actual
/// namespace identities (hermetic: this process only).
#[test]
fn live_bracket_capture_observes_identities_and_stamps_after_reads() {
    let identities = BTreeMap::from([(
        "self".to_string(),
        NodeProcessIdentity {
            node: "self".to_string(),
            pid: std::process::id(),
            start_time_ticks: process_start_ticks(std::process::id())
                .expect("own start ticks"),
            network_namespace: None,
        },
    )]);
    let bounds = ReadinessAcquisitionBounds {
        max_acquisition_ms: 90_000,
        max_rpc_call_ms: READINESS_MAX_RPC_CALL_MS,
        max_peer_rows: READINESS_MAX_PEER_ROWS,
        max_tcp_rows: READINESS_MAX_TCP_ROWS,
        max_fd_entries: READINESS_MAX_FD_ENTRIES,
    };
    let bracket = capture_bracket(&identities, &bounds).expect("own bracket");
    let sample = bracket.processes.get("self").expect("own sample");
    // Actual observed identities, never None or declared names.
    assert!(sample.netns_identity.is_some());
    assert!(sample.exe.is_some());
    assert_eq!(bracket.host_netns_identity, sample.netns_identity);
    // The stamp cannot lie in the future relative to a later read.
    let now = crate::bootstrap::monotonic_ms().expect("monotonic now");
    assert!(bracket.sampled_at_mono_ms <= now);
}
