"""Declared runtime metadata dependencies, verified before any actor startup."""
import json
import os
from pathlib import Path
import re
import shutil
import subprocess

from common import artifact, encoded, fresh_root, load, require, sha
from nix_derivation import derivation_record


def ref(path):
    path = Path(path).resolve(strict=True)
    return {'path': str(path), 'sha256': sha(path)}


def declared_tools(consumer):
    consumer = Path(consumer).resolve(strict=True)
    require(consumer.is_relative_to('/nix/store') and consumer.name == 'ap5-tools', 'immutable AP5 tools package required')
    lines = re.findall(r'^export PATH="([^"\n]+)"$', consumer.read_text(), re.M)
    require(len(lines) == 1 and lines[0].endswith(':$PATH'), 'AP5 wrapper must declare its Nix tool prefix')
    dirs = lines[0][:-len(':$PATH')].split(':')
    require(dirs and all(Path(p).is_absolute() and Path(p).resolve().is_relative_to('/nix/store') and '..' not in Path(p).parts and Path(p).is_dir()
                         and '$' not in p for p in dirs), 'AP5 tool prefix contains an undeclared or mutable directory')
    tools = {}
    for name in ('nix', 'nix-store', 'git'):
        path = shutil.which(name, path=os.pathsep.join(dirs))
        require(path is not None, 'AP5 closure lacks declared metadata tool: ' + name)
        tools[name] = Path(path)  # Preserve nix-store's multicall argv[0].
        require(tools[name].resolve(strict=True).is_relative_to('/nix/store'), 'metadata executable escaped Nix store')
    return consumer, tools


def git_binding(consumer):
    consumer, tools = declared_tools(consumer)
    package = Path('/nix/store') / consumer.relative_to('/nix/store').parts[0]
    closure = subprocess.check_output([str(tools['nix-store']), '-q', '--requisites', str(package)], text=True, timeout=15).splitlines()
    git = tools['git']; git_package = Path('/nix/store') / git.relative_to('/nix/store').parts[0]
    require(str(git_package) in closure, 'Git is not in the actual AP5 tools closure')
    deriver = subprocess.check_output([str(tools['nix-store']), '-q', '--deriver', str(git_package)], text=True, timeout=15).strip()
    raw = subprocess.check_output([str(tools['nix']), '--extra-experimental-features', 'nix-command', 'derivation', 'show', deriver], timeout=15)
    record = derivation_record(raw, deriver)
    require(any(v.get('path') == str(git_package) for v in record['outputs'].values()), 'Git actual derivation output differs')
    attrs = record.get('structuredAttrs', record['env'])
    attrs = json.loads(attrs['__json']) if '__json' in attrs else attrs
    require(attrs.get('pname') == 'git', 'declared metadata helper is not the Git derivation')
    source = Path(attrs['src'])
    require(source.is_absolute() and source.resolve().is_relative_to('/nix/store') and source.is_file(), 'Git source archive binding required')
    with git.open('rb') as stream:
        require(stream.read(4) == b'\x7fELF', 'Git metadata helper must be the actual native executable')
    return {'schema': 'spc.lab.ap5.runtime-metadata-tools/v1', 'consumer_tools': ref(consumer),
            'git': ref(git), 'git_deriver': ref(deriver), 'git_source': ref(source),
            'git_version': attrs['version'], 'derivation_tools': {n: {'path': str(tools[n]), 'sha256': sha(tools[n])} for n in ('nix', 'nix-store')},
            'scope': 'metadata executable provenance only; no application, protocol or funded capability'}


def runtime_path(base, git):
    git = Path(git)
    require(git.is_absolute() and git.resolve().is_relative_to('/nix/store') and git.is_file(),
            'runtime metadata Git must be an actual Nix executable')
    dirs = base.split(':')
    require(base and all(p and Path(p).is_absolute() and Path(p).resolve().is_relative_to('/nix/store') and '..' not in Path(p).parts and Path(p).is_dir()
                         for p in dirs), 'runtime PATH must contain only explicit existing Nix directories')
    directory = str(git.parent)
    return os.pathsep.join([directory] + [p for p in dirs if p != directory])


def require_resolved_git(path_env, git):
    # No ambient PATH or fallback search: exactly the environment handed to Runtime.
    require(runtime_path(path_env, git), 'invalid explicit metadata PATH')
    resolved = shutil.which('git', path=path_env)
    require(resolved is not None and Path(resolved).resolve() == Path(git).resolve(),
            'declared Git missing or shadowed in exact runtime PATH before actor startup')


def preflight(probe, path_env, binding, output):
    require(encoded(git_binding(artifact(binding['consumer_tools']))) == encoded(binding), 'metadata tool source/deriver/closure binding changed')
    git = artifact(binding['git']);require_resolved_git(path_env, git)
    root = fresh_root(output)
    script = Path(__file__).with_name('probe_metadata_check.py')
    fixture = Path(__file__).parent / 'fixtures/probe-metadata-only.json'
    program = artifact(probe['program']);probe_root = artifact(probe['script']).parent
    argv = [str(program), '-B', str(script), str(probe_root), str(git), str(fixture), str(root)]
    result = subprocess.run(argv, env={'PATH': path_env, 'HOME': str(root)}, capture_output=True, timeout=30)
    (root / 'stdout.log').write_bytes(result.stdout);(root / 'stderr.log').write_bytes(result.stderr)
    require(result.returncode == 0, 'installed probe metadata writer failed under exact runtime PATH before actor startup')
    manifest = load(root / 'manifest.json')
    require(manifest['base_commit'] is None, 'installed non-Git source must not acquire an invented approved base commit')
    source = load(artifact(probe['source_manifest']))
    program_binding = load(artifact(probe['program_binding']))
    fixtures = load(artifact(program_binding['fixture_manifest']))
    require(manifest['freeze_recipe']['files'] == {**source['files'], **fixtures['files']}, 'metadata writer source inventory changed')
    receipt = {'schema': 'spc.lab.ap5.runtime-metadata-preflight/v1', 'status': 'PASSED',
        'scope': 'actual installed file-only metadata writer; no node/RPC/funding or approved base-commit claim',
        'path_env': path_env, 'argv': argv, 'git': ref(git), 'helper': ref(script), 'fixture': ref(fixture),
        'exit': result.returncode, 'manifest': ref(root / 'manifest.json'), 'base_commit': None,
        'stdout': ref(root / 'stdout.log'), 'stderr': ref(root / 'stderr.log')}
    (root / 'receipt.json').write_bytes(encoded(receipt) + b'\n')
    return receipt


def validate_binding(inputs, probe, output):
    require(inputs.get('input_recipe') == 'spc.lab.ap5.prepared-inputs/2-explicit-metadata-tools',
            'explicit metadata input recipe required before actor startup')
    record_ref = inputs.get('runtime_metadata')
    require(type(record_ref) is dict and set(record_ref) == {'path', 'sha256'}, 'declared runtime metadata dependency binding required before actor startup')
    record = load(artifact(record_ref))
    git = record['git']
    require(inputs['binaries'].get('git') == {**git, 'source_manifest': record_ref['path'],
                                            'source_manifest_sha256': record_ref['sha256']}, 'Git binary/source seal differs from metadata binding')
    return preflight(probe, inputs['path_env'], record, output)
