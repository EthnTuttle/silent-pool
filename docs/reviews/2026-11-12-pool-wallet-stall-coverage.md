# Pool wallet stall coverage correction

Date: 2026-11-12
Status: source prepared; focused default/release verification awaits the
parent-assigned build slot. No new tests or builds have run for this revision.

## Evidence retained, scope corrected

The old `wallet_control.rs` is preserved at
`53f5e78981a590c3b73ffb6b2d9aaaf49f1f73b66ab76bbc3a1514d605a7ba8f`
in the R4 packet and the separate source baseline capture. The investigation
is `/tmp/opencode/pool-wallet-stall-profile-analysis-20261112.md`.

Completed evidence is preserved under
`/tmp/opencode/test-ark-debug-failure-evidence-r1/`:

- Run 4 debug: full raw log hash
  `2cc448c0b25acb28d2327e63db828e67b9d667d488371edda5e056e18311dd42`;
  296 passed, 1 failed at the old `<7s` elapsed assertion. 26.46 seconds is
  the suite duration, not a measured duration of that test.
- Run 3: full log lost through overwrite. Only the quoted same assertion and
  296/1 result with suite duration 39.46 seconds remain. Individual elapsed
  values and phase timings were not recorded for either failed attempt.
- Run 5 release: the pool's 297 tests passed in 5.05 seconds; the frozen
  complete log contains 33 successful result groups and the gate exits zero.
- A separate completed host debug gate at `82c4d7c...` also passed the same
  297-test pool group in 5.21 seconds. Its log is
  `ibd-readiness-v3-bundle-r1/gate/test-pool-ark.log`, hash
  `7a7cfe01690d44aebe6c38c1948b36fbe622642926cca5deed494eeb8518e873`.

The source had no debug-specific deadline/control path. Release alignment is
a valid artifact-profile choice, not proof that debug durable writes caused
the failures. Disk, scheduling and profile effects were not isolated. The
old passing test established only that its particular `step` invocation fit
the wall-clock threshold and that the test subsequently dropped its own
mutex. It did not exercise production wallet-capacity release.

## Bounded implementation change

`Controller::process_key_with_capacity` contains the existing per-key code
from `run`, in the same order:

1. acquire the supplied operation mutex;
2. run `process_key`, including outcome/fault persistence;
3. leave the guard scope;
4. advance the cursor and report the same cursor error;
5. yield.

`run` passes the actual `wallet.client.operation_lock` and its existing
`advance` call to this helper. The async cursor callback runs that synchronous
write immediately in production; tests can pause at the same boundary.
There is no second runtime mutex, nested acquisition, new timeout on the
handler, or change to admission, money-effect, cancellation, durability,
maintenance, protection, unknown-state or grant semantics.

The dev-only Tokio `test-util` feature enables a controlled runtime clock.
The declared dependency version/pin is unchanged. `READ_DEADLINE` remains
five seconds and `RPC_TIMEOUT` remains one second.

## New coverage

- `stalled_snapshot_obeys_read_deadline_and_persists_unavailable` drives the
  actual `process_key -> observe_read -> tokio::time::timeout` path. A probe
  observes entry before the pending await and destruction on cancellation.
  The controlled Tokio clock verifies the deadline; the real private-file
  checkpoint must contain rejected/unavailable with null balance/time fields,
  zero completed observations and no prepare/start. It does not use elapsed
  filesystem or host wall time as a latency bound or change the host clock.
- `production_key_span_releases_capacity_before_cursor_commit` invokes the
  SAME helper as `run`. Its waiter uses the same operation-mutex dependency
  supplied to that helper; the test does not acquire or drop the controller's
  guard. It checks that capacity is held during the pending read, the safe
  unavailable outcome is durably present, and capacity is available while
  cursor persistence is deliberately paused. The cursor callback asserts it
  is outside the guard. Removing the guard, holding it after cursor entry or
  retaining it indefinitely must fail. The short virtual-time progress
  watchdog is test coordination, not a production fsync or money-operation SLA.

Both tests retain actual `PrivateDir` writes. This is helper/lock-span coverage
with a controlled owner, not a live funded-wallet or complete send/grant test.
It deliberately makes no hard seven-second claim for a whole filesystem span.

## Serial integration

The code diff is against the exact frozen `53f5e789...` wallet-control file.
The Cargo hunk adds only the dev feature. A separate
`/tmp/opencode/pool-wallet-stall-coverage-20261112/flake-comment.patch`
replaces the unsupported causal comment in the captured current primary
flake while keeping all three `--release` test commands. The older owned
worktree flake is not overlaid with primary changes. Parent owns integration,
commit/push and any required source-contract/bundle rebinding.

After the assigned slot, run the two new focused cases in default and release
profiles and the relevant source/style checks. Preserve each source/command
receipt and report actual outcomes. Historical debug failures do not become
green retroactively, and this correction does not establish their unmeasured
timing cause or a production funds-safety failure.
