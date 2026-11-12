# 2026-11-10 - Net pool value includes signed pending repayment

## Authority and contradiction

Parent independent math review of the retained r4 failed candidate, together
with SPEC §4.6/§11.9 and the binding 2026-10-12 PR203 conservation correction.
The Nov-3 decision's `fee + ark_due + error-held` funding shorthand and the
later reserved-pool wording incorrectly suggested an independent gross floor.
They cannot override signed delta accounting. This is a code/spec correction,
not permission to pay partial entitlement or waive fees.

## Rule

Let K=T-sum(raw), P be frozen available prior, E resulting error pending,
and delta=E-P. Per identity:

```
paid + ark + forfeit + E = raw + P
pay_P = T - sum(paid) = K + sum(ark) + sum(forfeit) + sum(delta)
```

Gross fees are already removed from fresh raw. A negative delta repays old
liabilities; the lower current net pool UTXO is offset by liability reduction
and previous holdings. Pool-template routing therefore retains the checked
T-sum(actual full ladder paid) bound and removes the extra raw.pool_sats floor.
It does not substitute a gross-fee floor. Rungs remain downward, greedy and
complete; actual paid above T fails without trimming. Total owed above T may
remain legitimately held when actual coinbase payments fit.

raw.pool_sats and Routing.pool_fee stay gross K. Legacy JD routing, weights,
O11/F scaling, Ark send policy, caps, float and maturity are unchanged.
Known-complete empty windows use raw=0/K=T and the same ordinary frozen-P
routing/settlement/ownership checks. They can repay P; empty is not P=0.
Unknown history, missing high water, missing frozen v2 Q and false outputs
remain invalid. With zero prior the empty coinbase remains pool-only.

## Actual failed-state regression

Retained `/tmp/opencode/ladder-native-rule-b-run-r4`: log record 424, window
67/Q65, credits 23/22/22, T=5,000,000,000, fee_bps=100, fees=0. Transaction
2509 booked P before the first erroneous guard at log record 429.

| row | raw | P | full paid | E | delta |
|---|---:|---:|---:|---:|---:|
| A | 1699253731 | 386759 | 1699500000 | 140490 | -246269 |
| B | 1625373134 | 566201 | 1625500000 | 439335 | -126866 |
| C | 1625373134 | 547037 | 1625500000 | 420171 | -126866 |

K=50,000,001; paid=4,950,500,000; pay_P=49,500,000; delta=-500,001.
It fits T and reserves 500,001. This is reconstruction of a failed candidate,
not a claim that it was already accepted by a node. The old trace, database,
binaries and failure remain preserved. QA's independent r12 replay confirms
the same vector; its prior suite counts are not relabeled as new tests here.

## Verification boundary

The tested 419b5f snapshot was archived before editing as
`/tmp/opencode/ladder-pool-correction-predecessor-419.tar.gz`, SHA256
`9b2c9bc55f50606bbff7661cde6184bcfc4eeba7a52bf48e313544cbb7fc6386`.
Its 483/47/13 counts remain predecessor evidence. New arithmetic, settlement,
ownership, property and JD-isolation regressions are authored. Rust and a new
same-budget native consecutive-pending run await the next assigned slot.
