"""Offline source-attribution/window/denomination negative proof cases."""
import copy
import unittest

from oracle import Chain, Inconclusive, Invalid, decode_transaction, sha256d
from payout import verify_distribution
from test_oracle import block_bytes, coinbase, policy


class DistributionProofTests(unittest.TestCase):
    def fixture(self):
        genesis = block_bytes(bytes(32), [coinbase(10000000, tag=b"\1\0")])
        keys = [bytes([n]) * 32 for n in range(1, 5)]
        pool_script = b"\x51\x20" + b"\x09" * 32
        outputs = [(500000, pool_script)] + [(value, b"\x51\x20" + key)
                  for key, value in zip(keys, (5000000, 3000000, 1000000, 500000))]
        cb = coinbase(outputs=outputs)
        raw = block_bytes(sha256d(genesis[:80]), [cb], 1600000001)
        p = policy(genesis=sha256d(genesis[:80])[::-1].hex(), regtest_pow_bits_overlay=None,
                   subsidy_initial_sats=10000000)
        chain = Chain(p, [genesis, raw])
        job = {"coinbase_prefix": cb.hex(), "coinbase_suffix": "", "merkle_path": [],
               "prev_hash_le": sha256d(genesis[:80]).hex(), "nbits": 0x207fffff,
               "share_target_be": "7f" + "ff" * 31, "m_issue": 1, "generation": 1,
               "job_id": "job-1", "transactions": []}
        shares = []
        for nonce in range(256):
            header = raw[:76] + nonce.to_bytes(4, "little")
            digest = sha256d(header)
            if int.from_bytes(digest, "little") < 1 << 255:
                shares.append({"identity": "alice", "version": 1, "ntime": 1600000001,
                               "nonce": nonce, "extranonce": "", "share_hash_le": digest.hex(),
                               "generation": 1, "job_id": "job-1", "credited_work": 1, "job_key": "one"})
                if len(shares) == 2:
                    break
        self.assertEqual(len(shares), 2)
        bundle = {"block_hash": chain.tip["hash"], "reference_target_be": "7f" + "ff" * 31,
                  "window_n": 1, "fee_bps": 100, "finding_slice_id": 2,
                  "pool_script_hex": pool_script.hex(), "jobs": {"one": job},
                  "snapshot": {"height": 1, "work_unit_version": 2, "window_rule": 1, "window_target": 2,
                               "high_water": {"0": 1}, "work_unit_fingerprint": "22" * 32},
                  "slices_newest_first": [{"front_id": 0, "slice_id": 1, "difficulty": 2,
                                            "n_shares": 2, "work_unit_fingerprint": "22" * 32, "shares": shares}],
                  "entries": {"alice": {"format_version": 2, "owed_sats": 9900000, "sats": 9500000,
                                          "p_x": [k.hex() for k in keys], "delivery": "registered_derived"}},
                  "ledger": {"alice": {"paid_sats": 9500000, "ark_due_sats": 400000,
                                         "forfeited_sats": 0, "pending_delta": 0}}}
        return bundle, chain

    def test_independent_ladder_paid_owed(self):
        bundle, chain = self.fixture()
        result = verify_distribution(bundle, chain, {"alice": 0})
        self.assertEqual(result["paid"], 9500000)
        self.assertEqual(result["window_work"], 2)

    def test_pool_claim_cannot_override_actual_coinbase(self):
        bundle, chain = self.fixture()
        bundle["ledger"]["alice"]["paid_sats"] += 1
        with self.assertRaises(Invalid):
            verify_distribution(bundle, chain, {"alice": 0})

    def test_no_default_zero_pending(self):
        bundle, chain = self.fixture()
        with self.assertRaises(Inconclusive):
            verify_distribution(bundle, chain, {})

    def test_changed_generation_and_credit_are_rejected(self):
        for field, value in (("generation", 2), ("credited_work", 2)):
            bundle, chain = self.fixture()
            bundle["slices_newest_first"][0]["shares"][0][field] = value
            with self.assertRaises(Invalid):
                verify_distribution(bundle, chain, {"alice": 0})

    def test_missing_window_never_becomes_zero(self):
        bundle, chain = self.fixture()
        bundle["slices_newest_first"] = []
        with self.assertRaises(Inconclusive):
            verify_distribution(bundle, chain, {"alice": 0})

    def test_same_band_different_owed_and_swapped_k_are_rejected(self):
        bundle, chain = self.fixture()
        bundle["entries"]["alice"]["owed_sats"] += 1
        with self.assertRaises(Invalid):
            verify_distribution(bundle, chain, {"alice": 0})

    def test_A3_omitted_front_or_missing_latest_never_passes(self):
        bundle, chain = self.fixture()
        bundle["snapshot"]["high_water"]["1"] = (1 << 56) + 1
        with self.assertRaisesRegex(Inconclusive, "high-water front/latest"):
            verify_distribution(bundle, chain, {"alice": 0})
        bundle, chain = self.fixture()
        bundle["snapshot"]["high_water"]["0"] = 3
        with self.assertRaises(Inconclusive):
            verify_distribution(bundle, chain, {"alice": 0})

    def test_A3_front_bits_and_unmapped_stream_ids_reject(self):
        bundle, chain = self.fixture()
        bundle["slices_newest_first"][0]["front_id"] = 1
        with self.assertRaisesRegex(Invalid, "id/front"):
            verify_distribution(bundle, chain, {"alice": 0})
        bundle, chain = self.fixture()
        bundle["snapshot"]["high_water"]["0"] = "1700000000000-0"
        with self.assertRaisesRegex(Inconclusive, "authenticated stream"):
            verify_distribution(bundle, chain, {"alice": 0})

    def test_A3_present_unselected_front_needs_authenticated_order(self):
        bundle, chain = self.fixture()
        sid = (1 << 56) + 1
        bundle["snapshot"]["high_water"]["1"] = sid
        bundle["slices_newest_first"].append({**bundle["slices_newest_first"][0], "front_id": 1, "slice_id": sid})
        with self.assertRaisesRegex(Inconclusive, "authenticated global publication order"):
            verify_distribution(bundle, chain, {"alice": 0})

    def test_A3_explicit_zero_front_does_not_add_work(self):
        bundle, chain = self.fixture()
        bundle["snapshot"]["high_water"]["1"] = 0
        self.assertEqual(verify_distribution(bundle, chain, {"alice": 0})["window_work"], 2)

    def crossing_fixture(self):
        bundle, chain = self.fixture()
        source = bundle["slices_newest_first"][0]
        first, second = source["shares"]
        third = None
        for nonce in range(256):
            if nonce in {first["nonce"], second["nonce"]}:
                continue
            raw_hash = sha256d(chain.tip["raw"][:76] + nonce.to_bytes(4, "little"))
            if int.from_bytes(raw_hash, "little") < 1 << 255:
                third = {**first, "nonce": nonce, "share_hash_le": raw_hash.hex()}
                break
        self.assertIsNotNone(third)
        rows = [{**source, "slice_id": 3, "difficulty": 1, "n_shares": 1, "shares": [first]},
                {**source, "slice_id": 2, "difficulty": 1, "n_shares": 1, "shares": [second]},
                {**source, "slice_id": 1, "difficulty": 1, "n_shares": 1, "shares": [third]}]
        bundle["slices_newest_first"] = rows
        bundle["finding_slice_id"] = 4
        bundle["snapshot"]["high_water"]["0"] = 3
        return bundle, chain

    def test_A3_Q_crossing_excludes_older_retained_tail(self):
        bundle, chain = self.crossing_fixture()
        proof = verify_distribution(bundle, chain, {"alice": 0})
        self.assertEqual(proof["selected_slice_ids"], [3, 2])
        self.assertEqual(proof["retained_tail_rows_outside_window"], 1)
        self.assertEqual(proof["window_work"], 2)

    def test_A3_reordered_window_and_Q_underfill_reject(self):
        bundle, chain = self.crossing_fixture()
        bundle["slices_newest_first"][0:2] = bundle["slices_newest_first"][0:2][::-1]
        with self.assertRaisesRegex(Invalid, "order"):
            verify_distribution(bundle, chain, {"alice": 0})
        bundle, chain = self.crossing_fixture()
        bundle["slices_newest_first"] = bundle["slices_newest_first"][:1]
        with self.assertRaisesRegex(Inconclusive, "underfilled"):
            verify_distribution(bundle, chain, {"alice": 0})

    def test_swapped_k_associated_amounts_are_rejected(self):
        bundle, chain = self.fixture()
        keys = bundle["entries"]["alice"]["p_x"]
        keys[0], keys[1] = keys[1], keys[0]
        with self.assertRaises(Invalid):
            verify_distribution(bundle, chain, {"alice": 0})


if __name__ == "__main__":
    unittest.main()
