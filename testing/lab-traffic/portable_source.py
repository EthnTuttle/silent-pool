#!/usr/bin/env python3
"""Portable QA source snapshots and genuine local execution receipts.

Neither operation admits a runtime, supplies funds, or invents passing checks.
The AP5 owner consumes the source descriptor and owns the shared coordinator.
"""
import argparse
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import tarfile
import time

ROOTS=('testing/lab-traffic','testing/lab-witness')
IGNORED={'.git','__pycache__','target','.venv','private','node_modules'}
API='t3-controller-r24'
RECIPE='spc.qa.source-tree/1'


def sha(raw):return hashlib.sha256(raw).hexdigest()


def encode(value):return (json.dumps(value,sort_keys=True,indent=2)+'\n').encode()


def inventory(repo):
    repo=Path(repo).resolve();rows=[]
    for name in ROOTS:
        root=repo/name
        if not root.is_dir():raise ValueError('required source directory missing: '+name)
        for path in sorted(root.rglob('*')):
            rel=path.relative_to(repo)
            if set(rel.parts)&IGNORED:continue
            if path.is_symlink():raise ValueError('source symlink: '+str(rel))
            if not path.is_file():continue
            if path.suffix not in {'.py','.md','.json'}:raise ValueError('unlisted source/data type: '+str(rel))
            info=path.stat()
            if not stat.S_ISREG(info.st_mode) or info.st_nlink!=1 or info.st_size>8*1024*1024:raise ValueError('source file type/link/size: '+str(rel))
            mode=stat.S_IMODE(info.st_mode)
            if info.st_mode&0o7000 or not mode&0o400:raise ValueError('source must be owner-readable without special mode bits: '+str(rel))
            # Git preserves the executable bit, not per-machine umask/read-only
            # modes. This normalization is part of the versioned recipe.
            git_mode=0o100755 if mode&0o111 else 0o100644
            raw=path.read_bytes();rows.append({'path':rel.as_posix(),'sha256':sha(raw),'bytes':len(raw),'mode':format(git_mode,'o')})
    if sum(r['bytes'] for r in rows)>32*1024*1024:raise ValueError('QA source snapshot exceeds 32 MiB')
    return rows


def fresh_output(repo,out):
    repo=Path(repo).resolve();out=Path(out).absolute()
    if out!=out.resolve() or out.is_relative_to(repo) or repo.is_relative_to(out):raise ValueError('output must be a canonical path outside checkout')
    out.mkdir(mode=0o700,parents=False,exist_ok=False);return out


def write(path,raw):
    with os.fdopen(os.open(path,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600),'wb') as stream:
        stream.write(raw);stream.flush();os.fsync(stream.fileno())


def snapshot(repo,out):
    repo=Path(repo).resolve();out=fresh_output(repo,out);rows=inventory(repo)
    manifest={'schema':'spc.qa.source-manifest/1','recipe':RECIPE,'api':API,'roots':list(ROOTS),'files':rows,'source_bytes':sum(r['bytes'] for r in rows)}
    data=encode(manifest);write(out/'manifest.json',data)
    buffer=io.BytesIO()
    with gzip.GzipFile(filename='',fileobj=buffer,mode='wb',mtime=0) as gz:
        with tarfile.open(fileobj=gz,mode='w',format=tarfile.USTAR_FORMAT) as archive:
            for row in rows:
                raw=(repo/row['path']).read_bytes()
                if len(raw)!=row['bytes'] or sha(raw)!=row['sha256']:raise ValueError('source changed during snapshot')
                info=tarfile.TarInfo('source/'+row['path']);info.size=len(raw);info.mode=int(row['mode'],8)&0o777;info.mtime=0;info.uid=info.gid=0
                archive.addfile(info,io.BytesIO(raw))
    raw=buffer.getvalue();write(out/'source.tar.gz',raw)
    if inventory(repo)!=rows:raise ValueError('source changed during snapshot')
    descriptor={'schema':'spc.lab.ap5.qa-source/v1','api':API,'archive':{'path':'source.tar.gz','sha256':sha(raw)},'manifest':{'path':'manifest.json','sha256':sha(data)}}
    encoded=encode(descriptor);write(out/'qa-source.json',encoded)
    return {'descriptor':str(out/'qa-source.json'),'descriptor_sha256':sha(encoded),'source_bytes':manifest['source_bytes'],'files':len(rows)}


def record_check(repo,out,purpose,argv,timeout):
    repo=Path(repo).resolve();out=fresh_output(repo,out);before=inventory(repo)
    if not argv:raise ValueError('explicit build/test argv required')
    import shutil
    program=shutil.which(argv[0])
    if program is None:raise ValueError('check executable unavailable')
    executable_sha=sha(Path(program).read_bytes());start=time.time_ns();exit_code=None;timed_out=False
    with os.fdopen(os.open(out/'check.log',os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600),'wb') as log:
        try:exit_code=subprocess.run([program,*argv[1:]],cwd=repo,stdout=log,stderr=subprocess.STDOUT,timeout=timeout).returncode
        except subprocess.TimeoutExpired:timed_out=True
        log.flush();os.fsync(log.fileno())
    after=inventory(repo);source=encode({'schema':'spc.qa.source-manifest/1','recipe':RECIPE,'files':before})
    write(out/'source-manifest.json',source)
    receipt={'schema':'spc.qa.local-check/1','purpose':purpose,'argv':[program,*argv[1:]],'executable_sha256':executable_sha,
        'executable_unchanged':sha(Path(program).read_bytes())==executable_sha,
        'started_unix_ns':start,'finished_unix_ns':time.time_ns(),'exit_code':exit_code,'timed_out':timed_out,'source_unchanged':before==after,
        'source_manifest':{'path':'source-manifest.json','sha256':sha(source)},'log':{'path':'check.log','sha256':sha((out/'check.log').read_bytes())},
        'scope':'actual_local_execution_receipt_not_runtime_or_funded_admission'}
    write(out/'receipt.json',encode(receipt))
    return receipt


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--repo',type=Path,default=Path(__file__).resolve().parents[2])
    sub=parser.add_subparsers(dest='action',required=True)
    snap=sub.add_parser('snapshot');snap.add_argument('--out',type=Path,required=True)
    check=sub.add_parser('record-check');check.add_argument('--out',type=Path,required=True);check.add_argument('--purpose',required=True)
    check.add_argument('--timeout',type=int,default=120);check.add_argument('argv',nargs=argparse.REMAINDER)
    args=parser.parse_args()
    if args.action=='snapshot':result=snapshot(args.repo,args.out)
    else:
        argv=args.argv[1:] if args.argv[:1]==['--'] else args.argv
        result=record_check(args.repo,args.out,args.purpose,argv,args.timeout)
    print(json.dumps(result,sort_keys=True))
    if args.action=='record-check' and (result['exit_code']!=0 or result['timed_out'] or not result['source_unchanged'] or not result['executable_unchanged']):return 1
    return 0


if __name__=='__main__':raise SystemExit(main())
