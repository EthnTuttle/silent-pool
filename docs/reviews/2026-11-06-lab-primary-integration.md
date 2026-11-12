# Lab primary integration record

Date: 2026-11-06 (final integration checks completed 2026-11-07 UTC)
Author: TFF merge session
Primary: `/home/ethan/code/silent-pool`
Base/HEAD before and after transfer: `b0ab4ca34fda07c61a6335d05f598eaacc69678c`
Status: **INTEGRATED UNCOMMITTED SOURCE - gates passed; no new feature semantics**

This was a serial byte-verified transfer, not another feature packet. No commit, push,
git merge, reset, stash, clean, garbage collection, node upgrade or pin adoption was
performed. Both input trees remained frozen. Approval stays bound to the below input
snapshots; an aggregate for additional primary-only documentation is not relabelled
as either approved whole-source hash.

## Inputs and exact transfer result

| Input | Approval binding | Primary result |
|---|---|---|
| Final lab integration | `/tmp/opencode/LP-final-runtime-manifest.json`, manifest SHA-256 `8bbe097de563600ce8bf0b8cb9693b082a9cff9bb51580d96afd89d2c0c3021f`; full `9ffff6dae933fad03ec0884092da1d97fcc65a571b5b573a43e67d4432a62465`; code `30fedafdbba57b79fe4c13e2de0658aa3f05f527b778a242cbfab9c4ba253d76` | 368 paths byte/mode exact; the two explicit exceptions are README and `docs/MERGE-QUEUE.md`, below. |
| v0.9 validation | `/tmp/opencode/lab-v09-validation-manifest.json`, manifest SHA-256 `00e18e4c5380ce333a73941f089e86dcfa2bc418bb69f305fafbc5dcb47d03f4`; snapshot `5473dbc851b01b731f05810af792d810e002e5a004a760a342ae070dcf3f9da0` | All 10 new report/gate paths byte/mode exact. No final-input path overlaps. |

The applied unified patch was `/tmp/opencode/lab-primary-approved.patch`, SHA-256
`5a0d17c8994f31fdbceb993da629333ff9a80982fa1ab699dde238a385c14420`.
It covered 41 tracked updates, 153 final-input additions and ten v0.9 additions.
There were **no deletions**: the candidate's `links.rs` to `links/mod.rs` move needed
no primary deletion because `links.rs` was absent at base; `links/mod.rs` was added.
No `.git`, run output, credential or raw mining artifact was copied.

The complete per-path records, approved/actual hashes, modes and exceptions are in
`/tmp/opencode/lab-primary-integration-manifest.json`.

### Updated paths (41)

- `./`: `Cargo.lock`, `Cargo.toml`, `flake.nix`
- `bins/spc-agent/src/`: `config.rs`, `jdc.rs`, `sv2_client.rs`, `translator.rs`
- `bins/spc-agent/tests/`: `e2e_sv2_client.rs`, `e2e_translator.rs`
- `bins/spc-pool/`: `Cargo.toml`
- `bins/spc-pool/src/`: `api_backend.rs`, `block.rs`, `config.rs`, `front.rs`, `lib.rs`, `main.rs`, `payout.rs`, `sv1.rs`, `sv1_server.rs`, `sv2_server.rs`
- `bins/spc-pool/tests/`: `e2e_sv2.rs`
- `config/`: `spc-agent.example.toml`, `spc-pool.example.toml`
- `crates/spc-accounting/src/`: `lib.rs`, `tests.rs`
- `crates/spc-store/`: `Cargo.toml`
- `crates/spc-store/src/`: `bootstrap.rs`, `models.rs`, `postgres.rs`, `wire.rs`
- `crates/spc-store/tests/`: `postgres.rs`, `redis.rs`
- `docs/`: `OPERATIONS.md`, `SPEC.md`, `WIRE-FORMATS.md`, `share-accounting-ext-deltas.md`
- `docs/decisions/`: `2026-11-02-share-target-policy.md`
- `testing/`: `regtest-e2e.sh`, `regtest-rbitcoin-e2e.sh`, `regtest-rbitcoin-reorg-e2e.sh`, `regtest-services-e2e.sh`

