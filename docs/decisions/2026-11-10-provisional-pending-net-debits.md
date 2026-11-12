# 2026-11-10 - Provisional pending reservations use net debits (Rule B)

## Authority

Parent-authorized clarification after independent math/spec analysis
`ses_e3becebadffelAff0lIskGbFRx`. DESIGN's fresh per-block PPLNS and SPEC §4.6
`owed = raw + pending` outrank MULTI-REGION §6's full-unbooked-paid subtraction.
The binding 2026-10-14 multi-region decision requires provisional state but does
not define a full-paid formula. This clarifies that state; it does not suppress
fresh earnings or introduce capacity deferral. The previously tested source
`415d21fa88ad2124712cc1fe9a8400bc7fbdd951bd005f223f92f879068f2602`
and stock-Bitcoin component evidence are preserved separately.

## Rule B

For each identity B is booked pending; U is the COMPLETE canonical validated
unbooked set. A job freezes its available prior pending P_b. Actual mined-output
validation yields resulting error pending E_b and delta_b = E_b - P_b.
Reserve c_b = max(0,-delta_b); g_b = max(0,delta_b) is unavailable until booking.
R = sum_U c_b, A = B - R with checked arithmetic and 0 <= R <= B.
NEW owed = fresh raw + A. Never subtract full coinbase/Ark payments from raw.

- Found: reserve c_b, leave B unchanged, persist provenance/lifecycle/revision.
- Book: atomically B += delta_b and R -= c_b. A_after = A_before + g_b.
  Apply delta additively, never overwrite the current balance with E_b.
- Unbooked orphan: B unchanged; release c_b; discard g_b. Never apply -delta_b.
- Booked reorg: reverse actual booked effects under the existing irreversible
  Ark loss/pinned-aggregate rules, reconcile dependent reservations before
  exposing a coherent canonical view. No possibly sent value is refunded.
- Exact event replay is idempotent; conflicting payloads reject. Late Found
  cannot resurrect booked/cancelled state. Recanonicalization requires a new
  verified transition, not event replay or a higher revision alone.

Required vectors: P=0/raw=1M on consecutive blocks pays 1M each; P=500k/raw=1.1M,
paid=1.5M/E=100k reserves 400k and leaves 100k available; JD P=500k/raw=100k,
O11 claim/paid=300k/E=0 reserves 500k, not 300k. Unknown keys with P=0/raw=1M,
E=1M reserve zero, and that positive credit is unavailable until booking.
JD uses actual O11 scaling and fixed-output F; no denomination shaping on JD.

## Publication and recovery boundary

Persist frozen P and job provenance, actual validated delta/c, per-block
canonical ancestry/lifecycle, and economic revisions. Bootstrap atomically
returns booked B, complete reservation records, canonical reconciliation anchor
and revision/cursors. An absent reservation view is UNKNOWN, not an empty list.
Descendant publication requires a complete view bound to its actual parent
hash. A tip heartbeat/stream cursor alone is insufficient. Unknown pool block
events hold accounting readiness; raw non-pool coinbase/source evidence proves
irrelevance before advancing. Same-front found work cannot reuse consumed
pending while waiting for confirmation. V2 arithmetic errors invalidate work;
the explicitly legacy v1 saturating adapter remains separately identified.

SPCB/3 is the bounded amendment for mandatory frozen economic-job provenance;
SPCB/2 remains a historical record with unavailable Rule-B provenance, never
implicitly P=0. Economic state stays SPc-side; no identity/member/block mapping
is added to Ark messages. New economic tables and schema/API fields are recorded
in WIRE-FORMATS and the owner handoff. Over-revenue full entitlement still fails
closed. This is not a funding mechanism or final rbitcoin/funded-lab acceptance.
