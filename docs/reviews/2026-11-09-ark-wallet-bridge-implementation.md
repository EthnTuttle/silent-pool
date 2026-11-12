# AP-3b wallet adapter implementation checkpoint

Date: 2026-11-09
Status: compiled, scoped tests and owned unfunded Nix runtime passed; funded acceptance NOT RUN

## Current implementation (supersedes the initial checkpoint below)

The initial freeze was preserved before compile fixes in
`/tmp/opencode/AP3b-r0-archive/`: all 23 changed source rows plus its full/code
manifests, baseline delta, patch and receipts (1,027,353 bytes; no full source
snapshot). Its full aggregate remains
`87422db9fa57bb8f291b9329e6479cb969d53cca81ddca10d71389305893a1ac`.

The assigned `/tmp/opencode/T1a-target` is reused under nonblocking
`flock -n -o`. Every invocation checks the actual filesystem inside the lock
(observations approximately 91-100 GiB free), with at least 20 GiB projected
residual. No cache copies, cleanup, pin changes or registry ownership changes.
Compiler and Cargo are the verified Nix 1.93.0 toolchain at
`/nix/store/ncmxf65s36g51qsdcx5sw0wc0fwcph2b-rust-default-1.93.0/bin`.
The existing vendor directory is read-only; every workspace locked package is
present with matching package checksum and the exact Bark git revision.
`/tmp/opencode/AP3b-cargo-home/config.toml` supplies that configuration to
Cargo subprocesses as well as the outer command.

### Source now implemented

- Explicit `Scenario.wallet_agents` records, one writer per Ark mining identity
  and pool, validated before runtime preparation. Legacy random identity JSON
  cannot become BIP85 provisioning. Cold provisioning executes in a separate,
  bounded parent-death-protected process; the controller retains public metadata
  only for those identities.
- Wallet status ports use the existing resource allocator. Supervised wallet
  launch occurs after fronts/Ark and before miners. Initial `AddMiner`, dynamic
  `AddMiner`, and dynamic `AddConnection` use the same owned-process plus fresh
  committed-registration predicate. The hook does not activate any miner and
  therefore composes with a deferred T1 funding prerequisite.
- Native extension negotiation precedes channel opening. Every live registration
  reply produces a bounded status lease; a running process, historical Applied,
  Superseded, timeout, or stale lease cannot satisfy admission.
- Canonical full-block scan history and journaled reorg rewind, replayable epoch
  mailbox read schedule, retained proof bytes, and repeated receipt/anchor/status
  validation. Unknown live proof masks spendable/refresh aggregates rather than
  turning missing data into zero. The live role currently refuses non-derived
  mailbox modes, so it cannot register a delivery mode it does not consume.
- Owner-only command queue and `--wallet-command FILE` CLI submission; strict
  shape, immutable operation id/payload, explicit owned inputs, duplicate-input
  rejection, private atomic files and durable input reservations.
- Direct SP-owned delegated hArk refresh over standard SubmitRoundParticipation,
  RoundParticipationStatus, RequestLeafVtxoCosign, RequestForfeitNonces and
  ForfeitVtxos. Existing ark-lib leaf/forfeit helpers perform signing; output
  amount, policy, unlock hash, funding inclusion and exitability are checked
  before forfeiting inputs. Normal outputs use the stock child-wallet key chain
  and stock import, with no alternate signer or balance ledger.
- Direct SP-owned cooperative offboard over standard PrepareOffboard and
  FinishOffboard, stock OffboardForfeitContext, real fee-rate/fee discovery,
  deterministic persisted request, replayable signing, stable transaction
  broadcast and canonical confirmation proof. It does not require an extra
  arkoor hop. Ordinary child-wallet inputs use the stock maintenance APIs.
- Actual stock available-VTXO proof/status validation before combining its
  balance with the SP ledger. Pending/unknown operations remain explicit;
  a lost initial round response without an unlock hash remains unknown because
  the standard server has no client-operation-id lookup. No fabricated lookup,
  blind resubmission or success-from-process-liveness exists.
