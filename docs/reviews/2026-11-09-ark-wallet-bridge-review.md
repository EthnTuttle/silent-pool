# AP-3b wallet/miner bridge code audit

Date: 2026-11-09
Verdict: **REQUEST_CHANGES** - six concrete findings below, to fix before funded use.
Scope: asynchronous source/evidence review of the frozen AP3b candidate. This
review does not introduce a workflow approval hold. Funded acceptance remains
required before claiming end-to-end readiness.

## Source and evidence binding

Candidate: `/home/ethan/.local/share/opencode/worktree/ad8886/lab-ark-wallet-bridge`.
All source citations below are relative to that frozen candidate, not the
potentially different primary tree.

- Full aggregate: `e3bac49e741ecf2f00fa1bede472e8aa898590941f3200a690cb73fefc5e0ac2`.
- Code aggregate: `8b579e9962f63c7e405c4c1f0f2aa8d1847b1b2fb41caf01e05f26672b2647fa`.
- Baseline: `6529a8f22e808469889a422a2ab51e2950cc308ef520d2e9636245374d989460`.

Independently recomputed the sorted-key compact JSON
`{base_manifest_sha256,files}` recipe. All 686 full rows and 442 non-Markdown
rows match their modes, sizes and SHA256; the actual included path set matches
the manifest exactly. All 671 baseline rows match the self-hosted-Ark source.
All listed receipt-log hashes and five Nix executable hashes match.

The Nix build log identifies
`/nix/store/vm95bb22061a626nd1si133xc9prjz0r-source`; every code row matches
that source byte-for-byte. Its log compiles `spc-agent/ark` and passes 20 scoped
lab tests. The tools output is
`/nix/store/w52v4f6sr6pz32410y9la8sjkk02adga-spc-lab-ark-tools-0.1.0`.
The owned fixture's run manifest binds its ingestion-receipt hash and all four
wallet processes to agent SHA256
`3fa96baa52a9a800b9ed0f96431be0d68714868b1d6f9fee84aa42d3db5526e5`.

`AP3b-check-r6.log` records 142 agent tests passing, zero failing, one ignored;
its later lab-wallet scenario test fails, so the sequence exits 101. R7 passes
the three wallet-bridge tests, then fails agent clippy. R8 passes agent clippy,
then fails lab clippy. R9 passes lab clippy and exits 0. These are individual
step results, not an all-green r6/r7/r8 sequence. The earlier logs have no
independent per-step source freeze proving all those tests ran on the final
code aggregate; the final Nix compilation/runtime binding is independently
checkable. No passing broad check was rerun for this review.

Reviewed current primary/candidate AGENTS, DESIGN, SPEC frontmatter and §11,
ARCHITECTURE, relevant WIRE/ROADMAP sections, MAILBOX, RESULTS, dependency pins,
the candidate decision/report, and the prototype/client derivation and frozen
vectors. The local Bark checkout HEAD is exactly
`c419159fa54085572f8ca72d2ba1716556a9d3eb`, with no tracked modifications.
Compared the relevant vendored ark-lib, bark-wallet and server-rpc sources
against that checkout; corresponding source files match (Cargo manifests have
the expected vendor normalization). Signing/RPC observations below use this
actual upstream source, not assumptions about stock key lookup behavior.

## Findings

### R1 - P1: signed round output proofs are not durable before input forfeits

**Source:** `bins/spc-agent/src/wallet/round.rs:19-26,291-305,337-367`.

`RoundCheckpoint.outputs` contains only `VtxoRequest` policy/amount pairs.
The returned full output genesis chains and finalized leaf signatures exist
only in the local `outputs` vector. The `round_forfeiting` checkpoint stores
the request pairs, funding transaction and unlock hash, with `released=None`.
Only after `ForfeitVtxos` returns and the preimage is validated are the full
output bytes persisted.

**Reproduction:** let the standard server accept the input forfeits, lose the
response, and restart the wallet before `round_released` commits. Its database
has neither the new full output proofs nor the signed locked leaf transactions.
Restart must ask `RoundParticipationStatus` and `RequestLeafVtxoCosign` to
recreate them. If the server is then unavailable or withholds them, the client
has crossed the irreversible boundary without a durable copy of the outputs
whose exitability it checked. This is distinct from the correctly handled
lost *initial* submission response.

