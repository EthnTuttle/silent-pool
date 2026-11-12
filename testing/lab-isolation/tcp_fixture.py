#!/usr/bin/env python3
"""Real TCP fixture endpoints for the LP-4a isolation tests.

These are explicitly FIXTURES.  They are not miners, they do no proof of work,
they do not speak SV2 and they carry no pool, node or identity semantics.  They
exist so the lab can prove that a real TCP session is established, that a
sustained fault keeps failing, that the listener stays bound while the link is
faulted, that an already established session is cut by the fault, and that
delivery resumes after the fault is healed.

Modes
-----
server  bind a TCP listener and answer one line per connection
        (--persistent keeps the connection open and echoes every line)
client  open TCP sessions in a retry loop and report every attempt
session connect once, exchange, hold, exchange again (established session test)
probe   single connect attempt, report the outcome as JSON

Every mode appends JSON lines to --log and prints a JSON summary on stdout.
"""

from __future__ import annotations

import argparse
import errno
import json
import os
import re
import signal
import socket
import sys
import time

PONG_PREFIX = "PONG"
_IPV6_RE = re.compile(r"^\[(?P<host>.+)\]:(?P<port>\d+)$")


def now_ms() -> int:
    return int(time.time() * 1000)


def now_iso() -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


class EventLog:
    def __init__(self, path: str, label: str):
        self.path = path
        self.label = label
        self.fh = None
        if path:
            directory = os.path.dirname(os.path.abspath(path))
            if directory:
                os.makedirs(directory, exist_ok=True)
            self.fh = open(path, "a", buffering=1)

    def write(self, event: str, **fields):
        record = {"ts_unix_ms": now_ms(), "ts_iso": now_iso(), "event": event, "label": self.label}
        record.update(fields)
        if self.fh:
            self.fh.write(json.dumps(record, sort_keys=True) + "\n")
        return record


def classify(exc: OSError) -> str:
    code = getattr(exc, "errno", None)
    if code == errno.ECONNREFUSED:
        return "refused"
    if code in (errno.ENETUNREACH, errno.EHOSTUNREACH, errno.EADDRNOTAVAIL):
        return "unreachable"
    if code == errno.ECONNRESET:
        return "reset"
    if code == errno.EPIPE:
        return "broken_pipe"
    if code == errno.ETIMEDOUT:
        return "timeout"
    if isinstance(exc, socket.timeout):
        return "timeout"
    return "error"


def parse_addr(text: str):
    """Parse `host:port` or `[v6addr]:port` into (host, port)."""
    match = _IPV6_RE.match(text)
    if match:
        return match.group("host"), int(match.group("port"))
    if ":" not in text:
        raise ValueError("expected host:port, got %r" % text)
    host, _, port = text.rpartition(":")
    return host, int(port)


def make_socket(family: str):
    if family == "inet6":
        sock = socket.socket(socket.AF_INET6, socket.SOCK_STREAM)
        # Dual stack: the same listener also answers IPv4, which is what a
        # default daemon does and what the partition must cover.
        try:
            sock.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 0)
        except OSError:
            pass
    else:
        sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    return sock


def connect_to(host: str, port: int, family: str, scope_iface: str = "", timeout: float = 2.0):
    sock = make_socket(family)
    sock.settimeout(timeout)
    if family == "inet6":
        scope = 0
        if scope_iface:
            scope = socket.if_nametoindex(scope_iface)
        sock.connect((host, port, 0, scope))
    else:
        sock.connect((host, port))
    return sock


