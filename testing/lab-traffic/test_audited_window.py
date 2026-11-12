"""Signed source/SQL/window vectors, not live native or compiled-wallet proof."""
import copy
import contextlib
from functools import lru_cache
import hashlib
import json
import os
from pathlib import Path
import tempfile
import socket
import threading
import time
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from audited_window import prove_eligible,rewards
from case_contract import canonical_json
from oracle import Invalid,Inconclusive,decode_transaction,merkle,sha256d,target,Chain
from pool_source import PoolSource,trace_digest,leaf_hash
from source_capture import SourceCapture
from source_feed_provider import ReplayProvider
from source_feed_codec import CONTRACT,SCHEMA,window_facts,trace_digest as seal_digest,_sign_digest,tagged
from source_jobs import verify_job
from test_oracle import coinbase,block_bytes,policy
from test_economic_checkpoint import captured
from signature_oracle import _multiply
from public_derivation import _GX,_GY,_N,_point_add
from test_source_feed import pub

BASE=None
PROFILE={"reference_target":"7f"+"ff"*31,"window_n":1}
FINGERPRINT=hashlib.sha256(b"spc-lab/work-unit/v2\0"+bytes.fromhex(PROFILE["reference_target"])).hexdigest()


def compressed(point):return bytes([2+(point[1]&1)])+point[0].to_bytes(32,"big")


@lru_cache(maxsize=1)
def identity_keys():
    scan=compressed(_multiply(3,(_GX,_GY)));spend=_multiply(4,(_GX,_GY));sender=compressed((_GX,_GY))
    identity=tagged("SPc/Id",scan+compressed(spend)).hex()
    ih=int.from_bytes(tagged("SPc/Inputs",(1).to_bytes(4,"big")+sender),"big");assert 0<ih<_N
    shared=compressed(_multiply(ih*3,(_GX,_GY)))
    keys=[]
    for k in range(990):
        scalar=int.from_bytes(tagged("SPc/TierSpc",shared+k.to_bytes(4,"big")),"big");assert 0<scalar<_N
        point=_point_add(spend,_multiply(scalar,(_GX,_GY)));keys.append(point[0].to_bytes(32,"big").hex())
    return identity,tuple(keys)


def encode_events(events):
    lines=[];previous="00"*32
    for seq,(kind,payload) in enumerate(events,1):
        source={"schema":"spc.source-evidence/2","session":"01"*16,"seq":str(seq),"previous":previous,"complete_prefix_required":True}
        source["digest"]=trace_digest(kind,payload,source);previous=source["digest"]
        seal={"session":[1]*16,"seq":seq,"digest":list(bytes.fromhex(previous)),"signature":""}
        seal["signature"]=_sign_digest(7,seal_digest(seal));source["pool_seal"]=seal
        lines.append(('INFO spc_evidence: event="'+kind+'" evidence='+json.dumps({**payload,"source":source},separators=(",",":"))+"\n").encode())
    return b"".join(lines)


def parse(events):
    raw=encode_events(events)
    return PoolSource(raw,independently_captured_log_sha256=hashlib.sha256(raw).hexdigest(),source_contract=CONTRACT,pool_pubkey=pub(7))


