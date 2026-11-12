"""Strict v3 raw-readiness consumer. No reads of /proc, RPC, signing or hashing work.

The caller must supply independently bound run/process/clock facts. Structural
fixture validation does not admit a bundle or prove live block propagation.
"""
import ipaddress
from pathlib import PurePosixPath
import re

from common import NODE_SHA, require
from pool_control import strict_json
from readiness_proof import (FORM, PROOF_FIELDS as V2_FIELDS, closed, endpoint,
                             handshake, integer, pairs, text)

SOURCE_LIMITS = {'max_rpc_call_ms': 10000, 'max_peer_rows': 256,
                 'max_tcp_rows': 8192, 'max_fd_entries': 8192}
MAX_ACQUISITION_MS = 90000
PROOF_FIELDS = V2_FIELDS | set('handshake_sample_a peer_sample_a_at_mono_ms peer_sample_b_at_mono_ms '
                              'dialer_netns_identity acceptor_netns_identity host_netns_identity '
                              'dialer_exe acceptor_exe raw_bracket_before raw_bracket_after'.split())
RAW_PROCESS = set('pid start_ticks netns_identity exe fd_socket_inodes owned_tcp_rows'.split())
BRACKET = set('boot_id host_netns_identity processes sampled_at_mono_ms'.split())
REPORT = set('status detail contract_version evidence_form acquisition planned_edges covered_edges '
             'planned_dials covered_dials nodes proofs'.split())
NODE = set('node outbound inbound listener_owned peer_samples listener_evidence process_binding'.split())
PROCESS_BINDING = set('run_id pid start_time_ticks executable_path executable_sha256 observed_exe '
                      'netns_identity host_netns_identity boot_id'.split())
LISTENERS = set('rpc_listen p2p_listen rpc_before rpc_after p2p_before p2p_after'.split())
IDENTITY = ('id', 'addr', 'addrbind', 'inbound', 'version', 'subver', 'services')


def netns(value):
    require(type(value) is list and len(value) == 2, 'v3 observed namespace identity required')
    integer(value[0]); integer(value[1], minimum=1)
    return tuple(value)


def executable(value):
    text(value)
    path = PurePosixPath(value)
    require(path.is_absolute() and str(path) == value and '..' not in path.parts
            and value.startswith('/nix/store/'), 'v3 source-bound executable path required')


def boot(value):
    require(type(value) is str and re.fullmatch(r'[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}', value), 'v3 boot identity')


def raw_endpoint(value):
    """Kernel projection also contains wildcard/zero remotes on LISTEN rows."""
    text(value)
    try:
        host, port = value.rsplit(':', 1)
        address = ipaddress.ip_address(host[1:-1] if host.startswith('[') and host.endswith(']') else host)
        require(re.fullmatch(r'0|[1-9][0-9]{0,4}', port) and int(port) <= 65535, 'v3 raw port')
        rendered = f'[{address}]:{int(port)}' if address.version == 6 else f'{address}:{int(port)}'
        require(rendered == value, 'v3 canonical raw endpoint')
    except (ValueError, TypeError):
        require(False, 'v3 raw socket endpoint')


def raw_process(value, limits):
    closed(value, RAW_PROCESS, 'v3 raw process')
    integer(value['pid'], bits=32, minimum=1)
    integer(value['start_ticks'], minimum=1)
    netns(value['netns_identity']); executable(value['exe'])
    fds = value['fd_socket_inodes']
    require(type(fds) is list and len(fds) <= limits['max_fd_entries'], 'v3 fd observation absent/over bound')
    for inode in fds:
        integer(inode, minimum=1)
    require(fds == sorted(set(fds)), 'v3 fd inode set must be sorted unique')
    rows = value['owned_tcp_rows']
    require(type(rows) is list and len(rows) <= limits['max_tcp_rows'], 'v3 TCP observation absent/over bound')
    seen = set()
    for row in rows:
        closed(row, {'local', 'remote', 'state', 'inode'}, 'v3 raw TCP')
        raw_endpoint(row['local']); raw_endpoint(row['remote'])
        integer(row['inode'], minimum=1)
        require(type(row['state']) is str and re.fullmatch(r'[0-9A-F]{2}', row['state']), 'v3 raw TCP state')
        require(row['inode'] in fds, 'v3 TCP row is not owned by designated fd table')
        key = (row['local'], row['remote'], row['state'], row['inode'])
        require(key not in seen, 'v3 duplicated raw TCP row')
        seen.add(key)


