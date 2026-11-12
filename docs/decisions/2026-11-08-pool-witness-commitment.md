# 2026-11-08 - Pool-template witness commitments (BIP 141)

## Trigger

The pool-template job path was witness-blind. It folded every template
transaction as its stripped serialization (txid and wtxid conflated), would
have assembled a mixed block with no witness commitment - invalid at consensus
when any transaction carries witness data - and the settlement mode detection
in `payout` would have read a commitment output as a Job Declaration carrier,
misclassifying every witness-bearing pool block as JD mode at settlement. The
witness work packet T4a covers the independent pool-side primitives with
deterministic conformance vectors; it is not the live canonical-block gate
(T4b) and not JD-side witness handling.

## Decision

1. **Strict template decode, fail closed.** Every template transaction is
   decoded with the full-transaction consensus deserializer (trailing bytes
   rejected) and re-serialized byte-exactly; a blob that is not its own
   canonical encoding - a segwit marker/flag with empty witness, or the
   zero-input serialization ambiguity - is rejected. A template refresh
   containing any non-canonical transaction fails the refresh. There is no
   best-effort skip, no donation fallback, no silent witness stripping.
2. **txid/wtxid distinction.** Transactions are analyzed into txids (stripped
   serialization), wtxids (full serialization) and a witness count. The txid
   merkle root the front already publishes is unchanged. The witness root is a
   separate merkle fold over the wtxids with the coinbase leaf fixed at zero
   (BIP 141: the coinbase wtxid is the zero hash for this computation).
3. **Commitment output.** A job whose transaction set contains at least one
   witness-bearing transaction gains exactly one additional coinbase output:
   zero value, 38-byte script `OP_RETURN 0x24 0xaa21a9ed <commitment>`, the
   commitment being `SHA256d(witness_root || reserved)`. The output is
   appended last, so the pool payout output stays at index 0 and every
   consumer index is unchanged. BIP 141's consensus rule recognizes a
   commitment by the 6-byte prefix with a script of at least 38 bytes and,
   where several outputs match, takes the highest output index; the pool's
   policy for its own coinbases is deliberately stricter - exactly one
   well-formed 38-byte zero-value output, duplicates and malformed shapes
   rejected outright (a header with the wrong length is ambiguous and is
   not guessed at). Parsing arbitrary node-accepted blocks under the
   lenient consensus rule is a separate concern, crosschecked against a
   real node in T4b (rework 2026-11-08: this point restates the BIP 141
   facts correctly; the earlier draft wrongly said BIP 141 makes multiple
   commitments invalid).
4. **Reserved value.** BIP 141 places no constraint on the witness reserved
   value (rework 2026-11-08: the earlier draft wrongly attributed a
   SHOULD-random rule to BIP 141). The front draws it from the OS RNG once
   per witness root and caches a single entry keyed by that root - a
   pool-side share-stability choice, not a protocol obligation. Randomness
   is therefore per transaction-set, not per republication: a refresh that
   republishes an unchanged transaction set keeps the commitment, so shares
   already issued against that job stay valid. A changed transaction set
   gets fresh randomness. The cache is kept as-is through the rework to
   avoid a needless behavior change; a zero reserved value remains
   representable for tests and vectors.
5. **Job identity.** The commitment joins the job-change comparison alongside
   fees, distribution, transaction set and version. Same-txid witness
   malleation moves the wtxid, hence the witness root, hence the commitment,
   so the job is republished even though the non-coinbase txids and the merkle
   branch are unchanged - the malleation-resistance property: the pool never
   pays for a template whose witness data differs from what it validated. The
   txid merkle root also moves, because the commitment output is part of the
   stripped coinbase.
6. **Block assembly, fail closed.** A winning share on a witness job assembles
   the block by strict verification: every transaction is strict-decoded, the
   coinbase must carry exactly one zero-value commitment output, and that
   output's commitment must equal the one recomputed from the actual
   transaction set. A wrong reserved value, a missing, duplicate, malformed or
   valued commitment, or a commitment mismatch rejects the block; it is never
   submitted. The witness is injected into the stripped coinbase (marker/flag
   after the version; one 32-byte witness item before the locktime) and leaves
   the coinbase txid unchanged, so shares mined on the stripped coinbase
   remain valid evidence for the merkle root the front already verified.
