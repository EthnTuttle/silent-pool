"""No active-only control or final-boundary claim on incomplete bootstrap."""
from pathlib import Path
import tempfile
import time
from types import SimpleNamespace
import unittest
from unittest.mock import Mock

from campaign import load_controller
from oracle import Invalid
from source_jobs import MissingComponent
from source_feed_runtime import CollectorRuntime


class FinalizationStageTests(unittest.TestCase):
    def controller(self,stage='bootstrapping',state='observed_in_flight'):
        c=object.__new__(load_controller().Controller);c.run_id='run'
        c.manifest=Mock(return_value={'run_id':'run','traffic_stage':stage,'processes':[]})
        c.capture=SimpleNamespace(add=Mock());c.bootstrap_status=Mock(return_value={'state':state,'operation_id':'original'})
        c.command=Mock();c.native_capture=Mock();c.scenario={'miners':[]};c.deadline=time.monotonic()+60
        return c

    def test_pre_quiescence_gate_rejects_bootstrap_without_command_or_fake_terminal(self):
        c=self.controller()
        with self.assertRaisesRegex(MissingComponent,'traffic_not_active'):c.require_active_finalization()
        c.command.assert_not_called();c.bootstrap_status.assert_not_called()

    def test_active_stage_requires_real_original_success_and_phase(self):
        c=self.controller('active')
        with self.assertRaisesRegex(MissingComponent,'original_bootstrap_incomplete'):c.require_active_finalization()
        c.bootstrap_status.assert_called_once_with(require_phase=True);c.command.assert_not_called()
        c.bootstrap_status.return_value={'state':'succeeded','operation_id':'original'}
        self.assertEqual(c.require_active_finalization()['operation_id'],'original')
        c.manifest.return_value['run_id']='foreign'
        with self.assertRaises(Invalid):c.require_active_finalization()

    def test_unstarted_and_bootstrap_collectors_keep_one_inconclusive_drain(self):
        for started,stage,reason in ((False,'active','collector_start_incomplete'),(True,'bootstrapping','traffic_not_active')):
            with self.subTest(started=started,stage=stage):
                c=self.controller(stage);r=CollectorRuntime(c);r.start_complete=started
                r._drain_sample=Mock();r._stop_pools=Mock()
                with self.assertRaisesRegex(MissingComponent,reason):r.final_drain()
                self.assertTrue(r.drain_attempted);self.assertIsNone(r.drain_result);self.assertIsNone(r.final_boundary_ref)
                with self.assertRaisesRegex(MissingComponent,'cannot_restart'):r.final_drain()
                c.command.assert_not_called();c.native_capture.expect_miner_stop.assert_not_called()
                r._drain_sample.assert_not_called();r._stop_pools.assert_not_called()

    def test_completed_bootstrap_uses_existing_single_mining_stop_path(self):
        c=self.controller('active','succeeded');c.scenario={'miners':[{'id':'m'}]}
        c.manifest.return_value['processes']=[{'role':'miner','entity':'m'}]
        c.until=Mock(return_value=[{'status':'gone'}]);r=CollectorRuntime(c);r.start_complete=True;r.quiesce_feed=Mock()
        r._stop_mining()
        c.bootstrap_status.assert_called_once_with(require_phase=True)
        c.command.assert_called_once_with('drain-remove-m',{'type':'remove_miner','miner_id':'m'},timeout=10)
        c.native_capture.expect_miner_stop.assert_called_once_with('m');r.quiesce_feed.assert_called_once()


if __name__=='__main__':unittest.main()
