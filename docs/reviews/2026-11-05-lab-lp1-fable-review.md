# 2026-11-05 - Fable review: LP-1 FULL v2 share-policy integration

Reviewer: Fable (subagent). Role: gate review only. No candidate edits, no
commits, no pushes, no subagents. This file is the only artifact written into
any tree. Probe sources and logs live under `/tmp/opencode/Fable-LP1-*`.

## Verdict

**REQUEST_CHANGES** - bounded. The accounting core, version dispatch,
persistence, wire codes and settlement freeze are correct as far as my
independent probes reach (section 4), and the legacy v1 path is unchanged.
Two findings block approval of the *protocol* and *guard-ordering* gates the
task set: v2 does not actually refuse type-32 at the SV2 ingress (F1), and a
v2 issuance failure inside `Front::refresh` leaves the previous job live after
the slice/height state has already moved (F2, counterexample in section 4.7).
Both are small, local fixes. Everything else is ranked below as non-blocking
and should be folded into the same rework.

## Snapshot reviewed

Worktree `/home/ethan/.local/share/opencode/worktree/ad8886/lab-share-integration`,
HEAD `b0ab4ca34fda07c61a6335d05f598eaacc69678c` (detached), 56 modified or
untracked files, nothing else modified (`git status --porcelain -uall` shows no
path outside the manifest). Manifest `/tmp/opencode/lab-lp1-review-manifest.json`
(file SHA-256 `7f0a81055e3f69033c258aa6041092ec6568d03da5e35342bd01c40d36e119a5`).
Aggregate recomputed with the stated recipe over `base` plus `path`/`sha256`
only:

```
81fa17d27ee80641167aaeb53e97f627ba8daa4688e56a227a1ef44c9776b329   (matches)
```

All 56 files were re-hashed from disk before the review and again after the
last probe run: 0 mismatches both times. Baseline
`/tmp/opencode/lab-lp1-baseline.json` (file SHA-256
`31a5d22a03a3c74939c11c73ebf8883e12ccb9f7344b059b631b55e3da7ddb9e`,
combined `ed5a40f1...bc16a`): the 38 approved inputs were re-hashed in their
source trees (`lab-sv2-conformance` L1 `fa532607`, `lab-share-math` LP1a
`45df0eb3`, `lab-contracts` L3 `3edcd4e7`) and every `baseline_sha256` in the
manifest equals the approved hash. Summary 28 unchanged / 10 modified /
18 added confirmed.

Precedence applied: `AGENTS.md` (worktree copy), `DESIGN.md` > `SPEC.md` >
`WIRE-FORMATS.md`; the two L3 decisions and `docs/REGTEST-LAB-IMPLEMENTATION.md`
section 4 as the binding v2 contract; `docs/decisions/2026-11-05-lab-lp1-integration.md`
as the candidate's own statement of intent. The LP1a pure module and the L1
files were treated as frozen; I reviewed their *adoption*, not their
arithmetic.

## 1. What I read

Diffs of the 10 modified files against their approved versions
(`work_unit.rs`, `spc-accounting/src/lib.rs`, `sv1.rs`, `front.rs`,
`config.rs`, `main.rs`, `sv2_server.rs`, `api_backend.rs`,
`tests/sv2_conformance.rs`, `bins/spc-agent/src/sv2_client.rs`), the 18 added
files in full (notably `evidence.rs`, `payout.rs` and store `postgres.rs`,
`wire.rs`, `bootstrap.rs`, `models.rs` diffs against HEAD, migration 0005,
`work_units.rs`, `v2_integration_tests.rs`, `main_sv2_pipeline.rs`), the
decision/changelog/report, and the logs `LP1-workspace-final.log`
(38 `test result` lines summing to 483 passed, 0 failed),
`LP1-main-pipeline-run3.log` (1 passed), `LP1-db-tests-9.log` (20 + 1 passed
against a real Postgres), `LP1-clippy-final.log` (clean), `LP1-sv1-regtest.log`
(`OK: regtest e2e passed`), `LP1-conformance-fixed2.log` (4 passed). The
logged claims match the report.

## 2. Verified correct (code reading plus probes)

Normative / policy:

- Config matrix (`config.rs:689-830`): `work_unit_version` other than 1/2
  fails; v2 requires regtest, 32-byte hex `work_reference_target`,
  power-of-two `share_work_multiplier` via `validate_multiplier`,
  `max_work_multiplier <= 2^32`, `max_window_target_units` u128 decimal
  `<= 2^64`, `max_slice_shares <= 65000`, explicit `window_rule` only if it
  is the matching one; `share_difficulty` with v2, v2 fields with v1, SV1
  enabled, or `extensions.share_accounting = true` are errors. v1 keeps the
  implicit default difficulty 1 and `Option<u64>` parsing of the legacy key.
  No alias, no unit conversion. `validate()` runs at load (`config.rs:835`).
- Generation authority (`front.rs:612-624, 766-781`): any `bits`/`version`
  change is `job_changed`, so a retarget always allocates a new generation;
  `rec.d = job.work.m_issue` (`front.rs:894`) and the share-target check use
  the same job; stale job ids are rejected before the dedup insert and before
  any booking (`front.rs:834-879`).
- Stepdown/step-up (`front.rs:769-774`, `work_unit.rs:451-480`): O(1), old
  `JobWork` values immutable (probe 1: m 2 -> 1 at `0x207fffff` -> 2 again,
  generations strictly increasing, generation-A share after B is `StaleJob`,
  not repriced).
- Q frozen per job: `window_target` is recomputed only in `refresh` from the
  template that produced the job; block events carry 65 at `0x200fffff` and 9
  at the pow limit (probe 1); settlement has no current-chain input at all
  and selects different windows from the same slices under frozen Q 9 vs 65
  (probe 2); a v2 snapshot without a frozen target is `MissingFrozenV2Target`
  rather than an estimate (`payout.rs:548-551`).
