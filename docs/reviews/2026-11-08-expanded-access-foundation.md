# EXP-ACCESS foundation: claims, authentication, replay binding, filtered reads

Date: 2026-11-08
Author: TFF implementation agent
Status: rework complete after Fable REQUEST_CHANGES (findings F1-F8), all
dispositions in section 0; requesting re-review by the same reviewer before
integration
Worktree: `/home/ethan/.local/share/opencode/worktree/ad8886/lab-expanded-access`
Base: `b0ab4ca34fda07c61a6335d05f598eaacc69678c`
Bootstrapped from: the shared pre-OP primary snapshot - the primary tree's
own working-tree state before any EXP topic, bound by the manifest
`/tmp/opencode/EXP-OP-baseline-manifest.json` (583 file records, base
`b0ab4ca34fda07c61a6335d05f598eaacc69678c`). This is NOT an OP-worker delta:
the 583 records are the primary's own uncommitted working-tree copies,
re-verified against the primary tree by the reviewer. No primary file was
touched by this packet.
Supersedes: the pre-rework freeze of this same packet (review manifest
sha256 `7a7087e0...`, code snapshot `d3a5f078...`, both preserved verbatim
at `/tmp/opencode/EXP-ACCESS-review-manifest.pre-rework.json` with their
receipts). The pre-rework report text and receipts are unchanged history;
this rework re-freezes a new manifest over the same file set.
This rework: manifest `/tmp/opencode/EXP-ACCESS-review-manifest.json`
(regenerated at re-freeze per T4a: base + all sorted file records, compact
canonical JSON; the code snapshot excludes only this report's path) - code
snapshot
`8085616eaf3f9424ddba6faef50435797ba44f6373d2c7ae8d5cf6280d8cc56b`
(stable across report-text edits by construction; the manifest carries the
full-snapshot hash, which covers this file and therefore cannot be quoted
inside it).
Contract: EXP-ACCESS, plan section 5 of
`/tmp/opencode/role-ui-plan-review-v5/2026-11-07-live-lab-integration-contracts.md`
(acceptance list in section 5.5); decision record
`docs/decisions/2026-11-08-lab-expanded-access-foundation.md`; changelog
fragment `docs/changelog/2026-11-08-lab-expanded-access-foundation.md`
(normalized to the single-bullet fragment format per
`docs/changelog/README.md`)

This packet is an independent implementation topic on top of the shared
pre-OP primary snapshot. Nothing is committed; the tree is the baseline
state plus exactly the files in section 1.

---

## 0. Fable rework dispositions (F1-F8)

Source: `docs/reviews/2026-11-08-expanded-access-foundation-fable.md`
(REQUEST_CHANGES, findings F1-F8). Every disposition is code plus regression
test plus decision-record text; none is a doc-only waiver.

| Finding | Disposition | Regression proof |
|---|---|---|
| F1 fmt/clippy gate | `cargo fmt` over the packet's files; the two `useless_format` sites in `access_scope.rs` fixed (`format!("/api/...")` -> string literals); the `needless_range_loop` in `constant_time_eq` rewritten as an iterator zip; explicit `cargo clippy -p spc-lab --all-targets -- -D warnings` run with exit 0 (no omitted-passing gate) | fmt/clippy receipts at section 3 (final gate all exit 0) |
| F2 artifact cross-principal read | The expanded `Route::Artifact` arm is operator-only, mirroring the catalog: a user token is a closed 403 ("artifacts require the operator claim") BEFORE any file I/O; tokenless 401 stands via the generic gate; the operator keeps the allowed shareable reads; the private-path sanitizer is unchanged and still backs the operator route | `whole_run_artifacts_are_operator_only`: the shareable manifest (carrying both principals' commands) is user-403 with a body that leaks no command id, operator-200 with the real ids; alias-encoded (`%73hareable`), traversal (`..`), encoded-traversal (`%2e%2e`) and empty artifact paths are all user-403; a user replaying bob's stored command id gets the generic denial |
| F3 dir-mtime revocation gap | Removed. Every `authenticate` performs an unconditional, fully validated `load_dir` - no cached authority, no directory-change signal, no mtime tracking (`AccessState.dir_mtime` and the per-match existence check are deleted). In-place token rotation, scope reduction and corruption are observed on the very next request; a failed load clears all claims (fail-closed all-or-nothing, operator included); deletion stays per-principal; repair recovers on the next request | `in_place_claim_edits_take_effect_on_the_next_request`: same-size token overwrite -> old 401 / new 200 with no directory change; worker-scope removal -> command closed + read empty; in-place corrupt JSON -> all-denied immediately, repair -> all recovered |
| F4 reserved ids and duplicate authority | `derive_user_bindings` rejects identity ids equal to `operator`/`controller`, prefixed `sha256-`, or colliding on a claim filename (operator's included) - before any filesystem effect; `load_dir` rejects a `controller`-principal file, `is_operator() != (principal == "operator")` namespace mixing, duplicate principals and duplicate tokens (BTreeSet) - never last-wins. Ids stay standard-typed, never raw paths | `reserved_principals_and_filename_collisions_fail_prepare_closed`: all three reserved ids fail prepare with code `access_bindings` and NO `private/access` directory created; `forged_claim_files_with_duplicate_authority_fail_closed`: forged duplicate-principal, duplicate-token, controller-claim and mixed-authority files each deny every token until removed |
| F5 secret-file hygiene | `load_dir` lstats (`symlink_metadata`, never following) the directory and every `*.json` entry: real directory, regular file, mode `& 0o077 == 0`, uid == process uid (via `/proc/self` metadata); violations are closed errors -> deny-all. Bad files are never chmod'ed into approval; creation-time 0700/0600 owned modes unchanged; error strings carry paths only, never tokens | `claim_files_with_bad_mode_type_or_owner_fail_closed`: file 0644, dir 0755, symlinked claim, directory-named-like-a-claim each deny all until restored (bounded: a symlink/fifo is lstat'ed, never opened); foreign-owner case exercised only when the test runs as root and explicitly omitted otherwise |
| F6 operator SSE revocation | `serve_events` gained a `revocation: Option<(token, principal)>` guard: the expanded operator stream re-authenticates at loop top, before every event and at every idle heartbeat - exactly like the filtered user stream; a deleted, rotated or re-scoped operator claim closes the open stream at the next defined boundary with nothing further delivered. The legacy call site passes `None` (no credential exists; path unchanged) | `an_operator_stream_reauthenticates_and_closes_on_rotation_or_revocation`: in-place token rotation closes the old-token stream at the heartbeat (no directory change), the rotated token still opens a fresh working stream, deletion closes at the next event boundary with no further private event; legacy-off SSE unchanged is pinned in `legacy_loopback_behavior_is_unchanged` |
| F7 user worker scope | A user's `PauseWorkers`/`ResumeWorkers` must name a non-empty subset of the claim's own worker ids; an empty list ("the whole miner") is operator-only. The rule deliberately does not lean on the prepare-time one-identity-per-miner invariant. The read whitelist stays the claim's worker list: a worker entity merely matching the claim's miner id never enters the filtered snapshot | `user_pause_resume_require_explicit_own_workers`: user empty list and foreign-worker list are closed scope failures with no journal effect; own explicit ids succeed; operator empty list still pauses the whole miner; control v1's closed `add_worker` outcome shown (no runtime path can invalidate the invariant today, and the rule does not depend on it); `user_snapshot_whitelist_is_the_claim_worker_list` (unit): a foreign worker row on the claim's own miner is excluded from the projection |
| F8 precise proofs/docs | The REAL preserved pre-access journal of a finished live hub run (`/tmp/opencode/LP-hub-run-1/private/commands.jsonl`, sha256 `ac67e403...`, read-only source, exactly one StopRun begin/finish pair, only public lab facts - no token/credential/address/identity payload) is copied verbatim into the owned fixture test: today's kind-only payload hash equals the preserved baseline hash, a fresh open replays it to `Existing`, an authenticated `begin_scoped` over it is `Denied`, and today's legacy writer reproduces the journal bytes byte-for-byte from scratch (`preserved_pre_access_journal_round_trips_byte_for_byte`); a synthetic deterministic format-equality case is kept but explicitly labeled non-capture (`frozen_legacy_begin_line_round_trips_byte_for_byte`); a direct legacy `begin()` on a scheme-bearing entry is `Denied` with no journal write (`legacy_begin_denies_a_scheme_bearing_entry`); 64 distinct CSPRNG tokens asserted (`generated_tokens_are_csprng_distinct`) and no `.tmp-claim-*` leftovers anywhere in the run tree (extended privacy test) - the entropy/atomicity claims now match what is proven; the fixture `occurred_at_mono_ms` 0->7 default change is reverted (the `--snapshot-worker` hook stays); the relay `<role>.api` mapping is documented as test-only (AP-4 dependency, no production relay reach claimed); the expanded root `/` is documented as the reserved legacy SPA shell, not a functional workbench; the Origin literal-IP limitation is documented; the tokenless runtime health probe is documented as a known auth-blocked limitation (no Bearer-capable existing hook; no public health-auth bypass introduced); this report's baseline wording is corrected to the shared pre-OP primary snapshot | the preserved-capture, format-equality and scheme-denial tests above; the unit test above; the extended privacy test; decision record items 1-6 and section 5 below |

## 1. What was built

The spc-lab controller gained an identity-scoped access boundary for
`expanded_access: true` scenarios: run-local per-principal claim files,
Bearer authentication on every expanded route with a closed static
bootstrap exception, principal-bound command scope evaluated before any
journal effect, a persisted command-log hash-scheme discriminator that
binds replay idempotence to the principal, run and full payload, and a
server-side filtered read projection (snapshot, SSE replay/live/resync,
miner-poll DTO). Legacy scenarios are byte-for-byte unchanged.

| Path | Kind | Purpose |
|---|---|---|
| `crates/spc-lab/src/access.rs` | new | Claims store (0700/0600 atomic writes, CSPRNG 32-byte hex tokens, `sha256-<hex>.json` for unsafe principal ids, unconditional fully validated reload per authentication with hygiene checks on owner/mode/type and duplicate-authority rejection, deletion- and rewrite-based revocation, fail-closed all-or-nothing reload), `derive_user_bindings` (identity to exactly one miner, >= 1 worker, exactly one pool per miner, reserved-principal/filename-namespace enforcement - validated at prepare before any filesystem/lock effect), `check_command_scope` (non-empty explicit worker subset for user pause/resume), closed-rejection and `me_view` builders, `filter_event`/`user_snapshot` (filter before serialization; the claim's worker list is the whitelist), the relay closed asset map, the unavailable-kind list. Six unit tests inside the module. |
| `crates/spc-lab/src/scenario.rs` | edited (serial hunk) | The additive optional `expanded_access: bool` scenario field (absent = legacy behavior unchanged). |
| `crates/spc-lab/src/command.rs` | edited (serial hunk) | `LogLine::Begin` gains optional `hash_scheme`/`principal` (`serde(default)`, `skip_serializing_if` - legacy lines serialize byte-identically, pinned against a frozen baseline literal); `HASH_SCHEME_KIND_ONLY = "kind_only_v1"` / `HASH_SCHEME_EXPANDED = "expanded_v1"`; `expanded_payload_hash` = sha256 over scheme tag, principal, run id, full kind; `BeginOutcome::Denied(CommandOutcome)` with the generic detail `command id is not available for this principal` (no journal write, no cross-principal outcome leak); `begin_scoped(cmd, principal)`; the legacy `begin()` denies scheme-bearing entries (an old runner never adopts an expanded entry); unknown/missing discriminator on a scheme-bearing line fails closed. |
| `crates/spc-lab/src/engine.rs` | edited (serial hunk) | `submit()` wraps `submit_as(cmd, principal)`; `submit_authenticated(&AuthContext, cmd)` runs `check_command_scope` then `submit_as`; `expanded_access()`, `access()`, `authenticate_bearer()`, `user_snapshot()`, `user_projection()` (single-miner v1 poll, null + reason on failure - never zeros), `user_projections()` (operator array), `relay_target(role)` via `addresses["<role>.api"]`; claim generation at prepare for expanded scenarios. |
| `crates/spc-lab/src/http.rs` | edited (serial hunk) | `handle_expanded` takes over when `engine.access().is_some()`: exactly-one-Bearer gate (constant-time compare), closed unauthenticated static list (workbench HTML/JS/CSS, favicon, relay static assets), Me/Projection routes, operator-only Catalog and Artifact routes (user 403 before any file I/O), same-origin browser-effect check, CSP on expanded HTML, GET-only role relay over declared topology, filtered SSE and re-authenticating operator SSE (both re-auth at every event and heartbeat, close on revocation/rotation; stale cursor yields a filtered resync event). |
| `crates/spc-lab/src/lib.rs` | edited (serial hunk) | One line: `pub mod access;`. |
| `crates/spc-lab/src/bin/fixture.rs` | edited (test-infrastructure hunk) | Additive repeatable `--snapshot-worker <id>`: the fixture miner's v1 snapshot returns real worker rows (requested 100000 hps, achieved 4242, `active_job_id "job-fixture-1"`). Test-only; nothing in production keys off it. The fixture's default event shape (including `occurred_at_mono_ms`) is unchanged from the baseline. |
| `crates/spc-lab/tests/access_scope.rs` | new | Eighteen integration tests (section 3), own request/bearer/SSE helpers, `tests/common/mod.rs` untouched. |
| `crates/spc-lab/tests/access_projection.rs` | new | Six integration tests (section 3). |
| `docs/decisions/2026-11-08-lab-expanded-access-foundation.md` | new | The decision record (updated in this rework: reload semantics, namespace reservation, worker scope, operator SSE, artifacts, relay/root/Origin/probe limitations). |
| `docs/changelog/2026-11-08-lab-expanded-access-foundation.md` | new | The SPEC changelog fragment, normalized to the exact single-bullet frontmatter line per `docs/changelog/README.md` (merge-owner splice). |
| `docs/reviews/2026-11-08-expanded-access-foundation.md` | new | This report. |

No root `Cargo.toml`, `Cargo.lock`, flake or config change. No dependency
added. No OP-worker, hub, static-gallery, miner-protocol, signer, T1 or
Ark file touched. No pin bumped.

## 2. File inventory (baseline -> this packet)

sha256 over the worktree files, verified against the frozen baseline
manifest (`/tmp/opencode/EXP-OP-baseline-manifest.json`, 583 records, base
`b0ab4ca34fda07c61a6335d05f598eaacc69678c`): 577 records re-hash
identically, 0 missing, 0 unexpected files anywhere in the tree (589
snapshot files total), 6 modified + 6 added - exactly the packet below:

| Path | Baseline | This packet |
|---|---|---|
| `crates/spc-lab/src/access.rs` | (absent) | `9b7c820ecee136419a2af6319141535af5449c4309356ec8fa56a605a1c46652` |
| `crates/spc-lab/src/bin/fixture.rs` | `b3baec02689f4a634fe5eeda8d150597e67e36561cfc5ffeea6eadfabc1b10f7` | `e9c021fed248c07ccf9b051fc2174f760c486f95600e7b2467cde765f2c22834` |
| `crates/spc-lab/src/command.rs` | `62d0589945bc8f563c9ab8ba644275e62986e66b0323bd8f28da35a0154e7ec8` | `4596511f645f1d0abc0fe6b6eb4c1fc4e86df13efc6bd7803a23161dd9754db6` |
| `crates/spc-lab/src/engine.rs` | `bf221dd393e5171577872fbd5800deb0bec9b5ac0267ff4206cd32afb20842fa` | `35eff59707192bb57ca82aa1930723d58fc4a1239835635fb51cb28dcb2a061c` |
| `crates/spc-lab/src/http.rs` | `a8642ac5da57db5ef53cbf2ac1c3b1a710e41f1a36a02c75842c1b86111b7ea7` | `d0114c7e7db5316f24ab225a2412abf68683502ec8340c73f565d017ee8b319a` |
| `crates/spc-lab/src/lib.rs` | `712ee4d0d36f86d2ddf16042cdc67a0f306e1c2f39fa1ba20cd7a40cec97edcc` | `26b2c7e4d247ae5a45b2cbb29a33202cb555082d0c95c9c53324feffd4a43c40` |
| `crates/spc-lab/src/scenario.rs` | `b1fec4af4bdbcb0229f55222e0a26bbef534394eb5075fb9d8e710deebf7379b` | `ed697277daf2a0a3976350f182bbac5cac293ff5ed991516686cac4ed45c582f` |
| `crates/spc-lab/tests/access_projection.rs` | (absent) | `dc9fd3b6d474bf1ead154733250a06de4a12486e2bd84d2883403e5e025f17b2` |
| `crates/spc-lab/tests/access_scope.rs` | (absent) | `c9c58e0338c67a71c95d705a575de5788bf34a79210084a3bd35201dea5d3428` |
| `docs/changelog/2026-11-08-lab-expanded-access-foundation.md` | (absent) | `5a06393a88b1a5aae1e2a82e8c63bfccf6e88b257af0ed8a36d610e673df9616` |
| `docs/decisions/2026-11-08-lab-expanded-access-foundation.md` | (absent) | `70692d39fb43bf50b7cc278b392f8140f3be2217cf0d51281b7d45d2732122f3` |
| `docs/reviews/2026-11-08-expanded-access-foundation.md` | (absent) | (this report; covered by the full-snapshot hash in the manifest) |

This report's own hash is carried by the review manifest (the recipe's
code snapshot excludes exactly this path; its hash is quoted in the
header). The verification delta and every current hash are reproducible
from `/tmp/opencode/EXP-ACCESS-delta-inventory.json`; the full-tree
verification re-hashes all 583 baseline records against the tree and walks
for additions excluding `.git` and `target`.

## 3. Verification

All runs under the shared heavy-build lock
(`flock -n -o /tmp/opencode/silent-pool-heavy-build.lock`), target dir
`/tmp/opencode/EXP-ACCESS-target`, `CARGO_TARGET_DIR` reused across runs.
Final gate (2026-11-08T16:08Z, third run): every step exit 0.

- `cargo fmt -p spc-lab` (packet files formatted; `-- --check` clean;
  receipts `/tmp/opencode/EXP-ACCESS-fmt-rerun.log` and
  `/tmp/opencode/EXP-ACCESS-fmt-final.log`): **exit 0**.
- `cargo clippy -p spc-lab --all-targets -- -D warnings` (explicit
  deny-warnings gate, no omitted pass; receipt
  `/tmp/opencode/EXP-ACCESS-clippy-rerun.log`): **exit 0**.
  Receipt-log disposition: the first lines of that log are cargo's
  startup registry-cache maintenance warnings - a failed write to the
  crates.io index cache and a failed global-cache auto-clean of
  `wayland-scanner-0.29.5/Cargo.toml`, both `Permission denied` because
  parts of `~/.cargo/registry` are owned by another session. These are
  benign cache-maintenance failures at startup, not lint results: they
  are followed by `Checking spc-lab ...` and
  `Finished dev profile ... in 4.01s`, the log contains no lint
  diagnostic lines, and the recorded exit is 0. The whole log is
  preserved unedited; the cache was not touched or cleaned (not this
  session's to clean).
- `cargo test -p spc-lab --test access_scope --test access_projection`:
  **24 tests pass, 0 failed** - access_scope 18 (3.32 s), access_projection
  6 (21.05 s, includes the two SSE revocation-boundary tests) (receipt
  `/tmp/opencode/EXP-ACCESS-test-rerun.log`).
- Legacy regression suites: `command_concurrency` (1), `command_idempotency`
  (6), `engine_restart` (3), `http_protocol` (7), `http_sse` (3),
  `http_subscriber_lag` (1), `scenario_v1` (8), `snapshot_resume` (4):
  **33 tests pass, 0 failed** (receipt
  `/tmp/opencode/EXP-ACCESS-test-legacy-rerun.log`).
- `cargo test -p spc-lab --lib --bins`: **42 unit tests pass, 0 failed**
  (lib 42, fixture bin 0; receipt
  `/tmp/opencode/EXP-ACCESS-test-units-rerun.log`), including the six
  in-module access unit tests:
  `hostile_principal_ids_never_become_raw_paths`,
  `token_compare_and_decode_are_strict`,
  `generated_tokens_are_csprng_distinct`,
  `user_snapshot_whitelist_is_the_claim_worker_list`,
  `relay_asset_list_is_closed`,
  `unavailable_kinds_are_the_closed_t0_list`.
- Zero leftover fixture processes after the green runs (checked with
  `pgrep`; every started engine is stopped by its test; the build lock is
  held with `flock -o` so no child can inherit it).

Failure history (all three gate runs under the same lock, receipts
overwritten by the final green run; the two failing runs' lints are listed
here in full):

- Run 1 (16:04Z): clippy exit 101 - two `needless_borrow` in `http.rs`
  (`&engine` at the operator-SSE and artifact call sites, where `engine`
  is already `&Arc<RunEngine>`); access-suites exit 101 - E0277 on the
  root-only chown literals (`Some(1.into())` inferred `i32`). Fixed:
  `engine` passed directly, `Some(1u32)`/`Some(0u32)`. Legacy 33, units
  42 and fmt were already green.
- Run 2 (16:06Z): clippy exit 101 - one `search_is_some` in
  `access_scope.rs` (a pre-rework `find(&alice).is_none()` assertion that
  only surfaces now that the `-D warnings` gate actually runs). Fixed:
  `!me.text().contains(&alice)`. Access 24, legacy 33, units 42, fmt
  green.
- Run 3 (16:08Z): **all green** - clippy 0, access 24, legacy 33, units
  42, fmt 0, no fixture processes.

The pre-rework runs (14 access + 33 legacy + 40 unit tests, all green) are
preserved in `/tmp/opencode/EXP-ACCESS-test-run5.log`,
`/tmp/opencode/EXP-ACCESS-test-legacy.log` and
`/tmp/opencode/EXP-ACCESS-test-units.log`; they covered the pre-rework
tree and are history only - the rework counts above are the new 24/33/42
including the preserved pre-access capture fixture.

`tests/access_scope.rs` (18):

- `expanded_access_requires_bearer_for_every_private_route` - tokenless
  command/snapshot/projection/SSE are 401 with no journal entry; the
  closed static bootstrap list loads without a token and contains no
  private fact or token; exactly-one-Bearer is enforced (zero or two
  headers reject).
- `user_scope_forged_ids_and_unavailable_kinds_are_closed_before_journal` -
  a user token issuing `StopRun` is a closed outcome with no journal
  effect; forged actor and cross-user miner/worker ids reject before any
  effect; `send_user_payment`/`bootstrap_fund` answer as unavailable
  facts through the real outcome surface; the operator credential gets
  the real outcome.
- `replay_binding_is_per_principal_and_survives_restart` - same principal
  + payload replays idempotently; a changed in-scope payload under a known
  `command_id` is `rejected_conflict`; a cross-principal `command_id`
  fails closed without revealing that an outcome exists; the checks
  survive a journal restart.
- `legacy_journal_stays_byte_identical_and_replays_unchanged` - a legacy
  journal line is byte-identical before and after the scheme change, and
  a pre-change entry replays to its stored outcome (never a conflict).
- `expanded_run_fails_closed_on_crafted_journal_lines` - a scheme-bearing
  line with an unknown discriminator, and a legacy `begin()` facing a
  scheme-bearing entry, fail closed.
- `claim_files_are_private_atomic_and_never_escape_the_access_dir` - dir
  0700, files 0600, token entropy, atomic rename (no partial file, and no
  `.tmp-claim-*` leftover anywhere in the run tree), no other file under
  the run tree contains a token, hostile principal ids hash into
  filenames.
- `revocation_closes_access_without_touching_other_principals` - deleting
  bob's claim is immediate and per-principal (alice and operator keep
  working).
- `relay_is_a_closed_get_only_declared_role_surface` - a real local TCP
  upstream stands in for the declared role listener: cross-origin POST
  403, relay POST 405, forged role/path 404, declared asset passes with
  the closed content-type map, upstream failure is the relay's own closed
  error with no body detail, token appears in no page source, URL or log.
- `legacy_loopback_behavior_is_unchanged` - a legacy scenario keeps
  unauthenticated loopback behavior exactly (no access dir, no Bearer
  requirement, SSE opens and streams unauthenticated).
- `in_place_claim_edits_take_effect_on_the_next_request` (F3) - same-size
  token rotation: old 401 / new 200 on the next request with no directory
  change; worker-scope removal closes the command and empties the read;
  in-place corrupt JSON denies every token immediately (operator
  included); repair recovers the store.
- `claim_files_with_bad_mode_type_or_owner_fail_closed` (F5) - a 0644
  claim file, a 0755 access directory, a symlinked claim and a
  directory-named-like-a-claim each deny every token until restored (never
  silently chmod'ed back); the foreign-owner case runs only as root and is
  explicitly omitted otherwise.
- `reserved_principals_and_filename_collisions_fail_prepare_closed` (F4) -
  identity ids `operator`, `controller` and `sha256-deadbeef` each fail
  prepare with code `access_bindings` and NO access directory created.
- `forged_claim_files_with_duplicate_authority_fail_closed` (F4) - forged
  files with a duplicate principal, a duplicate token, the reserved
  controller principal, or user-principal operator authority each deny
  every token until removed; the store recovers.
- `whole_run_artifacts_are_operator_only` (F2) - the shareable manifest
  carrying both principals' commands: user 403 before file I/O with no
  command-id leak, operator 200 with the ids; alias-encoded, traversal,
  encoded-traversal and empty artifact paths are user-403; a user
  replaying another principal's command id gets the generic denial.
- `user_pause_resume_require_explicit_own_workers` (F7) - a user's empty
  worker list and foreign worker list are closed scope failures with no
  journal effect; own explicit ids succeed; the operator's empty list
  still pauses the whole miner; control v1's `add_worker` stays closed.
- `frozen_legacy_begin_line_round_trips_byte_for_byte` (F8) - today's
  legacy writer emits the exact frozen baseline begin line byte-for-byte;
  a journal holding only that line round-trips a fresh open, an in-flight
  begin, a finish, and then `Existing`.
- `legacy_begin_denies_a_scheme_bearing_entry` (F8) - a fresh open over a
  scheme-bearing journal denies a legacy `begin()` with the generic
  detail and no journal write.
- `preserved_pre_access_journal_round_trips_byte_for_byte` (F8) - the real
  preserved pre-access hub-run journal (origin
  `/tmp/opencode/LP-hub-run-1/private/commands.jsonl`, sha256 `ac67e403...`,
  read-only, non-secret StopRun begin/finish pair only): today's kind-only
  payload hash equals the preserved baseline hash; a fresh open replays the
  captured command to `Existing` with the stored outcome and no byte
  rewritten; an authenticated `begin_scoped` over the legacy entry is
  `Denied`; and today's legacy writer reproduces the preserved journal
  bytes byte-for-byte from scratch.

`tests/access_projection.rs` (6):

- `user_snapshot_is_a_filtered_projection_with_honest_watermark_and_gaps` -
  bob's snapshot carries only his own worker row and allowlisted
  aggregates, the global watermark, explicit gap facts and
  wallet/payment not-available facts; zero alice ids anywhere; the
  operator view is unfiltered; a revoked token is 401.
- `user_sse_filters_replay_and_live_events_and_honors_last_event_id` -
  replay and live events filter per principal (bob sees no alice
  block/tx/amount event), multiline and chunked frames parse, tokenless
  401, bad cursor 400, `Last-Event-ID` resume works, revoked 401.
- `a_stale_cursor_gets_a_filtered_resync_snapshot_event` - schema
  `spc-user-snapshot/v1`, `filtered: true`, `resync_required: true`, no
  raw event data.
- `a_revoked_stream_closes_at_the_next_boundary_or_heartbeat` - an
  already-open USER stream delivers nothing after revocation and closes
  at the next event boundary or the idle heartbeat (~15 s, bounded under
  20 s).
- `an_operator_stream_reauthenticates_and_closes_on_rotation_or_revocation`
  (F6) - an open OPERATOR stream closes at the idle heartbeat after an
  in-place token rotation (no directory change), the rotated token still
  opens a fresh working stream, and deletion closes the stream at the
  next event boundary with no further private event.
- `miner_poll_projection_serves_real_rows_or_nulls_with_reasons_never_zeros` -
  bob gets a real row through the fixture `--snapshot-worker w-bob-1`
  hook (requested 100000, achieved 4242, `job-fixture-1`); a missing
  worker row is `null` with the reason `worker row is missing from the
  miner snapshot`; the operator gets the unfiltered array of two; an
  unstarted engine yields `null` with `miner control pipe is unavailable
  (not started, adopted or stopped)` - never zeros.

## 4. Negative proofs (fail-closed inventory)

- **Authentication**: zero, two or malformed Bearer headers reject; an
  unknown or revoked token is 401 BEFORE any journal entry, stored-outcome
  lookup or effect; constant-time compare over the decoded 32 bytes; a
  loopback source address is never operator authority (the operator is
  exactly the operator claim).
- **Scope**: a user token on an operator kind, a forged payload actor, a
  cross-user miner/worker id, a user's empty (whole-miner) worker list -
  all closed outcomes with no journal effect; the closed-rejection detail
  never reveals whether another principal holds an outcome for the id.
- **Replay**: idempotence is keyed by principal, run and full kind
  payload; a changed payload is `rejected_conflict`; a cross-principal
  `command_id` fails closed; a legacy entry never becomes an expanded
  outcome and an expanded entry is never adopted by the legacy `begin()`;
  an unknown or missing scheme discriminator on a scheme-bearing line
  fails closed; the legacy line format itself is pinned byte-for-byte
  against the frozen baseline literal.
- **Claims storage**: hostile principal ids never form raw path
  components (closed charset or `sha256-<hex>.json`); reserved
  principals/prefixes and filename collisions reject at prepare before
  any directory exists; atomic temp-plus-rename with no `.tmp-claim-*`
  leftovers; 0600/0700 process-owned; every authentication re-validates
  the whole directory (owner/mode/type hygiene, no symlinks, no
  duplicates, namespace consistency) and a failed load clears ALL claims
  (operator included) until repair - never last-wins, never a silently
  chmod'ed file.
- **Reads**: another principal's miner/worker/identity id, block hash,
  txid or amount never enters a serialized user response (checked on
  snapshot, replay, resync and after revocation); the claim's worker list
  - not a miner-id match - is the read whitelist; whole-run artifacts and
  the catalog are operator-only (user 403 before any file I/O); gaps stay
  explicit gap/watermark facts; a failed poll is null-with-reason, never
  zeros; the static exception is a closed list (an unknown path is a
  closed 404/403, never a wildcard); the artifact sanitizer's `private/`
  rejection is unchanged.
- **Streams**: every expanded SSE stream - the operator's included -
  re-authenticates before every event and at every idle heartbeat; a
  deleted, rotated or re-scoped claim closes the open stream at the next
  defined boundary with nothing further delivered; the legacy
  (expanded-off) stream passes no credential and is unchanged.
- **Relay**: GET-only, declared-topology listeners only (never a
  browser-supplied target), closed asset-to-`{status, content_type}` map,
  bounded body and timeout, upstream failure becomes the relay's own
  closed error with no body detail, no CORS header anywhere.

## 5. Not claimed (live gates that remain)

- **No live Ark or wallet fact.** The user projection surfaces
  wallet/payment facts only as not-available until the AP-4/T1 projection
  interfaces exist; no Ark codec, signer or claim is exercised. The
  claims/scoping code itself has no Ark dependency.
- **No live agent workbench UI.** The browser workflow (bootstrap page,
  sessionStorage keying, the interactive workbench) is EXP-AGENT scope;
  this packet provides only the server-side boundary it will sit on (the
  closed static list, the relay, the filtered routes and the projection).
  No page in this packet carries a token or private fact. The expanded
  root `/` serves the legacy SPA shell without a Bearer entry point - a
  reserved static surface, not a functional workbench.
- **No cross-namespace live proof.** The relay is proven against a real
  local TCP upstream standing in for the declared listener; a real
  namespace-mounted role daemon reachability check is QA/AP-4 scope. The
  relay's `<role>.api` target is currently a TEST-ONLY mapping (the
  runtime populates `*.api_addr` for fixture roles); no production
  namespace relay reach is claimed. The agent role's endpoint mapping is
  AP-4's; until it exists the agent workbench is host-mode only.
- **Token distribution is manual.** The 0600 claim file is the
  distribution channel (an operator reads it); no token is ever placed in
  a URL, log, shareable artifact or the hub feed.
- **Known auth-blocked probe limitation.** A tokenless runtime health
  probe reports the expanded run as unreachable (every private route is
  401 by design). No public health-auth bypass is introduced; the
  existing `namespace::http_request` helper supports Basic auth only, so
  wiring an operator credential would expand this packet's shared-file
  delta. Handed off as a documented limitation.
- **Origin check is literal-IP only.** Same-origin enforcement matches the
  serving listener's literal authority; hostname-based origins are not
  supported (documented in the decision record).

## 6. Merge instructions for the serial owner

Owned outright (apply whole, no conflicts expected): `access.rs`,
`tests/access_scope.rs`, `tests/access_projection.rs`, the decision record
and the changelog fragment.

`crates/spc-lab` serial hunks (apply as focused hunks if the files moved;
none of them are edited beyond the shared baseline):

1. `scenario.rs`: the additive optional `expanded_access: bool` field.
2. `command.rs`: the `Begin` optional `hash_scheme`/`principal` fields,
   the two scheme constants, `expanded_payload_hash`,
   `BeginOutcome::Denied`, `begin_scoped`, and the legacy `begin()`
   scheme-bearing rejection.
3. `engine.rs`: claim generation at prepare (expanded scenarios only),
   `submit`/`submit_as`/`submit_authenticated`, `expanded_access`,
   `access`, `authenticate_bearer`, `user_snapshot`, `user_projection`,
   `user_projections`, `relay_target`.
4. `http.rs`: `handle_expanded` and the expanded route set (Me,
   Projection, relay), the auth gate, operator-only Catalog/Artifact
   routes, same-origin check, CSP, the filtered SSE path and the
   re-authenticating operator SSE path.
5. `lib.rs`: one line, `pub mod access;`.
6. `bin/fixture.rs`: the additive repeatable `--snapshot-worker <id>`
   flag (test infrastructure only).

The changelog fragment `docs/changelog/2026-11-08-lab-expanded-access-foundation.md`
is spliced by the merge session under `changelog:` per
`docs/MERGE-QUEUE.md`; this packet does not edit `docs/SPEC.md`.

## 7. Everything is uncommitted

Per the packet contract, nothing is committed and nothing outside section
1's file list was modified. The frozen baseline manifest
(`/tmp/opencode/EXP-OP-baseline-manifest.json`) is untouched. The
pre-rework freeze of this packet (manifest `7a7087e0...`, code snapshot
`d3a5f078...`) is preserved verbatim at
`/tmp/opencode/EXP-ACCESS-review-manifest.pre-rework.json` with its
receipts. This rework's review manifest is
`/tmp/opencode/EXP-ACCESS-review-manifest.json` (code snapshot
`8085616eaf3f9424ddba6faef50435797ba44f6373d2c7ae8d5cf6280d8cc56b`); the
delta inventory is `/tmp/opencode/EXP-ACCESS-delta-inventory.json`; the
rework receipts are the `-rerun` logs named in section 3 plus
`/tmp/opencode/EXP-ACCESS-receipts-rerun.sha256`. This report requests
re-review by the same Fable reviewer before any integration into primary;
on approval, hand off to the serial merge owner per section 6.
