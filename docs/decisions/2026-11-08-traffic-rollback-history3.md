# 2026-11-08 - Atomic traffic rollback evidence and history 3 (amendment f)

## Scope

Approved traffic-runtime plan section 3.5a, amendment (f), SHA256
`44daaa2baa41a4b93eb46a9e69dfdcf2bba1e0ce73367e27e83a87ab3a6b3307`;
Round 5 peer APPROVE in the planning tree's
`docs/plans/reviews/2026-11-08-traffic-runtime-peer-review.md`.

Supersedes only the journal-version/exact-value and core leniency portions
of `2026-11-08-traffic-history-v2.md` and the nondecreasing-only chain
admission portion of the T1a core. The complete approved history-v2-r1
integration remains the primary acceptance prerequisite. This isolated
source packet builds on its exact frozen 602-file layer.

## Decisions

1. **Atomic format activation.** Core history version is 3 and journal
   protocol is `spc-lab-tx-journal/3`. Core writer/reopen and T0
   header/scenario/accepted hello-ack constants agree. The header has the
   same 14 fields in the same order. CONTROL remains
   `spc-lab-tx-control/v1`. Key derivation remains version 2; key-file
   format, key mode, HMAC domains, seed and allocation-digest computation
   are unchanged. The approved v2 golden bytes are preserved alongside
   an explicit v3 golden with only history/protocol changed.
2. **One proof-bearing record.** `ChainRollback {seq, evidence,
   observed_unix_ms}` contains required `RollbackEvidence {version: 1,
   old_anchor, new_anchor, common_ancestor, old_headers, new_headers,
   displaced_block_hashes}`. Each anchor is `{block_hash, height}`.
   Both raw-header branches are tip-to-common-ancestor INCLUSIVE. Each
   raw header is exactly 160 lowercase hex characters (80 consensus
   bytes). No optional/default proof is recognized. Record, evidence and
   anchor types reject unknown and duplicate fields. The history-3
   `JournalRecord` enum rejects unknown fields throughout.
3. **Acceptance policy, exactly.** Under the existing journal mutex,
   old anchor MUST equal the current committed tip; new height MUST be
   strictly lower; common height MUST be at or below new height. Each
   branch length MUST be `anchor.height - common.height + 1`, checked
   for overflow and bounded to 4096 headers. Starting at each anchor,
   every header hash MUST equal the expected hash; each next expected
   hash is the decoded `prev_blockhash`. Header hashes cannot repeat in
   a branch. Compact targets MUST be nonzero, nonnegative and
   non-overflowing under consensus SetCompact rules, and at most
   rust-bitcoin's `MAX_ATTAINABLE_REGTEST`; each header MUST satisfy its
   decoded target. Both branches end at the claimed common hash and
   identical raw ancestor header. There MUST be no earlier common hash.
   The supplied displacement vector MUST exactly equal the old branch
   excluding the ancestor, in old-tip-first order (no duplicates,
   omissions, additions or retained hashes).
4. **Trust boundary.** These checks prove linkage of supplied bytes to
   the committed old anchor and the supplied new anchor. They are NOT
   an independent consensus validator, SPV chain selector or longest-chain
   inference. The runtime observer MUST obtain validated canonical-node
   facts from the persisted selected bootstrap node, verify the bound
   run consensus profile (including exact target schedule), and perform
   the approved raw-block canonicality checks. The regtest target ceiling
   here is a format safety bound, not permission to substitute the pow
   limit for the run's actual target. Other nodes only corroborate.
5. **Bounded decode.** Each proof vector is bounded during typed
   deserialization. Replay first borrows the record tag while ignoring
   other values without building a JSON value tree, then refuses an
   encoded rollback record over 2 MiB before the internally tagged enum
   buffers its contents. Hex decode/allocation follows the 80-byte check.
   Existing journal file read size is still proportional to the full
   journal; this packet adds no unbounded proof expansion.
6. **Atomic application/replay.** `record_chain_rollback` validates the
   proof, durably writes that ONE record, then applies the same
   tip/displacement/cascade logic as ordinary chain observations. It
   consumes one sequence and stamps displacement/chain recency with that
   sequence. Failed validation writes nothing. Failed append uses the
   existing rollback-or-poison machinery. No unproved lower `ChainObserved`
   is ever persisted. Ordinary nondecreasing observations and S3
   same-block re-inclusion after fresh chain evidence remain supported.
7. **Reopen and tails.** History 2 is refused before chmod, truncation,
   `SessionStarted` or automatic migration. All complete records are
   replay-validated, then caps/allocation/driver and wallet
   run/seed/network bindings are checked before any repair/append.
   A complete malformed or unsupported final JSON record is refused even
   without its newline. A recognizable truncated `chain_rollback` record
   is refused and preserved, never repaired away. An unidentifiable torn
   prefix or ordinary incomplete tail retains the existing uncommitted
   tail recovery rule: it cannot authorize any rollback/effect, and only
   the validated prefix is recovered. Missing/invalid proof in a complete
   record never defaults to acceptance.
8. **Maturity on every send.** Both `BroadcastSent` (including every
   Unknown same-bytes retry) and `ReorgResubmit` check all direct inputs
   and recursively relevant OwnTransaction funding ancestry against the
   real committed tip, using an iterative visited-outpoint walk. Missing,
   invalidated or immature backing blocks admission. Own creators must
   remain confirmed. `height + 100` uses checked arithmetic; overflow
   remains immature. A retained coinbase can become immature without
   changing its provenance/status, releasing its reservation or consuming
   retry/resubmit caps. Signed/reserved claims remain held until maturity
   is restored. No AcceptedByRpc transition is invented.
9. **Local forwarding handoff.** `pending_send_intent(command_id)` returns
   an opaque identity of the exact durable send (header binding, txid,
   send sequence and record sequence). `check_send_before_forward(intent,
   expected_committed_seq)` checks that identity, exact current committed
   sequence, nonpoisoned/unpaused state, BroadcastSent state and the same
   ancestry/maturity rule. It writes nothing. The controller MUST
   serialize capture/revalidation + local rollback processing + actual
   forwarding, and enforce its own once-only forwarding correlation. This
   method releases its mutex on return and is NOT a remotely atomic
   network permit. A withheld journaled intent retains uncertainty and
   follows the existing Unknown/new-send retry discipline; already
   forwarded calls retain their honest outcome/Unknown handling.

## Sections edited

- `crates/spc-lab-txd/src/journal.rs`: evidence/admission/replay, maturity,
  opaque send identity, pre-forward guard and history constants.
- `crates/spc-lab-txd/src/wallet.rs`: identity comparison moved before
  journal session append through the internal bound-reopen entry.
- `crates/spc-lab-txd/src/lib.rs`: public DTO re-exports.
- `crates/spc-lab/src/traffic/{protocol,spec}.rs`: history-3 activation.
- `crates/spc-lab/src/traffic/projection.rs`: ONE shared cfg(test)
  history-version fixture line only, requiring serialization with J1.
- Core `tests/history3.rs` and T0 `traffic_contract` /
  `traffic_history_conformance`: offline contract gates.

## Consequences

This is a core fixture gate. Actor/controller binding must consume history
3 before the first runtime writer, with the local serialization required
above. Scoped builds/tests precede integration; under the owner's subsequent
execution grant, implementation review may follow the tested freeze
asynchronously rather than imposing another planning hold. No runtime
controller, native funding, RPC compatibility,
probe lineage or unmodified-captaind interoperability result is inferred
from this source packet. SPEC frontmatter is a serial merge fragment.
