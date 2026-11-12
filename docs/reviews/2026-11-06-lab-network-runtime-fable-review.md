# LP-network-runtime (namespaced two-pool mining lab): Fable review

Date: 2026-11-06
Reviewer: Fable (Claude Fable 5.1), read-only review subagent; no candidate edit, commit or subagent
Candidate: worktree `/home/ethan/.local/share/opencode/worktree/ad8886/lab-network-runtime`, base `b0ab4ca34fda07c61a6335d05f598eaacc69678c`
Snapshot: `/tmp/opencode/LP-network-runtime-manifest.json` (file sha256 `ee300caa...6201b4`), recipe `sha256(compact sorted JSON of {base, files[path, sha256, mode]})`:
- full aggregate `553cd4e8bb40d34be514a4ebd55d466904e1357f174b2b184b5dcf8724aa5692` (recomputed: match; 364 files, 0 hash/mode mismatches, no `.git` entry)
- code aggregate (all files except `docs/reviews/2026-11-06-lab-network-runtime-implementation.md`) `2d6c752d24d4f4cc9ea30f6d5ced8b507dfd860acb8721ded85c758992705098` (recomputed: match, 363 records)
Baselines: approved host runtime `e891f34c`/`d75ab391`; LP-4a broker `bce93151` (`scripts/lab/lib/lab_backend.py` = `069a87e4...`, byte-identical); approved docs `1d40b2b7` (applied; four owned docs then edited for the namespace guide, see N7)
Delta against the approved host: 11 changed files (`bins/spc-lab/src/main.rs`, `crates/spc-lab/src/{adapters,engine,lib,manifest,runtime,supervisor}.rs`, `crates/spc-lab/tests/common/mod.rs`, `docs/{REGTEST-LAB-IMPLEMENTATION,SPEC,WIRE-FORMATS}.md`), `links.rs` moved to `links/mod.rs`, new `namespace.rs`, `topology.rs`, `links/netns.rs`, `tests/topology_namespace.rs`, scenario `lab-network-runtime-two-pools.json`, `docs/LAB-RUNTIME.md`, three docs reports. Every other approved-host file is byte-identical.
Probe artifacts: `/tmp/opencode/Fable-network-runtime-*` (`live/`, `live.log`, `crash/`, `crash-reap.json`, `probe/probe.log`, `probe/pdeath_*.py`, `lib-tests.log`, `netns-tests.log`, `target/`)

## Verdict

**APPROVE (bounded combined namespace candidate)** for code aggregate `2d6c752d24d4f4cc9ea30f6d5ced8b507dfd860acb8721ded85c758992705098` (full `553cd4e8...4aa5692`), on the old L2 node `eb257af7...`.

The assigned deliverable now exists and was reproduced here end to end: two rbitcoin nodes, two pools/fronts, two Redis instances and two native SV2 CPU miners placed in broker-owned namespaces by the runtime itself, a real P2P partition and heal driven through the durable command journal, independent node-scoped canonical evidence of the fork and the reorg, 0 bypass violations, scoped accounting consistent, clean stop, and a kernel-enforced cleanup on controller SIGKILL. T1-T9 from the previous review are fixed in code (T3 partially, see N1) and the F1 host fix is carried. The findings below are non-blocking and are to be folded into the merge revision; none changes a published claim that this approval relies on.

Approval is bounded exactly as stated in "Bounds" below: consensus-link faults only, `--pow-bits 0x207ffffe` (almost every accepted share is a block; no non-block share proof is drawn from this run), the external L2 debug node, host network tools, and no full header/window/fee/witness/Ark/SV1/causal-retarget claim.

## Independently reproduced

### The live two-pool run (`/tmp/opencode/Fable-network-runtime-live`, 1 m 55 s, exit 0)

Same command as the report (frozen Nix `spc-lab`/`spc-pool`/`spc-cpu-miner`/`spc-scan`, Nix PostgreSQL 17.11 and Redis 8.10.3, external node, `--topology`). Results from the run directory, not from the candidate's evidence script:

- `shareable/result.json`: all four assertions `passed`; `accounted_target_consistency`: `consistent`, 78/78 consistent, 0 unknown, 0 inconsistent, 0 malformed. Accounted per front: a1 38, b1 40 (`private/evidence.jsonl`).
- `shareable/topology.json` phases (journaled `wait_height`/`pause_workers`/`partition_link`/`resume_workers`/`heal_link`/`assert`): before partition both nodes at height 4 `1da455d09b`; during partition node-a h6 `62823c36dc` vs node-b h6 `16b4fed82e` (**distinct hashes at the same height**: a real fork), then a h7 vs h6; `nft_dropped_packets` 85 -> 89 while faulted, 0 after heal; no-bypass during the fault: `ok: true`, 276 checks, 0 violations; after heal node-b adopted node-a's h7 `60a6394e69`, both at h8 `58ef82b933`, convergence `converged` at h44 `2ae9939604` after the journaled pause/re-read (attempts 2).
- Independent canonical evidence (`canonical_at`, tip sampler): node-b's observed h6 `16b4fed82e` later read back as a **different** hash -> `no` (its partitioned branch was reorganised away); node-a's h41 later displaced by node-b's branch -> `no`. 11 `yes`, 2 `no`, 0 invented; one baseline `atomic_tip` failure was reported as `atomic: false`, not as a view.
- Node API logs: 0 `generate*`/`setmocktime`; 38 `submitblock` on node-a; blocks came from `getblocktemplate`/`submitblock` work by the pools, i.e. native header PoW through SV2, not RPC generation.
- Storage really is what the report says: `private/pool-front-a1.toml` has `redis = "redis://:<pw>@10.250.0.2:34443/0"` (the storage veth address; `redis.conf` binds `10.250.0.2`, `requirepass` set) and `postgres = "...@localhost:43039/...?host=<run>/postgres/pg-pool-a/socket"`; front `listen`/`jd` on the miner-link address, `rpc_url` on the pool-node link address. No cross-namespace loopback.
- Teardown: 8 `stopped` + 2 `exited`, 0 broker/holder/actor survivors, 0 PostgreSQL processes with the run cwd, `/run/netns` empty, no lab interfaces.
- No secret (Redis password, PostgreSQL password, RPC cookie) appears in any `shareable/*.json`.

### Controller SIGKILL (`/tmp/opencode/Fable-network-runtime-crash`)

With 10 actors running, broker + 8 holders alive and **20 PostgreSQL processes** (postmasters and workers) whose cwd was the run directory, `kill -KILL` of the `spc-lab runtime` process (exe verified as the Nix store binary): 0 survivors among broker/holders/actors after **511 ms**; 0 PostgreSQL processes with the run cwd and 0 processes mentioning the run directory 3 s later; `/run/netns` empty; `spc-lab reap --run-dir` then acquired the released lock (rc 0) and reported all ten records `exited`. This covers the descendants the candidate's driver did not enumerate (PostgreSQL workers) and matches `/tmp/opencode/LP-network-kimi-final-crash-v1-evidence.json`, whose survivor check runs before `reap` (no masking).

### PDEATHSIG across the credential transitions (`probe/pdeath_*.py`)

`PR_GET_PDEATHSIG` read after `prctl(PR_SET_PDEATHSIG, SIGKILL)` + exec: plain exec 9; through `unshare -U --map-root-user -r -n` (broker path) 9; through `nsenter --user=<broker userns> --net=<endpoint> --preserve-credentials` (actor path) 9; control `unshare -U --map-user=0 --map-group=0` 9. The kernel does not clear the death signal on these user-namespace joins (`cred_cap_issubset` holds for a child/owned namespace), so the `actor-exec` chain is sound in theory and in practice; the SIGKILL run above is the operational proof.

### Tests and Nix

- Host cargo (this tree): `spc-lab --lib` 19/19 (F1 trio, atomic-tip displacement, P2P peer mapping, storage URL tests included); `--test topology_namespace` default **5 passed / 13 ignored**; with `SPC_LAB_NETNS_TESTS=1` and the Nix Redis paths, `--include-ignored --test-threads=1` **18/18** (51 s), host clean afterwards.
- Nix, from the frozen source `/tmp/opencode/LP-network-kimi-frozen-src-v2` (363/363 code files match the manifest; `.git` excluded): `nix eval .#spc-lab.drvPath` = `vl2rxvk8...` and the recorded outputs exist with the recorded binary hashes (`spc-lab 6a3b61f5...`, `spc-pool aac1278f...`, `spc-cpu-miner 443de438...`, `spc-scan f8000b43...`); `checks.x86_64-linux.test` (`z288y8w1...`) and `clippy` are realised; the workspace-tests log shows `topology_namespace`: `5 passed; 0 failed; 13 ignored`. The default Nix gate is green again (it was red on the previous snapshot). The iproute2/nftables/util-linux realisation remains blocked by unreachable mirrors: every namespace result above is a **host-tool** result (util-linux 2.37.2, iproute2 5.15.0, nftables 1.0.2, python 3.10) and is labelled so in the report.

## Previous findings (T1-T9) and F1

