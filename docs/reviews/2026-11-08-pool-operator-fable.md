# EXP-OP pool operator observation page: Fable review

Date: 2026-11-08
Reviewer: Claude Fable 5.1 (`amazon-bedrock/global.anthropic.claude-fable-5-1`),
subagent session.
Packet: EXP-OP (contract v3 section 4 only), uncommitted in
`/home/ethan/.local/share/opencode/worktree/ad8886/lab-pool-operator`,
base `b0ab4ca34fda07c61a6335d05f598eaacc69678c`.
Contract: `/tmp/opencode/role-ui-plan-review-v3/` (components plan
`a07b8ab30e2875c457ed540cd2894588d90f87003ba1b5c9e80af463a773dc2f`,
integration contracts
`ace881da5100d9d4e9b4056c067d53aaff9d322544b3bcc8a9d8caa079756b2c`; both
re-hashed here).
Authority manifest: `/tmp/opencode/EXP-OP-parent-review-manifest.json`, 592
rows, canonical snapshot
`ed00ba4826f7ce096fa409f8af1ebdb121ae1bd9c0027eb4b8b9a00005023ad9`, code
snapshot (report excluded)
`2429095336d59c6e2fa14b27119cf452eb04e8329d6ddcae796f4146a43545d1`.
Baseline: `/tmp/opencode/EXP-OP-baseline-manifest.json`, 583 rows, snapshot
`3e2a31be506a36355aee80aef8be60457ca1a1ad68192dc02af80bfbda7c9cb0`, frozen
source `/tmp/opencode/EXP-OP-baseline-src`.
Candidate report: `docs/reviews/2026-11-08-pool-operator-implementation.md`
(in the worktree). Author freeze: `/tmp/opencode/EXP-OP-freeze/manifest.json`.

## Result

**REQUEST_CHANGES**, bounded, tied to code snapshot `24290953`.

The implementation is sound and does exactly what the contract asks and
nothing more: an optional, off-by-default, loopback-only listener inside the
API process; four GET routes; a closed-code nullable DTO; one SQL statement
for the money snapshot; exact decimal strings and BigInt on the page; inert
config strings; no POST, no controls, no Ark, no auth platform. I reproduced
all 20 targeted tests on my own throwaway Postgres, exercised the frozen
binary as a real process (fail-closed config, role override, bind collision,
public-router scope, method/body rejection, 30 s outage bound, recovery,
SIGTERM), confirmed `--features rpc` (the Nix production feature set)
compiles, and re-linked the frozen source back to the exact recorded binary
hash. Every finding below is a test-strength, hygiene or page-robustness
item; none is a money, privacy or exposure defect.

Three bounded items block approval as written (section 3, R1-R3). R1 is the
reason for the verdict: the store-level "settle racing the snapshot" test the
contract requires cannot fail on a torn read, and I demonstrated on the real
schema shape that a discriminating oracle exists and is cheap. R2 and R3 are
small and belong in the same rework cycle because R3 changes `operator.js`
(embedded in the binary), which re-opens the browser proof anyway.

Everything else - the 15-file code delta, the decision record, the changelog
fragment, the example config, the serial-merge hunks - is approved as-is and
needs no re-review beyond the per-file hashes of the three touched files.

## 1. Provenance and scope verification

- Parent manifest: every one of the 592 rows re-hashed against the worktree
  (0 mismatches, 0 missing, 0 extra against `git ls-files --cached --others
  --exclude-standard`). Canonical recipe `SHA256(compact sorted JSON
  {base,files})` reproduces `ed00ba48...` and, on the baseline manifest,
  `3e2a31be...`. Delta vs baseline: 7 modified + 9 new (8 implementation +
  the report), 0 removed; identical to the manifest's `delta` and to the
  author's 15-file freeze (freeze hashes equal parent hashes; freeze records
  raw modes `0o644/0o664`, parent records normalized git modes `100644`).
- Author `code_sha256_excluding_report` `96ed7895...` reproduces with the
  clarified recipe `SHA256(''.join(path + sha256))` over the 15 freeze rows
  sorted by path (1463 chars). The parent full manifest remains the review
  authority; no corruption or drift.
