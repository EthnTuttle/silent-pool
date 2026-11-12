# Design review app (lab-design-mockups): Fable review

Date: 2026-11-07
Author: Fable subagent (bounded review of the DESIGN FEEDBACK TOOL only)
Status: **REQUEST_CHANGES** - 3 blocking, 4 should-fix, several minor
Scope: the 8 candidate files under
`/home/ethan/.local/share/opencode/worktree/ad8886/lab-design-mockups`
(`design/review-app/**` + `docs/design/2026-11-07-review-app.md`) at
canonical base `b0ab4ca34fda07c61a6335d05f598eaacc69678c`.
Snapshot: `/tmp/opencode/lab-design-review-app-review-snapshot.json`,
sha256 `dc63be6ba58e93bf7513a0f6bfe617ebac325b339a7fc25b56b1ec957c07a087`.
All 8 file hashes re-verified byte-for-byte against the snapshot before
reading (`sha256sum`, worktree HEAD = `b0ab4ca`).

Out of scope and deliberately not done: `design/mockups/**` (designer-owned,
concurrently mutating; not read, no claims made about it), the product
design itself (a separate later Fable check), any production UI, mining,
Nix. No candidate file was edited; nothing was committed; no subagents
were spawned; the real state dir `~/.local/share/silent-pool/design-review`
was never created and port 18481 was never bound (verified at the end).

Evidence (all outside the repo, reproducible):

- `/tmp/opencode/fable-review/probe_server.py` (sha256
  `701df6cd...6190dd1f5da`): 22 server-level probes, 12 pass.
- `/tmp/opencode/fable-review/probe_ui.py` (sha256
  `108132c1...ead6959c7`): 32 real headless-Chrome UI flows driven over
  the DevTools protocol (Chrome 156, disposable profile, `websockets` 16
  sync client already on the system; no Playwright), using a **real
  1280x800 / 390x844 patterned PNG fixture** and real note CRUD; 26 pass.
- `/tmp/opencode/fable-review/probe_dup.py` (sha256
  `cace80b2...361344050`): the duplicated-view bug, 1 of 4 pass.
- Screenshots: `/tmp/opencode/fable-review/shots/*.png`.

Baseline reproduced first: `test_review_app.py` **26 OK**,
`test_browser_smoke.py` **3 OK**, exactly as reported.

## 1. Verdict

The server core is well built: atomic JSON writes, fail-closed on corrupt
or unknown-version state (file preserved byte-for-byte, confirmed),
optimistic 409 with the current state returned, orphan retention, verbatim
block-quoted Markdown with an explicit "feedback, not instructions" label,
strict path containment (traversal/symlink/NUL/backslash/absolute all
refused, re-tested), whitelist-only static files, no CORS, Host/Origin/
Sec-Fetch guards on POST, sandboxed HTML (CSP `sandbox` verified to block
the mockup's inline script top-level and inside the iframe while a relative
`<img>` still resolves). The UI's note path really works end to end: typing
stores a per-`design_version:mockup` draft that survives reload, Add writes
`feedback.json` + `feedback.md`, Edit/Delete/status round-trip, a stale
second tab gets the 409 toast with its text kept and the second press saves
exactly once, and everything is still there after a server restart.

But three things the user will hit immediately are broken or unsafe, and
the headless smoke test could not see them because it used a 1x1 PNG and
never changed a filter, opened a compare, or zoomed:

- **B1** Changing any filter or the compare selector appends a second
  (third, fourth...) copy of the whole page instead of re-rendering; the
  duplicated panels share element ids, so a note saved from the visible
  (lower) editor updates the stale (upper) list. Screenshot
  `dup-after-filter.png`, `compare.png`.
- **B2** Two server processes on the same `--state-dir` silently clobber
  each other's notes. There is no cross-process lock, only a per-process
  `RLock`; the "409 so concurrent edits never silently lose notes" claim
  holds only within one process.
- **B3** Zoom is broken for real-size mockups: at "100%" the left ~190 px of
  a 1280 px mockup is unreachable (centered flex overflow clips the left
  edge; screenshot `detail-z1.png`), and "200%" renders at exactly the same
  size as 100% (flex-shrink defeats `width: 200%`). The README's
  "fit -> 100% -> 200%" is not what ships.

