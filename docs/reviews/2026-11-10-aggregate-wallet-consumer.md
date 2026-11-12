# Aggregate wallet consumer follow-up

Date: 2026-11-10
Status: API source handoff consumed; compilation/live proof pending assigned slot

## Source binding

Sole write tree:
`/home/ethan/.local/share/opencode/worktree/ad8886/lab-aggregate-wallet`.
Bootstrap used exactly the UI provider's 694 manifest rows (11,203,298 bytes),
full aggregate `e97ade1235110d58b008d527518ae97fe37fcd863ebe438a383558d631d6fdb6`,
code `7119fd63576efd950e0c2f1c7f687dd7e0cbed95565992097bae89292bf79cc0`.
Its approved AP3 base is full
`0b0c5fd7d5b52e314a0a1aec818b13eb9d134aafa2ca93734310bed028949468`, code
`a270582ddfe2e0a20f11de5316b67b913f3a5a6404419f029633bd19b63a01a3`.

Selective ladder inputs are bound in
`/tmp/opencode/aggregate-wallet-ladder-inputs.json` to owner source
`415d21fa88ad2124712cc1fe9a8400bc7fbdd951bd005f223f92f879068f2602`.
The complete authorization codec and ladder module are exact imports. Scanner
`Found.k` and `scan_outputs` are selective imports with narrow dependency and
module wiring. Pool runtime/store sources were inspected, not imported.
No upstream pin/package version change is part of this follow-up.

## Implementation

- `wallet/aggregate_api.rs`: bounded direct HTTPS transport (explicit regtest
  loopback HTTP exception), redirects/proxy disabled, new challenge and B_scan
  signature per read, sensitive auth headers, owner identity checks.
- `wallet/aggregate_watch.rs`: existing SQLite owner/origin/order-key binding;
  immutable ordered member/anchor amounts; transactional status ingestion;
  two-phase transitions before reassignment so newest-first cancellation views
  work; durable receipt work cursor advanced before I/O. Missing status rows do
  not delete watches. Authenticated exact frozen bytes are signature-checked
  against the persisted public key, matched to the order ID/total and retained
  immutably; no unsigned order is reconstructed from status.
- `wallet/aggregate_receive.rs`: frozen order/grant envelope verification;
  exact full Pubkey policy and configured, ArkInfo-validated full server key;
  canonical unique output-set/value checks; full stock genesis and canonical
  funding proof validation; whole funding-set recheck; owned status attestations;
  one ledger/receipt/member-allocation transaction using the existing ledger
  writer. Retained verified source blocks can survive origin orphaning.
- `wallet/live_receive.rs`: independent mailbox proofs persist as candidates;
  they cannot pre-credit a partial aggregate. Checkpoints remain restart-safe.
  Revalidation and the existing UI capture use only credited records and full
  policy/server equality.
- `wallet/scan.rs`: retain all owned k/vout/tweak observations, one epoch per
  block; immutable replay checks and canonical flag restoration; frozen
  nonanchors follow their anchor; unresolved watches survive retry exhaustion.
- `wallet/mod.rs` / `config.rs`: existing wallet tick uses explicit aggregate
  API config and bounded polling; failed orders yield to later work. Existing
  command processing, stock maintenance and UI publication remain the runtime
  integration points.

## Exact runtime interface

Existing entry point: `spc-agent --config <private-agent.toml> --roles wallet`
from an `ark` feature build. Add to the actual generated private config:

```toml
[ark.aggregate_api]
origin = "https://<declared-pool-owner-api-origin>/"
order_pubkey = "<64 hex chars: pool BIP340 order/authorization public key>"
```

The client-owned narrow runtime join now supplies the actual leased SPc API
listener and configured public order key. See the hash-bound primary join in
`docs/reviews/2026-11-10-aggregate-wallet-runtime-join.md`; the serial owner must
union its attach/UI hook with current startup work. Neither value is inferred
from an Ark server key, ArkId or SV2 authority. Config omission still reports
`aggregate_api_unconfigured_watches_retained`. Canonical origins and public keys
are validated by `AgentConfig::validate` in default and ark-feature builds.

POST `/api/sp/challenge` returns `{nonce,expires_in}`; each following GET uses
`Authorization: SPc <sp_address> <nonce> <BIP340 signature>`, with the existing
`SPc/Api` digest over nonce ASCII. Reads are:

