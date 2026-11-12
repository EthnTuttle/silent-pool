# Lab L2 (`--retarget-regtest`) review: Fable

Date: 2026-11-05
Reviewer: Claude Fable 5.1 (subagent; read-only on the candidate)
Candidate: `/tmp/opencode/rbitcoin-lab-retarget-profile`, branch
`lab-retarget-profile`, uncommitted on base
`e8cf9c9d61f7c63ae59cc11ab1179cace50ca142` (verified ancestor of HEAD; HEAD
equals base).
Manifest: `/tmp/opencode/lab-l2-source-review-manifest.json`, 17 files,
`snapshot_sha256` `646656a84cbb5001f96559d98511499eaba35501d93038dc9560cd787bc764ff`.
Requirement source: `docs/REGTEST-LAB.md` §3.2, §4, §8 P0 and
`docs/reviews/2026-11-05-regtest-lab-review-summary.md` (corrected claims
only).

## Verdict

**REQUEST_CHANGES**, tied to snapshot `646656a8`.

- 1 code blocker (finding 1), 1 required test (finding 2), 3 suggestions,
  3 nits.
- Broad gates are **pending**; see "Gate status". Even with findings 1-2
  fixed this review cannot become APPROVE until the pending gates finish with
  recorded exit codes. No conditional approval is given.

## Hash verification

All 17 per-file SHA256 values in the manifest match the working tree
(verified with `sha256sum` over each path; no diff). `git status` shows the
same 15 modified plus 2 untracked source paths as the manifest, plus
`docs/reviews/2026-11-05-lab-profile-implementation.md` and
`scripts/__pycache__/` which are declared outside the code hash. The
`snapshot_sha256` aggregate did not reproduce from any obvious derivation
(concatenated hashes, `sha256sum`-style lines, canonical JSON with or without
the field); its derivation is not recorded in the manifest. Treat the
per-file hashes as the binding reference; the aggregate is unverified, not
mismatched. Record the derivation in the manifest for the re-review.

## What was checked and holds

Stated briefly so the findings can be read against it.

- Genesis and height 1: `check_header` validates genesis bits against
  `genesis_block(params)` and `pow_limit` (unchanged `0x207fffff`), so
  keeping `pow_limit` and introducing a separate floor is what lets genesis
  still validate. Height 1 computes `prev_bits = pow_limit` -> non-boundary
  -> `min_diff_bits` returns `prev_bits` (min-difficulty off) -> overlay
  rewrites to the floor or `--regtest-pow-bits`. Heights 2..23 inherit by
  `prev_bits`; the overlay only fires at height 1 on a chain validated under
  the profile.
- One next-work path: `apply_regtest_pow_overlay` has exactly two callers
  (`consensus::header::next_work_bits`, `net::chain::expected_bits_off_tip`).
  `expected_next_bits` (query) feeds `generate*`, `getblocktemplate`,
  `getmininginfo.next`, the SV2 template provider (`rbitcoin-sv2/src/template.rs:126`,
  `session.rs:634`), the regtest pad, and `mine_paying_block`. IBD lookahead
  and confirm both call `next_work_bits`. The `period_first` helpers in
  query, IBD, and net all gate on `no_pow_retargeting()` and fetch
  `height - interval`, so turning retargeting on makes every path require
  the period-first header (`check_header` rejects
  `missing retarget first header` otherwise). Consistent.
- `pow_limit` vs floor: `pow_hash_meets_target` and the net header
  pre-check (`chain.rs:1066`) use `pow_limit` as the validation ceiling;
  the floor enters only through `btc.max_attainable_target` inside
  `from_next_work_required` (`max_transition_threshold = min(prev << 2,
  max_attainable_target)`). Because `header.bits` must equal the computed
  bits, nothing between the floor and `pow_limit` is acceptable at height
  >= 1. Chainwork is computed from header bits (`header_work_bits`), so fork
  choice follows real work under the profile.
- Wrap claim: `bitcoin-0.32.102/src/pow.rs:404-411` does
  `prev_target.mul(actual_timespan)` with `U256::mul` =
  `overflowing_mul` + `debug_assert!(!overflow)` (`pow.rs:989-993`).
  Regtest `pow_limit` is `0x7fffff << 232` (about `2^255`); times the
  60 s clamp minimum overflows. The floor `0x7fffff << 216` (about `2^239`)
  times 960 is about `2^249`. The hazard is real and the floor bound is
  correct. See finding 3 for the mechanism wording.
