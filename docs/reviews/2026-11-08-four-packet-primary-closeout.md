# Four-packet primary closeout: final merged-tree Nix flake check (2026-11-08)

Owner: serial primary code integration owner
Status: closeout evidence only - **no code changes in this step**; the
approved packet bytes are exactly as transferred and verified
(parent-independent verification PASS: 126 distinct transferred paths,
exact hash/modes, reconstructed SPEC matching `3f669947...`, index
`44f21a92...` unchanged).

This document closes out the four approved primary integrations and
records the final required Nix gate. It supersedes nothing else; all
prior receipts are untouched and linked below.

## 1. The four packets and their receipts (untouched)

| Packet | Integration receipt | External manifest |
|---|---|---|
| T4a witness primitives | `docs/reviews/2026-11-08-witness-primary-integration.md` | `/tmp/opencode/T4a-primary-integration-manifest.json` |
| AP-3a agent wallet (fix-r2) | `docs/reviews/2026-11-08-agent-wallet-primary-integration.md` | `/tmp/opencode/AP3a-primary-integration-manifest.json` |
| AP-1a ark-rpc probe (r2) | `docs/reviews/2026-11-08-ark-rpc-probe-primary-integration.md` | `/tmp/opencode/AP1a-primary-integration-manifest.json` |
| T0 traffic contract (fix-r1) | `docs/reviews/2026-11-08-traffic-contract-primary-integration.md` | `/tmp/opencode/T0-primary-integration-manifest.json` |

## 2. Nix gate history, truthfully distinguished

1. **Pre-T0 scoped `test-ark` gate** (recorded in the AP-3a receipt,
   preserved untouched): `nix build ... #checks.x86_64-linux.test-ark` on
   the **pre-T0/AP-1a merged tree** snapshot; exit 0, derivation
   `gq00f0dls...`, spc-crypto (ark) 102 + spc-pool (rpc,ark) 274 +
   spc-agent (ark) 131/1 ignored = 507 passed / 0 failed. Scope: it does
   **not** build spc-lab (T0) or the AP-1a tooling tree.
2. **Final closeout gate (this document)**: full `nix flake check` - all
   six checks - on the **current merged tree** (all four packets). This
   was the required remaining gate: the reviewer requested the merged
   flake check, and the prior test-ark run alone did not close it.

**Corrected rationale (supersedes the AP-3a receipt's skip note, which is
left untouched per instruction):** the AP-3a receipt stated the full
flake check was not run because it "would reexecute unrelated
namespace/native checks with cold closures". Reading the actual flake,
that rationale was wrong: the `checks` attrset defines exactly **six**
checks - `fmt`, `clippy`, `doc`, `test`, `ui-node`, `test-ark` - and no
live namespace or native hooks (the default `cargo test` suites carry
their ignored netns/node-gated tests). `nix flake check` additionally
*evaluates* (does not build) `packages`, `devShells`, `nixosModules` and
`overlays`, and by default omits incompatible systems
(aarch64-darwin, aarch64-linux, x86_64-darwin are skipped with a
warning; x86_64-linux - the system all receipts target - is checked).
The skip was therefore unnecessary; the final gate below closes it.

## 3. Final gate: `nix flake check` on the merged tree

- **Command**: `nix flake check --max-jobs 1 --cores 2
  path:/tmp/opencode/four-packet-nix-src` (run 2026-11-08
  05:51:58-06:08:12 EST; serialized with all other work).
- **Exit: 0.** Full log: `/tmp/opencode/four-packet-nix-gate-logs/`
  (`flake-check.log` plus one `nix log` capture per check). External
  receipt: `/tmp/opencode/primary-four-packet-nix-gate.json` (sha256
  `abaabc23113cdb1c261daa2025ca023bd4fd40f60034c02d46ae12eb8b3beb74`).

### 3.1 Clean source-only snapshot (no raw worktree ingestion)

`/tmp/opencode/four-packet-nix-src`: 523 files, 9,8 MB, aggregate sha256
`92dc2e25c70031c33b00c90171a206aec47c4a1c8abfde779ff46e9902e417a9`
(sorted-key compact JSON `{base, files}` row manifest at
`/tmp/opencode/four-packet-nix-src-manifest.json`). Frozen from the
current primary working tree, excluding: `.git`, any `target/` (never
raw-copied - the 30G hazard), `__pycache__` (runtime `.pyc` from the
AP-1a unittest execution), root `result*`, `spc_ark_e2e/test` (gitignored
regtest runtime), symlinks. Verified byte-faithful by an independent
re-walk of both sides (identical `{path, mode, sha256}` row sets; T0
15/15 and AP-1a 74/74 manifest rows byte-equal inside it;
`docs/SPEC.md` = `3f669947...`): `/tmp/opencode/four-packet-snapshot-verify.log`.