def bracket(value, nodes, limits):
    closed(value, BRACKET, 'v3 raw bracket')
    boot(value['boot_id']); netns(value['host_netns_identity'])
    integer(value['sampled_at_mono_ms'])
    require(type(value['processes']) is dict and set(value['processes']) == set(nodes), 'v3 raw process census')
    for process in value['processes'].values():
        raw_process(process, limits)
        require(process['netns_identity'] == value['host_netns_identity'], 'v3 node namespace differs from observed host')


def identity(row):
    return tuple(row[k] for k in IDENTITY)


def peer_pair(a, b):
    handshake(a); handshake(b)
    require(identity(a) == identity(b), 'v3 peer identity changed between reads')
    for name in ('bytesrecv_version', 'bytesrecv_verack', 'bytesrecv_getheaders'):
        if a[name] is not None:
            require(b[name] is not None and b[name] >= a[name], 'v3 peer counters moved backwards/disappeared')


def joined_row(process, local, remote, state, inode):
    matches = [row for row in process['owned_tcp_rows'] if row['local'] == local and row['remote'] == remote
               and row['state'] == state and row['inode'] == inode]
    require(len(matches) == 1 and inode in process['fd_socket_inodes'], 'v3 raw tuple/fd evidence missing or ambiguous')


def directed_proof(value, limits=None):
    limits = SOURCE_LIMITS if limits is None else limits
    closed(value, PROOF_FIELDS, 'v3 directed proof')
    require(type(value['contract_version']) is int and value['contract_version'] == 3
            and value['evidence_form'] == FORM, 'v3 required; older proof cannot be extended or relabelled')
    text(value['command_id'], nullable=True)  # Null is structural-only, never runtime-bound.
    for role in ('dialer', 'acceptor'):
        text(value[role]); integer(value[role + '_pid'], bits=32, minimum=1)
        integer(value[role + '_start_time_ticks'], minimum=1)
        require(value['network_namespace_' + role] is None, 'v3 host-only declared namespace')
        netns(value[role + '_netns_identity']); executable(value[role + '_exe'])
    require(value['dialer'] != value['acceptor'] and value['dialer_pid'] != value['acceptor_pid'], 'v3 self/process dial')
    boot(value['boot_id']); netns(value['host_netns_identity'])
    endpoint(value['planned_target'])
    peer_pair(value['handshake_sample_a'], value['handshake'])
    row = value['handshake']
    require(row['addrbind'] == value['planned_target'], 'v3 peer target differs')
    require(value['snapshot_confirmed'] is True and value['start_verified'] is True
            and value['getheaders_corroboration_required'] is False, 'v3 summary flags contradict required evidence')
    stamps = [value[k] for k in ('bracket_before_mono_ms', 'peer_sample_a_at_mono_ms',
                                'peer_sample_b_at_mono_ms', 'bracket_after_mono_ms')]
    for stamp in stamps:
        integer(stamp)
    integer(value['bracket_interval_ms'])
    require(stamps == sorted(stamps) and value['bracket_interval_ms'] == stamps[-1] - stamps[0], 'v3 bracket/sample order')
    for side, stamp in (('before', stamps[0]), ('after', stamps[-1])):
        raw = value['raw_bracket_' + side]
        bracket(raw, (value['dialer'], value['acceptor']), limits)
        require(raw['sampled_at_mono_ms'] == stamp and raw['boot_id'] == value['boot_id']
                and raw['host_netns_identity'] == value['host_netns_identity'], 'v3 raw bracket summary substitution')
        for role, local, remote in (('dialer', row['addr'], row['addrbind']), ('acceptor', row['addrbind'], row['addr'])):
            process = raw['processes'][value[role]]
            require(process['pid'] == value[role + '_pid'] and process['start_ticks'] == value[role + '_start_time_ticks']
                    and process['exe'] == value[role + '_exe']
                    and process['netns_identity'] == value[role + '_netns_identity'], 'v3 raw process differs from proof binding')
            sock = value[role + '_socket']
            closed(sock, {'local', 'remote', 'state', 'inode', 'fd_owned'}, 'v3 endpoint join')
            integer(sock['inode'], minimum=1)
            require(sock['local'] == local and sock['remote'] == remote and sock['state'] == '01'
                    and sock['fd_owned'] is True, 'v3 exact reversed ESTABLISHED endpoints required')
            joined_row(process, local, remote, '01', sock['inode'])
    require(type(value['endpoint_inodes_differ']) is bool and value['endpoint_inodes_differ'] ==
            (value['dialer_socket']['inode'] != value['acceptor_socket']['inode']), 'v3 endpoint inode summary contradiction')
    return value


