"""Actual producer bytes plus labelled synthetic mutations; no live observations."""
import copy
import hashlib
import json
from pathlib import Path
import unittest

from common import Blocked
from readiness_proof import FORM, decode_proof, directed_proof, check_report_projection

PACKET = Path('/tmp/opencode/ibd-readiness-owned-socket-source-r1')
SOURCE = Path('/nix/store/zlvpj4d3awm8g0hppcna5pdw9lannhc0-source')
SOURCE_FILES = {
    'crates/spc-lab-txd/src/net_readiness.rs': '66656cbe1d720fe7e43045a112519a8c6ac9c6ae2e710c19dc006edf11da8d73',
    'crates/spc-lab-txd/src/net_readiness/tests.rs': 'df144496fecc781034d47d58f48810fe9481741c123b22820c5bdc5ebbf07582',
    'crates/spc-lab-txd/src/net_readiness/fixture_directed_dial_proof.json': 'dc2eec974d5b86300ceb9094001f79c466bb1bc90c77ab6b98944bc8c6c10637',
    'crates/spc-lab-txd/src/lib.rs': '6ccfeae9da4805b87137eb99c00d6996e45cd3fb2a1580e37819d30c32228d29',
    'crates/spc-lab-txd/src/native_bootstrap/tests.rs': '0306945ec5db4a13ce09325ba8280bc8446e96ff454d1bbf9443cc8336269d3c',
    'crates/spc-lab/src/engine/traffic_runtime.rs': 'f914f50107fa10aa6f1d70a7f951497924b0f0eea50e026476ea722266c2d67e',
    'crates/spc-lab/src/evidence.rs': 'd387c110613361b48bae3423b88d32d7a1ae3b766f2d21550e9f9ce84b2722e6',
    'docs/decisions/2026-11-10-node-network-readiness-owned-socket-proof.md': '3ff5e8553b96f93df855b8c329b024d891839decaef2850bba320d2b717611e6',
    'docs/changelog/2026-11-10-node-network-readiness-owned-socket-proof.md': '8ff0881be76f1fd5839b2ad03efae603972d24c133000a209f3f7a8acadd15ae',
}


def bound_bytes(name, digest):
    raw = (PACKET / name).read_bytes()
    assert hashlib.sha256(raw).hexdigest() == digest
    return raw


def fixture():
    return json.loads(bound_bytes('fixture-directed-dial-proof.json',
                                 SOURCE_FILES['crates/spc-lab-txd/src/net_readiness/fixture_directed_dial_proof.json']))


def synthetic_report():
    """Synthetic report/context only: never presented as the actual regression report."""
    proof = fixture()
    proof['command_id'] = 'unit-bootstrap'
    other = copy.deepcopy(proof)
    other.update(dialer='node-a', acceptor='node-b', planned_target='127.0.0.1:35167',
                 dialer_pid=4101, acceptor_pid=4102, dialer_start_time_ticks=91234, acceptor_start_time_ticks=91235)
    other['handshake'].update(id=4, addr='127.0.0.1:47756', addrbind='127.0.0.1:35167')
    other['dialer_socket'].update(local='127.0.0.1:47756', remote='127.0.0.1:35167', inode=83001)
    other['acceptor_socket'].update(local='127.0.0.1:35167', remote='127.0.0.1:47756', inode=83002)
    dials = [['node-a', 'node-b'], ['node-b', 'node-a']]
    edges = [['node-a', 'node-b']]
    report = dict(status='ready', detail=None, contract_version=2, evidence_form=FORM,
                  planned_dials=dials, covered_dials=copy.deepcopy(dials), planned_edges=edges,
                  covered_edges=copy.deepcopy(edges), proofs=[proof, other], nodes=[])
    for p in report['proofs']:
        report['nodes'].append(dict(node=p['acceptor'], outbound=[], inbound=[p['handshake']['addr']],
                                    listener_owned=True, rows=[copy.deepcopy(p['handshake'])]))
    context = dict(identities={'node-a': dict(pid=4101, start_time_ticks=91234, network_namespace=None),
                               'node-b': dict(pid=4102, start_time_ticks=91235, network_namespace=None)},
                   listeners={'node-a': '127.0.0.1:44919', 'node-b': '127.0.0.1:35167'},
                   connects={'node-a': '127.0.0.1:35167', 'node-b': '127.0.0.1:44919'},
                   command_id='unit-bootstrap', boot_id=proof['boot_id'],
                   window_start_ms=4900, window_end_ms=5500, max_interval_ms=500)
    return report, context


