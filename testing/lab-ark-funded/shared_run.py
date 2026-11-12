#!/usr/bin/env python3
"""One live runtime, T3 capture owner, AP5 lifecycle, then one final drain/teardown.

No archive is treated as a live adapter. QA code is imported from the exact
reviewed archive. Missing source/bundle/profile evidence rejects before launch.
"""
import argparse
import copy
import json
from pathlib import Path
import signal
import threading
from types import SimpleNamespace

from ap5 import Controller
from common import Blocked, Evidence, check_bundle, encoded, fresh_root, require, sha, same_json_value
from pool_control import strict_json
from shared_sources import load_qa
FINAL_CASES = frozenset({'raw_chain', 'source_publication', 'full_window_coverage',
                         'payout_ladder', 'premining_proof_feed', 'witness_job_republication'})


def coordinated_qa_class(module):
    # r20 owns the live/final split. No subclass, callback proxy or SQL view.
    require(callable(getattr(module.Controller, 'audit_live', None))
            and callable(getattr(module.Controller, 'final_audit_after_drain', None)), 'actual r20 Controller exports required')
    return module.Controller


def checked_final_audit(qa, result, drained):
    require(type(result) is dict and set(result) == {'status', 'scope', 'boundary', 'stable_key', 'checks'}, 'r20 final audit closed result')
    require(result['status'] in {'PASSED', 'FAILED', 'INCONCLUSIVE'}
            and result['scope'] == 'final_raw_source_SQL_native_prefix_only', 'r20 final audit status/scope')
    expected = set(FINAL_CASES)
    if not qa.args.witness:
        expected.remove('witness_job_republication')
    require(type(result['checks']) is dict and set(result['checks']) == expected, 'r20 final check census')
    for value in result['checks'].values():
        require(type(value) is dict and set(value) == {'status', 'detail'}
                and value['status'] in {'PASSED', 'FAILED', 'INCONCLUSIVE'}, 'r20 final check result shape')
    ref = result['boundary']
    require(type(ref) is dict and set(ref) == {'path', 'sha256', 'coverage_status'}
            and ref == qa.source_feed_runtime.final_boundary_ref and ref['coverage_status'] == 'COMPLETE_CAPTURE', 'r20 final boundary reference')
    path = Path(ref['path'])
    require(path == qa.priv / 'economic-final-boundary.json' and not path.is_symlink()
            and path.resolve().is_relative_to(qa.priv.resolve()) and path.is_file()
            and path.stat().st_size <= 128 * 1024 * 1024 and sha(path) == ref['sha256'], 'r20 retained boundary digest/path')
    boundary = strict_json(path.read_bytes())
    require(set(boundary) == {'schema', 'run_id', 'status', 'tip', 'stable_key', 'chain_evidence', 'native_boundary',
            'checkpoint_journal', 'pool_config_sha256', 'pools', 'source_EOF', 'checkpoint_coverage',
            'unbooked_liabilities_preserved', 'scope'}, 'r20 closed economic boundary')
    require(boundary['schema'] == 'spc-lab-owned-economic-drain/2' and boundary['run_id'] == qa.run_id
            and boundary['status'] == 'CONVERGED' and boundary['stable_key'] == result['stable_key'] == drained['stable_key']
            and boundary['tip'] == drained['tip'], 'r20 final boundary run/schema/drain mismatch')
    require(qa.results.get('final_sealed_prefix', {}).get('status') == result['status'], 'r20 returned status differs from sticky final verdict')
    if result['status'] == 'PASSED':
        require(all(v['status'] == 'PASSED' for v in result['checks'].values())
                and all(qa.results.get(k, {}).get('status') == 'PASSED' for k in expected), 'r20 missing/failed cases cannot pass')
    return result['status']