- Nix tools closure includes the Ark-feature agent. The expanded scenario
  `lab-ark-wallets.json` declares four identities, wallet writers, miner processes
  and workers, with 8 hashes/sec total; transaction-traffic wallets are separate.

### Compiled/tested evidence

`AP3b-check-r6.log`: agent all-target check and **142 passed, 0 failed,
1 ignored** (the frozen-fixture regeneration test); scan rewind/restart,
height-only cursor refusal and private command idempotence tests passed.
Infrastructure tests: 4 passed; lifecycle tests: 3 passed. The new scenario
initially exposed missing explicit miner-to-front links; these were added.
`AP3b-check-r7.log`: wallet bridge tests 3 passed (four-count/hash-budget axes,
missing/duplicate/wrong-pool writer rejection, and stale/process-only readiness).
`AP3b-check-r8.log`: agent all-target clippy passed. `AP3b-check-r9.log`: lab and
lab CLI all-target clippy passed. Agent formatting passed in the same sequence.

Captured and fixed errors: incomplete offline index (use complete read-only
vendor source); Cargo subprocess source configuration; three API trait/import
ambiguities in the new maintenance adapters; command-test directory mode;
explicit scenario links; and bounded clippy findings. No assertion was removed
to make these checks pass. Every log/exit file is retained under
`/tmp/opencode/AP3b-check-r*.{log,exit}`.

Nix source ingestion is filtered directly into the store (11,099,156 bytes,
686 files at the first attempt), excluding Git, target and generated caches.
No extra `/tmp` full source snapshot is made. Initial Nix attempts exposed a
getFlake string-context issue and the expected stale vendor lock/hash from the
two new dependency edges; both are tracked in `AP3b-nix-tools-r*.log` and the
vendor probe receipt. The pinned package versions remain unchanged.

### Integration boundaries and remaining acceptance

Shared merge hunks: agent config/main/SV2 helper and Cargo edges; lab Scenario,
resource allocator, Engine startup/dynamic guards, RuntimeOptions/cold identity
loading/startup hook, CLI option and AP-4 infrastructure precondition; Nix tools
package/launcher. T1's core signer/journal and node/server implementations are
untouched. T1 must retain its funding barrier before its eventual call to miner
activation; that call then also encounters this wallet readiness predicate.

The owned zero-miner fixture script is
`/tmp/opencode/AP3b-unfunded-fixture.py`. It uses immutable Nix binaries and fresh
private run storage, and does not access the protected inspection controller.
Actual fixture outcome is recorded separately when run. Funded receive,
refresh, offboard and native witness-spend acceptance remain **NOT RUN** until
T1/T2/DEST/T4 supply real canonical funds. Compilation and unit tests do not
substitute for that acceptance.

### Nix and owned runtime result

Nix tools package passed its 20 infrastructure/lifecycle/inventory/adoption/
isolation tests and installed the Ark-feature agent:

- Output: `/nix/store/w52v4f6sr6pz32410y9la8sjkk02adga-spc-lab-ark-tools-0.1.0`.
- Deriver: `/nix/store/bgzns420bafnhlbv2yhhjzng5yl4gs5q-spc-lab-ark-tools-0.1.0.drv`.
- Agent SHA256: `3fa96baa52a9a800b9ed0f96431be0d68714868b1d6f9fee84aa42d3db5526e5`.
- Lab CLI SHA256: `e0f5897b5405a2a3b10887473c32e02c843e0f6be20f3ff6275153dd0a349e2a`.
- Actual build source: `/nix/store/vm95bb22061a626nd1si133xc9prjz0r-source`;
  every non-Markdown ingestion row was rehashed against it with zero mismatches.
- Correct regenerated vendor-staging hash:
  `sha256-Sz5lzJn7U6MoYveR6n71u06h9Q0ux9DHpodJcL0HrhQ=`. The earlier outer-vendor
  probe hash is diagnostic only and was not adopted. Package versions and git
  revisions did not change.

Owned runtime:
`/tmp/opencode/AP3b-owned-unfunded-20261109T044603Z/run`.
Controller exit 0, node height 0, **15 declared processes, all stopped**, and
zero matching owned processes still alive (PID/start-time checked).
Counts: one node, two fronts, two pool-Ark roles, one stock captaind, three
Postgres processes, two Redis processes, and four wallet agents. Zero miners,
zero transaction-traffic wallets, zero generated coins.