- `/api/sp/me/ark-aggregates`: `spc.aggregate-status/1` JSON.
- `/api/sp/me/ark-aggregates/<32 lowercase order-id hex>/receipt`: SPRP/1 bytes.
- `/api/sp/me/ark-aggregates/<32 lowercase order-id hex>/order`: parent-assigned
  `application/octet-stream`, exact canonical stored ArkSendOrder including its
  signature. Codec bounds are 93-348 bytes. Producer source `0515a6a8` and
  schema `ae1eaed4` are verified; HTTP 200 also requires `no-store`.
  No successful producer endpoint execution is claimed.

Private queue/status/UI hooks remain the approved interfaces:
`--wallet-command <json>` submits to `<ark.datadir>/commands`; loopback status
uses `ark.status_listen`; selectable observations use
`<ark.datadir>/ui-inputs.json` and `spc-agent-private-inputs/v1`. Pool Sent,
unknown obligations and unallocated candidates never become selectable inputs.

## Blockers and verification

1. The exact `/order` source handoff is consumed. Its fetch and immutable
   storage path match the verified producer contract; the named MIN/MAX codec
   constants are the only required codec delta. See
   `2026-11-10-aggregate-wallet-producer-r3.md` for the full source/schema hashes
   and selective SourceFeed union. This closes the source/API-shape gap.
   SPRP/1 signs a digest over the complete encoded order INCLUDING its signature.
   Missing/invalid endpoint replies retain watches and produce no credit. The
   original dependency is recorded in
   `/tmp/opencode/aggregate-wallet-api-binding-question.md`.
2. Cancelled status lacks explicit per-member no-delivery reconciliation.
   Watches remain; no inferred refund or no-delivery outcome.
3. `/tmp/opencode/aggregate-wallet-check-r1.log`: lock acquired; target/store
   availability 46,559,821,824 bytes, below 53,687,091,200 required; exit 43 before
   Cargo. Parent later reported 92.36 GiB after explicitly authorized cleanup;
   primary owns the next global heavy slot. No new build until slot notification.
   Future invocations set `CARGO_INCREMENTAL=0`, reuse `/tmp/opencode/T1a-target`,
   and retain the nonblocking global lock and inside-lock 50/20-GiB guard.
   New source has never compiled or run its Rust tests/clippy. Earlier AP3/UI
   passing counts are not evidence for these changes.

Lightweight source checks completed: Nix 1.93 `rustfmt --check` on the changed
Rust modules exited 0, with identical before/after full and code manifests;
`git diff --check` exited 0. This checks formatting/parsing, not Rust type
checking. Read-only comparison of all 694 approved UI-provider rows and all
690 approved AP3 rows found no hash, size or mode changes in those source trees.

Authored regressions cover fresh challenge use across bad receipts, wrong owner
and signer, replayed challenge rejection, insecure origins, unbatched/missing
status retention, old anchors, whole-set reorg retention, cancellation ordering,
durable fair receipt rotation, partial/duplicate/replayed/foreign allocation,
transaction rollback, eleven-page candidate continuation and all-990-k scan
replay. A2 negatives use genuinely signed stock board/arkoor proof fixtures and
a genuine grant, rejecting same-U HTLC policy and another full server key.
These are local cryptographic/SQLite fixtures, not funded-node acceptance.

After an explicitly authorized build opportunity: scoped default and
agent ark all-target check; new aggregate/auth tests; affected R1-R6, wallet
bridge/scan/UI regressions; codec/ladder/scanner tests; fmt and clippy, with
before/after source manifests and required shared build guard. The reqwest
feature/vendor closure also needs build-time verification. Funded receipt,
send/receive/refresh/offboard and outage/reorg acceptance belongs to integrated
native-funding tests against the pinned unmodified self-hosted Ark server.

## Packet

The initial 703-row and R2 706-row packets remain preserved. The current
follow-up is bound by `/tmp/opencode/aggregate-wallet-r3-manifest-{full,code}.json`,
`aggregate-wallet-r3-delta.json`, `aggregate-wallet-r3-core.patch`,
`aggregate-wallet-r3-receipts.json`, `aggregate-wallet-r3-interface.json`,
the unchanged `aggregate-wallet-runtime-r2.patch`, and
`aggregate-wallet-sourcefeed-r3-join.patch`. Compilation, serial union and live
acceptance remain pending; this is not an approved or funded implementation.
