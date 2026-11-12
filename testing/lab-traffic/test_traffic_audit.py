"""Actual traffic_audit/1 serde-shape fixtures; offline adapter tests only."""
import copy
import unittest

from oracle import Inconclusive, Invalid, decode_transaction
from test_oracle import tx_bytes
from traffic_audit import TrafficAudit, compact, digest, miner_wire_bytes, read_commands

C, A, M = "aa" * 32, "bb" * 32, "cc" * 32


def row(seq, event):
    return {"record": "traffic_audit", "seq": seq, "version": 1, "boot_id": "boot",
            "monotonic_ms": seq * 10, "event": event}


def session(incarnation="inc"):
    return {"event": "session", "binding": {"run_id": "run", "driver_id": "txd-0", "incarnation": incarnation,
            "controller_sha256": C, "actor_sha256": A, "acceptance_enabled": True}}


class AuditTests(unittest.TestCase):
    def setUp(self):
        self.raw = tx_bytes()
        self.txid = decode_transaction(self.raw)["txid"]
        self.history = [
            {"record": "header", "seq": 0, "run_id": "run", "driver_id": "txd-0"},
            {"record": "chain_observed", "seq": 1},
            {"record": "command_begin", "seq": 2, "command_id": "pay"},
            {"record": "reserved", "seq": 3, "command_id": "pay"},
            {"record": "signed_tx", "seq": 4, "command_id": "pay", "txid": self.txid, "raw_hex": self.raw.hex()},
            {"record": "broadcast_sent", "seq": 5, "command_id": "pay", "txid": self.txid, "send_sequence": 1},
            {"record": "accepted_by_rpc", "seq": 6, "command_id": "pay", "txid": self.txid, "node_id": "node-a"},
            {"record": "send_barrier", "seq": 7, "command_id": "pause", "send_sequence": 1, "monotonic_ms": 55},
            {"record": "send_resumed", "seq": 8, "command_id": "resume"},
        ]
        self.attempt = {"event": "forward_attempt", "attempt": {
            "incarnation": "inc", "rpc_request_id": 1, "node_id": "node-a", "method": "sendrawtransaction",
            "send_sequence": 1, "payload_sha256": digest(compact([self.raw.hex()])), "permit_revision": 0,
            "context": {"command_id": "pay", "journal_seq": 5, "chain_revision": 1, "intent_seq": 5, "raw_sha256": digest(self.raw)}}}

    def result(self, attempt=2, suppressed=False):
        return {"event": "forward_result", "attempt_seq": attempt, "outcome": "ok",
                "response_sha256": digest(compact({"outcome": "ok", "result": self.txid})), "response_suppressed": suppressed}

    def watermark(self, revision, last=2, completed=1, closed=False, command=None):
        return {"event": "watermark", "incarnation": "inc", "last_attempt_seq": last, "completed_count": completed,
                "permit_revision": revision, "closed": closed, "command_id": command}

    def ordinary(self):
        return [row(1, session()), row(2, self.attempt), row(3, self.result()), row(4, self.watermark(1)),
                row(5, self.watermark(2, closed=True, command="pause")),
                row(6, self.watermark(3, closed=True, command="pause")),
                row(7, self.watermark(4, closed=False, command="resume"))]

    def reader(self, rows, history=None, profile=None):
        parsed = read_commands(b"".join(compact(r) + b"\n" for r in rows))
        return TrafficAudit(parsed, {"txd-0": history or self.history}, run_id="run", controller_sha256=C,
                            txd_sha256=A, miner_sha256=M, node_ids=["node-a", "node-b"], acceptance_enabled=True,
                            rejection_bindings={"txd-0": profile} if profile else None, expected_rejection_binding=profile)

    def hook(self, transition, **detail):
        return {"event": "hook", "key": {"driver_id": "txd-0", "incarnation": "inc", "command_id": "pay"},
                "detail": {"transition": transition, **detail}}

    def held(self):
        attempt = copy.deepcopy(self.attempt)
        attempt["attempt"]["permit_revision"] = 1
        return [row(1, session()), row(2, self.hook("armed", mode="hold_before_intent", deadline_ms=1000, boot_id="boot")),
                row(3, self.hook("signed", raw_sha256=digest(self.raw), journal_seq=4)),
                row(4, self.watermark(1, last=0, completed=0)),
                row(5, self.hook("released", raw_sha256=digest(self.raw))), row(6, attempt),
                row(7, self.result(attempt=6)), row(8, self.watermark(2, last=6))]

    def test_real_schema_send_preimage_and_pause_interval(self):
        audit = self.reader(self.ordinary())
        self.assertEqual(audit.pause_intervals()[0]["forward_attempts"], 0)
        self.assertEqual(audit.pause_intervals()[0]["last_used_send_sequence"], 1)

    def test_unknown_missing_and_truncated_records_inconclusive(self):
        for data in (b"", b"{}", b'{"record":"unknown","seq":1}\n', b'{"record":"begin","seq":1,"seq":1}\n'):
            with self.assertRaises(Inconclusive):
                read_commands(data)
        rows = self.ordinary()
        rows[1]["unexpected"] = "value"
        with self.assertRaises(Inconclusive):
            self.reader(rows)
        rows = self.ordinary()
        rows[0]["version"] = True
        with self.assertRaises(Inconclusive):
            self.reader(rows)

    def test_missing_session_or_outcome_never_proves_zero(self):
        rows = self.ordinary()
        rows[0] = row(1, {"event": "measured_boundary", "command_id": "bootstrap", "tip_hash": "11" * 32, "tip_height": 101})
        with self.assertRaises(Inconclusive):
            self.reader(rows)
        with self.assertRaises(Inconclusive):
            self.reader(self.ordinary()[:2])

    def test_wrong_source_hash_fails_closed(self):
        rows = self.ordinary()
        rows[0]["event"]["binding"]["actor_sha256"] = "dd" * 32
        with self.assertRaises(Invalid):
            self.reader(rows)

    def test_payload_hash_is_recomputed_from_complete_signed_bytes(self):
        rows = self.ordinary()
        rows[1]["event"]["attempt"]["payload_sha256"] = digest(compact([self.raw.hex()[:-2]]))
        with self.assertRaisesRegex(Invalid, "raw/params hash"):
            self.reader(rows)

    def test_stale_chain_revision_and_wrong_intent(self):
        for field, value in (("chain_revision", 0), ("intent_seq", 4), ("command_id", "other")):
            rows = self.ordinary()
            rows[1]["event"]["attempt"]["context"][field] = value
            with self.subTest(field=field), self.assertRaises(Invalid):
                self.reader(rows)

    def test_watermark_requires_actual_completions(self):
        rows = self.ordinary()
        rows[2] = row(3, self.watermark(1))
        with self.assertRaises(Inconclusive):
            self.reader(rows)
        rows = self.ordinary()
        rows[3]["event"]["completed_count"] = 2
        with self.assertRaises(Invalid):
            self.reader(rows)

    def test_forward_while_gate_closed_is_rejected(self):
        rows = self.ordinary()[:6]
        a = copy.deepcopy(self.attempt)
        a["attempt"]["rpc_request_id"] = 2
        a["attempt"]["permit_revision"] = 3
        rows.append(row(7, a))
        with self.assertRaisesRegex(Invalid, "gate closed"):
            self.reader(rows)

    def test_hold_checkpoint_and_release_bind_original_raw_bytes(self):
        proof = self.reader(self.held()).hold_proof("txd-0", "pay")
        self.assertEqual(proof["forward_attempts_while_held"], 0)
        self.assertEqual(proof["forward_attempts_after_release"], 1)
        rows = self.held()
        rows[4]["event"]["detail"]["raw_sha256"] = "00" * 32
        with self.assertRaisesRegex(Invalid, "different signed bytes"):
            self.reader(rows)

    def test_hold_cancellation_and_expiry_never_become_grants(self):
        rows = self.held()
        rows[4]["event"] = self.hook("cancelled")
        with self.assertRaisesRegex(Invalid, "after cancellation"):
            self.reader(rows)
        rows = self.held()
        for r in rows[5:]:
            r["monotonic_ms"] += 2000
        with self.assertRaisesRegex(Invalid, "after expiry"):
            self.reader(rows)

    def test_actual_response_loss_is_not_synthetic_acceptance(self):
        history = copy.deepcopy(self.history[:7])
        history[-1]["record"] = "unknown_outcome"
        rows = [row(1, session()), row(2, self.hook("armed", mode="lose_one_response", deadline_ms=1000, boot_id="boot")),
                row(3, self.hook("signed", raw_sha256=digest(self.raw), journal_seq=4)), row(4, self.attempt),
                row(5, self.result(attempt=4, suppressed=True)), row(6, self.hook("response_discarded", attempt_seq=4)),
                row(7, self.watermark(1, last=4))]
        proof = self.reader(rows, history).response_loss("txd-0", "pay")
        self.assertTrue(proof["real_success_response_suppressed_once"])
        rows[4]["event"]["response_sha256"] = digest(compact({"outcome": "unknown", "class": "transport"}))
        with self.assertRaisesRegex(Invalid, "success"):
            self.reader(rows, history).response_loss("txd-0", "pay")

    def test_miner_attempt_cannot_precede_boundary(self):
        attempt = {"event": "miner_attempt", "run_id": "run", "miner_id": "miner-a", "incarnation": "miner-inc",
                   "request_id": "request", "payload_sha256": "aa" * 32, "controller_sha256": C, "actor_sha256": M}
        with self.assertRaisesRegex(Invalid, "precedes measured"):
            self.reader([row(1, session()), row(2, attempt)])

    def test_miner_wire_struct_order_and_newline_are_committed(self):
        payload = {"connection": {"workers": [], "id": "one"}}
        expected = (b'{"protocol":"spc-cpu-miner-control","version":1,"command_id":"req",'
                    b'"operation":"add_connection","payload":{"connection":{"id":"one","workers":[]}}}\n')
        self.assertEqual(miner_wire_bytes("req", payload), expected)
        self.assertNotEqual(digest(expected), digest(expected[:-1]))

    def test_definitive_rejection_requires_exact_preimage_and_profile(self):
        rows = self.ordinary()
        rows[2]["event"]["outcome"] = "node_rejected"
        rows[2]["event"]["response_sha256"] = digest(compact({"outcome": "node_rejected", "code": -26, "message": "version"}))
        history = copy.deepcopy(self.history)
        history[6].update(record="rejected", code="rpc_tx_version_nonstandard")
        binding = {"source_sha256": "11" * 32, "binary_sha256": "22" * 32, "consensus_fingerprint": "33" * 32}
        audit = self.reader(rows, history, profile=binding)
        self.assertEqual(audit.definitive_rejections({"txd-0": binding}, binding)[0]["code"], "rpc_tx_version_nonstandard")
        with self.assertRaises(Inconclusive):
            audit.definitive_rejections({"txd-0": binding}, None)
        rows[2]["event"]["response_sha256"] = digest(compact({"outcome": "node_rejected", "code": -26, "message": "version "}))
        with self.assertRaises(Invalid):
            self.reader(rows, history, profile=binding).definitive_rejections({"txd-0": binding}, binding)


if __name__ == "__main__":
    unittest.main()