Fixes are small and local (see section 3). Re-review needed after.

## 2. What was verified (passes)

Server (probe_server.py + candidate tests): corrupt JSON / schema 99 / bad
shape -> 503, file untouched; `os.replace` failure -> 500, previous file
intact, no temp leftovers; GETs never mutate; 6 MB PNG served whole with
`image/png`, HEAD returns headers only; `.txt` in catalog dir -> 403; state
files unreachable by any path; incomplete headers and short bodies dropped
at the 20 s socket timeout; oversized body 413; Host variants
`127.0.0.1.evil.example`, `localhost.evil.example`, `127.0.0.1:1:2` all
rejected on POST; 8-thread optimistic retry lands 8 notes at revision 8.

UI (probe_ui.py, real fixture): gallery thumbs load the 1280x800 image and
fit the card (`object-fit: contain`); detail "fit" has no horizontal
scroll; compare shows both panes fitted (1280 and 390 wide images); draft
keyed `draft-1:O1` persists in localStorage across reload and is deleted on
save; textarea cleared on save; note text `<b>not html</b>` rendered
literally (no `<b>` element created; `textContent` throughout, no
`innerHTML` anywhere in `app.js`); `feedback.md` contains the note
block-quoted line by line; status button -> active + persisted; Edit ->
Save, Delete (confirm dialog) both persist; overall note stored with
`mockup_id: null`; sandboxed HTML mockup: script did not run
(title/background unchanged) top-level or in the `sandbox=""` iframe, and
its relative `O1.png` resolved to 1280 px; ArrowRight -> `#O2`; after
server restart the UI shows both notes and the `revise` status.

## 3. Findings

### Blocking

**B1 - `renderMain()` appends, never replaces, so filters and compare
duplicate the page and break save feedback.**
`design/review-app/static/app.js:202` (`appEl.appendChild(main)`) is
reached from `app.js:212` (filter `change`) and `app.js:319` (compare
`change`) without the `appEl.replaceChildren(head)` that only
`renderHeader()` (`app.js:142`) performs. Counterexample
(`probe_dup.py` D1-D3): three Role-filter changes -> 4 `<main>` elements,
10 cards for a 4-screen catalog; one compare change -> 2 `<main>`, 2 note
editors with duplicate ids `notes-list-O1` / `status-panel-O1`; Add note
from the visible lower editor saves to disk (good) but
`renderNotesListOnly()` (`app.js:472`, `getElementById`) refreshes the
upper stale panel: per-panel counts `[1, 0]`, i.e. the user sees "No notes
yet." under the note they just saved. Fix: have `renderMain()` remove the
existing `main` (or call `renderAll()`) before appending. Test: after a
filter/compare change assert
`document.querySelectorAll('main').length === 1`.

**B2 - no cross-process single-writer contract; a second server on the
same state dir destroys notes.**
`design/review-app/server.py:359` (`threading.RLock`) is the only
serialization; `server.py:435-454` writes the in-memory state without
re-checking the file. Counterexample (`probe_server.py` P1): servers A and
B started on the same `--state-dir` (different ports); A adds "note via
server A" (revision 1 on disk); B, still at in-memory revision 0, accepts
`expected_revision: 0` and writes -> disk now `notes=['note via server B']
revision=1`. A's note is gone with two 200 OK responses. This is realistic
here: the README itself suggests passing either the repo root or
`design/mockups` as root, agents may start a second instance on another
port for API access, and `docs/design/2026-11-07-review-app.md:80-83`
promises "concurrent edits never silently lose notes". Fix (either, ideally
both): (a) take a non-blocking `fcntl.flock` on `<state-dir>/.lock` in
`FeedbackStore.__init__` for the process lifetime and refuse to start
("state dir already served by pid N") if held; (b) before each `_persist()`
stat `feedback.json` and compare (`st_ino`, `st_size`, `st_mtime_ns`) with
the values recorded at the last load/write; on mismatch return 409/503
"state changed on disk" rather than overwrite. Document in README that
`feedback.json` must not be written by hand while a server runs. Test: P1
above should end with both notes on disk or B refusing to start.

