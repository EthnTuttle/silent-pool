# Closed confirmed-funding admission contract for the serial owner

This corrects the historical mis-scoped admission hold. R1's private-entry
LOCK check belongs to the new wallet-control module; sharing the pool role
does not put that code in the current no-control diagnostic binary.

Supply this policy only after obtaining the actual completed Nix closure's
source input and the exact successful joined gate. The serial owner's existing
`pool-source-identity/v1` receipt is also supported, as specified below. Its
real g105 bundle passed canonical admission; live attempts found the separate
[Runtime bootstrap blocker](RUNTIME-BOOTSTRAP-BLOCKER.md). Consumer fixtures
simulate the Nix boundary and are labelled separately from those actual checks.

## Canonical admission path

Call the existing `ap5.py check-bundle --through confirmed-funding ...`.
The same `common.check_bundle(bundle, full=False)` used by `run` verifies all
artifact hashes, all six application derivers and their common actual Nix
`src`, all 13 exact critical rows, pinned node/server hashes, PostgreSQL
binding, history 3, one 40 H/s bootstrap/worker authority, peer wiring and
registration-before-workers. Both profiles require explicit Ark-feature
agent/pool tests/builds and the compiled UI/browser gate.

It then inventories the actual complete `bins/spc-pool/**` file tree and
compares it with the joined manifest. Symlinks/unlisted files are rejected.
Known control source paths, control identifiers in Rust/Cargo source, or an
installed client prevent a no-control classification. A declared client with
absent code is unbound and rejected. Classification precedes source-policy
selection; a caller-supplied class label cannot hide new code.

- **Control code present (even in a diagnostic):** require
  `pool_control_source_policy` with matching `source_root`, joined
  `source_manifest_sha256`, every actual pool file in `files`, and corrected
  tested/compiled source admitted after the same-auditor recheck. Exact R4
  `d00299c2...` / `f7649004...` with review `956ba9ab...` is now eligible; its
  four control/proof/client source rows must match the actual joined Nix input.
  This layer approval cannot be attached to pre-control g105. dd9/55a and
  unknown/unbound control implementations remain refused in all scopes.
- **No control code:** reject any supplied `pool_control_source_policy` or
  `pool_wallet_executor`. Full AP5 is unavailable. Only the explicitly bound
  `confirmed-funding` diagnostic below is eligible.

## Bundle fields

Add `scope: "confirmed-funding"` and
`diagnostic_source_policy: {path, sha256}`. Retain the normal exact joined
manifest, validation receipt and binary artifacts. Do not attach an unrelated
dd9 policy as "best available". No new accept/approval flag exists.
New executable bundles also require the explicit `probe_invocation` tuple
defined in PROBE-INVOCATION-HANDOFF.md. The wrapper reference alone does not
bind the executed interpreter, script, imported source or prefix arguments.

## Policy: exact top-level fields

All objects described as closed below reject missing or extra fields. SHA256
values are lowercase hex. Artifact references are `{path, sha256}`.

| Field | Required value/binding |
| --- | --- |
| `schema` | `spc.lab.ap5.confirmed-funding-source-policy/v1` |
| `scope` | `confirmed-funding` |
| `source_class` | `legacy-no-wallet-control` for the current diagnostic; `wallet-control` only after that actual source separately clears control admission |
| `source_root` | Exact common Nix deriver `src`, not a primary worktree path |
| `source_manifest_sha256` | Exact bundle joined-manifest artifact hash |
| `joined_validation_sha256` | Exact bundle joined-validation artifact hash |
| `app_binaries` | Closed logical map lab/txd/pool/agent/miner/scanner to actual binary SHA256 |
| `pool_files` | Complete actual `bins/spc-pool/**` path-to-hash map, equal to source and manifest census |
| `absent_control_paths` | Ordered list below for no-control; empty for admitted control-bearing source |
| `absent_control_executables` | `["pool-wallet-executor"]` for no-control; empty for admitted control-bearing source |
| `joined_gate` | Artifact for the closed joined-332 record below |
| `legacy_maintenance` | Closed bounded maintenance declaration below |
| `limits` | Exact fixed object below |
| `runtime_bindings` | Exact fixed/binary/profile bindings below |

