# Fable code audit: traffic journal history 3 (observer snapshot, atomic rollback, send maturity)

Date: 2026-11-08
Reviewer: Fable (same reviewer as `docs/reviews/2026-11-08-traffic-wallet-fable.md`, Parts 1-4)
Candidate: worktree `lab-traffic-journal-v3`, frozen under
`/tmp/opencode/traffic-journal-v3-tested-r1-manifest-full.json`
Candidate report: `docs/reviews/2026-11-08-traffic-journal-v3-implementation.md` (in the candidate)
Plan: approved runtime plan v5 (`44daaa2b...`), Section 3.5a amendment (f) and 3.5d amendment (e)
Disposition: **APPROVE (bounded, exact hash below)** - no concrete bug found; four
low observations and the controller integration requirements are recorded.

This is an asynchronous audit, not a ceremonial hold. Only this file was written.
No candidate, manifest or other primary file was edited; no commits, subagents,
services, nodes, mining or cache operations.

## 1. Snapshot verification (actual bytes)

Recomputed with the Part 1 recipe over the worktree (excluding `.git`,
`target`, `result`, `cache`, `temp`, `spc_ark_client/test`):

| Quantity | Parent freeze | Recomputed |
|---|---|---|
| file count | 608 | 608 |
| full snapshot | `ff6353c276373e38737e10d1659c1f57206f2cbf83de79242b3b5bc9c4a57746` | identical |
| code snapshot (excl. candidate report) | `df0b05458a6744ed435f094568cb570edc583adcb7d5b75ae959be19369a735a` | identical |

Delta against the approved history-v2-r1 layer (602 files, full `e2dc40c7...`):
**8 modified** (`crates/spc-lab-txd/src/{journal,lib,wallet}.rs`,
`crates/spc-lab/src/traffic/{projection,protocol,spec}.rs`,
`crates/spc-lab/tests/{traffic_contract,traffic_history_conformance}.rs`),
**6 added** (`tests/history3.rs`, two decisions, two changelog fragments, the
report), 0 removed. The baseline worktree `lab-traffic-history-v2` still holds
`journal.rs` at the delta's "before" hash `54d02b35...`, so the journal diff
reviewed below is the true 829-line diff, not the git-HEAD patch. Key
derivation (`keys.rs`), builder, money, the signature/crypto vectors and
`Cargo.lock` are byte-identical to the approved layer. Control protocol stays
`spc-lab-tx-control/v1`; `TRAFFIC_HISTORY_VERSION` / `TX_JOURNAL_PROTOCOL_*`
move to 3; `DERIVATION_VERSION` and the allocation-digest code are unchanged.

Receipts: all five step logs in `traffic-journal-v3-tested-r1-recipe.json`
hash-match on disk with exit 0: core 81 passing executions including the
spawned R1 child (28 + 1 + 26/2 ignored + 12 + 6 + 5 + 3), T0 42 + 7,
projection fixture 7; fmt and `clippy -D warnings` clean. The report's
"136 top-level + child" is arithmetically correct.

## 2. Execution note (disk/lock policy)

`/tmp/opencode/silent-pool-heavy-build.lock` was held by another worker at
the time of this audit (no priority ticket present; 64 GiB free). Per the
policy I report this once and did not poll. A six-probe adversarial crate was
prepared at `/tmp/opencode/T1a-fable-probe-v3/` (lockfile copied from the
candidate, target `/tmp/opencode/T1a-target`) but **was not executed**. The
verdict below therefore rests on the source audit of the true diff plus the
hash-bound receipts of the candidate's own 12 history-3 tests, whose negative
loops I read line by line (Section 4). If a build slot opens, the probes run
with: `flock -n /tmp/opencode/silent-pool-heavy-build.lock bash -c 'cd
/tmp/opencode/T1a-fable-probe-v3 && CARGO_TARGET_DIR=/tmp/opencode/T1a-target
cargo test -j2 -- --test-threads=1'` (space check inside the lock first).
None of them is expected to fail; P2 documents a trust boundary, not a bug.

## 3. Source audit findings per contract item

### 3.1 `observer_snapshot` (amendment e)

Under the single `Mutex<JournalState>`, clones `committed_seq = next_seq - 1`,
the tip as `ChainAnchor`, and a `BTreeMap<OutPoint, ObserverOutpoint>` with
amount/script/owner/locus/source/context/current `OutpointStatus` (including
`Invalidated { prior }`) and the direct `coinbase_immature` flag. No I/O, no
append, no lent mutable state. The DTOs are plain data and cannot construct
`ValidatedAllocation` (private fields, no `Deserialize`) or a
`PendingSendIntent` (private fields, no deserializer). Context mapping:
canonical -> `CanonicalBlock`, `OwnTransaction` -> `OwnTransaction`, fixture
-> `Fixture`. Coherent and immutable as specified.

### 3.2 `record_chain_rollback` / `RollbackEvidence` (amendment f)

