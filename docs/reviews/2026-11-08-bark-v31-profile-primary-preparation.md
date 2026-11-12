# AP-1b lab bark-v31 profile - primary source/doc preparation (2026-11-08)

Date: 2026-11-08
Author: TFF lab-integration subagent (source/doc preparation only)
Status: **PREPARATION COMPLETE; build/test gate PENDING.** This is not an
"integration complete" claim. No compile, test, node launch or probe run
was performed: the primary six-check Nix gate was still running, and this
step was deliberately bounded to source/doc preparation to avoid a build
storm. The parent dispatches verification after the Nix completion
notification.

## 1. Ownership boundary respected

Owned and written in this step, nothing else:

- `/tmp/opencode/AP1b-lab-integration` - new SOURCE-ONLY L2-derived tree
  (checked absent before creation; never overwrote anything).
- Primary additive `testing/rbitcoin-bark-v31/**` (80 files) plus the two
  packet-listed implementation reports
  `docs/reviews/2026-11-08-bark-v31-profile-implementation{,-r1}.md`
  (both verified absent before transfer; all 82 transferred with the
  per-path absent/exact-already guard).
- New docs: `docs/decisions/2026-11-08-lab-bark-v31-profile.md`,
  `docs/changelog/2026-11-08-lab-bark-v31-profile.md`, and this file.

No existing primary Rust/config/Cargo/flake/`UPSTREAM_DEPS.md`, no
`SPEC.md` frontmatter, no status/merge-queue doc and no other doc was
touched (git shows only untracked additions in scope; zero tracked-file
modifications by this step). The primary Fable review
`docs/reviews/2026-11-08-bark-v31-profile-fable.md` was preserved
untouched (it is not among the 82 packet records and was never written).
No commits, no subagents, no cache cleanup, no pin changes. The 27
`*.log` records are gitignored (`.gitignore:6`) and were transferred as
local evidence only - nothing was `git add`ed.

Not touched: the frozen `lab-node-compat-profile` packet worktree (read
only), the `/tmp/opencode/AP1b-bark-profile` candidate, the approved L2
tree and its `eb257af7...` binary, and the frozen lab-node-compat-profile
packet manifests.

## 2. Baseline re-verification (before any preparation)

- Approved L2 tree `/tmp/opencode/rbitcoin-lab-retarget-profile`: HEAD
  `e8cf9c9d61f7c63ae59cc11ab1179cace50ca142`; all 18 records of
  `/tmp/opencode/lab-l2-source-review-manifest.json` match; aggregate
  `d2f37b9113e8855245866289ed393043aed6e6153e8101375072cbb23624d499`
  reproduced; binary `target/debug/rbitcoin-node` =
  `eb257af7c10cdf0ded584adfadf2ae8124466304b13d26c17f855e76f9320f7c`,
  untouched (never written by this step).
- Candidate `/tmp/opencode/AP1b-bark-profile`: all 25 records of
  `/tmp/opencode/AP1b-r1-candidate-manifest.json` match; aggregate
  `f8a5111119d9e0df79cc8987f23d4b380c6ecad2df094b4c8196b2e2b04ba6d3`
  reproduced; candidate binary
  `target/dev/debug/rbitcoin-node` =
  `b75dad84bdd6663673a29c07352d10c4f78d2dcd14e885f62ad3209d214443e4`.
- Frozen packet (worktree `lab-node-compat-profile`, HEAD/base
  `b0ab4ca34fda07c61a6335d05f598eaacc69678c` = primary HEAD): all 82
  records of `/tmp/opencode/AP1b-r1-full-review-manifest.json` match
  (27 of them `*.log`); aggregate
  `9375653ca90158446b99ae5c02aaee8482153984c68ba53aef447b115df40050`
  reproduced. Patch record verified:
  `testing/rbitcoin-bark-v31/node-candidate.patch` =
  `97f84755b8ca470a3cc64145649eafae732913c2e2a02a116abfbe855fbb2a95`.

## 3. Prepared integrated source tree (before/after hashes)

Recipe (reproducible):

1. `mkdir /tmp/opencode/AP1b-lab-integration` (verified absent).
2. `rsync -a --exclude='/target' --exclude='/.git' --exclude='/docs/reviews' --exclude='/result' --exclude='__pycache__' --exclude='*.pyc' /tmp/opencode/rbitcoin-lab-retarget-profile/ /tmp/opencode/AP1b-lab-integration/`
   - same exclusions as the candidate's own construction
     (`baseline-binding.json`); the 12G `target/` was never copied; the
     submodule gitfile `third_party/bitcoin/.git` was removed to match
     the candidate's unanchored `.git` exclusion.
3. Pre-patch index ("before"): all 18 snapshot files match the L2
   manifest; aggregate `d2f37b91...` reproduced over the fresh copy.
