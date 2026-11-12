"""Independent public consumer of frozen native_activation.rs r3; no signer.

Source-only conformance is not execution of the compiled native/wallet path.
"""
import copy
import re
from pathlib import PurePosixPath

from case_contract import canonical_json
from oracle import Inconclusive, decode_transaction, integer, require, strict_json, unhex
from source_feed_codec import (SCHEMA as FEED_SCHEMA, array, fields, le, tagged,
    received_digest, checkpoint_digest, consumed_digest, committed_digest,
    verify_public_evidence, verify_signature)

CONTRACT_SHA256="e0b72a8ae3243c0708bdd45a22cba16aac7a61ad3932453663dff1f6c72b4da3"
SOURCE_SHA256="39c75d44c8ff16cf1e08b873eca91ad2988f9da1937ef18265e62d537d8abd99"
SCHEMA="spc.native-job-gate/1"
CAPABILITY="native_source_audit_committed_v1"
INSTANCE_SCHEMA="spc.native-wallet-instance/1"
REGISTRY_SCHEMA="spc.native-job-peers/1"
MAX_REQUEST,MAX_RESPONSE=1024*1024,16*1024
REQUEST="schema run_id miner_id connection_id identity wallet_instance peer_generation connection_config_digest expires_at_unix_ms session_epoch candidate_epoch nonce job".split()
PROCESS="pid uid start_time_ticks boot_id executable_sha256 run_token_sha256 config_sha256".split()
INSTANCE="schema instance_id run_id identity_id identity pool_id front_id process config_path socket_path socket_device socket_inode collector_pubkey pool_pubkey".split()
PEER="miner_id connection_id identity pool_id front_id generation process config_path connection_config_digest".split()
AUDIT="mode run_id miner_id wallet_socket wallet_instance wallet_process wallet_socket_device wallet_socket_inode peer_generation connection_config_digest collector_pubkey pool_pubkey".split()
JOB_SNAPSHOT="job_id target_generation version version_rolling_allowed prev_hash_le min_ntime nbits share_target_le network_target_le merkle_path_le coinbase_tx_prefix coinbase_tx_suffix extranonce_prefix".split()


def blob(value):
    require(isinstance(value,str),"native string type")
    raw=value.encode("utf-8");return le(len(raw),4)+raw


def identifier(value):
    require(isinstance(value,str) and re.fullmatch(r"[A-Za-z0-9._:/-]{1,128}",value) is not None,"native identifier")


def path(value):
    require(isinstance(value,str) and PurePosixPath(value).is_absolute() and ".." not in PurePosixPath(value).parts
            and not any(ord(c)<32 or ord(c)==127 for c in value),"native absolute path")


def full_job_id(job):
    received_digest(job)
    require(len(job["coinbase_prefix"])<=65535 and len(job["coinbase_suffix"])<=65535
            and len(job["pool_extranonce"])<=32 and 4<=job["miner_extranonce_len"]<=32,"native coinbase/extranonce bounds")
    raw=array(job["coinbase_prefix"])+array(job["pool_extranonce"])+bytes(job["miner_extranonce_len"])+array(job["coinbase_suffix"])
    tx=decode_transaction(raw);require(tx["coinbase"],"native candidate is not coinbase")
    script=tx["inputs"][0]["script"];require(bool(script),"native BIP34 height absent")
    # Exact frozen scanner read_first_push/decode_script_num. The independent
    # source-job verifier additionally checks canonical height/carrier bytes.
    op=script[0]
    if op==0:value=b""
    elif 0x51<=op<=0x60:value=script[:1]
    else:
        require(1<=op<=75 and len(script)>op,"native BIP34 push")
        value=script[1:op+1]
    if len(value)==1 and 0x51<=value[0]<=0x60:height=value[0]-0x50
    else:
        require(len(value)<=4 and (not value or not value[-1]&0x80),"native BIP34 height")
        height=int.from_bytes(value,"little")
    return (height<<32)|job["job_id"]


def validate_request(r,now=None):
    fields(r,REQUEST);require(r["schema"]==SCHEMA,"native request schema")
    for k in ("run_id","miner_id","connection_id"):identifier(r[k])
    identity=r["identity"]
    require(isinstance(identity,str) and 0<len(identity.encode())<=255
            and not any(ord(c)<32 or ord(c)==127 for c in identity),"native identity encoding")
    for k,n in (("nonce",16),("wallet_instance",16),("connection_config_digest",32)):
        require(any(array(r[k],n)),"native nonzero binding")
    for k in ("peer_generation","candidate_epoch","expires_at_unix_ms"):integer(r[k],1,(1<<64)-1)
    integer(r["session_epoch"],0,(1<<64)-1);full_job_id(r["job"])
    if now is not None:require(integer(now)<r["expires_at_unix_ms"]<=now+10000,"native request expiry/lifetime")


def request_digest(r):
    validate_request(r)
    raw=le(1,4)+b"".join(blob(r[k]) for k in ("run_id","miner_id","connection_id","identity"))
    raw+=le(r["session_epoch"])+le(r["candidate_epoch"])+array(r["wallet_instance"],16)+le(r["peer_generation"])
    raw+=array(r["connection_config_digest"],32)+le(r["expires_at_unix_ms"])+array(r["nonce"],16)+received_digest(r["job"])
    return tagged("SPc/NativeJobRequestV1",raw)