class MirrorCapture:
    """Retain the actual QA launch argv in AP5's existing replayable event seal."""
    def __init__(self, capture, evidence):
        self.capture, self.evidence, self.launches = capture, evidence, 0

    def __getattr__(self, name):
        return getattr(self.capture, name)

    def add(self, kind, payload):
        sequence = self.capture.add(kind, payload)
        if kind == 'command_spawn_intent':
            require(self.launches == 0, 'shared runtime launch repeated')
            self.launches += 1
            self.evidence.save('launch', {'argv': payload['argv'], 'scope': 'single QA-owned shared runtime'})
        return sequence


class SharedCoordinator:
    """Single serial owner; finalization does not recurse into QA.cleanup()."""
    def __init__(self, qa, ap5, validate_binding):
        self.qa, self.ap5, self.validate_binding = qa, ap5, validate_binding
        self.state = 'new'
        self.lock = threading.Lock()
        self.outcome = {'execution': 'not_started', 'drain': 'not_attempted', 'final_audit': 'not_attempted', 'cleanup': 'not_attempted'}
        self.errors = []
        self.cancel_requested = False
        self.live_audit_finished = False
        self.traffic_quiesced = False

    def emit(self, name, payload):
        # Failure of one evidence sink must not skip cleanup or the other sink.
        for write in (lambda: self.ap5.ev.save(name, payload), lambda: self.qa.capture.add(name, payload)):
            try:
                write()
            except BaseException as error:
                self.errors.append({'stage': 'evidence:' + name, 'class': type(error).__name__})

    def failure(self, stage, error):
        value = {'stage': stage, 'class': type(error).__name__, 'status': 'inconclusive'}
        self.errors.append(value)
        self.emit('shared-stage-incomplete', value)

    def qa_result(self, name, state, detail):
        try:
            self.qa.result(name, state, detail)
        except BaseException as error:
            self.failure('qa_result:' + name, error)

    def invalidate_final_passes(self):
        if self.qa.results.get('final_sealed_prefix', {}).get('status') != 'FAILED':
            self.qa_result('final_sealed_prefix', 'INCONCLUSIVE', 'complete final-prefix audit unavailable')
        for name in FINAL_CASES:
            if self.qa.results.get(name, {}).get('status') == 'PASSED':
                self.qa_result(name, 'INCONCLUSIVE', 'earlier prefix is not a final-capture verdict')

    def quiesce_final_traffic(self):
        drivers = [a['id'] for a in self.qa.scenario['traffic']['driver']['actors']]
        for driver in drivers:
            if self.live_audit_finished and driver == self.qa.driver:
                continue  # Actual r20 audit_live already acknowledged this pause.
            self.qa.command('shared-final-pause-' + driver, {'type': 'pause_tx_driver', 'driver_id': driver}, timeout=10)
        self.traffic_quiesced = True
        self.emit('shared-final-traffic-quiesced', {'drivers': drivers})

    def run(self):
        with self.lock:
            require(self.state == 'new', 'shared coordinator cannot start a second runtime')
            self.state = 'running'
            self.run_thread = threading.get_ident()
        try:
            self.qa.prerequisites()
            self.validate_binding(self.qa, self.ap5)
            self.qa.start()  # Sole runtime constructor; Runtime owns bootstrap.
            self.ap5.adopt_shared_runtime(self.qa)
            self.qa.bootstrap()  # Observes original phase; never HTTP bootstrap.
            self.ap5.observe_started_runtime()
            # Run the real funded QA phases before creating Ark anchors: the
            # reorg exercise must not displace already-verified board/offboard
            # proofs. Destination comes from this run's actual native allocation.
            self.qa.args.destination = self.ap5.return_address('alice')
            self.emit('shared-qa-manual-destination', {'address': self.qa.args.destination, 'owner': 'alice'})
            self.qa.measured()
            self.qa.reorg()
            self.ap5.fund()
            self.ap5.board()
            self.ap5.lifecycle()
            self.emit('shared-ap5-lifecycle-finished', {'run_id': self.ap5.id})
            self.qa.audit_live()
            self.live_audit_finished = True
            self.outcome['execution'] = 'observations_finished'
        except BaseException as error:
            self.outcome['execution'] = 'inconclusive'
            self.failure('execution', error)
        finally:
            self.finalize()
        return copy.deepcopy({**self.outcome, 'errors': self.errors})

    def finalize(self):
        # Reentrant/concurrent calls return without touching actors; only the
        # caller holding this lock owns the drain, source close and cleanup.
        if not self.lock.acquire(blocking=False):
            return {'status': 'finalization_in_progress'}
        try:
            if self.state == 'running' and getattr(self, 'run_thread', None) != threading.get_ident():
                self.cancel_requested = True
                return {'status': 'cancellation_requested_owner_must_finalize'}
            if self.state == 'done':
                return copy.deepcopy(self.outcome)
            self.state = 'finalizing'
            try:
                if getattr(self.qa, 'process', None) is not None and self.ap5.child is None:
                    self.ap5.adopt_shared_runtime(self.qa)
            except BaseException as error:
                self.failure('partial_start_binding', error)
            runtime = getattr(self.qa, 'source_feed_runtime', None)
            try:
                if getattr(self.qa, 'process', None) is not None:
                    self.quiesce_final_traffic()
            except BaseException as error:
                self.failure('final_traffic_quiescence', error)
            try:
                require(runtime is not None, 'actual QA source-feed runtime unavailable for final drain')
                result = runtime.final_drain()  # Exactly once, also on early failure.
                require(type(result) is dict and result.get('status') == 'CONVERGED_COMPLETE_CAPTURE',
                        'QA final drain did not establish complete capture')
                self.outcome['drain'] = 'converged_complete_capture'
                self.qa_result('economic_final_drain', 'PASSED', result)
                self.emit('shared-final-drain-result', result)
                try:
                    require(self.traffic_quiesced, 'ordinary traffic quiescence is unproved')
                    audited = self.qa.final_audit_after_drain()
                    state = checked_final_audit(self.qa, audited, result)
                    self.outcome['final_audit'] = {'PASSED': 'final_prefix_audited', 'FAILED': 'failed', 'INCONCLUSIVE': 'inconclusive'}[state]
                    self.emit('shared-final-audit-result', audited)
                except BaseException as error:
                    self.outcome['final_audit'] = 'inconclusive'
                    self.failure('final_capture_audit', error)
                    self.invalidate_final_passes()
            except BaseException as error:
                self.outcome['drain'] = 'inconclusive'
                self.failure('final_drain', error)
                self.qa_result('economic_final_drain', 'INCONCLUSIVE', type(error).__name__)
                self.invalidate_final_passes()
            # Retain drain success/failure in BOTH still-open streams before any
            # teardown. Closing the actual collector drains its reader/queue.
            try:
                if runtime is not None:
                    result = runtime.close()
                    self.qa_result('source_feed_cleanup', 'PASSED', result)
                    self.emit('shared-capture-closed', result)
            except BaseException as error:
                self.failure('source_close', error)
                self.qa_result('source_feed_cleanup', 'INCONCLUSIVE', type(error).__name__)
            try:
                self.ap5.stop()  # Sole controller stop/reap, never QA.cleanup().
                self.outcome['cleanup'] = self.ap5.cleanup.get('status', 'inconclusive')
            except BaseException as error:
                self.outcome['cleanup'] = 'inconclusive'
                self.failure('ap5_cleanup', error)
            try:
                self.qa.http_workers.shutdown(wait=True, cancel_futures=True)
            except BaseException as error:
                self.failure('qa_http_workers', error)
            self.emit('shared-finalization-result', {**self.outcome, 'errors': list(self.errors)})
            self.state = 'done'
            return copy.deepcopy(self.outcome)
        finally:
            self.lock.release()