- SV2 target binding (`sv2_server.rs:566-645`): `SetTarget` precedes the job
  it must govern; the target can only change with `nbits`, which forces a
  future job plus `SetNewPrevHash`, so an active job's target is never
  rewritten (L1 conformance test 4/4 still passes with legacy `JobWork`).
- Exact booked `d` (`main.rs:407-414`, `sv2_server.rs:463-470`):
  `new_shares_sum` is the same `job.work.m_issue` the `ShareRec` receives;
  no `credited_difficulty` recomputation remains on the SV2 path. Main-process
  peer test observes `d = 2` twice (rerun `Fable-LP1-rerun-pipeline.log`).
- Disjoint correlation: SV1 ids carry bit 31 (`sv1_server.rs:184`), SV2 ids
  are `1..=0x7fffffff` (`sv2_server.rs:500`).
- Timeout/lost outcome (`sv2_server.rs:448-461`): registry entry removed,
  `share_outcome_unknown` emitted, session closed with an error; no
  definitive `SubmitSharesError` is sent while credit may still land.
- Evidence boundary: `crate::evidence` is referenced only from `front.rs` and
  `sv2_server.rs`; nothing under `ark/`, `spc-ext`, orders or receipts gained
  a field; `accounted` (with `sp_id`) is a tracing event on the SPc side only.
  `job_issued` carries front/channel/job/generation/nbits/target/m_issue/
  version/fingerprint/issued_at/activation/prev_hash as the contract requires.

Accounting:

- `push_v2`, `agg_v2`, `select_window_v2`, `raw_payouts_v2`, `claim_jd_v2`
  are the functions the front and payout actually dispatch to for v2
  (`front.rs:658-669, 684-691, 920-931, 1002-1005`, `payout.rs:555-585,
  715-718`). `raw_payouts_v2` keeps the SPEC §4.4 floor positions (one
  subsidy floor per identity, one fee floor per identity-slice term with
  denominator `t * DF_S`), uses the 256x256/512 exact divide, and refuses a
  `u64` result that does not fit. Probe 3: with `F_net * df = 2^162` the v2
  result is exact (`F_net`), v1 saturates to `u128::MAX / df` as documented;
  on an in-range two-slice/two-miner window v2 == v1 bit for bit.
- `push_v2` leaves the slice untouched on overflow; `agg_v2` fails the seal
  on fee-work overflow; `select_window_v2` fails on window overflow and on any
  non-v2 or wrong-fingerprint slice, including restored ones.
- Underfill/overshoot are derived (`Window::underfilled/underfill/
  target_overshoot`); no column.
- v1 untouched: `Slice::push/agg`, `select_window`, `raw_payouts`,
  `claim_jd` unchanged and still dispatched for `WorkUnitVersion::V1`;
  `JobWork::legacy` uses `credited_difficulty`/`effective_share_target`
  (probe 6: `m_issue == credited_difficulty(16, 0x200fffff) == 1`, window
  target `8 * 1`, rule 0, fingerprint None). No v1 test function was removed
  from `front.rs`, `sv1.rs`, `sv2_server.rs`, `payout.rs`, `tests.rs`,
  `wire.rs` or `tests/postgres.rs` (name-set diff against the approved trees).

Persistence / wire:

- Migration 0005: `SMALLINT NOT NULL DEFAULT 1`, `BYTEA` nullable
  fingerprint, `window_rule SMALLINT NOT NULL DEFAULT 0`; CHECKs for version
  `IN (1,2)`, `(version = 1) = (fingerprint IS NULL)`, rule `IN (0,1)` and the
  version/rule pairing. Existing rows acquire `(1, NULL, 0)` without
  conversion. `work_units.rs` proves the CHECKs against a real Postgres.
- Every `slice`/`distribution_snapshot` INSERT and SELECT carries the new
  columns (`postgres.rs:206-262, 270-298, 317-331, 792-802, 876-888,
  1108-1120, 1257-1282`); `work_version`/`window_rule_from_i16`/`opt_id32`
  fail closed on unknown codes and wrong lengths.
