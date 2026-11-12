# Fable review - AP-1a ark-rpc read-only node capability probe (2026-11-08)

**Decision: REQUEST_CHANGES (bounded).** The headline evidence is genuine and
reproduced: both approved rbitcoin binaries report `getnetworkinfo.version =
190000`, bark `c419159` requires `>= 310000` (server) and `>= 290000`
(wallet), and the probe fails that gate honestly without any version
override, shim, or proxy. That part is an **expected policy failure, not a
probe defect**, and nothing below changes it.

What does not hold up is the completeness claim built on top of it ("the
version number is the only failing gate; every other availability/shape
check on the bark-c419159 surface passes", report section 5 item 1), the
freezing of the contract JSON (`48894548...`) as the absorption interface,
and one factual finding in the report (section 9 item 2) that is wrong on
both counts. Those need to be fixed before the report is used as a
decision input for a `bark-v31` profile.

This review does **not** approve a Core compatibility claim, a new RPC
version number, a node patch, bark acceptance, or anything in AP-1b / AP-2.

---

## 1. Scope and provenance checks

| item | result |
|---|---|
| Candidate | `/home/ethan/.local/share/opencode/worktree/ad8886/lab-ark-rpc-probe`, base `b0ab4ca34fda07c61a6335d05f598eaacc69678c`, 38 untracked files, no commits |
| Review manifest `/tmp/opencode/lab-ark-rpc-probe-review-manifest.json` | all 38 `(path, sha256, mode)` entries match the tree; untracked set == manifest set; recipe `{"base", "files": sorted by path}` as compact JSON with **sorted keys** reproduces snapshot `810236fb...d363c18` (unsorted-key compact JSON does not; the parent's "sorted" applies to keys as well as the file list) |
| Per-run manifests `reports/*/manifest.json` | each freezes 28 files = the 38 minus the 9 `reports/**` artifacts minus the one file outside `testing/ark-rpc/` (`docs/reviews/...-implementation.md`); every one of the 28 hashes equals the review-manifest hash for the same path; the two run manifests carry identical file maps. No conflict, just different scope |
| Node binaries | `sha256sum` of `/tmp/opencode/rbitcoin-lab-retarget-profile/target/debug/rbitcoin-node` = `eb257af7...` and `/tmp/opencode/rbitcoin-v0.9-lab/target/dev/debug/rbitcoin-node` = `bcb6d5ee...`, matching `node.binary_sha256` in both reports and manifests |
| Candidate tests | `python3 -m unittest discover -s tests`: 49 tests OK; `./probe.py self-test`: 10 scenarios, 0 failed; `contract-show` sha256 = `48894548fc32...a3a4b7` as reported |
| Committed evidence | both `report.json` validate against `report.schema.json`; verdict `incompatible` with reasons `[fail-version-gate, incomplete]`; 28 check rows each; the two nodes differ only in `subversion` (`/rbitcoin:0.8.99/` vs `/rbitcoin:0.9.0/`); `raw_responses.json` carries the real `-5`, `-22`, `-32601` errors and the genesis block hex |
| Credential hygiene | no `ap1a:<hex>` or `Authorization` material in any committed artifact; cookie contents never recorded |
| Pinned sources used for verification | `/tmp/opencode/bark-c419159` (HEAD `c419159fa540...`), its `Cargo.lock` resolving `bitcoind-async-client 0.15.0`, `corepc-types 0.13.0`, `bitcoincore-rpc(-json) 0.19.0`, `bdk_bitcoind_rpc 0.22.0`, `bitcoin-units 0.1.3`, `jsonrpc 0.18.0`; crate sources read from `~/.cargo/registry/src/*/` |

I made no edits to the candidate and spawned no subagents. The only file
written is this one.

## 2. Bounded own-node probe (why and what)

The committed `raw_responses.json` records only `result` / `error` per
exchange, never the HTTP status, the echoed `id`, or the `jsonrpc` field.
That leaves the bark async client's hard requirement unverifiable from the
evidence: `bitcoind-async-client-0.15.0 src/client/mod.rs:129-133` declares
`struct Response<R> { result: Option<R>, error: Option<BitcoinRpcError>, id:
u64 }`, and the non-200 error path (`mod.rs:214-219`) only yields
`ClientError::Server(code, msg)` (the thing `is_not_found()` matches) if the
error body parses as that struct. A node that echoed ids as strings or
dropped them on error would pass every committed check and break every
bark RPC call.

So I started one fresh node per binary using the candidate's own
`OwnedNode` (same isolation: fresh datadir, loopback free port, cookie
0600, `--no-listen --no-seeds --no-discover`, `--max-run-secs 90`, SIGTERM
reaped) and issued read-only raw HTTP calls plus the same invalid `"00"`
sample. No mining, no funding, no other process touched (the unrelated
`rbitcoin-node` instances on 39001 and 48332 were left alone). Run tokens
`AP1a-5f18fe2c5ad8-fable` (l2) and `AP1a-05f78a267071-fable` (v09); both
exited 0; script and results at `/tmp/opencode/fable-ap1a-rawprobe.py`
and `/tmp/opencode/fable-ap1a-rawprobe.json`. Both binaries behaved
identically:

| raw check | l2 and v09 |
|---|---|
| `jsonrpc:"1.0", id:7` (async-client style) | 200, `id` echoed as integer `7`, keys `error,id,result` |
| `jsonrpc:"1.0"` error (`getrawtransaction` genesis coinbase) | **HTTP 500**, body `{"result":null,"error":{"code":-5,...},"id":8}`, `Content-Length` set, not chunked -> parses as `Response<Value>` with `id: u64`, so bark's `is_not_found()` mapping to `Ok(None)` does hold |
| unknown method | HTTP 404 with `-32601` body and integer id |
| `sendrawtransaction ["00"]` | HTTP 500, `-22`, integer id |
| `jsonrpc:"2.0", id:11` (sync `jsonrpc 0.18` style) | 200, `jsonrpc:"2.0"` echoed, integer id; on error **HTTP 200 with no `result` key** (JSON-RPC 2.0 form, same as Core 28+) - compatible with `jsonrpc::Response` (`result` is `Option`) and with the nonce check at `jsonrpc-0.18.0 src/client.rs:127` |
| string id `"ap1a-probe"` (what the candidate sends) | echoed as string; absent id -> no `id` key |
| `getblock <genesis> 1` | object with `nTx`, `weight`, `size`, `strippedsize`, `tx[]`, `versionHex`, `merkleroot`, `chainwork`, ... - would parse as `bitcoincore-rpc-json 0.19 GetBlockResult` |
| `getrawtransaction <txid> false` and `[txid, true, null]` (the sync client's and server's actual encodings) | accepted, `-5` as with integer verbosity |
| `estimatesmartfee 1 "ECONOMICAL"` (what bark actually sends) | accepted, same `blocks + errors` result as lowercase |
| `getblockheader <genesis> true/false` | verbose object with `nTx` / 160-char header hex |

These results **substantiate** several of the candidate's claims that its own
evidence could not (id echo, error-path parseability, `-5` mapping), and
they show the two coverage gaps in finding F2 would pass if added. They are
my observations, not the candidate's recorded evidence, and should not be
cited as such.

## 3. What I verified against the real serde contracts

Read directly from the pinned sources, not from the candidate's tests:

- `server/src/bitcoind.rs:31` `MIN_BITCOIND_VERSION = 31_00_00`; `bark/src/chain.rs:51` `= 290000`. Gates as stated.
- `server/src/bitcoind.rs:104-124` `tx_status` reads `rpc::json::GetBlockHeaderResult` via `call_raw("getblockheader", [hash, true])`. `bitcoincore-rpc-json-0.19.0 src/lib.rs:221-245`: `#[serde(rename_all = "camelCase")]`, `pub n_tx: usize` -> key **`nTx`**, required. The `nTx` requirement and the camelCase trap are real. `version_hex` has `default`, `median_time`/prev/next are `Option`. Contract matches.
- `bitcoind-async-client-0.15.0 src/client/v30.rs:182-188` `network()` -> `corepc_types::v30::GetBlockchainInfo` which `v30/mod.rs:364` re-exports from `v29/blockchain/mod.rs:279-324`: `bits`, `target`, `time` are required `String`/`i64`; `warnings: Vec<String>`. Default feature is `30_2`; bark adds `raw_rpc`. Contract matches.
- `bitcoincore-rpc-json 0.19.0 src/lib.rs:1017-1058` `GetBlockchainInfoResult`: no `bits`/`target`/`time`; `warnings: StringOrStringArray`; `softforks` `#[serde(default)]`. Contract matches except the null nuance in F4(d).
- `bitcoin-ext/src/rpc/mod.rs:474-498` `require_txindex` / `try_get_tx_out` use `json::GetIndexInfoResult` (`src/lib.rs:2141-2153`: `IndexStatus { synced: bool, best_block_height: u32 }`) and `json::GetTxOutResult` (`src/lib.rs:740-748`). Null-result mapping relies on `ClientError::Other("Empty data received")` (`mod.rs:231-234`) - confirmed.
- `bitcoin-units-0.1.3 src/amount.rs:1729-1734` `des_btc` = `f64::deserialize` then `Amount::from_btc`; `:961-968` `from_float_in` rejects negative and re-parses `value.to_string()` (so >8 decimals fail). Upper bound is `Amount::MAX = u64::MAX` sats (`:875`), **not** 21M BTC - see F4(c).
- `bitcoin-ext/src/rpc/mod.rs:66-78` `serde_hex` = `Vec::<u8>::from_hex(&s)`; the empty string parses to an empty vector - see F4(a).
- Core v29.0 `src/rpc/node.cpp:345-355` `SummaryToJSON` emits `synced` and **`best_block_height`**; `corepc-types-0.13.0 src/v21/util.rs:21-26` also `best_block_height: u32` - see F1.
- `bdk_bitcoind_rpc-0.22.0 src/lib.rs:134-151, 203, 287-327` (Emitter used by `bark/src/chain.rs:581-628`): `get_block_count`, `get_block_hash`, `get_raw_mempool`, `get_raw_transaction(txid, None)` (`bitcoincore-rpc-0.19.0 src/client.rs:488-495` sends `[txid, false]`), `get_block` (verbosity 0) and **`get_block_info`** (`client.rs:344-346`, `getblock [hash, 1]` -> `GetBlockResult`, `src/lib.rs:195-219`) - see F2.
- `bark/src/chain.rs:337-349` `fetch_fee_rates` serialises `rpc::json::EstimateMode::Economical`, which is `#[serde(rename_all = "UPPERCASE")]` (`src/lib.rs:1828-1834`) -> `"ECONOMICAL"`; `:816-823` falls back only when `fallback_fee` is configured, otherwise the error propagates - the report's "bark falls back" is true only when configured.

## 4. Findings, ranked

Severity: **H** blocks using the artefact for its stated purpose; **M**
should be fixed in this packet; **L** cosmetic or fail-closed over-strictness.

### F1 (H, report accuracy) - section 9 item 2 is wrong on both counts

`docs/reviews/2026-11-07-ark-rpc-probe-implementation.md:87` states "Core
itself uses `best_block_hash`" and that the contract "would break against a
real bitcoind parsed by a newer corepc". Core v29.0 `src/rpc/node.cpp:352`
emits `best_block_height`; `corepc-types 0.13.0 src/v21/util.rs:25` reads
`best_block_height`. The probe's contract (`contracts.py:398-403`) is
correct; the finding is an unsupported baseline claim that would mislead
the `bark-v31` profile work. Delete it or replace with the verified
statement.

### F2 (H, surface completeness) - the derived "required surface" misses a wallet-path RPC, so section 5 item 1 overreaches

`contracts.py` has no contract for `getblock <hash> 1` (`GetBlockResult`),
yet `bdk_bitcoind_rpc 0.22.0` calls `get_block_info` at `src/lib.rs:300`
and `:309` on every `poll_once`, inside the Emitter loop bark runs at
`bark/src/chain.rs:581-628`. Likewise no probe exercises the sync client's
parameter encodings (`getrawtransaction [txid, false]`, `jsonrpc: "2.0"`
with a numeric id). The probe therefore cannot support "every other
availability/shape check on the bark-c419159 surface passes" (report
`:57`, `:40`). Both binaries do in fact serve `getblock 1` with a
`GetBlockResult`-compatible shape (section 2), so this is a **coverage
gap, not a node failure** - but the claim must be downgraded until the
probe checks it. Add `getblock.verbosity1` (genesis, read-only; requires
`hash, confirmations, size, weight, height, version, merkleroot, tx[],
time, nonce, bits, difficulty, chainwork, nTx`; optional `strippedsize,
versionHex, mediantime, previousblockhash, nextblockhash`) and a
bool-verbosity variant of `getrawtransaction` expecting `-5`.

### F3 (M, availability axis) - id echo and HTTP status are neither checked nor recorded

`client.py:119-124` sends `"id": "ap1a-probe"` (a string) and `:180-194`
never reads `payload["id"]`, the HTTP status on success, or `jsonrpc`.
`reporting.py:86-106` writes only `result`/`error`. Given
`bitcoind-async-client 0.15.0 src/client/mod.rs:129-133` (`id: u64`,
required) and `jsonrpc 0.18.0 src/client.rs:52-59,127-129` (numeric nonce,
`NonceMismatch`), a node that echoes a non-integer id or omits it on error
would pass this probe and fail bark. Fix: send an integer id, record
`http_status`, `id`, `jsonrpc` per exchange, add an `id-echo` check (integer
id echoed as integer on both 200 and error paths). Self-test
counterexample: patch the fake server to echo `str(rid)` and expect a
failure; today it would pass.

Related latent bug: `client.py:180` labels any response lacking a
`result` key `bad_jsonrpc`. rbitcoin's (and Core 28+'s) JSON-RPC 2.0 error
form has no `result` key (section 2, B2), so the client cannot be reused
for the sync-client dialect as written.

