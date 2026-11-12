"""Self-test: runs the full probe against an in-process fake JSON-RPC server.

Scenarios live in fixtures/scenarios/*.json and are applied on top of
fixtures/base_responses.json (captured from a real owned node run and
normalized; see `probe.py make-fixtures`). Each scenario pins the
expected verdict and the expected status of specific checks, so the
honesty of the verdict logic is itself tested:

- version 190000 (real node baseline) -> incompatible / fail-version-gate
- version 310000 alone -> surface_compatible with funded-unproven;
  NEVER a full-compat outcome (read-only mode cannot grant one)
- missing / wrong-type / wrong-network / wrong-genesis / auth fail /
  method-not-found / shape mismatch / no txindex -> the matching failure
- id string echo (F3) and the C1 id-echo variants (wrong-int /
  missing / bool / null, on the 200 path AND the HTTP 500/404 error
  path) -> protocol.id-echo fails; protocol-level failures are never
  method evidence, and no error code behind a mismatched id is
  interpreted; the fake server serves error bodies over HTTP 500
  (404 overridable via error_http_status) -- the real Core/rbitcoin
  form, never 200-with-error-body
- protocol errors -32700/-32600 (F3/F8) -> nothing is reported as
  method-missing; the node is incompatible via protocol, not surface
- anchor scriptPubKey type (F4) -> accepted by the bark bitcoin-ext
  contract (getrawtransaction verbose) while the 0.19 gettxout contract
  rejects it (unit-tested in tests/test_contracts.py and validated
  against the real Rust types by devcheck/)
- empty scriptSig hex and u64-range amounts (F4) -> accepted
"""

from __future__ import annotations

import copy
import http.server
import json
import os
import tempfile
import threading

from .client import JsonRpcClient
from .contracts import get_profile
from .probe import run_probe

FIXTURES_DIR = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                            "fixtures")


class _FakeNodeHandler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0") or 0)
        body = self.rfile.read(length)
        try:
            request = json.loads(body)
        except ValueError:
            self._respond(500, {"result": None, "id": None,
                                "error": {"code": -32700, "message": "parse error"}})
            return
        rid = request.get("id")
        if self.server.http_status is not None:
            self.send_response(self.server.http_status)
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        # C1: error outcomes are served over a real HTTP error status
        # (Core serves JSON-RPC errors at 500; rbitcoin serves -32601 at
        # 404), never as 200-with-error-body.
        status = 200
        if self.server.protocol_error_code is not None:
            # Every request gets a JSON-RPC protocol error (-32700 /
            # -32600). These must never be counted as method evidence.
            rid, include_id = self._echo_id(rid, True)
            payload = {"result": None, "id": rid,
                       "error": {"code": self.server.protocol_error_code,
                                 "message": "protocol error"}}
            status = self.server.error_http_status
        else:
            entry = self.server.match(request.get("method"),
                                      request.get("params", []))
            is_error = entry is None or "error" in entry
            rid, include_id = self._echo_id(rid, is_error)
            if entry is None:
                payload = {"result": None, "id": rid,
                           "error": {"code": -32601,
                                     "message": "Method not found"}}
                status = self.server.error_http_status
            elif "error" in entry:
                payload = {"result": None, "id": rid, "error": entry["error"]}
                status = self.server.error_http_status
            else:
                payload = {"result": entry.get("result"), "id": rid,
                           "error": None}
            if not include_id:
                payload.pop("id", None)
        self._respond(status, payload)

    def _echo_id(self, rid, is_error):
        """Transform the echoed response id per the server's id_echo
        mode (C1 negative variants). Returns (id_value, include_id)."""
        mode = self.server.id_echo_mode
        if mode is None:
            return rid, True
        if mode == "string":
            # Echo the numeric request id as a JSON string: the pinned
            # async client (Response.id: u64) cannot deserialize this.
            return (str(rid) if rid is not None else rid), True
        if mode == "string-errorpath":
            # Fable's r1 counterexample: successes echo the numeric id
            # exactly, but error bodies (HTTP 500/404) echo it as a
            # string. The error path must enforce the id echo exactly
            # like the 200 path (C1) or this launders method evidence.
            if is_error:
                return (str(rid) if rid is not None else rid), True
            return rid, True
        if mode == "wrong-int":
            return ((rid + 1000) if isinstance(rid, int) and
                    not isinstance(rid, bool) else rid), True
        if mode == "missing":
            return None, False
        if mode == "bool":
            return True, True
        if mode == "null":
            return None, True
        raise ValueError("unknown id_echo mode %r" % mode)

    def _respond(self, status, payload):
        data = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *args):
        pass


