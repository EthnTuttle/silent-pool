# Lab traffic wallet core (T1a) implementation report

Date: 2026-11-08 (revision 3: rework per Fable review; round 3 fixes
S3/S4)
Author: TFF implementation agent
Status: **REWORKED (R3) - all findings H1-H5/M1-M7 of the 2026-11-08 Fable
review addressed with named regression tests, the round-2 fixes
(R1/R2/R12/R6 and packaging) from sections 2.4-2.7, and the round-3
fixes (S3/S4) from section 3.4 of the round-2 re-review; returned for
re-review by the same Fable before integration; no approval claimed**
Worktree: `/home/ethan/.local/share/opencode/worktree/ad8886/lab-traffic-wallet`
Base: `b0ab4ca34fda07c61a6335d05f598eaacc69678c` (frozen lab-final-integration baseline)

Packet T1a of the approved lab traffic plan: the real legacy (P2PKH) traffic
wallet/signing core plus the durable reservation/provenance journal, as a new
crate `crates/spc-lab-txd`. No commits, pushes, resets, stashes, cleans or
main-tree edits were made; the primary worktree, the frozen source and other
agents' worktrees were not touched.

**Revision 1** applies the dispositions of
`docs/reviews/2026-11-08-traffic-wallet-fable.md` (findings H1-H5, M1-M7,
plus the report corrections of its section 7, which are folded into this
revision): journal header v2, ever-uncertain payments never releasing inputs,
the derived invalidation cascade, typed persisted retry caps, the exclusive
journal flock, verified funding observations, digest coverage of payer and
category, direct-index key resolution, allocation validation, write-fault
rollback/poison, derivation v2, and always-record late observations. Every
review counterexample has a named regression test (section 3.4). The normative
record is `docs/decisions/2026-11-08-traffic-wallet-core-rework.md`, with a
`docs/SPEC.md` frontmatter changelog entry in the same change.

**Revision 2** (same review, sections 2.4-2.7) fixes the four remaining
findings and the packaging: R1 (the production write path shares the
rollback-or-poison recovery with injected faults, proven by a real
`RLIMIT_FSIZE` fault in a child process), R2 (a restart appends a durable
`SessionStarted` record - reopen is a writer operation - moving in-flight
`BroadcastSent` transactions to unknown without consuming retry budgets or
inflating attempt counters), R12 (canonical re-inclusion recordable from
`Reorged` but never naming the displaced block; restore cascade one
generation at a time; `note_rejected` finish-guarded with nothing committed
before the error), and R6 (production provisioning consumes an opaque
`ValidatedAllocation` token whose canonical digest is persisted in the v2
header and verified on reopen before any mutation). The SPEC frontmatter
entry moved to the changelog-fragment convention
(`docs/changelog/2026-11-08-traffic-wallet-core-rework.md`); the worktree
`docs/SPEC.md` is restored to the exact frozen baseline bytes.

