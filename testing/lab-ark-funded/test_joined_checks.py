import copy
import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from common import Blocked, artifact, load, sha, verify_source, validate_pool_control_admission, check_bundle
from joined_checks import UI_FILES, cargo_command, validate_joined_features
from common import POOL_CONTROL_R4_FILES, POOL_CONTROL_R4_REVIEW


class JoinedChecks(unittest.TestCase):
    def test_r4_source_layer_requires_exact_review_code_and_approved_rows(self):
        policy = {"source_snapshot_sha256": "d00299c259aaa5e8098542f48f5c3c90c66737be3f965f5d65e5fd2a763914b2",
                  "tested_code_snapshot_sha256": "f764900419db45901f558c367b5485938ed1b20b5ee5abdc039e3b66c230ae0c",
                  "same_auditor_review": copy.deepcopy(POOL_CONTROL_R4_REVIEW), "files": dict(POOL_CONTROL_R4_FILES)}
        # Layer eligibility only. Canonical bundle admission separately requires
        # actual code presence, all Nix rows/root, binaries and feature evidence.
        validate_pool_control_admission(policy)
        changed = copy.deepcopy(policy)
        changed["same_auditor_review"]["sha256"] = "00" * 32
        with self.assertRaises(Blocked):
            validate_pool_control_admission(changed)
        changed = copy.deepcopy(policy)
        changed["files"]["bins/spc-pool/src/ark/wallet_control.rs"] = "00" * 32
        with self.assertRaises(Blocked):
            validate_pool_control_admission(changed)

    def test_dd9r3_tested_interface_cannot_admit_restart_defective_source(self):
        policy = {"source_snapshot_sha256": "dd9f0c37d7e79474a56132170272fcd6509eb3ebdc501aaa93372e8dd8a4102b",
                  "compiled": True, "tests_passed": True}
        with self.assertRaisesRegex(Blocked, "admission held"):
            validate_pool_control_admission(policy)

    def test_new_hash_does_not_bypass_pending_corrected_source_and_recheck(self):
        with self.assertRaisesRegex(Blocked, "same-auditor recheck"):
            validate_pool_control_admission({"source_snapshot_sha256": "ab" * 32, "compiled": True, "tests_passed": True})

    def test_default_48_agent_tests_are_not_ark_wallet_coverage(self):
        with self.assertRaisesRegex(Blocked, "explicit Ark-feature"):
            cargo_command(["cargo", "test", "-p", "spc-agent"], "test", "spc-agent", ark=True)
        cargo_command(["cargo", "test", "-p", "spc-agent", "--features", "ark"], "test", "spc-agent", ark=True)
        for option in ("--no-run", "--list", "--ignored"):
            with self.assertRaises(Blocked):
                cargo_command(["cargo", "test", "-p", "spc-agent", "--features=ark", option], "test", "spc-agent", ark=True)

    def test_joined_feature_source_binary_and_complete_ui_gate(self):
        # Consumer fixtures only: these logs are not actual compiled evidence.
        with tempfile.TemporaryDirectory(prefix="ap5-joined-", dir="/tmp/opencode") as tmp:
            root = Path(tmp)
            log = root / "fixture.log"
            log.write_text("test wallet::audit_regressions::unit ... ok\ntest ark::unit ... ok\ntest result: ok. 2 passed; 0 failed\n")
            log_ref = {"path": str(log), "sha256": sha(log)}
            source_hash = "11" * 32
            bundle = {"joined_source_manifest": {"sha256": source_hash},
                      "binaries": {n: {"sha256": (n + "-fixture")} for n in ("agent", "pool", "lab")}}
            checks = []
            for name in ("agent", "pool"):
                for verb, suffix in (("test", "tests"), ("build", "build")):
                    checks.append({"purpose": f"{name}-ark-{suffix}", "argv": ["cargo", verb, "-p", "spc-" + name, "--features", "ark"],
                                   "exit": 0, "source_manifest_sha256": source_hash, "log": log_ref, "binary_sha256": name + "-fixture"})
            checks += [{"purpose": "ui-build", "argv": ["cargo", "build", "-p", "spc-lab-cli"], "exit": 0,
                        "source_manifest_sha256": source_hash, "log": log_ref, "binary_sha256": "lab-fixture"},
                       {"purpose": "ui-browser-tests", "argv": ["node", "fixture.cjs"], "exit": 0,
                        "source_manifest_sha256": source_hash, "log": log_ref, "tested_binary_sha256": "lab-fixture",
                        "required_cases": ["base", "wallet"], "passed_cases": ["base", "wallet"]}]
            files = {}
            for name in UI_FILES:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("fixture:" + name)
                files[name] = sha(path)
            validation = {"checks": checks, "ui_source_files": files}
            manifest = {"files": [{"path": k, "sha256": v} for k, v in files.items()]}
            validate_joined_features(validation, bundle, root, manifest)
            for field, value in (("exit", 1), ("tested_binary_sha256", "old-binary"), ("passed_cases", ["base"]),
                                 ("source_manifest_sha256", "old-source")):
                changed = copy.deepcopy(validation)
                changed["checks"][-1][field] = value
                with self.assertRaises(Blocked):
                    validate_joined_features(changed, bundle, root, manifest)
            missing = copy.deepcopy(validation)
            del missing["ui_source_files"]["crates/spc-lab/src/http.rs"]
            with self.assertRaisesRegex(Blocked, "base/http/DOM"):
                validate_joined_features(missing, bundle, root, manifest)
            (root / "bins/spc-lab/static/app.js").write_text("old omitted DOM")
            with self.assertRaisesRegex(Blocked, "actual Nix build source is stale"):
                validate_joined_features(validation, bundle, root, manifest)

    def test_exact_six_row_lineage_preserves_core_guards(self):
        here = Path(__file__).parent
        policy = load(here / "critical-sources.json")
        lineage = load(artifact(policy["source_lineage_amendment"]))
        expected_paths = {"crates/spc-lab-txd/src/" + name + ".rs" for name in ("actor", "admit", "builder")}
        expected_paths |= {"bins/spc-agent/src/wallet/" + name + ".rs" for name in ("live_receive", "maintenance", "mod")}
        self.assertEqual(set(lineage["files"]), expected_paths)
        self.assertEqual(len(policy["files"]), 13)
        for name, value in lineage["files"].items():
            self.assertEqual(policy["files"][name], value)
        for ref in lineage["manifests"]:
            artifact(ref)
        self.assertEqual(policy["files"]["bins/spc-agent/src/wallet/audit_regressions.rs"], "d59b3d81ec11176b05bf0ce0da697dc628a3ab4a54901445e14e31c8fadb2777")
        self.assertEqual(policy["files"]["bins/spc-agent/src/wallet/round.rs"], "10da12d69435470e4920fa8dfb8bbfbf64f24918f4dd7b3597957b23f8eac3f6")
        self.assertEqual(policy["files"]["bins/spc-agent/src/wallet/scan.rs"], "19b4b33b1d728a6e3effce3e1fdd5f0cc90c00b360c1f22fdf71fd4d7f2434dd")

    def test_actual_old_w52_source_still_fails_core_guard(self):
        old = Path("/nix/store/vm95bb22061a626nd1si133xc9prjz0r-source")
        # The preserved known-bad source may be absent on another QA host.
        # verify_source still fails closed for either absent or stale bytes.
        policy = load(Path(__file__).parent / "critical-sources.json")["files"]
        manifest = {"files": [{"path": k, "sha256": v} for k, v in policy.items()]}
        with self.assertRaises(Blocked):
            verify_source(old, manifest, policy)


if __name__ == "__main__":
    unittest.main()
