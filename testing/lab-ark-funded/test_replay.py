import base64
import copy
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from cleanup import REQUIRED_LISTENERS, REQUIRED_ROLES, coverage_errors, validate_cleanup
from common import Blocked, Evidence, encoded, sha
from component_verify import final_verdict, replay_component, mandatory_components, native_funding_verdict
from scenario import USERS, POOLS
from raw_verify import parse_block, parse_tx
from test_raw_verify import tx, unit_block
from verify_archive import read_archive, verify


def cleanup_fixture():
    rows = []
    for role, count in REQUIRED_ROLES.items():
        for _ in range(count):
            rows.append({"pid": 10000 + len(rows), "start_time_ticks": 123, "role": role, "run_token": "unit-only"})
    resources = {"listeners": {name: {"host": "127.0.0.1", "port": 20000 + i} for i, name in enumerate(sorted(REQUIRED_LISTENERS))},
                 "postgres_databases": [{"port": 22000}, {"port": 22001}],
                 "redis_instances": [{"port": 23000}, {"port": 23001}]}
    endpoints = list(resources["listeners"].values()) + [{"host": "127.0.0.1", "port": r["port"]} for g in ("postgres_databases", "redis_instances") for r in resources[g]]
    inv = {"active_manifest": {"processes": rows, "resources": resources}, "coverage_errors": [],
           "observed_socket_inodes": [], "observed_unix_paths": [], "run_tokens": ["unit-only"], "observed_file_inodes": [],
           "processes": [{**r, "status": "gone_or_reused"} for r in rows],
           "listeners": [{"kind": "planned_tcp_endpoint", **e, "released": True} for e in endpoints],
           "file_references": [], "locks": [], "capture_errors": [], "unreadable_same_uid_processes": []}
    return {"schema": "spc.lab.ap5.cleanup/v2", "inventory": inv, "inventory_sha256": hashlib.sha256(encoded(inv)).hexdigest(),
            "exhaustive": True, "all_resources_released": True, "status": "passed"}


def reseal_cleanup(c):
    c["inventory_sha256"] = hashlib.sha256(encoded(c["inventory"])).hexdigest()


def policy_fixture():
    raw = {"network": "regtest", "serverPubkey": base64.b64encode(b"s" * 33).decode(),
           "mailboxPubkey": base64.b64encode(b"m" * 33).decode(), "fees": {},
           "vtxoExitDelta": 144, "maxVtxoExitDepth": 100}
    return {"raw_response_hex": encoded(raw).hex(), "ark_info": raw,
            "bound": {"server_pubkey": (b"s" * 33).hex(), "mailbox_pubkey": (b"m" * 33).hex(),
                      "fees_sha256": hashlib.sha256(encoded(raw["fees"])).hexdigest()}}


def transaction_fixture():
    parent_raw = tx(value=120000100)
    parent_block = unit_block([parent_raw])
    parent_id = parse_tx(parent_raw)["txid"]
    raw = tx(parent_id, value=120000000)
    block = unit_block([tx(value=5000000000), raw])
    return {"raw": raw, "txid": parse_tx(raw)["txid"], "raw_block": block,
            "block_hash": parse_block(block)["hash"], "height": 101, "fee_sats": 100,
            "inputs": [{"raw": parent_raw, "txid": parent_id, "vout": 0, "amount_sats": 120000100,
                        "raw_block": parent_block, "block_hash": parse_block(parent_block)["hash"], "height": 1}]}


