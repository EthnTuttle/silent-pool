# Traffic runtime implementation - tested controller packet R8

## QA acceptance follow-up (2026-11-10)

Final source result: PASS. 114 core test instances plus two HTTP/schema tests
passed across source-bound receipts (116 total, plus the separate durability
fault child). The initial full core invocation passed its first 109 instances;
the strict-selector regression then exposed Serde's unit-variant extra-field
behavior. An explicit closed struct decoder fixed it, and the five affected
version tests passed on the corrected source. This is not a claim that the
earlier failed whole invocation returned success. Passing suites were retained
rather than broadly repeated.

Receipts under `/tmp/opencode/`:

- `traffic-runtime-r2-gate-20261110T061303Z/`: HTTP stage/operator/default-off/
  native deadline/debit checks and existing core suites, including default v2
  goldens, maturity/reorg and prior-unknown/accepted PossiblySpent regressions.
- `traffic-runtime-r2-gate-20261110T062156Z/`: all five final version-fixture
  tests, including independently verified final v3 signatures, fee convergence,
  replay/reserved recovery and unknown-input retention. The saved
  `version3-synthetic-fixture.json` is explicitly not native-funded evidence.
- `traffic-runtime-r2-gate-20261110T061815Z/doc.log`: documentation checks.
- `traffic-runtime-r2-gate-20261110T062417Z/`: all-target clippy, formatting,
  CLI/txd builds and immutable `binary-bindings.json`.

The last source change after functional tests is a test-only unit-variant
pattern correction. The diagnostic type-inference failure, real hook-result
mapping failure (`recorded` was incorrectly Unknown), strict-decoder failure
and lint failures remain archived. No assertions were weakened.

Sealed current binary SHA256s:

- spc-lab: `ce96a4c39a14df3c16c309d9101d5e831f48c5deca896659d9e39fb753dfafd1`.
- spc-lab-txd: `26a7839decc3ccf81268a8ca61ac038be00c8ffbe1f44deb94b8b9e401a7016b`.

These binaries include the new T1 constructor, not the separately owned T2
native installer. They must not be presented as the joined funded pipeline.
Final delta/source/schema/evidence handoff:
`/tmp/opencode/traffic-runtime-followup-final-handoff.json`.
The prior passing UI layer is separately collected, with its exact 19 source
rows and patch, in `/tmp/opencode/traffic-runtime-ui-collected-for-parent.json`.

UI-v2's 19-test/clippy/fmt completion receipt remains collected at
`/tmp/opencode/traffic-runtime-r2-gate-20261110T043945Z/receipt.json`. The new
separate follow-up fixes early operator hook admission, adds a closed operator
version-rejection constructor before the existing signer, and binds only the
source-reviewed release bba/f159/4634 exact version row. Constructor selection
is digest-covered in the existing request/journal and immutable across replay.
The existing core prior-unknown/accepted/reorg PossiblySpent rule is unchanged.

Node source proof and actual route-only evidence are preserved. The preliminary
version-after-sign diagnostic is explicitly NOT valid-signature/funding proof.
The unrun external re-sign/replayed-chain candidate was archived and removed;
the production construction gap is addressed instead. Exact QA API and live
case recipe: `docs/acceptance-version-rejection-v1.md`. Fresh native-funded
execution remains with the joined T2/QA runtime; no node build or hidden hash
source is used. Focused source/HTTP/core checks passed as recorded above;
fresh native-funded version-rejection execution is still unclaimed.

## Separate T1 UI provider continuation (2026-11-09)

Final result: PASS, 19 focused tests plus all-target clippy and formatting at
`/tmp/opencode/traffic-runtime-r2-gate-20261110T043945Z/`. This includes one
actor money/read-only test, two scoped provider tests, eight owned-actor channel
tests, seven HTTP/projection tests and one explicit capability-registry test.
The separate final patch/schema handoff is
`/tmp/opencode/traffic-runtime-ui-final-handoff.json`. UI-owner integration maps
`projection.user_id` to its `wallet.identity_id` and copies `payment_wallet`;
the authoritative schema is `spc-t1-payment-wallet/v1`. No UI-owner files were
edited, and this is not a full funded UI/T3/Ark/HTML acceptance claim.