def activation_digest(r,receipt):
    fields(receipt,("request_digest","candidate_epoch","nonce","evidence","wallet_signature"))
    array(receipt["request_digest"],32);array(receipt["nonce"],16);integer(receipt["candidate_epoch"],0,(1<<64)-1)
    e=receipt["evidence"];fields(e,("schema","checkpoint","consumed","committed"))
    require(e["schema"]==FEED_SCHEMA,"native inner source-feed schema")
    cp,ack,commit=e["checkpoint"],e["consumed"],e["committed"]
    raw=request_digest(r)+blob(CAPABILITY)+blob(FEED_SCHEMA)+checkpoint_digest(cp)
    raw+=unhex(cp["pool"]["signature"],64)+unhex(cp["signature"],64)+consumed_digest(ack)+unhex(ack["signature"],64)
    raw+=committed_digest(commit["attempt"],commit["ack_digest"])+unhex(commit["signature"],64)
    return tagged("SPc/NativeJobActivationV1",raw)


def keys(identity,collector,pool):
    from source_feed_runtime import public_identity
    scan,_=public_identity(identity)
    unhex(collector,32);unhex(pool,32)
    require(len({scan,collector,pool})==3,"native authorities not independent")
    return scan,collector,pool


def verify_response(r,response,collector,pool,now):
    validate_request(r,now);scan,collector,pool=keys(r["identity"],collector,pool)
    if response.get("kind")=="unknown":
        fields(response,("kind","schema","request_digest","candidate_epoch","nonce","code"))
        integer(response["candidate_epoch"],0,(1<<64)-1)
        require(response["schema"]==SCHEMA and response["code"] in {"unavailable","timeout","rejected","superseded"}
                and array(response["request_digest"],32)==request_digest(r) and response["candidate_epoch"]==r["candidate_epoch"]
                and response["nonce"]==r["nonce"],"native Unknown correlation")
        raise Inconclusive("native wallet returned "+response["code"])
    fields(response,("kind","schema","capability","receipt"))
    require(response["kind"]=="approved" and response["schema"]==SCHEMA and response["capability"]==CAPABILITY,"native authenticated capability missing")
    receipt=response["receipt"];digest=activation_digest(r,receipt);e=receipt["evidence"];cp=e["checkpoint"]
    require(array(receipt["request_digest"],32)==request_digest(r) and receipt["candidate_epoch"]==r["candidate_epoch"]
            and receipt["nonce"]==r["nonce"] and cp["job_id"]==full_job_id(r["job"]),"native receipt/request/full job binding")
    window=verify_public_evidence(e,r["nonce"],received_digest(r["job"]),pool,collector,scan,now)
    verify_signature(digest,receipt["wallet_signature"],scan)
    return {"request_digest":request_digest(r).hex(),"candidate_epoch":r["candidate_epoch"],"job_id":cp["job_id"],
        "generation":cp["generation"],"checkpoint_digest":checkpoint_digest(cp).hex(),"consumed_digest":consumed_digest(e["consumed"]).hex(),
        "committed_digest":committed_digest(e["committed"]["attempt"],e["committed"]["ack_digest"]).hex(),
        "expires_at_unix_ms":min(cp["expires_at_unix_ms"],r["expires_at_unix_ms"]),"window":window}


def validate_process(p):
    fields(p,PROCESS);integer(p["pid"],1,(1<<32)-1);integer(p["uid"],0,(1<<32)-1);integer(p["start_time_ticks"],1,(1<<64)-1)
    require(isinstance(p["boot_id"],str) and re.fullmatch(r"[a-fA-F0-9-]{36}",p["boot_id"]) is not None,"native boot id")
    for k in ("executable_sha256","run_token_sha256","config_sha256"):require(any(array(p[k],32)),"native process hash absent")


def validate_instance(w):
    fields(w,INSTANCE);require(w["schema"]==INSTANCE_SCHEMA and any(array(w["instance_id"],16)),"native wallet lifecycle instance")
    for k in ("run_id","identity_id","pool_id","front_id"):identifier(w[k])
    for k in ("config_path","socket_path"):path(w[k])
    integer(w["socket_device"],0,(1<<64)-1);integer(w["socket_inode"],1,(1<<64)-1)
    validate_process(w["process"]);keys(w["identity"],w["collector_pubkey"],w["pool_pubkey"])


