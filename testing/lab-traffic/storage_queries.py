"""Closed, source-owned, read-only PostgreSQL observations (SPEC 5.1).

No caller SQL, identifiers or predicates are accepted. Explicit columns avoid
exporting a new secret column added by a later migration. Bytea fields use
PostgreSQL's lossless JSON bytea representation; NUMERIC is decoded exactly.
"""
import hashlib

QUERY_ID = "spc-lab-immutable-payout-rows/2"
ROW_CAP = 65536
# table: (explicit columns, stable primary-key ordering)
TABLES = {
    "block_found": ("block_hash height fingerprint status found_at settled_at ledger_published", "block_hash"),
    "distribution_snapshot": ("fingerprint height a_send_x distribution_id entries high_water window_target work_unit_version work_unit_fingerprint window_rule created_at", "fingerprint"),
    "slice": ("slice_id height prev_hash ref_job_id ref_fees n_shares difficulty root shares work_unit_version work_unit_fingerprint sealed_at", "slice_id"),
    "slice_credit": ("slice_id sp_id diff_sum diff_fee_sum", "slice_id sp_id"),
    "ledger_history": ("block_hash sp_id rail paid_sats ark_due_sats forfeited_sats pending_delta", "block_hash sp_id"),
    "pending_balance": ("sp_id sats updated_at", "sp_id"),
    "ark_delivery": ("sp_id ark_id b_ark mailbox_mode mailbox_m mailbox_d epoch_fallback updated_at", "sp_id"),
    "ark_payout": ("block_hash sp_id sats due_height status vtxo_id expiry_height", "block_hash sp_id"),
    "ark_aggregate": ("batch_seq order_id sp_id anchor_hash anchor_height destination sats not_before_height selected_tip selected_tip_hash order_bytes order_digest state revision vtxo_id expiry_height pool_loss_sats resolution_proof created_at", "batch_seq"),
    "ark_aggregate_member": ("order_id ordinal block_hash sp_id block_height due_height sats reserved orphaned", "order_id ordinal"),
    "ark_send_authorization": ("order_id nonce revision order_digest checked_tip checked_tip_hash expires_at grant_bytes created_at", "order_id"),
    "source_accounting_history": ("event_id transaction_id table_name operation before_row after_row recorded_at", "event_id"),
}


def _identifiers(words):
    return ", ".join('"' + word + '"' for word in words.split())


def query():
    return _render(TABLES, QUERY_ID, "spc-lab-sql-observation/2")


def _render(definitions, query_id, schema):
    tables = []
    counts = []
    for name, (columns, order) in definitions.items():
        tables.append("'" + name + "', (SELECT coalesce(json_agg(t), '[]'::json) FROM "
                      "(SELECT " + _identifiers(columns) + ' FROM public."' + name + '" ORDER BY '
                      + _identifiers(order) + " LIMIT " + str(ROW_CAP + 1) + ") AS t)")
        counts.append("'" + name + "', (SELECT count(*) FROM public.\"" + name + '\")')
    return ("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;\n"
            "SET LOCAL statement_timeout = '10000ms';\n"
            "SET LOCAL lock_timeout = '2000ms';\n"
            "SET LOCAL idle_in_transaction_session_timeout = '12000ms';\n"
            "SET LOCAL search_path = pg_catalog;\n"
            "SET LOCAL bytea_output = 'hex';\n"
            "SELECT json_build_object(\n"
            "'schema', '" + schema + "',\n"
            "'query_id', '" + query_id + "',\n"
            "'server', json_build_object(\n"
            " 'database', current_database(), 'role', current_user,\n"
            " 'port', current_setting('port')::integer,\n"
            " 'data_directory', current_setting('data_directory'),\n"
            " 'unix_socket_directories', current_setting('unix_socket_directories'),\n"
            " 'listen_addresses', current_setting('listen_addresses'),\n"
            " 'server_addr', host(inet_server_addr()), 'server_port', inet_server_port(),\n"
            " 'postmaster_start_epoch', extract(epoch FROM pg_postmaster_start_time())::text,\n"
            " 'system_identifier', (pg_control_system()).system_identifier::text,\n"
            " 'transaction_read_only', current_setting('transaction_read_only'),\n"
            " 'transaction_isolation', current_setting('transaction_isolation'),\n"
            " 'backend_pid', pg_backend_pid(), 'snapshot', pg_current_snapshot()::text),\n"
            "'history_boundary', json_build_object('wal_lsn', pg_current_wal_lsn()::text,\n"
            " 'event_watermark', (SELECT max(event_id) FROM public.source_accounting_history)),\n"
            "'counts', json_build_object(" + ",\n".join(counts) + "),\n"
            "'tables', json_build_object(" + ",\n".join(tables) + "));\nROLLBACK;\n")


QUERY_SHA256 = hashlib.sha256(query().encode()).hexdigest()