**B3 - zoom: 100% clips the left edge; 200% does not zoom.**
`design/review-app/static/app.css:73-78`. `.imagebox` is
`display:flex; justify-content:center; overflow:auto`; a centered flex
item wider than the box overflows both sides and the left overflow is
unscrollable. Counterexample (`probe_ui.py` U5b): at z1 on a 1280 px
image, `scrollLeft=0` gives `imgLeft=-164.5` vs `boxLeft=16`,
`scrollWidth=1107 < 1280+16`; visible in `detail-z1.png` (the black
left-border stripe of the fixture is gone). For `.z2`, `width: 200%` on a
flex item with default `flex-shrink: 1` shrinks back to the line: U6
measured 1280 px at z2 (identical to z1) for the desktop image and 901 px
(= box width, not 2x) for the 390 px mobile image (U7b). Also `.z2` never
sets `cursor: zoom-out` (U6b). Fix: `.imagebox img { flex: none; }` plus
`justify-content: safe center` (or drop flex centering and use
`margin: 0 auto` on the fit state only), and add `.imagebox.z2 { cursor:
zoom-out }`. Test: at z1 `img.getBoundingClientRect().left >=
box.getBoundingClientRect().left` with `scrollLeft=0`; at z2
`img width ~= 2 * box.clientWidth`.

### Should fix before the user review

**S1 - Host header is only checked on POST; GET leaks feedback to DNS
rebinding.** `server.py:737-780` (`_route_get`) never calls `_host_ok`
(`server.py:667`), which is used only in `_post_guard` (`server.py:680`).
Counterexample (P2/P2b): `GET /api/feedback` and `/api/export.md` with
`Host: evil.example:18481` -> 200 with the note text. A page on an
attacker-controlled name resolving to 127.0.0.1 reads all notes
same-origin (no CORS needed). The README's security model
(`README.md:176-194`) says nothing about this. Fix: apply `_host_ok` to
every request (return 400/421). Test: P2.

**S2 - SVG mockups opened top-level run scripts at the app origin.**
`server.py:814-818` sets `Content-Security-Policy: sandbox` only for
`TEXT_TYPES`; `.svg` is in `IMAGE_TYPES` (`server.py:66`) and the detail
view offers "Open image" top-level (`app.js:309`). Counterexample (P3):
`evil.svg` with an inline `<script>` served `image/svg+xml`, `CSP=None`; a
script there can POST to `/api/notes` with a passing same-origin `Origin`.
Fix: send the same `sandbox` CSP for `.svg` (harmless for `<img>` use).
Test: assert CSP header on `/mockups/**.svg`.

