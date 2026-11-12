# Lab host-run node P2P wiring, bootstrap readiness gate, and cleanup attach

Date: 2026-11-10
Status: implementation decision; lab controller only (spc-lab / spc-lab-txd).
SPEC body is unchanged; the serial frontmatter fragment is in
`docs/changelog/2026-11-10-lab-host-node-p2p-and-reap.md`.
Blocks: `testing/lab-ark-funded/NETWORK-BOOTSTRAP-BLOCKER.md` (host runs
launched isolated nodes that could never peer, so bootstrap funding could
not propagate; and `reap` exit 1 on stopped traffic runs).

## Decision

### 1. Host-run node peering is planned by the engine

A host-mode run (no namespace topology, no published node P2P addresses)
whose nodes were all rendered in the full isolated listener shape
(`--no-listen --no-seeds --no-discover --max-outbound 1`) is peered by the
engine before launch: each node's `--no-listen` is dropped and it gets
`--listen` on its own leased loopback P2P port plus `--connect` to its
scenario-order ring successor. One node stays isolated. A run where no node
carries `--no-listen` is the caller's own wiring and is untouched. Namespace
topology runs (published `node.*.p2p_addr` addresses) keep the existing
published-address wiring; the ring never applies there.

Before any process is launched, every node argv is rendered through the same
launch context the loop uses and validated. A caller-supplied P2P flag
(`--listen`, `--connect`, `--addnode`) or a duplicated engine-managed
isolation flag (`--no-listen`, `--no-seeds`, `--no-discover`,
`--max-outbound`) fails closed with `node_p2p_argv_invalid` and zero node
process, profile or manifest effects (storage roles launch first and are
stopped by the failing start). A run mixing isolated and non-isolated node
shapes fails closed with `node_p2p_plan_invalid`; so do shared leased P2P
ports and multi-host address sets. A rendered flag that is ambiguous
(repeated, or trailing with no value) is read as absent by the flag reader,
so ambiguity can never be mistaken for a planned endpoint.

### 2. Observed-connectivity gate before any hash budget

`begin_traffic_bootstrap` now derives the planned node peer graph from the
recorded launch profiles (a listen without a matching dial, an unknown dial
target, or two nodes sharing a listen endpoint is `node_network_plan_invalid`),
then waits for that graph to be live before `start_hash_budget` is opened.
The gate is bounded (90 s wall clock, 500 ms poll, capped by the operation
deadline) and observes real node state per poll: the recorded process must
be the supervisor-owned one, the host-mode RPC and planned P2P listeners must
be owned by that process (file-descriptor proof; namespaced listeners skip
the proof), `getpeerinfo` must show exactly the planned outbound dial
(an unplanned peer refuses), inbound accepts must not exceed the planned
dialer count, and every planned undirected edge must be covered by at least
one side's report. Transient states (not yet dialed, listener not yet owned,
RPC unavailable) retry until the deadline; structural contradictions refuse
at once.

The gate holds no traffic lock while polling, so an operator stop stays
responsive, and it aborts cooperatively when the run is no longer active. A
gate failure leaves the command unbegun: rejected precondition, zero hash
debits, the run is not stopped. Readiness (ready, refused or unready) is
persisted as evidence - a private `traffic/node-network-readiness.json`
artifact plus a journal payload - before the bootstrap proceeds or rejects.

### 3. Cleanup-purpose attach for `reap`

`RunEngine` attach now has three modes. Live adoption is unchanged. A new
cleanup mode takes the owner lock and the traffic capability gate, validates
only the persisted bindings (each recorded node process's argv and binary
hash still match the recorded launch profile; driver config hashes are
unchanged; a missing node process record is not a failure there), and loads
the recorded process census verbatim. The `reap` command uses this mode: a
stopped traffic run - whose nodes are no longer `Ownership::Owned`, which
the live path's node-binding proof refuses - can be cleaned up, and
expected-but-absent traffic nodes are reported as `Exited` instead of
vanishing from the cleanup report. A persisted contradiction (rewritten
argv, changed binary, changed driver config) still rejects, so cleanup never
trusts rewritten history.

## Verification

- `node_p2p_wiring` (9): two- and three-node host rings over distinct leased
  ports, single-node isolation, namespace published-address wiring
  unchanged, four fail-closed argv/shape cases with zero node effects, and
  the ring argv normalizing as the peered profile shape under the frozen
  launch-profile grammar.
- `reap_cleanup` (2 external) + internal traffic-binding suite (7), including
  the pinned live-vs-cleanup contrast: a stopped traffic node fails the live
  node-binding proof while the persisted-binding validation accepts the same
  on-disk state.
- Existing suites unchanged: spc-lab lib 84, traffic_contract 49, and the
  traffic/restart/adoption/lifecycle/topology/scenario suites; spc-lab-txd
  all green.
- Env-gated real two-node propagation (`SPC_LAB_RBITCOIN_NODE`): two real
  rbitcoin regtest nodes peered in the planned ring shape mine one real-PoW
  block; `submitblock` is sent only to the authority node and both nodes
  observe the same tip hash at height 1, proving the wiring propagates
  blocks exactly as the blocker's minimum correction requires.

No SPEC body section changes: node P2P wiring, the readiness gate and the
cleanup attach are lab controller behavior. Pool payout, SV2, SPc and Ark
semantics are untouched.
