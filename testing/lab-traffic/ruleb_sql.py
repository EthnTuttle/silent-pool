"""Closed working-r2 SQL collection shape and offline economic row replay.

Not selected by the live collector until a tested producer binding is supplied.
No supplied available-A value, reload hint or current database row replaces
the committed history and exact parent/revision checkpoint.
"""
from collections import defaultdict
import hashlib

import accounting_history
import storage_queries
from oracle import Inconclusive, integer, require
from ruleb_source import (API_SHA256, bootstrap_view, consumption, fields, frozen_job,
                         hash_array, u64, working_pin)

SCHEMA = "spc-lab-sql-observation/3-working-ruleb-r2"
QUERY_ID = "spc-lab-immutable-payout-rows/3-working-ruleb-r2"
TABLES = {**storage_queries.TABLES,
    "economic_state": ("id revision anchor_height anchor_hash complete reason", "id"),
    "economic_block": ("block_hash parent_hash height payload_digest raw_event frozen_job validated_ledger consumption lifecycle transition revision", "block_hash"),
    "economic_chain": ("height block_hash parent_hash pool_block coinbase_digest revision", "height"),
    "ark_aggregate": (storage_queries.TABLES["ark_aggregate"][0] + " server_pubkey", "batch_seq"),
    "ark_receipt_disposition": ("stream stream_id status reason payload payload_sha256 original_len attempts", "stream stream_id"),
    "ark_receipt_cursor": ("stream last_id", "stream")}
KEYS = {**accounting_history.KEYS, "economic_state": ("id",), "economic_block": ("block_hash",), "economic_chain": ("height",)}
REDACTED = accounting_history.REDACTED | {"raw_event"}


def query(*, api_sha256):
    working_pin(api_sha256)
    return storage_queries._render(TABLES, QUERY_ID, SCHEMA)


def sql_bytes(value, size=None):
    require(isinstance(value, str) and value.startswith("\\x"), "economic SQL bytea encoding")
    try:
        raw = bytes.fromhex(value[2:])
    except ValueError as error:
        raise Inconclusive("economic SQL bytes unavailable") from error
    require(value == "\\x" + raw.hex() and (size is None or len(raw) == size), "economic SQL bytea length/canonical encoding")
    return raw


def parse_block(row):
    extra=['reconstruction_observation'] if 'reconstruction_observation' in row else []
    fields(row, TABLES["economic_block"][0].split()+extra, "economic_block")
    if extra and row['reconstruction_observation'] is not None:integer(row['reconstruction_observation'],1)
    block = sql_bytes(row["block_hash"], 32)
    parent = sql_bytes(row["parent_hash"], 32)
    payload = sql_bytes(row["payload_digest"], 32)
    height = integer(row["height"], 0, (1 << 31) - 1)
    integer(row["revision"], 0, (1 << 63) - 1)
    integer(row["transition"], 1, (1 << 63) - 1)
    if row["lifecycle"] not in {"unbooked", "booked", "cancelled", "loss_held"}:
        raise Inconclusive("unsupported stored economic lifecycle")
    job = frozen_job(row["frozen_job"], height=height, parent_hash=parent.hex())
    require(job["revision"] < row["revision"], "stored block does not follow its frozen economic revision")
    consumed = consumption(row["consumption"], job)
    ledger = row["validated_ledger"]
    if not isinstance(ledger, list):
        raise Inconclusive("validated ledger rows missing")
    by_id = {}
    for item in ledger:
        fields(item, "sp_id rail paid_sats ark_due_sats forfeited_sats pending_delta".split(), "validated LedgerRow")
        identity = hash_array(item["sp_id"])
        require(identity not in by_id, "duplicate validated ledger identity")
        require(item["rail"] in {"Coinbase", "Ark", "Pending", "Forfeit"}, "stored LedgerRail serialization")
        for name in ("paid_sats", "ark_due_sats", "forfeited_sats"):
            u64(item[name])
        by_id[identity] = integer(item["pending_delta"], -(1 << 63), (1 << 63) - 1)
    require(by_id == {hash_array(item["sp_id"]): item["delta"] for item in consumed}, "consumption/immutable validated ledger deltas differ")
    if row["raw_event"] is not None:
        from ladder_wire import decode_spcb_working
        decoded = decode_spcb_working(sql_bytes(row["raw_event"]), api_sha256=API_SHA256)
        require(decoded["economic"] == job and decoded["block_hash"] == block[::-1].hex()
                and decoded["snapshot_key"] == payload.hex(), "economic raw event/digest/frozen job mismatch")
    return {"block_hash": list(block), "parent_hash": list(parent), "height": height, "payload_digest": list(payload),
            "job_id": job["job_id"], "generation": job["generation"], "lifecycle": row["lifecycle"],
            "transition": row["transition"], "revision": row["revision"], "rows": consumed}


