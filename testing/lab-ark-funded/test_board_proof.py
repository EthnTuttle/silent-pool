"""Consumer negatives using the producer's labelled, public NOT-FUNDED fixture."""
import copy
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import Mock, patch

from ap5 import Controller
from board_proof import bind_retained_proof, canonical, canonical_original, digest
from common import Blocked, Evidence, encoded, sha
from pool_control import Operation, parse_result, strict_json, validate_request
from verify_archive import bind_component_pool_events, verify_pool_events

FIXTURE = Path(__file__).parent / "fixtures/pool-wallet-control-r3-unit-NOT-FUNDED.json"


def facts():
    reply = json.loads(FIXTURE.read_text())
    original = reply["proof"]["original"]
    read = {"schema": "spc.lab.ap5.pool-wallet-request/v1", "operation_id": reply["operation_id"],
            "action": "board_proof_v1", "original_board_operation_id": original["original_board_operation_id"]}
    confirmed = strict_json(bytes.fromhex(original["terminal_receipt_context_canonical_json_hex"]))
    confirmed["retained_proof_sha256"] = reply["proof"]["proof_sha256"]
    return read, reply, copy.deepcopy(original["board_request"]), confirmed


def repack(reply, confirmed=None):
    envelope = reply["proof"]
    raw = canonical_original(envelope["original"])
    envelope["canonical_json_hex"] = raw.hex()
    envelope["proof_sha256"] = digest(raw)
    if confirmed is not None:
        confirmed["retained_proof_sha256"] = envelope["proof_sha256"]


def nonavailable(reply, state, error=None):
    value = copy.deepcopy(reply)
    del value["proof"]
    value.update(state=state, error_code=error)
    return value


