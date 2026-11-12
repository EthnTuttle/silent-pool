# AP5 headless funded-lifecycle controller

Date: 2026-11-10
Status: v2 light follow-up source packet; offline verifier lock reproduced;
compiled verifier and funded runtime remain pending. Python verification is
recorded in the latest light-follow-up section below (prior v2: 35 passed).
Funded AP5 is unexecuted/inconclusive, blocked on the corrected tested joined
bundle and compiled public-proof verifier. No funded Ark or whole-lab
completion is claimed. The initial r1 work below is historical; the v2 update
at the end records the current contract and dependencies.

## Ownership and source reading

Fresh isolated worktree:
`/home/ethan/.local/share/opencode/worktree/ad8886/lab-ark-funded`, detached at
`b0ab4ca34fda07c61a6335d05f598eaacc69678c`. Only new
`testing/lab-ark-funded/**` and this report are owned here. No source overlay,
target copy, primary edit, other-owner edit, commit, subagent or cleanup occurred.

Read current primary AGENTS, README, DESIGN, SPEC frontmatter and the relevant
architecture/wire/roadmap/mailbox/pin requirements. Authority remains DESIGN >
SPEC > architecture/wire, with binding decisions. Read the actual T2/context/
DEST source and receipts, AP3 R1-R6 fixes, wallet command/readiness/persistence
interfaces, and stock c419159 wallet/RPC interfaces before implementation.

Inputs:

- T2 full `9c8f9511774d4e3b77d828cff99ab4958f8c75363245c21717fa298ead6d53ee`,
  code `d33e88fc4cec6fda2763ce6bbddacf84dfac492511a7d5b1ca1654ccc960eda4`;
  `/tmp/opencode/T2-native-freeze-receipt.json` and
  `/tmp/opencode/T2-T1DEST-verification-receipt.json`. Its 105 blocks/80
  allocations and external unconfirmed transfers are correctly scoped as T2
  with inert front/storage/miner actors, not AP5.
- AP3 full `0b0c5fd7d5b52e314a0a1aec818b13eb9d134aafa2ca93734310bed028949468`,
  code `a270582ddfe2e0a20f11de5316b67b913f3a5a6404419f029633bd19b63a01a3`;
  `/tmp/opencode/AP3b-audit-fix-r2-receipts.json`: 11 regressions, 153 agent
  tests and 3 bridge tests, not funded execution.
- Existing immutable self-hosted infrastructure from
  `/tmp/opencode/self-hosted-ark-packet`; node release `bba47522...`, source
  `f1592c93...`, NAR `t6LQ...`, and stock captaind `4cac675b...` unchanged.

## Implemented orchestration

`ap5.py prepare|check-bundle|run` is concrete source with bounded subprocess and
HTTP operations, private immutable event files and a hash-chained event index.
It generates the exact requested 2-node/2-pool/4-identity/4-wallet/4-miner
default, eight background wallets, four native user-payment wallets, 40 H/s
aggregate and 0.2 ordinary tx/s. No smaller profile replaces the default.
Native bootstrap and real workers belong to the joined controller's one budget
authority; the Python harness never hashes candidate blocks or signs a spend.

The controller drives existing `send_user_payment`, actual two-node transaction
confirmation and pool block records, private agent `--wallet-command FILE`,
read-only wallet/operation/reservation SQL, and stock `GetVtxo` plus the stock
Bark decoder. It records real SP proof bytes, checkpoint version 2 before/
after refresh, imported child round anchors, automatic second-renewal inputs
and new actual expiry, and normal/direct-SP offboard transaction evidence.
It does not manually queue the second renewal or invent an order lookup RPC.
The ladder's actual `ark_payout` sent rows must match received VTXO ids;
unattributed direct delivery cannot satisfy mining-derived acceptance.

The confirmed-funding diagnostic is explicitly named `--through
confirmed-funding`. It still requires real pooled confirmations; it does not
reopen the native-bootstrap mining window after completion. The full path
targets 115M sats of observed net float per pool, with real server fee data.
Unknown command outcomes keep their original ids and are not blindly retried.
Read-only SQL cannot create or mutate wallet databases. Protected inspection
run directories, descendants and ancestors reject before directory creation.

## Enforced joined-source admission

`critical-sources.json` freezes 13 source hashes from the approved AP3 R1-R6
and combined T2/context/DEST manifests. These rows were independently checked
against those input manifests. `check_bundle` resolves each of CLI/txd/pool/
agent/miner/scanner through its actual Nix deriver's `src`; all must reference
one joined source. Both the supplied source manifest and actual Nix source
must contain the exact critical files. A new txd beside an old CLI/agent does
not qualify. The joined validation receipt must bind the actual executable
hashes and source manifest and reference passing, hash-checked logs.

Actual negative check against `/nix/store/vm95bb22061a626nd1si133xc9prjz0r-source`
rejected the unsafe old `w52v4f6...` closure before any startup or funding:
`actual Nix build source is stale: bins/spc-agent/src/wallet/audit_regressions.rs`.
Receipt: `/tmp/opencode/AP5-stale-closure-check.json`. Its absent test module
and old critical wallet code cannot be hidden by a new manifest or a filename
containing `joined`.

## Exact outstanding runtime handoff

The serial integration owner supplies the source-bound bundle documented in
`testing/lab-ark-funded/README.md`: the actual joined six application binaries,
source manifest, combined validation/log receipts and the existing immutable
node/server/storage/probe dependencies. Required CLI flags include
`--wallet-agent-bin`, `--txd` and `--traffic-probe` in the same real runtime.
Both fresh wallet registration and verified native funding must precede user
miner AddConnection. No old AP3-only or T2-inert startup is substituted.

Two concrete source gaps were reported to the parent:

1. The inspected host runtime emits `--no-listen` for both nodes. Default AP5
   needs leased host peer listen/connect groups before canonical launch-profile
   binding, as the successful T2 diagnostic supplied through custom adapters.
2. Pool `BoardingLoop` scans only height-indexed coinbase outputs. It does not
   board native transfers to the ordinary published pool-wallet address. No
   live pool-wallet command/status API is exposed, and `RestartRole` explicitly
   returns unimplemented. The harness therefore requires a real same-wallet
   client executor (contract in README), using the live role's writer or an
   implemented owned exclusive handoff. It does not add a second signer,
   silently reopen the wallet concurrently, or mutate ledger balances.

The pinned stock Bark CLI is needed only as a public proof decoder; its
`dev vtxo decode` path does not open or create a wallet. The source-bound pool
executor and decoder must be actual artifacts before full execution. This is
an executable-interface dependency, not a workflow approval hold.

## Verification / scope

Initial ten Python tests passed, covering exact default census/budgets,
protected roots, no adoption/truncation, source mismatch rejection, unknown
RPC results, exact amounts, fresh registration leases, no resubmission after
ambiguous native sends, and stock outpoint/public-address encoding. Additional
read-only database and wallet-role-index regressions were then added for the
final source check. These are harness contract tests, not funded evidence.

Initial r1 final source check: **12 passed, zero failed**, with ResourceWarning promoted
to errors and no resource warnings in the log. The intermediate r2 check
identified that a Python SQLite connection context manages transactions but
does not close the connection; explicit `contextlib.closing` corrected both
the evidence reader and test setup. The earlier log is preserved. Final log
and source-bound receipt: `/tmp/opencode/AP5-harness-tests-r3.log` and
`/tmp/opencode/AP5-harness-tests-r3-receipt.json`.

The actual `prepare` CLI completed at
`/tmp/opencode/AP5-controller-prepared-r1`: private generated default scenario,
hash-chained artifact index, and `prepared-not-executed` result. It launched
no services and reported no funded success. Stock VTXO decoding is delegated
to the pinned Bark CLI, not reimplemented in Python; automatic-renewal output
anchors must match the actual submitted round ids before they are offboarded.

No Rust or Nix build was attempted, no joined bundle was supplied to this
packet, and no funded runtime was started. The inspection runs
`ark-inspection-20261109T025651Z/run` and `live-lab-20261108T231027Z/run`, prior
T2/Ark evidence, pinned binaries and all other owners' sources remain intact.
The final harness result keeps whole-lab completion false until independent
T3/T4b/ladder oracle evidence closes the integrated run.

