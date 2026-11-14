"""Supplemental qualification must bind genuine complete, unskipped test logs."""
import copy
import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from oracle import Invalid
from runtime_qualification import rust_groups, supplemental
from runtime_profile import bundle_references


LOG='running 1 test\ntest engine::cleanup ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 9 filtered out; finished in 0.01s\n'


class RuntimeQualificationTests(unittest.TestCase):
    def test_exact_runner_groups_refuse_marker_truncation_failure_or_duplicate(self):
        expected=[{'passed':['engine::cleanup'],'ignored':{}}]
        self.assertEqual(rust_groups(LOG),expected)
        for text in ('passed=true\n',LOG.split('test result:')[0],LOG.replace('... ok','... FAILED'),
                     LOG.replace('1 passed','2 passed'),LOG.replace('running 1 test','running 2 tests'),
                     LOG.replace('test result:', 'test engine::cleanup ... ok\ntest result:')):
            with self.subTest(text=text),self.assertRaises(Invalid):rust_groups(text)

    def test_ignored_case_is_not_counted_as_a_pass(self):
        text=LOG.replace('... ok','... ignored, explicit host gate').replace('1 passed','0 passed').replace('0 ignored','1 ignored')
        self.assertEqual(rust_groups(text),[{'passed':[],'ignored':{'engine::cleanup':'ignored, explicit host gate'}}])

    def test_supplemental_log_and_source_and_complete_required_census_are_pinned(self):
        with tempfile.TemporaryDirectory(prefix='runtime-supplemental-') as folder:
            base=Path(folder);log=base/'cleanup.log';log.write_text(LOG)
            sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
            review={'schema':'spc.qa.runtime-supplemental/1','source_root':'/unit/compiler','source_manifest_sha256':'11'*32,
                'source_rows':{'engine.rs':'22'*32},'checks':[{'name':'cleanup','log':{'path':'cleanup.log','sha256':sha(log)},
                    'groups':rust_groups(LOG),'required_cases':['engine::cleanup'],'provenance':{'source_root':'/unit/compiler'},'scope':'synthetic test only'}]}
            policy={'source_root':'/unit/compiler','source_manifest_sha256':'11'*32,'critical_source_rows':{'engine.rs':'22'*32},
                'required_supplemental_checks':['cleanup'],'requires_private_run_token_hash':True}
            def bind(value):
                path=base/'review.json';path.write_text(json.dumps(value));policy['supplemental_evidence']={'path':'review.json','sha256':sha(path)}
            bind(review);self.assertEqual(supplemental(policy,base=base)[0]['name'],'cleanup')
            for change in ('source','row','omit','required','ignored'):
                bad=copy.deepcopy(review)
                if change=='source':bad['source_root']='/foreign/compiler'
                elif change=='row':bad['source_rows']['engine.rs']='33'*32
                elif change=='omit':bad['checks']=[]
                elif change=='required':bad['checks'][0]['required_cases']=['another::cleanup']
                else:bad['checks'][0]['groups']=[{'passed':[],'ignored':{'engine::cleanup':'ignored'}}]
                bind(bad)
                with self.subTest(change=change),self.assertRaises(Invalid):supplemental(policy,base=base)
            bind(review);log.write_text('passed=true\n')
            with self.assertRaisesRegex(Invalid,'seal changed'):supplemental(policy,base=base)
            policy['supplemental_evidence']['path']='../review.json'
            with self.assertRaises(Invalid):supplemental(policy,base=base)
            del policy['supplemental_evidence']
            with self.assertRaises(Invalid):supplemental(policy,base=base)

    def test_selected_bundle_mutable_references_cannot_outlive_their_hash(self):
        with tempfile.TemporaryDirectory(prefix='runtime-bundle-refs-') as folder:
            path=Path(folder)/'binding.json';path.write_bytes(b'actual reference')
            bundle={'policy':{'path':str(path),'sha256':hashlib.sha256(path.read_bytes()).hexdigest()}}
            self.assertEqual(bundle_references(bundle),1)
            path.write_bytes(b'replaced reference')
            with self.assertRaises(Invalid):bundle_references(bundle)


if __name__=='__main__':unittest.main()
