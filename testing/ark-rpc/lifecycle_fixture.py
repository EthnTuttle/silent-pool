"""Real helper/probe/node lifetime fixtures, no mining or production services.

The fixture controller forks its probe, then execs the sealed spc-lab HTTP
command against a private parked socket. Before the probe executes actor-exec,
the parent is the REAL sealed spc-lab executable, with the SAME PID/start ticks.
Production validate_parent_handoff is never replaced. This is a lifetime test
controller, not an implementation of the J2 runtime preflight stage.
"""
import argparse
import ctypes
import decimal
import hashlib
import json
import os
from pathlib import Path
import signal
import socket
import sys
import time

from arkrpc import nodeproc
from arkrpc.contracts import get_profile, contract_hash
from arkrpc.launchprofile import normalize, digest
from arkrpc.probe import run_probe
from arkrpc.reporting import build_report
from arkrpc.schemadef import validate_report
from arkrpc.trafficfacts import effective_consensus

CASES = ("read-only-stop", "controller-kill", "probe-kill", "early-startup",
         "preinstall-parent-death", "partial-observed-failure", "cli-profile")
PROTECTED_PID = 199214
PROTECTED_PORTS = {18481, 36387, 43525, 34009}


def read_json(path):
    return json.loads(Path(path).read_bytes())


def write(path, value):
    def json_safe(v):
        if isinstance(v, decimal.Decimal):
            return str(v)
        if isinstance(v, dict):
            return {k: json_safe(x) for k, x in v.items()}
        if isinstance(v, list):
            return [json_safe(x) for x in v]
        return v
    nodeproc.private_json(str(path), json_safe(value))


def identity(pid):
    assert pid > 1 and pid != PROTECTED_PID
    return (pid, nodeproc.start_ticks(pid))


def exists(who):
    try:
        return nodeproc.start_ticks(who[0]) == who[1]
    except (OSError, ValueError):
        return False


def send(who, sig):
    assert who[0] > 1 and who[0] != PROTECTED_PID
    if exists(who):
        os.kill(who[0], sig)


def wait_file(path, timeout=20):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if path.exists():
            try:
                return read_json(path)
            except ValueError:
                pass
        time.sleep(0.01)
    raise RuntimeError("fixture deadline waiting for " + path.name)


def wait_reaped(who, timeout=5):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            pid, status = os.waitpid(who[0], os.WNOHANG)
            if pid:
                return os.waitstatus_to_exitcode(status)
        except ChildProcessError:
            if not exists(who):
                return "reaped-by-probe"
        time.sleep(0.01)
    raise RuntimeError("owned child did not exit/reap within bound")


def wait_zombie(who, timeout=10):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if not exists(who):
            return
        text = Path('/proc/%d/stat' % who[0]).read_text()
        if text[text.rfind(')') + 2:].split()[0] == 'Z':
            return
        time.sleep(0.01)
    raise RuntimeError('probe did not exit before controller teardown')


def file_hashes(root):
    return [{"path": str(p.relative_to(root)), "sha256": nodeproc.sha256_file(str(p)),
             "bytes": p.stat().st_size} for p in sorted(root.rglob('*')) if p.is_file()]


def profile_for(root, binary_hash, env):
    return {"profile_version": 1, "binary_sha256": binary_hash,
        "argv": ["--network", "regtest", "--datadir", str(root / 'declared-datadir'),
            "--rpc", "--rpc-listen", "127.0.0.1:19443", "--rpc-cookie-file", str(root / 'declared.cookie'),
            "--regtest-pow-bits", "200fffff", "--api-log", str(root / 'declared.jsonl'),
            "--log-level", "info", "--max-run-secs", "120", "--no-listen", "--no-seeds",
            "--no-discover", "--max-outbound", "1"],
        "env": env, "config_files": [],
        "substitutions": [{"class": "datadir", "argv_index": 3},
            {"class": "rpc-listen", "argv_index": 6}, {"class": "cookie-file", "argv_index": 8},
            {"class": "api-log", "span": [11, 15]}, {"class": "max-run-secs", "argv_index": 16},
            {"class": "p2p-isolation", "span": [17, 22]}]}