- Binary: `candidate-sha-before.txt` and `candidate-sha-after.txt` both record
  `404423102e09a16aa1d2ab10f2dd8d65a59f3250e7849feedffb6cb4da3e179e` for
  `target/debug/spc-pool`. I copied that file (`cp`, not a hardlink; the
  worktree file has link count 2 into `deps/`) to
  `/tmp/opencode/EXP-OP-fable/spc-pool.candidate` and verified the hash
  before any build. All real-process checks below ran on that copy.
  As the report warned, my `cargo test` runs replaced the worktree path with
  a test-profile artifact (`7cd09aaf...`). A final `cargo build -j 2 -p
  spc-pool` on the unchanged source re-linked the worktree path back to
  exactly `40442310...` (5.3 s, no source change; post-run tree re-hashed, 0
  drift). That closes the "which build is this" question: the recorded hash
  is the deterministic dev-profile build of the frozen source.
- Untouched by the delta, confirmed by hash: `bins/spc-pool/src/api.rs`,
  `api_backend.rs`, `metrics.rs`, `docs/SPEC.md`, `flake.nix`, every
  AP3/AP1a/T0/T4 file. The flake's source filter excludes only `target`,
  `.git` and `result*`, so `bins/spc-pool/static/` is in the Nix source and
  the `include_str!` embedding needs no flake change (contract 4.4 listed a
  flake asset hunk; it is correctly unnecessary).
- Disk/lock policy: every cargo invocation ran under nonblocking `flock` on
  `/tmp/opencode/silent-pool-heavy-build.lock`, 95 GiB free checked after
  acquisition and after each step (no measurable growth), reusing the
  candidate's existing `./target` (9.0 GiB); no reviewer target, no
  workspace-wide or Nix build. Lock was free throughout.

## 2. What I verified

### 2.1 Targeted tests (real Postgres 17.11, my own fixture)

Throwaway cluster: `initdb -U spc --auth=trust`, `127.0.0.1:55433`, private
socket dir, `SPC_TEST_DATABASE_URL=postgres://spc@127.0.0.1:55433/expop_fable`;
stopped and its data dir removed afterwards; logs kept in
`/tmp/opencode/EXP-OP-fable/`.

- `cargo test -j 2 -p spc-store --test liability_snapshot`: 4 passed (0.36 s).
- `cargo test -j 2 -p spc-pool --test operator_observation`: 16 passed
  (0.37 s), including the two real-database tests, which executed (not
  skipped) against the fixture.
- `cargo check -j 2 -p spc-pool --features rpc --lib --bins`: clean (26 s).
  `operator.rs` uses only existing production deps (`axum`, `serde`,
  `serde_json`, `hex`, `tokio`, `thiserror`, `tracing`, `spc_store`); the
  `chrono` methods it calls (`to_rfc3339`) are reached through types
  re-exported by `spc-store`, and `now_rfc3339()` is exported there, so no
  production `chrono`/`sqlx` dependency is added to `spc-pool`. The three new
  `bins/spc-pool` entries are `[dev-dependencies]` only; `Cargo.lock` gains
  exactly `chrono`, `sqlx`, `uuid` edges on `spc-pool`, no version changes.
- The full `-p spc-pool -p spc-store` suites were not rerun (report cites
  all green; the delta touches no other test and no shared production path
  beyond the additive `postgres.rs` hunk).

### 2.2 Real-process checks on the frozen binary

Config: regtest, mock template, `roles = ["api"]`, `[api] listen =
"0.0.0.0:38090"` (deliberately public), `[operator] listen =
"127.0.0.1:38091"`.

- `ss -ltnp`: `0.0.0.0:38090` and `127.0.0.1:38091`, same pid. The operator
  socket is loopback even when the public API is wildcard-bound.
