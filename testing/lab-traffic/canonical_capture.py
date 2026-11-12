"""Per-node timed canonical fences and portable raw evidence for T3/AP5.

This component has no wallet, funding, mining or Ark operation. A consistent
fence attests the captured RPC responses, not continuous canonicality between
samples. Historical bodies remain distinguished from fresh anchor responses.
"""
from dataclasses import asdict
import hashlib
import json
import os
from pathlib import Path
import time

from oracle import Chain, Inconclusive, integer, require, sha256d, strict_json, unhex
from rpc_capture import sync_dir


class CanonicalCapture:
    def __init__(self, policy, recorder, capture, *, max_blocks=4096, deadline=None):
        self.policy, self.recorder, self.capture = policy, recorder, capture
        self.max_blocks = integer(max_blocks, 1, 100000)
        self.deadline = deadline
        self.raws, self.views, self.body_observations, self.cohorts = {}, {}, {}, []

    def check_deadline(self):
        if self.deadline is not None and time.monotonic() >= self.deadline:
            raise Inconclusive("canonical capture deadline")

    def _ancestry(self, node, tip):
        blocks, current, seen = [], tip, set()
        while True:
            self.check_deadline()
            require(current not in seen, "cycle in raw header ancestry")
            seen.add(current)
            key = node.node_id, current
            if key not in self.body_observations:
                encoded, observation = node.recorded("getblock", [current, 0])
                raw = unhex(encoded)
                require(len(raw) >= 81 and sha256d(raw[:80])[::-1].hex() == current, "raw RPC block header/hash binding")
                if current in self.raws:
                    require(self.raws[current] == raw, "nodes disagree on complete raw block/witness bytes")
                else:
                    self.raws[current] = raw
                record = {"node_id": node.node_id, "block_hash": current,
                          "raw_block": self.recorder.blob(raw), "rpc": self.recorder.reference(observation)}
                self.body_observations[key] = record
                self.capture.add("canonical_block_body", record)
            raw = self.raws[current]
            blocks.append(raw)
            if current == self.policy.genesis:
                break
            if len(blocks) >= self.max_blocks:
                raise Inconclusive("canonical ancestry exceeds declared capture bound")
            current = raw[4:36][::-1].hex()
        view = self.views.get(tip)
        if view is None:
            view = Chain(self.policy, blocks[::-1])
            for old in list(self.views):
                if old in view.by_hash:
                    self.views[old] = view.prefix(view.by_hash[old]["height"])
            self.views[tip] = view
        return view

    def sample(self, nodes, phase="observation"):
        self.check_deadline()
        nodes = list(nodes)
        require(len(nodes) >= 1 and len({n.node_id for n in nodes}) == len(nodes), "canonical node census")
        starts, views, assertions = {}, {}, {}
        # Both begin reads precede either end read, so the recorded per-node
        # intervals bracket a common observation interval rather than two
        # sequential, unrelated snapshots.
        for node in nodes:
            tip, ref = node.recorded("getbestblockhash", [])
            unhex(tip, 32)
            starts[node.node_id] = (tip, ref)
        for node in nodes:
            tip, _ = starts[node.node_id]
            view = self._ancestry(node, tip)
            views[node.node_id] = view
            mapped, map_ref = node.recorded("getblockhash", [view.tip["height"]])
            header, header_ref = node.recorded("getblockheader", [tip])
            require(isinstance(header, dict), "canonical verbose header response type")
            integer(header["height"])
            require(type(header["confirmations"]) is int, "canonical confirmations integer type")
            assertions[node.node_id] = (mapped, map_ref, header, header_ref)
        observations, stable = [], True
        for node in nodes:
            end_tip, end_ref = node.recorded("getbestblockhash", [])
            tip, begin_ref = starts[node.node_id]
            view = views[node.node_id]
            mapped, map_ref, header, header_ref = assertions[node.node_id]
            node_stable = (end_tip == tip == mapped == header.get("hash")
                           and header["height"] == view.tip["height"] and header["confirmations"] > 0)
            stable = stable and node_stable
            observations.append({"node_id": node.node_id, "tip_hash": tip, "height": view.tip["height"],
                                 "consistent_fence": node_stable,
                                 "begin_tip": self.recorder.reference(begin_ref), "height_hash": self.recorder.reference(map_ref),
                                 "header": self.recorder.reference(header_ref), "end_tip": self.recorder.reference(end_ref)})
        common = stable and len({o["tip_hash"] for o in observations}) == 1
        cohort = {"schema": "spc-lab-canonical-fence/1", "cohort": len(self.cohorts) + 1,
                  "phase_annotation": phase, "authority": "observation_only", "boot_id": self.recorder.boot_id,
                  "status": "consistent_common_fence" if common else "inconclusive_race_or_disagreement",
                  "nodes": observations, "body_observations_can_predate_fence": True,
                  "continuous_canonicality_claimed": False}
        self.cohorts.append(cohort)
        self.capture.add("canonical_fence", cohort)
        return views[nodes[0].node_id] if common else None

    def export(self, path, *, run_id, binary_hashes, scenario_sha256, source_snapshot_sha256):
        """Portable read-only AP5 input; never a funding/send authorization."""
        unresolved = [oid for oid, entry in self.recorder.exchanges.items() if "end" not in entry]
        data = {"schema": "spc-lab-chain-evidence/1", "run_id": run_id,
                "authority": "observation_only_no_hash_send_or_credit_authority", "policy": asdict(self.policy),
                "binary_hashes": binary_hashes, "scenario_sha256": scenario_sha256,
                "source_snapshot_sha256": source_snapshot_sha256, "boot_id": self.recorder.boot_id,
                "private_blob_root": str(self.recorder.root), "cohorts": self.cohorts,
                "block_bodies": list(self.body_observations.values()), "unresolved_rpc_observations": unresolved,
                "full_T3_acceptance": False, "Ark_lifecycle_acceptance": False}
        raw = json.dumps(data, sort_keys=True, separators=(",", ":")).encode() + b"\n"
        fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        with os.fdopen(fd, "wb") as stream:
            stream.write(raw); stream.flush(); os.fsync(stream.fileno())
        sync_dir(Path(path).parent)
        return {"path": str(path), "sha256": hashlib.sha256(raw).hexdigest(), "bytes": len(raw),
                "authority": data["authority"], "cohorts": len(self.cohorts)}


