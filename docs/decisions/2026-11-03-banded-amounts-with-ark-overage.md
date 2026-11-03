# 2026-11-03 — Banded coinbase amounts with Ark overage

**Trigger.** Operator product decision: coinbase payouts are quantized to a
public band ladder and the overage (`owed − band_floor(owed)`) is paid over
Ark. Prior analysis: `docs/analysis/2026-11-03-amount-banded-payouts.md`,
written against the previous verdict; its end-to-end measurement map (where
exact sats arise, what banding would touch) stands and is cited throughout;
its verdict ("not worthwhile", overage-to-Ark rejected for overage-to-pending)
is superseded by this decision for the routing question.

## The tradeoff, accepted as a product decision

This is adopted with open eyes; the costs are the point of this record and are
not mitigated elsewhere:

1. **Every coinbase-tier miner gains an Ark footprint.** Today a kept miner
   has zero Ark-side presence (Ark orders exist only for sub-`C` rows; one per
   `(block_hash, sp_id)`, `payout.rs:1669`, `payout.rs:1798`). With banding,
   every kept miner with a nonzero overage — in the model below, effectively
   all of the up-to-256 per block — receives an Ark send at `H + 6`.
2. **The overage is trustodial until the miner refreshes.** Value that today
   is non-custodial at mine time (SPEC §0) moves to the §11.7 path: custodial
   for the 6-block confirmation window, then sender-or-server-honest for up
   to the VTXO expiry.
3. **The Ark server sees exact per-miner amounts.** Combined with the public
   band positions, `overage + band_floor = owed` reconstructs the exact
   per-miner per-block amounts for every paid miner, not just sub-`C` ones
   (pre-band status quo ante: `payout.rs:1623-1689` emits orders only for
   Ark-rail rows). SPEC §11.7's cross-layer observer becomes total.

The privacy claim is thereby restated as: **the chain observer sees a band,
not an exact amount**; amount privacy now rests on the Ark-server trust
assumptions the design already makes (SPEC §11.7). Nothing is claimed against
the pool.

## Decision

1. **Ladder.** Logarithmic, `k = 4` sub-bands per octave: edges
   `band(0) = C`, `band(j) = floor(C · 2^(j/k))` for `j = 1, 2, …` (deduped,
   monotone, computed in integers only, unbounded so any carried `owed`
   remains coverable). `band_floor(x)` = the largest edge `≤ x`. With
   `C = 330` this is about 19 % relative width, 81 edges covering
   `[C, T]` for `T ≈ 3.3e8` — at most 7 bits per miner per block to a chain
   observer, versus an effectively unique u64 today.
2. **Amounts.** For kept ids: `pay(id) = band_floor(owed(id))`;
   `overage(id) = owed(id) − pay(id)` routes to `ark_due` if the id has a
   registered, propagated Ark destination (§11.4), else to `pending`. The
   overage of a kept id is never forfeited. `raw_payouts` and the kept-gate
   `owed ≥ C` are untouched: banding is a post-pass inside `route_tiers`
   between the keep decision and the emitted amount
   (`crates/spc-accounting/src/lib.rs:442-457`). Rounding stays down
   (rounding up would spend `pay_P`, `lib.rs:437`).
3. **Minimum viable version.** Banding applies to **pool-template mode only**
   in v1; JD distributions keep exact-`owed` weights (§4.7) and the decision
   of a JDC-side convention is deferred (below, item 6 of the study).
4. **Sub-minimum overages aggregate.** `ark_min_overage_sats` (default
   10 000): per-block overage rows accumulate per `sp_id`; `payout` emits one
   order for the accumulated sum once it clears the threshold, or when the
   oldest unsent row is `ark_overage_max_delay_blocks` (default 144) old,
   whichever first. Every sat of overage is booked into `ark_payout` at
   settlement, so "eventually paid over Ark" holds by construction of the
   ledger, not by a promise.
