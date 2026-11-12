# Live lab hub implementation and verification report

Date: 2026-11-08
Author: fresh-context recovery and sole writer for this packet
Status: **BOUNDED IMPLEMENTATION AND VERIFICATION COMPLETE - 84/84 recorded-browser checks passed; Fable first review required before primary integration**
Worktree: `/home/ethan/.local/share/opencode/worktree/ad8886/lab-live-hub`
Base: `b0ab4ca34fda07c61a6335d05f598eaacc69678c`
Plan: primary `docs/reviews/2026-11-07-lab-hub-plan-reviews.md`

The pre-fix implementation and receipts are preserved, not rewritten. The old
binary, original failed browser run, successful old-code checks and source
identity remain useful evidence within their stated scope. This recovery found
and fixed three real frontend defects and corrected misleading recorded-mode
and wallet text. The final source has passing direct Node tests and the Rust
suite passed after its only Rust label/test edit. The frozen final code then
received a new `spc-lab-cli` binary and passed all 84 recorded-browser checks
with byte-identical embedded/source assets at both required viewports. Fable
review remains required before primary integration; no approval is claimed.

## 1. Frozen inputs, preservation and resource limits

- Original pre-report manifest: `/tmp/opencode/LP-hub-pre-report-manifest.json`.
  Its file SHA-256 is
  `0f70d04f832edb4fa2326c4986fa3d16234aaf0d529d0585e8cf7779c5d87faa`;
  its 377-row `{base, files}` recipe reproduces
  `1a1c74661cb9c15c8d8aca0749e038465cdcb98abb5a24ab010cb47a8ae80012`.
- Original partial review manifest: `/tmp/opencode/LP-hub-review-manifest.json`,
  file SHA-256
  `4e3d0f3ef74547fd2df8cb88a323087dc0241a065da338949903490f7a48389e`,
  378 rows and full snapshot
  `503ec69595f3fd4283e0e67e47985e86a89209969de3f068efa67c93c8c6512f`.
  Its code hash excludes only this report and equals the pre-report recipe.
- Before any recovery edit, the partial report, both manifests, browser script,
  failed attempt-3 receipt, screenshots, DOM, logs and the three successful
  verification logs were copied to `/tmp/opencode/LP-hub-recovery-preedit/`.
  `preservation-receipt.json` there records 15 preserved files and their hashes.
  A later independent check found every original and archived copy unchanged.
  The archive also records that old serve PID 1761900 and both old Chrome PIDs
  were absent; no old process or fixture was killed or removed.
- `/tmp/opencode/LP-hub-run-1` was never adopted, resumed, mined or edited. The
  pre-edit walk found 2780 files with manifest SHA-256
  `defa82fa48fe678d3265629f7dd99020fe9eeb4f42d78bac5915238fb9ae344e`, journal
  `ac67e403edf1cd7a7db67da99ac43a098a01e0d27f889e29c455d9ff4a5b5866`
  and evidence
  `8b11b6a6f6a21571a4e4959d08d3516c733551f2fadcc49c5fa9d0a25e7a39ff`.
  Both completed recovery serves independently verified all 2780 files,
  journal and evidence unchanged.
- The original Nix `spc-lab` binary is hash-bound to the old source and was used
  only for pre-fix reproduction. It is not evidence for edited assets. New Rust
  commands reused `/tmp/opencode/LP-hub-verify-target`; no full workspace/Nix
  build, fresh target, source-copy target or shared/Nix cleanup was performed.
- Heavy-build commands used nonblocking
  `/tmp/opencode/silent-pool-heavy-build.lock` and checked the actual free space
  inside the critical section. The completed test/build started at 96 GiB and
  ended at 95 GiB. A later JS-only build found the lock held, logged `BLOCKED`
  once in `LP-hub-recovery-final-binary-build.log`, and was not retried or
  polled. After the parent checked once and found no lock holder, the final
  bounded build acquired the lock, started at 94 GiB, finished at 93 GiB, and
  explicitly released the lock after Cargo/compiler children exited and before
  any serve/Chrome spawn. The primary's separate six-check Nix gate is not rerun
  or claimed as hub proof.

## 2. Exact packet delta and recovery edits

