# EXP-OP read-only operator page - primary integration (2026-11-08)

Owner: serial primary Rust/Cargo integration writer (sole writer for this
packet; other agents isolated in hub/access/AP2 worktrees and docs-only on
primary; AP2 holds the build-priority ticket, no contention occurred after
this packet's builds completed).

Approval: `docs/reviews/2026-11-08-pool-operator-fable.md`
(sha256 `000a7c504b341815194319bf89030e13e66514b7c45b7add17b718bc89f3dc3e`),
rework re-review section - **APPROVE bounded to code snapshot
`365e0bf200a98252...` / full snapshot `d3710c0c3100c862...`**: the
operator module, static assets, `operator_observation` tests, the
`Store::liability_snapshot` store surface and `liability_snapshot` tests,
pool config/lib/main hunks, the example config section, the 3 spc-pool
dev-dep lock edges, and the OP changelog fragment/decision/report docs.
Explicitly NOT approved (and NOT done here): operator host-runtime wiring
(separate isolated packet `lab-operator-runtime-wiring`, needs Fable), any
`float` status source before AP-4, Ark/payout/mining claims, E2E claims,
and any change to the public `[api]` router, metrics, or standard runtime
flags/addresses.

External manifest: `/tmp/opencode/EXP-OP-primary-integration-manifest.json`
(sha256 recorded inside the T1a-closed receipts pattern; final hash below).

## 1. Pre-write verification (read-only, before any transfer)

- Candidate manifest `/tmp/opencode/EXP-OP-rework-parent-manifest.json`
  (file sha256
  `37b6abfde13015950bd26765f1131283107ccbfd301e116e070f5ea4edfb3fde`)
  reproduced: full snapshot
  `d3710c0c3100c862...`, code snapshot `365e0bf200a98252...`; all 592
  records re-hashed and mode-checked against the candidate worktree
  (`/home/ethan/.local/share/opencode/worktree/ad8886/lab-pool-operator`,
  read-only): 0 mismatches.
- 583-row baseline manifest
  (`/tmp/opencode/EXP-OP-baseline-manifest.json`, file sha256
  `ab174afe666d3689...`) matches primary byte-exactly for all 7 modified
  source files; all 9 new paths absent from primary.
- Primary pre-state roots: `Cargo.toml` `179312b2...`, `Cargo.lock`
  `b447fb06...` (T1a-closed state), `docs/SPEC.md` `1769cad1...` (44
  changelog entries), `flake.nix` `7233ba5c...` (vendor hash
  `sha256-xO1PIE3c...`), index `44f21a926d4d065ac6937fb0f81b698c7b2d0627`
  (== HEAD tree, nothing staged), status 184 lines.
- Known exception planned: the candidate lock row is the pre-T1 lock
  (`87c7b2ca...`); the merge adds only the 3 new dev-dep edges to the
  T1a-closed primary lock. Documented in section 3.

## 2. Transfer (exact approved delta only)

- **9 new files copied byte-exact** (source re-hash + post-copy
  hash+mode verify, all 100644):
  `bins/spc-pool/src/operator.rs` (`575059e7...`),
  `bins/spc-pool/static/operator/index.html` (`1c70699c...`),
  `bins/spc-pool/static/operator/operator.css` (`ec28ae49...`),
  `bins/spc-pool/static/operator/operator.js` (`349d7fb0...`),
  `bins/spc-pool/tests/operator_observation.rs` (`d768d8fd...`),
  `crates/spc-store/tests/liability_snapshot.rs` (`d6e3ea28...`),
  `docs/changelog/2026-11-08-pool-operator-page.md` (`73f6bdfe...`),
  `docs/decisions/2026-11-08-pool-operator-page.md` (`76ce4771...`),
  `docs/reviews/2026-11-08-pool-operator-implementation.md`
  (`48da337f...`).
- **6 modified files applied as edit-tool hunks** (never whole-file
  copies), each verified byte-exact to its manifest row afterwards:
  `bins/spc-pool/Cargo.toml` (`d8c71aca...`, dev-deps block),
  `bins/spc-pool/src/config.rs` (`7164f3ec...`, `[operator]` section +
  3 ConfigError variants + loopback/api-role validation),
  `bins/spc-pool/src/lib.rs` (`de59724d...`, `pub mod operator;`),
  `bins/spc-pool/src/main.rs` (`0e389c4b...`, operator task wiring),
  `config/spc-pool.example.toml` (`28fb0869...`, commented section),
  `crates/spc-store/src/postgres.rs` (`0cc93c92...`,
  `is_schema_mismatch` + `now_rfc3339` + `LiabilitySnapshot` +
  `liability_snapshot()`).
- **`Cargo.lock`** - exactly 3 dependency lines added to the primary
  `spc-pool` block (`"chrono",`, `"sqlx",`, `"uuid",`; dev-dep edges for
  the new test suite). Pre `b447fb06...` -> post
  `7ec525f8f09c5db12b7095467bff09f846056c60fa4860f6ea88e30658b734e8`.
  Proof: block-scoped reverse-removal of the 3 lines reconstructs the
  pre-state byte-exactly (`b447fb06...`); merged = pre + exactly 3 lines.
  Exception vs the candidate row (`87c7b2ca...`, pre-T1 baseline): the
  merged lock keeps the T1a `spc-lab-txd` block and all prior packet
  edges; nothing else changed.
- **`docs/SPEC.md`** - double splice at the head of the changelog list:
  the OP bullet (314 bytes, this packet) then the pending AP1b
  node-profile bullet (240 bytes,
  `docs/changelog/2026-11-08-lab-bark-v31-profile.md`, parent-directed
  consumption of the validated opt-in lab artifact; doc-only, no
  runtime/node/binary/default/pin change). Pre `1769cad1...` -> post
  `81de2be84d45b01abacce8146866c7a0e0d1d19e6b40a0d5a8bd0a6a000f1c8c`.
  Proof: independent double reverse-splice reproduces `1769cad1...`
  byte-exactly; 44 -> 46 entries; all prior bullets preserved (T1a, T0,
  AP-3a, T4 markers verified).
- Post-transfer full-delta verification: 15/15 non-lock manifest rows
  byte-exact (the 16th is the documented lock exception); `flake.nix`
  and root `Cargo.toml` unchanged at that point; index still
  `44f21a92...`.

## 3. Targeted validation (owned fixtures, flock + >=50 GiB policy)

All Rust builds/tests under nonblocking
`/tmp/opencode/silent-pool-heavy-build.lock` with the >=50 GiB check
inside (85-86 GiB free at acquisition; 83-84 GiB after; growth leaves
far more than 20 GiB). One blocked acquisition occurred (10:25, probe
step; the AP2 session's live postgres/redis fixture held the lock):
reported once, source work continued, single retry after source work
succeeded - no polling. The earlier tests step also had one blocked
first attempt (~10:15) retried once successfully at 10:19:59.

- Real-Postgres fixtures: owned throwaway PostgreSQL 17.11
  (`/tmp/opencode/EXP-OP-pg`, port 55923, trust auth, superuser;
  PostgreSQL binaries from the same nix store path the shared instance
  uses - the shared `/tmp/opencode/wt/pg` instance was never touched).
  `SPC_TEST_DATABASE_URL` pointed at it; both suites create/drop their
  own `expop_testfixtures_<hex>` databases; after all runs zero
  `expop_testfixtures_%` databases remained (verified, log
  `/tmp/opencode/EXP-OP-pg-leftover-dbs.txt`), and the instance was
  stopped and removed after validation.
- `cargo test -j2 -p spc-pool --test operator_observation`:
  **16 passed; 0 failed** (log
  `/tmp/opencode/EXP-OP-integration-logs/test-operator.log`; two of the
  16 are real-database tests).
- `cargo test -j2 -p spc-store --test liability_snapshot`:
  **5 passed; 0 failed** (real DB, no skips; the rework's
  race-conserved-oracle counterfactual included; log
  `.../test-liability.log`).
- `cargo fmt -p spc-pool -p spc-store -- --check`: clean.
- `cargo clippy -j2 -p spc-pool -p spc-store --all-targets --
  -D warnings`: clean (exit 0).
- No broad suites re-run: the four-packet and T1a gates remain green on
  their frozen snapshots; the only new code is this packet's delta,
  covered by the suites above plus the Nix gate below.

## 4. Production binary identity and operator smoke

- Merged binary: `cargo build -j2 -p spc-pool --features rpc` (dev
  profile; features: `rpc` only - default features empty, `ark` NOT
  enabled), exit 0, binary
  `target/debug/spc-pool` sha256
  `2ca52ba15d52987471d7172142d4304409eb336059c140cd08de5f6f3d8385e7`
  (identity record `/tmp/opencode/EXP-OP-integration-logs/binary-identity.txt`).
  Per the approval's provenance note, this is NOT expected to match the
  sealed candidate hashes (`1684cc84...` = candidate
  `cargo test --no-run` feature-unified, `2dbaadb8...` = candidate plain
  build); both candidate seals are preserved untouched in
  `/tmp/opencode/EXP-OP-rework-freeze/`; the merged primary builds its
  own identity recorded above.
- Operator smoke (owned fixture config
  `/tmp/opencode/EXP-OP-smoke/pool.toml`, adapted from the candidate's
  sealed browser-proof config: regtest, mock template, roles `[api]`,
  api `127.0.0.1:38190`, operator `127.0.0.1:38191`, owned PG database
  `expop_testfixtures_smoke`; log `.../smoke.log`), **all behaviors
  verified**:
  - `GET /` -> 200 `text/html`, `GET /operator.css` -> 200 `text/css`,
    `GET /operator.js` -> 200 `application/javascript`,
    `GET /api/pool/observation` -> 200 `application/json` with
    `cache-control: no-store`; DTO closed shape (`schema` =
    `spc-pool-observation/v1`, `errors` a list, no
    `endpoints`/`sp_id`/`vtxo`/`order_id`/`ark_address` anywhere).
  - Unknown operator path -> 404; the public `:38190` listener 404s
    all four operator paths.
  - Non-loopback `[operator] listen` (`0.0.0.0:38195`): rejected at
    startup, exit 1 with the `OperatorNotLoopback` message; nothing
    ever bound on 38195.
  - Off-by-default (no `[operator]` section): clean exit 0 after
    `api listener ready`, zero operator log lines, `:38191` never
    bound.
  - One recorded probe artifact (not a product defect): the smoke
    script's off-by-default step originally polled the public listener
    for 30s and reported "did not come up", because the approved main.rs
    (byte-exact to the candidate) spawns the api listener detached and
    an api-only role set with no operator task exits immediately after
    startup ("no roles selected; exiting"). The corrected recheck
    (same binary `2ca52ba1...`) asserts startup, clean exit and absence
    of any operator surface - all pass. Original 16/17 smoke log
    preserved at `.../smoke.log`; corrected recheck run in
    `/tmp/opencode/EXP-OP-smoke/pool-no-operator-r2.log`.
- NOT claimed here: the operator page "hosted" in any lab/production
  runtime (that is the separate `lab-operator-runtime-wiring` packet,
  not approved, not imported), any browser-suite re-run (the candidate's
  29/29 sealed browser proof stands on its own binary), and any change
  to standard runtime flags or default addresses (none transferred).

## 5. Vendor hash and Nix workspace-test gate

The lock changed (3 dev-dep edges) so the flake's `fetchCargoVendor`
hash had to be re-derived once:

- Probe: throwaway copy `/tmp/opencode/EXP-OP-nix-probe` (644 files,
  flake hash blanked, primary untouched), `nix build ... #checks.
  x86_64-linux.test` failed on the FOD hash mismatch reporting
  `got: sha256-2aQzyvfiU5S835wHOlUNYE9A42aGe00OvQslLnPn2Hg=`
  (log `/tmp/opencode/EXP-OP-nix-gate-logs/vendor-hash-probe.log`).
- Single sanctioned flake.nix edit (same-length hash swap):
  `sha256-xO1PIE3c...` -> `sha256-2aQzyvfi...`; flake.nix
  `7233ba5c...` -> `07c0a887e284b7ae03ab512f516d1d6aee2660b2a85381d7cad209fb17acf258`.
  No version bump, no other flake change.
- Frozen snapshot: `/tmp/opencode/EXP-OP-nix-src` (644 files,
  10014390 bytes, aggregate recipe sha256
  `6c02ed16396134ebcdfc302a72d0237043f3e21ec6bc2ab43cad6b2b2f90fb7d`,
  row manifest `/tmp/opencode/EXP-OP-nix-src-manifest.json`).
  Independent verify: snapshot == primary byte-faithful over the
  standard exclusion set; 15/15 OP delta rows byte-equal inside;
  Cargo.toml/Cargo.lock/SPEC/flake.nix all match the post-edit primary.
- Gate: `nix build --max-jobs 1 --cores 2
  path:/tmp/opencode/EXP-OP-nix-src#checks.x86_64-linux.test` ->
  **exit 0** (log `/tmp/opencode/EXP-OP-nix-gate-logs/gate.log`,
  check log `.../check-test.log`): 92 result lines,
  **815 passed / 0 failed / 18 ignored** (T1a gate: 90 lines, 793/0/18;
  delta +22 = +1 spc-pool lib test (247->248, operator.rs unit tests),
  +16 `operator_observation`, +5 `liability_snapshot`).
- Vendor FOD actual reproduction: staging FOD output
  `/nix/store/jwjbvmmnmyyc4vw4n065ifk51ljck54v-cargo-deps-vendor-staging`,
  narHash `sha256:0y6qwxrjw98bpl74syw6cvil0kv01makl1wwvyy98lz2yz53796r`
  = SRI `sha256-2aQzyvfiU5S835wHOlUNYE9A42aGe00OvQslLnPn2Hg=` - exact
  match with the declared flake hash (no silent stale vendor reuse);
  composed vendor output
  `/nix/store/6gbqkm8rrjz3lbjkdvp20vaj4fcvky23-cargo-deps-vendor`
  carries `Cargo.lock` sha256 `7ec525f8...` - the merged lock.
- Source binding: `/nix/store/hi2nyrd8z6ffm29ly4ag5mjkcwibm5vl-source`
  (narHash `sha256:0ci1p3zch57fvh3imq42wrvfzj4qx6dbipfy4s1aflsrc88hq1pv`,
  644 files; snapshot SPEC.md `81de2be8...`, Cargo.lock `7ec525f8...`).
- Output: `/nix/store/nz6024kfng3sb9lzl9il07ylqjxkx3ia-spc-workspace-tests-0.1.0`
  (deriver `2pq3j0gs...-spc-workspace-tests-0.1.0.drv`).

## 6. Final state and concurrent-work record

- Index `44f21a926d4d065ac6937fb0f81b698c7b2d0627` (== HEAD tree;
  nothing staged, no commits made) - unchanged from before the packet.
- Status 194 lines at receipt time: 184 pre-OP + 7 OP paths + 3
  concurrent foreign review docs
  (`docs/reviews/2026-11-08-expanded-access-foundation-fable.md`,
  `docs/reviews/2026-11-08-live-lab-hub-fable.md`,
  `docs/reviews/2026-11-08-traffic-history-v2-fable.md` - other agents'
  review records for queued packets; not absorbed, not reverted).
  Status captures: `EXP-OP-integration-gitstatus-{before,after}.txt`,
  `EXP-OP-receipt-gitstatus.txt`.
- Shared roots final: `Cargo.toml` `179312b2...` (unchanged this
  packet), `Cargo.lock` `7ec525f8...`, `docs/SPEC.md` `81de2be8...`
  (46 entries), `flake.nix` `07c0a887...`.
- Queue after this receipt (source work continues while the AP2 build
  priority ticket is active; no new build slots acquired): LP-hub
  (live lab hub, Fable APPROVE `712fa8cb...`) then T0-history-v2
  (traffic history v2 r1, Fable APPROVE `4c1bce8c...`).

## 7. Remaining not-running state (unchanged by this packet)

- The operator listener is off by default; nothing in any lab or
  production runtime config enables it. First manual runtime/config
  wiring belongs to the separate `lab-operator-runtime-wiring` packet
  (needs Fable before merge).
- `float.not_configured` until AP-4's real status source exists.
- No Ark/payout/mining/E2E claim is made or implied by this transfer.

External manifest: `/tmp/opencode/EXP-OP-primary-integration-manifest.json`.