The acceptance layer is frozen and passing at full `57069b95...`, code
`fb07bd4c...`, with its 16-path adapter-only patch separately handed off.
This additive continuation exposes authenticated per-user T1 test-payment
funds through the existing projection and actor channel. Exact schema/path/UI
mapping is `docs/t1-payment-wallet-provider-v1.md`. New reads derive from the
validated live allocation, preserve receive cursors and journal bytes, exclude
reserved/unknown outgoing funds, and leave unproven pending/unconfirmed money
null. No native provider/funding stage means no available UI capability.

All pre-edit rows/manifests are preserved at
`/tmp/opencode/traffic-runtime-ui-v1-before/`. Source/API/owned-process checks
for this new layer passed; the earlier acceptance pass is not relabeled
as a UI-provider pass. The UI owner's files and Ark private-input producer
remain separately owned.

Initial UI gate `/tmp/opencode/traffic-runtime-r2-gate-20261110T043726Z/`
stopped at a test-only partial move of the address before serialization. The
fixture now clones that value. The controller additionally rechecks the sampled
actor incarnation and active/native-provider state after a potentially slow
read; UI handoff explicitly requires expiry of cached browser observations.
The failed receipt is retained and the revised focused gate passed as above.

## Separate acceptance-adapter delta (2026-11-09)

Final result: PASS for 58 focused adapter/context/DEST/access tests and
all-target clippy plus formatting. Receipts are source-bound at
`traffic-runtime-r2-gate-20261110T041707Z` (seven unchanged context tests),
`traffic-runtime-r2-gate-20261110T042209Z` (51 tests including the actual owned
txd Signed/held/retained-coinbase-rollback/withhold fixture), and
`traffic-runtime-r2-gate-20261110T042512Z` (final clippy/fmt). All paths are
under `/tmp/opencode/`. The final post-test change is a test-placement lint
annotation, not runtime behavior. Failure receipts remain preserved. The
acceptance-only 16-path handoff is
`/tmp/opencode/traffic-runtime-acceptance-final-handoff.json`; it excludes
T2's installer and dependency edges. Funded live T3 acceptance remains unclaimed.

The tested J1/T1 packet below remains frozen at final full `e53118dd...`, code
`8b1bbdc9...`; its 706 rows were reverified before this separate delta. The
changed rows/report/manifests are archived at
`/tmp/opencode/traffic-runtime-acceptance-v1-before/`.

The controller-owned T3 gaps are addressed by version-1 private forwarding
attempt/result/watermark records in the existing strict command log, actual
AddConnection-before-write evidence after a durable measured boundary, and an
explicitly opt-in operator-only signed-before-intent hold/cancel or one actual
correlated response loss. Exact schema/API is in
`docs/traffic-acceptance-adapter-v1.md`; normative scope is the dated acceptance
adapter decision and changelog fragment.

The T2 handoff subsequently arrived and all 719 frozen producer rows were
verified. The J1-based `T2-contextual-T1-join.patch` (c73327b3...) is consumed
once; the alternative DEST-based contextual patch is not double-applied.
The separate T1-DEST delta (ef2d624c...) supplies destination guards and tests.
Overlapping actor tests are adapted only to the new private `rpc_forward`
envelope. Context module/tests are exact producer bytes; the CPU-miner test
dependency is dev-only. No native executor/installer is imported into T1.
Prejoin candidate rows/manifests are archived under
`/tmp/opencode/traffic-runtime-acceptance-v2-before/`.

T2's separate native-funded receipt records 105 blocks, 80 outputs and 1809
measured/1810 charged hashes on two actual nodes. That is producer evidence,
not this new acceptance-adapter layer's live-funded acceptance. The previous
waiting-contextual readiness receipt is now historical, superseded by this join.

