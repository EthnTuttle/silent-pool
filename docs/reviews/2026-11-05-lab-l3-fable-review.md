# L3 lab contracts: Fable review (first pass)

Date: 2026-11-05
Reviewer: Claude Fable 5.1 (`amazon-bedrock/us.anthropic.claude-fable-5-1`),
session `ses_e524e6989ffelCFmkVUllSKJla`.
Packet: `lab-contracts` (TFF DeepSeek), uncommitted in
`/home/ethan/.local/share/opencode/worktree/ad8886/lab-contracts`, base
`b0ab4ca34fda07c61a6335d05f598eaacc69678c`.

## Result

**REQUEST_CHANGES**, tied to snapshot
`6a2a5967113417e6ed8c348f11a6498c98e24685b21f2548f0d34b62b0f935f0`.

The packet is close. The versioned v1/v2 separation, the exact family
`S(m) = (R + 1)/m - 1`, the `Q(T) = ceil(N (R + 1)/(T + 1))` window with its
`< E(R)` ceiling bound, the evidence-state table, the no-absence-invalid rule,
the adoption/reap algorithm and the netns backend are all implementable as
written, and every arithmetic vector I recomputed independently matches. Four
defects block approval; each is a bounded text change in a decision record or
the contract document, not a redesign. Items 5-12 are non-blocking and may be
folded into the same revision or deferred with a note.

The TFF implementer should revise and return; I will re-review against a new
manifest. No candidate file was edited by this review.

## Snapshot verification

```sh
cd /home/ethan/.local/share/opencode/worktree/ad8886/lab-contracts
git rev-parse --short HEAD            # b0ab4ca
git status --short                    # exactly the seven ?? files in the manifest
python3 - <<'EOF'
import json,hashlib
m=json.load(open('/tmp/opencode/lab-l3-review-manifest.json'))
for f in m['files']:
    assert hashlib.sha256(open(f['path'],'rb').read()).hexdigest()==f['sha256'], f['path']
body=json.dumps({k:v for k,v in m.items() if k!='snapshot_sha256'},separators=(',',':'))
assert hashlib.sha256(body.encode()).hexdigest()==m['snapshot_sha256']
print("manifest ok")
EOF
```

All seven per-file SHA256 values match. The `snapshot_sha256` reproduces as
`sha256` of the manifest's compact JSON (`base` + `files`, no spaces), so the
snapshot is bound to exactly these bytes.

Implementer validation re-run from the worktree:

```sh
python3 scripts/check-lab-docs.py     # OK: 6 files, ASCII clean, links resolve (3 EXPECTED merge inputs)
git diff --check                      # exit 0 (tracked tree only; untracked files scanned separately)
LC_ALL=C grep -nP '[^\x00-\x7F]' <seven files>   # no output
grep -nE ' +$' <seven files>                     # no output
```

Both claims in `docs/reviews/2026-11-05-lab-l3-contracts.md` hold.

## Independent arithmetic

Run with Python integers/`fractions.Fraction`; no floats on the way to any
integer result.

```text
bits 0x200fffff: T = 0x0fffff << 232 (252 bits);  E(T) = 2^256/(T+1) = 16.000015258...
bits 0x207fffff (regtest pow limit): T_pl = 0x7fffff << 232 (255 bits) = 2^255 - 2^232

R = 2^254 - 1 (the recommended reference):
  m=1 S=2^254-1 E=4   S>=T yes   S>=T_pl NO
  m=2 S=2^253-1 E=8   S>=T yes   S>=T_pl NO
  m=4 S=2^252-1 E=16  S>=T yes   S>=T_pl NO
  m=8 S=2^251-1 E=32  S>=T NO
  Q(T) = 8*2^254/(T+1) = 32.0000305... -> ceil 33, floor 32           (matches both records)
  ceil error Q*E(R) - 8*E(T) = 3.99988 hashes, 0 <= err < E(R)=4      (bound holds)
  Q(T_pl) = 4.0000005 -> ceil 5

R = 2^255 - 1 (largest power-of-two family allowed by R <= 2^256 - 2):
  m=1 S=2^255-1 E=2   S>=T yes   S>=T_pl YES
  m=2 S=2^254-1 E=4   S>=T yes   S>=T_pl no
  Q(T) = 64.00006 -> ceil 65;  Q(T_pl) = 8.00000095 -> ceil 9

work_unit_fingerprint(R = 2^254-1) = sha256("spc-lab/work-unit/v2" || 0x00 || R_be32)
  = 2fc4270b351c943cc49909e369f72df47e38ed48ee0f69bdf0bc1b2cb76a1859

Compact decode (Core SetCompact rules): 0x2100ffff -> 256-bit target, VALID
  0x220000ff -> 256-bit target, VALID;  0x22000100, 0x21010000 -> overflow;
  0x00123456, 0x01800000 -> zero target;  0x02123456 -> 13-bit target, VALID

Aggregate bound check, m <= u64::MAX, f <= 2.1e15 sats, 65 000 shares:
  m*f = 115 bits;  65000*m*f = 131 bits  > 128   (overflows u128)
```

