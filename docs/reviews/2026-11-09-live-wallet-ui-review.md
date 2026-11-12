# Fable code audit: live wallet UI on the EXP-ACCESS foundation

Date: 2026-11-10 (audit of the 2026-11-09 final r2 freeze)
Reviewer: Fable (concrete-code review subagent)
Candidate: `/home/ethan/.local/share/opencode/worktree/ad8886/lab-live-wallet-ui`
Candidate report: `docs/reviews/2026-11-09-live-wallet-ui-implementation.md`
(in the candidate worktree)
Freeze: `/tmp/opencode/live-wallet-ui-final-r2-manifest.json` (693 rows),
full `19aa61a04672b63a1cd928fedda299d8e78d93a99f29397572eefcbc206ee0c6`,
code (excludes only the report)
`0f559b5d8cd4c423d08e787944e5d3a87af821cd429d1957a61c5564d8a0d4f1`,
baseline snapshot `4561a2df...` (ladder integration primary baseline),
patch `/tmp/opencode/live-wallet-ui-final-r2-baseline.patch` sha256
`e979a21848de13a0de484887bcea3b88f6431fd25fff022e26d52d69abd5c4e5`.
Disposition: **REQUEST_CHANGES**, bounded to one concrete replay-state bug
(W1) with a five-line fix and one test; everything else in this audit is
either verified sound or a non-blocking note. The audit scope is the UI
packet's code; no funded Ark/AP-3/T2 proof is asked for or claimed.

Nothing in the candidate, primary source, or any other worktree was edited;
no cargo, Nix, node or browser run was started; no lock was touched; no
commits; no cache or garbage-collection action. This file is the only
artifact written. The inspection processes and data named by the parent
(1441192 / 199214) were not touched.

## 1. Freeze verification (actual bytes)

Recomputed from the worktree with the manifest recipe: 693 records, every
content hash matches the tree, full and code snapshot hashes reproduce
exactly. Delta against the baseline: 8 Rust files (5 modified, 3 new), 11
static/JS files (7 modified, 4 new test/harness files), decision, changelog
fragment, report. `Cargo.toml`, `Cargo.lock`, flake and pins are
baseline-identical (no dependency change; `rustix`, `bitcoin`, `sha2`,
`hex` were already dependencies of `spc-lab`). The baseline files used for
every diff below are the immutable copies under
`/tmp/opencode/ladder-integration-baseline-src`.

## 2. Receipts read (not re-run)

- `/tmp/opencode/live-wallet-ui-rust-r8/`: `clippy.exit`, `build.exit`,
  `browser-build.exit`, `wallet-scope-recheck.exit` all `0`; the clippy log
  ends in `Finished` with only the pre-existing `sqlx-postgres`
  future-incompat note (not a lint); the wallet recheck shows `6 passed`;
  `clippy-before.json == clippy-after.json` (source unchanged across the
  step). r3/r4 test logs sum to 66 `passed` lines, consistent with the
  report's 64 distinct after the two suites re-run in r4.
- `/tmp/opencode/live-wallet-ui-compiled-browser-r1/proof.json`: 13 checks,
  binaries bound by sha256 (`spc-lab` `363dbb19...`, `wallet_browser_server`
  `15d69bdd...`, fixture `bae1a8d8...`), 77 recorded requests, zero
  exceptions and zero loading failures, zero owned process survivors, both
  fixture listeners closed. I scanned every recorded request URL: the only
  query strings are `events?after=<n>`; no token appears in any URL. The
  proof's own scope line states the wallet/node/miner providers are
  synthetic fixtures; this audit treats it as a transport/UI proof, not a
  funded one.
- The compatibility artifacts and the provider interface
  (`/tmp/opencode/wallet-ui-provider-interface.json`) were read, not
  re-executed.

Provenance caveat: the r8 receipts bind source by their own
before/after recipe, which differs from the final r2 manifest recipe; the
parent attests the binary-bound sources are unchanged and the manifest's
`binary_bound_sources_unchanged: true` records that. I accept it for a UI
audit and did not rebuild.

