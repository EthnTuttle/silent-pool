# Node network readiness: receiver-handshake with owned reverse TCP

Date: 2026-11-10
Status: implementation decision; lab controller only (spc-lab /
spc-lab-txd). SPEC body is unchanged; the serial frontmatter fragment is
in `docs/changelog/2026-11-10-node-network-readiness-owned-socket-proof.md`.
Blocks: the readiness-gate half of
`testing/lab-ark-funded/NETWORK-BOOTSTRAP-BLOCKER.md` as diagnosed in the
node-owner report (`/tmp/opencode/node-ibd-peer-readiness-source/REPORT.md`)
and the AP5 consumer requirements: the previous gate required each node's
getpeerinfo `outbound` list to contain its planned peer, but the pinned
node reports getpeerinfo from its PeerHub snapshot only while the IBD
`--connect` dials deliberately stay outside the dialer's PeerHub, so the
gate was cyclic at genesis and rejected a healthy two-node run after 90 s.

## Decision

### 1. The evidence form: `receiver_handshake_with_owned_reverse_tcp`

A new shared proof module `spc-lab-txd/src/net_readiness.rs`
(observational contract version 2) replaces the outbound-report check.
Placing it in spc-lab-txd (not spc-lab) is forced by the dependency
direction: spc-lab depends on spc-lab-txd, so the engine gate and the
env-gated regression exercise one implementation. For every planned
directed dial `D -> R`:

1. R's planned P2P listener is owned at its exact address (address AND
   port, no suffix matching) by R's supervisor-recorded process, via the
   per-PID `/proc/PID/net/tcp{,6}` view intersected with R's fd-inode
   set. The bound RPC control listener is proven the same way before any
   peer is sampled.
2. R reports a completed inbound getpeerinfo row (strict typed parse; no
   absent-field inference) with `addrbind` equal to R's planned listener
   endpoint, `addr` equal to D's ephemeral endpoint, `inbound` exactly
   true, protocol version 70016, subver `/rbitcoin:0.8.99/`, the
   `NETWORK | WITNESS | P2P_V2` service bits, and positive received
   VERSION/VERACK accounting markers (the pinned node accounts a 24-byte
   header per received message, so present markers are always positive).
3. Both D and R own ESTABLISHED kernel sockets with the exact reversed
   four-tuples (the two inodes normally differ; tuple reversal is the
   match), in both samples of a bracket that surrounds the RPC sample.
   The row's peer id, endpoints, states and fd ownership must be
   identical across the bracket; the boot id and both processes'
   `/proc/<pid>/stat` start ticks (PID-reuse guard) must be unchanged.
4. The dialer's ephemeral port must not be a planned listener port, and
   endpoints are literal IPv4 only; anything else (IPv6, hostname, NAT)
   fails closed explicitly rather than being approximated.

The RPC sample is bracketed with fresh identity/kernel samples; ready
means all planned directed dials carry complete proofs. Undirected edge
coverage is only derived from proven dials, never claimed from bare peer
arrays, port presence or a relabelled inbound direction. An unplanned
peer or endpoint refuses; duplicate inbound rows beyond the planned
dialer count refuse; every observed P2P row is accounted (outbound rows
are checked for plan conformance only - they never prove a dial).
Transient states (handshake incomplete, socket not yet ESTABLISHED,
listener not yet bound, RPC unavailable, socket replaced between
brackets) retry within the original deadline; the deadline is never
reset and no new command id is minted. Structural contradictions
(foreign process ownership, PID reuse, unplanned endpoint, unreadable
required table, boot-id change) refuse at once. Namespaced nodes fail
closed explicitly - the host /proc view cannot prove namespace-internal
ownership, so the gate refuses rather than skipping the proof.

Source-bound constants (protocol version, subver, service bits, the
`{:016x}` services encoding, the `acct_bytes` positivity argument, and
the addr/addrbind field semantics) were extracted from the pinned node
source (UPSTREAM_DEPS.md pins), not from observed output alone.

### 2. Engine gate and public evidence

`begin_traffic_bootstrap` keeps its shape: bounded wait (90 s, 500 ms
poll, capped by the operation deadline), no traffic lock held,
cooperative abort, gate failure = rejected precondition with zero hash
debits, readiness persisted before the bootstrap proceeds or rejects.
The readiness condition is now `covered_dials == planned_dials` with no
failure. The persisted report (`traffic/node-network-readiness.json`
plus the journal payload) is versioned: contract_version,
evidence_form, planned/covered directed dials, per-node raw typed
handshake rows, and the complete public `DirectedDialProof` records
(PIDs, start ticks, both endpoint joins with inodes and fd ownership,
bracket timestamps and interval, boot id, operation binding). Proofs
carry no secret token or environment bytes. Old version-1 reports still
deserialize (serde defaults) but never satisfy a version-2 consumer.
`PlannedNodeNetwork.dialers` is removed; dialer counts are recomputed by
the proof layer (`PlannedPeers::dialers_of`).

### 3. Regression

A new env-gated real two-node regression
(`SPC_LAB_RBITCOIN_NODE`) launches the exact AP5 diagnostic argv shape
(leased loopback ports, `--rpc-cookie-file`, `--regtest-pow-bits
0x200fffff`, `--api-log`, `--rpc-compat-profile bark-v31`, ring
`--listen`/`--connect`) and requires the strict shared-observer gate to
pass at genesis - both directed dials proven, no hash reservation before
the gate - then mines with an authority-only `submitblock` and asserts
both nodes hold the same block hash at height 1. The existing
propagation test (which only waited for nonempty peer arrays) is
retargeted through the same parameterized helpers; it does not exercise
the gate.

## Verification

- `net_readiness` unit tests: the positive fixtures replicate the exact
  join table of the blocked AP5 run (per-PID fd sets prove the join is
  fd-intersection, not table presence; duplicate fd aliases dedup), plus
  the full negative matrix (connecting row, wrong version/subver/
  services, zero or absent accounting markers, outbound row, wrong
  addrbind, listener port as ephemeral, IPv6 fail-closed, unplanned and
  duplicate peers, unplanned outbound, one-dial-only, foreign-owned
  socket, absent/wrong-port/non-ESTABLISHED socket, single-sided socket,
  inode replacement, PID reuse, unreadable fd/boot/start tables, missing
  identity), /proc parsing, `PlannedPeers` validation, proof
  serde round-trip, and a live hermetic listener-ownership test.
- The env-gated exact-AP5-profile regression above (real two-node run).
- Existing suites: no fixture getpeerinfo flowed through the old gate's
  row parser (only the runtime node_views fixture, which is unchanged),
  so no green test depends on the removed behavior.

No SPEC body section changes: the readiness gate is lab controller
behavior. Pool payout, SV2, SPc and Ark semantics are untouched.
