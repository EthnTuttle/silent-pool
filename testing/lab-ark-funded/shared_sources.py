"""Load the complete hash-bound T3 Python closure from a frozen source archive."""
import hashlib
import importlib.abc
import importlib.util
from pathlib import Path, PurePosixPath
import sys
import tarfile

from common import artifact, require
from pool_control import strict_json

R18_ARCHIVE = '07ffa5f1f15a59ad5ef3f0ca4002b6d501d13482223fd51bfaac14becc1d8236'
R18_MANIFEST = '25ad9b5d43121c4fd56bdfa13796d5f84ad469b8dc1bdfea2c374591a5d80664'
R19_ARCHIVE = '336b7f914bfdb6f8fdc04c1ec0416c2b18feafe6df05591546994a0b541d6e74'
R19_MANIFEST = 'db5f7f981bdd0ce2f15e1be6a981ec7e488a7410acb7eb71cff314a4a772862b'
R20_ARCHIVE = 'cce72a37d4bb2f05fe551a718aad5d7e5d5daf3cb51e71de226af41d55620364'
R20_MANIFEST = '12954d0fa956961ddce5e03cc26d0c8362db9529b06f26b8add493fa24ee5804'
PROFILES = {
    't3-controller-r18': (R18_ARCHIVE, R18_MANIFEST, 98, 'ap5_bound_t3_r18'),
    't3-controller-r19': (R19_ARCHIVE, R19_MANIFEST, 102, 'ap5_bound_t3_r19'),
    't3-controller-r20': (R20_ARCHIVE, R20_MANIFEST, 107, 'ap5_bound_t3_r20'),
    't3-controller-r21': ('59a0f849d7697b915a940855093709df39aed2c7ed4ca8894b30aecc1f282f8c',
                          '41dcb01b9f75ef6605773d5d2467c17024d768cf72b5605abdb135e0dca0494f', 123, 'ap5_bound_t3_r21'),
}


class BoundModules(importlib.abc.MetaPathFinder, importlib.abc.Loader):
    def __init__(self, root, sources):
        self.root, self.sources = root, sources

    def find_spec(self, fullname, path=None, target=None):
        if fullname in self.sources:
            return importlib.util.spec_from_loader(fullname, self, origin=str(self.root / self.sources[fullname][0]))
        return None

    def create_module(self, spec):
        return None

    def exec_module(self, module):
        name, raw = self.sources[module.__name__]
        path = self.root / name
        require(path.read_bytes() == raw, 'shared QA source changed before import')
        module.__file__ = str(path)
        exec(compile(raw, str(path), 'exec'), module.__dict__)


