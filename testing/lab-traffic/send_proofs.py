"""Verified cross-stream outcome evidence, not a deserializable authority."""
from types import MappingProxyType
import hashlib
import json

from oracle import Inconclusive, require


class SendOutcomeProofs:
    def __init__(self, run_id, rejections, reconciled, acceptances):
        self.run_id = run_id
        self.rejections = MappingProxyType({k: MappingProxyType(dict(v)) for k, v in rejections.items()})
        self.reconciled = tuple(MappingProxyType(dict(v)) for v in reconciled)
        self.acceptances = MappingProxyType({k: MappingProxyType(dict(v)) for k, v in acceptances.items()})

    def acceptance(self, header, record, history):
        require(header["run_id"] == self.run_id, "outcome proof belongs to another run")
        key = header["driver_id"], record["seq"]
        if key not in self.acceptances:
            raise Inconclusive("journal acceptance lacks verified actual RPC outcome")
        proof = self.acceptances[key]
        prefix = json.dumps(history[:record["seq"] + 1], sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()
        require(hashlib.sha256(prefix).hexdigest() == proof["history_prefix_sha256"], "acceptance proof/history prefix changed")
        require(proof["command_id"] == record["command_id"] and proof["txid"] == record["txid"]
                and proof["node_id"] == record["node_id"], "acceptance proof/history mismatch")
        return proof

    def rejection(self, header, record, history):
        require(header["run_id"] == self.run_id, "outcome proof belongs to another run")
        key = header["driver_id"], record["seq"]
        if key not in self.rejections:
            raise Inconclusive("journal rejection lacks verified actual RPC outcome")
        proof = self.rejections[key]
        prefix = json.dumps(history[:record["seq"] + 1], sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()
        require(hashlib.sha256(prefix).hexdigest() == proof["history_prefix_sha256"], "outcome proof/history prefix changed")
        require(proof["command_id"] == record["command_id"] and proof["txid"] == record["txid"]
                and proof["code"] == record["code"], "rejection proof/history mismatch")
        return proof
