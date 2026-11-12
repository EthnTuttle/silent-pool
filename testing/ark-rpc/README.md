# AP-1a ark-rpc probe (read-only node capability probe)

## Portable Nix package (2026-11-12)

`nix/lab/ark-probe.nix` uses this repository directory, filtered to an explicit
14-file canonical runtime closure plus the 24 public JSON fixtures needed by
`self-test`. Reports, devcheck builds, private configs and caches are excluded
from the installed package. The lifecycle fixture resolves its own directory;
its installed `--compat` behavior matches the previously deployed r4 helper.
Python tests honor `TMPDIR`, including on machines without `/tmp/opencode`.

The RPC contract remains `754e1e6131955f9e5bbce7ef81b6f9aabaa2f1379ec6409f1c3a7a9e88be91f9`.
The thin `probe.py` entry still has SHA256 `833288d7...`, but that is NOT the
identity of the full package. The portable lifecycle file and new derivation
paths require fresh source and program bindings; do not reuse old manifests.
Each build writes, under `$out/share/`:

- `ark-rpc-source-manifest.json`: actual installed 14-file hashes/root, retaining
  `spc.lab.ap5.probe-source/v1` shape;
- `ark-rpc-fixtures-manifest.json`: actual installed public fixture hashes;
- `ark-rpc-program-binding.json`: native ELF Python, script and wrapper paths/
  hashes plus both manifest hashes and the unchanged contract hash.

The runtime adapter remains native `${python3}/bin/python3` with the derived
`<probe>/share/ark-rpc/probe.py` argument. The runtime appends `node-probe` once;
the convenience wrappers do not append a subcommand. Rebind the actual output
tuple and complete source manifest in fresh runtime admission. No node/server
build or RPC/normalization policy change follows from this packaging change.
See `docs/decisions/2026-11-12-portable-canonical-probe.md` for provenance.

## Traffic J1/J2 candidate (2026-11-08)

The same executor now emits the size-only `spc-lab-tx-rpc-consumer/1`
check and retains the full corepc-v30 model failure verbatim. The Bark
verdict remains separate. `node-probe --launch-profile` is the closed,
fresh-regtest profile mode with exact finalized env, RFC8785 comparison
and the same-built-spc-lab two-level `actor-exec` lifetime handoff.
Legacy invocation retains its separate default launcher semantics.

See `docs/reviews/2026-11-08-traffic-probe-profile-implementation.md` for
the J2 caller API, shared goldens, source-layer binding and pending
Rust/process/Fable gates. Existing r2 reports and manifests are historical
evidence under `f4fc3e04...`; they are not regenerated for this contract.

Safe, read-only RPC capability probe for the Ark vertical: it checks an
rbitcoin regtest node against the RPC surface and response shapes that
the **pinned bark `c419159`** actually parses, and produces a two-axis
gap matrix plus an honest verdict.

The probe is Python 3 stdlib only. It never modifies node code, never
fakes a Core version, never funds or broadcasts anything, and only ever
talks to nodes it owns (or an explicitly given `--rpc-url`).

## Statuses

Every check is reported on two axes:

| axis | meaning | statuses |
|---|---|---|
| availability | did the node produce a usable RPC exchange (or the expected domain error)? | `passed` / `failed` / `unknown` |
| behavior | does the observed exchange satisfy the bark contract? | `passed` / `failed` / `unknown` / `not_run` |

- `unknown` = ran but unproven (empty-mempool fee data, mempool-entry
  shapes that need a funded transaction, ...). Missing data is never a
  pass.
- `not_run` = deliberately not exercised. The funded broadcast paths
  (`testmempoolaccept` / `submitpackage` / `sendrawtransaction` with a
  valid funded transaction) are classified `not_run`; an invalid sample
  (`"00"`, valid hex, not a transaction) proves method recognition only.

## Verdict

Three statuses (mirrored by `contracts.VERDICT_RULES`, `schemadef`, and
the schema):

