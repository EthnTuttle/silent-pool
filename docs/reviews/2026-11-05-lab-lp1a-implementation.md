# LP-1a pure arithmetic precursor: implementation report

Date: 2026-11-05
Status: implemented, uncommitted work in
`/home/ethan/.local/share/opencode/worktree/ad8886/lab-share-math`, base
`b0ab4ca`. User-authorized TFF work with Fable gate.

This is the LP-1a precursor only: the pure v2 work-unit arithmetic module.
The full LP-1 integration (job generations, persistence, wire fields, front
adoption) waits for the L1 SV2 merge and is listed verbatim under "Remaining
LP-1 integration" below. Parent Fable must review this module before it is
adopted anywhere.

## Inputs used

- `docs/decisions/2026-11-05-lab-share-work-units.md` (approved decision, not
  the old review math).
- `docs/decisions/2026-11-05-window-network-block-work.md` (approved
  decision).
- `docs/REGTEST-LAB-IMPLEMENTATION.md` section 4 (implementation contract)
  and the LP-1 packet definition.
- Fable APPROVE on snapshot
  `3edcd4e744149667b184f44bdf3833ba465e210671eb90912fb1db9b9b11ba25`
  (`docs/reviews/2026-11-05-lab-l3-fable-review.md` in the main worktree,
  third pass).
- Repo `AGENTS.md` working conventions (fail closed, plain ASCII, no
  commits on behalf of a human).

All frozen vectors were recomputed independently (Python integers and
`fractions.Fraction`, no floats) before implementation, and the module's own
property tests re-verify them against an in-test bignum oracle.

## Scope delivered

- `crates/spc-accounting/src/work_unit.rs` - new pure module (no I/O, no
  clock, no pool state, no `unsafe`).
- `crates/spc-accounting/src/work_unit_tests.rs` - dedicated test file with
  frozen approved vectors, an independent base-10^9 bignum oracle, and
  proptests.
- `crates/spc-accounting/src/lib.rs` - minimal re-export (7 added lines: the
  module declaration plus four re-exported names). This is the only change
  to a tracked file.
- `crates/spc-accounting/proptest-regressions/work_unit_tests.txt` - the
  standard proptest seed file (recommended for source control by proptest);
  it records the two shrunken cases that drove the fail-first fixes.

Nothing else was touched. No v1 routing/window behavior edits; no
pool/store/wire/schema files; no `Cargo.toml` or `Cargo.lock` change (the
module uses only existing workspace dependencies: `bitcoin` hashes for the
fingerprint, `thiserror`, and the already-present dev-dependencies `hex` and
`proptest`); no commits, pushes, merges, resets or stashes; no subagents.

## API

### Constants

| Name | Value | Source |
|---|---|---|
| `DEFAULT_REFERENCE_EXPONENT` | `255` (`R = 2^255 - 1`) | work-unit decision s.3 |
| `HARD_CAP_MAX_WORK_MULTIPLIER` | `2^32` | work-unit decision s.5.5 |
| `HARD_CAP_MAX_WINDOW_TARGET_UNITS` | `2^64` | window decision s.5 |
| `WORK_UNIT_FINGERPRINT_PREFIX` | `b"spc-lab/work-unit/v2"` | work-unit decision s.7.1 |

### Types

- `Target([u8; 32])` - 256-bit consensus target, big-endian; derived `Ord`
  is numeric order (hash meets target iff `hash <= target`). Copy,
  `from_be_bytes`/`to_be_bytes`/`is_zero`/`bit_length`.
