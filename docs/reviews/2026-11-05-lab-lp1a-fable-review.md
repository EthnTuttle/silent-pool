# 2026-11-05 - Fable review: LP-1a pure arithmetic precursor (`work_unit`)

Reviewer: Fable (subagent). Role: gate review only. No candidate edits, no
commits, no pushes, no subagents. This file is the only artifact written.

## Verdict

**APPROVE** - bounded to the pure module as snapshotted below. This approval
covers the arithmetic, the API shape, the fail-closed error set and the test
pinning of `crates/spc-accounting/src/work_unit.rs`. It does not approve, and
nothing in the candidate claims, a live v2 pipeline, persistence, wire fields,
or an overflow-safe end-to-end v2 accounting path. Those remain explicit
integration gates (section 7).

## Snapshot reviewed

Worktree `/home/ethan/.local/share/opencode/worktree/ad8886/lab-share-math`,
base `b0ab4ca34fda07c61a6335d05f598eaacc69678c` (HEAD, clean apart from the
listed files). Manifest `/tmp/opencode/lab-lp1a-review-manifest.json`,
recomputed with
`sha256(json.dumps({'base','files'}, sort_keys, separators=(',',':')))`:

```
45df0eb346787828e4b58a2fec229e1fce82ded72d1730ce011e82463696379c   (matches)
```

Per-file SHA-256, all recomputed from disk and matching the manifest:

| File | SHA-256 |
|---|---|
| `crates/spc-accounting/src/work_unit.rs` (992 lines, new) | `62fde9bd...e73a0` |
| `crates/spc-accounting/src/work_unit_tests.rs` (1096 lines, new) | `1a64a9d0...bc31d` |
| `crates/spc-accounting/proptest-regressions/work_unit_tests.txt` (new) | `652c3cd9...6eed2` |
| `crates/spc-accounting/src/lib.rs` (7 added lines, only tracked edit) | `cb4bad1f...b3545` |
| `docs/reviews/2026-11-05-lab-lp1a-implementation.md` (new) | `b80fa70e...8cfb` |

`git diff --stat b0ab4ca` shows exactly `lib.rs | 7 +++++++`. No change to
`Cargo.toml`, `Cargo.lock`, any crate manifest, or `tests.rs`. `#![deny(unsafe_code)]`
is set on the module; no `unsafe`, no float types, no new dependencies.

Inputs applied: `AGENTS.md` (worktree copy), the approved L3 decisions
`docs/decisions/2026-11-05-lab-share-work-units.md` and
`docs/decisions/2026-11-05-window-network-block-work.md` and contract
`docs/REGTEST-LAB-IMPLEMENTATION.md` section 4, all read from the
`lab-contracts` worktree (Fable third-pass APPROVE on snapshot `3edcd4e7...`),
and SPEC section 4.4 (floors per term) for the payout-product helper.

## 1. Supported-environment verification (nix)

The implementer report recorded plain `cargo` commands. I re-ran under the
flake's shell with `CARGO_BUILD_JOBS=2`:

```
cd <worktree> && CARGO_BUILD_JOBS=2 nix develop --command sh -c '
  rustc --version; cargo --version;
  cargo test -p spc-accounting;                                echo EXIT_TEST=$?
  cargo fmt --all -- --check;                                   echo EXIT_FMT=$?
  cargo clippy -p spc-accounting --all-targets -- -D warnings;  echo EXIT_CLIPPY=$?'
```

Log `/tmp/opencode/Fable-LP1a-nix-test.log`
(sha256 `eb75aecaaf17d6537b9e18a069a9c8674dd46abe48d5411a854f6b45e7299f98`):

- `rustc 1.93.0 (254b59607 2026-01-19)`, `cargo 1.93.0` - matches
  `rust-toolchain.toml` and the flake pin.
- `test result: ok. 59 passed; 0 failed` (24 new `work_unit` tests + 35
  legacy `tests.rs`, which is untouched), `EXIT_TEST=0`.
- `EXIT_FMT=0`, `EXIT_CLIPPY=0`, `nix develop exit=0`.
- The `Permission denied` cargo-registry cache warnings are environmental and
  identical to the implementer's logs; they do not affect the build.

