"""Offline conformance vectors only; these never count as funded acceptance."""
import copy
import json
import unittest
from dataclasses import replace

from oracle import (Chain, Inconclusive, Invalid, Policy, Reader, compact_size,
                    decode_block, decode_transaction, expected_bits, header,
                    ladder, merkle, raw_payouts, rollback_proof, sha256d,
                    strict_json, target, to_compact, witness_commitment)
from journal import Journal, hash_budget, native_observations, parse_records


def policy(**changes):
    p = Policy("0" * 64, 0x207fffff, 0x200fffff, 0x200fffff,
               True, True, 600, 1209600, 2016, False, None, 5_000_000_000, 150, 100)
    return replace(p, **changes)


def tx_bytes(inputs=None, outputs=None, witness=None):
    inputs = inputs or [(b"\x01" * 32, 0, b"", 0xffffffff)]
    outputs = outputs or [(900, b"\x51")]
    body = compact_size(len(inputs))
    for prev, index, script, seq in inputs:
        body += prev + index.to_bytes(4, "little") + compact_size(len(script)) + script + seq.to_bytes(4, "little")
    body += compact_size(len(outputs))
    for amount, script in outputs:
        body += amount.to_bytes(8, "little") + compact_size(len(script)) + script
    if witness is not None:
        body += b"".join(compact_size(len(stack)) + b"".join(compact_size(len(x)) + x for x in stack) for stack in witness)
    return b"\x02\0\0\0" + (b"\0\x01" if witness is not None else b"") + body + bytes(4)


def coinbase(amount=5_000_000_000, outputs=None, witness=None, tag=b"\x01\x01"):
    return tx_bytes([(bytes(32), 0xffffffff, tag, 0xffffffff)], outputs or [(amount, b"\x51")], witness)


def block_bytes(previous, txs, time=1600000000):
    """Bounded offline artificial header vector, never submitted to a node."""
    root = merkle([decode_transaction(tx)["txid_bytes"] for tx in txs])
    prefix = b"\x01\0\0\0" + previous + root + time.to_bytes(4, "little") + bytes.fromhex("ffff7f20")
    for nonce in range(256):
        raw = prefix + nonce.to_bytes(4, "little")
        if int.from_bytes(sha256d(raw), "little") <= target(0x207fffff):
            return raw + compact_size(len(txs)) + b"".join(txs)
    raise AssertionError("offline vector's finite header cap exhausted")


