"""Frozen r24 stage/reader/collector boundaries inside the real AP5 coordinator.

Public synthetic journals and call-order hooks only; no actors or live proof.
"""
from pathlib import Path
import subprocess
import sys
import unittest


SETUP = '''
from pathlib import Path
from types import MethodType,SimpleNamespace
from unittest.mock import Mock
import os,tempfile,threading,time
from common import sha,Blocked
from source_package import package_qa
from shared_sources import load_qa
from shared_run import SharedCoordinator
from test_shared_run import Hooks,AP5Hooks
tmp=tempfile.TemporaryDirectory(prefix='ap5-r24-boundaries-')
root=Path(tmp.name).resolve()
package_qa(Path(os.environ.get('AP5_QA_TEST_SOURCE',str(Path.cwd().parents[1]))),root/'package')
descriptor=root/'package/qa-source.json'
module,profile=load_qa(descriptor,sha(descriptor),root/'source')
assert profile['api']=='t3-controller-r24'
from runtime_bootstrap import RuntimeBootstrap
from source_feed_runtime import CollectorRuntime
from test_runtime_bootstrap import fixture,finish,encoded
from source_jobs import MissingComponent
qa=Hooks();ap5=AP5Hooks(qa)
scenario,rows=fixture();phase=finish(rows)
scenario['miners']=[{'id':'m'}]
qa.scenario=scenario;qa.run_id='run';ap5.id='run'
qa.run=root/'runtime';(qa.run/'private').mkdir(parents=True,mode=0o700);qa.run.chmod(0o700)
def write(name,raw):
 p=qa.run/'private'/name;p.write_bytes(raw);p.chmod(0o600)
write('commands.jsonl',encoded(rows));write('evidence.jsonl',phase)
qa.phase_bootstrap=RuntimeBootstrap(scenario);qa.observation_lock=threading.RLock();qa.live_journal_readers={}
for name in ('require_active_finalization','bootstrap_status','journal_prefix'):
 setattr(qa,name,MethodType(getattr(module.Controller,name),qa))
manifest={'run_id':'run','traffic_stage':'active','processes':[{'role':'miner','entity':'m'}]}
qa.manifest=lambda:manifest
qa.deadline=time.monotonic()+60;qa.native_capture=Mock()
commands=[]
def command(name,payload,timeout=10):
 commands.append(payload);qa.note('command:'+payload['type'])
qa.command=command
coordinator=SharedCoordinator(qa,ap5,lambda *_:None)
def real_early_collector():
 old_start=qa.start
 def start():
  old_start();r=CollectorRuntime(qa);r.start_complete=True
  r.final_drain=Mock(wraps=r.final_drain)
  r.close=qa.close # no actual feed was started by this call-order fixture
  qa.source_feed_runtime=r
 qa.start=start
def assert_one_incomplete(result):
 r=qa.source_feed_runtime
 assert r.final_drain.call_count==1 and r.drain_attempted and r.drain_result is None
 assert qa.stops==1 and not coordinator.traffic_quiesced and commands==[]
 assert result['drain']=='inconclusive' and result['final_audit']!='final_prefix_audited'
 assert qa.results['final_sealed_prefix']['status']=='INCONCLUSIVE'
 assert [e['stage'] for e in result['errors']].count('final_traffic_quiescence')==1
 assert [e['stage'] for e in result['errors']].count('final_drain')==1
 assert all(e['class']=='MissingComponent' for e in result['errors'] if e['stage'] in {'final_traffic_quiescence','final_drain'})
 assert any(f['function']=='require_active_finalization' for e in result['errors'] for f in e['traceback_frames'])
 coordinator.finalize();coordinator.finalize()
 assert r.final_drain.call_count==1 and qa.stops==1
'''


