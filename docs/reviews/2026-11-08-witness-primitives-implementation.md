# T4a: pool witness primitives (txid/wtxid, BIP 141 commitment, mixed-block assembly)

Date: 2026-11-08
Author: TFF implementation agent
Status: rework complete (Fable REQUEST_CHANGES disposition), returned to the
same Fable reviewer for approval
Worktree: `/home/ethan/.local/share/opencode/worktree/ad8886/lab-witness-primitives`
Base: `b0ab4ca34fda07c61a6335d05f598eaacc69678c`
Bootstrapped from: the approved final-runtime source, uncommitted, bound by the
frozen manifest `/tmp/opencode/LP-final-runtime-manifest.json` (370 file
records, `code_snapshot_sha256
30fedafdbba57b79fe4c13e2de0658aa3f05f527b778a242cbfab9c4ba253d76`, base
`b0ab4ca34fda07c61a6335d05f598eaacc69678c`)
First packet: manifest `/tmp/opencode/T4a-review-manifest.json` (immutable,
superseded by this rework; snapshot `72c2ffd5f8cdb835e76ef77a71feeae88ae8a238f948f955de878308ef57946a`)
This rework: manifest `/tmp/opencode/T4a-rework-manifest.json` (same recipe:
base + all 375 sorted file records, compact canonical JSON; the code snapshot
excludes only this report's path)
Review: `docs/reviews/2026-11-08-witness-primitives-fable.md` (primary,
read-only)
Contract: witness work packet T4a (peer-cleared); decision record
`docs/decisions/2026-11-08-pool-witness-commitment.md`; changelog fragment
`docs/changelog/2026-11-08-pool-witness-commitment.md`

This packet is an independent implementation topic. It does not inherit the
final runtime's approval and does not touch any other worker's files. Nothing
is committed; the tree is the baseline dirty state plus exactly the files in
section 1 (verified by re-hashing every baseline manifest record against the
tree: 9 modified, 5 added, 0 missing, 0 unexpected - the additions being the
two code files and the three documents; `main.rs` joins the modified set in
this rework for B1).

---

## 1. What was built

The pool-template path now distinguishes txid from wtxid, constructs a BIP 141
witness commitment when a job's transaction set carries witness data, and
assembles a mixed canonical block from a winning share by strict
re-verification, fail closed. Witness-free templates are byte-for-byte
unchanged. The rework (same packet) closes the Fable review items B1-B3 and
R4-R9; section 6 is the disposition table.

| Path | Kind | Purpose |
|---|---|---|
| `bins/spc-pool/src/witness.rs` | new | BIP 141 primitives: commitment header/script constants, `witness_root` (wtxid fold, zero coinbase leaf), `witness_commitment`, `coinbase_commitment` scan (rejects duplicates and malformed shapes), weight constants (188/36/224 WU). Rework: module docs state the BIP 141 facts exactly (unconstrained reserved value; consensus >= 38 bytes, highest matching index wins; pool policy deliberately stricter), `WITNESS_COMMITMENT_OVERHEAD_WU` documents the CompactSize 252->253 boundary (+8 WU) and the settlement mirror. |
| `bins/spc-pool/src/template.rs` | edited | Strict full-tx decode (`deserialize` + byte-exact roundtrip + explicit zero-input rejection) shared as `decode_transaction_strict_reason`; `analyze_transactions -> TransactionSet {txids, wtxids, witness_count}`; `template_txids` becomes a wrapper. Rework (B2): the reviewer's raw zero-input vector `020000000001000101000000000000000000000000` is a frozen negative. |
| `bins/spc-pool/src/block.rs` | edited | `coinbase_with_witness` (byte-splice of marker/flag and the 32-byte witness item, self-checked), `assemble_witness` (strict-decode all txs, exactly one zero-value commitment, re-verify against the actual tx set), `AssembledWitnessBlock`, `BlockError`. Rework (R8): `compact_size_growth_at_the_253rd_output_is_8_wu` pins the documented boundary. |
| `bins/spc-pool/src/coinbase.rs` | edited | `commitment_output()` (38-byte script, zero value); `CoinbaseTemplate::assemble_with_witness`. |
| `bins/spc-pool/src/template_rpc.rs` | edited | `rpc` feature: keeps witness-bearing transactions (strictly decoded), fees = all included. |
| `bins/spc-pool/src/lib.rs` | edited | One line: `pub mod witness;`. |
| `bins/spc-pool/src/front.rs` | edited (integration hunk) | `WitnessPlan {wtxid_root, reserved, commitment}`; `reserved_for` (OsRng draw, one-entry cache keyed by wtxid root); `commitment_moved` in the job-change comparison; +224 WU tier overhead when a commitment is present; commitment output appended last; `on_share` assembles via `assemble_witness`; `invalidate_current_work` clears the plan, keeps the cache. Rework (B3/R6): `reserved_for` doc states the reserved value is unconstrained per BIP 141 (the per-root draw is a pool-side choice); a post-credit assembly failure logs a bounded error (front id, height, job id, error) and returns `FrontError::Block` - the credit stands, nothing is submitted, no silent loss. |
| `bins/spc-pool/src/payout.rs` | edited (integration hunk) | `detect_mode` excludes `6a24aa21a9ed`-shaped outputs from the JD-carrier test. Rework (R4/R5): the JD carrier exercised in tests is the real `spc_crypto::carriers::emit_op_return` script (`6a23535063...`), alone and alongside a commitment; settlement's `tier_config` mirrors the front's 224 WU witness overhead, detected from the mined coinbase, with a budget-boundary test. |
| `bins/spc-pool/src/main.rs` | edited (integration hunk; new in this rework) | B1: the publication dedup key `Published` gains the front job id (drops `Copy`); a commitment-only same-txid witness change moves the job id while every other key field stays equal, so the job is republished to both ingresses. Test module `witness_publication` (section 3). |
| `bins/spc-pool/tests/witness_conformance.rs` | new | Ten tests over frozen vectors from an independent Python reference, positive and negative. Rework (B2/R9): the zero-input vector joins the negatives; the header states the P2WPKH/P2TR signatures are structural patterns, not valid spends. |
| `docs/decisions/2026-11-08-pool-witness-commitment.md` | new | The decision record (rework amendments in the same file). |
| `docs/changelog/2026-11-08-pool-witness-commitment.md` | new | The SPEC changelog fragment (merge-owner splice), updated for the rework. |
| `docs/SPEC.md` | edited | Dated body amendments to the three cited sections: 2.4.2 (SPc carrier clarification), 3.3 "Witness commitments (2026-11-08)" and 4.8 "Witness overhead (2026-11-08)". No frontmatter edit. |
| `docs/reviews/2026-11-08-witness-primitives-implementation.md` | new | This report. |

No root `Cargo.toml`, `Cargo.lock`, flake, config or shared-export change. No
miner-protocol, signer, ladder, node-patch or runtime change. No pin bumped.

## 2. File inventory (baseline -> this packet)

sha256 over the worktree files, verified against the baseline manifest:

| Path | Baseline | This packet |
|---|---|---|
| `bins/spc-pool/src/witness.rs` | (absent) | `b8b6270d402a7a7e791f7188f223d0b9200cd4d2ec944609391480accb019813` |
| `bins/spc-pool/src/template.rs` | `9e62e79b7834f2d056f0d9e508838aad163e8a4c12349b6e4e66321332abfaa6` | `b6207efca535ab5a2ff9a0927aa6b2df6ba1c551c57f60cf465e11c28fa21848` |
| `bins/spc-pool/src/block.rs` | `87626b600cfa8525985ede75f8c98e0e0b8ceae8b56dedb905beab0cab6aa41a` | `cd50c58ff83b3f4a55e79fc45382ceead8573b40183cf76c63d7c9897eee9272` |
| `bins/spc-pool/src/coinbase.rs` | `e422c3eb3ca4c6610c620bd1766a657ab23e5258bf43b64b52f2cf505e5b399c` | `81f33161691238dc140394409944c63e68156d52a4460ccaa0cc44332df5091f` |
| `bins/spc-pool/src/template_rpc.rs` | `a6c728f34146ba90761c6bcd3cac70816f28e5f3a6c1b20596bcdb0af4ddff0d` | `7ae26496c0dbd03c8cdd7eb6d158d948b1e53e8511c1c69350cf94c214255958` |
| `bins/spc-pool/src/lib.rs` | `9735a9f99066519c7a910b8729dc8bfbe4b4f59775be0df256f8184b612907f4` | `8ecf44abc3c1f0bfd959a8ae63c93af593002b39b82c229228bea2b571b45505` |
| `bins/spc-pool/src/front.rs` | `9b8232436e527a7c9501e540a61b4cfe9b750875b592075574967067e9ab7112` | `604ef5e8ac2f408aba106db9b54d5af4d9701cd8796c84be2913a88a0b9e0954` |
| `bins/spc-pool/src/payout.rs` | `52e1e1af20a569a0984b93ed9f4d195c9d4bcb3835b2941f64e16d9808af69f3` | `aa6bd28e90660e0c54db96d7d806c533e8904eedb26a8cac28a77a524bad7e6e` |
| `bins/spc-pool/src/main.rs` | `5fe65bb6c163d1123a022db811c8373fb00d3780f4f597aa7b070d62f2cd8a0c` | `1b27b9ac6333f1e1e57c01bda48fc7d5c47d5be9d3e6282fceb45d33ed037639` |
| `bins/spc-pool/tests/witness_conformance.rs` | (absent) | `c7e5f49ca00e045a77529818610220720b2aa94e3af53d5d0a47ad700b60b173` |
| `docs/SPEC.md` | `5de914d7e2a38505beafa4410bd41798de5b2d2a65b46144832c3f769edbb788` | `759d0f330b7fad11b1ef135e55a6b7949b069814e18bf2a6d1208e705e093ef0` |
| `docs/decisions/2026-11-08-pool-witness-commitment.md` | (absent) | `d45ca03e44ab38cb593cb3adc3a6a1e2c4082b213cdba55c32164b6c3586f8f8` |
| `docs/changelog/2026-11-08-pool-witness-commitment.md` | (absent) | `d4ec9eec68dfb3099fa1c44c4a5f1e8605adb7c4f3f33f696560c60bf505a47a` |

This report's own hash is carried by the rework manifest (the recipe's code
snapshot excludes exactly this path). The baseline's other 361 manifest
records re-hash identically (the manifest's own excluded set aside: `.git`,
`target`, `result`, `cache`, `temp`, `spc_ark_client/test`). The full-tree
verification command and its 9-M / 5-A / 0-D result (the 5 additions being
the 2 code files and 3 documents) are reproducible from the baseline manifest
alone; the rework manifest `/tmp/opencode/T4a-rework-manifest.json` carries
the same 375-record file list with this packet's final hashes.

