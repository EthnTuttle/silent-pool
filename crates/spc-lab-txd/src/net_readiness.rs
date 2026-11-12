//! Node network readiness: receiver-handshake with owned reverse-TCP
//! proofs for the pre-hash gate (observational contract version 3).
//!
//! Version 3 closes two source gaps confirmed against the built v2
//! package by the AP5 consumer review:
//!
//! * **Whole-acquisition deadline (V2-TIME).** The acquisition clock
//!   starts once, before the first bracket read, and its deadline is
//!   never reset: the bootstrap operation's own deadline capped by the
//!   engine's source constant. Every read of the acquisition - process
//!   identity, namespace identity, listeners, both peer resamples -
//!   happens inside that window, every timestamp is taken AFTER the
//!   reads it encloses completed, timestamp order is checked (never
//!   saturating-accepted), and the deadline is checked on SUCCESS
//!   before a ready record is persisted or any hash budget reserved.
//!   A structurally valid observation completed after the deadline is
//!   unready. All cardinalities (peer rows, per-PID TCP rows, fd
//!   entries) and every RPC call are bounded by the source-declared
//!   [`ReadinessAcquisitionBounds`], so one acquisition cannot hold
//!   engine locks indefinitely.
//! * **Raw bracket evidence (V2-BRACKET).** The observed network
//!   namespace identity (`(st_dev, st_ino)` of `/proc/<pid>/ns/net`)
//!   is captured per designated PID and for the host controller in
//!   both brackets, and must be an actual (non-`None`) value, stable,
//!   and identical to the host controller's. P2P and RPC LISTEN
//!   ownership is proven from the bracket samples before AND after
//!   the peer samples. The peer rows are resampled (A after the
//!   before bracket, B before the after bracket) and the exact
//!   identity tuples must be stable across the acquisition. The
//!   minimal RAW before/after samples - fd socket-inode membership,
//!   per-process TCP state/four-tuples, namespace identity, exe, peer
//!   rows and their timing - are persisted in the public records; the
//!   summary booleans alone are not the proof.
//!
//! The bootstrap readiness gate must prove that every planned directed
//! node dial `D -> R` is a live Bitcoin connection BEFORE the sole hash
//! budget of the operation is reserved. The pinned rbitcoin node reports
//! `getpeerinfo` from its PeerHub snapshot only, while the `--connect`
//! dials of the initial-block-download phase deliberately stay outside
//! the dialer's PeerHub (`rbitcoin-ibd/src/ibd/peer_io.rs` opens the
//! socket, handshakes and keeps a private `PeerSlot`); the connection
//! surfaces only as an `inbound` row on the acceptor. A gate that
//! requires each node's `outbound` list to contain its planned peer is
//! therefore cyclic at genesis, and a nonzero peer count proves nothing
//! about ownership.
//!
//! This module implements the source-backed replacement evidence form
//! `receiver_handshake_with_owned_reverse_tcp`. For every planned
//! directed dial `D -> R`:
//!
//! 1. R's planned P2P listener is proven separately: a LISTEN socket on
//!    the exact planned local address/port whose inode is reachable from
//!    R's fd table (the engine checks this before observing peers).
//! 2. A typed, completed inbound `getpeerinfo` row on R with peer id,
//!    `addrbind` = R's planned listener, `addr` = D's ephemeral endpoint,
//!    protocol version 70016, the pinned rbitcoin subver, the pinned
//!    service bits and received VERSION/VERACK accounting markers.
//!    Connecting rows (`version == 0`, absent markers) never pass.
//! 3. Both kernel sockets of the connection are proven: in D's
//!    `/proc/<D_pid>/net/tcp` view an ESTABLISHED row with
//!    `(local = addr, remote = addrbind)` whose inode is open by D's fd
//!    table, and independently in R's view the exact reversed tuple
//!    `(local = addrbind, remote = addr)` owned by R. The two endpoint
//!    inodes are normally different; the reversed four-tuples are
//!    matched, never inode equality across endpoints.
//! 4. The RPC sample is bracketed by identity/kernel samples: boot id,
//!    `/proc/<pid>/stat` start ticks, fd tables and TCP views must be
//!    consistent before and after, with the same peer id/endpoints and
//!    socket ownership/state across the recorded monotonic interval.
//!
//! Direction is retained exactly as reported (no inbound relabelling);
//! coverage of the undirected edge is derived only after the directed
//! dial's typed proof succeeds, and every planned directed dial remains
//! an independent obligation. Report-only rows never prove anything.
//!
//! Scope: the proof supports supervisor-launched processes identified by
//! host PIDs with literal IPv4 planned endpoints (the admitted AP5
//! launch profile). Unknown address families or non-literal endpoints
//! fail closed. No UID/IP/count-only mapping, NAT inference, `addnode`
//! side effects, scanning of unrelated processes or hash-gate bypass is
//! performed, and no secret token or environment bytes are published:
//! proofs record PID/start-tick identity, sockets and handshake fields
//! only.
//!
//! Source-bound constants verified against the pinned node source
//! `/nix/store/hzhwz6j2d31y2bm4iz1awyr5s6xzasg9-source`: protocol version
//! 70016 (`rbitcoin-net/src/peer.rs`, `OUR_PROTOCOL_VERSION`), subver
//! `/rbitcoin:0.8.99/` (workspace version, no UA comments in the
//! admitted argv; the IBD dialer builds it with
//! `rbitcoin_subversion(env!("CARGO_PKG_VERSION"), &[])`), service base
//! `NETWORK | WITNESS | P2P_V2` = 0x809 (`rbitcoin-net/src/seeds.rs`,
//! `required_seed_services`), and the received
//! `bytesrecv_per_msg.version`/`.verack` completion markers
//! (`rbitcoin-net/src/peers.rs` inbound registration). A received
//! `getheaders` counter on the same row is recorded as corroboration;
//! the schema encodes explicitly that it is NOT required.

use std::collections::{BTreeMap, BTreeSet};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};

use serde::{Deserialize, Serialize};

/// Observational contract version of the readiness evidence.
///
/// 3 = receiver-handshake with owned reverse TCP, whole-acquisition
/// deadline and raw bracket/namespace evidence. Records of earlier
/// versions fail to parse against the version 3 types (the new
/// mandatory fields are absent and the old field sets are rejected);
/// there is no silent extension or fallback acceptance.
pub const READINESS_CONTRACT_VERSION: u32 = 3;
/// The evidence form name persisted in every proof record.
pub const EVIDENCE_FORM: &str = "receiver_handshake_with_owned_reverse_tcp";
/// P2P protocol version the pinned node binaries advertise
/// (`OUR_PROTOCOL_VERSION`; BIP339 wtxidrelay needs >= 70016).
pub const EXPECTED_PROTOCOL_VERSION: i64 = 70016;
/// BIP14 user agent the pinned node sends (workspace version 0.8.99,
/// no UA comments).
pub const EXPECTED_SUBVER: &str = "/rbitcoin:0.8.99/";
/// Service bits every non-pruned pinned node advertises:
/// `NETWORK | WITNESS | P2P_V2`.
pub const EXPECTED_SERVICE_BASE_MASK: u64 = 0x809;

/// `/proc/net/tcp` state field for ESTABLISHED sockets.
pub const TCP_STATE_ESTABLISHED: &str = "01";
/// `/proc/net/tcp` state field for LISTEN sockets.
pub const TCP_STATE_LISTEN: &str = "0A";

// ---------------------------------------------------------------------------
// Source-declared acquisition bounds (contract version 3)
// ---------------------------------------------------------------------------

/// Source-declared bound on one RPC call of a readiness acquisition.
/// The typed host traffic transport is internally bounded at exactly
/// this value (3 s connect, 10 s hard whole-call read deadline,
/// `traffic/rpc.rs` `call_direct`); namespaced broker calls never run
/// inside a bounded acquisition (declared-namespace nodes are refused
/// before any RPC).
pub const READINESS_MAX_RPC_CALL_MS: u64 = 10_000;
/// Source-declared per-node `getpeerinfo` row cardinality bound. The
/// supported lab topology plans at most a handful of peers per node
/// (the ring profiles dial exactly one); 256 rows is two orders of
/// magnitude of headroom while keeping one sample bounded.
pub const READINESS_MAX_PEER_ROWS: usize = 256;
/// Source-declared per-PID TCP-table row cardinality bound (both
/// `/proc/<pid>/net/tcp` and `tcp6` combined). A lab node's namespace
/// carries at most a few hundred sockets; 8192 rows bounds the read
/// without ever truncating a supported topology.
pub const READINESS_MAX_TCP_ROWS: usize = 8_192;
/// Source-declared per-PID fd-table entry cardinality bound. Same
/// derivation as [`READINESS_MAX_TCP_ROWS`]: bounded read, never a
/// truncation of a supported process.
pub const READINESS_MAX_FD_ENTRIES: usize = 8_192;

