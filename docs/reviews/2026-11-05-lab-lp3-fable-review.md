# LP-3 headless supervisor, evidence and thin browser: Fable review

Date: 2026-11-05
Reviewer: Fable (subagent), read-only review; no candidate file was edited, no
commit, push or subagent was made.
Candidate: `lab-runner` worktree, base `b0ab4ca34fda07c61a6335d05f598eaacc69678c`.
Snapshot: `/tmp/opencode/lab-lp3-review-manifest.json`, 53 files, aggregate
`sha256(json.dumps({'base','files'}, sort_keys, compact))` =
`0f2a49c4c2becb9941aebf004977c4e3662d560854abf91cac2273226ace517a`.
Contract: approved L3 snapshot `3edcd4e744149667b184f44bdf3833ba465e210671eb90912fb1db9b9b11ba25`
(`lab-contracts/docs/REGTEST-LAB-IMPLEMENTATION.md` §3, §6-9, LP-3), re-verified
file by file (7/7 match).

## Verdict

**REQUEST_CHANGES** for snapshot `0f2a49c4...6ace517a`.

The foundation is substantial and most of it is the real thing: strict
scenario v1 with nominal IDs, a fsynced command log with payload-conflict
rejection, distinct evidence states with `unknown` preserved, a sound sampled
credit oracle, per-pool authority pairs in 0600 files, a bounded loopback
HTTP/SSE surface that refuses non-loopback binds and private artifact routes,
and an exact `spc-cpu-miner-control` v1 envelope. The claimed test results
reproduce. But three durable-record defects contradict the contract's core
promises (§6.2.5 "never assumed successful", §6.3 outcomes, §8.1 process
states) and are observable in the implementer's own run artifact, so the packet
cannot be approved as a usable runner contract yet. All blockers are bounded
fixes inside LP-3 owned paths; none require a future packet.

## What was verified

- Manifest: all 53 files hash-match; aggregate reproduces.
- Nix evidence: `/tmp/opencode/LP3-nix-src-final13` matches the manifest
  53/53; the four derivations named in `LP3-final13-nix-builds.log`
  (`spc-lab`, `spc-fmt`, `spc-clippy`, `spc-workspace-tests`) have valid output
  paths; `nix log` of the clippy derivation shows 0 warnings/errors and the
  workspace-tests derivation shows 481 passed, 0 failed.
- `LP3-final13-nix.log`: 52 passed, 2 ignored for `spc-lab`; fmt and clippy
  clean. `LP3-final13-workspace-test.log`: 481 passed, 0 failed, 2 ignored.
- Ignored proofs: `LP3-final13-real-node.log` (1 passed, Bearer RPC baseline
  profile, `no_pow_retargeting=true`) and `LP3-final13-real-cpu-miner.log`
  (1 passed, control-only `--check-config`/snapshot/stop). Both are process
  proofs only, as the report says.
- Re-run here (`cargo test -j2 -p spc-lab`, `/tmp/opencode/Fable-LP3-test.log`):
  52 passed, 0 failed, 2 ignored. No fixture processes leaked afterwards.
- LP-2 envelope: compared `adapters.rs` against
  `lab-cpu-miner/bins/spc-cpu-miner/PROTOCOL.md` v1: command/result/event
  envelopes, operation names, payload shapes (`total_attempts`,
  `worker_ids`, `switch_pool`), and outcome vocabulary match. Unknown
  `event.name` values (including the additive `submissions_evicted`) pass
  through as generic `miner_event`; no ACK is turned into `accounted`.
- LP-1 adapter: `spc_evidence.rs` matches the actual
  `lab-share-integration/bins/spc-pool/src/evidence.rs` shape (`target =
  "spc_evidence"`, `fields.event`, `fields.evidence` as a JSON string,
  `generation` as a decimal string, BE `share_target`, `front_id: u8`,
  `sp_id` hex on `accounted`). Raw lines go to a 0600 private file and
  `accounted` is retained as a private raw record, not promoted.
- L2 introspection: compared `RpcNodeControl::effective_profile` against
  `rbitcoin-lab-retarget-profile/crates/rbitcoin-rpc/src/methods/chain.rs::consensus_info`
  (e8cf9c9d6). Present in both: `network`, `pow_limit_bits`,
  `retarget_floor_bits`, `pow_target_spacing`, `pow_target_timespan`,
  `difficulty_adjustment_interval`, `no_pow_retargeting`,
  `allow_min_difficulty_blocks`, `regtest_pow_bits_overlay`,
  `subsidy_halving_interval`, `coinbase_maturity`. Read by the adapter but not
  emitted by L2 today: `consensus_profile_id`, `profile_fingerprint`,
  `initial_target_bits`, `subsidy_initial_sats`,
  `witness_commitment_supported`, `tdp_session_cap` (all resolve to `None`;
  see B11 for the one place `None` is not fail-closed).
- Real pool config: the actual L1 `bins/spc-pool/src/config.rs` requires an
  inline `authority_secret_key` (`RedactedSecret`) in `[pool]`. The runner
  provides `{authority_public_key}`/`{authority_secret_file}` placeholders and a
  0600 `[pool]` TOML fragment, but has no pool TOML construction/templating.
  This is an honest integration gap, stated in report §9.1 and §10, not a
  hidden claim. It must stay a named non-claim until LP-3.1 renders a complete
  private pool config.

## Probes run (all outputs under `/tmp/opencode/Fable-LP3-*`)

1. `spc-lab run --headless --keep` then `spc-lab command stop_run`
   (`Fable-LP3-run-keep.log`, `Fable-LP3-stop-probe.log`): outcome
   `succeeded "run stopped"` recorded durably in `private/commands.jsonl`
   while redis/postgres/node/front PIDs stayed alive; `spc-lab reap` later
   stopped them (`Fable-LP3-reap-probe.log`).
