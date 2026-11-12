"""Final-prefix replay only: synthetic signed sources, no live RPC or writers."""
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import threading
from types import SimpleNamespace
import unittest
from unittest.mock import Mock

from canonical_capture import CanonicalCapture
from case_contract import canonical_json, required
from checkpoint_retention import CheckpointRetention
from economic_checkpoint import query
from final_audit import inputs, assess, FINAL_GATES
from native_capture import NativeCapture
from native_observer import controller_events
from native_retention import verify_exchange
from oracle import Inconclusive, Invalid
from rpc_capture import RpcRecorder
from source_capture import SourceCapture
from source_feed_codec import CONTRACT
from source_feed_runtime import CollectorRuntime
from test_audited_window import fixture, encode_events, parse, PROFILE
from test_native_audit import fixture as native_fixture
from test_native_retention import inputs as native_inputs
from test_rpc_capture import RawNode, Capture
from test_source_feed import pub

sha=lambda raw:hashlib.sha256(raw).hexdigest()


class FinalPrefixTests(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory(prefix='t3-final-prefix-');self.addCleanup(self.tmp.cleanup)
        root=Path(self.tmp.name);qa=root/'qa';qa.mkdir(mode=0o700);run=root/'run';run.mkdir(mode=0o700)
        (run/'private').mkdir(mode=0o700);(run/'logs').mkdir(mode=0o700)
        events,observation,chain,migrations,key,identity=fixture(0);self.source_events=events;raw_source=encode_events(events);source=parse(events)
        scenario={'pools':[{'id':'pool','work_profile':PROFILE}],'fronts':[{'id':'front','pool_id':'pool'}]}
        self.c=SimpleNamespace(run=run,priv=qa,run_id='run',policy=chain.policy,scenario=scenario,args=SimpleNamespace(witness=False),
            binding={'binaries':{'node':{'sha256':'11'*32}},'source_snapshot_sha256':'22'*32,
                'source_feed':{'contract':CONTRACT,'pool_pubkeys':{'front':pub(7)}}},native_capture=None)
        self.c.capture=Capture();source_capture=object.__new__(SourceCapture)
        source_capture.output=qa/'pool-source';source_capture.output.mkdir(mode=0o700)
        source_capture.expected={'front':run/'logs/front-front.log'};source_capture.policy=chain.policy;source_capture.scenario=scenario
        self.c.source_capture=source_capture
        for path in (source_capture.expected['front'],source_capture.output/'front.log'):self.write(path,raw_source)
        self.write(qa/'scenario.json',canonical_json(scenario))
        self.write(run/'private/pool-front.toml',b'[pool]\npool_payout_descriptor="unsupported-unit-descriptor"\n')
        recorder=RpcRecorder(qa,self.c.capture);canonical=CanonicalCapture(chain.policy,recorder,self.c.capture)
        block=chain.blocks[0];raw=block['raw']+b'\1'+block['transactions'][0]['raw']
        self.nodes=[RawNode(n,recorder,raw) for n in ('a','b')];canonical.sample(self.nodes);self.c.nodes={n.node_id:n for n in self.nodes}
        export=canonical.export(qa/'drain-chain-evidence.json',run_id='run',binary_hashes={'node':'11'*32},scenario_sha256=sha((qa/'scenario.json').read_bytes()),source_snapshot_sha256='22'*32)
        envelope={'schema':'spc-lab-bound-sql-capture/1','binding':{'run_id':'run','pool_id':'pool'},'execution':{'exit_code':0,'timed_out':False},
            'query_sha256':sha(query(source_contract=CONTRACT).encode()),'observation':observation}
        original=qa/'original-sql.json';self.write(original,canonical_json(envelope))
        retention=CheckpointRetention(qa/'checkpoints','run',{'front':'pool'},lambda *_:{'path':str(original),'sha256':sha(original.read_bytes())},CONTRACT)
        self.addCleanup(lambda:retention.close(capture_barrier=False));record=retention.barrier('front')
        self.runtime=CollectorRuntime(self.c);self.runtime.retention=retention;self.runtime.migrations=migrations
        stable=sha(canonical_json([chain.tip['hash'],[['pool',record['revision'],record['history_watermark'],{'front':source.heads}]]]))
        self.boundary={'schema':'spc-lab-owned-economic-drain/2','run_id':'run','status':'CONVERGED','stable_key':stable,
            'tip':{'height':0,'hash':chain.tip['hash']},'source_EOF':{'front':{'digest':sha(raw_source)}},'chain_evidence':export,
            'checkpoint_journal':{'sequence':retention.sequence,'head':retention.previous,'records':len(retention.records),'sha256':sha((retention.root/'attempts.jsonl').read_bytes())},
            'pools':{'pool':{'checkpoint':record,'source_heads':{'front':source.heads}}},'native_boundary':None,
            'pool_config_sha256':{'pool':[sha((run/'private/pool-front.toml').read_bytes())]}}
        self.runtime.drain_result={'status':'CONVERGED_COMPLETE_CAPTURE','tip':self.boundary['tip'],'stable_key':stable}
        self.seal()

    def write(self,path,raw):path.write_bytes(raw);path.chmod(0o600)

    def seal(self):
        path=self.c.priv/'economic-final-boundary.json';raw=canonical_json(self.boundary);self.write(path,raw)
        self.runtime.final_boundary_ref={'path':str(path),'sha256':sha(raw)}

    def test_exact_post_EOF_raw_SQL_watermark_and_source_heads_match_without_RPC(self):
        counts=[n.calls for n in self.nodes]
        boundary,sources,views,finals,proof=inputs(self.c,self.runtime)
        self.assertEqual(finals['pool']['history_boundary']['event_watermark'],boundary['pools']['pool']['checkpoint']['history_watermark'])
        self.assertEqual(views[boundary['tip']['hash']].tip['height'],0)
        result=assess(self.c,self.runtime)
        self.assertEqual(result['status'],'INCONCLUSIVE') # No mined block/full Q/native proof in this fixture.
        self.assertEqual(set(result['checks']),FINAL_GATES)
        self.assertEqual(counts,[n.calls for n in self.nodes])
        self.assertIn('final_sealed_prefix',required('funded'))

    def test_late_invalid_source_event_cannot_hide_behind_earlier_pass(self):
        inputs(self.c,self.runtime)
        path=self.c.source_capture.expected['front']
        with path.open('ab') as stream:stream.write(b'INFO spc_evidence: event="unknown_late_event" evidence={}\n')
        with self.assertRaisesRegex(Invalid,'sealed EOF'):assess(self.c,self.runtime)

    def test_invalid_event_included_by_drain_is_reparsed_despite_matching_file_seals(self):
        raw=encode_events(self.source_events+[("unknown_late_event",{"front_id":0})])
        for path in (self.c.source_capture.expected['front'],self.c.source_capture.output/'front.log'):self.write(path,raw)
        self.boundary['source_EOF']['front']['digest']=sha(raw);self.seal()
        with self.assertRaises((Invalid,Inconclusive)):assess(self.c,self.runtime)

    def test_current_SQL_or_watermark_cannot_replace_selected_final_capture(self):
        record=self.boundary['pools']['pool']['checkpoint'];path=Path(record['original_artifact']['path'])
        value=json.loads(path.read_bytes());value['observation']['history_boundary']['event_watermark']+=1
        self.write(path,canonical_json(value))
        with self.assertRaisesRegex(Invalid,'original SQL'):inputs(self.c,self.runtime)

    def test_changed_retention_or_stable_key_and_early_audit_refuse(self):
        self.boundary['stable_key']='00'*32;self.seal()
        with self.assertRaises(Invalid):inputs(self.c,self.runtime)
        self.runtime.drain_result=None
        with self.assertRaises(Inconclusive):self.runtime.final_audit_after_drain()

    def test_controller_preserves_old_failure_and_marks_late_prefix_failure(self):
        spec=importlib.util.spec_from_file_location('final_controller_test',Path(__file__).with_name('regtest-traffic-e2e.py'))
        module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
        c=object.__new__(module.Controller);c.results={'payout_ladder':{'status':'FAILED','detail':'earlier contradiction'}};c.capture=Mock();c.args=SimpleNamespace(witness=False);c.source_feed_runtime=Mock()
        c.source_feed_runtime.final_audit_after_drain.return_value={'status':'PASSED','checks':{k:{'status':'PASSED','detail':{}} for k in FINAL_GATES}}
        self.assertEqual(c.final_audit_after_drain()['status'],'FAILED')
        c.source_feed_runtime.final_audit_after_drain.side_effect=Invalid('late invalid event')
        with self.assertRaises(Invalid):c.final_audit_after_drain()
        self.assertEqual(c.results['final_sealed_prefix']['status'],'FAILED')


class FinalNativeTests(unittest.TestCase):
    def test_exact_final_native_prefix_passes_but_late_invalid_event_blocks(self):
        raw,values=native_inputs();_,_,bindings,collectors,events=native_fixture(9)
        events[0]['occurred_at_wall_ms']=999;events[1]['event']['payload']['durable_evidence']=values['handle.json']
        rows=[]
        for event in events:
            rows.append({'run_id':'run','event_id':event['event_id'],'source':'miner-miner','source_cursor':'token:55:'+str(event['event_id']),
                'actor':{'kind':'miner','id':'miner'},'command_id':None,'occurred_at_wall':event['occurred_at_wall_ms'],
                'occurred_at_mono':event['occurred_at_mono_ms']*1000000,'ingested_at':2000,
                'payload':{'kind':'miner_event','data':{'miner_id':'miner','incarnation':'token:55','driver_event_id':event['event_id'],'name':event['event']['name'],'payload':event['event']['payload'],'dropped_before':0}}})
        with tempfile.TemporaryDirectory(prefix='t3-final-native-') as tmp:
            root=Path(tmp);private=root/'private';private.mkdir(mode=0o700);out=root/'retained';out.mkdir(mode=0o700)
            prefix=b''.join(canonical_json(row)+b'\n' for row in rows)
            for path in (private/'evidence.jsonl',out/'prefix.json'):path.write_bytes(prefix);path.chmod(0o600)
            capture=object.__new__(NativeCapture);capture.lock=threading.RLock();capture.c=SimpleNamespace(run=root,run_id='run');capture.out=out
            capture.incarnations={'miner':'token:55'};capture.bindings=bindings;capture.processes={'miner':bindings[('miner','connection')]['observed_miner']};capture.files={}
            exchange=verify_exchange(raw['record.json'],values['manifest.json'],raw['manifest.json'],values['handle.json']['request_digest']+'.json',bindings[('miner','connection')])
            reader=Mock();reader.manifest_raw=raw['manifest.json'];reader.records={values['handle.json']['request_digest']+'.json':raw['record.json']};reader.pending=[];reader.outer.return_value={values['handle.json']['request_digest']:exchange}
            capture.retention_readers={'miner':reader}
            bound=[{'miner_id':m,'connection_id':con,'binding':v} for (m,con),v in sorted(bindings.items())]
            boundary={'schema':'spc-lab-native-final-prefix/1','run_id':'run','controller_prefix':{'source_path':str(private/'evidence.jsonl'),'path':str(out/'prefix.json'),'sha256':sha(prefix)},
                'miner_events_sha256':sha(canonical_json(controller_events(prefix,'run',capture.incarnations))),
                'stores':{'miner':{'manifest_sha256':sha(reader.manifest_raw),'records':{k:sha(v) for k,v in reader.records.items()}}},'bindings_sha256':sha(canonical_json(bound)),'incarnations':capture.incarnations}
            from unittest.mock import patch
            with patch('native_capture.collector_entries',return_value=collectors):
                self.assertEqual(capture.audit_after_drain([],boundary)['status'],'PASSED')
                late=copy.deepcopy(rows[-1]);late.update(event_id=5,source_cursor='token:55:5');late['payload']['data']['driver_event_id']=5;late['payload']['data']['payload']['candidate_epoch']=999
                with (private/'evidence.jsonl').open('ab') as stream:stream.write(canonical_json(late)+b'\n')
                with self.assertRaisesRegex(Inconclusive,'late_native_event'):capture.audit_after_drain([],boundary)


if __name__=='__main__':unittest.main()
