# 2026-11-08 - Lab traffic contract T0 rework (F1-F8)

Reworked the T0 traffic-contract packet per the Fable review: the traffic
capability gate now runs in `RunEngine::prepare` before any filesystem,
lock or authority effect and on the adopt path before the owner lock
(`traffic_capability_unrecorded` when no record exists, even if the node
config declares the capability name); the AP-1a projection gained the
frozen 17-row expected-check table with exact id-method binding, strict
report provenance/target/profile validation and full per-check rows the
gate recomputes both verdict stages from; the single `satisfied` verdict
became the two-stage `recognized` (preflight gate) vs `behavior_proven`
(false by construction for read-only reports; T3 consumes its own
exercised evidence); the Fable-approved r2 contract
`f4fc3e04...` is bound as the sole approved lineage (hash + table +
decision in one change; the r1 candidate stays recognized-unapproved
diagnostics; unknown hashes fail closed); `send_user_payment` gained the
explicit fail-closed `destination_address` field (regtest p2pkh,
resolving the plan 6.6/12 ambiguity); actor/driver ids are typed
`ActorId`s; the history journal is confined to `traffic/`; the protocol
string is exact (no trim) and the planned-schedule seed domain is frozen.
Traffic scenarios still cannot start until T1 records a capability -
the safe-unavailable state. Decisions:
`docs/decisions/2026-11-08-lab-traffic-contract-t0.md` and
`docs/decisions/2026-11-08-traffic-ap1a-lineage-r2-approved.md`.
