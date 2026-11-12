# 2026-11-08 - Traffic wallet core rework (journal v2 header, fail-closed lifecycle)

## Scope

This is a bounded lab traffic-wallet decision (traffic plan packet T1a,
crate `spc-lab-txd`), implementing the dispositions of the 2026-11-08
Fable review (`docs/reviews/2026-11-08-traffic-wallet-fable.md`,
findings H1-H5, M1-M7). It is not a pool accounting, payout, SPc/Ark
or production-network decision. The T0 traffic contract types are
approved and frozen (snapshot sha256 `3304dffa84c32d546ee783bd15beec0
9bbfe5fa6755a71c9c5721bc6241d41a8`) and are referenced read-only; this
crate imports nothing from them.

## Decisions

1. **Journal header version 2 (T0 coordination mandate).** The first
   record of every journal (sequence 0) is a `Header` with exactly:
   `format_magic = "SPC-LAB-TRAFFIC-JOURNAL"`, `history_version = 2`,
   `protocol = "spc-lab-tx-journal/2"`, `network`, `key_mode`,
   `derivation_version = 2`, `run_id`, `driver_id`,
   `run_seed_fingerprint`, `allocation_digest`, `caps` and
   `created_unix_ms`. The T0-approved v1 `TrafficHistoryHeader`
   `{history_version, run_id, driver_id, protocol}` cannot carry the
   network, key-mode, derivation-version, fingerprint, allocation and
   caps fields this core's fail-closed reopen mandates, so v2 is a
   hard precondition for the first T1 journal write: T0 adopts the
   matching v2 type and the allocation-digest semantics
   (`allocation_digest` is the SHA-256 of the canonical sorted-key
   JSON encoding of the validated allocation - the shared run seed,
   run id, network and caps, plus per actor the driver id,
   bootstrap-host flag, both index ranges, lookahead and fee policy -
   and is verified on every reopen before any mutation; a mismatch is
   `AllocationMismatch`, fail closed). Journals
   shaped like the pre-rework v1 (no `format_magic`) are refused on
   reopen with a hint pointing at the rework; there is no v1
   compatibility or migration path. The private key file stays at
   `history_version: 1`, extended with the new binding fields.
2. **Uncertainty never releases inputs (H1).** A `Rejected` record
   releases the command's reserved inputs to `Available` only when the
   durable history proves the payment was never uncertain: never
   `unknown`, never RPC-accepted, never reorg-resubmitted. Otherwise
   the claim degrades conservatively to `PossiblySpent`. The decision
   reads durable per-transaction counters (`ever_unknown`,
   `ever_accepted`, `reorg_resubmits`), not live state, so crash
   replay takes the same decision. The transition table includes
   `(Unknown, Rejected)`: a node may answer a rejection for the send
   whose outcome was unknown, and that path degrades. A restart while
   a send is in flight sets the same flag durably (decision 5), so a
   rejection after a restart degrades identically.
3. **Derived invalidation cascade (H2).** After every committed
   record the journal runs a cascade to a fixpoint, deterministic from
   the record stream alone: (a) own-transaction funding is backed only
   while its creating transaction is `Confirmed` (a reorged, rejected
   or conflicted creator invalidates the funding, remembering the
   prior status; a `Spent` prior restores conservatively to
   `PossiblySpent`); (b) a transaction with any invalidated input is
   `Invalidated`, remembering its prior state (a `Confirmed` prior
   restores to `Reorged`, the conservative resubmit path) once all of
   its inputs are non-invalidated again. Explicit records never touch
   an `Invalidated` transaction; recovery proceeds generation by
   generation through `reorg_resubmit`. A canonical re-inclusion of a
   reorged transaction in the new chain is recordable from `Reorged`
   (`(Reorged, Confirmed)` is a legal edge: a confirmation is block
   evidence, not a send - only `(Reorged, BroadcastSent)` stays
   resubmit-only), but it must not name the displaced block itself on
   STALE evidence (the journal remembers the displaced `(block hash,
   height)` AND the displacement moment - the seq of the displacing
   record - plus the seq of the last committed `ChainObserved`, both
   derived deterministically from the record stream; naming the
   displaced block while no chain observation is newer than the
   displacement is an `IllegalTransition`, fail closed, and the
   explicit `record_reorged` path without any chain observation
   always stays refused). A chain observation NEWER than the
   displacement - the chain reorged back and the block is canonical
   again - makes the same-hash confirmation admissible as canonical
   evidence (S3). Re-inclusion restores the cascade one generation at
   a time: a descendant displaced to `Invalidated` returns to `Reorged`
   (the conservative reconfirm-or-resubmit path) keeping its ORIGINAL
   displacement identity - its own displaced block and the original
   displacement seq, never overwritten by the restoring parent's
   confirmation - and re-including the child completes the recovery.
