# Historical AP5 development notes (not fresh-machine setup instructions)

This is an execution harness, not another signer, miner, Ark server, or payout
implementation. `ap5.py` uses the joined runtime's native bootstrap and
`send_user_payment`, the real pool's mining and payout records, the agent's
existing `--wallet-command FILE`, stock `GetVtxo`, and stock Bark's public
`dev vtxo decode`. Wallet and SQL evidence reads are read-only. Raw proofs,
transactions, operation checkpoints, input reservations and errors remain in
the new private run's hash-chained evidence directory.

**Current status:** Python consumer/controller exercised in real diagnostic
attempts; no confirmed native transfers or funded AP5 acceptance yet.
The public-only Rust VTXO verifier is compiled and tested in Nix; its six
unconditional conformance tests pass with the unchanged frozen Cargo.lock. Full
execution requires the corrected tested joined bundle, including pool-control
and aggregate source policies. See [CONTRACT-v2.md](CONTRACT-v2.md) for the
sealed result, component replay, source binding and cleanup contract.

The corrected Runtime bundle was executed and its asynchronous bootstrap wait
worked. Native bootstrap then encountered absent operational P2P wiring: node
A reached height 1 while B stayed at genesis, both with zero peers. It was
stopped under the original bootstrap id. See
[NETWORK-BOOTSTRAP-BLOCKER.md](NETWORK-BOOTSTRAP-BLOCKER.md); the older
[phase blocker](RUNTIME-BOOTSTRAP-BLOCKER.md) remains historical. No user-payment
transfer or funded Ark acceptance is claimed.
Future bundles must explicitly list the executed probe interpreter/script and
source/argv bindings; see [PROBE-INVOCATION-HANDOFF.md](PROBE-INVOCATION-HANDOFF.md).
The retained r2 probe baseline is used without repeating the read-only probe.

The verifier's embedded signed synthetic-conformance fixtures provide an
unconditional valid positive and signature/policy/server/anchor negatives.
No live corpus is required to compile or run those tests. Live proof remains
mandatory for the first actual AP5 board's funded acceptance.
Full-bundle admission now uses the parent-verified immutable compiled artifact
and its nine source/receipt bindings, rather than the mutable verifier tree;
see [COMPILED-VERIFIER-ADMISSION.md](COMPILED-VERIFIER-ADMISSION.md).

## Commands

Use Python 3.11+ (the Nix Python is suitable):

```sh
python3 -B testing/lab-ark-funded/ap5.py prepare --out /tmp/opencode/ap5-prepared-new
python3 -B testing/lab-ark-funded/ap5.py check-bundle \
  --bundle /path/to/serial-owner-bundle.json --out /tmp/opencode/ap5-check-new
python3 -B testing/lab-ark-funded/ap5.py run \
  --bundle /path/to/serial-owner-bundle.json --out /tmp/opencode/ap5-live-new
python3 -B -m unittest discover -s testing/lab-ark-funded -p 'test_*.py'
```

`run --through confirmed-funding` is an explicitly labelled diagnostic: it
verifies native transfers to the published provider/pool addresses and waits
for real pool-mined confirmations, then stops. It is not AP5 acceptance. The
default is the complete lifecycle path. Every run is bounded (default 7200 s)
and stopped/reaped on success or failure. There is no existing-run adoption.
The protected inspection/live directories and their ancestors/descendants are
refused before filesystem effects.

Diagnostic admission is based on the actual Nix pool source. A tested closure
without wallet-control uses the closed
[diagnostic source policy](DIAGNOSTIC-SOURCE-POLICY.md), not an unrelated dd9
control policy. Actual control-bearing source retains its corrected-source
hold in every scope. Both profiles require exact critical source, Ark-feature
and compiled UI checks. The diagnostic emits only a native-funding verdict;
it cannot invoke Ark client/lifecycle commands or the full AP5 aggregator.

