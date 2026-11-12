# Delivered readiness v2: actual parser and source findings (2026-11-11)

The consumer now implements the delivered closed DirectedDialProof,
PeerHandshakeRow, EndpointJoin and NodeNetworkReadinessReport projection in
`readiness_proof.py`. Thirteen targeted tests passed, including exact frozen
producer bytes, the two retained real-regression projections and counterfeit
type/direction/endpoint/owner/operation/time/coverage cases. The synthetic
full report in tests is explicitly labelled; it is not a producer run record.
No parser result confers source admission or first-block propagation proof.

## Actual package binding checked

- Package: `/nix/store/rxgkawg2hd98k22y3x73k83gxyvwzndr-spc-lab-ark-tools-0.1.0`.
- CLI SHA256: `41ca8a6e1c28167821b73c3e854f5a33cec25fefc43b110666696747c03060fa`.
- Deriver: `/nix/store/mny1m6ib78a1n8ca3lgwyx2srv69s186-spc-lab-ark-tools-0.1.0.drv`.
- Actual deriver source: `/nix/store/zlvpj4d3awm8g0hppcna5pdw9lannhc0-source`.
- All nine producer handoff source rows match that actual Nix input.
- `net_readiness.rs`: `66656cbe1d720fe7e43045a112519a8c6ac9c6ae2e710c19dc006edf11da8d73`.
- `engine/traffic_runtime.rs`: `f914f50107fa10aa6f1d70a7f951497924b0f0eea50e026476ea722266c2d67e`.
- Real-regression proof bytes: `1450f54909d6303c7883d414d7ed76cc9690eec92bc211f0d38c6e22a94abc0b`.

The real records correctly retain inbound direction and both reversed owned
socket tuples. They have null command_id and are directed projections, not an
engine-stamped complete report or AP5 bootstrap journal. The report-binding
consumer rejects null operation binding. Their 19 ms reported interval is
useful evidence for that regression, not proof of the generic bounded path.

## Source findings requiring producer resolution

Paths below are relative to the actual Nix source above. These are static
control-flow findings, not fabricated live failure results.

### V2-TIME: bounded interval and successful deadline path are not enforced

`crates/spc-lab-txd/src/net_readiness.rs:1005-1009` copies the bracket times and
computes their difference with saturating_sub; `decide_directed_dial` has no
maximum interval check or rejection of reversed sample time. In
`crates/spc-lab/src/engine/traffic_runtime.rs:1284-1289`, a successful pass is
persisted and returned before checking the deadline. The deadline check at
1302-1315 applies only to the non-ready branch. Thus the claimed bounded,
fresh pre-hash proof is not guaranteed by this source's success path.

Moreover, `capture_bracket` at net_readiness.rs:632 timestamps before reading
all process samples (634-643). The reported after timestamp is the start of
the final capture, not the completion of those reads. The serialized interval
does not enclose the entire observation acquisition.

Minimum correction: define the actual source-bound acquisition/freshness
limit, measure a monotonic interval enclosing completed observations, reject
reversed/expired/overlong samples, and check the original operation deadline
on the successful gate path before hash authority starts. Retain ordering and
test overlong successful samples/deadline expiry without resetting the budget.
The consumer takes explicit bounds from trusted operation/source context;
it does not invent a numeric limit or repair a missing producer check.

### V2-BRACKET: some agreed observations are not bracketed or retained

`ProcessSample`/`BracketSample` at net_readiness.rs:608-625 retain PID/start
ticks, fd sets, TCP rows and boot id in memory, but no actual namespace
identity. The proof namespace strings at 987-988 are copied from declared
identities. The engine refuses declared namespaced nodes at
traffic_runtime.rs:1396-1404, then installs namespace=None at 1461-1468.
There is no observed host namespace identity check in the bracket capture.
Supervisor verify at supervisor.rs:519-563 checks process/start/token/exe,
not network namespace identity.

RPC/P2P LISTEN ownership is checked before bracket capture at
traffic_runtime.rs:1429-1458, not after it. One getpeerinfo acquisition at
1495-1537 is used between two kernel samples. Peer id/endpoints are not
resampled for the claimed across-bracket peer stability. Serialized proof
retains only the selected endpoint joins and snapshot_confirmed/start_verified
booleans; the underlying before/after samples are not persisted. A consumer
can check the reported tuple/projection but cannot independently recover the
unretained listener, namespace and peer-bracket observations.

Minimum correction: satisfy the agreed listener/process/namespace and peer
identity bracketing with actual observations and retained source-bound proof.
Keep the producer responsible for any proof-record/version additions; do not
have the consumer guess fields, infer namespace ownership from UID or replace
absent samples with success flags. The same applies to complete accounting
of observed connections. The strict parser remains scoped to the DTO actually
delivered; it does not purport to validate absent raw evidence.

## Execution blockers and full-scope boundary

The completed package directory supplies a Nix output and prose test receipts,
but no canonical AP5 bundle.json with its complete joined source/validation
bindings was supplied there. That descriptor and actual completed gate logs
remain necessary for canonical admission; no old 2484/4d/g105 bundle is reused.
The parent has been asked for the producer completion contract and to resolve
the source findings before the next fresh diagnostic.

The four inspected R4 control files do match their approved hashes in this
package. That is not the complete requested source union: the completion
receipt explicitly queues Ladder/R4 semantic integration and native/wallet
integration, and its installed CPU is still
`0712d0f95194201ffc7e2af1b908bfadde3013e447a271a794e9ebd3a1d31d54`.
The full union requires actual corrected CPU/source and controller/independent
history evidence, not unrelated attached policies. Full funded stock-Ark
lifecycle remains unexecuted by this work.

Evidence: `/tmp/opencode/AP5-readiness-v2-consumer-r1/` contains the targeted
test log/receipt and actual package/source verification. No Rust/Nix build,
live node RPC, hashing, new process campaign, garbage collection, producer
edit or failed-archive reclassification occurred. Successful reap and compiled
verifier artifacts remain preserved; global cleanup is still inconclusive.
