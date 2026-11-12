# Pool operator observation page implementation (EXP-OP)

Date: 2026-11-08
Packet: EXP-OP (`lab-pool-operator`, base `b0ab4ca34fda07c61a6335d05f598eaacc69678c`,
baseline 583-file snapshot sha
`3e2a31be506a36355aee80aef8be60457ca1a1ad68192dc02af80bfbda7c9cb0`)
State: implementation and all verification complete; ready for Fable
re-review before parent integration. No commit made.

## Scope and result

Contract v3 §4 (`2026-11-07-live-lab-integration-contracts.md`), the
EXP-OP-approved portion only: a read-only pool operator observation page on
the pool API process, reachable without the SP auth platform. Built as a new
optional `[operator]` config section that is off by default, loopback-only,
and requires the `api` role. When enabled it serves exactly four GET routes
(`/`, `/operator.css`, `/operator.js`, `/api/pool/observation`) from its own
listener, separate from the public `[api]` router (which is untouched and
404s all four operator paths — asserted by a scope test).

The `spc-pool-observation/v1` DTO carries five primary facts (network/work
profile, latest observed block, total pending liability, Ark payout counts by
status, Ark float state) plus a bounded `recent_blocks` table. Every
store-sourced field is nullable with a closed error model: `{field, code,
source}` triplets, `code` from exactly
`store_unavailable | timeout | schema_mismatch | invalid_value |
snapshot_inconsistent | unknown`. No identity field (`sp_id`, `B_scan`,
`B_spend`, `P_i`), no VTXO id, no Ark address, and no DSN, SQL text, table or
column name ever reaches the DTO or the page. The block rows carry the full
64-hex block hash, which is the existing pool-side `block_found` fact and is
allowed by the contract (no Ark-role input, no identity/order/VTXO join).

Money is exact: `Store::liability_snapshot()` is one SQL statement reading
pending balance, scheduled-Ark component and `now()` together, so
`total_sats == pending_balance_sats + ark_due_sats` with one `as_of` by
construction; the DTO re-verifies the invariant and nulls the whole object
on violation. All sats serialize as decimal strings and the page formats
them with `BigInt`, never `Number`.

This packet moves no money and controls no process. There is no POST route,
no sweep/retry/rotation action, no Ark daemon start, no auth platform. The
float reports `not_configured` until AP-4's real status source exists.

## File delta vs baseline (exactly the packet, nothing else)

Changed (7):

- `Cargo.lock` — dev-dep additions only.
- `bins/spc-pool/Cargo.toml` — `[dev-dependencies]`: sqlx, chrono, uuid
  (test-only; shipped binary unchanged).
- `bins/spc-pool/src/config.rs` — `[operator]` section, loopback and
  api-role validation, three `ConfigError` variants.
- `bins/spc-pool/src/lib.rs` — `pub mod operator;`.
- `bins/spc-pool/src/main.rs` — fail-closed role check, listener spawn as
  a tracked task.
- `config/spc-pool.example.toml` — commented `[operator]` section.
- `crates/spc-store/src/postgres.rs` — `liability_snapshot()`,
  `LiabilitySnapshot`, `LIABILITY_TOTAL_OVERFLOW`,
  `StoreError::is_schema_mismatch()`, `now_rfc3339()`.

New (8):

- `bins/spc-pool/src/operator.rs` — trait, DTO, error codes, backend,
  server, router, embedded assets.
- `bins/spc-pool/static/operator/{index.html,operator.css,operator.js}`.
- `bins/spc-pool/tests/operator_observation.rs` — 16 tests.
- `crates/spc-store/tests/liability_snapshot.rs` — 4 tests.
- `docs/changelog/2026-11-08-pool-operator-page.md` (merge-owner splice
  per `docs/changelog/README.md`).
- `docs/decisions/2026-11-08-pool-operator-page.md`.

