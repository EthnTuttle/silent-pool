# LP-3 headless supervisor/evidence and thin browser: implementation report

Date: 2026-11-05
Worktree: `/home/ethan/.local/share/opencode/worktree/ad8886/lab-runner`
Base: `b0ab4ca34fda07c61a6335d05f598eaacc69678c`
State: implementation complete for review; all changes uncommitted. No
commit, push, merge, reset or stash was made. No other worktree was edited.

Contract: approved L3 implementation contract at
`lab-contracts/docs/REGTEST-LAB-IMPLEMENTATION.md`, snapshot
`3edcd4e744149667b184f44bdf3833ba465e210671eb90912fb1db9b9b11ba25`, and the
Fable review at `docs/reviews/2026-11-05-lab-l3-fable-review.md`.

## 1. Result

This packet implements the usable runner foundation, not a proposal:

- a strict `scenario_version = 1` model with distinct nominal IDs, graph
  validation, unique listeners, per-pool storage ownership and an exact
  power-of-two work-profile check;
- a durable command log with payload conflict rejection, idempotent retry,
  preconditions and crash `unknown` reconciliation from the run manifest;
- append-only evidence with source/tip scoping, `unknown`, explicit gaps,
  reorg history retention, and no absence-equals-invalid path;
- an independent v2 credit oracle that recomputes the share hash and `d` from
  `job_issued`, with wrong-generation, wrong-credit, missing-job and
  below-target cases;
- a native process supervisor with PID start-time tokens, executable identity,
  per-run environment tokens, safe adoption/reap and process-group cleanup;
- unique ports, data directories, per-pool Redis instances and per-pool
  Postgres clusters, with requested and actual endpoints in the manifest;
- per-pool Noise authority key pairs written only to private mode-0600
  artifacts, public keys in the shareable manifest;
- a bounded loopback HTTP command API, resumable snapshot/SSE stream and thin
  HTML/JS browser over the same contract;
- scripted real child-process fixtures for tests and `run-once.sh`, plus an
  ignored real-`rbitcoin-node` baseline process proof.

The implementation does not claim real mining, an SV2 interoperability result,
a payout/credit integration or a retarget proof from fixtures.

## 1.1 R2 remediation (Fable snapshot `0f2a49c4...6ace517a`)

Core fixes are complete for B1-B7 and B9/B11-B18; B8 and B10 are the frozen
HTTP/browser coauthor packet (`docs/reviews/2026-11-05-lab-lp3-http-implementation.md`).

