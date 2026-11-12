# Wallet durability and fair maintenance scheduling

Date: 2026-11-09
Status: implementation; verification tracked in the audit-fix report

## Decision

Resolve AP-3b audit R1-R6 without changing server RPCs, keys, pool privacy
boundaries, registration codecs or upstream pins:

1. Round checkpoint version 2 binds input IDs, requested outputs, funding,
   unlock hash and the FULL leaf-cosigned, still-hash-locked output proof bytes.
   Those bytes commit before ForfeitVtxos can be invoked. Recovery uses the
   stored signed leaves and checks recovered preimages against the same binding.
   A legacy checkpoint without the required version/proofs cannot forfeit.
2. Terminal command/payload tombstones remain immutable. Terminal work is
   filtered before the eight-command limit. A persisted filename cursor advances
   before parsing or RPC work, giving unknown and malformed entries bounded
   round-robin service without starving later commands across restart.
3. Actual mailbox due-height filtering precedes the 128-epoch limit. The exact
   retry offsets remain [0, 6, 36, 144, 1008] after the six-block delay. A durable
   due-work cursor rotates continuing/failed reads; invalid height arithmetic
   fails closed.
4. The unattended loop uses the pinned stock wallet's refresh selection helpers
   after SP-round outputs enter its child persister. Pending stock rounds,
   queued owner commands and operation reservations exclude inputs. Selected
   renewals use the same immutable private command/reservation path; no second
   signer or funds ledger exists.
5. Explicit local fee/dust failure and the stock refresh None result have
   distinct no-effect provenance. A successful no-effect transaction journals
   rejection/no-op and releases only that operation's reservations atomically,
   preserving its payload tombstone. Legacy/possibly submitted/forfeited or lost
   responses cannot acquire this proof from a generic error. SQL failure does
   not produce terminal success. Unknown command failures keep live balances
   unknown while the fair scheduler continues other work.
6. A ten-page mailbox burst returns continuation when have_more is still true.
   The epoch/read counter stays open/unchanged until the mailbox is exhausted.
   Continuation is durable before reading, checkpoints still follow verified
   ledger commits, and arrival is recovered from joined receipt/ledger rows so
   duplicate-only crash replay does not lose it.

## Rationale

Durable exit proofs must precede irreversible input forfeits. Maintenance must
follow value across wallet key classes and make progress despite completed,
unknown, empty or truncated older work. Safe no-effect failures must not strand
funds, while uncertain effects must retain their reservations.

## Sections edited

- SPEC §11.5 wallet audit durability/scheduling clarification.
- Dedicated changelog fragment for serial frontmatter integration.

## Downstream aggregation dependency

The ladder owner will provide an ownership-authenticated SPc-side pending/
aggregate-status API with fresh challenge/session authentication. This packet
does not invent that wire contract. A positive unbatched Ark obligation/pending
anchor must later keep its epoch alive until its own VTXO is verified; a pool
Sent receipt only wakes fetching. Coinbase discovery does not close an Ark
epoch because it only pays the on-chain band. Unknown owed obligations cannot
be discarded using H+1014; retention/status integration awaits that concrete
handoff and will compose with this due queue and pagination continuation.

## Evidence

`docs/reviews/2026-11-09-ark-wallet-bridge-audit-fixes.md` distinguishes authored,
compiled, exercised and funded states. Mocked protocol/durability regressions
are not funded interoperability evidence.