2. Appended crash-style `begin` records for `resume_workers`, `pause_workers`,
   `add_miner` and ran `spc-lab adopt` (`Fable-LP3-adopt-probe.log`):
   `resume_workers` reconciled `succeeded: effect is present in the run
   manifest`; the other two `unknown`.
3. Scenario with a `remove_miner` phase (`Fable-LP3-remove-probe.log`,
   run dir `Fable-LP3-runs/remove-probe/run`): `remove_miner` `succeeded`,
   process record went `orphanunverified`; at stop both miners
   `orphanunverified`; final `run-manifest.json` is `stopped` with all six
   processes still `running`. The implementer's own
   `/tmp/opencode/LP3-runs-final9/20261106T021009Z-2515425/private/evidence.jsonl`
   shows the same (events 20, 21) and the same stale manifest.
4. `spc-lab serve` fed a 64 MiB request line with no CRLF
   (`Fable-LP3-serve-probe.log`): accepted and buffered (RSS 10 MB -> 70 MB)
   before the 400.
5. `wait_height` with no producer able to advance returned `blocked_on_height`;
   timeout returned `failed` (probe 3). Correct per §6.3/§8.4.

## Ranked blockers

### B1 (high) - Own exited child is classified `orphan_unverified`

`crates/spc-lab/src/supervisor.rs:369-392` runs `verify()` before
`process_gone()`. For a zombie (our pipe miner exits on `stop`/EOF and is not
yet reaped), `/proc/<pid>/environ` is empty and `/proc/<pid>/exe` is
unreadable, so `verify()` (`:290-322`, `:573-592`) returns
`RunTokenMismatch`/`ExecutableMismatch` and `stop()` marks
`OrphanUnverified`, the state §8.1.3-4 reserves for "may be an unrelated
process". Counterexample: probe 3 and the implementer's run-once artifact.
Consequences: `remove_miner` reports success while the record says unverified;
every normal stop of a pipe miner ends `orphan_unverified`; `reap()` then
reports `stale_resources` for a clean shutdown; the `orphan_unverified` signal
becomes noise. Fix: in `verify()`/`stop()`, check `children.try_wait()` and
`process_state_char == 'Z'` first and return `Missing`/mark `Exited`; reap
children promptly. Test: launch `miner-jsonl`, send `stop`, wait for state
`Z`, assert `supervisor.stop` returns `Ok(false)` and the record is `Exited`,
never `OrphanUnverified`. Also tighten `real_cpu_miner_proof.rs:56` which
discards the `stop` result and hides this.

### B2 (high) - `stop_run` records `succeeded` without stopping anything

`crates/spc-lab/src/engine.rs:1733-1738` returns `Ok(())` when `!started &&
!manifest_state().is_live()`; `apply` (`:1165-1168`) turns that into
`succeeded "run stopped"`. After `attach`, `started` is always false
(`:336-339`) and `adopt_processes` (`:1935-1945`) moves `Running` to
`CompletedWithErrors` whenever a pipe miner exists, so the early return fires
while adopted processes are alive (probe 1). This is a durable success record
that lies about its effect. Fix: `stop()` must call `stop_all` whenever the
supervisor holds verified records regardless of `started`, and return a
`rejected_precondition`/distinct "nothing to stop" outcome when there are
none. Test: `run --keep`, `attach`, submit `stop_run`, assert all adopted
PIDs are gone before the outcome is `succeeded`.

### B3 (high) - Crash reconciliation assumes `resume_workers` succeeded

`crates/spc-lab/src/engine.rs:1856-1871` reconciles against the in-memory
`EngineState`, which is `default()` after attach; `ResumeWorkers =>
!paused_miners.contains(..)` is vacuously true, so any interrupted resume is
`succeeded` (probe 2), violating §6.2.5 and the report's own §4.4. AddMiner/
RemoveMiner/Pause are `unknown` only by accident of the empty maps, with a
detail text claiming the manifest was consulted. Fix: Pause/Resume/
SetWorkerBudget -> `unknown` always (pipe control is gone); AddMiner ->
manifest has a verified `("miner", id)` record; RemoveMiner -> record absent
or exited. Test: append a `begin` for `resume_workers`, attach, assert
`Unknown`.

### B4 (medium) - Manifest process records are not synced on stop

`engine.rs:1740-1751` saves `state: stopped` without copying
`supervisor.records()` into `manifest.processes` (done only in adopt/reap at
`:1933-1934`, `:1960`). Both run artifacts show `stopped` with six `running`
records, so a later adopt verifies six dead PIDs as if they were expected to be
alive. Fix: sync records before every `save` in `stop()`/`start()` failure.
Test: after `stop()`, every manifest record is `stopped|exited`.

### B5 (medium) - Entity state is memory-only and misreported after adoption

`EngineState` (`engine.rs:65-73`) is never rebuilt from the command log on
attach. After a crash, `entity_states()` (`:1038-1070`) shows paused workers as
`running`, `chain_producers()` (`:1641`) counts removed miners, and
`extra_miners` is empty. The honest state after adoption is `unknown` or
`adopted_control_lost` per worker. Fix: replay finished `succeeded` entity
commands from the log to rebuild the sets, and report worker status `unknown`
when the miner control is lost. Test: pause, drop engine, attach, assert no
worker is `running`.

### B6 (medium) - Commands are not serialized; check-then-act races

