# 2026-11-06 - LP-runtime host implementation

Status: implementation report for Fable review. Real host runs and the Nix
source/build gates were executed; the node's own full suite is **not** claimed
green. No commits, pushes, merges, resets, stashes, subagents or main-tree edits
were made.

## 1. Scope and result

This worktree combines the approved LP-1, LP-2, LP-3 and LP-4a inputs and adds a
built-in real-process runtime:

- renders actual rbitcoin, `spc-pool`, `spc-cpu-miner`, Redis and PostgreSQL
  configuration from a versioned scenario;
- allocates fresh per-run ports, storage directories, cookies, credentials and
  per-pool Noise authorities;
- generates write-once private SP identities with the approved crypto APIs;
- starts the approved node/pool/miner binaries and two independent pools with
  isolated Redis/PostgreSQL storage;
- tails the approved `spc_evidence` stream, observes the node tip, reads
  PostgreSQL/Redis privately, and scans canonical blocks;
- serves a browser catalog that builds the existing durable `AddMiner`,
  `SwitchPool`, `SetWorkerBudget`, `PauseWorkers` and `ResumeWorkers` commands;
- reports requested versus measured miner rates and separate
  submitted/ACK/rejected/unknown/block-qualified/stale/canonical counters;
- stops all owned processes cleanly and records the final command outcomes.

The source-bound two-pool run (packet2, Nix-built runner/pool/miner/scanner)
ended `stopped` with no owned process left running, both fronts asserted
accounted work, and the browser commands (including idempotent retry and payload
conflict) were exercised. The pre-fix run (live6) reached height 16; the
post-fix provisional replay (live8) reached height 159; both are retained only
as mechanism evidence because their manifests carry the old literal
`runtime`/`runtime` provenance fields. Packet2 is covered in section 5.

## 2. Approved inputs

Input manifests were re-verified against their source worktrees before copying
(57/57, 27/27, 63/63, 12/12 and 5/5 hashes matched).

| Input | Source worktree | Snapshot |
|---|---|---|
| LP-1 | `lab-share-integration` | `c16032a78902e88b0157efed6780bc8b424e942b1c3bf46893d8fb09ae4708ee` |
| LP-2 | `lab-cpu-miner` | `5e27e9180e765e8049cbcccbcadc38b24c85d402505108f8f20c320f73961250` |
| LP-3 | `lab-runner` (R3) | `6a97dff090e9cea1b37258fc3e9c944cd5d9df1afd0f9689a896bfc769fe3290` |
| LP-4a | `lab-link-preflight` | recipe `bce931514318110699a5aeb3ae0bdebd8d9204cc7e07b429b5961b9e9977036e` (`snapshot_sha256` null in the input manifest) |
| LP-live | `lab-live-e2e` (Fable-approved packet) | `aa3b37b67960071ec6bd4f09c7cd0ac83e1b526404231994c42dc331dc44ded2` |

The 151 non-shared component files plus the five approved LP-live files were
copied byte-for-byte with recorded modes. Source worktrees were left frozen.

The local L2 clearance is **CLEARED_WITH_RECORDED_BASELINE_LIMITATIONS** at
`d2f37b9113e8855245866289ed393043aed6e6153e8101375072cbb23624d499`. This report
makes no full-node-suite claim; only `sv2_tp_bootstrap` passed 6/6, and the
other candidate timing failures are recorded baseline limitations rather than
pending adjudication or an all-green claim.

## 3. Shared-file merge

`Cargo.toml`, `Cargo.lock` and `flake.nix` were combined consciously:

- workspace members are the LP-3 set plus `bins/spc-cpu-miner`;
- the lock is the union of LP-1/LP-2/LP-3 package blocks (565 packages);
  `cargo metadata` and the full workspace build pass;
- the LP-1 dev `channels_sv2` dependency remains in the lock;
- no upstream pin was bumped;
- the flake keeps LP-1/LP-3 checks and adds `spc-cpu-miner`, `spc-lab`,
  `spc-pool` (RPC feature) and `spc-scan` packages;
- the vendor hash was recomputed for the combined lock:
  `sha256-TET0F9jXQELPadNaPKBiPXgFNGPnU9uRaJWjkSAX52w=`.

## 4. Implementation

### 4.1 Scenario and resource rendering

`crates/spc-lab/src/runtime.rs` adds `Runtime::prepare`:

1. parse the versioned scenario;
2. load or create one private identity per declared slot;
3. load or create run credentials;
4. build the real adapters (node, pool, miner, Redis, PostgreSQL);
5. call the LP-3 `RunEngine::prepare` allocator;
6. render node cookies/datadirs, Redis configs, PostgreSQL clusters, pool TOMLs
   and miner TOMLs;
7. install the startup hook (create databases after storage/node readiness) and
   the miner provisioner (write a strict TOML for miners added later).

Rendered pool configuration uses the approved v2 profile: exact reference
target, power-of-two multiplier, network-block work window, SV1 disabled and
`extensions.share_accounting = false`. The pool binds the real SV2, JD and API
listeners. `FrontListen` gained an optional `api` endpoint for this; existing
scenarios leave it absent.

No operator-written TOML or authority fragment is merged. The pool authority is
read from the LP-3 private file and placed inline in the generated TOML.

### 4.2 Identity persistence and add-miner

Identity artifacts are `private/identity-<id>.json`, mode 0600, and contain the
SP address, `sp_id`, public keys and scan/spend keys. An existing artifact is
loaded and validated; a damaged artifact is an error and is never regenerated.
This is what makes a retried `AddMiner` safe after a partial crash.

The browser reads `GET /api/lab/v1/runs/<id>/catalog`, which exposes only
fronts/pools, declared addresses and used state, and live miner specs. It builds
the existing typed command payloads; the server does not accept an unprefixed or
implicit add. Switching pools uses the existing `switch_pool` command with the
miner's recorded connection id.

### 4.3 Collectors and observations

- **Pool evidence:** each pool log is tailed from byte 0. ANSI-coloured
  `spc_evidence` lines are parsed and fed to the LP-1 `SpcEvidenceAdapter`.
  `job_issued` becomes typed `JobIssued` evidence; `accounted` and multiplier
  steps stay exact private records.
- **Node:** authenticated `getblockcount`/`getblockhash` observations emit
  `canonical_at` only for the node's own reported tip.
- **PostgreSQL:** authenticated `psql` aggregate reads (`slice`,
  `slice_credit`, `sp_identity`) produce private observations; failures are
  recorded as errors.
- **Redis:** authenticated `PING` and `XLEN spc:front:events` reads.
- **Scanner:** in-process `spc_scan::scan_block` over canonical blocks fetched
  by RPC, for every private identity, with found output counts.
- **Metrics:** `runtime_metrics` reports the durable requested rate and the
  miner v1 `Snapshot` measured rate, plus submitted/ACK/rejected/unknown/
  block-qualified/stale/canonical counters. The note states explicitly that a
  block-qualified share is not a canonical block and that no causal
  hashrate-response claim is made.
- **Accounted-target consistency:** each accounted record is joined to the
  announced `job_issued` record by `(front, job_id, generation)`; its target
  and `m_issue`/booked `d` must match, and the recorded share hash must be at
  or below the announced target. The emitted topic is
  `accounted_target_consistency` with `consistent` / `inconsistent` / `unknown`
  and an explicit checked/not-checked scope. Missing, duplicate-conflicting or
  mismatched jobs cannot pass. It does **not** recompute the 80-byte header,
  derive the family target independently, verify DB credit sums, payout, or
  canonical inclusion; the full header/payout oracle is explicitly
  `inconclusive`. No miner ACK is used.

## 5. Real host proof

### Primary run (packet2, Nix-built binaries)

Source association: `code_snapshot_sha256 =
d75ab391b0ca1c8ab5b03af27dd5cf45a8a384ef8ac056a1ceeb2b2f456d8684`,
frozen before the Nix build and run. `base_revision =
b0ab4ca34fda07c61a6335d05f598eaacc69678c`; both are recorded in the run
manifest.