- Cross-network refusal, mixed flags: `apply_regtest_retarget` refuses
  non-regtest; `apply_regtest_pow_bits` runs first in
  `NodeConfig::chain_params` and rejects zero, negative, and overflowing
  bits; `apply_regtest_retarget` then refuses an overlay easier than the
  floor and accepts equal. Default regtest and every other network return
  from the overlay before the new guard (`regtest_pow_bits_overlay` is
  `None`, or `network != Regtest`); `apply_regtest_retarget` is only
  called when the flag is set. Non-lab behavior unchanged.
- Introspection: `consensus` object carries network, genesis hash,
  `pow_limit_*`, `retarget_floor_*`, spacing, timespan, interval, the three
  retarget flags, the overlay, subsidy interval, maturity, and buried
  heights. Nothing secret. Enough for L3 to read a reference
  `R >= effective pow limit` and stable profile metadata without a schema
  change. Fallback to `ChainParams::for_network` when `ctx.chain` is `None`
  follows the existing pattern in `mine.rs`/`chain.rs` and is not reachable
  in the production node.
- No second algorithm, no orphan backdoor, no env knob, no schema change.
  The profile is Core's retarget formula with a lab schedule and a floor.
  The non-parameterized schedule is acceptable as the first concrete
  profile; it is an assumption the lab inherits (see suggestion 5).
- Floor hashrate assumption: at the floor a block costs about `2^17`
  hashes, so with a 10 s spacing the floor binds once aggregate lab hashrate
  drops below about 13 kH/s. Above that the schedule tracks hashrate within
  the 4x clamps. Tests do not and should not claim benchmark numbers.

## Findings

### 1. Blocker: the `height >= interval` guard is unreachable on a profile chain and, where it is reachable, feeds `pow_limit` into the retarget multiply

`crates/rbitcoin-consensus/src/header.rs:328-330`.

With retargeting on and min-difficulty off, the `bits` argument at a
non-boundary height is the parent's validated bits. On any chain validated
under the profile the parent at height >= `interval` carries either a
retarget result (`<= floor`) or an inherited one, never `pow_limit`. So the
guard never returns on a chain the profile produced; the "no snap-back"
property the RPC test proves at height 26 comes from `bits != pow_limit`
at line 321, not from the guard. The guard is a dead branch on the
profile's own chains (`docs/code-shape.md`, "No new silent dead branches").

The one way to reach it is a datadir mined under default regtest (pow-limit
bits) and reopened with `--retarget-regtest`. Then, at the next
non-boundary height, the guard returns `pow_limit` as the expected bits, so
the legacy difficulty is extended to the next boundary, where
`retarget_bits` calls `from_next_work_required(pow_limit, ...)`: the debug
assertion in `U256::mul` fires (every `cargo test`, every debug node) or the
release build computes the wrapped value. That is exactly the failure the
floor exists to prevent. Without the guard the same reopen returns the
overlay (floor) as the expected bits, the chain heals onto the floor at the
next block, and the next boundary multiplies from the floor. The guard
makes the only reachable case strictly worse.

Fix: delete lines 328-330 and the matching sentence in the doc comment
(lines 303-305, "the overlay never returns past it"). Replace the third
assertion of `retargeting_overlay_initializes_the_first_period_only`
(`tests/header/regtest_overlay_tests.rs:35-39`), which constructs the
unreachable input, with either nothing (the accept-path test already pins
no snap-back at height 26) or a journey that reopens a default-regtest store
of >= 24 blocks under the profile and asserts the next expected bits are the
floor and the following boundary validates without panic.

Verification: `cargo test -p rbitcoin-rpc --test suite
retarget_profile_boundary_takes_over_from_the_overlay` still passes with
the guard removed (it never depended on it); the consensus unit test is
rewritten as above.

### 2. Required test: the floor is never exercised as a clamp; no second boundary; no easier direction

`crates/rbitcoin-rpc/tests/methods/mod.rs:6377-6430`,
`crates/rbitcoin-test/tests/scenarios.rs:3627-3722`.

