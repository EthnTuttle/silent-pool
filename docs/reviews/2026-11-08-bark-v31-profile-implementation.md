# AP-1b: rbitcoin `--rpc-compat-profile bark-v31` (node compat profile candidate)

Date: 2026-11-08
Author: TFF implementation agent
Status: implementation complete, requesting Fable review; **candidate only -
not adopted, not merged, no pin change**
Worktree: `/home/ethan/.local/share/opencode/worktree/ad8886/lab-node-compat-profile`
(packet only; the node candidate lives outside the repo, see below)
Base: `b0ab4ca34fda07c61a6335d05f598eaacc69678c` (worktree; nothing committed)
Node candidate source: `/tmp/opencode/AP1b-bark-profile`, derived by copy from
the approved L2 tree `/tmp/opencode/rbitcoin-lab-retarget-profile`
(base `e8cf9c9d61f7c63ae59cc11ab1179cace50ca142` + the 18-file Fable-approved
snapshot `d2f37b9113e8855245866289ed393043aed6e6153e8101375072cbb23624d499`,
verified byte-identical at copy time - receipt
`testing/rbitcoin-bark-v31/baseline-binding.json`)
Plan: `docs/plans/2026-11-07-lab-ark-services.md` section 5.4 / AP-1 (approved
profile contract)
Probe: the frozen AP1a ark-rpc probe (Fable APPROVE r2,
`docs/reviews/2026-11-08-ark-rpc-probe-fable.md`), contract sha256
`f4fc3e043f420870675095ab24144ba14e0afba6cb9ad69d818d99b1bccb2f62`, run
unchanged (probe tree re-verified against the r2 freeze manifest
`/tmp/opencode/lab-ark-rpc-probe-r2-review-manifest.json`, snapshot
`f0aa504aa6f247474eb247b4c25d12f286bb3f88abb1d42e77c3ba643f65ce12`:
74 files, 0 mismatches)

Authorization boundary respected: implementation only. No primary adoption,
no `UPSTREAM_DEPS.md` pin change, no v0.9 adoption, no commit, no push. The
approved L2 tree and its binary
(`/tmp/opencode/rbitcoin-lab-retarget-profile/target/debug/rbitcoin-node`,
sha256 `eb257af7c10cdf0ded584adfadf2ae8124466304b13d26c17f855e76f9320f7c`)
were never written (re-verified after all work: all 18 snapshot files and the
binary hash match).

---

## 1. What was built

A named, default-off RPC compatibility profile for the rbitcoin node, exactly
as the approved plan section 5.4 names it: `--rpc-compat-profile bark-v31`
(CLI and conf key `rpc_compat_profile`) plus the env form
`RBITCOIN_RPC_COMPAT_PROFILE=bark-v31`. When set, and only on regtest:

- `getnetworkinfo.version` reports `310000` (the bark `c419159` server floor)
  instead of the node's own `190000` RPC shape floor, while `subversion`
  keeps the real rbitcoin software identity and every other field is
  unchanged.
- The response carries a structured `rbitcoin_compat` object:
  `{"profile":"bark-v31","claimed_version":310000,"package_dialect":"core-is-child-with-parents"}`.
- The node selects the existing Core `IsChildWithParents` `submitpackage`
  dialect through the existing `RBITCOIN_RPC_PACKAGE_DIALECT` code path
  (the profile turns the same internal flag on).
- A startup log line states the claim explicitly:
  `rpc_compat_profile=bark-v31 claimed_version=310000 software=rbitcoin software_version=0.8.99`.

What it deliberately is NOT:

- No arbitrary version knob: the enum `RpcCompatProfile` has exactly one
  variant; any other value (CLI, conf, or env) is a config error naming the
  accepted value.
- Regtest only: `NodeConfig::validate()` rejects the profile on any other
  network before the datadir or store is touched (fail closed, before
  effects).
- No proxying, no response rewriting, no bark version-guard bypass, no
  change to consensus, P2P, or default RPC outputs. Default behavior is
  byte-identical (asserted: default `getnetworkinfo` still reports `190000`
  and carries no `rbitcoin_compat` key).