- `WorkUnitProfile` - validated `R = 2^k - 1`, `1 <= k <= 255`:
  - `from_exponent(k, max_work_multiplier)`,
    `from_reference(reference_be32, max_work_multiplier)`,
    `lab_default()` (`k = 255`, cap `2^32`);
  - accessors `exponent`, `reference_target`, `reference_bytes`,
    `max_work_multiplier`;
  - `fingerprint()` - `sha256("spc-lab/work-unit/v2" || 0x00 || R_be32)`;
  - `validate_pow_limit(pow_limit_bits)` - `R >= T_powlimit`, fails with
    `reference_below_pow_limit` naming both values;
  - `validate_multiplier(m_cfg) -> j_cfg` - power of two in
    `[1, min(max_work_multiplier, 2^(k-1))]`, else
    `multiplier_not_in_family` / `multiplier_above_hard_cap`;
  - `share_target(j) -> S = 2^(k-j) - 1` (`S >= 1` guaranteed);
  - `multiplier_for_share_target(S) -> m` - the exact `E(S)/E(R) = 2^j`
    ratio; refuses non-family targets instead of rounding;
  - `issuance(j_cfg, network_target) -> Issuance` - O(1) selection
    `j_issue = min(j_cfg, k - ceil_log2(T + 1))` with
    `ceil_log2(n) = (n - 1).bit_length()`; no scan, no divisor search.