Both behavioral tests drive exactly one boundary and it goes harder
(tight period, 60 s clamp, `0x1e7fffff -> 0x1e1fffff`), which lands below
the floor and never touches `max_attainable_target`. The only assertions
on the floor are field equality (`tests/cli.rs:1170-1178`) and the RPC
string `retarget_floor_bits`. Remove `self.btc.max_attainable_target =
floor` from `apply_regtest_retarget` (`params.rs:470`) and every behavioral
test still passes; a slow period would then retarget above the floor
toward `pow_limit` and the wrap returns after a few slow periods. The
change's central mechanism has no test that fails when it is removed,
which is the test-first rule in `docs/how-we-plan.md` applied to behavior,
not to a struct field.

Fix: extend `retarget_profile_boundary_takes_over_from_the_overlay` (or add
one accept-path test) past height 25: mine 26..47 with widely spaced
timestamps (for example 1000 s apart; 2011-era mocked times are far below
the wall-clock future cap) so the period timespan clamps at 960 s, assert
the boundary at 48 is `0x1e7fffff` (4x easier from `0x1e1fffff`, equal to
the floor), then mine 49..71 slow again and assert the boundary at 72 stays
`0x1e7fffff` (the `prev << 2` result is clamped to the floor, not to
`pow_limit`). This covers the easier direction, the floor clamp, and a
multi-period chain in one test. Keep the journey as is; the clamp is
consensus math a unit-level accept-path test may own (`TESTING.md`).

Verification: with `params.rs:470` temporarily removed the new assertion
at height 72 fails (`0x1f7fffff`); restored, it passes.

### 3. Suggestion: "wraps" understates what a debug build does

`params.rs:449-452`, `cli.rs:376-377`, `COMPAT.md` new row,
`docs/operator/operations.md` new row, `changelog.d/retarget-regtest.md`,
and the implementation record.

`U256::mul` is `overflowing_mul` with `debug_assert!(!overflow)`
(`bitcoin-0.32.102/src/pow.rs:989-993`). A debug node or any `cargo test`
panics inside header validation; only a release build wraps. "Core
`arith_uint256` wraps the same way" is true of Core's type, but Core
regtest never retargets so Core never reaches it. Say "trips a debug
assertion in debug builds and wraps in release" once, in the owner row
(`operations.md`), and let the other sites point there instead of each
restating the mechanism (one owner per fact).

### 4. Suggestion: the first period is anchored at genesis time, so on a real clock boundary 24 always clamps to the slow side

`header.rs:273-285` (`retarget_bits`), period-first resolution at
`height - interval = 0`.

Core's first period measures from the genesis header. Regtest genesis is
2011; a lab on wall-clock time has a first-period timespan of years, so the
boundary at 24 is always the 960 s clamp: from the floor it stays at the
floor, from a harder `--regtest-pow-bits` it goes 4x easier. The first
hashrate-responsive boundary is 48. The journey hides this with
`setmocktime(GENESIS_TIME + 30)`. The implementation record's runner
contract should state it, and the `operations.md` row should not imply
that the tight-period 4x result is what a wall-clock run sees. This is a
documentation gap, not a code defect; the behavior is Core's.

### 5. Suggestion: declare the `generatetoaddress` grind limit under the profile

`crates/rbitcoin-consensus/src/regtest_pad.rs:60-84`
(`regtest_mine_fits_one_nonce_span`, ratio cap `2^31`).

The floor is `2^16` below the pow limit. Each clamped-tight boundary
hardens 4x, so after seven consecutive tight periods (boundary at height
192) the target no longer fits one nonce span and `generatetoaddress`
errors. SV2 CPU miners are unaffected (they do not use the node's
grinder), and on a real clock difficulty equilibrates instead. The
`--regtest-pow-bits` row already documents the nonce-span error; the new
row should too, since mocktime-driven harness runs will hit it. Also
record the floor-binding hashrate (about 13 kH/s aggregate at 10 s
spacing) as the profile's assumption.

### 6. Nit: `chainwork` docs still say regtest is 2 per block

`docs/rpc.md:139`, `crates/rbitcoin-rpc/src/methods/mod.rs:1056-1057`.

Under the profile regtest header work is `2^256 / (target + 1)`, about
`2^17` at the floor. Qualify as "default regtest 2 per block".

### 7. Nit: `--retarget-regtest=0` works but is not advertised

`cli.rs:351` usage shows `[--retarget-regtest]`; sibling bool keys show
`[=0|1]`. `is_bool_key` accepts `--retarget-regtest=0` and `parse_conf_bool`
takes `0|1|true|false|yes|no|on|off`. Align the usage token and the
`operations.md` row with the other bool flags.

### 8. Nit: `apply_regtest_pow_bits` after `apply_regtest_retarget` bypasses the floor check