# ------------------------------------------------------------------- server
def run_server(args) -> int:
    host, port = parse_addr(args.bind)
    family = args.family
    if family == "inet6" and ":" in host:
        pass
    log = EventLog(args.log, args.label)
    srv = make_socket(family)
    srv.bind((host, port))
    srv.listen(16)
    srv.settimeout(0.25)
    bound = srv.getsockname()
    log.write("listen", bind="%s:%d" % (bound[0], bound[1]), family=family)
    print(json.dumps({"ok": True, "event": "listen", "bind": "%s:%d" % (bound[0], bound[1]),
                      "family": family, "label": args.label}), flush=True)

    running = {"value": True}

    def _stop(signum, frame):
        running["value"] = False

    signal.signal(signal.SIGTERM, _stop)
    signal.signal(signal.SIGINT, _stop)

    seq = 0
    try:
        while running["value"]:
            try:
                conn, peer = srv.accept()
            except socket.timeout:
                continue
            except OSError:
                break
            seq += 1
            log.write("accept", seq=seq, peer="%s:%d" % (peer[0], peer[1]))
            conn.settimeout(args.timeout)
            try:
                if args.persistent:
                    # Keep the session open and echo every line.  This is what
                    # an established session regression needs.
                    while True:
                        data = conn.recv(4096)
                        if not data:
                            break
                        request = data.decode("utf-8", "replace").strip()
                        log.write("request", seq=seq, bytes=len(data), request=request)
                        response = "%s %s %d %d\n" % (PONG_PREFIX, args.label, seq, now_ms())
                        conn.sendall(response.encode())
                        log.write("respond", seq=seq, bytes=len(response))
                else:
                    data = conn.recv(4096)
                    request = data.decode("utf-8", "replace").strip()
                    log.write("request", seq=seq, bytes=len(data), request=request)
                    response = "%s %s %d %d\n" % (PONG_PREFIX, args.label, seq, now_ms())
                    conn.sendall(response.encode())
                    log.write("respond", seq=seq, bytes=len(response))
            except OSError as exc:
                log.write("session_error", seq=seq, error=str(exc), kind=classify(exc))
            finally:
                try:
                    conn.close()
                except OSError:
                    pass
    finally:
        log.write("exit", accepted=seq)
        try:
            srv.close()
        except OSError:
            pass
    return 0


# ------------------------------------------------------------------- client
def one_attempt(host: str, port: int, family: str, scope_iface: str, timeout: float,
                expect: str, label: str, index: int):
    started = time.monotonic()
    result = {"attempt": index, "target": "%s:%d" % (host, port), "family": family,
              "result": "error", "error": None, "errno": None, "elapsed_ms": 0,
              "response": None}
    sock = None
    try:
        sock = connect_to(host, port, family, scope_iface, timeout)
        sock.sendall(("PING %s %d\n" % (label, index)).encode())
        data = sock.recv(4096)
        text = data.decode("utf-8", "replace").strip()
        result["response"] = text
        if expect and expect not in text:
            result["result"] = "unexpected_response"
        else:
            result["result"] = "ok"
    except socket.timeout:
        result["result"] = "timeout"
    except OSError as exc:
        result["result"] = classify(exc)
        result["error"] = exc.strerror
        result["errno"] = exc.errno
    finally:
        if sock is not None:
            try:
                sock.close()
            except OSError:
                pass
    result["elapsed_ms"] = int((time.monotonic() - started) * 1000)
    return result


def run_client(args) -> int:
    host, port = parse_addr(args.connect)
    log = EventLog(args.log, args.label)
    results = []
    started = time.monotonic()
    index = 0
    deadline = None
    if args.duration > 0:
        deadline = started + args.duration
    while True:
        index += 1
        attempt = one_attempt(host, port, args.family, args.scope_iface, args.timeout,
                              args.expect, args.label, index)
        attempt["ts_unix_ms"] = now_ms()
        attempt["ts_iso"] = now_iso()
        results.append(attempt)
        log.write("attempt", **attempt)
        if args.attempts > 0 and index >= args.attempts:
            break
        if deadline is not None and time.monotonic() >= deadline:
            break
        if args.attempts <= 0 and deadline is None:
            break
        if args.interval > 0:
            time.sleep(args.interval)
    ok = [r for r in results if r["result"] == "ok"]
    summary = {
        "ok": True,
        "label": args.label,
        "target": "%s:%d" % (host, port),
        "family": args.family,
        "attempts": len(results),
        "succeeded": len(ok),
        "failed": len(results) - len(ok),
        "duration_ms": int((time.monotonic() - started) * 1000),
        "results": results,
    }
    print(json.dumps(summary, sort_keys=True))
    return 0


