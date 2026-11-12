# T1a traffic wallet core (`spc-lab-txd`) - primary integration (2026-11-08)

Owner: serial primary Rust/Cargo integration writer (sole writer for this
packet; other agents isolated in OP/access/AP2 worktrees, node-prep
docs-only).

Approval: `docs/reviews/2026-11-08-traffic-wallet-fable.md` Part 4
(r3 re-check) - **APPROVE bounded to code snapshot `a8d28cea...`**:
the `spc-lab-txd` crate (15 files), its decision record and changelog
fragment, the +1 workspace member line, and the one 18-line
`spc-lab-txd` Cargo.lock block. Explicitly NOT approved (and NOT done
here): the T0 `history_version` 2 / allocation-digest header amendment
(separate parent-started packet gating the first T1 journal write), T1b
driver/runtime, T2-L bootstrap, T3 live proof, node profile,
mempool/RPC, Ark readiness.

External manifest: `/tmp/opencode/T1a-primary-integration-manifest.json`
(sha256 `6b04b62cf88c9257e7c9656eee241f08d71f99dd5d3a64fc46281bdcd48aeb94`).

## 1. Pre-write verification (read-only, before any transfer)

- Candidate manifest `/tmp/opencode/T1a-fix-r3-manifest-full.json`
  reproduced: full snapshot
  `65a16584fb89429ef2f59e88285fbfdd587a5892bd51c3f3489e2c7ad8eb3748`,
  code snapshot (report excluded)
  `a8d28ceac3a9bd63fc72886ca5040544431c1235b31a98e902290ee4ff834628`;
  all 388 records re-hashed and mode-checked against the candidate
  worktree (0 mismatches).
- Primary pre-state: all 18 new paths absent; `Cargo.toml` at
  `17fe3832...`, `Cargo.lock` at `01cc9d89...` (baseline + AP-3a edges),
  `docs/SPEC.md` at `3f669947...` (four-packet merged state); index
  `44f21a92...`; status 178 lines.
- Shared-root deltas confirmed as exactly one member line (Cargo.toml)
  and one 18-line lock block whose 12 dep edges all reference packages
  already in the lock.

## 2. Transfer (exact approved delta only)

- **18 new files copied byte-exact** (source re-hash + post-copy
  hash+mode verify, all 100644): 15 `crates/spc-lab-txd/**` files, the
  changelog fragment, the decision record, the implementation report.
- **`Cargo.toml`** - one member line `"crates/spc-lab-txd",` after
  `"crates/spc-lab",` (edit tool, not whole-file copy). Pre
  `17fe3832...` -> post
  `179312b2c0a0a0c8b7e778186b9d9aba184d303a81294c90a6c41582729cdfda`.
  Exception vs the candidate row: the merged file keeps the AP-3a
  `bip39 = "2.2"` workspace edge (the candidate was built on the
  pre-AP-3a baseline); merged = baseline + member line + bip39 edge.
- **`Cargo.lock`** - the 18-line `spc-lab-txd` package block inserted
  before the `spc-pool` block (edit tool). Pre `01cc9d89...` -> post
  `b447fb06947dd8cfea9cd829c6b66b640e3c1b6bd03cd29ff072e92dd5327a80`.
  Proof: reverse-removal of the exact block reconstructs the pre-state
  byte-exactly; merged = pre + exactly 18 lines. Exception vs the
  candidate row: the merged lock keeps all AP-3a edges (diff vs the
  candidate lock = exactly the 7 AP-3a edge lines).
- **`docs/SPEC.md`** - the fragment's single 3151-byte bullet spliced at
  the head of the changelog list. Pre `3f669947...` -> post
  `1769cad120af4071d6aba5a938aba7cd73decfa26905bfab7b364a8db7a5f98d`.
  Proof: reverse-splice reproduces `3f669947...` byte-exactly
  (verified independently post-write); 43 -> 44 entries with the
  T4/AP-3a/T0 bullets retained (marker counts unchanged); `updated:`
  stays 2026-11-08. The candidate SPEC was never copied (it is
  baseline, not delta).
