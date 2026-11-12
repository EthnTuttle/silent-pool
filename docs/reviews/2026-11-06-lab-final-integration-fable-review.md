# Lab final integration (N1-N7 merge revision): Fable review

Date: 2026-11-06
Reviewer: Fable (Claude Fable 5.1), read-only review subagent; no candidate edit, commit or subagent
Candidate: worktree `/home/ethan/.local/share/opencode/worktree/ad8886/lab-final-integration`, base `b0ab4ca34fda07c61a6335d05f598eaacc69678c`
Snapshot: `/tmp/opencode/LP-final-runtime-manifest.json` (file sha256 `76e6d5bd...446854`), recipe `sha256(compact sorted JSON of {base, files[path, sha256, mode]})`:
- full `6271865396b5312616ddef5448f75845f5ca8fc218abfd39db05e4dcd2fde9a2` (recomputed: match; 370 files, 0 hash/mode mismatches, no `.git` entry)
- code (all but `docs/reviews/2026-11-06-lab-final-integration-implementation.md`) `37acb9aa5eb3ea23d2525dc4921ccec8d23e90e43b53ca6809bcee93af938bd1` (recomputed: match, 369 records; `/tmp/opencode/LP-final-frozen-src-v3` matches all 369)
Baseline: approved namespace runtime full `553cd4e8` / code `2d6c752d` (frozen, unchanged inputs); LP-4a broker `069a87e4...` byte-identical; node `eb257af7...` external exception
Delta against `2d6c752d`: 15 changed files (`bins/spc-lab/src/main.rs`, `crates/spc-lab/src/{adapters,engine,evidence,lib,namespace,runtime,scenario}.rs`, `links/netns.rs`, `tests/topology_namespace.rs`, the two-pools scenario, `docs/{LAB-RUNTIME,OPERATIONS,REGTEST-LAB-IMPLEMENTATION,SPEC}.md`) and 6 new (`convergence.rs`, `tests/manifest_credentials.rs`, `tests/topology_evidence.rs`, decision + changelog `2026-11-06-lab-convergence-v1.md`, the report). Nothing outside the N1-N7 scope was reopened and no counterexample against previously approved code was found.
Probe artifacts: `/tmp/opencode/Fable-final-{convergence-probe.log,netns-tests.log,nix-test-v3.log,target}`

## Verdict

**REQUEST_CHANGES (focused)** for code aggregate `37acb9aa5eb3ea23d2525dc4921ccec8d23e90e43b53ca6809bcee93af938bd1` (full `6271865396...fde9a2`).

One reachable defect in the new convergence driver (C1) lets a `converged` certificate be published while the miners are still running, which is the exact property the N5 decision record, the SPEC frontmatter entry and the report say is guaranteed. The fix is one line plus one test vector. One documentation placement (C2) contradicts the approved docs decision and must be resolved by the merge session before the splice. Everything else in N1-N7 is implemented and verified below; a revision that fixes C1 (and resolves C2) can be approved on hash with no further live run, since the live evidence does not depend on the faulty path.

## C1 (blocking) - a tie-break resume clears nothing: `converged` can be published without the paused re-read

Code: `crates/spc-lab/src/convergence.rs:71, 113-137`. `quiesced` is set `true` on the first shared observation (pause issued) and is never reset. If the re-read after the pause is *not* shared (a block that was in flight when the pause landed), the loop takes the `allow_resume` branch and resumes the miners (`:131-136`); the next shared observation then hits `if shared.is_some()` with `quiesced == true` and returns `"converged"` (`:120-129`) **while the workers are resumed**. No pause and no re-read precede that certificate.

Counterexample (`/tmp/opencode/Fable-final-convergence-probe.log`, a `ConvergenceDriver` fed `[tie@8/tie@8, a9@9/tie@8, winner@10/winner@10]`, `allow_resume: true, max_resume_attempts: 1`):

```
observe -> [tie@8, tie@8] (mining=true)
worker_control(resume=false)
observe -> [a9@9, tie@8]  (mining=false)
worker_control(resume=true)
observe -> [winner@10, winner@10] (mining=true)
status=converged attempts=3 controls=[false, true] workers_mining_at_converged=true
```

This contradicts `docs/decisions/2026-11-06-lab-convergence-v1.md` decision 3 ("workers are paused and both nodes are re-read before a `converged` result is published"), the SPEC frontmatter entry and report section 2 N5. The live run `LP-final-native-v3` did not take this path (`attempts: 2`, controls `[pause, pause]`), so its evidence stands; the defect is in the recovery path the resume budget exists for.

