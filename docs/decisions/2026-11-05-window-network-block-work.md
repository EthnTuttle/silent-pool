# 2026-11-05 - Window meaning: network-block work, not N share-difficulties

Status: revised candidate for Fable re-review. This record is not normative
until Fable approves it and the merge session applies the companion changelog
fragment to `docs/SPEC.md`.

**Trigger.** SPEC section 4.3 currently selects the reward window by accumulating
whole slices until `sum(difficulty) >= N * d`, with `N = 8` and `d` the credited
share difficulty. Changelog (d) changed the target from `N * network_difficulty`
to `N * d` so a low-difficulty regtest chain could fill a window. That made the
binding rule an **N-share-work threshold at the credited share difficulty**,
enlarged by whole-slice granularity. It is not an N-block work threshold.

The source material for PPLNS-JD / SLICE says the window is `8 * network
difficulty` in accumulated share difficulty (TIDES uses the same kind of
`8 * current_block_difficulty` share log). See
`docs/research/wiki/concepts/pplns-jd.md` and
`docs/research/wiki/concepts/sv2-share-accounting-ext.md`; those files are
non-normative evidence for the intent, not requirements by themselves. The
important consequence is not "eight blocks in wall time": it is eight blocks
of *network work*. At 1 % of network hashrate the window spans much longer than
eight blocks; at 100 % it is roughly eight blocks. The current `N * d` rule
does not have that property and becomes a very short window whenever the share
target is easier than the network target.

This is a payout-policy decision, not a unit conversion. The unit decision is
`docs/decisions/2026-11-05-lab-share-work-units.md`; the two records deliberately
use separate version fields so one cannot silently imply the other.

## Decision

### 1. The intended meaning is network-block work

For a network target `T` decoded from the template's `nbits`, define the exact
expected work of one block:

```
E_block(T) = 2^256 / (T + 1)              # rational, not a float
```

The window threshold for `N` blocks is `N * E_block(T)`.

In the v2 reference work units of
`docs/decisions/2026-11-05-lab-share-work-units.md`, one unit is `E(R)` and a
share at the exact target `S(m)` carries `d = m`. Therefore the window
threshold in accumulated `d` is

```
Q(T) = ceil( N * (R + 1) / (T + 1) )
```

where:

- `R` is the v2 reference target and `R + 1` is exact;
- `T` is the network target of the job whose share found the block (the same
  `nbits` used to validate the block), not a later template;
- the division is exact rational arithmetic followed by one ceiling to an
  integer `u128`;
- `Q` is persisted with the distribution and settlement never recomputes it
  from current network state.

`Q` is an integer count of reference units. It is not a converted network
difficulty, and it is not a count of shares: the share target changes how many
shares carry those units.

### 2. Ceiling, error bound and underfill

The ceiling is deliberate. With `floor`, the window could contain less than the
declared `N` blocks of work. With `ceil`:

```
0 <= Q(T) * E(R) - N * E_block(T) < E(R)
```

so the threshold rounding error is strictly less than one reference unit. It
is not zero, and this decision does not claim it is zero. Whole-slice
granularity adds a second, usually larger, overshoot:

```
actual_window_work = sum(difficulty of taken slices) * E(R)
actual_window_work >= Q(T) * E(R) >= N * E_block(T)
actual_window_work - N * E_block(T) <  E(R) + (work in the crossing slice)
```

When the stored tail contains at least `Q(T)` units, the window is therefore
never under target; it may be over target by less than one unit plus one whole
slice. If the available tail ends before `Q(T)`, the distribution is
`underfilled`: it pays from the available slices with the normal score
normalisation, but it is excluded from window-length/statistical claims and
must not be described as N-block coverage.

`underfilled`, `actual_sum` and `overshoot` are **derived**, not persisted
columns. Given the persisted `high_water` and `window_target`, a reader sums the
slice difficulties at or below the high-water vector:

