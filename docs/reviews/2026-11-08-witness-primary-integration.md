# T4a witness primitives: primary integration

Date: 2026-11-08
Owner: serial primary code integration owner (approved T4a only)
Status: complete; primary selected code frozen after the checks in section 4
Candidate: `/home/ethan/.local/share/opencode/worktree/ad8886/lab-witness-primitives`
(read-only throughout), rework manifest `/tmp/opencode/T4a-rework-manifest.json`
(file SHA-256 `579d248d8c82441057870f2d255c5b10ed38bb5cd9613efd007ff91c13691fdf`),
375 records, snapshot
`b9961746533632eaaa64eb30a508640cc95d9e234521cd81fdebc1a3795439f9`, code
snapshot `c3f9b53ac6ac8f7e4e496b72b5d04e8f73427931e0037c7ee48c0bc9f2f86688`,
base `b0ab4ca34fda07c61a6335d05f598eaacc69678c`.
Approval: Fable re-review appended to
`docs/reviews/2026-11-08-witness-primitives-fable.md`, APPROVE bounded to
snapshot `b9961746` (primitives plus minimal front/main/payout wiring).
Candidate implementation report:
`docs/reviews/2026-11-08-witness-primitives-implementation.md` (transferred
byte-exact).

This document is the sole unapproved evidence record of the transfer. It is
not a behavior change.

## 1. Pre-edit verification (all before any write to primary)