`submit()` (`engine.rs:1110-1155`) holds the command-log mutex only in
`begin`; `apply()` takes `state`/`adapters` locks piecemeal. Two concurrent
`add_miner` with the same `miner_id` and different `command_id`s both pass
`:1180-1183`, launch two processes, and `records.insert`/`upsert_process`
overwrite the first record (untracked process); `remove_miner` racing
`add_connection` is similar. §6.2.4 requires log-ordered precedence recorded
in the outcome. Fix: one `apply` mutex (commands are already synchronous) and
include the log `seq` in the outcome. Test: two threads `add_miner` same id ->
exactly one `succeeded`, one `rejected_precondition`, one process record.

### B7 (medium) - Snapshot/cursor is not a consistent barrier; future cursor is a silent skip

`engine.rs:1016-1018` evaluates `entity_states()` before `evidence.snapshot`
takes the evidence lock, so an event between the two is neither reflected in
the entities nor replayed after `snapshot_cursor`. `evidence.rs:640` and
`:594` only resync when `after < retained_from - 1`; `after > last` yields an
empty replay and no `resync`, so a client with a bogus cursor silently misses
every event until ids catch up (§7.5 "no silent skip"). Fix: take the cursor
under the evidence lock first, then read entities (idempotent replay), and
return `resync_required` for `after > last`. Test: `subscribe(10_000)` on a
10-event store -> `ResyncRequired`.

### B8 (medium) - Browser reconnect and controls do not meet the stated contract

`http.rs:266-331` ignores `Last-Event-ID`; `app.js:86-89` lets `EventSource`
auto-reconnect with the original `?after=`, so a reconnect replays duplicates
(the Rust test reconnects manually and does not cover this). `app.js:93` lists
fixed kinds and omits `miner_event`/`spc_evidence`, so the cursor does not
advance on them. `app.js:113-119` sends `miner_id: <worker id>` from worker
rows and `worker_id: <miner id>` from the budget control, so the budget button
cannot succeed. Fix: honor `Last-Event-ID` (prefer it over `after`), close and
re-attach on `error` with `state.cursor`, carry `miner_id` on worker rows, and
use `kind()` generically. Test: HTTP test sends `Last-Event-ID: N` and asserts
no event `<= N` is replayed.

### B9 (medium) - Private artifacts are world-readable

`artifacts.rs:250-259` (`write_json`), `:233-247` (`copy_private_evidence`),
`command.rs:226-230`, `evidence.rs:476-480` all create files with the default
umask; observed `-rw-rw-r--` on `private/evidence.jsonl`,
`private/runtime.json` (the `LabConfig`, which carries `rpc_auth` such as
`basic:user:pass`/`bearer:<token>` when configured), `private/scenario.json`
and `private/commands.jsonl`. Only the authority files and the spc_evidence raw
capture are 0600. §7.7 requires separate access for identity-bearing evidence.
Fix: create `private/` 0700 and open every private file with `.mode(0o600)`;
`artifacts_privacy.rs` should assert modes.

### B10 (medium) - HTTP header parsing is unbounded before the limit check

`http.rs:403` and `:417` call `read_line` on an unbounded `BufReader` and only
compare the length after the whole line is buffered (probe 4: 64 MiB
accepted). Loopback-only limits exposure, but the "16 KiB headers" claim does
not hold and 64 such connections are allowed. Fix: wrap the reader in
`take(MAX_HEADER_BYTES)` or read byte-bounded lines. Test: 32 KiB without
CRLF -> 400 after at most 16 KiB read.

### B11 (medium) - Subsidy gate is silently skipped when the node does not expose it

