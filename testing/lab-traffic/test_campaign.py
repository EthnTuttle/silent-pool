import copy
import contextlib
import io
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import campaign
from case_contract import ORDER, required, verdict, semantic_scenario
from oracle import Inconclusive, Invalid, strict_json

BASE = None  # tempfile follows the caller's TMPDIR.


class CampaignTests(unittest.TestCase):
    def setUp(self):
        output = contextlib.redirect_stdout(io.StringIO())
        output.__enter__()
        self.addCleanup(output.__exit__, None, None, None)

    def receipt(self, case="funded"):
        expected = {"case": case, "root": Path("/owned/" + case), "run_id": case + "-fresh",
                    "bindings": {"inputs_sha256": "11" * 32, "scenario_sha256": "22" * 32,
                                 "scenario_semantic_sha256": "33" * 32, "source_snapshot_sha256": "44" * 32},
                    "binary_hashes": {"node": "55" * 32}}
        receipt = {"schema": "spc-lab-funded-acceptance-receipt/2", "scope": "campaign-case-only", "case": case,
                   "run_root": str(expected["root"]), "run_id": expected["run_id"], "bindings": expected["bindings"].copy(),
                   "binary_hashes": expected["binary_hashes"].copy(), "required_cases": sorted(required(case, case == "funded")),
                   "cases": {k: {"status": "PASSED"} for k in required(case, case == "funded")}, "status": "PASSED"}
        return receipt, expected

    def test_every_required_case_is_mandatory_and_no_failure_is_averaged(self):
        receipt, expected = self.receipt()
        self.assertEqual(campaign.assess_case(receipt, **expected), "PASSED")
        for key in required("funded", True):
            changed = copy.deepcopy(receipt)
            del changed["cases"][key]
            with self.subTest(key=key), self.assertRaises(Invalid):
                campaign.assess_case(changed, **expected)
            changed["status"] = "INCONCLUSIVE"
            self.assertEqual(campaign.assess_case(changed, **expected), "INCONCLUSIVE")
        receipt["cases"]["extra-diagnostic"] = {"status": "FAILED"}
        self.assertEqual(verdict(receipt["cases"], required("funded", True)), "FAILED")

    def test_scope_and_source_input_config_root_bindings_cannot_be_substituted(self):
        for field in ("inputs_sha256", "scenario_sha256", "scenario_semantic_sha256", "source_snapshot_sha256"):
            receipt, expected = self.receipt()
            receipt["bindings"][field] = "00" * 32
            with self.assertRaisesRegex(Invalid, "binding"):
                campaign.assess_case(receipt, **expected)
        for key, value in (("scope", "component-only"), ("run_root", "/another"), ("run_id", "adopted")):
            receipt, expected = self.receipt()
            receipt[key] = value
            with self.assertRaises(Invalid):
                campaign.assess_case(receipt, **expected)

    def test_T3_T4b_and_whole_lab_are_independent_required_conjunctions(self):
        cases = {case: {"status": "PASSED"} for case in ORDER}
        result = campaign.aggregate(cases, {"status": "INCONCLUSIVE"})
        self.assertTrue(result["T3_T4b_pass"])
        self.assertFalse(result["whole_lab_pass"])
        self.assertTrue(campaign.aggregate(cases, {"status": "PASSED"})["whole_lab_pass"])
        cases["bootstrap-stop"] = {"status": "FAILED"}
        self.assertEqual(campaign.aggregate(cases, {"status": "PASSED"})["whole_lab_status"], "FAILED")
        del cases["bootstrap-stop"]
        self.assertFalse(campaign.aggregate(cases, {"status": "PASSED"})["T3_T4b_pass"])

    def test_case_requirements_assign_auxiliary_gates_without_omitting_them(self):
        self.assertIn("bootstrap_stop", required("bootstrap-stop"))
        self.assertIn("retained_maturity_rollback", required("bootstrap-maturity"))
        self.assertNotIn("bootstrap_stop", required("funded", True))
        self.assertTrue({"witness_job_republication", "reorg_original_restore", "reorg_displaced_retry",
                         "v3_prior_unknown_retention", "cleanup"} <= required("funded", True))

    def test_decimal_rate_survives_case_rewrite_and_semantic_binding(self):
        value = strict_json(b'{"run":{"id":"base"},"traffic":{"rate":0.2,"amount":9007199254740993}}')
        before = semantic_scenario(value)
        value["run"]["id"] = "fresh-case"
        raw = campaign.encode(value)
        self.assertIn(b'"rate":0.2', raw)
        self.assertIn(b'9007199254740993', raw)
        self.assertEqual(before, semantic_scenario(strict_json(raw)))

    def test_real_preflight_failure_stops_serial_campaign_without_spawning_runtime(self):
        with tempfile.TemporaryDirectory(prefix="t3-campaign-vector-", dir=BASE) as tmp:
            root = Path(tmp) / "campaign"
            args = SimpleNamespace(out=str(root), inputs=str(campaign.HERE / "inputs.template.json"),
                scenario=str(campaign.HERE / "scenario.json"), destination="fixture-unused", case_timeout=1,
                timeout=500, start_timeout=1, measure_seconds=1, pause_seconds=1, max_blocks=10,
                amount_sats=10000, ap5_evidence=None)
            with patch("campaign.LOCK", str(Path(tmp) / "lock")):
                self.assertEqual(campaign.execute(args), 2)
            report = json.loads((root / "campaign.json").read_bytes())
            self.assertEqual(report["T3_T4b_status"], "INCONCLUSIVE")
            self.assertEqual(set(report["cases"]), set(ORDER))
            self.assertFalse((root / "1-bootstrap-stop" / "runtime").exists())
            self.assertFalse((root / "2-bootstrap-maturity").exists())
            self.assertFalse((root / "3-funded").exists())

    def test_existing_and_protected_roots_are_not_adopted(self):
        with tempfile.TemporaryDirectory(prefix="t3-no-adopt-", dir=BASE) as tmp:
            marker = Path(tmp) / "user-file"
            marker.write_bytes(b"retain")
            with self.assertRaises(Invalid):
                campaign.fresh_root(tmp)
            self.assertEqual(marker.read_bytes(), b"retain")
        with self.assertRaises(Invalid):
            campaign.fresh_root(campaign.PROTECTED[0] / "new-child")

    def test_deadline_always_enters_cleanup_before_return(self):
        calls = []
        class Capture:
            def add(self, *args):
                pass
        class Controller:
            capture = Capture()
            def __init__(self, args):
                pass
            def prerequisites(self):
                calls.append("preflight")
                raise campaign.Deadline()
            def result(self, *args):
                calls.append("inconclusive")
        def finalize(controller):
            calls.append("cleanup")
            return 2
        module = SimpleNamespace(Controller=Controller, finalize_controller=finalize)
        with patch("campaign.signal.signal"), patch("campaign.signal.setitimer"):
            _, code = campaign.run_case(module, SimpleNamespace(case="funded"), {}, campaign.time.monotonic() + 1)
        self.assertEqual(code, 2)
        self.assertEqual(calls, ["preflight", "inconclusive", "cleanup"])

    def test_serial_scheduler_uses_three_distinct_roots_and_ids_with_same_policy(self):
        with tempfile.TemporaryDirectory(prefix="t3-serial-vector-", dir=BASE) as tmp:
            args = SimpleNamespace(out=str(Path(tmp) / "campaign"), inputs=str(campaign.HERE / "inputs.template.json"),
                scenario=str(campaign.HERE / "scenario.json"), destination="fixture-unused", case_timeout=1,
                timeout=500, start_timeout=1, measure_seconds=1, pause_seconds=1, max_blocks=10,
                amount_sats=10000, ap5_evidence=None)
            calls = []
            def run(_module, options, record, deadline):
                calls.append((options, record))
                return SimpleNamespace(root=Path(options.run_root)), 0
            def harvested(controller, expected, code):
                self.assertEqual(expected["root"], controller.root)
                self.assertEqual(code, 0)
                return {"status": "PASSED", "cleanup_passed": True}
            with patch("campaign.LOCK", str(Path(tmp) / "lock")), patch("campaign.run_case", run), patch("campaign.harvest", harvested):
                self.assertEqual(campaign.execute(args), 2)  # AP5 remains absent.
            self.assertEqual([o.case for o, _ in calls], list(ORDER))
            self.assertEqual(len({o.run_root for o, _ in calls}), 3)
            self.assertEqual(len({r["run_id"] for _, r in calls}), 3)
            self.assertEqual(len({r["bindings"]["scenario_semantic_sha256"] for _, r in calls}), 1)
            self.assertEqual([o.witness for o, _ in calls], [False, False, True])
            report = json.loads((Path(args.out) / "campaign.json").read_bytes())
            self.assertTrue(report["T3_T4b_pass"])
            self.assertFalse(report["whole_lab_pass"])

    def test_cleanup_failure_blocks_every_later_case_even_if_first_receipt_claims_pass(self):
        with tempfile.TemporaryDirectory(prefix="t3-cleanup-stop-vector-", dir=BASE) as tmp:
            args = SimpleNamespace(out=str(Path(tmp) / "campaign"), inputs=str(campaign.HERE / "inputs.template.json"),
                scenario=str(campaign.HERE / "scenario.json"), destination="unused", case_timeout=1, timeout=500,
                start_timeout=1, measure_seconds=1, pause_seconds=1, max_blocks=10, amount_sats=10000, ap5_evidence=None)
            with patch("campaign.LOCK", str(Path(tmp) / "lock")), \
                 patch("campaign.run_case", return_value=(object(), 1)) as run, \
                 patch("campaign.harvest", return_value={"status": "FAILED", "cleanup_passed": False}):
                self.assertEqual(campaign.execute(args), 1)
            self.assertEqual(run.call_count, 1)
            report = json.loads((Path(args.out) / "campaign.json").read_bytes())
            self.assertEqual(report["cases"]["funded"]["status"], "INCONCLUSIVE")

    def test_existing_failure_or_unknown_cannot_be_rewritten_green(self):
        module = campaign.load_controller()
        controller = object.__new__(module.Controller)
        controller.results = {}
        controller.result("gate", "INCONCLUSIVE", "missing")
        controller.result("gate", "PASSED", "later")
        self.assertEqual(controller.results["gate"]["status"], "INCONCLUSIVE")
        controller.result("gate", "FAILED", "contradiction")
        controller.result("gate", "PASSED", "later")
        self.assertEqual(controller.results["gate"]["status"], "FAILED")


if __name__ == "__main__":
    unittest.main()
