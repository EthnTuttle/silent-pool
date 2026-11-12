"""Pure validation/golden tests: no Rust builds, nodes or namespace services."""
import copy
import hashlib
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch, Mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from arkrpc.launchprofile import (loads, normalize, digest, canonical_bytes,
                                 isolated_profile, LaunchProfileError)
from arkrpc.nodeproc import ProfileOwnedNode, NodeError

CONFIG = b"minrelaytxfee=0.00001000\n"
ENDPOINTS = ["10.231.0.2:18444", "10.231.0.3:18444", "10.231.0.2:18443"]


def namespace_profile():
    return {
        "profile_version": 1, "binary_sha256": "ab" * 32,
        "argv": ["--network", "regtest", "--datadir", "/run/lab/private/node-a", "--rpc",
            "--rpc-listen", "10.231.0.2:18443", "--rpc-cookie-file", "/run/lab/private/a.cookie",
            "--regtest-pow-bits", "200fffff", "--no-seeds", "--no-discover", "--max-outbound", "1",
            "--api-log", "/run/lab/private/a.jsonl", "--log-level", "info", "--max-run-secs", "300",
            "--listen", "10.231.0.2:18444", "--connect", "10.231.0.3:18444",
            "--config", "/policy/node.conf"],
        "env": {"SPC_LAB_RUN_TOKEN": "golden_public_fixture_token", "RBITCOIN_RPC_PACKAGE_DIALECT": "1"},
        "config_files": [{"argv_index": 26, "sha256": hashlib.sha256(CONFIG).hexdigest()}],
        "substitutions": [{"class": "datadir", "argv_index": 3},
            {"class": "rpc-listen", "argv_index": 6}, {"class": "cookie-file", "argv_index": 8},
            {"class": "p2p-isolation", "span": [11, 15]}, {"class": "api-log", "span": [15, 19]},
            {"class": "max-run-secs", "argv_index": 20}, {"class": "p2p-isolation", "span": [21, 25]}],
    }


def norm(raw):
    return normalize(raw, owned_endpoints=ENDPOINTS, read_bytes=lambda _: CONFIG)


def isolated(raw):
    with patch("arkrpc.launchprofile.Path.read_bytes", return_value=CONFIG):
        return isolated_profile(raw, datadir="/private/probe/datadir", rpc_listen="127.0.0.1:19443",
            cookie_file="/private/probe/rpc.cookie", api_log="/private/probe/api.jsonl",
            owned_endpoints=ENDPOINTS)