`config.rs:546-559` compares `subsidy_initial_sats` only when both sides are
`Some`, unlike every other field routed through `compare_option`
(`:618-639`, which fails closed with `profile_requirement_missing`). Real L2
`consensus_info` does not emit `subsidy_initial_sats`, so an economic scenario
requiring it passes against a real node (§8.4 "Preflight fail before any
economic scenario"). `check_tdp_capacity` (`:738-746`) likewise skips when the
cap is unknown. Fix: route both through missing-is-failure. Test: requirement
`Some`, profile `None` -> preflight error.

### B12 (medium-low) - A slow browser decides assertion outcomes; real durable losses are silent

`evidence.rs:882-931` records a per-subscriber SSE overflow as a global `Gap`,
and `evaluate_assertion` (`:745-749`) turns any gap into `inconclusive` for
every non-statistics assertion, although the durable store lost nothing. Meanwhile
durable-path losses are not gaps: evidence write errors are ignored
(`:873-874`, `:923-924`), driver `dropped_before > 0` is stored but never
becomes `event_lost`, and unparseable driver events are dropped silently
(`adapters.rs:410-414`). Fix: scope subscriber overflow to that subscriber
(SSE `event: gap`), and record write failures, `dropped_before`, and parse
failures as store-level `event_lost`/`unknown`. Test: fill one SSE queue ->
`exists(run_state)` still passes; inject a write failure -> inconclusive.

### B13 (low-medium) - Run lock takeover race; heartbeat never written

`manifest.rs:246-248` removes a stale lock and retries `create_new`; two
adopters that both observed the stale lock can have the second `remove_file`
delete the first adopter's fresh lock, leaving two owners. `heartbeat()`
(`:258`) is never called, so "live owner heartbeat" is PID liveness only. Fix:
`flock()` the lock file (kernel-released on death) and keep the PID record as
metadata.

### B14 (low) - Two spellings of one vocabulary

`format!("{:?}").to_ascii_lowercase()` (`engine.rs:1052`, `:1150`, `:1772`,
`:1814`, `:2113`, `:2126`) emits `blockedonheight`, `rejectedprecondition`,
`orphanunverified`, `adoptedcontrollost`, `completedwitherrors` in evidence,
snapshots and the shareable manifest, while command outcomes serialize
`blocked_on_height` etc. One contract should have one spelling; serialize via
serde.

### B15 (low) - Placeholder/dead public API

`oracle.rs:284-298` `family_error` computes and discards; `config.rs:876-901`
`pool_storage_urls` builds a malformed `postgres://role@host/db/<run_dir>`
URL; `submit_candidate`, `hash_hex_meets_target`, `mark_orphan` are unused;
`secrets.rs:120-125` zeroizes a copy of the secret, not the key. The oracle sums
`accounted.diff_sum` per record (`:195-211`) but every test uses one share per
identity, so slice-aggregate versus per-share semantics is untested. Remove or
implement, and add a two-share oracle test.

### B16 (low) - Signalling details

`supervisor.rs:502-506` shells out to `kill` (fork/exec widens the
verify-to-signal window and requires `kill` on PATH for the Nix package); use
`libc::kill`/`killpg`. `executable_path` is captured right after spawn
(`:231`); a `#!` wrapper that later execs yields `ExecutableMismatch`, so such
an adapter can never be stopped by the supervisor (fail-closed leak); record
and document. The "never kills an unrelated PID" claim otherwise holds:
start-time token, run token and executable are re-verified before TERM and
again before KILL, and PID reuse yields `orphan_unverified`.

### B17 (low) - Readiness can be satisfied by a foreign listener

`resource.rs:103-129` reserves ports by bind/drop with a process-local set; no
cross-run manifest check exists despite §3.4, and `Readiness::Tcp`
(`supervisor.rs:535-545`) connects without proving the listener belongs to the
child. Loopback ephemeral ports make this rare; note it in the manifest as a
known limitation or verify via `/proc/net/tcp` inode ownership.

### B18 (low) - Lifecycle edges

`stop()` on a `Created` run returns `succeeded "run stopped"`; `reconcile`
treats `Failed` as a successful stop (`command.rs:435-442`); `spc-lab events`
blocks forever on a finished run (`main.rs:260`); `spc-lab serve` of a kept run
shows every entity as `declared` because read-only attach has no supervisor
records; `spc-lab run` calls `start()`/`stop()` directly so the primary path
leaves `commands.jsonl` empty (observed), and the report's "adopt followed by
the HTTP API" path does not exist (`adopt` is one-shot and `started` stays
false, so every entity command is `rejected_precondition`).

## Test quality notes

- `scenario_v1_one_pool_two_miners.json` asserts only `exists(run_state)`,
  which passes as soon as the run emits its own state: it checks the runner
  against itself. Add at least one assertion that can fail or be inconclusive
  for a reason outside the runner.
- `engine_restart.rs:109-116` cleans up by `kill -TERM -- -<pid>` from the
  test and leaves live processes if that fails; `real_cpu_miner_proof.rs:56`
  discards the `stop` result (masks B1); `restart_adopts...` accepts any
  non-`Adopted` state for miners.
- `http_sse.rs` sleeps 100 ms (`:91`) rather than waiting on a condition;
  `height_wait.rs` and `two_runs_isolation.rs` are sound.
- No test exercises concurrent `submit`, `Last-Event-ID`, a future cursor, or a
  multi-share oracle sum.

## Positives worth keeping

Nominal IDs and `deny_unknown_fields` everywhere; fsync-before-effect command
log with a tolerant torn-tail replay and conflict records; `unknown`-preserving
`canonical_at` with reorg history retained and `node_rejected` as the only
"invalid" source; sampled oracle that recomputes SHA256d, checks the target
family `S(2^j)` against `m_issue` and catches wrong generation/credit/target;
0600 per-pool authority pairs with a forbidden-key scan on the shareable
manifest; loopback-only HTTP that blocks `private/`, parent components and
commands on read-only attach; fixture adapters consistently tagged and
`non_claims` written into artifacts; `partition_link` refused on the fixture
backend; `add_worker` honestly refused rather than faked.

## Remaining real integration gates (not LP-3 blockers)

1. Real `spc-pool` launch: inline `authority_secret_key` plus a complete pool
   TOML must be rendered into a 0600 private file per front from the run's
   actual ports/URLs; today only a fragment exists.
2. LP-1 (REQUEST_CHANGES pending): wire `SpcEvidenceAdapter` to the live front
   log lifecycle after the revision; treat field additions
   (`share_hash`, extranonce) as a versioned handoff and keep `accounted` raw
   until mapped.
3. LP-2 (under re-review): runner is compatible with the unchanged v1 envelope;
   re-run `miner_protocol` against the final fixtures.
4. L2 profile: six adapter fields have no L2 source today; keep them
   fail-closed when required (after B11) and do not label a baseline run as
   retargeting.
5. LP-4: netns backend; partition assertions stay `inconclusive`.
6. Cross-packet retention: the root front's `keep_slices`/`retain_windows`
   default (16) versus `window_n` under network-block work for minority pools
   is an LP-1 item. LP-3's `warmup.window_fills`/statistics declarations must
   not be read as evidence that N-block history was retained; until LP-1
   reports retention, any window-coverage assertion is `inconclusive`.
7. No real mining, SV2 interoperability, payout, retarget or canonical-block
   claim is made or supported by this snapshot; that is correctly stated.

## Approval path

Fix B1-B11 (B12-B18 may follow in the same or next revision), add the named
tests, re-run `cargo test -p spc-lab`, the Nix fmt/clippy/package/test
derivations on a fresh source copy, and `run-once.sh --headless`; then publish
a new manifest hash. Re-review will start from B1-B3 artifacts
(`evidence.jsonl` process states, `commands.jsonl` reconciliation records, and
a kept-run `stop_run` probe).