/// The source-declared bounds of one readiness acquisition (contract
/// version 3). Declared by the trusted engine profile and serialized
/// with every report so a consumer can cross-check the acquisition it
/// is handed; an incoming untrusted proof never chooses a bound.
///
/// Numeric derivation (all from this trusted source, not from the
/// observed system): the whole-acquisition budget is the bootstrap
/// operation deadline capped by the engine's readiness timeout
/// constant; every RPC call is bounded at
/// [`READINESS_MAX_RPC_CALL_MS`]; the kernel-table cardinalities are
/// bounded at the constants above. A consumer that wants tighter
/// bounds applies its own trusted profile and rejects the record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadinessAcquisitionBounds {
    /// Whole-acquisition wall budget from the gate's monotonic start
    /// (before the first bracket read) to the final after-bracket
    /// completion. Never reset by a retry.
    pub max_acquisition_ms: u64,
    /// Per-RPC-call bound ([`READINESS_MAX_RPC_CALL_MS`]).
    pub max_rpc_call_ms: u64,
    /// Per-node getpeerinfo row cardinality bound
    /// ([`READINESS_MAX_PEER_ROWS`]).
    pub max_peer_rows: usize,
    /// Per-PID TCP-table row cardinality bound
    /// ([`READINESS_MAX_TCP_ROWS`]).
    pub max_tcp_rows: usize,
    /// Per-PID fd-table entry cardinality bound
    /// ([`READINESS_MAX_FD_ENTRIES`]).
    pub max_fd_entries: usize,
}

/// The whole-acquisition clock record persisted with every readiness
/// report (contract version 3): the gate's monotonic start, its
/// never-reset deadline, the stamp taken after the final read
/// completed, and the source-declared bounds. All stamps share one
/// monotonic clock domain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadinessAcquisitionRecord {
    /// Monotonic stamp at gate start, before the first bracket read.
    pub started_at_mono_ms: u64,
    /// The never-reset acquisition deadline (the bootstrap operation
    /// deadline capped by the engine's source constant).
    pub deadline_mono_ms: u64,
    /// Monotonic stamp after the final pass's last read completed.
    pub completed_at_mono_ms: u64,
    /// The source-declared bounds of this acquisition.
    pub bounds: ReadinessAcquisitionBounds,
}

// ---------------------------------------------------------------------------
// Kernel TCP table parsing (per-PID namespace views)
// ---------------------------------------------------------------------------

/// One row of `/proc/net/tcp` or `/proc/net/tcp6`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TcpRow {
    pub local: SocketAddr,
    pub remote: SocketAddr,
    /// Raw hex state field (`01` established, `0A` listen, ...).
    pub state: String,
    pub inode: u64,
}

/// Parse one kernel TCP table (`/proc/net/tcp` or `/proc/net/tcp6`
/// text). Malformed rows are skipped: the kernel emits well-formed
/// tables, and a skipped row can never weaken a proof because every
/// proof requires a positive row match in both bracket samples.
pub fn parse_tcp_table(text: &str) -> Vec<TcpRow> {
    let mut rows = Vec::new();
    for line in text.lines().skip(1) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 10 {
            continue;
        }
        let (Some(local), Some(remote), Ok(inode)) = (
            parse_hex_endpoint(fields[1]),
            parse_hex_endpoint(fields[2]),
            fields[9].parse::<u64>(),
        ) else {
            continue;
        };
        rows.push(TcpRow {
            local,
            remote,
            state: fields[3].to_string(),
            inode,
        });
    }
    rows
}

/// Read the TCP tables of `pid`'s own network namespace
/// (`/proc/<pid>/net/tcp{,6}`).
///
/// The per-PID view lists that namespace's sockets, not only the PID's
/// sockets: every lookup is intersected with the PID's fd-inode set by
/// the caller.
pub fn pid_tcp_rows(pid: u32) -> Vec<TcpRow> {
    let mut rows = Vec::new();
    for table in [
        format!("/proc/{pid}/net/tcp"),
        format!("/proc/{pid}/net/tcp6"),
    ] {
        if let Ok(text) = std::fs::read_to_string(&table) {
            rows.extend(parse_tcp_table(&text));
        }
    }
    rows
}

/// Parse a `/proc/net/tcp{,6}` endpoint field (`hexaddr:hexport`).
///
/// IPv4 addresses are one little-endian 32-bit word; IPv6 addresses are
/// four 32-bit words, each printed with its bytes in little-endian order
/// (the kernel's `%08X` formatting of `in6_addr` words). Ports are hex.
fn parse_hex_endpoint(field: &str) -> Option<SocketAddr> {
    let (address, port) = field.split_once(':')?;
    let port = u16::from_str_radix(port, 16).ok()?;
    if address.len() == 8 {
        let word = u32::from_str_radix(address, 16).ok()?;
        Some(SocketAddr::V4(std::net::SocketAddrV4::new(
            Ipv4Addr::from(word.swap_bytes()),
            port,
        )))
    } else if address.len() == 32 {
        let mut bytes = [0u8; 16];
        for (index, chunk) in address.as_bytes().chunks(8).enumerate() {
            let text = std::str::from_utf8(chunk).ok()?;
            let word = u32::from_str_radix(text, 16).ok()?;
            bytes[4 * index..4 * index + 4].copy_from_slice(&word.to_le_bytes());
        }
        Some(SocketAddr::V6(std::net::SocketAddrV6::new(
            Ipv6Addr::from(bytes),
            port,
            0,
            0,
        )))
    } else {
        None
    }
}

/// Socket inodes currently open by `pid` (its `/proc/<pid>/fd` table).
///
/// Duplicate fd aliases of one inode are normal and deduplicate into the
/// set. `None` when the table cannot be read; proofs treat that as a
/// refusal (fail closed) once the process is confirmed alive.
pub fn process_socket_inodes(pid: u32) -> Option<BTreeSet<u64>> {
    let entries = std::fs::read_dir(format!("/proc/{pid}/fd")).ok()?;
    let mut inodes = BTreeSet::new();
    for entry in entries.flatten() {
        let Ok(target) = std::fs::read_link(entry.path()) else {
            continue;
        };
        let target = target.to_string_lossy();
        if let Some(rest) = target.strip_prefix("socket:[") {
            if let Some(number) = rest.strip_suffix(']') {
                if let Ok(inode) = number.parse::<u64>() {
                    inodes.insert(inode);
                }
            }
        }
    }
    Some(inodes)
}

/// `/proc/<pid>/stat` field 22: the process start time in clock ticks.
///
/// The same identity token the supervisor records; a mismatch means the
/// recorded PID was reused by another process.
pub fn process_start_ticks(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let close = stat.rfind(')')?;
    // Fields after the comm field start at field 3 (state). starttime is
    // field 22, so it is the 20th token after the closing parenthesis.
    stat[close + 1..]
        .split_whitespace()
        .nth(19)?
        .parse::<u64>()
        .ok()
}

/// The ACTUAL observed network-namespace identity of `pid`:
/// `(st_dev, st_ino)` of `/proc/<pid>/ns/net`, read via metadata.
/// `None` when unreadable: proofs treat that as a refusal (fail
/// closed). This is never a declared namespace name.
pub fn netns_identity_of(pid: u32) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata(format!("/proc/{pid}/ns/net")).ok()?;
    Some((meta.dev(), meta.ino()))
}

/// The host controller's own observed network-namespace identity
/// (`/proc/self/ns/net`), read by the observing process itself.
/// `None` when unreadable: proofs treat that as a refusal.
pub fn host_netns_identity() -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata("/proc/self/ns/net").ok()?;
    Some((meta.dev(), meta.ino()))
}

/// The observed executable of `pid` (`/proc/<pid>/exe` readlink).
/// Public, nonsecret binding; `None` when unreadable.
pub fn process_exe(pid: u32) -> Option<String> {
    std::fs::read_link(format!("/proc/{pid}/exe"))
        .ok()
        .map(|path| path.to_string_lossy().into_owned())
}

/// Bounded fd-table read: the socket inodes open by `pid`, refusing
/// (not truncating) when the table exceeds the source-declared
/// cardinality bound. `Ok(None)` = unreadable table (fail closed by
/// the proof layer); `Err` = cardinality exceeded.
pub fn process_socket_inodes_bounded(
    pid: u32,
    max_entries: usize,
) -> Result<Option<BTreeSet<u64>>, String> {
    let entries = match std::fs::read_dir(format!("/proc/{pid}/fd")) {
        Ok(entries) => entries,
        Err(_) => return Ok(None),
    };
    let mut inodes = BTreeSet::new();
    let mut count = 0usize;
    for entry in entries.flatten() {
        count += 1;
        if count > max_entries {
            return Err(format!(
                "fd table of pid {pid} exceeds the source-declared bound ({max_entries} entries)"
            ));
        }
        let Ok(target) = std::fs::read_link(entry.path()) else {
            continue;
        };
        let target = target.to_string_lossy();
        if let Some(rest) = target.strip_prefix("socket:[") {
            if let Some(number) = rest.strip_suffix(']') {
                if let Ok(inode) = number.parse::<u64>() {
                    inodes.insert(inode);
                }
            }
        }
    }
    Ok(Some(inodes))
}

