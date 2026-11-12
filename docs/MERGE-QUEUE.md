# Merge queue and conflict protocol

Purpose: let implementation packets run in parallel worktrees without fighting
over the same files, and make one session - the **merge session** - the only
writer to the shared integration surface.

The orchestrator dispatches packets and owns the board's dispatch rows. The
merge session owns integration: rebase, conflict resolution, the gate, the
SPEC changelog and the merge commits.

## Shared files (merge-session-owned)

These are the collision points. A packet may need a change here; it ships the
change as a *merge note* in its queue entry (exact hunk or a patch file under
`merge/<branch>/`) and lets the merge session apply it. A packet never lands a
commit that edits these on its own branch unless the queue entry says it owns
the file for that packet.

| File | Why it collides | Owner |
|---|---|---|
| `docs/SPEC.md` frontmatter `changelog:` | every decision packet adds an entry at the same line | merge session |
| `docs/STATUS.md` | every packet wants to report | merge session (dispatch rows: orchestrator) |
| `bins/spc-pool/src/config.rs` | every packet adds fields/validation | merge session until the config split lands |
| `bins/spc-pool/src/main.rs` | role wiring for every packet | merge session until the main split lands |
| `docs/OPERATIONS.md` | several packets add sections | merge session until the docs split lands |
| `Cargo.toml` / `Cargo.lock` / `flake.nix` | dependency and vendor-hash changes | merge session |
| `config/*.example.toml` | settings for several packets | merge session |

Everything else is packet-owned: a packet's module, its tests, and its
decision record (`docs/decisions/YYYY-MM-DD-<slug>.md`) are disjoint by
construction.

## Changelog fragments (removes the SPEC conflict)

A packet with normative changes does **not** edit the `SPEC.md` frontmatter.
It adds `docs/changelog/YYYY-MM-DD-<slug>.md` containing the exact bullet(s)
that belong under `changelog:`. At merge the merge session splices them in
(newest first, above the previous entry) and deletes the fragment. Git keeps
the history.

## Queue entry

The dispatching orchestrator appends one line per branch:

```
| branch | base SHA | ready SHA | owned paths | merge notes | gate |
```

The merge session appends the outcome:

```
merged <sha> | conflict: <files> <resolution> | gate: fmt/test/clippy/nix/e2e <result>
```

## Merge session procedure

1. `git fetch` and `scripts/merge-check.sh <branch>`; read the conflict list
   before touching anything.
2. Rebase the branch onto `master` **in the branch's worktree** if the session
   that wrote it is gone; otherwise ask it to rebase. Resolve only the files in
   the ownership table above; leave packet-owned files to the packet.
3. Splice changelog fragments; update the board.
4. Run `scripts/merge-gate.sh` on the merged tree. Any red stops the merge and
   the branch goes back to its author with the failing output.
5. Commit the merge with the why, push, and record the outcome here.
6. Never `stash`/`reset --hard`/`clean` the main tree while another packet's
   work is uncommitted in it.

## Enabling refactor (next, then parallelism scales)

The durable fix is to stop sharing those files:

- `config.rs` -> `config/mod.rs` + per-section modules (`template.rs`,
  `accounting.rs`, `ark.rs`, `reach.rs`, ...); a packet adds a module and one
  `mod` line, not a field in a hot struct.
- `main.rs` -> `main/roles/{front,payout,api,ark}.rs` plus a thin `main.rs`; a
  packet wires its role module.
- `docs/OPERATIONS.md` -> `docs/operations/*.md` with an index.
- `docs/STATUS.md` stays single-writer (merge session).

Until that lands, the merge session absorbs the config/main conflicts.

## Regtest lab dispatch (2026-11-05)

User-requested TFF implementation with Fable approval/rework gates. Tracking:
[`REGTEST-LAB-STATUS.md`](REGTEST-LAB-STATUS.md). These packets remain uncommitted
until a human authorizes commits; ready work is identified by its reviewed diff
and test evidence. Shared-file ownership below is restricted to the packet's
topic-worktree copies; integration remains serial.

