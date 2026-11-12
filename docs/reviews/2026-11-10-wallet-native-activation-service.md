# Typed wallet native activation service

Date: 2026-11-10
Status: actual service/config/runtime source authored; uncompiled by build priority

## Preserved source and exact inputs

Verified the loaded-717 full source
`58722b20d57ee0435d6cf73c9b077a7ae42b92c66fd7bf83c8909f1b491c06b3`
before editing and archived it in
`/tmp/opencode/aggregate-wallet-loaded-717-preserved.tar.gz` with a hash receipt
at `aggregate-wallet-native-typed-baseline.json`. The prior 708 archive and
approved 0b0/e97 source trees remain unchanged.

Verified inputs are recorded in `/tmp/opencode/aggregate-wallet-native-typed-intake.json`:

- Native codec `39c75d44c8ff16cf1e08b873eca91ad2988f9da1937ef18265e62d537d8abd99`
  and tests `fc6bc8d2c86bbcff1649ed219497b3c12e1da9565b02ac4f7960b42fc5ac76a6`.
- Contract r3 `e0b72a8ae3243c0708bdd45a22cba16aac7a61ad3932453663dff1f6c72b4da3`;
  unchanged DTO/framing details are read from its r2 contract.
- Controller helper `6c887a4504d3b8c0508dc37b2ea3d94b4403761ecafefde9babd421b4529302e`.
- Controller current-primary patch `d1ef48deb6fd6b063ddc7d55900899def4154aef614ce58b5108af415e3a602b`.
- Shared consuming auditor remains exact source `edebd11e...`, interface
  `981e4dbf96d49e2c6544832bdbb1f860d962eae2a87c723e2c069e0f39465971`.

No CPU implementation files were overwritten in this wallet tree. The serial
integration still includes the native owner's 33-file CPU gate packet.

## Actual implemented path

`config.rs` adds first-class optional `ark.native_audit` and records skipped,
non-deserializable loaded-file path/hash provenance. The wallet role validates
that provenance against its actual --config argv/file before publishing a
native instance. Profiles require regtest, explicit scope and independent
canonical public source/collector keys.

`wallet/native_audit_io.rs` reads private regular non-symlink/non-hardlinked
files with bounds and change checks, atomically fsyncs lifecycle publication,
and captures kernel PID/UID/start ticks/boot ID/executable SHA256, the selected
run-token hash and actual --config bytes/path. Start ticks are checked before
and after observation; peer network namespace must match the wallet.

`wallet/native_audit.rs` is the concrete service, authorizer, handler and
wallet-owner signer. It uses the exact CandidateRequest and PeerRegistry types.
Authorization happens before the consuming auditor. No paths, keys or observed
process data are accepted from a request. Unknown/foreign/stale requests yield
only typed Unknown or closed transport; private diagnostics are not returned.

The shared auditor receives the unchanged request nonce and actual ReceivedJob.
Its verified public SourceFeed v2 evidence is passed to the wallet owner through
a bounded typed channel. The owner independently checks evidence/full job ID,
computes `native_activation::activation_digest`, signs with the existing B_scan,
erases the ephemeral keypair, then uses shared `verify_response` before caching
and release. The service rechecks process/registry/instance/candidate/expiry
after completion and immediately before sending Approved. No raw digest signing
endpoint exists.

The replay table retains each connection's latest generation/session/epoch.
Only identical completed requests can return identical cached framed bytes,
bounded by min(request expiry, checkpoint expiry). Live nonce rebinding and
same-epoch mutations reject. Supersession signals cancel a waiting auditor;
client disconnect also cancels. A fresh lifecycle always generates a fresh
instance ID; a live previous instance is never replaced. Only a proved dead,
matching previous instance/socket may be reclaimed. Ambiguous orphan artifacts
fail closed. Stop revokes publication/candidates before bounded executor join
and removes only matching lifecycle files; the controller registry remains.

`wallet/mod.rs` starts this concrete service under the existing wallet writer
and concurrently drives its short construction/signing owner loop with the
existing wallet work loop. Source verification stays on the bounded audit
thread, and signing yields between requests. Maintenance/commands keep their
existing ledger/reservation/round machinery. No fifth wallet, extra seed,
long-lived auditor clone, CPU scan-key file or translator is introduced.

## Runtime rendering and serial hooks

Each declared wallet actor can include this **operator-owned** optional profile:

```json
"native_audit": {
  "collector_socket": "collector.sock",
  "pool_pubkey": "<actual source-trace public key>",
  "collector_pubkey": "<independent collector public key>"
}
```

The relative socket resolves under that run's private root; an absolute socket
must remain inside it. Parent ownership/canonical path is checked; an existing
socket must be private/owned. An absent collector later produces Unknown, never
approval. Keys come from the declared source/collector configuration, not Ark
server or aggregate-order keys. No private signing key is opened by the renderer.
The renderer supplies run_id/pool_id/front_id/identity_id from actual scenario
mapping and writes all seven `ark.native_audit` fields into the existing agent
config. Lifecycle filenames derive exclusively from actual ark.datadir.

The composite runtime patch combines the native owner's exact engine hooks/
helper, the current-primary wallet renderer and the existing aggregate/UI
binding join. It adds read-only native attach validation when that profile is
enabled; owner-provided adoption and pre-AddConnection peer publication remain
intact. No TCP listener key represents this Unix lease. The native owner remains
responsible for direct-SV2 CPU gate/first-hash ordering and the explicit unaudited
smoke capability. The final audited scenario must supply this profile with real
collector/pool authorities. No fake successful values are installed here.

## Authored checks and remaining execution

Added tests cover exact replay, epoch/nonce mutation, supersession, public
profile confinement, instance/socket replacement, actual unregistered CPU
credentials and stale instance rejection before auditing. The typed service
test runs the actual consuming full-owned source verifier against an explicitly
mocked complete empty-history collector, checks SourceFeed v2 consumption and
COMMITTED, then checks the real loaded-wallet native signature with the shared
public verifier. CPU peer facts come from a real Python subprocess; wallet
startup metadata in that fixture is synthetic. It proves no funded acceptance
or actual native worker hashing. Existing concurrency tests exercise the real
private command queue/SQLite reservation while an audit stalls.

No Rust compilation, test, clippy, live wallet or native actor was executed in
this step. Source formatting/patch checks are recorded separately. All tests
remain authored until the assigned global slot; use CARGO_INCREMENTAL=0, the
existing T1a target, nonblocking lock and inside-lock 50/20-GiB guard. Required
integrated checks still include actual wallet-instance bootstrap/restart/attach,
native CPU zero hashes before COMMITTED, full-Q attainment and funded native/
stock-Ark lifecycle acceptance.
