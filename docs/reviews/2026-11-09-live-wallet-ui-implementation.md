# Live lab access and wallet controls implementation

> Audited-input history. The 2026-11-10 W1-W4 correction and its exact current
> verification/build-slot status are recorded in
> `2026-11-10-live-wallet-ui-audit-fixes.md`. Prior freezes below remain preserved.

Date: 2026-11-09
Owner session: `ses_e3c670affffeBwvH6vNd799DJY`
Tree: `/home/ethan/.local/share/opencode/worktree/ad8886/lab-live-wallet-ui`

**Final verification:** 64 distinct targeted Rust tests passed, the six wallet
tests passed again after the scope-guard lint fix, clippy and CLI/browser-host
builds passed, and 13 targeted Chromium workflows passed on the compiled Rust
controller and final compiled CLI routes. All 14 static route responses were
byte-exact to source. The AP-3b private input producer is now implemented and
tested in its owner's packet; runtime registry/source integration and the T1
provider join remain separate. No funded workflow is claimed. Latest artifacts
are `live-wallet-ui-final-r2-*` under
`/tmp/opencode/`; earlier freezes/receipts are retained unchanged.

## Result and scope

Implemented browser entry into the existing expanded-access controller:
operator/user token entry, authenticated context discovery, authenticated
fetch-SSE, logout/revocation, the existing operator directory and controls,
and a user-scoped worker/wallet view. The wallet adapter reads real registered
agent status and consumes the actual AP-3b private queue shape. It does not
invent a wallet, select inputs from SQLite, or report queueing as completion.

**Initial browser/JS verification passed against an owned test-only
HTTP/provider fixture. The followup below records newly authorized compiled
verification; the initial receipts remain immutable. No funded workflow or
full-lab integration is claimed.** The precise
remaining source integrations are below; this is implementation, not a mockup.

## Baseline and file ownership

Started at `b0ab4ca34fda07c61a6335d05f598eaacc69678c`, verified clean before
bootstrap. Copied exactly 682 hash-verified selected rows (11,105,307 bytes)
from immutable `/tmp/opencode/ladder-integration-baseline-src` using
`/tmp/opencode/ladder-integration-primary-baseline-20261109.json`.
The manifest file hash is
`e50db9e7735c630959375bb3ebc3d13aac6652d6b8d5b26fcde71898c33930ab`;
its compact `{base,files}` snapshot hash is
`4561a2df12d9980ee1a16ee285e6667a94d807af7b9c5aa6d30bc81a7ef012b6`.
No git/runtime/target copy was used.

Owned additions:

- `bins/spc-lab/static/access.js`: shared public bootstrap/session/SSE parser.
- `bins/spc-lab/tests/access.test.cjs`, `live_browser.cjs`.
- `crates/spc-lab/src/wallet_projection.rs`, `wallet_control.rs`.
- `crates/spc-lab/tests/wallet_ui.rs`.
- Dated decision and this report.

Modified existing UI assets: `app.js`, `index.html`, `style.css`, `lab.js`,
`lab.html`, `lab.css`; the existing hub test now allows only its two session
buttons while still forbidding controller command effects from the hub.

Serial-owner hunks are limited to `crates/spc-lab/src/{access,command,engine,
http,lib}.rs` and the `docs/changelog` SPEC fragment. Parent must merge these against
AP-3b and final T1 J1; do not copy another topic's whole tree over this baseline.
No agent wallet/core/signature implementation was edited or copied.
Cargo roots, lockfile, flake, pins and `docs/lab-build-report.html` retain their
baseline hashes. No primary/other-tree edits, commits or shared-cache cleanup.
The paused dense mockups, gallery and user feedback were not touched.

## Implemented behavior

### Session and streams

- Exact public static `/access.js` plus the existing static routes. No private
  inline facts. Existing authenticated health then run-scoped `/me` supply
  context; no new unauthenticated data route.
- Bearer is sent only in Authorization on same-origin controller fetches.
  Tokens use `sessionStorage` keyed by run/principal; no URL/query/fragment,
  localStorage, cookie, log, body or HTML echo. Request redirects are refused.
- Every clear/switch/revocation aborts active requests and stream readers and
  clears private DOM/caches. Epoch guards reject delayed old-session bodies.
  Unknown commands retain their exact envelope in the same scoped tab storage
  across authenticated same-context reload; explicit logout/switch clears it.