def validate_observation(observation, definitions=TABLES, schema=SCHEMA, query_id=QUERY_ID):
    require(observation["schema"] == schema and observation["query_id"] == query_id, "working economic SQL schema/query")
    if set(observation["tables"]) != set(definitions) or set(observation["counts"]) != set(definitions):
        raise Inconclusive("working SQL table census incomplete")
    server = observation["server"]
    require(server["transaction_read_only"] == "on" and server["transaction_isolation"] == "repeatable read", "economic rows not in one read-only snapshot")
    for name, (columns, _) in definitions.items():
        value = observation["tables"][name]
        integer(observation["counts"][name])
        if (not isinstance(value, list) or len(value) != observation["counts"][name]
                or len(value) > storage_queries.ROW_CAP):
            raise Inconclusive("economic SQL row count/truncation")
        for row in value:
            fields(row, columns.split(), name)


def replay_checkpoint(observation, job, canonical_parent, *, api_sha256=None, bootstrap_claim=None,
                      source_contract=None, migration_checksums=None, restoration_views=None, confirmation_depth=None):
    definitions, keys, schema, initial = TABLES, KEYS, SCHEMA, None
    if source_contract is None:
        working_pin(api_sha256)
        validate_observation(observation)
    else:
        from economic_checkpoint import observation_layout, validate_initial
        definitions, keys, schema, query_id = observation_layout(observation)
        validate_observation(observation, definitions, schema, query_id)
        initial = validate_initial(observation, migration_checksums=migration_checksums, source_contract=source_contract)
    job = frozen_job(job, height=canonical_parent.tip["height"] + 1,
                     parent_hash=bytes.fromhex(canonical_parent.tip["hash"])[::-1].hex())
    replay = accounting_history._replay(observation, definitions, keys, REDACTED, schema, initial_state=initial)
    # Every committed transaction's B changes must be explained by its actual
    # immutable ledger changes. SQL allocation gaps are not sequence gaps.
    pending_deltas, ledger_deltas = defaultdict(lambda: defaultdict(int)), defaultdict(lambda: defaultdict(int))
    for event in observation["tables"]["source_accounting_history"]:
        table, xid = event["table_name"], event["transaction_id"]
        if table == "economic_block":
            before, after = event["before_row"], event["after_row"]
            if before is not None and after is not None:
                immutable = ("block_hash", "parent_hash", "height", "payload_digest", "frozen_job", "validated_ledger", "consumption")
                if 'reconstruction_observation' in after:immutable+=('reconstruction_observation',)
                require(all(before[k] == after[k] for k in immutable), "historical per-block frozen economics rewritten")
                pair = before["lifecycle"], after["lifecycle"]
                permitted = {("unbooked", "booked"), ("unbooked", "cancelled"), ("booked", "cancelled"),
                             ("booked", "loss_held"), ("loss_held", "cancelled"), ("cancelled", "unbooked")}
                if source_contract is not None:permitted.add(("loss_held","booked"))
                if pair[0] != pair[1] and pair not in permitted:
                    raise Inconclusive("unsupported economic lifecycle transition")
                require(after["revision"] >= before["revision"]
                        and after["transition"] == before["transition"] + int(pair in {("cancelled", "unbooked"),("loss_held","booked")}),
                        "economic revision/transition binding")
                if pair == ("loss_held","booked"):
                    from oracle import Chain, ChainPrefix
                    if not restoration_views or xid not in restoration_views or confirmation_depth is None:
                        raise Inconclusive("loss-held restoration needs transaction-bound raw confirmation provenance")
                    view=restoration_views[xid]
                    require(isinstance(view,(Chain,ChainPrefix)),"restoration raw chain type")
                    bh=sql_bytes(after["block_hash"],32)[::-1].hex()
                    require(bh in view.by_hash and view.tip["height"] >= after["height"]+integer(confirmation_depth,1),
                            "loss-held restoration is shallow or noncanonical")
                    validate_restoration(observation["tables"]["source_accounting_history"],xid,before,after)
        if table not in {"pending_balance", "ledger_history"}:
            continue
        for sign, side in ((-1, "before_row"), (1, "after_row")):
            row = event[side]
            if row is None:
                continue
            identity = sql_bytes(row["sp_id"], 32).hex()
            if table == "pending_balance":
                pending_deltas[xid][identity] += sign * u64(row["sats"])
            else:
                ledger_deltas[xid][identity] += sign * integer(row["pending_delta"], -(1 << 63), (1 << 63) - 1)
    for xid in set(pending_deltas) | set(ledger_deltas):
        p, l = pending_deltas[xid], ledger_deltas[xid]
        require(all(p.get(identity, 0) == l.get(identity, 0) for identity in set(p) | set(l)), "booked pending change lacks same-transaction ledger replay")
    state_rows = list(replay["state"]["economic_state"].values())
    if len(state_rows) != 1 or state_rows[0]["id"] != 1:
        raise Inconclusive("economic state singleton checkpoint missing")
    state = state_rows[0]
    integer(state["id"], 1, 1); integer(state["revision"], 0, (1 << 63) - 1)
    integer(state["anchor_height"], 0, (1 << 31) - 1)
    if state["complete"] is not True or state["anchor_hash"] is None:
        raise Inconclusive("economic parent checkpoint incomplete")
    if state["revision"] != job["revision"]:
        raise Inconclusive("frozen revision needs its exact historical snapshot; current DB is not prior")
    require(state["anchor_height"] == canonical_parent.tip["height"]
            and sql_bytes(state["anchor_hash"], 32)[::-1].hex() == canonical_parent.tip["hash"], "economic checkpoint is not bound to raw parent")
    chain_rows = sorted(replay["state"]["economic_chain"].values(), key=lambda r: r["height"])
    if len(chain_rows) != len(canonical_parent.blocks):
        raise Inconclusive("economic canonical ancestry census incomplete")
    for row, raw in zip(chain_rows, canonical_parent.blocks):
        require(row["height"] == raw["height"] and sql_bytes(row["block_hash"], 32)[::-1].hex() == raw["hash"]
                and sql_bytes(row["parent_hash"], 32)[::-1].hex() == raw["prev"], "economic chain differs from raw ancestry")
        expected_cb = bytes(32) if row["height"] == 0 else hashlib.sha256(raw["transactions"][0]["raw"]).digest()
        require(sql_bytes(row["coinbase_digest"], 32) == expected_cb and type(row["pool_block"]) is bool
                and 0 <= integer(row["revision"]) <= state["revision"], "economic chain coinbase/revision mismatch")
    # Use same-snapshot raw_event only as a digest preimage, never as a past B.
    by_block = {r["block_hash"]: r for r in observation["tables"]["economic_block"]}
    active = []
    for key, historical in replay["state"]["economic_block"].items():
        stored = {**historical, "raw_event": by_block[key[0]]["raw_event"]}
        item = parse_block(stored)
        if item["lifecycle"] in {"unbooked", "booked"} and bytes(item["block_hash"])[::-1].hex() not in canonical_parent.by_hash:
            raise Inconclusive("booked/unbooked economic block is not canonical at checkpoint")
        actual_ledger = {row["sp_id"]: row for row in replay["state"]["ledger_history"].values() if row["block_hash"] == key[0]}
        if item["lifecycle"] == "booked":
            expected = {}
            for row in stored["validated_ledger"]:
                identity = "\\x" + hash_array(row["sp_id"])
                expected[identity] = {**row, "sp_id": identity, "block_hash": key[0], "rail": row["rail"].lower()}
            require(actual_ledger == expected, "booked block lacks its exact immutable ledger effects")
        elif item["lifecycle"] == "unbooked":
            require(not actual_ledger, "unbooked block has premature booked ledger effects")
        elif item["lifecycle"] == "cancelled" and actual_ledger:
            raise Inconclusive("cancelled block retains unreconciled ledger effects")
        if item["lifecycle"] in {"unbooked", "loss_held"}:
            active.append(item)
    economic_rows = replay["state"]["economic_block"]
    for row in chain_rows:
        if row["pool_block"]:
            if (row["block_hash"],) not in economic_rows or economic_rows[(row["block_hash"],)]["lifecycle"] not in {"unbooked", "booked"}:
                raise Inconclusive("canonical pool block lacks complete economic lifecycle")
    view = {"version": 1, "revision": state["revision"], "anchor_height": state["anchor_height"],
            "anchor_hash": list(sql_bytes(state["anchor_hash"], 32)), "complete": True,
            "booked": [[list(sql_bytes(row["sp_id"], 32)), row["sats"]]
                       for row in sorted(replay["state"]["pending_balance"].values(), key=lambda r: r["sp_id"])],
            "reservations": sorted(active, key=lambda r: (r["height"], r["block_hash"]))}
    calculated = bootstrap_view(view)["balances"]
    for balance in job["balances"]:
        identity = hash_array(balance["sp_id"])
        # An absent identity is zero only after a complete-from-empty replay,
        # never because SPCB/2 omitted economic provenance.
        own = calculated.get(identity, {"booked": 0, "reserved": 0, "available": 0})
        require(all(balance[k] == own[k] for k in ("booked", "reserved", "available")), "pool FrozenJob B/R/A differs from independent history replay")
    if bootstrap_claim is not None:
        bootstrap_view(bootstrap_claim)
        require(bootstrap_claim == view, "bootstrap active-set/parent/revision differs from replayed checkpoint")
    return {"scope": "offline_working_RULEB_row_replay_only", "available_from_replay": {k: v["available"] for k, v in calculated.items()},
            "revision": state["revision"], "parent_hash": canonical_parent.tip["hash"],
            "history_watermark": replay["event_watermark"], "bootstrap": view,
            "raw_settlement_entitlement_verified": False, "trusted_history_checkpoint_issued": False,
            "missing_components": ["tested_producer_binding", "independently_verified_settlement_and_frozen_job_ancestry"],
            "live_acceptance_eligible": False}


