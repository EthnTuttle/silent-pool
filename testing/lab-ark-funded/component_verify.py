"""Independent AP5 component checks, replayable from immutable raw evidence.

The verifier consumes bytes and expected policy/owner bindings, not Sent or
Confirmed labels. Whole-lab/T3 acceptance is deliberately a separate verdict.
"""
import hashlib
import json
from pathlib import Path
import subprocess
import base64

from common import artifact, encoded, require, sha, stock_id_bytes
from pool_control import strict_json, uint, parse_result
from raw_verify import parse_tx, transaction_record
from board_proof import bind_retained_proof


def raw_transaction(views, maturity):
    require(type(views) is list and len(views) == 2, "two-node raw transaction evidence required")
    results = [transaction_record(v, maturity) for v in views]
    require(results[0] == results[1] and views[0]["raw"] == views[1]["raw"], "two-node raw proof disagreement")
    return results[0]


def public_vtxo(verifier, raw_hex, views, expected, maturity, runner=None):
    anchor = raw_transaction(views, maturity)
    executable = artifact(verifier)
    request = {"schema": "spc.lab.ap5.vtxo-proof-request/v1", "raw_vtxo_hex": raw_hex,
               "anchor_tx_hex": views[0]["raw"], "expected": expected}
    reply = (runner([str(executable)], input=encoded(request), timeout=30) if runner else
             subprocess.run([str(executable)], input=encoded(request), capture_output=True, timeout=30))
    require(reply.returncode == 0, "full stock ark-lib raw VTXO/signature validation failed")
    value = strict_json(reply.stdout)
    require(value.get("schema") == "spc.lab.ap5.vtxo-proof/v1" and value.get("verified") is True
            and value.get("validator") == "stock-ark-lib-full-chain-and-signatures"
            and value.get("ark_revision") == "c419159fa54085572f8ca72d2ba1716556a9d3eb", "unsupported raw VTXO verifier reply")
    require(value["raw_sha256"] == hashlib.sha256(bytes.fromhex(raw_hex)).hexdigest()
            and value["anchor_raw_sha256"] == hashlib.sha256(bytes.fromhex(views[0]["raw"])).hexdigest(), "raw VTXO verifier input binding mismatch")
    require(value["policy_kind"] == "pubkey" and value["chain_anchor"].split(":")[0] == anchor["txid"], "full policy/anchor mismatch")
    for field in ("user_pubkey", "server_pubkey", "amount_sats", "expiry_height", "exit_delta"):
        require(value[field] == expected[field], "VTXO expected field mismatch: " + field)
    require(value["exit_depth"] <= expected["max_exit_depth"], "VTXO exceeds actual exit-depth policy")
    return value


def proto_uint(value):
    # Canonical protobuf JSON uses decimal strings for uint64. This is separate
    # from the pool client DTO, whose money fields MUST be JSON integers.
    require(type(value) is str and value.isascii() and value.isdecimal(), "noncanonical ArkInfo uint64")
    n = int(value)
    uint(n)
    return n


