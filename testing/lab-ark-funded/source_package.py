"""Invoke the reviewed source-owned r26 exporter; hashes grant no live capability."""
import hashlib
from pathlib import Path
import subprocess
import sys

from common import require, sha
from pool_control import strict_json

QA_API = 't3-controller-r26'
QA_MANIFEST_SHA256 = '5cdd273c9c1de757f64b4527f57fbfc82dddfcf739da74844c5b8c991aed51fd'
QA_ARCHIVE_SHA256 = 'cec97f46656c634be69d46a8ce1cc32710967f4927949f41912fd8bf62ee4c9c'
QA_DESCRIPTOR_SHA256 = '2fd49d986d7133653ce8590a82f94fc7fafdee76ee774eb13e4a64f30c1ad737'
QA_FILES = 153
QA_BYTES = 1775218
EXPORTER_SHA256 = 'f6c31ef9972d472f8969f5a0013f0af2dcadf14a3ef49c9405714318b35d6e00'


def exporter(repo):
    repo = Path(repo).resolve(strict=True)
    path = repo / 'testing/lab-traffic/portable_source.py'
    require(not path.is_symlink() and path.resolve().is_relative_to(repo) and sha(path) == EXPORTER_SHA256,
            'QA exporter differs from reviewed r26 source')
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
                'QA source differs from reviewed r26 source/mode/byte census')
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
    require(sha(root / 'qa-source.json') == QA_DESCRIPTOR_SHA256, 'QA descriptor differs from reviewed r26 bytes')
    return value
