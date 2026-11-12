"""Transport-retention regressions; no production RPC or funding authority."""
import hashlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from urllib.error import HTTPError

from oracle import Inconclusive, Invalid, sha256d
from rpc_capture import Rpc, RpcRecorder, stamp
from canonical_capture import CanonicalCapture, validate_export
from test_oracle import block_bytes, coinbase, policy

TEMP_BASE = None


class Capture:
    def __init__(self):
        self.rows = []

    def add(self, kind, payload):
        self.rows.append((kind, payload))
        return len(self.rows)


class Binding:
    node_id, broker, url = "node-a", None, "http://127.0.0.1:1"

    def __init__(self, root):
        self.root = root
        self.cookie = root / "cookie"
        self.cookie.write_bytes(b"test-only:secret")
        self.cookie.chmod(0o600)

    def verify(self):
        return {"node_id": self.node_id, "route": "host"}


class RawNode:
    def __init__(self, name, recorder, raw):
        self.node_id, self.recorder, self.raw = name, recorder, raw
        self.tip = sha256d(raw[:80])[::-1].hex()
        self.calls, self.race = 0, False

    def recorded(self, method, params):
        self.calls += 1
        request = {"jsonrpc": "1.0", "id": self.calls, "method": method, "params": params}
        oid, _ = self.recorder.begin({"node_id": self.node_id, "route": "host"}, request)
        result = {"getbestblockhash": self.tip, "getblockhash": self.tip, "getblock": self.raw.hex(),
                  "getblockheader": {"hash": self.tip, "height": 0, "confirmations": 1}}[method]
        if self.race and method == "getbestblockhash" and self.calls > 1:
            result = "44" * 32
        self.recorder.end(oid, sent=stamp(), raw=json.dumps({"id": self.calls, "result": result, "error": None}).encode(), status=200)
        return result, oid


class RpcCaptureTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="t3-rpc-vectors-", dir=TEMP_BASE)
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.capture = Capture()
        self.recorder = RpcRecorder(self.root, self.capture)

    def test_http_error_retains_exact_body_and_correlated_legacy_observation(self):
        raw = b'{"id":1,"result":null,"error":{"code":-5,"message":"not found"}}\n'
        rpc = Rpc(Binding(self.root), self.recorder)
        error = HTTPError(rpc.binding.url, 500, "failure", {}, io.BytesIO(raw))
        with patch("rpc_capture.urllib.request.urlopen", side_effect=error), self.assertRaises(Inconclusive):
            rpc.call("getrawtransaction", ["11" * 32, 0])
        entry = self.recorder.exchanges[rpc.last_exchange]
        self.assertEqual((self.root / entry["end"]["response_body"]["path"]).read_bytes(), raw)
        self.assertEqual(entry["end"]["http_status"], 500)
        self.assertIsNone(entry["end"]["failure"])
        self.assertEqual(self.capture.rows[-1][0], "node_rpc")
        self.assertEqual(self.capture.rows[-1][1]["response"]["error"]["code"], -5)
        with self.assertRaises(Inconclusive):
            self.recorder.decoded(rpc.last_exchange)

    def test_malformed_body_and_transport_loss_remain_incomplete(self):
        class Response(io.BytesIO):
            status = 200
        rpc = Rpc(Binding(self.root), self.recorder)
        with patch("rpc_capture.urllib.request.urlopen", return_value=Response(b'{broken')), self.assertRaises(Inconclusive):
            rpc.call("getbestblockhash")
        entry = self.recorder.exchanges[rpc.last_exchange]
        self.assertEqual((self.root / entry["end"]["response_body"]["path"]).read_bytes(), b'{broken')
        with patch("rpc_capture.urllib.request.urlopen", side_effect=TimeoutError()), self.assertRaises(Inconclusive):
            rpc.call("getbestblockhash")
        self.assertIsNone(self.recorder.exchanges[rpc.last_exchange]["end"]["response_body"])
        self.assertNotIn("node_rpc", [kind for kind, _ in self.capture.rows])

    def test_readonly_rpc_cannot_acquire_send_or_mining_authority(self):
        rpc = Rpc(Binding(self.root), self.recorder)
        for method in ("sendrawtransaction", "invalidateblock", "generate", "generatetoaddress", "submitblock"):
            with self.assertRaises(Invalid):
                rpc.call(method, [])
        self.assertEqual(self.recorder.exchanges, {})

    def setup_chain(self):
        raw = block_bytes(bytes(32), [coinbase(10000000, tag=b"\1\0")])
        p = policy(genesis=sha256d(raw[:80])[::-1].hex(), regtest_pow_bits_overlay=None, subsidy_initial_sats=10000000)
        chain = CanonicalCapture(p, self.recorder, self.capture)
        nodes = [RawNode(name, self.recorder, raw) for name in ("node-a", "node-b")]
        return p, chain, nodes

    def export(self, chain):
        path = self.root / "chain.json"
        seal = chain.export(path, run_id="run", binary_hashes={"node": "11" * 32}, scenario_sha256="22" * 32,
                            source_snapshot_sha256="33" * 32)
        return path, seal

    def validate(self, path, p, seal=None):
        return validate_export(path, "run", {"node": "11" * 32},
            expected_export_sha256=seal or hashlib.sha256(path.read_bytes()).hexdigest(),
            expected_source_snapshot_sha256="33" * 32, expected_scenario_sha256="22" * 32,
            expected_policy=p, expected_node_ids=["node-a", "node-b"])

    def test_two_node_raw_fences_export_and_external_seal(self):
        p, chain, nodes = self.setup_chain()
        self.assertIsNotNone(chain.sample(nodes))
        # Cached bodies are still attributed to each node; a later fence must
        # perform new canonicality reads without pretending bodies are fresh.
        self.assertIsNotNone(chain.sample(nodes))
        path, seal = self.export(chain)
        verified = self.validate(path, p, seal["sha256"])
        self.assertEqual(len(verified["verified_fences"]), 2)
        self.assertFalse(verified["funding_or_send_authority"])
        with self.assertRaisesRegex(Invalid, "external seal"):
            self.validate(path, p, "00" * 32)

    def test_dropped_node_or_missing_body_does_not_pass_export(self):
        p, chain, nodes = self.setup_chain()
        chain.sample(nodes)
        path, _ = self.export(chain)
        original = path.read_bytes()
        data = json.loads(original)
        data["cohorts"][0]["nodes"].pop()
        path.write_text(json.dumps(data))
        with self.assertRaisesRegex(Invalid, "census"):
            self.validate(path, p)
        path.write_bytes(original)
        ref = data["block_bodies"][0]["raw_block"]
        (self.root / ref["path"]).unlink()
        with self.assertRaisesRegex(Inconclusive, "unavailable"):
            self.validate(path, p)

    def test_racing_tip_and_unresolved_begin_are_inconclusive(self):
        p, chain, nodes = self.setup_chain()
        nodes[1].race = True
        self.assertIsNone(chain.sample(nodes))
        path, _ = self.export(chain)
        with self.assertRaisesRegex(Inconclusive, "no complete"):
            self.validate(path, p)
        data = json.loads(path.read_bytes())
        data["unresolved_rpc_observations"] = [999]
        path.write_text(json.dumps(data))
        with self.assertRaisesRegex(Inconclusive, "unresolved"):
            self.validate(path, p)


if __name__ == "__main__":
    unittest.main()
