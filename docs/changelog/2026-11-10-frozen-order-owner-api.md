# Frozen-order owner API

The private SPc API now has the source implementation for
`GET /api/sp/me/ark-aggregates/<order_id_hex>/order`: exact stored signed
ArkSendOrder bytes, application/octet-stream, maximum 348 bytes. Ownership
is checked before access and immutable bytes are verified before return.
The existing fresh B_scan challenge is required for each read. See WIRE §3.8
and the same-date decision. Runtime tests are pending the assigned build slot.
