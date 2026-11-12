# 2026-11-05 - Lab share targets and reference work units

Status: revised candidate for Fable re-review. This record is not normative
until Fable approves it and the merge session applies the companion changelog
fragment to `docs/SPEC.md`.

**Trigger.** The regtest lab needs real CPU-scale shares. The current policy
credits an integer share difficulty and clamps the effective share target to be
no harder than the network target. On a low-difficulty regtest chain that makes
every accepted share block-qualified, so a CPU miner cannot exercise ordinary
(non-block) shares, and a target update cannot change per-share credit without
either a float conversion or an integer truncation.

The existing implementation is the legacy baseline, not a suitable general
work representation:

- `[accounting] share_difficulty` is a `u64` and MUST be `> 0`;
- `target_for_difficulty(d)` is `difficulty_1_target / d`, with integer division;
- `effective_share_target(d, bits)` is the easier of that target and the
  network target, so on easy regtest bits the network target wins;
- `credited_difficulty(d, bits)` is
  `min(d, max(1, round(network_difficulty(bits))))` and uses `f64` in
  `network_difficulty`;
- every consumer - `ShareRec.d`, the slice `difficulty`, slice credits,
  type-32 aggregates, and payout weights - treats that number as the credited
  work.

Those functions are correct for the policy recorded in
`docs/decisions/2026-11-02-share-target-policy.md`. They are not a CPU-scale
work model: a target update from one difficulty to another changes the number
of accepted submissions but not the work represented by each submission, and
there is no exact relationship between a submission and expected hashes.

This record decides the representation independently of the window rule. The
window rule is decided in
`docs/decisions/2026-11-05-window-network-block-work.md`.

## Decision

### 1. A versioned work-unit profile, with the legacy policy frozen

1. **Version 1 (`v1`) is the existing policy, unchanged.** It retains integer
   `share_difficulty`, the `max(share, network)` effective target, the
   `min(d, max(1, round(network_difficulty)))` credit, and the existing float
   only where v1 already uses it. A v1 profile is never silently reinterpreted
   as v2. Existing rows and distributions keep v1.
2. **Version 2 (`v2`) is the CPU-scale lab profile.** It is opt-in for a lab
   run and any future deployment that adopts it. It uses an exact,
   fingerprinted reference target and an integer work multiplier. It does not
   use `f64` anywhere in target selection, credit, window selection or
   persistence.
3. **The two versions do not mix inside one live accounting window.** The
   stored version and fingerprint select the arithmetic. A restore, a front
   restart or a distribution build that would put v1 and v2 work in one window
   fails closed. Settlement of an already frozen v1 distribution may still read
   its own v1 `window_target`; that is history, not a new window.

### 2. Expected work is defined by `T + 1`, not by floating difficulty

For a 256-bit target `T` (the inclusive threshold, `0 <= T < 2^256`) and a
uniformly distributed 256-bit hash, the exact expected number of hashes to a
solution is

```
E(T) = 2^256 / (T + 1)          # a rational number; not a float
```

The acceptance test is `hash <= T`; `hash = T` is accepted and `hash = T + 1`
is not. `T = 2^256 - 1` is the all-ones target and has `E = 1`.

The current code's `difficulty_float()` and `round()` are not used by v2 for
this relationship. A v2 implementation computes with integers and exact
rational comparisons, and only displays a decimal approximation in diagnostics.

### 3. The exact target family and profile validity

A v2 profile declares:

- a **reference target** `R = 2^k - 1` for an integer `k` with
  `1 <= k <= 255`. Equivalently, `R + 1 = 2^k` is an exact power of two that
  fits in 256 bits. A value whose `R + 1` is not a power of two is rejected at
  preflight with `reference_not_power_of_two`; it would be a new version.
- a configured **work multiplier** `m_cfg = 2^j_cfg` for an integer
  `j_cfg` with `0 <= j_cfg <= k - 1`. A value that is not a power of two is
  rejected at preflight with `multiplier_not_in_family`.
- `m_cfg <= max_work_multiplier <= 2^32`; `max_work_multiplier` is a hard cap,
  so a v2 configuration MUST reject a larger value because the accounting
  width bound in section 5 is part of the profile contract.
- the network's effective `pow_limit_bits` from capability introspection.

**Profile validity.** A v2 profile is valid for a network only if