- One policy resolution path: `NodeConfig::resolve_rpc_env(&mut self, env)`
  is the single place the profile and the package dialect resolve into
  `RpcOpts`, called once from `finish_operator_config` after CLI/conf/env are
  all in. An explicit `RBITCOIN_RPC_PACKAGE_DIALECT=0` contradicts the
  profile's Core dialect claim and is a startup config error (exit 2), never
  a silent resolution.

### Why both a CLI flag and an env var

The approved plan names both forms (section 5.4: `--rpc-compat-profile` and
`RBITCOIN_RPC_COMPAT_PROFILE`). The env form is also load-bearing for
verification: the frozen AP1a probe launches its owned nodes by inheriting
`os.environ` (with `RBITCOIN_RPC_PACKAGE_DIALECT=1` preset and no
extra-argv option), so the profile-on probe run is only expressible through
the env form. **Ask-first note:** `RBITCOIN_RPC_COMPAT_PROFILE` is a new
`RBITCOIN_*` knob; per the node repo's ask-first rule this is flagged here
for the reviewer rather than pre-approved. It is not a tunable (one accepted
value, fail-closed otherwise), and it exists to serve the already-approved
probe mechanism.

### Single-profile CLI/env agreement

With one accepted profile name, CLI and env cannot disagree on *which*
profile: both parse to the same variant or both fail. `resolve_rpc_env`
therefore uses a simple `or`-fill (no unreachable conflict branch). A second
profile would need an explicit CLI-vs-env conflict rule recorded in
`docs/decisions/` before it is added; the proposed decision fragment says
so (`testing/rbitcoin-bark-v31/decision-fragment.md`, PROPOSED, not
binding).

## 2. Change inventory (node candidate)

11 files diverge from the approved L2 tree (6 source, 5 test), unified diff
`testing/rbitcoin-bark-v31/node-candidate.patch` (863 lines, sha256
`93894700a1db7851367ec197bc38268b454ed5528cae87ad01e76f8c5ac1c0e2`; applies
cleanly to the L2 tree, `patch --dry-run` verified). No changelog.d, docs,
or owner-file changes in the node tree; the only non-code text change is the
node's own `--help` usage lines.

| Path | Kind | Change |
|---|---|---|
| `crates/rbitcoin-rpc/src/methods/net.rs` | src | `RpcCompatProfile` enum (name/claimed_version/package_dialect/parse) beside `RPC_COMPAT_VERSION`; `getnetworkinfo` reports the claim and appends `rbitcoin_compat` when a profile is set |
| `crates/rbitcoin-rpc/src/methods/mod.rs` | src | re-export `RpcCompatProfile`; `RpcContext.compat_profile: Option<RpcCompatProfile>` |
| `crates/rbitcoin-rpc/src/server.rs` | src | `RpcConfig.compat_profile`; threaded into the served `RpcContext` |
| `crates/rbitcoin-node/src/config.rs` | src | `RpcOpts.compat_profile`; `NodeEnv.rpc_compat_profile` + tri-state `rpc_package_dialect: Option<bool>` (`env_flag_opt`, `env_profile` helpers); `resolve_rpc_env` (the one policy path); `apply_kv` key `rpc_compat_profile`; regtest-only check in `validate()` |
| `crates/rbitcoin-node/src/cli.rs` | src | usage text; `finish_operator_config` calls `resolve_rpc_env` (NodeError -> exit 2), replacing the old `package_dialect |= env.rpc_package_dialect` absorb |
| `crates/rbitcoin-node/src/run.rs` | src | `RpcConfig` literal gains the profile; startup `info!` banner; esplora template `RpcContext` stays `None` |
| `crates/rbitcoin-rpc/tests/methods/mod.rs` | test | new `getnetworkinfo_compat_profile_reports_claim_not_identity` (default-vs-claim counterfactual, field-by-field equality) |
| `crates/rbitcoin-rpc/tests/regtest_chain_ops_journey.rs` | test | new `chain_ops_compat_profile_bark_v31_claim_and_dialect` journey test: claim + real subversion + the dialect actually enforced on `submitpackage` |
| `crates/rbitcoin-node/tests/cli.rs` | test | new `rpc_compat_profile_is_bark_v31_regtest_only` (default off, both CLI spellings, unknown names, signet refusal, env form, dialect conflict, malformed env); `env_flags_take_one_true_zero_or_false` reworked for the tri-state dialect |
| `crates/rbitcoin-node/tests/health/metrics.rs`, `crates/rbitcoin-rpc/tests/server.rs` | test | mechanical `compat_profile: None` in `RpcContext` literals |

