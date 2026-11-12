# Implementation Review r1 — AP-1a ark-rpc read-only node capability probe (2026-11-08)

**Status: COMPLETE — Fable F1-F9 revision applied, T0 traffic-slice
`getmempoolinfo` contract/check added, fresh two-node evidence re-run on
both approved binaries.**

Worktree: `lab-ark-rpc-probe` (base commit `b0ab4ca34fda07c61a6335d05f598eaacc69678c`, changes uncommitted for review).
Scope owned: `testing/ark-rpc/**` and review docs. No node, pool, agent, primary-tree, or pin changes; nothing committed.

- Canonical contract set (this revision):
  `arkrpc.contracts.canonical_contract_json()` sha256
  **`11bdb1007e130a749720f87482f287c1fb65d1e743c142fc4b1ca15a782d89e7`**
- This hash supersedes both earlier hashes: the v1-era
  `48894548fc322b2b54da4d2e30c4ca9c8eadb20c72e7607ca86e7a8b12a3a4b7`
  (never approved) and the intermediate r1-draft
  `905bf4b82b1200d1f4e4146cdeb5c1ce3e182199cdce64011bcc35fc3837a6ce`
  (pre-`getmempoolinfo`; also never frozen — T0 was explicitly told not
  to freeze an unfinished hash, and neither was). Only the hash above is
  final for this packet.

## 0. Provenance disclosure (Fable findings)

> **Correction (2026-11-08, r2 revision, Fable D1):** the paragraph
> below was wrong. The Fable review exists at the primary absolute path
> `/home/ethan/code/silent-pool/docs/reviews/2026-11-08-ark-rpc-probe-fable.md`
> (this worktree is a linked worktree of that repo, but the author
> searched only the candidate worktree). The F1-F9 dispositions in §2
> and the C1/C2/D1-D7 dispositions in the r2 review doc
> (`2026-11-08-ark-rpc-probe-r2.md`) were cross-checked against that
> file line by line.

The Fable review file (`docs/reviews/2026-11-08-ark-rpc-probe-fable.md`
or similar) **does not exist anywhere in this worktree or the repo**
(checked `docs/`, `docs/reviews/`, worktree-wide filename and content
search). Findings F1-F9 were received verbatim in the requester's
message and are dispositioned below as received. If the original review
file surfaces, this document should be cross-checked against it.
*(Superseded by the correction above.)*

## 1. What changed in r1

The probe (`testing/ark-rpc/`, Python 3 stdlib only) was revised end to
end per findings F1-F9 and extended with the approved traffic T0 slice
requirement:

- **Manifest v2 / report schema v2**: three-status verdict
  (`incompatible` / `surface_compatible` / `unproven`), permanent
  `funded_state: "unproven"`, permanent `full_acceptance: false`. The
  v1-era `incomplete`/`full_compat` outcomes are gone; the frozen v1
  evidence dirs still validate through a `SCHEMA_V1` dispatch and are
  never rewritten.
- **Exact client-call encodings** (F2/F3): the probe now sends every
  call exactly as a pinned consumer sends it (params form, envelope
  "1.0" vs "2.0", numeric request ids) and tracks id-echo receipts
  (`protocol.id-echo`).
