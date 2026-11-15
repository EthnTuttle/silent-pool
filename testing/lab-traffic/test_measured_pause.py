"""Two-driver measured pause with real history-3 files and controlled acknowledgements.

These are public boundary fixtures, not signed-transaction/funding conformance.
No process, RPC, SQL, or runtime actor is launched.
"""
import copy
import hashlib
import json
from pathlib import Path
import tempfile
import threading
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

from campaign import load_controller
from journal import read_records
from oracle import Invalid, Inconclusive


class MeasuredPauseTests(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory(prefix='two-driver-pause-');self.addCleanup(self.tmp.cleanup)
        self.module=load_controller();self.c=object.__new__(self.module.Controller);self.root=Path(self.tmp.name)
        self.c.run=self.root;self.c.run_id='run';self.c.driver='txd-0'
        self.c.scenario={'traffic':{'driver':{'actors':[{'id':'txd-0'},{'id':'txd-1'}]}}}
        self.c.scenario['expanded_access']=True
        self.c.binding={'binaries':{'lab':{'sha256':'33'*32},'txd':{'sha256':'44'*32}}}
        self.c.observation_lock=threading.RLock();self.c.live_journal_readers={}
        self.c.args=SimpleNamespace(pause_seconds=1,measure_seconds=0,destination=None,witness=False)
        self.c.deadline=1000;self.c.measured_pause_acknowledgements={};self.c.commands={}
        self.now=1_000_000_000;self.paused=set();self.effects=[];self.calls=[];self.events=[];self.rows={};self.paths={};self.sequences={}
        for i,driver in enumerate(('txd-0','txd-1')):
            path=self.root/'private/traffic'/('d-'+str(i))/'private/traffic/history.jsonl'
            path.parent.mkdir(parents=True,mode=0o700);self.paths[driver]=path;self.sequences[driver]=0
            self.rows[driver]=[{'record':'header','seq':0,'format_magic':'SPC-LAB-TRAFFIC-JOURNAL','history_version':3,
                'protocol':'spc-lab-tx-journal/3','network':'regtest','key_mode':'lab_deterministic_run_seed_v1','derivation_version':2,
                'run_id':'run','driver_id':driver,'run_seed_fingerprint':'11'*32,'allocation_digest':'22'*32,
                'caps':{'unknown_retry_sends':3,'reorg_resubmits':3},'created_unix_ms':1}]
            self.write(driver)
        self.manifest={'run_id':'run','traffic_driver_paths':{d:{'journal_path':str(p)} for d,p in self.paths.items()}}
        self.c.manifest=lambda:self.manifest;self.c.bounded=Mock()
        self.c.capture=SimpleNamespace(add=lambda kind,value:self.events.append((kind,copy.deepcopy(value))))
        self.before_ack=None;self.during_interval=None;self.resume_failure=None
        self.c.command=self.command;self.c.common_chain=self.tick
        self.c.check=Mock();self.c.acceptance_hooks=Mock();self.c.version_rejection_case=Mock();self.c.version_rejection_loss_case=Mock()
        self.control=[];self.control_path=self.root/'private/commands.jsonl';self.control_path.touch(mode=0o600)
        self.incarnations={d:'inc-'+d for d in self.paths}
        for driver in self.paths:self.session(driver)

    def control_record(self,row):
        row={**row,'seq':len(self.control)+1};self.control.append(row)
        with self.control_path.open('ab') as f:f.write((json.dumps(row,separators=(',',':'))+'\n').encode())
        return row

    def audit(self,event):
        return self.control_record({'record':'traffic_audit','version':1,'boot_id':'unit-boot',
            'monotonic_ms':self.now//1_000_000,'event':event})

    def session(self,driver):
        self.audit({'event':'session','binding':{'run_id':'run','driver_id':driver,'incarnation':self.incarnations[driver],
            'controller_sha256':'33'*32,'actor_sha256':'44'*32,'acceptance_enabled':True}})

    def watermark(self,driver,cid,closed=True):
        self.audit({'event':'watermark','incarnation':self.incarnations[driver],'last_attempt_seq':0,
            'completed_count':0,'permit_revision':len(self.control)+1,'closed':closed,'command_id':cid})

    def write(self,driver):
        path=self.paths[driver];path.write_bytes(b''.join((json.dumps(r,separators=(',',':'))+'\n').encode() for r in self.rows[driver]));path.chmod(0o600)

    def append(self,driver,record):
        row={**record,'seq':len(self.rows[driver])};self.rows[driver].append(row)
        with self.paths[driver].open('ab') as f:f.write((json.dumps(row,separators=(',',':'))+'\n').encode())
        return row

    def send(self,driver,kind='broadcast_sent'):
        self.sequences[driver]+=1;seq=self.sequences[driver]
        row={'record':kind,'command_id':f'ordinary/{driver}/{seq}','txid':'bb'*32,'send_sequence':seq}
        if kind=='reorg_resubmit':row['proven_unspent']=['aa'*32+':0']
        self.append(driver,row);self.effects.append(driver)

    def command(self,label,kind,**kwargs):
        driver=kind['driver_id'];cid='cid-'+label;self.c.commands[label]={'command_id':cid,'kind':dict(kind)}
        self.calls.append((kind['type'],driver,cid))
        cmd={'command_id':cid,'run_id':'run','issued_at_wall':100,'actor':'operator','kind':dict(kind)}
        self.control_record({'record':'begin','command':cmd,'principal':'operator','hash_scheme':'expanded_v1',
            'payload_hash':self.c.driver_control_payload_hash(cmd,'operator')})
        if kind['type']=='pause_tx_driver':
            if self.before_ack:self.before_ack(driver)
            self.watermark(driver,cid)
            self.paused.add(driver)
            barrier=self.append(driver,{'record':'send_barrier','command_id':cid,'wall_unix_ms':100,
                'monotonic_ms':self.now//1_000_000,'last_txid':'bb'*32 if self.sequences[driver] else None,
                'send_sequence':self.sequences[driver]})
            detail={'status':'paused','command_id':cid,'wall_ms':100,'monotonic_ms':self.now//1_000_000,
                'last_used_send_sequence':barrier['send_sequence'],'last_txid':barrier['last_txid'],
                'last_intent_txid':barrier['last_txid'],'journal_seq':barrier['seq']}
            self.watermark(driver,cid)
        else:
            self.paused.remove(driver);self.append(driver,{'record':'send_resumed','command_id':cid})
            if driver==self.resume_failure:raise Inconclusive('controlled unknown resume acknowledgement')
            detail={'status':'resumed'}
        response={'state':'succeeded','command_id':cid,'detail':json.dumps(detail)}
        self.control_record({'record':'finish','command_id':cid,'outcome':{**response,'resolved_at':100,'journal_seq':0}})
        return response

    def tick(self):
        for driver in self.paths:
            if driver not in self.paused:self.send(driver)
        if self.during_interval:
            callback=self.during_interval;self.during_interval=None;callback()

    def sleep(self,seconds):self.now+=int(seconds*1_000_000_000)

    def run_pause(self):
        with patch.object(self.module.time,'monotonic',side_effect=lambda:self.now/1_000_000_000),\
             patch.object(self.module.time,'monotonic_ns',side_effect=lambda:self.now),\
             patch.object(self.module.time,'sleep',side_effect=self.sleep):
            self.c.measured()

    def test_real_two_journals_demonstrate_single_driver_pause_leaves_other_active(self):
        self.command('old-pause',{'type':'pause_tx_driver','driver_id':'txd-0'})
        before={r[0]['driver_id']:r for _,r,_ in self.c.journals()};self.tick()
        after={r[0]['driver_id']:r for _,r,_ in self.c.journals()}
        self.assertEqual(after['txd-0'],before['txd-0'])
        self.assertEqual(after['txd-1'][-1]['record'],'broadcast_sent');self.assertEqual(self.effects,['txd-1'])
        with self.assertRaisesRegex(Invalid,'driver census'):self.c.measured_journal_census(['txd-0'])

    def test_all_acknowledgements_precede_common_interval_and_both_drivers_resume(self):
        self.before_ack=lambda driver:self.send(driver) if driver=='txd-1' else None
        def data_only():
            for driver in self.paths:self.append(driver,{'record':'chain_observed','tip_block_hash':'aa'*32,'tip_height':1,
                'displaced_block_hashes':[],'observed_unix_ms':100})
        self.during_interval=data_only;self.run_pause()
        self.assertEqual([(kind,driver) for kind,driver,_ in self.calls],
            [('pause_tx_driver','txd-0'),('pause_tx_driver','txd-1'),('resume_tx_driver','txd-0'),('resume_tx_driver','txd-1')])
        self.assertEqual(self.effects,['txd-1']) # Allowed before that driver's own acknowledgement.
        self.assertEqual(self.c.measured_pause_acknowledgements,{})
        events=[k for k,_ in self.events];self.assertLess(events.index('measured_pause_baseline'),events.index('pause_read_interval'))
        self.assertEqual(events[:2],['measured_driver_pause_ack']*2)
        interval=next(v for k,v in self.events if k=='pause_read_interval')
        self.assertEqual(interval['drivers'],['txd-0','txd-1']);self.assertGreaterEqual(interval['end_monotonic_ns']-interval['start_monotonic_ns'],1_000_000_000)
        for driver in self.paths:
            raw=self.paths[driver].read_bytes().splitlines(keepends=True)
            for point in ('before','after'):
                record=interval[point][driver]
                self.assertEqual(hashlib.sha256(b''.join(raw[:record['records']])).hexdigest(),record['sha256'])
            self.assertEqual(read_records(self.paths[driver])[0][-1]['record'],'send_resumed')

    def test_send_or_reorg_by_either_acknowledged_driver_is_still_invalid(self):
        for driver,kind in (('txd-0','broadcast_sent'),('txd-1','broadcast_sent'),('txd-1','reorg_resubmit')):
            # Independent real-file fixture per case; no previous barrier reuse.
            with self.subTest(driver=driver,kind=kind):
                other=MeasuredPauseTests();other.setUp()
                try:
                    other.during_interval=lambda:other.send(driver,kind)
                    with other.assertRaisesRegex(Invalid,'acknowledged driver pause'):other.run_pause()
                    other.assertFalse(any(k=='resume_tx_driver' for k,_,_ in other.calls))
                    other.assertEqual(set(other.c.measured_pause_acknowledgements),{'txd-0','txd-1'})
                    other.assertIn('measured_pause_end',[k for k,_ in other.events])
                finally:other.doCleanups()

    def test_send_after_first_ack_while_waiting_for_second_ack_cannot_hide_in_baseline(self):
        self.before_ack=lambda driver:self.send('txd-0') if driver=='txd-1' else None
        with self.assertRaisesRegex(Invalid,'acknowledged driver pause'):self.run_pause()
        self.assertFalse(any(k=='resume_tx_driver' for k,_,_ in self.calls))

    def test_missing_after_journal_cannot_pass_by_zip_truncation(self):
        self.during_interval=lambda:self.paths['txd-1'].unlink()
        with self.assertRaisesRegex(Invalid,'journal driver census'):self.run_pause()

    def test_foreign_or_duplicate_journal_header_is_rejected(self):
        self.rows['txd-1'][0]['driver_id']='txd-0';self.write('txd-1')
        with self.assertRaises(Invalid):self.run_pause()

    def test_foreign_run_and_extra_manifest_driver_are_rejected(self):
        self.manifest['run_id']='foreign'
        with self.assertRaisesRegex(Invalid,'actual run'):self.run_pause()
        self.manifest['run_id']='run';self.manifest['traffic_driver_paths']['extra']={'journal_path':'/foreign'}
        with self.assertRaisesRegex(Invalid,'driver census'):self.c.measured_journal_census(['txd-0','txd-1'])

    def test_unknown_second_pause_never_starts_interval_or_resumes_first(self):
        def fail(driver):
            if driver=='txd-1':raise Inconclusive('controlled unacknowledged pause')
        self.before_ack=fail
        with self.assertRaises(Inconclusive):self.run_pause()
        self.assertEqual(set(self.c.measured_pause_acknowledgements),{'txd-0'})
        self.assertNotIn('measured_pause_baseline',[k for k,_ in self.events])
        self.assertFalse(any(k=='resume_tx_driver' for k,_,_ in self.calls))

    def test_wrong_acknowledgement_barrier_refuses(self):
        original=self.c.command
        def wrong(label,kind,**kwargs):
            response=original(label,kind,**kwargs);detail=json.loads(response['detail']);detail['journal_seq']+=1
            response['detail']=json.dumps(detail);return response
        self.c.command=wrong
        with self.assertRaisesRegex(Invalid,'barrier missing'):self.run_pause()

    def test_unknown_resume_cannot_leave_a_reusable_paused_claim(self):
        self.resume_failure='txd-0'
        with self.assertRaises(Inconclusive):self.run_pause()
        self.assertNotIn('txd-0',self.c.measured_pause_acknowledgements)
        self.assertIn('txd-1',self.c.measured_pause_acknowledgements)
        self.assertEqual([d for kind,d,_ in self.calls if kind=='resume_tx_driver'],['txd-0'])

    def test_acknowledged_pause_helper_joins_current_driver_controller_ack_and_barrier(self):
        self.command('pause',{'type':'pause_tx_driver','driver_id':'txd-0'})
        proof=self.c.acknowledged_traffic_pause('txd-0')
        self.assertEqual(proof['driver_id'],'txd-0');self.assertEqual(proof['incarnation'],'inc-txd-0')
        self.assertEqual(proof['ack']['journal_seq'],1);self.assertEqual(proof['journal']['barrier_seq'],1)
        self.assertIsNone(self.c.acknowledged_traffic_pause('txd-1'))
        self.assertEqual(len(self.calls),1) # The helper issued no command.

    def test_driver_control_hash_matches_actual_b16_pause_begin(self):
        command={'run_id':'ap5-98f-20261114-b16','kind':{'type':'pause_tx_driver','driver_id':'txd-0'}}
        self.assertEqual(self.c.driver_control_payload_hash(command,'operator'),
            '2f68b1fcb111bf0c1e43fe730760a84a500a46c2451570c4ab3700d209c7e627')

    def test_acknowledged_pause_helper_does_not_waive_paused_driver_send(self):
        self.command('pause',{'type':'pause_tx_driver','driver_id':'txd-0'});self.send('txd-0')
        with self.assertRaisesRegex(Invalid,'after acknowledged pause'):self.c.acknowledged_traffic_pause('txd-0')

    def test_acknowledged_pause_helper_rejects_controller_forward_even_without_local_send(self):
        self.command('pause',{'type':'pause_tx_driver','driver_id':'txd-0'})
        self.audit({'event':'forward_attempt','attempt':{'incarnation':'inc-txd-0','method':'sendrawtransaction'}})
        with self.assertRaisesRegex(Invalid,'forwarded an effect'):self.c.acknowledged_traffic_pause('txd-0')

    def test_resume_attempt_or_new_incarnation_invalidates_reuse(self):
        self.command('pause',{'type':'pause_tx_driver','driver_id':'txd-0'});self.resume_failure='txd-0'
        with self.assertRaises(Inconclusive):self.command('resume',{'type':'resume_tx_driver','driver_id':'txd-0'})
        self.assertIsNone(self.c.acknowledged_traffic_pause('txd-0'))
        self.command('pause-again',{'type':'pause_tx_driver','driver_id':'txd-0'})
        self.assertIsNotNone(self.c.acknowledged_traffic_pause('txd-0'))
        self.incarnations['txd-0']='new-incarnation';self.session('txd-0')
        self.assertIsNone(self.c.acknowledged_traffic_pause('txd-0'))

    def test_bad_barrier_or_wrong_actor_binding_cannot_be_reused(self):
        self.command('pause',{'type':'pause_tx_driver','driver_id':'txd-0'})
        self.rows['txd-0'][-1]['command_id']='foreign';self.write('txd-0')
        with self.assertRaisesRegex(Invalid,'barrier mismatch'):self.c.acknowledged_traffic_pause('txd-0')
        self.c.binding['binaries']['txd']['sha256']='55'*32
        with self.assertRaisesRegex(Invalid,'incarnation binding'):self.c.acknowledged_traffic_pause('txd-0')

    def test_malformed_pause_outcome_is_not_treated_as_no_acknowledgement(self):
        self.command('pause',{'type':'pause_tx_driver','driver_id':'txd-0'})
        self.control[-1]['outcome']['state']='unrecognized'
        self.control_path.write_bytes(b''.join((json.dumps(r,separators=(',',':'))+'\n').encode() for r in self.control))
        with self.assertRaisesRegex(Invalid,'malformed controller outcome'):self.c.acknowledged_traffic_pause('txd-0')


if __name__=='__main__':unittest.main()