def validate_restoration(history, xid, before, after):
    """Complete-ledger restoration, with retained Sent rows never reissued."""
    group=[e for e in history if e["transaction_id"]==xid]
    # Reconstruct the committed prefix of relevant per-block rows. Events from
    # this transaction are grouped; no numeric sequence-contiguity assumption.
    first=min(e["event_id"] for e in group)
    prior={"ledger_history":{},"ark_payout":{}}
    for e in history:
        if e["event_id"]>=first or e["table_name"] not in prior:continue
        table=e["table_name"]
        for side,remove in (("before_row",True),("after_row",False)):
            row=e[side]
            if row is None or row["block_hash"]!=before["block_hash"]:continue
            if remove:prior[table].pop(row["sp_id"],None)
            else:prior[table][row["sp_id"]]=row
    expected={"\\x"+hash_array(r["sp_id"]):r for r in after["validated_ledger"]}
    require(set(prior["ledger_history"])<=set(expected),"restoration retained unrelated ledger identity")
    for identity,row in prior["ledger_history"].items():
        original=expected[identity]
        require(all(row[k]==original[k] for k in ("paid_sats","ark_due_sats","forfeited_sats","pending_delta"))
                and row["rail"]==original["rail"].lower() and row["ark_due_sats"]>0,"retained irreversible ledger differs from frozen effects")
        payout=prior["ark_payout"].get(identity)
        if payout is None or payout["status"] not in {"sent","scheduled"}:
            raise Inconclusive("retained Ark provenance missing")
        if payout["status"]=="scheduled":
            raise Inconclusive("retained scheduled restoration needs transaction-bound reserved aggregate membership")
        changes=[e for e in group if e["table_name"] in {"ledger_history","ark_payout"}
                 and any(e[s] is not None and e[s].get("block_hash")==before["block_hash"] and e[s].get("sp_id")==identity for s in ("before_row","after_row"))]
        require(not changes,"retained Sent/ledger effects must not be rewritten or reissued")
    inserted={e["after_row"]["sp_id"] for e in group if e["table_name"]=="ledger_history" and e["operation"]=="INSERT"
              and e["after_row"]["block_hash"]==before["block_hash"]}
    require(inserted==set(expected)-set(prior["ledger_history"]),"restoration did not insert exactly missing ledger identities")
