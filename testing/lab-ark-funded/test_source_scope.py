"""Canonical check_bundle tests: real file hashes, simulated Nix metadata/mount.

No admission validator is mocked. The six fixture app paths are a real Nix Python
executable; Nix metadata, one source mount and trusted qualification data are simulated.
These are consumer fixtures, not receipts that the real joined bundle passed.
"""
import copy
import hashlib
import json
from pathlib import Path
import shutil
import sys
import tempfile
import unittest
from unittest.mock import patch

import common
from ap5 import Controller
from common import Blocked, Evidence, encoded, sha
from joined_checks import UI_FILES
from scenario import scenario
from source_scope import APPS, CONTROL_PATHS, LIMITS, NATIVE_COMPONENTS, POOL_BASE, runtime_bindings

SOURCE_ID = "/nix/store/00000000000000000000000000000000-ap5-unit-source"


class BundleFixture:
    def __init__(self, root, control=False):
        self.root, self.source, self.control = root, root / "source", control
        self.source.mkdir()
        policy = json.loads((Path(__file__).parent / "critical-sources.json").read_text())
        self.critical = policy["files"]
        # r2: primary source is the preserved pre-union legacy snapshot with
        # the four re-frozen agent-wallet rows overlaid at committed union
        # HEAD bytes (consumer fixture source, not a receipt; the legacy
        # no-control class cannot be built from the union tree itself).
        primary = Path("/tmp/opencode/AP5-source-lineage-r2/fixture-primary-source")
        for name in POOL_BASE | UI_FILES | set(self.critical):
            dest = self.source / name
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(primary / name, dest)
            if name in self.critical:
                assert sha(dest) == self.critical[name], "test needs approved immutable critical bytes: " + name
        if control:
            for name in CONTROL_PATHS:
                dest = self.source / name
                dest.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(Path("/tmp/opencode/pool-wallet-control-packet-r3/source") / name, dest)
        bindings = json.loads(Path("/tmp/opencode/self-hosted-ark-packet/package-bindings.json").read_text())["outputs"]
        self.binaries = {name: self.ref(Path(sys.executable)) for name in (*APPS, "grpcurl", "redis-server", "redis-cli")}
        for name, group, exe in (("node", "node_release", "rbitcoin-node"), ("captaind", "captaind", "captaind"), ("probe", "probe", "ark-rpc-probe")):
            self.binaries[name] = self.ref(Path(bindings[group]["executables"][exe]["path"]))
        self.pg = Path(bindings["postgres"]["output"]) / "bin"
        self.binaries["psql"] = self.ref(self.pg / "psql")
        log = root / "unit-only.log"
        log.write_text("CONSUMER FIXTURE, NOT A REAL COMPILED GATE\ntest wallet::audit_regressions::unit ... ok\ntest ark::board::tests::matured_output_boards_only_the_shortfall ... ok\ntest result: ok. 332 passed; 0 failed\n")
        self.log = self.ref(log)
        self.nix_calls = []
        self.publish()

    @staticmethod
    def ref(path):
        return {"path": str(path), "sha256": sha(path)}

    def write(self, name, data):
        p = self.root / name
        p.write_bytes(encoded(data))
        return self.ref(p)

    def publish(self, change_checks=None):
        rows = [{"path": str(p.relative_to(self.source)), "sha256": sha(p)} for p in sorted(self.source.rglob("*")) if p.is_file()]
        manifest = self.write("manifest.json", {"files": rows})
        checks = []
        for name in ("agent", "pool"):
            for verb, suffix in (("test", "tests"), ("build", "build")):
                checks.append({"purpose": f"{name}-ark-{suffix}", "argv": ["cargo", verb, "-p", "spc-" + name, "--features", "ark"],
                               "source_manifest_sha256": manifest["sha256"], "exit": 0, "log": self.log,
                               "binary_sha256": self.binaries[name]["sha256"]})
        checks += [{"purpose": "ui-build", "argv": ["cargo", "build", "-p", "spc-lab-cli"], "exit": 0, "log": self.log,
                    "source_manifest_sha256": manifest["sha256"], "binary_sha256": self.binaries["lab"]["sha256"]},
                   {"purpose": "ui-browser-tests", "argv": ["node", "compiled_browser.cjs"], "exit": 0, "log": self.log,
                    "source_manifest_sha256": manifest["sha256"], "tested_binary_sha256": self.binaries["lab"]["sha256"],
                    "required_cases": ["base", "wallet"], "passed_cases": ["base", "wallet"]}]
        if change_checks:
            change_checks(checks)
        validation = {"schema": "spc.lab.ap5.join-validation/v1", "source_manifest_sha256": manifest["sha256"],
                      "binaries": {n: self.binaries[n]["sha256"] for n in APPS}, "checks": checks,
                      "ui_source_files": {name: sha(self.source / name) for name in UI_FILES}}
        validated = self.write("validation.json", validation)
        pool_files = {r["path"]: r["sha256"] for r in rows if r["path"].startswith("bins/spc-pool/")}
        gate = self.write("gate.json", {"schema": "spc.lab.ap5.joined-332-gate/v1", "source_manifest_sha256": manifest["sha256"],
                          "checks_sha256": hashlib.sha256(encoded(checks)).hexdigest(), "passed": 332, "failed": 0, "evidence": [self.log]})
        coverage = self.write("maintenance.json", {"schema": "spc.lab.ap5.legacy-maintenance-coverage/v1", "source_manifest_sha256": manifest["sha256"],
            "pool_files": pool_files, "covered_paths": ["initial-sync", "periodic-maintenance", "matured-coinbase-refill", "failure-retry"],
            "check_purpose": "pool-ark-tests", "log": self.log, "conclusion": "supported-for-bounded-native-funding-observation"})
        self.bundle = {"schema": "spc.lab.ap5.bundle/v1", "scope": "confirmed-funding", "base_revision": "unit-only",
            "joined_source_manifest": manifest, "joined_validation_receipt": validated, "binaries": self.binaries,
            "postgres_bin_dir": str(self.pg), "history_version": 3, "bootstrap_and_workers_single_budget": True,
            "host_peering_before_profile_binding": True, "fresh_registration_before_workers": True}
        self.bundle["probe_invocation"] = json.loads(Path("/tmp/opencode/AP5-probe-and-pid-handoff-r1/probe-invocation.json").read_text())
        self.policy = {"schema": "spc.lab.ap5.confirmed-funding-source-policy/v1", "scope": "confirmed-funding",
            "source_class": "wallet-control" if self.control else "legacy-no-wallet-control", "source_root": SOURCE_ID,
            "source_manifest_sha256": manifest["sha256"], "joined_validation_sha256": validated["sha256"],
            "app_binaries": {n: self.binaries[n]["sha256"] for n in APPS}, "pool_files": pool_files,
            "absent_control_paths": [] if self.control else list(CONTROL_PATHS), "absent_control_executables": [] if self.control else ["pool-wallet-executor"],
            "joined_gate": gate, "limits": copy.deepcopy(LIMITS), "runtime_bindings": runtime_bindings(self.bundle),
            "legacy_maintenance": {"mode": "enabled-source-tested-observation-only", "max_runtime_secs": 600,
                "native_transfer_boarding": "not-in-legacy-height-scan", "automatic_coinbase_refill": "may-run-no-ark-acceptance", "coverage": coverage}}
        self.update_policy()
        if self.control:
            self.control_policy = {"source_root": SOURCE_ID, "source_manifest_sha256": manifest["sha256"], "files": pool_files,
                "source_snapshot_sha256": "dd9f0c37d7e79474a56132170272fcd6509eb3ebdc501aaa93372e8dd8a4102b", "compiled": True, "tests_passed": True}
            self.bundle["pool_control_source_policy"] = self.write("control.json", self.control_policy)

    def update_policy(self):
        self.bundle["diagnostic_source_policy"] = self.write("diagnostic.json", self.policy)

    def check(self, full=False, split_root=False):
        real_path = Path
        def mount(value, *args):
            return self.source if str(value) == SOURCE_ID and not args else real_path(value, *args)
        def nix(argv, **kwargs):
            self.nix_calls.append(argv)
            if argv[:3] == ["nix-store", "-q", "--deriver"]:
                return "/nix/store/unit-joined.drv\n"
            if argv[:3] == ["nix", "derivation", "show"]:
                src = SOURCE_ID if not split_root or len(self.nix_calls) <= 2 else SOURCE_ID + "-different"
                return json.dumps({"/nix/store/unit-joined.drv": {"env": {"src": src}}})
            raise AssertionError(argv)
        registry = {'schema': 'spc.lab.ap5.joined-gate-qualifications/v1', 'qualified_350_sources': {},
                    'historical_332_sources': {self.bundle['joined_source_manifest']['sha256']: {'source_root': SOURCE_ID}}}
        with patch("common.Path", side_effect=mount), patch("common.subprocess.check_output", side_effect=nix), patch('gate_inventory.qualification_registry', return_value=registry):
            return common.check_bundle(self.bundle, full=full)


