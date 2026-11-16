"""Verify original controller RPC tip samples before Runtime phase submission.

This consumes earlier observations, not ancestry obtained after mining. The
reviewed Runtime emits operator-readiness records synchronously before starting
its controller HTTP server and calling execute_phases. That same-stream marker
orders the initial node samples without comparing unrelated monotonic clocks.
"""
import hashlib
import json
import re

from oracle import integer, require, strict_json
from source_jobs import MissingComponent
from runtime_bootstrap import RuntimeBootstrap
from traffic_audit import read_commands

RUNTIME_SHA256='78c81ae25b7794d93ab88a1842ab2fe45a5e2a3c3e0c1ac978944cedea8283e0'
TIP_DETAIL='tip sampler: atomic getbestblockhash/getblockheader/getblockhash observation; not full canonical coverage or reorg tracking'
EVENT_FIELDS=set('run_id event_id source source_cursor actor command_id occurred_at_wall occurred_at_mono ingested_at payload'.split())


def verify_prephase_genesis(evidence_raw,commands_raw,scenario,genesis,source_binding):
    require(source_binding.get('profile_id')=='qualified-98f-opaque-cleanup'
        and source_binding.get('runtime_source_sha256')==RUNTIME_SHA256,
        'prephase genesis source profile not reviewed')
    require(evidence_raw and evidence_raw.endswith(b'\n') and len(evidence_raw)<=128*1024*1024,
        'prephase genesis complete bounded evidence prefix')
    nodes=[n['id'] for n in scenario['nodes']];fronts=[f['id'] for f in scenario['fronts']]
    require(len(nodes)==2 and len(set(nodes))==2 and fronts and len(set(fronts))==len(fronts),'prephase genesis node/front census')
    events=[]
    for index,line in enumerate(evidence_raw.splitlines(),1):
        row=strict_json(line)
        require(isinstance(row,dict) and set(row)==EVENT_FIELDS and type(row['event_id']) is int
            and row['event_id']==index and row['run_id']==scenario['run']['id'],'prephase genesis event prefix/run binding')
        require(isinstance(row['payload'],dict) and set(row['payload'])=={'kind','data'},'prephase genesis payload fields')
        events.append(row)
    markers={}
    for row in events:
        payload=row['payload']
        if payload['kind']!='unknown' or not isinstance(payload['data'],dict) or payload['data'].get('topic')!='operator_observation':continue
        require(row['source']=='operator' and row['actor']=={'kind':'observer','id':'runtime-operator'}
            and row['command_id'] is None,'prephase Runtime marker origin')
        cursor=re.fullmatch(r'operator-readiness-(\d+)',row['source_cursor'])
        require(cursor is not None and int(cursor[1])<row['event_id'],'prephase Runtime marker cursor')
        detail=strict_json(payload['data']['detail'])
        require(set(detail)=={'front','listen','loopback','page_served','observation_route'}
            and detail['front'] in fronts and detail['front'] not in markers and detail['loopback'] is True,
            'prephase Runtime marker front census')
        require(isinstance(detail['listen'],str) and re.fullmatch(r'(127\.0\.0\.1|\[::1\]):[0-9]{1,5}',detail['listen']) is not None
            and type(detail['page_served']) is bool and (detail['observation_route'] is None or type(detail['observation_route']) is bool),
            'prephase Runtime marker listener/result shape')
        markers[detail['front']]=row['event_id']
    if set(markers)!=set(fronts):raise MissingComponent('complete_prephase_Runtime_markers_unavailable')
    boundary=max(markers.values());samples={}
    for row in events[:boundary]:
        if row['payload']['kind']=='event_lost':raise MissingComponent('prephase_controller_evidence_gap')
        if row['payload']['kind']!='canonical_at':continue
        data=row['payload']['data']
        if not isinstance(data,dict) or data.get('node_id') not in nodes:continue
        node=data['node_id']
        require(row['source']=='node-'+node and row['actor']=={'kind':'node','id':node} and row['command_id'] is None,
            'prephase genesis node observation origin')
        require(set(data)==set('node_id block_hash height anchored_tip observed_at source result detail'.split())
            and data['source']==row['source'] and data['detail']==TIP_DETAIL,'prephase genesis actual RPC sampler shape')
        require(type(data['height']) is int and data['height']==0 and data['block_hash']==genesis
            and data['anchored_tip']==genesis and data['result']=='yes'
            and row['source_cursor']=='canonical-0-'+genesis,'prephase node is not the expected genesis tip')
        integer(data['observed_at']);integer(row['occurred_at_mono'])
        require(node not in samples,'duplicate prephase genesis node observation')
        samples[node]={'event_id':row['event_id'],'height':0,'hash':genesis,'observed_at':data['observed_at']}
    if set(samples)!=set(nodes):raise MissingComponent('both_nodes_prephase_genesis_observations_unavailable')
    bootstrap=RuntimeBootstrap(scenario).observe(commands_raw);rows=read_commands(commands_raw)
    start=next(r for r in rows if r['record']=='hash_start')
    authority=bootstrap['begin']['command']['kind']['node_id'];attempts={};reads=[]
    expected=hashlib.sha256(json.dumps({'outcome':'ok','result':genesis},separators=(',',':')).encode()).hexdigest()
    params=hashlib.sha256(b'[]').hexdigest();sessions={}
    for row in rows[:start['seq']-1]:
        if row['record']!='traffic_audit':continue
        require(row['boot_id']==start['operation']['boot_id'],'pre-HashStart RPC boot mismatch')
        event=row['event']
        if event['event']=='session':sessions[event['binding']['incarnation']]=event['binding']
        if event['event']=='forward_attempt':attempts[row['seq']]=event['attempt']
        if event['event']!='forward_result' or event['attempt_seq'] not in attempts:continue
        attempt=attempts[event['attempt_seq']]
        if attempt['method']!='getbestblockhash' or attempt['node_id']!=authority:continue
        session=sessions.get(attempt['incarnation'])
        require(session is not None and session['run_id']==scenario['run']['id']
            and session['controller_sha256']==source_binding['binary_sha256']
            and session['actor_sha256']==source_binding['txd_sha256']
            and session['driver_id'] in {a['id'] for a in scenario['traffic']['driver']['actors']}, 'pre-HashStart RPC controller/run binding')
        require(attempt['send_sequence'] is None and attempt['payload_sha256']==params
            and event['outcome']=='ok' and event['response_suppressed'] is False
            and event['response_sha256']==expected,'pre-HashStart authority node did not return genesis tip')
        reads.append({'attempt_seq':event['attempt_seq'],'result_seq':row['seq'],'node_id':authority,'response_sha256':expected})
    if not reads:raise MissingComponent('pre_HashStart_genesis_tip_read_commitment_unavailable')
    return {'height':0,'hash':genesis,'run_id':scenario['run']['id'],'basis':'source_bound_prephase_RPC_tip_observations',
        'nodes':samples,'prephase_marker_event_ids':markers,'bootstrap_operation_id':bootstrap['operation_id'],
        'bootstrap_begin_seq':bootstrap['begin']['seq'],'hash_start_seq':start['seq'],'pre_hash_start_reads':reads,
        'evidence_prefix_sha256':hashlib.sha256(evidence_raw).hexdigest(),'command_prefix_sha256':hashlib.sha256(commands_raw).hexdigest(),
        'scope':'initial_genesis_observations_only; not later canonical coverage or funded acceptance'}
