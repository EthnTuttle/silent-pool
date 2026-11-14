"""Profile binding negatives; fixture tuples never enter the public allowlist."""
import copy
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from case_contract import canonical_json
from oracle import Invalid, Inconclusive
import runtime_profile as rp


def sha(raw):return hashlib.sha256(raw).hexdigest()


class RuntimeProfileTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory(prefix='runtime-profile-');self.addCleanup(self.temp.cleanup)
        self.root=Path(self.temp.name);self.src=self.root/'compiler-source';self.pkg=self.root/'package'
        (self.pkg/'bin').mkdir(parents=True);self.src.mkdir()
        self.cli=self.pkg/'bin/spc-lab';self.cli.write_bytes(b'\x7fELFsynthetic-binding-unit')
        paths=['crates/spc-lab/src/runtime.rs','bins/spc-cpu-miner/src/connection/outbound.rs']
        rows=[]
        for name in paths:
            p=self.src/name;p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(name.encode())
            rows.append({'path':name,'sha256':sha(p.read_bytes()),'bytes':p.stat().st_size})
        raw=canonical_json({'files':rows});self.manifest=self.root/'manifest.json';self.manifest.write_bytes(raw)
        self.p={'package':str(self.pkg),'source_root':str(self.src),'deriver':'/nix/store/'+'a'*32+'-unit.drv','system':'x86_64-linux',
            'binaries':{'lab':sha(self.cli.read_bytes())},'source_manifest_sha256':sha(raw),'source_files':len(rows),
            'critical_source_rows':{r['path']:r['sha256'] for r in rows}}
        self.binary={'path':str(self.cli),'sha256':self.p['binaries']['lab'],'source_manifest':str(self.manifest),'source_manifest_sha256':sha(raw)}

    def probe(self,_):
        return {self.p['deriver']:{'env':{'src':str(self.src),'doInstallCheck':'1'},'outputs':{'out':{'path':str(self.pkg)}},'system':'x86_64-linux'}}

    def test_files_only_manifest_gets_root_only_from_checked_compiler(self):
        root,m=rp.compiler_root(self.binary,self.p,'lab',probe=self.probe)
        self.assertEqual(root,self.src);self.assertEqual(set(m),{'files'})
        self.assertEqual(len(rp.source_tree(root,m,self.p)),2)

    def test_counterfeit_manifest_and_executable_are_rejected(self):
        for kind in ('manifest','executable'):
            with self.subTest(kind=kind):
                path=self.manifest if kind=='manifest' else self.cli;old=path.read_bytes();path.write_bytes(old+b' ')
                try:
                    with self.assertRaises(Invalid):rp.compiler_root(self.binary,self.p,'lab',probe=self.probe)
                finally:path.write_bytes(old)

    def test_wrong_compiler_root_output_or_unqualified_derivation_refuses(self):
        for kind in ('root','output','checks','numeric_check_marker','system'):
            value=self.probe(None);item=value[self.p['deriver']]
            if kind=='root':item['env']['src']=str(self.root/'caller-root')
            elif kind=='output':item['outputs']['out']['path']=str(self.root/'other-package')
            elif kind=='checks':item['env']['doInstallCheck']=''
            elif kind=='numeric_check_marker':item['env']['doInstallCheck']=1
            else:item['system']='another-system'
            with self.subTest(kind=kind),self.assertRaises(Invalid):rp.compiler_root(self.binary,self.p,'lab',probe=lambda _:value)

    def test_missing_outbound_or_changed_source_cannot_pass_complete_census(self):
        m=json.loads(self.manifest.read_bytes());outbound=self.src/'bins/spc-cpu-miner/src/connection/outbound.rs'
        raw=outbound.read_bytes();outbound.unlink()
        with self.assertRaises(Invalid):rp.source_tree(self.src,m,self.p)
        outbound.write_bytes(raw+b'changed')
        with self.assertRaises(Invalid):rp.source_tree(self.src,m,self.p)
        with self.assertRaises(Invalid):rp.source_tree(self.src,{'files':m['files'][:1]},self.p)

    def test_other_profile_and_old_native_rows_do_not_enter_current_policy(self):
        p=rp.profile()
        with self.assertRaisesRegex(Invalid,'another runtime profile'):
            rp.selected({'sha256':p['binaries']['lab'],'source_manifest_sha256':rp.MANIFEST_SHA256,'runtime_profile':'another'})
        with self.assertRaises(Inconclusive):rp.selected({'sha256':'00'*32,'source_manifest_sha256':rp.MANIFEST_SHA256})
        with self.assertRaises(Inconclusive):rp.selected({'sha256':p['binaries']['lab'],'source_manifest_sha256':'00'*32})

    def test_98f_profile_is_distinct_and_seals_all_28_supplemental_cases(self):
        from runtime_qualification import supplemental
        p=rp.profile(rp.OPAQUE_MANIFEST_SHA256);old=rp.profile()
        self.assertEqual(p['runtime_source_sha256'],old['runtime_source_sha256'])
        self.assertNotEqual(p['binaries']['lab'],old['binaries']['lab'])
        self.assertEqual(p['bundle_sha256'],['3cf18a8ee2611ad698e3002ca7862b2d5a9ec3d7382aab025ca211513c310ce0'])
        self.assertTrue(p['requires_private_run_token_hash']);self.assertTrue(p['verify_all_bundle_refs'])
        checks=supplemental(p)
        self.assertEqual([len(c['required_cases']) for c in checks],[7,1,8,12])
        for name in ('bins/spc-cpu-miner/src/connection/outbound.rs','crates/spc-lab/src/wallet_native_render.rs',
                     'bins/spc-agent/src/wallet/native_audit_io.rs','crates/spc-lab/src/engine/native_audit_cleanup_tests.rs'):
            self.assertIn(name,p['critical_source_rows'])
        with self.assertRaises(Inconclusive):rp.selected({'sha256':old['binaries']['lab'],'source_manifest_sha256':rp.OPAQUE_MANIFEST_SHA256})
        self.assertEqual(rp.selected({'sha256':p['binaries']['lab'],'source_manifest_sha256':rp.OPAQUE_MANIFEST_SHA256})['id'],p['id'])

    def test_actual_Nix_flat_and_version4_formats_are_strict(self):
        drv='/nix/store/'+'a'*32+'-unit.drv';out='b'*32+'-output'
        data={'version':4,'derivations':{Path(drv).name:{'version':4,'env':{'__json':json.dumps({'src':'/nix/store/source','doInstallCheck':True})},'outputs':{'out':{'path':out}},'system':'x86_64-linux'}}}
        env,outputs,system=rp.derivation(data,drv)
        self.assertEqual(outputs['out'],'/nix/store/'+out);self.assertTrue(env['doInstallCheck'])
        for kind in ('version','record_version','float_record_version','extra','path'):
            bad=copy.deepcopy(data)
            if kind=='version':bad['version']=5
            elif kind=='record_version':bad['derivations'][Path(drv).name]['version']=5
            elif kind=='float_record_version':bad['derivations'][Path(drv).name]['version']=4.0
            elif kind=='extra':bad['derivations']['other.drv']=next(iter(bad['derivations'].values()))
            else:bad['derivations'][Path(drv).name]['outputs']['out']['path']='../other'
            with self.subTest(kind=kind),self.assertRaises(Invalid):rp.derivation(bad,drv)

    def test_counterfeit_log_is_not_a_passing_gate_marker(self):
        q=self.pkg/'share/ap5-qualification';(q/'unit').mkdir(parents=True)
        tool=self.root/'tool';tool.write_bytes(b'unit tool')
        log=q/'unit/output.log';log.write_bytes(b'test genuine ... ok\n')
        plan={'checks':[{'name':'unit','cases':['genuine']}],'capabilities':{'runtime':['unit']}}
        checks=[{'name':'unit','exit':0,'source_manifest_sha256':self.p['source_manifest_sha256'],'executed_cases':['genuine'],
            'log':{'path':str(log),'sha256':sha(log.read_bytes())},'execution_program':{'path':str(tool),'sha256':sha(tool.read_bytes())}}]
        values={'joined-validation.json':{'schema':'spc.lab.ap5.join-validation/v1','source_manifest_sha256':self.p['source_manifest_sha256'],'binaries':self.p['binaries'],'checks':checks},
            'tested-capabilities.json':{'source_manifest_sha256':self.p['source_manifest_sha256'],'contract_sha256':'','capabilities':plan['capabilities'],'checks_sha256':sha(canonical_json(checks))}}
        directory=self.src/'testing/lab-ark-funded';directory.mkdir(parents=True)
        raw=canonical_json(plan);(directory/'gate-plan.json').write_bytes(raw);self.p['gate_plan_sha256']=sha(raw)
        raw=canonical_json({'gate_plan_sha256':sha(raw)});(directory/'source-contract.json').write_bytes(raw);self.p['source_contract_sha256']=sha(raw)
        values['tested-capabilities.json']['contract_sha256']=sha(raw)
        for name,value in values.items():
            raw=canonical_json(value);(q/name).write_bytes(raw)
            self.p['joined_validation_sha256' if name=='joined-validation.json' else 'tested_capabilities_sha256']=sha(raw)
        rp.gates(self.src,self.p)
        log.write_bytes(b'passed=true\n')
        with self.assertRaisesRegex(Invalid,'gate log changed'):rp.gates(self.src,self.p)

    def test_actual_Explicit_serde_address_shape_and_foreign_identity(self):
        from source_feed_runtime import rendered_address
        from test_native_audit import address
        actual=address()
        self.assertEqual(rendered_address({'address':actual},actual),actual)
        for value in ({'explicit':actual},{'fixture':'alice'},{'address':actual,'fixture':'alice'},
                      {'address':actual,'explicit':actual},{'address':'tsp1'+'q'*300},{'address':42}):
            with self.subTest(value=value),self.assertRaises(Invalid):rendered_address(value)
        foreign='tsp1qq2vfczmkedtrju0aexl0x8kqds6kpueyn4hwnewc83tky4vkup0k7qlhdguaq45xud9yggyf0c6exuvrv9za6wtnuwvz269kp7zr8tw7dcjewn6g'
        rendered_address({'address':foreign})
        with self.assertRaisesRegex(Invalid,'differs from bound'):rendered_address({'address':actual},foreign)

    def test_selected_envelope_hash_is_returned_and_foreign_root_or_tuple_refused(self):
        p=copy.deepcopy(self.p);p.update(base_revision='unit-revision',runtime_source_sha256='11'*32,id='unit-profile',supplemental_regressions={},admission_scope='unit-only',joined_validation_sha256='22'*32)
        p['binaries']={role:'33'*32 for role in rp.APP_NAMES}
        policy=self.root/'policy.json';policy.write_bytes(canonical_json({'source_root':str(self.src)}))
        value={'schema':'spc.lab.ap5.bundle/v1','scope':'full','base_revision':p['base_revision'],
            'joined_source_manifest':{'sha256':p['source_manifest_sha256']},'joined_validation_receipt':{'sha256':p['joined_validation_sha256']},
            'binaries':{role:{'path':str(self.pkg/'bin'/name),'sha256':p['binaries'][role]} for role,name in rp.APP_NAMES.items()},
            'pool_control_source_policy':{'path':str(policy),'sha256':sha(policy.read_bytes())}}
        refs=[]
        for index in range(2):
            path=self.root/('bundle'+str(index)+'.json');path.write_bytes(canonical_json(value)+b' '*index)
            refs.append({'path':str(path),'sha256':sha(path.read_bytes())})
        p['bundle_sha256']=[ref['sha256'] for ref in refs]
        with patch.object(rp,'selected',return_value=p),patch.object(rp,'compiler_root',return_value=(self.src,{})),patch.object(rp,'source_tree',return_value={}),patch.object(rp,'gates',return_value={'checks':[]}):
            for ref in refs:self.assertEqual(rp.admit(self.binary,ref)['verified_bundle_sha256'],ref['sha256'])
            with self.assertRaises(Inconclusive):rp.admit(self.binary,None)
            with self.assertRaises(Invalid):rp.admit(self.binary,{})
            with self.assertRaises(Invalid):rp.admit(self.binary,dict(refs[0],sha256=refs[1]['sha256']))
            for kind in ('root','tuple'):
                bad=copy.deepcopy(value)
                if kind=='root':
                    policy.write_bytes(canonical_json({'source_root':str(self.root/'foreign')}));bad['pool_control_source_policy']['sha256']=sha(policy.read_bytes())
                else:bad['binaries']['miner']['sha256']='44'*32
                path=self.root/'bad.json';path.write_bytes(canonical_json(bad));ref={'path':str(path),'sha256':sha(path.read_bytes())};p['bundle_sha256'].append(ref['sha256'])
                with self.subTest(kind=kind),self.assertRaises(Invalid):rp.admit(self.binary,ref)


if __name__=='__main__':unittest.main()
