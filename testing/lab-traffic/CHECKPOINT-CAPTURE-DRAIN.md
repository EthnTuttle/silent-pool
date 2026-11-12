# Owned checkpoint retention and final drain

Source contract: `/tmp/opencode/ladder-native-capture-r3/handoff.json`, harness
`45178f83c4037a5651bd778b952756997762842c88b15d1bbea36ac1e5cce221`.
Its five source files and two check-log seals were verified. The owned consumer
implements the same boundary in `checkpoint_retention.py`, `SourceCapture`,
`ReplayProvider`, `CollectorRuntime` and `settlement_drain.py`; it does not run
the producer driver/oracle or introduce another wallet/key/verifier.

## Capture and activation

The continuous source watcher starts before the collector's joined feed
listeners. It retains every log byte, including incomplete tails, and enqueues
complete-line notifications for frozen jobs, advertisements, economic changes,
publications and pending applications. Notifications only schedule closed
read-only SQL captures. The complete signed source parser still controls source
admission; an arbitrary unknown event is not silently accepted.

The bounded FIFO records requests, results, overflow and errors in a durable
hash-chained journal. Both original capture-envelope hashes and immutable
canonical observation copies are retained. Multiple observations at one
revision are preserved; caller mutation or rewritten original artifacts reject.

The actual provider path calls:

```
retain_provider_checkpoint(retention, provider, front, key)
```

It calls the existing `capture_snapshot` and retains the exact admitted cache
entry and its original artifact. The returned revision, anchor, front, history
watermark and digest must match. A mismatch raises before SourceBundle or a
prepared collector proof can be released. Existing cache hits keep their
original observations. New current-view reads are retained even when they are
too late for the requested revision, but never masquerade as that revision.

Final coverage enumerates every frozen and advertised job and delegates exact
revision/parent/history validation to the existing QA checkpoint oracle. The
current Store API is not an arbitrary historical-revision lookup. Missing
revisions, missing frozen counterparts, incomplete source, queue overflow or
journal/reader failure produce CERTIFICATION_BLOCKED, not reconstructed rows.

## Termination boundary

After the live audit checks, the funded case requires `economic_final_drain`:

1. Remove/drain the actual declared miners using existing controller commands,
   observe their original process identities exited, and quiesce feed grants.
   No new mining authority is issued. Source/checkpoint readers remain active.
2. Sample the common independently decoded raw chain, original SQL and signed
   source. Every canonical chain row must match raw ancestry. Every locally
   mined pool block needs publication, economic acceptance and consumption.
   Other pools' and native-bootstrap blocks require canonical chain records,
   not fabricated local ledger rows.
3. Require mature ledger rows/publications to drain while shallow canonical
   liabilities remain unbooked. There is no forced booking, liability deletion,
   target change or extra block production to satisfy this check.
4. Require three stable tip/revision/history-watermark/source-head samples.
   Before stopping writers, poll the actual source tail, wait for FIFO barriers,
   require exact frozen/advertised checkpoint coverage and recheck that the
   raw/source/SQL boundary has not changed during coverage replay. Missing
   coverage or an expired drain deadline prevents the normal pool-stop path.
   Stop owned pool writers through PID-bound handles only after complete
   run/process/start/executable/config checks. Source EOF is separately checked;
   a zombie's closed writer is not an assertion of successful final reaping.
5. Drain the source watcher, repeat exact checkpoint coverage, then recheck raw
   tip, source and SQL after EOF. The deadline check precedes the final seal and
   follows the potentially slow coverage replay and final raw/SQL capture.
   Changed bytes, incomplete tails, a changed stable key or deadline failure
   prevents the converged seal. Original final SQL remains in its capture file.
6. Record the final boundary and exact-job coverage. Convergence can be a valid
   scoped observation while missing exact checkpoints still block certification.

Collector/source/checkpoint thread shutdown failure also keeps cleanup
INCONCLUSIVE so the serial campaign cannot proceed as if cleanup passed.
The final drain is a capture/termination gate; it does not replace entitlement,
native activation/revocation/expiry, current float or Ark lifecycle proofs.

## Controller-owned finalization hook

The shared runner now uses `Controller.audit_live()` for live observations,
then the one `source_feed_runtime.final_drain()`, then
`Controller.final_audit_after_drain()` before close/cleanup. The final audit
issues no RPC, SQL capture, drain or stop command. It reparses the exact sealed
EOF sources and raw-chain export, checks original SQL envelopes/checkpoints,
watermarks and retention-journal head, then evaluates final source/full-Q,
payout/Rule B and native-handle/activation evidence. `final_sealed_prefix` is a
mandatory funded gate. Earlier failures remain blocking, and late source or
native events cannot inherit an earlier payout/window pass.

The QA-private boundary schema is `spc-lab-owned-economic-drain/2`: it adds the
run id, sealed raw export, checkpoint-journal head/hash/count, pool-config hashes
and final native prefix. This is independent of the unchanged public native
and source-feed codecs. A scoped economic drain can converge while final
native/delivery evidence is unavailable; that is not a final-prefix pass.

`Controller.source_feed_runtime.final_drain()` is the existing in-process hook
for the one T3 controller that owns this runtime. Call it after AP5 and T3 have
completed their live observations, while pool writers, SQL and readers remain
available. It returns `CONVERGED_COMPLETE_CAPTURE` only after the ordered
coverage/EOF/raw/SQL checks above. Its durable records are
`economic_checkpoint_coverage`, `economic_final_drain_started`,
`economic_final_drain_incomplete` (on failure), and `economic_final_boundary`.
The final boundary file is `Controller.priv / "economic-final-boundary.json"`.

The collector's final drain, final audit, close, and controller cleanup path
share one nonblocking in-process lock. A competing cleanup cannot advance to
`stop_run` or reaping while that drain owns the boundary. Failed attempts are
not restarted; a completed attempt can return an independent copy of its
previous result without performing another stop. Failure cleanup remains
available and does not imply convergence.

This lock does not coordinate a separate AP5 executor process. The campaign
owner must designate this controller as shutdown coordinator and arrange for
AP5 to finish its live observations before this hook, with AP5's independent
teardown deferred until after the boundary. The current optional
`campaign.py --ap5-evidence` interface is archival and grants no live shutdown
coordination. No external producer RPC or proof version is invented here.

## Preserved scopes

The r16 independent partial pass and every r5 failure seal remain immutable.
In particular r5's H23 raw block/publication does not become economic acceptance
because a later harness can capture/drain correctly. The old missing revisions
are not synthesized or copied back into that archive.

Wallet-service handoff `aggregate-wallet-service-tested-r1` records compiled
Ark binary `aee146fb...`, source code `2a022552...`, and joint ordering
commit 5 / activation 6 / first hash 7 with zero earlier hashes. Its startup
metadata and independent-key empty-history collector are fixtures, so this is
not full-controller/history/funded or late-revocation/lease proof.

The parent's same-auditor N1/N2 approval applies to `ccf53fa9` / native CPU
`7b62234f...`. It does not upgrade the old `aee91...` joint fixture. Wire
schemas remain unchanged. The actual merged main lifecycle, complete history,
readiness and funded/native-audited Nix campaign remain pending.