Verified against the baseline manifest with a full-tree hash walk: 7
changed, 8 new, 0 missing, 0 unexpected. `api.rs`, `api_backend.rs`,
`metrics.rs`, AP3-agent, T0-lab and QA/gallery ports are untouched.

## Design notes

The full decision record is
`docs/decisions/2026-11-08-pool-operator-page.md`; the points that shaped
the implementation most:

- **`ObservationSource` trait with RPITIT `+ Send` bounds** (no
  `async_trait` object safety) so the failure mapping is provable without a
  database, mirroring the existing `ApiBackend`/`StoreBackend` pattern. The
  trait's block rows carry `settled_at` already RFC3339-formatted and
  spc-store exports `now_rfc3339()`, so spc-pool gains no production
  chrono/sqlx dependency — only test dev-deps.
- **The backend enforces `recent_blocks` bounds itself** (height-DESC sort
  + truncate to 10) regardless of source behavior; payout counts zero-fill
  exactly the three schema statuses.
- **Config strings are inert**: `work_profile` is rebuilt through an
  explicit 11-key allowlist; `endpoints` is omitted entirely; the page
  renders everything with `textContent` only.
- **`<link rel="icon" href="data:,">`** in `index.html` so Chrome's
  favicon request cannot create a fifth route or console error.

### The 90s-to-30s fix

The first working version read the three store facts sequentially. With
the store stopped, each read independently hit the sqlx pool acquire
timeout (~30s), so the response held ~90s and the page sat on stale data
long past its 10s poll interval — confirmed by direct `time curl` (1m30s)
before the fix. The three reads are independent, so they now run under one
`tokio::join!`: a store outage costs one pool timeout (~30s), error order
in the DTO stays fixed, and the sealed browser proof shows the page flip
to `unknown (store_unavailable, postgres)` with closed-code errors within
that bound.

## Test evidence

All runs against a real throwaway PostgreSQL 17.11 (initdb'd fresh for
this packet; private port 55432, private socket, trust auth, owned pids;
deleted after). Rust builds and tests ran on the existing `./target` under
`/tmp/opencode/silent-pool-heavy-build.lock` with an in-lock >=50 GiB free
check (102 GiB at start, ~95 GiB at end). Rust 1.93.0, `-j 2`.

- `cargo test -p spc-pool --test operator_observation`:
  **16 passed; 0 failed** (~0.5-0.8s; two of the sixteen are real-database
  tests that create and drop their own `expop_testfixtures_*` databases
  under `SPC_TEST_DATABASE_URL`).
- `cargo test -p spc-store --test liability_snapshot`:
  **4 passed; 0 failed** (~0.3s, real DB): single-statement snapshot with
  the contract fixture, settle-racing-the-snapshot never tears the pair,
  stopped store is an error not zeros, broken schema is an error not
  zeros.
- Full suites, final source, real DB:
  `cargo test -p spc-pool -p spc-store` — **all green**, including 248
  spc-pool lib tests, every e2e target, `example_config` (the example
  config with the new commented `[operator]` section still validates),
  and 40 spc-store tests (10 lib + 20 postgres + 4 redis + 2 work_units +
  4 liability_snapshot).
- `cargo fmt -p spc-pool -p spc-store`: clean.
- `cargo clippy -p spc-pool -p spc-store --all-targets`: zero findings.

Coverage highlights (the properties the contract actually demands):

- DTO shape is closed: no fifth field on block rows, exactly three payout
  statuses, no identity fields anywhere.
- Liability serializes as exact decimal strings; amounts beyond
  `Number.MAX_SAFE_INTEGER` survive as strings (BigInt boundary probe:
  `9007199254740990 + 4 + 3 = 9007199254740997`).
- Liability invariant failure nulls the whole object with
  `snapshot_inconsistent` only — never a partial number.
- Broken schema with a secret-bearing DSN leaks nothing: no DSN, no
  password, no role, no table/column name in the DTO.