The generated default is exactly two nodes, two pools, four SP identities,
four wallet processes, four miner processes at 10 H/s each, eight background
traffic wallets, four separate native user-payment wallets, 0.2 ordinary tx/s,
one stock server and two pool Ark roles. Native bootstrap owns the same 40 H/s
authority before worker activation. It starts with 101 maturity blocks and
eight allocation outputs per traffic wallet (96 outputs for this roster, not
the 80-output two-user T2 diagnostic). Node consensus and the bark-v31 selector
are unchanged.

## Required joined executable bundle

The serial owner supplies JSON with schema `spc.lab.ap5.bundle/v1`:

- `base_revision`: actual source base.
- `joined_source_manifest`: `{path, sha256}`, with `files` rows carrying
  `path` and `sha256` for the joined source.
- `joined_validation_receipt`: `{path, sha256}`. Its JSON schema is
  `spc.lab.ap5.join-validation/v1`, with `source_manifest_sha256`, `binaries`
  (logical names below to actual SHA256), and nonempty `checks` records
  `{name, exit: 0, log: {path, sha256}}` pointing at the actual combined checks.
  Full-profile admission also requires the feature-aware check purposes and
  complete compiled UI source/browser bindings in CONTRACT-v2.md. Default
  `cargo test -p spc-agent` does not cover the Ark wallet feature.
- `binaries`: `{name: {path, sha256}}`, mandatory names `lab`, `txd`, `pool`,
  `agent`, `miner`, `scanner`, `node`, `captaind`, `grpcurl`, `probe`,
  `redis-server`, `redis-cli`, `psql`; full lifecycle also requires `bark`.
- `postgres_bin_dir`: the matching Nix PostgreSQL bin directory.
- `history_version: 3` and factual joined-runtime contracts
  `bootstrap_and_workers_single_budget: true`,
  `host_peering_before_profile_binding: true`,
  `fresh_registration_before_workers: true`.
- Full lifecycle: `pool_wallet_executor: {path, sha256}`, described below.
- Full lifecycle: `vtxo_verifier: {path, sha256, cargo_lock_sha256}` from
  `verifier/package.nix`; source and installed dependency lock are checked.
- Full lifecycle: `pool_control_source_policy` and `aggregate_source_policy`,
  each `{path, sha256}` pointing at corrected producer facts: `compiled: true`,
  `tests_passed: true`, `source_snapshot_sha256`, `files` (path-to-SHA256 map),
  and nonempty `checks: [{name, exit: 0, log: {path, sha256}}]`.
  The actual joined Nix source must match every policy file. Known defective
  pool `55a488...` and aggregate `415d21fa...` snapshots are rejected.

The six joined application executables must resolve to source-bearing Nix
derivations over one actual source input. The harness queries their derivers;
it does not trust a filename such as `joined` or a caller-supplied source path.
Thirteen critical AP3 R1-R6 and T2/context/DEST files must match the approved
rows in `critical-sources.json` in both the manifest and actual Nix input. A
later legitimate code amendment needs an explicitly source-verified policy
update, not a bypass switch. The old `w52v4f6...` closure fails this check.

Required joined CLI flags are `runtime --txd --traffic-probe
--wallet-agent-bin --node --pool --miner --scanner --captaind --grpcurl
--postgres-bin-dir --redis-server --redis-cli --scenario --run-dir
--base-revision --dirty-tree-hash --listen --operator-ui --duration-secs`.

The immutable release node must retain SHA256 `bba4752298aff258adb6cce99c6db849b02f50736a8ec32766ed1710045051aa`;
stock captaind must retain `4cac675bccc017608378382129613ed5202dd90e7300a92c4ed799869abaef9d`.
The existing self-hosted Nix package bindings supply node, server, storage,
grpcurl and canonical probe paths. No new node/server build is needed. Stock
Bark's CLI decoder must be Nix-built at c419159 and carry its source-revision
file; decoding never opens a wallet or adds a signer.

## Actual execution and remaining artifact dependencies

