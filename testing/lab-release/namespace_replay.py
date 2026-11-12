#!/usr/bin/env python3
"""Replay a genuine namespace fork/heal without starting or mining a node.

The proof is intentionally distinct from the aggregate scenario checks: every
fork branch must descend by raw-header parent links from a paused common block,
and the heal displacement must refer to that partition's losing chain and the
winning chain anchored on it. Missing evidence is unknown, never a pass.
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import re
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from release_lib import ValidationError, atomic_write, load_json, sha256_file

EVIDENCE_SCHEMA = "spc.lab.namespace_chain_evidence.v1"
NODES = ("node-a", "node-b")


def fail(code: str, message: str, unknown: bool = False):
    raise ValidationError(code, message, "unknown" if unknown else "failed")


def display_hash(raw: bytes) -> str:
    return hashlib.sha256(hashlib.sha256(raw).digest()).digest()[::-1].hex()


def milliseconds(value: str) -> float:
    return dt.datetime.fromisoformat(value.replace("Z", "+00:00")).timestamp() * 1000


def read_tip_records(path: Path, node: str) -> list[dict]:
    records = []
    for line in path.read_text(encoding="utf-8", errors="strict").splitlines():
        match = re.search(r"^(\S+) INFO tip: best=([0-9a-f]{64}) height=(\d+)\b", line)
        if match:
            observed, block_hash, height = match.groups()
            records.append({"node": node, "observed_at": observed,
                            "wall_ms": milliseconds(observed), "height": int(height),
                            "block_hash": block_hash, "source": "tip: best"})
    if not records:
        fail("namespace_chain_evidence_missing", f"no best-tip observations for {node}", True)
    return records


def read_submitted_headers(path: Path, destination: dict) -> None:
    for line in path.read_text(encoding="utf-8", errors="strict").splitlines():
        row = json.loads(line)
        if row.get("method") != "submitblock" or row.get("ok") is not True:
            continue
        match = re.match(r"\[\s*\"([0-9a-fA-F]+)", row.get("params", ""))
        if not match or len(match.group(1)) < 160:
            fail("namespace_chain_evidence_missing", "successful submitblock has no complete raw header", True)
        try:
            raw = bytes.fromhex(match.group(1)[:160])
        except ValueError as error:
            fail("namespace_chain_evidence_missing", f"invalid raw header: {error}", True)
        if len(raw) != 80:
            fail("namespace_chain_evidence_missing", "truncated raw header", True)
        block_hash = display_hash(raw)
        header = {"block_hash": block_hash, "parent_hash": raw[4:36][::-1].hex(),
                  "header_hex": raw.hex()}
        previous = destination.get(block_hash)
        if previous is not None and previous != header:
            fail("namespace_chain_conflict", f"one hash has different raw headers: {block_hash}")
        destination[block_hash] = header


def read_events(path: Path) -> tuple[list[dict], list[dict]]:
    links = []
    displacements = []
    for line in path.read_text(encoding="utf-8", errors="strict").splitlines():
        row = json.loads(line)
        payload = row.get("payload", {})
        data = payload.get("data", {})
        if payload.get("kind") == "link_policy":
            links.append({"event_id": row["event_id"], "wall_ms": row["occurred_at_wall"],
                          "link_id": data.get("link_id"), "policy": data.get("policy")})
        elif payload.get("kind") == "canonical_at" and data.get("result") == "no":
            displacements.append({
                "event_id": row["event_id"], "wall_ms": data.get("observed_at", row["occurred_at_wall"]),
                "node": data.get("node_id"), "height": data.get("height"),
                "block_hash": data.get("block_hash"), "anchored_tip": data.get("anchored_tip"),
            })
    return links, displacements


def collect_namespace_evidence(run_dir: Path) -> dict:
    paths = {
        "node_a_log": run_dir / "logs" / "node-node-a.log",
        "node_b_log": run_dir / "logs" / "node-node-b.log",
        "node_a_api": run_dir / "private" / "node-api-node-a.jsonl",
        "node_b_api": run_dir / "private" / "node-api-node-b.jsonl",
        "evidence": run_dir / "private" / "evidence.jsonl",
        "result": run_dir / "shareable" / "result.json",
    }
    input_hashes = {}
    for name, path in paths.items():
        if not path.is_file() or path.is_symlink():
            fail("namespace_chain_evidence_missing", f"missing regular evidence file: {path}", True)
        input_hashes[name] = {"path": str(path), "sha256": sha256_file(path)}
    result = load_json(paths["result"])
    phases = []
    for phase in result.get("topology", {}).get("phases", []):
        phases.append({
            "phase": phase.get("phase"),
            "outcome": {"detail": phase.get("outcome", {}).get("detail")},
            "nodes": [{"node": row.get("node"), "atomic": row.get("atomic"),
                       "height": row.get("height"), "hash": row.get("hash")}
                      for row in phase.get("nodes", [])],
        })
    links, displacements = read_events(paths["evidence"])
    headers = {}
    for name in ["a", "b"]:
        read_submitted_headers(paths[f"node_{name}_api"], headers)
    return {
        "schema": EVIDENCE_SCHEMA,
        "input_hashes": input_hashes,
        "tips": {f"node-{name}": read_tip_records(paths[f"node_{name}_log"], f"node-{name}") for name in ["a", "b"]},
        "headers": headers, "phases": phases, "links": links, "displacements": displacements,
    }


def atomic_view(row: dict) -> dict:
    if row.get("atomic") is not True or type(row.get("height")) is not int or not re.fullmatch(r"[0-9a-f]{64}", row.get("hash", "")):
        fail("namespace_chain_evidence_missing", "malformed/non-atomic phase view", True)
    return {"node": row.get("node"), "height": row["height"], "hash": row["hash"]}


def two_node_views(phases: list[dict], phase_name: str, before: int) -> dict:
    choices = [row for row in phases[:before] if row["phase"] == phase_name and
               row["outcome"].get("detail") == "workers paused"]
    if not choices:
        fail("namespace_chain_evidence_missing", f"missing paused phase {phase_name}", True)
    phase = choices[-1]
    views = [atomic_view(row) for row in phase["nodes"]]
    if len(views) != 2 or {row["node"] for row in views} != set(NODES):
        fail("namespace_chain_evidence_missing", f"{phase_name} does not contain exactly two node views", True)
    return {row["node"]: row for row in views}


def observed(records: list[dict], height: int, wall_from: float | None = None,
             wall_to: float | None = None) -> list[str]:
    return [row["block_hash"] for row in records if row["height"] == height and
            (wall_from is None or row["wall_ms"] > wall_from) and
            (wall_to is None or row["wall_ms"] < wall_to)]


def own_partition_chain(evidence: dict, node: str, common: dict, preheal: dict,
                        partition_ms: float, heal_ms: float) -> dict[int, str]:
    tips = evidence["tips"][node]
    height = preheal["height"]
    current_hash = preheal["hash"]
    outside = observed(tips, height)
    inside = observed(tips, height, partition_ms, heal_ms)
    if current_hash not in inside:
        observed_elsewhere = any(row["block_hash"] == current_hash
                                 for records in evidence["tips"].values() for row in records)
        if current_hash in outside or inside or observed_elsewhere:
            fail("namespace_no_real_fork",
                 f"{node} pre-heal tip is not in its own partition-window chain; a lag/cross-node substitution is not a fork")
        fail("namespace_chain_evidence_missing",
             f"no observation joins the {node} pre-heal tip to the partition window", True)
    chain = {common["height"]: common["hash"]}
    for level in range(height, common["height"], -1):
        inside = observed(tips, level, partition_ms, heal_ms)
        if current_hash not in inside:
            if inside:
                fail("namespace_cross_node_chain",
                     f"{node} raw parent at height {level} is not the node's announced block")
            fail("namespace_chain_hole",
                 f"{node} has no partition-window best-tip observation at height {level}", True)
        header = evidence["headers"].get(current_hash)
        if header is None:
            fail("namespace_chain_hole", f"raw header missing for {node} block {current_hash} at height {level}", True)
        chain[level] = current_hash
        current_hash = header["parent_hash"]
        if level == common["height"] + 1 and current_hash != common["hash"]:
            fail("namespace_chain_link_mismatch",
                 f"{node} first partition block does not have the paused common block as parent")
    if current_hash != common["hash"]:
        fail("namespace_chain_link_mismatch", f"{node} branch does not descend to the paused common block")
    return chain


def anchor_extension(evidence: dict, winner: str, winner_tip: int,
                     winner_hash: str, anchor_hash: str, heal_ms: float) -> dict | None:
    """Prove a displacement anchor is the winning tip or its raw descendant."""
    candidates = []
    for records in evidence["tips"].values():
        candidates.extend(row for row in records if row["block_hash"] == anchor_hash and
                          row["height"] >= winner_tip and row["wall_ms"] > heal_ms)
    for anchor in candidates:
        height = anchor["height"]
        current = anchor_hash
        valid = True
        while height > winner_tip:
            header = evidence["headers"].get(current)
            if header is None:
                valid = False
                break
            current = header["parent_hash"]
            height -= 1
        if valid and height == winner_tip and current == winner_hash:
            return {"anchor_height": anchor["height"], "anchor_hash": anchor_hash,
                    "descends_from_winner_preheal_tip": True,
                    "observed_by_node": anchor["node"], "observed_at": anchor["observed_at"]}
    return None


def prove_namespace_chain(evidence: dict) -> dict:
    if evidence.get("schema") != EVIDENCE_SCHEMA:
        fail("namespace_chain_evidence_missing", "unsupported chain evidence schema", True)
    partitions = [row for row in evidence["links"] if row["link_id"] == "link-p2p-nodes" and row["policy"] == "partition"]
    heals = [row for row in evidence["links"] if row["link_id"] == "link-p2p-nodes" and row["policy"] == "normal"]
    if len(partitions) != 1 or len(heals) != 1:
        fail("namespace_chain_evidence_missing", "expected exactly one partition and heal link event", True)
    partition, heal = partitions[0], heals[0]
    if partition["event_id"] >= heal["event_id"]:
        fail("namespace_phase_order", "partition/heal link events are out of order")
    phase_names = [row["phase"] for row in evidence["phases"]]
    if "partition-p2p" not in phase_names or "heal-p2p" not in phase_names:
        fail("namespace_chain_evidence_missing", "partition/heal phase anchors missing", True)
    partition_phase = phase_names.index("partition-p2p")
    heal_phase = phase_names.index("heal-p2p")
    if partition_phase >= heal_phase:
        fail("namespace_phase_order", "partition/heal phase views are out of order")
    common_views = two_node_views(evidence["phases"], "quiesce-before-partition", partition_phase)
    preheal_views = two_node_views(evidence["phases"], "quiesce-before-heal", heal_phase)
    if common_views["node-a"]["height"] != common_views["node-b"]["height"] or common_views["node-a"]["hash"] != common_views["node-b"]["hash"]:
        fail("namespace_chain_ancestor_unknown", "nodes did not pause on one exact pre-partition block", True)
    common = dict(common_views["node-a"])
    common.pop("node", None)
    if common["hash"] not in evidence["headers"]:
        fail("namespace_chain_ancestor_unknown", "raw common-ancestor header is unavailable", True)
    for node in NODES:
        if common["hash"] not in observed(evidence["tips"][node], common["height"], None, partition["wall_ms"]):
            fail("namespace_chain_ancestor_unknown", f"{node} did not observe the common block before partition", True)
        if preheal_views[node]["height"] <= common["height"]:
            fail("namespace_no_real_fork", f"{node} produced no partition work above the common block")
    chains = {node: own_partition_chain(evidence, node, common, preheal_views[node],
                                       partition["wall_ms"], heal["wall_ms"]) for node in NODES}
    common_max = min(preheal_views[node]["height"] for node in NODES)
    if chains["node-a"][common_max] == chains["node-b"][common_max]:
        fail("namespace_no_real_fork",
             f"both nodes have the same block at common pre-heal height {common_max}; this is one chain, not a fork")

    heal_candidates = []
    for loser, winner in [("node-a", "node-b"), ("node-b", "node-a")]:
        winner_tip = preheal_views[winner]["height"]
        reannounced = []
        complete = True
        for level in range(common["height"] + 1, winner_tip + 1):
            expected = chains[winner][level]
            matching = [row for row in evidence["tips"][loser]
                        if row["height"] == level and row["block_hash"] == expected and row["wall_ms"] > heal["wall_ms"]]
            if not matching:
                complete = False
                break
            reannounced.append({"height": level, "block_hash": expected, "observed_at": matching[0]["observed_at"]})
        if not complete:
            continue
        winning_hashes = set(chains[winner].values())
        unresolved_anchor = False
        for displacement in evidence["displacements"]:
            height = displacement.get("height")
            if not (displacement.get("node") == loser and displacement["wall_ms"] > heal["wall_ms"] and
                    type(height) is int and common["height"] < height <= common_max and
                    chains[loser].get(height) == displacement.get("block_hash")):
                continue
            anchor = None
            if displacement.get("anchored_tip") in winning_hashes:
                anchor = {"anchor_height": preheal_views[winner]["height"],
                          "anchor_hash": displacement["anchored_tip"],
                          "descends_from_winner_preheal_tip": False,
                          "equality_with_winner_preheal_tip": True}
            else:
                anchor = anchor_extension(evidence, winner, preheal_views[winner]["height"],
                                          preheal_views[winner]["hash"], displacement.get("anchored_tip"),
                                          heal["wall_ms"])
            if anchor is None:
                unresolved_anchor = True
                continue
            heal_candidates.append({"loser": loser, "winner": winner,
                                    "reannounced_chain": reannounced, "displacement": displacement,
                                    "anchor_proof": anchor})
        if unresolved_anchor and not heal_candidates:
            fail("namespace_chain_hole",
                 "a relevant displacement anchor cannot be joined to the winning chain by raw parent headers", True)
    if not heal_candidates:
        fail("namespace_reorg_evidence",
             "no post-heal, above-fork-point displacement joins the losing partition block to the surviving chain")
    if len(heal_candidates) != 1:
        fail("namespace_reorg_evidence", "heal proof is ambiguous or describes multiple incompatible chains")
    chosen = heal_candidates[0]
    return {
        "status": "passed", "common_anchor": common,
        "preheal_tips": preheal_views,
        "partition_chains": {
            node: [{"height": height, "block_hash": block_hash}
                   for height, block_hash in sorted(chains[node].items()) if height >= common["height"]]
            for node in NODES
        },
        "common_comparison_height": common_max,
        "fork_hashes_at_common_height": {node: chains[node][common_max] for node in NODES},
        "heal": chosen,
        "proof_scope": "paused common block + per-node observed best-tip/raw-header parent descent + post-heal displacement on the surviving chain",
        "unknown_semantics": "missing ancestry, chain observations, or raw headers raise qualified unknown; a same-chain lag is not a fork",
    }


def replay(run_dir: Path, output: Path) -> dict:
    evidence = collect_namespace_evidence(run_dir)
    proof = prove_namespace_chain(evidence)
    for row in evidence["input_hashes"].values():
        if sha256_file(Path(row["path"])) != row["sha256"]:
            fail("namespace_input_drift", f"input changed during replay: {row['path']}")
    result = {
        "schema": "spc.lab.namespace_chain_replay.v1", "derived_from_historical_artifacts": True,
        "no_new_live_run": True, "input_hashes": evidence["input_hashes"], "proof": proof,
        "generated_by_script_sha256": sha256_file(Path(__file__).resolve()),
    }
    if output.exists():
        fail("replay_output_exists", f"refusing to overwrite existing replay: {output}")
    output.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    os.chmod(output.parent, 0o700)
    atomic_write(output, result)
    return {"output": str(output), "sha256": sha256_file(output), "proof": proof}


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run-dir", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        print(json.dumps(replay(args.run_dir.resolve(), args.output.resolve()), sort_keys=True))
        return 0
    except ValidationError as error:
        print(json.dumps({"status": error.outcome, "error": error.as_dict()}, sort_keys=True), file=sys.stderr)
        return 2 if error.outcome == "unknown" else 1


if __name__ == "__main__":
    raise SystemExit(main())