`params.rs:421` checks `target >= self.pow_limit` only. `NodeConfig::chain_params`
orders the calls correctly, so this is library hardening: check against
`self.btc.max_attainable_target` (equal to `pow_limit` on default regtest,
the floor after `apply_regtest_retarget`) so the order of the two calls
does not matter.

## Not flagged

- `lint/hex-baseline.txt` +1 for `params.rs`: the new constant is consensus
  data; the baseline line is the correct ratchet use, not an exception.
- `docs/README.md` `reviews/` row: a new owner category in a repo that
  keeps one owner per fact. The record restates facts owned by `COMPAT.md`
  and `operations.md`. Left to the maintainers; it is outside the code
  hash and the parent declared it mutable.
- Journey uses `setmocktime` + `generatetoaddress`, not an SV2 CPU miner.
  Declared as such in the record; acceptable for L2. Genuine PoW is still
  required by `pow_hash_meets_target` at every height, and the node owns
  expected bits on every template path, which is the user's authority goal.
- SV2 `MAX_SESSIONS = 8` disclosure: correctly declared as a capacity
  limit, not claimed as a capability.

## Gate status (mutable, outside the code snapshot)

Read once from `/tmp/opencode/*.log` at about 17:50 local; not polled.

| Gate | Status | Evidence |
|---|---|---|
| fmt, unwrap/hex ratchets + self-tests | pass (exit 0) | `fmt-check.log`, `unwrap-ratchet.log`, `hex-ratchet.log`, `*-selftest.log` |
| clippy consensus/node/rpc/test/net | pass | `clippy.log` |
| consensus, node, rpc suites | pass | `suite-consensus.log`, `suite-node.log`, `suite-rpc.log` |
| New targeted tests | pass | `green-consensus.log`, `green-node.log`, `green-rpc.log`, `green-rpc-info.log`, `green-journey.log` (1 passed) |
| `rbitcoin-net` suite | **unresolved**: 723 passed, 1 failed (`tip_accept::full_lane_parks_instead_of_spinning`, "saw 0"); isolated rerun passed | `suite-net.log`, `net-flake-rerun.log` |
| `rbitcoin-test` suite (default silo) | **unresolved**: 91 passed, 6 failed (2 `asmap_journey` silo, 4 timing journeys) | `suite-test.log` |
| ast-grep self-test + scan | self-test ok; scan log 188 bytes, exit code not recorded in the log | `ast-grep-selftest.log`, `ast-grep.log` |
| `cargo deny check` | **failed to query registry** (offline/sandbox), no verdict | `cargo-deny.log` |
| workspace clippy `--all-targets -D warnings` | log has cache warnings only; exit code not recorded | `clippy-workspace.log` |
| `cargo test --workspace --exclude rbitcoin-bench` (dev silo) | **running** | `workspace-test.log` |
| `nix build .#checks...nixos-module-eval` | **not started** (no log) | - |

"No `crates/rbitcoin-net/` change in the diff" and "timing-heavy" are
plausible but are not causal proof. For APPROVE I need, per failing test:
a pass in the corrected `target/dev` silo under the full suite, or a
reproducible failure on base `e8cf9c9d` with the same command (baseline
comparison). Exit codes for ast-grep, workspace clippy, and deny must be
recorded, not inferred from a quiet log.

## What re-review needs

1. Findings 1 and 2 applied; new manifest with per-file hashes and the
   aggregate derivation recorded.
2. Final exit codes for every row above, with the net and rbitcoin-test
   failures resolved as described.
3. Suggestions 3-5 and nits 6-8 are not gating; say which were taken.

---

# Second verdict (revision 2)

Date: 2026-11-05 (later the same day)
Manifest: same path, now 18 files; `snapshot_sha256`
`d2f37b9113e8855245866289ed393043aed6e6153e8101375072cbb23624d499`,
recipe recorded in the manifest (`sha256` of canonical sorted compact JSON of
`{base, files}`). Excludes `docs/reviews/` and bytecode.

## Verdict

**APPROVE**, tied to snapshot `d2f37b91`, for the L2 profile as source and
for lab use. One CI-stability item is recorded below as a non-blocking
follow-up with a bounded check; it is not a correctness condition on the
profile and I explain why. The parent may still choose to hold merge on it.

## Hash verification

