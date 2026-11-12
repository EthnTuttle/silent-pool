# Live wallet UI audit corrections (W1-W4)

Date: 2026-11-10
Owner: `ses_e3c670affffeBwvH6vNd799DJY`, `lab-live-wallet-ui` only.
Review: primary `docs/reviews/2026-11-09-live-wallet-ui-review.md`, including
the two-layer W1 addendum and the parent's transient-observation clarification.

**Final status:** W1-W4 corrections and the latest append/torn-tail guard are
compiled and tested. Audit-r5 replay/journal/idempotency tests, fmt, clippy and
latest CLI/host builds all exited 0. The unchanged browser assets retain the
15-workflow audit-r2 proof; the latest CLI's seven complete static responses were
also byte-checked. Source-bound narrow engine tests cover the new guard.
Final handoff uses `live-wallet-ui-audit-final-*` under `/tmp/opencode/`.

## Audited input and preservation

Verified all 693 rows of the audited r2 input before editing: full
`19aa61a04672b63a1cd928fedda299d8e78d93a99f29397572eefcbc206ee0c6`,
code `0f559b5d8cd4c423d08e787944e5d3a87af821cd429d1957a61c5564d8a0d4f1`.
Only planned changed source rows (335,698 bytes) and small receipt/manifest
copies were archived under `/tmp/opencode/live-wallet-ui-audit-before/`.
The prior CLI `363dbb19...`, compiled host, 13-check browser proof, final r2
manifest/patch and all previous receipts remain untouched.

## Disposition

- **W1, both user and operator:** current actor/capability/identity and
  scenario-derived pool scope run first. An actual wrong registry pool is
  denied; registry absence no longer means wrong identity. The scoped journal
  then distinguishes new intent from existing operation. New unbound requests
  reject without queueing. Existing Running/Unknown operations observe Unknown
  with the original ID when unbound, without a new Finish or journal rewrite.
  Read-only attachments can return a matching stored terminal result; otherwise
  their observation cannot cancel old funds. Retry denials without durable
  resolution evidence retain the browser envelope.
- **W2:** `supersede_nonterminal` reuses the existing Finish record and the
  reader's existing last-Finish behavior. Scope/payload are checked under the
  log mutex, terminal outcomes are immutable, concurrent checks produce one
  effective update, and same outcomes do not append. Only actual agent terminal
  observations are promoted on replay; transient Unknown/Running observations
  do not overwrite the durable initial outcome. File sync precedes RAM update;
  write/sync failure reports Unknown rather than a false durable Success.
  Durable changes emit uniquely keyed command evidence and refresh summaries.
  Legacy `finish()` and unchanged-command bytes are retained.
- **W3:** a bounded 16-entry pending-envelope set replaces the single global
  envelope. Worker pause/resume/budget may proceed while funds are Running or
  Unknown; every unresolved envelope remains immutable across same-context
  reload. Resolution clears only its entry. A pending funds envelope still
  blocks another funds submission. The selector and retry button identify the
  actual retained command, not whichever unrelated request finished last.
- **W4:** operator wallet shape/network validation now runs before the operator
  shortcut. Hard-link failure removes only the current call's acquired UI temp,
  preserving commands/results. Exact private 0700/0600 requirements remain and
  are documented. The nonblocking WebAssets coupling cleanup is explicitly
  deferred; static routes/CSP and byte-equality proof remain in scope.

## Registry reconstruction handoff

The new `bind_persisted` consumer is tested in this tree. It verifies private
agent/front config against declared identity/front/pool and persisted listener
identity, checks matching public provisioning address and initialized private
state-directory metadata, and then installs the registry binding. It neither
spawns/provisions nor opens/queries SQLite or rewrites any wallet/key/config.

The exact serial patch is
`/tmp/opencode/live-wallet-ui-audit-registry-join/registry-launch-and-attach.patch`.
It adds one shared `wallet_bridge::rebind_ui_registry` caller after launch and
after `attach_internal`'s adopt/read-only setup. Failed reconstruction leaves
the provider unbound and safely Unknown for unresolved operations. It does not
abort unrelated worker controls or fabricate a binding. Frozen provider inputs:

- `wallet_bridge.rs`: `d33dec30ae192087590864901f740c34802f91de5a42349742086d5838706784`.
- `engine.rs`: `e7665bd451f8cecabb51de97ded6eec78153ce66e16c646b9c879bb87e0df920`.

Both frozen inputs, joined outputs and hashes are retained in that directory.
No AP-3b source tree was edited or copied into this candidate. The serial owner
must union these caller hunks with the UI engine hunks; the real provider join
is not claimed by a fixture bind. The separately tested AP-3b private input
producer remains implementation-ready, and the separate T1 projection/data join
is not manufactured by this change.

## Verification record

The assigned existing primary target is used as cache only, always with
nonblocking `flock -n -o`, pinned Nix Rust/Cargo 1.93.0, inside-lock target/store
space checks (>=50 GiB, 5 GiB growth, >=20 GiB residual), and source hashes before
and after each step. Runner: `/tmp/opencode/live-wallet-ui-audit-gate.sh`.

- Audit r1: acquired with 70,455,934,976 bytes available. Five new integration
  regressions passed: user/operator actual HTTP queue-submit -> attach with empty
  registry -> Unknown, immutable queue/journal on unavailable rechecks;
  concurrent durable terminal resolution/restart; malformed operator rejection;
  publication-temp cleanup; read-only persisted registry reconstruction and
  config/resource mismatch rejection. One journal unit test passed both actual
  write failure (`/dev/full`) and sync failure (socket-backed file descriptor),
  retaining Running and refusing a false durable Success. Logs/source bindings:
  `/tmp/opencode/live-wallet-ui-audit-r1/`.
- Node regression suite: 36 passed, including pending wallet plus independent
  worker resolution/reload and denial-retention. Receipt:
  `/tmp/opencode/live-wallet-ui-audit-node-r1.log`.
- Audit r2: 57 affected wallet/access/SSE/protocol/replay tests passed. Package
  fmt, clippy (`spc-lab`/`spc-lab-cli`, all targets, `-D warnings`), CLI build
  and compiled browser-host build passed. Source hashes before/after every step
  matched. Sealed r2 CLI is `b6aaeace943b699ef5db7150adac2122c81e6f4ea2dc97f3095b05b58943dca6`;
  host is `ae5a44b84cd9c3ee9125e93baa129804218416822ccf9573c526254c25e65f7b`.
- Post-r2 failure-path review found that a torn tail surviving reopen must also
  keep wallet appends closed. A small wallet-only journal availability guard now
  blocks new wallet intents/resolutions on such a reopen, while leaving the
  existing legacy reader and its bytes unchanged. The new test verifies no new
  queue effect or journal write; W1's HTTP test also now explicitly exercises a
  changed wrong-pool claim and wrong identity. These additions are newer than
  r2 and are not attributed to its binary.
- Audit r3 attempted the required narrow recheck/build for those additions, but
  its nonblocking lock acquisition returned **42 (busy)**. No wait or poll was
  performed, and no subsequent build attempt is made without a released/assigned
   slot. At that checkpoint the latest guard/test additions remained blocked;
   the parent-authorized r4/r5 checks below subsequently verified them.
- Compiled browser checkpoint: all **15** targeted workflows passed against
  the sealed audit-r2 binaries, including the original 13 workflows plus W3
  worker pause during a running wallet operation and W1 missing-registry
  retry/reload without journal/queue rewrite or envelope loss. All 14 static
  responses were byte-exact; no uncaught exceptions, unexpected loading failures
  or owned process survivors were found. Evidence is in
  `/tmp/opencode/live-wallet-ui-audit-browser-r1/`. The current browser assets
  equal those tested bytes. `scope-and-cleanup.json` explicitly identifies the
  latest torn-tail guard/test delta not present in that binary; this checkpoint
  is not represented as a compiled proof of that later delta.

