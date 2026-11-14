"""Reviewed Runtime profiles, independent of the QA artifact's own source.

Only exact compiled app tuples enter this path. Source/test qualification is
not live acceptance, and no caller-supplied root or status marker is trusted.
"""
import hashlib
import json
from pathlib import Path
import re
import subprocess

from case_contract import canonical_json
from oracle import integer, require, strict_json
from source_jobs import MissingComponent

PROFILE_SHA256='d619692e4d5dadcf146e22e715ced350122621d47656d0decb2434a3d3570371'
MANIFEST_SHA256='2085d16dc957451547d96d995a52d55302a6581e23fbd3af710f927d9e586524'
OPAQUE_MANIFEST_SHA256='85a0d7d0d78c8820e28b404f8d7b74ec998fbb1b574d9e3c69ca420a65df3572'
PROFILES={MANIFEST_SHA256:('runtime-e4.json',PROFILE_SHA256),
    OPAQUE_MANIFEST_SHA256:('runtime-98f.json','0f7fab9b06c55547f7cb32ee60c7888d87fc15a7142bb1e261f3eaff10932c1e')}
APP_NAMES={'lab':'spc-lab','pool':'spc-pool','miner':'spc-cpu-miner','agent':'spc-agent','scanner':'spc-scan','txd':'spc-lab-txd'}


def digest(path):
    h=hashlib.sha256()
    with Path(path).open('rb') as stream:
        for chunk in iter(lambda:stream.read(1024*1024),b''):h.update(chunk)
    return h.hexdigest()


def sealed(path,expected,limit=32*1024*1024):
    path=Path(path);require(path.is_file() and not path.is_symlink() and path.stat().st_size<=limit,'runtime profile artifact type/size')
    with path.open('rb') as stream:raw=stream.read(limit+1)
    require(len(raw)<=limit and hashlib.sha256(raw).hexdigest()==expected,'runtime profile artifact seal: '+str(path))
    return strict_json(raw)


def profile(manifest_sha256=MANIFEST_SHA256):
    if manifest_sha256 not in PROFILES:raise MissingComponent('reviewed_qualified_runtime_source_profile_unavailable')
    name,digest=PROFILES[manifest_sha256]
    return sealed(Path(__file__).with_name('profiles')/name,digest)


def selected(binary,role='lab'):
    p=profile(binary.get('source_manifest_sha256'));require(p['schema']=='spc.qa.runtime-profile/1','runtime profile schema')
    if role not in p['binaries'] or binary.get('sha256')!=p['binaries'][role]:
        raise MissingComponent('reviewed_qualified_runtime_executable_profile_unavailable')
    if 'runtime_profile' in binary:require(binary['runtime_profile']==p['id'],'caller selected another runtime profile')
    for key in ('source_root','nix_src_root'):
        if key in binary:require(binary[key]==p['source_root'],'caller runtime root differs from reviewed compiler root')
    return p


def derivation(value,expected):
    """Actual flat Nix output and Nix 2.34's version-4 wire format."""
    require(isinstance(value,dict),'Nix derivation object')
    if set(value)=={'version','derivations'}:
        require(type(value['version']) is int and value['version']==4,'unsupported Nix derivation format version')
        key=Path(expected).name;require(set(value['derivations'])=={key},'versioned Nix derivation census')
        result=value['derivations'][key];require(type(result.get('version')) is int and result['version']==4,'versioned Nix derivation record version')
        outputs={}
        for name,item in result['outputs'].items():
            raw=item.get('path');require(isinstance(raw,str) and re.fullmatch(r'[a-z0-9]{32}-[^/]+',raw),'versioned Nix output path')
            outputs[name]='/nix/store/'+raw
    else:
        require(set(value)=={expected},'Nix derivation census');result=value[expected]
        outputs={name:item['path'] for name,item in result['outputs'].items()}
    env=result['env'];env=strict_json(env['__json']) if '__json' in env else env
    return env,outputs,result['system']


def compiler_root(binary,p,role,*,probe=None):
    require(Path(binary['path'])==Path(p['package'])/'bin'/APP_NAMES[role],'runtime executable outside reviewed package')
    require(digest(binary['path'])==p['binaries'][role],'runtime executable changed')
    with Path(binary['path']).open('rb') as stream:require(stream.read(4)==b'\x7fELF','runtime executable is not native ELF')
    manifest=sealed(binary['source_manifest'],p['source_manifest_sha256'])
    require(set(manifest)=={'files'},'qualified runtime manifest must remain the actual files-only format')
    if probe is None:
        actual=subprocess.run(['nix-store','--query','--deriver',p['package']],capture_output=True,check=True,timeout=10).stdout.decode().strip()
        require(actual==p['deriver'],'runtime actual Nix deriver differs')
        raw=subprocess.run(['nix','--extra-experimental-features','nix-command','derivation','show',actual],capture_output=True,check=True,timeout=15).stdout
        value=strict_json(raw)
    else:value=probe(p['deriver'])
    env,outputs,system=derivation(value,p['deriver'])
    install_check=env.get('doInstallCheck')
    require(env.get('src')==p['source_root'] and outputs.get('out')==p['package'] and system==p['system']
        and (install_check is True or type(install_check) is str and install_check=='1'),'runtime compiler source/output/qualification binding')
    return Path(p['source_root']),manifest


