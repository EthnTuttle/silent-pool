"""Exact node effect census: production forwards plus explicit fixture traces."""
from collections import Counter

from oracle import Inconclusive, decode_transaction, require, strict_json, unhex


def reconcile_node_sends(audit, node_records, observer_records=(), fixture_references=()):
    expected = Counter()
    for row in audit.attempts.values():
        attempt = row["event"]["attempt"]
        if attempt["method"] == "sendrawtransaction":
            signed = audit._send_preimage(attempt)
            expected[(attempt["node_id"], unhex(signed["raw_hex"]))] += 1
    production = expected.copy()
    observations = {}
    for seq, row in enumerate(observer_records, 1):
        if row.get("seq") != seq:
            raise Inconclusive("fixture observer trace sequence gap")
        observations[seq] = row
    fixture_ids, fixture_rpcs = set(), set()
    for reference in fixture_references:
        require(set(reference) == {"intent_seq", "rpc_seq", "completion_seq"}, "fixture effect reference fields")
        first, rpc_seq, last = reference["intent_seq"], reference["rpc_seq"], reference["completion_seq"]
        if any(seq not in observations for seq in (first, rpc_seq, last)):
            raise Inconclusive("fixture effect trace missing")
        require(first < rpc_seq < last and rpc_seq not in fixture_rpcs, "fixture effect order/duplicate RPC")
        intent, rpc, completion = (observations[seq] for seq in (first, rpc_seq, last))
        require(intent["kind"] == "test_only_witness_send" and rpc["kind"] == "node_rpc"
                and completion["kind"] == "test_only_witness_result", "unrecognized extra node effect channel")
        intent, rpc, completion = intent["payload"], rpc["payload"], completion["payload"]
        require(intent["actor"] == completion["actor"] == "witness-fixture" and intent["kind"] in {"p2wpkh", "p2tr"},
                "extra node effect is not a declared witness fixture")
        require(intent["command_id"] == completion["command_id"] and intent["command_id"] not in fixture_ids,
                "duplicate fixture effect identity")
        raw = unhex(intent["raw_hex"])
        tx = decode_transaction(raw)
        require(tx["witness"] and tx["txid"] == intent["txid"] == completion["txid"], "fixture actual witness bytes/txid")
        require(rpc["node"] == intent["node_id"] == completion["node_id"]
                and rpc["request"]["method"] == "sendrawtransaction" and rpc["request"]["params"] == [raw.hex()],
                "fixture declaration differs from actual observer RPC")
        require(type(rpc["response"]["id"]) is int and type(rpc["request"]["id"]) is int
                and rpc["response"]["id"] == rpc["request"]["id"] and rpc["response"].get("error") is None
                and rpc["response"]["result"] == tx["txid"]
                and completion["rpc_seq"] == rpc_seq and completion["intent_seq"] == first,
                "fixture completion is not an actual correlated node response")
        key = rpc["node"], raw
        require(key not in production, "fixture label overlaps a production forward")
        expected[key] += 1
        fixture_ids.add(intent["command_id"]); fixture_rpcs.add(rpc_seq)
    observed = Counter()
    for node, rows in node_records.items():
        for row in rows:
            method = row.get("method")
            require(isinstance(method, str) and not method.startswith("generate") and method != "setmocktime", "hidden generation/time override")
            if method != "sendrawtransaction":
                continue
            try:
                params = strict_json(row["params"]) if isinstance(row["params"], str) else row["params"]
            except ValueError as error:
                raise Inconclusive("node effect body truncated; exact census unavailable") from error
            require(isinstance(params, list) and len(params) == 1, "node raw-send request form")
            raw = unhex(params[0])
            decode_transaction(raw)
            observed[(node, raw)] += 1
    extras = observed - expected
    require(not extras, "unlisted or duplicate actual node send")
    if expected - observed:
        raise Inconclusive("forward/fixture effect lacks exact node observation")
    if not expected:
        raise Inconclusive("no actual send effects to reconcile")
    return {"production_forwards": sum(production.values()), "explicit_fixture_sends": len(fixture_ids),
            "node_observed_sends": sum(observed.values()), "unlisted_sends": 0}
