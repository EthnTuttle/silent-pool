# Live lab hub: Fable first review

Date: 2026-11-08
Reviewer: Claude Fable 5.1 (`amazon-bedrock/global.anthropic.claude-fable-5-1`),
subagent session; read-only on the candidate, sole write is this file.
Candidate: `/home/ethan/.local/share/opencode/worktree/ad8886/lab-live-hub`,
uncommitted, base `b0ab4ca34fda07c61a6335d05f598eaacc69678c`.
Plan record: `docs/reviews/2026-11-07-lab-hub-plan-reviews.md` (primary).
Candidate report: `docs/reviews/2026-11-08-live-lab-hub-implementation.md`
(in the worktree, SHA-256 `1c07603b...46bac9`).
Authority manifest: `/tmp/opencode/LP-hub-verified-review-manifest.json`
(file SHA-256 `355163c3b37b57d64c26a7a65047c6a56d981d8d4774ad1a88306bc424d24a7d`),
378 rows, full snapshot
`3896b132e8accda44fa4aa8efd7eccbfe37c07d0be4fb3c397e733f5b4034168`, code
snapshot (report excluded)
`712fa8cba9326b65a91b889b1453484dca9f0d89c8ce2504786bc6653914f606`.
Approved runtime baseline: `/tmp/opencode/LP-final-runtime-manifest.json`,
370 rows, code snapshot `30fedafd...`, frozen source
`/tmp/opencode/LP-final-runtime-src-v4`.
Final binary: `/tmp/opencode/LP-hub-verify-target/debug/spc-lab`, SHA-256
`a577aea74e28a2eba64e517416e8baf733e068ba7c0b5a7aeb9e44aa235e5630`
(immutable copy taken before my cargo runs:
`/tmp/opencode/LP-hub-fable-pinned-spc-lab-a577`, same hash).
Browser receipt: `/tmp/opencode/LP-hub-sealed-browser-receipt.json`, SHA-256
`555e81f486212bcc3243b35fb3cd6e724faa55a486a08a67833ea9e9e407e6ed`.

## Result

**APPROVE, bounded to code snapshot `712fa8cb` and the exact scope below**,
with two conditions the merge owner must apply in the serial integration
(section 3, R1 and R2). Neither condition touches an embedded asset or any
Rust source, so the binary/browser proof is not reopened by them. R3 to R8 are
recommendations and evidence-reading notes; none blocks.

Scope approved: the small read-only `/lab` directory (`/lab`, `/lab.js`,
`/lab.css`) and `GET /api/lab/v1/runs/<id>/inventory` on the existing loopback
`HttpServer`, the `/` header link and recorded-mode pill/banner, the
`inventory.rs` projection, `docs/LAB-ROLES.md`, and the LAB-RUNTIME.md /
README.md / ARCHITECTURE.md text deltas as corrected by R1. Not approved and
not claimed by this packet: any operator/agent UI, Ark end-to-end, wallet
functionality, a new auth platform, live operator integration, or a fresh
native live experiment on the fixed binary.

Why approve: I re-hashed every manifest row and receipt, rebuilt the
provenance chain from source mtimes to the final binary, found all six
static assets embedded byte-identical in the recorded binary myself, re-ran
the Node and Rust suites plus fmt/clippy on the frozen source, and read the
whole 17-file delta. The projection is allowlisted by construction, both URL
guards close the bypasses that matter, the hub issues no POST, recorded mode
is explicit at the API (`view`) and in both pages, and the capability text
says "not running in this experiment / planned required integration" rather
than "optional" or "implemented". The one defect that survives is a single
documentation sentence (R1) in a file the merge owner has to hand-merge
anyway.

## 1. Provenance and byte checks (independent, not inherited)

- All 378 manifest rows re-hashed in the worktree: 0 mismatches, 0 missing,
  modes agree with the manifest's git-normalized `100644` (the eight on-disk
  `0664` files are umask only; not secrets, not a gate). `git ls-files
  --cached --others --exclude-standard` yields exactly the 378 paths; no
  ignored or stray files. Canonical `SHA256(compact sorted JSON {base,files})`
  reproduces `3896b132...` and, with the report row removed,
  `712fa8cb...`. The manifest contains no self-hash; its file hash
  `355163c3...` matches the validation record.