First focused gate `/tmp/opencode/traffic-runtime-r2-gate-20261110T041707Z/`
passed all-target compilation, command/audit tests, socket/forwarding tests and
all seven unchanged contextual tests. The owned actor fixture failed before
launch because it labeled a coinbase as non-coinbase `CanonicalBlock`; the core
correctly rejected that source/context pairing. The fixture now uses the
existing `BootstrapCoinbase` source. The failure receipt is retained.
Revision 3 also serializes hook cancellation with the forwarding mutex,
rechecks hook vetoes at dispatch and drops completed replay-map entries to
bound memory. Result evidence says `response_suppressed`, not an unproven claim
of actor receipt. The focused checks passed as recorded above.

This layer has its own source-bound focused checks. Tests use
unit/socket and owned-txd-process fixtures only; they do not constitute live
funded T3 acceptance. Actual funded tests remain dependent on contextual T2
and the sealed combined runtime. Prior J1 tests and fresh-profile passes are
historical boundary evidence, not a pass asserted for these new source rows.

## Final J1 / fresh T1 profile result (2026-11-09)

PASS for this joined source's unfunded T1 node/probe/profile/txd boundary.
Atomic J1 validation passed all-target check, 70 tests (including explicitly
enabled immutable actual default/compat reports), clippy and formatting at
`/tmp/opencode/traffic-runtime-r2-gate-20261109T024746Z/`. The fresh controller
build, CLI clippy and formatting passed at
`/tmp/opencode/traffic-runtime-r2-gate-20261109T030038Z/`.

The latter gate freshly executed the canonical r4 producer against the sealed
node for all four cases. `fresh-profile/summary.json` records:

| Case | Actual preflight | Traffic stage | Cleanup |
| --- | --- | --- | --- |
| default | recognized | provisioned | all 14 owned identities gone |
| bark-v31 | recognized | provisioned | all 14 owned identities gone |
| node-b ENV mismatch | failed closed | no driver activation | all 8 gone |
| node-b policy mismatch | failed closed | no driver activation | all 8 gone |

Both positive cases reached the expected `traffic_bootstrap_executor_unavailable`
command outcome. Funding and worker AddConnection counts remained zero. The
nodes, canonical probe and txd were real; storage/front/miner processes were
explicitly labeled inert fixtures. This is not native T2 funding acceptance or
full Nix Ark infrastructure acceptance. `behavior_proven` remains false and the
definitive rejection catalog remains separate and disabled.

Fresh binary and report bindings:

- Controller: `26441b4db640855637a3fea9ca2c5d3258a1e2d4ff1bc26bda93faadb4b52f06`.
- txd: `57920111be725aaa6f0970aab45bd182642c1f4bc374b7db0e5759b66f872afa`.
- Node: `36e2387ada164e2c89b3d20f5d515bf19c33f30fb927e8ca11621eba3658d7d9`.
- Default report: `96d9fa004edfd9a137148ec259aa9ecf1f7f663135d2fa57e4b62cf9e23769ac`.
- Compat report: `e4e398ebc01fd96e0826e28a1ac18761b96d2c7aeca80137fb9b40b42815d1f0`.
- Tested source full: `2f3181c1c2327bad1e31f5398fca6f8f1110f7269b088719a25877c41503fe66`.
- Tested manifest bytes: `cf42604aaf1e0320abbca6378aa015f429bb9339ff57a39230eb9739c1cd36ec`.

### Actual ownership-token binding

The existing traffic-only new-run branch in `engine.rs` hashes the run/owner
tuple into 64 lowercase hexadecimal bytes before constructing the supervisor.
This internal ownership encoding is not an authentication credential exchange.
The exact token is injected into actual child ENV and stored in process records;
attach reuses the recorded process token, and reap checks the actual token and
start identity. Nontraffic legacy ownership contexts keep their prior encoding.

The fresh harness verified both actual node ENV maps against the complete raw
profiles and verified token equality with persisted process records. Canonical
r4 normalization accepted the actual new profiles and rejected detached copies
with `legacy-run:legacy-owner` as `missing or invalid ownership token`. No
observed token was rewritten and no grammar/contract relaxation was made. This
directly covers the colon-token issue reported from the separate Ark infra tree;
that tree's unfunded live run is not relabeled as a passing profile case.

### Narrow integration corrections and evidence