def validate_shared_binding(qa, ap5):
    require(qa.run_id == ap5.id and same_json_value(qa.scenario, ap5.spec), 'shared scenario differs between consumers')
    require(qa.args.operator_ui is True and qa.args.allow_reorg is True and qa.args.witness is True,
            'shared QA phase/actor scope differs')
    require(qa.binding.get('source_feed'), 'actual four-wallet source feed binding required')
    require(type(qa.binding.get('path_env')) is str and qa.binding['path_env']
            and all(Path(p).is_absolute() and p.startswith('/nix/store/') for p in qa.binding['path_env'].split(':')),
            'shared actor PATH must be supplied by the immutable Nix closure')
    require(all(w.get('native_audit') for w in qa.scenario['wallet_agents']), 'shared wallet native audit profiles missing')
    for name in ('lab', 'pool', 'miner', 'txd', 'node', 'scanner', 'agent', 'captaind', 'grpcurl', 'redis-server', 'redis-cli', 'psql'):
        left, right = qa.binding['binaries'][name], ap5.bundle['binaries'][name]
        require(left['sha256'] == right['sha256'] and Path(left['path']).resolve() == Path(right['path']).resolve(),
                'shared executable binding differs: ' + name)
    require(qa.binding['source_snapshot_sha256'] == ap5.bundle['joined_source_manifest']['sha256'],
            'shared joined source snapshot differs')
    probe = ap5.bundle['probe_invocation']
    require(qa.binding['binaries']['traffic-probe']['path'] == probe['program']['path']
            and qa.binding['binaries']['traffic-probe']['sha256'] == probe['program']['sha256']
            and qa.binding['probe_script']['path'] == probe['script']['path']
            and qa.binding['probe_script']['sha256'] == probe['script']['sha256'], 'shared executed probe tuple differs')
    ap5.deadline = min(ap5.deadline, qa.deadline)
    qa.deadline = ap5.deadline


