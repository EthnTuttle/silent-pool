# AP-1b r1: rbitcoin `--rpc-compat-profile bark-v31` (parent-corrected candidate)

Date: 2026-11-08
Author: TFF implementation agent
Status: r1 implementation complete (bounded-scope correction of the r0
packet applied), requesting the single parent Fable review of the entire
corrected node candidate; **candidate only - not adopted, not merged, no
pin change**
Worktree: `/home/ethan/.local/share/opencode/worktree/ad8886/lab-node-compat-profile`
Base: `b0ab4ca34fda07c61a6335d05f598eaacc69678c` (nothing committed)
Node candidate source: `/tmp/opencode/AP1b-bark-profile` (r1 state), derived
from the approved L2 tree `/tmp/opencode/rbitcoin-lab-retarget-profile`
(base `e8cf9c9d61f7c63ae59cc11ab1179cace50ca142` + the 18-file Fable-approved
snapshot `d2f37b91...`; receipt `testing/rbitcoin-bark-v31/baseline-binding.json`)
r0 packet: archived byte-identical under `testing/rbitcoin-bark-v31/r0/`
(all 39 files; the parent's 27-file initial manifest
`/tmp/opencode/AP1b-initial-packet-manifest.json`, snapshot
`016b90b885cb8e3131cb59962f3ac7b00b210446f7676916adfe0444d7890dd7`,
verified against it). The r0 report stays frozen at
`docs/reviews/2026-11-08-bark-v31-profile-implementation.md`; this r1 file
is the corrected report. `testing/rbitcoin-bark-v31/r1-manifest.json` is the
versioned replacement manifest (every file's r0 -> r1 status).
Probe: the frozen AP-1a ark-rpc probe, unchanged (re-verified after all r1
work against the r2 freeze manifest: 74 files, 0 mismatches), contract
sha256 `f4fc3e043f420870675095ab24144ba14e0afba6cb9ad69d818d99b1bccb2f62`.

Authorization boundary respected: implementation only. No primary adoption,
no `UPSTREAM_DEPS.md` pin change, no v0.9 adoption, no commit, no push, no
subagents, no cache cleanup. The approved L2 tree and its binary
(`eb257af7c10cdf0ded584adfadf2ae8124466304b13d26c17f855e76f9320f7c`) were
never written (re-verified after all r1 work).

---

## 1. The r1 corrections (what changed vs r0 and why)

1. **`RBITCOIN_RPC_COMPAT_PROFILE` env knob removed.** The parent did not
   retroactively approve that new `RBITCOIN_*` env surface (node ORIENT
   47-50 ask-first rule). The r1 candidate implements the reviewed named
   CLI flag `--rpc-compat-profile bark-v31` and conf key
   `rpc_compat_profile=bark-v31` only. `NodeEnv.rpc_compat_profile` and the
   `env_profile` parser are gone; an env variable of that name is inert
   (unknown `RBITCOIN_*` names are ignored), asserted by a test written
   failing-first. The existing `RBITCOIN_RPC_PACKAGE_DIALECT` input remains
   the actual policy input (tri-state: unset / on / explicit off); explicit
   off + profile still fails closed at startup, and default dialect
   semantics are unchanged from the baseline. The plan's env form stays
   *proposed*; the decision fragment now says explicitly that the candidate
   is CLI/conf-only and does not claim all planned forms are implemented.
2. **False probe claim corrected.** The r0 report claimed the frozen probe
   could only express profile-on via the env form. That was wrong: the
   approved probe CLI has an external mode (`probe.py probe --rpc-url ...
   --cookie-file ...`, `arkrpc/cli.py:369-381`) for an already-running
   node. The two r1 probe runs therefore launch **owned fresh candidate
   nodes with an explicit CLI argv** (private cookie, private scratch
   datadir, fresh port) through the packet-owned harness
   `testing/rbitcoin-bark-v31/r1-external-run.py`, and run the frozen
   probe unchanged in external mode. The harness writes an
   `external-node-binding.json` per run (argv, pid, binary sha256, source
   name, effective `getnetworkinfo` config, startup banner, devcheck
   receipt, cleanup); it never fabricates the probe's internal OwnedNode
   fields, which external-mode reports do not carry by design. No proxy,
   no wrapper, no response changes, no frozen-probe edits. The harness also
   refuses to start if `RBITCOIN_RPC_COMPAT_PROFILE` is set, so no run can
   depend on the removed knob.
3. **Node owner docs and changelog added.** r0 wrongly reported "only code,
   no changelog". The r1 candidate adds the node changelog fragment
   `changelog.d/rpc-compat-profile-bark-v31.md` and the owner-doc updates
   per the node doc map: `docs/rpc.md` owns the RPC surface contract (new
   "Opt-in named compat profile `bark-v31`" section + an operator-knob
   table row) and `COMPAT.md` owns the intentional-difference statement
   (under "Core-class JSON-RPC (subset)"). `OPERATOR.md` routes to
   `docs/rpc.md` for RPC knobs and needed no separate row. Content: opt-in
   regtest named claim, real identity preserved, existing Core package
   dialect selected, default 190000, known funded-unproven and the selected
   mempool modelling consumer's `maxdatacarriersize` mismatch left
   mismatched.
4. **Initial packet preserved under a versioned r0 archive**; new r1
   candidate files are separate, with `r1-manifest.json` as the clear
   versioned replacement manifest covering the testing packet (r0/
   archive excluded) plus both review docs, recording every file's
   r0 -> r1 status (unchanged / replaced / new). The aggregate and counts
   live in that file itself; they are deliberately not restated here
   because the manifest hashes this document (restating them would be a
   circular, always-stale copy).

All already-verified r0 logic is kept except what these corrections
required (env removal, docs, the new external harness).

## 2. What the candidate is (unchanged from r0 in substance)

A default-off, regtest-only named RPC compatibility profile:

- `getnetworkinfo.version` reports **310000** (the bark `c419159` server
  floor) while `subversion` keeps the real rbitcoin identity and every
  other field is unchanged; the response appends
  `rbitcoin_compat {profile:"bark-v31", claimed_version:310000,
  package_dialect:"core-is-child-with-parents"}`.
- Startup banner:
  `rpc_compat_profile=bark-v31 claimed_version=310000 software=rbitcoin software_version=0.8.99`.
- The profile selects the existing Core `IsChildWithParents`
  `submitpackage` dialect through the existing
  `RBITCOIN_RPC_PACKAGE_DIALECT` code path; one policy resolution path
  (`NodeConfig::resolve_rpc_env`), called once from
  `finish_operator_config`.
- Fail closed: unknown profile name (exit 2 at CLI parse / conf error),
  non-regtest network (config validation before the datadir or store
  opens), explicit dialect off + profile (startup error). No arbitrary
  version integer, no proxying, no response rewriting, no bark
  version-guard bypass, no consensus/P2P/default-output change.

## 3. Change inventory (r1 node candidate, 14 diverged files)

Unified diff `testing/rbitcoin-bark-v31/node-candidate.patch` (939 lines,
sha256 `97f84755b8ca470a3cc64145649eafae732913c2e2a02a116abfbe855fbb2a95`;
applies cleanly to the approved L2 tree including the new file,
`patch --dry-run` verified). Candidate manifest
`candidate-manifest.json` (same recipe as the L2 snapshot manifest; 25
records = 18 snapshot files + every diverged/new candidate file; aggregate
`f8a5111119d9e0df79cc8987f23d4b380c6ecad2df094b4c8196b2e2b04ba6d3`).

| Path | Kind | Change |
|---|---|---|
| `crates/rbitcoin-rpc/src/methods/net.rs` | src | `RpcCompatProfile` enum; `getnetworkinfo` claim + `rbitcoin_compat` (r1: doc comment now names CLI/conf only) |
| `crates/rbitcoin-rpc/src/methods/mod.rs`, `src/server.rs` | src | `RpcContext`/`RpcConfig` carry `compat_profile` |
| `crates/rbitcoin-node/src/config.rs` | src | `RpcOpts.compat_profile` (CLI/conf); tri-state `rpc_package_dialect`; `resolve_rpc_env`; `apply_kv` key; regtest-only validation. **r1: `NodeEnv.rpc_compat_profile` + `env_profile` removed** |
| `crates/rbitcoin-node/src/cli.rs` | src | usage text; `finish_operator_config` calls `resolve_rpc_env` (NodeError -> exit 2) |
| `crates/rbitcoin-node/src/run.rs` | src | `RpcConfig` wiring; startup banner; esplora template stays `None` |
| `crates/rbitcoin-node/tests/cli.rs` | test | r1: `rpc_compat_profile_is_bark_v31_regtest_cli_only` (default off, both CLI spellings, conf key, unknown names, signet refusal, CLI profile + dialect-off conflict) and `rpc_compat_profile_env_is_not_a_knob` (env inert, written failing-first); `env_flags` tri-state rework kept; malformed-env entry for the removed knob dropped |
| `crates/rbitcoin-rpc/tests/methods/mod.rs`, `regtest_chain_ops_journey.rs` | test | the two r0 counterfactual/journey tests (unchanged in r1) |
| `crates/rbitcoin-rpc/tests/server.rs`, `crates/rbitcoin-node/tests/health/metrics.rs` | test | mechanical `compat_profile: None` literals |
| `changelog.d/rpc-compat-profile-bark-v31.md` | new | node changelog fragment (r1) |
| `docs/rpc.md` | docs | "Opt-in named compat profile `bark-v31`" section + operator-knob row (r1) |
| `COMPAT.md` | docs | intentional-difference statement under "Core-class JSON-RPC (subset)" (r1) |

## 4. Verification (failing-first, targeted per the r1 correction scope)

- **Red first (r1)**: `rpc_compat_profile_env_is_not_a_knob` failed against
  the r0 code with `env must not activate` (receipt
  `test-receipts/red-node.exit`, EXIT 101) before the knob was removed.
  The r0 Red receipts (missing-surface compile errors) are archived under
  `r0/test-receipts/`.
- **Green (targeted, `-j2`, nix-shell rustc 1.95.0, same AP1b target dir)**:
  - `cargo test -p rbitcoin-node --test suite rpc_compat_profile`: **2
    passed** (CLI/conf-only + env-inert).
  - `env_flags`: **1 passed**; `malformed_env`: **1 passed**; `conf`
    filter: **37 passed**.
  - `cargo test -p rbitcoin-rpc --test suite compat_profile`: **2 passed**
    (the r0 counterfactual/journey tests, re-run because a net.rs doc
    comment changed).
  - `cargo clippy -p rbitcoin-rpc -p rbitcoin-node --all-targets`: clean;
    `cargo fmt -- --check`: clean.
- **Full-suite runs were not repeated in r1** (per the correction scope):
  the r0 full runs (rpc suite 214/214, node suite 117/117) covered the r0
  candidate; the r1 delta is confined to node env/config/CLI parsing, the
  docs, and the removed knob, which the targeted tests above cover. Truthful
  flaky record from r0: the first r0 full rpc-suite run had **4 failures**
  (`invalidate_*` x3, `request_task_generate_is_an_active_command`) -
  timing-flaky under parallel load; each passed isolated and the full suite
  passed on rerun. That is a rerun-based clearance of flaky tests, **not**
  an unconditional no-regression proof; the baseline L2 suite receipt
  (212/212, Nov 5) is the comparison point.
- **Startup fail-closed checks** (r0 binary, same code paths re-verified in
  r1 tests): unknown profile exit 2; non-regtest fails before effects.
- Build budget: `-j2`, incremental in the existing AP1b-scoped target dir;
  23G free before and after the r1 build (never close to exhaustion; no
  whole-workspace build; no cargo mutants; no mainnet datadir).

## 5. Runtime proof (frozen probe, external mode, explicit CLI)

The devcheck golden-vector gate ran first, exactly as the probe's own
`run_two_nodes.sh` does: **exit 0, 12 vectors**
(`devcheck-receipt.json`; also bound inside each run's binding because the
external probe mode has no `--devcheck-receipt` option).

Both runs: owned fresh candidate node launched by the packet harness with
an explicit CLI argv (`--network regtest --datadir <private scratch>
--rpc --rpc-listen 127.0.0.1:<fresh port> --rpc-cookie-file <0600
single-run cookie> --no-listen --no-seeds --no-discover --max-run-secs
300`, plus `--rpc-compat-profile bark-v31` only in the profile run), env
mirroring the probe's OwnedNode dialect default
(`RBITCOIN_RPC_PACKAGE_DIALECT=1`), `RBITCOIN_RPC_COMPAT_PROFILE` verified
unset. The frozen probe then ran unchanged in external mode
(`probe.py probe --rpc-url ... --cookie-file ... --expected-network
regtest`). Scratch (cookie, token, datadir) destroyed after each run;
cleanup receipts in each `external-node-binding.json` and
`cleanup-receipt.json`.

**Run 1 - default (no profile flag):** verdict `incompatible`, reasons
`fail-version-gate, funded-unproven`; effective config observed
`version 190000`, no `rbitcoin_compat`, no banner. The counterfactual
holds: default is off and nothing else changed.

**Run 2 - profile ON via explicit CLI flag:** verdict
`surface_compatible`, reasons `funded-unproven`, `full_acceptance: false`;
server gate `310000 >= 310000`, wallet gate `310000 >= 290000`; effective
config observed `version 310000`, `subversion /rbitcoin:0.8.99/`,
`rbitcoin_compat` present; banner
`rpc_compat_profile=bark-v31 claimed_version=310000 software=rbitcoin software_version=0.8.99`.

Honest reading, unchanged from r0: `surface_compatible` is a read-only
surface and encoding verdict, not full compatibility; funded behavior is
structurally unproven by a read-only probe; and the `getmempoolinfo`
check row **failed** (`maxdatacarriersize: expected JSON integer, got
null`) - the known traffic-shape limitation of the selected v30 modelling
consumer, deliberately **not patched** (patching it would launder traffic
compatibility). The candidate binary for both runs is
`b75dad84bdd6663673a29c07352d10c4f78d2dcd14e885f62ad3209d214443e4`
(source name `AP1b-bark-profile-r1`), bound in both
`external-node-binding.json` files and `binary-build-binding.json`.

## 6. Boundary checks

- Approved L2 tree + binary re-verified untouched after all r1 work
  (18/18 manifest files, aggregate, binary sha).
- Frozen probe tree re-verified untouched (74/74 freeze-manifest files);
  devcheck ran through its own `run.sh` and wrote only to `/tmp/opencode`
  and its pre-existing target dir.
- r0 archive verified byte-identical to the parent's 27-file initial
  manifest (aggregate `016b90b8...` reproduced).
- Worktree `git status` at freeze shows only the owned packet paths and
  the two review docs (r0 + r1); nothing committed.
- No silent-pool identity data involved; nothing crosses the ark role.

## 7. What this packet does NOT claim

- Not a claim of full bark-v31 compatibility (limited, named RPC-surface
  claim; funded behavior always unproven by the read-only probe;
  `full_acceptance` false).
- Not a claim that the plan's env form is implemented - it is not; it
  remains proposed pending its own ask-first approval.
- The `getmempoolinfo.maxdatacarriersize` traffic row remains failed and
  unpatched (separate limitation).
- No full-suite r1 rerun (targeted coverage per the correction scope;
  r0 full-suite results documented with their truthful flaky history).
- Not adopted: no primary tree, pin, or `UPSTREAM_DEPS.md` change.

## 8. Handoff

The parent Fable review covers the entire corrected node candidate once
(patch, manifest, docs, changelog, tests, probe evidence). If approved,
the serial merge owner applies `node-candidate.patch` to a fresh
L2-derived tree, promotes the decision fragment (recording the CLI/conf-only
scope and the still-proposed env form), and re-runs the frozen probe
against the merged binary. Any future `RBITCOIN_RPC_COMPAT_PROFILE` env
knob needs its own ask-first approval in the node repo first.
