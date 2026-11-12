"""Source-qualified joined-350 census. Counts alone never confer admission.

The qualification registry is intentionally empty until actual final producer
evidence is received. No production 350 pass is created by this preparation.
"""
import hashlib
import json
from pathlib import Path
import re

from common import artifact, encoded, load, require, verify_source
from joined_checks import cargo_command, validate_joined_features

MATRIX = {
    'agent-ark-tests': 162, 'pool-ark-tests': 31, 'lib': 84, 'node': 36,
    'ui-browser-tests': 15, 'command-idempotency': 6, 'command-concurrency': 1,
    'engine-restart': 3, 'traffic-payment-view': 1, 'node-p2p-wiring': 9, 'reap-cleanup': 2,
}
TARGETS = {'command-idempotency': 'command_idempotency', 'command-concurrency': 'command_concurrency',
           'engine-restart': 'engine_restart', 'traffic-payment-view': 'traffic_payment_view',
           'node-p2p-wiring': 'node_p2p_wiring', 'reap-cleanup': 'reap_cleanup'}
FIX_FILES = {'crates/spc-lab/src/engine.rs', 'crates/spc-lab/src/engine/traffic_runtime.rs',
             'crates/spc-lab/src/supervisor.rs', 'crates/spc-lab/src/evidence.rs',
             'crates/spc-lab/src/engine/reap_cleanup_tests.rs', 'crates/spc-lab/tests/node_p2p_wiring.rs',
             'crates/spc-lab/tests/reap_cleanup.rs', 'crates/spc-lab/tests/common/mod.rs',
             'crates/spc-lab-txd/src/native_bootstrap/tests.rs', 'bins/spc-lab/src/main.rs'}
GATE_FIELDS = {'schema', 'source_manifest_sha256', 'checks_sha256', 'passed', 'failed', 'evidence'}
QUALIFICATION_FIELDS = {'schema', 'gate_schema', 'source_root', 'source_manifest', 'validation', 'gate',
                        'app_binaries', 'source_files', 'check_inventory', 'suites', 'command_source',
                        'p2p_test_scope', 'live_propagation_proved'}
ANSI = re.compile(r'\x1b\[[0-?]*[ -/]*[@-~]|\x1b[()][A-Za-z0-9]')


