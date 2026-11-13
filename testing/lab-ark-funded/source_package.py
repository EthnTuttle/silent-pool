"""Invoke the reviewed source-owned r22 exporter; hashes grant no live capability."""
import hashlib
from pathlib import Path
import subprocess
import sys

from common import require, sha
from pool_control import strict_json

QA_API = 't3-controller-r22'
QA_MANIFEST_SHA256 = '88a2e6d2f07936420041a8903133dcd1ec3932ca8ab3a44f42dc49bfc32715e8'
QA_ARCHIVE_SHA256 = '0d580efb996615ac0889654b021068e7e5d77aa722a17c5da1f0fb66ed489671'
QA_FILES = 127
QA_BYTES = 1493665
EXPORTER_SHA256 = 'c0ff78afd921d52bf85485842ecf40d8605394d83722b0beb2ead8efc30e506d'


def exporter(repo):
    repo = Path(repo).resolve(strict=True)
    path = repo / 'testing/lab-traffic/portable_source.py'
    require(not path.is_symlink() and path.resolve().is_relative_to(repo) and sha(path) == EXPORTER_SHA256,
            'QA exporter differs from reviewed r22 source')
    return repo, path


def qa_manifest(repo, require_approved=True):
    repo, path = exporter(repo)
    # Execute only the hash-verified exporter, with no mutable module imports.
    namespace = {'__name__': 'ap5_reviewed_qa_exporter', '__file__': str(path)}
    raw = path.read_bytes()
    require(hashlib.sha256(raw).hexdigest() == EXPORTER_SHA256, 'QA exporter changed before execution')
    exec(compile(raw, str(path), 'exec'), namespace)
    rows = namespace['inventory'](repo)
    value = {'schema': 'spc.qa.source-manifest/1', 'recipe': namespace['RECIPE'], 'api': QA_API,
             'roots': list(namespace['ROOTS']), 'files': rows, 'source_bytes': sum(r['bytes'] for r in rows)}
    raw = namespace['encode'](value)
    if require_approved:
        require(len(rows) == QA_FILES and value['source_bytes'] == QA_BYTES
                and hashlib.sha256(raw).hexdigest() == QA_MANIFEST_SHA256,
                'QA source differs from reviewed r22 source/mode/byte census')
    return value, raw


def package_qa(repo, output):
    repo, path = exporter(repo)
    qa_manifest(repo)
    result = subprocess.run([sys.executable, '-B', str(path), '--repo', str(repo), 'snapshot', '--out', str(output)],
                            capture_output=True, timeout=60)
    require(result.returncode == 0, 'QA source-owned snapshot failed: ' + result.stderr.decode())
    root = Path(output)
    value = strict_json((root / 'qa-source.json').read_bytes())
    require(value == {'schema': 'spc.lab.ap5.qa-source/v1', 'api': QA_API,
                     'archive': {'path': 'source.tar.gz', 'sha256': QA_ARCHIVE_SHA256},
                     'manifest': {'path': 'manifest.json', 'sha256': QA_MANIFEST_SHA256}}, 'QA exporter descriptor differs')
    require(sha(root / 'source.tar.gz') == QA_ARCHIVE_SHA256 and sha(root / 'manifest.json') == QA_MANIFEST_SHA256,
            'QA snapshot differs from reviewed deterministic recipe')
    return value