## V2 source packet and current verification

The initial seven-file, 72,164-byte source freeze is preserved at
`/tmp/opencode/AP5-orchestration-r1-preserved`. The new contract is
`testing/lab-ark-funded/CONTRACT-v2.md`; source packet, tests and handoff live at
`/tmp/opencode/AP5-orchestration-v2-r1`.

Implemented additions:

- Closed pool-result parsing, exact JSON u64/null handling, identical-request
  polling for Queued/Unknown/Submitted, immutable proposal/policy/VTXO identity,
  historical terminal replies and separate fresh-id snapshot observations.
- Raw Bitcoin transaction/block, txid/wtxid, encoded-target PoW, Merkle/witness,
  input-value/claimed-height maturity and fee replay. Canonical collection uses
  both actual nodes; complete ancestry/contextual consensus stays with T3/T4b.
- Public-only stock full VTXO signature/policy verifier source and Nix recipe.
  It checks full Pubkey policy equality and stock `validate(anchor)`, rather
  than treating CLI decode output as cryptographic proof. It is uncompiled;
  no Cargo.lock or actual crypto-vector execution is claimed.
- Component-specific receive/refresh/renewal/offboard checks, raw checkpoint
  serialization, operation/input/output/result binding, actual fee deltas and
  server/mailbox/fee-policy observations. Historical original-board proof is
  distinct from current float. Confirmed float requires a board component.
- Result-v2, component-verdict-v2 and cleanup-v2, sealed event replay and
  mandatory component/resource census. Empty, truncated or contradictory
  inventories cannot claim release; unreadable coverage remains inconclusive.
  Controller, tools, probes, observed children, exact run tokens, planned
  endpoints, socket inodes, file references and locks are inventoried. Tool
  helpers use actor-exec, process groups and run-token ownership.
- Full admission additionally checks corrected tested pool-control/aggregate
  policy files in the actual joined Nix source, passing check logs, the pool
  executor's joined source, and the verifier source/recipe/dependency lock.

Latest lightweight check: **35 passed, zero failed**, ResourceWarning promoted
to errors. Exact command/interpreter hash and tested code hashes are in
`/tmp/opencode/AP5-orchestration-v2-r1/test-receipt.json`; full output is
`tests.log` in that packet. Consumer tests include malformed/changed money and
operation/VTXO bindings, missing crypto proofs, foreign provider/policy facts,
component relabelling, absent tx-driver census, omitted actors/listeners,
zombie/reference/lock/denial contradictions, altered event seals/artifacts,
truncated streams, and partial-archive replay remaining inconclusive. Stubbed
post-verifier/source-admission fixtures are labelled in the tests; they do not
count as real Nix admission or cryptographic verification.

Actual v2 `prepare` completed at `/tmp/opencode/AP5-controller-prepared-v2-r1`.
Its sealed terminal is `prepared-not-executed`. No services or funding were
started. The source packet retains the prepare seal and its verification facts.

### Precise remaining dependencies

1. A source-verified joined CLI/txd/pool/agent/miner/scanner bundle, with the
   final corrected tested pool-control and aggregate policies. The old
   `w52v4f6...`, `55a488...` pool source and `415d21fa...` aggregate source remain
   inadmissible. The r1 pool audit repairs are not assumed to be complete.
2. A reviewed frozen Cargo.lock and compiled source-bound Nix public-proof
   verifier, with actual valid/invalid stock crypto-vector checks. Heavy-build
   priority remains with the parent and pool-control owner; none was acquired.
3. Final pool-producer facts for retrieving the original full board proof,
   public owner-policy binding and descendant history after maintenance or
   pruning. Current code uses stock GetVtxo and wallet public key registries;
   absence remains unknown. Ordinary post-checkpoint board errors preserve the
   original unresolved operation and reservation.
4. A fresh bounded native-funded default run, followed by independent QA of
   raw proofs, resource cleanup and the separately owned aggregate/T3/T4b
   evidence. V2 replay is not live evidence until that run exists.

Current actual AP5 component verdict: **inconclusive - not executed**. The
controller computes a concrete passed/failed/inconclusive component verdict
when run evidence exists; it no longer ends at an awaiting-oracles placeholder.
Whole-lab acceptance remains external. No new Rust/Nix build, primary or
other-owner write, protected-run action, commit, push or subagent occurred.

## Light follow-up: frozen lock, crypto-test source and exact lineage

Current packet: `/tmp/opencode/AP5-orchestration-v2-r2`. The previous 19-file,
187,079-byte/35-test packet remains preserved. Component/result/cleanup schemas
remain v2; CONTRACT-v2.md now also specifies the strict joined feature/UI gates.

### Reproducible lock completed without compilation

Copied the dependency closure from the current project's already locked
Cargo.lock, SHA256 `307a71fe5d279320a74ad06310d0bac79d9c7b4d4cb1a0b14af7e0940553d0c5`.
The standalone verifier pins ark-lib **0.7.1** at the existing c419159 revision,
bitcoin 0.32.102, anyhow 1.0.104, hex 0.4.3, serde 1.0.229 and serde_json 1.0.154.
It does not select new registry versions or change project upstream pins.

`lock_seed.py` copied the exact 184-package conservative closure. A private
offline Cargo home exposed only those packages through read-only links to the
existing vendor cache. Nix Cargo 1.96.1 metadata normalized it to 91 packages,
including the verifier; the audit proves all source/version/checksum identities
and dependency edges came from the parent lock. A second fresh extraction and
offline normalization produced an identical lock. Actual `cargo metadata
--offline --locked` passed, with no target directory created.

Final Cargo.lock SHA256:
`ac24ee3b09c7c2dd8731417672b9d7b072aa6e9e04127b9c96e8ad80ae842833`.
Receipt, commands, complete metadata and frozen parent/vendor input bindings:
`/tmp/opencode/AP5-verifier-lock-r1/receipt.json`. Available target/store space
was checked (about 88 GiB); no heavy-build slot, compilation or Rust tests ran.

### Prepared public crypto tests; recorded live corpus still missing

Extracted only six full VTXO proofs and two anchor transactions from stock
c419159's frozen `VTXO_VECTORS` literals. The exact Git object, vendored and
self-hosted vector/validation source files were compared. Public bytes and
their hashes are frozen in `verifier/fixtures/stock-public.json`; private-key
fixture fields and the vector generator are excluded.

Five Rust tests call the production verifier for valid board/round/arkoor
proofs; a cryptographically valid same-U HTLC with wrong full policy; foreign
owner/server; first and terminal input-signature corruption that must still
decode and pass unsigned structure checks; and wrong anchors/truncation.
The production verifier now explicitly binds anchor txid before full stock
signature validation. Rustfmt parsing/formatting passes; the tests are source
only and have not been compiled or executed. Nix doCheck will run them in the
parent-assigned later build slot.

The inspected project prototype fixture is a derivation vector, and AP3a
explicitly contains synthetic receipt IDs, not full proofs. No preserved live
stock receipt with both original full proof and raw anchor was located in the
supplied inputs. That corpus needs an exact public artifact path; it is not
replaced with a decoder claim, synthetic ledger record or new service run.

### Six exact source-lineage amendments and stricter joined gate

Rechecked all six parent-approved hashes against primary bytes and copied the
two producer manifests into `/tmp/opencode/AP5-source-lineage-r1`.
The receipt SHA256 is
`d8f877b120a8105475d3c7081424ce3fb1a5db18790fc17d98cdd89fc4e75f61`.
T1 actor/admit/builder use full snapshot `6740857f...`; wallet
live_receive/maintenance/mod use `e97ade12...`. Only those six critical policy
rows changed. All other seven guards, including R1-R6 audit/round/scan, remain.
The known old w52 source still fails admission.

`joined_checks.py` now requires actual explicit Ark-feature agent/pool test
and build invocations with executed test logs and admitted-binary bindings.
It also requires the compiled spc-lab UI, browser tests against that binary,
and exact original base/http/DOM asset hashes. The parent's current merged
eight-packet gate is explicitly not passing: its 48 default-feature agent
tests are not AP3 wallet coverage, browser failures await missing-base repair,
and its Nix bundle skeleton is unbuilt. No broad merged success is inferred.