| Role | Store path / binary | SHA-256 | Derivation |
|---|---|---|---|
| runner | `/nix/store/y7ff12kjkf7xjh4fi7297w37dl4dww6k-spc-lab-0.1.0/bin/spc-lab` | `f8aa1d57...4936bc` | `/nix/store/faxv28gprg9jr99089j7vbyi3x74d55l-spc-lab-0.1.0.drv` |
| pool | `/nix/store/89pr88pxasf9fi52ial5559dpwxhmi7s-spc-pool-0.1.0/bin/spc-pool` | `aac1278f...6c481d` | `/nix/store/mzabmpcgpcckw1k318mqsgwx78rpql2c-spc-pool-0.1.0.drv` |
| miner | `/nix/store/ha780rm610zaivgyb4cc62lmviyb2wmp-spc-cpu-miner-0.1.0/bin/spc-cpu-miner` | `443de438...9ffa472` | `/nix/store/1ywggp10c5xhf1km65d2716h57l0znis-spc-cpu-miner-0.1.0.drv` |
| scanner | `/nix/store/qv4hr4xnbpk93cd00wcmwxx4aj7ww054-spc-scan-0.1.0/bin/spc-scan` | `f8000b43...bf14417` | `/nix/store/36frhck9b42w1xahh0k6hrcphlhgwdr7-spc-scan-0.1.0.drv` |
| node | `/tmp/opencode/rbitcoin-lab-retarget-profile/target/debug/rbitcoin-node` | `eb257af7...320f7c` | external pinned L2 source build (`d2f37b91...`); no Nix node output was available |

Redis/PostgreSQL are the Nix-store binaries recorded in the run manifest
(`834c2a45...`, `5be10c0c...`). The node is the one externally supplied
provenance exception; it is not presented as a Nix build.

Run directory: `/tmp/opencode/lab-runtime-runs/packet2/`.

- both pool/front pairs live with distinct Redis instances and PostgreSQL
  clusters/databases/roles/passwords;
- commands: `add_miner` succeeded, the same command id retry returned the
  stored outcome with no second process, the same id with a different budget
  returned `rejected_conflict`, and `switch_pool`, `set_worker_budget`,
  `pause_workers`, `resume_workers` all succeeded;
- `stop_run` succeeded, final state `stopped`; all 10 process records ended
  `stopped`/`exited`; no run process remained;
- assertions in `shareable/result.json`: `assert-jobs`,
  `assert-pool-evidence`, `assert-front-a1-accounted` and
  `assert-front-b1-accounted` all `passed`;
- `accounted_target_consistency`: status `consistent`, 810 accounted records,
  810 checked against their announced job, 0 inconsistent, 0 unknown, 267
  scoped job keys; `full_header_payout_oracle` explicitly `inconclusive`;
- PostgreSQL private reads: `spc_lab_runtime_a 57|57|1`,
  `spc_lab_runtime_b 224|349|3` (slice|slice_credit|identity counts);
- Redis: both instances `PONG`, front stream lengths 25 and 124;
- node scope: tip sampler, 25 tip observations / 25 distinct tip heights,
  max height seen 175; not continuous canonical coverage;
- scanner: cursor 189, 61 blocks scanned in the final pass, 183 outputs found,
  no skipped heights;
- 4,833 retained evidence events, 0 durable gaps.

### Prior provisional runs

| Run | Pool SHA-256 | Miner SHA-256 | Base / dirty | Use |
|---|---|---|---|---|
| live6 | `80679012...a2cd67` | `6e012307...f7176` | literal `runtime` / `runtime` | pre-fix two-pool mechanics only |
| live8 | `11a82a87...afbd` | `55ba2357...e8db` | literal `runtime` / `runtime` | provisional latency replay |

Those debug hashes are not reproducible and the manifests carry the old
literal defaults. They are retained as evidence of the mechanism, not as
source-bound proof. Packet2 is the source-bound run.

## 6. Pool stale-work latency fixes

Input: main-worktree `docs/reviews/2026-11-06-lab-stale-work-diagnostic.md`.
The diagnostic measured a dominant pre-publication delay: the front's blocking
Redis stream reader and the slice/block publisher shared one physical client,
so `XREADGROUP ... BLOCK` serialized `XADD`. It also found a secondary SV2
head-of-line wait where an unresolved front outcome delayed a new job on the
same connection.

Two bounded changes were integrated in this worktree only:

- `bins/spc-pool/src/main.rs`: `spawn_front_event_reader` now takes the Redis
  URL and opens its own `Streams`; it cannot accept a publisher clone. Group,
  consumer, pending replay, ACK-after-apply, bounded queues and
  slice-before-job publication order are unchanged.
- `bins/spc-pool/src/sv2_server.rs`: the accepted-share path keeps one pinned
  outcome future and the original fixed ten-second deadline while a nested
  `select!` delivers new jobs through the unchanged `send_job`. A new job does
  not reprice the pending share, restart its deadline, or create a second
  response. Lagged publication keeps the existing error policy; a closed job
  channel stops job delivery without extending the deadline.