| Item | Disposition |
|---|---|
| B1 own-child zombie | `stop` reaps `try_wait`/zombie before token/exe classification; own exited children end `exited`, never `orphan_unverified`. Probe/test `an_exited_own_child_is_reaped_and_never_marked_orphan_unverified`; real driver stop result is asserted. |
| B2 false stop success | `stop` calls `stop_all` whenever verified records exist, regardless of `started`/state; no-record `created`/`failed` is `rejected_precondition`, terminal already-stopped is idempotent. `stop_after_crash_stops_adopted_processes_and_records_are_terminal`. |
| B3 crash inference | Pause/resume/budget/add-connection/switch are `unknown` after crash; add/remove miner use verified manifest process records only. `pipe_control_commands_are_unknown_after_a_crash`. |
| B4 manifest process sync | `start` failure and `stop` copy `supervisor.records()` before every final save; run manifest has no stale `running` records. |
| B5 entity state after adopt | Finished journal rebuilds desired entity/tombstone sets; observed worker state is `unknown`, miner `control_lost`/`exited`, and chain producers require live control. `pause_state_is_not_inferred_after_adoption`. |
| B6 command serialization | One whole-submit mutex covers begin/apply/finish; outcomes carry durable `journal_seq`; concurrent same-ID add produces one process. `concurrent_add_miner_is_log_serialized_and_creates_one_process`. |
| B7 snapshot barrier/future cursor | `EvidenceStore::snapshot_with` holds the evidence lock while the entity projection is read; future cursors return `ResyncRequired`; snapshot reports `resync_required`. |
| B8 browser reconnect/IDs | Frozen HTTP coauthor report; generic `message` SSE type, `Last-Event-ID`, manual cursor reconnect, worker `miner_id` controls. |
| B9 private modes | private dir 0700; runtime/scenario/commands/evidence/run manifest/logs/authority/raw spc_evidence files 0600, including atomic temp files. `artifacts_privacy` asserts modes. |
| B10 header bound | Frozen HTTP coauthor report; byte-budgeted 16 KiB header read before allocation. |
| B11 missing gates | Required subsidy and TDP cap now fail closed with `profile_requirement_missing`; only required fields gate. L2 does not emit subsidy-initial, witness or TDP-cap today, so a real scenario requiring them fails preflight instead of inventing values. Operational capability metadata remains a future registry integration. `preflight` test. |
| B12 loss scopes | Subscriber overflow is `GapScope::Subscriber` and does not make assertions inconclusive; durable write failures, driver cumulative dropped deltas (once per driver incarnation) and parse failures are `GapScope::Store`. Reordered IDs alone are not treated as loss; only the approved cumulative-delta and parse failure evidence is. |
| B13 run lock | Kernel `flock` on a 0600 file, no unlink stale-lock race, no TTL ownership. `run_lock` test. |
| B14 vocabulary | State strings use serde-compatible `as_str` methods (`blocked_on_height`, `rejected_precondition`, `adopted_control_lost`, `completed_with_errors`). |
| B15 dead API | Removed dead `family_error`, `hash_hex_meets_target`, `pool_storage_urls`, `submit_candidate`/`CandidateSubmission`, `mark_orphan`, `heartbeat`, unused `zeroize` copy; authority secret uses `non_secure_erase`. Added two-share/aggregate oracle test. |
| B16 signalling | Safe `rustix` `kill_process_group`; no shell `kill`; `#!` wrapper scripts are refused before launch. Tests cover wrapper refusal and zombie handling. |
| B17 readiness/ports | TCP readiness requires the listener socket inode to belong to the child; port allocation holds kernel-backed cross-run locks. Foreign-listener test. |
| B18 lifecycle | Primary CLI `run` uses durable StartRun/StopRun commands; stop is postcondition-based; `events` exits after replay on read-only attach; `serve` uses recorded manifest process rows; adopt is explicitly one-shot/read-only. |

**D1 (fixed): snapshot/stop lock inversion.** `emit_run_state` previously held
the supervisor guard in the `for record in self.supervisor.lock()...` temporary
while appending evidence, while the B7 snapshot barrier holds evidence and then
takes supervisor. The emission path now collects `supervisor.records()` into an
owned local and drops the guard before every append. A regression test starts
real fixture children, hammers `/snapshot` from eight threads and submits
`stop_run` under a bounded deadline; it passes with no hung run. No other
evidence-append path keeps supervisor/state/adapters/manifest locks across the
append.

Low-item disposition:

- L1 fixed: `CommandLog::Entry` stores `seq`; `entries()` and `in_flight()` sort
  by journal sequence, not command id. The replay-order test uses `z-add`
  before `a-remove`.
- L2 fixed: a full subscriber queue closes that subscriber; the HTTP stream
  answers the terminal disconnect with the existing `event: resync` snapshot,
  and the store records a subscriber-scoped gap. Global durable assertions stay
  valid. `http_subscriber_lag` covers the live HTTP path.
- L3 fixed: `wait_height` releases the whole-submit mutex after the durable
  begin, observes the run state each loop, and is cancelled by `stop_run`.
  `long_height_wait_does_not_block_stop_and_is_cancelled`.
- L4/L5 documented: ELF launchers that exec after spawn and daemonising
  adapters are not claimed supported; the direct ELF binaries used by the lab
  are. L6 uses `SPC_LAB_PORT_LOCK_ROOT`, then `XDG_RUNTIME_DIR`/`TMPDIR`, then
  `/tmp`. L7 run-once/preflight use `umask 077`; the fixture also starts with
  `umask 077`. L8 stop detail carries the final state. L9 proof tests print
  binary and SHA-256. L10 `heartbeat_at` is documented as acquisition metadata,
  not a TTL.

The deliberate R2 wire additions are additive: `EntitySnapshot.miner_id` on
worker rows (`{"kind":"worker", ..., "miner_id":"<miner>"}`; omitted elsewhere),
`Gap.scope` (`store|subscriber`), `CommandOutcome.journal_seq`, and the SSE
ordinary event type `message` with the kind inside the JSON payload.