class R24CoordinatorBoundaries(unittest.TestCase):
    def execute(self, code):
        text = SETUP + '\ntry:\n' + '\n'.join(' ' + line for line in code.splitlines())
        text += '\nfinally:\n qa.temporary.cleanup();tmp.cleanup()\n'
        result = subprocess.run([sys.executable, '-B', '-W', 'error::ResourceWarning', '-c', text],
                                cwd=Path(__file__).parent, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stdout.decode() + result.stderr.decode())

    def test_bootstrapping_blocks_pause_and_actual_collector_remove_once_even_reentrant(self):
        self.execute('''
manifest['traffic_stage']='bootstrapping'
qa.fail='observe-original-bootstrap';real_early_collector()
reentered=[];add=qa.capture.add
def capture(kind,value):
 result=add(kind,value)
 if kind=='economic_final_drain_started':reentered.append(coordinator.finalize())
 return result
qa.capture.add=capture
result=coordinator.run();assert_one_incomplete(result)
assert reentered==[{'status':'finalization_in_progress'}]
qa.native_capture.expect_miner_stop.assert_not_called()
assert qa.live_journal_readers=={} # stage rejection precedes even bootstrap reads
''')

    def test_active_manifest_without_original_phase_blocks_both_control_boundaries(self):
        self.execute('''
write('evidence.jsonl',b'')
qa.fail='observe-original-bootstrap';real_early_collector()
result=coordinator.run();assert_one_incomplete(result)
assert set(qa.live_journal_readers)=={'commands.jsonl','evidence.jsonl'}
qa.native_capture.expect_miner_stop.assert_not_called()
''')

    def test_active_manifest_with_cancelled_original_phase_is_not_success(self):
        self.execute('''
_,cancelled=fixture();phase=finish(cancelled,'cancelled')
write('commands.jsonl',encoded(cancelled));write('evidence.jsonl',phase)
qa.fail='observe-original-bootstrap';real_early_collector()
result=coordinator.run();assert_one_incomplete(result)
qa.native_capture.expect_miner_stop.assert_not_called()
''')

    def test_active_original_success_uses_real_reader_guard_and_miner_stop_before_full_order(self):
        self.execute('''
real_collector=CollectorRuntime(qa);real_collector.start_complete=True
qa.until=lambda callback,*a,**kw:[{'status':'gone'}] # no process is launched
real_collector.quiesce_feed=lambda:qa.note('feed-quiesced')
old_drain=qa.drain
def drain():
 real_collector._stop_mining()
 return old_drain()
qa.drain=drain
seen=[];qa.reenter=lambda:seen.append(coordinator.finalize())
result=coordinator.run();coordinator.finalize()
assert result['execution']=='observations_finished' and result['final_audit']=='final_prefix_audited'
assert coordinator.traffic_quiesced and (qa.drains,qa.stops)==(1,1)
assert [c['type'] for c in commands]==['pause_tx_driver','remove_miner']
names=[n for n,_ in qa.events]
ordered=['qa-audit','original_runtime_bootstrap_observation','command:pause_tx_driver']
assert names.index('qa:original_runtime_bootstrap_observation')<names.index('command:pause_tx_driver')
ordered=['command:pause_tx_driver','command:remove_miner','feed-quiesced','drain-enter','exact-checkpoints',
 'economic-convergence','pool-writer-stop','source-EOF','final-raw-SQL','qa-final-audit','capture-close','ap5-owned-teardown']
assert [names.index(n) for n in ordered]==sorted(names.index(n) for n in ordered)
assert sum(n=='qa:original_runtime_bootstrap_observation' for n in names)==2
assert seen==[{'status':'finalization_in_progress'}]
qa.native_capture.expect_miner_stop.assert_called_once_with('m')
''')

    def test_guard_precedes_even_driver_list_access(self):
        self.execute('''
manifest['traffic_stage']='bootstrapping'
qa.scenario={} # accessing traffic before the guard would raise KeyError
try:coordinator.quiesce_final_traffic()
except MissingComponent:pass
else:raise AssertionError('inactive gate bypassed')
assert not coordinator.traffic_quiesced and commands==[]
''')