`flake.lock` is byte-identical in the snapshot and the primary
(`2152143d...`): existing trusted cache/pins only; no flake, lock or
trust change; realized outputs may be reused, nothing force-rebuilt; no
shared cache deletion or GC. Disk: 22G free before, 21G after.

### 3.2 The six checks (actual results)

| Check | Derivation | Result |
|---|---|---|
| `fmt` | `npp1brscfnx3mgz01r524ib5qi6m0qvz-spc-fmt-0.1.0.drv` | `cargo fmt --all -- --check`: clean |
| `clippy` | `7i0ksq2apq3x1li3dk9h5sm0xsc26ciw-spc-clippy-0.1.0.drv` | `cargo clippy --workspace --all-targets --offline -- -D warnings`: zero warnings |
| `doc` | `2d6mvbiv3mknq7nvzkir548l1ba8jj1l-spc-doc-0.1.0.drv` | `cargo doc --workspace --no-deps --offline`: built |
| `test` | `d234ijbr091vd762a2wk3i3b1vmzjgr8-spc-workspace-tests-0.1.0.drv` | `cargo test --workspace --offline`: 83 test binaries, **724 passed, 0 failed, 16 ignored** (first Nix-level run of the T0-merged spc-lab, including traffic_contract 42 and the engine prepare/adopt gate suites) |
| `ui-node` | `yfxiplk7bm0k3r0xnn3bmjf50r80yqf8-spc-lab-ui-tests.drv` | `node --test bins/spc-lab/tests/*.test.cjs`: **14 tests, 14 pass, 0 fail** |
| `test-ark` | `9pnh9z7fgsmvrnpqxfsnd5ivk1q8v2by-spc-ark-feature-tests-0.1.0.drv` | spc-crypto (ark) + spc-pool (rpc,ark) + spc-agent (ark): 27 test binaries, **507 passed, 0 failed, 1 ignored** (rebuilt from the new source; same totals as the pre-T0 scoped gate) |

Source store binding: `/nix/store/i116v18ygjxl9ym9ywn1pv89pjjzq6pf-source`
(narHash `sha256:09q5w31an1ri430ajkf6ggbxm9jmjvr361gxlvkxzd64d56lc8xx`;
523 files, matching the snapshot exactly; `docs/SPEC.md` inside the
store source verified as `3f669947...` and the T0 traffic rows match
their manifest entries byte-exact). The naive in-gate requisites grep
initially matched an unrelated nixpkgs hook source; the corrected lookup
method is recorded in the gate log and the external receipt.

## 4. T0 receipt wording precision (applied, no code edit)

The T0 integration receipt's section 3 now states precisely: the derived
changelog bullet inserted into `docs/SPEC.md` is **1500 bytes without a
trailing newline**, sha256
`9b3abff5f1837fa142318aa0661bfe5e8c1262433fb08b18201f67ef91ff2e9a`, and
the reference copy at `/tmp/opencode/T0-integration-spliced-bullet.txt`
is **1501 bytes** (bullet plus one trailing newline), sha256
`ed9325a424075025b347686307bac6470fceba498fe485c124007d0445c05bb2`.
The external T0 manifest (`11fb3757...`) is untouched so the
parent-independent PASS verification against it stands.

## 5. Final state

- Index `44f21a926d4d065ac6937fb0f81b698c7b2d0627` before == after
  (== HEAD tree; nothing staged; no commits made anywhere in this work).
- All four packets frozen; the only in-repo writes in this closeout were
  the T0 receipt section 3 wording correction and this new report.
- The `__pycache__` directories under `testing/ark-rpc/` (created by the
  sanctioned AP-1a unittest check, excluded from the gate snapshot)
  remain in the working tree untracked; they are runtime artifacts, not
  packet content, and removing them is left to the owner's discretion.
- Remaining work belongs to the other agents (AP-2 narrow-hunk merge,
  AP-3b followups, T1 recording path); nothing here starts it.