```
R >= T_powlimit
```

where `T_powlimit` is the exact target decoded from the network's effective
`pow_limit_bits`. Preflight fails with the named error
`reference_below_pow_limit`, naming `R` and `T_powlimit`, otherwise. It also
validates the `k` bounds, the exact-power condition, `j_cfg` bounds and the
`max_work_multiplier` cap. This is what makes the family usable at genesis, at
a retarget clamp, and at a minimum-difficulty exception: `m = 1` has
`S(1) = R >= T_powlimit`, and every consensus target satisfies
`T <= T_powlimit`, so a feasible member always exists.

The share target for a multiplier `m = 2^j` is

```
S(2^j) = 2^(k - j) - 1
```

subject to:

- `0 <= j <= k - 1` (the family is exactly the powers of two; divisibility is
  free);
- `S(m) >= 1` (a zero target is invalid; this is exactly `j <= k - 1`);
- `S(m) >= T` for the network target current at issuance. A target harder than
  the network target is rejected for that generation, never silently clamped,
  because clamping would make `m` overstate the work the accepted share
  actually represents.

When those conditions hold,

```
E(S(2^j)) / E(R) = 2^k / 2^(k-j) = 2^j = m     # exact, no rounding
```

and the credited `d` for a share at `S(m)` is exactly `m`. The work unit is
`E(R)`; `d` is the number of reference units. `m = 1` is the easiest target in
the family, so every accepted v2 share has `d >= 1` without a clamp.

A convenient regtest reference is `R = 2^255 - 1` (`k = 255`), for which
`R + 1 = 2^255` and the exact multipliers are `1, 2, 4, ...`. It is
larger/easier than the regtest pow limit `0x207fffff`
(`T_powlimit = 2^255 - 2^232`), so it passes profile validity.
`R = 2^254 - 1` (`k = 254`) is retained only as a negative vector for that pow
limit; it is not the recommended profile.

A target outside the power-of-two family is rejected with a
configuration/update error. It is never rounded to the nearest member. This is
the central difference from a fixed-point representation: v2 claims exact
ratios only for targets it can represent exactly, and it refuses the rest.

### 4. Bounded stepdown and step-up at a new job generation

The configured `m_cfg = 2^j_cfg` is a ceiling, not an unconditional per-share
credit. The selection is **O(1)**; it MUST NOT scan `[1, m_cfg]` or search for
divisors. Define

```
ceil_log2(n) = (n - 1).bit_length()          # n >= 1; exact power boundary works
j_issue      = min(j_cfg, k - ceil_log2(T + 1))
m_issue      = 2^j_issue
```

For each new job generation with network target `T` (with `T >= 1`):

1. `j_issue >= 0` because a valid profile has `R >= T_powlimit >= T`, hence
   `2^k >= T + 1`; `m_issue = 1` is always valid. `S(m_issue) >= T` follows from
   the formula, including the exact-power boundary: if `T + 1 = 2^p`, then
   `ceil_log2(T + 1) = p`, `j_issue = min(j_cfg, k - p)`, and
   `S = 2^p - 1 = T`.
2. The generation records `m_issue`, `j_issue`, computes `d = m_issue`, and
   uses `S(m_issue) = 2^(k - j_issue) - 1` for submission validation.
3. If `j_issue < j_cfg`, emit a `multiplier_stepdown` event carrying `j_cfg`,
   `j_issue`, `T`, `S(m_issue)` and the generation id.
4. If a previous generation stepped down and the next generation has
   `j_issue == j_cfg`, use `m_cfg` again and emit `multiplier_stepup`.
   Step-up happens at a new generation only; it never re-prices an old share.
5. `target_profile_unplannable` is a defensive guard only. It is unreachable
   for a valid profile because `m = 1` is in the family; if observed, it is a
   profile/configuration bug and the generation fails closed.
6. A stepdown is not a clamp: the target and the credited `d` both change to the
   selected member. The old generation remains untouched.

This covers pow-limit templates, retarget clamps and minimum-difficulty
exceptions with constant-time integer bit-length logic and no trial division.

### 5. Arithmetic, rounding, compact decode, zero and overflow

