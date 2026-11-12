"""Portable probe manifest intake only; local recipe files cannot confer Nix admission."""
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile
import unittest

from common import Blocked, encoded, probe_invocation, sha
from probe_package import CONTRACT_SHA, validate_program_binding


class ProbePackage(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='ap5-probe-package-')
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        reference = Path(os.environ['AP5_PROBE_TEST_PACKAGE'])
        self.script_root = self.root / 'share/ark-rpc'
        self.script_root.parent.mkdir(parents=True)
        shutil.copytree(reference / 'share/ark-rpc', self.script_root)
        self.bin = self.root / 'bin'; self.bin.mkdir()
        self.program = Path(sys.executable).resolve()
        self.script = self.script_root / 'probe.py'
        self.wrapper = self.bin / 'ark-rpc-probe'
        self.refs = {}
        for name, script in [('ark-rpc-probe', self.script), ('ark-lab-profile-check', self.script_root / 'lifecycle_fixture.py')]:
            path = self.bin / name
            path.write_text(f'#!/unit-binding-fixture\nexec {self.program} {script} "$@"\n')
            self.refs[name] = self.ref(path)
        for name in ('ark-rpc-source-manifest.json', 'ark-rpc-fixtures-manifest.json'):
            value = json.loads((reference / 'share' / name).read_text())
            value['root'] = str(self.script_root)
            (self.root / 'share' / name).write_bytes(encoded(value))
        self.source = self.ref(self.root / 'share/ark-rpc-source-manifest.json')
        self.binding = {'schema': 'spc.lab.probe-program/v1', 'contract_sha256': CONTRACT_SHA,
                        'native_python': self.ref(self.program), 'native_python_resolved': str(self.program),
                        'probe_script': self.ref(self.script), 'wrappers': self.refs, 'source_manifest': self.source,
                        'fixture_manifest': self.ref(self.root / 'share/ark-rpc-fixtures-manifest.json')}

    def ref(self, path):return {'path': str(path), 'sha256': sha(path)}

    def check(self):
        path = self.root / 'share/ark-rpc-program-binding.json';path.write_bytes(encoded(self.binding))
        return validate_program_binding(self.ref(path), self.source, self.wrapper, self.program, self.script)

    def test_runtime_fixture_and_program_manifests_bind_all_files_but_not_Nix_admission(self):
        self.check()
        with self.assertRaisesRegex(Blocked, 'immutable Nix'):probe_invocation(self.wrapper)

    def test_fixture_or_program_substitution_and_double_subcommand_reject(self):
        self.wrapper.write_text(f'#!/unit-binding-fixture\nexec {self.program} {self.script} node-probe "$@"\n')
        self.binding['wrappers']['ark-rpc-probe'] = self.ref(self.wrapper)
        with self.assertRaises(Blocked):self.check()

    def test_missing_or_changed_fixture_is_not_hidden_by_runtime_manifest(self):
        value = json.loads(Path(self.binding['fixture_manifest']['path']).read_text())
        path = self.script_root / next(iter(value['files']))
        path.write_bytes(b'changed fixture')
        with self.assertRaises(Blocked):self.check()