---

# Second pass (2026-11-06): LP-3 combined R2

Reviewer: Fable (subagent), read-only; no candidate edit, commit or subagent.
Candidate: same `lab-runner` worktree, base `b0ab4ca34fda07c61a6335d05f598eaacc69678c`.
Snapshot: `/tmp/opencode/lab-lp3-r2-review-manifest.json`, 60 files, aggregate
`9bec018d34273f0a7e783cfc36e1e8646b9c032ea915cce32464168888842f85`
(re-verified 60/60; 7 new files, 32 changed, none removed versus R1).

## Verdict

**REQUEST_CHANGES (focused)** for snapshot `9bec018d...8842f85`.

Every R1 blocker B1-B18 is addressed in code, not just in the report, and the
R1 counterexamples no longer reproduce (details below). One new defect was
introduced by the B7 fix and reproduces reliably: a lock-order deadlock between
the snapshot barrier and run-state emission hangs a live `run --listen` at
`stopping` with every child alive and the owner lock held. It is a one-line
fix with a clear test, so this pass is focused on it; nothing else below is a
gate for the bounded foundation.

## Evidence verified

- `/tmp/opencode/LP3-r2-nix-src-final` matches the R2 manifest 60/60;
  `nix build --dry-run` on that copy for `packages.spc-lab` and checks `fmt`,
  `clippy`, `test`, `ui-node` reports nothing to build (all outputs realised),
  so the empty `LP3-r2-final-nix-builds.log` is a silent success, not a gap.
- `LP3-r2-final-nix.log`: 71 passed / 2 ignored; fmt and clippy clean (the only
  warnings are the cargo cache permission notices and the dirty-tree note).
  `LP3-r2-final-workspace-tests.log`: 500 passed, 0 failed, 2 ignored.
  `LP3-r2-final-node.log`: 10 passed, 0 failed.
- Re-run here: `cargo test -j2 -p spc-lab` -> 71 passed / 2 ignored
  (`/tmp/opencode/Fable-LP3-r2-test.log`); no fixture processes leaked.
- Ignored proofs: real node (`LP3-r2-final-real-node.log`) and real control-only
  miner (`LP3-r2-approved-lp2-proof.log`, `LP3-r2-final-real-cpu-miner.log`)
  pass. The report's claim that the miner proof used the approved binary
  `sha256:23961abe...` is not verifiable from those logs (no path or hash is
  printed; no store or target binary with that hash was found here). Not a
  blocker; see L9.
- `#![deny(unsafe_code)]` in `crates/spc-lab/src/lib.rs:8`; no `unsafe` in the
  packet; signalling uses `rustix::process::kill_process_group`
  (`supervisor.rs:556`).

## R1 dispositions checked against code and probes

| R1 | Disposition | How verified |
|---|---|---|
| B1 zombie -> orphan | Fixed. `stop()` checks `process_gone` (try_wait/`Z`) before identity (`supervisor.rs:406-409`); `verify()` treats `Z` as `Missing` (`:323-325`). | Probe `Fable-LP3-r2-remove.log`: `remove_miner` -> record `exited` (events 20, 27); both miners `exited` at stop; test `an_exited_own_child_is_reaped_and_never_marked_orphan_unverified`. |
| B2 false stop success | Fixed. `stop()` stops whenever verified records exist; no-record `created`/`failed` is `stop_precondition` -> `rejected_precondition` (`engine.rs:1850-1869`, `:1271-1277`). | Probe `Fable-LP3-r2-stop.log`: kept run + `stop_run` -> all six PIDs dead before `succeeded`, manifest `stopped`. |
| B3 crash inference | Fixed. Pause/resume/budget/connection/switch -> `unknown`; add/remove miner use manifest process records only (`command.rs:434-537`). | Probe: injected `resume_workers`/`add_miner` begins -> both `unknown`. |
| B4 manifest sync | Fixed. `sync_processes()` before final saves (`engine.rs:1881`, `:1903-1911`, start failure `:399`). | Both probe manifests show terminal records only. |
| B5 state after adopt | Fixed with a caveat (L1). Journal replay of `succeeded` outcomes (`engine.rs:2009-2093`); workers `unknown` without live control (`:1134-1140`). | `pause_state_is_not_inferred_after_adoption`. |
| B6 serialization | Fixed. Whole-submit `command_mutex` (`engine.rs:1200`); `journal_seq` on outcomes and command evidence. | `concurrent_add_miner_is_log_serialized_and_creates_one_process`. |
| B7 barrier / future cursor | Fixed but introduced D1. `snapshot_with` holds the evidence lock around the entity projection (`evidence.rs:615-661`); `after > last` -> resync (`:691`, `:628`). | Probe `Fable-LP3-r2-http-probe.log`: `?cursor=999999` -> `resync_required=true`; SSE `after=999999` -> `event: resync`. |
| B8 browser | Fixed. `Last-Event-ID` wins over `after` (`http.rs:246-257`); generic `event: message` (`:391`); app.js closes and re-attaches from its own cursor (`app.js:246-268`), worker rows carry `miner_id` (`:51-57`), cursor dedupe (`:217-219`). | Probe: `Last-Event-ID: 30` over `after=0` replays `[31, 32]` only; 10 Node helper tests. |
| B9 private modes | Fixed. `private/` and `logs/` 0700; manifest, commands, evidence, runtime, scenario, logs, authority files 0600 (`artifacts.rs:52-62`, `:255-262`, `manifest.rs:131-138`, `evidence.rs:496-544`). | `ls -la` on `LP3-r2-runs-final/...` and the probe runs. See L7 for the run-root leftovers. |
| B10 header bound | Fixed. Byte-budgeted `read_bounded_line` over `take()` (`http.rs:490-519`), identity framing only, body cap before allocation (`:567-585`). | Probe: 64 MiB request line -> reset after ~1 MiB sent, 400, RSS unchanged (3592 kB). |
| B11 gates | Fixed. Required `subsidy_initial_sats` and `tdp_session_cap` fail closed with `profile_requirement_missing` (`config.rs:546-566`, `:752-766`). | `preflight` tests. |
| B12 loss scopes | Fixed. `GapScope::{Store,Subscriber}`; only store gaps make assertions inconclusive (`evidence.rs:786-787`); durable write failure -> store gap (`:928-939`); driver cumulative `dropped_before` delta once per incarnation and `driver_parse_error` -> store gap (`engine.rs:949-976`, `adapters.rs:487`). | `driver_loss` test. See L2 for the lagging subscriber's own view. |
| B13 run lock | Fixed. `flock` exclusive non-blocking on a 0600 file opened without truncate; metadata written after the lock via `set_len`/`write_all`; release is `Unlock` only, never rename or unlink (`manifest.rs:219-299`). | Kernel release confirmed: `spc-lab reap` succeeded on four runs whose owner was SIGKILLed (`Fable-LP3-r2-deadlock-reap.log`). |
| B14 vocabulary | Fixed. `as_str()` everywhere; `blocked_on_height`, `completed_with_errors` observed in probe output. | Probe logs. |
| B15 dead API | Fixed. Listed helpers removed; `non_secure_erase` on the secret (`secrets.rs:121`); two-share/aggregate oracle test (`oracle_generation_credit.rs:191`). | grep + test. |
| B16 signalling | Fixed with a caveat (L4). `rustix` group signal; `#!` scripts refused before launch (`supervisor.rs:213-217`, `:597-607`). | `executable_wrapper_scripts_are_refused_before_launch`. |
| B17 readiness/ports | Fixed with caveats (L5, L6). TCP readiness requires the LISTEN inode from `/proc/net/tcp{,6}` to appear in the child's own `/proc/<pid>/fd` (`supervisor.rs:611-646`); ports hold `flock`ed per-port files under a shared root (`resource.rs:150-176`). | `readiness_rejects_a_foreign_listener_on_the_same_port`; two-run test. |
| B18 lifecycle | Fixed. CLI `run` journals `StartRun`/`StopRun` (`main.rs:169-210`); `created` stop is `rejected_precondition`; `events` exits after replay; `serve` uses manifest rows. | `LP3-r2-final-run-once.log` state `stopped`; `stop_on_created_run_is_a_rejected_precondition_not_success`. |

