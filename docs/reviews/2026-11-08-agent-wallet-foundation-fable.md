# Fable review: AP-3a agent wallet foundation (concrete candidate)

Date: 2026-11-08
Reviewer: Fable (Claude Fable 5.1), read-only review subagent
Candidate: `/home/ethan/.local/share/opencode/worktree/ad8886/lab-agent-wallet` (uncommitted, base `b0ab4ca`)
Frozen manifest: `/tmp/opencode/AP3a-review-manifest.json`
Implementation report under review: `docs/reviews/2026-11-07-agent-wallet-foundation.md` (candidate copy)
Plan: `lab-experiment-expansion/docs/plans/2026-11-07-lab-ark-services.md` (sections 3.3, 4.1.1, 4.4, 4.7, 11.0, AP-3)

## Verdict: REQUEST_CHANGES (bounded)

The derivation core, the persistent counter and the fail-closed posture are
real and independently reproduced. The candidate is not approvable as
"AP-3a wallet foundation" because the dispatched scope included a persistent
embedded stock bark wallet and the candidate ships only a `WalletSeed`
fingerprint, while the report labels that "Persistent child wallet"; and
because the registration run loop has a concrete terminal-outcome hot loop
that allocates fresh request ids without backoff. Both are bounded fixes that
need no fake Ark server, no chain source and no live transport. Items 1-5
below block; 6-11 should be fixed in the same pass or explicitly carried.

This review makes no Ark-ready claim, adopts no node profile or pin, and
approves nothing beyond the exact 22-file delta identified below.

## 0. What was reproduced (independent of the report)

Manifest and source identity:

- Recomputed `snapshot_sha256` from the manifest rows =
  `ec8edf5778dacd2865ba0c1e72774a975bb33a54577501e2ee1b13274e2f72d9` and
  `code_snapshot_sha256` (excluding only the candidate report) =
  `6af70f0f5c3e067625d69460715297f20784f4e16d58edf09cffe8d5b3874b92`; both match.
- Every one of the 383 manifest rows matches the on-disk file hash; no file
  in `git ls-files --cached --others --exclude-standard` is missing from the
  manifest and vice versa.
- Baseline manifest `/tmp/opencode/LP-final-runtime-manifest.json` hash
  `8bbe097d...` matches the `baseline_manifest_sha256` field; baseline code
  snapshot `30fedaf...` as stated. Diffing candidate rows against baseline rows
  gives exactly 13 added + 9 modified + 0 removed = the 22 `delta` rows. The
  report's "105 uncommitted path entries" is `git status` against `b0ab4ca`
  (which includes the frozen lab-runtime delta); the 22-row delta is the
  authority for this review.
- `/tmp/opencode/AP3a-nix-src` and the Nix store source actually used by all
  four check derivations (`/nix/store/9g1gkx5985fxqgn6biw8lsvb90g6lw4y-source`)
  are byte-identical to the manifest's code set (every code row matches; only
  the report is absent, as expected; no extra files).

Check receipts: the four `/tmp/opencode/AP3a-nix-*-full.log` files contain only
"this derivation will be built" lines; they are not receipts. The receipts are
`nix log` for the four derivations named in them, which I read:

- `spc-ark-feature-tests` (`lz64ha62...`): checkPhase completed; suites include
  `spc_crypto` lib 57 passed, `spc_agent` lib 78 passed, `wallet_receive` 3,
  `wallet_recovery` 4 (+1 ignored), `wallet_registration` 5, plus the
  pre-existing agent suites; 0 failures.
- `spc-workspace-tests` (`jslykl3q...`): checkPhase completed, 0 failures.
- `spc-clippy` (`r0xzmcj5...`): buildPhase completed, only the pre-existing
  upstream `sqlx-postgres` future-incompat note.
- `spc-fmt` (`sws151n7...`): completed.

Local re-run in the candidate worktree (plain cargo, not `--offline`):
`cargo test -p spc-crypto --features ark` 57 passed; `cargo test -p spc-agent
--features ark` 78 lib + 2+2+1+1 pre-existing + 3+4+5 wallet integration, 0
failed, 1 ignored (fixture generator). `cargo clippy -p spc-agent -p
spc-crypto --all-targets --features ark -- -D warnings` clean. The report's
"96 agent ark tests (78 lib + 18 integration)" is accurate.

Dependency scope: `Cargo.lock` diff vs baseline is only new dependency edges
(`axum`, `bark-wallet`, `chrono`, `rusqlite`, `tempfile`, `zeroize` on
spc-agent; `bip39` on spc-crypto); no package added, removed or moved. The
`bark-wallet` git rev is the UPSTREAM_DEPS pin `c419159`. `UPSTREAM_DEPS.md`
untouched. No pin bump. `flake.nix` delta vs baseline is two hunks (vendor
hash, one `cargo test` line); the report's merge note 6.4 states them.

Derivation, reproduced in pure Python (own BIP32/BIP85/BIP39/bech32m code,
no Rust, wordlist taken from the `bip39-2.2.0` crate source):

- BIP 85 app 39 spec vectors from `bip-0085.mediawiki` for the spec master
  xprv: 12 words index 0 entropy `6250b68daf746d12a24d58b4787a714b` and
  mnemonic "girl mad pet galaxy ... nose"; 18 words
  `938033ed...1ce9d9dc`; 24 words `ae131e23...9c51f37f`. All match. The crate
  pins only the 12-word vector (`hd.rs:291-305`); that is sufficient but the
  18/24 vectors would be a cheap addition.
- Fixture `tests/fixtures/ap3a-recovery-fixture.json`: master seed equals
  `sha256("spc-ap3a-recovery-fixture-left-v1") || sha256(...-right-v1)`;
  `m/83696968'/39'/0'/12'/0'` -> 16 bytes -> 12 English words -> PBKDF2
  (`"mnemonic"`, empty passphrase) reproduces `child_seed_hex` exactly.
