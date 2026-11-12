#!/usr/bin/env python3
"""Build the annotated LP-runtime review manifest.

Historical review-session tool: the INPUTS below are local packet
archives from the 2026-11 lab review campaigns, not fresh-setup
preconditions and not part of the lab runtime. Reproducing a review
manifest requires those preserved archives; building or running the lab
never invokes this script.

The full aggregate covers every reviewed file. `code_snapshot_sha256` is the
same computation with mutable run-report files excluded; freeze that hash
before building/running the proof, then add the report afterwards. Input
manifests are recorded verbatim; LP-4a has no snapshot hash and is recorded by
its recipe hash instead.
"""

import argparse
import hashlib
import json
import os
import subprocess
from pathlib import Path

BASE = "b0ab4ca34fda07c61a6335d05f598eaacc69678c"
INPUTS = {
    "lab-lp1": "/tmp/opencode/lab-lp1-review-manifest.json",
    "lab-lp2": "/tmp/opencode/lab-lp2-review-manifest.json",
    "lab-lp3-r3": "/tmp/opencode/lab-lp3-r3-review-manifest.json",
    "lab-lp4a": "/tmp/opencode/lab-lp4a-review-manifest.json",
    "lab-live": "/tmp/opencode/lab-live-review-manifest.json",
}
MUTABLE = {
    "docs/reviews/2026-11-06-lab-runtime-implementation.md",
    "docs/reviews/2026-11-06-lab-live-fable-review.md",
}


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def aggregate(files):
    body = json.dumps({"base": BASE, "files": files}, sort_keys=True,
                      separators=(",", ":")).encode()
    return hashlib.sha256(body).hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path,
                        default=Path("/tmp/opencode/lab-runtime-review-manifest.json"))
    args = parser.parse_args()

    inputs = {}
    hashes = {}
    for name, path in INPUTS.items():
        data = json.loads(Path(path).read_text())
        hashes[name] = {item["path"]: item["sha256"] for item in data["files"]}
        inputs[name] = {
            "file_count": len(data["files"]),
            "snapshot_sha256": data.get("snapshot_sha256"),
            "recipe_sha256": data.get("recipe_sha256"),
        }

    changed = set(subprocess.check_output(
        ["git", "diff", "--name-only", BASE], text=True).splitlines())
    untracked = set(subprocess.check_output(
        ["git", "ls-files", "--others", "--exclude-standard"], text=True).splitlines())
    paths = sorted(path for path in changed | untracked
                   if path and not path.startswith("target/")
                   and not path.startswith("result/")
                   and "__pycache__" not in path)
    files = []
    for path in paths:
        if not os.path.isfile(path):
            continue
        digest = sha256(path)
        owners = [name for name, values in hashes.items() if path in values]
        if owners:
            same = all(hashes[name][path] == digest for name in owners)
            source = owners[0]
            change = ("unchanged-from-approved-input" if same
                      else "combined-or-edited-after-approved-input")
        else:
            source = "integration"
            change = "added-by-runtime-integration"
        files.append({"path": path, "sha256": digest, "source": source,
                      "change": change})

    code_files = [item for item in files if item["path"] not in MUTABLE]
    manifest = {
        "schema": "spc.lab.runtime_review_manifest.v1",
        "base": BASE,
        "inputs": inputs,
        "files": files,
        "counts": {
            "total": len(files),
            "from_inputs_unchanged": sum(
                1 for item in files if item["change"] == "unchanged-from-approved-input"),
            "combined_or_edited": sum(
                1 for item in files if item["change"] == "combined-or-edited-after-approved-input"),
            "added_by_integration": sum(
                1 for item in files if item["change"] == "added-by-runtime-integration"),
        },
        "recipe": {
            "aggregate": ("sha256(json.dumps({'base': base, 'files': files}, "
                          "sort_keys=True, separators=(',', ':')))"),
            "fields": ["path", "sha256", "source", "change"],
            "code_subset": "all files except the mutable run report and the copied review context",
        },
        "snapshot_sha256": aggregate(files),
        "code_snapshot_sha256": aggregate(code_files),
    }
    args.output.write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps({
        "output": str(args.output),
        "counts": manifest["counts"],
        "snapshot_sha256": manifest["snapshot_sha256"],
        "code_snapshot_sha256": manifest["code_snapshot_sha256"],
    }, indent=2))


if __name__ == "__main__":
    main()