**Minimal fix:** persist the validated, leaf-cosigned, still-hash-locked full
VTXOs together with their funding/unlock binding before sending any input
forfeit. Resume from those bytes and validate any recovered preimage against
them. Retain the explicit unknown state after an ambiguous send. A focused
crash-boundary check should prove the signed output proofs survive a lost
forfeit reply without another leaf-cosign RPC. No Ark-server change is needed.

### R2 - P1: completed commands permanently monopolize the eight-command window

**Source:** `bins/spc-agent/src/wallet/maintenance.rs:470-511,606-629`.

Every tick sorts all command `.json` files and processes the same first eight.
Completion writes a `.result` file but neither retires the command nor excludes
terminal operations from selection. There is no persisted scheduling cursor.

**Reproduction:** queue nine valid commands with ascending operation IDs;
complete the first eight. Every subsequent tick still selects those eight,
so command nine is never attempted. The same occurs naturally after eight
automatic refreshes when a later command sorts behind their retained files.
Repeated unknown operations can monopolize the window as well. This can prevent
refresh/offboard until expiry even with a healthy wallet and server.

**Minimal fix:** select pending work fairly before applying the batch limit,
retaining terminal command/payload history for idempotence. Archive or exclude
completed queue entries, and give unresolved operations bounded round-robin
service. Verify that a ninth command progresses after eight completed commands
and alongside eight long-lived unknown commands.

### R3 - P1: mailbox limit is applied before the due-time predicate

**Source:** `bins/spc-agent/src/wallet/scan.rs:114-135,214-216`.

The SQL selects the oldest 128 open epochs; only afterward does Rust discard
epochs whose next retry is not due. Empty older mailboxes remain open through
their final retry at `H+1014`. They hide newer due epochs behind the SQL limit.

**Reproduction:** use open epochs 1 through 128 with `reads=4`, plus epoch 129
with `reads=0`, at tip 500. The first four reads of every older epoch could
already have occurred (latest fourth-read height is 278). None is due again
before height 1015. Epoch 129 has been due since 135, yet `due_epochs(500)`
returns an empty list. I executed the exact source-extracted SELECT against
an in-memory SQLite database and observed this result. The scan marks every
SPc carrier as an epoch, including blocks from other pools, making empty
mailboxes a normal source of these old open rows.

**Minimal fix:** filter by the next due height before limiting, or maintain a
fair persisted due-work cursor. Keep the existing retry offsets and bounds.
Exercise the 129-epoch case with chronologically valid read counts.

### R4 - P1: automatic refresh stops following value after its first SP round

**Source:** `bins/spc-agent/src/wallet/live_bark.rs:82-98`;
`wallet/maintenance.rs:414-437,447-468`;
`wallet/round.rs:380-394`; `wallet/mod.rs:686-715`.

The first SP-owned round imports its outputs into the stock child-wallet
persister and marks its SP-ledger inputs spent. Automatic refresh selection
only examines spendable SP-ledger rows. The stock wallet is opened with
`run_daemon=false`; subsequent ticks call pending-round/offboard synchronization
and balance validation, but never stock maintenance refresh selection.
Stock inputs are refreshable through an explicit owner command, which does
not supply the required unattended renewal.

**Reproduction:** complete one SP refresh, then advance the canonical chain
until its stock-owned output is within the refresh threshold. Keep the wallet
running without an explicit command. It never queues the next refresh; the
value can age through expiry while the process continues to report a live
registration. This violates SPEC §11.5's auto-refresh requirement.

**Minimal fix:** schedule ordinary child-wallet maintenance using the pinned
stock selection/refresh helpers, coordinating with the command reservations.
The standard `maybe_schedule_maintenance_refresh_delegated` helper exists at
upstream `bark/src/lib.rs:1841-1854`; `sync_pending_rounds` only progresses
existing round states (`bark/src/round/mod.rs:2450-2455`). Verify a second
automatic refresh of the output of the first SP round.

### R5 - P2: definite pre-send failures leak input reservations permanently

**Source:** `bins/spc-agent/src/wallet/maintenance.rs:79-86,163-205,562-605,742-765`;
`wallet/round.rs:78-109`.