**S3 - design_version can be silently wrong on notes and statuses.**
(a) `app.js:111` polls the catalog only in the `awaiting` state;
`app.js:432` stamps notes with the catalog cached at page load.
Counterexample (U24): tab opened at `draft-1`, designer bumps the catalog
to `draft-2` (same image URLs, `Cache-Control: no-store` so the browser
fetches the new bytes), user adds a note -> stored `design_version:
'draft-1'`, header badge still "draft-1". The note is attributed to the
wrong revision with no warning. Fix: poll `/api/catalog` always (or ETag
on `design_version`) and show a "catalog changed to draft-2 - reload"
banner; and/or have `add_note`/`set_status` reject a `design_version` that
differs from the current OK catalog with a clear 409 ("catalog is now
draft-2; reload"). (b) Statuses are a single slot per mockup
(`server.py:615-619`) and neither the UI (`app.js:84-87`) nor the export
(`server.py:306`) shows their `design_version`. Counterexample (P6): O1
"like" set on draft-1, catalog moves to draft-2, export line reads
`### O1  (status: like)` with no version. Fix: render
`(status: like, set on draft-1 - stale)` when it differs from the catalog
version, and show the same in the status panel; consider keeping status
history per version rather than overwriting.

**S4 - catalog warnings never reach the user.** `app.js` never reads
`catalog.warnings`. Counterexample (U25): catalog overwritten with a
partial write, Refresh pressed, server returns the last-good view plus
`warnings=['catalog unreadable ... showing last good version if any']`,
page shows nothing. Per-mockup warnings (missing image, bad role) are
equally invisible. Fix: render `c.warnings` under the header banner.

### Minor / documentation

- **M1** Failure *after* `os.replace` (the directory fsync at
  `server.py:121-125`) rolls memory back (`server.py:451-453`) and
  answers 500 "internal error; state unchanged" (`server.py:863`) while
  the disk already holds revision N+1. Counterexample (P5/P5b): disk
  rev 2 / 2 notes vs memory rev 1 / 1 note; the next successful write
  dropped the on-disk note. Rare path, but the message is false. Fix:
  treat post-rename failures as a logged warning, not a rollback (or
  reload from disk on any exception after `_persist`).
- **M2** `_load` (`server.py:401-410`) accepts any list for `notes`, then
  `render_markdown` indexes `n['created_at']` etc.; `_write_md` is guarded
  by `except OSError` only. Counterexample (P4): a hand-edited note lacking
  `created_at` -> `KeyError` traceback at startup (file untouched, so fail
  closed, but not the documented 503). Validate per-note shape in `_load`
  and set `self.error`.
- **M3** On add-conflict (`app.js:441-442`) the toast says "state reloaded"
  but `renderNotesListOnly` is not called; the list stays stale until the
  next action (U16b: shows 1 while 2 exist). Status conflict handles this
  correctly (`app.js:387`).
- **M4** The shell CSP `style-src 'self'` (`server.py:663`) blocks the
  inline `style` attributes set at `app.js:449` and `app.js:465`
  (console violations; "No notes yet." renders centered). Use classes.
- **M5** `_host_ok` accepts `[::1]:x` (bracket branch skips port
  validation, `server.py:671-672`); harmless since the host is loopback.
- **M6** `expected_revision: null` bypasses the check (`server.py:540`).
  Intentional for CLI, but undocumented in the README API section.
- **M7** When `catalog.json` is *absent* (not partial) the view flips to
  `awaiting` and the gallery disappears, although images keep serving from
  the cached base (P7/P7b). Acceptable; mention it in the README's
  "mid-write" paragraph or keep last-good for a grace period.
- **D1** `docs/design/2026-11-07-review-app.md:102-108` says the smoke
  suite verifies that deep-link `/#O1` "renders metadata/status/note-editor
  panels" and produced `smoke-detail.png` / `smoke-overall.png`.
  `tests/test_browser_smoke.py:94-125` does neither: it only calls the API
  and asserts the output filename ends in `.png`; the two PNGs were not
  produced by the test. State plainly that DOM smoke covers gallery and
  awaiting only. The 1x1 fixture (`test_review_app.py:27`) cannot exercise
  fit/zoom; add a generated non-trivial PNG (the probe shows ~15 lines of
  stdlib code suffice).
- **D2** `README.md:78` "fit -> 100% -> 200%" (see B3); `README.md:122-124`
  and the handoff doc's "never silently lose notes" need the single-process
  qualifier until B2 is fixed; `README.md:176-194` should state that the
  guards are loopback/same-origin hygiene, not user authentication
  (anything running as the user, or any same-origin page, can write).
  The handoff doc's "authenticated-for-loopback POSTs" (line 80) overstates
  this.

## 4. Suggested acceptance tests for the fix

1. Browser: after each of Role/Platform/Direction change and a compare
   change, `querySelectorAll('main').length === 1` and one `.note-input`.
2. Browser: z1 left edge reachable; z2 width ~= 2x box; third click fit.
3. Server: P1 two-server scenario -> second refuses to start (lock) or the
   second write fails 409/503 and both notes remain on disk.
4. Server: `GET /api/feedback` with non-loopback Host -> 400/421.
5. Server: `/mockups/x.svg` carries `Content-Security-Policy: sandbox`.
6. Server/browser: catalog `design_version` bump while a tab is open ->
   banner shown and/or note add refused with a message; export marks a
   status whose `design_version` differs from the catalog as stale.
7. Browser: catalog partial write + Refresh -> warning text visible.

## 5. Cleanup

Temporary servers used OS-assigned ephemeral ports and `mkdtemp` state
dirs (all removed); Chrome profiles were disposable and removed; no files
under `~/.local/share/silent-pool` exist; port 18481 is not listening. The
only artifacts left are under `/tmp/opencode/fable-review/`.

---

# R2 verdict (2026-11-07, same day, second pass)

Status: **REQUEST_CHANGES (bounded) - no blockers remain; 2 should-fix,
3 minor.** All R1 blockers (B1, B2, B3) and all R1 should-fix items
(S1-S4) are verified fixed with the candidate's own suites plus
independent probes. The two remaining items are small (~10-20 lines each)
and affect the real review scenario, so I am not issuing a clean APPROVE.

Scope: 9 frozen files at canonical base `b0ab4ca34fda07c61a6335d05f598eaacc69678c`.
R2 snapshot `/tmp/opencode/lab-design-review-app-r2-review-snapshot.json`,
sha256 `123c2eceb77da1b52973e7b62a1a76ff49c112a65afcc6faf9538f1b2af6e518`;
input manifest sha256
`cfe4edae8e54bbb559337f4df18d7a40584dfff2f209e5d0c0667f035d98a40b`.
All 9 hashes re-verified before reading and again after testing (no
candidate file changed). Designer `design/mockups/**` not read; no product
design judgement implied. No commits, no subagents, no real state dir
(`~/.local/share/silent-pool` does not exist), port 18481 never bound.

Evidence: `/tmp/opencode/fable-review/probe_r2.py` (sha256 `0672ec5b...`),
`probe_svg.py` (`01485577...`), screenshots
`/tmp/opencode/fable-review/shots-r2/`. Candidate suites reproduced as
shipped: `test_review_app.py` **35 OK**, `test_browser_smoke.py` **2 OK**,
`test_browser_cdp.py` **5 OK** (Chrome 156 via DevTools protocol, system
`websockets` 16). Candidate screenshots `fix-zoom-{fit,100,200,mobile}.png`
inspected: patterned 1280x800 fixture, left black border visible at 100%
and 200%, both scrollbars present, mode label shown.

## R1 findings: status after R2

| R1 id | Status | How verified |
|---|---|---|
| B1 duplicate `<main>` | Fixed | `app.js:253-281` removes the old `main`; CDP test_1 (3 filter changes -> 1 main / 2 cards; compare -> 1 editor, 1 `status-panel-O1`; visible editor saves to visible list). Independently confirmed via the candidate test run. |
| B2 two-process clobber | Fixed | `server.py:405-418` exclusive `flock` on `<state-dir>/.lock` taken before `_load`, fd held for process life, never unlinked; `main()` exits 1 on `StateLocked`. Candidate tests: in-process refusal, true two-process subprocess refusal, restart keeps notes. My R1/R1b: a server whose *bind* fails (port busy) exits 1 and its lock is released so the next server on that state dir starts. |
| B3 zoom | Fixed | `app.css:73-84` block layout, `margin: 0 auto` only in fit; `app.js:362-400` sets `naturalWidth` / `2*naturalWidth` via CSSOM (not blocked by CSP); per-pane label `fit/100%/200%`, `zoom-out` cursor at 200%. CDP test_2: z1 = 1280, z2 = 2560, `imgLeft >= boxLeft` at scroll 0, both axes scroll; mobile 390/390/780. Intrinsic semantics accepted per parent. |
| S1 Host on GET | Fixed | `server.py:873-876` -> 421 on every GET/HEAD. My R6: `127.0.0.1:0`, `127.0.0.1:65536`, `[::1]x` -> 421; `LOCALHOST:<port>`, `[::1]`, `[::1]:<port>` -> 200; HEAD with evil Host -> 421. |
| S2 SVG CSP | Fixed | `server.py:958-962`. My R5: an SVG whose inline script POSTs a note, opened top-level in Chrome, wrote nothing; SVGs still render as thumbnails and in detail (`probe_svg.py`, naturalWidth 200). |
| S3 stale design_version | Fixed | `server.py:665-675` 409 on mismatch when a usable catalog is loaded (none when awaiting); UI polls always (`app.js:130-156`), banner + badge; status panel and export carry `set on <v>` and `STALE` (`server.py:325-336`, `app.js:477-487`). My R7/R7b/R7c confirm export lines before/after a bump. See R4 below for the draft side of this flow. |
| S4 warnings invisible | Fixed, but over-gated | Banners render (`app.js:193-209`). See R3 below. |
| M1 false "unchanged" | Fixed | `PostCommitError` (`server.py:84-87, 142-150, 553-555`): rename = commit, 200 with new revision, md/dir-fsync as warnings. Candidate test injects a real post-rename dir-fsync failure. See R2 (minor) on client-visible semantics. |
| M2 startup KeyError | Fixed | `_valid_note` + status shape checks (`server.py:421-490`) -> 503 "malformed", file untouched (candidate test). |
| M3 conflict list stale | Fixed | `app.js:564-566`; CDP test_4. |
| M4 CSP inline style | Fixed | classes `.hint-left`; zoom via `element.style` (CSSOM). |
| M5-M7, D1, D2 | Fixed / documented | README durability, auth-hygiene wording, `expected_revision: null`, catalog-absent behaviour, test claims now match test code (35 / 2 / 5). |

## Remaining findings

### Should fix

**R3 - status gating keys on *any* catalog warning, including permanent
per-entry ones, so one imperfect screen disables statuses on every
screen.** `app.js:492` (`const ambiguous = S.catalogWarnings.length > 0`)
does not distinguish the transient last-good warnings
(`server.py:614-615`, `621`, `607`) from static validation warnings
(`server.py:232`, `247`, `259`). Counterexample (`probe_r2.py` R3/R3b,
screenshot `shots-r2/static-warning-gating.png`): a valid catalog with one
extra entry `{"id": "D1", "role": "admin", "image": "screens/D1.png"}`
(image not yet produced) -> two banners, **all four status buttons
disabled on O1** with "catalog ambiguous - statuses disabled until a clean
refresh"; Refresh does not clear it because the warnings are permanent.
With the designer still producing 14 screens, a missing asset or odd field
is likely during the real review. Fix: have the server mark last-good
views explicitly (e.g. `"stale_view": true` set in `_cat_on_error` and the
rejected-catalog branch) and gate on that flag only; keep static warnings
as banners. Test: catalog with a missing image -> banners shown, status
buttons enabled; partial write -> banners shown, buttons disabled.

**R4 - after "Reload now" the draft typed against the old version is not
shown, and there is no UI path to save it.** Drafts are keyed
`design_version:mockup` (`app.js:38-40`), the banner says "Reload to
rebind to the new version - your unsaved drafts are kept"
(`app.js:198-201`), and the handoff doc says "drafts preserved". The
server correctly rejects the save before reload (409, text kept: R4a
passes); after reload the editor reads `drafts["draft-2:O2"]`, which is
empty. Counterexample (`probe_r2.py` R4b, screenshot
`shots-r2/after-rebind-editor.png`): editor value `''`, localStorage
`{'draft-1:O2': 'long paragraph typed against draft-1'}`. From the user's
point of view the paragraph is gone; it is only reachable if the catalog
reverts to draft-1. (CDP test_3 masks this by re-typing the text after the
reload, `test_browser_cdp.py:302-304`.) Fix: when an editor opens with an
empty draft under the current key and a draft exists under another version
key for the same mockup, prefill it and label it "draft carried over from
draft-1" (or offer a one-click "Restore draft from draft-1"); delete the
old key on save. Test: type, bump catalog, reload, assert the editor shows
the text and the saved note carries the new `design_version`.

### Minor

- **R-m1** `_load` initial-state creation (`server.py:443-448`) catches
  `OSError` only; `atomic_write` now raises `PostCommitError` (not an
  `OSError`) on a dir-fsync failure, so a first start on a flaky
  filesystem would traceback instead of warning. Catch `PostCommitError`
  there too (or make it an `OSError` subclass).
- **R-m2** Durability semantics after a post-rename failure are honest in
  the README (commit stands, warning logged, retried next save) but the
  client sees a plain 200 (`probe_r2.py` R2: keys `ok`, `feedback` only)
  and the README does not say that such a commit is visible to all readers
  yet not guaranteed to survive power loss until the next successful
  write. Suggest a `warnings` array in the 200 body (toast in UI) and that
  one sentence in the README durability rules. Not a correctness issue:
  `feedback.json` is the single source of truth and the next save
  re-fsyncs the directory.
- **R-m3** `_check_design_version_fresh` (`server.py:679`, `731`) runs
  before `store.mutate` takes the store lock, so a catalog bump between the
  check and the write could label one note with the previous version.
  Window is microseconds on a local tool; moving the check inside `op()`
  closes it for free.
- **R-m4** Shared `SHOT_DIR` in `test_browser_cdp.py:34`: any importer of
  `CDP` writes screenshots into the candidate's evidence directory (I moved
  my two out). Make it a constructor argument.

