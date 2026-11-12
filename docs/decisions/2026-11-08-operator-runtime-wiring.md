# 2026-11-08 - Host operator runtime wiring: --operator-ui, port lease and hub link (EXP-OP runtime)

## Trigger

The approved EXP-OP operator page
(`2026-11-08-pool-operator-page.md`) exists in `spc-pool` behind an
optional, off-by-default, loopback-only `[operator]` config section, but
no lab surface can reach it: the host runtime never renders the section,
allocates no listener for it, and the lab-live-hub inventory (Fable
approved, code snapshot `712fa8cb...`, full snapshot `3896b132...`) has
no typed interface for it and its JS link guard rejects the operator
root URL. This packet wires the approved page into the host runtime end
to end without touching the pool's operator implementation, its assets
or its SQL.

## Decision

1. **One CLI flag, off by default, host only.** `spc-lab runtime
   --operator-ui` (default `false`) is the only entry point.
   `Runtime::prepare` rejects the combination with `--topology` as its
   first statement - before any binary check, filesystem effect or port
   lease - because no reviewed controller relay exists for a host
   browser to reach an isolated front's loopback listener. The pool's own
   `OperatorRequiresApiRole` remains as defense in depth.
2. **One loopback listener per front through the existing port-lease
   machinery.** `plan_resources(scenario, run_id, run_dir, operator_ui)`
   (signature change) plans `front.<id>.operator` with requested port 0
   exactly like every other listener: cross-process `flock` lease,
   `Endpoint::loopback` by construction, recorded in the run manifest so
   attach/adopt reconstructs it. The key follows the existing
   `front.<id>.api` pattern. No ad-hoc listener exists anywhere in this
   packet.
3. **Every front must carry the api role, checked before the run
   directory exists.** The operator listener runs inside the pool API
   process (EXP-OP decision item 1), so a front without the api role
   cannot host it. `Runtime::prepare` rejects such a scenario before
   `run_dir`, identities or port leases exist, producing a clear
   preparation error instead of a pool process that dies on its own
   config validation.
4. **Rendered TOML is validated by the real pool parser.** The pool TOML
   body construction moved from `render_pool_configs` into
   `fn pool_config_body(&PoolConfigBody) -> String`; the format template
   is unchanged and the `[operator]` section is appended only when an
   operator endpoint was planned, so the opt-out body is byte-for-byte
   the pre-EXP-OP shape. `spc-lab` gains `spc-pool` as a dev-dependency
   so tests parse and validate the rendered body with the real
   `spc_pool::config::Config::load` (including
   `OperatorNotLoopback`/`OperatorRequiresApiRole` and a mirrored
   pre-OP `deny_unknown_fields` struct proving an old pool binary fails
   closed on the section), never by string matching.
5. **Publish only after a route-readiness proof.** After `start`, the
   runtime probes `http://127.0.0.1:<port>/` with a bounded retry (20
   attempts, 500 ms apart). `http_request` only returns `Ok` on a 2xx
   status, so an HTTP 200 on the page is the proof that the page route
   is served; it is **not** a funds-health claim. The URL is printed
   (`spc-lab-runtime: operator front-<id>=<url>`) only on proof; a
   failed probe prints a warning and no URL. The endpoint is re-checked
   as loopback before any probe (defense in depth; a non-loopback
   listener is never probed and never published).
6. **The observation JSON probe is informational only.** A second probe
   of `/api/pool/observation` checks the `schema` field against
   `spc-pool-observation/v1` and records `Some(true)/Some(false)/None`
   (answered-expected / answered-other / not reachable). It never gates
   URL publishing and never claims health.
7. **Readiness is recorded as evidence; the hub derives "served" from
   it.** Each front's probe outcome is appended as an `UnknownEvidence`
   event, topic `operator_observation`, detail
   `{front, listen, loopback, page_served, observation_route}`, actor
   `Observer/runtime-operator`. The hub inventory (which performs no
   network requests by design) collects the latest fact per front and
   pushes a typed `operator_ui` interface **only** when the listener key
   exists in the manifest, the fact says `page_served: true`, and
   `browser_url(endpoint, "/", namespaced)` yields a host loopback URL.
   A served fact without the listener, or a listener without the served
   fact, publishes nothing.