## 2. Files

New packet-owned paths:

```text
crates/spc-lab/
  Cargo.toml
  src/lib.rs, ids.rs, wide.rs, scenario.rs, config.rs, secrets.rs
  src/command.rs, evidence.rs, oracle.rs, artifacts.rs, resource.rs
  src/manifest.rs, supervisor.rs, adapters.rs, links.rs, engine.rs, http.rs
  src/bin/fixture.rs                       # test/run-once child process fixture
  tests/fixtures/scenario_v1_one_pool_two_miners.json
  tests/fixtures/scenario_v1_two_pools.json
  tests/*.rs                               # 14 integration test files
bins/spc-lab/
  Cargo.toml, src/main.rs
  static/index.html, static/app.js, static/style.css
scripts/lab/
  run-once.sh, preflight.sh, fixture-config.json.in
```

Shared files touched locally, with merge notes in section 9:

```text
Cargo.toml        # members + spc-lab + bs58/num-bigint workspace deps
Cargo.lock        # resolved lock entries for the new packet
flake.nix         # local package/check/source-filter copy for validation
```

No existing pool, agent, accounting, store, protocol, spec, roadmap or
decision file was changed.

## 3. Scenario v1 and validation

`Scenario::from_json` reads `scenario_version` before strict deserialization.
Version 1 is the only accepted version; unknown top-level or nested fields are
rejected. The model uses newtypes for `RunId`, `NetworkId`, `NodeId`, `PoolId`,
`FrontId`, `IdentityId`, `WorkerId`, `MinerId`, `ConnectionId`, `JobId`,
`ShareId`, `CandidateId`, `CommandId`, `LinkId` and `ArkBackendId`; a
`NodeId` cannot be passed as a `PoolId`.

Structural and semantic validation covers:

- node -> network, pool -> network/node, front -> pool, connection -> front and
  identity, worker -> identity/connection/miner, and worker identity equality;
- duplicate IDs, duplicate resources, disjoint nonce ranges, no implicit link
  (every connection and pool-node relation must name a link);
- per-pool exactly one Redis instance and one Postgres database, matching
  owner, unique ports/database/role, relative data directories;
- loopback-only listener syntax, unique non-zero endpoint ports across every
  role, and port `0` left for the allocator;
- v2 work profile: `R = 2^k - 1`, `1 <= k <= 255`, `m_cfg = 2^j_cfg`,
  `j_cfg <= k - 1`, `m_cfg <= max_work_multiplier <= 2^32`,
  `max_window_target_units <= 2^64`, `window_n > 0`, v2 with
  `network_block_work` only, v1 with `share_work_legacy` only;
- profile validity against the declared `pow_limit_bits`
  (`reference_below_pow_limit`), issuance exponent and `Q(T)` against the
  configured hard cap;
- time policy (`node_header`/`mock`, `hold`/`roll`, explicit mock time), warmup
  declarations, statistics estimator/planned N/stopping rule/confidence, and
  rejection of a time-scoped assertion written as bare `is_canonical`.

The exact target helpers are in `wide.rs`: consensus `SetCompact`, 256-bit
comparison, `S(2^j)`, O(1) `issuance_exponent`, `Q(T)` with a wide intermediate,
and the approved `R = 2^255 - 1` fingerprint
`7fe9ed1a...d77c6b`.

## 4. Command API and durable outcomes

HTTP API (bounded, loopback-only; 64 concurrent connections, 16 KiB headers,
256 KiB body, read/write timeouts):

```text
GET  /api/lab/v1/runs
GET  /api/lab/v1/runs/{run_id}/snapshot?cursor=<event_id>
GET  /api/lab/v1/runs/{run_id}/events?after=<event_id>     # SSE
POST /api/lab/v1/runs/{run_id}/commands
GET  /api/lab/v1/runs/{run_id}/artifacts/shareable/...
GET  /api/lab/v1/health
GET  /, /app.js, /style.css
```

Only `shareable/` artifacts are served. `private/evidence.jsonl` and the
private scenario/runtime/authority files are not reachable through the HTTP
artifact route.

`POST /commands` takes the typed envelope:

```json
{
  "command_id": "cmd-...",
  "run_id": "run-...",
  "issued_at_wall": 1793920355,
  "actor": "operator",
  "kind": { "type": "pause_workers", "miner_id": "miner-1", "workers": [] }
}
```

Command kinds: `start_run`, `stop_run`, `add_miner`, `remove_miner`,
`add_connection`, `remove_connection`, `add_worker`, `set_worker_budget`,
`pause_workers`, `resume_workers`, `switch_pool`, `wait_height`, `assert`,
`kill_role`, `restart_role`, `partition_link`, `heal_link`, `inject_tx`.
`add_worker` is intentionally rejected: LP-2 protocol v1 defines workers inside
`add_connection`, so adding one to a live connection cannot be represented
without recreating the connection.

Outcome states are `accepted`, `rejected_precondition`, `running`, `succeeded`,
`failed`, `cancelled`, `blocked_on_height`, `unknown` and
`rejected_conflict`.

Durability rules:

1. `begin` appends and fsyncs the command payload before the effect.
2. Same ID and same payload returns the stored outcome and does not repeat the
   effect; same ID and different payload appends a conflict and returns
   `rejected_conflict`.
3. A failed precondition is a durable `rejected_precondition`, not a crash.
4. A command with no final record after a crash is `unknown` until
   `reconcile` reads the manifest. `StartRun`/`StopRun` reconcile from the
   manifest state; pipe-control entity commands reconcile only to `unknown`
   because their effect cannot be observed after a crash.
5. `stop` beats a later `start`; a removed miner ID cannot be added again in
   the same engine state. All commands are serialized through the log/engine.

`run_id` mismatch returns `rejected_precondition`. `--keep` leaves processes
for a later `spc-lab adopt`/`reap`.

## 5. Evidence, snapshots and oracle

Every event carries `run_id`, collector `event_id`, `source`, `source_cursor`,
actor, optional `command_id`, wall/mono/ingested times and a typed payload.
Implemented payload kinds: `job_issued`, `submitted`, `ingress_ack`,
`ingress_rejected`, `accounted`, `accounting_rejected`, `candidate`,
`canonical_at`, `node_rejected`, `settled`, `delivered`, `unknown`,
`event_lost`, `inconclusive`, `run_state`, `process`, `command`,
`link_policy`, `miner_event`, `spc_evidence`. `miner_event` preserves the LP-2
event name and payload; it never turns the driver's `accounted: "unknown"` into
accounting evidence. `spc_evidence` preserves an LP-1 front evidence record and
its private raw payload; it is emitted only by the explicit adapter below.

Rules enforced by the types and tests:

- `current_status(node, block)` returns `yes`, `no` or `unknown` from the latest
  `canonical_at` observation for that node/block; missing observation is
  `unknown`, never invalid.
- Only a `node_rejected` event exposes an authoritative rejection reason.
- Reorg observations are retained; a later `no` changes current status without
  deleting the earlier `yes`.
- A full subscriber queue produces an explicit `event_lost` gap and an
  `inconclusive` assertion; publication never waits for the subscriber.
- Snapshot with an old cursor returns `resync_required` and a new
  `snapshot_cursor`; SSE emits `event: resync` with that snapshot. There is no
  silent skip.
- Wide integers (`u128` work/credit, 256-bit targets) serialize as decimal or
  hex strings. Evidence `d`/`diff_sum` and target fields are strings.

LP-1 front evidence is captured by `spc_evidence.rs`, not by scraping a
front-specific log format into guessed fields. The adapter accepts the
`tracing` JSON line with `target = "spc_evidence"`, reads
`fields.event`/`fields.evidence`, and resolves `front_id` through an explicit
scenario-front-to-numeric map and `sp_id` through an explicit
scenario-identity map. `job_issued` becomes a typed `JobIssued` only when the
front mapping and all required fields are present; `accounted` is retained as a
private raw `spc_evidence` record with its mapped identity, never converted to
`accounted` from a miner ACK. `share_target` is recorded as the LP-1 big-endian 256-bit hex value
(`be256_hex`), and the approved `share_hash`, `extranonce` and byte-order
annotations are captured verbatim. Neither is silently byte-reversed and neither
is converted into business logic in the independent oracle. The source cursor
is a local line count, and every raw line is written to a private mode-0600
artifact. The full pool/node evidence collector wiring remains an integration
gate because LP-1 is not approved and its main pipeline uses a mock template.

