# Fable review - AP-1b r1 rbitcoin named RPC compat profile `bark-v31` (2026-11-08)

**Decision: APPROVE (bounded).** The approval covers exactly:

- the node patch `testing/rbitcoin-bark-v31/node-candidate.patch`, sha256
  `97f84755b8ca470a3cc64145649eafae732913c2e2a02a116abfbe855fbb2a95`
  (939 lines, 14 files), equivalently the candidate manifest aggregate
  `f8a5111119d9e0df79cc8987f23d4b380c6ecad2df094b4c8196b2e2b04ba6d3`
  (25 records over base `e8cf9c9d...`), and the binary built from it,
  `b75dad84bdd6663673a29c07352d10c4f78d2dcd14e885f62ad3209d214443e4`;
- the **read-only** profile counterfactual: default `getnetworkinfo.version`
  190000 unchanged; with `--rpc-compat-profile bark-v31` (CLI or conf) the
  node reports 310000 as an explicit, machine-readable claim while the
  real `/rbitcoin:0.8.99/` identity, `protocolversion` and every other
  field stay unchanged, and `submitpackage` is actually gated by Core
  `IsChildWithParents`.

What this approval authorizes: the parent may take the **candidate
integration decision for the lab** on this exact patch. It does **not**
by itself replace the approved L2 node, change any pin or
`UPSTREAM_DEPS.md` row, promote the decision fragment, accept funded
Bark/Ark behavior, or claim Core v31 parity. Those remain the parent's
explicit, separate steps (section 7).

I wrote only this file. No candidate, node, packet, or primary edits; no
commits; no subagents; no cache cleanup; no adoption.

---

## 1. Freeze and provenance (all reproduced from bytes)

| item | result |
|---|---|
| Full review authority `/tmp/opencode/AP1b-r1-full-review-manifest.json` | 82 records, base `b0ab4ca3...`; all 82 `(path, sha256)` match the worktree bytes (27 of them are `*.log` files gitignored by `.gitignore:6`, so the earlier 55-path git-based manifest could not see them); canonical compact JSON of `{base, files}` with sorted keys reproduces `9375653ca90158446b99ae5c02aaee8482153984c68ba53aef447b115df40050` |
| Internal `r1-manifest.json` | 41 records (8 unchanged / 18 replaced / 15 new vs r0), all match bytes; aggregate `8630a38ba2fcca481831bcf2e73f25477b98fbb2a570ff9c15ad660f77ab3d67` reproduced; the 41 files missing from it relative to the 82 are exactly the `r0/` archive (40) plus itself |
| `candidate-manifest.json` | 25 records = the 18 L2 snapshot paths + 7 diverged/new paths; all match `/tmp/opencode/AP1b-bark-profile`; aggregate `f8a51111...` reproduced |
| r0 archive `testing/rbitcoin-bark-v31/r0/` | all 27 records of the parent's `/tmp/opencode/AP1b-initial-packet-manifest.json` match the archived bytes; aggregate `016b90b8...` reproduced. The r0 report `docs/reviews/2026-11-08-bark-v31-profile-implementation.md` is untouched (hash `727765de...` in the 82-file freeze). History preserved |
| Approved L2 tree `/tmp/opencode/rbitcoin-lab-retarget-profile` | HEAD `e8cf9c9d61f7c63ae59cc11ab1179cace50ca142`; all 18 files of `/tmp/opencode/lab-l2-source-review-manifest.json` match; aggregate `d2f37b9113e8855245866289ed393043aed6e6153e8101375072cbb23624d499` reproduced; binary `target/debug/rbitcoin-node` = `eb257af7c10cdf0ded584adfadf2ae8124466304b13d26c17f855e76f9320f7c`, mtime Nov 5 18:07, untouched |
| Candidate vs L2 | `diff -rq` (excluding `target`, `.git`) shows exactly 13 modified files + 1 new `changelog.d/rpc-compat-profile-bark-v31.md`; L2-only entries are `docs/reviews/` and the `result` symlink (deliberately excluded at copy time per `baseline-binding.json`); `.cargo/mutants.toml` identical |
| Patch == candidate | copied the 13 L2 files into scratch, `patch -p1` applied cleanly (exit 0, 14 files), and every patched file is byte-identical (`cmp`) to the candidate tree. No out-of-manifest source change exists |
| Frozen AP-1a probe | `/tmp/opencode/lab-ark-rpc-probe-r2-review-manifest.json`: 74/74 match, aggregate `f0aa504aa6f247474eb247b4c25d12f286bb3f88abb1d42e77c3ba643f65ce12` reproduced; both r1 probe reports carry contract hash `f4fc3e04...`; probe worktree has no changes outside its own untracked packet |
| Binary identity | `rbitcoin-node --version` = `0.8.99`; `strings` finds the five candidate-only literals (`unknown rpc-compat-profile`, `rpc_compat_profile=`, `requires network=regtest`, `RBITCOIN_RPC_PACKAGE_DIALECT is explicitly off`, `core-is-child-with-parents`) in `b75dad84...` and none in L2 `eb257af7...`; **zero** occurrences of `RBITCOIN_RPC_COMPAT_PROFILE` in the candidate binary; no source file under `crates/` is newer than the binary (built 05:36 local after the last edit) |
| Test receipts | r1: `red-node` EXIT 101 with `rpc_compat_profile_env_is_not_a_knob` failing `left: Some(BarkV31) right: None` against r0 code (genuine red); greens: profile 2/2, envflags 1/1, malformed 1/1, conf 37/37, rpc compat 2/2, clippy clean (the "Permission denied" lines are cargo registry cache-clean warnings, not lints), fmt clean, build exit 0. r0 full suites: rpc 210/214 first run then 214/214 rerun with the 4 flaky `invalidate_*`/`request_task_generate` tests also passing isolated; node 117/117 |

