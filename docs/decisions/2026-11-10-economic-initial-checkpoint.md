# 2026-11-10 - Journal the actual initial economic state

## Authority and defect

QA identified that migration 0009 inserts economic_state before its history
trigger exists. Complete-from-empty replay therefore lacked its first row.

## Decision

Add migration 0011 without rewriting any applied migration/history. Under
monetary locks it captures actual state bytes, bounded snapshot bytes/digest,
prior migration checksums, history watermark, economic revision and SQL
transaction/snapshot coordinates. The checkpoint INSERT itself enters the
append-only source history. Only an exact fresh empty database qualifies as
`fresh_empty`; nonempty/inconsistent legacy data stays `unverified_existing`
with all balances preserved. Unknown is not an implicit empty state.

Economic reads/initialization require a valid seed matching the immutable
history copy. Alteration, removal or an unverified compatibility snapshot
holds readiness. The private collector API/columns are WIRE §3.11 and SPEC
§5.1. Real-PG positive replay and missing/altered/nonempty cases are authored;
actual checkpoint bytes and test receipts await the assigned execution slot.