- All 33 receipt rows re-hashed with byte counts: 0 mismatches.
- Candidate = approved 370-file baseline + 17-file delta (10 modified, 7 new)
  + the report. No baseline file removed; no candidate file outside the 17
  differs from baseline except the report. No `Cargo.toml`, `Cargo.lock`,
  `flake.nix`, pin or `config/` change.
- Primary alignment (hashes today): `bins/spc-lab/src/main.rs`,
  `static/app.js`, `static/index.html`, `crates/spc-lab/src/http.rs`,
  `tests/http_sse.rs` and `docs/ARCHITECTURE.md` are byte-equal to baseline
  on primary, so those hunks apply cleanly. `README.md`,
  `crates/spc-lab/src/engine.rs`, `crates/spc-lab/src/lib.rs` and
  `docs/LAB-RUNTIME.md` carry primary-side T0/AP-3a/AP-1a/T4a content beyond
  baseline and need the serial semantic merge the report describes; the
  candidate's `engine.rs` hunk is three additive methods and one import, so
  the T0 `prepare`/adopt gates must be kept verbatim. The seven new files are
  absent on primary; the recovery bytes (not the pre-recovery hashes in the
  17-row table) are the integration input.
- Binary binding, independent of the browser receipt: the `a577...` binary
  contains `lab.html`, `lab.js`, `lab.css`, `index.html`, `app.js` and
  `style.css` each exactly once and byte-identical to the frozen source.
- Timeline of the "already captured" evidence holds: `inventory.rs` mtime
  12:42Z and `lab_hub.rs` 13:12Z precede the Rust run at 13:16Z
  (`LP-hub-recovery-rust-build.log`, explicit `test_exit: 0`, 29 lib + 3
  `http_sse` + 6 `lab_hub`); `lab.js` 13:27Z and `lab_helpers.test.cjs`
  13:28Z precede the Node run at 13:28:59Z (`exit: 0`, 24/24). The final
  build at 14:20Z compiled only `spc-lab-cli` (the lib fingerprint was fresh,
  confirming no Rust source changed after its test), and the browser gate ran
  14:21-14:23Z against that binary.
- My reruns on the frozen source, under nonblocking
  `/tmp/opencode/silent-pool-heavy-build.lock`, in the existing
  `LP-hub-verify-target` (86 GiB free before and after; binary hash unchanged
  afterwards): `node --test bins/spc-lab/tests/*.test.cjs` 24/24;
  `cargo test -p spc-lab --lib --test lab_hub --test http_sse --locked`
  29 + 3 + 6 passed (`/tmp/opencode/LP-hub-fable-rust-rerun.log`);
  `cargo fmt --all -- --check` clean; `cargo clippy -p spc-lab -p spc-lab-cli
  --all-targets --locked -- -D warnings` clean
  (`/tmp/opencode/LP-hub-fable-fmt-clippy.log`). The pre-recovery Nix
  fmt/clippy results (`LP-hub-check-result*`) predate the recovery edits; my
  targeted rerun covers the edited crates, and I do not ask for a full Nix
  rerun for a frontend-plus-one-label delta.
- The old 57/84 and 80/84 receipts, the blocked `292d3b8f` manifest and the
  `LP-hub-recovery-final-binary-build.log` BLOCKED record are preserved and
  correctly described as history, not proof. The 120 s live host proof
  (`LP-hub-live-*`) is bound to binary `fc17e002...` and source `1a1c7466...`;
  see section 2.7 for what it still proves.

## 2. Functional review

### 2.1 Projection and allowlisting (`crates/spc-lab/src/inventory.rs`)

