"""Test-only signer known vectors. Never a live witness acceptance receipt."""
import importlib.util
from pathlib import Path
import unittest

from oracle import Invalid, address_script, decode_transaction

path = Path(__file__).resolve().parent.parent / "lab-witness" / "fixture.py"
spec = importlib.util.spec_from_file_location("witness_fixture", path)
fixture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixture)


class FixtureTests(unittest.TestCase):
    def test_public_bip340_vector_zero(self):
        # bitcoin/bips bip-0340/test-vectors.csv, vector 0: sk=3,
        # aux=000...000, message=000...000. Independent published expected sig.
        expected = ("e907831f80848d1069a5371b402410364bdf1c5f8307b0084c55f1ce2dca8215"
                    "25f66a4a85ea8b71e482a74f382d2ce5ebeee8fdb2172f477df4900d310536c0")
        self.assertEqual(fixture.schnorr(3, bytes(32)).hex(), expected)

    def test_p2wpkh_known_public_keyhash(self):
        self.assertEqual(fixture.script("p2wpkh").hex(), "0014751e76e8199196d454941c45d1b3a323f1433bd6")
        self.assertTrue(fixture.address("p2wpkh").startswith("bcrt1q"))
        self.assertTrue(fixture.address("p2tr").startswith("bcrt1p"))
        for kind in ("p2wpkh", "p2tr"):
            self.assertEqual(address_script(fixture.address(kind)), fixture.script(kind))
            with self.assertRaises(Invalid):
                address_script(fixture.address(kind)[:-1] + "!")

    def test_fixture_spends_have_distinct_txid_wtxid(self):
        for kind in ("p2wpkh", "p2tr"):
            raw = fixture.spend(kind, "12" * 32, 7, 100000, fixture.script(kind), 1000)
            tx = decode_transaction(raw)
            self.assertNotEqual(tx["txid"], tx["wtxid"])
            self.assertEqual(tx["outputs"][0]["value"], 99000)
            self.assertEqual(tx["inputs"][0]["outpoint"], "12" * 32 + ":7")
            self.assertEqual(len(tx["witness"][0]), 2 if kind == "p2wpkh" else 1)

    def test_test_only_scalar_no_derivation(self):
        self.assertEqual(fixture.FIXTURE_KEYS, {"p2wpkh": 1, "p2tr": 2})
        with self.assertRaises(ValueError):
            fixture.spend("p2tr", "00" * 32, 0, 100, b"\x51", 101)


if __name__ == "__main__":
    unittest.main()
