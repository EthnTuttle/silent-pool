"""Durable-root Unix socket limits, without generating keys or starting actors."""
import os
from pathlib import Path
import unittest

from common import Blocked
from prepare_shared_inputs import validate_socket_paths, validate_service_socket_paths
from scenario import scenario


class PreparedSocketPaths(unittest.TestCase):
    def test_n14a_root_fits_wallet_but_rejects_actual_ark_postgres_layout(self):
        root = Path('/home/ethan/.local/share/silent-pool/ap5-runs/n14a')
        validate_socket_paths(root, 'alice', 'collector-alice.sock')
        with self.assertRaisesRegex(Blocked, 'Ark PostgreSQL Unix socket'):
            validate_service_socket_paths(root, scenario('unit', 1))

    def test_short_root_covers_both_pool_databases_and_ark_postgres(self):
        rows = validate_service_socket_paths(Path('/home/ethan/.local/share/silent-pool/r/b14'), scenario('unit', 1))
        self.assertEqual(len(rows), 3)
        self.assertTrue(all(r['bytes'] < 108 for r in rows))

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