- Native fetch/ReadableStream SSE with bounded incremental framing, streaming
  UTF-8, CR/LF/CRLF, comments, multiline data, persistent id, last-event-id and
  resync. No native EventSource or query-token workaround.
- Added server revocation checks before replay/resync emissions; changed user
  claim scope closes the stream rather than continuing with an old whitelist.
  Existing duplicate/unknown/revoked header checks, exact Origin check, CSP and
  no-CORS behavior are retained.
- Legacy access-off discovers and streams without a token. A 401 displays
  login rather than retrying private reads indefinitely.

### Role flows and display

- Operator keeps the small `/lab` role directory and existing pool operator
  links, plus root controls. The separate pool read-only listener is untouched.
- A user on `/lab` goes to root's own filtered snapshot/projection. It never
  fetches whole Inventory or catalog. A 403 has no raw-JSON/fallback path.
- Worker requested/achieved H/s and active job come from the existing single
  miner-control poll projection, with age/error. There is no second miner pipe.
- SP/Ark funds, watch-only coinbase outputs and T1 test-payment funds are named
  separately. T1/background balances are never added to earnings. Missing
  amounts stay unknown, not zero; decimal-string money uses BigInt formatting.
- Keyed VTXO controls and restored worker/disclosure focus preserve selection,
  budget input and keyboard focus during polling. Tables scroll locally and
  hostile text/URLs stay inert. Mobile and desktop overflow were checked.
- T1 handlers match the examined final J1 fields exactly:
  `send_user_payment {user,destination_address,amount_sats,fee_rate_sats_vb}` and
  `pause_tx_driver/resume_tx_driver {driver_id}`. JSON u64 form input is limited
  to exact JS integers, rejecting unsafe values instead of rounding. Actual
  signer checksum/network/fee policy validation remains authoritative.

### Private wallet provider and queue

- Expanded `/wallets` authenticates, checks view capability, and filters the
  server registry by claim identity AND pool before any provider I/O. Operator
  explicitly may read all registered wallets. No browser path or endpoint is
  accepted. The source registry validates scenario identity/front/pool mapping,
  private canonical datadir and loopback status address.
- Fixed `/status` reads are bounded; only allowlisted fields cross into the
  controller DTO. Agent u64 money becomes exact decimal strings before browser
  serialization. Raw error strings become closed codes.
- `WalletRefresh`, `WalletOffboard`, `WalletReadInputs` are explicitly scoped
  before journal/replay. Newly generated claims carry maintenance capabilities;
  old claims are not silently expanded. Offboard supports direct derived-SP or
  stock-normal inputs in a homogeneous set and additionally requires a parsed
  native regtest witness destination.
- The existing AP-3b queue is used exactly:
  `{operation_id, vtxo_ids, action:{kind:"refresh"}|{kind:"offboard",address}}`
  under the registered datadir's private `commands` directory. Input IDs are
  unique 36-byte outpoints, 1..64 per action, present in the fresh source list.
  Files are owner-only, no-follow/nonblocking reads reject nonregular data,
  and queue publication is no-replace with file/directory fsync.
- Queue write returns running/submitted, never completed. Same-command checks
  only read `<operation_id>.result`; they never requeue. `stock_submitted`
  remains running; only actual matching round/offboard-confirmed results
  succeed. Missing or ambiguous results remain unknown with the immutable ID.
  No wallet-network wait holds the controller command lock.

## Exact provider/integration dependencies

The full handoff is `/tmp/opencode/live-wallet-ui-provider-handoff.md`.

1. **AP-3b launch registry hook.** The baseline lacks `Scenario.wallet_agents`;
   this packet deliberately does not copy the AP-3b topic. After serial merge,
   call `WalletRegistry::bind` for each declared entry after its successful
   launch. Actual AP-3b `wallet_bridge.rs:288` configures `dir.join("state")`
   as the datadir, not the online provisioning root. The handoff supplies the
   exact narrow caller snippet, identity/front, leased status and canonical
   configured datadir. Until that call is integrated, `/wallets` returns an
   empty not-configured registry.