def worker(args):
    root = Path(args.out)
    binding = read_json(args.binding)
    helper, node = binding['helper'], binding['node']
    profile = read_json(root / 'profile.json')
    # This checks the real executable, real parent identity and installed
    # PDEATHSIG before any node launch or fault injection.
    nodeproc.validate_parent_handoff(helper['path'], helper['sha256'], args.parent_pid, args.parent_ticks)
    if args.case == 'cli-profile':
        os.execve(sys.executable, [sys.executable, str(Path(__file__).with_name('probe.py')),
            'node-probe', '--binary', node['path'], '--source-name', 'sealed-L2-eb257af7',
            '--launch-profile', str(root / 'profile.json'), '--actor-exec', helper['path'],
            '--actor-exec-sha256', helper['sha256'], '--controller-pid', str(args.parent_pid),
            '--controller-start-ticks', str(args.parent_ticks), '--label', 'node',
            '--out', str(root / 'cli'), '--ready-timeout', '10', '--keep-scratch'], dict(os.environ))
    owned = nodeproc.ProfileOwnedNode(node['path'], str(root), 'node', profile,
        helper=helper['path'], helper_sha256=helper['sha256'], controller_pid=args.parent_pid,
        controller_ticks=args.parent_ticks, ready_timeout_s=10, source_name='sealed-L2-eb257af7')
    assert owned.port not in PROTECTED_PORTS
    if args.case == 'preinstall-parent-death':
        real_spawn = nodeproc.subprocess.Popen
        def spawn(*a, **kw):
            def before_exec():
                # Real fork child pauses before helper exec/PDEATHSIG install.
                write(root / 'gap-child.json', {'pid': os.getpid(), 'start_ticks': nodeproc.start_ticks(os.getpid()),
                    'parent_pid': os.getppid(), 'stage': 'before-actor-exec'})
                os.kill(os.getpid(), signal.SIGSTOP)
            kw['preexec_fn'] = before_exec
            return real_spawn(*a, **kw)
        nodeproc.subprocess.Popen = spawn
    if args.case == 'partial-observed-failure':
        real_write = nodeproc.private_json
        def fail_observation(path, value):
            if path == owned.observed_path:
                real_write(str(root / 'fault-child.json'), value)
                raise OSError('fixture injected observed-record failure')
            return real_write(path, value)
        nodeproc.private_json = fail_observation
    try:
        owned.start()
        if args.case == 'early-startup':
            write(root / 'pre-readiness.json', {'node_pid': owned.proc.pid, 'readiness_called': False})
            time.sleep(20)
            raise RuntimeError('fixture parent did not kill early-startup probe')
        client = owned.wait_ready()
        chain = client.call('getblockchaininfo', [])
        write(root / 'observed-chain.json', chain.as_dict())
        assert chain.error is None and chain.result['chain'] == 'regtest' and chain.result['blocks'] == 0
        assert chain.result['consensus']['regtest_pow_bits_overlay'] == '200fffff'
        env = dict(item.split(b'=', 1) for item in Path('/proc/%d/environ' % owned.proc.pid).read_bytes().split(b'\0') if item)
        assert env == {k.encode(): v.encode() for k, v in profile['env'].items()}
        assert 'RBITCOIN_RPC_PACKAGE_DIALECT' not in profile['env']
        data = {'pid': owned.proc.pid, 'start_ticks': owned.identity_ticks,
            'binary_sha256': nodeproc.sha256_file('/proc/%d/exe' % owned.proc.pid),
            'height': chain.result['blocks'], 'network': chain.result['chain'],
            'consensus': chain.result['consensus'], 'comparison_sha256': owned.comparison_digest,
            'expected_comparison_sha256': digest(normalize(profile)), 'env_exact': True,
            'probe_pid': os.getpid(), 'probe_start_ticks': nodeproc.start_ticks(os.getpid())}
        assert data['binary_sha256'] == node['sha256']
        assert data['comparison_sha256'] == data['expected_comparison_sha256']
        write(root / 'ready.json', data)
        if args.case in ('controller-kill', 'probe-kill'):
            time.sleep(20)
            raise RuntimeError('fixture parent did not inject death')
        try:
            nodeproc.validate_parent_handoff(helper['path'], helper['sha256'], args.parent_pid, args.parent_ticks + 1)
        except ValueError:
            pass
        else:
            raise AssertionError('stale controller start ticks accepted')
        original_ticks = owned.identity_ticks
        owned.identity_ticks += 1
        try:
            owned.stop()
        except nodeproc.NodeError:
            pass
        else:
            raise AssertionError('stale node start ticks signaled')
        finally:
            owned.identity_ticks = original_ticks
        assert client.call('getblockcount', []).result == 0
        write(root / 'stale-identity.json', {'controller_mismatch_refused': True, 'node_mismatch_refused': True,
            'node_alive_after_refusal': True})
        result = run_probe(client, get_profile('bark-c419159'))
        info = owned.describe()
        try:
            info['effective_consensus'] = effective_consensus(chain.result)
            report = build_report(result, target_label='fixture', target_kind='owned-fresh-regtest-node',
                run_token='public-fixture-report-id', contract_hash_value=contract_hash(), node_info=info)
            validate_report(report)
            write(root / 'report.json', report)
            validation = {'status': 'complete'}
        except ValueError as exc:
            validation = {'status': 'binding-unavailable', 'error': str(exc)}
        # Preserve honest raw read-only checks even if this pinned binary
        # cannot supply all required launch-profile consensus metadata.
        write(root / 'checks.json', result['checks'])
        assert client.call('getblockcount', []).result == 0
        assert client.call('getrawmempool', []).result == []
        write(root / 'read-only-result.json', {'report_validation': validation,
            'height_after': 0, 'mempool_after': [], 'funded_behavior_proven': False})
    except OSError as exc:
        if args.case != 'partial-observed-failure' or str(exc) != 'fixture injected observed-record failure':
            raise
        assert owned.proc.returncode is not None and not Path('/proc/%d' % owned.proc.pid).exists()
        write(root / 'partial-result.json', {'status': 'expected-failure-cleaned', 'node_returncode': owned.proc.returncode,
            'reaped': True, 'observed_record_exists': Path(owned.observed_path).exists()})
    finally:
        outcome = owned.stop()
        write(root / 'worker-exit.json', outcome)


