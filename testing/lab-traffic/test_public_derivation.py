"""Public RIPEMD160/BIP32 vectors for the read-only derivation oracle."""
import unittest

from oracle import Invalid
from public_derivation import _child, _master, _public_scalar, expected_p2pkh_script, ripemd160


class PublicDerivationTests(unittest.TestCase):
    def test_ripemd160_published_vectors(self):
        vectors = [(b"", "9c1185a5c5e9fc54612808977ee8f548b2258d31"),
                   (b"a", "0bdc9d2d256b3ee9daae347be6f4dc835a467ffe"),
                   (b"message digest", "5d0689ef49d2fae572b881b123a85ffa21595f36"),
                   (b"abcdefghijklmnopqrstuvwxyz", "f71c27109c692c1b56bbdceb5b9d2865b3708dbc"),
                   (b"1234567890" * 8, "9b752e45573d4b39f4dbd3323cab82bf63326bfb")]
        for data, expected in vectors:
            with self.subTest(message=data):
                self.assertEqual(ripemd160(data).hex(), expected)

    def test_bip32_vector_one_public_keys(self):
        master = _master(bytes.fromhex("000102030405060708090a0b0c0d0e0f"))
        self.assertEqual(_public_scalar(master[0]).hex(), "0339a36013301597daef41fbe593a02cc513d0b55527ec2df1050e2e8ff49c85c2")
        child = _child(master, 0x80000000)
        self.assertEqual(_public_scalar(child[0]).hex(), "035a784662a4a20a65bf6aab9ae98a6c068a81c52e4b032c0fb5400c706cfccc56")

    def test_public_run_domain_and_roles_are_disjoint(self):
        scripts = {expected_p2pkh_script(1, run, role, chain, 0, hardened_index=True)
                   for run in ("a", "b") for chain in ("receive", "change")
                   for role in ({"kind": "bootstrap_funding"}, {"kind": "network_wallet", "index": 0},
                                {"kind": "user_payment", "index": 0})}
        self.assertEqual(len(scripts), 12)
        with self.assertRaises(Invalid):
            expected_p2pkh_script(1, "bad\0run", {"kind": "bootstrap_funding"}, "receive", 0, hardened_index=True)

    def test_child_hardening_is_explicit_and_not_a_fallback(self):
        args = (1, "a", {"kind": "bootstrap_funding"}, "receive", 0)
        self.assertNotEqual(expected_p2pkh_script(*args, hardened_index=True), expected_p2pkh_script(*args, hardened_index=False))
        with self.assertRaises(Invalid):
            expected_p2pkh_script(*args, hardened_index=None)


if __name__ == "__main__":
    unittest.main()
