"""Portable final API regression cases, now run against frozen r24 (not live proof)."""
from pathlib import Path
import subprocess
import sys
import unittest


SETUP = '''
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock,patch
import copy,hashlib,json,os,tempfile,threading
from shared_sources import load_qa
from shared_run import SharedCoordinator,coordinated_qa_class,checked_final_audit
from common import sha,Blocked
from source_package import package_qa
source_tmp=tempfile.TemporaryDirectory(prefix='ap5-r21-code-')
reference=Path(os.environ.get('AP5_QA_TEST_SOURCE',str(Path.cwd().parents[1])))
package_qa(reference,Path(source_tmp.name)/'package')
descriptor=Path(source_tmp.name)/'package/qa-source.json'
module,_=load_qa(descriptor,sha(descriptor),Path(source_tmp.name)/'source')
assert coordinated_qa_class(module) is module.Controller
from test_final_audit import FinalPrefixTests
from case_contract import canonical_json
from oracle import Invalid,Inconclusive
fixture=FinalPrefixTests();fixture.setUp()
qa=object.__new__(module.Controller);qa.__dict__.update(vars(fixture.c))
qa.results={};qa.process=None;qa.source_feed_runtime=fixture.runtime
fixture.runtime.controller=qa
qa.http_workers=SimpleNamespace(shutdown=lambda **kw:None)
fixture.boundary.update(checkpoint_coverage={'status':'COMPLETE_CAPTURE'},unbooked_liabilities_preserved=True,
 scope='controlled synthetic final-stage test, not live acceptance')
def seal():
 fixture.seal();fixture.runtime.final_boundary_ref['coverage_status']='COMPLETE_CAPTURE'
seal()
events=[]
ap5=SimpleNamespace(child=None,id=qa.run_id,ev=SimpleNamespace(save=lambda name,value:events.append((name,value))),cleanup={'status':'passed'})
def stop():events.append(('AP5_STOP',None))
ap5.stop=stop
coordinator=SharedCoordinator(qa,ap5,lambda *_:None)
coordinator.traffic_quiesced=True # final-stage fixture: earlier runtime work is not simulated as proof
def finish():
 counts=[n.calls for n in fixture.nodes]
 with patch.object(fixture.runtime,'final_drain',wraps=fixture.runtime.final_drain) as drain,patch.object(qa,'final_audit_after_drain',wraps=qa.final_audit_after_drain) as audit,patch.object(fixture.runtime,'close',wraps=fixture.runtime.close) as close:
  result=coordinator.finalize();coordinator.finalize()
  assert drain.call_count==audit.call_count==close.call_count==1
 assert sum(n=='AP5_STOP' for n,_ in events)==1
 assert counts==[n.calls for n in fixture.nodes], 'final API performed fresh node reads'
 return result
'''


