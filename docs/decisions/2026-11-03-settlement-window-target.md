# 2026-11-03 - Persist the settlement window target

**Trigger.** Settlement reconstructed the PPLNS window target by
*estimating* it from the stored slices: `N x d` where `d` is the share
difficulty implied by the newest (or finding-height) slice's
`difficulty / n_shares`. The front does not estimate — it computed the
exact `N x d` when it selected the window — so the two sides could
disagree. A difficulty retarget between the reference slice and the
finding block (the slice sealed before the retarget, the block after)
makes the estimate wrong by the retarget ratio, and settlement then
rebuilds a *different window* than the one the distribution was frozen
from: the reconstruction mismatches the snapshot entries and settlement
holds, or worse, settles a window the front never intended.

## Decision

1. **`distribution_snapshot.window_target NUMERIC(40,0)`, nullable.** The
   front persists the exact `N x d` it selected the window with, at the
   moment it builds the distribution. `NUMERIC(40,0)` is the same exact
   u128 encoding `slice.difficulty` already uses; `N x d` is computed in
   u128 (saturating) because it can exceed BIGINT, and a value that does
   not fit is stored, not truncated. Nullable because rows written before
   the migration have no value.

2. **The `spc:blocks` event carries it.** `BlockEvent` gains
   `window_target: U128` (encoded after `high_water`, WIRE-FORMATS
   §3.1), and `payout` persists the event's value into the snapshot row.
   `front` and `payout` are roles of one binary, so the wire format
   changes atomically in a single deploy; an old-format event fails
   decode (fail closed) rather than being interpreted with a guessed
   target.

3. **Settlement uses the persisted value; only pre-migration rows
   estimate.** `reconstruct` takes the window target from the snapshot
   row. A NULL value means the row predates the column: settlement falls
   back to the slice estimate *and warns*, naming the block, so an
   estimated settlement is always visible. A stored value that is not a
   u128 decimal string is a corrupt row: settlement fails closed (the
   consumer group retries and alarms), never a silent fall-back.

## Why

- The window is the settlement contract (§4.3, §7.4): the distribution
  was frozen from a specific set of slices, and rebuilding that set
  requires the same target it was selected with. Everything else in the
  reconstruction (the high-water vector, the entries) is already frozen;
  the target was the one input still derived from mutable state.
- The estimate is not merely imprecise, it is *wrong across a retarget*:
  `d` is read from a slice sealed at the old difficulty while `N` counts
  shares the front accepted at the new one. There is no way to recover
  the front's exact target from the stored slices after the fact, which
  is exactly why the front must persist it at build time.
- Fail closed on unparseable values matches the packet rules: a
  `NUMERIC(40,0)` can hold values beyond u128 (2^128 itself fits), and
  guessing or zero-filling such a row would silently mis-size the window.

## Alternatives rejected

- **Keep estimating, but pin the reference slice**: the finding share
  lives in the still-open slice above the high-water mark (2026-11-02
  follow-up, decision 1), so the "reference" is at best the newest
  *sealed* slice — sealed before the block, i.e. before any retarget
  boundary between them. Pinning cannot fix a retarget that lands
  between the seal and the find.
- **Recompute `N x d` in settlement from config**: config can change
  between the find and the settle (`share_difficulty`, `window_n`), and
  settlement must reconstruct *this* block's window, not the current
  policy's.
- **`BIGINT` / i64 column**: `N x d` with mainnet-scale `d` and `N = 8`
  exceeds i64; a silent clamp would mis-size the window exactly like the
  estimate does.
- **Store the per-share `d` and multiply at settle time**: same retarget
  problem one level down, plus a second derivation path to keep in sync
  with the front's.

## Edits applied

- `crates/spc-store/migrations/0004_distribution_window_target.sql`: the
  column. Numbered 0004, not the 0003 the packet suggested, because
  `0003_ledger_published.sql` already exists.
- `crates/spc-store/src/models.rs`, `crates/spc-store/src/postgres.rs`:
  `SnapshotRow.window_target: Option<String>` (u128 decimal string),
  bound `$n::numeric` on insert, selected `window_target::text` — the
  `slice.difficulty` convention.
- `crates/spc-store/src/wire.rs`: `BlockEvent.window_target: u128` after
  `high_water`, u128 LE per the codec's existing `U128`.
- `bins/spc-pool/src/front.rs`: the front keeps the `N x d` it computed
  in `refresh` and emits it in the block event.
- `bins/spc-pool/src/payout.rs`: `ReconstructInput.window_target`, the
   warned estimate fallback for NULL rows, fail-closed parse, and the
   tests (retarget scenario, pre-migration fallback, corrupt value).
- `crates/spc-store/tests/postgres.rs`: round-trip at u128::MAX,
   nullability for pre-migration rows, and the 0004-on-0003 migration
   test on a scratch database.
- `docs/SPEC.md`: frontmatter changelog (2026-11-03), §3.3 step 5, §5.1
  table, §5.2 stream table, §7.4.
- `docs/WIRE-FORMATS.md`: §3.1 `U128 window_target`.
