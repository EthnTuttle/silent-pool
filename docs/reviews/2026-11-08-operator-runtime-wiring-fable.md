# EXP-OP host operator runtime wiring: Fable review

Date: 2026-11-08
Reviewer: Claude Fable 5.1 (`amazon-bedrock/global.anthropic.claude-fable-5-1`),
subagent session (same reviewer as the EXP-OP backend reviews in
`2026-11-08-pool-operator-fable.md`).
Packet: host operator runtime wiring (`--operator-ui`, port lease, rendered
`[operator]` section, route-readiness proof, hub typed interface),
uncommitted in
`/home/ethan/.local/share/opencode/worktree/ad8886/lab-operator-runtime-wiring`,
base `b0ab4ca34fda07c61a6335d05f598eaacc69678c`.
Authority manifest: `/tmp/opencode/EXP-OP-runtime-freeze-manifest.json`,
604 rows, raw on-disk modes (`0o644`/`0o664`), full snapshot
`fd198c4496931028fc52903201a13220bd8f5a363fb9de445aa09ed824e2137f`, code
snapshot (report excluded)
`a9b6e4fc64b468b0e6d754930749c839412bb2f3b79b50e6c35fba6cd3bcd6d2`.
Baselines layered: approved EXP-OP (`d3710c0c...` / `365e0bf2...`,
`/tmp/opencode/EXP-OP-rework-parent-manifest.json`) and approved lab-live-hub
(`3896b132...` / `712fa8cb...`,
`/tmp/opencode/LP-hub-verified-review-manifest.json`, source root
`lab-live-hub`).
Sealed CLI binary:
`/tmp/opencode/EXP-OP-runtime-spc-lab-bin-7af444a983d0e913e026d810e2f46a5184e219b8eecbe0d8e11c3d6b27dfe156`.
Candidate report: `docs/reviews/2026-11-08-operator-runtime-wiring.md` (in
the worktree). Contract: `/tmp/opencode/role-ui-plan-review-v3/` section 4
(OP-1 serving scope) plus the EXP-OP decision record.

## Result

**APPROVE**, bounded to code snapshot `a9b6e4fc...` and sealed binary
`7af444a9...`, with three documentation merge conditions (section 4, D1-D3)
and no code change required.

The new wiring hunks do what the report says and only that: one off-by-
default host flag; `--topology` rejected both by clap and as the first
statement of `Runtime::prepare`; the per-front api-role requirement rejected
before the run directory exists; one extra `front.<id>.operator` loopback
listener leased by the existing port machinery; the pool TOML body extracted
into a struct whose opt-out rendering is byte-identical to the pre-EXP-OP
template and whose opt-in rendering appends exactly the approved
`[operator]` section, validated in tests by the real `spc_pool::config::
Config::load`; a bounded start-time HTTP 200 route proof recorded as an
evidence fact; and a hub interface that is typed `operator_ui`, gated on the
planned listener plus the recorded `page_served: true`, and linkable in
`lab.js` only as the exact loopback root. The approved OP backend, its
assets and SQL are untouched by hash; the approved hub layer is consumed at
its reviewed hashes with three narrow semantic merges that I diffed.

This review was conducted as a source and receipt review only: the heavy-
build priority ticket (`/tmp/opencode/silent-pool-build-priority.json`) is
held by the AP-2 session, so no cargo, node or Nix command was run and no
lock was attempted. The receipts are internally consistent with the source,
the sealed binary embeds every static asset byte-exact, and the only
remaining gaps are documentation claims; none warrants a rebuild. The fresh
manual host run after merge remains the gate for any runtime claim.

## 1. Provenance and inventory

- Manifest recipe reproduced: `SHA256(compact sorted JSON {base,files})`
  over the 604 rows with raw modes gives `fd198c44...`; excluding only the
  candidate report gives `a9b6e4fc...`. All 604 rows re-hashed against the
  worktree, modes compared as raw `st_mode & 0o777` (0 mismatches),
  inventory equal to `git ls-files --cached --others --exclude-standard`
  (0 extra, 0 missing), 0 files removed vs the OP baseline.