| branch | base SHA | ready SHA | owned paths | merge notes | gate |
|---|---|---|---|---|---|
| `lab-sv2-conformance` | `b0ab4ca` | uncommitted snapshot `fa532607` | pool SV2/session/Merkle code and tests; agent SV2 client/tests; tightly required local config/main copies | explicit shared-file diff; coordinate with existing `sv2-tp`; no share-unit/window policy change | Fable APPROVE (second pass); integration gates retained |
| `lab-contracts` | `b0ab4ca` | uncommitted snapshot `3edcd4e7` | new lab design/implementation docs and decision/changelog fragments | no main SPEC/STATUS/config edits; approved contract supplied to downstream packets | Fable APPROVE (third pass) |
| `lab-share-math` | `b0ab4ca` | uncommitted snapshot `45df0eb3` | new `spc-accounting` pure work module, minimal re-export and tests; regression seeds | LP-1a only; full integration waits L1; no dependency changes | Fable APPROVE; independent arithmetic cross-check green |
| `lab-share-integration` | `b0ab4ca` + approved L1/LP-1a/L3 snapshots | uncommitted snapshot `c16032a7` | full v2 accounting/job/window code, store/wire/migration/tests; local config/main/Cargo/flake copies as required | original input worktrees frozen; schema slot recheck at integration; SPEC/changelog notes | Fable APPROVE (third pass); full-node integration pending |
| `lab-cpu-miner` | `b0ab4ca` | uncommitted snapshot `5e27e918` | `bins/spc-cpu-miner/**`, own tests/protocol docs; local Cargo/flake copies | independent vectors/native legacy node proof; full v2 integration remains | Fable APPROVE (fourth pass) |
| `lab-runner` | `b0ab4ca` | uncommitted snapshot `6a97dff0` | core library/CLI/scripts and combined HTTP work; local Cargo/flake copies | bounded foundation; real configs/collectors/interactive runtime follows | Fable APPROVE (third pass) |
| `lab-runner` (HTTP coauthor) | same snapshot `0f2a49c4` | pending, uncommitted | `crates/spc-lab/src/http.rs`, `bins/spc-lab/static/**`, `http_sse.rs` and dedicated HTTP/JS tests, own report | GLM Flash coauthor, disjoint files in same worktree; no Cargo/flake/core edits | Combined LP-3 Fable re-review after both freeze |
| `lab-link-preflight` | `b0ab4ca` | uncommitted snapshot `bce93151` | `scripts/lab/netns-preflight.sh`, `scripts/lab/link-fault.sh` and scoped helpers; `testing/lab-isolation/**`; own report | bounded primitives; runtime namespace/address integration and Nix tooling pending | Fable APPROVE (third pass) |
| `lab-live-e2e` | `b0ab4ca` | uncommitted snapshot `aa3b37b6` | `testing/regtest-lab-v2-e2e.*`, `testing/lab-live/**`, own report | verified real-work/settlement/scanner and retarget arithmetic; no causal budget-response claim; originals frozen | Fable APPROVE (third pass) |
| `lab-runtime` | `b0ab4ca` + approved snapshots | uncommitted snapshot `e891f34c` (tested code `d75ab391`) | serial combined source/config/docs/Cargo/flake integration and real runtime/UI additions | frozen approved host; malformed-hash F1 fix assigned to topology integration | Fable APPROVE bounded host and pool delta |
| `lab-topology` | `b0ab4ca` + approved host `e891f34c` | frozen helper snapshot `374241a3` | namespace helpers, supervisor launch, F1 hash guard and node-only probe | partial delivery; Git metadata exclusion and T1-T9 fixes required; F1 accepted | Fable REQUEST_CHANGES |
| `lab-network-runtime` | approved host + topology helper candidate | uncommitted snapshot `553cd4e8` (code `2d6c752d`) | actual namespace runtime/config/RPC/stdio placement and multi-pool native-mining scenarios | frozen approved baseline; N1-N7 cleanup in separate integration tree | Fable APPROVE bounded combined namespace |
| `lab-final-integration` | approved namespace `553cd4e8` | uncommitted snapshot `9ffff6da` (code `30fedafd`) | reviewed N1-N7 merge cleanup, operator/docs consolidation, final tests/provenance | approved input frozen; exact source transferring to primary | Fable APPROVE final bounded merge revision |
| `lab-v09-validation` | `b0ab4ca` + explicit frozen lab inputs | uncommitted snapshot `5473dbc8` | release validation scripts/scenarios and own report | v0.9+lab-patch functional validation complete on recorded inputs; no pin/adoption change | Fable APPROVE (second pass) |
| `lab-docs` | `b0ab4ca` + approved host docs `e891f34c` | uncommitted snapshot `1d40b2b7` | SPEC/WIRE/OPERATIONS, lab guide, decision/changelog integration and own report | apply only owned approved delta; namespace additions need combined review | Fable APPROVE (second pass) |