4. **Retry caps bound sends, not evidence (H3).** Caps are typed
   (`JournalCaps { unknown_retry_sends: 0..=3, reorg_resubmits:
   1..=3 }`), persisted in the v2 header and revalidated on every
   reopen (a differing expectation is `CapsMismatch`, fail closed). A
   cap of N on unknown retries admits at most N retry SENDS (N+1 total
   sends of the same bytes); recording unknown OUTCOMES is never
   capped. Admission (not outcome recording) is the capped operation
   because capping honest evidence reporting would corrupt the
   journal's provenance purpose: an operator must always be able to
   record what was observed, while the core must bound how often the
   same bytes leave the process. A send committed from the `unknown`
   state increments the durable `retry_sends` counter. `Reorged` has
   no `(Reorged, BroadcastSent)` edge: re-entry after a reorg is
   exclusively `ReorgResubmit`, which requires every original input
   proven unspent (only `Available`/`PossiblySpent` claims qualify) and
   is capped by `reorg_resubmits`. A process restart is not a retry
   send: the restart-induced unknown (decision 5) consumes no retry
   budget and inflates no attempt counter (crash-looping reopens
   cannot amplify the counters), while the caps still bind across
   the restart once a retry is actually sent.
5. **One journal, one owner (H4).** An exclusive non-blocking kernel
   flock is taken before any replay or truncation, on create and on
   reopen; a second live handle fails with `HeldByAnotherOwner`
   without mutating (the check precedes every read-side effect). Owner
   exit releases the lock; the parent directory is fsynced on create
   and the 0600 mode is propagated. Reopen is a **writer** operation:
   after replay and all binding checks (caps, then allocation digest,
   then driver id) it appends a durable
   `SessionStarted { seq, restarted_unix_ms }` record under the same
   exclusive lock - a restart while a transaction is in
   `BroadcastSent` is journaled uncertainty (the transaction becomes
   `Unknown` with `ever_unknown` set durably; only `BroadcastSent`
   transactions move, never the retry counters), there is no
   read-only auditor handle, and a failed session write fails the
   reopen (or poisons the handle) rather than admitting an unlogged
   restart.
6. **Funding observations are verified claims (H5).** A
   `FundingObservation` carries the outpoint, amount, the owner role
   and its derivation locus `(chain, index)`, the source and the
   provenance context - and **no script**: the wallet set derives the
   script at the locus and the journal compares it byte-for-byte with
   the evidence. Own-transaction funding requires the creating
   transaction to be journaled and `Confirmed`, with the decoded vout
   matching the observation exactly (script and amount). Bootstrap
   coinbase and canonical-block observations must carry their
   `CanonicalBlock { block_hash, height, coinbase }` context. Coinbase
   funding counts as available only at 100 confirmations of the
   observed tip (`tip >= height + 100`); with no observed tip, coinbase
   maturity is unprovable and the entry is treated as immature (fail
   closed). Only the canonical re-observation of an invalidated entry
   (same identity, fresh block evidence) is admissible as a duplicate.
7. **Digest covers payer and category (M1); decoded inputs are the
   reserved set (M6).** `begin_command` computes the payload digest
   over the canonical `RequestRecord` encoding including the payer
   role and the payment category, so the same command id with a
   different payer or category is a durable conflict, before and after
   signing. A `SignedTx` record whose claimed inputs match the
   reservation but whose decoded bytes spend something else (or spend
   a duplicate input) is refused as a tamper.
8. **Spending resolves keys by direct derivation (M2).** Every
   spending path resolves the input key at the outpoint's recorded
   `(chain, index)` - never by script scanning - so the script
   lookahead bounds address derivation for external display only and
   never bricks a role's change chain.
9. **Allocation validation is core and type-state enforced (M3, R6).**
   `ProvisionConfig` carries explicit `RangeInclusive` index ranges
   per wallet kind plus a `hosts_bootstrap_funding` flag.
   `validate_allocation` checks a whole multi-actor allocation
   (non-empty; exactly one bootstrap-funding host; one shared run
   seed, run id and network; equal caps across actors - the caps are
   covered by the digest; unique driver ids; pairwise-disjoint
   per-kind index ranges; valid ids and caps; no empty role set) and
   returns an opaque `ValidatedAllocation` token with private fields
   and no deserialization path - the token's only constructor is
   `validate_allocation` itself. Production `provision` and `reopen`
   take `(&ValidatedAllocation, driver_id, dir)`: the orchestrator
   can no longer skip whole-allocation validation, because the token
   IS the proof, and the token's canonical digest is persisted in the
   v2 header (decision 1) and verified on every reopen before any
   mutation. The layering keeps the core free of cross-actor state
   while making a double signer structurally impossible;
   config-only conformance helpers exist for the frozen test probes
   only (test-only by name and doc).
