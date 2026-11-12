#!/usr/bin/env python3
"""Copy only public frozen hex literals from the exact stock Git object."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess

REVISION = "c419159fa54085572f8ca72d2ba1716556a9d3eb"
SOURCE = "lib/src/test_util/vectors.rs"
SOURCE_SHA256 = "7926e40e613fb9817d3710f78b34ae5d19eff6dc2aced25be25a9f37831417fa"
PUBLIC = ("anchor_tx", "board_vtxo", "arkoor_htlc_out_vtxo", "arkoor2_vtxo",
          "round_tx", "round1_vtxo", "round2_vtxo", "arkoor3_vtxo")


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--bark-repository", type=Path, required=True)
    p.add_argument("--out", type=Path, required=True)
    args = p.parse_args()
    raw = subprocess.check_output(["git", "-C", str(args.bark_repository), "show", REVISION + ":" + SOURCE])
    assert hashlib.sha256(raw).hexdigest() == SOURCE_SHA256, "pinned Git source mismatch"
    constants = raw.decode().split("pub static ref VTXO_VECTORS:", 1)[1].split("\n\t};", 1)[0]
    public = {}
    for name in PUBLIC:
        match = re.findall(r"\b" + name + r': (?:ProtocolEncoding::)?deserialize_hex\("([0-9a-f]+)"\)', constants)
        assert len(match) == 1, name
        bytes.fromhex(match[0])
        public[name] = match[0]
    value = {"schema": "spc.lab.ap5.public-stock-vectors/v1", "revision": REVISION,
             "fixture_kind": "synthetic_signed_conformance",
             "source_path": SOURCE, "source_sha256": SOURCE_SHA256,
             "provenance": "stock frozen public crypto fixtures; not a live chain or funded AP5 receipt",
             "public": public,
             "public_sha256": {k: hashlib.sha256(bytes.fromhex(v)).hexdigest() for k, v in public.items()}}
    args.out.parent.mkdir(parents=True, exist_ok=True)
    with args.out.open("x") as out:
        json.dump(value, out, sort_keys=True, indent=2)
        out.write("\n")


if __name__ == "__main__":
    main()
