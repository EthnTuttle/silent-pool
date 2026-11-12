"""Public-only r3 retained-board envelope binding, independently replayable.

This authenticates captured bytes/relations, not the producer's crypto claim.
The component verifier MUST separately validate the full VTXO and both-node
raw anchor evidence. Partial stock movement history never proves current float.
"""
import hashlib
import json
import re

from common import encoded, require
from pool_control import parse_result, strict_json, uint, validate_request
from raw_verify import parse_tx

REQUEST_ORDER = ("schema", "operation_id", "action", "net_target_sats", "max_fee_sats", "original_board_operation_id")
BOARD_ORDER = ("gross_board_sats", "net_board_sats", "board_fee_sats", "onchain_fee_sats", "actual_fee_sats",
               "expiry_height", "required_confirmations", "server_pubkey", "vtxo_exit_delta", "min_board_sats", "max_vtxo_sats", "vtxo_id")
RESULT_ORDER = ("schema", "operation_id", "state", "onchain_trusted_sats", "float_spendable_sats",
                "observed_at_unix_ms", "observed_tip_height", "error_code", "funding_txid", "board")
ORIGINAL_ORDER = ("schema", "original_board_operation_id", "board_request", "board_request_sha256",
                  "original_vtxo_id", "original_full_vtxo_hex", "original_full_vtxo_sha256", "vtxo_encoding",
                  "raw_anchor_tx_hex", "anchor_txid", "anchor_vout", "owner_pubkey", "owner_policy_hex",
                  "owner_policy_kind", "server_pubkey", "board", "checked_at_unix_ms", "checked_tip_height",
                  "funding_confirmations", "stock_checkpoint_ref", "terminal_receipt_context_schema",
                  "terminal_receipt_context_canonical_json_hex", "terminal_receipt_context_sha256")
ENVELOPE_FIELDS = {"schema", "proof_sha256", "canonical_json_hex", "original", "lineage_state",
                   "lineage_complete", "round_imports", "lineage_error_code"}
EDGE_FIELDS = {"schema", "stock_movement_id", "input_vtxo_ids", "output_vtxo_ids", "round_funding_txid",
               "offchain_fee_sats", "completed_at_unix_ms"}


def ordered(value, fields):
    require(type(value) is dict and set(value) == set(fields), "retained proof closed field set")
    return {key: value[key] for key in fields}


def canonical(value):
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"), allow_nan=False).encode("utf-8")


def raw_hex(value, max_bytes=4 * 1024 * 1024):
    require(type(value) is str and 0 < len(value) <= 2 * max_bytes and len(value) % 2 == 0
            and re.fullmatch("[0-9a-f]+", value), "retained proof canonical bounded hex")
    return bytes.fromhex(value)


def digest(value):
    return hashlib.sha256(value).hexdigest()


def hash_hex(value):
    require(type(value) is str and re.fullmatch("[0-9a-f]{64}", value), "retained proof hash")


def vtxo_id(value):
    require(type(value) is str and re.fullmatch(r"[0-9a-f]{64}:(0|[1-9][0-9]*)", value), "canonical retained VTXO id")
    uint(int(value.split(":")[1]), 32)


def canonical_request(request):
    validate_request(request)
    return canonical({key: request[key] for key in REQUEST_ORDER if key in request})


def canonical_context(context):
    value = ordered(context, RESULT_ORDER)
    value["board"] = ordered(value["board"], BOARD_ORDER)
    return canonical(value)


def canonical_original(original):
    value = ordered(original, ORIGINAL_ORDER)
    value["board_request"] = strict_json(canonical_request(value["board_request"]))
    value["board"] = ordered(value["board"], BOARD_ORDER)
    return canonical(value)


def validate_lineage(envelope, original_id):
    require(envelope["lineage_complete"] is False, "partial lineage cannot claim completeness")
    edges = envelope["round_imports"]
    require(type(edges) is list and len(edges) <= 256, "round lineage exceeds bound")
    if envelope["lineage_state"] == "unavailable":
        require(not edges and envelope["lineage_error_code"] in
                {"no_retained_round_imports", "round_import_evidence_unavailable"}, "unavailable lineage contradiction")
        return
    require(envelope["lineage_state"] == "recorded_partial" and edges and envelope["lineage_error_code"] is None,
            "partial lineage shape")
    movement_ids, consumed, produced = [], set(), set()
    for edge in edges:
        ordered(edge, EDGE_FIELDS)
        require(edge["schema"] == "spc.lab.ap5.pool-wallet-round-import/v1", "round import schema")
        uint(edge["stock_movement_id"], 32)
        uint(edge["offchain_fee_sats"])
        uint(edge["completed_at_unix_ms"])
        require(edge["completed_at_unix_ms"] > 0, "round import completion context")
        hash_hex(edge["round_funding_txid"])
        movement_ids.append(edge["stock_movement_id"])
        for name in ("input_vtxo_ids", "output_vtxo_ids"):
            ids = edge[name]
            require(type(ids) is list and 0 < len(ids) <= 256, "round import id census")
            for value in ids:
                vtxo_id(value)
            require(ids == sorted(set(ids)), "round import ids not sorted unique")
        inputs, outputs = set(edge["input_vtxo_ids"]), set(edge["output_vtxo_ids"])
        require(not (inputs & consumed or outputs & produced or inputs & outputs) and original_id not in outputs,
                "conflicting round lineage identities")
        consumed |= inputs
        produced |= outputs
    require(movement_ids == sorted(set(movement_ids)), "conflicting/unsorted stock movement identities")
    known, selected = {original_id}, set()
    while True:
        before = len(selected)
        for index, edge in enumerate(edges):
            if known.intersection(edge["input_vtxo_ids"]):
                known.update(edge["output_vtxo_ids"])
                selected.add(index)
        if len(selected) == before:
            break
    require(len(selected) == len(edges), "round lineage has no input-id path from original; amounts are not ancestry")