Diff relative to the approved LP-1 source (`c16032a7`):

- `bins/spc-pool/src/main.rs`;
- `bins/spc-pool/src/sv2_server.rs`;
- `bins/spc-pool/tests/main_sv2_pipeline.rs`;
- `bins/spc-pool/tests/sv2_outcome_preemption.rs` (new);
- `crates/spc-store/tests/redis.rs` (new real-Redis regressions);
- `crates/spc-store/Cargo.toml` (test-only `futures-util`/`redis` dev deps).

Regressions:

- `crates/spc-store/tests/redis.rs` (real Redis): isolated reader does not
  delay publisher `XADD` (<250 ms); one raw shared client still reproduces
  >400 ms serialization; the published entry is pending until `XACK`.
- `bins/spc-pool/tests/sv2_outcome_preemption.rs`: with an old share outcome
  deliberately unresolved, a new-prevhash job and activation arrive first;
  the old share then resolves exactly once with its original sequence and
  credited work.
- `main_sv2_pipeline` now tolerates a new job arriving before a found-block
  share success and still requires exactly one success with original work.

Fail-first status: the SV2 preemption test was actually run against the old
blocking wait in an isolated scratch copy and failed at the new-job 2 s
timeout; the Redis isolated test was run with a deliberately shared client and
failed the prompt assertion. Those red runs are **not stored as cargo logs**
(the pool reviewer correctly noted this); the claims are source-reasoned plus
the transcript above, not a fabricated artifact. The green runs are recorded
in `/tmp/opencode/lab-runtime-pool-tests.log` and
`/tmp/opencode/lab-runtime-redis-tests.log`.

Qualified replay table, generated by
`scripts/lab/analyze-runtime-latency.py` (stored with the run). Clocks:
node `tip:` log line after tip commit (display hash reversed to the pool's
internal order); pool `job_issued.issued_at` wall ms; miner
`occurred_at_wall_ms` wall ms. Collector `ingested_at` is not used. The first
two tips are excluded as startup; raw counts include them.

| Run | tip -> first issue (qualified) | issue -> activation | submitted / stale | block-qualified | outcome-unknown |
|---|---|---|---|---|---|
| live6 (pre-fix, provisional) | n14 p50 760 / p95 1,899 / max 2,012 ms | n92 p50 1 / p95 42 / max 587 ms | 410 / 281 | 99 | 0 |
| live8 (post-fix, provisional) | n157 p50 13 / p95 15 / max 17 ms | n92 p50 1 / p95 33 / max 43 ms | 663 / 51 | 168 | 0 |
| packet2 (Nix-bound) | n194 p50 4 / p95 7 / max 8 ms | n99 p50 0 / p95 38 / max 42 ms | 792 / 51 | 196 | 0 |

Raw tip->issue maxima including startup were 8,711 / 8,721 / 294 ms
respectively. The two-pool issue->activation join only uses unambiguous
`(prev_hash, job_id)` pairs; 504 packet2 activations were unmatched/ambiguous
across fronts and are not claimed. The three runs have different chains and
load; this is directional mechanism evidence, not a causal hashrate-budget or
stale-rate proof. Requested/measured rates remained separate (packet2:
20/19, 20/19, 60/59 h/s).

**Approval boundary.** The combined runtime plus pool binary is materially
changed and is not covered by the old LP-1/LP-2/L2 approval snapshots. The
packet2 run is source-bound but provisional for host acceptance. Final
acceptance requires the new manifest/approval binding below, not an
unstructured `APPROVE` substring.

### R1-R4 and N dispositions

- **R1 fixed:** `accounted_target_consistency` replaces the generic label;
  status is `consistent|inconsistent|unknown`, scope lists checked/not-checked,
  join is `(front, job_id, generation)` with target/m_issue/booked-d equality,
  and conflicting/missing jobs cannot pass. The full header/payout oracle is
  `inconclusive`. Tests cover tampered self-consistent records, cross-front
  same-id disambiguation, duplicate/conflicting jobs and missing jobs.
- **R2 fixed:** both scenarios assert `exists(job_issued)`,
  `exists(spc_evidence && canonical_at)`, and one `accounted_work(front=...)`
  per declared front. Packet2 `result.json` shows all assertions passed and
  includes the R1 consistency summary.
