"""Minimal JSON-RPC client: cookie auth, deadlines, bounded body, receipts,
credential-exact redaction.

Protocol fidelity (F3): the two pinned bark clients both send NUMERIC
request ids -- bitcoind-async-client 0.15.0 uses a usize counter from 0
with envelope "1.0", and bitcoincore-rpc 0.19 (jsonrpc 0.18) uses a
usize nonce from 1 with envelope "2.0". This client mirrors both: each
call carries a numeric u64 id and a caller-chosen envelope, and the
response id must echo the request id EXACTLY on BOTH the 200 path and
the HTTPError (500/404) path (C1): a string echo of a numeric request
breaks the async client, whose Response.id is declared u64, so the
probe treats it as a protocol failure, never as method support, and no
error code behind a mismatched id is ever interpreted.

Receipts: every call -- success or error -- records the HTTP status, the
JSON-RPC envelope field of the response (if any), and the response id,
so reports carry exact HTTP/id receipts for 200/500/404 alike.

Protocol errors: JSON-RPC -32700 (parse) and -32600 (invalid request)
are categorized "protocol_error"; they say nothing about method
support. -32601 is "method_missing"; -32602 is "invalid_params"; other
JSON-RPC error objects are "server_error_<code>".

Privacy: the node credential never appears in argv, reports or logs.
It is read from the cookie file (Core `username:password` format, no
trailing newline) and only ever sent as an HTTP Basic Authorization
header. `redact()` removes the exact credential VALUES (and anything
shaped like an Authorization header value); it does not mangle
non-secret text such as `--rpc-cookie-file` paths in provenance.
"""

from __future__ import annotations

import base64
import json
import re
import socket
import decimal
import urllib.error
import urllib.request

PROTOCOL_ERROR_CODES = (-32700, -32600)


class RpcCallError(Exception):
    """An RPC call failed. `category` is one of the documented categories.

    Categories:
      transport       connection refused / reset / DNS / unreachable
      timeout         the per-call deadline elapsed
      auth            HTTP 401 (bad or missing cookie credentials)
      http_<code>     other non-200 HTTP status without a JSON-RPC error
      method_missing  JSON-RPC error code -32601
      invalid_params  JSON-RPC error code -32602
      protocol_error  JSON-RPC error codes -32700 / -32600 (parse /
                      invalid request): says nothing about method support
      server_error_<code>  any other JSON-RPC error object
      parse           response body is not JSON
      bad_jsonrpc     JSON but not a JSON-RPC response object
      id_mismatch     response id did not echo the numeric request id
      oversized       response body exceeded the bounded-body cap
    """

    def __init__(self, category: str, detail: str, code=None, http_status=None,
                 receipt=None):
        super().__init__("%s: %s" % (category, redact(detail)))
        self.category = category
        self.detail = redact(detail)
        self.code = code
        self.http_status = http_status
        self.receipt = receipt

    def as_dict(self) -> dict:
        d = {"category": self.category, "detail": self.detail}
        if self.code is not None:
            d["code"] = self.code
        if self.http_status is not None:
            d["http_status"] = self.http_status
        if self.receipt is not None:
            d["receipt"] = self.receipt
        return d


# ---------------------------------------------------------------------------
# Credential-exact redaction (F6)
# ---------------------------------------------------------------------------

# Exact secret values registered by live clients (the cookie password
# and the base64 Authorization header value). Redaction replaces the
# exact VALUE wherever it appears; non-secret text such as
# "--rpc-cookie-file /path" is never touched.
_ACTIVE_SECRETS = []

# Fallback patterns for credentials this process did not register:
# a full Authorization header value, or "password"/"cookie" KEYED
# assignments (key: secret / key=secret). These match on the key name
# with an explicit separator, not on the bare word "cookie".
_SECRET_KEY_RE = re.compile(
    r'(?i)\b(password|passphrase|authorization|cookie)\s*[:=]\s*("([^"]*)"|(\S+))')


