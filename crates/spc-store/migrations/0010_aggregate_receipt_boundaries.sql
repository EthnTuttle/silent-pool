ALTER TABLE ark_aggregate ADD COLUMN server_pubkey BYTEA CHECK(octet_length(server_pubkey)=33);
CREATE FUNCTION spc_frozen_aggregate_server() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.server_pubkey IS DISTINCT FROM OLD.server_pubkey THEN RAISE EXCEPTION 'immutable aggregate server'; END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER aggregate_server_immutable BEFORE UPDATE ON ark_aggregate FOR EACH ROW EXECUTE FUNCTION spc_frozen_aggregate_server();

CREATE TABLE ark_receipt_disposition (
    stream TEXT NOT NULL, stream_id TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('terminal_invalid','retry_unknown','applied')),
    reason TEXT NOT NULL,
    payload BYTEA NOT NULL CHECK(octet_length(payload)<=4194304),
    payload_sha256 BYTEA NOT NULL CHECK(octet_length(payload_sha256)=32),
    original_len BIGINT NOT NULL,
    attempts BIGINT NOT NULL DEFAULT 1,
    PRIMARY KEY(stream,stream_id)
);
CREATE TABLE ark_receipt_cursor (stream TEXT PRIMARY KEY,last_id TEXT NOT NULL);