## 3. What holds (verified against the code, not the labels)

**Browser session (`bins/spc-lab/static/access.js`).**
- Token lives only in memory and `sessionStorage` keys
  `spc-lab-token:<run>:<principal>` plus a `spc-lab-session` pointer; no
  `localStorage`, cookie, URL, fragment, body or log write anywhere in the
  three scripts (grepped). `request()` only accepts paths beginning
  `/api/lab/v1/` without `\` or `#`, and every fetch uses
  `redirect: "error"`, `credentials: "omit"`, `cache: "no-store"` -
  same-origin by construction, redirects refused.
- `clear()` bumps the epoch, aborts every live `AbortController`, wipes
  all `spc-lab-token:*`, `spc-lab-pending:*` and the session pointer, and
  blanks the private DOM synchronously. `request()`/`json()`/`stream()`
  re-check the epoch after the response and after the body; the SSE parser
  callback is epoch-gated; `mountAccess` binds each login/restore attempt
  to its epoch so a cancelled older discovery cannot clear a newer login
  (the race the author found and tested). `pagehide` aborts and hides;
  `pageshow(persisted)` re-authenticates.
- Discovery is the existing authenticated `/api/lab/v1/health` then
  run-scoped `/me`; in the expanded profile a tokenless health is 401,
  which `request()` turns into `clear()` plus the login form. There is no
  404 fallback path and no new unauthenticated data route. The legacy
  (access-off) context is synthesized only when health answers 200 without
  a token, which is exactly the legacy profile.
- Pending command envelope: stored per run/principal, restored only when
  `run_id` and `actor` match the authenticated context, retried with the
  same `command_id`, cleared by logout/switch. Unknown/running outcomes
  never mint a new id.
- SSE: incremental WHATWG framing with CR/LF/CRLF, streaming `TextDecoder`,
  comments, multiline `data`, persistent `id`, `Last-Event-ID` header, a
  1 MiB frame bound, `event: resync` handled as authoritative.
- Rendering: every DTO string in the wallet, worker and auth areas is set
  via `textContent` or `createElement`; the pre-existing operator tables
  use `escapeHtml` on every interpolated field. Money goes through
  `BigInt` on canonical decimal strings; non-canonical or missing values
  render `unknown`, never `0`.

**Controller (`crates/spc-lab/src/http.rs`, `engine.rs`, `access.rs`).**
- `/access.js` is an exact new public route (both legacy and expanded
  branches), embedded with `include_str!`; no other new pre-auth route. The
  Hub `Lab/LabJs/LabCss` static and the operator-only `Inventory` from the
  EXP-ACCESS R4 conditions are already present in this baseline.
- `/wallets` is behind the Bearer gate; a user without `view_own_status`
  is 403; `WalletRegistry::scoped(claim)` filters by identity AND pool
  before `view()` performs any status socket or file I/O; the operator
  explicitly gets all bindings. Unbound registry answers
  `state: "not_configured"` with an empty list.
- Both SSE paths now re-check `still_valid` before the initial resync
  write, before every replay event, before the overflow resync and in the
  live loop; for users the check also requires an unchanged `me_view`, so a
  scope rewrite closes the stream rather than continuing with an old
  whitelist.
- `check_command_scope` for `WalletRefresh`/`WalletOffboard`/
  `WalletReadInputs`: claim identity must equal the payload identity and
  the claim's `may` must carry `wallet_refresh` / `wallet_offboard` /
  `view_own_status`; old claims without the new capabilities are rejected.
  `submit_authenticated` additionally requires a registry binding whose
  pool equals the claim's pool before `submit_as`. Cross-principal
  `command_id` replay stays `Denied` at `begin_scoped`. Tested in
  `every_wallet_kind_is_scoped_before_journal_and_replay` with a no-append
  assertion.