def register_secret(value: str) -> None:
    """Register an exact secret value for redaction (idempotent).

    Only values of a sane credential length are registered as exact
    values: a 1-2 character string would match ordinary text and mangle
    it (e.g. a test password "p" inside the word "passed"). Short
    secrets are still covered by the keyed patterns in redact().
    """
    if value and len(value) >= 8 and value not in _ACTIVE_SECRETS:
        _ACTIVE_SECRETS.append(value)


def redact(text: str) -> str:
    """Remove exact credential values from recorded text.

    Two layers:
    1. every registered exact secret VALUE is replaced wholesale;
    2. keyed assignments (password: x, authorization: y) lose their
       value, with the key preserved.
    Unlike the v1 marker scan, this cannot mangle non-secret argv or
    provenance text such as `--rpc-cookie-file /run/token/cookie`.
    """
    out = str(text)
    for secret in _ACTIVE_SECRETS:
        if secret in out:
            out = out.replace(secret, "<redacted>")
    out = _SECRET_KEY_RE.sub(
        lambda m: m.group(0)[:m.start(2) - m.start()] + "<redacted>", out)
    return out


def read_cookie(path: str) -> tuple:
    """Read `user:pass` credentials from a Core-format cookie file."""
    with open(path, "rb") as fh:
        line = fh.read().decode("utf-8", "replace")
    line = line.rstrip("\r\n")
    if ":" not in line:
        raise ValueError("cookie file %s is not user:pass format" % path)
    user, _, password = line.partition(":")
    if not user or not password:
        raise ValueError("cookie file %s has an empty user or password" % path)
    return user, password


def _mk_receipt(request_id, status, payload):
    """Receipt: HTTP status, response envelope, response id, id match."""
    resp_id = payload.get("id") if isinstance(payload, dict) else None
    envelope = payload.get("jsonrpc") if isinstance(payload, dict) else None
    return {
        "http_status": status,
        "request_id": request_id,
        "response_id": resp_id,
        "response_envelope": envelope,
        "id_match": (
            isinstance(payload, dict)
            and isinstance(resp_id, int) and not isinstance(resp_id, bool)
            and resp_id == request_id
        ),
    }


def _categorize_error(code, message):
    if code == -32601:
        return "method_missing", message
    if code == -32602:
        return "invalid_params", message
    if code in PROTOCOL_ERROR_CODES:
        return "protocol_error", message
    return "server_error_%s" % code, message


class CallRecord:
    """One RPC exchange: result, error, and the protocol receipt."""

    def __init__(self, method, params, envelope, request_id, result, error,
                 receipt, raw_body=None):
        self.method = method
        self.params = params
        self.envelope = envelope
        self.request_id = request_id
        self.result = result
        self.error = error
        self.receipt = receipt
        self.raw_body = raw_body  # internal only: exact numeric lexemes, never serialized

    @property
    def ok(self):
        return self.error is None

    def as_dict(self):
        return {
            "method": self.method,
            "params": self.params,
            "envelope": self.envelope,
            "request_id": self.request_id,
            "result": self.result,
            "error": self.error.as_dict() if self.error is not None else None,
            "receipt": self.receipt,
        }


