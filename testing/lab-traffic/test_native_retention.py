"""Final producer signed synthetic bytes; never a live/native-build receipt."""
import copy
import hashlib
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch
from types import SimpleNamespace

import native_codec as native
import native_retention as retention
from native_observer import replay
from oracle import Inconclusive, Invalid
from case_contract import canonical_json
from test_native_audit import fixture

FIXTURE=Path(__file__).resolve().parent/'fixtures/native-evidence-v1'
SEALS={
    'manifest.json':'13cba2fba244a1914790c510395084fc025bde2570b581ac8c3ad400b64db74f',
    'record.json':'f12ff73e6f24126b12f7812ee2550f1e211f4793b4741c216efc9c70b8e564b3',
    'request.json':'43020688e0b6d1c0f3ea42e7cda791efa6739acdadd19769c0eac95d72bf557b',
    'response.json':'1d16911b57ca4be46ff533efecea3d3af3dae27cb186bf5482f9a4e39ab76a00',
    'handle.json':'cd93e2e3551881c6d44c40cdb3d4551b4afdcc49f23bb5cfcee0b260c4726fb0',
}


def inputs():
    raw={name:(FIXTURE/name).read_bytes() for name in SEALS}
    assert all(retention.sha(raw[n])==digest for n,digest in SEALS.items())
    return raw,{name:json.loads(value) for name,value in raw.items()}