Inputs are reserved before local fee/dust validation, and no code releases
`live_operation_inputs` on a definite failure or no-op. There is no terminal
safe-rejection state. In particular, stock `refresh_vtxos_delegated` returning
`None` is recorded as `stock_submitted` even though it did not consume inputs.

**Reproduction:** request offboard of a proven SP VTXO whose value cannot cover
the discovered fee plus destination dust. Reservation commits, then local
`validate_and_subtract_fee_min_dust` fails before `PrepareOffboard`. Request a
new operation aggregating that input with another owned input: its reservation
INSERT fails on the existing primary key forever. Restart preserves the leak;
the immutable original command cannot be changed to aggregate more value.
The stranded `prepared` operation also keeps the entire live balance masked
as uncertain despite the locally known pre-send failure.

**Minimal fix:** validate deterministic prerequisites before reserving where
possible; journal a definite no-effect rejection/no-op and atomically release
only those reservations. Preserve the operation ID/payload tombstone. Keep
reservations for every possibly submitted/forfeited operation until actual
reconciliation proves release safe. Cover local dust rejection and the stock
`None` result separately from lost-response cases.

### R6 - P2: a truncated mailbox burst is reported as complete and closes the epoch

**Source:** `bins/spc-agent/src/wallet/live_receive.rs:225-226,323-333`;
`wallet/scan.rs:138-143,214-216`.

After ten pages, `receive_epoch` returns `Ok(received)` even if the last
response has `have_more=true`. `scan_tick` interprets any newly inserted VTXO
as arrival and closes the epoch. Unread pages are never scheduled again.
The prototype explicitly distinguishes truncated reads
(`spc_ark_mailbox/src/lib.rs:311-349`, `EpochReadResult.truncated`).

**Reproduction:** provide a valid epoch mailbox requiring eleven pages, with
at least one newly credited VTXO in the first ten and another on page eleven.
The first call advances checkpoints through page ten, returns a positive
count, and sets `closed=1`; page eleven remains undiscovered. No forged proof
or custom server method is needed.

**Minimal fix:** return/persist a continuation indication and resume the
mailbox before closing the epoch or consuming a scheduled retry. Preserve
the ten-page per-invocation limit and commit-before-checkpoint ordering.

## Checked boundaries and acceptance limits

- Cold provisioning is a separate bounded process with parent-death handling;
  runtime identity loading retains public fields only. Exact BIP85 derivation,
  scan/Ark account checks and prototype key/parity derivations are retained.
  Live key structs do not expose secret Debug output. Wallet/startup partial
  database checks and one-writer locking are present.
- Registration uses the canonical signed counter and required D, checks reply
  correlation, and grants a 30-second lease only on a current successful
  response. Both AddMiner and AddConnection use `require_ready`. Startup TCP
  readiness checks the listener inode against the owned child, in addition to
  later process ownership checks; a bare static status stub is not sufficient
  to pass that startup check. This fixture did not exercise mining admission.
- Full 36-byte IDs, canonical block/anchor checks, owner signatures, receipt
  bytes, deduplication and checkpoint-after-ledger ordering are present.
  Unsupported live mailbox modes are refused. Round policy/amount/unlock,
  unsigned-genesis and funding checks precede forfeits; R1 concerns durability
  of the resulting signed proof, not absence of those checks. Initial round
  response loss correctly remains unknown without an invented lookup.
- Direct offboard uses standard PrepareOffboard/FinishOffboard and the stock
  OffboardForfeitContext with discovered fees. Requests and prepared bytes are
  persisted; changed command payloads reject before spending effects. Signing
  uses upstream helpers with fresh nonces. No custom Ark RPC or new identity
  field in Ark-role streams was found in this delta. Wallet-side chain queries
  are consistent with the client authority boundary.
- Four wallets, identities, miner processes and workers are explicit independent
  declarations. The 8 H/s scenario is a small fixture, not a new global default.
  Its wallet startup hook does not activate miners; combined T1 startup must
  retain its actual funding prerequisite.

The preserved owned run is
`/tmp/opencode/AP3b-owned-unfunded-20261109T044603Z/run`. Its manifest records
15 stopped actors and zero miners; read-only inspection of the four wallet
databases confirms zero VTXOs, and the recorded node height is zero. Receipts
record four fresh registered/connected observations, effective request ID 1
with `already_applied` retry outcomes, private file modes and null balances.
Those observations prove unfunded opening/registration, not payout delivery.

