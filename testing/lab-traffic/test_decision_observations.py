"""Source-only 0012 schema/wire fixtures. No PG, node or wallet execution."""
import copy
import hashlib
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest

import decision_observations as d
from case_contract import canonical_json
from checkpoint_retention import CheckpointRetention
from economic_checkpoint import query, validate_initial
from oracle import Inconclusive, Invalid
from payout_history import transaction_effects, rail_effects, proposed_outputs
from pool_source import PoolSource
from ruleb_sql import replay_checkpoint
from source_feed_codec import CONTRACT, _sign_digest, tagged
from storage import query_spec
from test_audited_window import fixture, encode_events, pub
from test_payout_history import rail_case, PUB

STAMP='1970-01-01T00:01:40.001600+00:00'


def base():
    events,o,chain,migrations,key,identity=fixture(0)
    o['schema']=d.SCHEMA;o['query_id']=d.QUERY_ID;o['server']['snapshot']='1:100:'
    for name in d.layout()[0]:o['tables'].setdefault(name,[])
    o['counts']={name:len(rows) for name,rows in o['tables'].items()}
    return events,o,chain,migrations,key,identity


def append(o,kind,body,subject,attempt=None,xid=10,stamp=STAMP):
    rows=o['tables']['source_decision_observation'];oid=len(rows)+1
    value={'observation_id':oid,'schema_version':1,'kind':kind,'subject':'\\x'+subject,'attempt':None if attempt is None else '\\x'+attempt,
        'transaction_id':xid,'snapshot_boundary':f'1:100:{xid}','observed_at':stamp,'body':copy.deepcopy(body)}
    rows.append(value);h=o['tables']['source_accounting_history']
    h.append({'event_id':h[-1]['event_id']+1,'transaction_id':xid,'table_name':'source_decision_observation','operation':'INSERT',
              'before_row':None,'after_row':copy.deepcopy(value),'recorded_at':stamp})
    o['history_boundary']['event_watermark']=h[-1]['event_id'];o['counts']['source_accounting_history']=len(h);o['counts']['source_decision_observation']=len(rows)
    return value


def front_body(events):
    p=events[1][1]
    return {'schema':'spc.front-derivation/1','source_session':'01'*16,'front_id':0,'a_send_x':p['a_send_x'],
        'queried_identity_census':[],'census_complete':True,'status':'frozen_job','job_id':p['economic']['job_id'],
        'generation':int(p['generation']),'height':p['height'],'economic_revision':p['economic']['revision'],
        'distribution_fingerprint':'44'*32,'rows':[]}


def grant_fixture():
    oid=b'\1'*16;digest=b'\3'*32;nonce=b'\2'*16
    payload=b'SPAG\1'+oid+digest+nonce+(3).to_bytes(8,'little')+(110).to_bytes(8,'little')
    grant=payload+bytes.fromhex(_sign_digest(1,tagged('SPc/ArkAuthorizationV1',payload)))
    request=b'SPAR\1'+oid+digest+nonce
    return oid,digest,nonce,grant,request