class DecoderTests(unittest.TestCase):
    def test_compact_size_boundaries(self):
        for n in (0, 1, 252, 253, 65535, 65536, 0xffffffff, 1 << 32):
            self.assertEqual(Reader(compact_size(n)).count((1 << 64) - 1), n)

    def test_noncanonical_varints(self):
        for raw in ("fdfc00", "feffff0000", "ffffffffff00000000"):
            with self.subTest(raw=raw), self.assertRaises(Invalid):
                Reader(bytes.fromhex(raw)).count((1 << 64) - 1)

    def test_truncated_varints(self):
        for raw in (b"", b"\xfd", b"\xfd\1", b"\xfe\0\0", b"\xff\0"):
            with self.subTest(raw=raw), self.assertRaises(Invalid):
                Reader(raw).count()

    def test_txid_display_endian_and_fee(self):
        raw = tx_bytes()
        tx = decode_transaction(raw)
        self.assertEqual(tx["txid"], sha256d(raw)[::-1].hex())
        self.assertEqual(tx["weight"], 4 * len(raw))
        self.assertEqual(tx["outputs"][0]["value"], 900)

    def test_amount_overflow(self):
        for outputs in ([(1 << 63, b"\x51")], [(21_000_000 * 100_000_000, b"\x51"), (1, b"\x51")]):
            with self.assertRaises(Invalid):
                decode_transaction(tx_bytes(outputs=outputs))

    def test_duplicate_inputs(self):
        inp = (b"\1" * 32, 0, b"", 0xffffffff)
        with self.assertRaises(Invalid):
            decode_transaction(tx_bytes([inp, inp]))

    def test_unknown_witness_flag(self):
        raw = tx_bytes(witness=[[b"sig"]])
        with self.assertRaises(Invalid):
            decode_transaction(raw[:5] + b"\x02" + raw[6:])

    def test_superfluous_witness(self):
        with self.assertRaises(Invalid):
            decode_transaction(tx_bytes(witness=[[]]))

    def test_trailing_and_truncated_transaction(self):
        for raw in (tx_bytes() + b"\0", tx_bytes()[:-1]):
            with self.assertRaises(Invalid):
                decode_transaction(raw)

    def test_wtxid_distinction(self):
        tx = decode_transaction(tx_bytes(witness=[[b"sig", b"key"]]))
        self.assertNotEqual(tx["txid"], tx["wtxid"])
        self.assertEqual(tx["txid"], decode_transaction(tx["stripped"])["txid"])
        self.assertEqual(tx["weight"], 3 * len(tx["stripped"]) + len(tx["raw"]))

    def test_merkle_odd_and_mutation(self):
        a, b, c = [sha256d(bytes([x])) for x in range(3)]
        self.assertEqual(merkle([a, b, c]), sha256d(sha256d(a + b) + sha256d(c + c)))
        with self.assertRaises(Invalid):
            merkle([a, b, c, c])

    def test_compact_valid_33_34_and_invalid(self):
        self.assertEqual(target(0x21007fff), 0x7fff << 240)
        self.assertEqual(target(0x220000ff), 0xff << 248)
        for bits in (0, 0x20800001, 0x23000001, 0x21010000, 0x01000001):
            with self.subTest(bits=bits), self.assertRaises(Invalid):
                target(bits)
        self.assertEqual(to_compact(target(0x207fffff)), 0x207fffff)

    def test_duplicate_json_and_decimal(self):
        with self.assertRaises(Invalid):
            strict_json('{"x":1,"x":2}')
        with self.assertRaises(Invalid):
            strict_json('{"x":NaN}')
        self.assertEqual(str(strict_json('{"x":0.00000001}')["x"]), '1E-8')