Funded epoch-mailbox receive, SP round refresh, ordinary-output renewal,
cooperative offboard/finality, failure/restart recovery and mining admission
with T1 funding remain unexercised. The findings above come from source traces
and the stated in-memory SQL reproduction, not a claimed funded run. No heavy
build, new runtime, protected-controller access, candidate edit, commit,
subagent or cache cleanup was performed. The candidate report's initial
checkpoint is explicitly superseded; its old compile/wiring gaps are not
current findings in this review.

## Same-auditor R1-R6 recheck - current disposition

Date: 2026-11-09
Verdict: **APPROVE, confined to the R1-R6 source fixes and scoped compiled
evidence. All six findings are closed for this packet.** This supersedes the
earlier REQUEST_CHANGES for the e3bac/8b579 baseline, whose review above remains
historical evidence. It does not assert funded maintenance or end-to-end
readiness and introduces no workflow approval hold.

### Exact source and test binding

Rechecked `/tmp/opencode/AP3b-audit-fix-r2-manifest-{full,code}.json` against
the candidate worktree:

- Full: `0b0c5fd7d5b52e314a0a1aec818b13eb9d134aafa2ca93734310bed028949468`;
  690 rows, all byte lengths, regular-file status, modes and SHA256 matching.
- Code: `a270582ddfe2e0a20f11de5316b67b913f3a5a6404419f029633bd19b63a01a3`;
  443 rows, matching the recomputed aggregate recipe.
- The ten-file delta is against the original reviewed full aggregate
  `e3bac49e741ecf2f00fa1bede472e8aa898590941f3200a690cb73fefc5e0ac2`.
  Reviewed the changed implementations, patch hunks, all 11 focused test
  bodies, SPEC amendment, decision and candidate fix report.

Independently verified all six log hashes and exit files in
`/tmp/opencode/AP3b-audit-fix-r2-receipts.json`. Each step's before/after source
files are byte-identical, and both recipes recompute correctly. All six bind
the final code aggregate above. Their full aggregate is
`d7136f976473898ab7c2048336f0c970072ec0168d1701cabd84864a5cedd306`; its sole
difference from the handoff is the subsequent candidate audit-fix report edit.

The combined r3 sequence exits 0: agent all-target check, 11 focused
regressions, 153 agent tests with zero failures and one fixture-regeneration
test ignored, three lab bridge tests, formatting and all-target agent clippy
with `-D warnings`. The 11 regressions are included in the 153, not additional
unique tests. Unlike the original packet's mixed sequences, this sequence is
green as a whole and has per-step source binding. Prior failed attempts remain
historical. No checks were rerun by this auditor.

### Closure by finding

All citations here refer to the fixed candidate's `bins/spc-agent/src/wallet/`.

**R1 closed - durable signed output proofs.** `round.rs:19-102,209-229,284-355,
407-484` requires checkpoint version 2, records full finalized locked VTXO
bytes, and hashes their input/request/funding/unlock binding. The live path
checks output policy, amount, funding and unsigned genesis, finalizes the leaf
signature, then passes through `forfeit_after_commit`; its closure cannot send
forfeits until the SQLite transaction commits. `maintenance.rs:124-139` now
requires exactly one operation row to be checkpointed. The existing store uses
`synchronous=FULL` (`state.rs:164-166`). Recovery loads the stored locked bytes,
skips both output-status reconstruction and leaf cosigning, and verifies the
returned preimage against the stored binding before release. Legacy JSON lacks
required version/input fields and is refused; an ambiguous forfeit checkpoint
without locked proofs is refused explicitly.

The two tests at `round.rs:560-637` inspect durable bytes inside the send
closure, simulate a lost reply/reopen, check preserved inputs and wrong
preimage/binding rejection, and use an aborting SQLite trigger to prove a
failed checkpoint never invokes the closure. Their proof bytes are explicitly
synthetic. Production cryptographic validation and the no-recosign branch were
source-traced; the tests do not claim a funded leaf/forfeit exchange.

