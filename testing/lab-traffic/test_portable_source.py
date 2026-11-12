"""Source-only packaging and qualification refusal; no builds or lab actors."""
import hashlib
import json
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

from portable_source import inventory, snapshot, record_check
from ap5_verifier import CompiledVerifier, BINARY_SHA256
from oracle import Invalid, Inconclusive


class PortableSourceTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory(prefix='spc-portable-');self.addCleanup(self.temp.cleanup)
        self.root=Path(self.temp.name);self.repo=self.root/'checkout'
        for name in ('lab-traffic','lab-witness'):
            path=self.repo/'testing'/name;path.mkdir(parents=True);(path/'example.py').write_text('VALUE = 1\n')
            (path/'example.py').chmod(0o644)

    def test_snapshot_is_relocatable_deterministic_and_binds_Git_modes(self):
        first=snapshot(self.repo,self.root/'first')
        file=self.repo/'testing/lab-traffic/example.py';file.chmod(0o600)
        second=snapshot(self.repo,self.root/'second')
        self.assertEqual(first['descriptor_sha256'],second['descriptor_sha256'])
        self.assertEqual((self.root/'first/source.tar.gz').read_bytes(),(self.root/'second/source.tar.gz').read_bytes())
        descriptor=json.loads((self.root/'first/qa-source.json').read_bytes())
        self.assertEqual(descriptor['archive']['path'],'source.tar.gz');self.assertEqual(descriptor['manifest']['path'],'manifest.json')
        rows={r['path']:r for r in json.loads((self.root/'first/manifest.json').read_bytes())['files']}
        with tarfile.open(self.root/'first/source.tar.gz') as archive:
            for member in archive.getmembers():
                row=rows[member.name.removeprefix('source/')];raw=archive.extractfile(member).read()
                self.assertTrue(member.isfile());self.assertEqual(len(raw),row['bytes']);self.assertEqual(hashlib.sha256(raw).hexdigest(),row['sha256'])
                self.assertEqual(member.mode,int(row['mode'],8)&0o777)

    def test_symlink_and_checkout_output_are_refused(self):
        with self.assertRaises(ValueError):snapshot(self.repo,self.repo/'output')
        (self.repo/'testing/lab-traffic/alias.py').symlink_to('example.py')
        with self.assertRaises(ValueError):inventory(self.repo)

    def test_actual_failed_check_is_retained_not_labelled_qualification(self):
        result=record_check(self.repo,self.root/'check','unit-command',[sys.executable,'-B','-c','raise SystemExit(7)'],5)
        self.assertEqual(result['exit_code'],7);self.assertFalse(result['timed_out']);self.assertTrue(result['source_unchanged'])
        self.assertEqual(result['scope'],'actual_local_execution_receipt_not_runtime_or_funded_admission')

    def test_portable_verifier_profile_cannot_substitute_binary_or_boolean(self):
        for artifact in ({'schema':'spc.lab.ap5.verifier-reproduction/v1','compiled':True},
                         {'schema':'spc.lab.ap5.verifier-reproduction/v1','path':'/unit/not-run','sha256':'00'*32,'nix_binding':{},'test_log':{}}):
            with self.subTest(artifact=artifact),patch('reproduced_verifier.subprocess.run') as run:
                with self.assertRaises((Invalid,Inconclusive)):CompiledVerifier(artifact)
                run.assert_not_called()


if __name__=='__main__':unittest.main()
