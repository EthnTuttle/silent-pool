# Fable concrete-code review: T1a traffic wallet / signing / durable journal

Date: 2026-11-08
Reviewer: Fable (concrete-code review subagent)
Candidate: `/home/ethan/.local/share/opencode/worktree/ad8886/lab-traffic-wallet`,
crate `crates/spc-lab-txd` (7,070 lines incl. tests)
Candidate report: `docs/reviews/2026-11-08-traffic-wallet-implementation.md`
(in the candidate worktree)
Disposition: **REQUEST_CHANGES** (ranked counterexamples in Section 4; each
has a minimal fix and a regression test)

Nothing in the candidate, the primary tree or any other worktree was edited.
No commits, services, nodes, mining, broadcasts or cache deletions. This file
is the only artifact written into the primary tree. Scratch evidence lives
under `/tmp/opencode/T1a-fable-*` (listed in Section 7).

## 1. Snapshot verification (actual bytes, not git HEAD)

Recomputed independently from the worktree (all regular files, path-sorted,
excluding `.git`, `target`, `result`, `cache`, `temp`, `spc_ark_client/test`;
recipe `sha256(compact JSON {base, files}, sorted keys)`, files =
`{path, mode, sha256}`; code excludes only the candidate report):

| Quantity | Parent freeze (`/tmp/opencode/T1a-review-manifest.json`) | Recomputed |
|---|---|---|
| file count | 385 | 385 |
| full snapshot | `5f00c62609a4a46a87de219059e50201f76569af9274404d20eef15a06a776d8` | identical |
| code snapshot | `cd515a26ca854da6a4a4baced51eeb21e0daec8c6567a38cc465e168838a12ea` | identical |
| baseline manifest sha | `8bbe097d...` | identical |

Delta against the frozen baseline `/tmp/opencode/LP-final-runtime-manifest.json`
(370 files, code `30fedaf...`, recomputed and matching): **0 removed, 2
modified, 15 added**. The two modified roots are additive only:

- `Cargo.toml`: one workspace member line `"crates/spc-lab-txd"`.
- `Cargo.lock`: one new `[[package]] spc-lab-txd 0.1.0` block (16 lines,
  deps `bitcoin, hex, hmac, secp256k1 0.29.1, serde, serde_json, sha2 0.10.9,
  tempfile, thiserror 2.0.21, zeroize`). No version changed, no pin bumped.
  The report's "68 added lines" is measured against git HEAD `b0ab4ca`, not
  the frozen baseline; against the actual integration base it is 16.

Every baseline file is byte-identical. The 15 added files are the crate
(14) plus the report (1). The approved plan hash
`d64a423570509f0974f3853453942e530ec0e21e0c5c3d1904d4d1427022ad18` was
re-verified against `/tmp/opencode/traffic-plan-approved-r2/plan.md`.

## 2. What was run

- `cargo test -p spc-lab-txd -j2` with `CARGO_TARGET_DIR=/tmp/opencode/T1a-target`
  (existing dir, online as the report did; `--offline` fails on unrelated
  workspace resolution). Result: **27 + 6 + 5 + 3 = 41 green**, matching the
  report. Note: the `ripemd160_matches_published_vectors` test lives in
  `tests/common/mod.rs` and is compiled into each of the three integration
  binaries, so it is counted three times; there are **38 distinct tests**.
  The report's per-suite lists name it only in two of the three suites.
- `cargo clippy -p spc-lab-txd --all-targets` and `cargo fmt --check`: clean
  (the only diagnostic is the root-owned `~/.cargo` registry cache warning
  the report describes).
- Independent golden-fixture check in Python (hashlib only): raw length 226,
  txid `799e4d03...6fa8` reproduced from SHA-256d of the bytes, outputs
  `250000` to the destination script and `749319` change, fee `681 =
  3 * 227` (upper-bound size 227, actual 226; realized 3.01 sat/vB), exact
  consumption of the serialization, 33-byte compressed pubkey push.
- A scratch probe crate (`/tmp/opencode/T1a-fable-probe`, path dependency on
  the candidate crate, lock versions copied from the workspace, same target
  dir) with 12 counterexample tests. **All 12 reproduce** (Section 4). The
  probe source is the evidence for each finding; it edits nothing.

## 3. What is sound (bounded, as verified)

- Signing: `legacy_signature_hash(index, prevout_script, SIGHASH_ALL)` per
  input over the unsigned body, RFC 6979, `normalize_s`, DER + `0x01`,
  minimal two-push scriptSig with the compressed pubkey whose HASH160 is the
  prevout script. Multi-input sighash construction is correct (other inputs
  carry the empty script). Production HASH160 is rust-bitcoin; the
  hand-written RIPEMD-160 exists only in the test verification path and is
  pinned to the published vectors (million-'a' constant is the official one).
- Builder: exact `inputs = outputs + fee` asserted on the signed bytes;
  destinations matched exactly once by script with exact amounts; change only
  to the payer's change-chain script at the journal-issued index; a
  destination equal to the change script is rejected; dust and fee ceiling
  explicit; band compared with `u128` products on the measured final vbytes.
- Money: exact decimal parser rejects sign, exponent, second dot, >8 digits,
  empty parts; checked u64/u128 everywhere.
- Journal mechanics: append-only JSONL, `create_new` + mode 0600,
  write + flush + `sync_data` per record, strictly monotonic `seq`, torn
  final line truncated, header version check, `validate_record` shared by
  runtime and replay, raw bytes re-decoded and re-hashed to the txid, fee
  recomputed from the ledger, reserved set == recorded spent set, outputs
  re-checked against the request, change index must be the deterministic
  next, send sequence monotonic, barrier refuses broadcasts.
- Reservation-before-sign, same-id/same-digest replay returning stored
  bytes across reopen, different-digest durable conflict, `unknown` not
  releasing directly, `SyntheticFixture` refused by production constructors
  (and a fixture journal cannot be reopened by the production reopen).