- `scan_xprv` / `ark_xprv`: depth 4, child `0x80000001` / `0x80000002`, key
  and chain code equal my `m/352'/0'/0'/1'` and `.../2'`, shared parent
  fingerprint `e3409c27`, `tprv` magic `04358394`.
- `b_ark_pubkey` = `m/352'/0'/0'/2'/0`; `M` = `m/350'/1'` under the child
  seed; `normal_vtxo_pubkey` = `m/350'/0'/0`; `wallet_fingerprint_hex` =
  HASH160 of the `m/350'` pubkey under the child seed. All match. The bark
  constants `350 / 0 / 1` match `bark/src/lib.rs:480-484` at `c419159`, and
  `WalletSeed::new_from_seed` / `fingerprint()` (`lib.rs:691-712`) derive the
  same `m/350'` root the in-tree cross-check uses.
- `sp_address`: HRP `tsp`, bech32m checksum valid, version 0, payload is
  `B_scan || B_spend` from the scan/spend branches; spend != ark.

Runtime inspection (not comments):

- `spc_crypto::provision` is `pub` and re-exported, but inside `bins/` and
  `crates/` it is called only from `#[cfg(test)]` modules
  (`keys.rs:360/448/519/533`, `hd.rs:351/395`) and from `tests/`. The
  production agent has no call path to a master seed.
- Key files are read with an exact `0600` check and never chmod'd
  (`keys.rs:260-285`); the datadir and `state.sqlite` are tightened, which
  the code documents. No `Debug` impl on `WalletKeys`, `ProvisionedKeys`,
  `ChildWalletMaterial` or `WalletStore` prints material; `tracing` calls in
  the wallet module log network, fingerprint, datadir, counters and outcomes
  only. Status JSON carries no `sp_id`, key, `U` or VTXO id (asserted in
  tests and confirmed by reading `StatusSnapshot`).
- Counter: reservation is committed in one sqlite transaction before `send`
  (`state.rs:369-430`), the pending id is reused only for an equal payload
  digest, `u32::MAX` is refused before any send, the 8x8 concurrent
  reservation test is real and gapless, and `record_registration_outcome`
  clears the pending slot only when it matches the resolved id.
- `0x5350` codec is not duplicated: `registration.rs` carries a semantic
  `RegistrationPayload` and a local correlation digest; the trait seam
  `RegistrationTransport` is the only exit. The digest covers exactly the
  plan 4.1.1 signed-field set (`sp_id, version, ark_id, b_ark, mailbox_mode,
  mailbox_pubkey, mailbox_dh, epoch_fallback`) with presence flags for the
  two `B0_33` fields, so its equivalence classes match AP-2's signed
  message; it is correctly not claimed to be the BIP340 digest.
- Wallet role never co-runs with translator/jdc (`lib.rs:validate_role_selection`,
  wired in `main.rs`); `--roles wallet` without the `ark` feature bails.

## 1. Blocking findings (ranked)

### 1. Promised persistent stock bark wallet is absent; the report labels a fingerprint as "Persistent child wallet"

Dispatch (`docs/REGTEST-LAB-STATUS.md:113`, `docs/MERGE-QUEUE.md` AP-3a row):
"persistent keystore/stock bark wallet". Plan 3.2 row for `spc-agent --roles
wallet`: state is "local wallet sqlite + bark wallet sqlite + status file".
Plan AP-3 deliverable: "the same `--roles wallet` process embeds the in-tree
SP keystore and the stock bark wallet opened on the BIP85 child seed".

What exists: `keys.rs:345-351` calls `bark::WalletSeed::new_from_seed` and
keeps `fingerprint()`; `state.rs` persists that fingerprint in `meta`. No
bark persister, no bark sqlite, no `WalletProperties` is ever created. The
report says so in section 1 ("full stock bark `Wallet::open`" excluded) and
section 9, but section 3 headlines "**Persistent child wallet**: stock bark
`WalletSeed::new_from_seed`", and section 4 says recovery "recreates state
from the same seed with identical `U_i`/VTXO ids". Neither is a stock wallet
claim that the code supports.

The report's reason for deferral ("`Wallet::open` needs a live chain source")
is true for `Wallet::open` (`bark/src/lib.rs:1341-1372` builds a
`ChainSource` and calls `require_version`), but it does not justify shipping
no stock wallet state at all. The stock persister is usable offline with no
server and no chain:

- `bark::persist::sqlite::SqliteClient::open(<datadir>/bark/db.sqlite)`
  (`persist/sqlite/mod.rs:53`, public under the default `sqlite` feature);
- `BarkPersister::init_wallet(&WalletProperties { network, fingerprint,
  server_pubkey: None, server_mailbox_pubkey: None })`
  (`persist/mod.rs:105`); this is exactly what `Wallet::create(...,
  allow_unreachable_server = true)` writes (`lib.rs:1255-1266`) minus the
  connection attempt;
- `read_properties()` on reopen plus `properties.fingerprint == seed.fingerprint()`
  and network check, failing closed on mismatch, exactly as `Wallet::open`
  does at `lib.rs:1322-1324` before it ever touches the chain.

Required (bounded, offline, no fake server):

