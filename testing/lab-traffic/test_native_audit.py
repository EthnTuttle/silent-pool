"""Synthetic signed native r3 consumer vectors; no producer tests or live grant."""
import copy
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
from types import SimpleNamespace

import native_codec as n
from native_capture import NativeCapture, observe_process, require_audited_sources, SOURCE_ROWS
from native_observer import bind_connection, replay, controller_events
from source_feed_codec import (SCHEMA, _sign_digest, tagged, received_digest, trace_digest, checkpoint_digest,
    consumed_digest, committed_digest, window_facts)
from source_feed_runtime import public_identity
from storage import ProcFS, sha_file
from test_source_feed import pub
from test_oracle import coinbase
from public_derivation import _GX,_GY
from signature_oracle import _multiply
from oracle import Inconclusive,Invalid
from case_contract import canonical_json, required

BASE=None


def address():
    data=b""
    for secret in (3,4):
        x,y=_multiply(secret,(_GX,_GY));data+=bytes([2+(y&1)])+x.to_bytes(32,"big")
    values=[0];acc=bits=0
    for byte in data:
        acc=(acc<<8)|byte;bits+=8
        while bits>=5:bits-=5;values.append((acc>>bits)&31)
    if bits:values.append((acc<<(5-bits))&31)
    hrp="tsp";check=1
    for v in [ord(c)>>5 for c in hrp]+[0]+[ord(c)&31 for c in hrp]+values+[0]*6:
        top=check>>25;check=((check&0x1ffffff)<<5)^v
        for i,g in enumerate((0x3b6a57b2,0x26508e6d,0x1ea119fa,0x3d4233dd,0x2a1462b3)):
            if top>>i&1:check^=g
    check^=0x2bc830a3
    return hrp+"1"+"".join("qpzry9x8gf2tvdw0s3jn54khce6mua7l"[v] for v in values+[(check>>(5*i))&31 for i in range(5,-1,-1)])


