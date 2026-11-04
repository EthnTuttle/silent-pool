# 2026-11-03 — Denomination-ladder coinbase payouts with Ark remainders

**Trigger.** Operator product decision, three steps on 2026-11-03. (1)
Coinbase payouts are denomination-quantized; the remainder is paid over Ark.
(2) Owner policy, stated as constraints: **A1** maximize the on-chain paid
share of every block, always; **A2** nothing is deferred for capacity or
trimming — there is no waiting; **B3** whatever cannot be paid in the
coinbase goes over Ark. (3) Owner follow-up: denominations should be **large
in nominal value**, coinbase transaction size is part of the analysis, and
the one-band-vs-many question must be answered explicitly. Prior analysis:
`docs/analysis/2026-11-03-amount-banded-payouts.md` (measurement map stands;
its overage-to-Ark rejection was a value call the product decision
overrides). This record supersedes its own earlier same-day drafts (single
banded output per miner, then uniform single denomination).

## The tradeoffs, accepted as product decisions

1. **Every miner gains an Ark footprint**, and identities with `owed < u`
   (the base unit) get **no coinbase output at all** this block — their whole
   balance rides Ark. Counts by population are in the study below.
2. **Remainders are trustodial until refresh**: `r_i = owed_i mod u < u` is
   custodial for the 6-block confirmation window, then
   sender-or-server-honest (§11.7).
3. **The Ark server sees exact per-miner remainder amounts**; with the
   ladder public it can bound each miner's coinbase-carried value from the
   denomination multiset it cannot otherwise decompose. Exact-amount privacy
   rests wholly on the Ark-server trust assumption the design already makes.
4. **UTXO fan-out is real but modest** under the ladder: per-miner outputs
   per block are the base-8 digit sum of `floor(owed/u)` — ~2-18 in practice
   (study Q5), not hundreds.

Chain-facing claim: output values come from a short public denomination list
`{u·8^j}`; within a block each denomination class is an anonymity set, and
no per-miner varying value exists on-chain.

## Decision

