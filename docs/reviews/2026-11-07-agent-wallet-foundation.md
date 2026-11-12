# AP-3a agent wallet foundation implementation report

Date: 2026-11-07 (written 2026-11-08; reworked 2026-11-08, fix-r1)
Author: TFF implementation agent
Status: **AP-3a COMPLETE (fix-r1) - bounded slice only; AP-3b explicitly not claimed**
Worktree: `/home/ethan/.local/share/opencode/worktree/ad8886/lab-agent-wallet`
Base: `b0ab4ca34fda07c61a6335d05f598eaacc69678c` (frozen lab source, uncommitted)
Plan: `lab-experiment-expansion/docs/plans/2026-11-07-lab-ark-services.md`
(SS 3.3, 4.1.1, 4.4, 4.7; AP-3 packet)

No commits, pushes, resets, stashes, cleans, subagents, or main-tree edits were
made. All changes are uncommitted in the worktree. The frozen source and its
verification record are unchanged inputs (section 8).

## 0. Rework (2026-11-08, fix-r1)

The 2026-11-08 Fable review returned REQUEST_CHANGES with five blockers and
six should-fixes. All eleven are dispositioned below; every code/doc change is
in this worktree, uncommitted, on top of the original AP-3a state (pre-rework
snapshot: `/tmp/opencode/AP3a-fix-r1-base/`, 383 files; patch:
`/tmp/opencode/AP3a-fix-r1.patch`; rework manifest:
`/tmp/opencode/AP3a-fix-r1-manifest.json`). New evidence lives under
`/tmp/opencode/AP3a-fix-r1-*`; the frozen parent manifest and all pre-rework
evidence are untouched inputs.

1. **Stock bark persister (blocker)**: done. `bark_store.rs` opens the pinned
   stock `SqliteClient` (bark `c419159`) at `<datadir>/bark/db.sqlite` and
   initializes through the stock `BarkPersister::init_wallet` with
   `WalletProperties(network, fingerprint, None, None)` - no in-tree sqlite
   mirror of bark's schema. Fresh vs damaged is decided before any mutating
   open: a read-only pre-check plus a two-marker sidecar lifecycle
   (`spc-creating` before the stock client first opens a fresh file,
   `spc-initialized` only after `init_wallet` commits). Missing/0-byte file =
   fresh; properties row = validate identity (normal reopen); no row +
   completion marker = damaged, refused; no row + creation marker = our own
   crashed creation, completed; bark-shaped tables with no row and no markers
   = foreign, refused. Refusals leave the bytes untouched; directories 0700
   and files 0600 exist before sqlite opens anything. 9 unit tests + a
   role-level restart test (`role_opens_the_stock_bark_persister_across_restarts`).
   Scope is honest: this is the persister **foundation** - no `Wallet::open`,
   no live balance, no recovery claims (those are AP-3b; SPEC 11.5 now says
   so explicitly).
2. **Fatal terminals + counter exhaustion (blocker)**: done. Hard terminals
   (`bad-signature` etc.) and request-id exhaustion are fatal immediately:
   exactly one send (verified by exact send/id counts via a scripted shared
   transport), status `error` with the exact code, `run()` returns `Err`, and
   further attempts refuse without sending until an operator restart. The
   wedge kinds (`mismatch`/`stale`/`superseded`) get a bounded unwedge - at
   most ONE fresh same-payload retry, then a second consecutive terminal for
   the same payload is fatal; success or a payload change resets the streak.
   All non-success outcomes sit in the 1/2/4/8/16/30 s backoff; after six
   failed attempts local `registration_unavailable (6+ failed attempts)` is
   surfaced (deliberately distinct from the per-attempt
   "registration transport unavailable (attempt N)" message) while the 30 s
   retries continue. Bounded-time tests use `#[tokio::test(start_paused =
   true)]` (tokio `test-util` dev-dep; Cargo.lock unchanged - no vendor-hash
   impact).
3. **Branch-index validation (blocker)**: done. `load_branch_xprv` checks the
   exact depth-4 hardened child (`1'` scan, `2'` ark - a `3'` file is
   refused), network magic, and the shared parent fingerprint between the
   two branch files (`BranchesNotSiblings`). The 4-byte fingerprint is
   documented as required provisioning-metadata consistency, not ancestry
   proof. Negative tests: 3' branch refused, foreign master refused; the
   valid fixture is unchanged.
4. **Effective registration binding (blocker)**: done. State schema v2 with
   `effective_digest`/`effective_request_id`: registered means the *current*
   payload matches the most recent **acknowledged** `applied`/`already_applied`
   state - not any historical row (applied A -> B -> A does not re-register
   A), `superseded` clears the binding it names, an interrupted B/unknown
   verdict flips nothing, and the durable counter/pending-replay semantics
   survive. A v1 database is rejected explicitly (`SchemaVersion { found: 1,
   expected: 2 }`), never silently adopted. The AP-2 verdict-to-outcome
   mapping stays a seam concern (section 7).