- Key file: provision-once, 0600, wrong seed / wrong network / corrupt file
  fail closed, nothing regenerated; `derive_from_run_seed` refuses
  non-regtest; no secret bytes in any `Debug` (secp `SecretKey` Debug is a
  keyed hash).
- Scope honesty: the report does not claim broadcast, node, funded readiness,
  Ark readiness or T0 approval.

## 4. Findings (ranked; each reproduced by the named probe)

Probe ids refer to `/tmp/opencode/T1a-fable-probe/src/lib.rs`.

### High: core money-safety invariants (fix before any integration)

**H1. `Rejected` releases inputs after an `Unknown` or `AcceptedByRpc`.**
(P1, P2) `commit_record(Rejected)` sets every reserved input back to
`Available`. The transition table permits `Unknown -> BroadcastSent ->
Rejected` and `AcceptedByRpc -> Rejected`. After a timeout the first send may
have been accepted by a node; the same-bytes retry then returns e.g.
`txn-already-in-mempool`, the driver records `rejected`, the coin is released
and the next payment spends it again (P1 asserts the second command signs the
same outpoint). "`unknown` never releases" holds only for the direct edge.
Minimal fix: in `commit_record(Rejected)` release only when
`tx.unknown_retries == 0 && tx.reorg_resubmits == 0` and the prior state was
`BroadcastSent`; otherwise set `PossiblySpent`. (Alternatively drop
`(AcceptedByRpc, Rejected)` from the table.) Tests: P1, P2 as regressions
asserting balance stays 0 and `c2` fails `InsufficientFunds`.

**H2. Reorg does not invalidate the displaced tx's own outputs or their
descendants.** (P3) After `Confirmed` and explicit `observe_funding(
OwnTransaction)` of the change and self-pay outputs, `record_reorged` only
degrades the *inputs* to `PossiblySpent`; the funding entries whose
`outpoint.txid == reorged txid` stay `Available` and a child is signed on a
displaced parent. The ledger has no link from a funded outpoint to its
creating tx. Minimal fix: on `Reorged`, mark every funding entry with
`outpoint.txid == txid` as a new `OutpointStatus::Invalidated` (not
reservable; re-credited only by a new confirmation), and cascade: any
journaled tx whose `spent_outpoints` include such an outpoint moves to an
`Invalidated` lifecycle state (this is the T0 `Invalidated` state, conflict
2) with its own outputs treated the same way. Test: P3 asserting balances
drop to 0 and `c2` fails.

**H3. Reorg and unknown caps are bypassable.** (P8, P9)
`record_broadcast_sent` accepts `Reorged -> BroadcastSent` whenever the
command phase is still `Signed` (always the case if the process died between
`record_confirmed` and `finish_command`), skipping the proven-unspent check
and the resubmit cap entirely (P8 loops five times). Separately, the unknown
cap bounds `record_unknown`, not the retry *send*: after three unknowns a
fourth same-bytes send is journaled and its unknown outcome can then not be
recorded (P9). Minimal fix: `BroadcastSent` validation accepts only
`tx.state == Signed || tx.state == Unknown`; from `Unknown` require
`tx.unknown_retries < MAX_UNKNOWN_RETRIES` (define the cap as retry sends).
`Reorged` is reachable only through `ReorgResubmit`. Tests: P8 expecting
`IllegalTransition`; P9 expecting the fourth send refused.

**H4. No exclusivity across handles or processes.** (P6) `reopen` takes no
lock; a second `TrafficWalletSet::reopen` on a live journal reserves and
signs the same coin under another command (P6: byte-different txs spending
the same outpoint), and the file then fails replay on the next reopen
(`Integrity`). Worse, `reopen_inner` truncates a "torn tail" that may be
another live writer's in-progress line. A Rust `&mut`/`Mutex` is
process-local and does not satisfy plan 6.7 item 1. Minimal fix: take
`flock(LOCK_EX | LOCK_NB)` on the journal fd in `create_inner`/`reopen_inner`
and hold it for the handle's lifetime (rustix is already a workspace
dependency); fail closed with a named `JournalError::HeldByAnotherOwner`.
Test: P6 expecting the second reopen to fail before any reservation.

**H5. Funding observations are unverified caller claims; the ledger has no
coinbase maturity.** (P5, P10)
- `observe_funding(... source: OwnTransaction)` for a txid the journal has
  never seen, with an arbitrary amount, is accepted and credited.
- A foreign P2PKH script owner-tagged to a role is accepted; because
  `sign_fresh` resolves *every* available outpoint before selection, one such
  entry makes every payment from that role fail (`Signing("resolve input
  key...")`). One bad observation bricks a wallet.
