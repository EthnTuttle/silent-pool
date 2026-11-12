"""Existing-controller export constraints, without another runtime/coordinator."""
import importlib.util
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock

from oracle import Invalid
from source_feed_runtime import collector_socket_path

HERE=Path(__file__).resolve().parent


class SharedExportTests(unittest.TestCase):
    def module(self):
        spec=importlib.util.spec_from_file_location("qa_export_test",HERE/"regtest-traffic-e2e.py")
        module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module);return module

    def test_operator_flag_explicit_and_host_API_only(self):
        module=self.module();scenario={"fronts":[{"roles":["front","api"]}]}
        self.assertEqual(module.operator_arguments(False,{},scenario),[])
        self.assertEqual(module.operator_arguments(True,{},scenario),["--operator-ui"])
        for enabled,binding,spec in ((1,{},scenario),(True,{"topology":True},scenario),(True,{}, {"fronts":[{"roles":["front"]}]})):
            with self.assertRaises(Invalid):module.operator_arguments(enabled,binding,spec)

    def test_collector_uses_renderer_runtime_private_not_QA_sibling(self):
        with tempfile.TemporaryDirectory(prefix="t3-export-") as tmp:
            root=Path(tmp);private=root/"runtime"/"private";private.mkdir(mode=0o700,parents=True)
            path=private/"collector-alice.sock"
            self.assertEqual(collector_socket_path(str(path),private),path)
            qa=root/"private";qa.mkdir(mode=0o700)
            with self.assertRaises(Invalid):collector_socket_path(str(qa/"collector.sock"),private)

    def test_collector_rejects_symlinks_public_parent_and_occupied_endpoint(self):
        with tempfile.TemporaryDirectory(prefix="t3-export-") as tmp:
            root=Path(tmp);private=root/"private";private.mkdir(mode=0o700)
            public=private/"public";public.mkdir(mode=0o755)
            with self.assertRaises(Invalid):collector_socket_path(str(public/"collector.sock"),private)
            link=private/"link";link.symlink_to(public,target_is_directory=True)
            with self.assertRaises(Invalid):collector_socket_path(str(link/"collector.sock"),private)
            path=private/"collector.sock";path.write_bytes(b"existing-runtime-data")
            with self.assertRaises(Invalid):collector_socket_path(str(path),private)
            self.assertEqual(path.read_bytes(),b"existing-runtime-data")

    def test_controller_final_audit_dispatches_actual_retained_adapter(self):
        from final_audit import FINAL_GATES
        module=self.module();c=object.__new__(module.Controller);c.results={};c.capture=Mock();c.source_feed_runtime=Mock();c.args=SimpleNamespace(witness=False)
        c.source_feed_runtime.final_audit_after_drain.return_value={"status":"PASSED","checks":{name:{"status":"PASSED","detail":{"actual":"adapter-result"}} for name in FINAL_GATES}}
        result=c.final_audit_after_drain()
        c.source_feed_runtime.final_audit_after_drain.assert_called_once()
        self.assertEqual(c.results['payout_ladder']['detail'],{"actual":"adapter-result"})
        self.assertEqual(result['status'],'PASSED')


if __name__=="__main__":unittest.main()