@lru_cache(maxsize=3)
def base_fixture(count):
    observation,_,old_chain,migrations=captured()
    genesis=old_chain.blocks[0];raw=genesis["raw"]+b"\1"+genesis["transactions"][0]["raw"]
    p=policy(genesis=genesis["hash"],initial_target_bits=0x200fffff,regtest_pow_bits_overlay=0x200fffff)
    chain=Chain(p,[raw])
    h=observation["tables"]["source_accounting_history"][:4]
    observation["tables"]={name:[] for name in observation["tables"]}
    observation["tables"]["source_accounting_history"]=h
    observation["tables"]["economic_checkpoint"]=[copy.deepcopy(h[1]["after_row"])]
    observation["tables"]["economic_state"]=[copy.deepcopy(h[2]["after_row"])]
    observation["tables"]["economic_chain"]=[copy.deepcopy(h[3]["after_row"])]
    observation["counts"]={name:len(rows) for name,rows in observation["tables"].items()}
    observation["history_boundary"]["event_watermark"]=h[-1]["event_id"]
    identity,keys=identity_keys();sender=(_GX).to_bytes(32,"big").hex();poolscript=b"\x51\x20"+bytes.fromhex(pub(8))
    tag=b"\x51\x20"+bytes.fromhex(sender)+bytes(2)
    def job(number,slices):
        outputs=[(5000000000,poolscript)] if not slices else [(50000000,poolscript)]+[(5000000,bytes.fromhex("5120"+k)) for k in keys]
        cb=coinbase(outputs=outputs,tag=tag);split=42+len(tag)-2
        balances=[] if not slices else [{"sp_id":list(bytes.fromhex(identity)),"booked":0,"reserved":0,"available":0}]
        economic={"version":1,"revision":1,"parent_hash":list(bytes.fromhex(genesis["hash"])[::-1]),"job_id":(1<<32)|number,
                  "generation":number,"balances":balances,"slice_ids":[int(r["slice_id"]) for r in slices]}
        payload={"front_id":0,"job_id":f"{number:08x}","generation":str(number),"height":1,"version":1,"ntime":1600000001,
            "prev_hash":bytes.fromhex(genesis["hash"])[::-1].hex(),"nbits":0x200fffff,"coinbase_prefix":cb[:split].hex(),"coinbase_suffix":cb[split+2:].hex(),
            "extranonce_len":2,"merkle_branch":[],"a_send_x":sender,"share_target":"7f"+"ff"*31,"share_target_byte_order":"big-endian unsigned 256-bit",
            "m_issue":1,"work_unit_version":2,"work_unit_fingerprint":FINGERPRINT,"transactions":[],"witness_plan":None,"fees":"0","subsidy":"5000000000","fee_bps":100,
            "window_target":"9","window_rule":1,"window_underfilled":len(slices)<9,"slices":slices,"high_water":{"0":slices[0]["slice_id"] if slices else "0"},
            "finding_slice_id":str(count+1 if slices else 1),"finding_slice_excluded":True,"economic":economic,
            "entries":[] if not slices else [{"format_version":2,"sp_id":identity,"rail":0,"p_x":list(keys),"sats":"4950000000","owed_sats":"4950000000",
                                               "delivery_state":"unregistered","destination_digest":None}]}
        return payload,cb
    first,cb=job(1,[])
    events=[("bootstrap_applied",{"front_id":0,"restored_slice_ids":[],"pending":[],"applied_blocks":[],"revision":1,"external_history_checkpoint_required":True}),
            ("distribution_frozen",first),("job_issued",{**first,"channel_id":1})]
    rows=[];publications=[]
    header_prefix=(1).to_bytes(4,"little")+bytes.fromhex(first["prev_hash"])+decode_transaction(cb)["txid_bytes"]+(1600000001).to_bytes(4,"little")+(0x200fffff).to_bytes(4,"little")
    nonces=[]
    # Finite offline non-block share vectors. Never submitted to a node.
    for nonce in range(256):
        digest=sha256d(header_prefix+nonce.to_bytes(4,"little"))
        if target(0x200fffff)<int.from_bytes(digest,"little")<(1<<255):nonces.append((nonce,digest))
        if len(nonces)>=count:break
    assert len(nonces)>=count
    for i,(nonce,digest) in enumerate(nonces[:count],1):
        leaf={"nonce":nonce,"ntime":1600000001,"version":1,"extranonce":[0,0],"job_id":(1<<32)|1,"share_index":i-1}
        root=leaf_hash(leaf)
        events.append(("accounted",{"front_id":0,"channel_id":1,"job_id":"00000001","generation":"1","slice_id":str(i),"sp_id":identity,
            "nonce":nonce,"ntime":1600000001,"version":1,"extranonce":"0000","share_index":i-1,"d":1,"m_issue":1,"share_hash":digest.hex(),"credited_job_fees":"0","coinbase":cb.hex()}))
        rows.append({"slice_id":str(i),"front_id":0,"height":1,"root":root.hex(),"difficulty":"1","diff_fee_sum":"0",
                     "credits":[{"sp_id":identity,"diff_sum":"1","diff_fee_sum":"0"}]})
        blob=json.dumps([leaf]).encode();le=lambda n,size:n.to_bytes(size,"little")
        data=le(i,8)+le(1,4)+bytes.fromhex(first["prev_hash"])+le((1<<32)|1,8)+le(0,8)+le(1,4)+le(1,16)+root+le(len(blob),4)+blob
        data+=le(1,2)+bytes.fromhex(identity)+le(1,16)+le(0,16)+b"\2\x20"+bytes.fromhex(FINGERPRINT)
        publications.append(("source_publication",{"stream":"spc:slices","stream_id":f"{100+i}-0","slice_id":str(i),"publication_complete":True,"raw_d_hex":data.hex()}))
    if count:
        current,_=job(2,rows[::-1]);events.extend([("distribution_frozen",current),("job_issued",{**current,"channel_id":1})]);events.extend(publications)
    key=("01"*16,0,"00000002" if count else "00000001",2 if count else 1)
    return events,observation,chain,migrations,key,identity


