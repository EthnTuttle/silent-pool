# Implementation Review r2 — AP-1a ark-rpc read-only node capability probe (2026-11-08)

**Status: COMPLETE — Fable r1 re-review REQUEST_CHANGES findings applied
(C1, C2 blocking; D1, D2, D3, D5 required; D4, D6, D7 non-blocking),
fresh r2 two-node evidence re-run on both approved binaries,
`testing/ark-rpc/**` now FROZEN. Submitted for the same Fable
re-review.**

Worktree: `lab-ark-rpc-probe` (base commit `b0ab4ca34fda07c61a6335d05f598eaacc69678c`, changes uncommitted for review).

Scope owned: `testing/ark-rpc/**` and review docs. No node, pool, agent, primary-tree, or pin changes; nothing committed.

## 0. Provenance

The Fable review file exists at the primary absolute path
`/home/ethan/code/silent-pool/docs/reviews/2026-11-08-ark-rpc-probe-fable.md`;
its r1 re-review (lines 300-594) is the authority for every finding
dispositioned in section 2 below, and each disposition was cross-checked
against that file line by line. Fable's decision was REQUEST_CHANGES:
C1 (error-path id echo not enforced) and C2 (three frozen ENCODINGS
statements contradict the pinned client code) blocking; D1/D2/D3
(document corrections) and D5 (selected consumer + aggregation rule)
required; D4/D6/D7 recommended.

## 1. Canonical contract hash (r2 candidate; NOT yet approved)

```
f4fc3e043f420870675095ab24144ba14e0afba6cb9ad69d818d99b1bccb2f62
```

- Covers `CONTRACTS` + `ENCODINGS` + `CHECKS` + `VERDICT_RULES` via
  `arkrpc.contracts.canonical_contract_json()` (sha256). Embedded and
  verified matching in both r2 reports and both r2 manifests.
- Supersedes (all unapproved, none ever frozen by T0): the v1-era
  `48894548fc322b2b54da4d2e30c4ca9c8eadb20c72e7607ca86e7a8b12a3a4b7`,
  the r1-draft
  `905bf4b82b1200d1f4e4146cdeb5c1ce3e182199cdce64011bcc35fc3837a6ce`,
  and the r1
  `11bdb1007e130a749720f87482f287c1fb65d1e743c142fc4b1ca15a782d89e7`.
- **This hash is a candidate only.** Fable R7 stands: nothing here
  approves it, and no `bark-v31` named compatibility profile, Core
  RPC-version claim, node patch, or bark acceptance is authorized by
  this document.

## 2. Dispositions of the Fable r1 re-review findings

### C1 (code, blocking) — RESOLVED

Error-path id echo is now enforced on both axes:

- `arkrpc/client.py:291-298` (HTTPError branch): when a non-200 body
  parses as a dict and `receipt["id_match"]` is false, the client
  returns `id_mismatch` exactly as the 200 path does (Fable's suggested
  fix, first variant), with `http_status` and the receipt attached.
- `arkrpc/probe.py:124-141` (`ProbeSession.call`): every receipt is
  counted regardless of category — `id_match` False increments
  `id_mismatch_count` even when the call was classified as
  `server_error_<code>` / `method_missing` behind the broken echo.
- `_check_id_echo` now runs after `_observe_zmq`
  (`probe.py:1246-1250`), so the "all N exchanges" count includes the
  ZMQ observation call: "all 30 exchanges (200 and HTTP-error paths
  alike) echoed the numeric request id exactly" in both r2 reports.
