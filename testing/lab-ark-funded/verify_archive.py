#!/usr/bin/env python3
"""Offline AP5 component replay from a parent-pinned immutable event seal."""
import argparse
import hashlib
from pathlib import Path
import re

from common import artifact, encoded, require, sha, check_bundle
from cleanup import validate_cleanup
from component_verify import final_verdict, mandatory_components, replay_component, native_funding_verdict
from source_scope import NATIVE_COMPONENTS
from pool_control import strict_json, Operation
from board_proof import bind_retained_proof
from scenario import USERS, POOLS


def read_archive(seal_path, seal_hash):
    require(sha(seal_path) == seal_hash, "AP5 external seal mismatch")
    seal = strict_json(Path(seal_path).read_bytes())
    require(set(seal) == {"root", "events_sha256", "last_seq", "last_digest"}, "AP5 seal shape")
    root = Path(seal["root"])
    require(root.is_absolute() and root.resolve() == root and not root.is_symlink(), "AP5 root binding")
    raw = (root / "events.jsonl").read_bytes()
    require(len(raw) <= 32 * 1024 * 1024 and raw.endswith(b"\n") and hashlib.sha256(raw).hexdigest() == seal["events_sha256"], "AP5 event stream seal/tail mismatch")
    previous, records = "0" * 64, []
    for seq, line in enumerate(raw.splitlines(), 1):
        row = strict_json(line)
        require(set(row) == {"seq", "file", "sha256", "previous", "unix_ms", "record_sha256"}, "AP5 event fields")
        require(row["seq"] == seq and row["previous"] == previous, "AP5 event chain discontinuity")
        previous = hashlib.sha256(encoded({k: v for k, v in row.items() if k != "record_sha256"})).hexdigest()
        require(row["record_sha256"] == previous, "AP5 event digest mismatch")
        name = re.fullmatch(r"([0-9]{6})-([a-z0-9-]+)\.json", row["file"])
        require(name and int(name[1]) == seq, "AP5 event path/sequence mismatch")
        path = root / row["file"]
        require(not path.is_symlink() and path.stat().st_size <= 64 * 1024 * 1024 and sha(path) == row["sha256"], "AP5 artifact seal mismatch")
        records.append((name[2], strict_json(path.read_bytes()), row))
    require(len(records) == seal["last_seq"] and previous == seal["last_digest"], "AP5 terminal seal mismatch")
    require(records and records[-1][0] == "result" and records[-1][1]["schema"] == "spc.lab.ap5.result/v2", "AP5 final v2 result required")
    return root, records


def verify_pool_events(records):
    """Replay real request/reply events before trusting embedded component copies."""
    operations, latest = {}, {}
    for label, value, _ in records:
        if label == "pool-wallet-request":
            key = value["operation_id"]
            require(key not in operations, "pool operation id reused for another request event")
            operations[key] = Operation(value)
        elif label == "pool-wallet-result":
            key = value["operation_id"]
            require(key in operations, "pool reply has no preceding sealed request")
            latest[key] = operations[key].observe(encoded(value))
            if value["state"] == "proof_available":
                request = operations[key].request
                original = request["original_board_operation_id"]
                require(original in latest and original in operations, "retained proof lacks original sealed mutation/receipt")
                bind_retained_proof(request, value, operations[original].request, latest[original])
    return {key: {"request": operation.request, "result": latest.get(key)} for key, operation in operations.items()}


def bind_component_pool_events(data, operations):
    pairs = []
    if data["id"].startswith("board-"):
        pairs += [(data["board_request"], data["confirmed_receipt"]), (data["proof_read_request"], data["proof_read_result"])]
    if data["id"].startswith("float-"):
        pairs.append((data["board_request"], data["board_result"]))
        snapshot = data["fresh_snapshot"]
        saved = operations.get(snapshot["operation_id"])
        require(saved is not None and saved["request"]["action"] == "snapshot"
                and encoded(saved["result"]) == encoded(snapshot), "float snapshot differs from sealed independent observation")
    for request, result in pairs:
        saved = operations.get(request["operation_id"])
        require(saved is not None and encoded(saved["request"]) == encoded(request)
                and encoded(saved["result"]) == encoded(result), "component pool request/receipt differs from sealed event history")


