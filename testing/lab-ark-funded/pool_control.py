"""Closed consumer of the tested pool-wallet-control r3 DTO (legacy compatible).

Transport completion and a producer's Confirmed label are NOT proof verdicts.
The AP5 raw verifier must separately establish every funding/VTXO claim.
"""
import json
import re

from common import Blocked, encoded, require

FIELDS = {"schema", "operation_id", "state", "onchain_trusted_sats", "float_spendable_sats",
          "observed_at_unix_ms", "observed_tip_height", "error_code", "funding_txid", "board"}
BOARD_FIELDS = {"gross_board_sats", "net_board_sats", "board_fee_sats", "onchain_fee_sats",
                "actual_fee_sats", "expiry_height", "required_confirmations", "server_pubkey",
                "vtxo_exit_delta", "min_board_sats", "max_vtxo_sats", "vtxo_id"}
STATES = {"queued", "observed", "rejected", "already_at_target", "unknown", "submitted", "confirmed"}
PROOF_STATES = {"proof_available", "proof_missing", "proof_unavailable"}
TERMINAL_STATES = {"observed", "rejected", "already_at_target", "confirmed"} | PROOF_STATES
PROOF_ERRORS = {"original_operation_missing", "original_board_unresolved", "original_operation_unavailable",
                "original_operation_mismatch", "original_board_not_confirmed", "proof_not_retained",
                "proof_unavailable", "proof_digest_mismatch", "proof_binding_mismatch"}


def strict_json(raw):
    def pairs(items):
        result = {}
        for key, value in items:
            require(key not in result, "duplicate pool result field")
            result[key] = value
        return result
    return json.loads(raw, object_pairs_hook=pairs,
                      parse_constant=lambda _: (_ for _ in ()).throw(Blocked("nonfinite JSON number")))


def uint(value, bits=64, nullable=False):
    require((nullable and value is None) or (type(value) is int and 0 <= value < 2**bits),
             "pool amount/height must be an exact JSON unsigned integer or permitted null")


def operation_id(value):
    require(type(value) is str and 0 < len(value) <= 128 and all(33 <= ord(c) <= 126 for c in value), "pool operation id")


def validate_request(request):
    require(type(request) is dict and request.get("schema") == "spc.lab.ap5.pool-wallet-request/v1", "pool request schema")
    operation_id(request.get("operation_id"))
    base = {"schema", "operation_id", "action"}
    action = request.get("action")
    if action == "snapshot":
        require(set(request) == base, "snapshot request fields")
    elif action == "board_float":
        require(set(request) == base | {"net_target_sats", "max_fee_sats"}, "board request fields")
        for name in ("net_target_sats", "max_fee_sats"):
            uint(request[name])
            require(request[name] <= 2_100_000_000_000_000, "board amount exceeds Bitcoin money range")
        require(request["net_target_sats"] > 0, "zero board target")
    elif action == "board_proof_v1":
        require(set(request) == base | {"original_board_operation_id"}, "proof read fields; money is forbidden")
        operation_id(request["original_board_operation_id"])
        require(request["operation_id"] != request["original_board_operation_id"], "proof read id must differ from mutation id")
    else:
        require(False, "unsupported pool action")
    return request


