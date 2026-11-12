"""Read actual migration-0011 checkpoint bytes; never synthesize monetary rows."""
from dataclasses import dataclass
import hashlib

from case_contract import canonical_json
from oracle import Inconclusive, integer, require, strict_json
from ruleb_sql import TABLES, KEYS, sql_bytes
from storage_queries import _render
from source_feed_codec import READ_COMPATIBLE_CONTRACTS

COLUMNS = "checkpoint_id schema_version migration_version classification economic_revision history_watermark transaction_id snapshot_boundary state_bytes snapshot_bytes snapshot_sha256 recorded_at"
SCHEMA = "spc-lab-sql-observation/4-source-r3"
QUERY_ID = "spc-lab-immutable-payout-rows/4-source-r3"
TABLES_R3 = {**TABLES, "economic_checkpoint": (COLUMNS,"checkpoint_id")}
KEYS_R3 = {**KEYS, "economic_checkpoint": ("checkpoint_id",)}
INITIAL = {"id":1,"revision":0,"anchor_height":0,"anchor_hash":None,"complete":False,"reason":"unreconciled"}
SNAPSHOT_FIELDS = "schema migration_version complete_from_empty state row_limit history_start booked_balances economic_blocks chain found ledger ark_payouts prior_migrations".split()


def query(*, source_contract):
    from decision_observations import CONTRACT as DECISIONS, query as decision_query
    if source_contract==DECISIONS:return decision_query()
    require(source_contract in READ_COMPATIBLE_CONTRACTS,"economic checkpoint source pin")
    return _render(TABLES_R3,QUERY_ID,SCHEMA)


def observation_layout(observation):
    from decision_observations import SCHEMA as DECISIONS, layout
    return layout() if observation.get('schema')==DECISIONS else (TABLES_R3,KEYS_R3,SCHEMA,QUERY_ID)


def visible(xid, snapshot):
    parts = snapshot.split(":")
    require(len(parts)==3,"checkpoint SQL snapshot shape")
    lo, hi = integer(int(parts[0])), integer(int(parts[1]))
    active = [integer(int(v)) for v in parts[2].split(",") if v]
    require(lo <= hi and active == sorted(set(active)) and all(lo <= v < hi for v in active),"checkpoint SQL snapshot bounds")
    return xid < hi and xid not in active


@dataclass(frozen=True)
class InitialCheckpoint:
    state_bytes: bytes
    snapshot_digest: str
    history_event_id: int
    preceding_watermark: int
    transaction_id: int

    def replay_seed(self):
        return {"economic_state": {(1,): strict_json(self.state_bytes)}}


def validate_initial(observation, *, migration_checksums, source_contract):
    require(source_contract in READ_COMPATIBLE_CONTRACTS,"economic seed source pin")
    values = observation["tables"].get("economic_checkpoint")
    if not isinstance(values,list) or len(values)!=1:
        raise Inconclusive("actual economic checkpoint singleton missing")
    row = values[0]
    require(set(row)==set(COLUMNS.split()),"economic checkpoint columns changed")
    for k,v in (("checkpoint_id",1),("schema_version",1),("migration_version",11),("economic_revision",0)):
        if type(row[k]) is not int or row[k]!=v:
            raise Inconclusive("unsupported or nonfresh economic checkpoint version/revision")
    if row["classification"] != "fresh_empty":
        raise Inconclusive("nonempty compatibility checkpoint is unverified; amounts preserved")
    raw = sql_bytes(row["state_bytes"]); snapshot_raw = sql_bytes(row["snapshot_bytes"])
    if len(raw)>4096 or len(snapshot_raw)>8*1024*1024:
        raise Inconclusive("economic checkpoint byte bound")
    require(hashlib.sha256(snapshot_raw).digest()==sql_bytes(row["snapshot_sha256"],32),"economic checkpoint snapshot digest mismatch")
    state, snapshot = strict_json(raw), strict_json(snapshot_raw)
    require(set(snapshot)==set(SNAPSHOT_FIELDS),"checkpoint snapshot fields missing/unknown")
    require(canonical_json(state)==canonical_json(INITIAL) and canonical_json(snapshot["state"])==canonical_json(state),"economic checkpoint is not the exact captured initial row")
    require(snapshot["schema"]=="spc.economic-checkpoint/1" and type(snapshot["migration_version"]) is int
            and snapshot["migration_version"]==11 and snapshot["complete_from_empty"] is True
            and type(snapshot["row_limit"]) is int and snapshot["row_limit"]==1025,"economic snapshot schema/completeness")
    for name in ("booked_balances","economic_blocks","chain","found","ledger","ark_payouts"):
        if snapshot[name] != []:
            raise Inconclusive("initial economic snapshot is not empty: "+name)
    migrations = snapshot["prior_migrations"]
    require(isinstance(migrations,list) and all(set(r)=={"version","checksum"} for r in migrations),"migration checksum rows")
    actual = {integer(r["version"],1,10):r["checksum"] for r in migrations}
    if not migration_checksums or actual != migration_checksums or len(actual)!=len(migrations):
        raise Inconclusive("prior migration checksums not independently bound to source")
    history = observation["tables"]["source_accounting_history"]
    records = [h for h in history if h["table_name"]=="economic_checkpoint"]
    if len(records)!=1:
        raise Inconclusive("checkpoint INSERT journal coverage missing/ambiguous")
    record = records[0]
    require(record["operation"]=="INSERT" and record["before_row"] is None and record["after_row"]==row,
            "checkpoint readback differs from actual journal INSERT")
    prior = [h for h in history if h["event_id"] < record["event_id"]]
    if len(prior)!=1:
        raise Inconclusive("fresh checkpoint has nonempty/unverified prior history")
    start = prior[0]; start_image={k:start[k] for k in ("event_id","table_name","operation","after_row")}
    require(start_image==snapshot["history_start"] and start["before_row"] is None
            and start["table_name"]=="checkpoint" and start["operation"]=="history_start"
            and start["after_row"].get("schema")=="spc.accounting-history/1"
            and start["after_row"].get("complete_from_empty") is True
            and start["after_row"].get("external_checkpoint_required") is False,"economic history_start binding")
    require(integer(row["history_watermark"],1)==start["event_id"] and record["event_id"]>row["history_watermark"],"economic preceding history watermark mismatch")
    xid=integer(row["transaction_id"],1)
    require(record["transaction_id"]==xid and xid!=start["transaction_id"]
            and visible(start["transaction_id"],row["snapshot_boundary"])
            and visible(xid,observation["server"]["snapshot"]),"checkpoint transaction/snapshot correlation")
    return InitialCheckpoint(raw,hashlib.sha256(snapshot_raw).hexdigest(),record["event_id"],row["history_watermark"],xid)
