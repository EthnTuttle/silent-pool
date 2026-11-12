# Full-rail decision observations v1 (2026-11-11)

Source work following QA `T3-fullrail-r19/PRODUCER-FIELDS-NEEDED.md`.
This packet starts new observations; approved 2314/r5/r16 evidence is unchanged.
All identity-bearing records stay in the SPc database/source trace.

## SQL envelope

Migration 0012 adds append-only `source_decision_observation`:
`observation_id bigint`, `schema_version smallint=1`, `kind text`,
`subject bytea(16|32)`, `attempt bytea(16)|null`, `transaction_id bigint`,
`snapshot_boundary text`, `observed_at timestamptz`, `body jsonb` (8 MiB cap).
`(kind,subject,attempt)` is unique when attempt is present. Exact replay of an
attempt returns its original record; conflicting bytes reject. INSERT is also
recorded in append-only `source_accounting_history` in the same transaction.

Body schemas:

- `spc.settlement-delivery-read/1`: block hash, full frozen economic job,
  payload digest, ordered queried identity census, one result per identity,
  complete/unknown status. Results include the actual delivery row and actual
  registration state/request rows, or explicit nulls from the same complete
  repeatable-read query. Failure is unknown, never an absence result.
- `spc.settlement-reconstruction/1`: delivery-read observation ID, block/payload
  binding, accepted ledger/deltas or reconstruction error. Its observation ID
  and transaction/history coordinates order the read before economic acceptance.
- `spc.front-derivation/1`: source run/session, front, height, job/generation,
  economic revision, public a_send_x, identity census and per-identity actual
  public-input availability, committed registration revision/request digest,
  Ark server/mailbox/epoch inputs, exact destination bytes and classified result.
  Public keys are allowed here; private scalars and ECDH secrets are excluded.
- `spc.authorization-decision/1`: exact request, caller clock, bracketed raw
  canonical observations, queried member census, outcome and exact signed grant
  bytes. This record commits atomically with the grant/aggregate SQL checks.
- `spc.authorization-row/1`: INSERT/UPDATE/DELETE full before/after rows,
  including exact signed grant bytes. It survives deletion of the working row.
- `spc.authorization-consumption/1`: exact opaque consumption request/grant,
  actor clock, DB receipt clock and digest. Durable receipt precedes cosign;
  a later failed/expired escape remains pinned, not a no-effect/refund claim.

Settlement `results` entries are `{sp_id,delivery,registration_state,
registration_request,epoch_mailbox}`. Each nested row is either SQL null or the
actual `to_jsonb` row from the same query:

| Row | Exact columns |
|---|---|
| delivery | sp_id, ark_id, b_ark, mailbox_mode, mailbox_m, mailbox_d, updated_at, epoch_fallback |
| registration_state | sp_id, last_request_id, signed_fields_digest, registration_rev, updated_at |
| registration_request | sp_id, request_id, signed_fields_digest, outcome, registration_rev, error_code, created_at |
| epoch_mailbox | sp_id, height, mailbox_mi |

The result census follows queried identity order, including duplicates if the
caller supplied them. Only a complete read has an array of results; an unknown
read uses `results:null`. Delivery-row presence remains the exact pre-existing
settlement decision; provenance fields do not change rail selection.

Authorization decision outcomes are `issued`, `canonical_read_unknown`,
`aggregate_absent`, `request_not_issuable`,
`members_not_canonical_or_reserved`, and `issuance_unknown`. A null
`canonical_read` on the retained legacy library entry point is missing proof,
not canonical-at-time evidence. The actual payout worker uses the observed API.
Consumption bodies have `status:recorded|unknown`; unknown attempts have no
durable consumption receipt and never resolve the monetary order. Accepted
consumption attempts are keyed by order ID/nonce; repeated identical bytes return
the original ID/time, including after working-row scrub. Conflicts are refused.

Full authorization-row bodies use PostgreSQL JSON bytea encoding (`\\x...`).
Rust-authored byte fields use lowercase hex except `FrozenJob` and
`CanonicalRead`, which use their typed byte-array encoding. Repeated reads have separate attempts.
No consumer may treat a later read or migration-era row as past-state proof.

`economic_block.reconstruction_observation` is a nullable immutable reference
to the exact accepted reconstruction. Its referenced delivery read must precede
it and match the same block/frozen payload and queried identity census. Existing
rows remain null; no observation is invented for an old block. New authoritative
acceptance requires this reference. Working-row deletion cannot delete evidence.

`CanonicalRead` fields: `schema`, `source_session[16]`, nullable
`started_unix_ms`/`finished_unix_ms`, `authoritative`, nullable `tip`/`tip_hash`,
nullable `tip_after`/`tip_hash_after`, `members[{height,expected_hash[32],
observed_hash[32]|null,read_status}]`, and `outcome`. Member read statuses are
`not_read`, `present`, `unavailable`, or `read_failed`; an unqueried member is
never an observed absence. Hashes are exactly the internal byte
order returned by the existing ChainView. Grant observations include the complete
queried SQL member census and the actual joined member/block/payout rows. Grant
checks and their observation use one repeatable-read SQL transaction.

Front `rows` retain `scan_public`, `spend_public`, `coinbase_key_status`,
`registration_presence`, `registration`, `server`, `epoch_mailbox`,
`mailbox_selection`, `commit_provenance`, `destination_bytes`, `outcome`, and
on error `error_stage`/`error_reason`. Registration includes
`commit{request_id,registration_rev,signed_fields_digest|null}`. This optional
metadata is also transported in the private bootstrap JSON. Front events retain
their existing wire bytes; their request/revision resolves the exact immutable
request row, never the current delivery. Unavailable provenance is explicit.

## Opaque consumption edge

Front derivations travel only on private SPc `spc:source:decisions` as a bounded
JSON `FrontObservation{schema_version:1,session[16],attempt[16],body}`. Payout,
the sole Postgres writer, appends the immutable body and returns
`{digest:sha256(exact_request_bytes),observation_id}` in
`spc:source:decision-acks`, keyed by lowercase-hex session:attempt. Front awaits
the matching receipt before advertising the new generation. Neither SPc key
is permitted by the Ark ACL. Front's economic/registration SQL access remains
read-only. Dev/library fixtures without the sink retain source-trace-only status.
The signed SPc source trace emits `derivation_prepared{attempt,record}` before
publication and `derivation_observed{attempt,record,observation_id}` after the
durable receipt. A prepared record alone is not an advertised or activated job.

Local `SPAC/1` carries only signed grant bytes and consumption clock. The Ark
worker publishes it to `spc:ark:authorization:consumption`; payout persists it
and returns a digest-correlated receipt in
`spc:ark:authorization:consumed`. No identity, block hash, member map, scan/spend
key or destination is added to this frame. The same wallet permit remains held.
The existing 10-second grant is rechecked after receipt and immediately before
cosign. Timeout/loss after a grant remains unknown/pinned; no replacement grant.
This is an observation durability fence, not a second monetary authorization.

## Verification scope

Required tests cover complete/absent/failed reads, exact replay/conflict,
transaction rollback, authorization deletion/history retention, front public
input/error classifications, consumption expiry/duplicate/receipt loss and
restricted Ark ACL/privacy. Source-only until the parent assigns a build slot.
