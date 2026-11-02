-- SPEC §5.1. Written only by the `payout` role.
-- No per-share rows: slices are the aggregation unit.

CREATE TABLE sp_identity (
    sp_id       BYTEA PRIMARY KEY,
    b_scan      BYTEA NOT NULL CHECK (octet_length(b_scan) = 33),
    b_spend     BYTEA NOT NULL CHECK (octet_length(b_spend) = 33),
    network     SMALLINT NOT NULL, -- 0 mainnet, 1 testnet4, 2 signet, 3 regtest
    first_seen  TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen   TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- A delivery capability, not identity. At most one per sp_id.
CREATE TABLE ark_delivery (
    sp_id        BYTEA PRIMARY KEY REFERENCES sp_identity,
    ark_id       BYTEA NOT NULL,
    b_ark        BYTEA NOT NULL CHECK (octet_length(b_ark) = 33),
    mailbox_mode TEXT NOT NULL
        CHECK (mailbox_mode IN ('none', 'static', 'epoch_derived', 'epoch_registered')),
    mailbox_m    BYTEA,
    mailbox_d    BYTEA,
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK ((mailbox_mode = 'none') = (mailbox_m IS NULL AND mailbox_d IS NULL))
    -- b_ark != b_spend is enforced by the registration handler (SPEC §11.4),
    -- not by a CHECK: Postgres CHECK constraints cannot reference sp_identity.
);

-- MAILBOX.md variant A only.
CREATE TABLE ark_epoch_mailbox (
    sp_id      BYTEA REFERENCES ark_delivery ON DELETE CASCADE,
    height     INTEGER NOT NULL,
    mailbox_mi BYTEA NOT NULL,
    PRIMARY KEY (sp_id, height)
);

CREATE TABLE slice (
    slice_id    BIGINT PRIMARY KEY,
    height      INTEGER NOT NULL,
    prev_hash   BYTEA NOT NULL,
    ref_job_id  BIGINT NOT NULL,
    ref_fees    BIGINT NOT NULL,
    n_shares    INTEGER NOT NULL,
    difficulty  NUMERIC(40, 0) NOT NULL,
    root        BYTEA NOT NULL,
    shares      BYTEA NOT NULL,
    sealed_at   TIMESTAMPTZ NOT NULL
);
CREATE INDEX slice_height_idx ON slice (height DESC, slice_id DESC);

-- The only share-to-identity link; the scrub target.
CREATE TABLE slice_credit (
    slice_id     BIGINT REFERENCES slice ON DELETE CASCADE,
    sp_id        BYTEA REFERENCES sp_identity,
    diff_sum     NUMERIC(40, 0) NOT NULL,
    diff_fee_sum NUMERIC(60, 0) NOT NULL,
    PRIMARY KEY (slice_id, sp_id)
);

CREATE TABLE distribution_snapshot (
    fingerprint     BYTEA PRIMARY KEY,
    height          INTEGER NOT NULL,
    a_send_x        BYTEA NOT NULL,
    distribution_id BIGINT,
    entries         JSONB NOT NULL,
    high_water      JSONB NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE block_found (
    block_hash  BYTEA PRIMARY KEY,
    height      INTEGER NOT NULL,
    fingerprint BYTEA REFERENCES distribution_snapshot,
    status      TEXT NOT NULL CHECK (status IN ('pending', 'confirmed', 'orphaned')),
    found_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    settled_at  TIMESTAMPTZ,
    CHECK (status <> 'confirmed' OR fingerprint IS NOT NULL)
);

-- Idempotency and accounting; never a script or key.
CREATE TABLE ledger_history (
    block_hash    BYTEA REFERENCES block_found,
    sp_id         BYTEA REFERENCES sp_identity,
    rail          TEXT NOT NULL CHECK (rail IN ('coinbase', 'ark', 'pending', 'forfeit')),
    paid_sats     BIGINT NOT NULL,
    ark_due_sats  BIGINT NOT NULL DEFAULT 0,
    forfeited_sats BIGINT NOT NULL DEFAULT 0,
    pending_delta BIGINT NOT NULL,
    PRIMARY KEY (block_hash, sp_id)
);

CREATE TABLE ark_payout (
    block_hash    BYTEA,
    sp_id         BYTEA,
    sats          BIGINT NOT NULL,
    due_height    INTEGER NOT NULL,
    status        TEXT NOT NULL CHECK (status IN ('scheduled', 'sent', 'failed_to_pending')),
    vtxo_id       BYTEA,
    expiry_height INTEGER,
    PRIMARY KEY (block_hash, sp_id),
    FOREIGN KEY (block_hash, sp_id) REFERENCES ledger_history
);

CREATE TABLE pending_balance (
    sp_id      BYTEA PRIMARY KEY REFERENCES sp_identity,
    sats       BIGINT NOT NULL CHECK (sats >= 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