1. The joined runtime provisions real storage/fronts/wallets and performs
   native bootstrap. Four owned wallet processes must have fresh committed
   registration leases before the runtime admits user miner connections.
2. Query stock captaind's current wallet address. Use the existing native user
   signers to send 200M sats each from Alice/Bob to the provider and 120M each
   from Carol/Dave to the corresponding published pool onchain address. These
   are explicit funding amounts, not balance assumptions. Each transfer waits
   for three canonical confirmations on both nodes and a real pool block record.
3. Board each pool to an actually observed **115M-sat net spendable float**, with
   real fee policy and a 5M-sat bound. Target configuration never counts as a
   balance. Insufficient float does not pause miners.
4. Wait for SP-owned raw receipt proofs and matching production `ark_payout`
   sent rows. This check deliberately blocks if the ladder/aggregate attribution
   is not yet available; a direct test payment is not relabelled mining-derived.
5. Queue an SP refresh using explicit owned inputs. Require the version-2
   retained signed-output checkpoint, real round confirmation and imported child
   VTXOs rooted in that round. Watch the **automatic second stock renewal**:
   no second manual refresh command is sent. Require matching operation inputs,
   old inputs spent, new actual expiry, and new outputs rooted in its real round.
6. Use the same agent command executor for normal and direct SP offboards;
   retain results/reservations and wait for both nodes to confirm the actual
   transactions. Missing replies remain unknown and do not generate new IDs.

The pool-control owner's tested r3 same-live-wallet queue now includes retained
original-board proof reads. Source snapshot `dd9f0c37...`, code `8ec102f5...`,
has 25 final focused tests plus build/clippy evidence; its earlier 54 Ark tests
are not counted as 79 unique final tests. The parent reports auditor recheck
ongoing. This closes the client interface gap. Full execution still requires
the corrected control/grant/ladder code in one tested joined Nix bundle;
the old `55a488...` implementation remains inadmissible.

The stable CLIENT executor contract is:

```text
pool-wallet-executor --config <actual pool-ark/pool.toml> --request <private JSON>
```

Request: `schema=spc.lab.ap5.pool-wallet-request/v1`, immutable `operation_id`,
`action=snapshot|board_float`; board adds `net_target_sats` and `max_fee_sats`.
Reply: `schema=spc.lab.ap5.pool-wallet-result/v1`, matching `operation_id`,
`state`, actual `onchain_trusted_sats`/`float_spendable_sats`; a board submission
returns `funding_txid` and actual fee/board facts. `snapshot` must be a fresh
observation (the harness gives it a fresh correlation id). Mutation requires
the live role's one writer/stock wallet, or an implemented controller-owned
exclusive handoff; no second wallet DB or seed. Unknown board attempts stay
unknown and cannot be resubmitted under a fresh operation id. The payload has
no SP identity fields or block hash. It is a pool client interface, using
standard stock Ark RPCs. Ordinary `board_psbt` errors after proposal persistence
retain the same unresolved ID. Mempool absence and currently unspent inputs do
not release an ambiguous reservation.

Confirmed is historical: its original full VTXO/anchor must be verified even
after maintenance spends it. Current float is checked separately with a fresh
snapshot and current wallet rows. `board_proof_v1` uses a new read operation id
plus the unchanged `original_board_operation_id`, with no money fields. Queued
reads poll with identical bytes/id; later missing/unavailable reads can use a
fresh read id. Original proof retrieval uses retained same-wallet bytes, not
remote `GetVtxo` after pruning. Missing proof leaves AP5 inconclusive.

The new optional Confirmed `retained_proof_sha256` binds the retained canonical
object. Every other original terminal field must match its canonical context;
no balance/tip/time is invented on the historical read. Recorded movement
lineage is always partial, and cannot replace the separate current proof set.

The reported whole-lab-complete flag remains false until the independently
owned T3/T4b and ladder oracle work closes. No old T2 inert-process run or old
unfunded AP3 closure is accepted as funded evidence.