The rbitcoin `lab-retarget-profile` packet is in that separate repository and
follows its own contribution rules, with TFF GLM implementation and Fable review.

## Approved lab primary transfer

Orchestrator assigns `ses_e4def8af7ffehCm53JCnmeFu56` (TFF Kimi) serial merge-
session ownership of approved lab code/shared build/config/SPEC/OPS surfaces in
the primary checkout, for byte-verified transfer of `9ffff6da` and the approved
release-validation gate `5473dbc8`. This is not authorization to commit or push.
Existing primary README links and all review/progress records are preserved;
this queue and `REGTEST-LAB-STATUS.md` stay parent-owned. Unexpected concurrent
changes block overwrite. Frozen input worktrees and current node pins stay intact.

Transfer complete, uncommitted: 41 tracked updates, 153 final-input additions,
10 release-validation additions, no deletions or index changes. Parent verified
368/370 final paths (documented README/queue exceptions), 10/10 validation paths,
frozen input hashes and 15 recorded check-log hashes. Fmt, locked RPC-feature
workspace tests/clippy and all six git-free-source Nix checks passed; driver/gate
self-tests and reviewed namespace replay also passed. Initial offline Cargo
cache miss and successful locked retry are both retained. No commits, pushes or
node-pin adoption. Details: `reviews/2026-11-06-lab-primary-integration.md`.

## Nix network-tool limitation follow-up (2026-11-07)

User requested TFF agents resolve pinned network-tool realization and validate
the namespace path with Nix userspace. Existing primary code stays intact.

| branch / packet | baseline | owned paths | merge notes | gate |
|---|---|---|---|---|
| Nix-net fetch | primary locked nixpkgs `0bb2eedc` | scratch diagnosis/realization and `docs/reviews/2026-11-07-nix-network-fetch.md` | GLM owns tool-closure build diagnosis; no source/config/pin edits | exact cause and verified realized tools or narrow remedy |
| `lab-nix-network-tools` | approved final lab `9ffff6da` | local flake/Nix packaging, focused tool validation scripts and operator docs | Kimi owns isolated implementation; no primary overwrite before review | Nix-only userspace namespace evidence -> Fable -> primary integration |

## Browser QA follow-up (2026-11-07)

`lab-browser-qa` owns only new `testing/browser/**` and its implementation report.
It tests explicit approved binaries against separate disposable runtimes; it must
not mutate the manual QA server at port 18480. Root flake/Cargo and production UI
changes are merge notes or separate reviewed fixes, avoiding overlap with the
network-tool packet. Gate: actual browser clicks/state assertions/screenshots ->
Fable review -> integration. Existing helper tests are not browser coverage.

Browser QA is now paused at user request; no harness files were written. Manual
QA on port 18480 was stopped through the durable command API.

## Design/mockup review phase (2026-11-07)

No production UI implementation until rendered mockups receive independent
multi-model review, user feedback and Fable's final check.

