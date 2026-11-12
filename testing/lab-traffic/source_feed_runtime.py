"""Production-controller integration for owned per-wallet collector sockets.

Consumes existing rendered source_audit config; does not edit it, switch to
SV1, spawn extra wallets, or put scan secrets in the CPU miner.
"""
import copy
import hashlib
import json
import os
from pathlib import Path
import signal
import stat
import threading
import time

from case_contract import canonical_json
from oracle import Inconclusive, require, strict_json
from source_feed_codec import CONTRACT, CollectorKey, tagged
from source_feed_provider import ReplayProvider
from source_feed_server import FeedServer
from source_jobs import MissingComponent
from storage import collect_database, option, private_bytes, sha_file


def config_fields(raw):
    section=None;result={}
    for line in raw.decode().splitlines():
        line=line.strip()
        if line.startswith("["):section=line
        elif section=="[ark.native_audit]" and line and not line.startswith("#"):
            name,value=line.split("=",1);name=name.strip()
            require(name not in result,"duplicate rendered source_audit key")
            result[name]=strict_json(value.strip())
    if not {"collector_socket","pool_pubkey","collector_pubkey"}<=set(result):
        raise MissingComponent("rendered_existing_wallet_native_audit_config")
    # These exact fields are used by the source-bound controller/AP3 contract.
    # Other service scope fields are checked against the published instance.
    return {"socket":result["collector_socket"],"pool_pubkey":result["pool_pubkey"],"collector_pubkey":result["collector_pubkey"]}


def public_identity(address):
    require(isinstance(address,str) and address==address.lower(),"canonical silent address required")
    hrp,encoded=address.rsplit("1",1);alphabet="qpzry9x8gf2tvdw0s3jn54khce6mua7l"
    require(hrp=="tsp" and all(c in alphabet for c in encoded),"test-network silent address encoding")
    values=[alphabet.index(c) for c in encoded];check=1
    for value in [ord(c)>>5 for c in hrp]+[0]+[ord(c)&31 for c in hrp]+values:
        top=check>>25;check=((check&0x1ffffff)<<5)^value
        for i,g in enumerate((0x3b6a57b2,0x26508e6d,0x1ea119fa,0x3d4233dd,0x2a1462b3)):
            if top>>i&1:check^=g
    require(check==0x2bc830a3 and values[0]==0,"silent address checksum/version")
    acc=bits=0;raw=bytearray()
    for value in values[1:-6]:
        acc=(acc<<5)|value;bits+=5
        while bits>=8:
            bits-=8;raw.append((acc>>bits)&255)
    require(bits<5 and acc&((1<<bits)-1)==0 and len(raw)==66,"silent address padding/program")
    from signature_oracle import public_point
    public_point(bytes(raw[:33]));public_point(bytes(raw[33:]))
    return bytes(raw[1:33]).hex(),tagged("SPc/Id",bytes(raw)).hex()


def collector_socket_path(value,runtime_private):
    """Match the approved wallet renderer's owned runtime/private boundary."""
    path=Path(value);root=Path(runtime_private)
    require(path.is_absolute() and root.is_absolute() and path.is_relative_to(root)
            and path.resolve()==path and root.resolve()==root and path!=root,
            "collector socket outside canonical runtime private tree")
    require(not os.path.lexists(path),"collector socket already occupied")
    parent=path.parent
    while True:
        info=parent.lstat()
        require(stat.S_ISDIR(info.st_mode) and not parent.is_symlink() and info.st_uid==os.getuid()
                and stat.S_IMODE(info.st_mode)==0o700,"collector parent is not owned private renderer directory")
        if parent==root:break
        parent=parent.parent
    return path


