-- Bound the ledger republish path (SPEC §5.2, §7.4).
--
-- A confirmed block starts unpublished; payout marks it published only after
-- the `LedgerApplied` XADD succeeds. The republish timer selects only
-- unpublished blocks, so a healthy front does not make the event stream grow.
ALTER TABLE block_found
    ADD COLUMN ledger_published BOOLEAN NOT NULL DEFAULT true;
