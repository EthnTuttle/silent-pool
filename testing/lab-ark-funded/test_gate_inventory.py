"""Synthetic consumer log fixtures only; no producer gate is admitted here."""
import copy
import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from common import Blocked, encoded, sha
from gate_inventory import MATRIX, TARGETS, FIX_FILES, rust_cases, node_cases, validate_completed_350, check_qualified_350, check_historical_332_source
from joined_checks import UI_FILES


def rust_log(count, prefix):
    return 'running %d tests\n' % count + ''.join(f'test {prefix}{n} ... ok\n' for n in range(count)) + f'test result: ok. {count} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n'


def node_log(count=36):
    return ''.join(f'✔ unit-{n} (0.1ms)\n' for n in range(count)) + f'ℹ tests {count}\nℹ pass {count}\nℹ fail 0\nℹ skipped 0\n'


class InventoryFixture:
    def __init__(self, base):
        self.base, self.root = base, base / 'source'
        self.root.mkdir()
        self.source_id = '/nix/store/unit-only-network-reap-source'
        self.source_files = {}
        for name in FIX_FILES | UI_FILES:
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('UNIT SOURCE FIXTURE: ' + name)
            self.source_files[name] = sha(path)
        self.manifest = {'files': [{'path': k, 'sha256': v} for k, v in sorted(self.source_files.items())]}
        source_ref = self.write('manifest.json', self.manifest)
        binaries = {name: {'sha256': hashlib.sha256(name.encode()).hexdigest()} for name in ('lab', 'txd', 'pool', 'agent', 'miner', 'scanner')}
        self.bundle = {'joined_source_manifest': source_ref, 'binaries': binaries}
        checks, suites = [], {}
        for name, count in MATRIX.items():
            if name in ('node', 'ui-browser-tests'):
                cwd = 'bins/spc-lab/tests'
                argv = [['node', f] for f in ('access.test.cjs', 'app_helpers.test.cjs', 'lab_helpers.test.cjs')] if name == 'node' else ['node', 'compiled_browser.cjs']
                text = node_log() if name == 'node' else json.dumps({'passed': 15, 'out': str(base / 'browser')})
                cases = node_cases(text) if name == 'node' else [f'browser-{n}' for n in range(15)]
            else:
                cwd = '.'
                package = 'spc-agent' if name == 'agent-ark-tests' else 'spc-pool' if name == 'pool-ark-tests' else 'spc-lab'
                argv = ['cargo', 'test', '--offline', '--locked', '-p', package]
                if name in ('agent-ark-tests', 'pool-ark-tests'):
                    argv += ['--features', 'ark']
                    if name == 'pool-ark-tests':argv += ['ark::']
                elif name == 'lib':argv += ['--lib']
                else:argv += ['--test', TARGETS[name]]
                prefix = 'wallet::unit_' if package == 'spc-agent' else 'ark::board::unit_' if package == 'spc-pool' else 'unit_'
                text = rust_log(count, prefix)
                cases = rust_cases(text)
            path = base / (name + '.log');path.write_text(text)
            ref = {'path': str(path), 'sha256': sha(path)}
            check = {'name': name, 'exit': 0, 'source_manifest_sha256': source_ref['sha256'], 'log': ref}
            if name in ('agent-ark-tests', 'pool-ark-tests', 'ui-browser-tests'):
                check.update(purpose=name, argv=argv)
            suite = {'expected_passed': count, 'log': ref, 'argv': argv, 'cwd_relative': cwd, 'cases': cases, 'ignored_cases': {}}
            if name == 'ui-browser-tests':
                suite['proof'] = self.write('browser-proof.json', {'checks': cases})
                check.update(tested_binary_sha256=binaries['lab']['sha256'], required_cases=cases, passed_cases=cases)
            suites[name] = suite
            checks.append(check)
        for name, package, binary in (('agent-ark-build', 'spc-agent', 'agent'), ('pool-ark-build', 'spc-pool', 'pool'), ('ui-build', 'spc-lab-cli', 'lab')):
            argv = ['cargo', 'build', '-p', package] + ([] if binary == 'lab' else ['--features', 'ark'])
            checks.append({'name': name, 'purpose': name, 'exit': 0, 'source_manifest_sha256': source_ref['sha256'],
                           'argv': argv, 'binary_sha256': binaries[binary]['sha256'], 'log': suites['lib']['log']})
        self.validation = {'checks': checks, 'ui_source_files': {name: self.source_files[name] for name in UI_FILES}}
        self.bundle['joined_validation_receipt'] = self.write('validation.json', self.validation)
        evidence = [suite.get('proof', suite['log']) for suite in suites.values()]
        self.gate = {'schema': 'spc.lab.ap5.joined-350-gate/v1', 'source_manifest_sha256': source_ref['sha256'],
                     'checks_sha256': hashlib.sha256(encoded(checks)).hexdigest(), 'passed': 350, 'failed': 0, 'evidence': evidence}
        self.q = {'schema': 'spc.lab.ap5.joined-inventory-qualification/v1', 'gate_schema': self.gate['schema'],
                  'source_root': self.source_id, 'source_manifest': source_ref, 'validation': self.bundle['joined_validation_receipt'],
                  'gate': self.write('gate.json', self.gate), 'source_files': self.source_files,
                  'app_binaries': {k: v['sha256'] for k, v in binaries.items()}, 'check_inventory': {c['name']: c for c in checks},
                  'suites': suites, 'command_source': self.write('unit-command-source.json', {'unit_fixture': True}),
                  'p2p_test_scope': 'fixture-rendered-argv-only', 'live_propagation_proved': False}

    def write(self, name, value):
        path = self.base / name;path.write_bytes(encoded(value))
        return {'path': str(path), 'sha256': sha(path)}

    def check(self, gate=None, qualification=None):
        return validate_completed_350(gate or self.gate, self.validation, self.bundle, self.root, self.manifest,
                                      self.source_id, qualification or self.q)