**Revision 3** (the same review's round-2 re-review, section 3.4) fixes the
two remaining findings, nothing else: S3 (the displaced-block refusal is
stale-evidence only - the journal derives the displacement seq and the
last committed `ChainObserved` seq from the record stream alone, so a
same-block re-inclusion is refused only while no chain observation is
newer than the displacement; once the chain reorgs back, the same-hash
confirmation is admissible as canonical evidence, and the ORIGINAL
displacement identity survives a cascade `Invalidated` and the promotion
back to `Reorged`, so a reorg-back heals the whole cascade generation by
generation - no timestamps, no nondeterministic external checks) and S4
(`note_rejected` mirrors `note_confirmed`/`note_conflicted`: it always
records the rejection - the journal-level transition rules remain the
sole legality gate - and guards only the finish, so a genuine
`bad-txns-inputs-missingorspent` rejection arriving after a reorg
resubmit of a finished command is journaled without error, advances
exactly one record and degrades the inputs; classifying node answers is
T1b's concern and the core never parses the code string). The r2-era
error-first `note_rejected` guard is reversed by S4 and its regression
test was rewritten accordingly.

## 1. Inputs and bounded scope

- Plan (immutable, SHA-256 verified
  `d64a423570509f0974f3853453942e530ec0e21e0c5c3d1904d4d1427022ad18`):
  `/tmp/opencode/traffic-plan-approved-r2/plan.md` with `peer-review.md` and
  `receipt.json` (APPROVED). Sections load-bearing here: plan §5 (wallets and
  keys), §6.3-6.7 (construction, reservation, lifecycle, crediting), §9.8
  (evidence), §11 (money and privacy constraints), §14 (packet split).
- Bootstrap: `/tmp/opencode/LP-final-runtime-manifest.json` applied with
  `patch` only.
- Fable review (authoritative for this rework):
  `docs/reviews/2026-11-08-traffic-wallet-fable.md` with its probe crate
  (`/tmp/opencode/T1a-fable-probe`, 12 counterexamples P1-P6, P8-P13) and
  evidence under `/tmp/opencode/`.
- T0 traffic contract types: **APPROVED SCHEMA/VOCABULARY, FROZEN** (snapshot
  sha256 `3304dffa84c32d546ee783bd15beec09bbfe5fa6755a71c9c5721bc6241d41a8`,
  manifests under `/tmp/opencode/T0-fix-r1-*.json`), usable READ-ONLY as a
  reference. Nothing from them is imported (see §6); the journal header v2
  mandate for T0 is recorded there.

Scope delivered: keys, builder, journal, wallet set, money, errors, adapter
contract, and the test evidence below. Not delivered (owned by other packets,
see §7): the driver actor/control loop, the RPC broadcast loop, the bootstrap
mining loop, node observation wiring, Ark orchestration.

## 2. Deliverable manifest

`crates/spc-lab-txd/` (11,266 lines including tests; line counts
refreshed for revision 3 - the revision-1 table had gone stale through
round 2):

| File | Lines | Content |
|---|---|---|
| `Cargo.toml` | 28 | workspace deps + direct `hmac = "0.12"` and `rustix.workspace = true` (flock, H4; see §5) |
| `src/lib.rs` | 51 | module map, scope statement |
| `src/error.rs` | 230 | fail-closed error surface (`Error`, `KeyError`, `MoneyError`, `BuilderError`, `WalletError`, `JournalError` incl. `HeldByAnotherOwner`, `Poisoned`, `CapsMismatch`) |
| `src/money.rs` | 178 | exact BTC decimal parsing, checked u64/u128 satoshi arithmetic, exact band comparison |
| `src/keys.rs` | 722 | per-role traffic key derivation (v2), provisioning, mode-0600 private artifact, fail-closed reopen, id vocabulary |
| `src/builder.rs` | 697 | legacy P2PKH construction, deterministic fee convergence, exact verification, `sha256d` helper |
| `src/journal.rs` | 3,664 | append-only JSONL journal: v2 header, caps, flock, lifecycle state machine with invalidation cascade and the S3 displacement-seq/chain-observed-seq stale-evidence rule, replay |
| `src/wallet.rs` | 1,307 | wallet set, payment flow, all crash-recovery paths, `checked_destination`, allocation validation, guarded wrappers (S4: `note_rejected` always records) |
| `src/adapter.rs` | 280 | T0 mapping contract, 10 conflicts, v2 header mandate, cross-packet remainder (documentation-only module) |
| `src/tests.rs` | 961 | 23 in-crate unit tests |
| `tests/common/mod.rs` | 655 | conformance fixtures + the independent verification path + RIPEMD-160 vectors |
| `tests/signing_independent.rs` | 163 | golden fixture + cross-process determinism |
| `tests/journal_durability.rs` | 231 | crash/replay/conflict/exclusivity/reopen |
| `tests/lifecycle_reorg.rs` | 258 | lifecycle walk, crediting, reorg caps, pause barrier |
| `tests/fable_regressions.rs` | 1,841 | named regressions for every review counterexample (§3.4) |

Key domain facts (all fail-closed, no donation fallback anywhere):

- **Keys (M5)**: key mode `lab_deterministic_run_seed_v1`, derivation
  version 2 - one BIP32 master per role, seed =
  `HMAC-SHA256(key = run_seed big-endian, msg = run_id || 0x00 || role
  label)`. The run-id domain (absent in v1) makes keys derived for one run
  useless replayed into another; keys are **reproducible from the public
  `RunSpec.seed` and run id** - there is no secret provisioning channel, by
  design (a lab wallet, not a custody system). Labels are byte-identical to
  the T0 seed domains: `spc-lab/traffic/bootstrap-funding`,
  `spc-lab/traffic/network-wallet/<i>`,
  `spc-lab/traffic/user-payment-wallet/<i>`. Fully disjoint from SPc
  master/child/Ark keys by construction. `run_id` and `driver_id` are
  validated against the T0 id vocabulary (non-empty, <= 128 bytes,
  `[A-Za-z0-9-_.:/@]`). Private artifact is mode 0600 with a seed
  fingerprint and network field (key file stays `history_version: 1`,
  extended with the v2 binding fields); reopening with a different seed
  (`FingerprintMismatch`), a different network (`WrongNetwork`), a different
  run id or a corrupt file fails closed - never regenerates. No secret
  material appears in `Debug` output; seeds are zeroized after derivation.
- **Money**: u64/u128 checked arithmetic with named overflow/underflow
  errors; exact decimal BTC parsing (no floats anywhere, including fee
  rates); fee = inputs - outputs exactly; dust and per-tx fee ceilings
  explicit (`dust_sats` 546 default, `max_fee_sats_per_tx` 100,000 default);
  fee band checked against the **final signed vbytes** with exact integer
  products (`min_rate * vbytes <= fee <= max_rate * vbytes`).
- **Fee convergence**: an upper-bound estimate (change output + max scriptSig
  overhead 108 vB/input) selects the input prefix, so **the first reservation
  always builds or fails closed with the reservation released**; the signed
  transaction is then measured exactly. The
  transaction is signed once (RFC 6979 deterministic nonces, low-S
  normalized); only an out-of-band re-target re-signs, bounded by
  `MAX_SIGN_ITERATIONS = 8`, else the named `NotConverged` error.
- **Destinations**: exact requested amounts only; known-script-only change
  (the payer's deterministic change-chain script at the journal-issued
  change index); a destination equal to the change script is rejected as
  ambiguous; outputs are ordered destinations-then-change.
- **Journal (H3, H4, M4)**: single append-only JSONL (mode 0600, write +
  flush + `sync_data` per record, proven-durable length advanced only after
  the sync, torn-final-line tolerated and truncated, strict monotonic
  `seq`); `validate_record`/`commit_record` split shared by runtime and
  replay so replay integrity equals runtime integrity. The first record
  (seq 0) is the **v2 header** binding `format_magic`, `history_version:
  2`, `protocol: "spc-lab-tx-journal/2"`, network, key mode, derivation
  version, run id, driver id, run-seed fingerprint, the typed
  `JournalCaps` and the creation time; v1-shaped journals (no
  `format_magic`) are refused on reopen with a hint - no migration path.
  Caps (`unknown_retry_sends` 0..=3, `reorg_resubmits` 1..=3) are
  persisted in the header and revalidated on every reopen (a differing
  expectation is `CapsMismatch`, fail closed). An exclusive non-blocking
  kernel flock is taken before any replay or truncation: a second live
  owner - in-process or across processes - fails with
  `HeldByAnotherOwner` without mutating; owner exit releases the lock. A
  failed write is rolled back provably (`set_len` + `sync_data`) or the
  handle is poisoned (every further mutation fails closed). The journal is
  the sole funding-provenance ledger: every spendable satoshi entered
  through an explicit `FundingObserved` record with `FundingSource`
  provenance (`BootstrapCoinbase`, `OwnTransaction`, `CanonicalBlock`,
  ...); `SyntheticFixture` funding is refused by production constructors
  and admitted only through the conformance helpers.
- **Reservation/dedup semantics (M1, M6)**: reservation is durable BEFORE
  signing; `command_id` + payload digest dedup where the digest covers the
  canonical `RequestRecord` including the **payer role and payment
  category** - same id + same digest returns the stored bytes (identical,
  never re-signed, across crashes); same id + different digest (payer,
  category, destinations, amounts, fee rate) is a journaled durable
  `CommandConflict`, before and after signing; reservations are exclusive
  across commands (`AlreadyReserved`); a `SignedTx` record whose decoded
  inputs are not exactly the reserved set (or duplicate an input) is
  refused as a tamper; raw bytes and txid are durable before any broadcast
  record; stored raw bytes are re-verified against the stored txid.
- **Funding verification (H5)**: a `FundingObservation` carries the
  outpoint, amount, owner role and its derivation locus `(chain, index)`,
  the source and the provenance context - and **no script**: the wallet
  derives the script at the locus and the journal compares it with the
  evidence. Own-transaction funding requires the journaled `Confirmed`
  creator with an exact vout match (script and amount). Coinbase funding
  counts as available only at 100 confirmations of the observed tip
  (`tip >= height + 100`); no observed tip means immature (fail closed).
- **Lifecycle (H1, H2, H3, M7)**: 10-state `TxState` machine (v1 plus
  `Invalidated`). A `Rejected` record releases the command's inputs only
  when the durable history proves the payment was never uncertain (never
  unknown, never RPC-accepted, never reorg-resubmitted); otherwise the
  claim degrades conservatively to `PossiblySpent` - and the decision reads
  durable counters, so crash replay agrees. Reorg displacement moves
  `Confirmed -> Reorged` and runs a derived cascade to a fixpoint: own-tx
  funding is invalidated when its creator leaves `Confirmed` (prior status
  remembered), descendants on invalidated funding become `Invalidated`, and
  recovery proceeds generation by generation through proven-unspent,
  capped `reorg_resubmit` (`Reorged` has no plain-broadcast edge).
  `unknown` outcomes never release inputs; the unknown cap bounds retry
  SENDS (N retries + the original = N+1 total), never outcome recording.
  `Conflicted` is terminal and consumes the claims; the pause barrier
  refuses broadcasts after the acknowledgment until resume. A canonical
  re-inclusion is recordable from `Reorged` as block evidence, and the
  displaced-block refusal is stale-evidence only (S3): a same-hash
  confirmation becomes admissible once a `ChainObserved` newer than the
  displacement exists, with the original displacement identity preserved
  through the cascade and its generational restore. Late
  confirmations/conflicts/rejections after a reorg cycle are journaled
  (only the command finish is guarded; `note_rejected` always records -
  S4). Confirmed destinations are NOT auto-credited
  - crediting is an explicit `observe_funding` with
  `FundingSource::OwnTransaction` (T3 wires canonical block observation).
- **Key resolution (M2)**: every spending path resolves the input key by
  direct derivation at the outpoint's recorded `(chain, index)`, never by
  script scanning - the script lookahead bounds address derivation for
  external display only and cannot brick a role's change chain.
- **Allocation (M3/R6)**: `ProvisionConfig` carries explicit
  `RangeInclusive` index ranges per wallet kind plus a
  `hosts_bootstrap_funding` flag; `validate_allocation` checks a whole
  multi-actor allocation (non-empty, exactly one bootstrap host, one run
  identity, equal caps, unique driver ids, pairwise-disjoint per-kind
  ranges, valid ids and caps, no empty role set) and returns an opaque
  `ValidatedAllocation` token (private fields, no `Deserialize`) whose
  only constructor it is. Production `provision`/`reopen` take
  `(&ValidatedAllocation, driver_id, dir)` - the token is the proof, and
  its canonical SHA-256 digest (sorted-key JSON over the whole
  allocation) is persisted in the v2 header and verified on every
  reopen before any mutation. Config-only conformance helpers remain
  for the frozen test probes (test-only by name and doc).

## 3. Test evidence (all green)

`cargo test -p spc-lab-txd` (target dir `/tmp/opencode/T1a-target`, log
`/tmp/opencode/T1a-fix-r3-test.log`):

- lib: **28 passed** (23 in `src/tests.rs` + 5 in `src/money.rs`:
  keys/provision/reopen/modes/Debug-secrets, builder
  dust/fee/determinism/exactness, journal replay/conflict/release/barrier/
  torn-line/mode/header binding, unknown-never-releases,
  checked-destination gates, money parsing/arithmetic)
- `fable_regressions`: **26 passed, 2 ignored** (the ignored tests are
  child modes spawned by their parents: the cross-process lock holder
  - killed by design once the parent has proven `HeldByAnotherOwner`,
  its exit is not a pass/fail signal - and the R1 real-write-fault
  prober, which runs to completion in its child process and reports
  `ok` there; the RIPEMD-160 vector test is compiled into this binary
  too - see the count note below). Round 2 added the R1 real-fault
  parent/child pair, three R2 restart tests, three R12 re-inclusion
  tests and the R6 token test; round 3 adds `s3_reorg_back_to_same_
  block_is_canonical_after_a_fresh_chain_observation` (with a
  grandchild, exercising the original-displacement-seq preservation
  two levels deep), rewrites the R12 `note_rejected` test as
  `s4_note_rejected_records_genuine_rejection_and_guards_only_the_
  finish` and keeps the remaining two `r12_*` re-inclusion tests
  unchanged (the new-block re-inclusion edge is untouched by S3/S4).
- `journal_durability`: **6 passed** (crash-at-Reserved resume,
  crash-after-Signed replay, durable conflict, reservation exclusivity,
  wrong-seed/network/run reopen refused, RIPEMD vector module)
- `lifecycle_reorg`: **5 passed** (walk-to-confirmation + explicit
  crediting, reorg displacement + bounded same-bytes resubmission +
  tampered-bytes refusal, terminal conflict, pause barrier, RIPEMD vector
  module)
- `signing_independent`: **3 passed** (golden fixture, cross-process
  determinism, RIPEMD-160 published vectors)

Count note (report correction, updated for revision 3): the run reports
**68 passing test executions** in the parent processes (28 + 26 + 6 +
5 + 3) plus the R1 real-fault child's own `1 passed` run; the shared
RIPEMD-160 vector test in `tests/common/mod.rs` is compiled into each of
the four integration binaries and therefore executes four times, so
there are **67 distinct test functions** (28 + 28 incl. 2 ignored child
modes + 6 + 5 + 3, counting the RIPEMD module once) - 65 of them pass
in-process and the 2 ignored child modes are exercised by their parents
(the lock child holds the lock and is killed by design; the R1 child
runs to completion and its assertions are additionally asserted by the
parent through the result file and exit status). Revision 1 was 59
executions / 57 distinct; revision 2 was 67 / 66; round 3 adds two
tests and removes one (net +1: the s3 test, the s4 rewrite of the R12
`note_rejected` test).

`cargo fmt` clean (`--check`, log `/tmp/opencode/T1a-fix-r3-fmt-check.log`,
one apply pass in `/tmp/opencode/T1a-fix-r3-fmt-apply.log`);
`cargo clippy --all-targets` clean (log
`/tmp/opencode/T1a-fix-r3-clippy.log`; the only remaining cargo
diagnostics are environmental: the shared `~/.cargo` registry cache contains
root-owned files this user cannot clean - unrelated to this crate).

Round-3 counterexample inversion evidence: the frozen round-2 probe
(`/tmp/opencode/T1a-fable-probe-r2/`, never modified) was copied to a
scratch directory (`/tmp/opencode/T1a-r3-probe-check/`) and run against
the fixed crate with `--test-threads=1` (serialized: the probe's
in-process `RLIMIT_FSIZE` in `ok_s1` interferes with parallel tests;
log `/tmp/opencode/T1a-fix-r3-probe-run.log`). Both counterexample
probes now fail exactly on `unwrap_err()`-on-`Ok(())` (`s3_...` at
probe `src/lib.rs:118`, `s4_...` at `src/lib.rs:139` - the fixes
inverted the outcomes) while all four `ok_*` regression probes
(R1/R2/R6/R12) still pass.

### 3.1 Golden fixture

Fixed derivation: key mode `lab_deterministic_run_seed_v1`, derivation
version 2 (run id `t1a-fixed-fixture`, driver `t1a-conformance`), run seed
`0x5452_4146_4649_4321`, one synthetic 1,000,000-sat UTXO `[0x42;32]:0` to
the bootstrap-funding receive-0 address, 250,000 sats to
`NetworkWallet{0}` receive-0, rate 3 sat/vB, change index 0. RFC 6979
nonces make the signed bytes a pure function of the request:

- `GOLDEN_TXID = 10636a5abf76021132012a01f34c22c42d2f302507e210ed50514cf60f446211`
- `GOLDEN_RAW_HEX` (embedded in `tests/common/mod.rs`, 225 bytes):
  `020000000142...88ac00000000` (full constant in the source; also asserted
  byte-for-byte by the crash-resume test, i.e. a crash between reservation
  and signing is invisible in the final transaction).

The derivation change (M5: the run-id domain) changes every derived key, so
the golden constants were **regenerated under derivation v2** and re-verified
through the independent parser/verifier path (§3.2); the v1 values are
obsolete. The `verify_signed_traffic` helper asserts the decoded shape and
conservation exactly (the v1 candidate's bogus even-length hex assertion was
removed).

### 3.2 Independent verification path

`tests/common/mod.rs` verifies every signature, txid, fee and output using a
second implementation unrelated to the signer: a hand-written raw-transaction
parser, a hand-written compact-size codec, direct SHA-256d over the sha2
crate, a hand-written RIPEMD-160 (for HASH160 of the pushed pubkeys), and raw
secp256k1 ECDSA verify. No `SighashCache`, no `bitcoin::hashes` anywhere in
the verification path. It also asserts low-S, SIGHASH_ALL, the fixed v1 shape
(version 2, locktime 0, max sequence), exact fee arithmetic
(inputs - outputs), and the fee band on the final signed size.

The hand-written RIPEMD-160 is pinned to the published vectors
("", "a", "abc", "message digest", "abcdefghijklmnopqrstuvwxyz", the 448-bit
vector, and the million-'a' vector). All expected constants were
cross-checked against an independent system implementation
(`openssl rmd160 -provider legacy -provider default`). Incident note: the
million-'a' constant initially written from memory was wrong
(`52783243fdc7098b...` - a confabulation anchored on the famous prefix);
openssl gives
`52783243c1697bdbe16d37f97f68f08325dc1528`, which is what the hand-written
implementation computes. The test constant was corrected against openssl, not
against the implementation under test. Lesson recorded: hash constants are
never written from memory in this repo.

### 3.3 Crash, conflict and reorg evidence

- **Crash at Reserved** (journal driven to the exact phase, set dropped,
  reopened from disk): resume re-signs the SAME outpoints and change index -
  byte-identical to the golden fixture; inputs stayed locked across the
  crash.
- **Crash after Signed**: same-id retry returns the stored bytes; stored raw
  bytes re-hash to the stored txid.
- **Durable conflict**: same id + different payload is refused before and
  after a crash+reopen; the original still replays.
- **Exclusivity**: a second command's reservation of a reserved outpoint
  fails `AlreadyReserved`; it also cannot reselect around it (no other
  funding) and fails closed without spending.
- **Reorg**: displacement preserves possibly-spent claims (balance 0,
  nothing reselectable); resubmission requires the Reorged state +
  byte-identical bytes + all inputs proven unspent; tampered bytes and
  unproven inputs are refused; the budget is capped by the persisted
  `JournalCaps` (default 3 same-bytes resubmissions), then terminal refusal.
- **Run binding**: wrong seed, wrong network and wrong run id all fail
  closed on reopen; the correct run still reopens with its funding intact.

### 3.4 Fable regression suite (revision 1)

`tests/fable_regressions.rs` - every review counterexample mirrored as a
named regression (probe ids in parentheses; all green in
`/tmp/opencode/T1a-fix-r1-test8.log`):

| Finding | Test | Asserts the fixed invariant |
|---|---|---|
| H1 (P1) | `h1_p1_reject_after_unknown_retry_degrades_to_possibly_spent` | reject after an unknown retry send degrades to `PossiblySpent` (never `Available`), no double-spend reselection, crash replay agrees |
| H1 (P2) | `h1_p2_reject_after_rpc_accept_degrades_to_possibly_spent` | reject after an RPC acceptance degrades likewise |
| H1 (variant) | `h1_unknown_then_direct_rejected_degrades_to_possibly_spent` | the `(Unknown, Rejected)` edge degrades likewise |
| H2 (P3) | `h2_p3_three_generation_cascade_and_recovery` | three-generation cascade invalidates descendants with prior statuses remembered; generation-by-generation recovery through `reorg_resubmit` restores the exact pre-reorg balances; crash replay reconstructs the recovered world |
| M1 (P4) | `m1_p4_payer_or_category_change_is_a_command_conflict` | payer/category change is a durable `CommandConflict` pre- and post-sign; the original still replays |
| H5 (P5) | `h5_p5_funding_backing_and_derivation_are_verified` | unjournaled creator, wrong derivation locus and wrong amount all refused (`FundingNotVerifiable`); the correct observation credits |
| H4 (P6) | `h4_p6_second_live_owner_fails_without_mutation` | second live handle fails `HeldByAnotherOwner` without mutating; live owner keeps working |
| H4 | `h4_owner_exit_releases_the_lock` | owner drop releases the flock |
| H4 | `h4_lock_held_across_processes` (+ `h4_lock_child`, ignored child mode) | a real second process holding the journal makes reopen fail `HeldByAnotherOwner`; kernel releases the flock at child exit |
| H3 (P8) | `h3_p8_reorged_reenters_only_via_resubmit` | no `(Reorged, BroadcastSent)` edge; unproven resubmission refused; cap enforced; every resubmission is a journaled send |
| H3 (P9) | `h3_p9_retry_send_counts_are_exact_across_restart` | caps 0/1/3 give exactly 1/2/4 total sends; outcome recording never capped; counts and cap survive restart |
| H5 (P10) | `h5_p10_coinbase_maturity_gates_availability` | no tip / tip = h+99 / tip = h+100 maturity gating; immature coinbase not selectable |
| M7 (P11) | `m7_p11_late_confirmation_after_reorg_cycle_is_journaled` | late confirm/conflict journaled after reorg cycles; finish guarded (raw `finish_command` stays non-idempotent) |
| M2 (P12) | `m2_p12_small_lookahead_does_not_brick_the_role` | lookahead 2, five chained payments plus a sixth sign - change chain unbounded by lookahead |
| M3 (P13) | `m3_p13_allocation_validation_and_disjoint_provisioning` | allocation validation (empty, two hosts, overlap, divergent identity, bad caps, empty role set) + disjoint two-set provisioning with disjoint role sets |
| M4 (P7) | `m4_write_faults_roll_back_or_poison` | PartialWrite/SyncFailure roll back provably (nothing commits, handle healthy); RollbackFailure poisons (every mutation refused); reopen truncates the torn tail and is healthy |

M5 (derivation v2 / id vocabulary) is covered by the unit tests
(`journal_header_binds_run_identity`,
`wallet_set_from_another_run_or_network_is_refused`, key-store reopen
roundtrip) and by the regenerated golden fixture (§3.1); M6 (decoded inputs
must equal the reserved set) is enforced in `validate_record`
(`src/journal.rs`, with the tampered-bytes resubmission refusal covered in
`lifecycle_reorg`).

Round 2 (re-review sections 2.4-2.7) added `r1_real_write_fault_rolls_
back_or_poisons` (+ its ignored child mode), the three `r2_restart_*` /
`r2_crash_loop_*` tests, the three `r12_*` re-inclusion tests and
`r6_production_provision_requires_a_validated_allocation`. Round 3
(re-review section 3.4) adds and rewrites two rows:

| Finding | Test | Asserts the fixed invariant |
|---|---|---|
| S3 | `s3_reorg_back_to_same_block_is_canonical_after_a_fresh_chain_observation` | same-block re-inclusion refused only on stale evidence (no `ChainObserved` newer than the displacement); after the chain reorgs back, parent/child/grandchild re-confirm in their ORIGINAL blocks generation by generation - the original displacement seq survives the cascade `Invalidated` and the promotion back to `Reorged` (a parent-reconfirmation stamp would wrongly refuse the child); balance restored; replay identical |
| S4 | `s4_note_rejected_records_genuine_rejection_and_guards_only_the_finish` | a genuine `bad-txns-inputs-missingorspent` rejection after a reorg resubmit of a finished command records through the wrapper (exactly +1 seq, no second finish, no error); inputs degrade to `PossiblySpent` (balance 0); reopen matches |

## 4. Contract clarification implemented (parent decision)

The plan §6.6 vs §12 ambiguity is settled by the parent: the reworked
`SendUserPayment` carries an explicit mandatory `destination_address`
alongside the source user, exact amount and fee rate; no implicit destination
rule. This core implements the domain side of that contract:

- `TrafficWalletSet::checked_destination(Address<NetworkUnchecked>, amount)`
  validates `is_valid_for_network` (rejecting wrong-network addresses with
  `DestinationNetworkWrong` before anything is journaled), then the P2PKH
  script class (`NotP2pkhDestination`), then dust, then owner-tags the
  destination when its script belongs to a set wallet (within the
  look-ahead). Testnet and regtest **share base58 version bytes for P2PKH
  addresses but have different human-readable prefixes** (`tb` vs `bcrt`);
  bech32 hrps differ as well. The run's persisted network field carries the
  binding (documented on the method).
- `validate_request` re-asserts the same gates for every destination before
  `begin_command` - a rejected request leaves **no journal record at all**
  (asserted by test: `command(id)` is `None` and `last_seq` is unchanged).
- `build_and_sign` re-asserts the script class (defense in depth) so no
  caller path can sign a non-P2PKH destination.
- Exact amount verification and known-change-only are inherent to the
  builder (§2).

Driver/actor references are moving to a standard bounded `ActorId` type in
the T0 rework; this core takes command-id strings and records the
controlling `command_id` in the journal, so the mapping is a T1b detail.

## 5. Root and shared-file merge surface (merge-owned)

The root `Cargo.toml`, `Cargo.lock` and flake are shared merge-owned; only
compilation-necessary changes were made, all minimal:

1. **Root `Cargo.toml`** - this packet's hunk is the single
   `"crates/spc-lab-txd"` line in `workspace.members` (present since the
   frozen baseline). The worktree root manifest also carries the frozen
   baseline's other member lines and dependency entries (`spc-lab`,
   `spc-cpu-miner`, `bins/spc-lab`, `rustix = "1.1"`, ...) which predate
   this packet. The crate's `rustix.workspace = true` (added in revision 1
   for the H4 flock) reuses that pre-existing workspace entry.
