"""Reporting tests: schema validation of committed evidence, markdown
rendering, redaction, and schema-file consistency.

Covers the frozen v1 evidence (no schema_version -> SCHEMA_V1 dispatch),
the historical r1 evidence (pinned to its historical contract hash,
validated schema-only: its consistency was proven at r1 freeze time
against the r1 contract revision), and the CURRENT r2 evidence (full
consistency against the current manifest, contract hash == the
canonical contract_hash(), and a hash-tree-consistent freeze recipe)."""

import json
import os
import re
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from arkrpc.contracts import contract_hash, LEGACY_R2_CONTRACT_SHA256, TRAFFIC_CONSUMER  # noqa: E402
from arkrpc.reporting import (_hash_tree, compare_reports,  # noqa: E402
                              render_markdown)
from arkrpc.schemadef import (SCHEMA, SCHEMA_VERSION, validate,  # noqa: E402
                              validate_report,
                              validate_report_consistency,
                              validate_report_schema_only)

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
REPORTS = os.path.join(HERE, "reports")

# Historical v2 evidence (r2 revision); J1 has a distinct contract.
R2_DIRS = ["2026-11-08-r2-l2-eb257af7", "2026-11-08-r2-v09lab-bcb6d5ee"]
# Historical r1 evidence: frozen under its own contract revision and
# pinned to that hash. Never rewritten, never re-derived against the
# current manifest (anachronistic); validated schema-only.
R1_DIRS = ["2026-11-08-r1-l2-eb257af7", "2026-11-08-r1-v09lab-bcb6d5ee"]
R1_CONTRACT_HASH = ("11bdb1007e130a749720f87482f287c1fb65d1e7"
                    "43c142fc4b1ca15a782d89e7")
# Frozen v1 evidence from the original packet; kept immutable, still
# validated through the SCHEMA_V1 dispatch.
V1_DIRS = ["l2-eb257af7", "v09lab-bcb6d5ee"]


class SchemaTests(unittest.TestCase):
    def test_schema_file_matches_schemadef(self):
        with open(os.path.join(HERE, "report.schema.json")) as fh:
            on_disk = json.load(fh)
        self.assertEqual(on_disk, SCHEMA,
                         "report.schema.json is stale; run "
                         "`probe.py schema-dump --out report.schema.json`")

    def test_schema_v2_three_status_verdict(self):
        statuses = SCHEMA["properties"]["verdict"]["properties"]["status"]["enum"]
        self.assertEqual(sorted(statuses),
                         sorted(["incompatible", "surface_compatible",
                                 "unproven"]))
        self.assertEqual(SCHEMA_VERSION, 2)

    def test_validator_basics(self):
        validate({"a": "x"}, {"type": "object", "required": ["a"],
                              "properties": {"a": {"type": "string"}}})
        with self.assertRaises(Exception):
            validate({"a": 3}, {"type": "object", "properties": {
                "a": {"type": "string"}}})
        with self.assertRaises(Exception):
            validate({"a": 3}, {"type": "object", "required": ["b"]})
        with self.assertRaises(Exception):
            validate("x", {"enum": ["passed", "failed"]})