`validate_rollback` (called from `validate_record` so runtime and replay share
it) checks, in order: `version == 1`; `state.tip == (old.height, old.hash)`
(a `None` tip refuses); `new.height < old.height`; `common.height <=
new.height`; per branch, `count = anchor.height - common.height + 1` with
checked arithmetic, `count <= 4096` and `headers.len() == count`; each header
exactly 160 lowercase hex chars (checked before `hex::decode`), consensus
decode, hash equal to the expected (anchor first, then each `prev_blockhash`),
no repeated hash in the branch, compact-target negative/overflow rules
(`size > 34`, `word > 0xff && size > 33`, `word > 0xffff && size > 32`),
nonzero target, `target <= MAX_ATTAINABLE_REGTEST`, `validate_pow`; both
branches end at the common hash with identical raw ancestor header; no old
non-ancestor hash appears anywhere in the new branch; the supplied
displacement vector equals `old[..len-1]` exactly (old-tip-first). I checked
rust-bitcoin 0.32.8's `Target::from_compact` uses `wrapping_shl` (masked
shift), so a hostile exponent cannot panic before the explicit overflow check
refuses it. A pure retreat (`new == common`, one header) is admitted, which is
correct (`invalidateblock`-style). Equal/higher-height reorgs remain on the
unproved `ChainObserved` path, exactly as the plan scopes amendment (f).

Atomicity: validation precedes `write_line` (rollback-or-poison on failure);
`commit_record` normalizes the record to the ordinary `ChainObserved`
displacement/cascade with the same `seq`, so one record consumes one seq,
stamps `displaced_at_seq` and `last_chain_observed_seq`, and S3 same-block
re-inclusion needs a strictly later chain observation. Replay re-validates
against the replayed tip (deterministic). The ordinary `ChainObserved` lower
height check is unchanged (`malformed_proofs_...` asserts the unproved lower
tip is refused).

Decode bounds: `bounded_rollback_vec` caps every proof vector at 4096 during
typed deserialization (no `size_hint` trust); `parse_record` borrows only the
tag, refuses a `chain_rollback` line over 2 MiB, then parses. 4096 headers x
2 branches x 160 chars + 4095 hashes fits under 2 MiB, so the bound is
consistent. The enum now carries `deny_unknown_fields`; the candidate's
replay test (mutation 3, an extra record-level field on an otherwise valid
proof) can only fail through that rule, so the rule is proven effective.

### 3.3 Reopen ordering, tails, history 2

`reopen_inner`: lock -> read -> split at the last newline (no truncation yet)
-> replay every complete line with `parse_record` + `validate_record` (a v2
header fails `UnsupportedHistoryVersion` at line 0, before any mutation) ->
caps -> allocation digest -> driver id -> (bound entry only) run id / seed
fingerprint / network -> tail policy -> truncate torn tail -> chmod ->
`SessionStarted`. The wallet reopens through `reopen_bound`, so the
run/seed/network binding now precedes repair and session append (the
candidate test mutates each header field with a torn tail appended and
asserts the bytes are untouched). Tail policy: a complete JSON object without
newline is parsed and validated (so a malformed/unsupported proof cannot hide
behind a missing newline) and then discarded as uncommitted; an incomplete
tail whose streaming tag probe reads `"record":"chain_rollback"` is refused
and preserved; an unidentifiable prefix (e.g. `{"rec`) is truncated as
before. serde_json writes the tag first, so the probe misses only a prefix
shorter than the tag itself, which can never carry proof content.

### 3.4 Send-time maturity (`validate_send_funding`)

Called at the start of `BroadcastSent` and `ReorgResubmit` validation and from
`check_send_before_forward`. Iterative walk with a visited set over the
spent outpoints: each must exist, not be `Invalidated`, not be
`coinbase_immature` at the committed tip (`height.checked_add(100)`; overflow
is immature; no tip is immature); an `OwnTransaction` input requires its
creator `Confirmed` and pushes the creator's inputs. The DAG is finite and
visited-guarded, so it terminates without recursion. Refusal writes nothing:
a retained coinbase keeps its `Reserved`/`Available` status and provenance,
no retry send or resubmit is counted, and the first send after maturity is
restored proceeds (candidate tests `retained_coinbase_...`,
`resubmit_checks_...`, `reservation_survives_...`,
`own_change_ancestry_...`, `maturity_overflow_...` cover each branch with
state/seq equality assertions, not bare `is_err`).

### 3.5 `pending_send_intent` / `check_send_before_forward` (J2 local handoff)

The intent captures the header facts, txid and the exact `(send_sequence,
record seq)` of the last durable send; it is only obtainable while the tx is
`BroadcastSent` on a non-poisoned journal. The check refuses when poisoned,
paused, the committed seq differs from the caller's expectation, the header
facts differ (foreign journal), the tx is not `BroadcastSent`, or the last
send identity moved; then it re-runs the ancestry/maturity rule. It writes
nothing and releases the mutex on return. The API therefore supports the
plan's serialized capture -> local rollback -> forward discipline; it does not
and cannot claim remote-chain atomicity, and the decision says so. Because
`SessionStarted` moves in-flight sends to `Unknown`, no intent survives a
restart; a withheld send is recorded `Unknown` and re-admitted through the
capped retry path (candidate test asserts `retry_sends == 1` after exactly one
withheld-then-retried cycle).