| packet | owner | paths | gate |
|---|---|---|---|
| `lab-design-mockups` screens | TFF Kimi `ses_e48e2d10affefAJO5LaBPFXwH2` | `design/mockups/**`, brief/index docs | 14 rendered mockups -> independent GLM/DeepSeek reviews -> revisions/user feedback -> Fable |
| `lab-design-mockups` review tool | TFF GLM Flash `ses_e48e129dbffe5xO1k1BcuLdBLr` | `design/review-app/**`, own review-app doc | persistent JSON/Markdown feedback, safe local gallery and tests -> Fable tool check |

The two writers share a worktree with disjoint paths. User notes stay in an
external persistent state directory; neither writer may overwrite them or turn
mockup interactions into real pool/wallet actions.

## Live Lab hub pivot (2026-11-07)

Draft-2 is paused at user request. `lab-live-hub` (TFF Kimi
`ses_e46b967adffeM95EnJ79kzc2xx`) owns an isolated minimal `/lab` read-only role
directory, an allowlisted live inventory projection, tests and current role docs.
GLM Flash and DeepSeek independently approved the plan; Fable will review the
implementation. Existing controls remain the only command surface. No production
wallet/mockup implementation or primary overwrite is authorized from the topic;
reviewed transfer follows the normal gate. Nix-tool packaging remains separate.

## Expanded live-lab components (2026-11-07)

`lab-experiment-expansion` is plan-only initially. Disjoint TFF owners write:
role/UI/client integration (`ses_e46b3f034ffej2SoSmAtX2BN2Z`), transaction actors
and population presets (`ses_e46b2954affeTU8vy9e9iA4Pwn`), and required Ark services
(`ses_e46b21c1fffeIEl5Aw2Sj4lDY3`) under `docs/plans/`. Ark is core user-requested
scope. Cross-model plan review precedes bounded, isolated implementation packets;
Fable reviews each concrete diff/evidence before serial integration. These plans
do not modify the active hub, Nix tooling, primary code or user design notes.

Ark plan peer review: APPROVE-plan with six clarifications, being incorporated.
`lab-ark-rpc-probe` is the first approved implementation precursor: TFF GLM
`ses_e469bb926ffeVwXphP8aizG45M` owns only `testing/ark-rpc/**` and its report,
read-only capability probes against owned fresh node instances. No node version,
production behavior or upstream pin change is authorized by this probe. Gate:
actual capability results and negative tests -> Fable -> named compatibility
profile implementation decision.

AP-2/AP-3a implementation dispatch (isolated, no primary writes):

| worktree | owner | scope | gate |
|---|---|---|---|
| `lab-ark-registration` | TFF Kimi `ses_e4683467dffeSvKkshwNsMjADp` | 0x5350 codecs, durable registration/store/front integration, destination derivation, related tests and spec fragments | actual wire/DB/privacy evidence -> Fable |
| `lab-agent-wallet` | TFF GLM `ses_e4682e8a7ffeRXSDct2er76CSZ` | wallet key/state/counter/recovery/status foundation; consumes AP-2 codec without duplicating it | fixture/persistence/agent evidence -> Fable; live Ark lifecycle later |

Shared Cargo/config/main/schema edits are local testing copies with merge notes.
AP-4/AP-5 require named node compatibility, witness mining, ladder integration
and a live float/expiry budget that does not deadlock chain progress.

T4a witness primitives (2026-11-08): `lab-witness-primitives`, TFF GLM
`ses_e463308d8ffeQPpMjPUNcF9fRf`, uses the frozen approved final runtime baseline.
Owns pool template/block/coinbase witness primitives and deterministic vectors.
Minimal `front.rs` wiring is a serial merge hunk coordinated with AP-2; shared
Cargo/config/exports remain merge-owned. No services or mining in this packet.
Gate: independent vectors and targeted checks -> Fable -> serial integration ->
T4b native-PoW canonical mixed-witness proof before Ark A4/A7/A8 acceptance.
Traffic plan's eight peer findings are being amended separately; legacy driver
dispatch follows a confined plan re-check. There is one signer and one canonical
probe lineage, with AP-5 orchestrating Ark funding after real addresses exist.

