# 2026-11-08 - Agent wallet foundation (AP-3a) normative behavior

This record covers the AP-3a agent wallet foundation slice (`--roles wallet`,
cargo feature `ark`) after the 2026-11-08 rework review and its fix-r2
follow-up: the stock bark persister foundation, the registration terminal
policy, branch-key validation, the effective registration binding, and the
spec/config basis that was deliberately deferred in the original AP-3a
report (its merge note 6.6). Item 2 incorporates the fix-r2 correction:
markers are consulted before any mutating step regardless of database
existence, and the changelog entry is a `docs/changelog/` fragment, not a
SPEC frontmatter edit.

## Decisions

1. **The stock bark persister is a bounded offline foundation.** The agent
   opens the pinned stock `SqliteClient` (bark `c419159`) at
   `<datadir>/bark/db.sqlite` and initializes through the stock
   `BarkPersister::init_wallet` with
   `WalletProperties(network, fingerprint, None, None)`. Nothing here opens a
   full bark `Wallet`, talks to a chain or server, or asserts a balance - that
   is AP-3b with a live chain source. The rework review's blocker 1 asked for
   the stock persister specifically (an in-tree sqlite mirror would prove
   nothing about the real wallet); this delivers exactly the persister, no
   more.
2. **Fresh vs damaged is decided before any mutating open, from the
   markers first.** A read-only pre-check plus a two-marker sidecar
   lifecycle (`spc-creating` written before the stock client first opens a
   fresh file, `spc-initialized` written only after `init_wallet` commits)
   distinguishes: database missing or empty + completion marker (partial
   datadir damage, refused, never re-initialized - the deliberate reset
   removes the whole datadir, never `bark/` alone), database missing +
   creation marker only (impossible for this module's ordering - the file
   is created before the marker - refused), database empty + creation
   marker only (our own crash window between file creation and the stock
   client's first open, completed), properties row (validate network +
   fingerprint, normal reopen), no row + completion marker (damaged,
   refused), no row + creation marker (our own crashed creation,
   completed), bark-shaped tables with no row and no markers (foreign,
   refused), no tables and no markers (fresh). The completion-marker
   absence is re-checked immediately before `init_wallet`, so a marker
   collision can only ever be a true concurrent race, never damage
   discovered after a mutation. `SqliteClient::open` runs migrations and
   therefore never sees a database we are about to refuse; refused opens
   leave the bytes untouched and create nothing. Directories are 0700,
   files 0600, created private before sqlite opens them. Stock migrations
   seed some tables, so row-count emptiness is not a fresh/damaged
   discriminator - the markers are. A `state.sqlite` cross-binding
   (`bark_initialized` in its meta table, recorded after every validated
   open) additionally refuses a wholly missing `bark/` directory while
   the agent store survives - "whole `bark/` gone, `state.sqlite` intact"
   is partial damage too. Stores created before this cross-binding
   existed heal it on their next validated open; until then they are not
   covered.
3. **Terminal registration outcomes stop the loop.** Hard terminals
   (`bad-signature`, `ark-key-is-spend-key`, `wrong-ark-server`,
   `bad-mailbox-fields`, `unsupported-mailbox-mode`, unknown codes) and
   request-id exhaustion are fatal immediately: one send, the attempt
   journaled, the exact code surfaced, status `error`, `run()` returns `Err`,
   and every further attempt refuses without sending. Recovery requires an
   explicit operator restart. Retrying a bad signature forever was the
   historical bug. The wedge kinds (`mismatch`, `stale`, `superseded`) get a
   bounded unwedge: at most ONE fresh same-payload retry (fresh request id),
   then a second consecutive terminal for the same payload is fatal; a
   success or a payload change resets the streak. Every non-success outcome
   participates in the 1/2/4/8/16/30 s backoff; after six failed attempts
   local `registration_unavailable` is surfaced while the 30 s retries
   continue.