Source facts checked on master `b0ab4ca`: `crates/spc-accounting/src/lib.rs`
uses `saturating_add`/`saturating_mul` in `Slice::push` (105), `Slice::agg`
(119-124), `select_window` (211) and `raw_payouts` (260-298);
`bins/spc-pool/src/front.rs:585-588` computes `window_target` as
`window_n.saturating_mul(credited_difficulty)`; `bins/spc-pool/src/sv1.rs:94-106`
is the only `f64` path (`network_difficulty`). `crates/spc-store/migrations/`
ends at `0004` on `master`, `ladder` and `sv2-tp`, so `0005` is free.
Pinned sv2-spec `d8bf5b8` `05-Mining-Protocol.md` section 5.3.21 (`SetTarget`):
"applicable for all jobs received on the channel in the future or already
received with an empty `min_ntime`. The message is not applicable for already
received jobs with `min_ntime=nTime`, as their maximum target remains stable."

## Blocking findings

### B1 (High) The recommended family cannot be issued at the node's pow limit; the step-down rule is missing

- Files: `docs/decisions/2026-11-05-lab-share-work-units.md` lines 92-96
  (`S(m) >= T_network` at issuance), 108-111 (`R = 2^254 - 1` recommended),
  150-153 (rejected update keeps the previous target), 187-191
  (`target_profile_unplannable`); `docs/decisions/2026-11-05-window-network-block-work.md`
  lines 208-218 (vectors assume the same `R`);
  `docs/REGTEST-LAB-IMPLEMENTATION.md` lines 216-217 (scenario uses that `R`),
  334-335, 349-359.
- What fails. Regtest's pow limit `0x207fffff` decodes to `2^255 - 2^232`,
  which is larger (easier) than `R = 2^254 - 1`. Under rule 3 every family
  member satisfies `S(m) <= R < T_pl`, so the moment a template carries pow-limit
  bits there is no issuable v2 target and the pool stops issuing jobs. Pow-limit
  bits are not exotic: they are genesis/height-1 bits for a retarget profile
  that does not use the overlay, the result of any retarget that clamps to the
  limit, and the min-difficulty exception result when it is left on (my
  earlier review A1/A2, confirmed by the synthesis). The hash-loss and
  min-target scenarios the lab exists to run (proposal P4, LP-4
  `retarget-boundary`) would therefore stall the pool, and if that pool is the
  only miner, the chain. "Fail closed" is correct for an unplannable profile;
  it is not acceptable for a profile the record itself recommends.
- A second gap sits in rule 6.5: it defines behavior only when *no* member is
  easier than `T`. When the configured `m` is unplannable but a smaller member
  exists (for example `m = 4`, `R = 2^254 - 1`, `T` moves to `0x201fffff`), the
  text both forbids issuing `S(m)` (rule 3) and forbids changing the target
  (rule 5.4 "keeps the previous target"). The implementer has no rule.
- Concrete change.
  1. Add a profile-validity rule: a v2 profile is valid for a network only if
     `R >= T_powlimit` for that network's effective `pow_limit_bits`
     (already a required introspection field, implementation doc line 750).
     Then `m = 1` is issuable for every target consensus can produce, because
     `T <= T_powlimit <= R = S(1)` always. Preflight fails with a named error
     (`reference_below_pow_limit`, naming both values) otherwise.
  2. Define the issuance rule when `S(m_cfg) < T`: select the largest family
     member `m' <= m_cfg` with `(R + 1) mod m' = 0` and `S(m') >= T`; this is
     a new job generation credited exactly at `d = m'` and is recorded as a
     `multiplier_stepdown` event. It is not a clamp, because credit and
     target agree. `target_profile_unplannable` is reserved for the case
     where no `m'` exists, which rule 1 makes unreachable for a valid
     profile. State whether the pool steps back up to `m_cfg` when `T`
     hardens again (recommended: yes, at the next job generation, recorded).
  3. Re-pick the recommended regtest reference so it passes rule 1:
     `R = 2^255 - 1` (`R + 1 = 2^255`, still a pure power-of-two family) is
     the natural choice. Update the vectors in both records and the scenario
     example: `E(R) = 2`, on `0x200fffff` `m = 1,2,4,8` give `E(S) = 2,4,8,16`
     (`m = 8` is the near-all-blocks choice), `Q(T) = 65` (floor 64),
     `Q(T_pl) = 9`. If the author prefers to keep `R = 2^254 - 1` as the
     *vector* for the `0x200fffff` overlay, keep it only as a test vector and
     state explicitly that preflight rejects it on any network whose pow
     limit is `0x207fffff`.
- Acceptance expectation. A unit test `profile_rejects_reference_below_pow_limit`
  (`R = 2^254 - 1`, `pow_limit_bits = 0x207fffff` -> named error) and
  `issuance_steps_down_to_largest_feasible_multiplier` (`m_cfg = 8`, `T` from
  `0x200fffff` to `0x207fffff` -> new generation at `m' = 1`, old generation
  untouched) are listed in the record's test table. The `Q(T) = 33/32` vector
  is replaced or explicitly scoped.

### B2 (High) The overflow claim is false for `sum(d*f)`, `m <= u64::MAX` is unsound for SPEC 4.4, and the shared saturating paths are not addressed

