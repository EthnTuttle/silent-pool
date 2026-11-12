"""Fresh-location packaging/admission negatives. No builds, actors or Git writes."""
import copy
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

from common import (Blocked, POOL_CONTROL_R4_FILES, POOL_CONTROL_R4_REVIEW,
                    POOL_CONTROL_STALL_COVERAGE_REVIEW,
                    sha, validate_pool_control_admission)
from portable import (CAPABILITIES, CAPABILITY_PURPOSES, PURPOSES, VERIFIER_STATIC, check_contract, check_lineage,
                      committed_rows, validate_plan)
from portable_verifier import CRYPTO_CASES, validate_crypto_log
from source_package import package_qa, qa_manifest, QA_MANIFEST_SHA256

HERE = Path(__file__).parent


def plan():
    checks = []
    for purpose in sorted(PURPOSES):
        package = 'spc-agent' if purpose.startswith('agent') else 'spc-pool'
        if purpose.endswith('build'):
            if purpose == 'ui-build':package = 'spc-lab-cli'
            argv = ['cargo', 'build', '--release', '-p', package, '--features', 'ark']
            runner, cases = 'build', []
        elif purpose == 'ui-browser-tests':
            argv, runner, cases = ['node', 'bins/spc-lab/tests/compiled_browser.cjs'], 'browser', ['unit-ui-case']
        elif purpose == 'qa-consumer-tests':
            argv, runner, cases = ['python3', '-B', '-m', 'unittest', 'test_unit', '-v'], 'unittest', ['test_unit (test_unit.Unit.test_unit)']
        else:
            if purpose == 'native-gate-tests':package = 'spc-cpu-miner'
            if purpose == 'readiness-tests':package = 'spc-lab'
            argv, runner, cases = ['cargo', 'test', '-p', package, '--features', 'ark'], 'rust', ['0:unit_case']
        checks.append({'name': purpose, 'purpose': purpose, 'argv': argv, 'cwd': '.', 'env': {},
                       'runner': runner, 'cases': cases, 'ignored_cases': {}})
    checks.extend({'name': purpose, 'purpose': purpose, 'argv': argv, 'cwd': 'testing/lab-ark-funded/verifier',
                   'env': {}, 'runner': 'command', 'cases': [], 'ignored_cases': {}} for purpose, argv in VERIFIER_STATIC.items())
    return {'checks': checks, 'capabilities': {c: sorted(CAPABILITY_PURPOSES[c]) for c in CAPABILITIES}}


