# Lab wallet replay observation and durable resolution

Date: 2026-11-10
Scope: lab-only W1-W4 audit corrections; no pool payout, wallet signer or Ark
server change. SPEC body is unchanged; serial frontmatter fragment is in
`docs/changelog/2026-11-10-wallet-ui-replay-resolution.md`.

## Decision

Current claim actor, capability, identity and scenario-derived pool scope are
checked before any outcome lookup. An actually wrong registry pool is denied;
an absent registry is an unavailable provider, not evidence of wrong scope.
The scoped journal distinguishes new requests from existing immutable intents.
New unbound commands reject without a queue effect. Rechecks of already
recorded wallet operations with an absent provider return Unknown and retain the
original ID and payload, for users and operators alike. Such transient reads
do not append a Finish or rewrite the initial Running outcome.

An authenticated read-only attachment may replay a scope/payload-matched stored
terminal wallet outcome without mutation. An unresolved operation stays Unknown
there: an observation denial cannot prove that the original effect was cancelled.

Actual kind-matched agent terminal results are persisted by an explicit
`supersede_nonterminal` operation over the existing append-only command journal.
It checks scope and payload under the journal mutex, appends the existing Finish
record only over an absent/Running/Unknown wallet outcome, syncs before updating
RAM, and makes terminal outcomes immutable. Identical rechecks do not append;
concurrent engine checks produce one effective update. The existing reader
already takes the last Finish, so no new log format, ledger or blanket legacy
reader rewrite is introduced. The legacy `finish` path and unchanged-command
Begin/Finish bytes retain their prior behavior. Write/sync failure returns
Unknown, retains the ID, and prevents further uncertain wallet-resolution
appends until reopen. Updated durable outcomes also update command evidence and
the summary; transient provider failures do not become terminal evidence.
If reopen encounters an unparsable journal line, wallet appends remain closed
until the journal is repaired; neither a new wallet intent nor a terminal
resolution may append behind a torn tail. This wallet-only availability flag
does not truncate, repair or change the legacy reader's treatment of that line.
The same wallet-unavailable flag is latched by every failed append, including
an initial Begin or legacy Finish, so another wallet intent cannot append behind
a partial record in the still-running process. A wallet Begin write/sync error
is reported Unknown with the original ID. Non-wallet errors and record bytes
retain their existing behavior.

The browser retains a bounded (16-entry) run/principal-scoped set of immutable
pending envelopes. Unrelated worker pause/resume/budget requests may proceed
while a wallet action is Running or Unknown. All unresolved envelopes survive
same-context reload; resolving one removes only that entry. Another funds action
is blocked while a funds envelope is pending. A retry denial without durable
resolution evidence retains the old operation rather than cancelling it.

All wallet command shape/network validation also precedes the operator early
return. Failed hard-link publication removes only the current call's acquired
`.ui.tmp`, preserving destination commands and results. Private producer files
remain exactly 0600 under exactly 0700 directories; 0400 is not silently treated
as a compatible result mode.

The runtime registry join must run both after launch and after attach/adopt. Its
read-only reconstruction verifies declared identity/front/pool, persisted agent
and front config, leased listener identity, matching public provisioning address,
canonical initialized state directory and exact private modes. It does not
spawn, provision, regenerate keys or open/interpret SQLite. Any failed binding
stays unavailable and follows the Unknown replay rule above.

## Nonblocking defer

The existing `include_str!` placement for `/access.js` stays for this bounded
correction. Moving it into WebAssets requires a separate constructor/embedding
cleanup and is not needed for replay correctness. The exact public route,
source bytes and CSP remain covered by the compiled browser check.
