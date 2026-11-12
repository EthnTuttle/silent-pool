# Live lab token bootstrap and private wallet adapter

Date: 2026-11-09
Scope: lab controller/browser only. SPEC body derivation, Ark order/receipt
privacy, stock server interfaces and pool operator listener are unchanged.

## Decision

The public static bootstrap gains exactly `/access.js`. It contains no run,
principal, status or credentials. Root and hub prompt for the existing run
token. Authenticated health then `/me` establish run/principal/capabilities.
Legacy health without Bearer continues to establish a legacy session.

Tokens and retained unresolved commands use sessionStorage keys scoped by run
and principal on the controller origin. Explicit login, logout, revocation or
context mismatch clears the previous context, requests, stream and DOM.
Reload restores pending metadata only after authenticating the same context.
Private SSE uses fetch Authorization, a bounded incremental UTF-8/event parser,
last-event-id, multiline data and authoritative resync; no token URL exists.
Replay and resync server boundaries recheck revocation, and a changed user
claim closes its old-scope stream.

Expanded `GET /api/lab/v1/runs/<run>/wallets` is authenticated. It filters the
server's immutable wallet registry by identity AND pool before provider I/O;
the operator may read all entries. Registry entries are installed only by the
run's agent launch hook using declared scenario identities/fronts and the exact
configured datadir/listener. The baseline without AP-3b registration has an
empty registry, reported `not_configured` with no invented funds.

Agent status is allowlisted and u64 amounts become decimal strings before
browser serialization. Raw error text is replaced by a closed code. A separate
owner-private input provider is required; the ordinary unauthenticated agent
status endpoint does not gain VTXO identifiers. The proposed input DTO and
source-owner integration are recorded in the implementation report.

Typed `wallet_refresh`, `wallet_offboard`, and `wallet_read_inputs` commands
carry an identity selector, never a path. User scope requires the matching
identity/pool and explicit `wallet_refresh`, `wallet_offboard`, or
`view_own_status` capability respectively before journal lookup. Newly generated
user claims include the two maintenance capabilities; existing claims are not
rewritten. Inputs are explicit, unique and present in a fresh private provider
snapshot. Each action requires a homogeneous input rail; offboard supports
both direct derived-SP and stock-normal inputs, matching AP-3b, and requires a
checksum/network-validated native regtest witness address. The agent remains final owner of fee/policy and
input-state validation. No estimated fee is manufactured by the UI.

The adapter writes only the existing AP-3b owner-private queue shape, with
create-new/private permissions, no-replace publication and directory fsync.
Operation IDs are deterministic over a domain tag/run/actor/command ID; the
existing command journal binds the immutable payload before this effect.
Queue submission is `running`; errors that may follow publication are
`unknown`. Same-command checks read only the owner's operation result, never
resubmit. Only matching `round_confirmed` or `offboard_confirmed` completes;
`stock_submitted` is still running. Unknown remains retained across reload.
The controller includes the SHA256 of the exact validated private input-file
bytes with their source timestamp; no additional producer DTO field is required.

## Rationale

This completes browser entry into the approved existing claims system and
keeps the user's SP/Ark wallet, watch-only coinbase outputs, T1 test-payment
wallet and background wallets distinct. Missing source contracts stay explicit
dependencies. No browser node RPC, credential bridge, new master key, generic
relay or Ark server modification is introduced.

## Sections/files

- `docs/changelog/2026-11-09-live-wallet-ui.md` is the one-bullet SPEC
  frontmatter fragment for serial integration; candidate SPEC is baseline-exact.
- `crates/spc-lab/src/{access,http,engine,command,wallet_projection,wallet_control}.rs`.
- `bins/spc-lab/static/` bootstrap, hub and controls.
- `docs/reviews/2026-11-09-live-wallet-ui-implementation.md` for actual evidence.
