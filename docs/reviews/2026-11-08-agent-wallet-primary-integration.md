# AP-3a agent wallet foundation: primary integration

Date: 2026-11-08
Owner: serial primary code integration owner (approved AP-3a fix-r2 only)
Status: complete; primary selected code frozen after the checks in section 5
Candidate: `/home/ethan/.local/share/opencode/worktree/ad8886/lab-agent-wallet`
(read-only throughout), fix-r2 manifest `/tmp/opencode/AP3a-fix-r2-manifest.json`
(file SHA-256 `60e2477a49c88dc5664600e48ddc6864641a1b9632a6e19d7ffd0e4c4c53e684`),
386 records, snapshot
`ae7e99e6d77b4be598bcf9a4e9d1466ebf46d7f31d49a398898c564dfabda218`, code
snapshot `cd09ed78b7b82c795a43b4798608228f300437e38fc50533638eb8b9b86d0530`,
base `b0ab4ca34fda07c61a6335d05f598eaacc69678c`.
Approval: Fable re-review, third (latest) section of
`docs/reviews/2026-11-08-agent-wallet-foundation-fable.md`, APPROVE for
fix-r2 (full snapshot `ae7e99e6`, code `cd09ed78`).
Candidate implementation report:
`docs/reviews/2026-11-07-agent-wallet-foundation.md` (transferred
byte-exact).

This document is the sole unapproved evidence record of the transfer. It is
not a behavior change.

## 1. Pre-edit verification (all before any write to primary)

