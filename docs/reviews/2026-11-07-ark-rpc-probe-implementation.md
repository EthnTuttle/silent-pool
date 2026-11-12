# Implementation Review — AP-1a ark-rpc read-only node capability probe (2026-11-07)

> **Superseded 2026-11-08 by `2026-11-08-ark-rpc-probe-r1.md`** (the Fable
> F1-F9 revision + the T0 traffic-slice `getmempoolinfo` addition). This
> document is kept as the v1-era historical record; its frozen evidence
> (`reports/l2-eb257af7`, `reports/v09lab-bcb6d5ee`, schema v1) and its
> contract hash `48894548...` refer to the pre-revision tree and were
> never approved. The line-87 correction below was applied in the r1
> revision (Fable F1).

**Status: COMPLETE — probe, tests, and real two-node evidence delivered**

Worktree: `lab-ark-rpc-probe` (base commit `b0ab4ca34fda07c61a6335d05f598eaacc69678c`, changes uncommitted for review).
Scope owned: `testing/ark-rpc/**` and this file. No node, pool, agent, primary-tree, or pin changes.

---

## 1. What was built

A safe, read-only RPC capability probe (`testing/ark-rpc/`, Python 3 stdlib only) that checks rbitcoin regtest nodes against the RPC surface and response shapes the pinned bark `c419159` actually parses, with real evidence from the two approved node binaries:

| node | binary sha256 | RPC version | verdict |
|---|---|---|---|
| `l2-eb257af7` | `eb257af7c10cdf0ded584adfadf2ae8124466304b13d26c17f855e76f9320f7c` | 190000 | `incompatible` (`fail-version-gate`, `incomplete`) |
| `v09lab-bcb6d5ee` | `bcb6d5eefe0dd0bd96e2ac79e173b4b99a91f67f7717dae6620c30786378a171` | 190000 | `incompatible` (`fail-version-gate`, `incomplete`) |

The version gate **fails honestly on both nodes** — bark requires >= 310000 (server, `server/src/bitcoind.rs:31`) and >= 290000 (wallet, `bark/src/chain.rs:51`); both binaries report `RPC_COMPAT_VERSION = 190000`. This is the expected, correct outcome of this packet. No Core version is faked, shimmed, proxied, or overridden anywhere.

Evidence: `testing/ark-rpc/reports/{l2-eb257af7,v09lab-bcb6d5ee}/{report.json,report.md,raw_responses.json,manifest.json}` and `reports/gap-matrix.md`. Each `manifest.json` freezes the base commit, the sha256 of all 28 probe source files, the node binary hash, argv, dialect provenance, and the contract hash.

## 2. Source authority (how the required surface was derived)

REQUIRED RPC calls were derived by reading the pinned bark source and the **exact crate versions its Cargo.lock resolves** — not from checklist method names:

- `bark c419159 server/src/bitcoind.rs` — version gate 31_00_00; tip/deep_tip via `getblockcount`+`getblockhash`; `getblockheader` for tx_status; `getrawtransaction` 0/1; `gettxout`; `getrawmempool`; `getmempoolentry` chunk shape (`chunkweight`, `fees.chunk` as_btc); `testmempoolaccept`; `submitpackage` (`package_msg == "success"`); `sendrawtransaction`.
- `bark c419159 bark/src/chain.rs` — wallet gate 290000; fee targets 1/3/6 economical; `getmempoolentry` ancestor info (bitcoincore-rpc-json 0.19 shape); broadcast paths; fallback fee on missing feerate.
- `bark c419159 bitcoin-ext/src/rpc/mod.rs` — `require_txindex` (fails closed when `getindexinfo` has no txindex), `try_get_tx_out` (null-or-GetTxOutResult), `GetRawTransactionResult` (camelCase), `SubmitPackageResult` (`tx-results` map, `package_msg`).
- bitcoind-async-client 0.15.0 (features `30_2` + `raw_rpc`) over corepc-types 0.13.0 `v29::GetBlockchainInfo` — the async shape **requires** `bits`/`target`/`time` (rbitcoin emits them only when a tip header exists; a fresh node has the genesis tip).
- bitcoincore-rpc-json 0.19.0 (bdk_bitcoind_rpc 0.22.0 sync companion) — `GetBlockchainInfoResult` (no bits/target/time; `warnings` string-or-array), `GetBlockHeaderResult` (`rename_all = "camelCase"`: parsed keys are `nTx`/`versionHex`; `bits` is a String), `GetMempoolEntryResult` (`vsize` aliased to `size`), `EstimateSmartFeeResult` (optional `feerate`), `TestMempoolAcceptResult`.
- bitcoin-units 0.1.3 `amount::serde::as_btc` — amounts deserialize via `f64`, so JSON strings are rejected; >= 0; <= 8 decimals; must fit u64 sats.

