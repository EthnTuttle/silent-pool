-- Private SPc-side audit ancestry. This is NOT an Ark stream or public DTO.
-- Identity attribution remains pool-attested; consumers independently verify
-- raw share/header/fee evidence and replay these actual committed transitions.
CREATE TABLE source_accounting_history (
    event_id BIGSERIAL PRIMARY KEY,
    transaction_id BIGINT NOT NULL DEFAULT txid_current(),
    table_name TEXT NOT NULL,
    operation TEXT NOT NULL CHECK (operation IN ('history_start','INSERT','UPDATE','DELETE')),
    before_row JSONB,
    after_row JSONB,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO source_accounting_history(table_name,operation,after_row)
SELECT 'checkpoint','history_start',jsonb_build_object(
    'schema','spc.accounting-history/1',
    'complete_from_empty',NOT EXISTS(SELECT 1 FROM ledger_history)
        AND NOT EXISTS(SELECT 1 FROM pending_balance)
        AND NOT EXISTS(SELECT 1 FROM block_found)
        AND NOT EXISTS(SELECT 1 FROM slice),
    'external_checkpoint_required',EXISTS(SELECT 1 FROM ledger_history)
        OR EXISTS(SELECT 1 FROM pending_balance)
        OR EXISTS(SELECT 1 FROM block_found)
        OR EXISTS(SELECT 1 FROM slice));

CREATE FUNCTION spc_record_accounting_history() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO source_accounting_history(table_name,operation,before_row,after_row)
    VALUES(TG_TABLE_NAME,TG_OP,
        CASE WHEN TG_OP='INSERT' THEN NULL ELSE to_jsonb(OLD)
            - ARRAY['destination','order_bytes','order_digest','grant_bytes','vtxo_id','resolution_proof'] END,
        CASE WHEN TG_OP='DELETE' THEN NULL ELSE to_jsonb(NEW)
            - ARRAY['destination','order_bytes','order_digest','grant_bytes','vtxo_id','resolution_proof'] END);
    RETURN NULL;
END $$;

CREATE TRIGGER source_history_ledger AFTER INSERT OR UPDATE OR DELETE ON ledger_history
FOR EACH ROW EXECUTE FUNCTION spc_record_accounting_history();
CREATE TRIGGER source_history_pending AFTER INSERT OR UPDATE OR DELETE ON pending_balance
FOR EACH ROW EXECUTE FUNCTION spc_record_accounting_history();
CREATE TRIGGER source_history_block AFTER INSERT OR UPDATE OR DELETE ON block_found
FOR EACH ROW EXECUTE FUNCTION spc_record_accounting_history();
CREATE TRIGGER source_history_ark AFTER INSERT OR UPDATE OR DELETE ON ark_payout
FOR EACH ROW EXECUTE FUNCTION spc_record_accounting_history();
CREATE TRIGGER source_history_aggregate AFTER INSERT OR UPDATE OR DELETE ON ark_aggregate
FOR EACH ROW EXECUTE FUNCTION spc_record_accounting_history();
CREATE TRIGGER source_history_member AFTER INSERT OR UPDATE OR DELETE ON ark_aggregate_member
FOR EACH ROW EXECUTE FUNCTION spc_record_accounting_history();

CREATE FUNCTION spc_history_append_only() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN RAISE EXCEPTION 'source accounting history is append-only'; END $$;
CREATE TRIGGER source_history_immutable BEFORE UPDATE OR DELETE ON source_accounting_history
FOR EACH ROW EXECUTE FUNCTION spc_history_append_only();
