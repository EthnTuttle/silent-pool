import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import ap5
from common import (Blocked, Evidence, PROTECTED, RpcUnavailable, command_id,
                    confirmed_transaction, fresh_root, sats, sha, status_ready,
                    stock_id_bytes, verify_source)
from scenario import scenario


class Contracts(unittest.TestCase):
    def test_default_is_full_roster_and_single_declared_rate(self):
        s = scenario("test", 123)
        self.assertEqual((len(s["nodes"]), len(s["pools"]), len(s["wallet_agents"]), len(s["miners"])), (2, 2, 4, 4))
        self.assertEqual(sum(int(m["workers"][0]["budget"]["hashes_per_second"]) for m in s["miners"]), 40)
        self.assertEqual(s["traffic"]["driver"]["bootstrap"]["hashes_per_second"], 40)
        self.assertEqual(sum(a["network_wallets"] for a in s["traffic"]["driver"]["actors"]), 8)
        self.assertEqual(s["traffic"]["driver"]["user_payment_wallets"], 4)
        self.assertEqual(s["traffic"]["rate"]["ordinary_tx_per_sec"], 0.2)
        self.assertEqual(s["traffic"]["bootstrap"]["mining_mode"], "native")
        self.assertFalse(any("fixture" in m["implementation"] for m in s["miners"]))

    def test_scenario_declares_runtime_normalized_config_surface(self):
        # The QA controller's config_binding case requires the runtime-rendered
        # private scenario to equal the prepared input exactly for the traffic
        # block, each miner worker budget and each pool work_profile. The
        # qualified runtime serde-defaults omitted keys (lab_acceptance,
        # total_hashes, legacy_share_difficulty) when it renders, so the input
        # must declare them. lab_acceptance=True is also the funded-campaign
        # opt-in that arms the reviewed acceptance adapters.
        s = scenario("test", 123)
        self.assertIs(s["traffic"]["lab_acceptance"], True)
        self.assertIs(s["expanded_access"], True)
        for miner in s["miners"]:
            for worker in miner["workers"]:
                self.assertIn("total_hashes", worker["budget"])
        for pool in s["pools"]:
            self.assertIn("legacy_share_difficulty", pool["work_profile"])

    def test_protected_run_and_ancestor_rejected_before_creation(self):
        for root in PROTECTED:
            for path in (root, root / "new", root.parent):
                with self.assertRaisesRegex(Blocked, "protected"):
                    fresh_root(path)

    def test_reusing_directory_never_adopts_or_truncates(self):
        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp) / "evidence"
            p.mkdir()
            (p / "keep").write_bytes(b"untouched")
            with self.assertRaises(Blocked):
                fresh_root(p)
            self.assertEqual((p / "keep").read_bytes(), b"untouched")

    def test_manifest_claim_cannot_mask_stale_binary_source(self):
        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp) / "maintenance.rs"
            p.write_text("audited")
            expected = sha(p)
            manifest = {"files": [{"path": p.name, "sha256": expected}]}
            verify_source(Path(tmp), manifest, {p.name: expected})
            p.write_text("old unsafe implementation")
            with self.assertRaisesRegex(Blocked, "actual Nix build source is stale"):
                verify_source(Path(tmp), manifest, {p.name: expected})
            with self.assertRaisesRegex(Blocked, "lacks approved"):
                verify_source(Path(tmp), {"files": []}, {p.name: expected})

    def test_absent_transaction_is_unknown_not_confirmed_zero(self):
        class Node:
            def read(self, *args):
                raise RpcUnavailable("getrawtransaction", {"code": -5, "message": "not found"})
        self.assertIsNone(confirmed_transaction([Node(), Node()], "00" * 32))

    def test_non_not_found_rpc_errors_are_not_silently_retried(self):
        class Node:
            def read(self, *args):
                raise RpcUnavailable("getrawtransaction", {"code": -32602})
        with self.assertRaises(RpcUnavailable):
            confirmed_transaction([Node(), Node()], "00" * 32)

    def test_exact_money_and_unknowns(self):
        self.assertEqual(sats("0.00000001"), 1)
        self.assertEqual(sats("1.2"), 120000000)
        for value in (None, True, "NaN", "0.000000001", "-1"):
            with self.assertRaises(Blocked):
                sats(value)

    def test_registration_requires_current_bounded_lease(self):
        base = {"state": "connected", "registered": True, "registration_live_until_unix_ms": 1200}
        self.assertTrue(status_ready(base, 1000))
        for field, value in [("state", "dormant"), ("registered", False),
                             ("registration_live_until_unix_ms", None),
                             ("registration_live_until_unix_ms", 999),
                             ("registration_live_until_unix_ms", 32000)]:
            self.assertFalse(status_ready({**base, field: value}, 1000))

    def test_ambiguous_native_payment_is_not_resubmitted(self):
        with tempfile.TemporaryDirectory() as tmp:
            ev = Evidence(Path(tmp))
            c = ap5.Controller({}, {}, Path(tmp), ev, scenario("run", 1), 10)
            c.url, c.token = "http://127.0.0.1:12345", "private"
            with patch.object(ap5, "request", side_effect=TimeoutError("lost reply")) as request:
                with self.assertRaisesRegex(Blocked, "retain original id"):
                    c.command("fund", {"type": "send_user_payment"})
                self.assertEqual(request.call_count, 1)
            ev.close()
            files = list(Path(tmp).glob("*-command-unknown.json"))
            self.assertEqual(json.loads(files[0].read_text())["command_id"], command_id("run", "fund"))

    def test_stock_outpoint_and_public_return_address(self):
        txid = "01" + "00" * 31
        self.assertEqual(stock_id_bytes(txid + ":2"), "00" * 31 + "01" + "02000000")
        self.assertEqual(ap5.p2pkh_address("76a914751e76e8199196d454941c45d1b3a323f1433bd688ac"),
                         "mrCDrCybB6J1vRfbwM5hemdJz73FwDBC8r")

    def test_return_address_uses_wallet_role_index_not_receive_index(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            script = "76a914751e76e8199196d454941c45d1b3a323f1433bd688ac"
            journal = root / "journal.jsonl"
            journal.write_text(json.dumps({"record": "funding_observed", "owner_role": {"kind": "user_payment", "index": 1},
                                           "index": 7, "script_hex": script}) + "\n")
            ev = Evidence(root)
            c = ap5.Controller({}, {}, root, ev, scenario("run", 1), 10)
            c.manifest = lambda: {"traffic_driver_paths": {"txd-0": {"journal_path": str(journal)}}}
            self.assertEqual(c.return_address("bob"), "mrCDrCybB6J1vRfbwM5hemdJz73FwDBC8r")
            with self.assertRaises(Blocked):
                c.return_address("alice")
            ev.close()

    def test_readonly_observation_cannot_create_or_mutate_a_wallet(self):
        import sqlite3
        from contextlib import closing
        from common import readonly_rows
        with tempfile.TemporaryDirectory() as tmp:
            db = Path(tmp) / "wallet.sqlite"
            with self.assertRaises(FileNotFoundError):
                readonly_rows(db, "SELECT 1")
            self.assertFalse(db.exists())
            with closing(sqlite3.connect(db)) as connection:
                with connection:
                    connection.execute("CREATE TABLE balance (sats INTEGER)")
                    connection.execute("INSERT INTO balance VALUES (7)")
            with self.assertRaises(Blocked):
                readonly_rows(db, "UPDATE balance SET sats=1000")
            self.assertEqual(readonly_rows(db, "SELECT sats FROM balance"), [{"sats": 7}])


if __name__ == "__main__":
    unittest.main()
