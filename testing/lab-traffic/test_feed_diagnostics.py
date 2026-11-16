"""Retain bounded code locations for source-feed failures without error values."""
import json
import os
import socket
from types import SimpleNamespace
import unittest
from unittest.mock import Mock,patch

from oracle import Invalid
from source_feed_server import FeedServer
from storage import ProcFS


class FeedDiagnosticsTests(unittest.TestCase):
    def server(self):
        s=object.__new__(FeedServer);s.agents={'alice':{'pid':os.getpid(),'start_time_ticks':ProcFS().identity(os.getpid())['start_time_ticks']}}
        s.journal=SimpleNamespace(poisoned=False);s.observe=Mock();return s

    def test_provider_failure_retains_stage_and_frames_not_secret_message_or_locals(self):
        s=self.server();s.proof=Mock(side_effect=Invalid('private-password-that-must-not-be-recorded'))
        a,b=socket.socketpair();self.addCleanup(b.close)
        frame={'kind':'job','attempt':[1]*16}
        with patch('source_feed_server.receive',return_value=frame),patch('source_feed_server.send'):
            result=s.exchange(a,'alice')
        self.assertEqual(a.fileno(),-1);self.assertEqual(result['status'],'INCONCLUSIVE')
        self.assertEqual(result['stage'],'prepare_proof');self.assertEqual(result['reason'],'Invalid')
        self.assertTrue(result['traceback_frames']);self.assertLessEqual(len(result['traceback_frames']),16)
        self.assertNotIn('private-password',json.dumps(result))
        self.assertTrue(all(set(f)=={'file','function','line'} and '/' not in f['file'] for f in result['traceback_frames']))
        s.observe.assert_called_once_with('source_feed_exchange_result',result)

    def test_success_reports_consumption_scope_without_changing_wire_calls(self):
        s=self.server();s.proof=Mock(return_value={'proof':'unit'});s.consume=Mock(return_value={'committed':'unit'})
        a,b=socket.socketpair();self.addCleanup(b.close);frame={'kind':'job','attempt':[2]*16}
        with patch('source_feed_server.receive',side_effect=[frame,{'ack':{'attempt':[2]*16}}]),patch('source_feed_server.send') as send:
            result=s.exchange(a,'alice')
        self.assertEqual(result['status'],'COMMITTED_CONSUMPTION_ONLY');self.assertEqual(send.call_count,2)
        self.assertNotIn('traceback_frames',result);self.assertEqual(result['attempt'],'02'*16)

    def test_diagnostic_sink_failure_poisoning_cannot_silently_lose_observations(self):
        s=self.server();s.observe.side_effect=OSError('unavailable')
        with self.assertRaises(OSError):s.record_exchange('source_feed_exchange_result',{'status':'INCONCLUSIVE'})
        self.assertTrue(s.journal.poisoned)


if __name__=='__main__':unittest.main()