**Wallet adapter (`wallet_control.rs`, `wallet_projection.rs`).**
- Queue filename is `sha256("spc-lab-wallet/v1", run_id, actor,
  command_id)` as 64 hex plus `.json`; result is `<op>.result`; the raw
  `command_id`/identity never becomes a path component. Datadir binding
  requires a canonical path under `<run>/private`, a real 0700 directory
  owned by the process, loopback status address, and unique
  datadir/status per registry.
- `private_read` opens with `O_NOFOLLOW | O_NONBLOCK`, `fstat`s the
  descriptor (regular, 0600, same uid, <= 1 MiB) and bounds the read -
  this is the fd-based hygiene I asked for in the access review, done
  correctly here.
- Inputs DTO: `deny_unknown_fields`, schema tag, <= 4096 rows, source age
  within 15 s and not in the future, id exactly 72 hex (36-byte outpoint)
  with case-insensitive de-duplication, closed `state`/`rail` sets,
  canonical `u64` decimal `amount_sats`. Empty is empty, not zero.
- Execution revalidates: `apply()` re-reads fresh inputs, requires every
  selected id present and `spendable`, requires one rail, and validates
  the offboard destination as a regtest `bcrt1` witness program; the read
  model never authorizes.
- Queue publication: `create_new` 0600 temp, `sync_all`, `hard_link` to the
  destination (no replace), unlink temp, directory `sync_all`. An existing
  destination with identical bytes is idempotent; a differing payload is
  not overwritten. Write failures yield `Unknown` with "retain this
  command id", never a fabricated success.
- Result mapping: `round_confirmed` succeeds only for `WalletRefresh`,
  `offboard_confirmed` only for `WalletOffboard`; `stock_submitted` stays
  `Running`; `rejected_no_effect`/`noop` are terminal
  `RejectedPrecondition`; anything else is `Unknown`. Replays only read the
  result file and never re-queue. No network wait happens under the
  command mutex (status sockets are only on the GET path).
