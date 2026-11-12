"""Offline actual producer framing/joins; fixtures are not native acceptance."""
import copy
import hashlib
import json
import unittest

from oracle import Inconclusive, Invalid, merkle
from pool_source import PoolSource, leaf_hash, trace_digest
import test_payout


def encode(events):
    previous = "00" * 32
    lines = []
    for seq, (kind, payload) in enumerate(events, 1):
        source = {"schema": "spc.source-evidence/2", "session": "11" * 16, "seq": str(seq),
                  "previous": previous, "complete_prefix_required": True}
        source["digest"] = trace_digest(kind, payload, source)
        previous = source["digest"]
        lines.append('INFO spc_evidence: event="' + kind + '" evidence=' + json.dumps({**payload, "source": source}))
    return ("\n".join(lines) + "\n").encode()


def parse(raw):
    return PoolSource(raw, independently_captured_log_sha256=hashlib.sha256(raw).hexdigest())


class PoolSourceTests(unittest.TestCase):
    def fixture(self):
        bundle, _ = test_payout.DistributionProofTests().fixture()
        j = bundle["jobs"]["one"]
        p = {"front_id": 0, "job_id": "00000001", "generation": "1", "height": 1, "version": 1,
             "ntime": 1600000001, "prev_hash": j["prev_hash_le"], "nbits": j["nbits"],
             "coinbase_prefix": j["coinbase_prefix"], "coinbase_suffix": "", "merkle_branch": [],
             "share_target": j["share_target_be"], "share_target_byte_order": "big-endian unsigned 256-bit",
             "m_issue": 1, "work_unit_version": 2, "work_unit_fingerprint": "22" * 32, "transactions": [],
             "window_target": "2", "window_rule": 1, "window_underfilled": False,
             "prior_pending": [], "pending_history_required": True}
        events = [("distribution_frozen", p), ("job_issued", {**p, "channel_id": 1})]
        leaves = []
        for index, share in enumerate(bundle["slices_newest_first"][0]["shares"]):
            leaf = {"nonce": share["nonce"], "ntime": share["ntime"], "version": share["version"],
                    "extranonce": [], "job_id": (1 << 32) | 1, "share_index": index}
            leaves.append(leaf)
            events.append(("accounted", {"front_id": 0, "job_id": p["job_id"], "generation": "1", "slice_id": "1",
                "channel_id": 1, "nonce": leaf["nonce"], "ntime": leaf["ntime"], "version": leaf["version"],
                "extranonce": "", "share_index": index, "sp_id": "33" * 32, "d": 1, "m_issue": 1,
                "share_hash": share["share_hash_le"], "credited_job_fees": "0", "coinbase": p["coinbase_prefix"]}))
        root = merkle([leaf_hash(leaf) for leaf in leaves])
        selected = {**p, "generation": "2", "job_id": "00000002", "high_water": {"0": "1"},
                    "slices": [{"slice_id": "1", "front_id": 0, "height": 1, "root": root.hex(),
                        "difficulty": "2", "diff_fee_sum": "0", "credits": [{"sp_id": "33" * 32, "diff_sum": "2", "diff_fee_sum": "0"}]}]}
        events.append(("distribution_frozen", selected))
        blob = json.dumps(leaves).encode()
        def le(n, size):
            return n.to_bytes(size, "little")
        raw = (le(1, 8) + le(1, 4) + bytes.fromhex(p["prev_hash"]) + le((1 << 32) | 1, 8) + le(0, 8)
               + le(2, 4) + le(2, 16) + root + le(len(blob), 4) + blob + le(1, 2) + bytes.fromhex("33" * 32)
               + le(2, 16) + le(0, 16) + bytes([2, 32]) + bytes.fromhex("22" * 32))
        # RAM window freeze precedes its successful XADD acknowledgement.
        events.append(("source_publication", {"publication_complete": True, "stream": "spc:slices",
                       "stream_id": "100-0", "slice_id": "1", "raw_d_hex": raw.hex()}))
        return events

    def window(self, events):
        source = parse(encode(events))
        key = ("11" * 16, 0, "00000002", 2)
        return source.verified_window(key, reference_target=(1 << 255) - 1, window_n=1, prevout_transactions={})

    def test_actual_framing_raw_share_join_and_ram_publication(self):
        result = self.window(self.fixture())
        self.assertEqual((result["window_target"], result["window_work"]), (2, 2))
        self.assertEqual(result["mappings"][0]["redis_stream_id"], "100-0")
        self.assertGreater(result["mappings"][0]["publication_source_seq"], result["source_watermark"])

    def test_external_seal_gap_and_truncated_tail(self):
        raw = encode(self.fixture())
        with self.assertRaisesRegex(Invalid, "external collector seal"):
            PoolSource(raw, independently_captured_log_sha256="00" * 32)
        lines = raw.splitlines(keepends=True)
        for damaged in (b"".join(lines[1:]), b"".join(lines[:2] + lines[3:]), raw[:-1]):
            with self.assertRaises(Inconclusive):
                parse(damaged)
        damaged = raw.replace(b'"credited_job_fees": "0"', b'"credited_job_fees": "1"', 1)
        with self.assertRaisesRegex(Invalid, "hash mismatch"):
            parse(damaged)

    def test_immutable_issued_job_and_channel_join(self):
        events = self.fixture()
        events[1][1]["coinbase_suffix"] = "00"
        with self.assertRaisesRegex(Invalid, "differs from frozen"):
            self.window(events)
        events = self.fixture()
        events[1][1]["channel_id"] = 2
        with self.assertRaisesRegex(Invalid, "channel job issuance"):
            self.window(events)
        events = self.fixture()
        events.pop(1)
        with self.assertRaisesRegex(Inconclusive, "issued job join"):
            self.window(events)

    def test_missing_failed_or_changed_publication_cannot_establish_window(self):
        events = self.fixture()
        events[-1][1]["publication_complete"] = False
        with self.assertRaisesRegex(Inconclusive, "publication missing"):
            self.window(events)
        events = self.fixture()
        events[-1][1]["slice_id"] = "2"
        with self.assertRaisesRegex(Invalid, "id/bytes mismatch"):
            self.window(events)

    def test_high_water_census_and_postfreeze_attribution(self):
        events = self.fixture()
        events[-2][1]["high_water"]["1"] = str((1 << 56) + 1)
        with self.assertRaisesRegex(Invalid, "high-water vector"):
            self.window(events)
        events = self.fixture()
        events[3], events[4] = events[4], events[3]
        with self.assertRaisesRegex(Invalid, "post-freeze share"):
            self.window(events)

    def test_unknown_provisional_pending_cannot_be_promoted_to_entitlement(self):
        result = self.window(self.fixture())
        self.assertNotIn("full_owed_verified", result)
        self.assertNotIn("prior_pending", result)
        events = self.fixture()
        events.insert(0, ("proof_unavailable", {"reason": "bound"}))
        with self.assertRaisesRegex(Inconclusive, "unavailable proof"):
            self.window(events)


if __name__ == "__main__":
    unittest.main()