1. **Decode a compact target with the consensus `SetCompact` rule.** For
   `size = bits >> 24` and `word = bits & 0x007fffff`:
   - reject when `word != 0` and the sign bit `0x00800000` is set;
   - reject when `word != 0` and
     (`size > 34` or (`word > 0xff` and `size > 33`) or
      (`word > 0xffff` and `size > 32`));
   - shift right `8 * (3 - size)` when `size <= 3`, otherwise left
     `8 * (size - 3)`;
   - reject a zero result.

   Exponent 33 with `word <= 0xffff` and exponent 34 with `word <= 0xff` are
   valid 256-bit targets. There is no arbitrary exponent-32 cutoff. The node,
   not the pool, enforces `target <= pow_limit`; the pool still checks
   `S(m) >= T` for issuance.
2. **Compare as 256-bit integers.** A hash meets `T` iff
   `u256(hash) <= u256(T)`. Do not compare decimal strings, do not compare
   big-endian arrays after losing leading zeros, and do not use a float
   threshold.
3. **Compute `S(m)` with one 256-bit division after the divisibility check.**
   `R + 1` is a 257-bit quantity only in the excluded `R = 2^256 - 1` case;
   the `R <= 2^256 - 2` restriction keeps it in 256 bits. `S(m) + 1` is the
   quotient, so no second rounding is needed.
4. **No clamp to 1, no saturation, and no silent zero.** Every invalid case is
   an error: `m = 0`, a non-power `m`, `R + 1` not a power of two or outside
   the supported `k` bounds, `S = 0`, a target harder than the network target
   with no feasible member (defensive only), an all-ones sentinel used as `R`,
   or a network target that decodes to zero.
5. **Profile and intermediate bounds.** `m_cfg = 2^j_cfg` is a power of two
   with `m_cfg <= max_work_multiplier <= 2^32`; `max_work_multiplier` is a hard
   cap. The window target bound is `Q <= max_window_target_units <= 2^64`
   (see the window decision); `max_window_target_units` is also a hard cap. A
   v2 profile MUST also require `max_slice_shares <= 65 000` unless a new width
   analysis is recorded. With `max_slice_shares = 65 000` and the lab's fee
   bound `f < 2^51` (a custom supply profile must re-evaluate this):
   - one slice's `sum(d) < 65 000 * 2^32 < 2^48`;
   - one slice's `sum(d*f) < 65 000 * 2^32 * 2^51 < 2^99`;
   - `D_win < Q + one slice < 2^64 + 2^48 < 2^65`;
   - `r_net * D_win < 2^51 * 2^65 = 2^116`.

   These are per-slice/current-limit calculations, not a universal proof for
   window-level fee aggregates. In particular, `DF_S` (the window sum of
   `d*f`) and `F_net * sum(d*f)` can exceed `u128` even when each per-slice
   value fits. v2 therefore uses checked arithmetic and an exact wider
   multiply/divide for the SPEC section 4.4 products; it does not claim a
   universal `u128` bound it has not calculated.
6. **v2 accounting is checked, v1 stays saturating.** The named fail-closed
   errors are:
   - `slice_work_overflow` when adding `d` to a slice would overflow `u128`;
   - `slice_fee_work_overflow` when `d*f`, a per-identity `diff_fee_sum`, or a
     slice `diff_fee_sum` overflows `u128`;
   - `window_work_overflow` when the window sum overflows;
   - `payout_product_overflow` when a SPEC section 4.4 product or its exact
     quotient cannot be represented in the required width;
   - `distribution_overflow` when a payout or coinbase amount does not fit its
     output type;
   - `reference_not_power_of_two` when `R + 1` is not `2^k`;
   - `multiplier_not_in_family` when `m_cfg` is not a power of two or is
     outside `[1, 2^(k-1)]`.

   A v2 `Slice::push` rejects the share before mutating the slice on
   `slice_work_overflow` (it does not silently seal and continue). A v2
   checked `agg` fails the seal/distribution on fee-work overflow. A v2
   `select_window` fails on window overflow. A v2 `raw_payouts` uses a reviewed
   exact wider multiply/divide for `r_net * d`, `F_net * df` and
   `t * DF_S`, and fails with `payout_product_overflow` if the exact result or
   final `u64` does not fit. v1 keeps its existing saturating functions
   unchanged, version-gated.

### 6. Per-job and per-channel target updates

The work represented by a share is the work of the job whose header the miner
actually hashed. It is not whatever target happens to be current when the
submission reaches the front.

