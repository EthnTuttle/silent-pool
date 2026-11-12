import unittest

from common import Blocked
from raw_verify import Reader, compact, dh, merkle, parse_block, parse_tx, transaction_record


def tx(parent=None, value=1000, witness=None):
    coinbase = parent is None
    prev = bytes(32) if coinbase else bytes.fromhex(parent)[::-1]
    index = 0xffffffff if coinbase else 0
    script = b"\x01\x01" if coinbase else b"\x51"
    vin = prev + index.to_bytes(4, "little") + compact(len(script)) + script + b"\xff" * 4
    vout = value.to_bytes(8, "little") + b"\x01\x51"
    body = b"\x02\0\0\0" + (b"\0\x01" if witness is not None else b"") + b"\x01" + vin + b"\x01" + vout
    if witness is not None:
        body += compact(len(witness)) + b"".join(compact(len(v)) + v for v in witness)
    return (body + bytes(4)).hex()


def unit_block(transactions):
    # Tiny bounded in-memory verifier fixture. No daemon/RPC/funding/miner is
    # started and these bytes are never submitted to any chain.
    root = merkle([bytes.fromhex(parse_tx(t)["txid"])[::-1] for t in transactions])
    prefix = b"\x01\0\0\0" + bytes(32) + root + (1700000000).to_bytes(4, "little") + bytes.fromhex("ffff7f20")
    target = 0x7fffff << 232
    for n in range(256):
        header = prefix + n.to_bytes(4, "little")
        if int.from_bytes(dh(header), "little") <= target:
            return (header + compact(len(transactions)) + b"".join(bytes.fromhex(t) for t in transactions)).hex()
    raise AssertionError("unit fixture bound")


class RawEvidence(unittest.TestCase):
    def test_raw_identity_and_exact_fee_ignore_decoded_producer_claims(self):
        funding = tx(value=1000)
        funding_block = unit_block([funding])
        parent = parse_tx(funding)
        spend = tx(parent["txid"], value=900)
        inclusion = unit_block([tx(value=5000000000), spend])
        record = {"raw": spend, "txid": parse_tx(spend)["txid"], "raw_block": inclusion,
                  "block_hash": parse_block(inclusion)["hash"], "height": 101,
                  "fee_sats": 100, "decoded": {"fabricated": True}, "inputs": [
                      {"raw": funding, "txid": parent["txid"], "vout": 0, "amount_sats": 1000,
                       "height": 1, "raw_block": funding_block, "block_hash": parse_block(funding_block)["hash"]}]}
        self.assertEqual(transaction_record(record, 100)["fee_sats"], 100)
        for field, value in [("fee_sats", 0), ("txid", "00" * 32), ("height", 100)]:
            with self.assertRaises(Blocked):
                transaction_record({**record, field: value}, 100)
        record["inputs"][0]["amount_sats"] = 2000
        with self.assertRaises(Blocked):
            transaction_record(record, 100)

    def test_witness_changes_wtxid_not_txid_and_needs_commitment(self):
        parent = "11" * 32
        a, b = parse_tx(tx(parent, witness=[b"a"])), parse_tx(tx(parent, witness=[b"b"]))
        self.assertEqual(a["txid"], b["txid"])
        self.assertNotEqual(a["wtxid"], b["wtxid"])
        with self.assertRaisesRegex(Blocked, "witness commitment"):
            parse_block(unit_block([tx(), a["raw"]]))

    def test_truncation_nonminimal_sizes_and_trailing_bytes_rejected(self):
        with self.assertRaises(Blocked):
            Reader(b"\xfd\x01\0").size()
        with self.assertRaises(Blocked):
            parse_tx(tx() + "00")
        with self.assertRaises(Blocked):
            parse_tx(tx()[:-2])

    def test_raw_block_mutation_cannot_be_saved_by_status_label(self):
        block = bytearray.fromhex(unit_block([tx()]))
        block[36] ^= 1
        with self.assertRaises(Blocked):
            parse_block(block.hex())


if __name__ == "__main__":
    unittest.main()