class FakeRpcServer:
    """Serves canned exchanges: exact (method, params) match first, then
    method-only entries, then -32601. An http_status forces that status
    for every request (auth-fail scenario). protocol_error_code forces a
    JSON-RPC protocol error for every request. id_echo_mode breaks the
    id echo ("string", "string-errorpath", "wrong-int", "missing",
    "bool", or "null"; None echoes exactly). Error outcomes (JSON-RPC
    error bodies) are served over error_http_status (default 500, the
    real Core/rbitcoin form), never as 200-with-error-body."""

    def __init__(self, entries, http_status=None, protocol_error_code=None,
                 id_string_echo=False, id_echo_mode=None,
                 error_http_status=500):
        if id_string_echo:
            id_echo_mode = "string"
        self.entries = entries
        self.http_status = http_status
        self.protocol_error_code = protocol_error_code
        self.id_echo_mode = id_echo_mode
        self.error_http_status = error_http_status
        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0),
                                                      _FakeNodeHandler)
        self.server.match = self.match
        self.server.http_status = http_status
        self.server.protocol_error_code = protocol_error_code
        self.server.id_echo_mode = id_echo_mode
        self.server.error_http_status = error_http_status
        self.thread = threading.Thread(target=self.server.serve_forever,
                                       daemon=True)

    def match(self, method, params):
        fallback = None
        for entry in self.entries:
            if entry.get("method") != method:
                continue
            if "params" in entry:
                if entry["params"] == params:
                    return entry
            elif fallback is None:
                fallback = entry
        return fallback

    def start(self):
        self.thread.start()
        return self

    @property
    def url(self):
        return "http://127.0.0.1:%d" % self.server.server_address[1]

    def stop(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)

    def __enter__(self):
        return self.start()

    def __exit__(self, *exc):
        self.stop()


# ---------------------------------------------------------------------------
# Override application
# ---------------------------------------------------------------------------


def _find_indices(entries, method, params=None):
    out = []
    for i, entry in enumerate(entries):
        if entry.get("method") != method:
            continue
        if params is not None and entry.get("params") != params:
            continue
        out.append(i)
    return out


def _set_dotted(obj, dotted, value):
    parts = dotted.split(".")
    for part in parts[:-1]:
        obj = obj.setdefault(part, {})
    obj[parts[-1]] = value


def _del_dotted(obj, dotted):
    parts = dotted.split(".")
    for part in parts[:-1]:
        if not isinstance(obj, dict) or part not in obj:
            return
        obj = obj[part]
    if isinstance(obj, dict):
        obj.pop(parts[-1], None)


def apply_overrides(entries, overrides):
    entries = copy.deepcopy(entries)
    http_status = None
    protocol_error_code = None
    id_echo_mode = None
    error_http_status = 500
    for ov in overrides:
        op = ov["op"]
        if op == "http_status":
            http_status = ov["status"]
        elif op == "protocol_error":
            protocol_error_code = ov["code"]
        elif op == "id_string_echo":
            id_echo_mode = "string"
        elif op == "id_echo":
            # {"op": "id_echo", "mode": "string"|"wrong-int"|
            #  "missing"|"bool"|"null"}
            id_echo_mode = ov["mode"]
        elif op == "error_http_status":
            # HTTP status used for JSON-RPC error outcomes (default
            # 500; set 404 to mimic rbitcoin's -32601 form).
            error_http_status = ov["status"]
        elif op == "remove_method":
            entries = [e for e in entries if e.get("method") != ov["method"]]
        elif op == "add_entry":
            # Append a method-only fallback entry (matches any params).
            entry = {"method": ov["method"]}
            entry.update(ov.get("response", {}))
            entries.append(entry)
        elif op == "set_response":
            indices = _find_indices(entries, ov["method"], ov.get("params"))
            if not indices:
                raise ValueError("set_response: no entries for %s %r"
                                 % (ov["method"], ov.get("params")))
            for i in indices:
                new = {"method": ov["method"]}
                if "params" in ov:
                    new["params"] = ov["params"]
                new.update(ov["response"])
                entries[i] = new
        elif op == "patch":
            indices = _find_indices(entries, ov["method"], ov.get("params"))
            if not indices:
                raise ValueError("patch: no entries for %s %r"
                                 % (ov["method"], ov.get("params")))
            for i in indices:
                result = entries[i].setdefault("result", {})
                _set_dotted(result, ov["field"], ov["value"])
        elif op == "patch_params":
            # Retarget an entry's params (e.g. getblock [genesis, 0] ->
            # [wronghash, 0]) so a wrong-genesis scenario can replay the
            # stored block data for a different requested hash.
            hit = False
            for entry in entries:
                if (entry.get("method") == ov["method"]
                        and entry.get("params") == ov["from_params"]):
                    entry["params"] = ov["to_params"]
                    hit = True
            if not hit:
                raise ValueError("patch_params: no entry for %s %r"
                                 % (ov["method"], ov["from_params"]))
        elif op == "delete":
            indices = _find_indices(entries, ov["method"], ov.get("params"))
            for i in indices:
                _del_dotted(entries[i].get("result", {}), ov["field"])
        else:
            raise ValueError("unknown override op %r" % op)
    return entries, http_status, protocol_error_code, id_echo_mode, \
        error_http_status


