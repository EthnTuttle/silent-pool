# AP5 v2 producer and offline consumer contract

Status: implemented Python consumer fixtures, unexecuted funded runtime.
`verifier/` is compiled and tested in Nix. Its checked-in Cargo.lock is reproduced byte-for-byte
from the current workspace closure and passes offline locked Cargo metadata.
Six stock full-proof Rust tests passed in Cargo and Nix, with zero ignored; see
[verifier/LOCK.md](verifier/LOCK.md). They use embedded signed synthetic
conformance fixtures and always execute a valid signed-board positive with
exact owner/server, full policy, signature and anchor assertions. No test is
skipped because a live archive is absent. An archived live public-proof
corpus is optional additional coverage, not a build or conformance-test
prerequisite. The first actual AP5 board must provide retained live proof
through the producer's read-only interface and pass verification before
funded acceptance.
The earlier seven-file/12-test source is preserved separately at
`/tmp/opencode/AP5-orchestration-r1-preserved`.

## Entry points and sealing

Producer: `ap5.py prepare|check-bundle|run --out NEW_ROOT [--bundle BUNDLE]`.
`run` defaults to the complete declared AP5 census and a 7200-second bound.
`--through confirmed-funding` is a native-transfer diagnostic.
Its exact source-applicable policy, bounded legacy maintenance coverage and
separate native verdict are specified in
[DIAGNOSTIC-SOURCE-POLICY.md](DIAGNOSTIC-SOURCE-POLICY.md). Actual no-control
source does not require a fabricated control policy; control-bearing source
remains subject to corrected-source admission in every scope.

The producer writes immutable `NNNNNN-label.json` files and `events.jsonl`.
Each event has exactly `seq`, `file`, `sha256`, `previous`, `unix_ms`,
`record_sha256`. The latter is SHA256 of compact, sorted-key JSON for the
other fields. Sequence starts at 1, and the first previous hash is 64 zeroes.
Artifact hashes cover exact bytes, including the trailing newline.

After closing the event stream, `seal.json` contains exactly:

```json
{"root":"/absolute/preserved/root","events_sha256":"HEX","last_seq":1,"last_digest":"HEX"}
```

The parent/QA must pin the exact seal SHA256 externally. A self-supplied new
seal does not authenticate a rewritten archive. Paths remain bound to the
preserved absolute root. Retain referenced Nix closures, source manifests,
bundle, validation logs and verifier lock alongside the run evidence.

```sh
python3 -B testing/lab-ark-funded/verify_archive.py \
  --seal /absolute/preserved/root/seal.json --seal-sha256 PARENT_PINNED_SHA256 \
  --out /tmp/opencode/ap5-qa-replay-new.json
```

Replay reads evidence, queries local Nix derivation metadata, and invokes only
the admitted public-proof verifier. It opens no wallet and contacts no server.
`read_archive()` also validates prepare/admission-failure envelopes; complete
component replay requires a run that emitted its verdict and cleanup inventory.
Invalid/missing archives fail closed rather than generating a passing receipt.

## Joined source and executable admission

New bundles must include `probe_invocation` with schema
`spc.lab.ap5.probe-invocation/v1`, wrapper/program/script artifact bindings,
the exact one-script `prefix_args`, `runtime_subcommand: node-probe`, and a
hashed `spc.lab.ap5.probe-source/v1` complete source manifest. Canonical
admission verifies every listed target/hash and the wrapper's exact resolved
target; runtime and replay compare the same tuple and argv. An unlisted
interpreter/script is not authorized by hashing only the wrapper. See
[PROBE-INVOCATION-HANDOFF.md](PROBE-INVOCATION-HANDOFF.md) for the concrete tuple
and minimal launcher/assembler correction. Historical 69-test/g105 artifacts
remain under their original frozen contract; they are not rewritten as new
tuple-bearing bundles. No probe or runtime is rerun by this contract update.

The 13 critical source guards remain exact hashes. The six parent-approved
source-lineage amendments are frozen in `critical-sources.json`, referencing
`/tmp/opencode/AP5-source-lineage-r1/receipt.json` at
`d8f877b120a8105475d3c7081424ce3fb1a5db18790fc17d98cdd89fc4e75f61`.
T1 actor/admit/builder come from snapshot `6740857f...`; wallet
live_receive/maintenance/mod come from `e97ade12...`. The other seven guards,
including R1-R6 audit/round/scan protection, are unchanged. The old w52 source
still rejects. Future aggregate/wallet/pool-control amendments need their own
explicit tested hashes; there is no latest-hash or bypass mode.