- **R3 fixed:** packet2 used Nix-built runner/pool/miner/scanner store paths,
  derivations and hashes, with `base_revision` and `dirty_tree_hash` set to the
  real frozen values. The node is the explicit external pinned-L2 exception.
  The latency table names clocks and comes from the stored script; fail-first
  red runs are stated as unrecorded/source-reasoned, not fabricated.
- **R4 fixed:** the node observer is labelled a tip sampler, records distinct
  tip heights versus max height, and rechecks the last 8 observed tips. A
  displaced hash emits `canonical_at(no)`; a failed read emits nothing new
  (unknown). It does not claim full canonical coverage or reorg tracking.
- **N1 fixed:** the node adapter declares a fixed known capability set.
- **N2 fixed:** identity artifacts are re-derived from the stored private keys
  on load; parseable public-field tampering is rejected.
- **N3 fixed:** miner handles are cloned out of the map before blocking
  snapshot requests, so evidence/snapshot locks are not held across a 10 s pipe
  wait.
- **N4 fixed:** collectors observe `Running|Preparing` and quiesce on stop;
  post-stop connection failures are not emitted as observations.
- **N5 fixed:** scanner failures record explicit `skipped_heights` and keep a
  contiguous retry cursor instead of advancing past a failed height.
- **N6 fixed:** PostgreSQL database/role identifiers are validated as
  `[a-z_][a-z0-9_]*` (<=63 bytes) at scenario time; observers receive the raw
  secret, not a URL reparse.
- **N7 fixed:** job counting is scoped by `(front, job_id, generation)`.
- **N8 fixed:** stale port-warning text now describes the kernel port locks.
- **N9/N10/N11 recorded** in section 8.

## 7. Verification gates

| Gate | Result | Evidence |
|---|---|---|
| `cargo fmt --all -- --check` | pass | `/tmp/opencode/lab-runtime-nix-develop.log` |
| `cargo clippy -p spc-store -p spc-pool -p spc-lab -p spc-lab-cli --all-targets -- -D warnings` | pass | `/tmp/opencode/lab-runtime-clippy-r1.log` |
| `cargo test -p spc-lab -p spc-store -p spc-pool` | 353 passed, 0 failed | `/tmp/opencode/lab-runtime-core-tests.log` |
| `cargo test -p spc-pool` | 236 passed, 0 failed | `/tmp/opencode/lab-runtime-pool-tests.log` |
| real-Redis reader/publisher tests (`SPC_TEST_REDIS_URL`) | 4 passed, 0 failed | `/tmp/opencode/lab-runtime-redis-tests.log` |
| `cargo test --workspace` | 609 passed, 0 failed | `/tmp/opencode/lab-runtime-workspace-tests.log` |
| `node --test bins/spc-lab/tests/*.test.cjs` | 14 passed, 0 failed | section 8 recipe |
| approved LP-live driver `--self-test` | 17 passed | `/tmp/opencode/lab-runtime-live-selftest.log` |
| approved LP-live `--check-scenario` (`baseline`, `retarget`) | both validated | `/tmp/opencode/lab-runtime-live-check-*.log` |
| `scripts/lab/run-once.sh --headless` | status 0, final `stopped` | `/tmp/opencode/lab-runtime-run-once.log` |
| Nix `fmt`, `clippy`, workspace tests, `spc-lab`, `spc-pool`, `spc-cpu-miner`, `spc-scan`, `ui-node` from a filtered source copy | all pass | `/tmp/opencode/lab-runtime-nix-builds.log` |
| real host run, one pool | clean stop | live5 artifacts |
| real host run, two isolated pools | clean stop | live6 artifacts (pre-fix, unbound) |
| real host replay after pool latency fixes | clean stop, height 159 | live8 artifacts (provisional, unbound) |
| source-bound two-pool run, Nix binaries | clean stop; both front accounted assertions passed; 0 gaps | packet2 artifacts |
| stored latency analysis | qualified n194 p50 4 / p95 7 ms | `scripts/lab/analyze-runtime-latency.py`, `/tmp/opencode/lab-runtime-latency-packet2.json` |

The frozen source copy is `/tmp/opencode/lab-runtime-final-src`; it was
captured after `code_snapshot_sha256 = d75ab391...` and used for the Nix build
and packet2 run. The vendor hash is
`sha256-TET0F9jXQELPadNaPKBiPXgFNGPnU9uRaJWjkSAX52w=`. Nix output and derivation
paths are in `/tmp/opencode/lab-runtime-packet2-provenance.json` (regenerate
the record for the final store paths listed in section 5).