### F4 (M, contract fidelity) - the frozen contract JSON disagrees with the pinned serde types in five places

The absorption hash `48894548...` is advertised (report `:60`, README) as
the interface the Rust `node_probe` must consume. Freezing it is premature;
each of these changes the hash:

- (a) **`hex` rejects the empty string** (`contracts.py:167-170`), but
  `bitcoin-ext serde_hex` (`rpc/mod.rs:75-78`) and `bitcoincore-rpc-json
  serde_hex` accept it. Every native-segwit input Core renders has
  `"scriptSig": {"asm": "", "hex": ""}`, and empty witness items are legal.
  Reproduced: a realistic segwit verbose tx is rejected at
  `vin[0].scriptSig.hex`. On the funded-fixture packet this contract would
  report `fail-shape` for a tx bark parses fine. Fail-closed, but a wrong
  contract. Fix: allow `""` for `hex` (keep `hash32` strict); add a
  segwit verbose fixture to `tests/test_contracts.py`.
- (b) **`gettxout.scriptPubKey` reuses `_SPK`** (`contracts.py:243-263`,
  `:587`), whose `type` enum includes `anchor` from bitcoin-ext's
  `ScriptPubkeyType` (`rpc/mod.rs:148-162`). bark's `try_get_tx_out`
  deserialises `bitcoincore-rpc-json 0.19 GetTxOutResult`, whose
  `ScriptPubkeyType` (`src/lib.rs:904-918`) has **no `Anchor`**. Reproduced:
  the probe accepts `"type": "anchor"` for `gettxout`; bark would error.
  Fail-open. Fix: separate SPK enums per consuming crate.