- The pending node-profile fragment
  `docs/changelog/2026-11-08-lab-bark-v31-profile.md` is NOT assigned to
  this packet and was not touched or spliced.

The new code is library-only: no CLI actor, no wallet role callback, no
runtime wiring; T0 is still v1 and `prepare(None)` rejects all traffic,
so no first runtime journal write is enabled by this packet.

## 3. Targeted cargo checks (flock policy observed)

First attempt 09:37 was **blocked once** (heavy-build lock held by
another session; reported, no polling); source work done meanwhile
(Nix snapshot frozen and verified). Second attempt 09:40 acquired the
lock with 89 GiB free (>= 50 GiB policy threshold checked inside):

- `cargo test -j2 -p spc-lab-txd` - exit 0: **68 passed / 0 failed /
  2 ignored** on the parent harnesses (lib 28, fable_regressions 26 + 2
  ignored, journal_durability 6, lifecycle_reorg 5, signing_independent
  3, doc-tests 0), exactly the Fable r3 expectation; the child-process
  harness spawned by fable_regressions additionally reports its own
  `1 passed` (the `r1_real_write_fault_child` run under its parent) -
  the same tests, not an extra.
- `cargo fmt -p spc-lab-txd -- --check` - exit 0, clean.
- `cargo clippy -j2 -p spc-lab-txd --all-targets -- -D warnings` -
  exit 0, zero warnings.
