# 2026-11-17 — Denomination-ladder implementation audit

Packet: `wp-ladder-audit` (base master `3c2df0d`). Scope: verify the
2026-11-03 ladder decision (`docs/decisions/2026-11-03-banded-amounts-with-ark-overage.md`,
landed `d62bf23`, record commit `7be2541`) against master's code, and
reconcile the stale `docs/STATUS.md` Remaining item 1 ("no code yet") and the
old `ladder` branch (tip `df7c654`).

## Headline finding

**STATUS Remaining item 1 is stale.** The ladder is implemented on master.
It arrived through the ladder-completion packet absorbed by the source-union
integration (`7b34914` and follow-ups; evidence record
`docs/reviews/2026-11-09-ladder-completion.md`, wire amendment
`docs/decisions/2026-11-09-ladder-entry-wire-v2.md`, SPEC frontmatter entry
2026-11-09). The old `ladder` branch (commits `b0be5d8`, `51fdc50`,
`df7c654`, based on pre-union master `5a6a0d7`) is **superseded by master in
all source code** and MUST NOT be merged: it would resurrect the per-`P_{i,k}`
entry representation that the ladder-entry-v2 amendment (one logical row,
`p_x[]` key array, mandatory `owed_sats`) replaced, and delete the v2
work-unit code. Three test/harness artifacts unique to that branch had been
dropped by the union; this packet re-lands them (see "Gaps closed").

## Checklist verdict table