1. A job record carries its exact `share_target`, its `m_issue`, its
   `work_unit_version`, its `work_unit_fingerprint`, a monotonic target
   generation, and an activation state (future/active). The front and the SV2
   listener use the job record's target for submission validation and
   accounting.
2. **SV2 `SetTarget` is standard, not an invented extension.** Pinned
   sv2-spec `d8bf5b8` `05-Mining-Protocol.md` section 5.3.21 says `SetTarget`
   applies to jobs received on the channel in the future or already received
   with an empty `min_ntime`; it is not applicable to already-received jobs
   with `min_ntime = nTime`, whose maximum target remains stable. A future
   job's target is the channel target in effect when its `SetNewPrevHash`
   activates it. L1 inherits that rule rather than a pool-local grace policy.
3. A target update or a network retarget creates a new generation. The old
   generation remains valid only while its job is valid under the negotiated
   protocol. A share for an old generation is either credited at that old
   generation's `m_issue` or rejected as stale; it is never re-credited at the
   new target.
4. A new `nbits` (network retarget) is a new job generation. The block path
   uses the new job's network target; the accounting path uses the new job's
   share target. A distribution built for an old job cannot be used to settle a
   block found under a new job unless the pool's normal job/distribution
   ordering explicitly retains it; settlement uses the frozen distribution's
   persisted target.
5. A reconnect starts a new session and a new target announcement. Old jobs
   are not reused across the reconnect.
6. The v2 profile check and multiplier selection are repeated at every job
   issuance and target update. They use the stepdown/step-up rule in section 4,
   not a clamp and not an unconditional `m_cfg`.

**Oracle source.** v2 credit is not reconstructible from `job_id` alone. The
front (or the harness capturing the pool job path) emits a `job_issued` record
with `front_id`, `channel_id`, `job_id`, `generation`, `nbits`, `share_target`,
`m_issue`, `work_unit_version`, `work_unit_fingerprint`, `issued_at` and
`activation`. `accounted` carries `job_id` and `generation`. The independent
oracle recomputes each sampled share's `d` from `job_issued` plus the share hash
and compares it with `slice_credit.diff_sum`. If the job record is missing or
ambiguous, the oracle reports `unknown`, not a guessed credit.

The type-32 v2 audit is **out of scope for this packet and not claimed**: the
current share blob/leaf does not carry `job_id -> (share_target, m_issue)`.
A future change to `docs/share-accounting-ext-deltas.md` must define how a v2
verifier obtains that mapping (an extension field or a lab evidence endpoint)
before any v2 type-32 claim. v1 type-32 behavior is unchanged.

### 7. Persistence, fingerprint and restore rejection

1. A v2 profile has a stable fingerprint

   ```
   work_unit_fingerprint = sha256("spc-lab/work-unit/v2" || 0x00 || R_be32)
   ```

   where `R_be32` is the 32-byte big-endian reference target. The fingerprint
   does not depend on `m_cfg`, `m_issue`, `N`, the pool, the node or the
   scenario. For `R = 2^255 - 1` the fingerprint is
   `7fe9ed1a4e2d4279b302cbb0bcf709758f03dc44db6dab82ac40c04eb0d77c6b`.
   The old `R = 2^254 - 1` vector is
   `2fc4270b351c943cc49909e369f72df47e38ed48ee0f69bdf0bc1b2cb76a1859`;
   it is rejected against the regtest pow limit, not used for the lab profile.
2. `work_unit_version` and `work_unit_fingerprint` are persisted on every new
   sealed slice and every new distribution snapshot. v1 rows have version 1
   and a NULL fingerprint. v2 rows have version 2 and a non-NULL fingerprint.
   The database constraints are:
   - `CHECK (work_unit_version IN (1, 2))`;
   - `CHECK ((work_unit_version = 1) = (work_unit_fingerprint IS NULL))`.
3. On front restore, every slice used to rebuild the live window must have the
   configured version and fingerprint. A mismatch is a startup failure with a
   named error (`work_unit_restore_mismatch`), naming the slice id and the two
   fingerprints. A v2 front MUST NOT convert v1 `d` values, treat them as v2
   units, or zero-fill the missing fields.
