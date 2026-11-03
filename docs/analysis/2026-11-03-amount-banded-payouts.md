---
title: "Amount-Banded Payouts"
type: analysis
format: markdown
date: 2026-11-03
status: closed
supersedes: none
summary: "Banding does not remove amount linkability, it bounds it to ceil(log2 L) bits per block; overage-over-Ark re-exposes exact sats to the Ark server and is not worthwhile, while overage-to-pending is feasible with small changes as a v2-scale mitigation."
---

# Amount-Banded Payouts

Convention: **measured** means read from code/config at the cited file:line on master (67a5a67); **(estimate)** marks arithmetic or modeling on top of those values. Section citations are `SPEC x.y` (`docs/SPEC.md`) unless prefixed otherwise.

## Question

If coinbase payouts were quantized into strict satoshi bands and any overage above the band floor were paid over Ark, would that remove the amount-linkability concern (SPEC §0 line 97: a stable hashrate share is fingerprintable by `amount_i / T` across blocks), and is it feasible in the current accounting?

## Short answer

No to removal, yes-with-changes to feasibility. Banding converts the on-chain leak from exact sats to a band index: with `L` bands an on-chain observer learns at most `ceil(log2 L)` bits per miner per block instead of an effectively unique value. But the Ark overage carries the exact remainder, and the Ark server already sees every Ark-tier amount (SPEC §11.7 line 1010: "amounts map to the pool's known payout distribution"), so the exact-amount leak is relocated to the designed cross-layer observer, not removed. Worse, it would give every coinbase-tier miner an Ark footprint, where today they have none. The project's own research reached the general form of this conclusion: "Denomination ladders are the real defense, and they bound the leak rather than removing it" (`docs/research/wiki/concepts/coinbase-amount-linkability.md` line 73), and the WabiSabi ceiling applies: the on-chain amount multiset and its fixed sum are public (same file, lines 53-63).

| Variant | Chain observer learns | Ark server learns | Feasibility |
|---|---|---|---|
| Exact (today) | exact `pay_i` per output | sub-`C` amounts only | status quo |
| Band + overage to Ark | band index | exact amounts for ALL paid miners, per block | feasible, not worthwhile |
| Band + overage to pending | band index | sub-`C` amounts only (unchanged) | feasible with changes; smallest viable design |
| Ark-only payouts | one pool output | everything | rejected by design (SPEC §0 custody, §11.1) |

Historical note: the spec once had a fixed 500 000-sat tier band and abolished it in favor of threshold-at-dust (SPEC frontmatter changelog, 2026-10-13 entry, line 25). Amount mitigations are otherwise deferred: ROADMAP R1 ("v2 candidates: randomised slot count, deferred/batched pending payouts -- out of v1 scope", ROADMAP.md line 290) and SPEC §10 O8 (line 862). Anything here is new normative behavior requiring a decision record and changelog entry per AGENTS.md.

## 1. Where exact sats come from today (end to end)

Credits are weights, never sats. Shares book `ShareRec { sp_id, d, job_fees, ... }` (`crates/spc-accounting/src/lib.rs:30-41`) with credited difficulty `d = min(share_difficulty, max(1, round(network_difficulty)))` (`bins/spc-pool/src/sv1.rs:108-110`, rounding at `sv1.rs:94-104`; policy in `docs/decisions/2026-11-02-share-target-policy.md`; SPEC §4.3). Slices aggregate per-id `(sum d, sum d . f)` as u128 (`lib.rs:115-136`); the window is whole slices back from the finding share until `D_win >= N x d` (`lib.rs:198-223`).

Revenue: `front` fetches a template and derives `subsidy`, `fees`, `coinbase_value` (`bins/spc-pool/src/template_rpc.rs:82-84`), then calls `raw_payouts(&window, template.subsidy, template.fees, self.cfg.fee_bps)` (`bins/spc-pool/src/front.rs:594`). Inside (`lib.rs:246-305`), all measured:

- `r_net = subsidy - floor(subsidy . fee_bps / 10_000)`; same for `f_net` (`lib.rs:259-261`; `fee_bps` default 100, `bins/spc-pool/src/config.rs:255-257`);
- subsidy term, one floor per identity: `floor(r_net . d_window / D_win)` (`lib.rs:288`);
- fee term, one floor per (identity, slice): `floor(f_net . df / (t . DF_S))` (`lib.rs:276-280`);
- `raw(id)` = sum of both, clamped to u64 (`lib.rs:288-291`); `pool_sats = subsidy + fees - sum(raw)` absorbs the fee and all floor remainders (`lib.rs:294-298`; SPEC §4.4 line 433: "floors per term; remainders fall to the pool output").