### 3.6 Constants and goldens

T0 `TRAFFIC_HISTORY_VERSION = 3`, `TX_JOURNAL_PROTOCOL_STRING =
"spc-lab-tx-journal/3"`, hello/scenario/header shape unchanged (14 fields),
projection fixture line 2 -> 3 (the one shared hunk, flagged for serial
application with J1). The approved v2 golden is retained and asserted
refused without mutation; a v3 golden differs only in history/protocol. No
crypto, key or allocation-digest bytes changed (manifest-bound).

## 4. Adversarial reading of the candidate tests

`malformed_proofs_and_unproved_lower_tips_never_mutate`: fifteen mutations
(version, anchor heights/hashes, common anchor, dropped header, wrong header,
dropped/duplicated displacement, truncated hex, `u64::MAX` height, unlinked
header, too-easy target `0x2100ffff`, invalid PoW) each asserted `is_err`
**and** snapshot-equal **and** file-bytes-equal; then the unproved lower
`ChainObserved` refused; then the good proof applied. `proof_replay_...`:
ten on-disk mutations x {with, without newline} all refuse reopen with bytes
preserved; the 4097-element vector and the duplicate field fail typed decode;
a half-written proof is refused and preserved; an unidentifiable `{"rec`
prefix recovers the committed tip. `run_caps_allocation_and_history2_...`:
six header drifts (including history 2 + protocol/2) with a torn tail each
refuse without touching the file. These are real negative assertions on the
specific failure condition, not name-only coverage.

## 5. Observations (low; none blocks)

- **L1 - no height/tip consistency guard for retained canonical facts.** A
  canonical funding entry or confirmation whose recorded block hash is not in
  the proof's displaced set stays valid after a rollback below its recorded
  height (prepared probe P2 demonstrates with an off-chain hash). Decision 4
  places canonicality with the observer, and plan 3.5(c) requires funding to
  be observed only from the anchored raw index, so this is consistent; an
  optional cheap guard (`recorded height <= new tip or displaced`) would turn
  an observer inconsistency into a fail-closed integrity error.
- **L2 - equal/higher-height reorgs remain unproved.** Pre-existing contract,
  outside amendment (f); the displaced list for a same-height branch switch
  is an observer claim derived from its anchored index.
- **L3 - permit strictness.** Any append (even a benign tip advance) makes a
  captured `expected_committed_seq` stale; the controller must re-read the
  committed seq and re-check before forwarding. Documented; safe by design.
- **L4 - `Reserved`/`SignedTx` do not run the ancestry walk.** Only direct
  coinbase maturity gates reservation; an own-change output whose ancestry
  turned immature can still be reserved and signed, after which the send is
  refused and the reservation is retained until maturity returns. Fail-closed
  with no money effect; with consistent observer evidence the case cannot
  arise (the creator's block would be displaced).

## 6. Controller integration requirements (carried, not implemented here)

1. Consume history 3 everywhere (scenario `protocol.history_version`, hello
   ack, controller binding) before the first runtime writer; refuse v2 files.
2. Serialize intent capture + `check_send_before_forward` + local
   rollback/chain commits + actual forwarding; refresh the committed seq and
   re-check after any append; withhold on refusal and record `Unknown`.
3. Observe funding only from the anchored canonical raw index (decoded vout
   script and value compared before `observe_funding`); never call
   `record_reorged` for a displacement a `ChainObserved`/`ChainRollback`
   already covers; derive displaced lists for same-height switches from the
   anchored index.
4. Build `RollbackEvidence` from the persisted selected node's headers with
   the run's bound consensus profile verified; the core's regtest ceiling is
   a format bound, not a target schedule.
5. Rescan inclusively from genesis after `reopen` using snapshot statuses:
   skip valid outpoints, re-observe invalidated external/coinbase funding,
   never re-insert own-transaction funding (creator re-confirmation
   restores it).
6. Map "already known / already in block chain" node answers only to a
   confirmation observation backed by real block evidence; never to a
   rejection or a bare confirmation.

## 7. Disposition

**APPROVE (bounded)** of the history-3 core packet exactly as frozen at code
snapshot `df0b05458a6744ed435f094568cb570edc583adcb7d5b75ae959be19369a735a`
(full `ff6353c276373e38737e10d1659c1f57206f2cbf83de79242b3b5bc9c4a57746`),
on the basis of the true 829-line journal diff, the wallet/T0 diffs, the
hash-bound receipts and the line-level reading of the twelve new tests. The
prepared probes in `/tmp/opencode/T1a-fable-probe-v3/` are optional
confirmation for a free build slot and do not condition this approval.
Nothing here approves the runtime controller, native funding, RPC/node
compatibility, probe lineage, live-chain acceptance, or any Ark
(self-hosted, pinned-source, unmodified `captaind`) interoperability result.