The independent oracle (`oracle.rs`) does not import `spc-accounting`. It
recomputes SHA256d over the 80-byte header, compares the numeric digest with the
job's share target, takes `d = job_issued.m_issue`, optionally checks the target
against the exact family `S(2^j)`, and compares the exact per-identity sum with
`accounted.diff_sum`. Missing/duplicate job records are `unknown`; wrong
generation, wrong `d`, wrong job target and below-target hashes are mismatches.

## 6. Process, resources and adapters

`Supervisor` records for every process: PID, process group, `/proc/<pid>/stat`
field-22 start-time token, executable path and SHA-256, argv, run token, log
path, readiness and role/entity. Adoption verifies start-time token, run token
and executable before reattaching. A pipe-backed miner is recorded with
`control: pipe`; if its process survives, adoption marks it
`adopted_control_lost` and the run reports `stale_resources`, because stdin/
stdout cannot be reopened. A PID reuse, token mismatch or executable mismatch
is `orphan_unverified`; it is never killed. Graceful stop signals the
verified process group with `kill -TERM -- -<pgid>`, waits, re-verifies, then
uses `KILL` only within the deadline. `reap` reports unverified records instead
of killing them. Tests cover adoption, PID reuse, token/executable mismatch and
missing processes.

`plan_resources` allocates every node P2P/RPC/TDP, front SV2/SV1/JD, Ark, Redis
and Postgres endpoint and records the actual port before dependents start.
Every run gets a unique directory and owner lock; every pool gets its own Redis
port/data directory and its own Postgres cluster port/database/role. Two live
runs are tested with four unique Redis ports/directories and four unique
Postgres cluster ports.

Per-pool authority material: `PoolAuthority::generate` creates a distinct
secp256k1 authority pair per pool at prepare time, writes the base58check
secret and a merge fragment only under `private/` with mode 0600, and exposes
the public key to the front adapter context and the shareable manifest. The
test asserts distinct keys across two pools, mode 0600, and no secret in the
shareable manifest. This is the L1 handoff integration point; the secret is
never placed in process records.

Adapter contracts:

- **Node**: external adapter is a process plus either a fixture profile or an
  authenticated JSON-RPC `getblockcount`/`getblockchaininfo` reader. RPC auth
  accepts `bearer:<token>`, `bearer-file:<path>`, `basic:<user:pass>` or raw
  Basic. The runner compares effective profile fields
  (`pow_limit_bits`, floor, spacing, timespan, interval, retarget switch,
  min-difficulty, subsidy/halving, maturity, witness, TDP cap) and treats the
  fingerprint as advisory, never as the only evidence.
- **Miner**: `MinerControl` speaks the Fable-approved LP-2
  `spc-cpu-miner-control` version 1 protocol exactly (snapshot `5e27e918`,
  binary SHA-256 recorded in the test log). The envelope is
  `{ "protocol": "spc-cpu-miner-control", "version": 1, "command_id": "...",
  "operation": "...", "payload": {...} }`; the operations are
  `add_connection`, `remove_connection`, `set_worker_budget`,
  `pause_workers`, `resume_workers`, `switch_pool`, `snapshot`, `stop`.
  Results are `{ "kind": "command_result", "command_id", "outcome",
  "result"?, "error"? }`; asynchronous observations are separate
  `spc-cpu-miner-events` envelopes. Responses are correlated by `command_id`;
  events are appended as `miner_event` evidence with the driver incarnation and
  process-local cursor in the source cursor.
  Protocol v1 has no independent `add_worker` operation: a new worker is part
  of `add_connection`, so the runner rejects `add_worker` rather than faking it.
  Pipe-backed control cannot be reopened after a supervisor crash. On adoption
  a live miner is marked `adopted_control_lost`; a real driver's EOF shutdown
  makes it exited/unverified. The run reports `stale_resources` and never
  promises live hash workers recovered through a closed pipe.
- **Pool**: front processes use the pool adapter executable/args. Placeholders
  include `{sv2_port}`, `{sv1_port}`, `{jd_port}`, `{redis_url}`,
  `{postgres_url}`, `{authority_public_key}` and `{authority_secret_file}`.
  The private authority fragment is ready for the L1 config merge.