- Pre-existing `~/.cargo` registry-cache permission warnings (another
  user's files) are environment noise; non-fatal.

Logs: `/tmp/opencode/T1a-integration-logs/` (`checks.log`).

## 4. Nix workspace-test gate (one explicit clean-source check)

Purpose: validate the new workspace crate + root lock integration
without repeating the other five unchanged checks from the four-packet
closeout gate (that gate is preserved scoped and not restamped).

Snapshot: `/tmp/opencode/T1a-nix-src`, 630 files, 12 MB, manifest
`/tmp/opencode/T1a-nix-src-manifest.json` (aggregate sha256
`6bb3c97c057c2d026be6513ae3cd8dc2962dc3e35566afcc1e3a881ca836ee56`).
Exclusions: `.git`, any `target/`, `__pycache__`, root `result*`,
`spc_ark_e2e/test`, symlinks. Verified: 630/630 rows re-hashed clean
inside the frozen snapshot; the 18 T1a rows and the four critical
shared roots (`Cargo.toml` `179312b2`, `Cargo.lock` `b447fb06`,
`docs/SPEC.md` `1769cad1`, `flake.nix` `7233ba5c`) match the primary
byte-exactly at gate time. Freeze policy: frozen at ~09:41; the only
post-freeze sync is the sanctioned flake.nix vendor-hash update below;
concurrent post-freeze primary changes by other agents (their
bark-v31-profile validation report; the live
`docs/REGTEST-LAB-STATUS.md`) are deliberately not synced - outside the
T1a delta and outside the cargo build graph. `flake.lock` byte-identical
(`2152143d...`) both sides; no raw worktree ingestion; no GC.

### 4.1 Attempt 1 - stale vendor hash (actual repro)

`nix build ... path:/tmp/opencode/T1a-nix-src#checks.x86_64-linux.test`
failed (exit 1, derivation
`93wpx0hx...-spc-workspace-tests-0.1.0.drv`): the cargoSetupHook
consistency check found the vendored Cargo.lock lacks the `spc-lab-txd`
block. Root cause: `fetchCargoVendor` is a fixed-output derivation -
its output store path is addressed by the DECLARED hash, so Nix silently
reused the pre-T1a vendor output (old hash `yyjqKI9Okm...`) without
re-running `cargo vendor`. This is the actual repro of the anticipated
"root lock / vendor FOD might need merged flake hash update": the old
hash is genuinely stale for the merged lock, so the sanctioned update
was required.

### 4.2 Probe and sanctioned flake.nix update

Empty-hash probe on a throwaway copy (`/tmp/opencode/T1a-nix-probe`,
`cargoDeps.hash = ""`; primary untouched by the probe; run under the
flock, 88 GiB free) reported the new vendor output hash:
`sha256-xO1PIE3cx5rOC+NqhNMDIkVBueG9S1gLWe+4MLZSiSs=`.

Applied to the primary `flake.nix` as **exactly one same-length
hash-string replacement** (line 45, `yyjqKI9Okm...` ->
`xO1PIE3c...`); reversing the single replacement reconstructs the
pre-edit flake byte-exactly (`f0240305...` -> `7233ba5c...`). No
version bump; `flake.lock` untouched. Sanctioned by the packet
instruction ("only update hash if new lock requires, no version
bump"). The edited flake.nix was synced into the frozen snapshot.

### 4.3 Final gate - PASS

`nix build --max-jobs 1 --cores 2
path:/tmp/opencode/T1a-nix-src#checks.x86_64-linux.test` - **exit 0**
(09:49-09:56 EST, lock held, 88 GiB free before / 89 GiB after).

- Derivation `44ra4lby...-spc-workspace-tests-0.1.0.drv`, output
  `k66j2n71...-spc-workspace-tests-0.1.0`, log
  `/tmp/opencode/T1a-nix-gate-logs/check-test.log` (2080 lines).
- `cargo test --workspace --offline`: **90 test-result lines, 793
  passed, 0 failed, 18 ignored** - exactly the four-packet baseline
  (83 / 724 / 16) plus spc-lab-txd (69 passed incl. the child-harness
  line, 2 ignored across 7 binaries).
- Vendor FOD proven, not assumed: the staging fixed-output derivation
  `bsrcxs205...-cargo-deps-vendor-staging.drv` output
  `83zrncjr...-cargo-deps-vendor-staging` hashes to
  `sha256:0aw9aav3...` (nix-base32) = the flake-declared
  `sha256-xO1PIE3c...` (SRI), and its embedded Cargo.lock is
  byte-identical to the merged lock `b447fb06...` **containing the
  spc-lab-txd block** - `cargo vendor` genuinely ran on the new source.
  (The wrapper derivation `71pgzl20...-cargo-deps-vendor.drv` has its
  own narHash `0r5syc2x...`; that is not the flake hash.)
- Source binding: `/nix/store/x3lg6sxs4q1gns595y6p8j186ca3b61b-source`,
  narHash `sha256:0xcz1fvslwpv9q5pzs06j6ig2ncrcgmz773sr8c14qbkcinl3d06`,
  630 files; `docs/SPEC.md` inside verified `1769cad1...`, `Cargo.lock`
  verified `b447fb06...`.
- Gate log: `/tmp/opencode/T1a-nix-gate-logs/gate.log`.

## 5. State

- Index `44f21a926d4d065ac6937fb0f81b698c7b2d0627` before == after
  (== HEAD tree; nothing staged; no commits).
- Status delta is exactly the 4 T1a untracked paths; the one other new
  status line (`docs/reviews/2026-11-08-bark-v31-profile-primary-validation.md`)
  is another agent's concurrent work in the shared primary, not a T1a
  write.
- The primary tree carries other agents' node-prep work (docs and
  `testing/rbitcoin-bark-v31/` evidence) that predates this packet's
  before-state; none of it touches the workspace build graph, and the
  four critical roots above were verified unchanged by it at gate time.

## 6. Remaining gates (NOT enabled by this packet)

- T0 `history_version` 2 / allocation-digest header amendment -
  separate parent-started packet; until it lands, T0 is v1,
  `prepare(None)` rejects all traffic, and **no first runtime journal
  write is enabled**.
- T1b driver/runtime integration; T2-L bootstrap; T3 live proof (live
  gates); node profile / dev schema / runtime config unchanged here.
