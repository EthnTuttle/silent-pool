#!/usr/bin/env python3
"""Read-only stock-Bitcoin ladder archive check of the actual job adapter.

Reads raw block ancestry from the archived Bitcoin Core blk file (including
its supported disk XOR format). No RPC, live process or mining operation.
"""
import argparse
import hashlib
import json
import re
from pathlib import Path

from oracle import Chain, Inconclusive, Invalid, Policy, require, sha256d, strict_json, unhex
from pool_source import PoolSource
from source_jobs import verify_job, verify_publication
from source_capture import full_window_evidence


def audit(archive, output):
    archive, output = Path(archive), Path(output)
    output.mkdir(mode=0o700, exist_ok=False)
    inputs = {}
    def read(path, maximum):
        before = path.stat()
        require(not path.is_symlink() and before.st_size <= maximum, "archive input symlink/bound")
        raw = path.read_bytes()
        after = path.stat()
        require((before.st_ino, before.st_size, before.st_mtime_ns) == (after.st_ino, after.st_size, after.st_mtime_ns),
                "archive changed during read")
        inputs[str(path)] = {"sha256": hashlib.sha256(raw).hexdigest(), "bytes": len(raw)}
        return raw
    receipt = strict_json(read(archive / "receipt.json", 1024 * 1024))
    require(receipt["scope"] == "native_sv2_ladder_component"
            and receipt["node_sha256"] == "92fef1310d77eb9c74498e2cdba24172b76dd66ea53261a58004ad0e6240d997",
            "archive must be the bound stock-Bitcoin component")
    block_dir = archive / "private/bitcoin/regtest/blocks"
    key = read(block_dir / "xor.dat", 8)
    require(len(key) == 8, "Bitcoin archive XOR key length")
    files = sorted(block_dir.glob("blk*.dat"))
    require(files and len(files) <= 4, "bounded archive block-file census")
    blocks = {}
    for path in files:
        physical = read(path, 64 * 1024 * 1024)
        data = bytes(value ^ key[index % 8] for index, value in enumerate(physical))
        position = 0
        while position < len(data):
            if physical[position:position + 8] == bytes(8):
                require(not any(physical[position:]), "nonzero data after block-file padding")
                break
            require(data[position:position + 4] == bytes.fromhex("fabfb5da"), "archive regtest block-file magic")
            size = int.from_bytes(data[position + 4:position + 8], "little")
            require(81 <= size <= 4 * 1024 * 1024 and position + 8 + size <= len(data), "archive raw block size/tail")
            raw = data[position + 8:position + 8 + size]
            block_hash = sha256d(raw[:80])[::-1].hex()
            require(block_hash not in blocks or blocks[block_hash] == raw, "archive same hash/different block bytes")
            blocks[block_hash] = raw
            position += size + 8
    p = Policy("0f9188f13cb7b2c71f2a335e3a4fc328bf5beb436012afca590b1a11466e2206", 0x207fffff,
               0x207fffff, None, True, True, 600, 1209600, 2016, False, None, 5000000000, 150, 100)
    tip, ancestry, seen = receipt["block_hash"], [], set()
    while True:
        require(tip in blocks and tip not in seen and len(seen) < 4096, "archive parent ancestry missing/cyclic")
        raw = blocks[tip]; ancestry.append(raw); seen.add(tip)
        if tip == p.genesis:
            break
        tip = raw[4:36][::-1].hex()
    chain = Chain(p, ancestry[::-1])
    raw_record = strict_json(read(archive / "raw-block.json", 4 * 1024 * 1024))
    require(ancestry[0] == unhex(raw_record["raw_hex"]), "raw block record differs from archived store")
    log = read(archive / "private/pool.log", 128 * 1024 * 1024)
    source = PoolSource(log, independently_captured_log_sha256=hashlib.sha256(log).hexdigest())
    views = {chain.tip["hash"]: chain}
    publication = verify_publication(source, receipt["block_hash"], views, p)
    job = verify_job(source, tuple(publication["job_key"]), views, p)
    config=read(archive / "private/pool.toml",4*1024*1024).decode()
    def scalar(name):
        values=re.findall(r"^\s*"+re.escape(name)+r"\s*=\s*(.*?)\s*$",config,re.MULTILINE)
        require(len(values)==1,"archive work profile scalar missing/ambiguous")
        return strict_json(values[0])
    full=full_window_evidence(source,views,p,{"reference_target":scalar("work_reference_target"),"window_n":scalar("window_n")})
    component = {"schema": "spc-lab-archive-source-job/1", "status": "PASSED_JOB_PUBLICATION_COMPONENT_ONLY",
        "scope": "stock-Bitcoin native-SV2 archived component; not rbitcoin T3/T4b or Ark",
        "inputs": inputs, "raw_ancestry_blocks": len(ancestry), "publication": publication,
        "job": job["job"], "eventual_full_window_coverage": full,
        "missing_components": ["complete_RULEB_economic_history", "runtime_prepublication_proof_feed",
                                                   "actual_witness_refresh_in_this_nonwitness_archive"],
        "full_entitlement_verified": False, "full_T3_T4b_acceptance": False, "Ark_acceptance": False}
    path = output / "receipt.json"
    path.write_text(json.dumps(component, sort_keys=True, indent=2) + "\n")
    path.chmod(0o600)
    print(json.dumps({"receipt": str(path), "status": component["status"], "raw_ancestry_blocks": len(ancestry)}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", required=True); parser.add_argument("--output", required=True)
    args = parser.parse_args()
    output = Path(args.output)
    require(not output.exists(), "source archive output must be fresh")
    try:
        audit(args.archive, args.output)
    except (Inconclusive, Invalid, OSError, ValueError, KeyError, TypeError) as error:
        status = "FAILED" if isinstance(error, Invalid) else "INCONCLUSIVE"
        if output.is_dir():
            path = output / "receipt.json"
            path.write_text(json.dumps({"schema": "spc-lab-archive-source-job/1", "status": status,
                "error_class": type(error).__name__, "missing_component": getattr(error, "component", None),
                "full_T3_T4b_acceptance": False, "Ark_acceptance": False}) + "\n")
            path.chmod(0o600)
        raise SystemExit(1 if status == "FAILED" else 2)


if __name__ == "__main__":
    main()
