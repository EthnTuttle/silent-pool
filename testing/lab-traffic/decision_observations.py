"""Migration-0012 decision receipts, consumed without retrospective backfill.

Schemas are from sealed 4eb5495f source, not compiled/runtime admission.
Existing raw/Rule B replay and grant crypto remain the monetary oracles.
"""
from datetime import datetime, timezone
import hashlib
import re

from aggregate_history import parse_grant, grant_bytes
from case_contract import canonical_json
from oracle import Inconclusive, Reader, integer, require, strict_json, unhex
from signature_oracle import public_point
from source_feed_codec import fields, array, tagged
from source_jobs import MissingComponent

CONTRACT="e5283a85ddf91f9b44bd9fe1a678afe9b84a6f35ce7d468356605338e0f79803"
SCHEMA="spc-lab-sql-observation/5-fullrail-v1"
QUERY_ID="spc-lab-immutable-payout-rows/5-fullrail-v1"
MAX_BODY=8*1024*1024
COLUMNS="observation_id schema_version kind subject attempt transaction_id snapshot_boundary observed_at body"
KINDS={"settlement_delivery_read","settlement_reconstruction","front_derivation","authorization_decision","authorization_row","authorization_consumption"}
REGISTRATION_TABLES={
    "ark_registration_state":("sp_id last_request_id signed_fields_digest registration_rev updated_at","sp_id"),
    "ark_registration_request":("sp_id request_id signed_fields_digest outcome registration_rev error_code created_at","sp_id request_id"),
    "ark_epoch_mailbox":("sp_id height mailbox_mi","sp_id height")}
SOURCE_ROWS={
    "crates/spc-store/migrations/0012_fullrail_observations.sql":"12dfcd404a3dc3f52d15366600b900964ab6f4ce89fdf581c265327d29ebac88",
    "crates/spc-store/src/observations.rs":"ee7b501436ae30d024e425c26730c9ef8633a6029aa8dceb700aad772a63adf9",
    "crates/spc-ext/src/authorization.rs":"b7b6f29714c12378926306b676cfbb9c4802681158bf94416a3f76af7842b38c",
    "bins/spc-pool/src/front.rs":"92c2e50dc0cfff70c133136ace55c462cf7ae22d0c3c00fb1712ddae5a3d2da9",
    "bins/spc-pool/src/payout.rs":"250f4a86b3a335410bb27a9afbf22755b18089bc983d5f05505e7536109d5465",
    "bins/spc-pool/src/ark/send_fence.rs":"29a2ff7f58d2d45f95a90d0355a2802f8cdc8d0b6407996ef879ade5122b1a0b"}


def require_source(binary):
    from pathlib import Path
    raw=Path(binary['source_manifest']).read_bytes();require(hashlib.sha256(raw).hexdigest()==binary['source_manifest_sha256'],"decision source manifest seal")
    value=strict_json(raw);rows={r['path']:r['sha256'] for r in value['files']};require(len(rows)==len(value['files']),"duplicate decision source rows")
    if any(rows.get(k)!=v for k,v in SOURCE_ROWS.items()):raise MissingComponent('fullrail_decision_source_binding_unavailable')
    if 'nix_src_root' not in value:raise MissingComponent('fullrail_decision_compiled_source_root_unavailable')
    for name,digest in SOURCE_ROWS.items():require(hashlib.sha256((Path(value['nix_src_root'])/name).read_bytes()).hexdigest()==digest,"decision source bytes changed")


def front_event(kind,payload,session):
    expected=['attempt','record']+(['observation_id'] if kind=='derivation_observed' else [])
    fields(payload,expected);hex_value(payload['attempt'],16);front_body(payload['record'])
    require(payload['record']['source_session']==session,"front decision event session mismatch")
    if 'observation_id' in payload and payload['observation_id'] is not None:integer(payload['observation_id'],1,(1<<63)-1)


def layout():
    from economic_checkpoint import TABLES_R3, KEYS_R3
    tables={**TABLES_R3,**REGISTRATION_TABLES,"source_decision_observation":(COLUMNS,"observation_id")}
    tables["economic_block"]=(TABLES_R3["economic_block"][0]+" reconstruction_observation","block_hash")
    keys={**KEYS_R3,**{k:tuple(v[1].split()) for k,v in REGISTRATION_TABLES.items()},
          "ark_delivery":("sp_id",),"source_decision_observation":("observation_id",)}
    return tables,keys,SCHEMA,QUERY_ID


def query():
    from storage_queries import _render
    return _render(layout()[0],QUERY_ID,SCHEMA)


def hex_value(value,size=None,maximum=MAX_BODY):
    require(isinstance(value,str) and len(value)<=2*maximum,"decision hex size/type")
    if size is not None:require(len(value)==2*size,'decision fixed hex size')
    return unhex(value,size)