def fixture(actual=1):
    identity=address();assert public_identity(identity)[0]==pub(3)
    proc={"pid":1234,"uid":1000,"start_time_ticks":55,"boot_id":"01234567-89ab-cdef-0123-456789abcdef",
          "executable_sha256":[1]*32,"run_token_sha256":[2]*32,"config_sha256":[3]*32}
    wallet={"schema":n.INSTANCE_SCHEMA,"instance_id":[21]*16,"run_id":"run","identity_id":"alice","identity":identity,
        "pool_id":"pool-a","front_id":"front-a","process":{**proc,"pid":1235},"config_path":"/unit/private/wallet.toml",
        "socket_path":"/unit/private/wallet/native-audit.sock","socket_device":1,"socket_inode":5,"collector_pubkey":pub(8),"pool_pubkey":pub(7)}
    worker={"id":"worker","budget":{"hashes_per_second":10,"total_attempts":None},"ntime_policy":"hold","max_ntime_roll_seconds":600,"batch_size":256}
    config={"id":"connection","endpoint":"127.0.0.1:2000","identity":identity,"authority_public_key":"actual-sv2-authority-fixture",
        "min_extranonce_size":4,"reconnect":{"enabled":True,"initial_backoff_ms":100,"max_backoff_ms":5000,"handshake_timeout_ms":10000},"workers":[worker],
        "source_audit":{"mode":"required","run_id":"run","miner_id":"miner","wallet_socket":wallet["socket_path"],"wallet_instance":wallet["instance_id"],
            "wallet_process":wallet["process"],"wallet_socket_device":1,"wallet_socket_inode":5,"peer_generation":2,
            "connection_config_digest":[0]*32,"collector_pubkey":pub(8),"pool_pubkey":pub(7)}}
    config["source_audit"]["connection_config_digest"]=list(n.connection_digest(config))
    peer={"miner_id":"miner","connection_id":"connection","identity":identity,"pool_id":"pool-a","front_id":"front-a","generation":2,
        "process":proc,"config_path":"/unit/private/miner.toml","connection_config_digest":config["source_audit"]["connection_config_digest"]}
    reg={"schema":n.REGISTRY_SCHEMA,"run_id":"run","wallet_instance":wallet["instance_id"],"generation":2,"peers":[peer]}
    route={"run_id":"run","miner_id":"miner","connection_id":"connection","identity":identity,"endpoint":config["endpoint"],
        "authority_public_key":config["authority_public_key"],"workers":[worker],"front_id":"front-a","pool_id":"pool-a","miner_config_path":peer["config_path"]}
    binding=bind_connection(config,wallet,reg,proc,route)
    cb=coinbase(tag=b"\x51\x08"+bytes(8));job={"job_id":5,"prev_hash":[6]*32,"version":1,"nbits":0x207fffff,"ntime":100,
        "coinbase_prefix":list(cb[:44]),"coinbase_suffix":list(cb[52:]),"merkle_path":[],"share_target_be":[255]*32,"pool_extranonce":[1]*4,"miner_extranonce_len":4}
    r={"schema":n.SCHEMA,"run_id":"run","miner_id":"miner","connection_id":"connection","identity":identity,"wallet_instance":wallet["instance_id"],
        "peer_generation":2,"connection_config_digest":config["source_audit"]["connection_config_digest"],"expires_at_unix_ms":11000,
        "session_epoch":2,"candidate_epoch":3,"nonce":[4]*16,"job":job}
    trace={"session":[5]*16,"seq":6,"digest":[7]*32,"signature":""};trace["signature"]=_sign_digest(7,trace_digest(trace))
    bundle={"schema":"spc.payout-proof/1","job":{"extranonce_len":8},"window_target":"9","slices":[] if not actual else [{"difficulty":str(actual)}],
            "window":{"facts":window_facts(9,actual),"eligible_history_complete":True}}
    cp={"attempt":r["nonce"],"wire_job_digest":list(received_digest(job)),"job_id":n.full_job_id(job),"generation":9,"source_watermark":6,"history_watermark":10,
        "bundle_digest":list(hashlib.sha256(canonical_json(bundle)).digest()),"history_digest":[12]*32,"expires_at_unix_ms":20000,"complete":True,"pool":trace,"signature":""}
    cp["signature"]=_sign_digest(8,checkpoint_digest(cp))
    ack={k:cp[k] for k in ("attempt","job_id","generation","bundle_digest","history_digest")}
    ack.update(checkpoint_digest=list(checkpoint_digest(cp)),window=window_facts(9,actual),signature="");ack["signature"]=_sign_digest(3,consumed_digest(ack))
    commit={"attempt":r["nonce"],"ack_digest":list(consumed_digest(ack)),"signature":""};commit["signature"]=_sign_digest(8,committed_digest(commit["attempt"],commit["ack_digest"]))
    evidence={"schema":SCHEMA,"checkpoint":cp,"consumed":ack,"committed":commit}
    receipt={"request_digest":list(n.request_digest(r)),"candidate_epoch":3,"nonce":r["nonce"],"evidence":evidence,"wallet_signature":""}
    receipt["wallet_signature"]=_sign_digest(3,n.activation_digest(r,receipt))
    response={"kind":"approved","schema":n.SCHEMA,"capability":n.CAPABILITY,"receipt":receipt}
    snapshot={"job_id":5,"target_generation":2,"version":1,"version_rolling_allowed":False,"prev_hash_le":bytes(job["prev_hash"]).hex(),"min_ntime":100,"nbits":job["nbits"],
        "share_target_le":"ff"*32,"network_target_le":((1<<255)-1).to_bytes(32,"little").hex(),"merkle_path_le":[],"coinbase_tx_prefix":bytes(job["coinbase_prefix"]).hex(),
        "coinbase_tx_suffix":bytes(job["coinbase_suffix"]).hex(),"extranonce_prefix":"01"*4}
    proof=n.verify_response(r,response,pub(8),pub(7),1000)
    payloads=[("job_candidate",{"connection_id":"connection","session_slot":2,"candidate_epoch":3,"request_digest":n.request_digest(r).hex(),"job":snapshot}),
        ("source_audit_committed",{"connection_id":"connection","session_slot":2,"capability":n.CAPABILITY,**proof}),
        ("job_activated",{"connection_id":"connection","job":snapshot}),
        ("audit_first_hash",{"connection_id":"connection","worker_id":"worker","session_slot":2,"candidate_epoch":3,"request_digest":n.request_digest(r).hex(),"job_id":5})]
    events=[{"protocol":"spc-cpu-miner-events","version":1,"kind":"event","source":"process","event_id":i,"source_cursor":i,"miner_id":"miner",
        "occurred_at_wall_ms":1000+i,"occurred_at_mono_ms":i,"dropped_before":0,"event":{"name":name,"payload":p}} for i,(name,p) in enumerate(payloads,1)]
    collectors={consumed_digest(ack).hex():{"agent":"alice","binding":{"miner_id":"miner","front":"front-a","pid":1235,"start_time_ticks":55,"scan_pubkey":pub(3),"pool_pubkey":pub(7)},
        "evidence":evidence,"bundle":bundle,"collector_pubkey":pub(8),"attempt":bytes(r["nonce"]).hex()}}
    return r,response,{("miner","connection"):binding},collectors,events


