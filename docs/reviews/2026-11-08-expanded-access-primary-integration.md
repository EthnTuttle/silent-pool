# EXP-ACCESS lab expanded-access foundation - primary integration (2026-11-08)

Owner: serial primary Rust/Cargo integration writer (sole writer for this
packet; build-priority reservation held by this session,
`ses_e458119eeffe4Jl3MZjJ4J4HJR`, for the Hub -> history-v2 -> access ->
operator-runtime -> ark-registration serial window).

Approval: `docs/reviews/2026-11-08-expanded-access-foundation-fable.md`,
**APPROVE bounded to code snapshot `8085616e...` / full snapshot
`b89f74d2...`** (Rework round): the expanded-access foundation - claim
files, Bearer authentication, scoped commands, principal-bound journal
hash scheme, filtered user projection, relay bootstrap - plus the R4
merge-handoff conditions (section "R4. Merge-handoff conditions") binding
this integration. Explicitly NOT approved and NOT done here: EXPAGENT
wallet/token-entry UI, AP-4 agent endpoint mapping, TOCTOU hardening,
legacy corrupt-log fixes, auth retrofit on the OP listener, future T1
command scopes.

External manifest: `/tmp/opencode/EXP-ACCESS-primary-integration-manifest.json`
(final sha256 below).

## 1. Pre-write verification (read-only, at transfer time)

- Candidate manifest `/tmp/opencode/EXP-ACCESS-review-manifest.json`
  reproduced: full `b89f74d2...` / code `8085616e...` recipes MATCH;
  candidate rows clean. Log: `EXP-ACCESS-prewrite-reverify.log`.
- Primary pre-state: the 6 new paths absent; the 3 byte-apply files
  byte-equal to baseline (fixture `b3baec02...`, command `62d05899...`,
  scenario `b1fec4af...`); roots `Cargo.lock` `55938583...`,
  `docs/SPEC.md` `9932149d...` (48 entries), `Cargo.toml`
  `179312b2...`, `flake.nix` `c9c477d1...`.
- Staged 3-way merges re-verified NON-STALE after the history-v2 landing
  (`.ours` == primary for engine/lib/http; history-v2 touched none of
  them): `EXP-ACCESS-3way/staleness-reverify-2026-11-08.log`
  (`6e666c37...`).
- Index `44f21a926d4d065ac6937fb0f81b698c7b2d0627` (== HEAD tree,
  nothing staged) before transfer.

## 2. Transfer (exact approved delta + R4 adaptation)

- **6 new files byte-exact** (100644; worktree 0664 modes normalized on
  copy, content verified against manifest rows): `src/access.rs`
  `9b7c820e...`, `tests/access_projection.rs` `dc9fd3b6...`,
  `tests/access_scope.rs` `c9c58e03...`, changelog fragment
  `5a06393a...`, decision record `70692d39...`, candidate report
  `35fc3270...`.
- **3 byte-apply files** (primary == baseline verified pre-copy, so
  candidate bytes apply as bytes): `bin/fixture.rs` `e9c021fe...`,
  `command.rs` `4596511f...`, `scenario.rs` `ed697277...`.
