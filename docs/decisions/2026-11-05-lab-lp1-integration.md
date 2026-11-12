# 2026-11-05 - LP-1 share-policy integration

The approved LP-1a arithmetic and L3 decisions are the authority for the v2
work-unit and network-block-window rules. This record describes their integration
into the current pool and does not rewrite those inputs or the L1 conformance
history.

## Integration decisions

1. **Explicit version matrix.** v1 remains the existing integer share-difficulty
   policy with saturating arithmetic and `N * d`. v2 is opt-in and restricted to
   `pool.network = regtest`. Its required fields are `work_unit_version = 2`, a
   32-byte `work_reference_target`, a power-of-two `share_work_multiplier`, the
   caps, a decoded `pow_limit_bits`, and optionally the matching explicit
   `window_rule = network_block_work`. The legacy `share_difficulty` key is an
   error with v2, as are SV1 ingress and type-32 advertising, which remain
   unsupported capabilities rather than being silently approximated.
2. **Generation is the accounting authority.** Each issued job carries a
   monotonically increasing generation, `nbits`, exact share target, `m_issue`,
   version and reference fingerprint. Retarget or multiplier changes allocate a
   new generation; old generations are never repriced. SV1 `clean_jobs = true`
   remains the latest-only policy. SV2 preserves the L1 `(prev_hash, nbits)`
   activation rule and applies `SetTarget` only to the appropriate future or
   subsequent jobs.
3. **Checked v2 accounting, frozen v1.** V2 slice work, fee work, restoration,
   window aggregation, per-identity numerators/denominators, subsidy/fee
   products and final conversions use named checked errors and exact wide
   multiply/divide. v1 keeps its saturating path and persisted meaning. The
   finding slice stays excluded, whole crossing slices stay included, and
   underfill/overshoot are derived rather than columns.
4. **Frozen persistence and restore.** New sealed slices and distribution
   snapshots persist version/fingerprint. Snapshots also persist `window_rule`
   and the exact target. Legacy rows default to `(1, NULL, share_work_legacy)`
   without conversion. A mismatched restore or mixed live window fails closed,
   never zero-fills or converts. Settlement uses the snapshot's frozen rule,
   target and job generation, not later chain state.
5. **Wire codes.** `spc:slices` appends `U8 version = 1|2` and
   `B0_32 fingerprint`; `spc:blocks` appends those plus `U8 window_rule = 0|1`.
   v1 writes the 32-byte zero sentinel (NULL in SQL); v2 requires nonzero.
   Old-format events, unknown enums and invalid combinations fail decode.
6. **Schema.** Migration `0005_lab_work_units.sql` was allocated only after
   checking the separate `sv2-tp` and `ladder` branches. Version enums are
   `(1,2)`, window rules `(0,1)`, v1 fingerprint NULL, and distribution
   version/rule combinations are checked in both Rust and Postgres.
7. **Evidence and replies.** `job_issued` records front/channel/job/generation,
   nbits, target, credit, version/fingerprint, issuance time and activation.
   `accounted` adds the exact booked `d` and share identity on the SPc side only.
   No such event is added to Ark orders/receipts or any Ark stream. Protocol
   success now resolves from that booked source, not a second difficulty
   calculation. SV1 and SV2 channel ids are disjoint. Timeout/lost outcome
   closes the session and emits `share_outcome_unknown` rather than claiming a
   definitive invalid/rejected result while credit could still land.
8. **Evidence boundary.** The main-process SV2 peer proved genuine m=2
   non-block and block-qualified shares with d=2 and Q=65. Other real-node/TDP,
   multi-transaction and CPU-driver scenarios remain separate packets, not
   claims made by this record.

9. **Fail-closed refresh and extension refusal (Fable rework).** Every failed
   `Front::refresh`, including a failure after roll/key/height/open transitions,
   invalidates the job, policy and template. Main clears the current SV1/SV2
   generation too, so old work cannot be accepted under a new tip. Type-32 is
   gated at the SV2 router by `extensions.share_accounting`: disabled requests
   receive `Type32Message::Error { code: "unsupported-extension" }`, and only
   enabled v1 slices may populate `Type32Store`. Regtest v2 also requires the
   actual consensus `pow_limit_bits = 0x207fffff`; a stronger or weaker override
   is not an alternate effective authority.
10. **Bounded v2 history retention amendment (R2 re-review).** Stored row
   versions, not the payout process's configured work-unit version, select
   durable deletion horizons. V1 tail and durable scrub policy stay unchanged
   in every payout configuration; SQL deletes v1 `slice_credit` only below the
   legacy cutoff computed from the configured `retain_windows`, confirmation
   depth and N. SQL deletes v2 `slice_credit` only below the separate **4096
   network-height** cutoff for that stored v2 row. The cutoff is strictly
   `height < tip - horizon`, so a row at `tip - horizon` is retained. Unknown
   versions remain undeleted (fail closed). Both cutoffs come from one tip, so
   mixed snapshots behave identically under v1-configured and v2-configured
   payouts. **Privacy/retention disclosure:** the 4096-height horizon
   deliberately extends SPc-side identity-bearing attribution retention for v2
   rows beyond the default SCRUB horizon; it never extends v1 attribution
   retention and sends no new identity field across the Ark boundary.

   The live v2 RAM tail retains **every eligible stored-v2 slice** up to the
   explicit **4096-slice hard cap**, independently of current or previous Q.
   It applies the same `height >= tip - 4096` eligibility cutoff as durable
   credits before current-Q window selection. Bootstrap fetches no more than
   that cap and applies this cap/horizon on first refresh; subsequent rolls use
   the new tip before selection. Unbounded retention and live use of credits
   already expired under the SQL horizon are forbidden. This bounded headroom
   preserves history for a future harder/larger-Q window instead of pruning it
   merely because an older/easier window needed less. Current-Q selection still
   uses whole crossing slices, and exhausting either genuine cap/horizon below
   Q leaves `window_underfilled` true (an ambiguous payout walk fails closed).
   LP3 warmup/oracle logic reports this as warmup or an inconclusive
   bounded-history run, never as an N-block window. The approved R/work-unit
   arithmetic and all v1 policy remain unchanged.

## Merge notes

Apply the L1/LP1a/L3 changelog fragments first, then this packet's fragment.
The LP-1 sections in SPEC 3.4/4.1-4.4/5.1/5.3/7.4, WIRE-FORMATS 3.1/3.6 and
`docs/share-accounting-ext-deltas.md` must reflect this complete path, not just
the arithmetic precursor. In particular, SPEC 5.3 needs the bounded v2
work-aware retention amendment; CONFIG/OPERATIONS must preserve v1 retention
while documenting the row-version SQL gates, the v2 4096-slice/4096-height
bounds, the deliberate v2 attribution-retention extension and truthful
underfill. `docs/OPERATIONS.md` must carry the new profile and
supported-network restrictions. The separately owned TDP and ladder sources
are integration inputs, not rewritten here.