def front_transport(request_raw,ack_raw,index):
    require(isinstance(request_raw,bytes) and len(request_raw)<=MAX_BODY and isinstance(ack_raw,bytes) and len(ack_raw)<=4096,'front transport bound')
    request_value=strict_json(request_raw.decode('utf-8'));fields(request_value,'schema_version session attempt body'.split())
    integer(request_value['schema_version'],1,1);session=array(request_value['session'],16);attempt=array(request_value['attempt'],16);front_body(request_value['body'])
    require(request_value['body']['source_session']==session.hex(),'front transport source session')
    ack=strict_json(ack_raw.decode('utf-8'));fields(ack,['digest','observation_id']);hex_value(ack['digest'],32);integer(ack['observation_id'],1)
    require(ack['digest']==hashlib.sha256(request_raw).hexdigest(),'front ACK exact request bytes')
    observation=index.get(ack['observation_id'],'front_derivation')
    require(sql_bytes(observation['subject'],16)==session and sql_bytes(observation['attempt'],16)==attempt and observation['body']==request_value['body'],'front ACK immutable decision row')
    return {'observation_id':ack['observation_id'],'receipt_field':session.hex()+':'+attempt.hex(),'request_sha256':ack['digest']}


def escape_records(raw):
    """Read the source-owned tracing fields; no local escape implies delivery."""
    require(isinstance(raw,bytes) and len(raw)<=32*1024*1024,'cosign trace byte bound')
    if raw and not raw.endswith(b'\n'):raise MissingComponent('incomplete_cosign_escape_log')
    from pool_source import ANSI
    result=[]
    pattern=r'schema="spc\.cosign-escape/1"\s+order_id=([0-9a-f]{32})\s+nonce=([0-9a-f]{32})\s+observation_id=([1-9][0-9]*)\s+entered_unix_ms=([0-9]+)'
    for line in raw.decode('utf-8').splitlines():
        text=ANSI.sub('',line)
        if 'spc.cosign-escape/' not in text:continue
        matches=list(re.finditer(pattern,text))
        require(len(matches)==1 and 'spc_ark_observation' in text,'unsupported cosign escape trace fields/version')
        require(set(re.findall(r'([A-Za-z_][A-Za-z0-9_]*)=',text))=={'schema','order_id','nonce','observation_id','entered_unix_ms'},'cosign trace contains unreviewed fields')
        m=matches[0]
        result.append({'schema':'spc.cosign-escape/1','order_id':m[1],'nonce':m[2],'observation_id':integer(int(m[3]),1),
            'entered_unix_ms':integer(int(m[4]))})
    return result


def sql_bytes(value,size=None):
    require(isinstance(value,str) and value.startswith('\\x'),"decision SQL bytea")
    return hex_value(value[2:],size)


def snapshot(value):
    require(isinstance(value,str) and re.fullmatch(r'[0-9]+:[0-9]+:(?:[0-9]+(?:,[0-9]+)*)?',value),"decision snapshot encoding")
    low,high,active=value.split(':');low,high=int(low),int(high);active=[] if not active else list(map(int,active.split(',')))
    require(low<=high and active==sorted(set(active)) and all(low<=v<high for v in active),"decision snapshot bounds")
    return low,high,active


def visible(xid,value):
    _,hi,active=snapshot(value);return xid<hi and xid not in active


def timestamp_us(value):
    require(isinstance(value,str) and re.fullmatch(r'\d{4}-\d\d-\d\d[T ]\d\d:\d\d:\d\d(?:\.\d{1,6})?(?:Z|[+-]\d\d:\d\d)',value),"decision DB clock format")
    dt=datetime.fromisoformat(value.replace('Z','+00:00'));delta=dt-datetime(1970,1,1,tzinfo=timezone.utc)
    result=(delta.days*86400+delta.seconds)*1000000+delta.microseconds
    return integer(result)


def request(raw):
    require(len(raw)==69,"SPAR exact size");r=Reader(raw);require(r.take(5)==b'SPAR\1',"SPAR version")
    result={"order_id":r.take(16),"order_digest":r.take(32),"nonce":r.take(16)};r.end()
    require(any(result['nonce']),"SPAR zero nonce");return result


def consumption(raw,public_key=None):
    require(isinstance(raw,bytes) and len(raw)==162,"SPAC exact size")
    r=Reader(raw);require(r.take(5)==b'SPAC\1',"SPAC version");encoded=r.take(149);clock=r.uint(8);r.end()
    grant=grant_bytes(encoded,public_key) if public_key is not None else parse_grant(encoded)[0]
    return {"grant":grant,"grant_bytes":encoded,"actor_unix_ms":clock,"digest":tagged('SPc/ArkConsumptionV1',raw),"signature_verified":public_key is not None}


def consumption_ack(raw):
    require(isinstance(raw,bytes) and len(raw)==53,"SPCA exact size")
    r=Reader(raw);require(r.take(5)==b'SPCA\1',"SPCA version")
    value={"digest":r.take(32),"observation_id":r.uint(8),"DB_unix_ms":r.uint(8)};r.end();integer(value['observation_id'],1)
    return value


def binding(value):
    fields(value,'block_hash height frozen_job payload_digest'.split());hex_value(value['block_hash'],32);hex_value(value['payload_digest'],32);integer(value['height'],0,0xffffffff)
    if value['frozen_job'] is not None:
        from ruleb_source import frozen_job
        # Observation records may retain rejected or synthetic input contexts.
        # The contextual packed-height check belongs to settlement admission.
        frozen_job(value['frozen_job'])


def public_key(value):public_point(hex_value(value,33))