**R2 closed - command fairness.** `maintenance.rs:231-275,816-863` excludes
terminal stages before selecting work and commits a byte-filename cursor
before parsing/network work. The eight-item loop tracks visited paths, wraps
fairly and survives restart. Terminal payload rows remain; `reserve_maintenance`
checks immutable payload equality before replay (`289-349,942-945`).
`audit_regressions.rs:50-117` covers command nine after eight terminal commands,
after eight unknown commands and a reopen, plus malformed-file rotation and
changed-payload rejection. Terminal history is retained rather than deleted.

**R3 closed - due filtering before the limit.** `scan.rs:119-156,254-257`
places the exact CASE-based due predicate in SQL before LIMIT 128, validates
read-count/height arithmetic, and advances a durable epoch cursor before a
read. Continuations remain selectable without waiting for another retry
offset. `audit_regressions.rs:128-157` exercises the original chronologically
valid reads=4/epoch-129/tip-500 case and continuation fairness across reopen.

**R4 closed - renewal follows stock-child outputs.**
`maintenance.rs:353-432,864-888` now calls the actual pinned stock selection
helper after progressing pending rounds. Exclusions include pending-round
inputs, durable operation reservations and valid nonterminal queued owner
commands. Selected renewals enter the same private queue and atomic unique
input-reservation path as manual operations. This is new renewal selection,
not a claim that `sync_pending_rounds` selects new work. The explicit refresh
call remains stock `refresh_vtxos_delegated` (`1082-1091`).
`audit_regressions.rs:292-377` models a stock-owned output of the first SP round,
its later due selection, second renewal, reservation-based duplicate
suppression and manual-input exclusion. The selector/round result is scripted;
real second-generation refresh remains a funded acceptance case.

**R5 closed - safe no-effect release.** `maintenance.rs:142-214,289-349,
505-550,1082-1091` and `round.rs:155-175` bind release to the two specific
call-site proofs: local fee/dust rejection while prepared/local, or an actually
returned stock refresh None with stock-call provenance. Outcome, safety phase
and deletion of only that operation's reservations commit in one transaction;
immutable payload tombstones remain. Generic errors, timeouts and restart of a
possibly invoked call cannot obtain this release path. The three tests at
`audit_regressions.rs:175-290` cover aggregate reuse after dust rejection/reopen,
stock no-op, attempted release in Prepare/Forfeit/Finish uncertainty, payload
mismatch and rollback of an aborting release transaction.

**R6 closed - durable burst continuation.** `live_receive.rs:20-50,258-370`
returns `continuation=true` at a ten-page limit with unread pages.
`scan.rs:142-184,254-257` persists continuation before reading and leaves both
retry count and closure unchanged on truncation. Exhaustion alone resolves the
read; arrival is recovered by joining credited ledger rows to receipts and
the canonical epoch hash. Proof/ledger writes still precede mailbox checkpoint
advancement. `audit_regressions.rs:379-465` exercises ten pages, reopen at
checkpoint 10, page eleven, duplicate-only replay and rejection without
checkpoint advancement. This is a scripted persistence/pagination test, not
a live mailbox-service test.

### State-format and integration limits

The implementation adds safety/scheduling/continuation tables to the existing
SQLite store rather than replacing either wallet database. Existing operations
without safety provenance become `legacy_unknown`, never guessed local/no-effect
(`maintenance.rs:97-113`); only new reservations create local provenance.
Required round-checkpoint version 2 fields have no permissive legacy defaults.
The scan continuation table has an ON DELETE CASCADE foreign key, and the store
enables foreign keys, so a journaled rewind removes displaced continuations.
The command/epoch cursors are persisted, not reset on every loop or reopen.

The original Nix/unfunded fixture remains evidence for the archived baseline,
not execution of these fixed bytes. This recheck started no build, offline
fixture or funded service and made only this append. The parent-reported T2
native-block/allocation progress was not independently exercised here and does
not establish AP3 funded receive/maintenance/finality.

The ownership-authenticated pending/aggregate-status and ladder verification
hunks remain a separate integration dependency. Positive unbatched obligations
must retain their epoch until the wallet verifies its VTXO; a pool Sent status
only wakes retrieval, and a coinbase band does not discharge its Ark remainder.
The future status/retention integration must handle unknown obligations rather
than treating H+1014 as proof of no obligation. This confined R1-R6 approval
does not claim that contract is implemented. Funded mailbox receive, SP round
refresh, stock-child renewal, offboard confirmation/recovery and combined
funding/miner admission still require their actual integrated acceptance runs.
