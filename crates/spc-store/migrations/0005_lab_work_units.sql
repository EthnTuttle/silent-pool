-- LP-1 versioned work units and network-block window rule.
-- Existing rows retain their exact v1 meaning; no conversion is performed.
ALTER TABLE slice
    ADD COLUMN work_unit_version SMALLINT NOT NULL DEFAULT 1,
    ADD COLUMN work_unit_fingerprint BYTEA;

ALTER TABLE distribution_snapshot
    ADD COLUMN work_unit_version SMALLINT NOT NULL DEFAULT 1,
    ADD COLUMN work_unit_fingerprint BYTEA,
    ADD COLUMN window_rule SMALLINT NOT NULL DEFAULT 0;

ALTER TABLE slice
    ADD CONSTRAINT slice_work_unit_version_check CHECK (work_unit_version IN (1, 2)),
    ADD CONSTRAINT slice_work_unit_fingerprint_check
        CHECK ((work_unit_version = 1) = (work_unit_fingerprint IS NULL)),
    ADD CONSTRAINT slice_work_unit_fingerprint_shape_check
        CHECK (work_unit_fingerprint IS NULL OR (
            octet_length(work_unit_fingerprint) = 32
            AND work_unit_fingerprint <> decode(repeat('00', 32), 'hex')
        ));

ALTER TABLE distribution_snapshot
    ADD CONSTRAINT distribution_work_unit_version_check CHECK (work_unit_version IN (1, 2)),
    ADD CONSTRAINT distribution_work_unit_fingerprint_check
        CHECK ((work_unit_version = 1) = (work_unit_fingerprint IS NULL)),
    ADD CONSTRAINT distribution_work_unit_fingerprint_shape_check
        CHECK (work_unit_fingerprint IS NULL OR (
            octet_length(work_unit_fingerprint) = 32
            AND work_unit_fingerprint <> decode(repeat('00', 32), 'hex')
        )),
    ADD CONSTRAINT distribution_window_rule_check CHECK (window_rule IN (0, 1)),
    ADD CONSTRAINT distribution_version_window_rule_check
        CHECK ((work_unit_version = 1 AND window_rule = 0) OR (work_unit_version = 2 AND window_rule = 1));