### Added final-input paths (153), grouped by parent

- `bins/spc-cpu-miner/`: `Cargo.toml`, `PROTOCOL.md`
- `bins/spc-cpu-miner/fixtures/`: `control.jsonl`, `events.jsonl`, `miner.toml`, `two-pools.toml`
- `bins/spc-cpu-miner/src/`: `budget.rs`, `config.rs`, `connection.rs`, `control.rs`, `hash.rs`, `ids.rs`, `lib.rs`, `main.rs`, `miner.rs`, `report.rs`, `session.rs`, `target.rs`, `worker.rs`
- `bins/spc-cpu-miner/tests/`: `control_schema.rs`, `live_session.rs`, `sv2_session.rs`, `target_budget.rs`
- `bins/spc-lab/`: `Cargo.toml`
- `bins/spc-lab/src/`: `main.rs`
- `bins/spc-lab/static/`: `app.js`, `index.html`, `style.css`
- `bins/spc-lab/tests/`: `app_helpers.test.cjs`
- `bins/spc-pool/src/`: `evidence.rs`
- `bins/spc-pool/tests/`: `main_sv2_pipeline.rs`, `sv2_conformance.rs`, `sv2_outcome_preemption.rs`
- `crates/spc-accounting/proptest-regressions/`: `work_unit_tests.txt`
- `crates/spc-accounting/src/`: `v2_integration_tests.rs`, `work_unit.rs`, `work_unit_tests.rs`
- `crates/spc-lab/`: `Cargo.toml`
- `crates/spc-lab/scenarios/`: `lab-live-v2-two-pools.json`, `lab-live-v2.json`, `lab-network-runtime-two-pools.json`, `lab-topology-two-nodes.json`
- `crates/spc-lab/src/`: `adapters.rs`, `artifacts.rs`, `command.rs`, `config.rs`, `convergence.rs`, `engine.rs`, `error.rs`, `evidence.rs`, `http.rs`, `ids.rs`, `lib.rs`, `manifest.rs`, `namespace.rs`, `oracle.rs`, `resource.rs`, `runtime.rs`, `scenario.rs`, `secrets.rs`, `spc_evidence.rs`, `supervisor.rs`, `topology.rs`, `wide.rs`
- `crates/spc-lab/src/bin/`: `fixture.rs`
- `crates/spc-lab/src/links/`: `mod.rs`, `netns.rs`
- `crates/spc-lab/tests/`: `artifacts_privacy.rs`, `command_concurrency.rs`, `command_idempotency.rs`, `driver_loss.rs`, `engine_restart.rs`, `evidence_states.rs`, `height_wait.rs`, `http_protocol.rs`, `http_sse.rs`, `http_subscriber_lag.rs`, `lifecycle_stop.rs`, `long_wait_stop.rs`, `manifest_credentials.rs`, `miner_protocol.rs`, `oracle_generation_credit.rs`, `pool_authority.rs`, `preflight.rs`, `real_cpu_miner_proof.rs`, `real_node_proof.rs`, `run_lock.rs`, `scenario_v1.rs`, `snapshot_resume.rs`, `snapshot_stop_race.rs`, `spc_evidence_adapter.rs`, `supervisor_adoption.rs`, `topology_evidence.rs`, `topology_namespace.rs`, `two_runs_isolation.rs`
- `crates/spc-lab/tests/common/`: `mod.rs`
- `crates/spc-lab/tests/fixtures/`: `scenario_v1_one_pool_two_miners.json`, `scenario_v1_two_pools.json`
- `crates/spc-store/migrations/`: `0005_lab_work_units.sql`
- `crates/spc-store/tests/`: `work_units.rs`
- `docs/`: `LAB-RUNTIME.md`, `REGTEST-LAB-IMPLEMENTATION.md`
- `docs/changelog/`: `2026-11-05-lab-lp1-integration.md`, `2026-11-05-lab-share-work-units.md`, `2026-11-05-sv2-native-conformance.md`, `2026-11-05-window-network-block-work.md`, `2026-11-06-lab-convergence-v1.md`, `2026-11-06-lab-runtime-host.md`, `2026-11-06-pool-reader-outcome-latency.md`
- `docs/decisions/`: `2026-11-05-lab-lp1-integration.md`, `2026-11-05-lab-share-work-units.md`, `2026-11-05-sv2-native-conformance.md`, `2026-11-05-window-network-block-work.md`, `2026-11-06-lab-convergence-v1.md`, `2026-11-06-lab-runtime-host.md`, `2026-11-06-pool-reader-outcome-latency.md`
- `docs/reviews/`: `2026-11-05-lab-l1-implementation.md`, `2026-11-05-lab-l3-contracts.md`, `2026-11-05-lab-live-implementation.md`, `2026-11-05-lab-lp1-implementation.md`, `2026-11-05-lab-lp1a-implementation.md`, `2026-11-05-lab-lp2-implementation.md`, `2026-11-05-lab-lp3-http-implementation.md`, `2026-11-05-lab-lp3-implementation.md`, `2026-11-05-lab-lp4a-implementation.md`, `2026-11-06-lab-docs-baseline.md`, `2026-11-06-lab-docs-implementation.md`, `2026-11-06-lab-final-integration-implementation.md`, `2026-11-06-lab-network-runtime-implementation.md`, `2026-11-06-lab-runtime-implementation.md`, `2026-11-06-lab-topology-implementation.md`
- `scripts/`: `check-lab-docs.py`
- `scripts/lab/`: `analyze-runtime-latency.py`, `build-runtime-manifest.py`, `fixture-config.json.in`, `link-fault.sh`, `netns-preflight.sh`, `preflight.sh`, `run-once.sh`
- `scripts/lab/lib/`: `lab_backend.py`, `lab_common.sh`, `lab_sock.py`, `netns_probe.py`, `ns_exec.py`, `nsprobe-inner.sh`
- `testing/`: `regtest-lab-v2-e2e.py`
- `testing/lab-isolation/`: `relay_fixture.py`, `run.sh`, `tcp_fixture.py`
- `testing/lab-live/`: `baseline.json`, `oracle.py`, `retarget.json`
- `testing/lab-topology/`: `run-two-nodes.sh`