All 18 per-file hashes match the working tree. The aggregate reproduces
exactly from the recorded recipe (sorted keys, compact separators, over
`{'base': ..., 'files': [...]}`). `git status` matches the manifest plus the
two declared exclusions; HEAD is still the base commit.

## Findings from revision 1, as applied

- **B1 (guard) - fixed.** `header.rs` now carries only the doc-comment
  change; the `height >= interval` branch is gone. The unit test's third
  assertion (`regtest_overlay_tests.rs:40-46`) now asserts the reachable
  behavior: pow-limit bits past the interval step onto the overlay. The
  doc comment states the fresh-datadir contract and says the overlay steps
  a legacy chain onto the profile at the next non-boundary height. The
  implementation record says plainly that this is a documented contract,
  not open-time enforcement, and that a legacy datadir reopened exactly at
  a boundary (parent bits = pow limit, boundary height) is outside the
  contract and can still reach the overflow. That statement is correct and
  I do not restate any runtime old-data rejection; there is none.
- **B2 (floor clamp test) - fixed.** `retarget_profile_boundary_takes_over_from_the_overlay`
  (`rpc/tests/methods/mod.rs:6372-6478`) now drives three periods through
  the accept path: tight period -> 24 = `0x1e1fffff`; slow period
  (1000 s blocks, 960 s clamp) -> 48 = `0x1e7ffffc`; slow period -> 72 =
  `0x1e7fffff`. I checked the arithmetic: `0x1fffff << 216` times 4 is
  `0x7ffffc << 216`, below the floor by compact quantization, so 48 is
  `1e7ffffc` and not the floor; `0x7ffffc << 216` times 4 is
  `0x1fffff0 << 216`, which the floor clamps to `1e7fffff`; without the
  clamp it would encode as `1f01ffff`. The height-72 assertion fails when
  `params.rs:470` (`max_attainable_target = floor`) is removed, which is
  the behavioral pin revision 1 asked for. Timestamps stay monotonic and
  far below the wall-clock future cap.
- **Nit 8 (call order) - fixed with one precision note.**
  `apply_regtest_pow_bits` now bounds against
  `btc.max_attainable_target` (`params.rs:424-426`) and
  `tests/params.rs:222-238` pins both orders. The record's claim should be
  stated as "the order cannot bypass the floor", which is true, not "the
  orders are equivalent": in production order (`pow_bits` then
  `retarget`) an overlay equal to the floor is accepted (`> floor` check);
  in the reverse order it is refused (`>= max_attainable_target`, the
  pre-existing "not harder is a no-op" rule). Both are safe. The
  equal-to-floor case in production order is not covered by a test
  (`tests/cli.rs` uses `0x1d7fffff` and `0x1f7fffff`); a one-line
  assertion would close it. Not gating.
- **Suggestions 3-5, nits 6-7 - taken.** `operations.md` owns the
  overflow mechanism (debug assert, release wrap) and the other sites point
  to it; the genesis-anchored first period and the 48 = first responsive
  boundary are documented; the `generatetoaddress` nonce-span limit and
  the ~13 kH/s floor-binding assumption are recorded; `chainwork` docs
  say "default regtest 2 per block"; usage shows
  `--retarget-regtest[=0|1]`.

## Re-checked on the frozen source

- `pow_limit` (validation ceiling, unchanged) vs `max_attainable_target`
  (retarget clamp, the floor): still the only two roles, still applied at
  the same sites as revision 1. Genesis validates against `pow_limit`;
  height 1 gets the overlay; no target between the floor and the pow limit
  is acceptable at height >= 1.
- `getblockchaininfo.consensus` fields unchanged from revision 1; all
  consensus data, nothing secret; `retarget_floor_*` equals `pow_limit_*`
  on networks without a floor, which is the right default semantics.
- Default regtest and other networks: `apply_regtest_pow_overlay` returns
  before any profile logic when the overlay is `None` or the network is
  not regtest; `apply_regtest_retarget` runs only under the flag.
- Node authority over the target: every template path (`generate*`,
  `getblocktemplate`, `getmininginfo.next`, SV2 `template.rs:126`,
  `session.rs:634`) reads `expected_next_bits`; the profile adds no input
  by which a miner, pool, or share policy could substitute the block
  target. Share targets are untouched by this change. Genuine PoW is still
  required at every height by `pow_hash_meets_target`.

## Gate evidence (complete per the record; read once, not polled)

