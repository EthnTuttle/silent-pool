"""Controlled in-process hooks only; no runtime/miner/SQL/live proof is simulated as acceptance."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from common import Blocked, Evidence, sha
from ap5 import Controller as AP5Controller
from scenario import scenario
from shared_run import MirrorCapture, SharedCoordinator, coordinated_qa_class, FINAL_CASES


class Sink:
    def __init__(self, events, prefix):
        self.events, self.prefix = events, prefix

    def add(self, kind, payload):
        self.events.append((self.prefix + ':' + kind, payload))
        return len(self.events)

    save = add


class Hooks:
    """A controlled call-order fixture, not a substitute live-controller adapter."""
    def __init__(self, fail=None, cancel=False):
        self.events = []
        self.fail, self.cancel = fail, cancel
        self.runtime_starts = self.bootstrap_observations = self.drains = self.stops = 0
        self.process = None
        self.capture = Sink(self.events, 'qa')
        self.results = {}
        self.source_feed_runtime = None
        self.http_workers = SimpleNamespace(shutdown=lambda **kw: self.note('http-workers-closed'))
        self.reenter = None
        self.args = SimpleNamespace(destination=None, witness=True, allow_reorg=True, operator_ui=True)
        self.run_id = 'unit-shared'
        self.driver = 'txd-0'
        self.scenario = {'traffic': {'driver': {'actors': [{'id': 'txd-0'}, {'id': 'txd-1'}]}}}
        self.temporary = tempfile.TemporaryDirectory(prefix='ap5-r21-hooks-')
        self.priv = Path(self.temporary.name)
        self.final_status = 'PASSED'

    def note(self, stage):
        self.events.append((stage, None))
        if self.fail == stage:
            if self.cancel:
                raise KeyboardInterrupt('unit cancellation')
            raise Blocked('unit hook failure: ' + stage)

    def prerequisites(self):
        self.note('prerequisites')

    def start(self):
        self.runtime_starts += 1
        self.process = object()
        self.source_feed_runtime = SimpleNamespace(final_drain=self.drain, close=lambda: self.close())
        self.note('runtime-start')

    def bootstrap(self):
        self.bootstrap_observations += 1
        self.note('observe-original-bootstrap')

    def measured(self):
        assert self.args.destination == 'unit-allocated-return-address'
        self.note('qa-measured')
        self.note('qa-manual')
        self.note('qa-witness')
        self.note('qa-fault-cases')

    def reorg(self):
        self.note('qa-reorg')

    def audit(self):
        self.note('qa-audit')

    def audit_live(self):
        self.audit()

    def command(self, name, payload, timeout=10):
        assert payload['type'] == 'pause_tx_driver'
        self.note('qa-final-traffic-pause')

    def final_audit_after_drain(self):
        self.note('qa-final-audit')
        checks = {name: {'status': self.final_status, 'detail': 'unit-hook-only'} for name in FINAL_CASES}
        for name, result in checks.items():self.result(name, result['status'], result['detail'])
        self.result('final_sealed_prefix', self.final_status, 'unit-hook-only')
        return {'status': self.final_status, 'scope': 'final_raw_source_SQL_native_prefix_only',
                'boundary': self.source_feed_runtime.final_boundary_ref, 'stable_key': 'unit-stable-key', 'checks': checks}

    def result(self, name, status, detail):
        self.results[name] = {'status': status, 'detail': detail}

    def drain(self):
        self.drains += 1
        self.note('drain-enter')
        if self.reenter:
            self.reenter()
        for name in ('mining-feed-drained', 'exact-checkpoints', 'economic-convergence', 'pool-writer-stop', 'source-EOF', 'final-raw-SQL'):
            self.note(name)
        tip = {'height': 2, 'hash': '22' * 32}
        boundary = {'schema': 'spc-lab-owned-economic-drain/2', 'run_id': self.run_id, 'status': 'CONVERGED',
                    'tip': tip, 'stable_key': 'unit-stable-key', 'chain_evidence': None, 'native_boundary': None,
                    'checkpoint_journal': None, 'pool_config_sha256': {}, 'pools': {}, 'source_EOF': {},
                    'checkpoint_coverage': {'status': 'COMPLETE_CAPTURE'}, 'unbooked_liabilities_preserved': True,
                    'scope': 'controlled unit sequencing only; not financial evidence'}
        path = self.priv / 'economic-final-boundary.json';path.write_text(json.dumps(boundary))
        self.source_feed_runtime.final_boundary_ref = {'path': str(path), 'sha256': sha(path), 'coverage_status': 'COMPLETE_CAPTURE'}
        return {'status': 'CONVERGED_COMPLETE_CAPTURE', 'scope': 'unit-hook-order-only', 'tip': tip, 'stable_key': 'unit-stable-key'}

    def close(self):
        self.note('capture-close')
        return {'unit': 'reader-join-completed'}

    def cleanup(self):
        raise AssertionError('second independent QA teardown forbidden')


class AP5Hooks:
    def __init__(self, owner):
        self.owner, self.child = owner, None
        self.ev = Sink(owner.events, 'ap5')
        self.id = 'unit-shared'
        self.cleanup = {'status': 'inconclusive'}

    def start(self):
        raise AssertionError('AP5 cannot start a second runtime')

    def adopt_shared_runtime(self, owner):
        assert owner is self.owner
        self.child = owner.process
        self.owner.note('bind-same-runtime')

    def observe_started_runtime(self):
        self.owner.note('ap5-observe')

    def fund(self):
        self.owner.note('ap5-fund')

    def return_address(self, user):
        assert user == 'alice'
        self.owner.note('actual-allocation-destination')
        return 'unit-allocated-return-address'

    def board(self):
        self.owner.note('ap5-board')

    def lifecycle(self):
        self.owner.note('ap5-lifecycle')

    def stop(self):
        self.owner.stops += 1
        self.owner.note('ap5-owned-teardown')
        self.cleanup = {'status': 'passed'}


class SharedRunTests(unittest.TestCase):
    def make(self, fail=None, cancel=False):
        qa = Hooks(fail, cancel)
        self.addCleanup(qa.temporary.cleanup)
        ap5 = AP5Hooks(qa)
        run = SharedCoordinator(qa, ap5, lambda q, a: q.note('same-binding-verified'))
        return qa, ap5, run

    def test_one_runtime_bootstrap_and_lifecycle_before_exact_final_drain_order(self):
        qa, ap5, run = self.make()
        result = run.run()
        names = [name for name, _ in qa.events]
        ordered = ['runtime-start', 'observe-original-bootstrap', 'ap5-observe', 'actual-allocation-destination',
                   'qa-measured', 'qa-manual', 'qa-witness', 'qa-fault-cases', 'qa-reorg', 'ap5-fund', 'ap5-board',
                   'ap5-lifecycle', 'qa-audit', 'qa-final-traffic-pause', 'drain-enter', 'exact-checkpoints', 'economic-convergence',
                   'pool-writer-stop', 'source-EOF', 'final-raw-SQL', 'ap5:shared-final-drain-result',
                   'qa:shared-final-drain-result', 'qa-final-audit', 'ap5:shared-final-audit-result',
                   'capture-close', 'ap5-owned-teardown', 'http-workers-closed']
        self.assertEqual(sorted(names.index(n) for n in ordered), [names.index(n) for n in ordered])
        self.assertIs(ap5.child, qa.process)
        self.assertEqual((qa.runtime_starts, qa.bootstrap_observations, qa.drains, qa.stops), (1, 1, 1, 1))
        self.assertEqual(result['drain'], 'converged_complete_capture')

    def test_failed_required_QA_phase_cannot_be_counted_as_lifecycle_completion(self):
        for phase in ('qa-measured', 'qa-manual', 'qa-witness', 'qa-reorg'):
            qa, ap5, run = self.make(phase)
            outcome = run.run()
            names = [name for name, _ in qa.events]
            self.assertNotIn('ap5-board', names)
            self.assertNotIn('shared-ap5-lifecycle-finished', names)
            self.assertEqual(outcome['execution'], 'inconclusive')
            self.assertEqual((qa.runtime_starts, qa.drains, qa.stops), (1, 1, 1))

    def test_failed_checkpoint_drain_is_retained_in_both_sinks_before_owned_cleanup(self):
        qa, ap5, run = self.make('exact-checkpoints')
        result = run.run()
        names = [n for n, _ in qa.events]
        self.assertNotIn('pool-writer-stop', names)
        self.assertNotIn('source-EOF', names)
        self.assertEqual(result['drain'], 'inconclusive')
        self.assertEqual(qa.results['economic_final_drain']['status'], 'INCONCLUSIVE')
        for sink in ('qa', 'ap5'):
            self.assertLess(names.index(sink + ':shared-stage-incomplete'), names.index('ap5-owned-teardown'))
        self.assertEqual((qa.drains, qa.stops), (1, 1))

    def test_post_EOF_drain_failure_does_not_claim_pass_or_skip_cleanup(self):
        qa, ap5, run = self.make('final-raw-SQL')
        self.assertEqual(run.run()['drain'], 'inconclusive')
        self.assertEqual(qa.stops, 1)
        self.assertNotEqual(qa.results['economic_final_drain']['status'], 'PASSED')

    def test_failed_final_audit_invalidates_earlier_prefix_pass_without_losing_failure(self):
        qa, ap5, run = self.make('qa-final-audit')
        qa.results['payout_ladder'] = {'status': 'PASSED', 'detail': 'earlier-prefix-only'}
        qa.results['history3'] = {'status': 'FAILED', 'detail': 'earlier-contradiction'}
        result = run.run()
        self.assertEqual(result['final_audit'], 'inconclusive')
        self.assertEqual(qa.results['payout_ladder']['status'], 'INCONCLUSIVE')
        self.assertEqual(qa.results['history3']['status'], 'FAILED')
        self.assertEqual((qa.drains, qa.stops), (1, 1))
        run.finalize()
        self.assertEqual(sum(name == 'qa-final-audit' for name, _ in qa.events), 1)

    def test_failed_drain_does_not_invoke_final_audit(self):
        qa, ap5, run = self.make('final-raw-SQL')
        run.run()
        self.assertNotIn('qa-final-audit', [name for name, _ in qa.events])
        self.assertEqual(qa.results['final_sealed_prefix']['status'], 'INCONCLUSIVE')

    def test_nonpassing_returned_final_dict_never_becomes_pass_and_still_cleans_once(self):
        for status in ('INCONCLUSIVE', 'FAILED'):
            qa, ap5, run = self.make()
            qa.final_status = status
            result = run.run()
            self.assertEqual(result['final_audit'], status.lower())
            self.assertEqual(qa.results['final_sealed_prefix']['status'], status)
            self.assertEqual((qa.drains, qa.stops), (1, 1))

    def test_early_start_failure_binds_partial_runtime_and_cleans_it_once(self):
        qa, ap5, run = self.make('runtime-start')
        result = run.run()
        self.assertIs(ap5.child, qa.process)
        self.assertEqual((qa.runtime_starts, qa.bootstrap_observations, qa.drains, qa.stops), (1, 0, 1, 1))
        self.assertEqual(result['execution'], 'inconclusive')

    def test_preflight_failure_never_starts_runtime_and_cleanup_still_runs(self):
        qa, ap5, run = self.make('prerequisites')
        result = run.run()
        self.assertEqual((qa.runtime_starts, qa.drains, qa.stops), (0, 0, 1))
        self.assertEqual(result['drain'], 'inconclusive')

    def test_timeout_cancellation_and_lifecycle_error_all_finalize(self):
        for cancel in (False, True):
            qa, ap5, run = self.make('ap5-lifecycle', cancel)
            result = run.run()
            self.assertEqual(result['execution'], 'inconclusive')
            self.assertEqual((qa.drains, qa.stops), (1, 1))

    def test_actual_timeout_exception_runs_final_drain_and_owned_cleanup(self):
        qa, ap5, run = self.make()
        ap5.fund = lambda: (_ for _ in ()).throw(subprocess.TimeoutExpired('unit-owner-client', 1))
        result = run.run()
        self.assertEqual((qa.drains, qa.stops), (1, 1))
        self.assertTrue(any(e['class'] == 'TimeoutExpired' for e in result['errors']))

    def test_double_and_reentrant_finalization_never_repeats_drain_or_teardown(self):
        qa, ap5, run = self.make()
        seen = []
        qa.reenter = lambda: seen.append(run.finalize())
        run.run()
        run.finalize(); run.finalize()
        self.assertEqual(seen, [{'status': 'finalization_in_progress'}])
        self.assertEqual((qa.drains, qa.stops), (1, 1))
        with self.assertRaises(Blocked):run.run()
        self.assertEqual(qa.runtime_starts, 1)

    def test_reader_close_error_and_cleanup_error_preserve_inconclusive(self):
        for stage in ('capture-close', 'ap5-owned-teardown'):
            qa, ap5, run = self.make(stage)
            result = run.run()
            self.assertTrue(result['errors'])
            self.assertEqual(qa.stops, 1)
            self.assertIn('http-workers-closed', [n for n, _ in qa.events])

    def test_result_sink_failure_cannot_skip_owned_cleanup(self):
        qa, ap5, run = self.make('exact-checkpoints')
        qa.result = lambda *args: (_ for _ in ()).throw(OSError('unit result write failure'))
        result = run.run()
        self.assertEqual(qa.stops, 1)
        self.assertTrue(any(e['stage'].startswith('qa_result:') for e in result['errors']))

    def test_QA_actual_final_class_has_no_legacy_audit_callback_proxy(self):
        class Base:
            def audit_live(self):raise AssertionError('export inspection only')
            def final_audit_after_drain(self):raise AssertionError('export inspection only')
        self.assertIs(coordinated_qa_class(SimpleNamespace(Controller=Base)), Base)
        with self.assertRaises(Blocked):coordinated_qa_class(SimpleNamespace(Controller=object))

    def test_actual_launch_capture_rejects_second_runtime_intent(self):
        events = []
        mirror = MirrorCapture(Sink(events, 'qa'), Sink(events, 'ap5'))
        mirror.add('command_spawn_intent', {'argv': ['actual-argv-from-owner']})
        self.assertEqual([payload['argv'] for name, payload in events if name == 'ap5:launch'], [['actual-argv-from-owner']])
        with self.assertRaises(Blocked):mirror.add('command_spawn_intent', {'argv': ['second']})

    def test_actual_AP5_shared_adoption_binds_same_process_path_and_original_deadline(self):
        with tempfile.TemporaryDirectory(prefix='ap5-shared-bind-') as tmp:
            root = Path(tmp)
            evidence_root = root / 'ap5'; evidence_root.mkdir()
            qa_root = root / 'qa'; qa_root.mkdir()
            run = qa_root / 'runtime'; (run / 'private/access').mkdir(parents=True)
            spec = scenario('unit-same-runtime', 1)
            (run / 'private/access/operator.json').write_text(json.dumps(
                {'run_id': spec['run']['id'], 'may': ['operator'], 'token': 'unit-only'}))
            ev = Evidence(evidence_root)
            try:
                ap5 = AP5Controller({}, {}, evidence_root, ev, spec, 60)
                process = SimpleNamespace(pid=1234, poll=lambda: None)
                owner = SimpleNamespace(root=qa_root, run=run, process=process, url='http://127.0.0.1:1',
                                        run_id=ap5.id, scenario=spec, deadline=ap5.deadline - 5)
                with patch('ap5.process_identity', return_value=(9876, 'S')):
                    ap5.adopt_shared_runtime(owner)
                self.assertIs(ap5.child, process)
                self.assertEqual(ap5.run, run)
                self.assertEqual(ap5.deadline, owner.deadline)
                self.assertEqual(ap5.controller_ticks, 9876)
                self.assertIsNone(ap5.log)
                self.assertFalse(any('launch' in p.name for p in evidence_root.iterdir()))
            finally:
                ev.close()

    def test_portable_QA_r21_import_and_descriptor_tamper_refusal(self):
        # Fresh interpreter avoids global module-name collisions. Imports only;
        # no Controller construction, actors, services or source observer start.
        code = '''
from pathlib import Path
import json,os,tempfile
from source_package import package_qa
from shared_sources import load_qa
from common import sha,Blocked
with tempfile.TemporaryDirectory(prefix='ap5-qa-code-') as t:
 package_qa(Path(os.environ.get('AP5_QA_TEST_SOURCE',str(Path.cwd().parents[1]))),Path(t)/'package')
 p=Path(t)/'package/qa-source.json'; bad=Path(t)/'bad.json'
 value=json.loads(p.read_text());value['api']='unreviewed';bad.write_text(json.dumps(value))
 try:load_qa(bad,sha(bad),Path(t)/'bad-source')
 except Blocked:pass
 else:raise AssertionError('unknown API admitted')
 module,_=load_qa(p,sha(p),Path(t)/'source')
 assert callable(module.Controller.bootstrap) and callable(module.Controller.receipt)
 import source_feed_runtime
 assert callable(source_feed_runtime.CollectorRuntime.final_drain)
 assert callable(source_feed_runtime.CollectorRuntime.close)
 print('PORTABLE_R21_IMPORT_ONLY_PASS')
'''
        result = subprocess.run([sys.executable, '-B', '-c', code], cwd=Path(__file__).parent, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr.decode())
        self.assertIn(b'PORTABLE_R21_IMPORT_ONLY_PASS', result.stdout)