def load_qa(descriptor, descriptor_sha256, output):
    """No mutable worktree import, no build, and no producer/runtime invocation."""
    path = artifact({'path': str(descriptor), 'sha256': descriptor_sha256})
    value = strict_json(path.read_bytes())
    require(type(value) is dict and set(value) == {'schema', 'api', 'archive', 'manifest'}, 'shared QA descriptor shape')
    require(value['schema'] == 'spc.lab.ap5.qa-source/v1' and value['api'] in PROFILES, 'shared QA API version')
    archive_hash, manifest_hash, census, module_name = PROFILES[value['api']]
    for name in ('archive', 'manifest'):
        require(type(value[name]) is dict and set(value[name]) == {'path', 'sha256'}, 'shared QA artifact descriptor shape')
    require(value['archive']['sha256'] == archive_hash and value['manifest']['sha256'] == manifest_hash,
            'shared QA source version requires reviewed exact API binding')
    portable = value['api'] == 't3-controller-r21'
    def resolve(ref, filename):
        if not portable:
            return artifact(ref)
        require(ref['path'] == filename, 'shared QA portable artifact must be descriptor-relative')
        target = path.parent / filename
        require(not target.is_symlink() and target.resolve().parent == path.parent.resolve()
                and target.stat().st_nlink == 1, 'shared QA portable artifact alias/escape')
        return artifact({'path': str(target), 'sha256': ref['sha256']})
    archive = resolve(value['archive'], 'source.tar.gz')
    manifest = strict_json(resolve(value['manifest'], 'manifest.json').read_bytes())
    if portable:
        require(set(manifest) == {'schema', 'recipe', 'api', 'roots', 'files', 'source_bytes'}
                and manifest['schema'] == 'spc.qa.source-manifest/1' and manifest['recipe'] == 'spc.qa.source-tree/1'
                and manifest['api'] == value['api'] and manifest['roots'] == ['testing/lab-traffic', 'testing/lab-witness']
                and type(manifest['source_bytes']) is int and manifest['source_bytes'] == 1456511,
                'shared QA portable manifest schema/recipe/byte census')
    rows = {r['path']: r for r in manifest['files']}
    require(len(rows) == len(manifest['files']) == census, 'shared QA source census')
    if portable:
        require(list(rows) == sorted(rows) and sum(r['bytes'] for r in rows.values()) == manifest['source_bytes'],
                'shared QA portable manifest order/bytes')
        for name, row in rows.items():
            require(set(row) == {'path', 'sha256', 'bytes', 'mode'} and row['mode'] in {'100644', '100755'}
                    and type(row['bytes']) is int and 0 <= row['bytes'] <= 8 * 1024 * 1024,
                    'shared QA portable row shape/mode/bytes')
        header = archive.read_bytes()[:10]
        require(header[:4] == b'\x1f\x8b\x08\x00' and header[4:8] == bytes(4), 'shared QA deterministic gzip metadata')
    content = {}
    with tarfile.open(archive, 'r:gz') as tar:
        for member in tar.getmembers():
            if member.isdir():
                require(not portable, 'shared QA portable archive extra directory')
                continue
            require(member.isfile() and not member.issym() and not member.islnk(), 'shared QA archive regular files only')
            name = member.name.removeprefix('source/')
            rel = PurePosixPath(name)
            require(not rel.is_absolute() and '..' not in rel.parts and name in rows and name not in content,
                    'shared QA archive path/census')
            require(member.size == rows[name]['bytes'] and member.size <= 8 * 1024 * 1024, 'shared QA source size')
            if portable:
                require(member.name == 'source/' + name and member.mode == (int(rows[name]['mode'], 8) & 0o777)
                        and member.mtime == member.uid == member.gid == 0 and member.uname == member.gname == ''
                        and not member.pax_headers and member.linkname == '', 'shared QA deterministic tar metadata/mode')
            with tar.extractfile(member) as stream:
                raw = stream.read()
            require(hashlib.sha256(raw).hexdigest() == rows[name]['sha256'], 'shared QA source row hash')
            content[name] = raw
    require(set(content) == set(rows), 'shared QA missing source rows')
    if portable:
        require(list(content) == list(rows), 'shared QA deterministic archive order')
    return import_content(value, content, output, module_name, rows if portable else None)


def import_content(value, content, output, module_name, rows=None):
    output = Path(output)
    output.mkdir(mode=0o700)
    sources = {}
    for name, raw in content.items():
        target = output / name
        target.parent.mkdir(parents=True, exist_ok=True)
        with target.open('xb') as stream:
            stream.write(raw)
        target.chmod(0o555 if rows and rows[name]['mode'] == '100755' else 0o444)
        rel = PurePosixPath(name)
        if rel.parent == PurePosixPath('testing/lab-traffic') and rel.suffix == '.py' and rel.stem.isidentifier():
            sources[rel.stem] = (name, raw)
    entry = 'testing/lab-traffic/regtest-traffic-e2e.py'
    sources[module_name] = (entry, content[entry])
    require(not (set(sources) & set(sys.modules)), 'shared QA module name already loaded from another source')
    loader = BoundModules(output, sources)
    sys.meta_path.insert(0, loader)
    # QA modules are compiled only from the hash-checked byte map. Their data
    # paths resolve within the separately retained read-only extracted tree.
    import importlib
    module = importlib.import_module(module_name)
    for name in ('prerequisites', 'start', 'bootstrap', 'measured', 'reorg', 'witness', 'audit', 'receipt', 'check', 'result'):
        require(callable(getattr(module.Controller, name, None)), 'shared QA Controller export missing: ' + name)
    if value['api'] in {'t3-controller-r19', 't3-controller-r20', 't3-controller-r21'}:
        require(callable(getattr(module, 'operator_arguments', None)), 'shared QA operator export missing')
        runtime = importlib.import_module('source_feed_runtime')
        require(callable(getattr(runtime, 'collector_socket_path', None)), 'shared QA collector-root export missing')
    if value['api'] in {'t3-controller-r20', 't3-controller-r21'}:
        for name in ('audit_live', 'final_audit_after_drain'):
            require(callable(getattr(module.Controller, name, None)), 'shared QA final API missing: ' + name)
        require(callable(getattr(runtime.CollectorRuntime, 'final_audit_after_drain', None)), 'shared QA final collector API missing')
    return module, value
