"""Consume the portable probe's actual installed runtime/fixture/program seals."""
from pathlib import Path
import hashlib
import shlex

from common import artifact, encoded, load, require, sha

CONTRACT_SHA = '754e1e6131955f9e5bbce7ef81b6f9aabaa2f1379ec6409f1c3a7a9e88be91f9'
ENTRY_SHA = '833288d7b4a9491ff9cfa4d7186328f38f04e7fe7e5ef1c135496b5f9a62754a'
RUNTIME_MAP_SHA = '90d33070b190f89e5f6d35d07761a176bbb82a8bd903626186f819751c20deef'


def validate_program_binding(binding_ref, source_ref, wrapper, program, script):
    binding = load(artifact(binding_ref))
    require(set(binding) == {'schema', 'contract_sha256', 'native_python', 'native_python_resolved',
                             'probe_script', 'wrappers', 'source_manifest', 'fixture_manifest'}, 'portable probe program binding shape')
    require(binding['schema'] == 'spc.lab.probe-program/v1' and binding['contract_sha256'] == CONTRACT_SHA,
            'portable probe contract identity')
    require(artifact(binding['native_python']).resolve() == program.resolve()
            and binding['native_python_resolved'] == str(program.resolve())
            and binding['native_python']['sha256'] == sha(program), 'portable probe native ELF binding changed')
    with program.open('rb') as stream:
        require(stream.read(4) == b'\x7fELF', 'portable probe interpreter must be native ELF')
    require(artifact(binding['probe_script']) == script and binding['probe_script']['sha256'] == ENTRY_SHA,
            'portable probe thin entry changed')
    require(set(binding['wrappers']) == {'ark-rpc-probe', 'ark-lab-profile-check'}
            and artifact(binding['wrappers']['ark-rpc-probe']) == wrapper, 'portable probe wrapper identity')
    for name, ref in binding['wrappers'].items():
        path = artifact(ref)
        require(path == wrapper.parent / name, 'portable probe wrapper is outside its installed package')
        lines = path.read_text().splitlines()
        require(len(lines) == 2 and lines[0].startswith('#!'), 'portable probe wrapper shape')
        words = shlex.split(lines[1])
        expected_script = script if name == 'ark-rpc-probe' else script.parent / 'lifecycle_fixture.py'
        require(len(words) == 4 and words[0] == 'exec' and words[3] == '$@'
                and Path(words[1]).resolve() == program.resolve() and Path(words[2]) == expected_script,
                'portable probe wrapper changes program/script/argument forwarding')
    require(binding['source_manifest'] == source_ref, 'portable probe source manifest reference differs')
    source = load(artifact(source_ref)); fixtures = load(artifact(binding['fixture_manifest']))
    require(hashlib.sha256(encoded(source['files'])).hexdigest() == RUNTIME_MAP_SHA,
            'portable probe runtime content differs from reviewed source; this hash alone is not build admission')
    for value, schema, count in ((source, 'spc.lab.ap5.probe-source/v1', 14),
                                 (fixtures, 'spc.lab.probe-fixtures/v1', 24)):
        require(set(value) == {'schema', 'root', 'files'} and value['schema'] == schema
                and value['root'] == str(script.parent) and type(value['files']) is dict
                and len(value['files']) == count, 'portable probe source/fixture census')
        for name, digest in value['files'].items():
            path = script.parent / name
            require(not Path(name).is_absolute() and '..' not in Path(name).parts and not path.is_symlink()
                    and path.is_file() and sha(path) == digest, 'portable probe installed file differs')
    require(not (set(source['files']) & set(fixtures['files'])), 'portable probe source/fixture overlap')
    actual = {}
    for path in script.parent.rglob('*'):
        require(not path.is_symlink(), 'portable probe source alias')
        if path.is_file() and '__pycache__' not in path.parts:
            actual[str(path.relative_to(script.parent))] = sha(path)
    require(actual == {**source['files'], **fixtures['files']}, 'portable probe omitted/substituted installed files')
    return binding
