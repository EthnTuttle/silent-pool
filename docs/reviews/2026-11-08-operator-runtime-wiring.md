# 2026-11-08 - Operator runtime wiring review (EXP-OP host wiring + hub link)

Status: verified and frozen Fable-ready. All Rust tests, clippy and
rustfmt pass under the granted build window (see §7); JS guard tests
pass. No live experiment was run or is claimed.

## 1. Scope and baselines

This packet wires the Fable-approved EXP-OP operator page
(`docs/decisions/2026-11-08-pool-operator-page.md`, frozen baseline
`lab-pool-operator`, manifest `/tmp/opencode/EXP-OP-rework-parent-manifest.json`,
592 files, snapshot `d3710c0c...`, code `365e0bf2...`) into the host
runtime, and consumes the Fable-approved lab-live-hub layer (manifest
`/tmp/opencode/LP-hub-verified-review-manifest.json`, code snapshot
`712fa8cb...`, full snapshot `3896b132...`, base `b0ab4ca`) as an
explicit baseline layer by narrow hunks. Nothing else in the OP baseline
was touched: no OP module/asset/SQL edits, no miners/hash/difficulty/
payout/wallet/Ark behavior changes, no GUI gallery 18481 or manual 18480
changes.

## 2. Hub layer consumption record

Verified before overlay: the hub source root matches its manifest
(378 files, 0 mismatches) and the hub's own runtime baseline
(`/tmp/opencode/LP-final-runtime-manifest.json`, 370 files, verified
accessible) matches too. The hub made no `runtime.rs` change (identical
to its baseline), so my runtime wiring was never in conflict.

Consumed verbatim at reviewed hashes (my tree had no divergent lineage
for these - pure fast-forward of the approved hub delta - or hub-new
files):

| Path | SHA-256 (first 16) |
|---|---|
| `bins/spc-lab/src/main.rs` (pre-my-edit base) | `7d2350820d277a74` |
| `bins/spc-lab/static/app.js` | `1f0b6a8619b54245` |
| `bins/spc-lab/static/index.html` | `0e4335cb6041c2c6` |
| `crates/spc-lab/src/http.rs` | `0ba5102b859def80` |
| `crates/spc-lab/tests/http_sse.rs` | `93d739a4428bf49e` |
| `docs/ARCHITECTURE.md` | `e7dc2ecba2181768` |
| `bins/spc-lab/static/lab.css` (new) | `ccb829145f213947` |
| `bins/spc-lab/static/lab.html` (new) | `b452cd0494526fc4` |
| `bins/spc-lab/static/lab.js` (pre-my-edit base, new) | `3c6f7b549dedfbcc` |
| `bins/spc-lab/tests/lab_helpers.test.cjs` (base, new) | `99fe62f26ef874e5` |
| `crates/spc-lab/src/inventory.rs` (base, new) | `00c57593124f7aaa` |
| `crates/spc-lab/tests/lab_hub.rs` (new) | `962601e080e079be` |
| `docs/LAB-ROLES.md` (new) | `61471eea56d32ade` |

Semantic merges (recorded before my additional changes):

- `crates/spc-lab/src/lib.rs`: one hunk, `pub mod inventory;` (hub
  delta vs its baseline is exactly this line; my tree adds nothing
  else). Now `7bc62c3de7a9e9e3`.
- `crates/spc-lab/src/engine.rs`: hub delta is the `ProcessRecord`
  import plus three methods (`process_records`, `is_read_only`,
  `lab_inventory`) after `manifest_snapshot()`. Applied on top of the
  T0 lineage, then my own `EngineOptions.operator_ui` field and
  `plan_resources(..., options.operator_ui)` call. Now
  `fb2add588924ff74`.
- `README.md`: two narrow hunks (the `/` + `/lab` serve sentence with
  the LAB-ROLES pointer, and the docs-index LAB-ROLES bullet). Now
  `098f0165cdc05918`.

