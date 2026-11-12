"""Working-contract decoding vectors only; no tested-producer or live claim."""
import copy
import hashlib
import json
import unittest

from ladder_wire import decode_spcb2, decode_spcb_working
from oracle import Inconclusive, Invalid
from ruleb_source import (API_SHA256, bootstrap_view, canonical_frozen, consumption, economic_changed,
                         frozen_job, require_live_binding)
from test_ladder_wire import vector


def job(height=105, prior=500000):
    return {"version": 1, "revision": 3, "parent_hash": [7] * 32, "job_id": (height << 32) | 9,
            "generation": 4, "balances": [{"sp_id": [0x33] * 32, "booked": prior, "reserved": 0, "available": prior}],
            "slice_ids": [123, 122]}


def frame(value, encoded=None):
    data = canonical_frozen(value) if encoded is None else encoded
    return b"SPCB\3" + vector()[5:] + len(data).to_bytes(4, "little") + data


def consumption_row(prior=500000, error=100000):
    return {"sp_id": [0x33] * 32, "prior": prior, "error_pending": error,
            "delta": error-prior, "consumed": max(0, prior-error)}


def reservation_row(block=1, lifecycle="unbooked", prior=500000, error=100000):
    return {"block_hash": [block] * 32, "parent_hash": [0] * 32, "height": 1, "payload_digest": [9] * 32,
            "job_id": (1 << 32) | 1, "generation": 4, "lifecycle": lifecycle, "transition": 1, "revision": 3,
            "rows": [consumption_row(prior, error)]}


def view():
    return {"version": 1, "revision": 3, "anchor_height": 1, "anchor_hash": [1] * 32, "complete": True,
            "booked": [[[0x33] * 32, 500000]], "reservations": [reservation_row()]}