class RetentionCodecTests(unittest.TestCase):
    def setUp(self):
        self.raw,self.values=inputs();self.m=self.values['manifest.json'];self.h=self.values['handle.json']
        _,_,self.bindings,self.collectors,self.events=fixture(9)
        self.binding=self.bindings[('miner','connection')]

    def decode(self,raw=None,m=None,h=None):
        return retention.verify_exchange(self.raw['record.json'] if raw is None else raw,self.m if m is None else m,
            self.raw['manifest.json'],self.h['request_digest']+'.json',self.binding,self.h if h is None else h)

    def committed_events(self):
        events=copy.deepcopy(self.events)
        events[0]['occurred_at_wall_ms']=999
        events[1]['event']['payload']['durable_evidence']=copy.deepcopy(self.h)
        return events

    def test_original_signed_bytes_verified_once_then_joined_to_commit_activation_hash(self):
        with patch.object(native,'verify_response',wraps=native.verify_response) as verify:
            result=self.decode()
            self.assertEqual(result.request,self.values['request.json']);self.assertEqual(result.response,self.values['response.json'])
            detail=replay(self.committed_events(),self.bindings,self.collectors,{self.h['request_digest']:result},require_durable=True)
            verify.assert_called_once()
        self.assertEqual(detail['status'],'PASSED')
        self.assertTrue(detail['durable_event_join_required']);self.assertEqual(detail['retained_not_activated'],[])

    def test_final_locator_version_and_exact_manifest_process_root(self):
        m=retention.manifest(self.raw['manifest.json'],Path('/unit'),'run','miner',self.binding['observed_miner'],Path('/unit/private/miner.toml'),self.m['config'])
        loc={'schema':retention.LOCATOR_SCHEMA,'capability':retention.CAPABILITY,'directory':m['config']['directory'],
             'manifest_relative_path':self.h['manifest_relative_path'],'manifest_sha256':retention.sha(self.raw['manifest.json'])}
        self.assertEqual(retention.locator(loc,m,self.raw['manifest.json']),loc)
        for k,v in (('schema',retention.SCHEMA),('directory','/foreign/private/native-evidence/miner'),('capability','healthy')):
            with self.subTest(field=k),self.assertRaises(Invalid):retention.locator({**loc,k:v},m,self.raw['manifest.json'])

    def test_manifest_missing_field_unknown_version_or_foreign_process_reject(self):
        for change in ('missing','schema','pid','config','root'):
            m=copy.deepcopy(self.m)
            if change=='missing':del m['config']['max_sessions']
            elif change=='schema':m['schema']='spc.native-evidence-manifest/2'
            elif change=='pid':m['process']['pid']+=1
            elif change=='config':m['config_path']='/unit/private/other.toml'
            else:m['config']['directory']='/foreign/private/native-evidence/miner'
            with self.subTest(change=change),self.assertRaises((Invalid,Inconclusive)):
                retention.manifest(canonical_json(m),Path('/unit'),'run','miner',self.binding['observed_miner'],Path('/unit/private/miner.toml'),self.m['config'])

    def test_changed_payload_rejected_even_after_record_hash_is_resealed(self):
        record=copy.deepcopy(self.values['record.json']);response=copy.deepcopy(self.values['response.json'])
        response['receipt']['wallet_signature']='00'*64;raw_response=canonical_json(response)
        record['response_json_hex']=raw_response.hex();record['response_sha256']=retention.sha(raw_response)
        raw=canonical_json(record);handle={**self.h,'record_sha256':retention.sha(raw)}
        with self.assertRaisesRegex(Invalid,'signature'):self.decode(raw=raw,h=handle)

    def test_record_hash_payload_hash_checkpoint_and_typed_binding_cannot_be_substituted(self):
        for change in ('recordhash','payloadhash','checkpoint','binding','bool'):
            record=copy.deepcopy(self.values['record.json']);h=copy.deepcopy(self.h)
            if change=='payloadhash':record['request_sha256']='00'*32
            elif change=='checkpoint':record['checkpoint_digest']='00'*32
            elif change=='binding':record['binding']['wallet_instance'][0]^=1
            elif change=='bool':record['binding']['candidate_epoch']=True
            raw=canonical_json(record);h['record_sha256']='00'*32 if change=='recordhash' else retention.sha(raw)
            with self.subTest(change=change),self.assertRaises(Invalid):self.decode(raw=raw,h=h)

    def test_unknown_record_version_and_unbounded_or_noncanonical_hex_reject_before_decode(self):
        for field,value in (('version',2),('activation','activated'),('request_json_hex','aa'*(native.MAX_REQUEST+1)),
                            ('response_json_hex','AA'),('request_json_hex','0'),('kind','health')):
            record={**self.values['record.json'],field:value};raw=canonical_json(record);h={**self.h,'record_sha256':retention.sha(raw)}
            with self.subTest(field=field),self.assertRaises(Invalid):self.decode(raw=raw,h=h)

    def test_handle_paths_digest_and_sequence_are_not_discovery_authority(self):
        for field,value in (('record_relative_path','../../record.json'),('manifest_relative_path','process-9999-55/manifest.json'),
                            ('request_digest','00'*32),('receipt_sequence',2),('capability','healthy'),('schema','unknown')):
            with self.subTest(field=field),self.assertRaises(Invalid):self.decode(h={**self.h,field:value})

    def test_payload_UTF16_is_rejected_even_with_correct_envelope_hashes(self):
        for kind in ('request','response'):
            record=copy.deepcopy(self.values['record.json'])
            payload=json.dumps(self.values[kind+'.json'],separators=(',',':')).encode('utf-16-le')
            record[kind+'_json_hex']=payload.hex();record[kind+'_sha256']=retention.sha(payload)
            raw=canonical_json(record);h={**self.h,'record_sha256':retention.sha(raw)}
            with self.subTest(kind=kind),self.assertRaisesRegex(Invalid,'UTF8'):self.decode(raw=raw,h=h)

    def test_stored_but_cancelled_is_not_an_activation(self):
        exchange=self.decode();events=self.committed_events()[:1]
        row=copy.deepcopy(self.events[1]);row['event']={'name':'source_audit_unknown','payload':{'connection_id':'connection','session_slot':2,'candidate_epoch':3,'code':'native_evidence_expired_or_cancelled'}}
        events.append(row)
        result=replay(events,self.bindings,self.collectors,{self.h['request_digest']:exchange},require_durable=True)
        self.assertEqual(result['status'],'INCONCLUSIVE');self.assertEqual(result['verified_commits'],[])
        self.assertEqual(result['retained_not_activated'][0]['reason'],'unknown:native_evidence_expired_or_cancelled')
        events.append(copy.deepcopy(self.committed_events()[2]))
        with self.assertRaises(Invalid):replay(events,self.bindings,self.collectors,{self.h['request_digest']:exchange},require_durable=True)

    def test_missing_record_handle_and_hash_only_pair_cannot_satisfy_durable_path(self):
        events=self.committed_events();exchange=self.decode()
        self.assertEqual(replay(events,self.bindings,self.collectors,{},require_durable=True)['status'],'INCONCLUSIVE')
        with self.assertRaisesRegex(Inconclusive,'retained_native_exchange'):
            replay(events,self.bindings,self.collectors,{self.h['request_digest']:(exchange.request,exchange.response)},require_durable=True)
        del events[1]['event']['payload']['durable_evidence']
        with self.assertRaisesRegex(Inconclusive,'DurableHandle'):
            replay(events,self.bindings,self.collectors,{self.h['request_digest']:exchange},require_durable=True)

    def test_retained_verification_must_fall_between_candidate_and_commit(self):
        exchange=self.decode();events=self.committed_events();events[0]['occurred_at_wall_ms']=1001
        with self.assertRaisesRegex(Invalid,'clock boundary'):
            replay(events,self.bindings,self.collectors,{self.h['request_digest']:exchange},require_durable=True)

    def test_native_capture_supplies_verified_outer_map_and_requires_event_handle(self):
        from native_capture import NativeCapture
        capture=object.__new__(NativeCapture);miners=['miner','second','third','fourth'];exchange=self.decode()
        manifest={'processes':[{'role':role,'entity':name} for role in ('wallet','miner') for name in miners]}
        capture.c=SimpleNamespace(native_payloads=Mock(),manifest=lambda:manifest,run_id='run',scenario={'miners':[{'id':m} for m in miners]},capture=Mock())
        capture.poll=Mock();capture.issues={};capture.processes={m:{} for m in miners};capture.bindings=self.bindings;capture.files={}
        capture.logs={'controller':{'raw':b'actual-input-passed-to-controller-parser'}};capture.incarnations={}
        capture.retention_readers={}
        for m in miners:
            reader=Mock();reader.pending=[];reader.outer.return_value={self.h['request_digest']:exchange} if m=='miner' else {}
            capture.retention_readers[m]=reader
        with patch('native_observer.controller_events',return_value=self.committed_events()),patch('native_capture.collector_entries',return_value=self.collectors):
            result=capture._audit([])
        self.assertEqual(result['status'],'PASSED');self.assertTrue(result['durable_event_join_required']);self.assertEqual(result['retained_exchanges'],1)
        # Controlled dispatch fixture only: the four process observations above
        # are stubs. Real process/roster binding remains NativeCapture.poll's job.

    def test_pre_retention_source_or_uncompiled_manifest_without_Nix_root_cannot_admit(self):
        with tempfile.TemporaryDirectory(prefix='t3-retention-source-') as tmp:
            path=Path(tmp)/'manifest.json'
            for rows in ([],[{'path':k,'sha256':v} for k,v in retention.SOURCE_ROWS.items()]):
                raw=canonical_json({'files':rows});path.write_bytes(raw)
                with self.assertRaises(Inconclusive):retention.require_sources({'source_manifest':str(path),'source_manifest_sha256':retention.sha(raw)})


class StoreReaderTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory(prefix='t3-retention-');self.addCleanup(self.temp.cleanup)
        self.run=Path(self.temp.name)/'run';self.run.mkdir(mode=0o700)
        self.raw,self.values=inputs();self.m=copy.deepcopy(self.values['manifest.json'])
        self.m['config'].update(directory=str(self.run/'private/native-evidence/miner'),run_manifest=str(self.run/'run-manifest.json'))
        self.m['config_path']=str(self.run/'private/miner.toml');self.m['process']['uid']=os.getuid()
        self.root=Path(self.m['config']['directory']);self.session=self.root/retention.session_name(self.m['process'])
        for path in (self.run/'private',self.run/'private/native-evidence',self.root,self.session):path.mkdir(mode=0o700)
        self.record=copy.deepcopy(self.values['record.json']);self.name=self.record['request_digest']+'.json'
        self.sink=Mock();self.rewrite()

    def rewrite(self):
        manifest_raw=canonical_json(self.m);self.record['manifest_sha256']=retention.sha(manifest_raw)
        for name,raw in (('manifest.json',manifest_raw),(self.name,canonical_json(self.record))):
            path=self.session/name;path.write_bytes(raw);path.chmod(0o600)
        self.reader=retention.StoreReader(self.run,'run','miner',self.m['process'],self.m['config_path'],self.m['config'],self.sink)

    def test_actual_files_discovered_readonly_and_retained_then_pruning_detected(self):
        report=self.reader.poll();self.assertEqual(report['records'],1);self.assertFalse(report['locator_observed'])
        self.assertEqual(self.sink.call_count,2)
        self.reader.poll();self.assertEqual(self.sink.call_count,2)
        (self.session/self.name).unlink()
        with self.assertRaisesRegex(Inconclusive,'pruned'):self.reader.poll()

    def test_missing_manifest_and_truncated_record_stay_inconclusive(self):
        (self.session/'manifest.json').unlink()
        with self.assertRaisesRegex(Inconclusive,'manifest_missing'):self.reader.poll()
        self.rewrite();self.reader.poll();(self.session/self.name).write_bytes(b'{')
        with self.assertRaisesRegex(Inconclusive,'truncated'):self.reader.poll()

    def test_symlink_record_and_ancestor_and_hardlink_refuse_without_following(self):
        path=self.session/self.name;original=path.read_bytes();path.unlink();foreign=self.run/'unrelated';foreign.write_bytes(original);foreign.chmod(0o600)
        path.symlink_to(foreign)
        with self.assertRaises((Invalid,OSError)):self.reader.poll()
        path.unlink();os.link(foreign,path)
        with self.assertRaisesRegex(Invalid,'links'):self.reader.poll()
        path.unlink();self.rewrite();moved=self.run/'moved-native-root';self.root.rename(moved);self.root.symlink_to(moved,target_is_directory=True)
        with self.assertRaises(OSError):self.reader.poll()

    def test_foreign_PID_manifest_and_replaced_session_refuse(self):
        self.reader.poll();m=copy.deepcopy(self.m);m['process']['pid']+=1
        (self.session/'manifest.json').write_bytes(canonical_json(m))
        with self.assertRaisesRegex(Invalid,'manifest rewritten'):self.reader.poll()
        self.rewrite();self.reader.poll();previous=self.root/'process-9999-9999';self.session.rename(previous);self.session.mkdir(mode=0o700)
        for name in ('manifest.json',self.name):
            target=self.session/name;target.write_bytes((previous/name).read_bytes());target.chmod(0o600)
        with self.assertRaisesRegex(Invalid,'directory replaced'):self.reader.poll()

    def test_record_count_session_count_and_total_bytes_are_bounded(self):
        self.m['config']['max_records']=1;self.rewrite()
        extra=self.session/('00'*32+'.json');extra.write_bytes((self.session/self.name).read_bytes());extra.chmod(0o600)
        with self.assertRaises((Invalid,Inconclusive)):self.reader.poll()
        extra.unlink();self.m['config']['max_sessions']=1;self.rewrite();(self.root/'process-9999-9999').mkdir(mode=0o700)
        with self.assertRaisesRegex(Inconclusive,'over_budget'):self.reader.poll()
        (self.root/'process-9999-9999').rmdir();self.m['config']['max_bytes']=retention.MAX_RECORD;self.rewrite()
        path=self.session/self.name;raw=path.read_bytes();path.write_bytes(raw+b' '*(retention.MAX_RECORD-len(raw)))
        with self.assertRaisesRegex(Invalid,'byte bound'):self.reader.poll()

    def test_config_above_consumer_capacity_or_wrong_root_cannot_expand_scan(self):
        for key,value in (('max_bytes',16*1024**3),('max_records',65536),('max_sessions',256),('directory','/outside')):
            c={**self.m['config'],key:value}
            with self.subTest(key=key),self.assertRaises((Invalid,Inconclusive)):
                retention.StoreReader(self.run,'run','miner',self.m['process'],self.m['config_path'],c,self.sink)

    def test_failed_retention_sequence_holes_are_not_filled_with_synthetic_records(self):
        self.record['receipt_sequence']=3;self.rewrite();self.assertEqual(self.reader.poll()['records'],1)
        stage=self.session/('.'+'22'*32+'-4.pending');stage.write_bytes(b'{');stage.chmod(0o600)
        report=self.reader.poll();self.assertEqual(report['pending_files'],[stage.name]);self.assertEqual(report['records'],1)


if __name__=='__main__':unittest.main()