## D1 (blocker) - Snapshot barrier deadlocks against run-state emission

`engine.rs:2331` iterates `for record in self.supervisor.lock().unwrap().records()`.
A temporary created in a `for` head lives until the loop ends, so the
supervisor mutex is held while each iteration calls `self.evidence.append`
(lock order supervisor -> evidence). `evidence.rs:615-629` now holds the
evidence mutex while `RunEngine::entity_states` (`engine.rs:1060-1071`) takes
supervisor, miners, manifest and state (lock order evidence -> supervisor). Any
`GET /snapshot` (or SSE resync, which calls `engine.snapshot`) concurrent with
`emit_run_state("stopping"|"running"|...)` can deadlock both threads.

Reproduction (`/tmp/opencode/Fable-LP3-r2-deadlock-probe.py`, log
`Fable-LP3-r2-deadlock-probe.log`): `spc-lab run --headless --listen
127.0.0.1:0 --duration-secs 2` with eight threads polling `/snapshot`; 4 of 8
iterations hung past 25 s. Every hung run's manifest was `stopping` and its
last evidence line was `run_state: stopping` with no process observations
after it, which is exactly the loop at `:2331`. Children stayed alive and the
owner `flock` stayed held until the process was killed; `spc-lab reap` then
cleaned up (16 orphaned fixtures were left by the probe and reaped).

Fix: collect `let records = self.supervisor.lock().unwrap().records();` into a
local before the loop (and audit for any other lock held across an evidence
append; `record_miner_event` and `stop()` are already correct). Alternatively
compute the entity projection under a single engine-wide read lock taken
before the evidence lock, but the local-Vec fix is sufficient here. Test: start
a fixture run with `HttpServer`, spawn N threads hammering `/snapshot`, submit
`stop_run`, and assert completion within a bounded time (or a `try_lock`
ordering assertion in `snapshot_with`).

## Low items (not gates; record or fix in LP-3.1)

- L1 Replay order. `CommandLog::entries()` (`command.rs:421-428`) iterates a
  `BTreeMap<CommandId, _>`, so `rebuild_state_from_log` (`engine.rs:2011`)
  replays in command-id order, not journal order. Generated ids happen to sort
  by time, but phase/browser ids (`crypto.randomUUID`, `cmd-remove-1`) do not;
  `add_miner z-add` then `remove_miner a-remove` rebuilds the miner as present.
  Latent today because every entity command is rejected after adoption, but
  the projection is wrong by construction. Store `seq` in `Entry` and sort.
- L2 Lagging subscriber. On queue overflow the dropped event and the gap
  record are never delivered to that subscriber (`evidence.rs:955-958`,
  `:970-971`); when it drains, it sees an id jump and `app.js:217-219` accepts
  any newer id. The durable store is intact (correct per B12), but §7.5 says
  the observer must not silently skip: set a per-subscriber `lagged` flag and
  send a `gap` marker on the next successful send, or have the client refetch
  the snapshot on a non-consecutive id.
