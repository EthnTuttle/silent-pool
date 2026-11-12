"""Read-only admission of the single already-built, parent-verified verifier."""
import copy
import json
from pathlib import Path
import subprocess
import unittest
from unittest.mock import patch

from common import Blocked
from verifier_admission import admit_compiled_verifier


def approved_record():
    return json.loads(Path('/tmp/opencode/AP5-verifier-build-r1/vtxo-verifier-artifact.json').read_text())


class CompiledVerifierAdmission(unittest.TestCase):
    def test_approved_real_nix_artifact_does_not_read_mutable_verifier_tree(self):
        record = approved_record()
        mutable = Path(__file__).parent / 'verifier'
        original_open = Path.open
        def guarded_open(path, *args, **kwargs):
            if path.is_relative_to(mutable):
                raise AssertionError('mutable verifier source is not admission authority')
            return original_open(path, *args, **kwargs)
        with patch.object(Path, 'open', guarded_open):
            self.assertEqual(admit_compiled_verifier(record), Path(record['path']))
            minimal = {key: record[key] for key in ('path', 'sha256', 'cargo_lock_sha256')}
            self.assertEqual(admit_compiled_verifier(minimal), Path(record['path']))

    def test_wrong_binary_lock_source_or_receipt_labels_are_rejected(self):
        original = approved_record()
        for key, value in (('sha256', '00' * 32), ('cargo_lock_sha256', '00' * 32),
                           ('source_root', '/nix/store/unapproved-source'), ('deriver', '/nix/store/unapproved.drv')):
            changed = {**original, key: value}
            with self.assertRaises(Blocked):
                admit_compiled_verifier(changed)
        changed = copy.deepcopy(original)
        changed['nix_binding']['sha256'] = '00' * 32
        with self.assertRaises(Blocked):
            admit_compiled_verifier(changed)

    def test_real_derivation_source_cannot_be_replaced_by_an_artifact_claim(self):
        original_output = subprocess.check_output
        def different_source(argv, **kwargs):
            result = original_output(argv, **kwargs)
            if argv[:3] == ['nix', 'derivation', 'show']:
                data = json.loads(result)
                env = next(iter(data.values()))['env']
                if '__json' in env:
                    structured = json.loads(env['__json'])
                    structured['src'] = '/nix/store/unapproved-source'
                    env['__json'] = json.dumps(structured)
                else:
                    env['src'] = '/nix/store/unapproved-source'
                return json.dumps(data)
            return result
        with patch('verifier_admission.subprocess.check_output', side_effect=different_source):
            with self.assertRaisesRegex(Blocked, 'actual source/check binding'):
                admit_compiled_verifier(approved_record())
