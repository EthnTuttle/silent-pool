#!/usr/bin/env python3
"""Read-only BIP95 endpoint identity check. Never starts or modifies a node."""
import argparse
import base64
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import stat
import sys
import urllib.error
import urllib.parse
import urllib.request

LIMIT = 1024 * 1024
NETWORK = json.loads(Path(__file__).with_name("network.json").read_text())
METHODS = (
    ("getblockchaininfo", []),
    ("getblockhash", [0]),
    ("getblockheader", [NETWORK["genesis_hash"], False]),
    ("getnetworkinfo", []),
)


def require(ok, reason):
    if not ok:
        raise ValueError(reason)


def unique(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, "duplicate JSON key")
        result[key] = value
    return result


def decode(raw):
    require(len(raw) <= LIMIT, "JSON response too large")
    return json.loads(raw, object_pairs_hook=unique,
                      parse_constant=lambda _: require(False, "nonfinite JSON number"))


def validate(capture):
    require(type(capture) is dict and set(capture) == {m for m, _ in METHODS},
            "incomplete or unknown RPC capture fields")
    chain = capture["getblockchaininfo"]
    require(type(chain) is dict and chain.get("chain") == NETWORK["chain"], "wrong chain")
    require(type(chain.get("blocks")) is int and chain["blocks"] >= 0, "unknown height")
    require(type(chain.get("initialblockdownload")) is bool, "unknown sync state")
    require(capture["getblockhash"] == NETWORK["genesis_hash"], "wrong genesis")
    header_hex = capture["getblockheader"]
    require(type(header_hex) is str and header_hex == NETWORK["genesis_header_hex"],
            "wrong genesis header")
    header = bytes.fromhex(header_hex)
    digest = hashlib.sha256(hashlib.sha256(header).digest()).digest()[::-1].hex()
    require(len(header) == 80 and digest == NETWORK["genesis_hash"], "genesis hash mismatch")
    info = capture["getnetworkinfo"]
    require(type(info) is dict and type(info.get("version")) is int
            and info["version"] > 0 and type(info.get("subversion")) is str
            and bool(info["subversion"]), "unknown node version")
    return {
        "schema": "spc.testnet5.node-identity/1",
        "chain": NETWORK["chain"], "genesis_hash": digest,
        "height": chain["blocks"], "initialblockdownload": chain["initialblockdownload"],
        "reported_version": info["version"], "reported_subversion": info["subversion"],
        "network_identity_verified": True,
        "full_consensus_validation_proven": False,
        "deployment_ready": False,
    }


def local_url(url):
    parsed = urllib.parse.urlsplit(url)
    require(parsed.scheme == "http" and parsed.path in ("", "/") and not parsed.query
            and not parsed.fragment and parsed.username is None and parsed.password is None,
            "expected loopback HTTP RPC origin without credentials or path")
    require(parsed.hostname is not None and ipaddress.ip_address(parsed.hostname).is_loopback,
            "RPC must use a literal loopback address")
    require(parsed.port is not None and 1 <= parsed.port <= 65535, "explicit RPC port required")
    return url


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise ValueError("RPC redirect refused")


def read_cookie(path):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(fd, "rb") as stream:
        meta = os.fstat(stream.fileno())
        require(stat.S_ISREG(meta.st_mode) and meta.st_uid == os.getuid()
                and meta.st_mode & 0o077 == 0, "cookie must be an owned private regular file")
        cookie = stream.read(4097).strip()
    require(0 < len(cookie) <= 4096 and b":" in cookie
            and b"\n" not in cookie and b"\r" not in cookie, "invalid cookie")
    return cookie


def collect(url, cookie):
    local_url(url)
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
    auth = "Basic " + base64.b64encode(cookie).decode("ascii")
    capture = {}
    for ident, (method, params) in enumerate(METHODS, 1):
        request = urllib.request.Request(url, json.dumps({
            "jsonrpc": "2.0", "id": ident, "method": method, "params": params,
        }).encode(), {"Content-Type": "application/json", "Authorization": auth})
        with opener.open(request, timeout=10) as response:
            value = decode(response.read(LIMIT + 1))
        require(type(value) is dict and type(value.get("id")) is int
                and value["id"] == ident and value.get("error") is None
                and "result" in value, "RPC failed or response correlation mismatch")
        capture[method] = value["result"]
        # Stop immediately at a wrong network instead of probing more methods.
        if method == "getblockchaininfo":
            require(type(capture[method]) is dict
                    and capture[method].get("chain") == NETWORK["chain"], "wrong chain")
        if method == "getblockhash":
            require(capture[method] == NETWORK["genesis_hash"], "wrong genesis")
    return capture


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--capture", type=Path, help="offline JSON method-result capture; not live evidence")
    mode.add_argument("--rpc-url", help="explicit future owned loopback RPC endpoint")
    parser.add_argument("--cookie", type=Path, help="owned private Core cookie file; never inline")
    args = parser.parse_args()
    require(bool(args.cookie) == bool(args.rpc_url), "--cookie required only with --rpc-url")
    if args.capture:
        with args.capture.open("rb") as stream:
            capture = decode(stream.read(LIMIT + 1))
    else:
        capture = collect(local_url(args.rpc_url), read_cookie(args.cookie))
    result = validate(capture)
    result["observation_mode"] = "offline_capture" if args.capture else "live_read_only_rpc"
    print(json.dumps(result, sort_keys=True))


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, urllib.error.URLError):
        # Do not echo remote text, the cookie, or user-supplied credentials.
        print("Testnet5 node identity validation failed", file=sys.stderr)
        sys.exit(1)