- (c) **Amount upper bound** (`contracts.py:96,199-200`): 21M BTC, while
  `bitcoin-units 0.1.3` only rejects values exceeding `u64` sats
  (`amount.rs:875`). Fail-closed; `tests/test_contracts.py::
  AmountBtcTests::test_over_supply_rejected` pins the wrong semantics - this
  is exactly the "self-test mirroring an incorrect Python contract"
  pattern. Also `Decimal("0.100000000")` (9 places, trailing zero) is
  rejected while `f64 -> to_string` yields `"0.1"` and parses.
- (d) **`#[serde(default)]` non-`Option` fields accept null in the probe**
  (`contracts.py:128-136` treats every `optional` as absent-or-null):
  `softforks` (`GetBlockchainInfoResult`), `_SPK.addresses`,
  `vin[].txinwitness`. serde `default` covers only the absent case; an
  explicit `null` fails in Rust. Fail-open, low likelihood.
- (e) **`fee_estimate_mode: "economical"`** (`contracts.py:63`) while bark
  sends `"ECONOMICAL"` (`EstimateMode` is `rename_all = "UPPERCASE"`,
  `src/lib.rs:1829`). Both binaries accept both (section 2), but the
  profile should encode what bark actually sends.
- (f) **`getnetworkinfo` observation set gates the verdict**
  (`contracts.py:271-307`, `probe.py:174-175`): bark deserialises only
  `version`, but `relayfee`, `incrementalfee`, `networks[].*` are required
  fields of the contract, and a miss becomes `fail-shape` with the summary
  "response shapes do not parse against the bark-consuming crate types" -
  a false statement for that case. Split gate contract (`version` only)
  from observations (recorded, never a failure).

### F5 (M, absorption contract) - the check-id list and verdict rules are not part of the frozen interface

`canonical_contract_json()` (`contracts.py:763-770`) carries profiles and
shape specs only. The 28 check ids and their `fail_kind`s live in
`probe.py` control flow; the verdict rules live in `compute_verdict`
(`probe.py:691-715`) and prose. `report.schema.json` does not enumerate
required check ids and does not tie `verdict.status` to `checks`, so a
truncated or hand-edited report validates (`cmd_compare` only
schema-validates). The parent's requirement of "one canonical list / status
contract, no duplicated different verdict" is not met for a Rust consumer.
Fix: include `checks: [{id, method, fail_kind}]` and the verdict rule table
in the canonical JSON; add a `verify` step that recomputes the verdict from
`checks` and asserts the id set is complete; make
`tests/test_reporting.py::test_verdicts_are_honest` recompute rather than
string-match.

Also: `getmempoolentry.shape.*` are hard-coded `unknown` (`probe.py:577-584`),
so `full_compat` is unreachable on **any** node with this code, not only
on an unfunded one as the docstring (`probe.py:28-30`) and README say. State
that plainly, or make those rows depend on a mempool entry when one exists.

### F6 (L, evidence accuracy) - report.json argv and node record are mangled by redaction; pin label wrong

`reporting.py:59-64` passes `node_info` through `_redact_value`, and
`client.py:50-65 redact()` rewrites anything after the substring `cookie`
on a line. Result in both committed `report.json`: argv entries
`"--rpc-cookie-file:<redacted>"` and
`".../rpc.cookie:<redacted>"`, and `cookie_file: "rpc.cookie:<redacted>"`.
The `manifest.json` argv is correct, so the two artefacts disagree about
the actual command line. The cookie *path* is not a credential; redact
only `Authorization` values and the cookie *contents*. Separately,
`cli.py:165-171` hard-codes `name: rbitcoin-lab-retarget-profile, role:
probed node binary (approved L2)` into `upstream_source_pins`, so the v0.9
manifest mislabels its own binary.

### F7 (L, report framing) - section 9 item 1 presents Core-identical behaviour as an rbitcoin finding