2. **`Cargo.lock`** - report correction (review §7 and §2.5): the
   r1-era "68 added lines" was measured against git HEAD `b0ab4ca`;
   **against the prior T1a candidate the r2 delta is exactly one
   line** - `"libc"` added to the existing `spc-lab-txd` package block
   (the new R1 dev-dependency; `libc 0.2.191` was already in the frozen
   lockfile as a dependency of `rustix`, so no new dependency version
   was introduced). **Against the frozen LP baseline the delta is the
   18-line whole-crate block** (17 lines at r1 plus the `"libc"` line).
   Against git HEAD the diff is 74 added lines, which include the frozen
   baseline's pre-existing lab-crate blocks (`spc-cpu-miner`, `spc-lab`,
   `spc-lab-cli`). On merge, cargo regenerates these entries
   mechanically; no pins were bumped (UPSTREAM_DEPS.md untouched).
3. **`hmac = "0.12"`** is declared directly in the crate (like `spc-lab`'s
   `tempfile`) precisely so the merge-owned
   `[workspace.dependencies]` table needs no new entry; `hmac 0.12.1` was
   already in the frozen lockfile, so the lock gained no new dependency
   versions.
4. **flake** - untouched.

## 6. T0 adapter mapping (recorded, not imported)

`src/adapter.rs` is the typed mapping the parent applies when the reviewed
T0 types are handed over - no second signer, no second journal, no type
duplication, and **zero imports** from the T0 crate. Status: the T0
schema/vocabulary is **APPROVED and FROZEN** (snapshot sha256 `3304dffa...d41a8`),
usable read-only; the T0 preflight gate is recognized as a separate funded
behavior proof, so the independent journal/signature tests here are
unaffected.

