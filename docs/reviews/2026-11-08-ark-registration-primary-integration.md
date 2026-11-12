# AP-2 pool-side Ark registration, acknowledgment and destination wiring - primary integration (2026-11-08)

Owner: serial primary Rust/Cargo integration writer (sole writer for this
packet; build-priority reservation held by this session,
`ses_e458119eeffe4Jl3MZjJ4J4HJR`, for the Hub -> history-v2 -> access ->
operator-runtime -> ark-registration serial window).

Approval: `docs/reviews/2026-11-08-ark-registration-fable.md`, Round 3
**APPROVE** of the exact bounded delta (23 modified + 9 added files against
the frozen 370-file LP-final-runtime baseline) frozen as
`/tmp/opencode/AP2-r21-final-manifest.json`, full
`d94bb15841d888724fd987365dbdeb81e3cf816443cd4ed07b2f821c26f78953`, code
`de0e2158b2ea24846e2edf3b3ecefd401f9b98880b933fed5378a092b099956e`.
Conditions verified at merge: R3.1 journal-lookup closure (below), the
SS5 AP-3 handoff boundaries (below), and the R3.0 receipts. Explicitly
NOT claimed: any actual Ark server API change, any funded Ark or mining
acceptance, any AP-3b `current_request_id` wire addition, and any
J1/history-3 work.

External manifest: `/tmp/opencode/AP2-primary-integration-manifest.json`
(final sha256 below).

## 1. Pre-write verification (read-only, at transfer time)

- Candidate manifest reproduced with the declared ordered-array recipe:
  full `d94bb158...` MATCH, code `de0e2158...` MATCH; worktree rows
  379/379 clean, archive rows 379/379 clean; source bytes 5930569 match
  (`AP2-prewrite-reverify.log`).
- The 9 added paths absent from primary; migration slot
  `0006_ark_registration_state.sql` free (re-checked at transfer as
  required; it lands as the last migration).
- Byte-apply gates: 12 of the 23 modified rows had primary ==
  LP-final-runtime-src-v4 base (the other 11 are the three-way files
  where primary moved past base via T4/AP-3/T0/history2/OP).
- Roots at the post-OP pre-state: `Cargo.toml` `8530ce08...`,
  `Cargo.lock` `6ca42938...`, `docs/SPEC.md` `21f47313...` (50 entries),
  `flake.nix` `b4bf0503...` (vendor hash `sha256-PvxybBOJ...`); index
  `44f21a926d4d065ac6937fb0f81b698c7b2d0627` (== HEAD tree, nothing
  staged).

## 2. Transfer (exact approved delta + merge-owner adaptations)

- **9 new files byte-exact** (candidate re-hash before copy, post-copy
  hash + mode verify; worktree 0664 normalized to manifest modes):
  `bins/spc-pool/src/registration.rs` `544decba...`,
  `bins/spc-pool/tests/ark_publication.rs` `75a8b1a8...`,
  `bins/spc-pool/tests/ark_registration.rs` `0156193d...`,
  `crates/spc-ext/tests/registration_v1.rs` `d8352e19...`,
  `crates/spc-store/migrations/0006_ark_registration_state.sql`
  `243325ab...`, `crates/spc-store/tests/ark_registration.rs`
  `0ab2a096...`, `docs/changelog/2026-11-08-ark-registration-ack.md`
  `670282b4...`, `docs/decisions/2026-11-08-ark-registration-ack.md`
  `630bb121...`,
  `docs/reviews/2026-11-07-ark-registration-implementation.md`
  `ac289d1b...` (the frozen candidate report, transferred unmodified).
- **12 byte-apply files** (primary == base verified pre-copy, so the
  candidate bytes apply as bytes; post-apply hashes match the manifest
  rows): `api_backend.rs`, `sv2_server.rs`, spc-accounting `lib.rs` +
  `tests.rs`, spc-ext `lib.rs` + `tests.rs`, spc-store `bootstrap.rs`,
  `models.rs`, `redis.rs`, `wire.rs`, `tests/postgres.rs`,
  `docs/WIRE-FORMATS.md`.
