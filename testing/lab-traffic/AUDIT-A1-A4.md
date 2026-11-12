# Acceptance checker A1-A4 disposition

Date: 2026-11-10. Scope: independent Python checker fixes only.

The original v3 source, 97-test log and real T2 receipts remain frozen in
`/tmp/opencode/T3-T4b-acceptance-v3-97-frozen.tar.gz` (SHA256
`087689fae1292d6391f1fb0aed511559b2dd89a2d0bbc3c7f1d5cc98ee40224a`).
The two unintegrated timed-capture modules authored before this review are
separately preserved in `/tmp/opencode/T3-audit-fixes-prechange-delta.tar.gz`;
they are not dependencies of these fixes.

## A1 - every actual outcome is reconciled

`TrafficAudit` now joins every actual send to its exact run/driver,
incarnation, durable intent, send sequence and local journal-revision interval.
The first local response must occur after the forwarding context and before
the next intent for that payment. A missing result or local response is
inconclusive. Unknown, suppressed and successful actual responses cannot be
turned into a definitive rejection. Listed node rejections require their exact
typed response digest, code and reviewed source/binary/profile binding.

The journal no longer proves its own NoEffect classification. Any Rejected
record requires a cross-stream proof tied to a hash of that exact history
prefix. Actual prior uncertainty/acceptance or reorg resubmission keeps the
reservation even when a later *fresh* attempt is definitively rejected.
Later canonical confirmation and reorg remain valid independent observations.
Tests use a signed raw transaction and a full 100-block-mature offline funding
chain, not malformed transactions that happen to fail another check.

## A2 - one intent authorizes at most one actual forward

Uniqueness keys are `(run_id, driver_id, journal_intent_seq)` and
`(run_id, driver_id, send_sequence)`. Incarnation is deliberately excluded:
reopening an actor cannot redeem an old authorization. A legitimate retry
needs its own durable intent and remains subject to the journal retry cap.

Node effect counts are exact, keyed by node and complete raw bytes. Extra
observed sends fail rather than satisfying a greater-than comparison.
Witness-fixture sends have separate, uniquely referenced intent/RPC/completion
records, a fixed test actor, actual witness bytes and correlated node result.
An enabled witness flag or an unverified declaration does not excuse extra
node effects. Truncated node bodies remain inconclusive. The exact effect
census is now a required acceptance gate.

## A3 - complete high-water vector before Q selection

All nonzero front marks and their latest slice records are checked before
arithmetic or an early Q stop. Numeric IDs must encode the same front as their
row and watermark. Missing fronts/latest records, reordered rows and mismatched
front IDs cannot disappear behind a full window. Explicit zero marks add no
work. Retained rows below the Q-crossing slice are not forced into the selected
window; selected membership must match the complete published vector.

The existing numeric primitive is explicitly single-front. Numeric IDs are
not a global multi-front publication order, nor an authenticated conversion
of Redis stream IDs. Multi-front or raw Redis-mark inputs are inconclusive
until the actual immutable producer bundle supplies the authenticated mapping
and visibility/order proof. Even a present older unselected-front mark cannot
be silently ignored. The default scenario has one front per independent pool;
this restriction does not substitute a made-up multi-front rule.

## A4 - cleanup is more than a PID count

A nonzero reaper exit is FAILED even with no surviving PIDs. A zero exit must
carry the source-defined exhaustive tuple result, known ownership states,
unchanged process/resource inventory, independent PID/start-token checks and
an applicable resource cleanup receipt. Resource checks cover owned process
groups, private Unix listeners, host listeners and recorded namespace holders.
Status strings cannot conceal recorded survivors. Missing ownership,
manifest/resource evidence, timeouts and exceptions are inconclusive; known
survivors fail. Cleanup failure paths still emit the receipt and close its
private capture. No broader process killing was added.

## Current production acceptance boundary

These fixes do not change the real T2 raw PoW/fees/maturity result. Its old
truncated send logs still cannot become a full T3 pass. No old evidence was
edited, and no node RPC, Rust/Nix build, production edit or commit was performed.

The parent has now provided a source-bound release-node version-rejection
proof and reports the catalog enabled; the older empty-catalog observation is
superseded. The before-sign acceptance-only v3 API is still awaited. The full
suite additionally requires `funded_v3_sighash_all`: an actual v3 transaction
whose final SIGHASH_ALL signatures, mature prevouts, fees and raw bytes are
independently verified. A post-sign version mutation or the node's early
`(-26, "version")` error is not that proof. This new live case remains
unexecuted/inconclusive until its concrete frozen producer contract arrives.

## Same-auditor follow-up: late AcceptedByRpc (2026-11-10)

The parent reports A2-A4 APPROVED and the original unsafe A1 input release
closed. The remaining P2 was reproduced against the preserved 171-test r3
state: actual transport Unknown, journal Unknown, then AcceptedByRpc without
a new send passed the first-outcome-only reconciliation.

The correction scans **every** AcceptedByRpc in each command's durable-intent
interval. Each requires that interval's actual unsuppressed `ok` result,
the exact signed-txid response preimage, matching transaction/node and a local
sequence after the forwarding context. An earlier attempt's success cannot
support a later attempt's acceptance. Orphan acceptance classifications remain
INCONCLUSIVE. Accepted outcome proofs retain intent/attempt/result identity and
the exact history-prefix hash; journal replay with those proofs rejects later
history/node substitution. Standalone journal parsing still describes local
state only, not proof of an actual RPC result.

The actual traffic_audit/1 format permits one result per forwarding attempt.
A recorded actual Unknown therefore cannot be reclassified using an inferred
later success. A local Unknown followed by acceptance remains valid when that
same attempt has a genuine retained, unsuppressed successful result. Fresh
durable-intent retries with their own successful results remain valid too.
Unknown -> Confirmed still requires independent raw canonical transaction
proof and does not require or manufacture RPC acceptance.

Six new regression tests cover the minimal suffix, node rejection, suppressed
success/error, protocol/shape/timeout Unknown, gate closure, wrong node/txid,
borrowed attempt success, orphan acceptance, valid delayed/retry recovery and
proof/history substitution. The existing raw confirmation/reorg positive is
retained. Same-auditor disposition of this correction is pending.
