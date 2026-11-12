"""Strict consumer of the delivered net_readiness v2 DTO, without I/O/actions.

Validating the retained projection is not source approval or live readiness.
The producer's absent raw bracket samples cannot be reconstructed here. The
source-review gaps must be resolved before admitting an execution bundle.
"""
import ipaddress
import re

from common import require
from pool_control import strict_json

FORM = 'receiver_handshake_with_owned_reverse_tcp'
PROOF_FIELDS = set('contract_version evidence_form command_id dialer acceptor planned_target handshake '
                  'getheaders_corroboration_required dialer_pid dialer_start_time_ticks acceptor_pid '
                  'acceptor_start_time_ticks network_namespace_dialer network_namespace_acceptor boot_id '
                  'dialer_socket acceptor_socket endpoint_inodes_differ bracket_before_mono_ms '
                  'bracket_after_mono_ms bracket_interval_ms snapshot_confirmed start_verified'.split())
ROW_FIELDS = set('id addr addrbind inbound version subver services startingheight bytesrecv_version '
                'bytesrecv_verack bytesrecv_getheaders transport_protocol_type connection_type'.split())
SOCKET_FIELDS = set('local remote state inode fd_owned'.split())
REPORT_FIELDS = set('status detail contract_version evidence_form planned_edges covered_edges '
                   'planned_dials covered_dials nodes proofs'.split())
NODE_FIELDS = set('node outbound inbound listener_owned rows'.split())


def closed(value, fields, label):
    require(type(value) is dict and set(value) == fields, f'readiness {label}: closed fields required')


def integer(value, bits=64, minimum=0, signed=False):
    high = (1 << (bits - int(signed))) - 1
    require(type(value) is int and minimum <= value <= high, 'readiness integer type/range')
    return value


def text(value, nullable=False):
    require((nullable and value is None) or (type(value) is str and bool(value)), 'readiness string type/value')
    return value


def endpoint(value):
    text(value)
    parts = value.split(':')
    require(len(parts) == 2 and re.fullmatch(r'[1-9][0-9]{0,4}', parts[1]), 'readiness literal IPv4 endpoint')
    try:
        address = ipaddress.IPv4Address(parts[0])
    except ipaddress.AddressValueError:
        require(False, 'readiness literal IPv4 endpoint')
    port = int(parts[1])
    require(str(address) == parts[0] and not address.is_unspecified and port <= 65535,
            'readiness exact non-wildcard endpoint')
    return str(address), port


def handshake(row, completed=True):
    closed(row, ROW_FIELDS, 'handshake')
    integer(row['id'], signed=True, minimum=-(1 << 63))
    integer(row['version'], signed=True, minimum=-(1 << 63))
    integer(row['startingheight'], signed=True, minimum=-(1 << 63))
    require(type(row['inbound']) is bool, 'readiness inbound must be boolean')
    endpoint(row['addr']); endpoint(row['addrbind'])
    require(type(row['subver']) is str, 'readiness subver type')
    require(type(row['services']) is str and re.fullmatch(r'[0-9a-f]{16}', row['services']),
            'readiness source-rendered services')
    for key in ('bytesrecv_version', 'bytesrecv_verack', 'bytesrecv_getheaders'):
        if row[key] is not None:
            integer(row[key])
    for key in ('transport_protocol_type', 'connection_type'):
        text(row[key], nullable=True)
    if completed:
        require(row['inbound'] is True and row['version'] == 70016 and row['subver'] == '/rbitcoin:0.8.99/',
                'readiness completed pinned inbound handshake required')
        require(int(row['services'], 16) & 0x809 == 0x809, 'readiness pinned service mask')
        for key in ('bytesrecv_version', 'bytesrecv_verack'):
            require(row[key] is not None and row[key] > 0, 'readiness missing completed handshake marker')
    return row