The `-5` message in `raw_responses.json` ("The genesis block coinbase is
not considered an ordinary transaction and cannot be retrieved") is Core's
verbatim string; Core behaves the same with `txindex=1`. Reframe as
Core-identical; the conclusion (verbose shape stays `unknown` without a
funded fixture) stands.

### F8 (L, recognition heuristic) - any JSON-RPC error counts as "method routed"

`probe.py:557-558, 652-661` treat every `server_error_*` category as proof
of routing. Protocol-level codes (`-32700`, `-32600`, `-32602`) are not
evidence that the method handler ran. Exclude them (`unknown`). Both
binaries returned `-22` here, so the committed evidence is unaffected.

### F9 (L, hygiene) - cookies persist at rest after the node is dead

`nodeproc.py:148-170 stop()` leaves `rpc.cookie` (0600) in the retained
scratch directory. Harmless for a dead per-run node; deleting it in
`stop()` is cheap.

## 5. What is confirmed sound

- Version gate failure is honest and expected policy: both binaries really
  report `190000`; no override, shim, or proxy exists anywhere in the 38
  files (grepped for version rewriting; none).
- Mutating methods are exercised only with `"00"` and classified
  `not_run` on the behavior axis; the report and README say so clearly.
- Two-axis status model and the ordered reasons are implemented as
  documented; `version_310000_only` really yields `incomplete`.
- Genesis walk is independently verified (80-byte header re-hash equals
  the requested hash; `nTx == 1`, height 0). Wrong-network and wrong-genesis
  fail closed in self-test, and `_resolve_profile` (`cli.py:47-65`) rejects
  CLI overrides that disagree with the named profile.
- Owned-node lifecycle: fresh datadir, free loopback port, launcher-written
  cookie never in argv, `--max-run-secs` backstop, terminate/kill/wait
  reap, exit recorded. My own two runs left no process behind.
- Per-call deadline (10s default), 1 MiB body cap, error categories as
  listed; auth/transport/timeout/parse failures never count as recognition
  (`auth_fail` scenario confirms).
- `getblockheader` `nTx` requirement, `getblockchaininfo` async vs sync
  split, `getindexinfo` shape, `getmempoolentry.server` chunk shape,
  `SubmitPackageResult` keys, `TestMempoolAcceptResult`, `EstimateSmartFeeResult`
  (`feerate` optional) all match the pinned Rust types.

## 6. Required before re-review

1. F1: correct or delete report section 9 item 2.
2. F2: add `getblock.verbosity1` (`GetBlockResult`) and the bool-verbosity
   `getrawtransaction` check; re-run both nodes; downgrade the "only failing
   gate / everything else passes" wording to "every check the probe
   currently covers".
3. F3: integer request id; record `http_status`, `id`, `jsonrpc` per
   exchange; add the `id-echo` check and its negative self-test.
4. F4 (a), (b), (e), (f) at minimum; (c), (d) if the contract JSON is to be
   called "serde-strict". Re-freeze and re-publish the contract hash.
5. F5: put check ids and verdict rules into the canonical JSON; add the
   recompute/complete-set verification.
6. F6: stop redacting the cookie path; fix the v0.9 pin label.

F7-F9 are recommended, not blocking.

## 7. Explicit non-approvals

- No Core RPC-version compatibility claim is approved; `190000` remains
  the only observed value and the gate remains failed.
- No named `--rpc-compat-profile bark-v31`, node patch, or version override
  is approved or implied.
- Contract hash `48894548...` is **not** approved as the AP-1b absorption
  interface (F4, F5).
- Nothing here approves AP-1b or AP-2 wallet contracts, or bark acceptance.
- My section 2 observations are review notes, not committed evidence; the
  candidate must reproduce them through its own probe and record them.

---
---

# Fable re-review r1 - AP-1a probe after the F1-F9 rework + T0 `getmempoolinfo` (2026-11-08, later)

**Decision: REQUEST_CHANGES (small, bounded).** The r1 rework resolves F1,
F2 (coverage), F4, F5, F6, F7, F8 and F9 substantively, the new evidence
is genuine and reproduces, and the three-status verdict cannot launder a
funded pass. Two things stop approval of the candidate hash
`11bdb1007e130a749720f87482f287c1fb65d1e743c142fc4b1ca15a782d89e7`:
one real code gap in the F3 negative path (error-path id echo is not
enforced; counterexample below), and encoding statements baked into the
frozen contract JSON that are wrong against the pinned client code. Both
are small; the rest are documentation corrections.

Nothing here reopens the headline: both approved binaries report
`190000`, the gate fails honestly, no override exists, funded behavior is
permanently `unproven`, `full_acceptance` is permanently `false`.

## R1. Scope and provenance

| item | result |
|---|---|
| r1 review manifest `/tmp/opencode/lab-ark-rpc-probe-r1-review-manifest.json` | 59 `(path, sha256, mode)` entries all match the tree; untracked set == manifest set (caches excluded); sorted-key compact JSON of `{base, files}` reproduces snapshot `620e42e32d744b8ac5b0ca18d20fc0a9e3532d57ac6fcf7bdb3ffb6da26108dd` |
| Candidate state | base `b0ab4ca`, still uncommitted, untouched by me |
| Run manifests `reports/2026-11-08-r1-*/manifest.json` | both freeze 33 runtime files (59 minus 3 docs, 17 report artifacts, 6 `devcheck/`); every frozen hash equals the review-manifest hash for the same path; the two file maps are identical; `contract_hash` in both reports and both manifests == `11bdb100...` == `./probe.py contract-show` |
| Actual evidence run token | **`AP1a-b1aeb688e017`** in both r1 reports and manifests (generated 07:37:11Z/12Z). The r1 doc section 4 says `AP1a-89410a8483f1` - stale, see D2 |
| Node binaries | `eb257af7...` and `bcb6d5ee...e173b4b9...` re-hashed on disk; match reports and manifests |
| v1 evidence | `reports/l2-eb257af7`, `reports/v09lab-bcb6d5ee` byte-identical to the v1 review manifest (untouched); v1 implementation doc carries the dated F1 correction (now at line 95) and a superseded banner - permitted historical clarification |
| Parent one-pager `docs/reviews/2026-11-08-ark-rpc-probe-parent-manifest.md` | the five artefact hashes it lists match the tree; its v0.9 binary sha256 has a typo (`...e173f4b99a...` for `...e173b4b99a...`), see D3 |
| Tests reproduced | `python3 -m unittest discover -s tests`: 87 OK; `./probe.py self-test`: 15/15; `devcheck/run.sh` (offline, cached build): 12/12 PASS; `devcheck/Cargo.lock` resolves `bitcoincore-rpc-json 0.19.0`, `corepc-types 0.13.0`, `bitcoin 0.32.9`, `bitcoin-units 0.1.3`, `serde_json 1.0.149`, `bark-bitcoin-ext 0.7.1` (path, checkout at `c419159`) |
| Own-node probing this round | none needed: the r1 `raw_responses.json` now carries per-exchange HTTP status / request id / response id / envelope receipts, which answer the questions my first-round raw probe had to ask. My two first-round scratch dirs (`AP1a-*-fable`, containing dead cookies) were removed. Five author-era `AP1a-*` scratch dirs from v1 runs remain under `/tmp/opencode` with cookies at rest; not mine, flagged in D6 |

## R2. Cross-check of the F1-F9 dispositions (not accepted as stated; verified)

| finding | verified status |
|---|---|
| F1 | Resolved. v1 doc line 95 strikes the false `best_block_hash` claim with a dated correction; `contracts.py:507-552` cites `corepc-types 0.13.0 src/v21/util.rs:18-26` and Core's own field name. Correct. |
| F2 | Coverage resolved: `getblock.verbosity1` (`GetBlockResult`, envelope 2.0, `[hash, 1]`) and `getblock.singleparam` added and pass on both nodes; bool-verbosity `getrawtransaction` forms added. **Fidelity partly wrong**, see C2. |
| F3 | Numeric ids, both envelopes, receipts recorded, `protocol.id-echo` row, `id_string_echo` scenario - all present. **Negative path incomplete**, see C1. |
| F4 | Resolved against the real crates: `hex` `allow_empty` (`contracts.py:216-228`), `_SPK_CORE` vs `_SPK_BARK` enum split (`:307-362`), `_MAX_SATS = 2**64-1` (`:134`), `"null": "reject"` for `serde(default)` + custom deserializers (`:174-183`), `"ECONOMICAL"` (`:95`), `getnetworkinfo.observe` never fails (`probe.py:243-258`). devcheck 12/12 against `bitcoincore-rpc-json 0.19.0`, `corepc-types 0.13.0`, `bitcoin-units 0.1.3`, bark `bitcoin-ext`; I re-ran it. |
| F5 | Resolved. `canonical_contract_json()` carries `contracts + encodings + checks + verdict_rules` (`contracts.py:1424-1449`); `schemadef.validate_report_consistency` (`:277-381`) rejects undeclared / duplicate / missing check ids and re-derives the three-status verdict, `funded_state`, `full_acceptance`, reason order and `unproven_checks`; `validate_report` dispatches v1/v2. `full_compat` is gone. |
| F6 | Resolved. `redact()` is value-exact plus keyed patterns (`client.py:90-128`); both r1 `report.json` carry the intact argv and `cookie_file: "rpc.cookie"`. |
| F7 | Resolved. `source_name` (`rbitcoin-lab-retarget-profile`, `rbitcoin-v0.9-lab`) in reports and `upstream_source_pins`; v0.9 no longer labelled "approved L2". |
| F8 | Resolved. `-32700/-32600`, id mismatch, auth/transport are `protocol_level` -> availability `unknown` -> verdict `unproven` (`probe.py:153-179`, scenario `protocol_error_parse`); genesis `-5` / `gettxout` null wording is Core-identical. |
| F9 | Resolved on the success path: `OwnedNode.remove_scratch()` (`nodeproc.py:220-228`) runs after the report is written; the r1 run left no scratch. **Not on the failure path**, see D6. |

## R3. T0 `getmempoolinfo` traffic check

Verified against sources, not the summary:

- `corepc-types-0.13.0 src/v30/blockchain/mod.rs:20-59`: `GetMempoolInfo`
  has 13 required fields, `max_data_carrier_size: u64`, no `Option`, no
  default. A JSON `null` is a hard serde error. devcheck vector
  `getmempoolinfo_shapes` proves it against the real crate (PASS).
- `bitcoind-async-client-0.15.0 src/client/v30.rs:128-131`
  `get_mempool_info` calls `getmempoolinfo []` and deserialises that type;
  `client/mod.rs:21-30` makes v29/v30 mutually exclusive; bark pins
  default features (`30_2`) + `raw_rpc`.
- Both r1 `raw_responses.json`: `getmempoolinfo []` returns 18 keys with
  `"maxdatacarriersize": null` on both binaries. The row is
  `passed / failed`, `fail_kind: shape`, and the verdict reasons are
  unchanged (`fail-version-gate`, `funded-unproven`).
- `compute_verdict` (`probe.py:1095-1107`) and
  `validate_report_consistency` (`schemadef.py:318-327`) skip coverage
  class `traffic` for fail-kind accumulation and for the
  `surface_compatible` requirement, in both directions; scenarios
  `version_310000_only` and `traffic_mempoolinfo_ok` pin it. No
  null-to-zero coercion exists (`int` kind rejects null). No node change
  is in the packet.

Narrowness: the r1 doc section 7 states it correctly ("a divergence from
the pinned client's v30 deserializer ... the traffic owner's decision").
Section 3's "Grounding" is less careful: "The consumer is the pinned
async client's Reader trait" reads as if that were T0's actual consumer.
Nothing in bark `c419159` calls `getmempoolinfo` (confirmed by grep), and
T0's `RpcNodeControl` typed route is not bitcoind-async-client. The
contract `consumer` string (`contracts.py:864-868`) and section 3 must
say "selected modelling consumer (corepc v30 via bitcoind-async-client
Reader::get_mempool_info); whether T0's `RpcNodeControl` requires the
full v30 superset, and `maxdatacarriersize` as an integer, is for the T0
owner to state explicitly". The row itself stays as is: it is a correct
result for that selected consumer, not a universal traffic failure and
not a new bark requirement. See D5.

Traffic projection visibility: protocol-level failures do not vanish
from the traffic rows. A protocol failure on the `getmempoolinfo`
exchange yields availability `unknown` on the row (`_failure_axes`), and
an id mismatch on that exchange is still counted into the gating
`protocol.id-echo` row (`ProbeSession.call`). What the projection lacks
is an explicit aggregation rule for the worker; see D5.

## R4. Findings, ranked

### C1 (code, blocking) - error-path id echo is not enforced; the F3 negative covers HTTP 200 only

`client.py:272-305` (the `HTTPError` branch): when a non-200 body carries
a JSON-RPC error object, the client returns `server_error_<code>` /
`method_missing` and only *records* `receipt.id_match`; the `id_mismatch`
category is produced solely on the 200 path (`client.py:349-357`).
`ProbeSession.call` (`probe.py:124-134`) then increments
`id_mismatch_count` only when `error.category == "id_mismatch"`, so an
error-path receipt with `id_match: False` is dropped from
`protocol.id-echo`. The self-test fake server always answers HTTP 200
(`selftest.py:83-88 _respond(200, ...)`), so `id_string_echo` never
exercises the real nodes' 500/404 error form.

Why it matters: `bitcoind-async-client-0.15.0 src/client/mod.rs:214-219`
parses the non-200 body as `Response<Value>` (`id: u64` required,
`:129-133`); a string id there makes the parse fail and the call becomes
`ClientError::Status`, so `is_not_found()` (`server/src/bitcoind.rs:55-57`)
never matches and bark's `-5 -> Ok(None)` mapping breaks. Both real
binaries echo integer ids on 500/404 (all 30 r1 receipts have
`id_match: true`), so the committed evidence stands - but the check
cannot catch the failure it exists for.

Counterexample (run by me against the candidate, fake server returning
HTTP 500/404 error bodies with `"id": "<string>"`, 200 successes with
integer ids): verdict `incompatible [fail-version-gate, funded-unproven]`;
`protocol.id-echo` = `passed / passed` "all 21 exchanges echoed the
numeric request id exactly" (30 exchanges, 9 error-path receipts with
`id_match: False` uncounted); `getrawtransaction.verbosity1` and
`getmempoolentry.recognized` = `passed / unknown` claiming "bark maps -5
to None"; `sendrawtransaction.recognized` = `passed / not_run` "method
routed".