10. **Write-path atomicity (M4).** Every append is flushed and
    `sync_data`d before the proven-durable length advances. A failed
    write is rolled back (`set_len` + `sync_data`) and the fault is
    consumed; if the rollback cannot be proven the handle is poisoned
    and every further mutation fails closed with `Poisoned`. A torn
    final line is truncated on reopen. The production write path and
    every injected fault flow through one shared rollback-or-poison
    recovery, so a REAL fault (not an injected one) behaves
    identically - proven by an `RLIMIT_FSIZE` file-size limit with
    `SIGXFSZ` ignored, exercised in a child process (the parent's
    limit is untouched): the append fails with an `Io` naming the
    rollback, the file is durably restored to the exact pre-fault
    length (no fragment survives), the next append lands clean and
    the reopen is consistent.
11. **Key derivation v2 and id vocabulary (M5).** Traffic keys derive
    as `HMAC-SHA256(key = run_seed big-endian, msg = run_id || 0x00 ||
    role_label)` under key mode `lab_deterministic_run_seed_v1`,
    derivation version 2 (v1 lacked the run-id domain: the same seed
    replayed in another run derived the same wallet keys). `run_id`
    and `driver_id` are validated against the T0 id vocabulary
    (non-empty, at most 128 bytes, `[A-Za-z0-9-_.:/@]`).
12. **Late observations are journaled (M7, S4).** The `note_confirmed`,
    `note_conflicted` and `note_rejected` wrappers always record the
    observation; only the command finish is guarded
    (`command_phase != Finished`). A late confirmation or conflict
    after a reorg cycle is journaled without error; an illegal
    transition still fails closed, and the raw journal API remains
    non-idempotent (the guard lives in the wrapper). `note_rejected`
    mirrors them exactly (round 3 reverses the round-2 error-first
    guard, which silently dropped genuine rejections): a genuine
    rejection of an already-finished command - e.g. a
    `bad-txns-inputs-missingorspent` node answer arriving after a
    reorg resubmission - is always journaled (the journal-level
    transition rules are the sole legality gate; a resubmitted
    payment degrades its inputs to possibly-spent, never releases
    them), exactly one record advances, and the command keeps its
    original terminal outcome with no second finish. Classifying node
    answers is the driver's (T1b's) concern, never the core's: an
    "already known / already in block chain" answer is a confirmation
    observation, not a rejection of the funding claim - T1b maps it
    to `note_confirmed` with the canonical block, and the core never
    parses the rejection code string to infer canonicality.

## Consequences

Every review counterexample has a named regression test in
`crates/spc-lab-txd/tests/fable_regressions.rs` (H1-P1/P2 plus the
direct unknown-then-rejected variant, H2-P3 three-generation cascade
with full recovery, H3-P8/P9, H4-P6 in-process and cross-process,
H5-P5/P10, M1-P4, M2-P12, M3-P13, M4 fault injection, M7-P11); the
round-2 findings are covered the same way (R1 by the
`r1_real_write_fault_rolls_back_or_poisons` parent with its
`r1_real_write_fault_child` child mode, R2 by the three `r2_restart_*`
/ `r2_crash_loop_*` tests, R12 by the `r12_*` re-inclusion tests, R6 by
`r6_production_provision_requires_a_validated_allocation`); the
round-3 findings likewise (S3 by `s3_reorg_back_to_same_block_is_
canonical_after_a_fresh_chain_observation` - parent, child and
grandchild, stale refusal before the reorg-back observation,
generation-by-generation restore in the original blocks, replay
identity - and S4 by `s4_note_rejected_records_genuine_rejection_and_
guards_only_the_finish`, which replaces the round-2 R12
`note_rejected` test whose error-first expectation the round-3 fix
reverses); the full
suite is 67 distinct test functions (65 passing in-process plus the two
ignored child modes exercised by their parents; 68 passing parent
executions), all green. The derivation change (decision
11) changes every derived key, so the golden fixture was regenerated
under derivation v2 (txid
`10636a5abf76021132012a01f34c22c42d2f302507e210ed50514cf60f446211`;
the raw-bytes constant lives in `tests/common/mod.rs` and is verified
by the independent parser/verifier path). T0 must adopt the v2 header
type and allocation digest before the first T1 journal write (decision
1); the adapter contract in `crates/spc-lab-txd/src/adapter.rs` records
the field-by-field handoff, the resolved conflicts (1, 3, 5, 10) and
the type-state allocation enforcement (9). The SPEC changelog entry
lives in the merge fragment
`docs/changelog/2026-11-08-traffic-wallet-core-rework.md` (the worktree
`docs/SPEC.md` stays at the frozen baseline bytes). No pool accounting,
payout, SPc/Ark or production-network behavior changes.