def front_body(body):
    failed=body.get('status')=='failed_unadvertised_attempt'
    expected='schema source_session front_id a_send_x queried_identity_census census_complete status job_id generation height rows'.split()
    if not failed:expected+=['economic_revision','distribution_fingerprint']
    fields(body,expected);require(body['schema']=='spc.front-derivation/1',"front decision schema");hex_value(body['source_session'],16);integer(body['front_id'],0,127)
    if body['a_send_x'] is not None:hex_value(body['a_send_x'],32)
    require(type(body['census_complete']) is bool and body['census_complete'] is not failed,"front census classification")
    if failed:require(body['job_id'] is None and body['generation'] is None,"failed front attempt advertised a job")
    else:
        require(body['status']=='frozen_job',"front status");integer(body['job_id']);integer(body['generation']);hex_value(body['distribution_fingerprint'],32)
        integer(body['height'],0,0xffffffff);hex_value(body['a_send_x'],32)
        if body['economic_revision'] is not None:integer(body['economic_revision'])
    if body['height'] is not None:integer(body['height'],0,0xffffffff)
    rows=body['rows'];require(isinstance(rows,list) and len(rows)<=65536 and body['queried_identity_census']==[r['sp_id'] for r in rows],"front complete ordered census")
    base='sp_id input_scope observed_unix_ms scan_public spend_public coinbase_key_status registration_presence registration server epoch_mailbox mailbox_selection destination_bytes outcome'.split()
    for row in rows:
        extra=['commit_provenance'] if row.get('registration') is not None else []
        if row.get('outcome')=='derivation_error':extra+=['error_stage','error_reason']
        fields(row,base+extra);hex_value(row['sp_id'],32)
        require(row['input_scope']=='actual_front_cache',"front input scope")
        if row['observed_unix_ms'] is not None:integer(row['observed_unix_ms'])
        if row['scan_public'] is not None:
            public_key(row['scan_public']);public_key(row['spend_public'])
            require(row['coinbase_key_status']=='available' and tagged('SPc/Id',hex_value(row['scan_public'])+hex_value(row['spend_public'])).hex()==row['sp_id'],"front identity/key binding")
        else:require(row['spend_public'] is None and row['coinbase_key_status']=='missing_miner_keys',"front missing key classification")
        registration=row['registration'];require(row['registration_presence']==('present' if registration is not None else 'absent_in_front_cache'),"front cache presence")
        if registration is not None:
            fields(registration,'ark_id b_ark mailbox_mode mailbox_m mailbox_d epoch_fallback commit'.split())
            hex_value(registration['ark_id'],4);integer(registration['epoch_fallback'],0,1)
            invalid_input_observation=failed and row.get('outcome')=='derivation_error'
            if invalid_input_observation:hex_value(registration['b_ark'],maximum=65535)
            else:public_key(registration['b_ark'])
            require(registration['mailbox_mode'] in {'none','static','epoch_derived','epoch_registered'},"front mailbox mode")
            for k in ('mailbox_m','mailbox_d'):
                if registration[k] is not None:
                    if invalid_input_observation:hex_value(registration[k],maximum=65535)
                    else:public_key(registration[k])
            commit=registration['commit']
            if commit is not None:
                fields(commit,'request_id registration_rev signed_fields_digest'.split());integer(commit['request_id'],1,0xffffffff);integer(commit['registration_rev'],1)
                if commit['signed_fields_digest'] is not None:hex_value(commit['signed_fields_digest'],32)
            require(row['commit_provenance'] in {'cached','missing_commit_provenance','exact_request_row','missing_request_provenance','request_lookup_failed_unknown'},"front commit provenance")
        server=row['server']
        if server is not None:
            fields(server,'ark_id server_pubkey mailbox_pubkey testnet'.split());hex_value(server['ark_id'],4);public_key(server['server_pubkey']);public_key(server['mailbox_pubkey']);require(type(server['testnet']) is bool,"front server network")
        epoch=row['epoch_mailbox']
        if epoch is not None:fields(epoch,['height','mailbox_mi']);integer(epoch['height'],0,0xffffffff);public_key(epoch['mailbox_mi'])
        require(row['mailbox_selection'] in {None,'none','static','epoch_derived','registered_epoch','hold_missing_epoch','static_fallback'},"front mailbox selection")
        expected_selection=None
        if registration is not None:
            mode=registration['mailbox_mode']
            expected_selection=mode if mode!='epoch_registered' else 'registered_epoch' if epoch is not None else 'hold_missing_epoch' if registration['epoch_fallback']==1 else 'static_fallback'
        require(row['mailbox_selection']==expected_selection,'front epoch/fallback selection mismatch')
        require(row['outcome'] in {'not_attempted','server_metadata_unavailable','derivation_error','derived','epoch_mailbox_held','missing_miner_keys','unregistered_in_front_cache'},"front derivation outcome")
        if row['destination_bytes'] is not None:
            destination=hex_value(row['destination_bytes'],maximum=65535);require(destination and row['outcome']=='derived',"front destination/result");destination.decode('utf-8')
        if row['outcome']=='derived':
            require(row['destination_bytes'] is not None and registration is not None and server is not None and row['scan_public'] is not None,"front derived without inputs")
            require(registration['ark_id']==server['ark_id'] and expected_selection!='hold_missing_epoch','front derived for wrong server/held epoch')
        if row['outcome']=='epoch_mailbox_held':require(expected_selection=='hold_missing_epoch' and row['destination_bytes'] is None,'front held epoch outcome')
        if row['outcome']=='derivation_error':require(row['error_stage'] in {'miner_keys','send_context','ecdh','ark_destination'} and isinstance(row['error_reason'],str) and row['error_reason'],"front error classification")
    return body