Green with exit 0: fmt, both ratchets and self-tests, workspace clippy,
consensus 318, node 116, RPC 212, journey 1, bench 49, ast-grep, deny,
NixOS module eval. Full workspace suite exits 101 on both candidate and
clean base (`/tmp/opencode/rbitcoin-base-repro`, HEAD = base, clean).
Base reproduces `tip_accept::full_lane_parks_instead_of_spinning` with the
identical panic, and the `end_of_ibd`, `invalidate_*`, and other timing
families rotate on both sides. Those are not attributable to the diff.

### The unresolved split: `cross_surface::sv2_tp_bootstrap`

Facts I verified in the logs: failed 4/4 candidate full-suite runs, passed
2/2 base full-suite runs, passed in isolation on the candidate. The four
candidate failures are at **three different sites**:
`cross_surface.rs:3684` (second template lacks the just-sent fee, twice),
`cross_surface.rs:3853` (fee push took 5.35 s against a 1.5 s bound),
and `cross_surface.rs:3501` (10 s SV2 receive timeout in the `sv2_recv`
helper). Facts I verified in the source: the test runs
`ChainParams::regtest()` with no profile flag, never calls
`getblockchaininfo`, and exercises SV2 template distribution and mempool
fee visibility. The diff's only production code on that path is
`apply_regtest_pow_overlay`'s early return on a `None` overlay and the
added `NodeConfig` field; `consensus_info` runs only inside
`getblockchaininfo`. The journey-skip diagnostic rules out the one added
test as the load source, and nothing else.

Assessment: a logic regression fails at one site; three sites across four
runs is load/timing. There is no code path from this diff to the test's
behavior. The 4/4 vs 0/2 split is still statistically notable (under a
shared failure rate the best case is roughly a 2 percent chance), so the
author's "binary-layout/timing drift" is a hypothesis, not proven, and I
do not accept it as proven. It is also not evidence against the profile's
correctness, which is why it does not block this approval.

Bounded follow-up (CI stability, pre-merge, no re-review needed unless it
fails): run `cargo test -p rbitcoin-test --test suite` three times each on
candidate and base in the same silo with no other load, same command,
and record the `sv2_tp_bootstrap` outcomes. Candidate 0/3 failures, or
base >= 1/3, settles it as a flake. Candidate >= 2/3 with base 0/3 is a
real split and needs root cause (start at the fee-visibility race at
`cross_surface.rs:3684`, which is the most frequent site) before merge.

## Remaining non-gating items

1. Pin equal-to-floor `--regtest-pow-bits` in production order
   (`tests/cli.rs`), and phrase the call-order claim as "cannot bypass",
   not "equivalent".
2. The `sv2_tp_bootstrap` A/B above.

---

# Pre-integration adjudication (2026-11-06)

Inputs: `docs/reviews/2026-11-06-lab-l2-stability.md`,
`/tmp/opencode/lab-l2-stability-results.json`,
`/tmp/opencode/L2-stability-{candidate,base}-{1,2,3}.log`,
`/tmp/opencode/L2-stability-load-evidence.txt`. Candidate snapshot
`d2f37b91` and base `e8cf9c9d` unchanged before and after the runs.

## Judgment

**CLEARED_WITH_RECORDED_BASELINE_LIMITATIONS** for lab integration of the
L2 profile (building and running `--retarget-regtest` nodes from this
worktree for the live-retarget scenario). This is not "all green". The
limitation below is recorded against the merge-to-master gate, with one
bounded diagnostic that is recommended before merge and is not required
for lab use.

## Criterion outcome

My rule (candidate >= 2/3 `sv2_tp_bootstrap` failures with base 0/3) was
**not triggered**: 0/3 candidate, 0/3 base, six serial alternating
package-suite runs, quiet preflight (load 0.42). The test's only failures
remain the 2026-11-05 full-workspace-load runs. The specific concern that
held revision 2 is closed.

## New fact the check surfaced

Every candidate suite-alone run exited 101 with exactly one failure; every
base run exited 0.

| Run | Tree | Failure | Site | Message |
|---|---|---|---|---|
| 1 | candidate | `end_of_ibd::end_of_ibd_work_fork` | `end_of_ibd.rs:1026` | 30 s wall `Elapsed` |
| 3 | candidate | `integration_multinode::mocktime_generate_keeps_ponging_peer` | `integration_multinode.rs:147` | "ponging peer dropped on mock step 5: []" |
| 5 | candidate | same | same | "... mock step 10: []" |
| 2, 4, 6 | base | none | | |

