import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from ap5_observation import inspect, USERS
from campaign import encode
from oracle import Invalid

BASE = None


class AP5ObservationTests(unittest.TestCase):
    def archive(self, root, status="funded-lifecycle-observed-awaiting-independent-oracles", omit=None):
        common = {"binary_hashes": {n: "11" * 32 for n in ("lab", "node", "pool", "miner", "scanner", "txd")},
                  "binary_source_manifests": {n: "22" * 32 for n in ("lab", "node", "pool", "miner", "scanner", "txd")}}
        values = [("verified-bundle", {"source_manifest_sha256": "22" * 32,
                  "bundle": {"schema": "spc.lab.ap5.bundle/v1", "binaries": {n: {"sha256": h} for n, h in common["binary_hashes"].items()}}})]
        labels = {"declared-default", "native-funding-confirmed", "boarded-float-pool-a", "boarded-float-pool-b"}
        labels |= {prefix + user for user in USERS for prefix in ("sp-owned-received-", "mining-payout-attribution-",
            "sp-round-confirmed-", "round-chain-proof-", "automatic-second-renewal-", "second-round-chain-",
            "stock-offboard-confirmed-", "sp-offboard-confirmed-", "sp-offboard-chain-proof-", "wallet-final-")}
        values += [(label, {}) for label in sorted(labels) if label != omit]
        values += [("cleanup", {"all_owned_actors_stopped": True, "survivors": []}),
                   ("result", {"schema": "spc.lab.ap5.result/v1", "scope": "full", "status": status,
                               "native_funding_confirmed": True, "funded_ark_lifecycle": True, "whole_lab_complete": False})]
        previous, lines = "0" * 64, []
        for seq, (label, value) in enumerate(values, 1):
            filename = f"{seq:06d}-{label}.json"
            raw = encode(value)
            (root / filename).write_bytes(raw)
            row = {"seq": seq, "file": filename, "sha256": hashlib.sha256(raw).hexdigest(), "previous": previous, "unix_ms": seq}
            previous = hashlib.sha256(encode(row).rstrip(b"\n")).hexdigest()
            lines.append(encode({**row, "record_sha256": previous}))
        raw = b"".join(lines)
        (root / "events.jsonl").write_bytes(raw)
        descriptor = root / "collector.json"
        descriptor.write_bytes(encode({"root": str(root), "events_sha256": hashlib.sha256(raw).hexdigest(),
                                       "last_seq": len(lines), "last_digest": previous}))
        return descriptor, common

    def test_actual_producer_terminal_observation_is_not_independent_whole_lab_pass(self):
        with tempfile.TemporaryDirectory(prefix="t3-ap5-records-", dir=BASE) as tmp:
            descriptor, common = self.archive(Path(tmp))
            result = inspect(descriptor, common)
            self.assertTrue(result["lifecycle_observed"])
            self.assertEqual(result["status"], "INCONCLUSIVE")
            self.assertTrue(result["missing_components"])

    def test_missing_user_lifecycle_or_funding_only_never_passes(self):
        for status, omit in (("confirmed-native-transfers-only", None),
                             ("funded-lifecycle-observed-awaiting-independent-oracles", "automatic-second-renewal-dave")):
            with tempfile.TemporaryDirectory(prefix="t3-ap5-missing-", dir=BASE) as tmp:
                descriptor, common = self.archive(Path(tmp), status, omit)
                result = inspect(descriptor, common)
                self.assertEqual(result["status"], "INCONCLUSIVE")
                self.assertFalse(result["lifecycle_observed"])

    def test_artifact_tail_and_joined_binary_substitutions_fail(self):
        with tempfile.TemporaryDirectory(prefix="t3-ap5-binding-", dir=BASE) as tmp:
            root = Path(tmp)
            descriptor, common = self.archive(root)
            common["binary_hashes"]["pool"] = "33" * 32
            with self.assertRaisesRegex(Invalid, "different joined binaries"):
                inspect(descriptor, common)
            common["binary_hashes"]["pool"] = "11" * 32
            path = root / "events.jsonl"
            path.write_bytes(path.read_bytes().splitlines(keepends=True)[0])
            with self.assertRaisesRegex(Invalid, "parent seal"):
                inspect(descriptor, common)

    def test_cleanup_failure_dominates_lifecycle_flags(self):
        with tempfile.TemporaryDirectory(prefix="t3-ap5-cleanup-", dir=BASE) as tmp:
            descriptor, common = self.archive(Path(tmp), "cleanup-failed")
            self.assertEqual(inspect(descriptor, common)["status"], "FAILED")


if __name__ == "__main__":
    unittest.main()