The implementer's recorded logs (`/tmp/opencode/LP1a-*.log`) were inspected
and agree: 59/24/3/2 passes, fmt and clippy clean, and the fail-first log shows
the expected `E0583` for the then-missing module (not a behavioral red).

## 2. Independent cross-check (not trusting the in-tree oracle or 256 cases)

Because the module ships its own `U256`/`U512` limb arithmetic (shifts, add-1,
schoolbook `u128 x u128`, restoring 512/256 division), I built a separate
oracle in Python big integers (no floats) and a scratch crate that depends on
`spc-accounting` by path and exercises **public API only**:

- `/tmp/opencode/Fable-LP1a-xcheck/gen_vectors.py`
  (sha256 `0cd02e52...6e741`) -> `vectors.txt` (sha256 `5fe2b650...7b75d`),
  110 903 vectors.
- `/tmp/opencode/Fable-LP1a-xcheck/src/main.rs` (sha256 `5bbbe81d...1cd24`).
- Run under `nix develop` in release and debug (debug catches limb-index or
  shift overflow panics); logs `/tmp/opencode/Fable-LP1a-xcheck.log`,
  `/tmp/opencode/Fable-LP1a-xcheck-debug.log`. Both:
  `vectors=110903 mismatches=0 target_ord_bitlen_mismatches=0`, `EXIT=0`.

Coverage by kind (all exact value and error-variant matches):

| Kind | Count | What it pins |
|---|---|---|
| D decode | 9 680 | Core `SetCompact` + reject flags for every `size` 0..=255 x 30 structured words (0, 1, 0xff, 0x100, 0xffff, 0x10000, 0x7fffff, sign-bit variants) + 2 000 random `u32`. Exponents 33/34 small-mantissa accept, 0x21010000 / 0x22000100 overflow, word=0 -> zero for any size, sign only when word != 0. |
| Q window target | 48 938 | `ceil(N 2^k/(T+1))` for k in {1,2,3,8,31,32,33,63,64,65,127,128,129,191,192,193,254,255}, N in {1,2,7,8,9,2^32-1,2^32,2^63,2^64-1}, T over 2^p-1 / 2^p / 2^p+1 for 23 exponents plus pow-limit, overlay, random; cap at exactly Q, Q-1 and 2^64. Includes Q = 2^64 exactly accepted and Q = 2^64+1 -> `window_target_too_large`; N=0, T=0 and cap 2^64+1 named errors. |
| M mul/div | 14 404 | `floor(a b/den)` over boundary values (0, 1, 2^64 +- 1, 2^127 +- 1, 2^128-1, random) incl. den=0 -> error, quotient exactly 2^128 -> `payout_product_overflow`, `(2^128-1)^2/(2^128-1)` ok. |
| I issuance | 27 540 | brute-force largest feasible `j` by direct integer comparison vs the O(1) `k - bit_length(T)` rule, all k x j_cfg in {0..3,31,32,33,63,64,100} x T set; includes the `T+1 = 2^p` edge both sides, stepdown flag, `S >= T` acceptability of the result, `j_cfg >= 64`, above-cap and `j > k-1` rejections, defensive unplannable for small k. |
| R ratio | 6 930 | `multiplier_for_share_target` for S = 2^p-1 all p, non-family and zero targets; `Overflow` only when `k-p >= 64`. |
| E expected hashes | 385 | exact `2^(256-p)` only for `T+1 = 2^p`, all-ones -> 1, otherwise `None`. |
| F fingerprint | 255 | `sha256("spc-lab/work-unit/v2" || 0x00 || R_be32)` for every k; both free function and profile method; frozen `7fe9ed1a...d77c6b` and `2fc4270b...6a1859` asserted in Python too. |
| V from_reference | 2 304 | R in {2^k-1, 2^k, 2^k-2} for all k, 0, all-ones, overlay, pow limit; caps 2^32, 2^32+1, 1. |
| S share_target, L pow-limit validity | 467 | `2^(k-j)-1` and `j > k-1` rejection; `R >= T_powlimit` across k and seven `bits`. |
| Target ordering | 20 000 pairs | derived `Ord` == 256-bit integer order (incl. single-byte perturbations); `bit_length` == integer view. |

## 3. Math and API against the approved contract