- **11 three-way merges** (`git merge-file --diff3` in
  `/tmp/opencode/AP2-3way/`, base = LP-final-runtime-src-v4, ours =
  primary, theirs = candidate). Nine merged clean. Two conflicts,
  both resolved with proofs:
  - `bins/spc-pool/Cargo.toml` -> `b46d9ad4...`: the candidate's
    dev-dep additions (`sqlx`, `uuid`) are byte-identical to the
    EXP-OP block primary already carries (ours is a strict superset
    with the OP comment + `chrono`); resolution keeps ours' block.
    The clean part applies the candidate's
    `spc-crypto = { workspace = true, features = ["ark"] }` line -
    verified as the ONLY ours->merged diff line pair. The `ark`
    feature, `bark-wallet` and `server_rpc` optional deps were already
    present in the base (AP-3 landed them); no pin moves.
  - `bins/spc-pool/src/front.rs` -> `6419c04d...` (pre-reconciliation
    view; see below): `invalidate_current_work` conflict resolved to
    keep BOTH the T4-owned `self.witness_plan = None;` and AP-2's
    `self.published_entries = None;`.
  - `Cargo.lock`: merged result is byte-identical to ours - a NO-OP.
    The candidate's 2-line lock delta (bark-wallet/server_rpc edges)
    was already landed in primary by AP-3; the history2 spc-lab-txd
    and OP spc-pool edges are preserved. No lock write was needed.
- Every merged file verified: all added lines ours->merged exist in
  the candidate; the removals are candidate rewrites of base regions
  ours did not own. main.rs keeps the OP/T4-era content and gains the
  registration service wiring (`with_registration_ark_id`); SPEC keeps
  all 50 primary changelog bullets while applying the candidate's
  SS11.3/11.4 body hunks (the -14 removed lines are the candidate's
  approved SS11.4 rewrite of base content).
- **payout.rs reconciliation** (`c1595e1c` merged ->
  `91515f4a91fc8f8a2d1f4f8c1c1a5cd14cff863e521fb881ca47be6f8d02e159`
  final): the merged file kept ours' three T4-era counterfactual
  tier-mirror test constructors (the candidate predates them; its
  payout.rs has a single production `ArkCapability` site). The
  candidate's spc-accounting gained
  `ArkCapability::registration_committed`, so clippy `--all-targets`
  and the ark-feature check failed E0063 at payout.rs:2630/2637/2644.
  Fix: each constructor gained a 3-line comment +
  `registration_committed: false,` (field order matches the
  candidate's production site; `false` preserves the tests'
  fully-unregistered counterfactual semantics). The diff vs the staged
  merged output is EXACTLY 3x (comment+field) additions. Same
  merge-owner adaptation class as the OP packet's EngineOptions
  reconciliations; validated by round-2 runs below.
- **SPEC frontmatter splice (merge-owner convention)**: the candidate
  provides the changelog fragment
  `docs/changelog/2026-11-08-ark-registration-ack.md` (byte-exact,
  archived) and no direct SPEC frontmatter edit. Per the corrected
  workflow (AGENTS.md working conventions, MERGE-QUEUE fragment
  procedure), the merge owner consumed the fragment at merge time as
  ONE doc-only edit after the transfer: the fragment is a **3-bullet
  unit** (original packet entry + AP-2 rework entry + R2.1
  clarification entry), so the count is 50 -> **53** (the prep note's
  "50 -> 51" single-bullet assumption was wrong and is corrected
  here). Each bullet is byte-exact modulo the 2-space frontmatter
  indent normalization, which follows the archived-fragment precedent
  (`2026-11-05-lab-lp1-integration.md` landed the same way). Reverse-
  splice proof reproduces the pre-splice state `2a83cb64...`; all
  prior-entry markers preserved (ACCESS, history2, hub, OP runtime,
  node-profile, T1a, T0, AP-3a, T4); SPEC body unchanged by the
  splice. Final `docs/SPEC.md`
  `bf222a83eef6b10865bca6e6e049abeb8ab241a23c6ed7ba48ac6b1e60127f72`.
- Status 214 -> 226 lines; index `44f21a92...` unchanged throughout.

## 3. R3.1 and AP-3 handoff boundary verification (fable SS5 / R3.1)

- **R3.1**: the journal lookup `SELECT outcome, signed_fields_digest,
  registration_rev, error_code FROM ark_registration_request WHERE
  sp_id = $1 AND request_id = $2 FOR UPDATE` is present inside
  `apply_registration` (transferred `postgres.rs`), after the
  `sp_identity ... FOR UPDATE` lock and the `ark_registration_state`
  read, BEFORE `is_new` - exactly the required closure position.