/// Bounded per-PID TCP-table read (both tables combined), refusing
/// (not truncating) when the row count exceeds the source-declared
/// cardinality bound. An unreadable table yields no rows; a proof that
/// needs this namespace's sockets then fails closed on the positive
/// match requirement.
pub fn pid_tcp_rows_bounded(pid: u32, max_rows: usize) -> Result<Vec<TcpRow>, String> {
    let mut rows = Vec::new();
    for table in [
        format!("/proc/{pid}/net/tcp"),
        format!("/proc/{pid}/net/tcp6"),
    ] {
        if let Ok(text) = std::fs::read_to_string(&table) {
            for row in parse_tcp_table(&text) {
                if rows.len() >= max_rows {
                    return Err(format!(
                        "tcp tables of pid {pid} exceed the source-declared bound ({max_rows} rows)"
                    ));
                }
                rows.push(row);
            }
        }
    }
    Ok(rows)
}

/// The Linux boot id, via the crate's validated helper (36-character
/// hex-and-dash form). `None` (unreadable or malformed) fails closed.
pub fn boot_id() -> Option<String> {
    crate::bootstrap::boot_id().ok()
}

/// Outcome of an exact-address listener ownership check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListenerOwnership {
    /// The LISTEN socket bound to exactly this endpoint (address AND
    /// port, no suffix matching) is open by the pid's fd table.
    Owned,
    /// No LISTEN socket at this exact endpoint is open by the pid:
    /// either nothing listens there or another process owns it. A
    /// listener is not a connection; this never proves a dial.
    NotOwned,
    /// The per-PID TCP tables or the fd table cannot be read: unknown,
    /// never confirmation (fail closed).
    Unreadable,
}

/// Check that the LISTEN socket bound to exactly `endpoint` (address AND
/// port, no suffix matching) is a file descriptor of `pid`, using the
/// pid's own network-namespace view intersected with its fd-inode set.
pub fn listener_owned_at(pid: u32, endpoint: SocketAddr) -> ListenerOwnership {
    let rows = pid_tcp_rows(pid);
    if rows.is_empty() {
        // An entirely unreadable namespace view (both tables missing).
        return ListenerOwnership::Unreadable;
    }
    let mut inode = None;
    for row in rows {
        if row.state == TCP_STATE_LISTEN && row.local == endpoint {
            inode = Some(row.inode);
            break;
        }
    }
    let Some(inode) = inode else {
        return ListenerOwnership::NotOwned;
    };
    let Some(inodes) = process_socket_inodes(pid) else {
        return ListenerOwnership::Unreadable;
    };
    if inodes.contains(&inode) {
        ListenerOwnership::Owned
    } else {
        ListenerOwnership::NotOwned
    }
}

/// Exact-address LISTEN ownership decided from an already-captured
/// process sample (contract version 3): the LISTEN row at exactly
/// `endpoint` (address AND port, no suffix matching) in the sample's
/// namespace tables, with its inode open by the sample's fd-inode set.
/// Used for the before AND after bracket checks.
pub fn listener_owned_in_sample(sample: &ProcessSample, endpoint: SocketAddr) -> ListenerOwnership {
    let mut inode = None;
    for row in &sample.tcp_rows {
        if row.state == TCP_STATE_LISTEN && row.local == endpoint {
            inode = Some(row.inode);
            break;
        }
    }
    let Some(inode) = inode else {
        return ListenerOwnership::NotOwned;
    };
    let Some(inodes) = sample.socket_inodes.as_ref() else {
        return ListenerOwnership::Unreadable;
    };
    if inodes.contains(&inode) {
        ListenerOwnership::Owned
    } else {
        ListenerOwnership::NotOwned
    }
}

// ---------------------------------------------------------------------------
// Planned peer graph and directed dials
// ---------------------------------------------------------------------------

/// The planned P2P graph: each node's declared `--listen` endpoint and
/// `--connect` target, as literal `ip:port` strings.
///
/// Built by the engine from launch profiles that already passed
/// fail-closed validation; [`PlannedPeers::new`] applies the same rules
/// for other callers (the strict-gate regression builds one directly
/// from the argv it launched).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedPeers {
    /// node -> planned `--listen` endpoint.
    pub listen: BTreeMap<String, String>,
    /// node -> planned `--connect` target.
    pub connect: BTreeMap<String, String>,
}

impl PlannedPeers {
    /// Fail-closed construction: a listen without a dial, a dial of an
    /// unknown endpoint, or two nodes sharing one listen endpoint are
    /// all rejected.
    pub fn new(
        listen: BTreeMap<String, String>,
        connect: BTreeMap<String, String>,
    ) -> Result<Self, String> {
        if listen.keys().collect::<BTreeSet<_>>() != connect.keys().collect::<BTreeSet<_>>() {
            return Err("nodes mix --listen and --connect".into());
        }
        if listen.values().collect::<BTreeSet<_>>().len() != listen.len() {
            return Err("two nodes plan the same listen endpoint".into());
        }
        for (node, target) in &connect {
            if !listen
                .iter()
                .any(|(listener, endpoint)| endpoint == target && listener != node)
            {
                return Err(format!("node {node} dials unplanned endpoint {target}"));
            }
        }
        Ok(Self { listen, connect })
    }

    /// Undirected edges as sorted node-id pairs.
    pub fn edges(&self) -> BTreeSet<(String, String)> {
        self.dials()
            .iter()
            .map(|dial| sorted_pair(&dial.dialer, &dial.acceptor))
            .collect()
    }

    /// The planned directed dials (one obligation per `--connect`).
    pub fn dials(&self) -> BTreeSet<PlannedDial> {
        let mut dials = BTreeSet::new();
        for (node, target) in &self.connect {
            if let Some(owner) = self.listen_owner(target, node) {
                dials.insert(PlannedDial {
                    dialer: node.clone(),
                    acceptor: owner,
                    target: target.clone(),
                });
            }
        }
        dials
    }

    /// The nodes planning to dial `listen_endpoint`.
    pub fn dialers_of(&self, listen_endpoint: &str) -> Vec<String> {
        self.connect
            .iter()
            .filter(|(_, target)| *target == listen_endpoint)
            .map(|(node, _)| node.clone())
            .collect()
    }

    /// The node (other than `excluding`) whose planned listener is
    /// `target`.
    pub fn listen_owner(&self, target: &str, excluding: &str) -> Option<String> {
        self.listen
            .iter()
            .find(|(node, endpoint)| *endpoint == target && node.as_str() != excluding)
            .map(|(node, _)| node.clone())
    }

    /// Every planned listener port (the ephemeral-endpoint check
    /// excludes exactly these).
    pub fn listener_ports(&self) -> BTreeSet<u16> {
        self.listen
            .values()
            .filter_map(|endpoint| endpoint.rsplit_once(':'))
            .filter_map(|(_, port)| port.parse::<u16>().ok())
            .collect()
    }
}

/// One planned directed dial `dialer -> acceptor` at `target` (the
/// acceptor's planned listen endpoint, i.e. the dialer's declared
/// `--connect` target).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PlannedDial {
    pub dialer: String,
    pub acceptor: String,
    pub target: String,
}

/// Sorted (undirected) pair.
fn sorted_pair(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}

// ---------------------------------------------------------------------------
// Handshake rows (raw getpeerinfo fields, persisted)
// ---------------------------------------------------------------------------

