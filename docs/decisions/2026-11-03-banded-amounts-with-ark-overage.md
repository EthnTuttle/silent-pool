# 2026-11-03 — Denomination-ladder coinbase payouts with Ark remainders

**Trigger.** Operator product decision, iterated on 2026-11-03:
(1) coinbase payouts are denomination-quantized, remainder over Ark;
(2) policy constraints: **A1** maximize the on-chain share, **A2** no
deferral for capacity or trimming, **B3** whatever cannot be paid in the
coinbase goes over Ark; (3) large nominal denominations, coinbase size in
scope; (4) ladder decided; (5) **FINAL**: the ladder is a constant of fixed
satoshis — **no era scaling, no halving re-parameterization, ever**. Prior
analysis: `docs/analysis/2026-11-03-amount-banded-payouts.md` (its
measurement map stands). This record supersedes its earlier same-day drafts.

## The tradeoffs, accepted as product decisions

1. **Miners below the base rung get no coinbase output.** Base rung
   500 000 sats; *(model)* the zero-coinbase population per block in era 1:
   ~34/100, ~860/1 000, ~9 950/10 000 identities; those balances ride Ark.
2. **Remainders `< 500 000` sats are trustodial until refresh** (custodial
   the 6-block window, then sender-or-server-honest, §11.7).
3. **The Ark server sees exact remainders and full sub-base balances**;
   chain-side amount privacy rests on the Ark-server trust assumption the
   design already makes.
4. **UTXO fan-out is small**: whale worst case ~14-16 outputs per block
   *(model)*.
5. **Fixed satoshis drift against the reward.** See "Halvings (accepted
   consequence)" — accepted with no scheduled change.

## Decision

1. **Ladder (final, constant):** `{500 000; 1 000 000; 3 000 000;
   5 000 000}` sats, forever; greedy largest-first decomposition of
   `owed_i`, repeated rungs allowed (one output per piece at `P_{i,k}`,
   §2.3); remainder `owed_i mod 500 000` to Ark. The candidate tail rungs
   {10M, 25M} were evaluated (they would cut whale UTXOs ~3x, ~16 → ~5 per
   block) and are NOT adopted: the whale case is small in absolute terms and
   the FINAL constraint fixes the ladder at four rungs.
2. **Purity (why fixed sats are worth stating as a property):** the band
   function is trivially pure over the amount. It has **no dependence on
   height, subsidy, fees, or the coinbase value** — build, settlement and
   miner audit all apply the same four constants, so the front/settlement
   divergence risk that any height- or value-keyed ladder would carry
   (fail-closed halt at the §4.6 recompute) is removed entirely. No
   boundary vectors exist because there is no boundary.
3. **Output capacity.** Every output is `≥ 500 000` sats, so
   `outputs ≤ T_owed / 500 000` ≈ **619 in era 1** (619 × 43 B = 26.6 KB;
   × 172 = 106k WU). This is a value bound, independent of population —
   with fees the bound is `T/500 000`, so a fee-doubled era-1 block could
   reach ~1 230. **Recommended policy cap: `max_payout_slots = 1024`**
   (1 024 outputs = 44.0 KB, 176k WU — plenty above 619), with
   **2 048 (88.1 KB, 352k WU) as the headroom setting** that absorbs a
   fee-doubling block; both sit far under the 1M-WU ceiling (coinbase ≤
   25 % of block). A build that would exceed the cap fails closed and
   alarms — it is never silently truncated and never defers a miner.
4. **Remainders and errors.** Sub-base balances and remainders go to
   `ark_due` when deliverable, sent at `H + confirmation_depth`, aggregated
   per `sp_id` below `min_overage_sats` (10 000) or past
   `overage_max_delay_blocks` (144). The `pending` rail is an error rail
   only (failed sends, unknown keys at build); every entitlement is booked
   at settlement, so "eventually paid over Ark" is a ledger property.
5. **Settlement predicate:** pool-template settlement fails closed per
   identity on `Σ_k value(P_{i,k}) == owed_i − (owed_i mod 500 000)` under
   the constant ladder; the paid map accumulates across `k`.