The CLI now supplies its existing actor-exec helper for declared traffic. The
controller supplies canonical probe's required `--source-name` using a truthful
actual-binary hash label. The first fresh run's missing-argument failure remains
at `/tmp/opencode/traffic-runtime-r2-gate-20261109T025638Z/`; it was not discarded.
Pre-revision row/report/manifests are archived under the J1 `*-before` paths.

The final handoff is `/tmp/opencode/traffic-runtime-j1-final-handoff.json`, with
the atomic nine-row J1 patch, separate CLI/engine patches, exact row hashes and
source/log manifests. This report-only finalization does not change tested code.
No additional broad gate was repeated after the passing fresh gate.

## Atomic J1 continuation (2026-11-09)

The previous tested 700-row R8 packet and all gate receipts remain preserved
under `/tmp/opencode/traffic-runtime-j1-v1-before/` and their original receipt
paths. This continuation is a NEW source candidate; the R8 pass below is
historical evidence, not a pass claimed for the new J1 join.

Explicit parent/user authorization activates frozen canonical r4 contract
`754e1e6131955f9e5bbce7ef81b6f9aabaa2f1379ec6409f1c3a7a9e88be91f9` only together
with the exact 20-row table, selected-consumer metadata and full diagnostic-row
preservation. All 616 frozen producer code rows/modes were verified first.
The producer's separately unrun Rust tests are consumed into this runtime's
source-bound gate, not attributed as a separate producer pass. Historical
f4fc3e04 approval is unchanged but does not authorize the new schema. No new
Fable approval is claimed. See the dated atomic J1 activation decision.

The joined tests retain missing/duplicate/wrong-method/unknown-hash/funded-proof
negatives and add explicitly invoked hash-bound tests against the actual
default and bark-v31 reports from node 36e2387a. The three public fixture copies
match their frozen producer hashes. Ownership environments and private paths
from live reports were not copied into fixtures. Current source preserves the
T1 allocation/bootstrap-limit/history-3/OP/access changes and audited core
write algorithms. Fresh-process acceptance must still execute the canonical
probe and compare every actual node's full launch/consensus/genesis binding.
Definitive rejection catalog activation remains separate.

Joined compilation/tests/clippy and any fresh-profile result will be recorded
in `/tmp/opencode/traffic-runtime-j1-*-recipe.json`; until an actual pass receipt
exists this continuation is not accepted on the basis of the prior R8 checks.

## Preserved R8 report

Date: 2026-11-08.
Worktree: `/home/ethan/.local/share/opencode/worktree/ad8886/lab-traffic-runtime`.
Status: **SCOPED COMPILATION / TESTS / FORMATTING / CLIPPY PASS.**
Native T2 is absent in this tree. No funded T2/T3 or Ark readiness is claimed.

## Preserved inputs

The full 949-line plan was read and verified:
`44daaa2baa41a4b93eb46a9e69dfdcf2bba1e0ce73367e27e83a87ab3a6b3307`.
The current primary AGENTS and authority documents were consulted.

Baseline construction remains exactly the recorded recipe in
`2026-11-08-traffic-runtime-baseline.md`: 658 AP2 indexed files, each hash
verified before copying, then exactly 15 verified tjv3 current-primary rows.
The combined 664-row baseline manifest is
`/tmp/opencode/traffic-runtime-layered-baseline.json`, SHA-256
`087e34b7c9e91df34bd5d8a93125f3ac1780173ac642dc44b16257421bf990f7`.
The original parent-merged traffic-contract row was `3a4177bf739f26e0...`.

Before this scope extension's first edit, the entire prior **44-row delta**
was verified and archived under `/tmp/opencode/traffic-runtime-incomplete-r1/`.
It preserves full `f2e2e401c85711e5cb8bd0751370dfbae9b7d1df7095f68f0da9ecb4b2845735`
and code `0b5bcd6326f1dfab371563f672c7b712105ebe9d1310f5cbff17bb02aeaf2f9c`.
The archive is 214,414 bytes, not a full workspace/target copy.