/// One `getpeerinfo` row with the raw handshake fields the proof needs,
/// parsed strictly. Malformed or missing fields are an error: the old
/// classifier's "not true means outbound" fallback is not acceptable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeerHandshakeRow {
    /// The PeerHub peer id (matched across the bracket interval).
    pub id: i64,
    /// The peer's socket address as reported (the remote endpoint).
    pub addr: String,
    /// The connection's local bind as reported (the listener side for
    /// inbound rows).
    pub addrbind: String,
    /// Accepted-on-listener, exactly as reported.
    pub inbound: bool,
    /// The peer's P2P protocol version (0 for connecting rows).
    pub version: i64,
    /// The peer's BIP14 user agent (empty for connecting rows).
    pub subver: String,
    /// The peer's service bits, as the `{:016x}` string reported.
    pub services: String,
    /// The peer's starting height (-1 for connecting rows; not a
    /// handshake indicator).
    pub startingheight: i64,
    /// `bytesrecv_per_msg.version`, when received. The pinned node
    /// accounts at least a 24-byte header per received message
    /// (`peers.rs` `acct_bytes`), so a present entry is always positive.
    #[serde(default)]
    pub bytesrecv_version: Option<u64>,
    /// `bytesrecv_per_msg.verack`, when received (present entries carry
    /// the accounted 24-byte header; a zero payload is normal for
    /// VERACK).
    #[serde(default)]
    pub bytesrecv_verack: Option<u64>,
    /// `bytesrecv_per_msg.getheaders` on the same row, when received
    /// (post-handshake corroboration; NOT required - see
    /// [`DirectedDialProof::getheaders_corroboration_required`]).
    #[serde(default)]
    pub bytesrecv_getheaders: Option<u64>,
    /// Reported transport protocol type (recorded only; `v2` alone
    /// proves nothing).
    #[serde(default)]
    pub transport_protocol_type: Option<String>,
    /// Reported connection type (recorded only).
    #[serde(default)]
    pub connection_type: Option<String>,
}

/// Why a handshake row does not carry completed pinned-node markers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandshakeMarkerFailure {
    /// The row is a connecting/incomplete row: transient, retry.
    Incomplete(String),
    /// The row's identity contradicts the pinned node: foreign peer,
    /// refuse.
    Foreign(String),
}

impl PeerHandshakeRow {
    /// Strict parse of one raw getpeerinfo entry. Any missing or
    /// malformed required field is an error (fail closed; no absent-field
    /// inference).
    pub fn parse(entry: &serde_json::Value) -> Result<Self, String> {
        let id = entry
            .get("id")
            .and_then(|value| value.as_i64())
            .ok_or("row without integer id")?;
        let addr = entry
            .get("addr")
            .and_then(|value| value.as_str())
            .ok_or("row without addr")?
            .to_string();
        let addrbind = entry
            .get("addrbind")
            .and_then(|value| value.as_str())
            .ok_or("row without addrbind")?
            .to_string();
        let inbound = entry
            .get("inbound")
            .and_then(|value| value.as_bool())
            .ok_or("row without boolean inbound")?;
        let version = entry
            .get("version")
            .and_then(|value| value.as_i64())
            .ok_or("row without integer version")?;
        let subver = entry
            .get("subver")
            .and_then(|value| value.as_str())
            .ok_or("row without subver")?
            .to_string();
        let services = entry
            .get("services")
            .and_then(|value| value.as_str())
            .ok_or("row without services")?
            .to_string();
        let startingheight = entry
            .get("startingheight")
            .and_then(|value| value.as_i64())
            .ok_or("row without startingheight")?;
        let counters = entry
            .get("bytesrecv_per_msg")
            .ok_or("row without bytesrecv_per_msg")?;
        let counter = |name: &str| -> Result<Option<u64>, String> {
            match counters.get(name) {
                Some(value) => value
                    .as_u64()
                    .map(Some)
                    .ok_or_else(|| format!("bytesrecv_per_msg.{name} is not an integer")),
                None => Ok(None),
            }
        };
        Ok(Self {
            id,
            addr,
            addrbind,
            inbound,
            version,
            subver,
            services,
            startingheight,
            bytesrecv_version: counter("version")?,
            bytesrecv_verack: counter("verack")?,
            bytesrecv_getheaders: counter("getheaders")?,
            transport_protocol_type: entry
                .get("transport_protocol_type")
                .and_then(|value| value.as_str())
                .map(str::to_string),
            connection_type: entry
                .get("connection_type")
                .and_then(|value| value.as_str())
                .map(str::to_string),
        })
    }

    /// True for connecting rows: version 0 or absent VERSION/VERACK
    /// accounting markers. Such rows must never satisfy readiness.
    pub fn is_connecting(&self) -> bool {
        self.version == 0 || self.bytesrecv_version.is_none() || self.bytesrecv_verack.is_none()
    }

    /// The identity tuple that must be exactly stable across the two
    /// peer samples of one bounded acquisition (contract version 3):
    /// peer id, both endpoints, direction, protocol version, user
    /// agent and services. Byte counters are excluded; they are
    /// recorded raw in both samples and may grow.
    pub fn identity_tuple(&self) -> (i64, String, String, bool, i64, String, String) {
        (
            self.id,
            self.addr.clone(),
            self.addrbind.clone(),
            self.inbound,
            self.version,
            self.subver.clone(),
            self.services.clone(),
        )
    }

    /// Validate the completed inbound handshake markers against the
    /// pinned node source: protocol 70016, subver
    /// `/rbitcoin:0.8.99/`, the `NETWORK | WITNESS | P2P_V2` service
    /// bits and received VERSION/VERACK accounting. Positive synced
    /// heights, `relaytxes` or `transport_protocol_type` are NOT
    /// handshake indicators and are not required.
    pub fn completed_inbound_markers(&self) -> Result<(), HandshakeMarkerFailure> {
        if self.version == 0 {
            return Err(HandshakeMarkerFailure::Incomplete(
                "connecting row (version 0)".into(),
            ));
        }
        let marker = |name: &str, value: Option<u64>| -> Result<(), HandshakeMarkerFailure> {
            match value {
                // The pinned node's `acct_bytes` accounts a 24-byte
                // header per received message, so a real received
                // marker is always positive; zero contradicts it.
                Some(count) if count > 0 => Ok(()),
                Some(_) => Err(HandshakeMarkerFailure::Foreign(format!(
                    "received {name} accounting marker is not positive"
                ))),
                None => Err(HandshakeMarkerFailure::Incomplete(format!(
                    "no received {name} accounting marker yet"
                ))),
            }
        };
        marker("version", self.bytesrecv_version)?;
        marker("verack", self.bytesrecv_verack)?;
        if self.version != EXPECTED_PROTOCOL_VERSION {
            return Err(HandshakeMarkerFailure::Foreign(format!(
                "peer protocol version {} is not the pinned {EXPECTED_PROTOCOL_VERSION}",
                self.version
            )));
        }
        if self.subver != EXPECTED_SUBVER {
            return Err(HandshakeMarkerFailure::Foreign(format!(
                "peer subver {subver} is not the pinned {EXPECTED_SUBVER}",
                subver = self.subver
            )));
        }
        let Ok(services) = u64::from_str_radix(&self.services, 16) else {
            return Err(HandshakeMarkerFailure::Foreign(format!(
                "peer services {services} is not a hex service field",
                services = self.services
            )));
        };
        if services & EXPECTED_SERVICE_BASE_MASK != EXPECTED_SERVICE_BASE_MASK {
            return Err(HandshakeMarkerFailure::Foreign(format!(
                "peer services {services:#x} lack the pinned NETWORK|WITNESS|P2P_V2 bits {EXPECTED_SERVICE_BASE_MASK:#x}"
            )));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Identities and bracket samples
// ---------------------------------------------------------------------------

/// Identity of one planned node process for link proofs: the
/// supervisor's recorded host PID and its `/proc/<pid>/stat` start-time
/// token. No ownership token or environment bytes are published.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeProcessIdentity {
    pub node: String,
    pub pid: u32,
    pub start_time_ticks: u64,
    /// The engine's expected network namespace for the node, when one is
    /// declared (recorded only; the kernel views are per-PID).
    pub network_namespace: Option<String>,
}

/// One process's kernel sample: identity, fd socket-inode set, the
/// TCP tables of its own network namespace, and the observed namespace
/// and executable identities (contract version 3).
#[derive(Debug, Clone)]
pub struct ProcessSample {
    pub pid: u32,
    /// Live start ticks (`None`: unreadable - fail closed).
    pub start_ticks: Option<u64>,
    /// fd socket inodes (`None`: unreadable - fail closed). Duplicate fd
    /// aliases of one inode deduplicate.
    pub socket_inodes: Option<BTreeSet<u64>>,
    /// `/proc/<pid>/net/tcp{,6}` rows of the process's namespace.
    pub tcp_rows: Vec<TcpRow>,
    /// The ACTUAL observed network-namespace identity `(st_dev,
    /// st_ino)` of `/proc/<pid>/ns/net` (`None`: unreadable - fail
    /// closed). Never the declared namespace name.
    pub netns_identity: Option<(u64, u64)>,
    /// The observed executable (`/proc/<pid>/exe` readlink; public,
    /// nonsecret). `None`: unreadable.
    pub exe: Option<String>,
}

/// One bracket sample: every planned process's kernel/identity sample,
/// the boot id, the host controller's own observed namespace identity,
/// taken at a monotonic timestamp stamped AFTER all reads completed.
#[derive(Debug, Clone)]
pub struct BracketSample {
    pub boot_id: Option<String>,
    /// The host controller's observed netns identity (`/proc/self/
    /// ns/net`), captured with the same bracket. Must be `Some` and
    /// stable; host-network node processes must match it.
    pub host_netns_identity: Option<(u64, u64)>,
    pub processes: BTreeMap<String, ProcessSample>,
    /// Monotonic stamp taken AFTER every read of this bracket
    /// completed; it can never precede the observations it encloses.
    pub sampled_at_mono_ms: u64,
}

/// Capture a live bracket sample for the given identities under the
/// source-declared cardinality bounds. Every read (identity, fd table,
/// TCP tables, namespace identity, executable, boot id, host namespace)
/// completes BEFORE the monotonic stamp is taken; a cardinality
/// violation is an error (never a truncation).
pub fn capture_bracket(
    identities: &BTreeMap<String, NodeProcessIdentity>,
    bounds: &ReadinessAcquisitionBounds,
) -> Result<BracketSample, String> {
    let mut processes = BTreeMap::new();
    for identity in identities.values() {
        let start_ticks = process_start_ticks(identity.pid);
        let socket_inodes = process_socket_inodes_bounded(identity.pid, bounds.max_fd_entries)?;
        let tcp_rows = pid_tcp_rows_bounded(identity.pid, bounds.max_tcp_rows)?;
        let netns_identity = netns_identity_of(identity.pid);
        let exe = process_exe(identity.pid);
        processes.insert(
            identity.node.clone(),
            ProcessSample {
                pid: identity.pid,
                start_ticks,
                socket_inodes,
                tcp_rows,
                netns_identity,
                exe,
            },
        );
    }
    let boot_id = boot_id();
    let host_netns_identity = host_netns_identity();
    // The stamp is taken AFTER every read completed (contract version
    // 3): the bracket's timestamp encloses its observations.
    let sampled_at_mono_ms = crate::bootstrap::monotonic_ms().map_err(|error| error.to_string())?;
    Ok(BracketSample {
        boot_id,
        host_netns_identity,
        processes,
        sampled_at_mono_ms,
    })
}

// ---------------------------------------------------------------------------
// Raw persisted samples and the dual peer resample (contract version 3)
// ---------------------------------------------------------------------------

/// One raw TCP row as persisted in the public bracket records: the
/// four-tuple, the raw kernel state field and the socket inode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawTcpRow {
    pub local: String,
    pub remote: String,
    pub state: String,
    pub inode: u64,
}

/// One process's raw observation as persisted for offline checking:
/// the fd socket-inode membership, the process's OWN TCP rows (raw
/// state and four-tuples, filtered to the sockets its fd table holds),
/// the observed network-namespace identity and the observed exe.
/// Booleans in the typed proofs are summaries; these are the proof.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawProcessObservation {
    pub pid: u32,
    pub start_ticks: Option<u64>,
    /// Observed `(st_dev, st_ino)` of `/proc/<pid>/ns/net`.
    pub netns_identity: Option<[u64; 2]>,
    /// Observed `/proc/<pid>/exe` readlink.
    pub exe: Option<String>,
    /// The fd socket-inode set exactly as read (`None`: unreadable).
    pub fd_socket_inodes: Option<Vec<u64>>,
    /// The process's own TCP rows (listeners and connections), raw.
    pub owned_tcp_rows: Vec<RawTcpRow>,
}