| Item | State |
|---|---|
| T1 link ids / routes / multiple consensus links | Fixed: `topology.rs:211-307` keeps scenario ids verbatim, drops the generated parallel route for a declared pair, `consensus_links: Vec`, fails on unplaced endpoints, duplicate ids and generated-id collisions; `the_scenario_preflight_passes_against_its_own_broker` and `an_unplaced_scenario_link_is_refused` pass. |
| T2 broker start | Fixed: `namespace.rs:281-448` refuses a non-empty dir (`StaleRunDir`), validates the 8-hex token, propagates `up-error-<token>.json`, requires manifest token match and a `ping` whose token matches; `Drop` sends `down` then kills and waits. |
| T3 error vocabulary / `current()` | Partially fixed: `current()` asks the broker and returns `None` on a dead broker; a failed apply clears the cache. The error classification does not work as described (N1). |
| T4 exec handshake | Fixed: canonicalised comparison, `try_wait` -> `LaunchFailed{status}`, kill **and wait** on timeout, namespace-owner start-time check before launch, in-namespace listener probe with inode ownership (`supervisor.rs:302-475, 753-890`); tested in `the_exec_handshake_survives_a_symlink_and_reports_a_failed_launch` and the combined user+net test. |
| T5 atomic tip | Fixed: `atomic_tip` = `getbestblockhash` -> `getblockheader(hash).height` -> `getblockhash(height) == hash`, mismatch is an error/unknown (`runtime.rs:2380-2403`); `a_racy_tip_view_is_unknown_and_never_a_displacement` passes. |
| T6 Nix default green | Fixed: process-level tests `#[ignore]` behind `SPC_LAB_NETNS_TESTS=1`; default 5/13; Nix `test` check realised. |
| T7 tool/bounds | Fixed: tools from `PATH`/options, host-address enumeration failure is fatal, 8 MiB response cap, 120 s read timeout, exec exit codes checked. |
| T8 storage | Fixed and used by the real fronts (Redis in `storage-<pool>` namespace with auth on the published address; PostgreSQL host-local over the private Unix socket). |
| T9 report accuracy | Fixed: host approved; old artifact labelled RPC `generate`/shell `nsenter`; packets not blocks. |
| F1 | Carried unchanged from the frozen lab-topology tree (malformed -> `inconsistent` + counter, missing -> `unknown`); 3 unit tests green. |

## Findings (non-blocking; fold into the merge revision)

### N1 (medium-low) - `LinkError` classification keys on text the broker never sends; `current()` turns `partial` into `Normal`