### Added v0.9 paths (10)

- `docs/reviews/`: `2026-11-06-lab-v09-validation.md`
- `testing/lab-release/`: `candidate.example.json`, `contract.json`, `independent_checks.py`, `namespace_replay.py`, `release_lib.py`, `review_contract.md`, `test_release.py`, `v09_release.py`
- `testing/lab-release/fixtures/`: `namespace-v09-structure.json`

### Deleted paths

None.

## Preserved primary context and index effects

- `docs/MERGE-QUEUE.md` and `docs/REGTEST-LAB-STATUS.md` were not edited. Their
  pre-transfer hashes remain `b4fbddbaecb19935d8d6d450d0bbe61fd6ba0cf6a8ca0bc7f81489f073d22bb0` and `85c7596b850c62a157bd186c0a95aa59257b1d396675d682899e4b6288a1436b`.
- All 28 pre-existing primary records other than README are byte/mode preserved,
  including both regtest proposal/status documents and every pre-existing Fable
  review. There were no differing report collisions requiring an additional
  keep-primary exception.
- README is the sole docs-only merge exception. It contains all approved README
  content plus the two pre-existing links to `docs/REGTEST-LAB.md` and
  `docs/reviews/2026-11-05-regtest-lab-review-summary.md`, inserted after ROADMAP.
  Its resulting SHA-256 is `58a87c320baf552615b020a198e27929c0c416bd28016e7739b898a5dd0f4c76`; it deliberately differs from the
  approved input's single-README hash without changing software semantics.
- The index is untouched: zero staged paths before and after, no intent-to-add,
  and no staging of pre-existing user changes. New delivered source remains
  untracked in this explicitly uncommitted primary checkout.
- Original proposal, review history and parent boards were not rolled back.

## Actual final checks