- **3 semantic merges from the staged 3-way results** (base = pre-OP
  baseline, ours = primary, theirs = candidate; ours verified == primary
  immediately before apply; applied as reviewed `a/...`->`b/...` patches,
  results byte-equal to the staged merges):
  - `engine.rs` -> `64ecabc9...` (clean merge, 11 hunks; the one removal
    is the candidate's scoped-journal `begin_scoped` refactor);
  - `lib.rs` -> `5d6a7770...` (clean, 1 hunk: `pub mod access;` joins
    `pub mod inventory;`);
  - `http.rs` union merge -> staged `b4f51638...` (one conflict at the
    route() arms, resolved by union; all Route variants from both sides
    kept), then the R4 hunks below -> final `7ffa90ed...`.
- **R4.1 (authored at merge)**: `("GET", Route::Inventory)` arm in the
  post-auth expanded match - operator-only via `engine.lab_inventory()`
  (the Hub `spc.lab.inventory.v1` DTO); user claim -> 403 ("inventory
  requires the operator claim") BEFORE any inventory build,
  serialization or file I/O; tokenless -> 401 at the existing Bearer
  gate. `_ => 404` unchanged (closed). No wildcard.
- **R4.2 (authored at merge, gated)**: `Route::Lab` / `LabJs` / `LabCss`
  joined the closed pre-auth static GET list ONLY after the byte gate
  passed: all 6 static assets byte-equal to the Hub-approved embedded set
  (lab.html `b452cd04...`, lab.js `3c6f7b54...`, lab.css `ccb82914...`,
  index.html `0e4335cb...`, app.js `1f0b6a86...`, style.css
  `615f2643...`) and scanned for inline private facts - the only pattern
  hits are benign code identifiers (a URL-parser credential-rejection
  guard; DOM focus-accessibility helpers), no private value, status,
  address or token. Log: `EXP-ACCESS-lab-asset-bytecheck.log`.
- **R4.3**: no token entry UI, no JS auth change (EXPAGENT future).
- **R4.4 (authored at merge)**: new integration test file
  `crates/spc-lab/tests/access_hub_routes.rs` (merge-owner artifact,
  final sha256 `aa8a79da...`, 3 tests): user 403 on inventory, tokenless
  401, operator 200 + DTO schema, legacy inventory 200 unchanged, and
  the public lab assets served pre-auth byte-equal to the embedded
  source with no token/identity leak.
- **R4.5**: decision + changelog spliced; SPEC changelog 48 -> 49
  (`9932149d...` -> `5aca112c...`), all prior bullets preserved,
  reverse-splice reproduces the pre-state. The spliced bullet carries
  the candidate fragment byte-exact plus one 398-byte merge-handoff
  sentence recording the R4 hub-route classification (the normative
  handoff-semantics note required by the review).
- **R4.2 (lock)**: primary `Cargo.lock` wins - unchanged `55938583...`
  (the access delta contains no dependency). `Cargo.toml`, `flake.nix`
  unchanged; no new vendor hash needed for this packet.
- **R4.6 verified**: `check_traffic_capability` (engine.rs line 167)
  still precedes the `access_bindings` check (line 176) in `prepare`.
- Status 206 -> 209 (6 new + 3 modified rows net of the byte-apply
  modifications + new test file); index `44f21a92...` unchanged.

## 3. Validation (actual runs, pinned devshell, flock + >=50 GiB policy)

Runner `/tmp/opencode/EXP-ACCESS-validation.sh` (sha256
`06d9a18794bdaa5b9ebd901922537055e514f9c6831af0736a46be89f52b4cc8`),
under `nix develop` (cargo 1.93.0), session-bound ticket guard before and
inside the flock, disk 71-74 GiB free, existing primary `./target`.

- **Access suites (candidate)**: `--test access_scope --test
  access_projection` - exit 0, **24 passed, 0 failed** (18 + 6).
- **R4 hub-route tests (merge-owner)**: `--test access_hub_routes` -
  exit 0, **3 passed, 0 failed** (final r3 run).
- **Legacy suite** (command_concurrency 1 + command_idempotency 6 +
  engine_restart 3 + http_protocol 7 + http_sse 3 + http_subscriber_lag
  1 + scenario_v1 8 + snapshot_resume 4): exit 0, **33 passed,
  0 failed**.
- **Lib unit tests**: exit 0, **42 passed, 0 failed**.
- **lab_hub regression** (Hub behavior preserved through the merge):
  exit 0, **6 passed, 0 failed**.
- **fmt**: exit 0. **clippy** (`--all-targets -D warnings`): exit 0.
- **Final aggregate**: SUCCESS, all 7 steps exit 0
  (`aggregate-final.log`).

### Exceptions (merge-owner test-authoring defects, documented; product code unaffected)

1. **Wrong inventory path (r1)**: my R4 test file requested top-level
   `/inventory`; the route is run-scoped
   (`/api/lab/v1/runs/{run_id}/inventory`), so the requests hit the
   closed `Route::Artifact` fallback and got 403 "artifact path is not
   shareable" - the fail-closed catch-all working as designed. Fixed the
   test paths; r2 green.
2. **Missing engine cleanup (r1/r2)**: my tests did not call
   `engine.stop()`/`server.shutdown()` (the candidate's tests do), so
   12 `spc-lab-fixture` children leaked, inherited the build-lock fd and
   blocked the next lock acquisition (one silent BLOCKED attempt,
   reported once). I killed exactly my own leaked fixtures (the
   remaining `spc-lab-fixture` process belongs to another session's
   `lab-pool-operator` worktree and was not touched), added the
   candidate's cleanup pattern to all three tests, and verified the r3
   run leaks zero fixtures.
3. fmt failure in r1 was my test file's formatting (collapsed format!
   line, assert! macro layout, one double blank line) - fixed; r2/r3
   green.

No candidate-side defect was found; all candidate suites were green on
the first merged-tree run.

## 4. Final state

- Index `44f21a926d4d065ac6937fb0f81b698c7b2d0627` (== HEAD tree;
  nothing staged, no commits made) - unchanged across the whole packet.
- Status 209 lines at receipt time.
- Roots final: `Cargo.toml` `179312b2...`, `Cargo.lock` `55938583...`
  (both unchanged), `docs/SPEC.md` `5aca112c...` (49 entries),
  `flake.nix` `c9c477d1...` (unchanged; no new vendor hash this packet).
- Queue after this receipt: operator-runtime wiring integration (3-way
  merges precomputed against this post-access state:
  `EXP-OP-3way-precompute-classify-r2.log` - engine.rs 3-way; lib.rs and
  http.rs SKIP (candidate == Hub-as-landed, ours keeps ACCESS content);
  runtime.rs, resource.rs, main.rs, inventory.rs, lab.js,
  lab_helpers.test.cjs, common/mod.rs byte-apply), then AP-2 ark
  registration. J1/history-3 remains unapproved and unmerged.

## 5. Not claimed here

- No EXPAGENT wallet, no token-entry UI, no AP-4 agent relay mapping, no
  TOCTOU hardening, no legacy corrupt-log fix, no OP-listener auth
  retrofit, no Ark claim, no combined Nix gate (deferred per batching
  instruction), no fresh mining.