- any failed check -> `incompatible`, reasons ordered:
  `fail-version-gate`, `fail-method-surface`, `fail-encoding`,
  `fail-protocol`, `fail-network`, `fail-genesis`, `fail-shape`, then
  the permanent `funded-unproven`.
- no failures but some gating check (coverage `required` /
  `encoding` / `protocol`) not proven available -> `unproven` with the
  `surface-unproven` reason. Protocol-level failures (auth,
  transport, -32700/-32600 protocol errors, id mismatches) are never
  method evidence and must never be laundered into a compatibility
  claim.
- no failures and every gating check proven -> `surface_compatible`.

Checks with coverage class `traffic` (the T0 lab-traffic-contract
subset: `getmempoolinfo`) never gate the verdict in either direction:
bark `c419159` itself does not call the method, so their honest
passed/failed rows are traffic-slice information only (a traffic
failure adds no reason; a traffic pass satisfies no requirement).

Checks with coverage class `observational` (`getbestblockhash`,
`getnetworkinfo.observe`) behave exactly like traffic rows: they are
honest rows for the pinned async client's API surface / lab
bookkeeping, but bark `c419159` has no caller for them
(`getbestblockhash` is only reachable through the async client's
`get_current_timestamp`, v30.rs:107-110, which bark never calls), so
they never gate the verdict in either direction either.

`funded_state` is permanently `unproven` and `full_acceptance`
permanently `false` in read-only mode: a 310000 version alone must NOT
yield full acceptance, and there is no `full_compat` outcome at all.

Both probed rbitcoin binaries report RPC version 190000 while bark
requires >= 310000 (server) / >= 290000 (wallet), so the version gate
**fails honestly** on both. That is the correct, expected outcome of
this packet; the probe exit code reflects probe health, not node
compatibility.

## Layout

```
probe.py                 launcher (thin; imports arkrpc.cli)
arkrpc/
  contracts.py           named profiles + serde-mirroring contract specs
                         (every spec cites the consuming Rust type)
  client.py              JSON-RPC client: cookie auth, deadlines, bounded
                         body, error categories, redaction
  probe.py               check orchestration + verdict logic
  nodeproc.py            owned fresh regtest node lifecycle
  reporting.py           report.json / report.md / manifest / gap matrix
  schemadef.py           report schema + stdlib mini-validator
  selftest.py            in-process fake JSON-RPC server + scenarios
  cli.py                 subcommands
fixtures/
  base_responses.json    normalized exchanges captured from a real
                         owned node run (self-test replays them)
  scenarios/*.json       20 self-test scenarios incl. the load-bearing
                         version_310000_only (must be
                         surface_compatible with funded-unproven,
                         never full acceptance),
                         traffic_mempoolinfo_ok (traffic rows never
                         gate the verdict in either direction), and
                         the id-echo family (id_string_echo plus the
                         C1 variants: string echo on the error path,
                         wrong-int, missing, bool, null -- the id echo
                         is enforced on the 200 AND the HTTP-error
                         path, and no error code behind a broken echo
                         is ever interpreted)
tests/                   unit tests (contracts, selftest, reporting);
                         #devcheck-marked tests have Rust twins in
                         devcheck/
devcheck/                dev-only Rust checker: validates the
                         #devcheck golden vectors against the real
                         pinned crate types (excluded from the
                         manifest hash BY DESIGN; every r2 manifest
                         binds its exact sources and its gate receipt
                         via the dev_verification key)
reports/                 committed evidence: the r2 two-node runs
                         (2026-11-08-r2-*, schema v2, current
                         contract hash) + gap matrix, the historical
                         r1 runs (2026-11-08-r1-*, pinned to their
                         historical contract hash, validated
                         schema-only), plus the frozen v1 runs
                         (l2-eb257af7, v09lab-bcb6d5ee) kept as
                         immutable history
report.schema.json       schema for report.json (generated by schema-dump)
run_two_nodes.sh         reproduces the committed r2 evidence
                         (devcheck gate first, fails closed)
```

