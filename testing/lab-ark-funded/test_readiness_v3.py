"""Frozen v3 directed bytes and labelled synthetic complete reports; no live proof."""
import copy
import hashlib
import json
import os
from pathlib import Path
import unittest

from common import Blocked, NODE_SHA, check_bundle
from readiness_v3 import SOURCE_LIMITS, FORM, check_ready_report, decode_proof, decode_ready_report

FIXTURE = Path(os.environ.get('AP5_QA_TEST_SOURCE', str(Path(__file__).resolve().parents[2]))) / 'testing/lab-traffic/fixtures/readiness-v3/directed-dial-proof.json'
DIGEST = 'ec4a81f78fa5a921489ed91cd6625a06ca8a54cfe19c5ea4e2f590d854decdf8'


def fixture():
    raw = FIXTURE.read_bytes()
    assert hashlib.sha256(raw).hexdigest() == DIGEST
    return json.loads(raw)


def synthetic_report():
    p = fixture(); p['command_id'] = 'ap5-native-bootstrap'
    rpc = {'node-a': '127.0.0.1:44441', 'node-b': '127.0.0.1:44442'}
    listeners = {'node-a': '127.0.0.1:44919', 'node-b': '127.0.0.1:35167'}
    for side in ('before', 'after'):
        for i, name in enumerate(('node-a', 'node-b')):
            proc = p['raw_bracket_' + side]['processes'][name]
            proc['fd_socket_inodes'] = sorted(proc['fd_socket_inodes'] + [83001 + i])
            proc['owned_tcp_rows'].append({'local': rpc[name], 'remote': '0.0.0.0:0', 'state': '0A', 'inode': 83001 + i})
    q = copy.deepcopy(p)
    q.update(dialer='node-a', acceptor='node-b', planned_target=listeners['node-b'],
             dialer_pid=4101, acceptor_pid=4102, dialer_start_time_ticks=91234, acceptor_start_time_ticks=91235)
    for field in ('handshake', 'handshake_sample_a'):
        q[field].update(id=4, addr='127.0.0.1:47756', addrbind=listeners['node-b'])
    q['dialer_socket'].update(local='127.0.0.1:47756', remote=listeners['node-b'], inode=82008)
    q['acceptor_socket'].update(local=listeners['node-b'], remote='127.0.0.1:47756', inode=82011)
    report = {'status': 'ready', 'detail': None, 'contract_version': 3, 'evidence_form': FORM,
              'acquisition': {'started_at_mono_ms': 4900, 'deadline_mono_ms': 94900,
                              'completed_at_mono_ms': 5500, 'bounds': {**SOURCE_LIMITS, 'max_acquisition_ms': 90000}},
              'planned_edges': [['node-a', 'node-b']], 'covered_edges': [['node-a', 'node-b']],
              'planned_dials': [['node-a', 'node-b'], ['node-b', 'node-a']],
              'covered_dials': [['node-a', 'node-b'], ['node-b', 'node-a']], 'proofs': [p, q], 'nodes': []}
    expected = {}
    for proof in (p, q):
        name = proof['acceptor']; raw = p['raw_bracket_before']['processes'][name]
        expected[name] = {'pid': raw['pid'], 'start_time_ticks': raw['start_ticks'],
                          'executable_path': raw['exe'], 'executable_sha256': NODE_SHA}
        evidence = {'rpc_listen': rpc[name], 'p2p_listen': listeners[name]}
        for kind, addr in (('rpc', rpc[name]), ('p2p', listeners[name])):
            inode = next(r['inode'] for r in raw['owned_tcp_rows'] if r['state'] == '0A' and r['local'] == addr)
            for side in ('before', 'after'):
                evidence[kind + '_' + side] = {'state': '0A', 'inode': inode, 'fd_membership': True}
        report['nodes'].append({'node': name, 'outbound': [], 'inbound': [proof['handshake']['addr']], 'listener_owned': True,
            'peer_samples': {'a': {'rows': [copy.deepcopy(proof['handshake_sample_a'])], 'sampled_at_mono_ms': 5100},
                             'b': {'rows': [copy.deepcopy(proof['handshake'])], 'sampled_at_mono_ms': 5300}},
            'listener_evidence': evidence, 'process_binding': {'run_id': 'unit', **expected[name],
                 'observed_exe': raw['exe'], 'netns_identity': raw['netns_identity'],
                 'host_netns_identity': p['host_netns_identity'], 'boot_id': p['boot_id']}})
    context = {'run_id': 'unit', 'command_id': 'ap5-native-bootstrap', 'node_records': expected,
               'p2p_listeners': listeners, 'rpc_listeners': rpc,
               'connects': {'node-a': listeners['node-b'], 'node-b': listeners['node-a']},
               'boot_id': p['boot_id'], 'host_netns_identity': p['host_netns_identity'],
               'operation_deadline_ms': 100000, 'command_started_ms': 4000,
               'success_observed_ms': 5510, 'hash_start_ms': 5520}
    return report, context