/// The minimal raw bracket record persisted with each proof: only the
/// endpoint processes of that dial, only the sockets their fd tables
/// own (the namespace-wide tables also contain unrelated host sockets,
/// which are not public evidence), with the boot id, the host
/// controller's observed namespace identity and the after-reads
/// monotonic stamp.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawBracketSample {
    pub boot_id: Option<String>,
    pub host_netns_identity: Option<[u64; 2]>,
    pub processes: BTreeMap<String, RawProcessObservation>,
    pub sampled_at_mono_ms: u64,
}

impl BracketSample {
    /// Minimal raw projection of this bracket for the given nodes.
    pub fn to_raw(&self, nodes: &[&str]) -> RawBracketSample {
        let mut processes = BTreeMap::new();
        for node in nodes {
            let Some(sample) = self.processes.get(*node) else {
                continue;
            };
            let owned_tcp_rows = sample
                .tcp_rows
                .iter()
                .filter(|row| {
                    sample
                        .socket_inodes
                        .as_ref()
                        .is_some_and(|inodes| inodes.contains(&row.inode))
                })
                .map(|row| RawTcpRow {
                    local: row.local.to_string(),
                    remote: row.remote.to_string(),
                    state: row.state.clone(),
                    inode: row.inode,
                })
                .collect();
            processes.insert(
                (*node).to_string(),
                RawProcessObservation {
                    pid: sample.pid,
                    start_ticks: sample.start_ticks,
                    netns_identity: sample.netns_identity.map(|(dev, ino)| [dev, ino]),
                    exe: sample.exe.clone(),
                    fd_socket_inodes: sample
                        .socket_inodes
                        .as_ref()
                        .map(|set| set.iter().copied().collect()),
                    owned_tcp_rows,
                },
            );
        }
        RawBracketSample {
            boot_id: self.boot_id.clone(),
            host_netns_identity: self.host_netns_identity.map(|(dev, ino)| [dev, ino]),
            processes,
            sampled_at_mono_ms: self.sampled_at_mono_ms,
        }
    }
}

/// One `getpeerinfo` sample: the strictly typed rows and the monotonic
/// stamp taken AFTER the read completed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeerSample {
    pub rows: Vec<PeerHandshakeRow>,
    pub sampled_at_mono_ms: u64,
}

/// One node's two peer samples inside one bounded acquisition
/// (contract version 3): sample A right after the before bracket,
/// sample B right before the after bracket. The identity tuples
/// (id, addr, addrbind, inbound, version, subver, services) must be
/// exactly stable across A and B; the byte counters are recorded raw
/// in both samples and may grow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeerDualSample {
    pub a: PeerSample,
    pub b: PeerSample,
}

impl PeerDualSample {
    /// The identity tuples of both samples (sorted multisets) are
    /// exactly equal: the observed peer set is stable across the
    /// acquisition. Counters are excluded; they are recorded raw and
    /// may grow between the samples.
    pub fn stable_identities(&self) -> bool {
        let key = |sample: &PeerSample| {
            let mut tuples: Vec<_> = sample.rows.iter().map(|row| row.identity_tuple()).collect();
            tuples.sort();
            tuples
        };
        key(&self.a) == key(&self.b)
    }

    /// Checked clock order: the before-bracket stamp <= sample A <=
    /// sample B <= the after-bracket stamp, all in one monotonic
    /// domain. A violation means a stamp does not enclose the reads it
    /// claims to; refuse (never a saturating accept).
    pub fn ordered_within(&self, before_mono_ms: u64, after_mono_ms: u64) -> bool {
        before_mono_ms <= self.a.sampled_at_mono_ms
            && self.a.sampled_at_mono_ms <= self.b.sampled_at_mono_ms
            && self.b.sampled_at_mono_ms <= after_mono_ms
    }
}

// ---------------------------------------------------------------------------
// The directed-dial proof
// ---------------------------------------------------------------------------

/// Why a directed-dial proof did not complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkProofFailure {
    /// Transient: the planned connection is not physically established
    /// yet, the handshake is incomplete, or the kernel tables moved
    /// between the bracket samples (expired/replaced socket). Retry
    /// within the original deadline; never reset it.
    Retry { detail: String },
    /// Structural: the observation contradicts the plan or the pinned
    /// node identity (unplanned endpoint, foreign process ownership, PID
    /// reuse, unsupported address family, unreadable required table).
    Refuse { detail: String },
}

impl LinkProofFailure {
    pub fn detail(&self) -> &str {
        match self {
            Self::Retry { detail } | Self::Refuse { detail } => detail,
        }
    }
}

/// One endpoint's kernel socket join.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EndpointJoin {
    /// `local:port` of this endpoint's socket.
    pub local: String,
    /// `remote:port` of this endpoint's socket.
    pub remote: String,
    /// Kernel TCP state (required `01`, ESTABLISHED).
    pub state: String,
    /// The socket inode (nonzero).
    pub inode: u64,
    /// The inode is open by this endpoint's designated process fd table.
    pub fd_owned: bool,
}