- `FundingObserved` carries no height and no coinbase flag;
  `BootstrapCoinbase` funding is immediately reservable (P10 signs against
  it). Plan 5 item 2 and plan 15 item 4 ("still-immature coinbases are
  tracked separately and any spend before maturity is rejected") are
  binding, and the journal is declared the single ledger.
Minimal fix: (a) `TrafficWalletSet::observe_funding` requires
`wallet(owner).owns_script(script)` (better: the observation carries
`(chain, index)` and the core verifies `script_at(chain, index) == script`,
see M2); (b) for `OwnTransaction`, the journal requires `txid` to be a
journaled tx in `Confirmed` state and `tx.output[vout]` to equal
`(script, amount)`; (c) `FundingObserved` gains `height: u64` and
`coinbase: bool`, required for `BootstrapCoinbase`/`CanonicalBlock`, and
`available_outpoints`/`reserve` take a `tip_height` and refuse
`coinbase && tip_height < height + 100`. Tests: P5 expecting both
observations refused; P10 expecting `NotReservable` at tip 50 and success at
tip 101. These change the v1 record schema, which is why they belong now,
before T1b writes any journal.

### Medium: correctness gaps that will bite T1b/T3 (fix with the above)

**M1. Dedup digest excludes `payer_role` and `category`; the journal trusts
the caller's digest.** (P4) Same id + same destinations/fee but a different
payer returns `ReplayedStored` of the other wallet's transaction instead of a
conflict. `begin_command` also never checks that `payload_digest ==
sha256(request)`. Fix: add `payer_role` and `category` to `RequestRecord`;
have the journal compute the digest itself (drop the parameter or verify it).
Test: P4 expecting `CommandConflict`.

**M2. Script-to-key resolution is a 512-deep scan; after 512 reservations a
role's own change outputs are unresolvable and the role can no longer pay.**
(P12, shown with `lookahead = 2`) The change index grows by one per
reservation (released ones included) with no bound; `key_for_script` scans
`0..lookahead` on both chains and fails beyond it, and one unresolvable UTXO
fails the whole role. At 1 tx/s across 8 wallets this is roughly an hour of
the medium preset. Fix: record `(chain, index)` on every owned script in the
journal (`ChangeRecord.index` already exists; add it to `FundingObserved`)
and resolve by direct derivation plus equality check, no scan. The lookahead
then only serves `checked_destination` owner tagging.

**M3. Two-actor provisioning collides.** (P13) `ProvisionConfig::roles()`
always yields `BootstrapFunding` plus indices `0..n`; two actors from the same
seed derive identical keys and both own the single bootstrap wallet, with two
journals for the same coins. The T0 candidate spec likewise defines per-actor
counts with no global index mapping. Adapter conflict 5 ("the core's flat
counts already support this") is not accurate. Fix: `ProvisionConfig` takes
explicit index ranges (`network_wallet_indices`, `user_payment_indices`) and
`hosts_bootstrap_funding: bool`; the integration validates disjointness and
exactly one bootstrap host. Record as adapter conflict 10.

**M4. Write failure leaves an unrecoverable file.** `write_line` on a partial
`write_all`/`sync_data` error returns `Err` with the in-memory state
untouched, but the fd stays positioned after the fragment; the caller's
natural retry appends the next record onto the same line. The file then
fails replay (`Integrity`) and an acknowledged signed/broadcast transaction
is inside an unparseable line. Fix: capture `len_before`, on error
`set_len(len_before)`, and if that fails poison the handle (every further
call returns `JournalError::Io("journal poisoned")`). Test: a test-only
constructor over a read-only fd.

**M5. Key mode is undeclared; "private" is overstated.** All traffic keys
are `HMAC(run_seed_be, label)` with `run_seed = RunSpec.seed`, a public
scenario field (`"seed": 20261106` in all four shipped scenarios). Every
funding/network/user-payment key is recoverable from the scenario or the run
manifest, and scenarios sharing a seed share keys across runs, which is not
"derived per run" (plan 2). The plan does mandate run-seed derivation for
reproducible fixtures, so this is a declaration/binding gap, not a crypto
bug: the report's "private artifact" framing must say the file mode protects
nothing the public seed does not already reveal. Fix: record
`key_mode: "lab_deterministic_run_seed_v1"` in the key file and the journal
header, state it in the report, and put the question of mixing `run_id`
into the HMAC (keeps fixtures reproducible when the fixture fixes `run_id`)
to the parent as a decision; do not approve any secrecy claim for these keys.

**M6. `SignedTx` validation does not bind the decoded inputs.** The
reserved-set check compares the caller-supplied `spent_outpoints` list, not
`tx.input[*].previous_output`; the fee is then recomputed from the claimed
list. In-crate callers are consistent, but the journal's stated replay
invariant and the future `RpcSignedTxPayload` admission (conflict 7) need
`tx.input` to equal the reserved set. One-line fix plus a test.

**M7. Wrapper finish-once inconsistency after reorg.** (P11) `note_confirmed`
and `note_conflicted` commit the lifecycle record and then fail on
`finish_command` ("finished twice") whenever the command was already
finished by its first confirmation; the candidate's own reorg test avoids
the wrapper and calls `journal().record_confirmed` directly. Fix: wrappers
finish only when the stored phase is not `Finished` (or reorg re-opens the
command); test through the wrapper.

### Low (documentation / precision)

- L1. `checked_destination` docs and report 4 say testnet and regtest share
  the "same bech32 hrp"; they do not (`tb` vs `bcrt`; rust-bitcoin rejects a
  `tb1` address for `Regtest`). Only the base58 version bytes (and `tprv`)
  are shared. Fix the wording.
- L2. Fee convergence: re-targeting only ever triggers at `rate == max_rate`;
  then the strictly decreasing size sequence is bounded by `2 * inputs + 2`
  iterations, so with >= 4 inputs at the maximum rate `MAX_SIGN_ITERATIONS =
  8` can (with negligible probability) yield `NotConverged`. The outcome is a
  released reservation and a named error, so it is acceptable, but
  "always builds" in `builder.rs`/the report should read "always builds or
  fails closed with the reservation released". Also: with `min == max` the
  loop reports `FeeAboveBand` for an actually below-band fee; the
  `FeePolicy::dust_sats` comment says "at or below" while the code admits
  `== dust` (546 is correct Core behaviour; fix the comment).
- L3. `next_receive_address` advances only on funding observation, so a
  bootstrap miner asking for 101 addresses up front gets receive-0 101 times
  (address reuse, harmless on regtest; T2-L should know).
- L4. The persisted xprv is bound to re-derivation by a 4-byte BIP32
  fingerprint only; since `load_wallets` re-derives from the seed, the file
  is effectively metadata. Compare the full xprv or document this.
- L5. No parent-directory fsync after `create_new`; `set_permissions` error
  ignored on reopen. Nits.
- L6. Destinations are restricted to P2PKH although plan 7 explicitly allows
  legacy-input transactions paying *to* SegWit outputs pre-T4. A deliberate
  narrowing; record it so T3 does not expect SegWit-owned UTXOs.

## 5. T0 integration conflicts (report 6): core now vs T1b

| # | Conflict | Disposition |
|---|---|---|
| 1 | journal header shape | Core now, because H5/M2/M5 already change the v1 record schema: add `key_mode`, caps (3) and the additive `run_id`/`driver_id`/`protocol` fields in one pass, with a dated decision record for `history.jsonl` v1 (Section 6). |
| 2 | lifecycle 12 vs 9 states | `Invalidated` is required now by H2 (cascade). `Dropped`/`Expired` remain T1b/T6 (declared, never silent). |
| 3 | configurable caps vs consts | Core now: `JournalCaps { max_unknown_retries, reorg_resubmit_max }` at construction, recorded in the header and re-validated on reopen. Pinning 3 while the scenario declares a cap is a silent field drop and is not acceptable. |
| 4 | fee band data | T1b (FeePolicy is data). |
| 5 | per-actor populations | Core now (M3): index ranges and single bootstrap host. |
| 6 | explicit destination | Resolved; implemented correctly. |
| 7 | `RpcSignedTxPayload` admission | T1b/T3, but M6 is its prerequisite in the core. |
| 8 | error mapping | T1b. |
| 9 | AP-1a projection unconsumed | Not a core matter. |

## 6. Normative records

The journal format (`private/traffic/history.jsonl` v1 record set), the
release/possibly-spent/invalidated semantics and the money decisions (dust
546, fee band on final vbytes, release-on-reject rules) are new lab
behaviour that T1b, T2-L, T3 and AP-5 will all depend on. A dated
`docs/decisions/` record (lab scope, same format as the existing ones)
should accompany the fixed candidate before integration, and it should be
written after the H/M fixes so it records the final v1 schema once.

## 7. Report corrections required

- Section 8: `/tmp/opencode` is **not** a tmpfs. `findmnt -T /tmp/opencode`
  resolves to `/dev/nvme0n1p2` ext4, the same root filesystem (938G, 96%
  used at review time). The artifacts filled the disk, not RAM. Only the
  packet's own `/tmp/opencode/T1a-target/debug/incremental` was removed; that
  stays as stated and no further cleanup is requested by this review.
- Section 3: 41 reported tests = 38 distinct (RIPEMD vector test counted
  three times). Say so.
- Section 5: 68 lock lines is against git HEAD; against the frozen baseline
  the delta is 16 lines (one package block).
- Section 2 "Keys": state the key mode and that keys are reproducible from
  the public `RunSpec.seed` (M5).
- Section 4 and `wallet.rs`: hrp wording (L1).
- Section 2 "Fee convergence": "always builds" qualifier (L2).

## 8. Evidence paths (scratch, outside the primary tree)

- `/tmp/opencode/T1a-fable-verify.py` - snapshot/delta recomputation.
- `/tmp/opencode/T1a-fable-tests.log` - the 41-test run.
- `/tmp/opencode/T1a-fable-probe/` - probe crate (`src/lib.rs`,
  `result.log`: 12/12 counterexamples reproduce). P7 (write failure) is
  argued from code, not executed.

## 9. Non-claims

This review approves nothing beyond the bounded statements in Section 3 and
does not: approve node compatibility, Ark readiness, T1b/T2-L/T3 behaviour,
any live or funded proof, the T0 candidate, or the AP-1a lineage. The
golden fixture is a conformance vector over synthetic funding, not runtime
evidence. Once the High and Medium items are fixed with the named regression
tests, a bounded re-review of the diff is sufficient; the sound parts in
Section 3 need not be re-derived.

---

# Part 2 - Rework r1 re-review (same reviewer, 2026-11-08)

Candidate: same worktree, rework freeze
`/tmp/opencode/T1a-fix-r1-manifest-full.json`.
Inputs read: the readme/index, the T0 adapter disposition, the decision
record `docs/decisions/2026-11-08-traffic-wallet-core-rework.md`, the report
revision 1, `src/adapter.rs`, and the full crate source and tests.
Disposition: **REQUEST_CHANGES (bounded)** - the H1-H5/M1-M7 fixes are
real and hold under adversarial probes; four remaining items are listed in
Section 2.4, each small, and one packaging item (Section 2.6) must land
before merge. Part 1 above is preserved unchanged.

## 2.1 Snapshot verification (actual bytes)

Recomputed with the same recipe as Part 1 over the worktree:

| Quantity | Parent freeze | Recomputed |
|---|---|---|
| file count | 387 | 387 |
| full snapshot | `fef50f3437df50a52e143442541f47a70df4bba7bb896f7ff48a9f0470b1eb0e` | identical |
| code snapshot | `a5d3318f61683ec8cf06ee7d1237fb4c3acefaa71250c5a1e6198a42ce7074ce` | identical |
| prior manifest `T1a-review-manifest.json` | `37cb2a87...` preserved | identical |

Delta against the original T1a snapshot (`5f00c626...`, 385 files): **2
added** (`crates/spc-lab-txd/tests/fable_regressions.rs`,
`docs/decisions/2026-11-08-traffic-wallet-core-rework.md`), **0 removed**,
**14 modified** (11 crate files, `Cargo.lock`, `docs/SPEC.md`, the candidate
report). The supplied 522 KB patch is against git HEAD and was not used as
the review delta. Against the LP frozen baseline (370 files) the shared-file
delta is: `Cargo.toml` +1 member line; `Cargo.lock` one new 17-line
`spc-lab-txd` block (now including `rustix`; no pin changed, `rustix 1.1`
already in the lock); `docs/SPEC.md` +1 frontmatter changelog line (see
2.6). The report's "exactly one line vs the frozen baseline" is true only
against the prior T1a candidate, not the LP baseline.

## 2.2 What was run

- `cargo test -p spc-lab-txd -j2` in `/tmp/opencode/T1a-target`: **59
  passed, 1 ignored** (28 + 17 + 6 + 5 + 3). The RIPEMD vector test runs in
  four binaries; 57 distinct test functions (56 passing + the ignored child
  mode). The report's arithmetic is correct. Clippy and fmt clean.
- Independent derivation check (pure-Python secp256k1 + BIP32): the golden
  fixture's pushed pubkey equals `HMAC-SHA256(key = seed_be, msg = run_id ||
  0x00 || "spc-lab/traffic/bootstrap-funding")` -> BIP32 `m/0'/0'`, i.e. the
  declared derivation v2; the v1 (no run id) key differs; txid
  `10636a5a...6211` reproduced over 225 raw bytes, fee 681 (3 x 227
  upper-bound size).
- Adversarial probe crate `/tmp/opencode/T1a-fable-probe-r1/` (13 tests,
  `result.log`): nine `ok_` probes confirm the fixes beyond the
  candidate's own regressions; four reproduce remaining counterexamples
  (Section 2.4). Log: `/tmp/opencode/T1a-fable-r1-tests.log`.

## 2.3 Fixes verified (by probe, not by self-test summary)

- **H1** (`ok_r11`, candidate h1_*): `ever_unknown` / `ever_accepted` /
  `reorg_resubmits` are rebuilt from the record stream; unknown then
  restart then `rejected` degrades to `PossiblySpent`; `(Unknown, Rejected)`
  degrades too. Boundary gap: see R2 below.
- **H2** (`ok_r3`, `ok_r3b`): parent/child/grandchild plus a branch into a
  second wallet (`DEST -> NetworkWallet{1}`) all become `Invalidated` when
  the parent is displaced (explicit record or `ChainObserved`), every
  balance including the third wallet's goes to 0, explicit records are
  refused on invalidated txs, the summary is identical after reopen
  (replay determinism), and recovery is strictly generational: parent
  re-confirmed -> children `Reorged` (not spendable), child re-confirmed ->
  grandchild `Reorged`, balances restored only after every generation is
  re-confirmed. Nothing is auto-restored to `Confirmed`.
- **H3** (`ok_r4`): cap 3 = three retry sends after the original (four
  sends), the fifth send is `UnknownRetryCapExceeded` while unknown outcomes
  remain recordable; caps persist in the header; a different expectation on
  reopen is `CapsMismatch`; no `(Reorged, BroadcastSent)` edge.
- **H4** (`ok_r10`): a second owner fails `HeldByAnotherOwner` before any
  truncation (a torn tail appended behind a live owner is left intact); the
  candidate's cross-process test covers process exit.
- **H5** (`ok_r5`): own-tx funding refused before the creator is confirmed,
  with a wrong index, wrong vout, wrong owner or wrong amount; coinbase
  funding unavailable without a tip, at tip = height + 99, available at
  height + 100; source/context mispairing refused.
- **M1** (`ok_r8`), **M2** (`ok_r9`, lookahead 1, four chained change
  spends), **M6** (`ok_r7`: duplicate input and foreign decoded input
  refused with the command left in `Reserved`), **M5** (derivation v2 and
  the key-mode/derivation-version/run-id bindings in both artifacts, no
  serde defaults on the new key-file fields, full xprv string comparison),
  **M7** for `note_confirmed`/`note_conflicted`.

## 2.4 Remaining counterexamples (ranked, each reproduced)

**R1 - M4 is fixed only on the injected-fault path (High).**
`write_line`'s real path (`None` arm) still does `write_all ... sync_data
?` with no `rollback_or_poison`. Probe `r1_real_write_fault_is_not_rolled_
back` sets `RLIMIT_FSIZE` (SIGXFSZ ignored): the append fails with `Io`, a
40-byte fragment of the `CommandBegin` line stays on disk, the handle is
not poisoned, the next append lands behind the fragment, and reopen fails
`Integrity`. Decision 10 ("a failed write is rolled back ... or the handle
is poisoned") is therefore not true for production faults. Minimal fix:
in the `None` arm, on any error call `rollback_or_poison(state, len_before)`
before returning the `Io` error. Test: the RLIMIT_FSIZE probe (or a
read-only fd through a test constructor) asserting rollback to
`len_before` and a clean reopen.

**R2 - a restart while `BroadcastSent` erases uncertainty (High; the
parent's explicit "crash after BroadcastSent before Unknown" item).**
Probe `r2_restart_in_broadcast_sent_then_rejected_releases`: sign,
broadcast, drop, reopen (tx still `BroadcastSent`, `ever_unknown = false`),
`note_rejected` -> inputs released, the same outpoint is re-spent by the
next command. The first send may have been accepted. Minimal fix that keeps
replay determinism: `reopen_inner` appends a `SessionStarted` record after
replay; committing it moves every tx in `BroadcastSent` to `Unknown`
(attempt +1, reason "session restart"; `ever_unknown = true`). A rejection
after the restart then degrades, and the retry send counts against the
cap. Test: R2 inverted (balance stays 0, `c2` fails).

**R12 - no `(Reorged, Confirmed)` edge for canonical re-inclusion
(Medium).** After displacement a tx is normally re-mined by the new chain
from the mempool without any wallet action. The core refuses
`record_confirmed` from `Reorged` (probe `r12_...`); the only route is a
resubmission whose node answer for an already-chained tx is an error, and
recording that as `rejected` makes a canonical transaction terminal
`Rejected` (inputs `PossiblySpent`, never recoverable). Fix: admit
`(Reorged, Confirmed)` as a canonical-observation edge (confirmation is
block evidence, not a send; `BroadcastSent` stays `ReorgResubmit`-only).
Also the `note_rejected` wrapper lacks the M7 finish guard (it commits
`Rejected` then errors "finished twice" on a reorg-cycled command);
apply the same guard.

**R6 - M3 enforcement is orchestrator-dependent (Medium).**
`validate_allocation` is a public helper that `provision` does not call;
probe `r6_...` provisions two configs with overlapping network-wallet
ranges and two bootstrap hosts, both succeed and derive identical scripts.
"Structurally impossible for validated allocations" holds, but nothing in
the API forces validation. Minimal type-state fix:
`validate_allocation(&[ProvisionConfig]) -> Result<ValidatedAllocation>`
and `provision`/`reopen` take `(&ValidatedAllocation, driver_id)`; record
an allocation digest in the header and verify it on reopen. Adapter
conflict 10 then becomes moot.

## 2.5 Precision items (small; do not expand the core)

- Report section 2 still says "the first reservation always builds";
  `wallet.rs` has the correct "builds, or fails closed with the reservation
  released" wording. Align the report (NotConverged exists and releases).
- Report section 5: "exactly one line vs the frozen baseline" should read
  "vs the prior T1a candidate"; vs the LP baseline it is the 17-line block.
- Dust ("strictly below") and the base58-vs-bech32 wording are now correct.
- Test-count note is arithmetically right (57 distinct incl. the ignored
  child; 56 distinct passing).

## 2.6 Normative packaging (must land before merge)

- `docs/SPEC.md` frontmatter was edited directly. `docs/MERGE-QUEUE.md`
  and `docs/changelog/README.md` require a fragment
  `docs/changelog/2026-11-08-traffic-wallet-core-rework.md` containing the
  bullet; the merge session splices it. Move the bullet to the fragment and
  restore `docs/SPEC.md` to the baseline bytes in the candidate. This
  matters doubly here: the candidate's `SPEC.md` is the old frozen-baseline
  file and lacks the primary's current T0 (`3304dffa`), AP-3a and T4
  frontmatter entries, so transferring it would regress the primary. No
  whole-SPEC transfer.
- The decision record (12 items, lab scope) is adequate once the R-items
  above are folded in (decision 10 wording; a decision for `SessionStarted`
  and the `(Reorged, Confirmed)` edge).
- Journal header v2 is a hard precondition on T0: the approved T0 (`3304`)
  still carries `history_version: 1` / the v1 `TrafficHistoryHeader`. The
  core correctly refuses v1. The T0 v2 amendment is a separate follow-up
  packet that must land before the first T1 journal write; it is not part
  of this candidate and is not approved here. `Invalidated` is now core;
  `Dropped`/`Expired` remain an exhaustive adapter mapping for later.

## 2.7 Disposition

**REQUEST_CHANGES (bounded).** Required before approval: R1 (real-fault
rollback/poison), R2 (`SessionStarted` restart uncertainty), R12
(`(Reorged, Confirmed)` edge + `note_rejected` guard), R6 (type-state
allocation enforcement), the changelog fragment / SPEC.md restoration
(2.6), and the two report precision lines (2.5). Everything in 2.3 is
approved as fixed and need not be re-derived; a diff-scoped re-review of
these items with their named tests suffices. No T1b/T2-L/T3 live proof,
Ark, node-profile or T0 v2 amendment is approved by this review.

Evidence: `/tmp/opencode/T1a-fable-probe-r1/{Cargo.toml,src/lib.rs,result.log}`,
`/tmp/opencode/T1a-fable-r1-tests.log`. The original probe crate and
manifests were left untouched.

---

# Part 3 - Rework r2 re-review (same reviewer, 2026-11-08)

Candidate: same worktree, freeze `/tmp/opencode/T1a-fix-r2-manifest-full.json`.
Scope: Part 2 items R1, R2, R12, R6 and the packaging items (2.5/2.6).
Nothing approved in Part 2 section 2.3 was re-derived.
Disposition: **REQUEST_CHANGES (bounded to two small items, 3.4)**; R1,
R2, R6 and the packaging are closed.

## 3.1 Snapshot and delta (actual bytes)

| Quantity | Parent freeze | Recomputed |
|---|---|---|
| file count | 388 | 388 |
| full snapshot | `ed0d100a9ef44d97d6cbb4df0206839cd3f47adb2d5f568943fd25366e4004b0` | identical |
| code snapshot | `47158c2fa2e2964b1fa5dbbca0113e747193b228ea82db179495c6122c23c5c1` | identical |
| r1 manifest | `e17889b5...` preserved | identical |

r1 -> r2: **1 added** (`docs/changelog/2026-11-08-traffic-wallet-core-rework.md`),
**0 removed**, **11 modified** (`Cargo.lock`, 7 crate files, `docs/SPEC.md`,
decision record, report). `docs/SPEC.md` is byte-identical to the LP
baseline (`5de914d7...`), so vs the LP baseline the shared-file delta is
now only `Cargo.toml` (+1 member) and `Cargo.lock` (one 18-line
`spc-lab-txd` block, now with `rustix` and dev-only `libc`; no pin
changed). The changelog fragment is one bullet in the frontmatter format.
The report's lock wording ("+1 vs the prior candidate / 18-line block vs
the LP baseline") and the "builds or fails closed with the reservation
released" wording are now correct.

## 3.2 What was run (disk policy observed)

Single invocation under nonblocking `flock
/tmp/opencode/silent-pool-heavy-build.lock`; 95 GiB available after
acquiring the lock; existing `/tmp/opencode/T1a-target`, `-j2`; 95 GiB
after. `cargo test -p spc-lab-txd`: **67 passed, 2 ignored** (28 + 25 + 6
+ 5 + 3; the R1 child reports its own `1 passed`). 66 distinct test
functions (28 + 24 + 2 child modes + 5 + 4 + 2 + the RIPEMD vector test
once) - the author's count is correct. Log:
`/tmp/opencode/T1a-fable-r2-tests.log`. Adversarial probe crate
`/tmp/opencode/T1a-fable-probe-r2/` (6 tests, `result.log`): four `ok_`
probes confirm the fixes; two reproduce the remaining items. The frozen
r1 probe was left untouched (its two `provision(&cfg, dir)` calls no
longer compile - that is the R6 enforcement, as the author states).

## 3.3 Closed

- **R1** (`ok_s1`, in-process `RLIMIT_FSIZE` with `SIGXFSZ` ignored):
  the production `None` arm now shares `rollback_or_poison`; the failed
  `CommandBegin` leaves the file at exactly the pre-fault length (no
  fragment), the `Io` names the rollback, the same command then signs
  normally, and the reopen is clean with the tx present once. Code:
  `write_line` routes every append failure (real or injected) through one
  recovery; `rollback_or_poison` poisons on a failed `set_len`/sync.
- **R2** (`ok_s2`): `reopen_inner` appends `SessionStarted` only after
  the lock, replay and all binding checks; its commit moves every
  `BroadcastSent` tx to `Unknown` with `ever_unknown = true` and
  `unknown_retries + 1` **without** touching `retry_sends`; three
  crash-loop reopens leave `unknown_retries = 1`; the full retry budget
  (3 sends) remains and the cap then binds; a later rejection keeps the
  coins `PossiblySpent`. A failed session append fails the reopen (no
  usable handle).
- **R6** (`ok_s5`): `ValidatedAllocation` has private fields, no
  `Deserialize`, and only `validate_allocation` as a constructor;
  `provision`/`reopen` take the token plus a driver id that must be in the
  allocation; the canonical digest is persisted in the header and checked
  on reopen **before** the session record (file length unchanged on
  `AllocationMismatch`, `CapsMismatch` and the driver mismatch; caps are
  reported first as documented); overlapping ranges or two bootstrap
  hosts cannot produce a token. Nit (non-blocking): the digest relies on
  `serde_json` map ordering (no `preserve_order` anywhere in the lock
  today); serializing an explicit struct would make that independent of
  future feature unification.
- **R12, re-inclusion edge** (`ok_s6`, candidate `r12_*`): `(Reorged,
  Confirmed)` is legal; a re-inclusion in a new block restores exactly one
  generation (the child becomes `Reorged`, its own outputs stay
  invalidated until its own re-inclusion); replay reproduces it. The
  candidate's test reaches the proof without inventing an RPC acceptance.
- **Packaging**: fragment present, SPEC.md restored, decision record
  carries the r2 semantics (SessionStarted, re-inclusion edge, token,
  real-fault rollback, wrapper guard).

## 3.4 Remaining (two concrete items, both small)

**S3 - the "never name the displaced block" rule refuses a legitimate
reorg back (Medium; funds frozen).** Probe `s3_reorg_back_to_same_block_
cannot_be_recorded`: parent confirmed in B10, child in B11;
`ChainObserved(tip 0x20 @11, displaced [B10, B11])` -> parent `Reorged`,
child `Invalidated`; the original branch then re-wins
(`ChainObserved(tip 0x12 @12, displaced [0x20])`), so B10 and B11 are
canonical again. `record_confirmed("p", B10, 10)` is refused
("re-inclusion names the displaced block"); `note_rejected` refuses
(finished); the journal-level rejection would terminalise a canonical tx.
Parent stays `Reorged`, child `Invalidated`, balance 0, no legal record
can express the canonical truth. Partition/heal is a shipped lab
scenario, so this is reachable. The displaced identity is not permanently
invalid; it is invalid only while the displacement is the latest chain
evidence. Minimal fix: store the journal `seq` of the `Reorged`
transition (`reorged_at_seq`) and the seq of the last committed
`ChainObserved`; refuse the same-block re-inclusion only when no
`ChainObserved` has been committed after the displacement (stale
evidence); any newer chain observation makes the same-hash confirmation
admissible as canonical evidence (the explicit `record_reorged` path
without any chain observation stays refused). Deterministic from the
record stream. Test: S3 inverted (parent `Confirmed` in B10 after the
reorg-back observation, child `Reorged`, then child re-confirmed in B11).

**S4 - `note_rejected` drops genuine rejections of finished commands
(Low).** Probe `s4_...`: after a reorg and a resubmit, the node's genuine
`bad-txns-inputs-missingorspent` cannot be recorded through the wrapper
(refused before any record, `last_seq` unchanged, tx left
`BroadcastSent`); only the journal-level `record_rejected` records it
(correctly degrading to `PossiblySpent`). The wrapper's refusal message
describes the "already in block chain" case, but it blocks every code
without inspection. Minimal fix: mirror `note_confirmed`/`note_conflicted`
- always `record_rejected`, guard only the finish. The "already known /
already in block chain" node answer is a driver mapping concern (T1b maps
it to a confirmation observation with block evidence, never a rejection);
the core must not infer canonicality from the error string, and it does
not.

## 3.5 Disposition

**REQUEST_CHANGES (bounded)**: S3 and S4 only, with the named tests.
R1, R2, R6, the re-inclusion edge in a new block, and the packaging are
approved as fixed. A diff-scoped re-check of the two items is sufficient.
Still outside this approval: the T0 `history_version` 2 / allocation-digest
header amendment (separate first-write gate), any T1b/T2-L/T3 live proof,
node profile, or Ark readiness.

Evidence: `/tmp/opencode/T1a-fable-probe-r2/{Cargo.toml,src/lib.rs,result.log}`,
`/tmp/opencode/T1a-fable-r2-tests.log`.

---

# Part 4 - Rework r3 re-check (S3/S4 only; same reviewer, 2026-11-08)

Candidate: same worktree, freeze `/tmp/opencode/T1a-fix-r3-manifest-full.json`.
Scope: Part 3 items S3 and S4 and the packaging text. Nothing cleared in
Parts 2-3 was re-derived.
Disposition: **APPROVE (bounded, exactly the T1a core as frozen below)**.

## 4.1 Snapshot and delta (actual bytes)

| Quantity | Parent freeze | Recomputed |
|---|---|---|
| file count | 388 | 388 |
| full snapshot | `65a16584fb89429ef2f59e88285fbfdd587a5892bd51c3f3489e2c7ad8eb3748` | identical |
| code snapshot | `a8d28ceac3a9bd63fc72886ca5040544431c1235b31a98e902290ee4ff834628` | identical |
| r2 manifest | `098cc565...` preserved | identical |

r2 -> r3: **0 added, 0 removed, exactly 7 modified** (`journal.rs`,
`wallet.rs`, `adapter.rs`, `tests/fable_regressions.rs`, the changelog
fragment, the decision record, the report). `Cargo.lock`, `Cargo.toml` and
`docs/SPEC.md` are unchanged from r2 (SPEC.md still byte-identical to the
LP baseline). The 597 KB patch is a full git-HEAD diff and was not used as
the review delta.

## 4.2 What was run (disk policy observed)

Single invocation under nonblocking `flock
/tmp/opencode/silent-pool-heavy-build.lock`; 94 GiB available after the
lock and after the run; existing `/tmp/opencode/T1a-target`, `-j2`.
`cargo test -p spc-lab-txd`: **68 passed, 2 ignored** (28 + 26 + 6 + 5 +
3; R1 child `1 passed`), 67 distinct functions - the author's counts are
correct (`/tmp/opencode/T1a-fable-r3-tests.log`). Probes:
- the frozen r2 probe crate re-run unchanged against r3
  (`/tmp/opencode/T1a-fable-probe-r2/result-vs-r3.log`): the four `ok_`
  fixes still pass; `s3_...` and `s4_...` now **fail** - they asserted the
  r2 bugs, which are gone;
- a new scratch crate `/tmp/opencode/T1a-fable-probe-r3/` (4 tests, all
  pass, `result.log`), described below.

## 4.3 S3 - same-block re-inclusion after a reorg back (closed)

Code: `TxEntry.reorged_from_block` and `displaced_at_seq` are set from the
committing record's `seq` on the explicit `Reorged` commit, on a
`ChainObserved` displacement, and on a cascade `Confirmed -> Invalidated`
(the descendant keeps its own confirmed block and the displacing record's
seq); the promotion `Invalidated -> Reorged` only sets the state, so the
original displacement identity and seq survive; `Confirmed` clears both.
`JournalState.last_chain_observed_seq` is the recency witness. The
same-hash confirmation is refused only when no `ChainObserved` is newer
than the displacement (message names "stale evidence"); any newer chain
observation admits the supplied block evidence. All of it is derived from
`record.seq`, so replay reproduces it.

Probes: `ok_t1` is my exact Part 3 counterexample extended with a
grandchild (B10/B11/B13 displaced by a competing tip, stale refusal, reorg
back via a fresh `ChainObserved`, parent then child then grandchild
re-confirmed in their original blocks, payer balance exactly restored,
summary identical after reopen). `ok_t2`: the explicit `record_reorged`
path with no chain observation stays refused for the same block while a
new-block re-inclusion (the r2-verified R12 case) is unchanged.
`boundary_t3` records the bounded contract: a newer chain observation that
is merely a tip advance of the competing chain also lifts the guard - the
rule is a staleness guard, not a canonicality proof; canonicality is the
supplied block evidence from T3's canonical observation. That is the
contract I asked for in Part 3 and it is what the decision record and the
fragment now state. No RPC-string inference, no resubmit bypass.

## 4.4 S4 - `note_rejected` (closed)

`note_rejected` now always calls `record_rejected` (the journal's
transition table is the sole legality gate) and guards only the finish,
symmetric with `note_confirmed`/`note_conflicted`. Probe `ok_t4`: a genuine
`bad-txns-inputs-missingorspent` after reorg + resubmit on a finished
command is recorded (`last_seq + 1`, exactly one record), the original
`confirmed` finish outcome is retained, the inputs degrade to
`PossiblySpent` (no second spend possible), replay matches, and an illegal
rejection (from `Confirmed`) still fails closed. The "already known / in
mempool / in block chain" node answers are left to T1b to map onto a
confirmation observation with real block evidence; the core parses nothing.

## 4.5 Packaging

Decision record, changelog fragment and report revision 3 describe the
stale-evidence rule and the wrapper guard truthfully; test counts and the
file table were refreshed. Report-only edits after the final test run;
crate sources are byte-identical to the tested tree (manifest hashes).

## 4.6 Disposition

**APPROVE - bounded to the T1a core exactly as frozen at code snapshot
`a8d28ceac3a9bd63fc72886ca5040544431c1235b31a98e902290ee4ff834628`**
(crate `crates/spc-lab-txd`, its decision record and changelog fragment,
the +1 workspace member line and the one `spc-lab-txd` lock block). All
findings from Parts 1-3 are closed with named regressions and confirmed by
independent probes. Non-blocking nits already recorded: allocation digest
via `serde_json` map ordering (Part 3), lookahead-bounded owner tagging in
`checked_destination` (Part 2).

Not approved by this review: the T0 `history_version` 2 / allocation-digest
header amendment (a separate packet that gates the first T1 journal
write), T1b driver/runtime, T2-L bootstrap, T3 live proof, any node
profile, mempool/RPC behaviour, or Ark readiness. The golden fixture and
all probes are conformance vectors over synthetic funding, not runtime
evidence.

Evidence: `/tmp/opencode/T1a-fable-probe-r3/{Cargo.toml,src/lib.rs,result.log}`,
`/tmp/opencode/T1a-fable-probe-r2/result-vs-r3.log`,
`/tmp/opencode/T1a-fable-r3-tests.log`.
