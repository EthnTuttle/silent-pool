# Durable public native evidence v1

Public DTO/reader implementation: `crates/spc-scan/src/native_evidence.rs`.
This envelope does not change `native_activation.rs`, its wire version, or any
signing preimage. Capability: `native_public_evidence_durable_v1`.

## Startup and discovery

Optional non-lab MinerConfig field `native_evidence` has these required fields:
`schema="spc.native-evidence-config/1"`, `run_id`, `directory`, `run_manifest`,
`max_records`, `max_bytes`, `max_sessions`. If present, all audited connections
require durable retention. Full lab audited profiles MUST render this field
and require the capability before AddConnection. There is no capture-path RPC.

The controller derives `directory=<run>/private/native-evidence/<miner_id>`
and `run_manifest=<run>/run-manifest.json`. The directory is owner-only 0700.
Each process creates a new `process-<pid>-<start_time_ticks>` subdirectory,
containing immutable 0600 `manifest.json` and `<request_digest_hex>.json`
records. PID/start/boot/executable/config/run-token hashes bind the manifest
to the existing run manifest's actual process/source/binary provenance; source
identity is resolved through that run manifest, never asserted by a response.
No automatic deletion occurs. Restart uses a new process directory. Existing
process directories are never reopened for append. A bounded root session
count prevents unlimited restart rotation.

Recommended lab bounds: 16384 attempted records and 268435456 bytes per process,
32 process directories per miner. Failed staging writes consume budget too.
Maximum accepted configuration: 65536 records, 16 GiB, 256 sessions. Exceeding
a limit fails closed. There is one blocking retention operation per miner;
waiting approval tasks remain subject to their original admission deadlines.

## Encoding and exact bytes

Manifest schema is `spc.native-evidence-manifest/1`, kind `capture_session`.
Record schema is `spc.native-evidence/1`, kind `approved_exchange`, version 1.
Every record has `activation="not_asserted"`: a cancelled or superseded record
is not an activation claim. Only the actual committed/activated/first-hash
events establish whether it was used.

`request_json_hex` and `response_json_hex` contain exact UTF8 JSON payload bytes
actually sent/received on the wallet connection, encoded as lowercase hex.
They exclude the U32LE transport length. The corresponding complete frame is
U32LE(payload byte length) followed by those exact bytes. Request payload is
at most 1048576 bytes, response at most 16384. No response is reconstructed
from an outcome or event. The record is compact UTF8 JSON; its maximum size is
2138112 bytes (two hex-expanded frame bounds plus 8192 bytes of metadata).
Consumers hash the stored file bytes, not a JSON reserialization.

Record fields are defined by the closed public `Record` struct. They include
capture ID, one-based receipt sequence, manifest SHA256, actual request signing
digest, SHA256 of both payload byte strings, signed checkpoint digest, actual
verification wall time, and `Binding` copied from the request. Binding includes
run/miner/connection, session/candidate epochs, wallet instance, peer generation,
complete connection-config digest and nonce/attempt. The envelope is not a new
signature: those fields are checked against the decoded signed request/receipt.

`DurableHandle` schema is `spc.native-evidence-handle/1`. It contains capture ID,
receipt sequence, root-relative manifest/record paths and their SHA256 hashes,
request signing digest and capability. SourceAuditCommitted carries this handle
when capture is configured. The miner snapshot advertises the immutable capture
manifest locator under `native_evidence`, with schema
`spc.native-evidence-locator/1`, capability, directory, manifest_relative_path
and manifest_sha256. Both paths are confined to the actual process directory.

## Reader and admission

`decode_record(record_bytes, manifest_bytes, expected_handle, expected_keys)`
checks closed schemas, size/hex/content hashes, paths, sequence, all duplicated
bindings and checkpoint digest. It parses both byte strings through the
existing native codec and calls its existing public verifier at the recorded
verification time. It returns `(Record, CandidateRequest, Response)` for QA's
`outer[request_digest]` map. Keys still come from the controller's independent
binding. The file's stored keys or success flags cannot authorize anything.

The pending approval task verifies the actual response, retains both byte
strings, fsyncs the file and directory, then returns proof plus durable handle.
Publication uses NOFOLLOW, exclusive staging creation and NOREPLACE rename.
An existing request digest must contain the exact same verified request/response
bytes; conflicting content or a missing/altered retained record fails closed.
Replay returns the original sequence/bytes rather than rewriting the record.

The session selector never performs fsync or waits for a worker lock during
retention. Cancellation/epoch changes revoke work independently of storage.
The original N2 deadline and verification-time proof bound remain in force
through retention and completion. Late retained records are harmless evidence
of an exchange, not an activation. Retention failure produces bounded Unknown
and no worker publication.

## Component fixture

`/tmp/opencode/native-evidence-contract-r1/fixture.json` binds a genuinely
Schnorr-signed request/response component vector and the exact manifest/record/
handle byte hashes. Existing independent `native_codec.verify_response` checked
its signatures at 1000 ms. It uses explicit test identities/history and is not
a live capture or a replacement for production evidence. Runtime and Rust
retention tests require their own subsequent build/execution receipts.