- Consumer compatibility, checked against the actual AP-3b
  `bins/spc-agent/src/wallet/maintenance.rs` in `lab-ark-wallet-bridge`:
  `WalletCommand {operation_id, vtxo_ids, action}` with
  `action: {kind: "refresh"} | {kind: "offboard", address}` and
  `deny_unknown_fields`; operation id must be 64 hex and equal the file
  stem; the consumer lists `*.json` only (the UI temp `.ui.tmp` and the
  agent's `.result.tmp` are invisible to it); command files must be 0600
  regular and <= 16 KiB (64 ids x 72 chars plus envelope is about 4.7 KiB);
  the five terminal stage strings are exactly the five the UI maps.
- `project_status` is a closed allowlist: `last_error` becomes the literal
  code `agent_reported_error`; u64 money becomes decimal strings before
  serialization; unknown keys never cross. The test feeds `xprv`, `proof`
  and a `postgres://` URL and asserts none survive.
- Join patch `/tmp/opencode/live-wallet-ui-ap3-join-r1/registry-bind.patch`
  compiles against the candidate's public `RunEngine` fields
  (`scenario`, `run_dir`, `wallet_registry` are `pub`), and binds
  `dir.join("state").canonicalize()` after launch, matching the
  interface's `<run>/private/wallet-<identity>/state/ui-inputs.json`.

## 4. Findings, ranked

### W1 (blocking, replay honesty): an unbound registry turns an in-flight wallet command into a false terminal rejection

`wallet_control::apply(binding, command, replay)` returns
`RejectedPrecondition("wallet provider not registered")` when `binding` is
`None` **before** it looks at `replay`. `RunEngine::submit_as` calls
`apply(registry.get(identity), &command, true)` whenever the journal holds
an `Existing` outcome in `Running | Unknown` for a wallet command. The
registry is in-memory and empty after `RunEngine::attach` (the AP-3b hook
binds only in the launch path; adopt and read-only attach never rebind).

Counterexample, fully reproducible with the existing fixtures:

1. Expanded run, wallet bound, user submits `WalletRefresh` -> `Running`,
   `<op>.json` is in the agent queue (test
   `queue_submission_is_not_completion_and_unknown_never_requeues` gets
   this far).
2. Controller restarts; `RunEngine::attach` -> `wallet_registry` is empty.
3. User presses "retry" (or reloads with the retained envelope) -> same
   `command_id` -> `begin_scoped` `Existing(Running)` -> `apply(None, ..,
   true)` -> `rejected_precondition`.
4. `app.js postCommand` treats `rejected_precondition` as resolved: clears
   `pendingCommand`, calls `savePending(null)`, re-enables the buttons.
   The agent may meanwhile execute the queued refresh or offboard. The
   user can now click again and mint a **new** `command_id`, i.e. a second
   queue file for the same VTXOs.

This breaks the parent's stated invariant ("ambiguous results remain
unknown, same ID, no resubmit") and the packet's own rule that only an
actual agent result is terminal. It is not a cross-principal or privacy
issue; it is a false terminal state.

Minimal fix (`wallet_control.rs`): when `replay` is true and `binding` is
`None`, return `OutcomeState::Unknown` with detail "wallet provider not
registered; result unavailable, retain original command". Keep the
non-replay `None` path as `RejectedPrecondition` (nothing was queued).
Regression test: bind, `apply(.., false)` -> `Running`; then
`apply(None, .., true)` -> `Unknown`, queue bytes unchanged. Optionally a
`wallet_browser_server`/engine test: submit -> `Running`, `attach` a new
engine without binding, replay -> `unknown`, UI keeps the retained
envelope.

### W2 (should fix, durability): the agent's terminal state never reaches the journal

In the `Existing` re-check `submit_as` calls
`command_log.finish(&command_id, &outcome)` after `apply(.., true)`, but
`CommandLog::finish` returns early when the entry already has an outcome
(it does, `Running` from the first apply). The returned outcome is correct
for that request, but the journal and the `command` evidence event stay at
`Running` forever; every later replay re-reads `<op>.result`. If the owner
ever prunes result files, a confirmed refresh regresses to `Unknown` on
replay, and the shareable `commands` summary reports `running` for
completed wallet operations. Minimal fix: either add an explicit
`CommandLog::supersede_nonterminal` that appends a `finish` line only when
the stored outcome is non-terminal (one new record type is not needed;
reuse `Finish` and let `open()` take the last one), or document that
wallet outcomes are read-through and never durable. The former is a
10-line change plus one restart test; the latter is a decision-record
sentence.

### W3 (should fix, UX honesty, not security): a `Running` wallet command blocks every other control for its whole duration

`postCommand` refuses any new command while `pendingCommand` is set, and
`unresolved` includes `running`. A `WalletRefresh` legitimately sits in
`Running` until the agent's round confirms (minutes), during which the
user cannot pause/resume/budget their own workers. The retained-envelope
rule exists for `unknown` (ambiguous), not for `running` (the controller
has durably recorded it). Minimal fix: keep the retained envelope and the
retry button for `running`, but only block new submissions while the
state is `unknown` (or while a POST is in flight).

### W4 (nits; fix opportunistically)

- `check_command_scope` returns `Ok` for the operator before the wallet
  arm runs `wallet_control::validate`, so a malformed operator wallet
  command is journaled and then rejected by `apply` (closed, no effect,
  but one wasted journal line). Move the `validate` call ahead of the
  operator early-return for wallet kinds.
- On `hard_link` failure the `<op>.ui.tmp` file is left behind (the
  consumer ignores non-`.json`, so harmless); `remove_file(temp)` in the
  error path would keep the queue directory clean.
- `ACCESS_JS` is `include_str!("../../../bins/spc-lab/static/access.js")`
  inside the library crate, while every other asset flows through
  `WebAssets` from the binary. It works in-tree, but it couples
  `crates/spc-lab` to `bins/spc-lab/static` at compile time and bypasses
  the `WebAssets` pattern; moving `access_js` into `WebAssets` is the
  consistent shape.
- `private_dir`/`private_read` require modes exactly `0700`/`0600`
  (stricter than `& 0o077 == 0`); the agent writes exactly those, so it
  matches today - note it in the decision so a future `0400` result file
  is not a surprise `Unknown`.

## 5. Scope statements I checked and accept

- No funded workflow is claimed anywhere in the report, decision or UI
  strings; T1 payment and watch-only are explicitly `not_available` with
  a reason, and `payment-send` is disabled until a server-side
  `payment_wallet.state === "available"` that nothing in this tree sets.
- The AP-3b registry caller and the private-inputs producer are
  separately owned serial joins; the candidate reports `/wallets` as
  `not_configured` until they land. The join patch and the producer
  interface are byte-compatible with this consumer as read (Section 3).
- The compiled browser proof is a transport/UI proof against synthetic
  providers through the real Rust controller; it is correctly labeled.
- The `http_sse.rs` assertion change replaces a stale implementation
  string (`addEventListener("message"`) with a byte-equality check of the
  served `app.js`/`access.js` against source plus a `new EventSource`
  prohibition; the replay/command assertions remain. Not a weakening.
- Changelog fragment is in the README single-bullet form; `SPEC.md` is
  baseline-identical (`bfb28139...`); the report's "initial not run"
  section is historical and superseded by the r8 receipts.

## 6. Merge-handoff conditions (for the serial owner)

1. Land W1 (and ideally W2/W3) in this packet before any user-facing use
   of the wallet buttons; W1 is the only item I consider blocking.
2. The AP-3b registry caller must also run on the adopt/attach path (or
   the controller must refuse wallet replays until it does); otherwise W1's
   fix only yields `Unknown` after restart rather than a live result,
   which is honest but degraded.
3. After the producer and registry caller join, run the merged
   `wallet_ui`, `access_scope`, `access_projection`, `http_sse` and the
   opt-in `wallet_browser_server` host against the joined tree; the
   current receipts cover this packet's tree, not the joined one.
4. T1 per-user balance/catalog projection remains unavailable; do not
   flip `payment_wallet.state` to `available` without an authenticated
   per-user source and its own scope arm.
5. Funded refresh/offboard behavior is AP-5/T2 validation; nothing here
   substitutes for it.

## 7. Verdict

**REQUEST_CHANGES**, bounded: fix W1 (five lines plus one test) so an
unbound registry on replay yields `Unknown` with the retained command
instead of a terminal rejection that releases the UI envelope. W2-W4 are
recommended in the same resubmission. The session, auth, scope, path
confinement, private-file hygiene, queue atomicity, DTO allowlisting and
money handling were audited against the actual code and the actual AP-3b
consumer and hold. No live-Ark, funded or UI-ceremony condition is added.

---

## Addendum (2026-11-10): W1 fix shape must cover the user path too

The parent read the W1 source and is right that the minimal fix in Section
4 (`apply(None, .., replay=true)` -> `Unknown`) closes the hole only for the
operator. In the frozen `19aa61a0...` tree `RunEngine::submit_authenticated`
(`engine.rs` 1718-1728) rejects a **user** wallet command with
`closed_rejection("wallet provider is outside this claim or unavailable")`
when `wallet_registry.get(identity)` is `None`, before `submit_as` and
therefore before the journal lookup. After `attach` the registry is empty,
so a user's retry of a `Running` refresh still gets a terminal
`rejected_precondition` and the UI still releases the envelope. Both layers
need to change together:

1. `submit_authenticated` (user path): keep every authority check as-is -
   revoked token (401 upstream), cross-principal `Denied`, wrong identity
   and wrong pool stay closed rejections. Split "provider unavailable" from
   "outside this claim": when the identity is the claim's own and the pool
   matches the scenario-derived binding (`derive_user_bindings`), an
   **absent registry binding** must fall through to `submit_as` so the
   journal decides. A *new* command (`BeginOutcome::New`) with no binding
   then reaches `apply(None, .., false)` -> `RejectedPrecondition` (nothing
   queued, honest). An *existing* `Running`/`Unknown` entry reaches
   `apply(None, .., true)` -> `Unknown` with the same `command_id` (W1).
2. `apply`: `replay && binding.is_none()` -> `Unknown`, detail "wallet
   provider not registered; result unavailable, retain original command".
   Keep the `None`+non-replay rejection.

Required recheck once the fix is frozen: one end-to-end test each for
operator and user - bind, submit -> `Running`, `attach` a fresh engine
(empty registry), replay the same `command_id` -> `unknown`, journal line
count unchanged, queue bytes unchanged; plus the negative that a
different user's token on that `command_id` is still the generic `Denied`,
and a wrong-pool / wrong-identity payload is still `rejected_precondition`
with no journal write. An actual owner `noop`/`rejected_no_effect` result
must still map to `rejected_precondition` (definite no-effect is terminal;
"provider unavailable" is not).

For W3, the same framing: worker controls may proceed while a wallet
command is `Running`, but the retained wallet envelope and its retry
button must survive untouched (per-kind or bounded tracking; never
overwrite `spc-lab-pending` with the worker command). For W2, a guarded
`supersede_nonterminal` that appends a `Finish` only over a stored
non-terminal outcome (and lets `open()` take the last `Finish`) is sound;
it must not be reachable for terminal outcomes.

Binding note: at the time of this addendum `engine.rs`,
`wallet_control.rs`, `command.rs` and `app.js` in the candidate worktree
already differ from the `19aa61a0...` freeze (the author is mid-fix). This
audit's Sections 1-7 are bound to `19aa61a0...` only; the fixed tree will
be re-checked against the new frozen hash the parent returns, not against
the moving worktree.

---

# Confined W1-W4 recheck (same auditor, 2026-11-10)

Candidate: same worktree, final freeze
`/tmp/opencode/live-wallet-ui-audit-final-manifest.json` (697 rows), full
`a964f157f93a007656d743d2104447290911c01392b1d07796ef1b957a16f533`, code
(excludes both report paths)
`b360b332a3c2c76e281f06be6a1ff0f58f40b43087ccd4348f33827208c1d5b2`;
incremental patch from the audited `19aa61a0...` tree
`/tmp/opencode/live-wallet-ui-audit-final-from-r2.patch` sha256
`29b18dd5775758c391ebd0bb16259af118aab4efd98a01a72efea20a2506aae5`.
Candidate fix report: `docs/reviews/2026-11-10-live-wallet-ui-audit-fixes.md`.
Disposition: **APPROVE** the UI packet at code `b360b332...` as a bounded,
fixture-proven controller/browser surface. W1-W3 are closed in code and
test; W4 is closed except the explicitly deferred `WebAssets` coupling.
Not covered (unchanged from Section 5): funded refresh/offboard/payment,
the real AP-3b/T1 joins, and the registry join below, which is reviewed
read-only and is not landed.

No code, candidate, primary source or other-worktree write; no cargo, node,
browser or lock use; receipts were read, not re-run. This appended section
is the only artifact.

## R1. Freeze verification

Recomputed: all 697 content hashes match the worktree; full and code hashes
reproduce with the manifest's declared `code_excludes`. Delta from
`19aa61a0...`: 11 modified (access.js, app.js, two JS test harnesses,
`access.rs`, `command.rs`, `engine.rs`, `wallet_control.rs`,
`wallet_projection.rs`, `wallet_browser_server.rs`, the implementation
report), 4 added (`tests/wallet_replay.rs`, decision, changelog fragment,
fix report). `Cargo.toml`/`Cargo.lock` untouched; `bind_persisted` uses only
existing `toml` and `spc-crypto` dependencies.

Receipts: `/tmp/opencode/live-wallet-ui-audit-r5/` has seven steps
(`fmt`, `affected`, `browser-build`, `clippy`, `replay`, `build`,
`journal`) all exit 0 with before == after source bindings; the narrow
Rust count is 6 (`wallet_ui`) + 2 (`command.rs` resolution units) + 6
(`wallet_replay`) = 14 as stated. Browser proof
`/tmp/opencode/live-wallet-ui-audit-browser-r1/proof.json`: 15 checks, 0
exceptions, 0 failures, no 64-hex token in any request URL, and its seven
asset hashes equal the current static bytes (`access.js` `78bd4df9...`,
`app.js` `8548baf6...`, etc.), so the browser proof covers the final UI
bytes; the later r5 Rust-only guard changes are proven by the compiled
engine/journal tests, not by a browser run, exactly as the parent states.

## R2. Findings closed (verified in source)

**W1 - both principals.** `submit_authenticated` now rejects a user
wallet command only when a registry binding exists with a different pool
or the scenario-derived binding for that identity does not match the
claim's pool (`derive_user_bindings`); own identity + matching pool with
an *absent* binding falls through to `submit_as`. In `submit_as`, every
wallet-kind `Existing`/`InFlight` goes through `resolve_wallet_replay`:
terminal stays as stored; non-terminal calls `apply(binding, .., true)`,
and `apply` now answers `Unknown("... retain original command")` when
`binding` is `None` on replay while the non-replay `None` path keeps the
honest `RejectedPrecondition` (nothing queued). A `Begin` write error for
a wallet kind returns `Unknown`, not `Failed`. Verified by
`user_and_operator_http_replay_after_attach_without_registry_stays_unknown`:
user and operator both `running` -> stop -> `attach` (empty registry) ->
HTTP replay `unknown` with the original `command_id`, journal `last_seq`
unchanged across two replays, queue bytes unchanged; forged actor,
wrong-pool claim rewrite, wrong identity all `journal_seq == 0` with no
append; changed payload `rejected_conflict`; read-only attach replay
`Unknown` with journal bytes unchanged; revoked token 401. The compiled
browser check "W1 registry-loss retry/reload stays unknown with original
ID and no journal or queue rewrite" exercises the user path end to end.
Authority was not relaxed anywhere: revoked, cross-principal, wrong
identity and wrong pool remain denied before any I/O or journal write.

**W2 - durable resolution.** `CommandLog::supersede_nonterminal` appends a
reused `Finish` record only when the stored outcome is `Running|Unknown`
and the new outcome differs; terminal is immutable (the test tries to
overwrite `Succeeded` with `Failed` and gets the stored `Succeeded`, no
append); RAM is updated only after a successful write+sync; scope and
full payload are re-checked (`entry_scope_matches`: scheme, principal,
hash, run, actor); the existing reader already takes the last `Finish`,
so no new record type and legacy `finish()` is byte/behavior unchanged.
Eight concurrent replays produce exactly one append (`seq + 1`) and all
return `Succeeded`; the terminal survives provider loss and restart on
both adopt and read-only attach. Transient `Unknown`/`Running` from an
observation does not append (journal_seq pinned to the stored intent).

**Torn-tail / append-failure guard (new, wallet-only).**
`wallet_resolution_failed` latches on any unparseable line at `open()`
and on any failed `append_record` (including a failed initial `Begin` and
a legacy `Finish`); while latched, wallet `Begin`s and resolutions return
`Err` -> `Unknown`, never a success, and non-wallet legacy behavior is
untouched (no hidden repair, no format change). Covered by
`failed_initial_begin_latches_wallet_guard_before_another_intent`
(UnixStream pair: bytes written, `sync` fails) and
`failed_wallet_resolution_write_or_sync_never_claims_durable_success`
(`/dev/full` write failure and sync failure, RAM stays `Running`, file
bytes unchanged, reopen still `Running`), plus
`torn_wallet_tail_on_reopen_cannot_authorize_a_new_queue_effect` (fresh
intent `Unknown`, no queue file, old intent `Unknown`, journal bytes
unchanged).

**W3 - bounded pending book.** `PendingCommands` (`access.js`): at most
16 envelopes, duplicate ids refused, one funds-class operation
(`wallet_refresh`/`wallet_offboard`/`send_user_payment`/`bootstrap_fund`/
`inject_tx`) at a time, unrelated worker controls allowed while funds are
`Running|Unknown`; `resolve()` clears an entry only on a terminal outcome
and, for a retry, only when it is durable (`journal_seq > 0` and not
`rejected_conflict`) - so a scope/observation/read-only denial on retry
retains the original wallet envelope. Storage is the same per
run/principal `spc-lab-pending` key (`spc-lab-pending/v2`), restored only
when every envelope's `run_id` and `actor` match the authenticated
context, and cleared on logout/switch/revoke as before. Node test and the
compiled check "W3 worker pause succeeds while running wallet envelope
remains independently tracked" (one `pause_workers` Begin appended, the
wallet envelope still present) cover it.