- On first start create the stock bark persister under
  `<datadir>/bark/` (0700/0600) with `WalletProperties` as above; on every
  later start reopen it and fail closed on fingerprint or network mismatch
  or missing properties; never recreate or overwrite. Keep the in-tree
  `state.sqlite` separate (plan: "separate from the embedded bark wallet
  sqlite").
- Tests: first run creates the bark db and properties; reopen with the same
  seed succeeds and reads the same fingerprint; reopen with a different child
  seed is refused and the file is untouched; datadir destruction then
  recreation from the same seed yields the same fingerprint in the new db.
- Report wording: replace "Persistent child wallet" with what is true after
  the fix ("stock bark persister initialised and fingerprint-bound; full
  `Wallet::open`, server pubkeys and chain source deferred to AP-3b"). Until
  then no document may call the candidate wallet-ready or describe the
  recovery test as wallet recovery (see finding 8).

If the parent explicitly re-scopes AP-3a to exclude any stock wallet state,
the minimum is the relabel above everywhere (report sections 3, 4, the
`keys.rs:341-344` doc comment and the dispatch rows), and the dispatch
promise must be carried forward as an open AP-3b item, not closed.

### 2. Terminal registration outcomes drive an unbounded fresh-id hot loop

`mod.rs:327-379` (`WalletRole::run`, `Some(transport)` arm) loops until
`is_registered()`; `registration.rs:503-516` journals every terminal outcome
(`Mismatch`, `Stale`, `Terminal { .. }`, `Superseded`) and clears the pending
slot, so the next `attempt` reserves a fresh id. Neither path calls
`backoff.record_failure()` (only `Retryable`/`Unavailable` do,
`registration.rs:459/475/483`), `WalletState::Error` is never set anywhere
(`grep` finds only its label at `status.rs:55`), and the loop has no exit for
terminal codes.

Counterexample: a transport that returns `Terminal { "bad-signature" }` (or
`wrong-ark-server`, `ark-key-is-spend-key`, `unsupported-mailbox-mode`)
every time. The role reserves ids 1, 2, 3, ... one per `delay_secs().max(1)`
= 1 s, journals a terminal row each, and reports `state = awaiting_registration`
forever. SPEC 11.5 says a registration failure "is a hard stop, not a
warning"; the plan's fresh-id rule applies to `registration-mismatch` (one
fresh id so the client does not self-wedge), not to every terminal code.
Even `request-id-exhausted` (returned without a send) spins the loop at 1 s.

Required:

- Terminal codes other than the plan's explicit unwedge cases move the role
  to `WalletState::Error` with the code in `last_error` and stop the attempt
  loop (operator action restarts it). `Mismatch`/`Stale`/`Superseded` may
  allocate exactly one fresh id per terminal verdict and must then apply
  backoff; a second consecutive terminal verdict for the same payload ends
  in `Error` too.
- `record_failure()` on every non-success outcome so the delay schedule is
  honoured.
- Test through `WalletRole::run` with a scripted transport: N consecutive
  terminal verdicts consume a bounded number of request ids (assert the
  exact count), end in `state = "error"`, and never report `connected`.

### 3. Branch xprv validation does not check the branch index or the shared parent

`keys.rs:302` checks `depth == 4 && child_number.is_hardened()` only. The
`branch: &'static str` argument ("1'", "2'") is used solely in the error text.
The scan branch is effectively verified through `B_scan` in `sp_address`
(`keys.rs:168`), but the ark branch has no external anchor (`B_ark` is not in
the address), so its path is unverified.

Counterexample: write `m/352'/0'/0'/3'` (or any depth-4 hardened key from a
different master) into `ark.xprv`. `load_branch_xprv` passes, `b_ark != b_scan`
and `b_ark_pub != B_spend`, so `WalletKeys::load` succeeds and registers a
`B_ark` whose `u_i` the provisioner never intended. The fixture already shows
the enforceable data: both branches carry `child_number` `1'`/`2'` and the
same `parent_fingerprint` (`e3409c27`).

Required: assert `child_number == hardened(1)` for the scan file and
`hardened(2)` for the ark file, and `scan.parent_fingerprint ==
ark.parent_fingerprint` (same `m/352'/0'/0'` account). Tests: ark file at
branch `3'` refused; ark file from a different master at `2'` refused
(parent fingerprint); the fixture still loads. This closes the "exact
derivation branch validation" the report claims.

### 4. Registration journal has no payload digest; `is_registered()` is payload-agnostic

`state.rs:494-503` returns true if any `applied`/`already_applied` row exists;
the journal schema (`state.rs:235-242`) stores no `signed_fields_digest`.
`mod.rs:331-340` treats `is_registered()` as "nothing to send" and parks the
loop in `Connected`.

Counterexample: register payload A (`applied`, id 1). Operator changes
`[ark] ark_id` (or the mailbox mode) and restarts. `run(Some(transport))`
sees `is_registered() == true`, sets `Connected`, never sends payload B, and
the status reports `registered = true` for a registration the server holds
under different fields. The plan requires a higher id for the new fields; the
client never tries.

Required: store the payload digest in `registration_journal`; define
registered as "an `applied`/`already_applied` row whose digest equals the
current payload digest"; the run loop sends when the current payload is not
the registered one. Test: applied A, mutate the payload, assert
`is_registered()` is false and the next attempt sends with a fresh id.

### 5. Normative and configuration deltas with no SPEC hunk or decision record

Report 6.6 declines a SPEC edit because "AP-2 is concurrently editing the SPEC
11.4/11.9 registration areas". AP-3's owner paths in the plan are SPEC
section 11.5 / O13 (plan 11 AP-3), and the AP-2 worktree's `docs/SPEC.md`
diff touches neither 11.5 nor 11.10 (checked against the baseline). The
candidate introduces, in code:

- an exact BIP 85 definition (`m/83696968'/39'/0'/12'/0'`, English, 12 words,
  empty passphrase, 64-byte seed) where SPEC 11.5 says "BIP 85-style";
- six new `[ark]` agent keys (`scan_xprv`, `ark_xprv`, `datadir`,
  `status_listen`, `network`, `ark_id`) absent from SPEC 11.10 and from
  `config/spc-agent.example.toml` (unchanged vs baseline);
- a loopback status surface (plan 4.7 fields plus `state`) with specific
  null-not-zero semantics;
- client-side counter/journal semantics (pending-digest reuse, terminal
  clearing) that are now behaviour.

AGENTS.md requires a dated decision record and a SPEC changelog entry in the
same change for normative behaviour. Required: SPEC 11.5 and 11.10 hunks
(exact text, in the change; a merge note is acceptable only if the merge
session has claimed those sections, which the plan does not say) and a
decision record for the exact BIP 85 definition and the agent config/status
surface. The `2026-11-08-ark-refresh-normal-wallet.md` record can still land
with AP-3b; it is not the record this slice needs.

## 2. Should-fix findings (same pass or explicitly carried)

6. **`connected` without a connection.** `mod.rs:189-193` and `:294-299` set
   `WalletState::Connected` purely from `is_registered()`, including under
   `run(None)` where nothing is connected and no `last_error` is set (the
   dormancy message at `mod.rs:303-316` is skipped when already registered).
   Rename the state to `registered` or require a live transport for
   `connected`; the restart test should assert the dormant-but-registered
   snapshot is honest.
7. **`registration_unavailable` after six attempts is never surfaced.**
   `Backoff::exhausted()` is tested but unused at runtime; the loop sets
   `last_error` to the raw retryable code (`mod.rs:362-370`). Plan 4.1.1:
   after six attempts surface local `registration_unavailable` and keep
   retrying at 30 s.
8. **Fixture expiry is `height + 432` labelled `VtxoAncestry`.**
   `wallet_recovery.rs:134` computes the exact finding-height pattern the plan
   forbids and tags it as ancestry; `receive.rs` has no ancestry or receipt
   parsing, `ExpirySource` is a caller-asserted label, and `VtxoState::Spendable`
   is likewise asserted by the caller. The ledger is a persistence/validation
   layer (ownership, non-zero amount, non-zero expiry height, forward-only
   transitions), not a receive path. The report's section 1 phrase "expiry
   from actual VTXO ancestry, never a finding height" and section 4 phrase
   "identical `U_i`/VTXO ids" must be reworded: VTXO ids here are
   `sha256("spc-ap3a-vtxo" || height || a_send)` fixture labels, and the
   recovery test proves deterministic key re-derivation plus ledger replay
   idempotence, not mailbox or stock-wallet recovery. Also note the test's
   `AlreadyApplied` for id 1 after datadir loss models a server whose
   `last_request_id` is 1; a real server at N > 1 answers `stale`/`mismatch`
   first, which is exactly why finding 2 matters.
9. **Scan cursor is monotonic-only with no rewind API** (`state.rs:790-828`).
   A reorg below the cursor cannot be rescanned without destroying the
   datadir. AP-3b will need an explicit, journaled rewind; record it as a
   schema/API caveat now so the v1 schema does not get frozen around it.
10. **Smaller hardening.** `StatusService::serve` (`status.rs:154`) binds any
    address; enforce `is_loopback()` there as well as in config (defence in
    depth for programmatic `WalletSettings`). `WalletKeys::drop`
    (`keys.rs:131-139`) scrubs `b_scan`/`b_ark`/seed but not the `mailbox` and
    `normal_vtxo` keypairs. `WalletStore::open` runs `Connection::open` before
    `chmod_0600` (`state.rs:116-125`), a short default-umask window; create
    the file with `OpenOptions::mode(0o600)` first. A datadir pointed at a
    foreign sqlite file with no `meta` table is silently adopted as a fresh
    store (`state.rs:265-289`); require an empty file or a complete `meta`.
    The status error path returns `e.to_string()` which may include the
    datadir path (loopback-only; acceptable, but note it).
11. **Report corrections.** (a) Section 5's four `.log` files are not exit
    receipts; cite `nix log <drv>` for the four derivations (which do show
    completion and counts). (b) Section 6.5: a non-git copy is not "required"
    and no commit is needed for Nix visibility; `nix build
    path:/<worktree>#checks.x86_64-linux.test-ark` includes untracked files
    (the existing `cleanSourceWith` filter already drops `target/`, `.git`,
    `result*`). Do not force a git commit to satisfy Nix. (c) Section 8's 105
    entries vs the 22-row delta, as above. (d) Section 3's "Persistent child
    wallet" and section 4's recovery phrasing, per findings 1 and 8. (e) The
    flake vendor hash will change again after AP-2 merges; the merge session
    owns the final value (plan 11.0), which the report should say.

## 3. What this review does not cover (AP-3b and later; not held against AP-3a)

Live mailbox reads, `GetVtxoStatus` and claim with `u_i`; chain scan via
`spc-scan`; `static` and `epoch_registered` modes (correctly refused in
`WalletSettings::from_config`); full `Wallet::open` with a chain source;
delegated round refresh and offboard; `ArkPayoutNotice` reconciliation
against real notices; the AP-2 `0x5350` adapter behind `RegistrationTransport`;
Postgres-journal interplay after datadir loss; `--offboard-on-exit` and the
SPEC 11.5 shutdown warning. None of these are claimed by the candidate, and
this review does not require a fake server to close finding 1.

## 4. Separate from this candidate

- The flake.nix edits are the shared merge hunk the plan's section 11.0
  reserves for the merge session; the earlier corrected Nix network packet
  is unaffected and not reviewed here.
- No node profile, pin, or UPSTREAM_DEPS change is adopted by this review.
- Primary code and the candidate were not modified; no commits, subagents,
  services or mining were run. Local artefacts written: this file and
  `/tmp/opencode/AP3a-fable-{crypto,agent}-test.log`.

---

# Re-review: AP-3a fix-r1 (same reviewer, confined to findings 1-11 and the fix itself)

Date: 2026-11-08 (later the same day)
Candidate: same worktree, uncommitted, base `b0ab4ca`
Frozen manifest: `/tmp/opencode/AP3a-fix-r1-manifest.json`
Pre-rework snapshot: `/tmp/opencode/AP3a-fix-r1-base` (verified byte-identical
to the original 383-row candidate manifest; original manifest untouched,
sha256 `d95270cb...`)
Patch under review: `/tmp/opencode/AP3a-fix-r1.patch`

## Verdict: REQUEST_CHANGES (narrow) - one bounded code gap in the new bark persister, two doc corrections

Findings 2-11 of the original review are closed and were verified against
code, not the report. Finding 1 (stock bark persister) is implemented with
the pinned stock `SqliteClient` + `init_wallet`, but the fresh-vs-damaged
boundary it introduces has one concrete hole that recreates a destroyed
wallet database when its completion marker is still present. That is the
exact "partial damage vs deliberate reset" distinction the persister exists
to make, and the decision record codifies the wrong rule ("missing file =
fresh"). The fix is a handful of lines plus two tests; nothing else blocks.

Still no Ark end-to-end readiness claim, no pin or node-profile adoption; full
`Wallet::open`, live refresh/offboard (AP-3b) and the AP-2 `0x5350` transport
remain pending and are not held against this slice.

## 0. Reproduced

- Manifest: recomputed full `cefc18226fac0363654417486c51f22cc2d1509d0e41cf528faa45a2b780feee`
  and code `e6af84a91714a9f85cbf04fcbb12cf976d02431b38fd144e7366d0ec174fdd5c`
  from the rows; all 385 on-disk hashes match; git inventory
  (`--cached --others --exclude-standard`) equals the manifest set exactly.
  Delta vs the original candidate rows: 2 added (`wallet/bark_store.rs`,
  `docs/decisions/2026-11-08-agent-wallet-foundation.md`) + 13 modified = the
  15 `delta` rows. `Cargo.lock`, root `Cargo.toml`, `flake.nix` unchanged
  (confirmed by hash); `bins/spc-agent/Cargo.toml` adds only
  `tokio = { workspace = true, features = ["test-util"] }` under
  dev-dependencies - no new package, no vendor-hash impact.
- Patch: `AP3a-fix-r1.patch` body is identical to my own
  `diff -ruN AP3a-fix-r1-base <worktree>` (excluding `target/`, `.git`,
  `result*`) apart from diff-header timestamps.
- Nix: `/tmp/opencode/AP3a-fix-r1-nix-src` and the derivation's actual
  `src` (`/nix/store/v2fb8x2pqajp5j1icn0cbzv5mcrsq6ph-source`, from
  `nix derivation show q5x5r45n...`) match every manifest code row; the
  only differing file is the implementation report (excluded from the code
  hash by recipe; the copy predates its final edit). `nix log` for
  `q5x5r45n82s34kiwf623mhr7wkc2v41b-spc-ark-feature-tests-0.1.0.drv`: 26
  `test result` lines, zero failures, checkPhase completed; agent suites
  exactly `spc_agent` lib 104, `wallet_receive` 3, `wallet_recovery` 4 (+1
  ignored), `wallet_registration` 9, e2e 2/2/1, `example_config` 1
  (agent crate total 126, matching the cargo evidence file). The report's
  aggregate "18 suites, 137 passed" matches neither the agent crate (126) nor
  the whole check (468 across 26 suites); the per-suite numbers are correct,
  the headline number is not.
- Local, reusing the existing target with `-j 2`:
  `cargo test -p spc-agent --features ark` 104 + 3 + 4(+1) + 9 + 2/2/1/1,
  0 failed; `cargo clippy -p spc-agent --all-targets --features ark
  -D warnings` clean. Workspace log `AP3a-fix-r1-workspace-test.log`: 81
  `test result` lines, 641 passed, 0 failed, `rc=0`; not re-run.
- Fixture: key material (`master_seed_hex`, `child_seed_hex`, both xprvs,
  `sp_address`, `sp_id`, `M`, normal key, fingerprint, per-block `U_i`) is
  byte-identical to the original fixture I reproduced in pure Python; only
  `expiry_height` of block 2 (900000), the `expiry_provenance` strings and
  the top-level `provenance` label changed.
- Candidate hashes re-checked after all my commands: unchanged.

Disk incident: the worker reports deleting ~38 GB of stale Nov-3 cargo
caches (`m64-target-{guardian,main,runtime}`) from `/tmp/opencode`; the
receipt is `df` plus names. None of the evidence this re-review relies on
lives there (nix store derivation log with byte-verified source; the
candidate's own `target/`; the frozen manifests, whose hashes I re-verified).
I did not delete, recover or approve anything; the cleanup is noted, not
reviewed.

## 1. Disposition of the original findings (verified against code)

1. **Stock bark persister - implemented, one gap (see section 2).**
   `bark_store.rs` uses `bark::persist::sqlite::SqliteClient::open`,
   `BarkPersister::init_wallet(WalletProperties { network, fingerprint,
   server_pubkey: None, server_mailbox_pubkey: None })` and
   `read_properties()`; no server, no chain, no in-tree mirror schema.
   Read-only pre-check via `SQLITE_OPEN_READ_ONLY` before the migrating
   open; private dir/file creation (`create_new` + `0o600`) before sqlite
   touches the path; refusal paths leave bytes untouched (asserted by
   sha256 in tests). `WalletRole::start` opens it after `WalletStore`
   (`mod.rs`), and the role-level restart test checks 0700/0600 and
   `is_fresh() == false` on reopen. Stock sidecars (`-journal`) inherit the
   0600 main-file mode via sqlite's unix VFS. Verified closed except for
   the marker/absent-file case in section 2.
2. **Terminal hot loop - closed.** `RegistrationManager` now carries
   `fatal`, `terminal_streak`, `streak_digest`; hard terminals
   (`Terminal { .. }`) and `request-id-exhausted` are fatal on first sight
   with exactly one (or zero) send; `Mismatch`/`Stale`/`Superseded` get one
   fresh-id retry then fatal; every non-success outcome calls
   `record_failure()`; `attempt()` returns `RegistrationError::Fatal` and
   never sends again; `run()` sets `WalletState::Error` with the exact
   code, stops the status server and returns `Err`. `exhausted()` is now
   consumed by the run loop and surfaces `registration_unavailable (6+
   failed attempts)` while 30 s retries continue. Tests
   `run_stops_fatally_on_bad_signature_after_one_send` (sent == [1]),
   `run_stops_fatally_after_two_consecutive_wedge_terminals` (sent ==
   [1, 2]), `run_surfaces_unavailable_and_keeps_retrying_until_shutdown`
   (paused time, >= 6 sends, continues, clean shutdown),
   `retryable_unknown_preserves_the_pending_id`,
   `request_id_exhaustion_is_fatal_without_a_send`. Note: because the
   status listener is torn down before `run` returns, the `error` state is
   observable in-process and via exit status/logs, not by polling
   `/status` afterwards; acceptable for a hard stop, record it in the
   operator notes.
3. **Branch validation - closed.** `load_branch_xprv(path, expected_index,
   ..)` requires `depth == 4 && child_number == hardened(expected_index)`;
   `WalletKeys::load` requires `scan.parent_fingerprint ==
   ark.parent_fingerprint` (`BranchesNotSiblings`). Tests: 3' refused for
   both slots, foreign-master 2' refused, spend branch in the ark slot
   refused at index check, valid fixture unchanged. The 4-byte fingerprint
   is labelled a provisioning-metadata consistency check, not ancestry
   proof - correct.
4. **Effective binding - closed.** Schema v2 adds `effective_digest` /
   `effective_request_id`; `record_registration_outcome` updates the
   binding only for the currently pending id and only on
   `applied`/`already_applied`; `superseded` clears the binding only when
   it names the effective digest; other outcomes clear pending only;
   `is_registered(&digest)` compares against the current payload digest.
   A -> B -> A does not re-register A from history; interrupted B leaves A
   effective and B unregistered; a late ack for a non-pending id journals
   only (`replayed_request_id_cannot_rewrite_the_binding`). v1 is rejected
   explicitly (`SchemaVersion { found: 1, expected: 2 }`, test present);
   a `meta`-less sqlite with tables is refused as foreign.
5. **SPEC/decision/example config - closed in substance, one convention
   miss (section 2, item B).** SPEC 11.5 gains the exact BIP 85 derivation,
   provisioning shape, persistence layout, client journal/terminal policy,
   status honesty and the AP-3b reorg-rewind dependency; 11.10 gains the
   seven `[ark]` agent keys; `config/spc-agent.example.toml` documents them
   (paths/ids only); the dated decision record exists. The text does not
   weaken DESIGN.md (derivation paths and the hot/cold split are
   unchanged; it tightens "BIP 85-style" to a definition). AP-2's SPEC
   diff still touches neither 11.5/11.10 nor the frontmatter, so the
   sections are disjoint for serial integration.
6. **registered != connected - closed.** `WalletState::Dormant` added;
   `start` and `run(None)` set `dormant` (with `last_error` naming AP-2)
   even when the effective binding is registered; `Connected` is no longer
   set without a transport.
7. **registration_unavailable after six attempts - closed** (folded into 2).
8. **Fixture honesty - closed in code and fixture; report text not fully
   updated (section 2, item C).** Fixture carries `provenance: "SYNTHETIC:
   ..."` and per-block `expiry_provenance`; expiries are explicit constants
   (800432 in-window, 900000 out-of-window), no longer `height + 432`;
   `load_fixture` asserts the label; `receive.rs` and `wallet_recovery.rs`
   headers state that `Spendable` is caller-asserted and that the test is
   key derivation + ledger replay.
9. **Reorg rewind - closed as an explicit AP-3b dependency** (SPEC 11.5,
   decision item 8, `set_scan_height` still refuses backward moves). Not
   demanded live.
10. **Hardening - closed.** `StatusService::serve` refuses non-loopback at
    bind (test with 192.168.x and 2001:db8::1; `::1` accepted);
    `WalletStore::open` creates `state.sqlite` with `create_new` + 0600
    before sqlite opens it; foreign non-empty sqlite refused with bytes
    untouched; `WalletKeys::zeroize_keys` overwrites branch secrets, child
    seed and both keypairs with a dummy key and is honestly labelled
    best-effort (the replaced `SecretKey`/`Keypair` values are dropped, not
    scrubbed in place - the doc comment says so).
11. **Report provenance - mostly closed.** Section 5 now cites `nix log`
    and the derivation/output/source paths; the `path:` property is stated
    correctly; the 22-row delta is acknowledged. Remaining inaccuracies are
    in section 2 below.

## 2. Remaining required changes (minimal)

### A. (code + tests) `bark_store.rs`: a destroyed or truncated database with the completion marker present is recreated, not refused

`BarkStore::open` (`bark_store.rs:154-158`) consults the markers only when
`db_path.exists()`; `read_prior` (`:318-324`) returns `Prior::Empty` for a
0-byte file before looking at markers. Both paths then take the `fresh`
branch. Counterexamples:

- initialize once, delete `bark/db.sqlite` only (markers remain). Next
  start: `Prior::Absent` -> creates a new db + `spc-creating`, migrates,
  `init_wallet` writes fresh properties, then
  `create_private_file(&initialized_marker)` fails with `create_new`
  ("File exists") **after** the database has already been re-initialized.
  The start fails with a misleading "cannot create" error; the start after
  that sees a properties row, validates, removes `spc-creating`, and runs
  on a silently recreated wallet database.
- initialize once, truncate `bark/db.sqlite` to 0 bytes (markers remain):
  same sequence minus file creation.

This contradicts the module's own rule ("a database that claims
initialization ... but cannot present complete identity metadata is damaged
prior state, not fresh") and the decision record's item 2 ("missing file
(fresh), empty file (fresh)") codifies the wrong rule. The plan's recovery
procedure destroys the whole agent datadir; a missing or zeroed database
inside a `bark/` directory that still carries `spc-initialized` is partial
damage, not a deliberate reset.

Required:

- Before any mutating step: if `prior` is `Absent` or `Empty` and
  `spc-initialized` exists -> `Err(Corrupt("initialization marker present
  but database missing/empty: damaged prior initialization, refusing to
  re-initialize"))`. `Absent` with only `spc-creating` present cannot arise
  from this module's own ordering (db file is created before the marker) and
  should also be refused; `Empty` with only `spc-creating` present is the
  module's own crash window and may proceed.
- Check marker existence before `init_wallet` so `create_new` on the
  completion marker can only fail on a true race, never after a mutation.
- Tests: (1) init, remove `db.sqlite` only, reopen -> `Corrupt`, no new
  `db.sqlite` created, markers untouched; (2) init, truncate to 0 bytes,
  reopen -> `Corrupt`, file still 0 bytes; (3) existing
  `datadir_loss_deterministically_reinitializes_the_same_child` (whole
  `bark/` removed) stays fresh.
- Update decision record item 2 and the `bark_store.rs` module doc to the
  corrected rule.

Recommended, not required: record `bark_initialized` in `state.sqlite`
`meta` at first bark init and refuse when `bark/` is wholly absent while
`state.sqlite` says it was initialized, so that "whole `bark/` directory
gone, `state.sqlite` intact" is also recognised as partial damage. The
deliberate reset remains "whole datadir gone".

### B. (docs) Changelog entry must be a fragment file, not a frontmatter edit

`docs/changelog/README.md` (merge-queue convention): each packet adds
`docs/changelog/YYYY-MM-DD-<slug>.md` and does not edit the `changelog:`
list in `docs/SPEC.md`. fix-r1 edited the frontmatter directly (new first
bullet and `updated: 2026-11-08`) while the report (0.5, 6.6) and the
decision record ("Sections edited") call it a "fragment added ... pending
splice". There is no new `docs/changelog/` file in the 15-row delta.
Required: move the bullet into
`docs/changelog/2026-11-08-agent-wallet-foundation.md` (exact bullet text,
leading two spaces and `- "` quoting), revert the two frontmatter lines, and
correct the wording in the report and record. The 11.5/11.10 body edits stay.

### C. (docs) Remaining report overclaims

- Section 1 still says "receive/claim prototype logic behind backend traits
  with persistence and validation tests (expiry from actual VTXO ancestry,
  never a finding height)". No ancestry parsing exists; `ExpirySource` is a
  caller-supplied label. Reword to match `receive.rs`'s own header
  ("bounded ledger; expiry provenance is recorded as supplied; no finding
  height is ever used as an expiry").
- Section 4 "Notable test cases" still says recovery "recreates state from
  the same seed with identical `U_i`/VTXO ids"; say "identical `U_i` set
  and identical synthetic fixture record ids".
- Section 5 headline "18 suites, 137 passed" does not match the log
  (agent crate 126 across 10 binaries; whole check 468 across 26); use the
  per-suite numbers, which are correct.

## 3. Not required by this re-review

Full stock `Wallet::open` over the persister, live mailbox/claim, chain
scan with journaled reorg rewind, refresh/offboard, static/epoch_registered
modes, and the AP-2 `0x5350` adapter remain AP-3b/AP-2 work. The status
listener teardown on fatal (section 1, item 2) is an operator-notes item,
not a change request. No Ark readiness, pin or profile adoption follows from
this review. Candidate untouched; no commits, subagents, services, mining or
cache deletion by the reviewer. Local artefacts written: this appended
section, `/tmp/opencode/AP3a-fable-r1-actual.patch`,
`/tmp/opencode/AP3a-fable-r1-agent-test.log`.

---

# Re-review: AP-3a fix-r2 (same reviewer, confined to items A/B/C and the new cross-binding)

Date: 2026-11-08 (later again)
Candidate: same worktree, uncommitted, base `b0ab4ca`
Frozen manifest: `/tmp/opencode/AP3a-fix-r2-manifest.json` (file sha256
`60e2477a49c88dc5664600e48ddc6864641a1b9632a6e19d7ffd0e4c4c53e684`)
Pre-r2 snapshot: `/tmp/opencode/AP3a-fix-r2-base` (verified byte-identical
to the fix-r1 manifest's 385 rows)
Patch under review: `/tmp/opencode/AP3a-fix-r2.patch`

## Verdict: APPROVE - exact bounded fix-r2 candidate (full `ae7e99e6d77b4be598bcf9a4e9d1466ebf46d7f31d49a398898c564dfabda218`, code `cd09ed78b7b82c795a43b4798608228f300437e38fc50533638eb8b9b86d0530`)

Items A, B and C from the fix-r1 re-review are closed in code and docs, and
the optional cross-binding was implemented without turning whole-datadir
recovery into a wipe. Two non-blocking notes are carried to serial
integration / AP-3b (section 3); neither needs another review round.

Approval is of this exact file set only. It is not an Ark readiness claim;
full stock `Wallet::open`, live receive/refresh/offboard (AP-3b), the AP-2
`0x5350` transport and any pin or node-profile change remain separate.

## 0. Reproduced

- Manifest: recomputed full `ae7e99e6...` and code `cd09ed78...` from the
  386 rows; all on-disk hashes match; git inventory equals the manifest set.
  Delta vs the frozen r1 rows: 1 added (`docs/changelog/2026-11-08-agent-wallet-foundation.md`)
  + 7 modified (`wallet/bark_store.rs`, `wallet/mod.rs`, `wallet/state.rs`,
  `tests/wallet_registration.rs`, `docs/SPEC.md`, the decision record, the
  report) = the 8 `delta` rows. No `Cargo.*`, `flake.nix`, `keys.rs`,
  `registration.rs`, `receive.rs`, `status.rs` or fixture change: the r1
  registration/key fixes are byte-unchanged.
- Patch body of `AP3a-fix-r2.patch` equals my own
  `diff -ruN AP3a-fix-r2-base <worktree>` (headers aside).
- Receipts `/tmp/opencode/AP3a-fix-r2-{lib-test,registration-test,agent-tests,fmt,clippy}.log`
  carry `rc=0`; every code file's mtime (04:39-04:46) precedes the receipts
  (04:51-04:55); only the report (excluded from the code hash) was edited
  afterwards (04:56). Counts in the receipts: lib 108, registration 10,
  agent crate 131 passed / 0 failed / 1 ignored across 10 binaries.
- My own run on the exact candidate, existing target, `-j 2`:
  `cargo test -p spc-agent --features ark` 131 passed, 0 failed, 1 ignored
  (lib 108, wallet_registration 10, wallet_receive 3, wallet_recovery 4+1,
  e2e 2/2/1, example_config 1); the 14 `bark_store` unit tests listed by
  name, all ok; `cargo clippy -p spc-agent --all-targets --features ark
  -D warnings` clean; `cargo fmt --all --check` clean. The fix-r1 Nix
  (`q5x5r45n...`, 468/26) and workspace (641/81) receipts are correctly
  presented as r1 evidence, not re-claimed for r2; the merged tree's
  `nix flake check` remains the integration gate.
- Candidate hashes re-verified unchanged after my commands. No candidate
  writes, no cache deletion, no services.

## 1. A - bark persister fresh/damaged boundary (closed)

`BarkStore::open` now consults the markers before any mutating step, with
or without a database file (`bark_store.rs`, consistency block right after
`read_prior`):

- `Absent`/`Empty` + `spc-initialized` -> `Corrupt("... database
  missing/empty: damaged prior initialization ...")` before any file is
  created; tests `deleted_db_with_completion_marker_is_corrupt_not_recreated`
  (asserts no `db.sqlite` created, completion marker untouched, no
  `spc-creating` written) and
  `truncated_db_with_completion_marker_is_corrupt_and_stays_zero_bytes`
  (two consecutive attempts, file stays 0 bytes, marker untouched).
- `Absent` + `spc-creating` only -> refused as impossible for the module's
  own ordering; test `creating_marker_without_db_file_is_impossible_and_refused`
  (no db created).
- `Empty` + `spc-creating` only -> the module's own crash window, completes;
  test `empty_db_with_creating_marker_is_our_crash_window_and_completes`.
- `fresh && initialized_marker.exists()` is re-checked immediately before
  `create_private_file(&db_path)` / `SqliteClient::open` / `init_wallet`,
  so the later `create_new` on the completion marker can only fail on a
  true concurrent race, never after this process has mutated the database.
- Whole `bark/` removal still re-initialises deterministically
  (`datadir_loss_deterministically_reinitializes_the_same_child`).

Module doc and decision record item 2 now state the corrected rule.

Cross-binding (was recommended, now implemented): `WalletStore` gains
`bark_initialized()` / `set_bark_initialized()` (`meta` key, `INSERT OR
REPLACE`); `WalletRole::start` refuses when the store records an initialised
bark wallet and `<datadir>/bark` is absent, and records/heals the flag after
every validated `BarkStore::open`. Checked:

- whole agent datadir loss: `state.sqlite` is gone too, the fresh store has
  no flag, the role proceeds to a fresh persister - recovery is not turned
  into a refusal or a wipe;
- `bark/` gone, `state.sqlite` intact: refused before `BarkStore::open`,
  `bark/` is not recreated (test
  `role_refuses_whole_bark_dir_loss_as_partial_damage`);
- concurrency: the flag is written only after a validated open; two
  processes on one datadir are serialised by sqlite on `state.sqlite` and
  by `create_new` on the bark files - no new race surface;
- pre-r2 stores: flag absent until the next validated open; the
  limitation is stated in the decision record.

## 2. B and C (closed)

B: `docs/changelog/2026-11-08-agent-wallet-foundation.md` exists (one
bullet, leading two spaces and `- "` quoting per `docs/changelog/README.md`);
SPEC frontmatter (`updated:` and `changelog:`) is byte-identical to the
pre-packet baseline; the 11.5/11.10 body additions are unchanged from r1.
The decision record and report now describe a fragment file, which matches
the real file. The merge session splices it at serial integration and must
not overwrite the primary tree's later frontmatter (T4 and others already
landed there).

C: report section 1 now says "bounded ledger: expiry provenance is recorded
as supplied; no actual ancestry parser"; section 4 says "identical `U_i`
set and identical synthetic fixture record ids"; the "18 suites, 137"
headline is replaced by the per-crate numbers (agent 126 / whole check 468
for r1; 108 / 10 / 131 for r2) with r2 receipts listed separately.

## 3. Non-blocking notes carried to integration / AP-3b

1. Cross-binding condition. `mod.rs:183` tests `datadir.join("bark").exists()`.
   A `bark/` directory that still exists but has lost both `db.sqlite` and
   `spc-initialized` (for example `rm bark/*`) passes that test, reaches
   `BarkStore::open` as `Absent` with no markers, and is re-initialised as
   fresh while `state.sqlite` records a prior initialisation. This is a
   narrower partial-damage case than item A covered and the code matches
   its documented claim ("wholly missing `bark/` directory"), so it is not
   held against this candidate; the one-line tightening is to test
   `datadir.join("bark/db.sqlite").exists()` (or any persister artefact)
   instead of the directory, plus a test that removes the directory's
   contents but not the directory. Apply at integration or with AP-3b.
2. SPEC 11.5 wording "A missing database is the only genuinely-fresh case"
   is now weaker than the code (which also requires no sidecar marker and
   no cross-binding record). Not a safety gap - the code refuses strictly
   more - but the sentence should read "a missing database with no sidecar
   marker and no `state.sqlite` cross-binding record is the only fresh
   case" when the section is next touched.
3. Fatal registration stop tears down the status listener before `run`
   returns `Err` (unchanged from r1): the `error` state is observable via
   exit status and logs, not by polling `/status` afterwards. Operator-notes
   item.

Local artefacts written by this re-review: this appended section,
`/tmp/opencode/AP3a-fable-r2-actual.patch`,
`/tmp/opencode/AP3a-fable-r2-agent-test.log`.
