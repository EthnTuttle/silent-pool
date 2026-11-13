"""Reject damaged or self-resealed r22 packages before importing controller code."""
import json
import os
from pathlib import Path
import shutil
import tempfile
import unittest

from common import Blocked, sha
from shared_sources import load_qa
from source_package import package_qa


class R22SourceIntake(unittest.TestCase):
    def test_missing_wrong_archive_api_census_root_and_descriptor_reject(self):
        repo = Path(os.environ.get('AP5_QA_TEST_SOURCE', str(Path(__file__).resolve().parents[2])))
        with tempfile.TemporaryDirectory(prefix='ap5-r22-negative-') as directory:
            root = Path(directory)
            package_qa(repo, root / 'original')
            for case in ('missing', 'archive', 'resealed-archive', 'api', 'census', 'old-byte-census', 'root', 'descriptor'):
                package = root / case;shutil.copytree(root / 'original', package)
                descriptor = package / 'qa-source.json';value = json.loads(descriptor.read_text())
                archive = package / 'source.tar.gz';manifest = package / 'manifest.json'
                if case == 'missing':archive.unlink()
                elif case in {'archive', 'resealed-archive'}:
                    archive.write_bytes(archive.read_bytes()[:-1])
                    if case == 'resealed-archive':value['archive']['sha256'] = sha(archive)
                elif case == 'api':value['api'] = 't3-controller-unreviewed'
                elif case in {'census', 'old-byte-census'}:
                    data = json.loads(manifest.read_text())
                    if case == 'census':data['files'].pop()
                    else:data['source_bytes'] = 1456511
                    manifest.write_text(json.dumps(data))
                    value['manifest']['sha256'] = sha(manifest)
                elif case == 'root':value['archive']['path'] = '../original/source.tar.gz'
                descriptor.write_text(json.dumps(value))
                digest = '00' * 32 if case == 'descriptor' else sha(descriptor)
                with self.subTest(case=case), self.assertRaises((Blocked, OSError)):
                    load_qa(descriptor, digest, package / 'must-not-import')
                self.assertFalse((package / 'must-not-import').exists())
