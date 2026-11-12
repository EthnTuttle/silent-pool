# AP-3b R1-R6 audit fixes

Date: 2026-11-09
Status: R1-R6 fixes passed the final combined scoped gate; funded acceptance remains pending

## Baseline and scope

Reviewed the complete primary audit
`docs/reviews/2026-11-09-ark-wallet-bridge-review.md` (R1-R6). Verified all
686 baseline rows/modes/lengths and recomputed full
`e3bac49e741ecf2f00fa1bede472e8aa898590941f3200a690cb73fefc5e0ac2`;
code `8b579e9962f63c7e405c4c1f0f2aa8d1847b1b2fb41caf01e05f26672b2647fa`.
Preserved the 40 changed rows plus manifests, patches, receipts, logs and
scripts in `/tmp/opencode/AP3b-audit-baseline-archive` (2,152,967 bytes).
No full source snapshot or target copy was made. The prior Nix/unfunded fixture
and 142-test records remain intact and are scoped baseline evidence only.

Edits are confined to the wallet maintenance/round/mailbox/scan modules, their
tests and normative documentation. Crypto, provisioning, registration and lab
readiness controls are retained. No primary/other-worktree edit, subagent,
commit, pin change, server modification or cache cleanup.

## Disposition

- R1: mandatory version-2 round checkpoint; full locked signed output bytes
  and funding/unlock/request/input binding; durable write-before-send helper;
  recovery skips leaf cosigning and verifies preimages against persisted bytes.
  A failed/missing checkpoint cannot invoke the forfeit closure. Tests script
  response loss, reopen, wrong preimage/binding, legacy shape and SQL failure.
- R2: terminal filtering precedes the eight-command limit; a SQLite filename
  cursor advances before parsing/network work. Unknown and malformed work
  rotates across restart; immutable payload/outcome tombstones remain. Tests
  exercise ninth-command progress after eight completed commands and alongside
  eight unknown commands, including malformed filenames.
- R3: SQL CASE uses the exact next retry offset in the WHERE predicate before
  LIMIT 128; schedule arithmetic is validated. A persisted due cursor and
  continuation rows rotate work. Tests include epochs 1..128 at reads=4 and
  epoch 129 at reads=0 with tip 500 (valid read chronology), plus restart fairness.
- R4: unattended ticks progress pending stock rounds, exclude their input IDs
  plus owner-queued/reserved inputs, call the pinned stock selection helper,
  and enqueue selected stock renewals through the same reservation path.
  A scripted selector test models first SP-round import followed by a second
  expiry-triggered stock renewal, duplicate suppression and manual reservation
  priority. It does not assert funded server execution.
- R5: per-operation durable safety provenance distinguishes local checks,
  possibly effective RPCs, a specifically observed stock refresh None, and
  legacy unknown history. Known fee/dust rejection/no-op atomically releases
  only its own reservations and retains a terminal payload tombstone. Generic
  errors never release. Tests cover SP dust/aggregate retry across restart,
  stock None, lost Prepare/Forfeit/Finish phases, payload mismatch and a failed
  release transaction retaining uncertainty/reservations.
- R6: explicit burst-continuation result, durable continuation before wire,
  and no retry/closure until have_more=false. Arrival survives burst/restart
  using receipt rows joined to the credited ledger. The eleven-page scripted
  regression covers a first ten-page positive burst, page eleven after reopen,
  duplicate-only replay and an invalid observation without checkpoint advance.

## Verification

Initial `/tmp/opencode/AP3b-audit-r1-gate.log` / `.exit`: nonblocking flock was busy
(exit 1); no compiler or test started. Reported once, no polling/wait/retry loop.
The prepared gate uses only verified Nix 1.93 compiler/Cargo paths, the assigned
`/tmp/opencode/T1a-target`, the complete read-only vendor configuration,
inside-lock >=50 GiB checks, 5 GiB incremental estimate and >=20 GiB residual.
Each individual check/test/fmt/clippy step captures full/code source manifests
before and after, compares them byte-for-byte, and records a distinct log and
exit code. A final combined green gate is required on the changed source.

