"""Reproduce the public, signed synthetic native-retention conformance files.

Uses only the existing published test keys/fixture signer. Never use these keys
for a wallet or a live run. Expected hashes bind the original producer vector.
"""
import hashlib
import json
from pathlib import Path
import sys

ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT))
from test_native_audit import fixture
from native_codec import request_digest
from source_feed_codec import checkpoint_digest

EXPECTED={
 'manifest.json':'13cba2fba244a1914790c510395084fc025bde2570b581ac8c3ad400b64db74f',
 'record.json':'f12ff73e6f24126b12f7812ee2550f1e211f4793b4741c216efc9c70b8e564b3',
 'request.json':'43020688e0b6d1c0f3ea42e7cda791efa6739acdadd19769c0eac95d72bf557b',
 'response.json':'1d16911b57ca4be46ff533efecea3d3af3dae27cb186bf5482f9a4e39ab76a00',
 'handle.json':'cd93e2e3551881c6d44c40cdb3d4551b4afdcc49f23bb5cfcee0b260c4726fb0'}

def generate():
    request,response,bindings,_,_=fixture(9);binding=bindings[('miner','connection')]
    encode=lambda value:json.dumps(value,separators=(',',':')).encode()
    sha=lambda value:hashlib.sha256(value).hexdigest()
    manifest={'schema':'spc.native-evidence-manifest/1','kind':'capture_session','capability':'native_public_evidence_durable_v1',
       'capture_id':'15'*16,'miner_id':'miner','config':{'schema':'spc.native-evidence-config/1','run_id':'run','directory':'/unit/private/native-evidence/miner',
       'run_manifest':'/unit/run-manifest.json','max_records':16384,'max_bytes':268435456,'max_sessions':32},
       'process':binding['observed_miner'],'config_path':'/unit/private/miner.toml'}
    raw_manifest=encode(manifest);req=encode(request);resp=encode(response);digest=request_digest(request).hex()
    keys='run_id miner_id connection_id session_epoch candidate_epoch wallet_instance peer_generation connection_config_digest nonce'.split()
    record={'schema':'spc.native-evidence/1','kind':'approved_exchange','version':1,'activation':'not_asserted','capture_id':'15'*16,'receipt_sequence':1,
       'manifest_sha256':sha(raw_manifest),'request_digest':digest,'request_sha256':sha(req),'response_sha256':sha(resp),
       'checkpoint_digest':checkpoint_digest(response['receipt']['evidence']['checkpoint']).hex(),'binding':{k:request[k] for k in keys},
       'verified_at_unix_ms':1000,'request_json_hex':req.hex(),'response_json_hex':resp.hex()}
    raw_record=encode(record)
    handle={'schema':'spc.native-evidence-handle/1','capability':'native_public_evidence_durable_v1','capture_id':'15'*16,'receipt_sequence':1,
        'manifest_relative_path':'process-1234-55/manifest.json','manifest_sha256':sha(raw_manifest),'record_relative_path':'process-1234-55/'+digest+'.json',
        'record_sha256':sha(raw_record),'request_digest':digest}
    values={'manifest.json':raw_manifest,'record.json':raw_record,'request.json':req,'response.json':resp,'handle.json':encode(handle)}
    assert {k:sha(v) for k,v in values.items()}==EXPECTED
    return values

if __name__=='__main__':
    for name,raw in generate().items():
        path=Path(__file__).parent/name
        if path.exists():assert path.read_bytes()==raw
        else:
            with path.open('xb') as stream:stream.write(raw)
            path.chmod(0o644)
