# TIME correction source recheck (2026-11-12)

Status: source review only; UNCOMPILED, UNTESTED and UNADMITTED. No Rust/Nix,
Python test, RPC, runtime, mining or live-actor action was performed by this review.
The existing coordinator's 23 tests and v3 consumer's 14 tests retain their
original scope; they are not tests of this corrected producer source.

Exact producer engine source SHA256:
`b5da9192848e1176fc87bd89eeda2d08cd4912e0a6860efb5d169ec9c635f72f`.
Source was read from primary and retained independently at
`/tmp/opencode/AP5-time-fix-source-recheck-20261112-r1/producer-source`.
Producer report:
`/tmp/opencode/ibd-readiness-owned-socket-source-r2/TIME-FIX-SOURCE-REPORT-20261112.md`,
SHA256 `16446b7e3f0cdf83920a7629d6de2ff2b247d849d8ba78872a0e98cffdc799d2`.

## Confirmed corrections at the engine boundary

In `crates/spc-lab/src/engine/traffic_runtime.rs`:

- Lines 2658-2680 now reject a completed stamp later than now, a completed
  stamp past the deadline, and a fresh now past the deadline. Some(completed)
  no longer suppresses the now check. The old Some(10000)/now10600/deadline10000
  case is a negative at 2886; reversed stamps are a negative at 2897.
- The readiness gate returns its never-reset acquisition deadline (1258-1270,
  1328-1330). The caller retains it at 2068, rather than discarding the tighter
  expiry in favor of the original operation deadline.
- Lines 2089-2090 read current monotonic time after ready-record persistence
  and the traffic-lock wait, and check both deadlines before began=true and
  the call to start_hash_budget. A refusal at this call site leaves began false.
- The diff against frozen b115d4df changes these call sites/helpers and their
  unit assertions; no v3 serialized DTO change is required or inferred.

These are genuine source corrections to the two previously identified engine
call-site gaps. Executed producer checks and the actual regression remain due.

## Remaining source finding: command-log lock follows the fresh check

The helper is called before `CommandLog::start_hash_budget`. The latter still
acquires `self.state.lock()` at
`crates/spc-lab/src/command/traffic_budget.rs:190`, then writes HashStart at
206-211 without another current-time check. The separately retained file hash is
`f66d75b606220c3ecb3366477240ba4972b2cc110b1fcb20f1feab1864f95167`.

A finite counterexample remains in that control flow: the engine reads now=9400
with acquisition expiry=9500 and operation deadline=10000; its helper accepts.
The command-log mutex is unavailable until 9600; start_hash_budget then writes
the record without judging the new time. The new traffic-lock check does not
cover this subsequent lock wait. This is a static finding, not a claim that a
late HashStart or debit was observed in a live run.

The producer should carry the relevant expiry into the command-log transition
and check a fresh clock after acquiring the write-state lock, before authorizing
the HashStart record. Exercise an actual delayed command-log-lock transition,
not just the scalar helper. Preserve original operation/deadline and honest
rejected/unknown semantics if a record cannot be established. The authored
slow_persist_or_lock_wait test at 2923 supplies scalar timestamps to the helper;
it does not take the real lock or inspect an actual journal write.

## Remaining boundary mismatch: original deadline equality

`hash_start_within_deadline` at 2698 rejects now > operation_deadline, so equality
is accepted. Existing `reserve_hashes` at traffic_budget.rs:233-236 rejects
monotonic_ms >= operation.deadline_monotonic_ms. The current v3 consumer likewise
requires hash_start_ms < operation_deadline_ms (readiness_v3.py check_acquisition).

Thus now=operation_deadline=acquisition_deadline can create a HashStart record
that the consumer rejects and that cannot authorize even its first reservation.
Resolve this original-deadline boundary explicitly in the producer transition
and regression; do not silently relax the consumer. This is distinct from an
inclusive acquisition-boundary check while the original operation window is
still open, which the consumer already permits.

## Evidence, archive integrity and admission disposition

The producer withdrew 112/all-green. Its only supplied affected log remains
111 passed / 1 failed / 1 ignored. The new assertions are authored, not executed;
the report is not a passing receipt. No source qualification map, descriptor,
capability or critical-source pin was changed by this review.

The first read-only r4 census found all 74 listed rows intact and the producer
report as one unlisted addition. During this review the producer relocated it.
The second census verifies all 74 listed rows intact with zero extras. Both
censuses are retained; this review neither moved nor removed anything in r4.
The relocated report was copied only into the new review packet as input.

Canonical bundle source/field inventory, actual semantic union, R4 binding,
Ark-feature receipts, orchestration flags/shared-run provenance and real v3
regression remain evidence obligations. They are not a new approval ceremony
or permanent hold. The native -> Ladder -> primary build schedule is unchanged.
The actual corrected gates and Nix/deriver/source-qualified bundle must arrive
before admission or a fresh live run.
