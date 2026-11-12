import copy
import unittest

from accounting_history import replay
from oracle import Inconclusive, Invalid
from storage_queries import TABLES


class AccountingHistoryTests(unittest.TestCase):
    def fixture(self):
        start = {"event_id": 1, "transaction_id": 2, "table_name": "checkpoint", "operation": "history_start",
                 "before_row": None, "after_row": {"schema": "spc.accounting-history/1", "complete_from_empty": True,
                                                    "external_checkpoint_required": False}, "recorded_at": "2026-11-10T00:00:00Z"}
        return {"schema": "spc-lab-sql-observation/2", "server": {"snapshot": "10:20:12,15"},
                "history_boundary": {"event_watermark": 1, "wal_lsn": "0/100"},
                "tables": {**{name: [] for name in TABLES}, "source_accounting_history": [start]}}

    def append(self, value, eid, xid, op, before, after):
        value["tables"]["source_accounting_history"].append({"event_id": eid, "transaction_id": xid,
            "table_name": "pending_balance", "operation": op, "before_row": copy.deepcopy(before),
            "after_row": copy.deepcopy(after), "recorded_at": "2026-11-10T00:00:00Z"})
        value["history_boundary"]["event_watermark"] = eid

    def test_rollback_sequence_gaps_and_delete_tombstone_are_valid(self):
        value = self.fixture()
        a = {"sp_id": "\\x" + "11" * 32, "sats": 500000, "updated_at": "2026-11-10T00:00:00Z"}
        b = {**a, "sats": 100000}
        self.append(value, 4, 11, "INSERT", None, a)
        self.append(value, 9, 11, "UPDATE", a, b)
        self.append(value, 15, 14, "DELETE", b, None)
        result = replay(value)
        self.assertEqual(result["transactions"], {2: [1], 11: [4, 9], 14: [15]})
        self.assertEqual(result["state"]["pending_balance"], {})
        self.assertFalse(result["past_job_pending_verified"])

    def test_external_checkpoint_or_missing_start_is_inconclusive(self):
        for checkpoint in ({"complete_from_empty": False}, {"external_checkpoint_required": True}):
            value = self.fixture()
            value["tables"]["source_accounting_history"][0]["after_row"].update(checkpoint)
            with self.assertRaises(Inconclusive):
                replay(value)
        value = self.fixture()
        value["tables"]["source_accounting_history"] = []
        with self.assertRaises(Inconclusive):
            replay(value)

    def test_tail_truncation_and_current_state_substitution_are_inconclusive(self):
        value = self.fixture()
        value["history_boundary"]["event_watermark"] = 2
        with self.assertRaises(Inconclusive):
            replay(value)
        value = self.fixture()
        value["tables"]["pending_balance"] = [{"sp_id": "\\x" + "11" * 32, "sats": 4, "updated_at": "now"}]
        with self.assertRaises(Inconclusive):
            replay(value)

    def test_invisible_transaction_or_changed_before_image_fails(self):
        a = {"sp_id": "\\x" + "11" * 32, "sats": 500000, "updated_at": "now"}
        for xid in (12, 15, 20, 21):
            value = self.fixture()
            self.append(value, 4, xid, "INSERT", None, a)
            value["tables"]["pending_balance"] = [a]
            with self.assertRaisesRegex(Invalid, "not visible"):
                replay(value)
        value = self.fixture()
        self.append(value, 4, 11, "INSERT", None, a)
        self.append(value, 5, 11, "DELETE", {**a, "sats": 4}, None)
        with self.assertRaisesRegex(Invalid, "before-image differs"):
            replay(value)

    def test_unknown_new_economic_producer_is_not_silently_skipped(self):
        value = self.fixture()
        self.append(value, 4, 11, "INSERT", None, {})
        value["tables"]["source_accounting_history"][-1]["table_name"] = "economic_block"
        with self.assertRaisesRegex(Inconclusive, "unrecognized"):
            replay(value)


if __name__ == "__main__":
    unittest.main()
