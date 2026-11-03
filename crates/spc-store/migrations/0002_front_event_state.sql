-- SPEC §5.2/§7.4: the highest `spc:front:events` revision `payout` has
-- published. Fronts ignore events at or below their bootstrap revision, so a
-- front restart cannot double-apply a ledger delta that is already reflected
-- in `pending_balance`.
--
-- Singleton row; `payout` upserts it with
-- `GREATEST(rev + 1, now_ms)` before every XADD.

CREATE TABLE front_event_state (
    id  SMALLINT PRIMARY KEY CHECK (id = 1),
    rev BIGINT NOT NULL CHECK (rev >= 0)
);
