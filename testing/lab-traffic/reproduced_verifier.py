"""Relocatable qualification of the same already-approved public verifier.

The fixed binary/source policy is retained. Fresh Nix logs replace historical
machine-specific check files; this adds no verifier or crypto implementation.
"""
from pathlib import Path
import re
import subprocess

from oracle import Inconclusive, require, strict_json
from source_feed_codec import fields
from storage import sha_file

SCHEMA='spc.lab.ap5.verifier-reproduction/v1'
POLICY_SHA256='a9c36c674236a32d82140cfa918837e44bc4cabdce7478a7a6045959b9606bdc'


def admit(artifact):
    from ap5_verifier import BINARY_SHA256, LOCK_SHA256, CRYPTO_CASES, read_sealed
    fields(artifact,'schema path sha256 nix_binding test_log'.split())
    require(artifact['schema']==SCHEMA and artifact['sha256']==BINARY_SHA256,'reproduced verifier must be the approved binary')
    binary=Path(artifact['path']);require(binary.is_absolute() and str(binary).startswith('/nix/store/') and sha_file(binary)==BINARY_SHA256,'reproduced verifier binary bytes/path')
    for ref in (artifact['nix_binding'],artifact['test_log']):fields(ref,['path','sha256'])
    binding=strict_json(read_sealed(artifact['nix_binding']['path'],artifact['nix_binding']['sha256']))
    require(binding['schema']=='spc.lab.ap5.verifier-nix-binding/v1' and binding['binary']=={'path':str(binary),'sha256':BINARY_SHA256}
        and binding['cargo_lock_sha256']==LOCK_SHA256,'reproduced verifier binding')
    policy=strict_json(read_sealed(Path(__file__).parent/'fixtures/verifier-source-policy.json',POLICY_SHA256))
    root=Path(binding['source']);require(root.is_absolute() and str(root).startswith('/nix/store/'),'reproduced verifier source root')
    rows={r['path']:r for r in binding['source_rows']};expected={r['path']:r for r in policy['files']}
    require(len(rows)==len(binding['source_rows']) and set(rows)==set(expected),'reproduced verifier exact source census')
    require({str(p.relative_to(root)) for p in root.rglob('*') if p.is_file()}==set(expected),'reproduced verifier extra/missing source')
    for name,row in expected.items():
        require(rows[name]['sha256']==row['sha256'] and rows[name]['bytes']==row['bytes'],'reproduced source policy mismatch')
        raw=read_sealed(root/name,row['sha256']);require(len(raw)==row['bytes'] and bool((root/name).stat().st_mode&0o111)==row['executable'],'reproduced verifier source size/mode')
    package=binary.parent.parent
    deriver=subprocess.run(['nix-store','--query','--deriver',str(package)],capture_output=True,timeout=10,check=True).stdout.decode().strip()
    require(deriver==binding['deriver'],'reproduced verifier actual deriver')
    result=subprocess.run(['nix','--extra-experimental-features','nix-command','derivation','show',deriver],capture_output=True,timeout=10,check=True)
    derivations=strict_json(result.stdout);require(set(derivations)=={deriver},'reproduced verifier derivation census')
    drv=derivations[deriver];env=drv['env'];env=strict_json(env['__json']) if '__json' in env else env
    require(env['src']==str(root) and drv['outputs']['out']['path']==str(package) and env['doCheck'] in (True,'1'),'reproduced verifier source/check/output derivation')
    read_sealed(package/'share/ap5-verifier/Cargo.lock',LOCK_SHA256)
    retained=read_sealed(artifact['test_log']['path'],artifact['test_log']['sha256'])
    log=subprocess.run(['nix','--extra-experimental-features','nix-command','log',deriver],capture_output=True,timeout=30)
    if log.returncode!=0:raise Inconclusive('actual reproduced Nix test log unavailable; build/check receipt required')
    require(log.stdout==retained,'reproduced verifier retained log differs from actual derivation log')
    text=retained.decode()
    require(all(re.search(re.escape(case)+r' \.\.\. ok',text) for case in CRYPTO_CASES) and '6 passed; 0 failed; 0 ignored' in text,'reproduced verifier six actual crypto checks incomplete')
    return {'status':'ADMITTED_REPRODUCED_PUBLIC_VERIFIER','binary_sha256':BINARY_SHA256,'source_root':str(root),'source_rows':rows,
        'test_log_sha256':artifact['test_log']['sha256'],'unique_crypto_cases':6,'historical_machine_receipts_required':False,'first_actual_funded_board_required':True}