- Candidate manifest file re-hashed:
  `579d248d8c82441057870f2d255c5b10ed38bb5cd9613efd007ff91c13691fdf` (matches
  the reviewer's record).
- All 375 candidate manifest records re-hashed in the source worktree:
  0 missing, 0 hash mismatches, 0 mode mismatches.
- Aggregate recipe reproduced exactly (SHA-256 over compact canonical JSON
  `{"base", "files"}` with records sorted by path; the code snapshot excludes
  only the implementation report's path): snapshot `b9961746...` and code
  snapshot `c3f9b53a...` both reproduce.
- Baseline `/tmp/opencode/LP-final-runtime-manifest.json` (370 records, file
  SHA-256 `8bbe097de563600ce8bf0b8cb9693b082a9cff9bb51580d96afd89d2c0c3021f`,
  code snapshot `30fedafdbba57b79fe4c13e2de0658aa3f05f527b778a242cbfab9c4ba253d76`,
  frozen source `/tmp/opencode/LP-final-runtime-src-v4`):
  - the 9 candidate-modified primary paths were byte-equal to the baseline
    records (modes included);
  - the 5 candidate-added paths were absent in primary;
  - the entire `bins/spc-pool/` tree (34 baseline records) was byte-equal to
    the baseline with no extra files, so the post-transfer package state is
    byte-identical to the approved candidate package.
- Primary index hash before any edit: `git write-tree` =
  `44f21a926d4d065ac6937fb0f81b698c7b2d0627`, equal to `HEAD^{tree}` (nothing
  staged). The full pre-existing `git status --porcelain=v1` was captured to
  `/tmp/opencode/T4a-primary-integration-gitstatus-before.txt` (the LP final
  runtime and all other user work; nothing was reset, cleaned, stashed,
  committed or staged at any point).

## 2. Transfer (patch tool, exact approved diffs only)

Thirteen patches were generated from the frozen baseline source to the
candidate (9 modified files + 4 new files; the 14th delta path, the changelog
fragment, is handled in section 2.1) and applied to primary with
`patch --fuzz=0 --no-backup-if-mismatch -p1`. No fuzz, no offsets, no
rejects, no backup files. No Cargo.toml, Cargo.lock, flake or pin change is
part of the delta; primary's pre-existing modifications to those files were
not touched.

Post-transfer, every transferred code path is byte-equal (hash and mode) to
the approved candidate manifest:

| Path | Final SHA-256 |
|---|---|
| `bins/spc-pool/src/block.rs` | `cd50c58ff83b3f4a55e79fc45382ceead8573b40183cf76c63d7c9897eee9272` |
| `bins/spc-pool/src/coinbase.rs` | `81f33161691238dc140394409944c63e68156d52a4460ccaa0cc44332df5091f` |
| `bins/spc-pool/src/front.rs` | `604ef5e8ac2f408aba106db9b54d5af4d9701cd8796c84be2913a88a0b9e0954` |
| `bins/spc-pool/src/lib.rs` | `8ecf44abc3c1f0bfd959a8ae63c93af593002b39b82c229228bea2b571b45505` |
| `bins/spc-pool/src/main.rs` | `1b27b9ac6333f1e1e57c01bda48fc7d5c47d5be9d3e6282fceb45d33ed037639` |
| `bins/spc-pool/src/payout.rs` | `aa6bd28e90660e0c54db96d7d806c533e8904eedb26a8cac28a77a524bad7e6e` |
| `bins/spc-pool/src/template.rs` | `b6207efca535ab5a2ff9a0927aa6b2df6ba1c551c57f60cf465e11c28fa21848` |
| `bins/spc-pool/src/template_rpc.rs` | `7ae26496c0dbd03c8cdd7eb6d158d948b1e53e8511c1c69350cf94c214255958` |
| `bins/spc-pool/src/witness.rs` (new) | `b8b6270d402a7a7e791f7188f223d0b9200cd4d2ec944609391480accb019813` |
| `bins/spc-pool/tests/witness_conformance.rs` (new) | `c7e5f49ca00e045a77529818610220720b2aa94e3af53d5d0a47ad700b60b173` |
| `docs/decisions/2026-11-08-pool-witness-commitment.md` (new) | `d45ca03e44ab38cb593cb3adc3a6a1e2c4082b213cdba55c32164b6c3586f8f8` |
| `docs/reviews/2026-11-08-witness-primitives-implementation.md` (new) | `eb63ce59cd67c96ff8e23dde7af6f9ed6fc364b283c32d61392305a5cb14c807` |

### 2.1 SPEC integration and the frontmatter exception

The approved SPEC body hunks (sections 2.4.2, 3.3 "Witness commitments
(2026-11-08)", 4.8 "Witness overhead (2026-11-08)") were applied via the same
patch mechanism and are byte-identical to the candidate. The candidate SPEC
itself carries no frontmatter change; per `docs/changelog/README.md` the
merge session splices the approved fragment
`docs/changelog/2026-11-08-pool-witness-commitment.md` (candidate SHA-256
`d4ec9eec68dfb3099fa1c44c4a5f1e8605adb7c4f3f33f696560c60bf505a47a`) into the
SPEC frontmatter and deletes it. This is the explicit serial-merge exception
to byte identity. The exact frontmatter diff applied to the candidate SPEC
text:

```diff
-updated: 2026-11-06
+updated: 2026-11-08
 changelog:
+  - "2026-11-08: pool-template jobs support segwit transaction sets, fail closed. [...] See docs/decisions/2026-11-08-pool-witness-commitment.md."
```

(the inserted bullet is the fragment's single line, byte-exact, inserted at
the head of the reverse-chronological list; full text in the fragment and in
`docs/SPEC.md` frontmatter). Resulting primary SPEC SHA-256:
`46d557875562717fbe44a6a200b82711831c0b13f2e7651cb933328b0e079242`.
Verified mechanically: primary SPEC minus exactly this splice equals the
candidate SPEC byte-for-byte, so the body and the remainder of the frontmatter
are unchanged from the approved candidate.

The fragment file was not created in primary: the README's procedure consumes
it at merge (splice and delete), so not creating it produces the identical
end state. No other changelog fragment was touched, created or deleted.

## 3. Non-interference

- Index hash before == after all edits and tests:
  `44f21a926d4d065ac6937fb0f81b698c7b2d0627` (nothing staged by this
  transfer).
- `git status` delta vs the pre-edit snapshot is exactly the expected set:
  `M` entries added for `coinbase.rs`, `template.rs`, `template_rpc.rs`
  (`block.rs`, `front.rs`, `lib.rs`, `main.rs`, `payout.rs`, `docs/SPEC.md`
  were already modified by pre-existing work and stay modified), and `??`
  entries for the four new files above. Nothing was removed or reverted.
- Candidate source worktree: all 375 manifest records re-verified unchanged
  after the transfer (read-only; no source-worktree write).
- No node, miner, live T4b service, Nix build or full-workspace build was
  started. All cargo runs used `-j2` and the existing primary `target/`
  directory. No shared cache cleanup or path copy was performed.

## 4. Targeted checks (merged tree)

Commands, exit codes and log SHA-256 (logs under
`/tmp/opencode/T4a-primary-integration-logs/`):

| Command | Exit | Result | Log SHA-256 |
|---|---|---|---|
| `cargo fmt -p spc-pool -- --check` | 0 | clean | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` (empty log) |
| `cargo test -j2 -p spc-pool --test witness_conformance` | 0 | 10 passed, 0 failed | `5cb5f745344b69b54fe843579463fb691c0a5e17886946e4112ce2648573f0f0` |
| `cargo test -j2 -p spc-pool --bin spc-pool` | 0 | 2 passed, 0 failed, incl. `witness_publication::commitment_only_witness_change_republishes_both_ingresses` (B1) | `700a9f24286d2c9ea0bde1d29718edce29b14f919c477cbdddf2c9e8384ac84c` |
| `cargo test -j2 -p spc-pool --lib` | 0 | 247 passed, 0 failed | `4b54b5709e64be347b1c12f3e236764a765d004d21b6b311eda243d19d195ffd` |
| `cargo check -j2 -p spc-pool --features rpc` | 0 | clean | `4693e861425bb98f69bf53d123266d8b4276fe072625449d32e864ea2de17d16` |
| `cargo clippy -j2 -p spc-pool --all-targets` | 0 | no `spc-pool` warnings; only the pre-existing environment/sqlx-postgres lines | `9f40738fcb9578528b267accf804224f90a9ef61e01bd9d41bfbbc7b15ca8dbc` |

The 11 pre-existing integration tests were not re-run (unchanged files; the
candidate's 270-total result is consistent with 247 + 2 + 21, per the
reviewer's re-review). The merged `bins/spc-pool` package is byte-identical
to the approved candidate package, on which the reviewer ran the same suites.

## 5. Scope and limits

- Fable's approval is scoped to the witness primitives and their minimal
  front/main/payout wiring on snapshot `b9961746`. This transfer does not
  establish T4b live canonical-block acceptance, any Ark readiness, or
  adoption of the v0.9 node line.
- `template_rpc.rs` (feature `rpc`) remains compile-checked only; the live
  GBT path and the R7 cross-checks are T4b handoff items.
- AP2 (lab-ark-registration) is not integrated; the `front.rs`, `payout.rs`
  and `main.rs` hunks here were applied on the baseline state. Sequencing AP2
  against this transfer is the parent merge owner's call.
- No commit was made; the transferred work sits in the working tree alongside
  the pre-existing primary user work, exactly as the candidate sat in its
  worktree.

## 6. Freeze

The primary selected code (section 2 table plus `docs/SPEC.md`) is frozen as
of the completion of the section 4 checks. Receipts:
`/tmp/opencode/T4a-primary-integration-manifest.json` and the
`/tmp/opencode/T4a-primary-integration-*` files (patches, logs, index and
git-status snapshots, verification scripts).