All commands ran from the primary checkout except the explicitly sourced Nix check.
`PYTHONDONTWRITEBYTECODE=1`, `TMPDIR=/tmp/opencode` and a private umask were used for
the Python checks. No live mining, crash run or `--full` harness was invoked.

| Command / check | Exit | Result / log |
|---|---:|---|
| `python3 testing/regtest-lab-v2-e2e.py --check-scenario testing/lab-live/baseline.json` | 0 | `status=validated`; `lab-primary-driver_baseline.log` |
| Same driver, `--check-scenario testing/lab-live/retarget.json` | 0 | `status=validated`; `lab-primary-driver_retarget.log` |
| Same driver, `--self-test` | 0 | 17/17 OK; `lab-primary-driver_selftest_17.log` |
| `python3 testing/lab-release/v09_release.py --check-contract` | 0 | contract `adcba5c9...`; `lab-primary-v09_contract.log` |
| `v09_release.py --static-audit /tmp/opencode/v09-audit` | 0 | passed; `lab-primary-v09_static_audit.log` |
| `python3 testing/lab-release/test_release.py` | 0 | 15/15 OK; `lab-primary-v09_selftest_15.log` |
| `v09_release.py --inputs /tmp/opencode/LP-v09-r2-profile-stock-input.json --release-check --check-input` | 0 | exact release inputs bound; `lab-primary-v09_release_input.log` |
| Read-only `namespace_replay.py` on approved namespace artifacts | 0 | replay SHA-256 `a5322a67ca8ada3e24b8c00940259ad2d821fe9e92d54ba8506204d2cd1c2480`, byte-identical to reviewed evidence |
| `cargo fmt --all -- --check` | 0 | `lab-primary-fmt.log` |
| Offline locked workspace RPC-feature test | 101 | Dependency resolution only: cache lacked `ar_archive_writer 0.5.3`; no compilation/tests ran. `lab-primary-workspace_tests_rpc_locked.log` |
| Same `cargo test --locked --workspace --all-targets --features spc-pool/rpc`, registry access enabled | 0 | All executed tests passed; `lab-primary-workspace_tests_rpc_locked_registry_retry.log` |
| `cargo clippy --locked --workspace --all-targets --features spc-pool/rpc -- -D warnings` | 0 | `lab-primary-workspace_clippy_rpc_locked.log` |
| `nix flake check --offline --max-jobs 1 /tmp/opencode/lab-primary-integration-src` | 0 | All six: fmt, clippy, doc, workspace test, ui-node, test-ark; `lab-primary-nix-flake-check.log` |

`CARGO_TARGET_DIR=/dev/shm/LP-primary-cargo-target`, two build jobs and
`RUSTFLAGS=-Cdebuginfo=1` were used for the Cargo/Nix checks. The initial offline
Cargo result is retained as an environment-resolution failure, not presented as a
pass. The successful locked retry changed no source or lockfile.

`merge-gate.sh` requires a Git root and its bare Nix invocation would filter out the
untracked transferred crates. Instead its fmt/workspace-test/workspace-clippy/Nix
constituents ran directly, with `--locked` added by this task. Nix used the git-free
explicit source copy `/tmp/opencode/lab-primary-integration-src`, which matched every
transferred path and included all new files. Thus no intent-to-add or fake Git-filter
pass was used, and no artificial wrapper failure was necessary.

## Remaining bounds

- The v0.9 node stays in its separate source/temp repository and Nix/store inputs;
  `UPSTREAM_DEPS.md`, current node pins and operator deployment state were not changed.
  Transferring the validation gate is not an operator node upgrade.
- The pinned Nix network-tool realization remains externally mirror-fetch blocked.
  This integration did not force that unrelated realization; namespace evidence
  remains host-tool evidence with separately recorded Nix project/PostgreSQL/Redis
  inputs.
- Existing approval/exclusion scopes remain authoritative: no production deployment,
  general convergence, quiet A/B node stability, full fee/witness/Ark/SV1 or broad
  historical live-claim expansion follows from this transfer.
- The primary's additional preserved documents and this integration record are
  outside the approved input subsets. The transfer manifest verifies per-input
  paths rather than claiming the whole primary equals either approved snapshot.
