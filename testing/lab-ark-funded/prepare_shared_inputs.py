"""Prepare fresh public bindings and private attestation keys; never start actors."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

from common import artifact, check_bundle, encoded, fresh_root, load, require, sha
from portable import ref
from portable_verifier import nix_source
from scenario import scenario
from shared_sources import load_qa
from metadata_tools import git_binding, runtime_path, preflight


def write(path, value):
    with Path(path).open('xb') as stream:
        stream.write(encoded(value) + b'\n')
    Path(path).chmod(0o600)


def prepare(args):
    bundle_path = Path(args.bundle).resolve(strict=True)
    bundle = load(bundle_path); check_bundle(bundle, full=True)
    outputs = load(args.outputs)
    require(outputs['schema'] == 'spc.lab.ap5.nix-outputs/v1' and outputs['base_revision'] == bundle['base_revision'],
            'runtime outputs revision differs from selected bundle')
    for name, item in bundle['binaries'].items():
        require(ref(outputs['binaries'][name]) == item, 'runtime output differs from bundle: ' + name)
    require(outputs['runtime_script'] == bundle['runtime_script']
            and ref(outputs['pool_wallet_executor']) == bundle['pool_wallet_executor']
            and ref(outputs['vtxo_verifier']) == {k: bundle['vtxo_verifier'][k] for k in ('path', 'sha256')},
            'runtime/executor/verifier outputs differ from selected bundle')
    path_env = outputs['qa_path_env']
    require(path_env and all(Path(p).is_absolute() and Path(p).is_relative_to('/nix/store') and Path(p).is_dir()
                            for p in path_env.split(':')), 'immutable runtime PATH required')
    run_out = Path(args.run_out).absolute()
    require(run_out == run_out.resolve() and not run_out.exists(), 'fresh canonical run output required')
    root = fresh_root(args.out)
    metadata = git_binding(args.consumer_tools)
    path_env = runtime_path(path_env, metadata['git']['path'])
    write(root / 'runtime-metadata.json', metadata)
    preflight(bundle['probe_invocation'], path_env, metadata, root / 'metadata-preflight')
    _, _, source, _ = nix_source(bundle['binaries']['lab']['path'])
    _, _, node_source, _ = nix_source(bundle['binaries']['node']['path'])
    template_path = source / 'testing/lab-traffic/inputs.template.json'
    template = load(template_path)
    _, qa_descriptor = load_qa(args.qa_source, args.qa_source_sha256, root / 'qa-provisioning-source')
    from source_feed_codec import CONTRACT, CollectorKey, _N, _GX, _GY, _multiply
    from decision_observations import CONTRACT as DECISION_CONTRACT
    key_root = fresh_root(root / 'attestation-keys')

    def key(name):
        # Existing QA source-attestation codec only; no wallet or Ark keys.
        value = 0
        while not 0 < value < _N:
            value = int.from_bytes(os.urandom(32), 'big')
        path = key_root / (name + '.hex')
        with os.fdopen(os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600), 'w') as stream:
            stream.write(value.to_bytes(32, 'big').hex() + '\n')
        public = _multiply(value, (_GX, _GY))[0].to_bytes(32, 'big').hex()
        require(CollectorKey(path, public).public_key == public, 'fresh attestation key binding')
        return path, public

    collector_path, collector_public = key('collector')
    spec = scenario(args.run_id, args.seed)
    public_keys = {}
    for front in spec['fronts']:
        require('ark' not in front['roles'], 'front source key must not enter Ark role')
        path, public = key(front['id'])
        front['env'] = {'SPC_SOURCE_EVIDENCE_KEY_FILE': str(path)}
        public_keys[front['id']] = public
    agents = {}
    for wallet in spec['wallet_agents']:
        identity, front = wallet['identity_id'], wallet['front_id']
        socket_name = 'collector-' + identity + '.sock'
        socket = run_out / 'qa/runtime/private' / socket_name
        require(len(os.fsencode(socket)) < 108, 'collector Unix socket path exceeds platform bound')
        wallet['native_audit'] = {'collector_socket': socket_name, 'pool_pubkey': public_keys[front],
                                  'collector_pubkey': collector_public}
        agents[identity] = {'wallet_entity': identity, 'miner_id': 'miner-' + identity, 'front': front}
    require(len(set(public_keys.values()) | {collector_public}) == 3, 'distinct collector/front authorities required')
    joined = bundle['joined_source_manifest']
    binaries = {name: {**bundle['binaries'][name], 'source_manifest': joined['path'],
                       'source_manifest_sha256': joined['sha256']}
                for name in ('lab', 'txd', 'pool', 'agent', 'miner', 'scanner')}

    def bind(name, program, manifest):
        item, seal = ref(program), ref(manifest)
        binaries[name] = {**item, 'source_manifest': seal['path'], 'source_manifest_sha256': seal['sha256']}

    bind('git', metadata['git']['path'], root / 'runtime-metadata.json')

    node_manifest = Path(outputs['binaries']['node']).parent.parent / 'share/rbitcoin-lab/source.json'
    bind('node', bundle['binaries']['node']['path'], node_manifest)
    captaind = Path(bundle['binaries']['captaind']['path'])
    bind('captaind', captaind, captaind.parent.parent / 'share/ark/source-revision')
    probe = bundle['probe_invocation']
    bind('traffic-probe', probe['program']['path'], artifact(probe['program_binding']))
    pg = Path(bundle['postgres_bin_dir'])
    tools = {name: Path(bundle['binaries'][name]['path']) for name in ('grpcurl', 'redis-server', 'redis-cli')}
    tools.update({name: pg / name for name in ('initdb', 'postgres', 'pg_ctl', 'psql', 'createdb')})
    seals = fresh_root(root / 'tool-seals')
    for name, program in tools.items():
        program = program.resolve(strict=True)
        package = Path('/nix/store') / program.relative_to('/nix/store').parts[0]
        drv = subprocess.check_output(['nix-store', '-q', '--deriver', str(package)], text=True).strip()
        require(drv.endswith('.drv'), 'actual tool deriver required')
        manifest = seals / (name + '.json')
        write(manifest, {'schema': 'spc.lab.campaign.tool-seal/v1', 'name': name, **ref(program), 'deriver': drv})
        bind(name, program, manifest)
    migrations = {}
    migration_sources = {}
    for path in sorted((source / 'crates/spc-store/migrations').glob('*.sql')):
        version = int(path.name.split('_', 1)[0])
        if 1 <= version <= 10:
            require(str(version) not in migrations, 'duplicate prior migration version')
            migrations[str(version)] = hashlib.sha384(path.read_bytes()).hexdigest()
            migration_sources[str(version)] = ref(path)
    require(set(migrations) == {str(i) for i in range(1, 11)}, 'all ten pre-checkpoint migration checksums required')
    policy = template['policy']
    # The actual node reports the unmodified attainable-target floor even with
    # an initial mining-target overlay; this corrects the empty input template.
    policy['retarget_floor_bits'] = policy['pow_limit_bits']
    mapping = {name: name for name in policy if name not in {'genesis', 'subsidy_initial_sats'}}
    mapping.update(target_spacing='pow_target_spacing', target_timespan='pow_target_timespan')
    subsidy_source = node_source / 'crates/rbitcoin-consensus/src/block/mod.rs'
    binding = {'schema': 'spc-lab-funded-acceptance-inputs/1', 'base_revision': bundle['base_revision'],
        'input_recipe': 'spc.lab.ap5.prepared-inputs/2-explicit-metadata-tools',
        'source_snapshot_sha256': joined['sha256'], 'payout_policy': 'design-fixed-ladder', 'path_env': path_env,
        'binaries': binaries, 'definitive_rejection_binding': template['definitive_rejection_binding'],
        'runtime_metadata': ref(root / 'runtime-metadata.json'),
        'probe_script': probe['script'], 'policy': policy, 'consensus_field_map': mapping,
        'consensus_source_facts': {'subsidy_initial_sats': {'value': policy['subsidy_initial_sats'],
            'source_path': str(subsidy_source), 'source_sha256': sha(subsidy_source)}},
        'source_feed': {'contract': CONTRACT, 'sql_contract': DECISION_CONTRACT, 'collector_key_file': str(collector_path),
            'collector_pubkey': collector_public, 'pool_pubkeys': public_keys, 'agents': agents, 'migration_checksums': migrations}}
    write(root / 'scenario.json', spec); write(root / 'qa-inputs.json', binding)
    result = subprocess.run([bundle['binaries']['lab']['path'], 'validate', '--scenario', str(root / 'scenario.json')],
                            capture_output=True, timeout=30, env={'PATH': path_env})
    (root / 'scenario-validation.stdout').write_bytes(result.stdout)
    (root / 'scenario-validation.stderr').write_bytes(result.stderr)
    require(result.returncode == 0, 'actual runtime rejected prepared scenario: ' + result.stderr.decode())
    write(root / 'execution-plan.json', {'status': 'PREPARED_NOT_LAUNCHED; REVIEWED_R22_AND_IMMUTABLE_CONSUMER_REQUIRED',
        'bundle': ref(bundle_path), 'outputs': ref(args.outputs), 'inputs': ref(root / 'qa-inputs.json'),
        'scenario': ref(root / 'scenario.json'), 'run_output': str(run_out), 'timeout_secs': 7200,
        'provisioning_qa_descriptor': ref(args.qa_source), 'provisioning_qa_api': qa_descriptor['api'],
        'runtime_source_root': str(source), 'template': ref(template_path), 'node_source_root': str(node_source),
        'migration_sources': migration_sources, 'scenario_validation': {'exit': result.returncode,
            'stdout': ref(root / 'scenario-validation.stdout'), 'stderr': ref(root / 'scenario-validation.stderr')},
        'input_recipe': 'spc.lab.ap5.prepared-inputs/2-explicit-metadata-tools',
        'metadata_preflight': ref(root / 'metadata-preflight/receipt.json'),
        'runtime_metadata': ref(root / 'runtime-metadata.json'),
        'required_order': 'one Runtime bootstrap; common first block; maturity; 96 allocations; four transfers at depth 3 on both nodes; measured/manual/witness/fault/reorg; Ark lifecycle; audit_live; final_drain; final_audit_after_drain; close; one AP5 stop/reap',
        'original_operation': 'ap5-native-bootstrap', 'bootstrap_deadline_secs': 3600,
        'aggregate_hashes_per_second': 40, 'ordinary_transactions_per_second': 0.2,
        'source_policies': {name: bundle[name] for name in ('pool_control_source_policy', 'aggregate_source_policy', 'vtxo_verifier')},
        'global_cleanup_scope': 'four previously unreadable PIDs remain unresolved; scoped reaper success cannot imply global cleanup'})
    return root


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('bundle', 'outputs', 'qa-source', 'out', 'run-out', 'consumer-tools'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--qa-source-sha256', required=True)
    parser.add_argument('--run-id', required=True)
    parser.add_argument('--seed', type=int, required=True)
    args = parser.parse_args()
    print(prepare(args))


if __name__ == '__main__':main()