T0 traffic contract (2026-11-08): `lab-traffic-contract`, TFF GLM
`ses_e4604a9d0ffeHEKwnwhfH0S61J`, independently cleared by traffic peer review
against plan `ca4e520730692e84c06f07fad594831a38ffc1ccdc05b17e2106898fef9b100e`.
Owns new traffic schema/protocol/projection modules and tests; shared scenario,
module exports and runtime/package integration remain serial merge hunks.
No second probe or signer, services or mining. Canonical AP-1a mapping depends
on the corrected owner-side report and five-method traffic coverage. Fable
reviews concrete code before adoption. T2-L/T5 plan corrections proceed separately.

Traffic plan corrections are complete: approved plan hash
`d64a423570509f0974f3853453942e530ec0e21e0c5c3d1904d4d1427022ad18`, frozen with
review and parent receipt at `/tmp/opencode/traffic-plan-approved-r2/`.
T1a `lab-traffic-wallet`, TFF GLM `ses_e45fe3e45ffe2yR0qc3Arkzs94`, owns
`crates/spc-lab-txd/**` signing/wallet/journal core and tests. T0 retains public
traffic schema/control/history types; T1a supplies adapter requirements without
forking them. Root Cargo/lock/package edits remain serial merge hunks. T2-L's
`bootstrap.rs` native block loop and T1b actor/runtime integration follow these
foundations. Gate: targeted crypto/crash/replay proofs -> Fable -> integration;
real funded canonical traffic acceptance remains T3.

T4a candidate frozen: `/tmp/opencode/T4a-review-manifest.json`, full
`72c2ffd5f8cdb835e76ef77a71feeae88ae8a238f948f955de878308ef57946a`;
Fable `ses_e45f15b2cffeumsClu5MdysSYk` reviewing. Besides front.rs, T4a has
a narrow payout.rs `detect_mode` commitment-recognition change; merge it as
a reviewed hunk with AP-2, never replace that file. SPEC body amendments and
frontmatter changelog fragment must be reconciled in the same serial integration.
T4b's real mixed-witness and independent-oracle evidence remains a later gate.

T4a Fable rework: main.rs `Published` must include job identity so witness-only
changes reach miners; a minimal local hunk plus ingress test is authorized.
Payout witness-overhead cross-check and real JD-carrier tests also join the
serial AP-2 merge surface. SPEC §2.4.2 must distinguish the prohibited SPc
carrier from the allowed BIP141 commitment. Revised candidate returns to Fable
before any integration; primitive vector correctness alone is not approval.

T4a rework Fable APPROVE: full `b9961746533632eaaa64eb30a508640cc95d9e234521cd81fdebc1a3795439f9`,
code `c3f9b53ac6ac8f7e4e496b72b5d04e8f73427931e0037c7ee48c0bc9f2f86688`.
All affected primary paths still match the frozen baseline. TFF GLM
`ses_e458119eeffe4Jl3MZjJ4J4HJR` is the sole primary code writer for this
transfer, including the approved SPEC frontmatter splice and integration receipt.
Order is T4a first, then reviewed AP-2 hunks preserving witness main/front/payout
changes. No optional nits or unrelated changes in this exact transfer.

AP-1a r2 probe/contract Fable APPROVE: `f4fc3e043f420870675095ab24144ba14e0afba6cb9ad69d818d99b1bccb2f62`,
packet `/tmp/opencode/lab-ark-rpc-probe-r2-review-manifest.json` (`f0aa504a`).
T0 lineage binding still requires its own reviewed decision/table update.
`lab-node-compat-profile`, TFF GLM `ses_e4578e2b0ffecRY7O8BchbJK9r`, now owns
an isolated named-profile candidate and source-bound patch/evidence only.
No upstream pin or primary node changes; node Fable approval/adoption and real
funded/witness/Ark acceptance remain distinct gates.

