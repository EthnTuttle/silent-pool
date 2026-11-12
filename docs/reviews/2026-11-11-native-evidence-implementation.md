# Native public evidence retention implementation

Status: **source implemented/formatted; UNCOMPILED; no Rust tests executed**.
The primary owns the heavy turn. This followup preserves the approved ccf53fa9
source, 7b62234f CPU seal and 22-test receipts at
`/tmp/opencode/native-lifecycle-tested-r1`. They are not passes for this change.

## Concrete delivery

- `spc_scan::native_evidence` defines the closed versioned manifest, record,
  binding, durable handle and read-only decoder. It uses the unchanged native
  codec/public verifier, not a second cryptographic verifier.
- `spc_cpu_miner::evidence::CaptureStore` retains the exact request payload sent
  and exact Approved payload received. Each has its own SHA256, with the full
  record and immutable process manifest also hashed. The record is keyed by the
  native request signing digest and labels activation as not_asserted.
- Capture is startup-only optional MinerConfig. Main opens the private store
  before accepting commands. Snapshot advertises the actual capture manifest;
  full lab rendering enables it, and AddConnection first obtains a bounded
  normal Snapshot then checks the advertised capability against private files
  and supervised process/config facts. Read-only attach validates the same files.
- Pending approval awaits retention before completing. One semaphore slot per
  miner remains held by the blocking operation even if its async caller is
  cancelled. There is no filesystem sync in the native selector or worker lock.
  Capture cancellation flags, epoch checks and original N2 deadlines prevent
  late writes from becoming hash authority. N1 writer/revocation code remains.
- Anchored NOFOLLOW opens, 0600/0700 owner checks, exclusive staging creation,
  NOREPLACE rename, file/directory fsync and readback protect publication. Record,
  byte and process-directory limits reject exhaustion; no automatic cleanup
  occurs. Missing or conflicting replay bytes fail closed.

The exact schema, paths, frame encoding and bounds are in
`docs/NATIVE-EVIDENCE.md`. QA can use returned `(request,response)` pairs as
`outer[request_digest]`, joined only through actual committed/first-hash events.
The existing run manifest provides source/binary provenance through the bound
process executable/config/run-token hashes; no claimed source hash is invented.

## Signed component contract

The early completed public unit is
`/tmp/opencode/native-evidence-contract-r1/handoff.json`. It includes a genuinely
Schnorr-signed request/Approved component vector, checked with the existing
independent Python native verifier at its recorded time. Its record SHA256 is
`f12ff73e6f24126b12f7812ee2550f1e211f4793b4741c216efc9c70b8e564b3`.
The exact bytes are frozen under `bins/spc-cpu-miner/fixtures/native-evidence`.
The fixture uses explicit synthetic identities/history and is never substituted
for a production record or described as live campaign evidence.

## Authored verification

Seven new tests cover: signed-envelope/hash/request joins and tampering;
symlink/traversal/NOREPLACE/privacy; process rotation without deletion; actual
durable bytes before activation and exact replay/conflict/missing records;
record/byte/permission failure; delayed retention supersession and stop;
and expiry during blocked storage. Delayed-storage tests use a blocking gate
with a guaranteed release-on-panic guard, not a successful fake fsync.

Next own-ticket gate: the new evidence tests and four retention-related lifecycle
cases, then affected native async/config cases, N1/N2 regressions, CPU check/
clippy and a new sealed CPU. Runtime capture/render helpers require focused
checks in the corrected pool/store union; the old baseline dev-dependency
limitation remains separately documented. No unchanged scanner crypto suite
needs a broad rerun merely to inherit a test count.

## Serial integration

The handoff exports the owned delta against ccf53fa9 and an additive runtime
patch against the immutable captured primary engine/runtime bytes in
`/tmp/opencode/native-audit-primary-before`. The latter preserves existing
funding, registration and traced command paths. It includes initial/dynamic
capture-path rendering and readiness/manifest validation. No primary files
were edited. Parent applies the narrow hunks to its current union and records
the actual resulting source/binary rows for QA.

Actual controller-owned live archives and the independent full-history/funded
campaign remain pending until this source is compiled, tested and integrated.