Candidate manifest: `testing/rbitcoin-bark-v31/candidate-manifest.json`
(same recipe as the L2 snapshot manifest: base
`e8cf9c9d61f7c63ae59cc11ab1179cace50ca142`, 24 file records = the 18
approved snapshot files with candidate hashes + the 6 diverged files outside
the snapshot set; aggregate sha256
`b3c64ae56655b39e0ff10a145db1e2a1437d20b718de52a96644b309c3ed8fca`).
Copy fidelity note: the initial rsync missed `.cargo/mutants.toml`
(cargo-mutants config, unrelated to this work); it was restored
byte-identical to L2 before manifesting, so the candidate tree is exactly
L2 + the 11-file patch.

## 3. Verification (failing-first)

Counterfactual tests were written and run RED before any implementation
change (receipts `testing/rbitcoin-bark-v31/test-receipts/red-*.exit`):

- `red-rpc`: EXIT 101 - E0609 no field `compat_profile`, E0433 no
  `RpcCompatProfile` in scope (exactly the missing surface).
- `red-node`: EXIT 101 - same E0609 through the node crate.

Green (all `-j2`, nix-shell rustc 1.95.0, candidate-scoped
`CARGO_TARGET_DIR`):

- `cargo test -p rbitcoin-rpc --test suite compat_profile`: **2 passed**
  (the two new tests).
- `cargo test -p rbitcoin-node --test suite rpc_compat_profile`: **1
  passed**; `env_flags`: **1 passed**.
- `cargo test -p rbitcoin-rpc --test suite` (full): **214 passed, 0
  failed** on rerun. The first full run had 4 failures
  (`invalidate_*` x3, `request_task_generate_is_an_active_command`) - all
  timing-flaky under parallel load: each passed isolated, and the whole
  suite passed on the rerun (receipts
  `green-rpc-suite{,-rerun,-invalidate-rerun,-reqtask-rerun}`). The L2
  baseline suite was 212 passed / 0 failed (Nov 5 receipt,
  `/tmp/opencode/suite-rpc.log`), so 214 = 212 + the 2 new tests, no
  regression.
- `cargo test -p rbitcoin-node --test suite` (full): **117 passed, 0
  failed** (includes the reworked env-flag test and the touched
  `health::metrics` tests).
- `cargo clippy -p rbitcoin-rpc -p rbitcoin-node --all-targets`: clean.
- `cargo fmt -p rbitcoin-rpc -p rbitcoin-node -- --check`: clean.

Startup fail-closed checks run against the built candidate binary:

- `--network signet --rpc-compat-profile bark-v31`: configuration error
  `--rpc-compat-profile bark-v31 requires network=regtest` before the
  datadir/store is created (run-path exit 1, the repo's mapping for
  validate-time config errors).
- `--rpc-compat-profile core-v31`: `unknown rpc-compat-profile "core-v31":
  expected bark-v31`, exit 2 at CLI parse.

Build budget: `-j2`, `CARGO_INCREMENTAL=0`, `CARGO_PROFILE_DEV_DEBUG=0`,
candidate-only `CARGO_TARGET_DIR`. Disk: 34G free before the first build,
26G free after the last (never close to exhaustion; recorded in
`/tmp/opencode/AP1b-green-df-*.txt`). Only `-p rbitcoin-rpc` /
`-p rbitcoin-node` targets were built and tested; no whole-workspace green
claim is made. No cargo mutants (`.cargo/mutants.toml` untouched). No
mainnet datadir was ever created (probe datadirs are regtest, under
`/tmp/opencode`, and are destroyed - section 5).

## 4. Runtime proof (frozen AP1a probe, unchanged)

The approved probe was run twice against the candidate binary
(`cc81e310c7d8fd975db26291108f5ee1547d35077e8a53db37d90ea6901a8e9e`,
55,506,584 bytes, built from the candidate tree; both probe manifests bind
exactly this sha with source name `AP1b-bark-profile`). Both runs:
fresh single-run cookie credentials, fresh ports, regtest, isolated
scratch, cleanup receipt `testing/rbitcoin-bark-v31/cleanup-receipt.json`.

