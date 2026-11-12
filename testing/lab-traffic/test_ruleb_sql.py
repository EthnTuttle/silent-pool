"""Concrete working SQL/history rows; synthetic arithmetic, not native proof."""
import copy
import hashlib
import unittest

from oracle import Chain, Inconclusive, Invalid, sha256d
from ruleb_source import API_SHA256
from ruleb_sql import TABLES, KEYS, SCHEMA, QUERY_ID, query, replay_checkpoint
from test_oracle import block_bytes, coinbase, policy
from test_ruleb_source import job, consumption_row


def fixture():
    raws = []
    for height in range(3):
        raws.append(block_bytes(sha256d(raws[-1][:80]) if raws else bytes(32),
                               [coinbase(tag=b"\2" + height.to_bytes(2,"little"))], 1600000000+height))
    chain = Chain(policy(genesis=sha256d(raws[0][:80])[::-1].hex(), regtest_pow_bits_overlay=None), raws)
    sql_hash = lambda height: "\\x" + sha256d(raws[height][:80]).hex()
    current = {table: {} for table in KEYS}
    history = [{"event_id": 1, "transaction_id": 1, "table_name": "checkpoint", "operation": "history_start",
                "before_row": None, "after_row": {"schema":"spc.accounting-history/1", "complete_from_empty":True,
                                                    "external_checkpoint_required":False}, "recorded_at":"now"}]
    def event(table, after, xid=2, before=None):
        op = "INSERT" if before is None else "DELETE" if after is None else "UPDATE"
        history.append({"event_id": history[-1]["event_id"]+2, "transaction_id":xid, "table_name":table,
                        "operation":op, "before_row":copy.deepcopy(before), "after_row":copy.deepcopy(after), "recorded_at":"now"})
        if before is not None:
            del current[table][tuple(before[k] for k in KEYS[table])]
        if after is not None:
            current[table][tuple(after[k] for k in KEYS[table])] = copy.deepcopy(after)
    state = {"id":1,"revision":1,"anchor_height":0,"anchor_hash":sql_hash(0),"complete":True,"reason":""}
    event("economic_state", state)
    for height in range(3):
        event("economic_chain", {"height":height,"block_hash":sql_hash(height),
            "parent_hash":sql_hash(height-1) if height else "\\x"+"00"*32,"pool_block":height>0,
            "coinbase_digest":"\\x"+(hashlib.sha256(chain.blocks[height]["transactions"][0]["raw"]).hexdigest() if height else "00"*32),
            "revision":height+1}, xid=height+2)
    def block(height, prior, error):
        frozen = job(height, prior)
        frozen["parent_hash"] = list(sha256d(raws[height-1][:80])); frozen["revision"] = height
        ledger = {"sp_id":[0x33]*32,"rail":"Pending","paid_sats":0,"ark_due_sats":0,"forfeited_sats":0,"pending_delta":error-prior}
        return {"block_hash":sql_hash(height),"parent_hash":sql_hash(height-1),"height":height,"payload_digest":"\\x"+"55"*32,
                "frozen_job":frozen,"validated_ledger":[ledger],"consumption":[consumption_row(prior,error)],
                "lifecycle":"unbooked","transition":1,"revision":height+1}
    first = block(1,0,500000)
    event("economic_block", first, xid=3)
    event("economic_block", {**first,"lifecycle":"booked"}, xid=3, before=first)
    event("ledger_history", {"block_hash":sql_hash(1),"sp_id":"\\x"+"33"*32,"rail":"pending",
        "paid_sats":0,"ark_due_sats":0,"forfeited_sats":0,"pending_delta":500000}, xid=3)
    event("pending_balance", {"sp_id":"\\x"+"33"*32,"sats":500000,"updated_at":"now"}, xid=3)
    second = block(2,500000,100000)
    event("economic_block", second, xid=4)
    event("economic_state", {**state,"revision":4,"anchor_height":2,"anchor_hash":sql_hash(2)}, xid=4, before=state)
    tables = {name:list(current.get(name, {}).values()) for name in TABLES}
    for row in tables["economic_block"]:
        row["raw_event"] = None  # Raw settlement proof is explicitly not claimed by these vectors.
    tables["source_accounting_history"] = history
    observation = {"schema":SCHEMA,"query_id":QUERY_ID,
        "server":{"snapshot":"20:30:22", "transaction_read_only":"on", "transaction_isolation":"repeatable read"},
        "history_boundary":{"event_watermark":history[-1]["event_id"],"wal_lsn":"0/100"},
        "tables":tables,"counts":{name:len(rows) for name,rows in tables.items()}}
    frozen = job(3,100000); frozen["revision"] = 4; frozen["parent_hash"] = list(sha256d(raws[-1][:80]))
    frozen["balances"][0].update(booked=500000,reserved=400000,available=100000)
    return observation, frozen, chain


