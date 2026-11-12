-- Compatibility for 0009's pre-trigger initial singleton INSERT. Never rewrite
-- applied migrations or synthesize balances. A nonempty old database remains
-- unknown until its history can be independently proved.
SELECT pg_advisory_xact_lock(1397769025);
LOCK TABLE economic_state, pending_balance, economic_block, economic_chain,
    block_found, ledger_history, ark_payout IN SHARE MODE;

CREATE TABLE economic_checkpoint (
    checkpoint_id SMALLINT PRIMARY KEY CHECK(checkpoint_id=1),
    schema_version SMALLINT NOT NULL CHECK(schema_version=1),
    migration_version BIGINT NOT NULL CHECK(migration_version=11),
    classification TEXT NOT NULL CHECK(classification IN ('fresh_empty','unverified_existing')),
    economic_revision BIGINT NOT NULL,
    history_watermark BIGINT NOT NULL,
    transaction_id BIGINT NOT NULL,
    snapshot_boundary TEXT NOT NULL,
    state_bytes BYTEA NOT NULL CHECK(octet_length(state_bytes)<=4096),
    snapshot_bytes BYTEA NOT NULL CHECK(octet_length(snapshot_bytes)<=8388608),
    snapshot_sha256 BYTEA NOT NULL CHECK(octet_length(snapshot_sha256)=32),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

WITH observed AS (
    SELECT s.*, COALESCE((SELECT max(event_id) FROM source_accounting_history),0) AS watermark,
        s.revision=0 AND s.anchor_height=0 AND s.anchor_hash IS NULL AND NOT s.complete
        AND s.reason='unreconciled'
        AND (SELECT count(*) FROM source_accounting_history)=1
        AND EXISTS(SELECT 1 FROM source_accounting_history WHERE table_name='checkpoint' AND operation='history_start'
            AND after_row->>'schema'='spc.accounting-history/1' AND after_row->>'complete_from_empty'='true'
            AND after_row->>'external_checkpoint_required'='false')
        AND NOT EXISTS(SELECT 1 FROM pending_balance)
        AND NOT EXISTS(SELECT 1 FROM economic_block)
        AND NOT EXISTS(SELECT 1 FROM economic_chain)
        AND NOT EXISTS(SELECT 1 FROM block_found)
        AND NOT EXISTS(SELECT 1 FROM ledger_history)
        AND NOT EXISTS(SELECT 1 FROM ark_payout) AS fresh,
        to_jsonb(s) AS state
    FROM economic_state s WHERE id=1
), captured AS (
    SELECT observed.*, jsonb_build_object(
        'schema','spc.economic-checkpoint/1','migration_version',11,
        'complete_from_empty',fresh,'state',state,'row_limit',1025,
        'history_start',(SELECT jsonb_build_object('event_id',event_id,'table_name',table_name,'operation',operation,'after_row',after_row)
            FROM source_accounting_history WHERE operation='history_start' ORDER BY event_id LIMIT 1),
        'booked_balances',COALESCE((SELECT jsonb_agg(to_jsonb(r)) FROM (SELECT * FROM pending_balance ORDER BY sp_id LIMIT 1025) r),'[]'::jsonb),
        'economic_blocks',COALESCE((SELECT jsonb_agg(to_jsonb(r)) FROM (SELECT block_hash,parent_hash,height,payload_digest,lifecycle,transition,revision FROM economic_block ORDER BY height,block_hash LIMIT 1025) r),'[]'::jsonb),
        'chain',COALESCE((SELECT jsonb_agg(to_jsonb(r)) FROM (SELECT * FROM economic_chain ORDER BY height LIMIT 1025) r),'[]'::jsonb),
        'found',COALESCE((SELECT jsonb_agg(to_jsonb(r)) FROM (SELECT * FROM block_found ORDER BY height,block_hash LIMIT 1025) r),'[]'::jsonb),
        'ledger',COALESCE((SELECT jsonb_agg(to_jsonb(r)) FROM (SELECT * FROM ledger_history ORDER BY block_hash,sp_id LIMIT 1025) r),'[]'::jsonb),
        'ark_payouts',COALESCE((SELECT jsonb_agg(to_jsonb(r)-'vtxo_id') FROM (SELECT * FROM ark_payout ORDER BY block_hash,sp_id LIMIT 1025) r),'[]'::jsonb),
        'prior_migrations',COALESCE((SELECT jsonb_agg(jsonb_build_object('version',version,'checksum',encode(checksum,'hex')) ORDER BY version) FROM _sqlx_migrations WHERE success),'[]'::jsonb)
    ) AS snapshot FROM observed
)
INSERT INTO economic_checkpoint(checkpoint_id,schema_version,migration_version,classification,economic_revision,history_watermark,
    transaction_id,snapshot_boundary,state_bytes,snapshot_bytes,snapshot_sha256)
SELECT 1,1,11,CASE WHEN fresh THEN 'fresh_empty' ELSE 'unverified_existing' END,revision,watermark,
    txid_current(),txid_current_snapshot()::text,convert_to(state::text,'UTF8'),convert_to(snapshot::text,'UTF8'),sha256(convert_to(snapshot::text,'UTF8'))
FROM captured;

-- This is a real checkpoint INSERT, not a fabricated historical INSERT into
-- economic_state. Its raw byte strings are the exact initial-row/snapshot
-- serialization and its watermark denotes the preceding committed history.
INSERT INTO source_accounting_history(table_name,operation,before_row,after_row)
SELECT 'economic_checkpoint','INSERT',NULL,to_jsonb(c) FROM economic_checkpoint c;
CREATE INDEX source_history_economic_checkpoint ON source_accounting_history(event_id) WHERE table_name='economic_checkpoint';

UPDATE economic_state SET complete=false,reason='economic initial history checkpoint unverified'
WHERE EXISTS(SELECT 1 FROM economic_checkpoint WHERE classification='unverified_existing');