Tier routing (`route_tiers`, `lib.rs:405-523`): `owed = raw + pending` (`lib.rs:414-424`), sorted descending (`lib.rs:426`), coinbase capacity `T - pool_sats` (`lib.rs:437`); an id is kept iff `owed >= C` (`lib.rs:446`; `C` default = dust = 330, `config.rs:133-134,300-302`) and `kept.len() < 256` and `used_wu + 172 <= budget_wu` (`lib.rs:447-448`; defaults 256 slots, 172 WU/output, 50 000 WU floor measured at `config.rs:303-305,315-320`), then pays the full exact `owed` (`lib.rs:450-457`), with one capacity-clamped partial-pay edge (`lib.rs:458-469`). Trimmed `owed >= C` goes wholly to pending (`lib.rs:475-480`); sub-`C` goes to Ark / pending / forfeit (`lib.rs:482-506`; SPEC §11.1). `hold_undeliverable` demotes keeps with unknown keys to pending at full `owed` (`front.rs:959-993`). `build_distribution` emits `[pool(pay_p)] ++ [P2TR(P_i, exact sats) per kept id]` (`lib.rs:696-764`). This is SPEC §2.3 line 180 (`output_i = P2TR(ser_x(P_i)), amount = pay_i`) and §4.6 line 460 (`pay(id) = owed(id)`): exact-satoshi output semantics are normative.

Settlement (`bins/spc-pool/src/payout.rs`): re-reads economics from `getblockstats` (`payout.rs:249-268`), hard-checks `T_actual = subsidy + fees` against the mined coinbase (`payout.rs:491-503`), re-runs the identical `raw_payouts` (`payout.rs:539-544`), computes `owed = raw_sats + before` (`payout.rs:598-602`), and fail-closes on `paid < raw_sats || paid > owed_snapshot` (`payout.rs:656`); the booked residual is `after = amount - paid` (`payout.rs:688`).

A banding step would round exactly one thing: `pay(id)` for kept ids, inserted between `route_tiers` and `build_distribution` (`front.rs:604-640`). Everything upstream (`raw_payouts`, `owed`) must stay exact, because settlement and the miner's own audit recompute it.

## 2. Leak model: what banding hides and what it leaves

Observers and today's exposures (DESIGN.md lines 175-205; SPEC §0 line 97, §11.7 lines 1006-1011):

**On-chain observer.** Sees the coinbase output multiset: values, count `k + 1`, block timing. Amounts are effectively unique per (miner, block): `raw(id)` is a u128-floored rational of window difficulty shares and per-slice fee sums, so `amount_i / T` is a high-precision cross-block fingerprint (SPEC §0 line 97). The subset-sum structure is maximally hostile: one input of publicly known value, no input shuffling, no padding possible (`docs/research/wiki/concepts/coinbase-amount-linkability.md` lines 31-51). `pay_P` (the single pool output, `lib.rs:706`) leaks only the aggregate `T - sum(pay)`.

**Ark server.** Sees destinations, amounts, timing of all Ark-tier sends; pool bursts ~6 blocks after each pool block are fingerprintable; amounts map to the pool's payout distribution; boards ~100 blocks out link the pool's own outputs to its Ark wallet (SPEC §11.7 lines 1010-1011). Today this covers sub-`C` miners only; coinbase-tier miners emit no Ark orders at all (one order per `(block_hash, sp_id)` booked from the ledger, `payout.rs:1623-1689`; `crates/spc-store/src/postgres.rs:495-508`) (measured).

**Pool.** Sees everything regardless (DESIGN.md line 175). Banding changes nothing here.

What banding **hides** (chain observer): all sub-band-width information; miners colliding in the same band in the same block are amount-indistinguishable; per-block resolution drops to the band index.

What it **leaves**:

1. **Band index and trajectory.** Band transitions across blocks still track hashrate changes coarsely, and the ladder must be public for miner verification (§11.6), so band boundaries are known attacker-side ("the ladder itself" leaks nothing new but sharpens matching).
2. **Heavy tail.** Pool hashrate is heavy-tailed, so the largest miner often occupies a singleton band and stays fingerprintable (research note, lines 118-123).
3. **Timing and output count.** Block at `H`, sends at `H + 6`; `k + 1` outputs exact. Randomised slot count is a different (deferred, R1) mitigation.
4. **Ark-side exact amounts (overage-to-Ark variant).** The overage is exact sats. The Ark server sees the coinbase bands publicly and the overage stream privately; since overage `= pay - band_floor(pay)` and the pool's payout distribution is known (SPEC §11.7 line 1010), it can re-associate overages with band positions. Every kept miner also gains a per-block Ark footprint, enlarging the server's cross-layer view beyond today (measured: today only sub-`C` rows exist).
5. **Tier transitions** remain observable to anyone watching both layers (DESIGN.md lines 198-205).