class WorkingRuleBTests(unittest.TestCase):
    def test_SPCB3_retains_entry_v2_and_lossless_u64_snapshot_bytes(self):
        value = job()
        value["revision"] = 2**53 + 7
        value["generation"] = 2**64 - 1
        value["balances"][0].update(booked=2**53+9, available=2**53+9)
        raw = frame(value)
        decoded = decode_spcb_working(raw, api_sha256=API_SHA256)
        self.assertEqual((decoded["wire_version"], decoded["format_version"]), (3, 2))
        self.assertEqual(decoded["economic"], value)
        self.assertEqual(decoded["snapshot_key"], hashlib.sha256(b"SPc/DistributionSnapshotV2" + raw).hexdigest())
        self.assertEqual(decoded["entries"][0]["format_version"], 2)
        with self.assertRaises(Invalid):
            decode_spcb2(raw)
        with self.assertRaises(Invalid):
            decode_spcb_working(raw, api_sha256="00" * 32)

    def test_historical_SPCB2_is_missing_economics_not_zero_prior(self):
        value = decode_spcb_working(vector(), api_sha256=API_SHA256)
        self.assertIsNone(value["economic"])
        with self.assertRaisesRegex(Inconclusive, "not P=0"):
            frozen_job(value["economic"])

    def test_frozen_identity_context_and_order_are_bound(self):
        value = job()
        frozen_job(value, height=105, wire_job_id=9, generation=4, parent_hash="07"*32, slice_ids=[123,122])
        for kwargs in ({"height": 104}, {"wire_job_id": 10}, {"generation": 5}, {"parent_hash": "08"*32}, {"slice_ids": [122,123]}):
            with self.subTest(kwargs=kwargs), self.assertRaises(Invalid):
                frozen_job(value, **kwargs)
        value["balances"][0]["sp_id"] = [8]*32
        with self.assertRaisesRegex(Invalid, "coverage"):
            decode_spcb_working(frame(value), api_sha256=API_SHA256)

    def test_missing_unknown_versions_and_noninteger_values_never_default(self):
        for field in job():
            value = job(); del value[field]
            with self.subTest(field=field), self.assertRaises(Inconclusive):
                frozen_job(value)
        for version in (None, 2, True):
            value = job(); value["version"] = version
            with self.assertRaises(Inconclusive):
                frozen_job(value)
        for amount in (500000.0, "500000", True, 2**64):
            value = job(); value["balances"][0]["booked"] = amount
            with self.assertRaises(Invalid):
                frozen_job(value)

    def test_noncanonical_truncated_and_oversized_economic_frames(self):
        raw = frame(job())
        with self.assertRaises(Inconclusive):
            decode_spcb_working(raw[:-1], api_sha256=API_SHA256)
        with self.assertRaisesRegex(Invalid, "noncanonical"):
            decode_spcb_working(frame(job(), json.dumps(job()).encode()), api_sha256=API_SHA256)
        raw = b"SPCB\3" + vector()[5:] + (4*1024*1024+1).to_bytes(4,"little")
        with self.assertRaises(Inconclusive):
            decode_spcb_working(raw, api_sha256=API_SHA256)

    def test_consumption_uses_frozen_P_and_full_delta_not_claim_paid(self):
        self.assertEqual(consumption([consumption_row()], job())[0]["consumed"], 400000)
        contraction = consumption_row(error=0)
        self.assertEqual(consumption([contraction], job())[0]["consumed"], 500000)
        contraction["consumed"] = 300000
        with self.assertRaisesRegex(Invalid, "E-P"):
            consumption([contraction], job())
        with self.assertRaisesRegex(Invalid, "immutable frozen"):
            consumption([consumption_row(prior=100000)], job())

    def test_bootstrap_only_unbooked_contributes_to_R_and_full_active_set_is_required(self):
        value = view(); value["reservations"].append(reservation_row(2, "loss_held"))
        result = bootstrap_view(value)
        self.assertEqual(result["balances"]["33"*32], {"booked":500000,"reserved":400000,"available":100000})
        self.assertEqual(result["authority"], "unverified_checkpoint_claim")
        for field in ("reservations", "booked", "anchor_hash"):
            missing = copy.deepcopy(value); del missing[field]
            with self.assertRaises(Inconclusive):
                bootstrap_view(missing)
        value["reservations"] = [reservation_row()] * 4097
        with self.assertRaises(Inconclusive):
            bootstrap_view(value)

    def test_revision_incomplete_overflow_and_inactive_rows_are_refused(self):
        value = view(); value["reservations"][0]["revision"] = 4
        with self.assertRaisesRegex(Inconclusive, "revision"):
            bootstrap_view(value)
        value = view(); value["complete"] = False
        with self.assertRaises(Inconclusive):
            bootstrap_view(value)
        value = view(); value["reservations"][0]["lifecycle"] = "booked"
        with self.assertRaises(Invalid):
            bootstrap_view(value)
        value = view(); value["booked"][0][1] = 2**64-1
        value["reservations"] = [reservation_row(n, prior=2**63, error=0) for n in (1,2)]
        with self.assertRaises(Invalid):
            bootstrap_view(value)

    def test_EconomicChanged_is_only_a_reload_hint_and_working_pin_cannot_grant_live(self):
        raw = b"\5" + (8).to_bytes(8,"little") + (2**53+1).to_bytes(8,"little") + bytes([3])*32
        value = economic_changed(raw)
        self.assertEqual(value["economic_revision"], 2**53+1)
        self.assertFalse(value["checkpoint_verified"])
        self.assertEqual(value["authority"], "reload_hint_only")
        with self.assertRaises(Inconclusive):
            require_live_binding()

    def test_actual_source_envelope_joins_SPCB3_to_frozen_packed_job(self):
        from pool_source import PoolSource
        from test_source_jobs import fixture
        from test_pool_source import encode
        from source_jobs import verify_publication
        events, chain, policy = fixture()
        for index in (0, 2):
            payload = events[index][1]
            economic = job(payload["height"], 0)
            economic.update(parent_hash=list(bytes.fromhex(payload["prev_hash"])), balances=[], slice_ids=[],
                            job_id=(payload["height"] << 32) | int(payload["job_id"], 16), generation=int(payload["generation"]))
            payload["economic"] = economic
        publication = events[-1][1]
        raw = bytes.fromhex(publication["raw_d_hex"])
        data = canonical_frozen(events[2][1]["economic"])
        raw = b"SPCB\3" + raw[5:] + len(data).to_bytes(4,"little") + data
        publication["raw_d_hex"] = raw.hex()
        publication["sql_snapshot_key"] = hashlib.sha256(b"SPc/DistributionSnapshotV2" + raw).hexdigest()
        events.append(("provisional_candidate", {"tentative_only": True}))
        log = encode(events)
        source = PoolSource(log, independently_captured_log_sha256=hashlib.sha256(log).hexdigest(), working_ruleb_api_sha256=API_SHA256)
        result = verify_publication(source, chain.tip["hash"], {chain.tip["hash"]:chain}, policy)
        self.assertFalse(result["owed_verified"])
        events[2][1]["economic"]["job_id"] += 1
        log = encode(events)
        with self.assertRaisesRegex(Invalid, "wire job sequence"):
            PoolSource(log, independently_captured_log_sha256=hashlib.sha256(log).hexdigest(), working_ruleb_api_sha256=API_SHA256)


if __name__ == "__main__":
    unittest.main()
