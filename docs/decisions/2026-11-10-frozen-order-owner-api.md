# 2026-11-10 - Exact frozen signed orders for the authenticated owner

## Authority

Parent-authorized AP3 blocker resolution. SPRP/1 contains the grant and VTXO
proofs but not the signed order required to validate the grant's order digest.

## Decision

Add `GET /api/sp/me/ark-aggregates/<order_id_hex>/order` to the existing private
SPc API, using its fresh single-use B_scan challenge. Select by authenticated
sp_id and order_id before reading any order bytes. Return immutable canonical
stored bytes after ID, digest, frozen fields, signature and any grant-digest
checks. Bound the read and response by ArkSendOrder's actual 348-byte codec
maximum. No mutable destination reserialization or file/SQL selector exists.

HTTP 200 is application/octet-stream and no-store. Unknown/other-owner is
404; malformed ID is 400; unavailable/integrity failure is 503; authentication
failure is 401. Unknown is not NoEffect. All lifecycle states remain readable
and wallet watches remain active until their existing terminal proof boundary.

## Scope and checks

SPEC §11.3 and WIRE §3.8 document this bounded addition. No Ark-role/server
message or wallet RPC changes. The API-only role uses the configured public
order key, not the signing secret. Added real-store/router cases cover fresh
owner proof, wrong owner, unknown/corrupt bytes, signature/digest binding and
uncertain-state retrieval. Source is authored; execution waits for the next
parent-assigned heavy slot.