class CleanupReplay(unittest.TestCase):
    def test_complete_fixture_then_omitted_actor_and_endpoint(self):
        c = cleanup_fixture()
        self.assertTrue(validate_cleanup(c))
        for field in ("processes", "listeners"):
            changed = copy.deepcopy(c)
            changed["inventory"][field].pop()
            reseal_cleanup(changed)
            with self.assertRaises(Blocked):
                validate_cleanup(changed)

    def test_empty_or_truncated_census_cannot_claim_exhaustive(self):
        for manifest in ({}, {"processes": [], "resources": {}}):
            c = cleanup_fixture()
            c["inventory"]["active_manifest"] = manifest
            c["inventory"]["coverage_errors"] = coverage_errors(manifest)
            reseal_cleanup(c)
            with self.assertRaises(Blocked):
                validate_cleanup(c)
        c = cleanup_fixture()
        c["inventory"]["active_manifest"]["processes"] = [r for r in c["inventory"]["active_manifest"]["processes"] if r["role"] != "tx_driver"]
        c["inventory"]["coverage_errors"] = coverage_errors(c["inventory"]["active_manifest"])
        reseal_cleanup(c)
        with self.assertRaises(Blocked):
            validate_cleanup(c)

    def test_zombies_descriptors_locks_denials_and_listener_contradictions(self):
        for field, value in (("file_references", [{"pid": 1}]), ("locks", ["lock"]),
                             ("capture_errors", [{"kind": "denied"}]), ("unreadable_same_uid_processes", [1])):
            c = cleanup_fixture()
            c["inventory"][field] = value
            reseal_cleanup(c)
            with self.assertRaises(Blocked):
                validate_cleanup(c)
        for field, key, value in (("processes", "status", "zombie_unreaped"), ("listeners", "released", False)):
            c = cleanup_fixture()
            c["inventory"][field][0][key] = value
            reseal_cleanup(c)
            with self.assertRaises(Blocked):
                final_verdict([], set(), c, {})


class ComponentReplay(unittest.TestCase):
    def replay(self, data, identity=None):
        with tempfile.TemporaryDirectory(prefix="ap5-replay-", dir="/tmp/opencode") as tmp:
            path = Path(tmp) / "component.json"
            path.write_bytes(encoded(data))
            return replay_component(Path(tmp), {"id": identity or data["id"], "evidence": {"file": path.name, "sha256": sha(path)}}, None)

    def funding(self):
        transaction = transaction_fixture()
        return {"schema": "spc.lab.ap5.raw-component/v2", "id": "fund-pool-a", "maturity": 100,
                "ark_info": policy_fixture(), "transactions": [[transaction, copy.deepcopy(transaction)]], "vtxos": [],
                "payments": [{"txid": transaction["txid"], "script": "51", "sats": 120000000}]}

    def test_raw_funding_and_destination_negative(self):
        d = self.funding()
        self.assertEqual(self.replay(d)["raw_transactions"][0]["fee_sats"], 100)
        d["payments"][0]["sats"] = 120000001
        with self.assertRaises(Blocked):
            self.replay(d)

    def test_funding_cannot_be_relabelled_as_lifecycle_or_confirmed(self):
        d = self.funding()
        with self.assertRaises(Blocked):
            self.replay(d, "receive-alice")
        for identity in ("receive-alice", "board-pool-a", "sp-refresh-alice", "second-renewal-alice", "stock-offboard-alice"):
            d["id"] = identity
            with self.assertRaises(Blocked):
                self.replay(d)

    def test_raw_policy_server_mailbox_and_fee_mutation(self):
        for field in ("server_pubkey", "mailbox_pubkey", "fees_sha256"):
            d = self.funding()
            d["ark_info"]["bound"][field] = "00" * 32
            with self.assertRaises(Blocked):
                self.replay(d)

    def test_missing_raw_crypto_proof_never_uses_confirmed_label(self):
        d = self.funding()
        d.update(id="board-pool-a", board={"state": "confirmed"}, vtxos=[{}])
        with self.assertRaises((Blocked, KeyError)):
            self.replay(d)

    def test_verified_vtxo_must_still_match_component_provider_policy(self):
        d = self.funding()
        d.update(id="receive-alice", vtxos=[{"raw_hex": "00", "anchor_views": [], "expected": {}}], payout_attribution=[{"status": "sent"}])
        # Isolate post-crypto consumer validation. This is not a signature test.
        with patch("component_verify.public_vtxo", return_value={"id": "11" * 32 + ":0", "server_pubkey": "wrong"}):
            with self.assertRaisesRegex(Blocked, "bound server policy"):
                self.replay(d)


