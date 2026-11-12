#!/usr/bin/env python3
"""Connect probe executed inside an owned network namespace.

The backend never changes its own network namespace and never forks a thread
that could inherit a namespace.  It runs this probe as an external command
through `nsenter --net`, so the probe always runs in the target namespace and
the controller process is unaffected.

Output: one JSON object on stdout.

    {"outcome": "connect_failed"|"connected"|"probe_error",
     "errno": <int|null>, "error": <string|null>,
     "elapsed_ms": <int>, "target": "<host>", "port": <int>,
     "family": "inet"|"inet6"}
"""

from __future__ import annotations

import argparse
import errno
import json
import socket
import sys
import time


def main() -> int:
    parser = argparse.ArgumentParser(description="namespace connect probe")
    parser.add_argument("--host", required=True)
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--family", choices=("inet", "inet6"), default="inet")
    parser.add_argument("--timeout", type=float, default=1.0)
    parser.add_argument("--iface", default="")
    args = parser.parse_args()

    family = socket.AF_INET if args.family == "inet" else socket.AF_INET6
    result = {
        "outcome": "probe_error",
        "errno": None,
        "error": None,
        "elapsed_ms": 0,
        "target": args.host,
        "port": args.port,
        "family": args.family,
    }
    started = time.monotonic()
    sock = socket.socket(family, socket.SOCK_STREAM)
    sock.settimeout(args.timeout)
    try:
        if family == socket.AF_INET6:
            scope = 0
            if args.iface:
                try:
                    scope = socket.if_nametoindex(args.iface)
                except OSError as exc:
                    result["error"] = "if_nametoindex(%s): %s" % (args.iface, exc)
                    result["errno"] = exc.errno
                    print(json.dumps(result, sort_keys=True))
                    return 2
            sock.connect((args.host, args.port, 0, scope))
        else:
            sock.connect((args.host, args.port))
        result["outcome"] = "connected"
    except socket.timeout:
        result["outcome"] = "connect_failed"
        result["errno"] = errno.ETIMEDOUT
        result["error"] = "timed out"
    except OSError as exc:
        result["outcome"] = "connect_failed"
        result["errno"] = exc.errno
        result["error"] = exc.strerror
    finally:
        try:
            sock.close()
        except OSError:
            pass
    result["elapsed_ms"] = int((time.monotonic() - started) * 1000)
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