- Operator listener: `/`, `/operator.css`, `/operator.js`,
  `/api/pool/observation` -> 200 with the right content types; `/favicon.ico`,
  `/api/pool`, `/metrics`, `/healthz`, `/index.html`,
  `/api/pool/observation/` -> 404; POST/PUT/OPTIONS on known paths -> 405;
  GET with `Content-Length: 1` or `Transfer-Encoding: chunked` -> 400 before
  routing; `?x=1` query ignored (200). HEAD on the four paths -> 200 with no
  body (axum's implicit HEAD for `get()`); that is the standard same-origin
  semantics and the only method beyond GET that succeeds. Proportionate; the
  contract's "exactly four routes, all GET" holds in the sense that matters
  (no extra path, no state-changing method).
- Public listener (`:38090`): the four operator paths -> 404; `/api/pool` and
  `/healthz` -> 200 (unchanged).
- DTO on an empty migrated database: `rows: []`, `"0"` strings, zero-filled
  counts, `errors: []`, `float.not_configured`; `work_profile` carries exactly
  the 11 allowlisted keys (which today is the full `profile_json()` key set;
  all values scalar or null) and no `endpoints`.
- Store stopped (`pg_ctl -m fast stop`), three concurrent requests: each
  returned 200 in 30.02 s (not 90 s; concurrent, not serialized), with
  `pending_liability`, `ark_payouts_by_status`, `recent_blocks` all `null`
  and exactly three `{field, store_unavailable, postgres}` entries. No zero,
  no empty list. Store restarted: next request 26 ms, data back, no errors.
- SIGTERM with a 30 s request in flight: process exited in 7 ms, both ports
  released, client saw a reset. No graceful drain exists anywhere in
  `main.rs` (baseline behaviour); the operator listener does not block
  shutdown.
- Fail-closed config, all exit 1 before any bind: `listen = "0.0.0.0:38091"`
  -> `OperatorNotLoopback`; `listen = "localhost:38091"` -> `OperatorListen`;
  unknown key under `[operator]` -> deny_unknown_fields; `--roles payout`
  CLI override with `[operator]` set -> the `main.rs` bail (config validation
  alone would have passed because the TOML roles include api).
- Port collision: with `127.0.0.1:38091` occupied, startup logged "api
  listener ready" then exited 1 with `operator listener bind failed: Address
  already in use` in 37 ms; `:38090` released, nothing left half-running.
- Lifecycle note confirmed against baseline source: with `roles = ["api"]`
  alone the baseline process exits immediately ("no roles selected; exiting")
  because the api task is spawned detached and never pushed to `tasks`;
  with `[operator]` set the pushed operator task keeps it alive. Decision
  record item 8 states this honestly. Not a regression; the pre-existing
  asymmetry is out of EXP-OP's scope.

### 2.3 Code audit (DTO, money, privacy)

- `Store::liability_snapshot()`: one `SELECT` with two scalar subqueries and
  `now()`; a single statement runs under one MVCC snapshot in every isolation
  level, so pending and ark are read atomically with respect to any committed
  settle. `::bigint` casts, `sats_u64` rejects negatives (`Invalid`),
  `checked_add` fails closed with the exported `LIABILITY_TOTAL_OVERFLOW`
  (no saturation, unlike the pre-existing `liability_sats()`). `as_of` is the
  statement's `now()`.
- Backend re-verifies `pending + ark == total` with `checked_add`; violation
  nulls the whole object with `snapshot_inconsistent`. Three reads run under
  one `tokio::join!`; error order is fixed. Payout counts zero-fill exactly
  `scheduled|sent|failed_to_pending` on success only; failure nulls the map.
  `recent_blocks` re-sorts height DESC and truncates to 10 regardless of the
  source; `observed_at` is its own timestamp; `[]` is success, `null` plus
  code is failure. Block hash is full hex (64 chars), which is the existing
  pool-side `block_found` fact; the report's "truncated hex label the schema
  itself stores" wording is inaccurate (the schema stores the full 32-byte
  hash), but the behaviour is contract-conformant (block hash allowed, no
  Ark-role input, no identity/order/VTXO join). Report wording nit only.
- Error mapping: `Timeout` only for `StoreError::Timeout` (connect timeout);
  a sqlx pool-acquire timeout is `Db(PoolTimedOut)` -> `store_unavailable`.
  A `bigint out of range` from the SQL cast (SQLSTATE 22003) would also map to
  `store_unavailable` rather than `invalid_value`. Both are closed codes and
  both null the field; the labels are defensible. Non-blocking.
- No `StoreError` display, DSN, SQL, table/column name or row value reaches
  the DTO: `errors` entries are three `&'static str`/enum fields by type, and
  `tracing::warn!` is the only sink for `%e`. Verified by the secret-DSN
  broken-schema test (real DB) and by reading every `Serialize` struct.
- No `sp_id`, `B_scan`, `B_spend`, `P_i`, VTXO id, order id or Ark address
  appears in any DTO type; no Ark-role stream or module is touched.
- Malicious-source robustness: the `ObservationSource` trait returns plain
  data; a hostile implementation can only return rows or errors, never
  panic the backend (no unwraps/indexing on source data).

### 2.4 Page audit and browser proof review

- `operator.js`: only `textContent`/`createElement`; no `innerHTML`, `eval`,
  `href`/`src` from data, no `window.open`; one relative `fetch('api/pool/
  observation', {cache:'no-store'})`; `formatSats` validates with `BigInt`
  and groups the original digit string (no `Number`/`parseInt`); error
  reasons rendered as `code, source`; `null` fields render `unknown (...)`,
  `rows: []` renders "none observed yet" with the explicit "successful read
  of zero rows" note; float renders the server's `state (reason)`;
  `/healthz` is never fetched. "Of which" wording present; the page never
  adds the two components.
- `index.html`: five facts, keyboard-native `<details>`, the ten-row bound
  stated, `data:,` icon suppressing favicon. `operator.css`: table overflow
  contained in `.table-wrap` (`overflow-x: auto`), no page-level
  `overflow: hidden`.
- Browser proof (`/tmp/opencode/EXP-OP-browser/`): harness drives the frozen
  binary over CDP with real `Input.dispatchKeyEvent` Enter presses, real
  `Network.requestWillBeSent` accounting (5 requests, all four paths, no
  stray), `Runtime.consoleAPICalled`(error) + `Log.entryAdded`(error)
  capture, a real `pg_ctl stop` for the outage case, and screenshots. The
  hostile-string case is a `fetch` stub (XSS negative only, as stated); the
  other 25 checks hit the real server and a real seeded fixture. The three
  PNGs match the JSON (`stopped-store.png` shows the three facts as
  `unknown (store_unavailable, postgres)`, float `not_configured`, no stale
  banner because the server answered; no zero anywhere). Cleanup: pool pids
  in `pool.pid` are dead, Chrome profile process gone, PG data dir removed.
  These are TESTFIXTURES values (BigInt boundary probe, 13 synthetic blocks);
  not mined liability, not an E2E, exactly as the report says.
- Proof gaps (see F4): the 390x844 overflow measurement ran with Details
  closed; `Runtime.exceptionThrown` is not subscribed; the "wrapper
  contained" check also passes on `null`.

## 3. Findings

Ranked. R = required for approval; F = non-blocking, merge owner's
discretion.

### R1. The store-level race test cannot detect a torn snapshot (contract 4.4 "settle racing the snapshot")

`crates/spc-store/tests/liability_snapshot.rs::settle_racing_the_snapshot_never_tears_the_pair`
races `mark_ark_sent` (scheduled -> sent, pending untouched, stays 0) against
`liability_snapshot()` and asserts `total_sats == pending_balance_sats +
ark_due_sats`. That equality is computed by Rust in `liability_snapshot()`
itself (`checked_add`), so it holds for any two numbers the SQL returns,
torn or not; and with pending fixed at 0 there is no cross-table quantity
whose consistency a tear could violate. The test proves "ark is
non-increasing while sends run", which a three-statement implementation
would pass equally. The contract's store-level item (a racing settle "cannot
yield a torn or partial object") is therefore asserted, not tested.

Counterexample oracle, demonstrated on my fixture against the schema shape
(`ark_payout.status` + `pending_balance.sats`, 2000 one-sat scheduled rows,
a writer committing one atomic `scheduled -> failed_to_pending` plus
`pending += sats` transaction at a time, i.e. the real
`mark_ark_failed_to_pending` shape): over a 30 s run, the candidate's single
statement read `pending + ark` 1236 times with **0** deviations from 2000;
the same two sums issued as two separate statements in one session tore
**40/1236** times (sum 1999 or 2001). So the property the contract wants is
real, observable, and cheap to test; the current test just does not look at
it.

Minimal fix (test-only, ~20 lines, `liability_snapshot.rs`): seed N
scheduled 1-sat Ark rows (N >= 50; the 2 ms sleep can go), spawn a sender
that calls `store.mark_ark_failed_to_pending(block, &sp_id)` for each, and in
the snapshot loop assert `snap.total_sats == N` on every observation (the
conserved sum), plus the existing mid-transition observation requirement.
Keep the current `mark_ark_sent` loop if desired, but the conserved-sum
assertion is the one that distinguishes one snapshot from two. The pool-level
test (`real_store_observation_serves_the_contract_fixture`) does not need to
change.

### R2. `broken_schema_with_secret_dsn_leaks_nothing` leaks one fixture database per run

`bins/spc-pool/tests/operator_observation.rs:778` calls `isolated_fixture()`
only to obtain `admin`, discards the created database name, and drops only
its second database. After my single run the server still held
`expop_testfixtures_ea27e1bf3d0b4cc59f59a4253e0d08db`
(`/tmp/opencode/EXP-OP-fable/leaked-dbs.txt`). The report's "fixture-database
cleanup verified clean afterward" was a manual sweep, not the test. Fix:
connect `admin` directly (`PgPool::connect("{head}/postgres")`) instead of
going through `isolated_fixture()`, or drop the returned `name` at the end.

### R3. Poll loop has no in-flight guard and no ordering guard

`operator.js` fires `fetch` from `setInterval` every ~10 s unconditionally
and renders whichever response resolves, whenever it resolves. Two
consequences, both reproducible from the measured 30 s outage bound:

- During an outage the page holds up to three concurrent observation
  requests (30 s / 10 s); each holds three pool-acquire waiters server side.
  Bounded today only by the server timeout and the browser's per-host
  connection cap, not by the page.
- Out-of-order overwrite: request A issued during an outage times out at
  t+30 s with all fields `null`; if the store recovered at t+5 s, request B
  (t+10 s) already rendered fresh data at t+10 s; A's arrival at t+30 s then
  flips every fact to `unknown (store_unavailable, postgres)` and sets
  `lastGoodAt`, although the store is healthy, until the t+40 s poll. The
  failure direction is safe (unknown, not stale-as-current), but the page
  advertises an outage that is over, and the same mechanism could in
  principle let an older successful response land after a newer one.

Minimal fix (~15 lines): keep an `inFlight` flag (skip the tick, or abort
the previous request with `AbortController`, when set) and a monotonically
increasing request sequence number; ignore any response whose sequence is
lower than the last rendered one. Optionally compare `generated_at` as a
belt-and-braces check. Because this edits `operator.js`, which is
`include_str!`-embedded, the binary hash changes and the browser proof must
be re-run; fold the F4 items into that rerun.

### F4. Browser-proof coverage gaps (fold into the R3 rerun)

- The 390x844 "no horizontal overflow" and "table overflow contained"
  checks ran with Details closed (the harness closes it with Enter before
  switching viewports), so the wide table was never laid out at mobile
  width; the CSS makes containment very likely, but the proof does not show
  it. Open Details before measuring at 390x844 and add a screenshot.
- Subscribe to `Runtime.exceptionThrown`; uncaught page exceptions do not
  necessarily surface through `consoleAPICalled` or `Log.entryAdded`, so
  "console clean" is currently under-observed.
- `wrapContained === true || wrapContained === null` passes when the wrapper
  is absent; make it `=== true`.
- `started` in `browser-proof.json` is written at the end of the run
  (misnamed `finished`). Cosmetic.

### F5. Stale values are shown indefinitely after fetch failures

On a transport/HTTP failure (pool process down, listener gone) the page keeps
the last values visible with a red banner "showing data as of Ns ago" and a
`.stale` class forever. Contract 3.1's vocabulary says a stale source is
`unknown` with its reason. The banner is explicit and `aria-live`, so this is
not "stale as current", but an age cap (e.g. after 3 missed polls, replace
the five facts with `unknown (observation unreachable)`) would match the
vocabulary. Merge owner's call; could ride with R3.

### F6. Minor code notes (non-blocking)

- `/api/pool/observation` has no `Cache-Control: no-store` header while the
  three static routes do; the page requests with `cache: 'no-store'`, so no
  functional effect. One-line consistency fix.
- `formatSats` accepts a leading `-` and renders negative sats; the server
  never emits one (u64), but for a sats field "negative" could be treated as
  `invalid value`.
- The operator bind happens after the public API bind and after payout/front
  tasks are spawned; a collision therefore aborts a process that already
  started those tasks. Verified clean exit, no half-running state; binding
  the operator listener before the role tasks would be marginally tidier.
- `StoreError::is_schema_mismatch()` covers 42P01/42703 plus
  `ColumnNotFound`/`ColumnDecode`/`TypeNotFound`; the real broken-schema test
  accepts either `schema_mismatch` or `store_unavailable`, which is loose but
  honest about sqlx's error surface.

## 4. Decision record, changelog, example config, merge shape

- `docs/decisions/2026-11-08-pool-operator-page.md`: nine numbered
  decisions, all matched by the code I read; item 8 documents the api-only
  lifecycle quirk honestly; "Edits applied" lists exactly the 15 files.
  Binding as written; no SPEC section is changed, so no SPEC cross-reference
  edits are needed.
- `docs/changelog/2026-11-08-pool-operator-page.md`: one bullet in the
  `docs/changelog/README.md` format (two leading spaces, `- "..."`), names
  the decision record; `docs/SPEC.md` untouched (correct per the fragment
  policy).
- `config/spc-pool.example.toml`: commented `[operator]` block after `[api]`,
  states loopback-only, api-role requirement and the four routes; the
  example still validates (`example_config` test, cited; the commented block
  cannot change parsing).
- Serial-merge hunks are small and additive: `config.rs` (one field, one
  struct, three error variants, one validation block), `main.rs` (one
  pre-check, one `store.clone()`, one spawn block, one `tasks.push`),
  `lib.rs` (one line), `postgres.rs` (additive items only, nothing existing
  edited), `Cargo.toml` (dev-deps). They will merge as hunks alongside AP-2's
  pending `postgres.rs` ownership without whole-file transfer.
- Runtime enablement: nothing in `crates/spc-lab`, `scripts/lab`,
  `testing/lab-*`, `nix/` or `docs/OPERATIONS.md` sets `[operator]`. The
  page is **implemented, not running**: an operator must add `[operator]
  listen = "127.0.0.1:<port>"` to a pool TOML whose roles include `api`
  until lab/config deployment wires it. The report says this correctly.

## 5. Report accuracy

The report is accurate on scope, delta, tests, the 90 s -> 30 s fix (I
measured 30.02 s for three concurrent requests), the browser proof's
fixture-only nature, and the binary provenance caveat. Two wording nits:
"truncated hex label the schema itself stores" (section 2.3 above: the DTO
carries the full 64-hex block hash, which is allowed) and "fixture-database
cleanup verified clean afterward" (R2: the test itself leaks one database).
Neither changes the verdict.

## 6. Scope statement

- This review approves nothing beyond EXP-OP as bounded by contract v3
  section 4: no Ark float source, no auth platform, no controls, no
  cross-listener fetch. `float.not_configured` until AP-4 is approved scope,
  not a gap; EXP-OP is not gated behind ACCESS/AGENT/AP-4 or any funded
  chain.
- No live Ark, payout, mining or E2E claim is made or implied; every number
  exercised here is a TESTFIXTURES value on a throwaway database.
- Reviewer artifacts: `/tmp/opencode/EXP-OP-fable/` (preserved binary copy
  `spc-pool.candidate` = `40442310...`, cargo logs, pool logs, outage DTOs,
  fixture TOMLs, leaked-db listing). My Postgres cluster is stopped and its
  data removed; no pool, Chrome or Postgres process of mine remains; no
  candidate file was edited; nothing was committed.
- Re-review of the rework needs only: the new per-file hashes of
  `crates/spc-store/tests/liability_snapshot.rs`,
  `bins/spc-pool/tests/operator_observation.rs`,
  `bins/spc-pool/static/operator/operator.js`, a fresh binary hash with
  before/after records, the R1 test output on a real database, and the
  re-run browser proof including the F4 mobile open-Details check.

---

# Rework re-review (same day)

Date: 2026-11-08 (second cycle, same reviewer and session).
Candidate: the same worktree, rework of R1-R3 with F4-F6 folded in.
Authority manifest: `/tmp/opencode/EXP-OP-rework-parent-manifest.json`, 592
rows, canonical snapshot
`d3710c0c3100c86251abd377043b80d1ffcec5b9ba1eca64bf6f97c69e91d1d4`, code
snapshot (report excluded)
`365e0bf200a98252278dec495bbb68d341c0f118e1bf4acbb8de175807a80614`.
Sealed binary: `/tmp/opencode/EXP-OP-rework-freeze/spc-pool.candidate`,
sha256 `1684cc8402a3d496ea30ea0a9b6faccc747de4c43d6dc20e8e90917130448096`
(independent inode; the first-cycle `40442310...` copy and its proofs are
preserved alongside).
Browser proof: `/tmp/opencode/EXP-OP-rework-browser/`.
The first review above is preserved unchanged.

## Result

**APPROVE**, bounded to code snapshot `365e0bf2...` and binary
`1684cc84...`.

R1, R2 and R3 are closed as specified; F4, F5 and F6 are folded in and
verified. The six changed files vs. the first candidate are exactly
`operator.rs`, `operator.js`, `operator_observation.rs`,
`liability_snapshot.rs`, the decision record and the report; no startup
order, config, public API, SQL or shared-hunk change. No remaining
counterexample. The remaining notes (section 4) are wording/hardening nits
for the merge owner's discretion and do not gate integration.

Approval covers EXP-OP as bounded by contract v3 section 4: read-only,
loopback-only, off-by-default observation; `float.not_configured` until
AP-4 is approved scope; no Ark, payout, mining or E2E claim; first manual
runtime/config wiring (`[operator] listen` in a pool TOML whose roles
include `api`) remains after approval. EXP-OP is not gated behind the Ark
suite or EXP-ACCESS.

## 1. Provenance

- Canonical recipe reproduces `d3710c0c...` and `365e0bf2...` from the
  manifest; all 592 rows re-hashed and mode-checked against the worktree
  (0 mismatches), inventory equal to `git ls-files --cached --others
  --exclude-standard` (0 extra, 0 missing). Diff against the first-cycle
  manifest: exactly the six files named above. Re-hashed after all my
  runs: 0 drift.
- Binary provenance, settled deterministically: with the frozen source,
  `cargo test -j 2 -p spc-pool --test operator_observation --no-run`
  leaves `target/debug/spc-pool` at exactly `1684cc84...` (reproduced
  twice), while a plain `cargo build -j 2 -p spc-pool` of the same source
  yields `2dbaadb8...` (reproduced twice, also after a `touch`). The two
  differ only in cargo's dev-dependency feature unification (the test
  invocation builds the bin alongside the integration test, so shared
  deps such as `spc_store`/`sqlx` and `tracing_subscriber` carry the
  dev-dep features; fingerprint diff in `.fingerprint/spc-pool-5844fc8d*`
  vs `spc-pool-700134e6*`, features `["default"]` and profile identical in
  both). So the sealed `1684cc84...` is the dev-profile `spc-pool` bin as
  linked by the test invocation of the frozen source - same source, same
  profile, default features; the report's "cargo build (dev profile)"
  wording is imprecise (section 4). Additional source-binding checks on
  the sealed copy: `GET /operator.js` byte-equals the frozen
  `operator.js` (sha256 `349d7fb0...`), and the binary byte-contains all
  three frozen assets and the `inFlight` guard; the preserved
  `40442310...` does not contain the rework JS.