def canonical_read(value,*,admit=False):
    fields(value,'schema source_session started_unix_ms finished_unix_ms authoritative tip tip_hash tip_after tip_hash_after members outcome'.split())
    require(value['schema']=='spc.canonical-read/1' and type(value['authoritative']) is bool,"canonical read schema");array(value['source_session'],16)
    for k in ('started_unix_ms','finished_unix_ms','tip','tip_after'):
        if value[k] is not None:integer(value[k],0,(1<<64)-1 if k.endswith('_ms') else 0xffffffff)
    for k in ('tip_hash','tip_hash_after'):
        if value[k] is not None:array(value[k],32)
    require(isinstance(value['members'],list) and len(value['members'])<=2048,"canonical member bound")
    for member in value['members']:
        fields(member,'height expected_hash observed_hash read_status'.split());integer(member['height'],0,0xffffffff);array(member['expected_hash'],32)
        if member['observed_hash'] is not None:array(member['observed_hash'],32)
        require(member['read_status'] in {'not_read','present','unavailable','read_failed'},"canonical member read status")
        require((member['read_status']=='present')==(member['observed_hash'] is not None),"canonical present/absence correlation")
    require(value['outcome'] in {'complete','not_authoritative','tip_read_failed','tip_hash_read_failed','tip_hash_unavailable','member_hash_read_failed','member_hash_unavailable','tip_after_read_failed','tip_hash_after_read_failed','tip_changed','clock_unavailable'},"canonical outcome")
    if admit:
        require(value['outcome']=='complete' and value['authoritative'] and value['started_unix_ms'] is not None and value['finished_unix_ms'] is not None
            and value['started_unix_ms']<=value['finished_unix_ms'] and value['tip'] is not None and value['tip_hash'] is not None
            and value['tip']==value['tip_after'] and value['tip_hash']==value['tip_hash_after']
            and all(m['read_status']=='present' for m in value['members']),"incomplete/reversed canonical read claims complete")
    return value


def sql_member(row):
    fields(row,'order_id ordinal block_hash sp_id block_height due_height sats reserved orphaned'.split())
    sql_bytes(row['order_id'],16)
    for k in ('block_hash','sp_id'):sql_bytes(row[k],32)
    integer(row['ordinal'],0,2047);integer(row['block_height'],0,0xffffffff);integer(row['due_height'],row['block_height'],0xffffffff);integer(row['sats'],1)
    require(type(row['reserved']) is bool and type(row['orphaned']) is bool,"member boolean")


def authorization_row(row):
    fields(row,'order_id nonce revision order_digest checked_tip checked_tip_hash expires_at grant_bytes created_at'.split())
    for k in ('order_id','nonce'):sql_bytes(row[k],16)
    for k in ('order_digest','checked_tip_hash'):sql_bytes(row[k],32)
    integer(row['checked_tip'],0,0xffffffff);integer(row['revision'],1);integer(row['expires_at'],1);timestamp_us(row['created_at'])
    grant=parse_grant(sql_bytes(row['grant_bytes'],149))[0]
    require(all(grant[k]==sql_bytes(row[k]) for k in ('order_id','nonce','order_digest')) and grant['revision']==row['revision'] and grant['expires_at']==row['expires_at'],"full authorization row/grant mismatch")