Absent control paths, in order:

1. `bins/spc-pool/src/ark/wallet_control.rs`
2. `bins/spc-pool/src/ark/wallet_control_io.rs`
3. `bins/spc-pool/src/ark/wallet_control_proof.rs`
4. `bins/spc-pool/src/bin/pool-wallet-executor.rs`

`limits` is exactly:

```json
{
  "controller_commands": ["bootstrap_fund", "send_user_payment", "stop_run"],
  "components": ["fund-pool-a", "fund-pool-b", "fund-server-alice", "fund-server-bob"],
  "ark_lifecycle_acceptance": false,
  "aggregate_acceptance": false,
  "whole_lab_complete": false
}
```

`runtime_bindings` is closed: `node_sha256`, `server_sha256`, `probe_sha256`
match the actual bundle artifacts; `scenario_template_sha256` is SHA256 of
`common.encoded(scenario("ap5-policy-template", 0))`; `history_version: 3`,
`global_hashes_per_second: 40`, `fresh_registration_before_workers: true`,
`host_peering_before_profile_binding: true`, and
`bootstrap_and_workers_single_budget: true`. Runtime scenario comparison
normalizes only run id and seed. This preserves the default two-node/two-pool/
four-agent/four-miner census, canonical node policy and traffic profile.

## Exact joined gate

The completed source-qualified 350 revision is registered separately in
[GATE-INVENTORY-CONTRACT-v1.md](GATE-INVENTORY-CONTRACT-v1.md). It does not
replace the historical 332 binding globally. Exact manifest 033a091f is bound
to qualification 9c7790b7 after completed source/log/case verification; a draft
`passed: 350` value cannot admit another source. The qualified diagnostic was
executed and rejected before hashing at its observed-peer gate.

Closed `spc.lab.ap5.joined-332-gate/v1` object: `schema`,
`source_manifest_sha256`, `checks_sha256`, integer `passed: 332`, integer
`failed: 0`, and nonempty `evidence` (hashed raw gate/log artifacts).
`checks_sha256` is SHA256 of `common.encoded(joined_validation["checks"])`.
This binds the complete actual check set; it does not convert an old count,
default-feature agent run or incomplete UI test into a joined gate. The
existing feature-aware six check purposes and UI source census are enforced
independently by `check_bundle` for this diagnostic too.

## Legacy maintenance: actual exposure, not absent-code R1

The inspected legacy source starts sync/maintenance and a 60-second loop that
scans height-derived pool coinbases and refills after maturity. Native funding
to the separately published onchain wallet address is not that height-scan
input. Automatic maintenance/refill can nevertheless run during a diagnostic;
do not assert it is disabled or that a Python timeout proves a height barrier.

Closed `legacy_maintenance` fields:

- `mode: "enabled-source-tested-observation-only"`
- `max_runtime_secs`: positive integer, at most 7200. Both the harness deadline
  and actual runtime `--duration-secs` are bounded by it.
- `native_transfer_boarding: "not-in-legacy-height-scan"`
- `automatic_coinbase_refill: "may-run-no-ark-acceptance"`
- `coverage`: artifact for the closed record below.

Closed coverage record: `schema: "spc.lab.ap5.legacy-maintenance-coverage/v1"`,
same `source_manifest_sha256` and exact `pool_files`, `covered_paths` exactly
`["initial-sync", "periodic-maintenance", "matured-coinbase-refill", "failure-retry"]`,
`check_purpose: "pool-ark-tests"`, its exact hashed `log`, and
`conclusion: "supported-for-bounded-native-funding-observation"`.
The log must contain executed passing `ark::board::` cases. The serial owner
must substantiate this source/test coverage, including retry/ambiguity
behavior and native-input exclusion; the 332 counter alone is insufficient.
If an affected reachable funded maintenance path lacks this coverage, report
that actual deficiency. Do not supply the absent dd9 R1 policy, fabricate a
NoEffect proof, or reinterpret this coverage as funded Ark validation.