The exact missing read-only same-wallet original-board proof request is
`POOL-BOARD-PROOF-HANDOFF.md`, sent through the parent for the pool-control
owner. The main board mutation remains same-ID. No post-pruning remote API or
producer reply shape is guessed before the tested interface handoff.

Latest verification: **41 Python tests passed**, zero failures, including the
default-feature/partial-UI rejection, unchanged old-w52 rejection, exact
six-row lineage, frozen-lock drift rejection and public-corpus census checks.
Exact code/interpreter hashes and full output are in
`/tmp/opencode/AP5-orchestration-v2-r2/test-receipt.json` and `tests.log`.
Nix recipe parsing and Rustfmt also passed; neither compiles Rust. The five
Rust crypto tests remain unexecuted. Funded AP5 and whole-lab acceptance remain
inconclusive. The report/source packet records no heavy build, protected-run
action, subagent, commit, push, garbage collection or project pin change.

## Signed synthetic-conformance follow-up: no circular build dependency

Packet: `/tmp/opencode/AP5-orchestration-v2-r3`. This explicitly supersedes
the r2 handoff's ambiguous listing of the recorded live corpus among blockers.
**Live corpus availability is not a prerequisite for compiling the verifier
or running any mandatory offline crypto test.** The only pending execution
permission is the parent's later compilation/test turn, with its existing
topic cache, `CARGO_INCREMENTAL=0`, frozen lock and normal shared build guard.

Added a dedicated unconditional signed-board positive using the embedded
c419159 stock vector. It checks canonical serialization, nonempty signed
transitions, exact frozen owner/server, full Pubkey policy, amount, VTXO and
anchor identities, raw hashes, and the production full signature validator.
The existing valid round/arkoor positives and signature/same-U wrong-policy/
foreign-server/anchor negatives remain mandatory. There are now six prepared
Rust tests, none ignored, environment-skipped or dependent on a live archive.

The corpus metadata explicitly labels `synthetic_signed_conformance`. All six
full VTXOs and two anchor byte strings, their hashes, the upstream frozen
vectors and the Cargo.lock remain unchanged. Fresh extraction from the exact
pinned Git object reproduced the updated JSON byte-for-byte. Its synthetic
anchors are not represented as funded chain evidence.

The two relevant Python provenance/lock tests passed again; the prior full
41-test result is retained separately. Rustfmt passed. No Rust compilation or
crypto-test execution has occurred. Exact source/check receipts are in the
r3 packet. The first actual AP5 board will supply a retained live full proof
through the producer's forthcoming exact read-only same-wallet interface;
that proof must validate before funded acceptance. An archived live corpus
can add optional coverage later. No schema is guessed while the active
pool-control owner completes that handoff, and the board mutation keeps its
original operation id.

The parent-verified r2 CONTRACT-v2 hash
`4f9e6da9298331d88930d68c120ec889a65cf87548ec8f4c246634864c7a7290`
remains preserved. The r3 handoff supplies the clarified contract's new hash
and records that predecessor for QA parser lineage; result/component/cleanup
schemas remain v2. Funded AP5 remains unexecuted/inconclusive.

## Tested pool-control r3 proof interface consumed

Packet: `/tmp/opencode/AP5-orchestration-v2-r4`. The source/interface gap is
closed by the exact producer handoff at
`/tmp/opencode/pool-wallet-control-packet-r3/pool-wallet-control-proof-handoff-r3.json`.
Producer full source is
`dd9f0c37d7e79474a56132170272fcd6509eb3ebdc501aaa93372e8dd8a4102b`;
tested code is
`8ec102f55081c58f2ff29aaadb5a78c56c9531a1245a74ffa77b484e077e0e42`.
The parent verified the producer's source/artifact seals and 25 final focused
tests, clippy and build. Its 54 earlier Ark tests preceded the proof follow-up;
they are not added to claim 79 unique final tests. Auditor recheck remains
ongoing per parent. No funded producer or AP5 run is inferred.

`pool_control.py` now consumes `board_proof_v1` with a separate read id and
unchanged original board id, forbidding money fields on the read. Queued reads
reuse exact request bytes/path/id. ProofMissing/ProofUnavailable are historical
terminal results; a later read can use a new correlation without resubmitting
the board. Historical read balances/tip/time/effect fields must all be null.
The optional Confirmed digest remains compatible with legacy absence, but a
legacy receipt cannot satisfy retained-proof acceptance or be rewritten later.

New `board_proof.py` validates exact bounded canonical JSON/hex/digests, declared
Rust field ordering, original mutation/request identity, full public proof and
anchor references, full Pubkey-policy bytes, server, board values, historical
confirmation/checkpoint facts, and terminal context. The context must equal
the original Confirmed receipt with only its self-referential retained digest
removed; no amount, observation or operation field can be dropped or changed.

The controller's historical board collector now uses retained same-wallet
bytes and the original pre-board Ark policy observation. It performs no remote
GetVtxo lookup for the original and imposes no current-wallet-key-row or
currently-spendable-original requirement. The full VTXO still goes through the
independent stock cryptographic verifier, and the retained anchor must match
the actual two-node raw chain evidence. Missing proof remains inconclusive.

Archive replay verifies the request/reply state machines and binds embedded
component copies to the actual separately sealed original mutation, Confirmed
receipt and read request/result events. Partial movement lineage is limited
to 256 sorted, unique and explicitly reachable stock-ID edges. Completeness,
conflicts, noncanonical or unrelated edges reject. Unavailable lineage can
coexist with a valid original proof. Current float always requires its separate
fresh observation and distinct full-proof set; equal amounts or partial
movement history are not descendant equivalence.

The exact public producer example is preserved under an explicit
`unit-NOT-FUNDED` filename. Its canonical envelope is used only for DTO/binding
fixtures; it lacks the independent canonical chain/input evidence needed for
funded acceptance. Frozen contract inputs and their receipt are at
`/tmp/opencode/AP5-pool-control-r3-inputs`, referenced by
`pool-control-proof-lineage.json`.

Verification: **54 AP5 Python tests passed** (41 prior plus 13 new retained-proof
tests), including canonical/terminal digest mutations, read-vs-mutation ids,
same-read polling/new-read retry, pruning without remote lookup, partial-lineage
capacity/conflicts/completeness/reachability and sealed archive contradictions.
The former 12/41-test archives and all later freezes remain intact. The six
offline Rust crypto tests remain unexecuted; no compilation was attempted.

Remaining execution dependencies are the parent-assigned verifier build turn
and the serially joined corrected control/grant/ladder source with explicit
Ark-feature and complete compiled UI coverage. The primary owns the next heavy
gate. Source API availability does not constitute funded AP5 readiness, and
funded/whole-lab verdicts remain inconclusive pending actual live proof and QA.

## Admission hold: dd9r3 restart incompatibility

Parent update: the same auditor closed maintenance/queue/sync/historical-proof
findings but found a new startup R1. Stock Bark leaves a 0644 LOCK after first
start, and the root private-entry check rejects it on restart. The producer is
fixing narrow restart compatibility and adding a regression. Consequently,
`dd9f0c37...` is not final funded/recovery-approved source.

`common.py` now holds pool-control source admission with an empty exact-source
allowlist, including diagnostic profiles that start the same pool role. It can
be populated only when the corrected tested source and the
same-auditor recheck arrive; changing a supplied hash or asserting passing tests
does not clear this hold. This is an admission invariant, not an approval UI.
The historical r3 proof schema, source-lineage file, CONTRACT-v2 and 54-test
consumer archive remain unchanged. No source is relabelled as a new producer
version. No runtime, Rust/Nix build or lock/priority polling follows this note.

## Source-applicable scope correction, actual bundle checks and live attempts

The previous admission-hold receipt is preserved as historical and explicitly
mis-scoped for a no-control diagnostic. The actual g105 Nix source contains no
new wallet-control module or client; sharing the pool role does not make dd9's
R1 implementation applicable. `check_bundle` now determines the class from
the real common Nix src, complete pool file census, known module/identifier
presence and installed client. Control-bearing code requires a matching tested
source policy in every scope. No-control diagnostic source rejects an unrelated
control policy and requires its own exact limited profile/evidence.