Fix (small): in the `HTTPError` branch, when the parsed body is a dict
and `receipt["id_match"]` is false, return `id_mismatch` exactly as the
200 path does (and let `_failure_axes` make it protocol-level); or count
`receipt["id_match"] is False` in `ProbeSession.call` regardless of
category. Add a fake-server mode that serves error bodies with HTTP
500/404 (the real rbitcoin/Core 1.0 form) and a scenario
`id_string_echo_errorpath` expecting `protocol.id-echo` failed and the
three `*.recognized` rows at availability `unknown`. Related nit:
`_check_id_echo` runs before `_observe_zmq` (`probe.py:1205-1206`), so
its "all N exchanges" count excludes the last exchange (29 of 30 in r1).

### C2 (contract fidelity inside the frozen hash, blocking for the hash) - three encoding statements contradict the pinned client code

- `contracts.py:1115-1120` `getrawtransaction.bool-null.sync` claims
  bitcoincore-rpc / bitcoin-ext send `[txid, true, null]` because
  "handle_defaults keeps the trailing null". `bitcoincore-rpc-0.19.0
  src/client.rs:116-147 handle_defaults` returns `&args[..required_num]`
  when every optional is null, i.e. the trailing null is **stripped**:
  `get_raw_transaction_info(txid, None)` (`client.rs:507-513`) and
  bitcoin-ext `get_raw_transaction_info` (`rpc/mod.rs:327-328`) send
  `[txid, true]`; `get_raw_transaction(txid, None)` (`client.rs:488-495`)
  sends `[txid, false]`. The probe's `getrawtransaction.bool-verbose`
  check therefore exercises a form no pinned client sends (the node
  accepts it, so no false result, but the r1 doc section 5 row "sync
  exact form `[txid, true, null]`" is wrong). The actual sync form
  `[txid, true]` envelope 2.0 is not exercised; `[txid, true]` envelope
  1.0 is (`bool-verbose.async`).
- `contracts.py:1151-1155` `estimatesmartfee.sync` "mode None ->
  trailing null kept": same `handle_defaults` (`client.rs:1083-1090`)
  strips it; the sync form is `[target]`.
- `contracts.py:1156-1160` and `probe.py:982-984`: `testmempoolaccept`
  and `submitpackage` are sent only as envelope 2.0 "sync 0.19" /
  "bitcoin-ext broadcast_package". Every active bark caller is async
  envelope 1.0: `server/src/bitcoind.rs:207-221` (`call_raw`),
  `bark/src/chain.rs:685` (`rpc.call_raw("submitpackage", ...)`),
  callers at `server/src/offboards.rs:485`, `bark/src/exit/
  transaction_manager.rs:470,497,630`, `bark/src/actions/board.rs:490`.
  The sync `BitcoinRpcExt::broadcast_package` exists but has no caller
  in `c419159`.

Fix: correct the three `ENCODINGS` entries, send `getrawtransaction
[txid, true]` under 2.0 for the sync check (drop the `null` form or
label it "node tolerance, not a pinned form"), add 1.0 forms for
`testmempoolaccept`/`submitpackage` (the bark forms), re-freeze. Since
the hash is explicitly unapproved, this is the moment to do it.

### D1 (doc, required) - r1 section 0 is false

`docs/reviews/2026-11-08-ark-rpc-probe-r1.md:21-28` and the parent
one-pager's "Provenance disclosure" state the Fable review "does not
exist anywhere in this worktree or the repo". It exists at the primary
path given in the task, `/home/ethan/code/silent-pool/docs/reviews/
2026-11-08-ark-rpc-probe-fable.md` (this file); the author searched only
the candidate worktree. Replace the paragraph with the absolute path and
a statement that dispositions were cross-checked against it.

### D2 (doc, required) - r1 section 4 run token is stale

`r1.md:122` says `AP1a-89410a8483f1`; both r1 reports and manifests say
`AP1a-b1aeb688e017` (the parent one-pager has it right). Correct the doc
so the token binds to the committed artefacts.

### D3 (doc, required) - parent one-pager binary hash typo

`2026-11-08-ark-rpc-probe-parent-manifest.md:34` prints
`bcb6d5eefe0dd0bd96e2ac79e173f4b99a91f67f7717dae6620c30786378a171`; the
binary, both r1 manifests and both reports have `...e173b4b99a...`. One
character, but it is the identity of the probed node.

### D4 (classification, small) - `getbestblockhash` is `required` but bark never calls it

`contracts.py:567-575` and `CHECKS` `:1239-1241` mark it `required`
("async get_current_timestamp"). No code under `server/`, `bark/`,
`bitcoin-ext/` in `c419159` calls `getbestblockhash` or
`get_current_timestamp` (grep). It passes on both nodes, so the evidence
is unaffected, but a `required` failure here would wrongly make a node
"incompatible with the pinned bark". Reclassify as `observational` (or
an explicitly named non-bark class) with the consumer stated as the
async client API, not bark. Same precision applies to several
`sources` lines citing `bitcoind-async-client ... src/client/v29.rs`
while bark compiles `v30.rs` (same line numbers; cite the compiled
module).

### D5 (doc, required for T0 handoff) - state the selected consumer and the aggregation rule

Per R3: (a) `contracts.py:864-868` and `r1.md` section 3 must label the
`getmempoolinfo` contract as the *selected modelling consumer*
(corepc-types 0.13.0 `v30::GetMempoolInfo` via bitcoind-async-client
`Reader::get_mempool_info`), not "the consumer", and say explicitly that
whether T0's `RpcNodeControl` route requires the full v30 superset is the
T0 owner's statement to make; (b) section 5 should give the worker one
aggregation sentence: any projected row with availability != `passed`
is a traffic-surface blocker irrespective of the bark verdict, and the
`protocol.id-echo` / `protocol.envelope20.getnetworkinfo` rows apply to
the traffic surface too.

### D6 (hygiene, small) - failure-path cookies and inherited scratch

`cmd_node_probe` (`cli.py:140-195`) calls `remove_scratch()` only after
a successful report; if `wait_ready()` raises, the `finally` stops the
node but the 0600 cookie stays in `/tmp/opencode/<token>/<label>/`. Move
the removal into the `finally` (honouring `--keep-scratch`). Five v1-era
scratch dirs with cookies remain under `/tmp/opencode/AP1a-*` from the
author's earlier runs; not touched by me, should be removed by the
owner.

### D7 (binding, recommended) - the run manifest does not bind devcheck

`reporting._hash_tree` excludes `devcheck/` by design (documented). The
packet-level review manifest does include the six devcheck files, so the
freeze is bound at that level, but the per-run `manifest.json` that
travels with the evidence cannot tell a consumer which devcheck sources
and result backed "12/12 PASS". Recommend recording
`devcheck/{Cargo.toml,Cargo.lock,src/main.rs}` hashes and the vector
result line under a `dev_verification` key, or stating in the manifest
that devcheck binding is at the packet manifest level. Also add a unit
test asserting every committed r1 `report.json`/`manifest.json`
`contract_hash` equals `contract_hash()` (today only verified by hand),
and extend `validate_report_consistency` to check that each `fail-<kind>`
reason corresponds to an actually failed kind (it currently checks order
only). Nit: `devcheck/src/main.rs:41` `GENESIS_MERKLE` has a typo
(`...618f76773e...` for `...618f76673e...`); harmless as a 64-hex
placeholder, but it is labelled as the genesis merkle root.

## R5. Confirmed sound in r1

- Evidence integrity: 40 rows per node, identical across both binaries,
  receipts on every exchange (HTTP 200/500/404, integer ids echoed,
  `jsonrpc: "2.0"` echoed on 2.0 calls), schema v2 + consistency
  validation passing, contract hash embedded and matching.
- Verdict honesty: `incompatible` with `fail-version-gate` on both;
  `funded_state: unproven`, `full_acceptance: false` permanent;
  `version_310000_only` yields `surface_compatible` and never more;
  auth / parse-error / string-id (200 path) scenarios yield `unproven` or
  `fail-protocol`, never method evidence.
- Contracts now match the pinned serde types where the first review
  found divergence (empty hex, anchor split, u64 range, null-reject,
  `nTx`, v29 `bits/target/time`, uppercase estimate mode); verified
  against the registry sources and by the devcheck crate I re-ran.
- Safety: fresh owned nodes, loopback, cookie 0600 never in argv or
  text, reaped, scratch removed on success; no mining, no funding, the
  review-gallery and other sessions' nodes untouched.

## R6. Required before approval of the hash

1. C1: enforce id echo on the non-200 path; add the error-path fake
   server mode and scenario.
2. C2: fix the three `ENCODINGS` statements and the probe forms they
   drive; re-freeze and republish the hash.
3. D1, D2, D3: correct the three documents.
4. D5: narrow the `getmempoolinfo` consumer wording; add the projection
   aggregation rule.

D4, D6, D7 are recommended, not blocking.

## R7. Explicit non-approvals (unchanged)

- Contract hash `11bdb100...` is **not** approved; neither are
  `48894548...` or `905bf4b8...`.
- No Core RPC-version compatibility claim, no `bark-v31` named profile,
  no node patch or version override, no bark acceptance. r1 section 8
  "None blocking" does not authorise the named compat profile; that
  requires this approval plus the funded/witness gate.
- AP-1b, AP-2, AP-3 scopes are not reviewed here.

---
---

# Fable re-review r2 - AP-1a probe, C1/C2 + D1-D7 closure (2026-11-08, final)

**Decision: APPROVE - the bounded read-only probe and the exact contract
`f4fc3e043f420870675095ab24144ba14e0afba6cb9ad69d818d99b1bccb2f62`.**
Every blocking and required item from the r1 re-review is resolved in
code and evidence, verified against the pinned sources and by re-running
the candidate's gates and my own counterexample. Three cosmetic notes
remain (R2-N1..N3); none affects the contract, the evidence, or the
verdict, and none requires a new hash.

Approval scope is exactly: (a) `testing/ark-rpc/**` as frozen by the r2
review manifest `f0aa504a...` as a read-only node capability probe, and
(b) the canonical contract JSON hashing to `f4fc3e04...` (contracts,
encodings, check ids with coverage class / `unknown_behavior_ok` /
funded flags, verdict rules) as the AP-1a lineage T0 may bind to after
the recorded decision. Nothing else; see R2-7.

## R2-1. Freeze and provenance

| item | result |
|---|---|
| r2 review manifest `/tmp/opencode/lab-ark-rpc-probe-r2-review-manifest.json` | 74 `(path, sha256, mode)` entries match the tree; untracked set == manifest set (caches excluded); sorted-key compact JSON of `{base, files}` reproduces `f0aa504aa6f247474eb247b4c25d12f286bb3f88abb1d42e77c3ba643f65ce12` |
| Historical evidence | all 16 v1 + r1 evidence files byte-identical to the r1 review manifest; v1 and r1 docs carry dated corrections only |
| r2 run manifests (`reports/2026-11-08-r2-*/manifest.json`) | token `AP1a-1fd3d1813c33` in both manifests and both reports; 38 runtime files frozen, every hash equal to the r2 review-manifest hash for the same path, file maps identical across nodes; `contract_hash` == `f4fc3e04...` in all four artefacts and in `./probe.py contract-show` |
| `dev_verification` | present in both manifests, identical; `devcheck/{Cargo.toml,Cargo.lock,src/main.rs}` hashes equal the frozen tree; exit 0, 12 vectors, receipt text embedded. I re-ran `cargo run --offline -j 2` on the frozen `devcheck/` (incremental; 36 GB free respected): 12/12 PASS on the final `main.rs` (with the corrected `GENESIS_MERKLE`) |
| Candidate tests re-run | `python3 -m unittest discover -s tests`: **113 OK**; `./probe.py self-test`: **20/20**; 20 scenario files present, all parse |
| Node binaries | `eb257af7...` / `bcb6d5ee...e173b4b9...` in reports, manifests, parent one-pager r2 and r2 doc; D3 typo gone |
| ENOSPC recovery | fixtures all parse and are non-trivial (27 entries; the `[txid, true, null]` entry is gone); `fixtures/base_responses.json` `captured_run_token` is `AP1a-4ab95813a3b2`, a pre-final r2-cycle capture used only as the self-test replay base (not evidence); the r2 evidence dirs carry the final token and the `dev_verification` receipt built from the frozen sources, so the evidence is source-bound and not a rebuilt-proof mislabel |
| Hygiene | no `AP1a-*` scratch dirs under `/tmp/opencode`; no owned nodes running; the five v1-era dirs flagged in D6 are gone |

No candidate edits, commits, subagents, shared-cache deletion, mining or
fresh node runs by me this round: the r2 `raw_responses.json` receipts
answer every question a node run would have.

## R2-2. C1 - error-path id echo: RESOLVED (regression confirmed)

- `client.py:272-312` (`HTTPError` branch): a parseable non-200 body
  without `id` -> `bad_jsonrpc`; with `id_match` false -> `id_mismatch`
  *before* any error code is interpreted; only then `-5`/`-32601`/etc.
- `probe.py:124-141` `ProbeSession.call`: every receipt is counted
  (`id_match` true/false) regardless of category.
- `_check_id_echo` now runs after `_observe_zmq` (`probe.py:1246-1250`);
  both r2 reports read "all 30 exchanges (200 and HTTP-error paths
  alike)". The r2 receipts show 8 non-200 exchanges (7x HTTP 500, 1x
  404) plus the JSON-RPC 2.0 error at HTTP 200, all with integer ids
  echoed exactly.