5. **Settlement predicate.** The pool-template fail-close at
   `bins/spc-pool/src/payout.rs:656` (`*value < raw_sats || *value > e.sats`)
   becomes an exact check: `paid(id) == band_floor(pay_routed(id))`, where
   `pay_routed` comes from re-running `route_tiers` **with the ladder** at
   settlement (payout role already re-runs the identical accounting,
   `payout.rs:539-544,629`). The ladder is a static, config-pinned parameter
   published at `/api/pool`; a template-derived ladder is forbidden because
   any front/settlement divergence becomes a per-block settlement halt.
6. **Config caps follow the ladder.** `[ark] max_order_sats = 0` no longer
   means `C`; it means `top covered band width − 1` (with the k=4 ladder
   covering to `≥ 2^28`, about 5.1e7 sats). It remains an operator
   circuit-breaker, now sized so any legitimate single overage passes.

## Parameter study

Convention: **measured** = read from code/config at the cited file:line;
*(model)* = arithmetic on a stated model. The model: pool of M = 1000 miners;
`owed_i` proportional to hashrate share of `T = 332 500 000` sats (the SPEC
§4.5 fixture; measured); kept = `owed ≥ C`, descending, capped at 256 slots
(measured: `C = 330`, `config.rs:448-449,336`; 256 slots, `config.rs:303-305`,
172 WU per output, §4.8). Two share distributions *(model)*: log-normal with
ln-σ = 1.5 (P90/P10 payout ratio ≈ 47x), and Pareto α = 1.2 (heavy tail).
50 independent pool compositions, means reported; seed 20261103. The model
omits fee variance, window-floor noise and pending carry (`owed ≈ raw` steady
state) and truncates the sub-`C` mass (the modeled sub-`C` Ark flow is a
few sats per block and is ignored below — a real deployment's sub-`C`
flow adds to the float need but does not change the ordering of ladders).

### Q1 — Ladders, anonymity sets, overages

Per found block. "Anon" = how many of the ≤256 kept miners share a miner's
band (the chain-observer anonymity set; the slot cap bounds it at 256
regardless of ladder). Overages are per kept miner per block. L = band count
covering `[C, T]`; bits = `ceil(log2 L)`.

| Ladder | L / bits | Anon mean (median), log-normal | Anon mean (median), Pareto | Singleton miners | Overage mean / p99 / max (sats), log-normal | Overage mean / p99 / max, Pareto | Total overage per block | % of T |
|---|---|---|---|---|---|---|---|---|
| (a) powers of two above `C` | 22 / 5 | 81 (82) | 82 (95) | 0.3-0.6 % | 290 890 / 1.98e6 / 5.2e6 | 276 975 / 2.25e6 / 1.73e7 | 7.0-7.4e7 sats | 21.3-22.4 % |
| (b) k=2 per octave | 41 / 6 | 41 (43) | 48 (54) | 0.6-1.0 % | 157 483 / 1.00e6 / 2.6e6 | 146 421 / 1.26e6 / 6.3e6 | 3.7-4.0e7 | 11.3-12.1 % |
| (b) **k=4 per octave** | 81 / 7 | 23 (23) | 26 (27) | 1.2-1.8 % | 85 802 / 6.0e5 / 1.8e6 | 80 448 / 7.5e5 / 3.8e6 | 2.1-2.2e7 | 6.2-6.6 % |
| (b) k=8 per octave | 161 / 8 | 12 (12) | 14 (14) | 2.8-4.0 % | 44 377 / 3.1e5 / 8.3e5 | 40 220 / 3.4e5 / 1.9e6 | 1.0-1.1e7 | 3.1-3.4 % |
| (c) fixed 25 % width | 63 / 6 | 29 (30) | 32 (34) | 0.9-1.6 % | 107 213 / 7.5e5 / 2.0e6 | 101 502 / 7.8e5 / 5.8e6 | 2.6-2.7e7 | 7.8-8.3 % |

