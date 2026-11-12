# Source-qualified joined gate inventory v1

Status: completed evidence verified and registered for exact source manifest
`033a091fa0b81648fa9dbf3b161000b7783e69878f5480317fc47d4f78cdc838`, Nix source
`/nix/store/2jjpwirz3igg8ij8qplp7h30hg3h255j-source`, and CLI
`ddd7dd270716bfd24006037d758bf6ab91def7db6de33be74556d04b22a40dbf`.
Qualification: `/tmp/opencode/AP5-qualified-350-r1/qualification.json`, SHA256
`9c7790b7feeba1ba1e53ac4c88c7ec086cb8294b44f8745c53cb577d29254cba`.
Verification covered 838 actual source rows, 13 binaries, 19 check-log bindings
and all 16 completed gate-step exits. Canonical admission passed; the actual
fresh diagnostic then hit the pre-hash readiness blocker documented in
`NETWORK-READINESS-LIVE-BLOCKER.md`. The frozen preparation packets retain
their historical pending status.

The earlier 70-test source packet and 332/g105/runtime-fix receipts remain
immutable and source-qualified under their original contracts. This change
does not globally replace 332 with 350 or accept a minimum-count threshold.

## Source-qualified matrix (observed completed passes)

| Declared suite/check | Expected passed | Command selection |
| --- | ---: | --- |
| agent-ark-tests | 162 | spc-agent, explicit ark feature |
| pool-ark-tests | 31 | spc-pool, explicit ark feature, ark:: filter |
| lib | 84 | spc-lab --lib |
| node | 36 | three actual Node helper-script invocations |
| ui-browser-tests | 15 | compiled_browser.cjs against the bound CLI |
| command-idempotency | 6 | --test command_idempotency |
| command-concurrency | 1 | --test command_concurrency |
| engine-restart | 3 | --test engine_restart |
| traffic-payment-view | 1 | --test traffic_payment_view |
| node-p2p-wiring | 9 | --test node_p2p_wiring |
| reap-cleanup | 2 | --test reap_cleanup |

Total: 350 passed. Rust case lines and complete runner-group summaries must
agree, with zero failures. The agent suite's exact source-declared exclusion
`8:regenerate_fixture` has reason `ignored, fixture generation; run explicitly
to (re)create the committed file` and is not counted in 162 or 350. Other
unqualified skips reject. Node case lines and invocation
summaries must agree. Browser proof case names must match the required/passed
case list and the admitted binary. Empty/truncated/counter-only logs, duplicate
cases, swapped suite targets, missing suites and substituted logs reject.

The nine node-p2p-wiring cases use FixtureNodeControl and assert rendered argv
and shape behavior. They do not prove two real rbitcoin nodes propagated a
block. Source qualification explicitly records `fixture-rendered-argv-only`
and `live_propagation_proved: false`. The compiled observed-peer gate must be
present in the qualified source; upcoming live common-chain observation must
prove actual A-to-B propagation before funding. No manual block relay is
authorized and no env-skipped optional live test is counted as that proof.

## Versioned source qualification

`source_scope.py` retains the 332 branch qualified to the historical 4d3778
source-manifest/root pair; the g105 received-format reader is likewise bound
to its original source pair. A new source cannot relabel itself 332 to omit
the added regression suites. The distinct
`spc.lab.ap5.joined-350-gate/v1` branch calls `gate_inventory.py` and requires a
source-specific immutable qualification. `joined-gate-qualifications.json`
maps the **exact joined source-manifest SHA256** to `{path, sha256}` for that
completed qualification. Only the exact source above is registered for 350.
Neither a different root nor a replaced binary/log can borrow this inventory.

After actual completion and source verification, the qualification has the
closed schema `spc.lab.ap5.joined-inventory-qualification/v1` and these fields:

- `schema`, `gate_schema`, `source_root`: exact schema and actual Nix input.
- `source_manifest`, `validation`, `gate`: exact artifact references from the
  qualified bundle/policy. Gate remains the producer's closed six-field object.
- `app_binaries`: all six application SHA256 values.
- `source_files`: the complete manifest path-to-hash map, including the actual
  engine, observed-peer gate, supervisor, evidence, CLI and network/reap tests.
- `check_inventory`: every completed joined check record keyed by its exact
  name, including its source/binary/feature/exit/log facts. No omissions or
  substitutions are allowed.
- `command_source`: hashed actual gate command script/record used to establish
  the per-suite argv/CWD mapping. Its mapping must be verified when producing
  the qualification, not inferred from a count or test name.
- `suites`: exactly the matrix above. Each entry has `expected_passed`, `log`,
   `argv`, `cwd_relative`, `cases`, `ignored_cases`, plus `proof` for browser tests. Rust commands
  select the exact package/feature/target. Node helper argv is the three actual
  invocations with their source-relative CWD, not an invented combined command.
  `cases` is the exact executed case-identity list retained from the final log.
- `p2p_test_scope: "fixture-rendered-argv-only"` and
  `live_propagation_proved: false`.

Admission verifies the qualification's artifact seal, root/source/validation
bindings, all source rows, the complete check inventory, required feature/build
and compiled UI checks, exact per-suite log/proof evidence set, parsed case
lists and total. The same count on an old/broken source cannot borrow the new
qualification. A self-consistent fabricated counter still fails if case lines
do not support it; changing logs cannot match a pinned final qualification.

## Completion workflow

For each new source, verify actual completed logs, exits, source and binary
rows and the command mapping; freeze its qualification and digest. Then run
canonical admission and the authorized diagnostic. This source's canonical
check is `/tmp/opencode/AP5-qualified-350-check-r1`; its actual run is
`/tmp/opencode/AP5-network-native-live-r1`. Offline replay confirmed the sealed
inconclusive result. The nine fixture tests did not establish live propagation.

The old network failure, compiled verifier 5330c3..., R4 control source policy,
global unreadable-PID coverage and native-audit/Rule B scopes remain separate.
Source qualification confers no propagation, funded or full-acceptance claim.