The candidate remains the approved 370-file runtime baseline plus the original
17-file hub delta: **10 modified + 7 new files**. This is not a claim that all
17 were untracked. No approved baseline file was removed. There is no mockup,
EXP-OP duplicate UI, gallery feedback, user-note, pin, flake or Cargo manifest
edit in this packet.

The pre-fix report's 17 path list is retained in the immutable pre-report
manifest. Relative to that frozen 377-file code snapshot, this recovery changes
exactly seven source files and this report. An independent re-walk is recorded
in `/tmp/opencode/LP-hub-recovery-source-state.json`: zero missing baseline
rows, zero unexpected extra files, and exactly the following differences.

| Path | Pre-recovery SHA-256 | Current purpose |
|---|---|---|
| `bins/spc-lab/static/lab.css` | `09e8baf8cc45ad950a2c508afce8684fa70ddd20bdf350ef78903203ed981f46` | Correct grid/details intrinsic minimums while retaining an internally scrollable 850 px table |
| `bins/spc-lab/static/lab.html` | `1684baad91f4e4fc3498ad93b8ec39255f10815ea706c779e775c78ee55dbb8a` | Neutral pre-render monitor label and an id for view-accurate footer text |
| `bins/spc-lab/static/lab.js` | `9dad73d009f571adf5c35f8d82811c5b609fdb990c05c2cde28d0ffbcae87f16` | Preserve disclosure open/focus state across real polls; recorded labels; suppress stale external listener links |
| `bins/spc-lab/tests/lab_helpers.test.cjs` | `b385b6d1d7030285af58ee225716dfe745efcac91d16538308638578e0f9faff` | Focused recorded-mode and row-open regression tests |
| `crates/spc-lab/src/inventory.rs` | `74dad28044eeaa0249f61cac2b37d607d66ced40530b0ddea1f350a54c1ad437` | Replace the globally stale wallet-unimplemented sentence with the actual standard-run absence |
| `crates/spc-lab/tests/lab_hub.rs` | `f1635a5c64926ad6e9736db4484b7b5e66bfd4a9cbb184dd4dab48e2ba887773` | Targeted assertion for that scoped label |
| `docs/LAB-ROLES.md` | `8d6d864cfd450499013a5e33726fcc4d2c7c1d20bf4bb048fb005cbed31a2bd1` | Standard-run facts plus explicit primary-integration boundary |

The final source hashes are recorded in the external verified-review manifest,
not duplicated into a self-referential hash chain here. Seven original files
have group-write `0664` on disk; manifest `mode` remains the normal git-style
`100644`. This is only executable-bit normalization, not an approval or merge
gate. No mode housekeeping, commit, staging or flake housekeeping is directed.

## 3. Reproduced UI failures and fixes

### 3.1 Mobile overflow was a real layout defect

The old attempt-3 receipt and the independent recovery reproduction measured
`html/body scrollWidth = 868`, `clientWidth = 390` and a mobile layout viewport
of `868 x 1879` at an emulated `390 x 844` screen. Element geometry showed the
850 px table's min-content contribution escaping through `details.group`.
`overflow-x: auto` on `.table-wrap` could not constrain a grid item whose own
automatic minimum size was already 850 px.

The fix sets `.groups` to `grid-template-columns: minmax(0, 1fr)`, gives the
bordered disclosure elements `min-width: 0`, and caps `.table-wrap` at 100%.
The table keeps `min-width: 850px`; its wrapper provides the local horizontal
scroll. No content is deleted and there is no global `overflow: hidden` mask.
The first fixed binary then measured exactly `390 x 844` and 390/390 page
widths on mobile; desktop remained 1425/1425 with a 1440 x 900 viewport.

### 3.2 Keyboard failure had both harness and product causes

The old harness sent Enter keyDown/keyUp without Enter's text. The immutable
recovery harness sends native CDP key events with `text/unmodifiedText = "\r"`
for Enter and a single space for Space; it does not install a product keyboard
handler or assign a `details.open` attribute.

With the same old binary and source, corrected Enter immediately toggled both
the group and integration disclosures on both viewports. Thus the original
"Enter never toggles" observation alone was an incomplete-CDP-event artifact,
not a reason to add a duplicate application key handler.

