import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import tomllib
import unittest


VERIFIER = Path(__file__).parent / "verifier"


class VerifierSources(unittest.TestCase):
    def test_frozen_lock_audit_rejects_checksum_or_version_drift(self):
        lock = VERIFIER / "Cargo.lock"
        self.assertEqual(hashlib.sha256(lock.read_bytes()).hexdigest(), "ac24ee3b09c7c2dd8731417672b9d7b072aa6e9e04127b9c96e8ad80ae842833")
        packages = tomllib.loads(lock.read_text())["package"]
        ark = next(p for p in packages if p["name"] == "ark-lib")
        self.assertEqual(ark["version"], "0.7.1")
        self.assertTrue(ark["source"].endswith("#c419159fa54085572f8ca72d2ba1716556a9d3eb"))
        argv = [sys.executable, "-B", str(VERIFIER / "lock_seed.py"), "audit", "--workspace-lock",
                "/tmp/opencode/AP5-verifier-lock-r1/parent-Cargo.lock", "--workspace-sha256",
                "307a71fe5d279320a74ad06310d0bac79d9c7b4d4cb1a0b14af7e0940553d0c5", "--lock"]
        subprocess.run([*argv, str(lock)], check=True, capture_output=True)
        with tempfile.TemporaryDirectory(prefix="ap5-lock-negative-", dir="/tmp/opencode") as tmp:
            changed = Path(tmp) / "Cargo.lock"
            original = lock.read_text()
            checksum = next(p["checksum"] for p in packages if "checksum" in p)
            for text in (original.replace(checksum, "00" * 32, 1), original.replace('version = "0.7.1"', 'version = "0.0.0"', 1)):
                changed.write_text(text)
                self.assertNotEqual(subprocess.run([*argv, str(changed)], capture_output=True).returncode, 0)

    def test_public_corpus_is_exactly_six_full_proofs_two_anchors_no_private_fields(self):
        corpus = json.loads((VERIFIER / "fixtures/stock-public.json").read_text())
        self.assertEqual(corpus["fixture_kind"], "synthetic_signed_conformance")
        self.assertEqual(corpus["source_sha256"], "7926e40e613fb9817d3710f78b34ae5d19eff6dc2aced25be25a9f37831417fa")
        self.assertEqual(set(corpus["public"]), {"anchor_tx", "round_tx", "board_vtxo", "arkoor_htlc_out_vtxo", "arkoor2_vtxo", "round1_vtxo", "round2_vtxo", "arkoor3_vtxo"})
        for name, raw in corpus["public"].items():
            self.assertEqual(hashlib.sha256(bytes.fromhex(raw)).hexdigest(), corpus["public_sha256"][name])
        self.assertIn("not a live chain", corpus["provenance"])


if __name__ == "__main__":
    unittest.main()
