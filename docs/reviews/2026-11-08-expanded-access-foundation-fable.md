# Fable concrete-code review: EXP-ACCESS foundation (claims, auth, replay, filtered reads)

Date: 2026-11-08
Reviewer: Fable (concrete-code review subagent)
Candidate: `/home/ethan/.local/share/opencode/worktree/ad8886/lab-expanded-access`,
packet = 12 files under `crates/spc-lab` and `docs/` (6 modified, 6 added)
Candidate report: `docs/reviews/2026-11-08-expanded-access-foundation.md`
(in the candidate worktree)
Contract: plan section 5 of
`/tmp/opencode/role-ui-plan-review-v5/2026-11-07-live-lab-integration-contracts.md`
(sha256 `05c0d4f8...` re-verified; components plan `cd218538...` re-verified)
Disposition: **REQUEST_CHANGES** (ranked counterexamples in Section 4; each
has a minimal fix and a regression test). The security core is sound and the
legacy routing holds; the blockers are one gate failure, one cross-principal
read leak, and one revocation gap that the candidate's own test pins as
policy rather than fixes.

Nothing in the candidate, the primary tree or any other worktree was edited.
No commits, services, nodes, miners or Ark processes; no cache cleanup. This
file is the only artifact written into the primary tree. Scratch evidence:
`/tmp/opencode/EXP-ACCESS-fable-fmt.log` (sha256 `ba6cd57c...`).

## 1. Snapshot verification (actual bytes)

Recomputed independently from the worktree with the manifest recipe
(`sha256(compact sorted-key JSON {base, files})`, files path-sorted):

| Quantity | Parent freeze (`/tmp/opencode/EXP-ACCESS-review-manifest.json`) | Recomputed |
|---|---|---|
| file records | 589 | 589 (every content hash matches; `git ls-files --cached --others --exclude-standard` yields exactly the same 589 regular files, 0 extra, 0 missing) |
| full snapshot | `7a7087e070d2be2172934bc18860b3a3ed2579d5363f85e0ade355a5185e0020` | identical |
| code snapshot (excludes the report) | `d3a5f078a5f3369a0c4f8258809f7a926f805b7995c4550428c0f4b172da3d73` | identical |
| baseline manifest sha | `ab174afe...` | identical; baseline snapshot `3e2a31be...` recomputes from its 583 records |

Delta against the baseline: **0 removed, 6 modified, 6 added**, exactly the
report's Section 1 list. The six modified files are in `crates/spc-lab/src`
only; root `Cargo.toml`, `Cargo.lock`, `crates/spc-lab/Cargo.toml`, flake and
config are byte-identical to the baseline (no dependency change). The six
`src` hunks are the ones described; the T0 `check_traffic_capability` guard
in `RunEngine::prepare` is preserved and the new `derive_user_bindings` check
sits after it, before any filesystem effect.

Two corrections to the candidate report's framing:

- The report calls the baseline "the OP worker's uncommitted delta". It is
  not: the baseline manifest's `source_root` is `/home/ethan/code/silent-pool`,
  and the six modified files hash identically to primary's current copies
  (`b3baec02`, `62d05899`, `bf221dd3`, `a8642ac5`, `712ee4d0`, `b1fec4af`).
  The baseline is the shared primary snapshot taken before OP, not an OP
  implementation delta. The packet is therefore a clean delta on primary's
  `spc-lab`, which is good news for the merge owner.
- The manifest's `delta` list records the report at `05f6a0dd...` while the
  `files` list (and disk) have `d751f1b3...`: the report was edited after
  the delta list was produced. Harmless because the code snapshot excludes
  the report, but the delta list should be regenerated with the resubmission.

## 2. What was run and what was not

Under `flock -n -o /tmp/opencode/silent-pool-heavy-build.lock`, 87 GiB
available after acquiring the lock:

- `cargo fmt -p spc-lab -- --check`: **FAILS** with 53 diffs, all in packet
  files (`access.rs` 9, `command.rs` 2, `engine.rs` 3, `http.rs` 3,
  `tests/access_projection.rs` 12, `tests/access_scope.rs` 24). The baseline
  copies of `command.rs`, `engine.rs` and `http.rs` are rustfmt-clean under
  the repo `rustfmt.toml`, so every diff is introduced by this packet. The
  justfile gate is `cargo fmt --all -- --check`; this blocks integration as
  it stands (Section 4, F1).
