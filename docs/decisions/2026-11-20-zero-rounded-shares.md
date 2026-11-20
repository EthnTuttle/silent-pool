# 2026-11-20 - A zero-rounded share is on no rail

## Authority and gap

Packet `wp-payout-liveness`, from a preserved lab halt (run
`20261119T112514Z-play5`). DESIGN.md and SPEC §4.4/§4.6/§4.7/§11.1 define
`raw(id)` with per-term floors, say the floor remainders fall to the pool
output, and route every `owed(id)` to a rail. None of them says what happens
to an identity the window credits whose `owed(id)` is exactly zero. §4.6 (e)
("an id is on exactly one rail per block") and §4.7 ("for id in window") can
be read as requiring an entry; §11.1 has no branch that could carry one. The
two implementations read the gap differently, which is the defect below.
This record closes the gap. It changes no amount: `raw`, `owed`, `pay_P`, the
ladder, forfeiture, Ark delivery and Rule B are untouched.

## Rule

```
published(id)  <=>  owed(id) = raw(id) + pending(id) > 0
```

- An identity with `owed(id) = 0` is on **no rail**: no distribution entry,
  no JD weight, no frozen balance, no ledger row, no Ark order, no forfeit.
  It is owed nothing, so nothing is taken from it. Its floor remainder is
  already part of gross `K = T - sum(raw)` in the pool output (§4.4).
- An identity with `owed(id) > 0` is published and is on exactly one rail
  (§4.6 (e), §11.1), unchanged.
- Settlement requires an entry for exactly the published identities: every
  id with fresh `raw(id) > 0`, and in pool-template mode every id with frozen
  prior `pending(id) > 0`. A missing one is still `WindowMismatch` and holds
  the block (fail closed). JD settlement keeps its existing scope: it checks
  the window's fresh credits, not prior pending.
- Both sides apply one function. `spc_accounting::published_ids` (per-id
  predicate `is_published`) is the set `route_tiers_with_keys` and
  `route_jd_tiers` iterate at build and the set `reconstruct_settlement`
  requires at settlement.

Conservation is unchanged and still checked at settlement:
`paid + ark + forfeit + E = raw + P` per published row and
`pay_P = K + sum(ark) + sum(forfeit) + sum(E - P)`. An unpublished id
contributes `raw = P = 0` to both sides.

## Why not a zero-sat entry

Every rail means something that zero sats cannot: `ark_due` schedules a
send (a 0-sat order), `pending` is a liability, `forfeited(id) > 0` is the
only forfeit §4.6 (f) describes, and a coinbase row needs keys and a rung.
A new "none" rail would be a wire change (WIRE-FORMATS §3.1, SPCB entry
format) for a row that moves no value. The routing already dropped zero
balances (`zero_balances_are_dropped`); settlement was the side that
disagreed.

## Observed failure

Regtest halves the subsidy every 150 blocks: 4 sats for heights 4500-4649,
2 sats from 4650, `fee_bps = 100` so `r_net = T`. Pool B, job `f38ddc0e`,
height 4650, `Q = 65`, 11 slices:

| id | window work | raw at height 4649, T=4 | raw at height 4650, T=2 |
|---|---:|---:|---:|
| `da2443ea..` | 38 of 66 | floor(4*38/66) = 2 | floor(2*38/66) = 1 |
| `fdda338d..` | 28 of 66 | floor(4*28/66) = 1 | floor(2*28/66) = 0 |

(The height-4649 window, job `f38ddc0d`, had 13 slices and the same 38/28
split of 66; it published both ids and settled.)

The front published one entry (`da2443ea..`, rail Ark, 1 sat) and a single
2-sat pool output. Settlement recomputed `raw = {da..: 1, fd..: 0}` and
required an entry for every key of that map, so `fd..` at zero sats was
reported as `WindowMismatch` on every retry. Pool A failed the same way at
height 4651 (`b9b8d4ba..` 40/66 -> 1 sat, `355bc9fd..` 26/66 -> 0). With the
canonical pool block unvalidated the Rule-B view stayed incomplete and both
fronts stopped issuing work (see below). The lab hit it at a 2-sat reward,
but the condition is `r_net * d_id / D_win < 1` with no fees and no pending:
any share small enough against the window, at any reward.

## The hold is kept

One unsettleable canonical pool block still stops template building. That is
the designed fail-closed behavior, not part of this change: SPEC §4.6 Rule B
needs the *complete* canonical validated unbooked set to compute available
pending, SPEC §5.3 says dropped or reordered hints "cannot authorize work
from an incomplete view", and decision 2026-11-10 (provisional pending net
debits, "Publication and recovery boundary") says unknown pool block events
hold accounting readiness. The fix removes a false mismatch; it does not let
a front build on unreconciled economics.

## Sections affected

SPEC §4.6 (routing pseudo-code and invariant (e)), §4.7 (`entries`), §7.4
(settlement check and slice wait) and §11.1 (tier policy). The body hunks
and the frontmatter bullet are in `docs/changelog/2026-11-20-payout-liveness.md`
for the merge session. WIRE-FORMATS §3.1 needs no change: one entry per
published identity and frozen balances covering exactly the entries already
hold.

## Verification boundary

Unit tests encode the lab window for pool B at 4 and 2 sats (failing with
`WindowMismatch` before the fix), prove a window owing an unpublished
identity still fails closed, and check conservation at 0-4 sat rewards in
`spc-accounting` (v1 and v2 raw, pool-template and JD routing) and through
publication plus settlement in `spc-pool`. No lab run was made on this
branch: rewards of 1 and 0 sats (heights 4800+ and 4950+) are covered by
those tests only.