The `.cargo` directory mtime in the candidate (05:22) differs from L2 but
its only file is identical; nothing else lives there.

## 2. Independent live counterfactuals (own fresh read-only nodes)

The r1 harness mirrors the probe's OwnedNode default and sets
`RBITCOIN_RPC_PACKAGE_DIALECT=1` in **both** runs, so the packet's runtime
evidence cannot by itself show that the profile, alone, forces the Core
path. I ran 31 launches of the candidate binary `b75dad84...` with a clean
environment (every `RBITCOIN_*` stripped, then only the named variable
added), a never-pre-created private datadir, 0600 single-run cookie,
loopback-only RPC on a fresh port, `--no-listen --no-seeds --no-discover
--max-run-secs 90`, SIGTERM stop, `wait()` reap, scratch destroyed. No
mining, no funding, no bark. The two unrelated `rbitcoin-node` processes on
39001 and 48332 were left alone. Script, stdout and JSON:
`/tmp/opencode/AP1b-fable-r1-scratch/counterfactual.{py,out,json}`
(sha256 `f08efc10...`, `887d7238...`, `807240c5...`). 13 nodes started,
18 refused; all 31 reaped, all scratch gone, no live candidate process.

The `submitpackage` sample is two unrelated 1-input/1-output transactions
spending fabricated prevouts (not child-with-parents); the node rejects
them in every run and `getrawmempool` stays `[]`.

