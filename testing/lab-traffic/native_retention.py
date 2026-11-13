"""Read-only consumer of native_evidence.rs 017fa69f; no wallet or signer.

Stored Approved exchanges explicitly do not assert activation. Only the actual
controller event's DurableHandle and subsequent activation/hash events do so.
"""
from copy import deepcopy
import hashlib
import json
import os
from pathlib import Path
import re
import stat

import native_codec as native
from case_contract import canonical_json
from oracle import Inconclusive, Invalid, integer, require, strict_json
from source_feed_codec import array, fields
from source_jobs import MissingComponent

SCHEMA="spc.native-evidence/1"
MANIFEST_SCHEMA="spc.native-evidence-manifest/1"
CONFIG_SCHEMA="spc.native-evidence-config/1"
HANDLE_SCHEMA="spc.native-evidence-handle/1"
LOCATOR_SCHEMA="spc.native-evidence-locator/1"
CAPABILITY="native_public_evidence_durable_v1"
MAX_RECORD=2138112
MAX_MANIFEST=16384
MAX_RECORDS=16384
MAX_BYTES=256*1024*1024
MAX_SESSIONS=32
CONFIG="schema run_id directory run_manifest max_records max_bytes max_sessions".split()
MANIFEST="schema kind capability capture_id miner_id config process config_path".split()
HANDLE="schema capability capture_id receipt_sequence manifest_relative_path manifest_sha256 record_relative_path record_sha256 request_digest".split()
LOCATOR="schema capability directory manifest_relative_path manifest_sha256".split()
BINDING="run_id miner_id connection_id session_epoch candidate_epoch wallet_instance peer_generation connection_config_digest nonce".split()
RECORD="schema kind version activation capture_id receipt_sequence manifest_sha256 request_digest request_sha256 response_sha256 checkpoint_digest binding verified_at_unix_ms request_json_hex response_json_hex".split()
SOURCE_ROWS={
    "crates/spc-scan/src/native_activation.rs":native.SOURCE_SHA256,
    "crates/spc-scan/src/native_evidence.rs":"017fa69f8a4a8a79cb8eee450390df4f25a15b0b4902022c9f0c1a2986e337de",
    "bins/spc-cpu-miner/src/evidence.rs":"2d72744570b06c9f0b40a3ba0046fc699d93c5dbbc336ad95b8365a6e51a738a",
    "bins/spc-cpu-miner/src/native_gate.rs":"220c92c4c1602709fa037a24a3660a99a21170fc49ea04188c9e52612b286569",
    "bins/spc-cpu-miner/src/report.rs":"0aa88f278a057b73a80f74c7a7a3eb412b8e722a2e14ed0b97ddf27756ff7932",
    "bins/spc-cpu-miner/src/config.rs":"e68a49013d1e31b95a32e63779cc2c6ad1103467e02646ad721c48fe59ef46ba",
}
_TOKEN=object()


def sha(raw):return hashlib.sha256(raw).hexdigest()


def hex_bytes(value,maximum,exact=None):
    require(isinstance(value,str) and len(value)<=2*maximum and len(value)%2==0
            and re.fullmatch("[0-9a-f]*",value) is not None,"native retention bounded lowercase hex")
    if exact is not None:require(len(value)==2*exact,"native retention hex length")
    return bytes.fromhex(value)


def identifier(value):
    require(isinstance(value,str) and re.fullmatch(r"[A-Za-z0-9._:-]{1,128}",value) is not None,"native retention identifier")


def absolute(value):
    require(isinstance(value,str) and value.startswith("/") and not value.startswith("//") and value==str(Path(value))
            and all(part not in {".",".."} for part in value.split("/"))
            and not any(ord(c)<32 or ord(c)==127 for c in value),"native retention canonical absolute path")
    return Path(value)


def session_name(process):return f"process-{process['pid']}-{process['start_time_ticks']}"