4. A settlement of an already frozen distribution uses that snapshot's
   persisted `work_unit_version`, `work_unit_fingerprint`, `window_rule` and
   `window_target`. A payout role running with a different configured profile
   may settle old rows because it is reading history; it MUST reject a new
   distribution that mixes versions in one window.
5. **The wire fields are definite, not conditional.** Append to `spc:slices`
   after its existing fields:
   `U8 work_unit_version`, `B0_32 work_unit_fingerprint`.
   Append to `spc:blocks` after its existing fields:
   `U8 work_unit_version`, `B0_32 work_unit_fingerprint`,
   `U8 window_rule` (the rule enum is defined in the window decision).
   v1 uses an all-zero 32-byte fingerprint sentinel on the wire; the database
   stores NULL for v1. A v2 event with an all-zero fingerprint fails decode.
   Front and payout are roles of one binary, so the wire and database change
   deploy atomically; old-format events fail decode rather than being
   interpreted with a guessed profile.
6. `pending_balance`, ledger rows, denomination rungs and Ark delivery do not
   gain a second unit. They are satoshi amounts and remain unchanged. A v1
   pending balance is not converted into v2 units because it is already a
   monetary balance.

### 8. Configuration surface (semantic names only)

The following are semantic fields for the implementation packet. Their exact
TOML/CLI spelling and whether they live in `[accounting]` or a lab profile
section are for the implementation and the merge session; this record does not
assert that a running binary already has them.

- `work_unit_version`: `1` (legacy) or `2` (reference-target exact family).
- `work_reference_target`: `R = 2^k - 1` in 32-byte hex, required for v2,
  forbidden for v1; `1 <= k <= 255`.
- `share_work_multiplier`: `m_cfg = 2^j_cfg`, a power of two, required for v2,
  forbidden for v1.
- `max_work_multiplier`: hard cap, default `2^32`; values above are rejected
  for v2. `m_cfg` must not exceed it.
- `max_window_target_units`: hard cap, default `2^64`; values above are
  rejected for v2. See the window decision.
- The effective profile is exposed read-only for preflight comparison: version,
  reference target, fingerprint, `m_cfg`, current `m_issue`, current share
  target, pow limit and the network target it was checked against.

The existing `share_difficulty` remains the v1 field. A v2 configuration MUST
NOT accept `share_difficulty` as an alternative spelling for `m_cfg`, because
`m_cfg` is a reference work multiplier, not a difficulty-1 ratio.

## Why

- The lab's purpose is to observe real CPU shares, including non-block shares.
  Equal credit per submission is not work-proportional across target changes,
  and the current clamp makes the question untestable on easy regtest bits.
- The type-32 audit and the payout oracle need a work weight they can recompute
  exactly. A float ratio cannot represent it without rounding; a fixed-point
  floor is deterministic but systematically under-credits the exact ratio and
  needs a propagated error-bound contract before payout conclusions.
- The exact family is small enough to implement in one pure module and one
  persistence migration. It gives an exact `E(S)/E(R) = m` proof instead of an
  approximate claim, and it makes unsupported targets a visible preflight
  failure.
- `R >= pow_limit` plus `m = 1` is what makes the family total over the
  consensus targets a node can emit. Without it, a valid-looking profile would
  stall at genesis, a retarget clamp or a minimum-difficulty block.
- The stepdown rule is not a target clamp: it changes both the target and the
  credit to the same exact member, records the event, and leaves old jobs
  untouched.
- Bounding `m` and `Q` and checking every intermediate is necessary because
  `sum(d)` being small does not bound `sum(d*f)` or the fee products. v1's
  saturation is preserved only because it is historical; v2 fails closed.
- Keeping v1 frozen protects existing rows and deployments. A v2 restore that
  meets v1 rows must fail, not reinterpret them.
- Per-job credit prevents a target update from rewriting history. This is the
  same principle as the persisted settlement window target: freeze the exact
  input that was used, then audit against it.

## Alternatives rejected

- **Use `share_difficulty` as a floating value and round per share.**
  Binary64 cannot represent the 256-bit ratio exactly, and JavaScript
  `Number`/cross-language rounding makes independent agreement fragile.
- **Keep the integer difficulty and make all regtest shares blocks.** It does
  not meet the lab's non-block-share requirement and cannot test target
  updates.
- **Use a fixed-point multiplier for arbitrary targets now.** It can be
  deterministic and auditable, but it systematically under-credits the exact
  ratio and would need its own propagated error-bound contract and residual
  policy. Revisit only with a new version.