## Enforced scope and replay

The diagnostic controller cannot call pool wallet APIs (`board_float`,
`board_proof_v1`, or snapshot), wallet maintenance commands, SP/stock proof
collection or full lifecycle execution. It accepts only the four raw native
funding components. Its HTTP command allowlist is bootstrap_fund (only the
declared txd-0/node-a route), send_user_payment and stop_run. Native bootstrap
remains declared in the runtime scenario, as required before provisioning.
Asynchronous command observation reads the original same-ID journal; it never
reposts a mutation or allocates a replacement id. The built g105 phase runner
still needs the same-ID bootstrap wait fix documented in the blocker report.

After cleanup it emits `native-funding-verdict`, schema
`spc.lab.ap5.native-funding-verdict/v1`, with `diagnostic_status`, the four
component census, cleanup and source bindings. It never calls the full AP5
verdict aggregator. `ark_lifecycle_ready`, `funded_ark_lifecycle`,
`aggregate_acceptance` and `whole_lab_complete` remain false.

Terminal result remains `spc.lab.ap5.result/v2`, with `scope`,
`native_funding_verdict: {file, sha256}`, `diagnostic_status` and status
`native-funding-diagnostic-passed|failed|inconclusive`. It contains no full
`component_verdict` reference. Archive replay enforces the same admission,
rejects Ark command events or non-native component payloads, and computes only
the native diagnostic verdict. Full AP5 still requires corrected control,
aggregate policy, compiled verifier and actual lifecycle proof.

## Received serial-owner schema: actual g105/f2c96b bundle

`received_profile.py` supports the supplied `pool_source_identity` artifact,
schema `spc.lab.ap5.pool-source-identity/v1`. Its closed fields are `schema`,
`scope`, `pool_binary`, `pool_deriver`, `nix_src_root`, `pool_source_layers` and
`pool_control_packet_dd9r3_binding`. The exact diagnostic scope strings from
the serial receipt are recognized; arbitrary prose or full-claim aliases are
not. No fields are inserted into or rewritten in the parent bundle.

Canonical `check_bundle` still performs the common six-deriver/root, artifact,
13-critical-row, node/server pin, PostgreSQL, probe and runtime contract checks.
The received-format reader additionally verifies the complete actual Nix source
census (833 rows for this bundle), all 44 pool rows, no control implementation
or installed client, the real pool deriver, and Nix build flags selecting all
six apps plus spc-pool/rpc, spc-pool/ark and spc-agent/ark.

It validates all 14 hashed green check logs and their source snapshots,
allowing only the three explicitly recorded non-Rust post-gate changes
(flake wiring/vendor hash, runtime Nix txd argument, parent HTML report).
It recomputes 296 passing Rust tests including 162 Ark-agent tests, 36 passing
Node tests, and the separate 15 compiled-browser cases. Compiled UI route
bytes must equal the actual Nix assets; gated debug binaries remain hash-bound.
This validates the actual supplied gate, rather than inventing the draft's
six-purpose receipt or claiming a new pool-Ark test suite was run.

The known legacy main/board/Bark-adapter rows are explicitly bound. Maintenance
and matured coinbase refill remain enabled within the 7200-second observation
bound; native-address transfers are not inputs to the height-derived refill
scan. This profile does not claim audited funded maintenance, native-gate,
warmup, aggregate or Ark lifecycle acceptance. No guessed source policy or
runtime accept-any-source flag is used. Control-bearing builds cannot take
this no-control receipt path even if they use the same diagnostic label.

The pinned probe wrapper's resolution is checked against the bundle's explicit
Nix Python ELF interpreter, same-package `share/ark-rpc/probe.py`, source-file
manifest and one-script prefix via the supported `--traffic-probe-arg`
interface. The same full tuple is retained in a `probe-invocation` event and
compared against launch argv during replay. The supervisor's no-wrapper rule
is preserved; `node-probe` is appended by runtime, not added to prefix args.
