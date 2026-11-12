# Lab L2 stability check: sv2_tp_bootstrap pre-integration criterion

Date: 2026-11-06
Runner: GLM 5.3 (subagent; read-only validation packet)
Candidate: `/tmp/opencode/rbitcoin-lab-retarget-profile`, branch
`lab-retarget-profile`, uncommitted on base
`e8cf9c9d61f7c63ae59cc11ab1179cace50ca142`, file snapshot
`d2f37b9113e8855245866289ed393043aed6e6153e8101375072cbb23624d499`
(`/tmp/opencode/lab-l2-source-review-manifest.json`, 18 files, verified
byte-identical before and after the runs).
Baseline: `/tmp/opencode/rbitcoin-base-repro`, clean at the same commit, own
`target/dev` silo.
Machine-readable results: `/tmp/opencode/lab-l2-stability-results.json`.

## Criterion outcome

**NOT TRIGGERED.** Fable's rule: candidate ≥ 2/3
`cross_surface::sv2_tp_bootstrap` failures with base 0/3 requires root cause
before integration. Observed: `sv2_tp_bootstrap` passed **3/3 candidate runs
and 3/3 base runs** (six of six; `... ok` lines grepped per log). The test has
now never failed suite-alone on either tree; its only observed failures are
the 2026-11-05 full-workspace-load runs (candidate 4/4 including the
journey-skipped diagnostic, base 0/2).

## Setup

Preflight 2026-11-06T06:59:53Z: no cargo, rustc, or nextest processes; load
average 0.42. Process census captured names only (routine desktop services).
Both trees use the same `nix-shell` (rustc 1.95.0 `59807616e`, pinned via
flake.lock), per-tree `CARGO_TARGET_DIR=$PWD/target/dev`, `RUST_TEST_THREADS`
unset (default threads), `RUST_BACKTRACE=1` (shell default, identical both
trees). `rbitcoin-node` debug binaries and suite test binaries present in
both silos.

Command per run (recorded, identical for both trees):

```bash
cd <tree> && nix-shell --run 'CARGO_BUILD_JOBS=2 cargo test -p rbitcoin-test --test suite'
```

Stdout/stderr redirected to `/tmp/opencode/L2-stability-<tree>-<n>.log`; exit
code and wall duration appended to each log. Six runs, strictly serial,
alternating candidate/base. No `--skip`, no retries, no timeout or
test-semantics changes, no concurrent suites. Host load per run:
`/tmp/opencode/L2-stability-load-evidence.txt` (0.80–4.83 across the
interval).

## Results

| # | Tree | Exit | Wall | Suite binary | Passed/failed | Failing test (assertion) |
|---|------|------|-----|--------------|----------------|--------------------------|
| 1 | candidate | 101 | 100 s | 95.02 s | 96/1 | `end_of_ibd::end_of_ibd_work_fork` (`end_of_ibd.rs:1026`, wall: `Elapsed(())`) |
| 2 | base | 0 | 79 s | 17.53 s | 96/0 | — |
| 3 | candidate | 101 | 37 s | 27.33 s | 96/1 | `integration_multinode::mocktime_generate_keeps_ponging_peer` (`integration_multinode.rs:147`, mock step 5) |
| 4 | base | 0 | 62 s | 58.27 s | 96/0 | — |
| 5 | candidate | 101 | 48 s | 43.76 s | 96/1 | `mocktime_generate_keeps_ponging_peer` (`integration_multinode.rs:147`, mock step 10) |
| 6 | base | 0 | 53 s | 44.23 s | 96/0 | — |

`cross_surface::sv2_tp_bootstrap`: ok in all six logs.
`retarget_regtest_node_journey` (candidate only, 97th test): ok in all three
candidate logs. Clean-pass suite-binary duration itself ranged 17.5–58.3 s on
base, so wall duration carries little signal; the failing runs are longer
because the wall-timeout test burns its budget before failing.

## Facts and uncertainty

- Each candidate run failed exactly one test (`end_of_ibd_work_fork` 1/3,
  `mocktime_generate_keeps_ponging_peer` 2/3); base was clean 3/3. This
  3/3-vs-0/3 suite-alone split is a fact from this sample, not an attributed
  cause.
- Both failing tests have same-command base repros from the 2026-11-05
  full-workspace runs: base run 1 failed both identically (same assertions,
  `end_of_ibd.rs` wall timeout and `integration_multinode.rs:147` peer
  drop), base run 2 failed `end_of_ibd_work_fork` again. Neither test is
  candidate-only, and neither runs under the retarget profile.
- Today's base sample (3 clean suite-alone runs) does not measure the
  base-alone failure rate under this regime; yesterday's base failures were
  under full-workspace load, a different regime.
- One plausible, unproven mechanism for the split: the candidate suite
  binary carries one additional test (`retarget_regtest_node_journey`, 97
  vs 96 tests) that spawns a real node inside the same process, adding load
  the base binary does not have. n=3 cannot confirm or exclude a margin
  shift from it.
- No blanket "proven flake" or "fully green" claim is made. The specific
  bootstrap criterion passes; the suite-alone split is left for Fable to
  weigh.

## Cleanup

No cargo or rustc processes remain. No source edits, commits, silo cleanup,
or policy changes; candidate git status unchanged (19 entries, snapshot
`d2f37b91…` re-verified post-run), base still clean. `scripts/__pycache__`
absent.

## Adjudication (Fable, 2026-11-06)

**CLEARED_WITH_RECORDED_BASELINE_LIMITATIONS** for lab integration; the
`sv2_tp_bootstrap` criterion is closed (0/3 vs 0/3). The suite-alone 3/3 vs
0/3 split in `end_of_ibd_work_fork` and `mocktime_generate_keeps_ponging_peer`
is recorded as a merge-gate limitation with one bounded diagnostic (three
candidate runs with `--skip scenarios::retarget_regtest_node_journey`),
not as a hold. Criteria, facts, mechanism checks, and outcome branches are in
`2026-11-05-lab-l2-fable-review.md`, section "Pre-integration adjudication
(2026-11-06)".
