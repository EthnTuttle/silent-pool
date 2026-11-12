import copy
import json
import unittest

from common import Blocked
from component_verify import final_verdict, mandatory_components
from pool_control import Operation, fresh_snapshot, parse_result


def request(action="board_float"):
    value = {"schema": "spc.lab.ap5.pool-wallet-request/v1", "operation_id": "exact-opaque-operation", "action": action}
    if action == "board_float":
        value.update(net_target_sats=115000000, max_fee_sats=5000000)
    return value


def result(state="queued"):
    value = {"schema": "spc.lab.ap5.pool-wallet-result/v1", "operation_id": "exact-opaque-operation", "state": state,
             "onchain_trusted_sats": None, "float_spendable_sats": None, "observed_at_unix_ms": None,
             "observed_tip_height": None, "error_code": None, "funding_txid": None, "board": None}
    if state in ("observed", "already_at_target", "confirmed"):
        value.update(onchain_trusted_sats=200000000, float_spendable_sats=115000000,
                     observed_at_unix_ms=1000, observed_tip_height=100)
    if state in ("unknown", "submitted", "confirmed"):
        value["funding_txid"] = "22" * 32
        value["board"] = {"gross_board_sats": 115001000, "net_board_sats": 115000000,
                          "board_fee_sats": 1000, "onchain_fee_sats": 100, "actual_fee_sats": 1100,
                          "expiry_height": 532, "required_confirmations": 3,
                          "server_pubkey": "02" + "11" * 32, "vtxo_exit_delta": 144,
                          "min_board_sats": 20000, "max_vtxo_sats": None,
                          "vtxo_id": "33" * 32 + ":0" if state == "confirmed" else None}
    return value


class PoolControl(unittest.TestCase):
    def test_queued_unknown_same_id_progression_and_historical_terminal(self):
        op = Operation(request())
        for state in ("queued", "unknown", "queued", "submitted", "confirmed", "confirmed"):
            self.assertEqual(op.observe(json.dumps(result(state)))["state"], state)
        changed = result("confirmed")
        changed["observed_at_unix_ms"] += 1
        with self.assertRaises(Blocked):
            op.observe(json.dumps(changed))

    def test_money_is_exact_u64_no_bool_float_string_or_missing(self):
        for amount in (-1, True, 1.0, "115000000", 2**64):
            r = result("observed")
            r["float_spendable_sats"] = amount
            with self.assertRaises(Blocked):
                parse_result(json.dumps(r), request("snapshot"))
        r = result("observed")
        del r["float_spendable_sats"]
        with self.assertRaises(Blocked):
            parse_result(json.dumps(r), request("snapshot"))

    def test_wrong_operation_unknown_field_and_duplicate_rejected(self):
        for field, value in [("operation_id", "wrong"), ("extra", 1)]:
            r = result()
            r[field] = value
            with self.assertRaises(Blocked):
                parse_result(json.dumps(r), request())
        with self.assertRaises(Blocked):
            parse_result(json.dumps(result())[:-1] + ',"state":"queued"}', request())

    def test_confirmed_requires_real_claim_fields_and_is_not_acceptance(self):
        for field in ("board", "funding_txid"):
            r = result("confirmed")
            r[field] = None
            with self.assertRaises(Blocked):
                parse_result(json.dumps(r), request())
        r = result("confirmed")
        r["board"]["vtxo_id"] = None
        with self.assertRaises(Blocked):
            parse_result(json.dumps(r), request())
        # Even syntactically complete Confirmed does not produce a raw-proof
        # component or a passed AP5 verdict.
        parse_result(json.dumps(result("confirmed")), request())
        verdict = final_verdict([], mandatory_components(["alice"], ["pool-a"]), {}, {})
        self.assertEqual(verdict["component_status"], "inconclusive")
        self.assertFalse(verdict["whole_lab_complete"])

    def test_no_effect_states_cannot_invent_txid_or_credit_configuration(self):
        r = result("already_at_target")
        self.assertIsNone(parse_result(json.dumps(r), request())["funding_txid"])
        r["float_spendable_sats"] -= 1
        with self.assertRaises(Blocked):
            parse_result(json.dumps(r), request())
        for state in ("queued", "rejected", "already_at_target"):
            r = result(state)
            r["funding_txid"] = "11" * 32
            with self.assertRaises(Blocked):
                parse_result(json.dumps(r), request())

    def test_snapshot_freshness_does_not_refresh_historical_receipt(self):
        r = parse_result(json.dumps(result("observed")), request("snapshot"))
        fresh_snapshot(r, 900, 1100)
        with self.assertRaises(Blocked):
            fresh_snapshot(r, 2000, 3000)
        with self.assertRaises(Blocked):
            fresh_snapshot(r, 900, 20000)

    def test_unknown_proposal_cannot_change_or_become_no_effect(self):
        op = Operation(request())
        op.observe(json.dumps(result("unknown")))
        r = result("submitted")
        r["funding_txid"] = "44" * 32
        with self.assertRaises(Blocked):
            op.observe(json.dumps(r))
        with self.assertRaises(Blocked):
            op.observe(json.dumps(result("rejected")))
        op.request["net_target_sats"] += 1
        with self.assertRaises(Blocked):
            op.observe(json.dumps(result("unknown")))

    def test_fee_policy_and_full_board_shape_are_checked(self):
        for field, value in [("actual_fee_sats", 0), ("board_fee_sats", 2), ("required_confirmations", 0),
                             ("max_vtxo_sats", 10), ("server_pubkey", "00" * 33)]:
            r = result("confirmed")
            r["board"][field] = value
            with self.assertRaises(Blocked):
                parse_result(json.dumps(r), request())

    def test_learned_original_vtxo_cannot_change_during_unresolved_recovery(self):
        op = Operation(request())
        first = result("submitted")
        first["board"]["vtxo_id"] = "33" * 32 + ":0"
        op.observe(json.dumps(first))
        changed = result("confirmed")
        changed["board"]["vtxo_id"] = "44" * 32 + ":0"
        with self.assertRaises(Blocked):
            op.observe(json.dumps(changed))


if __name__ == "__main__":
    unittest.main()
