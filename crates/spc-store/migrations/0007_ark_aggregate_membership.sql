-- SPc-side only. None of these tables is available to the Ark role.
CREATE TABLE ark_aggregate (
    batch_seq BIGSERIAL UNIQUE NOT NULL,
    order_id BYTEA PRIMARY KEY CHECK (octet_length(order_id) = 16),
    sp_id BYTEA NOT NULL REFERENCES sp_identity,
    anchor_hash BYTEA NOT NULL CHECK (octet_length(anchor_hash) = 32),
    anchor_height INTEGER NOT NULL CHECK (anchor_height >= 0),
    destination BYTEA NOT NULL CHECK (octet_length(destination) BETWEEN 1 AND 65535),
    sats BIGINT NOT NULL CHECK (sats > 0),
    not_before_height INTEGER NOT NULL CHECK (not_before_height >= 0),
    selected_tip INTEGER NOT NULL CHECK (selected_tip >= not_before_height),
    selected_tip_hash BYTEA NOT NULL CHECK (octet_length(selected_tip_hash) = 32),
    order_bytes BYTEA NOT NULL,
    order_digest BYTEA NOT NULL CHECK (octet_length(order_digest) = 32),
    state TEXT NOT NULL CHECK (state IN ('frozen','published','authorized','blocked_unknown','sent','cancelled')),
    revision BIGINT NOT NULL DEFAULT 1 CHECK (revision > 0),
    vtxo_id BYTEA,
    expiry_height INTEGER,
    pool_loss_sats BIGINT NOT NULL DEFAULT 0 CHECK (pool_loss_sats >= 0 AND pool_loss_sats <= sats),
    resolution_proof BYTEA,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK ((state = 'sent') = (vtxo_id IS NOT NULL AND expiry_height IS NOT NULL)),
    CHECK (state NOT IN ('sent','cancelled') OR resolution_proof IS NOT NULL)
);

CREATE TABLE ark_aggregate_member (
    order_id BYTEA NOT NULL REFERENCES ark_aggregate,
    ordinal INTEGER NOT NULL CHECK (ordinal BETWEEN 0 AND 2047),
    block_hash BYTEA NOT NULL CHECK (octet_length(block_hash) = 32),
    sp_id BYTEA NOT NULL REFERENCES sp_identity,
    block_height INTEGER NOT NULL CHECK (block_height >= 0),
    due_height INTEGER NOT NULL CHECK (due_height >= block_height),
    sats BIGINT NOT NULL CHECK (sats > 0),
    reserved BOOLEAN NOT NULL DEFAULT true,
    orphaned BOOLEAN NOT NULL DEFAULT false,
    PRIMARY KEY (order_id, ordinal),
    UNIQUE (order_id, block_hash, sp_id)
    -- Deliberately no FK to ark_payout: cancelled/orphaned membership tombstones
    -- must survive normal orphan reversal deleting the original payout row.
);
CREATE UNIQUE INDEX ark_aggregate_one_reservation ON ark_aggregate_member(block_hash, sp_id) WHERE reserved;
CREATE INDEX ark_aggregate_identity ON ark_aggregate(sp_id, batch_seq);

CREATE TABLE ark_send_authorization (
    order_id BYTEA PRIMARY KEY REFERENCES ark_aggregate,
    nonce BYTEA UNIQUE NOT NULL CHECK (octet_length(nonce) = 16),
    revision BIGINT NOT NULL CHECK (revision > 0),
    order_digest BYTEA NOT NULL CHECK (octet_length(order_digest) = 32),
    checked_tip INTEGER NOT NULL CHECK (checked_tip >= 0),
    checked_tip_hash BYTEA NOT NULL CHECK (octet_length(checked_tip_hash) = 32),
    expires_at BIGINT NOT NULL CHECK (expires_at > 0),
    grant_bytes BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- An emitted/potentially authorized record's frozen fields are immutable even
-- to another payout caller. State, receipt and loss-risk fields may advance.
CREATE FUNCTION spc_aggregate_frozen_fields() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF ROW(NEW.batch_seq, NEW.order_id, NEW.sp_id, NEW.anchor_hash, NEW.anchor_height,
           NEW.destination, NEW.sats, NEW.not_before_height, NEW.selected_tip,
           NEW.selected_tip_hash, NEW.order_bytes, NEW.order_digest)
       IS DISTINCT FROM
       ROW(OLD.batch_seq, OLD.order_id, OLD.sp_id, OLD.anchor_hash, OLD.anchor_height,
           OLD.destination, OLD.sats, OLD.not_before_height, OLD.selected_tip,
           OLD.selected_tip_hash, OLD.order_bytes, OLD.order_digest) THEN
        RAISE EXCEPTION 'immutable aggregate binding';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER ark_aggregate_immutable BEFORE UPDATE ON ark_aggregate
FOR EACH ROW EXECUTE FUNCTION spc_aggregate_frozen_fields();

CREATE FUNCTION spc_aggregate_member_frozen_fields() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF ROW(NEW.order_id,NEW.ordinal,NEW.block_hash,NEW.sp_id,NEW.block_height,NEW.due_height,NEW.sats)
       IS DISTINCT FROM ROW(OLD.order_id,OLD.ordinal,OLD.block_hash,OLD.sp_id,OLD.block_height,OLD.due_height,OLD.sats) THEN
        RAISE EXCEPTION 'immutable aggregate member';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER ark_aggregate_member_immutable BEFORE UPDATE ON ark_aggregate_member
FOR EACH ROW EXECUTE FUNCTION spc_aggregate_member_frozen_fields();