- `cargo clippy -p spc-lab --all-targets -- -D warnings`: **not run**. The
  heavy-build lock was held by another session when I reached this step;
  per policy I report it once and did not poll. Static reading finds three
  diagnostics that `-D warnings` will reject: `clippy::useless_format` at
  `tests/access_scope.rs:240` and `:241` (`format!("/api/lab/v1/health")`,
  `format!("/api/lab/v1/runs")`), and `clippy::needless_range_loop` at
  `access.rs:541` (`for index in 0..TOKEN_BYTES { diff |= a[index] ^ b[index] }`).
  Treat clippy as red until a receipt exists (F1).
- Tests: not re-run (no new change to justify a second full run; the lock
  was busy). The parent's receipts were checked instead:
  `/tmp/opencode/EXP-ACCESS-test-run5.log` 9 + 5 = 14 green,
  `EXP-ACCESS-test-legacy.log` 1+6+3+7+3+1+8+4 = 33 green,
  `EXP-ACCESS-test-units.log` 40 green, hashes match
  `EXP-ACCESS-receipts.sha256`. `pgrep -f spc-lab-fixture` shows no leftover
  fixture process. The tests stop the engines they start (`engine.stop()` /
  `attached.stop(); attached.reap()`), consistent with `Drop` not killing
  children; the two tests that never `start()` spawn nothing.

## 3. What holds (verified by reading the actual code paths)

- **Opt-in boundary.** `RunEngine::prepare` sets `access: Some(..)` iff
  `scenario.expanded_access`; `RunEngine::attach` (adopt and read-only) does
  the same and errors `access_claims` if the claim directory or the operator
  claim is missing. There is no other constructor. `handle_client` switches
  on `engine.access().is_some()` before any legacy match arm, so an expanded
  run never reaches the unauthenticated routes - including after a failed
  reload (the store stays `Some` with an empty claim map, denying everyone)
  and on a read-only attach. `Scenario` is `deny_unknown_fields`, so an old
  binary cannot even load an expanded run's `private/scenario.json`.
- **Programmatic path.** `engine.submit()` is only called by in-process code
  (runtime phases, CLI); the expanded HTTP POST calls `submit_authenticated`,
  which runs `check_command_scope` before `submit_as`. In expanded runs the
  harness principal is `controller`, so a harness `command_id` presented over
  HTTP by any claim is `Denied` (principal mismatch).
- **Static exception is closed.** Pre-auth GETs are exactly `/`,
  `/index.html`, `/app.js`, `/style.css`, `/favicon.ico` (embedded
  `include_str!` assets, no runtime data) and the three relay assets
  (`index.html`, `workbench.css`, `workbench.js`) for a role that resolves
  through `addresses["<role>.api"]`. `status` and everything else are behind
  the Bearer gate. `split_query` percent-decodes the path exactly once before
  routing, and every asset is exact-matched, so `%73tatus`, `%2F`, `..`,
  double-encoding all end at 401/404 - no alias reaches the private branch.
- **Authentication.** Exactly one `authorization` header, `Bearer ` prefix,
  hex decode to exactly 32 bytes, constant-time compare, per-match existence
  check. Zero/duplicate/malformed/unknown/revoked all return the same 401
  before any journal lookup (`authenticate_request` precedes the route
  match). No loopback fallback: operator authority is exactly the operator
  claim. Tokens are never echoed (`me_view` omits `token`; tests assert no
  page body contains a token).
- **Scope and replay.** `check_command_scope` rejects payload actor !=
  claim actor, cross-miner, cross-worker and non-permitted kinds with a
  `rejected_precondition` outcome and no journal write (tests assert the
  line count). `begin_scoped` checks scheme, principal and hash before
  returning any stored outcome; legacy (`None`) and unknown scheme entries
  are `Denied` with the generic detail; cross-principal is `Denied`; same
  principal + changed payload is a durable `Conflict`. `CommandOutcome`
  carries the request's `command_id`. Legacy `begin()` denies
  scheme-bearing entries. `LogLine::Begin`'s new fields are `Option` with
  `skip_serializing_if`, declared after `payload_hash`, so a `None` line
  serializes to the baseline bytes by construction.
- **Browser boundary.** `check_same_origin` compares `Origin` to
  `http://<stream.local_addr()>` (never the `Host` header) and rejects
  `Sec-Fetch-Site: cross-site`; applied to `POST /commands`; no CORS header
  anywhere; CSP on expanded HTML; relay is GET-only (405), declared targets
  only, closed MIME map, 1 MiB body cap over `namespace::http_request`'s
  30 s timeouts, upstream failure is a bodyless 502. Unsupported T0 kinds
  (`send_user_payment`, `bootstrap_fund`) are detected pre-parse and answer
  `rejected_precondition` "not available"; they cannot report success.