class JsonRpcClient:
    """Cookie-authed JSON-RPC-over-HTTP client with hard bounds.

    Envelope and ids mirror the pinned consumers: envelope "1.0" with a
    numeric counter starting at 0 (bitcoind-async-client 0.15.0) or
    envelope "2.0" with a numeric nonce starting at 1 (bitcoincore-rpc
    0.19 via jsonrpc 0.18). The first call under each envelope gets that
    client's first id, exactly as the pinned libraries behave.
    """

    def __init__(self, url: str, cookie_file: str, deadline_s: float = 10.0,
                 max_body_bytes: int = 1 << 20):
        if not url.startswith("http://"):
            # Fail closed: no TLS support is fine for loopback lab nodes and
            # an https node would silently fail otherwise.
            raise ValueError("only http:// rpc urls are supported (loopback lab nodes)")
        self.url = url.rstrip("/")
        self.cookie_file = cookie_file
        self.user, self.password = read_cookie(cookie_file)
        self.deadline_s = float(deadline_s)
        self.max_body_bytes = int(max_body_bytes)
        self._auth = base64.b64encode(
            ("%s:%s" % (self.user, self.password)).encode()).decode()
        # Credential-exact redaction: register the actual secret values so
        # they can never leak into any recorded text.
        register_secret(self.password)
        register_secret("%s:%s" % (self.user, self.password))
        register_secret("Basic %s" % self._auth)
        # Counters mirroring the pinned clients: async starts at 0,
        # sync (jsonrpc 0.18 nonce) starts at 1.
        self._next_id = {"1.0": 0, "2.0": 1}

    def _next_request_id(self, envelope: str) -> int:
        rid = self._next_id[envelope]
        self._next_id[envelope] = rid + 1
        return rid

    def call(self, method: str, params: list, envelope: str = "1.0"):
        """Invoke `method`; returns a CallRecord (never raises for RPC
        outcomes; only transport construction errors propagate)."""
        if envelope not in ("1.0", "2.0"):
            raise ValueError("envelope must be \"1.0\" or \"2.0\"")
        rid = self._next_request_id(envelope)
        body = json.dumps({
            "jsonrpc": envelope,
            "id": rid,
            "method": method,
            "params": params,
        }).encode()
        req = urllib.request.Request(
            self.url, data=body, method="POST",
            headers={
                "Content-Type": "application/json",
                "Authorization": "Basic %s" % self._auth,
                "User-Agent": "ap1a-ark-rpc-probe",
            },
        )
        try:
            with urllib.request.urlopen(req, timeout=self.deadline_s) as resp:
                status = resp.status
                raw = resp.read(self.max_body_bytes + 1)
        except urllib.error.HTTPError as exc:
            detail = "%s %s" % (exc.code, exc.reason)
            body = b""
            try:
                body = exc.read(4096)
            except Exception:
                pass
            receipt = None
            # Core (500) and rbitcoin (404 for -32601, etc.) carry the
            # JSON-RPC error object in the body of non-200 responses;
            # the body wins over the HTTP status for categorization.
            payload = self._try_parse(body)
            if isinstance(payload, dict):
                receipt = _mk_receipt(rid, exc.code, payload)
            # C1: shared envelope validation with the 200 path. A
            # parseable JSON-RPC body must be a response object whose id
            # echoes the numeric request id EXACTLY, whatever the HTTP
            # status. Until this holds, no error code is interpreted:
            # a -5 or -32601 behind a mismatched id is a protocol
            # failure, never method or behavior evidence.
            if isinstance(payload, dict) and "id" not in payload:
                return CallRecord(method, params, envelope, rid, None,
                                  RpcCallError(
                                      "bad_jsonrpc",
                                      "not a JSON-RPC response object: %r"
                                      % (payload,),
                                      http_status=exc.code, receipt=receipt),
                                  receipt)
            if isinstance(payload, dict) and not receipt["id_match"]:
                return CallRecord(method, params, envelope, rid, None,
                                  RpcCallError(
                                      "id_mismatch",
                                      "response id %r does not exactly echo "
                                      "the numeric request id %d"
                                      % (receipt["response_id"], rid),
                                      http_status=exc.code, receipt=receipt),
                                  receipt)
            if isinstance(payload, dict) and payload.get("error") is not None:
                err = payload["error"]
                code = err.get("code") if isinstance(err, dict) else None
                message = (err.get("message", "") if isinstance(err, dict)
                           else str(err))
                category, msg = _categorize_error(code, message)
                return CallRecord(method, params, envelope, rid, None,
                                  RpcCallError(category, msg, code=code,
                                               http_status=exc.code,
                                               receipt=receipt),
                                  receipt)
            if body:
                detail += " " + body.decode("utf-8", "replace")
            if exc.code == 401:
                return CallRecord(method, params, envelope, rid, None,
                                  RpcCallError("auth", detail, http_status=401,
                                               receipt=receipt),
                                  receipt)
            return CallRecord(method, params, envelope, rid, None,
                              RpcCallError("http_%d" % exc.code, detail,
                                           http_status=exc.code,
                                           receipt=receipt),
                              receipt)
        except socket.timeout:
            return self._transport_err(method, params, envelope, rid,
                                       "timeout",
                                       "no response within %.1fs deadline"
                                       % self.deadline_s)
        except urllib.error.URLError as exc:
            if isinstance(getattr(exc, "reason", None), socket.timeout):
                return self._transport_err(method, params, envelope, rid,
                                           "timeout",
                                           "no response within %.1fs deadline"
                                           % self.deadline_s)
            return self._transport_err(method, params, envelope, rid,
                                       "transport", str(exc.reason or exc))
        except (ConnectionError, OSError) as exc:
            if isinstance(exc, socket.timeout):
                return self._transport_err(method, params, envelope, rid,
                                           "timeout",
                                           "no response within %.1fs deadline"
                                           % self.deadline_s)
            return self._transport_err(method, params, envelope, rid,
                                       "transport", str(exc))

        if len(raw) > self.max_body_bytes:
            return self._transport_err(method, params, envelope, rid,
                                       "oversized",
                                       "response body exceeds %d bytes"
                                       % self.max_body_bytes)
        try:
            # parse_float=Decimal keeps amount precision exact for contracts.
            payload = json.loads(raw.decode("utf-8"),
                                 parse_float=decimal.Decimal)
        except (ValueError, UnicodeDecodeError) as exc:
            return self._transport_err(method, params, envelope, rid,
                                       "parse", "invalid JSON: %s" % exc)

        receipt = _mk_receipt(rid, status, payload)
        # Mirror the pinned clients' serde: jsonrpc 0.18 Response and the
        # async client's Response<R> both declare result/error as plain
        # Options (an absent key deserializes as None; Bitcoin Core sends
        # "error": null explicitly, rbitcoin omits the key -- both fine)
        # and require the id field. A payload that is not an object, or
        # has no id, is not a JSON-RPC response either client can parse.
        if not isinstance(payload, dict) or "id" not in payload:
            return CallRecord(method, params, envelope, rid, None,
                              RpcCallError(
                                  "bad_jsonrpc",
                                  "not a JSON-RPC response object: %r" % (payload,),
                                  http_status=status, receipt=receipt),
                              receipt)
        # F3: exact numeric id match. A string echo of a numeric request
        # breaks the async client (Response.id is u64); it is a protocol
        # failure, never evidence about method support.
        if not receipt["id_match"]:
            return CallRecord(method, params, envelope, rid, None,
                              RpcCallError(
                                  "id_mismatch",
                                  "response id %r does not exactly echo the "
                                  "numeric request id %d"
                                  % (receipt["response_id"], rid),
                                  http_status=status, receipt=receipt),
                              receipt)
        if payload.get("error") is not None:
            err = payload["error"]
            code = err.get("code") if isinstance(err, dict) else None
            message = (err.get("message", "") if isinstance(err, dict)
                       else str(err))
            category, msg = _categorize_error(code, message)
            return CallRecord(method, params, envelope, rid, None,
                              RpcCallError(category, msg, code=code,
                                           http_status=status,
                                           receipt=receipt),
                              receipt)
        if status != 200:
            return CallRecord(method, params, envelope, rid, None,
                              RpcCallError("http_%d" % status,
                                           "non-200 with null error",
                                           http_status=status,
                                           receipt=receipt),
                              receipt)
        return CallRecord(method, params, envelope, rid, payload.get("result"),
                          None, receipt, raw_body=raw)

    def _transport_err(self, method, params, envelope, rid, category, detail):
        return CallRecord(method, params, envelope, rid, None,
                          RpcCallError(category, detail), None)

    def _try_parse(self, body: bytes):
        if not body:
            return None
        try:
            return json.loads(body.decode("utf-8", "replace"),
                              parse_float=decimal.Decimal)
        except (ValueError, UnicodeDecodeError):
            return None
