"""Source-only query intake; no SQL execution, actor startup or frozen-input edits."""
from pathlib import Path
import subprocess
import sys
import unittest


class QueryBindingTests(unittest.TestCase):
    def execute(self, body):
        code = '''
from pathlib import Path
import copy,hashlib,os,sys
from common import Blocked
# Explicit source fixture: source-only qualification, not loader/API admission.
default_source=os.environ.get('AP5_QA_TEST_SOURCE',str(Path.cwd().parents[1]))
source=Path(os.environ.get('AP5_QA_QUERY_TEST_SOURCE',default_source))
sys.path.insert(0,str(source/'testing/lab-traffic'))
from qa_query import fullrail_query,validate_query_binding
binding,raw=fullrail_query()
feed={'sql_contract':binding['sql_contract'],'sql_query':binding}
''' + body
        result = subprocess.run([sys.executable, '-B', '-c', code], cwd=Path(__file__).parent, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stdout.decode() + result.stderr.decode())

    def test_current_QA_exporters_derive_one_query_and_validate_it(self):
        self.execute('''
assert validate_query_binding(feed)==binding
assert binding['sha256']==hashlib.sha256(raw).hexdigest()
assert binding['query_id']=='spc-lab-immutable-payout-rows/5-fullrail-v1'
assert binding['schema']=='spc-lab-sql-observation/5-fullrail-v1'
assert binding['table_count']==22
''')

    def test_same_contract_and_query_id_do_not_admit_stale_query_hash_or_metadata(self):
        self.execute('''
for key,value in (('sha256','00'*32),('bytes',binding['bytes']+1),('bytes',float(binding['bytes'])),
                  ('table_count',21),('table_count',22.0),('schema','old')):
 bad=copy.deepcopy(feed);bad['sql_query'][key]=value
 try:validate_query_binding(bad)
 except Blocked:pass
 else:raise AssertionError('changed query metadata admitted: '+key)
for bad in ({'sql_contract':binding['sql_contract']},{'sql_contract':'old','sql_query':binding}):
 try:validate_query_binding(bad)
 except Blocked:pass
 else:raise AssertionError('missing query/changed contract admitted')
''')

    def test_collect_and_final_audit_exporter_disagreement_rejects(self):
        self.execute('''
import economic_checkpoint
original=economic_checkpoint.query
economic_checkpoint.query=lambda **kw:original(**kw)+'-- different final audit query'
try:fullrail_query()
except Blocked:pass
else:raise AssertionError('collector/final-audit query disagreement admitted')
''')

    def test_r26_query_matches_stock_gate_and_rejects_actual_r24_and_r25_digests(self):
        self.execute('''
assert binding['sha256']=='7d4c64aa165618cac9b73024042a858161d03e812612abd3dfe174185d204dac'
assert binding['bytes']==8828 and b"'postmaster_identity_file'" in raw
assert b"SET LOCAL TimeZone = 'UTC';" in raw and b"SET LOCAL DateStyle = 'ISO, MDY';" in raw
for digest,size in (('dbf0473d9747316f6852fc760f8b7604414c2e7d43a305e9520441063a25f1f0',8710),
                    ('16c8d72f970d1aed7a776e2b7a1f1269277d3f39dddb6cf6b78306f3d3802a1b',8699)):
 old=copy.deepcopy(feed);old['sql_query'].update(sha256=digest,bytes=size)
 try:validate_query_binding(old)
 except Blocked:pass
 else:raise AssertionError('old SQL admitted merely because contract/id/schema match')
''')

    def test_shared_preflight_rejects_stale_query_before_runtime_or_metadata_work(self):
        self.execute('''
from types import SimpleNamespace
from shared_run import validate_shared_binding
bad=copy.deepcopy(feed);bad['sql_query']['sha256']='dbf0473d9747316f6852fc760f8b7604414c2e7d43a305e9520441063a25f1f0'
scenario={'traffic':{'lab_acceptance':True}}
qa=SimpleNamespace(run_id='unit',scenario=scenario,binding={'source_feed':bad},
 args=SimpleNamespace(operator_ui=True,allow_reorg=True,witness=True))
ap5=SimpleNamespace(id='unit',spec=scenario)
try:validate_shared_binding(qa,ap5)
except Blocked as error:assert 'SQL query bytes/schema/census' in str(error)
else:raise AssertionError('stale query reached later startup/preflight work')
''')
