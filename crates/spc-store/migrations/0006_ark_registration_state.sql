-- Durable 0x5350 v1 registration correlation (SPEC section 11.4).
-- The Redis ack hash is a cache; these rows are authoritative.

ALTER TABLE ark_delivery
    ADD COLUMN epoch_fallback SMALLINT NOT NULL DEFAULT 0
        CHECK (epoch_fallback IN (0, 1));

CREATE TABLE ark_registration_state (
    sp_id                 BYTEA PRIMARY KEY REFERENCES sp_identity,
    last_request_id       BIGINT NOT NULL CHECK (last_request_id BETWEEN 1 AND 4294967295),
    signed_fields_digest  BYTEA NOT NULL CHECK (octet_length(signed_fields_digest) = 32),
    registration_rev      BIGINT NOT NULL CHECK (registration_rev > 0),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE ark_registration_request (
    sp_id                 BYTEA NOT NULL REFERENCES sp_identity,
    request_id            BIGINT NOT NULL CHECK (request_id BETWEEN 1 AND 4294967295),
    signed_fields_digest  BYTEA NOT NULL CHECK (octet_length(signed_fields_digest) = 32),
    outcome               TEXT NOT NULL CHECK (outcome IN ('applied','superseded','mismatch','stale','rejected')),
    registration_rev      BIGINT NOT NULL CHECK (registration_rev > 0),
    error_code            TEXT,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (sp_id, request_id)
);

CREATE INDEX ark_registration_request_state_idx
    ON ark_registration_request (sp_id, request_id DESC);