| # | Normative item (decision / SPEC) | Verdict | Evidence (file:line, test) |
|---|---|---|---|
| 1 | Constant band set `{500k, 1M, 3M, 5M}` sats, no era/height/subsidy input | present | `crates/spc-accounting/src/ladder.rs:4` (`LADDER_BASE_RUNGS_SATS`; `decompose` takes only `owed`); published at `/api/pool`: `bins/spc-pool/src/api_backend.rs:30-32` (`pool_template_payout_format: "fixed_ladder_v1"`, `ladder_base_rungs_sats`) |
| 2 | Greedy largest-first decomposition, repeated rungs, canonical | present | `ladder.rs:40-58` (`decompose`); tests `largest_first_canonical_vectors`, `worked_vector_and_regtest_maximum`, proptest `decomposition_properties` (canonical order, rung membership, `pay + rem == owed`, count = `decomposition_len`) |
| 3 | Rail gate `owed ≥ 500_000` on un-decomposed balances; remainder `owed mod 500k` to Ark when deliverable, never forfeited | present | `crates/spc-accounting/src/lib.rs:633-656` (`route_tiers_with_keys`: `band_amount`, `ark_due` push, pending/forfeit branches); tests `ladder_completion_tests.rs:15` (`worked_vector_groups_coinbase_rows_and_remainder_components`), `:97` (committed/unknown keys hold), `tests.rs` §4.5 fixture |
| 4 | No trim, no partial pay, no capacity deferral; over-revenue fails closed | present | `lib.rs:596-683` (no trim branch; `EntitlementExceedsRevenue` at 676-678); `ladder_completion_tests.rs:128` (`jd_is_explicit_exact_weight_and_pool_capacity_is_a_hard_failure`: `PayoutSlots`/`PayoutWeight` hard errors); `payout.rs:3592` (net-pending correction tests) |
| 5 | `max_payout_slots` policy cap 1024, 2048 representation/fee headroom; build fails closed at the cap | present | default: `bins/spc-pool/src/config.rs:436-438` (`default_max_slots() = 1024`); bound 1..=2048 enforced: `config.rs:949-953` (against `MAX_LADDER_OUTPUTS`), error `config.rs:661`; `ladder.rs:7` (`MAX_LADDER_OUTPUTS = 2048`); slot-cap errors in `lib.rs:662-668` and `lib.rs:1106-1111`; example `config/spc-pool.example.toml:67` |
| 6 | Era-1 output bound 619 (value bound `T_owed/500k`) | present (doc) / **test added** | SPEC §4.1/§4.8 (lines 580, 801); new test `ladder.rs` `era_one_value_bound_sits_under_policy_and_representation_caps` (619 ≤ 1024 ≤ 2048) |
| 7 | Settlement predicate: fail closed per identity on `Σ_k value(P_{i,k}) == owed − (owed mod 500k)`, accumulating across `k`, against the mined coinbase | present | `bins/spc-pool/src/payout.rs:697-710` (per-key accumulation), `:729-738` (`band_amount` row check + full `owed_sats` binding), `:790-810` (per-`k` `decompose` equality: value, count, per-output amount), `:812-821` (no unrepresented P2TR output), `:767-773` (pool output = `pay_p`); tests `ladder_grouped_settlement_binds_full_owed_and_witness_shape` (`payout.rs:3491`; owed+1 / swapped-k / missing-k / duplicate all reject), `raw_at_base_rung_pays_full_canonical_amount_and_retains_only_remainder` (`:5385`), `settlement_rail_rechecks_delivery_at_settlement` (`:4027`), `forfeit_confiscates_raw_and_keeps_prior_pending` (`:5324`) |
| 8 | Kept-row remainder booked `ark_due` on the coinbase ledger row; scheduled at `H + confirmation_depth`; error rail retained | present | `payout.rs:853-872` (remainder → `ark_due_sats` on the coinbase row); `crates/spc-store/src/postgres.rs:1119-1133` (schedules on `ark_due_sats > 0`, not rail); **test re-added** `crates/spc-store/tests/postgres.rs` `coinbase_remainder_schedules_ark_once_and_orphan_reverses_it` |
| 9 | Aggregation per `sp_id` below `min_overage_sats` 10 000 / `overage_max_delay_blocks` 144 | present | `crates/spc-store/src/aggregation.rs:7-8` (constants), selection tests in-module; emission `payout.rs:2602` (`emit_due_ark_orders`), `:2837` (`emit_aggregate_orders`) |
| 10 | Band-aware miner/agent own-payout check (sum over `k` equals greedy value of `owed`; ambiguity rejects; sub-base miner legitimately absent) | present | `crates/spc-scan/src/lib.rs:160-214` (`scan_outputs` full `k` association, gap/duplicate reject), `:218-253` (`audit_outputs`, `PayoutFormat::FixedLadderV1` recomputes `decompose(owed)`, exact per-`k` amounts, returns remainder); `bins/spc-agent/src/verify.rs:59-73` (`verify_entitlement`), `:44-54` (source-bound variant); refuse-without-audit-feed: `bins/spc-agent/src/translator.rs:84-102`; `require_own_payout` warn/refuse: `bins/spc-agent/src/config.rs:39,328,461`; scan tests at `spc-scan/src/lib.rs:622-704` (swapped/missing composition rejects; 990-output recovery) |
| 11 | JD mode stays exact-`owed` weights (shaping pool-template-only, O20) | present | `lib.rs` `route_jd_tiers` (explicit separate path; doc at 685-719); `ladder_completion_tests.rs:128` (JD weight = exact owed 9 500 123 vs pool kept 9 500 000); JD settlement unshaped: `payout.rs:823-842` |
| 12 | `C` retained as JD floor and rung sanity check (`rung_min ≥ C ≥ dust`) | present | `ladder.rs:17-22` (`validate_ladder`: `dust ≤ C ≤ 500k`), called from `lib.rs:603`; `config.rs:954-956` (`C ≥ dust`) |
| 13 | Float sizing guidance (94-block lag law) and `float_target_sats` | present (doc + knob) | SPEC §11.3 (line 1516); `config.rs:337,1055-1056` (`float_target_sats`, required when ark enabled); `docs/OPERATIONS.md:129,408` |
| 14 | No era scaling / purity statement in docs | present | SPEC §4.4 (line 701), §4.1 table (578-580); `DESIGN.md:60-78`; `docs/WIRE-FORMATS.md:55-71` (`p_x[]`, no ladder fields) |
| 15 | Banded e2e shape assertion (regtest) | **re-added** | `testing/regtest-services-e2e.sh` (990 × 5M-rung outputs, sum 4.95 BTC, rung-set membership; value-bearing outputs only so zero-value commitment outputs don't false-fail) |

## Gaps closed on `wp-ladder-audit`

All three are artifacts the old `ladder` branch (commit `51fdc50`, `df7c654`)
had and the source union dropped; master's runtime behavior already matched,
only the tests/harness were missing:

1. `crates/spc-store/tests/postgres.rs` —
   `coinbase_remainder_schedules_ark_once_and_orphan_reverses_it`: a
   coinbase-rail row with `ark_due_sats > 0` schedules exactly one
   `ark_payout` at `H+6` and orphaning removes it. No existing test exercised
   the `ark_due_sats > 0` (rather than `rail == Ark`) scheduling condition at
   `postgres.rs:1119`, which is exactly the hunk `51fdc50` changed.
2. `testing/regtest-services-e2e.sh` — the found-block banded-contract
   assertion (990 × 5 000 000-sat rungs, paid sum 4 950 000 000). Adapted
   from `51fdc50` to ignore zero-value (witness-commitment) outputs, which
   master can emit and the old branch predated.
3. `crates/spc-accounting/src/ladder.rs` — era-1 bound test
   `era_one_value_bound_sits_under_policy_and_representation_caps`
   (619 ≤ 1024 ≤ 2048). Replaces the old branch's
   `payout_slots_cover_the_era_one_ladder_bound` config test; placed in the
   packet-owned accounting crate because `bins/spc-pool/src/config.rs` is
   merge-session-owned (`docs/MERGE-QUEUE.md`), and the config half of that
   old test (default = 1024, 1..=2048 validation) is already live code at
   `config.rs:436-438,949-953`.

## Pre-existing master breakage found and fixed by the gate

`bins/spc-agent/src/config.rs` test
`aggregate_api_fields_validate_in_default_and_ark_builds` fails on master
tip `3c2df0d`: commit `c55b737` (chain identity union) added the
`chain_network()` early `sp_address` gate to `AgentConfig::validate()`, so
setting `[ark] network = "mainnet"` on the test's `tsp` sample address now
returns `InvalidAddress` before the intended `AggregateApi` rejection is
reached. Not a ladder issue and not introduced by this packet; fixed
minimally by switching the off-regtest case to `signet` (same `tsp`
encoding, still exercises "http aggregate origin is regtest-only").

## Old `ladder` branch disposition

**Fully superseded; safe to archive/delete after this packet merges.**
Per-file:

- `crates/spc-accounting/src/ladder.rs` — master's version supersedes
  (adds `MAX_LADDER_OUTPUTS` clamp; drops `decompose_for_mode`/`LadderMode`,
  replaced by the explicit `route_jd_tiers` path; rung ≥ C check folded into
  `validate_ladder` as `C ≤ LADDER_BASE_SATS`, equivalent because 500k is the
  minimum rung).
- `crates/spc-accounting/src/lib.rs`, `tests.rs` — superseded; the old
  branch's §4.5 worked-vector routing additions exist as
  `ladder_completion_tests.rs:15` with the same numbers (67 outputs;
  remainders 491 854 / 178 612 / 4 532; `pay_p` 4 000 000).
- `bins/spc-pool/src/payout.rs` (`51fdc50`) — superseded by the
  ladder-entry-v2 representation (one logical row, `p_x[]`, full `owed_sats`,
  per-`k` checks, witness-aware fixtures); the old `group_entries` fold has
  no equivalent because the v2 wire removed the split-entry encoding it
  compensated for.
- `bins/spc-pool/src/front.rs`, `config.rs` — superseded (JD k0 map,
  budget-at-ceiling, 1024 default all on master).
- `crates/spc-store/src/postgres.rs` — the `ark_due_sats > 0` hunk is on
  master verbatim (line 1119).
- `crates/spc-store/tests/postgres.rs`, `testing/regtest-services-e2e.sh`,
  config-default test — the three dropped artifacts, re-landed here.

## Gate (this worktree)

- `cargo fmt --all -- --check`: clean.
- `cargo clippy -p spc-accounting -p spc-pool -p spc-agent --all-targets
  --features spc-pool/rpc -- -D warnings`: clean (exit 0; the only cargo
  note is the pre-existing sqlx-postgres future-incompat report).
- `cargo test -p spc-accounting -p spc-pool -p spc-agent
  --features spc-pool/rpc`: pass, all 21 test binaries, 0 failures
  (includes the new `era_one_value_bound_sits_under_policy_and_representation_caps`
  and the repaired agent config test). Environment note for the gate: the
  pre-existing `main_sv2_pipeline` integration test spawns the real
  `spc-pool` binary front-only with the default `[storage] redis` URL;
  since `c55b737` a front-only process fails closed unless the local Redis
  on 6379 already carries the regtest `spc:chain-identity` binding (the
  payout role initializes it; seeding the key manually also works). With
  neither Redis nor the binding the test fails its 15 s listen timeout —
  pre-existing master-tip behavior, not a regression from this packet, and
  flagged for the merge session because the `nix flake check` test
  derivation (no Redis in the sandbox) appears exposed to the same failure.
- `cargo test -p spc-store --test postgres --no-run`: the new test
  compiles; DB-backed cases skip without a configured test database, as
  designed.

Residuals already tracked elsewhere (not regressions, out of packet scope):
aggregate-destination binding and the provisional-ancestry formula are
recorded open in `docs/reviews/2026-11-09-ladder-completion.md`; funded
stock-captaind lifecycle acceptance is Phase-7/lab scope.
