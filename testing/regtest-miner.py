#!/usr/bin/env python3
"""Tiny SV1 miner for the regtest harness. Mines `--shares` accepted shares.

It renders the job exactly as the pool sent it (coinb1 + extranonce1 +
extranonce2 + coinb2), searches a nonce against the pool's nbits, and submits
the standard params. This is a test fixture, not a miner implementation.
"""

import argparse
import hashlib
import json
import socket
import struct
import sys
import time


def dsha(b: bytes) -> bytes:
    return hashlib.sha256(hashlib.sha256(b).digest()).digest()


def target_from_bits(bits: int) -> int:
    exponent = bits >> 24
    mantissa = bits & 0xFFFFFF
    if exponent <= 3:
        return mantissa >> (8 * (3 - exponent))
    return mantissa << (8 * (exponent - 3))


class Conn:
    def __init__(self, host: str, port: int):
        self.s = socket.create_connection((host, port), timeout=10)
        self.buf = b""

    def send(self, obj):
        self.s.sendall(json.dumps(obj).encode() + b"\n")

    def read(self):
        while b"\n" not in self.buf:
            chunk = self.s.recv(65536)
            if not chunk:
                raise RuntimeError("pool closed the connection")
            self.buf += chunk
        line, self.buf = self.buf.split(b"\n", 1)
        return json.loads(line)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, required=True)
    ap.add_argument("--address", required=True)
    ap.add_argument("--shares", type=int, default=1)
    args = ap.parse_args()

    c = Conn(args.host, args.port)
    c.send({"id": 1, "method": "mining.subscribe", "params": []})
    sub = c.read()
    extranonce1 = sub["result"][1]
    en2_size = int(sub["result"][2])
    c.send({"id": 2, "method": "mining.authorize", "params": [args.address, "x"]})
    auth = c.read()
    if auth.get("result") is not True:
        print("authorize failed", auth, file=sys.stderr)
        return 1

    mined = 0
    deadline = time.time() + 120
    while mined < args.shares and time.time() < deadline:
        msg = c.read()
        if msg.get("method") != "mining.notify":
            continue
        p = msg["params"]
        job_id = p[0]
        prev_hash = bytes.fromhex(p[1])[::-1]
        coinb1 = bytes.fromhex(p[2])
        coinb2 = bytes.fromhex(p[3])
        branches = [bytes.fromhex(b) for b in p[4]]
        version = int(p[5], 16)
        nbits = int(p[6], 16)
        ntime = int(p[7], 16)

        en2 = bytes([0x42 + mined]) * en2_size
        coinbase = coinb1 + bytes.fromhex(extranonce1) + en2 + coinb2
        merkle = dsha(coinbase)
        for b in branches:
            merkle = dsha(merkle + b)

        target = target_from_bits(nbits)
        nonce = 0
        while nonce < 50_000_000:
            header = (
                struct.pack("<I", version)
                + prev_hash
                + merkle
                + struct.pack("<I", ntime)
                + struct.pack("<I", nbits)
                + struct.pack("<I", nonce)
            )
            h = dsha(header)
            if int.from_bytes(h, "little") <= target:
                break
            nonce += 1
        else:
            print("no nonce found", file=sys.stderr)
            return 1

        c.send(
            {
                "id": 100 + mined,
                "method": "mining.submit",
                "params": [args.address, job_id, en2.hex(), f"{ntime:08x}", f"{nonce:08x}"],
            }
        )
        res = c.read()
        if res.get("result") is not True:
            print("submit failed", res, file=sys.stderr)
            return 1
        print(f"share {mined} accepted nonce={nonce:08x} job={job_id}", flush=True)
        mined += 1

    print(f"mined {mined} share(s)", flush=True)
    return 0 if mined >= args.shares else 1


if __name__ == "__main__":
    raise SystemExit(main())