A real defect remained: every two-second `innerHTML` replacement dropped the
focused summary to `BODY`. A second physical key dispatched without refocusing
therefore did nothing. The fix captures a keyed summary/link focus token,
restores the equivalent node after replacement with `preventScroll`, and moves
open-state capture to after the final network await so an in-flight response
cannot overwrite a newer interaction.

The first fixed binary passed all group and integration state/focus checks but
exposed one additional real omission: row-detail `open` state was collected in
the `Set` but never applied to replacement row markup. Both viewports showed
immediate row toggles followed by closure after polls, while focus remained
correct. Passing that set into `groupMarkup` and applying each `row-<id>` key is
the final bounded fix. A focused Node regression test covers this mapping. The
new final binary then passed every group, row-detail and integration
Enter/Space/open/focus check across two real polls per transition on both
viewports, completing 84/84 checks. The intermediate 80/84 run remains separate
history and is not represented as the final proof.

### 3.3 Recorded links and capability text

Recorded serve previously said `Open live controls` while its banner said
read-only, and rendered four `http://127.0.0.1:<old-port>/...` links belonging
to stopped actors. Now live mode retains `Open live controls`; recorded mode
uses `Open recorded monitor (read-only)`, including the controller-row link and
footer. Historical external API/metrics listener URLs become non-clickable
labels with an explicit historical/no-current-reachability note. Local `/` and
`/lab` routes remain links. The existing `/` application was not redesigned.

The inventory and role guide no longer say globally that wallet support is
unimplemented. They say what the standard scenario actually does: it launches
`spc-cpu-miner`, no `spc-agent`, no agent wallet and no Ark backend, and wires
no live per-user registration/wallet UI. Backend foundation is not denied.

## 4. Receipts and their exact scope

All paths below are under `/tmp/opencode` unless stated otherwise. Old failure
receipts remain evidence and were not edited.

### 4.1 Previously completed checks, not rerun unnecessarily

- `LP-hub-cargo-test-rust-online.log`: explicit exit 0 on the pre-recovery code,
  29 lib + 3 `http_sse` + 6 `lab_hub` tests.
- `LP-hub-node-test-rerun.log`: explicit exit 0, 22 tests on pre-recovery JS.
- `LP-hub-doc-check-online.log`: explicit exit 0. No doc-blocking error was
  found; existing warnings remain documented by the log.
- The original live host run and inventory/privacy evidence in section 5 remain
  bound to the old source and `fc17e002...` binary. They are not a new live
  experiment on the fixed binary.

### 4.2 Recovery reproductions and tests

| Artifact | Result and scope |
|---|---|
| `LP-hub-verify-browser-receipt.json` | Original attempt 3, **FAIL 5/44**: mobile overflow/viewport, both Enter checks. Old binary `fc17e002...` |
| `LP-hub-browser-recovery.js` | SHA-256 `0b463d72b407480f18d55845e79ec7d7b7d34f076d23fffcf0c3db6ac71a47f5`; unchanged assertions used for corrected reproduction, intermediate fixed proof and final proof |
| `LP-hub-recovery-pre-browser-receipt.json` | SHA-256 `99f8e42ffe3bbdede0d48a811ae88f4af45da66a24e469cd33125d471d86f52a`; old binary, **FAIL 27/84**. Proves native Enter works with correct events and independently reproduces focus, overflow, stale-link defects; served assets matched old source |
| `LP-hub-recovery-rust-build.log` | SHA-256 `90074a7dd35d9054c4bff4e1cc208ce85324629171a622499213b713ae490d9a`; explicit test and build exit 0 after the Rust label/test edit. Same 29 + 3 + 6 tests pass; target reused under lock |
| `LP-hub-node-recovery-final2.log` | Explicit exit 0, **24/24** direct-source tests on current JS, including recorded labels and row-open regression coverage |
| `LP-hub-recovery-final-run.log` and `LP-hub-recovery-final-browser-receipt.json` | Intermediate binary `d35fa232...`, **FAIL 4/84 (80 passed)**, only row-open state after polls on two viewports. All overflow, focus, keyboard, recorded-label, stale-link, console/network, no-POST and read-only-monitor checks passed. Its served embedded assets matched that intermediate source |
| `LP-hub-recovery-final-binary-build.log` | Historical **BLOCKED, not a pass**: nonblocking heavy-build lock was held. No poll/retry occurred. Preserved with the blocked manifest under `LP-hub-sealed-pre-report/` |
| `LP-hub-sealed-binary-build.log` | SHA-256 `b11b450523552d887dbd298a7c9301245a8a3bc97383af2ff109e9a608d2806f`; final `spc-lab-cli`-only build exit 0 in the reused target, 94 -> 93 GiB free; lock released before browser/serve spawn |
| `LP-hub-sealed-browser-run.log` + `LP-hub-sealed-browser-receipt.json` | Browser driver exit 0. Receipt SHA-256 `555e81f486212bcc3243b35fb3cd6e724faa55a486a08a67833ea9e9e407e6ed`; **PASS 84/84** on the new final binary, mobile and desktop |

