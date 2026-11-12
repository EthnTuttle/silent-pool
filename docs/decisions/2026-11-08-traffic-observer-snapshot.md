# 2026-11-08 - Traffic observer committed snapshot (amendment e)

## Scope

Approved traffic-runtime plan section 3.5d, amendment (e), plan SHA256
`44daaa2baa41a4b93eb46a9e69dfdcf2bba1e0ce73367e27e83a87ab3a6b3307`.
This is the core observer read surface over the preserved history-v2-r1
baseline, paired with the separately recorded history-3 activation.

## Decisions

1. `TrafficJournal::observer_snapshot(&self) -> ObserverSnapshot` clones
   committed state while holding the journal's existing `Mutex<JournalState>`.
   All record validation, durable append and commit use that same mutex;
   the file's existing lifetime flock continues to exclude another owner.
2. The public typed DTO contains `committed_seq`, optional `ChainAnchor`
   (tip block hash and real height), and `BTreeMap<OutPoint, ObserverOutpoint>`.
   Each entry contains amount, script, owner, chain/index, source/context,
   current `OutpointStatus` including invalidation/prior claim, and the
   direct coinbase maturity flag. Ordering is deterministic by outpoint.
3. This performs no file I/O, no append, no second writer or replay. It
   lends no mutable internal state. Cloning or editing the returned value
   cannot construct `ValidatedAllocation`, alter its run/caps binding, or
   authorize a spend. These DTOs are not accepted as mutation authority.
4. Reusing a snapshot after any append intentionally returns old data and
   its original immutable sequence value. The observer requests a new
   snapshot for fresh facts. A committed tip is not a completed-scan cursor:
   the runtime observer must perform the approved inclusive genesis rescan,
   skipping valid entries and re-observing invalidated external funding.
   Own-transaction outputs restore through creator confirmation only.

## Sections edited

- `crates/spc-lab-txd/src/journal.rs`: DTOs and accessor.
- `crates/spc-lab-txd/src/lib.rs`: DTO re-exports.
- `crates/spc-lab-txd/tests/history3.rs`: detached-copy, current-status,
  sequence/freshness and concurrent-coherence tests.

## Consequences

The observer can distinguish invalidated entries from valid previously
seen outpoints without opening a second ledger. Runtime observation,
canonicality verification and scan completion remain the controller/actor
integration's responsibility. SPEC frontmatter is supplied as a fragment
for serial integration; no pool payout or Ark protocol behavior changes.