Options (b) and (c) are one family: constant relative width `r` is
`k ≈ log(2)/log(r)` sub-bands per octave (r = 1.25 ⇔ k ≈ 3.1), so (c) needs
no separate treatment; its numbers sit between k=2 and k=4 as expected.
Worst-case relative overage (`rel max`, the largest observed
`overage / owed`): 49.8-49.9 % (k=1), 29.2 % (k=2), 15.8-15.9 % (k=4),
8.3 % (k=8): for a geometric ladder the relative width is uniform, so these
hold for any distribution; only the anonymity columns are model-sensitive.
Mean relative overage is distribution-independent for the same reason:
24-28 % (k=1), 15.5 % (k=2), 8.2 % (k=4), 4.2 % (k=8) — consistent with the
log-uniform expectation `ln(band_ratio)/2`.

### Q2 — Recommendation: k = 4 sub-bands per octave

Coarse bands buy anonymity linearly in companions but spend custody
quadratically-ish in the worst case (band width doubles per coarsening step
while companions roughly double). The numbers: k=1 routes **a fifth of all
coinbase-tier value** (21-22 % of T) through the trustodial rail every block,
with a worst single-miner overage of 0.17 BTC in the Pareto run — a recurring,
not rare, exposure for the pool's largest miners. k=4 keeps the absolute
privacy gain (23-26 companions where today every amount is unique; the
≤256-slot cap is the binding anonymity ceiling anyway) while cutting the
trustodial flow 3.4x to 6.2-6.6 % of T and bounding any miner's per-block
overage to ≤16 % of their payout. k=8 halves the anonymity set (12-14) for
another ~2x custody cut — diminishing on both axes, and it pushes 20-29 % of
rows under the 10k aggregation threshold, multiplying deferred sends. The
heavy tail is unfixed at every k (0.3-4 % of kept miners sit in singleton
bands — see Q8); paying more custody for k=1 does not fix it either. Whether
paying in several standard-value outputs per miner beats one banded output is
analysed in Q9 below — it does not change this recommendation.

### Q9 — Multi-output decomposition (product-owner option, 2026-11-03)

Shapes compared: **A** = the decided single banded output per kept miner
(k=4, remainder `owed − band_floor(owed)` to Ark). **B** = fixed-denomination:
a kept miner is paid `floor(owed / w)` outputs of exactly `w` sats, the
remainder `owed mod w` to Ark; the per-block output count is independent of
miner count, ceiling `T / w`. **C** = binary decomposition: each miner is paid
its amount as denomination powers (`d · 2^j`), remainder below `d` to Ark.

**B, numbers.** *(model; same pools as Q1, but `T = 312 500 000` sats per the
owner's framing; "outputs" is the realized count — the ceiling `T / w` binds
only if nearly all value sits above `w`.)* Slot ceiling 256 (measured,
`config.rs:303-305`); weight ceiling `400 000 / 172 = 2 325` outputs
(§4.8 autoscaler bounds):

| w (sats) | outputs/blk ceiling | outputs/blk realized (log-norm / Pareto) | miner share below w (`w/T`) | Ark-only miners (log-norm / Pareto) | Ark flow per block |
|---|---|---|---|---|---|
| 1.25e6 | 250 (250/256 slots — saturates; weight fine) | 89 / 97 | 0.40 % | 95.3 % / 97.0 % | 2.0e8 sats = 61-64 % of T |
| 2e6 | 156 | 40 / 52 | 0.64 % | 97.7 % / 98.3 % | 2.1-2.3e8 = 67-75 % of T |
| 5e6 | 62 | 6 / 13 | 1.60 % | 99.5 % / 99.5 % | 2.5-2.8e8 = 79-90 % of T |
| 1e7 | 31 | 1 / 4 | 3.20 % | 99.9 % / 99.8 % | 2.7-3.0e8 = 86-97 % of T |