- Fake server: error bodies over HTTP 500 (404 via
  `error_http_status`), `id_echo_mode` in {`string`, `string-errorpath`,
  `wrong-int`, `missing`, `bool`, `null`}; six scenarios pin
  `protocol.id-echo` failed with `*.recognized` and error-path rows at
  availability `unknown`.
- **My r1 counterexample re-run against the r2 code** (fake server, 500/404
  error bodies with string ids, integer ids on 200): verdict
  `incompatible [fail-version-gate, fail-protocol, funded-unproven]`;
  `protocol.id-echo` = `failed / failed` "9 of 30 exchanges echoed the
  request id incorrectly"; `getrawtransaction.verbosity1`,
  `getmempoolentry.recognized` = `unknown / unknown`;
  `sendrawtransaction/testmempoolaccept/submitpackage.recognized` =
  `unknown / not_run`, all with "protocol-level failure, not method
  evidence: id_mismatch". No `-5 -> None` or "method routed" claim
  survives behind a bad id.

## R2-3. C2 - wire forms vs pinned call sites: RESOLVED

Captured r2 requests (both nodes identical):

| wire form sent | pinned call site it mirrors | verified |
|---|---|---|
| `getrawtransaction [txid, true]` envelope 2.0 | `bitcoincore-rpc-0.19.0 src/client.rs:507-513` and `bitcoin-ext/src/rpc/mod.rs:325-333` (`custom_get_raw_transaction_info`): args `[txid, true, null]`, `handle_defaults` (`client.rs:116-147`) returns `&args[..required_num]` when all optionals are null -> `[txid, true]` | yes; the null form is gone from probe, fixtures and scenarios |
| `getrawtransaction [txid, true]` envelope 1.0 | `server/src/bitcoind.rs:137-151`, `bark/src/chain.rs:868` (`call_raw`) | yes |
| `getrawtransaction [txid, 0]` / `[txid, 1]` 1.0 | async `get_raw_transaction_verbosity_zero/one` | yes |
| `estimatesmartfee [t, "ECONOMICAL"]` 1.0 | `bark/src/chain.rs:337-344` with `EstimateMode::Economical` (`rename_all = "UPPERCASE"`) | yes; `ENCODINGS.estimatesmartfee.sync` now documents `[target]` (null stripped) and is not sent (no bark caller) |
| `testmempoolaccept [[hex]]`, `submitpackage [[hex]]` 1.0 | `server/src/bitcoind.rs:207-221`, `bark/src/chain.rs:685`; callers `offboards.rs:485`, `exit/transaction_manager.rs:470,497,630`, `actions/board.rs:490` all async | yes; `ENCODINGS` now records `.async` 1.0 and labels the sync `broadcast_package` as caller-less in `c419159` |
| `getrawmempool []` 1.0 and `[false]` 2.0 | async `get_raw_mempool`; `bitcoin-ext rpc/mod.rs:393` | yes |
| `getblock [hash, 0]` 1.0, `[hash, 1]` 2.0, `[hash]` 1.0; `getblockheader [hash, true]` 1.0/2.0, `[hash, false]` 2.0 | async `get_block`; bdk `get_block_info` (`client.rs:344-346`); async `get_block_height`; `server/src/bitcoind.rs:108`; sync `get_block_header_info`/`get_block_header` | yes |