- **Storage/link**: Redis and Postgres adapters are process contracts with
  `{port}`, `{data_dir}`, `{database}`, `{owner_role}` and readiness probes.
  `links.rs` supplies the backend interface and a fixture backend; an unknown
  backend fails preflight with `link_backend_unavailable`. LP-4 owns the
  network-namespace backend.

Startup is readiness-based: Tcp or file readiness has a bounded deadline; a
child exit during readiness reports the role/entity and cleans up. `run-once`
leaves no fixture processes after a normal stop.

## 7. Thin browser

`bins/spc-lab/static` is a read-only-plus-miner-controls view:

- obtains `/api/lab/v1/runs`, then a snapshot and cursor;
- opens `EventSource` at `/events?after=<cursor>` and reconnects from the last
  event ID; `resync` events replace state with a fresh snapshot;
- renders run/entity state, source watermarks, explicit gaps, and a timeline;
- sends miner pause/resume/budget commands through `POST /commands`;
- does not start or stop the run, and closing/reopening the tab has no effect
  on run lifetime.

`spc-lab serve` attaches read-only: it does not acquire ownership, adopt a
process, reconcile commands or permit command effects. Live miner controls use
`spc-lab run --listen`; `adopt` is a one-shot verification, not a control
daemon.
The test opens HTTP, pauses a miner, closes the SSE stream, verifies the run is
still `running` and snapshot-readable, reconnects after the last event, and
checks no duplicate delivery. A small-capacity run verifies the SSE `resync`
path. A non-loopback bind is refused.

## 8. Tests, fixtures and results

Fixtures are real child processes: `crates/spc-lab/src/bin/fixture.rs` binds
loopback ports, writes readiness files, and speaks the JSONL control envelope.
They are explicitly tagged `fixture: true`; they are not miners, nodes or
pools, and no simulated share/hash is counted as live PoW.

Required LP-3 tests and results (all in `/tmp/opencode/LP3-r3-final-nix.log`):

| Command / test target | Result |
|---|---|
| `cargo test -p spc-lab --test scenario_v1` | 8 passed |
| `cargo test -p spc-lab --test command_idempotency` | 6 passed |
| `cargo test -p spc-lab --test evidence_states` | 5 passed |
| `cargo test -p spc-lab --test oracle_generation_credit` | 5 passed |
| `cargo test -p spc-lab --test snapshot_resume` | 4 passed |
| `cargo test -p spc-lab --test supervisor_adoption` | 7 passed |
| `cargo test -p spc-lab --test two_runs_isolation` | 1 passed |
| `cargo test -p spc-lab --test http_sse` | 3 passed |
| `cargo test -p spc-lab --test engine_restart` | 3 passed |
| `cargo test -p spc-lab --test height_wait` | 1 passed |
| `cargo test -p spc-lab --test artifacts_privacy` | 1 passed |
| `cargo test -p spc-lab --test pool_authority` | 1 passed |
| `cargo test -p spc-lab --test preflight` | 3 passed |
| `cargo test -p spc-lab --test lifecycle_stop` | 3 passed |
| `cargo test -p spc-lab --test command_concurrency` | 1 passed |
| `cargo test -p spc-lab --test driver_loss` | 1 passed |
| `cargo test -p spc-lab --test run_lock` | 1 passed |
| `cargo test -p spc-lab --test snapshot_stop_race` | 1 passed |
| `cargo test -p spc-lab --test http_subscriber_lag` | 1 passed |
| `cargo test -p spc-lab --test long_wait_stop` | 1 passed |
| `cargo test -p spc-lab --test http_protocol` | 7 passed |
| `cargo test -p spc-lab --test spc_evidence_adapter` | 3 passed |
| `cargo test -p spc-lab --test miner_protocol` | 1 passed |
| `cargo test -p spc-lab --lib` | 7 passed |
| `cargo test -p spc-lab` total | 75 passed, 2 ignored |
| `cargo test -p spc-lab --test real_node_proof -- --ignored` | 1 passed with `SPC_LAB_RBITCOIN_NODE` |
| `cargo test -p spc-lab --test real_cpu_miner_proof -- --ignored` | 1 passed with `SPC_LAB_CPU_MINER` and `SPC_LAB_CPU_MINER_CONFIG` |