- L3 Whole-submit mutex and long waits. `wait_for_height` sleeps under
  `command_mutex` (`engine.rs:1200`, `:1689-1748`), so `stop_run` and every
  HTTP command block for up to `timeout_secs`. §6.2.4 "stop beats a concurrent
  start" is realised only as FIFO ordering. Make waits cancellable by a pending
  stop, or bound them in the scenario validator.
- L4 Wrapper refusal scope. Only `#!` scripts are refused. An ELF launcher that
  `exec`s another binary (Nix `makeWrapper` binary wrappers, `exec`-style
  launchers) still changes `/proc/<pid>/exe` after the record is captured
  (`supervisor.rs:257`) and becomes `ExecutableMismatch` -> `orphan_unverified`,
  a fail-closed leak. The required Rust binaries (`rbitcoin-node`,
  `spc-cpu-miner`, `spc-pool`) are direct ELF today; the report should state the
  limit instead of "wrappers refused".
- L5 Ownership check scope. `listener_owned_by` inspects the leader PID's fd
  table only. A daemonising adapter (double fork) or one whose listener lives
  in a helper can never pass readiness (fail-closed). `listener_inode` picks
  the first LISTEN row for the port regardless of address. Document, or check
  the process group's fd tables.
- L6 Port lock root. Default `/tmp/opencode/spc-lab-port-locks` 0700
  (`resource.rs:151-155`) is host-wide for spc-lab runs (good) but hard-coded
  to this machine's convention; a second user hits EACCES. Prefer
  `$XDG_RUNTIME_DIR`/`$TMPDIR` with the env override kept.
- L7 Run-root leftovers. `run-once.sh` writes `lab-config.json` (operator
  adapter config, may carry `rpc_auth`) into the run root with the default
  umask; fixture `miner-*.events.jsonl`, `profile-*.json`, `height-*.txt` are
  0644. Set `umask 077` in the scripts or write the config under `private/`.
- L8 Stop outcome text. `stop()` with an `orphan_unverified` record returns
  `succeeded "run stopped"` while the manifest is `completed_with_errors`
  (`engine.rs:1882-1895`); the outcome detail should carry the final state.
- L9 Proof provenance. The ignored proofs do not print the binary path/sha256,
  so the report's "approved binary 23961abe..." claim is unverifiable; print
  `hash_file(program)` in both proofs.
- L10 `LockRecord.heartbeat_at` is a misnomer now that there is no heartbeat;
  rename or document as `acquired_at`.
- Cross-packet (unchanged): the runner has no retention/`keep_slices`
  observation; warmup and statistics declarations must not be read as evidence
  that `window_n` history was retained. Any window-coverage assertion stays
  `inconclusive` until LP-1 reports retention.

## Integration gates (unchanged, explicitly not LP-3 claims)

Full pool TOML rendering for the inline `authority_secret_key` contract, the
live front evidence collector wiring, L2 fields not emitted by
`getblockchaininfo.consensus` (kept fail-closed), LP-4 netns backend and
capability probe, and any real-mining, SV2 interoperability, v2 credit,
retarget or payout claim. All are named in report §9 and §11 as gaps.

## Approval path

Fix D1 (and preferably L1-L3) with the named test, re-run `cargo test -p
spc-lab`, the Nix `fmt`/`clippy`/`test`/`ui-node`/package derivations on a
fresh source copy, and `run-once.sh --headless` with `--listen` under a
snapshot-polling client; publish the new manifest hash. Re-review will start
from the deadlock probe; the remaining R1 dispositions are accepted as verified
above and need no re-examination unless the touched files change.

---

# Third pass (2026-11-06): LP-3 R3, confined to D1 and the folded low items

Reviewer: Fable (subagent), read-only; no candidate edit, commit or subagent.
Snapshot: `/tmp/opencode/lab-lp3-r3-review-manifest.json`, 63 files, aggregate
`6a97dff090e9cea1b37258fc3e9c944cd5d9df1afd0f9689a896bfc769fe3290`
(re-verified 63/63 in the worktree; 17 files changed and 3 tests added versus
R2; `/tmp/opencode/LP3-r3-nix-src-final` matches 63/63).

## Verdict

**APPROVE (bounded LP-3 foundation)** for snapshot `6a97dff0...9fe3290`.

D1 is fixed at the root and no longer reproduces under the same probe; the
folded low items L1-L10 are implemented as described; no lock inversion was
introduced by the wait release or the snapshot barrier; the R1/R2 dispositions
remain intact. The approval covers the headless supervisor, durable command
journal, evidence store and snapshot/SSE contract, fixture-driven lifecycle,
and the thin browser, as a runner foundation. It does not cover, and the
candidate does not claim, real mining, SV2 interoperability, v2 credit,
retarget, payout, a rendered pool TOML, live front/node collectors,
interactive `add_miner` across restarts, or namespace placement; those remain
LP-3.1/LP-4 work as named in report §9 and §11.

## D1 verified

- Fix: `engine.rs:2352-2353` collects `supervisor.records()` into a local
  before the loop, so no supervisor guard is held across `evidence.append`.
  Audit of every `for ... in <mutex>.lock()` head in `crates/spc-lab/src`:
  none remain. Lock order is now evidence -> {supervisor, miners, manifest,
  state} only (`snapshot_with`), and no path takes evidence while holding any
  of those.
- Probe (`/tmp/opencode/Fable-LP3-r3-deadlock-probe.{py,log}`): the R2
  reproducer (8 snapshot-polling threads against `run --listen` with
  `--duration-secs 2`), 10 iterations: 10/10 exited 0 within the deadline;
  sampled manifests are `stopped` with only `stopped|exited` records. R2 hung
  4/8 on the same probe.
