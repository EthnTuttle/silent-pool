"""Before-sign v3 versus post-sign mutation: independent signature vectors."""
import unittest

import test_audit_regressions as vectors
from test_oracle import tx_bytes
from oracle import Invalid, decode_transaction, sha256d
from signature_oracle import verify_legacy_sighash_all
from journal import Journal


class SignatureOracleTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        vectors.FundedOutcomeRegressions.setUpClass()
        cls.fixture = vectors.FundedOutcomeRegressions

    def signed(self, version):
        f = self.fixture
        txid, vout = f.point.split(":")
        outputs = [(o["value"], o["script"]) for o in f.tx["outputs"]]
        prev = bytes.fromhex(txid)[::-1]
        pre = tx_bytes([(prev, int(vout), f.script, 0xffffffff)], outputs)
        pre = version.to_bytes(4, "little") + pre[4:] + b"\x01\0\0\0"
        signature = f.fixture_signer.ecdsa(1, sha256d(pre)) + b"\1"
        pub = f.fixture_signer.pubkey(1)
        script_sig = bytes([len(signature)]) + signature + bytes([len(pub)]) + pub
        raw = tx_bytes([(prev, int(vout), script_sig, 0xffffffff)], outputs)
        return version.to_bytes(4, "little") + raw[4:]

    def test_valid_version_three_signed_before_serialization(self):
        proof = verify_legacy_sighash_all(self.signed(3), self.fixture.chain, expected_version=3)
        self.assertEqual(proof["sighash_all_inputs_verified"], 1)
        self.assertEqual(proof["fee_sats"], 1000)
        self.assertFalse(proof["node_acceptance_or_rejection_inferred"])

    def test_post_signature_v2_to_v3_mutation_is_rejected(self):
        raw = self.signed(2)
        verify_legacy_sighash_all(raw, self.fixture.chain, expected_version=2)
        with self.assertRaisesRegex(Invalid, "ECDSA verification"):
            verify_legacy_sighash_all(b"\3\0\0\0" + raw[4:], self.fixture.chain, expected_version=3)

    def test_changed_output_or_locktime_breaks_signature(self):
        raw = self.signed(3)
        modified = bytearray(raw)
        modified[-4] ^= 1
        with self.assertRaisesRegex(Invalid, "ECDSA verification"):
            verify_legacy_sighash_all(bytes(modified), self.fixture.chain, expected_version=3)
        tx = decode_transaction(raw)
        value = tx["outputs"][0]["value"].to_bytes(8, "little")
        modified = raw.replace(value, (tx["outputs"][0]["value"] + 1).to_bytes(8, "little"), 1)
        with self.assertRaisesRegex(Invalid, "ECDSA verification"):
            verify_legacy_sighash_all(modified, self.fixture.chain, expected_version=3)

    def test_immature_prevout_cannot_satisfy_funded_signature_case(self):
        with self.assertRaisesRegex(Invalid, "immature"):
            verify_legacy_sighash_all(self.signed(3), self.fixture.chain.prefix(100), expected_version=3)

    def test_exact_construction_effect_binds_journal_version(self):
        helper = self.fixture("test_A1_unknown_actual_result_cannot_release_first_send")
        history = helper.history(["accepted_by_rpc"])[:6]
        signed = history[-1]
        raw = self.signed(3)
        decoded = decode_transaction(raw)
        signed.update(raw_hex=raw.hex(), txid=decoded["txid"], weight_wu=decoded["weight"], vbytes=decoded["vbytes"])
        with self.assertRaisesRegex(Invalid, "immutable before-sign"):
            Journal(history, {"run_id": "run"}, {self.fixture.chain.tip["hash"]: self.fixture.chain})
        history[3]["request"]["construction_effect"] = {"schema": "spc-lab-version-rejection/v1"}
        Journal(history, {"run_id": "run"}, {self.fixture.chain.tip["hash"]: self.fixture.chain})
        for invalid in ({"schema": "spc-lab-version-rejection/v2"}, {"schema": "spc-lab-version-rejection/v1", "version": 3}, None):
            history[3]["request"]["construction_effect"] = invalid
            with self.assertRaisesRegex(Invalid, "construction effect"):
                Journal(history, {"run_id": "run"}, {self.fixture.chain.tip["hash"]: self.fixture.chain})


if __name__ == "__main__":
    unittest.main()