| Contract item | Finding |
|---|---|
| `R = 2^k - 1`, `1 <= k <= 255`; `reference_not_power_of_two`; k=0/256 by bounds (decision s.3, s.5.4) | Correct. `from_reference` computes `R+1` with checked add; all-ones maps to k=256 -> `ReferenceExponentOutOfRange(256)`; R=0 -> k=0 rejected. |
| `m = 2^j`, `0 <= j <= k-1`, `m_cfg <= max_work_multiplier <= 2^32` hard cap (s.3, s.5.5) | Correct in `validate_multiplier`, `from_exponent`/`from_reference` (cap > 2^32 rejected with its own name), and re-checked inside `issuance`. |
| Consensus `SetCompact` (s.5.1) | Byte-exact vs Core semantics, incl. the `word != 0` gate, 33/34 small-mantissa accept, no exponent-32 cutoff, zero reject. |
| 256-bit integer comparison, hash meets iff `hash <= T` (s.5.2) | `Target([u8;32])` big-endian with derived `Ord`; verified numeric. `share_target_is_acceptable` uses `<`. |
| O(1) selector `j_issue = min(j_cfg, k - ceil_log2(T+1))` incl. `T+1 = 2^p` edge, stepdown/step-up data, unplannable guard (s.4) | Correct; `ceil_log2(T+1) = T.bit_length()` is exact. `T = 2^252-1 -> m=8, S=T`; `T = 2^252 -> m=4`. No scan. The step-up *event* is deliberately left to the front's generation state (data `stepped_down` is provided) - acceptable for a pure module, see gate G1. |
| `Q(T) = ceil(N(R+1)/(T+1))`, 512-bit intermediate, one ceiling, cap `<= 2^64`, N>0, T>0, never saturate (window s.1, s.6) | Correct. 65 / 9 frozen; floor 64 / 8 impossible by construction; Q above cap or `u128` -> `window_target_too_large` carrying `cap` and the computed value when representable. |
| Fingerprint preimage (s.7.1) | Exact bytes; both frozen vectors. |
| Enums and version/rule rule (s.7.2, window s.3) | Codes 1/2 and 0/1; `from_u8` fails closed; cross combinations rejected. |
| Window metadata derived, crossing slice included, underfill (window s.2) | `accumulate_slice_work` walks newest-first, includes the crossing slice, checked sum, derives `overshoot`/`underfill`/`underfilled`; nothing persisted. Finding-slice exclusion is the caller's precondition (documented) - gate G2. |
| Checked helpers with named errors (s.5.6) | `slice_work_overflow`, `slice_fee_work_overflow`, `window_work_overflow`, `payout_product_overflow` present; `checked_mul_div_u128` is an exact floor (SPEC 4.4 floors per term) via 256-bit product + 512/256 division. |
| `expected_hashes_exact` honesty | Returns `Some` only when `T+1 = 2^p` and `2^(256-p) <= 2^127`; otherwise `None`; no float. |
| Public entry points validate their own inputs | Yes: `issuance`, `share_target`, `window_target_network_work`, `accumulate_slice_work`, `from_reference`, `validate_multiplier` each enforce cap/range/zero themselves. `Target::from_be_bytes` is intentionally a raw 256-bit constructor. |
| v1 unchanged | `tests.rs` untouched, 35 legacy tests pass; no edits outside the new module and the re-export. |

Private arithmetic reviewed line by line (`work_unit.rs` 711-988): `low_ones`
limb fill, `bit_length`, `add1_checked` carry, `checked_shl` top-bit guard,
`mul_u128` carry propagation (the final carry out of limb 0 is provably zero
because the partial sum never exceeds the 256-bit product), `from_u64_shifted`
(k <= 255 so the carry limb index is in range), restoring division (remainder
`< den < 2^256` so `shl1_checked` cannot fire). No boundary defect found; the
cross-check above exercises every reachable path at both ends.

## 4. Test quality

- Frozen vectors pin exact inputs and outputs (hex bytes, exact `u128`/`u64`,
  exact error variants with payloads, incl. `ReferenceBelowPowLimit` naming
  both hex values). They are not "is_err()"-only, except for `k` bounds where
  the variant is checked elsewhere.
- The in-tree oracle is a base-10^9 bignum with its own MSB-first bit
  division; it is structurally independent of the module's 64-bit limbs and
  never calls module helpers. My Python oracle is a third independent
  implementation and agrees.