`owed < w ⇒` no coinbase output at all: **B creates a hard coinbase threshold
of `w`**, one to two orders of magnitude above today's `C = 330`; in the
modeled pools 95-99.9 % of miners become Ark-only (fully custodial for the
confirmation window, then trustodial until refresh), and 61-97 % of every
block's revenue flows over Ark — the float at Q3 would have to cover most of
per-find revenue instead of ~6 %. That is ArkPool's custody shape with fresh
keys, not this pool's payout model; the product's non-custodial coinbase
property would survive only for miners above 0.4-3.2 % of pool hashrate.

**C, numbers.** Output count per miner = `popcount(owed / d)`: at the dust
minimum (`d = 330`): 0.5M sats -> 7 outputs, 3.3M -> 9, 33M -> 12, 100M -> 12;
a 1000-miner modeled pool emits ≈ 4 700-4 900 outputs per block — infeasible
(2.3x the absolute weight ceiling, 19x the slot cap). At `d = 2^21` (2.1M
sats) the pool emits 20-25 outputs and the slot cap holds, but then
`d` itself is a hard coinbase threshold at 0.67 % share (a 0.5M-sat miner gets
zero outputs, a 3.3M-sat miner 1 output plus 1.2M over Ark, a 33M miner 4,
a 100M miner 5) — the same Ark-only population problem as B.

**Mechanics, checked against the real limits.** (i) *Ordering/shuffling is
safe*: settlement is script-led — the reconstruct path matches outputs to
entries by script, never by position (`payout.rs:560-585`, `DuplicateScript`
rejects collisions); the fingerprint covers the published output list
(`lib.rs:755`), so any shuffle is just another fingerprint and the job-bump
logic (`front.rs:647`) is unaffected; JD validation (§3.5) checks the published
order verbatim, so a shuffled publication is wire-legal (in fact today's
`sp_id` sort at `lib.rs:704` is already identity-independent on-chain).
(ii) *Multi-output is protocol-provided*: §2.3 deliberately keeps `ser32(k)`
in the tweak preimage "so a later version can pay several outputs per tier
without a new tag"; the miner scans `k = 0, 1, …` until the first miss and the
own-payout/§11.6 sum becomes `Σ_k value(P_{i,k})` — scan-and-sum works by
construction, at linear scan cost in outputs per block. (iii) *But* §4.6(b)'s
one-output-per-`(sp_id, H)` invariant would need amendment, trim semantics for
a multi-output miner (drop which of its outputs?) are undefined in §4.8, and
wallet UX becomes `floor(owed/w)` fresh UTXOs per block per miner with
merge-at-redemption — and co-spending/boarding those identical outputs later
re-clusters them, i.e. the chain leak reappears at redemption time.

**What decomposition hides, precisely.** B's chain view is a degenerate
multiset: `n` identical `w`-sat outputs, `n`, `pay_P`, timing. Per-miner value
attribution is information-theoretically absent on-chain (each miner's
contribution is `q` indistinguishable copies under fresh silent keys) —
strictly more hiding than A's 7-bit band. So decomposition **does** improve
attribution privacy over one-output bands on-chain; the owner intuition is
correct there. C does **not**: the denomination-count multiset plus the known
total is a constrained partition problem, singleton high denominations
re-identify the heavy tail exactly like a power-of-two band, and cross-block
bit-pattern stability preserves the trajectory channel — C is A-with-coarser
bands plus more outputs. The bill for B's on-chain hiding lands entirely on
the other side of the ledger the product decision just balanced: exact amounts
for ~95-99.9 % of miners (not remainders — full balances) and 61-97 % of
revenue move onto the Ark-server-visible, trustodial rail.

**Verdict for v1:** **A stands; build A first** (WP0-WP5 unchanged). B is
recorded as a considered, numerically-quantified alternative for a future
profile willing to make the pool Ark-primary; it is strictly additive on A's
plumbing (k-indexed outputs, scan loop, §4.6(b)/§4.8 text), so deciding A now forecloses nothing. C is rejected outright (count-infeasible at fine `d`, no
information gain at coarse `d`).

### Q3 — Float and Ark load