- Float stays `not_configured` until AP-4.
- Config: `[operator]` defaults off; non-loopback listen is a validation
  error; listen without the api role is a validation error;
  `OperatorServer::bind` independently refuses non-loopback addresses.
- Router serves exactly four GET routes; the public `[api]` router 404s
  all four operator paths.

Test-bug notes (for the record, all resolved): early red runs were caused
by serde_json's alphabetical key ordering vs my insertion-order
expectation, a duplicated `[pool]` table in one test's TOML, wrong expected
heights in the fixture (two settles share height 800_000), and a redaction
assert that tripped on the DTO's own contract field name
`ark_payouts_by_status` (which contains the substring `ark_payout` by
spec). None were product bugs. Seven fixture databases leaked by the
panicking early runs were removed by a manual sweep after the fact - not
by the tests' own cleanup; the broken-schema test also leaked one fixture
database per run by discarding the name `isolated_fixture()` created
(the Fable review caught this; fixed in the rework below so every created
name is dropped and verified on every path).

## Browser proof (real Chrome, real binary, sealed)

Candidate: `cargo build` (dev profile) of the final source, sha256
`404423102e09a16aa1d2ab10f2dd8d65a59f3250e7849feedffb6cb4da3e179e`,
recorded immediately before and after the proof — identical. (A `cargo
test` artifact with different metadata may occupy the same path afterward;
both build the identical final source. The proof ran on the recorded
build.)

Runtime: the candidate `spc-pool` (roles `[api]`, mock template, regtest)
serving the operator listener on `127.0.0.1:38081` against a seeded
throwaway Postgres (TESTFIXTURES values only: the BigInt boundary probe
above, 13 blocks, one scheduled and one sent Ark payout). Driven by Chrome
156 headless over CDP from node v24; harness at
`/tmp/opencode/EXP-OP-browser/harness.mjs`, results in
`browser-proof.json`.

**29/29 checks passed:**

- 5 primary facts render (network/work profile, latest block height
  800001 pending, liability, Ark due, float `not_configured`).
- BigInt exactness: `9 007 199 254 740 997 sats` and `3 sats` — the
  boundary value is not rounded.
- Details table: ten rows, keyboard-operable (Enter opens/closes), focus
  and disclosure state preserved across a poll cycle.
- No horizontal overflow at 1440x900 or 390x844; the wide block table's
  overflow is contained in its wrapper, not the page.
- Network: only the four operator routes requested (favicon suppressed by
  the data-URI icon); console clean throughout.
- Hostile-string inertness via a stubbed `fetch` serving a payload full of
  `<script>`, `<img onerror>`, `<iframe src=javascript:>`, `javascript:`
  URIs: no `__pwned`, no injected elements, hostile strings rendered as
  literal text, money still BigInt-exact; the page recovers to the real
  payload after stub removal.
- Stop-store (pg_ctl fast stop): liability and latest block flip to
  `unknown (store_unavailable, postgres)` — not a cached value, not a
  zero — with the closed-code error list (`pending_liability`,
  `ark_payouts_by_status`, `recent_blocks`, all
  `store_unavailable (source: postgres)`), console still clean. The flip
  lands within the ~30s single pool timeout (the `join!` fix above).

Screenshots: `desktop-1440x900.png`, `mobile-390x844.png`,
`stopped-store.png` under `/tmp/opencode/EXP-OP-browser/`.

Operational notes recorded for reruns: the pool must be started with
`setsid` (a `nohup`/`$!` pair captured a wrapper pid and orphaned the real
binary); `pg_ctl start/restart` needs explicit
`-o "-p 55432 -k <sock> -c listen_addresses=127.0.0.1"` (restart drops
options otherwise).

## Freeze

