"""A1/A2/A4 regressions: signed raw inputs, 100-block maturity, exact joins.

Offline construction only. No fixture result is a production acceptance pass.
"""
import copy
import importlib.util
import json
import io
import contextlib
import tempfile
import subprocess
from types import SimpleNamespace
from pathlib import Path
import unittest
from unittest.mock import patch

from oracle import Chain, Inconclusive, Invalid, decode_transaction, sha256d
from journal import Journal, parse_records
from test_oracle import block_bytes, coinbase, policy, tx_bytes
from traffic_audit import TrafficAudit, compact, digest, read_commands
from effect_counts import reconcile_node_sends
from cleanup_proof import assess_cleanup

C, A, M = "aa" * 32, "bb" * 32, "cc" * 32
PROFILE = {"source_sha256": "11" * 32, "binary_sha256": "22" * 32, "consensus_fingerprint": "33" * 32}


class FundedOutcomeRegressions(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        path = Path(__file__).resolve().parent.parent / "lab-witness" / "fixture.py"
        spec = importlib.util.spec_from_file_location("a1_test_signer", path)
        fixture = importlib.util.module_from_spec(spec); spec.loader.exec_module(fixture)
        cls.fixture_signer = fixture
        cls.script = bytes.fromhex("76a914751e76e8199196d454941c45d1b3a323f1433bd688ac")
        genesis = block_bytes(bytes(32), [coinbase(tag=b"\1\0")])
        raws = [genesis]
        for height in range(1, 102):
            cb = coinbase(outputs=[(5000000000, cls.script)], tag=b"\2" + height.to_bytes(2, "little"))
            raws.append(block_bytes(sha256d(raws[-1][:80]), [cb], 1600000000 + height))
        p = policy(genesis=sha256d(genesis[:80])[::-1].hex(), regtest_pow_bits_overlay=None)
        cls.chain = Chain(p, raws)
        funding = cls.chain.blocks[1]["transactions"][0]
        cls.point = funding["txid"] + ":0"
        outputs = [(100000, cls.script), (4999899000, cls.script)]
        signing = tx_bytes([(funding["txid_bytes"], 0, cls.script, 0xffffffff)], outputs) + b"\1\0\0\0"
        signature = fixture.ecdsa(1, sha256d(signing)) + b"\1"
        pub = fixture.pubkey(1)
        cls.raw = tx_bytes([(funding["txid_bytes"], 0, bytes([len(signature)]) + signature + bytes([len(pub)]) + pub, 0xffffffff)], outputs)
        cls.tx = decode_transaction(cls.raw)

    def history(self, outcomes):
        script, txid = self.script.hex(), self.tx["txid"]
        rows = [
            {"record": "header", "seq": 0, "format_magic": "SPC-LAB-TRAFFIC-JOURNAL", "history_version": 3,
             "protocol": "spc-lab-tx-journal/3", "network": "regtest", "key_mode": "lab_deterministic_run_seed_v1",
             "derivation_version": 2, "run_id": "run", "driver_id": "txd-0", "run_seed_fingerprint": "00" * 32,
             "allocation_digest": "11" * 32, "caps": {"unknown_retry_sends": 3, "reorg_resubmits": 3}, "created_unix_ms": 1},
            {"record": "chain_observed", "seq": 1, "tip_block_hash": self.chain.tip["hash"], "tip_height": 101,
             "displaced_block_hashes": [], "observed_unix_ms": 1},
            {"record": "funding_observed", "seq": 2, "outpoint": self.point, "amount_sats": 5000000000,
             "script_hex": script, "chain": "receive", "index": 0, "owner_role": {"kind": "bootstrap_funding"},
             "source": "bootstrap_coinbase", "context": {"canonical_block": {"block_hash": self.chain.blocks[1]["hash"], "height": 1, "coinbase": True}}, "observed_unix_ms": 1},
            {"record": "command_begin", "seq": 3, "command_id": "pay", "request": {"payer_role": {"kind": "bootstrap_funding"},
             "category": "bootstrap_distribution", "destinations": [{"script_hex": script, "amount_sats": 100000, "owner_role": None}], "fee_rate_sats_vb": 1}},
            {"record": "reserved", "seq": 4, "command_id": "pay", "outpoints": [self.point], "change_index": 0},
            {"record": "signed_tx", "seq": 5, "command_id": "pay", "txid": txid, "raw_hex": self.raw.hex(),
             "vbytes": self.tx["vbytes"], "weight_wu": self.tx["weight"], "fee_sats": 1000, "fee_rate_sats_vb": 1,
             "category": "bootstrap_distribution", "payer_role": {"kind": "bootstrap_funding"}, "spent_outpoints": [self.point],
             "destinations": [{"script_hex": script, "amount_sats": 100000, "owner_role": None}],
             "change": {"script_hex": script, "amount_sats": 4999899000, "index": 0}, "size_class": None}]
        for index, outcome in enumerate(outcomes):
            rows.append({"record": "broadcast_sent", "seq": len(rows), "command_id": "pay", "txid": txid, "send_sequence": index + 1})
            common = {"seq": len(rows), "command_id": "pay", "txid": txid}
            if outcome == "rejected":
                rows.append({**common, "record": outcome, "code": "rpc_tx_version_nonstandard"})
            elif outcome == "unknown_outcome":
                rows.append({**common, "record": outcome, "reason": "transport", "attempt": index + 1})
            elif outcome == "session_started":
                rows.append({"seq": len(rows), "record": outcome, "restarted_unix_ms": 2})
            else:
                rows.append({**common, "record": "accepted_by_rpc", "node_id": "node-a"})
        return parse_records(b"".join(compact(r) + b"\n" for r in rows))

    def audit(self, history, results, *, duplicate=False, restart=False, missing_result=False, profile=PROFILE,
              unknown_class="transport"):
        rows = []
        def put(event):
            seq = len(rows) + 1
            rows.append({"record": "traffic_audit", "seq": seq, "version": 1, "boot_id": "boot", "monotonic_ms": seq * 10, "event": event})
            return seq
        def session(inc):
            put({"event": "session", "binding": {"run_id": "run", "driver_id": "txd-0", "incarnation": inc,
                 "controller_sha256": C, "actor_sha256": A, "acceptance_enabled": True}})
        session("one")
        intents = [r for r in history if r["record"] in {"broadcast_sent", "reorg_resubmit"}]
        if duplicate:
            intents.append(intents[0])
        for index, intent in enumerate(intents):
            inc = "two" if restart and index else "one"
            if inc == "two":
                session(inc)
            seq = put({"event": "forward_attempt", "attempt": {"incarnation": inc, "rpc_request_id": 1 if inc == "two" else index + 1,
                 "node_id": "node-a", "method": "sendrawtransaction", "send_sequence": intent["send_sequence"],
                 "payload_sha256": digest(compact([self.raw.hex()])), "permit_revision": 0 if inc == "two" else index,
                 "context": {"command_id": "pay", "journal_seq": intent["seq"], "chain_revision": max(r["seq"] for r in history[:intent["seq"]] if r["record"] in {"chain_observed", "chain_rollback"}),
                 "intent_seq": intent["seq"], "raw_sha256": digest(self.raw)}}})
            if missing_result:
                break
            outcome, suppressed = results[min(index, len(results) - 1)]
            if outcome == "unknown":
                actual = {"outcome": "unknown", "class": unknown_class}
            elif outcome == "gate_closed":
                actual = {"outcome": "gate_closed"}
            elif outcome == "node_rejected":
                actual = {"outcome": outcome, "code": -26, "message": "version"}
            else:
                actual = {"outcome": "ok", "result": self.tx["txid"]}
            put({"event": "forward_result", "attempt_seq": seq, "outcome": outcome, "response_sha256": digest(compact(actual)), "response_suppressed": suppressed})
            put({"event": "watermark", "incarnation": inc, "last_attempt_seq": seq, "completed_count": 1 if inc == "two" else index + 1,
                 "permit_revision": 1 if inc == "two" else index + 1, "closed": False, "command_id": None})
        return TrafficAudit(read_commands(b"".join(compact(r) + b"\n" for r in rows)), {"txd-0": history}, run_id="run",
                            controller_sha256=C, txd_sha256=A, miner_sha256=M, node_ids=["node-a"], acceptance_enabled=True,
                            rejection_bindings={"txd-0": profile} if profile else None, expected_rejection_binding=profile)

    def replay(self, history, proof=None):
        return Journal(history, {"run_id": "run"}, {self.chain.tip["hash"]: self.chain}, proof)

    def test_A1_unknown_actual_result_cannot_release_first_send(self):
        h = self.history(["rejected"])
        with self.assertRaises(Inconclusive):
            self.replay(h)
        with self.assertRaisesRegex(Invalid, "unknown/suppressed"):
            self.audit(h, [("unknown", False)])

    def test_A1_suppressed_or_successful_actual_result_cannot_reject(self):
        h = self.history(["rejected"])
        for result in (("ok", True), ("node_rejected", True), ("ok", False)):
            with self.subTest(result=result), self.assertRaises(Invalid):
                self.audit(h, [result])

    def test_A1_missing_response_is_inconclusive_not_noeffect(self):
        with self.assertRaises(Inconclusive):
            self.audit(self.history(["rejected"]), [], missing_result=True)

    def test_A1_exact_reviewed_first_rejection_can_release(self):
        h = self.history(["rejected"])
        audit = self.audit(h, [("node_rejected", False)])
        replay = self.replay(h, audit.outcome_proofs)
        self.assertNotIn(self.point, replay.reserved)
        with self.assertRaises(Inconclusive):
            self.audit(h, [("node_rejected", False)], profile=None)

    def test_A1_prior_unknown_then_fresh_rejection_retains(self):
        h = self.history(["unknown_outcome", "rejected"])
        audit = self.audit(h, [("unknown", False), ("node_rejected", False)])
        self.assertIn(self.point, self.replay(h, audit.outcome_proofs).reserved)

    def accepted_suffix(self, history, node="node-a", txid=None):
        history.append({"record": "accepted_by_rpc", "seq": len(history), "command_id": "pay",
                        "txid": txid or self.tx["txid"], "node_id": node})
        return history

    def test_A1_late_acceptance_after_actual_unknown_without_new_send_is_rejected(self):
        h = self.accepted_suffix(self.history(["unknown_outcome"]))
        # The local state transition alone is syntactically legal; it does not
        # establish RPC acceptance or release its mature funded input.
        self.assertEqual(self.replay(h).txs[self.tx["txid"]]["state"], "accepted")
        with self.assertRaisesRegex(Invalid, "RPC acceptance lacks"):
            self.audit(h, [("unknown", False)])

    def test_A1_every_late_acceptance_requires_unsuppressed_actual_success(self):
        for outcome, suppressed, unknown_class in (("node_rejected", False, "transport"), ("ok", True, "transport"),
                ("node_rejected", True, "transport"), ("unknown", False, "protocol"),
                ("unknown", False, "shape"), ("unknown", False, "timeout"), ("gate_closed", False, "transport")):
            h = self.accepted_suffix(self.history(["unknown_outcome"]))
            with self.subTest(outcome=outcome, suppressed=suppressed, kind=unknown_class), self.assertRaisesRegex(Invalid, "RPC acceptance lacks"):
                self.audit(h, [(outcome, suppressed)], unknown_class=unknown_class)

    def test_A1_late_acceptance_cannot_borrow_another_attempt_success_or_wrong_node(self):
        h = self.accepted_suffix(self.history(["unknown_outcome", "unknown_outcome"]))
        with self.assertRaisesRegex(Invalid, "RPC acceptance lacks"):
            self.audit(h, [("ok", False), ("unknown", False)])
        for node, txid in (("node-b", self.tx["txid"]), ("node-a", "44" * 32)):
            h = self.accepted_suffix(self.history(["unknown_outcome"]), node, txid)
            with self.subTest(node=node, txid=txid), self.assertRaisesRegex(Invalid, "accepted outcome"):
                self.audit(h, [("ok", False)])

    def test_A1_late_recovery_with_real_success_and_fresh_retry_success_are_valid(self):
        h = self.accepted_suffix(self.history(["unknown_outcome"]))
        audit = self.audit(h, [("ok", False)])
        self.assertEqual(self.replay(h, audit.outcome_proofs).txs[self.tx["txid"]]["state"], "accepted")
        self.assertEqual(len(audit.attempts), 1)
        h = self.history(["unknown_outcome", "accepted_by_rpc"])
        audit = self.audit(h, [("unknown", False), ("ok", False)], restart=True)
        self.assertEqual(len(audit.send_authorizations), 2)
        self.assertEqual(self.replay(h, audit.outcome_proofs).txs[self.tx["txid"]]["state"], "accepted")
        self.assertIn(self.point, self.replay(h, audit.outcome_proofs).reserved)

    def test_A1_acceptance_before_any_intent_lacks_actual_correlated_evidence(self):
        h = self.history(["unknown_outcome"])
        h.insert(6, {"record": "accepted_by_rpc", "seq": 6, "command_id": "pay", "txid": self.tx["txid"], "node_id": "node-a"})
        for seq, row in enumerate(h):
            row["seq"] = seq
        with self.assertRaisesRegex(Inconclusive, "acceptance has no exact"):
            self.audit(h, [("unknown", False)])

    def test_A1_acceptance_proof_cannot_be_reused_after_history_node_changes(self):
        h = self.accepted_suffix(self.history(["unknown_outcome"]))
        proof = self.audit(h, [("ok", False)]).outcome_proofs
        h[-1]["node_id"] = "node-b"
        with self.assertRaisesRegex(Invalid, "prefix changed"):
            self.replay(h, proof)

    def test_A1_unknown_then_raw_confirmed_and_reorg_remains_valid(self):
        h = self.history(["unknown_outcome"])
        # Reconstruct the already validated offline vector chain from each
        # block's header and exact transaction bytes.
        from oracle import compact_size
        raws = [b["raw"] + compact_size(len(b["transactions"])) + b"".join(t["raw"] for t in b["transactions"]) for b in self.chain.blocks]
        cb = coinbase(outputs=[(5000001000, self.script)], tag=b"\2\x66\0")
        raw = block_bytes(bytes.fromhex(self.chain.tip["hash"])[::-1], [cb, self.raw], 1600000102)
        high = Chain(self.chain.policy, raws + [raw])
        h.append({"record": "chain_observed", "seq": len(h), "tip_block_hash": high.tip["hash"], "tip_height": 102,
                  "displaced_block_hashes": [], "observed_unix_ms": 2})
        h.append({"record": "confirmed", "seq": len(h), "command_id": "pay", "txid": self.tx["txid"], "block_hash": high.tip["hash"], "height": 102})
        anchor = lambda c: {"block_hash": c.tip["hash"], "height": c.tip["height"]}
        proof = {"version": 1, "old_anchor": anchor(high), "new_anchor": anchor(self.chain), "common_ancestor": anchor(self.chain),
                 "old_headers": [high.tip["raw"].hex(), self.chain.tip["raw"].hex()], "new_headers": [self.chain.tip["raw"].hex()],
                 "displaced_block_hashes": [high.tip["hash"]]}
        h.append({"record": "chain_rollback", "seq": len(h), "evidence": proof, "observed_unix_ms": 3})
        h.append({"record": "reorged", "seq": len(h), "command_id": "pay", "txid": self.tx["txid"], "from_block_hash": high.tip["hash"], "height": 102})
        audit = self.audit(h, [("unknown", False)])
        replay = Journal(h, {"run_id": "run"}, {self.chain.tip["hash"]: self.chain, high.tip["hash"]: high}, audit.outcome_proofs)
        self.assertEqual(replay.txs[self.tx["txid"]]["state"], "reorged")
        self.assertIn(self.point, replay.reserved)

    def test_A1_outcome_proof_cannot_be_reused_for_changed_history(self):
        h = self.history(["rejected"])
        proof = self.audit(h, [("node_rejected", False)]).outcome_proofs
        h[0]["caps"]["unknown_retry_sends"] = 2
        with self.assertRaisesRegex(Invalid, "prefix changed"):
            self.replay(h, proof)

    def test_A2_same_intent_double_forward_same_or_new_incarnation(self):
        h = self.history(["unknown_outcome"])
        for restart in (False, True):
            with self.subTest(restart=restart), self.assertRaisesRegex(Invalid, "authorization forwarded more than once"):
                self.audit(h, [("unknown", False)], duplicate=True, restart=restart)

    def test_A2_fresh_retry_intent_after_restart_is_valid(self):
        h = self.history(["session_started", "accepted_by_rpc"])
        audit = self.audit(h, [("ok", False), ("ok", False)], restart=True)
        self.assertEqual(len(audit.send_authorizations), 2)
        self.assertIn(self.point, self.replay(h, audit.outcome_proofs).reserved)

    def test_A2_node_counts_are_exact_and_unlisted_extra_is_refused(self):
        h = self.history(["accepted_by_rpc"])
        audit = self.audit(h, [("ok", False)])
        actual = {"node-a": [{"method": "sendrawtransaction", "params": [self.raw.hex()]}]}
        self.assertEqual(reconcile_node_sends(audit, actual)["node_observed_sends"], 1)
        actual["node-a"].append(actual["node-a"][0])
        with self.assertRaisesRegex(Invalid, "unlisted or duplicate"):
            reconcile_node_sends(audit, actual)

    def test_A2_only_fully_traced_witness_channel_can_add_effect(self):
        audit = self.audit(self.history(["accepted_by_rpc"]), [("ok", False)])
        fixture = self.fixture_signer
        raw = fixture.spend("p2wpkh", "12" * 32, 0, 100000, fixture.script("p2wpkh"), 1000)
        txid = decode_transaction(raw)["txid"]
        trace = [
            {"seq": 1, "kind": "test_only_witness_send", "payload": {"actor": "witness-fixture", "kind": "p2wpkh", "command_id": "fixture-1", "node_id": "node-a", "raw_hex": raw.hex(), "txid": txid}},
            {"seq": 2, "kind": "node_rpc", "payload": {"node": "node-a", "request": {"id": 1, "method": "sendrawtransaction", "params": [raw.hex()]}, "response": {"id": 1, "result": txid, "error": None}}},
            {"seq": 3, "kind": "test_only_witness_result", "payload": {"actor": "witness-fixture", "command_id": "fixture-1", "node_id": "node-a", "txid": txid, "intent_seq": 1, "rpc_seq": 2}}]
        references = [{"intent_seq": 1, "rpc_seq": 2, "completion_seq": 3}]
        actual = {"node-a": [{"method": "sendrawtransaction", "params": [r.hex()]} for r in (self.raw, raw)]}
        self.assertEqual(reconcile_node_sends(audit, actual, trace, references)["explicit_fixture_sends"], 1)
        with self.assertRaises(Invalid):
            reconcile_node_sends(audit, actual, trace, [])
        trace[-1]["payload"]["rpc_seq"] = 99
        with self.assertRaises(Invalid):
            reconcile_node_sends(audit, actual, trace, references)


class CleanupRegressions(unittest.TestCase):
    def fixture(self):
        manifest = {"run_id": "run", "placement": None, "resources": {"listeners": {}, "postgres_databases": [], "redis_instances": []},
                    "processes": [{"role": "node", "entity": "node-a", "pid": 100, "start_time_ticks": 1000,
                    "pgid": 100, "run_token": "test-only", "executable_path": "/test/node", "executable_sha256": "ab" * 32}]}
        identities = {("node", "node-a"): {"status": "gone"}}
        resources = {"schema": "spc-lab-owned-resource-cleanup/1", "run_id": "run", "status": "PASSED",
                     "process_groups_checked": [100], "owned_group_survivors": [], "unknown_group_members": [],
                     "remaining_unix_listeners": [], "remaining_unattributed_host_listeners": [], "namespace_holders": []}
        return manifest, identities, resources

    def test_A4_nonzero_reap_no_survivors_is_failed(self):
        m, ids, resources = self.fixture()
        self.assertEqual(assess_cleanup(1, b"[]", m, m, ids, resources)["status"], "FAILED")

    def test_A4_successful_complete_reap_and_resources_pass(self):
        m, ids, resources = self.fixture()
        self.assertEqual(assess_cleanup(0, b'[["node","node-a","exited"]]', m, m, ids, resources)["status"], "PASSED")

    def test_A4_zero_exit_survivor_fails(self):
        m, ids, resources = self.fixture()
        ids[("node", "node-a")] = {"status": "surviving"}
        self.assertEqual(assess_cleanup(0, b'[["node","node-a","stopped"]]', m, m, ids, resources)["status"], "FAILED")

    def test_A4_missing_identity_resource_or_manifest_is_inconclusive(self):
        m, ids, resources = self.fixture()
        output = b'[["node","node-a","exited"]]'
        for args in ((0, output, None, m, ids, resources), (0, output, m, m, {}, resources), (0, output, m, m, ids, None)):
            with self.subTest(args=args):
                self.assertEqual(assess_cleanup(*args)["status"], "INCONCLUSIVE")
        ids[("node", "node-a")] = {"status": "unknown"}
        self.assertEqual(assess_cleanup(0, output, m, m, ids, resources)["status"], "INCONCLUSIVE")

    def test_A4_resource_status_string_cannot_hide_live_listener(self):
        m, ids, resources = self.fixture()
        resources["remaining_unix_listeners"] = [{"path": "broker/control.sock", "socket_inode": "123"}]
        self.assertEqual(assess_cleanup(0, b'[["node","node-a","exited"]]', m, m, ids, resources)["status"], "FAILED")

    def test_A4_controller_failure_paths_emit_and_close_receipt(self):
        here = Path(__file__).resolve().parent
        spec = importlib.util.spec_from_file_location("cleanup_controller_regression", here / "regtest-traffic-e2e.py")
        module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
        parent = "/tmp/opencode" if Path("/tmp/opencode").is_dir() else None
        for failure in ("success", "nonzero", "timeout", "missing_manifest", "exception"):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory(prefix="cleanup-regression-", dir=parent) as directory:
                args = SimpleNamespace(run_root=str(Path(directory) / "owned"), timeout=10, case="bootstrap-stop", witness=False,
                                       inputs=str(here / "inputs.template.json"), scenario=str(here / "scenario.json"))
                controller = module.Controller(args)
                controller.process = SimpleNamespace(pid=123456789, returncode=0, poll=lambda: 0)
                controller.binaries = {"lab": "/sealed/lab"}
                manifest, _, resource_receipt = self.fixture()
                controller.manifest = lambda: manifest
                if failure == "missing_manifest":
                    controller.manifest = lambda: None
                elif failure == "exception":
                    def broken():
                        raise OSError("owned metadata unavailable")
                    controller.manifest = broken
                error = subprocess.TimeoutExpired(["reap"], 35) if failure == "timeout" else None
                completed = subprocess.CompletedProcess(["reap"], 0 if failure == "success" else 1,
                                                       b'[["node","node-a","exited"]]', b"")
                with patch.object(module.subprocess, "run", side_effect=error, return_value=completed), \
                     patch("cleanup_proof.identity_observation", return_value={"status": "gone"}), \
                     patch("cleanup_proof.collect_resource_receipt", return_value=resource_receipt), contextlib.redirect_stdout(io.StringIO()):
                    module.finalize_controller(controller)
                receipt = json.loads((controller.root / "acceptance.json").read_bytes())
                expected = "PASSED" if failure == "success" else ("FAILED" if failure == "nonzero" else "INCONCLUSIVE")
                self.assertEqual(receipt["cases"]["cleanup"]["status"], expected)
                self.assertTrue(controller.capture.stream.closed)


if __name__ == "__main__":
    unittest.main()