def parse_result(raw, request):
    validate_request(request)
    value = strict_json(raw)
    require(type(value) is dict and FIELDS <= set(value) <= FIELDS | {"proof", "retained_proof_sha256"}, "pool result shape")
    require(value["schema"] == "spc.lab.ap5.pool-wallet-result/v1", "pool result schema")
    require(value["operation_id"] == request["operation_id"], "pool operation correlation mismatch")
    state = value["state"]
    require(type(state) is str and state in STATES | PROOF_STATES, "unsupported pool state")
    for field in ("onchain_trusted_sats", "float_spendable_sats", "observed_at_unix_ms"):
        uint(value[field], nullable=True)
    uint(value["observed_tip_height"], 32, nullable=True)
    require(value["error_code"] is None or (type(value["error_code"]) is str and
            0 < len(value["error_code"]) <= 256 and value["error_code"].isascii()), "pool error code")
    if "retained_proof_sha256" in value:
        require(state == "confirmed" and type(value["retained_proof_sha256"]) is str
                and re.fullmatch("[0-9a-f]{64}", value["retained_proof_sha256"]), "retained proof digest only belongs to Confirmed")
    if request["action"] == "board_proof_v1":
        require(state in PROOF_STATES | {"queued"}, "proof read cannot mutate or observe current float")
        require(all(value[f] is None for f in ("onchain_trusted_sats", "float_spendable_sats", "observed_at_unix_ms",
                                              "observed_tip_height", "funding_txid", "board")), "historical proof read invented current observation/effect")
        if state == "proof_available":
            require("proof" in value and value["error_code"] is None, "available proof missing envelope or has error")
            from board_proof import validate_envelope
            validate_envelope(value["proof"], request["original_board_operation_id"])
        else:
            require("proof" not in value, "non-available read carries a proof")
            require(value["error_code"] is None if state == "queued" else value["error_code"] in PROOF_ERRORS, "proof read error code")
            if state == "proof_missing":
                require(value["error_code"] in {"original_operation_missing", "proof_not_retained"}, "proof missing reason")
        return value
    require(state not in PROOF_STATES and "proof" not in value, "proof envelope/state on non-read action")
    effect = state in {"unknown", "submitted", "confirmed"}
    if request["action"] == "snapshot":
        require(state in {"queued", "observed", "rejected"}, "snapshot cannot board")
    else:
        require(state != "observed", "board operation cannot return snapshot Observed")
    if not effect:
        require(value["funding_txid"] is None and value["board"] is None, "no-board state invented a transaction")
    else:
        require(isinstance(value["funding_txid"], str) and re.fullmatch("[0-9a-f]{64}", value["funding_txid"]),
                "effect-bearing state lacks exact funding txid")
        board = value["board"]
        require(type(board) is dict and set(board) == BOARD_FIELDS, "board facts missing or unknown fields")
        for name in BOARD_FIELDS - {"server_pubkey", "vtxo_id", "max_vtxo_sats"}:
            uint(board[name], 32 if name in ("expiry_height", "vtxo_exit_delta") else 64)
        uint(board["max_vtxo_sats"], nullable=True)
        require(isinstance(board["server_pubkey"], str) and re.fullmatch("0[23][0-9a-f]{64}", board["server_pubkey"]), "board server key")
        require(board["gross_board_sats"] - board["board_fee_sats"] == board["net_board_sats"] > 0, "board gross/net/fee contradiction")
        require(board["actual_fee_sats"] == board["board_fee_sats"] + board["onchain_fee_sats"] <= request["max_fee_sats"], "board fee cap/total contradiction")
        require(board["required_confirmations"] > 0 and board["gross_board_sats"] >= board["min_board_sats"], "board policy contradiction")
        require(board["max_vtxo_sats"] is None or board["net_board_sats"] <= board["max_vtxo_sats"], "board exceeds observed max VTXO")
        if board["vtxo_id"] is not None:
            require(type(board["vtxo_id"]) is str and re.fullmatch("[0-9a-f]{64}:[0-9]+", board["vtxo_id"]), "board VTXO outpoint")
            uint(int(board["vtxo_id"].split(":")[1]), 32)
        if state == "confirmed":
            require(board["vtxo_id"] is not None, "Confirmed without actual VTXO id is unsupported")
    if state in {"observed", "already_at_target", "confirmed"}:
        require(value["observed_at_unix_ms"] is not None and value["observed_tip_height"] is not None, "observation has no clock/tip")
    if state in {"observed", "already_at_target"}:
        require(value["onchain_trusted_sats"] is not None and value["float_spendable_sats"] is not None, "successful snapshot cannot default unknown balances")
    if state == "already_at_target":
        require(value["float_spendable_sats"] >= request["net_target_sats"], "AlreadyAtTarget below target")
    return value


class Operation:
    """Same request bytes for every unresolved poll; immutable effect binding."""
    def __init__(self, request):
        validate_request(request)
        self.request = request
        self.bytes = encoded(request)
        self.binding = None
        self.vtxo_id = None
        self.terminal = None

    def observe(self, raw):
        require(encoded(self.request) == self.bytes, "operation payload mutated during polling")
        result = parse_result(raw, self.request)
        if result["funding_txid"] is not None:
            board = dict(result["board"])
            # The VTXO id is learned during recovery; all signed proposal,
            # amount, fee and policy fields must stay bound across polls.
            board.pop("vtxo_id")
            binding = encoded([result["funding_txid"], board])
            require(self.binding is None or self.binding == binding, "same operation changed signed board/policy binding")
            self.binding = binding
            learned = result["board"]["vtxo_id"]
            if learned is not None:
                require(self.vtxo_id is None or self.vtxo_id == learned, "same operation changed original board VTXO id")
                self.vtxo_id = learned
        require(not self.binding or result["state"] not in {"observed", "rejected", "already_at_target"},
                "effect-bearing operation reverted to a no-effect terminal")
        if self.terminal is not None:
            require(result == self.terminal, "terminal receipt was rewritten")
        if result["state"] in TERMINAL_STATES:
            self.terminal = result
        return result


def fresh_snapshot(result, requested_at_ms, now_ms, max_age_ms=15000):
    require(result["state"] == "observed", "fresh snapshot requires Observed")
    require(requested_at_ms <= result["observed_at_unix_ms"] <= now_ms
            and now_ms - result["observed_at_unix_ms"] <= max_age_ms, "stale/future snapshot; request a NEW snapshot id")
    return result
