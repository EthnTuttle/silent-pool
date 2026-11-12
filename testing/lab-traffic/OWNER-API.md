# Concrete producer inputs needed by the independent adapters

These are semantic requirements for the assigned production owners, not an
invented accepted producer schema. Send the actual record path, version,
field types, source rows and example raw record through the parent; the
consumer will parse that actual format. No fixture name substitution counts
as runtime evidence.

## T1 acceptance-evidence owner

**QA v2 contract consumed:** `/tmp/opencode/traffic-runtime-acceptance-qa-v2.json`,
document SHA256 `6622a2d66266bf1bff7c3ab059d7df4965baee1f003080edc5f1106e20442c84`.
`traffic_audit.py` now parses that actual `traffic_audit/1` schema and the
controller drives its operator hook API. Full send params need not be copied
into producer records: the complete SignedTx gives an independent preimage for
both `payload_sha256` and `context.raw_sha256`. Known success/error outcomes
likewise have independently reconstructible digest preimages. AddConnection
bytes are rebuilt from the actual recorded scenario and rendered public pool
fields; no duplicate producer edits are requested.

Two original QA-v2 gaps, now resolved by the tested follow-up below:

1. `traffic/runtime.rs:admitted` rejected `traffic_acceptance` in provisioned
   and bootstrapping, although `engine.rs:submit_as` gives this command its
   intentional mutex bypass. The strong retained-coinbase test must arm the
   first bootstrap-distribution hold before signing, while the tracked native
   operation is explicitly paused below its funding height. Its actual
   actor command ID is `bootstrap/<sha256(bootstrap_command_id UTF-8)>/00000000`
   (`actor/bootstrap_control.rs`). Arming late preserves the 120-second hook
   deadline instead of consuming it during the initial hundred blocks; the
   native operation's original deadline and hash debit are not reset.
   Ordinary user payments spend distribution
   outputs and cannot substitute for a retained-coinbase test. Admit the
   explicit operator hook in those stages, or provide the named equivalent.
2. `send_outcome.rs:REVIEWED_PROFILES` was empty. Full definitive-rejection
   acceptance needs the reviewed `source_sha256`, `binary_sha256`,
   `consensus_fingerprint` tuple enabled and supplied in each applicable
   actor's `node_profile_binding`, plus an actual transaction eliciting one
   frozen pre-publication error. The consumer now verifies the exact outcome
   hash and durable core Rejected code when supplied; it does not turn a
   disabled catalog or an unlisted numeric error into success.

The following semantic checklist is retained for completeness; items already
represented by the QA-v2 records are consumed through that format, not a new
wire schema:

Need one complete, durable, private observation stream (or explicitly joined
streams with contiguous sequences), containing:

- Header binding: run id, schema version, controller/driver incarnation,
  Linux boot id, actual source/config/binary fingerprints; complete sequence
  interval and closed/poisoned/truncated status.
- Forward admission/attempt: durable original command id, actor incarnation,
  wire request id, journal send sequence, node id, exact method/params or the
  raw transaction plus its byte digest, gate decision, and monotonic time.
  The forward-attempt record must precede the actual RPC call. A missing
  outcome after a recorded attempt remains unknown; no inferred rejection.
- Forward outcome: correlation to that exact attempt, actual raw response
  or bounded structured node error, versus transport/protocol unknown.
  For the post-response-loss seam retain the **real** received node response
  separately from the deliberately lost downstream delivery.
- Pause/resume: original command id and exact committed journal sequence,
  last-used send sequence (zero before any sends), forwarding gate drained
  boundary, ack outcome and monotonic clock. This joins the raw `SendBarrier`
  through the next `SendResumed` without treating a filtered SSE gap as time.
- Miner activation: first actual `AddConnection` send and correlated result,
  miner/connection/worker ids, monotonically ordered against bootstrap
  distribution confirmation and `traffic_measured_from`. Include the actual
  control payload bytes, not only a summary claiming activation.
- Hold-after-sign seam: configured default-off authorization, original
  command id, txid/raw-byte binding, selected reserved input set, committed
  SignedTx sequence, held boundary, resume/release request, real send-admission
  result and resulting forward-attempt record or refusal. It must be possible
  to invalidate down below retained-coinbase maturity while the signed payment
  is held, observe refusal without a send, then restore maturity and send the
  original bytes. No synthetic Accepted or Confirmed transition.
- Stop/cancel: the bootstrap command's terminal outcome plus actual owned
  PID/start-token identities and completed cleanup. Existing command-log
  `hash_start/hash_reserve/hash_reconcile` remains hash authority.

Need the exact operator command payloads/response shape and private
configuration switch for both seams, including source-bound definitive error
catalog activation. The existing public payment command and the old T1
control seam cannot provide these interposition observations.

## Ladder/runtime proof owner

`ladder-entry-v2` is now declared in your actual WIRE-FORMATS 3.1 and
2026-11-09 decision. The consumer needs the following actual source records:

- Raw `spc:blocks` `d` bytes (`SPCB`/2), producer/front id, Redis stream id,
  publication sequence/time and retained completeness boundaries. Include
  both the event's coinbase fingerprint and its SHA256
  `SPc/DistributionSnapshotV2 || complete_event` SQL snapshot key.
- Full JSON/SQL entry values including `format_version:2`, `p_x` byte arrays
  in k order, `owed_sats`, `sats`, rail and destination bytes; no scalar-k0
  compatibility branch. Distinguish SQL snapshot key from coinbase fingerprint.
- The immutable window actually visible when this job was built: exact
  high-water vector, work profile/reference fingerprint, Q, ordered sealed
  slice ids, source stream ids and full share blobs/roots. No reconstruction
  from a later unbounded SQL tail that the front had not observed.
- Per-share source attribution and issued job linkage: front/channel/miner/
  identity, job id+generation, exact header fields, full coinbase prefix +
  extranonce + suffix, deepest-first Merkle path, issued target/credited work,
  raw selected job transactions. Raw prevout transactions can be supplied by
  the independently captured node-chain index when available.
- Complete prior pending and provisional-paid ancestry as of that publication:
  ordered actual ledger revisions/rows, provisional found-block debits and
  orphan reversals, registration/destination propagation state. State which
  revision boundary is complete; an omitted prior row is not a zero balance.
- Witness-plan publication: full txid/wtxid list or raw transactions, reserved
  value and commitment output, job id+generation and prior/superseded job id.
  Join actual job bytes and stale disposition metadata to the ordinary native
  miner path; the oracle recomputes commitment/weight/revenue itself.

The read-only SQL collector now captures explicit source-owned columns from
`block_found`, `distribution_snapshot`, `slice`, `slice_credit`,
`ledger_history`, `pending_balance`, `ark_delivery`, `ark_payout` under one
repeatable-read/read-only snapshot. It records actual server cluster id,
postmaster start epoch, database/role/port/datadir and transaction snapshot.
Please identify additional immutable aggregate/provisional/publication tables
and exact columns needed by the final bundle. The collector will add named
source-owned queries; it does not accept arbitrary SQL from a bundle.

## Current real-data evidence boundary

The independent archive consumer has already checked the supplied real T2
chain and history, 33 artifact seals, four distribution transactions, 80
allocation outputs and 1809 measured/1810 charged hashes. That T2 scenario has
8 network wallets and **2** payment wallets; it is not the four-user final T3
scenario. Its node API logs retain method/params/ok/err but not complete
canonical read response bodies. All six `sendrawtransaction.params` values
are also truncated to 384 characters plus an ellipsis (385 characters total).
Those prefixes cannot verify complete forward bodies. Next combined run must retain per-node raw
canonical read responses and anchored observation times, rather than infer
historical canonicality from a single final archived chain.

The optional public-seed ownership check now verifies all 189 funding scripts
and six change scripts. It found a documentation-only derivation mismatch:
sealed `keys.rs` SHA256 `c67e8e561e9a28905e4af66d8cee2a2a185f7c61a8e2e5a047dfc26cf6071a32`
uses `m/chain'/index'` at lines 324-326, but its line-21 header comment omits
the second apostrophe. The oracle binds both hardened levels explicitly to
that source hash; it does not search alternative paths or modify production.

## Consumed follow-up handoffs (2026-11-10)

The tested before-sign/early-hook producer handoff is
`/tmp/opencode/traffic-runtime-followup-final-handoff.json`. Its API document
SHA256 is `a7c8db6c9760d9380a0fff955d47625a8b8f569434cb0cb7f14d9fad55af494b`.
The consumer invokes exactly `acceptance_version_rejection_v1` with user,
destination_address, amount_sats=100000 and fee_rate_sats_vb=3. It checks the
immutable construction effect, final v3 signatures, same-ID no-resend,
changed-semantics conflict and retained prior Unknown. Exact rejection is
restricted to the reviewed f159 source / bba release / 4634 profile tuple.
The debug T2 node and post-sign version mutation cannot satisfy this gate.
The handoff's CLI/txd binaries omit the native installer; the actual joined
bundle is still required for execution.

`/tmp/opencode/ladder-completion-owner-api-r1.json` and the r2 source binding
`415d21fa88ad2124712cc1fe9a8400bc7fbdd951bd005f223f92f879068f2602`
are consumed by `pool_source.py` and the explicit SQL query v2. The collector
now includes all four additional named tables, snapshot/WAL/event watermark,
and actual migration-0008 row replay with transaction visibility and tombstones.
No contiguous SQL sequence-ID assumption is made. The archived r1 provisional
placeholder stays UNKNOWN. New RULEB records require a separately frozen
producer contract and complete chain/economic watermark before distribution.

AP5 can consume `canonical_capture.validate_export` with the externally sealed
export hash, expected run/binaries/source/scenario/policy and complete node IDs.
The controller emits `private/chain-evidence.json` and private `rpc-bodies/`
blobs; move them only with an explicitly rebound collector location and seal.
These observations carry no funding, send, hash, credit or Ark-close authority.
