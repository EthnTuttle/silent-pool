#!/usr/bin/env python3
"""Repository/Nix-derived AP5 packaging and canonical full-bundle generation.

This never invents a test pass. gate runs the committed plan, in the qualified
Nix install-check phase; assemble verifies those immutable outputs and then
runs the existing canonical admission validator. No historical run is input.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import os

from common import (artifact, check_bundle, encoded, fresh_root, load, require, sha,
                    probe_invocation, POOL_CONTROL_R4_FILES, POOL_CONTROL_R4_REVIEW)
from gate_inventory import rust_cases, node_cases
from joined_checks import UI_FILES, cargo_command, validate_joined_features
from portable_verifier import nix_source, descriptor as verifier_descriptor
from source_package import package_qa, qa_manifest

CONTRACT = 'testing/lab-ark-funded/source-contract.json'
AGGREGATE_LINEAGE = 'testing/lab-ark-funded/aggregate-source-lineage.json'
APPS = {'lab': 'spc-lab', 'txd': 'spc-lab-txd', 'pool': 'spc-pool', 'agent': 'spc-agent',
        'miner': 'spc-cpu-miner', 'scanner': 'spc-scan'}
PURPOSES = {'agent-ark-tests', 'agent-ark-build', 'pool-ark-tests', 'pool-ark-build',
            'ui-build', 'ui-browser-tests', 'readiness-tests', 'native-gate-tests', 'ladder-tests', 'qa-consumer-tests'}
CAPABILITIES = {'single-hash-budget', 'host-peering', 'fresh-registration', 'readiness',
                'pool-control', 'aggregate-history', 'native-audit'}
VERIFIER_STATIC = {
    'verifier-fmt': ['cargo', 'fmt', '--all', '--', '--check'],
    'verifier-clippy': ['cargo', 'clippy', '--offline', '--locked', '--all-targets', '--', '-D', 'warnings'],
}
CAPABILITY_PURPOSES = {
    'single-hash-budget': {'native-gate-tests', 'readiness-tests'},
    'host-peering': {'readiness-tests'}, 'fresh-registration': {'agent-ark-tests'},
    'readiness': {'readiness-tests'}, 'pool-control': {'pool-ark-tests', 'pool-ark-build'},
    'aggregate-history': {'ladder-tests'}, 'native-audit': {'native-gate-tests', 'qa-consumer-tests'},
}
UNION_PATHS = {
    'bins/spc-pool/src/ark/send_fence.rs', 'bins/spc-cpu-miner/src/native_gate.rs',
    'bins/spc-cpu-miner/src/connection/outbound.rs', 'crates/spc-store/src/economic.rs',
    'crates/spc-store/src/aggregate_store.rs', 'crates/spc-lab-txd/src/net_readiness.rs',
    'crates/spc-store/migrations/0007_ark_aggregate_membership.sql',
    'crates/spc-store/migrations/0008_source_accounting_history.sql',
    'crates/spc-store/migrations/0009_pending_net_debit_reservations.sql',
    'crates/spc-store/migrations/0010_aggregate_receipt_boundaries.sql',
    'crates/spc-store/migrations/0011_economic_history_checkpoint.sql',
} | set(POOL_CONTROL_R4_FILES)


def ref(path):
    path = Path(path).resolve(strict=True)
    return {'path': str(path), 'sha256': sha(path)}


def write(path, value):
    with Path(path).open('xb') as stream:
        stream.write(encoded(value) + b'\n')


def excluded(name):
    p = Path(name)
    return any(v in {'.git', 'target', '.direnv', '.cache', '__pycache__', 'node_modules'} or v.startswith('result') for v in p.parts) or name.startswith(('spc_ark_client/test/', 'spc_ark_e2e/test/'))


def source_rows(root):
    root = Path(root).resolve(strict=True)
    rows = []
    for path in sorted(root.rglob('*')):
        name = str(path.relative_to(root))
        if excluded(name):
            continue
        require(not path.is_symlink(), 'indirect source path: ' + name)
        if path.is_file():
            rows.append({'path': name, 'sha256': sha(path), 'bytes': path.stat().st_size})
    return rows


def committed_rows(root):
    """Read HEAD objects, never the index. Refuse dirty/missing tracked sources."""
    root = Path(root).resolve(strict=True)
    raw = subprocess.check_output(['git', '-C', str(root), 'ls-tree', '-rz', '--full-tree', 'HEAD'])
    rows = []
    for entry in raw.split(b'\0'):
        if not entry:
            continue
        meta, name = entry.split(b'\t', 1)
        mode, kind, oid = meta.decode().split(); name = name.decode()
        if excluded(name) or name == CONTRACT:
            continue
        require(kind == 'blob' and mode in {'100644', '100755'}, 'committed regular files required: ' + name)
        path = root / name
        require(path.is_file() and not path.is_symlink(), 'committed source missing/indirect: ' + name)
        data = path.read_bytes(); framed = b'blob ' + str(len(data)).encode() + b'\0' + data
        digest = hashlib.sha1(framed).hexdigest() if len(oid) == 40 else hashlib.sha256(framed).hexdigest()
        require(digest == oid, 'working source differs from HEAD: ' + name)
        rows.append({'path': name, 'sha256': hashlib.sha256(data).hexdigest(), 'bytes': len(data)})
    return sorted(rows, key=lambda r: r['path'])


def validate_plan(plan):
    require(type(plan) is dict and set(plan) == {'checks', 'capabilities'}, 'gate plan closed shape')
    checks = plan['checks']
    require(type(checks) is list and checks and len({c['name'] for c in checks}) == len(checks), 'complete unique gate plan required')
    require(PURPOSES | set(VERIFIER_STATIC) <= {c['purpose'] for c in checks}, 'gate plan omits required feature/proof suites')
    names = {c['name'] for c in checks}
    require(set(plan['capabilities']) == CAPABILITIES and all(type(v) is list and v and set(v) <= names for v in plan['capabilities'].values()),
            'each capability needs actual source-bound checks, not a boolean')
    purposes = {c['name']: c['purpose'] for c in checks}
    require(all(required <= {purposes[n] for n in plan['capabilities'][cap]} for cap, required in CAPABILITY_PURPOSES.items()),
            'capability cites unrelated check purposes')
    for c in checks:
        require(set(c) == {'name', 'purpose', 'argv', 'cwd', 'env', 'runner', 'cases', 'ignored_cases'}, 'closed gate step')
        require(re.fullmatch(r'[a-z0-9-]+', c['name']) and type(c['argv']) is list and len(c['argv']) >= 2
                and all(type(v) is str for v in c['argv']), 'gate command/name shape')
        require(not Path(c['cwd']).is_absolute() and '..' not in Path(c['cwd']).parts, 'gate working directory escape')
        require(c['runner'] in {'build', 'aux-build', 'command', 'rust', 'node', 'browser', 'unittest'}, 'gate runner classification')
        expected_program = {'build': 'cargo', 'aux-build': 'cargo', 'command': 'cargo', 'rust': 'cargo', 'node': 'node', 'browser': 'node', 'unittest': 'python3'}[c['runner']]
        require(c['argv'][0] == expected_program and type(c['env']) is dict and 'PATH' not in c['env']
                and all(type(k) is str and type(v) is str for k, v in c['env'].items()), 'gate must use its declared Nix tool')
        require(not any(marker in value for value in c['argv'] + list(c['env'].values())
                        for marker in ('/home/', '/tmp/opencode', '/nix/store/')), 'gate plan contains a workstation/store-instance path')
        if c['runner'] in {'build', 'rust'}:
            require(c['argv'][1] == ('build' if c['runner'] == 'build' else 'test')
                    and not any(v in c['argv'] for v in ('--no-run', '--list', '--ignored', '--exclude')), 'gate did not execute declared Cargo action')
        if c['runner'] == 'build':require('--release' in c['argv'], 'installed release build binding required')
        if c['runner'] == 'aux-build':
            require(c['purpose'] not in PURPOSES and c['argv'][1] in {'build', 'test'}
                    and '--message-format=json' in c['argv'], 'auxiliary compilation is not a required test pass')
        if c['runner'] == 'command':
            require(c['purpose'] not in PURPOSES and c['argv'][1] in {'fmt', 'clippy'}, 'static gate command not permitted')
        if c['purpose'] in VERIFIER_STATIC:
            require(c['runner'] == 'command' and c['cwd'] == 'testing/lab-ark-funded/verifier'
                    and c['argv'] == VERIFIER_STATIC[c['purpose']], 'verifier static gate must check the exact public verifier crate')
        if c['runner'] == 'unittest':
            require('-m' in c['argv'] and c['argv'][c['argv'].index('-m') + 1] == 'unittest', 'QA gate must execute unittest')
        require(type(c['cases']) is list and all(type(v) is str and v for v in c['cases']) and len(c['cases']) == len(set(c['cases']))
                and (c['runner'] in {'build', 'aux-build', 'command'} or c['cases']), 'actual expected case inventory required')
        require(type(c['ignored_cases']) is dict, 'explicit ignored-case inventory required')
        if c['purpose'].startswith(('agent-ark-', 'pool-ark-')):
            package = 'spc-agent' if c['purpose'].startswith('agent') else 'spc-pool'
            cargo_command(c['argv'], 'build' if c['runner'] == 'build' else 'test', package, ark=True)
        if c['purpose'] == 'ui-build':cargo_command(c['argv'], 'build', 'spc-lab-cli')
        if c['purpose'] == 'native-gate-tests':cargo_command(c['argv'], 'test', 'spc-cpu-miner')
        if c['purpose'] == 'readiness-tests':
            require(c['runner'] == 'rust' and any(p in c['argv'] for p in ('spc-lab', 'spc-lab-txd')), 'readiness must test actual lab/txd source')
        if c['purpose'] == 'ladder-tests':
            require(c['runner'] == 'rust' and any(p in c['argv'] for p in ('spc-pool', 'spc-store', 'spc-accounting')), 'ladder must test pool/store/accounting source')
        if c['purpose'] == 'qa-consumer-tests':require(c['runner'] == 'unittest', 'QA consumer test runner required')


def make_contract(repo, plan_path, output):
    plan = load(plan_path); validate_plan(plan)
    rows = committed_rows(repo); files = {r['path']: r['sha256'] for r in rows}
    require(UNION_PATHS <= set(files), 'committed source omits required cumulative union modules')
    relative_plan = str(Path(plan_path).resolve().relative_to(Path(repo).resolve()))
    require(relative_plan in files and sha(plan_path) == files[relative_plan], 'gate plan must already be committed')
    require(AGGREGATE_LINEAGE in files and sha(Path(repo) / AGGREGATE_LINEAGE) == files[AGGREGATE_LINEAGE], 'aggregate lineage must already be committed')
    lineage = load(Path(repo) / AGGREGATE_LINEAGE)
    check_lineage(lineage, files)
    write(output, {'schema': 'spc.lab.ap5.committed-source-contract/v1', 'state': 'source-bound-not-tested',
                   'files': files, 'gate_plan': relative_plan, 'gate_plan_sha256': files[relative_plan],
                   'aggregate_source_snapshot_sha256': lineage['source_snapshot_sha256'],
                   'aggregate_files': sorted(n for n in files if n.startswith(('crates/spc-store/', 'crates/spc-accounting/', 'bins/spc-pool/')))})


def check_lineage(lineage, files):
    require(type(lineage) is dict and set(lineage) == {'schema', 'source_snapshot_sha256', 'files'}
            and lineage['schema'] == 'spc.lab.ap5.aggregate-source-lineage/v1', 'aggregate source lineage shape')
    value = lineage['source_snapshot_sha256']
    require(type(value) is str and re.fullmatch(r'[0-9a-f]{64}', value)
            and value != '415d21fa88ad2124712cc1fe9a8400bc7fbdd951bd005f223f92f879068f2602',
            'actual reviewed aggregate source lineage is not supplied')
    required = {n for n in UNION_PATHS if n.startswith('crates/spc-store/') or n.endswith('send_fence.rs')}
    require(required <= set(lineage['files']) and all(files.get(n) == h for n, h in lineage['files'].items()),
            'aggregate lineage does not bind the actual complete reviewed core')


def check_contract(source):
    source = Path(source)
    contract = load(source / CONTRACT)
    require(contract.get('schema') == 'spc.lab.ap5.committed-source-contract/v1'
            and contract.get('state') == 'source-bound-not-tested', 'complete committed source contract not yet supplied')
    rows = source_rows(source); files = {r['path']: r['sha256'] for r in rows if r['path'] != CONTRACT}
    require(files == contract['files'] and UNION_PATHS <= set(files), 'actual Nix source differs from the committed contract')
    lineage = load(source / AGGREGATE_LINEAGE); check_lineage(lineage, files)
    require(contract['aggregate_source_snapshot_sha256'] == lineage['source_snapshot_sha256'], 'aggregate source lineage changed')
    plan_path = source / contract['gate_plan']
    require(str(plan_path.resolve()).startswith(str(source.resolve()) + '/') and sha(plan_path) == contract['gate_plan_sha256'], 'source-owned gate plan changed')
    plan = load(plan_path); validate_plan(plan)
    return contract, plan, rows


def executed_cases(step, text, directory):
    if step['runner'] in {'build', 'aux-build', 'command'}:return []
    if step['runner'] == 'rust':return sorted(rust_cases(text, step['ignored_cases']))
    if step['runner'] == 'node':return sorted(node_cases(text))
    if step['runner'] == 'browser':
        proof = load(directory / 'browser/proof.json')
        require(type(proof['checks']) is list, 'actual browser proof cases missing')
        return sorted(proof['checks'])
    matches = re.findall(r'^(test_\S+ \([^\n]+\)) \.\.\. ok$', text, re.M)
    total = re.search(r'^Ran (\d+) tests? in ', text, re.M)
    require(total and int(total[1]) == len(matches) and text.rstrip().endswith('OK'), 'complete unittest runner output required')
    return sorted(matches)


def run_gate(source, installed, output):
    """Only invoked in Nix installCheckPhase, not by an admission label."""
    source, installed = Path(source).resolve(), Path(installed).resolve()
    contract, plan, rows = check_contract(source)
    root = fresh_root(output); write(root / 'source-manifest.json', {'files': rows})
    manifest = ref(root / 'source-manifest.json'); checks = []; built = {}
    for step in plan['checks']:
        directory = root / step['name']; directory.mkdir()
        def expand(v):
            if v.startswith('@artifact/'):
                require(v in built, 'compiled helper artifact not supplied by an earlier actual build: ' + v)
                return built[v]
            return v.replace('@installed', str(installed)).replace('@source', str(source)).replace('@build', str(Path.cwd())).replace('@evidence', str(directory))
        argv = [expand(v) for v in step['argv']]
        env = {**os.environ, **{k: expand(v) for k, v in step['env'].items()}, 'PYTHONDONTWRITEBYTECODE': '1'}
        program = Path(shutil.which(argv[0], path=env.get('PATH', '')) or '')
        require(program.is_absolute() and program.resolve().is_relative_to('/nix/store'), 'gate tool is not from the Nix closure')
        if step['runner'] == 'browser':
            env['LIVE_UI_CLI_BIN'] = str(installed / 'bin/spc-lab')
            env['LIVE_UI_COMPILED_PROOF'] = str(directory / 'browser')
            require(env.get('CHROME', '').startswith('/nix/store/'), 'browser must be the Nix-built tool')
        result = subprocess.run(argv, cwd=Path.cwd() / step['cwd'], env=env, capture_output=True)
        log = directory / 'output.log'; log.write_bytes(result.stdout + result.stderr)
        require(result.returncode == 0, 'actual gate step failed: ' + step['name'])
        if step['runner'] == 'aux-build':
            for line in result.stdout.splitlines():
                event = json.loads(line)
                if event.get('reason') == 'compiler-artifact' and event.get('executable'):
                    path = Path(event['executable']).resolve(strict=True)
                    require(path.is_relative_to(Path.cwd().resolve()), 'compiled helper escapes build directory')
                    key = '@artifact/' + step['name'] + '/' + event['target']['name']
                    require(key not in built or built[key] == str(path), 'ambiguous compiled helper artifact')
                    built[key] = str(path)
        cases = executed_cases(step, log.read_text(), directory)
        require(cases == sorted(step['cases']), 'actual gate case inventory changed: ' + step['name'])
        record = {'name': step['name'], 'purpose': step['purpose'], 'exit': result.returncode,
                  'argv': argv, 'source_manifest_sha256': manifest['sha256'], 'log': ref(log)}
        record['execution_program'] = ref(program)
        if step['purpose'] in {'agent-ark-build', 'pool-ark-build', 'ui-build'}:
            binary = {'agent-ark-build': 'spc-agent', 'pool-ark-build': 'spc-pool', 'ui-build': 'spc-lab'}[step['purpose']]
            record['binary_sha256'] = sha(installed / 'bin' / binary)
        if step['runner'] == 'browser':
            record.update(tested_binary_sha256=sha(installed / 'bin/spc-lab'), required_cases=cases, passed_cases=cases)
        record['executed_cases'] = cases; checks.append(record)
    validation = {'schema': 'spc.lab.ap5.join-validation/v1', 'source_manifest_sha256': manifest['sha256'],
                  'binaries': {n: sha(installed / 'bin' / p) for n, p in APPS.items()}, 'checks': checks,
                  'ui_source_files': {n: sha(source / n) for n in UI_FILES}}
    write(root / 'joined-validation.json', validation)
    write(root / 'tested-capabilities.json', {'schema': 'spc.lab.ap5.tested-capabilities/v1',
          'source_manifest_sha256': manifest['sha256'], 'contract_sha256': sha(source / CONTRACT),
          'capabilities': plan['capabilities'], 'checks_sha256': hashlib.sha256(encoded(checks)).hexdigest()})


def assemble(outputs_file, output):
    """Produce the EXISTING canonical envelope using freshly qualified Nix data."""
    outputs = load(outputs_file)
    require(outputs['schema'] == 'spc.lab.ap5.nix-outputs/v1', 'Nix output inventory schema')
    binaries = {n: ref(p) for n, p in outputs['binaries'].items()}
    package, _, source, env = nix_source(binaries['lab']['path'])
    require(env.get('doInstallCheck') in ('1', True), 'joined build did not execute the source-bound qualification plan')
    contract, plan, rows = check_contract(source)
    qualified = package / 'share/ap5-qualification'
    manifest_ref = ref(qualified / 'source-manifest.json'); validation_ref = ref(qualified / 'joined-validation.json')
    require(load(Path(manifest_ref['path'])) == {'files': rows}, 'qualified source census changed')
    validation = load(Path(validation_ref['path'])); caps = load(qualified / 'tested-capabilities.json')
    require(caps['source_manifest_sha256'] == manifest_ref['sha256'] and caps['contract_sha256'] == sha(source / CONTRACT)
            and caps['capabilities'] == plan['capabilities'] and caps['checks_sha256'] == hashlib.sha256(encoded(validation['checks'])).hexdigest(),
            'actual source/check/capability binding differs')
    require([c['name'] for c in validation['checks']] == [c['name'] for c in plan['checks']], 'qualified check census changed')
    for step, check in zip(plan['checks'], validation['checks']):
        require(check['exit'] == 0 and check['source_manifest_sha256'] == manifest_ref['sha256'], 'uncompleted qualification')
        text = artifact(check['log']).read_text()
        artifact(check['execution_program'])
        require(executed_cases(step, text, Path(check['log']['path']).parent) == sorted(step['cases']), 'qualification log no longer matches actual case inventory')
    root = fresh_root(output)
    files = {r['path']: r['sha256'] for r in rows}; pool = {n: h for n, h in files.items() if n.startswith('bins/spc-pool/')}
    control = {'source_root': str(source), 'source_manifest_sha256': manifest_ref['sha256'], 'compiled': True, 'tests_passed': True,
               'source_snapshot_sha256': 'd00299c259aaa5e8098542f48f5c3c90c66737be3f965f5d65e5fd2a763914b2',
               'tested_code_snapshot_sha256': 'f764900419db45901f558c367b5485938ed1b20b5ee5abdc039e3b66c230ae0c',
               'same_auditor_review': ref(source / 'testing/lab-ark-funded/provenance/pool-control-r4-review.md'),
               'files': pool, 'checks': [c for c in validation['checks'] if c['purpose'].startswith('pool-ark-')]}
    require(control['same_auditor_review']['sha256'] == POOL_CONTROL_R4_REVIEW['sha256']
            and all(pool.get(n) == h for n, h in POOL_CONTROL_R4_FILES.items()), 'actual control source lacks approved R4 binding')
    write(root / 'pool-control-source-policy.json', control)
    aggregate = {n: files[n] for n in contract['aggregate_files']}
    require(aggregate and UNION_PATHS.intersection(aggregate), 'aggregate source policy missing')
    write(root / 'aggregate-source-policy.json', {'compiled': True, 'tests_passed': True,
          'source_snapshot_sha256': contract['aggregate_source_snapshot_sha256'], 'files': aggregate,
          'checks': [c for c in validation['checks'] if c['purpose'] == 'ladder-tests']})
    wrapper = Path(binaries['probe']['path']); program, script = probe_invocation(wrapper)
    require(script is not None, 'canonical script probe required')
    share = wrapper.parent.parent / 'share'
    probe_source = ref(share / 'ark-rpc-source-manifest.json')
    probe_program = ref(share / 'ark-rpc-program-binding.json')
    from probe_package import validate_program_binding
    validate_program_binding(probe_program, probe_source, wrapper, program, script)
    bundle = {'schema': 'spc.lab.ap5.bundle/v1', 'scope': 'full', 'base_revision': outputs['base_revision'],
        'binaries': binaries, 'postgres_bin_dir': outputs['postgres_bin_dir'], 'runtime_script': outputs['runtime_script'],
        'joined_source_manifest': manifest_ref, 'joined_validation_receipt': validation_ref,
        'pool_control_source_policy': ref(root / 'pool-control-source-policy.json'),
        'aggregate_source_policy': ref(root / 'aggregate-source-policy.json'),
        'pool_wallet_executor': ref(outputs['pool_wallet_executor']), 'vtxo_verifier': verifier_descriptor(outputs['vtxo_verifier'], root / 'verifier'),
        'history_version': 3, 'bootstrap_and_workers_single_budget': True, 'host_peering_before_profile_binding': True,
        'fresh_registration_before_workers': True, 'probe_invocation': {'schema': 'spc.lab.ap5.probe-invocation/v1',
            'wrapper': binaries['probe'], 'program': ref(program), 'script': ref(script), 'prefix_args': [str(script)],
            'runtime_subcommand': 'node-probe', 'source_manifest': probe_source, 'program_binding': probe_program}}
    validate_joined_features(validation, bundle, source, {'files': rows})
    write(root / 'bundle.json', bundle)
    check_bundle(bundle, full=True)
    write(root / 'admission.json', {'scope': 'source/test-qualified only, not live acceptance', 'bundle': ref(root / 'bundle.json')})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    for name in ('package-qa', 'inspect-qa', 'source-contract'):
        p = sub.add_parser(name); p.add_argument('--repo', type=Path, required=True); p.add_argument('--out', type=Path, required=True)
        if name == 'source-contract':p.add_argument('--plan', type=Path, required=True)
    p = sub.add_parser('gate'); p.add_argument('--source', type=Path, required=True); p.add_argument('--installed', type=Path, required=True); p.add_argument('--out', type=Path, required=True)
    p = sub.add_parser('assemble'); p.add_argument('--outputs', type=Path, required=True); p.add_argument('--out', type=Path, required=True)
    p = sub.add_parser('check-contract'); p.add_argument('--source', type=Path, required=True)
    args = parser.parse_args()
    if args.command == 'package-qa':package_qa(args.repo, args.out)
    elif args.command == 'inspect-qa':
        manifest, raw = qa_manifest(args.repo, require_approved=False)
        write(args.out, {'status': 'CANDIDATE_NOT_ADMITTED', 'manifest': manifest, 'manifest_sha256': hashlib.sha256(raw).hexdigest()})
    elif args.command == 'source-contract':make_contract(args.repo, args.plan, args.out)
    elif args.command == 'gate':run_gate(args.source, args.installed, args.out)
    elif args.command == 'check-contract':check_contract(args.source)
    else:assemble(args.outputs, args.out)


if __name__ == '__main__':main()