def directed_proof(value):
    """Validate the exact serialized projection; command_id may be null in a fixture."""
    closed(value, PROOF_FIELDS, 'directed proof')
    integer(value['contract_version'], bits=32)
    require(value['contract_version'] == 2 and value['evidence_form'] == FORM, 'readiness version/form')
    for key in ('command_id', 'network_namespace_dialer', 'network_namespace_acceptor', 'boot_id'):
        text(value[key], nullable=True)
    for key in ('dialer', 'acceptor'):
        text(value[key])
    require(value['dialer'] != value['acceptor'], 'readiness self dial')
    endpoint(value['planned_target'])
    for key in ('dialer_pid', 'acceptor_pid'):
        integer(value[key], bits=32, minimum=1)
    for key in ('dialer_start_time_ticks', 'acceptor_start_time_ticks'):
        integer(value[key], minimum=1)
    require(value['dialer_pid'] != value['acceptor_pid'], 'readiness same process at both endpoints')
    require(type(value['boot_id']) is str and re.fullmatch(
        r'[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}', value['boot_id']), 'readiness boot identity')
    require(value['getheaders_corroboration_required'] is False, 'readiness getheaders is optional in v2')
    require(value['snapshot_confirmed'] is True and value['start_verified'] is True,
            'readiness producer bracket/identity observation incomplete')
    row = handshake(value['handshake'])
    require(row['addrbind'] == value['planned_target'], 'readiness handshake target substitution')
    for key in ('dialer_socket', 'acceptor_socket'):
        sock = value[key]
        closed(sock, SOCKET_FIELDS, 'endpoint join')
        endpoint(sock['local']); endpoint(sock['remote'])
        integer(sock['inode'], minimum=1)
        require(sock['state'] == '01' and sock['fd_owned'] is True, 'readiness owned ESTABLISHED socket required')
    dialer, acceptor = value['dialer_socket'], value['acceptor_socket']
    require(dialer['local'] == acceptor['remote'] == row['addr']
            and dialer['remote'] == acceptor['local'] == row['addrbind'], 'readiness exact reversed tuples')
    require(type(value['endpoint_inodes_differ']) is bool
            and value['endpoint_inodes_differ'] == (dialer['inode'] != acceptor['inode']),
            'readiness endpoint inode observation contradicts joins')
    for key in ('bracket_before_mono_ms', 'bracket_after_mono_ms', 'bracket_interval_ms'):
        integer(value[key])
    require(value['bracket_after_mono_ms'] >= value['bracket_before_mono_ms']
            and value['bracket_interval_ms'] == value['bracket_after_mono_ms'] - value['bracket_before_mono_ms'],
            'readiness bracket arithmetic/order')
    return value


def decode_proof(raw):
    return directed_proof(strict_json(raw))


def pairs(value):
    require(type(value) is list and all(type(p) is list and len(p) == 2
            and all(type(n) is str and n for n in p) and p[0] != p[1] for p in value), 'readiness graph pair shape')
    result = {tuple(p) for p in value}
    require(len(result) == len(value), 'readiness duplicate graph pair')
    return result


