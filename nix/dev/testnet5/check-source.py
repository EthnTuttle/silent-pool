#!/usr/bin/env python3
"""Offline source/fixture binding check; source must already have been fetched."""
import argparse
import hashlib
import json
from pathlib import Path
import re

FILES = {
    "Cargo.lock": "658f3964c566cc4fbb5281af06bcba4839ec3156b95f04395b2ca8d450650051",
    "Cargo.toml": "93753ef21e0ba3cc9beea17264a6801499eaa070d00fb72725be2887eef73c48",
    "rust-toolchain.toml": "12dc5d64dbd96c1b07be2943241d52d9f6127a79ba47a820455ba7b873fc896a",
    "nix/rbitcoin.nix": "db2541b18bfa38ef98e1336a7cc18ac00960cda1fd43c604cb00a2a564c59609",
    "crates/rbitcoin-primitives/src/lib.rs": "909c4aa238673e34427fa96a6d4e999daa6b81ce3b539735a9dc8018b71d86cd",
    "crates/rbitcoin-consensus/src/params.rs": "0fe4ea9e75265d307a32d132173bd3072ad1ea98e6d8face4d7b682f89a30edc",
    "crates/rbitcoin-net/src/seeds.rs": "258557200f82ad2c2d47c64953f296c3aaeaeb0137153d9bf02f4652c88c44e0",
    "crates/rbitcoin-node/src/config.rs": "4b658da8bc1789b07fd7b5d729a662a4a58a4098440bbc78efd852cec7140049",
    "crates/rbitcoin-rpc/src/methods/mine.rs": "10433e30dc6c0f83c199a068fca414351f87a888a6d4ce2af61d07435e5dce3c",
}


def check(source, root):
    for name, digest in FILES.items():
        if hashlib.sha256((source / name).read_bytes()).hexdigest() != digest:
            raise ValueError("wrong pinned source bytes: " + name)
    pin = json.loads((root / "source.json").read_text())
    network = json.loads((root / "network.json").read_text())
    # These files have already passed byte-exact hashes above; these are
    # assertions over the reviewed format, not a general-purpose TOML parser.
    lock = (source / "Cargo.lock").read_text()
    if re.search(r'^source = "git\+', lock, re.MULTILINE):
        raise ValueError("unexpected Cargo Git dependency")
    toolchain = (source / "rust-toolchain.toml").read_text()
    if f'channel = "{pin["rust_toolchain"]}"' not in toolchain:
        raise ValueError("toolchain drift")
    params = (source / "crates/rbitcoin-consensus/src/params.rs").read_text()
    block_hex = params.split('const TESTNET5_GENESIS_HEX: &str = "', 1)[1].split('";', 1)[0]
    block_hex = re.sub(r"[^0-9a-f]", "", block_hex)
    header = bytes.fromhex(block_hex[:160])
    digest = hashlib.sha256(hashlib.sha256(header).digest()).digest()[::-1].hex()
    if digest != network["genesis_hash"] or header.hex() != network["genesis_header_hex"]:
        raise ValueError("network fixture differs from source genesis")
    primitive = (source / "crates/rbitcoin-primitives/src/lib.rs").read_text()
    for key in ("default_p2p_port", "default_rpc_port"):
        if f'Network::Testnet5 => {network[key]},' not in primitive:
            raise ValueError("network fixture differs from source port")
    service = (source / "crates/rbitcoin-net/src/service.rs").read_text()
    if 'Magic::from_bytes(*b"FIVE")' not in service or bytes.fromhex(network["magic_wire_hex"]) != b"FIVE":
        raise ValueError("network magic drift")
    seeds = (source / "crates/rbitcoin-net/src/seeds.rs").read_text()
    if 'Network::Testnet5 | Network::Regtest => &[]' not in seeds or 'Network::Testnet5 | Network::Regtest => ""' not in seeds:
        raise ValueError("source no-seed fact changed")
    config = (source / "crates/rbitcoin-node/src/config.rs").read_text()
    expected = {
        "network": "testnet5", "datadir": "@DATADIR@", "listen": "0", "max_inbound": "0",
        "no_discover": "1", "no_seeds": "1", "connect": "lapis.erisian.com.au:18353",
        "max_outbound": "8", "milestone": "0", "rpc_listen": "127.0.0.1:18352",
        "rpc_socket": "@DATADIR@/rpc.sock", "rpc_cookie_file": "@DATADIR@/.cookie",
        "rpc_cookie_perms": "owner", "log_level": "info",
    }
    actual = {}
    for line in (root / "node.conf.in").read_text().splitlines():
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        key, value = line.split("=", 1)
        if key in actual or key not in expected or f'"{key}" =>' not in config:
            raise ValueError("unknown/duplicate/unimplemented fixture key: " + key)
        actual[key] = value
    if actual != expected:
        raise ValueError("node-only fixture policy changed")
    return {"schema": "spc.testnet5.source-fixture-check/1", "passed": True,
            "checked_source_files": FILES, "genesis_hash": digest,
            "config_key_source_review": True, "config_parsed_by_node": False,
            "compiled_or_live_node_validation": False}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", required=True, type=Path)
    args = parser.parse_args()
    print(json.dumps(check(args.source, Path(__file__).parent), indent=2, sort_keys=True))