- Files: `docs/decisions/2026-11-05-lab-share-work-units.md` lines 77-79
  (`1 <= m <= u64::MAX`), 154-159 ("one slice cannot overflow `u128`"),
  330-345 (edit list omits SPEC 4.2 and 4.4);
  `docs/decisions/2026-11-05-window-network-block-work.md` lines 151-167;
  `docs/REGTEST-LAB-IMPLEMENTATION.md` lines 332-345.
- What fails. The "cannot overflow" statement is true for `sum(d)` (65 000 x
  2^64 is 80 bits) and false for the per-slice `diff_fee_sum = sum(d*f)` that
  SPEC 5.1 `slice_credit.diff_fee_sum` and WIRE-FORMATS 3.6 carry as `U128`:
  with `m` near `u64::MAX` and fees bounded only by supply, one slice reaches
  131 bits. SPEC 4.4's `r_net * sum(d)` and `(F_net/t) * sum(d*f)` products are
  "all arithmetic in u128" with no checked requirement, and the live code
  saturates silently at every step (`Slice::push`, `Slice::agg`, `select_window`,
  `raw_payouts`). A record that promises "fail closed, never saturating" for
  v2 while leaving `m` at 64 bits and leaving those paths out of the edit list
  cannot be implemented as stated; a saturated `raw_payouts` would mis-pay
  silently, which is the exact failure class the record says v2 removes.
- Concrete change.
  1. Bound `m` tightly at the profile boundary: a configurable
     `max_work_multiplier` with a default no larger than `2^32`, and state the
     resulting worst-case widths: `sum(d)` per slice `< 2^48`, `sum(d*f)` per slice
     `< 2^99` (fees `< 2^51`), `r_net * D_win` safe while `D_win < 2^77`.
     Bound `Q` accordingly (reject `Q > 2^64` or a stated limit) so `D_win`
     stays in range; `u128::MAX` is the type's limit, not a sound policy limit.
  2. Correct line 157 to say what is actually bounded, and name
     `diff_fee_sum` explicitly.
  3. Add SPEC 4.2 and 4.4 to "Edits required at merge": for v2 the slice
     accumulation, aggregates and payout products are checked arithmetic;
     overflow in `push` seals the slice before the share (or rejects the share
     with a named error, author's choice, but say which), overflow in the
     window or payout fails the distribution build with a named error.
  4. State that v1 keeps the saturating behavior (frozen), so the change is
     version-gated, and name the test that proves `raw_payouts` under v2 does
     not saturate (`proptest` against a big-integer oracle already in the
     test list; make it cover 4.4, not only the family and `Q`).
- Acceptance expectation. The record's test 7 ("synthetic sum above
  `u128::MAX`") becomes a set: slice `sum(d)`, slice `sum(d*f)`, `D_win`, and each
  4.4 product, each returning a named error under v2. A fixture with
  `m = max_work_multiplier + 1` fails config validation.

### B3 (Medium) Compact-target decode rule must be the consensus rule, not an exponent cutoff

- File: `docs/decisions/2026-11-05-lab-share-work-units.md` lines 135-139.
- What fails. "reject ... a negative/zero exponent, and any exponent that
  shifts beyond 256 bits" is ambiguous and, read literally, wrong. Exponent is
  an unsigned byte (never negative). Exponent 33 with mantissa `<= 0xffff` and
  exponent 34 with mantissa `<= 0xff` are valid 256-bit targets under Bitcoin
  Core's `arith_uint256::SetCompact` overflow test; a cutoff at exponent 32
  would reject bits a node can legitimately emit. Exponents 1-2 are valid
  right shifts. A zero exponent yields a zero target and is already covered by
  the zero-target rule.
- Concrete change. Replace the bullet with the consensus rule: decode
  `size = bits >> 24`, `word = bits & 0x007fffff`; shift right `8 (3 - size)`
  when `size <= 3`, else left `8 (size - 3)`; reject when `word != 0` and
  the sign bit `0x00800000` is set; reject when `word != 0` and
  (`size > 34` or (`word > 0xff` and `size > 33`) or (`word > 0xffff` and
  `size > 32`)); reject a zero result. Note that the node, not the pool,
  enforces `target <= pow_limit`.
- Acceptance expectation. Vectors in the record: `0x2100ffff` and `0x220000ff`
  accepted (256-bit targets), `0x21010000` and `0x22000100` rejected
  (overflow), `0x00123456` and `0x20800000` rejected (zero / sign),
  `0x02123456` accepted (13-bit target), `0x207fffff` accepted.

### B4 (Medium) The independent oracle has no stated source for `job_id -> (nbits, S, m)`

- Files: `docs/REGTEST-LAB-IMPLEMENTATION.md` lines 349-359 (job generation
  record), 562-573 (evidence states), 1126 (traceability claims the oracle is
  handled by 7.2); `docs/decisions/2026-11-05-lab-share-work-units.md`
  lines 171-174.
- What fails. v2 makes per-share credit a function of the job's target. The
  persisted surfaces (`slice.shares` blob, `slice_credit`, `distribution_snapshot`,
  `spc:blocks`) carry `job_id` and aggregate `d`, but no job record: jobs are
  front RAM. The proposal's correctness method (section 6, "independently
  implemented policy oracle over recorded accepted work and the exact frozen
  window/target") and the type-32 verifier (`share-accounting-ext-deltas.md`
  "recompute the share hash and check it meets the share target") cannot
  verify `d = m` per share without knowing which `S` each `job_id` was issued
  at. Under v1 a single configured difficulty made this implicit; v2 removes
  that shortcut and the contract does not replace it.
