# 2026-11-08 - Atomic private traffic recovery snapshot

## Scope

Explicit implementation scope extension following the incomplete T1b freeze
`f2e2e401...` / `0b5bcd63...`. This is an additive read API over the audited
history-3 journal. It changes no journal bytes, header, derivation, protocol
version, validation, maturity, rollback or state-transition algorithm.

## API and decisions

`TrafficJournal::recovery_snapshot(&self) -> RecoverySnapshot` holds the
existing journal mutex once and returns owned, deterministic clones:

- `committed_seq`, `tip`, `barrier_active`, `next_send_sequence`;
- `last_broadcast_txid`, `last_chain_observed_seq`, persisted `caps`, `poisoned`;
- **all** `commands: BTreeMap<String, StoredCommand>`;
- **all** `transactions: BTreeMap<Txid, RecoveryTransaction>`.

Each RecoveryTransaction contains the existing `SignedTxFacts`, `last_send`
as `(send_sequence, intent_record_seq)`, `reorged_from_block`,
`displaced_at_seq`, and `invalidated_prior`. Finished commands remain present:
their transactions can subsequently reorg or become uncertain. The actor
selects work by the actual transaction state and persisted send caps.

The snapshot types deliberately implement neither Serialize nor Deserialize.
No mutable internal reference or authority token is returned. A changed copy
cannot authorize anything; existing mutation paths re-check caps, maturity,
identity and phase, and only the core creates PendingSendIntent. HTTP/SSE
receive the separate closed TrafficObservation projection, never these maps,
scripts or raw transaction bytes.

The existing public `tx_facts` view now delegates to a private JournalState
helper containing its original field-for-field clone. The snapshot calls that
helper and `stored_view` while already holding the mutex, avoiding relocking.
There is no file read, replay, new writer or second durable ledger.

## Compatibility note

`last_broadcast_txid` is an exact copy of the existing core field, not a
reinterpretation. The audited core updates it on BroadcastSent; ReorgResubmit
updates the transaction's `last_send` tuple. Consequently a resubmission of an
older transaction can have a newer send sequence than that legacy field's
transaction. The actor preserves the committed SendBarrier's `last_txid` and
also reports the derived `last_intent_txid` from the maximum last_send tuple.
This packet does not silently change the audited transition or barrier bytes.

## Source and checks

The external `/tmp/opencode/traffic-runtime-r2-core-read-api.diff` contains
only the DTOs, read method and private view-helper extraction. Tests cover
empty and detached views, command/transaction identity and raw-byte binding,
SessionStarted uncertainty with a retained barrier/send identity, finished
reorged commands, resubmission sequences, and concurrent barrier updates.
These tests require the scoped build slot before acceptance.

The original 44-row source delta and its manifests were archived before any
extension edits at `/tmp/opencode/traffic-runtime-incomplete-r1/`; its source
archive is 214,414 bytes and contains no target or full workspace copy.
