# 2026-11-08 - Bootstrap reservation durability in CommandLog

## Scope

Runtime plan v5 section 4.1. This is controller authorization accounting,
not another money ledger or an implementation of native PoW.

## Decisions

- The existing CommandLog has an explicit traffic-strict mode and records
  operation limits, boot id and absolute monotonic deadline before reserving.
- HashReserve binds operation, incarnation, unique batch id, reserved attempts
  and cumulative debit. At most one batch is unresolved. Authorization is
  returned only after write, flush and sync_data all succeed.
- Any strict-mode write/flush/sync failure poisons further writes. No
  authorization is returned and no later append follows an uncertain tail.
- Strict replay refuses malformed complete lines, gaps, invalid cumulative
  arithmetic, duplicate batches or missing operation/reservation history.
  Only a syntactically incomplete final unacknowledged tail is repaired after
  every complete record validates. A complete non-newline tail is refused.
- Reconciliation is idempotent, records measured attempts separately, and
  never refunds reserved debit. An old-incarnation reservation cannot be
  redeemed by a new incarnation; unknown work stays fully charged.
- Legacy serialization and the approved principal-bound hash scheme remain
  unchanged. A budget-bearing log requires the strict reader.

## Sections edited

`command.rs`, `command/traffic_budget.rs`, bootstrap control seam and tests.
No wallet journal, snapshot, proof, maturity or signer code is changed. The
SPEC changelog entry is supplied separately for serial integration.