- Delta vs the OP baseline: 29 rows, 17 modified + 12 new, as declared.
  Classified three ways against the hub manifest (whose 378-row source root
  `lab-live-hub` still matches its manifest, 378/378):
  - hub-verbatim (byte-equal to the approved hub): `app.js`, `index.html`,
    `lab.css`, `lab.html`, `http.rs`, `http_sse.rs`, `lab_hub.rs`,
    `ARCHITECTURE.md`, `LAB-ROLES.md`;
  - hub + wiring (hub base plus this packet's hunks): `main.rs`, `lab.js`,
    `lab_helpers.test.cjs`, `inventory.rs`, `engine.rs`, `lib.rs`,
    `README.md`;
  - wiring only (vs OP): `Cargo.toml`, `Cargo.lock`,
    `crates/spc-lab/Cargo.toml`, `resource.rs`, `runtime.rs`,
    `tests/common/mod.rs`, `traffic_contract.rs`, `LAB-RUNTIME.md`, the two
    new test files, decision, changelog fragment, report.
  The `lib.rs`/`engine.rs` diffs against the hub show only T0/traffic
  lineage already present in the OP baseline plus the wiring hunks; against
  the OP baseline they show exactly the hub's `pub mod inventory;`,
  `ProcessRecord` import and three projection methods plus the wiring
  hunks. No wholesale file transfer; the hub's `LAB-RUNTIME.md` was not
  imported (the OP/T0 version is kept), and the hub Fable R1 sentence
  ("`spc-agent --roles wallet` ... are not implemented") does not appear in
  the candidate's `LAB-RUNTIME.md` (grep clean).
- Dependency edges: root `Cargo.toml` adds one `[workspace.dependencies]`
  line `spc-pool = { path = "bins/spc-pool" }`; `crates/spc-lab/Cargo.toml`
  adds one `[dev-dependencies]` table with `spc-pool.workspace = true`.
  `Cargo.lock` delta vs OP is exactly one line, `"spc-pool",` - inside the
  `[[package]] name = "spc-lab"` dependency list (line 4112, between
  `spc-crypto` and `spc-scan`), which is the correct place for a
  dev-dependency of `crates/spc-lab`. The report's text says "spc-lab-cli
  dependency list"; that is a report wording error, not a manifest error
  (D3). No version pin moved; no cycle (`bins/spc-pool`, `spc-store`,
  `spc-accounting`, `spc-crypto` have no `spc-lab` dependency). Dev-dep
  only: the shipped `spc-lab` CLI does not link `spc-pool`.
- Sealed binary `7af444a9...` (105,772,016 bytes, mode 500): contains
  `bins/spc-lab/static/{lab.html, lab.js, lab.css, index.html, app.js}`
  byte-exact, each exactly once (not just the `operator_ui` substring
  count the report cites); contains the `--operator-ui` help text, the
  `host-runtime only` and `has no api role` rejection strings and the
  "Pool operator page" label once. The approved hub binary
  (`/tmp/opencode/LP-hub-verify-target/debug/spc-lab`) contains the hub's
  `lab.js` and not this packet's, so the old 84/84 hub browser proof is
  truthfully scoped to the hub source, not to this binary.
- Receipts (`/tmp/opencode/EXP-OP-runtime-*.log`): `cargo test -p spc-lab`
  175 passed / 0 failed / 16 ignored across 34 suites, including the 10
  `runtime::tests::*operator*` unit tests inside the 46-test lib suite and
  the 5-test `operator_ui_inventory` suite; `cargo test -p spc-lab-cli`
  3/3 (`cli_operator_ui`); clippy log shows no code warnings (only the
  pre-existing `sqlx-postgres` future-compat notice and cargo cache
  permission notices; clippy was run without `-D warnings`, so "0
  warnings" is read from the log, not inferred from an exit code). All
  runs used the OP-family target as `CARGO_TARGET_DIR`, as the policy
  requires. The JS 12/12 result is cited from the author's earlier run; the
  `lab.js` bytes in the sealed binary equal the source I reviewed.

## 2. What I verified in the source

### 2.1 Flag and preparation order

- `bins/spc-lab/src/main.rs`: `#[arg(long, default_value_t = false,
  conflicts_with = "topology")] operator_ui: bool` on the `runtime`
  command only; threaded into `RuntimeOptions`; the engine-direct `Run`
  command hard-codes `operator_ui: false`. `cli_operator_ui.rs` asserts
  the help text, clap exit 2 with "cannot be used with", and exit 1 with
  "no api role" and no run directory for a scenario whose front lacks
  `api`.
- `Runtime::prepare`: the `operator_ui && topology.is_some()` bail is the
  first statement (before `binaries.verify()`, scenario read, run dir,
  identities or leases); the api-role loop runs after the scenario parse
  and before `create_dir_all(run_dir)`. Both are unit-tested with
  `!run_dir.exists()` assertions, the topology case with nonexistent
  binary paths to prove nothing was touched first.
- `EngineOptions.operator_ui` is appended after `link_backend`; every
  in-tree constructor (`main.rs` Run, `runtime.rs`, `tests/common/mod.rs`
  x2, `traffic_contract.rs`) sets it explicitly, `false` everywhere except
  the runtime pass-through and the new test helper.

### 2.2 Resource plan

`plan_resources(scenario, run_id, run_dir, operator_ui)` adds
`front.<id>.operator` with requested port 0 through the same `add(...)` /
`PortAllocator` path as every other listener (flock lease, loopback
`Endpoint`), only when `operator_ui`. The unit test asserts absence without
the flag, presence with it, `host == "127.0.0.1"`, non-zero port, all
planned ports distinct, and exactly one extra listener. The key follows the
`front.<id>.api` pattern and lands in the run manifest, so attach/adopt
reconstructs it.

### 2.3 Config body extraction and rendering

- `pool_config_body(&PoolConfigBody)` carries the former inline template
  verbatim; the only template change is `{operator_section}` appended
  after the `[api]` block, which is the empty string when `operator` is
  `None`. The opt-out body is therefore byte-identical to the pre-EXP-OP
  render; the opt-in body appends exactly `\n[operator]\nlisten =
  "<host:port>"\n`. `render_pool_configs` resolves the planned endpoint
  with `.context("front operator listener is missing")?` (fails closed if
  the plan and the flag ever disagree).
- Tests load the rendered files through the real
  `spc_pool::config::Config::load`: plain body parses with
  `operator.listen == None`; opt-in body parses with the exact listen
  string; removing `api` from roles yields `requires the api role`;
  rewriting the listen to `10.0.0.8` yields `not a loopback address`; a
  mirrored pre-OP `deny_unknown_fields` struct accepts the plain body and
  rejects the opt-in body with an error naming `operator`, proving an old
  pool binary fails closed rather than ignoring the section. The fixture's
  `work_reference_target` is the pool parser's required 32-byte hex (the
  documented fixture fix); `[template] source = "rpc"` is the production
  template source and `Config::load` validation does not depend on the
  `rpc` cargo feature.

### 2.4 Route proof and evidence

- `publish_operator_urls` runs after `self.start()` (engine start waits on
  each front's SV2 `Readiness::Tcp`; in `spc-pool` `main.rs` the api role
  block - which binds the public API and then the operator listener - runs
  before the front role block, so SV2 readiness implies the operator
  listener is already bound or the process has already exited) and before
  the hub HTTP server starts. For each front with a planned operator key it
  re-checks `host` is `127.0.0.1`/`::1`, probes `http://<listen>/` with
  `operator_page_served` (20 attempts x 500 ms; `namespace::http_request`
  returns `Ok` only for 2xx, rejects 3xx/4xx/5xx, malformed status lines,
  heads over 64 KiB and bodies over 8 MiB, `Connection: close`, 30 s
  read/write timeouts), then runs the informational
  `operator_observation_route_present` (schema string equality; `None` /
  `Some(false)` / `Some(true)`) only if the page was served. One
  `UnknownEvidence` fact per front, topic `operator_observation`, detail
  `{front, listen, loopback, page_served, observation_route}`, actor
  `Observer/runtime-operator`, source `operator`. URL printed only on
  `page_served`; otherwise a warning and no URL. No identity field, no
  credential, no private path in the detail.
- The JSON probe never gates publishing and no code path turns a 200 into
  a funds or health claim; the interface note reads "served proof was
  recorded at start".
- Unit tests cover: stub serving the schema (`Some(true)`), stub without
  the route (`None`), stub with another schema (`Some(false)`), and a
  refused connection running the full window to `false` (the ~9.5 s in the
  receipt matches 19 x 500 ms).

### 2.5 Hub inventory and JS guard

- `inventory.rs`: `OperatorFact { page_served }` collected per `front`
  from the `operator_observation` topic (last fact wins); in `pool_group`
  the interface is pushed only when the manifest has
  `front.<id>.operator`, the fact says `page_served: true`, and
  `browser_url(endpoint, "/", namespaced)` returns `Some` - which it does
  only for a non-namespaced run whose endpoint host parses as a loopback
  IP. The URL is formed from the manifest endpoint, never from the fact.
  No network request is made by the projection. The five integration
  tests cover typed-and-matching, not-served, not-planned, namespaced
  (asserting no `http://127.0.0.1:` anywhere in the JSON) and recorded
  read-only (interface kept; unlinking is the JS guard's job, as for
  `api_json`/`metrics`).
- `lab.js`: the only changes are the `kind` parameter and the single
  allowance `kind === "operator_ui" && url.pathname === "/"`; protocol,
  loopback hostname, username, password, search and hash rejections are
  unchanged and still apply to the operator kind; `interfaceMarkup` passes
  `item.kind`; `staleRecordedEndpoint` still nulls any absolute URL in
  recorded mode. The node tests assert the root links for `operator_ui`
  on `127.0.0.1`/`[::1]`, every other kind still rejects the root, and the
  operator kind rejects `/operator`, `/api/pool/observation`, `?proxy=1`,
  `#x`, credentials, `https:` and `10.0.0.8`; recorded markup has no `<a`
  and no port. `lab.html`/`lab.css` are hub byte-equal.

### 2.6 Boundaries

- Approved OP backend untouched by hash: `bins/spc-pool/**`,
  `crates/spc-store/**`, `static/operator/**`, `config/*.toml` all equal
  the `d3710c0c...` baseline rows. No Ark server, no `captaind` touch, no
  server modification anywhere (AGENTS "vanilla Ark interoperability"
  holds trivially; this packet never reaches an Ark interface).
- EXP-ACCESS is deliberately not imported (no `expauth`/access files in
  the delta). The report's section 6 says "`expAccess` remains
  unapproved"; that is stale (ACCESS has a Fable approval at
  `8085616e...`) but the conclusion the packet draws from it - do not
  compose here, fail closed at serial merge - is the right one. The merge
  handoff must preserve the approved ACCESS scope: inventory operator data
  only when expanded, static lab assets public with closed facts only, and
  the OP page stays on its own loopback listener without a retrofitted
  auth layer. This packet authorizes no expanded access and no manual
  wallet flow.