Canonical producer r4 was consumed as a typed source interface, not promoted:
contract `754e1e6131955f9e5bbce7ef81b6f9aabaa2f1379ec6409f1c3a7a9e88be91f9`;
full `71b4ee6f57a16a0451af669f594491a8623b5eea036a558f61139b9a5bca5351`;
code `22fd2fcdbd486191f3935738a1625ec9c5333d1068554046ab5b9f11cd180d70`.
Its two golden fixture rows are cited in the Rust golden tests. No producer
source was copied or edited, and no candidate lineage hash was approved here.

## Additive atomic read API - exact shape

`TrafficJournal::recovery_snapshot(&self) -> RecoverySnapshot` now copies,
under ONE existing journal mutex hold:

- `committed_seq: u64`, `tip: Option<ChainAnchor>`;
- `barrier_active: bool`, `next_send_sequence: u64`;
- `last_broadcast_txid: Option<Txid>`;
- `last_chain_observed_seq: Option<u64>`, `caps: JournalCaps`, `poisoned: bool`;
- `commands: BTreeMap<String, StoredCommand>` containing **all commands**;
- `transactions: BTreeMap<Txid, RecoveryTransaction>` containing **all txs**.

RecoveryTransaction contains `facts: SignedTxFacts`,
`last_send: Option<(u64, u64)>` (send sequence, intent-record sequence),
`reorged_from_block: Option<(BlockHash, u64)>`, `displaced_at_seq: Option<u64>`,
and `invalidated_prior: Option<TxState>`. Finished commands are not filtered.

Both snapshot DTOs are detached and have no Serialize/Deserialize. The existing
tx_facts clone was moved into a private JournalState helper so neither public
lookup nor snapshot relocks the mutex. There is no file replay, mutation,
authority deserializer or borrowed mutable internal state. Existing journal
mutation paths remain authoritative for caps, ownership, maturity and phase.

The 143-line external `traffic-runtime-r2-core-read-api.diff` shows that the
audited journal changes are only those DTOs/accessors and view-helper reuse.
All rollback, proof, admission validation and transition algorithms are intact.
Control v1, journal/history 3, derivation 2 and header bytes are unchanged.

Compatibility detail: the old core's last_broadcast_txid field is copied
faithfully. ReorgResubmit advances each transaction's last_send tuple but does
not replace that legacy field. Pause reports the committed barrier's last_txid
plus last_intent_txid derived from the latest tuple; no transition change was
smuggled into a read API. See the new recovery-snapshot decision and fragment.

## Runtime joins authored

- Native binary remains inside the existing spc-lab-txd package. Bounded
  JSONL requests use per-incarnation wire ids separate from durable command ids;
  duplicate/out-of-order/corrupt messages fail closed. Controller-node RPC
  preserves parsed numeric errors, transport/protocol uncertainty and gate
  closure, through both host HTTP and namespace helper responses.
- Full global allocation reconstruction, sorted cumulative network/user ranges,
  one bootstrap host, encoded driver roots, nested 0700/0600 hygiene, exact
  traffic/ history semantics and explicit driver/node/user routes are present.
- The actor background pump now runs the raw anchored observer and deterministic
  ordinary scheduler. It recovers all states from the new snapshot, including
  finished/reorged transactions; retries use stored bytes and actual persisted
  caps. Deferrals are durable existing CommandBegin/CommandFinished records,
  not zero-success payments. Planned and measured timing/rate evidence is
  separate. Control checkpoints interrupt scans; snapshot reads are serviced
  during scans without dropping queued mutating controls or RPC outcomes.
- Pause restores from barrier_active after reopen, derives last-send metadata
  from committed state, and drains/classifies RPC outcomes before the barrier
  ack. Driver-specific restart uses a fresh pipe/incarnation, re-verifies live
  binary/argv/environment/profile/genesis/facts and immutable actor configs,
  preserves the measured boot-bound epoch, and restores bootstrap debit/window
  metadata. Generic RestartRole remains unsupported.
- Declared raw transactions now have a conditional admission implementation:
  strict consensus roundtrip, witness/profile/size bounds, known available
  same-owner prevouts, minimal P2PKH scriptSig, SIGHASH_ALL/low-S signature
  verification, exact fee and output policy, byte/txid dedup, then the existing
  begin/reserve/signed journal path. A partial old unsigned admission is refused
  without releasing inputs or re-signing newly supplied unbound bytes.