NOT taken from the hub: its `docs/LAB-RUNTIME.md` (this lab keeps its
T0 traffic content; the Fable R1 sentence correction is deferred to the
primary merge), wholesale README/engine/lib/CLI overwrite (only the
narrow hunks above), and all lineage-only diffs (Cargo.toml/lock,
flake.nix, spc-pool/*, spc-store/*, spc-agent/*, spc-crypto/*,
scenario.rs, manifest.rs, config examples) - the OP baseline versions of
those files stay.

## 3. What this packet adds on top

- `crates/spc-lab/src/runtime.rs` (now `24aa016a47d6ce55`):
  `RuntimeOptions.operator_ui`; prepare-time rejection of
  `--operator-ui` + `--topology` as the first statement (before any
  binary check, filesystem effect or port lease); per-front api-role
  rejection before the run directory exists; `EngineOptions` threading;
  `render_pool_configs(..., operator_ui)`; `PoolConfigBody` +
  `pool_config_body()` extraction (template verbatim, `[operator]`
  section appended only when planned, so the opt-out body is
  byte-for-byte pre-EXP-OP); `publish_operator_urls` (bounded 20 x 500 ms
  page probe, loopback re-check before any probe, evidence fact topic
  `operator_observation` with `{front, listen, loopback, page_served,
  observation_route}`, print only on proof) plus free helpers
  `operator_page_served` / `operator_observation_route_present`; unit
  tests (see §5).
- `bins/spc-lab/src/main.rs` (now `a384804445e37891`): `--operator-ui`
  flag, `conflicts_with = "topology"`, default off; `RuntimeOptions`
  pass-through; `operator_ui: false` in the Run command's engine
  options.
- `crates/spc-lab/src/resource.rs` (now `7a4ebf3966584b03`):
  `plan_resources` signature + `front.<id>.operator` allocation
  (requested port 0, same flock port-lease machinery, `Endpoint::
  loopback` by construction).
- `crates/spc-lab/src/inventory.rs` (now `565ac045a75a1f54`, hub base
  `00c57593124f7aaa`): `OperatorFact` collected from the
  `operator_observation` evidence topic; gated `operator_ui` interface
  push in `pool_group` (listener present AND fact `page_served: true`
  AND `browser_url(endpoint, "/", namespaced)` is a host loopback URL).
  The projection still performs no network request.
- `bins/spc-lab/static/lab.js` (now `d7366d047259e5fe`, hub base
  `3c6f7b549dedfbcc`): exactly two changes - `safeActualUrl(value,
  kind)` gains the `kind === "operator_ui" && url.pathname === "/"`
  allowance, and `interfaceMarkup` passes `item.kind`. Credential,
  query, hash, protocol and non-loopback rejection is unchanged; the
  recorded-mode `staleRecordedEndpoint` logic already suppresses
  absolute URLs, so recorded serves never render a stale external
  operator link. `lab.html` and `lab.css` are byte-identical to the
  hub baseline.
- `Cargo.toml` / `crates/spc-lab/Cargo.toml`: `spc-pool` workspace +
  dev-dependency entries so the rendered TOML is validated by the real
  pool parser. No cycle (spc-pool does not depend on spc-lab).
- Tests: `crates/spc-lab/tests/operator_ui_inventory.rs` (new,
  `ba05d26071b42067`), `bins/spc-lab/tests/cli_operator_ui.rs` (new,
  `32f56e733799ba00`), `crates/spc-lab/tests/common/mod.rs`
  `prepare_engine_with_operator_ui` helper,
  `bins/spc-lab/tests/lab_helpers.test.cjs` operator guard tests (on the
  hub base), `operator_ui: false` constructor hunks in
  `crates/spc-lab/tests/common/mod.rs` (second site) and
  `crates/spc-lab/tests/traffic_contract.rs`.
- Docs: `docs/decisions/2026-11-08-operator-runtime-wiring.md`,
  `docs/changelog/2026-11-08-operator-runtime-wiring.md`, and the
  `--operator-ui` section in `docs/LAB-RUNTIME.md` §4 (T0 traffic
  content untouched).

## 4. Explicit merge notes (non-owned hunks)

- `engine.rs`: hub's three projection methods + import merged first
  (recorded above); my two hunks are `pub operator_ui: bool` in
  `EngineOptions` and the `plan_resources` call-site argument. An
  earlier hunk ordering note: the field is appended after `link_backend`
  to keep the hub's field set prefix intact.
- `resource.rs`: signature change is breaking for any in-tree caller;
  the only callers are `engine.rs` (updated) and the new unit test.
- `common/mod.rs` / `traffic_contract.rs`: `operator_ui: false`
  additions keep every existing constructor call compiling unchanged.
- `Cargo.toml` (root): one line `spc-pool = { path = "bins/spc-pool" }`
  in `[workspace.dependencies]`; `crates/spc-lab/Cargo.toml`: one
  `[dev-dependencies]` table. `Cargo.lock` delta vs the frozen OP
  baseline is exactly one line - `"spc-pool",` added to the spc-lab-cli
  dependency list - verified by direct diff (frozen OP
  `87c7b2ca0b05a9f5...` vs mine `636c8983ee265ead...`).
- `main.rs` Run command: `operator_ui: false` in the engine options
  constructed for the (separate) `Run` flow - the operator page is a
  `runtime` command option only.
- `bins/spc-lab/src/main.rs` was hub-verbatim before my flag edits; the
  flag hunk is mine, everything else is the reviewed hub content.

## 5. Tests

Rust (all passing, see §7):

- runtime.rs unit tests: `pool_config_body_without_operator_is_pre_op_
  shape_and_parses`, `pool_config_body_with_operator_parses_and_
  validates`, `pool_parser_rejects_operator_without_api_role_and_
  non_loopback`, `rendered_operator_section_fails_closed_on_a_pre_op_
  pool_config` (mirrored deny_unknown_fields pre-OP struct),
  `plan_resources_allocates_operator_listeners_only_when_opted_in`,
  `runtime_prepare_rejects_operator_ui_with_topology_before_any_effect`,
  `runtime_prepare_rejects_operator_ui_without_api_role_before_run_dir`,
  `operator_page_probe_proves_route_and_observation_probe_is_
  informational`, `operator_observation_probe_reports_missing_route_
  and_wrong_schema`, `operator_page_probe_fails_closed_on_a_refused_
  connection` (bounded ~9.5 s).
- `crates/spc-lab/tests/operator_ui_inventory.rs`: typed interface
  matching the planned listener; absent when the fact says not served;
  absent without the opted-in listener; namespaced suppression; recorded
  serve keeps the interface (client-side unlinking is covered by the JS
  tests).
- `bins/spc-lab/tests/cli_operator_ui.rs`: `runtime --help` documents
  the flag; `--operator-ui` + `--topology` is a clap conflict (exit 2);
  the flag reaches `Runtime::prepare` and fails closed on a front
  without the api role before the run directory exists.

JS (passing, 12/12, `node --test bins/spc-lab/tests/
lab_helpers.test.cjs`): the typed operator root URL is linkable only
for `operator_ui`; every other kind still rejects it; the operator kind
opens no other path/query/credential/scheme/host; live mode links and
recorded mode unlinks with no stale port.

## 6. Boundaries and fail-closed posture

- `expAccess` remains unapproved: any future shared http/inventory/
  expauth composition must fail closed at serial merge (expanded-data
  operator only). No per-user private operator route exists here.
- No full native mining experiment was run; the parent runs the fresh
  manual host experiment only after Fable approval of this packet.
- The observation JSON probe never gates publishing and never claims
  funds health; an HTTP 200 on the page proves route presence only.
- The hub inventory performs no network requests; "served" comes only
  from the recorded evidence fact.
- No identity field (`sp_id`, `B_scan`, `B_spend`, `P_i`, block hash)
  crosses into the hub inventory or the operator URL.

## 7. Verification status (actual, 2026-11-08 build window)

All commands run under the heavy-build flock (ticket
`ses_e444fa08bffes1FApjGS7mp54I`, nonblocking, released after each
sequence) with `CARGO_TARGET_DIR` pointed at the shared OP-family
target `/home/ethan/.local/share/opencode/worktree/ad8886/lab-pool-operator/target`
(cache-only; frozen OP sources and sealed binaries untouched). The
pinned toolchain is cargo/rustc 1.93.0. Disk at lock time: 77-82 GiB
free, well above the 50 GiB floor.

Actual commands and results:

- `cargo test -p spc-lab` - exit 0. 34 suites, **175 passed, 0 failed,
  16 ignored** (the ignored are pre-existing traffic/T0 gates, not this
  packet). Includes all 10 runtime.rs operator unit tests (46/46 lib)
  and all 5 `operator_ui_inventory` tests. Log:
  `/tmp/opencode/EXP-OP-runtime-test-spc-lab.log`.
- `cargo test -p spc-lab-cli` - exit 0. **3 passed, 0 failed** (the
  real CLI package is `spc-lab-cli`, bin `spc-lab`; there is no
  `spc-lab-bin` package in this tree). Log:
  `/tmp/opencode/EXP-OP-runtime-test-spc-lab-cli.log`.
- `cargo clippy -p spc-lab -p spc-lab-cli --all-targets` - exit 0,
  **0 code warnings**. The only remaining notice is the pre-existing
  upstream `sqlx-postgres v0.8.0` future-compat line plus cargo
  cache-tooling permission notices, both present before this packet.
  Log: `/tmp/opencode/EXP-OP-runtime-clippy.log`.
- `cargo build -p spc-lab-cli` - exit 0. Sealed proof binary (copied by
  hash before any target relink, mode 500):
  `/tmp/opencode/EXP-OP-runtime-spc-lab-bin-7af444a983d0e913e026d810e2f46a5184e219b8eecbe0d8e11c3d6b27dfe156`.
  Compiled-asset proof on the sealed binary: the `operator_ui` guard
  string occurs 6 times and the "Pool operator page" label once. (The
  approved sealed OP binary's 1684-test feature is its own
  spc-pool-suite build and is not this binary; hash differences are
  expected and legitimate.)
- `rustfmt --edition 2021 --check` on every touched Rust file - clean
  under the repo `rustfmt.toml` (edition 2021, max_width 100).
- JS guard tests: 12/12 pass (`node --test bins/spc-lab/tests/
  lab_helpers.test.cjs`), run earlier; not repeated because the JS
  source is unchanged since and the compiled-asset proof above covers
  the shipped binary.
- `Cargo.lock` delta vs the frozen OP baseline: exactly one line
  (`"spc-pool",` in the spc-lab-cli dependency list), verified by
  direct file diff - no other dependency moved.

Bounded fix round (compile/lint only, no weakened assertions; every
failing test was fixed in the wiring or fixture, never by relaxing an
assertion):

- `operator_observation_route_present`: bind the parsed JSON to a local
  before borrowing (E0515), same semantics.
- Test stubs: the probe server threads are deliberately detached -
  joining them would deadlock on the `incoming()` loop; they exit with
  the test process.
- `PreOpConfig` mirror: `#[derive(Debug)]` + `#[allow(dead_code)]` so
  the fail-closed assertion can print the error.
- Two prepare-rejection tests: `.err().expect(...)` instead of
  `unwrap_err()` (Runtime is not Debug).
- Test fixture `work_reference_target`: plain 32-byte hex without the
  `0x` prefix (the pool parser requires it; the real scenario files use
  the same form) and the CLI fixture path concat needed a leading `/`.
- Removed the dead `at_ms` field from `OperatorFact` and elided an
  explicit lifetime in the inventory test (clippy).

Packet freeze (post-verification, recipe identical to the parent OP
manifest and reproduced against it byte-for-byte):

- Manifest: `/tmp/opencode/EXP-OP-runtime-freeze-manifest.json`.
- 604 files (walk excludes `.git`/`target`/runtime test datadirs), 0
  missing vs the OP baseline; delta 29 rows = 17 modified + 12 new.
- Full snapshot `5a84bef61b85fcc825cd91a31e9587e1d46b1036b8f8cf1ff8a706ac18e2db94`;
  code snapshot (excludes only this report)
  `a9b6e4fc64b468b0e6d754930749c839412bb2f3b79b50e6c35fba6cd3bcd6d2`;
  base `b0ab4ca34fda07c61a6335d05f598eaacc69678c`.

## 8. Hub handoff contract (for the primary serial merge)

- The run manifest carries one extra listener key per front when (and
  only when) the run opted in: `front.<id>.operator` -> loopback
  `Endpoint` (same shape as `front.<id>.api`).
- The rendered pool config gains an optional `[operator]` section with
  `listen = "<loopback host:port>"`; the pool parser validates it
  (loopback-only, api-role-gated).
- The runtime appends one evidence event per front, topic
  `operator_observation`, detail `{front, listen, loopback,
  page_served, observation_route}`; `page_served: true` is the only
  basis for exposing the interface.
- The hub inventory exposes a typed interface `kind: "operator_ui"`,
  `url: http://127.0.0.1:<port>/`, label "Pool operator page", only
  under the §3 gating. The JS guard links exactly that root URL for
  that kind. A future hub entry point consumed via serial merge should
  treat these four facts as the whole contract.