4. **Registration liveness is the effective binding.** The state schema moves
   to v2 with `effective_digest`/`effective_request_id`: a payload is
   registered only while it matches the most recent acknowledged
   `applied`/`already_applied` state. A historical journal row is not proof
   (applied A, then B, then back to A does not re-register A); a `superseded`
   verdict clears the binding it names; an interrupted attempt flips nothing;
   a replayed old request id journals but cannot rewrite the binding or steal
   the pending reservation. A v1 database is rejected explicitly - there is no
   silent adoption (the v1 journal cannot answer the effective-binding
   question).
5. **Branch files are validated as exact siblings.** Each branch file must be
   the exact depth-4 hardened account for its index (`1'` scan, `2'` ark),
   with matching network magic, and the two files must share a parent
   fingerprint. The fingerprint check is documented as required provisioning
   metadata consistency, not a cryptographic ancestry proof (the daemon holds
   no master key; 4 bytes cannot prove a path). Failures are fatal at load;
   nothing is regenerated.
6. **Registered is not connected.** The status surface reports `registered`
   (the durable effective-binding answer) and the role state as distinct
   questions. Without a registration transport the role is `dormant` - even
   with a persistent ack - with `last_error` naming the missing AP-2 codec;
   a live connection is never invented. The loopback constraint on the status
   listener is enforced at bind (including programmatic addresses), not only
   in config validation.
7. **Honest scope labels on the receive tests.** The committed fixture is
   synthetic: deterministic derivations with explicit expiry constants
   (independent of any finding height), and the recovery proven is key
   derivation plus ledger replay - not real VTXO ancestry parsing, live
   mailbox reads, or stock bark VTXO recovery. `VtxoState::Spendable` in an
   observation is caller-asserted and faithfully persisted, never verified
   against a chain. Live verification is AP-3b's network backend.
8. **AP-3b dependencies are explicit, not hidden.** The scan cursor is
   monotonic-only here; a chain reorg needs a journaled rewind (fork point
   recorded, replay forward, then advance) before live scanning, and
   `set_scan_height` refuses backward moves until it exists. Best-effort key
   zeroization on drop covers the secp-supported surface (branch secrets,
   child seed, mailbox and normal keypairs replaced with a dummy key).

## Consequences

The wallet role now has a persistent foundation whose failure modes are all
fail-closed and tested: foreign sqlite refused with bytes untouched, damaged
claimed-initialization refused rather than re-initialized, identity/network
mismatch refused, v1 schema rejected, wedge terminals bounded, hard terminals
fatal. The AP-3b remainder is unchanged in shape but narrower: the full bark
`Wallet` over the persister foundation, the chain scan with journaled reorg
rewind, live mailbox/status RPCs behind the existing seams, delegated refresh
and offboard, and the `0x5350` transport adapter (AP-2 owns the codec).

## Sections edited

- `docs/SPEC.md` §11.5: BIP 85 exact derivation, provisioning shape,
  persistence layout, client journal/terminal policy, status honesty, and the
  AP-3b reorg-rewind dependency (normative text added ahead of the existing
  bullets; no existing bullet removed).
- `docs/SPEC.md` §11.10: spc-agent `[ark]` gains `seed_child`, `scan_xprv`,
  `ark_xprv`, `datadir`, `status_listen`, `network`, `ark_id`.
- `docs/changelog/2026-11-08-agent-wallet-foundation.md`: the changelog
  entry, as a fragment file per the merge-queue convention
  (`docs/changelog/README.md`) - the merge session splices it into the
  SPEC frontmatter at serial integration and deletes the fragment. The
  SPEC frontmatter itself is **not** edited by this packet; the
  concurrent AP-2 (lab-ark-registration) §11.4/§11.9 fragment is a
  disjoint file, so the two never conflict.
- `config/spc-agent.example.toml`: the six AP-3a `[ark]` keys documented
  (paths/ids only, no secret values).

AP-2's §11.4/§11.9 edits are untouched (precedence: DESIGN.md > SPEC.md; the
registration semantics AP-2 owns live in those sections). The dated record
`2026-11-08-ark-refresh-normal-wallet.md` and the refresh/offboard SPEC deltas
still land with AP-3b, where the behavior they govern is implemented.