AP-3a fix-r2 Fable APPROVE: full
`ae7e99e6d77b4be598bcf9a4e9d1466ebf46d7f31d49a398898c564dfabda218`, code
`cd09ed78b7b82c795a43b4798608228f300437e38fc50533638eb8b9b86d0530`.
Queued with existing sole primary writer `ses_e458119eeffe4Jl3MZjJ4J4HJR`
AFTER T4a checks/report/manifest. Apply only AP-3a delta, preserve T4 SPEC
sections/frontmatter, and reconcile root Cargo/lock/flake hunks. Record a
separate wallet integration receipt with merged tests and source-bound Nix gate.
AP-2 follows both packets through narrow hunks; no whole-file replacement.

T0 Fable APPROVE: `/tmp/opencode/T0-fix-r1-review-manifest.json`, full
`3304dffa84c32d546ee783bd15beec09bbfe5fa6755a71c9c5721bc6241d41a8`.
The same primary writer's queue is T4a -> AP-3a -> approved AP-1a tooling -> T0.
Each gets its own transfer receipt; combine not-yet-started Nix validation on
the final clean source snapshot rather than repeat already-green heavy checks.
No other primary code writer is authorized. T1's history-version-2/header
amendment remains a prerequisite before its first journal write; approved T0
is transferred exactly, with that follow-up recorded rather than silently added.

AP-1b Fable APPROVE: node patch `97f84755`, aggregate `f8a51111`, binary
`b75dad84`. Parent authorizes separate opt-in Ark lab profile preparation only.
Owner `ses_e45178b81ffeaN10b9kNhMcxlm`: fresh external node source plus additive
`testing/rbitcoin-bark-v31/**`, exact implementation reports and new conditional
decision/changelog/preparation receipt. No primary runtime/Cargo/flake/SPEC edits.
Node full-suite/probe validation waits for the primary Nix gate to finish;
standard L2 replacement and pin adoption are not part of this transfer.

EXP-OP implementation authorized after both Kimi/DeepSeek v3 approvals:
`lab-pool-operator`, TFF GLM `ses_e44f03a1cffe5EjIbGPld4kKc2`, baseline
`/tmp/opencode/EXP-OP-baseline-manifest.json` (`3e2a31be`, 583 files).
Owns operator module/assets/tests and the focused liability snapshot store test.
Pool config/main/lib and Store snapshot query are local serial-merge hunks,
coordinated with AP-2; no API-backend rewrite or primary edits by this worker.
Gate: actual DB/browser evidence -> Fable -> serial integration. Later ACCESS/
AGENT/QA plans remain separate and cannot change the approved OP scope silently.

EXP-ACCESS foundation dispatched after both v5 plan approvals:
`lab-expanded-access`, TFF GLM `ses_e44bc72bfffefwuJrKdTOymUjk`.
Reuses `/tmp/opencode/EXP-OP-baseline-src` (`3e2a31be`); parent verified 203
Rust/Cargo/flake records against current primary. New access modules/tests are
owned; command/engine/http/scenario/manifest exports are local serial merge
hunks, preserving T0 and future hub changes. No live agent/wallet/traffic claim.
All builds use the shared disk-budget lock; concrete Fable review is required.

T1a core round-3 Fable APPROVE (`65a16584` / `a8d28cea`): existing serial
primary writer `ses_e458119eeffe4Jl3MZjJ4J4HJR` owns exact core transfer and
shared Cargo/lock/vendor/SPEC reconciliation. No actor activation in this step.
`lab-traffic-history-v2`, owner `ses_e4473a19effeWS3JcEvSRsSrC2`, separately
implements the required T0 header-v2 conformance amendment against approved T1a.
That amendment and all runtime traffic remain gated on their own review.
AP-1b validation may proceed under the same build lock, but its SPEC fragment
must not race the T1a primary writer; keep the splice pending if necessary.