- **Filtered reads.** `filter_event` and `user_snapshot` run before
  serialization, allowlist by kind and own identity/miner, strip the raw
  miner payload, keep the global `last_event_id`, `gap_count`, `gaps`,
  `retained_from`, `resync_required` and `cursor` so a filtered-empty stream
  is not mistaken for healthy; `EventLost` passes to everyone as a gap fact.
  Wallet/payment are `not_available` with a reason, never zero. The
  `Disconnected` (overflow) branch answers with a filtered resync, matching
  the operator path. The user SSE loop re-authenticates before every event
  and at the 15 s heartbeat and closes on revocation (test bounds it under
  20 s).
- **Miner-poll projection.** `MinerControl::request` correlates replies by
  a generated `command_id` through a pending map and a reader thread, with a
  mutex on stdin and a bounded `recv_timeout`; a projection poll concurrent
  with the runtime observer poll cannot steal a response or hang the
  process. The projection reads `requested.hashes_per_second`,
  `achieved_hps`, `active_job_id` - the real `WorkerSnapshot`/`HashBudget`
  shape in `bins/spc-cpu-miner/src/report.rs`. Failures are `null` plus a
  closed reason string (the wire error text is not forwarded).
- **Privacy constraints untouched.** No `ark` role, epoch, signed counter or
  D file is in the delta; no identity field crosses into any Ark stream.

## 4. Counterexamples, ranked (REQUEST_CHANGES)

### F1 (blocking, gate): fmt fails; clippy very likely fails

See Section 2. Fix: `cargo fmt -p spc-lab`; replace the two `format!("...")`
literals with `.to_string()`; write `constant_time_eq` over `a.iter().zip(b)`.
Re-manifest afterwards (the code snapshot hash will change). Add the fmt and
clippy receipts to the report; the current report lists neither.

### F2 (blocking, cross-principal leak): a user token reads the whole-run shareable artifacts

`handle_expanded` routes `("GET", Route::Artifact(path)) => serve_artifact`
for **any** authenticated claim. `shareable/manifest.json` (written by
`write_artifacts` at stop and on `refresh_artifacts`) contains
`commands: [{command_id, kind, state}]` for every principal's command,
`evidence.counts` per kind for the whole run (the very aggregate
`user_snapshot` deliberately omits), `resources` (every redis/postgres port
and owner pool) and `pool_authorities`. Counterexample: after the run stops
(or any refresh), bob does
`GET /api/lab/v1/runs/<run>/artifacts/shareable/manifest.json` with his
own token and learns alice's `command_id`s and their outcome states - the
exact fact `BeginOutcome::Denied` exists to withhold. Plan 5.3: "the
whole-run routes stay operator/legacy-only". No test exercises `/artifacts`
with a user token.

Minimal fix: in `handle_expanded`, `("GET", Route::Artifact(_))` returns 403
unless `auth.claim.is_operator()` (mirror the `Catalog` arm). Regression
test: user token on `shareable/manifest.json` is 403; operator token is 200;
tokenless is 401.

### F3 (blocking, revocation gap pinned as policy): in-place claim edits are invisible to the store

`AccessStore::authenticate` reloads only when the **directory** mtime
changed. Overwriting a claim file in place (`std::fs::write`, `echo >`,
editors that write in place) does not change the directory mtime, so:

- an operator who "rotates" a token or removes a worker from `worker_ids`
  by overwriting the file leaves the OLD token and OLD scope valid
  indefinitely, while the NEW token does not work - until an unrelated
  directory change (another revoke, a stray file) flips the whole store;
- a corrupt file is likewise latent until that unrelated change, at which
  point ALL principals are denied at once.

The candidate's own test
(`revocation_closes_access_without_touching_other_principals`) needs a
`reload-trigger` marker file to make corruption observable, and decision
record item 2 writes this down as the behavior. I do not accept that as
approved policy: the plan says "Missing, unknown or revoked tokens are
rejected before any ... effect", and nothing in the plan, decision, claim
file or report tells the operator that the only supported revocation is
`rm`. The API as written supports exactly immutable-claims-plus-deletion,
and the surprising failure mode (rotate => old token lives, new token dead)
is a fail-open relative to operator intent.