- Candidate manifest file re-hashed:
  `60e2477a49c88dc5664600e48ddc6864641a1b9632a6e19d7ffd0e4c4c53e684` (matches
  the reviewer's record).
- All 386 candidate manifest records re-hashed in the source worktree
  (`python3 /tmp/opencode/AP3a-fix-r2-verify-manifest.py`: VERIFY OK, 386
  files, code `cd09ed78`, delta 8 rows vs the frozen fix-r1 manifest; the
  full aggregate `ae7e99e6` was additionally recomputed from the re-walked
  rows and matches).
- Baseline `/tmp/opencode/LP-final-runtime-manifest.json` (370 records, file
  SHA-256 `8bbe097de563600ce8bf0b8cb9693b082a9cff9bb51580d96afd89d2c0c3021f`),
  frozen source `/tmp/opencode/LP-final-runtime-src-v4` re-verified intact
  (`VERIFY OK: 370 files, full=9ffff6da code=30fedafd`).
- AP-3a delta vs the approved 370-file baseline computed from the two frozen
  manifests: 16 added + 11 modified, 0 removed (386 = 370 + 16). Every delta
  path was checked in primary before any edit:
  - the 10 modified paths other than `docs/SPEC.md` were byte-equal to the
    baseline records (modes included);
  - the 15 added paths other than the changelog fragment were absent;
  - `docs/SPEC.md` was at the post-T4 state
    `46d557875562717fbe44a6a200b82711831c0b13f2e7651cb933328b0e079242` -
    the one expected divergence (T4a's frontmatter splice plus sections
    2.4.2/3.3/4.8).
- Primary index hash before any edit: `git write-tree` =
  `44f21a926d4d065ac6937fb0f81b698c7b2d0627`, equal to `HEAD^{tree}` (nothing
  staged). The full pre-existing `git status --porcelain=v1` (140 lines: the
  LP final runtime, the T4a transfer and all other user work) was captured to
  `/tmp/opencode/AP3a-integration-gitstatus-before.txt`. Nothing was reset,
  cleaned, stashed, committed or staged at any point.

## 2. Transfer (patch tool, exact approved diffs only)

Twenty-six patches were generated from the frozen baseline source to the
fix-r2 candidate (10 modified files + 15 new files + the SPEC body-hunk
patch) and applied to primary with `patch --fuzz=0 --no-backup-if-mismatch
-p1`, each preceded by a fresh pre-state verification (section 1 checks
re-asserted per target) and a `--dry-run`. No fuzz, no rejects, no backup
files. The two SPEC hunks relocated by exactly the 54 lines T4a's body edits
added ahead of them (offset is context relocation, not fuzz).

Only the AP-3a delta vs the approved 370-file baseline was transferred; the
whole candidate was not copied.

Post-transfer, every transferred path is byte-equal (hash and mode) to the
approved fix-r2 candidate manifest:

| Path | Kind | Final SHA-256 |
|---|---|---|
| `Cargo.toml` | modified | `17fe3832141ea6e36f45ce0a85d7fe387053c0eb72b1066249cf4e3b61dc3569` |
| `Cargo.lock` | modified | `01cc9d89074057c6ee3354efb3405448cfd43cb344345631fb06b9a6ae4a2477` |
| `flake.nix` | modified | `f024030549098eb8fbe2000d6a493f61bf1c3b993866c0fca93a91f301076fe6` |
| `bins/spc-agent/Cargo.toml` | modified | `67445fe09c22a9f305de0083cb60f68ca3dd1ccd8d16829e65345e09b460c8be` |
| `bins/spc-agent/src/config.rs` | modified | `affb8dde2dde1228ff931de2e917302b1f6d2c66e3dab6bf5b092cda0721eaab` |
| `bins/spc-agent/src/lib.rs` | modified | `42000e87365fced4c0963469629473ed084a8c5e530fc67e78b93a578a4b6f2f` |
| `bins/spc-agent/src/main.rs` | modified | `7c4df5a70608808448c4a5eeab10b3807f2cf5873911bfee935a75ff2128cbc3` |
| `config/spc-agent.example.toml` | modified | `b719b3c24886af1dcf7bfb6fa9da90ff0cff83c48811f60bfa087581b1cee7d8` |
| `crates/spc-crypto/Cargo.toml` | modified | `e0e5da1d59736b83b2723027b933bfc4625df4b5747af0cd58001138778fdf1a` |
| `crates/spc-crypto/src/lib.rs` | modified | `d72351e3cccc4ac75bbbf2bc8f608fafcc96d6082e300bf1ad71744aa5182ee0` |
| `bins/spc-agent/src/wallet/bark_store.rs` (new) | added | `75de465a11520c2ac7341110a09ac05d3d6b490c6f48eb6b29e7e25c505fb2af` |
| `bins/spc-agent/src/wallet/keys.rs` (new) | added | `ff292369b1d4d92922ab9a49a79720454c7157ab343ca7e0018ff093efe230bd` |
| `bins/spc-agent/src/wallet/mod.rs` (new) | added | `528d09884e164b71631496c076f6ce082ec899ff5f39200dd481a907bb1d0006` |
| `bins/spc-agent/src/wallet/receive.rs` (new) | added | `53a4b8fd105199b37516ce138faf35e4fb70efd4f2b612600524ec088bb7713e` |
| `bins/spc-agent/src/wallet/registration.rs` (new) | added | `6946cfc4071c63e41a42ac3127c891d653cb1a1abfd0c4161de5098126c0efe3` |
| `bins/spc-agent/src/wallet/state.rs` (new) | added | `024a63f463b765b7ab619cc65980e8e148b876c24d661be107802b659c8794c6` |
| `bins/spc-agent/src/wallet/status.rs` (new) | added | `dba005b893741dd8a8dc718f314d392770569beb8e3fd034e159998b96f5d4a8` |
| `bins/spc-agent/tests/fixtures/ap3a-recovery-fixture.json` (new) | added | `df55a07493d84740d9d14762e583356c2c31f802e5390ebb38f228665d728121` |
| `bins/spc-agent/tests/wallet_common/mod.rs` (new) | added | `10d3bcb378a2a7003514ea193e5aad5a8d4d28c5e3214124cea603ce33019445` |
| `bins/spc-agent/tests/wallet_receive.rs` (new) | added | `88fb114988e8db064f2c920bd575235927024f90d4d6e3cbfc456cb8cdf632ff` |
| `bins/spc-agent/tests/wallet_recovery.rs` (new) | added | `8eed0439282173d24e2098c19f015f507250e2cac8533502812869585a4bd9fe` |
| `bins/spc-agent/tests/wallet_registration.rs` (new) | added | `9937c0648c974c38316f64294eddfcb47b36dd5848169b0b75a6bd8569e042bc` |
| `crates/spc-crypto/src/hd.rs` (new) | added | `144f4d11163c5094dd7b791882e85bb62c196512164eb694c92951ebffe8c47b` |
| `docs/decisions/2026-11-08-agent-wallet-foundation.md` (new) | added | `659075664331d3237366549b531ef6d9ce9bfb3fb9ddc11d372f9d90e49ce1a8` |
| `docs/reviews/2026-11-07-agent-wallet-foundation.md` (new) | added | `ba3ada35c781d7a84f610bf2afd2ef81c723ee06d3f7d7a81c44bd33f5292056` |

### 2.1 Shared-root files (Cargo.toml, Cargo.lock, flake.nix)

The three shared-root files were applied as exact approved hunks. Pre-edit,
primary was byte-equal to the approved 370-file baseline at all three paths
(T4a made no Cargo/flake change), so the hunks preserve every prior approved
change by construction. Verified specifics:

- `Cargo.lock`: 565 `[[package]]` entries on both sides with identical name
  sets; the entire diff is 8 dependency-edge lines (spc-agent gains `axum`,
  `bark-wallet`, `chrono`, `rusqlite`, `tempfile`, `zeroize`; spc-crypto
  gains `bip39` - all packages already pinned in the approved lock). **No
  pin moved and no package version changed.**
- `flake.nix`: exactly the two approved hunks - the `fetchCargoVendor` hash
  `sha256-TET0F9jXQELPadNaPKBiPXgFNGPnU9uRaJWjkSAX52w=` ->
  `sha256-yyjqKI9Okm7Jr4Qd/fRgvcjC986HI4LriGAxLp39YY8=` and the `test-ark`
  checkPhase gaining `cargo test -p spc-agent --features ark --offline`.
  The vendor hash is consistent with the merged `Cargo.lock` (the merged
  (lock, flake) pair is byte-identical to the candidate pair the reviewer's
  Nix run `q5x5r45n...drv` validated; the section 5 gate re-proves it on
  the merged tree by building green).

### 2.2 SPEC integration and the frontmatter exception

The candidate SPEC carries no frontmatter change (fix-r2 reverted it; only
the 11.5/11.10 body hunks remain). Those two body hunks were applied
byte-exact via the same patch mechanism. Per `docs/changelog/README.md` the
merge session splices the approved fragment
`docs/changelog/2026-11-08-agent-wallet-foundation.md` (candidate SHA-256
`3d139cfa7b700cf75db808ba9590f15cc85e306640c11ca8e005eab630a8f5dd`, a
single 2156-byte line) into the SPEC frontmatter and deletes it. The bullet
was inserted byte-exact at the head of the reverse-chronological changelog
list (ahead of the same-date T4a entry); `updated:` stays `2026-11-08`
(already set by the T4a splice). This is the explicit serial-merge
exception to byte identity. The fragment file was not created in primary:
the README's procedure consumes it at merge (splice and delete), so not
creating it produces the identical end state. No other changelog fragment
was touched, created or deleted.

Mechanical verification, both directions:

- Reverse-applying the two body-hunk patches to the final primary SPEC and
  removing exactly the spliced bullet reproduces the T4-state SPEC
  (`46d557875562717fbe44a6a200b82711831c0b13f2e7651cb933328b0e079242`)
  byte-for-byte - so the AP-3a SPEC edit is exactly (approved body hunks) +
  (the fragment bullet), nothing else.
- The residual diff of the final primary SPEC against the candidate SPEC is
  exactly the T4a delta (the `updated:` line, the T4a changelog bullet, and
  the T4a section 2.4.2/3.3/4.8 body hunks) plus the AP-3a bullet - four
  hunks, no other difference.

Final primary SPEC SHA-256:
`8f3117f778a45e641e3412f48dfa1a2c4ed93de0bdb798372f6e20f6fd2b9b54`.

## 3. Non-interference

- Index hash before == after all edits and tests:
  `44f21a926d4d065ac6937fb0f81b698c7b2d0627` (nothing staged by this
  transfer).
- `git status` delta vs the pre-edit snapshot is exactly 14 added lines:
  `M` entries newly listed for `bins/spc-agent/Cargo.toml`, `lib.rs`,
  `main.rs`, `crates/spc-crypto/Cargo.toml`, `crates/spc-crypto/src/lib.rs`
  (`Cargo.toml`, `Cargo.lock`, `flake.nix`, `config.rs`,
  `config/spc-agent.example.toml`, `docs/SPEC.md` were already modified by
  pre-existing work and stay modified), and `??` entries for the new wallet
  sources/tests, `hd.rs`, the decision record and the implementation
  report. Nothing was removed or reverted; git status is byte-identical
  after the section 5 checks (the test runs touched only `target/`).
- Candidate source worktree: read-only throughout; the frozen manifests,
  the frozen baseline source and all prior receipts (T4a included) are
  untouched inputs.
- No node, miner, UI or live service was started. All cargo runs used `-j2`
  and the existing primary `target/` directory. No shared cache cleanup, no
  pin bump, no `nix flake check` of unrelated namespaces.

## 4. Targeted checks (merged tree, plain cargo)

Commands, exit codes and log SHA-256 (logs under
`/tmp/opencode/AP3a-integration-logs/`):

| Command | Exit | Result | Log SHA-256 |
|---|---|---|---|
| `cargo test -j2 -p spc-agent --features ark` | 0 | 131 passed, 0 failed, 1 ignored across the crate's 10 test binaries (lib 108, wallet_registration 10, wallet_receive 3, wallet_recovery 4 + 1 ignored, example_config 1, e2e 2+2+1, main/doc-tests 0) | `adcb7bb5f1677271cad7621fed369fd69d21f81f86b49b8d37119fd83c266133` |
| `cargo test -j2 -p spc-crypto --features ark` | 0 | 102 passed, 0 failed (lib 57, bip352_address 1, carriers 17, vectors 26, version 1, doc-tests 0) | `efb1141ad7dd88c86ee30a41d743b1ff22ba8fa1e5a9139edec9f0f702da2dbb` |
| `cargo fmt --all --check` | 0 | clean | `19eaf43821a7660ec323a87c8457bf74823beb296c39f5e01aa8a683aa50f061` |
| `cargo clippy -j2 -p spc-agent --all-targets --features ark -- -D warnings` | 0 | clean (pre-existing sqlx-postgres future-incompat toolchain note only) | `495cddeb76b4926cd9a00f2f322d93de9ea37d00d53131601028ae2b76a9d84d` |
| `cargo clippy -j2 -p spc-crypto --all-targets --features ark -- -D warnings` | 0 | clean | `d89d487f06838751a4de47560f148392116f60a8769e7581fb8b3a2b5ccaf68d` |

These match the reviewer's fix-r2 receipts exactly (agent 131/0/1 with lib
108 and wallet_registration 10; spc-crypto ark 102; fmt and both clippy
runs clean). The featureless workspace regression (r1: 641/81) and the
featureless flake checks were not re-run: no featureless code path changed
relative to the candidate the reviewer already ran them on, and the merged
tree differs from the candidate only by the T4a spc-pool edits (whose own
targeted suites were run green at T4a integration and whose ark/rpc suite
runs in the section 5 gate below).