def config(value,run,run_id,miner):
    fields(value,CONFIG);identifier(run_id);identifier(miner);root=absolute(str(run))
    require(value["schema"]==CONFIG_SCHEMA and value["run_id"]==run_id,"native retention config schema/run")
    require(absolute(value["run_manifest"])==root/"run-manifest.json" and absolute(value["directory"])==root/"private/native-evidence"/miner,"native retention foreign run/miner root")
    integer(value["max_records"],1,65536);integer(value["max_bytes"],MAX_RECORD,16*1024**3);integer(value["max_sessions"],1,256)
    if value["max_records"]>MAX_RECORDS or value["max_bytes"]>MAX_BYTES or value["max_sessions"]>MAX_SESSIONS:
        raise MissingComponent("native_retention_config_exceeds_consumer_resource_capacity")
    return value


def manifest(raw,run,run_id,miner,process,config_path,expected_config):
    require(isinstance(raw,bytes) and 0<len(raw)<=MAX_MANIFEST,"native retention manifest size")
    try:m=strict_json(raw.decode("utf-8"))
    except (json.JSONDecodeError,UnicodeDecodeError) as error:raise MissingComponent("native_retention_manifest_truncated_or_unreadable") from error
    fields(m,MANIFEST);native.validate_process(process);native.validate_process(m["process"])
    require(m["schema"]==MANIFEST_SCHEMA and m["kind"]=="capture_session" and m["capability"]==CAPABILITY,"native retention manifest schema/capability")
    require(any(hex_bytes(m["capture_id"],16,16)),"native retention capture id")
    config(m["config"],run,run_id,miner)
    require(m["config"]==expected_config and m["process"]==process and m["miner_id"]==miner,"native retention manifest actual process/config")
    require(absolute(m["config_path"])==absolute(str(config_path)) and Path(config_path).is_relative_to(Path(run)/"private"),"native retention actual config path")
    return m


def locator(value,m,manifest_raw):
    fields(value,LOCATOR)
    require(value=={"schema":LOCATOR_SCHEMA,"capability":CAPABILITY,"directory":m["config"]["directory"],
        "manifest_relative_path":session_name(m["process"])+"/manifest.json","manifest_sha256":sha(manifest_raw)},"native retention advertised locator mismatch/version")
    return deepcopy(value)


def handle(value,m,manifest_raw):
    fields(value,HANDLE)
    require(value["schema"]==HANDLE_SCHEMA and value["capability"]==CAPABILITY,"native retention event handle schema/capability")
    for k in ("manifest_sha256","record_sha256","request_digest"):hex_bytes(value[k],32,32)
    integer(value["receipt_sequence"],1,m["config"]["max_records"])
    prefix=session_name(m["process"])+"/"
    require(value["capture_id"]==m["capture_id"] and value["manifest_sha256"]==sha(manifest_raw)
            and value["manifest_relative_path"]==prefix+"manifest.json"
            and value["record_relative_path"]==prefix+value["request_digest"]+".json","native retention handle path/process/hash")
    return deepcopy(value)


