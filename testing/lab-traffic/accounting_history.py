"""Replay actual migration-0008 committed row transitions, SPc-private only.

This checks history consistency at the collected SQL snapshot. It does not
establish a past job's economic watermark or RULEB reservation completeness.
The caller must authenticate the capture independently of its contents.
"""
from copy import deepcopy

from oracle import Inconclusive, integer, require
from storage_queries import TABLES

REDACTED = {"destination", "order_bytes", "order_digest", "grant_bytes", "vtxo_id", "resolution_proof"}
KEYS = {"ledger_history": ("block_hash", "sp_id"), "pending_balance": ("sp_id",),
        "block_found": ("block_hash",), "ark_payout": ("block_hash", "sp_id"),
        "ark_aggregate": ("order_id",), "ark_aggregate_member": ("order_id", "ordinal")}


def projection(table, row, definitions=TABLES, redacted=REDACTED):
    require(isinstance(row, dict), "history row must be an object")
    expected = set(definitions[table][0].split()) - redacted
    projected = {key: value for key, value in row.items() if key not in redacted}
    if set(projected) != expected:
        raise Inconclusive("history producer row columns missing/unknown: " + table)
    return projected


def row_key(table, row, keys=KEYS):
    key = tuple(row[name] for name in keys[table])
    require(all(isinstance(v, (str, int)) and not isinstance(v, bool) for v in key), "history primary key shape")
    return key


def replay(observation):
    return _replay(observation, TABLES, KEYS, REDACTED, "spc-lab-sql-observation/2")


def _replay(observation, definitions, keys, redacted, schema, initial_state=None):
    require(observation["schema"] == schema, "history SQL observation version")
    rows = observation["tables"]["source_accounting_history"]
    if not rows:
        raise Inconclusive("accounting history start missing")
    boundary = observation["history_boundary"]["event_watermark"]
    ids = [integer(row["event_id"], 1) for row in rows]
    require(ids == sorted(set(ids)), "history event order/duplicates")
    if boundary != ids[-1]:
        raise Inconclusive("accounting history does not reach collected watermark")
    # BIGSERIAL gaps are legal after rollback. Snapshot visibility, not a
    # contiguous event-ID sequence, determines committed transaction membership.
    parts = observation["server"]["snapshot"].split(":")
    require(len(parts) == 3, "history transaction snapshot shape")
    xmin, xmax = integer(int(parts[0])), integer(int(parts[1]))
    active = {integer(int(v)) for v in parts[2].split(",") if v}
    require(xmin <= xmax and all(xmin <= v < xmax for v in active), "history transaction snapshot bounds")
    first = rows[0]
    checkpoint = first["after_row"]
    if first["operation"] != "history_start" or first["table_name"] != "checkpoint":
        raise Inconclusive("accounting history prefix missing")
    require(first["before_row"] is None and isinstance(checkpoint, dict)
            and checkpoint.get("schema") == "spc.accounting-history/1", "history start schema")
    if checkpoint.get("complete_from_empty") is not True or checkpoint.get("external_checkpoint_required") is not False:
        raise Inconclusive("history requires an independently verified external checkpoint")
    state = {table: {} for table in keys}
    if initial_state is not None:
        from economic_checkpoint import InitialCheckpoint
        require(isinstance(initial_state, InitialCheckpoint), "initial rows require a validated captured checkpoint")
        state.update(initial_state.replay_seed())
    transactions = {}
    for index, event in enumerate(rows):
        xid = integer(event["transaction_id"], 1)
        require(xid < xmax and xid not in active, "history transaction not visible in captured snapshot")
        transactions.setdefault(xid, []).append(event["event_id"])
        if index == 0:
            continue
        table, op = event["table_name"], event["operation"]
        if table not in keys:
            raise Inconclusive("unrecognized accounting history producer table: " + table)
        require(op in {"INSERT", "UPDATE", "DELETE"}, "unexpected history operation/repeated start")
        before, after = event["before_row"], event["after_row"]
        require((before is None) == (op == "INSERT") and (after is None) == (op == "DELETE"), "history transition null sides")
        for row in (before, after):
            if row is not None:
                require(not (set(row) & redacted), "history contains producer-redacted fields")
                projection(table, row, definitions, redacted)
        if before is not None:
            key = row_key(table, before, keys)
            if key not in state[table]:
                raise Inconclusive("history before-image has no retained ancestry")
            require(state[table][key] == before, "history before-image differs from replay")
            del state[table][key]
        if after is not None:
            key = row_key(table, after, keys)
            require(key not in state[table], "history duplicate live primary key")
            state[table][key] = deepcopy(after)
    for table in keys:
        actual = {}
        for row in observation["tables"][table]:
            row = projection(table, row, definitions, redacted)
            key = row_key(table, row, keys)
            require(key not in actual, "snapshot duplicate live primary key")
            actual[key] = row
        if state[table] != actual:
            raise Inconclusive("history replay differs from same-snapshot rows: " + table)
    return {"scope": "committed_accounting_row_replay_only", "event_watermark": boundary,
            "events": len(rows), "transactions": transactions, "state": state,
            "reservation_history_complete": False, "past_job_pending_verified": False}