- **Domain separation (SS4.6, landed)**: `SPc/ArkIdentity` and
  `SPc/EpochMailboxes` are distinct `tagged()` domains in
  `crates/spc-ext/src/lib.rs` (lines 485/615).
- **Signed canonical request_id**: both `signing_payload`s include
  `w.u32(self.request_id)` as the final signed field (the replay
  amendment); AP-2's wire digest includes `request_id` while AP-3's
  client-side `local_digest` comparator excludes it - the two objects
  stay separate. The AP-3 `local_digest` code lives only in
  `bins/spc-agent/` files, which have ZERO overlap with this packet's
  32-row delta (boundary preserved by construction).
- **`D` required**: `registration.rs` uses
  `spc_crypto::ark_feature::{build_bark_address, pool_blinded_mailbox_id}`
  (line 436); `mailbox_dh` remains a required field (`D = b_ark *
  S_mb` supplied by AP-3) - the candidate correctly refuses to make it
  optional.
- **`superseded` semantics / no `current_request_id` hint**: unchanged
  from the candidate; the AP-3b follow-up is not claimed.

## 4. Vendor-hash consistency (probe: hash UNCHANGED)

Only `bins/spc-pool/Cargo.toml` changed among the vendor FOD's Cargo
inputs (the `spc-crypto` features line; `Cargo.lock` is a no-op), so
the vendored crate set is identical. Probe
(`/tmp/opencode/AP2-vendor-probe.sh`, `44c331dd...`, same procedure as
history2/OP: throwaway copy of the git-selected 658-file source, valid-
SRI same-length blank, `nix build path:...#checks.x86_64-linux.test`):
the FOD reported **got:
`sha256-PvxybBOJ1f4X+nkf6DQCdHM6Le9Dr67v3nhoTdrLYBM=`** - identical to
the flake's specified hash. **No flake edit was needed this packet**;
`flake.nix` stays `b4bf0503...` and the OP-time vendor hash binds the
AP-2 final source state (the probe ran against the post-transfer,
post-splice source). Log:
`/tmp/opencode/AP2-integration-logs/vendor-hash-probe.log`.

**Frozen source snapshot** `/tmp/opencode/AP2-nix-src` (658 files,
10828681 bytes / 10.3 MiB, same git-selection, frozen after the
payout.rs reconciliation so it binds exactly the validated source),
row manifest `/tmp/opencode/AP2-nix-src-manifest.json`
(`9fc3c7e14997de3cc22bf7e534d08398f2967377e99eccf854c50df8731a9987`).

## 5. Validation (actual runs, pinned devshell, flock + >=50 GiB policy)

Round 1 (`/tmp/opencode/AP2-validation.sh`, `48f5b114...`): fresh owned
throwaway storage started INSIDE the guard with fd 9 closed for daemon
children (PostgreSQL 17.11 + Redis, `/tmp/opencode/AP2-storage.0IoWlD`,
ports 57963/60395; the shared `/tmp/opencode/wt/pg` was never touched),
disk 67-70 GiB free throughout, existing primary `./target`, candidate
frozen commands with `--test-threads=1` for the PG/Redis suites:

- **spc-ext** (`cargo test -j2 -p spc-ext`): exit 0, **8 + 6 = 14
  passed, 0 failed** (lib units + registration_v1; the candidate
  report's "5 v1" undercounted - the byte-exact transferred file has 6
  tests, all passing).
- **spc-store** (`--test postgres --test ark_registration
  --test-threads=1`): exit 0, **2 + 20 passed, 0 failed** - matches the
  candidate's frozen gate exactly.
- **spc-pool ark_registration** (`--test-threads=1`): exit 0, **8
  passed, 0 failed**, including the R2.1 regression
  (`rejected_id_collision_is_terminal_mismatch_without_poison_or_mutation`).
- **spc-pool ark_publication** (`--test-threads=1`): exit 0, **3
  passed, 0 failed** (live 0x5350 -> payout -> front-event ->
  frozen-destination integration; real Noise/SV2 + PG/Redis, no node
  or Ark server claimed).
