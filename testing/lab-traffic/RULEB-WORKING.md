# Working RULEB API r2: offline consumer, not producer approval

For the later source-r3 migration-0011 seed and live collector codec, see
[SOURCE-FEED-IMPLEMENTATION.md](SOURCE-FEED-IMPLEMENTATION.md). The r2 history
gap recorded below now has an explicit source contract and consumer parser;
an actual PG execution receipt and tested producer are still required.

The parent authorized offline parsing of
`/tmp/opencode/ladder-completion-owner-api-r2.json`, SHA256
`bad6ff5e1a8602e8b0e25fabac23020af4862691db26e925363b8102fbf0f78c`.
Its declared status is `working-source-unverified-disk-blocked`; tests passed
is false and its tested freeze is null. This pin is not a live acceptance
binding. The final tested producer/source/binary tuple is still required.

The mutable handoff path subsequently gained two AP3 order-endpoint fields.
The hash guard correctly refused those changed bytes. The original bytes,
verified against the pin above, are preserved at
`/tmp/opencode/T3-RULEB-working-parser-r5/owner-api-r2-original.json`; the later
observed bytes and exact working Rust field-type/SQL-column inspection are
retained beside it. No working snapshot is represented as tested source.

## Implemented concrete adapters

- `ruleb_source.py` validates actual `FrozenJob`, `Balance`, `Consumption`,
  `Reservation` and bootstrap `View` fields from working `economic.rs`.
  Missing/unknown fields, unsupported versions, malformed byte arrays, overflow,
  duplicate identities, inconsistent B/R/A, mismatched parent/generation and
  out-of-order slices reject. JSON u64 values remain integers above 2^53.
- `ladder_wire.decode_spcb_working(..., api_sha256=...)` parses SPCB/3's bounded
  U32-length FrozenJob JSON. Logical Entry format_version stays 2. The decoder
  requires canonical struct-field JSON and map encoding before hashing the
  complete frame with `SPc/DistributionSnapshotV2`. Historical SPCB/2 returns
  economic=None; it is never interpreted as P=0.
- `PoolSource(..., working_ruleb_api_sha256=...)` explicitly opts into the
  pinned working format for offline records. It joins FrozenJob.job_id to
  `(height << 32) | wire_job_sequence`, generation, parent and the exact frozen
  slice list. SPCB/3 economic bytes must match that frozen source record.
  `provisional_candidate` remains tentative evidence, not a reservation grant.
- `ruleb_sql.query(api_sha256=...)` renders a closed read-only repeatable-read
  query with the exact additional tables/columns in API lines 40-49, including
  server_pubkey, economic tables and receipt disposition/cursor. It is not
  selected automatically by the live SQL collector.
- `ruleb_sql.replay_checkpoint` replays actual before/after row images and
  transaction membership, checks immutable per-block economics across lifecycle
  transitions, and requires every booked-pending change to match the same
  transaction's ledger changes. It binds the checkpoint to raw parent ancestry,
  exact revision, full economic-chain rows and coinbase digests. A later
  current-DB revision cannot substitute for the job's snapshot.
- Available A is recomputed as booked B minus all canonical unbooked consumed
  amounts. Every active unbooked/loss_held record must be retained (4096 cap),
  but only unbooked contributes to R. Bootstrap claims are compared with the
  complete replayed active set, parent and revision. Frozen P is compared with
  this independently computed B/R/A, not trusted because the pool emitted A.
- Discriminator 0x05 decodes only as EconomicChanged: U8, U64 rev, U64 economic
  revision, B032 anchor. Its result explicitly has reload-hint-only authority.

## Evidence still required

The offline replay result does not issue a TrustedSource/history checkpoint.
It separately reports missing tested-producer binding and independently
verified raw settlement/frozen-job ancestry. Ledger consistency alone does not
authenticate an input history or prove the actual mined E. No synthetic
row fixture establishes native pending reservation behavior.

One concrete bootstrap completeness issue must be covered by the final
evidence: working migration 0009 inserts the default `economic_state` singleton
**before** installing its history trigger. A first UPDATE before-image therefore
has no preceding INSERT in `source_accounting_history`. The current independent
replayer refuses that missing initial ancestry. Supply independently verified
initial-checkpoint evidence (or the final producer's explicit history-start
coverage); do not silently seed this row or a pending balance from defaults.

Working recanonicalization/loss-held behavior is not producer-approved. Unknown
transitions remain INCONCLUSIVE and no timeout/marker/hint releases funds.
The runtime pre-mining transport, before-publish acknowledgement and actual
finding-slice boundary are still assigned to the producer owner.

The A1 late-AcceptedByRpc correction was frozen separately before these edits:
`/tmp/opencode/T3-A1-late-acceptance-fix-r4/handoff.json` (177 tests).
That isolated five-file packet is the same-auditor recheck target.