- Nothing starts a miner, node, Ark daemon or gallery; no live
  experiment is claimed.

## 3. Findings

No blocking defect. Ranked, with proof gaps separated from behavior.

### Behavior (non-blocking hardening, merge owner's discretion)

**H1. Inventory ignores the fact's `listen`/`loopback` and source.** The
`operator_observation` branch reads only `front` and `page_served`; the
redis/postgres branches next to it gate on `event.source`. The integration
tests illustrate the gap: their fixture fact carries `listen:
"127.0.0.1:48181"`, which never equals the planned ephemeral port, and the
interface is still granted. Within the run model this cannot mislead - the
URL is always formed from the manifest's own planned loopback endpoint,
evidence and manifest are per run directory, prepare refuses an existing
run directory, and the only producer of `Unknown`-topic events is the
runtime itself - so it is not a defect in this slice. Suggested tightening
(six lines, test fixture included): collect `listen` and `loopback` into
`OperatorFact`, require `fact.listen == endpoint.display() &&
fact.loopback` alongside `page_served`, and require
`event.source.as_str() == "operator"`; make the fixture use the planned
listener's `display()`. If taken, it is a code change and needs a fresh
freeze; I do not require it for this approval.

**H2. Probe worst case.** With a listener that accepts and never answers,
`operator_page_served` can take 20 x 30 s read timeouts (about 10 min) per
front before giving up; the realistic failure (connection refused, process
dead) completes in about 10 s, as the unit test shows. Sequential fronts
add linearly. A shorter per-attempt read timeout for this probe (the
`http_request` helper is shared, so it would need a parameter) would bound
startup tighter. Not reachable with axum in the loop; noted for
proportionality, not required.