The draft closed schema is in DIAGNOSTIC-SOURCE-POLICY.md. The actual supplied
serial-owner `pool-source-identity/v1` and prose scope are also supported by
`received_profile.py`, which verifies their real rows/deriver and all 14 log
artifacts/source snapshots, not a supplementary would-pass assertion. It
recomputes the 296 Rust (including 162 Ark-agent) + 36 Node gate and separate
15 compiled-browser cases, checks UI assets against the actual source, and
requires all six Nix app selections and Ark build features. The known legacy
maintenance path is explicitly a bounded observation, not audited funded Ark.

The parent bundle SHA256 is
`f2c96b21ccb85300b76ba93aff218fa39861dbda531c0e424a8cb5a71e7171af`.
Actual package: `/nix/store/g105903s5z1izz6wwhbm3579madx4xbh-spc-lab-ark-tools-0.1.0`;
source: `/nix/store/wr1ix2s3hnhjhhmi2dv7v5cfd0mb28gc-source`.
Canonical `ap5.py check-bundle --through confirmed-funding` passed at
`/tmp/opencode/AP5-real-joined-check-r2`, `-r3`, and `-r4` as the concrete
harness invocation fixes were applied. Check r1's schema mismatch is retained.
Neither the parent bundle nor its source hashes were changed.

Executed fresh bounded native diagnostic attempts:

1. `/tmp/opencode/AP5-native-confirmed-live-r1`: failed at canonical probe
   launch because the immutable probe entrypoint is a shell wrapper. The
   harness now supplies the exact pinned native Python interpreter and same-
   package probe.py through the existing traffic-probe-arg interface. The
   supervisor's exec-target check is retained.
2. `/tmp/opencode/AP5-native-confirmed-live-r2`: probe passed its read-only
   surface checks; real roles started. Native bootstrap was durably Running,
   but runtime execute_phases required immediate Succeeded and stopped the run.
   Original `ap5-native-bootstrap` finished Cancelled / bootstrap cancelled.
3. `/tmp/opencode/AP5-native-confirmed-live-r3`: a harness-only attempt to
   declare no auto-executed phases and submit the authenticated native command
   instead failed before actor launch: traffic_bootstrap_route_missing. The
   engine requires that route in scenario.phases; the declaration was restored.

The exact source contradiction and minimal same-ID wait fix are in
RUNTIME-BOOTSTRAP-BLOCKER.md: native execution returns Running at
engine/traffic_runtime.rs:1275; runtime.rs:757/840 rejects it. Its intended wait
is misplaced in the duration StopRun branch at runtime.rs:582-587. The producer
must move that wait into execute_phases and test the Runtime path. No source
outside this owner's harness/report was edited and no new compile was run.

No user-payment command ran. All recorded processes in the three attempts
(12 / 41 / 1) are gone and all recorded listeners released, with no observed
run-file references or locks. Four unreadable same-UID processes keep global
exhaustive cleanup inconclusive; this was not relabelled passed. Reap errors
are preserved: absent preflight traffic capability in attempt 1 and stopped-
node launch-profile ownership mismatch in attempt 2. All runs, logs, command
journals and event seals remain retained. The native-only verdict is now
separate (`spc.lab.ap5.native-funding-verdict/v1`); diagnostic scope cannot
invoke wallet-control/lifecycle APIs or full AP5 verdict aggregation.

Latest Python suite: **69 passed**, including canonical check_bundle fixtures
for valid exact no-control source, old agent rejection, control-bearing
diagnostic rejection, unrelated policy/root/census failures, scope limits and
same-ID asynchronous journal observation. Fixtures fake only Nix metadata and
a source mount while exercising canonical checks; actual bundle admission is
recorded separately above. Source/check evidence is retained under
`/tmp/opencode/AP5-source-scope-correction-r1`.

## R4 control source-layer approval received

Exact delivery `d00299c259aaa5e8098542f48f5c3c90c66737be3f965f5d65e5fd2a763914b2`,
tested code `f764900419db45901f558c367b5485938ed1b20b5ee5abdc039e3b66c230ae0c`,
same-auditor bounded approval SHA256
`956ba9ab524b8946b5d367c7e814abefafd75cfd523fa5bcae129cd9155e5d0b`, and
the four exact control/proof/client rows are now bound in admission.
`pool-control-r4-admission.json` preserves the artifact references. The 28
producer tests cover actual stock Fcntl LOCK creation/restart and cross-process
exclusion; no live restart, grant-union or funded approval is inferred.
dd9 and 55a remain explicitly denied when their code is present. R4 approval
does not allow attaching an unrelated R4 manifest to the pre-control g105
bundle. Actual joined Nix source/rows/features remain mandatory. The six Rust
verifier tests and its build turn remain pending; no compiler ran under the
LADDER reservation. Full Ark/native-audit/aggregate claims remain unmade.

All three actual event seals were independently replayed by verify_archive.py
after execution. Replay agrees: native-funding diagnostic inconclusive,
native_funding_confirmed false, whole_lab_complete false. The replay receipts
are `live-r1-replayed.json`, `live-r2-replayed.json`, and `live-r3-replayed.json`
under `/tmp/opencode/AP5-source-scope-correction-r1`.

## Probe tuple and public-only unreadable-PID handoff

Packet: `/tmp/opencode/AP5-probe-and-pid-handoff-r1`. No runtime, read-only
probe, native node test, compiler or RPC was started for this follow-up. The
69-test packet and three canonical/live attempt receipts remain immutable;
the required bootstrap phase stays declared pending the primary owner's
source-tested same-ID Runtime wait fix and corrected bound executable.

The successful r2 invocation is frozen in `probe-invocation.json` (hash
`d01b43be03626bd6e7750c126919650df306b59de82d07b409ff1e2179af017c`):
wrapper `ark-rpc-probe` SHA256 `3d43a195...`, actual native Python3.14 SHA256
`5c4a05f6...`, same-package probe.py SHA256 `833288d7...`, one script prefix,
and runtime-added `node-probe`. All 14 installed probe source hashes match the
retained r2 manifest. `node-probe --launch-profile` invokes the strict
normalizer internally; the separate ark-lab-profile-check wrapper targets
lifecycle_fixture.py and is not the runtime adapter.

The existing Nix launcher has no traffic-probe flags. The minimal launcher
and flake wiring proposal is `probe-launcher-correction.patch`, which passed
read-only `git apply --check` against the recorded Nix source. It was not
applied to primary. The assembler must add the explicit tuple to its bundle;
canonical admission now refuses to execute unlisted Python/script targets.
Runtime emits the same tuple and archive replay checks bundle/event/argv
equality. This is a metadata/argument binding correction, not a new probe or
an exec-target exception. Detailed hashes, observed argv, baseline references
and Nix/assembler instructions are in PROBE-INVOCATION-HANDOFF.md.

The retained probe's contract is `754e1e61...`, comparison profile
`0d36f9e3...`, helper `70d775b3...`, exit 0/reaped. The surface-compatible,
funded-unproven scope is preserved, including its consumer-model failure for
getmempoolinfo.maxdatacarriersize (null versus integer). It is not upgraded
to funded/native-audit acceptance and was not rerun.

Public PID classification read only stat/comm/UID status and namespace
symlink identities plus the public PID/role subset of recorded/protected
censuses. No environ, cmdline, descriptors, memory, keys or wallet data were
read; no process was signalled. Stable current identities, UID 1000:

| PID | Start ticks | comm / current role indication |
| --- | --- | --- |
| 2922 | 18362 | (sd-pam), user-session PAM helper |
| 123843 | 664799 | gpg-agent, user OpenPGP agent |
| 130255 | 753908 | ssh-agent, user SSH agent |
| 1286798 | 3888772 | sd_espeak-ng-mb, already-existing zombie speech backend |

Each current identity predates all three recorded run-controller starts
(60198619, 60220851, 60246962 ticks) on a consistent boot. Current ancestry
leads to user systemd, gnome-keyring or speech-dispatch services. There are no
exact PID/start matches in the three recorded AP5-owned or two protected-run
censuses. PID/user namespaces match the current collector for the zombie;
namespace identity is unavailable for the three nondumpable helpers, and the
zombie's network namespace is unavailable. These are current public facts:
original denial records contain bare PIDs, not historical start times, and
comm/ancestry does not prove historical descriptor ownership.