# ------------------------------------------------------------------ session
def run_session(args) -> int:
    """Open one session, exchange, hold, exchange again.

    The hold window is where the fault is applied from the outside, so the
    second exchange shows whether an already established session survives the
    fault.
    """
    host, port = parse_addr(args.connect)
    log = EventLog(args.log, args.label)
    out = {"ok": True, "label": args.label, "target": "%s:%d" % (host, port),
           "family": args.family, "phase1": None, "phase2": None, "phase2_ms": 0}
    sock = None
    try:
        sock = connect_to(host, port, args.family, args.scope_iface, args.timeout)
        sock.settimeout(args.timeout)
        sock.sendall(b"one\n")
        out["phase1"] = sock.recv(4096).decode("utf-8", "replace").strip()
        out["phase1_ts"] = now_iso()
        log.write("phase1", response=out["phase1"])
        time.sleep(args.hold)
        out["phase2_start_ts"] = now_iso()
        started = time.monotonic()
        try:
            sock.sendall(b"two\n")
            out["phase2"] = sock.recv(4096).decode("utf-8", "replace").strip()
        except (socket.timeout, OSError) as exc:
            out["phase2"] = "FAIL:%s:%s" % (type(exc).__name__, exc)
        out["phase2_ms"] = int((time.monotonic() - started) * 1000)
        log.write("phase2", response=out["phase2"], elapsed_ms=out["phase2_ms"])
    except OSError as exc:
        out["phase1"] = "FAIL:%s:%s" % (type(exc).__name__, exc)
        out["phase2"] = "FAIL:%s:%s" % (type(exc).__name__, exc)
    finally:
        if sock is not None:
            try:
                sock.close()
            except OSError:
                pass
    print(json.dumps(out, sort_keys=True))
    return 0


# -------------------------------------------------------------------- probe
def run_probe(args) -> int:
    host, port = parse_addr(args.connect)
    result = one_attempt(host, port, args.family, args.scope_iface, args.timeout,
                          args.expect, args.label or "probe", 1)
    result["ok"] = result["result"] == "ok"
    print(json.dumps(result, sort_keys=True))
    return 0 if result["ok"] else 1


def main() -> int:
    parser = argparse.ArgumentParser(description="LP-4a TCP fixture endpoints")
    sub = parser.add_subparsers(dest="mode", required=True)

    srv = sub.add_parser("server")
    srv.add_argument("--bind", required=True)
    srv.add_argument("--family", choices=("inet", "inet6"), default="inet")
    srv.add_argument("--persistent", action="store_true")
    srv.add_argument("--log", default="")
    srv.add_argument("--label", default="fixture")
    srv.add_argument("--timeout", type=float, default=10.0)
    srv.set_defaults(func=run_server)

    cli = sub.add_parser("client")
    cli.add_argument("--connect", required=True)
    cli.add_argument("--family", choices=("inet", "inet6"), default="inet")
    cli.add_argument("--scope-iface", default="")
    cli.add_argument("--log", default="")
    cli.add_argument("--label", default="client")
    cli.add_argument("--attempts", type=int, default=1)
    cli.add_argument("--duration", type=float, default=0.0)
    cli.add_argument("--timeout", type=float, default=1.0)
    cli.add_argument("--interval", type=float, default=0.2)
    cli.add_argument("--expect", default=PONG_PREFIX)
    cli.set_defaults(func=run_client)

    ses = sub.add_parser("session")
    ses.add_argument("--connect", required=True)
    ses.add_argument("--family", choices=("inet", "inet6"), default="inet")
    ses.add_argument("--scope-iface", default="")
    ses.add_argument("--hold", type=float, default=3.0)
    ses.add_argument("--timeout", type=float, default=3.0)
    ses.add_argument("--log", default="")
    ses.add_argument("--label", default="session")
    ses.set_defaults(func=run_session)

    prb = sub.add_parser("probe")
    prb.add_argument("--connect", required=True)
    prb.add_argument("--family", choices=("inet", "inet6"), default="inet")
    prb.add_argument("--scope-iface", default="")
    prb.add_argument("--timeout", type=float, default=1.0)
    prb.add_argument("--expect", default=PONG_PREFIX)
    prb.add_argument("--label", default="probe")
    prb.set_defaults(func=run_probe)

    args = parser.parse_args()
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