All four wallets reported `connected`, `registered=true` and an unexpired
registration lease. Their SQLite stores retained request id 1 as effective,
no pending request, and the real subsequent `already_applied` outcome. Stock
Bark properties contain regtest, distinct child fingerprints and actual stored
server/mailbox keys. Every checked key/database file is 0600 and wallet/cold
directory is 0700. VTXO record counts are zero; spendable, pending and refresh
balances remained null rather than fabricated funding proof.

The first owned attempt failed safely because the fixture asked the node for
a pow-limit overlay that was not harder than its attainable target. The
fixture was corrected to the already supported `0x200fffff`; no node code or
policy was changed. Its failed run/log/receipt remain intact. No protected
inspection controller, port or archive was used.

Receipts: `/tmp/opencode/AP3b-nix-binaries.json`,
`AP3b-nix-source-binding.json`, `AP3b-nix-tools-build.log`,
`AP3b-unfunded-fixture-receipt.json`, and
`AP3b-unfunded-state-verification.json` (all under `/tmp/opencode`).

### Parent HTML/component/API facts

Default expanded declaration: four SP identities, four wallet writers, four
miner processes, four workers, 8 configured hashes/sec, two pools and one local
provider. Wallets do not hash and do not create implicit traffic wallets.
Runtime publishes four private loopback `/status` listeners and four owner-only
command queues. CLI cold provisioning and command submission accept paths and
public command data; no seed/xprv is in argv or public JSON. Registration uses
the native Noise/SV2 `0x5350` client. Receive, refresh and offboard use ordinary
stock Ark RPCs, with SP ownership/signing confined to the wallet adapter. No
browser-to-node secret proxy or custom Ark-server API was added.

The Nix/unfunded fixture proves opening, registration, retry acknowledgment,
private files and cleanup. It does not prove funded mailbox delivery, refresh,
offboard, mining admission under real funding, or T1/T2 integration. Those
acceptance cases remain explicitly pending; the corresponding source paths
are implemented, compiled and fail closed.

## Initial checkpoint history (retained, superseded by current section)

## Source layer

Sole-owned worktree:
`/home/ethan/.local/share/opencode/worktree/ad8886/lab-ark-wallet-bridge`.
Git base: `b0ab4ca34fda07c61a6335d05f598eaacc69678c`.

Bootstrap input: `/tmp/opencode/self-hosted-ark-packet/manifest-full.json`.
Verified all 671 file rows against `lab-self-hosted-ark`: byte length, SHA256,
regular-file/non-symlink status and exact mode. Verified aggregate
`6529a8f22e808469889a422a2ab51e2950cc308ef520d2e9636245374d989460`
with SHA256 of sorted-key compact JSON containing `base_manifest_sha256` and
`files`. Imported only those listed files (10,939,547 bytes), without a second
source snapshot. Baseline manifest retained as
`/tmp/opencode/AP3b-baseline-manifest-full.json`.

## Implemented source

- `wallet/live_bark.rs`: stock Wallet::open using the existing validated
  SqliteClient, child seed and chain cookie; actual server/mailbox pins checked
  before stock first-use persistence; stock fee validation and heartbeat.
  No new wallet database or generated identity replaces the AP-3a material.
- `wallet/live_registration.rs`: native Noise/SV2 extended-channel connection,
  real extension negotiation and canonical AP-2 signed request codec. Responses
  bind request id, ArkId and nonzero revision. Superseded is not Applied;
  commit-unknown stays retryable. Epoch-derived requests require D.
- `wallet/mod.rs`: online entrypoint opens stock Bark and drives the real
  registration exchange with durable reservations before wire, AP-3a terminal
  policy, bounded timeouts and fresh resolution every 15 seconds. Reconnect
  revalidation uses the original operation counter. The offline seam also
  no longer declares connected solely from a prior stored acknowledgment.
- `wallet/state.rs`: acknowledgment identity/payload binding, mismatch clears
  the contradicted binding, revalidation reservation, raw-receipt/checkpoint
  metadata in the existing SQLite file. No second balance ledger.