4. `patch -p1 --dry-run` then `patch -p1` with the packet's
   `node-candidate.patch` (hash verified above): both exit 0; exactly 14
   files patched (13 modified + new
   `changelog.d/rpc-compat-profile-bark-v31.md`). No shell patch bypass;
   no optional nits or code changes were made.
5. Post-patch index ("after"):
   - each of the 14 diverged node files (incl. `COMPAT.md`, `docs/rpc.md`
     and the new changelog fragment) is byte-identical to the candidate;
   - all 25 candidate-manifest records match; aggregate
     `f8a51111...` reproduced over the prepared tree;
   - `diff -rq` vs the L2 tree (same exclusions) shows exactly the 14
     diverged files and nothing else;
   - `diff -rq -x target` vs the candidate shows **zero** differences:
     the prepared tree is byte-identical to the approved candidate
     outside `target/`.
6. Full-tree source index: 2102 files, canonical-JSON index aggregate
   `da21f56dd6ab581e0d8353fe7431cc44efa9eb29bffb998459cbd7ac29105a9e`
   (`/tmp/opencode/AP1b-lab-integration-source-index.json`, referenced
   and hashed in `/tmp/opencode/AP1b-lab-integration-prepared.json`).
   Tree size 40M; no `.git`, no `target/`, no runtime files.

## 4. Primary transfer (82 exact records)

All 82 packet paths (80 under `testing/rbitcoin-bark-v31/` incl. the r0
archive and logs, plus the two implementation reports) were absent in the
primary; each was transferred byte-exact from the frozen packet worktree
with mode 100644, then re-verified: 82/82 sha256 match, 82/82 modes match,
and the 82-record aggregate `9375653c...` reproduces over the primary
copies. No file was overwritten (guard refuses a differing existing
path); no newly generated artifact was substituted for an approved hash.

## 5. Promoted decision and changelog

`docs/decisions/2026-11-08-lab-bark-v31-profile.md` records the parent
lab adoption with the exact approved bindings and the authorization
boundary: opt-in isolated Ark development integration only; default node
binary `eb257af7...` and config unchanged; no Core v31 parity or full
Bark acceptance; profile default off `190000`, `310000` claim only on
regtest via CLI/conf; real identity + Core `IsChildWithParents`; the
removed env knob remains unapproved; the `getmempoolinfo` selected-model
null failure untouched; full `rbitcoin-node`/`rbitcoin-rpc` suites plus
fresh approved-probe default/profile runs on the INTEGRATED binary are
required BEFORE any opt-in use; funded/witness/real-Bark gates later. The
decision record keeps provider approval (Fable, bounded) distinct from
parent lab adoption and does not claim Fable approved a runtime
replacement. The changelog fragment is one bullet per
`docs/changelog/README.md`; `SPEC.md` is not spliced while the primary
gate is in progress.

## 6. Carried notes (not fixed in code)

Fable's non-blocking notes N1-N8 from
`docs/reviews/2026-11-08-bark-v31-profile-fable.md` are carried here as
prep notes only: N1 `docs/env-knobs.md` owner-row link, N2 stale
`config.rs` comment, N3 `run_rpc` hardening for a hypothetical second
constructor, N4 `conf`-prefixed CLI error wording, N5 harness
partial-failure cleanup/reap gaps, N6 report wording on approval vs
adoption, N7 `COMPAT.md` paragraph placement, N8 full-suite rerun
requirement (covered by the pre-use gate in the decision). None is a
code fix in this step; each belongs to a future node packet, never a
fork of the approved patch.

## 7. Pending validation (after the Nix gate completes; not run here)

To be dispatched by the parent on the integrated tree, reusing existing
dependencies with a separate safe binary target dir (disk ~21G free at
preparation; no new build was started):

- `cargo fmt -- --check` and
  `cargo clippy -p rbitcoin-node -p rbitcoin-rpc --all-targets`
  (node crates `rbitcoin-node`, `rbitcoin-rpc`).
- `cargo test -p rbitcoin-node --test suite` - full node suite (r0
  reference: 117/117; baseline L2 117/117-class receipts).
- `cargo test -p rbitcoin-rpc --test suite` - full rpc suite (r0
  reference: 214/214 after a truthful flaky rerun; rerun-cleared flakes
  are not a no-regression proof, hence the fresh full rerun).
- Fresh external-mode runs of the frozen approved probe (contract
  `f4fc3e043f420870675095ab24144ba14e0afba6cb9ad69d818d99b1bccb2f62`)
  default and profile-on against the INTEGRATED binary, via the packet
  harness `testing/rbitcoin-bark-v31/r1-external-run.py` (devcheck
  golden-vector gate first).
- Funded/witness/real-Bark gates and native WITNESS/T4b acceptance
  remain separate later work.

## 8. Non-claims

No build or test result is claimed. The integrated binary does not exist
yet; the candidate binary `b75dad84...` remains the only built artifact
and was produced by the packet owner, not this step. The 27 log records
are local evidence. Nothing here weakens the privacy constraints or the
fail-closed rules; no identity data crosses the ark role.