/// A complete, versioned proof that one planned directed dial is a live
/// Bitcoin connection: the receiver's PeerHub handshake record joined to
/// both process-owned kernel sockets with the exact reversed tuples.
///
/// This is the public evidence record for QA/AP5 binding. It carries the
/// raw RPC handshake fields, both process identities (PID and start
/// ticks only - no tokens or environment bytes), both endpoint joins,
/// the bracket timing, and the operation binding. It persists
/// everything an offline validator needs to recompute the proof.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectedDialProof {
    /// Observational contract version ([`READINESS_CONTRACT_VERSION`]).
    pub contract_version: u32,
    /// The evidence form ([`EVIDENCE_FORM`]).
    pub evidence_form: String,
    /// The command this proof was acquired for (stamped by the engine).
    #[serde(default)]
    pub command_id: Option<String>,
    /// The dialing node.
    pub dialer: String,
    /// The accepting node.
    pub acceptor: String,
    /// The acceptor's planned listen endpoint (the dialer's declared
    /// `--connect` target).
    pub planned_target: String,
    /// The receiver's raw handshake row (typed fields, exactly as
    /// reported).
    pub handshake: PeerHandshakeRow,
    /// Explicitly encoded: a received `getheaders` counter is
    /// corroboration only and is NOT required for this proof. The
    /// observed counter, when present, is in
    /// `handshake.bytesrecv_getheaders`.
    pub getheaders_corroboration_required: bool,
    /// The dialer's recorded PID.
    pub dialer_pid: u32,
    /// The dialer's recorded start ticks.
    pub dialer_start_time_ticks: u64,
    /// The acceptor's recorded PID.
    pub acceptor_pid: u32,
    /// The acceptor's recorded start ticks.
    pub acceptor_start_time_ticks: u64,
    /// The dialer's declared network namespace, when the engine declared
    /// one (recorded only).
    #[serde(default)]
    pub network_namespace_dialer: Option<String>,
    /// The acceptor's declared network namespace, when the engine
    /// declared one (recorded only).
    #[serde(default)]
    pub network_namespace_acceptor: Option<String>,
    /// The Linux boot id shared by both bracket samples.
    pub boot_id: Option<String>,
    /// The dialer-side socket join (local = the dialer's ephemeral
    /// endpoint, remote = the acceptor's listener endpoint).
    pub dialer_socket: EndpointJoin,
    /// The acceptor-side socket join (the exact reversed tuple).
    pub acceptor_socket: EndpointJoin,
    /// Recorded: the two endpoint inodes normally differ; tuple reversal
    /// is the match, never inode equality across endpoints.
    pub endpoint_inodes_differ: bool,
    /// Monotonic timestamp of the bracket sample before the RPC
    /// observation (stamped after that bracket's reads completed).
    pub bracket_before_mono_ms: u64,
    /// Monotonic timestamp of the bracket sample after the RPC
    /// observation (stamped after that bracket's reads completed).
    pub bracket_after_mono_ms: u64,
    /// The bounded monotonic acquisition interval (checked
    /// subtraction: reversed bracket stamps refuse, never saturate).
    pub bracket_interval_ms: u64,
    /// Both tuples were ESTABLISHED with identical inodes and ownership
    /// in both bracket samples.
    pub snapshot_confirmed: bool,
    /// Both processes' live start ticks matched the recorded values in
    /// both bracket samples (no PID reuse; no boot change).
    pub start_verified: bool,
    /// The same peer's row from sample A (identity-stable with the
    /// sample B row; its counters are recorded raw as of sample A).
    pub handshake_sample_a: PeerHandshakeRow,
    /// Monotonic stamps of the two peer samples, each taken after that
    /// read completed; both ordered inside the bracket interval.
    pub peer_sample_a_at_mono_ms: u64,
    pub peer_sample_b_at_mono_ms: u64,
    /// The ACTUAL observed network-namespace identity `(st_dev,
    /// st_ino)` of the dialer's process, identical in both brackets
    /// and equal to the host controller's (contract version 3: the
    /// observed identity, never the declared name).
    pub dialer_netns_identity: [u64; 2],
    /// The acceptor's observed netns identity (same invariants).
    pub acceptor_netns_identity: [u64; 2],
    /// The host controller's observed netns identity (`/proc/self/
    /// ns/net`), identical in both brackets (an actual value, never a
    /// `None` claim).
    pub host_netns_identity: [u64; 2],
    /// The dialer's observed executable (`/proc/<pid>/exe`), stable
    /// across both brackets (public, nonsecret binding).
    pub dialer_exe: String,
    /// The acceptor's observed executable (same invariants).
    pub acceptor_exe: String,
    /// The minimal raw before-bracket sample for this dial's two
    /// endpoint processes: fd socket-inode membership, owned TCP
    /// state/four-tuples, namespace identity, exe, boot id, host
    /// netns identity and the after-reads stamp. The booleans above
    /// are summaries of this record, never a substitute for it.
    pub raw_bracket_before: RawBracketSample,
    /// The raw after-bracket sample (same shape, captured after the
    /// peer samples).
    pub raw_bracket_after: RawBracketSample,
}

/// Look up one process's bracket sample.
fn process_sample<'a>(
    bracket: &'a BracketSample,
    node: &str,
) -> Result<&'a ProcessSample, LinkProofFailure> {
    bracket
        .processes
        .get(node)
        .ok_or_else(|| LinkProofFailure::Refuse {
            detail: format!("process {node} was not sampled"),
        })
}

/// Find the inode of the ESTABLISHED socket `(local, remote)` in the
/// process's namespace tables and require it to be open by the process.
fn established_join(
    sample: &ProcessSample,
    node: &str,
    local: SocketAddr,
    remote: SocketAddr,
) -> Result<u64, LinkProofFailure> {
    let mut established: Option<u64> = None;
    let mut other_state: Option<String> = None;
    for row in &sample.tcp_rows {
        if row.local == local && row.remote == remote {
            if row.state == TCP_STATE_ESTABLISHED {
                established = Some(row.inode);
            } else {
                other_state = Some(row.state.clone());
            }
        }
    }
    let Some(inode) = established else {
        let state = other_state.as_deref().unwrap_or("absent");
        return Err(LinkProofFailure::Retry {
            detail: format!(
                "socket {local} -> {remote} of {node} is not established (state {state})"
            ),
        });
    };
    if inode == 0 {
        return Err(LinkProofFailure::Refuse {
            detail: format!("socket {local} -> {remote} of {node} has a zero inode"),
        });
    }
    let Some(inodes) = sample.socket_inodes.as_ref() else {
        return Err(LinkProofFailure::Refuse {
            detail: format!("fd table of {node} (pid {}) is unreadable", sample.pid),
        });
    };
    if !inodes.contains(&inode) {
        return Err(LinkProofFailure::Refuse {
            detail: format!(
                "socket {local} -> {remote} of {node} is owned by another process (inode {inode})"
            ),
        });
    }
    Ok(inode)
}

/// Verify one process's identity (start ticks) within one bracket
/// sample.
fn verify_start_ticks(
    bracket: &BracketSample,
    identity: &NodeProcessIdentity,
) -> Result<(), LinkProofFailure> {
    let sample = process_sample(bracket, &identity.node)?;
    match sample.start_ticks {
        Some(live) if live == identity.start_time_ticks => Ok(()),
        Some(live) => Err(LinkProofFailure::Refuse {
            detail: format!(
                "process {} (pid {}) start time changed: recorded {}, live {live}",
                identity.node, identity.pid, identity.start_time_ticks
            ),
        }),
        None => Err(LinkProofFailure::Refuse {
            detail: format!(
                "start time of process {} (pid {}) is unreadable",
                identity.node, identity.pid
            ),
        }),
    }
}

