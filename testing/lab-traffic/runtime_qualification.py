"""Sealed original supplemental test receipts for reviewed Runtime profiles.

These files establish source/test provenance, never live campaign acceptance.
The enclosing profile hash is pinned by the consumer, not selected by callers.
"""
import hashlib
from pathlib import Path
import re

from oracle import require, strict_json


def rust_groups(text):
    text=re.sub(r'\x1b\[[0-?]*[ -/]*[@-~]','',text)
    groups=[];active=None;declared=0;seen=set()
    for line in text.splitlines():
        start=re.fullmatch(r'running (\d+) tests?',line.strip())
        if start:
            require(active is None,'supplemental Rust group truncated')
            declared=int(start[1]);active={'passed':[],'ignored':{}};seen=set();continue
        case=re.fullmatch(r'test (\S+) \.\.\. (.+)',line.strip())
        if case:
            require(active is not None and case[1] not in seen,'supplemental Rust duplicate/orphan test')
            seen.add(case[1])
            if case[2]=='ok':active['passed'].append(case[1])
            else:
                require(case[2].startswith('ignored'),'supplemental Rust test failed')
                active['ignored'][case[1]]=case[2]
        if line.startswith('test result:'):
            summary=re.match(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;',line)
            require(active is not None and summary is not None and int(summary[2])==0
                and int(summary[1])==len(active['passed']) and int(summary[3])==len(active['ignored'])
                and len(seen)==declared,'supplemental Rust summary/census mismatch')
            active['passed'].sort();groups.append(active);active=None
    require(active is None and bool(groups),'supplemental Rust missing/truncated execution')
    return groups


def artifact(ref,base):
    require(isinstance(ref,dict) and set(ref)=={'path','sha256'},'supplemental artifact reference')
    name=ref['path'];require(isinstance(name,str) and not Path(name).is_absolute() and '..' not in Path(name).parts,
        'supplemental artifact must be source-relative')
    path=base/name
    require(path.resolve().is_relative_to(base.resolve()) and path.is_file() and not path.is_symlink()
        and path.stat().st_size<=4*1024*1024,'supplemental artifact path/type/bound')
    with path.open('rb') as stream:raw=stream.read(4*1024*1024+1)
    require(hashlib.sha256(raw).hexdigest()==ref['sha256'],'supplemental artifact seal changed')
    return raw


def supplemental(p,*,base=None):
    if 'supplemental_evidence' not in p:
        require(not p.get('requires_private_run_token_hash',False),'opaque Runtime supplemental qualification missing')
        return []
    base=Path(__file__).with_name('profiles') if base is None else Path(base)
    review=strict_json(artifact(p['supplemental_evidence'],base))
    require(review['schema']=='spc.qa.runtime-supplemental/1' and review['source_root']==p['source_root']
        and review['source_manifest_sha256']==p['source_manifest_sha256'],'supplemental compiler-source binding')
    require(review['source_rows'] and all(p['critical_source_rows'].get(n)==h for n,h in review['source_rows'].items()),
        'supplemental reviewed source closure mismatch')
    for ref in review.get('owner_records',[]):artifact(ref,base)
    checks=review['checks'];require([c['name'] for c in checks]==p['required_supplemental_checks'],
        'supplemental required check census')
    results=[]
    for check in checks:
        groups=rust_groups(artifact(check['log'],base).decode('utf-8'))
        require(groups==check['groups'],'supplemental original execution inventory changed')
        passed=[case for group in groups for case in group['passed']]
        require(check['required_cases'] and all(passed.count(case)==1 for case in check['required_cases']),
            'supplemental required case absent/duplicated/ignored')
        require(check['provenance']['source_root']==p['source_root'],'supplemental check foreign source provenance')
        results.append({'name':check['name'],'log_sha256':check['log']['sha256'],
            'required_cases':check['required_cases'],'scope':check['scope']})
    return results