Full-profile joined validation additionally requires six unique `checks`
with `purpose` values `agent-ark-tests`, `agent-ark-build`, `pool-ark-tests`,
`pool-ark-build`, `ui-build` and `ui-browser-tests`. Every check binds the exact
joined `source_manifest_sha256`, actual `argv`, integer `exit: 0`, and hashed
log artifact. Agent/pool Cargo test/build commands must explicitly select the
package and `--features ark` (or package-qualified Ark feature). Test logs must
contain executed wallet/Ark cases and a passing result. Default 48-agent-test
coverage, `--no-run` and list-only invocations do not qualify. Build checks
also bind `binary_sha256` to the admitted corresponding binary.

UI-build is the actual `cargo build -p spc-lab-cli` output. Browser checks bind
`tested_binary_sha256` to that admitted compiled CLI and provide nonempty
`required_cases` and equal `passed_cases`. `ui_source_files` is a path-to-hash
map verified against both the joined manifest and actual Nix source; it must
include the CLI main, HTTP source and all index/app/style and lab HTML/JS/CSS
assets. This prevents admitting a partial UI overlay without its original
base/http/DOM files. The current eight-packet gate is not passing: missing UI
base code is being repaired, and its default-feature agent run did not test
AP3's Ark wallet. Source lineage and an unbuilt Nix skeleton are not validation.

## Result and component verdict

The terminal event is `result`, schema `spc.lab.ap5.result/v2`. Run results
include `scope`, `status`, `native_funding_confirmed`, `funded_ark_lifecycle`,
`component_status`, `component_verdict: {file, sha256}` and
`whole_lab_complete: false`. Errors add `error` and/or `cleanup_error`.
Preparation/admission failure has no invented component-verdict reference.
For `confirmed-funding`, the reference is instead `native_funding_verdict`,
with `diagnostic_status` and the separate `spc.lab.ap5.native-funding-verdict/v1`
schema. That scope has no full component-verdict reference, no Ark lifecycle
readiness claim and no full AP5 aggregation. Its four native-transfer
components and resource cleanup are still independently replayed.

The referenced event is `ap5-component-verdict`, schema
`spc.lab.ap5.component-verdict/v2`, containing:

- `component_status`: `passed`, `failed` or `inconclusive`.
- `components`: records with `id`, `status`, `evidence: {file, sha256}`;
  passed records also have `requires` and locally computed `verification`.
- `missing`, `failed`, `unknown`: component identity lists.
- `source_bindings`: exact bundle artifact/hash, joined manifest, joined
  validation receipt, binary artifacts, pool executor and VTXO verifier.
- `cleanup`: the entire cleanup-v2 receipt.
- `whole_lab_complete: false`, `external_qa_verdict: null`.

Required components are four native funding transfers, two fresh pool floats,
and receive/first-SP-refresh/automatic-second-renewal/stock-offboard/direct-SP-
offboard for each of Alice, Bob, Carol and Dave. A float whose boarding outcome
is Confirmed additionally requires `board-POOL`; AlreadyAtTarget supplies no
invented board transaction. Duplicate components, missing dependencies,
non-passed components or unproved cleanup prevent a pass. Failed components
remain failed during archive replay. Whole-lab completion requires external QA.

## Raw evidence

Each component references a sealed `raw-component` event with schema
`spc.lab.ap5.raw-component/v2`, matching `id`, actual `maturity`, `ark_info`,
`transactions`, `vtxos` and component-specific facts. Relabelling a funding
transaction as a lifecycle component fails the required-field checks.

- Transactions: exactly two node observations, each with raw transaction,
  txid, raw inclusion block, block hash/height, exact fee, and every input's
  raw parent transaction, output index/value and raw inclusion block/height.
  Python recomputes txid/wtxid, PoW against the encoded target, Merkle/witness
  commitments, input values, claimed-height maturity and fees. Live collection
  checks each canonical block hash on both actual nodes. Contextual difficulty,
  complete header ancestry and campaign work accounting remain external T3/T4b
  checks; these component bytes alone do not establish a canonical chain.
- Ark policy: raw GetArkInfo response bytes, decoded response, bound server
  and mailbox keys, exact fee-schedule hash, endpoint and observation time.
  Replay checks raw/decoded equality and all bindings. Policy observations are
  source-bound local RPC observations, not server-signed attestations.
- VTXOs: full `raw_hex`, two-node `anchor_views`, and `expected` public owner,
  server, amount, expiry, exit delta and maximum depth. The Nix verifier checks
  full Pubkey-policy equality and calls stock `Vtxo::<Full>::validate(anchor)`.
  Public Bark decoding is not signature validation. Duplicate VTXOs reject.