**H3. Detached stub threads in the three probe unit tests** loop on
`incoming()` until the test process exits, so their ephemeral listeners
stay bound for the lifetime of the `spc-lab` lib test binary. Harmless in
`cargo test` (three sockets, released at process exit, no run-dir or lock
effect); a `set_nonblocking` accept loop with a stop flag would release
them per test. Hygiene only.

### Documentation (merge conditions, section 4)

D1-D3 below. None changes behavior.

### Report accuracy nits (no action beyond D3)

- Full snapshot quoted in the report (`5a84bef6...`) is a stale pre-report
  value; the sealed manifest's full snapshot is `fd198c44...`. Expected
  (the report cannot contain its own hash) but should not be quoted as
  the freeze value. Code snapshot `a9b6e4fc...` is quoted correctly.
- "Cargo.lock delta ... in the spc-lab-cli dependency list" is wrong; it
  is the `spc-lab` list (section 1).
- "`expAccess` remains unapproved" is stale (see 2.6).
- "Compiled-asset proof ... occurs 6 times" is a weaker proof than the
  binary supports; the full assets are embedded byte-exact (section 1).

## 4. Merge conditions (documentation only)

**D1.** `docs/decisions/2026-11-08-operator-runtime-wiring.md` item 9 and
`docs/LAB-RUNTIME.md` section 4 say "The Ark expanded profile is still
required for the page's money facts; without it those facts read unknown."
That is inaccurate against the approved page: total pending liability,
Ark-due and payout counts are read from the pool's Postgres regardless of
any Ark role (an empty store legitimately reads `0`, not unknown); only
the float reads `not_configured` until AP-4. Replace with: "The float fact
reads `not_configured` until the AP-4 status source exists; the liability
and payout facts are the pool store's actual values."