class DifficultyVectors(unittest.TestCase):
    def test_fresh_long_gap_legitimate_powlimit(self):
        self.assertEqual(expected_bits(policy(), [{"bits": 0x207fffff, "time": 100}], 1301), 0x207fffff)

    def test_exact_gap_boundary_uses_overlay(self):
        self.assertEqual(expected_bits(policy(), [{"bits": 0x207fffff, "time": 100}], 1300), 0x200fffff)

    def test_strict_no_min_flag(self):
        self.assertEqual(expected_bits(policy(allow_min_difficulty_blocks=False), [{"bits": 0x207fffff, "time": 100}], 9999), 0x200fffff)

    def test_harder_inheritance(self):
        self.assertEqual(expected_bits(policy(), [{"bits": 0x1f7fffff, "time": 100}], 101), 0x1f7fffff)

    def test_walk_back_powlimit(self):
        ancestry = [{"bits": 0x207fffff, "time": 100}, {"bits": 0x200fffff, "time": 101},
                    {"bits": 0x207fffff, "time": 1400}]
        self.assertEqual(expected_bits(policy(), ancestry, 1401), 0x200fffff)

    def test_boundary_parent_precedes_gap(self):
        p = policy(target_timespan=1200, difficulty_adjustment_interval=2)
        ancestry = [{"bits": 0x207fffff, "time": 100}, {"bits": 0x200fffff, "time": 101}]
        self.assertEqual(expected_bits(p, ancestry, 9999), 0x200fffff)
        ancestry[-1]["bits"] = 0x207fffff
        self.assertEqual(expected_bits(p, ancestry, 102), 0x207fffff)

    def test_retarget_clamps_and_floor(self):
        p = policy(no_pow_retargeting=False, allow_min_difficulty_blocks=False,
                   target_timespan=1200, difficulty_adjustment_interval=2, retarget_floor_bits=0x200fffff)
        a = [{"bits": 0x200fffff, "time": 100}, {"bits": 0x200fffff, "time": 101}]
        self.assertEqual(expected_bits(p, a, 102), to_compact(target(0x200fffff) // 4))
        a[-1]["time"] = 10000
        self.assertEqual(expected_bits(p, a, 10001), 0x200fffff)

    def test_missing_flag_rejected(self):
        with self.assertRaises(Invalid):
            policy(allow_min_difficulty_blocks=None)

    def test_subsidy_halvings(self):
        p = policy()
        self.assertEqual([p.subsidy(h) for h in (149, 150, 300, 9600)], [5000000000, 2500000000, 1250000000, 0])


class WitnessVectors(unittest.TestCase):
    def fixture(self, duplicate=False):
        legacy = decode_transaction(tx_bytes())
        wit = decode_transaction(tx_bytes(outputs=[(800, b"\x51")], witness=[[b"sig"]]))
        reserved = bytes(range(32))
        root = merkle([bytes(32), legacy["txid_bytes"], wit["wtxid_bytes"]])
        script = bytes.fromhex("6a24aa21a9ed") + sha256d(root + reserved)
        outputs = [(5000000000, b"\x51")]
        if duplicate:
            outputs.append((0, bytes.fromhex("6a24aa21a9ed") + bytes(32)))
        outputs.append((0, script))
        cb = decode_transaction(coinbase(outputs=outputs, witness=[[reserved]]))
        return [cb, legacy, wit]

    def test_mixed_valid_commitment(self):
        result = witness_commitment(self.fixture())
        self.assertTrue(result["mixed"])
        self.assertEqual(result["witness_transactions"], 1)

    def test_bip141_highest_match(self):
        self.assertEqual(witness_commitment(self.fixture(True), production=False)["index"], 2)
        with self.assertRaises(Invalid):
            witness_commitment(self.fixture(True), production=True)

    def test_bad_reserved_and_missing(self):
        txs = self.fixture()
        txs[0]["witness"] = [[bytes(32)]]
        with self.assertRaises(Invalid):
            witness_commitment(txs)
        txs = self.fixture()
        txs[0]["outputs"].pop()
        with self.assertRaises(Invalid):
            witness_commitment(txs)

    def test_witness_mutation_detected(self):
        txs = self.fixture()
        txs[2] = decode_transaction(tx_bytes(outputs=[(800, b"\x51")], witness=[[b"other"]]))
        with self.assertRaises(Invalid):
            witness_commitment(txs)


class ChainVectors(unittest.TestCase):
    def test_retained_coinbase_actual_100_block_maturity_rollback(self):
        g = block_bytes(bytes(32), [coinbase(tag=b"\1\0")])
        p = policy(genesis=sha256d(g[:80])[::-1].hex(), regtest_pow_bits_overlay=None)
        raws = [g]
        for height in range(1, 102):
            raws.append(block_bytes(sha256d(raws[-1][:80]),
                                    [coinbase(tag=b"\x02" + height.to_bytes(2, "little"))],
                                    1600000000 + height))
        high, low = Chain(p, raws), Chain(p, raws[:100])
        point = high.blocks[1]["transactions"][0]["txid"] + ":0"
        # The retained output is in BOTH canonical views; this is distinct
        # from merely refusing an outpoint whose funding block was displaced.
        self.assertIn(point, high.utxos)
        self.assertIn(point, low.utxos)
        verifier = Journal.__new__(Journal)
        verifier.chain = high
        verifier.eligible(point)
        verifier.chain = low
        with self.assertRaisesRegex(Invalid, "retained coinbase immature"):
            verifier.eligible(point)
        verifier.chain = high
        verifier.eligible(point)

    def test_raw_fee_and_conservation(self):
        genesis = block_bytes(bytes(32), [coinbase(tag=b"\x01\0")])
        p = policy(genesis=sha256d(genesis[:80])[::-1].hex(), regtest_pow_bits_overlay=None,
                   initial_target_bits=0x207fffff, coinbase_maturity=1)
        cb1 = coinbase(tag=b"\x01\1")
        b1 = block_bytes(sha256d(genesis[:80]), [cb1], 1600000001)
        spend = tx_bytes([(decode_transaction(cb1)["txid_bytes"], 0, b"", 0xffffffff)], [(4999999000, b"\x51")])
        cb2 = coinbase(amount=5000001000, tag=b"\x01\2")
        b2 = block_bytes(sha256d(b1[:80]), [cb2, spend], 1600000002)
        chain = Chain(p, [genesis, b1, b2])
        self.assertEqual(chain.tip["fees"], 1000)
        self.assertEqual(chain.tip["claim"], chain.tip["subsidy"] + 1000)
        self.assertEqual(chain.fee(decode_transaction(spend)), 1000)
        point = decode_transaction(cb1)["txid"] + ":0"
        self.assertIn(point, chain.prefix(1).utxos)
        self.assertNotIn(point, chain.prefix(2).utxos)
        self.assertNotIn(decode_transaction(spend)["txid"], chain.prefix(1).transactions)
        self.assertIs(chain.prefix(1).master, chain)

    def test_excess_coinbase_and_merkle(self):
        raw = block_bytes(bytes(32), [coinbase(amount=5000000001)])
        p = policy(genesis=sha256d(raw[:80])[::-1].hex())
        with self.assertRaises(Invalid):
            Chain(p, [raw])
        with self.assertRaises(Invalid):
            decode_block(raw[:-1] + b"\x01", p)

    def test_rollback_exact_headers(self):
        g = block_bytes(bytes(32), [coinbase(tag=b"\1\0")])
        p = policy(genesis=sha256d(g[:80])[::-1].hex(), regtest_pow_bits_overlay=None)
        b = block_bytes(sha256d(g[:80]), [coinbase(tag=b"\1\1")], 1600000001)
        old, new = Chain(p, [g, b]), Chain(p, [g])
        anchor = lambda c: {"block_hash": c.tip["hash"], "height": c.tip["height"]}
        proof = {"version": 1, "old_anchor": anchor(old), "new_anchor": anchor(new),
                 "common_ancestor": anchor(new), "old_headers": [b[:80].hex(), g[:80].hex()],
                 "new_headers": [g[:80].hex()], "displaced_block_hashes": [old.tip["hash"]]}
        rollback_proof(proof, old, new)
        bad = copy.deepcopy(proof)
        bad["old_headers"].reverse()
        with self.assertRaises(Invalid):
            rollback_proof(bad, old, new)


class AccountingVectors(unittest.TestCase):
    def native_fixture(self):
        g = block_bytes(bytes(32), [coinbase(tag=b"\1\0")])
        p = policy(genesis=sha256d(g[:80])[::-1].hex())
        chain = Chain(p, [g])
        b = block_bytes(sha256d(g[:80]), [coinbase(tag=b"\1\1")], 1600001201)
        auth = {"operation_id": "op", "driver_incarnation": "one", "batch_id": 1,
                "reserved_attempts": 1, "cumulative_reserved": 1}
        command = [{"record": "hash_start", "operation": {"operation_id": "op", "deadline_monotonic_ms": 100}},
                   {"record": "hash_reserve", "seq": 2, **auth}]
        records = [{"event": "hash_batch", "source": "bootstrap", "exclude_from_statistics": True,
                    "monotonic_ms": 10, "authorization": auth, "measured_attempts": 1,
                    "raw_headers": [b[:80].hex()], "target": target(0x207fffff).to_bytes(32, "big").hex(), "height": 1}]
        return records, command, {chain.tip["hash"]: chain}, p

    def test_native_observations_cannot_authorize_hashes(self):
        records, commands, views, p = self.native_fixture()
        proof = native_observations(records, commands, views, p, 1600000000)
        self.assertTrue(proof["observation_only"])
        self.assertEqual(proof["raw_attempt_headers"], 1)
        with self.assertRaises(Invalid):
            native_observations(records, [], views, p, 1600000000)

    def test_native_expectedbits_is_not_copied_from_log(self):
        records, commands, views, p = self.native_fixture()
        raw = bytearray.fromhex(records[0]["raw_headers"][0])
        raw[72:76] = (0x200fffff).to_bytes(4, "little")
        records[0]["raw_headers"] = [raw.hex()]
        records[0]["target"] = target(0x200fffff).to_bytes(32, "big").hex()
        with self.assertRaisesRegex(Invalid, "native contextual target"):
            native_observations(records, commands, views, p, 1600000000)

    def test_native_faked_old_time_and_reused_incarnation(self):
        records, commands, views, p = self.native_fixture()
        with self.assertRaisesRegex(Invalid, "historical"):
            native_observations(records, commands, views, p, 1700000000)
        records[0]["authorization"] = {**records[0]["authorization"], "driver_incarnation": "two"}
        with self.assertRaises(Invalid):
            native_observations(records, commands, views, p, 1600000000)

    def test_spec_worked_example(self):
        s = [[{"identity": "A", "work": 60, "fees": 15000000}, {"identity": "B", "work": 40, "fees": 15000000}],
             [{"identity": "A", "work": 50, "fees": 18000000}, {"identity": "B", "work": 30, "fees": 18000000},
              {"identity": "C", "work": 20, "fees": 19000000}]]
        payouts, pool = raw_payouts(s, 312500000, 20000000, 100)
        self.assertEqual(payouts, {"A": 180991854, "B": 115178612, "C": 33004532})
        self.assertEqual(pool, 3325002)

    def test_fixed_ladder(self):
        self.assertEqual(ladder(999999), ([500000], 499999))
        self.assertEqual(ladder(12345678), ([5000000, 5000000, 1000000, 1000000], 345678))

    def test_history_gaps_unknown_truncated(self):
        for data in (b"", b"{}", b"{}\n", b'{"record":"poison","seq":0}\n'):
            with self.assertRaises(Inconclusive):
                parse_records(data)

    def test_hash_reservation_unresolved_no_refund(self):
        r = {"record": "hash_reserve", "seq": 1, "operation_id": "op", "driver_incarnation": "a",
             "batch_id": 1, "reserved_attempts": 20, "cumulative_reserved": 20}
        self.assertEqual(hash_budget([r], 40, require_start=False), {"charged_reserved": 20, "measured_executed": 0, "charged_unresolved": 20})
        with self.assertRaises(Invalid):
            hash_budget([r, {**r, "seq": 2}], 40, require_start=False)
        with self.assertRaises(Invalid):
            hash_budget([r], 19, require_start=False)

    def test_missing_budget_history_inconclusive(self):
        r = {"record": "hash_reserve", "seq": 1, "operation_id": "op", "driver_incarnation": "a",
             "batch_id": 1, "reserved_attempts": 20, "cumulative_reserved": 20}
        with self.assertRaises(Inconclusive):
            hash_budget([r], 40)

    def test_no_send_pause_reopen_resume(self):
        h = {"record": "header", "seq": 0, "format_magic": "SPC-LAB-TRAFFIC-JOURNAL", "history_version": 3,
             "protocol": "spc-lab-tx-journal/3", "network": "regtest", "key_mode": "lab_deterministic_run_seed_v1",
             "derivation_version": 2, "run_id": "test", "driver_id": "txd-0", "run_seed_fingerprint": "0" * 64,
             "allocation_digest": "1" * 64, "caps": {"unknown_retry_sends": 3, "reorg_resubmits": 3}, "created_unix_ms": 1}
        barrier = {"record": "send_barrier", "seq": 1, "command_id": "pause", "wall_unix_ms": 1,
                   "monotonic_ms": 1, "last_txid": None, "send_sequence": 0}
        session = {"record": "session_started", "seq": 2, "restarted_unix_ms": 2}
        rows = parse_records(b"".join(json.dumps(r).encode() + b"\n" for r in [h, barrier, session]))
        verifier = Journal(rows, {"run_id": "test"}, {})
        self.assertTrue(verifier.paused)
        self.assertEqual(verifier.send_sequence, 0)
        verifier.apply({"record": "send_resumed", "seq": 3, "command_id": "resume"})
        self.assertFalse(verifier.paused)
        with self.assertRaises(Invalid):
            Journal([h, {**barrier, "send_sequence": 1}], {}, {})


if __name__ == "__main__":
    unittest.main()
