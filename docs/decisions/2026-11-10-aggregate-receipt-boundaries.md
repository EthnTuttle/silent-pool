# 2026-11-10 - Aggregate receipt and authorization boundaries

## Authority

Parent-directed fixes for the five concrete findings in the independent
2026-11-10 ladder aggregate review, against frozen source
`415d21fa88ad2124712cc1fe9a8400bc7fbdd951bd005f223f92f879068f2602`.
This corrects implementation boundaries under the binding 2026-11-09
aggregate policy and concurrent 2026-11-10 Rule-B clarification.

## Decision

- A1: independent payout-owned fresh-authorization dispatch, separate Redis
  connection, one correlated request consumer. Real deadline expiry preserves
  durable unknown send intent and never enables a second grant/attempt.
- A2: exact complete VTXO policy and full 33-byte server key match against
  frozen verified ArkInfo; retain all cryptographic, amount and grant checks.
- A3: aggregate mode rejects legacy receipts; Store serializes reserved-member
  exclusion with freeze even for explicit legacy callers.
- A4: orphan reversal is per identity, preserving reserved/sent effects while
  reversing unrelated reversible rows once. No-authorization cancellation and
  orphan handling share the transaction boundary and Rule-B lifecycle update.
- A5: durable bounded quarantine before ACK, per-item error handling, and a
  restart-persistent fair PEL cursor. Unknown proof evidence stays reserved.

## Schema and interfaces

Migration 0010 adds frozen `server_pubkey`, `ark_receipt_disposition`, and
`ark_receipt_cursor`. Migration 0009 and SPCB/3 carry the Rule-B monetary
provenance described in the separate decision. `EconomicChanged` (0x05) is an
SPc-side reload hint, never authority. No identity/member/block mapping is
added to Ark messages; client order/grant/proved-receipt formats stay at their
existing versions. Pool Sent remains a retrieval hint to the wallet.

Edited SPEC §5.1-§5.3 and §11.3, WIRE §3.1-§3.2 and §3.5, ARCHITECTURE's
stream table, and MULTI-REGION §6/§10 to remove the superseded full-paid rule.

## Verification boundary

Source and regression cases are authored; current verification is blocked by
the mandatory disk floor. Earlier 260 pool tests, three aggregate PostgreSQL
tests and the stock-Bitcoin 990-output run remain predecessor evidence only.
A tested combined freeze, the five focused boundary suites and native
consecutive-pending proof are required before marking these findings fixed.
Funded stock-captaind/rbitcoin lab acceptance remains separate.