- Worktree state at hand-back: `target/debug/spc-pool` is at
  `2dbaadb8...` (my plain build); a final `--no-run` restore to
  `1684cc84...` did not run because the heavy-build lock was held by
  another session at that moment (reported once, not polled). The sealed
  copies are unaffected; the parent should treat the freeze copy as the
  pinned artifact, not the worktree path.
- Resource policy: every cargo step under nonblocking `flock`; 95 then 89
  GiB free checked inside the lock, no measurable growth; candidate
  `./target` reused; no workspace or Nix build.

## 2. Disposition of R1-R3

### R1 - closed

`settle_racing_the_snapshot_never_tears_the_pair` now seeds 64 one-sat
scheduled Ark rows, moves them with the real
`Store::mark_ark_failed_to_pending` transaction (the production
`scheduled -> failed_to_pending` + `pending += sats` path), and asserts on
every concurrent snapshot `total_sats == 64` and `pending + ark == 64`
against the seeded constant, plus a required mid-transition observation,
final pending 64 / ark 0, and the real monotonicity of the two halves. The
oracle is no longer the Rust `checked_add` identity. The new
`torn_two_statement_counterfactual_is_detectable` forces the interleave in
program order (read pending; commit exactly one move; read ark) with no
sleep or probability, shows the two-statement sum is `N-1`, asserts it
differs from `N`, and shows the single-statement snapshot at the same
logical point is exact (pending 1, ark 2, total 3). The production SQL is
unchanged (`postgres.rs` hash equal to the first cycle). Both tests passed
on my fixture cluster (5/5 in 0.46 s).