The intermediate browser receipt also proves a real mouse activation of `/`
entered the existing recorded monitor in both sessions: banner visible,
`recorded / read-only` mode, 13 miner controls and zero enabled controls.
Across both sessions all requests were GET, inventory APIs returned 200, and
there were no console exceptions/error events, failed loads or HTTP >=400
responses. Serve exited 0, both owned Chrome processes exited, and no owned
process remained. These statements apply to that intermediate run, not to an
invented final run.

Four completed recovery Chrome profiles were removed only after their receipts
had embedded/source byte binding and no matching process remained. The exact
paths, sizes and receipt references are in
`LP-hub-recovery-browser-profile-cleanup.json`; no shared cache or other
session's profile was touched.

### 4.3 Final binary and 84/84 binding - complete

The final code snapshot is
`712fa8cba9326b65a91b889b1453484dca9f0d89c8ce2504786bc6653914f606`.
It did not change for this closeout, and the 24 Node and 29 + 3 + 6 Rust green
results were not rerun unnecessarily. `LP-hub-sealed-binary-build.log` records
only the final `spc-lab-cli` production build, exit 0, in the existing target.
The new binary is `/tmp/opencode/LP-hub-verify-target/debug/spc-lab`, SHA-256
`a577aea74e28a2eba64e517416e8baf733e068ba7c0b5a7aeb9e44aa235e5630`.

The unchanged browser script ran a fresh owned recorded serve of the stopped
run on an ephemeral port with fresh bounded Chrome profiles. Its receipt binds
that binary and all three embedded assets to the frozen source bytes:

| Asset | Source and served SHA-256 (identical) |
|---|---|
| `lab.html` / `/lab` | `b452cd0494526fc4bff63445d29b8a66c9a227f931b6bcf490e12ac7dfb2ca1a` |
| `lab.js` | `3c6f7b549dedfbcc855bf9291877210b0bda6b701269c14dcdd4a9f0883ce9d0` |
| `lab.css` | `ccb829145f213947e3943dae216a012f99ba900c44982d51b25115d86aa29d61` |

`LP-hub-sealed-browser-receipt.json` records `overall_ok: true`, 84 checks and
zero failures. In particular:

- Mobile: 390 x 844 actual layout viewport, HTML/body 390/390 widths; table
  content remains available through its local table scroll. Desktop: 1440 x 900
  viewport and 1425/1425 widths.
- Every group, row-detail and integration transition passed immediate native
  Enter/Space activation, open-state retention across two polls, focus retention
  across two polls, a second key without refocusing, and another two-poll check.
  The final row fix is therefore exercised in the embedded binary, not only by
  helper tests or manual DOM changes.
- Both sessions display `Open recorded monitor (read-only)`, have no external
  API/metrics anchor, and activate `/` with real mouse input. The linked monitor
  shows its recorded banner, `recorded / read-only`, 13 controls and zero enabled
  controls. The hub itself has no form/input/button controls.
- Every browser request is GET. There are zero exceptions, console errors,
  failed loads or HTTP >=400 responses. Serve exited 0 and both Chrome processes
  exited; cleanup scans found no owned serve or Chrome process.
- The run contains 2780 files before and after and the complete tree compares
  unchanged. Journal remains `ac67e403...b5866` and evidence remains
  `8b11b6a6...39ff`.