- `project()` takes three short snapshots (`manifest_snapshot`,
  `process_records`, `evidence().events()`) and holds no lock across the
  walk. Every `note`, `source`, `availability`, `freshness`, `purpose`
  (group) and `kind` is `&'static str`; the only dynamic strings are scenario
  identifiers (`node.id`, `front.id`, `pool.id`, resource ids, miner ids and
  implementations), `run_id` and `lifecycle`. No manifest, process record,
  argv, env, artifact path, endpoint credential, descriptor or identity field
  is serialized. `manifest_resources` returns ids only.
- Evidence `Unknown` detail is parsed with `serde_json` and used only as a
  lookup key (`node`, `ping`, `observation`) or a prefix match on
  `event.source`; a malicious or nested detail string is never forwarded.
  `inventory_is_allowlisted_and_observation_age_comes_from_evidence` plants
  Redis/Postgres passwords, an env marker, a secret detail and a private path
  and asserts the lowercase encoding contains none of 23 markers (including
  `sp_id`, `b_scan`, `b_spend`, `payout_descriptor`, `tr(tpub`, `cookie`,
  `password`, `/tmp/`, `/home/`, `redis://`, `postgres://`). I also scanned
  the real recorded and live inventories from the stopped run: only the four
  loopback `/api/pool` and `/metrics` URLs, no 64-hex, path, `tpub` or `sp1`
  token.
- Roles vs processes: one `spc-pool` row per front with `logical_roles`
  `["front","payout","api"]`, no invented payout/api daemons; scanner is a
  row with `recorded_state = embedded_no_standalone_process` and an
  explicitly absent daemon; miners are `spc-cpu-miner`, labelled "not
  spc-agent". Counts: declared from scenario + manifest resources, recorded
  from (role, entity) pairs, live from `Running|Adopted|AdoptedControlLost`.
  The real run shows `9 declared / 9 recorded / 9 live` running and `0 live`
  stopped, which is correct.
- Health is never inferred from HTTP 200 or a process record: each row's
  `observation` comes only from collected evidence (`*_when_observed`,
  `evidence_received`, `collector_observed`) with its real `observed_at_ms`,
  `age_ms`, `freshness` (`recent` within 15 s, `older`, or `recorded`) and a
  collector `source`; absence is `unknown` with `source = "no collected
  availability observation"`. The controller row's `serving_this_response`
  is the one honest self-observation. `recorded_state` is the supervisor or
  manifest record, falling back to `not_started`/`not_recorded`.
- Interface kinds are accurate: `ui` only for `/` and `/lab`; pool API is
  `api_json` and `metrics`, never `ui`; node is `authenticated_json_rpc` with
  `url: null`; Redis/Postgres are `native_protocol`, `url: null`; miner is
  `jsonl_control`; scanner `cli_library`. `browser_url` emits a URL only for
  a loopback `IpAddr` host, never under `--topology`, and only for the two
  known routes.
- Capability gap: `spc-agent` is `not_running` (or `declared_in_scenario`),
  `required_next_profile: false`; independent Ark server, per-pool Ark
  delivery/float and SPc user wallets are `not_running_planned_required`
  with `required_next_profile: true`. The recovery label reads "The standard
  scenario launches no agent wallet and wires no live registration or
  per-user wallet UI" and the test asserts the blanket "wallet remains
  unimplemented" sentence is gone. That is as-of-this-run semantics, correct
  against the current primary (AP-3a foundation present, no wallet launched
  by the standard scenario).

### 2.2 Routes and server (`http.rs`, `main.rs`)

- Three static routes (`/lab`, `/lab/`, `/lab/index.html`; `/lab.js`;
  `/lab.css`) and `Route::Inventory` scoped to the engine's own run id
  prefix; any non-GET falls to the existing 404 arm. No new listener, no
  proxy, no new command path. `WebAssets` gains three fields with
  API-only defaults; `http_sse.rs` is updated for the struct only.
- `get_hub_and_inventory_do_not_mutate_run_or_commands` asserts command log
  length, evidence cursor and manifest `state`/`updated_at`/process count are
  unchanged across GETs of all four routes.
- Read-only serve: `attach_read_only` (baseline) sets `read_only = !adopt`;
  `RunEngine::submit` rejects with `RejectedPrecondition` before any effect
  when `read_only` (baseline `engine.rs` ~1515). So the server, not just the
  UI, refuses control in recorded mode. See R4 for the missing direct test.