## Exact commands

Run everything from this directory (`testing/ark-rpc`):

```sh
# unit + scenario tests (no node binaries needed)
python3 -m unittest discover -s tests
./probe.py self-test

# the two-node evidence run (approved binaries; fresh owned nodes).
# Runs the devcheck golden-vector gate first (fails closed; its
# receipt is bound into every manifest via --devcheck-receipt), then
# writes reports/2026-11-08-r2-*/ (schema v2); the historical r1 dirs
# and the frozen v1 dirs (l2-eb257af7, v09lab-bcb6d5ee) are immutable
# history and are never rewritten.
./run_two_nodes.sh

# equivalent single-node invocation (add --devcheck-receipt <json> to
# bind the devcheck gate result into the manifest)
./probe.py node-probe \
  --binary /tmp/opencode/rbitcoin-lab-retarget-profile/target/debug/rbitcoin-node \
  --label rbitcoin-lab-retarget-profile-r2 \
  --source-name rbitcoin-lab-retarget-profile \
  --out reports/2026-11-08-r2-l2-eb257af7

# probe an already-running node instead
./probe.py probe --rpc-url http://127.0.0.1:PORT --cookie-file ./cookie --out outdir

# the frozen contract set (absorption interface for the future Rust
# node_probe; its sha256 is recorded in every run manifest)
./probe.py contract-show

# regenerate fixtures after a capture
./probe.py make-fixtures --capture <run>/raw_responses.json --out fixtures/base_responses.json

# dev-only: validate the #devcheck golden vectors against the real
# pinned Rust crate types (needs the pinned bark checkout + cargo
# registry cache; not part of the probe or the manifest hash)
./devcheck/run.sh
```

`node-probe` records the binary's pin name via `--source-name` (the
build label, never a raw local path as the identity) and cleans up its
scratch directory (including the cookie credential) automatically;
pass `--keep-scratch` to keep it for debugging.

## Node ownership and safety

`node-probe` starts a **fresh** regtest node per run under
`/tmp/opencode/AP1a-<token>/<label>/`:

- loopback RPC on a free port; `--no-listen --no-seeds --no-discover`
  (no peers, ever);
- the RPC credential is generated by the launcher, lives only in a
  cookie file (`user:pass`, mode 0600, no trailing newline), and never
  appears in argv, reports, or logs;
- `--max-run-secs` bounds the node's own lifetime as a backstop;
- the node is reaped by the probe (SIGTERM, then SIGKILL, with waitpid
  verification) and its exit status is recorded;
- `RBITCOIN_RPC_PACKAGE_DIALECT=1` is set in the node environment (an
  RPC-surface knob in the rbitcoin source, not a consensus parameter;
  provenance is recorded in every manifest).

No mining, no block generation, no transaction funding: the only
transaction the probe looks at is the genesis coinbase, and the only
"mutating" calls are invalid samples that both Core and rbitcoin
reject before any state changes.

## Contract authority (why these shapes)

The contracts in `arkrpc/contracts.py` are derived by reading the
pinned sources, not from a checklist:

- bark `c419159`: `server/src/bitcoind.rs` (version gate 31_00_00,
  `getmempoolentry` chunk shape, tx_status via `getblockheader`),
  `bark/src/chain.rs` (wallet gate 290000, fee targets 1/3/6
  economical, mempool ancestor info), `bitcoin-ext/src/rpc/mod.rs`
  (GetRawTransactionResult, SubmitPackageResult, try_get_tx_out).
- exact crate versions from bark's Cargo.lock: bitcoind-async-client
  0.15.0 (features `30_2` + `raw_rpc`; bark's tree compiles the v30
  client module, so async-client citations are v30.rs) over
  corepc-types 0.13.0 `v29::GetBlockchainInfo` (requires
  `bits`/`target`/`time`), bitcoincore-rpc-json 0.19.0 (sync
  companion; `GetBlockHeaderResult` with camelCase `nTx`),
  bitcoin-units 0.1.3 (`amount::serde::as_btc`).