| run | result |
|---|---|
| profile ON, env **unset** | `version 310000`, `subversion /rbitcoin:0.8.99/`, `protocolversion 70016`, `rbitcoin_compat {profile bark-v31, claimed_version 310000, package_dialect core-is-child-with-parents}`, banner `rpc_compat_profile=bark-v31 claimed_version=310000 software=rbitcoin software_version=0.8.99`; `submitpackage` -> **-25 package topology disallowed**. The profile alone selects the Core path |
| default, env unset | `190000`, no `rbitcoin_compat`, no banner; `submitpackage` -> -1 relay-disabled (no topology gate) |
| default, env=1 | `190000`, no compat; `submitpackage` -> -25 (pre-existing knob alone unchanged) |
| default, env=0 | identical to default/unset (explicit off without a profile is still just off) |
| profile ON + env=0 / env=FALSE | exit 2 **before the datadir exists**: `rpc-compat-profile bark-v31 selects package dialect core-is-child-with-parents, but RBITCOIN_RPC_PACKAGE_DIALECT is explicitly off` |
| profile ON + env="" | runs as profile ON (empty = unset, matching the baseline `node-env-malformed` changelog semantics) |
| profile ON + env=yes | exit 2, pre-existing malformed-env error, before datadir |
| `RBITCOIN_RPC_COMPAT_PROFILE=bark-v31`, no flag | `190000`, no compat, no banner, no topology gate: the removed knob is inert |
| profile ON + `--network signet` / `testnet4`; conf `signet` + `rpc_compat_profile=bark-v31` | exit 1, `--rpc-compat-profile bark-v31 requires network=regtest`, datadir never created |
| values `BARK-V31`, `Bark-V31`, `bark_v31`, ` bark-v31`, `bark-v31 `, `core-v31`, `310000`, `--rpc-compat-profile=` | exit 2 `unknown rpc-compat-profile "...": expected bark-v31` (exact, case-sensitive match; no trimming; no integer knob) |
| `--rpc-compat-profile` with no value | exit 2 `--rpc-compat-profile requires a value` (not a bool key) |
| `--rpc-compat-profile=bark-v31`, `--rpc_compat_profile bark-v31`, `--RPC-COMPAT-PROFILE bark-v31` | all activate (the generic `apply_kv` lowercases and maps `-` to `_`, the same rule every operator flag already follows) |
| conf `rpc_compat_profile=bark-v31`, `RPC-Compat-Profile=bark-v31`, `rpc_compat_profile = bark-v31 ` | activate (key normalised, value trimmed by the conf reader); conf `rpc_compat_profile=BARK-V31` -> exit 2 with `conf <path>:1:` prefix |
| conf typo `rpc_compat_profil=` | unknown key warned and ignored (pre-existing conf rule); node runs default-off, so a bark gate would fail honestly rather than pass by accident |
| conf `bark-v31` + CLI `bark-v32` | exit 2 (there is one accepted name, so CLI and conf cannot disagree except by failing) |
| conf `bark-v31` + env=0 | exit 2 conflict, before datadir |

Fail-before-effects holds as claimed: `cli_main` only reaches
`ensure_datadir()` after `operator_config_from_args` (which includes
`resolve_rpc_env`), and `ensure_datadir` runs `validate()` as its first
statement before `create_dir_all`. The non-regtest check sits in
`validate()` (`config.rs:866-873`); the dialect conflict sits in
`resolve_rpc_env` (`config.rs:1214-1227`), called once from
`finish_operator_config` (`cli.rs:202`), which also removed the old
`package_dialect |= env.rpc_package_dialect` line, so there is one policy
path and no second path left beside it.

## 3. Source review of the policy, not just the metadata

- The enforcing site is `crates/rbitcoin-rpc/src/methods/mempool.rs:2168`
  `submitpackage_topology_disallowed(&txs, ctx.package_dialect)`
  (unchanged). The profile reaches it only by setting
  `NodeConfig.rpc.package_dialect = true` in `resolve_rpc_env`, which
  `run.rs:224-225` copies into `RpcConfig` together with
  `compat_profile`, and `server.rs:233-237` copies into `RpcContext`.
- `rbitcoin_compat.package_dialect` is the profile's static label
  (`RpcCompatProfile::package_dialect()`), not a readback of
  `ctx.package_dialect`. On every path reachable from CLI/conf/env the two
  cannot diverge because `resolve_rpc_env` either sets the dialect or
  fails. `RpcConfig`/`RpcContext` are `pub`, but the only production
  constructor in the workspace is `rbitcoin-node/src/run.rs`
  (`rbitcoin-test` depends on the crate without building either struct);
  the other `RpcContext` in `run.rs:2527` is the internal template
  context with `compat_profile: None`. A hand-built context with
  `compat_profile: Some(..)` and `package_dialect: false` would advertise
  a dialect it does not enforce, but that is not a supported operator
  path. Hardening note N3 below; not blocking.
- `getnetworkinfo` (`net.rs:446-477`): `version` is
  `compat_profile.map_or(RPC_COMPAT_VERSION, claimed_version)`; the compat
  object is appended only when the profile is set. The rpc test
  `getnetworkinfo_compat_profile_reports_claim_not_identity` asserts every
  other key equal between the plain and profiled responses; my live runs
  confirm the same key set plus `rbitcoin_compat`.
- `RpcCompatProfile::parse` is an exact string compare with one accepted
  value and an error naming it; there is no integer path and no second
  profile. `env_flag_opt` keeps `env_flag`'s exact accepted spellings
  (`1`/`true`/`0`/`false`, ASCII-case-insensitive words, empty = unset), so
  the three remaining boolean knobs and the default dialect semantics are
  unchanged; the node test `env_flags_take_one_true_zero_or_false` now
  asserts the tri-state for the dialect knob explicitly.