- **devcheck/** (F4): a dev-only standalone Rust crate that validates
  the Python contract semantics against the REAL pinned crate types
  (bitcoincore-rpc-json 0.19.0, corepc-types 0.13.0, bitcoin 0.32.9 /
  bitcoin-units 0.1.3, and the pinned bark checkout's bitcoin-ext).
  12 golden vectors, all PASS; `#devcheck`-marked Python twins.
- **getmempoolinfo traffic check** (T0): additive, owner-side,
  source-grounded, read-only; new coverage class `traffic` (see §3).

## 2. F1-F9 dispositions

| finding | disposition (all applied, tested, re-proven on real nodes) |
|---|---|
| F1 (false `getindexinfo` claim) | The v1 review doc claimed "Core itself uses `best_block_hash`". **False**: Core's documented response uses `best_block_height` ("The block height to which the index is synced"), as modeled by corepc-types 0.13.0 `v21/util.rs` (re-exported unchanged through v29/v30) and declared by bitcoincore-rpc-json 0.19's `IndexStatus`. The probe check was already correct; the review doc line 87 now carries a dated correction. `getindexinfo` passes on both nodes with `best_block_height`. |
| F2 (exact client forms) | Every call is sent in the exact form a pinned consumer sends: `ENCODINGS` in `contracts.py` records method, params form, and envelope for all 30 exchanges (e.g. `getblock [hash, 1]` envelope 2.0 for the bdk emitter; `getblock [hash]` single-param for async `get_block_height`; `estimatesmartfee [target, "ECONOMICAL"]` uppercase literal; `getrawtransaction [txid, true, null]` sync trailing-null form). Encoding-class checks gate the verdict under fail-kind `encoding`. |
| F3 (numeric ids + receipts) | Both pinned clients use NUMERIC request ids (async: usize counter from 0, envelope "1.0"; sync/jsonrpc 0.18: usize nonce from 1, envelope "2.0"). Every exchange records an id-echo receipt; `protocol.id-echo` aggregates them (a string echo breaks the async client whose `Response.id` is `u64`). Self-test scenario `id_string_echo` proves a string echo fails the protocol axis and is never method evidence. |
| F4 (real pinned serde) | Python contracts mirror serde semantics strictly; `devcheck/` proves the mirror against the actual pinned Rust types (anchor enum split, empty hex, u64 amount boundary, null-reject custom Options, v29-requires-bits blockchaininfo, and now the getmempoolinfo v30 shape incl. the observed `maxdatacarriersize` null rejection). 12/12 vectors PASS via `devcheck/run.sh` (verifies the pinned bark checkout commit `c419159`). `getnetworkinfo.observe` (the non-bark superset) never gates the verdict. |
| F5 (canonical contract + 3-status verdict) | `canonical_contract_json()` freezes CONTRACTS + ENCODINGS + CHECKS + VERDICT_RULES; its sha256 (header of this doc) is embedded in every report and manifest. Three-status verdict with ordered reasons (`fail-version-gate`, `fail-method-surface`, `fail-encoding`, `fail-protocol`, `fail-network`, `fail-genesis`, `fail-shape`, `surface-unproven`, `funded-unproven`). `schemadef.validate_report_consistency` mechanically re-derives the verdict from the rows: no verdict/check disagreement, no manifest check silently missing. Unit- and scenario-tested both directions. |
| F6 (redaction) | `client.redact()` is credential-exact: registered secret VALUES are removed, not every line mentioning a word. `register_secret` requires length >= 8. All free text in reports/manifests passes through it; a test scans every committed evidence file for credentials. |
| F7 (pin names) | Node identity in reports is `--source-name` (build label: `rbitcoin-lab-retarget-profile`, `rbitcoin-v0.9-lab`), never a raw local path; binary sha256 recorded alongside. |
| F8 (genesis/protocol honesty) | Protocol errors (-32700/-32600), id mismatches, auth/transport failures are protocol-level and never method evidence (`_failure_axes`: availability UNKNOWN -> verdict `unproven`, never a fake method-missing). The genesis coinbase `getrawtransaction` -5 and `gettxout` null are documented as Core-identical, not rbitcoin quirks. Genesis hash pinned to the Core regtest constant and checked. |
| F9 (cookie hygiene) | The RPC credential is launcher-generated, lives only in a 0600 cookie file, never in argv/logs/reports (only its basename is recorded); scratch dirs incl. cookies are removed by the probes (verified: no AP1a-* scratch remains from these runs); `node-probe` reaps the node (SIGTERM -> SIGKILL -> waitpid). |

## 3. T0 traffic slice: `getmempoolinfo` (new in r1)

Requirement: the approved traffic T0 subset is exactly
`sendrawtransaction`, `getrawmempool`, `getrawtransaction`,
`getmempoolinfo`, `getblock`; the traffic worker
(`ses_e4604a9d0ffeHEKwnwhfH0S61J` in lab-traffic-contract) consumes
THIS probe's canonical report lineage — no second probe, client, or
verdict. `getmempoolinfo` lacked canonical coverage.

**Grounding (source-verified, all in the pinned tree):**

- bark `c419159` itself never calls `getmempoolinfo` — this is NOT a
  bark requirement.
- The consumer is the pinned async client's `Reader` trait:
  bitcoind-async-client 0.15.0 `src/client/v30.rs:128-131`
  (`get_mempool_info`: `call::<GetMempoolInfo>("getmempoolinfo", &[])`,
  envelope "1.0" per `client/mod.rs:189-196`, numeric usize id from 0).
- bark's workspace pins `bitcoind-async-client = { version = "0.15.0",
  features = ["raw_rpc"] }` with defaults ON (`30_2`); the v29/v30
  client modules are mutually exclusive (`client/mod.rs:21-30`), so the
  single deserializer compiled in bark's tree is **corepc-types 0.13.0
  `v30::GetMempoolInfo`**: 13 required fields (loaded, size, bytes,
  usage, total_fee f64, maxmempool, mempoolminfee, minrelaytxfee,
  incrementalrelayfee, unbroadcastcount, fullrbf,
  permitbaremultisig, maxdatacarriersize u64), no Option, no defaults.

**Design: new coverage class `traffic`.** The check emits honest
`passed`/`failed`/`unknown` rows with real fail_kinds, but NEVER gates
the bark verdict in either direction: no fail-kind is accumulated and
no `surface_compatible` requirement applies, because the verdict
answers "compatible with the pinned bark?" and bark does not consume
the method. This is enforced in `probe.compute_verdict`, mirrored in
`schemadef.validate_report_consistency`, documented in
`VERDICT_RULES.traffic_coverage` (inside the frozen contract JSON), and
pinned by three tests (unit + two self-test scenarios:
`version_310000_only` proves a FAILED traffic check does not block
`surface_compatible`; `traffic_mempoolinfo_ok` proves a PASSED traffic
check does not launder an `incompatible` verdict).

**Observed on both approved binaries (r1 evidence):** `getmempoolinfo`
is routed (envelope 1.0, params `[]`) and serves all 13 v30 fields
plus extras (`limitclustercount`, `limitclustersize`, `optimal`,
`orphanage`, `relay_enabled` — correctly ignored; corepc does not build
with deny-unknown-fields), **but `maxdatacarriersize` is JSON `null`
where Bitcoin Core emits an integer**. Plain `u64` (no Option, no
serde default), so the pinned v30 deserializer rejects it: the traffic
check fails honestly on both nodes (`passed/failed`, fail-kind shape).
devcheck vector `getmempoolinfo_shapes` validates exactly this against
the real corepc-types 0.13.0. This is traffic-surface information for
T0, not a bark-verdict change (verdict reasons are unchanged:
`fail-version-gate, funded-unproven`).

## 4. r1 evidence (both approved binaries, final run)

Run token `AP1a-b1aeb688e017` *(corrected 2026-11-08, Fable D2: this
line previously said `AP1a-89410a8483f1`, a stale draft token; the
value here is read from both committed r1 `manifest.json` files, where
it also appears in both `report.json` files)*; reports under
`testing/ark-rpc/reports/2026-11-08-r1-*/` (schema v2):

| node | binary sha256 | verdict | reasons | checks |
|---|---|---|---|---|
| `rbitcoin-lab-retarget-profile-r1` | `eb257af7c10cdf0ded584adfadf2ae8124466304b13d26c17f855e76f9320f7c` | `incompatible` | `fail-version-gate`, `funded-unproven` | 40 |
| `rbitcoin-v0.9-lab-r1` | `bcb6d5eefe0dd0bd96e2ac79e173b4b99a91f67f7717dae6620c30786378a171` | `incompatible` | `fail-version-gate`, `funded-unproven` | 40 |

Both nodes are identical on every check. Version gates fail honestly
(190000 < 310000 server / < 290000 wallet). Everything else on the
bark-c419159 surface passes except the legitimately-unknown rows
(below). The frozen v1 evidence (`reports/l2-eb257af7`,
`reports/v09lab-bcb6d5ee`) is untouched history under `SCHEMA_V1`.

**Failed (both, honestly):** `getnetworkinfo.gate.server`,
`getnetworkinfo.gate.wallet` (version); `getmempoolinfo` (traffic
class; shape: `maxdatacarriersize` null — gates nothing).

**Unknown (ran but unproven, never a pass):**
`getrawtransaction.verbosity0/.verbosity1/.bool-verbose/.bool-verbose.async`
(genesis coinbase -5, Core-identical; shapes need a funded fixture),
`getmempoolentry.recognized` (unfunded), `estimatesmartfee.fee-data`
(empty mempool).

**Not_run (deliberate):** funded broadcast behavior of
`testmempoolaccept`/`submitpackage`/`sendrawtransaction` (invalid
samples prove recognition only). `funded_state` is `unproven` and
`full_acceptance` is `false` in every read-only run by construction; no
funded pass is fabricated anywhere.

**Manifests:** each r1 `manifest.json` freezes the base commit, the
sha256 of every source file under `testing/ark-rpc` at probe time
(reports/, devcheck/, caches excluded), the node binary hash +
`source_name`, argv, dialect provenance, and the contract hash. Both
manifests verified consistent with the final tree (no mismatched or
missing files).

## 5. T0 projection interface (the deliverable to the traffic worker)

The traffic worker consumes the canonical report lineage directly:
`report.json` (schema v2; validated by `schemadef.validate_report`;
`report.contract_hash` must equal the final hash in this doc's header).
Rows live in `report.checks[]` as
`{id, method, availability, behavior, fail_kind, detail, evidence}`;
statuses are `passed|failed|unknown|not_run` on the two axes.
`report.verdict` is the **bark** verdict and is NOT a traffic verdict:
no Bark version floor is imposed on the traffic projection — the honest
`fail-version-gate` coexists with a usable traffic surface.

Exact projection for the five methods (check IDs are stable contract
identifiers; statuses below are identical on both approved r1
binaries):

| method | check id | what it proves | r1 status (availability/behavior) |
|---|---|---|---|
| sendrawtransaction | `sendrawtransaction.recognized` | method recognition only (invalid sample routed, `server_error_-22`); funded behavior deliberately not exercised | `passed / not_run` |
| getrawmempool | `getrawmempool` | array-of-txids shape, async form `[]` envelope 1.0 | `passed / passed` |
| getrawmempool | `getrawmempool.bool-form` | sync form `[false]` envelope 2.0 (bitcoin-ext) | `passed / passed` |
| getrawtransaction | `getrawtransaction.verbosity0` | hex form `[txid, 0]` envelope 1.0; genesis coinbase -5 (Core-identical) leaves shape unproven without a funded fixture | `passed / unknown` |
| getrawtransaction | `getrawtransaction.verbosity1` | verbose form `[txid, 1]` envelope 1.0 (same -5 caveat) | `passed / unknown` |
| getrawtransaction | `getrawtransaction.bool-verbose` | sync exact form `[txid, true, null]` envelope 2.0 (same -5 caveat) | `passed / unknown` |
| getrawtransaction | `getrawtransaction.bool-verbose.async` | bark call_raw form `[txid, true]` envelope 1.0 (same -5 caveat) | `passed / unknown` |
| getmempoolinfo | `getmempoolinfo` | corepc v30 13-field shape, async form `[]` envelope 1.0; coverage class `traffic` (never gates the bark verdict) | `passed / failed` (`maxdatacarriersize` null) |
| getblock | `getblock.verbosity0` | hex form `[hash, 0]` envelope 1.0 | `passed / passed` |
| getblock | `getblock.verbosity0.rehash` | independent double-SHA256 of the 80-byte header equals the requested hash | `passed / passed` |
| getblock | `getblock.verbosity1` | `GetBlockResult` shape, sync form `[hash, 1]` envelope 2.0 (bdk emitter) | `passed / passed` |
| getblock | `getblock.singleparam` | single-param form `[hash]` envelope 1.0 (verbosity defaults to 1 server-side) | `passed / passed` |

Recognition/shape vs funded-behavior is preserved exactly: the only
funded-behavior row in the subset (`sendrawtransaction.recognized`) is
`not_run` by design, and the `getrawtransaction` shape unknowns are
honest "needs a funded fixture" unknowns, never passes. The
`getmempoolinfo` failure is real traffic-surface information (the
pinned client's v30 deserializer would reject the response).

**Contract hash to freeze (T0 was holding for this):
`11bdb1007e130a749720f87482f287c1fb65d1e743c142fc4b1ca15a782d89e7`.**
Regenerate and diff with `./probe.py contract-show` (its sha256) if
needed; both r1 reports and manifests embed and match it.

## 6. Verification

- 87 unit tests (`python3 -m unittest discover -s tests`): contract
  negatives per field, encoding manifest closure, coverage-class enum,
  traffic non-gating (both directions), verdict three-status rules,
  committed-evidence validation (r1 v2 + frozen v1), credential-leak
  scan, markdown/gap-matrix rendering.
- 15 self-test scenarios (`./probe.py self-test`), incl. the
  load-bearing `version_310000_only` (surface_compatible with a FAILED
  traffic check; never full acceptance) and `traffic_mempoolinfo_ok`
  (traffic pass does not launder an incompatible verdict).
- 12 devcheck golden vectors vs the real pinned crate types
  (`./devcheck/run.sh`; verifies the bark checkout commit `c419159`).
- Full reproduction: `./run_two_nodes.sh` (self-test gate -> both fresh
  owned-node probes -> gap matrix; scratch incl. cookies auto-removed).

## 7. Non-claims

- Funded broadcast/mempool behavior unproven everywhere (`not_run` /
  `unknown`); needs the separate funded-fixture opt-in packet. No
  fabricated funded pass exists anywhere in this packet.
- `getrawtransaction` verbose shapes and the non-null `GetTxOutResult`
  shape are unverified on an unfunded node (genesis coinbase -5 is
  Core-identical).
- The `getmempoolinfo` shape failure is a divergence from the pinned
  client's v30 deserializer, observed on both binaries; whether the
  traffic layer can live with it (or needs the node to emit an integer)
  is the traffic owner's decision — this packet only reports it.
- ZMQ absent on both nodes; not a gap for the pinned bark (it polls).
- No Core version was faked, shimmed, proxied, or overridden.

## 8. Decision requested

None blocking. The `bark-v31` named-profile decision proceeds
separately per the parent plan; the inputs are unchanged from the v1
review (version number is the only failing bark gate on both binaries),
plus the new traffic-surface fact in §3/§5.