```
actual_sum  = sum(slice.difficulty for slices at/below high_water)
overshoot   = max(0, actual_sum - window_target)
underfill   = max(0, window_target - actual_sum)
underfilled = underfill > 0
```

The lab evidence/metrics record these derived values. There is no new
`underfilled` column and no second source of truth; if the slices needed for
the derivation are missing, the result is `unknown`/`inconclusive`.

### 3. Scope: v2 uses this rule; v1 is a named legacy rule

1. **v2 (the reference-target work profile) uses `Q(T)` above.** The finding
   slice is still excluded from reward, the walk is still newest-first, whole
   slices are still taken, and the slice that crosses the threshold is still
   included.
2. **v1 remains the frozen legacy rule `Q_v1 = N * d` with the existing
   saturating arithmetic.** It is named `share_work_legacy`, not
   "network-block work". It is preserved so existing stored windows and
   distributions are never reinterpreted. A new lab run uses v2. A future
   product migration may retire v1, but that is a separate decision.
3. **The persisted `window_rule` selects the meaning.** The enum is used by
   both the database `SMALLINT` and the wire `U8`:

   | Code | Name | Meaning |
   |---|---|---|
   | 0 | `share_work_legacy` | stored `window_target` is `N * d` (v1) |
   | 1 | `network_block_work` | stored `window_target` is `Q(T)` in v2 units |

   Settlement uses the row's rule and target; it does not infer the rule from
   the current config.
4. **Invalid combinations fail closed.** `work_unit_version = 2` with
   `window_rule = share_work_legacy`, or `work_unit_version = 1` with
   `window_rule = network_block_work`, is rejected at preflight and at restore.
   A v2 distribution whose window contains a v1 slice is rejected; a v1
   distribution is never rebuilt as v2.
5. **No hidden conversion.** There is no function that converts `N * d` into
   `N * network_difficulty`, or the reverse, and no migration rewrites stored
   `window_target` values. Old distributions retain their stored meaning.

### 4. Retarget and multiplier interaction

1. `Q(T)` is computed when the distribution/job is built, from that job's
   network target. A later network retarget does not change the target of a
   distribution already frozen for an earlier job.
2. A new network target creates a new job generation, a new distribution and a
   new `Q`. The old job is either still valid under the protocol or stale; it
   is never re-scored against the new window target.
3. A block found under job A settles from distribution A's persisted `Q_A`,
   even if the tip has since retargeted or reorged. Reorg handling changes the
   block's current canonical/settlement status, not the frozen window
   arithmetic.
4. The `m_issue` stepdown/step-up rule in the work-unit decision changes the
   per-share `d` at a new generation. It does not change the persisted `Q` of
   an old distribution; the next distribution uses its own job target.
5. Tests must cover a retarget between distribution build and settlement, and
   a retarget between two distributions in the same run. A test that only reads
   the current network target at settlement is insufficient.

### 5. Parameters and naming

- `N` remains the multiplier (default 8). In v2 its unit is **network blocks of
  expected work**, expressed in reference units by `Q(T)`.
- `d` remains the per-share credited work in the active unit profile. For v2,
  `d = m_issue`; it is not a network difficulty.
- `window_target` remains the persisted integer threshold. For v2 it is
  `Q(T)`, not `N * d`.
- `window_rule` is the persisted discriminator with codes 0 and 1 above.
- `max_window_target_units` defaults to `2^64` and is a **hard cap**: a v2
  configuration MUST reject a larger value. A computed `Q` above it is a named
  configuration/distribution failure (`window_target_too_large`), not a
  truncation. This is also the bound used by the accounting-width analysis.
- `work_unit_version`, `window_rule` and the work-unit fingerprint travel with
  the distribution. Unknown values fail closed.

### 6. Arithmetic and overflow