Additional results:

| Command | Result | Log |
|---|---|---|
| `nix develop -c cargo fmt --all -- --check` | pass | `/tmp/opencode/LP3-r3-final-nix.log` |
| `nix develop -c cargo clippy -p spc-lab -p spc-lab-cli --all-targets -- -D warnings` | pass | `/tmp/opencode/LP3-r3-final-nix.log` |
| `nix develop -c cargo test -p spc-lab` | pass (75 + 2 ignored) | `/tmp/opencode/LP3-r3-final-nix.log` |
| `nix develop -c cargo test --workspace` | pass: 504 passed, 2 ignored across the workspace | `/tmp/opencode/LP3-r3-final-workspace-tests.log` |
| `scripts/lab/preflight.sh` | pass; scenario, capability and effective-profile gates evaluated without launch | `/tmp/opencode/LP3-r3-final-preflight.log` |
| `scripts/lab/run-once.sh --headless` | status 0, artifact run under `/tmp/opencode/LP3-r3-runs-final/20261106T060812Z-3503158` | `/tmp/opencode/LP3-r3-run-once.log` |
| `nix build .#packages.x86_64-linux.spc-lab .#checks...fmt ...clippy ...test ...ui-node` in a temp source copy | pass | `/tmp/opencode/LP3-r3-final-nix-builds.log` |
| `node --test bins/spc-lab/tests/*.test.cjs` | 10 passed, 0 failed | `/tmp/opencode/LP3-r3-final-node.log` |
| real `rbitcoin-node` baseline: launch, Bearer RPC profile/height, safe stop | pass; `network=regtest`, `pow_limit_bits=207fffff`, `no_pow_retargeting=true` | `/tmp/opencode/LP3-r3-final-real-node.log` |
| real `spc-cpu-miner` control-only driver: `--check-config`, snapshot, stop | pass; exact `spc-cpu-miner-control` v1 envelope | `/tmp/opencode/LP3-r3-final-real-cpu.log` |
| `git diff --check` / ASCII scan of new files | pass | local run |

The ignored real-node and real-driver tests are process proofs only. They do
not mine, submit a block or exercise the pool/front; the legacy baseline is
used because the approved L2 retarget profile's boundary scenario is a separate
integration gate.

## 9. Dependency status and real vertical-slice gate

This packet is blocked from claiming a real end-to-end slice until all of the
following are true:

1. **L1 merge.** L1 is Fable-approved at
   `fa532607fd20ce8f9dabcae1f552973e3c45c12b99ab7be1eb2f148e36056cd2` and not
   merged here. The runner generates distinct per-pool authority pairs and
   writes a private merge fragment, but a real `spc-pool` start still needs the
   L1 config/main merge notes (matching authority pair, front id, correlated
   outcome wiring). Do not reuse the published example pair for two pools.
2. **LP-1 integration.** The full v2 LP-1 bounded packet is Fable-approved at
   snapshot `c16032a78902e88b0157efed6780bc8b424e942b1c3bf46893d8fb09ae4708ee`
   in `lab-share-integration`; it is not merged into this worktree. The runner now has an explicit `spc_evidence`
   adapter with front/identity mappings and private raw capture, but it must be
   wired to the approved front log lifecycle, and its `main_sv2_pipeline` still
   uses a declared mock template. The approved `accounted` contract adds
   explicit raw-digest `share_hash`, full coinbase-order `extranonce` and byte
   order annotations; the adapter captures these verbatim in private evidence
   (no business logic is copied into the independent oracle). No v2 payout,
   window, type-32 or full-node claim is made here.
3. **LP-2 miner (approved).** Fable approved the bounded miner at snapshot
   `5e27e9180e765e8049cbcccbcadc38b24c85d402505108f8f20c320f73961250`; the
   final stdout contract guarantees strictly increasing delivered event IDs
   with `dropped_before` as a cumulative failed-publication total, while
   history-only `submissions_evicted`/`late_verdict` are not raw-capture loss.
   The runner consumes the exact v1 envelope, preserves unknown event names as
   `miner_event`, computes dropped-loss deltas once per driver incarnation and
   does not infer loss from reordered IDs. The real control-only proof used the
   approved binary (`sha256:7e02b0fdf7434edd081aa60ee421c7eef999418f56345a2f302a756cbab07169`) and passed
   `--check-config`/snapshot/stop. Native L1+Core one-block proof is a separate
   integration gate; no CPU share, acceptance or accounting claim is made
   here. Pipe-backed adoption remains explicitly stale because the driver
   exits on EOF.