class ReadinessProjection(unittest.TestCase):
    def test_exact_producer_source_rows_and_frozen_fixture(self):
        for name, digest in SOURCE_FILES.items():
            self.assertEqual(hashlib.sha256((SOURCE / name).read_bytes()).hexdigest(), digest, name)
        p = decode_proof((PACKET / 'fixture-directed-dial-proof.json').read_bytes())
        self.assertIsNone(p['command_id'])
        self.assertNotEqual(p['dialer_socket']['inode'], p['acceptor_socket']['inode'])

    def test_actual_regression_projections_are_unstamped_not_runtime_admission(self):
        raw = bound_bytes('ap5-real-run-proofs.jsonl', '1450f54909d6303c7883d414d7ed76cc9690eec92bc211f0d38c6e22a94abc0b')
        proofs = [decode_proof(line) for line in raw.splitlines()]
        self.assertEqual({(p['dialer'], p['acceptor']) for p in proofs}, {('node-a', 'node-b'), ('node-b', 'node-a')})
        self.assertTrue(all(p['command_id'] is None and p['handshake']['bytesrecv_getheaders'] is None for p in proofs))

    def test_closed_duplicate_and_missing_fields_reject(self):
        p = fixture()
        with self.assertRaises(Blocked):decode_proof(json.dumps(p)[:-1] + ',"contract_version":2}')
        for action in ('extra', 'missing', 'legacy'):
            value = copy.deepcopy(p)
            if action == 'extra':value['inferred_peer_owner'] = True
            elif action == 'missing':del value['handshake']['inbound']
            else:value['contract_version'] = 1
            with self.subTest(action=action), self.assertRaises(Blocked):directed_proof(value)

    def test_connecting_foreign_and_false_handshake_markers_reject(self):
        for key, bad in [('inbound', False), ('inbound', 1), ('version', 0), ('version', 310000),
                         ('version', True), ('subver', '/foreign/'), ('services', '0000000000000009'),
                         ('services', '809'), ('bytesrecv_verack', None), ('bytesrecv_version', 0),
                         ('bytesrecv_verack', False), ('bytesrecv_getheaders', -1)]:
            p = fixture();p['handshake'][key] = bad
            with self.subTest(key=key, bad=bad), self.assertRaises(Blocked):directed_proof(p)

    def test_both_socket_owners_and_states_required(self):
        for side in ('dialer_socket', 'acceptor_socket'):
            for key, bad in [('fd_owned', False), ('fd_owned', 1), ('inode', 0), ('inode', True),
                             ('state', '0A'), ('state', '02'), ('state', '06'), ('state', '08')]:
                p = fixture();p[side][key] = bad
                with self.subTest(side=side, key=key, bad=bad), self.assertRaises(Blocked):directed_proof(p)

    def test_ip_only_port_suffix_and_wrong_reverse_tuples_reject(self):
        for bad in ('127.0.0.2:44919', '127.0.0.1:4491', '127.0.0.1:044919', 'localhost:44919', '[::1]:44919', '0.0.0.0:44919'):
            p = fixture();p['acceptor_socket']['local'] = bad
            with self.subTest(bad=bad), self.assertRaises(Blocked):directed_proof(p)

    def test_genesis_and_optional_getheaders_do_not_require_positive_height(self):
        p = fixture();p['handshake'].update(startingheight=-1, bytesrecv_getheaders=None)
        self.assertEqual(directed_proof(p)['handshake']['startingheight'], -1)

    def test_bracket_arithmetic_and_boolean_claims_reject(self):
        for key, bad in [('bracket_after_mono_ms', 4999), ('bracket_interval_ms', 399),
                         ('snapshot_confirmed', False), ('start_verified', 1), ('dialer_pid', True),
                         ('boot_id', None), ('endpoint_inodes_differ', False)]:
            p = fixture();p[key] = bad
            with self.subTest(key=key), self.assertRaises(Blocked):directed_proof(p)

    def test_synthetic_report_consistency_does_not_confer_admission(self):
        report, context = synthetic_report()
        result = check_report_projection(report, **context)
        self.assertEqual(result['directed_projection_count'], 2)
        self.assertFalse(result['source_admission_conferred'])
        self.assertFalse(result['live_propagation_proved'])

    def test_missing_duplicate_direction_and_extra_peer_reject(self):
        for case in ('missing', 'duplicate', 'wrong-coverage', 'extra-peer', 'relabelled', 'legacy'):
            r, context = synthetic_report()
            if case == 'missing':r['proofs'].pop()
            elif case == 'duplicate':r['proofs'][1] = copy.deepcopy(r['proofs'][0])
            elif case == 'wrong-coverage':r['covered_dials'].pop()
            elif case == 'extra-peer':r['nodes'][0]['rows'].append(copy.deepcopy(r['nodes'][0]['rows'][0]))
            elif case == 'relabelled':r['nodes'][0]['outbound'] = r['nodes'][0]['inbound'];r['nodes'][0]['inbound'] = []
            else:r.pop('proofs')
            with self.subTest(case=case), self.assertRaises(Blocked):check_report_projection(r, **context)

    def test_operation_identity_namespace_and_boot_substitution_reject(self):
        for key, bad in [('command_id', None), ('command_id', 'another-bootstrap'), ('dialer_pid', 9900),
                         ('dialer_start_time_ticks', 1), ('acceptor_start_time_ticks', 1),
                         ('network_namespace_dialer', 'other'), ('boot_id', '00000000-0000-0000-0000-000000000000')]:
            r, context = synthetic_report();r['proofs'][0][key] = bad
            with self.subTest(key=key), self.assertRaises(Blocked):check_report_projection(r, **context)

    def test_stale_long_late_and_mixed_brackets_reject(self):
        for case in ('start', 'end', 'duration', 'mixed-pass'):
            r, context = synthetic_report()
            if case == 'start':context['window_start_ms'] = 5001
            elif case == 'end':context['window_end_ms'] = 5399
            elif case == 'duration':context['max_interval_ms'] = 399
            else:
                p = r['proofs'][1];p['bracket_before_mono_ms'] += 1;p['bracket_after_mono_ms'] += 1
            with self.subTest(case=case), self.assertRaises(Blocked):check_report_projection(r, **context)

    def test_report_row_listener_and_target_bindings_reject(self):
        for case in ('row', 'listener', 'target', 'ephemeral'):
            r, context = synthetic_report()
            if case == 'row':r['nodes'][0]['rows'][0]['bytesrecv_version'] += 1
            elif case == 'listener':r['nodes'][0]['listener_owned'] = None
            elif case == 'target':context['connects']['node-a'] = context['listeners']['node-a']
            else:
                p = r['proofs'][0];p['handshake']['addr'] = '127.0.0.1:35167'
                p['dialer_socket']['local'] = p['acceptor_socket']['remote'] = p['handshake']['addr']
            with self.subTest(case=case), self.assertRaises(Blocked):check_report_projection(r, **context)