6. **Miner contract:** two-part. (a) coinbase leg: the scanned sum over
   `P_{i,k}` equals the greedy-decomposition value of `owed` (type-32
   window) on the published four-rung ladder; (b) Ark leg: accumulated
   VTXOs equal the `owed mod 500 000` rows at `/api/sp/me`. Ambiguity or an
   unpublished ladder is a hard reject; sub-base miners legitimately have
   no coinbase output.
7. **JD mode:** pool-template-only in v1 (O20 tracks a JDC convention).
8. **Config:** `ladder_base_rungs_sats = [500000, 1000000, 3000000,
   5000000]` (constant); `max_payout_slots = 1024` (2048 documented as
   fee-spike headroom); `C` retained as JD `dust_limits` floor and the
   `rung_min ≥ C` sanity check. `/api/pool` publishes the rungs; there is
   no era field because the ladder never changes.

## Halvings (accepted consequence)

The rungs are constant satoshis, so each halving doubles their relative
size: the 500k base is 0.16 % of the era-1 owed pool, 0.32 % in era 2,
0.64 % in era 3, 1.28 % in era 4. The zero-coinbase population and the Ark
flow/float fraction grow correspondingly every era. **This is accepted with
no scheduled change**; the rungs are a published constant and their drift
is visible to miners at `/api/pool` and in every decomposition they audit.

## Parameter study (decided ladder, era 1)

Model: `T_owed = 309 375 000` sats; log-normal σ=1.5 / Pareto α=1.2 pools;
40 draws; seed 20261103. Greedy over the four rungs.

| m | dist | outputs/blk | coinbase size | on-chain % | Ark flow/blk | zero-coinbase | whale UTXOs |
|---|---|---|---|---|---|---|---|
| 100 | lognorm | 135 | 5.7 KB / 23k WU | 92.4 | 2.4e7 sats | 34 | 14 |
| 100 | pareto | 167 | 7.0 KB / 29k WU | 91.9 | 2.5e7 | 4 | 16 |
| 1 000 | lognorm | 186 | 7.8 KB / 32k WU | 56.5 | 1.35e8 | 856 | 4 |
| 1 000 | pareto | 120 | 5.0 KB / 21k WU | 54.4 | 1.41e8 | 917 | 11 |
| 10 000 | lognorm | 52 | 2.2 KB / 9k WU | 11.1 | 2.75e8 | 9 952 | 2 |
| 10 000 | pareto | 69 | 2.9 KB / 12k WU | 28.2 | 2.22e8 | 9 951 | 7 |

Class sizes (rung outputs per block, era 1): m=100: 500k ≈ 38-54,
1M ≈ 49-71, 3M ≈ 11-14, 5M ≈ 31-36; m=1 000: 500k ≈ 59-103, 1M ≈ 36-69,
3M/5M single digits to 17; m=10 000: 500k ≈ 35-39, higher rungs single
digits. The honest reading: the 500k base makes on-chain share
population-bound — ~55 % of value at 1 000 identities, 11-28 % at
10 000 — so a large pool under this ladder is Ark-primary. That is the
decided ladder's consequence, recorded, not re-litigated.

**Float.** Law: `float = (per-find Ark flow) × 94-block refill lag
(H+6 → H+100) × pool share f × 1.5 margin`; worst-case flow per find is
bounded by `T` (every miner sub-base), i.e.
`float_worst ≤ 94 f T × 1.5`. Corrected arithmetic (the owner's
examples ran 10x low): era-1 worst case f=1 % ⇒ **2.94e8 sats ≈ 2.9 BTC**,
f=10 % ⇒ **≈ 29.4 BTC** (before margin). Means for the decided ladder:
m=100 ⇒ ≈ 3.4e7 sats (0.34 BTC) at f=1 %; m=1 000 ⇒ ≈ 1.9-2.0e8
(1.9-2.0 BTC); m=10 000 ⇒ ≈ 3.2-3.9e8 (3.2-3.9 BTC) — near the worst-case
bound, because the 500k base exiles most of a large pool by design.
Funding closure is unchanged: `pay_P` carries `fee + Σ ark_due +
error-held`, so the H+100 board refills exactly what the H+6 sends spent;
a short float fails the send and retries it — never the entitlement.
The earlier draft's "100x flow per block" phrasing is superseded by the
94-block lag formula everywhere.

