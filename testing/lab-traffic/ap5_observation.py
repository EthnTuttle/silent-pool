"""Read the actual AP5 Evidence/result-v1 archive; never execute Ark operations.

The parent descriptor supplies an external event-stream seal. The current AP5
producer ends at lifecycle-observed-awaiting-independent-oracles, not a final
independent acceptance verdict. That precise gap must remain visible.
"""
from pathlib import Path
import hashlib
import re

from oracle import Inconclusive, integer, require, strict_json
from campaign import encode

USERS = ("alice", "bob", "carol", "dave")


def read(path, maximum=32 * 1024 * 1024):
    if path.is_symlink():
        raise Inconclusive("AP5 evidence symlink unsupported")
    with path.open("rb") as stream:
        data = stream.read(maximum + 1)
    if len(data) > maximum:
        raise Inconclusive("AP5 evidence bound exceeded")
    return data


def inspect(descriptor_path, common):
    descriptor = strict_json(read(descriptor_path))
    require(set(descriptor) == {"root", "events_sha256", "last_seq", "last_digest"}, "AP5 collector descriptor shape")
    root = Path(descriptor["root"])
    require(root.is_absolute() and root.resolve() == root, "AP5 archive root binding")
    raw = read(root / "events.jsonl")
    require(hashlib.sha256(raw).hexdigest() == descriptor["events_sha256"], "AP5 event stream differs from parent seal")
    if not raw.endswith(b"\n"):
        raise Inconclusive("AP5 event tail truncated")
    previous, labels = "0" * 64, {}
    rows = raw.splitlines()
    integer(len(rows), 1, 65536)
    for seq, line in enumerate(rows, 1):
        event = strict_json(line)
        require(set(event) == {"seq", "file", "sha256", "previous", "unix_ms", "record_sha256"}
                and event["seq"] == seq and event["previous"] == previous, "AP5 event prefix/link mismatch")
        core = {k: v for k, v in event.items() if k != "record_sha256"}
        # AP5 encoded() does not append the on-disk newline to hash preimages.
        previous = hashlib.sha256(encode(core).rstrip(b"\n")).hexdigest()
        require(previous == event["record_sha256"], "AP5 source event hash mismatch")
        match = re.fullmatch(r"([0-9]{6})-([a-z0-9-]+)\.json", event["file"])
        require(match is not None and int(match[1]) == seq, "AP5 artifact name/sequence mismatch")
        data = read(root / event["file"])
        require(hashlib.sha256(data).hexdigest() == event["sha256"], "AP5 artifact hash mismatch")
        labels.setdefault(match[2], []).append(strict_json(data))
    require(len(rows) == descriptor["last_seq"] and previous == descriptor["last_digest"], "AP5 external terminal head mismatch")
    if len(labels.get("result", [])) != 1 or not rows[-1].endswith(b"}"):
        raise Inconclusive("AP5 terminal result missing/ambiguous")
    require(strict_json(rows[-1])["file"].endswith("-result.json"), "AP5 result is not terminal")
    result = labels["result"][0]
    if result.get("schema") != "spc.lab.ap5.result/v1":
        raise Inconclusive("new AP5 result schema requires its sealed producer contract")
    if result.get("status") == "cleanup-failed":
        return {"status": "FAILED", "lifecycle_observed": False, "reason": "AP5 cleanup failed"}
    if (result.get("status") != "funded-lifecycle-observed-awaiting-independent-oracles"
            or result.get("scope") != "full" or result.get("funded_ark_lifecycle") is not True
            or result.get("native_funding_confirmed") is not True):
        return {"status": "INCONCLUSIVE", "lifecycle_observed": False, "reason": "AP5 full lifecycle not completed"}
    mandatory = {"verified-bundle", "declared-default", "native-funding-confirmed", "boarded-float-pool-a",
                 "boarded-float-pool-b", "cleanup"}
    mandatory |= {prefix + user for user in USERS for prefix in ("sp-owned-received-", "mining-payout-attribution-",
        "sp-round-confirmed-", "round-chain-proof-", "automatic-second-renewal-", "second-round-chain-",
        "stock-offboard-confirmed-", "sp-offboard-confirmed-", "sp-offboard-chain-proof-", "wallet-final-")}
    missing = sorted(mandatory - set(labels))
    if missing:
        return {"status": "INCONCLUSIVE", "lifecycle_observed": False, "missing_components": missing}
    require(len(labels["verified-bundle"]) == len(labels["cleanup"]) == 1, "AP5 bundle/cleanup census")
    bundle_record = labels["verified-bundle"][0]
    bundle = bundle_record["bundle"]
    require(bundle["schema"] == "spc.lab.ap5.bundle/v1", "AP5 actual bundle schema")
    for name in ("lab", "node", "pool", "miner", "scanner", "txd"):
        require(bundle["binaries"][name]["sha256"] == common["binary_hashes"][name], "AP5 and T3 use different joined binaries")
        require(bundle_record["source_manifest_sha256"] == common["binary_source_manifests"][name],
                "AP5 and T3 use different source manifests")
    cleanup = labels["cleanup"][0]
    require(cleanup.get("all_owned_actors_stopped") is True and cleanup.get("survivors") == [], "AP5 cleanup contradicts completion")
    if bundle_record["source_manifest_sha256"] == "415d21fa88ad2124712cc1fe9a8400bc7fbdd951bd005f223f92f879068f2602":
        raise Inconclusive("reviewed aggregate predecessor is not funded-aggregate acceptance")
    return {"status": "INCONCLUSIVE", "lifecycle_observed": True, "source_result": result["status"],
            "events_sha256": descriptor["events_sha256"], "events": len(rows),
            "missing_components": ["AP5 independently verified raw proof/full-policy/server and lifecycle verdict contract",
                                   "AP5 exhaustive resource cleanup receipt (result-v1 records process census only)"]}