7. **Weight accounting.** The commitment output is 47 vbytes / 188 WU and the
   coinbase witness adds 36 WU: a witness job's fixed overhead is 224 WU,
   added to the tier weight budget only when a commitment is present.
   Witness-free templates are byte-for-byte unchanged - same coinbase, same
   merkle root, same job identity, no overhead.
8. **Settlement mode detection.** `payout`'s template-mode detection excludes
   `6a24aa21a9ed`-shaped outputs from the JD-carrier test. The real JD
   carrier is `OP_RETURN OP_PUSHBYTES_35 "SPc" <ser_x(A_send)>`
   (`6a23535063...`, SPEC 2.4.2, `spc_crypto::carriers::emit_op_return`);
   its `6a23` push cannot collide with the commitment's `6a24` push
   (rework 2026-11-08: the earlier draft misdescribed the carrier as a
   `6a20` 32-byte push). A pool-template block carrying a witness
   commitment settles in pool mode; the actual JD carrier - alone or
   alongside a commitment - still selects JD mode.

## Fable rework amendments (2026-11-08, REQUEST_CHANGES disposition)

The first packet was returned with review
`docs/reviews/2026-11-08-witness-primitives-fable.md`; these amendments
apply in the same packet and are normative:

- **B1 - main publication identity.** `main`'s publication dedup key
  (`Published`) now includes the front job id in addition to height,
  prev-hash, txid-merkle identity, version, bits and distribution
  fingerprint. A commitment-only same-txid witness change moves the job id
  while leaving every other key field equal, so the job is republished to
  both ingresses (SV1 notify and SV2 NewExtendedMiningJob) with no height
  or fee change involved. An unchanged set still does not republish.
- **B2 - zero-input rejection.** Strict decode explicitly rejects
  transactions with no inputs. The reviewer's raw vector
  `020000000001000101000000000000000000000000` (marker/flag, zero inputs)
  decodes and roundtrips byte-for-byte, yet is not consensus-valid and has
  a wtxid different from its txid while no input carries witness data -
  exactly the shape that must never fall through to the legacy txid path.
  The vector is a frozen negative in both the module tests and the
  conformance suite. Witness detection remains structural (decoded input
  witness stacks), never txid/wtxid inequality alone.
- **B3 - BIP 141 factual corrections.** Points 3 and 4 above are restated:
  the reserved value is unconstrained, consensus commitments are >= 38
  bytes with the highest matching index winning, the pool's own-coinbase
  policy is deliberately stricter, and an independent parser of arbitrary
  node-accepted blocks is T4b, not this packet. The per-root random
  reserved cache is kept (no behavior change) and documented truthfully.
  SPEC 2.4.2 is amended: pool-template mode must not emit the SPc
  `OP_RETURN` carrier - the prohibition targets that carrier script, not
  `OP_RETURN` outputs in general; the witness commitment is not an SPc
  carrier.
- **R5 - settlement overhead mirror.** Settlement's `tier_config` adds the
  same 224 WU the front budgeted whenever the mined coinbase carries a
  commitment-shaped output, so the budget crosscheck agrees at the exact
  boundary (224 + k x 172 WU) instead of keeping unpaid ids.
- **R6 - post-credit assembly failure.** If witness block assembly fails
  after a share was credited, the front logs a bounded error (front id,
  height, job id, error) stating that the credit stands in the slice and the
  evidence, no block event is emitted and no block is submitted, then
  returns the error to the caller. No reorder, no protocol rewrite, no
  silent loss: the accounting truth (the share was real work) is preserved
  and the assembly failure is loud.
- **R8 - CompactSize boundary.** The 224 WU overhead does not cover the
  coinbase output-count CompactSize growing from one byte (<= 252 outputs)
  to three (`0xfd` + 2 bytes, >= 253 outputs) when the commitment output
  crosses that boundary (+2 vbytes = +8 WU). The whole-block weight model
  stays the preexisting tier-budget approximation - the ladder governs
  pool-controlled payout outputs - and the boundary is pinned by a test
  (`compact_size_growth_at_the_253rd_output_is_8_wu`) and documented in
  `WITNESS_COMMITMENT_OVERHEAD_WU`, not redesigned into a hard block weight
  limit.