`/tmp/opencode/EXP-OP-freeze/`: the 15-file delta (7 changed + 8 new) with
per-file sha256 and modes in `manifest.json`;
`code_sha256_excluding_report` =
`96ed789504a21c8fd261a6e9ddf3c5afc232996497ad62dbfd9372550501fe70`
(recipe: `SHA256(''.join(path + sha256))` over the 15 freeze rows sorted
by path). The candidate binary is pinned by hash at the worktree
`target/debug/spc-pool`, not copied (never feed target trees to
snapshots). This report is copied into the freeze but excluded from the
code hash by design. This freeze and its browser proofs
(`/tmp/opencode/EXP-OP-browser/`, 29/29, binary `40442310...`) are
preserved unchanged as the first-cycle record; the rework below has its
own freeze and proofs.

## Fable rework (2026-11-08, same day)

The first Fable review
(`docs/reviews/2026-11-08-pool-operator-fable.md`, tied to code snapshot
`24290953...`) returned REQUEST_CHANGES with three bounded items and
explicitly approved the backend/listener/DTO shape, the SQL, the privacy
properties and the merge shape. Disposition of every finding:

- **R1 (required, fixed).** The store-level race test's oracle was
  tautological (it re-asserted the Rust `checked_add` identity, which
  holds for any two numbers, torn or not). Replaced with the conserved-sum
  oracle: 64 one-sat scheduled Ark rows are moved to pending one at a
  time by the real `mark_ark_failed_to_pending` transaction while
  snapshots run concurrently, and every snapshot must read
  `total_sats == 64` and `pending + ark == 64` against the seeded
  constant; the test requires mid-transition observations and a final
  state of pending 64 / ark 0. A new deterministic
  `torn_two_statement_counterfactual_is_detectable` test proves the
  oracle discriminates: reading pending and ark as two separate
  statements around one committed move yields N-1, exactly the tear the
  oracle rejects, while the real single-statement snapshot at the same
  logical point is exact (forced interleave, no sleep, no probability;
  the review's own live measurement - 40 tears in 1236 two-statement
  reads vs 0 for the single statement - is the field evidence). The
  production `liability_snapshot()` SQL is untouched. The legal
  empty/zero tests are unchanged.
- **R2 (required, fixed).** `broken_schema_with_secret_dsn_leaks_nothing`
  no longer calls `isolated_fixture()` (which created a database whose
  name it discarded): it connects its admin pool directly, records
  assertions as failure strings so cleanup runs on every path including
  assertion failure, drops the database and login role it creates, and
  verifies by name that neither remains. Verified after the rework test
  runs: zero `expop%` databases and zero `expop%` roles on the throwaway
  cluster. The earlier leak (one database per run of the old test, plus
  seven from panicking pre-fix runs) was cleaned by a manual sweep, not
  by test cleanup - stated accurately above.
- **R3 (required, fixed).** The poll loop keeps an `inFlight` guard (a
  tick while a response pends is skipped; at most one observation request
  is ever outstanding) and a monotonic request sequence (an obsolete
  completion or failure is dropped; no older payload or error can reset
  newer state). `lastGoodAt` advances only on a payload with no
  closed-code errors - an HTTP 200 whose fields are null is an outage,
  not freshness. The browser proof's R3 regression drives a 12 s delayed
  fetch (longer than the 10 s interval): exactly one request is issued
  and never overlapped, where the old loop fired a second overlapping
  fetch; the delayed payload (the newest request) renders, and the next
  poll restores fresh data.
- **F4 (folded, fixed).** The 390x844 overflow measurement now runs with
  Details open (keyboard), the wrapper-containment check is strict
  (`=== true`, wrapper present, `overflow-x: auto`), and
  `Runtime.exceptionThrown` is subscribed - the detector itself is
  verified each run by injecting a probe exception and confirming
  capture before resetting. `browser-proof.json` records real `started`
  and `finished` timestamps.
