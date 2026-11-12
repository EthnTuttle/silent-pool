import copy
import json
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from arkrpc.client import CallRecord
from arkrpc.contracts import CONTRACTS, ContractError, TRAFFIC_CONSUMER, contract_hash, get_profile
from arkrpc.probe import ProbeSession, _check_getmempoolinfo, compute_verdict
from arkrpc.reporting import build_report
from arkrpc.trafficfacts import NumberLexeme, sats_exact, policy_facts


class ConsumerTests(unittest.TestCase):
    def test_exact_genesis_rpc_forms_against_frozen_and_selected_fixtures(self):
        from arkrpc.client import RpcCallError
        from arkrpc.probe import _check_genesis_walk, GENESIS_COINBASE_TXID
        fixtures = Path(__file__).parents[1] / "fixtures"
        entries = json.loads((fixtures / "base_responses.json").read_bytes())["entries"]
        selected = json.loads((fixtures / "scenarios/traffic_selected_consumer.json").read_bytes())
        entries += [dict(method=o["method"], **o["response"]) for o in selected["overrides"]]
        class FixtureClient:
            def __init__(self):
                self.calls = []
            def call(self, method, params, envelope):
                self.calls.append((method, params, envelope))
                # JSON token typing distinguishes integer 0/1 from bools.
                key = json.dumps(params)
                entry = next(e for e in entries if e["method"] == method and json.dumps(e["params"]) == key)
                error = entry.get("error")
                return CallRecord(method, params, envelope, len(self.calls), entry.get("result"),
                    RpcCallError("server_error_%s" % error["code"], error["message"], code=error["code"])
                    if error else None, None)
        client = FixtureClient()
        profile = get_profile("bark-c419159")
        session = ProbeSession(client, profile, "regtest", profile["expected_genesis"])
        _check_genesis_walk(session)
        genesis = profile["expected_genesis"]
        for call in (("getblock", [genesis, 0], "1.0"), ("getblock", [genesis], "1.0"),
                     ("getblockheader", [genesis], "1.0"),
                     ("getrawtransaction", [GENESIS_COINBASE_TXID, 0, genesis], "1.0")):
            self.assertEqual(client.calls.count(call), 1)
        rows = {row["id"]: row for row in session.checks}
        self.assertEqual(rows["getblock.verbosity0"]["behavior"], "passed")
        self.assertEqual(rows["getblock.singleparam"]["behavior"], "passed")
        self.assertEqual(rows["getblockheader.traffic-consumer"]["behavior"], "passed")
        self.assertEqual(rows["getrawtransaction.blockhash.traffic-consumer"]["behavior"], "unknown")
        self.assertFalse(any(method in ("sendrawtransaction", "submitblock", "generatetoaddress")
                             for method, _, _ in client.calls))

    def test_selected_consumer_obeys_exact_rpc_body_limit_before_parsing(self):
        import io
        from unittest.mock import Mock, patch
        from arkrpc.client import JsonRpcClient
        body = b'{"result":{"size":0},"error":null,"id":0}'
        for limit, expected in ((len(body), "passed"), (len(body) - 1, "unknown")):
            response = Mock(status=200)
            response.read.side_effect = io.BytesIO(body).read
            response.__enter__ = Mock(return_value=response)
            response.__exit__ = Mock(return_value=False)
            with patch("arkrpc.client.read_cookie", return_value=("fixture-user", "fixture-password")), \
                    patch("arkrpc.client.urllib.request.urlopen", return_value=response) as urlopen:
                client = JsonRpcClient("http://127.0.0.1:18443", "/unused-fixture-cookie", max_body_bytes=limit)
                profile = get_profile("bark-c419159")
                session = ProbeSession(client, profile, "regtest", profile["expected_genesis"])
                _check_getmempoolinfo(session)
            response.read.assert_called_once_with(limit + 1)
            request = json.loads(urlopen.call_args.args[0].data)
            self.assertEqual(request["method"], "getmempoolinfo")
            self.assertEqual(request["params"], [])
            row = next(c for c in session.checks if c["id"] == "getmempoolinfo.traffic-consumer")
            self.assertEqual(row["availability"], expected)
            self.assertEqual(row["behavior"], expected)
            if expected == "unknown":
                self.assertIn("oversized", row["detail"])
                self.assertNotIn("mempoolinfo.size", session.observations)
                self.assertEqual(session.observations["traffic_fee_policy"]["errors"],
                                 ["raw_policy_response_unavailable"])

    def test_size_exact_u64_bounds(self):
        from arkrpc.contracts import parse_spec
        spec = CONTRACTS["getmempoolinfo.traffic-consumer"]["spec"]
        for value in (0, 2**64-1):
            self.assertEqual(parse_spec({"size": value, "maxdatacarriersize": None}, spec), {"size": value})
        for value in (-1, 2**64, True, None, "0", 0.0):
            with self.subTest(value=value), self.assertRaises(ContractError):
                parse_spec({"size": value}, spec)
        with self.assertRaises(ContractError):
            parse_spec({}, spec)

    def test_single_exchange_preserves_full_model_failure_verbatim(self):
        result = {"loaded": True, "size": 0, "bytes": 0, "usage": 0, "total_fee": 0,
            "maxmempool": 300000000, "mempoolminfee": 0, "minrelaytxfee": 0,
            "incrementalrelayfee": 0, "unbroadcastcount": 0, "fullrbf": True,
            "permitbaremultisig": True, "maxdatacarriersize": None}
        class Client:
            count = 0
            def call(self, method, params, envelope):
                self.count += 1
                return CallRecord(method, params, envelope, 0, result, None, None,
                                  raw_body=json.dumps({"result": result}).encode())
        client = Client()
        session = ProbeSession(client, get_profile("bark-c419159"), "regtest", "ab"*32)
        _check_getmempoolinfo(session)
        self.assertEqual(client.count, 1)
        model, selected = session.checks
        self.assertEqual(model["detail"], "shape: maxdatacarriersize: expected JSON integer, got null")
        self.assertEqual((model["availability"], model["behavior"]), ("passed", "failed"))
        self.assertEqual(selected["behavior"], "passed")
        verdict = compute_verdict(session.checks, session.profile)
        self.assertNotIn("fail-shape", verdict["reasons"])
        report = build_report({"checks": session.checks, "profile": session.profile,
            "observations": session.observations, "verdict": verdict}, target_label="fixture",
            target_kind="remote-url", run_token="public-report-id", contract_hash_value=contract_hash())
        self.assertEqual(report["consumer_model_failures"], [model])
        self.assertEqual(report["traffic_consumer"], TRAFFIC_CONSUMER)
        self.assertFalse(report["verdict"]["full_acceptance"])

    def test_raw_decimal_lexemes_never_float_roundtrip(self):
        for text, expected in (("0.00001000", 1000), ("184467440737.09551615", 2**64-1), ("0", 0)):
            self.assertEqual(sats_exact(NumberLexeme(text)), expected)
        for text in ("1e-8", "-0", "-1", "0.000000001", "184467440737.09551616", "NaN", "+1"):
            with self.subTest(text=text), self.assertRaises(ContractError):
                sats_exact(NumberLexeme(text))
        for value in (0.00001, "0.00001000", 0):
            with self.assertRaises(ContractError):
                sats_exact(value)
        raw = b'{"result":{"minrelaytxfee":0.00001000,"maxmempool":300000000,"bytes":0,"usage":0}}'
        rec = CallRecord("getmempoolinfo", [], "1.0", 0, json.loads(raw)["result"], None, None, raw_body=raw)
        facts = policy_facts(rec)
        self.assertEqual(facts["minrelaytxfee_btc_kb_lexeme"], "0.00001000")
        self.assertEqual(facts["minrelaytxfee_sats_kb"], 1000)
        self.assertEqual(facts["errors"], [])

    def test_projection_table_matches_canonical_manifest(self):
        import re
        from arkrpc.contracts import CHECKS
        path = Path(__file__).resolve().parents[3] / "crates/spc-lab/src/traffic/projection.rs"
        text = path.read_text()
        self.assertIn(contract_hash(), text)
        table = text.split("pub const TRAFFIC_EXPECTED_CHECKS:", 1)[1].split("\n];", 1)[0]
        for cid, method, coverage, unknown in re.findall(
                r'id: "([^"]+)",\s+method: (None|Some\("[^"]+"\)),\s+coverage: "([^"]+)",\s+unknown_behavior_ok: (true|false)', table):
            expected = CHECKS[cid]
            self.assertEqual(None if method == "None" else method[6:-2], expected["method"])
            self.assertEqual(coverage, expected["coverage"])
            self.assertEqual(unknown == "true", expected.get("unknown_behavior_ok", False))
        self.assertNotIn('id: "getmempoolinfo",', table)

    def test_effective_consensus_is_closed_typed_and_fail_closed(self):
        from arkrpc.trafficfacts import effective_consensus
        facts = {
            "network": "regtest", "consensus_profile_id": "fixture-only",
            "profile_fingerprint": "ab" * 32, "initial_target_bits": "200fffff",
            "regtest_pow_bits_overlay": "200fffff", "pow_limit_bits": "207fffff",
            "retarget_floor_bits": "207fffff", "subsidy_initial_sats": "5000000000",
            "pow_target_spacing": 600, "pow_target_timespan": 1209600,
            "difficulty_adjustment_interval": 2016, "subsidy_halving_interval": 150,
            "coinbase_maturity": 100, "no_pow_retargeting": True,
            "allow_min_difficulty_blocks": True,
        }
        self.assertEqual(effective_consensus({"consensus": dict(facts, extra="not projected")}), facts)
        for key, value in (("network", "mainnet"), ("coinbase_maturity", 100.0),
                           ("pow_target_spacing", True), ("profile_fingerprint", "bad"),
                           ("subsidy_initial_sats", "5e9")):
            bad = dict(facts, **{key: value})
            with self.subTest(key=key), self.assertRaises(ContractError):
                effective_consensus({"consensus": bad})
        for key in facts:
            bad = dict(facts)
            del bad[key]
            with self.subTest(missing=key), self.assertRaises(ContractError):
                effective_consensus({"consensus": bad})


if __name__ == "__main__":
    unittest.main()