def decode_record(raw,m,manifest_raw,filename,expected_handle=None):
    require(isinstance(raw,bytes) and 0<len(raw)<=MAX_RECORD,"native retention record size")
    if expected_handle is not None:
        handle(expected_handle,m,manifest_raw)
        require(sha(raw)==expected_handle["record_sha256"],"native retention event record hash")
    try:value=strict_json(raw.decode("utf-8"))
    except (json.JSONDecodeError,UnicodeDecodeError) as error:raise MissingComponent("native_retention_record_truncated_or_unreadable") from error
    fields(value,RECORD)
    require(value["schema"]==SCHEMA and value["kind"]=="approved_exchange" and type(value["version"]) is int
            and value["version"]==1 and value["activation"]=="not_asserted","native retention record schema/activation claim")
    require(value["capture_id"]==m["capture_id"] and value["manifest_sha256"]==sha(manifest_raw),"native retention record/manifest binding")
    integer(value["receipt_sequence"],1,m["config"]["max_records"]);integer(value["verified_at_unix_ms"],1,(1<<64)-1)
    for k in ("manifest_sha256","request_digest","request_sha256","response_sha256","checkpoint_digest"):hex_bytes(value[k],32,32)
    require(filename==value["request_digest"]+".json","native retention digest filename")
    request_raw=hex_bytes(value["request_json_hex"],native.MAX_REQUEST)
    response_raw=hex_bytes(value["response_json_hex"],native.MAX_RESPONSE)
    require(sha(request_raw)==value["request_sha256"] and sha(response_raw)==value["response_sha256"],"native retention exact payload hash")
    for payload in (request_raw,response_raw):
        try:payload.decode("utf-8")
        except UnicodeDecodeError as error:raise Invalid("native retention payload is not UTF8") from error
        # json.loads(bytes) also accepts UTF16/32; the producer accepts only
        # UTF8 JSON. Literal NUL bytes cannot occur in a valid UTF8 JSON text.
        require(b"\0" not in payload,"native retention payload is not UTF8 JSON")
    request=native.decode_frame(len(request_raw).to_bytes(4,"little")+request_raw,True)
    response=native.decode_frame(len(response_raw).to_bytes(4,"little")+response_raw,False)
    require(response["kind"]=="approved","native retention stored non-Approved response")
    fields(value["binding"],BINDING)
    require(canonical_json(value["binding"])==canonical_json({k:request[k] for k in BINDING}) and request["run_id"]==m["config"]["run_id"]
            and request["miner_id"]==m["miner_id"] and native.request_digest(request).hex()==value["request_digest"],"native retention request/run/epoch/config binding")
    if expected_handle is not None:
        require(expected_handle["request_digest"]==value["request_digest"] and expected_handle["receipt_sequence"]==value["receipt_sequence"],"native retention handle sequence/request")
    return value,request,response


class RetainedExchange:
    def __init__(self,record,request,response,proof,m,manifest_raw,raw,token):
        require(token is _TOKEN,"native retention verification required")
        self.record,self.request,self.response,self.proof=deepcopy((record,request,response,proof))
        self.manifest=deepcopy(m);self.manifest_raw=manifest_raw;self.raw=raw

    def join(self,event_handle,binding,candidate_wall_ms,commit_wall_ms):
        record,request,response=decode_record(self.raw,self.manifest,self.manifest_raw,self.record["request_digest"]+".json",event_handle)
        require(canonical_json((record,request,response))==canonical_json((self.record,self.request,self.response)),"native retention decoded exchange changed")
        require(event_handle["record_sha256"]==sha(self.raw) and event_handle["request_digest"]==self.record["request_digest"]
                and event_handle["receipt_sequence"]==self.record["receipt_sequence"],"native retention actual DurableHandle differs")
        require(self.manifest["process"]==binding["observed_miner"],"native retention observed process changed")
        now=self.record["verified_at_unix_ms"]
        require(candidate_wall_ms<=now<=commit_wall_ms<self.proof["expires_at_unix_ms"],"native retention candidate/verification/commit clock boundary")
        native.authorize(binding["registry"],binding["wallet"],self.request,binding["observed_miner"],commit_wall_ms)
        return deepcopy((self.request,self.response,self.proof))


def verify_exchange(raw,m,manifest_raw,filename,binding,expected_handle=None):
    record,request,response=decode_record(raw,m,manifest_raw,filename,expected_handle)
    require(m["process"]==binding["observed_miner"] and m["config_path"]==binding["route"]["miner_config_path"],"native retention peer process/config provenance")
    w=binding["wallet"];now=record["verified_at_unix_ms"]
    native.authorize(binding["registry"],w,request,binding["observed_miner"],now)
    require(request["connection_id"]==binding["config"]["id"] and array(request["connection_config_digest"],32)==native.connection_digest(binding["config"]),"native retention full actual connection digest")
    proof=native.verify_response(request,response,w["collector_pubkey"],w["pool_pubkey"],now)
    require(record["checkpoint_digest"]==proof["checkpoint_digest"],"native retention checkpoint digest")
    return RetainedExchange(record,request,response,proof,m,manifest_raw,raw,_TOKEN)