- `Issuance` - `j_cfg`, `j_issue`, `m_issue`, `share_target`,
  `network_target`, `stepped_down` (the `multiplier_stepdown` event data; a
  step-up is the next generation with `stepped_down == false`; the event
  itself belongs to the front's generation state machine).
- `WorkUnitVersion` - `V1` (code 1) / `V2` (code 2), `from_u8` fails closed.
- `WindowRule` - `ShareWorkLegacy` (code 0) / `NetworkBlockWork` (code 1),
  `from_u8` fails closed; `check_version_rule_compatibility` rejects the
  cross combinations.
- `WindowSelection` - derived window metadata: `slice_ids`, `actual_sum`,
  `overshoot`, `underfill`, `underfilled`. Nothing here is a persisted
  column (window decision s.2).

### Free functions

| Function | Behavior |
|---|---|
| `decode_compact_target(bits)` | consensus `SetCompact` (decision s.5.1): sign/overflow/zero rules with the `word != 0` gate; exponents 33/34 valid with small mantissas; no exponent-32 cutoff |
| `share_target_is_acceptable(S, T)` | `S >= 1` and `S >= T`; harder targets rejected, never clamped |
| `window_target_network_work(profile, T, N, cap)` | `Q = ceil(N (R + 1)/(T + 1))` via a 512-bit intermediate and one ceiling; `N = 0`, `T = 0`, a cap above `2^64`, and `Q` above the cap or `u128` all fail closed |
| `accumulate_slice_work(slices_desc, Q)` | checked whole-slice walk, newest-first, crossing slice included; caller has already excluded the finding slice; derives overshoot/underfill/underfilled |
| `work_unit_fingerprint(R_be32)` | the stable preimage hash |
| `expected_hashes_exact(T)` | `2^256/(T + 1)` as an integer only when `T + 1` is a power of two; `None` otherwise, never a float |
| `checked_add_slice_work`, `checked_fee_work`, `checked_add_fee_work`, `checked_add_window_work` | checked `u128` accumulators with the named errors |
| `checked_mul_div_u128(a, b, den)` | exact 256-bit product and 512-bit restoring division for the SPEC 4.4 products; `payout_product_overflow` when the quotient does not fit `u128` |

### Named errors (`WorkUnitError`)

Every message carries its snake_case decision name:
`overflow`, `compact_sign`, `compact_overflow`, `compact_zero`,
`reference_not_power_of_two`, `reference_exponent_out_of_range`,
`reference_below_pow_limit` (naming `R` and `T_powlimit` in hex),
`multiplier_not_in_family`, `multiplier_above_hard_cap`,
`max_work_multiplier_above_hard_cap`, `max_window_target_units_above_hard_cap`,
`network_target_zero`, `share_target_zero`, `share_target_below_network`,
`target_profile_unplannable` (defensive guard only),
`window_n_zero`, `window_target_zero`,
`window_target_too_large` (with the computed value when it fits `u128`),
`window_work_overflow`, `slice_work_overflow`, `slice_fee_work_overflow`,
`payout_product_overflow`, `unknown_work_unit_version`, `unknown_window_rule`,
`version_rule_mismatch`.

### Arithmetic contract

- No `f64`/`f32` anywhere in code; a source-scan test enforces this (comment
  text is stripped first so documentation cannot trip it).
- No saturation of results, no wrapping, no implicit lossy casts. Wide
  intermediates are private big-endian `[u64; 4]`/`[u64; 8]` limb types with
  checked shifts, checked additions and a checked restoring division; every
  invalid case is an error.
- The module docs state the decision 5.5 caveat explicitly: `d * f` fitting
  `u128` does not bound the window-level fee aggregates, and a small `m`
  alone guarantees nothing about the fee products.

## Tests (fail-first, then implementation)

The test file was written first; the first run failed to compile with
`E0583: file not found for module work_unit` (log:
`/tmp/opencode/LP1a-failfirst.log`). Implementation followed, then the
failures found during bring-up were fixed (an oracle floor-vs-ceiling bug,
a slice-index typo, a missing `S + 1` in the ratio function, and the exact
ratio correctly failing closed above `u64`). The proptest regression seeds
record the two shrink cases.

Commands and results (worktree root, `CARGO_BUILD_JOBS=2`, toolchain
1.93.0 matching the flake):

```sh
cargo test -p spc-accounting work_unit         # 24 passed
cargo test -p spc-accounting window_network    # 3 passed
cargo test -p spc-accounting accounting_overflow  # 2 passed
cargo test -p spc-accounting                   # 59 passed (24 new + 35 legacy)
cargo fmt --all -- --check                     # exit 0
cargo clippy -p spc-accounting --all-targets -- -D warnings  # exit 0
```

Logs: `/tmp/opencode/LP1a-failfirst.log`,
`/tmp/opencode/LP1a-work-unit-tests.log`,
`/tmp/opencode/LP1a-window-network-tests.log`,
`/tmp/opencode/LP1a-accounting-overflow-tests.log`,
`/tmp/opencode/LP1a-accounting-full.log`, `/tmp/opencode/LP1a-fmt.log`,
`/tmp/opencode/LP1a-clippy.log`. (The cargo registry emits unrelated
`Permission denied` cache-cleanup warnings in this environment; they do not
affect the build.)

Coverage highlights, keyed to the decision test tables:

1. **Frozen vectors**: both fingerprints
   (`7fe9ed1a...d77c6b`, `2fc4270b...1859`); compact decode
   `0x2100ffff`/`0x220000ff`/`0x02123456`/`0x207fffff` accepted,
   `0x21010000`/`0x22000100` overflow, `0x00123456`/`0x01800000` zero,
   `0x20800001` sign, `0x20800000` rejected (word == 0, so consensus fails
   it on the zero rule); family `m = 1, 2, 4, 8, 16` for `k = 255` with the
   exact equation `m (S + 1) = R + 1` and `E(S) = 2, 4, 8, 16, 32`;
   `Q = 65`/`9` with floor `64`/`8` rejected and the exact ceiling-error
   bound `0 <= Q E(R) - N E_block < E(R)` verified in integer arithmetic.
2. **Profile validity**: `R = 2^254 - 1` vs `0x207fffff` ->
   `reference_below_pow_limit` naming both values; `R = 2^255 - 1` passes;
   `R = 2^128 + 1`-style and `2^255` non-powers rejected;
   `R = 0` (k = 0), the all-ones sentinel (k = 256) and `k = 256` rejected
   by the bounds.
3. **Stepdown/step-up and boundary**: `m_cfg = 8` at `0x200fffff` -> 8, at
   `0x207fffff` -> 1 with `stepped_down`, back to 8 (old generation
   untouched); `j_cfg = 32` on `0x200fffff` -> 3 (ceiling binds);
   `T = 2^252 - 1` -> `j_issue = 3`, `S == T` exactly;
   `T = 2^252` -> `j_issue = 2`, `m = 4`.
4. **Rejections**: `m_cfg = 3`, `m = 0`, `m_cfg = 2^33` above the cap,
   `max_work_multiplier = 2^32 + 1`, `j > k - 1`, `N = 0`, zero network
   target, `Q` above cap / above `u128::MAX` (`T = 1`, `N = 2^63`), the
   defensive `target_profile_unplannable`, and the version/rule cross
   combinations.
5. **Window selection**: the 10/10/10 `Q = 25` vector (all three slices,
   overshoot 5), exact-threshold inclusion, underfill, empty tail, and
   checked-overflow failure.
6. **Oracle proptests** (256 cases each): decode classification/value vs
   the bignum `SetCompact` oracle over all `u32`; family, issuance vs a
   brute-force largest-feasible-exponent oracle by direct bignum
   comparison; `Q` vs bit long division plus the ceiling characterization
   `q den >= num > (q - 1) den`; `mul_div` vs the bignum quotient;
   exact-ratio equations for all `(k, j)`.
7. **Legacy unchanged**: all 35 pre-existing `spc-accounting` tests pass
   unmodified (`tests.rs` was not touched).

## Non-claims

- This module does NOT make the full v2 accounting path overflow-safe. The
  checked helpers are building blocks; the v2 claim can only be made after
  LP-1 adopts them in every aggregation and payout path (SPEC 4.2/4.4) and
  the persistence/wire layers carry the version, fingerprint and rule.
- No type-32 v2 audit claim; that scope note from the decision stands.
- No mainnet or production claim; the profile is a lab CPU-scale design.

## Remaining LP-1 integration (after L1 SV2 merge, per the contract)

Owned paths not touched by this precursor:

1. `bins/spc-pool/src/front.rs` - job generations (`job_issued` evidence,
   activation state, generation ids), per-job credit, stepdown/step-up
   event emission, window build with `Q(T)` and distribution freezing.
2. `bins/spc-pool/src/sv1.rs` - v1 target helpers stay as-is; no v2
   conversion (already untouched here).
3. `crates/spc-store/src/models.rs` + `migrations/0005_lab_work_units.sql` -
   `work_unit_version`, `work_unit_fingerprint`, `window_rule` columns and
   the CHECK constraints; restore rejection (`work_unit_restore_mismatch`).
4. `crates/spc-store/src/wire.rs` - definite fields on `spc:slices`
   (`U8 version`, `B0_32 fingerprint`) and `spc:blocks` (`U8 version`,
   `B0_32 fingerprint`, `U8 window_rule`); v1 zero sentinel; unknown codes
   fail decode.
5. `crates/spc-accounting/src/tests.rs` + aggregation/payout paths - v2
   checked `Slice::push`/`agg`, window accumulation and SPEC 4.4 products
   via `checked_mul_div_u128`, with the named errors and v1 saturating
   fixtures kept separate.
6. `bins/spc-pool/src/config.rs`, `bins/spc-pool/main.rs`,
   `config/spc-pool.example.toml`, `Cargo.toml`, `Cargo.lock`,
   `docs/SPEC.md`, `docs/STATUS.md` - shared-file merge notes per the
   contract.

The remaining LP-1 test commands (`share_target_v2`, `job_generation`,
`work_unit_restore`, `e2e_sv2`, `cargo test --workspace`) belong to that
packet; the three pure filters (`work_unit`, `window_network`,
`accounting_overflow`) already run green against the precursor.

## Fable gate

Parent Fable should review this module before any adoption: the API names,
the O(1) selector and its exact-power boundary, the compact-decode rule,
the `Q` arithmetic and cap behavior, the checked-helper error set, and the
explicit non-claims above.