## 5. Nix integration gate (explicit source)

Per the reviewer's corrected provenance procedure: the check is built from a
clean **source-only** snapshot of the merged primary tree, never from the
raw worktree (whose `path:` store copy would ingest the multi-gigabyte
`target/`).

- Snapshot `/tmp/opencode/AP3a-integration-nix-src`: rsync of the merged
  primary excluding `.git`, any `target/`, root `result`/`result-*`, and
  the `spc_ark_e2e/test` runtime datadir - 436 files, 8.5 MB. Verified
  byte-faithful: the snapshot and the merged primary produce identical
  `{path, mode, sha256}` row sets (436 rows) over the same exclusion set,
  and every AP-3a transferred path inside it is byte-equal to the frozen
  fix-r2 candidate manifest (log:
  `/tmp/opencode/AP3a-integration-snapshot-verify.log`).
- Serialized with all other heavy work (the section 4 cargo checks had
  already completed); `--max-jobs 1 --cores 2`; ~29 GB free at start
  (sufficient; the 30 GB `path:`-on-worktree hazard was avoided by
  construction). No shared cache was deleted.
- Build: `nix build --max-jobs 1 --cores 2 --no-link
  path:/tmp/opencode/AP3a-integration-nix-src#checks.x86_64-linux.test-ark`
  - exit 0. Derivation:
  `/nix/store/gq00f0dls72hl33rcs384q41mf9f9n4f-spc-ark-feature-tests-0.1.0.drv`;
  output
  `/nix/store/a21p436s9a7rm3n9c8qkvs394wq560g7-spc-ark-feature-tests-0.1.0`;
  source store binding
  `/nix/store/51kchp0ghgkjj1b45d89y8svs2sg84yi-source`
  (narHash `sha256:0shzl0fss56jazlv3srfhi3z2vyfx78iwzlkkpj6klhd1fp5g94z`).
