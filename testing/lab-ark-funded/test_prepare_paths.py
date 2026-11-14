"""Durable-root Unix socket limits, without generating keys or starting actors."""
import os
from pathlib import Path
import unittest

from common import Blocked
from prepare_shared_inputs import validate_socket_paths


class PreparedSocketPaths(unittest.TestCase):
    def test_short_persistent_root_fits_both_actual_socket_layouts(self):
        validate_socket_paths(Path('/home/user/.local/share/lab/r1'), 'alice', 'collector-alice.sock')

    def test_collector_fit_does_not_hide_longer_native_socket_failure(self):
        root = Path('/' + 'x' * 54)
        self.assertLess(len(os.fsencode(root / 'qa/runtime/private/collector-alice.sock')), 108)
        with self.assertRaisesRegex(Blocked, 'native wallet Unix socket'):
            validate_socket_paths(root, 'alice', 'collector-alice.sock')

    def test_byte_limit_not_character_count(self):
        root = Path('/' + '\u00e9' * 40)
        self.assertLess(len(str(root / 'qa/runtime/private/collector-alice.sock')), 108)
        with self.assertRaises(Blocked):
            validate_socket_paths(root, 'alice', 'collector-alice.sock')