class DecisionTests(unittest.TestCase):
    def test_closed_readonly_query_and_extended_checkpoint_reuse_existing_replay(self):
        events,o,chain,migrations,key,_=base()
        text,qid,schema,tables=query_spec(d.CONTRACT)
        self.assertEqual(text,query(source_contract=d.CONTRACT));self.assertEqual((qid,schema),(d.QUERY_ID,d.SCHEMA))
        self.assertIn('REPEATABLE READ READ ONLY',text);self.assertIn('"reconstruction_observation"',text)
        self.assertNotIn('SELECT *',text);self.assertEqual(set(tables),set(o['tables']))
        replay=replay_checkpoint(o,events[1][1]['economic'],chain,source_contract=CONTRACT,migration_checksums=migrations)
        self.assertEqual(replay['available_from_replay'],{})
        seed=validate_initial(o,migration_checksums=migrations,source_contract=CONTRACT)
        self.assertIsInstance(transaction_effects(o,seed),list)

    def test_actual_collector_retains_new_query_bytes_not_old_query_alias(self):
        events,o,_,_,_,_=base()
        with tempfile.TemporaryDirectory(prefix='t3-decision-capture-') as tmp:
            root=Path(tmp);path=root/'sql.json'
            body={'schema':'spc-lab-bound-sql-capture/1','binding':{'run_id':'run','pool_id':'pool'},'execution':{'exit_code':0,'timed_out':False},
                'query_sha256':hashlib.sha256(d.query().encode()).hexdigest(),'observation':o}
            path.write_bytes(canonical_json(body));ref={'path':str(path),'sha256':hashlib.sha256(path.read_bytes()).hexdigest()}
            retention=CheckpointRetention(root/'retained','run',{'front':'pool'},lambda *_:ref,CONTRACT,sql_contract=d.CONTRACT)
            try:self.assertEqual(retention.barrier('front')['revision'],o['tables']['economic_state'][0]['revision'])
            finally:retention.close(capture_barrier=False)
            old=CheckpointRetention(root/'old','run',{'front':'pool'},lambda *_:ref,CONTRACT)
            try:
                with self.assertRaises(Invalid):old.barrier('front')
            finally:old.close(capture_barrier=False)

    def test_front_actual_event_receipt_and_frozen_generation_join(self):
        events,o,_,_,key,_=base();body=front_body(events);events[1][1]['fingerprint']=body['distribution_fingerprint']
        value=append(o,'front_derivation',body,'01'*16,'02'*16)
        events=events[:1]+[('derivation_prepared',{'attempt':'02'*16,'record':body}),('derivation_observed',{'attempt':'02'*16,'record':body,'observation_id':value['observation_id']})]+events[1:]
        raw=encode_events(events);source=PoolSource(raw,independently_captured_log_sha256=hashlib.sha256(raw).hexdigest(),source_contract=CONTRACT,pool_pubkey=pub(7),decision_contract=d.CONTRACT)
        result=d.Index(o).front(source,key);self.assertEqual(result['observation_id'],1)
        with self.assertRaises(Inconclusive):PoolSource(raw,independently_captured_log_sha256=hashlib.sha256(raw).hexdigest(),source_contract=CONTRACT,pool_pubkey=pub(7))
        source.events[2]['payload']['observation_id']=None
        with self.assertRaises(Inconclusive):d.Index(o).front(source,key)

    def test_new_schema_real_provider_requires_front_receipt_before_prepared_material(self):
        from source_capture import SourceCapture
        from source_feed_provider import ReplayProvider
        from test_audited_window import PROFILE
        events,o,chain,migrations,key,identity=base();body=front_body(events);events[1][1]['fingerprint']=body['distribution_fingerprint']
        append(o,'front_derivation',body,'01'*16,'02'*16)
        events=events[:1]+[('derivation_prepared',{'attempt':'02'*16,'record':body}),('derivation_observed',{'attempt':'02'*16,'record':body,'observation_id':1})]+events[1:]
        raw=encode_events(events);source=PoolSource(raw,independently_captured_log_sha256=hashlib.sha256(raw).hexdigest(),source_contract=CONTRACT,pool_pubkey=pub(7),decision_contract=d.CONTRACT)
        capture=object.__new__(SourceCapture);capture.source_feed={'contract':CONTRACT,'sql_contract':d.CONTRACT};capture.sources={'front':source};capture.issues={};capture.policy=chain.policy
        capture.scenario={'run':{'id':'run'},'fronts':[{'id':'front','pool_id':'pool'}],'pools':[{'id':'pool','work_profile':PROFILE}]};capture.capture=SimpleNamespace(add=lambda *args:None)
        with tempfile.TemporaryDirectory(prefix='t3-decision-provider-') as tmp:
            root=Path(tmp);path=root/'sql.json';envelope={'schema':'spc-lab-bound-sql-capture/1','binding':{'run_id':'run','pool_id':'pool'},
                'execution':{'exit_code':0,'timed_out':False},'query_sha256':hashlib.sha256(d.query().encode()).hexdigest(),'observation':o}
            encoded=canonical_json(envelope);path.write_bytes(encoded);ref={'path':str(path),'sha256':hashlib.sha256(encoded).hexdigest()}
            retention=CheckpointRetention(root/'retained','run',{'front':'pool'},lambda *_:ref,CONTRACT,sql_contract=d.CONTRACT)
            try:
                provider=ReplayProvider(capture,{chain.tip['hash']:chain},{'alice':{'front':'front','pool_pubkey':pub(7),'sp_id':identity}},lambda *_:ref,lambda:chain,migrations,retention)
                p=source.jobs[key]['payload'];received={'job_id':1,'prev_hash':list(bytes.fromhex(p['prev_hash'])),'version':p['version'],'nbits':p['nbits'],'ntime':p['ntime'],
                    'coinbase_prefix':list(bytes.fromhex(p['coinbase_prefix'])),'coinbase_suffix':list(bytes.fromhex(p['coinbase_suffix'])),'merkle_path':[],
                    'share_target_be':list(bytes.fromhex(p['share_target'])),'pool_extranonce':[],'miner_extranonce_len':2}
                proof=provider.prepare('alice',received);self.assertEqual(json.loads(proof.bundle)['owed'][identity],0)
                self.assertTrue(any(r['trigger']['kind']=='provider_checkpoint_admitted' for r in retention.records))
                source.events[2]['payload']['observation_id']=None
                with self.assertRaises(Inconclusive):provider.prepare('alice',received)
            finally:retention.close(capture_barrier=False)

    def test_front_ACK_exact_bytes_and_unknown_or_private_fields_reject(self):
        events,o,_,_,_,_=base();body=front_body(events);value=append(o,'front_derivation',body,'01'*16,'02'*16)
        raw=canonical_json({'schema_version':1,'session':[1]*16,'attempt':[2]*16,'body':body})
        ack=canonical_json({'digest':hashlib.sha256(raw).hexdigest(),'observation_id':1})
        self.assertEqual(d.front_transport(raw,ack,d.Index(o))['observation_id'],1)
        with self.assertRaises(Invalid):d.front_transport(raw+b' ',ack,d.Index(o))
        for change in ('private','missing','version'):
            broken=copy.deepcopy(value)
            if change=='private':broken['body']['scan_secret']='11'*32
            elif change=='missing':del broken['body']['economic_revision']
            else:broken['schema_version']=2
            with self.subTest(change=change),self.assertRaises((Invalid,Inconclusive)):d.row(broken)

    def settlement(self):
        _,o,_,_,_,_=base();args=list(rail_case(raw=20000,prior=50,front_rail=3,settled='Forfeit',forfeit=20000))
        p,wire,block,prior,window,stored=args;stored['reconstruction_observation']=2
        bind={'block_hash':'11'*32,'height':1,'frozen_job':stored['frozen_job'],'payload_digest':'55'*32}
        read={'schema':'spc.settlement-delivery-read/1','binding':bind,'queried':['33'*32],'status':'complete',
              'results':[{'sp_id':'33'*32,'delivery':None,'registration_state':None,'registration_request':None,'epoch_mailbox':None}]}
        append(o,'settlement_delivery_read',read,'11'*32,'04'*16,xid=10)
        rec={'schema':'spc.settlement-reconstruction/1','binding':bind,'delivery_read_observation':1,'status':'complete','ledger':stored['validated_ledger'],'pending_deltas':[]}
        append(o,'settlement_reconstruction',rec,'11'*32,'04'*16,xid=11)
        o['tables']['source_accounting_history'].append({'event_id':100,'transaction_id':12,'table_name':'economic_block','operation':'INSERT','before_row':None,'after_row':copy.deepcopy(stored),'recorded_at':STAMP})
        return o,args

    def test_exact_reconstruction_reference_proves_absence_not_current_delivery(self):
        o,args=self.settlement();index=d.Index(o);read=index.settlement(args[-1],args[1])
        self.assertEqual(read['delivered'],[])
        o['tables']['ark_delivery']=[{'sp_id':'\\x'+'33'*32,'untrusted_current':True}]
        result=rail_effects(*args,delivery_read=read)
        self.assertEqual(result['per_identity'][0]['E'],50)
        self.assertNotIn('settlement_delivery_read_at_reconstruction',{m['field'] for m in result['missing']})

    def test_registered_lag_is_pending_even_if_front_froze_forfeit(self):
        o,args=self.settlement();read=o['tables']['source_decision_observation'][0]
        delivery={'sp_id':'\\x'+'33'*32,'ark_id':'\\x'+'01'*4,'b_ark':'\\x02'+PUB,'mailbox_mode':'none','mailbox_m':None,'mailbox_d':None,'epoch_fallback':0,'updated_at':STAMP}
        read['body']['results'][0]['delivery']=delivery
        o['tables']['source_accounting_history'][-3]['after_row']=copy.deepcopy(read)
        context=d.Index(o).settlement(args[-1],args[1])
        with self.assertRaisesRegex(Invalid,'impossible'):rail_effects(*args,delivery_read=context)
        args=list(rail_case(raw=20000,prior=50,front_rail=3,settled='Pending'))
        self.assertEqual(rail_effects(*args,delivery_read=context)['per_identity'][0]['E'],20050)

    def test_unknown_read_null_old_reference_bad_census_or_ledger_never_backfill(self):
        for change in ('null','unknown','census','ledger','visibility'):
            o,args=self.settlement()
            if change=='null':args[-1]['reconstruction_observation']=None
            else:
                target=o['tables']['source_decision_observation'][0 if change!='ledger' else 1]
                if change=='unknown':target['body'].update(status='unknown',results=None,error_class='read_or_persistence_failed')
                elif change=='census':target['body']['queried']=[]
                elif change=='ledger':target['body']['ledger'][0]['forfeited_sats']+=1
                else:o['tables']['source_decision_observation'][1]['snapshot_boundary']='1:100:10,11'
                for event in o['tables']['source_accounting_history']:
                    if event['table_name']=='source_decision_observation':event['after_row']=copy.deepcopy(o['tables']['source_decision_observation'][event['after_row']['observation_id']-1])
            with self.subTest(change=change),self.assertRaises((Invalid,Inconclusive)):d.Index(o).settlement(args[-1],args[1])

    def test_observed_bad_packed_height_is_not_accepted_as_native_settlement(self):
        o,args=self.settlement();args[-1]['frozen_job']['job_id']=1
        for value in o['tables']['source_decision_observation']:
            value['body']['binding']['frozen_job']['job_id']=1;d.row(value)
        for event in o['tables']['source_accounting_history']:
            if event['table_name']=='source_decision_observation':event['after_row']=copy.deepcopy(o['tables']['source_decision_observation'][event['after_row']['observation_id']-1])
        with self.assertRaisesRegex(Invalid,'packed height'):d.Index(o).settlement(args[-1],args[1])

    def test_premining_noncoinbase_shape_does_not_invent_settlement_ledger(self):
        p,_,block,prior,window,_=rail_case(raw=20000,prior=50,front_rail=3,settled='Forfeit',forfeit=20000)
        derived={'rows':{'33'*32:{'coinbase_key_status':'available','destination_bytes':None}}}
        result=proposed_outputs(p,block['transactions'][0],prior,window,block['subsidy'],block['fees'],derived)
        self.assertFalse(result['future_settlement_ledger_claimed']);self.assertEqual(result['paid'],0)
        p['entries'][0]['sats']='20050'
        with self.assertRaisesRegex(Invalid,'shaped'):proposed_outputs(p,block['transactions'][0],prior,window,block['subsidy'],0,derived)

    def test_high_value_proposal_pending_needs_actual_missing_keys(self):
        p,_,block,prior,window,_=rail_case(raw=500000,prior=50,front_rail=2,settled='Pending')
        derived={'rows':{'33'*32:{'coinbase_key_status':'available','destination_bytes':None}}}
        with self.assertRaisesRegex(Invalid,'key-unavailability'):proposed_outputs(p,block['transactions'][0],prior,window,block['subsidy'],0,derived)
        derived['rows']['33'*32]['coinbase_key_status']='missing_miner_keys'
        self.assertFalse(proposed_outputs(p,block['transactions'][0],prior,window,block['subsidy'],0,derived)['future_settlement_ledger_claimed'])

    def test_epoch_hold_and_public_only_front_inputs(self):
        events,*_=base();body=front_body(events);scan='02'+PUB;spend='02'+pub(2)
        identity=tagged('SPc/Id',bytes.fromhex(scan+spend)).hex()
        row={'sp_id':identity,'input_scope':'actual_front_cache','observed_unix_ms':100001,'scan_public':scan,'spend_public':spend,'coinbase_key_status':'available',
            'registration_presence':'present','registration':{'ark_id':'11'*4,'b_ark':scan,'mailbox_mode':'epoch_registered','mailbox_m':scan,'mailbox_d':spend,'epoch_fallback':1,'commit':None},
            'server':{'ark_id':'11'*4,'server_pubkey':scan,'mailbox_pubkey':spend,'testnet':True},'epoch_mailbox':None,'mailbox_selection':'hold_missing_epoch',
            'commit_provenance':'missing_commit_provenance','destination_bytes':None,'outcome':'epoch_mailbox_held'}
        body['rows']=[row];body['queried_identity_census']=[identity];d.front_body(body)
        row['outcome']='derived';row['destination_bytes']=b'address'.hex()
        with self.assertRaisesRegex(Invalid,'held epoch'):d.front_body(body)
        row['scan_secret']='11'*32
        with self.assertRaises(Inconclusive):d.front_body(body)
        del row['scan_secret'];row['outcome']='derivation_error';row['destination_bytes']=None
        row['registration']['b_ark']='00'*33;row['error_stage']='ark_destination';row['error_reason']='invalid B_ark registration'
        body['status']='failed_unadvertised_attempt';body['job_id']=body['generation']=None;body['census_complete']=False
        del body['economic_revision'];del body['distribution_fingerprint']
        d.front_body(body) # Retain invalid public input as a failed observation.