def observe_qa_cleanup(qa, ap5):
    """QA's actual independent resource assessor; consumes the ONE AP5 reap."""
    from cleanup_proof import assess_cleanup, collect_resource_receipt, identity_observation
    record = ap5.reap_record
    require(record is not None, 'shared reaper record unavailable')
    after = qa.manifest()
    require(after == record['after'], 'shared post-reap manifest changed')
    identities = {(p['role'], p['entity']): identity_observation(p) for p in after['processes']}
    resources = collect_resource_receipt(qa.run, after)
    result = assess_cleanup(record['exit'], record['stdout'], record['before'], after, identities, resources)
    qa.capture.add('shared_cleanup_resources', resources)
    qa.result('cleanup', result['status'], result)
    if qa.results.get('source_feed_cleanup', {}).get('status') != 'PASSED':
        qa.result('cleanup', 'INCONCLUSIVE', 'source workers did not close completely')


def seal_ap5(ap5, bundle_path, outcome):
    bundle = ap5.bundle
    verdict = ap5.verdict({'bundle_sha256': sha(bundle_path),
        'bundle_artifact': {'path': str(bundle_path.resolve()), 'sha256': sha(bundle_path)},
        'joined_source_manifest': bundle['joined_source_manifest'], 'joined_validation_receipt': bundle['joined_validation_receipt'],
        'binaries': bundle['binaries'], 'vtxo_verifier': bundle.get('vtxo_verifier'),
        'pool_wallet_executor': bundle.get('pool_wallet_executor')})
    path = ap5.ev.save('ap5-component-verdict', verdict)
    ap5.ev.save('result', {'schema': 'spc.lab.ap5.result/v2', 'scope': 'full',
        'status': 'shared-campaign-' + verdict['component_status'], 'component_status': verdict['component_status'],
        'component_verdict': {'file': path.name, 'sha256': sha(path)}, 'shared_execution': outcome,
        'funded_ark_lifecycle': outcome['execution'] == 'observations_finished', 'whole_lab_complete': False})
    ap5.ev.close()
    value = {'root': str(ap5.root), 'events_sha256': sha(ap5.ev.path),
             'last_seq': ap5.ev.sequence, 'last_digest': ap5.ev.previous}
    with (ap5.root / 'seal.json').open('x') as stream:
        json.dump(value, stream, sort_keys=True); stream.write('\n')
    return verdict['component_status']


