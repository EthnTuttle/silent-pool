# 2026-11-08 - Read-only loopback pool operator page (EXP-OP)

## Trigger

The live-lab integration contract
(`2026-11-07-live-lab-integration-contracts.md` v3, section 4, plan pair
approved for EXP-OP only) calls for a pool-operator observation page: five
primary facts about liability, blocks and payout counts, on the pool API
process, readable without the SP auth platform. The pool had no such
surface, and the public `[api]` router is the wrong place for one: it is
Internet-facing (via the Tor egress design), it carries per-identity
endpoints behind BIP 340 challenges, and an operator surface there would
either need the auth platform (EXP-ACCESS, not approved) or would publish
pool internals publicly.

## Decision

1. **A separate, optional, off-by-default listener inside the pool API
   process.** A new `[operator]` config section with a single optional
   `listen` key. Absent section or absent key: no listener, no behavior
   change. When set, the address must parse as a socket address and must
   be loopback - a non-loopback value is a configuration validation error
   (`ConfigError::OperatorNotLoopback`), not a silent public bind. The
   listener requires the api role in `[pool] roles` because it runs inside
   the same process (`ConfigError::OperatorRequiresApiRole`, plus a
   fail-closed bail in `main`); the public `[api]` router is byte-for-byte
   unchanged and `OperatorServer::bind` independently refuses non-loopback
   addresses, so neither a config mistake nor a programmatic caller can
   mount the operator page publicly.
2. **Exactly four GET routes, nothing else.** `/` (the page),
   `/operator.css`, `/operator.js` and `/api/pool/observation`, served from
   `include_str!`-embedded static assets (no filesystem read, no package or
   flake change). Unknown paths are 404; non-GET on a known path is 405
   (axum method routing); a request carrying a body (Content-Length != 0 or
   any Transfer-Encoding) is rejected 400 before routing. There is no POST
   route, no control, no auth platform and no Ark daemon startup: this
   packet claims no money movement and no process control.
3. **One `Store::liability_snapshot()` call for the money facts.** A single
   SQL statement reads the pending balance, the scheduled-Ark component
   and `now()` together, so `total_sats == pending_balance_sats +
   ark_due_sats` with one `as_of` by construction and a settle racing the
   read can never tear the pair. A sum that cannot be represented as u64
   fails closed with `StoreError::Invalid(LIABILITY_TOTAL_OVERFLOW)`
   (exported constant) instead of saturating. The DTO re-verifies the
   invariant at its boundary; a violation nulls the whole object with
   `snapshot_inconsistent`, never a partial number. All sats serialize as
   exact decimal strings; the page formats them with `BigInt`, never
   `Number`.
4. **A closed failure model.** Every store-sourced DTO field is nullable.
   A failed or inconsistent read yields `null` plus one `{field, code,
   source}` entry with `field` from a fixed set, `source` fixed per
   source, and `code` from exactly `store_unavailable | timeout |
   schema_mismatch | invalid_value | snapshot_inconsistent | unknown`.
   The underlying error is logged server-side only; the DTO never carries
   a `StoreError` display string, DSN, private path, SQL text, table or
   column name. `StoreError::is_schema_mismatch()` (added next to the
   error type in `spc-store`) owns the sqlx error-tree classification so
   no consumer pattern-matches sqlx internals.
