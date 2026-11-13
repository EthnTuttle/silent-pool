"""Controller fail-closed boundary checks, with no production actors spawned."""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

HERE = Path(__file__).resolve().parent
TEMP_BASE = None


class ControllerBoundaryTests(unittest.TestCase):
    def controller_class(self):
        spec = importlib.util.spec_from_file_location("acceptance_controller", HERE / "regtest-traffic-e2e.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module.Controller

    def test_constructor_bundle_reaches_prerequisites_without_mutating_inputs(self):
        import hashlib
        with tempfile.TemporaryDirectory(prefix='r22-kwargs-') as folder:
            root=Path(folder);exe=root/'sealed-unit';exe.write_bytes(b'not executed');exe.chmod(0o700)
            manifest=root/'manifest.json';manifest.write_text('{}')
            digest=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
            binding=json.loads((HERE/'inputs.template.json').read_bytes())
            names=('lab','pool','miner','txd','node','scanner','traffic-probe','redis-server','redis-cli','initdb','postgres','pg_ctl','psql','createdb')
            binding['binaries']={n:{'path':str(exe),'sha256':digest(exe),'source_manifest':str(manifest),'source_manifest_sha256':digest(manifest)} for n in names}
            binding['source_snapshot_sha256']='00'*32;binding['probe_script']={'path':str(exe),'sha256':digest(exe)}
            fallback={'path':'input-envelope','sha256':'11'*32};selected={'path':'selected-envelope','sha256':'22'*32}
            binding['runtime_bundle']=fallback;inputs=root/'inputs.json';inputs.write_text(json.dumps(binding));before=inputs.read_bytes()
            for index,keyword in enumerate((selected,None,{})):
                args=SimpleNamespace(run_root=str(root/str(index)),timeout=10,inputs=str(inputs),scenario=str(HERE/'scenario.json'),case='funded')
                controller=self.controller_class()(args,runtime_bundle=keyword)
                try:
                    with patch('runtime_bootstrap.source_admission',return_value={'scope':'unit-only'}) as admit,patch('native_capture.require_audited_sources'),patch('base_network.reject_known_host_network_failure'):
                        controller.prerequisites()
                        admit.assert_called_once_with(binding['binaries']['lab'],bundle=fallback if keyword is None else keyword)
                    self.assertIsNone(controller.process);self.assertFalse(controller.run.exists());self.assertEqual(inputs.read_bytes(),before)
                finally:controller.http_workers.shutdown(wait=True,cancel_futures=True);controller.capture.stream.close()

    def test_cleanup_cannot_stop_runtime_while_final_drain_owns_boundary(self):
        from oracle import Inconclusive
        cls=self.controller_class();controller=object.__new__(cls)
        lock=threading.Lock();controller.source_feed_runtime=SimpleNamespace(finalize_lock=lock,_close=Mock())
        controller._cleanup_owned=Mock()
        with lock,self.assertRaisesRegex(Inconclusive,"no_controller_teardown"):controller.cleanup()
        controller._cleanup_owned.assert_not_called()

    def test_cleanup_holds_same_boundary_lock_through_controller_teardown(self):
        cls=self.controller_class();controller=object.__new__(cls)
        lock=threading.Lock();close=Mock();controller.source_feed_runtime=SimpleNamespace(finalize_lock=lock,_close=close)
        def cleanup(callback):
            self.assertIs(callback,close);self.assertFalse(lock.acquire(blocking=False));callback()
            raise OSError("reaper unavailable")
        controller._cleanup_owned=cleanup
        with self.assertRaisesRegex(OSError,"reaper unavailable"):controller.cleanup()
        close.assert_called_once();self.assertTrue(lock.acquire(blocking=False));lock.release()

    def test_missing_seals_yield_inconclusive_receipt(self):
        with tempfile.TemporaryDirectory(prefix="t3-controller-vector-", dir=TEMP_BASE) as root:
            run = Path(root) / "fresh"
            result = subprocess.run([sys.executable, "-B", str(HERE / "regtest-traffic-e2e.py"),
                                     "--inputs", str(HERE / "inputs.template.json"),
                                     "--scenario", str(HERE / "scenario.json"),
                                     "--run-root", str(run)], capture_output=True, timeout=10)
            self.assertEqual(result.returncode, 2, result.stderr.decode())
            receipt = json.loads((run / "acceptance.json").read_bytes())
            self.assertEqual(receipt["status"], "INCONCLUSIVE")
            self.assertFalse((run / "runtime").exists())
            self.assertFalse(receipt["full_ark_acceptance"])
            self.assertEqual((run.stat().st_mode & 0o777), 0o700)

    def test_existing_run_is_never_adopted(self):
        with tempfile.TemporaryDirectory(prefix="t3-existing-vector-", dir=TEMP_BASE) as root:
            marker = Path(root) / "user-data"
            marker.write_bytes(b"preserve")
            result = subprocess.run([sys.executable, "-B", str(HERE / "regtest-traffic-e2e.py"),
                                     "--inputs", str(HERE / "inputs.template.json"),
                                     "--scenario", str(HERE / "scenario.json"),
                                     "--run-root", root], capture_output=True, timeout=10)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(marker.read_bytes(), b"preserve")
            self.assertFalse((Path(root) / "private").exists())

    def test_rpc_generation_is_not_an_allowed_tool(self):
        spec = importlib.util.spec_from_file_location("acceptance_controller", HERE / "regtest-traffic-e2e.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        permitted = module.Rpc.READS | module.Rpc.TEST_WRITES
        self.assertNotIn("submitblock", permitted)
        self.assertFalse(any(m.startswith("generate") or m == "setmocktime" for m in permitted))


if __name__ == "__main__":
    unittest.main()