## Cleanup

Temp servers on ephemeral ports and `mkdtemp` dirs removed; Chrome
profiles disposable and removed; no `~/.local/share/silent-pool`; port
18481 not listening; no `server.py` processes left. Artifacts only under
`/tmp/opencode/fable-review/`.

---

# R3 verdict (2026-11-07, third pass)

Status: **APPROVE (review tool only).** The two R2 should-fix items (R3
status gating, R4 draft carry-over) and all R2 minors are fixed and
verified independently. Three cosmetic nits remain; none loses data,
mislabels a note, or blocks the review. No product-mockup approval is
implied; the design content review (GLM/DeepSeek, then Fable) is a separate
later task and `design/mockups/**` was not read.

Scope: 9 frozen files at canonical base `b0ab4ca34fda07c61a6335d05f598eaacc69678c`.
R3 snapshot `/tmp/opencode/lab-design-review-app-r3-review-snapshot.json`,
sha256 `68a4d8edd96c775669a9477095a727d3b8c6b80bcd91af80764d7926b0f1c267`;
input manifest sha256
`ad82e7cd9e635fd39b538dc0432ff981c253676b980ac35b1cdb9a5ce472802a`.
All 9 hashes verified before reading and after testing. No candidate
edits, commits, subagents; no `~/.local/share/silent-pool`; 18481 never
bound; no server processes left.

