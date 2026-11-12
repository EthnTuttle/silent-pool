# LP-runtime host integration: Fable review

Date: 2026-11-06
Reviewer: Fable (subagent), read-only. No candidate edit, commit, push or
subagent. Probe outputs are under `/tmp/opencode/Fable-runtime-*`.
Candidate: `lab-runtime` worktree, base `b0ab4ca34fda07c61a6335d05f598eaacc69678c`.
Snapshot: `/tmp/opencode/lab-runtime-review-manifest.json`, 172 files, aggregate
over `{base, files}` (full records including `source`/`change`), sorted compact
JSON = `c9dfa986c2f6ad0daca44912906a0a2ec94129c25d465eee246f19a43ccd543d`.
Scope of this review: runtime, rendered config, collectors, UI, supervision,
shared Cargo/Nix, docs, and the overall host gate. The pool/store delta
(`bins/spc-pool/src/main.rs`, `sv2_server.rs`, two pool tests,
`crates/spc-store/{Cargo.toml,tests/redis.rs}`) was reviewed separately and
is APPROVED bounded at this same aggregate
(`docs/reviews/2026-11-06-lab-runtime-pool-fable-review.md`); nothing below
reopens it.

## Verdict

**REQUEST_CHANGES (focused, bounded)** for snapshot `c9dfa986...cd543d`.

The runtime is real and works: it renders node/pool/miner/Redis/PostgreSQL
configuration from a strict scenario, allocates isolated per-run storage with
per-pool credentials and Noise authorities, starts the approved binaries,
drives two independent pools with distinct ledgers, exposes the LP-3 durable
command journal to a browser catalog, collects pool/node/PostgreSQL/Redis/
scanner evidence, and stops cleanly. I reproduced a two-pool run end to end
with my own commands (idempotent retry and payload conflict included) and a
clean duration stop. The LP-3 foundation fixes (deadlock, snapshot barrier,
stop idempotence, journal replay) survive under collector concurrency in that
run and in the recorded live runs.

Four items block a bounded host approval because they concern the
truthfulness of the produced evidence and the binding of the host proof to the
frozen source, not the mechanics. All are small: one evidence label and a
join, two scenario assertion lines, and report corrections plus a rerun with
reproducibly built binaries. Future scope (namespace placement, causal
retarget response, fee/witness coverage, full-window payout verification) is
not reopened here and is not demanded for the bounded approval.

## What was verified

- Manifest: 172/172 hashes match the worktree; aggregate reproduces. All 139
  `unchanged-from-approved-input` files hash-match their frozen source
  worktrees (LP-1 51, LP-2 24, LP-3 R3 48, LP-live 4, LP-4a 12). The LP-4a
  `snapshot_sha256: null` in the manifest `inputs` block is a bookkeeping gap
  only: the 12 files match `lab-link-preflight` and
  `/tmp/opencode/lab-lp4a-review-manifest.json` reproduces `bce93151...`.
- Nix: `/tmp/opencode/lab-runtime-nix-src` matches the manifest on 167/172
  files; the 5 differences are `docs/SPEC.md`, the two 2026-11-06 pool
  changelog/decision records, the implementation report, and
  `testing/regtest-lab-v2-e2e.py` (the later removal of an unreachable
  `return`). Every Rust, Cargo and `flake.nix` file is identical, and the copy's
  `flake.nix` carries vendor hash `sha256-TET0F9jXQELPadNaPKBiPXgFNGPnU9uRaJWjkSAX52w=`.
  `nix build --dry-run` on the copy for `spc-lab`, `spc-pool`, `spc-cpu-miner`,
  `spc-scan`, `fmt`, `clippy`, `test`, `ui-node` reports nothing to build (all
  nine derivations incl. the vendor drv are realised); `nix log` of the
  workspace-tests derivation shows 604 passed / 0 failed; clippy log has 0
  warnings.
- Recorded logs: workspace 604/0/2, pool 236/0, spc-lab+spc-store 112/0/2,
  real-Redis 4/0, nix develop spc-lab 76/0, LP-live self-test 17 OK (run at
  04:22 against the edited driver), run-once `stopped`. Re-run here: Node
  helper tests 14/14; `runtime::tests::parses_ansi_coloured_pool_evidence_lines`
  passes.