def shared_exit_status(qa_exit, component_status, errors, cleanup_status):
    if qa_exit == 1 or component_status == 'failed':
        return 1
    return 0 if qa_exit == 0 and component_status == 'passed' and not errors and cleanup_status == 'passed' else 2


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('bundle', 'qa-inputs', 'scenario', 'qa-source', 'out'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--qa-source-sha256', required=True)
    parser.add_argument('--timeout-secs', type=int, default=7200)
    args = parser.parse_args()
    require(0 < args.timeout_secs <= 7200, 'shared run deadline bound')
    root = fresh_root(args.out)
    qa_module, descriptor = load_qa(args.qa_source, args.qa_source_sha256, root / 'qa-source')
    require(descriptor['api'] == 't3-controller-r21', 'shared live runner requires exact r21 portable source and final audit exports')
    bundle = strict_json(args.bundle.read_bytes())
    paths, source, receipt = check_bundle(bundle, full=True)  # Before actor construction.
    spec = strict_json(args.scenario.read_bytes())
    ap5_root = fresh_root(root / 'ap5')
    evidence = Evidence(ap5_root)
    evidence.save('verified-bundle', {'bundle': bundle, 'source_manifest_sha256': sha(source), 'validation_receipt_sha256': sha(receipt)})
    evidence.save('scenario', spec)
    evidence.save('probe-invocation', bundle['probe_invocation'])
    evidence.save('shared-qa-source', {'descriptor': descriptor, 'descriptor_sha256': args.qa_source_sha256})
    ap5 = Controller(bundle, paths, ap5_root, evidence, spec, args.timeout_secs)
    qa_args = qa_arguments(root, args)
    qa = coordinated_qa_class(qa_module)(qa_args)
    qa.capture = MirrorCapture(qa.capture, evidence)
    coordinator = SharedCoordinator(qa, ap5, validate_shared_binding)
    previous = {}
    def cancel(signum, frame):
        coordinator.cancel_requested = True
        if coordinator.state not in {'finalizing', 'done'}:
            raise KeyboardInterrupt('shared run cancelled')
    for sig in (signal.SIGINT, signal.SIGTERM):
        previous[sig] = signal.signal(sig, cancel)
    try:
        outcome = coordinator.run()
        # The assessor writes its own scoped status; do not wrap it in check(),
        # which would label a returned INCONCLUSIVE assessment PASSED.
        try:
            observe_qa_cleanup(qa, ap5)
        except BaseException as error:
            coordinator.failure('qa_cleanup_observation', error)
            coordinator.qa_result('cleanup', 'INCONCLUSIVE', type(error).__name__)
        if qa.process is not None:
            qa.check('post_cleanup_hash_budget', qa.audit_budget)
        # Both streams are open for the final cross-component observations.
        coordinator.emit('shared-receipts-closing', outcome)
        qa_exit = 2
        try:
            qa_exit = qa.receipt()
            evidence.save('shared-qa-receipt', {'path': str(qa.root / 'acceptance.json'),
                          'sha256': sha(qa.root / 'acceptance.json'), 'exit': qa_exit})
        except BaseException as error:
            coordinator.failure('qa_receipt', error)
        finally:
            outcome['errors'] = list(coordinator.errors)
            component_status = seal_ap5(ap5, args.bundle, outcome)
        return shared_exit_status(qa_exit, component_status, coordinator.errors, outcome['cleanup'])
    finally:
        for sig, handler in previous.items():
            signal.signal(sig, handler)
        if not evidence.file.closed:
            evidence.close()
        if not qa.capture.stream.closed:
            qa.capture.stream.close()


def qa_arguments(root, args):
    return SimpleNamespace(run_root=str(root / 'qa'), inputs=str(args.qa_inputs), scenario=str(args.scenario),
        case='funded', timeout=args.timeout_secs, start_timeout=180, measure_seconds=90, pause_seconds=5,
        max_blocks=100000, destination=None, amount_sats=10000, allow_reorg=True, witness=True, operator_ui=True)


if __name__ == '__main__':
    raise SystemExit(main())