### R2 - closed

`broken_schema_with_secret_dsn_leaks_nothing` connects its admin pool
directly (no `isolated_fixture()` call, so no discarded database), collects
assertion failures into a vector so cleanup runs on every path, drops both
the database and the login role it created, then queries `pg_database` and
`pg_roles` by name and asserts neither remains. Verified independently:
after the real-DB tests executed on my fresh cluster (no `skip` output
under `--nocapture`), the only `expop%` database left was my own
`expop_fable2` and there were zero `expop%` roles
(`/tmp/opencode/EXP-OP-fable/leaked-dbs-rework.txt`). The report now
states accurately that the earlier leaks were removed by a manual sweep,
not by test cleanup.

### R3 - closed

`operator.js` keeps `inFlight` (a tick while a request pends returns
without fetching), a monotonic `requestSeq`/`lastRenderedSeq` pair (an
obsolete completion or failure is dropped before any state change), and
`applyPayload` advances `lastGoodAt`/`lastGoodDto` only when
`dto.errors.length === 0` - an HTTP 200 whose fields are null renders the
server's unknowns but is not freshness. Partial-error payloads are still
rendered field by field as served (no masking); only the freshness clock
is withheld. The browser proof's R3 regression (a stubbed 12 s first
response) records `calls=1` at t=11 s and `maxActive=1` (the old loop
would have issued a second overlapping fetch at t=10 s), then the delayed
payload renders as the newest request and the next poll restores the real
payload. Under the guard the sequence check is belt-and-braces (requests
are serialized), which is the right shape.