Fix: reset `quiesced = false` whenever a resume is issued (after `:132`), so a later shared view must pause and re-read again; optionally record `pause_reread_confirmed: true` in `TopologyConvergenceEvidence` and let `shareable/topology.json` consumers require it. Test: add the vector above to `convergence::tests` and assert `calls == [false, true, false]` and that the last observation precedes no resume. The five existing vectors stay green (they never re-observe a non-shared view after a pause).

## C2 (required before the SPEC splice; merge-session decision) - lab-only entry in the normative SPEC frontmatter

`docs/SPEC.md:12` adds a `2026-11-06` changelog entry for the lab-only convergence DSL and typed topology evidence. The approved docs packet (`docs/reviews/2026-11-06-lab-docs-implementation.md`, decision 9, "No tool-only SPEC entry") explicitly keeps the 2026-11-06 host-runtime and pool-latency lab records out of the normative SPEC frontmatter and leaves the final SPEC/OPERATIONS splice to the merge session. This entry re-introduces exactly that class of record. The decision record and changelog fragment are the right home (`docs/decisions/2026-11-06-lab-convergence-v1.md`, `docs/changelog/2026-11-06-lab-convergence-v1.md`, both present); the SPEC line should be removed or the merge session must explicitly reverse docs decision 9 for all three lab records at once. The entry's content is accurate; only its placement is inconsistent. Note that after C1 the entry's "paused and re-read" sentence will be true.

## Verified (N1-N7)