def rust_cases(text, expected_ignored=None):
    """Require complete runner groups, actual case lines, no failures/skips."""
    text = ANSI.sub('', text)
    expected_ignored = expected_ignored or {}
    seen_ignored = {}
    cases, active, group, names, declared, passed, ignored = [], False, -1, set(), 0, 0, 0
    for line in text.splitlines():
        start = re.fullmatch(r'running (\d+) tests?', line.strip())
        if start:
            require(not active, 'truncated Rust test group')
            active, declared, names, passed, ignored = True, int(start[1]), set(), 0, 0
            group += 1
            continue
        case = re.fullmatch(r'test ([^ ]+) \.\.\. (.+)', line.strip())
        if case:
            require(active and case[1] not in names, 'failed/skipped/duplicate Rust case')
            names.add(case[1])
            identity = f'{group}:{case[1]}'
            if case[2] == 'ok':
                cases.append(identity)
                passed += 1
            else:
                require(identity in expected_ignored and case[2] == expected_ignored[identity], 'failed/skipped/duplicate Rust case')
                seen_ignored[identity] = case[2]
                ignored += 1
        if line.startswith('test result:'):
            summary = re.match(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;', line)
            require(active and summary is not None and int(summary[1]) == passed and len(names) == declared
                    and int(summary[2]) == 0 and int(summary[3]) == ignored, 'Rust summary differs from actual executed cases')
            active = False
    require(group >= 0 and not active, 'missing/truncated Rust test results')
    require(seen_ignored == expected_ignored, 'declared ignored-case inventory mismatch')
    return cases


def node_cases(text):
    """Flat Node test logs, TAP or spec reporter, with per-invocation totals."""
    text = ANSI.sub('', text)
    cases, group, names, summaries = [], 0, set(), {}
    def finish():
        require(summaries.get('tests') == summaries.get('pass') == len(names)
                and summaries.get('fail') == 0 and summaries.get('skipped', 0) == 0
                and summaries.get('cancelled', 0) == 0, 'Node summary differs from executed cases')
    for line in text.splitlines():
        match = re.match(r'^✔ (.+?)(?: \([0-9.]+m?s\))?$', line)
        tap = re.match(r'^ok \d+(?: -)? (.+)$', line)
        if match or tap:
            if 'fail' in summaries:
                finish()
                names, summaries, group = set(), {}, group + 1
            name = (match or tap)[1]
            require('# SKIP' not in name and '# TODO' not in name and name not in names, 'skipped/duplicate Node case')
            names.add(name)
            cases.append(f'{group}:{name}')
        summary = re.fullmatch(r'(?:ℹ|#) (tests|pass|fail|skipped|cancelled) (\d+)', line)
        if summary:
            require(summary[1] not in summaries, 'duplicate Node summary')
            summaries[summary[1]] = int(summary[2])
        require(not line.startswith(('not ok ', '✖ ')), 'failed Node case')
    finish()
    require(cases, 'Node log contains only an asserted counter')
    return cases


def suite_command(name, argv, cwd_relative='.'):
    require(not Path(cwd_relative).is_absolute() and '..' not in Path(cwd_relative).parts, 'suite cwd escapes source')
    if name in ('agent-ark-tests', 'pool-ark-tests'):
        require(cwd_relative == '.', 'Cargo suite cwd mismatch')
        package = 'spc-agent' if name == 'agent-ark-tests' else 'spc-pool'
        cargo_command(argv, 'test', package, ark=True)
        if name == 'pool-ark-tests':
            require('ark::' in argv, 'pool suite must select the declared Ark namespace')
    elif name == 'lib' or name in TARGETS:
        require(cwd_relative == '.', 'Cargo suite cwd mismatch')
        cargo_command(argv, 'test', 'spc-lab')
        if name == 'lib':
            require('--lib' in argv and '--test' not in argv, 'lib gate target mismatch')
        else:
            targets = [argv[i+1] for i, value in enumerate(argv[:-1]) if value == '--test']
            require(targets == [TARGETS[name]], 'Rust integration suite target substitution')
    elif name == 'ui-browser-tests':
        require(type(argv) is list and argv and Path(argv[0]).name == 'node', 'Node/browser command mismatch')
        require(len(argv) == 2 and str(Path(cwd_relative) / argv[1]) == 'bins/spc-lab/tests/compiled_browser.cjs', 'browser source suite substitution')
    else:
        require(name == 'node' and type(argv) is list and len(argv) == 3, 'Node suite needs its three actual invocations')
        expected = ['bins/spc-lab/tests/access.test.cjs', 'bins/spc-lab/tests/app_helpers.test.cjs', 'bins/spc-lab/tests/lab_helpers.test.cjs']
        require(all(type(command) is list and len(command) == 2 and Path(command[0]).name == 'node' for command in argv)
                and [str(Path(cwd_relative) / command[1]) for command in argv] == expected, 'Node source suite substitution')


def validate_completed_350(gate, validation, bundle, root, manifest, source_identity, qualification):
    """Qualification must come from the pinned registry, never bundle assertions."""
    require(set(gate) == GATE_FIELDS and gate['schema'] == 'spc.lab.ap5.joined-350-gate/v1'
            and type(gate['passed']) is int and gate['passed'] == sum(MATRIX.values())
            and type(gate['failed']) is int and gate['failed'] == 0, 'exact joined-350 gate shape/census')
    q = qualification
    require(type(q) is dict and set(q) == QUALIFICATION_FIELDS, 'closed gate source qualification shape')
    require(q['schema'] == 'spc.lab.ap5.joined-inventory-qualification/v1'
            and q['gate_schema'] == gate['schema'] and q['source_root'] == source_identity
            and q['source_manifest'] == bundle['joined_source_manifest']
            and q['validation'] == bundle['joined_validation_receipt'], 'gate inventory is qualified for a different source/root/validation')
    require(encoded(load(artifact(q['gate']))) == encoded(gate), 'gate differs from qualified completed artifact')
    artifact(q['command_source'])
    require(gate['source_manifest_sha256'] == bundle['joined_source_manifest']['sha256']
            and gate['checks_sha256'] == hashlib.sha256(encoded(validation['checks'])).hexdigest(), 'gate/check/source binding mismatch')
    require(q['app_binaries'] == {n: bundle['binaries'][n]['sha256'] for n in ('lab', 'txd', 'pool', 'agent', 'miner', 'scanner')}, 'qualified gate binary census mismatch')
    require(FIX_FILES <= set(q['source_files']), 'qualified inventory omits network/reap implementation or tests')
    require(set(q['source_files']) == {r['path'] for r in manifest['files']}, 'qualified inventory must bind the complete source manifest')
    verify_source(root, manifest, q['source_files'])
    checks = {c['name']: c for c in validation['checks']}
    require(len(checks) == len(validation['checks']) and set(checks) == set(q['check_inventory']), 'missing/extra/substituted completed gate check')
    for name, check in checks.items():
        require(encoded(check) == encoded(q['check_inventory'][name]) and type(check['exit']) is int and check['exit'] == 0
                and check['source_manifest_sha256'] == gate['source_manifest_sha256'], 'check is incomplete or differs from qualified source evidence')
        artifact(check['log'])
    require(set(q['suites']) == set(MATRIX), 'missing or extra declared test suite')
    evidence = {(r['path'], r['sha256']) for r in gate['evidence']}
    require(len(evidence) == len(gate['evidence']), 'duplicate joined test evidence')
    required_evidence, observed = set(), {}
    for name, expected_count in MATRIX.items():
        suite = q['suites'][name]
        check = checks[name]
        require(set(suite) == {'expected_passed', 'log', 'argv', 'cwd_relative', 'cases', 'ignored_cases'} | ({'proof'} if name == 'ui-browser-tests' else set()), 'closed suite inventory shape')
        allowed_ignore = {'8:regenerate_fixture': 'ignored, fixture generation; run explicitly to (re)create the committed file'} if name == 'agent-ark-tests' else {}
        require(type(suite['ignored_cases']) is dict and all(allowed_ignore.get(k) == v for k, v in suite['ignored_cases'].items()), 'unapproved skipped suite/case')
        if suite['ignored_cases']:
            declaration = (root / 'bins/spc-agent/tests/wallet_recovery.rs').read_text()
            require('#[ignore = "fixture generation; run explicitly to (re)create the committed file"]' in declaration
                    and 'fn regenerate_fixture()' in declaration, 'ignored generator is not source-bound')
        require(type(suite['expected_passed']) is int and suite['expected_passed'] == expected_count and suite['log'] == check['log'], 'suite counter/log mapping changed')
        suite_command(name, suite['argv'], suite['cwd_relative'])
        if 'argv' in check:
            require(check['argv'] == suite['argv'], 'check command differs from qualified execution')
        text = artifact(suite['log']).read_text()
        if name == 'ui-browser-tests':
            summary = json.loads(text)
            proof = load(artifact(suite['proof']))
            cases = proof['checks']
            require(type(summary['passed']) is int and summary['passed'] == expected_count
                    and all(type(case) is str and case for case in cases) and len(set(cases)) == len(cases)
                    and cases == check['required_cases'] == check['passed_cases']
                    and check['tested_binary_sha256'] == bundle['binaries']['lab']['sha256'], 'browser cases/binary differ from actual proof')
            required_evidence.add((suite['proof']['path'], suite['proof']['sha256']))
        else:
            cases = node_cases(text) if name == 'node' else rust_cases(text, suite['ignored_cases'])
            required_evidence.add((suite['log']['path'], suite['log']['sha256']))
        require(len(cases) == expected_count and cases == suite['cases'], 'executed cases differ from declared source-qualified suite inventory')
        observed[name] = len(cases)
    require(evidence == required_evidence, 'joined gate evidence is missing or substitutes suite logs')
    for value in gate['evidence']:
        artifact(value)
    validate_joined_features(validation, bundle, root, manifest)
    require(q['p2p_test_scope'] == 'fixture-rendered-argv-only' and q['live_propagation_proved'] is False,
            'rendered-argv tests cannot establish actual block propagation')
    return {'schema': gate['schema'], 'suite_counts': observed, 'source_qualified': True,
            'live_propagation_proved': False, 'funded_acceptance': False}


def qualification_registry():
    registry = load(Path(__file__).with_name('joined-gate-qualifications.json'))
    require(registry['schema'] == 'spc.lab.ap5.joined-gate-qualifications/v1', 'gate qualification registry schema')
    return registry


def check_historical_332_source(gate, source_identity):
    profiles = qualification_registry()['historical_332_sources']
    key = gate.get('source_manifest_sha256')
    require(key in profiles and profiles[key]['source_root'] == source_identity,
            '332 inventory belongs only to its qualified historical source')


def check_qualified_350(gate, validation, bundle, root, manifest, source_identity):
    registry = qualification_registry()
    key = gate['source_manifest_sha256']
    require(key in registry['qualified_350_sources'], 'completed source-qualified 350 gate receipts have not been supplied')
    qualification = load(artifact(registry['qualified_350_sources'][key]))
    return validate_completed_350(gate, validation, bundle, root, manifest, source_identity, qualification)
