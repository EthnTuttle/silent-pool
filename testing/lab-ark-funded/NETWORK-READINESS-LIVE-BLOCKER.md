# Actual qualified-350 diagnostic: pre-hash peer-readiness blocker

Run: `/tmp/opencode/AP5-network-native-live-r1`.
Evidence/replay handoff: `/tmp/opencode/AP5-network-native-outcome-r1`.

Canonical admission passed at `/tmp/opencode/AP5-qualified-350-check-r1`.
The 52-file frozen harness at `/tmp/opencode/AP5-qualified-350-r1/harness`
then executed this actual command, exiting 1:

```sh
python3.14 -B ap5.py run --through confirmed-funding \
  --bundle /tmp/opencode/network-reap-diagnostic-r1/bundle.json \
  --out /tmp/opencode/AP5-network-native-live-r1 --timeout-secs 7200
```

Bundle SHA256: `2484b1e83a1152e91ecdcd796b4b46f560ecaadd8baf44606bdb5d1772f00092`.
CLI SHA256: `ddd7dd270716bfd24006037d758bf6ab91def7db6de33be74556d04b22a40dbf`.
Actual immutable application source:
`/nix/store/2jjpwirz3igg8ij8qplp7h30hg3h255j-source`.

## Observed failure

`run/private/commands.jsonl` has exactly one command begin and finish:
original `ap5-native-bootstrap`, actor `runtime-phase`, principal `controller`,
route txd-0/node-a. Begin seq 16 at 1794351820112; finish seq 2082 at
1794351910383 (about 90 seconds), `rejected_precondition`, detail:

`node_network_not_ready: node node-b has not dialed its planned peer yet`

The retained readiness report and evidence event 342 agree: planned undirected
edge A-B, covered edges empty, both owned RPC/P2P listeners verified, outbound
lists empty, A inbound `127.0.0.1:33620`, B inbound `127.0.0.1:47756`.
Manifest argv actually connects A (listen 44919) to B (35167), and B to A.
This is not the prior missing-connect-argv failure.

Topology evidence event 343 independently records one peer on each node and
both at genesis, height 0/hash
`0f9188f13cb7b2c71f2a335e3a4fc328bf5beb436012afca590b1a11466e2206`.
The controller-health 401 is also retained; the command's explicit terminal
cause is peer readiness. Node logs report IBD peers ready and repeated
genesis/empty-path stalls. A TCP/peer count does not prove block propagation.

In the immutable application source, `crates/spc-lab/src/engine/traffic_runtime.rs`
lines 1427-1447 classify `getpeerinfo.inbound` and require each planned outbound;
lines 1460-1465 cover an edge only from that outbound observation. Lines
1291-1303 persist and reject at the readiness deadline. This happens before
the hash-budget start. The command journal has no HashStart/reservation rows,
no payment command and no bootstrap native record. No first block, maturity,
96 allocations or any of the four external transfers was reached.

Pinned node source `/nix/store/hzhwz6j2d31y2bm4iz1awyr5s6xzasg9-source`
provides useful producer investigation context: RPC `methods/net.rs:47-54`
reports the PeerHub snapshot; node `run.rs:815-850` awaits IBD before refreshing
follow targets, and notes IBD has its own peers. These source facts and the
logs suggest a genesis/IBD observation mismatch, but the exact underlying
transport/registration defect is not proven by the retained report alone.

## Minimum producer correction and regression

Reconcile the pre-hash readiness contract with the pinned node's actual
genesis/IBD peer behavior, retaining ownership and exact planned-endpoint
proof. Establish a supported launch/readiness path for these unchanged node
and server binaries. An arbitrary nonzero peer count, relabelled inbound
address or skipped gate is insufficient. If evidence needs a new runtime
observer, bind its source/tests/Nix artifact and retain its observations.

Exercise the actual two fresh nodes with the exact launch profile: owned graph
readiness before the sole original bootstrap HashStart, then authority-only
first-block submission and actual same block on B without manual relay.
The nine FixtureNodeControl/rendered-argv cases do not test this behavior.
An external optional regression was not used to credit this run. No node or
server modification, substitute chain, generate call or extra hasher is
authorized by this handoff. No owner-harness edit can legitimately manufacture
the missing compiled-runtime readiness evidence.

## Cleanup and independent replay

The corrected post-stop reap ran once and exited 0. Captured stdout lists 23
exited manifest actors; stderr is empty and both full byte strings and hashes
are retained in `000008-tool-exit.json`. No retry was needed.
The sealed cleanup inventory records all 66 tracked process identities gone
or reused, all 608 recorded listener entries released, no remaining file
references or locks, and no recorded-owned survivor. These are separate facts
from global exhaustiveness: unreadable PIDs 2922, 123843, 130255 and 1286798
remain unclassified and untouched. Global cleanup stays INCONCLUSIVE.

Run seal SHA256:
`6fe9f92fb118c0eaf26e3245db9c225529a4b222705c4bf66fa4de69e064b768`.
The frozen harness's `verify_archive.py` exited 0 and reproduced the
inconclusive diagnostic verdict. This is a successful replay of a failed run,
not funding or propagation acceptance. Native audit, Rule B, Ark lifecycle and
whole-lab acceptance remain false. All earlier packets and runs are preserved.

## Subsequent pinned-node source explanation

The completed read-only report
`/tmp/opencode/node-ibd-peer-readiness-source/REPORT.md` establishes why the
outbound RPC prerequisite is cyclic at genesis: IBD outbound sockets live in
private PeerSlots, outside PeerHub, and follow-outbound registration awaits
IBD completion, which explicitly refuses height zero. This corrects the
earlier investigation uncertainty about the source behavior; it does not
establish the missing historical socket-owner joins in this failed run.

The primary is implementing a source-bound completed-inbound-handshake plus
both owned reversed ESTABLISHED-socket observer. Consumer evidence preparation
is in `PEER-READINESS-CONSUMER-REQUIREMENTS.md`. Its actual producer schema is
still required before parser implementation. The 2484 failure stays frozen;
the next execution requires a new tested source-bound bundle.