class SourceApplicableAdmission(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="ap5-scope-", dir="/tmp/opencode")
        self.addCleanup(self.tmp.cleanup)
        self.fixture = BundleFixture(Path(self.tmp.name))

    def test_canonical_diagnostic_passes_actual_hash_root_feature_and_gate_checks(self):
        f = self.fixture
        paths, manifest, receipt = f.check()
        self.assertEqual(paths["node"], Path(f.binaries["node"]["path"]))
        self.assertEqual(sha(manifest), f.bundle["joined_source_manifest"]["sha256"])
        self.assertEqual(sha(receipt), f.bundle["joined_validation_receipt"]["sha256"])
        self.assertEqual(len(f.nix_calls), 12)  # All six real deriver/root code paths.
        with self.assertRaisesRegex(Blocked, "full AP5"):
            f.check(full=True)

    def test_350_count_cannot_admit_old_source_or_unfinished_gate(self):
        f = self.fixture
        gate = json.loads((f.root / 'gate.json').read_text())
        gate.update(schema='spc.lab.ap5.joined-350-gate/v1', passed=350)
        f.policy['joined_gate'] = f.write('gate.json', gate)
        f.update_policy()
        with self.assertRaisesRegex(Blocked, 'completed source-qualified 350'):
            f.check()

    def test_old_agent_cannot_hide_behind_new_no_control_label(self):
        f = self.fixture
        (f.source / "bins/spc-agent/src/wallet/audit_regressions.rs").write_text("old w52 agent")
        f.publish()
        with self.assertRaisesRegex(Blocked, "approved critical source"):
            f.check()

    def test_executed_probe_requires_listed_program_script_source_and_exact_prefix(self):
        f = self.fixture
        good = copy.deepcopy(f.bundle["probe_invocation"])
        del f.bundle["probe_invocation"]
        with self.assertRaisesRegex(Blocked, "declare the executed probe"):
            f.check()
        for field in ("program", "script", "source_manifest"):
            f.bundle["probe_invocation"] = copy.deepcopy(good)
            f.bundle["probe_invocation"][field]["sha256"] = "00" * 32
            with self.assertRaises(Blocked):
                f.check()
        f.bundle["probe_invocation"] = copy.deepcopy(good)
        f.bundle["probe_invocation"]["prefix_args"].append("node-probe")
        with self.assertRaisesRegex(Blocked, "unsupported probe prefix"):
            f.check()

    def test_split_derivers_wrong_source_and_missing_policy_fail(self):
        f = self.fixture
        with self.assertRaisesRegex(Blocked, "not a joined build"):
            f.check(split_root=True)
        f.policy["source_root"] = SOURCE_ID + "-wrong"
        f.update_policy()
        with self.assertRaisesRegex(Blocked, "different source/root/gate"):
            f.check()
        del f.bundle["diagnostic_source_policy"]
        with self.assertRaisesRegex(Blocked, "exact tested diagnostic"):
            f.check()

    def test_unrelated_dd9_policy_cannot_be_attached_to_absent_code(self):
        f = self.fixture
        f.bundle["pool_control_source_policy"] = f.write("unrelated.json", {"source_snapshot_sha256": "dd9f0c37"})
        with self.assertRaisesRegex(Blocked, "unrelated control policy"):
            f.check()

    def test_dd9_present_in_diagnostic_still_requires_corrected_admission(self):
        f = self.fixture
        for name in CONTROL_PATHS:
            p = f.source / name
            p.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(Path("/tmp/opencode/pool-wallet-control-packet-r3/source") / name, p)
        f.control = True
        f.publish()
        with self.assertRaisesRegex(Blocked, "admission held"):
            f.check()
        f.control_policy["source_root"] += "-unrelated"
        f.bundle["pool_control_source_policy"] = f.write("control.json", f.control_policy)
        with self.assertRaisesRegex(Blocked, "different actual source/root"):
            f.check()
        del f.bundle["pool_control_source_policy"]
        with self.assertRaisesRegex(Blocked, "control code present"):
            f.check()

    def test_undeclared_code_or_client_and_unknown_source_class_fail(self):
        f = self.fixture
        f.policy["source_class"] = "latest-any-source"
        f.update_policy()
        with self.assertRaisesRegex(Blocked, "source class mismatch"):
            f.check()
        f.policy["source_class"] = "legacy-no-wallet-control"
        f.update_policy()
        f.bundle["pool_wallet_executor"] = f.binaries["pool"]
        with self.assertRaisesRegex(Blocked, "declared control executable"):
            f.check()
        del f.bundle["pool_wallet_executor"]
        (f.source / CONTROL_PATHS[0]).write_text("new unbound code")
        with self.assertRaisesRegex(Blocked, "source census"):
            f.check()

    def test_332_ark_ui_and_legacy_maintenance_evidence_are_required(self):
        f = self.fixture
        f.publish(lambda checks: checks[0].update(argv=["cargo", "test", "-p", "spc-agent"]))
        with self.assertRaisesRegex(Blocked, "explicit Ark-feature"):
            f.check()

        f.publish(lambda checks: checks[-1].update(passed_cases=["base"]))
        with self.assertRaisesRegex(Blocked, "browser case census"):
            f.check()
        f.publish()
        gate = json.loads((f.root / "gate.json").read_text());gate["passed"] = 48
        f.policy["joined_gate"] = f.write("gate.json", gate);f.update_policy()
        with self.assertRaisesRegex(Blocked, "exact successful joined 332"):
            f.check()
        f.publish()
        coverage = json.loads((f.root / "maintenance.json").read_text());coverage["covered_paths"] = ["initial-sync"]
        f.policy["legacy_maintenance"]["coverage"] = f.write("maintenance.json", coverage);f.update_policy()
        with self.assertRaisesRegex(Blocked, "reachable legacy maintenance paths"):
            f.check()

    def test_diagnostic_cannot_relax_registration_budget_probe_or_declared_scope(self):
        f = self.fixture
        for field in ("fresh_registration_before_workers", "bootstrap_and_workers_single_budget", "host_peering_before_profile_binding"):
            f.bundle[field] = False
            with self.assertRaises(Blocked):
                f.check()
            f.bundle[field] = True
        f.policy["runtime_bindings"]["probe_sha256"] = "00" * 32
        f.update_policy()
        with self.assertRaisesRegex(Blocked, "node/probe/budget"):
            f.check()
        f.publish()
        del f.bundle["scope"]
        with self.assertRaisesRegex(Blocked, "scope must be explicit"):
            f.check()

    def test_controller_and_verdict_cannot_escalate_diagnostic_scope(self):
        f = self.fixture
        paths, _, _ = f.check()
        root = Path(self.tmp.name) / "run-evidence";root.mkdir()
        ev = Evidence(root)
        self.addCleanup(ev.close)
        c = Controller(f.bundle, paths, root, ev, scenario("unit", 3), 100, scope="confirmed-funding")
        for fn in (c.board, c.lifecycle, lambda: c.pool_wallet("pool-a", "board_float"), lambda: c.pool_wallet("pool-a", "board_proof_v1"),
                   lambda: c.wallet_command("alice", [], {}, "x"), lambda: c.command("x", {"type": "restart_role"}),
                   lambda: c.component("float-pool-a")):
            with self.assertRaises(Blocked):fn()
        with patch("ap5.final_verdict", side_effect=AssertionError("full AP5 verdict forbidden")):
            result = c.verdict({})
        self.assertEqual(result["schema"], "spc.lab.ap5.native-funding-verdict/v1")
        self.assertEqual(set(result["missing"]), NATIVE_COMPONENTS)
        self.assertFalse(result["ark_lifecycle_ready"])
        with self.assertRaisesRegex(Blocked, "runtime exceeds"):
            Controller(f.bundle, paths, root, ev, scenario("unit", 3), 601, scope="confirmed-funding")