2. **AP-3b private selectable-input DTO - implementation dependency closed.** Examined `StatusSnapshot` exposes
   nullable balances, registration/state, lease and source timestamp, but no
   VTXO inventory. The separately implemented owner-private atomic `ui-inputs.json` under the
   configured datadir: `{schema:"spc-agent-private-inputs/v1",updated_at_ms,
   inputs:[{id,amount_sats:decimal-string,state,expiry_height:null|u32,
   rail:"derived"|"normal"}]}`. This adapter accepts only that closed allowlist,
   at most 4096 rows and a <=15-second source age. Missing/stale/empty input
   source disables actions. The actual producer is now ready in
   `lab-wallet-ui-provider`, full `e97ade1235110d58b008d527518ae97fe37fcd863ebe438a383558d631d6fdb6`,
   code `7119fd63576efd950e0c2f1c7f687dd7e0cbed95565992097bae89292bf79cc0`.
   It derives observations from existing validated SP/stock inputs and excludes
   reservations, unselectable rows and unknown expiry; it uses the oldest proof
   batch's start time. Empty means no eligible proven inputs, not zero balance.
   Its serial source integration and registry caller remain pending here; its
   implementation does not. No arbitrary SQLite read or fabricated input list
   substitutes for the real producer.
3. **Canonical watch-only aggregation** remains AP-3b-owned. The UI explicitly
   says unavailable until the actual mature/immature scan summary exists.
4. **T1 per-user balance/address projection** is still absent in the examined
   final J1 access/http source. The frontend has real SendUserPayment handlers,
   but the live button is unavailable until an authenticated per-user source
   supplies real spendable/unconfirmed balances and receive address. Optional
   catalog `traffic_controls:{available:true,drivers:[{driver_id}]}` activates
   operator pause/resume after actual T1 wiring; it is not synthesized from
   a running process. Current J1 reference:
   `/tmp/opencode/traffic-runtime-j1-final-handoff.json`, full `e53118dd...`,
   tested code `8b1bbdc9...`.
5. **Funded backend checks.** The parent reports the six AP-3b audit defects
   fixed and their combined gate approved; the private producer is an additive
   packet against that fixed base. No old funded packet or wallet/core
   implementation was copied. Funded real-VTXO provider validation remains AP-5,
   and this UI packet does not claim funded refresh/offboard/T1-send acceptance.

## Initial verification (before the assigned-cache followup)

- `node --test bins/spc-lab/tests/access.test.cjs
  bins/spc-lab/tests/app_helpers.test.cjs
  bins/spc-lab/tests/lab_helpers.test.cjs`: **33 passed, 0 failed**.
  Receipt: `/tmp/opencode/live-wallet-ui-node-tests.log`.
  Includes all split boundaries/multiline/CRLF/id/resync, a real loopback HTTP
  fetch stream with split UTF-8, revocation and abort, strict money >2^53,
  real command field shapes, no 403 fallback, and pending-envelope retention.
- Real Chromium/CDP against the candidate asset bytes and owned test-only
  HTTP/provider fixture: **13 workflow checks passed** in
  `/tmp/opencode/live-wallet-ui-browser-final2/proof.json`.
  Captures source asset hashes, Node executable hash, network URLs/statuses,
  console log entries, loading failures and Runtime exceptions. Screenshots:
  `operator-mobile.png`, `user-mobile.png`, `desktop.png` in that directory.
  Checked real sign-in/logout, operator directory, user no-Inventory reads,
  hostile text/URL handling, keyboard/disclosure/budget selection across polls,
  exact money, actual refresh click/envelope, immutable unknown retry/reload,
  revoked stream close, rotated-token login, legacy tokenless and 390x844 /
  1440x900 overflow. Zero uncaught exceptions/unexpected loading failures;
  expected login/revocation 401 console entries are recorded explicitly.
- Early helper test failure was the old hub assertion forbidding every button;
  it now specifically allows its two session buttons and still rejects command
  effects. An expanded CDP attempt (`live-wallet-ui-browser-final`) clicked a
  root logout under the sticky header because the test scrolled to block-start;
  the final harness centers its real click targets and asserts storage clearing
  immediately. That failed receipt is preserved, not counted as a pass.
- Changed Rust modules and `wallet_ui.rs`: **rustfmt check passed** using the
  installed Nix-store rustfmt, no build. Receipt:
  `/tmp/opencode/live-wallet-ui-rustfmt.log`.
