#!/usr/bin/env python3
"""Extract the exact locked dependency closure; never choose a new version.

Cargo may prune unused feature edges during offline metadata normalization.
The audit subcommand requires every resulting package/source/checksum to be
present in this parent lock and every edge to remain within its original set.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import tomllib

ROOTS = ("anyhow", "ark-lib", "bitcoin", "hex", "serde", "serde_json")
REVISION = "c419159fa54085572f8ca72d2ba1716556a9d3eb"


def identity(package):
    return (package["name"], package["version"], package.get("source", ""))


def resolve(edge, packages):
    match = re.fullmatch(r"([^ ]+)(?: ([^ ]+))?(?: \((.+)\))?", edge)
    assert match, edge
    name, version, source = match.groups()
    found = [p for p in packages if p["name"] == name and (version is None or p["version"] == version)
             and (source is None or p.get("source") == source)]
    assert len(found) == 1, (edge, [identity(p) for p in found])
    return found[0]


def closure(packages):
    selected = {}
    todo = [resolve(name, packages) for name in ROOTS]
    while todo:
        package = todo.pop()
        key = identity(package)
        if key in selected:
            continue
        assert package.get("source"), "workspace/path package in external closure"
        selected[key] = package
        todo += [resolve(edge, packages) for edge in package.get("dependencies", [])]
    ark = resolve("ark-lib", packages)
    assert ark["version"] == "0.7.1" and ark["source"] == f"git+https://gitlab.com/ark-bitcoin/bark.git?rev={REVISION}#{REVISION}"
    return selected


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("action", choices=("seed", "audit"))
    parser.add_argument("--workspace-lock", type=Path, required=True)
    parser.add_argument("--workspace-sha256", required=True)
    parser.add_argument("--lock", type=Path, required=True)
    args = parser.parse_args()
    raw = args.workspace_lock.read_bytes()
    assert hashlib.sha256(raw).hexdigest() == args.workspace_sha256, "workspace lock changed"
    parent = tomllib.loads(raw.decode())["package"]
    selected = closure(parent)
    if args.action == "seed":
        blocks = re.split(r"(?=\[\[package\]\])", raw.decode())[1:]
        kept = [b.rstrip() for b in blocks if identity(tomllib.loads(b)["package"][0]) in selected]
        root = '[[' + 'package]]\nname = "ap5-vtxo-verifier"\nversion = "0.1.0"\ndependencies = [\n'
        root += ''.join(' ' + json.dumps(n) + ',\n' for n in ROOTS) + ']'
        with args.lock.open("x") as file:
            file.write('# Exact workspace closure seed; normalize only with frozen offline metadata.\nversion = 4\n\n' + '\n\n'.join([root, *kept]) + '\n')
    else:
        normalized = tomllib.loads(args.lock.read_text())["package"]
        roots = [p for p in normalized if p["name"] == "ap5-vtxo-verifier"]
        assert len(roots) == 1 and not roots[0].get("source")
        for p in normalized:
            if p is roots[0]:
                assert {resolve(e, normalized)["name"] for e in p["dependencies"]} == set(ROOTS)
                continue
            original = selected[identity(p)]
            assert p.get("checksum") == original.get("checksum"), identity(p)
            before = {identity(resolve(e, parent)) for e in original.get("dependencies", [])}
            after = {identity(resolve(e, normalized)) for e in p.get("dependencies", [])}
            assert after <= before, (identity(p), "new dependency edge")
    print(json.dumps({"workspace_lock_sha256": args.workspace_sha256,
                      "lock_sha256": hashlib.sha256(args.lock.read_bytes()).hexdigest(),
                      "parent_closure_packages": len(selected), "action": args.action}, sort_keys=True))


if __name__ == "__main__":
    main()