Code: `namespace.rs:518-530` (`Rejected { op, detail }` drops the broker's `error.code`), `links/netns.rs:230-248` (`map_error` matches `detail` for the strings `invalid_policy`, `policy_apply_failed`, `partial`, `busy`, `unknown link`), `links/netns.rs:208-211` (`policy_from_json(..).unwrap_or(LinkPolicy::Normal)`).

Counterexample (`probe/probe.log`): `apply(latency 0)` -> `Unavailable("delay_ms out of range: 0")` (broker code `invalid_policy`; expected `Unsupported`); `apply(loss 101)` same; unknown link -> `Unavailable("no link 'nope' in this topology")` (correct by accident). `Busy` and `Partial` are unreachable because the code is not propagated. The engine maps every apply error to `OutcomeState::Failed` with the detail string, so the run outcome is unaffected and fail-closed; only the report's "partial/busy/unsupported/unavailable are distinct" claim and the `LinkBackend` contract are wrong. Separately, a link the broker reports as `{"type":"partial", ..}` is returned by `current()` as `Normal`; `current()` has no caller in the runtime today, but that is a fail-open default in a public API.

Fix: carry `code` in `NamespaceError::Rejected` and match on it; map `partial` to `LinkError::Partial` in `current()` (or return `None`). Test: `invalid_policy` -> `Unsupported`, `busy` -> `Busy`, status `partial` -> not `Normal`. Update report section 2 T3 wording.

### N2 (low) - the node RPC password travels on the helper's argv inside the namespace

Code: `adapters.rs:103-125`, `namespace.rs:809-854` (`spc-lab rpc --user .. --pass <password>` executed through broker `exec`). The per-run 24-hex RPC password is in `/proc/<pid>/cmdline` of each short-lived helper (same-uid visibility only) and is echoed in the broker's in-memory exec response `argv`. It was not found in any run artifact, log or `shareable/` file (verified), so this is hygiene, not a leak. Fix: `--pass-file <cookie>` (the 0600 cookie already exists) or an environment variable, and keep `--pass` out of argv. The front API path passes no credentials.

### N3 (low) - helper input handling

- `bins/spc-lab/src/main.rs` `Rpc`: an invalid `--params` JSON silently becomes `[]` (`unwrap_or`); reject instead.
- `namespace.rs:733` accepts any status line containing `" 2"`; parse the status code.
- `run-manifest.json` (0600, private) carries the Redis URL **with** the password under `resources.redis_instances[].url` (the host runtime carried an unauthenticated URL there). It is not in `shareable/`; document it as private or redact the credential in the manifest.

### N4 (low) - topology evidence is typed `unknown`

`runtime.rs:621-630, 668-680`: phase views, counters, no-bypass results and convergence are appended as `EvidencePayload::Unknown { topic: "topology_phase" | "topology_convergence", detail: <stringified JSON> }`. The run's central partition evidence therefore sits in the same `unknown` bucket as metrics and collector noise (188 `unknown` events in the candidate run). This follows the host runtime's existing pattern for `runtime_metrics`, but a consumer that treats `unknown` as unparsed will drop it. Add typed payloads in the merge revision; `shareable/topology.json` already carries the same data, which is why this is not blocking.

### N5 (low) - convergence recovery is runtime-initiated mining outside the scenario phases

`runtime.rs:639-711`: after the phases, if the two atomic views differ the runtime resumes both miners once ("break the tie") and pauses again before declaring `converged`; `not_converged` after 90 s fails the run. The controls are journaled and the report says so; this is acceptable, but the behaviour belongs in the scenario vocabulary (an explicit `converge` phase with its own timeout) so a scenario author can see and bound it. Keep the recorded 969/913 no-heal run as a limitation; no general convergence claim is made or accepted.

### N6 (docs) - do not oversell the actor boundary

Report section 1: "Actors can mutate only their broker-owned namespaces." They can mutate only their broker-owned **network** namespaces; every actor is an ordinary uid-1000 host process with the full host filesystem and the shared run directory. `docs/LAB-RUNTIME.md:284-285` already states it correctly ("do not mutate host networking"); align the report and keep "not a security sandbox" explicit.

### N7 (merge note) - docs differ from the approved docs snapshot

`README.md`, `docs/OPERATIONS.md`, `docs/REGTEST-LAB-IMPLEMENTATION.md` and `docs/LAB-RUNTIME.md` differ from `1d40b2b7` (the namespace guide additions); `SPEC.md`, `WIRE-FORMATS.md`, the share-target decision and the deltas file are identical to the approved docs. The merge session must record these four as this packet's edits on top of the docs approval. The frozen source used for the Nix outputs contains an earlier revision of the mutable report (`ff3c928d...` vs `7c036cce...` in the tree); the code aggregate is unaffected.

### Environment note (not attributable to this candidate)

Five `redis-server` processes from earlier sessions (`LP-live-*`, `lab-runtime-runs/live1`, Nov 3-6) are still running on this host; none belongs to this candidate's runs or tests, which left no process behind.

## Bounds of this approval

- Node: the external L2 debug build `eb257af7...` (source `d2f37b91...`), an explicit non-Nix exception; nothing about rbitcoin v0.9.0 is inferred. The parallel v0.9.0+lab-patch validation is separate work and must bind its own binaries.
- Difficulty: `--pow-bits 0x207ffffe`; nearly every accepted share is a block, so this run proves native PoW, propagation, fork and reorg, not sub-block share accounting (the approved LP-live covers non-block shares).
- Faults: consensus link only; the Redis storage link and the PostgreSQL Unix socket are not partitioned and no storage-IP fault or Ark privacy-boundary claim is made.
- Isolation: network isolation of broker-owned namespaces on a shared filesystem with same-uid processes; not a sandbox.
- Not claimed: full header/payout-window oracle, fee/witness, causal hash-budget or retarget response, SV1, Ark, general SV2 interoperability, general convergence (one earlier 969 vs 913 no-heal run is retained as a limitation).
- Tooling: project binaries and the default `nix flake check` are Nix-built and green from the frozen source; `ip`/`nft`/`unshare`/`nsenter`/python/Redis/PostgreSQL are host or store tools recorded by version; Nix realisation of the network tools is an external blocker, not a pass.

## Final binding

This approval binds code aggregate `2d6c752d24d4f4cc9ea30f6d5ced8b507dfd860acb8721ded85c758992705098` (full `553cd4e8bb40d34be514a4ebd55d466904e1357f174b2b184b5dcf8724aa5692`), the frozen source at `/tmp/opencode/LP-network-kimi-frozen-src-v2`, the four Nix outputs listed in report section 6, the node binary `eb257af7...`, and the two reproduced runs under `/tmp/opencode/Fable-network-runtime-{live,crash}`. Any change to a file inside the code subset requires re-review; N1-N7 are expected in the merge revision and do not reopen this verdict.