8. **The JS link guard permits exactly the typed operator root URL.**
   `safeActualUrl(value, kind)` in `lab.js` adds a single allowance:
   `kind === "operator_ui" && url.pathname === "/"` on an `http://`
   loopback URL with no username, password, query or fragment. No other
   kind may link the operator root; the operator kind opens no other
   path (no wildcard, no proxy, no any-path loopback). The existing
   `staleRecordedEndpoint` logic already suppresses absolute URLs in
   recorded mode, so a recorded serve never renders a stale external
   operator link. `lab.html`, `lab.css` and every other byte of the hub
   UI are unchanged.
9. **No new experiment or behavior claims.** This packet changes no
   miners, hash, difficulty, payout, wallet or Ark behavior and starts
   no full native mining experiment; the parent runs the fresh manual
   host experiment after Fable approval. The float fact reads
   `not_configured` until the AP-4 status source exists; the liability
   and payout facts are the pool store's actual values.

## Compatibility and boundaries

- Without `--operator-ui`: `plan_resources` plans no operator key, the
  rendered pool TOML is byte-for-byte the pre-EXP-OP shape, no probe
  runs, no evidence fact is recorded and the hub inventory is unchanged.
- `expAccess` remains unapproved: any future shared http/inventory/
  expauth composition must fail closed at serial merge (expanded data
  operator only). No per-user private operator route is invented here.
- The hub layer is consumed as an explicit approved baseline: 13 files
  verbatim at their reviewed hashes plus three narrow semantic merges
  (`lib.rs` `pub mod inventory;`, `engine.rs` `ProcessRecord` import and
  three projection methods after `manifest_snapshot()`, `README.md` two
  hunks). The hub's `LAB-RUNTIME.md` was not copied; the lab keeps its
  T0 traffic content.
- The pool's operator implementation (`operator.rs`, assets, SQL,
  `spc-store`) is not edited. The public `[api]` router is not edited.

## Edits applied

Packet-owned:

- `crates/spc-lab/src/runtime.rs`: `RuntimeOptions.operator_ui`;
  prepare-time topology and api-role rejections; `EngineOptions`
  threading; `render_pool_configs(..., operator_ui)`; `PoolConfigBody` +
  `pool_config_body()`; `publish_operator_urls` + probe helpers; unit
  tests.
- `bins/spc-lab/src/main.rs`: `--operator-ui` flag (conflicts with
  `--topology`), `RuntimeOptions` pass-through, `operator_ui: false`
  for the Run command's engine options.
- `crates/spc-lab/src/inventory.rs`: `OperatorFact` collection from
  `operator_observation` evidence and the gated `operator_ui` interface
  push in `pool_group`.
- `bins/spc-lab/static/lab.js`: `safeActualUrl(value, kind)` and the
  `item.kind` pass in `interfaceMarkup` (the only JS change).
- `bins/spc-lab/tests/lab_helpers.test.cjs`: operator guard tests.
- `crates/spc-lab/tests/operator_ui_inventory.rs` (new),
  `bins/spc-lab/tests/cli_operator_ui.rs` (new), `crates/spc-lab/tests/
  common/mod.rs` `prepare_engine_with_operator_ui` helper.
- `docs/LAB-RUNTIME.md`: `--operator-ui` launch-command section.

Serial-merge hunks (explicit local merge notes in the packet report):

- `crates/spc-lab/src/resource.rs`: `plan_resources` signature +
  `front.<id>.operator` allocation.
- `crates/spc-lab/src/engine.rs`: `EngineOptions.operator_ui` field and
  the `plan_resources` call (on top of the hub's projection-method
  merge).
- `Cargo.toml` / `crates/spc-lab/Cargo.toml`: `spc-pool` workspace and
  dev-dependency entries.
- `crates/spc-lab/tests/common/mod.rs`, `crates/spc-lab/tests/
  traffic_contract.rs`: `operator_ui: false` constructor hunks.