Minimal fix (either is acceptable; the first is simplest and still cheap at
lab scale): (a) reload unconditionally on every `authenticate` (a handful
of 0600 files; `claim_file_exists` already costs one stat), or (b) key the
reload on a fingerprint of the directory listing
`[(name, mtime, len, ino)]` rather than the directory mtime alone. In both
cases keep the fail-closed clear on parse failure. Regression tests: (1)
overwrite alice's file with a fresh token - the old token is 401 on the
very next request and the new one is 200, with no directory change; (2)
overwrite bob's file with `not json` - bob is 401 on the next request, and
state the all-or-nothing consequence in the test name. Then rewrite
decision item 2 and add one sentence to the report's "token distribution"
paragraph: revoke by deleting or rewriting the file; both take effect on
the next request.

### F4 (should fix before merge, closed-charset namespace): reserved principals and filename collisions are not rejected at prepare

`IdentityId` accepts `operator`, `controller` and `sha256-<64 hex>`.
`AccessStore::prepare` writes the operator claim first and then, for an
identity literally named `operator`, writes the user claim to the same
`operator.json` and replaces the operator entry in the map: the run comes up
with **no operator claim** (silently), and the next `attach` fails closed
with "requires the operator claim". An identity named `controller` shares
the harness replay principal, so the user and the in-process runtime are the
same principal in the journal. Two identities `u0/x` (unsafe, hashed to
`sha256-H.json`) and a literal `sha256-H` (safe charset, verbatim) collide on
one filename; after a reload only one survives. None of these is an
escalation, but each silently breaks the "exactly one operator / one claim
per principal" invariant and the plan's closed-ID discipline.

Minimal fix in `derive_user_bindings` (already the pre-effect validation
point): reject `user_id` in `{operator, controller}`, reject any id starting
with `sha256-`, and reject two bindings whose `claim_filename` collide.
`load_dir` should also reject a duplicate `principal` or duplicate `token`
across files instead of last-wins. Regression: a scenario with identity
`operator` fails `prepare` with `access_bindings`; no directory is created.

### F5 (should fix, secret-file hygiene): claim files are not validated on load