**W4.** `check_command_scope` runs `wallet_control::validate` before the
operator short-circuit (test `malformed_operator_is_rejected_before_journal_or_private_queue_access`);
on `hard_link` failure only our own `create_new` temp is unlinked, never
the destination or a result (`failed_link_removes_only_our_created_ui_temp`);
`private_dir`/`private_read` limits are unchanged. The `include_str!`
`WebAssets` coupling is explicitly deferred - non-blocking, as I said.

## R3. Registry join (read-only review, not landed)

`/tmp/opencode/live-wallet-ui-audit-registry-join/registry-launch-and-attach.patch`
adds `rebind_ui_registry(engine)` in `wallet_bridge.rs`, called after agent
launch and at the end of `RunEngine::attach` (both adopt and read-only). It
feeds `WalletRegistry::bind_persisted` from the resource manifest's
`wallet.<id>.status` and `front.<id>.sv2` leases plus the private
`wallet-<id>/agent.toml`, `pool-<front>.toml`, `provision.json` and the
`state/{state.sqlite,bark/db.sqlite}` 0600 files. `bind_persisted` only
`lstat`s/reads with the existing `private_read` guard, cross-checks
network, datadir, status listener, upstream address/authority key and the
provisioned SP address against the scenario identity, then delegates to
`bind` (loopback, unique datadir/listener, canonical path under private).
No spawn, no key material, no SQLite query, no config rewrite. A failed
rebind leaves the identity unbound, which the UI reports as
`not_configured` and which keeps queued operations at `Unknown` - never a
fabricated healthy wallet. Shape is sound. It is against provider-base
hashes `wallet_bridge.rs d33dec30 -> dc6b6fe9` and `engine.rs e7665bd4 ->
75201046`, not merged anywhere; the merged tree needs the `wallet_replay`
and `wallet_browser_server` suites run after the serial join. I do not
claim it landed or that any funded path works.