class LaunchProfileTests(unittest.TestCase):
    def test_rpc_compat_policy_goldens_preserve_literal_forms(self):
        fixtures = Path(__file__).parents[1] / "fixtures"
        golden = json.loads((fixtures / "launch-profile-rpc-policy-goldens.json").read_bytes())
        self.assertEqual(hashlib.sha256((fixtures / "launch-profile-goldens.json").read_bytes()).hexdigest(),
                         golden["base_fixture_sha256"])
        default = namespace_profile()
        self.assertEqual(digest(norm(default)), golden["default_comparison_sha256"])
        for case in golden["cases"]:
            with self.subTest(form=case["name"]):
                raw = copy.deepcopy(default)
                raw["argv"].extend(case["append_argv"])
                probe = isolated(raw)
                comparison = norm(raw)
                self.assertEqual(comparison, norm(probe))
                self.assertEqual(digest(comparison), case["comparison_sha256"])
                self.assertEqual(canonical_bytes(comparison).decode(), case["canonical_utf8"])
                self.assertNotEqual(digest(comparison), golden["default_comparison_sha256"])
                self.assertEqual(comparison["argv"][-len(case["append_argv"]):],
                                 [{"literal": token} for token in case["append_argv"]])
                start = probe["argv"].index(case["append_argv"][0])
                self.assertEqual(probe["argv"][start:start + len(case["append_argv"])], case["append_argv"])
                self.assertEqual(probe["env"], raw["env"])
                self.assertFalse(any(s["class"] == "rpc-compat-profile" for s in probe["substitutions"]))

    def test_rpc_compat_policy_unknown_duplicate_and_substitution_refusals(self):
        bad_forms = [
            ["--rpc-compat-profile"], ["--rpc-compat-profile", ""],
            ["--rpc-compat-profile", "core-v31"], ["--rpc-compat-profile", "bark-v32"],
            ["--rpc-compat-profile", "310000"], ["--rpc-compat-profile", "BARK-V31"],
            ["--rpc-compat-profile=bark-v32"], ["--rpc-compat-profile="],
            ["--rpc-compat-profile", "bark-v31", "--rpc-compat-profile", "bark-v31"],
            ["--rpc-compat-profile=bark-v31", "--rpc-compat-profile=bark-v31"],
            ["--rpc-compat-profile", "bark-v31", "--rpc-compat-profile=bark-v31"],
            ["--rpc-compat-profile=bark-v31", "--rpc-compat-profile", "bark-v31"],
            ["--rpc-compat-profile", "bark-v31", "--rpc-compat-profile", "core-v31"],
        ]
        for tokens in bad_forms:
            p = namespace_profile()
            p["argv"].extend(tokens)
            with self.subTest(tokens=tokens), self.assertRaises(LaunchProfileError):
                norm(p)
        for cls in ("rpc-compat-profile", "datadir", "p2p-isolation"):
            p = namespace_profile()
            start = len(p["argv"])
            p["argv"].extend(["--rpc-compat-profile", "bark-v31"])
            p["substitutions"].append({"class": cls, "span": [start, start + 2]})
            with self.subTest(substitution=cls), self.assertRaises(LaunchProfileError):
                norm(p)

    def test_rpc_compat_policy_regtest_and_exact_env_conflicts(self):
        for dialect in ("0", "false", "FALSE", "False", "off", " true", "2"):
            p = namespace_profile()
            p["argv"].extend(["--rpc-compat-profile", "bark-v31"])
            p["env"]["RBITCOIN_RPC_PACKAGE_DIALECT"] = dialect
            with self.subTest(dialect=dialect), self.assertRaises(LaunchProfileError):
                norm(p)
        for dialect in (None, "", "1", "true", "TRUE", "True"):
            p = namespace_profile()
            p["argv"].extend(["--rpc-compat-profile=bark-v31"])
            if dialect is None:
                p["env"].pop("RBITCOIN_RPC_PACKAGE_DIALECT")
            else:
                p["env"]["RBITCOIN_RPC_PACKAGE_DIALECT"] = dialect
            self.assertEqual(norm(p)["env"], p["env"])
        p = namespace_profile()
        p["argv"][1] = "signet"
        p["argv"].extend(["--rpc-compat-profile", "bark-v31"])
        with self.assertRaises(LaunchProfileError):
            norm(p)
        # No selector remains default policy even with an explicitly-off dialect.
        p = namespace_profile()
        p["env"]["RBITCOIN_RPC_PACKAGE_DIALECT"] = "0"
        self.assertEqual(norm(p)["env"], p["env"])

    def test_bad_rpc_compat_cli_profiles_fail_before_effects(self):
        from arkrpc.cli import main
        with tempfile.TemporaryDirectory(prefix="traffic-policy-test-") as root:
            for n, tokens in enumerate((["--rpc-compat-profile", "bark-v32"],
                ["--rpc-compat-profile", "bark-v31", "--rpc-compat-profile=bark-v31"])):
                p = namespace_profile()
                p["argv"].extend(tokens)
                path = Path(root) / (str(n) + ".json")
                path.write_text(json.dumps(p))
                path.chmod(0o600)
                out = Path(root) / (str(n) + "-out")
                with patch("arkrpc.nodeproc.validate_parent_handoff", side_effect=AssertionError("too late")) as handoff, \
                        patch("arkrpc.nodeproc.subprocess.Popen") as spawn, self.assertRaises(SystemExit):
                    main(["node-probe", "--binary", "/invalid/node", "--source-name", "fixture", "--label", "fixture",
                        "--out", str(out), "--launch-profile", str(path), "--actor-exec", "/invalid/spc-lab",
                        "--actor-exec-sha256", "ab"*32, "--controller-pid", "42", "--controller-start-ticks", "1"])
                handoff.assert_not_called()
                spawn.assert_not_called()
                self.assertFalse(out.exists())

    def test_namespace_and_isolated_golden(self):
        raw = namespace_profile()
        probe = isolated(raw)
        self.assertNotEqual(raw["argv"], probe["argv"])
        self.assertEqual(norm(raw), norm(probe))
        self.assertEqual(set(norm(raw)), {"profile_version", "binary_sha256", "argv", "env", "config_files", "p2p_isolation"})
        self.assertEqual(norm(raw)["p2p_isolation"], "validated_lab_transport")
        self.assertEqual(norm(raw)["config_files"][0]["argv_index"], 15)
        fixture = json.loads((Path(__file__).parents[1] / "fixtures/launch-profile-goldens.json").read_bytes())
        self.assertEqual(fixture["namespace"], raw)
        self.assertEqual(fixture["isolated"], probe)
        self.assertEqual(fixture["comparison"], norm(raw))
        self.assertEqual(fixture["comparison_sha256"], digest(norm(raw)))
        self.assertEqual(fixture["canonical_utf8"].encode(), canonical_bytes(norm(raw)))
        changed = copy.deepcopy(raw)
        change = fixture["policy_change"]
        changed["argv"][change["raw_argv_index"]] = change["value"]
        self.assertEqual(digest(norm(changed)), change["comparison_sha256"])
        self.assertNotEqual(change["comparison_sha256"], fixture["comparison_sha256"])

    def test_policy_binary_and_env_differences_are_bound(self):
        raw = namespace_profile()
        for field in ("pow", "binary", "env", "ownership"):
            other = copy.deepcopy(raw)
            if field == "pow":
                other["argv"][10] = "207fffff"
            elif field == "binary":
                other["binary_sha256"] = "cd" * 32
            elif field == "env":
                other["env"]["RBITCOIN_RPC_PACKAGE_DIALECT"] = "0"
            else:
                other["env"]["SPC_LAB_RUN_TOKEN"] = "another_public_fixture_token"
            self.assertNotEqual(digest(norm(raw)), digest(norm(other)))

    def test_config_bytes_are_never_sanitized(self):
        raw = namespace_profile()
        altered = b"minrelaytxfee=0.00002000\n"
        with self.assertRaises(LaunchProfileError):
            normalize(raw, owned_endpoints=ENDPOINTS, read_bytes=lambda _: altered)
        other = copy.deepcopy(raw)
        other["config_files"][0]["sha256"] = hashlib.sha256(altered).hexdigest()
        self.assertNotEqual(digest(norm(raw)), digest(normalize(other,
            owned_endpoints=ENDPOINTS, read_bytes=lambda _: altered)))

    def test_jcs_utf16_key_order_and_safe_integer(self):
        self.assertEqual(canonical_bytes({"\ue000": 1, "\U00010000": 2}),
                         '{"\U00010000":2,"\ue000":1}'.encode())
        self.assertEqual(canonical_bytes({"n": 9007199254740991}), b'{"n":9007199254740991}')
        for value in (9007199254740992, 1.0, True, None):
            with self.assertRaises(LaunchProfileError):
                canonical_bytes({"n": value})

    def test_raw_json_refusals(self):
        for raw in ('{"x":1,"x":2}', '{"x":1.0}', '{"x":1e0}', '{"x":NaN}',
                    '{"x":Infinity}', '{"x":9007199254740992}', '{"x":"\\u0000"}',
                    '{"x":"\\ud800"}', '{"x":"\\u0085"}', '{"x":true}'):
            with self.subTest(raw=raw), self.assertRaises(LaunchProfileError):
                loads(raw)

    def test_profile_refusals(self):
        mutations = [lambda p: p.update(unknown=1), lambda p: p.pop("env"),
            lambda p: p.update(profile_version=2), lambda p: p["env"].pop("SPC_LAB_RUN_TOKEN"),
            lambda p: p["argv"].extend(["--datadir", "/other"]),
            lambda p: p["argv"].extend(["--unknown-policy", "1"]),
            lambda p: p["argv"].__setitem__(1, "mainnet"),
            lambda p: p["argv"].__setitem__(3, "/run/a/../b"),
            lambda p: p["argv"].__setitem__(22, "10.0.0.99:18444"),
            lambda p: p["argv"].__setitem__(14, "4"),
            lambda p: p["substitutions"].pop(),
            lambda p: p["substitutions"].append({"class": "datadir", "argv_index": 3}),
            lambda p: p["substitutions"][0].update(argv_index=10),
            lambda p: p["substitutions"][0].update(argv_index=True),
            lambda p: p["config_files"][0].update(argv_index=25),
            lambda p: p["config_files"].clear()]
        for mutate in mutations:
            p = namespace_profile()
            mutate(p)
            with self.subTest(profile=p), self.assertRaises(LaunchProfileError):
                norm(p)

    def test_substitution_order_not_hashed_remaining_argv_order_is(self):
        p = namespace_profile()
        p["substitutions"].reverse()
        self.assertEqual(norm(p), norm(namespace_profile()))
        p = isolated(namespace_profile())
        # Insert a policy flag with a value supported by the closed parser.
        p["argv"][0:2], p["argv"][9:11] = p["argv"][9:11], p["argv"][0:2]
        self.assertNotEqual(digest(norm(p)), digest(norm(namespace_profile())))

    def test_bad_cli_profile_has_no_effect(self):
        from arkrpc.cli import main
        with tempfile.TemporaryDirectory(prefix="traffic-profile-test-") as root:
            bad = Path(root) / "bad.json"
            bad.write_text('{"profile_version":1,"profile_version":1}')
            bad.chmod(0o600)
            out = Path(root) / "out"
            with patch("arkrpc.nodeproc.subprocess.Popen") as spawn, self.assertRaises(SystemExit):
                main(["node-probe", "--binary", "/not/a/node", "--source-name", "test", "--label", "test",
                    "--out", str(out), "--launch-profile", str(bad), "--actor-exec", "/not/a/helper",
                    "--actor-exec-sha256", "ab"*32, "--controller-pid", "42", "--controller-start-ticks", "1"])
            self.assertFalse(out.exists())
            spawn.assert_not_called()

    def test_cleanup_checks_identity_before_signal(self):
        node = object.__new__(ProfileOwnedNode)
        node.proc = Mock(pid=123, returncode=None)
        node.proc.poll.return_value = None
        node.identity_ticks = 45
        with patch("arkrpc.nodeproc.start_ticks", return_value=46), patch("arkrpc.nodeproc.os.killpg") as kill:
            with self.assertRaises(NodeError):
                node.stop()
            kill.assert_not_called()

    def test_partial_spawn_receipt_failure_reaps_owned_child(self):
        from arkrpc import nodeproc
        with tempfile.TemporaryDirectory(prefix="traffic-profile-test-") as out:
            node = object.__new__(ProfileOwnedNode)
            node.root = os.path.join(out, "node")
            node.datadir = os.path.join(node.root, "datadir")
            node.cookie_path = os.path.join(node.root, "cookie")
            node.log_path = os.path.join(node.root, "node.log")
            node.intent_path = os.path.join(out, "intent.json")
            node.observed_path = os.path.join(out, "observed.json")
            node.helper, node.helper_sha256 = "/declared/spc-lab", "ab"*32
            node.controller_pid, node.controller_ticks = 1, 2
            node.binary, node.binary_sha256 = "/declared/node", "ab"*32
            node.profile = isolated(namespace_profile())
            node.comparison_digest = digest(norm(node.profile))
            node.argv = [node.binary] + node.profile["argv"]
            node.proc, node._log_fh, node.identity_ticks = None, None, None
            child = Mock(pid=321, returncode=-15)
            child.poll.return_value = None
            calls = []
            def record(path, value):
                calls.append(Path(path).name)
                if path == node.observed_path:
                    raise OSError("injected fsync failure after spawn")
            def spawn(argv, **kw):
                calls.append("spawn")
                self.assertEqual(calls[0], "intent.json")
                self.assertEqual(argv[:2], [node.helper, "actor-exec"])
                self.assertEqual(kw["env"], node.profile["env"])
                self.assertTrue(kw["start_new_session"])
                return child
            with patch.object(nodeproc, "validate_parent_handoff"), patch.object(nodeproc, "sha256_file", return_value="ab"*32), \
                    patch.object(nodeproc, "start_ticks", return_value=4), patch.object(nodeproc, "private_json", side_effect=record), \
                    patch.object(nodeproc.subprocess, "Popen", side_effect=spawn), patch.object(nodeproc.os, "killpg") as kill, \
                    patch("arkrpc.launchprofile.Path.read_bytes", return_value=CONFIG):
                with self.assertRaises(OSError):
                    node.start()
            self.assertEqual(calls, ["intent.json", "spawn", "observed.json"])
            kill.assert_called_once_with(321, __import__("signal").SIGTERM)
            child.wait.assert_called_once_with(timeout=2.0)
            self.assertIsNone(node._log_fh)

    def test_public_metadata_omits_token_paths_and_env(self):
        node = object.__new__(ProfileOwnedNode)
        node.label, node.source_name = "fixture", "fixture-source"
        node.binary_sha256 = "ab" * 32
        node.comparison_digest = "cd" * 32
        node.raw_profile_digest = "ef" * 32
        node.helper_sha256 = "12" * 32
        node.profile = namespace_profile()
        text = json.dumps(node.describe())
        self.assertNotIn(node.profile["env"]["SPC_LAB_RUN_TOKEN"], text)
        self.assertNotIn("SPC_LAB_RUN_TOKEN", text)
        self.assertNotIn("/run/lab", text)
        self.assertNotIn('"env"', text)


if __name__ == "__main__":
    unittest.main()
