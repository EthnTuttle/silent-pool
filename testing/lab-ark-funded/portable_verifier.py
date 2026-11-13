"""Fresh Nix verifier admission from committed public sources and build checks."""
import json
from pathlib import Path
import subprocess

from common import artifact, encoded, fresh_root, require, sha
from gate_inventory import rust_cases
from nix_derivation import derivation_record

CRYPTO_CASES = {
    'vectors::synthetic_signed_board_positive_is_unconditional',
    'vectors::frozen_board_round_and_arkoor_full_signatures_validate',
    'vectors::same_u_in_valid_htlc_is_not_full_pubkey_policy',
    'vectors::foreign_owner_and_server_are_rejected',
    'vectors::first_and_terminal_input_signature_corruption_is_not_just_decode_failure',
    'vectors::wrong_anchor_input_and_output_and_truncated_proof_reject',
}
SCHEMA = 'spc.lab.ap5.verifier-reproduction/v1'
POLICY_SHA = 'a9c36c674236a32d82140cfa918837e44bc4cabdce7478a7a6045959b9606bdc'
BINARY_SHA = '5330c3ce8eaa65a4b96076badbeae420f654c33c2a611927932fab49bb8357dd'
LOCK_SHA = 'ac24ee3b09c7c2dd8731417672b9d7b072aa6e9e04127b9c96e8ad80ae842833'


def nix_source(program):
    program = Path(program).resolve(strict=True)
    require(program.is_relative_to('/nix/store'), 'installed Nix program required')
    package = Path('/nix/store') / program.relative_to('/nix/store').parts[0]
    drv = subprocess.check_output(['nix-store', '-q', '--deriver', str(package)], text=True).strip()
    require(drv.endswith('.drv'), 'actual Nix deriver unavailable')
    data = derivation_record(subprocess.check_output(['nix', '--extra-experimental-features', 'nix-command', 'derivation', 'show', drv], text=True), drv)
    require(any(v.get('path') == str(package) for v in data['outputs'].values()), 'actual Nix output differs from program package')
    env = data['env']
    env = json.loads(env['__json']) if '__json' in env else env
    source = Path(env['src'])
    require(source.is_relative_to('/nix/store') and source.is_dir(), 'actual immutable Nix source required')
    return package, drv, source, env


def inspect(program):
    package, drv, source, env = nix_source(program)
    policy_file = Path(__file__).with_name('verifier-source-policy.json')
    require(sha(policy_file) == POLICY_SHA, 'approved public verifier source policy changed')
    policy = json.loads(policy_file.read_text())
    rows = policy['files']; names = {row['path'] for row in rows}
    require(len(rows) == len(names) == 9 and {str(p.relative_to(source)) for p in source.rglob('*') if p.is_file()} == names,
            'verifier exact nine-file source census')
    for row in rows:
        path = source / row['path']
        require(not path.is_symlink() and sha(path) == row['sha256'] and path.stat().st_size == row['bytes']
                and bool(path.stat().st_mode & 0o111) == row['executable'], 'verifier actual public source/size/mode differs: ' + row['path'])
    binary = sha(program)
    require(binary == BINARY_SHA, 'rebuilt verifier bytes differ; source/build review required')
    require(env.get('doCheck') in ('1', True), 'verifier build omitted required checks')
    require(sha(source / 'Cargo.lock') == LOCK_SHA == sha(package / 'share/ap5-verifier/Cargo.lock'), 'verifier lock differs')
    log = subprocess.run(['nix', '--extra-experimental-features', 'nix-command', 'log', drv], capture_output=True, timeout=30)
    require(log.returncode == 0, 'actual Nix verifier test log unavailable; completed build/check required')
    validate_crypto_log(log.stdout.decode())
    return {'schema': 'spc.lab.ap5.verifier-nix-binding/v1', 'binary': {'path': str(Path(program).resolve()), 'sha256': binary},
            'cargo_lock_sha256': LOCK_SHA, 'deriver': drv, 'source': str(source), 'source_rows': rows}, log.stdout


def descriptor(program, output):
    from verifier_cli_check import check
    binding, log = inspect(program)
    root = fresh_root(output)
    (root / 'nix-test.log').write_bytes(log)
    check(program, Path(binding['source']) / 'fixtures/stock-public.json', root / 'cli-checks.json')
    binding['cli_checks'] = {'path': str(root / 'cli-checks.json'), 'sha256': sha(root / 'cli-checks.json')}
    (root / 'nix-binding.json').write_bytes(encoded(binding) + b'\n')
    return {'schema': SCHEMA, **binding['binary'],
            'nix_binding': {'path': str(root / 'nix-binding.json'), 'sha256': sha(root / 'nix-binding.json')},
            'test_log': {'path': str(root / 'nix-test.log'), 'sha256': sha(root / 'nix-test.log')}}


def validate_crypto_log(text):
    cases = rust_cases(text)
    names = {case.split(':', 1)[1] for case in cases}
    require(len(cases) == 6 and names == CRYPTO_CASES, 'six exact unconditional crypto cases required')


def admit(record):
    from verifier_cli_check import check
    require(set(record) == {'schema', 'path', 'sha256', 'nix_binding', 'test_log'} and record['schema'] == SCHEMA,
            'reproduced verifier closed descriptor')
    program = artifact(record); expected, actual_log = inspect(program)
    binding = json.loads(artifact(record['nix_binding']).read_text())
    require(set(binding) == set(expected) | {'cli_checks'}
            and all(encoded(binding[k]) == encoded(v) for k, v in expected.items()), 'reproduced verifier source/deriver binding differs')
    require(artifact(record['test_log']).read_bytes() == actual_log, 'reproduced verifier test log differs from actual Nix log')
    cli = json.loads(artifact(binding['cli_checks']).read_text())
    require(cli['schema'] == 'spc.lab.ap5.installed-verifier-checks/v1' and cli['binary_sha256'] == BINARY_SHA
            and len(cli['cases']) == 2 and {r['name'] for r in cli['cases']} == {'signed-board-positive', 'foreign-server-negative'}
            and all(r['expected_outcome_passed'] is True for r in cli['cases']), 'reproduced verifier CLI checks incomplete')
    require(encoded(cli) == encoded(check(program, Path(expected['source']) / 'fixtures/stock-public.json')),
            'reproduced verifier CLI receipt differs from actual public checks')
    return program