def require_sources(binary):
    """Source applicability only; compiled/whole-union admission is separate."""
    if not {"source_manifest","source_manifest_sha256"}<=set(binary):
        raise MissingComponent("native_public_evidence_durable_v1_source_binding_unavailable")
    raw=Path(binary["source_manifest"]).read_bytes()
    require(sha(raw)==binary["source_manifest_sha256"],"native retention source manifest seal")
    value=strict_json(raw);rows={r["path"]:r["sha256"] for r in value["files"]}
    require(len(rows)==len(value["files"]),"native retention duplicate source rows")
    if any(rows.get(p)!=digest for p,digest in SOURCE_ROWS.items()):
        raise MissingComponent("native_public_evidence_durable_v1_source_binding_unavailable")
    root=value.get("nix_src_root")
    if not isinstance(root,str):
        from runtime_profile import verified_source_root
        root,_=verified_source_root(binary,'miner');root=str(root)
    for path,digest in SOURCE_ROWS.items():
        require(sha((absolute(root)/path).read_bytes())==digest,"native retention source bytes changed")
    return {"rows":SOURCE_ROWS,"scope":"retention_source_applicability_not_compiled_admission"}


def _metadata(info,uid,directory):
    require(info.st_uid==uid and stat.S_IMODE(info.st_mode)==(0o700 if directory else 0o600)
            and (stat.S_ISDIR(info.st_mode) if directory else stat.S_ISREG(info.st_mode) and info.st_nlink==1),"native retention private ownership/type/links")


def _open_directory(path,private_root,uid):
    path=absolute(str(path));private_root=absolute(str(private_root))
    fd=os.open("/",os.O_RDONLY|os.O_DIRECTORY|os.O_CLOEXEC)
    traversed=Path("/")
    try:
        for part in path.parts[1:]:
            child=os.open(part,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW|os.O_CLOEXEC,dir_fd=fd)
            os.close(fd);fd=child;traversed/=part
            if traversed.is_relative_to(private_root):_metadata(os.fstat(fd),uid,True)
        return fd
    except BaseException:os.close(fd);raise


def _read_file(fd,name,uid,bound):
    require("/" not in name and name not in {".",".."},"native retention filename")
    opened=os.open(name,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK|os.O_CLOEXEC,dir_fd=fd)
    try:
        before=os.fstat(opened);_metadata(before,uid,False)
        require(0<=before.st_size<=bound,"native retention file over byte bound")
        with os.fdopen(opened,"rb",closefd=False) as stream:raw=stream.read(bound+1)
        after=os.fstat(opened);_metadata(after,uid,False)
        require((before.st_dev,before.st_ino,before.st_size,before.st_mtime_ns,before.st_ctime_ns)==
                (after.st_dev,after.st_ino,after.st_size,after.st_mtime_ns,after.st_ctime_ns) and len(raw)==before.st_size,"native retention file changed during read")
        named=os.stat(name,dir_fd=fd,follow_symlinks=False)
        require((named.st_dev,named.st_ino)==(after.st_dev,after.st_ino),"native retention filename replaced during read")
        return raw
    finally:os.close(opened)


def _names(fd,maximum):
    result=[]
    with os.scandir(fd) as scan:
        for entry in scan:
            if len(result)>=maximum:raise MissingComponent("native_retention_directory_census_over_budget")
            result.append(entry.name)
    return sorted(result)