def verify(seal_path, seal_hash):
    root, records = read_archive(seal_path, seal_hash)
    pool_operations = verify_pool_events(records)
    terminal = records[-1][1]
    require(terminal["scope"] in {"full", "confirmed-funding"}, "unknown archive scope")
    diagnostic = terminal["scope"] == "confirmed-funding"
    label_name = "native-funding-verdict" if diagnostic else "ap5-component-verdict"
    reference_name = "native_funding_verdict" if diagnostic else "component_verdict"
    status_name = "diagnostic_status" if diagnostic else "component_status"
    if diagnostic:
        require(not pool_operations and terminal.get("funded_ark_lifecycle") is False
                and terminal.get("whole_lab_complete") is False and "component_verdict" not in terminal,
                "diagnostic archive contains Ark operations/acceptance")
        require(not any(label in {"ap5-component-verdict", "wallet-command-queued"} or label.startswith("wallet-command-") for label, _, _ in records),
                "diagnostic archive contains full lifecycle commands/verdict")
        for label, value, _ in records:
            if label == "command-intent":
                require(value["kind"]["type"] in {"bootstrap_fund", "send_user_payment", "stop_run"}, "diagnostic command exceeds native-funding scope")
                if value["kind"]["type"] == "bootstrap_fund":
                    require(value["kind"] == {"type": "bootstrap_fund", "driver_id": "txd-0", "node_id": "node-a"}, "diagnostic archive bootstrap route mismatch")
    candidates = [(v, r) for label, v, r in records if label == label_name]
    require(len(candidates) == 1, "AP5 component-verdict census")
    producer, row = candidates[0]
    require(terminal[reference_name] == {"file": row["file"], "sha256": row["sha256"]}, "terminal component reference differs")
    require(producer["schema"] == ("spc.lab.ap5.native-funding-verdict/v1" if diagnostic else "spc.lab.ap5.component-verdict/v2"), "scope-specific verdict schema")
    bindings = producer["source_bindings"]
    bundle = strict_json(artifact(bindings["bundle_artifact"]).read_bytes())
    require(bindings["bundle_sha256"] == bindings["bundle_artifact"]["sha256"], "bundle hash binding mismatch")
    check_bundle(bundle, full=terminal["scope"] == "full")
    probe_events = [v for label, v, _ in records if label == "probe-invocation"]
    require(len(probe_events) == 1 and encoded(probe_events[0]) == encoded(bundle["probe_invocation"]), "executed probe tuple differs from admitted bundle")
    launches = [v for label, v, _ in records if label == "launch"]
    require(len(launches) == 1, "runtime launch census")
    argv = launches[0]["argv"]
    programs = [argv[i+1] for i, value in enumerate(argv[:-1]) if value == "--traffic-probe"]
    prefixes = [argv[i+1] for i, value in enumerate(argv[:-1]) if value == "--traffic-probe-arg"]
    require(programs == [bundle["probe_invocation"]["program"]["path"]] and prefixes == bundle["probe_invocation"]["prefix_args"], "runtime launch probe argv differs from bundle")
    for key in ("binaries", "joined_source_manifest", "joined_validation_receipt", "vtxo_verifier", "pool_wallet_executor"):
        require(bindings.get(key) == bundle.get(key), "archive source differs from admitted bundle: " + key)
    for value in bindings["binaries"].values():
        artifact(value)
    artifact(bindings["joined_source_manifest"])
    artifact(bindings["joined_validation_receipt"])
    verifier = bindings["vtxo_verifier"]
    if verifier is not None:
        artifact(verifier)
    sealed_components = {(r["file"], r["sha256"]) for label, _, r in records if label == "raw-component"}
    referenced = {(c["evidence"]["file"], c["evidence"]["sha256"]) for c in producer["components"]}
    require(referenced == sealed_components and len(referenced) == len(producer["components"]), "raw component event census mismatch")
    components = []
    for component in producer["components"]:
        if diagnostic:
            require(component["id"] in NATIVE_COMPONENTS and not component.get("requires"), "non-native component in diagnostic archive")
            data = strict_json((root / component["evidence"]["file"]).read_bytes())
            require(set(data) == {"schema", "id", "maturity", "ark_info", "transactions", "vtxos", "payments"}
                    and not data["vtxos"], "diagnostic raw evidence includes non-native lifecycle payload")
        if component["status"] != "passed":
            components.append(component)
            continue
        result = replay_component(root, component, verifier)
        data = strict_json((root / component["evidence"]["file"]).read_bytes())
        bind_component_pool_events(data, pool_operations)
        require(component["requires"] == result["requires"], "component dependency census differs")
        components.append({"id": component["id"], "status": "passed", "evidence": component["evidence"], "requires": result["requires"], "replayed": result})
    cleanups = [v for label, v, _ in records if label == "cleanup-inventory"]
    require(len(cleanups) == 1 and cleanups[0] == producer["cleanup"], "cleanup reference/census mismatch")
    cleanup = cleanups[0]
    validate_cleanup(cleanup)
    inventory = cleanup["inventory"]
    censuses = [v for label, v, _ in records if label == "active-resource-census"]
    if cleanup["exhaustive"]:
        require(len(censuses) == 1 and censuses[0] == inventory["active_manifest"], "active cleanup census binding mismatch")
        ids = {(p["pid"], p["start_time_ticks"]) for p in inventory["processes"]}
        require(any(p["role"] == "controller" for p in inventory["processes"]), "controller omitted from cleanup")
        tools = [v for label, v, _ in records if label == "tool-exit"]
        require(len(tools) == sum(label == "tool-intent" for label, _, _ in records)
                and all((v["pid"], v["ticks"]) in ids for v in tools), "auxiliary tool cleanup census incomplete")
    result = native_funding_verdict(components, cleanup, bindings) if diagnostic else final_verdict(components, mandatory_components(USERS, POOLS), cleanup, bindings)
    if diagnostic:
        require(all(producer.get(k) is False for k in ("ark_lifecycle_ready", "funded_ark_lifecycle", "aggregate_acceptance", "whole_lab_complete")),
                "diagnostic producer claimed Ark/aggregate readiness")
    require(result[status_name] == producer[status_name] == terminal[status_name], "producer/replayed scoped verdict contradiction")
    require(all(result[k] == producer[k] for k in ("missing", "failed", "unknown")), "producer/replayed component census contradiction")
    result["external_event_seal_sha256"] = seal_hash
    result["producer_" + status_name] = producer[status_name]
    result["whole_lab_complete"] = False
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--seal", type=Path, required=True)
    parser.add_argument("--seal-sha256", required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    result = verify(args.seal, args.seal_sha256)
    with args.out.open("xb") as out:
        out.write(encoded(result) + b"\n")