- Proptest regression seeds (`k = 65, j = 64`; `k = 1, n = 1, t = ...02`) are
  committed, so the two shrunken bring-up failures stay pinned.
- Known-bug pins: floor-vs-ceil (64 / 8 asserted unequal), the ceiling error
  bound `0 <= Q E(R) - N E_block < E(R)` in integers, `m = 16` rejected on
  `0x200fffff`, `S + 1` in the ratio, cap-above-2^64 rejection.
- The source-scan "no float" test is supplemental; the correctness proof is
  the read-through plus the integer oracles.

## 5. Observations (non-blocking, no change required for this gate)

O1. `work_unit.rs:452` - `issuance(j_cfg >= 64, ..)` returns the generic
`Overflow` rather than `multiplier_above_hard_cap`/`multiplier_not_in_family`;
`:404` `share_target(j >= 64)` reports `MultiplierNotInFamily(0)`. Both fail
closed; naming only. Expectation if polished: `MultiplierAboveHardCap`.

O2. `work_unit.rs:566` - `window_target_network_work` with `T = 2^256-1`
returns generic `Overflow` from `add1_checked`. Not reachable from compact
decode (max decodable target is `< 2^256-1`); naming only.

O3. Decision vector table (`lab-share-work-units.md:477`) lists `0x20800000`
as "rejected sign". Core and the decision's own s.5.1 rule (sign checked only
when `word != 0`) make it a zero target; the module returns `CompactZero` and
the test documents this, adding `0x20800001` for the sign path. This is a one
word nit in the decision table for the merge session; the code follows the
normative rule.

O4. `prop_window_target_matches_oracle` draws uniform 32-byte `T`, so most
cases have `T ~ 2^255` and small `Q`; large-`Q`/cap regions are covered by the
frozen vectors and by my cross-check (48 938 cases), not by that proptest.

O5. `checked_fee_work(u128, u128)`: with `d, f: u64` the product cannot
overflow `u128`; the wider signature is the right building block for window
aggregates and the docs say so. The decision's `max_slice_shares <= 65 000`
bound is configuration-level and not represented here (gate G5).

## 6. Scope confirmation

The candidate is what it says: a pure module plus a 7-line re-export. No pool,
store, wire, schema, config or SPEC edits; no v1 behavior change; no new
dependencies; no `unsafe`. The implementation report's non-claims (no full v2
overflow-safety claim, no type-32 v2 audit, no mainnet claim) are accurate.

## 7. Integration gates that remain explicit (not satisfied by this approval)

G1. Generation state machine: `multiplier_stepdown`/`multiplier_stepup`
emission, `job_issued` evidence, per-job credit, stale rejection (decision s.4
items 3-4, s.6; tests 3, 8, 10).
G2. Finding-slice exclusion before `accumulate_slice_work`; `Q` computed at
build and persisted; settlement reads the frozen `window_rule`/`window_target`
(window s.3-4; tests 3, 5, 7, 8).
G3. Adoption of the checked helpers in v2 `Slice::push`, `agg`,
`select_window`, `raw_payouts` with v1 saturating fixtures kept separate
(decision s.5.6; test 9). Only then may "the v2 path fails closed on overflow"
be claimed.
G4. Persistence and wire: version/fingerprint/rule columns and CHECKs, zero
fingerprint sentinel, `work_unit_restore_mismatch`, unknown-code decode
failure (s.7; test 11; window test 9).
G5. Configuration surface and caps: `max_work_multiplier`,
`max_window_target_units`, `max_slice_shares <= 65 000`, no
`share_difficulty` aliasing for v2 (s.5.5, s.8).
G6. SPEC/WIRE-FORMATS edits and the changelog fragments listed under "Edits
required at merge" in both decisions, plus the O3 table nit.
G7. Window-meaning contract test (window test 12) and the v1-vs-v2 fixture
split (window test 6, contract 4.5).

## 8. Result

APPROVE for snapshot `45df0eb3...6379c` (base `b0ab4ca`) as a bounded pure
precursor. Verified under `nix develop` (rustc 1.93.0): 59 tests pass, fmt and
clippy exit 0; 110 903 independent oracle vectors and 20 000 ordering checks
match with zero mismatches in debug and release. No blocking findings.