def _read_blob(root, ref):
    if ref is None:
        raise Inconclusive("RPC evidence body absent")
    relative = Path(ref["path"])
    require(not relative.is_absolute() and ".." not in relative.parts and len(relative.parts) == 2
            and relative.parts[0] == "rpc-bodies", "RPC blob path confinement")
    path = Path(root) / relative
    require(not path.is_symlink() and not path.parent.is_symlink(), "RPC blob symlink")
    from rpc_capture import MAX_RESPONSE
    integer(ref["bytes"], 0, MAX_RESPONSE)
    try:
        with path.open("rb") as stream:
            raw = stream.read(MAX_RESPONSE + 1)
    except OSError as error:
        raise Inconclusive("RPC evidence body unavailable") from error
    require(len(raw) == ref["bytes"] and hashlib.sha256(raw).hexdigest() == ref["sha256"], "RPC evidence blob hash/length")
    return raw


def validate_export(path, expected_run_id, expected_binary_hashes, *, expected_export_sha256,
                    expected_source_snapshot_sha256, expected_scenario_sha256, expected_policy, expected_node_ids):
    """Read-only consumer for AP5 and offline replay; missing data is not zero."""
    from oracle import Policy
    raw_export = Path(path).read_bytes()
    require(hashlib.sha256(raw_export).hexdigest() == expected_export_sha256, "chain export differs from external seal")
    data = strict_json(raw_export)
    require(data["schema"] == "spc-lab-chain-evidence/1" and data["run_id"] == expected_run_id
            and data["binary_hashes"] == expected_binary_hashes, "chain evidence run/binary binding")
    require(data["authority"] == "observation_only_no_hash_send_or_credit_authority", "chain evidence is not an authority grant")
    require(data["source_snapshot_sha256"] == expected_source_snapshot_sha256
            and data["scenario_sha256"] == expected_scenario_sha256 and data["policy"] == asdict(expected_policy),
            "chain source/scenario/consensus binding")
    expected_nodes = set(expected_node_ids)
    require(expected_nodes and len(expected_nodes) == len(expected_node_ids), "expected node census")
    if data["unresolved_rpc_observations"]:
        raise Inconclusive("export contains unresolved RPC observations")
    root, policy = Path(data["private_blob_root"]), Policy(**data["policy"])
    def exchange(ref, node_id, method, params):
        require(ref["node"] == node_id and ref["method"] == method and ref["failure"] is None
                and ref["boot_id"] == data["boot_id"], "RPC reference node/method/boot/failure binding")
        request = strict_json(_read_blob(root, ref["request_body"]))
        response = strict_json(_read_blob(root, ref["response_body"]))
        require(request["method"] == method and request["params"] == params and type(request["id"]) is int
                and type(response["id"]) is int and response["id"] == request["id"]
                and response.get("error") is None, "exact raw RPC request/response binding")
        sent, received = integer(ref["sent"]["monotonic_ns"]), integer(ref["received"]["monotonic_ns"])
        require(sent <= received, "RPC time interval reversed")
        return response["result"], sent, received
    bodies = {}
    for item in data["block_bodies"]:
        key = item["node_id"], item["block_hash"]
        require(key not in bodies, "duplicate per-node block observation")
        require(item["node_id"] in expected_nodes, "unexpected node body")
        raw = _read_blob(root, item["raw_block"])
        result, _, _ = exchange(item["rpc"], item["node_id"], "getblock", [item["block_hash"], 0])
        require(result == raw.hex(), "node raw-response/block mismatch")
        require(sha256d(raw[:80])[::-1].hex() == item["block_hash"], "export raw block hash")
        bodies[key] = raw
    verified = []
    for cohort in data["cohorts"]:
        if cohort["status"] != "consistent_common_fence":
            continue
        tips, intervals = set(), []
        census = [node["node_id"] for node in cohort["nodes"]]
        require(len(census) == len(set(census)) and set(census) == expected_nodes, "fence node census differs from expected")
        for node in cohort["nodes"]:
            require(node["consistent_fence"], "false consistent fence")
            replies, times = {}, {}
            for field, method, params in (("begin_tip", "getbestblockhash", []), ("end_tip", "getbestblockhash", []),
                                           ("height_hash", "getblockhash", [node["height"]]), ("header", "getblockheader", [node["tip_hash"]])):
                ref = node[field]
                result, sent, received = exchange(ref, node["node_id"], method, params)
                replies[field], times[field] = result, (sent, received)
            require(replies["begin_tip"] == replies["end_tip"] == replies["height_hash"] == node["tip_hash"], "fence tip changed")
            require(replies["header"]["hash"] == node["tip_hash"] and replies["header"]["height"] == node["height"]
                    and replies["header"]["confirmations"] > 0, "fence header membership")
            begin, end = times["begin_tip"][1], times["end_tip"][0]
            require(begin <= end and all(begin <= times[k][0] <= times[k][1] <= end for k in ("height_hash", "header")),
                    "membership responses not bracketed by tip observations")
            intervals.append((begin, end))
            chain, current = [], node["tip_hash"]
            while True:
                if (node["node_id"], current) not in bodies:
                    raise Inconclusive("per-node raw canonical ancestry missing")
                raw = bodies[node["node_id"], current]
                chain.append(raw)
                if current == policy.genesis:
                    break
                require(len(chain) <= 100000, "export ancestry bound")
                current = raw[4:36][::-1].hex()
            view = Chain(policy, chain[::-1])
            require(view.tip["height"] == node["height"], "fence raw height mismatch")
            tips.add(view.tip["hash"])
        require(len(tips) == 1 and max(a for a, _ in intervals) <= min(b for _, b in intervals), "non-overlapping/common-node fence")
        verified.append({"cohort": cohort["cohort"], "tip_hash": next(iter(tips)), "height": cohort["nodes"][0]["height"],
                         "nodes": [n["node_id"] for n in cohort["nodes"]]})
    if not verified:
        raise Inconclusive("no complete common canonical fence in export")
    return {"scope": "raw_chain_fences_only", "verified_fences": verified,
            "funding_or_send_authority": False, "full_T3_acceptance": False, "Ark_lifecycle_acceptance": False}