# ---------------------------------------------------------------------------
# Scenario runner
# ---------------------------------------------------------------------------


def run_scenario(scenario, base_entries, workdir):
    """Runs one scenario; returns a list of expectation failures (empty=ok)."""
    (entries, http_status, protocol_error_code, id_echo_mode,
     error_http_status) = apply_overrides(
        base_entries, scenario.get("overrides", []))
    cookie_path = os.path.join(workdir, "cookie")
    with open(cookie_path, "w") as fh:
        fh.write("selftest-user:selftest-pass")
    server = FakeRpcServer(entries, http_status=http_status,
                           protocol_error_code=protocol_error_code,
                           id_echo_mode=id_echo_mode,
                           error_http_status=error_http_status)
    server.start()
    try:
        client = JsonRpcClient(server.url, cookie_path, deadline_s=5.0)
        profile = get_profile("bark-c419159")
        result = run_probe(client, profile)
    finally:
        server.stop()

    failures = []
    expect = scenario.get("expect", {})
    verdict = result["verdict"]
    if "verdict_status" in expect and verdict["status"] != expect["verdict_status"]:
        failures.append("verdict status %r != expected %r"
                        % (verdict["status"], expect["verdict_status"]))
    if "reasons" in expect and verdict["reasons"] != expect["reasons"]:
        failures.append("reasons %r != expected %r"
                        % (verdict["reasons"], expect["reasons"]))
    if "funded_state" in expect and verdict["funded_state"] != expect["funded_state"]:
        failures.append("funded_state %r != expected %r"
                        % (verdict["funded_state"], expect["funded_state"]))
    checks = {c["id"]: c for c in result["checks"]}
    for cid, wanted in expect.get("checks", {}).items():
        check = checks.get(cid)
        if check is None:
            failures.append("check %r missing from result" % cid)
            continue
        if isinstance(wanted, str):
            wanted = {"behavior": wanted}
        for axis in ("availability", "behavior"):
            if axis in wanted and check[axis] != wanted[axis]:
                failures.append("%s.%s = %r, expected %r (%s)"
                                % (cid, axis, check[axis], wanted[axis],
                                   check["detail"]))
    return failures


def load_base(fixtures_dir=FIXTURES_DIR):
    path = os.path.join(fixtures_dir, "base_responses.json")
    with open(path) as fh:
        base = json.load(fh)
    return base["entries"]


def run_all_scenarios(fixtures_dir=FIXTURES_DIR, verbose=False):
    base_entries = load_base(fixtures_dir)
    scenarios_dir = os.path.join(fixtures_dir, "scenarios")
    scenario_files = sorted(f for f in os.listdir(scenarios_dir)
                            if f.endswith(".json"))
    results = []
    tmp_parent = "/tmp/opencode" if os.path.isdir("/tmp/opencode") else None
    for name in scenario_files:
        with open(os.path.join(scenarios_dir, name)) as fh:
            scenario = json.load(fh)
        workdir = tempfile.mkdtemp(prefix="AP1a-selftest-", dir=tmp_parent)
        try:
            failures = run_scenario(scenario, base_entries, workdir)
        finally:
            for leftover in os.listdir(workdir):
                os.unlink(os.path.join(workdir, leftover))
            os.rmdir(workdir)
        results.append((scenario.get("name", name), failures))
        if verbose or failures:
            status = "PASS" if not failures else "FAIL"
            print("[%s] %s: %s" % (status, name, scenario.get("description", "")))
            for failure in failures:
                print("       - %s" % failure)
        else:
            print("[PASS] %s: %s" % (name, scenario.get("description", "")))
    return results
