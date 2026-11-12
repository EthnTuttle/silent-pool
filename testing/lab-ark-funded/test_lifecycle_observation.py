"""Orchestration race cases; fake decoded objects are not cryptographic evidence."""
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch

from ap5 import Controller, p2pkh_address
from common import Blocked, Evidence, encoded, stock_id_bytes
from scenario import scenario


class LifecycleObservation(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='ap5-lifecycle-', dir='/tmp/opencode')
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.ev = Evidence(self.root)
        self.addCleanup(self.ev.close)
        self.c = Controller({}, {}, self.root, self.ev, scenario('unit-only', 1), 30)
        self.c.run.mkdir()
        self.anchor = '11' * 32
        self.child_id = '22' * 32 + ':1'
        self.other_id = '33' * 32 + ':0'
        self.key = '02' + '44' * 32
        self.decoded = {'id': self.child_id, 'chain_anchor': self.anchor + ':0', 'user_pubkey': self.key,
                        'amount_sat': 900, 'expiry_height': 600}
        self.checkpoint = {'version': 2, 'locked': [[1]], 'proof_binding': [2] * 32, 'released': [[3, 4, 5]]}
        self.child = {'id': self.child_id, 'amount_sats': 900, 'expiry_height': 600, 'state': 'Spent'}
        self.other = {'id': self.other_id, 'amount_sats': 850, 'expiry_height': 900, 'state': 'Spendable'}
        self.c.decode_vtxo = Mock(return_value=self.decoded)
        self.c.user_stock_keys = Mock(return_value={self.key})
        self.c.fetch_stock_proof = Mock(side_effect=AssertionError('historical checkpoint must not use remote GetVtxo'))
        def verify(raw, decoded, owner):
            self.c.proofs[decoded['id']] = {'raw_hex': raw, 'expected': {'user_pubkey': owner}}
        self.c.verify_public_proof = Mock(side_effect=verify)

    def test_first_round_uses_only_retained_outputs_even_when_already_spent(self):
        children, proofs = self.c.retained_round_children('alice', self.checkpoint,
                                                        {'stock': [self.other, self.child]}, self.anchor)
        self.assertEqual(children, [self.child])
        self.assertEqual(proofs[0]['raw_hex'], '030405')
        self.c.verify_public_proof.assert_called_once_with('030405', self.decoded, self.key)
        self.c.fetch_stock_proof.assert_not_called()

    def test_missing_duplicate_or_malformed_released_outputs_reject(self):
        for released in (None, [], [[True]], [[256]], [[3, 4, 5], [3, 4, 5]]):
            c = {**self.checkpoint, 'released': released}
            with self.subTest(released=released), self.assertRaises(Blocked):
                self.c.retained_round_children('alice', c, {'stock': [self.child]}, self.anchor)
        with self.assertRaisesRegex(Blocked, 'missing/duplicate'):
            self.c.retained_round_children('alice', self.checkpoint, {'stock': [self.other]}, self.anchor)

    def test_retained_output_must_match_wallet_key_value_expiry_and_original_anchor(self):
        for key, value in [('user_pubkey', '03' + '55' * 32), ('amount_sat', 901),
                           ('expiry_height', 601), ('chain_anchor', '66' * 32 + ':0')]:
            self.c.decode_vtxo.return_value = {**self.decoded, key: value}
            with self.subTest(key=key), self.assertRaisesRegex(Blocked, 'wallet/anchor mismatch'):
                self.c.retained_round_children('alice', self.checkpoint, {'stock': [self.child]}, self.anchor)

    def test_retained_output_crypto_failure_propagates(self):
        self.c.verify_public_proof.side_effect = Blocked('invalid signed Full proof')
        with self.assertRaisesRegex(Blocked, 'invalid signed Full'):
            self.c.retained_round_children('alice', self.checkpoint, {'stock': [self.child]}, self.anchor)

    def snapshot(self, stage='stock_submitted'):
        ids = [stock_id_bytes(self.child_id)]
        return {'stock': [self.child, self.other], 'operations': [
            {'id': '77' * 32, 'stage': stage, 'result': 'round_submitted:' + '88' * 32,
             'payload': json.dumps({'operation_id': '77' * 32, 'vtxo_ids': ids, 'action': {'kind': 'refresh'}})}]}

    def test_completed_renewal_already_in_first_snapshot_is_still_a_candidate(self):
        snapshot = self.snapshot()
        self.c.registrations = Mock(return_value={'alice': {'registered': True}})
        with patch('ap5.wallet_snapshot', return_value=snapshot):
            value = self.c.renewal_candidate('alice', {stock_id_bytes(self.child_id)}, 600, 'first-operation')
        self.assertEqual(value['outputs'], [self.other])
        self.assertEqual(value['operations'], snapshot['operations'])
        # This method never declares an automatic or cryptographic pass.
        self.assertNotIn('automatic', value)

    def test_renewal_candidate_requires_every_old_input_and_nonambiguous_stock_stage(self):
        self.c.registrations = Mock(return_value=True)
        for kind in ('missing-input', 'unknown-operation', 'old-not-spent', 'no-live-registration'):
            snapshot = copy.deepcopy(self.snapshot())
            ids = {stock_id_bytes(self.child_id)}
            if kind == 'missing-input':ids.add('99' * 36)
            elif kind == 'unknown-operation':snapshot['operations'][0]['stage'] = 'stock_unknown'
            elif kind == 'old-not-spent':snapshot['stock'][0]['state'] = 'Locked'
            else:self.c.registrations.return_value = None
            with self.subTest(kind=kind), patch('ap5.wallet_snapshot', return_value=snapshot):
                self.assertIsNone(self.c.renewal_candidate('alice', ids, 600, 'first-operation'))

    def journal(self, driver, user_index, tail=b'', terminated=True):
        path = self.c.run / (driver + '.jsonl')
        row = {'record': 'funding_observed', 'seq': 2, 'outpoint': 'ab' * 32 + ':0',
               'amount_sats': 1000, 'script_hex': '76a914' + 'cd' * 20 + '88ac', 'chain': 'receive',
               'index': 0, 'owner_role': {'kind': 'user_payment', 'index': user_index},
               'source': 'bootstrap_distribution', 'context': {}, 'observed_unix_ms': 1}
        path.write_bytes(encoded(row) + (b'\n' if terminated else b'') + tail)
        return path, row

    def test_return_address_skips_other_drivers_incomplete_tail(self):
        a, _ = self.journal('txd-0', 0, tail=b'{"record":')
        b, row = self.journal('txd-1', 2)
        self.c.manifest = lambda: {'traffic_driver_paths': {'txd-0': {'journal_path': str(a)}, 'txd-1': {'journal_path': str(b)}}}
        self.assertEqual(self.c.return_address('carol'), p2pkh_address(row['script_hex']))
        saved = json.loads(next(self.root.glob('*-allocated-return-address-carol.json')).read_text())
        self.assertEqual(saved['funding_observed'], row)
        self.assertEqual(saved['driver_id'], 'txd-1')

    def test_unterminated_allocation_cannot_be_used_and_duplicate_keys_reject(self):
        path, row = self.journal('txd-0', 0, terminated=False)
        self.c.manifest = lambda: {'traffic_driver_paths': {'txd-0': {'journal_path': str(path)}}}
        with self.assertRaisesRegex(Blocked, 'unavailable'):self.c.return_address('alice')
        path.write_bytes(encoded(row)[:-1] + b',"record":"funding_observed"}\n')
        with self.assertRaisesRegex(Blocked, 'duplicate'):self.c.return_address('alice')

    def test_return_address_refuses_journal_outside_owned_run(self):
        self.c.manifest = lambda: {'traffic_driver_paths': {'txd-0': {'journal_path': str(self.root / 'outside.jsonl')}}}
        with self.assertRaisesRegex(Blocked, 'escapes'):self.c.return_address('alice')