Recorded-owned cleanup remains all-gone/endpoints-released. **Global unreadable
coverage remains INCONCLUSIVE**, with unknown ownership beyond public UID and
census comparisons. `cleanup.py` and its acceptance criteria were not changed.
The classified metadata and exact baseline hashes are in pid-classification.json.

Verification: 70 Python consumer tests passed, including missing/unbound probe
program/script/source and duplicate-subcommand-prefix negatives. No Rust or
probe tests ran. R4 source-layer approval remains scoped to d002/f764 and its
exact review binding; no full campaign, funded or cleanup-pass claim is made.

## Assigned verifier build completed; corrected-runtime diagnostic executed

Evidence packet: `/tmp/opencode/AP5-verifier-build-r1`. The parent assigned
session `ses_e429358c5ffeV7N1lyDn76dAVl` the standalone verifier turn. Every
Cargo/Nix invocation held the one nonblocking `flock -n -o` build lock, checked
that matching ticket and the existing `/home/ethan/code/silent-pool/target`,
and rechecked at least 50 GiB available plus a projected 20 GiB residual.
`CARGO_INCREMENTAL=0`, pinned Rust 1.93, offline vendor inputs and unchanged
lock `ac24ee3b...` were used. No cold Cargo target, GC, node/server build or
upstream pin/lock change was introduced. The slot is now released; the parent
retains responsibility for ticket reassignment.

The first real verifier compile executed all six tests: five passed and one
found a test expectation error. Stock round2 is an HTLC, so production full
Pubkey-policy equality correctly rejected it. Its unchanged signed bytes now
remain covered as a full-signature-valid/same-U-wrong-policy negative together
with the arkoor HTLC fixture. Production validation was not weakened and no
fixture was skipped. A formatting-only intermediate failure is also preserved.

Final Cargo gate: format, **6 passed / 0 failed / 0 ignored**, strict Clippy
all-targets and actual executable build, all green. Before/after source hashes,
commands, logs and disk/ticket receipts exist for each step. Debug artifact
SHA256: `1aa46f5a552b5e7f5dc635e6bb037aab4c13ddd806811dee12828fa6e7b1285b`.

The verifier-only Nix source was measured before ingestion: **9 files, 59,084
bytes**, excluding target/.git/generated caches. Standard Cargo.lock import
worked offline without a fake hash or dependency update. Nix built only the
verifier and its dependency preparation derivations. Nix doCheck executed the
same six tests successfully; these are six unique cases, not twelve unique
tests. Installed CLI checks also accepted the signed-board input and rejected
the foreign-server input, verifying the real stdin/stdout protocol.

- Package: `/nix/store/kyq7kv5wb38353l2abp2iiwy0hf01yq9-ap5-vtxo-verifier-0.1.0`
- Binary SHA256: `5330c3ce8eaa65a4b96076badbeae420f654c33c2a611927932fab49bb8357dd`
- Deriver: `/nix/store/h1jyr4k5sqqph79q7958z49ysfmnazci-ap5-vtxo-verifier-0.1.0.drv`
- Actual source: `/nix/store/3grmjmz22whyp9armd4lqpy28idr5346-source`
- Installed Cargo.lock SHA256: `ac24ee3b09c7c2dd8731417672b9d7b072aa6e9e04127b9c96e8ad80ae842833`

`nix-binding.json` verifies every frozen source row against the actual deriver
input; `nix-r1-build-full.log` records six passing release tests, and
`cli-check-receipt.json` binds the installed binary and public request/replies.
Documentation was updated after these builds; executable source remains the
tested source. The live diagnostic did not wait for or depend on this verifier.

### Actual corrected-runtime attempt and new P2P blocker

Launched `/tmp/opencode/AP5-corrected-native-live-r1` immediately from the
immutable AP5-probe-and-pid-handoff-r1 harness and parent bundle
`4d3778e8d4c65cffa95af483828fe1013a1d4307da32b3469801ce1624f036fd`, with
the declared bootstrap phase and 7200-second bound. The bound harness/runtime
source was not edited while running. The new `52e45eab...` CLI correctly waited
on the original asynchronous bootstrap operation and admitted the canonical
probe. The full declared actor process roster was observed.

Native authority produced one real block on A, but both nodes had zero peers:
A height 1 (`5445b184...93ce`), B genesis. Recorded operational node argvs both
contained `--no-listen --no-seeds --no-discover --max-outbound 1` and no explicit
connection. The immutable Runtime source still renders that isolation at
runtime.rs:1726. Native bootstrap submits to A and requires common canonical
observation on both nodes before funding credit, so it could not advance.
This is separate from the now-fixed asynchronous phase handling. Exact source,
observations and minimal producer correction are in NETWORK-BOOTSTRAP-BLOCKER.md.

Stopped through the normal authenticated operator `stop_run`, which returned
Succeeded. The original `ap5-native-bootstrap` reached Cancelled / bootstrap
cancelled; no new bootstrap id, deadline reset, changed live argv, manual
cross-node block injection, generate RPC, second miner or balance credit was
used. Observed budget: one 40 H/s start, three reserved/three measured attempts,
zero unknown reconciliations. Native-audit correctness is not inferred from
these observations. **No user-payment command or four-transfer confirmation
proof was reached.** The native-only diagnostic verdict is inconclusive.

All 68 recorded process identities are gone and recorded listeners released;
no run-file references or locks were found. The same four preexisting
unreadable same-UID processes leave global cleanup INCONCLUSIVE. The post-stop
reap failure is preserved. The live event seal, full command journal/datadirs,
chain/argv observations, normal stop intent/reply and terminal outcomes remain
retained. There is no full Ark, audited native, Rule B or whole-lab claim.

## Read-only reap diagnosis and immutable verifier admission

Packet: `/tmp/opencode/AP5-verifier-admission-and-reap-r1`. The primary owns
the operational P2P fix/build turn. No new peer-ready field names or graph
receipt are guessed before the producer supplies that contract; existing
guards remain checked and the A1/B0/no-peer run stays a failed network case.
The 68-process cleanup record, four unreadable PIDs and three measured native
hash attempts are preserved without a funding/native/Ark-positive relabel.

Exact post-stop reaper stderr was recovered by matching its full bytes to the
recorded SHA256 `ac9c338cb3beb77443d4b9be33c8ceaaaf232f774636277df2b00992be459d7e`:
`Error: traffic_launch_profile_mismatch: node ownership/argv/binary changed`.
The exit was 1, not a timeout. The argv hash also matches the expected bound
CLI reap invocation. No reap/service was rerun and no process was signalled.

The source path is CLI main.rs:423 -> RunEngine::attach(adopt=true) ->
engine.rs:520 reverify_traffic_binding -> engine/traffic_runtime.rs:1057-1065,
which requires all nodes to be live Ownership::Owned. The saved run and node
states are Stopped; saved argv and binary hashes match their launch profiles.
Missing/reused nodes cannot pass that live-adoption gate, so engine.reap is
never reached. The exact internal Ownership variant was not recorded. This
is a separate structural cleanup-path failure, not the four-PID global
visibility issue. Known recorded resources being gone does not dismiss it.
REAP-ATTACH-BLOCKER.md documents the source, minimal cleanup-specific attach
requirements and regression cases, including why a naive attach_read_only
switch would risk an empty supervisor/census.

Full bundle admission now calls verifier_admission.py, which uses the pinned
parent-verified handoff, artifact, CONTRACT-v2 `9cb6915a...`, actual Nix
deriver/source, all nine frozen source rows, installed lock/source/fixture and
nine compiled receipt seals. It no longer guesses authority from the current
mutable verifier directory. The single executable remains `5330c3ce...` at
the already-built Nix output; no verifier was duplicated or rebuilt.
verifier-admission.json and COMPILED-VERIFIER-ADMISSION.md publish the exact
descriptor and scope. Other full-bundle requirements remain independent.

