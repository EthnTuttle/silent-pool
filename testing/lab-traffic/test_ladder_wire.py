"""Independent literal SPCB/2 wire vectors; no production encoder imports."""
import hashlib
import unittest

from ladder_wire import decode_spcb2
from oracle import Invalid

GX = bytes.fromhex("79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798")
TWO_GX = bytes.fromhex("c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5")


def vector(owed=8000001):
    prefix = b"SPCB\2" + b"\x11" * 32 + (105).to_bytes(4, "little") + b"\1" + b"\x22" * 32 + GX
    high = b"\1\0\1\x03" + b"123" + (65).to_bytes(16, "little")
    entry = (b"\1\0" + b"\x33" * 32 + b"\0\2\0" + GX + TWO_GX + b"\0\0"
             + (8000000).to_bytes(8, "little") + owed.to_bytes(8, "little"))
    return prefix + high + entry + b"\2\x20" + b"\x44" * 32 + b"\1"


class LadderWireTests(unittest.TestCase):
    def test_full_owed_and_separate_snapshot_identity(self):
        raw = vector()
        value = decode_spcb2(raw)
        self.assertEqual(value["entries"][0]["owed_sats"], 8000001)
        self.assertEqual(value["entries"][0]["p_x"], [GX.hex(), TWO_GX.hex()])
        self.assertEqual(value["snapshot_key"], hashlib.sha256(b"SPc/DistributionSnapshotV2" + raw).hexdigest())
        self.assertNotEqual(value["snapshot_key"], value["coinbase_fingerprint"])
        changed = decode_spcb2(vector(8000002))
        self.assertNotEqual(changed["snapshot_key"], value["snapshot_key"])
        self.assertEqual(changed["coinbase_fingerprint"], value["coinbase_fingerprint"])

    def test_old_version_and_trailing_bytes_rejected(self):
        for raw in (vector()[5:], b"SPCB\1" + vector()[5:], vector() + b"\0", vector()[:-1]):
            with self.assertRaises(Invalid):
                decode_spcb2(raw)

    def test_duplicate_and_invalid_keys_rejected(self):
        with self.assertRaises(Invalid):
            decode_spcb2(vector().replace(TWO_GX, GX))
        with self.assertRaises(Invalid):
            decode_spcb2(vector().replace(TWO_GX, b"\xff" * 32))

    def test_work_rule_is_not_inferred_from_version(self):
        with self.assertRaises(Invalid):
            decode_spcb2(vector()[:-1] + b"\0")


if __name__ == "__main__":
    unittest.main()