class V3Readiness(unittest.TestCase):
    def test_readiness_version_cannot_replace_canonical_bundle_envelope(self):
        with self.assertRaisesRegex(Blocked, 'unsupported AP5 bundle'):
            check_bundle({'schema': 'spc.lab.readiness.bundle/v3', 'readiness_contract_version': 3}, full=True)

    def test_exact_directed_fixture_is_structural_only_with_null_operation(self):
        p = decode_proof(FIXTURE.read_bytes())
        self.assertIsNone(p['command_id'])
        self.assertEqual(p['contract_version'], 3)

    def test_v2_and_duplicate_unknown_or_missing_v3_fields_reject(self):
        old = fixture(); old['contract_version'] = 2
        old = json.dumps(old).encode()
        with self.assertRaises(Blocked):decode_proof(old)
        p = fixture()
        for field in ('raw_bracket_before', 'dialer_exe', 'handshake_sample_a'):
            value = copy.deepcopy(p);del value[field]
            with self.assertRaises(Blocked):decode_proof(json.dumps(value))
        with self.assertRaises(Blocked):decode_proof(json.dumps(p)[:-1] + ',"contract_version":3}')
        p['invented_success'] = True
        with self.assertRaises(Blocked):decode_proof(json.dumps(p))
        r, c = synthetic_report()
        with self.assertRaises(Blocked):decode_ready_report(json.dumps(r)[:-1] + ',"status":"ready"}', **c)

    def test_complete_synthetic_report_checks_raw_inputs_without_admitting_source(self):
        r, c = synthetic_report();v = check_ready_report(r, **c)
        self.assertEqual(v['directed_proofs_checked'], 2)
        self.assertFalse(v['source_admission_conferred'])
        self.assertFalse(v['live_propagation_proved'])

    def test_null_operation_and_foreign_executable_or_hash_cannot_bind_report(self):
        for case in ('null-operation', 'exe', 'hash', 'pid'):
            r, c = synthetic_report()
            if case == 'null-operation':r['proofs'][0]['command_id'] = None
            elif case == 'exe':c['node_records']['node-a']['executable_path'] = '/nix/store/foreign/bin/node'
            elif case == 'hash':c['node_records']['node-a']['executable_sha256'] = '00' * 32
            else:r['nodes'][0]['process_binding']['pid'] = True
            with self.subTest(case=case), self.assertRaises(Blocked):check_ready_report(r, **c)

    def test_completed_on_time_but_expired_success_or_HashStart_rejects(self):
        for changes in ({'success_observed_ms': 94901, 'hash_start_ms': 94902},
                        {'hash_start_ms': 94901}, {'hash_start_ms': 5509}, {'operation_deadline_ms': 5515}):
            r, c = synthetic_report();c.update(changes)
            with self.subTest(changes=changes), self.assertRaises(Blocked):check_ready_report(r, **c)

    def test_original_deadline_caps_acquisition_and_bounds_cannot_be_self_selected(self):
        r, c = synthetic_report();c['operation_deadline_ms'] = 6000
        r['acquisition']['deadline_mono_ms'] = 6000;r['acquisition']['bounds']['max_acquisition_ms'] = 1100
        check_ready_report(r, **c)
        for name in (*SOURCE_LIMITS, 'max_acquisition_ms'):
            changed = copy.deepcopy(r);changed['acquisition']['bounds'][name] += 1
            with self.subTest(name=name), self.assertRaises(Blocked):check_ready_report(changed, **c)
        del r['acquisition']
        with self.assertRaises(Blocked):check_ready_report(r, **c)

    def test_raw_fd_namespace_exe_state_and_tuple_mismatches_reject_despite_true_flags(self):
        for case in ('fd', 'namespace', 'exe', 'state', 'tuple', 'null'):
            p = fixture();process = p['raw_bracket_after']['processes']['node-b']
            if case == 'fd':process['fd_socket_inodes'].remove(82012)
            elif case == 'namespace':process['netns_identity'] = [0, 999]
            elif case == 'exe':process['exe'] = '/nix/store/foreign/bin/node'
            elif case == 'state':process['owned_tcp_rows'][-1]['state'] = '08';process['owned_tcp_rows'][1]['state'] = '08'
            elif case == 'tuple':process['owned_tcp_rows'][1]['remote'] = '127.0.0.1:44918'
            else:process['fd_socket_inodes'] = None
            with self.subTest(case=case), self.assertRaises(Blocked):decode_proof(json.dumps(p))

    def test_listener_summaries_require_before_and_after_raw_rows(self):
        for field in ('rpc_before', 'rpc_after', 'p2p_before', 'p2p_after'):
            r, c = synthetic_report();r['nodes'][0]['listener_evidence'][field]['inode'] += 1000
            with self.subTest(field=field), self.assertRaises(Blocked):check_ready_report(r, **c)

    def test_dual_peer_identity_and_time_stability(self):
        for case in ('id', 'direction', 'stamp', 'counter'):
            r, c = synthetic_report();sample = r['nodes'][0]['peer_samples']['a']
            if case == 'id':sample['rows'][0]['id'] += 1
            elif case == 'direction':sample['rows'][0]['inbound'] = False
            elif case == 'stamp':sample['sampled_at_mono_ms'] = 4999
            else:
                r['proofs'][0]['handshake']['bytesrecv_verack'] = 1
                r['nodes'][0]['peer_samples']['b']['rows'][0]['bytesrecv_verack'] = 1
            with self.subTest(case=case), self.assertRaises(Blocked):check_ready_report(r, **c)

    def test_counter_growth_is_valid_and_getheaders_remains_optional(self):
        p = fixture();p['handshake_sample_a']['bytesrecv_getheaders'] = None
        p['handshake']['bytesrecv_version'] += 30
        decode_proof(json.dumps(p))

    def test_duplicate_dial_unproved_transport_and_mixed_brackets_reject(self):
        for case in ('dial', 'extra', 'mixed'):
            r, c = synthetic_report()
            if case == 'dial':r['proofs'][1] = copy.deepcopy(r['proofs'][0])
            elif case == 'mixed':r['proofs'][1]['raw_bracket_after']['processes']['node-a']['fd_socket_inodes'].append(99999)
            else:
                for proof in r['proofs']:
                    for side in ('before', 'after'):
                        proc = proof['raw_bracket_' + side]['processes']['node-a']
                        proc['fd_socket_inodes'].append(99999)
                        proc['owned_tcp_rows'].append({'local': c['p2p_listeners']['node-a'], 'remote': '127.0.0.1:40000', 'state': '01', 'inode': 99999})
            with self.subTest(case=case), self.assertRaises(Blocked):check_ready_report(r, **c)

    def test_boolean_namespace_and_reversed_brackets_reject(self):
        p = fixture();p['host_netns_identity'][0] = False
        with self.assertRaises(Blocked):decode_proof(json.dumps(p))
        p = fixture();p['bracket_before_mono_ms'] = 5500
        with self.assertRaises(Blocked):decode_proof(json.dumps(p))

    def test_over_bound_raw_collections_are_rejected_not_truncated(self):
        for field, size in [('fd_socket_inodes', 8193), ('owned_tcp_rows', 8193)]:
            p = fixture();proc = p['raw_bracket_before']['processes']['node-a']
            proc[field] = list(range(1, size + 1)) if field == 'fd_socket_inodes' else [proc[field][0]] * size
            with self.subTest(field=field), self.assertRaisesRegex(Blocked, 'over bound'):
                decode_proof(json.dumps(p))
        r, c = synthetic_report();sample = r['nodes'][0]['peer_samples']['a']
        sample['rows'] = sample['rows'] * 257
        with self.assertRaisesRegex(Blocked, 'peer cardinality'):check_ready_report(r, **c)