def authorize(reg,w,r,observed,now):
    validate_request(r,now);validate_instance(w);validate_process(observed)
    fields(reg,("schema","run_id","wallet_instance","generation","peers"));integer(reg["generation"],1,(1<<64)-1)
    require(reg["schema"]==REGISTRY_SCHEMA and reg["run_id"]==w["run_id"]==r["run_id"]
            and reg["wallet_instance"]==w["instance_id"]==r["wallet_instance"] and r["identity"]==w["identity"],"native registry instance/run binding")
    require(isinstance(reg["peers"],list) and len(reg["peers"])<=256,"native peer registry bound")
    seen=set();match=None
    for p in reg["peers"]:
        fields(p,PEER);validate_process(p["process"]);integer(p["generation"],1,reg["generation"])
        pair=(p["miner_id"],p["connection_id"]);require(pair not in seen,"native ambiguous peer");seen.add(pair)
        if pair==(r["miner_id"],r["connection_id"]):match=p
    require(match is not None,"native peer unregistered")
    require(match["identity"]==r["identity"] and match["pool_id"]==w["pool_id"] and match["front_id"]==w["front_id"]
            and match["generation"]==r["peer_generation"] and match["connection_config_digest"]==r["connection_config_digest"]
            and match["process"]==observed and observed["uid"]==w["process"]["uid"] and observed["boot_id"]==w["process"]["boot_id"],"native observed peer mismatch")
    return match


def connection_digest(config):
    value=copy.deepcopy(config);audit=value.get("source_audit",{})
    require(audit.get("mode")=="required" and "connection_config_digest" in audit,"native required config shape")
    audit["connection_config_digest"]=[0]*32;out=bytearray()
    def encode(v,depth=0):
        require(depth<=32 and len(out)<=MAX_REQUEST,"native config bound")
        if v is None:out.append(0)
        elif type(v) is bool:out.extend((1,int(v)))
        elif type(v) is int:
            if v>=0:out.extend(b"\2"+le(v))
            else:integer(v,-(1<<63),-1);out.extend(b"\3"+v.to_bytes(8,"little",signed=True))
        elif isinstance(v,str):out.extend(b"\4"+blob(v))
        elif isinstance(v,list):
            out.extend(b"\5"+le(len(v),4))
            for item in v:encode(item,depth+1)
        elif isinstance(v,dict):
            require(all(isinstance(k,str) for k in v),"native config key type");out.extend(b"\6"+le(len(v),4))
            for k in sorted(v):out.extend(blob(k));encode(v[k],depth+1)
        else:raise Inconclusive("native config float/unsupported type")
    encode(value);require(len(out)<=MAX_REQUEST,"native config bound")
    return tagged("SPc/NativeConnectionConfigV1",out)


def decode_frame(raw,request=True):
    maximum=MAX_REQUEST if request else MAX_RESPONSE
    require(isinstance(raw,bytes) and len(raw)>=4,"native truncated frame")
    size=int.from_bytes(raw[:4],"little")
    require(0<size<=maximum and len(raw)==size+4,"native frame length/bound")
    value=strict_json(raw[4:])
    def bounded(v,depth=0):
        require(depth<128,"native JSON recursion bound")
        if isinstance(v,dict):
            for item in v.values():bounded(item,depth+1)
        elif isinstance(v,list):
            for item in v:bounded(item,depth+1)
    bounded(value)
    if request:validate_request(value)
    elif value.get("kind")=="approved":
        fields(value,("kind","schema","capability","receipt"));receipt=value["receipt"]
        fields(receipt,("request_digest","candidate_epoch","nonce","evidence","wallet_signature"))
        array(receipt["request_digest"],32);array(receipt["nonce"],16);integer(receipt["candidate_epoch"],0,(1<<64)-1)
        e=receipt["evidence"];fields(e,("schema","checkpoint","consumed","committed"))
        fields(e["committed"],("attempt","ack_digest","signature"))
        checkpoint_digest(e["checkpoint"]);consumed_digest(e["consumed"]);committed_digest(e["committed"]["attempt"],e["committed"]["ack_digest"])
        for signature in (receipt["wallet_signature"],e["checkpoint"]["pool"]["signature"],e["checkpoint"]["signature"],e["consumed"]["signature"],e["committed"]["signature"]):unhex(signature,64)
    else:
        fields(value,("kind","schema","request_digest","candidate_epoch","nonce","code"))
        require(value["kind"]=="unknown" and value["code"] in {"unavailable","timeout","rejected","superseded"},"native response variant")
        array(value["request_digest"],32);array(value["nonce"],16);integer(value["candidate_epoch"],0,(1<<64)-1)
    return value


def received_from_snapshot(j,miner_extranonce_len):
    fields(j,JOB_SNAPSHOT)
    for k in ("job_id","version","min_ntime","nbits"):integer(j[k],0,(1<<32)-1)
    integer(j["target_generation"]);require(type(j["version_rolling_allowed"]) is bool,"native version rolling type")
    return {"job_id":j["job_id"],"prev_hash":list(unhex(j["prev_hash_le"],32)),"version":j["version"],"ntime":j["min_ntime"],"nbits":j["nbits"],
        "coinbase_prefix":list(unhex(j["coinbase_tx_prefix"])),"coinbase_suffix":list(unhex(j["coinbase_tx_suffix"])),
        "merkle_path":[list(unhex(v,32)) for v in j["merkle_path_le"]],"share_target_be":list(unhex(j["share_target_le"],32)[::-1]),
        "pool_extranonce":list(unhex(j["extranonce_prefix"])),"miner_extranonce_len":miner_extranonce_len}