EXP-OP rework Fable APPROVE (`d3710c0c` / `365e0bf2`) is queued with the
same serial writer after T1a checks/receipt. Preserve current T1/AP-3 lock
edges; production `rpc` binary provenance is separate from the sealed
test-feature `1684cc84` browser binary. Host-runtime wiring is isolated in
`lab-operator-runtime-wiring` (`ses_e444fa08bffes1FApjGS7mp54I`) and unapproved.
AP-1b node validation is complete; the serial writer may now consume its
pending SPEC fragment alongside the next planned splice, with no default-node
change or funded-Ark claim. Hub and history-v2 packets remain under Fable review.

Hub Fable APPROVE (`3896b132` / `712fa8cb`) queued after T1a/EXP-OP.
Merge owner must apply the review's exact LAB-RUNTIME wallet-scope replacement
and author a lab-tool decision plus one-line changelog, preserving primary
T0/AP-3a content. No embedded-asset changes in the approved transfer.
Host operator wiring may layer the now-approved hub code through narrow hunks
and add its known operator endpoint/link tests; that wiring remains a separate
unapproved packet. History-v2 is awaiting a wording-only correction/recheck.

History-v2 r1 Fable APPROVE (`e2dc40c7` / `4c1bce8c`), queued for exact
amendment-only integration after prior approved packets. The 15 T1a overlay
files and root member/block are already present and must not be recopied.
Only the dev-dependency edge and owned T0/header/docs hunks are new.
The serial integration window is complete and its priority ticket is cleared.
User removed workflow approval holds. Active work may proceed; all heavy
builds still require the shared nonblocking lock and disk headroom. Actual
runtime validation and accurate verification receipts remain required.

Traffic-runtime plan v5 `44daaa2b` is independently APPROVED (Round 5).
Source implementation is dispatched in two isolated worktrees, both based on
the approved 602-file history-v2-r1 layer (`e2dc40c7` / `4c1bce8c`):

- `lab-traffic-probe-profile`, `ses_e43876decffeA3fiuqwCH6FmlA`: canonical
  probe J1 plus probe-side J2, with narrow T0 projection/table/lineage hunks.
- `lab-traffic-journal-v3`, `ses_e4386c6b5ffevCnI2y8xQOxPuJ`: core observer
  snapshot and rollback/maturity amendments with atomic history-3 core/T0
  header/hello/scenario/conformance updates. Control protocol remains v1.

Neither packet is approved code. Complete primary history-v2 integration
precedes their acceptance; use amendment-only serial merges, preserving
operator/hub/access hunks. Shared projection/test fixtures require narrow
reconciliation between these packets. Build windows remain parent-reserved;
no new target directories or unapproved live services are authorized.

EXP-ACCESS rework Fable APPROVE (`b89f74d2` / `8085616e`), queued with the
serial integration owner after hub validation/receipts and history-v2
integration. Six modified plus six added files against pre-OP baseline;
no dependency changes. Preserve primary lock and hub/T0 shared hunks.
Review R4 requires explicit operator-only expanded-mode Inventory, tested
user 403/tokenless 401, and only exact verified data-free hub static assets
in the public exception. These integration adaptations must be recorded
and tested; OP listener and future T1 command scopes are separate.

Host operator-runtime wiring Fable APPROVE (`fd198c44` / `a9b6e4fc`),
queued after hub/history-v2/access. Apply only wiring hunks atop the already
integrated OP/hub code; preserve access route classification and history-2
fixtures. Review D1-D3 are documentation-only conditions (float versus store
facts, opt-in role link, report metadata). Build/validate merged binaries
before a first live `--operator-ui` host run.

AP-2 R2.1 Fable APPROVE (`d94bb158` / `de0e2158`), queued after the
UI/access work. Use `/tmp/opencode/AP2-r21-final-manifest.json`, not the
superseded finalgreen freeze. Reconcile migration slot 0006 without changing
existing migration checksums; merge main/front/payout/SPEC hunks preserving
T4/AP-3/T0. Real DB/Redis and merged RPC/Ark-feature checks are required.
The currently running hub gate must close before any queued source transfer.