Artifacts include `LP-hub-sealed-mobile.png` (SHA-256 `97c80d1c...a673c`),
`LP-hub-sealed-desktop.png` (`3e9788da...4994`), both DOM files, both Chrome
logs and `LP-hub-sealed-serve.log`. Exact full hashes for these and all other
receipts are in the final manifest. `LP-hub-sealed-browser-summary.json` is a
compact review index; the full receipt is authoritative.

## 5. Existing live-run and privacy evidence, without overclaiming

The original native host validation remains the only live experiment. Its
receipts prove two co-located pool fronts, CPU miner ownership, recorded versus
observed state, allowlisted endpoints, read-only GETs, recorded attach behavior
and an unchanged command journal. The supporting table in the immutable
original report is preserved in the recovery archive; relevant receipt hashes
are carried into the final manifest.

The fixed source did not run a new live pool/node experiment. Runtime process,
protocol and native observer behavior was not edited. The only backend-source
hunk in this recovery is one display label and its test; therefore even the
qualified "older backend identical except frontend" shorthand is not used as
a byte-identity claim for the new binary. Old live evidence is reused only for
the unchanged native behavior it actually proves.

Recorded observations and external endpoint URLs remain historical facts in
the API projection; the frontend now declines to present those URLs as current
clickable destinations. No new health request or synthetic availability value
was added. Allowlisted serialization and the no-identity/private-material
boundary are unchanged.

## 6. Current primary status and merge alignment

Read-only primary checks and exact hashes are in
`LP-hub-recovery-primary-merge-audit.json`. Primary integration receipts exist
for T4a witness primitives, AP-1a Ark RPC probe tooling, AP-3a wallet
foundation and the T0 lab traffic contract. **This candidate does not contain
those primary changes.** The primary standard scenario still has zero
`ark_backends`, two `front/payout/api` fronts and two `spc-cpu-miner` miners.

AP-3a is foundation, not a live expanded lab: its own scope excludes live
registration transport, full stock-bark `Wallet::open`, chain scanning and
delegated refresh/offboard. The hub's `not running in this experiment` and
`planned required integration` labels therefore describe the run, not the
existence of primary source code.

Current merge surface against the actual primary, not the older assumption in
the partial report:

- `main.rs`, `app.js`, `index.html`, `http.rs`, `http_sse.rs` and
  `docs/ARCHITECTURE.md` still match the approved baseline on primary.
- `README.md`, `crates/spc-lab/src/engine.rs`, `crates/spc-lab/src/lib.rs` and
  `docs/LAB-RUNTIME.md` have primary-side content beyond that baseline and need
  serial, semantic merge review. In particular, do not overwrite T0 changes.
- The seven original new hub files are still absent on primary. Their recovery
  versions, not the stale pre-recovery bytes, are the integration input.
- No candidate change to `Cargo.toml`, `Cargo.lock`, `flake.nix`, dependency
  pins or upstream versions exists. Their primary/OP/T1 state is not a hub merge
  obligation. A fake commit or flake-tracking cleanup is neither required nor
  authorized here.

The final handoff is to **Fable for code/evidence review before primary
integration and any new manual live lab session**. This report is not a Fable
approval.

## 7. Verified-review manifest and remaining gates

`/tmp/opencode/LP-hub-verified-review-manifest.json` is the external final
packet index. It includes the complete 378-file source/report set and modes,
the original 17-file approved-baseline delta, the seven recovery edits, new
code/full recipe hashes, original manifest bindings, final source/binary
evidence and receipt hashes. It deliberately contains no hash of itself.
The blocked full snapshot `292d3b8f...e415` and its manifest file
`c591e10c...f3837fdb` are preserved before replacement under
`/tmp/opencode/LP-hub-sealed-pre-report/`; the old 57/84 and 80/84 receipts are
also untouched.

The bounded implementation and final binary/browser gate is complete.
Remaining gates, in order:

1. **Fable first review** of this final source, manifest and receipts. No
   primary integration precedes that review.
2. Apply the hub delta serially with the current primary semantic merge points
   above, then run whatever integration checks the merge owner requires. No
   new live run is claimed by this packet.