Evidence: `/tmp/opencode/fable-review/probe_r3.py` (sha256 `fee329f2...`,
26 checks) and `probe_toast.py` (`2fafe310...`); screenshots
`/tmp/opencode/fable-review/shots-r3/`. Candidate suites reproduced as
shipped: `test_review_app.py` **38 OK**, `test_browser_smoke.py` **2 OK**,
`test_browser_cdp.py` **6 OK** (per-run shot dirs
`/tmp/opencode/design-review-shots/cdp-<ts>-<pid>/`, R-m4 fixed).

Interpretation of my own R2 probe results, as requested: R1c was an
information marker from code reading (now fixed, see R-m1 below), and R5b
was a lazy-load timing artifact in my probe (`>= 5` cards matched before
the SVG thumbnail loaded); `probe_svg.py` confirmed SVGs render at 200 px
as thumbnails and in detail. Neither was a product defect. All true
failures from R2 (R3, R4) are addressed below.

## R2 items: status after R3

**R3 - status gating.** Fixed. `server.py:274` sets `stale_view: False`
on every freshly built view; only the last-good fallback paths set it
`True` (`server.py:643` rejected update, `:657` unreadable/partial).
`app.js:523` gates on `S.versionNotice || catalog.stale_view` only.
Verified (`probe_r3.py` R3a-R3g): a valid catalog with one broken entry
(`role: "admin"`, image missing) -> two banners, `stale_view` false, all
status buttons enabled, a status saves and persists
(`shots-r3/static-warnings-statuses-enabled.png`); partial write ->
`stale_view` true, buttons disabled; schema-2 replacement -> last-good +
`stale_view` true; catalog restored + Refresh -> enabled again.