5. **SPEC + decision record + example config (blocker)**: done.
   `docs/decisions/2026-11-08-agent-wallet-foundation.md` (dated record);
   SPEC 11.5 gains the exact BIP 85 derivation, provisioning shape,
   persistence layout, client journal/terminal policy, status honesty and the
   AP-3b reorg-rewind dependency; 11.10 gains the spc-agent `[ark]` keys;
   the changelog entry lands as an actual fragment file,
   `docs/changelog/2026-11-08-agent-wallet-foundation.md` (merge-queue
   convention; byte-exact bullet, leading two spaces and `- "` quoting),
   which the merge session splices into the SPEC frontmatter at serial
   integration - the SPEC frontmatter itself is untouched, and the
   concurrent AP-2 11.4/11.9 fragment is a disjoint file.
   `config/spc-agent.example.toml` documents the six
   AP-3a `[ark]` keys (`scan_xprv`, `ark_xprv`, `datadir`, `status_listen`,
   `network`, `ark_id`; `seed_child` pre-existed) - paths/ids only, no
   secret values; `example_config` test green. DESIGN.md precedence and
   AP-2's 11.4/11.9 edits untouched (merge note 6.6 is superseded).
6. **(should-fix) registered != connected**: done. Without a transport the
   role is `dormant` even with a persistent ack - `registered` is the
   durable journal answer, a live connection is never invented.
7. **(should-fix) exhausted counter is fatal**: folded into item 2.
8. **(should-fix) fixture honesty**: done. Explicit independent constants
   `FIXTURE_EXPIRY_IN_WINDOW` (800_432, inside the tip+96 window) and
   `FIXTURE_EXPIRY_OUT_OF_WINDOW` (900_000, outside), per-block
   `expiry_provenance` + top-level `provenance` labels; `load_fixture`
   asserts the SYNTHETIC label. Ledger replay is documented as key
   derivation + ledger replay - not real ancestry/mailbox/VTXO recovery -
   and `VtxoState::Spendable` is caller-asserted. Fixture regenerated
   (deterministic; only expiry constants, provenance fields and the
   `needs_refresh_sats` expectation changed).
9. **(should-fix) reorg rewind as explicit AP-3b dependency**: done. The
   scan-cursor docs state monotonic-only with journaled reorg rewind
   (record fork point, replay forward, then advance) as an AP-3b dependency;
   `set_scan_height` refuses backward moves; nothing is implemented in
   secret here.
10. **(should-fix) defense-in-depth details**: done. Loopback enforced at
    bind (programmatic addresses included); best-effort key zeroize on drop
    (branch secrets, child seed, mailbox/normal keypairs -> dummy key);
    private file creation before open (`create_new` + 0600); foreign
    non-empty sqlite rejected; targeted failure tests for each.
11. **(should-fix) Nix provenance correction**: done - see section 5. The
    pre-rework AP3a-nix logs recorded *build announcements* (the sandbox
    stdout), not test output; the corrected procedure is `nix log <drv>`.
    `path:` sources include untracked files (that is why the checks must be
    built from a non-git copy or `path:`), and the parent manifest delta
    was 22 rows (not the HEAD dirty count).

## 0.1. Rework 2 (2026-11-08, fix-r2)

The Fable re-review of fix-r1 returned three narrow REQUEST_CHANGES
findings (A/B/C); findings 2-11 of the original review were confirmed
closed. All three are dispositioned here. Fix-r2 evidence is kept
separate from the fix-r1 receipts (new logs under
`/tmp/opencode/AP3a-fix-r2-*`); no r1 log was relabelled.

- **A (code + tests) - markers before mutation**: done. `bark_store.rs`
  now inspects the sidecar markers **before any mutating step,
  regardless of whether the database file exists**: a missing or 0-byte
  `db.sqlite` with `spc-initialized` present is `Corrupt` (partial
  datadir damage - the deliberate reset removes the whole datadir,
  never `bark/` alone), never re-initialized; a missing database with
  only `spc-creating` cannot arise from this module's ordering (the
  file is created before the marker) and is refused; a 0-byte database
  with only `spc-creating` is the module's own crash window and may
  complete. The completion-marker absence is re-checked immediately
  before `init_wallet`, so `create_new` on it can only ever fail on a
  true concurrent race, never after a mutation. New regressions:
  delete-db-only -> `Corrupt` with no database created and markers
  untouched; truncate-to-zero -> `Corrupt`, still 0 bytes, identical on
  a repeated attempt; creating-marker-without-db -> refused, nothing
  created; empty-db + creating-marker -> completes (the crash window).
  The reviewer's optional cross-binding is implemented (bounded):
  `state.sqlite` meta records `bark_initialized` after every validated
  open, and a wholly missing `bark/` while the agent store survives is
  refused as partial damage at role start (the pre-fix-r2 limitation -
  stores created before the binding existed are healed on their next
  validated open, and are not covered until then). The unit test
  `datadir_loss_deterministically_reinitializes_the_same_child`
  (BarkStore-level, whole `bark/` removed) and all other existing
  bark/recovery tests stay green.