- `NodeEnv::from_env` reads only named keys; no `RBITCOIN_RPC_COMPAT_PROFILE`
  read exists in non-test, non-doc source (grep confirms; binary confirms).
- No consensus, P2P, mining/template, retarget, or default RPC output file
  is touched: the 14 files are the five production files named in the
  report, seven test files, and three docs. The L2 retarget overlay files
  are byte-identical to the approved snapshot.
- Docs land with their owners per the node doc map: `docs/rpc.md` (RPC
  surface: new section + knob row), `COMPAT.md` (intentional difference),
  `changelog.d/rpc-compat-profile-bark-v31.md` (unreleased note, not
  `CHANGELOG.md`). `OPERATOR.md` routes RPC knobs to `docs/rpc.md`. The
  em dashes in the new text match existing house style in those files
  (L2 `docs/rpc.md` already has 8, `COMPAT.md` 11).
- Node ask-first rule (`docs/ORIENT.md` "Ask first": "add an `RBITCOIN_*`
  knob") is respected in r1: the CLI flag and conf key are ordinary
  operator surface, and the env knob is gone.

## 4. Packet evidence and harness

- Both r1 probe runs are honest external-mode runs of the frozen probe
  (`target.kind: remote-url`, contract `f4fc3e04...`) against owned
  candidate nodes with explicit argv; `external-node-binding.json` records
  argv, pid, binary sha, `--source-name`, effective config, banner,
  devcheck (12/12), exit 0 after SIGTERM, cleanup verified. Default:
  `incompatible [fail-version-gate, funded-unproven]`, `190000`, no
  banner. Profile: `surface_compatible [funded-unproven]`,
  `full_acceptance false`, gates `310000 >= 310000` / `>= 290000`, banner
  present. 11 checks remain `not_proven` in both runs; `getmempoolinfo`
  behavior is **failed** (`maxdatacarriersize: expected JSON integer, got
  null`) in both, as the report states, and the candidate deliberately
  does not patch it.
- The frozen probe's own verdict sentence ("every read-only check passed")
  is AP-1a wording that coexists with the failed `getmempoolinfo` row and
  the 11 unproven rows. That is inherited, not this packet's claim; the
  packet's report §5 and §7 state the failed row and the unproven funded
  path correctly. Nothing here should be summarised as "all other checks
  verified".
- Harness `r1-external-run.py` (packet-owned, not part of the frozen
  probe): random 24-byte cookie password, 0600, never recorded; 10 s RPC
  timeout; 90 s readiness; `--max-run-secs 300` bounds the node's life
  even if the harness dies; SIGTERM then `wait(30)` then kill; binary sha
  computed from the launched path. Gaps (N5): the early-exit and
  not-ready paths return without removing scratch (cookie left at rest)
  and the not-ready path terminates without reaping; the probe
  `subprocess.run` is not in a `try/finally`, so a harness exception
  would leave the node up until `--max-run-secs`. Neither path was taken
  in the recorded runs (both bindings show exit 0 and `cleanup.verified:
  true`). `source_name` is a self-asserted label; the source binding
  rests on `binary-build-binding.json` (toolchain, command, target dir),
  the candidate manifest, and my mtime/strings checks, which is adequate
  for a lab candidate but is not a reproducible-build proof.

## 5. Non-blocking notes (no hash change required for this approval)

- **N1 (doc owner gap)** `docs/env-knobs.md:46`, the owner of the residual
  `RBITCOIN_*` inventory, still describes `RBITCOIN_RPC_PACKAGE_DIALECT` as
  a plain on/off knob. The fact "explicit `0` with `--rpc-compat-profile`
  refuses startup" is stated in `docs/rpc.md` and `COMPAT.md` but not
  linked from the knob's own row. One sentence plus a link to the
  `docs/rpc.md` section would satisfy "one fact, one owner". Fold into the
  adoption change or the next node packet; do not fork the patch for it.
- **N2 (stale comment)** `crates/rbitcoin-node/src/config.rs:280-282`
  still says `package_dialect` is "Set only from
  `NodeEnv::rpc_package_dialect`"; it is now also set by the profile via
  `resolve_rpc_env`.
- **N3 (hardening)** `run_rpc` could reject `compat_profile.is_some() &&
  !package_dialect` (or `rbitcoin_compat.package_dialect` could be
  derived from `ctx.package_dialect`) so a future second constructor
  cannot advertise an unenforced dialect. Not reachable today.
- **N4 (message prefix)** a bad CLI value reports `conf rpc_compat_profile:
  unknown rpc-compat-profile ...` because CLI and conf share `apply_kv`;
  exit code and accepted-value hint are right, the word `conf` is
  cosmetic.
- **N5** harness partial-failure cleanup/reap gaps, section 4.
- **N6 (report wording)** r1 report §8 and the decision fragment treat
  Fable approval as the trigger for the serial merge owner to apply the
  patch and promote the fragment. Read it as: Fable approval is a
  necessary input; the parent's explicit adoption decision is a separate
  act (section 7). The fragment's "becomes binding only if the parent
  Fable review approves adoption" should be understood the same way.
- **N7 (COMPAT.md layout)** the new paragraph sits between "Group status
  (intentional scope):" and its table, splitting the lead-in from the
  table it introduces. Cosmetic.
- **N8 (suite coverage)** r1 did not rerun the full rpc/node suites; the
  r1 delta vs r0 is confined to node env/config/CLI parsing, tests and
  docs, which the targeted runs cover. The r0 full-suite result stands with
  its truthful flaky-rerun history. The integration step (already in the
  handoff) should run `cargo test -p rbitcoin-node --test suite` and
  `-p rbitcoin-rpc --test suite` once on the merged tree and keep the
  receipts; a rerun-cleared flake is not a no-regression proof.
- Pre-existing, not introduced here: any `validate()` failure (including
  the non-regtest refusal) happens after logging init and after
  `--api-log` would create its file, but before the datadir, store, or
  listeners. Exit code for `validate()` failures is 1; for parse/resolve
  failures it is 2.

## 6. Confirmed claims of the r1 report (checked, not accepted as stated)

§1 items 1-4 (env knob removed, probe claim corrected, owner docs added,
r0 preserved) are true against bytes. §2 behavior is true at runtime
without the env mirror. §3 inventory matches the diff exactly. §4 receipts
match the exit files and logs. §5 verdicts match `report.json`. §6
boundary checks reproduced (L2 tree, L2 binary, frozen probe, r0 archive,
worktree `git status` shows only the packet paths and the two review docs,
nothing committed). §7 non-claims are accurate and should be carried
forward verbatim into any decision record.

## 7. Explicit scope of this approval, and non-approvals

Approved: the exact patch `97f84755...` / candidate aggregate
`f8a51111...` / binary `b75dad84...` as a **lab-only, regtest-only, opt-in,
named RPC-surface compatibility claim**, and the read-only counterfactual
in section 2. Any byte change to the 14 files is a new candidate.

Not approved or not claimed by this review:

- Automatic replacement of the approved L2 node, any `UPSTREAM_DEPS.md`
  or pin change, or v0.9 adoption. The parent decides adoption
  explicitly; if it does, apply this exact patch to a fresh L2-derived
  tree, re-bind binary/source hashes, run the two suites (N8), rerun the
  frozen probe in external mode with the explicit CLI flag against the
  merged binary, then promote the decision fragment with a `SPEC.md`
  frontmatter entry per silent-pool conventions. No mandatory upstream
  pin bump precedes that; the rbitcoin row in `UPSTREAM_DEPS.md` is a
  reference point, not a gate.
- Full Core v31 compatibility, Bark server startup, Ark readiness, or any
  funded `testmempoolaccept`/`submitpackage`/`sendrawtransaction`/
  `getrawtransaction`/`getmempoolentry`/fee-estimate behavior: all remain
  `not_proven`; `full_acceptance` is false in every read-only run.
- The `getmempoolinfo.maxdatacarriersize: null` row: still failed against
  the selected v30 modelling consumer, deliberately unpatched, a separate
  traffic limitation.
- Any env form of the profile. `RBITCOIN_RPC_COMPAT_PROFILE` is inert and
  must stay so unless the node repo grants its own ask-first approval.
- A general version proxy, a free-form version integer, response
  rewriting, or a second profile name; each would be a new decision.
- AP-1a probe semantics: unchanged and not reopened here.