- `wallet/live_receive.rs`: callable epoch-derived ReadMailbox adapter using
  canonical coinbase-derived owner/mailbox keys, stock proof validation,
  canonical anchor inclusion and owner-signed spend-state attestation. Actual
  expiry and full 36-byte outpoint IDs are persisted. Checkpoints follow proof
  and ledger commits; replay before checkpoint is deduplicated.
- `wallet/provision.rs` and main CLI: separate one-shot cold regtest provisioner
  (`--provision-wallet PATH --cold-store PATH`). Returns only public metadata.
  Online key files retain AP-3a's exact derivation; complete restart validates
  them and partial provisioning refuses regeneration.
- Exclusive datadir writer; a missing `bark/db.sqlite` after recorded Bark
  initialization now refuses even when its containing directory still exists.

New agent `[ark]` config fields: `server_address`, `server_pubkey`,
`mailbox_pubkey`, `chain_rpc_url`, `chain_cookie`. All secrets remain in files;
the server keys are actual public ArkInfo values. Wallet status stays loopback
and unknown balances remain null until receipt evidence exists.

## Checks and blockers

- Bootstrap row/mode/recipe verification passed.
- Rustfmt parser/formatting was run on modified Rust sources. This is NOT a
  compilation or test result.
- Initial disk observation: 107,451,584,512 bytes available on the target/store
  filesystem; `/tmp/opencode/T1a-target` occupies approximately 12 GiB.
- Proposed shared-cache assignment: `/tmp/opencode/T1a-target`, 5 GiB estimated
  incremental growth. Assignment has not been received. No cargo or Nix build
  or test was started, and no heavy-build lock was acquired. The next invocation
  must respect the reservation file, take nonblocking flock, then recheck space.
- New unit/integration test source covers required D and signed counter,
  fake/foreign replies, Superseded, commit-unknown, restart revalidation,
  changed-payload acknowledgment rejection, cold provisioning tamper/partial
  state, and proof-metadata replay. These tests have NOT RUN.

## Remaining implementation, explicitly not claimed

1. The lab's initial and dynamic `ark_agent_bridge_unavailable` predicates
   remain in force. Explicit wallet-agent declarations, per-identity public
   mapping, resource leases, supervised launch and fresh-readiness barrier are
   not implemented by this checkpoint. No miners were enabled.
2. `receive_epoch` is a callable verified receive path, not yet connected to a
   live scan/reorg scheduler or a private command listener. Static/none receive
   and registered-epoch windows are not implemented here.
3. SP-owned refresh/offboard, durable outgoing operation reconciliation and
   stock-key normalization are not implemented. No fake maintenance entrypoint
   or unsupported-key success is exposed.
4. The two new optional Cargo dependency edges use already pinned packages;
   the Nix vendor hash must be regenerated/verified before a Nix build. No pin
   was changed. No Nix agent package is claimed from this checkpoint.
5. No owned live fixture was started. Stock-wallet opening and AP-2 registration
   have not been exercised by this packet. Funded receive/refresh/offboard:
   NOT RUN; requires T1/T2/DEST native funding and actual spends.

## Merge notes and downstream report facts

Transfer only AP3b delta hunks relative to the verified 671-row bootstrap, never
the whole layered tree. Shared files touched are agent Cargo.toml, Cargo.lock
(dependency edges only), agent config/main and the wallet-only accessor in
sv2_client.rs. No lab Runtime/Engine/T1/T2/AP4 source hunk exists yet, so this
checkpoint cannot replace their startup barriers. SPEC body and changelog
fragment are separate for serial merge.

Funds-flow facts: the client derives SPc ownership locally; registration secrets
go only through authenticated mining transport to the pool front; Ark receives
ordinary mailbox authorization, outpoint proofs and spend attestations, with
no sp_id, scan key, SP spend key, pool block hash or attribution metadata. Raw
receipt metadata remains client-side. A full Wallet::open is not funding.

No primary/other-worktree/frozen-input edits, commits, subagents, pin changes,
cache cleanup or inspection-controller interactions were performed.
