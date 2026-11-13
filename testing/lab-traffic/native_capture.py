"""Read-only observer of actual native process logs and private lifecycle files."""
import hashlib
import os
from pathlib import Path
import stat
import time
import threading

from case_contract import canonical_json
from oracle import Inconclusive, Invalid, require, strict_json
from source_jobs import MissingComponent
from storage import ProcFS, confined, option, private_bytes, sha_file
from native_codec import (SOURCE_SHA256, AUDIT, array, fields, connection_digest, validate_process)
from native_observer import bind_connection, collector_entries, replay

SOURCE_ROWS={
    "crates/spc-scan/src/native_activation.rs":SOURCE_SHA256,
    "crates/spc-scan/src/lib.rs":"81bc3e10b76ffec453f5029bdefc1d45572b37e8c21402bca5658e1853bb496c",
    "crates/spc-scan/src/source_feed.rs":"612ec34eae7bb811cae966fe5e2fd8421502a0ff567d398a0dc8c5907ef4bfd7",
    "crates/spc-scan/src/source_audit.rs":"1100d99a9946fbb0d25b769a675047d1b6f30bcc42f520b004e1171b36ddb15a",
    "bins/spc-cpu-miner/src/config.rs":"0f19dea04832b44bc21ca37b043937fd7cd5a934ece7408e2f06059b3248c8ec",
    "bins/spc-cpu-miner/src/native_gate.rs":"6e884ec9703878b14f9d9d9ab22ddaf0e528208705f072b296addd5b9d3b5f93",
    "bins/spc-cpu-miner/src/connection.rs":"4de4560cdb40548b55a1f1af4632ddf15c6082713e0d1a3b6e65f7c715526f72",
    "bins/spc-cpu-miner/src/worker.rs":"0f995d049c1a861b9c999a25bd3d8d44a6fc023fed717e1e680bfdaca2095184",
    "bins/spc-cpu-miner/src/report.rs":"b3e047e5f96d42af3991b740582b917b46a52576d022c74b4b1edcdf5057ae3e",
    "crates/spc-lab/src/engine/native_audit.rs":"6c887a4504d3b8c0508dc37b2ea3d94b4403761ecafefde9babd421b4529302e",
    "crates/spc-lab/src/evidence.rs":"4f0c3f0bc57c2d1920436075914b8edff1889c447daf04ad2d68c1dc7b56fe6e",
    "crates/spc-lab/src/artifacts.rs":"49191720e120a5fe93bda793933e20ec9d08ec1d7e7652dd94af0c47bf441e0f",
    "crates/spc-lab/src/adapters.rs":"a15f84bf29cb20c7acfc357fe34ed3fa7b20e495d15732ac9815d6cd47bfbc61",
}


def config_scalar(raw,section,name):
    current=None;values=[]
    for line in raw.decode().splitlines():
        line=line.strip()
        if line.startswith("["):current=line
        elif current=="["+section+"]" and line and not line.startswith("#") and "=" in line:
            key,value=line.split("=",1)
            if key.strip()==name:values.append(strict_json(value.strip()))
    require(len(values)==1 and isinstance(values[0],str),"native rendered config scalar missing/ambiguous: "+section+"."+name)
    return values[0]


def require_audited_sources(binding):
    """Sealed source applicability, not compilation or funding authority."""
    if not binding.get("source_feed"):raise MissingComponent("full_native_campaign_source_feed_required")
    from source_feed_codec import CONTRACT
    require(binding["source_feed"]["contract"]==CONTRACT,"native campaign source-feed contract")
    from runtime_profile import verified_source_root
    roots=set();profile=None
    for role in ('lab','miner','agent'):
        if role not in binding['binaries']:raise MissingComponent('native_audited_complete_bundle_pending_base_diagnostic_bundle_not_applicable')
        try:root,profile=verified_source_root(binding['binaries'][role],role)
        except MissingComponent as error:raise MissingComponent('native_audited_complete_bundle_pending_base_diagnostic_bundle_not_applicable') from error
        roots.add(str(root))
    require(len(roots)==1,'native applications have different compiler sources')
    return {'source_rows':profile['critical_source_rows'],'verified_source_root':next(iter(roots)),
        'runtime_profile':profile['id'],'scope':'native_source_applicability_only','live_activation_proved':False}