- **Rust tests/clippy/compiled-controller CDP: not run.** No parent-assigned
  primary-family target or build scope was provided. Disk observation was
  about 81 GiB free on the actual target/store filesystem; no build was started,
  no cold target created, and no lock was waited on. `wallet_ui.rs` contains five
  meaningful pending tests for exact private projection, queue/result states,
  stale/malformed/private-file rejection, cross-wallet scope before replay, and
  actual HTTP private/own-only routes. Run it with existing access/SSE tests
  under the parent-assigned nonblocking heavy-build slot before integration.

Existing ACCESS/Hub/OP primary approval reports were read as baseline evidence,
not reused as proof of this changed candidate. No existing daemon, inspection
port, gallery or funded wallet was used for these checks.

## Transfer artifacts

Final source/delta hashes and baseline-relative hunks are frozen in
`/tmp/opencode/live-wallet-ui-manifest.json` and
`/tmp/opencode/live-wallet-ui-baseline.patch`; the latter must be applied by
the serial merge owner, with focused conflict resolution for AP-3b/T1.
`docs/changelog/2026-11-09-live-wallet-ui.md` now supplies the standard
one-bullet frontmatter fragment. The initial direct SPEC entry was reversed
exactly: candidate SPEC SHA256 is the original baseline
`bfb28139f811e8fb74095509a6819b8447d9c3ea98c2f66d3d2f73321d0dce35`.
The parent owns the separate final HTML writeup.

## Assigned-cache compiled verification followup

The parent assigned `/home/ethan/code/silent-pool/target` as cache only; all
source paths remain this owned worktree. The runner
`/tmp/opencode/live-wallet-ui-rust-gate.sh` uses nonblocking `flock -n -o`,
checks both actual target/store filesystems inside the lock, enforces >=50 GiB
and 5 GiB projected growth with >=20 GiB residual, verifies pinned Nix
Rust/Cargo 1.93.0, and hashes source before/after every step.

- R1: acquired with 85,368,389,632 available bytes. Offline default crates.io
  index lacked locked `ar_archive_writer 0.5.3`; dependency resolution failed
  before compiling. No lockfile or dependency change was made.
- R2: read the existing vendor configuration at
  `/tmp/opencode/AP3b-cargo-home/config.toml` via Cargo `--config` (no edits to
  that home/vendor). At 85,363,351,552 available bytes, pinned 1.93 built this
  candidate and all **five authored wallet_ui tests passed**. Source binding
  before/after matched. Receipt: `/tmp/opencode/live-wallet-ui-rust-r2/`.
- Followup source adds explicit direct-derived offboard/homogeneous-set tests,
  R5 no-effect/noop terminal checks, source byte-hash projection and alias
  rejection. It keeps the exact private producer DTO agreed with AP-3b.
- The exact AP-3b registry caller patch is frozen in
  `/tmp/opencode/live-wallet-ui-ap3-join-r1/registry-bind.patch`, against input
  `d33dec30ae192087590864901f740c34802f91de5a42349742086d5838706784`.
  Frozen input, joined output and recipe hashes are in that directory; neither
  AP-3b tree was edited. The parent serial owner applies the caller after the
  approved wallet source and private producer are integrated.
- A new opt-in `wallet_browser_server` integration-test binary serves the
  actual Rust RunEngine/HttpServer/AccessStore/CommandLog and wallet adapters,
  with only explicitly owned node/miner/agent provider fixtures. Its CDP client
  is `bins/spc-lab/tests/compiled_browser.cjs`; it also verifies candidate CLI
  asset embedding. This is separate from the prior Node HTTP fixture proof.
- R3: six wallet tests passed, including direct-derived offboard, mixed-input
  rejection, wrong network, matching-confirmation-only success, and R5 noop /
  definite-no-effect results. Access scope/projection/hub routes and legacy
  concurrency/idempotency/restart/HTTP protocol suites passed. `http_sse` then
  failed its old implementation-string assertion for EventSource's
  `addEventListener("message")`, which cannot hold for the requested fetch
  transport. The test now verifies exact served app/access asset bytes and
  forbids `new EventSource`; its actual message-frame/replay/command assertions
  remain. A stop-on-drop guard was added around that owned fixture.
- That assertion panic left four hold/node fixtures alive after their test
  directory was dropped. Only PIDs 1377591/1377593/1377595/1377597 were stopped,
  through pidfds after checking recorded start ticks, executable and the exact
  unique r3 `http-sse` fixture path. Receipt:
  `/tmp/opencode/live-wallet-ui-rust-r3/owned-fixture-cleanup.json`. No inspection
  daemon or foreign process was signaled. R4 resumes only the failed/not-yet-run
  suites and fmt/clippy/build, retaining the prior passing receipts.
