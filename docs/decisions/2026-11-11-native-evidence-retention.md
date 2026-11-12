# 2026-11-11 - Retain actual public native approval evidence

## Decision

Full lab audited native CPU profiles require durable retention of the exact
CandidateRequest JSON sent and Approved JSON received before emitting
SourceAuditCommitted/JobActivated or publishing work. The versioned public
retention envelope is `spc.native-evidence/1`; the capability is
`native_public_evidence_durable_v1`. The frozen native wallet wire/signing
protocol and source-feed/2 preimages are unchanged.

## Rationale

QA's native observer requires actual outer request/response pairs keyed by
request digest. Event digests cannot reconstruct the wallet signature or full
VerifiedSourceEvidence. Retention therefore belongs in the source that owns
the actual received bytes, not in a consumer success-flag adapter.

## Lifetime, privacy and storage

Only already-public CPU material is retained. The existing controller derives
the private startup-only capture directory; no per-request path/write API is
added. Process/config/executable/run-token hashes join source and binary
provenance through the existing run manifest. Capture is optional in non-lab
MinerConfig; full lab audited AddConnection requires an actual capability
snapshot matching its private durable manifest.

Private owner-only files/directories, anchored NOFOLLOW opens, exclusive
staging, NOREPLACE publication, fsync, readback and content hashes bound each
record. Record/byte/process-rotation budgets fail closed. No retained or failed
staging evidence is automatically deleted. Conflicting or missing replay data
does not become an inferred success.

Retention runs in the pending approval path, with one blocking I/O slot per
miner. Original monotonic admission and verification proof-expiry bounds
remain binding. Cancellation can leave an exchange record, always labelled
not_asserted; the event join alone establishes activation. Storage failure,
deadline or supersession grants no hash authority. N1 independent revocation
and N2 clock handling remain required.

## Edited sections and verification

SPEC §3.6's before-mining requirement is implemented with source-owned evidence
retention for full lab profiles. `docs/NATIVE-EVIDENCE.md` defines the exact
schema, files, hashes, limits and reader API. Tests cover durable-before-publish,
conflict/missing/altered/cross-request records, permissions and traversal,
storage budgets, blocked retention cancellation and deadline expiry.
Source-only work and signed component vectors are not live campaign evidence.