No mode-string or default drift remains. Invalid `"00"` samples still
prove recognition only; all three rows are `not_run` on behavior.

## R2-4. D1-D7 cross-check

| item | verified |
|---|---|
| D1 | `r1.md:21-38` and the parent one-pager carry a dated correction with the primary absolute path; the r2 doc section 0 states it directly. |
| D2 | `r1.md:132-134` reads `AP1a-b1aeb688e017` with the dated note; r2 docs bind `AP1a-1fd3d1813c33`; both match the committed artefacts. |
| D3 | `...e173b4b99a...` everywhere. |
| D4 | `getbestblockhash` is coverage `observational` (`contracts.py:1276-1279`, contract `:573-576` names the async-client API and "no bark c419159 caller"); `compute_verdict` and `validate_report_consistency` skip it like `traffic`; `VERDICT_RULES.observational_coverage` added (`:1457-1465`); no `v29.rs` citations remain. |
| D5 | `contracts.py:871-880` and `ENCODINGS.getmempoolinfo.async`: "SELECTED MODELLING CONSUMER: corepc-types 0.13.0 v30::GetMempoolInfo via bitcoind-async-client Reader::get_mempool_info ... bark c419159 itself never calls getmempoolinfo, and the T0 RpcNodeControl consumer is NOT automatically this consumer". The row stays `passed / failed` (`maxdatacarriersize: null`, uncoerced) on both nodes; verdict reasons unchanged. r2 doc section 4 and README give the aggregation rule: every projected availability must pass on its own; `protocol.id-echo` / `protocol.envelope20.getnetworkinfo` and network/genesis rows apply independent of the bark version floor; `unknown`/`not_run` is never a funded pass. |
| D6 | `cmd_node_probe` outer `finally` removes scratch on every path unless `--keep-scratch` (`cli.py:237-255`), owned dir only; no scratch left by r2. |
| D7 | `dev_verification` bound and verified (R2-1); `--devcheck-receipt` fails closed on a missing/failing receipt (`cli.py:129-135`); `run_two_nodes.sh` runs the gate first. `tests/test_reporting.py` pins r2 `contract_hash == contract_hash()` (`:104-112`), r1 as historical `11bdb100...` schema-only (`:114-123`), devcheck source hashes (`:149`), approved binary pins (`:213`). `validate_report_consistency` re-derives the full reason list (`schemadef.py:400-420`); I confirmed a fabricated `fail-shape`, a dropped check row, and a `surface_compatible` status over failing rows are each rejected on the real r2 report. `GENESIS_MERKLE` corrected (`devcheck/src/main.rs:45`). |