At k=4 the Ark outflow at `H + 6` per found block is *(model)* ≈ **2.1e7 sats**
(2.06e7 Pareto … 2.20e7 log-normal; 6.2-6.6 % of T), versus ≈ 7.0-7.4e7 at
k=1 and ≈ the sub-`C` trickle today. Under the §11.3/§11.10 sizing rule
(Ark-tier sats of the finds expected in 100 network blocks): a 1 % pool needs
`float_target_sats` ≈ 2.1e7 sats (~0.21 BTC) plus variance margin — recommend
≥ 2x, i.e. ≈ 4.5e7 sats; a 10 % pool ≈ 4.5e8 sats (~4.5 BTC). That is new
hot capital that did not exist pre-band; the §11.3/§11.10 text now says so.
Aggregation is **required**, not optional, independent of float: with k=4,
10-16 % of overage rows are 330..10 000 sats and 0.4-0.5 % are sub-dust
*(model)*, no pool-side minimum exists today (`ark.rs:108-109` bounds only
above), and server minima (e.g. `max_arkoor_amount`, any floor) are
discovered per server and never assumed
(`docs/decisions/2026-11-02-bark-adapter.md`). Micro-VTXOs are exactly the
balances §11.1 calls "prohibitively expensive to exit". Aggregation keeps
"eventually paid": every overage is an `ark_payout` row (ledger-booked at
settlement, so liability is reported by the unchanged
`/api/pool/liability` rule §6.1), and rows only accumulate, so the per-`sp_id`
sum crosses `ark_min_overage_sats` or the 144-block age cap in bounded time.

### Q4 — Order volume and caps

Orders per found block: today = number of registered sub-`C` rows only
(measured, `payout.rs:1623-1689`); with banding = one row per kept miner with
nonzero overage, up to the 256-slot cap — *(model)* ~256 rows, of which
214-230 clear 10 000 sats at k=4 and emit immediately at `H + 6`; the rest
emit via aggregation. Idempotency stays per `(block_hash, sp_id)` at booking
(`ark_order_id`, `payout.rs:1798`; rows `postgres.rs:495-508`; §5.1
`ark_payout` PK); an aggregated send covers several rows, so its `order_id`
is derived deterministically over `(sp_id, block_hash of the oldest covered
row)` — restart- and replay-stable, so `ark`'s existing dedupe
(`SeenOrders`, `ark.rs` order check) keeps exactly-once. `max_order_sats`
default `0 = C` (§11.10) is dead: at k=4 the mean overage is ≈ 250x `C` and
83-90 % of rows exceed `C`; the default becomes ladder-derived
(decision item 6).

### Q5 — Interaction with `C` and the tiers

Floor-to-band cannot put a kept miner below `C`: the smallest band **is**
`C`, `band_floor` is monotone, so `owed ≥ C ⇒ band_floor(owed) ≥ C ≥ dust`
(the config invariant `C ≥ dust`, measured `config.rs:618`, is unchanged).
The rail is decided from un-banded `owed` (`lib.rs:446`) before banding runs;
banding only reduces the amount within the coinbase rail. Same for the
capacity-clamped partial-pay edge: its precondition `available ≥ C`
(`lib.rs:458`) implies `band_floor(available) ≥ C`. **Coinbase wins by
construction; nothing re-derives a rail from a banded amount**, and both
`route_tiers` and `band_floor` are pure functions of `(owed, ladder,
capacity)`, so the same inputs give the same rail and amount at build and at
settlement. Cross-block `owed` movement changes the band, never the rail —
there is no new boundary to oscillate across; the existing `C` boundary
behaves exactly as before. An unregistered kept miner: band in the coinbase,
**overage to `pending`, never forfeited** (parity with §11.1: nothing but
unregistered sub-`C` raw is ever forfeited).

### Q6 — JD mode: pool-template-only in v1