Every contract in `arkrpc/contracts.py` cites its consuming Rust type. Contract semantics mirror serde strictly: missing non-Option field = error; Option accepts absent AND null; unknown fields ignored; bool/float never accepted for int; `as_btc` = JSON number only, exact 8-decimal check via `decimal.Decimal`.

## 3. Verification strength (what is proven vs not)

Two axes per check (availability / behavior), statuses `passed`/`failed`/`unknown`/`not_run`:

- **Proven passed (both nodes)**: `getnetworkinfo` shape; `getblockchaininfo` network=regtest + both crate shapes; `getindexinfo` txindex present and synced; `getblockcount`; `getblockhash` genesis == Core regtest genesis; `getblock` verbosity 0 with **independent re-hash of the 80-byte header == requested hash**; `getblockheader` verbose strict shape + genesis hash/height/nTx (incl. the camelCase `nTx` key trap); `gettxout` null-or-contract for the genesis coinbase outpoint (include_mempool true/false); `getrawmempool` shape; `estimatesmartfee` 1/3/6 shape; method recognition of all three mutating methods (invalid sample `"00"` → `server_error_-22` from both binaries, proving routing without exercising funded behavior).
- **Failed (both nodes, honestly)**: version gates — 190000 < 310000 (server) and < 290000 (wallet).
- **Unknown (ran but unproven, never a pass)**: `getrawtransaction` 0/1 — rbitcoin answers code -5 for the genesis coinbase (it does not serve it even with txindex; the verbose-tx shapes need a funded fixture); `getmempoolentry` entry shapes (server chunk shape and wallet ancestor shape need a funded mempool entry); `estimatesmartfee.fee-data` — empty mempool returns errors+blocks, no feerate.
- **Not_run (deliberate)**: funded broadcast behavior of `testmempoolaccept`/`submitpackage`/`sendrawtransaction`. Invalid samples prove METHOD recognition only, never funded behavior. This classification is load-bearing: a funded-fixture packet is a separate opt-in.

Verdict logic (frozen, mirrored by `report.schema.json` and self-tested): any failure -> `incompatible` with ordered reasons (`fail-version-gate`, `fail-method-surface`, `fail-network`, `fail-genesis`, `fail-shape`, then `incomplete`); unknowns/not_run without failures -> `incomplete`; **`full_compat` is unreachable on an unfunded node by design — version 310000 alone must NOT yield full acceptance** (self-test scenario `version_310000_only` pins exactly this: gates pass, verdict stays `incomplete`).

## 4. Gap matrix summary

Both nodes are **identical** on every check (see `reports/gap-matrix.md`): everything passes except the two version-gate rows (failed), with the unknown/not_run rows listed above. There is no surface or shape divergence between the old L2 build and the v0.9+lab build on the bark-c419159 contract — the only blocker for both is the version gate, plus the unproven funded paths.

Notable observations recorded in the reports: `subversion` `/rbitcoin:0.8.99/` (L2) vs `/rbitcoin:0.9.0/` (v0.9); `getzmqnotifications` absent on both (-32601) — bark polls via `getblockcount`/`getblockhash`/`getrawmempool`, so ZMQ absence is not a gap for the pinned bark; `RBITCOIN_RPC_PACKAGE_DIALECT=1` set by the launcher (RPC-surface knob, not consensus; provenance in every manifest).

## 5. AP-1 handoff: named compat profile decision inputs

The parent plan proposes a future named `--rpc-compat-profile bark-v31` (exactly 310000 + Core package dialect, default off 190000, regtest only, real rbitcoin identity in subversion + explicit profile metadata). **No node patch is approved yet**; this packet changes nothing there. The inputs this probe hands to that decision:

1. **The version number is the only failing gate** on both binaries; every other availability/shape check on the bark-c419159 surface passes (modulo the unknowns that need funded data).
2. The named profile must remain a named constant in code (this probe's `bark-c419159` profile is the pattern: profiles are constants, `--expected-network`/`--expected-genesis` are sanity assertions that fail closed on disagreement, never free-form configuration).
3. Any `bark-v31` claim must keep availability/shape vs funded behavior separate: even with 310000, funded paths and live mempool/fee data stay `unknown`/`not_run`, so acceptance cannot be "full" without the funded-fixture packet.
4. The probe's JSON contract set (`arkrpc.contracts.canonical_contract_json()`, sha256 `48894548fc322b2b54da4d2e30c4ca9c8eadb20c72e7607ca86e7a8b12a3a4b7` at this tree) is the frozen interface. The future Rust `node_probe` (per the plan) must consume this same contract set rather than re-derive it; the sha256 recorded in every run manifest detects drift. This Python implementation is retained (not discarded) as the reference and self-test oracle for that absorption.

## 6. Non-claims

- Funded broadcast behavior untested (`not_run`); needs the separate funded-fixture opt-in packet.
- No transaction funding, mining, or block generation occurred; nodes stayed read-only; the only transaction examined is the genesis coinbase.
- rbitcoin does not serve the genesis coinbase via `getrawtransaction` (code -5) and returns `null` from `gettxout` for it — verbose-tx and non-null-GetTxOutResult shapes are `unknown`.
- Fee data on an empty mempool is `unknown` (node returns errors+blocks only), never a pass.
- ZMQ is absent from rbitcoin; documented as an observation, not a gap (bark polls).
- No Core version override, shim, response proxy, or client guard disable exists anywhere in this packet.

## 7. Safety and privacy properties

- Nodes are fresh per run (`/tmp/opencode/AP1a-<token>/<label>/datadir`), loopback RPC on a free port, `--no-listen --no-seeds --no-discover` (no peers), bounded by `--max-run-secs`, reaped by the probe (SIGTERM -> SIGKILL -> waitpid verification), exit status recorded.
- The RPC credential is launcher-generated, lives only in a cookie file (`user:pass`, 0600, no trailing newline; the node reads it and rejects wider modes), never in argv or logs; reports record only the cookie basename. All recorded free text passes through `redact()`; the committed evidence contains no credentials (test-enforced).
- No live user QA/services were touched; the review-gallery node on 18481 was not touched; no manual restarts of anything.
- CLI: credential via `--cookie-file` only; per-call deadlines; bounded response bodies; error categories (transport/timeout/auth/http/method_missing/server_error_*/parse/bad_jsonrpc/oversized); no config-driven code execution (profiles and contracts are code constants).

## 8. Tests and reproduction

- 49 unit tests (`python3 -m unittest discover -s tests`): contract negative fixtures per field (missing/wrong-type/bool-as-int/float-as-int/8+-decimal amounts/negative/over-supply, `nTx` vs `n_tx` camelCase trap, `vsize`/`size` alias, null-vs-absent Option, submitpackage keys, string-or-array warnings), scenario suite, schema validation of the committed evidence, credential-leak scan of all committed evidence.
- 10 self-test scenarios (`./probe.py self-test`) against an in-process fake JSON-RPC server replaying the real-node capture: baseline 190000, version 310000 alone, version missing, version wrong type, wrong network, wrong genesis, auth fail (401), method not found (-32601), shape mismatch (missing `bits`: async fails, sync passes), no txindex.
- Full reproduction: `./run_two_nodes.sh` (self-test gate, then both fresh owned-node probes, then the gap matrix). Exact commands in `testing/ark-rpc/README.md`.

## 9. Findings for reviewers

1. **rbitcoin `getrawtransaction` cannot serve the genesis coinbase** (code -5) even with txindex synced. Harmless for bark boarding/rounds (they query funded round transactions), but it means the verbose-tx contract cannot be verified on an unfunded node. If a future funded-fixture packet runs, re-check `getrawtransaction.verbose` first.
2. ~~`getindexinfo` reports `best_block_height` (matching bitcoincore-rpc-json 0.19's `IndexStatus`); Core itself uses `best_block_hash`.~~ **CORRECTED 2026-11-08 (Fable F1): the struck claim is false.** Bitcoin Core's own documented `getindexinfo` response uses `best_block_height` ("The block height to which the index is synced") — exactly what corepc-types 0.13.0 models from the Core v0.21 API docs (`src/v21/util.rs`, re-exported unchanged through v29/v30) and what bitcoincore-rpc-json 0.19's `IndexStatus` declares. `best_block_height` is therefore the Core-compatible baseline, not a divergence: rbitcoin's emission is Core-compatible and the probe check is correct as written. Nothing to revisit for the `bark-v31` profile on this axis.
3. The async `GetBlockchainInfo` requires `bits`/`target`, which rbitcoin emits only when a tip header exists. A node still doing headers-only init would fail the async shape. Fresh regtest nodes always have the genesis tip, so this is theoretical, but the check documents the dependency.
4. `getnetworkinfo.networks[].proxy` is `null` on rbitcoin vs `String` in bitcoincore-rpc-json 0.19. No bark consumer deserializes that struct (bark reads only `version`), so the probe's observation set accepts both and records the divergence in `contracts.py`.

## 10. Decision requested

None blocking. Fable to review code + evidence; the named `--rpc-compat-profile bark-v31` decision (and any node-side version change) proceeds separately per the parent plan. This packet is read-only evidence infrastructure and requires no spec change; per repo convention a decision record was not added because no normative behavior changed.