- Concrete change. Add a `job_issued` evidence record to 7.2 emitted by the
  front (or captured by the harness from the pool's job path) with
  `front_id, channel_id, job_id, generation, nbits, share_target, m,
  work_unit_version, work_unit_fingerprint, issued_at, activation
  (future/active)`, and state that `accounted` carries `job_id` and
  `generation`. Then list the oracle's inputs in one place: `job_issued`
  records, slice blobs, `slice_credit`, the snapshot's `window_rule`/
  `window_target`/`high_water`, and the on-chain coinbase. Say whether the
  type-32 path needs a corresponding change (a merge note for
  `share-accounting-ext-deltas.md`) or is explicitly out of scope for v2 in
  this packet; either answer is acceptable, silence is not.
- Acceptance expectation. LP-1/LP-3 acceptance includes an oracle test that
  recomputes `d` for every sampled share from `job_issued` + the share's hash
  and matches `slice_credit.diff_sum` per identity, and a negative test where
  a share credited at the wrong generation is caught.

## Non-blocking findings

### 5 (Should fix) Cite SV2 `SetTarget` semantics; add the future-job rule

- `docs/decisions/2026-11-05-lab-share-work-units.md` lines 165-195;
  `docs/REGTEST-LAB-IMPLEMENTATION.md` lines 423-428.
- Per-job credit is not an invented extension: pinned sv2-spec `d8bf5b8`
  section 5.3.21 already says `SetTarget` applies to future jobs and later
  jobs and "is not applicable for already received jobs with `min_ntime`
  set, as their maximum target remains stable". Cite it, so L1 inherits a
  standard requirement rather than a pool-local one. Add the one case the
  record leaves open: a future job's target is the channel target in effect
  when its `SetNewPrevHash` activates it. Rule 6.2's "old generation remains
  valid only while its job is valid" then needs no grace policy of its own.

### 6 (Should fix) LP-1 ownership overlaps L1 on `sv2_server.rs` and `front.rs`

- `docs/REGTEST-LAB-IMPLEMENTATION.md` lines 889-901; MERGE-QUEUE
  `lab-sv2-conformance` row owns "pool SV2/session/Merkle code".
- The report (lines 101-103) already says LP-1 "must rebase after L1". Make
  that a dependency in the packet (`Depends: L1 merged for sv2_server.rs and
  the target-lifecycle parts of front.rs`), or move the announced-target
  plumbing to an L1 merge note and keep LP-1's owned paths disjoint. As
  written LP-1 cannot be dispatched in parallel without a parallel-write
  conflict on two files.

### 7 (Should fix) Where `underfilled`, actual sum and overshoot are recorded

- `docs/decisions/2026-11-05-window-network-block-work.md` lines 87-93;
  `docs/REGTEST-LAB-IMPLEMENTATION.md` lines 339-343, 363-377.
- The decision says the implementation MUST record them; the persistence
  section adds no column or event field. Say whether they are derived (sum of
  slices at/below `high_water` versus `window_target` is sufficient to derive
  all three) and recorded only in lab evidence/metrics, or persisted. Derived
  is fine and avoids a schema change; state it.

### 8 (Should fix) Make the wire change definite and encode the enum

- `docs/decisions/2026-11-05-lab-share-work-units.md` lines 223-226 and
  342-343 ("if the implementation carries them on the wire");
  `docs/reviews/2026-11-05-lab-l3-contracts.md` lines 79-81; changelog
  fragment says "are amended".
- Rule 7.5 requires events to carry version/fingerprint so `payout` does not
  consult config; the edit list hedges with "if". Remove the hedge and give the
  fields: `spc:slices` + `U8 work_unit_version, B0_32 work_unit_fingerprint`;
  `spc:blocks` + `U8 work_unit_version, B0_32 work_unit_fingerprint,
  U8 window_rule`; define codes (`0 = share_work_legacy`, `1 =
  network_block_work`) once, used by both the `SMALLINT` column and the wire.
  Same-binary atomic deploy as the 2026-11-03 decision; old-format events fail
  decode.

### 9 (Should fix) Name mismatch: `initial_share_work_multiplier` vs `share_work_multiplier`

- `docs/REGTEST-LAB-IMPLEMENTATION.md` line 217 vs decision line 241. Pick one.

### 10 (Optional) Support the fixed-point/float claims or soften them

- `docs/decisions/2026-11-05-lab-share-work-units.md` lines 118-131, 256-259.
- "A float difficulty or a truncated ratio cannot be audited to the satoshi"
  overstates the case: a fixed-point floor with a normative scale is
  deterministic and auditable; its real defect is systematic under-crediting
  relative to `E(S)/E(R)` that needs a propagated error bound, which the
  record already says in section 4. Reword "cannot audit" to "biased and
  needs an error-bound contract". For floats the defensible objections are
  JS `Number` precision and cross-language rounding, not platform
  non-determinism of IEEE 754 basic operations.

### 11 (Optional) Carry the proposal's artifact privacy rules into the contract

- `docs/REGTEST-LAB-IMPLEMENTATION.md` section 7 and 8.1.
- The proposal (section 6) forbids recording `a_send`, shared secrets or
  transient tweaks and separates the shareable manifest from identity-bearing
  fixture material. The contract keeps the Ark-stream boundary (7.4) but does
  not restate those two rules for the harness's own artifacts, which will hold
  `sp_id`-bearing `accounted`/`slice_credit`-derived evidence. One paragraph.

### 12 (Optional) Unprivileged netns path and flake inputs

- `docs/REGTEST-LAB-IMPLEMENTATION.md` lines 704-741; `flake.nix` has no
  `iproute2`/`nftables`.
- Note that user namespaces (`unshare -Urn`) let an unprivileged runner create
  veth/nft/tc inside its own netns on kernels that allow them, that `sch_netem`
  must be loaded on the host, and that LP-4's merge note must add `iproute2`,
  `nftables` and `util-linux` to the flake. This turns rule 8's fallback into a
  documented capability probe.

### 13 (Optional) Small hardening

- Add a `CHECK ((work_unit_version = 1) = (work_unit_fingerprint IS NULL))`
  to make rule 7.2's corrupt-row definition a constraint.
- Add the fingerprint vector for the chosen `R` (for `2^254 - 1` it is
  `2fc4270b...1859` above) so implementers can test the preimage framing.

## Checks against the review brief

- Normative amendment completeness: SPEC 3.4, 4.1, 4.2 (`ShareRec.d`), 4.3,
  5.1, 5.3, 7.4 and WIRE 3.1/3.6 are listed; SPEC 4.2 slice accumulation and
  4.4 products are missing (B2); the 2026-11-02 record gets a superseded-in-part
  note (good).
- Family availability across pow limit / retarget / genesis: fails for the
  recommended `R` (B1).
- Target updates vs SV2 channel semantics: consistent with the pinned spec;
  cite it and add the future-job rule (5). No unnegotiated extension needed.
- Compact decode: must match consensus (B3).
- Fixed-point/float claims: partly rhetorical (10).
- Oracle inputs: incomplete (B4).
- Ledger identity privacy: the Ark boundary is preserved; artifact rules
  should be restated (11).
- Paused height waits: `blocked_on_height` with declared producers is
  implementable and falsifiable; no change.
- Canonical evidence / no absence rejection: `canonical_at` and
  `node_rejected` are correctly separated; `unknown` covers tip movement during
  observation; no change.
- Filesystem/PID recovery: `/proc/<pid>/stat` field 22 plus a run token is a
  sound Linux identity; the no-kill-by-PID rule is explicit; no change.
- netns privileges in the Nix environment: feasible via user namespaces on
  permissive kernels; the relay fallback is correctly declared weaker (12).
- L1/L2 interfaces: L2 fields are semantic only (good); LP-1/L1 file overlap
  needs sequencing (6).
- Doc checker: ran clean; `git diff --check` cannot see untracked files, and
  the report says so; my separate scans found no trailing whitespace or
  non-ASCII.

## Re-review procedure

Return a new manifest at `/tmp/opencode/lab-l3-review-manifest.json` with the
same format (the snapshot hash is `sha256` of the compact JSON of `base` +
`files`). I will verify hashes, re-run the arithmetic for the chosen `R`, and
check B1-B4 against the exact lines changed. Items 5-13 do not block approval
if the revision records which were taken and which were deferred.

## Verdict

REQUEST_CHANGES. Blockers: B1 (family must cover the node's pow limit and
define step-down), B2 (overflow bound and SPEC 4.2/4.4 checked arithmetic),
B3 (consensus compact decode), B4 (oracle `job_id -> target` source). No
commits, pushes, or edits to candidate files were made.

---

# Second pass (same day, same reviewer)

Snapshot under review:
`a53e9a29611df3d79aac01d12272b853a18998441f943ba764c7c014bbeeff2f`
(`/tmp/opencode/lab-l3-review-manifest.json`, base `b0ab4ca`, same seven
files, all uncommitted in the `lab-contracts` worktree).

## Result

**REQUEST_CHANGES**, narrowed to one blocker (R1) plus two nits. Every
first-pass finding B1-B4 and 5-13 is resolved as described in the implementer's
disposition table; I checked each against the revised text and recomputed every
vector. R1 is new only in the sense that the first pass did not reach it: the
revised stepdown rule introduces a search whose cost is not bounded for an
arbitrary `R`. The fix is one paragraph in the work-unit decision and a matching
line in the implementation contract. If R1 is fixed by either option below and
nothing else material changes, the third pass is an APPROVE confined to that
diff.

## Snapshot verification

```sh
cd /home/ethan/.local/share/opencode/worktree/ad8886/lab-contracts
git rev-parse --short HEAD     # b0ab4ca
git status --short             # the same seven ?? files
python3 - <<'EOF2'
import json,hashlib
m=json.load(open('/tmp/opencode/lab-l3-review-manifest.json'))
for f in m['files']:
    assert hashlib.sha256(open(f['path'],'rb').read()).hexdigest()==f['sha256'], f['path']
body=json.dumps({k:v for k,v in m.items() if k!='snapshot_sha256'},separators=(',',':'))
assert hashlib.sha256(body.encode()).hexdigest()==m['snapshot_sha256']
EOF2
python3 scripts/check-lab-docs.py   # OK: 6 files, ASCII clean, links resolve, vectors pass (3 EXPECTED)
git diff --check                    # exit 0
LC_ALL=C grep -nP '[^\x00-\x7F]' <seven files>; grep -nE ' +$' <seven files>   # no output
```

All seven per-file hashes and the manifest hash match. The checker now carries
an independent `fractions.Fraction`/integer vector suite (consensus
`SetCompact`, family, pow-limit validity, stepdown, `Q`, ceiling error,
fingerprints, widths, prose anchors); I read it and its logic is correct.

## Independent arithmetic (recomputed, not taken from the checker)

```text
R = 2^255 - 1, E(R) = 2;  T_pl = 0x207fffff -> 2^255 - 2^232;  R >= T_pl  OK
m=1  S=2^255-1 E=2   >=T yes  >=T_pl yes
m=2  S=2^254-1 E=4   >=T yes  >=T_pl no
m=4  S=2^253-1 E=8   >=T yes
m=8  S=2^252-1 E=16  >=T yes
m=16 S=2^251-1 E=32  >=T NO   (record: "rejected")                      OK
Q(0x200fffff) = 64.000061 -> ceil 65, floor 64; err = 1.99988 in [0,2)   OK
Q(0x207fffff) = 8.00000095 -> ceil 9, floor 8;  err = 1.99999 in [0,2)   OK
stepdown m_cfg=8: feasible at T = {1,2,4,8} -> 8; at T_pl = {1} -> 1     OK
65 m=1 shares = 65 units, 130 hashes; 33 m=2 shares = 66 >= 65, 132 hashes OK
fingerprint(2^255-1) = 7fe9ed1a4e2d4279b302cbb0bcf709758f03dc44db6dab82ac40c04eb0d77c6b OK
fingerprint(2^254-1) = 2fc4270b351c943cc49909e369f72df47e38ed48ee0f69bdf0bc1b2cb76a1859 OK
widths: 65000*2^32 < 2^48; 65000*2^32*2^51 < 2^99; 2^64+2^48 < 2^65; 2^51*2^65 = 2^116 OK
compact: 0x2100ffff, 0x220000ff, 0x02123456 (=0x1234), 0x207fffff accept;
         0x21010000, 0x22000100 overflow; 0x00123456 zero; 0x20800000 sign    OK
```

## First-pass findings: verification

| # | Status | Where verified |
|---|---|---|
| B1 | Resolved | Work-unit decision 3 (`R >= T_powlimit`, `reference_below_pow_limit`), 4 (`m_issue = max(feasible)`, stepdown/step-up events, unplannable unreachable for a valid profile), vectors; window decision vectors 65/9; implementation 4.1-4.2, scenario `R = 0x7fff...`. |
| B2 | Resolved | Decision 5.5-5.6: `m_cfg <= max_work_multiplier <= 2^32`, `Q <= max_window_target_units`, per-slice widths stated as limits, explicit statement that `DF_S`/`F_net * sum(d*f)` are not bounded and use checked wider mul/div; five named errors; v1 saturation frozen; SPEC 4.2/4.4 in the edit list; test 9. |
| B3 | Resolved | Decision 5.1 is the Core `SetCompact` rule verbatim in structure; test 5 vectors match mine. |
| B4 | Resolved | `job_issued` state (implementation 7.2 table and 7.2.1 input list), `accounted` carries `job_id`/`generation`, oracle `unknown` on missing record; v2 type-32 explicitly not claimed with a `share-accounting-ext-deltas.md` merge note. |
| 5 | Resolved | Decision 6.2 and implementation 4.3.6/5.3 cite sv2-spec `d8bf5b8` 5.3.21 and define future-job activation. |
| 6 | Resolved | LP-1 `Depends: ... L1 merged` for `sv2_server.rs` and target-lifecycle `front.rs`; announced-target plumbing is an L1 merge note. |
| 7 | Resolved | Window decision 2: `underfilled`/`actual_sum`/`overshoot` derived from `high_water` + `window_target`; no column. |
| 8 | Resolved | Definite fields and order on `spc:slices`/`spc:blocks`; enum codes 0/1; v1 zero sentinel on the wire, NULL in DB; old events fail decode. |
| 9 | Resolved | `share_work_multiplier` canonical; `m_cfg` vs `m_issue` distinguished throughout. |
| 10 | Resolved | Fixed point described as deterministic but biased; float objection reframed to binary64/JS precision. |
| 11 | Resolved | Implementation 7.7: shareable manifest vs identity-bearing material; no `a_send`/secrets/tweaks. |
| 12 | Resolved | Implementation 8.3: `unshare -Urn` probe, `sch_netem`, LP-4 flake merge note. |
| 13 | Resolved | DB `CHECK`s for version/fingerprint relation and enum; both fingerprint vectors recorded. |

## Remaining blocker

### R1 (Medium) `issuance_multiplier` has no bounded algorithm for an arbitrary `R`

- Files: `docs/decisions/2026-11-05-lab-share-work-units.md` lines 77-80
  (`R` is any value in `[0, 2^256 - 2]`), 81-82 (`m_cfg` only bounded above),
  144-148 (`feasible = { m in [1, m_cfg] : (R+1) mod m == 0 and S(m) >= T }`,
  `m_issue = max(feasible)`); `docs/REGTEST-LAB-IMPLEMENTATION.md` lines
  340, 352-357.
- What fails. The rule is a set comprehension, not an algorithm. Because
  `S(m) >= T` is monotone in `m`, it reduces to "largest divisor of `R + 1`
  that is `<= min(m_cfg, floor((R+1)/(T+1)))`". For a general 256-bit `R + 1`
  that is a divisor search: a naive descending scan is up to `2^32` 256-bit
  modulus operations per job generation, and a precomputed divisor list needs
  factoring `R + 1`, which is not feasible in general. The record is binding
  for every `R` it admits, and the job path issues a new generation on every
  template change, so an implementer either writes an unbounded hot-path loop
  or invents a restriction the record does not state. Two adjacent gaps in the
  same lines: `m_cfg` is not required to divide `R + 1` (a non-divisor `m_cfg`
  would emit `multiplier_stepdown` on every generation and never
  `multiplier_stepup`), and no complexity requirement is stated.
- Concrete change (either option; (a) is simpler and matches the only
  recommended profile):
  - (a) **Power-of-two family.** Require `R + 1 = 2^k` for v2 (`R = 2^k - 1`,
    `k <= 255`). Then the family is `m = 2^j`, `0 <= j <= k - 1`, divisibility is
    free, and `m_issue = 2^min(j_cfg, floor(log2((R+1)/(T+1))))` is O(1) with
    256-bit shifts and a leading-zero count. Require `m_cfg = 2^j_cfg`;
    validation rejects anything else with a named error
    (`multiplier_not_in_family`). Note that a non-power-of-two `R + 1` would be
    a new version.
  - (b) **Declared divisor list.** Keep arbitrary `R`, but require the profile
    to declare `allowed_multipliers`, a sorted list of at most a stated length
    (for example 64) with each entry `<= max_work_multiplier` and each
    verified at preflight to divide `R + 1`; `m_cfg` must be in the list;
    `m_issue` is the largest listed member with `S(m) >= T`, O(list length).
    Fold the list into the fingerprint preimage or state why it is excluded
    (it changes credit outcomes, so including it is the safer choice).
  - In both cases, state the requirement explicitly: issuance selection MUST be
    bounded by a constant or by the declared list length, never by a search
    over `[1, m_cfg]`. Update `issuance_multiplier` in implementation 4.2 and
    the checker's `issuance()` helper (it currently enumerates `range(1, m_cfg + 1)`;
    fine for a vector, misleading as a specification).