def decode_proof(raw):
    require(len(raw) <= 16 * 1024 * 1024, 'v3 proof byte bound')
    return directed_proof(strict_json(raw))


def check_acquisition(value, *, operation_deadline_ms, command_started_ms, success_observed_ms, hash_start_ms):
    closed(value, {'started_at_mono_ms', 'deadline_mono_ms', 'completed_at_mono_ms', 'bounds'}, 'v3 acquisition')
    for key in ('started_at_mono_ms', 'deadline_mono_ms', 'completed_at_mono_ms'):
        integer(value[key])
    for v in (operation_deadline_ms, command_started_ms, success_observed_ms, hash_start_ms):
        integer(v)
    start, end, completed = (value[k] for k in ('started_at_mono_ms', 'deadline_mono_ms', 'completed_at_mono_ms'))
    bounds = value['bounds']
    closed(bounds, set(SOURCE_LIMITS) | {'max_acquisition_ms'}, 'v3 acquisition bounds')
    for v in bounds.values():
        integer(v)
    require(all(bounds[k] == v for k, v in SOURCE_LIMITS.items()), 'v3 untrusted source limits substituted')
    require(start <= (1 << 64) - 1 - MAX_ACQUISITION_MS, 'v3 deadline overflow')
    require(end == min(operation_deadline_ms, start + MAX_ACQUISITION_MS)
            and end >= start and bounds['max_acquisition_ms'] == end - start, 'v3 acquisition deadline/reset/bound mismatch')
    require(command_started_ms <= start <= completed <= success_observed_ms <= end
            and success_observed_ms <= hash_start_ms <= end and hash_start_ms < operation_deadline_ms,
            'v3 expired/reversed success or HashStart; completion alone is insufficient')
    return bounds


def listener_row(process, address, observation):
    closed(observation, {'state', 'inode', 'fd_membership'}, 'v3 listener observation')
    integer(observation['inode'], minimum=1)
    require(observation['state'] == '0A' and observation['fd_membership'] is True, 'v3 owned LISTEN evidence required')
    joined_row(process, address, '0.0.0.0:0', '0A', observation['inode'])