1. **Ladder shape (answers "one band or multiple": MULTIPLE).** Denominations
   `d_j = u · 8^j`, `j = 0, 1, …` up to coverage of `T`; ratio **8**; base
   unit **`u_min = 10 000 sats`** (`>= dust` with margin; the remainder sets
   the Ark flow, so the base unit is the float lever, not the top band — a
   large nominal denomination costs zero extra outputs). At `u = 10 000` the
   ladder is only 6 active denominations in this subsidy era:
   `10k, 80k, 640k, 5.12M, 40.96M, 327.68M` sats. Each identity's coinbase
   payout is the base-8 decomposition of `floor(owed_i / u)`: `digit_j ∈
   0..7` outputs of value `d_j` at per-output keys `P_{i,k}` (§2.3's
   reserved `k` now enumerates an id's outputs).
2. **The budget is the lever; u is adaptive, never deferred (A1+A2).**
   `N = floor((budget_wu − overhead_wu) / 172)`. Per block the pool computes
   `u` = the smallest integer `≥ u_min` such that `Σ_i digitsum₈(⌊owed_i/u⌋)
   ≤ N` (monotone in u; binary search; pure function of the owed map, so
   settlement re-runs it byte-identically and the snapshot pins `(u, r, N)`
   for the miner audit). Capacity is self-limiting: trimming is deleted —
   when the owed map is too fine-grained for the budget, `u` rises and the
   remainder mass moves to Ark; no miner waits a block for capacity.
   `Σ q_i ≤ N` holds by construction.
3. **Remainders (B3).** `r_i = owed_i mod u`, plus full below-`u` balances,
   go to `ark_due` when deliverable, sent at `H + confirmation_depth`;
   sub-`min_overage_sats` (10 000) remainders aggregate per `sp_id` until
   they clear the threshold or the oldest unsent row is 144 blocks old.
   Every remainder sat is ledger-booked at settlement (liability-reported),
   so "eventually paid over Ark" is a ledger property.
4. **No capacity deferral; errors are held, entitlements are not.** The
   `C`-gate on owed and §4.8's trim loop are removed. The retained
   `pending` rail becomes an **error rail only**: failed Ark sends awaiting
   retry and balances whose delivery keys are unknown at build time
   (`hold_undeliverable`). It is never a capacity decision; bootstrap and
   `LedgerApplied` semantics are unchanged.
5. **Ordering.** Outputs are sorted by script bytes (deterministic
   pseudorandom order; no RNG shuffle needed). Settlement matches outputs to
   entries by script (`payout.rs:560-585`) — order-invariant and
   duplicate-script-rejecting; JD validation (§3.5) checks the published
   order verbatim, so any published order is legal. Nothing leaks from
   order.
6. **Settlement predicate.** Pool-template settlement re-runs the same
   accounting and fails closed per identity on
   `Σ_k value(P_{i,k}) == (base-8 value of floor(owed_i/u)) · u` with `u`
   recomputed from the owed map and pinned `(r, N)` — replacing the guard at
   `payout.rs:656`; the paid map accumulates across `k` per identity.
7. **Miner contract (§3.6/§11.6).** Two-part, fail closed: coinbase leg is
   the scan-and-sum equality above (scan `k` until first miss); Ark leg is
   the accumulated remainder VTXOs. `require_own_payout` compares the sum;
   below-`u` miners legitimately have no coinbase output and are told so via
   `/api/sp/me` (a registered miner's whole balance parks on the Ark rail —
   the contract must show it, or "no output" reads as theft).
8. **JD mode: pool-template-only in v1.** The JDC computes sats from
   weights; the pool cannot force denominations. Under JD, A1 is trivially
   satisfied (exact-`owed` weights pay ~100 % on-chain) but all values are
   miner-specific — the denomination privacy of this decision is
   unavailable. O20 tracks a JDC-side convention.
9. **Config.** `[accounting]`: `budget_wu` ceiling raised **400 000 →
   1 000 000 WU** (coinbase share of the block capped at 25 %; floor stays
   50 000); `max_payout_slots` superseded — set = `N` (5 800 at the new
   ceiling); `base_unit_min_sats = 10 000`; `ladder_ratio = 8` (fixed; not
   operator-tunable in v1). `[ark] max_order_sats = 0` now means
   "top-of-era denomination − 1"; remainders are `< u ≤ 40k` in practice, so
   the cap is only the operator circuit-breaker.

## Parameter study

Convention: **measured** = code/config at file:line; *(model)* = stated
model. Model: `T = 312 500 000` sats, 1 % fee ⇒ owed pool
`T_owed = 309 375 000`; shares ∝ hashrate over `m` identities; log-normal
ln-σ = 1.5 and Pareto α = 1.2; 40 compositions, means, seed 20261103.
Coinbase output = 43 B / 172 WU (measured §4.8).

### Owner's arithmetic, verified and corrected

- Uniform one-band, `m=1, w=1M`: 309 outputs, 13.3 KB, 53.1k WU, 99.88 %
  on-chain — **confirmed**.
- `m=100, w=1M`: 267 outputs, 11.5 KB, 45.9k WU, **86.3 %** on-chain (owner
  estimated 300/97 %; the gap is remainders plus sub-`w` miners in a skewed
  size distribution — their figure assumes near-even splits).
- `m=1000, w=1M`: not literally zero — 125 outputs, all whales, **40.3 %
  on-chain**, 60 % of value to Ark; and `m=100, w=5M`: 35 outputs, 56.3 %.
  The owner's directional claim is right: a single band silently exiles the
  pool to Ark once typical `owed < w`; strictness of the zero depends on
  the tail.
- Pow2 ladder (r=2), `u=100k`, `m=100`: 398-427 outputs, ~17 KB, 68-73k
  WU, 99.84 % on-chain — **confirmed** (owner: 400/17.3 KB/69k WU/97 %).
- `m=1000, u=100k` (r=2): 2 255-2 528 outputs, ~95-106 KB, ~388-435k WU —
  fits the corrected budget; on-chain 98.4 % at `u=10k` (at `u=100k`,
  remainders rise 10x to ~5e7 sats ≈ 16 % of owed pool: **the base unit
  inflates the float, the top denomination costs nothing** — owner
  confirmed).
- Coinbase size ladder: 2 300 outputs = 99 KB / 396k WU; 5 800 = 250 KB /
  1.0M WU; 23 000 = 990 KB / 4.0M WU — **confirmed**, and adopted: the
  binding cap is block-weight share, so the 256-slot policy is superseded by
  the budget (Decision 9).

### Q1 — One band vs multiple: confirm the ordering, with numbers

Confirmed, and sharpened:

| Shape | Outputs/miner | Within-block anonymity | Coverage | Remainder | Feasible population at N=5 800 |
|---|---|---|---|---|---|
| One uniform band `w` | `owed/w` (LINEAR) | maximal: the whole output set | hard floor at `owed ≥ w` | `< w` | ~2 000-3 000 identities |
| Geometric ladder `u·r^j` | digit sum, `O(log_r(owed/u))` (LOG) | per-denomination classes (table below) | floor at `owed ≥ u` (u is 100x smaller than any viable w) | `< u` | 10 000+ (u adapts) |

One band makes sense ONLY while typical `owed ≫ w`: at `m ≤ ~100` it gives
perfect within-block anonymity at ~97-99.9 % on-chain inside budget. Above a
few hundred identities it degenerates — either `w` small enough to cover
everyone blows the budget, or `w` big enough for the budget exiles the pool
to Ark (86→40 % on-chain at 1M sats). The geometric ladder keeps both
properties at every population: logarithmic count, small base unit, large
nominal tops for free. The cost vs one band: mixed values reintroduce
coarse value attribution — denomination counts plus the public total form a
constrained partition, and the top occupied class of a whale is
near-singleton (WabiSabi-style subset-sum; the base unit hides everything
below `u` on-chain, while per-miner composition above `u` is partially
structured). Pareto points (m=1 000, log-normal): **uniform w=135 869 →
2 251 outputs / 387k WU / full-set anonymity / 82.2 % on-chain**;
**r=2 u=10k → 2 255 outputs / 388k WU / median class 187 / 98.4 %**;
**r=4 → 3 233 / 556k WU / median class 528 / 98.4 %**;
**r=8 → 4 720 / 812k WU / median class 1 428 / 98.4 %**.
Privacy-per-weight favors r=8 everywhere in budget.

### Q2 — Recommended ladder, numbers by population

**Ratio 8, `u_min = 10 000` adaptive, N = 5 800** (Decision 9). Whether the
ladder fits at `u_min` depends on population; `u` doubles until it fits:

| m | dist | u used | outputs/blk | coinbase KB / WU | on-chain % | Ark flow/blk | zero-coinbase |
|---|---|---|---|---|---|---|---|
| 100 | both | 10 000 | 870-898 | 36-38 KB / 150-155k WU | 99.84 | 5.0e5 | 0 |
| 1 000 | lognorm | 10 000 | 4 720 | 198 KB / 812k WU | 98.39 | 5.0e6 | 66 (6.6 %) |
| 1 000 | pareto | 10 000 | 4 909 | 206 KB / 844k WU | 98.39 | 5.0e6 | 6 (0.6 %) |
| 10 000 | lognorm | ~40 000 | 3 622 | 156 KB / 623k WU | 62.2 | 1.17e8 | 8 230 (82 %) |
| 10 000 | pareto | ~19 750 | 4 779 | 205 KB / 822k WU | 69.7 | 9.4e7 | 7 703 (77 %) |

**Float** (sends at H+6, pool output refills at H+100 ⇒ 94-block lag; owner
framing "~100x per-find flow" is the 100 %-share bound; general rule
`float ≈ ark/blk × 94·pool_share × 1.5 margin`): m=1 000 ⇒ ark/blk ≈ 5.0e6
sats → **1 % pool ≈ 7.1e6 sats float; 10 % pool ≈ 7.1e7**. m=10 000 ⇒
ark/blk ≈ 9.4e7-1.17e8 → **1 % pool ≈ 1.5e8 sats (~1.5 BTC); 10 % pool ≈
1.5e9 (~15 BTC)**. Fundability: the block's pool output carries
`fee + Σ ark_due + error-held`, so the refill at H+100 replaces exactly what
the float spent at H+6 — the design never books an Ark payout whose sats are
not in the minable coinbase. Short float ⇒ send fails, retry with the same
`order_id`, `failed_to_pending`+alarm after `retry_blocks`; the entitlement
was booked at settlement and is never cancelled. Fail closed on the send,
never on the entitlement.

### Q3 — Output budget to adopt

**`budget_wu` ceiling 1 000 000 WU** (coinbase ≤ 25 % of block weight; a
5 800-output coinbase is 250 KB / 1 000 600 WU including overhead), floor
unchanged 50 000 WU; **slots: `max_payout_slots` = N(= 5 800)** — the slot
policy is set equal to the weight budget, one limit, not two. Anything
larger starves fee revenue the pool itself needs; the 25 % share is the
stated ceiling of what the operator may claim from the template.

### Q4 — What a large nominal band changes (1M / 10M sats)

As a **uniform** band: `w=1M`: m=100 → 267 outputs, 86 % on-chain (anonymity
= whole set, 267 rows); m=1 000 → 125 whale-only outputs, 40 % on-chain.
`w=10M` (earlier grid): ~31-output ceiling, 86-97 % of revenue to Ark,
>99 % of miners Ark-only. As a **ladder denomination**, e.g. the 5.12M class
at m=1 000: it is populated only by miners owed ≥5.12M (~5-8 outputs in that
class, near the tail), while the 640k class carries ~1 400+ companions
(measured class-set medians, r=8 table). Confirmation of the owner's rule:
a large denomination's cost is `≤ 7` extra outputs per qualifying miner; its
privacy value is that it hides within-8x; the remainder and the float are
set by `u` alone.

### Q5 — What the miner's wallet sees (recommended ladder)

Per found block, outputs per miner = base-8 digit sum of `floor(owed/u)`:

- **small miner** (owed 50k sats, u=10k): quotient 5 → **5 UTXOs**
  (5×10k);
- **medium** (1M): quotient 100 = `144₈` → **9 UTXOs** (1×640k + 4×80k +
  4×10k);
- **large** (50M): 5 000 = `11610₈` → **9 UTXOs**; **whale** (200M): 20 000
  = `46620₈` → **18 UTXOs**.

Wallet/agents: scan-and-sum over `k` per block (§2.3 preimage already
carries `ser32(k)`); UTXO churn is tens, not hundreds — versus 385-560 for
the uniform-w whale. Agent guidance stands: never auto-consolidate (merging
re-links the block's set); consolidate only into coinjoin-aware spends or
Ark refreshes; consolidation fee ≈ 57.5 vB per key-path input when it
happens.

## Alternatives rejected

- **One uniform band**: maximal within-block anonymity but linear outputs
  and a per-miner coverage floor at `w`; degenerates silently for
  populations above a few hundred (Q1). Kept as the mental model the ladder
  generalizes.
- **r = 2 (binary):** smallest counts but per-class anonymity halves
  (median class 86-353 vs 594-1 454 at r=8, m=1 000-10 000) and singleton
  top classes re-identify whales within 2x instead of 8x. Dominated in
  privacy-per-output.
- **r = 4:** strictly between; r=8 preferred since count inflation vs r=4
  is ≤ 35 % while class sizes ~double.
- **Single banded output per miner (k=4 octave ladder, first draft):** fails
  A1 worse than it helps privacy (93 % on-chain but per-band anonymity
  23-26 with miner-varying values); superseded.
- **Deferral variants:** removed by policy A2; capacity is now impossible
  to exceed by construction (adaptive `u`), and the error rail covers the
  rest.
- **JDC-side denomination convention in v1:** O20.

## Edits applied

- `docs/SPEC.md`: frontmatter changelog (replaced inline); §0; §2.3 (output
  rule with `k`-per-output, `k = 0` bullet retired); §3.6; §4.1
  (`base_unit_min_sats`, `ladder_ratio`, budget/slot rows); §4.4 payout
  shape; §4.6 pseudocode/custody/invariants; §4.7 (multi-output entries,
  snapshot pins `(u, r, N)`); §4.8 (trim removed; budget 1M WU ceiling);
  §5.1; §6.1/§6.3; §10 (O8, O20); §11.1; §11.2; §11.3 (float sizing);
  §11.6; §11.7; §11.9; §11.10.
- `docs/WIRE-FORMATS.md` §3.1: blocks message pins `(u, r, N)`; entry sats
  comment.
- `DESIGN.md`: payout-shape section; proposal bullets; observer analysis.
- `docs/ROADMAP.md`: R1 row points here.

## Implementation plan (work packets, dependency order)

- **WP0 — ladder primitive** (`crates/spc-accounting/src/lib.rs`): pure
  base-8 decomposition, `u` solver (smallest `u ≥ u_min` with
  `Σ digitsum ≤ N`, binary search), validation. Tests: boundary vectors,
  `Σq ≤ N`, monotone-u, idempotence; coverage `d_J ≥ T`.
- **WP1 — routing rewrite**: `route_tiers` (`lib.rs:405-523`) minus C-gate,
  trim, partial-pay; `Routing.kept = (id, [d_j × digit])`; error rail;
  `build_distribution` multi-output (`lib.rs:696-764`); `front.rs:596-640`
  wiring; config. Tests: conservation, adaptivity, §4.5 re-pinned fixture.
- **WP2 — settlement**: per-id script accumulation and the equality
  predicate (Decision 6) replacing `payout.rs:656`; remainder `ark_payout`
  rows in the same tx. Tests: wrong-count/wrong-sum reject, idempotent
  re-apply, adaptive-u recompute parity.
- **WP3 — Ark emission**: aggregation gate, `max_order_sats =` ladder-derived,
  covered-row migration (`payout.rs:1623-1689`, `ark.rs:108`,
  `postgres.rs`). Tests: aggregation, retry/dedupe, short-float drill
  (fail send, keep entitlement, retry, alarm).
- **WP4 — agent**: k-scan-and-sum, two-part reconciliation, UTXO churn
  warnings per Q5. Tests: vectors, mismatch alarms.
- **WP5 — e2e + docs**: regtest (ladder outputs, remainder sends,
  aggregation, short-float), README sync.

## Risks (top three)

1. **Redemption-time clustering does not close.** Denominations hide
   per-block value attribution; spending behavior re-links (`q` outputs
   merged in one tx/board reveal the miner's set and, repeated, the
   trajectory). Wallet discipline mitigates; only an O19 covenant tree
   closes it.
2. **Ark rail becomes population-scale infrastructure**: up to ~5 000
   orders/block (remainders) and up to ~1.2e8 sats/block at 10 000
   identities; server-side limits are discovered, not assumed; float sizing
   above is a hard requirement (10 % pool × 10k identities ≈ 15 BTC).
3. **`u` is consensus-critical policy**: computed at build, recomputed at
   settlement and by every auditing miner. Any ambiguity in the solver
   (ties, search order, `u_min`, `N`) halts settlement (fail closed) or
   breaks the audit — the solver spec and the snapshot pin `(u, r, N)` are
   the load-bearing pieces, and implementation MUST reuse one pure function
   for all three callers.