- Raw observation scans genesis inclusively, verifies hashes/merkle/PoW and
  reward limits from decoded prevouts, uses one canonical authority, applies
  proof-bearing lower rollbacks, and discovers receives/change using exact
  source/context/outpoint/locus/value/script checks. Mempool eligibility uses
  complete decoded membership with an anchor recheck and excludes own presence
  from conflicts. Own creator reconfirmation restores its existing cascade.
- Existing CommandLog traffic-strict reservation/reconcile mode is connected
  to native hash_reserve/hash_reconcile/hash_status messages. Actor consumption
  is at most once; duplicates and old incarnations cannot grant another batch.
  Unresolved old-incarnation reservations are reconciled unknown and fully
  charged. No actual hash executes in this packet's default configuration.
- The tracked bootstrap operation releases the global command mutex, keeps its
  durable begin in flight, and finishes from actual completion/failure/cancel.
  Its T1 completion verifier now checks raw canonical data on all actual nodes,
  global allocation destinations and complete output counts, funding-wallet
  ancestry/maturity, fees and the host signer's confirmed distribution ids.
  It records the measured boundary before the first miner AddConnection, then
  activates the schedulers. Phase execution waits outside the command mutex.
- Closed TxTraffic/stage/boundary evidence projections are wired to the existing
  evidence store. Recovery maps, raw transactions, scripts and allocation users
  are not dumped through HTTP/SSE. Existing expanded-access user projection
  excludes these operator events; user payments retain explicit self scope,
  destination validation, origin checks and principal-bound outcome replay.

## Actual boundaries, not workflow holds

- **Native T2 remains absent.** BootstrapExecutor/BootstrapHost and actor-side
  bounded wallet/RPC/hash operations are concrete. Production has no default
  PoW executor and reports that condition. The independent T1 completion path
  no longer contains an unconditional completion-verifier-unavailable branch.
- **Node/probe acceptance remains external.** R4 producer Rust checks and the
  actual four-field node metadata/profile build were pending at handoff. The
  runtime's lineage and every actual-node profile check remain closed.
- The exact definitive rejection table is implemented, but its reviewed
  source/binary/consensus-profile catalog is empty pending real binding input.
  Unbound numeric responses remain Unknown; no unknown hash/version string is
  treated as approval. This is distinct from the ordinary transport classifier.
- Traffic adoption refuses if it cannot recreate a verified actual-node route
  (including an unavailable namespace transport). Live-controller driver
  restart uses the retained typed host/namespace registry. An unobservable old
  bootstrap operation is never restarted with a fresh deadline or debit.

## Checks and tests

All checks used repository-pinned Nix Rust/Cargo 1.93.0, with actual executable
paths and hashes verified before Cargo, `--locked`, the existing full vendor
configuration read-only, and `/tmp/opencode/T1a-target` reused. Every invocation
held nonblocking `flock -n -o`; disk checks ran inside the slot before each
step, preserving the 50 GiB minimum and projected 20 GiB residual requirement.
Initial free space was 59,432,800,256 bytes; the final step ended with
53,875,671,040 bytes. No cache cleanup, pin or dependency change was used.

Passing receipts (under `/tmp/opencode/traffic-runtime-r2-gate-<timestamp>/`;
the inherited log-directory prefix does not identify the source revision -
each receipt records the exact actual candidate hash):

| Check | Result | Receipt timestamp |
|---|---|---|
| cargo check, txd/lab/CLI, all targets | pass | 20261109T013442Z |
| full spc-lab-txd package, including binary and history-3 tests | 90 top-level tests pass; 2 explicitly ignored; spawned fault child also passes | 20261109T013442Z |
| independent offline completion/common-node/half-scan/change fixture | 1 pass | 20261109T013442Z |
| traffic_contract | 42 pass | 20261109T013916Z |
| traffic_history_conformance | 7 pass | 20261109T013916Z |
| traffic_rpc_transport | 1 pass | 20261109T013916Z |
| traffic_runtime profile/stage cases | 3 pass | 20261109T013916Z |
| strict command budget tests | 3 pass | 20261109T013916Z |
| clippy, txd/lab/CLI all targets, -D warnings | pass | 20261109T014613Z |
| pinned-toolchain cargo fmt --check | pass | 20261109T014829Z |
| access_hub_routes / access_projection / access_scope | 3 + 6 + 18 pass | 20261109T014829Z |
| touched actor/admission regressions | 4 + 1 pass | 20261109T014829Z |
| touched profile/budget regressions | 3 + 3 pass | 20261109T014829Z |