Banding cannot be applied pool-side under JD: the JDC computes output sats
from weights at its own template (§4.7 `weight_id = owed(id)`; §4.6 O11
`claim(id) = floor(owed(id) · (T_actual − F) / W)`; measured:
`build_weights` from full-`owed` routing, `front.rs:621`; `claim_jd`,
`lib.rs:547-560`). A banded JD would need (i) the ladder shipped to and
enforced by every JDC — a new PR #203-side convention, (ii) an O11-class
reformula (`band_floor` composed with revenue scaling; flooring before/after
scaling differ), and (iii) double the fail-close surface: any
pool/JDC band mismatch is a per-block settlement halt via the existing
mined-coinbase recompute (`payout.rs:491-503,656`). That buys JD miners
nothing the deferred O19 tree would not do better. **v1 declares banding
pool-template-only**; the JDC-side banding convention is recorded as an open
item (§10 O20).

### Q7 — The miner contract

Two-part verification per pool block, both legs fail-closed:

1. **Coinbase leg (pre-block, §3.6; at booking, §11.6).**
   `paid_coinbase == band_floor(owed)`, with `owed` recomputed from the
   type-32 window plus the prior pending balance (`/api/sp/me`), and the
   ladder from `/api/pool`. Below-`C` miners keep today's no-output case.
   `require_own_payout = warn | refuse` now compares against
   `band_floor(owed)` instead of `owed`: a kept miner whose `P_i` output
   carries any other value treats the pool exactly as a pool paying a wrong
   amount today (warn, or refuse to submit, per the flag). Any ladder
   ambiguity or unpublished/mismatched ladder parameter is a hard reject.
2. **Overage leg (at/after `H + 6`).** The VTXO(s) for `U_i` sum to the
   overages accumulated over the covered blocks; reconciliation stays the
   existing §11.6 identity, now read as genuinely two-rail:
   `owed − paid_coinbase − ark_due_sent_accumulated − forfeited − Δpending == 0`,
   with per-block rows from `/api/sp/me`. A sum mismatch after the greater
   of (`ark_min_overage_sats` clearing, the delay cap, `retry_blocks`
   exhaustion) alarms. The agent reports one balance; custody wording at
   §0/§11.7 covers the trustodial overage window.

### Q8 — The new observer picture

| Observer | Learns, per miner per block |
|---|---|
| Chain | Band index only: ≤ `ceil(log2 L)` = **7 bits** (k=4), vs an effectively unique amount today; plus `k + 1` output count, block timing, pay_P (unchanged semantics, now also carrying the overage sum — derivable from public bands, no new information) |
| Ark server | Exact `owed` for **every** paid miner (`band`, public, + overage it serves), timing at `H + 6`; accepted as tradeoff (3) |
| Pool | Everything (unchanged) |

New anonymity set vs chain-only observers *(model, k=4)*: 23-26 miners share
a median kept miner's band per block (up from 1). Residual channels,
unchanged in kind from the prior analysis: band **trajectory** across blocks
(tracks hashrate coarsely — 7 bits/block still compounds), **output count**
`k + 1`, **timing** (`H` find, `H + 6` sends), the **top-of-ladder
singleton** (0.3-4 % of kept miners in singleton bands *(model)*; a large
stable miner remains fingerprintable by its band alone), and tier
transitions visible to cross-layer watchers (DESIGN.md). None of these is
weakened by banding; the singleton and trajectory channels cap what any
ladder can claim.

## Alternatives rejected

- **Overage to pending** (prior analysis verdict): strictly less Ark load and
  no new server exposure, but converts the privacy gain into payment delay
  and leaves coinbase value custodial-in-`pay_P` indefinitely — superseded by
  the product decision. Remains the designated fallback if Ark-side order
  volume or server limits prove unmanageable in the Phase 7 regtest.
- **k = 1 (powers of two)** — prior "smallest viable": 21-22 % of T
  trustodial per block, 0.17 BTC worst single-miner overage (Pareto), for an
  anonymity set that is 3x k=4's but already capped by the 256-slot ceiling.
- **k = 8**: another 2x custody cut for half the anonymity set and 3-6x more
  deferred sub-threshold rows.
- **Template-derived per-block ladder**: rejected with the prior analysis —
  front/settlement divergence becomes per-block settlement halt.
