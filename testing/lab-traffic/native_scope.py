"""Private opaque-token provenance joins, outside the native public codec.

The controller records the exact Supervisor marker. Only its digest leaves
these helpers; no token shape, run-ID prefix, or decoded hex is an authority.
"""
import hashlib
from pathlib import Path
import re
import tomllib

from native_codec import array, fields, identifier, validate_instance, validate_process
from oracle import Invalid, require
from source_jobs import MissingComponent

MAX_MARKER=1024
MAX_ENV=1024*1024
PRIVATE_FIELDS='run_id run_token_hash identity_id pool_id front_id collector_socket pool_pubkey collector_pubkey'.split()


def expected_digest(value):
    require(isinstance(value,str) and re.fullmatch('[0-9a-f]{64}',value) is not None and value!='0'*64,
        'native expected run token digest shape')
    return bytes.fromhex(value)


def marker_digest(marker):
    require(isinstance(marker,str),'native recorded marker type')
    try:raw=marker.encode('utf-8')
    except UnicodeError:raise Invalid('native recorded marker UTF8') from None
    require(0<len(raw)<=MAX_MARKER,'native recorded marker bound')
    return hashlib.sha256(raw).digest()


def environment_digest(raw,expected):
    require(isinstance(raw,bytes) and len(raw)<=MAX_ENV,'native proc environment bound')
    tokens=[entry.split(b'=',1)[1] for entry in raw.split(b'\0') if entry.startswith(b'SPC_LAB_RUN_TOKEN=')]
    require(len(tokens)==1 and 0<len(tokens[0])<=MAX_MARKER,'native process marker census/bound')
    digest=hashlib.sha256(tokens[0]).digest()
    require(digest==expected and any(expected),'native process marker digest mismatch')
    return digest


def process_digest(proc,record):
    expected=marker_digest(record.get('run_token'))
    with (Path(proc)/'environ').open('rb') as stream:raw=stream.read(MAX_ENV+1)
    return environment_digest(raw,expected)


def recorded_digest(manifest,run_id):
    require(manifest.get('run_id')==run_id,'native recorded marker run binding')
    records=manifest.get('processes')
    require(isinstance(records,list) and records,'native recorded marker census missing')
    digests={marker_digest(record.get('run_token')) for record in records}
    require(len(digests)==1,'native recorded marker census inconsistent')
    return next(iter(digests))


def private_scope(raw):
    require(isinstance(raw,bytes) and len(raw)<=4*1024*1024,'native rendered config bound')
    try:value=tomllib.loads(raw.decode('utf-8'))
    except (ValueError,UnicodeError):raise Invalid('native rendered config encoding/shape') from None
    ark=value.get('ark');scope=ark.get('native_audit') if isinstance(ark,dict) else None
    if scope is None:raise MissingComponent('rendered_existing_wallet_native_audit_config')
    fields(scope,PRIVATE_FIELDS)
    require(all(isinstance(v,str) for v in scope.values()),'native rendered scope scalar types')
    expected_digest(scope['run_token_hash'])
    for key in ('run_id','identity_id','pool_id','front_id'):identifier(scope[key])
    return scope


def wallet_scope(raw,instance,observed,manifest,run_id):
    """Join actual config/instance/process to the complete recorded census."""
    scope=private_scope(raw);validate_instance(instance);validate_process(observed)
    expected=expected_digest(scope['run_token_hash'])
    require(scope['run_id']==instance['run_id']==run_id,'native private scope run mismatch')
    for key in ('identity_id','pool_id','front_id','collector_pubkey','pool_pubkey'):
        require(scope[key]==instance[key],'native private scope/instance mismatch: '+key)
    require(instance['process']==observed,'native private scope instance/process mismatch')
    require(hashlib.sha256(raw).digest()==array(observed['config_sha256'],32),'native private scope config changed')
    require(expected==array(observed['run_token_sha256'],32)==recorded_digest(manifest,run_id),
        'native expected/observed/recorded marker digest mismatch')
    return {'run_token_hash':scope['run_token_hash'],'config_sha256':hashlib.sha256(raw).hexdigest(),
        'recorded_processes':len(manifest['processes']),'scope':'private_config_instance_process_recorded_marker_join'}