**Journal header v2 mandate (hard precondition for the first T1 journal
write):** the T0-approved v1 `TrafficHistoryHeader` `{history_version,
run_id, driver_id, protocol}` cannot carry the fields this core's
fail-closed reopen mandates (network, key mode, derivation version,
run-seed fingerprint, caps), so this core writes a v2 header - the first
record of every journal, at seq 0 - with the exact shape recorded in
`src/adapter.rs` and in the decision record. T0 must adopt the matching v2
type and canonical digest semantics (digest over the full canonical record
encoding, `format_magic` and `caps` included); v1-shaped journals are
refused, never migrated. The key file stays at `history_version: 1`.

Verified matches (checked field-by-field against the frozen source): the
three seed-domain labels byte-identical; history path
`<run_dir>/private/traffic/history.jsonl`; fee ceiling 100,000 sats; retry
caps 3/3 (now the defaults of the persisted `JournalCaps`, resolving the
earlier const-vs-configurable conflict); P2PKH-only v1 profile; native-only
bootstrap mining.

Ten conflicts recorded for the parent to reconcile at handoff (details in
`src/adapter.rs`): (1) journal header shape - **RESOLVED core-side, T0
adoption required** (the v2 mandate above); (2) lifecycle vocabulary (12 vs
10 states - `Invalidated` is now a core state; `Dropped`/`Expired` still
missing; `created` vs `created_signed`; terminality flag differs but the
observable transition table matches); (3) retry caps - **RESOLVED** (typed,
persisted, revalidated `JournalCaps`); (4) fee band width (spec admits up to
100, core default band 1..=20 - `FeePolicy` is data); (5) wallet populations
- **RESOLVED core-side** (range-based `ProvisionConfig` per actor); (6)
`SendUserPayment` destination - **RESOLVED** by the parent decision (§4);
(7) `RpcSignedTxPayload` admission not yet implemented in the core
(fail-closed `admit_declared_tx`-style API is a T1b/T3 remainder); (8) error
types map at the actor boundary; (9) AP-1a projection unconsumed by this
core; (10) allocation validation layering - `validate_allocation` is
core-owned but orchestrator-called before provisioning (M3).