/// Pure decision over collected samples: prove the planned directed dial
/// `dial` using the acceptor's dual peer samples (A and B) and the
/// bracket samples taken around them.
///
/// Check order (each negative is pinned by a unit test): a non-inbound
/// row or a foreign handshake identity refuses; a connecting row
/// (version 0, absent markers) in EITHER peer sample retries; a peer
/// id absent from sample A, or an identity-tuple change between the
/// samples, retries (replaced connection); peer-sample stamps outside
/// the bracket interval or reversed bracket stamps refuse (checked
/// order, never a saturating accept); an unreadable, changed or
/// non-host network namespace identity (node or host controller)
/// refuses; an unreadable or changed observed exe refuses; an
/// unplanned acceptor endpoint, a listener port used as the dialer
/// endpoint, a non-IPv4 literal endpoint or an unknown address family
/// refuses; a missing identity or a boot-id change refuses; PID reuse
/// (start-tick mismatch) or an unreadable fd table refuses; a socket
/// not yet ESTABLISHED (absent, SYN_SENT, TIME_WAIT, CLOSE_WAIT)
/// retries; a socket owned by another process refuses; an inode change
/// or disappearance between the bracket samples retries (replaced
/// connection).
pub fn decide_directed_dial(
    dial: &PlannedDial,
    identities: &BTreeMap<String, NodeProcessIdentity>,
    dual: &PeerDualSample,
    row: &PeerHandshakeRow,
    before: &BracketSample,
    after: &BracketSample,
    listener_ports: &BTreeSet<u16>,
) -> Result<DirectedDialProof, LinkProofFailure> {
    let refuse = |detail: String| Err(LinkProofFailure::Refuse { detail });
    let retry = |detail: String| Err(LinkProofFailure::Retry { detail });
    // Direction is retained exactly as reported: only the receiver's
    // inbound row can carry this proof form.
    if !row.inbound {
        return refuse(format!(
            "row {} is not inbound: the receiver-handshake proof requires the acceptor's row",
            row.id
        ));
    }
    // Source-bound handshake markers on the sample B row.
    match row.completed_inbound_markers() {
        Ok(()) => {}
        Err(HandshakeMarkerFailure::Incomplete(detail)) => {
            return retry(format!("handshake not completed yet: {detail}"));
        }
        Err(HandshakeMarkerFailure::Foreign(detail)) => return refuse(detail),
    }
    // The same peer must be present with the same identity tuple in
    // sample A (taken right after the before bracket): the observed
    // peer set is stable across the bounded acquisition.
    let row_a = dual
        .a
        .rows
        .iter()
        .find(|candidate| candidate.id == row.id)
        .ok_or_else(|| LinkProofFailure::Retry {
            detail: format!(
                "peer id {} is absent from the acceptor's sample A (connection replaced)",
                row.id
            ),
        })?;
    match row_a.completed_inbound_markers() {
        Ok(()) => {}
        Err(HandshakeMarkerFailure::Incomplete(detail)) => {
            return retry(format!("sample A handshake not completed yet: {detail}"));
        }
        Err(HandshakeMarkerFailure::Foreign(detail)) => return refuse(detail),
    }
    if row_a.identity_tuple() != row.identity_tuple() {
        return retry(format!(
            "peer id {} identity changed between the resampled peer rows (connection replaced)",
            row.id
        ));
    }
    // Checked clock order: the bracket stamps must enclose the two
    // peer-sample stamps, and the brackets themselves must not be
    // reversed. A stamp that does not enclose its reads refuses.
    if !dual.ordered_within(before.sampled_at_mono_ms, after.sampled_at_mono_ms) {
        return refuse(format!(
            "peer sample stamps ({} -> {} ms) do not lie inside the bracket interval ({} -> {} ms)",
            dual.a.sampled_at_mono_ms,
            dual.b.sampled_at_mono_ms,
            before.sampled_at_mono_ms,
            after.sampled_at_mono_ms
        ));
    }
    let bracket_interval_ms = match after
        .sampled_at_mono_ms
        .checked_sub(before.sampled_at_mono_ms)
    {
        Some(interval) => interval,
        None => {
            return refuse(format!(
                "bracket timestamps are reversed ({} ms after {} ms)",
                after.sampled_at_mono_ms, before.sampled_at_mono_ms
            ));
        }
    };
    // The acceptor endpoint must be the dial's declared target.
    if row.addrbind != dial.target {
        return refuse(format!(
            "row {} addrbind {} is not the planned target {}",
            row.id, row.addrbind, dial.target
        ));
    }
    // Endpoints: literal IPv4 only in this bounded proof form.
    let Ok(dialer_addr) = row.addr.parse::<SocketAddr>() else {
        return refuse(format!(
            "row {} addr {} is not a socket address",
            row.id, row.addr
        ));
    };
    let Ok(acceptor_addr) = row.addrbind.parse::<SocketAddr>() else {
        return refuse(format!(
            "row {} addrbind {} is not a socket address",
            row.id, row.addrbind
        ));
    };
    if !matches!(dialer_addr, SocketAddr::V4(_)) || !matches!(acceptor_addr, SocketAddr::V4(_)) {
        return refuse(format!(
            "proof supports literal IPv4 endpoints only ({} -> {})",
            row.addr, row.addrbind
        ));
    }
    // The dialer endpoint is an ephemeral port, never a planned listener
    // port.
    if listener_ports.contains(&dialer_addr.port()) {
        return refuse(format!(
            "row {} addr {} is a planned listener port, not an ephemeral dial endpoint",
            row.id, row.addr
        ));
    }
    // Identities for both endpoints.
    let Some(dialer_identity) = identities.get(&dial.dialer) else {
        return refuse(format!(
            "planned dialer {} has no process identity",
            dial.dialer
        ));
    };
    let Some(acceptor_identity) = identities.get(&dial.acceptor) else {
        return refuse(format!(
            "planned acceptor {} has no process identity",
            dial.acceptor
        ));
    };
    // Boot id: identical and readable in both brackets.
    match (before.boot_id.as_deref(), after.boot_id.as_deref()) {
        (Some(left), Some(right)) if left == right => {}
        (None, _) | (_, None) => {
            return refuse("boot id is unreadable".into());
        }
        _ => return refuse("boot id changed between bracket samples".into()),
    }
    // Host controller namespace identity: an ACTUAL observed value
    // (never a None or declared claim), identical in both brackets.
    let host_netns = match (before.host_netns_identity, after.host_netns_identity) {
        (Some(left), Some(right)) if left == right => left,
        (None, _) | (_, None) => {
            return refuse("host controller network namespace identity is unreadable".into());
        }
        _ => {
            return refuse(
                "host controller network namespace identity changed between bracket samples".into(),
            );
        }
    };
    // Each endpoint process's observed namespace identity: actual,
    // readable, stable across both brackets, and equal to the host
    // controller's (this host-network proof form does not apply to a
    // process in another namespace).
    let refuse_failure = |detail: String| LinkProofFailure::Refuse { detail };
    let netns_of = |node: &str| -> Result<[u64; 2], LinkProofFailure> {
        match (
            process_sample(before, node)?.netns_identity,
            process_sample(after, node)?.netns_identity,
        ) {
            (None, _) | (_, None) => Err(refuse_failure(format!(
                "network namespace identity of process {node} is unreadable"
            ))),
            (Some(left), Some(right)) if left != right => Err(refuse_failure(format!(
                "network namespace identity of process {node} changed between bracket samples"
            ))),
            (Some(left), Some(_)) => {
                if left != host_netns {
                    Err(refuse_failure(format!(
                        "process {node} lives in another network namespace ({left:?}) than the host controller ({host_netns:?})"
                    )))
                } else {
                    Ok([left.0, left.1])
                }
            }
        }
    };
    let dialer_netns = netns_of(&dial.dialer)?;
    let acceptor_netns = netns_of(&dial.acceptor)?;
    // Each endpoint's observed executable: readable and stable across
    // both brackets (public, nonsecret binding; the source binding is
    // the supervisor's record).
    let exe_of = |node: &str| -> Result<String, LinkProofFailure> {
        match (
            process_sample(before, node)?.exe.clone(),
            process_sample(after, node)?.exe.clone(),
        ) {
            (None, _) | (_, None) => Err(refuse_failure(format!(
                "executable of process {node} is unreadable"
            ))),
            (Some(left), Some(right)) if left != right => Err(refuse_failure(format!(
                "executable of process {node} changed between bracket samples"
            ))),
            (Some(left), Some(_)) => Ok(left),
        }
    };
    let dialer_exe = exe_of(&dial.dialer)?;
    let acceptor_exe = exe_of(&dial.acceptor)?;
    // Identity recheck in both brackets (PID reuse guard).
    verify_start_ticks(before, dialer_identity)?;
    verify_start_ticks(before, acceptor_identity)?;
    verify_start_ticks(after, dialer_identity)?;
    verify_start_ticks(after, acceptor_identity)?;
    // Both process-owned kernel sockets, exact reversed tuples, in both
    // brackets, with stable inodes.
    let dialer_before = process_sample(before, &dial.dialer)?;
    let dialer_after = process_sample(after, &dial.dialer)?;
    let acceptor_before = process_sample(before, &dial.acceptor)?;
    let acceptor_after = process_sample(after, &dial.acceptor)?;
    let dialer_inode_before =
        established_join(dialer_before, &dial.dialer, dialer_addr, acceptor_addr)?;
    let acceptor_inode_before =
        established_join(acceptor_before, &dial.acceptor, acceptor_addr, dialer_addr)?;
    let dialer_inode_after =
        established_join(dialer_after, &dial.dialer, dialer_addr, acceptor_addr)?;
    let acceptor_inode_after =
        established_join(acceptor_after, &dial.acceptor, acceptor_addr, dialer_addr)?;
    if dialer_inode_before != dialer_inode_after || acceptor_inode_before != acceptor_inode_after {
        return retry(format!(
            "socket inode for {} -> {} changed between bracket samples (connection replaced)",
            row.addr, row.addrbind
        ));
    }
    Ok(DirectedDialProof {
        contract_version: READINESS_CONTRACT_VERSION,
        evidence_form: EVIDENCE_FORM.to_string(),
        command_id: None,
        dialer: dial.dialer.clone(),
        acceptor: dial.acceptor.clone(),
        planned_target: dial.target.clone(),
        handshake: row.clone(),
        getheaders_corroboration_required: false,
        dialer_pid: dialer_identity.pid,
        dialer_start_time_ticks: dialer_identity.start_time_ticks,
        acceptor_pid: acceptor_identity.pid,
        acceptor_start_time_ticks: acceptor_identity.start_time_ticks,
        network_namespace_dialer: dialer_identity.network_namespace.clone(),
        network_namespace_acceptor: acceptor_identity.network_namespace.clone(),
        boot_id: before.boot_id.clone(),
        dialer_socket: EndpointJoin {
            local: row.addr.clone(),
            remote: row.addrbind.clone(),
            state: TCP_STATE_ESTABLISHED.to_string(),
            inode: dialer_inode_before,
            fd_owned: true,
        },
        acceptor_socket: EndpointJoin {
            local: row.addrbind.clone(),
            remote: row.addr.clone(),
            state: TCP_STATE_ESTABLISHED.to_string(),
            inode: acceptor_inode_before,
            fd_owned: true,
        },
        endpoint_inodes_differ: dialer_inode_before != acceptor_inode_before,
        bracket_before_mono_ms: before.sampled_at_mono_ms,
        bracket_after_mono_ms: after.sampled_at_mono_ms,
        bracket_interval_ms,
        snapshot_confirmed: true,
        start_verified: true,
        handshake_sample_a: row_a.clone(),
        peer_sample_a_at_mono_ms: dual.a.sampled_at_mono_ms,
        peer_sample_b_at_mono_ms: dual.b.sampled_at_mono_ms,
        dialer_netns_identity: dialer_netns,
        acceptor_netns_identity: acceptor_netns,
        host_netns_identity: [host_netns.0, host_netns.1],
        dialer_exe,
        acceptor_exe,
        raw_bracket_before: before.to_raw(&[&dial.dialer, &dial.acceptor]),
        raw_bracket_after: after.to_raw(&[&dial.dialer, &dial.acceptor]),
    })
}