**D2.** `docs/LAB-ROLES.md` rows 66 and 69 still say the API/metrics links
belong to the api role, "not to a pool HTML operator UI", with no mention
of the opt-in page. Add one clause to the pool-front row: "With `spc-lab
runtime --operator-ui` (host runs only) each front also serves the
approved read-only operator page on a separate leased loopback listener,
linked in `/lab` as the typed `operator_ui` interface after a start-time
HTTP 200 route proof." `LAB-ROLES.md` is hub-verbatim today, so this is a
hand-merge hunk for the merge owner, same as the hub's own R1 sentence.

**D3.** Correct the three report nits above (full snapshot value,
`spc-lab` vs `spc-lab-cli` lock location, ACCESS status) in the packet
report before it is copied into primary, or annotate them in the merge
record.

The changelog fragment is one bullet in the `docs/changelog/README.md`
format and names the decision record; `docs/SPEC.md` is untouched
(correct, this is lab tooling). The decision record is otherwise
consistent with the code I read (items 1-8 each map to a hunk and a
test).

## 5. Scope statement

- Verified by me: manifest recipe and all 604 rows (bytes, raw modes,
  inventory); three-way classification of the 29-row delta against the
  OP and hub manifests with the hub source root re-verified; every new
  wiring hunk read in full (`runtime.rs` 729-line diff, `resource.rs`,
  `engine.rs`, `main.rs`, `inventory.rs`, `lab.js`, both JS test
  additions, both new Rust test files, constructor hunks); the
  `http_request` helper's bounds; the sealed binary's embedded assets and
  strings; the four receipt logs; Cargo manifests and lock placement; the
  absence of the hub R1 sentence; docs and fragment format.
- Not executed by me: any cargo, node or Nix command (priority ticket held
  by another session; no lock attempt made, no polling). If a window is
  granted later, the bounded rerun I would do is `cargo test -p spc-lab
  --lib runtime::tests`, `cargo test -p spc-lab --test
  operator_ui_inventory`, `cargo test -p spc-lab-cli`, and `node --test
  bins/spc-lab/tests/lab_helpers.test.cjs`, all on the OP-family target
  under nonblocking flock with the 50/20 GiB checks. The receipts and the
  byte-exact asset embedding make this confirmatory, not gating.
- Not approved here: any live experiment, native mining run, Ark
  end-to-end, expanded access or manual wallet flow; the first manual host
  run with `--operator-ui` after merge remains the gate for any claim
  that the page is served in the lab. This review covers the wiring code
  as frozen and nothing beyond it.
- No candidate file was edited; nothing was committed; no shared cleanup
  performed; no process started.