## 8. Honest limits and non-claims

- The node's full suite is **not** claimed green. Only `sv2_tp_bootstrap`
  passed 6/6. L2 is CLEARED_WITH_RECORDED_BASELINE_LIMITATIONS at
  `d2f37b91...`; the other candidate timing failures are recorded limitations,
  not an all-green claim.
- No controlled retarget response or causal hash-budget claim is made.
  The baseline node profile in these runs has `no_pow_retargeting=true`.
- No SV1 interoperability, TDP/ladder production, Ark settlement, wallet,
  production payout, or mainnet claim is made.
- Evidence comparisons that need a full window are not treated as passed by
  this runtime.
- `redis-server` clears its environment and rewrites its process title. The
  supervisor accepts its missing run token only for the `redis` role, while
  still checking kernel start time, executable path and executable hash. This
  is a deliberate, recorded ownership exception.
- The runtime runs in the host namespace. Namespace placement, address
  publication and controller RPC transport are unsupported and not emulated.
- No approval parser was copied from LP-live. The runtime does not accept
  "approval text anywhere" or "hash anywhere"; it reads no review record for
  runtime decisions.
- Both pools render the same throwaway payout descriptor. Ledgers and
  storage are independent; the coinbase destination is shared for lab
  convenience. "Two independent pools" therefore does not mean two payout
  destinations.
- `/catalog` publishes SP addresses on the loopback-only control surface.
  They are public keys, not secrets, and are not copied into `shareable/`.
- The 2026-11-06 lab-runner and pool-latency changelog entries are kept as
  decision/changelog fragments, not in the normative SPEC frontmatter; the
  merge session decides the final SPEC/OPERATIONS splice.

## 9. Reproduction recipes

Build the binaries (the pool must include the RPC feature for the rendered
`template.source = "rpc"`):

```sh
CARGO_BUILD_JOBS=2 cargo build -p spc-lab-cli -p spc-scan
CARGO_BUILD_JOBS=2 cargo build -p spc-pool --features rpc
CARGO_BUILD_JOBS=2 cargo build -p spc-cpu-miner
```

Two-pool run (Nix-built binaries from the frozen source copy):

```sh
/nix/store/y7ff12kjkf7xjh4fi7297w37dl4dww6k-spc-lab-0.1.0/bin/spc-lab runtime \
  --scenario /tmp/opencode/lab-runtime-final-src/crates/spc-lab/scenarios/lab-live-v2-two-pools.json \
  --run-dir /tmp/opencode/lab-runtime-runs/packet2 \
  --node /tmp/opencode/rbitcoin-lab-retarget-profile/target/debug/rbitcoin-node \
  --pool /nix/store/89pr88pxasf9fi52ial5559dpwxhmi7s-spc-pool-0.1.0/bin/spc-pool \
  --miner /nix/store/ha780rm610zaivgyb4cc62lmviyb2wmp-spc-cpu-miner-0.1.0/bin/spc-cpu-miner \
  --scanner /nix/store/qv4hr4xnbpk93cd00wcmwxx4aj7ww054-spc-scan-0.1.0/bin/spc-scan \
  --postgres-bin-dir /nix/store/vync24csmxy5saql5ah77vkn59yg2x5h-postgresql-17.11/bin \
  --redis-server /nix/store/nql1lir6650xigvf43n11zz4hh0lbw9z-redis-8.10.3/bin/redis-server \
  --redis-cli /nix/store/nql1lir6650xigvf43n11zz4hh0lbw9z-redis-8.10.3/bin/redis-cli \
  --base-revision b0ab4ca34fda07c61a6335d05f598eaacc69678c \
  --dirty-tree-hash d75ab391b0ca1c8ab5b03af27dd5cf45a8a384ef8ac056a1ceeb2b2f456d8684 \
  --listen 127.0.0.1:0 --duration-secs 120
```

Latency table and manifest:

```sh
python3 scripts/lab/analyze-runtime-latency.py \
  /tmp/opencode/lab-runtime-runs/packet2 \
  --json /tmp/opencode/lab-runtime-latency-packet2.json
python3 scripts/lab/build-runtime-manifest.py \
  --output /tmp/opencode/lab-runtime-review-manifest.json
```