## Alternatives rejected

- **Subsidy/era-scaled rungs** (would keep output count, on-chain share,
  float fraction and anonymity constant across halvings): rejected by the
  FINAL constraint — fixed satoshis, no re-parameterization machinery,
  no height keying. Its determinism concern (front/settlement share of a
  pure height function) never materializes because there is no function.
- **Tail rungs {10M, 25M}:** evaluated above; not adopted under the FINAL
  fixed rung set (available to any future ladder review without touching
  the four base rungs).
- **Adaptive base unit / geometric ladders, uniform one-band:** earlier
  same-day drafts; superseded (one-band verified degenerate above a few
  hundred identities; geometric dominated by the decided set's
  policy simplicity).
- **Deferral variants:** removed by policy A2.

## Edits applied (this revision)

- `docs/SPEC.md`: frontmatter changelog entry; §0, §2.3, §3.6, §4.1
  (rungs constant, `max_payout_slots = 1024`), §4.4 (constant ladder +
  purity statement), §4.6, §4.7, §4.8 (cap = policy), §5.1, §6.1/§6.3,
  §10 (O8, O20), §11.1, §11.2, §11.3 (float law), §11.6, §11.7, §11.9,
  §11.10.
- `docs/WIRE-FORMATS.md` §3.1 (entry `p_x[]`; no ladder fields — constant).
- `DESIGN.md`: Denomination payouts section; observer analysis.
- `docs/ROADMAP.md`: R1 row.

## Implementation plan (work packets, dependency order)

- **WP0 — ladder primitive** (`crates/spc-accounting/src/lib.rs`): greedy
  decomposition over the constant rungs; validation (`rung ≥ C ≥ dust`).
  Tests: canonicality (largest-first), `pay + rem == owed`,
  `outputs ≤ T/500k`, rung-set membership of every output.
- **WP1 — routing + distribution** (`route_tiers` `lib.rs:405-523` minus
  C-gate/trim/partial-pay; `build_distribution` `lib.rs:696-764`
  multi-output sorted by script; `front.rs:596-640`; config). Tests:
  conservation, slot-cap invariant, §4.5 re-pinned fixture.
- **WP2 — settlement** (`payout.rs:539-699`): per-id `k`-accumulation;
  equality predicate replacing `payout.rs:656`; remainder rows. Tests:
  wrong-count/sum reject, idempotent re-apply.
- **WP3 — Ark emission** (`payout.rs:1623-1689`, `ark.rs:108`,
  `postgres.rs`): aggregation, `max_order_sats` default, covered-row
  migration. Tests: aggregation/retry/dedupe, short-float drill.
- **WP4 — agent**: k-scan-and-sum, UTXO-churn guidance (never auto-merge;
  consolidating a block's outputs re-links the set). Tests: vectors,
  mismatch alarms.
- **WP5 — e2e + docs**: regtest shapes + short-float drill; README sync.

## Risks (top three)

1. **Redemption-time clustering** — still the open channel: merging a
   block's outputs at spend/board time re-links the miner's set and its
   trajectory. Wallet discipline mitigates; only an O19 covenant tree
   closes it.
2. **Big-pool on-chain share is rung-bound** — ~55 % of value / ~86 % of
   identities Ark-ward at 1 000 identities *(model)*, and the float mean
   approaches the worst case as population grows; operators adopt the §11.3
   float law and alarm on population drift. Halving drift doubles this
   every era, by decision.
3. **Ark rail is population-scale infrastructure** — orders per found
   block ≈ all miners with remainders (thousands per block for big pools);
   server-side limits are discovered, not assumed (bark-adapter decision);
   the aggregation rule and the fail-closed send loop carry the load, but
   bursts at `H+6` after every find are the new steady state.