class StoreReader:
    def __init__(self,run,run_id,miner,process,config_path,expected_config,retain):
        self.run=Path(run);self.run_id=run_id;self.miner=miner;self.process=deepcopy(process);self.config_path=str(config_path)
        self.config=deepcopy(config(expected_config,run,run_id,miner));self.retain=retain
        self.manifest_raw=None;self.records={};self.exchanges={};self.session_identity=None;self.root_identity=None;self.pending=[]

    def poll(self,advertised=None):
        uid=self.process["uid"];root=Path(self.config["directory"]);session=session_name(self.process)
        rootfd=_open_directory(root,self.run/"private",uid)
        try:
            roots=_names(rootfd,self.config["max_sessions"])
            require(all(re.fullmatch(r"process-[1-9][0-9]*-[1-9][0-9]*",name) for name in roots),"native retention unexpected session name")
            for name in roots:_metadata(os.stat(name,dir_fd=rootfd,follow_symlinks=False),uid,True)
            if session not in roots:raise MissingComponent("native_retention_actual_process_session_missing")
            fd=os.open(session,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW|os.O_CLOEXEC,dir_fd=rootfd)
            try:
                _metadata(os.fstat(fd),uid,True)
                identities=[(os.fstat(d).st_dev,os.fstat(d).st_ino) for d in (rootfd,fd)]
                if self.root_identity is not None:require(identities==[self.root_identity,self.session_identity],"native retention directory replaced")
                self.root_identity,self.session_identity=identities
                names=_names(fd,self.config["max_records"]+2)
                if set(self.records)-set(names):raise MissingComponent("native_retention_original_record_pruned")
                if "manifest.json" not in names:raise MissingComponent("native_retention_manifest_missing")
                if len(names)-1>self.config["max_records"]:raise MissingComponent("native_retention_record_count_over_budget")
                raw=_read_file(fd,"manifest.json",uid,MAX_MANIFEST)
                if self.manifest_raw is not None:require(raw==self.manifest_raw,"native retention manifest rewritten")
                m=manifest(raw,self.run,self.run_id,self.miner,self.process,self.config_path,self.config)
                if advertised is not None:locator(advertised,m,raw)
                total=len(raw);records={};sequences=set();pending=[]
                for name in names:
                    if name=="manifest.json":continue
                    if re.fullmatch(r"\.[0-9a-f]{64}-[1-9][0-9]*\.pending",name):
                        stage=_read_file(fd,name,uid,MAX_RECORD);total+=len(stage);pending.append(name);continue
                    require(re.fullmatch(r"[0-9a-f]{64}\.json",name) is not None,"native retention unexpected record path")
                    data=_read_file(fd,name,uid,min(MAX_RECORD,self.config["max_bytes"]-total))
                    if name in self.records and len(data)<len(self.records[name]):raise MissingComponent("native_retention_original_record_truncated")
                    total+=len(data)
                    require(total<=self.config["max_bytes"],"native retention total byte budget")
                    value,_,_=decode_record(data,m,raw,name)
                    require(value["receipt_sequence"] not in sequences,"native retention duplicate receipt sequence")
                    sequences.add(value["receipt_sequence"]);records[name]=data
                require(len(records)+len(pending)<=self.config["max_records"] and total<=self.config["max_bytes"],"native retention count/byte budget")
                if names!=_names(fd,self.config["max_records"]+2) or roots!=_names(rootfd,self.config["max_sessions"]):
                    raise MissingComponent("native_retention_census_changed_during_read")
                checkroot=_open_directory(root,self.run/"private",uid)
                try:
                    checksession=os.open(session,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW|os.O_CLOEXEC,dir_fd=checkroot)
                    try:require([(os.fstat(d).st_dev,os.fstat(d).st_ino) for d in (checkroot,checksession)]==identities,"native retention directories changed during read")
                    finally:os.close(checksession)
                finally:os.close(checkroot)
                require(all(name in records and records[name]==data for name,data in self.records.items()),"native retention original record missing/changed")
                if self.manifest_raw is None:self.retain(root/session/"manifest.json",raw)
                for name,data in records.items():
                    if name not in self.records:self.retain(root/session/name,data)
                self.manifest_raw=raw;self.manifest=m;self.records=records;self.pending=pending
                return {"manifest_sha256":sha(raw),"records":len(records),"bytes":total,"pending_files":pending,"locator_observed":advertised is not None}
            finally:os.close(fd)
        finally:os.close(rootfd)

    def outer(self,bindings):
        if self.manifest_raw is None:raise MissingComponent("native_retention_manifest_not_captured")
        result={}
        for name,raw in self.records.items():
            record,_,_=decode_record(raw,self.manifest,self.manifest_raw,name)
            key=(self.miner,record["binding"]["connection_id"])
            if key not in bindings:raise MissingComponent("native_retention_actual_historical_connection_binding")
            digest=record["request_digest"]
            if digest not in self.exchanges:self.exchanges[digest]=verify_exchange(raw,self.manifest,self.manifest_raw,name,bindings[key])
            result[digest]=self.exchanges[digest]
        return result