- **Production rpc spc-pool binary** (`cargo build -j2 -p spc-pool
  --features rpc`): exit 0, `target/debug/spc-pool` sha256
  `e2953368fe535b5cd927a8495e9615f676fd424593c64cff73f654d51e048407`
  (built from the pre-reconciliation source; the reconciliation is
  `#[cfg(test)]`-gated so the non-test lib is identical).
- **spc-lab merged-tree check** (`cargo check -j2 -p spc-lab
  --all-targets`): exit 0 (spc-lab dev-deps on spc-pool/spc-store
  still compile against the merged tree).
- **docs** (`python3 scripts/check-lab-docs.py`): exit 0, "OK: 6
  files, ASCII clean, links resolve, vectors pass".
- Round-1 failures (the two compile-only steps) were the payout.rs
  E0063 described in section 2; both were resolved by the
  reconciliation and re-run green in round 2. No test step failed.

Round 2 (`/tmp/opencode/AP2-validation-r2.sh`, `b967afd2...`, after
the reconciliation only):

- **spc-pool lib tests** (`cargo test -j2 -p spc-pool --lib`): exit 0,
  **253 passed, 0 failed** (the candidate's 228 on its own source plus
  ours' T4/AP-3-era units, including the three reconciled
  counterfactual constructors).
- **ark feature check** (`cargo check -j2 -p spc-pool --features ark
  --all-targets`): exit 0 - the bark-wallet + server_rpc feature path
  compiles against the integrated code (matching the integrated AP-3
  ark feature path; deps already pinned in Cargo.lock).
- **clippy** (`cargo clippy -j2 -p spc-ext -p spc-store -p spc-pool
  --all-targets -- -D warnings`): exit 0.
- **fmt** (`cargo fmt --all -- --check`): exit 0.

Storage stopped cleanly inside the guard (PID-verified, fd hygiene);
**zero leaked primary-target fixtures, zero stale storage processes**.
Logs: `/tmp/opencode/AP2-integration-logs/`.

## 6. Combined merged Nix gate (queue-closing verification)

The ONE combined merged Nix workspace gate on the final state ran after
this receipt's validation, against the frozen snapshot
`/tmp/opencode/AP2-nix-src` (658 files; the receipt markdown added after
the freeze is not a build input - the FOD filters to
Cargo.toml/Cargo.lock):
`nix build --max-jobs 1 --cores 2 path:/tmp/opencode/AP2-nix-src#checks.x86_64-linux.test`
(script `/tmp/opencode/AP2-combined-gate.sh`, `5ca6e592...`; ticket
guard + flock + 66 GiB free). **Exit 0**: the `cargo-deps-vendor`
fixed-output derivation built with the real hash
(`sha256-PvxybBOJ...`, confirming the probe result binds the final
state) and the `spc-workspace-tests` derivation built clean. This gate
covers the merged result of all five packets in the window (Hub,
history-v2, EXP-ACCESS, EXP-OP-runtime, AP-2). Log:
`/tmp/opencode/AP2-integration-logs/combined-nix-gate.log`.

## 7. Final state

- Index `44f21a926d4d065ac6937fb0f81b698c7b2d0627` (== HEAD tree;
  nothing staged, no commits made) - unchanged across the whole packet.
- Status 226 lines at receipt time.
- Roots final: `Cargo.toml` `8530ce08...` (unchanged), `Cargo.lock`
  `6ca42938...` (unchanged - no-op merge), `docs/SPEC.md`
  `bf222a83...` (53 entries), `flake.nix` `b4bf0503...` (vendor hash
  `sha256-PvxybBOJ...`, probe-confirmed for this state).
- Queue after this gate: **the window is complete** - all five packets
  (Hub, history-v2, EXP-ACCESS, EXP-OP-runtime, AP-2) are integrated,
  validated and gate-verified. J1/history-3 remains unapproved and
  unmerged.

## 8. Not claimed here

- No actual Ark server API change (vanilla Second.tech/Bark
  interoperability is untouched; registration is SPc-side), no funded
  Ark or mining acceptance, no real wallet/registration flow (both
  ends exercised with synthetic keys per the review), no AP-3b
  `current_request_id` wire addition, no J1/history-3 work, no commit
  (index invariant), no candidate-binary reuse as merged proof.