1. Compute `Q(T)` with exact integer arithmetic:
   - reduce `N * (R + 1)` and `T + 1` by their greatest common divisor if the
     intermediate would overflow;
   - use a 512-bit or wider intermediate for the product, then one ceiling
     division;
   - if the result exceeds the hard cap `max_window_target_units` (itself
     `<= 2^64`) or `u128::MAX`, fail the distribution with
     `window_target_too_large` rather than saturating or truncating.
2. `sum(difficulty)` is a checked `u128` accumulation. Overflow is
   `window_work_overflow`, a fail-closed error, not a wrap or a clamp.
3. `Q(T) >= 1`; a zero, negative or non-decodable network target is rejected
   before the division. `T + 1` is never zero.
4. The implementation MUST NOT use `f64`, `difficulty_float()`, or a rounded
   network difficulty to compute `Q`.
5. `N` is validated `> 0`; the current default is 8. A zero `N` is a
   configuration error, not an empty window.
6. The window target bound does not by itself bound every payout product; the
   work-unit decision requires checked SPEC section 4.4 products and names
   their errors. This decision does not claim a universal `u128` bound for
   `F_net * DF_S`.

## Why

- The intended accounting is SLICE/PPLNS-JD: a window measured in network work,
  so its time span scales inversely with the pool's share of network hashrate.
  `N * d` measures share submissions at the pool's own target and has no such
  property.
- The current `N * d` rule was introduced to make a low-difficulty regtest
  window fill. The v2 work profile solves that problem without changing the
  meaning: on `0x200fffff`, `R = 2^255 - 1`, `m = 2`, the threshold is 65
  reference units (130 expected hashes), which is 8 blocks of about 16 hashes
  each. A CPU miner can fill that window with ordinary shares.
- Persisting both the rule and the target is the same discipline as
  `docs/decisions/2026-11-03-settlement-window-target.md`: settlement must
  reconstruct the window the front selected, not the one current config would
  select.
- Naming v1 explicitly avoids a silent policy change. Existing rows keep their
  meaning; new v2 rows carry a different rule and a different unit.
- Deriving underfill/overshoot from `high_water` and `window_target` avoids a
  schema column that could disagree with the slices.

## Alternatives rejected

- **Keep `N * d` as the universal rule and rename it "N share work".** It is
  testable and simple, but it does not implement the SLICE/TIDES window the
  product claims. Calling it network-block work would be false, and leaving it
  unqualified leaves the ambiguity that triggered this review.
- **Convert `N * d` to `N * network difficulty` at settlement using current
  bits.** This is the hidden conversion the review warned about: it changes a
  frozen window after the fact and breaks across retargets.
- **Use `floor` instead of `ceil`.** It can under-fill the declared work. The
  one-unit ceiling error is smaller and explicit.
- **Recompute `Q` at settlement from the chain's current target.** A retarget
  or a reorg would change the window a distribution was frozen from. Use the
  persisted value.
- **Persist underfill/overshoot columns.** They are derivable from the frozen
  target and high-water vector; a second stored copy can disagree with the
  slices.
- **Change v1 rows in place.** It would reinterpret history and make stored
  payouts unauditable. Version the new rule instead.

## Tests and reference vectors

These are required tests for the implementation packet.

| Vector | Value |
|---|---|
| `R` | `2^255 - 1` |
| `N` | 8 |
| Regtest bits | `0x200fffff` |
| `T` | `0x0fffff << 232` (`T + 1` is 252 bits) |
| `E_block(T)` | `2^256 / (T + 1)` = 16.000015... hashes |
| `E(R)` | 2 hashes |
| `Q(T)` | `ceil(8 * 2^255 / (T + 1)) = 65` reference units |
| `Q(T)` with `floor` | 64 (must not be used) |
| Pow-limit bits | `0x207fffff`; `Q(T_powlimit) = 9`; floor 8 |
| `m = 1` shares | each `d = 1`; 65 shares fill the threshold; 130 expected hashes |
| `m = 2` shares | each `d = 2`; 33 shares can cross 65 units; 132 expected hashes |

Required tests:

1. Exact `Q`: the vector above returns 65; a `floor` implementation returns 64
   and fails the test. Assert the exact rational equation and the `< E(R)`
   ceiling error (`< 2` hashes for this vector).
2. Pow-limit vector: `Q(0x207fffff) = 9`, floor 8, and the ceiling error is
   below `E(R) = 2`.
3. Slice walk: slices with `difficulty` 10, 10, 10 and `Q = 25` take all three
   slices, report `actual = 30`, `overshoot = 5`, and the third slice's high
   water. A slice exactly at the threshold is included; a slice below it is
   taken and the crossing slice is included.
4. Underfill: a tail whose total work is below `Q` yields an `underfilled`
   selection, pays from the available slices, and is excluded from
   window-length statistics; it is not reported as a full N-block window.
   `actual_sum`, `overshoot` and `underfilled` are derived from `high_water`
   and `window_target`; no column is written.
5. Finding slice: the slice containing the finding share is skipped even if it
   is the newest and would cross the threshold.
6. v1 regression: `N * d` still selects the existing window; the same fixture
   under v2 selects the `Q(T)` window and is visibly different.
7. Retarget: two distributions built at different `T` have different `Q`;
   settling the first uses its persisted `Q`, not the second's. A retarget
   between build and settlement does not change the first. A multiplier
   stepdown at the new generation does not rewrite the old `Q`.
8. Reorg: a canonical re-evaluation after settlement does not rewrite the
   stored `window_target`; it changes status/evidence only.
9. Persistence: `window_rule = share_work_legacy` with v1 and
   `window_rule = network_block_work` with v2 restore; the cross combinations
   fail closed. A pre-migration snapshot with a NULL rule is treated as v1 and
   settles with its stored `N * d`. A v2 row must use code 1; unknown codes
   fail decode.
10. No float: a proptest compares `Q` and the slice walk with a big-integer
    oracle; the v2 path has no `f64`.
11. Rounding/overflow: a synthetic `Q` above `max_window_target_units` or
    `u128::MAX` fails closed with `window_target_too_large`; a checked window
    sum overflow fails with `window_work_overflow`; a zero network target and a
    zero `N` fail closed.
12. Window meaning: a test that measures time-to-fill at two simulated pool
    hashrates (or two actual rates) shows the v2 window spans more wall time at
    the lower hashrate for the same network target, while `N * d` does not.
    This is a contract test for the property, not a statistical proof.

## Edits required at merge (not applied by this packet)

- `docs/SPEC.md` frontmatter: splice the companion changelog fragment.
- `docs/SPEC.md` section 4.1: change the `N` meaning to "network blocks of
  expected work" for the v2 window rule; keep the v1 legacy note; add
  `max_window_target_units`.
- `docs/SPEC.md` section 4.3: add `Q(T)` and the persisted `window_rule`; retain
  the finding-slice exclusion and whole-slice rule; state that underfill and
  overshoot are derived from `high_water`/`window_target`, not stored.
- `docs/SPEC.md` section 5.1: add `window_rule` with codes 0/1 and the
  work-unit version/fingerprint to `distribution_snapshot`; add the
  version/fingerprint to `slice`; add the enum/CHECK constraints.
- `docs/SPEC.md` section 5.3: add `window_rule`/work-unit restore rejection.
- `docs/SPEC.md` section 7.4: settle with the snapshot's `window_rule` and
  `window_target`; never recompute from current bits.
- `docs/WIRE-FORMATS.md` section 3.1/3.6: `spc:blocks` gains
  `U8 window_rule` after the work-unit fields; the enum is the same one used by
  the database. Old-format events fail decode.
- `docs/decisions/2026-11-02-share-target-policy.md`: add a superseded-in-part
  note for the v2 window; leave its v1 history intact.
- Tests in `crates/spc-accounting/src/tests.rs` and
  `bins/spc-pool/src/front.rs` that assert `N * d` must be split into explicit
  v1 fixtures and new v2 fixtures; no existing assertion is silently flipped.