class QualifiedInventory(unittest.TestCase):
    def test_historical_332_cannot_be_used_for_a_new_unqualified_source(self):
        check_historical_332_source({'source_manifest_sha256': '1e3b3ddae2e19e9dfee66150e5fd7adfc02c46649a479e8b8ce5701540f3175f'},
                                    '/nix/store/11icfby7ni0185p093hxmbsahyl0p2dl-source')
        with self.assertRaisesRegex(Blocked, 'qualified historical source'):
            check_historical_332_source({'source_manifest_sha256': '00' * 32}, '/nix/store/new-fix-source')
    def test_complete_synthetic_inventory_is_not_live_propagation(self):
        with tempfile.TemporaryDirectory(prefix='ap5-inventory-', dir='/tmp/opencode') as tmp:
            f = InventoryFixture(Path(tmp))
            result = f.check()
            self.assertEqual(result['suite_counts'], MATRIX)
            self.assertFalse(result['live_propagation_proved'])
            self.assertFalse(result['funded_acceptance'])
            # The production registry is unqualified: fixture parsing is not
            # canonical admission of the still-running producer's gate.
            with self.assertRaisesRegex(Blocked, 'completed source-qualified 350'):
                check_qualified_350(f.gate, f.validation, f.bundle, f.root, f.manifest, f.source_id)

    def test_counter_only_truncated_duplicate_and_skipped_rust_logs_fail(self):
        logs = ['test result: ok. 350 passed; 0 failed; 0 ignored;\n', rust_log(2, 'case_').replace('2 passed', '350 passed'),
                'running 1 test\ntest case_0 ... ok\n', rust_log(2, 'case_').replace('case_1', 'case_0'),
                rust_log(2, 'case_').replace('case_1 ... ok', 'case_1 ... ignored')]
        for text in logs:
            with self.assertRaises(Blocked):rust_cases(text)

    def test_node_counter_without_executed_cases_and_replaced_summary_fail(self):
        for text in ('ℹ tests 36\nℹ pass 36\nℹ fail 0\n', node_log(3).replace('pass 3', 'pass 36')):
            with self.assertRaises(Blocked):node_cases(text)

    def test_multiple_actual_runner_groups_keep_case_identity(self):
        self.assertEqual(rust_cases(rust_log(1, 'lib_') + rust_log(1, 'integration_')), ['0:lib_0', '1:integration_0'])
        tap = ''.join(f'TAP version 13\nok 1 - file-{n}\n1..1\n# tests 1\n# pass 1\n# fail 0\n# skipped 0\n' for n in range(3))
        self.assertEqual(node_cases(tap), ['0:file-0', '1:file-1', '2:file-2'])

    def test_explicit_ignored_generator_is_not_counted_as_passed(self):
        text = 'running 2 tests\ntest normal ... ok\ntest regenerate_fixture ... ignored, fixture generation\ntest result: ok. 1 passed; 0 failed; 1 ignored;\n'
        self.assertEqual(rust_cases(text, {'0:regenerate_fixture': 'ignored, fixture generation'}), ['0:normal'])
        with self.assertRaises(Blocked):rust_cases(text)

    def test_missing_suites_or_substituted_logs_fail_even_with_total_350(self):
        with tempfile.TemporaryDirectory(prefix='ap5-inventory-', dir='/tmp/opencode') as tmp:
            f = InventoryFixture(Path(tmp))
            q = copy.deepcopy(f.q);del q['suites']['node-p2p-wiring']
            with self.assertRaises(Blocked):f.check(qualification=q)
            q = copy.deepcopy(f.q);q['suites']['reap-cleanup']['log'] = q['suites']['node-p2p-wiring']['log']
            with self.assertRaises(Blocked):f.check(qualification=q)
            q = copy.deepcopy(f.q);q['check_inventory'].pop('lib')
            with self.assertRaises(Blocked):f.check(qualification=q)

    def test_source_binary_command_and_fix_row_substitution_fail(self):
        with tempfile.TemporaryDirectory(prefix='ap5-inventory-', dir='/tmp/opencode') as tmp:
            f = InventoryFixture(Path(tmp))
            q = copy.deepcopy(f.q);q['source_root'] = '/nix/store/old-broken-source'
            with self.assertRaises(Blocked):f.check(qualification=q)
            q = copy.deepcopy(f.q);q['app_binaries']['lab'] = '00' * 32
            with self.assertRaises(Blocked):f.check(qualification=q)
            q = copy.deepcopy(f.q);q['suites']['reap-cleanup']['argv'][-1] = 'node_p2p_wiring'
            with self.assertRaises(Blocked):f.check(qualification=q)
            (f.root / 'crates/spc-lab/src/engine/traffic_runtime.rs').write_text('old no-peer gate')
            with self.assertRaises(Blocked):f.check()

    def test_self_consistent_rehashed_counterfeit_still_fails_case_recount(self):
        with tempfile.TemporaryDirectory(prefix='ap5-inventory-', dir='/tmp/opencode') as tmp:
            f = InventoryFixture(Path(tmp))
            old = dict(f.q['suites']['lib']['log'])
            path = Path(old['path'])
            path.write_text(path.read_text().replace('running 84 tests', 'running 85 tests').replace('84 passed', '85 passed'))
            new = {'path': str(path), 'sha256': sha(path)}
            for check in f.validation['checks']:
                if check['log'] == old:check['log'] = new
            f.q['suites']['lib']['log'] = new
            f.q['check_inventory'] = {c['name']: c for c in f.validation['checks']}
            f.bundle['joined_validation_receipt'] = f.write('validation.json', f.validation)
            f.q['validation'] = f.bundle['joined_validation_receipt']
            f.gate['checks_sha256'] = hashlib.sha256(encoded(f.validation['checks'])).hexdigest()
            f.gate['evidence'] = [new if r == old else r for r in f.gate['evidence']]
            f.q['gate'] = f.write('gate.json', f.gate)
            with self.assertRaisesRegex(Blocked, 'summary differs'):
                f.check()

    def test_fixture_p2p_tests_cannot_be_relabelled_live(self):
        with tempfile.TemporaryDirectory(prefix='ap5-inventory-', dir='/tmp/opencode') as tmp:
            f = InventoryFixture(Path(tmp));q = copy.deepcopy(f.q);q['live_propagation_proved'] = True
            with self.assertRaisesRegex(Blocked, 'actual block propagation'):
                f.check(qualification=q)