## R2-5. Evidence and verdict (unchanged and honest)

Both approved binaries: 40 identical rows, `incompatible`
(`fail-version-gate`, `funded-unproven`), version `190000` vs `310000`
server / `290000` wallet, `funded_state: unproven`,
`full_acceptance: false`. Unknown rows are the Core-identical genesis
`-5` (four `getrawtransaction` forms), unfunded `getmempoolentry`, and
empty-mempool fee data; `not_run` rows are the three mutating
recognitions. The `getmempoolinfo` traffic row is `passed / failed` for
the selected modelling consumer only. Nothing is laundered: a
`version_310000_only` replay yields `surface_compatible` and never more.

## R2-6. Non-blocking notes (no hash change needed)

- **N1** r2 doc section 2 C1 says "including the 9 HTTP-error receipts";
  the receipts show 8 non-200 responses plus one JSON-RPC 2.0 error
  carried over HTTP 200 (9 error *exchanges*). Wording only.
- **N2** `run_two_nodes.sh` honours `SKIP_DEVCHECK=1`, which omits
  `dev_verification` from a manifest. Acceptable for dev convenience
  because `test_r2_manifest_binds_devcheck` requires the key on committed
  evidence; any future evidence drop should keep that test.
- **N3** The self-test fixture base is from capture `AP1a-4ab95813a3b2`
  (pre-final r2 cycle), not the evidence token. Fine for a replay base;
  if fixtures are ever regenerated, regenerate from the evidence capture
  so the two tokens coincide.

## R2-7. Explicit scope of this approval, and non-approvals

Approved: the AP-1a read-only probe as frozen (`f0aa504a...`) and the
exact contract `f4fc3e04...` as the canonical lineage T0 may bind to,
with the check-id / coverage / `unknown_behavior_ok` / funded table as
emitted in `canonical_contract_json()`. Any change under
`testing/ark-rpc/**` requires a new evidence run and a new hash.

Not approved, unchanged from R7:
- No Core RPC-version compatibility claim; both binaries fail the bark
  gate at `190000` and nothing here makes them pass.
- No `bark-v31` named compatibility profile, node patch, or version
  override; the probe is evidence infrastructure for that decision, not
  the decision.
- No bark acceptance, no Ark or traffic runtime claim; T0/T1/node-profile
  work is not approved through AP-1a. The `getmempoolinfo` row is a
  selected-consumer result, not a T0 requirement or a universal traffic
  failure.
- Funded behavior, witness/preflight proofs (T3) and live bark remain
  separate gates; `funded-unproven` is permanent in this mode.
- AP-1b, AP-2, AP-3 scopes are not reviewed here.
