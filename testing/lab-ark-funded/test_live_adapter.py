import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from ap5 import Controller
from common import Blocked, Evidence, encoded, probe_invocation
from scenario import scenario


class LiveAdapter(unittest.TestCase):
    def test_real_pinned_probe_uses_native_interpreter_and_prefix_script(self):
        wrapper = Path("/nix/store/c5gh85rs220kc6g0yg9v2yk2nqd7szkx-spc-ark-canonical-probe-r4/bin/ark-rpc-probe")
        program, script = probe_invocation(wrapper)
        self.assertEqual(script, wrapper.parent.parent / "share/ark-rpc/probe.py")
        with program.open("rb") as f:
            self.assertEqual(f.read(4), b"\x7fELF")

    def test_async_bootstrap_posts_once_then_reads_same_id_journal(self):
        with tempfile.TemporaryDirectory(prefix="ap5-async-", dir="/tmp/opencode") as tmp:
            root = Path(tmp)
            ev = Evidence(root)
            c = Controller({}, {}, root, ev, scenario("unit", 1), 10)
            c.run.mkdir()
            c.url, c.token = "http://127.0.0.1:1", "unit-token"
            journal = c.run / "commands.jsonl"
            c.manifest = lambda: {"artifact_paths": {"private_commands": str(journal)}}
            def response(url, payload, **kwargs):
                outcome = {"command_id": payload["command_id"], "state": "succeeded", "detail": "unit-only durable completion"}
                journal.write_bytes(encoded({"record": "begin", "command": payload}) + b"\n" +
                                    encoded({"record": "finish", "command_id": payload["command_id"], "outcome": outcome}) + b"\n")
                return {"command_id": payload["command_id"], "state": "running"}
            try:
                with patch("ap5.request", side_effect=response) as post:
                    result = c.command("native-bootstrap", {"type": "bootstrap_fund", "driver_id": "txd-0", "node_id": "node-a"})
                    self.assertEqual(result["state"], "succeeded")
                    self.assertEqual(post.call_count, 1)
            finally:
                ev.close()
            self.assertEqual(c.spec["phases"][0]["commands"][0]["kind"]["type"], "bootstrap_fund")

    def test_journal_does_not_rebind_original_command_or_accept_partial_terminal(self):
        with tempfile.TemporaryDirectory(prefix="ap5-journal-", dir="/tmp/opencode") as tmp:
            root = Path(tmp)
            ev = Evidence(root)
            c = Controller({}, {}, root, ev, scenario("unit", 1), 10)
            c.run.mkdir()
            path = c.run / "commands.jsonl"
            c.manifest = lambda: {"artifact_paths": {"private_commands": str(path)}}
            payload = {"command_id": "same-id", "kind": {"type": "bootstrap_fund"}}
            try:
                path.write_bytes(encoded({"record": "begin", "command": payload}) + b"\n" + b'{"record":"finish"')
                self.assertIsNone(c.command_terminal(payload))
                changed = {**payload, "kind": {"type": "other"}}
                path.write_bytes(encoded({"record": "begin", "command": changed}) + b"\n")
                with self.assertRaisesRegex(Blocked, "payload changed"):
                    c.command_terminal(payload)
            finally:
                ev.close()