- Wire: `spc:slices` appends `U8 version, B0_32 fingerprint`; `spc:blocks`
  appends those plus `U8 window_rule`; v1 writes the zero sentinel and SQL
  stores NULL; v2 with zero fingerprint fails encode and decode; v1 with a
  non-zero fingerprint, version 0/3/9, rule 2, and mismatched version/rule
  pairs all fail decode; truncated old-format events fail with `Eof`
  (probe 5, plus the candidate's own wire tests).
- `BootstrapSnapshot` JSON carries version and hex fingerprint per slice;
  `difficulty`/`window_target` remain decimal strings.
- Restore: `apply_bootstrap` rejects v1 rows on a v2 front, v2 rows on a v1
  front, and v2 rows with another reference, each naming the slice id, with
  no zero-fill (probe 4, all four directions plus a stored-difficulty
  mismatch -> `InvalidRestoredSlice`). A matching v2 row restores and feeds
  the next window (`window_underfilled()` true for 2 < 65).

## 3. Findings (ranked)

### F1 - BLOCKING - v2 type-32 is still served at the SV2 ingress

`sv2_server.rs:256-258` routes every `0x0020` frame to `handle_type32`
(`sv2_server.rs:517-562`), which answers `GetWindow`/`GetShares`/
`GetTransactionsInJob` from `Type32Store` regardless of configuration.
`extensions.share_accounting` is read nowhere outside `config.rs` (true in the
base too, but LP-1 is the packet that makes the distinction load-bearing), and
`roll_slice` feeds every sealed slice, v2 included, into the store
(`front.rs:1015-1017`). The decision (item 1) and the report ("Config and
`/api/pool` explicitly reject/omit it") describe v2 type-32 as an unsupported
capability; in fact only the HTTP advertisement is omitted. A verifier that
speaks type-32 to a v2 pool receives `SliceSummary.difficulty` in reference
units and share leaves with no `job_id -> (share_target, m_issue)` mapping,
which is exactly the silent, wrong audit the L3 decision forbids.

Acceptance: `Sv2Server` takes the flag (e.g. `with_type32_enabled(bool)` wired
from `config.extensions.share_accounting`, which validation already forces to
`false` for v2); when disabled, a type-32 frame gets a definite
`Type32Message::Error` (code such as `unsupported-extension`) or the session is
closed, and the v2 front does not populate `Type32Store`. Tests: the
main-process v2 peer sends `GetWindow` and receives the error; a v1
configuration still gets `GetWindowSuccess`; `e2e_type32.rs` unchanged.

### F2 - BLOCKING - guard ordering in `Front::refresh` (new fallible calls after mutation)

`refresh` rolls the open slice, generates the send key, updates `height`,
opens the new slice and clears `seen_shares` (`front.rs:625-637`) *before* the
LP-1 calls that can now fail: `job_work` (`front.rs:644-646`),
`window_target` (`654-656`), `select_window_v2` (`662-668`),
`raw_payouts_v2` (`688-690`) and the second `job_work` (`768`). On an error the
function returns early, `self.job`/`self.policy`/`self.template` keep the
previous generation, and `main.rs:950-953` logs and continues with that job.

Counterexample (probe 7, `Fable-LP1-probe-final.log`): profile `R = 2^254 - 1`
(the L3 negative vector), template `0x200fffff` -> job A at height 800000;
chain moves to `0x207fffff` -> `refresh` fails with
`TargetProfileUnplannable`; job A is still `front.job()`; two generation-A
shares are then accepted (`Ok(None)`) and sealed into a slice recorded as
`height 800001, prev_hash 0x22..., n_shares 2`, i.e. work hashed on the old
tip is booked under the new tip's slice identity and `ref_job_id`.

Reachability today requires a profile whose `R` is below the node's real pow
limit, which `validate_pow_limit` prevents only while `pow_limit_bits` is the
default (see F3). It is nevertheless a fail-open path introduced by this
packet and contradicts "guards before data mutation".

Acceptance: compute `preview_work`, `window_target` and the v2 window/payout
inputs before the first mutation (hoist them to just after `template_txids`),
or on any error after mutation clear `self.job`/`self.policy` so no share is
accepted until a refresh succeeds. Test: a refresh failure after a chain move
leaves `front.job()` unusable (`NoJob`/`StaleJob`) and no slice with the new
height receives old-generation shares.

### F3 - Medium - pow-limit validity uses an operator-supplied constant

`config.rs:216-217, 404-406` default `pow_limit_bits = 0x207fffff` and
accept any override. v2 is regtest-only, so the default is the actual
consensus value, but a wrong override makes `R >= T_powlimit` vacuous (e.g.
`0x1d00ffff` admits `R = 2^254 - 1`) and leads straight to F2 at the first
pow-limit template. The L3 contract words this as the network's *effective*
pow limit.

Acceptance: when `pool.network = regtest`, reject `pow_limit_bits` that is not
the regtest consensus limit (`bitcoin::Network::Regtest` params or the pinned
`0x207fffff`), or source it from node introspection when L2 lands; test the
`0x1d00ffff` + `2^254 - 1` combination is rejected at config time.

### F4 - Medium - payout-side v2 restoration is saturating

`payout.rs:880-884` (`slice_agg`) accumulates `d`, `df` and `diff_fee_sum`
with `saturating_add` for every row, while the front's twin
`slice_agg_from_row` (`front.rs:1201-1205`) uses the checked helpers for v2.
The decision (item 3) requires checked restoration for v2. Unreachable under
the profile bounds, but it is the one v2 aggregation path on the settlement
side that is not checked, and it does not compare stored `difficulty` with
the credit sum as the front does.

Acceptance: dispatch on `row.work_unit_version` to
`checked_add_window_work`/`checked_add_fee_work`, map to
`ReconstructError::WorkUnit`, and reject a stored-difficulty/credit-sum
disagreement for v2; unit test on a synthetic overflow row.

### F5 - Medium (docs) - WIRE-FORMATS field tables not updated

`docs/WIRE-FORMATS.md` section 3.1 gained a prose paragraph, but the byte
tables in 3.1 (`spc:blocks`) and 3.6 (`spc:slices`) still end at `entries`
and `credits`, and the `window_target` comment still says "the exact N x d".
The decision's merge note says the definite fields are appended in 3.1/3.6.
Add `U8 work_unit_version`, `B0_32 work_unit_fingerprint` (3.6) and the same
plus `U8 window_rule` (3.1) to the tables with the enum codes and the v1 zero
sentinel, and qualify `window_target` as `N * d` (rule 0) or `Q(T)` (rule 1).

### F6 - Low - DB does not pin fingerprint shape

Migration 0005 has no `octet_length(work_unit_fingerprint) = 32` and no
`<> zero` constraint. Rust reads fail closed on length (`id32`) and the wire
rejects a zero v2 fingerprint, so this is defence in depth only. Suggested:
`CHECK (work_unit_fingerprint IS NULL OR (octet_length(...) = 32 AND
work_unit_fingerprint <> decode(repeat('00', 32), 'hex')))`.

### F7 - Low - `accounted` omits `share_hash` and `extranonce`

`evidence.rs:60-78` logs nonce/ntime/version/share_index but not the hash or
the extranonce. Both are recoverable (extranonce in the slice `shares_blob`
leaf, hash from the runner's job record), so the contract's oracle recipe
still works, but adding `share_hash` (and `extranonce`) would let the oracle
verify `hash <= share_target` without the runner's copy. Not a contract
violation.

### F8 - Low - metric assertions weakened without a delta

`crates/spc-store/tests/postgres.rs:1171-1186` moved from exact counts to
`>= 1`. The reason (one shared `SPC_TEST_DATABASE_URL`, new `work_units.rs`
binary) is real, but exactness can be kept by reading the counts before the
fixture and asserting the delta. Production code is untouched.

### F9 - Low - error naming nits

`raw_payouts_v2` reports `WorkUnitRestoreMismatch` with the *first* slice id
rather than the offending one (`lib.rs:431-437`); the per-identity fee sum
reuses `SliceFeeWorkOverflow` and the empty-window case reuses
`WindowTargetZero`; `profile_json` reports the regtest `pow_limit_bits` for a
v1 pool on any network. All fail closed; names only.

### F10 - Info

`apply_bootstrap` filters other fronts' rows by `slice_front_id` before the
version check (single-front scope, payout's `select_window_v2` would still
reject a mixed window). `generation` is seeded from `unix_ms()` at start-up;
monotonic within a run, and across restarts unless more generations than
elapsed milliseconds were issued. `with_share_difficulty` on `Sv2Server` is
now dead for the job path (only `announced_share_target` uses the legacy
function); harmless.

## 4. Independent probes (not the candidate's tests)

Probe crate `/tmp/opencode/Fable-LP1-probe` (`tests/probe.rs` SHA-256
`64aefe7b027e9778c5cadc39240b6f68b6fc191675c089046d05190899c3d86e`,
`Cargo.toml` `fc4776ac...73f5`), path-depending on the candidate crates,
built and run under `nix develop` with `CARGO_BUILD_JOBS=2`, log
`/tmp/opencode/Fable-LP1-probe-final.log`: **8 passed, 0 failed**.

1. `probe_generation_retarget_stepdown_stepup_and_frozen_q` - fully hashed
   v2 shares; stale generation-A share after B rejected; B at the pow limit
   is block-qualified with `d = 1`, Q 9; C steps back up to `m = 2`, Q 65; the
   B block event is not rewritten.
2. `probe_settlement_follows_frozen_q_not_slices_or_config` - same rows,
   frozen Q 9 selects `[3]`, Q 65 selects `[3, 2]`; missing v2 target,
   mixed-version slice, wrong fingerprint and v2+rule 0 each fail closed.
3. `probe_v2_wide_products_exact_and_v1_saturation_unchanged` - described
   in section 2.
4. `probe_restore_mismatch_all_directions` - described in section 2.
5. `probe_wire_enums_and_sentinel` - described in section 2.
6. `probe_legacy_v1_unchanged` - described in section 2.
7. `probe_refresh_failure_after_mutation_keeps_old_job_live` - the F2
   counterexample (passes, i.e. demonstrates the defect).
8. `probe_q_vectors_cap_and_zero_n` - Q 65 / 9, `WindowTargetTooLarge
   {computed: Some(65), cap: 64}`, `WindowNZero`, hard-cap rejection,
   `SharePolicy::window_target(_, 0)` error, zero network target rejected.

Targeted reruns of the candidate's own suites (same shell, jobs 2):
`Fable-LP1-rerun-libs.log` - spc-accounting 64, spc-pool 215, spc-store 10
passed; `Fable-LP1-rerun-pipeline.log` - `main_sv2_pipeline` 1 and
`sv2_conformance` 4 passed. `python3 scripts/check-lab-docs.py` -> `OK: 6
files, ASCII clean, links resolve, vectors pass` (expected absent merge
inputs reported, not hidden). `git diff --check` clean. The Postgres suite was
not re-run by me; `LP1-db-tests-9.log` is consistent and `work_units.rs` is
the real-database proof of the CHECKs.

## 5. Evidence boundary (what this review does and does not establish)

Established: the bounded LP-1 code path against a mock template and a real
Noise/SV2 peer in the real `spc-pool --roles front` process, genuine SHA256d
non-block and block-qualified shares at `m = 2`, exact booked `d` in the ACK,
`job_issued`/`accounted` evidence, frozen Q in the block event.

Not established, and not claimed by the candidate: node-canonical acceptance
of a v2-built block, settlement through Redis/Postgres end to end on v2 rows,
TDP/ladder interplay, multi-transaction templates, any CPU-driver or runner
behaviour (LP2/LP3), the v2 type-32 audit (explicitly unsupported; F1 makes
the current state worse than "unsupported"), multi-front restore.

## 6. Still-open full-node / integration conditions (separate from F1-F10)

1. Real regtest node (`bitcoind` or `rbitcoin-node`) accepting a block mined
   under a v2 job, with the `spc:blocks` event settling through payout on
   migration 0005 and the ledger reconstructing from the frozen Q.
2. Retarget observed on a live chain between distribution build and
   settlement (window decision test 7) with persisted rows, not in-process.
3. LP2 runner consuming `job_issued`/`accounted` plus `share_outcome_unknown`
   and reconciling a timed-out submission out of line.
4. LP3 oracle recomputing `d` from `job_issued` plus the share hash against
   `slice_credit.diff_sum` on real rows, reporting `unknown` on a missing job.
5. TDP (`sv2-tp`) and ladder branches merged with L1's deepest-first Merkle
   path and their own acceptance tests; multi-transaction templates.
6. `docs/SPEC.md` 3.4 / 4.1-4.4 / 5.1 / 5.3 / 7.4, `docs/OPERATIONS.md`
   (profile restrictions, atomic front/payout deploy) and
   `docs/share-accounting-ext-deltas.md` (v2 type-32 source note) at merge.
7. Combined Nix vendor/flake gate for the `channels_sv2` dev-dependency and
   lockfile row (no pin bump, but the report asks for the flake check rather
   than inference from the dev shell).

## 7. Re-review scope

Fix F1 and F2 (and preferably F3-F5) in `lab-share-integration`, regenerate
the manifest with the same recipe, and re-run: `cargo test -p spc-pool --lib
--test main_sv2_pipeline --test sv2_conformance --test e2e_type32`, the
Postgres suite against a throwaway database, clippy with `-D warnings`, fmt,
`scripts/check-lab-docs.py`, and the probe crate above (probe 7 must then
fail at its first `COUNTEREXAMPLE` assertion or be rewritten to assert the
fail-closed behaviour). I will confine the next pass to the F1-F5 diff plus
any file whose hash moves.

---

# Second pass (revision r2) - 2026-11-05

Reviewer: Fable (subagent). Same rules: no candidate edits, no commits, no
subagents; this appended section is the only artifact written into a tree.
Probe sources and logs: `/tmp/opencode/Fable-LP1-probe`, `/tmp/opencode/Fable-LP1-r2-*`.

## Verdict (r2)

**REQUEST_CHANGES - focused on the new retention amendment only.**

Every first-pass finding F1-F10 is resolved in code and covered by the
candidate's regressions and by my re-run probes (section r2.2). The LP-1 core
(version matrix, generation authority, frozen Q, checked v2 accounting,
persistence, wire, settlement, evidence, type-32 refusal, fail-closed refresh)
is approvable as it stands. The revision also adds a new normative amendment
(decision item 10: v2 work-aware retention, 4096 slices / 4096 heights). That
amendment is implemented in a way that does not meet two of the review
conditions set for it, both cheap to fix:

- **R2-1 (blocking for the amendment)** - durable credit scrub is gated by the
  *payout's configured* `work_unit_version`, not by the slice row's version.
- **R2-2 (blocking for the amendment)** - the live v2 retention budget is the
  *current* Q and the chain-move roll trims with the *previous* template's Q,
  so an upward retarget leaves history the new Q needs out of the live window.

If the amendment were withdrawn (restoring the 16-slice behaviour with the
truthful underfill it already produced) the remaining packet would be
APPROVE-bounded. Re-review is confined to the two items plus any file whose
hash moves.

## r2.1 Snapshot

Same worktree, HEAD `b0ab4ca3...678c`. Manifest
`/tmp/opencode/lab-lp1-review-manifest.json` (file SHA-256
`ed3f0e16f756a0ded9d64b3c9fcaf9ece3bfccdeb28e1c09031c2fa2cc375052`, copied to
`/tmp/opencode/Fable-LP1-r2-manifest-copy.json`), 57 files, summary 28/10/19.
Aggregate recomputed over `base` plus `path`/`sha256` only:

```
69ac72b6ed7ab514f1870cfc55832d8c8d202576a9c01bc27aaeb72d2a20da6d   (matches)
```

All 57 files re-hashed from disk before and after my probe runs: 0 mismatches;
`git status --porcelain -uall` shows nothing outside the manifest. Every
`baseline_sha256` still equals the approved L1/LP1a/L3 hash (the three L3
decision/changelog files and `REGTEST-LAB-IMPLEMENTATION.md` are `unchanged`),
so the approved inputs were not rewritten; the retention policy is an explicit
amendment in the LP-1 decision, as requested. Files changed since the r1
snapshot: `api_backend.rs`, `config.rs`, `evidence.rs`, `front.rs`, `main.rs`,
`payout.rs`, `sv1_server.rs`, `sv2_server.rs`, `tests/main_sv2_pipeline.rs`,
`spc-accounting/src/{lib.rs,v2_integration_tests.rs,work_unit.rs}`,
`0005_lab_work_units.sql`, `spc-store/tests/{postgres.rs,work_units.rs}`,
`WIRE-FORMATS.md`, the LP-1 decision/changelog/report, and new `flake.nix`.
`work_unit.rs` differs from the approved LP1a module only by the serde derives
and two additive error variants (`PayoutWindowEmpty`,
`WorkUnitRestoreMismatch`); no arithmetic line changed.

Flake-check source association: `/tmp/opencode/LP1-r2-flake-source-path.txt`
names two copies; the later one (`LP1-r2-flake-src-VoHj8L`, 22:12) matches the
final snapshot for every file except the implementation report itself
(written afterwards). `LP1-r2-flake-check-final.log` (22:29) lists
`checks.x86_64-linux.{fmt,clippy,doc,test,test-ark}` and ends without error.

## r2.2 Disposition of F1-F10

| Finding | Verified fix (file:line) | Evidence |
|---|---|---|
| F1 type-32 | `Sv2Server::with_type32_enabled` (`sv2_server.rs:184`), router replies `Type32Message::Error{code:"unsupported-extension"}` with the request id before any service path (`sv2_server.rs:550-566`); `roll_slice` feeds `Type32Store` only when enabled **and** v1 (`front.rs:1062`); `main.rs:301,342` wire `extensions.share_accounting`. | Main-process v2 peer sends all three requests and checks the code (`main_sv2_pipeline.rs:353-393`); `e2e_type32.rs` unchanged and passing; retention test asserts an empty store. |
| F2 refresh | Deterministic planning hoisted ahead of roll/key/height/open/replay (`front.rs:651-659`); any `refresh_inner` error calls `invalidate_current_work` (`front.rs:620-640`); `main.rs:959-968` clears SV1/SV2 `current` so ingress answers `invalid-job-id`. | Candidate regression `failed_retarget_never_admits_old_generation_work_on_the_new_tip` (red log `LP1-r2-red-f2.log`); my probe 7 rewritten to assert the new behaviour (section r2.4). |
| F3 pow limit | v2 requires exactly `0x207fffff` (`config.rs:795-801`); v1 `profile_json.pow_limit_bits` is null. | Config test with `0x1d00ffff` + `2^254-1`. |
| F4 payout restore | `slice_agg` dispatches v2 through checked helpers and requires stored difficulty == credit sum (`payout.rs:893-917`, `StoredDifficultyMismatch`). | Candidate unit tests; my probe 2 unchanged. |
| F5 WIRE tables | 3.1 and 3.6 byte tables carry the appended fields and codes; `window_target` qualified per rule. | Read. |
| F6 SQL fingerprint | `octet_length = 32 AND <> zero` CHECKs on both tables (migration 0005). | `work_units.rs` 31-byte and zero writes. |
| F7 evidence | `accounted` adds raw `share_hash`, full `extranonce` and byte-order notes (`evidence.rs:60-85`). | Main-process test asserts both. |
| F8 metrics | `isolated_store` creates and drops a per-test database; exact counts (`tests/postgres.rs:24-46,1168`). Note: the test skips silently when `CREATE DATABASE` is not permitted; `LP1-r2-db-tests.log` shows it ran (`... ok`, no `skip:`). | Log. |
| F9 names | Offending slice id reported; `PayoutWindowEmpty`; per-identity fee aggregation maps to `PayoutProductOverflow`. | Read. |
| F10 info | Addressed by the retention amendment (see r2.3). | - |

Re-runs by me (jobs 2, `nix develop`): `Fable-LP1-r2-rerun-pool.log` -
spc-pool lib 220, `e2e_type32` 1, `main_sv2_pipeline` 1, `sv2_conformance` 4
passed; `Fable-LP1-r2-rerun-libs.log` - spc-accounting 65, spc-store 10
passed. Candidate logs read: workspace 489 passed over 38 result lines
(`LP1-r2-workspace-green.log`), targeted 220+1+1+4, Postgres 20+1 on a fresh
database, clippy clean, fmt clean, doc check OK, legacy SV1 regtest OK. The
mock-template main-process test remains protocol/accounting evidence only, not
node-canonical acceptance.

## r2.3 Findings on the retention amendment

### R2-1 - BLOCKING - scrub retention is config-gated, not row-version-gated

`payout.rs:1890-1906` chooses `retain = max(configured, 4096)` when the
*payout role's* `ScrubConfig.work_unit_version` is V2, else the legacy value,
and `Store::scrub` (`postgres.rs:594-600`) then deletes
`slice_credit ... WHERE s.height < $1` for every row regardless of
`s.work_unit_version`. Consequences:

- A v1-configured payout (the decision's own item 4 allows a payout with a
  different configured profile to settle history) scrubs v2 credits at the
  legacy horizon; a later v2 settlement whose 4096-slice walk needs them fails
  closed (`NoSlices`/`WindowTruncated`). History the amendment promises to keep
  is deleted.
- A v2-configured payout keeps **v1** rows' identity-bearing credits for at
  least 4096 heights as well, contradicting decision item 10 ("Legacy v1 tail
  and durable scrub retention stay unchanged") and extending SPEC §5.1
  attribution retention for legacy rows without disclosure.

The Ark boundary is unaffected (`slice_credit` never leaves SPc Postgres), but
the SPc-side identity retention change must be truthful and row-scoped.

Acceptance: gate the deletion by row version in SQL, e.g.
`DELETE ... WHERE (s.work_unit_version = 1 AND s.height < $v1_below) OR
(s.work_unit_version = 2 AND s.height < $v2_below)`, with both horizons
computed from the same tip; keep `ScrubConfig` free of a version field or use
it only for the v2 horizon. Test on real Postgres: mixed v1/v2 rows, one scrub,
v1 credits removed at the legacy horizon, v2 credits retained to 4096 heights,
under both payout configurations. Qualify decision item 10 and the changelog
fragment accordingly (disclose the v2 identity-retention extension vs SCRUB).

### R2-2 - BLOCKING - live v2 retention budget is the current Q, and the roll trims with the previous Q

`trim_retained_slices` (`front.rs:1106-1126`) keeps the legacy minimum and
then whole slices until cumulative work reaches `self.window_target`. Two
problems:

1. Ordering: in `refresh_inner` the chain-move `roll_slice` (`front.rs:679`)
   runs before `self.window_target = window_target` (`front.rs:703`), contrary
   to the comment at 700-702, so the roll's trim uses the previous template's
   Q. One roll per retarget is budgeted against the stale value.
2. Budget: even with the ordering fixed, trimming to the *current* Q means an
   upward retarget (T harder, Q larger) finds the live tail holding only the
   old Q's worth of history although the durable store (and the frozen
   contract, which measures the window at the finding job's Q) has more. The
   front never re-reads the store after bootstrap.

Counterexample (probe 10, `Fable-LP1-r2-probe-run2.log`): 70 single-share
`d = 1` slices at `0x207fffff` (Q 9) leave 16 retained; after one retarget to
`0x200fffff` (Q 65) the live window holds 16 units, `window_underfilled() ==
true`, although 70 units were sealed and persisted. The underfill is reported
truthfully (no fabricated coverage), but history the new Q needs was dropped
from the live window, and the resulting distribution/high-water pays fewer
contributors than the frozen-Q contract describes.

Reachability: regtest bits are constant unless the planned rbitcoin retarget
profile is used, so today this is latent; it will bite the first retarget
experiment the lab exists to run.

Acceptance: set `self.window_target` before the chain-move roll; size the v2
live budget to the hard cap or to a documented multiple of Q rather than the
current Q (4096 `SliceAgg` is a small memory cost), or re-read the bounded
tail from the store when Q grows; add a regression that raises Q across a
retarget and asserts the live window reaches the new Q when >= Q units exist
in history. If the budget is intentionally kept at current Q, decision item 10
must disclose that an upward retarget produces underfill until new slices
accumulate.

### R2-3 - Low (keep as documented limits)

- Bootstrap with v2 preserves up to 4096 restored rows until the first refresh
  applies the budget (`front.rs:558-563`); fine, and probe 9 shows 40 rows ->
  33 retained after refresh with Q 65.
- `store.bootstrap(retain)` receives 4096 (slice count) for v2 and
  `retain_windows` for v1 (`main.rs:719-724`); the parameter's unit (slices)
  is pre-existing.
- `isolated_store` tests return `None` (vacuous pass) without
  `CREATE DATABASE` privilege; the harness log shows they ran, but a future CI
  without the privilege would silently lose the exact-metric coverage.

## r2.4 Independent probes (r2)

`tests/probe.rs` SHA-256
`724352d17ae926509dc926a06ac1658aedbbfe50797705ddadce7547df946d06`, log
`/tmp/opencode/Fable-LP1-r2-probe-run2.log`: **11 passed, 0 failed**.

- Probes 1-6 and 8 from the first pass unchanged and still passing (generation
  immutability and frozen Q; settlement from frozen Q only; wide products and
  v1 saturation; restore mismatch all directions; wire enums/sentinel; legacy
  v1 unchanged; Q vectors/caps).
- Probe 7 rewritten to assert the *correct* behaviour: after a planning
  failure no roll happens, `front.job()` is `None`, an old-generation share is
  `NoJob`, no slice event is emitted, and the next good template seals the
  pre-failure share under its own height and issues a strictly newer
  generation. Passes.
- Probe 9 (new): 40 live single-share `d = 2` slices across 40 height
  advances at Q 65 retain exactly 33 whole slices (`ceil(65/2)`), >= 65 work,
  not underfilled; the next block event carries Q 65 and a high-water equal to
  the newest sealed slice. Bootstrap variant: 40 rows -> 33 retained, not
  underfilled, high-water 40; 20 rows -> truthful underfill with all 20 kept.
- Probe 10 (new): the R2-2 counterexample above (passes, i.e. demonstrates the
  gap).

## r2.5 Still-open conditions (unchanged from the first pass)

Items 1-7 of section 6 remain: real-node v2 acceptance and Redis/Postgres v2
settlement, live retarget between build and settlement, LP2 reconciliation,
LP3 oracle on real rows, TDP/ladder merge with deepest-first Merkle,
SPEC/OPERATIONS/share-accounting-ext-deltas edits at merge (now including the
qualified retention amendment), and the combined flake gate on the merged
tree. The mock-template main-process proof is not node-canonical acceptance.

## r2.6 Re-review scope

Fix R2-1 and R2-2 in `lab-share-integration`, regenerate the manifest with the
same recipe, and re-run `cargo test -p spc-pool --lib --test main_sv2_pipeline
--test e2e_type32 --test sv2_conformance`, the Postgres suite on a throwaway
database (including the new mixed-version scrub test), clippy `-D warnings`,
fmt, `scripts/check-lab-docs.py`, and the probe crate (probe 10 should then
fail at its `COUNTEREXAMPLE if false` assertions or be rewritten to assert the
new behaviour). I will confine the third pass to those two items plus any file
whose hash moves.

---

# Third pass (revision r3) - 2026-11-05

Reviewer: Fable (subagent). Same rules: no candidate edits, no commits, no
subagents; this appended section is the only artifact written into a tree.
Probe sources and logs: `/tmp/opencode/Fable-LP1-probe`, `/tmp/opencode/Fable-LP1-r3-*`.

## Verdict (r3)

**APPROVE - bounded LP-1.** R2-1 and R2-2 are resolved as specified, with
real-Postgres boundary/idempotency evidence for the row-version-gated scrub and
in-process regressions (candidate's and mine) for the Q-independent live tail,
the age horizon and the hard cap. No regression was found in the F1-F10 fixes
or the LP-1 core. The approval covers the code, persistence, wire and protocol
behaviour of the snapshot below, on the regtest-only v2 profile, with the
evidence boundary stated in r3.5. It does not approve, and the candidate does
not claim, node-canonical acceptance of a v2 block; that proof belongs to the
separate live-agent packet.

## r3.1 Snapshot

Same worktree, HEAD `b0ab4ca3...678c`. Manifest
`/tmp/opencode/lab-lp1-review-manifest.json` (file SHA-256
`5b0f2fcdb82e7ad5b2fdb11a2ee91c50615f3a528546be16dd71b2f5666f7188`, copied to
`/tmp/opencode/Fable-LP1-r3-manifest-copy.json`), 57 files, 28/10/19.
Aggregate recomputed over `base` plus `path`/`sha256` only:

```
c16032a78902e88b0157efed6780bc8b424e942b1c3bf46893d8fb09ae4708ee   (matches)
```

All 57 files re-hashed before and after my runs: 0 mismatches; nothing outside
the manifest in `git status --porcelain -uall`. Every row marked `unchanged`
still equals its approved L1/LP1a/L3 `baseline_sha256` (no baseline drift; the
L3 decisions are byte-identical). Files changed since the r2 snapshot, exactly
the R2-1/R2-2 surface: `front.rs`, `main.rs`, `payout.rs`,
`spc-store/src/{models.rs,postgres.rs}`, `spc-store/tests/{postgres.rs,
work_units.rs}`, the LP-1 decision, changelog fragment and report. No
dependency or lockfile change; `flake.nix` unchanged from r2. The flake-check
source copy named in `LP1-r3-flake-source-path.txt` matches every snapshot file
except the implementation report (written afterwards), and
`LP1-r3-flake-check.log` lists all five `checks.x86_64-linux.*` derivations.

## r3.2 R2-1 - row-version-gated scrub: resolved

- `ScrubConfig` has no work-unit version field (`payout.rs:994-997`);
  `main.rs:202-205` passes only `retain_windows`/`scrub_after_blocks`.
- `Store::scrub(tip, retain_v1, retain_v2, scrub_after)`
  (`postgres.rs:585-608`) computes both horizons from the same tip and deletes
  `slice_credit` with
  `(s.work_unit_version = 1 AND s.height < $v1) OR (s.work_unit_version = 2 AND s.height < $v2)`.
  A row at exactly `tip - horizon` is retained; an unknown version matches
  neither arm and is left alone (fail closed).
- `scrub_once` (`payout.rs:1890-1902`) passes the legacy
  `scrub_retain_windows(...)` for v1 and the constant
  `V2_RETENTION_HEIGHT_BLOCKS = 4096` (`models.rs:10`) for v2, so a payout
  configured either way neither erases v2 credits early nor extends v1
  attribution retention.
- Real Postgres test `mixed_version_scrub_uses_stored_row_horizons_in_either_payout_mode`
  (`work_units.rs:165-247`): v1 and v2 rows at heights 5903/5904/6000/9983/9984,
  tip 10000 -> v2 keeps `>= 5904`, v1 keeps `>= 9984`; run twice for
  idempotency. Failure-first log `LP1-r3-red-mixed-scrub.log`, green in
  `LP1-r3-db-tests.log` (20 + 2 passed, isolated metrics test ran, no `skip:`).
- Decision item 10 now discloses the v2-only SPc-side attribution-retention
  extension vs the SCRUB horizon and states that v1 retention and the Ark
  boundary are unchanged. The changelog fragment matches.

## r3.3 R2-2 - Q-independent live tail with age horizon: resolved

- `trim_retained_slices(tip_height)` (`front.rs:1111-1119`) for v2 keeps every
  slice with `height >= template_height - 4096`, then truncates to the
  4096-slice hard cap; it no longer reads Q. v1 keeps `keep_slices` unchanged.
- `self.window_target = window_target` now precedes the chain-move roll
  (`front.rs:662` vs `:682`), closing the ordering nit even though the trim no
  longer depends on it.
- Off-by-one against the SQL horizon: the front is called with the template
  height `H = tip + 1`, so RAM keeps `height >= tip - 4095`, while the scrub at
  the same tip keeps `height >= tip - 4096`. RAM is a strict subset of the
  durable set by one block; a row the SQL has pruned is never usable in RAM.
  (The decision text says "the same `height >= tip - 4096` cutoff"; the code is
  one block stricter, which is the safe direction. Documentation nit only.)
- Bootstrap keeps up to 4096 restored rows until the first refresh applies the
  age cutoff (`front.rs:558-563`, `main.rs:719-724` fetches at most 4096 for
  v2; v1 fetch unchanged).
- Candidate regressions (red/green logs listed in the report):
  `v2_roll_retention_preserves_headroom_for_q9_to_q65_retarget` and the
  bootstrap twin (70 `d = 1` slices at Q 9, retarget to Q 65 -> `>= 65` slices,
  not underfilled), `v2_age_horizon_excludes_expired_credits_and_reports_underfill`
  (70 rows at height 100 + 1 at 4999 with template 5000 -> 1 slice, underfilled),
  `v2_slice_cap_is_bounded_and_history_exhaustion_is_truthful` (4097 rows ->
  4096 retained, a 4097 target is underfilled). The earlier 40-slice / Q 65
  roll and bootstrap regressions remain.
- Frozen snapshots and v1 semantics: `window_target`/`high_water` in block
  events are unchanged in meaning; settlement still reads only the frozen
  snapshot (probe 2 unchanged and passing); v1 `select_window`/`raw_payouts`/
  tail/scrub are untouched (probe 6 unchanged and passing).

## r3.4 Independent probes (r3)

`tests/probe.rs` SHA-256
`b52403f54e1dcb8e59903d30cd67d23594f1c3ccaed2d346ebfd3d73da22f7d7`, log
`/tmp/opencode/Fable-LP1-r3-probe-final.log`: **12 passed, 0 failed**.

- Probes 1-8 (first pass) and the r2 fail-closed-refresh probe: unchanged
  except one fixture height moved inside the age horizon (a row at height 1
  under template 800000 is now, correctly, age-expired and excluded from the
  window), still passing.
- Probe 9 (r2, updated to r3 semantics): 40 live single-share `d = 2` slices
  across 40 height advances -> all 40 retained, `>= 65` work, not underfilled,
  block event Q 65 with high-water at the newest sealed slice; bootstrap twin
  40 rows -> 40 retained, high-water 40; 20 rows -> truthful underfill.
- Probe 10 (r2 counterexample, now asserting the fix): 70 `d = 1` slices at
  Q 9 -> 70 retained (nothing pruned); after the retarget to Q 65 -> 71 slices,
  71 units, `window_underfilled() == false`.
- Probe 11 (new): rows at heights `H-4098..H-4094` with template height `H`:
  front keeps exactly `H-4094, H-4095, H-4096`, all `>= (H-1) - 4096` (inside
  the SQL set); 60 < 65 -> underfilled; after one chain move to `H+1` the
  `H-4096` row ages out and the new `H` slice is retained.

Regression re-run by me (jobs 2, `nix develop`, `Fable-LP1-r3-rerun.log`):
spc-pool lib 224, `e2e_type32` 1, `main_sv2_pipeline` 1, spc-store lib 10
passed. Candidate logs read and consistent: targeted 224+1+1+4 = 230
(`LP1-r3-targeted-final.log`), workspace 494 over 38 result lines
(`LP1-r3-workspace-final.log`), Postgres 20 + 2, clippy/fmt clean, doc check
OK, legacy SV1 regtest OK, five flake checks. Old probes that assumed the
Q-sized tail are superseded by the r3 versions above, not presented as a green
legacy suite.

## r3.5 Approval boundary and remaining conditions

Approved: the LP-1 v2 share-policy integration as snapshotted: regtest-only
opt-in v2 profile with pinned pow limit; immutable per-generation
target/credit with O(1) stepdown/up; checked v2 accounting with exact wide
products and frozen v1; frozen version/fingerprint/rule/Q persistence,
migration 0005 with shape CHECKs, definite wire fields; settlement from the
frozen snapshot only; fail-closed refresh with ingress invalidation; type-32
refused at the SV2 router for v2; disjoint SV1/SV2 ids; exact booked `d` in
ACKs; `job_issued`/`accounted`/`share_outcome_unknown` evidence on the SPc side
only; row-version-gated scrub; Q-independent bounded live retention with
truthful underfill.

Not established here and not claimed by the candidate (unchanged list):

1. Node-canonical acceptance of a v2-built block and end-to-end v2
   Redis/Postgres settlement on a real regtest node - the LP live-agent packet.
2. Live retarget between build and settlement on a real chain (rbitcoin
   retarget profile).
3. LP2 runner/LP3 oracle consumption of the evidence contract, including
   `share_outcome_unknown` reconciliation and `unknown` on missing job records.
4. TDP (`sv2-tp`) and ladder merge with L1's deepest-first Merkle path;
   multi-transaction templates.
5. Merge-session edits: SPEC 3.4/4.1-4.4/5.1/5.3/7.4 (including the retention
   amendment and the one-block RAM/SQL horizon offset), WIRE 3.1/3.6 as now
   written, OPERATIONS (profile, deploy atomicity, retention/privacy
   disclosure, authority-secret storage), `share-accounting-ext-deltas.md`
   v2 source note, migration-number re-check at merge.
6. Combined flake gate on the merged tree.

Low notes carried forward, none blocking: `isolated_store` Postgres tests
skip silently without `CREATE DATABASE` privilege (the harness log shows they
ran); the decision's "same cutoff" wording vs the one-block-stricter RAM
cutoff; `profile_json` and example config still document only v1.
