-- Persist the exact window target (N x d) a distribution was built against
-- (SPEC §4.3, §7.4).
--
-- Settlement reconstructs the paid window with `select_window`, whose target
-- is `N x d` for the `d` in force when the front built the distribution. A
-- difficulty retarget between the newest stored slice and the finding block
-- made the slice-derived estimate wrong, so the exact value the front used is
-- frozen here with the rest of the snapshot metadata.
--
-- Nullable: rows written before this migration have no value, and settlement
-- falls back to the (warned) slice estimate for those rows only.
--
-- NUMERIC(40,0): the same exact u128 encoding as `slice.difficulty`. The
-- front computes `N x d` in u128 (saturating), which can exceed i64/BIGINT,
-- and NUMERIC is exact - no silent truncation.
ALTER TABLE distribution_snapshot
    ADD COLUMN window_target NUMERIC(40, 0);