Three targeted read-only metadata tests passed: real approved artifact with
all mutable verifier-directory reads prohibited; wrong descriptor/lock/source/
receipt labels rejected; actual deriver source mismatch rejected. No Rust
tests, crypto executable, services, probe, replay or runtime were rerun. The
contract bytes remain the parent-verified `9cb6915a...`; cleanup criteria and
global INCONCLUSIVE status are unchanged. Source-only diagnostics do not
constitute full bundle, native audit or funded acceptance.

## Joined-350 inventory preparation, producer completion still pending

Packet: `/tmp/opencode/AP5-gate-inventory-preparation-r1`. Before editing, the
current 49-file source/report set was frozen there. The old 70-test packet,
332/g105/4d3778 source-qualified receipts, compiled verifier and failed live
network case remain unchanged. The producer's ADMISSION-DELTA.md is retained
as a draft: its past-tense canonical/check claims and an existing passed=350
object are not treated as completed evidence while the parent reports gate
steps still running. No producer source was edited and no gate was polled.

The eight-line global 332-to-350 replacement is not used. The original 332
branch remains. A separate joined-350 schema dispatch now requires a pinned,
source-specific completed qualification keyed by exact source-manifest hash.
The production map in joined-gate-qualifications.json is deliberately empty:
no actual new source is admitted from preparation, a count floor or a caller
label. Normal source/test evidence qualification can populate it after the
completed final producer handoff is verified, without another user-go step.

gate_inventory.py prepares the exact expected matrix: Ark-agent 162, pool Ark
31, lib 84, Node 36, compiled browser 15, lifecycle suites 6/1/3/1, rendered
node-P2P suite 9 and reap-cleanup suite 2. It requires complete actual runner
case lines and summaries, exact case identity lists, package/features/targets,
all check records/log hashes, the browser proof and binary, complete source
rows including the network/reap implementation and tests, and actual Nix
source/root plus six executable bindings. Missing/substituted suites or logs,
counter-only/truncated/skipped/duplicate results and rehashed false summaries
fail even when a top-level counter says 350.

The nine node_p2p_wiring tests are explicitly FixtureNodeControl/rendered-argv
tests, not real rbitcoin propagation evidence. Qualification cannot label them
live propagation. The new source's observed-peer gate must be present in its
qualified implementation, and the later fresh live bootstrap must establish
an actual common block on A and B before funding. No manual block relay or
full native/Rule B/Ark claim is authorized by this source-test census.

Nine targeted Python consumer tests passed using clearly synthetic log/source
fixtures. This includes canonical check_bundle rejection of a 350 count on an
old/unqualified source. No actual 350 qualification, canonical check of the
pending producer bundle, runtime, Node/probe/crypto/Rust test or build ran.
GATE-INVENTORY-CONTRACT-v1.md publishes the exact new qualification/source
contract; its digest and the targeted test receipt are in the packet handoff.

Final preparation check: 20 targeted consumer tests passed after adding exact
historical 332 source/root qualification and verifying the canonical source
fixtures too. The old g105 reader and typed 4d3778 profile cannot lend their
332 census to a new source; a 350 schema on an old or pending source is also
refused. The production 350 map remains empty. No final producer result has
been accepted, no running gate was polled, and no live or heavy work ran.

## Completed 350 qualification and actual fresh diagnostic outcome

The subsequent final producer handoff was verified: 838 actual Nix source
rows, 13 binary hashes, 19 joined check/log bindings and all 16 completed gate
steps. The exact observed matrix is 162 agent Ark + 31 pool Ark + 84 lib +
36 Node + 15 browser + 6/1/3/1 lifecycle + 9 P2P fixture-argv + 2 cleanup = 350
passed. One source-declared agent fixture generator is explicitly excluded,
not counted as passed; generic skips remain refused. The consumer inventory
and source-scope suite passed 21 tests after that narrow exclusion correction.

Only source manifest 033a091fa0b81648fa9dbf3b161000b7783e69878f5480317fc47d4f78cdc838
is registered to `/tmp/opencode/AP5-qualified-350-r1/qualification.json`, hash
9c7790b7feeba1ba1e53ac4c88c7ec086cb8294b44f8745c53cb577d29254cba.
The source is `/nix/store/2jjpwirz3igg8ij8qplp7h30hg3h255j-source`; admitted
CLI SHA256 is ddd7dd270716bfd24006037d758bf6ab91def7db6de33be74556d04b22a40dbf.
Historical 332/g105/70/20 packets are preserved under their original bindings.
Canonical admission passed at `/tmp/opencode/AP5-qualified-350-check-r1`.

The actual 7200-second bounded diagnostic was then executed immediately using
the frozen 52-file/443168-byte harness, with output retained at
`/tmp/opencode/AP5-network-native-live-r1`. It exited 1 at a new compiled-runtime
readiness blocker. Original `ap5-native-bootstrap` began once as runtime-phase,
principal controller, and ended rejected_precondition about 90 seconds later:
`node_network_not_ready: node node-b has not dialed its planned peer yet`.
Both nodes had the correct reciprocal --connect argv and owned listeners;
both reported one inbound peer and zero outbound peers to the readiness gate.
The planned A-B edge therefore remained uncovered. Topology observation
recorded both at genesis. No HashStart/reservation, native bootstrap record,
first common mined block, maturity/96 allocations or user-payment command was
reached. The nine fixture-argv tests did not prove this live behavior.

Exact source locations, retained evidence, genesis/IBD investigation context
and the minimum producer regression are in
`testing/lab-ark-funded/NETWORK-READINESS-LIVE-BLOCKER.md`. The compiled runtime
must reconcile its readiness observer with the pinned node's actual startup
behavior while preserving endpoint ownership and pre-hash graph proof. No
new heavy build or producer/node/server modification was attempted here.

The corrected cleanup attach reached the real reaper: exit 0, 23 exited actor
rows, empty stderr, with full stdout/stderr bytes and hashes retained. The
separate owned-resource census recorded 66 process identities gone/reused,
all 608 listener entries released, no owned survivors or remaining recorded
locks/file references. Global cleanup remains INCONCLUSIVE because the same
four preexisting unreadable PIDs remain untouched and unclassified.

The original sealed run was replayed offline with its frozen harness. Replay
exited 0 and reproduced the inconclusive diagnostic verdict; seal SHA256 is
6fe9f92fb118c0eaf26e3245db9c225529a4b222705c4bf66fa4de69e064b768.
Outcome/replay/source receipt packet: `/tmp/opencode/AP5-network-native-outcome-r1`.
This records actual execution and a concrete new blocker. It does not confer
native funding, full native audit, Rule B, Ark lifecycle or whole-lab acceptance.

## Replacement observer: consumer evidence preparation

Read the completed pinned-node source report at
`/tmp/opencode/node-ibd-peer-readiness-source/REPORT.md` (SHA256
a814a0ac95cdee39619a63d51fbbb68a26d833a07b1e2a020a7b512dc002de06).
IBD owns outbound sockets outside PeerHub; its far ends appear as inbound
PeerHub sessions. Genesis prevents IBD completion and the subsequent normal
follow-outbound setup, making the old pre-hash outbound-row prerequisite
cyclic. The old readiness record lacks completed handshake fields and the
two-process socket joins, so its sealed failed verdict remains unchanged.

Prepared `testing/lab-ark-funded/PEER-READINESS-CONSUMER-REQUIREMENTS.md` with
source/report traceability, both directed handshake/kernel joins, LISTEN and
process/namespace identity bracketing, freshness/ordering, negative cases and
the separate first-real-block propagation requirement. It defines evidence
obligations, not a guessed producer schema or parser. Actual proof version,
DTO, journal record, temporal bounds and source-qualified tests must arrive
from the primary before integration or a fresh run. A new test census will
be qualified from completed evidence, not assumed to remain 350.

The 2484 failure, successful reap, verifier and prior receipts remain frozen.
Standalone propagation checks did not exercise the exact-profile strict gate.
This preparation performed no build, build polling, node RPC, process/socket
inspection, new run or manipulation of the four global unreadable processes.

## QA final capture/drain handoff: full shared-campaign coordination

