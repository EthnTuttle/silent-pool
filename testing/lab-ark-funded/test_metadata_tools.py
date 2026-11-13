"""Actual installed metadata writer checks; no nodes, RPC or wallet actors."""
import copy
import os
from pathlib import Path
import tempfile
import unittest

from common import Blocked, load
from metadata_tools import git_binding, preflight, ref, runtime_path


class MetadataTools(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.binding = git_binding(os.environ['AP5_METADATA_TEST_TOOLS'])
        root = Path(os.environ['AP5_METADATA_TEST_PROBE'])
        record = root / 'share/ark-rpc-program-binding.json'
        value = load(record)
        cls.probe = {'program': value['native_python'], 'script': value['probe_script'],
                     'source_manifest': value['source_manifest'], 'program_binding': ref(record)}
        cls.base_path = str(Path(value['native_python']['path']).parent)
        cls.path = runtime_path(cls.base_path, cls.binding['git']['path'])

    def test_installed_manifest_writer_exact_path_and_source_inventory(self):
        with tempfile.TemporaryDirectory(prefix='ap5-metadata-') as temp:
            record = preflight(self.probe, self.path, self.binding, Path(temp) / 'check')
            manifest = load(record['manifest']['path'])
            self.assertIsNone(record['base_commit'])
            self.assertIsNone(manifest['base_commit'])
            self.assertEqual(len(manifest['freeze_recipe']['files']), 38)
            self.assertEqual(manifest['run_token'], 'METADATA-ONLY-NO-NODE')
            self.assertEqual(record['exit'], 0)

    def test_missing_git_is_refused_before_metadata_process_or_actor(self):
        with tempfile.TemporaryDirectory(prefix='ap5-metadata-') as temp:
            output = Path(temp) / 'must-not-start'
            with self.assertRaisesRegex(Blocked, 'Git missing or shadowed'):
                preflight(self.probe, self.base_path, self.binding, output)
            self.assertFalse(output.exists())

    def test_fake_source_or_deriver_receipt_does_not_qualify_real_binary(self):
        for field in ('git_source', 'git_deriver', 'git'):
            changed = copy.deepcopy(self.binding);changed[field]['sha256'] = '00' * 32
            with tempfile.TemporaryDirectory(prefix='ap5-metadata-') as temp:
                output = Path(temp) / 'must-not-start'
                with self.subTest(field=field), self.assertRaises(Blocked):
                    preflight(self.probe, self.path, changed, output)
                self.assertFalse(output.exists())

    def test_host_path_and_lexical_store_escape_reject(self):
        for path in ('/usr/bin', '/nix/store/../../usr/bin', self.base_path + ':'):
            with self.subTest(path=path), self.assertRaises(Blocked):
                runtime_path(path, self.binding['git']['path'])
        with self.assertRaises(Blocked):runtime_path(self.base_path, '/usr/bin/git')
