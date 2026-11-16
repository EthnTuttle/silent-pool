# 2026-11-15 - Template-negotiated BIP54 coinbase construction

Draft for primary integration on the future Testnet5 development branch.
Suggested path: docs/decisions/2026-11-15-template-bip54.md.

## Trigger

The owner requested Testnet5 development now and deployment only after lab
approval. Public BIP95 Testnet5 activates BIP54 from height 1. Existing pool
RPC negotiation requests only segwit, builds final-sequence/zero-locktime
coinbases, and discards the node's minimum timestamp.

## Decision

1. RPC pool-template mining explicitly negotiates segwit and consensuscleanup.
   The response's active rules select coinbase consensus. Unknown mandatory
   rules fail the refresh. Signet templates require an unimplemented signet
   solution and are rejected. Unknown optional rules may be used as-is per
   BIP9/145. Network identity remains a separate genesis-bound admission check.
2. An active consensuscleanup template supplies nLockTime = height - 1 and
   nSequence = 0xfffffffe. BIP54 itself permits any non-final sequence; this is
   the pool's construction choice. Height zero and heights requiring timestamp
   interpretation of nLockTime are unsupported. Invalid field/height pairings
   fail before job publication. Legacy templates retain their existing bytes.
3. The serialized coinbase suffix carries these exact fields on native SV2 and
   SV1. The front freezes them with the job and checks the submitted coinbase
   against both those fields and the complete issued bytes before credit and
   block assembly. Consensus-field changes issue a new latest-only generation.
4. GBT mintime is required and curtime must satisfy it and any supplied maxtime.
   These node-computed bounds include active boundary timestamp requirements.
   Bounds changes retire old work; ordinary curtime advancement alone retains
   existing immutable-job behavior. Submission also checks the stored bounds
   and the existing roll window. GBT target must match checked compact nBits.
5. Active BIP54 templates reject exactly-64-byte stripped transactions. The
   trusted full node remains responsible for prevout-dependent script/sigops
   validity and contextual retarget calculation. GBT sigops cost is not the
   BIP54 legacy-input sigops count and MUST NOT be substituted for it.
6. RPC fee summation and subsidy extraction fail on overflow/underflow.
   This closes malformed template admission without changing valid payouts.

## Scope and integration

This implements the RPC pool-template path. TDP/IPC, JD custom-job mining and
Testnet5-aware source audit need their own actual adapters and validation.
Existing JD construction helpers remain legacy-only; their availability is
not a claim of BIP54 JD support. SPc derivation, payout denominations, Ark
delivery/accounting and witness commitment policy are unchanged.

No deployment authorization follows from source integration. Normative
accounting, stock Ark and self-hosting decisions remain separate owner work.

## Evidence

Immutable source references are in REPORT.md beside this draft. Seven official
BIP54 coinbase and thirteen transaction-size vectors are copied byte-exactly
under tests/fixtures; the lightweight independent parser verifies their
expected field/size classification, coinbase Merkle roots and last-block PoW.
Rust front/RPC/coinbase/SV2 tests are authored but not yet executed.

## Sections to amend

SPEC frontmatter, §3.3 and §3.4 using SPEC-DRAFT.patch. No project-specific
wire shape changes: standard SV2 coinbase prefix/suffix already bind the bytes.
Cross-reference review: WIRE §2 and §3.10; ROADMAP Phase 3 §3.3/§3.4 acceptance;
ARCHITECTURE §5.1/§7.1. These retain their existing section targets.