- Rendered pool TOML (live8 `private/pool-front-{a1,b1}.toml`): v2 profile
  values equal the scenario (`R = 2^255-1`, `m = 2`, `window_n = 8`,
  `network_block_work`, `pow_limit_bits = 0x207fffff`, `max_slice_shares = 4`),
  SV1 off, `share_accounting = false`, real `[jd]`/`[api]` listeners,
  `front_id` 1 and 2, inline authority pair from the LP-3 private file. The two
  files differ only in listeners, `front_id`, authority and credentials. Miner
  TOML is the strict v1 control-only shape.
- Isolation: distinct Redis ports/dirs/`requirepass`, distinct PostgreSQL
  clusters/databases/roles/passwords (scram), node cookie + Basic RPC auth,
  all under `private/` 0700 with 0600 files (identities, secrets, cookies,
  TOMLs, manifest, logs). Credentials never appear in argv (`PGPASSWORD`,
  `REDISCLI_AUTH`; the pool reads its own TOML). Shareable manifest carries
  public authority keys only; the forbidden-key scan still guards it.
- Supervision: Redis run-token exception is `None if role == "redis"` only
  (`supervisor.rs:338`); start-time tick, executable path and hash are still
  required; a present-but-different token still fails. Clean stop observed in
  live6/live8 and my run (10 records `stopped|exited`).
- Browser: `/catalog` exposes fronts, declared addresses with `used`, live
  miner specs and defaults; `addMinerCommand`/`switchPoolCommand` build the
  existing typed `add_miner`/`switch_pool` payloads and post them through the
  same `/commands` journal; closing the tab has no effect on the run.
- My probe (`/tmp/opencode/Fable-runtime-probe.py`, run dir
  `/tmp/opencode/Fable-runtime-runs/two-pools`, 90 s, fresh debug binaries,
  Nix-store Redis/PostgreSQL, explicit paths): `add_miner miner-c` succeeded,
  the same `command_id` retry returned the stored outcome without a second
  process, the same id with a different budget produced a `conflict` record,
  `switch_pool`/`pause`/`resume`/`set_worker_budget` succeeded, duration stop
  `succeeded (final state stopped)`, 0 run processes left. Evidence: 1161 typed
  `job_issued`, 1513 `spc_evidence` (accounted a1 124 / b1 1385), 46
  `canonical_at`, 0 gaps; PostgreSQL `57|57|1` vs `529|949|3`: two distinct
  ledgers with real work on both pools.
- Latency claim cross-check (live8): from the node api-log `submitblock`
  completion (node clock) to the first `job_issued` carrying the new
  `prev_hash` (pool `issued_at`), n = 162, p50 12 ms, p95 15 ms, max 848 ms.
  The report's p50 13 / p95 16 are consistent; its 8,721 ms max is not
  reproduced by this method (it likely includes the startup job).

## Blockers (ranked)

### R1 (medium) - `oracle_accounted_check` labels a self-consistency check `verified`

`crates/spc-lab/src/runtime.rs:366-448` compares each pool `accounted` record's
own `share_hash` (raw digest, read little-endian) against that same record's
`share_target`. It does not join to `job_issued` by (front, job_id,
generation), does not recompute the 80-byte header, does not check the target
against the family `S(2^j)` for `m_issue`, and does not compare `d`/credit
sums; `oracle::check_credit` (LP-3) and the approved `testing/lab-live/oracle.py`
are present but unused by the runtime. The emitted record says
`status: "verified"` with `shares_checked: 633`. The report (§6) says this is
"a separate share-target recheck, not a claim of full approved-oracle
equivalence", but the artifact itself says `verified`, which a reader of the
evidence will take as credit verification. Per §7.2.1 and the review brief,
hash <= target alone is not credit or header proof.
Fix: rename the topic/status to the exact scope (for example
`accounted_target_consistency` with `consistent|inconsistent|unknown`), add a
`scope` field stating what was and was not checked, and add the join that the
evidence does support: the `accounted` record's `share_target`/`m_issue`/
`generation` must equal the `job_issued` record for the same front and
`job_id`; a missing or mismatched job is `unknown`/`mismatch`, never
`verified`. Keep the full oracle as a separate, explicitly `inconclusive`
item until headers can be recomputed.
Test: a fabricated accounted record whose `share_target` differs from its
`job_issued` must not count toward a passing status.

### R2 (medium) - Real scenarios assert nothing real