def coordinate(args):
    root = Path(args.out)
    root.mkdir(mode=0o700, parents=False, exist_ok=False)
    binding = read_json(args.binding)
    helper, node = binding['helper'], binding['node']
    assert nodeproc.sha256_file(helper['path']) == helper['sha256']
    assert nodeproc.sha256_file(node['path']) == node['sha256']
    libc = ctypes.CDLL(None, use_errno=True)
    assert libc.prctl(36, 1, 0, 0, 0) == 0  # subreaper, so dead ancestors' children are reaped here
    env = {'PYTHONDONTWRITEBYTECODE': '1', 'SPC_LAB_RUN_TOKEN': os.urandom(24).hex()}
    write(root / 'profile.json', profile_for(root, node['sha256'], env))
    listener = socket.socket()
    listener.bind(('127.0.0.1', 0))
    assert listener.getsockname()[1] not in PROTECTED_PORTS
    listener.listen(1)
    listener.settimeout(15)
    parked_url = 'http://127.0.0.1:%d/' % listener.getsockname()[1]
    controller = os.fork()
    if controller == 0:
        os.setsid()
        parent_pid, ticks = os.getpid(), nodeproc.start_ticks(os.getpid())
        probe_pid = os.fork()
        if probe_pid == 0:
            os.setsid()
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline:
                if os.readlink('/proc/%d/exe' % parent_pid) == helper['path']:
                    break
                time.sleep(0.005)
            else:
                os._exit(90)
            log = os.open(root / 'probe.log', os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            os.dup2(log, 1); os.dup2(log, 2); os.close(log)
            argv = [helper['path'], 'actor-exec', '--parent-pid', str(parent_pid), '--parent-start-ticks', str(ticks),
                '--', sys.executable, str(Path(__file__).resolve()), '--worker', '--case', args.case,
                '--out', str(root), '--binding', args.binding, '--parent-pid', str(parent_pid), '--parent-ticks', str(ticks)]
            os.execve(helper['path'], argv, env)
        write(root / 'controller-spawn.json', {'controller': [parent_pid, ticks], 'probe': list(identity(probe_pid))})
        log = os.open(root / 'controller.log', os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        os.dup2(log, 1); os.dup2(log, 2); os.close(log)
        os.execve(helper['path'], [helper['path'], 'http', '--url', parked_url], env)
    known = [identity(controller)]
    connection = None
    receipt = {'case': args.case, 'status': 'failed', 'helper_sha256': helper['sha256'], 'node_sha256': node['sha256'],
        'fixture_source_sha256': nodeproc.sha256_file(__file__), 'binding_receipt_sha256': nodeproc.sha256_file(args.binding),
        'source_bindings': {p.name: nodeproc.sha256_file(str(p)) for p in sorted(Path(__file__).parent.joinpath('arkrpc').glob('*.py'))}}
    try:
        connection, _ = listener.accept()
        connection.settimeout(2)
        connection.recv(4096)
        spawn = wait_file(root / 'controller-spawn.json')
        assert tuple(spawn['controller']) == known[0]
        known.append(tuple(spawn['probe']))
        assert nodeproc.sha256_file('/proc/%d/exe' % controller) == helper['sha256']
        if args.case == 'preinstall-parent-death':
            gap = wait_file(root / 'gap-child.json')
            known.append((gap['pid'], gap['start_ticks']))
            assert gap['parent_pid'] == known[1][0]
            assert not (root / 'node-launch-observed.json').exists()
            send(known[1], signal.SIGKILL)
            # Wait for actual reparenting before resuming the pre-install child.
            deadline = time.monotonic() + 3
            while Path('/proc/%d/stat' % known[2][0]).read_text().split(')')[-1].split()[1] != str(os.getpid()):
                assert time.monotonic() < deadline
                time.sleep(0.005)
            send(known[2], signal.SIGCONT)
        elif args.case == 'partial-observed-failure':
            fault = wait_file(root / 'fault-child.json')
            known.append((fault['pid'], fault['start_ticks']))
            receipt['partial_result'] = wait_file(root / 'partial-result.json')
            assert not (root / 'node-launch-observed.json').exists()
        elif args.case == 'cli-profile':
            observed = wait_file(root / 'cli/node-launch-observed.json')
            known.append((observed['pid'], observed['start_ticks']))
            receipt['cli_node_exit'] = wait_file(root / 'cli/node-launch-exit.json')
        else:
            observed = wait_file(root / 'node-launch-observed.json')
            known.append((observed['pid'], observed['start_ticks']))
            if args.case == 'early-startup':
                wait_file(root / 'pre-readiness.json')
                send(known[1], signal.SIGKILL)
            else:
                receipt['ready'] = wait_file(root / 'ready.json')
                if args.case == 'controller-kill':
                    send(known[0], signal.SIGKILL)
                elif args.case == 'probe-kill':
                    send(known[1], signal.SIGKILL)
                else:
                    receipt['read_only'] = wait_file(root / 'read-only-result.json')
                    receipt['worker_exit'] = wait_file(root / 'worker-exit.json')
        # End the parked controller so exited probes are adopted and reaped.
        if args.case in ('read-only-stop', 'partial-observed-failure', 'cli-profile'):
            wait_zombie(known[1])
        if args.case != 'controller-kill':
            send(known[0], signal.SIGKILL)
        statuses = [wait_reaped(who) for who in known]
        assert all(not exists(who) for who in known)
        if args.case == 'controller-kill':
            assert statuses == [-9, -9, -9], statuses
        if args.case in ('probe-kill', 'early-startup'):
            assert statuses[1:] == [-9, -9], statuses
        if args.case == 'preinstall-parent-death':
            assert statuses[1:] == [-9, 1], statuses
            assert not list((root / 'node/datadir').iterdir())
        if args.case in ('read-only-stop', 'partial-observed-failure'):
            assert statuses[1] == 0, statuses
            assert statuses[2] == 'reaped-by-probe', statuses
        if args.case == 'cli-profile':
            receipt['cli_report_exists'] = (root / 'cli/report.json').exists()
            receipt['cli_probe_exit'] = statuses[1]
            assert statuses[1] in (0, 1) and statuses[2] == 'reaped-by-probe', statuses
        receipt.update(status='passed-lifetime', identities=[list(who) for who in known], exit_codes=statuses)
    finally:
        for who in reversed(known):
            send(who, signal.SIGKILL)
        for who in known:
            if exists(who):
                wait_reaped(who)
        if connection:
            connection.close()
        listener.close()
        receipt['artifacts'] = file_hashes(root)
        write(root / 'receipt.json', receipt)
    print(json.dumps({'case': args.case, 'status': receipt['status'], 'exit_codes': receipt.get('exit_codes'),
        'receipt_sha256': nodeproc.sha256_file(str(root / 'receipt.json'))}))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--case', choices=CASES, required=True)
    parser.add_argument('--out', required=True)
    parser.add_argument('--binding', required=True)
    parser.add_argument('--worker', action='store_true')
    parser.add_argument('--parent-pid', type=int)
    parser.add_argument('--parent-ticks', type=int)
    args = parser.parse_args()
    worker(args) if args.worker else coordinate(args)