- **Band-edge above `C` (`C` not itself a band)**: allows
  `band_floor(owed) < C` for `C ≤ owed < first edge`, violating the dust
  invariant; `band(0) = C` by construction.
- **Fixed-denomination decomposition (shape B, Q9)**: the only variant with
  strictly better on-chain privacy (degenerate multiset, no per-miner values),
  rejected for v1 as a product shape: a hard coinbase threshold of
  `w` = 0.4-3.2 % of pool hashrate makes 95-99.9 % of modeled miners Ark-only
  and routes 61-97 % of revenue onto the trustodial rail, with float to
  match. Kept open as a later profile; its plumbing is additive to A's.
- **Binary decomposition (shape C, Q9)**: infeasible at fine denominations
  (~4.8k outputs/block vs 256 slots / 2.3k weight ceiling) and information-
  equivalent to coarse bands at coarse denominations, with the same
  sub-threshold exile as B. Rejected.

## Edits applied

- `docs/SPEC.md`: frontmatter changelog entry (2026-11-03); §0 threat model +
  custody line; §2.3 output amount; §3.6 `require_own_payout` semantics;
  §4.1 parameters; §4.4 rounding rule; §4.6 payout pseudocode, custody
  statement, invariant (b) reaffirmation against decomposition, invariant (e);
  §4.7 entries/sats and JD note; §5.1 `ark_payout`
  comment; §6.1 `/api/pool`; §6.3 `[accounting]`/`[ark]` config; §10 O8, new
  O20; §11.1 tier policy; §11.2 mode table note; §11.3 send-loop aggregation
  and float sizing; §11.6 two-part audit; §11.7 trust/privacy statement;
  §11.9 tests; §11.10 config additions.
- `docs/WIRE-FORMATS.md`: §3.1 `Entry.sats` comment (banded sats for kept
  ids), `0x5350` note unchanged.
- `DESIGN.md`: tier-proposal bullets, privacy-by-observer section (banded
  amounts are now the standard mechanism, not a soft mitigation), open
  questions.
- `docs/ROADMAP.md`: R1 row amended (banded-amounts decision referenced).

## Implementation plan (minimum viable first)

Work packets in dependency order; each lists owned files and tests. No
production code exists yet for these paths — the packets land on the Phase
0-3 prototypes per ROADMAP.

- **WP0 — ladder primitive** (no behavior change; lands first).
  Files: `crates/spc-accounting/src/lib.rs` (new `BandLadder` type,
  `BandLadder::new(min_band: C, k)`, integer-exact `band_floor(u64) -> u64`;
  no floats, deduped monotone edges, fail-closed validation `k ≥ 1`,
  `min_band ≥ dust`).
  Tests: unit vectors (`band_floor` at edges, `±1` sat, u64 boundary);
  proptests: monotone, idempotent, `band_floor(x) ≤ x`,
  `x ≥ C ⇒ band_floor(x) ≥ C`, overage `< band width`.
- **WP1 — routing + config + publication**.
  Files: `crates/spc-accounting/src/lib.rs` (`route_tiers` post-pass over
  `kept` at `lib.rs:442-507`: kept `(id, band_floor(pay))`; overage to
  `Routing::ark_due` when `cap.registered && cap.dest_known`, else a
  `pending_delta` on the kept id — one ledger row per
  `(block_hash, sp_id)`, rail `coinbase`, per §5.1; `pay_p` absorbs the sum;
  `TierConfig` gains `ladder`), `bins/spc-pool/src/config.rs`
  (`[accounting] band_subdivisions_per_octave`), `bins/spc-pool/src/front.rs`
  (wire the ladder at `front.rs:596-610`; `hold_undeliverable` at
  `front.rs:959-993` demotes a kept id wholesale — band and overage both to
  `pending`), `bins/spc-pool/src/api_backend.rs` (`/api/pool` publishes the
  ladder).
  Tests: existing route_tiers/conservation proptests
  (`tests.rs:1093-1411`) re-run under ladders `k ∈ {1,4}`; §4.5 worked
  example gains a banded fixture; config validation rejects
  `min_band < C`, `k = 0`.
  Depends on WP0.