- Fake server (`arkrpc/selftest.py`): error bodies are served over HTTP
  500 by default, 404 overridable via `error_http_status`, and
  `id_echo_mode` gained `string-errorpath` (Fable's counterexample),
  `wrong-int`, `missing`, `bool`, `null` modes. Six scenarios pin the
  behavior: `id_string_echo`, `id_string_echo_errorpath` (expects
  `protocol.id-echo` failed and every `*.recognized` / error-path row
  at availability `unknown` — no error code behind a broken echo is
  interpreted), `id_echo_wrong_int`, `id_echo_missing`, `id_echo_bool`,
  `id_echo_null`.

Verified: all 20 scenarios pass; both r2 real-node runs report
`protocol.id-echo` passed/passed over all 30 exchanges including the
9 HTTP-error receipts.

### C2 (contract fidelity, blocking) — RESOLVED (hash re-frozen)

The three ENCODINGS statements are corrected against the pinned client
code, and the probe now sends exactly the wire forms a pinned client
sends:

- `getrawtransaction.bool.sync` (`contracts.py:1127-1135`): sync
  verbose form is `[txid, true]` envelope 2.0 — `handle_defaults`
  (bitcoincore-rpc 0.19 `client.rs:116-147`) strips the trailing null,
  so the `[txid, true, null]` form was never sent by a pinned client
  and is gone from the probe, the fixtures (regenerated from the r2
  capture; the stale entry was dropped), and every scenario. The
  `bool-blockhash.sync` `[txid, true, blockhash]` form (Some(blockhash))
  is unchanged.
- `estimatesmartfee.sync` (`contracts.py:1169-1178`): sync form is
  `[target]`; the "trailing null kept" claim is gone.
- `testmempoolaccept.async` / `submitpackage.async`
  (`contracts.py:1179-1191`): both are now recorded as envelope 1.0
  async `call_raw` forms (the only forms with active bark `c419159`
  callers: `server/src/bitcoind.rs:207-221`, `bark/src/chain.rs:685`,
  `offboards.rs:485`, `exit/transaction_manager.rs:470,497,630`,
  `actions/board.rs:490`). The sync `broadcast_package` form is labeled
  as having no caller in `c419159`.
- Sync `getrawtransaction [txid, false]` has no active bark caller and
  was deliberately NOT added (Fable's fix text did not require it; the
  sync non-verbose path is exercised via `getrawtransaction.verbosity0`
  on the async form, which is what bark sends).

Re-freeze: canonical hash is now `f4fc3e04...` (section 1), embedded
and matching in both r2 reports and manifests.

### D1 (doc, required) — RESOLVED

`docs/reviews/2026-11-08-ark-rpc-probe-r1.md` section 0 and the parent
one-pager's "Provenance disclosure" carry dated corrections (kept
superseded text): the Fable review exists at the primary absolute path
`/home/ethan/code/silent-pool/docs/reviews/2026-11-08-ark-rpc-probe-fable.md`;
dispositions were cross-checked against it.

### D2 (doc, required) — RESOLVED

`r1.md` section 4 now reads run token `AP1a-b1aeb688e017` with a dated
correction note (the stale `AP1a-89410a8483f1` draft token was never in
any committed artifact; the value is read from both r1 `manifest.json`
files).

### D3 (doc, required) — RESOLVED

Parent one-pager v0.9 binary sha256 corrected to
`bcb6d5eefe0dd0bd96e2ac79e173b4b99a91f67f7717dae6620c30786378a171`
(one-character typo, dated correction).

### D4 (classification, non-blocking) — RESOLVED

`getbestblockhash` is reclassified to a new coverage class
`observational` (`contracts.py:1277`, check manifest comment at
`:1212-1216`): an honest row that never gates the verdict in either
direction, consumer stated as the pinned async client's
`get_current_timestamp` form (`v30.rs:107-110`), which bark `c419159`
never calls. `compute_verdict` (`probe.py:1139`) and
`validate_report_consistency` (`schemadef.py:354`) skip
`observational` exactly like `traffic`; `VERDICT_RULES` gained an
`observational_coverage` rule (`contracts.py:1457-1458`). All v29.rs
source citations were replaced with v30.rs (bark compiles the v30
client module). `getnetworkinfo.observe` was already observational.
The r2 evidence is unaffected: the row passes on both nodes.

### D5 (doc, required for T0 handoff) — RESOLVED

- `contracts.py` (getmempoolinfo contract) and this document label the
  consumer as a **SELECTED MODELLING CONSUMER**: corepc-types 0.13.0
  `v30::GetMempoolInfo` via bitcoind-async-client 0.15.0
  `Reader::get_mempool_info` (`v30.rs:128`). It is stated explicitly
  that bark itself never calls `getmempoolinfo` and that the T0
  `RpcNodeControl` route is NOT automatically this consumer — whether
  T0 requires the full v30 superset (and `maxdatacarriersize` as an
  integer rather than JSON null) is the T0 owner's statement to make.
  The observed `passed / failed` row is preserved uncoerced: it is a
  correct result for the selected consumer, not a universal traffic
  failure and not a new bark requirement.
- Aggregation rule for the traffic worker (section 4 below, and
  `testing/ark-rpc/README.md` "Traffic slice" section): every projected
  availability must pass on its own (any availability != `passed` is a
  traffic-surface blocker irrespective of the bark verdict);
  `protocol.id-echo` / `protocol.envelope20.getnetworkinfo` and the
  network/genesis validation apply to the traffic projection
  independent of Bark version; a behavior of `unknown` or `not_run` is
  never a funded pass; the explicit required shape contract per method
  is the T0 consumer's to state.