`crates/spc-lab/scenarios/lab-live-v2*.json` carry the fixture assertion
`exists(run_state)` only (`assertions` in both files), so the shareable
`result.json` for a real two-pool run reports `assert-started: passed` on an
assertion the runner satisfies by emitting its own state. The brief asked for
meaningful real work on both pools with no fixture assertions. The evidence
exists (typed `job_issued`, `spc_evidence` per front, `canonical_at`,
PostgreSQL counts), but no declared assertion reads it.
Fix: declare at least `exists(job_issued)`, `exists(spc_evidence)` and
`exists(canonical_at)` (and, if the predicate language is extended, per-front
`accounted` presence) so an empty or one-pool run fails or is inconclusive
instead of passing; surface R1's consistency result as an assertion-like line
in `result.json`.

### R3 (medium) - Host proof is not bound to the frozen source; report tables are stale

- Report §5 lists one binary table (`spc-pool 80679012...`,
  `spc-cpu-miner 6e012307...`); those are live6's hashes. live8, the run the
  latency claims rest on, recorded `spc-pool 11a82a87...` and
  `spc-cpu-miner 55ba2357...` (`/tmp/opencode/lab-runtime-runs/live8/run-manifest.json`).
  The miner source is unchanged between the two, so the hash change shows that
  debug builds here are not reproducible: rebuilding the pool and miner from
  the identical frozen tree during this review produced yet other hashes
  (`3f18280d...`, `f6c1deaa...`; the on-disk pool before my rebuild was
  `e1e7dc73...`). A debug-binary hash therefore cannot bind a live run to the
  manifest. `base_revision`/`dirty_tree_hash` in the live manifests are the
  literal defaults `"runtime"`.
- Report §3 states vendor hash `sha256-wIAWKFllu0tj...`; `flake.nix:45` and
  the Nix source copy use `sha256-TET0F9jXQELP...`. §2 and §8 still say the
  node's other failures "remain under Fable adjudication" although L2 code is
  approved at `d2f37b91` (pre-merge CI stability is a separate item).
- Report §6 latency table does not state which clocks each stage uses, and no
  analysis script or output is stored for live6/live8 (the stored
  `LP-stale-analyze.py` targets a different run). My reproduction above shows
  the figures are source-clock based (node api-log vs pool `issued_at`), not
  collector `ingested_at`; say so, and keep the script.
- The pool reviewer's N1 stands: the "failed first" claims for the Redis and
  SV2 regressions have no stored red cargo logs
  (`/tmp/opencode/lab-runtime-redis-failfirst/` holds only a redis-server log).
  State them as unrecorded or re-run against the old wiring with the output
  kept; do not fabricate.
Fix: rerun the two-pool scenario once with binaries from `nix build` outputs
of the frozen source (store paths recorded via `--node/--pool/--miner`
pointing at `result/bin`), pass `--base-revision`/`--dirty-tree-hash` from the
manifest aggregate, and update §2/§3/§5/§6/§8 accordingly. The latency table
may stay provisional; it must name clocks and point at a stored script.

### R4 (medium-low) - Node observer scope is tips only; the report overstates coverage

`runtime.rs:1346-1384` emits `canonical_at(result=yes, anchored_tip=hash)` for
the current tip every 2 s. It never re-observes earlier heights, so a displaced
tip is never marked `no` (§7.3 current-status derivation would keep `yes`
forever), and most heights are never observed at all: live8 has 23
`canonical_at` records for a 158-block chain; my run 46 for a similar chain.
The report's "node tips / blocks observed: 159" reads as canonical coverage;
it is the tip height. Single-node regtest has no reorg here, so nothing is
false today, but the collector must be labelled "tip sampler, not reorg
observer".
Fix: say so in the report and the `detail` string; optionally re-check the
last K observed (height, hash) pairs each pass and emit `no` on displacement.

## Non-blocking notes (fix in the same revision if cheap, else record)

- N1 Capability self-assertion. `build_lab_config` (`runtime.rs:573-606`)
  declares the node adapter's capabilities as whatever
  `network.consensus_profile.required_capability` the scenario asks for, so
  the preflight capability gate is satisfied by construction; only the live
  profile field checks protect a `retarget`/`baseline` mismatch. Declare a
  fixed known set for the real node (or derive from introspection).
- N2 Identity artifact validation. `load_or_generate_identities`
  (`runtime.rs:467-484`) rejects unparseable artifacts (probe:
  `Fable-runtime-corrupt-id.log`, nothing started) but a parseable artifact
  with tampered `sp_id`/keys is accepted; the adapter would then fail to map
  `accounted` records silently. Re-derive `sp_id` and the address from the
  stored keys on load and fail on mismatch.
