"""Actual frozen r19 exports with controlled call hooks; never live acceptance."""
from pathlib import Path
import subprocess
import sys
import unittest


PRELUDE = '''
from pathlib import Path
from types import SimpleNamespace
import json,tempfile
from unittest.mock import Mock, patch
from common import sha, Blocked, Evidence, same_json_value
from shared_sources import load_qa
from shared_run import coordinated_qa_class, qa_arguments, validate_shared_binding
from shared_run import shared_exit_status
from ap5 import Controller as AP5Controller
from scenario import scenario
temporary=tempfile.TemporaryDirectory(prefix='ap5-r19-exports-',dir='/tmp/opencode')
root=Path(temporary.name)
descriptor=Path('qa-source-r19.json')
assert sha(descriptor)=='ea4436e72deae799b4d8fa97fb73d5e4ef732b041d581b7b602d2db4df06b9d4'
module,value=load_qa(descriptor,sha(descriptor),root/'source')
from oracle import Invalid
from source_feed_runtime import collector_socket_path
'''


class R19Exports(unittest.TestCase):
    def run_bound(self, code):
        result = subprocess.run([sys.executable, '-B', '-W', 'error::ResourceWarning', '-c',
                                 PRELUDE + '\n' + code + '\ntemporary.cleanup()\n'],
                                cwd=Path(__file__).parent, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stdout.decode() + result.stderr.decode())

    def test_actual_operator_and_collector_root_positive_and_mismatches(self):
        self.run_bound('''
args=qa_arguments(root,SimpleNamespace(qa_inputs=root/'inputs.json',scenario=root/'scenario.json',timeout_secs=7200))
spec=scenario('unit',1)
assert module.operator_arguments(args.operator_ui,{},spec)==['--operator-ui']
assert args.allow_reorg is True and args.witness is True
for enabled,binding,s in [(1,{},spec),(True,{'topology':True},spec),(True,{}, {'fronts':[{'roles':['front']} ]})]:
 try:module.operator_arguments(enabled,binding,s)
 except Invalid:pass
 else:raise AssertionError('operator scope mismatch accepted')
runtime_private=root/'qa/runtime/private';runtime_private.mkdir(parents=True,mode=0o700)
socket=runtime_private/'collector-alice.sock'
assert collector_socket_path(str(socket),runtime_private)==socket
sibling=root/'qa/private';sibling.mkdir(mode=0o700)
outside=[sibling/'collector.sock',root/'foreign.sock']
public=runtime_private/'public';public.mkdir(mode=0o755);outside.append(public/'collector.sock')
link=runtime_private/'link';link.symlink_to(sibling,target_is_directory=True);outside.append(link/'collector.sock')
for wrong in outside:
 try:collector_socket_path(str(wrong),runtime_private)
 except (Invalid,FileNotFoundError):pass
 else:raise AssertionError('wrong collector root/owner mode accepted')
socket.write_bytes(b'preserve-existing')
try:collector_socket_path(str(socket),runtime_private)
except Invalid:pass
else:raise AssertionError('occupied collector endpoint accepted')
assert socket.read_bytes()==b'preserve-existing'
''')

    def test_real_R19_Controller_and_AP5_bind_one_run_with_mismatch_rejection(self):
        self.run_bound('''
spec=scenario('unit-owned',1)
args=qa_arguments(root,SimpleNamespace(qa_inputs=root/'inputs.json',scenario=root/'scenario.json',timeout_secs=7200))
qa=module.Controller(args)
Path(args.scenario).write_text(json.dumps(spec))
qa.scenario=module.load(args.scenario);qa.run_id=spec['run']['id'];qa.url='http://127.0.0.1:1'
assert qa.scenario!=spec and same_json_value(qa.scenario,spec)
qa.process=SimpleNamespace(pid=8765,poll=lambda:None)
private=qa.run/'private';(private/'access').mkdir(parents=True,mode=0o700);private.chmod(0o700)
(private/'access/operator.json').write_text(json.dumps({'run_id':qa.run_id,'may':['operator'],'token':'unit-only'}))
ap5root=root/'ap5';ap5root.mkdir();evidence=Evidence(ap5root)
ap5=AP5Controller({}, {}, ap5root,evidence,spec,7200)
try:
 with patch('ap5.process_identity',return_value=(9876,'S')):
  ap5.adopt_shared_runtime(qa)
 assert ap5.child is qa.process and ap5.run==qa.run and ap5.id==qa.run_id
 assert collector_socket_path(str(ap5.run/'private/collector.sock'),qa.run/'private')==ap5.run/'private/collector.sock'
 assert ap5.deadline==min(ap5.deadline,qa.deadline)
 original=qa.process;qa.process=SimpleNamespace(pid=8766,poll=lambda:None)
 try:ap5.adopt_shared_runtime(qa)
 except Blocked:pass
 else:raise AssertionError('replacement runtime accepted')
 qa.process=original;qa.run_id='other-run'
 try:ap5.adopt_shared_runtime(qa)
 except Blocked:pass
 else:raise AssertionError('foreign run accepted')
 qa.run_id=ap5.id;qa.scenario['traffic']['rate']['ordinary_tx_per_sec']='0.2'
 try:ap5.adopt_shared_runtime(qa)
 except Blocked:pass
 else:raise AssertionError('numeric field changed to string accepted')
 qa.scenario=module.load(args.scenario)
 qa.run_id=ap5.id;qa.run=root/'outside'
 try:ap5.adopt_shared_runtime(qa)
 except Blocked:pass
 else:raise AssertionError('foreign owned root accepted')
 assert not any(p.name.endswith('-launch.json') for p in ap5root.iterdir())
finally:
 evidence.close();qa.capture.stream.close();qa.http_workers.shutdown(wait=True,cancel_futures=True)
''')

    def test_actual_measured_method_invokes_manual_witness_and_fault_cases(self):
        self.run_bound('''
calls=[]
c=SimpleNamespace(deadline=1000,args=SimpleNamespace(measure_seconds=0,pause_seconds=0,
 destination='unit-actual-allocated-address',amount_sats=10000,witness=True),driver='txd-0',
 scenario={'identities':[{'id':'alice'}]},capture=SimpleNamespace(add=lambda *a:None))
c.command=lambda name,payload:calls.append((name,payload))
c.journals=lambda:[]
c.witness=lambda:calls.append(('witness',None))
c.acceptance_hooks=lambda:calls.append(('hold-loss',None))
c.version_rejection_case=lambda:calls.append(('version-rejection',None))
c.version_rejection_loss_case=lambda:calls.append(('version-response-loss',None))
c.check=lambda name,callback:callback()
with patch.object(module.time,'monotonic',return_value=100),patch.object(module.time,'monotonic_ns',return_value=100):
 module.Controller.measured(c)
assert [n for n,_ in calls]==['pause','resume','manual','manual','witness','hold-loss','version-rejection','version-response-loss']
assert calls[2][1]==calls[3][1] and calls[2][1]['destination_address']=='unit-actual-allocated-address'
''')

    def test_actual_receipt_rejects_omitted_required_case_despite_Ark_completion(self):
        self.run_bound('''
from case_contract import required
args=qa_arguments(root,SimpleNamespace(qa_inputs=root/'inputs.json',scenario=root/'scenario.json',timeout_secs=7200))
spec=scenario('unit-receipt',1)
Path(args.inputs).write_text('{}');Path(args.scenario).write_text(json.dumps(spec))
qa=module.Controller(args)
qa.scenario=spec;qa.run_id=spec['run']['id'];qa.binding={'source_snapshot_sha256':'00'*32,'binaries':{}}
(qa.priv/'scenario.json').write_text(json.dumps(spec))
qa.results={name:{'status':'PASSED','detail':'synthetic unit status only'} for name in required('funded',True)}
del qa.results['witness_spends']
qa.results['ap5_ark_lifecycle']={'status':'PASSED','detail':'unit status cannot cover absent case'}
try:
 assert qa.receipt()==2
 receipt=json.loads((qa.root/'acceptance.json').read_text())
 assert receipt['status']=='INCONCLUSIVE' and receipt['cases']['witness_spends']['status']=='INCONCLUSIVE'
 assert 'witness_spends' in receipt['required_cases']
 assert shared_exit_status(2,'passed',[],'passed')==2
 assert shared_exit_status(0,'inconclusive',[],'passed')==2
 assert shared_exit_status(0,'passed',[],'inconclusive')==2
 assert shared_exit_status(1,'passed',[],'passed')==1
finally:
 qa.capture.stream.close();qa.http_workers.shutdown(wait=True,cancel_futures=True)
''')

    def test_r18_cannot_borrow_r19_descriptor_or_source_qualification(self):
        # The mismatch is refused before import; use a separate interpreter so
        # the later valid closure is never confused with already loaded modules.
        code = '''
from pathlib import Path
import json,tempfile
from shared_sources import load_qa
from common import sha,Blocked
with tempfile.TemporaryDirectory(prefix='ap5-r19-negative-',dir='/tmp/opencode') as tmp:
 root=Path(tmp);value=json.loads(Path('qa-source-r18.json').read_text());value['api']='t3-controller-r19'
 p=root/'wrong.json';p.write_text(json.dumps(value))
 try:load_qa(p,sha(p),root/'source')
 except Blocked:pass
 else:raise AssertionError('r18 source admitted as r19')
'''
        result = subprocess.run([sys.executable, '-B', '-c', code], cwd=Path(__file__).parent, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr.decode())

    def test_numeric_scenario_comparison_preserves_types_and_exact_decimal_values(self):
        from decimal import Decimal
        from common import same_json_value
        self.assertTrue(same_json_value({'rate': 0.2}, {'rate': Decimal('0.2')}))
        for left, right in [(True, 1), ('0.2', Decimal('0.2')), (0.2, Decimal('0.20000000000000001')),
                            (float('inf'), Decimal('Infinity')), ([1], [1, 2]), ({'x': 1}, {'y': 1})]:
            with self.subTest(left=left, right=right):
                self.assertFalse(same_json_value(left, right))