def board_policy(board, ark_info, raw_result):
    require(board["server_pubkey"] == ark_info["bound"]["server_pubkey"], "board provider binding mismatch")
    policy = ark_info["ark_info"]
    require(board["required_confirmations"] == policy["requiredBoardConfirmations"]
            and board["vtxo_exit_delta"] == policy["vtxoExitDelta"]
            and board["min_board_sats"] == proto_uint(policy["minBoardAmount"]), "board policy differs from actual ArkInfo")
    maximum = policy.get("maxVtxoAmount")
    require(board["max_vtxo_sats"] == (None if maximum is None else proto_uint(maximum)), "board maximum differs from advertised policy")
    fees = policy["fees"]["board"]
    # Stock c419159 BoardFees::calculate uses ceil(ppm * gross) + base, with a
    # minimum. No floating point and no inferred unlimited hidden max_board.
    fee = max(proto_uint(fees["minFeeSat"]), proto_uint(fees["baseFeeSat"]) +
              (board["gross_board_sats"] * proto_uint(fees["ppm"]) + 999999) // 1000000)
    require(fee == board["board_fee_sats"], "board fee differs from actual stock fee schedule")
    require(raw_result["fee_sats"] == board["onchain_fee_sats"], "board onchain fee differs from raw transaction")
    require(board["actual_fee_sats"] == fee + raw_result["fee_sats"], "board total fee mismatch")
    return {"board_fee_sats": fee, "onchain_fee_sats": raw_result["fee_sats"],
            "server_pubkey": board["server_pubkey"], "hidden_max_board": "standard_cosign_response_authority"}


def round_value(inputs, outputs):
    before = sum(i["amount_sats"] for i in inputs)
    after = sum(o["amount_sats"] for o in outputs)
    require(inputs and outputs and 0 < after <= before, "round value conservation failed")
    return {"input_sats": before, "output_sats": after, "user_paid_fee_sats": before - after}


def offboard_value(inputs, views, script, maturity):
    raw = raw_transaction(views, maturity)
    transaction = parse_tx(views[0]["raw"])
    received = sum(o["sats"] for o in transaction["vout"] if o["script"] == script)
    available = sum(i["amount_sats"] for i in inputs)
    require(0 < received <= available, "offboard paid wrong destination or exceeded input value")
    return {**raw, "ark_input_sats": available, "onchain_received_sats": received,
            "user_paid_fee_sats": available - received, "destination_script": script}


def mandatory_components(users, pools):
    expected = {"fund-server-alice", "fund-server-bob", *["fund-" + p for p in pools]}
    expected |= {"float-" + p for p in pools}
    for user in users:
        expected |= {prefix + user for prefix in ("receive-", "sp-refresh-", "second-renewal-", "stock-offboard-", "sp-offboard-")}
    return expected


def native_funding_verdict(components, cleanup, source_bindings):
    """A native-transfer diagnostic verdict, never a full AP5 verdict."""
    from source_scope import NATIVE_COMPONENTS
    by_id = {c["id"]: c for c in components}
    require(len(by_id) == len(components) and set(by_id) <= NATIVE_COMPONENTS, "non-funding component in diagnostic verdict")
    require(not any(c.get("requires") for c in components), "diagnostic cannot require Ark lifecycle components")
    missing = sorted(NATIVE_COMPONENTS - set(by_id))
    failed = sorted(c["id"] for c in components if c["status"] == "failed")
    unknown = sorted(c["id"] for c in components if c["status"] not in {"passed", "failed"})
    clean = False
    if cleanup.get("exhaustive") is True and cleanup.get("all_resources_released") is True:
        from cleanup import validate_cleanup
        clean = validate_cleanup(cleanup)
    status = "failed" if failed or cleanup.get("status") == "failed" else "passed" if not (missing or unknown) and clean else "inconclusive"
    return {"schema": "spc.lab.ap5.native-funding-verdict/v1", "scope": "confirmed-funding", "diagnostic_status": status,
            "components": components, "missing": missing, "failed": failed, "unknown": unknown, "cleanup": cleanup,
            "source_bindings": source_bindings, "native_funding_confirmed": not (missing or failed or unknown),
            "ark_lifecycle_ready": False, "funded_ark_lifecycle": False, "aggregate_acceptance": False, "whole_lab_complete": False}


def final_verdict(components, expected, cleanup, source_bindings):
    expected = set(expected) | {name for c in components for name in c.get("requires", [])}
    by_id = {c["id"]: c for c in components}
    require(len(by_id) == len(components), "duplicate component verdict identity")
    missing = sorted(expected - set(by_id))
    failed = sorted(c["id"] for c in components if c["status"] == "failed")
    unknown = sorted(c["id"] for c in components if c["status"] != "passed" and c["id"] not in failed)
    clean = cleanup.get("schema") == "spc.lab.ap5.cleanup/v2" and cleanup.get("exhaustive") is True and cleanup.get("all_resources_released") is True
    if clean:
        from cleanup import validate_cleanup
        clean = validate_cleanup(cleanup)
    status = "failed" if failed or cleanup.get("status") == "failed" else "inconclusive" if missing or unknown or not clean else "passed"
    return {"schema": "spc.lab.ap5.component-verdict/v2", "component_status": status,
            "components": components, "missing": missing, "failed": failed, "unknown": unknown,
            "source_bindings": source_bindings, "cleanup": cleanup,
            "verification": "raw Bitcoin bytes plus source-bound stock ark-lib full VTXO signatures/policy",
            "whole_lab_complete": False, "external_qa_verdict": None}


def replay_component(root, component, verifier, runner=None):
    """QA can replay every accepted raw component without touching a wallet."""
    reference = component["evidence"]
    path = Path(root) / reference["file"]
    require(path.parent.resolve() == Path(root).resolve() and not path.is_symlink(), "component evidence path escape")
    require(sha(path) == reference["sha256"], "component evidence was rewritten")
    data = strict_json(path.read_bytes())
    requires = []
    require(data.get("schema") == "spc.lab.ap5.raw-component/v2" and data.get("id") == component["id"], "raw component identity/schema mismatch")
    identity = component["id"]
    required = next((fields for prefix, fields in (
        ("fund-", ("payments", "transactions")),
        ("board-", ("board", "transactions", "vtxos", "board_request", "confirmed_receipt", "proof_read_request", "proof_read_result")),
        ("float-", ("already_at_target", "fresh_snapshot", "vtxos", "float_rows")),
        ("receive-", ("vtxos", "payout_attribution")), ("sp-refresh-", ("round", "vtxos")),
        ("second-renewal-", ("renewal", "vtxos")),
        ("stock-offboard-", ("offboard", "vtxos", "transactions")),
        ("sp-offboard-", ("offboard", "vtxos", "transactions"))) if identity.startswith(prefix)), None)
    require(required is not None and all(data.get(f) for f in required), "component-specific evidence missing")
    original = None
    if identity.startswith("board-"):
        original = bind_retained_proof(data["proof_read_request"], data["proof_read_result"],
                                       data["board_request"], data["confirmed_receipt"])
        require(data["board"] == original["board"], "component board differs from retained original")
    policy = data["ark_info"]
    raw_policy = strict_json(bytes.fromhex(policy["raw_response_hex"]))
    require(raw_policy == policy["ark_info"] and raw_policy["network"] == "regtest", "actual ArkInfo raw binding mismatch")
    require(base64.b64decode(raw_policy["serverPubkey"], validate=True).hex() == policy["bound"]["server_pubkey"]
            and base64.b64decode(raw_policy["mailboxPubkey"], validate=True).hex() == policy["bound"]["mailbox_pubkey"], "full provider/mailbox binding mismatch")
    require(hashlib.sha256(encoded(raw_policy["fees"])).hexdigest() == policy["bound"]["fees_sha256"], "actual fee-policy hash mismatch")
    tx = [raw_transaction(v, data["maturity"]) for v in data.get("transactions", [])]
    vtxos = [public_vtxo(verifier, v["raw_hex"], v["anchor_views"], v["expected"], data["maturity"], runner)
             for v in data.get("vtxos", [])]
    require(len({v["id"] for v in vtxos}) == len(vtxos), "duplicate raw VTXO census")
    for v in vtxos:
        require(v["server_pubkey"] == policy["bound"]["server_pubkey"]
                and v["exit_delta"] == raw_policy["vtxoExitDelta"]
                and v["exit_depth"] <= raw_policy["maxVtxoExitDepth"], "raw VTXO differs from bound server policy")
    if data.get("board"):
        require(len(tx) == len(vtxos) == 1, "board component missing independent raw evidence")
        b, v = data["board"], vtxos[0]
        require(v["id"] == b["vtxo_id"] and v["amount_sats"] == b["net_board_sats"]
                and v["expiry_height"] == b["expiry_height"]
                 and v["chain_anchor"].split(":")[0] == tx[0]["txid"], "historical board/VTXO binding mismatch")
        require(original is not None and data["vtxos"][0]["raw_hex"] == original["original_full_vtxo_hex"]
                and all(view["raw"] == original["raw_anchor_tx_hex"] for view in data["transactions"][0])
                and all(view["raw"] == original["raw_anchor_tx_hex"] for view in data["vtxos"][0]["anchor_views"])
                and v["user_pubkey"] == original["owner_pubkey"] and v["server_pubkey"] == original["server_pubkey"]
                and v["chain_anchor"] == f"{original['anchor_txid']}:{original['anchor_vout']}", "retained public proof/owner/full anchor differs from crypto evidence")
        require(original["checked_tip_height"] >= tx[0]["height"] + original["funding_confirmations"] - 1,
                "historical confirmation count exceeds checked chain context")
        board_policy(data["board"], data["ark_info"], tx[0])
    require(tx or vtxos or data.get("already_at_target"), "empty producer label is not a raw component")
    if data.get("already_at_target"):
        from scenario import FLOAT_TARGET
        require(data["target_sats"] == FLOAT_TARGET, "float target differs from default AP5 contract")
        board_result = parse_result(encoded(data["board_result"]), data["board_request"])
        require(board_result["state"] in ("already_at_target", "confirmed"), "float lacks terminal board/no-board outcome")
        if board_result["state"] == "confirmed":
            requires.append("board-" + identity.removeprefix("float-"))
        require(vtxos and sum(v["amount_sats"] for v in vtxos) >= data["target_sats"], "synthetic already-at-target proof")
        snapshot = data["fresh_snapshot"]
        require(snapshot["state"] == "observed" and snapshot["float_spendable_sats"] >= data["target_sats"], "target lacks fresh actual snapshot")
        rows = data["float_rows"]
        require(len(rows) == len(vtxos) and {r["id"] for r in rows} == {v["id"] for v in vtxos}, "float row/proof census mismatch")
        require(all(v["expiry_height"] >= snapshot["observed_tip_height"] + 64 for v in vtxos), "float proof outside spendable expiry window")
        for v in vtxos:
            r = next(r for r in rows if r["id"] == v["id"])
            require(r["state"] == "Spendable" and r["amount_sats"] == v["amount_sats"] and r["expiry_height"] == v["expiry_height"], "float row differs from raw proof")
    for payment in data.get("payments", []):
        raw = next((v[0]["raw"] for v in data["transactions"] if v[0]["txid"] == payment["txid"]), None)
        require(raw is not None and any(o == {"script": payment["script"], "sats": payment["sats"]} for o in parse_tx(raw)["vout"]), "funding destination/value lacks raw backing")
    if identity.startswith("fund-"):
        amount = 200_000_000 if identity in ("fund-server-alice", "fund-server-bob") else 120_000_000
        require(len(data["payments"]) == 1 and data["payments"][0]["sats"] == amount, "funding amount differs from default AP5 contract")
    by_id = {v["id"]: v for v in vtxos}
    by_bytes = {stock_id_bytes(v["id"]): v for v in vtxos}
    if "payout_attribution" in data:
        rows = data["payout_attribution"]
        require(rows and all(r.get("status") == "sent" for r in rows)
                and {str(r["vtxo_id"]).removeprefix("\\x").lower() for r in rows} == set(by_bytes), "mining payout/VTXO census mismatch")
    if "round" in data:
        r = data["round"]
        inputs = [by_bytes[i] for i in r["inputs"]]
        outputs = [by_id[i] for i in r["outputs"]]
        require(all(o["chain_anchor"].split(":")[0] == r["funding_txid"] for o in outputs), "refresh output has wrong round anchor")
        require(round_value(inputs, outputs) == r["value"], "round amount/fee claims differ from raw VTXOs")
        c = strict_json(r["checkpoint_raw"])
        require(c == r["checkpoint"], "round checkpoint rendering mismatch")
        require(c.get("version") == 2 and c.get("locked") and c.get("released"), "missing durable signed round proof")
        preimage = [c["version"], c["input_ids"], c["outputs"], c["unlock_hash"], c["funding"], c["locked"]]
        # Preserve serde's nested request field ordering from the raw checkpoint.
        digest = hashlib.sha256(json.dumps(preimage, separators=(",", ":"), ensure_ascii=False).encode()).digest()
        require(list(digest) == c["proof_binding"], "round checkpoint proof binding mismatch")
        require(parse_tx(bytes(c["funding"]).hex())["txid"] == r["funding_txid"], "checkpoint funding differs from round")
        require([bytes(i).hex() for i in c["input_ids"]] == r["inputs"], "checkpoint input binding mismatch")
        output_raw = {p["raw_hex"] for p, v in zip(data["vtxos"], vtxos) if v["id"] in r["outputs"]}
        require(len(c["released"]) == len(c["locked"]) == len(c["outputs"]) == len(outputs)
                and {bytes(v).hex() for v in c["released"]} == output_raw, "released checkpoint differs from verified child proofs")
    if "renewal" in data:
        r = data["renewal"]
        require(r["automatic"] is True and r["operations"], "second renewal has no automatic operation evidence")
        consumed = set()
        for operation in r["operations"]:
            payload = strict_json(operation["payload"])
            require(payload["action"] == {"kind": "refresh"} and len(payload["vtxo_ids"]) == 1, "automatic renewal command shape")
            raw_id = bytes.fromhex(payload["vtxo_ids"][0])
            require(operation["id"] == payload["operation_id"], "renewal operation id mismatch")
            height = operation["verified_auto_trigger_height"]
            uint(height, 32)
            require(hashlib.sha256(b"SPc/StockAutoRefresh/v1" + raw_id + height.to_bytes(4, "big")).hexdigest() == operation["id"], "second renewal is not bound to automatic selector recipe")
            require(operation["id"] not in r["manual_operation_ids"], "second renewal was manually submitted")
            consumed.update(payload["vtxo_ids"])
        require(set(r["inputs"]) == consumed and consumed <= set(by_bytes), "second renewal input census incomplete")
        require(r["outputs"] and len(set(r["outputs"])) == len(r["outputs"]), "empty/duplicate renewal outputs")
        require({op["result"].removeprefix("round_submitted:") for op in r["operations"]
                 if op["stage"] == "stock_submitted" and op["result"].startswith("round_submitted:")} == set(r["round_ids"]), "renewal result/round binding mismatch")
        require(all(by_id[i]["expiry_height"] > r["previous_expiry"] and by_id[i]["chain_anchor"].split(":")[0] in r["round_ids"] for i in r["outputs"]), "second renewal outputs/expiry not established by raw proof")
    if "offboard" in data:
        o = data["offboard"]
        inputs = [by_id.get(i) or by_bytes[i] for i in o["inputs"]]
        require(len(data["transactions"]) == 1, "offboard transaction census")
        payload = strict_json(o["operation"]["payload"])
        require(payload["operation_id"] == o["operation"]["id"] and payload["action"] == {"kind": "offboard", "address": o["destination_address"]}
                and set(payload["vtxo_ids"]) == {stock_id_bytes(v["id"]) for v in inputs}, "offboard operation/input binding mismatch")
        # Use the actual submitted command's allocated public return address.
        from ap5 import p2pkh_address
        require(p2pkh_address(o["destination_script"]) == o["destination_address"], "offboard destination/address mismatch")
        result = o["operation"]["result"]
        require(result == ("offboard_broadcast:" if identity.startswith("stock-") else "") + tx[0]["txid"], "offboard result/raw transaction mismatch")
        if identity.startswith("sp-"):
            checkpoint = strict_json(o["operation"]["checkpoint"])
            require(o["operation"]["stage"] == "offboard_confirmed" and checkpoint.get("signed")
                    and bytes(checkpoint["signed"]).hex() == data["transactions"][0][0]["raw"], "direct offboard signed checkpoint/raw mismatch")
        require(offboard_value(inputs, data["transactions"][0], o["destination_script"], data["maturity"]) == o["value"], "offboard amount/fee/destination differs from raw evidence")
    return {"id": component["id"], "raw_transactions": tx, "raw_vtxos": vtxos, "requires": requires}