`load_dir` reads every `*.json` regardless of type, mode or owner. The
AP-3a decision set the project norm for secret-bearing files ("0700/0600
before open, reopen re-validated"). Here a claim directory loosened to 0755,
a claim file at 0644, or a symlink dropped into the directory is accepted
silently. Minimal fix: `symlink_metadata` each entry; require a regular
file, `mode & 0o077 == 0`, same uid as the process, and the directory at
`mode & 0o077 == 0`; otherwise `Err` (which the existing fail-closed branch
turns into deny-all). Regression: chmod a claim to 0644, next request is
401 for everyone until it is restored.

### F6 (should fix, consistency): the operator SSE stream never re-authenticates

`("GET", Route::Events)` for the operator calls the legacy `serve_events`,
which has no `still_valid` check. Deleting `operator.json` leaves an open
operator stream delivering unfiltered events until the client disconnects.
The plan's revocation-on-open-stream rule is not user-specific. Minimal
fix: route the operator through the same re-auth loop (a `filter` that is
identity for operators), or add the `still_valid` check to `serve_events`
when `engine.access().is_some()`.

### F7 (should fix, scope robustness): empty `workers` relies on a prepare-time invariant the operator can break at runtime

`check_command_scope` allows `PauseWorkers { workers: [] }` (pause the whole
miner) for a user because `derive_user_bindings` proved one identity per
miner at prepare. `AddConnection`/`AddWorker` (operator commands) can later
attach another identity's connection or worker to the same miner; the claim
is a prepare-time snapshot and is not re-derived. Minimal fix: for user
claims, require `workers` to be non-empty and a subset of the claim's
`worker_ids` (the browser knows its own ids from `/me`); keep the empty form
operator-only. This also removes the load-bearing dependency the comment in
`access.rs:113-121` describes.

### F8 (report accuracy and documentation; fix in the resubmission)

- "a legacy `begin()` facing a scheme-bearing entry fails closed" is listed
  under `expanded_run_fails_closed_on_crafted_journal_lines`, but that test
  only exercises `begin_scoped` against a legacy line and an unknown-scheme
  line. The `begin()` denial exists in code and is untested. Add a direct
  `CommandLog` test: write an `expanded_v1` line, call `begin()` with the
  same id and kind, expect `Denied`.
- "byte-identical before and after the scheme change" is asserted by
  checking the new journal text does not contain `hash_scheme`/`principal`.
  That is the new codec checked against itself. Add one frozen pre-change
  `begin` line as a string fixture (taken from a baseline run) and assert
  the new codec round-trips it byte-for-byte and `begin()` returns
  `Existing` for it. (By construction it will pass; the point is that the
  invariant is pinned against real bytes.)
- "atomic rename (no partial file), token entropy" in the claims test: the
  test asserts names, modes and location; it does not observe atomicity or
  entropy. Either add the assertions (no `.tmp-claim-*` left behind; 64
  distinct tokens across 64 generated claims) or drop the words.
- `bin/fixture.rs` is described as "additive"; it also changes the default
  `occurred_at_mono_ms` from `0` to `7` for every fixture miner snapshot.
  Harmless (nothing reads it), but say so or revert it.
- `relay_target` keys on `addresses["<role>.api"]`. The runtime populates
  `front.<id>.api_addr`, `node.<id>.rpc_addr`, etc., and `attach` starts
  with an empty map; no runtime path ever inserts a `<role>.api` key. The
  relay is therefore reachable only through `EngineOptions.addresses` in
  tests today - fail-closed (404 for every role), which is fine for a
  foundation, but the report's "declared topology" wording overstates it.
  State plainly: the mapping is AP-4's; until then every relay role is 404.
- The runtime's partition-phase controller probe
  (`runtime.rs:620`, tokenless `GET /api/lab/v1/health`) will record
  `reachable: false` in every expanded run. Not a security issue (the probe
  is evidence, not authority), but it is a functional regression in the
  partition evidence for expanded scenarios. Either give the runtime the
  operator token for that probe or note it as a known limitation.
- In the expanded profile `/` serves the legacy operator SPA
  (`bins/spc-lab/static`), which has no way to present a Bearer token and
  will show 401s. The report calls it the "public static bootstrap"; it is a
  dead page with a CSP header. Say so; the workbench page is EXP-AGENT's.
- The `Origin` check compares against the socket's literal address
  (`127.0.0.1:<port>`); a page opened as `http://localhost:<port>` will have
  its POSTs rejected. Fail-closed, but EXP-AGENT needs to know the canonical
  origin is the IP literal.

## 5. Items reviewed and found acceptable as-is

- Dir-mtime reload race on delete is covered by the per-match existence
  check; a replace within one timestamp tick is subsumed by F3's fix.
- `CommandLog::open` skipping unparseable lines is pre-existing behavior.
  An old binary cannot reach an expanded journal (scenario gate), so the
  "old reader fails safely" property holds via the scenario, not the codec;
  worth one sentence in the decision record. A later hardening could
  distinguish a torn final line from an interior unparseable line.
- `Finish` for an unknown id is ignored and a stored `payload_hash` is
  trusted at load; the journal is 0600 in the same trust domain as the
  claims, so this is not a finding for this packet.
- Operator `user_projections()` polls every miner serially with the control
  timeout; bounded.
- The unauthenticated relay static branch lets any local client trigger an
  upstream GET to a declared address; bounded by `MAX_CONNECTIONS` and the
  30 s timeouts, declared targets only. Acceptable on loopback.
- Changelog fragment is a prose paragraph with a heading, matching the
  existing `2026-11-08-lab-traffic-contract-t0.md` fragment rather than the
  README's bullet form; the merge session already splices this shape. No
  direct `SPEC.md` edit (correct).

## 6. Decision

**REQUEST_CHANGES.** Required before I would approve the bounded
foundation: F1 (fmt + clippy receipts), F2 (artifacts operator-only), F3
(reload semantics that honor in-place edits, plus the decision and report
wording). F4-F7 are small and in the same files; I recommend landing them in
the same resubmission since each is a closed-charset or fail-closed
tightening with a one-test regression. F8 is report/decision accuracy. I am
not asking for EXP-AGENT, a browser wallet or live Ark coverage; the fixture
scope is correctly stated, and the legacy surface is verified unchanged
(`legacy_loopback_behavior_is_unchanged`, baseline bytes for the six
modified files reproduce primary's).

On resubmission, re-freeze the manifest (the report and code hashes will
change), regenerate `EXP-ACCESS-delta-inventory.json`, and attach the fmt,
clippy and test receipts.

---

# Rework re-review (same reviewer, same day)

Date: 2026-11-08 (second pass)
Candidate: same worktree; rework freeze bound by
`/tmp/opencode/EXP-ACCESS-rework-freeze-binding.md` (sha256
`c319a7bd6d07b265b569ac721904dd34bc26b3d4c16c7e91fcb6c5454418a327`)
Disposition: **APPROVE** the bounded fixture foundation exactly as frozen
(code snapshot `8085616e...`), with the merge-handoff conditions in R4.
Everything above this line is the first-pass review, preserved unchanged.

No code, candidate, primary or other-worktree file was edited; no cargo,
Nix or test invocation was started (the build priority ticket belongs to
host wiring); no lock attempt; no commits; no cache changes. This section
is the only artifact written.

## R1. Freeze verification (actual bytes)

Recomputed from the worktree with the manifest recipe:

| Quantity | Freeze | Recomputed |
|---|---|---|
| file records | 589 | 589; every content hash matches; `git ls-files --cached --others --exclude-standard` yields exactly the same regular-file set (0 extra, 0 missing) |
| full snapshot | `b89f74d267f5b3c1875a42b58f69de1327ee129bfe2124b330fac8909e98f7fd` | identical |
| code snapshot | `8085616eaf3f9424ddba6faef50435797ba44f6373d2c7ae8d5cf6280d8cc56b` | identical |
| delta vs 583-record pre-OP baseline | 6 modified, 6 added, 0 removed | identical set to the first pass; `Cargo.toml`, `Cargo.lock`, `crates/spc-lab/Cargo.toml`, flake and config byte-identical to the baseline (no dependency change) |
| baseline originals | `/tmp/opencode/EXP-ACCESS-baseline-sources/crates/spc-lab/src/*` | all six hash-match the baseline manifest; every diff below was taken against them |

The pre-rework packet bytes (`7a7087e0`/`d3a5f078`) are hash-only history;
I did not and cannot byte-diff the old candidate. The rework was judged as
a fresh delta against the baseline, guided by the first-pass findings.

## R2. Gate provenance (receipts read, not re-run)

Receipts `/tmp/opencode/EXP-ACCESS-{clippy-rerun,test-rerun,test-legacy-rerun,test-units-rerun,fmt-final}.log`
hash-match `EXP-ACCESS-receipts-rerun.sha256`; every sibling `.log.exit` is
`0`. Counts read from the logs: access_scope 18 + access_projection 6 =
**24**, legacy 1+6+3+7+3+1+8+4 = **33**, lib **42** + fixture 0, fmt silent
(zero-byte log, exit 0). The clippy log is 13 lines: 11 lines of cargo
registry-cache maintenance warnings (`Permission denied` writing the index
cache and auto-cleaning `~/.cargo/registry/src/.../wayland-scanner-0.29.5`,
owned by another session), then `Checking spc-lab` and `Finished` with no
lint diagnostic. Those are cargo cache warnings, not rustc/clippy warnings;
`-D warnings` governs lint output only, and exit 0 is recorded. I treat
the lint gate as proven; the cache is not this packet's to clean and must
not be deleted to tidy the log.

Provenance of what the receipts tested: every packet code file's mtime
(latest `tests/access_scope.rs` 16:08:01Z) precedes the clippy finish at
16:08:10Z minus the recorded 4.01 s compile, and the code snapshot hash is
stable across freeze passes 1-4 (only the report changed after the gate).
This is an mtime-ordered, hash-stable binding - consistent and I accept it
for a fixture foundation - but it is not a cryptographic proof that the
tested bytes equal the frozen bytes. The three clippy fixes named in the
run history (`needless_borrow` x2, `search_is_some`, `useless_format` x2,
`needless_range_loop`) are all present in the frozen source.

## R3. F1-F8 closure, verified against the source

- **F1 (gate)**: closed. `constant_time_eq` iterates `a.iter().zip(b)`;
  the two `format!("literal")` sites are gone; fmt exit 0 on the frozen
  tree; clippy `-D warnings` exit 0.
- **F2 (artifact leak)**: closed. `handle_expanded`'s
  `("GET", Route::Artifact(path))` arm returns 403 for `is_user()` before
  `serve_artifact` is called (no file I/O); tokenless stays 401 via the
  generic gate; `Catalog` arm unchanged. Test
  `whole_run_artifacts_are_operator_only` runs two principals' commands,
  calls `refresh_artifacts()`, asserts user 403 with a body that contains
  no command id, operator 200 containing the ids, alias/traversal/encoded
  forms all 403 for a user, and the cross-principal `command_id` replay
  still yields the generic denial.
- **F3 (reload gap)**: closed in the stronger form. `AccessState` lost
  `dir_mtime`; `authenticate` calls `load_dir` unconditionally on every
  request and clears all claims on any `Err`. Test
  `in_place_claim_edits_take_effect_on_the_next_request` performs a
  same-size in-place token overwrite (old 401 / new 200, no directory
  entry change), an in-place `worker_ids: []` scope reduction (command
  closed, filtered snapshot empty, no journal line), in-place corruption
  (operator and user both 401 immediately), and repair (both recover).
  Decision item 2 was rewritten to this semantics; the "pinned by test,
  not by scan" wording is gone. Operator SSE rotation while idle is
  additionally proven in `an_operator_stream_reauthenticates_and_closes_on_rotation_or_revocation`.
- **F4 (namespace)**: closed. `derive_user_bindings` rejects
  `operator`/`controller`, the `sha256-` prefix and any claim-filename
  collision (the operator's `operator.json` seeded first) before
  `create_dir`; `load_dir` rejects a `controller` principal, authority
  mixing (`is_operator() != (principal == "operator")`), duplicate
  principals and duplicate tokens (`BTreeSet`). Tests
  `reserved_principals_and_filename_collisions_fail_prepare_closed`
  (code `access_bindings`, `private/access` never created) and
  `forged_claim_files_with_duplicate_authority_fail_closed` (four forged
  files each deny everyone until removed, store recovers).
- **F5 (hygiene)**: closed with one precision note. `load_dir` lstats the
  directory (real dir, `mode & 0o077 == 0`, uid == `/proc/self` uid) and
  every `*.json` entry (regular file, `mode & 0o077 == 0`, same uid);
  `current_uid()` falls back to 0 when `/proc/self` is unreadable, which
  makes a non-root process deny-all - fail closed, correct direction.
  Test covers 0644 file, 0755 dir, symlinked claim, directory-named-like-
  a-claim; the foreign-owner case runs only as root and is explicitly
  skipped otherwise (honest). Precision: the check is `symlink_metadata`
  then `read_to_string` on the path, so a same-uid actor could swap a
  FIFO or symlink in between and the read would follow it or block the
  HTTP worker. A same-uid actor already owns the tokens (0700 directory,
  0600 files), so this is outside the threat the check defends against;
  the report's "never opened" wording should be read as "not opened when
  the lstat fails", which is what the test proves. Non-blocking hardening:
  open with `O_NOFOLLOW | O_NONBLOCK` and `fstat` the descriptor.
- **F6 (operator SSE)**: closed. `serve_events` gained
  `revocation: Option<(&str, &str)>`, checked at loop top, before each
  event and at each heartbeat; the expanded operator path passes
  `(token, principal)`, the legacy call site passes `None` (legacy bytes
  and behavior unchanged). Test proves heartbeat close on in-place
  rotation (bounded under 20 s, zero frames), that the rotated token
  streams, and event-boundary close on deletion with no private event
  delivered.
- **F7 (worker scope)**: closed. `check_command_scope` requires a
  non-empty subset of the claim's `worker_ids` for user
  `PauseWorkers`/`ResumeWorkers`; the empty "whole miner" form is
  operator-only; `SetWorkerBudget` unchanged (own worker). The read
  whitelist is the claim's worker list, not the miner id (unit test
  `user_snapshot_whitelist_is_the_claim_worker_list` excludes a foreign
  worker row on the own miner). The integration test also shows
  `add_worker` is a closed `rejected_precondition` today (control v1 has
  no such operation), so no runtime path enlarges a claim's scope, and
  the rule does not depend on that fact.
- **F8 (proofs/docs)**: closed.
  `preserved_pre_access_journal_round_trips_byte_for_byte` embeds the real
  pre-access journal `/tmp/opencode/LP-hub-run-1/private/commands.jsonl`
  (sha256 `ac67e403...`; I re-hashed the source and byte-compared the two
  string constants to it: identical; the records carry a command id, run
  id, timestamps, the `hub-validation` actor, a stop reason and the
  outcome - nothing secret). It asserts today's kind-only hash equals the
  stored `payload_hash`, a fresh open returns `Existing` with the stored
  outcome and leaves the bytes untouched, `begin_scoped` over it is
  `Denied`, and today's legacy writer reproduces both lines byte-for-byte
  from scratch. `legacy_begin_denies_a_scheme_bearing_entry` covers the
  previously untested `begin()` denial with a no-append assertion. The
  synthetic `frozen_legacy_begin_line_round_trips_byte_for_byte` is
  labeled as format-equality, not capture. `generated_tokens_are_csprng_distinct`
  (64 distinct) and the `.tmp-claim-*` sweep match the report's
  entropy/atomicity wording. The fixture `occurred_at_mono_ms` change is
  reverted (diff against the baseline is the `--snapshot-worker` hook
  only). The report now states the baseline correctly (shared pre-OP
  primary snapshot), the relay `<role>.api` key as test-only pending AP-4
  (runtime publishes `*.api_addr`), `/` as the legacy SPA shell without a
  Bearer entry, the literal-IP Origin rule, and the tokenless runtime
  health probe as an auth-blocked limitation with no public bypass
  introduced. The changelog fragment is now the README's single-bullet
  form.

Two cosmetic report nits, not conditions: the bullet for
`expanded_run_fails_closed_on_crafted_journal_lines` still attributes the
legacy-`begin()` denial to that test (it lives in
`legacy_begin_denies_a_scheme_bearing_entry`), and the privacy-test bullet
still says "token entropy" where the proof is the unit test.

## R4. Merge-handoff conditions (for the serial merge owner, not the candidate)

The packet is a clean delta on the frozen pre-OP baseline, but primary's
`crates/spc-lab` has moved since: `http.rs` (`0ba5102b...`), `engine.rs`
(`06c2c179...`) and `lib.rs` (`7bc62c3d...`) now differ from the baseline
and carry hub routes (`Route::Lab`, `LabJs`, `LabCss`, `Inventory` ->
`engine.lab_inventory()`) plus `src/inventory.rs`. Conditions at merge:

1. **Hub routes under expanded access must be explicitly classified.** As
   written, `handle_expanded` falls to `_ => 404` after the Bearer gate
   for any route it does not name, so a naive merge makes `Inventory`
   unreachable to everyone in expanded runs (closed, not leaking). The
   merge must make `Inventory` operator-only (it is whole-run private
   data) unless a typed user projection exists, and may add `Lab`,
   `LabJs`, `LabCss` to the pre-auth static list only after confirming
   those embedded assets carry no private inline fact, address, token or
   status. Do not resolve this by widening the static exception to a
   wildcard. Add the two negative tests (user 403 on inventory, tokenless
   401) in the integration tree.
2. **Primary's `Cargo.lock` wins.** The candidate's lockfile is the
   baseline's; primary's has advanced. Nothing in this packet changes
   dependencies, so the merge takes primary's lock unchanged.
3. **The OP read-only listener is a separate, approved surface.** This
   approval does not retrofit Bearer authentication onto it and does not
   ask for that; it stays under its own packet.
4. **Future T1 commands are out of scope of this approval.** Any new
   `CommandKind` reachable over HTTP in an expanded run needs an explicit
   `check_command_scope` arm (the default arm denies users, which is the
   right default; the operator path is open by claim).
5. **Decision and changelog splice.** One decision record, one changelog
   fragment, no direct `SPEC.md` edit; splice per `docs/MERGE-QUEUE.md`.
6. **Unchanged constraints.** T0 preflight (`check_traffic_capability`)
   precedes the access binding check in `prepare`; no Ark, epoch, signed
   counter, D or vanilla-captaind-facing file is in the delta.

## R5. Residual notes (non-blocking)

- Per-request full reload also runs on every SSE re-auth; cost is a
  directory listing plus one small read per claim, bounded by the number
  of principals. Fine for the lab; revisit only if principal counts grow.
- `AccessStore::revoke` is now a thin `remove_file` with no HTTP or CLI
  caller; the documented operator channel is the file itself.
- The legacy `CommandLog::open` still skips any unparseable interior line
  as "torn" (pre-existing). The scenario gate (`deny_unknown_fields`)
  keeps old binaries off expanded runs; a later hardening could reject
  interior unparseable lines.

## R6. Verdict

**APPROVE** the EXP-ACCESS fixture foundation as frozen (code
`8085616e...`, full `b89f74d2...`), bounded exactly as the plan states:
claims, Bearer gate, scope, principal-bound replay, filtered reads,
operator-only whole-run surfaces, GET-only relay against a test-declared
upstream. Not covered by this approval: EXP-AGENT browser workflow, live
wallet or Ark facts, real AP-4 role reach, the hub routes (R4.1), the OP
listener (R4.3) and any T1 command (R4.4). No commit or primary
integration was performed by this review.
