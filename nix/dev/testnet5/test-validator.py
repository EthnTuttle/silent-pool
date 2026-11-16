#!/usr/bin/env python3
"""Offline negative controls only; no socket or node process is used."""
import importlib.util
import json
from pathlib import Path
import unittest
from unittest.mock import patch

ROOT = Path(__file__).parent
spec = importlib.util.spec_from_file_location("validator", ROOT / "validate-node.py")
validator = importlib.util.module_from_spec(spec)
spec.loader.exec_module(validator)


def fixture():
    return {
        "getblockchaininfo": {"chain": "testnet5", "blocks": 0, "initialblockdownload": True},
        "getblockhash": validator.NETWORK["genesis_hash"],
        "getblockheader": validator.NETWORK["genesis_header_hex"],
        "getnetworkinfo": {"version": 190000, "subversion": "/rbitcoin:0.9.99/"},
    }


class IdentityTests(unittest.TestCase):
    def test_genesis_hash_and_pending_sync_do_not_claim_ready(self):
        result = validator.validate(fixture())
        self.assertTrue(result["network_identity_verified"])
        self.assertTrue(result["initialblockdownload"])
        self.assertFalse(result["deployment_ready"])
        self.assertFalse(result["full_consensus_validation_proven"])

    def test_wrong_network_and_malformed_observations(self):
        edits = [
            ("getblockchaininfo", "chain", value) for value in ("testnet4", "test", "regtest", "main")
        ] + [("getblockchaininfo", "initialblockdownload", None),
             ("getblockchaininfo", "blocks", True),
             ("getnetworkinfo", "version", "190000")]
        for method, key, value in edits:
            with self.subTest(method=method, key=key, value=value):
                row = fixture()
                row[method][key] = value
                with self.assertRaises(ValueError):
                    validator.validate(row)
        for method, value in [("getblockhash", "00" * 32),
                              ("getblockheader", "00" * 80)]:
            row = fixture()
            row[method] = value
            with self.assertRaises(ValueError):
                validator.validate(row)

    def test_no_defaults_for_missing_method_or_duplicate_json(self):
        row = fixture()
        del row["getnetworkinfo"]
        with self.assertRaises(ValueError):
            validator.validate(row)
        for raw in [b'{"chain":"testnet4","chain":"testnet5"}', b'{"x":NaN}',
                    b" " * (validator.LIMIT + 1)]:
            with self.assertRaises(ValueError):
                validator.decode(raw)

    def test_no_remote_url_credentials_redirect_or_proxy(self):
        for url in ["http://127.0.0.1:18352", "http://[::1]:18352/"]:
            self.assertEqual(validator.local_url(url), url)
        for url in ["http://127.0.0.1", "http://example.com:18352", "http://localhost:18352",
                    "http://127.0.0.1:18352/x", "http://user:pass@127.0.0.1:18352",
                    "http://127.0.0.1:18352/?x=1", "https://127.0.0.1:18352"]:
            with self.assertRaises(ValueError):
                validator.local_url(url)
        with self.assertRaises(ValueError):
            validator.NoRedirect().redirect_request(None, None, 302, "", {}, "http://example.com")

    def test_rpc_wrong_chain_stops_after_one_read_only_call(self):
        class Response:
            def __enter__(self):
                return self
            def __exit__(self, *args):
                pass
            def read(self, limit):
                return b'{"id":1,"result":{"chain":"regtest"}}'
        with patch.object(validator.urllib.request, "build_opener") as build:
            build.return_value.open.return_value = Response()
            with self.assertRaises(ValueError):
                validator.collect("http://127.0.0.1:18352", b"user:secret")
            self.assertEqual(build.return_value.open.call_count, 1)
            request = build.return_value.open.call_args.args[0]
            self.assertEqual(json.loads(request.data)["method"], "getblockchaininfo")
            self.assertEqual(build.call_args.args[0].proxies, {})

    def test_complete_rpc_exchange_and_response_correlation(self):
        class Response:
            def __init__(self, value):
                self.value = value
            def __enter__(self):
                return self
            def __exit__(self, *args):
                pass
            def read(self, limit):
                return json.dumps(self.value).encode()
        rows = fixture()
        responses = [Response({"id": i, "result": rows[m]})
                     for i, (m, _) in enumerate(validator.METHODS, 1)]
        with patch.object(validator.urllib.request, "build_opener") as build:
            build.return_value.open.side_effect = responses
            result = validator.collect("http://127.0.0.1:18352", b"user:secret")
            self.assertTrue(validator.validate(result)["network_identity_verified"])
            calls = build.return_value.open.call_args_list
            self.assertEqual([json.loads(c.args[0].data)["method"] for c in calls],
                             [m for m, _ in validator.METHODS])
        for bad in [{"id": 2, "result": rows["getblockchaininfo"]},
                    {"id": True, "result": rows["getblockchaininfo"]},
                    {"id": 1, "error": {"code": -1}}, {"id": 1}]:
            with self.subTest(response=bad), patch.object(validator.urllib.request, "build_opener") as build:
                build.return_value.open.return_value = Response(bad)
                with self.assertRaises(ValueError):
                    validator.collect("http://127.0.0.1:18352", b"user:secret")
                self.assertEqual(build.return_value.open.call_count, 1)


if __name__ == "__main__":
    unittest.main()