- Test `snapshot_stop_race::concurrent_snapshots_do_not_deadlock_a_stop_with_live_children`
  is real: fixture children, concurrent pollers with a completed-request count,
  bounded elapsed assertion, terminal records.

## Folded low items verified

- L1 journal replay: `CommandLog::Entry` carries `seq`; `entries()` and
  `in_flight()` sort by it (`command.rs:428-436`);
  `journal_replay_is_ordered_by_sequence_not_command_id` passes.
- L2 lagged subscriber: a full queue removes that subscriber
  (`evidence.rs:955-961`); after the receiver drains its buffered in-order
  events the HTTP stream answers the terminal `Disconnected` with the existing
  `event: resync` snapshot (`http.rs:361-373`). The gap stays
  `GapScope::Subscriber`; `http_subscriber_lag` asserts the resync and that a
  global `exists(...)` assertion still passes. No subscriber loss is treated as
  store loss.
- L3 long wait: `WaitHeight` releases `command_mutex` after the durable
  `begin` (`engine.rs:1243-1249`); the loop re-reads the manifest state and
  returns `cancelled` when the run leaves `Running|Preparing`
  (`engine.rs:1737-1746`). Lock check: the wait holds only `adapters.nodes`
  and takes manifest/state/miners briefly; `nodes` is taken nowhere else except
  `launch_nodes` under `start()`, which never runs concurrently with a wait, and
  nothing takes `nodes` while holding another engine lock. Mutating commands
  stay serialized. `long_height_wait_does_not_block_stop_and_is_cancelled`
  passes (stop under 5 s against a 30 s wait).
- L4/L5 documented as unsupported (ELF exec launchers, daemonising adapters)
  rather than claimed; `#!` scripts refused before launch. The direct ELF
  binaries this lab uses (`rbitcoin-node`, `spc-cpu-miner`, fixtures) are the
  supported class.
- L6 port lock root: `SPC_LAB_PORT_LOCK_ROOT`, else `$XDG_RUNTIME_DIR`, else
  `$TMPDIR`, else `/tmp` (`resource.rs:150-166`). Bound to record, not a
  universality claim: the kernel lock is shared only among processes that
  resolve the same root; two users (per-user `XDG_RUNTIME_DIR`) or two
  processes with different `TMPDIR` are in different scopes and are protected
  only by the OS bind probe plus the child-fd listener-ownership check. Nix
  sandbox builds resolve to `$TMPDIR=/build`, which is per-build.
- L7 `umask 077` in `run-once.sh`/`preflight.sh` and the fixture.
- L8 stop detail carries the final state (`run stopped (final state ...)`).
- L9 both ignored proofs print `program=` and `sha256=`; the recorded R3 logs
  show `spc-cpu-miner` `7e02b0fd...` (the current `lab-cpu-miner` debug build
  on this host) and `rbitcoin-node` `eb257af7...`. Identity is now auditable;
  mapping those hashes to the approved LP-2/L2 snapshots is the merge session's
  bookkeeping.
- L10 `heartbeat_at` documented as acquisition metadata.

## Evidence re-run and verified

- `cargo test -j2 -p spc-lab` here: 75 passed / 2 ignored, three times
  (`/tmp/opencode/Fable-LP3-r3-test*.log`); `http_protocol` alone four times.
- Recorded: `LP3-r3-final-nix.log` 75/0/2 with fmt and clippy clean;
  `LP3-r3-final-workspace-tests.log` 504/0/2; `LP3-r3-final-node.log` 10/0;
  `LP3-r3-final-run-once.log` final state `stopped`; real node and real
  control-only miner proofs pass with printed hashes.
- `nix build --dry-run` on `/tmp/opencode/LP3-r3-nix-src-final` for
  `packages.spc-lab` and checks `fmt`, `clippy`, `test`, `ui-node` reports
  nothing to build: all five outputs are realised for the exact R3 source.

## Non-gating notes for LP-3.1

- N1 One fixture process (`hold` postgres role, run token of the test binary)
  survived a passing `http_protocol::future_cursor_is_a_resync_never_a_silent_skip`
  during my first R3 suite run, which coincided with the 10-iteration deadlock
  probe spawning runs. Not reproduced in two further full suites and four
  targeted runs (0 leftovers each). The test did call `engine.stop()`, and the
  `TestDir` was removed, so the recorded state is unknown. Plausible paths are
  `process_gone` treating a transient `try_wait`/`/proc` read error as "gone"
  (`supervisor.rs:508-520`), or the 2 s + 500 ms force window expiring under
  load. Recommendation: distinguish probe error from exit (retry once, then
  record `orphan_unverified` with the error), and have the test harness assert
  no PID with its run token survives at exit. Fail-closed as is; no unrelated
  PID was signalled.
- N2 A second `stop_run` on a terminal run with all-terminal records re-enters
  `stopping` before returning to the same terminal state and rewrites
  artifacts; idempotent in effect, but a short-circuit on
  `Stopped|CompletedWithErrors` with no live record would avoid the transient.
- N3 The report's L6 line should state the scope bound above.
- Cross-packet (unchanged): no retention observation exists in the runner;
  window-coverage claims stay `inconclusive` until LP-1 reports
  `keep_slices`/`retain_windows` behaviour.

## Remaining integration gates (not reopened, not claimed)

Pool TOML rendering for the inline `authority_secret_key` contract; live
front/node evidence collectors; interactive `add_miner`/connection control
across restarts (pipe control is lost by design); L2 fields absent from
`getblockchaininfo.consensus` (fail-closed); LP-4 netns backend, placements and
capability probe; any real-mining, interoperability, credit, retarget or payout
claim.