- **R4 - real JD carrier in tests.** Folded into point 8 above: the JD
  carrier exercised in the settlement tests is the actual
  `spc_crypto::carriers::emit_op_return` script (`6a23535063...`), tested
  alone and alongside a witness commitment (which must not mask it);
  unknown `OP_RETURN` outputs still select JD mode.
- **R7 - GBT crosschecks.** Unchanged in scope: `template_rpc` (GBT)
  crosschecks against a live node remain part of the T4b handoff, not this
  packet.
- **R9 - structural signatures.** The conformance vectors' P2WPKH/P2TR
  inputs carry structural pattern signatures (correct lengths and script
  shapes), not signatures that would validly spend their nonexistent
  prevouts; the vectors exercise txid/wtxid separation, witness-root
  folding and commitment binding only. Stated in the test module header;
  real-key spend validity is T4b.

## Compatibility and boundaries

- Witness-free templates are byte-for-byte unchanged: same coinbase, same
  merkle root, no commitment output, no weight overhead. The only
  main-level behavior change for them is none: the publication dedup key
  gains the job id, which is stable for an unchanged job, so republication
  behavior is identical.
- The reserved-value cache is a pool-side share-stability choice. BIP 141
  places no requirement on the reserved value; drawing it per witness root
  (rather than per publication) is not a protocol deviation and needs no
  consensus waiver. It is kept through the rework unchanged.
- The pool's commitment scan is intentionally stricter than the BIP 141
  consensus rule (exactly one well-formed 38-byte zero-value output last,
  versus consensus's >= 38 bytes with the highest matching index winning).
  It applies only to coinbases the pool itself constructs and re-verifies;
  parsing arbitrary node-accepted blocks under the lenient consensus rule
  is T4b.
- `front.rs`, `payout.rs` and `main.rs` are shared with other work packets
  (AP2 touches front/main); this packet's changes there are narrow hunks
  enumerated in the implementation report, never wholesale file replaces.
- JD-side witness handling (JDP-supplied witness templates, JD weights)
  remains out of scope; the JD carrier (`6a23...`) and the commitment push
  (`6a24...`) cannot collide.

## Edits applied

First packet:

- `bins/spc-pool/src/witness.rs` (new), `template.rs`, `block.rs`,
  `coinbase.rs`, `template_rpc.rs`, `lib.rs`, `front.rs`, `payout.rs`,
  `tests/witness_conformance.rs` (new).
- `docs/SPEC.md`: dated body amendments 3.3 "Witness commitments
  (2026-11-08)" and 4.8 "Witness overhead (2026-11-08)".
- `docs/changelog/2026-11-08-pool-witness-commitment.md` (new; merge-owner
  splice per `docs/changelog/README.md`).

Rework (same packet, Fable REQUEST_CHANGES disposition):

- `bins/spc-pool/src/main.rs`: `Published` gains `job_id` (dedup key);
  `witness_publication` test module (B1 ingress-level proof).
- `bins/spc-pool/src/template.rs`: explicit zero-input rejection; the
  reviewer's raw vector as a frozen negative.
- `bins/spc-pool/src/witness.rs`, `block.rs`, `front.rs`: BIP 141 factual
  corrections (unconstrained reserved value, consensus vs pool policy,
  highest-index rule); R6 bounded assembly-failure log; R8 CompactSize
  boundary test.
- `bins/spc-pool/src/payout.rs`: real JD carrier in the mode-detection
  test (R4); settlement `tier_config` mirrors the 224 WU witness overhead
  with a budget-boundary test (R5).
- `bins/spc-pool/tests/witness_conformance.rs`: zero-input negative (B2);
  structural-signature note (R9).
- `docs/SPEC.md`: 2.4.2 amended (the pool-template prohibition targets the
  SPc `OP_RETURN` carrier, not `OP_RETURN` outputs in general); 3.3 and
  4.8 amended per B3/R5/R8.