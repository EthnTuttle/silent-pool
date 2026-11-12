# Native r3 consumer and observer

The native outer-codec source blocker is resolved by:

- `/tmp/opencode/native-audit-gate-source-r1/handoff.json`
- `/tmp/opencode/native-job-gate-public-r3/handoff.json`
- `/tmp/opencode/native-job-gate-codec-r3.md`, SHA256
  `e0b72a8ae3243c0708bdd45a22cba16aac7a61ad3932453663dff1f6c72b4da3`
- `crates/spc-scan/src/native_activation.rs`, SHA256
  `39c75d44c8ff16cf1e08b873eca91ad2988f9da1937ef18265e62d537d8abd99`
- Controller helper `engine/native_audit.rs`, SHA256
  `6c887a4504d3b8c0508dc37b2ea3d94b4403761ecafefde9babd421b4529302e`

These are frozen, uncompiled producer sources. Python consumer tests do not
execute or count the producer's Rust tests.

## Public contract

`native_codec.py` consumes `spc.native-job-gate/1` and the unchanged r2 request,
ProcessIdentity, WalletInstance, PeerRegistry and complete ConnectionConfig
shapes. The r3 override passes the native nonce unchanged as the inner feed
attempt. Only `spc.source-feed/2` public evidence is admitted.

The request digest binds run/miner/connection/SP address, session/candidate
epoch, actual wallet instance, peer generation, complete config digest, expiry,
nonce and actual ReceivedJob. Connection hashing uses the actual tagged binary
JSON format, zeroing only its own digest field, with all defaults materialized.

`SPc/NativeJobActivationV1` binds the request digest, capability, feed schema,
checkpoint digest, pool/collector signatures, ConsumedV2 digest/signature and
Committed digest/signature. The existing feed verifier checks its signatures,
attempt, actual job, completeness, expiry and signed WindowFacts. The native
wrapper additionally checks full u64 height/wire-job identity and the wallet
activation signature against the actual SP address's public scan key.

## Actual observer inputs

`native_capture.py` registers before runtime startup. It independently observes
PID/UID/start ticks/boot ID, actual executable, selected run-token hash and
actual `--config` bytes/path. It validates the wallet-owned instance/socket and
controller-owned peer registry/private lease digest. It compares public wallet
upstream identity/endpoint/Noise authority and native-audit keys with the actual
rendered configuration. It does not read a scan-key file.

Native stdout is a **controller pipe**, not the miner stderr log. The observer
checks the live pipe connection to the controller and captures the controller's
`private/evidence.jsonl`. The bound `record_miner_event` projection retains
actual payloads, driver IDs, source cursor/incarnation, dropped count and miner
wall time. Its monotonic time is controller ingestion time; it does not retain
the original miner monotonic timestamp. The observer preserves raw bytes once,
with immutable prefix hashes and lengths, and rejects missing/pruned/gapped
history. A later recovery cannot erase a recorded integrity failure.

Initial AddConnection payloads are reconstructed from the actual leased peer
generation/instance and materialized scenario fields. Their complete hashes
must still match the separate retained traffic intent/result histories after
the actual funding boundary. Direct pool SV2/Noise endpoints, four wallets,
four native miners, declared workers and 40 aggregate H/s remain required.
Rebinding or restarted histories require separately retained historical
registries; current mutable files cannot stand in for them.

`native_observer.py` joins each SourceAuditCommitted to the current JobCandidate,
actual durable collector ACK/bundle, wallet PID/key, full job/generation,
digests, window facts and expiry. JobActivated must follow that commit and
preserve the candidate bytes; AuditFirstHash must follow activation for the
declared worker. Unknown/target change/invalidation/reconnect/new-candidate
events clear admissible work. Full-Q coverage remains a separate mandatory
campaign obligation; valid underfill is recorded without fabricating Q work.

## Remaining evidence requirements

The supplied native events contain **digests**, not complete CandidateRequest
or signed Approved response bytes. The independent outer verifier accepts exact
retained public frames, but no production retention interface for those bytes
has been supplied. The live observer therefore reports the typed missing
component `retained_actual_native_request_and_signed_Approved_bytes` after
checking available event/collector joins. It never reconstructs a native nonce,
request expiry or activation signature from an event timestamp. AP3's actual
retention/lifecycle handoff is still needed for this final join.

AuditFirstHash is emitted *after* the first SHA256d call. If its enqueue crosses
expiry or replacement, the observed event alone cannot prove the hash occurred
after expiry/clear. Such cases are INCONCLUSIVE pending hash-time evidence.
The source's permit checks and revocation fence are source inspection, not a
substitute for an executed producer test or actual run.

Explicit fixture/unaudited profiles, `explicit_unaudited` events and health=true
cannot fulfil full native-audited acceptance. The funded case requires native
source applicability plus actual per-candidate capability, event and funding
bindings. A structured ReadOnly allowance is never funding authority.

## Bundle and AP5 status

The corrected Runtime base diagnostic bundle `4d3778e8...` and CLI `52e45eab...`
are now independently source/test/binary verified; see
[RUNTIME-BOOTSTRAP-CONSUMER.md](RUNTIME-BOOTSTRAP-CONSUMER.md). This remains
pre-native-gate, Ladder/Rule B and control integration. The **native-audited
complete bundle is pending**. Historical g105 `f2c96b...` remains blocked by
its phase-wait bug.

Parent reports the R4 pool-control `d002...`/`f764...` exact-source recheck
APPROVED; it is not yet merged into the complete bundle. AP5's standalone
full-crypto verifier is now compiled/admitted with six unique Cargo/Nix cases;
the first actual funded board and R4 grant/backend acceptance remain required.
The base Runtime's actual host-P2P failure is recorded in
[BASE-NETWORK-BOUNDARY.md](BASE-NETWORK-BOUNDARY.md). These statuses do not change retained-proof r8, warmup
r9, or their original test counts. Whole-lab acceptance remains INCONCLUSIVE.
