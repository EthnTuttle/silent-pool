"""Publish verified live Runtime URLs and manifest ports without credentials."""
import json
from pathlib import Path
import urllib.parse
import urllib.request

from common import encoded, require, sha
from cleanup import process


def publish_run_info(qa, ap5):
    require(qa.process is ap5.child and qa.process.poll() is None, 'run info requires the same live Runtime')
    before = process(qa.process.pid)
    require(before['ticks'] == ap5.controller_ticks and before['state'] not in {'Z', 'X'}, 'run-info Runtime identity changed')
    require(sha(Path('/proc') / str(qa.process.pid) / 'exe') == ap5.bundle['binaries']['lab']['sha256'],
            'run-info Runtime executable differs from admitted CLI')
    url = urllib.parse.urlsplit(qa.url)
    require(url.scheme == 'http' and url.hostname in {'127.0.0.1', '::1'} and url.port
            and not (url.username or url.password or url.query or url.fragment) and url.path in {'', '/'},
            'run-info origin must be the actual loopback Runtime listener')
    argv = (Path('/proc') / str(qa.process.pid) / 'cmdline').read_bytes().split(b'\0')
    require([argv[i + 1].decode() for i, value in enumerate(argv[:-1]) if value == b'--listen'] == [url.netloc],
            'run-info URL differs from actual Runtime listen argv')
    require([Path(argv[i + 1].decode()).resolve() for i, value in enumerate(argv[:-1]) if value == b'--run-dir'] == [ap5.run.resolve()],
            'run-info Runtime directory differs')
    snapshot = qa.http('snapshot')
    require(snapshot['run_id'] == ap5.id == qa.run_id, 'run-info API belongs to another run')
    origin = urllib.parse.urlunsplit((url.scheme, url.netloc, '', '', ''))
    pages = {}
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    for name, route in (('controls', '/'), ('lab', '/lab')):
        target = origin + route
        with opener.open(target, timeout=10) as response:
            require(response.status == 200 and response.url == target
                    and response.headers.get_content_type() == 'text/html', 'run-info UI route did not serve HTML')
            body = response.read(2 * 1024 * 1024 + 1)
            require(0 < len(body) <= 2 * 1024 * 1024, 'run-info UI response bound')
        pages[name] = target
    manifest = qa.manifest()
    require(manifest['run_id'] == qa.run_id, 'run-info manifest belongs to another run')
    listeners = {}
    for name, endpoint in manifest['resources']['listeners'].items():
        require(type(endpoint['host']) is str and type(endpoint['port']) is int and 0 < endpoint['port'] < 65536,
                'run-info manifest endpoint invalid')
        listeners[name] = {'host': endpoint['host'], 'port': endpoint['port']}
    after = process(qa.process.pid)
    require(after['ticks'] == before['ticks'] and after['state'] not in {'Z', 'X'} and qa.process.poll() is None,
            'Runtime exited during run-info observation')
    target = ap5.root.parent / 'run-info.json'
    info = {'schema': 'spc.lab.ap5.run-info/v1', 'run_id': qa.run_id,
            'scope': 'verified startup observation; final outcome is recorded separately after teardown',
            'snapshot_cursor': snapshot.get('snapshot_cursor'), 'snapshot_generated_at': snapshot.get('generated_at'),
            'status': 'runtime_running_acceptance_pending', 'pid': qa.process.pid,
            'start_time_ticks': before['ticks'], 'run_directory': str(ap5.run),
            'run_info_file': str(target), 'origin': origin, 'ui': pages,
            'snapshot_url': origin + '/api/lab/v1/runs/' + qa.run_id + '/snapshot',
            'operator_claim_file': str(ap5.run / 'private/access/operator.json'),
            'observed_manifest_state': manifest['state'], 'listeners': listeners,
            'listener_provenance': 'actual Runtime manifest; UI URLs additionally verified by HTTP'}
    with target.open('xb') as stream:
        target.chmod(0o600)
        stream.write(encoded(info) + b'\n')
    ap5.ev.save('run-info', info)
    qa.capture.add('run_info', info)
    print(json.dumps(info, sort_keys=True), flush=True)
    return info