Commands exercised through `POST /api/lab/v1/runs/<run>/commands`: `add_miner`,
`switch_pool`, `set_worker_budget`, `pause_workers`, `resume_workers`,
`stop_run`. The browser form is in `bins/spc-lab/static/`.

Node and workspace suites:

```sh
CARGO_BUILD_JOBS=2 nix develop --command bash -lc \
  'cargo fmt --all -- --check &&
   CARGO_BUILD_JOBS=2 cargo clippy -p spc-lab -p spc-lab-cli --all-targets -- -D warnings &&
   CARGO_BUILD_JOBS=2 cargo test -p spc-lab'
CARGO_BUILD_JOBS=2 cargo test --workspace
node --test bins/spc-lab/tests/*.test.cjs
```

Nix package/check gate (filtered source copy; do not include `.git`, `target` or
`result`):

```sh
tar --exclude=.git --exclude=target --exclude=result -cf - . |
  tar -xf - -C /tmp/opencode/lab-runtime-final-src
cd /tmp/opencode/lab-runtime-final-src
CARGO_BUILD_JOBS=2 nix build \
  .#packages.x86_64-linux.spc-lab \
  .#packages.x86_64-linux.spc-pool \
  .#packages.x86_64-linux.spc-cpu-miner \
  .#packages.x86_64-linux.spc-scan \
  .#checks.x86_64-linux.fmt \
  .#checks.x86_64-linux.clippy \
  .#checks.x86_64-linux.test \
  .#checks.x86_64-linux.ui-node --no-link --print-out-paths
```

## 10. LP-4 handoff

The following interface is stable for the namespace co-owner:

- owned paths: `<run_dir>/private`, `<run_dir>/logs`, and the per-run storage
  directories recorded in `run-manifest.json`;
- all node/pool/front/miner entities and their allocated endpoints are in the
  manifest; generated configs do not need to be parsed for placement;
- miner control stays the approved `spc-cpu-miner-control` v1 JSONL over
  stdin/stdout;
- the supervisor still refuses re-exec wrappers and verifies run tokens (with
  the explicit Redis exception);
- the pool has no normal stdin control pipe;
- PostgreSQL cannot run as root inside a user namespace.

Stable Rust API for the namespace/topology author (baseline `c9dfa986` plus the
bounded R1-R4/N changes; exact diff is the manifest):

- `RunEngine::scenario()` and `RunEngine::resources_manifest()` expose the
  declared entities, endpoints and storage without parsing generated configs;
- `RunEngine::runtime_catalog()` / `GET /catalog` expose fronts, pools, public
  SP addresses with `used`, and live miner specs; no secret material;
- `RunEngine::runtime_miner_specs()` and `runtime_miner_snapshots()` are the
  miner control read surface (snapshots now clone handles before blocking);
- `RunEngine::set_startup_hook()` runs after storage/nodes and before fronts;
  `set_miner_provisioner()` writes a strict TOML before any miner spawn;
- `RunEngine::refresh_artifacts()` is the explicit late-observer projection;
  `emit_canonical()` preserves historical observations;
- `Runtime::network_handoff()` and `node_endpoints()` expose owned paths and
  endpoints without starting anything.

Namespace placement, address publication and controller RPC/stdio transport
remain unsupported in this slice. `daemonising` node/pool behaviour and the
PostgreSQL userns limitation are integration gates, not hidden successes.

## 11. Review packet

- Frozen code manifest: `/tmp/opencode/lab-runtime-code-manifest.json`
  (`code_snapshot_sha256 =
  d75ab391b0ca1c8ab5b03af27dd5cf45a8a384ef8ac056a1ceeb2b2f456d8684`).
- Final annotated manifest: `/tmp/opencode/lab-runtime-review-manifest.json`
  (built by `scripts/lab/build-runtime-manifest.py`; the full aggregate includes
  this mutable report, while `code_snapshot_sha256` is the pre-build freeze).
- Nix provenance/paths:
  `/tmp/opencode/lab-runtime-packet2-provenance.json`.
- Latency script/output:
  `scripts/lab/analyze-runtime-latency.py`,
  `/tmp/opencode/lab-runtime-latency-packet2.json`.
- Run artifacts: `/tmp/opencode/lab-runtime-runs/packet2/`.

The combined binary is not covered by an existing approval. A new exact
manifest/approval binding (resolved against the manifest hash, not an
`APPROVE` substring) is required before acceptance; the approved LP-live
driver's 17 self-tests still cover its binding parser, and this packet does not
bypass them.
