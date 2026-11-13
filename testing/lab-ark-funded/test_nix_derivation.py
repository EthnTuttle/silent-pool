"""Actual Nix JSON format compatibility preserves exact derivation selection."""
import copy
import json
import unittest

from common import Blocked
from nix_derivation import derivation_record


class NixDerivation(unittest.TestCase):
    drv = '/nix/store/' + 'a' * 32 + '-unit.drv'
    output = 'b' * 32 + '-unit'

    def fixture(self):
        record = {'version': 4, 'env': {'src': '/nix/store/' + 'c' * 32 + '-source'},
                  'outputs': {'out': {'path': self.output}}}
        return {'version': 4, 'derivations': {self.drv.rsplit('/', 1)[1]: record}}

    def test_v4_and_legacy_decode_same_source_and_absolute_output(self):
        current = derivation_record(json.dumps(self.fixture()), self.drv)
        self.assertEqual(current['outputs']['out']['path'], '/nix/store/' + self.output)
        legacy = copy.deepcopy(current);del legacy['version']
        self.assertEqual(derivation_record(json.dumps({self.drv: legacy}), self.drv), legacy)
        self.assertEqual(current['env'], legacy['env'])

    def test_unknown_version_wrong_or_extra_deriver_and_output_escape_reject(self):
        for case in ('version', 'bool', 'record-version', 'deriver', 'extra', 'absolute', 'escape'):
            value = self.fixture();rows = value['derivations'];record = next(iter(rows.values()))
            if case == 'version':value['version'] = 5
            elif case == 'bool':value['version'] = True
            elif case == 'record-version':record['version'] = 3
            elif case == 'deriver':value['derivations'] = {self.drv: record}
            elif case == 'extra':rows['foreign.drv'] = record
            elif case == 'absolute':record['outputs']['out']['path'] = '/nix/store/' + self.output
            else:record['outputs']['out']['path'] = self.output + '/../escape'
            with self.subTest(case=case), self.assertRaises(Blocked):
                derivation_record(json.dumps(value), self.drv)

    def test_duplicate_json_and_legacy_multiple_derivers_reject(self):
        with self.assertRaises(Blocked):derivation_record('{"version":4,"version":4,"derivations":{}}', self.drv)
        with self.assertRaises(Blocked):derivation_record(json.dumps({self.drv: {'env': {}}, 'foreign': {'env': {}}}), self.drv)
