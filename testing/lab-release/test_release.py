#!/usr/bin/env python3
"""Dependency-free negative and frozen-reference tests for the release gate."""

import copy
import hashlib
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from independent_checks import check_namespace_artifacts, check_static_reference, compare_profile
from namespace_replay import prove_namespace_chain
from release_lib import (
    OwnedProcess, RunDirectory, ValidationError, approval_decision, canonical_snapshot,
    load_json, secure_tree, token_survivors, verify_source_manifest,
)

CONTRACT = load_json(HERE / "contract.json")
REAL = Path("/tmp/opencode/LP-network-kimi-final-live-v2")
REAL_V09 = Path("/tmp/opencode/LP-v09-live-namespace-r2-rt")
FIXTURE = load_json(HERE / "fixtures" / "namespace-v09-structure.json")


class ReleaseTests(unittest.TestCase):
    def expect_code(self, code, callback):
        with self.assertRaises(ValidationError) as caught:
            callback()
        self.assertEqual(caught.exception.code, code)
        return caught.exception

    def test_real_approvals_are_bound_to_exact_passes(self):
        locations = {
            "pool": ("/home/ethan/code/silent-pool/docs/reviews/2026-11-05-lab-lp1-fable-review.md",
                     "c16032a78902e88b0157efed6780bc8b424e942b1c3bf46893d8fb09ae4708ee"),
            "miner": ("/home/ethan/code/silent-pool/docs/reviews/2026-11-05-lab-lp2-fable-review.md",
                      "5e27e9180e765e8049cbcccbcadc38b24c85d402505108f8f20c320f73961250"),
            "l2": ("/home/ethan/code/silent-pool/docs/reviews/2026-11-05-lab-l2-fable-review.md",
                   "d2f37b9113e8855245866289ed393043aed6e6153e8101375072cbb23624d499"),
            "lp_live": ("/home/ethan/code/silent-pool/docs/reviews/2026-11-06-lab-live-fable-review.md",
                        "aa3b37b67960071ec6bd4f09c7cd0ac83e1b526404231994c42dc331dc44ded2"),
            "runtime": ("/home/ethan/code/silent-pool/docs/reviews/2026-11-06-lab-network-runtime-fable-review.md",
                        "553cd4e8bb40d34be514a4ebd55d466904e1357f174b2b184b5dcf8724aa5692"),
            "node_v09": ("/home/ethan/code/silent-pool/docs/reviews/2026-11-06-lab-v09-port-fable-review.md",
                         "0e037d358c057d382adcca8d6c094bcf4a46eb1e02758777d20f5d70eb4ddcda"),
        }
        for name, (path, snapshot) in locations.items():
            with self.subTest(name=name):
                self.assertTrue(approval_decision(Path(path).read_text(), snapshot)["approved"])
        runtime_text = Path(locations["runtime"][0]).read_text()
        self.assertTrue(approval_decision(runtime_text, "2d6c752d24d4f4cc9ea30f6d5ced8b507dfd860acb8721ded85c758992705098")["approved"])
        runtime = approval_decision(runtime_text, locations["runtime"][1])
        self.assertEqual(runtime["approval_sections"], ["# LP-network-runtime (namespaced two-pool mining lab): Fable review"])
        lp = approval_decision(Path(locations["lp_live"][0]).read_text(), locations["lp_live"][1])
        self.assertEqual(lp["approval_sections"], ["# Third pass (same day, same reviewer)"])
        all_rejections = {
            "pool": ["81fa17d27ee80641167aaeb53e97f627ba8daa4688e56a227a1ef44c9776b329",
                     "69ac72b6ed7ab514f1870cfc55832d8c8d202576a9c01bc27aaeb72d2a20da6d"],
            "miner": ["af5313704b81ee20868d634dc19127c97687a8d89b541e00c9d8d105a120e635",
                      "bcb9b34280fb8d21b0f5d10d07af1a18a4e4de8cc8e148bc6c519d21d24849bb",
                      "6e2eb2aa6537187bc78fb9ba822f0a564f895c4a3e8be26d85fc12ed3b146676"],
            "l2": ["646656a84cbb5001f96559d98511499eaba35501d93038dc9560cd787bc764ff"],
        }
        for name, hashes in all_rejections.items():
            text = Path(locations[name][0]).read_text()
            for snapshot in hashes:
                with self.subTest(previous_rejection=name, snapshot=snapshot):
                    self.assertFalse(approval_decision(text, snapshot)["approved"])

    def test_approval_negative_fences(self):
        target = "a" * 64
        other = "b" * 64
        self.assertFalse(approval_decision("# Review\n## Result\n**APPROVE** unrelated text\n", target)["approved"])
        self.assertFalse(approval_decision(f"# Review\n## Result\n**APPROVE**, tied to snapshot `{other}`\n", target)["approved"])
        revoked = f"# Pass\n## Result\n**APPROVE**, tied to snapshot `{target}`\n# Later\n## Result\n**REQUEST_CHANGES**, tied to snapshot `aaaaaaaa`\n"
        self.assertFalse(approval_decision(revoked, target)["approved"])
        self.assertFalse(approval_decision(revoked, other)["approved"])
        explicit = f"# Third pass\nSnapshot under review:\n`{target}`\n## Result\n**APPROVE - bounded**, tied to snapshot `aaaaaaaa...aaaa`\n"
        self.assertTrue(approval_decision(explicit, target)["approved"])
        # Fable's two multi-binding counterexamples: neither A nor B may bind.
        conflicting_history = (f"# Pass\nThe first-pass snapshot_sha256 `{target}` was rejected elsewhere.\n"
                               f"## Result\n**APPROVE**, tied to snapshot `bbbbbbbb`\n")
        self.assertFalse(approval_decision(conflicting_history, target)["approved"])
        self.assertFalse(approval_decision(conflicting_history, other)["approved"])
        conflicting_review = (f"# Pass\nSnapshot under review: `{target}`\n"
                              f"## Result\n**APPROVE**, tied to snapshot `{other}`\n")
        self.assertFalse(approval_decision(conflicting_review, target)["approved"])
        self.assertFalse(approval_decision(conflicting_review, other)["approved"])
        multiple = (f"# Pass\n## Result\n**APPROVE**, tied to snapshot `{target}`\n"
                    f"**APPROVE**, tied to snapshot `{other}`\n")
        self.assertFalse(approval_decision(multiple, target)["approved"])
        self.assertFalse(approval_decision(multiple, other)["approved"])
        fenced = (f"# Pass\n## Result\n```text\n**APPROVE**, tied to snapshot `aaaaaaaa`\n```\n"
                  f"{target}   (matches)\n")
        self.assertFalse(approval_decision(fenced, target)["approved"])
        collision = target[:8] + "cc" * 28
        self.assertFalse(approval_decision(explicit, collision)["approved"])
        self.assertFalse(approval_decision(revoked, target)["approved"])

    def test_old_l2_approval_never_binds_new_candidate(self):
        text = Path("/home/ethan/code/silent-pool/docs/reviews/2026-11-05-lab-l2-fable-review.md").read_text()
        self.assertFalse(approval_decision(text, CONTRACT["release"]["official_commit"].ljust(64, "0"))["approved"])

    def test_source_manifest_detects_content_and_symlink_drift(self):
        with tempfile.TemporaryDirectory(prefix="LP-v09-live-unit-manifest-", dir="/tmp/opencode") as temporary:
            root = Path(temporary) / "source"
            root.mkdir()
            (root / "a.txt").write_text("approved")
            (root / "b.txt").write_text("second")
            manifest = {"base": "test", "files": [
                {"path": "a.txt", "sha256": hashlib.sha256(b"approved").hexdigest()},
                {"path": "b.txt", "sha256": hashlib.sha256(b"second").hexdigest()},
            ]}
            manifest["snapshot_sha256"] = canonical_snapshot(manifest)
            path = Path(temporary) / "manifest.json"
            path.write_text(json.dumps(manifest))
            record = verify_source_manifest(root, path, manifest["snapshot_sha256"])
            self.assertTrue(record["verified"])
            (root / "a.txt").write_text("changed")
            self.expect_code("source_drift", lambda: verify_source_manifest(root, path, manifest["snapshot_sha256"]))
            (root / "a.txt").write_text("approved")
            (root / "b.txt").unlink()
            (root / "b.txt").symlink_to(root / "a.txt")
            self.expect_code("manifest_path", lambda: verify_source_manifest(root, path, manifest["snapshot_sha256"]))

    def test_profile_missing_is_not_accepted(self):
        expected = {"network": "regtest", "pow_target_timespan": 240}
        self.expect_code("profile_mismatch", lambda: compare_profile({"network": "regtest"}, {
            "network": "regtest", "pow_target_timespan": "missing"}, "negative"))
        with self.assertRaises(ValidationError):
            compare_profile({"network": "regtest"}, expected, "missing-field")

    def test_static_pinned_reference(self):
        with tempfile.TemporaryDirectory(prefix="LP-v09-live-static-", dir="/tmp/opencode") as temporary:
            destination = Path(temporary) / "audit"
            shutil.copytree("/tmp/opencode/v09-audit", destination)
            result = check_static_reference(destination, CONTRACT)
            self.assertEqual(result["status"], "passed")
            self.assertFalse(result["candidate_port_proven"])
            changed = destination / "v090" / "prim-lib.rs"
            changed.write_text(changed.read_text().replace("SCHEMA_VERSION: u16 = 26", "SCHEMA_VERSION: u16 = 25"))
            self.expect_code("static_input_drift", lambda: check_static_reference(destination, CONTRACT))

    def namespace_expected(self):
        expected = dict(CONTRACT["namespace"])
        expected["binary_hashes"] = dict(expected["binary_hashes"], node="eb257af7c10cdf0ded584adfadf2ae8124466304b13d26c17f855e76f9320f7c")
        return expected

    def test_existing_namespace_raw_reference_and_no_fork_negative(self):
        if not REAL.exists():
            self.skipTest("preserved runtime fixture is absent")
        source = Path(CONTRACT["namespace"]["source_dir"] if "source_dir" in CONTRACT["namespace"] else "/tmp/opencode/LP-network-runtime-final-src")
        scenario = source / CONTRACT["namespace"]["scenario"]
        observed = check_namespace_artifacts(REAL, scenario, self.namespace_expected())
        self.assertEqual(observed["status"], "passed")
        self.assertEqual(observed["no_bypass_checks"], 276)
        self.assertNotEqual(observed["fork_views"][0]["hash"], observed["fork_views"][1]["hash"])
        with tempfile.TemporaryDirectory(prefix="LP-v09-live-net-negative-", dir="/tmp/opencode") as temporary:
            root = Path(temporary) / "runtime"
            (root / "shareable").mkdir(parents=True)
            (root / "private").mkdir()
            shutil.copyfile(REAL / "run-manifest.json", root / "run-manifest.json")
            shutil.copyfile(REAL / "shareable" / "result.json", root / "shareable" / "result.json")
            for name in ["scenario.json", "evidence.jsonl", "node-api-node-a.jsonl", "node-api-node-b.jsonl"]:
                shutil.copyfile(REAL / "private" / name, root / "private" / name)
            shutil.copytree(REAL / "logs", root / "logs")
            result_path = root / "shareable" / "result.json"
            result = load_json(result_path)
            phase = next(row for row in reversed(result["topology"]["phases"]) if row["phase"] == "quiesce-before-heal")
            phase["nodes"][1]["hash"] = phase["nodes"][0]["hash"]
            result_path.write_text(json.dumps(result))
            self.expect_code("namespace_no_real_fork", lambda: check_namespace_artifacts(root, scenario, self.namespace_expected()))

    def namespace_expected_v09(self):
        expected = self.namespace_expected()
        expected["binary_hashes"] = dict(expected["binary_hashes"], node="bcb6d5eefe0dd0bd96e2ac79e173b4b99a91f67f7717dae6620c30786378a171")
        return expected

    def test_committed_v09_namespace_structure_and_raw_run(self):
        # Always runs without external artifacts: this is the new v0.9 chain,
        # not merely the old runtime fixture above.
        proof = prove_namespace_chain(copy.deepcopy(FIXTURE))
        self.assertEqual(proof["status"], "passed")
        self.assertEqual(proof["common_anchor"]["height"], 7)
        self.assertNotEqual(proof["fork_hashes_at_common_height"]["node-a"],
                            proof["fork_hashes_at_common_height"]["node-b"])
        self.assertEqual(len(proof["heal"]["reannounced_chain"]), 8)  # seven displaced blocks plus the winning tip at 15
        self.assertEqual(proof["heal"]["displacement"]["height"], 13)
        if REAL_V09.exists():
            # Optional full-artifact coverage, not the sole unit fixture.
            scenario = Path("/tmp/opencode/LP-network-runtime-final-src") / CONTRACT["namespace"]["scenario"]
            observed = check_namespace_artifacts(REAL_V09, scenario, self.namespace_expected_v09())
            self.assertEqual(observed["chain_proof"]["common_anchor"]["height"], 7)
            self.assertEqual(observed["node_api"][0]["forbidden_methods"], [])

    def test_namespace_one_block_lag_is_not_a_fork(self):
        # The exact Fable counterexample: node-b is one block behind on A's
        # chain; unequal final heights/tip hashes do not establish a fork.
        fixture = copy.deepcopy(FIXTURE)
        phase = next(row for row in reversed(fixture["phases"]) if row["phase"] == "quiesce-before-heal")
        a_tip = next(row for row in fixture["tips"]["node-a"] if row["height"] == 14)
        phase["nodes"][1]["height"] = 14
        phase["nodes"][1]["hash"] = a_tip["block_hash"]
        self.expect_code("namespace_no_real_fork", lambda: prove_namespace_chain(fixture))

    def test_namespace_displacement_only_before_partition_is_rejected(self):
        fixture = copy.deepcopy(FIXTURE)
        partition = next(row for row in fixture["links"] if row["policy"] == "partition")
        fixture["displacements"] = [{"event_id": 10, "wall_ms": partition["wall_ms"] - 1,
                                     "node": "node-b", "height": 3,
                                     "block_hash": "00" * 32, "anchored_tip": "00" * 32}]
        self.expect_code("namespace_reorg_evidence", lambda: prove_namespace_chain(fixture))

    def test_namespace_missing_ancestor_and_chain_hole_are_unknown(self):
        missing = copy.deepcopy(FIXTURE)
        common = next(row for row in missing["phases"] if row["phase"] == "quiesce-before-partition")["nodes"][0]
        del missing["headers"][common["hash"]]
        error = self.expect_code("namespace_chain_ancestor_unknown", lambda: prove_namespace_chain(missing))
        self.assertEqual(error.outcome, "unknown")
        hole = copy.deepcopy(FIXTURE)
        preheal = next(row for row in reversed(hole["phases"]) if row["phase"] == "quiesce-before-heal")
        del hole["headers"][preheal["nodes"][0]["hash"]]
        error = self.expect_code("namespace_chain_hole", lambda: prove_namespace_chain(hole))
        self.assertEqual(error.outcome, "unknown")

    def test_namespace_cross_node_confusion_is_not_a_fork(self):
        fixture = copy.deepcopy(FIXTURE)
        phase = next(row for row in reversed(fixture["phases"]) if row["phase"] == "quiesce-before-heal")
        phase["nodes"][1] = dict(phase["nodes"][0], node="node-b")
        self.expect_code("namespace_no_real_fork", lambda: prove_namespace_chain(fixture))

    def test_namespace_fork_outside_phase_is_not_promoted(self):
        fixture = copy.deepcopy(FIXTURE)
        partition = next(row for row in fixture["links"] if row["policy"] == "partition")
        partition["wall_ms"] = max(row["wall_ms"] for records in fixture["tips"].values() for row in records) + 1000000
        self.expect_code("namespace_no_real_fork", lambda: prove_namespace_chain(fixture))

    def test_owned_token_cleanup_and_modes(self):
        with tempfile.TemporaryDirectory(prefix="LP-v09-live-owner-", dir="/tmp/opencode") as temporary:
            root = RunDirectory(Path(temporary))
            process = OwnedProcess("sleep", ["sleep", "30"], root.root, root.private / "sleep.log", root.token)
            self.assertTrue(process.owned())
            process.signal(__import__("signal").SIGTERM)
            process.popen.wait(timeout=5)
            process.close_log()
            self.assertEqual(token_survivors(root.token), [])
            (root.private / "secret").write_text("x")
            os.chmod(root.private / "secret", 0o644)
            secured = secure_tree(root.root)
            self.assertEqual((root.private / "secret").stat().st_mode & 0o777, 0o600)
            self.assertGreaterEqual(secured["file_count"], 1)

    def test_subdirectory_name_cannot_escape_run_prefix(self):
        with tempfile.TemporaryDirectory(prefix="LP-v09-live-dir-guard-", dir="/tmp/opencode") as temporary:
            self.expect_code("run_dir", lambda: RunDirectory(Path(temporary) / "nested" / "run"))


if __name__ == "__main__":
    unittest.main(verbosity=2)
