-- Rule B, SPc-side only. No fields from these tables enter Ark streams.
CREATE TABLE economic_state (
    id SMALLINT PRIMARY KEY CHECK (id=1),
    revision BIGINT NOT NULL DEFAULT 0 CHECK (revision>=0),
    anchor_height INTEGER NOT NULL DEFAULT 0 CHECK (anchor_height>=0),
    anchor_hash BYTEA CHECK (octet_length(anchor_hash)=32),
    complete BOOLEAN NOT NULL DEFAULT false,
    reason TEXT NOT NULL DEFAULT 'unreconciled'
);
INSERT INTO economic_state(id) VALUES(1);
CREATE TABLE economic_block (
    block_hash BYTEA PRIMARY KEY CHECK(octet_length(block_hash)=32),
    parent_hash BYTEA NOT NULL CHECK(octet_length(parent_hash)=32),
    height INTEGER NOT NULL CHECK(height>=0),
    payload_digest BYTEA NOT NULL CHECK(octet_length(payload_digest)=32),
    raw_event BYTEA,
    frozen_job JSONB NOT NULL,
    validated_ledger JSONB NOT NULL,
    consumption JSONB NOT NULL,
    lifecycle TEXT NOT NULL CHECK(lifecycle IN ('unbooked','booked','cancelled','loss_held')),
    transition BIGINT NOT NULL DEFAULT 1 CHECK(transition>0),
    revision BIGINT NOT NULL CHECK(revision>=0)
);
CREATE TABLE economic_chain (
    height INTEGER PRIMARY KEY CHECK(height>=0),
    block_hash BYTEA UNIQUE NOT NULL CHECK(octet_length(block_hash)=32),
    parent_hash BYTEA NOT NULL CHECK(octet_length(parent_hash)=32),
    pool_block BOOLEAN NOT NULL,
    coinbase_digest BYTEA NOT NULL CHECK(octet_length(coinbase_digest)=32),
    revision BIGINT NOT NULL CHECK(revision>=0)
);
CREATE INDEX economic_unbooked ON economic_block(height) WHERE lifecycle='unbooked';
CREATE TRIGGER history_economic_state AFTER INSERT OR UPDATE OR DELETE ON economic_state
FOR EACH ROW EXECUTE FUNCTION spc_record_accounting_history();
CREATE TRIGGER history_economic_block AFTER INSERT OR UPDATE OR DELETE ON economic_block
FOR EACH ROW EXECUTE FUNCTION spc_record_accounting_history();
CREATE TRIGGER history_economic_chain AFTER INSERT OR UPDATE OR DELETE ON economic_chain
FOR EACH ROW EXECUTE FUNCTION spc_record_accounting_history();

-- Raw events contain scrub-sensitive output/destination attribution. Permanent
-- history retains only the frozen monetary inputs, deltas and lifecycle.
CREATE OR REPLACE FUNCTION spc_record_accounting_history() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO source_accounting_history(table_name,operation,before_row,after_row)
    VALUES(TG_TABLE_NAME,TG_OP,
        CASE WHEN TG_OP='INSERT' THEN NULL ELSE to_jsonb(OLD)
            - ARRAY['destination','order_bytes','order_digest','grant_bytes','vtxo_id','resolution_proof','raw_event'] END,
        CASE WHEN TG_OP='DELETE' THEN NULL ELSE to_jsonb(NEW)
            - ARRAY['destination','order_bytes','order_digest','grant_bytes','vtxo_id','resolution_proof','raw_event'] END);
    RETURN NULL;
END $$;
