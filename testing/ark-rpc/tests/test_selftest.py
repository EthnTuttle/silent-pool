"""Self-test of the self-test: all scenarios must pass, and the
version_310000_only scenario must yield surface_compatible with the
permanent funded-unproven marker, never any full-compat outcome."""

import json
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from arkrpc import selftest as st  # noqa: E402

TMP_PARENT = None  # tempfile honors the caller's TMPDIR on a fresh machine.


class ScenarioTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.base = st.load_base()
        cls.scenarios = {}
        scenarios_dir = os.path.join(st.FIXTURES_DIR, "scenarios")
        for name in sorted(os.listdir(scenarios_dir)):
            if name.endswith(".json"):
                with open(os.path.join(scenarios_dir, name)) as fh:
                    cls.scenarios[name] = json.load(fh)

    def _run(self, name):
        workdir = tempfile.mkdtemp(prefix="AP1a-selftest-ut-",
                                   dir=TMP_PARENT)
        try:
            return st.run_scenario(self.scenarios[name], self.base, workdir)
        finally:
            for leftover in os.listdir(workdir):
                os.unlink(os.path.join(workdir, leftover))
            os.rmdir(workdir)

    def test_all_scenarios_pass(self):
        for name in self.scenarios:
            with self.subTest(scenario=name):
                failures = self._run(name)
                self.assertEqual(failures, [],
                                 "%s: %r" % (name, failures))

    def test_protocol_failures_are_never_method_evidence(self):
        """F3/F8: -32700 for every request -> nothing reported missing."""
        overrides = self.scenarios[
            "protocol_error_parse.json"]["overrides"]
        (entries, http_status, protocol_error_code, id_echo_mode,
         error_http_status) = st.apply_overrides(self.base, overrides)
        self.assertIsNone(http_status)
        self.assertEqual(protocol_error_code, -32700)
        self.assertIsNone(id_echo_mode)
        # C1: error bodies are served over HTTP 500 (the real
        # Core/rbitcoin form), never 200-with-error-body.
        self.assertEqual(error_http_status, 500)
        verdict = self._probe(entries, http_status=http_status,
                              protocol_error_code=protocol_error_code,
                              id_echo_mode=id_echo_mode,
                              error_http_status=error_http_status)
        self.assertEqual(verdict["status"], "unproven")
        self.assertNotIn("fail-method-surface", verdict["reasons"])

    def test_apply_overrides_returns_full_server_state(self):
        """apply_overrides returns the 5-tuple the FakeRpcServer needs."""
        overrides = [
            {"op": "patch", "method": "getnetworkinfo",
             "field": "version", "value": 310000},
            {"op": "http_status", "status": 401},
        ]
        (entries, http_status, protocol_error_code, id_echo_mode,
         error_http_status) = st.apply_overrides(self.base, overrides)
        self.assertIsNotNone(entries)
        self.assertEqual(http_status, 401)
        self.assertIsNone(protocol_error_code)
        self.assertIsNone(id_echo_mode)
        self.assertEqual(error_http_status, 500)

    def test_310000_alone_is_not_full_compat(self):
        """The load-bearing negative guarantee of this packet.

        A passing version gate with every read-only check passing yields
        surface_compatible ONLY: funded_state stays "unproven",
        full_acceptance stays false, and the reasons carry the permanent
        funded-unproven marker. There is no full-compat outcome in
        read-only mode (F5).
        """
        failures = self._run("version_310000_only.json")
        self.assertEqual(failures, [])
        # Re-run directly to inspect the verdict.
        overrides = self.scenarios["version_310000_only.json"]["overrides"]
        (entries, http_status, protocol_error_code, id_echo_mode,
         error_http_status) = st.apply_overrides(self.base, overrides)
        verdict = self._probe(entries, http_status=http_status,
                              protocol_error_code=protocol_error_code,
                              id_echo_mode=id_echo_mode,
                              error_http_status=error_http_status)
        self.assertEqual(verdict["status"], "surface_compatible")
        self.assertEqual(verdict["reasons"], ["funded-unproven"])
        self.assertEqual(verdict["funded_state"], "unproven")
        self.assertIs(verdict["full_acceptance"], False)
        self.assertNotIn("fail-version-gate", verdict["reasons"])
        self.assertNotIn("surface-unproven", verdict["reasons"])

    def _probe(self, entries, http_status=None, protocol_error_code=None,
               id_echo_mode=None, error_http_status=500):
        from arkrpc.client import JsonRpcClient
        from arkrpc.contracts import get_profile
        from arkrpc.probe import run_probe
        workdir = tempfile.mkdtemp(prefix="AP1a-selftest-ut-",
                                   dir=TMP_PARENT)
        cookie = os.path.join(workdir, "cookie")
        with open(cookie, "w") as fh:
            fh.write("selftest-user:selftest-pass")
        server = st.FakeRpcServer(entries, http_status=http_status,
                                   protocol_error_code=protocol_error_code,
                                   id_echo_mode=id_echo_mode,
                                   error_http_status=error_http_status)
        server.start()
        try:
            client = JsonRpcClient(server.url, cookie, deadline_s=5.0)
            result = run_probe(client, get_profile("bark-c419159"))
        finally:
            server.stop()
            os.unlink(cookie)
            os.rmdir(workdir)
        return result["verdict"]

    def _probe_full(self, scenario_name):
        """Run one scenario against the fake server and return the full
        probe result (checks + exchanges + verdict)."""
        overrides = self.scenarios[scenario_name]["overrides"]
        (entries, http_status, protocol_error_code, id_echo_mode,
         error_http_status) = st.apply_overrides(self.base, overrides)
        from arkrpc.client import JsonRpcClient
        from arkrpc.contracts import get_profile
        from arkrpc.probe import run_probe
        workdir = tempfile.mkdtemp(prefix="AP1a-selftest-ut-",
                                   dir=TMP_PARENT)
        cookie = os.path.join(workdir, "cookie")
        with open(cookie, "w") as fh:
            fh.write("selftest-user:selftest-pass")
        server = st.FakeRpcServer(entries, http_status=http_status,
                                  protocol_error_code=protocol_error_code,
                                  id_echo_mode=id_echo_mode,
                                  error_http_status=error_http_status)
        server.start()
        try:
            client = JsonRpcClient(server.url, cookie, deadline_s=5.0)
            return run_probe(client, get_profile("bark-c419159"))
        finally:
            server.stop()
            os.unlink(cookie)
            os.rmdir(workdir)

    def test_run_all_scenarios_helper(self):
        results = st.run_all_scenarios(verbose=False)
        self.assertTrue(results)
        for name, failures in results:
            self.assertEqual(failures, [], name)

    def test_error_path_id_echo_enforced_receipts(self):
        """C1 regression (Fable r1 counterexample): 200 successes echo
        numeric ids, HTTP 500 error bodies echo strings. Every
        error-path receipt must be counted as a mismatch, and no error
        code behind the broken echo may be interpreted."""
        result = self._probe_full("id_string_echo_errorpath.json")
        checks = {c["id"]: c for c in result["checks"]}
        self.assertEqual(checks["protocol.id-echo"]["availability"],
                         "failed")
        self.assertEqual(checks["protocol.id-echo"]["behavior"], "failed")
        mismatched = [k for k, r in result["exchanges"].items()
                      if r["receipt"] and r["receipt"]["id_match"] is False]
        self.assertTrue(mismatched,
                        "error-path string echoes must be counted")
        for key in mismatched:
            receipt = result["exchanges"][key]["receipt"]
            self.assertEqual(receipt["http_status"], 500, key)
        # successes echoed the numeric id exactly (200 receipts match)
        for key, r in result["exchanges"].items():
            receipt = r["receipt"]
            if receipt and receipt["http_status"] == 200:
                self.assertTrue(receipt["id_match"], key)
        # recognized rows unknown: no method evidence behind the echo
        for cid in ("getmempoolentry.recognized",
                    "sendrawtransaction.recognized",
                    "testmempoolaccept.recognized",
                    "submitpackage.recognized"):
            self.assertEqual(checks[cid]["availability"], "unknown", cid)
        self.assertIn("fail-protocol", result["verdict"]["reasons"])

    def test_fake_server_serves_errors_over_http_500(self):
        """C1: the fake server serves JSON-RPC error bodies over HTTP
        500 (the real Core/rbitcoin form), never 200-with-error-body."""
        result = self._probe_full("baseline_190000.json")
        saw_error_status = False
        for key, r in result["exchanges"].items():
            receipt = r["receipt"]
            if receipt is None:
                continue
            if r["error"] is not None:
                self.assertIn(receipt["http_status"], (500, 404), key)
                saw_error_status = True
            else:
                self.assertEqual(receipt["http_status"], 200, key)
        self.assertTrue(saw_error_status,
                        "baseline must contain error-path exchanges")

    def test_id_echo_aggregate_counts_all_exchanges(self):
        """C1: the id-echo aggregate covers every receipt the run made
        (including HTTP error-path receipts and the ZMQ observation),
        never a silent subset like 'all 21' of 30."""
        result = self._probe_full("baseline_190000.json")
        checks = {c["id"]: c for c in result["checks"]}
        total_receipts = sum(1 for r in result["exchanges"].values()
                             if r["receipt"] is not None)
        self.assertGreater(total_receipts, 0)
        self.assertEqual(checks["protocol.id-echo"]["behavior"], "passed")
        self.assertIn("all %d exchanges" % total_receipts,
                      checks["protocol.id-echo"]["detail"])
        # the ZMQ observation call is inside the aggregate (exchange
        # keys are "method <params-json>")
        self.assertIn("getzmqnotifications []", result["exchanges"])

    def test_captured_request_forms_match_pinned_clients(self):
        """C2: the exchanges the probe actually captured (method,
        params, envelope in CallRecord.as_dict) must match the
        source-backed client forms -- not just the ENCODINGS metadata."""
        import json as _json
        from arkrpc.probe import GENESIS_COINBASE_TXID, INVALID_TX_HEX
        result = self._probe_full("baseline_190000.json")
        ex = result["exchanges"]

        def find(method, params, envelope):
            key = "%s %s" % (method, _json.dumps(params, sort_keys=True,
                                                 default=str))
            if envelope != "1.0":
                key += " [jsonrpc %s]" % envelope
            return ex.get(key)

        # sync getrawtransaction: bitcoincore-rpc 0.19 handle_defaults
        # (client.rs:116-147) STRIPS the trailing null, so
        # get_raw_transaction_info (client.rs:507-513) and bitcoin-ext
        # custom_get_raw_transaction_info (rpc/mod.rs:320-331) put
        # [txid, true] on the wire, envelope 2.0.
        rec = find("getrawtransaction", [GENESIS_COINBASE_TXID, True],
                   "2.0")
        self.assertIsNotNone(rec, "sync [txid, true] 2.0 form not captured")
        self.assertEqual(rec["params"], [GENESIS_COINBASE_TXID, True])
        self.assertEqual(rec["envelope"], "2.0")
        self.assertIsNone(
            find("getrawtransaction", [GENESIS_COINBASE_TXID, True, None],
                 "2.0"),
            "the wrong [txid, true, null] form must not be sent")
        # async forms unchanged
        self.assertIsNotNone(
            find("getrawtransaction", [GENESIS_COINBASE_TXID, 0], "1.0"))
        self.assertIsNotNone(
            find("getrawtransaction", [GENESIS_COINBASE_TXID, True], "1.0"))

        # mutating methods: every active bark caller is an async
        # call_raw (envelope 1.0): server bitcoind.rs:207-221
        # (submit_package / test_mempool_accept), chain.rs:685
        # (submitpackage), broadcast via send_raw_transaction.
        for method, params in (
                ("testmempoolaccept", [[INVALID_TX_HEX]]),
                ("submitpackage", [[INVALID_TX_HEX]]),
                ("sendrawtransaction", [INVALID_TX_HEX])):
            rec = find(method, params, "1.0")
            self.assertIsNotNone(rec,
                                 "%s async 1.0 form not captured" % method)
            self.assertEqual(rec["envelope"], "1.0")
            self.assertIsNone(find(method, params, "2.0"),
                              "%s must not be sent as envelope 2.0"
                              % method)

        # estimatesmartfee: bark wallet fetch_fee_rates async form
        # [target, "ECONOMICAL"] over envelope 1.0
        for target in (1, 3, 6):
            self.assertIsNotNone(
                find("estimatesmartfee", [target, "ECONOMICAL"], "1.0"))

        # every captured request used one of the two pinned envelopes
        # and a numeric request id
        for key, rec in ex.items():
            self.assertIn(rec["envelope"], ("1.0", "2.0"), key)
            self.assertIsInstance(rec["request_id"], int, key)
            self.assertNotIsInstance(rec["request_id"], bool, key)


if __name__ == "__main__":
    unittest.main()