Reviewed `/tmp/opencode/T3-checkpoint-drain-r17/handoff.json` and its owned
`testing/lab-traffic/CHECKPOINT-CAPTURE-DRAIN.md`. The handoff records 95
affected consumer tests and 98 archived source rows, parent-verified, with no
live joined execution. Recorded AP5 coordination requirements in
`testing/lab-ark-funded/SHARED-CAMPAIGN-FINAL-DRAIN.md`.

The eventual shared campaign needs one shutdown coordinator: drain mining
and feed grants while keeping capture alive; require exact retained candidate
checkpoints, canonical economic acceptance/consumption and due-ledger/source
convergence; then PID-bound pool-writer stop, source EOF and final raw/SQL
recheck before the final seal and remaining-role teardown. Neither harness
may stop the other's required writers/readers/SQL before those captures.
Use the existing QA consumer/oracle. AP5 cleanup and global acceptance remain
separate; source EOF is not successful reaping.

This full-campaign integration requirement does not delay the base IBD
diagnostic or replace its current fresh-readiness/first-real-block blocker.
The old r5 missing-H23 and failed network evidence remain immutable. No wire
or schema change, executable integration, new build/test, runtime or process
action was performed for this coordination note.

## 2026-11-11 resumed execution: actual v2 parser and source blockers

Located the producer's completed IBD observer package at
`/tmp/opencode/ibd-readiness-owned-socket-bundle-r1` and actual proof contract
at `/tmp/opencode/ibd-readiness-owned-socket-source-r1/HANDOFF.md`. Verified
its Nix deriver/source relationship and all nine bound observer source rows.
Implemented strict delivered-v2 proof/report projection parsing in the owned
`readiness_proof.py`; 13 targeted tests passed against source-pinned fixture
and real-regression bytes plus explicitly synthetic counterfeit reports.
Null command IDs in standalone regression records cannot be promoted to an
engine-bound AP5 proof. No source or runtime admission was inferred from them.

New source blockers are recorded in
`testing/lab-ark-funded/READINESS-V2-SOURCE-FINDINGS.md`: no acquisition-duration
bound in the decision, ready returning before the operation deadline check,
after timestamp preceding completion of its capture, and absent actual
namespace/LISTEN/peer-identity bracket evidence. The retained summary flags
cannot reconstruct these missing observations. These are findings in actual
source `zlvpj4d3...`, not a new live failure. Parent was asked for resolution
and the actual canonical bundle/source/check-log descriptor before execution.

The installed package has the four exact R4 control rows, but its receipt
explicitly queues full semantic/native/wallet integration and its CPU remains
0712d0f9. It is not the requested full source union. No old diagnostic rerun,
heavy build, GC or user-process manipulation occurred. The full funded Ark
lifecycle and canonical fresh base run remain blocked on admissible producer
evidence; this revision makes concrete parser/test progress while that source
work is owned by the primary. Evidence: `/tmp/opencode/AP5-readiness-v2-consumer-r1`.

## Full lifecycle API validation while the next observer is produced

Validated approved wallet 2a022552 source/API bindings, R4 retained-board
contracts, the renewal ID recipe, ordinary/derived offboard formats, public
Bark decoding and source-bound QA r17 shutdown ownership. The exact node/pool/
wallet/miner roster and rate declarations remain unchanged. Detailed findings
are in `testing/lab-ark-funded/LIFECYCLE-API-REVIEW.md`.

Fixed concrete owned observation races in ap5.py: first-refresh output identity
now comes from retained RoundCheckpoint.released Full bytes, not every new
Spendable stock row; historical children remain verifiable after automatic
renewal spends them. A second renewal already present in the first observation
can be found from its actual input/operation history, with all original child
rows required. Existing automatic ID, ancestry, crypto and fee checks still
decide acceptance. Return-address lookup now ignores incomplete journal tails,
rejects duplicate keys/foreign paths and seals the actual allocation selected.

Twenty-five affected Python 3.14 tests passed, including nine new race cases,
three existing live-adapter cases and thirteen retained-board cases. The first
system-Python invocation failed at import (3.10 lacks tomllib) and is retained;
it was rerun with the already-admitted interpreter. No live or compiled crypto
test, build, old bundle rerun, user-process action or schema edit occurred.

Identified the remaining full-run integration inputs before launch: actual
independent collector socket/public authorities in each wallet native_audit
profile, and a single shared runtime/final-drain owner. Current independent
AP5 main/finally and QA final_drain must not stop shared roles concurrently.
These do not block the separate base diagnostic. Source/test/evidence freeze:
`/tmp/opencode/AP5-lifecycle-api-review-r1`.

## Implemented shared AP5/T3 execution coordinator

Added the runnable `testing/lab-ark-funded/shared_run.py` entrypoint and strict
`shared_sources.py` loader. The `qa-source-r18.json` descriptor pins the actual
98-file r18 archive and manifest; imports compile verified bytes rather than
mutable QA worktree modules. The actual Controller/source_feed_runtime exports
are used, not campaign.py's archive-consumer option or a success stub.

QA alone creates Runtime and observes the original bootstrap. AP5 binds the
same in-process owner/Popen/run/deadline and performs its live funding, board
and lifecycle observations. The QA check dispatch defers only its existing
final-drain callback without returning a pass. The coordinator then calls
actual final_drain once, retains success/failure in both streams, closes the
collector, and performs AP5 stop/reap once. QA independently assesses that
same reaper evidence; its separate teardown loop is not called. Read-only
cross-component observations are appended before either receipt closes.

Partial startup, lifecycle error, timeout, cancellation, failed pre-EOF or
post-EOF drain, reader-close/result-write failure and repeated finalization
are covered by controlled hook tests. Finalization is non-reentrant; a repeated
run cannot create another runtime. 39 affected tests passed under Python 3.14,
including 14 coordinator/import/adoption cases and the 25 lifecycle/adapter/
retained-proof cases. Entry --help and real frozen r18 imports passed. These
are unit/import results, not funded or live shutdown proof.

Exact API/invocation and known qualified-launch constraints are in
`testing/lab-ark-funded/SHARED-RUN-ENTRYPOINT.md`. Parent/QA was asked to reconcile
r18's launcher operator-ui option and collector-root constraint in the actual
source-bound shared export/inputs. New readiness/full-union inputs remain
unavailable, so no old bundle or shared runtime was launched. Source/test
freeze: `/tmp/opencode/AP5-shared-coordinator-r1`.

## Shared coordinator r19 API rebind and mandatory case driving

Before changes, reverified all 66 rows of the immutable 39-test coordinator
packet. Verified the actual r19 descriptor ea4436e7, archive 336b7f91, manifest
db5f7f98 and all 102 source rows. Added qa-source-r19.json without editing the
old r18 descriptor. The loader keeps exact per-version source/count bindings;
the live shared entrypoint now requires r19 and its actual exports.

Consumed operator_ui=True and the corrected runtime/private collector-root
contract. Added actual qa.measured() and qa.reorg() invocations before AP5
funding/boarding, using Alice's actual allocated native return address. The
measured method invokes its existing manual/witness/fault cases; witness=True
and allow_reorg=True reflect the parent's explicit full-case instruction.
The two witness fixture keys remain labelled test-only and are not Ark wallet
or hash authorities. All phases use the same runtime and original bootstrap.
AP5 lifecycle completes before QA audit and the sole final_drain/stop/reap path.

Fixed a concrete cross-parser mismatch: QA loads JSON rates as Decimal, AP5
as float. Exact represented-numeric comparison now accepts the same 0.2 rate
without coercing strings/bools or accepting lost precision. Added combined
exit classification so QA-only or Ark-only completion cannot yield success.

21 focused tests passed, including real frozen r19 Controller/root helpers,
actual measured-method callback order, same-process/run adoption, wrong-root/
process/source-version refusals, required-case omission rejected by the actual
QA receipt, failure cleanup and single finalization. No broad 70-test rerun,
live campaign, RPC, build, cleanup or source-policy waiver was performed.

The r19 fullrail adapter and its H1-H18/39-transaction historical replay remain
QA's evidence, not a new AP5 live pass. Native outer source 8259f003 is still
uncompiled; exact historical fields requested by r19 remain producer-owned
SPc-side evidence. Next readiness/full-union descriptors remain required for
execution. Current source/API/tests freeze: `/tmp/opencode/AP5-shared-coordinator-r2`.