- Funding: actual destination script, txid and exact declared amount.
- Board: original board facts, original full VTXO and original raw anchor;
  board fee is recalculated from the observed stock schedule. The historical
  proof need not remain currently spendable. Required copies are
  `board_request`, `confirmed_receipt`, `proof_read_request` and
  `proof_read_result`; archive replay binds them to the separately sealed
  request/reply event history and verifies the retained envelope before
  matching its exact VTXO/anchor/owner to independent crypto/chain evidence.
  The component retains the actual pre-board policy observation, not a new
  fee schedule observed after the board has been refreshed.
- Float: actual immutable board request/result, fresh Observed snapshot,
  current Spendable rows and matching distinct full proofs meeting the 64-height
  float horizon. Target is exactly 115M sats, never a configured balance credit.
- Receive: full SP-owned proofs and matching source pool `ark_payout` sent
  rows. Aggregate/Rule B derivation remains subject to its corrected source
  policy and independent QA oracle.
- First refresh: input/output identities, value delta, funding txid and both
  `checkpoint_raw` and decoded `checkpoint`. Raw text preserves nested serde
  ordering for the version-2 SHA256 binding. Released proof bytes must equal
  the verified child proofs, and checkpoint input identities must match.
- Second renewal: retained operations/payloads, actual bounded trigger height,
  input/output census, round ids, previous expiry and manual-operation ids.
  Replay reconstructs `SHA256("SPc/StockAutoRefresh/v1" || vtxo_id_bytes ||
  height_u32_be)`, checks result/round linkage and rejects manual-id reuse.
- Offboards: operation payload/result, allocated return address/script,
  input VTXOs and confirmed raw destination transaction. Direct SP additionally
  binds the retained signed checkpoint to those exact transaction bytes.
  Value deltas report user-paid fees separately from raw chain transaction
  fees. They do not claim a replay of the stock dynamic offboard fee quote.

The verifier request/reply schemas are
`spc.lab.ap5.vtxo-proof-request/v1` and `spc.lab.ap5.vtxo-proof/v1`.
Requests contain only raw public VTXO/anchor bytes and expected public facts.
Replies bind their SHA256 hashes, VTXO/anchor identities, full-policy facts and
stock Ark revision `c419159fa54085572f8ca72d2ba1716556a9d3eb`.
Source admission checks the actual Nix input's main/vector source, public
fixtures, Cargo.toml, Cargo.lock and recipe, plus the installed frozen lock.
The lock SHA256 is
`ac24ee3b09c7c2dd8731417672b9d7b072aa6e9e04127b9c96e8ad80ae842833`.
The standalone Nix verifier is now available at
`/nix/store/kyq7kv5wb38353l2abp2iiwy0hf01yq9-ap5-vtxo-verifier-0.1.0`, binary
SHA256 `5330c3ce8eaa65a4b96076badbeae420f654c33c2a611927932fab49bb8357dd`.
Its actual source and six passing unconditional tests are bound in
`/tmp/opencode/AP5-verifier-build-r1/nix-binding.json`. This is offline
conformance, not a live/funded Ark proof or a reason to add a verifier to the
native-only diagnostic bundle.

## Pool queue

`pool_control.py` implements the exact tested r3 DTO, preserving legacy
non-proof replies. Unknown or missing fields, duplicate keys and non-u64 money
reject. Queued/Unknown/
Submitted repeat identical request bytes, path, invocation and operation id.
Effect-bearing states preserve the original proposal/policy/VTXO binding.
Terminal receipts are immutable history. A snapshot always uses a fresh id.
Ordinary `board_psbt` errors after the durable proposal remain unresolved;
absence from the mempool/currently unspent inputs is not generic NoEffect.

### Tested same-wallet retained-board proof action

The interface gap is closed by the producer's r3 source snapshot
`dd9f0c37d7e79474a56132170272fcd6509eb3ebdc501aaa93372e8dd8a4102b`,
tested code `8ec102f55081c58f2ff29aaadb5a78c56c9531a1245a74ffa77b484e077e0e42`.
`pool-control-proof-lineage.json` pins the received contract and the explicitly
NOT-FUNDED public unit example. Producer final evidence is 25 focused tests,
build and clippy; earlier 54 Ark tests are separate, not 79 unique final tests.
Auditor recheck is ongoing per parent. Joined control/grant/ladder and funded
acceptance remain separate requirements.

Request fields are exactly `schema`, `operation_id`, `action: board_proof_v1`
and `original_board_operation_id`. Read and original mutation ids differ;
money fields are forbidden. A queued read repeats identical bytes/path/id.
Terminal states are `proof_available`, `proof_missing`, `proof_unavailable`.
A new read id may request a later retry; the board mutation id never changes.
Every historical reply has null current balances, time, tip, funding txid and
board fields. Only ProofAvailable includes `proof`; typed missing/unavailable
results are inconclusive and never trigger remote proof reconstruction.

