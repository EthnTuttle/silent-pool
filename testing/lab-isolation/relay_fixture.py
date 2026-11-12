#!/usr/bin/env python3
"""Userspace TCP relay: the explicitly weaker LP-4a fallback backend.

This relay exists only for hosts where the network-namespace backend cannot be
proven.  It provides a CONTROLLED LINK OUTAGE: while the relay is cut, a
client that goes through the relay cannot reach the listener, and after the
relay is restored delivery resumes.

It is NOT an isolation boundary.  The listener it forwards to is still
reachable directly, so a "no bypass" assertion CANNOT be satisfied by this
backend.  The relay test asserts exactly that scope: the outage is proven,
and the absence of isolation is proven too, so an LP-3 preflight can downgrade
those assertions to `inconclusive` instead of passing them.

Usage:
  relay_fixture.py relay --listen IP:PORT --target IP:PORT \
      --state-file FILE --log FILE [--label NAME]

The state file contains "up" or "down".  It is polled continuously.
"""

from __future__ import annotations

import argparse
import json
import os
import signal
import socket
import sys
import threading
import time


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


def read_state(path: str) -> str:
    """Read the relay state.  Anything unreadable means the link is DOWN.

    A missing or corrupt state file must fail closed: the relay must not
    forward traffic because its control file could not be read.
    """
    try:
        with open(path, "r") as fh:
            value = fh.read().strip().lower()
    except OSError:
        return "down"
    if value not in ("up", "down"):
        return "down"
    return value


def pump(src: socket.socket, dst: socket.socket, log: EventLog, sid: int):
    try:
        while True:
            data = src.recv(65536)
            if not data:
                break
            dst.sendall(data)
    except OSError:
        pass
    finally:
        for sock in (src, dst):
            try:
                sock.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass


def handle_session(client: socket.socket, target, log: EventLog, sid: int):
    upstream = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    upstream.settimeout(5.0)
    try:
        upstream.connect(target)
    except OSError as exc:
        log.write("upstream_error", sid=sid, error=str(exc))
        client.close()
        upstream.close()
        return
    log.write("session_open", sid=sid)
    t1 = threading.Thread(target=pump, args=(client, upstream, log, sid), daemon=True)
    t2 = threading.Thread(target=pump, args=(upstream, client, log, sid), daemon=True)
    t1.start()
    t2.start()
    t1.join()
    t2.join()
    log.write("session_close", sid=sid)
    try:
        client.close()
    except OSError:
        pass
    try:
        upstream.close()
    except OSError:
        pass


def run_relay(args) -> int:
    host, _, port_s = args.listen.rpartition(":")
    port = int(port_s)
    target_host, _, target_port_s = args.target.rpartition(":")
    target = (target_host, int(target_port_s))
    log = EventLog(args.log, args.label)
    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind((host, port))
    srv.listen(16)
    srv.settimeout(0.2)
    log.write("relay_listen", listen="%s:%d" % (host, port), target="%s:%d" % target)

    running = {"value": True}
    sessions = []
    state = "up"
    sid = 0

    def _stop(signum, frame):
        running["value"] = False

    signal.signal(signal.SIGTERM, _stop)
    signal.signal(signal.SIGINT, _stop)

    while running["value"]:
        new_state = read_state(args.state_file)
        if new_state != state:
            log.write("state_change", **{"from": state, "to": new_state})
            state = new_state
            if state == "down":
                for sock in sessions:
                    try:
                        sock.shutdown(socket.SHUT_RDWR)
                    except OSError:
                        pass
                    try:
                        sock.close()
                    except OSError:
                        pass
                sessions = []
        try:
            conn, peer = srv.accept()
        except socket.timeout:
            continue
        except OSError:
            break
        sid += 1
        if state == "down":
            log.write("cut_refused", sid=sid, peer="%s:%d" % (peer[0], peer[1]))
            try:
                conn.close()
            except OSError:
                pass
            continue
        sessions.append(conn)
        thread = threading.Thread(target=handle_session, args=(conn, target, log, sid), daemon=True)
        thread.start()
    try:
        srv.close()
    except OSError:
        pass
    log.write("relay_exit")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description="LP-4a userspace TCP relay fallback")
    sub = parser.add_subparsers(dest="mode", required=True)
    relay = sub.add_parser("relay")
    relay.add_argument("--listen", required=True)
    relay.add_argument("--target", required=True)
    relay.add_argument("--state-file", required=True)
    relay.add_argument("--log", default="")
    relay.add_argument("--label", default="relay")
    relay.set_defaults(func=run_relay)
    args = parser.parse_args()
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