## 3. Band design options

- **Fixed powers of two above `C`.** `L = {C} union {2^k > C}`. With `C = 330` and block revenues around 3.3e8 sats (worked example era, SPEC §4.5, measured `tests.rs:74-84`), coverage needs 512 .. 2^29, about 21 bands (estimate), i.e. at most 4.4 bits/block. Simple to spec, trivial to verify, smallest band `>= C >= dust` by construction.
- **Decimal grid** (`10^k . {1, 2, 5}`): finer relative width, more bands, human-friendly; same privacy shape per bit of ladder size.
- **Difficulty-scaled** (multiples of the per-share epoch value): amounts already cluster near such multiples by construction, so it mostly hides fee-term jitter; the unit varies with fees per block, giving a block-dependent ladder. Weakest privacy per complexity.
- **Per-block ladder** derived from `(subsidy, fees, D_win)`: recomputable by miners from type-32 data plus the block; but any template-vs-settled divergence breaks reconciliation. In pool-template mode `T_actual = T` holds by construction (SPEC §4.6 line 471; `payout.rs:491-503`), so it is technically sound there, but under JD the mined amounts differ from the snapshot (same line). Fragile; not recommended.

Width vs privacy: width `w` caps on-chain resolution; wider bands give more collisions but push more value into the overage rail (more custody or more delay) and enlarge worst-case remainder `w - 1`. Rounding must be **down** (`pay = band_floor(owed)`): rounding up would spend the reserved pool share (`capacity = T - pool_sats`, `lib.rs:437`). Ladder publication: like `C` today, a configured, validated parameter published at `/api/pool` (SPEC §11.1 line 892), with validation extended so the smallest band `>= C` (today's invariant `C >= dust_limit_sats` at `config.rs:563-576`, measured).

Multi-output denomination splitting (several standard outputs per miner) is excluded: invariant §4.6(b) is one output per `(sp_id, H)`, slots are capped at 256 with 172 WU each (measured above), and coinbase output count is firmware-bounded (research note lines 68-71).

## 4. Overage routing

`overage(id) = owed(id) - band_floor(owed(id))`, in `[0, w)`.

**To Ark (the variant in the question).** Mechanically routable: order IDs stay unique because one id is on at most one rail per block (§4.6(e)) and `order_id = sha256("SPc/ArkOrderId" || block_hash || sp_id)[:16]` (`payout.rs:1798-1808`, measured). But: (a) it breaks the SPEC §11.10 invariant "`max_order_sats = 0 # 0 = coinbase_threshold_sats (an Ark payout is always < C)`" (line 1054) whenever a band width exceeds `C`, which every useful ladder has; (b) overage can be dust-scale (1..329 sats) with arbitrarily small probability mass no matter the ladder -- micro arkoor orders are the exact small-balance exits SPEC §11.1 line 898 calls "prohibitively expensive to exit", and server-side limits like `max_arkoor_amount` are discovered per server, never assumed (`docs/decisions/2026-11-02-bark-adapter.md`); (c) it moves non-custodial coinbase value (SPEC §0 lines 76-78) into the 6-block-custodial / 28-day-trustodial Ark path (SPEC §11.7 line 1006), contradicting §11.1 line 894 (large balances should not sit under arkoor trust); (d) it gives the Ark server exact amounts for every paid miner every block. Rejected as the standard mechanism.

**To pending.** Overages book as ordinary pending balances. This needs no new rail: settlement already computes `after = amount - paid` (`payout.rs:688`) and books `pending_delta` for a coinbase-rail row -- the capacity-clamped partial pay path already produces exactly that shape today (`lib.rs:458-469`; booking at `payout.rs:684-697`) (measured). The carried remainder adds into next block's `owed` (`lib.rs:414-424`) and is paid when the banded total crosses upward; conservation is untouched since `pay_P = T - sum(pay)` already "backs every held' balance" (SPEC §4.6 line 462) and the §11.9 line 1040 identity absorbs overages inside `Delta pending`. Liability reporting (`postgres.rs:613-625`, `api_backend.rs:94-96`) needs no change; reported liability grows by the standing overage balances.

**Burn / forfeit to pool. Rejected.** Forfeiture is a deliberately narrow published policy for unregistered sub-`C` earnings only (SPEC §11.1 lines 895-896). Confiscating registered miners' remainders breaks the custody statement (§4.6 line 467) and the §11.6 reconcile-to-zero audit; AGENTS.md: "There is no donation fallback." Remainder attribution must stay with the miner (pending), never the pool -- note today's floor remainders to `pool_sats` (`lib.rs:294-298`) are sub-satoshi arithmetic dust, whereas a band haircut is up to `w - 1` sats by design; folding it into the pool share would be a hidden `fee_bps` increase.

Interaction summary (all measured cites above): `fee_bps` orthogonal (applies pre-band); smallest band must be `>= C >= dust_limit_sats` or a kept id could floor below dust; the kept-gate stays `owed >= C` so `band_floor >= C` holds if the ladder is built as in §3; `max_payout_slots = 256`, 172 WU/output, `budget_wu` and the §4.8 autoscaler (`lib.rs:580-656`) are all unaffected because output count per miner is unchanged; JD weights are full `owed` already (`lib.rs:806-809`), so nothing changes on the weights side.

## 5. Feasibility against the current accounting

- **Slice credits:** no change. `ShareRec`/`SliceAgg` carry difficulty weights only (`lib.rs:30-41,115-136`) (measured).
- **`raw_payouts`:** no change, and must not change -- settlement re-runs it byte-identically (`payout.rs:539-544`) and the §4.5 worked example is a to-the-sat pinned fixture (`tests.rs:74-84`).
- **`route_tiers`:** add a pure post-pass `pay = band_floor(owed)` over `kept` (or fold into the keep loop); pending deltas for kept ids become the overage (`lib.rs:455` currently books `pending_delta(0, old)`). Disjointness and conservation proptests (`tests.rs:1093-1411`) still hold because the rail partition is untouched and `paid + pay_P = total` survives any kept-amount reduction.
- **`build_distribution` / weights:** unchanged shape; the fingerprint covers whatever amounts are emitted (`lib.rs:755`). JD mode cannot band pool-side: the JDC computes output sats from weights at its own template (`SPEC §4.7` line 490, §4.6 line 471; `claim_jd` at `lib.rs:547-560`). Banding is pool-template-only unless a JDC-side convention is added -- say so in the decision record.
- **Settlement:** one load-bearing predicate change. The guard `paid < raw_sats || paid > e.sats` (`payout.rs:656`) must become `paid == band(owed_recomputed)`: `band_floor(owed)` can legitimately be below this block's `raw_sats` when pending is carried, so today's guard would false-positive. The downstream formula `after = amount - paid` (`payout.rs:688`) then books the overage to pending with zero further changes; JD `settle_jd` (`lib.rs:568-578`) is untouched. The band function must be pure over `(amount, static ladder)` -- a template-derived ladder risks front/settlement divergence, which the fail-closed recompute (`payout.rs:491-503,656`) would turn into per-block settlement halts.
- **Liability endpoint:** no code change; balances simply include overages (`postgres.rs:613-625`).
- **Type-32:** no wire change -- records carry difficulty, fees, roots, never sats (`crates/spc-share-ext/src/lib.rs:139-148`; emission `front.rs:861-863`), so the miner can still recompute `raw(id)` and then apply the published band function.
- **Miner's own-payout verification (the real contract change).** Today the invariant is exactness: the output at `OP_1 ser_x(P_i)` equals `owed` (SPEC §2.3 line 180, §4.6 line 460; §11.6 line 1000 "carries exactly `ark_due`" for the Ark side; fail-closed enforcement `payout.rs:656`). With banding the contract becomes two-part: `paid == band(owed)` in the coinbase and `pending' - pending == owed - band(owed)` in the ledger -- the data for the second half already exists per block in `/api/sp/me` (`api_backend.rs:102-117`, measured). `spc-agent`'s audit must become band-aware, and the ladder becomes consensus-critical pool policy: any ambiguity is a fail-closed reject per project convention.

## 6. Ark load (if overage went to Ark anyway)

Measured mechanics: one `ArkSendOrder` per `(block_hash, sp_id)` booked from the ledger, no cross-block or cross-miner aggregation (`payout.rs:1623-1689`; rows inserted `postgres.rs:495-508`); emission loop every 30 s (`payout.rs:49`); sends gated to `due_height = height + confirmation_depth` (6, `config.rs:312-314`); ark-side rejects `sats == 0` or `sats > max_order_sats` (`bins/spc-pool/src/ark.rs:108-109`), optional daily cap (`ark.rs:128-132`); failed sends re-credit pending, never forfeit (bark-adapter decision). Float target is "Ark-tier sats of the blocks expected in 100" (SPEC §11.10 line 1053).

Estimates for the overage variant: extra orders per pool block = number of kept miners `k` (today Ark order count is only the registered sub-`C` count), each `< w` with mean ~ `w/2` if amounts are uniform within band (estimate); expected overage flow per block < `sum(w_i)/2` over kept miners (estimate). No minimum-order or dust floor exists pool-side (`ark.rs:108-109` only bounds above), so sub-dust overage orders are possible for any ladder. Since aggregating overage across blocks until it clears a threshold is precisely what the pending rail already does, the Ark variant has no aggregation story that does not collapse into "to pending".

## 7. Privacy math

Let the ladder have `L` bands covering `[C, T]`. Measured anchors: `C = 330`, `T` on the order of 3.3e8 sats (SPEC §4.5 numbers), so exact amounts live in a range of ~2^20 (estimate from those anchors).

- **Exact (today):** an observation is an effectively unique u64; magnitude alone carries up to ~20 bits, and cross-block ratio matching on `amount_i / T` compounds it. SPEC §0 line 97 concedes this fingerprint as out of scope of mitigation today.
- **Banded, overage to pending:** per miner per block, the chain observer gets at most `ceil(log2 L)` bits (~4.4 bits for the 21-band example, estimate). The sub-band remainder leaves the chain entirely, folded into the single `pay_P` output that already mixes fee + rounding + all held balances (`lib.rs:706`; SPEC §11.2). Residual channels: band trajectories, singleton top-band miners (heavy tail), output count, timing. Against chain-only observers this is the strongest variant short of Ark-only.
- **Banded, overage to Ark:** identical chain view, but the Ark server -- a named cross-layer observer in SPEC §11.7 -- receives exact per-miner per-block amounts and can match them against the public bands. Against the cross-layer adversary the design already assumes, nothing improves, and the server's coverage of miners strictly increases.
- **Ark-only:** best chain privacy (one output), worst custody; rejected by the design's custody statement (SPEC §0, §11.1).

## 8. Verdict

**Feasible with changes; worthwhile only as overage-to-pending; the overage-to-Ark variant in the question does not remove amount linkability (it relocates exact amounts to the Ark server and enlarges its coverage) and should not be built.** Even the good variant only bounds the leak at `ceil(log2 L)` bits per block and does nothing for the top-of-ladder singleton, timing, or output count. This matches the project's existing posture (R1/O8 defer amount mitigations to v2; DESIGN.md line 203 lists "amount padding/rounding" under soft mitigations only).

Smallest viable design (v2 candidate): static public ladder `{C} union {2^k > C}` published at `/api/pool`; `pay = band_floor(owed)`; overage to pending; pool-template mode only; new pure `band()` in `spc-accounting` with proptests; the single settlement predicate swap at `payout.rs:656`; config validation `min_band >= C`; band-aware `spc-agent` audit.

If adopted it would touch: a new `docs/decisions/` record (plus SPEC frontmatter changelog entry, per AGENTS.md); SPEC §2.3 line 180 (`amount = pay_i`), §4.4 (rounding rule gains a band step), §4.6 lines 456-462 (`pay(id) = band_floor(owed(id))`, invariant wording), §4.7 (entries carry banded sats), §11.1 lines 883-890 (tier ladder line 1), §11.6/§11.9 (audit reconciliation becomes two-part), §0 line 97 and §11.7 line 1011 (threat-model text: amounts become banded amounts), and DESIGN.md lines 198-205. It would NOT touch §11.3/§11.8/§11.10, the type-32 wire format, or slice accounting.

Open questions:

1. Real per-band anonymity-set sizes under plausible hashrate distributions -- needs data; the heavy-tailed top miner may cap the practical gain at near zero for small pools.
2. Whether banding beats the already-deferred R1 candidates (randomised slot count, deferred/batched pending) on leak-per-complexity; epoch-batched pending may deliver most of the benefit with no public ladder at all.
3. JD-mode story: declare pool-template-only, or specify a JDC-side banding convention.
4. Miner UX: wallets see rounded outputs plus ledger dust; does two-part verification erode the clean "the coinbase output is what I was owed" story?
5. Interaction with operator-raised `C` (§11.9: raising `C` moves value across rails without changing `sum owed`) -- band edges multiply the number of such boundaries.