### D6 (hygiene, non-blocking) — RESOLVED

`remove_scratch()` moved into the outer `finally` in `cmd_node_probe`
(honoring `--keep-scratch`), so the 0600 cookie is cleaned up on the
failure path too. The five v1-era `AP1a-*` scratch dirs under
`/tmp/opencode` (flagged by Fable) were removed after verifying no live
process used them; the r2 run leaves no scratch (verified: the run
token dir is empty and removed).

### D7 (binding, non-blocking) — RESOLVED

- Every r2 `manifest.json` carries a `dev_verification` key binding the
  devcheck gate to the run: sha256 of `devcheck/Cargo.toml`,
  `devcheck/Cargo.lock`, `devcheck/src/main.rs`, the gate's exit code
  (0), vector count (12), and the receipt line. `run_two_nodes.sh`
  runs the gate first and passes the receipt via `--devcheck-receipt`,
  which fails closed (a missing or failing receipt aborts the run
  before any node is started).
- Unit tests now pin the historical r1 evidence schema-only
  (`validate_report_schema_only`, `R1_CONTRACT_HASH == 11bdb100...`)
  and assert every committed r2 `report.json`/`manifest.json`
  `contract_hash` equals the live `contract_hash()` (no longer
  hand-verified only).
- `validate_report_consistency` re-derives each `fail-<kind>` reason
  from the actually failed kinds (not just order), via shared
  reason-derivation logic in `schemadef.py`.
- `devcheck/src/main.rs:45` `GENESIS_MERKLE` typo fixed to the real
  genesis merkle root
  `4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b`
  (with a ground-truth comment); all 12 vectors re-run PASS.

## 3. r2 evidence (fresh, read-only, both approved binaries)

Run token **`AP1a-1fd3d1813c33`** (read from both r2 `manifest.json`
files, where it also appears in both `report.json` files); reports
under `testing/ark-rpc/reports/2026-11-08-r2-*/` (schema v2):

| report dir | node binary sha256 | verdict | checks |
|---|---|---|---|
| `reports/2026-11-08-r2-l2-eb257af7/` | `eb257af7c10cdf0ded584adfadf2ae8124466304b13d26c17f855e76f9320f7c` | `incompatible` (`fail-version-gate`, `funded-unproven`) | 40 |
| `reports/2026-11-08-r2-v09lab-bcb6d5ee/` | `bcb6d5eefe0dd0bd96e2ac79e173b4b99a91f67f7717dae6620c30786378a171` | `incompatible` (`fail-version-gate`, `funded-unproven`) | 40 |

- The 40 check rows are identical across both binaries (availability
  and behavior), including the traffic and observational rows.
- Headline unchanged and honest: both binaries report version `190000`;
  the bark gate fails (`>= 310000` server, `>= 290000` wallet); funded
  broadcast/mempool behavior stays `not_run`/`unknown` (read-only
  mode); `funded_state: unproven`, `full_acceptance: false`, permanent.
- Hash-tree consistency verified: each manifest's `freeze_recipe`
  equals the current `_hash_tree` over the 38 hashed files; each
  report's `contract_hash` equals its manifest's equals the canonical
  `contract_hash()` (`f4fc3e04...`).
- devcheck gate bound into both manifests: exit code 0, 12 vectors,
  source hashes recorded (section 2, D7).
- Historical evidence preserved byte-unchanged: the r1 dirs
  (`2026-11-08-r1-*`, token `AP1a-b1aeb688e017`, hash `11bdb100...`,
  validated schema-only by unit tests) and the frozen v1 dirs
  (`l2-eb257af7`, `v09lab-bcb6d5ee`).
- `reports/gap-matrix.md` regenerated from the r2 reports.

Incident disclosure: the shared machine's disk (`/tmp/opencode` fill by
other agents' build dirs) hit ENOSPC mid-session, truncating one
scenario file mid-write. It was rebuilt, the devcheck target was
rebuilt, and every gate (unit tests, self-test, devcheck) was re-run
green BEFORE the final r2 evidence run above; the r2 dirs were written
after full recovery and nothing under `testing/ark-rpc/` changed after
them.

## 4. T0 traffic-worker projection (lab-traffic-contract)

Schema: `report.checks[]` rows, report schema v2
(`testing/ark-rpc/report.schema.json`). Each row:
`id`, `method`, `availability` (`passed`/`failed`/`unknown`),
`behavior` (`passed`/`failed`/`unknown`/`not_run`), `fail_kind`,
`detail`. Verdict reasons are re-derivable and validated
(`validate_report_consistency`); rows are keyed by check id and
identical across both approved binaries:

| check id | availability / behavior (both nodes) | note |
|---|---|---|
| `sendrawtransaction.recognized` | passed / not_run | method routed (invalid sample honestly rejected, `-22`); funded broadcast deliberately not exercised |
| `getrawmempool` | passed / passed | async `[]` form |
| `getrawmempool.bool-form` | passed / passed | sync `[false]` form |
| `getrawtransaction.verbosity0` | passed / unknown | genesis coinbase `-5` is Core-identical; honest unknown |
| `getrawtransaction.verbosity1` | passed / unknown | same |
| `getrawtransaction.bool-verbose` | passed / unknown | sync wire form `[txid, true]` envelope 2.0 |
| `getrawtransaction.bool-verbose.async` | passed / unknown | async `[txid, true]` envelope 1.0 |
| `getmempoolinfo` | passed / failed | all 13 v30 fields served plus extras, but `maxdatacarriersize` is JSON `null`; the selected modelling consumer's deserializer rejects it. Not a bark-verdict change; whether T0 requires an integer there is the T0 owner's statement |
| `getblock.verbosity0` | passed / passed | |
| `getblock.verbosity0.rehash` | passed / passed | |
| `getblock.verbosity1` | passed / passed | |
| `getblock.singleparam` | passed / passed | |
| `protocol.id-echo` | passed / passed | all 30 exchanges, 200 and HTTP-error paths alike; applies to the traffic surface |
| `protocol.envelope20.getnetworkinfo` | passed / passed | envelope 2.0 handling; applies to the traffic surface |
| `getbestblockhash` | passed / passed | observational (never gates) |

Aggregation rule for the worker (D5): every projected availability
must pass on its own; `protocol.id-echo` /
`protocol.envelope20.getnetworkinfo` and the network/genesis validation
rows apply independent of Bark version; behavior `unknown`/`not_run` is
never a funded pass; the explicit required shape contract per method is
the T0 consumer's to state.

Final hash for the T0 freeze decision: `f4fc3e04...` (section 1) —
**pending Fable approval**; T0 should not freeze it before that
approval lands.

## 5. Verification summary

- Unit tests: `python3 -m unittest discover -s tests` — **113 tests,
  OK** (all green, including the r2-evidence consistency tests and the
  r1 schema-only historical pin).
- Self-test: `./probe.py self-test` — **20 scenarios, 0 failed**
  (including the six id-echo scenarios and `version_310000_only`,
  which must yield `surface_compatible` + `funded-unproven`, never full
  acceptance).
- devcheck: `devcheck/run.sh` (offline, cached build) — **12/12 vectors
  PASS**, receipt bound into both r2 manifests.
- Fixtures regenerated from the fresh r2 capture (27 entries; the stale
  `[txid, true, null]` entry dropped); `captured_run_token` updated.
- `report.schema.json` current (`schema-dump` diff clean).
- README updated for r2 (scenario count, observational class, D5
  wording + aggregation rule, C2 wire-forms note, devcheck binding,
  r2+r1 evidence layout).

## 6. Non-claims (Fable R7, unchanged and standing)

- No funded behavior was exercised or proven: broadcast, mempool-entry,
  and fee-data behaviors remain `not_run`/`unknown` on both nodes.
- The candidate hash `f4fc3e04...` is NOT approved by this document;
  neither are `11bdb100...`, `905bf4b8...`, or `48894548...`.
- No Core RPC-version compatibility claim, no `bark-v31` named profile,
  no node patch or version override, no bark acceptance. The honest
  `fail-version-gate` stands on both binaries.
- AP-1b, AP-2, AP-3 scopes are not touched here.
- `testing/ark-rpc/**` is frozen as of the r2 run; nothing under it may
  change without a new evidence run and a new hash.

## 7. Files

- Changed under `testing/ark-rpc/**` (38 hashed files; see either r2
  manifest's `freeze_recipe` for the authoritative list with hashes).
- New: `reports/2026-11-08-r2-{l2-eb257af7,v09lab-bcb6d5ee}/`,
  regenerated `reports/gap-matrix.md`.
- Docs: this file; dated corrections in
  `docs/reviews/2026-11-08-ark-rpc-probe-r1.md` (D1, D2) and
  `docs/reviews/2026-11-08-ark-rpc-probe-parent-manifest.md` (D1, D3);
  new one-pager
  `docs/reviews/2026-11-08-ark-rpc-probe-parent-manifest-r2.md`.
