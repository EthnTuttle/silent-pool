"""Admit the parent-verified compiled artifact, independently of mutable sources.

This is metadata admission only. It executes no verifier, compiler, service,
wallet, node or test. All crypto remains in the single approved Nix binary.
"""
import json
from pathlib import Path
import subprocess

from common import artifact, encoded, load, require, sha

RECEIPTS = {"r3-fmt-receipt.json", "r3-test-receipt.json", "r3-clippy-receipt.json", "r3-build-receipt.json",
            "nix-r1-receipt.json", "nix-binding.json", "nix-r1-build-full.log", "cli-check-receipt.json", "nix-r1-source-size.json"}


def admit_compiled_verifier(record):
    if record.get('schema') == 'spc.lab.ap5.verifier-reproduction/v1':
        from portable_verifier import admit
        return admit(record)
    approved = load(Path(__file__).with_name("verifier-admission.json"))
    require(approved["schema"] == "spc.lab.ap5.compiled-verifier-admission/v1", "compiled verifier admission schema")
    handoff = load(artifact(approved["handoff"]))
    require(handoff["contract"] == approved["contract"], "verifier contract differs from parent-verified handoff")
    artifact(approved["contract"])
    expected = load(artifact(handoff["verifier"]))
    require(type(record) is dict and {"path", "sha256", "cargo_lock_sha256"} <= set(record) <= set(expected), "compiled verifier descriptor shape")
    require(all(encoded(record[key]) == encoded(expected[key]) for key in record), "verifier descriptor differs from approved artifact")
    executable = artifact(record)
    require(record["sha256"] == approved["binary_sha256"] and executable.is_relative_to("/nix/store"), "unapproved verifier executable")
    require(record["cargo_lock_sha256"] == approved["cargo_lock_sha256"], "unapproved verifier lock")
    binding = load(artifact(expected["nix_binding"]))
    require(binding["binary"] == {"path": record["path"], "sha256": record["sha256"]}
            and binding["source"] == expected["source_root"] == approved["source_root"]
            and binding["cargo_lock_sha256"] == approved["cargo_lock_sha256"], "verifier source/binary/lock receipt mismatch")
    package = Path("/nix/store") / executable.relative_to("/nix/store").parts[0]
    drv = subprocess.check_output(["nix-store", "-q", "--deriver", str(package)], text=True).strip()
    require(drv == expected["deriver"] == binding["deriver"], "verifier actual deriver mismatch")
    env = next(iter(json.loads(subprocess.check_output(["nix", "derivation", "show", drv], text=True)).values()))["env"]
    env = json.loads(env["__json"]) if "__json" in env else env
    require(env.get("src") == binding["source"] and env.get("doCheck") == "1", "verifier actual source/check binding mismatch")
    source = Path(binding["source"])
    rows = binding["source_rows"]
    names = {row["path"] for row in rows}
    require(len(rows) == len(names) == 9, "approved verifier source census mismatch")
    require(names == {str(path.relative_to(source)) for path in source.rglob("*") if path.is_file()}, "actual verifier source census mismatch")
    for row in rows:
        path = source / row["path"]
        require(not Path(row["path"]).is_absolute() and ".." not in Path(row["path"]).parts
                and not path.is_symlink() and sha(path) == row["sha256"] and path.stat().st_size == row["bytes"], "compiled verifier frozen source row mismatch")
    installed = package / "share/ap5-verifier"
    require(sha(installed / "Cargo.lock") == approved["cargo_lock_sha256"], "installed verifier lock mismatch")
    for actual, original in (("main.rs", "src/main.rs"), ("vectors.rs", "src/vectors.rs"),
                             ("Cargo.toml", "Cargo.toml"), ("fixtures/stock-public.json", "fixtures/stock-public.json")):
        require(sha(installed / actual) == sha(source / original), "installed verifier source/fixture mismatch")
    refs = handoff["compiled_receipts"]
    require(set(refs) == RECEIPTS, "compiled verifier receipt census mismatch")
    receipts = {name: artifact(value) for name, value in refs.items()}
    for name in ("r3-fmt-receipt.json", "r3-test-receipt.json", "r3-clippy-receipt.json", "r3-build-receipt.json"):
        value = load(receipts[name])
        require(type(value["exit"]) is int and value["exit"] == 0 and value["source_unchanged"] is True, "verifier Cargo gate failed or source changed")
        log = receipts[name].with_name(name.removesuffix("-receipt.json") + ".log")
        require(sha(log) == value["log_sha256"], "verifier Cargo gate log mismatch")
        if name == "r3-test-receipt.json":
            require("6 passed; 0 failed; 0 ignored" in log.read_text(), "six unconditional verifier tests required")
    nix = load(receipts["nix-r1-receipt.json"])
    require(nix["exit"] == 0 and nix["source_unchanged"] is True and nix["binary_sha256"] == approved["binary_sha256"]
            and nix["installed_lock_sha256"] == approved["cargo_lock_sha256"], "Nix verifier gate binding mismatch")
    require(binding["nix_tests_passed"] == 6 and binding["nix_tests_ignored"] == 0
            and sha(receipts["nix-r1-build-full.log"]) == binding["nix_log_sha256"]
            and "6 passed; 0 failed; 0 ignored" in receipts["nix-r1-build-full.log"].read_text(), "Nix verifier test proof mismatch")
    cli = load(receipts["cli-check-receipt.json"])
    require(cli["binary"] == binding["binary"] and cli["source_unchanged"] is True
            and {c["name"] for c in cli["cases"]} == {"signed-board-positive", "foreign-server-negative"}
            and len(cli["cases"]) == 2 and all(c["expected_outcome_passed"] is True for c in cli["cases"]), "installed verifier CLI receipt mismatch")
    return executable