class CollectorRuntime:
    def __init__(self,controller):
        self.controller=controller;self.stop_event=threading.Event();self.threads=[];self.servers=[];self.paths=[]
        self.serial=0;self.lock=threading.RLock()
        self.source_stop=threading.Event();self.source_reader=None;self.retention=None;self.provider=None;self.feed_quiet=False;self.closed=False
        self.finalize_lock=threading.Lock();self.drain_attempted=False;self.drain_result=None
        self.final_boundary_ref=None
        self.sql_contract=CONTRACT

    def capture_sql(self,front,key):
        c=self.controller
        pool=next(f["pool_id"] for f in c.scenario["fronts"] if f["id"]==front)
        database=next(d["id"] for d in c.manifest()["resources"]["postgres_databases"] if d["owner_pool_id"]==pool)
        with self.lock:
            self.serial+=1;path=c.priv/("source-sql-"+str(self.serial)+".json")
        result=collect_database(c.run,database,c.binding["binaries"]["psql"],c.binding["binaries"]["traffic-probe"],
            c.binding["binaries"]["postgres"],path,c.capture.add,query_contract=self.sql_contract)
        return {"path":str(path),"sha256":result["capture_sha256"]}

    def start(self):
        c=self.controller;settings=c.binding["source_feed"]
        require(settings["contract"]==CONTRACT,"collector runtime contract")
        self.sql_contract=settings.get('sql_contract',CONTRACT)
        if self.sql_contract!=CONTRACT:
            from decision_observations import CONTRACT as DECISIONS, require_source
            require(self.sql_contract==DECISIONS,'unsupported decision observation collector contract')
            require_source(c.binding['binaries']['pool'])
        from checkpoint_retention import CheckpointRetention
        front_pools={f["id"]:f["pool_id"] for f in c.scenario["fronts"]}
        self.retention=CheckpointRetention(c.priv/"checkpoints",c.run_id,front_pools,self.capture_sql,CONTRACT,sql_contract=self.sql_contract)
        c.source_capture.attach_checkpoints(self.retention)
        def watch_source():
            while not self.source_stop.is_set():
                try:
                    manifest=c.manifest()
                    if manifest is not None:c.source_capture.poll(manifest,c.views,materialize=False)
                except Exception as error:
                    self.retention.fail("source_reader_unavailable",type(error).__name__+": "+str(error));return
                self.source_stop.wait(0.05)
        self.source_reader=threading.Thread(target=watch_source,name="source-checkpoint-watch",daemon=True);self.source_reader.start()
        agents=settings["agents"];manifest=c.manifest()
        require(len(agents)==4 and len(c.scenario["miners"])==4,"four wallet/miner source-feed bindings required")
        if len([p for p in manifest["processes"] if p["role"]=="wallet"])!=4:
            raise MissingComponent("actual_four_wallet_verifier_process_roster")
        signer=CollectorKey(settings["collector_key_file"],settings["collector_pubkey"])
        actual=strict_json(private_bytes(c.run/"private/scenario.json",c.run))
        actual_identities={i["id"]:i["address_source"] for i in actual["identities"]}
        bound={};endpoints={}
        for identity,row in agents.items():
            wallets=[p for p in manifest["processes"] if p["role"]=="wallet" and p["entity"]==row["wallet_entity"]]
            require(len(wallets)==1,"source verifier wallet process identity")
            process=wallets[0]
            expected=c.binding["binaries"]["agent"]
            require(process["executable_sha256"]==expected["sha256"]
                    and Path(process["executable_path"]).resolve()==Path(expected["path"]).resolve()
                    and sha_file(f"/proc/{process['pid']}/exe")==expected["sha256"],"source verifier must be the actual sealed agent binary")
            info=os.stat(f"/proc/{process['pid']}/exe")
            executable_identity=[info.st_dev,info.st_ino,info.st_size,info.st_mtime_ns,info.st_ctime_ns]
            env=Path(f"/proc/{process['pid']}/environ").read_bytes().split(b"\0")
            require([v.split(b"=",1)[1] for v in env if v.startswith(b"SPC_LAB_RUN_TOKEN=")]==[process["run_token"].encode()],"source verifier process ownership token")
            config=config_fields(private_bytes(Path(option(process["args"],"--config")),c.run))
            require(config["collector_pubkey"]==signer.public_key and config["pool_pubkey"]==settings["pool_pubkeys"][row["front"]],"rendered source authority keys differ from pinned keys")
            # The owned rendered scenario provides the public identity binding,
            # independently of a job/trace. Optional parent pins are crosschecked;
            # no scan secret key file is read by the collector.
            require(identity in {i["id"] for i in c.scenario["identities"]},"source wallet identity outside declared roster")
            require(set(actual_identities[identity])=={"explicit"},"actual wallet public identity missing")
            scan,sp_id=public_identity(actual_identities[identity]["explicit"])
            require(("scan_pubkey" not in row or scan==row["scan_pubkey"]) and ("sp_id" not in row or sp_id==row["sp_id"]),
                    "source ACK key/identity differs from actual provisioned wallet")
            miners=[m for m in c.scenario["miners"] if m["id"]==row["miner_id"]]
            require(len(miners)==1 and any(con["identity_id"]==identity and con["front_id"]==row["front"] for con in miners[0]["connections"]),"source wallet/miner/front routing differs from scenario")
            path=collector_socket_path(config["socket"],c.run/"private")
            require(not (path.parent/"feed-journal.jsonl").exists(),"fresh campaign must not adopt a collector journal")
            endpoints[identity]=path
            bound[identity]={"scan_pubkey":scan,"pool_pubkey":config["pool_pubkey"],"front":row["front"],
                             "sp_id":sp_id,"pid":process["pid"],"start_time_ticks":process["start_time_ticks"],
                             "run_id":c.run_id,"miner_id":row["miner_id"],"executable_identity":executable_identity,
                             "executable_sha256":expected["sha256"]}
        checksums={int(k):v for k,v in settings["migration_checksums"].items()}
        provider=ReplayProvider(c.source_capture,c.views,bound,self.capture_sql,c.common_chain,checksums,self.retention)
        self.provider=provider;self.migrations=checksums
        servers={}
        for parent in {path.parent for path in endpoints.values()}:
            group={identity:bound[identity] for identity,path in endpoints.items() if path.parent==parent}
            server=FeedServer(parent,signer,group,provider);servers[parent]=server;self.servers.append(server)
        for identity,path in endpoints.items():
            server=servers[path.parent];self.paths.append(path)
            def serve(server=server,identity=identity,path=path):
                try:server.serve(identity,self.stop_event,path)
                except Exception as error:c.capture.add("source_feed_server_failure",{"agent":identity,"class":type(error).__name__})
            worker=threading.Thread(target=serve,name="source-feed-"+identity,daemon=True)
            self.threads.append(worker);worker.start()
        c.capture.add("source_feed_started",{"sockets":[str(p) for p in self.paths],"agents":bound,
            "collector_pubkey":signer.public_key,"additional_wallet_or_miner_processes":0,
            "signing_scope":"collector_attestations_only","native_activation_verified":False})

    def audit_activation(self):
        if self.controller.native_capture is None:raise MissingComponent("actual_native_observer_not_registered")
        return self.controller.native_capture.audit(self.servers)

    def audit_payout(self):
        import tomllib
        from payout_history import audit_pool
        from native_capture import config_scalar
        c=self.controller
        if c.common_chain() is None:raise MissingComponent("fullrail_actual_common_raw_chain")
        c.source_capture.poll(c.manifest(),c.views,materialize=False)
        self.retention.require_healthy();results={}
        for pool in c.scenario["pools"]:
            fronts=[f["id"] for f in c.scenario["fronts"] if f["pool_id"]==pool["id"]]
            record=self.retention.barrier(fronts[0]);raw=Path(record["path"]).read_bytes()
            require(hashlib.sha256(raw).hexdigest()==record["sha256"],"fullrail final original SQL seal")
            with c.source_capture.lock:
                if any(f in c.source_capture.issues or f not in c.source_capture.sources for f in fronts):
                    raise MissingComponent("fullrail_complete_signed_source_roster")
                sources={f:c.source_capture.sources[f] for f in fronts}
            observations={f:list(self.retention.observations(f)) for f in fronts}
            configs=[private_bytes(c.run/"private"/("pool-"+f+".toml"),c.run) for f in fronts]
            descriptors={config_scalar(value,"pool","pool_payout_descriptor") for value in configs}
            require(len(descriptors)==1,"fullrail pool descriptor differs across fronts")
            order_keys={tomllib.loads(value.decode()).get("ark",{}).get("order_pubkey") for value in configs}
            require(len(order_keys)==1,"fullrail Ark order key differs across fronts")
            result=audit_pool(sources,observations,strict_json(raw),dict(c.views),c.policy,pool["work_profile"],self.migrations,
                descriptor=next(iter(descriptors)),order_pubkey=next(iter(order_keys)))
            result["final_checkpoint"]=record
            result["public_config_sha256"]=[hashlib.sha256(value).hexdigest() for value in configs]
            results[pool["id"]]=result;c.capture.add("independent_payout_history",{"pool_id":pool["id"],"result":result})
        self.retention.require_healthy()
        if any(result["status"]!="PASSED" for result in results.values()):
            raise MissingComponent("fullrail_historical_facts_unproved_see_independent_payout_history")
        return results

    def quiesce_feed(self):
        if self.feed_quiet:return
        if self.provider is not None:
            # Cancellation must not wait behind a proof's SQL/replay lock.
            # prepare/_context also fence the flag before releasing evidence.
            self.provider.accepting=False
        self.stop_event.set()
        deadline=time.monotonic()+35
        for worker in self.threads:worker.join(max(0,deadline-time.monotonic()))
        if any(worker.is_alive() for worker in self.threads) or any(p.exists() for p in self.paths):
            raise Inconclusive("collector workers/listeners not completely stopped")
        self.feed_quiet=True

    def _stop_mining(self):
        c=self.controller
        from cleanup_proof import identity_observation
        records=[r for r in c.manifest()["processes"] if r["role"]=="miner"]
        require({r["entity"] for r in records}=={m["id"] for m in c.scenario["miners"]},"drain miner process census")
        for miner in c.scenario["miners"]:
            if c.native_capture is not None:c.native_capture.expect_miner_stop(miner["id"])
            c.command("drain-remove-"+miner["id"],{"type":"remove_miner","miner_id":miner["id"]},timeout=10)
        def exited():
            observations=[identity_observation(r) for r in records]
            return observations if all(o["status"] in {"gone","surviving_zombie"} for o in observations) else None
        stopped=c.until(exited,15)
        self.quiesce_feed()
        c.capture.add("mining_drained",{"processes":stopped,"new_hash_authority":False})

    def _stop_pools(self):
        c=self.controller;manifest=c.manifest()
        from native_capture import observe_process
        from cleanup_proof import identity_observation
        records=[r for r in manifest["processes"] if r["role"] in {"front","pool-ark"}]
        require({r["entity"] for r in records if r["role"]=="front"}==set(c.source_capture.expected),"drain pool/source census")
        if not hasattr(os,"pidfd_open") or not hasattr(signal,"pidfd_send_signal"):
            raise MissingComponent("pidfd_bound_pool_shutdown")
        for record in records:
            fd=os.pidfd_open(record["pid"])
            try:
                observed,path=observe_process(record,c.run,c.binding["binaries"]["pool"],manifest["locks"]["pid"])
                if record["role"]=="front":c.source_capture.expect_stop(record["entity"])
                c.capture.add("pool_drain_stop_intent",{"role":record["role"],"entity":record["entity"],"process":observed,"config_path":path,"signal":"SIGTERM"})
                signal.pidfd_send_signal(fd,signal.SIGTERM)
            finally:os.close(fd)
        def exited():
            observations=[identity_observation(r) for r in records]
            return observations if all(o["status"] in {"gone","surviving_zombie"} for o in observations) else None
        stopped=c.until(exited,15)
        for front in c.source_capture.expected:c.source_capture.mark_eof(front)
        c.source_capture.poll(c.manifest(),c.views,materialize=False)
        self.source_stop.set()
        if self.source_reader is not None:self.source_reader.join(15)
        if self.source_reader is not None and self.source_reader.is_alive():raise MissingComponent("pool_source_reader_EOF_not_drained")
        c.capture.add("pool_writers_drained",{"processes":stopped,"source_EOF":c.source_capture.eof,"no_forced_booking":True})

    def _drain_sample(self):
        c=self.controller
        from settlement_drain import convergence_gaps
        from economic_checkpoint import validate_initial, observation_layout
        from ruleb_sql import validate_observation, REDACTED
        import accounting_history
        chain=c.common_chain()
        if chain is None:raise MissingComponent("drain_common_raw_chain")
        self.retention.require_healthy();gaps=[];pools={};stable=[]
        try:c.source_capture.publications(c.views)
        except Inconclusive as error:gaps.append("raw/source publication join: "+str(error))
        for pool in c.scenario["pools"]:
            fronts=[f["id"] for f in c.scenario["fronts"] if f["pool_id"]==pool["id"]]
            record=self.retention.barrier(fronts[0]);observation=strict_json(Path(record["path"]).read_bytes())
            require(hashlib.sha256(canonical_json(observation)).hexdigest()==record["sha256"],"drain retained SQL changed")
            tables,keys,schema,query_id=observation_layout(observation)
            validate_observation(observation,tables,schema,query_id)
            seed=validate_initial(observation,migration_checksums=self.migrations,source_contract=CONTRACT)
            accounting_history._replay(observation,tables,keys,REDACTED,schema,initial_state=seed)
            sources=[]
            for front in fronts:
                if front in c.source_capture.issues or front not in c.source_capture.sources:gaps.append(front+": incomplete signed source")
                else:sources.append(c.source_capture.sources[front])
            local=convergence_gaps(chain,sources,observation,depth=6)
            gaps.extend(pool["id"]+": "+gap for gap in local)
            state=observation["tables"]["economic_state"][0]
            heads={front:c.source_capture.sources[front].heads for front in fronts if front in c.source_capture.sources}
            stable.append([pool["id"],state["revision"],observation["history_boundary"]["event_watermark"],heads])
            pools[pool["id"]]={"checkpoint":record,"source_heads":heads,"shallow_unbooked":[r["block_hash"] for r in observation["tables"]["economic_block"] if r["lifecycle"]=="unbooked"]}
        after=c.common_chain()
        if after is None or after.tip["hash"]!=chain.tip["hash"]:gaps.append("raw tip changed across SQL/source sample")
        for pool in c.scenario["pools"]:
            for front,heads in pools[pool["id"]]["source_heads"].items():
                if c.source_capture.sources[front].heads!=heads:gaps.append(front+": source changed across drain sample")
        value={"gaps":gaps,"stable_key":hashlib.sha256(canonical_json([chain.tip["hash"],stable])).hexdigest(),"tip":{"height":chain.tip["height"],"hash":chain.tip["hash"]},"pools":pools}
        c.capture.add("economic_drain_sample",value)
        return value

    def _checkpoint_coverage(self,phase):
        c=self.controller
        # Read the actual source tail before covering the frozen/advertised
        # census. A source notification schedules SQL; it is not admission.
        c.source_capture.poll(c.manifest(),c.views,materialize=False)
        for front in c.source_capture.expected:self.retention.barrier(front)
        with c.source_capture.lock:
            self.retention.require_healthy()
            coverage=self.retention.coverage(c.source_capture.sources,c.views,self.migrations,c.source_capture.issues)
            self.retention.require_healthy()
            c.capture.add("economic_checkpoint_coverage",{"phase":phase,"coverage":coverage,
                "source_heads":{front:source.heads for front,source in c.source_capture.sources.items()}})
        if coverage["status"]!="COMPLETE_CAPTURE":
            raise MissingComponent("exact_frozen_advertised_checkpoint_coverage_incomplete_"+phase)
        return coverage

    def _before_pool_stop(self,current):
        self._checkpoint_coverage("before_pool_stop")
        # Covering every historical job may take time. Recheck the boundary
        # while writers and readers are still available, not after teardown.
        after=self._drain_sample()
        if after["gaps"] or after["stable_key"]!=current["stable_key"]:
            raise MissingComponent("raw_source_SQL_changed_during_pre_stop_coverage")

    def final_drain(self):
        if not self.finalize_lock.acquire(blocking=False):
            raise MissingComponent("collector_finalization_already_active")
        try:
            if self.drain_result is not None:return copy.deepcopy(self.drain_result)
            if self.closed or self.drain_attempted:
                raise MissingComponent("collector_final_drain_cannot_restart")
            self.drain_attempted=True
            self.controller.capture.add("economic_final_drain_started",{"run_id":self.controller.run_id})
            try:self.drain_result=self._final_drain()
            except Exception as error:
                self.controller.capture.add("economic_final_drain_incomplete",{
                    "run_id":self.controller.run_id,"class":type(error).__name__,
                    "component":getattr(error,"component",None),"converged":False})
                raise
            return copy.deepcopy(self.drain_result)
        finally:self.finalize_lock.release()

    def _final_drain(self):
        c=self.controller
        from settlement_drain import drain
        coverage=None
        def final_sample():
            nonlocal coverage
            coverage=self._checkpoint_coverage("after_pool_EOF")
            # This is the last raw/source/SQL read, after potentially slow
            # coverage replay. drain checks its deadline before calling seal.
            return self._drain_sample()
        def seal(final):
            require(set(c.source_capture.eof)==set(c.source_capture.expected),"final source EOF census")
            self.retention.require_healthy()
            require(coverage is not None and coverage["status"]=="COMPLETE_CAPTURE","post-EOF coverage unavailable")
            chain_evidence=None;native_boundary=None;pool_configs={}
            if getattr(c,"canonical",None) is not None:
                chain_evidence=c.canonical.export(c.priv/"drain-chain-evidence.json",run_id=c.run_id,
                    binary_hashes={k:v["sha256"] for k,v in c.binding["binaries"].items()},
                    scenario_sha256=sha_file(c.priv/"scenario.json"),source_snapshot_sha256=c.binding["source_snapshot_sha256"])
                for pool in c.scenario["pools"]:
                    pool_configs[pool["id"]]=[sha_file(c.run/"private"/("pool-"+f["id"]+".toml")) for f in c.scenario["fronts"] if f["pool_id"]==pool["id"]]
            if getattr(c,"native_capture",None) is not None:
                try:native_boundary=c.native_capture.final_boundary()
                except Exception as error:c.capture.add("native_final_prefix_unavailable",{"class":type(error).__name__})
            journal=None
            if isinstance(getattr(self.retention,"root",None),Path):
                with self.retention.lock:
                    self.retention.require_healthy()
                    require(not self.retention.queue.unfinished_tasks,"final checkpoint queue not drained")
                    journal={"sequence":self.retention.sequence,"head":self.retention.previous,"records":len(self.retention.records),
                        "sha256":sha_file(self.retention.root/"attempts.jsonl")}
            summary={"schema":"spc-lab-owned-economic-drain/2","run_id":c.run_id,"status":"CONVERGED","tip":final["tip"],"stable_key":final["stable_key"],
                "chain_evidence":chain_evidence,"native_boundary":native_boundary,"checkpoint_journal":journal,"pool_config_sha256":pool_configs,
                "pools":final["pools"],"source_EOF":c.source_capture.eof,"checkpoint_coverage":coverage,"unbooked_liabilities_preserved":True,"scope":"capture and termination boundary, not whole entitlement/Ark approval"}
            if time.monotonic()>=seal_deadline:raise MissingComponent("final_drain_deadline_before_durable_seal")
            path=c.priv/"economic-final-boundary.json";raw=canonical_json(summary)+b"\n"
            with os.fdopen(os.open(path,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600),"wb") as stream:stream.write(raw);stream.flush();os.fsync(stream.fileno())
            ref={"path":str(path),"sha256":hashlib.sha256(raw).hexdigest(),"coverage_status":coverage["status"]}
            c.capture.add("economic_final_boundary",ref);self.final_boundary_ref=ref
            if coverage["status"]!="COMPLETE_CAPTURE":raise MissingComponent("exact_frozen_advertised_checkpoint_coverage_incomplete")
        remaining=min(60,max(0,c.deadline-time.monotonic()))
        if remaining<=0:raise MissingComponent("no_remaining_final_drain_deadline")
        seal_deadline=time.monotonic()+remaining
        final=drain(self._stop_mining,self._drain_sample,self._stop_pools,final_sample,seal,
            before_pool_stop=self._before_pool_stop,timeout=remaining)
        return {"status":"CONVERGED_COMPLETE_CAPTURE","tip":final["tip"],"stable_key":final["stable_key"],"scope":"capture and drain only"}

    def final_audit_after_drain(self):
        if not self.finalize_lock.acquire(blocking=False):raise MissingComponent("collector_finalization_already_active")
        try:
            if self.closed or self.drain_result is None:raise MissingComponent("final_audit_requires_successful_drain_before_close")
            from final_audit import assess
            return assess(self.controller,self)
        finally:self.finalize_lock.release()

    def close(self):
        if not self.finalize_lock.acquire(blocking=False):
            raise MissingComponent("collector_finalization_already_active")
        try:return self._close()
        finally:self.finalize_lock.release()

    def _close(self):
        if self.closed:return {"already_closed":True}
        errors=[]
        try:self.quiesce_feed()
        except Exception as error:errors.append(type(error).__name__)
        self.source_stop.set()
        if self.source_reader is not None:self.source_reader.join(35)
        if self.source_reader is not None and self.source_reader.is_alive():errors.append("source_checkpoint_reader_did_not_stop")
        if self.retention is not None:
            try:self.retention.close(35,capture_barrier=False)
            except Exception as error:errors.append(type(error).__name__)
        if not any(worker.is_alive() for worker in self.threads):
            for server in self.servers:
                try:server.journal.close()
                except OSError as error:errors.append(type(error).__name__)
        if errors:raise Inconclusive("collector/source/checkpoint close incomplete: "+",".join(errors))
        if any(server.journal.poisoned for server in self.servers):
            raise Inconclusive("collector journal durability failed; capture cannot be promoted")
        self.closed=True
        return {"sockets_closed":len(self.paths),"threads_stopped":len(self.threads)}