5. **`recent_blocks` from the existing `Store::recent_blocks` query.** One
   call with a fixed bound of ten rows, its own `observed_at` (explicitly
   distinct from the money snapshot's `as_of`), exactly the four existing
   row fields. `rows: []` is a legal empty table; a failed query nulls
   the whole object. The backend additionally sorts height-descending and
   truncates to the bound itself, so a misbehaving source cannot push an
   eleventh row or reorder the DTO. Payout counts use exactly the three
   schema statuses (`scheduled`, `sent`, `failed_to_pending`),
   zero-filled like the metrics exporter; no `reorged` status is invented.
   The float reports `not_configured` until AP-4's real status source
   exists; `/healthz` is never presented as pool health.
6. **A trait boundary so the failure mapping is provable without a
   database.** `operator::ObservationSource` (RPITIT with `+ Send` bounds,
   no `async_trait` object safety) is implemented by `Store`; the same
   pattern as `ApiBackend`/`StoreBackend`. The trait's block rows are
   `SourceBlockRow` with `settled_at` already RFC3339-formatted, and
   spc-store exports `now_rfc3339()`, so `spc-pool` keeps no direct
   chrono or sqlx production dependency.
7. **Config strings are inert.** The DTO's `work_profile` is rebuilt
   through an explicit 11-key allowlist (`WORK_PROFILE_FIELDS`);
   `endpoints` (the `[reach] advertise` republish) is omitted entirely.
   The page renders every config-derived value with `textContent` only -
   no `innerHTML`, no `eval`, no `window.open`, no `href`/`src` from
   config. No `sp_id`, VTXO id, order id or Ark address appears anywhere
   in the DTO or the page (privacy constraints, SPEC 11.3).
8. **Task lifecycle.** The operator listener is spawned as a tracked task
   and pushed into `main`'s role-task list, so an api+operator process
   stays alive and the listener stops with the process - no detached
   service, no orphan. Note the preexisting shape this builds on: with
   `--roles api` alone the process has historically exited immediately
   (the api listener is spawned detached and not awaited); an operator
   listener configured in that process now keeps it alive, which is the
   intended lab shape, not a regression.
9. **Test-support dev-dependencies.** `bins/spc-pool` gains `sqlx`,
   `chrono` and `uuid` as dev-dependencies so `operator_observation.rs`
   can follow the existing spc-store isolated-database test convention
   (`SPC_TEST_DATABASE_URL`, per-test `CREATE DATABASE`, `DROP DATABASE
   ... WITH (FORCE)`, TESTFIXTURES-labeled values, a LOGIN role with a
   password so the DSN genuinely carries a secret for the redaction
   proof). Dev-deps only: the shipped binary and every other crate are
   unaffected.

## Fable rework (2026-11-08, same day)

The first Fable review (`docs/reviews/2026-11-08-pool-operator-fable.md`,
REQUEST_CHANGES) approved the backend/listener/DTO shape and required three
bounded items; these are folded in without redesign:

10. **Poll discipline (R3).** The page keeps an `inFlight` guard (a poll
    tick while a response is pending is skipped, so at most one
    observation request is ever outstanding - during a 30 s store outage
    the old page stacked up to three) and a monotonic request sequence:
    a completion or failure older than the last applied sequence is
    dropped, so no older payload or error can reset newer state.
    `lastGoodAt` advances only on a payload with no closed-code errors;
    an HTTP 200 whose fields are null is an outage, not freshness.
11. **Bounded staleness (F5).** After three poll intervals (30 s)
    without good data on a transport failure, the five main facts read
    `unknown (observation unreachable)` instead of repeating the last
    good payload indefinitely; detail rows keep their observed-at
    labels. This matches the contract's stale-source vocabulary
    (`unknown` with reason) and differs from the store-outage case,
    where the server answers and the facts carry the server's own
    closed-code unknowns.
12. **Response and value hygiene (F6).** `/api/pool/observation` sets
    `Cache-Control: no-store` like the three static routes; a sats
    string with a leading `-` is an invalid value on the page (rendered
    `unknown (invalid value)`), never a negative amount - the server
    emits u64 only. Startup order is unchanged (the review verified
    clean shutdown on bind collision).
13. **Test oracles (R1, R2).** The store-level race test now uses the
    conserved-sum oracle: 64 one-sat scheduled Ark rows are moved to
    pending by the real `mark_ark_failed_to_pending` transaction while
    snapshots run, and every snapshot must read `total == 64` against
    the seeded constant (the previous form asserted the Rust
    `checked_add` identity, which cannot fail on a torn read); a
    deterministic forced-interleave counterfactual proves the oracle
    discriminates. The broken-schema redaction test connects its admin
    pool directly (no fixture database created on the way in), runs
    cleanup on every path including assertion failure, and verifies by
    name that the database and login role it creates are dropped.

## Compatibility and boundaries

- `[operator]` absent: zero behavior change anywhere (config, main, public
  API all byte-for-byte identical paths).
- The public `[api]` router, `api_backend.rs` and `metrics.rs` are
  untouched; the scope test asserts the public router 404s all four
  operator paths.
- The Ark float status, any operator action (sweep, retry, key rotation)
  and the SP auth platform are later gates (AP-4, EXP-ACCESS); this packet
  is read-only observation and claims nothing else.
- `config.rs`, `main.rs`, `lib.rs`, `spc-store`'s `postgres.rs`, the
  example config and `bins/spc-pool/Cargo.toml` `[dev-dependencies]` are
  serial-merge hunks; `operator.rs`, `static/operator/*` and the two new
  test files are packet-owned. `api.rs`, `api_backend.rs` and `metrics.rs`
  are not edited.

## Edits applied

- `bins/spc-pool/src/operator.rs` (new): trait, DTO, error codes, backend,
  server, router, embedded assets.
- `bins/spc-pool/static/operator/index.html`, `operator.css`,
  `operator.js` (new).
- `bins/spc-pool/tests/operator_observation.rs` (new): config/scope/
  router/DTO coverage via a scriptable source; two real-database tests
  under `SPC_TEST_DATABASE_URL`.
- `crates/spc-store/src/postgres.rs` (hunk): `liability_snapshot()`,
  `LiabilitySnapshot`, `LIABILITY_TOTAL_OVERFLOW`,
  `StoreError::is_schema_mismatch()`, `now_rfc3339()`.
- `crates/spc-store/tests/liability_snapshot.rs` (new): single-statement
  snapshot, torn-pair race, stopped store, broken schema.
- `bins/spc-pool/src/config.rs` (hunk): `[operator]` section, validation,
  three `ConfigError` variants.
- `bins/spc-pool/src/main.rs` (hunk): fail-closed role check, listener
  spawn, tracked task.
- `bins/spc-pool/src/lib.rs` (hunk): `pub mod operator;`.
- `bins/spc-pool/Cargo.toml` (hunk): dev-dependencies only.
- `config/spc-pool.example.toml` (hunk): commented `[operator]` section.
- `docs/changelog/2026-11-08-pool-operator-page.md` (new; merge-owner
  splice per `docs/changelog/README.md`).
