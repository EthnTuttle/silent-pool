"""Actual frozen pause proof helper through AP5's single finalizer; no actors."""
from pathlib import Path
import subprocess
import sys
import unittest


SETUP = '''
from pathlib import Path
import os,tempfile
from common import sha
from source_package import package_qa
from shared_sources import load_qa
from shared_run import SharedCoordinator
from test_shared_run import Hooks,AP5Hooks
tmp=tempfile.TemporaryDirectory(prefix='ap5-r25-proof-')
root=Path(tmp.name).resolve()
package_qa(Path(os.environ.get('AP5_QA_TEST_SOURCE',str(Path.cwd().parents[1]))),root/'package')
p=root/'package/qa-source.json'
module,profile=load_qa(p,sha(p),root/'source')
assert profile['api']=='t3-controller-r25'
from test_measured_pause import MeasuredPauseTests
from oracle import Inconclusive
fixture=MeasuredPauseTests();fixture.setUp()
qa=Hooks();qa.run_id='run';qa.fail='qa-measured'
qa.acknowledged_traffic_pause=fixture.c.acknowledged_traffic_pause
qa.command=fixture.command
ap5=AP5Hooks(qa);ap5.id='run'
coordinator=SharedCoordinator(qa,ap5,lambda *_:None)
def pause0():fixture.command('measured-existing',{'type':'pause_tx_driver','driver_id':'txd-0'})
def finish():
 result=coordinator.run();coordinator.finalize()
 assert (qa.drains,qa.stops)==(1,1)
 return result
def proofs():return [v for k,v in qa.events if k=='ap5:shared-final-traffic-pause-proof']
'''


class R25AcknowledgedPause(unittest.TestCase):
    def execute(self, body):
        code = SETUP + '\ntry:\n' + '\n'.join(' ' + line for line in body.splitlines())
        code += '\nfinally:\n fixture.doCleanups();qa.temporary.cleanup();tmp.cleanup()\n'
        result = subprocess.run([sys.executable, '-B', '-W', 'error::ResourceWarning', '-c', code],
                                cwd=Path(__file__).parent, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stdout.decode() + result.stderr.decode())

    def test_reuses_original_txd0_ack_and_only_pauses_running_txd1(self):
        self.execute('''
pause0();old=fixture.c.acknowledged_traffic_pause('txd-0')
seen=[];qa.reenter=lambda:seen.append(coordinator.finalize())
finish()
assert [(kind,d) for kind,d,_ in fixture.calls]==[('pause_tx_driver','txd-0'),('pause_tx_driver','txd-1')]
assert coordinator.traffic_quiesced
p=proofs();assert len(p)==2 and p[0]['reused'] is True and p[1]['reused'] is False
assert p[0]['proof']['command_id']==old['command_id']
assert p[0]['proof']['incarnation']=='inc-txd-0'
assert p[0]['proof']['journal']['barrier_seq']==1
assert seen==[{'status':'finalization_in_progress'}]
''')

    def test_unknown_resume_invalidates_reuse_and_requires_one_new_ack(self):
        self.execute('''
pause0();fixture.resume_failure='txd-0'
try:fixture.command('unknown-resume',{'type':'resume_tx_driver','driver_id':'txd-0'})
except Inconclusive:pass
assert fixture.c.acknowledged_traffic_pause('txd-0') is None
finish();assert coordinator.traffic_quiesced
assert len(fixture.calls)==4
assert all(p['reused'] is False for p in proofs())
assert proofs()[0]['proof']['command_id']=='cid-shared-final-pause-txd-0'
''')

    def test_new_incarnation_cannot_reuse_old_barrier(self):
        self.execute('''
pause0();fixture.incarnations['txd-0']='replacement-inc';fixture.session('txd-0')
assert fixture.c.acknowledged_traffic_pause('txd-0') is None
finish();assert coordinator.traffic_quiesced
assert len(fixture.calls)==3
assert proofs()[0]['proof']['incarnation']=='replacement-inc'
assert proofs()[0]['reused'] is False
''')

    def test_post_barrier_send_raises_without_repause_but_other_driver_is_quiesced(self):
        self.execute('''
pause0();fixture.send('txd-0')
result=finish()
assert [(kind,d) for kind,d,_ in fixture.calls]==[('pause_tx_driver','txd-0'),('pause_tx_driver','txd-1')]
assert not coordinator.traffic_quiesced
assert result['final_audit']=='inconclusive'
assert qa.results['final_sealed_prefix']['status']=='INCONCLUSIVE'
assert len(proofs())==1 and proofs()[0]['driver_id']=='txd-1'
error=next(e for e in result['errors'] if e['stage']=='final_pause:txd-0')
assert error['class']=='Invalid'
assert any(f['function']=='acknowledged_traffic_pause' for f in error['traceback_frames'])
''')

    def test_command_success_without_new_ack_cannot_establish_quiescence(self):
        self.execute('''
old=qa.command;attempted=[]
def command(label,kind,**kw):
 attempted.append(kind['driver_id'])
 if kind['driver_id']=='txd-0':return {'state':'succeeded'} # no retained ACK/barrier
 return old(label,kind,**kw)
qa.command=command
result=finish()
assert attempted==['txd-0','txd-1'] and not coordinator.traffic_quiesced
assert result['final_audit']=='inconclusive'
assert len(proofs())==1 and proofs()[0]['driver_id']=='txd-1'
''')

    def test_phase_boolean_does_not_skip_proof_or_running_driver(self):
        self.execute('''
pause0();coordinator.live_audit_finished=True
finish()
assert coordinator.traffic_quiesced and len(proofs())==2
assert [(kind,d) for kind,d,_ in fixture.calls]==[('pause_tx_driver','txd-0'),('pause_tx_driver','txd-1')]
''')