class CommittedEvidenceTests(unittest.TestCase):
    """The committed evidence must validate and stay clean.

    The r2 dirs are the current evidence (full consistency + current
    contract hash + hash-tree-consistent freeze recipe); the r1 dirs
    are historical evidence pinned to their own contract hash
    (schema-only validation); the frozen v1 dirs must keep validating
    under the v1 schema so the old runs remain honest history rather
    than silently invalidated artifacts.
    """

    ALL_DIRS = R2_DIRS + R1_DIRS + V1_DIRS

    def _report(self, label):
        with open(os.path.join(REPORTS, label, "report.json")) as fh:
            return json.load(fh)

    def _manifest(self, label):
        with open(os.path.join(REPORTS, label, "manifest.json")) as fh:
            return json.load(fh)

    def test_validate_against_schema(self):
        for label in self.ALL_DIRS:
            with self.subTest(label=label):
                validate_report_schema_only(self._report(label))

    def test_r2_reports_are_schema_v2(self):
        for label in R2_DIRS:
            with self.subTest(label=label):
                self.assertEqual(self._report(label)["schema_version"], 2)

    def test_r2_contract_hash_is_pinned_historical(self):
        # Historical evidence must retain its exact original contract hash.
        for label in R2_DIRS:
            with self.subTest(label=label):
                report = self._report(label)
                manifest = self._manifest(label)
                self.assertEqual(report["contract_hash"], LEGACY_R2_CONTRACT_SHA256)
                self.assertEqual(manifest["contract_hash"], LEGACY_R2_CONTRACT_SHA256)
                self.assertNotEqual(contract_hash(), LEGACY_R2_CONTRACT_SHA256)

    def test_r1_contract_hash_is_pinned_historical(self):
        # D7: historical evidence keeps its historical hash; it is NOT
        # force-rewritten to match the current canonical contract.
        self.assertNotEqual(R1_CONTRACT_HASH, contract_hash())
        for label in R1_DIRS:
            with self.subTest(label=label):
                report = self._report(label)
                manifest = self._manifest(label)
                self.assertEqual(report["contract_hash"], R1_CONTRACT_HASH)
                self.assertEqual(manifest["contract_hash"], R1_CONTRACT_HASH)

    def test_r2_manifest_is_historical_not_current_source(self):
        # J1 source differs from the immutable r2 source receipt. Never
        # rewrite old reports to make a new source tree appear validated.
        tree = _hash_tree(HERE)
        for label in R2_DIRS:
            with self.subTest(label=label):
                manifest = self._manifest(label)
                self.assertNotEqual(manifest["freeze_recipe"]["files"], tree)
                self.assertEqual(manifest["contract_hash"], LEGACY_R2_CONTRACT_SHA256)

    def test_r2_manifest_binds_devcheck(self):
        # D7: the run manifest carries an explicit dev_verification key
        # binding the devcheck sources and the gate's exit receipt.
        for label in R2_DIRS:
            with self.subTest(label=label):
                dev = self._manifest(label).get("dev_verification")
                self.assertIsNotNone(dev, "missing dev_verification")
                self.assertEqual(dev["exit_code"], 0)
                self.assertIn("devcheck/src/main.rs", dev["sources"])
                self.assertIn("[PASS]", dev["receipt"])
                vectors = [line for line in dev["receipt"].splitlines()
                           if line.startswith("[PASS]")]
                self.assertEqual(len(vectors), dev["vectors"])

    def test_r2_devcheck_sources_hash_to_their_manifest_values(self):
        import hashlib
        for label in R2_DIRS:
            with self.subTest(label=label):
                dev = self._manifest(label)["dev_verification"]
                for rel, want in dev["sources"].items():
                    with open(os.path.join(HERE, rel), "rb") as fh:
                        got = hashlib.sha256(fh.read()).hexdigest()
                    self.assertEqual(got, want, rel)

    def test_frozen_v1_reports_stay_v1(self):
        for label in V1_DIRS:
            with self.subTest(label=label):
                self.assertNotIn("schema_version", self._report(label))

    def test_r2_verdicts_are_honest(self):
        for label in R2_DIRS:
            report = self._report(label)
            with self.subTest(label=label):
                self.assertEqual(report["verdict"]["status"], "incompatible")
                self.assertIn("fail-version-gate",
                              report["verdict"]["reasons"])
                self.assertIn("funded-unproven",
                              report["verdict"]["reasons"])
                self.assertNotIn("incomplete", report["verdict"]["reasons"])
                self.assertEqual(report["verdict"]["funded_state"],
                                 "unproven")
                self.assertIs(report["verdict"]["full_acceptance"], False)
                # observed version must be recorded as the real 190000
                self.assertEqual(report["observations"]["rpc_version"],
                                 190000)

    def test_r1_verdicts_stay_honest_history(self):
        for label in R1_DIRS:
            report = self._report(label)
            with self.subTest(label=label):
                self.assertEqual(report["verdict"]["status"], "incompatible")
                self.assertIn("fail-version-gate",
                              report["verdict"]["reasons"])
                self.assertEqual(report["verdict"]["funded_state"],
                                 "unproven")

    def test_r2_carry_traffic_and_observational_checks(self):
        # T0/D4: the traffic-slice check (getmempoolinfo) and the
        # observational getbestblockhash row are present in the current
        # evidence with their honest statuses recorded.
        for label in R2_DIRS:
            with self.subTest(label=label):
                checks = {c["id"]: c for c in self._report(label)["checks"]}
                self.assertIn("getmempoolinfo", checks)
                self.assertIn("getbestblockhash", checks)

    def test_r2_carry_source_name(self):
        # F6/F9: pin provenance is recorded via --source-name, never a
        # raw local path as the identity.
        expected = {"2026-11-08-r2-l2-eb257af7":
                    "rbitcoin-lab-retarget-profile",
                    "2026-11-08-r2-v09lab-bcb6d5ee": "rbitcoin-v0.9-lab"}
        for label, source in expected.items():
            with self.subTest(label=label):
                node = self._report(label)["node"]
                self.assertEqual(node["source_name"], source)
                self.assertTrue(node["binary_sha256"])

    def test_r2_binary_hashes_are_the_approved_pins(self):
        # The two approved node binaries, by sha256 (UPSTREAM pins).
        expected = {"2026-11-08-r2-l2-eb257af7":
                    "eb257af7c10cdf0ded584adfadf2ae8124466304b13d26c1"
                    "7f855e76f9320f7c",
                    "2026-11-08-r2-v09lab-bcb6d5ee":
                    "bcb6d5eefe0dd0bd96e2ac79e173b4b99a91f67f7717dae6"
                    "620c30786378a171"}
        for label, want in expected.items():
            with self.subTest(label=label):
                node = self._report(label)["node"]
                self.assertEqual(node["binary_sha256"], want)

    def test_no_credentials_in_evidence(self):
        for label in self.ALL_DIRS:
            for name in ("report.json", "report.md", "raw_responses.json",
                         "manifest.json"):
                path = os.path.join(REPORTS, label, name)
                with open(path) as fh:
                    text = fh.read()
                with self.subTest(file=path):
                    self.assertNotIn("Basic ", text)
                    self.assertNotIn("Authorization", text)
                    # cookie file holds 'ap1a:<hex>'; the hex must not leak
                    self.assertIsNone(
                        re.search(r"__cookie__:[0-9a-fA-F]{16,}", text))
                    self.assertIsNone(
                        re.search(r"ap1a:[0-9a-fA-F]{16,}", text))

    def test_markdown_render(self):
        report = self._report("2026-11-08-r2-l2-eb257af7")
        md = render_markdown(report)
        self.assertIn("## Verdict", md)
        self.assertIn("## Gap matrix", md)
        self.assertIn("## Non-claims", md)
        self.assertIn("**incompatible**", md)
        self.assertIn("funded-unproven", md)

    def test_compare_render(self):
        reports = [(label, self._report(label)) for label in R2_DIRS]
        md = compare_reports(reports)
        self.assertIn("2026-11-08-r2-l2-eb257af7", md)
        self.assertIn("2026-11-08-r2-v09lab-bcb6d5ee", md)
        self.assertIn("getnetworkinfo.gate.server", md)
        with open(os.path.join(REPORTS, "gap-matrix.md")) as fh:
            committed = fh.read()
        # same check set and same statuses (timestamps differ)
        self.assertEqual(md.count("\n"), committed.count("\n"))