class NativeAuditTests(unittest.TestCase):
    def test_signed_empty_partial_full_native_receipts_preserve_actual_window(self):
        for actual in (0,1,9):
            r,response,b,c,events=fixture(actual)
            proof=n.verify_response(r,response,pub(8),pub(7),1000)
            self.assertEqual(proof["window"],window_facts(9,actual))
            self.assertEqual(proof["expires_at_unix_ms"],11000)
            result=replay(events,b,c,{n.request_digest(r).hex():(r,response)})
            self.assertEqual(result["status"],"PASSED");self.assertTrue(result["full_Q_campaign_still_required"])

    def test_request_preimage_includes_lifecycle_config_and_native_nonce(self):
        r,_,_,_,_=fixture();blob=lambda s:len(s.encode()).to_bytes(4,"little")+s.encode()
        raw=(1).to_bytes(4,"little")+b"".join(blob(r[k]) for k in ("run_id","miner_id","connection_id","identity"))
        raw+=(2).to_bytes(8,"little")+(3).to_bytes(8,"little")+bytes(r["wallet_instance"])+(2).to_bytes(8,"little")
        raw+=bytes(r["connection_config_digest"])+(11000).to_bytes(8,"little")+bytes([4]*16)+received_digest(r["job"])
        self.assertEqual(n.request_digest(r),tagged("SPc/NativeJobRequestV1",raw))
        for k in ("run_id","miner_id","connection_id","wallet_instance","peer_generation","connection_config_digest","expires_at_unix_ms","session_epoch","candidate_epoch","nonce"):
            q=copy.deepcopy(r);v=q[k]
            if isinstance(v,str):q[k]+="x"
            elif isinstance(v,list):q[k][0]^=1
            else:q[k]+=1
            self.assertNotEqual(n.request_digest(q),n.request_digest(r),k)

    def test_outer_signature_binds_source_schema_all_digests_and_signatures(self):
        r,response,_,_,_=fixture();receipt=response["receipt"];e=receipt["evidence"];cp=e["checkpoint"];a=e["consumed"];c=e["committed"]
        blob=lambda s:len(s).to_bytes(4,"little")+s.encode()
        raw=n.request_digest(r)+blob(n.CAPABILITY)+blob(SCHEMA)+checkpoint_digest(cp)+bytes.fromhex(cp["pool"]["signature"])+bytes.fromhex(cp["signature"])
        raw+=consumed_digest(a)+bytes.fromhex(a["signature"])+committed_digest(c["attempt"],c["ack_digest"])+bytes.fromhex(c["signature"])
        self.assertEqual(n.activation_digest(r,receipt),tagged("SPc/NativeJobActivationV1",raw))
        altered=copy.deepcopy(response);altered["receipt"]["evidence"]["schema"]="spc.source-feed/1"
        with self.assertRaises(Invalid):n.verify_response(r,altered,pub(8),pub(7),1000)
        altered=copy.deepcopy(response);altered["receipt"]["wallet_signature"]="00"*64
        with self.assertRaises(Invalid):n.verify_response(r,altered,pub(8),pub(7),1000)

    def test_old_r2_inner_attempt_override_and_unsigned_health_cannot_activate(self):
        r,response,_,_,_=fixture();q=copy.deepcopy(r);q["nonce"]=[9]*16
        changed=copy.deepcopy(response);rec=changed["receipt"];rec.update(request_digest=list(n.request_digest(q)),nonce=q["nonce"])
        rec["wallet_signature"]=_sign_digest(3,n.activation_digest(q,rec))
        with self.assertRaises(Invalid):n.verify_response(q,changed,pub(8),pub(7),1000)
        with self.assertRaises(Inconclusive):n.verify_response(r,{"healthy":True},pub(8),pub(7),1000)

    def test_unknown_codes_and_request_response_frames_are_closed(self):
        r,response,_,_,_=fixture()
        for value,is_request in ((r,True),(response,False)):
            raw=canonical_json(value);frame=len(raw).to_bytes(4,"little")+raw
            self.assertEqual(n.decode_frame(frame,is_request),value)
            for broken in (frame[:-1],frame+b"x",bytes(4)+raw):
                with self.assertRaises(Invalid):n.decode_frame(broken,is_request)
        raw=canonical_json(r);raw=raw[:-1]+b',"schema":"spc.native-job-gate/1"}'
        with self.assertRaises(Invalid):n.decode_frame(len(raw).to_bytes(4,"little")+raw)
        unknown={"kind":"unknown","schema":n.SCHEMA,"request_digest":list(n.request_digest(r)),"candidate_epoch":3,"nonce":r["nonce"],"code":"timeout"}
        with self.assertRaises(Inconclusive):n.verify_response(r,unknown,pub(8),pub(7),1000)
        unknown["code"]="healthy"
        with self.assertRaises(Invalid):n.verify_response(r,unknown,pub(8),pub(7),1000)

    def test_full_u64_job_and_expiry_are_not_truncated_or_refreshed(self):
        r,response,_,_,_=fixture();self.assertEqual(n.full_job_id(r["job"]),(1<<32)|5)
        for now in (0,11000,20000):
            with self.assertRaises(Invalid):n.verify_response(r,response,pub(8),pub(7),now)
        response["receipt"]["evidence"]["checkpoint"]["job_id"]=5
        with self.assertRaises(Invalid):n.verify_response(r,response,pub(8),pub(7),1000)

    def test_peer_authorizer_uses_observed_process_not_claimed_request_values(self):
        r,_,bindings,_,_=fixture();b=next(iter(bindings.values()))
        self.assertEqual(n.authorize(b["registry"],b["wallet"],r,b["observed_miner"],1000)["miner_id"],"miner")
        for field in n.PROCESS:
            p=copy.deepcopy(b["observed_miner"])
            if isinstance(p[field],list):p[field][0]^=1
            elif isinstance(p[field],str):p[field]="0"+p[field][1:];p[field]=p[field][:-1]+"e"
            else:p[field]+=1
            with self.assertRaises(Invalid):n.authorize(b["registry"],b["wallet"],r,p,1000)

    def test_peer_generations_restart_and_duplicate_bindings_reject(self):
        r,_,bindings,_,_=fixture();b=next(iter(bindings.values()))
        for kind in ("duplicate","future","instance","scope"):
            reg=copy.deepcopy(b["registry"])
            if kind=="duplicate":reg["peers"]*=2
            elif kind=="future":reg["peers"][0]["generation"]+=1
            elif kind=="instance":reg["wallet_instance"][0]^=1
            else:reg["peers"][0]["front_id"]="foreign"
            with self.assertRaises(Invalid):n.authorize(reg,b["wallet"],r,b["observed_miner"],1000)

    def test_connection_binary_json_hash_binds_defaults_workers_and_no_float(self):
        _,_,bindings,_,_=fixture();config=next(iter(bindings.values()))["config"]
        different=copy.deepcopy(config);different["source_audit"]["connection_config_digest"]=[255]*32
        self.assertEqual(n.connection_digest(config),n.connection_digest(different))
        for k in ("endpoint","workers","reconnect","source_audit"):
            changed=copy.deepcopy(config)
            if k=="endpoint":changed[k]="127.0.0.1:1234"
            elif k=="workers":changed[k][0]["budget"]["hashes_per_second"]=40
            elif k=="reconnect":changed[k]["initial_backoff_ms"]+=1
            else:changed[k]["peer_generation"]+=1
            self.assertNotEqual(n.connection_digest(config),n.connection_digest(changed))
        changed=copy.deepcopy(config);changed["reconnect"]["initial_backoff_ms"]=100.0
        with self.assertRaises(Inconclusive):n.connection_digest(changed)

    def test_fixture_unaudited_profile_never_fulfils_campaign(self):
        _,_,bindings,_,_=fixture();b=next(iter(bindings.values()));config=copy.deepcopy(b["config"]);config["source_audit"]={"mode":"unaudited"}
        with self.assertRaises(Inconclusive):bind_connection(config,b["wallet"],b["registry"],b["observed_miner"],b["route"])
        self.assertIn("native_source_applicability",required("funded"));self.assertIn("premining_proof_feed",required("funded"))
        with self.assertRaises(Inconclusive):require_audited_sources({})

    def test_valid_collector_exchange_without_outer_bytes_is_not_native_approval(self):
        _,_,b,c,events=fixture();result=replay(events,b,c)
        self.assertEqual(result["status"],"INCONCLUSIVE")
        self.assertIn("retained_actual_native_request_and_signed_Approved_bytes",result["missing_components"])
        self.assertEqual(len(result["verified_commits"]),1)
        self.assertEqual(replay([],b,c)["status"],"INCONCLUSIVE")

    def test_commit_activation_first_hash_order_and_window_tampering_reject(self):
        r,response,b,c,original=fixture();outer={n.request_digest(r).hex():(r,response)}
        for kind in ("before_commit","first_before_activation","window","digest","job"):
            events=copy.deepcopy(original)
            if kind=="before_commit":events[1]["event"],events[2]["event"]=events[2]["event"],events[1]["event"]
            elif kind=="first_before_activation":events[2]["event"],events[3]["event"]=events[3]["event"],events[2]["event"]
            elif kind=="window":events[1]["event"]["payload"]["window"]=window_facts(9,9)
            elif kind=="digest":events[1]["event"]["payload"]["checkpoint_digest"]="00"*32
            else:events[2]["event"]["payload"]["job"]["version"]+=1
            with self.subTest(kind=kind),self.assertRaises(Invalid):replay(events,b,c,outer)

    def test_omitted_drop_truncated_prefix_and_missing_durable_ACK_are_inconclusive(self):
        _,_,b,c,events=fixture()
        for changed in (events[1:],events[:2]+events[3:]):
            with self.assertRaises(Inconclusive):replay(changed,b,c)
        dropped=copy.deepcopy(events);dropped[-1]["dropped_before"]=1
        with self.assertRaises(Inconclusive):replay(dropped,b,c)
        with self.assertRaises(Inconclusive):replay(events,b,{})

    def test_post_hash_emission_after_expiry_or_new_candidate_stays_inconclusive(self):
        _,_,b,c,events=fixture();late=copy.deepcopy(events);late[-1]["occurred_at_wall_ms"]=11000
        with self.assertRaisesRegex(Inconclusive,"hash_time"):replay(late,b,c)
        later=copy.deepcopy(events[0]);later["event"]["payload"]["candidate_epoch"]+=1
        changed=events[:3]+[later]+events[3:]
        for i,row in enumerate(changed,1):row.update(event_id=i,source_cursor=i,occurred_at_wall_ms=1000+i,occurred_at_mono_ms=i)
        with self.assertRaisesRegex(Inconclusive,"hash_time"):replay(changed,b,c)

    def test_actual_EvidenceStore_projection_retains_origin_and_detects_gap(self):
        _,_,b,c,events=fixture();rows=[]
        for row in events:
            rows.append({"run_id":"run","event_id":row["event_id"],"source":"miner-miner","source_cursor":"token:55:"+str(row["source_cursor"]),
                "actor":{"kind":"miner","id":"miner"},"command_id":None,"occurred_at_wall":row["occurred_at_wall_ms"],
                "occurred_at_mono":row["occurred_at_mono_ms"]*1000000,"ingested_at":2000,
                "payload":{"kind":"miner_event","data":{"miner_id":"miner","incarnation":"token:55","driver_event_id":row["event_id"],
                    "name":row["event"]["name"],"payload":row["event"]["payload"],"dropped_before":0}}})
        raw=b"".join(canonical_json(r)+b"\n" for r in rows)
        self.assertEqual(controller_events(raw,"run",{"miner":"token:55"}),events)
        with self.assertRaises(Inconclusive):controller_events(raw[:-1],"run",{"miner":"token:55"})
        with self.assertRaises(Invalid):controller_events(raw,"run",{"miner":"token:56"})
        rows[1]["payload"]={"kind":"event_lost","data":{}}
        with self.assertRaises(Inconclusive):controller_events(b"".join(canonical_json(r)+b"\n" for r in rows),"run",{"miner":"token:55"})

    def test_process_observer_checks_real_proc_token_config_and_executable(self):
        with tempfile.TemporaryDirectory(prefix="t3-native-proc-",dir=BASE) as tmp:
            root=Path(tmp);private=root/"private";private.mkdir(mode=0o700);config=private/"miner.toml";config.write_text("offline_process_fixture=true\n");config.chmod(0o600)
            process=subprocess.Popen([sys.executable,"-c","import sys;sys.stdin.buffer.read()","--config",str(config)],stdin=subprocess.PIPE,
                env={**os.environ,"SPC_LAB_RUN_TOKEN":"public-unit-token"})
            try:
                ticks=ProcFS().identity(process.pid)["start_time_ticks"];exe=Path(f"/proc/{process.pid}/exe").resolve();digest=sha_file(exe)
                record={"pid":process.pid,"start_time_ticks":ticks,"executable_path":str(exe),"executable_sha256":digest,
                    "args":["--config",str(config)],"run_token":"public-unit-token"}
                observed,path=observe_process(record,root,{"path":str(exe),"sha256":digest},os.getpid())
                self.assertEqual(path,str(config));self.assertEqual(bytes(observed["run_token_sha256"]),hashlib.sha256(b"public-unit-token").digest())
                with self.assertRaises(Invalid):observe_process({**record,"run_token":"foreign"},root,{"path":str(exe),"sha256":digest},os.getpid())
                with self.assertRaises(Invalid):observe_process({**record,"start_time_ticks":ticks+1},root,{"path":str(exe),"sha256":digest},os.getpid())
                readlink=os.readlink
                def foreign_namespace(path,*args,**kwargs):
                    return 'net:[foreign-unit]' if str(path)==f'/proc/{process.pid}/ns/net' else readlink(path,*args,**kwargs)
                with patch('native_capture.os.readlink',side_effect=foreign_namespace),self.assertRaisesRegex(Invalid,'namespace'):
                    observe_process(record,root,{"path":str(exe),"sha256":digest},os.getpid())
            finally:
                process.stdin.close();process.wait(timeout=5)

    def test_controller_pipe_and_actual_append_only_EvidenceStore_are_captured(self):
        with tempfile.TemporaryDirectory(prefix="t3-native-capture-",dir=BASE) as tmp:
            root=Path(tmp);qa=root/"qa";qa.mkdir(mode=0o700);run=root/"run";records=[]
            exe=Path("/proc/self/exe").resolve();binary={"path":str(exe),"sha256":sha_file(exe)}
            c=SimpleNamespace(run=run,priv=qa,scenario={"miners":[{"id":"miner"}]},process=SimpleNamespace(pid=os.getpid()),
                binding={"binaries":{"miner":binary,"lab":binary}},capture=SimpleNamespace(add=lambda kind,value:records.append((kind,value))))
            capture=NativeCapture(c);run.mkdir(mode=0o700);private=run/"private";private.mkdir(mode=0o700)
            config=private/"miner.toml";config.write_text("offline=true\n");config.chmod(0o600)
            process=subprocess.Popen([sys.executable,"-c","import sys;sys.stdin.buffer.read()","--config",str(config)],stdin=subprocess.PIPE,stdout=subprocess.PIPE,
                env={**os.environ,"SPC_LAB_RUN_TOKEN":"public-unit-token"})
            try:
                record={"role":"miner","entity":"miner","pid":process.pid,"start_time_ticks":ProcFS().identity(process.pid)["start_time_ticks"],
                    "executable_path":str(exe),"executable_sha256":binary["sha256"],"args":["--config",str(config)],"run_token":"public-unit-token"}
                manifest={"locks":{"pid":os.getpid()},"processes":[record]}
                evidence=private/"evidence.jsonl"
                with evidence.open("w+b") as f:
                    evidence.chmod(0o600);f.write(b'{"retained_unit_prefix":1}\n');f.flush()
                    capture.poll(manifest);self.assertNotIn("controller",capture.issues)
                    self.assertEqual(capture.issues["miner"].component,"native_public_evidence_durable_v1_source_binding_unavailable")
                    f.write(b'{"retained_unit_prefix":2}\n');f.flush();capture.poll(manifest)
                    self.assertEqual((capture.out/"controller-evidence.jsonl").read_bytes(),evidence.read_bytes())
                    self.assertEqual(len([r for r in records if r[0]=="native_controller_evidence_prefix"]),2)
                    original=evidence.read_bytes();f.seek(0);f.write(b"X");f.flush();capture.poll(manifest)
                    self.assertIsInstance(capture.issues["controller"],Invalid)
                    f.seek(0);f.write(original);f.flush();capture.poll(manifest)
                    self.assertIsInstance(capture.issues["controller"],Invalid)
            finally:
                process.stdin.close();process.wait(timeout=5);process.stdout.close()

    def test_source_applicability_rejects_pre_native_diagnostic_manifest(self):
        with tempfile.TemporaryDirectory(prefix="t3-native-source-",dir=BASE) as tmp:
            p=Path(tmp)/"source.json";p.write_bytes(canonical_json({"files":[{"path":k,"sha256":v} for k,v in SOURCE_ROWS.items()]}))
            binary={"source_manifest":str(p),"source_manifest_sha256":sha_file(p)}
            from source_feed_codec import CONTRACT
            binding={"source_feed":{"contract":CONTRACT},"binaries":{"miner":binary,"lab":binary}}
            with self.assertRaisesRegex(Inconclusive,'base_diagnostic'):require_audited_sources(binding)
            p.write_bytes(canonical_json({"files":[]}));binary["source_manifest_sha256"]=sha_file(p)
            with self.assertRaisesRegex(Inconclusive,"base_diagnostic"):require_audited_sources(binding)

    def test_native_loaded_wallet_collector_config_has_no_legacy_scan_file_fallback(self):
        from source_feed_runtime import config_fields
        raw=b'[ark.native_audit]\ncollector_socket = "/unit/collector.sock"\npool_pubkey = "pool"\ncollector_pubkey = "collector"\nrun_id = "run"\nidentity_id = "alice"\npool_id = "pool-a"\nfront_id = "front-a"\nrun_token_hash = "'+b'01'*32+b'"\n'
        self.assertEqual(config_fields(raw),{"socket":"/unit/collector.sock","pool_pubkey":"pool","collector_pubkey":"collector",'run_token_hash':'01'*32})
        with self.assertRaises(Inconclusive):config_fields(b'[source_audit]\nsocket = "/unit/legacy.sock"\n')
        with self.assertRaises(Invalid):config_fields(raw+b'collector_pubkey = "other"\n')


if __name__=="__main__":unittest.main()