## 3. Folded items

- **F4**: the 390x844 overflow check now runs with Details open (asserted
  `open === true` at mobile size), the wrapper check requires
  `contained === true` and `overflow-x: auto`, `Runtime.exceptionThrown`
  is subscribed and verified each run by an injected probe exception
  (captured, then reset before the real check), and `browser-proof.json`
  carries real `started`/`finished` timestamps and the candidate sha.
  46/46 checks pass; all 29 first-cycle checks are present and re-run on
  the new binary. `mobile-390x844-open-details.png` shows the disclosure
  open; the table rows themselves are below the fold in the capture (the
  containment is measured programmatically, not shown).
- **F5**: after `STALE_AFTER_MS = 30 s` without good data on a transport
  failure, `renderUnreachable` sets the five facts to
  `unknown (observation unreachable)` with a banner naming the age; before
  the cap the old values stay with the explicit stale banner. The proof
  kills the pool process (SIGKILL), observes the pre-cap banner
  ("showing data as of 9s ago") with the last good value, the post-cap
  unknowns (distinct from the DB-outage `unknown (store_unavailable,
  postgres)`, no zero), then respawns the pool and observes recovery and
  banner clearance; store stop/restart recovery is verified separately.
  `transport-unreachable.png` matches.
- **F6**: `Cache-Control: no-store` on all four routes - asserted in the
  router test and confirmed by me on the sealed binary as a real process
  (`/`, `/operator.css`, `/operator.js`, `/api/pool/observation` all
  `no-store`; favicon/`/api/pool`/`/healthz` 404; POST 405; GET with body
  400; public listener still 404s the operator paths and serves
  `/api/pool`, `/healthz`). A leading `-` in a sats string is rejected by
  `formatSats` (renders `unknown (invalid value)`; proof check present).
  Startup order unchanged (`main.rs` hash equal to the first cycle).