- Acceptance expectation. A config test rejects `m_cfg = 3` (option a) or a
  non-dividing `allowed_multipliers` entry (option b) with the named error; the
  stepdown test still yields `m_issue = 1` at `0x207fffff` and `8` at
  `0x200fffff`; the record states the selection complexity.

## Nits (non-blocking; fold into the same revision if convenient)

- N1. `max_window_target_units`: the changelog fragment says
  `Q <= max_window_target_units <= 2^64` (a hard cap) while both decisions say
  only "default `2^64`" (work-unit 5.5 and 8, window 5). The width analysis
  assumes the cap. Make it a hard cap like `max_work_multiplier` ("a v2
  configuration MUST reject a larger value") in both records.
- N2. Window decision line 168 names `window_target_version`, which is defined
  nowhere else; the discriminators are `work_unit_version` and `window_rule`.
  Delete the stray name.

## What I did not re-check

Sections of `REGTEST-LAB-IMPLEMENTATION.md` outside 4, 5.3, 7.2, 7.7, 8.3 and
10 were read for consistency with the first pass, not re-reviewed line by line;
the first-pass assessment of the evidence, command, snapshot, adoption and
statistics contracts stands. No Rust was built or tested; this is a docs-only
design packet.

## Verdict (second pass)

REQUEST_CHANGES on snapshot
`a53e9a29611df3d79aac01d12272b853a18998441f943ba764c7c014bbeeff2f`. One
blocker, R1: bound the issuance-multiplier selection (power-of-two family or a
declared, preflight-verified divisor list) and require `m_cfg` to be a family
member. Nits N1-N2 optional. B1-B4 and 5-13 are resolved. No candidate files,
code, other worktrees or commits were touched; the only file written is this
review.

---

# Third pass (same day, same reviewer)

Snapshot under review:
`3edcd4e744149667b184f44bdf3833ba465e210671eb90912fb1db9b9b11ba25`
(`/tmp/opencode/lab-l3-review-manifest.json`, base `b0ab4ca`, the same seven
uncommitted files in the `lab-contracts` worktree). Scope: the R1/N1/N2
revision, plus a regression check that nothing resolved in passes one and two
was reopened.

## Result

**APPROVE**, tied to snapshot
`3edcd4e744149667b184f44bdf3833ba465e210671eb90912fb1db9b9b11ba25`.

R1, N1 and N2 are resolved. B1-B4 and 5-13 remain resolved. No new defect was
found. The two decision records and the implementation contract are
implementable as written; the merge session may apply the changelog fragments
and the "Edits required at merge" lists. Any material change to these seven
files after this snapshot requires another review.

## Snapshot verification

```sh
cd /home/ethan/.local/share/opencode/worktree/ad8886/lab-contracts
git rev-parse --short HEAD     # b0ab4ca
git status --short             # the same seven ?? files, nothing else
python3 - <<'EOF2'
import json,hashlib
m=json.load(open('/tmp/opencode/lab-l3-review-manifest.json'))
for f in m['files']:
    assert hashlib.sha256(open(f['path'],'rb').read()).hexdigest()==f['sha256'], f['path']
body=json.dumps({k:v for k,v in m.items() if k!='snapshot_sha256'},separators=(',',':'))
assert hashlib.sha256(body.encode()).hexdigest()==m['snapshot_sha256']
EOF2
python3 scripts/check-lab-docs.py   # OK: 6 files, ASCII clean, links resolve, vectors pass (3 EXPECTED merge inputs)
git diff --check                    # exit 0
LC_ALL=C grep -nP '[^\x00-\x7F]' <seven files>; grep -nE ' +$' <seven files>   # no output
grep -n "window_target_version" docs/decisions/2026-11-05-*.md docs/REGTEST-LAB-IMPLEMENTATION.md   # no output (N2)
grep -n "feasible = {\|m in \[1, m_cfg\]\|range(1, m_cfg" docs/*.md docs/decisions/*.md scripts/check-lab-docs.py   # no output (R1 leftovers)
```

All seven per-file hashes and the manifest hash match.

## R1 verification (option (a), power-of-two family)

Text checked: work-unit decision section 3 lines 79-89 (`R = 2^k - 1`,
`1 <= k <= 255`, `reference_not_power_of_two`; `m_cfg = 2^j_cfg`,
`0 <= j_cfg <= k - 1`, `multiplier_not_in_family`; `max_work_multiplier <= 2^32`
hard cap), section 4 lines 144-175 (O(1) selection, "MUST NOT scan `[1, m_cfg]`
or search for divisors"), vectors lines 440-452, tests; implementation 4.1-4.2
lines 321-364 (`issuance_exponent`); changelog fragment; checker `issuance()`
now `min(j_cfg, k - ceil_log2(target + 1))`.

Independent check of the formula, not taken from the checker:

```text
ceil_log2(n) = (n - 1).bit_length():  n=1->0, 2->1, 3->2, 4->2, 5->3   OK
S(2^j) >= T  <=>  2^(k-j) >= T+1  <=>  j <= k - ceil_log2(T+1); max j is the formula   OK
k=255, j_cfg=3:
  T = 0x200fffff           -> j_issue 3, m 8
  T = 0x207fffff (pow lim) -> j_issue 0, m 1
  T = 2^252 - 1            -> j_issue 3, m 8, S == T (exact-power boundary)   OK
  T = 2^252                -> j_issue 2, m 4                                    OK
  j_cfg = 32 on 0x200fffff -> j_issue 3 (ceiling binds);  j_cfg = 0 at pow limit -> 0
brute force: formula == max{ j <= j_cfg : 2^(k-j) - 1 >= T } for 2000 random (T, j_cfg)   OK
j_issue >= 0 for every T <= T_powlimit <= R, so m = 1 is always issuable; S(m) >= 1 from j <= k-1   OK
```

The restriction to `R + 1 = 2^k` is recorded as a scoped decision (a different
family is a new version), the fingerprint remains a function of `R` alone, and
the recommended profile `R = 2^255 - 1` is unchanged, so no pass-two vector
moved.

## N1 / N2 verification

- N1: `max_window_target_units` is a hard cap `<= 2^64` in work-unit decision
  5.5 and 8, window decision 5 (line 165) and 6.1, implementation 4.1 line 330,
  and both changelog fragments. Consistent.
- N2: `window_target_version` no longer appears anywhere in the packet.

## Regression check on passes one and two

Re-read for drift: work-unit decision sections 5-8 and tests; window decision
sections 2-6, vectors and edits; implementation 4.1-4.5, 5.3, 7.2/7.2.1, 7.7,
8.3, LP-1 dependencies; both fragments; the implementer report's disposition
table (now covering B1-B4, 5-13, R1, N1, N2 with no deferral). Unchanged and
still correct: consensus compact decode, `R >= T_powlimit`, named overflow
errors and SPEC 4.2/4.4 in the edit lists, `job_issued`/oracle inputs and the
explicit v2 type-32 non-claim, `SetTarget` citation and future-job rule,
LP-1-after-L1 sequencing, derived underfill/overshoot, definite wire fields
with codes 0/1 and the v1 zero sentinel, DB `CHECK`s, artifact privacy rules,
netns capability probe and flake merge note. The scenario example
(`share_work_multiplier: 2`, `max_work_multiplier: 4294967296`,
`R = 0x7fff...ff`) is a valid power-of-two profile.

## Notes for the merge session (not conditions of approval)

- The two fragments are long for a `changelog:` bullet; that is a style choice
  the merge session may trim, provided the section list and record links stay.
- `docs/share-accounting-ext-deltas.md` needs the merge note the work-unit
  decision asks for (v2 type-32 mapping source not claimed).
- `docs/REGTEST-LAB-STATUS.md` and `docs/MERGE-QUEUE.md` rows for L3 should
  record this approval and the snapshot hash.

## Verdict (third pass)

APPROVE on snapshot
`3edcd4e744149667b184f44bdf3833ba465e210671eb90912fb1db9b9b11ba25`. No
remaining blockers. No candidate files, code, other worktrees or commits were
touched; the only file written across all three passes is this review.