def validate_envelope(envelope, original_operation_id):
    ordered(envelope, ENVELOPE_FIELDS)
    require(envelope["schema"] == "spc.lab.ap5.pool-wallet-board-proof-envelope/v1", "retained proof envelope schema")
    original = envelope["original"]
    raw = raw_hex(envelope["canonical_json_hex"])
    require(raw == canonical_original(original) and canonical(strict_json(raw)) == raw,
            "retained original JSON is not exact canonical declared order")
    require(digest(raw) == envelope["proof_sha256"], "retained envelope digest mismatch")
    require(original["schema"] == "spc.lab.ap5.pool-wallet-board-proof/v1"
            and original["vtxo_encoding"] == "ark-lib/ProtocolEncoding/c419159", "retained original schema/codec")
    req = original["board_request"]
    require(req["action"] == "board_float" and req["operation_id"] == original["original_board_operation_id"] == original_operation_id,
            "retained original mutation binding")
    require(digest(canonical_request(req)) == original["board_request_sha256"], "retained request canonical digest mismatch")
    require(digest(raw_hex(original["original_full_vtxo_hex"])) == original["original_full_vtxo_sha256"], "retained full VTXO digest mismatch")
    vtxo_id(original["original_vtxo_id"])
    hash_hex(original["anchor_txid"])
    uint(original["anchor_vout"], 32)
    tx = parse_tx(raw_hex(original["raw_anchor_tx_hex"]).hex())
    require(tx["txid"] == original["anchor_txid"] and original["anchor_vout"] < len(tx["vout"]), "retained raw anchor binding")
    for key in ("owner_pubkey", "server_pubkey"):
        require(type(original[key]) is str and re.fullmatch("0[23][0-9a-f]{64}", original[key]), "retained public key encoding")
    # Exact c419159 Pubkey policy: tag 0x00 followed by the compressed key.
    # The independent stock verifier separately binds this to the full VTXO.
    require(original["owner_policy_kind"] == "pubkey" and original["owner_policy_hex"] == "00" + original["owner_pubkey"],
            "retained full owner policy differs from Pubkey")
    require(original["terminal_receipt_context_schema"] == "spc.lab.ap5.pool-wallet-terminal-receipt-context/v1", "terminal context schema")
    context_raw = raw_hex(original["terminal_receipt_context_canonical_json_hex"])
    require(digest(context_raw) == original["terminal_receipt_context_sha256"], "terminal context digest mismatch")
    context = strict_json(context_raw)
    require(canonical_context(context) == context_raw, "terminal context is not canonical projection")
    parse_result(context_raw, req)
    require(context["state"] == "confirmed" and context["funding_txid"] == original["anchor_txid"]
            and canonical(context["board"]) == canonical(ordered(original["board"], BOARD_ORDER)), "terminal context board binding")
    b = original["board"]
    require(b["vtxo_id"] == original["original_vtxo_id"] and b["server_pubkey"] == original["server_pubkey"]
            and tx["vout"][original["anchor_vout"]]["sats"] == b["gross_board_sats"], "original board/anchor facts mismatch")
    uint(original["checked_at_unix_ms"])
    uint(original["checked_tip_height"], 32)
    uint(original["funding_confirmations"])
    require(original["checked_at_unix_ms"] > 0 and original["checked_tip_height"] < b["expiry_height"]
            and original["funding_confirmations"] >= b["required_confirmations"] > 0, "retained historical confirmation context")
    require(original["stock_checkpoint_ref"] == f"board.{original['anchor_txid']}.{original['anchor_vout']}", "original stock checkpoint binding")
    validate_lineage(envelope, original["original_vtxo_id"])
    return original


def bind_retained_proof(read_request, read_result, board_request, confirmed):
    """Bind to the actual stored mutation/terminal receipt, not an envelope's claims."""
    validate_request(read_request)
    require(read_request["action"] == "board_proof_v1", "historical proof requires read-only action")
    parse_result(encoded(confirmed), board_request)
    result = parse_result(encoded(read_result), read_request)
    require(confirmed["state"] == "confirmed" and result["state"] == "proof_available", "retained original proof unavailable; no remote reconstruction")
    envelope = result["proof"]
    original = envelope["original"]
    require(canonical_request(original["board_request"]) == canonical_request(board_request), "proof covers different actual board request")
    require(confirmed.get("retained_proof_sha256") == envelope["proof_sha256"], "original Confirmed retained digest mismatch or legacy reference absent")
    projection = {key: value for key, value in confirmed.items() if key != "retained_proof_sha256"}
    require(canonical_context(projection) == raw_hex(original["terminal_receipt_context_canonical_json_hex"]),
            "terminal context differs from original receipt beyond retained digest")
    return original