## 4. Non-blocking notes (merge owner's discretion)

- Report wording: "cargo build (dev profile)" should read "the dev-profile
  `spc-pool` bin as linked by `cargo test -p spc-pool --test
  operator_observation` (dev-dep feature-unified)". Future freezes should
  record the exact cargo invocation next to the hash; the Nix `rpc` build
  remains the production artifact path.
- `fetch` has no client-side timeout; with `inFlight` set, a connection
  that never settles (e.g. a SIGSTOPped listener) suspends polling until
  the browser's own network timeout fires, then resumes. An
  `AbortController` deadline of roughly two poll intervals would bound
  that. Not reachable through any path exercised by the contract.
- Two banner wordings are slightly imprecise: a transport failure within
  30 s of a DB-outage payload says "showing data as of Ns ago" while the
  facts on screen are the server's unknowns; and a transport failure before
  any error-free payload says "No observation yet" even if an all-errors
  payload was received. Both are fail-safe (unknown, never a value).
- The mobile open-Details screenshot does not show the table rows; a
  scrolled capture or a taller viewport would make the visual record match
  the programmatic check.

## 5. Scope statement

- Verified by me this cycle: manifests, inventory, six-file diff against
  the first-cycle freeze, both test suites on a fresh fixture cluster (5 +
  16, real-DB tests executed), leak check by name, `--features rpc`
  check, binary reproducibility under both invocations, the sealed
  binary as a real process (routes, headers, methods, bodies, public
  scope, loopback bind with `[api]` on `0.0.0.0`), the harness source and
  all 46 results, screenshots and pool log, and the absence of lingering
  pool/Chrome/Postgres processes or data dirs from the author's run.
- Not rerun (cited): the author's browser proof itself, fmt/clippy, and the
  full `spc-store`/`spc-pool` suites; the delta touches no other test.
- Reviewer artifacts: `/tmp/opencode/EXP-OP-fable/` (`spc-pool.rework` =
  `1684cc84...` independent copy, `cargo-test-rework.log`,
  `cargo-rework-check-build.log`, `leaked-dbs-rework.txt`, `pool-b.toml`,
  `pool-b.log`). My second Postgres cluster is stopped and its data removed;
  no candidate file was edited; nothing committed; no shared cleanup
  performed.