## 7. Explicit remainder (owned by other packets)

- **T1b (driver actor/runtime)**: the actor and control loop
  (`spc-lab-tx-control/v1` hello/ack, capability registry), command-kind
  wiring, the broadcast loop with the drain barrier, RPC routing through the
  controller-mediated `RpcNodeControl` path, the identity-free `tx_traffic`
  evidence projection, calling `validate_allocation` over all actors before
  provisioning any of them (§6 conflict 10), and reconciling the §6
  conflicts.
- **T2-L (bootstrap funding)**: the native block-template loop funding
  traffic addresses (`FundingObserved { source: BootstrapCoinbase }` with
  canonical block context) plus the distribution transactions through this
  same journal and signer.
- **T3 (e2e acceptance)**: node observation wiring
  (`observed_in_mempool`/`confirmed`/`chain_observed` reorg reconcile), the
  independent conservation oracle, the real invalidation/reacceptance case,
  and `RpcSignedTxPayload` admission (conflict 7).
- **AP-5**: Ark funding orchestration through this same API after real Ark
  addresses exist - never a second signer or journal.

## 8. Incidents and environment

- **Disk-full incident** (2026-11-08) - report correction (review §7):
  `/tmp/opencode` is **not a tmpfs**; `findmnt -T /tmp/opencode` resolves to
  `/dev/nvme0n1p2` ext4, the **same root filesystem** (938G, ~96% used at
  review time). The accumulated artifacts filled the disk, not RAM.
  Symptoms: the harness shell tool returned empty output and the write tool
  created empty files (reads kept working). Recovery: removed only this
  packet's own `/tmp/opencode/T1a-target/debug/incremental`; usage evidence
  captured in `/tmp/opencode/du.txt`. No other sessions' artifacts were
  touched, and no further cleanup is requested by the review. Build writes
  were kept minimal afterwards (`-j2`, dedicated target dir); probe litter
  was removed.