class RetainedProof(unittest.TestCase):
    def test_exact_producer_fixture_is_binding_only_not_current_float_or_funding(self):
        self.assertEqual(sha(FIXTURE), "06fc304f4e6d5828dd988ff551f5686025cb15e96859b05098f481641c77b886")
        read, reply, request, confirmed = facts()
        original = bind_retained_proof(read, reply, request, confirmed)
        self.assertEqual(original["original_vtxo_id"], confirmed["board"]["vtxo_id"])
        self.assertEqual(confirmed["float_spendable_sats"], 0)
        self.assertIsNone(reply["float_spendable_sats"])
        self.assertFalse(reply["proof"]["lineage_complete"])
        # Even an available retained object has no two-node canonical blocks or
        # input-value evidence. The fixture is never promoted to a payment proof.
        from component_verify import raw_transaction
        view = {"raw": original["raw_anchor_tx_hex"], "txid": original["anchor_txid"]}
        with self.assertRaises((Blocked, KeyError)):
            raw_transaction([view, view], 100)

    def test_new_read_id_is_distinct_and_money_fields_are_forbidden(self):
        read, reply, request, confirmed = facts()
        for field, value in (("operation_id", request["operation_id"]), ("net_target_sats", 1), ("max_fee_sats", None)):
            with self.assertRaises(Blocked):
                validate_request({**read, field: value})
        changed = {**read, "operation_id": "later-read"}
        bind_retained_proof(changed, {**reply, "operation_id": "later-read"}, request, confirmed)
        with self.assertRaises(Blocked):
            parse_result(encoded(reply), changed)

    def test_read_poll_terminal_immutability_and_fresh_retry(self):
        read, reply, request, confirmed = facts()
        op = Operation(read)
        queued = nonavailable(reply, "queued")
        op.observe(encoded(queued))
        op.observe(encoded(queued))
        missing = nonavailable(reply, "proof_missing", "proof_not_retained")
        op.observe(encoded(missing))
        with self.assertRaisesRegex(Blocked, "terminal receipt"):
            op.observe(encoded(reply))
        changed = {**read, "operation_id": "new-read-after-missing"}
        Operation(changed).observe(encoded({**reply, "operation_id": changed["operation_id"]}))
        self.assertEqual(changed["original_board_operation_id"], request["operation_id"])

    def test_historical_read_has_no_current_observation_or_effect_fields(self):
        read, reply, _, _ = facts()
        for field in ("onchain_trusted_sats", "float_spendable_sats", "observed_at_unix_ms", "observed_tip_height"):
            with self.assertRaises(Blocked):
                parse_result(encoded({**reply, field: 0}), read)
        for field, value in (("funding_txid", "11" * 32), ("board", {}), ("retained_proof_sha256", "11" * 32)):
            with self.assertRaises(Blocked):
                parse_result(encoded({**reply, field: value}), read)

    def test_retained_digest_is_conditional_and_legacy_remains_missing(self):
        read, reply, request, confirmed = facts()
        legacy = {k: v for k, v in confirmed.items() if k != "retained_proof_sha256"}
        parse_result(encoded(legacy), request)
        with self.assertRaises(Blocked):
            bind_retained_proof(read, reply, request, legacy)
        for value in (None, True, "ff", "00" * 32):
            changed = {**confirmed, "retained_proof_sha256": value}
            with self.assertRaises(Blocked):
                bind_retained_proof(read, reply, request, changed)
        op = Operation(request)
        op.observe(encoded(legacy))
        with self.assertRaisesRegex(Blocked, "terminal receipt"):
            op.observe(encoded(confirmed))

    def test_canonical_original_tree_order_digest_and_unknown_fields(self):
        for mode in ("hash", "tree", "order", "extra"):
            read, reply, _, _ = facts()
            envelope = reply["proof"]
            if mode == "hash":
                envelope["proof_sha256"] = "00" * 32
            elif mode == "tree":
                envelope["original"]["checked_at_unix_ms"] += 1
            elif mode == "order":
                raw = encoded(envelope["original"])
                envelope.update(canonical_json_hex=raw.hex(), proof_sha256=digest(raw))
            else:
                envelope["extra"] = 1
            with self.assertRaises(Blocked):
                parse_result(encoded(reply), read)

    def test_terminal_context_digest_and_projection_cannot_change_other_fields(self):
        read, reply, request, confirmed = facts()
        for field in ("onchain_trusted_sats", "float_spendable_sats", "observed_at_unix_ms", "observed_tip_height"):
            changed = {**confirmed, field: confirmed[field] + 1}
            with self.assertRaisesRegex(Blocked, "terminal context differs"):
                bind_retained_proof(read, reply, request, changed)
        for mode in ("hash", "drop", "cycle"):
            altered = copy.deepcopy(reply)
            original = altered["proof"]["original"]
            if mode == "hash":
                original["terminal_receipt_context_sha256"] = "00" * 32
            else:
                context = strict_json(bytes.fromhex(original["terminal_receipt_context_canonical_json_hex"]))
                if mode == "drop":
                    del context["observed_at_unix_ms"]
                else:
                    context["retained_proof_sha256"] = altered["proof"]["proof_sha256"]
                raw = canonical(context)
                original.update(terminal_receipt_context_canonical_json_hex=raw.hex(), terminal_receipt_context_sha256=digest(raw))
            repack(altered)
            with self.assertRaises(Blocked):
                parse_result(encoded(altered), read)

    def test_request_binding_and_full_owner_policy_are_not_self_asserted(self):
        read, reply, request, confirmed = facts()
        changed_request = {**request, "net_target_sats": request["net_target_sats"] + 1}
        with self.assertRaisesRegex(Blocked, "different actual board request"):
            bind_retained_proof(read, reply, changed_request, confirmed)
        for field, value in (("owner_policy_hex", "09" + reply["proof"]["original"]["owner_pubkey"]),
                             ("stock_checkpoint_ref", "board.wrong.0"), ("anchor_vout", 1),
                             ("server_pubkey", "02" + "11" * 32), ("funding_confirmations", True)):
            changed = copy.deepcopy(reply)
            changed["proof"]["original"][field] = value
            repack(changed)
            with self.assertRaises(Blocked):
                parse_result(encoded(changed), read)

    def test_pruned_original_with_unavailable_lineage_still_binds(self):
        read, reply, request, confirmed = facts()
        for reason in ("no_retained_round_imports", "round_import_evidence_unavailable"):
            changed = copy.deepcopy(reply)
            changed["proof"].update(lineage_state="unavailable", lineage_complete=False, round_imports=[], lineage_error_code=reason)
            bind_retained_proof(read, changed, request, confirmed)
            self.assertIsNone(changed["float_spendable_sats"])

    def test_partial_lineage_completeness_conflicts_capacity_and_unrelated_ids(self):
        for mode in ("complete", "duplicate", "capacity", "unreachable", "unsorted", "unavailable-with-edges"):
            read, reply, _, _ = facts()
            envelope = reply["proof"]
            if mode == "complete":
                envelope["lineage_complete"] = True
            elif mode == "duplicate":
                envelope["round_imports"] *= 2
            elif mode == "capacity":
                envelope["round_imports"] *= 257
            elif mode == "unreachable":
                envelope["round_imports"][0]["input_vtxo_ids"] = ["ff" * 32 + ":0"]
            elif mode == "unsorted":
                envelope["round_imports"][0]["output_vtxo_ids"] = ["ff" * 32 + ":0", "00" * 32 + ":0"]
            else:
                envelope.update(lineage_state="unavailable", lineage_error_code="round_import_evidence_unavailable")
            with self.assertRaises(Blocked):
                parse_result(encoded(reply), read)

    def test_archive_uses_actual_sealed_mutation_receipt_and_read_events(self):
        read, reply, request, confirmed = facts()
        events = [("pool-wallet-request", request, {}), ("pool-wallet-result", confirmed, {}),
                  ("pool-wallet-request", read, {}), ("pool-wallet-result", reply, {})]
        ops = verify_pool_events(events)
        data = {"id": "board-pool-a", "board_request": request, "confirmed_receipt": confirmed,
                "proof_read_request": read, "proof_read_result": reply}
        bind_component_pool_events(data, ops)
        altered = copy.deepcopy(data)
        altered["confirmed_receipt"]["observed_tip_height"] += 1
        with self.assertRaises(Blocked):
            bind_component_pool_events(altered, ops)
        altered_events = copy.deepcopy(events)
        altered_events[1][1]["observed_tip_height"] += 1
        with self.assertRaisesRegex(Blocked, "terminal context differs"):
            verify_pool_events(altered_events)
        with self.assertRaises(Blocked):
            verify_pool_events(events[2:])

    def test_collector_uses_retention_after_pruning_without_remote_lookup(self):
        read, reply, request, confirmed = facts()
        original = reply["proof"]["original"]
        controller = Controller.__new__(Controller)
        controller.scope = "full"
        controller.pool_requests = {read["operation_id"]: read, request["operation_id"]: request}
        controller.pool_wallet = Mock(return_value=reply)
        controller.fetch_stock_proof = Mock(side_effect=AssertionError("must not query pruned server proof"))
        controller.pool_keys = Mock(side_effect=AssertionError("historical proof cannot require current wallet key rows"))
        controller.decode_vtxo = Mock(return_value={"id": original["original_vtxo_id"], "amount_sat": original["board"]["net_board_sats"],
            "expiry_height": original["board"]["expiry_height"], "chain_anchor": original["anchor_txid"] + ":0"})
        # Wiring isolation only: real crypto and raw chain validation remain
        # compulsory in production and are not represented as passed here.
        controller.verify_public_proof = Mock()
        controller.proofs = {original["original_vtxo_id"]: {"anchor_views": [{"raw": original["raw_anchor_tx_hex"]}]}}
        controller.component = Mock()
        controller.verify_board("pool-a", confirmed, {"unit_policy": True})
        controller.pool_wallet.assert_called_once_with("pool-a", "board_proof_v1", original_board_operation_id=request["operation_id"])
        controller.fetch_stock_proof.assert_not_called()
        self.assertEqual(controller.component.call_args.kwargs["proof_read_result"], reply)
        controller.pool_wallet.return_value = nonavailable(reply, "proof_missing", "proof_not_retained")
        controller.component.reset_mock()
        with self.assertRaises(Blocked):
            controller.verify_board("pool-a", confirmed, {})
        controller.component.assert_not_called()

    def test_controller_polls_same_read_bytes_and_allocates_new_later_correlation(self):
        _, reply, request, _ = facts()
        with tempfile.TemporaryDirectory(prefix="ap5-proof-poll-", dir="/tmp/opencode") as tmp:
            root = Path(tmp)
            evidence = Evidence(root)
            c = Controller({"pool_wallet_executor": {}}, {}, root, evidence, {"run": {"id": "unit-run"}}, 30)
            calls = []
            def tool(argv, **kwargs):
                raw = Path(argv[-1]).read_bytes()
                req = strict_json(raw)
                calls.append((list(argv), raw, req))
                result = copy.deepcopy(reply)
                result["operation_id"] = req["operation_id"]
                if len(calls) == 1:
                    result = nonavailable(result, "queued")
                return subprocess.CompletedProcess(argv, 0, encoded(result), b"")
            c.run_tool = tool
            try:
                with patch("ap5.artifact", return_value=Path("/unit/executor")), patch("ap5.time.sleep"):
                    c.pool_wallet("pool-a", "board_proof_v1", original_board_operation_id=request["operation_id"])
                    c.pool_wallet("pool-a", "board_proof_v1", original_board_operation_id=request["operation_id"])
            finally:
                evidence.close()
            self.assertEqual(calls[0][:2], calls[1][:2])
            self.assertNotEqual(calls[1][2]["operation_id"], calls[2][2]["operation_id"])
            self.assertTrue(all(call[2]["action"] == "board_proof_v1" and call[2]["original_board_operation_id"] == request["operation_id"] for call in calls))


if __name__ == "__main__":
    unittest.main()