The optional new Confirmed field `retained_proof_sha256`, when present, is an
exact 32-byte lowercase hex hash. Legacy absence remains parseable but cannot
satisfy retained-proof acceptance. Terminal receipts cannot be rewritten to
add it. It is forbidden on non-Confirmed replies; `proof` is forbidden except
on a successful proof read.

`board_proof.py` validates envelope schema
`spc.lab.ap5.pool-wallet-board-proof-envelope/v1`, its `canonical_json_hex`,
exact SHA256 and `original` tree. Canonical bytes are compact UTF-8 in the
producer's declared Rust struct field order, including nested request and
board fields; sorting keys or hashing a rendering is not equivalent. The
original schema is `spc.lab.ap5.pool-wallet-board-proof/v1`, with original
mutation/request digest, full VTXO bytes/digest and codec, raw anchor txid/vout,
owner/full Pubkey-policy bytes/server, board facts, historical checked time/tip,
confirmation count, `board.TXID.VOUT` checkpoint and terminal context.

The terminal context schema is
`spc.lab.ap5.pool-wallet-terminal-receipt-context/v1`. Its raw canonical JSON
and SHA256 must match the actual original Confirmed receipt after removing
**only** `retained_proof_sha256`. Every amount, observation, operation and board
field remains exact. That removed hash must equal the envelope hash. This
breaks the self-reference without weakening the historical receipt binding.
The original VTXO and raw anchor then undergo the normal independent verifier
and two-node chain checks; envelope availability alone is not a crypto verdict.

`lineage_complete` must always be false. `recorded_partial` carries at most
256 `spc.lab.ap5.pool-wallet-round-import/v1` facts with stock movement id,
sorted unique input/output IDs, round funding txid, exact offchain fee and
completion time. Movement ids are sorted/unique and every edge must be
reachable from the original through explicit input/output ID links. Conflicts,
unrelated IDs, noncanonical fields, excess capacity and completeness claims
reject. Producer `unavailable` lineage has an empty fact list and its typed
reason; the valid original proof remains usable. No equal-amount matching or
partial descendant history can replace the separate current full-proof set.

The producer retains the public object with fsync/NOREPLACE before publishing
the Confirmed digest and releasing the protective stock lock. AP5 consumes
that same-wallet read interface after spend/refresh/pruning; it no longer
fetches the historical original through remote GetVtxo or requires current
wallet key rows. GetVtxo remains available for independently observed current
float and ordinary child proofs. The original request is preserved in
[POOL-BOARD-PROOF-HANDOFF.md](POOL-BOARD-PROOF-HANDOFF.md).

## Cleanup

`cleanup-inventory`, schema `spc.lab.ap5.cleanup/v2`, carries `inventory`, its
compact sorted-JSON SHA256, `scope`, `exhaustive`, `all_resources_released`,
and `status: passed|failed|inconclusive`.

Inventory contains `active_manifest`, `coverage_errors`, `processes`,
`listeners`, `observed_socket_inodes`, `observed_unix_paths`, `run_tokens`,
`observed_file_inodes`, `file_references`, `locks`, `capture_errors`, and
`unreadable_same_uid_processes`.

The active manifest's mandatory minimum role census is node 2, front 2,
wallet 4, miner 4, pool-ark 2, stock Ark 1, Ark PostgreSQL 1, pool PostgreSQL 2,
Redis 2, and `tx_driver` 2. Planned server/admin/storage, node RPC/P2P/TDP,
front and wallet-status endpoints are mandatory. The public active census is
separately sealed; credential-bearing resource URLs are excluded from it.

The inventory covers controller and actor PID/start-time identities, observed
descendants, probe-node launch records, auxiliary-tool identities/exits,
exact runtime/tool run tokens, planned TCP endpoints, observed TCP/UDP/Unix
socket inodes, run-contained open descriptors/maps/cwd and file locks.
Each tool runs through the actual actor-exec helper with PDEATHSIG, an owned
process group and a run token; interruptions kill/reap its group. Cleanup
does not signal a process merely because a PID or filesystem owner matches.

Zombies remain unreaped, reused PIDs are distinguished by start ticks,
unreadable same-UID processes/capture errors/missing census leave coverage
inconclusive, and residual references/locks/listeners prevent release. TCP
TIME_WAIT entries without owned inodes are not live listeners. Endpoint reuse
prevents a release assertion. Retained inert Unix socket paths are reported;
evidence files and old runs are preserved.

Replay recomputes inventory consistency, mandatory coverage and its digest,
compares the separately sealed active census, and requires every tool intent
to have an exit identity included in cleanup. This is a replay of captured OS
observations, not a claim that an offline consumer can inspect a past kernel.
No live exhaustive-cleanup receipt has been produced for funded AP5 yet.
