# T4a pool witness primitives: Fable review

Date: 2026-11-08
Reviewer: Claude Fable 5.1 (`amazon-bedrock/global.anthropic.claude-fable-5-1`),
subagent session.
Packet: T4a witness primitives (TFF), uncommitted in
`/home/ethan/.local/share/opencode/worktree/ad8886/lab-witness-primitives`,
base `b0ab4ca34fda07c61a6335d05f598eaacc69678c`.
Manifest: `/tmp/opencode/T4a-review-manifest.json`, 375 file records,
snapshot `72c2ffd5f8cdb835e76ef77a71feeae88ae8a238f948f955de878308ef57946a`,
code snapshot (report excluded)
`2fe5aab912878fceae2cc1d387800bf16a65bab9aca287256f3fa8f2679704fb`.
Baseline: approved final runtime
`/tmp/opencode/LP-final-runtime-manifest.json` (370 records, code
`30fedafdbba57b79fe4c13e2de0658aa3f05f527b778a242cbfab9c4ba253d76`, manifest
file SHA-256 `8bbe097de563600ce8bf0b8cb9693b082a9cff9bb51580d96afd89d2c0c3021f`),
frozen source `/tmp/opencode/LP-final-runtime-src-v4`.
Candidate report: `docs/reviews/2026-11-08-witness-primitives-implementation.md`
(in the worktree). Contract: `/tmp/opencode/traffic-plan-approved-r2/plan.md`
T4 (lines 1117-1163) and `peer-review.md` fix 1.

## Result

**REQUEST_CHANGES**, bounded, tied to snapshot `72c2ffd5`.

The primitives themselves are correct. I independently re-derived every
frozen vector (txids, wtxids, stripped serializations, witness root,
commitment, commitment script, coinbase txid, merkle root and branch, header,
header hash, full block, all weights) with my own generic BIP 144 parser and
cross-checked the txid/wtxid/weight of every transaction and of the full
coinbase with Bitcoin Core 32.1 `bitcoin-tx`; all agree with
`/tmp/opencode/T4a-vectors/vectors.json` and with the constants embedded in
`bins/spc-pool/tests/witness_conformance.rs`. The strict decoder, the
zero-leaf wtxid fold, the `SHA256d(root || reserved)` commitment, the
marker/flag + single 32-byte item splice, the stripped-vs-full hashing
discipline and the fail-closed assembler all do what the report says, on the
exact snapshot. 245 lib + 10 conformance tests pass here; clippy
`--all-targets` is clean; `--features rpc` compiles.

Three bounded defects block approval of the packet as written. One is a
functional gap in the claimed job-identity property at the ingress boundary;
two are false normative/factual statements that would otherwise be merged
into SPEC and a binding decision record. None calls the hashing, commitment or
assembly code into question.

- **B1.** A commitment-only job change (same-txid witness malleation, the
  property the packet exists to deliver) bumps the front's `job_id` but is
  never published to miners: `bins/spc-pool/src/main.rs:899-919` keys
  republication on height, prev-hash, txid merkle identity, version, bits and
  the distribution fingerprint, none of which move. Every subsequent share is
  rejected `stale-share` until an unrelated field changes; a block found in
  that window is lost. See section 3.1.
- **B2.** The "zero-input serialization ambiguity is rejected" claim (SPEC
  §3.3 amendment line 319, decision record line 21, report line 137,
  `template.rs:44`) is false on this snapshot and untested: the strict decoder
  accepts the zero-input segwit-marked blob, whose txid and wtxid differ while
  `witness_count` stays 0, so the witness-free `assemble()` would hash it
  raw. Unreachable from a node template (consensus-invalid tx), but it is a
  normative statement. See section 3.2.
