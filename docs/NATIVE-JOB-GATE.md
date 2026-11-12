# Private native wallet activation protocol

Schema: `spc.native-job-gate/1`. Shared implementation:
[`native_activation.rs`](../crates/spc-scan/src/native_activation.rs).
This is CPU-to-existing-wallet local transport, not an Ark-server protocol.

Full lab profiles additionally require the source-owned
[durable public evidence capture](NATIVE-EVIDENCE.md) before activation. Its
versioned envelope retains the actual bytes without changing this wire codec.

## Frames and public evidence

Each frame is U32 little-endian length followed by UTF8 JSON. Requests are
bounded to 1 MiB, responses to 16 KiB, with one exchange per connection and a
10-second total deadline. JSON duplicate keys are rejected recursively;
unknown fields and decimal-valued integer fields reject. Canonical signing
preimages are binary, never JSON re-serialization or floating-point values.

CandidateRequest required fields, in transport-name order:

```
schema, run_id, miner_id, connection_id, identity,
wallet_instance: [u8;16], peer_generation: u64,
connection_config_digest: [u8;32], expires_at_unix_ms: u64,
session_epoch: u64, candidate_epoch: u64, nonce: [u8;16],
job: source_feed::ReceivedJob
```

The actual wallet lifecycle generates wallet_instance after binding its
socket. The controller validates and pins it. The CPU supplies a fresh nonce,
never a fabricated wallet instance. That SAME nonce is passed to the existing
source-feed/2 auditor and binds checkpoint, consumed and committed evidence.

Response is tagged `kind`. Approved contains `schema`, capability exactly
`native_source_audit_committed_v1`, and ActivationReceipt. Unknown contains
schema/request_digest/candidate_epoch/nonce plus a closed code:
unavailable, timeout, rejected or superseded. It never includes private detail.

ActivationReceipt:

```
request_digest: [u8;32], candidate_epoch: u64, nonce: [u8;16],
evidence: source_feed::VerifiedSourceEvidence,
wallet_signature: String
```

The exact evidence type is source-feed/2: checkpoint, Consumed with mandatory
WindowFacts, and CommittedEvidence. Window target/actual sum/shortfall are
canonical decimal strings in JSON and U128LE in SourceConsumedV2's digest.
The shared `verify_public_evidence` verifies all source signatures, attempt,
wire digest, correlations, complete history seal, expiry and window facts.
Native verification additionally binds the request and full job ID (actual
BIP34 coinbase height <<32 | actual u32 native SV2 job ID), and checks the
new wallet activation signature. No source-feed/1 fallback is allowed.

## Signing preimages

Request tag: `SPc/NativeJobRequestV1` over:

1. U32LE(1).
2. U32LE-length-prefixed UTF8 run_id, miner_id, connection_id, identity.
3. U64LE session_epoch, U64LE candidate_epoch.
4. wallet_instance16, U64LE peer_generation, connection_config_digest32.
5. U64LE expires_at_unix_ms, nonce16, ReceivedJobV1 digest32.

Wallet activation tag: `SPc/NativeJobActivationV1` over:

1. Request digest32.
2. U32LE-length-prefixed capability and `spc.source-feed/2` strings.
3. Checkpoint signing digest32, pool trace signature64, checkpoint signature64.
4. ConsumedV2 signing digest32, wallet consumed signature64.
5. Committed signing digest32, collector commit signature64.

All signatures are parsed to canonical binary64. The activation digest binds
the request's actual identity/lifecycle/config/epoch/nonce to these receipts.
Effective work expiry is min(request expiry, checkpoint expiry); no request
may have more than 10 seconds remaining. Approved is the authenticated
capability/readiness exchange for THIS candidate, not an unsigned Hello.

## Instance and peer registry

Under actual ark.datadir (owned non-symlink 0700):
`native-audit.sock` is a 0600 Unix socket;
`native-audit-instance.json` is the service's atomic 0600 WalletInstance;
`native-audit-peers.json` is the controller's atomic 0600 PeerRegistry.
The private controller `native-audit-leases.json` projection names the Unix
lease `wallet.<identity_id>.native_audit`. These are never browser-supplied
paths or public approval URLs, and are not TCP listener leases.

ProcessIdentity contains actual PID, UID, start ticks, boot ID, executable
SHA256, run-token SHA256 and config SHA256. WalletInstance additionally binds
run/identity/pool/front, actual config/socket paths and device/inode, instance
nonce and configured collector/pool public keys. PeerBinding binds actual
miner/connection identity, scope, process/config and monotonic generation.
PeerRegistry holds at most 256 unique miner/connection entries.

The service MUST derive observed process facts from SO_PEERCRED and procfs,
not from the request. `PeerRegistry::authorize` only compares typed facts;
it does not turn request claims into kernel observation. The controller
publishes before every AddConnection and refuses missing/torn publication
history. Adoption only validates persisted bindings; it does not reprovision.

## AddConnection and ordering

`connection.source_audit` is REQUIRED. Required mode contains run_id, miner_id,
wallet_socket, wallet_instance, wallet_process, wallet_socket_device/inode,
peer_generation, connection_config_digest, collector_pubkey and pool_pubkey.
The wallet key is parsed from actual connection.identity; no secret or secret
file path appears in CPU configuration. Explicit smoke mode is
`{"mode":"unaudited"}` and emits only an unaudited claim.

Connection digest tag `SPc/NativeConnectionConfigV1` covers the complete
materialized connection config with its digest field zeroed. Binary JSON
tags are null=0, bool=1+U8, unsigned=2+U64LE, negative=3+I64LE, string=4+blob,
array=5+U32LE count+items, object=6+U32LE count+sorted key blobs/values. Floats
reject. Runtime and CPU use the same public encoder.

The dedicated Noise reader keeps running while the candidate awaits its
wallet. Job/prevhash/target/extranonce changes revoke old work and cancel the
old exchange. Only the current matching committed receipt can enqueue
SourceAuditCommitted, then JobActivated, then publish to hash workers.
Workers fence revocation/expiry around each actual SHA256d. AuditFirstHash
records real work afterward. Signed underfilled/full window facts remain
visible for QA; full campaign acceptance requires Q reached independently.

Outbound transport is owned by a separate task with at most 64 queued frames
of at most 1024 bytes, plus one in flight. The session never awaits queue
capacity or socket writes while work is active. Queue overflow tears down the
connection. Submitted events describe accepted transport attempts; unresolved
attempts, including interrupted/queued writes, become transport-unknown on
teardown and are never inferred to be pool acknowledgements.

Stop fences each worker's permit before graceful close. Stop-all signals all
connections before joining any. The 500 ms graceful writer-close budget is
applied only after revocation and pending-audit cancellation. Cancelling a
partially written Noise frame discards that task's socket/cipher state; the
connection is never resumed using the interrupted writer.

The candidate's monotonic admission deadline is sampled before its exchange
starts and is reused by timeout and permit creation. Verification carries its
paired monotonic/wall sample through completion. Active lifetime is capped by
both the original admission deadline and the proof-expiry bounds from those
samples, so a wall rollback cannot extend the lease during handoff. An observed
expiry or clock failure permanently invalidates that permit.