def parse_body(kind,body):
    require(isinstance(body,dict) and len(canonical_json(body))<=MAX_BODY,"decision body size/object")
    if kind=='front_derivation':return front_body(body)
    if kind=='settlement_delivery_read':
        extra=['error_class'] if body.get('status')=='unknown' else []
        fields(body,'schema binding queried status results'.split()+extra);require(body['schema']=='spc.settlement-delivery-read/1',"delivery schema");binding(body['binding'])
        require(isinstance(body['queried'],list) and len(body['queried'])<=65536,"delivery census bound")
        for identity in body['queried']:hex_value(identity,32)
        if body['status']=='unknown':require(body['results'] is None and body['error_class']=='read_or_persistence_failed',"unknown delivery is not absence");return body
        require(body['status']=='complete' and isinstance(body['results'],list) and [r['sp_id'] for r in body['results']]==body['queried'],"delivery result ordered census")
        from storage_queries import TABLES
        specs={'delivery':TABLES['ark_delivery'][0],'registration_state':REGISTRATION_TABLES['ark_registration_state'][0],
               'registration_request':REGISTRATION_TABLES['ark_registration_request'][0],'epoch_mailbox':REGISTRATION_TABLES['ark_epoch_mailbox'][0]}
        for result in body['results']:
            fields(result,['sp_id',*specs]);identity=hex_value(result['sp_id'],32)
            for name,columns in specs.items():
                row=result[name]
                if row is None:continue
                fields(row,columns.split());require(sql_bytes(row['sp_id'],32)==identity,"delivery nested identity")
                for key in ('b_ark','mailbox_m','mailbox_d','mailbox_mi'):
                    if key in row and row[key] is not None:public_point(sql_bytes(row[key],33))
                if 'signed_fields_digest' in row:sql_bytes(row['signed_fields_digest'],32)
                for key in ('created_at','updated_at'):
                    if key in row:timestamp_us(row[key])
                if name=='delivery':
                    sql_bytes(row['ark_id'],4);integer(row['epoch_fallback'],0,1)
                    require(row['mailbox_mode'] in {'none','static','epoch_derived','epoch_registered'},"delivery mailbox mode")
                if name=='registration_request':
                    integer(row['request_id'],1,0xffffffff);integer(row['registration_rev'],1)
                    require(row['outcome'] in {'applied','superseded','mismatch','stale','rejected'} and (row['error_code'] is None or isinstance(row['error_code'],str)),"registration request outcome")
            state,request_row=result['registration_state'],result['registration_request']
            require(state is not None or request_row is None,"registration request without joined state")
            if state is not None:
                integer(state['last_request_id'],1,0xffffffff);integer(state['registration_rev'],1)
                if request_row is not None:require(request_row['request_id']==state['last_request_id'] and request_row['registration_rev']==state['registration_rev'] and request_row['signed_fields_digest']==state['signed_fields_digest'],"delivery committed request/revision")
            if result['epoch_mailbox'] is not None:require(result['epoch_mailbox']['height']==body['binding']['height'],"delivery epoch height")
        return body
    if kind=='settlement_reconstruction':
        extra=['ledger','pending_deltas'] if body.get('status')=='complete' else ['error']
        fields(body,'schema binding delivery_read_observation status'.split()+extra);require(body['schema']=='spc.settlement-reconstruction/1',"reconstruction schema");binding(body['binding']);integer(body['delivery_read_observation'],1)
        require(body['status'] in {'complete','unknown'},"reconstruction status")
        if body['status']=='unknown':require(isinstance(body['error'],str),"reconstruction error");return body
        require(isinstance(body['ledger'],list) and len(body['ledger'])<=65536,"reconstruction ledger bound")
        expected=[];seen=set()
        for row in body['ledger']:
            fields(row,'sp_id rail paid_sats ark_due_sats forfeited_sats pending_delta'.split());identity=array(row['sp_id'],32);require(identity not in seen,"reconstruction duplicate identity");seen.add(identity)
            require(row['rail'] in {'Coinbase','Ark','Pending','Forfeit'},"reconstruction rail")
            for k in ('paid_sats','ark_due_sats','forfeited_sats'):integer(row[k])
            integer(row['pending_delta'],-(1<<63),(1<<63)-1)
            if row['pending_delta']:expected.append([row['sp_id'],row['pending_delta']])
        require(body['pending_deltas']==expected,"reconstruction exact pending deltas")
        return body
    if kind=='authorization_row':
        fields(body,'schema operation before after'.split());require(body['schema']=='spc.authorization-row/1' and body['operation'] in {'INSERT','UPDATE','DELETE'},"authorization history operation")
        require((body['before'] is None)==(body['operation']=='INSERT') and (body['after'] is None)==(body['operation']=='DELETE'),"authorization history null sides")
        for row in (body['before'],body['after']):
            if row is not None:authorization_row(row)
        return body
    if kind=='authorization_consumption':
        extra=[]
        if body.get('status')=='unknown':
            extra=['reason']+(['observer_unix_ms'] if body.get('reason')=='grant_mismatch_or_expired' else [])
        fields(body,'schema status request_bytes request_digest grant_bytes consumed_at_unix_ms meaning'.split()+extra)
        require(body['schema']=='spc.authorization-consumption/1' and body['status'] in {'recorded','unknown'} and body['meaning']=='lease_consumed_before_possible_cosign_not_delivery',"consumption schema/meaning")
        value=consumption(hex_value(body['request_bytes'],162))
        require(value['digest']==hex_value(body['request_digest'],32) and value['grant_bytes']==hex_value(body['grant_bytes'],149) and value['actor_unix_ms']==integer(body['consumed_at_unix_ms']),"consumption body/wire mismatch")
        if extra:
            require(body['reason'] in {'grant_row_absent','grant_mismatch_or_expired'},"consumption unknown reason")
            if 'observer_unix_ms' in body:integer(body['observer_unix_ms'],-(1<<63),(1<<63)-1)
        return body
    require(kind=='authorization_decision',"unknown decision kind")
    outcome=body.get('outcome');small=outcome in {'canonical_read_unknown','issuance_unknown'}
    expected='schema request_bytes now canonical_read outcome grant_bytes'.split()
    if small:
        if outcome=='issuance_unknown':expected+=['reason']
    else:
        expected+='request_nonce checked_tip checked_tip_hash canonical_set sql_members'.split()
        if outcome in {'issued','members_not_canonical_or_reserved'}:expected+=['queried_members']
        if outcome=='issued':expected+=['aggregate_revision']
    fields(body,expected);require(body['schema']=='spc.authorization-decision/1' and outcome in {'issued','canonical_read_unknown','aggregate_absent','request_not_issuable','members_not_canonical_or_reserved','issuance_unknown'},"authorization decision schema/outcome")
    req=request(hex_value(body['request_bytes'],69));integer(body['now'])
    if body['canonical_read'] is not None:canonical_read(body['canonical_read'])
    if not small:
        require(req['nonce']==hex_value(body['request_nonce'],16),"authorization nonce");integer(body['checked_tip'],0,0xffffffff);hex_value(body['checked_tip_hash'],32)
        require(isinstance(body['canonical_set'],list) and body['canonical_set']==sorted(set(body['canonical_set'])),"canonical set ordering")
        for value in body['canonical_set']:hex_value(value,32)
        for member in body.get('queried_members',[]):sql_member(member)
        if body['sql_members'] is not None:
            for value in body['sql_members']:
                fields(value,'member block_status payout_status payout_sats'.split());sql_member(value['member']);integer(value['payout_sats'])
                require(value['block_status'] in {'pending','confirmed','orphaned'} and value['payout_status'] in {'scheduled','sent','failed_to_pending'},"authorization joined SQL statuses")
    if outcome=='issued':
        grant=parse_grant(hex_value(body['grant_bytes'],149))[0]
        require(all(grant[k]==req[k] for k in req) and grant['expires_at']==body['now']+10 and grant['revision']==integer(body['aggregate_revision'],1),"issued grant/request/lease/revision")
    else:require(body['grant_bytes'] is None,"nonissued observation claims grant")
    return body