- **Admit arbitrary `R + 1` and search for the largest divisor.** The set
  comprehension is not an algorithm: a descending scan costs up to `2^32`
  256-bit moduli per job generation, and factoring `R + 1` is not feasible in
  the general case. A declared divisor list is a possible future version, but
  the lab adopts the power-of-two family instead.
- **Allow a non-power-of-two `R + 1` under the same version.** It would make
  the issuance selection unbounded and change the exact family semantics. A
  future version needs its own bounded selection algorithm.
- **Clamp `S` to the network target when `S < T`.** That would accept a share
  at the network target but credit a harder target's multiplier, which is an
  overpayment and a hidden unit conversion.
- **Keep `R = 2^254 - 1` because it matched the overlay vector.** It is below
  the regtest pow limit, so a pow-limit template would have no feasible member.
  Keep it only as a negative test vector.
- **Let `m` use all of `u64`.** The `sum(d*f)` path would exceed `u128` under
  ordinary fee bounds, and the SPEC section 4.4 fee products are wider still.
- **Store `R` per share.** It is constant for a run profile; per-share storage
  adds bytes and a way for rows to disagree. The fingerprint binds the profile.
- **Treat the v1 `d` as "the same integer" and mix it with v2 `m`.**
  Numerically similar values have different meanings. Mixing is exactly the
  silent conversion this record forbids.

## Tests and reference vectors

These are required tests for the implementation packet. They are vectors, not
claims that the code exists today.

| Vector | Value |
|---|---|
| Recommended reference | `R = 2^255 - 1 = 0x7fff...ff`; `E(R) = 2` hashes |
| Regtest pow limit | `0x207fffff`; `T_powlimit = 2^255 - 2^232`; `R >= T_powlimit` |
| `m = 1` | `S = 2^255 - 1`; `E(S) = 2`; `d = 1` |
| `m = 2` | `S = 2^254 - 1`; `E(S) = 4`; `d = 2` |
| `m = 4` | `S = 2^253 - 1`; `E(S) = 8`; `d = 4` |
| `m = 8` | `S = 2^252 - 1`; `E(S) = 16`; `d = 8` |
| `m = 16` | `S = 2^251 - 1`; `E(S) = 32`; harder than `0x200fffff`, rejected |
| `0x200fffff` | `T = 0x0fffff << 232`; `E(T) = 16.000015...` hashes |
| Stepdown | `m_cfg = 8`, `T = T_powlimit` -> `m_issue = 1`; `m = 2` is not feasible |
| Exact power boundary | `T = 2^252 - 1` -> `T+1 = 2^252`, `j_issue = 3`, `m_issue = 8`, `S = T`; `T = 2^252` -> `j_issue = 2`, `m_issue = 4` |
| Non-power reference | `R = 2^254` (`R + 1 = 2^254 + 1`) -> `reference_not_power_of_two` |
| Non-power multiplier | `m_cfg = 3` -> `multiplier_not_in_family` |
| Old vector | `R = 2^254 - 1`, `pow_limit_bits = 0x207fffff` -> `reference_below_pow_limit` |
| Fingerprint | `R = 2^255 - 1` -> `7fe9ed1a...d77c6b` |
| Old fingerprint | `R = 2^254 - 1` -> `2fc4270b...6a1859` (negative vector) |

Required unit tests:

1. Exact family: for `R = 2^255 - 1` (`k = 255`) and each `m = 2^j`
   above, `S = 2^(k-j) - 1`, `S + 1 == (R + 1) / m`, and the exact integer
   equation `(R + 1) == m * (S + 1)`. Check the chosen fingerprint preimage.
2. **Profile validity:** `profile_rejects_reference_below_pow_limit` with
   `R = 2^254 - 1`, `pow_limit_bits = 0x207fffff` returns
   `reference_below_pow_limit` naming both values; `R = 2^255 - 1` passes;
   `R = 2^254` returns `reference_not_power_of_two`; `k = 0` and `k = 256`
   are rejected by the supported `k` bounds.
