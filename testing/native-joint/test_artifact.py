"""Runner correlation regressions; execute in the primary check, not at import."""
import unittest

from artifact import ATerm, correlate


SOURCE = "/nix/store/" + "0" * 32 + "-source"
OUTPUT = "/nix/store/" + "1" * 32 + "-spc-cpu-miner"
IDENTITY = "git:" + "a" * 40


def derivation(source=SOURCE, output=OUTPUT, identity=IDENTITY, inputs=None):
    import json
    def term(value):
        if isinstance(value, str):
            return json.dumps(value)
        left, right = ("(", ")") if isinstance(value, tuple) else ("[", "]")
        return left + ",".join(term(x) for x in value) + right
    return "Derive" + term(([("out", output, "", "")], [], inputs or [source],
                            "x86_64-linux", "/builder", [],
                            [("src", source), ("out", output),
                             ("SPC_BUILD_SOURCE_ID", identity)]))


class Correlation(unittest.TestCase):
    def test_classic_derivation(self):
        correlate(derivation(), OUTPUT, SOURCE, IDENTITY)

    def test_wrong_actual_source(self):
        with self.assertRaises(ValueError):
            correlate(derivation(source=SOURCE + "-other"), OUTPUT, SOURCE, IDENTITY)

    def test_wrong_actual_output(self):
        with self.assertRaises(ValueError):
            correlate(derivation(output=OUTPUT + "-other"), OUTPUT, SOURCE, IDENTITY)

    def test_source_label_without_input_dependency(self):
        with self.assertRaises(ValueError):
            correlate(derivation(inputs=[SOURCE + "-other"]), OUTPUT, SOURCE, IDENTITY)

    def test_wrong_actual_commit(self):
        with self.assertRaises(ValueError):
            correlate(derivation(identity="git:" + "b" * 40), OUTPUT, SOURCE, IDENTITY)

    def test_snapshot_does_not_claim_commit(self):
        correlate(derivation(identity="nix-snapshot"), OUTPUT, SOURCE, "nix-snapshot")
        with self.assertRaises(ValueError):
            correlate(derivation(identity="nix-snapshot"), OUTPUT, SOURCE, IDENTITY)

    def test_unknown_format_and_trailing_bytes(self):
        for raw in (derivation() + "garbage", "DrvWithVersion(1,[])"):
            with self.assertRaises(ValueError):
                ATerm(raw).parse()


if __name__ == "__main__":
    unittest.main()
