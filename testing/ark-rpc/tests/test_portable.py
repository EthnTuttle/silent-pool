"""Source-location regressions; no node, actor or Rust build is started."""
import hashlib
import importlib
import importlib.util
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
CONTRACT = "754e1e6131955f9e5bbce7ef81b6f9aabaa2f1379ec6409f1c3a7a9e88be91f9"


class PortableSourceTests(unittest.TestCase):
    def test_fixture_resolves_its_own_source_location(self):
        spec = importlib.util.spec_from_file_location("portable_lifecycle", ROOT / "lifecycle_fixture.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        self.assertEqual(module.CANONICAL_PROBE_ROOT, ROOT)
        self.assertTrue((module.CANONICAL_PROBE_ROOT / "probe.py").is_file())

    def test_import_closure_and_public_fixtures_are_local(self):
        for name in ("__init__", "cli", "client", "contracts", "launchprofile", "nodeproc",
                     "probe", "reporting", "schemadef", "selftest", "trafficfacts"):
            module = importlib.import_module("arkrpc" if name == "__init__" else "arkrpc." + name)
            self.assertTrue(Path(module.__file__).resolve().is_relative_to(ROOT))
        from arkrpc.contracts import contract_hash
        from arkrpc.selftest import FIXTURES_DIR
        self.assertEqual(contract_hash(), CONTRACT)
        self.assertEqual(Path(FIXTURES_DIR).resolve(), ROOT / "fixtures")
        self.assertTrue((Path(FIXTURES_DIR) / "base_responses.json").is_file())
        self.assertTrue((Path(FIXTURES_DIR) / "scenarios/traffic_selected_consumer.json").is_file())

    def test_cli_from_unrelated_cwd_and_empty_home(self):
        with tempfile.TemporaryDirectory(prefix="portable-probe-") as directory:
            env = {"HOME": directory, "TMPDIR": directory, "PYTHONDONTWRITEBYTECODE": "1"}
            result = subprocess.run([sys.executable, "-B", str(ROOT / "probe.py"), "contract-show"],
                                    cwd=directory, env=env, capture_output=True, timeout=10, check=True)
            body, printed_hash = result.stdout.rsplit(b"\n# contract sha256: ", 1)
            self.assertEqual(hashlib.sha256(body).hexdigest(), CONTRACT)
            self.assertEqual(printed_hash.strip(), CONTRACT.encode())
            help_result = subprocess.run([sys.executable, "-B", str(ROOT / "lifecycle_fixture.py"), "--help"],
                                         cwd=directory, env=env, capture_output=True, timeout=10, check=True)
            self.assertIn(b"--compat", help_result.stdout)


if __name__ == "__main__":
    unittest.main()