## R4. Non-blocking notes

- Under the torn-tail latch, a wallet command whose id is *not* in the
  journal returns `Unknown("wallet journal unavailable; retain original
  command")`. Nothing could have been queued for a never-recorded id (the
  queue write follows a durable `Begin`), so a definite
  `rejected_precondition` would be equally safe and would spare the user a
  permanently dead pending envelope. Conservative, not wrong; consider it
  when the latch policy is next touched.
- `rebind_ui_registry` swallows the failure reason (`let _ = attempt`).
  Surfacing a closed reason code (not config data) through `/wallets`
  would help the operator see why a wallet is `not_configured`.
- The latch has no in-run recovery path by design (no hidden repair); a
  run whose journal has a torn line keeps its wallet surface closed until
  a new run. Worth one sentence in operator docs.

## R5. Verdict

**APPROVE** the live wallet UI packet at code `b360b332...` (full
`a964f157...`) as a fixture-proven foundation: W1 closed for user and
operator with authority unchanged, W2 durable and append-only with a
sound wallet-only failure latch, W3 bounded and envelope-preserving, W4
closed except the deferred asset coupling. Conditions carried forward,
unchanged: the registry join is reviewed, not landed; merged-tree tests
after the AP-3b producer/registry and T1 joins; funded refresh/offboard/
payment validation remains AP-5/T2 and is not claimed here.