- wire forms are the forms on the wire, not pre-serialization argument
  arrays: bitcoincore-rpc 0.19 `handle_defaults` (client.rs:116-147)
  strips the trailing null, so the sync `getrawtransaction` verbose
  form is `[txid, true]` (envelope 2.0) and sync `estimatesmartfee`
  is `[target]`; every active bark caller of
  `testmempoolaccept`/`submitpackage` is an async `call_raw`
  (envelope 1.0). The probe sends exactly these forms and no others.

`canonical_contract_json()` + its sha256 are the frozen absorption
interface: the future Rust `node_probe` (AP-1) must consume the same
contract set rather than re-deriving it.

## Traffic slice (T0 lab-traffic-contract projection)

The approved traffic T0 subset is exactly five methods:
`sendrawtransaction`, `getrawmempool`, `getrawtransaction`,
`getmempoolinfo`, `getblock`. The canonical report lineage is the ONLY
probe/verdict for them: the traffic consumer (worker
`ses_e4604a9d0ffeHEKwnwhfH0S61J` in lab-traffic-contract) reads these
report rows directly -- there is no second probe, client, or verdict.

- `getmempoolinfo` is covered by the coverage-class `traffic` check.
  Its consumer of record is a SELECTED MODELLING CONSUMER: corepc-types
  0.13.0 `v30::GetMempoolInfo` via the pinned async client's
  `Reader::get_mempool_info` (v30.rs:128). bark itself never calls
  `getmempoolinfo`, and the T0 lab-traffic-contract `RpcNodeControl`
  consumer is NOT automatically this consumer -- whether the T0 side
  requires the full v30 superset (and in particular an integer
  `maxdatacarriersize` rather than JSON null) is the T0 owner's
  statement to make. The row records the honest observation without
  coercion; it never gates the bark verdict in either direction.
- The other four were already covered by gating-class checks (their
  recognition/shape rows vs the funded-behavior distinction is
  preserved: funded behavior stays `not_run`/`unknown`).
- No Bark version floor is imposed on the traffic projection: the
  honest `fail-version-gate` coexists with a usable traffic surface
  (both approved binaries: every traffic method routed and shaped
  except the `getmempoolinfo` `maxdatacarriersize` null, below).
- Aggregation rule for the traffic consumer: every projected
  availability must pass on its own; `protocol.id-echo` /
  `envelope20.getnetworkinfo` and the network/genesis validation apply
  to the traffic projection independent of Bark version; a behavior of
  `unknown` or `not_run` is never a funded pass. The explicit required
  shape contract for each method is the T0 consumer's to state.

Observed on both approved binaries (r2 evidence): `getmempoolinfo` is
routed and serves all 13 v30 fields plus extras (ignored), but
`maxdatacarriersize` is JSON `null` where Bitcoin Core emits an
integer -- the pinned v30 deserializer rejects it, so the traffic
check fails honestly on both nodes. This is traffic-surface
information, not a bark-verdict change.

## Non-claims

- Funded broadcast behavior is unproven (`not_run`); it needs a funded
  fixture from a separate opt-in packet.
- rbitcoin does not serve the genesis coinbase via `getrawtransaction`
  (code -5) and returns `null` from `gettxout` for it; the verbose tx
  shapes and the non-null `GetTxOutResult` shape are therefore
  `unknown`, not pass.
- Fee estimates on an empty mempool return no feerate; `unknown`.
- rbitcoin exposes no ZMQ; bark polls via `getblockcount` /
  `getblockhash` / `getrawmempool`, so this is not a gap for the
  pinned bark.
- A passing version gate alone can never yield more than
  `surface_compatible`: funded behavior stays `unproven` and
  `full_acceptance` stays `false` in every read-only run.