## Final-prefix ordering correction after r2 review

Preserved and reverified r2's 68 source rows and 21-test receipt before editing.
Replaced the stale-prefix order with live-fact collection, all-driver traffic
quiescence, one final_drain, final-prefix audit, collector close, AP5 teardown,
and a final file-only input recheck before receipt closure. No second drain or
stop is called from QA audit. Earlier failures remain sticky; preliminary
economic/history/native passes are not carried as unqualified final results.

The owned `final_audit.py` bridge binds economic-final-boundary.json to the
actual drain return and captured owner event, checks exact SQL/original-envelope
digests, revision/history watermarks, source EOF bytes/heads and final raw tip,
and supplies those retained inputs to the existing source-pinned r19 fullrail
oracle. It makes no SQL recapture or historical reconstruction. Existing QA
history/native callbacks run post-drain; chain-capturing callbacks use the
final chain. Final audit failure or subsequent input change revokes earlier
prefix passes without replacing an earlier FAILED status. Cleanup still runs
once on every failure path.

27 affected tests passed. Controlled fixtures show the second economic event
appended between preliminary observation and drain is audited at watermark 2,
not the earlier watermark 1; an invalid appended event rejects. Tests also
cover source/SQL mutation, source failure during sampling, final-raw-tip mismatch,
frozen retention (no replacement SQL query), no premature PASS, post-audit input
mutation, repeated finalization and cleanup after failure. These are unit input
selection/order checks, not a new live/native/financial acceptance claim.

QA was asked for its post-drain final-audit export. r19's existing native
activation callback still requires a live miner lookup; it cannot manufacture
a post-stop native pass from a prior result. That unavailable callback remains
inconclusive pending QA's retained-frame/process API and compiled native source.
No Rust build, old bundle rerun, source waiver or user-process action occurred.
Current code/test/source freeze: `/tmp/opencode/AP5-shared-coordinator-r3`.

## V3 readiness parser/source recheck and actual QA r20 integration

Verified and froze all seven v3 observer source rows from the producer handoff
(365987 bytes) before union drift. Implemented strict readiness_v3.py against
the actual delivered raw bracket/process/listener/namespace/dual-peer and
acquisition types. The canonical directed fixture ec4a81f7 parses structurally;
its null operation cannot bind a run. Fourteen targeted tests pass with synthetic
report negatives, including expired success/HashStart despite on-time completion,
wrong source bounds, missing raw joins, changed identity, v2 and duplicate keys.

The source recheck found TIME still incomplete: success_within_deadline ignores
now when completed is Some, and the source test explicitly accepts completion
10000 with now 10600 at deadline 10000. There is no fresh check after persistence/
locking immediately before HashStart. Also, the supplied v3-affected-tests.log
is a failed tail (111 passed, 1 failed, 1 ignored), not the claimed completed
112-pass gate. Both findings were reported; no source approval or run was made.
READINESS-V3-SOURCE-RECHECK.md records exact source locations. The stable AP5
bundle inventory is in CANONICAL-BUNDLE-PRODUCER-INPUT.md; readiness v3 does not
replace the spc.lab.ap5.bundle/v1 envelope or full cumulative union evidence.

Separately preserved/reverified all 70 r3 source rows and bound the actual
107-file QA r20 archive/manifest/API descriptor (33281231). shared_run now uses
the actual Controller class: audit_live, one source_feed_runtime.final_drain,
then final_audit_after_drain before close and one AP5 stop/reap. Removed the
obsolete custom final-audit proxy from active source after confirming it matches
the immutable r3 archive. Old descriptors and 39/21/27-test receipts remain
unchanged. Returned FAILED/INCONCLUSIVE dicts stay nonpassing, and the coordinator
checks the closed /2 boundary, exact result/check census and sticky source verdict.

23 affected r20 tests passed on a bounded retry after a 30-second tool timeout;
the retry took 38.708 seconds and the final-source check took 39.884 seconds.
Tests invoke the real r20 Controller,
collector final methods and native final API on signed synthetic source fixtures,
rejecting late invalid economic/source and native events with exactly one cleanup
and no new node fixture reads. No live or compiled native production pass is
claimed. Native8259 remains uncompiled; full union/readiness/canonical bundle
and historical producer fields remain required.

Source/evidence packets: `/tmp/opencode/AP5-readiness-v3-consumer-r1` and
`/tmp/opencode/AP5-shared-coordinator-r4`. No Rust build, producer edit, old live
retry, subagent, commit or garbage collection was performed.

## 2026-11-12 TIME source correction recheck

Read and independently retained exact engine source b5da9192848e1176fc87bd89eeda2d08cd4912e0a6860efb5d169ec9c635f72f.
The late-now and reversed-completion checks are now explicit, and the gate's
acquisition expiry is carried through the post-persistence/traffic-lock check
before began/start_hash_budget. No v3 wire change or consumer weakening occurred.

Two remaining static findings were reported: CommandLog::start_hash_budget
still waits on its own state mutex after that fresh check and writes without a
new time check; original-operation deadline equality is accepted by the new
helper but refused by both reservation and consumer semantics. The authored
scalar helper tests do not exercise that actual lock/journal transition.
Exact traces and required regression scope are in
`testing/lab-ark-funded/TIME-FIX-SOURCE-RECHECK-20261112.md`.

The relocated producer report 16446b7e is now in its own producer packet.
Read-only r4 censuses verified all 74 sealed rows unchanged, first with the
report as one unlisted addition and then with zero extras after producer
relocation. No r4 file was altered by this review. The r20 coordinator 23-test
and v3 parser 14-test scopes are preserved; no tests/builds or live actions ran.
Producer source remains uncompiled/unadmitted, the 111/1/1 failure is preserved,
and actual green gates plus canonical full source/field bindings remain pending.
Review packet: `/tmp/opencode/AP5-time-fix-source-recheck-20261112-r1`.

## 2026-11-12 portable AP5 integration and actual QA r21

Replaced the intermediate custom r20 tree exporter with actual reviewed r21
`portable_source.py snapshot`. The committed-source exporter is hash-verified;
the descriptor uses relative source.tar.gz/manifest.json references confined to
its directory. Intake checks the exact 123-file / 1456511-byte manifest, recipe,
Git modes, archive order/metadata and all byte hashes before importing. The
live shared runner now requires t3-controller-r21 while retaining one runtime,
original bootstrap, audit_live, one final drain, final audit, close and AP5 reap.

Restored the exact approved nine-file verifier corpus, including LOCK.md;
preserved the later notes byte-for-byte in provenance/verifier-development-lock.md.
The new verifier-reproduction/v1 generator checks the actual Nix source/deriver,
5330 binary, ac24 lock, doCheck and actual log with six exact crypto cases.
It executes public signed-board/foreign-server CLI cases; AP5 admission repeats
and compares those actual results. QA's real r21 reproduced_verifier.admit
accepted freshly generated metadata against the existing approved Nix output.
This is compatibility evidence, not a new build. No parent receipt was copied.
Full qualification additionally requires source-bound verifier fmt/Clippy gates.

Portable probe intake verifies the new actual installed 14-runtime/24-fixture
manifests and native ELF/thin-script/wrapper binding. Local recipe fixtures
remain unit evidence only. The published node Git c1505776/t6LQ source and
portable probe 90d33070 mapping require fresh actual build bindings from primary.

Fresh-machine README/flake wiring and exact source/gate generation are owned
AP5 files. Source-contract, aggregate lineage and gate-plan templates stay
unqualified until primary supplies the complete reviewed committed union.
Readiness fixture tests use committed public r21 bytes. Shared final-audit tests
use r21 snapshot generation rather than private historical archive descriptors.
Historical r4/r3/r2 freezes and failed live archives remain intact.

Initial revised run: 48/49 affected tests passed; the remaining test wrapper had
an indentation error, corrected before the isolated final handoff run. The
earlier 26 portability/probe/readiness tests passed. Final exact-source isolated
test and Nix-parse receipts are retained in the portable integration handoff.
No build, Git index operation, commit, push, runtime launch or cleanup ran.
TIME/current-clock, full compiled readiness regression and funded acceptance
remain pending the producer's complete qualified source/build/live evidence.