def row(value):
    fields(value,COLUMNS.split());integer(value['observation_id'],1);integer(value['schema_version'],1,1);integer(value['transaction_id'],1)
    require(value['kind'] in KINDS,"decision kind");subject=sql_bytes(value['subject']);require(len(subject) in (16,32),"decision subject length")
    if value['attempt'] is not None:sql_bytes(value['attempt'],16)
    snapshot(value['snapshot_boundary']);timestamp_us(value['observed_at']);parse_body(value['kind'],value['body'])
    body=value['body'];kind=value['kind']
    if kind.startswith('settlement_'):require(subject==hex_value(body['binding']['block_hash'],32) and value['attempt'] is not None,"settlement subject/attempt")
    elif kind=='front_derivation':require(subject==hex_value(body['source_session'],16) and value['attempt'] is not None,"front subject/attempt")
    elif kind=='authorization_decision':require(subject==request(hex_value(body['request_bytes'],69))['order_id'] and value['attempt'] is None,"authorization subject/attempt")
    elif kind=='authorization_row':require(value['attempt'] is None and all(r is None or sql_bytes(r['order_id'],16)==subject for r in (body['before'],body['after'])),"authorization row subject")
    else:
        grant=consumption(hex_value(body['request_bytes'],162))['grant']
        require(subject==grant['order_id'] and (value['attempt'] is None if body['status']=='unknown' else sql_bytes(value['attempt'],16)==grant['nonce']),"consumption subject/attempt")
    return value