- **N1** (`namespace.rs` `Rejected { op, code, detail }`; `links/netns.rs:map_error` matches `code` only: `unknown_link -> UnknownLink`, `invalid_policy -> Unsupported`, `busy -> Busy`, `partial|policy_apply_failed -> Partial`, other -> `Unavailable`; a successful apply whose returned policy is missing, malformed or not the requested one is `Partial` and clears the cache; `current()` returns `None` for partial/malformed/dead-broker, never `Normal`). Unit tests include a non-code detail containing `busy` and the `partial`/`bogus`/`{}` statuses; the opt-in broker test `the_link_errors_are_distinct_and_a_dead_broker_forgets_the_policy` passes against the real broker (my run). This closes the previous review's N1 counterexample.
- **N2** (`spc-lab rpc --auth-file`, `conflicts_with_all = ["user","pass"]`; `basic_auth_file`: `symlink_metadata` so a symlink is refused, regular file, mode `& 0o077 == 0`, <= 2048 bytes, single `user:password` line, no whitespace/control/NUL, errors never echo the body; `RpcNodeControl` refuses a namespaced call without an auth file). Opt-in test `rpc_auth_file_is_not_a_secret_in_helper_argv_or_response` captures real helper argv and the broker response and asserts the marker password and `--pass` are absent while `--auth-file`/the cookie path are present (passes here). Read-after-stat TOCTOU exists but is inside the same-uid run-directory trust domain the candidate declares; no stdin or environment credential path was added. `--pass` remains available on the CLI for manual host use only; the runtime never emits it.
- **N3** (`parse_rpc_params` requires valid JSON **array**; HTTP status parsed as `HTTP/1.0|1.1` + exactly three digits in 200..=299, 64 KiB head / 8 MiB body caps; `run-manifest.json` stays private 0600 with the authenticated Redis URL for restore, `shareable/` separate). `manifest_credentials` test (0600, restore round trip, marker absent from shareable) passes; live run: no password field in `shareable/*`.
- **N4** (`EvidencePayload::TopologyPhase`/`TopologyConvergence`, `schema_version: 1`, `deny_unknown_fields`, `AtomicTipObservation { node, height, hash, parent_hash, atomic, peers, error }`, `BrokerCallObservation { ok, value, error }`, `SharedChainAnchor`, legacy `unknown { topic: "topology_phase" }` still loads; generic SSE unchanged). The live run's 22 typed phase records carry real `previousblockhash` chains: pre-partition common anchor h9 `0210873e`; during the partition node-a h11 `25f426c4` (parent `4ff6e515`) vs node-b h10 `3ebbdc6a` (parent `0210873e` = the anchor), node-a h12 `4f825125` vs node-b h13 `2ca68198` whose parent `7222d95e` is a **different h12** - a genuine same-height divergence from the common anchor, provable from the typed parents, not from height differences; after heal node-a reorganised onto node-b's h14 `38a31a88`; final paused re-read both h44 `7d5e1ef0` (parent `4c9b2d49`). Consensus counters: 0 before the fault, 4 -> 34 dropped packets during it, 0 after heal (packets, not blocks). No-bypass at the partition step: `ok`, 276 checks, 0 violations. Source clocks: `occurred_at_wall`/`ingested_at` are the collector's, as in the approved host; no join between actors' clocks is asserted.
- **N5** (explicit `PhaseSpec.convergence { version: 1, timeout_secs, max_observations (1..=90), allow_resume, max_resume_attempts <= 1 }`, validated in `scenario.rs`; the implicit post-heal resume is gone; pause/resume are the existing durable `pause_workers`/`resume_workers` commands and a non-`Succeeded` outcome is an error, not ignored; stop -> `unknown` with reason; timeout/budget -> `not_converged`, which fails the run). Five fake-driver vectors pass and are not substituted for the live proof. Apart from C1, two limits to state: (a) the deadline is checked between observations only; one `observe()` is up to 2 nodes x 4 broker-exec RPCs x 30 s, so the 90 s budget can overshoot on a hung node (bound the exec timeout by the remaining budget, or document); (b) the action executes after **all** phases regardless of which phase declares it (`runtime.rs:578-590, 711`), so a mid-list declaration would be misleading; enforce "last phase" or document.
- **N6** `docs/LAB-RUNTIME.md`, the decision record and the report now say network-namespace isolation, same-uid actors, shared filesystem/run-directory trust domain, not a security sandbox.
- **N7** The four operator docs are listed as a recorded delta over docs snapshot `1d40b2b7`; `SPEC.md` differs from the approved docs copy only by the frontmatter line in C2; `WIRE-FORMATS.md` and the share-target decision are unchanged.
- **Default and Nix gates.** From the frozen source v3: `nix eval .#spc-lab.drvPath` = `pkx4bm8a...` (the report's), `bin/spc-lab` sha256 `0d3d31f6...`; `checks.x86_64-linux.test` drv `gwsgib41...` realised (`lzj9r5yy...`), its log shows `spc-lab` lib 28 passed including the 5 convergence, 2 netns-classification and 2 cookie/HTTP tests, `topology_evidence` 2/2, `manifest_credentials` 1/1, `topology_namespace` 5 passed / 14 ignored; `nix flake check --offline` log has all six checks and no error. These are new derivations, not the v2 cache. Host cargo here: opt-in suite 19/19 (5 default + 14 ignored, 56 s), host clean afterwards. The `/dev/shm` target and `RUSTFLAGS=-Cdebuginfo=1` were used for cargo checks only; the Nix outputs above are what the live run used (`binary_paths_and_hashes` in the run manifest match: pool `aac1278f`, miner `443de438`, node `eb257af7`, Redis `834c2a45`, PostgreSQL `5be10c0c`).
- **Live run `LP-final-native-v3`**: 4/4 assertions, 72/72 consistent (0 unknown/inconsistent/malformed), credits a1 30 / b1 42, 0 `generate*`/`setmocktime`, 30 `submitblock` on node-a, 8 stopped + 2 exited, `run-manifest.json` 0600. The crash-cleanup proof is carried from the approved tree honestly: `actor_exec`, the supervisor launch path and the broker are unchanged by this delta, so the previously replicated SIGKILL result applies.

## Bounds (unchanged)

Old L2 node only (v0.9.0 validation is a separate pending gate; no pin or deployment change is implied); `--pow-bits 0x207ffffe` (no non-block share proof here); consensus-link faults only, storage not partitioned; network isolation, not a sandbox; no full header/window/fee/witness/Ark/SV1/causal-retarget/stock-compat or general-convergence claim (969/913 no-heal diagnostic retained); network tools are host tools, Nix realisation externally blocked.

## Required for APPROVE

Fix C1 with the named regression; resolve C2 (remove the SPEC frontmatter line or record the merge session's explicit reversal of docs decision 9). Re-snapshot and re-hash; no new live run is required for these two changes.

---

# Second pass: C1/C2 revision

Date: 2026-11-06
Reviewer: Fable, read-only; no candidate edit, commit or subagent
Snapshot: `/tmp/opencode/LP-final-runtime-manifest.json` (file sha256 `8bbe097d...3021f`); full `9ffff6dae933fad03ec0884092da1d97fcc65a571b5b573a43e67d4432a62465`, code (report excluded) `30fedafdbba57b79fe4c13e2de0658aa3f05f527b778a242cbfab9c4ba253d76` (both recomputed: match; 370 files, 0 hash/mode mismatches; `/tmp/opencode/LP-final-frozen-src-v4` and `/tmp/opencode/LP-final-runtime-src-v4` match all 369 code files)
Delta against the reviewed `37acb9aa` code: exactly four files, `crates/spc-lab/src/convergence.rs`, `crates/spc-lab/src/scenario.rs` (doc comments only), `docs/LAB-RUNTIME.md` (wording), `docs/SPEC.md` (one line removed). No other file changed; nothing outside C1/C2 was reopened.
Probe artifacts: `/tmp/opencode/Fable-final-r2-{convergence-probe.log,lib-tests.log,nix-test.log}`

## Verdict

**APPROVE (final bounded merge revision)** for code aggregate `30fedafdbba57b79fe4c13e2de0658aa3f05f527b778a242cbfab9c4ba253d76` (full `9ffff6dae933...a62465`).

- **C1 fixed.** `convergence.rs:131-135`: `quiesced = false` is set before every attempted resume (so a failed or partial resume also invalidates quiescence), and a later shared view must pause again and be re-observed. My original counterexample (`[tie@8/tie@8, a9@9/tie@8, winner@10 x2]`, `allow_resume`) now yields controls `[pause, resume, pause]`, 4 attempts, and `workers_mining_at_converged = false` (`Fable-final-r2-convergence-probe.log`). The new vector `resumed_common_tip_requires_a_new_pause_and_fresh_revalidation` asserts the same plus consumption of the fourth scripted view; the candidate's fail-first log (`LP-final-c1-fail-first.log`: `left: [false, true]`) and green log are consistent with the pre- and post-fix code. The five earlier vectors are unchanged and green (6/6 convergence, 29/29 lib here; `topology_evidence` 2/2, `manifest_credentials` 1/1, `topology_namespace` 5 passed / 14 ignored by default).
- **C2 fixed.** `docs/SPEC.md` is byte-identical to the approved docs snapshot (`5de914d7...`) and to the `2d6c752d` tree; the convergence record stays in `docs/decisions/` and `docs/changelog/` only, consistent with docs decision 9. The merge session still owns the final SPEC splice.
- **Stated limits** (deadline checked cooperatively between blocking observation rounds; the action runs once after all phases regardless of the declaring phase's position) are now written into `ConvergenceSpec`/`PhaseSpec` doc comments and `docs/LAB-RUNTIME.md` and are not claimed as stronger than they are.
- **Nix binding.** From `/tmp/opencode/LP-final-frozen-src-v4`: `nix eval .#spc-lab.drvPath` = `h1r50rys...` and `nix build` yields `/nix/store/9vkjx7mh...-spc-lab-0.1.0` with `bin/spc-lab` sha256 `7249995c211fff13...dac3`, matching `LP-final-binary-provenance-v4.json`; `checks.x86_64-linux.test` drv `ch6r9yhn...` is realised, its log contains the new C1 test green with 0 FAILED; the v4 flake-check log lists all six checks and no error. Pool, miner and scan binaries are unchanged in content (`aac1278f...`, `443de438...`, `f8000b43...`).
- **Live evidence scope, honestly stated.** The retained native run `LP-final-native-v3` was produced by the v3 binary `0d3d31f6...` on code `37acb9aa`, converged with `attempts: 2` and controls `[pause, pause]`, and never entered the defective resume path; the report now says so (section 4) and makes no new live claim for the v4 binary. The C1 change is confined to the convergence driver's control flow and is covered by the unit vector and the Nix-built test derivation; as stated in the first pass, no new mining run is required for this revision.

## Bounds (unchanged)

Old L2 node `eb257af7...` only; the separately approved v0.9 port and independent live gate do not alter this candidate and no pin swap is implied. `--pow-bits 0x207ffffe`, consensus-link faults only, network isolation not a sandbox, no full header/window/fee/witness/Ark/SV1/causal-retarget/stock-compat or general-convergence claim; network tools are host tools, Nix realisation externally blocked.

## Final binding

This approval binds code aggregate `30fedafdbba57b79fe4c13e2de0658aa3f05f527b778a242cbfab9c4ba253d76` (full `9ffff6dae933fad03ec0884092da1d97fcc65a571b5b573a43e67d4432a62465`), the frozen source `/tmp/opencode/LP-final-frozen-src-v4`, the four v4 Nix outputs recorded in `LP-final-binary-provenance-v4.json`, and the retained v3 live evidence under the scope stated above. Any change inside the code subset requires re-review.
