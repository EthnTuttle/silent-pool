"""Run the installed probe manifest writer on a labelled public metadata fixture."""
import json
import os
from pathlib import Path
import shutil
import sys


def main():
    probe_root, git, fixture, output = map(Path, sys.argv[1:])
    found = shutil.which('git', path=os.environ['PATH'])
    if found is None or Path(found).resolve() != git.resolve():
        raise RuntimeError('declared Git unavailable in exact metadata environment')
    calls = []
    def audit(event, args):
        if event == 'subprocess.Popen':
            if args[1] != ['git', 'rev-parse', 'HEAD'] or Path(args[2]) != probe_root:
                raise RuntimeError('undeclared probe metadata subprocess')
            calls.append(args[1])
        if event in {'socket.connect', 'socket.bind'}:
            raise RuntimeError('metadata preflight cannot start a listener or contact RPC')
    sys.addaudithook(audit)
    sys.path.insert(0, str(probe_root))
    from arkrpc.contracts import contract_hash
    from arkrpc.reporting import build_manifest, git_base_commit, write_manifest
    values = json.loads(fixture.read_text())
    base_commit = git_base_commit(str(probe_root))
    if base_commit is not None:
        raise RuntimeError('metadata fixture expects installed source without Git checkout')
    manifest = build_manifest(run_token=values['run_token'], contract_hash_value=contract_hash(),
        node_description=values['node_description'], node_exit=values['node_exit'],
        probe_root=str(probe_root), base_commit=base_commit,
        dialect_note='metadata-only public fixture; no node, dialect injection or live evidence')
    write_manifest(str(output), manifest)
    if calls != [['git', 'rev-parse', 'HEAD']]:
        raise RuntimeError('metadata subprocess census changed')
    print(json.dumps({'base_commit': base_commit, 'subprocesses': calls, 'node_started': False, 'rpc_called': False}))


if __name__ == '__main__':main()