3. **Stepdown/step-up:** with `m_cfg = 8`, a generation at `0x200fffff` uses
   `m_issue = 8`; after a generation at `0x207fffff` it uses `m_issue = 1`
   and emits `multiplier_stepdown`; a later generation at `0x200fffff` uses
   `m_cfg = 8` again and emits `multiplier_stepup`. The old generation's
   credits are unchanged. The exact-power boundary `T = 2^252 - 1` gives
   `m_issue = 8` and `T = 2^252` gives `m_issue = 4`.
4. Range/rejection: `m = 0`; non-power `m = 3` -> `multiplier_not_in_family`;
   `S = 0`; `R = 2^256 - 1`; `R + 1` overflow; zero network target; a target
   harder than the network target with no feasible member;
   `m_cfg > max_work_multiplier`.
5. Compact decode consensus vectors: `0x2100ffff` and `0x220000ff` accepted;
   `0x21010000` and `0x22000100` rejected overflow; `0x00123456` rejected
   zero; `0x20800000` rejected sign; `0x02123456` accepted as 13-bit;
   `0x207fffff` accepted.
6. Boundary: a synthetic hash equal to `S` is accepted; `S + 1` is rejected;
   a hash equal to `T_network` is a block and a hash just above it is not.
7. No float: a proptest compares target decode, family arithmetic and target
   comparison against a big-integer oracle; the v2 module has no `f64` input or
   output.
8. Target update: job generation A with `m = 1`, generation B with `m = 2`; a
   share for A after B is credited as 1 while A is valid, or rejected as stale
   after A is invalidated; it is never credited as 2.
9. **Accounting overflow set:** slice `sum(d)`, slice `sum(d*f)`,
   `diff_fee_sum`, `D_win`, each SPEC section 4.4 product and each final `u64`
   conversion returns its named fail-closed error under v2 on a synthetic
   overflow. The same fixture under v1 keeps its documented saturating result.
10. **Oracle:** `job_issued` plus a sampled share recomputes `d = m_issue` and
    matches `slice_credit.diff_sum`; a share credited at the wrong generation
    is caught. A missing job record yields `unknown`.
11. Persistence: v1 slice rows with a v2 config fail restore with
    `work_unit_restore_mismatch`; v2 rows with a different `R` fail; the
    database CHECK rejects `(version = 1, fingerprint not NULL)` and
    `(version = 2, fingerprint NULL)`; a v1 distribution still settles from
    its persisted v1 target; a v2 distribution with a v1 slice in its window
    fails to build. Wire decode fails for an old-format event and for a v2
    event with a zero fingerprint.
12. Compatibility: the existing v1 tests in `bins/spc-pool/src/sv1.rs` and
    `bins/spc-pool/src/front.rs` pass unchanged.

## Edits required at merge (not applied by this packet)

- `docs/SPEC.md` frontmatter: splice the companion changelog fragment.
- `docs/SPEC.md` section 3.4: state the v2 power-of-two family
  (`R + 1 = 2^k`, `m = 2^j`), supported `k` bounds, profile validity,
  O(1) stepdown/step-up and per-job credit.
- `docs/SPEC.md` section 4.1: add the versioned work-unit parameters and the
  hard caps `max_work_multiplier`/`max_window_target_units`.
- `docs/SPEC.md` section 4.2: state that v2 `Slice::push` and `agg` use checked
  accumulation (`sum(d)`, `d*f`, per-identity and slice `diff_fee_sum`) with
  the named errors; v1 keeps saturation.
- `docs/SPEC.md` section 4.4: state that v2 payout products use checked/exact
  wider multiply/divide and fail with `payout_product_overflow`; v1 keeps
  saturation.
- `docs/SPEC.md` section 5.1: add the version/fingerprint columns and the
  database CHECKs to `slice` and `distribution_snapshot`; state that
  window-level underfill/overshoot metadata is derived, not stored.
- `docs/SPEC.md` section 5.3: add the restore rejection rule.
- `docs/SPEC.md` section 7.4: settle from the snapshot's persisted version,
  target and rule.
- `docs/WIRE-FORMATS.md` section 3.1/3.6: append the definite fields from
  section 7.5, including the v1 zero fingerprint sentinel.
- `docs/share-accounting-ext-deltas.md`: add a merge note that v2 type-32
  auditing needs a `job_id -> (share_target, m_issue)` source; this packet does
  not claim it.
- `docs/decisions/2026-11-02-share-target-policy.md`: add a superseded-in-part
  note pointing here for v2; do not rewrite its v1 history.