def fixture(count):return copy.deepcopy(base_fixture(count))


class AuditedWindowTests(unittest.TestCase):
    def verify(self,count,mutate=None):
        events,observation,chain,migrations,key,identity=fixture(count)
        if mutate:mutate(events,observation)
        source=parse(events)
        return prove_eligible(source,key,observation,{chain.tip["hash"]:chain},chain.policy,PROFILE,migrations),source,key,chain,identity

    def test_empty_partial_full_preserve_Q_and_use_actual_selected_denominator(self):
        for count in (0,1,9):
            window,_,_,_,identity=self.verify(count)
            self.assertEqual(window["window"]["facts"],window_facts(9,count))
            raw,pool=rewards(window,5000000000,0,100)
            self.assertEqual(raw,{} if count==0 else {identity:4950000000})
            self.assertEqual(pool,5000000000 if count==0 else 50000000)
            self.assertEqual(window["window_work"],count)

    def test_missing_bootstrap_checkpoint_or_source_slice_is_not_empty_history(self):
        def no_start(events,obs):events.pop(0)
        def incomplete_sql(events,obs):obs["tables"]["economic_checkpoint"]=[];obs["counts"]["economic_checkpoint"]=0
        def missing_slice(events,obs):events.pop()
        for mutate,count in ((no_start,0),(incomplete_sql,0),(missing_slice,1)):
            with self.assertRaises((Inconclusive,Invalid)):self.verify(count,mutate)

    def test_omitted_completed_work_and_false_highwater_or_underfill_reject(self):
        def omitted(events,obs):
            for kind,p in events:
                if kind=="distribution_frozen" and p["generation"]=="2":
                    p["slices"]=[];p["high_water"]={"0":"0"};p["economic"]["slice_ids"]=[]
        def false_flag(events,obs):
            for kind,p in events:
                if kind=="distribution_frozen" and p["generation"]=="2":p["window_underfilled"]=False
        def unknown_front(events,obs):
            for kind,p in events:
                if kind=="distribution_frozen" and p["generation"]=="2":p["high_water"]["1"]="0"
        for mutate in (omitted,false_flag,unknown_front):
            with self.assertRaises((Inconclusive,Invalid)):self.verify(1,mutate)

    def test_pruned_accounting_cannot_relabel_a_later_publication_as_empty(self):
        def remove_account_and_selection(events,obs):
            events[:]=[(k,v) for k,v in events if k!="accounted"]
            for kind,p in events:
                if kind=="distribution_frozen" and p["generation"]=="2":
                    p["slices"]=[];p["high_water"]={"0":"0"};p["economic"]["slice_ids"]=[]
        with self.assertRaisesRegex(Inconclusive,"accounted_prefix_or_visibility"):
            self.verify(1,remove_account_and_selection)

    def test_pool_only_empty_routing_is_checked_and_prior_is_not_zeroed(self):
        window,source,key,chain,identity=self.verify(0)
        provider=object.__new__(ReplayProvider)
        p=source.jobs[key]["payload"];checked=verify_job(source,key,{chain.tip["hash"]:chain},chain.policy)
        with self.assertRaisesRegex(Invalid,"omits"):
            provider._settlement(p,{"subsidy":5000000000,"fees":0,"transactions":[checked["coinbase"]]}, {identity:500000},window)
        with self.assertRaises(Inconclusive):rewards({"selected":[]},5000000000,0,100)

    def test_real_provider_builds_v2_window_annotation_from_complete_inputs(self):
        events,observation,chain,migrations,key,identity=fixture(1)
        events.append(("economic_change_observed",{"economic_revision":"1","anchor_hash":bytes.fromhex(chain.tip["hash"])[::-1].hex(),"coherent_snapshot_reload_required":True}))
        source=parse(events)
        capture=object.__new__(SourceCapture);capture.source_feed={"contract":CONTRACT};capture.sources={"front":source}
        capture.issues={};capture.policy=chain.policy
        capture.scenario={"run":{"id":"vector"},"fronts":[{"id":"front","pool_id":"pool"}],"pools":[{"id":"pool","work_profile":PROFILE}]}
        capture.capture=SimpleNamespace(add=lambda *args:None)
        agents={"alice":{"front":"front","pool_pubkey":pub(7),"sp_id":identity}}
        with contextlib.ExitStack() as stack:
            tmp=stack.enter_context(tempfile.TemporaryDirectory(prefix="t3-warmup-provider-",dir=BASE))
            from economic_checkpoint import query
            envelope={"schema":"spc-lab-bound-sql-capture/1","binding":{"run_id":"vector","pool_id":"pool"},
                "execution":{"exit_code":0,"timed_out":False},"query_sha256":hashlib.sha256(query(source_contract=CONTRACT).encode()).hexdigest(),"observation":observation}
            raw=canonical_json(envelope);path=Path(tmp)/"snapshot.json";path.write_bytes(raw)
            from checkpoint_retention import CheckpointRetention
            capture_sql=lambda *args:{"path":str(path),"sha256":hashlib.sha256(raw).hexdigest()}
            retention=CheckpointRetention(Path(tmp)/"retained","vector",{"front":"pool"},capture_sql,CONTRACT)
            stack.callback(retention.close)
            provider=ReplayProvider(capture,{chain.tip["hash"]:chain},agents,capture_sql,lambda:chain,migrations,retention)
            p=source.jobs[key]["payload"]
            received={"job_id":2,"prev_hash":list(bytes.fromhex(p["prev_hash"])),"version":p["version"],"nbits":p["nbits"],"ntime":p["ntime"],
                "coinbase_prefix":list(bytes.fromhex(p["coinbase_prefix"])),"coinbase_suffix":list(bytes.fromhex(p["coinbase_suffix"])),"merkle_path":[],
                "share_target_be":list(bytes.fromhex(p["share_target"])),"pool_extranonce":[],"miner_extranonce_len":2}
            evidence=provider.prepare("alice",received);bundle=json.loads(evidence.bundle)
            self.assertEqual(bundle["window"],{"facts":window_facts(9,1),"eligible_history_complete":True})
            self.assertEqual(bundle["owed"][identity],4950000000)
            self.assertEqual(len(bundle["slices"]),1)
            self.assertEqual(bundle["witness_reserved"],{})
            self.assertTrue(any(r["trigger"]["kind"]=="provider_checkpoint_admitted" for r in retention.records))
            # Exercise the actual v2 socket server with the real replay provider
            # over these synthetic inputs. No wallet service/process is replaced
            # and this is not a native or compiled-receiver execution receipt.
            from source_feed_codec import CollectorKey,send,receive,checkpoint_digest,consumed_digest,verify_public_evidence
            from source_feed_server import FeedServer
            from storage import ProcFS
            keyfile=Path(tmp)/"collector-key";keyfile.write_text(f"{8:064x}");keyfile.chmod(0o600)
            signer=CollectorKey(keyfile,pub(8))
            binding={**agents["alice"],"scan_pubkey":pub(3),"pid":os.getpid(),"start_time_ticks":ProcFS().identity(os.getpid())["start_time_ticks"]}
            server=FeedServer(tmp,signer,{"alice":binding},provider)
            retain=retention.retain
            def bad_metadata(*args,**kwargs):return {**retain(*args,**kwargs),"revision":999999}
            with patch.object(retention,"retain",side_effect=bad_metadata),self.assertRaisesRegex(Invalid,"metadata mismatch"):
                server.proof("alice",{"kind":"job","schema":SCHEMA,"attempt":[3]*16,"job":received})
            self.assertNotIn("prepared",server.attempts[bytes([3]*16).hex()])
            left,right=socket.socketpair();outcomes=[]
            worker=threading.Thread(target=lambda:outcomes.append(server.exchange(right,"alice")));worker.start()
            try:
                with left:
                    request={"kind":"job","schema":SCHEMA,"attempt":[4]*16,"job":received}
                    send(left,request,time.monotonic()+10);response=receive(left,time.monotonic()+10);cp=response["checkpoint"]
                    ack={k:cp[k] for k in ("attempt","job_id","generation","bundle_digest","history_digest")}
                    ack.update(checkpoint_digest=list(checkpoint_digest(cp)),window=window_facts(9,1),signature="")
                    ack["signature"]=_sign_digest(3,consumed_digest(ack))
                    send(left,{"kind":"consumed","schema":SCHEMA,"ack":ack},time.monotonic()+10)
                    committed=receive(left,time.monotonic()+10)
                    from source_feed_codec import received_digest
                    public={"schema":SCHEMA,"checkpoint":cp,"consumed":ack,"committed":{k:committed[k] for k in ("attempt","ack_digest","signature")}}
                    self.assertEqual(verify_public_evidence(public,[4]*16,received_digest(received),pub(7),pub(8),pub(3),time.time_ns()//1000000),window_facts(9,1))
                worker.join(10);self.assertFalse(worker.is_alive())
                self.assertEqual(outcomes[0]["status"],"COMMITTED_CONSUMPTION_ONLY")
            finally:
                worker.join(11)
                server.journal.close()


if __name__=="__main__":unittest.main()