def change(observation, table, before, after, xid=5):
    history = observation["tables"]["source_accounting_history"]
    def history_row(row):
        return {k:v for k,v in copy.deepcopy(row).items() if k != "raw_event"} if row is not None else None
    history.append({"event_id":history[-1]["event_id"]+2,"transaction_id":xid,"table_name":table,
                    "operation":"INSERT" if before is None else "DELETE" if after is None else "UPDATE",
                    "before_row":history_row(before),"after_row":history_row(after),"recorded_at":"now"})
    current = observation["tables"][table]
    if before is not None:
        current.remove(before)
    if after is not None:
        current.append(copy.deepcopy(after))
    observation["counts"][table] = len(current)
    observation["counts"]["source_accounting_history"] = len(history)
    observation["history_boundary"]["event_watermark"] = history[-1]["event_id"]


class WorkingSQLTests(unittest.TestCase):
    def replay(self, observation, frozen, chain, **kw):
        return replay_checkpoint(observation, frozen, chain, api_sha256=API_SHA256, **kw)

    def test_explicit_same_snapshot_query_and_independent_A_replay(self):
        sql = query(api_sha256=API_SHA256)
        self.assertIn("REPEATABLE READ READ ONLY", sql)
        self.assertNotIn("SELECT *", sql)
        for table in ("economic_state", "economic_block", "economic_chain", "ark_receipt_disposition", "ark_receipt_cursor"):
            self.assertIn('public."'+table+'"', sql)
        observation, frozen, chain = fixture()
        result = self.replay(observation, frozen, chain)
        self.assertEqual(result["available_from_replay"], {"33"*32:100000})
        self.assertFalse(result["trusted_history_checkpoint_issued"])
        self.assertFalse(result["live_acceptance_eligible"])

    def test_current_DB_or_pool_A_cannot_replace_exact_historical_revision(self):
        observation, frozen, chain = fixture()
        frozen["revision"] = 3
        with self.assertRaisesRegex(Inconclusive, "historical snapshot"):
            self.replay(observation, frozen, chain)
        frozen["revision"] = 4
        frozen["balances"][0].update(reserved=0, available=500000)
        with self.assertRaisesRegex(Invalid, "independent history"):
            self.replay(observation, frozen, chain)

    def test_missing_initial_economic_row_tail_or_active_reservation_is_unknown(self):
        for kind in ("initial", "tail", "active"):
            observation, frozen, chain = fixture()
            if kind == "initial":
                history = observation["tables"]["source_accounting_history"]
                history.pop(1)
                observation["counts"]["source_accounting_history"] -= 1
            elif kind == "tail":
                observation["history_boundary"]["event_watermark"] += 1
            else:
                observation["tables"]["economic_block"].pop()
                observation["counts"]["economic_block"] -= 1
            with self.subTest(kind=kind), self.assertRaises(Inconclusive):
                self.replay(observation, frozen, chain)

    def test_mismatched_bootstrap_active_census_is_not_assumed_empty(self):
        observation, frozen, chain = fixture()
        claim = self.replay(observation, frozen, chain)["bootstrap"]
        self.replay(observation, frozen, chain, bootstrap_claim=claim)
        claim["reservations"] = []
        with self.assertRaisesRegex(Invalid, "active-set"):
            self.replay(observation, frozen, chain, bootstrap_claim=claim)

    def test_pending_changes_need_actual_ledger_transaction_membership(self):
        observation, frozen, chain = fixture()
        pending = next(e for e in observation["tables"]["source_accounting_history"] if e["table_name"] == "pending_balance")
        pending["transaction_id"] = 5
        with self.assertRaisesRegex(Invalid, "same-transaction"):
            self.replay(observation, frozen, chain)

    def test_booked_row_cannot_rewrite_frozen_P_on_transition(self):
        observation, frozen, chain = fixture()
        events = observation["tables"]["source_accounting_history"]
        booking = next(e for e in events if e["table_name"] == "economic_block" and e["operation"] == "UPDATE")
        booking["after_row"]["frozen_job"]["generation"] += 1
        observation["tables"]["economic_block"][0]["frozen_job"]["generation"] += 1
        with self.assertRaisesRegex(Invalid, "frozen economics rewritten"):
            self.replay(observation, frozen, chain)

    def test_booking_retains_original_P_and_releases_only_its_c(self):
        observation, frozen, chain = fixture()
        block = next(row for row in observation["tables"]["economic_block"] if row["height"] == 2)
        change(observation, "economic_block", block, {**block,"lifecycle":"booked","revision":5})
        ledger = block["validated_ledger"][0]
        change(observation, "ledger_history", None, {**ledger,"sp_id":"\\x"+"33"*32,"block_hash":block["block_hash"],"rail":"pending"})
        pending = observation["tables"]["pending_balance"][0]
        change(observation, "pending_balance", pending, {**pending,"sats":100000})
        state = observation["tables"]["economic_state"][0]
        change(observation, "economic_state", state, {**state,"revision":5})
        frozen["revision"] = 5
        frozen["balances"][0].update(booked=100000,reserved=0,available=100000)
        result = self.replay(observation, frozen, chain)
        self.assertEqual(result["available_from_replay"]["33"*32],100000)
        self.assertEqual(block["consumption"][0]["prior"],500000)
        self.assertEqual(result["bootstrap"]["reservations"],[])

    def test_unbooked_orphan_releases_reservation_without_reversing_unbooked_delta(self):
        observation, _, chain = fixture()
        block = next(row for row in observation["tables"]["economic_block"] if row["height"] == 2)
        change(observation, "economic_block", block, {**block,"lifecycle":"cancelled","revision":5})
        row = next(row for row in observation["tables"]["economic_chain"] if row["height"] == 2)
        change(observation, "economic_chain", row, None)
        parent = chain.prefix(1)
        state = observation["tables"]["economic_state"][0]
        change(observation, "economic_state", state, {**state,"revision":5,"anchor_height":1,"anchor_hash":block["parent_hash"]})
        frozen = job(2,500000); frozen["revision"] = 5; frozen["parent_hash"] = list(bytes.fromhex(block["parent_hash"][2:]))
        result = self.replay(observation, frozen, parent)
        self.assertEqual(result["available_from_replay"]["33"*32],500000)
        self.assertEqual(result["bootstrap"]["reservations"],[])

    def test_booked_lifecycle_without_actual_ledger_booking_cannot_release_c(self):
        observation, frozen, chain = fixture()
        block = next(row for row in observation["tables"]["economic_block"] if row["height"] == 2)
        change(observation, "economic_block", block, {**block,"lifecycle":"booked","revision":5})
        state = observation["tables"]["economic_state"][0]
        change(observation, "economic_state", state, {**state,"revision":5})
        frozen["revision"] = 5; frozen["balances"][0].update(reserved=0,available=500000)
        with self.assertRaisesRegex(Invalid,"immutable ledger effects"):
            self.replay(observation, frozen, chain)


if __name__ == "__main__":
    unittest.main()