def observe_process(record,root,binary,supervisor_pid):
    """Observe /proc independently; no ProcessIdentity supplied by a request."""
    pid=record["pid"];before=ProcFS().identity(pid);proc=Path(f"/proc/{pid}")
    require(before["start_time_ticks"]==record["start_time_ticks"] and before["ppid"]==supervisor_pid,"native supervised PID/start identity")
    require(record["executable_sha256"]==binary["sha256"] and Path(record["executable_path"]).resolve()==Path(binary["path"]).resolve()
            and sha_file(proc/"exe")==binary["sha256"],"native actual executable/source binding")
    config=confined(Path(option(record["args"],"--config")),root/"private")
    require(config.stat().st_nlink==1 and stat.S_IMODE(config.parent.stat().st_mode)==0o700,"native private config link/directory")
    args=[v.decode() for v in (proc/"cmdline").read_bytes().split(b"\0") if v]
    require(option(args,"--config")==str(config),"native actual argv/config binding")
    env=(proc/"environ").read_bytes().split(b"\0")
    tokens=[v.split(b"=",1)[1] for v in env if v.startswith(b"SPC_LAB_RUN_TOKEN=")]
    require(tokens==[record["run_token"].encode()],"native actual selected run token")
    raw=private_bytes(config,root);uid=proc.stat().st_uid
    result={"pid":pid,"uid":uid,"start_time_ticks":before["start_time_ticks"],"boot_id":Path("/proc/sys/kernel/random/boot_id").read_text().strip(),
        "executable_sha256":list(bytes.fromhex(binary["sha256"])),"run_token_sha256":list(hashlib.sha256(tokens[0]).digest()),
        "config_sha256":list(hashlib.sha256(raw).digest())}
    require(ProcFS().identity(pid)==before,"native process changed during observation");validate_process(result)
    return result,str(config)