- **B (docs) - changelog fragment**: done. The fix-r1 frontmatter edit
  is reverted (the SPEC `changelog:` list and `updated:` are back to
  the pre-packet baseline; the 11.5/11.10 body hunks stay). The exact
  bullet now lives as
  `docs/changelog/2026-11-08-agent-wallet-foundation.md` (merge-queue
  convention, byte-exact, leading two spaces and `- "` quoting); the
  merge session splices it at serial integration. The decision record
  and this report now describe the actual mechanism (sections 0.5, 6.6,
  and the record's "Sections edited").
- **C (docs) - report overclaims**: done. Section 1 no longer claims
  ancestry parsing (bounded ledger; expiry provenance recorded as
  supplied; no finding height ever used as an expiry). Section 4 says
  "identical `U_i` set and identical synthetic fixture record ids", not
  real VTXO recovery. Section 5 uses the verified per-suite counts
  (agent crate 126 across 10 binaries; whole check 468 across 26),
  replacing the incorrect "18 suites, 137 passed" headline.

Fix-r2 test evidence (fresh receipts, `-j 2`, unchanged lock/deps; all
re-run against the final source after a cosmetic `mut` cleanup, so every
log is bound to exactly the frozen r2 code):

```
$ cargo test -p spc-agent --features ark --lib -j 2
108 passed, 0 failed   (was 104 in fix-r1; +4 new bark marker cases)

$ cargo test -p spc-agent --features ark --test wallet_registration -j 2
10 passed, 0 failed    (was 9; +1 whole-bark-dir-loss cross-binding case)

$ cargo test -p spc-agent --features ark -j 2
131 passed, 0 failed, 1 ignored across the crate's 10 test binaries
(lib 108, wallet_registration 10, wallet_receive 3, wallet_recovery
4+1 ignored, example_config 1, e2e 2/2/1, main/doc-tests 0)

$ cargo fmt --all --check
clean

$ cargo clippy -p spc-agent --all-targets --features ark -- -D warnings
clean (rc=0)
```

Logs: `/tmp/opencode/AP3a-fix-r2-lib-test.log`,
`/tmp/opencode/AP3a-fix-r2-registration-test.log`,
`/tmp/opencode/AP3a-fix-r2-agent-tests.log`,
`/tmp/opencode/AP3a-fix-r2-fmt.log`, `/tmp/opencode/AP3a-fix-r2-clippy.log`.
The fix-r1 workspace (641/81) and Nix (468/26) receipts remain the
standing evidence for the unchanged remainder - fix-r2 touched only
`bark_store.rs`, `state.rs` (two meta methods), `mod.rs` (the
cross-binding), the registration tests, the changelog fragment, the SPEC
frontmatter revert, the decision record and this report; no other code
changed, so no workspace or Nix re-run was warranted.

Explicit iteration delta (original AP-3a -> fix-r1 -> fix-r2):
original lib 78 / registration 5; fix-r1 lib 104 / registration 9;
fix-r2 lib 108 / registration 10. The fix-r1 manifest (385 files,
15 delta rows) is a frozen input; the fix-r2 manifest and patch are new
artifacts (section 10).

## 1. Scope

AP-3a is the first bounded slice of persistent wallet support for the agent
(`--roles wallet`, cargo feature `ark`):

- load exactly the provisioned key material (scan branch xprv
  `m/352'/0'/0'/1'`, ark branch xprv `m/352'/0'/0'/2'`, BIP 85 child seed,
  public b_spend from sp_address; never the master or the spend branch);
- open (or first-bind) the persistent `U_i -> u_i` keystore, VTXO ledger,
  per-identity monotonic registration counter/journal (with the **effective
  registration binding**, schema version 2) and scan cursor
  (`<datadir>/state.sqlite`, fail closed on corruption, mismatched
  identity, schema-version drift, or counter exhaustion - no wrap);
- open the **stock bark persister** foundation at `<datadir>/bark/`
  (pinned `SqliteClient` + stock `init_wallet`; fail closed on damage or
  foreign files - the full bark `Wallet::open` is AP-3b);
- drive the registration state machine against a transport **seam** (AP-2 owns
  the `0x5350` codec; without it the role stays explicitly
  `dormant` and never mines);
- receive/claim prototype logic behind backend traits with persistence and
  validation tests (a bounded ledger: expiry provenance is recorded as
  supplied by the caller, and no finding height is ever used as an
  expiry - there is no ancestry parser; real VTXO ancestry parsing is
  AP-3b);
- expose the honest loopback status JSON (unknown = null, never zero).

Explicitly **not** in AP-3a: funds UI, send endpoint, mining, live Ark RPCs,
the full stock bark `Wallet::open`, chain scan, delegated refresh/offboard
(see section 9).

## 2. Files changed (exact, vs the frozen source)

New (AP-3a-owned):

- `crates/spc-crypto/src/hd.rs` - BIP 85 / SPC HD derivation helpers
  (APP-39 child entropy, branch xprvs, bark root/mailbox/vtxo keypairs,
  `provision()` for fixtures)
- `bins/spc-agent/src/wallet/mod.rs` - `WalletRole` (start/run/shutdown),
  `WalletSettings::from_config`, `REFRESH_THRESHOLD_BLOCKS = 96`
- `bins/spc-agent/src/wallet/keys.rs` - key loading/validation
  (0600-or-refuse, depth==4 hardened branches, network magic, scan branch
  must derive sp_address's B_scan, B_ark != B_spend, bark fingerprint
  cross-check)
- `bins/spc-agent/src/wallet/state.rs` - sqlite store: identity binding,
  keystore, VTXO records (forward-only transitions), registration
  counter/pending journal, scan cursor
- `bins/spc-agent/src/wallet/registration.rs` - semantic
  `RegistrationPayload` (no wire bytes), `RegistrationManager::attempt`,
  request-id policy, `Backoff` (1,2,4,8,16,30 cap 30), `RegistrationOutcome`
- `bins/spc-agent/src/wallet/receive.rs` - `ReceiveBackend` trait,
  `ReceiveLedger` (U_i -> u_i derivation check, owned-UTXO filter,
  reconciliation alarms)
- `bins/spc-agent/src/wallet/status.rs` - loopback status service
  (axum 0.8, `GET /status`)
- `bins/spc-agent/tests/wallet_common/mod.rs` - shared test helpers
  (provisioning, mock transport, mock receive backend, raw-TCP HTTP client)
- `bins/spc-agent/tests/wallet_registration.rs` (5 tests)
- `bins/spc-agent/tests/wallet_receive.rs` (3 tests)
- `bins/spc-agent/tests/wallet_recovery.rs` (4 tests + `#[ignore]`d
  deterministic fixture generator)
- `bins/spc-agent/tests/fixtures/ap3a-recovery-fixture.json` - frozen
  recovery fixture (deterministic master, regtest, 2 blocks at heights
  800000/800001)

Modified (wiring):

- `Cargo.toml` - workspace dependency `bip39 = "2.2"` (bark wallet seed)
- `Cargo.lock` - new dep edges (see merge note 6.3)
- `flake.nix` - `test-ark` check now also runs
  `cargo test -p spc-agent --features ark --offline`; `cargoDeps`
  fetchCargoVendor hash updated (see merge note 6.4)
- `crates/spc-crypto/Cargo.toml` + `crates/spc-crypto/src/lib.rs` - `hd`
  module and its wiring (overlaps AP-2 owner paths, see merge note 6.1)
- `bins/spc-agent/Cargo.toml` - feature
  `ark = ["spc-crypto/ark", "dep:bark-wallet", "dep:rusqlite", "dep:axum", "dep:chrono"]`
- `bins/spc-agent/src/config.rs` - `[ark]` fields: `scan_xprv`, `ark_xprv`,
  `datadir`, `status_listen` (default `127.0.0.1:34256`, loopback-only
  enforced), `network` (optional; falls back to HRP-derived),
  `ark_id` (8 hex chars, required by the wallet role); `seed_child` reused.
  New `ConfigError` variants: `StatusListen`, `StatusNotLoopback`,
  `Network`, `ArkId`
- `bins/spc-agent/src/lib.rs` - `validate_role_selection` (wallet never
  co-runs with translator/jdc; used by main.rs, unit-tested)
- `bins/spc-agent/src/main.rs` - `--roles wallet` entry (bails
  "requires --features ark" without the feature; calls
  `wallet::run_from_config` with it)

No other files differ from the frozen source (verified, section 8). No
pool/store/Ark-role code was touched; the `0x5350` codec was not forked or
duplicated.

Fix-r1 additionally (all vs `/tmp/opencode/AP3a-fix-r1-base/`, the frozen
pre-rework snapshot; 15 delta rows in the rework manifest, section 10):
new `bins/spc-agent/src/wallet/bark_store.rs` and
`docs/decisions/2026-11-08-agent-wallet-foundation.md`; modified
`bins/spc-agent/Cargo.toml` (tokio `test-util` dev-dep), the wallet sources
(`keys.rs`, `mod.rs`, `receive.rs`, `registration.rs`, `state.rs`,
`status.rs`), `wallet_registration.rs`, `wallet_recovery.rs`, the regenerated
fixture, `config/spc-agent.example.toml`, `docs/SPEC.md`, and this report.
`Cargo.lock`, the root `Cargo.toml` and `flake.nix` are **unchanged** by
fix-r1 (no vendor-hash impact).

## 3. Key behaviors (as implemented and tested)

- **Provisioned material only**: the agent loads branch xprvs and the child
  seed from files that must be exactly 0600 (never chmod'd). Master or spend
  branch material on the agent has no code path. `spc_crypto::provision()`
  exists for test fixtures only.
- **Persistent child wallet**: the stock bark persister foundation
  (`bark_store.rs`): pinned stock `SqliteClient` at `<datadir>/bark/db.sqlite`,
  stock `init_wallet` with `WalletProperties(network, fingerprint, None,
  None)`, two-marker sidecar lifecycle deciding fresh vs damaged vs foreign
  before any mutating open, 0700/0600 created before sqlite opens them, and
  a reopen that re-validates network + fingerprint with the bytes untouched
  on refusal. No `Wallet::open`, no live balance, no recovery - AP-3b.
- **Counter policy** (plan 4.1.1): the request id is reserved durably before
  any send; retries of the same attempt (same payload digest) reuse it;
  transport failures and retryable server errors keep the reservation;
  terminal outcomes journal and clear it so a fresh id unwedges; `u32::MAX`
  is refused, never wrapped.
- **Terminal policy (fix-r1)**: hard terminals and counter exhaustion are
  fatal - one send, status `error` with the exact code, `run()` returns
  `Err`, no further attempts until an operator restart; wedge kinds
  (`mismatch`/`stale`/`superseded`) get exactly one fresh same-payload
  retry, then fatal; every non-success outcome sits in the 1/2/4/8/16/30 s
  backoff, and after six failed attempts local `registration_unavailable`
  is surfaced while the 30 s retries continue.
- **Effective binding (fix-r1)**: `registered` means the current payload
  matches the most recent acknowledged `applied`/`already_applied` state
  (schema v2) - a historical journal row is not liveness, `superseded`
  clears the binding it names, and a v1 database is rejected explicitly.
- **Fail closed**: corrupt state files, schema version mismatches (including
  v1), identity (fingerprint/sp_id/network) mismatches, half-specified or
  backwards VTXO records, non-loopback status addresses, wrong-network keys,
  non-sibling branch files, foreign or damaged bark databases - all refuse
  to start; state is never wiped or regenerated to make a mismatch go away,
  and a failed restore never regenerates seed material.
- **Honest dormancy**: without a transport the role is `dormant` (distinct
  from `awaiting_registration`, which means a transport is wired but no ack
  yet) with `last_error` explaining the missing AP-2 codec - even when a
  registration was acknowledged persistently; it never pretends to be
  connected and never mines. Status JSON fields follow plan 4.7 plus
  `state`; unknown values are `null`, never zero; no keys, VTXO ids or
  sp_id cross the status surface.
- **Graceful stop**: shutdown via a `tokio::sync::watch` channel (the
  initial receiver is held in the role so a pre-`run` shutdown is not
  dropped); the status port is released and the state moves to `stopped`.

## 4. Test evidence (exact commands, this worktree)

All run in `/home/ethan/.local/share/opencode/worktree/ad8886/lab-agent-wallet`
(plain cargo; `--offline` fails in this environment due to a pre-existing
psm/pest cache quirk, not this change):

```
$ cargo test -p spc-agent --features ark
lib (ark):            104 passed, 0 failed
wallet_registration:   9 passed, 0 failed
wallet_receive:        3 passed, 0 failed
wallet_recovery:       4 passed, 0 failed (1 ignored: regenerate_fixture)
example_config:        1 passed, 0 failed
existing integration:  5 passed, 0 failed (e2e 2/2/1)

$ cargo test --workspace        # featureless, regression check
81 test binaries, 641 passed, 0 failed

$ cargo test -p spc-crypto --features ark
lib: 102 passed, 0 failed (includes the BIP 85 APP-39 spec vector, pinned
     from the BIP 85 test-vector master root key)

$ cargo clippy --workspace --all-targets -- -D warnings
clean (pre-existing upstream sqlx-postgres future-incompat note only)

$ cargo clippy -p spc-agent --all-targets --features ark -- -D warnings
clean

$ cargo clippy -p spc-crypto --all-targets --features ark -- -D warnings
clean

$ cargo fmt --all --check
clean
```

Fix-r1 evidence logs (exact commands and outputs):
`/tmp/opencode/AP3a-fix-r1-cargo-evidence.txt` (targeted suites + clippy/fmt),
`/tmp/opencode/AP3a-fix-r1-workspace-test.log` (workspace regression, rc=0).

Feature-gated wallet code is **not** covered by the flake's default clippy/
test checks; the `test-ark` check was extended accordingly (merge note 6.4)
and the feature-gated clippy passes above are run manually.

Notable test cases (see the three `wallet_*.rs` files): corrupt keystore
secret refused; missing/wrong-network keys refused; counter reuse/restart/
terminal-unwedge/`u32::MAX` refusal; keystore persistence and ownership;
status redaction (unknown = null); graceful stop; restart reuses pending
request id; recovery from datadir destruction recreates state from the same
seed with identical `U_i` set and identical synthetic fixture record
ids (the fixture's VTXO records are deterministic test ids, not real
VTXOs - no real VTXO recovery is claimed); the committed fixture is
guarded by
`fixture_matches_its_own_master_seed`; wallet/translator/jdc co-run refused.
Fix-r1 additions: exact send/id counts for fatal and bounded-unwedge paths
(`run_stops_fatally_on_bad_signature_after_one_send`,
`run_stops_fatally_after_two_consecutive_wedge_terminals`,
`run_surfaces_unavailable_and_keeps_retrying_until_shutdown` - paused time,
bounded); the bark persister lifecycle (fresh/reopen/damaged/foreign/
crashed-creation/network-refusal/datadir-loss, plus the role-level restart
test with 0700/0600 assertions); schema v1 rejection; effective-binding
applied A -> B -> A, interrupted B, superseded clearing; 3' branch and
foreign-master refusals; the crafted sp_address exercising
`ArkKeyIsSpendKey` with the real key files restored first; dormancy without
a transport.

## 5. Nix check evidence

Fix-r1 (the corrected provenance - see item 0.11): the sandbox **stdout**
that `nix build` prints is build announcements only; the test output is in
`nix log <drv>`. The check below captures both, plus the derivation path and
the `path:` source binding (which includes untracked files - that is exactly
why the check is built from a non-git copy, since a dirty git tree filters
to tracked files and loses the untracked workspace members; merge note 6.5).

```
$ nix build -j 2 path:/tmp/opencode/AP3a-fix-r1-nix-src#checks.x86_64-linux.test-ark
exit 0
  derivation: /nix/store/q5x5r45n82s34kiwf623mhr7wkc2v41b-spc-ark-feature-tests-0.1.0.drv
  output:     /nix/store/3iy69p8k2q2m1p0xllsvid7f1aysw4f7-spc-ark-feature-tests-0.1.0
  source:     path:/tmp/opencode/AP3a-fix-r1-nix-src
              narHash=sha256-Y6dtNOB0vIKu/5sSz5gURK++VT4myBrx3CExQVkP6fs=
              -> /nix/store/v2fb8x2pqajp5j1icn0cbzv5mcrsq6ph-source (385 files)
  nix log:    whole check 468 passed, 0 failed (1 ignored) across 26
              test binaries. Per crate: spc-crypto (ark) 102 across 6,
              spc-pool (rpc,ark) 240 across 10, spc-agent (ark) 126
              across 10 (lib 104, wallet_registration 9 with the
              fatal/bounded-unwedge/exhaustion and bark-restart tests,
              wallet_receive 3, wallet_recovery 4+1 ignored,
              example_config 1, e2e 2/2/1, main/doc-tests 0);
              checkPhase 2m50s
```

Log: `/tmp/opencode/AP3a-fix-r1-nix-test-ark-run.log` (build announcements +
`nix log` test output + source binding + copy content check).

Fallback note: `nix build -j 2 path:<worktree>#...` was attempted directly on
the worktree first, but the `path:` store copy ingests the 30G `target/`
build cache and was about to exhaust the disk, so it was killed and the
anticipated rsync fallback used (`.git`, `target/`, `result*` excluded;
untracked source files included, 385 files - the untracked workspace members
are present, which is the property the copy exists to preserve). `Cargo.lock`
and `flake.nix` are unchanged by fix-r1, so no vendor-hash update was needed
(and any future one belongs to the merge session).

Pre-rework evidence (superseded provenance - the logs below record build
announcements, not test output): `nix build -j 2` from
`/tmp/opencode/AP3a-nix-src`; logs
`/tmp/opencode/AP3a-nix-{test-ark-full,test-full,clippy-full,fmt-full}.log`.
The parent review manifest's delta was 22 rows (not the HEAD dirty count).

## 6. Merge notes

1. **`crates/spc-crypto` overlap with AP-2** (lab-ark-registration owns
   spc-crypto editor paths in its packet): AP-3a adds `src/hd.rs` (new file)
   and touches `src/lib.rs` + `Cargo.toml` only to wire the module. AP-2's
   edits are in spc-ext/store/registration areas; the `lib.rs`/`Cargo.toml`
   hunks are small (module declaration, no new spc-crypto deps beyond the
   already-pinned `ark-lib` behind the existing `ark` feature). Merge both
   worktrees' spc-crypto trees; the only expected conflict surface is the
   `lib.rs` module list and `Cargo.toml` feature list.
2. **`bins/spc-agent` wiring** is AP-3a-owned (config/main/lib/Cargo.toml);
   AP-2 does not edit the agent binary per its packet.
3. **`Cargo.lock`**: new edges from the spc-agent `ark` feature - optional
   `rusqlite` 0.31 (+ `libsqlite3-sys`), `axum` 0.8 (+ tower/hyper/http
   transitive set), `chrono`, `bark-wallet` (git-pinned bark `c419159`,
   already in the lock via spc-crypto/spc-pool), `bip39` 2.2 (workspace dep
   added to the root `Cargo.toml` for the bark seed). No existing pins moved.
4. **`flake.nix`**: `test-ark` checkPhase gains
   `cargo test -p spc-agent --features ark --offline`; the `fetchCargoVendor`
   hash changed
   `sha256-TET0F9jXQELPadNaPKBiPXgFNGPnU9uRaJWjkSAX52w=`
   -> `sha256-yyjqKI9Okm7Jr4Qd/fRgvcjC986HI4LriGAxLp39YY8=`
   because the lockfile grew (probe log:
   `/tmp/opencode/AP3a-nix-hash-probe.log`). The default `spc-agent` package
   remains featureless (the binary bails without `--features ark`).
5. **Nix visibility caveat (affects every lab worktree, not just this one)**:
   the worktrees carry untracked frozen-source crates (e.g. `crates/spc-lab`,
   `bins/spc-lab`) and all AP-3a files are uncommitted, and Nix filters a
   dirty git tree to *tracked* files only - so `nix build .#checks...`
   evaluated inside any of these worktrees fails on missing workspace
   members (and the flake's `cleanSourceWith` filter cannot fix it). The
   checks above were therefore built from an rsync copy with `.git`,
   `target/` and `result*` removed (content-identical; the copy is at
   `/tmp/opencode/AP3a-nix-src`). The merge session should re-run the checks
   from the merged tree once the frozen crates and this work are committed,
   at which point in-worktree `nix flake check` works again.
6. **SPEC and decision record (fix-r1, supersedes the original note;
   fix-r2 corrects the changelog mechanism)**: the original AP-3a
   deliberately made no SPEC edit to avoid divergence with AP-2's
   concurrent 11.4/11.9 work. The rework review required the normative
   basis, so fix-r1 lands it: the dated record
   `docs/decisions/2026-11-08-agent-wallet-foundation.md`, the SPEC
   11.5/11.10 body edits, and the changelog entry as an actual fragment
   file `docs/changelog/2026-11-08-agent-wallet-foundation.md`
   (merge-queue convention, `docs/changelog/README.md`; the merge session
   splices it into the SPEC frontmatter at serial integration and
   deletes the fragment - the SPEC frontmatter itself is not edited by
   this packet, so the concurrent AP-2 11.4/11.9 fragment never
   conflicts). AP-2's 11.4/11.9 areas are untouched;
   DESIGN.md keeps precedence. The later record
   `2026-11-08-ark-refresh-normal-wallet.md` and the refresh/offboard SPEC
   deltas still land with AP-3b, where the behavior they govern is
   implemented.

## 7. AP-2 integration dependencies (handoff)

- **`0x5350` codec**: AP-2 (lab-ark-registration) owns the spc-ext wire
  codec. The wallet role consumes it exclusively through the
  `RegistrationTransport` seam; `registration.rs` carries semantic payloads
  only (`RegistrationPayload`, local sha256 digest for pending-id reuse).
  The adapter must live on the AP-2 side (or a follow-up) and encode the
  payload into the `spc:arkreg` frame.
- **Request-id/counter semantics**: the front must treat `request_id` as
  monotonic per sp_id; the ack mapping must produce the
  `RegistrationOutcome` variants (`Applied`, `AlreadyApplied`, `Superseded`,
  `Mismatch`, `Stale`, retryable error codes). The agent-side policy
  (pending-id reuse on same digest, fresh id after terminal outcomes) is
  already implemented and tested here against mocks.
- **Ack flow**: retryable error codes (e.g. `registration-commit-unknown`)
  must keep the pending reservation on the agent; the outcome mapping table
  in `registration.rs` is the contract.
- **The exact seam (fix-r1 clarification)**: the adapter implements

  ```rust
  pub trait RegistrationTransport: Send {
      fn send(&mut self, payload: &RegistrationPayload,
              request_id: u32) -> anyhow::Result<RegistrationOutcome>;
  }
  ```

  and nothing else crosses the boundary: `RegistrationPayload` is semantic
  (the ark_id, b_ark_pub, mailbox fields and the local sha256 digest used
  for pending-id reuse - no wire bytes), the transport owns the `0x5350`
  encoding into the `spc:arkreg` frame, and every server ack maps to
  exactly one `RegistrationOutcome` (`Applied`, `AlreadyApplied`,
  `Superseded`, `Mismatch`, `Stale`, `Terminal { error_code }`,
  `Retryable { error_code }`) or a transport error (`Unavailable` is
  local-only: the front could not be reached). The agent-side terminal
  policy (section 0, item 2) consumes these variants as-is; the AP-2 front
  must not invent new semantics for unknown server codes - unknown maps to
  `Terminal` and stops the loop (fail closed). The effective binding
  (schema v2) is fed only by acknowledged `Applied`/`AlreadyApplied`
  verdicts against the *current* payload digest.

## 8. Frozen source verification

```
$ python3 /tmp/opencode/AP3a-verify-manifest.py
VERIFY OK: 370 files, full=9ffff6da code=30fedafd

$ python3 /tmp/opencode/AP3a-verify-root.py   (SRC=/tmp/opencode/LP-final-runtime-src-v4)
VERIFY OK: 370 files, full=9ffff6da code=30fedafd
```

The frozen source copy is intact. A recursive diff of the worktree against
`/tmp/opencode/LP-final-runtime-src-v4` (excluding `.git`, `target`,
`result*`, `cache`, `temp`, `spc_ark_client/test`) shows exactly the file
set listed in section 2 - nothing else changed, nothing removed.

Git state: `HEAD` = `b0ab4ca34fda07c61a6335d05f598eaacc69678c` (base, no
commits made), 0 stashes, 0 staged files, 107 uncommitted path entries
(frozen-source delta + AP-3a work + fix-r1 rework), no subagents used.

## 9. AP-3b remainder (explicitly not done here)

The exact remainder after fix-r1 (the persister foundation from item 0.1 is
deliberately excluded from these claims):

- Open the full stock bark `Wallet` over the persister foundation
  (`Wallet::open` needs a live chain source; fix-r1 persists, initializes
  and re-validates the stock persister only - no live balance, no VTXO
  recovery from bark's own tables).
- Chain scan via spc-scan, including the **journaled reorg rewind** the
  monotonic cursor defers to (record the fork point, replay forward, then
  advance; `set_scan_height` refuses backward moves until it exists - this
  is an explicit AP-3b dependency, documented in SPEC 11.5 and the state
  store, not hidden).
- Real VTXO ancestry parsing and live mailbox reads behind
  `ReceiveBackend` (AP-3a/fix-r1 tests are honest about being key
  derivation plus ledger replay over a synthetic fixture; no fake Ark
  server, no DB-credit shortcuts).
- Live registration transport (the `0x5350` adapter; AP-2 owns the codec -
  section 7) and live status/mailbox RPCs.
- Delegated VTXO refresh and offboard (`REFRESH_THRESHOLD_BLOCKS = 96` and
  expiry alarms exist; no refresh/offboard transactions).
- Mailbox modes `static` and `epoch_registered` (need the Ark server's
  `S_mb` / RegisterEpochMailboxes; currently refused fail-closed in
  `WalletSettings::from_config` with a pointer to AP-3b).
- Decision record `2026-11-08-ark-refresh-normal-wallet.md` plus the
  refresh/offboard SPEC deltas it carries.

## 10. Fix-r1 bindings (source-of-truth artifacts)

- Pre-rework snapshot (immutable diff base): `/tmp/opencode/AP3a-fix-r1-base/`
  - 383 files, the worktree as the Fable review received it.
- Rework patch: `/tmp/opencode/AP3a-fix-r1.patch` - unified diff of the
  worktree vs that base (excluding `.git`, `target`, `result*`).
- Rework manifest: `/tmp/opencode/AP3a-fix-r1-manifest.json` (recipe
  identical to the parent `/tmp/opencode/AP3a-review-manifest.json`):
  - `files`: 385 rows `{path, mode, sha256}` sorted by path (the parent's
    383 + `bark_store.rs` + the decision record; `.git`/`target`/`result*`
    excluded);
  - `delta`: 15 rows vs the frozen parent manifest (the rework's exact
    footprint: the 14 code/doc rows from section 2's fix-r1 list plus this
    report);
  - `code_snapshot_sha256`:
    `e6af84a91714a9f85cbf04fcbb12cf976d02431b38fd144e7366d0ec174fdd5c`
    (SHA256 of sorted-key compact JSON `{base, files}` excluding only this
    report - so the value is stable against this report's own edits);
  - `snapshot_sha256`: same recipe over all 385 rows including this report
    (recorded in the manifest artifact; not cited here to avoid
    circularity).
- Verification (reviewer-side recompute): `python3
  /tmp/opencode/AP3a-fix-r1-verify-manifest.py` re-walks the worktree with
  the documented exclusions and re-derives `code_snapshot_sha256`.
- Evidence: `/tmp/opencode/AP3a-fix-r1-cargo-evidence.txt`,
  `/tmp/opencode/AP3a-fix-r1-workspace-test.log`,
  `/tmp/opencode/AP3a-fix-r1-nix-test-ark-run.log`. The frozen parent
  manifest and all pre-rework evidence (`/tmp/opencode/AP3a-*`) are
  untouched inputs.

### Fix-r2 bindings (new artifacts; the fix-r1 set above is a frozen input)

- Fix-r2 diff base (immutable snapshot of the fix-r1-final worktree):
  `/tmp/opencode/AP3a-fix-r2-base/` - 385 files, verified row-by-row
  against the frozen fix-r1 manifest (0 mismatches) before any r2 edit.
- Fix-r2 patch: `/tmp/opencode/AP3a-fix-r2.patch` - unified diff of the
  worktree vs that base (excluding `.git`, `target/`, `result*`); its
  touched-file set is exactly the 8 delta rows below.
- Fix-r2 manifest: `/tmp/opencode/AP3a-fix-r2-manifest.json` (same hash
  recipe as the fix-r1 manifest; the delta baseline is now the frozen
  fix-r1 manifest, `baseline_manifest_sha256` =
  `5c7dbd7f1f565ab3851d79c3266e41eb7cf14751737be04c53b883ae36640b28`):
  - `files`: 386 rows `{path, mode, sha256}` sorted by path (the fix-r1
    manifest's 385 + the new changelog fragment; same exclusions);
  - `delta`: 8 rows vs the frozen fix-r1 manifest (the exact fix-r2
    footprint: `bark_store.rs`, `state.rs`, `wallet/mod.rs`,
    `tests/wallet_registration.rs`, `docs/SPEC.md` (frontmatter revert),
    `docs/changelog/2026-11-08-agent-wallet-foundation.md` (new),
    `docs/decisions/2026-11-08-agent-wallet-foundation.md`, and this
    report);
  - `code_snapshot_sha256`:
    `cd09ed78b7b82c795a43b4798608228f300437e38fc50533638eb8b9b86d0530`
    (SHA256 of sorted-key compact JSON `{base, files}` excluding only
    this report - stable against this report's own finalization);
  - `snapshot_sha256`: same recipe over all 386 rows including this
    report (recorded in the manifest artifact; not cited here to avoid
    circularity).
- Verification (reviewer-side recompute): `python3
  /tmp/opencode/AP3a-fix-r2-verify-manifest.py` re-walks the worktree
  with the documented exclusions, re-derives `code_snapshot_sha256`,
  and re-checks the 8 delta rows and the baseline binding against the
  frozen fix-r1 manifest.
- Evidence: `/tmp/opencode/AP3a-fix-r2-lib-test.log` (108/0),
  `/tmp/opencode/AP3a-fix-r2-registration-test.log` (10/0),
  `/tmp/opencode/AP3a-fix-r2-agent-tests.log` (131/0/1 ignored),
  `/tmp/opencode/AP3a-fix-r2-fmt.log`, `/tmp/opencode/AP3a-fix-r2-clippy.log`
  (all rc=0, bound to the frozen r2 code). The fix-r1 manifest, base,
  receipts and the parent `/tmp/opencode/AP3a-*` artifacts are untouched
  inputs; nothing was relabelled.