- **F5 (folded, fixed).** Bounded staleness: after three poll intervals
  (30 s) without good data on a transport failure, the five main facts
  read `unknown (observation unreachable)` instead of repeating the last
  good payload indefinitely (detail rows keep their observed-at labels).
  The proof kills the pool process: pre-cap the page shows the stale
  banner with the last good values, post-cap `unknown (observation
  unreachable)` - a different closed vocabulary from the DB-outage
  `unknown (store_unavailable, postgres)`, never a zero - and both
  store and listener recoveries are verified.
- **F6 (folded, fixed).** `Cache-Control: no-store` on
  `/api/pool/observation` (asserted in the router test for all four
  routes); a sats string with a leading `-` renders
  `unknown (invalid value)`, never a negative amount (server u64
  unchanged; hostile-stub check). Startup order unchanged per the
  review's verification (clean shutdown on bind collision).

The rework cycle: fmt + clippy clean; targeted suites green on a fresh
throwaway Postgres 17.11 - `liability_snapshot` 5/5 (the race oracle and
counterfactual included), `operator_observation` 16/16, full spc-store
suites; zero fixture databases or roles left behind. New candidate binary
(JavaScript is `include_str!`-embedded) sha256
`1684cc8402a3d496ea30ea0a9b6faccc747de4c43d6dc20e8e90917130448096`,
sealed as an independent copy (not a hardlink) before the proof and
re-verified unchanged after. New browser proof: **46/46 checks** (the 29
first-cycle checks re-run against the new binary, plus the F4 open-details
mobile measurement, the exception-detector probe, the negative-sats
check, the R3 delayed-fetch regression, store recovery after restart, the
transport-failure cap, and listener-return recovery), harness and results
at `/tmp/opencode/EXP-OP-rework-browser/` (screenshots:
`desktop-1440x900.png`, `mobile-390x844-open-details.png`,
`stopped-store.png`, `transport-unreachable.png`). The pool log shows the
DB-outage response failed all three reads at one instant (the single
`join!` bound, ~30 s, unchanged) and the in-flight guard held the page to
one failed response per outage. Rework freeze:
`/tmp/opencode/EXP-OP-rework-freeze/` - the 16 delta files (7 changed +
9 new vs the 583-row baseline, per-file sha256 and git-normalized modes
in its `manifest.json`, recipe stated explicitly there;
`code_sha256_excluding_report` =
`ab38e0b8fbf8ce1028f55520d6faf3122d70253b924831ab3258b02f3397a2bd`).
The full 592-row parent-style manifest, rebuilt against the current
worktree with the same canonical recipe (rows sorted by path, modes
git-normalized, snapshot and code-snapshot hashes inside), is at
`/tmp/opencode/EXP-OP-rework-parent-manifest.json`. The old freeze and
its proofs are untouched. All TESTFIXTURES values on throwaway
databases; no mining, no live payout claims.

## Later gates (not this packet)

- **AP-4**: the float's real status source; until then it is
  `not_configured` with reason `ap-4 status source absent`.
- **EXP-ACCESS**: the SP auth platform; nothing here authenticates
  operators (loopback placement is the current access story).
- **Ark role integration**: the observation source reads the pool's own
  store only; no Ark role or stream is touched.

## Claim and handoff

No money movement, no process control, no public exposure: this packet is
read-only observation on an off-by-default loopback listener. The first
Fable review approved the shape and required the bounded R1-R3 rework
documented above; **the rework returns to the same Fable for re-review
before parent integration** (per that review's section 6, the re-review
needs the new per-file hashes of the four touched files, the fresh binary
hash with before/after records, the R1 test output on a real database, and
the re-run browser proof including the mobile open-Details check - all in
`/tmp/opencode/EXP-OP-rework-freeze/`). The serial-merge hunks
(config.rs/main.rs/lib.rs, spc-store postgres.rs, example config,
Cargo.toml dev-deps, Cargo.lock, decision record, changelog fragment) and
packet-owned files are listed above for the merge session. The public API
router is unchanged.