- The `test-ark` checkPhase runs `cargo test -p spc-crypto --features ark
  --offline`, `cargo test -p spc-pool --features rpc,ark --offline` (this
  is the first Nix-level run of the T4a-modified spc-pool sources) and
  `cargo test -p spc-agent --features ark --offline`, all against the
  flake's vendored deps - which also proves the vendor hash
  `sha256-yyjqKI9Okm7Jr4Qd/fRgvcjC986HI4LriGAxLp39YY8=` is consistent with
  the merged `Cargo.lock` (a mismatch fails the fetch, not the tests).
- Full test output captured via `nix log` of the derivation (not build
  announcements): see `/tmp/opencode/AP3a-integration-logs/nix-test-ark.log`.
  Actual result (checkPhase 2m39s): spc-crypto (ark) 102 passed across 6
  binaries; spc-pool (rpc,ark) 274 passed across 11 binaries (lib 251
  including the T4a witness suites, main 2, e2e_sv1 1, e2e_sv2 2,
  e2e_type32 1, example_config 1, main_sv2_pipeline 1, sv2_conformance 4,
  sv2_outcome_preemption 1, witness_conformance 10); spc-agent (ark) 131
  passed / 0 failed / 1 ignored across its 10 binaries. Total: **507
  passed, 0 failed, 1 ignored**. The spc-pool count exceeds the
  candidate-only r1 receipt (240) exactly because the merged tree carries
  the T4a witness work - the gate's purpose.