**R4 - draft carry-over.** Fixed. `app.js:50-63` finds drafts for the same
target under other version keys (suffix match on `:<mockup>`; ids and
versions cannot contain `:`), `app.js:605-633` prefills only when the
current key is empty, labels origin and target, offers "Start fresh
instead"; `clearDraftsFor` (`app.js:41-49`) clears all keys for that
target only on a successful save. Verified end to end with the text typed
**once** (`probe_r3.py` R4a-R4m, `shots-r3/carryover-prefilled.png`,
`carryover-saved.png`): type at draft-1 -> bump -> Add -> 409, text kept,
nothing stored, banner + "catalog now: draft-2" badge -> reload -> O2
editor prefilled with the exact text, label "Prefilled with your unsaved
draft from draft-1 ... saving stores it under draft-2" -> decoy drafts for
O1 / U1 / overall under draft-1 are not picked for O2 and are prefilled
only on their own screens -> a failed save (injected `os.replace` failure,
500) keeps the editor text and both O2 keys, stores nothing -> explicit
Save stores exactly one note, `design_version: draft-2`, clears only the
O2 keys, removes the carry-over row; the previously audited O1 status
("like", set on draft-1) is still correct and shown STALE in the UI and in
the export line `### O1  (status: like, set on draft-1 -- STALE: catalog
is now draft-2)`. No silent resubmission anywhere (the 409 path never
retries; the success path is one explicit click). Candidate CDP `test_3`
now also types the sentinel once (`test_browser_cdp.py:277-279`).