// ---------------------------------------------------------------------------
// The graph observation pass
// ---------------------------------------------------------------------------

/// Result of one strict observation pass over the planned graph.
#[derive(Debug, Clone, Default)]
pub struct GraphReadinessPass {
    /// Planned directed dials proven by a complete typed proof.
    pub covered_dials: BTreeSet<(String, String)>,
    /// Undirected edges covered by at least one proven dial (derived;
    /// never claimed without a proof).
    pub covered_edges: BTreeSet<(String, String)>,
    /// The proofs that covered the dials (public evidence records).
    pub proofs: Vec<DirectedDialProof>,
    /// Why the pass is not ready, when it is not.
    pub failure: Option<LinkProofFailure>,
}

/// One strict pass over the observed handshake rows.
///
/// Every planned directed dial must be proven by a
/// `receiver_handshake_with_owned_reverse_tcp` proof; report-only rows
/// never cover a dial. All observed P2P rows of both peer samples are
/// accounted: unplanned endpoints, duplicate peers beyond the planned
/// dialer count and foreign-owned connections refuse; incomplete
/// handshakes, unstable peer identities between the resampled rows and
/// sockets not yet established retry within the original deadline.
/// Reversed bracket stamps or peer-sample stamps outside the bracket
/// interval refuse (checked clock order, never saturating).
pub fn observe_peer_graph(
    peers: &PlannedPeers,
    samples_by_node: &BTreeMap<String, PeerDualSample>,
    identities: &BTreeMap<String, NodeProcessIdentity>,
    before: &BracketSample,
    after: &BracketSample,
) -> GraphReadinessPass {
    let mut pass = GraphReadinessPass::default();
    // Pass-level checked clock order: the brackets must not be
    // reversed, and every node's two peer-sample stamps must lie
    // inside the bracket interval.
    if after.sampled_at_mono_ms < before.sampled_at_mono_ms {
        pass.failure = Some(LinkProofFailure::Refuse {
            detail: format!(
                "bracket timestamps are reversed ({} ms after {} ms)",
                after.sampled_at_mono_ms, before.sampled_at_mono_ms
            ),
        });
        return pass;
    }
    for (node, dual) in samples_by_node {
        if !dual.ordered_within(before.sampled_at_mono_ms, after.sampled_at_mono_ms) {
            pass.failure = Some(LinkProofFailure::Refuse {
                detail: format!(
                    "node {node} peer sample stamps ({} -> {} ms) do not lie inside the bracket interval ({} -> {} ms)",
                    dual.a.sampled_at_mono_ms,
                    dual.b.sampled_at_mono_ms,
                    before.sampled_at_mono_ms,
                    after.sampled_at_mono_ms
                ),
            });
            return pass;
        }
        if !dual.stable_identities() {
            pass.failure = Some(LinkProofFailure::Retry {
                detail: format!(
                    "node {node} peer identities changed between the resampled peer samples (connection set replaced)"
                ),
            });
            return pass;
        }
    }
    let listener_ports = peers.listener_ports();
    let dials = peers.dials();
    for (node, dual) in samples_by_node {
        let rows = &dual.b.rows;
        let Some(planned_listen) = peers.listen.get(node) else {
            continue;
        };
        let planned_connect = peers.connect.get(node);
        // Inbound acceptors cannot exceed the planned dialer count.
        let inbound_count = rows.iter().filter(|row| row.inbound).count();
        let planned_dialers = peers.dialers_of(planned_listen);
        if inbound_count > planned_dialers.len() {
            pass.failure = Some(LinkProofFailure::Refuse {
                detail: format!(
                    "node {node} accepted {inbound_count} peers but only {} are planned",
                    planned_dialers.len()
                ),
            });
            return pass;
        }
        for row in rows {
            if row.inbound {
                // Candidate dials: the acceptor is this node and the
                // row's addrbind is the dial's declared target.
                let candidates: Vec<&PlannedDial> = dials
                    .iter()
                    .filter(|dial| dial.acceptor == *node && dial.target == row.addrbind)
                    .collect();
                if candidates.is_empty() {
                    pass.failure = Some(LinkProofFailure::Refuse {
                        detail: format!(
                            "node {node} accepted peer {} on unplanned endpoint {} (planned {planned_listen})",
                            row.addr, row.addrbind
                        ),
                    });
                    return pass;
                }
                let mut best: Option<LinkProofFailure> = None;
                let mut proven = false;
                for dial in candidates {
                    match decide_directed_dial(
                        dial,
                        identities,
                        dual,
                        row,
                        before,
                        after,
                        &listener_ports,
                    ) {
                        Ok(proof) => {
                            pass.covered_dials
                                .insert((dial.dialer.clone(), dial.acceptor.clone()));
                            pass.proofs.push(proof);
                            proven = true;
                            break;
                        }
                        Err(failure) => {
                            if best.is_none() {
                                best = Some(failure);
                            }
                        }
                    }
                }
                if !proven {
                    if let Some(failure) = best {
                        let wrapped = match failure {
                            LinkProofFailure::Refuse { detail } => LinkProofFailure::Refuse {
                                detail: format!(
                                    "node {node} inbound peer {} is not a planned dialer's connection: {detail}",
                                    row.addr
                                ),
                            },
                            LinkProofFailure::Retry { detail } => LinkProofFailure::Retry {
                                detail: format!(
                                    "node {node} inbound peer {} not yet a proven connection: {detail}",
                                    row.addr
                                ),
                            },
                        };
                        let refused = matches!(wrapped, LinkProofFailure::Refuse { .. });
                        pass.failure = Some(wrapped);
                        if refused {
                            return pass;
                        }
                    }
                }
            } else {
                // Outbound row: accounted for plan conformance only; it
                // never proves a dial by itself (the pinned node's IBD
                // dials are outside PeerHub, and a follow-outbound row
                // is not this evidence form).
                let Some(target) = planned_connect else {
                    pass.failure = Some(LinkProofFailure::Refuse {
                        detail: format!("node {node} reported an outbound peer but plans no dial"),
                    });
                    return pass;
                };
                if row.addr != *target {
                    pass.failure = Some(LinkProofFailure::Refuse {
                        detail: format!(
                            "node {node} peers outside the plan: {} (planned {target})",
                            row.addr
                        ),
                    });
                    return pass;
                }
            }
        }
    }
    // Every planned directed dial stays an independent obligation.
    if pass.failure.is_none() {
        for dial in &dials {
            if !pass
                .covered_dials
                .contains(&(dial.dialer.clone(), dial.acceptor.clone()))
            {
                pass.failure = Some(LinkProofFailure::Retry {
                    detail: format!(
                        "directed dial {} -> {} is not yet proven",
                        dial.dialer, dial.acceptor
                    ),
                });
                break;
            }
        }
    }
    for (dialer, acceptor) in &pass.covered_dials {
        pass.covered_edges.insert(sorted_pair(dialer, acceptor));
    }
    pass
}

#[cfg(test)]
mod tests;