- `nix flake check` in full was not run: it re-executes unrelated
  namespace/native checks whose closures are not warmed for the merged
  tree; the explicit-source `test-ark` build above is the gate the
  reviewer's procedure prescribes, and it is the check that covers every
  feature-gated line this packet touches.

## 6. Fable's three non-blocking notes (recorded, not fixed)

Per the exact-transfer rule no optional code fix was applied; the
reviewer's three non-blocking notes are recorded as the AP-3b followup
list:

1. A `bark/` directory that is intact but has had all of its contents
   removed is not cross-detected (the fix-r2 `state.sqlite`
   `bark_initialized` cross-binding refuses a wholly *missing* `bark/`
   only).
2. SPEC 11.5's "a missing database is the only genuinely-fresh case"
   wording is weaker than the code's marker-based fresh/damaged decision.
3. A fatal run error closes the `/status` listener (status surface
   teardown on fatal stop).

## 7. Scope and limits

- Fable's approval is scoped to the agent wallet **foundation**: the
  offline stock bark persister, the provisioned-key loading/validation, the
  persistent keystore/VTXO-ledger/registration-counter/journal/scan-cursor
  store (schema v2, effective binding), the registration state machine
  against the transport seam, and the loopback status surface. This
  transfer does **not** establish: the full stock bark `Wallet::open` (live
  chain source), live VTXO recovery or real ancestry parsing, a live
  registration transport (AP-2 owns the `0x5350` codec; without it the
  role stays `dormant`), chain scan, delegated refresh/offboard, or the
  `static`/`epoch_registered` mailbox modes - all AP-3b.
- AP2 (lab-ark-registration) is not integrated; its spc-ext codec and
  front-side work remain isolated. The `crates/spc-crypto` hunks here were
  applied on the baseline state (the `hd` module wiring); merging AP2's
  spc-crypto work against this is the parent merge owner's call.
- No commit was made; the transferred work sits in the working tree
  alongside the pre-existing primary user work, exactly as the candidate
  sat in its worktree.

## 8. Freeze

The primary selected code (section 2 table plus `docs/SPEC.md`) is frozen
as of the completion of the section 4 and 5 checks. Receipts:
`/tmp/opencode/AP3a-primary-integration-manifest.json` and the
`/tmp/opencode/AP3a-integration-*` files (patches, logs, index and
git-status snapshots, verification scripts, Nix gate log).