class AuthorizationTests(unittest.TestCase):
    def fixture(self):
        _,o,chain,_,_,_=base();oid,digest,nonce,grant,request=grant_fixture();bh=bytes.fromhex(chain.tip['hash'])[::-1]
        member={'order_id':'\\x'+oid.hex(),'ordinal':0,'block_hash':'\\x'+bh.hex(),'sp_id':'\\x'+'33'*32,'block_height':0,'due_height':0,'sats':50,'reserved':True,'orphaned':False}
        read={'schema':'spc.canonical-read/1','source_session':[1]*16,'started_unix_ms':99998,'finished_unix_ms':100000,'authoritative':True,
            'tip':0,'tip_hash':list(bh),'tip_after':0,'tip_hash_after':list(bh),'members':[{'height':0,'expected_hash':list(bh),'observed_hash':list(bh),'read_status':'present'}],'outcome':'complete'}
        auth={'order_id':'\\x'+oid.hex(),'nonce':'\\x'+nonce.hex(),'revision':3,'order_digest':'\\x'+digest.hex(),'checked_tip':0,'checked_tip_hash':'\\x'+bh.hex(),
              'expires_at':110,'grant_bytes':'\\x'+grant.hex(),'created_at':STAMP}
        append(o,'authorization_row',{'schema':'spc.authorization-row/1','operation':'INSERT','before':None,'after':auth},oid.hex(),xid=10)
        body={'schema':'spc.authorization-decision/1','request_bytes':request.hex(),'request_nonce':nonce.hex(),'now':100,'checked_tip':0,'checked_tip_hash':bh.hex(),
              'canonical_set':[bh.hex()],'canonical_read':read,'queried_members':[member],'sql_members':[{'member':member,'block_status':'confirmed','payout_status':'scheduled','payout_sats':50}],
              'outcome':'issued','grant_bytes':grant.hex(),'aggregate_revision':3}
        append(o,'authorization_decision',body,oid.hex(),xid=10)
        o['tables']['source_accounting_history'].append({'event_id':100,'transaction_id':10,'table_name':'ark_aggregate','operation':'UPDATE','before_row':{'state':'published','revision':2},
            'after_row':{'order_id':auth['order_id'],'state':'authorized','revision':3,'sats':50}})
        o['tables']['ark_send_authorization']=[auth]
        return o,chain,oid,nonce,grant

    def test_actual_grant_canonical_member_census_and_atomic_rows(self):
        o,chain,oid,_,_=self.fixture();index=d.Index(o)
        self.assertEqual(index.authorization('\\x'+oid.hex(),PUB,{chain.tip['hash']:chain})[0]['observation_id'],2)
        self.assertEqual(len(index.authorization_history()),1)

    def test_deleted_working_grant_is_retained_not_replaced(self):
        o,chain,oid,_,_=self.fixture();old=o['tables']['ark_send_authorization'].pop()
        append(o,'authorization_row',{'schema':'spc.authorization-row/1','operation':'DELETE','before':old,'after':None},oid.hex(),xid=11)
        index=d.Index(o);self.assertEqual(len(index.authorization_history()),2)
        self.assertEqual(len(index.authorization('\\x'+oid.hex(),PUB,{chain.tip['hash']:chain})),1)

    def test_incomplete_reversed_foreign_canonical_and_wrong_signature_refuse(self):
        for change in ('census','clock','foreign','signature'):
            o,chain,oid,_,_=self.fixture();v=o['tables']['source_decision_observation'][1];body=v['body']
            if change=='census':body['sql_members']=[]
            elif change=='clock':body['canonical_read']['finished_unix_ms']=0
            elif change=='foreign':body['canonical_read']['members'][0]['observed_hash']=[9]*32
            else:body['grant_bytes']=body['grant_bytes'][:-128]+'00'*64
            o['tables']['source_accounting_history'][-2]['after_row']=copy.deepcopy(v)
            with self.subTest(change=change),self.assertRaises((Invalid,Inconclusive)):d.Index(o).authorization('\\x'+oid.hex(),PUB,{chain.tip['hash']:chain})

    def test_unknown_reversed_canonical_observation_remains_unknown_not_a_grant(self):
        o,_,oid,_,_=self.fixture();prior=o['tables']['source_decision_observation'][1]['body'];read=copy.deepcopy(prior['canonical_read']);read['finished_unix_ms']=0
        body={'schema':'spc.authorization-decision/1','request_bytes':prior['request_bytes'],'now':100,'canonical_read':read,'outcome':'canonical_read_unknown','grant_bytes':None}
        value=append(o,'authorization_decision',body,oid.hex(),xid=11);d.row(value)

    def consumption_fixture(self):
        o,chain,oid,nonce,grant=self.fixture();raw=b'SPAC\1'+grant+(100001).to_bytes(8,'little')
        body={'schema':'spc.authorization-consumption/1','status':'recorded','request_bytes':raw.hex(),'request_digest':tagged('SPc/ArkConsumptionV1',raw).hex(),
            'grant_bytes':grant.hex(),'consumed_at_unix_ms':100001,'meaning':'lease_consumed_before_possible_cosign_not_delivery'}
        value=append(o,'authorization_consumption',body,oid.hex(),nonce.hex(),xid=11)
        ack=b'SPCA\1'+tagged('SPc/ArkConsumptionV1',raw)+value['observation_id'].to_bytes(8,'little')+(100002).to_bytes(8,'little')
        return o,value,raw,ack

    def test_exact_opaque_SPAC_SPCA_and_possible_escape_not_delivery(self):
        o,value,raw,ack=self.consumption_fixture();index=d.Index(o)
        result=index.consume(value['observation_id'],ack,PUB,{'schema':'spc.cosign-escape/1','order_id':'01'*16,'nonce':'02'*16,'observation_id':3,'entered_unix_ms':100004})
        self.assertEqual((len(raw),len(ack)),(162,53));self.assertTrue(result['possible_cosign_escape']);self.assertFalse(result['wallet_received'])
        self.assertEqual(result['DB_unix_ms'],100002) # PG bigint cast rounds, it does not truncate.
        for extra in (raw+b'identity',raw[:-1]):
            with self.assertRaises(Invalid):d.consumption(extra,PUB)
        with self.assertRaises(Invalid):d.consumption_ack(ack+b'block_hash')

    def test_lost_expired_or_foreign_ACK_stays_pinned(self):
        o,value,raw,ack=self.consumption_fixture();index=d.Index(o)
        with self.assertRaisesRegex(Inconclusive,'stays_pinned'):index.consume(3,None,PUB)
        with self.assertRaises(Invalid):index.consume(3,ack[:5]+bytes(32)+ack[37:],PUB)
        value['observed_at']='1970-01-01T00:01:50+00:00';o['tables']['source_accounting_history'][-1]['after_row']=copy.deepcopy(value)
        expired=ack[:-8]+(110000).to_bytes(8,'little')
        with self.assertRaisesRegex(Inconclusive,'pinned'):d.Index(o).consume(3,expired,PUB)

    def test_escape_trace_is_local_observation_not_extra_Ark_fields(self):
        raw=('INFO spc_ark_observation: existing lease enters possible cosign escape schema="spc.cosign-escape/1" order_id='+ '01'*16+' nonce='+'02'*16+' observation_id=3 entered_unix_ms=100004\n').encode()
        self.assertEqual(d.escape_records(raw)[0]['observation_id'],3)
        with self.assertRaises(Inconclusive):d.escape_records(raw[:-1])
        with self.assertRaises(Invalid):d.escape_records(raw[:-1]+b' sp_id=forbidden\n')


if __name__=='__main__':unittest.main()