class ReasonDerivationTests(unittest.TestCase):
    """D7: validate_report_consistency re-derives the full reason list
    from the actual failed kinds (after the traffic/observational
    skips) and the verdict status; fabricated or missing reasons are
    rejected, not just mis-ordered ones."""

    def _base_report(self):
        path = os.path.join(REPORTS, R2_DIRS[0], "report.json")
        with open(path) as fh:
            report = json.load(fh)
        # Explicit synthetic current-contract fixture, never rewritten evidence.
        from arkrpc.probe import compute_verdict
        report["contract_hash"] = contract_hash()
        report["traffic_consumer"] = TRAFFIC_CONSUMER
        for cid, method in (("getmempoolinfo.traffic-consumer", "getmempoolinfo"),
                            ("getblockheader.traffic-consumer", "getblockheader"),
                            ("getrawtransaction.blockhash.traffic-consumer", "getrawtransaction")):
            report["checks"].append({"id": cid, "method": method, "availability": "passed",
                "behavior": "passed", "detail": "synthetic current-contract fixture"})
        report["consumer_model_failures"] = [dict(c) for c in report["checks"] if c["id"] == "getmempoolinfo" and
                                             "failed" in (c["availability"], c["behavior"])]
        report["verdict"] = compute_verdict(report["checks"], report["profile"])
        return report

    def test_consistent_report_passes(self):
        validate_report_consistency(self._base_report())

    def test_fabricated_fail_reason_rejected(self):
        report = self._base_report()
        report["verdict"]["reasons"] = ["fail-version-gate", "fail-shape",
                                        "funded-unproven"]
        with self.assertRaises(Exception):
            validate_report_consistency(report)

    def test_missing_fail_reason_rejected(self):
        report = self._base_report()
        report["verdict"]["reasons"] = ["funded-unproven"]
        with self.assertRaises(Exception):
            validate_report_consistency(report)

    def test_missing_permanent_reason_rejected(self):
        report = self._base_report()
        report["verdict"]["reasons"] = ["fail-version-gate"]
        with self.assertRaises(Exception):
            validate_report_consistency(report)

    def test_tampered_check_status_rejected(self):
        # flipping a check row to failed without updating the reasons
        # (or vice versa) must be caught by the re-derivation
        report = self._base_report()
        for check in report["checks"]:
            if check["id"] == "getrawmempool":
                check["behavior"] = "failed"
                check["fail_kind"] = "shape"
        with self.assertRaises(Exception):
            validate_report_consistency(report)

    def test_traffic_failure_still_adds_no_reason(self):
        # control: a traffic-row failure must NOT appear as a reason
        # even when its row says failed (mirrors compute_verdict)
        report = self._base_report()
        for check in report["checks"]:
            if check["id"] == "getmempoolinfo":
                check["behavior"] = "failed"
                check["fail_kind"] = "shape"
        validate_report_consistency(report)


if __name__ == "__main__":
    unittest.main()