The preserved, verification-incomplete checkpoint freeze was
`/tmp/opencode/live-wallet-ui-audit-current-manifest.json`, with incremental
`live-wallet-ui-audit-from-r2.patch`, source/log hashes and the separately bound
registry join. It was marked verification-incomplete because the latest guard
needed a new build slot. The parent explicitly assigned a new nonblocking
attempt, and the final checks below close that checkpoint. Its old manifest
stays unchanged.

## Latest narrow verification closure

- Audit r4: the parent-authorized narrow gate passed six replay/registry tests,
  the write/sync-failure unit test, and 30 affected access/wallet/idempotency
  tests (37 total), plus fmt, clippy and CLI/host builds. All per-step source
  before/after hashes matched. This included the actual compiled engine test
  proving a torn tail on reopen cannot create a queue file or append an intent,
  and returns Unknown with the original ID.
- Verification of the live append-failure case then identified one missing
  latch: a failed initial Begin (or legacy Finish) append could leave a partial
  record without setting the wallet-unavailable flag. The existing append
  primitive now latches that wallet-only flag on any append error, and new
  wallet Begin writes are refused while it is set. Wallet Begin errors return
  Unknown. No record format, legacy error return or legacy reader repair was
  added. A focused unit test proves both scoped and legacy Begin write/sync
  failure prevent a second wallet intent from writing any bytes.
- Audit r5 closed that final guard: **six replay/registry integration tests,
  two journal failure tests and six command-idempotency tests passed (14)**.
  Fmt, clippy (`-D warnings`, all targets for the two affected packages), latest
  CLI build and compiled browser-host build all exited **0**. Source binding
  before/after every step matched. Logs and frozen binaries are in
  `/tmp/opencode/live-wallet-ui-audit-r5/`.
- Latest CLI SHA256:
  `98a03d2eb6b1fa1aca0356058d098cdd2844d1b78d516b7ab273ebb695f334d8`.
  Latest compiled host SHA256:
  `ab4537f0e376d34f0ec3a58dd9ca999d9db7637802bd94ef4e05290755b2de91`.
  These are new sealed copies; prior r2/r4 binaries and receipts remain intact.
- The 36 Node tests and 15-workflow browser suite were not rerun: their assets
  are unchanged. The new guard is verified by the compiled engine tests. A
  bounded latest-CLI `serve` check on an owned stopped UI fixture verified all
  seven full HTTP asset bodies against source and the tokenless private-wallet
  401; its source before/after matched and its CLI process exited. Receipt:
  `/tmp/opencode/live-wallet-ui-audit-latest-cli/proof.json`. No protected
  inspection or real wallet was used.

Registry handoff stays exactly hash-bound in
`live-wallet-ui-audit-registry-join/manifest.json`. Wallet bridge input/output
hashes are `d33dec30...` / `dc6b6fe9...`; engine input/output hashes are
`e7665bd4...` / `75201046...`; exact patch SHA256 is
`b70db204c61ccbbad12c5c4bcd903f67e252b8c90f4758d7af91d6416fd0da9b`.
These hashes describe the separate caller patch on the real provider base,
not a claim that another worktree was edited or that the serial join has landed.

The fully tested final manifest, incremental patch from audited r2, and evidence
index are `/tmp/opencode/live-wallet-ui-audit-final-manifest.json`,
`live-wallet-ui-audit-final-from-r2.patch`, and
`live-wallet-ui-audit-final-evidence.json`. Their explicit recipe records full
and code hashes, the latest binaries, exact source bindings and inherited browser
asset proof. This is the fixed source to dispatch to the same auditor.

No funded workflow is claimed. The compiled browser host uses explicit synthetic
provider inputs through real controller/auth/journal/queue code. Actual funds
still require the corrected AP-3b provider/runtime join, T1 data join and native
confirmation evidence. No protected inspection process or data, primary source,
other worktree, upstream pin, final HTML report, commit or shared cache cleanup
is part of this correction.