## 3. Verification

All commands run with `-j2`, default features unless stated:

- `cargo test -p spc-pool`: **247 lib + 2 main-bin + 21 integration tests
  pass, 0 failed** (270 total; 11 pre-existing integration across `e2e_sv1`,
  `e2e_sv2`, `e2e_type32`, `example_config`, `main_sv2_pipeline`,
  `sv2_conformance`, `sv2_outcome_preemption`, plus the 10
  `witness_conformance` tests).
- `cargo clippy -p spc-pool --all-targets`: clean (no warnings in this
  packet's code; the only output is the pre-existing environment/sqlx noise).
- `cargo check -p spc-pool --features rpc`: clean (covers `template_rpc.rs`,
  which the default-feature test run does not).
- `cargo fmt -p spc-pool -- --check`: clean.
- Pre-existing prototype vectors and every pre-existing test are unchanged
  and pass; witness-free template semantics are byte-for-byte preserved
  (asserted by `witness_free_templates_carry_no_commitment` and the untouched
  legacy vectors).

Front-side unit tests (inside the 247): `witness_templates_get_a_
commitment_and_assemble`, `witness_free_templates_carry_no_commitment`,
`a_same_txid_witness_malleation_republishes_the_job`,
`an_unchanged_witness_set_keeps_the_commitment`,
`a_non_canonical_template_transaction_fails_the_refresh`, plus the
`malleated_witness_tx` helper. Payout-side: `a_witness_commitment_output_is_
not_a_jd_carrier` (rework: exercises the real `emit_op_return` carrier) and
`settlement_mirrors_the_witness_overhead_at_the_budget_boundary` (R5).
Block-side: `compact_size_growth_at_the_253rd_output_is_8_wu` (R8).

Main-bin unit tests (the 2): `wiring_guards::
front_event_reader_requires_an_owned_redis_url` (pre-existing) and, new in
this rework, `witness_publication::
commitment_only_witness_change_republishes_both_ingresses` - a real
main-level publication test (B1): an in-process `Sv1Server`, `Sv2Server` and
`JdsServer` bound to `127.0.0.1:0`, a real SV1 line-protocol client
(subscribe/authorize, notify lines) and a real SV2 noise client
(setup-connection, open-extended-channel, NewExtendedMiningJob frames),
driven through `refresh_and_publish` with a swappable template source. It
proves: the first job reaches both ingresses carrying the same job id (SV1
notify job id string equals `format!("{:08x}", sv2_job_id)`); a
commitment-only same-txid witness change - same height, prev-hash, bits,
version, fees, txid-merkle identity and distribution fingerprint - republish
a **new** SV1 notify and a **new** SV2 NewExtendedMiningJob with a new job
id on both; an unchanged transaction set publishes nothing (400 ms quiet
asserts on both clients); a block-finding share on the new job assembles a
witness block through the real `on_share` path (commitment re-verified),
while the same share under the old job id is stale (`FrontError::StaleJob`).
No node and no live mining is involved; the share search runs at regtest
bits `0x207f_ffff`.

## 4. Conformance vectors

The expected values were produced by an independent reference generator
(`/tmp/opencode/T4a-vectors/gen.py`, Python stdlib `hashlib` only; frozen
output `vectors.json`) and embedded as hex constants in
`bins/spc-pool/tests/witness_conformance.rs`. The constants were rewritten
programmatically from `vectors.json` after two hand-transcription corruptions;
the Rust test asserts them byte-for-byte, so the vectors remain reproducible
from the test file alone. The frozen set is a legacy tx, a P2WPKH, a P2TR
key-path spend and a second P2WPKH (three witness-bearing), template order,
plus the reference pool coinbase (`try_build_pool_template(2, 800_000,
a_x = 0x40..0x5f, 8, outputs, 0)` with extranonce `aa11bb22cc33dd44`,
reserved `8081..9f`).

R9 (rework): the P2WPKH/P2TR inputs carry structural pattern signatures
(correct DER/Schnorr lengths and script shapes), not signatures that would
validly spend their nonexistent prevouts; the vectors exercise
txid/wtxid separation, witness-root folding and commitment binding only.
Real-key spend validity is T4b (section 9). This is stated in the test
module header.

| Quantity | Frozen value |
|---|---|
| coinbase txid | `8a51bac906638fde5715c4d046ee0bc4e03b1538cdc26453360604a2032d0dc1` |
| witness root (wtxids, zero coinbase leaf) | `074a69eca6c311a8d7dec43231be8be5dc89800e592969345f577e7d21e5d836` |
| commitment (`SHA256d(root || reserved)`) | `91959ba5b529dcc96825a62b66e67862cec627160ce3f2dcf44e736a076f910b` |
| commitment script | `6a24aa21a9ed91959ba5...076f910b` (38 bytes) |
| txid merkle root | `c1c5d9e383219ba93c39f245df9003d55606d0e9200da01662181fbed13783b4` |
| header hash | `d653c17dc4f81550e06b906d37cb0f30fc8c49062c371a39e84340e780fbf068` |
| assembled block | 956 bytes, 2855 WU |
| coinbase | 186 bytes stripped / 222 bytes full, 780 WU |
| commitment output | 47 vbytes, 188 WU |
| fixed witness overhead | 224 WU (188 output + 36 coinbase witness) |

Every row is asserted: txids/wtxids/witness count, witness root and
commitment, coinbase reproduction, witness injection, full block assembly,
weights, and the malleation split (commitment moves, txids and merkle branch
do not).

## 5. Negative proofs (fail-closed inventory)

- **Strict decode** rejects, per blob: trailing bytes, segwit marker/flag with
  an empty witness, the zero-input serialization ambiguity, and the
  reviewer's raw zero-input vector `020000000001000101000000000000000000000000`
  (marker/flag, zero inputs, decodes and roundtrips byte-for-byte - caught by
  the explicit no-input rejection, never by the roundtrip)
  (`strict_decode_rejects_every_ambiguity`, module twin in `template.rs`,
  shared core `decode_transaction_strict_reason`).
- **Template refresh** fails on any non-canonical template transaction
  (`a_non_canonical_template_transaction_fails_the_refresh`): no skip, no
  donation fallback, no witness stripping.
- **Block assembly** rejects and never submits: wrong reserved value, missing
  commitment, duplicate commitment, malformed commitment shape (wrong header
  length is ambiguous, not a commitment), valued commitment, commitment that
  does not match the recomputed value for the actual transaction set, bad
  transaction index/decode
  (`assemble_witness_rejects_wrong_reserved_and_missing_commitment`,
  `commitment_scan_rejects_duplicates_and_malformed_shapes`,
  `witness_injection_fails_closed_on_bad_coinbases`).
- **Malleation**: a same-txid witness change moves the wtxid, the witness root
  and the commitment, republishing the job even though the txid merkle branch
  is unchanged (`witness_malleation_changes_the_commitment_but_not_the_txids`,
  `a_same_txid_witness_malleation_republishes_the_job`); an unchanged set
  keeps the commitment (`an_unchanged_witness_set_keeps_the_commitment`);
  at main level, both ingresses republish (section 3, B1 test).
- **Merkle discipline**: the txid merkle root is the fold of the *stripped*
  serializations, not the full ones (`the_merkle_root_uses_the_stripped_
  serializations`); the injected witness leaves the coinbase txid unchanged
  (asserted in `witness.rs`/`block.rs`/`coinbase.rs` tests).
- **Settlement**: a `6a24` commitment output does not flip settlement to JD
  mode, while the real JD carrier (`6a23` `SPc`, from
  `spc_crypto::carriers::emit_op_return`) still does - alone and alongside a
  commitment, which must not mask it - and an unknown `OP_RETURN` still
  selects JD mode (`a_witness_commitment_output_is_not_a_jd_carrier`); the
  settlement routing mirrors the 224 WU witness overhead so the exact budget
  boundary (224 + 2 x 172 WU) keeps precisely the ids the mined coinbase
  pays, with the unmirrored counterfactual keeping an unpaid id and the
  witness-free legacy control unchanged
  (`settlement_mirrors_the_witness_overhead_at_the_budget_boundary`).
- **Weight model boundary (R8)**: the coinbase output-count CompactSize
  grows from one byte to three at the 253rd output (+2 vbytes = +8 WU) -
  pinned by `compact_size_growth_at_the_253rd_output_is_8_wu` as a
  documented model limit of the preexisting whole-block weight
  approximation, not redesigned into a hard limit.

## 6. Fable rework disposition (REQUEST_CHANGES)

Review: `docs/reviews/2026-11-08-witness-primitives-fable.md`. Every item is
addressed in this same packet; nothing is deferred except what the review
itself assigns to T4b (R7).

| Item | Disposition |
|---|---|
| B1 (no main/ingress-level malleation-republish proof) | Done. `Published` gains the front job id; `witness_publication::commitment_only_witness_change_republishes_both_ingresses` proves republish on both ingresses with no height/fee change, quiet on an unchanged set, new share accepted / old stale (section 3). |
| B2 (zero-input vector not rejected; txid!=wtxid detection concern) | Done. Explicit `tx.input.is_empty()` rejection before the roundtrip check; the reviewer's raw vector is a frozen negative in both the module tests and the conformance suite, both also asserting `analyze_transactions` errors on it. Witness detection is structural (decoded input witness stacks), never txid/wtxid inequality alone. |
| B3 (BIP 141 facts wrong at several sites) | Done. Reserved value: unconstrained 32 bytes (no SHOULD-random attributed to BIP 141); consensus: >= 38 bytes, highest matching index wins; pool policy: exactly one zero-value 38-byte output, last - stated at every site (`witness.rs` module docs, `DuplicateCommitment`, `looks_like_commitment`, `front.rs` `reserved_for`, `block.rs` duplicate comment, `WITNESS_COMMITMENT_OVERHEAD_WU`). The per-root random cache is kept and documented truthfully as a pool-side share-stability choice. SPEC 2.4.2 amended (the pool-template prohibition targets the SPc `OP_RETURN` carrier, not `OP_RETURN` outputs in general); 3.3 and 4.8 updated; decision record points 3/4/8 restated. |
| R4 (JD carrier is `6a23` SPc, not `6a20`) | Done. Real carrier built by `spc_crypto::carriers::emit_op_return` in comments and tests, alone and with a commitment (which must not mask it); unknown `OP_RETURN` outputs retained as JD-selecting. |
| R5 (settlement must mirror the 224 WU) | Done. `tier_config` adds `WITNESS_COMMITMENT_OVERHEAD_WU` when the mined coinbase carries a commitment-shaped output; boundary test with the unmirrored counterfactual and the legacy control (section 5). |
| R6 (post-credit assembly failure silent) | Done. Bounded `tracing::error` (front id, height, job id, error) states the credit stands and no block is submitted; returns `FrontError::Block`. No `on_share` reorder, no protocol rewrite. |
| R7 (GBT crosschecks) | Unchanged scope: remains part of the T4b handoff (section 9), per the review's own framing. |
| R8 (CompactSize 252->253 +8 WU undocumented) | Done. Documented in `WITNESS_COMMITMENT_OVERHEAD_WU`, SPEC 4.8 and the decision record; boundary test pins it. The whole-block weight model stays the preexisting tier-budget approximation; no ladder redesign, no hard-limit overclaim. |
| R9 (vectors' P2TR signatures not valid spends) | Done. Stated in the conformance module header (section 4); real-key spend validity is T4b. |

## 7. Remaining live gate (what T4a does not claim)

- **No node round-trip.** Canonical-block acceptance is asserted only against
  the frozen vectors and the internal strict re-verification. The commitment
  construction is standard BIP 141, but a real native-PoW mixed block accepted
  by a real node is T4b's gate (section 9), not claimed here. No live T4b
  execution was performed for this packet.
- **`template_rpc.rs` (`rpc` feature) is compile-checked only.** It keeps
  witness-bearing transactions and accounts their fees, but needs a node to
  integration-test; documented here instead of tested, and folded into the
  T4b handoff (R7).
- **OsRng reserved values are not deterministic.** The conformance vectors pin
  the reserved value explicitly; the front's runtime randomness is exercised
  only through the cache/stability tests, not through frozen values.
- **JD mode / JDC side.** Witness handling on the JD side (JDP-supplied
  templates with witness data, JD weights and carriers) is out of T4a scope;
  the JD carrier is `6a23535063...` (`OP_PUSHBYTES_35` SPc) and cannot
  collide with the commitment's `6a24` push.

## 8. Merge instructions for the serial owner

Owned outright (apply whole, no conflicts expected): `witness.rs`,
`tests/witness_conformance.rs`, the decision record and changelog fragment.

Small localized edits (apply as hunks if the files moved): `template.rs`,
`block.rs`, `coinbase.rs`, `template_rpc.rs`, `lib.rs` (one line,
`pub mod witness;`).

`front.rs` is also modified by AP2; apply this packet's changes as focused
hunks, never wholesale:

1. `block::` import list gains `assemble_witness` (and the witness types).
2. `TierConfig` gains `overhead_wu: u32` (0 by default).
3. The job struct gains `witness_plan: Option<WitnessPlan>`; the front gains
   `reserved_cache: Option<([u8; 32], [u8; 32])>`; `WitnessPlan {wtxid_root,
   reserved, commitment}` is new.
4. `invalidate_current_work` clears `witness_plan` only - `reserved_cache`
   survives so unchanged-set republications keep shares valid.
5. Refresh: `analyze_transactions`, then build the plan when
   `witness_count > 0` via `reserved_for(wtxid_root)`.
6. `job_changed` gains `commitment_moved` (old vs new plan commitment;
   `None != Some(commitment)` counts as moved).
7. Tier weight: `overhead_wu.saturating_add(224)` when a plan is present.
8. Coinbase build: append `witness::commitment_output()` **last** when a plan
   is present (pool output stays index 0).
9. `on_share`: with a plan present, assemble via `block::assemble_witness`
   (new `FrontError::Block` variant); on assembly error after the credit,
   log the bounded error and return `FrontError::Block` (R6 - the
   `Ok(assembled) => assembled.into(), Err(error) => { ... }` match);
   `revalidate_share` is unchanged because miners hash the stripped coinbase.
10. `reserved_for`: OsRng draw on cache miss, one entry, keyed by wtxid root
    (doc: the reserved value is unconstrained per BIP 141; the per-root draw
    is the pool's share-stability choice).

`payout.rs` (three touchpoints):

1. `detect_mode`'s JD-carrier predicate gains
   `&& !crate::witness::looks_like_commitment(&o.script)`
   (production change; the comment names the real `6a23` SPc carrier).
2. `tier_config` gains the 224 WU mirror when the mined coinbase carries a
   commitment-shaped output (production change, R5).
3. The tests `a_witness_commitment_output_is_not_a_jd_carrier` and
   `settlement_mirrors_the_witness_overhead_at_the_budget_boundary`.

`main.rs` is also modified by AP2; apply this packet's changes as focused
hunks, never wholesale (all three are new in this rework):

1. The `Published` struct drops `Copy` and gains `job_id: String` (the front
   job id, hex job sequence) - the publication-dedup key member.
2. `published_key` gains `job_id: front.job()?.job_id.clone()` (and the
   struct no longer copies; the `Option<Published>` sites are unchanged).
3. The `#[cfg(test)] mod witness_publication` test module (test-only; apply
   whole, including its `SwappableSource` and SV1/SV2 client helpers).

SPEC.md: this packet amends three dated body passages - 2.4.2 (the
pool-template prohibition targets the SPc `OP_RETURN` carrier, not
`OP_RETURN` outputs in general), 3.3 "Witness commitments (2026-11-08)" (after
the job-lifecycle paragraph) and 4.8 "Witness overhead (2026-11-08)" (after
the autoscaler paragraph) - and adds the fragment
`docs/changelog/2026-11-08-pool-witness-commitment.md` for the merge session
to splice under `changelog:` and delete, bumping `updated:`, per
`docs/changelog/README.md`. No other SPEC text moves; grep for inbound
references to the amended sections found none requiring fixes.

## 9. T4b handoff plan (exact requirements; no live execution here)

Goal: a real native-PoW mixed canonical block, proven by two independent
oracles, on the pinned approved node baseline. This packet performs none of
it; the following is the binding handoff specification.

1. **Node canonical acceptance** (oracle 1): submit the assembled block to
   the node and assert connect-block success with the witness commitment
   visible in the coinbase.
2. **Independent raw-bytes verification** (oracle 2, no shared code with the
   pool): over the full raw block bytes, parse the 80-byte header and re-do
   the proof of work (hash <= target derived from `bits`), re-fold the
   stripped-serialization merkle root, recompute the wtxid merkle root
   (coinbase leaf zero), recompute the commitment, and locate the commitment
   output by Core's consensus rule - the 6-byte header, script >= 38 bytes,
   **highest matching output index wins**.
3. **Separate pool-policy assert**: exactly one zero-value 38-byte
   commitment output, appended last (the pool's stricter rule, distinct from
   oracle 2's consensus rule; both must hold on the same block).
4. **Exact paid-UTXO oracle**: reconstruct the paid UTXO set from the block;
   it must recognize only the expected commitment (and the expected payout
   outputs) and still **fail on any other unknown output** - no fallback
   interpretation.
5. **Baseline**: the actual approved L2 binary, sha256
   `eb257af7c10cdf0ded584adfadf2ae8124466304b13d26c17f855e76f9320f7c`.
   Not a generic `UPSTREAM_DEPS.md` substitute; no v0.9 adoption; nothing
   else may stand in for that exact binary.
6. **Real spends and negatives**: the template must contain a real P2WPKH
   input and a valid Schnorr P2TR key-path input (native PoW on the
   assembled block). Negative controls in the same harness: a wrong reserved
   value must be rejected, and a legacy (witness-free) block must pass
   unchanged as the control.
7. **GBT crosschecks (R7)**: exercise the real `template_rpc` (GBT) path
   against the node in the same harness, closing the compile-check-only gap
   in section 7.
8. **This packet performed no live T4b execution**: no node run, no
   regtest submission, no miner beyond the owned local socket test in
   section 3.

## 10. Everything is uncommitted

Per the packet contract, nothing is committed and nothing outside section 1's
file list was modified. The immutable first-packet manifest
(`/tmp/opencode/T4a-review-manifest.json`) and the baseline manifest
(`/tmp/opencode/LP-final-runtime-manifest.json`) are untouched; this rework's
manifest is `/tmp/opencode/T4a-rework-manifest.json`. This report requests
Fable approval of the reworked packet before any integration into primary;
on approval, hand off to the serial merge owner per section 8.