- N3 Metrics lock hold. `runtime_miner_snapshots` (`engine.rs`) holds the
  `miners` mutex across blocking pipe requests (10 s timeout each). Because
  `snapshot_with` holds the evidence lock while `entity_states` takes `miners`,
  a slow or hung miner stalls every HTTP snapshot and every collector append
  for the request duration (no cycle, so no deadlock). Clone the controls out
  of the map (or request per miner with the lock released).
- N4 Observers outlive the stop. `Observers::stop()` runs after the journaled
  `stop_run`, so the final PostgreSQL/Redis observations are
  connection-refused errors (live8 pool-a). Quiesce observers on `Stopping` or
  tag post-stop observations.
- N5 Scanner gaps. A failed `getblockhash`/`getblock` for a height is
  `continue`d and `scanned_through` still advances, so skipped heights are
  never rescanned and only inferable from `blocks_scanned_this_pass`. Record
  `skipped_heights` explicitly.
- N6 Identifier hygiene. `PostgresDatabaseSpec.database`/`role` are only
  checked non-empty/unique; they are interpolated into the storage URL and
  TOML without escaping (fail-closed at pool startup, but not validated at
  scenario time). The PostgreSQL observer re-parses the URL-encoded password
  back out of the URL (`runtime.rs:1410-1418`); hex passwords survive, others
  would not. Pass the raw secret and restrict identifiers to
  `[a-z_][a-z0-9_]*`.
- N7 `job_issued` ids are 8 hex chars per front; `emit_oracle_summary` counts
  distinct `job_id` strings across fronts (`jobs_observed: 222` for 482 typed
  jobs). Key by (front, job_id, generation).
- N8 Inherited stale warning text: `engine.rs:194` still says "port allocation
  is process-local ... does not reserve ports across independent run
  directories" although LP-3 R3 holds kernel port locks. Present in approved
  R3 too; fix wording in the combined revision.
- N9 Shared payout destination. Both pools render the same throwaway
  `POOL_PAYOUT_DESCRIPTOR`; ledgers are distinct, the coinbase destination is
  not. Fine for the lab; say it in the report so "two independent pools" is not
  read as two payout destinations.
- N10 `/catalog` publishes SP addresses on the loopback control surface. They
  are public keys, not secrets, and are not written to `shareable/`; keep it
  that way and note it under §7.7.