- All cargo commands ran online (the offline vendored cache is incomplete),
  with `CARGO_TARGET_DIR=/tmp/opencode/T1a-target`.

## 9. Review gate

The original candidate was reviewed by the Fable
(`docs/reviews/2026-11-08-traffic-wallet-fable.md`, verdict REQUEST_CHANGES
with findings H1-H5/M1-M7). This revision addresses every finding with code
plus a named regression test per counterexample (§3.4), and is **returned
for re-review by the same Fable before any integration**, in particular of:
the H1 ever-uncertain release rule in `commit_record`, the H2 cascade
fixpoint and its recovery edges, the H3 admission-vs-outcome cap semantics,
the H4 flock placement relative to replay/truncation, the H5
observation-verification path, and the M4 rollback/poison proof. The
round-2 re-review (same file, Part 3) narrowed REQUEST_CHANGES to S3/S4
and approved the rest (R1/R2/R6, the new-block re-inclusion edge and
packaging); revision 3 fixes exactly those two items (S3 stale-evidence
displaced-block rule in `journal.rs`, S4 `note_rejected` guard in
`wallet.rs`) and is returned for a diff-scoped re-check of those two
items only. The
normative record is `docs/decisions/2026-11-08-traffic-wallet-core-rework.md`
(lab scope, 12 decisions) with the `docs/SPEC.md` frontmatter changelog
entry written in the same change. Nothing was committed; the worktree
remains uncommitted working state on top of the frozen baseline.