**R-m1** initial-state `PostCommitError` -> handled (`server.py:452-458`);
verified: first start with an injected dir-fsync failure comes up, serves
200, `feedback.json` committed (Rm2). **R-m2** 200 bodies carry
`warnings` when a durability step failed (`server.py:1036-1039`), normal
saves carry none (Rm1b); README now states the commit is readable but not
power-safe until the next fully durable write and that retrying would
duplicate. **R-m3** freshness check moved inside `op()` under the store
lock (`server.py:709`, `:761`; lock order store -> catalog, no reverse
nesting, no deadlock). **R-m4** per-run `shot_dir`
(`test_browser_cdp.py:41-46`).

B1/B2/B3 and the Host/SVG/CSP hardening are unchanged from R2 and still
pass in the candidate suites.

## Remaining nits (not blocking)

- **N1** On the *note add* path the durability toast is overwritten in the
  same tick: `apiPost` toasts "Saved, with a durability warning: ..."
  (`app.js:99-102`) and the add handler then toasts "Note saved"
  (`app.js:596`). Observed via a MutationObserver (`probe_toast.py`):
  toasts `[warning, 'Note saved']`, final text "Note saved"; the status
  path shows the warning correctly (`shots-r3/durability-toast-status.png`).
  The note is committed exactly once either way. Fix: skip or append the
  success toast when `r.data.warnings` is present.
- **N2** The carry-over prefill dispatches `input` (`app.js:614`), which
  stores the text under the new version key immediately; on a *second*
  visit to that screen before saving, the text shows without the origin
  label (the label is shown only once). Acceptable since the user has seen
  it, but a `carried_from` marker in the draft entry would keep the label
  until save. Also "Start fresh" leaves an empty-string entry under the new
  key (`drafts["draft-2:U1"] = ""`), which is harmless (`!drafts[key]` and
  `v.trim()` treat it as absent).
- **N3** `store.last_warnings` is read by the handler after `mutate()`
  returns (`server.py:1036`), outside the store lock; under concurrent
  mutations a warning could be attributed to the wrong response. Single
  user, loopback; cosmetic. Return the warnings from `mutate()` instead.
- **N4** A mockup id literally named `overall` (allowed by `MOCKUP_ID_RE`)
  would share draft keys with the overall-notes editor. The designer's
  ids are O1-O4/M1-M3/D1-D3/U1-U4; reserve the word in the catalog
  validator if cheap.

## Cleanup

Ephemeral ports and `mkdtemp` dirs removed; Chrome profiles disposable;
no `~/.local/share/silent-pool`; 18481 not listening; no `server.py`
processes. Artifacts only under `/tmp/opencode/fable-review/`.
