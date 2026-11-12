# Native audit gate - assigned build turn

This addendum supersedes the earlier source-only status for the checks below.
The original report and every failed invocation remain preserved. Evidence is
under `/tmp/opencode/native-audit-build-turn`; the final handoff is
`/tmp/opencode/native-audit-tested-r1/handoff.json`.

## Results

56 distinct tests passed across the retained invocations:

| Scope | Tests | Passing log stem |
|---|---:|---|
| Public codec: signatures, nonce/epoch/instance/config binding, v2 facts, JSON | 6 | `codec-r2` |
| Real Noise/CPU, simulated wallet: asynchronous gating and cancellation | 9 | `native-r4` |
| Isolated target/extranonce revocation without an accompanying new job | 1 | `native-isolated-revocation-r5` |
| Materialized config tampering and required audit-mode parsing | 1 | `native-config-r5` |
| Existing control/event schema | 7 | `cpu-regressions-r1` (6), `control-fixture-r2` (1) |
| Existing live SV2/control/pacing | 6 | `cpu-protocol-budget-regressions-r1` |
| Protocol/session/ntime | 8 | `cpu-protocol-budget-regressions-r1` |
| Target/hash-budget/extranonce | 9 | `cpu-protocol-budget-regressions-r1` |
| Corrected source-audit full/partial/empty window verification | 9 | `corrected-source-audit-r1` |

CPU/scanner `cargo check --all-targets` and `cargo clippy --all-targets -- -D
warnings` pass. Runtime `cargo check -p spc-lab --lib` and strict library clippy
pass, including the actual peer-publication helper. Formatting and the final
CPU build/seal receipts accompany the final handoff.

Tests explicitly observe zero active worker job, attempts and activation/
commit/first-hash events while the wallet response is withheld. Candidate
changes progress through the dedicated Noise reader while approval is pending.
Valid source-feed/2 signatures admit work only after queued commit/activation
events. Invalid/expired evidence, replacement, disconnect, pause and renewal
are exercised without relaxing signature or epoch checks.

## Corrections made during this turn

- Test fixtures used the wrong SP-address encoder import. Corrected to
  `spc_crypto::address::encode_sp_address`.
- Default tempfile permissions violated the production gate's 0700 directory
  requirement. The gate refused them correctly; fixtures now create private
  directories explicitly under `/tmp/opencode`.
- Added missing explicit unaudited mode to the existing control JSON fixture.
- Retained the frozen inline public DTO APIs. Narrow module-level allowances
  document `large_enum_variant` for the bounded receipt/config/control types;
  the public codec and AddConnection config file remain byte-identical.
- Applied only frozen corrected accounting-lib and source-audit hunks from
  ladder source `2314f6e2ec154d390b476e7656349819d53004bf2144b9220d39b9cf07d5d034`.
  Resulting helper hashes are respectively
  `8f8975aed6a799df588d8541c782834bf9a0cef3ae08c3bf0f0ab00cb8af6654`
  and `e563da68ee408975bc40bb003fde527bc9facfff4afa617bd31700c4f592d7ee`.
  No public source-feed preimage, loaded-key API or upstream pin changed.

## Exact remaining limitations

The private-file runtime unit test could not build: this focused baseline's
`spc-pool` dev dependency still consumes the older `p_x: Option<_>` accounting
API in `front.rs:1251` and `jds.rs:273`. The corrected accounting API has a
vector. The failure is retained as `runtime-r1`; no runtime unit-test pass is
claimed. Re-run it after serial union with the corrected pool/store packet.
Runtime production code itself passed check/clippy.

The native tests use a signed simulated wallet/collector response. They are
not full-source evidence of the actual AP3 service. Still required: AP3's
existing loaded wallet and kernel authorizer, actual wallet-generated instance
publication, controller peer-registry hooks, matching complete source-feed/2
collector history, and QA's joined four-wallet/four-miner 40 H/s run. Funding,
fresh registration and Q-reached campaign requirements remain independent.

## Build provenance

Each invocation acquires the global nonblocking flock, checks this session's
ticket, checks the actual T1a target/store filesystems for >=50 GiB free and
>=20 GiB projected residual, and freezes all source before Cargo runs. The
explicit turn assignment selects the existing `/tmp/opencode/T1a-target`;
the ticket's historical target field is recorded in every preflight receipt.
All commands use `CARGO_INCREMENTAL=0`, offline locked dependencies and the
existing Nix devshell. No primary edits, commits, subagents or cache cleanup
occurred. Full source hashes and binary seals are in the final handoff.