def check_report_projection(report, *, identities, listeners, connects, command_id, boot_id,
                            window_start_ms, window_end_ms, max_interval_ms):
    """Bind reported facts to independently admitted context, not proof assertions.

    Time bounds must be supplied by the actual operation/source contract. There
    is deliberately no invented default bound. This function does not approve
    the producer or reconstruct unretained kernel/namespace/listener samples.
    """
    closed(report, REPORT_FIELDS, 'report')
    require(type(report['contract_version']) is int and report['contract_version'] == 2
            and report['evidence_form'] == FORM, 'readiness report version/form')
    require(report['status'] == 'ready' and report['detail'] is None, 'readiness report is not ready')
    text(command_id); text(boot_id)
    for number in (window_start_ms, window_end_ms, max_interval_ms):
        integer(number)
    require(window_start_ms <= window_end_ms and max_interval_ms > 0, 'readiness invalid observation bounds')
    require(set(identities) == set(listeners) == set(connects) == {'node-a', 'node-b'},
            'readiness exact AP5 two-node context')
    require(len(set(listeners.values())) == 2, 'readiness ambiguous planned listener')
    for target in listeners.values():
        endpoint(target)
    targets = {v: k for k, v in listeners.items()}
    require(all(connects[n] in targets and targets[connects[n]] != n for n in connects), 'readiness unplanned dial')
    expected = {(n, targets[connects[n]]) for n in connects}
    edges = {tuple(sorted(p)) for p in expected}
    require(pairs(report['planned_dials']) == pairs(report['covered_dials']) == expected
            and pairs(report['planned_edges']) == pairs(report['covered_edges']) == edges, 'readiness exact graph coverage')
    require(type(report['nodes']) is list and len(report['nodes']) == 2, 'readiness node census')
    nodes = {}
    for node in report['nodes']:
        closed(node, NODE_FIELDS, 'node')
        text(node['node'])
        require(node['node'] in identities and node['node'] not in nodes, 'readiness duplicate/foreign node')
        require(node['listener_owned'] is True, 'readiness listener observation incomplete')
        require(type(node['rows']) is list and len(node['rows']) > 0, 'readiness missing handshake rows')
        rows = [handshake(r, completed=False) for r in node['rows']]
        require(len({r['id'] for r in rows}) == len(rows), 'readiness duplicated peer id')
        require(node['inbound'] == [r['addr'] for r in rows if r['inbound']]
                and node['outbound'] == [r['addr'] for r in rows if not r['inbound']], 'readiness raw direction relabelled')
        require(len(node['inbound']) == 1 and len(node['outbound']) <= 1, 'readiness unexpected peer census')
        require(all(r['addr'] == connects[node['node']] for r in rows if not r['inbound']), 'readiness foreign outbound peer')
        nodes[node['node']] = node
    require(type(report['proofs']) is list and len(report['proofs']) == len(expected), 'readiness directed proof census')
    seen, brackets, matched_rows = set(), set(), set()
    for raw in report['proofs']:
        p = directed_proof(raw)
        pair = (p['dialer'], p['acceptor'])
        require(pair in expected and pair not in seen, 'readiness missing/duplicated directed proof')
        seen.add(pair)
        require(p['command_id'] == command_id and p['boot_id'] == boot_id, 'readiness operation/boot substitution')
        require(p['planned_target'] == connects[p['dialer']], 'readiness planned target substitution')
        require(endpoint(p['handshake']['addr'])[1] not in {endpoint(v)[1] for v in listeners.values()},
                'readiness listener port is not an ephemeral endpoint')
        for role in ('dialer', 'acceptor'):
            identity = identities[p[role]]
            require(p[f'{role}_pid'] == identity['pid'] and p[f'{role}_start_time_ticks'] == identity['start_time_ticks'],
                    'readiness process generation substitution')
            require(p[f'network_namespace_{role}'] is None and identity['network_namespace'] is None,
                    'readiness v2 engine supports host namespace only')
        require(window_start_ms <= p['bracket_before_mono_ms'] <= p['bracket_after_mono_ms'] <= window_end_ms
                and p['bracket_interval_ms'] <= max_interval_ms, 'readiness stale/out-of-window bracket')
        brackets.add((p['bracket_before_mono_ms'], p['bracket_after_mono_ms']))
        require(p['handshake'] in nodes[p['acceptor']]['rows'], 'readiness proof row differs from report')
        matched_rows.add((p['acceptor'], p['handshake']['id']))
    require(len(brackets) == 1, 'readiness mixed acquisition passes')
    require(matched_rows == {(n, r['id']) for n, node in nodes.items() for r in node['rows'] if r['inbound']},
            'readiness unproven inbound row')
    return {'contract_version': 2, 'directed_projection_count': len(seen),
            'source_admission_conferred': False, 'live_propagation_proved': False}