def check_ready_report(report, *, run_id, command_id, node_records, p2p_listeners, rpc_listeners,
                       connects, boot_id, host_netns_identity, operation_deadline_ms,
                       command_started_ms, success_observed_ms, hash_start_ms):
    """All keyword context is from independently bound manifest/journal evidence.

    This checks the observed record; canonical complete-source/binary admission
    and the first real common block remain independent requirements.
    """
    closed(report, REPORT, 'v3 report')
    require(type(report['contract_version']) is int and report['contract_version'] == 3
            and report['evidence_form'] == FORM and report['status'] == 'ready' and report['detail'] is None, 'v3 ready report required')
    text(run_id); text(command_id); boot(boot_id); netns(host_netns_identity)
    nodes = {'node-a', 'node-b'}
    require(set(node_records) == set(p2p_listeners) == set(rpc_listeners) == set(connects) == nodes, 'v3 exact two-node context')
    require(len(set(p2p_listeners.values()) | set(rpc_listeners.values())) == 4, 'v3 listener collision')
    for value in list(p2p_listeners.values()) + list(rpc_listeners.values()):
        endpoint(value)
    owners = {value: name for name, value in p2p_listeners.items()}
    require(all(connects[n] in owners and owners[connects[n]] != n for n in nodes), 'v3 unplanned directed dial')
    dials = {(n, owners[connects[n]]) for n in nodes}
    edges = {tuple(sorted(d)) for d in dials}
    require(pairs(report['planned_dials']) == pairs(report['covered_dials']) == dials
            and pairs(report['planned_edges']) == pairs(report['covered_edges']) == edges, 'v3 exact graph coverage')
    limits = check_acquisition(report['acquisition'], operation_deadline_ms=operation_deadline_ms,
                              command_started_ms=command_started_ms, success_observed_ms=success_observed_ms, hash_start_ms=hash_start_ms)
    require(type(report['proofs']) is list and len(report['proofs']) == 2, 'v3 directed proof census')
    proofs = {}
    before = after = None
    for value in report['proofs']:
        p = directed_proof(value, limits)
        pair = (p['dialer'], p['acceptor'])
        require(pair in dials and pair not in proofs, 'v3 duplicate/foreign proof')
        require(p['command_id'] == command_id and p['boot_id'] == boot_id
                and p['host_netns_identity'] == host_netns_identity and p['planned_target'] == connects[p['dialer']],
                'v3 operation/boot/host/target binding mismatch')
        require(endpoint(p['handshake']['addr'])[1] not in {endpoint(v)[1] for v in p2p_listeners.values()}, 'v3 listener is not ephemeral endpoint')
        if before is None:
            before, after = p['raw_bracket_before'], p['raw_bracket_after']
        require(before == p['raw_bracket_before'] and after == p['raw_bracket_after'], 'v3 mixed raw acquisition passes')
        proofs[pair] = p
    require(report['acquisition']['started_at_mono_ms'] <= before['sampled_at_mono_ms']
            <= after['sampled_at_mono_ms'] <= report['acquisition']['completed_at_mono_ms'], 'v3 raw reads outside acquisition')
    require(type(report['nodes']) is list and len(report['nodes']) == 2, 'v3 node report census')
    seen = set(); sample_stamps = set()
    for node in report['nodes']:
        closed(node, NODE, 'v3 node')
        name = node['node']
        require(type(name) is str and name in nodes and name not in seen, 'v3 duplicate/foreign node')
        seen.add(name)
        require(node['listener_owned'] is True, 'v3 listener summary unready')
        binding, expected = node['process_binding'], node_records[name]
        closed(binding, PROCESS_BINDING, 'v3 process binding')
        integer(binding['pid'], bits=32, minimum=1); integer(binding['start_time_ticks'], minimum=1)
        text(binding['run_id']); executable(binding['executable_path']); executable(binding['observed_exe'])
        netns(binding['netns_identity']); netns(binding['host_netns_identity']); boot(binding['boot_id'])
        require(type(binding['executable_sha256']) is str and re.fullmatch(r'[0-9a-f]{64}', binding['executable_sha256']), 'v3 executable digest type')
        integer(expected['pid'], bits=32, minimum=1); integer(expected['start_time_ticks'], minimum=1)
        executable(expected['executable_path'])
        require(expected['executable_sha256'] == NODE_SHA, 'v3 node executable not pinned release')
        require(binding['run_id'] == run_id and binding['pid'] == expected['pid']
                and binding['start_time_ticks'] == expected['start_time_ticks']
                and binding['executable_path'] == binding['observed_exe'] == expected['executable_path']
                and binding['executable_sha256'] == expected['executable_sha256']
                and binding['boot_id'] == boot_id and binding['netns_identity'] == binding['host_netns_identity'] == host_netns_identity,
                'v3 manifest/run/source process binding mismatch')
        listeners = node['listener_evidence']
        closed(listeners, LISTENERS, 'v3 listeners')
        require(listeners['rpc_listen'] == rpc_listeners[name] and listeners['p2p_listen'] == p2p_listeners[name], 'v3 actual listener differs from plan')
        for side, raw in (('before', before), ('after', after)):
            process = raw['processes'][name]
            require(process['pid'] == expected['pid'] and process['start_ticks'] == expected['start_time_ticks']
                    and process['exe'] == expected['executable_path'], 'v3 raw process differs from admitted manifest')
            for kind, addresses in (('rpc', rpc_listeners), ('p2p', p2p_listeners)):
                listener_row(process, addresses[name], listeners[kind + '_' + side])
        for kind in ('rpc', 'p2p'):
            require(listeners[kind + '_before']['inode'] == listeners[kind + '_after']['inode'], 'v3 listener rebound during acquisition')
        dual = node['peer_samples']; closed(dual, {'a', 'b'}, 'v3 dual peers')
        samples = []
        for key in ('a', 'b'):
            sample = dual[key]; closed(sample, {'rows', 'sampled_at_mono_ms'}, 'v3 peer sample')
            integer(sample['sampled_at_mono_ms'])
            require(type(sample['rows']) is list and len(sample['rows']) <= limits['max_peer_rows'], 'v3 peer cardinality')
            for row in sample['rows']:handshake(row, completed=False)
            require(len({row['id'] for row in sample['rows']}) == len(sample['rows']), 'v3 duplicate peer id')
            samples.append(sample)
        a, b = samples
        require(before['sampled_at_mono_ms'] <= a['sampled_at_mono_ms'] <= b['sampled_at_mono_ms'] <= after['sampled_at_mono_ms'], 'v3 peer read timestamp outside bracket')
        sample_stamps.add((a['sampled_at_mono_ms'], b['sampled_at_mono_ms']))
        require(sorted(identity(r) for r in a['rows']) == sorted(identity(r) for r in b['rows']), 'v3 peer set changed between samples')
        require(node['inbound'] == [r['addr'] for r in b['rows'] if r['inbound']]
                and node['outbound'] == [r['addr'] for r in b['rows'] if not r['inbound']], 'v3 peer direction relabelled')
        require(len(node['inbound']) == 1 and len(node['outbound']) <= 1
                and all(addr == connects[name] for addr in node['outbound']), 'v3 unexpected P2P peer')
        incoming = next(p for (d, acceptor), p in proofs.items() if acceptor == name)
        require(incoming['handshake_sample_a'] in a['rows'] and incoming['handshake'] in b['rows']
                and incoming['peer_sample_a_at_mono_ms'] == a['sampled_at_mono_ms']
                and incoming['peer_sample_b_at_mono_ms'] == b['sampled_at_mono_ms'], 'v3 proof is not the actual dual peer row')
        allowed = set()
        for p in proofs.values():
            role = 'dialer' if p['dialer'] == name else 'acceptor'
            s = p[role + '_socket']
            allowed.add((s['local'], s['remote'], s['state'], s['inode']))
        for raw in (before, after):
            for s in raw['processes'][name]['owned_tcp_rows']:
                if s['state'] != '0A' and (s['local'] in owners or s['remote'] in owners):
                    require((s['local'], s['remote'], s['state'], s['inode']) in allowed,
                            'v3 additional unproved P2P transport')
    require(len(sample_stamps) == 1, 'v3 node samples are from different passes')
    return {'contract_version': 3, 'directed_proofs_checked': len(proofs), 'raw_before_after_checked': True,
            'source_admission_conferred': False, 'live_propagation_proved': False}


def decode_ready_report(raw, **context):
    require(len(raw) <= 64 * 1024 * 1024, 'v3 report byte bound')
    return check_ready_report(strict_json(raw), **context)