class NativeCapture:
    def __init__(self,controller):
        self.c=controller;self.logs={};self.bindings={};self.issues={};self.processes={};self.files={};self.incarnations={}
        self.lock=threading.RLock()
        self.stopping_miners=set()
        self.retention_readers={};self.retention_reports={}
        require(not controller.run.exists(),"native observer must register before runtime startup")
        self.out=controller.priv/"native-source";self.out.mkdir(mode=0o700)
        controller.capture.add("native_capture_registered",{"miners":[m["id"] for m in controller.scenario["miners"]],
            "source_binding":"separately verified native_source_applicability result; historical SOURCE_ROWS are not live admission",
            "registered_monotonic_ns":time.monotonic_ns(),"authority":"observation_only"})

    def retain(self,path,raw):
        digest=hashlib.sha256(raw).hexdigest();out=self.out/(digest+".json")
        if not out.exists():
            with os.fdopen(os.open(out,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600),"wb") as f:f.write(raw);f.flush();os.fsync(f.fileno())
        self.files[str(path)]={"source_path":str(path),"path":str(out),"sha256":digest}
        return self.files[str(path)]

    def poll(self,manifest):
        with self.lock:self._poll(manifest)

    def _poll(self,manifest):
        for record in manifest["processes"]:
            if record["role"]!="miner":continue
            miner=record["entity"]
            if miner in self.stopping_miners:continue
            try:
                observed,config=observe_process(record,self.c.run,self.c.binding["binaries"]["miner"],manifest["locks"]["pid"])
                if miner in self.processes:require(self.processes[miner]==observed,"native restarted producer requires new capture")
                self.processes[miner]=observed
                controller_pid=manifest["locks"]["pid"]
                require(controller_pid==self.c.process.pid and sha_file(f"/proc/{controller_pid}/exe")==self.c.binding["binaries"]["lab"]["sha256"],"native controller executable binding")
                pipe=os.readlink(f"/proc/{record['pid']}/fd/1")
                require(pipe.startswith("pipe:[") and pipe in ProcFS().fd_targets(controller_pid),"native actual stdout/controller control pipe")
                self.incarnations[miner]=record["run_token"]+":"+str(record["start_time_ticks"])
                from native_retention import StoreReader, require_sources
                if miner not in self.retention_readers:
                    source=require_sources(self.c.binding["binaries"]["miner"])
                    import tomllib
                    config_raw=private_bytes(Path(config),self.c.run)
                    require(hashlib.sha256(config_raw).digest()==array(observed["config_sha256"],32),"native retention observed config changed")
                    settings=tomllib.loads(config_raw.decode()).get("native_evidence")
                    if settings is None:raise MissingComponent("actual_native_evidence_config_capability_missing")
                    self.retention_readers[miner]=StoreReader(self.c.run,self.c.run_id,miner,observed,config,settings,self.retain)
                    self.c.capture.add("native_retention_source_binding",{"miner_id":miner,**source})
                report=self.retention_readers[miner].poll()
                if self.retention_reports.get(miner)!=report:
                    self.c.capture.add("native_public_evidence_captured",{"miner_id":miner,**report})
                    self.retention_reports[miner]=report
                if not isinstance(self.issues.get(miner),Invalid):self.issues.pop(miner,None)
            except Exception as error:
                if not isinstance(self.issues.get(miner),Invalid):self.issues[miner]=error
        if not self.processes:return
        try:
            # Native stdout belongs to the controller pipe, not a miner log.
            # record_miner_event persists the exact payload in EvidenceStore.
            log=confined(self.c.run/"private/evidence.jsonl",self.c.run)
            require(str(log) in ProcFS().fd_targets(manifest["locks"]["pid"]),"native evidence writer descriptor")
            fd=os.open(log,os.O_RDONLY|os.O_NOFOLLOW)
            try:
                info=os.fstat(fd);require(stat.S_ISREG(info.st_mode) and info.st_size<=128*1024*1024,"native log type/size")
                with os.fdopen(fd,"rb",closefd=False) as f:raw=f.read(info.st_size)
            finally:os.close(fd)
            old=self.logs.get("controller")
            if old:require(old["inode"]==(info.st_dev,info.st_ino) and raw.startswith(old["raw"]),"native log replaced/pruned")
            require(len(raw)==info.st_size,"native log short read")
            if not old or raw!=old["raw"]:
                try:
                    destination=self.out/"controller-evidence.jsonl"
                    flags=os.O_WRONLY|os.O_NOFOLLOW|(os.O_APPEND if old else os.O_CREAT|os.O_EXCL)
                    with os.fdopen(os.open(destination,flags,0o600),"wb") as f:
                        f.write(raw[len(old["raw"]) if old else 0:]);f.flush();os.fsync(f.fileno())
                    ref={"source_path":str(log),"path":str(destination),"prefix_bytes":len(raw),"sha256":hashlib.sha256(raw).hexdigest()}
                    self.c.capture.add("native_controller_evidence_prefix",{"processes":self.processes,"time_basis":"miner wall time; controller ingestion monotonic time",**ref})
                except Exception as error:raise Invalid("native durable prefix retention failed") from error
            self.logs["controller"]={"raw":raw,"inode":(info.st_dev,info.st_ino),"path":str(log)}
            if not isinstance(self.issues.get("controller"),Invalid):self.issues.pop("controller",None)
        except Exception as error:
            if not isinstance(self.issues.get("controller"),Invalid):self.issues["controller"]=error

    def expect_miner_stop(self,miner):
        with self.lock:
            require(miner in self.processes,"native drain lacks the observed miner incarnation")
            self.stopping_miners.add(miner)

    def connection(self,miner,declared,base):
        with self.lock:return self._connection(miner,declared,base)

    def _connection(self,miner,declared,base):
        """Reconstruct only from actual leased values, then verify payload digest.

        Initial AddConnection bytes still have to match the separate traffic
        audit intent/result hash; this method does not authorize AddConnection.
        """
        c=self.c;manifest=c.manifest();self.poll(manifest)
        if self.issues:raise next(iter(self.issues.values()))
        private=c.run/"private";lease_path=private/"native-audit-leases.json"
        raw=private_bytes(lease_path,c.run);leases=strict_json(raw);self.retain(lease_path,raw)
        fields(leases,("schema","run_id","leases"))
        require(leases["schema"]=="spc.native-audit-leases/1" and leases["run_id"]==c.run_id,"native private lease run binding")
        lease=leases["leases"]["wallet."+declared["identity_id"]+".native_audit"];fields(lease,("instance","registry_path","registry_sha256"))
        w=lease["instance"];datadir=Path(w["socket_path"]).parent
        require(Path(w["socket_path"]).name=="native-audit.sock" and Path(lease["registry_path"])==datadir/"native-audit-peers.json","native fixed lifecycle paths")
        wallet_records=[r for r in manifest["processes"] if r["role"]=="wallet" and r["entity"]==declared["identity_id"]]
        require(len(wallet_records)==1,"native actual wallet roster")
        observed,config=observe_process(wallet_records[0],c.run,c.binding["binaries"]["agent"],manifest["locks"]["pid"])
        require(w["process"]==observed and w["config_path"]==config,"native actual wallet process/config")
        wallet_config=private_bytes(Path(config),c.run)
        for section,name,expected in (("upstream","sp_address",base["identity"]),("upstream","address",base["endpoint"]),
                ("upstream","authority_public_key",base["authority_public_key"]),("ark","datadir",str(datadir)),
                ("ark.native_audit","collector_pubkey",w["collector_pubkey"]),("ark.native_audit","pool_pubkey",w["pool_pubkey"])):
            require(config_scalar(wallet_config,section,name)==expected,"native actual wallet config/lifecycle mismatch")
        require(hashlib.sha256(wallet_config).digest()==array(observed["config_sha256"],32),"native wallet config changed after observation")
        for p in (datadir,):
            confined(p,private);info=p.lstat();require(stat.S_ISDIR(info.st_mode) and stat.S_IMODE(info.st_mode)==0o700 and info.st_uid==observed["uid"],"native private lifecycle directory")
        instance_path=datadir/"native-audit-instance.json";instance_raw=private_bytes(instance_path,c.run,16*1024)
        require(strict_json(instance_raw)==w,"native lease/wallet lifecycle mismatch");self.retain(instance_path,instance_raw)
        sock=confined(Path(w["socket_path"]),private).lstat()
        require(stat.S_ISSOCK(sock.st_mode) and stat.S_IMODE(sock.st_mode)==0o600 and sock.st_uid==observed["uid"]
                and (sock.st_dev,sock.st_ino)==(w["socket_device"],w["socket_inode"]),"native actual socket device/inode/owner")
        registry_path=Path(lease["registry_path"]);registry_raw=private_bytes(registry_path,c.run,1024*1024)
        require(hashlib.sha256(registry_raw).digest()==array(lease["registry_sha256"],32),"native atomic registry/lease digest")
        reg=strict_json(registry_raw);self.retain(registry_path,registry_raw)
        peers=[p for p in reg["peers"] if p["miner_id"]==miner and p["connection_id"]==declared["id"]]
        require(len(peers)==1,"native initial peer binding missing");peer=peers[0]
        a={"mode":"required","run_id":c.run_id,"miner_id":miner,"wallet_socket":w["socket_path"],"wallet_instance":w["instance_id"],
            "wallet_process":w["process"],"wallet_socket_device":w["socket_device"],"wallet_socket_inode":w["socket_inode"],
            "peer_generation":peer["generation"],"connection_config_digest":peer["connection_config_digest"],
            "collector_pubkey":w["collector_pubkey"],"pool_pubkey":w["pool_pubkey"]}
        front=next(f for f in c.scenario["fronts"] if f["id"]==declared["front_id"])
        route={"run_id":c.run_id,"miner_id":miner,"connection_id":declared["id"],"identity":base["identity"],"endpoint":base["endpoint"],
            "authority_public_key":base["authority_public_key"],"workers":base["workers"],"front_id":front["id"],"pool_id":front["pool_id"],
            "miner_config_path":peer["config_path"]}
        record=next(r for r in manifest["processes"] if r["role"]=="miner" and r["entity"]==miner)
        actual_miner,actual_path=observe_process(record,c.run,c.binding["binaries"]["miner"],manifest["locks"]["pid"])
        require(peer["config_path"]==actual_path,"native registered/observed miner config path")
        config={**base,"source_audit":a};bound=bind_connection(config,w,reg,actual_miner,route)
        key=(miner,declared["id"])
        if key in self.bindings and self.bindings[key]!=bound:raise MissingComponent("native_rebind_historical_registry_capture")
        self.bindings[key]=bound
        c.capture.add("native_connection_observed",{"miner_id":miner,"connection_id":declared["id"],"binding":bound,
            "lifecycle_files":list(self.files.values()),"socket_observed":True,"authority":"observation_only"})
        return config

    def audit(self,servers):
        with self.lock:return self._audit(servers)

    def _audit(self,servers):
        self.c.native_payloads();self.poll(self.c.manifest())
        if self.issues:raise next(iter(self.issues.values()))
        manifest=self.c.manifest()
        require(set(self.processes)=={m["id"] for m in self.c.scenario["miners"]}
                and len([p for p in manifest["processes"] if p["role"]=="wallet"])==4
                and len([p for p in manifest["processes"] if p["role"]=="miner"])==4,"native actual four-wallet/four-miner roster")
        from native_observer import controller_events
        if "controller" not in self.logs:raise MissingComponent("actual_native_controller_evidence")
        events=controller_events(self.logs["controller"]["raw"],self.c.run_id,self.incarnations)
        if set(self.retention_readers)!=set(self.processes):raise MissingComponent("actual_native_retention_reader_roster")
        outer={}
        for reader in self.retention_readers.values():
            reader.poll()
            if reader.pending:raise MissingComponent("native_retention_incomplete_staged_files")
            for digest,exchange in reader.outer(self.bindings).items():
                require(digest not in outer,"native retention cross-process duplicate request")
                outer[digest]=exchange
        for ref in self.files.values():
            require(sha_file(ref["path"])==ref["sha256"],"native retained QA artifact missing/changed")
        detail=replay(events,self.bindings,collector_entries(servers),outer,require_durable=True)
        detail["retained_exchanges"]=len(outer)
        detail["time_basis"]="miner wall time and controller ingestion monotonic time; original miner monotonic clock not retained by projection"
        self.c.capture.add("native_activation_component",detail)
        if detail["status"]!="PASSED":raise MissingComponent(";".join(detail["missing_components"]))
        return detail

    def final_boundary(self):
        """Freeze native evidence after miner stop; no claim of activation here."""
        with self.lock:
            require(self.stopping_miners==set(self.processes),"native final boundary before all declared miner stops")
            self.poll(self.c.manifest())
            if self.issues:raise next(iter(self.issues.values()))
            if "controller" not in self.logs:raise MissingComponent("native_final_controller_prefix_missing")
            raw=self.logs["controller"]["raw"]
            from native_observer import controller_events
            events=controller_events(raw,self.c.run_id,self.incarnations)
            ref=self.retain(self.c.run/"private/evidence.jsonl",raw)
            stores={}
            for miner,reader in self.retention_readers.items():
                reader.poll()
                stores[miner]={"manifest_sha256":hashlib.sha256(reader.manifest_raw).hexdigest(),
                    "records":{name:hashlib.sha256(value).hexdigest() for name,value in reader.records.items()}}
            bindings=[{"miner_id":m,"connection_id":con,"binding":value} for (m,con),value in sorted(self.bindings.items())]
            return {"schema":"spc-lab-native-final-prefix/1","run_id":self.c.run_id,"controller_prefix":ref,
                "miner_events_sha256":hashlib.sha256(canonical_json(events)).hexdigest(),"stores":stores,
                "bindings_sha256":hashlib.sha256(canonical_json(bindings)).hexdigest(),"incarnations":dict(self.incarnations)}

    def audit_after_drain(self,servers,boundary):
        """Only retained files/previous process observations, never live peers."""
        with self.lock:
            fields(boundary,"schema run_id controller_prefix miner_events_sha256 stores bindings_sha256 incarnations".split())
            require(boundary["schema"]=="spc-lab-native-final-prefix/1" and boundary["run_id"]==self.c.run_id
                and boundary["incarnations"]==self.incarnations,"native final prefix run/incarnation")
            ref=boundary["controller_prefix"];require(Path(ref["source_path"])==self.c.run/"private/evidence.jsonl","native final original log path")
            raw=private_bytes(Path(ref["path"]),self.out,128*1024*1024)
            require(hashlib.sha256(raw).hexdigest()==ref["sha256"],"native final retained prefix changed")
            current=private_bytes(self.c.run/"private/evidence.jsonl",self.c.run,128*1024*1024)
            require(current.startswith(raw),"native final original prefix rewritten")
            from native_observer import controller_events
            events=controller_events(raw,self.c.run_id,self.incarnations)
            require(hashlib.sha256(canonical_json(events)).hexdigest()==boundary["miner_events_sha256"],"native final event digest")
            if controller_events(current,self.c.run_id,self.incarnations)!=events:
                raise MissingComponent("late_native_event_after_sealed_drain_prefix")
            bindings=[{"miner_id":m,"connection_id":con,"binding":value} for (m,con),value in sorted(self.bindings.items())]
            require(hashlib.sha256(canonical_json(bindings)).hexdigest()==boundary["bindings_sha256"],"native final historical binding changed")
            require(set(boundary["stores"])==set(self.retention_readers)==set(self.processes),"native final store census")
            outer={}
            for miner,reader in self.retention_readers.items():
                reader.poll()
                observed={"manifest_sha256":hashlib.sha256(reader.manifest_raw).hexdigest(),
                    "records":{name:hashlib.sha256(value).hexdigest() for name,value in reader.records.items()}}
                require(observed==boundary["stores"][miner] and not reader.pending,"native final retained store changed/incomplete")
                for digest,value in reader.outer(self.bindings).items():
                    require(digest not in outer,"native final duplicate request");outer[digest]=value
            for ref in self.files.values():require(sha_file(ref["path"])==ref["sha256"],"native retained QA artifact changed")
            result=replay(events,self.bindings,collector_entries(servers),outer,require_durable=True)
            if result["status"]!="PASSED":raise MissingComponent(";".join(result["missing_components"]))
            return {**result,"final_miner_events_sha256":boundary["miner_events_sha256"]}