Facts that bear on attribution, checked in source and logs:

- Neither test runs the profile. `mocktime_generate_keeps_ponging_peer`
  starts two `P2PNode`s on `ChainParams::regtest()` and asserts a 2 s wall
  window per mock step in which the peer must pong after a 600 s mock
  jump; `[]` means both peers were already dropped. `end_of_ibd_work_fork`
  is a 30 s wall over a two-miner, scripted-peer journey. Both are
  load-sensitive by construction.
- Both have same-command base repros under the 2026-11-05 full-workspace
  runs (base run 1 failed both at the identical sites; base run 2 failed
  `end_of_ibd_work_fork` again). They are not candidate-only failures.
- No shared-state path from the added journey: mock time is per
  `NodeClock` (`consensus/src/clock.rs`), not a process hook;
  `BlockingRegion` is thread-local; the journey's `spawn_blocking` +
  `block_on(run_p2p)` + `wait_published_listener` pattern is the same one
  three other tests in the same binary use; the process-global decode
  semaphores (`msg_decode.rs`) are untouched by a P2P-off node. The
  journey awaits `stop` and its join; it leaves nothing running.
- The diff's production code on the failing tests' path is the overlay's
  `None` early return and one `NodeConfig` field. `consensus_info` runs
  only inside `getblockchaininfo`, which neither test calls.
- Host load during the window rose from 0.80 to 4.8 across the six runs
  (`L2-stability-load-evidence.txt`); the base clean-pass binary time
  itself ranged 17.5-58.3 s, so the environment was not steady, and
  candidate run 1 (95 s, first run of the window) likely paid a cold-cache
  cost the base did not.

What this does and does not establish: 3/3 vs 0/3 is about a 1.6 percent
chance under a shared failure rate, so the split is a real signal and I do
not call it a proven flake. There is no mechanism from this diff to those
tests; the one candidate-only difference with a plausible effect is that
the suite binary carries one more in-process real-node test (97 vs 96),
which shifts the libtest parallel schedule and adds a few seconds of node
load. That is a hypothesis about the test binary, not about the profile,
and n=3 per side cannot confirm it.

## Why this is not a hold

A HOLD_FOR_SPECIFIC_FIX needs a specific fix. There is no identified
defect in candidate code, the profile's own tests passed 3/3 inside the
same failing runs, and the two tests that failed are known to fail on
base under load with no margin. Weakening their windows, serializing with
`RUST_TEST_THREADS=1`, or retry-farming are all ruled out by `TESTING.md`
and by this review. The remaining question (does the added journey's
presence in the shared binary tip these two tests) is a CI-stability
question for the merge gate, and it is answerable in one bounded step.

## Recorded limitation and bounded pre-merge diagnostic

Limitation: on this host, suite-alone, the candidate `rbitcoin-test`
binary fails one load-sensitive, profile-unrelated test per run while the
base binary does not. CI on other hardware may or may not reproduce it.

Diagnostic (recommended before merge, not required for lab use; about
3-5 minutes; needs a quiet window the parent must grant, not assume):

```
cd /tmp/opencode/rbitcoin-lab-retarget-profile && nix-shell --run \
  'CARGO_BUILD_JOBS=2 cargo test -p rbitcoin-test --test suite -- --skip scenarios::retarget_regtest_node_journey'
```

three serial runs, same silo, stdout to `/tmp/opencode`, exit codes and
failing names recorded.

- 0/3 failures: the journey's presence in the shared binary is the
  trigger. Then compare it with its three sibling `run_p2p` journeys for
  what differs (2011 mocktime with `max_tip_age_secs = u64::MAX`, tiny
  heads, the 23-block generate), and if nothing explains it, moving the
  journey into its own `rbitcoin-test` test target is a justified
  structural change: it keeps every assertion intact and removes the load
  from the shared binary. `TESTING.md` line 199 ("Default journeys are one
  binary") then needs a one-line recorded exception. Do this only on that
  outcome, not pre-emptively.
- >= 1/3 failures: the journey is not the trigger, the only remaining
  candidate-side difference is production code with no path to these
  tests, and the split is a baseline-rate flake that the base also shows
  under load. Record and merge; the two tests' margins are the
  maintainers' item, outside this change.

No further unbounded batches. If the parent prefers to skip the
diagnostic and merge on the base-repro evidence, that is defensible; the
record above states exactly what remains unproven.