- The focused JS access suite now has eight passing tests, including a delayed
  old-session response body rejected after switching token context. Receipt:
  `/tmp/opencode/live-wallet-ui-access-r3.log`; the original 33-test receipt is
  untouched.
- R4: all remaining HTTP/SSE/subscriber-lag/hub/snapshot tests passed, bringing
  the nonduplicated targeted Rust total to **64 passing tests** across R3/R4.
  `cargo fmt -p spc-lab -p spc-lab-cli -- --check` passed. Clippy's nested Cargo
  invocation lost the command-line vendor override and hit the same incomplete
  default offline index before reaching code diagnostics. An owned tiny Cargo
  home now supplies the unchanged vendor mapping through `CARGO_HOME`, so nested
  Cargo sees it too; the target remains the explicitly assigned primary cache.
- R5 checkpoint: the next nonblocking lock acquisition returned **42 (busy)** before any
  build, disk mutation or code check. No wait/poll/retry was performed after that
  busy result until the parent explicitly assigned the next attempt. At that
  checkpoint clippy, CLI/browser-host build and compiled-server CDP remained
  blocked; R8 below resolves that checkpoint.
  Resume runner: `LIVE_UI_GATE=<fresh receipt directory> LIVE_UI_FINISH=1 bash
  /tmp/opencode/live-wallet-ui-rust-gate.sh` after the parent releases/assigns
  the next slot. At that checkpoint the CDP script was syntax-checked only.
- Independent source review after the busy result found an initial-login race:
  a cancelled older bootstrap discovery could clear a newer login before its
  context arrived. `mountAccess` now epoch-binds the whole authentication
  attempt, including catch/ready handlers. **Nine focused JS tests pass**,
  including the actual overlapping discovery/new-login counterexample, in
  `/tmp/opencode/live-wallet-ui-access-r6.log`. This client-only change is newer
  than the R3/R4 compiled asset receipts and must be included in the pending
  compiled-server browser run; it is not back-attributed to those receipts.
- The preserved checkpoint progress summary is
  `/tmp/opencode/live-wallet-ui-followup-progress.json`. The original source
  freeze, 33-Node receipt and 13-Node-fixture-CDP receipt remain immutable. This
  followup is frozen separately under `live-wallet-ui-followup-*`.

## Final remaining-sequence result (R7/R8 and compiled browser R1)

The parent explicitly authorized a new nonblocking attempt after checking the
lock free. R7 acquired it with 82,973,057,024 bytes available and reached actual
clippy code diagnostics. One `nonminimal_bool` diagnostic affected this packet's
wallet scope guard. The equivalent `is_none_or(pool_mismatch)` expression
replaces `!is_some_and(pool_matches)`; missing/wrong-wallet rejection remains
closed and no assertion was weakened.

R8 acquired the same global lock with 82,964,336,640 bytes available, the same
5 GiB growth estimate and pinned Nix Rust/Cargo 1.93.0. Every step's source
before/after hashes matched. Receipt directory:
`/tmp/opencode/live-wallet-ui-rust-r8/`.

| Step | Actual result |
|---|---|
| `cargo clippy --offline --locked -j2 -p spc-lab -p spc-lab-cli --all-targets -- -D warnings` | exit 0 |
| Affected `wallet_ui` recheck after the guard fix | exit 0, 6 passed |
| Final `spc-lab` CLI build | exit 0 |
| Compiled `wallet_browser_server` host build | exit 0 |
| Rustfmt check of the changed guard | exit 0 (prior complete package fmt remains R4) |
| Targeted compiled-server Chromium workflows | exit 0, 13 passed |

Sealed binaries were copied by hash before releasing the build cache slot:

- CLI `/tmp/opencode/live-wallet-ui-rust-r8/bin/spc-lab`, SHA256
  `363dbb192166350d56e0dc1b196c3ac46500e5135559936384a4391e8104ef12`.
- Production-controller test host `wallet_browser_server`, SHA256
  `15d69bdd8110db2c7f00d81f6d189e3359534355ef763b949b2a0ce08cf81ae5`.
- Explicit node/miner process fixture `spc-lab-fixture`, SHA256
  `bae1a8d8e7d2c46c5ccb2dceb80903b219b560e7a9e44ba93e8965ce39d11076`.