This is 174 distinct top-level tests across the listed scopes, plus the
spawned durability-fault child. Focused reruns are not counted as new cases.
The test suites intentionally use explicit offline fixtures; these counts
are not native funding or vanilla Ark interoperability evidence.

Failures were retained and corrected narrowly:

- R3: three actor references corrected from Begun to the existing Began enum;
  no enum/serde change.
- R4: removed ambiguous AsRef on an already borrowed Path in command-log open.
- R5: old manifest-only adoption acceptance test updated to assert the stronger
  actual probe/profile requirement and no manifest/command-log mutation.
- Clippy initially lost a global Cargo --config argument in its nested Cargo
  invocation. The gate now forwards --config after the clippy subcommand;
  offline sources, dependencies and lockfile remain unchanged.
- R6-R7: equivalent fixed clamp / Option predicates and test slice borrowing.
- R8: one formatting-only line wrap; token sequence equals passing R7 clippy.

All before-edit rows, manifests, failure receipts and reports have versioned
small archives. R8's six-row cumulative patch from R2 contains four production
rows and two test rows, with no runtime/bootstrap interface change. The final
source differs from tested R8 only in this report; executable/source rows are
verified before and after checks and again in the final freeze recipe.

Authored tests include recovery empty/binding/detached/concurrent snapshots,
SessionStarted/barrier/capped retry, finished reorged commands, resubmission
send identity, actual closed-journal actor restart/pause metadata, bounded
process framing, global allocation/root confinement, declared valid-signature
admission and invalid-signature no-mutation, authorization duplicate/lost-ack/
old-incarnation consumption, actual HTTP id/code-type/large-body cases, r4
goldens, stage/profile negatives, independent offline raw distribution/common
node proof, inclusive half-scan restart and own-change discovery, and strict
budget write/sync/corruption/arithmetic/no-refund cases.

No native funded run or Nix binary/vendor-FOD build was executed here. Nix
provided the pinned development toolchain for the actual scoped Cargo checks.
Vendor-FOD/package integration remains the serial build owner's check. Baseline
AP2/tjv3 receipts were not substituted for this packet's tests.

## Serial integration

Use the layered-baseline delta, not git diff from b0ab4ca (which contains
inherited packets). Serialize controller, command, namespace/helper CLI,
runtime/config/supervisor/manifest/evidence/access/http and T0 fixture hunks
with their owners. The projection change here is fixture schema only; J1
lineage promotion remains its own frozen producer/table integration.

Recovery-snapshot decision and one-bullet fragment accompany the earlier three
runtime decisions/fragments. No direct SPEC frontmatter edit was made after
baseline layering. No pins changed. The spc-cpu-miner path/lock edge belongs to
the next T2 provider; existing-package txd Nix export is present and vendor-hash
verification still requires the serialized build slot.

AP4 retains pinned-source Nix captaind provisioning and real Ark metadata.
Shared startup order remains storage/nodes/DB hook, traffic preflight,
fronts/Ark/deferred miner processes, txd provisioning, verified native funding,
then the common measured boundary. No hosted endpoint/server package or Ark
protocol fork was added. The sealed live controller/run/ports were untouched.

Final source and evidence bindings are external under
`/tmp/opencode/traffic-runtime-tested-manifest-{full,code,delta}.json`,
`traffic-runtime-tested-log-manifest.json` and `traffic-runtime-tested-recipe.json`.
The T2 consumer should use `/tmp/opencode/traffic-runtime-r8-cumulative-handoff.json`
and its six-row patch from R2, not overwrite its nine-path native provider work
with the whole runtime delta. Source archives R1-R8 and failure logs remain.
