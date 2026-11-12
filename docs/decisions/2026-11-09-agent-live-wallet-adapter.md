# Agent live wallet adapter invariants

Date: 2026-11-09
Status: implemented and compiled; owned unfunded runtime validated; funded acceptance pending

## Decision

The AP-3b agent opens pinned stock Bark on AP-3a's existing persister and child
seed, pins actual server/mailbox metadata before first-use persistence, and
uses AP-2's canonical native Noise/SV2 registration codec. No server protocol,
upstream pin, key derivation or pool privacy boundary changes.

Registration reconnects revalidate the original signed operation id. Journal
application binds identity and pending payload; a mismatch cannot retain the
binding it contradicts. A datadir has one exclusive wallet writer.

Real Bark identifiers are 36-byte outpoints. AP-3a's frozen synthetic vector
identifiers remain unchanged in their fixture, but their test-only record
adapter supplies an explicit synthetic vout. Existing synthetic database rows
are refused by the live wallet. Proof bytes and mailbox checkpoints share the
existing SQLite file; they are metadata, not another funds ledger.

The cold one-shot provisioning process writes the existing exact BIP85 and
scan/Ark branch products. Master backup is in a separate cold directory;
complete provisioning is validated on reuse and partial provisioning is
refused. Online execution never reads the master backup.

The lab declares `wallet_agents` separately from identities, miners, workers
and hash budgets. Every Ark connection has one configured writer in its pool;
legacy random identity files cannot be adopted as wallet provisioning. The
same owned-process/current-registration predicate guards initial miner launch,
dynamic miner addition and dynamic connection addition. Wallet startup is
after fronts and Ark infrastructure, before the final miner-launch phase;
it does not activate miners from a hook or bypass T1's funding prerequisite.

Canonical scan history, mailbox schedules and reorg rewinds are journaled in
the existing SP wallet database. A reorg invalidates receipt freshness and
retains ownership/proofs for reconciliation. Missing proof means unknown.
The live role currently accepts epoch-derived delivery only; other receive
modes fail startup rather than registering an unconsumed delivery destination.

SP refresh uses standard delegated hArk participation, stock leaf-cosign and
hash-locked-forfeit helpers, with the SP-owned input signer kept client-side.
Outputs use the existing stock child-wallet key chain and stock import.
SP offboard directly uses standard PrepareOffboard/FinishOffboard and the
stock OffboardForfeitContext with SP input keys. Both use actual server fees,
validate canonical funding and output policies, and persist uncertainty before
mutating RPCs. Ordinary child-wallet inputs use stock refresh/offboard APIs.
The private owner-only CLI command queue binds an opaque operation id to an
immutable payload and reserves inputs before effects. A lost initial round
submission response stays unknown: the standard server supplies no client-id
lookup, and the adapter does not invent one or blindly resubmit. Confirmed
outcomes are checked against the node's canonical chain; RPC acceptance alone
is never called confirmation.

These are client/runtime changes only. Server source and RPC interfaces,
the master/BIP85 derivation, upstream pins and the T1 funds ledger stay intact.

## Rationale

A cached Applied result cannot establish a new connection's registration.
Truncating Bark's outpoint IDs would lose vout and destroy reliable deduplication
and future spending. Stock wallet opening must not be represented as funding.

## Sections edited

- SPEC §11.5, live-adapter invariants.
- `docs/changelog/2026-11-09-agent-live-wallet-adapter.md` for serial splice.

## Evidence

See `docs/reviews/2026-11-09-ark-wallet-bridge-implementation.md`. Source-authored,
compiled and funded states are tracked separately there.