Browser evidence: `/tmp/opencode/live-wallet-ui-compiled-browser-r1/proof.json`
and `cleanup-and-summary.json`; source binding is in the sibling
`live-wallet-ui-compiled-browser-r1-{before,after}.json` files. This run starts
the **final built CLI's real `serve` command** against the owned fixture run,
and separately the compiled opt-in host using the production Rust
RunEngine/HttpServer/AccessStore/CommandLog/wallet adapters with test-only
node/miner/agent inputs. No Node/Python HTTP server substitutes for those
controller routes. All seven complete public asset responses from each server
were compared byte-for-byte with source (14 checks, not binary substring counts),
and the final CLI's actual authenticated recorded directory was exercised in
Chromium. Live wallet actions use the production Rust adapters in the compiled
host; the production CLI's real provider registry remains unconfigured until
the separate AP-3b launch hook is integrated.

The 13 browser checks cover static/public entry and private 401/403, operator
directory, keyboard/disclosure preservation across polling, user-only wallet
projection, source hash and closed error projection, exact >2^53 money and null
unknown, single worker-control projection, actual queue submission as running,
immutable unknown replay/reload, actual private `noop` result as rejected,
cross-wallet replay rejection before journal change, token rotation and old
session shutdown, legacy tokenless entry, and mobile/desktop overflow. This is
test-provider behavior, not a funded Ark proof. The current cached-body and
overlapping-login protections are embedded in these tested binaries.

Zero uncaught exceptions and zero unexpected loading failures were captured.
Expected authentication 401 log entries are retained. The Rust host completed
its stop path with exit 0; the read-only CLI and browser were stopped; the two
fixture listener ports closed; final `/proc` inspection found **zero processes**
using any sealed fixture/CLI binary or the owned browser profile. Protected lab
inspection processes, ports and data were not touched.

Final cumulative baseline-relative patch/manifest and the final evidence index
are `/tmp/opencode/live-wallet-ui-final-baseline.patch`,
`/tmp/opencode/live-wallet-ui-final-manifest.json`, and
`/tmp/opencode/live-wallet-ui-final-evidence.json`. These supersede the earlier
handoff for integration while preserving every earlier receipt. Required real
source joins remain the hash-bound AP-3b registry caller, its separately owned
private input producer, and T1's separately owned authenticated payment/catalog
projection. Nothing in this proof marks an unconfigured adapter live.

## Actual private producer arrival (doc-only update after the sealed gate)

The parent supplied `/tmp/opencode/wallet-ui-provider-interface.json` and the
694-row producer manifest, full `e97ade1235110d58b008d527518ae97fe37fcd863ebe438a383558d631d6fdb6`,
code `7119fd63576efd950e0c2f1c7f687dd7e0cbed95565992097bae89292bf79cc0`.
Its independently run receipts comprise nine provider tests, five status tests,
check, fmt and clippy; those are provider-owner results, not re-run or counted
as this UI packet's tests. The producer source row `wallet/ui_inputs.rs` was
read and verified against SHA256
`4fd63d2832c476dce0849953a35320ad4da11cd757be5b534007559acb453b04`.

No consumer mismatch or source change was needed. A source-bound compatibility
check extracted the producer's actual public serialization golden from
`ui_inputs_exact_owner_dto_above_javascript_integer_range`, checked the exact
top-level/row field order against the consumer structs, verified matching
4096-row / 1 MiB / 15-second limits and canonical values, and exercised this
UI's formatter on both golden amounts (9007199254740993 and u64::MAX).
All checks passed. This is a serialization-contract/render check, not another
Rust build or a funded provider test. Artifacts:
`/tmp/opencode/live-wallet-ui-provider-compatibility-r2/`.

The gate, binaries, 13 compiled-browser checks, original final freeze and all
earlier receipts remain immutable. Only this report changes in the r2 source
freeze; Rust/JS/HTML/CSS and all binary-bound assets remain byte-identical.
The latest cumulative patch, manifest and evidence index use
`/tmp/opencode/live-wallet-ui-final-r2-{baseline.patch,manifest.json,evidence.json}`.
The AP-3b **implementation** dependency is closed; the parent serial owner still
must integrate the tested producer and hash-bound registry caller. Real funded
provider validation remains AP-5. The T1 authenticated per-user balance/catalog
join remains separately owned and is not marked available by this packet.
