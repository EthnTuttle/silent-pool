-- SPc/payout-only decision evidence. Never an Ark-role identity feed.
-- This starts observation at migration time; it does not backfill past reads.
CREATE TABLE source_decision_observation (
    observation_id BIGSERIAL PRIMARY KEY CHECK (observation_id>0),
    schema_version SMALLINT NOT NULL DEFAULT 1 CHECK (schema_version = 1),
    kind TEXT NOT NULL CHECK (kind IN ('settlement_delivery_read','settlement_reconstruction',
        'front_derivation','authorization_decision','authorization_row','authorization_consumption')),
    subject BYTEA NOT NULL CHECK (octet_length(subject) IN (16,32)),
    attempt BYTEA CHECK (attempt IS NULL OR octet_length(attempt)=16),
    transaction_id BIGINT NOT NULL DEFAULT txid_current(),
    snapshot_boundary TEXT NOT NULL DEFAULT txid_current_snapshot()::text,
    observed_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    body JSONB NOT NULL CHECK (jsonb_typeof(body)='object' AND octet_length(body::text)<=8388608),
    UNIQUE(kind,subject,attempt)
);
CREATE TRIGGER source_decision_immutable BEFORE UPDATE OR DELETE ON source_decision_observation
FOR EACH ROW EXECUTE FUNCTION spc_history_append_only();
CREATE TRIGGER source_history_decision AFTER INSERT ON source_decision_observation
FOR EACH ROW EXECUTE FUNCTION spc_record_accounting_history();
ALTER TABLE economic_block ADD COLUMN reconstruction_observation BIGINT
    REFERENCES source_decision_observation(observation_id);
CREATE FUNCTION spc_reconstruction_reference_immutable() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.reconstruction_observation IS DISTINCT FROM OLD.reconstruction_observation THEN
        RAISE EXCEPTION 'immutable reconstruction observation reference';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER economic_reconstruction_immutable BEFORE UPDATE ON economic_block
FOR EACH ROW EXECUTE FUNCTION spc_reconstruction_reference_immutable();

-- Retain complete signed grants even when the working authorization row is
-- scrubbed. The generic history trigger intentionally redacts grant_bytes,
-- hence this private, versioned full-row observation instead.
CREATE FUNCTION spc_observe_authorization_row() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO source_decision_observation(kind,subject,body)
    VALUES('authorization_row',CASE WHEN TG_OP='DELETE' THEN OLD.order_id ELSE NEW.order_id END,
        jsonb_build_object('schema','spc.authorization-row/1','operation',TG_OP,
            'before',CASE WHEN TG_OP='INSERT' THEN NULL ELSE to_jsonb(OLD) END,
            'after',CASE WHEN TG_OP='DELETE' THEN NULL ELSE to_jsonb(NEW) END));
    RETURN NULL;
END $$;
CREATE TRIGGER source_history_authorization AFTER INSERT OR UPDATE OR DELETE ON ark_send_authorization
FOR EACH ROW EXECUTE FUNCTION spc_observe_authorization_row();

-- Public registration inputs only. Existing rows have no invented pre-0012
-- ancestry; exact decision-read receipts supply their actual visibility.
CREATE TRIGGER source_history_delivery AFTER INSERT OR UPDATE OR DELETE ON ark_delivery
FOR EACH ROW EXECUTE FUNCTION spc_record_accounting_history();
CREATE TRIGGER source_history_registration_state AFTER INSERT OR UPDATE OR DELETE ON ark_registration_state
FOR EACH ROW EXECUTE FUNCTION spc_record_accounting_history();
CREATE TRIGGER source_history_registration_request AFTER INSERT OR UPDATE OR DELETE ON ark_registration_request
FOR EACH ROW EXECUTE FUNCTION spc_record_accounting_history();
CREATE TRIGGER source_history_epoch_mailbox AFTER INSERT OR UPDATE OR DELETE ON ark_epoch_mailbox
FOR EACH ROW EXECUTE FUNCTION spc_record_accounting_history();