- **WP2 — settlement predicate + booking**.
  Files: `bins/spc-pool/src/payout.rs` (`payout.rs:656` guard becomes
  `paid == band_floor(pay_routed)` from the re-run ladder-aware routing;
  coinbase-rail rows book `paid_sats = band`, `ark_due_sats = overage`;
  overage `ark_payout` rows inserted in the same transaction),
  `crates/spc-store/src/postgres.rs` (insert path; no schema change —
  `ledger_history` already carries both amount columns, `ark_payout` PK
  already fits).
  Tests: payout unit suite extended — banded happy path closes §11.6 to
  zero; wrong-band coinbase fails closed; partial-cap edge
  (`route_tiers` case 2) books `band_floor(available)`; idempotent
  re-apply unchanged.
  Depends on WP1.
- **WP3 — aggregated emission + caps**.
  Files: `bins/spc-pool/src/payout.rs` (`emit_due_ark_orders`
  `payout.rs:1623-1689`: per-`sp_id` sum of unsent rows; emit one order per
  `sp_id` when the sum `≥ ark_min_overage_sats` or the oldest row is past
  `ark_overage_max_delay_blocks`; `order_id =` tagged hash over
  `(sp_id ‖ oldest covered block_hash)`), `crates/spc-store/src/postgres.rs`
  (migration: covered-row linkage for aggregated sends; `sent` bookkeeping
  per covered row), `bins/spc-pool/src/ark.rs` (`max_order_sats` default =
  ladder-derived max overage; check itself unchanged), `config.rs`
  (`[ark] min_overage_sats = 10_000`, `overage_max_delay_blocks = 144`).
  Tests: accumulation below threshold emits nothing; crossing emits exactly
  one order for the sum; age cap forces emission; retry keeps `order_id`
  (dedupe, no double-pay); receipt handling marks every covered row;
  `failed_to_pending` restores the whole sum to `pending` (never forfeit).
  Depends on WP2.
- **WP4 — agent verification**.
  Files: `bins/spc-agent` (own-payout check compares against
  `band_floor(owed)`; `/api/sp/me` reconciliation becomes two-part per Q7;
  ladder fetched from `/api/pool`, pinned per block, ambiguity = hard
  reject).
  Tests: agent audit unit tests band-aware; mismatch in either leg alarms.
  Depends on WP2 (needs settled rows) and WP3 (aggregation state visible via
  API).
- **WP5 — e2e + docs**.
  Files: `testing/regtest-harness` (banded scenario: coinbase outputs on
  ladder edges, overage VTXOs at `H + 6`, aggregation, float sizing),
  `README.md` (threat text mirrors §11.7).
  Depends on WP3/WP4.

## Risks (top three, tracked in the packets above)

1. **Settlement divergence is a halt, not a warn.** `paid ==
   band_floor(pay_routed)` is equality; any ladder ambiguity
   (front/settlement config skew, float arithmetic — hence integer-exact
   WP0, snapshot-pinned ladder reference) stops booking per block via the
   existing fail-closed recompute (`payout.rs:491-503`).
2. **Ark-side operational load is new and bursty**: up to ~256 orders per
   found block at `H + 6` where today there are ~0, against server limits
   that are discovered, not assumed (`docs/decisions/2026-11-02-bark-adapter.md`),
   and a float that grows from ~0 to ~2.1e7 sats per expected find
   (Q3). A mis-sized float or a capped server turns into
   `failed_to_pending` churn — visible, alarming, but safe (never forfeit).
3. **Miner-contract complexity in the agent**: verification is now two-part
   with aggregation state on top; a stale or ambiguous ladder at the agent,
   or a bug in covered-row bookkeeping, either false-alarms honest pools or,
   worse, masks theft. WP4's hard-reject rule and WP3's deterministic
   covered-set keying exist to keep both failure modes loud.