- N11 The SPEC frontmatter changelog now records a lab tool ("add the built-in
  LP-runtime host runner"). The decision record exists, so the convention is
  met; the merge session should decide whether a lab runner belongs in the
  normative spec changelog or in the lab/OPERATIONS docs. The final docs
  splice remains the merge session's gate.
- N12 Reusable recipe. Report §9 works with explicit paths; my probe script is
  a second worked example (`python3 /tmp/opencode/Fable-runtime-probe.py
  <worktree> <run_dir> <seconds>`), and `spc-lab reap --run-dir <run_dir>`
  cleaned a run whose controller was killed (`Fable-runtime-reap1.log`). The
  eight long-lived `redis-server`/`postgres`/`rbitcoin-node` processes on this
  host belong to other sessions and were left alone.

## Remaining gates (explicit, not reopened)

Namespace placement, address publication and controller RPC transport
(LP-4); causal retarget-response or hash-budget response claims (the live
profile has `no_pow_retargeting=true`); full-window payout/`slice_credit`
verification against the approved oracle (header recompute needs data the
front does not emit); fee/witness workload coverage; SV1; Ark settlement;
production payout; the node's own full suite; the final SPEC/OPERATIONS
splice by the merge session.

## Approval path

Apply R1-R4 (and N1-N7 where cheap), rerun the two-pool scenario once with
Nix-built binaries and recorded store paths, store the latency analysis
script with clock labels, refresh the report, and publish a new aggregate.
The pool/store delta approval at `c9dfa986...` carries forward unchanged
unless those six files change. Re-review will start from the new
`oracle_accounted_check`/assertion artifacts and the binary provenance in the
new run manifest.

---

# Second pass (2026-11-06): LP-runtime host R2

Reviewer: Fable (subagent), read-only; no candidate edit, commit or subagent.
Probe outputs: `/tmp/opencode/Fable-runtime-r2-*`.

Snapshot (full, annotated): `/tmp/opencode/lab-runtime-review-manifest.json`,
174 files, aggregate over `{base, files}` with `path/sha256/source/change`
records = `e891f34c69c70c2a6748be1ca2a77a71aba4a63377eda55a43afeb8613b68043`
(174/174 hashes match the worktree).
Tested code aggregate: `d75ab391b0ca1c8ab5b03af27dd5cf45a8a384ef8ac056a1ceeb2b2f456d8684`.
I reproduced it independently: it is the same recipe over the 172 records
that remain after excluding `docs/reviews/2026-11-06-lab-runtime-implementation.md`
(the mutable run report) and `docs/reviews/2026-11-06-lab-live-fable-review.md`
(copied review context), exactly as the manifest's `recipe.code_subset` says.
`/tmp/opencode/lab-runtime-final-src` matches 173/174 files; the single
difference is the run report, which is outside the code subset, so the source
copy binds every Rust, Cargo, config, scenario, script, static-UI and test file.

## Verdict

**APPROVE (bounded host runtime)** for full snapshot `e891f34c...b68043` and
tested code snapshot `d75ab391...6d8684`.

R1-R4 are fixed in code and artifacts, the folded N1-N8 are implemented, and
no regression to the foundation's deadlock/stop/journal behaviour was found
(81 spc-lab tests pass here, including the R3 snapshot-stop race, long-wait
stop and subscriber-lag cases; packet2 and my own run stopped cleanly with 0
gaps). The six-file pool/store delta is unchanged since its approval
(`bins/spc-pool/src/main.rs 5fe65bb6...`, `sv2_server.rs a44385df...`,
`tests/main_sv2_pipeline.rs e25978f9...`, `tests/sv2_outcome_preemption.rs
2ba3bad7...`, `crates/spc-store/Cargo.toml 0dbe8c15...`, `tests/redis.rs
b8c83bcd...`; all mtimes precede the pool review) and is not reopened.

The approval is bounded exactly as the report states: a host-namespace lab
runtime with real node/pool/miner/Redis/PostgreSQL processes, durable browser
controls, collector-fed evidence and a scoped accounted-target consistency
check. It does not approve namespace placement or controller transport
(ongoing in `lab-topology`), causal retarget or hash-budget response, full
header/payout-window verification, fee/witness coverage, SV1, Ark, the node's
own full suite, or the final SPEC/OPERATIONS splice, which stays the merge
session's gate. One fail-closed defect (F1) must be fixed in the merge
revision; it does not change this verdict because the only real emitter cannot
trigger it.

## Provenance verified

- `nix eval` of `/tmp/opencode/lab-runtime-final-src` for the four packages
  and four checks yields exactly the derivation and output paths recorded in
  `/tmp/opencode/lab-runtime-packet2-provenance.json`
  (`Fable-runtime-r2-nix-eval.log`); every output is realised; the four
  `bin/` hashes equal the recorded ones (`spc-lab f8aa1d57...`, `spc-pool
  aac1278f...`, `spc-cpu-miner 443de438...`, `spc-scan f8000b43...`).
  `flake.nix` carries `sha256-TET0F9jXQELPadNaPKBiPXgFNGPnU9uRaJWjkSAX52w=`;
  the report now states the same.
- `nix log` of the workspace-tests derivation: 609 passed / 0 failed / 2
  ignored; clippy derivation 0 warnings; ui-node realised (14/14 locally).
  Recorded cargo logs: workspace 609/0/2, targeted 353/0/2, pool 236/0,
  real-Redis 4/0.
- Packet2 run manifest records `base_revision = b0ab4ca...`,
  `dirty_tree_hash = d75ab391...` (no `runtime` placeholders), the Nix store
  paths for pool and miner, Nix-store Redis/PostgreSQL, and the one explicit
  external exception: the node is the pinned-L2 debug build `eb257af7...`
  from `/tmp/opencode/rbitcoin-lab-retarget-profile` (source snapshot
  `d2f37b91...`), labelled as such in the report and the provenance record.
- The LP-4a input now carries `recipe_sha256 = bce93151...`; the earlier
  `null` is resolved.

## R1-R4 verified

- R1 `accounted_target_consistency` (`runtime.rs:409-569`): join key is
  (front, job_id, generation) from typed `job_issued`; a record passes only if
  its `share_target`, `m_issue` and booked `d` equal the announced job and its
  raw-digest `share_hash` read little-endian is `<=` the announced big-endian
  target; missing job -> `unknown`, conflicting duplicates -> `unknown`,
  mismatch -> `inconsistent`; `scope.checked/not_checked` lists are explicit
  and `full_header_payout_oracle: inconclusive`. Unit tests cover the
  cross-front same-id join, self-consistent tampered target, conflicting/
  missing jobs and front-scoped `accounted_work`. Packet2: 810/810 consistent,
  267 scoped job keys, 0 unknown, 0 inconsistent; the summary is copied into
  `shareable/result.json`.
- R2 Both scenarios now assert `exists(job_issued)`,
  `exists(spc_evidence && canonical_at)` and `accounted_work(front=...,min=1)`
  per declared front; an absent front yields `inconclusive`, not pass (unit
  test). Packet2 `result.json`: all four passed; accounted a1 130 / b1 680;
  PostgreSQL ledgers `57|57|1` vs `224|349|3`.
- R3 Report §2 states the L2 clearance as recorded baseline limitations (no
  "adjudication pending"); §3 vendor hash corrected; §5 separates the
  source-bound packet2 table from the unbound live6/live8 debug hashes; §6
  states that the fail-first red runs are unrecorded and source-reasoned.
  `scripts/lab/analyze-runtime-latency.py <run_dir> --json <out>` reproduces
  `/tmp/opencode/lab-runtime-latency-packet2.json` exactly here (qualified
  n=194, p50 4 / p95 7 / max 8 ms; 504 ambiguous multi-front activations
  excluded; clocks named; `collector_ingested_at: not used`; no causal budget
  claim). Note: the report's recipe uses the positional `run_dir`, which is
  the script's actual interface.
- R4 Node observer (`runtime.rs:1548-1657`) is labelled `tip_sampler`, emits
  `node_canonical_scope` with tip observations versus max height (packet2: 25
  of 175), rechecks the last 8 observed tips each pass, emits
  `canonical_at(no)` with the current tip as `anchored_tip` only when a
  different hash is read back, and leaves a failed read unknown. Unit test
  `displacement_is_no_only_for_an_observed_different_hash`.

## N1-N8 verified

Fixed node capability set (`build_lab_config`); identity artifacts re-derived
from the stored private keys with address/`sp_id` mismatch rejected
(`runtime.rs:615-645`); miner handles cloned out of the map before the
blocking snapshot request (`engine.rs:447-470`) so the evidence/snapshot locks
are not held across pipe waits; collectors exit on leaving `Running|Preparing`
so no post-stop connection-refused observations (packet2 last PostgreSQL
reads are the real counts); scanner keeps a contiguous cursor and records
`skipped_heights` (packet2: `[]`, cursor 189); PostgreSQL identifiers
validated `[a-z_][a-z0-9_]*` and raw secrets passed to observers; job counts
scoped by (front, job_id, generation); port-lock warning text now describes
the kernel locks. N9 (shared throwaway payout descriptor), N10 (catalog
addresses on loopback only) and N11 (SPEC changelog placement) are recorded in
report §8.

## Required fix for the merge revision (not a gate for this verdict)

- F1 `runtime.rs:506-524`: when `share_hash` is present but not valid 32-byte
  hex, the hash check is skipped and the record can still count as
  `consistent`. A missing hash correctly yields `unknown`; a malformed one
  must too. The approved LP-1 emitter always writes `hex32` of a 32-byte
  digest, so no real record reaches this path, but the oracle must fail
  closed. Three-line fix plus a unit test with `share_hash: "zz"`.

## Notes

- `write_oracle_result` rewrites `shareable/result.json` with a plain
  `std::fs::write` rather than the temp+rename used elsewhere; harmless for a
  shareable file, but use the same helper.
- My earlier two-pool run on fresh debug binaries also completed cleanly
  (`/tmp/opencode/Fable-runtime-runs/two-pools`); it is not source-bound and
  is not part of the approval evidence.

## Final acceptance binding

This approval binds the full annotated aggregate `e891f34c...b68043` and the
tested code aggregate `d75ab391...6d8684`. Any change to a file inside the
code subset (everything except the run report and the copied review context)
produces a new code aggregate and requires re-review; editing only the run
report changes the full aggregate but not the code binding. The merge session
should record both hashes with the packet2 provenance file when it splices
the shared files and decides where the lab-runtime changelog entry lives.