class Index:
    def __init__(self,observation):
        self.observation=observation;self.rows={};self.events={};attempts=set()
        if observation.get('schema')=='spc.fullrail-test-observation-capture/1':
            fields(observation,'schema history database snapshot read_only captured_at observations economic_blocks authorization_working_rows'.split())
            require(observation['read_only']=='on','executed fixture export is not read-only');timestamp_us(observation['captured_at'])
            require(isinstance(observation['database'],str) and 0<len(observation['database'].encode())<=63,'executed fixture database name')
            snapshot(observation['snapshot'])
            history=observation['history'];values=observation['observations'];visibility=observation['snapshot'];self.authorization_rows=observation['authorization_working_rows']
            self.scope='executed_test_SQL_export_not_funded_or_complete_raw_history'
        else:
            history=observation['tables']['source_accounting_history'];values=observation['tables']['source_decision_observation']
            visibility=observation['server']['snapshot'];self.authorization_rows=observation['tables']['ark_send_authorization'];self.scope='bound_original_SQL_observation'
        for value in values:
            row(value);oid=value['observation_id'];require(oid not in self.rows,"duplicate decision id")
            require(visible(value['transaction_id'],visibility),"decision transaction not visible in capture")
            if value['attempt'] is not None:
                key=(value['kind'],value['subject'],value['attempt']);require(key not in attempts,"duplicate decision attempt");attempts.add(key)
            self.rows[oid]=value
        for event in history:
            if event['table_name']!='source_decision_observation':continue
            require(event['operation']=='INSERT' and event['before_row'] is None,"decision history rewritten/deleted")
            value=event['after_row'];oid=value['observation_id']
            require(oid in self.rows and oid not in self.events and canonical_json(value)==canonical_json(self.rows[oid])
                and value['transaction_id']==event['transaction_id'],"decision exact same-transaction INSERT binding")
            self.events[oid]=event
        require(set(self.rows)==set(self.events),"decision history/capture census incomplete")
        self.history=history

    def get(self,oid,kind):
        if oid is None or oid not in self.rows:raise MissingComponent('actual_'+kind+'_observation_reference')
        value=self.rows[oid];require(value['kind']==kind,"decision reference kind mismatch");return value

    def settlement(self,stored,wire):
        from ruleb_source import frozen_job
        frozen_job(stored['frozen_job'],height=stored['height'])
        recon=self.get(stored.get('reconstruction_observation'),'settlement_reconstruction');body=recon['body']
        if body['status']!='complete':raise MissingComponent('settlement_reconstruction_unknown')
        read=self.get(body['delivery_read_observation'],'settlement_delivery_read');decision=read['body']
        if decision['status']!='complete':raise MissingComponent('settlement_delivery_read_unknown_not_absence')
        expected={'block_hash':sql_bytes(stored['block_hash'],32).hex(),'height':stored['height'],
            'frozen_job':stored['frozen_job'],'payload_digest':sql_bytes(stored['payload_digest'],32).hex()}
        require(body['binding']==decision['binding']==expected and body['ledger']==stored['validated_ledger'],"settlement exact block/frozen/ledger decision")
        require(read['attempt']==recon['attempt'] and read['observation_id']<recon['observation_id']
            and visible(read['transaction_id'],recon['snapshot_boundary']),"delivery read not visible before reconstruction")
        require(decision['queried']==[e['sp_id'] for e in wire['entries']],"settlement decision/SPCB complete ordered census")
        accepts=[e for e in self.history if e['table_name']=='economic_block' and e['operation']=='INSERT' and e['after_row']['block_hash']==stored['block_hash']]
        require(len(accepts)==1 and accepts[0]['after_row']['reconstruction_observation']==recon['observation_id']
            and self.events[recon['observation_id']]['event_id']<accepts[0]['event_id'],"economic acceptance lacks original reconstruction reference/order")
        return {'delivered':sorted({r['sp_id'] for r in decision['results'] if r['delivery'] is not None}),
            'read':read['observation_id'],'reconstruction':recon['observation_id'],'results':decision['results']}

    def front(self,source,key):
        frozen=source.jobs[key];p=frozen['payload'];matches=[]
        for event in source.events:
            if event['kind']!='derivation_observed':continue
            value=event['payload'];b=value['record']
            if b['status']=='frozen_job' and b['source_session']==key[0] and b['job_id']==p['economic']['job_id'] and b['generation']==key[3]:matches.append(event)
        if not matches:raise MissingComponent('actual_frozen_front_derivation_observation')
        require(len(matches)==1,"ambiguous frozen front derivation")
        event=matches[0];v=event['payload'];observation=self.get(v['observation_id'],'front_derivation');body=observation['body']
        require(body==v['record'] and sql_bytes(observation['attempt'],16).hex()==v['attempt'] and sql_bytes(observation['subject'],16).hex()==key[0],"front event/SQL exact attempt/body")
        prepared=[e for e in source.events if e['kind']=='derivation_prepared' and e['source']['session']==key[0] and e['payload']['attempt']==v['attempt']]
        require(len(prepared)==1 and prepared[0]['payload']['record']==body
            and int(prepared[0]['source']['seq'])<int(event['source']['seq'])<int(frozen['source']['seq']),"front persistence receipt/freeze ordering")
        require(body['height']==p['height'] and body['front_id']==p['front_id'] and body['economic_revision']==p['economic']['revision']
            and body['distribution_fingerprint']==p['fingerprint'] and body['a_send_x']==p['a_send_x'],"front frozen context mismatch")
        rows={r['sp_id']:r for r in body['rows']};require(len(rows)==len(body['rows']) and {e['sp_id'] for e in p['entries']}<=set(rows),"front credited identity census")
        missing=[];commits=[]
        for entry in p['entries']:
            r=rows[entry['sp_id']];destination=r['destination_bytes']
            require(entry['destination_digest']==(hashlib.sha256(hex_value(destination)).hexdigest() if destination is not None else None),"front frozen destination differs from actual derivation")
            registration=r['registration']
            if registration is not None:
                commit=registration['commit']
                if commit is None or commit['signed_fields_digest'] is None:
                    missing.append({'sp_id':entry['sp_id'],'field':'front_registration_commit_provenance_unavailable'});continue
                matched=[]
                for historical in self.history:
                    if historical['table_name']!='ark_registration_request' or not visible(historical['transaction_id'],observation['snapshot_boundary']):continue
                    for image in (historical['before_row'],historical['after_row']):
                        if image is not None and image['sp_id']=='\\x'+entry['sp_id'] and image['request_id']==commit['request_id'] and image['registration_rev']==commit['registration_rev']:
                            require(image['signed_fields_digest']=='\\x'+commit['signed_fields_digest'],'front registration commit digest conflicts with historical request')
                            if image['outcome'] in {'applied','superseded'}:matched.append(historical['event_id'])
                if not matched:missing.append({'sp_id':entry['sp_id'],'field':'immutable_registration_request_visible_at_front_observation'})
                else:commits.append({'sp_id':entry['sp_id'],'request_id':commit['request_id'],'registration_rev':commit['registration_rev'],'history_events':sorted(set(matched))})
        return {'observation_id':observation['observation_id'],'rows':rows,'source_receipt_sequence':event['source']['seq'],
            'registration_commits':commits,'missing':missing,'scope':'actual_front_cache_and_durable_ACK_order_not_secret_tweak_proof'}

    def authorization(self,order_id,public_key,views):
        oid=sql_bytes(order_id,16);issued=[]
        for value in self.rows.values():
            if value['kind']!='authorization_decision' or sql_bytes(value['subject'])!=oid or value['body']['outcome']!='issued':continue
            body=value['body'];grant=grant_bytes(hex_value(body['grant_bytes'],149),public_key);read=body['canonical_read']
            if read is None:raise MissingComponent('actual_grant_canonical_read_not_legacy_null')
            canonical_read(read,admit=True)
            members=body['queried_members'];joined=body['sql_members']
            require(members and joined is not None and [r['member'] for r in joined]==members,"grant complete SQL member census/join")
            require([(m['height'],bytes(m['expected_hash']).hex()) for m in read['members']]==[(m['block_height'],sql_bytes(m['block_hash'],32).hex()) for m in members],"grant canonical/SQL member census")
            actual=sorted({bytes(m['expected_hash']).hex() for m in read['members'] if m['expected_hash']==m['observed_hash']})
            require(body['canonical_set']==actual and all(sql_bytes(m['block_hash']).hex() in actual and m['order_id']==order_id and m['reserved'] for m in members),"grant canonical set/reservation")
            require(all(r['block_status']=='confirmed' and r['payout_status']=='scheduled' and r['payout_sats']==r['member']['sats'] for r in joined),"grant nonpayable SQL member")
            require(body['checked_tip']==read['tip'] and body['checked_tip_hash']==bytes(read['tip_hash']).hex()
                and read['finished_unix_ms']//1000<=body['now'],"grant canonical tip/issuance clock")
            tip=bytes(read['tip_hash'])[::-1].hex();candidates=[v for v in views.values() if tip in v.by_hash]
            if not candidates:raise MissingComponent('grant_raw_canonical_tip_ancestry')
            view=candidates[0].prefix(read['tip']);require(view.tip['hash']==tip,"grant raw tip height")
            require(all(m['height']<=view.tip['height'] and bytes(m['observed_hash'])[::-1].hex()==view.blocks[m['height']]['hash'] for m in read['members']),"grant observed hashes differ from raw chain")
            row_events=[r for r in self.rows.values() if r['kind']=='authorization_row' and r['transaction_id']==value['transaction_id'] and r['body']['operation']=='INSERT' and r['subject']==order_id]
            require(len(row_events)==1 and row_events[0]['body']['after']['grant_bytes']=='\\x'+body['grant_bytes'],"grant not atomic with complete authorization row")
            transitions=[e for e in self.history if e['table_name']=='ark_aggregate' and e['transaction_id']==value['transaction_id'] and e['operation']=='UPDATE' and e['after_row']['order_id']==order_id and e['after_row']['state']=='authorized']
            require(len(transitions)==1 and transitions[0]['before_row']['state']=='published' and transitions[0]['after_row']['revision']==grant['revision']
                and transitions[0]['before_row']['revision']+1==grant['revision'] and sum(m['sats'] for m in members)==transitions[0]['after_row']['sats'],"grant atomic aggregate revision/amount")
            issued.append({'observation_id':value['observation_id'],'grant':grant,'raw':hex_value(body['grant_bytes'],149),'canonical_read':read})
        return issued

    def authorization_history(self):
        state={};retained=[]
        for value in sorted(self.rows.values(),key=lambda r:r['observation_id']):
            if value['kind']!='authorization_row':continue
            body=value['body'];oid=value['subject'];before,after=body['before'],body['after']
            if before is not None:
                if oid not in state:raise MissingComponent('pre_migration_authorization_history_unknown')
                require(state.pop(oid)==before,"authorization history before-image mismatch")
            if after is not None:
                require(oid not in state,"authorization duplicate working row");state[oid]=after
            retained.append(value)
        current={r['order_id']:r for r in self.authorization_rows}
        if state!=current:raise MissingComponent('authorization_history_does_not_cover_working_rows')
        return retained

    def consume(self,observation_id,ack_raw,public_key,escape=None):
        value=self.get(observation_id,'authorization_consumption');body=value['body']
        parsed=consumption(hex_value(body['request_bytes'],162),public_key)
        if body['status']!='recorded':raise MissingComponent('unknown_consumption_keeps_existing_order_pinned')
        if ack_raw is None:raise MissingComponent('actual_SPCA_receipt_unavailable_order_stays_pinned')
        ack=consumption_ack(ack_raw);db_ms=(timestamp_us(value['observed_at'])+500)//1000
        require(ack['digest']==parsed['digest'] and ack['observation_id']==observation_id and ack['DB_unix_ms']==db_ms,"consumption exact DB receipt correlation")
        grant=parsed['grant'];low=grant['expires_at']-10;high=grant['expires_at']
        if not (low<=parsed['actor_unix_ms']//1000<high and low<=db_ms//1000<high):raise MissingComponent('expired_consumption_ack_keeps_order_pinned')
        issued=[r for r in self.rows.values() if r['kind']=='authorization_decision' and r['body']['outcome']=='issued' and r['body']['grant_bytes']==body['grant_bytes']]
        require(len(issued)==1 and visible(issued[0]['transaction_id'],value['snapshot_boundary']),"consumption lacks preceding exact issued grant")
        result={'observation_id':observation_id,'request_digest':parsed['digest'].hex(),'grant_revision':grant['revision'],'DB_unix_ms':db_ms,
            'actor_unix_ms':parsed['actor_unix_ms'],'wallet_received':False,'possible_cosign_escape':False,'no_refund_or_new_grant_authority':True}
        if escape is not None:
            fields(escape,'schema order_id nonce observation_id entered_unix_ms'.split())
            require(escape['schema']=='spc.cosign-escape/1' and hex_value(escape['order_id'],16)==grant['order_id'] and hex_value(escape['nonce'],16)==grant['nonce']
                and escape['observation_id']==observation_id and low<=integer(escape['entered_unix_ms'])//1000<high,"cosign escape grant/observation/lease")
            require(escape['entered_unix_ms']>=parsed['actor_unix_ms'],"actor consumption/escape clock reversed")
            result['possible_cosign_escape']=True
        return result