**Run 1 - default (no profile):** verdict `incompatible`, reasons
`fail-version-gate, funded-unproven`. `getnetworkinfo.version` observed
`190000 < 310000` - the honest default, identical to the approved L2
baseline behavior. This is the counterfactual: the profile is off by
default and nothing else changed.

**Run 2 - profile on (`RBITCOIN_RPC_COMPAT_PROFILE=bark-v31` in the probe's
inherited environment):** verdict `surface_compatible`, reasons
`funded-unproven`, `full_acceptance: false`. The server gate passes
(`310000 >= 310000`), the wallet gate passes (`310000 >= 290000`), and every
other read-only check against the bark `c419159` contract passes. The
startup banner in the retained `node.log` (extracted, then scratch
destroyed) reads exactly:
`rpc_compat_profile=bark-v31 claimed_version=310000 software=rbitcoin software_version=0.8.99`.

Honest reading of run 2, per the probe's own semantics:

- `surface_compatible` is a **read-only surface and encoding** verdict, not
  full compatibility. `funded-unproven` is structural: the read-only probe
  can never prove funded broadcast/mempool behavior. This packet does not
  claim "fully compatible with bark v31".
- The `getmempoolinfo` check row **failed**: `maxdatacarriersize: expected
  JSON integer, got null`. This is the known traffic-shape limitation the
  AP1a review already recorded for the selected v30 modelling consumer. It
  is a separate limitation of the node's mempool-info surface, out of scope
  for a version-claim profile, and deliberately **not patched** here:
  patching the node to launder that row would misrepresent traffic
  compatibility. It remains an open row for any future traffic-shape
  packet.

Reports: `testing/rbitcoin-bark-v31/probe-default/` and
`probe-profile-on/` (report.json/report.md/raw_responses.json/manifest.json;
the profile-on directory additionally carries the extracted `node.log`).
Both report the approved contract hash
`f4fc3e043f420870675095ab24144ba14e0afba6cb9ad69d818d99b1bccb2f62`.

## 5. Privacy, cleanup, and boundary checks

- No silent-pool identity data is involved in this packet (node-local RPC
  surface only); nothing crosses the ark role.
- All probe scratch (cookies, tokens, regtest datadirs) destroyed; receipt
  `testing/rbitcoin-bark-v31/cleanup-receipt.json`. The two smoke-test
  datadirs used for the startup fail-closed checks were removed
  immediately.
- The approved L2 tree and binary re-verified untouched after all work
  (18/18 manifest files + binary sha match).
- The frozen probe tree re-verified untouched (74/74 freeze-manifest files
  match). No second Rust probe was written and no AP1b code was absorbed
  into the probe, per scope.
- The silent-pool worktree contains only the owned packet paths
  (`testing/rbitcoin-bark-v31/**`, this review, and the pre-existing
  baseline dirty state); `git status` checked at freeze.

## 6. What this packet does NOT claim

- Not a claim of full bark-v31 compatibility: the profile is a limited,
  named RPC-surface claim (version floor + package dialect + explicit
  metadata). Funded behavior is always unproven by the read-only probe.
- Not adopted: no primary tree, pin, or `UPSTREAM_DEPS.md` change; the
  candidate lives at `/tmp/opencode/AP1b-bark-profile` and as a patch file
  in this packet. Adoption is the parent Fable reviewer's decision.
- The `getmempoolinfo.maxdatacarriersize` traffic row remains failed and
  unpatched (separate limitation, recorded above).
- No multi-region, SV2, or pool-side changes.

## 7. Handoff

If approved, the serial merge owner applies
`testing/rbitcoin-bark-v31/node-candidate.patch` to the node tree (it
applies cleanly to the approved L2 tree), records the decision fragment
(`testing/rbitcoin-bark-v31/decision-fragment.md`, PROPOSED -> accepted,
amended with the second-profile conflict rule if one is ever added), and
re-runs the frozen probe against the merged binary. The env knob
(`RBITCOIN_RPC_COMPAT_PROFILE`) needs the node repo's ask-first approval
recorded in that session.