class Portability(unittest.TestCase):
    def test_QA_package_relocates_and_imports_without_original_tree_or_private_archive(self):
        reference = Path(os.environ.get('AP5_QA_TEST_SOURCE', str(HERE.parents[1])))
        manifest, _ = qa_manifest(reference)
        with tempfile.TemporaryDirectory(prefix='ap5-portable-') as tmp:
            root = Path(tmp); repo = root / 'checkout'
            for row in manifest['files']:
                target = repo / row['path'];target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(reference / row['path'], target)
            package_qa(repo, root / 'package')
            (root / 'package').rename(root / 'relocated')
            shutil.rmtree(repo)  # Only the disposable copied fixture tree.
            descriptor = root / 'relocated/qa-source.json'
            value = json.loads(descriptor.read_text())
            self.assertEqual(value['archive']['path'], 'source.tar.gz')
            self.assertEqual(value['manifest'], {'path': 'manifest.json', 'sha256': QA_MANIFEST_SHA256})
            self.assertNotIn(str(root), descriptor.read_text())
            code = '''
import sys
from pathlib import Path
from shared_sources import load_qa
from common import sha
p=Path(sys.argv[1]);module,value=load_qa(p,sha(p),Path(sys.argv[2]))
assert callable(module.Controller.audit_live) and callable(module.Controller.final_audit_after_drain)
assert value['api']=='t3-controller-r21'
import json
manifest=json.loads((p.parent/'manifest.json').read_text())
for row in manifest['files']:
    target=Path(sys.argv[2])/row['path']
    assert bool(target.stat().st_mode&0o111)==(row['mode']=='100755')
    assert target.stat().st_size==row['bytes'] and sha(target)==row['sha256']
'''
            result = subprocess.run([sys.executable, '-B', '-c', code, str(descriptor), str(root / 'runtime-source')],
                                    cwd=HERE, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr.decode())

    def test_QA_code_change_does_not_acquire_portability_approval(self):
        reference = Path(os.environ.get('AP5_QA_TEST_SOURCE', str(HERE.parents[1])))
        manifest, _ = qa_manifest(reference)
        with tempfile.TemporaryDirectory(prefix='ap5-portable-') as tmp:
            repo = Path(tmp)
            for row in manifest['files']:
                target = repo / row['path'];target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(reference / row['path'], target)
            with (repo / 'testing/lab-traffic/oracle.py').open('a') as stream:stream.write('\n# changed source\n')
            with self.assertRaisesRegex(Blocked, 'differs from reviewed'):qa_manifest(repo)

    def test_public_R4_review_relocates_but_changed_bytes_reject(self):
        original = HERE / 'provenance/pool-control-r4-review.md'
        self.assertEqual(sha(original), POOL_CONTROL_R4_REVIEW['sha256'])
        stall_review = HERE.parents[1] / 'docs/reviews/2026-11-12-pool-wallet-stall-coverage.md'
        self.assertEqual(sha(stall_review), POOL_CONTROL_STALL_COVERAGE_REVIEW['sha256'])
        with tempfile.TemporaryDirectory(prefix='ap5-portable-') as tmp:
            target = Path(tmp) / 'public-review.md';shutil.copyfile(original, target)
            stall_target = Path(tmp) / 'stall-review.md';shutil.copyfile(stall_review, stall_target)
            policy = {'source_snapshot_sha256': 'd00299c259aaa5e8098542f48f5c3c90c66737be3f965f5d65e5fd2a763914b2',
                      'tested_code_snapshot_sha256': 'f764900419db45901f558c367b5485938ed1b20b5ee5abdc039e3b66c230ae0c',
                      'files': dict(POOL_CONTROL_R4_FILES),
                      'same_auditor_review': {'path': str(target), 'sha256': sha(target)},
                      'stall_coverage_review': {'path': str(stall_target), 'sha256': sha(stall_target)}}
            validate_pool_control_admission(policy)
            target.write_text('substituted review')
            with self.assertRaises(Blocked):validate_pool_control_admission(policy)

    def test_QA_relative_references_cannot_escape_or_alias_and_resealed_changes_reject(self):
        from shared_sources import load_qa
        reference = Path(os.environ.get('AP5_QA_TEST_SOURCE', str(HERE.parents[1])))
        with tempfile.TemporaryDirectory(prefix='ap5-portable-') as tmp:
            root = Path(tmp); package_qa(reference, root / 'package')
            descriptor = root / 'package/qa-source.json'
            original = json.loads(descriptor.read_text())
            for location in ('../manifest.json', '/absolute/manifest.json', './manifest.json'):
                bad = copy.deepcopy(original);bad['manifest']['path'] = location
                descriptor.write_text(json.dumps(bad))
                with self.subTest(location=location), self.assertRaises(Blocked):
                    load_qa(descriptor, sha(descriptor), root / 'never-import')
            descriptor.write_text(json.dumps(original))
            manifest = root / 'package/manifest.json'
            manifest.rename(root / 'manifest.json');manifest.symlink_to(root / 'manifest.json')
            with self.assertRaises(Blocked):load_qa(descriptor, sha(descriptor), root / 'never-import')
            manifest.unlink();(root / 'manifest.json').rename(manifest)
            value = json.loads(manifest.read_text());value['files'][0]['mode'] = '100755'
            manifest.write_text(json.dumps(value))
            bad = copy.deepcopy(original);bad['manifest']['sha256'] = sha(manifest)
            descriptor.write_text(json.dumps(bad))
            with self.assertRaises(Blocked):load_qa(descriptor, sha(descriptor), root / 'never-import')
            self.assertFalse((root / 'never-import').exists())

    def test_committed_source_reader_uses_HEAD_not_index_and_rejects_dirty_bytes(self):
        with tempfile.TemporaryDirectory(prefix='ap5-portable-') as tmp:
            root = Path(tmp);path = root / 'Cargo.toml';path.write_bytes(b'committed source\n')
            data = path.read_bytes();oid = hashlib.sha1(b'blob ' + str(len(data)).encode() + b'\0' + data).hexdigest()
            tree = f'100644 blob {oid}\tCargo.toml\0'.encode()
            with patch('portable.subprocess.check_output', return_value=tree) as git:
                self.assertEqual(committed_rows(root)[0]['sha256'], sha(path))
                self.assertIn('ls-tree', git.call_args.args[0]);self.assertIn('HEAD', git.call_args.args[0])
                path.write_bytes(b'uncommitted source\n')
                with self.assertRaisesRegex(Blocked, 'differs from HEAD'):committed_rows(root)

    def test_unqualified_contract_and_missing_lineage_stay_unavailable(self):
        with tempfile.TemporaryDirectory(prefix='ap5-portable-') as tmp:
            root = Path(tmp);target = root / 'testing/lab-ark-funded/source-contract.json';target.parent.mkdir(parents=True)
            target.write_text(json.dumps({'schema': 'spc.lab.ap5.committed-source-contract/v1', 'state': 'pending'}))
            with self.assertRaisesRegex(Blocked, 'not yet supplied'):check_contract(root)
        with self.assertRaises(Blocked):check_lineage({'schema': 'spc.lab.ap5.aggregate-source-lineage/v1', 'source_snapshot_sha256': None, 'files': {}}, {})

    def test_gate_requires_cases_complete_capability_links_and_no_machine_paths(self):
        value = plan();validate_plan(value)
        for change in ('missing-purpose', 'boolean-capability', 'unrelated-capability', 'counter-only', 'private-path', 'fake-program'):
            bad = copy.deepcopy(value)
            if change == 'missing-purpose':bad['checks'].pop()
            elif change == 'boolean-capability':bad['capabilities']['readiness'] = True
            elif change == 'unrelated-capability':bad['capabilities']['native-audit'] = ['ui-build']
            elif change == 'counter-only':next(c for c in bad['checks'] if c['runner'] == 'rust')['cases'] = []
            elif change == 'private-path':bad['checks'][0]['env']['CARGO_TARGET_DIR'] = '/home/old-machine/target'
            else:bad['checks'][0]['argv'][0] = 'echo'
            with self.subTest(change=change), self.assertRaises(Blocked):validate_plan(bad)

    def test_verifier_source_lock_and_public_vectors_require_no_live_corpus(self):
        policy = json.loads((HERE / 'verifier-source-policy.json').read_text())
        self.assertEqual(len(policy['files']), 9)
        self.assertEqual({str(p.relative_to(HERE / 'verifier')) for p in (HERE / 'verifier').rglob('*') if p.is_file()},
                         {r['path'] for r in policy['files']})
        for row in policy['files']:
            path = HERE / 'verifier' / row['path']
            self.assertEqual(sha(path), row['sha256'])
            self.assertEqual(path.stat().st_size, row['bytes'])
            self.assertEqual(bool(path.stat().st_mode & 0o111), row['executable'])
        fixture = json.loads((HERE / 'verifier/fixtures/stock-public.json').read_text())
        self.assertTrue(fixture['public']['board_vtxo']);self.assertTrue(fixture['public']['anchor_tx'])

    def test_verifier_log_cannot_replace_six_real_cases_with_a_count_or_duplicate_run(self):
        log = 'running 6 tests\n' + ''.join(f'test {name} ... ok\n' for name in sorted(CRYPTO_CASES))
        log += 'test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n'
        validate_crypto_log(log)
        for bad in (log + log, 'test result: ok. 6 passed; 0 failed; 0 ignored;\n', log.replace(' ... ok', ' ... ignored', 1)):
            with self.assertRaises(Blocked):validate_crypto_log(bad)