- **B3.** The decision record and code comments mis-state BIP 141 in three
  places (a "SHOULD be random" that does not exist, "more than one commitment
  is invalid" where BIP 141 says highest index wins, "exactly 38 bytes" where
  BIP 141 says at least 38), and SPEC §2.4.2 line 226 ("Pool-template mode
  MUST NOT emit the `OP_RETURN`") now contradicts §3.3 without an amendment.
  See section 3.3.

Approval of this packet, once B1-B3 are fixed, does not establish T4b live
canonical acceptance or any Ark readiness. Section 6 records required
corrections to the T4b handoff.

## 1. What I verified and how

### 1.1 Snapshot and delta

- Re-hashed all 375 manifest records in the worktree: 0 mismatches, 0 missing,
  modes match. Recomputed the recipe (`sha256(compact sorted-key JSON
  {base, files})`, rows sorted by path): snapshot `72c2ffd5...` and code
  snapshot `2fe5aab9...` reproduce exactly, before and after my test runs.
- Re-hashed the 370 baseline records against `LP-final-runtime-src-v4`: 0
  mismatches. Diffed candidate against the frozen approved source, not git
  HEAD: 8 modified + 5 added + 0 removed, exactly the declared delta. The 362
  other records are byte-identical to the approved runtime.
- `main.rs`, `sv1_server.rs`, `sv2_server.rs`, `jds.rs`, `spc-accounting`,
  `spc-crypto`, `spc-store` are untouched (which is where B1 lives).

### 1.2 Independent vector re-derivation

`/tmp/opencode/T4a-fable/` (scratch, not in any tree). My parser walks
inputs/outputs/witness generically with CompactSize reads, rejects trailing
bytes and marker-with-empty-witness, and strips independently of
`gen.py`'s fixed 1-in-1-out walk. Results on `vectors.json`:

| Quantity | Independent | Core 32.1 `bitcoin-tx` |
|---|---|---|
| 4 stripped serializations, txids, wtxids | match | txid/hash match (display-order reversed) |
| legacy tx wtxid == txid | yes | yes |
| witness root (zero coinbase leaf) `074a69ec...` | match | n/a |
| commitment `91959ba5...` = SHA256d(root \|\| reserved) | match | n/a |
| commitment script `6a24aa21a9ed` + 32, zero value, last output | match, present in coinbase | decodes |
| coinbase stripped -> full splice; txid `8a51bac9...` unchanged | match | txid match, weight 780 |
| merkle root `c1c5d9e3...` over `[cb_txid] ++ txids` | match | n/a |
| merkle branch (deepest-first) and fold -> root | match | n/a |
| header bytes, LE fields, hash `d653c17d...` | match; meets `0x207fffff` | n/a |
| block = header \|\| 0x05 \|\| coinbase_full \|\| txs, 956 B | match | n/a |
| weight 3*633 + 956 = 2855 WU; 188 + 36 = 224 WU | match | n/a |
| `tr1_malleated`: same txid, different wtxid | match | txid match, hash differs |

All test-file constants equal `vectors.json` (checked programmatically). The
vectors are structural: the P2WPKH/P2TR witnesses are byte patterns, not valid
signatures, as the report says. The P2TR case is a real `5120` output with a
single 64-byte witness item (key-path shape), not just a label.

### 1.3 Tests run (bounded, `-j2`, existing target dir)

- `cargo test -p spc-pool --test witness_conformance`: 10 passed.
- `cargo test -p spc-pool --lib`: 245 passed, 0 failed.
- `cargo clippy -p spc-pool --all-targets`: no warnings.
- `cargo check -p spc-pool --features rpc`: clean.
- Scratch probe (section 3.2) against `bitcoin 0.32.102` from `Cargo.lock`.

No node, miner, services, Nix build or full-workspace build.

## 2. Primitive correctness (scope items, in order)

**Full vs stripped serialization; txid/wtxid.** `template.rs:87-104`
derives txids via `compute_txid()` (stripped) and wtxids via
`compute_wtxid()` (full) from the strictly decoded transaction; legacy wtxid
== txid falls out of rust-bitcoin's serialization and is asserted
(`analyze_separates_txids_from_wtxids`, conformance test 1). Correct.

**Merkle discipline.** The txid tree and the coinbase branch use the stripped
txids; `the_merkle_root_uses_the_stripped_serializations` proves that
hashing full bytes gives the wtxids and a different root. `witness_root`
prepends the zero leaf and reuses the same duplicate-last fold Core uses.
Header is LE version/time/bits/nonce with internal-order hashes; `bits` from
GBT is `from_be_bytes` of the 4-byte field then `to_le_bytes` in the header,
consistent with Core. Correct.

**Strict decode.** `deserialize` + byte-exact re-serialization rejects
trailing bytes, truncation, garbage, empty, and marker/flag with all-empty
witness (Core's "Superfluous witness record"), flag != 1 (rust-bitcoin
"unsupported segwit version"). Gap: zero-input blob accepted (B2).

**Coinbase witness.** `block.rs:183-210` splices `00 01` after the version
and `01 20 <reserved>` before the locktime, re-strict-decodes the result and
asserts txid unchanged. Preconditions: input strict decode and "no witness
already present". Exactly one 32-byte item, only in the full block; SV1/SV2
prefix/extranonce/suffix and `revalidate_share` (`front.rs:1418-1446`) hash
the stripped coinbase. Correct, matches contract items 1 and 3.

**Commitment output.** `witness.rs:62-77`: 38 bytes `6a24aa21a9ed` + 32,
value 0, appended last (`front.rs:884-886`), pool output stays index 0.
`coinbase_commitment` rejects duplicates and any header-prefixed script that is
not exactly 38 bytes; `assemble_witness` additionally rejects missing,
valued and mismatched commitments and recomputes the commitment from the
actual transaction set. As a policy for the pool's own coinbase this is
correctly fail-closed (contract item 4). The accompanying BIP 141 claims are
wrong (B3) but the behaviour is stricter than consensus, not looser.

**Mixed block assembly.** `assemble_witness` strict-decodes every tx, folds
stripped txids, injects the witness, recomputes root/commitment, writes the
CompactSize tx count, and emits the full serializations. The whole block
re-decodes as `bitcoin::Block` with the coinbase witness in place (block.rs
test, conformance test 6). Correct.

**`assemble()` and all callers.** `block::assemble` still hashes raw bytes
(`block.rs:142-144`). Its only caller is the `None` arm at
`front.rs:1091-1099`, reached only when `witness_plan` is `None`, i.e.
`witness_count == 0`, which under strict decode means every tx is
non-segwit-serialized, so raw hash == txid. `revalidate_share` and the test
helper `miner_merkle_root` hash the stripped coinbase, which is correct by
construction. `block::txid` is not applied to non-coinbase raw bytes
elsewhere. The one hole is B2 (zero-input blob makes `witness_count == 0`
while raw hash != txid); closing B2 closes it.

**Template acquisition (`rpc` feature).** `template_rpc.rs:41-67` requests
GBT with the `segwit` rule, keeps every transaction in GBT order (parent
dependencies preserved), strict-decodes each, sums all fees, and
`coinbase_value = subsidy + fees` where `subsidy = gbt.coinbase_value -
fees`; so `coinbase_value == gbt.coinbase_value`, no inflation. Witness
retention: full `raw_tx` bytes are stored and re-emitted; `submit_block`
decodes to `Block` and re-serializes, which is byte-identical because every
tx passed the roundtrip check. The former witness filter is gone globally,
which is the contract. JD/type-32: `jds.rs` validation and
`set_job_txids` (stripped txids) are unchanged; no JD witness path was
enabled - a JD block's commitment is still the JDC's problem, as the record
says. Compile-checked only, as reported.

**Weight budget.** 224 = 188 (8 + 1 + 38 = 47 vB) + 36 (2 + 1 + 1 + 32)
verified against the serialized bytes. `saturating_add` on `u32`
(`front.rs:816-823`) is fine; `route_tiers` compares with saturating
arithmetic. The commitment output is not counted against
`max_payout_slots` (only `kept` is), which is the right reading of a payout
slot cap. The 252 -> 253 CompactSize output-count step (+8 WU) is not
modelled by the 172-WU-per-output budget either before or after this packet;
with the commitment it is now crossed at 251 payout outputs instead of 252.
Pre-existing approximation, operator `overhead_wu` territory; noted, not
blocking. Whole-block weight is not checked anywhere in the pool (also
pre-existing; the GBT path relies on the node's coinbase reservation).

**Exact payment unaffected.** Distribution outputs are unchanged; the
commitment adds a zero-value output after them, so `sum(outputs) == T`
still holds and settlement's `RevenueMismatch` check sums it as 0. No
donation path: a zero-value `OP_RETURN` is unspendable and worthless. The
`spc-crypto` carrier parser (`carriers.rs:201-224`) matches exactly 37 bytes
`6a23 'SPc'`, so it never mistakes the 38-byte commitment for a carrier.

**Job identity and reserved cache.** `commitment_moved` (`front.rs:728-736`)
compares old/new plan commitments including `None != Some`; the front test
`a_same_txid_witness_malleation_republishes_the_job` proves the front-level
bump. `reserved_for` (`front.rs:1260-1271`) is a one-entry cache keyed by
wtxid root, OsRng on miss; unchanged sets keep the commitment (test
`an_unchanged_witness_set_keeps_the_commitment`); `invalidate_current_work`
clears the plan but keeps the cache, so a failed refresh does not strand
issued shares. Cache eviction on an A -> B -> A root sequence redraws for A,
which only causes a republish (template_id also moves, so it is published).
Reserved reuse across prev-hash changes is harmless: the value has no
consensus meaning. The semantics are "random once per witness root", as the
report says - but the standard it claims to deviate from does not exist (B3).

**Races.** `on_share` rejects `share.job_id != job.job_id` before anything
else, and `witness_plan`/`template`/`job` are set together at the end of
`refresh_inner` and cleared together, so the plan used at assembly is always
the current job's. A share that passes `revalidate_share` has the job's
coinbase, hence the job's commitment, hence `assemble_witness` cannot fail
for it; if it ever did, the share has already been credited and the nullifier
consumed (`front.rs:1041-1065`) and `main.rs:420-425,858-866` reports it to
the miner as `invalid-share`. Unreachable, but see R6.

**Network target check.** `front.rs:1067-1072` unchanged, still the only
block gate, still fail-closed; invalid templates invalidate work
(`refresh`, `front.rs:656-677`), tested by
`a_non_canonical_template_transaction_fails_the_refresh`.

**Settlement `detect_mode`.** `payout.rs:830-841` excludes any script with
the 6-byte commitment prefix from the JD-carrier predicate (prefix match, not
exact length, which is the right choice here since consensus allows longer
commitments from a JDC/Core template). It still selects JD for any other
`OP_RETURN`, does not accept unknown or valued outputs any differently than
before, and does not bypass order verification. The negative test is
meaningful for the mode flip, but its "JD carrier" is `6a20` + 32, which is
not the §2.4.2 carrier (`6a23 535063` + 32); see R4.

**Privacy.** No wire format, stream payload, `BlockEvent`, `SliceEvent` or
`ark`-role surface changed; the reserved value is not an identity field and
is not emitted anywhere except inside the block. Firewall unchanged.

## 3. Findings

### 3.1 B1: commitment-only job changes never reach the ingresses

`bins/spc-pool/main.rs:899-919`:

```rust
struct Published { height, prev_hash, template: [u8;32] /* txid merkle id */,
                   version, bits, fingerprint /* distribution */ }
```

`refresh_and_publish` (`main.rs:965-996`) returns at line 988-990 when
`published_key(front) == *published`, without calling `sv1.publish`,
`sv2.set_job_txids` or `sv2.publish`. Counterexample: template T1 with
witness tx W; refresh -> job `J1`, commitment `C1`, published. Node replaces
W's witness (same txid; Core permits witness replacement) -> T2. Refresh:
`txset.txids` unchanged so `template_id` unchanged; fees, height, prev-hash,
bits, version unchanged; `distribution.fingerprint`
(`spc-accounting/src/lib.rs:1064-1075`) covers height, `a_x` and the payout
outputs only, so unchanged; `commitment_moved` is true (`front.rs:728-729`)
so `job_seq` bumps to `J2` and the front's coinbase now carries `C2`. But
`published_key` is identical, `refresh_and_publish` returns `false`, miners
keep `J1`. Every share: `front.rs:965-967` -> `StaleJob` -> `stale-share`.
A block-finding share on `J1` is also rejected (and would have failed
`assemble_witness` with `CommitmentMismatch` against T2 anyway). Duration:
until the next height/prev-hash/tx-set/fee/distribution change. The
front-level test at `front.rs:2484` cannot see this; the gate is in `main`.

Required change (one line of substance): include the job identity in
`Published`, e.g. `job_id: u32` from `front.job()?.job_id` (parsed) or the
job's `coinb2` hash, or the plan commitment. Including `job_id` is strictly
safer: every `bumped` path should republish. Required check: a `main`-level
or integration test that a commitment-only template change produces an SV1
`mining.notify` / SV2 `NewExtendedMiningJob` with the new job id. `main.rs`
is outside the packet's declared file list and is AP2-adjacent; route the
hunk through the serial merge owner, but the packet cannot claim
"republishes the job" until it lands.

### 3.2 B2: zero-input blob accepted by the strict decoder

`bins/spc-pool/src/template.rs:53-61`. rust-bitcoin serializes a zero-input
transaction with the segwit marker/flag and decodes it back, so the roundtrip
passes. Probe on `bitcoin 0.32.102` (same as `Cargo.lock`):

```
raw = 020000000001000101000000000000000000000000
decode_transaction_strict -> Ok, inputs = 0, witness_count = 0,
compute_txid != compute_wtxid
```

Core's deserializer throws "Superfluous witness record" on this blob, so the
pool is looser than the node here. Consequences on this snapshot: the SPEC
§3.3 amendment (line 319), the decision record (line 21), the report (line
137) and the `template.rs:44` comment assert a rejection that does not
happen; `strict_decode_fails_closed_on_every_ambiguity`
(`template.rs:200`) and conformance test 2 contain no zero-input case; and if
such a blob ever reached `refresh_inner`, `witness_count == 0` would route a
winning share through `assemble()` which hashes it raw (= wtxid), producing a
header whose merkle root disagrees with the issued branch. Practically
unreachable (a node never templates a vin-empty tx), but the normative text
must be true.

Required change: in `decode_transaction_strict_reason`, reject
`tx.input.is_empty()` explicitly (matches Core's outcome), and add the
zero-input vector to both negative tests. Recommended belt: in
`analyze_transactions`, count a transaction as witness-bearing iff
`compute_txid() != compute_wtxid()` (equivalently, `raw[4] == 0x00`), so the
plan decision cannot diverge from the serialization form; or make the `None`
arm at `front.rs:1091` use decoded txids too. Then fix the four text sites or
keep them, now true.

### 3.3 B3: false BIP 141 statements and a SPEC conflict

I read BIP 141 (`bip-0141.mediawiki`, "Commitment structure") directly:

- It says the commitment script "must be at least 38 bytes", with "39th byte
  onwards: optional data with no consensus meaning"; "if there are more than
  one scriptPubKey matching the pattern, the one with highest output index is
  assumed to be the commitment"; and the reserved value "currently has no
  consensus meaning". There is **no** SHOULD about randomness. Bitcoin Core's
  `GenerateCoinbaseCommitment` uses an all-zero reserved value.
- Candidate text contradicting this: decision record lines 37-45 ("BIP 141
  says the witness reserved value SHOULD be chosen (pseudo-)randomly ...
  deliberate, recorded deviation from a literal per-block reading of the
  SHOULD") and line 85; `front.rs:1255-1259` ("BIP 141's SHOULD-random is
  thus satisfied"); `witness.rs:12-15` and `:50-51` ("BIP 141 makes more
  than one invalid"), `block.rs:606`; decision record lines 34-36 and
  `witness.rs:69-71` describing a non-38-byte header-prefixed script as
  "ambiguous"/not a commitment.
- The *behaviour* is acceptable: random-per-root is harmless (the value is
  unconstrained), rejecting duplicates and non-38-byte shapes in the pool's
  own coinbase is a stricter policy. But a binding decision record must not
  cite a nonexistent standard as the thing it deviates from, and the T4b
  independent decoder (section 6) must implement Core's rule (>= 38 bytes,
  highest index) when reading a *node-accepted* block, or it will diverge
  from the oracle it is supposed to shadow.
- SPEC §2.4.2 line 226: "Pool-template mode MUST NOT emit the `OP_RETURN`
  (it would be a protocol fingerprint with no benefit)." After §3.3's
  amendment the pool-template coinbase does emit an `OP_RETURN`. The intent
  is clearly the SPc carrier, and the BIP 141 commitment is present in every
  segwit block so it is no fingerprint, but the normative sentence must be
  amended ("MUST NOT emit the SPc `OP_RETURN` carrier; the BIP 141 witness
  commitment output (§3.3) is not a carrier"). The report's "grep for
  inbound references ... found none requiring fixes" missed this.

Required change: rewrite decision point 4 as "BIP 141 leaves the reserved
value unconstrained (no consensus meaning; Core uses zero); this pool draws
it from the OS RNG once per witness root and caches one entry so unchanged
sets keep issued shares valid", drop the "SHOULD"/"deviation" language in the
record, `front.rs` and the compatibility bullet; reword `witness.rs:12-15,
50-51, 69-71` and `block.rs:606` to "stricter than BIP 141 for the pool's
own coinbase (BIP 141 would pick the highest index / accept optional
trailing bytes)"; amend §2.4.2 line 226 in the same packet and list §2.4.2
in the changelog fragment and record. Consider whether random reserved buys
anything; zero would make every job byte-reproducible from the template and
let T4b compare directly against GBT's `default_witness_commitment`. Either
is acceptable if documented truthfully.

### 3.4 Ranked remaining items (non-blocking, small)

- **R4** `payout.rs:831-835,2054,2065-2071`, `witness.rs:82-83,163`,
  decision record line 71: the JD carrier is described as `6a20` + 32. SPEC
  §2.4.2 and `spc-crypto/src/carriers.rs:170-224` define it as `OP_RETURN
  OP_PUSHBYTES_35 "SPc" <32>` = `6a23 535063` + 32 (37 bytes). The
  `detect_mode` logic is unaffected (any non-commitment `OP_RETURN` selects
  JD) but the comments are wrong and the test's positive JD vector is not the
  real carrier. Fix the comments; add the real `6a23535063...` carrier to
  `a_witness_commitment_output_is_not_a_jd_carrier`, and a case with both
  carrier and commitment present (JD block from a segwit-aware JDC) -> `Jd`.
- **R5** `payout.rs:985-994` `tier_config` passes `cfg.overhead_wu` without
  the 224 WU the front added (`front.rs:816-823`), so the settlement
  `route_tiers` cross-check (`payout.rs:675-690`) can warn "tier routing
  differs from the mined coinbase" on budget-bound witness blocks. Payment is
  value-driven from the mined coinbase, so no sats move; add
  `WITNESS_COMMITMENT_OVERHEAD_WU` when
  `coinbase.iter().any(|o| looks_like_commitment(&o.script))` to keep the
  cross-check honest. Document in §4.8 that settlement mirrors the overhead.
- **R6** `front.rs:1079-1090` + `main.rs:420-425,866`: a `FrontError::Block`
  surfaces to the miner as `invalid-share` after the share was credited and
  its nullifier consumed, and the block is dropped. Structurally unreachable
  for a share that passed `revalidate_share`; still, log at error level with
  the `BlockError` and emit evidence, because it would mean the plan and the
  job disagree. No reordering needed in this packet (AP2 touches `on_share`).
- **R7** `template_rpc.rs`: GBT already returns `transactions[].txid`,
  `transactions[].hash` and `default_witness_commitment`. Cheap fail-closed
  cross-checks at template time: computed txid/wtxid must equal the node's;
  `witness_commitment(root, [0;32])` must equal `default_witness_commitment`.
  This is exactly the "independent" confirmation T4b needs and costs no node
  round-trip of a block. Recommended for T4b, not required here.
- **R8** Budget model: the 252 -> 253 output-count CompactSize step (+8 WU)
  and total block weight are unmodelled (pre-existing). Record in §4.8 that
  `overhead_wu` is expected to cover the count prefix, or add 8 WU when
  `kept + 2 > 252`.
- **R9** Report §4 says "P2TR key-path spend"; the vector is a key-path
  *shape* (64-byte item) with a pattern signature. Fine, already stated as
  structural; T4b must spend a real P2TR key-path output.

## 4. SPEC / decision / changelog integration

- Body amendments to §3.3 (lines 315-341) and §4.8 (lines 655-660) are
  dated, self-contained and cite the record. Frontmatter untouched; fragment
  `docs/changelog/2026-11-08-pool-witness-commitment.md` follows
  `docs/changelog/README.md` (one `- "..."` bullet, two-space indent). The
  merge owner splices and bumps `updated:`. Complete as a procedure.
- Conflict resolution: `DESIGN.md > SPEC.md > lower docs`; nothing in
  DESIGN.md touches witness commitments, so SPEC §3.3 is the authority. The
  §2.4.2 MUST NOT (B3) is an intra-SPEC conflict that must be resolved in the
  same change, and the §3.3 zero-input sentence (B2) must become true or be
  removed.
- §4.8's surrounding paragraph says the budget no longer trims (ladder
  decision 2026-11-03) while the lab runtime's `route_tiers` still trims by
  `budget_wu`. Pre-existing drift, not this packet's; the 224 WU amendment is
  consistent with the code as it stands.
- Shared touchpoints: `front.rs` hunks are localized (import, two struct
  fields, plan build, `commitment_moved`, overhead, output push, `on_share`
  match, `reserved_for`, `invalidate_current_work`); `payout.rs` is one
  predicate plus one test. `/tmp/opencode/AP2-payout-durable.rs:832-838`
  still has the baseline `detect_mode`, so the hunk applies cleanly if AP2
  merges first. Serial application through the merge owner as the report
  proposes is correct; add the `main.rs` `Published` hunk (B1) to that list.

## 5. Report accuracy

Confirmed: snapshot/delta claims, 245 lib tests, 10 conformance tests,
clippy clean, rpc compile, every §4 vector row, §5 negatives except the
zero-input rejection (B2), §6 scope limits, §7 merge list (minus `main.rs`).
Not confirmed: "BIP 141 SHOULD-random deviation" (B3, the standard does not
say that), "zero-input serialization ambiguity rejected" (B2), "grep for
inbound references found none requiring fixes" (B3, §2.4.2). No test logs
were provided; I ran the tests on the verified snapshot and re-verified the
hash afterwards.

## 6. T4b handoff: required corrections (bounded, not an authorization)

Report §8 item 5 states "The strict oracle is the node; no local
re-implementation of consensus." That weakens the accepted T4 contract
(`plan.md` lines 1155-1163; peer-review fix 1): acceptance is node canonical
acceptance **plus** independently decoded raw-header PoW, stripped-txid
merkle root, wtxid root, commitment and full-block round-trip, plus the exact
payment oracle, and "canonical acceptance is evidence in addition to, never
instead of" the front's network-target check. The handoff must say so.
Specifically:

1. **Oracle**: independent decoder (the `testing/lab-live/oracle.py`
   discipline) over the node-returned raw block: header PoW against the
   template's `bits`, stripped-txid merkle root == header root, coinbase
   witness == exactly one 32-byte item, witness root with zero leaf,
   commitment == SHA256d(root || item) == the 38-byte output at the
   **highest** matching index with `>= 38` bytes (Core's rule, B3), full
   block bytes == what the pool submitted. Node `submitblock` success +
   `getblock` verbosity 2 agreement are evidence alongside, not instead.
2. **Payment oracle**: `testing/regtest-lab-v2-e2e.py:1475-1490`
   (`actual_paid_outputs`) currently raises `unknown_coinbase_output` on any
   non-payout output after index 0, so it will reject every witness block.
   Per contract item 5 it must recognize exactly one zero-value 38-byte
   BIP 141 output at the last index and keep the unknown-output check for
   everything else. This is T4b work; T4a correctly did not touch it.
3. **Baseline node**: not "the approved rbitcoin pin in `UPSTREAM_DEPS.md`"
   (that row is a reference point, not a gate, and does not identify the lab
   patch). The approved L2 binary is SHA-256
   `eb257af7c10cdf0ded584adfadf2ae8124466304b13d26c17f855e76f9320f7c`,
   source snapshot `d2f37b91...` (`docs/LAB-RUNTIME.md` sections 3 and 11);
   `v0.9.0+lab patch` (`09bcb6...`) was validated, not adopted. T4b runs on
   `eb257af7` unless a separate node decision says otherwise; this review
   authorizes no other node.
4. **GBT cross-checks** (R7) belong in T4b's harness as the cheapest
   independent confirmation of txid/wtxid/root before any block is mined.
5. **Negative live variants** as in report §8 item 4, plus: a block whose
   coinbase carries a commitment for a *different* reserved value (node must
   reject `bad-witness-merkle-match`), and a witness-free template on the
   same harness (no commitment, no witness, byte-identical legacy path).
6. Real P2TR key-path spend with a valid Schnorr signature and a real P2WPKH
   spend, both mined by native pool PoW through the front, not injected.

## 7. Scope statement

This review covers concrete primitive correctness on snapshot `72c2ffd5`.
It does not assess the node compatibility profile, Ark, the ladder, or any
live canonical acceptance; approval after B1-B3 is approval of the
primitives and their minimal wiring only. No candidate or primary code was
edited; the only file written is this review. Scratch verification lives in
`/tmp/opencode/T4a-fable/` (independent parser, zero-input probe).

---

# Rework review (same reviewer, confined to B1-B3 and R4-R9)

Date: 2026-11-08 (later session)
Candidate: same worktree, frozen `/tmp/opencode/T4a-rework-manifest.json`
(file SHA-256 `579d248d8c82441057870f2d255c5b10ed38bb5cd9613efd007ff91c13691fdf`),
375 records, snapshot
`b9961746533632eaaa64eb30a508640cc95d9e234521cd81fdebc1a3795439f9`, code
snapshot `c3f9b53ac6ac8f7e4e496b72b5d04e8f73427931e0037c7ee48c0bc9f2f86688`.
The first-pass manifest (`72c2ffd5`), `gen.py` and `vectors.json` are
untouched (mtimes 02:27 / 01:32; conformance constants still equal
`vectors.json`).

## Result

**APPROVE**, bounded to snapshot `b9961746`. B1, B2, B3 and R4, R5, R6, R8,
R9 are resolved within the confined scope; R7 is correctly left in the T4b
handoff. No new findings that would block. Two cosmetic nits are listed in
section 4 for the merge owner's discretion; neither changes behaviour.

Approval scope is unchanged from the first pass: the primitives and their
minimal front/main/payout wiring, on this snapshot. It does not establish
T4b live canonical acceptance, any Ark readiness, or the node compatibility
profile. Primary integration remains pending the serial AP2 merge of the
`front.rs`, `payout.rs` and `main.rs` hunks.

## 1. Verification

- Re-hashed all 375 records against the worktree: 0 mismatches, modes
  match; recipe reproduces snapshot `b9961746...` and code `c3f9b53a...`
  exactly, before and after my test runs. Delta vs the approved baseline
  (370, `30fedaf...`): 9 modified + 5 added + 0 removed, as declared; vs
  `72c2`: 11 files changed, the expected set (`main.rs` newly modified;
  `coinbase.rs`, `lib.rs`, `template_rpc.rs` unchanged since `72c2`).
- Targeted runs on the verified snapshot, `-j2`, existing target dir:
  `cargo test -p spc-pool --bin spc-pool` 2 passed (incl.
  `witness_publication::commitment_only_witness_change_republishes_both_ingresses`,
  0.95 s); `--lib` 247 passed; `--test witness_conformance` 10 passed;
  `cargo clippy -p spc-pool --all-targets` no warnings; `cargo fmt --
  --check` clean; `cargo check --features rpc` clean. The 11 pre-existing
  integration tests were not re-run (unchanged files; the report's 270
  total is consistent with 247 + 2 + 21). No node, miner, services or Nix.

## 2. Disposition of the blocking items

**B1 - resolved.** `main.rs:899-930`: `Published` drops `Copy`, gains
`job_id: String` from `front.job()?.job_id`; the doc comment states why the
job id is load-bearing. The new `#[cfg(test)] mod witness_publication`
(`main.rs:1037-1456`) is a real main-level test: it binds the production
`Sv1Server`, `Sv2Server` and `JdsServer` on `127.0.0.1:0`, connects a real
SV1 line client (subscribe/authorize/notify) and a real SV2 noise client
(SetupConnection, OpenExtendedMiningChannel, NewExtendedMiningJob), and
drives the production `refresh_and_publish` with a swappable template
source. I checked it is not vacuous:

- It exercises the dedup key, not just the front: the second
  `refresh_and_publish` asserts `true` after a same-txid witness swap in
  which height, prev-hash, bits, version, fees (0), txid-merkle identity
  and distribution fingerprint (no shares, pool output only) are all equal.
  On the `72c2` `Published` that call returned `false` (my section 3.1
  counterexample), so the assertion fails without the `job_id` field.
- Both ingresses are asserted independently: SV1 `mining.notify`
  `params[0]` and SV2 `NewExtendedMiningJob.job_id` differ from the first
  publication and agree with each other (`{:08x}`).
- The 400 ms quiet asserts follow positive frame receipt on the same
  connections, so a live channel is proven before quiet is asserted; any
  stray publication would fail them, not pass them.
- Binding: `block_finding_share` rebuilds the coinbase from the *new*
  job's `coinb1`/`coinb2`, folds the issued branch, searches a real nonce
  at `0x207fffff`, and `on_share` returns `Some(block)` only because
  `assemble_witness` recomputed the commitment from the current (malleated)
  `template.transactions` and matched the coinbase; the same share with
  the old job id is `StaleJob`. (Optional strengthening: also decode
  `block.block` and assert `txdata[1]` equals the malleated bytes; not
  required.)
- Workers: `bind` returns `JoinHandle`s; dropping them does not abort, but
  the test owns its own `#[tokio::test(multi_thread)]` runtime, so the
  accept loops end with the test. `_share_rx` is held so `share_tx` sends
  cannot error. No cross-test leak.

**B2 - resolved.** `template.rs:53-68`: explicit `tx.input.is_empty()`
rejection before the roundtrip, with the reason "transaction has no
inputs". My raw vector `020000000001000101000000000000000000000000` is a
frozen negative in `template.rs:224-242` (byte array transcribed and
asserted equal to the hex, rejected by both `decode_transaction_strict` and
`analyze_transactions`) and in `witness_conformance.rs:145-151`. Witness
detection stays structural (`template.rs:100`, decoded input witness
stacks). The SPEC §3.3 sentence about the zero-input ambiguity is now true
for every zero-input encoding. The reference vectors still match.

**B3 - resolved.** All false BIP 141 statements are gone from `witness.rs`
(module docs lines 12-28, `DuplicateCommitment` 78-83,
`looks_like_commitment` 113-119), `block.rs:394-396`, `front.rs:1275-1281`,
the decision record (points 3, 4, 8 and the rework section) and SPEC §3.3.
Each site now states: reserved value unconstrained (no SHOULD), consensus
rule >= 38 bytes with highest matching index winning, and the pool's
own-coinbase policy as deliberately stricter (exactly one zero-value
38-byte output, last). The per-root OsRng cache is unchanged and no longer
described as a deviation. SPEC §2.4.2 line 226 now prohibits the SPc
`OP_RETURN` carrier specifically and names the BIP 141 commitment as not a
carrier; the fragment and record list §2.4.2.

## 3. Disposition of R4-R9

- **R4 - resolved.** `payout.rs:830-846` and `witness.rs:113-119` describe
  the carrier as `6a23 "SPc"` via `spc_crypto::carriers::emit_op_return`;
  `a_witness_commitment_output_is_not_a_jd_carrier` (`payout.rs:2082-2119`)
  uses the real emitter alone (-> `Jd`), alongside a commitment (-> `Jd`,
  commitment does not mask it), and keeps an unknown `6a20` `OP_RETURN` as
  JD-selecting. Unknown-output handling is otherwise unchanged.
- **R5 - resolved.** `payout.rs:989-1012` `tier_config` adds
  `WITNESS_COMMITMENT_OVERHEAD_WU` when any mined output has the 6-byte
  prefix. `settlement_mirrors_the_witness_overhead_at_the_budget_boundary`
  (`payout.rs:2132-2286`) pins the boundary (224 + 2 x 172): the mirrored
  routing keeps exactly the paid ids, the unmirrored counterfactual keeps a
  third unpaid id, and a legacy coinbase at its own boundary gets overhead 0.
  No double count: the front adds 224 once at refresh and settlement adds
  224 once from the mined coinbase, in separate processes.
- **R6 - resolved within scope.** `front.rs:1080-1109`: on
  `assemble_witness` error after the credit, a bounded `tracing::error`
  (front id, height, job id, error; no identity fields) states the credit
  stands and nothing is submitted, then returns `FrontError::Block`. No
  reorder of `on_share`. The miner-facing mapping at `main.rs:858-866`
  (`_ => "invalid-share"`) is unchanged; the report does not claim
  otherwise, which is accurate.
- **R7 - deferred to T4b**, as my first pass framed it.
- **R8 - resolved as documentation + test.** `block.rs:439-460` pins the
  252 -> 253 CompactSize growth (+2 vB = +8 WU) on canonical coinbases;
  `WITNESS_COMMITMENT_OVERHEAD_WU` docs, SPEC §4.8 and the record state the
  model limit truthfully and make no whole-block weight guarantee.
- **R9 - resolved.** Conformance header lines 9-14 state the signatures are
  structural; spend validity is T4b.

## 4. Non-blocking nits (merge owner's discretion)

- `witness.rs:191` test comment still calls a `6a20` + 32 script "An
  OP_RETURN carrier"; it is now an *unrelated* `OP_RETURN` (the carrier is
  `6a23`). Comment-only.
- `main.rs` test: assert the assembled block's `txdata[1]` is the
  malleated serialization (section 2, B1) to make the full-tx binding
  explicit rather than implied by `assemble_witness`.

## 5. T4b handoff (report section 9) - checked, not executed

The handoff now specifies the dual oracle (node canonical acceptance plus
independent raw-bytes PoW / stripped-txid merkle / wtxid root / commitment
located by Core's >= 38-byte highest-index rule / full-block round-trip),
a separate pool-policy assert, the exact paid-UTXO oracle that still fails
on unknown outputs, the approved L2 binary
`eb257af7c10cdf0ded584adfadf2ae8124466304b13d26c17f855e76f9320f7c` (not
the generic pin, no v0.9 adoption), real P2WPKH and valid-Schnorr P2TR
spends, wrong-reserved and legacy-control negatives, and the deferred GBT
cross-checks. That matches the accepted T4 contract and my first-pass
section 6. No T4b work was started here.

## 6. Scope statement

Review-only: no candidate or primary code edited, no commits, no
subagents, no node or mining, no shared cache deletions. The only write is
this appended section. Scratch artifacts remain in `/tmp/opencode/T4a-fable/`.