### 2.3 Frontend (`lab.html`, `lab.js`, `lab.css`)

- Every dynamic value passes `escapeHtml` before `innerHTML`, including
  `data-key` ids, notes, roles, dependencies and URLs; run id, lifecycle,
  topology, counts and banner use `textContent`. No `eval`, no inline
  handlers, no form/input/button; `fetch` is GET-only (`/api/lab/v1/runs`,
  `.../inventory`), and the test file asserts `/commands` and
  `method: "POST"` are absent from the source.
- `safeActualUrl`: `/` and `/lab` pass; otherwise `new URL` must parse,
  protocol `http:`, hostname `127.0.0.1` or `[::1]`, pathname exactly
  `/api/pool` or `/metrics`, and empty username/password/search/hash. This
  closes credential-in-authority, query/fragment smuggling, scheme tricks and
  `127.0.0.1@evil` forms; path traversal is normalized by the parser before
  the exact match. Both guards (Rust emit, JS accept) are present as the plan
  required.
- Recorded mode: `view === "recorded_read_only"` drives the banner, the
  `.primary-link`/footer text ("Open recorded monitor (read-only)" /
  "Advanced recorded monitor (read-only)"), the controller-row link relabel,
  and the suppression of historical external `/api/pool` and `/metrics`
  anchors (rendered as labels with an explicit "no current endpoint or
  reachability is implied" note). Local `/` and `/lab` remain links. Live
  mode keeps "Open live controls".
- Poll/race: one `loading` flag prevents overlap; the 2 s `setInterval`
  skips while in flight rather than queueing, so there is no pressure loop.
  The open-state `Set` is read after the final `await` and immediately before
  node replacement, so an in-flight response cannot undo a newer toggle.
  Focus is captured as a keyed token (summary or nth link inside a
  `details[data-key]`) and restored with `preventScroll`. Row open flags are
  now applied in `groupMarkup` via the `open` set (the 80/84 defect), covered
  by `row disclosure state is carried into replacement group markup`.
- Mobile intrinsics: `.groups { grid-template-columns: minmax(0, 1fr) }`,
  `min-width: 0` on `.group/.integration/.row-detail`, `.table-wrap
  { overflow-x: auto; max-width: 100% }` with the table keeping
  `min-width: 850px`. The table scrolls locally; the page does not. Receipt:
  390/390 at 390 x 844, 1425/1425 at 1440 x 900.
- The existing `/` page changes are limited to a header link, a `view-mode`
  pill, a hidden `recorded-banner`, and `applyInventoryMode`, which only
  sets the already-existing `state.readOnly` and calls the existing
  `disableControls()` proactively instead of waiting for a rejected command.
  Behaviour of the controls themselves is unchanged.

### 2.4 Tests

- Node 24/24 (14 pre-existing `app_helpers`, 10 `lab_helpers`): URL
  allow/deny list including `javascript:`, `https:`, credentials, query,
  sub-path, non-loopback and `/proxy?url=`; recorded relabel and stale-link
  suppression; row-open carry; integration status strings never say running;
  unknown vs recorded observation text; escaping; static page has no controls
  and the JS has no command path.
- Rust 6 `lab_hub` tests: one- and two-pool co-location and counts (6 / 10
  declared), scanner embedded row, agent/Ark/wallet statuses and the scoped
  wallet label; allowlist/age/freshness with planted secrets; namespaced
  `topology` with null API/metrics URLs and no `127.0.0.1:` anywhere; host
  API/metrics typed `api_json`/`metrics` and never `ui`, node/Redis/Postgres
  URLs null; GET non-mutation; live `running` with >= 5 live actors, then
  `stopped` with 0, then `attach_read_only` gives `view =
  recorded_read_only`, `freshness = recorded`, `/lab` and `/lab.js` served,
  `/` contains `recorded-banner` and `href="/lab"`.

### 2.5 Browser proof (recorded mode, final binary)

- 42 checks x 2 viewports = 84/84 on `a577...`, Chrome 156, Node 24.11.1,
  fresh owned read-only serve of the stopped `LP-hub-run-1` on an ephemeral
  port with fresh profiles. Checks include: recorded banner and text;
  lifecycle/run id; 5 groups, 11 row disclosures, 6 integrations; zero
  buttons/forms/inputs; no html/body overflow; viewport applied; 7 distinct
  Tab stops; for each of group/row/integration: first native key (Enter or
  Space, with `text`/`unmodifiedText`) toggles, state and summary focus
  survive two counted inventory responses, second key toggles without
  refocus, state and focus survive two more; primary link label/href;
  zero external anchors; real mouse activation of `/` lands on the recorded
  monitor (`recorded / read-only`, 13 controls, 0 enabled); all requests GET;
  no exceptions, console errors, error log entries, load failures or >= 400.
- Served `/lab`, `/lab.js`, `/lab.css` bytes equal the frozen source; the
  2780-file run dir, journal and evidence hashes are unchanged before/after;
  serve exited 0; no owned Chrome or serve process remained.

### 2.6 Docs

- `LAB-ROLES.md` is a correct source-cited role/process/interface map with an
  explicit boundary from the historical `ARCHITECTURE.md`, a primary-
  integration note that names T4a/AP-1a/AP-3a/T0 as present on primary and
  absent from this candidate, and an explicit instruction not to carry a
  blanket "wallet unimplemented" claim into the merge.
- `ARCHITECTURE.md` banner: marks the file as a historical target map and
  corrects "the pool never speaks SV1". SPEC §1 (2026-11-02 native SV1) is
  the authority and already says the opposite, so this is a lower-document
  fix under the AGENTS.md precedence rule, not a normative change.
- `LAB-RUNTIME.md` and `README.md`: accurate about `/`, `/lab`, `serve`,
  recorded/read-only and the inventory route, except for the sentence in R1.

### 2.7 What is and is not proved

- Proved on the final binary: recorded-mode `/lab` and `/` behaviour in a
  real browser at both viewports; GET-only; non-mutation of a frozen run;
  byte-identical embedded assets.
- Proved by tests and code reading, not by a fresh browser run: live-mode
  rendering (links to `/api/pool` and `/metrics`, "Open live controls"),
  namespaced topology (Rust test only; namespaced URLs are blocked, and no
  namespace kernel UI run was attempted, which is valid bounded scope).
- Reusable from the old `fc17...`/`1a1c...` live proof: two co-located pool
  fronts, CPU miner ownership, recorded vs observed state, allowlisted
  endpoints, GET-only behaviour and an unchanged command journal, because the
  only backend hunk since is one label string and its test. The frontend
  bytes differ, so that proof says nothing about the fixed JS/CSS; the report
  states this correctly and does not claim a fresh native experiment.

## 3. Findings, ranked

**R1 (required at merge, documentation).** The candidate's
`docs/LAB-RUNTIME.md` out-of-scope bullet still says
"`spc-agent --roles wallet` and desktop/mobile wallet UIs are not
implemented." That is a global claim, and it is false against the current
primary: `bins/spc-agent/src/main.rs` on primary accepts `--roles wallet`
(AP-3a foundation, behind `--features ark`; a non-ark build bails with "this
build has no wallet role"). The recovery removed this blanket wording from
`inventory.rs` and `LAB-ROLES.md` but not here, so the report's section 3.3
claim is incomplete by one sentence. `LAB-RUNTIME.md` is already a semantic-
merge file (primary has T0 content). Apply this in the merge hunk: replace
the sentence with "The standard scenario launches no `spc-agent` wallet role
and wires no wallet desktop/mobile UI; backend wallet foundation on primary is
described in `LAB-ROLES.md` section 3 and is not launched by this run."
Landing the candidate sentence verbatim would be a defect of exactly the kind
this packet exists to prevent, and this approval does not cover it.

**R2 (required at merge, process).** No `docs/decisions/` record and no
`docs/changelog/` fragment ship with the packet. Repo precedent for lab-tool
surface changes is to record them: `2026-11-06-lab-runtime-host.md` records
`/catalog`; `2026-11-08-lab-traffic-contract-t0.md` records "lab-tool
vocabulary only" with a fragment; the EXP-OP read-only page carries both.
This packet adds a stable public lab API (`spc.lab.inventory.v1`,
`GET /api/lab/v1/runs/<id>/inventory`), three static routes, recorded-mode UI
semantics and the ARCHITECTURE.md banner. The merge owner should author
`docs/decisions/2026-11-08-lab-live-hub.md` (lab-tool scope, no SPEC body
change, no payout/privacy/Ark semantics) and a one-line fragment; the merge
session splices it. Per `MERGE-QUEUE.md` the candidate correctly did not edit
the SPEC frontmatter itself.

**R3 (recommended, next hub packet).** The Rust projection labels the
controller-row interface "Open live controls" / "Existing advanced monitor
and miner controls." regardless of `view`; only the JS relabels it in recorded
mode. API consumers of a recorded serve therefore see a live-controls label.
Make `controller_group` view-aware (one `if recorded`) and assert the label
in `refresh_uses_new_process_state_and_recorded_serve_is_explicit`. Rust-only
change; it does not reopen the browser proof.

**R4 (recommended).** The server-side rejection of control in recorded mode
rests on the baseline `submit` guard, which no test exercises directly. Add a
`lab_hub` (or `engine_restart`) test: `POST /api/lab/v1/runs/<id>/commands`
on an `attach_read_only` `HttpServer` returns `rejected_precondition` with
the read-only detail and leaves the command log and manifest bytes unchanged.
Cheap, Rust-only, and it is the claim the hub's banner makes.

**R5 (nit, docs).** `LAB-ROLES.md` section 1 item 3: "No observation is shown
as unknown." reads inverted. Intended: "A missing observation is shown as
unknown." Fold into the R1 merge pass if convenient.

**R6 (nit, frontend, defer).** `refresh()` has no fetch timeout. A hung
loopback fetch leaves `loading = true` forever and the page silently freezes
on stale content with no error banner. `AbortSignal.timeout(5000)` on both
fetches fixes it, but it changes an embedded asset and reopens the browser
proof, so it belongs in the next frontend packet, not this seal.

**R7 (nit, Rust).** `browser_url` would format an IPv6 loopback as
`http://::1:<port>/...` (unbracketed). The JS guard rejects it (fail closed,
no link), and `Endpoint::loopback` is `127.0.0.1` today, so there is no live
path; bracket or reject `Ipv6Addr` when touching this helper.

**R8 (evidence reading, no action).** Both passing sessions in the final
receipt carry `chrome_killed_after_failure: true`. In
`LP-hub-browser-recovery.js` the `finally` block sets that flag whenever
`chromeClosed` is not yet true after `Browser.close`, then SIGKILLs; it is a
harness naming artifact, not a failure. `chrome_exit: "exited"` and the
cleanup scans (no owned Chrome or serve process) are the facts that matter.

## 4. Conditions and hand-off

Approved for serial primary integration of code snapshot `712fa8cb` provided:

1. R1 is applied in the `docs/LAB-RUNTIME.md` merge hunk (byte-check the
   replacement sentence; the rest of the hunk is approved as written).
2. R2 is satisfied by a decision record and changelog fragment authored at
   integration, lab-tool scope.
3. The `engine.rs`, `lib.rs`, `README.md` and `LAB-RUNTIME.md` merges keep
   primary's T0 gate code and T0/AP-3a/AP-1a/T4a text intact; the six
   baseline-equal files and seven new files apply as bytes.
4. No candidate `Cargo.*`/`flake.nix` state is carried; none exists.
5. After the merge the integration owner runs its usual gate; no new live
   mining run is required by this review, and this review claims none.

R3 and R4 are the first two items for the next hub packet, together with R5
to R7. Nothing here gates on Ark, dense mockups, a new auth platform or live
operator integration; the EXP-OP page and any future operator URL in this
hub's interface list are separate, later work.