def source_tree(root,manifest,p):
    rows={r['path']:r for r in manifest['files']}
    require(len(rows)==len(manifest['files'])==p['source_files'],'runtime complete source census')
    for name,row in rows.items():
        require(set(row)=={'path','sha256','bytes'} and not Path(name).is_absolute() and '..' not in Path(name).parts,'runtime source row shape/path')
        path=root/name;integer(row['bytes'])
        require(path.is_file() and not path.is_symlink() and path.stat().st_size==row['bytes'] and digest(path)==row['sha256'],'runtime source row changed: '+name)
    require({str(path.relative_to(root)) for path in root.rglob('*') if path.is_file()}==set(rows),'runtime unlisted/missing compiler-source files')
    require(all(rows.get(name,{}).get('sha256')==h for name,h in p['critical_source_rows'].items()),'runtime mandatory security source closure')
    return rows


def gates(root,p):
    q=Path(p['package'])/'share/ap5-qualification'
    validation=sealed(q/'joined-validation.json',p['joined_validation_sha256'])
    caps=sealed(q/'tested-capabilities.json',p['tested_capabilities_sha256'])
    plan=sealed(root/'testing/lab-ark-funded/gate-plan.json',p['gate_plan_sha256'])
    contract=sealed(root/'testing/lab-ark-funded/source-contract.json',p['source_contract_sha256'])
    require(validation['schema']=='spc.lab.ap5.join-validation/v1' and validation['source_manifest_sha256']==p['source_manifest_sha256']
        and validation['binaries']==p['binaries'],'runtime gate source/binary tuple')
    require(caps['source_manifest_sha256']==p['source_manifest_sha256'] and caps['contract_sha256']==p['source_contract_sha256']
        and caps['capabilities']==plan['capabilities'] and caps['checks_sha256']==hashlib.sha256(canonical_json(validation['checks'])).hexdigest(),'runtime tested capability/gate seal')
    require(contract['gate_plan_sha256']==p['gate_plan_sha256'],'runtime gate-plan source contract')
    require([c['name'] for c in validation['checks']]==[s['name'] for s in plan['checks']],'runtime exact gate census')
    for check,step in zip(validation['checks'],plan['checks']):
        require(type(check['exit']) is int and check['exit']==0 and check['source_manifest_sha256']==p['source_manifest_sha256']
            and check['executed_cases']==sorted(step['cases']),'runtime gate not genuinely completed/source-bound')
        log=Path(check['log']['path']);require(log==q/check['name']/'output.log' and digest(log)==check['log']['sha256'],'runtime gate log changed')
        require(digest(check['execution_program']['path'])==check['execution_program']['sha256'],'runtime gate program changed')
    return validation


def verified_source_root(binary,role):
    p=selected(binary,role);root,manifest=compiler_root(binary,p,role);source_tree(root,manifest,p)
    return root,p


def bundle_references(value):
    checked=0
    if isinstance(value,dict):
        if 'path' in value and 'sha256' in value:
            path=Path(value['path']);require(path.is_absolute() and '..' not in path.parts and path.is_file()
                and digest(path)==value['sha256'],'selected bundle referenced artifact changed')
            checked+=1
        for item in value.values():checked+=bundle_references(item)
    elif isinstance(value,list):
        for item in value:checked+=bundle_references(item)
    return checked


def admit(binary,bundle):
    p=selected(binary);root,manifest=compiler_root(binary,p,'lab');rows=source_tree(root,manifest,p);validation=gates(root,p)
    from runtime_qualification import supplemental
    extra=supplemental(p)
    if bundle is None:raise MissingComponent('actual_canonical_runtime_bundle_binding_required')
    require(isinstance(bundle,dict) and set(bundle)=={'path','sha256'} and bundle['sha256'] in p['bundle_sha256'],'unreviewed selected runtime bundle profile')
    value=sealed(bundle['path'],bundle['sha256'])
    references=bundle_references(value) if p.get('verify_all_bundle_refs',False) else 0
    require(value['schema']=='spc.lab.ap5.bundle/v1' and value['scope']=='full' and value['base_revision']==p['base_revision']
        and value['joined_source_manifest']['sha256']==p['source_manifest_sha256']
        and value['joined_validation_receipt']['sha256']==p['joined_validation_sha256'],'selected runtime bundle/source/gates mismatch')
    for role,name in APP_NAMES.items():
        require(value['binaries'][role]=={'path':str(Path(p['package'])/'bin'/name),'sha256':p['binaries'][role]},'selected bundle has another runtime application')
    policy_ref=value['pool_control_source_policy'];policy=sealed(policy_ref['path'],policy_ref['sha256'])
    require(policy['source_root']==str(root),'selected bundle policy root differs from actual compiler source')
    return {'entrypoint':'runtime','profile_id':p['id'],'runtime_source_sha256':p['runtime_source_sha256'],
        'binary_sha256':binary['sha256'],'source_manifest_sha256':p['source_manifest_sha256'],'verified_bundle_sha256':bundle['sha256'],
        'verified_source_root':str(root),'verified_deriver':p['deriver'],'source_rows_verified':len(rows),'gate_checks_verified':len(validation['checks']),
        'phase_owner':'runtime','phase_regressions':p['supplemental_regressions'],'supplemental_checks_verified':extra,
        'bundle_references_verified':references,'scope':p['admission_scope'],'full_native_audited_campaign':False}