4. **L2 node profile.** Fable approved the retarget profile at snapshot
   `d2f37b9113e8855245866289ed393043aed6e6153e8101375072cbb23624d499`.
   Flags and the `getblockchaininfo.consensus` field contract are stable;
   `--retarget-regtest` is 10 s/24 blocks, min-difficulty off, floor
   `0x1e7fffff` against validation pow limit `0x207fffff`, fresh-datadir
   contract. Several required semantic fields are still not emitted by that
   node, so B11 keeps them fail-closed. Only the legacy baseline was used in
   this packet's real process proof; retarget-boundary evidence remains an
   integration gate.
5. **LP-4 link backend.** The fixture link backend is not a sustained
   partition. Partition/no-bypass assertions must remain `inconclusive` until
   the netns backend and capability probe land.

Until those land, a run may use `fixture: true` adapters for supervision,
command, evidence, snapshot/SSE and cleanup tests only. It must not be described
as real mining, SV2 interoperability, v2 share credit, retarget proof or payout.

## 10. Merge notes (shared files)

- `Cargo.toml`: add workspace members `crates/spc-lab` and `bins/spc-lab`; add
  `spc-lab` path dependency, `num-bigint = "0.4"`, and
  `bs58 = { version = "0.4", features = ["check"] }`. The CLI package is named
  `spc-lab-cli` with binary name `spc-lab` to avoid the duplicate package name
  of the library.
- `Cargo.lock`: accept the resolved `spc-lab`/`spc-lab-cli` entries and the
  direct `bs58`, `num-bigint`, `rand`, `secp256k1` and `zeroize` edges. No
  upstream pin was changed.
- `flake.nix`: local validation copy adds a `spc-lab` package and updates the
  `fetchCargoVendor` hash to
  `sha256-aCaRV7LviAJ7Ptn02yK4S1jJBXSAfd7Xb1XdQMddaqU=`. It also uses an
  explicit `cleanSourceWith` filter because the new crates are untracked in
  this worktree; after the merge tracks them, restore `cleanSource` if the
  merge session prefers it. The package mapping is
  `cargoBuildFlags = [ "-p" "spc-lab-cli" "--bin" "spc-lab" ]`.
- `docs/SPEC.md`, `docs/STATUS.md`, `docs/MERGE-QUEUE.md`,
  `docs/OPERATIONS.md`: not edited. This packet adds no normative product
  behavior; L3 status and any operation section remain merge-session work.
- Real pool config: merge the private `pool-authority-<pool>.toml.fragment`
  (`[pool] authority_secret_key`/`authority_public_key`) into the operator's
  pool TOML. Per-pool run keys must remain distinct; private config and
  authority files must stay mode 0600; the shareable manifest must not carry a
  secret.
- `scenarios/lab/`: not created here. The canonical runnable fixtures live in
  `crates/spc-lab/tests/fixtures/` as the packet owns them. LP-4 owns
  `scenarios/lab/*`; `run-once.sh` accepts any JSON scenario path and rejects a
  `.toml` path with an explicit message until the scenario format is decided.

## 11. Non-claims

- Fixture processes are not a node, pool or miner; no simulated hash is live
  PoW.
- The real `rbitcoin` proof is a launch/RPC/shutdown proof on the legacy
  baseline, not retargeting, mining or block acceptance. L2's retarget profile
  is approved but its retarget-boundary scenario is not claimed here.
- `candidate` evidence is not node acceptance or canonical inclusion. Only a
  `canonical_at` observation at a named tip, with `unknown` preserved, supports
  a time-scoped canonical statement; only `node_rejected` supports "invalid".
- The HTTP/browser view is lab infrastructure, not a production UI or a
  mainnet privacy/economic test.
- This report does not claim LP-1 accounting, LP-2 CPU mining, L1
  interoperability, L2 retargeting, LP-4 partitions or payout completion. The
  LP-1 is approved but not merged; its mock-template pipeline is still an
  integration gate, and v2 type-32 remains unclaimed.
