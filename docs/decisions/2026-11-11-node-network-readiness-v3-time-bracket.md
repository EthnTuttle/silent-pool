# Node network readiness v3: whole-acquisition deadline and raw bracket evidence

Date: 2026-11-11
Status: implementation decision; lab controller only (spc-lab /
spc-lab-txd). SPEC body is unchanged; the changelog fragment is
`docs/changelog/2026-11-11-node-network-readiness-v3-time-bracket.md`.
Blocks: the two source blockers confirmed against the BUILT version-2
package by the AP5 consumer review
(`/tmp/opencode/AP5-readiness-v2-consumer-r1/source/testing/lab-ark-funded/READINESS-V2-SOURCE-FINDINGS.md`,
sha256 60454b0a...): V2-TIME (no whole-acquisition deadline: the v2
gate re-derived a fresh 90 s window each poll, so a slow-but-valid
observation could certify readiness long after the bootstrap
operation's own deadline) and V2-BRACKET (summary booleans without raw
proof: listener ownership was checked once before the bracketed
observation with the result carried as a boolean, no observed
network-namespace identity was captured, and the "declared namespace
or None" gap let a namespaced node's absence of evidence pass as
evidence of absence).

## Decision

### 1. Contract version 3, no silent extension

`READINESS_CONTRACT_VERSION` is 3. The v3 types add mandatory fields
(acquisition record, dual peer samples, raw brackets, observed
namespace and executable identities); version-2 records fail to parse
against them (deny_unknown_fields plus absent mandatory fields), and a
v2 record can never satisfy a v3 consumer. The frozen v2 fixture is
retained in-tree as the canonical rejection input
(`v2_fixture_bytes_no_longer_parse_as_v3`). The version-2 built
package, its tests and its real-run projection are preserved as
failed-to-certify history; nothing is retrofitted.

### 2. V2-TIME: one bounded whole acquisition

One acquisition clock per gate run: a monotonic start before the first
bracket read, and one deadline - the bootstrap operation's own
deadline capped by the engine's source constant (90 s) - that is never
reset by retries or polls. Serialized with every report as
`ReadinessAcquisitionRecord {started_at_mono_ms, deadline_mono_ms,
completed_at_mono_ms, bounds}` with the source-declared
`ReadinessAcquisitionBounds` (max_acquisition_ms from the trusted
engine profile; max_rpc_call_ms 10_000 matching the typed host
transport's internal 3 s connect / 10 s hard call deadline;
max_peer_rows 256; max_tcp_rows 8_192; max_fd_entries 8_192 - numeric
bounds from the trusted source profile, never from an untrusted
proof). Every read of one acquisition - process identity, namespace
identity, executable, listeners, fd tables, TCP tables, BOTH peer
resamples, boot id, host namespace - happens inside that window; every
timestamp is taken AFTER the reads it encloses completed; timestamp
order is checked with checked subtraction (reversed bracket stamps,
and peer-sample stamps outside the bracket interval, refuse - never a
saturating accept); one monotonic clock domain for all stamps. The
deadline is checked on SUCCESS before a ready record is persisted and
before the caller can reserve any hash budget: a structurally valid
observation completed after the deadline is unready (Err keeps
operation began == false: no HashStart, no new operation id, no
timeout reset). Mid-pass expiry between phases cancels the pass as a
retry-class failure. Bounded kernel readers refuse (never truncate)
when a source-declared cardinality is exceeded; per-node getpeerinfo
row counts are bounded the same way, so one acquisition cannot hold
engine locks indefinitely. Cancellation is read-only: no state is
mutated on expiry.

### 3. V2-BRACKET: raw evidence, actual identities, resampled peers

The observed network-namespace identity - `(st_dev, st_ino)` of
`/proc/<pid>/ns/net` via metadata - is captured per designated PID AND
for the host controller (`/proc/self/ns/net`) in both brackets. It
must be an ACTUAL value (None refuses), stable across the brackets,
and equal to the host controller's; a node in another namespace
refuses (this host-network proof form does not apply), closing the v2
"declared namespace / None" gap by persisting what was actually
observed. The observed executable (`/proc/<pid>/exe` readlink; public,
nonsecret) must be readable and stable. P2P AND RPC LISTEN ownership
is proven from the RAW bracket rows (exact-address LISTEN row, socket
inode, fd membership) BEFORE and AFTER the peer samples, with listener
inode stability across the brackets. Peer rows are resampled (sample A
after the before bracket, sample B before the after bracket, each
stamped after its reads); the exact identity tuples (id, addr,
addrbind, inbound, version, subver, services) must be stable across
the acquisition - counters are recorded raw in both samples and may
grow. The minimal RAW before/after samples (fd socket-inode
membership, per-process OWNED TCP rows with state and four-tuples -
namespace-wide tables contain unrelated host sockets and are not
public evidence - namespace identity, exe, boot id, host netns
identity, after-read stamps) are persisted in the public proof
records via `BracketSample::to_raw`. The summary booleans are
retained for quick reading but are never the proof.

### 4. Public records (DTO deltas)

`NodeNetworkReadinessNode` replaces the `rows` field with mandatory
`peer_samples` (the `PeerDualSample`), `listener_evidence`
(`NodeListenerEvidence`: rpc/p2p LISTEN raw state/inode/fd_membership
before and after) and `process_binding` (`NodeProcessBinding`: run_id,
pid, start_time_ticks, executable_path, executable_sha256,
observed_exe, netns_identity, host_netns_identity, boot_id - public
nonsecret bindings; no run tokens).
`NodeNetworkReadinessReport` gains mandatory `acquisition`
(`ReadinessAcquisitionRecord`). `DirectedDialProof` keeps every v2
field name and adds the v3 mandatory fields listed above.

## Verification

- spc-lab-txd lib suite: 112 passed (v3 additions: reversed brackets;
  peer-sample stamps outside the interval; identity changed / id
  absent from sample A; connecting sample-A row; counters grown
  between samples still prove; node netns differs / unreadable; host
  netns unreadable / changed; exe unreadable / changed; acquisition
  bounds serialization; bounded kernel readers refuse over
  cardinality; live bracket capture stamps after reads; v2 fixture
  rejection; canonical v3 fixture frozen byte-exact).
- Engine `readiness_gate_tests`: slow valid success past the deadline
  is unready; valid pass within the deadline is ready (boundary
  inclusive); a missing completed stamp is judged by now
  (end-between-validation-and-start); the source-declared bounds are
  the documented constants.
- The env-gated exact-AP5-profile regression is retargeted to the v3
  observer (dual peer samples, bounded brackets) and runs against the
  pinned node binary at the next build gate.
- Canonical v3 fixture handed to the AP5/QA consumer packet
  (`/tmp/opencode/ibd-readiness-owned-socket-source-r2/`).

No SPEC body section changes: the readiness gate remains lab
controller behavior. Pool payout, SV2, SPc and Ark semantics are
untouched.