No new unfunded Nix fixture is claimed or needed for these local state-machine
fixes. Funded receive, SP/ordinary refresh, offboard/finality and native funding
integration still require actual T2/DEST funds. The 8 H/s AP3 scenario is a
small fixture, not the lab's global 40 H/s default.

### Build opportunity and first exercised gate

The parent explicitly released a build opportunity after T2's contextual native
funding gate. The authored 7184ff1b full freeze was reverified before invocation.
`AP3b-audit-r2`: check 0, regressions 0 (**11 passed**), full agent suite 0
(**153 passed, zero failed, one fixture-regeneration test ignored**), fmt 0,
clippy 101. The complete sequence therefore exited 1, not all-green. Every
step's full/code before/after manifest was byte-identical at full
`7184ff1b885bc9a24d598ee304ce9fb471f0386cb45047694af2a513a94c5d4c`,
code `c45e9039453be0de4c1c04eccc4f070664d51e50150ab5883949e9fe6fb50d18`.
Clippy reported only an unused test import and a nonminimal boolean expression;
both were corrected without changing checks or outcomes. Final gate receipts
are recorded separately on the corrected bytes.

Per-case exercised tests: R1 two crash/failed-write/legacy/binding tests; R2 two
terminal/unknown/malformed queue fairness tests; R3 two exact epoch-129 and
restart-continuation fairness tests; R4 one second-generation stock selector
and reservation coordination test; R5 three dust/no-op/uncertain/failed-release
transaction tests; R6 one eleven-page continuation/crash/dedup test. These
scripted state-machine tests are not funded Ark acceptance.

### Final corrected-source gate: all steps green

`/tmp/opencode/AP3b-audit-r3-gate.log` / `.exit`: overall **exit 0**.

| Step | Exit | Result |
|---|---:|---|
| agent ark all-target check | 0 | compiled |
| focused R1-R6 regressions | 0 | 11 passed, 0 failed |
| affected full agent ark suite | 0 | 153 passed, 0 failed, 1 fixture-regeneration test ignored |
| lab wallet bridge integration tests | 0 | 3 passed, 0 failed |
| agent formatting | 0 | clean |
| agent ark all-target clippy, `-D warnings` | 0 | clean |

Every individual step has an exit file, complete log and SHA-bound manifests
before/after execution at `/tmp/opencode/AP3b-audit-r3-<step>-{before,after}.json`.
Every before/after pair is byte-identical. The tested full aggregate is
`d7136f976473898ab7c2048336f0c970072ec0168d1701cabd84864a5cedd306`;
the tested/final code aggregate is
`a270582ddfe2e0a20f11de5316b67b913f3a5a6404419f029633bd19b63a01a3`.
This final report update alone changes the handoff full aggregate after the
gate; the finalizer verifies the exact tested code against the handoff and
records that documentation-only difference explicitly.

Versioned handoff: `/tmp/opencode/AP3b-audit-fix-r2-manifest-{full,code}.json`,
`AP3b-audit-fix-r2-delta.json`, `AP3b-audit-fix-r2.patch`, and
`AP3b-audit-fix-r2-receipts.json`. The original e3bac baseline, prior authored
fix freeze, failed sequences and unfunded Nix evidence remain preserved.
No old sequence with a later failure is represented as an all-green gate.

The round checkpoint shape is private SQLite data; the gate compiled and
exercised its new version, old-shape refusal, write failure and recovery branch.
It does not change public registration/runtime formats, so the old unfunded
Nix fixture was not rerun. Actual funded leaf/forfeit RPC execution, receive,
second-generation renewal and offboard still require the newly available native
T2/DEST tooling plus confirmed provider/pool-wallet funding in the integrated
lab. The same audit reviewer can recheck these ten scoped delta files now.

## Aggregation handoff

The ladder owner supplies the pending/aggregate-status contract authenticated
by fresh owner challenge/session proof. Registration signatures are not read
authentication. No competing wire is introduced here. Positive unbatched
ark_due/pending anchors must keep epochs until the wallet verifies its own
VTXO; Sent only wakes fetching, and coinbase discovery pays only the band.
Unknown obligations cannot expire at an arbitrary H+1014. This downstream
retention/status change will compose with the fair due queue and continuation;
it does not delay R1-R6 fixes or their verification.
