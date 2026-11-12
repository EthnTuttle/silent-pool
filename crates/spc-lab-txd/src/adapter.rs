//! Adapter contract between this core and the T0 traffic types (traffic
//! plan section 14, packet T1a; T0 session
//! `ses_e4604a9d0ffeHEKwnwhfH0S61J`, worktree `lab-traffic-contract`).
//!
//! **Status of the T0 types: APPROVED SCHEMA/VOCABULARY, FROZEN
//! (snapshot sha256 `3304dffa84c32d546ee783bd15beec09bbfe5fa6755a71c9c
//! 5721bc6241d41a8`), usable READ-ONLY by this core.** This core still
//! imports nothing from it: every public API takes **domain
//! arguments**, and the table below is the typed mapping the parent
//! applies - no second signer, no second journal, no type duplication.
//!
//! ## T0 coordination mandate: journal header version 2
//!
//! The T0-approved v1 `TrafficHistoryHeader`
//! `{history_version, run_id, driver_id, protocol}` **cannot carry the
//! fields this core's fail-closed reopen mandates** (network, key mode,
//! derivation version, run-seed fingerprint, retry caps). This core
//! therefore writes a **v2 header** - the first record of every
//! journal, at sequence 0 - with exactly this shape:
//!
//! ```json
//! {"record":"header","seq":0,
//!  "format_magic":"SPC-LAB-TRAFFIC-JOURNAL",
//!  "history_version":2,
//!  "protocol":"spc-lab-tx-journal/2",
//!  "network":"regtest",
//!  "key_mode":"lab_deterministic_run_seed_v1",
//!  "derivation_version":2,
//!  "run_id":"...","driver_id":"...",
//!  "run_seed_fingerprint":"...",
//!  "allocation_digest":"<64-hex sha256>",
//!  "caps":{"unknown_retry_sends":3,"reorg_resubmits":3},
//!  "created_unix_ms":0}
//! ```
//!
//! (Field names as serialized; `HeaderFacts` is the typed replay
//! view.) T0 must adopt the **matching v2 type and allocation digest
//! semantics** before the first T1 journal write: `allocation_digest`
//! is the SHA-256 of the canonical sorted-key JSON encoding of the
//! validated allocation (the shared `run_seed`, `run_id`, `network`
//! and `caps`, plus per actor - sorted by driver id - the driver id,
//! bootstrap-host flag, both index ranges, lookahead and fee policy),
//! computed by [`crate::wallet::ProvisionConfig::validate_allocation`]
//! and verified on every reopen **before any mutation or session
//! record** (`AllocationMismatch`, fail closed). Reopen itself is a
//! **writer** operation: after replay and all binding checks it
//! appends a durable `SessionStarted { seq, restarted_unix_ms }`
//! record under the exclusive lock - there is no read-only auditor
//! handle, and any external replayer must tolerate session records.
//! Journals shaped like the pre-rework v1 (no `format_magic`) are
//! refused on reopen with a hint pointing at the rework; there is
//! no v1 compatibility path. The key file stays at
//! `history_version: 1` with the new binding fields.
//!
//! ## Verified matches (frozen T0 types vs this core)
//!
//! These were checked field-by-field against the frozen T0 source:
//!
//! | T0 (frozen) | This core | Status |
//! |---|---|---|
//! | `SEED_DOMAIN_BOOTSTRAP_FUNDING` = `spc-lab/traffic/bootstrap-funding` | [`crate::keys::LABEL_BOOTSTRAP_FUNDING`] | byte-identical |
//! | `SEED_DOMAIN_NETWORK_WALLET_PREFIX` = `spc-lab/traffic/network-wallet/` + index | [`crate::keys::WalletRole::derivation_label`] (`NetworkWallet`) | byte-identical |
//! | `SEED_DOMAIN_USER_PAYMENT_WALLET_PREFIX` = `spc-lab/traffic/user-payment-wallet/` + index | [`crate::keys::WalletRole::derivation_label`] (`UserPayment`) | byte-identical |
//! | `TRAFFIC_HISTORY_VERSION` = 1 | [`crate::journal::SUPPORTED_HISTORY_VERSION`] = 2 | **v2 mandate above** - the v1 header cannot carry the core's mandatory binding fields |
//! | history path `<run_dir>/private/traffic/history.jsonl` | [`crate::journal::TrafficJournal::path_in`] | identical |
//! | `fees.max_fee_sats_per_tx` ceiling 100000 | [`crate::builder::FeePolicy`] default `max_fee_sats_per_tx` | equal |
//! | `caps.reorg_resubmit_max` default 3 | [`crate::journal::DEFAULT_REORG_RESUBMITS`] (the default of the persisted [`crate::journal::JournalCaps`]) | equal (conflict 3 resolved) |
//! | `caps.max_unknown_retries` default 3 | [`crate::journal::DEFAULT_UNKNOWN_RETRY_SENDS`] (the default of the persisted [`crate::journal::JournalCaps`]) | equal (conflict 3 resolved) |
//! | `spend_types` P2PKH-only (v1 profile) | legacy `is_p2pkh()` gating in [`crate::builder::SignableUtxo::resolve_indexed`] and `build_and_sign` | identical |
//! | `bootstrap.mining_mode` native-only (`rpc_generate` rejected) | core never mines; T2-L owns the native loop | consistent |
//!
//! ## Command payload mapping (T0 `commands.rs` -> this core)
//!
//! | T0 payload | This core | Notes |
//! |---|---|---|
//! | `SendUserPayment { user: IdentityId, destination_address, amount_sats, fee_rate_sats_vb }` | [`crate::wallet::TrafficWalletSet::checked_destination`] + [`crate::wallet::PaymentRequest`] | **the payload carries an explicit mandatory `destination_address`** (parent decision resolving the plan 6.6 vs 12 ambiguity); `checked_destination` verifies regtest, standard P2PKH/P2WPKH/P2TR outputs (T1-DEST) and dust **before any journal record or signature**, owner-tags only owned P2PKH scripts, and there is no implicit destination rule anywhere in this core - see conflict 6 |
//! | `PauseTxDriver { driver_id }` / `ResumeTxDriver { driver_id }` | [`crate::wallet::TrafficWalletSet::pause_sends`] / [`resume_sends`] (journaled barrier) | the journal records the controlling `command_id`; driver routing is T1b actor behavior (this core takes command-id strings) |
//! | `SetTxRate { driver_id, ordinary_tx_per_sec: f64 }` | none (pacing is T1b behavior) | f64 pacing never enters this core's money arithmetic, which stays u64/u128-only; per-payment `fee_rate_sats_vb` is an integer |
//! | `BootstrapFund { node_id }` | same journal + signer via [`crate::wallet::TrafficWalletSet::observe_funding`] with a [`crate::journal::FundingObservation`] (`source: BootstrapCoinbase`, `context: CanonicalBlock { block_hash, height, coinbase: true }`) | block-template loop is T2-L; the observation carries the owner's derivation locus `(chain, index)` and **no script** - the wallet derives the script (H5), and coinbase maturity gates availability on the observed tip |
//! | `RpcSignedTxPayload { raw_tx_hex }` | **not yet admitted** - see conflict 7 | T0 delegates full consensus decode to "T1's signing/journal domain", i.e. this core |
//!
//! ## Spec field mapping (T0 `spec.rs` -> this core)
//!
//! | T0 field | This core | Notes |
//! |---|---|---|
//! | `driver.actors[*].network_wallets` (1..=2 actors, total <= 256) + `user_payment_wallets` | [`crate::wallet::ProvisionConfig`] `network_wallets: RangeInclusive<u32>` / `user_payment_wallets: RangeInclusive<u32>` + `hosts_bootstrap_funding` | one `ProvisionConfig` (one wallet set, one journal) per actor/driver; the whole allocation must pass [`crate::wallet::ProvisionConfig::validate_allocation`] first - see conflict 5 (resolved) and 10 |
//! | `RunSpec.seed` | `ProvisionConfig::run_seed` | whole set derives through the per-role HMAC domains |
//! | `fees.rate_sats_vb { min, max }` within 1..=100 | [`crate::builder::FeePolicy`] `min_rate_sats_vb` / `max_rate_sats_vb` | T1b constructs the `FeePolicy` from the spec - see conflict 4 |
//! | `fees.max_fee_sats_per_tx` | `FeePolicy::max_fee_sats_per_tx` | direct |
//! | (no dust field in the spec) | `FeePolicy::dust_sats` (default 546) | dust policy stays core-owned unless the spec gains a field |
//! | `sizes.classes_vb [{ vb, mix }]` | journal `TxSigned.size_class: Option<String>` (currently `None`) + destination-amount choice when building `PaymentRequest`s | class mix drives T1b's request construction, not the builder |
//! | `caps.max_unknown_retries` (0..=3) / `caps.reorg_resubmit_max` (1..=3, default 3) | [`crate::journal::JournalCaps`] - provisioned into the header, revalidated on reopen (`CapsMismatch`) | conflict 3 resolved: the caps are scenario fields AND durable header facts |
//! | `pause_semantics` (no broadcast after ack) | journal barrier (`record_send_barrier` / `record_send_resumed`), later sends refused | drain loop is T1b |
//!
//! ## Conflicts the parent must reconcile at handoff
//!
//! 1. **Journal header shape - RESOLVED CORE-SIDE, T0 ADOPTION
//!    REQUIRED.** The v2 header mandate above is a hard precondition
//!    for the first T1 journal write: T0 adopts the matching v2 type
//!    and allocation digest semantics (decision record
//!    `docs/decisions/2026-11-08-traffic-wallet-core-rework.md` in this
//!    worktree). The T0 fields (`run_id`, `driver_id`, `protocol`)
//!    and the core fields (`network`, `run_seed_fingerprint`,
//!    `allocation_digest`, plus `format_magic`, `key_mode`,
//!    `derivation_version`, `caps`, `created_unix_ms`) are now one
//!    record; neither set can be dropped. v1-shaped journals are
//!    refused, never migrated.
//! 2. **Lifecycle vocabulary.** T0 `TxLifecycleState` has 12 states;
//!    this core's [`crate::journal::TxState`] now has 10
//!    (`Invalidated` is a core state since the H2 rework: the derived
//!    cascade state for transactions on invalidated funding, with
//!    recovery through reorg resubmission). Still missing in core:
//!    `Dropped` and `Expired` (later profile - never silent). Name:
//!    core `Signed` serializes as `"created_signed"`, T0 uses
//!    `created`. Terminality: T0 `is_terminal()` is
//!    `rejected | conflicted` (confirmed is *not* terminal - reorg
//!    displacement moves it); core `is_terminal()` includes `Confirmed`
//!    but encodes the `Confirmed -> Reorged` displacement as an explicit
//!    exception in `transition_legal`, so the observable transition
//!    table already matches - only the flag and the names differ.
//! 3. **Retry caps - RESOLVED.** The spec's `max_unknown_retries`
//!    (0..=3) and `reorg_resubmit_max` (1..=3) map onto the typed,
//!    persisted [`crate::journal::JournalCaps`]: provisioned into the
//!    v2 header, range-validated at provision, and revalidated on
//!    every reopen (a differing expectation is `CapsMismatch`, fail
//!    closed). Semantics (H3): the unknown cap bounds retry **sends**
//!    (N retries + the original = N+1 total sends of the same bytes);
//!    recording unknown **outcomes** is never capped, and outcome
//!    evidence is always admissible.
//! 4. **Fee band width.** The spec admits `rate_sats_vb` up to 100; the
//!    core `FeePolicy` *default* band is 1..=20. No hard conflict
//!    (`FeePolicy` is data), but T1b must build it from the spec rather
//!    than rely on the default.
//! 5. **Wallet population shape - RESOLVED CORE-SIDE.**
//!    [`crate::wallet::ProvisionConfig`] now carries explicit
//!    `RangeInclusive` index ranges per wallet kind plus a
//!    `hosts_bootstrap_funding` flag; the standard population is one
//!    config hosting everything, and a multi-actor deployment is
//!    expressed as several disjoint configs. See conflict 10 for the
//!    validation layering.
//! 6. **`SendUserPayment` destination (resolved by parent decision).**
//!    The plan 6.6 vs 12 ambiguity is settled: the payload carries an
//!    explicit mandatory `destination_address` alongside the source
//!    user, the exact amount and the fee rate - no implicit destination
//!    rule. This core already accepts actual destination scripts with
//!    exact requested amounts; the mapping is
//!    [`crate::wallet::TrafficWalletSet::checked_destination`], which
//!    rejects wrong-network addresses and non-P2PKH script classes
//!    BEFORE any journal record or signature exists (fail closed, no
//!    donation fallback), owner-tags set wallets, and leaves change to
//!    the payer's known change-chain script only. `IdentityId` still
//!    maps to `WalletRole::UserPayment { index }` at T1b.
//! 7. **`RpcSignedTxPayload` admission.** T0 routes a declared,
//!    pre-signed raw transaction "through the driver's standard state
//!    machine" and delegates full consensus decode to this core. The
//!    core has no public admit path for externally signed bytes yet;
//!    T1b/T3 need a fail-closed `admit_declared_tx`-style journal API
//!    (decode, raw-bytes <-> txid consistency, same lifecycle states,
//!    no input release) - recorded as an explicit remainder.
//! 8. **Error types.** T0's `TrafficError { code, detail }` is not
//!    shared with this core's typed errors; T1b maps core errors onto
//!    traffic error codes at the actor boundary. Nothing here changes.
//! 9. **AP-1a projection.** The projection hash lineage is a T0-side
//!    concern; this core does not consume the projection at all
//!    (scenario preflight only) and the independent journal/signature
//!    tests are unaffected. No action for the signer/journal.
//! 10. **Allocation validation layering (M3, R6 - ENFORCED
//!     CORE-SIDE).** Production provisioning and reopen are
//!     type-state enforced: [`crate::wallet::TrafficWalletSet::provision`]
//!     and `reopen` consume an opaque
//!     [`crate::wallet::ValidatedAllocation`] whose only constructor
//!     is [`crate::wallet::ProvisionConfig::validate_allocation`]
//!     over **all** actors' configs (non-empty, exactly one
//!     bootstrap-funding host, one shared run seed + run id + network,
//!     equal caps, unique driver ids, pairwise-disjoint per-kind index
//!     ranges, valid ids and caps, no empty role set). The orchestrator
//!     (T1b, driving the T0 actor list) can no longer skip
//!     whole-allocation validation: the token IS the proof, and its
//!     canonical digest is persisted in the v2 header and reverified on
//!     every reopen (conflict 1). The layering keeps the core free of
//!     cross-actor state while making a double signer structurally
//!     impossible. Test-only conformance helpers keep a config-only
//!     signature; they are excluded from production use by name and
//!     doc.
//!
//! ## Remainder owned by other packets (not implemented here)
//!
//! - **T1b (driver actor/runtime)**: the actor and control loop
//!   (`spc-lab-tx-control/v1` hello/ack exchange, capability registry -
//!   frozen EMPTY in the candidate, extended alongside behavior), the
//!   command-kind wiring (`CommandKind` additions under the serial merge
//!   contract), the broadcast loop with the drain barrier, RPC routing
//!   through the controller-mediated `RpcNodeControl` path, the
//!   identity-free `tx_traffic` evidence projection, calling
//!   `validate_allocation` before provisioning (conflict 10), and the
//!   conflicts above.
//! - **T2-L (bootstrap funding)**: the native block-template loop that
//!   funds traffic addresses and records
//!   `FundingObserved { source: BootstrapCoinbase }` entries with their
//!   canonical block context plus the distribution transactions (same
//!   journal, same signer).
//! - **T3 (e2e acceptance)**: node observation wiring
//!   (`observed_in_mempool` / `confirmed` / `chain_observed` reorg
//!   reconcile), the independent conservation oracle, the real
//!   invalidation/reacceptance case, and `RpcSignedTxPayload` admission
//!   (conflict 7).
//! - **AP-5**: Ark funding orchestration **through this same API** after
//!   real Ark addresses exist - never a second signer or journal.
//!
//! ## What this core already guarantees for those packets
//!
//! - **One journal, one owner (H4)**: an exclusive kernel flock is taken
//!   before any replay or truncation; a second live handle - in-process
//!   or across processes - fails with `HeldByAnotherOwner` without
//!   mutating; owner exit releases the lock.
//! - **Write-path atomicity (M4, R1)**: every append is flushed and
//!   `sync_data`d before the durable length advances; a failed write is
//!   rolled back provably or the handle is poisoned (fail closed); a
//!   torn final line is truncated on reopen. The production write path
//!   and every injected fault flow through one shared
//!   rollback-or-poison recovery, so a REAL fault (an `RLIMIT_FSIZE`
//!   file-size limit with `SIGXFSZ` ignored, exercised in a child
//!   process so the parent limit is untouched) rolls the file back to
//!   the exact pre-fault durable length with no surviving fragment,
//!   the next append lands clean, and the reopen is consistent.
//! - **A restart is journaled uncertainty (R2)**: every reopen appends
//!   a durable `SessionStarted` record under the exclusive lock
//!   (reopen is a writer operation) and moves any in-flight
//!   `BroadcastSent` transaction to `Unknown` with `ever_unknown` set -
//!   so a later rejection degrades instead of releasing - while
//!   touching neither the retry-send budget (a process restart is not
//!   a retry send) nor the attempt counters (crash-looping reopens
//!   cannot inflate them).
//! - Reservations are durable, exclusive and crash-atomic; the journal is
//!   the single funding-provenance ledger (JSONL, no SQL shadow).
//! - Command dedup follows the engine's `command_id` semantics exactly:
//!   same id + same payload digest = stored outcome; a different digest
//!   (including a different **payer role or category**, M1) is a durable
//!   conflict, never a second payment.
//! - Signed raw bytes and txid are durable before any broadcast record
//!   can exist; same-id retries return identical bytes.
//! - **Uncertainty never releases inputs (H1)**: an ever-unknown,
//!   ever-RPC-accepted or reorg-resubmitted payment degrades its inputs
//!   to possibly-spent on rejection - the durable counters decide, so
//!   crash replay takes the same decision.
//! - **Reorg cascade and recovery (H2, R12, S3)**: displacing a confirmed
//!   transaction invalidates its own funded outputs (remembering their
//!   prior status) and every descendant; recovery proceeds generation by
//!   generation through proven-unspent, capped reorg resubmission, and a
//!   canonical re-inclusion of a reorged transaction in the new chain
//!   is recordable from `Reorged` as block evidence, restoring the
//!   cascade one generation at a time. The displaced-block refusal is
//!   **stale-evidence only** (S3): the journal derives the displacement
//!   seq and the last committed `ChainObserved` seq from the record
//!   stream alone, and once a chain observation newer than the
//!   displacement exists - the chain reorged back and the block is
//!   canonical again - the same-hash confirmation is admissible; the
//!   original displacement identity survives a cascade `Invalidated`
//!   and the promotion back to `Reorged` (never overwritten by the
//!   restoring parent's confirmation). `note_rejected` always records
//!   the rejection and guards only the finish (S4): a genuine
//!   post-resubmit rejection of a finished command is journaled and
//!   degrades the inputs; classifying node answers ("already known /
//!   already in block chain" is a confirmation observation, never a
//!   rejection) is the driver's (T1b's) concern - the core never
//!   parses the code string to infer canonicality.
//! - **Bounded retries (H3)**: `Reorged` re-enters the network only via
//!   `ReorgResubmit`; retry sends and resubmits are capped by the
//!   persisted `JournalCaps`.
//! - **Verified funding (H5)**: every satoshi spendable by this signer
//!   entered through an explicit funding observation carrying source
//!   provenance and a derivation locus (the wallet derives the script;
//!   own-transaction backing requires the journaled confirmed creator
//!   with an exact vout match); coinbase funding is spendable only at
//!   100 confirmations of the observed tip; synthetic fixtures are
//!   refused by the production constructors.
//! - Spending resolves keys by **direct derivation at the recorded
//!   index (M2)** - the script lookahead never bounds the change chain.
//! - Late lifecycle observations after a reorg cycle are **journaled
//!   without error (M7)**; only the command finish is guarded.