class R21FinalExports(unittest.TestCase):
    def execute(self, code):
        text = SETUP + '\ntry:\n' + '\n'.join(' '+line for line in code.splitlines()) + '\nfinally:\n fixture.doCleanups();source_tmp.cleanup()\n'
        result = subprocess.run([sys.executable, '-B', '-W', 'error::ResourceWarning', '-c', text],
                                cwd=Path(__file__).parent, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stdout.decode() + result.stderr.decode())

    def test_actual_final_export_preserves_inconclusive_and_one_cleanup_without_RPC(self):
        self.execute('''
qa.results['payout_ladder']={'status':'FAILED','detail':'earlier phase contradiction'}
result=finish()
assert result['final_audit']=='failed'
assert qa.results['payout_ladder']['status']=='FAILED'
assert qa.results['final_sealed_prefix']['status']=='FAILED'
''')

    def test_late_invalid_economic_source_included_in_final_seal_fails_actual_API_and_cleans(self):
        self.execute('''
from test_audited_window import encode_events
raw=encode_events(fixture.source_events+[('unknown_late_event',{'front_id':0})])
for path in (qa.source_capture.expected['front'],qa.source_capture.output/'front.log'):fixture.write(path,raw)
fixture.boundary['source_EOF']['front']['digest']=hashlib.sha256(raw).hexdigest();seal()
qa.results['payout_ladder']={'status':'PASSED','detail':'old prefix cannot cover appended event'}
result=finish()
assert result['final_audit']=='inconclusive'
assert qa.results['final_sealed_prefix']['status'] in {'FAILED','INCONCLUSIVE'}
assert qa.results['payout_ladder']['status']!='PASSED'
''')

    def test_late_invalid_native_event_blocks_actual_native_final_API_and_cleans(self):
        self.execute('''
from native_capture import NativeCapture
from native_observer import controller_events
from native_retention import verify_exchange
from test_native_retention import inputs
from test_native_audit import fixture as native_fixture
raw,values=inputs();_,_,bindings,collectors,native_events=native_fixture(9)
native_events[0]['occurred_at_wall_ms']=999
native_events[1]['event']['payload']['durable_evidence']=values['handle.json']
rows=[]
for e in native_events:
 rows.append({'run_id':'run','event_id':e['event_id'],'source':'miner-miner','source_cursor':'token:55:'+str(e['event_id']),
  'actor':{'kind':'miner','id':'miner'},'command_id':None,'occurred_at_wall':e['occurred_at_wall_ms'],
  'occurred_at_mono':e['occurred_at_mono_ms']*1000000,'ingested_at':2000,
  'payload':{'kind':'miner_event','data':{'miner_id':'miner','incarnation':'token:55','driver_event_id':e['event_id'],
   'name':e['event']['name'],'payload':e['event']['payload'],'dropped_before':0}}})
prefix=b''.join(canonical_json(row)+b'\\n' for row in rows)
out=qa.priv/'native-retained';out.mkdir(mode=0o700)
for path in (qa.run/'private/evidence.jsonl',out/'prefix.json'):fixture.write(path,prefix)
capture=object.__new__(NativeCapture);capture.lock=threading.RLock();capture.c=qa;capture.out=out
capture.incarnations={'miner':'token:55'};capture.bindings=bindings;capture.processes={'miner':bindings[('miner','connection')]['observed_miner']};capture.files={}
exchange=verify_exchange(raw['record.json'],values['manifest.json'],raw['manifest.json'],values['handle.json']['request_digest']+'.json',bindings[('miner','connection')])
reader=Mock();reader.manifest_raw=raw['manifest.json'];reader.records={values['handle.json']['request_digest']+'.json':raw['record.json']};reader.pending=[]
reader.outer.return_value={values['handle.json']['request_digest']:exchange};capture.retention_readers={'miner':reader}
bound=[{'miner_id':m,'connection_id':con,'binding':v} for (m,con),v in sorted(bindings.items())]
H=lambda raw:hashlib.sha256(raw).hexdigest()
native_boundary={'schema':'spc-lab-native-final-prefix/1','run_id':'run',
 'controller_prefix':{'source_path':str(qa.run/'private/evidence.jsonl'),'path':str(out/'prefix.json'),'sha256':H(prefix)},
 'miner_events_sha256':H(canonical_json(controller_events(prefix,'run',capture.incarnations))),
 'stores':{'miner':{'manifest_sha256':H(reader.manifest_raw),'records':{k:H(v) for k,v in reader.records.items()}}},
 'bindings_sha256':H(canonical_json(bound)),'incarnations':capture.incarnations}
qa.native_capture=capture;fixture.boundary['native_boundary']=native_boundary;seal()
with patch('native_capture.collector_entries',return_value=collectors):
 assert capture.audit_after_drain([],native_boundary)['status']=='PASSED' # signed SYNTHETIC original prefix only
 late=copy.deepcopy(rows[-1]);late.update(event_id=5,source_cursor='token:55:5');late['payload']['data']['driver_event_id']=5
 late['payload']['data']['payload']['candidate_epoch']=999
 with (qa.run/'private/evidence.jsonl').open('ab') as stream:stream.write(canonical_json(late)+b'\\n')
 result=finish()
assert result['final_audit']=='inconclusive'
assert qa.results['premining_proof_feed']['status']=='INCONCLUSIVE'
assert 'late_native_event' in str(qa.results['premining_proof_feed'])
''')

    def test_actual_audit_live_never_calls_drain_or_final_audit(self):
        self.execute('''
with patch.object(qa,'audit') as audit:
 qa.audit_live()
 audit.assert_called_once_with(finalize=False)
assert fixture.runtime.drain_result is not None # prebuilt unit boundary, no new drain executed
''')

    def test_old_boundary_schema_or_changed_reference_rejected_by_coordinator(self):
        self.execute('''
report=qa.final_audit_after_drain()
assert checked_final_audit(qa,report,fixture.runtime.drain_result)=='INCONCLUSIVE'
fixture.boundary['schema']='spc-lab-owned-economic-drain/1';seal()
report['boundary']=fixture.runtime.final_boundary_ref
try:checked_final_audit(qa,report,fixture.runtime.drain_result)
except Blocked:pass
else:raise AssertionError('old boundary promoted')
''')