class ArchiveReplay(unittest.TestCase):
    def test_archive_replay_keeps_partial_diagnostic_inconclusive(self):
        with tempfile.TemporaryDirectory(prefix="ap5-archive-", dir="/tmp/opencode") as tmp:
            root = Path(tmp)
            source = root / "source.json"
            source.write_bytes(b"{}")
            ref = {"path": str(source), "sha256": sha(source)}
            bundle = {"binaries": {}, "joined_source_manifest": ref, "joined_validation_receipt": ref,
                       "vtxo_verifier": None, "pool_wallet_executor": None}
            bundle["probe_invocation"] = json.loads(Path("/tmp/opencode/AP5-probe-and-pid-handoff-r1/probe-invocation.json").read_text())
            bundle_path = root / "bundle.json"
            bundle_path.write_bytes(encoded(bundle))
            bindings = {**bundle, "bundle_sha256": sha(bundle_path),
                        "bundle_artifact": {"path": str(bundle_path), "sha256": sha(bundle_path)}}
            cleanup = cleanup_fixture()
            cleanup["inventory"]["processes"].append({"pid": 9999, "start_time_ticks": 42, "role": "controller", "status": "gone_or_reused"})
            reseal_cleanup(cleanup)
            ev = Evidence(root)
            probe = bundle["probe_invocation"]
            ev.save("probe-invocation", probe)
            ev.save("launch", {"argv": ["unit-runtime", "--traffic-probe", probe["program"]["path"], "--traffic-probe-arg", probe["prefix_args"][0]]})
            ev.save("active-resource-census", cleanup["inventory"]["active_manifest"])
            ev.save("cleanup-inventory", cleanup)
            verdict = native_funding_verdict([], cleanup, bindings)
            path = ev.save("native-funding-verdict", verdict)
            ev.save("result", {"schema": "spc.lab.ap5.result/v2", "scope": "confirmed-funding", "diagnostic_status": "inconclusive",
                               "funded_ark_lifecycle": False, "whole_lab_complete": False,
                               "native_funding_verdict": {"file": path.name, "sha256": sha(path)}})
            ev.close()
            seal = root / "seal.json"
            seal.write_bytes(encoded({"root": str(root), "events_sha256": sha(ev.path), "last_seq": ev.sequence, "last_digest": ev.previous}))
            # Source/Nix admission has separate tests; this fixture exercises
            # the actual archive/cleanup/result consumer without a built bundle.
            with patch("verify_archive.check_bundle") as admission:
                result = verify(seal, sha(seal))
                admission.assert_called_once_with(bundle, full=False)
            self.assertEqual(result["diagnostic_status"], "inconclusive")
            self.assertFalse(result["whole_lab_complete"])
            self.assertEqual(set(result["missing"]), {"fund-server-alice", "fund-server-bob", "fund-pool-a", "fund-pool-b"})

    def test_external_seal_and_artifact_mutation_and_truncated_tail(self):
        with tempfile.TemporaryDirectory(prefix="ap5-seal-", dir="/tmp/opencode") as tmp:
            root = Path(tmp)
            ev = Evidence(root)
            path = ev.save("result", {"schema": "spc.lab.ap5.result/v2", "status": "prepared-not-executed"})
            ev.close()
            seal = root / "seal.json"
            seal.write_bytes(encoded({"root": str(root), "events_sha256": sha(ev.path), "last_seq": ev.sequence, "last_digest": ev.previous}))
            digest = sha(seal)
            self.assertEqual(len(read_archive(seal, digest)[1]), 1)
            with self.assertRaises(Blocked):
                read_archive(seal, "00" * 32)
            original = path.read_bytes()
            path.write_bytes(b"{}")
            with self.assertRaises(Blocked):
                read_archive(seal, digest)
            path.write_bytes(original)
            ev.path.write_bytes(ev.path.read_bytes()[:-1])
            with self.assertRaises(Blocked):
                read_archive(seal, digest)


if __name__ == "__main__":
    unittest.main()
