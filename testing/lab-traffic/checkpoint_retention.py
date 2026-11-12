"""Owned FIFO checkpoint retention and the pre-activation provider-cache hook.

Source notifications schedule reads only. Exact admission stays in the existing
row/parent/history oracle; absent historical revisions are never synthesized.
"""
import hashlib
import os
from pathlib import Path
import queue
import re
import threading
import time

from case_contract import canonical_json
from economic_checkpoint import query
from oracle import Inconclusive, Invalid, integer, require, strict_json
from pool_source import ANSI, decimal_uint
from ruleb_sql import replay_checkpoint, sql_bytes
from source_jobs import MissingComponent, parent_view

CONTRACT_SHA256="45178f83c4037a5651bd778b952756997762842c88b15d1bbea36ac1e5cce221"
TRIGGERS={"distribution_frozen","job_issued","economic_change_observed","source_publication","pending_applied","derivation_observed"}


class CheckpointRetention:
    def __init__(self,root,run_id,front_pools,capture_sql,source_contract,*,capacity=4096,sql_contract=None):
        self.root=Path(root);self.root.mkdir(mode=0o700,exist_ok=False)
        self.run_id,self.front_pools,self.capture_sql,self.contract=run_id,dict(front_pools),capture_sql,source_contract
        self.sql_contract=source_contract if sql_contract is None else sql_contract
        self.queue=queue.Queue(maxsize=capacity);self.lock=threading.RLock();self.closed=False;self.closing=False;self.records=[];self.failures=[];self.poisoned=False
        self.sequence=0;self.previous="00"*32
        self.journal=(self.root/"attempts.jsonl").open("xb");(self.root/"attempts.jsonl").chmod(0o600)
        self.worker=threading.Thread(target=self._run,name="checkpoint-retention",daemon=True);self.worker.start()

    def _append(self,kind,value):
        with self.lock:
            require(not self.poisoned,"checkpoint retention journal poisoned")
            core={"seq":self.sequence+1,"previous":self.previous,"kind":kind,"monotonic_ns":time.monotonic_ns(),"value":value}
            digest=hashlib.sha256(canonical_json(core)).hexdigest();raw=canonical_json({**core,"digest":digest})+b"\n"
            try:self.journal.write(raw);self.journal.flush();os.fsync(self.journal.fileno())
            except OSError:self.poisoned=True;raise
            self.sequence+=1;self.previous=digest
            return digest

    def fail(self,kind,detail):
        with self.lock:
            failure={"kind":kind,"detail":str(detail)};self.failures.append(failure)
            self._append("failure",failure)

    def retain(self,front,artifact,trigger,*,expected_observation=None):
        require(front in self.front_pools,"checkpoint front outside configured pool mapping")
        path=Path(artifact["path"]);raw=path.read_bytes()
        require(hashlib.sha256(raw).hexdigest()==artifact["sha256"],"checkpoint original capture seal changed")
        envelope=strict_json(raw)
        require(envelope["schema"]=="spc-lab-bound-sql-capture/1"
                and envelope["binding"]["run_id"]==self.run_id and envelope["binding"]["pool_id"]==self.front_pools[front]
                and envelope["execution"]["exit_code"]==0 and type(envelope["execution"]["exit_code"]) is int
                and envelope["execution"]["timed_out"] is False
                and envelope["query_sha256"]==hashlib.sha256(query(source_contract=self.sql_contract).encode()).hexdigest(),"checkpoint capture run/pool/query/execution binding")
        observation=envelope["observation"];canonical=canonical_json(observation)
        if expected_observation is not None:
            require(canonical==canonical_json(expected_observation),"provider cached checkpoint differs from its original capture bytes")
        states=observation["tables"]["economic_state"];require(len(states)==1,"retained economic singleton")
        state=states[0];revision=integer(state["revision"]);anchor=state["anchor_hash"]
        if anchor is not None:sql_bytes(anchor,32)
        digest=hashlib.sha256(canonical).hexdigest();destination=self.root/(digest+".json")
        with self.lock:
            require(not self.closed and not self.poisoned,"checkpoint retention is closed/unavailable")
            if destination.exists():require(not destination.is_symlink() and destination.read_bytes()==canonical,"immutable checkpoint collision/change")
            else:
                fd=os.open(destination,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)
                with os.fdopen(fd,"wb") as stream:stream.write(canonical);stream.flush();os.fsync(stream.fileno())
                directory=os.open(self.root,os.O_RDONLY|os.O_DIRECTORY)
                try:os.fsync(directory)
                finally:os.close(directory)
            record={"front":front,"pool_id":self.front_pools[front],"path":str(destination),"sha256":digest,"original_artifact":dict(artifact),
                "revision":revision,"anchor_hash":anchor,"history_watermark":observation["history_boundary"]["event_watermark"],
                "trigger":strict_json(canonical_json(trigger)),"retained_wall_ns":time.time_ns()}
            self._append("retained",record);self.records.append(record)
            return dict(record)

    def _run(self):
        while True:
            item=self.queue.get()
            if item is None:self.queue.task_done();return
            front,trigger,done,answer=item
            try:answer.append(self.retain(front,self.capture_sql(front,None),trigger))
            except Exception as error:
                try:self.fail("capture_unavailable",type(error).__name__+": "+str(error))
                except Exception:self.poisoned=True
                answer.append(error)
            finally:self.queue.task_done();done.set()

    def request(self,front,trigger,*,wait=False,timeout=30):
        with self.lock:
            require(front in self.front_pools and not self.closed and not self.closing and not self.poisoned,"checkpoint request unavailable")
            trigger=strict_json(canonical_json(trigger));done=threading.Event();answer=[]
            self._append("requested",{"front":front,"trigger":trigger})
            try:self.queue.put_nowait((front,trigger,done,answer))
            except queue.Full:
                self.fail("capture_queue_overflow",front);raise MissingComponent("checkpoint_capture_queue_overflow")
        if not wait:return
        if not done.wait(timeout):
            self.fail("capture_barrier_deadline",front)
            raise MissingComponent("checkpoint_capture_barrier_deadline")
        if isinstance(answer[0],Exception):raise answer[0]
        return answer[0]

    def notify(self,front,raw_lines):
        # These scheduling hints can precede full signature verification. They
        # carry no monetary authority; final coverage requires the verified log.
        try:
            for line in raw_lines.decode().splitlines():
                text=ANSI.sub("",line)
                if "spc_evidence" not in text:continue
                match=re.search(r'event="([a-z_]+)"\s+evidence=',text)
                if match is None:raise Inconclusive("source notification shape")
                if match[1] not in TRIGGERS:continue
                payload=strict_json(text[match.end():]);source=payload["source"];decimal_uint(source["seq"])
                self.request(front,{"kind":match[1],"session":source["session"],"source_seq":source["seq"]})
        except Exception as error:self.fail("notification_unavailable",type(error).__name__+": "+str(error))

    def barrier(self,front,timeout=30):
        return self.request(front,{"kind":"barrier"},wait=True,timeout=timeout)

    def require_healthy(self):
        with self.lock:
            if self.poisoned or self.closed or self.closing or any(f["kind"]!="capture_unavailable" for f in self.failures):
                raise MissingComponent("checkpoint_capture_reader_queue_or_journal_incomplete")

    def observations(self,front):
        with self.lock:records=[dict(r) for r in self.records if r["front"]==front]
        seen=set()
        for record in records:
            original=record["original_artifact"]
            require(hashlib.sha256(Path(original["path"]).read_bytes()).hexdigest()==original["sha256"],"original retained SQL artifact changed")
            if record["sha256"] in seen:continue
            seen.add(record["sha256"]);raw=Path(record["path"]).read_bytes()
            require(hashlib.sha256(raw).hexdigest()==record["sha256"],"retained checkpoint changed")
            yield record,strict_json(raw)

    def coverage(self,sources,views,migrations,source_issues=None):
        resolved=[];unavailable=[{"front":front,"reason":"source reader/prefix incomplete","detail":detail} for front,detail in (source_issues or {}).items()]
        for front in self.front_pools:
            if front not in sources:unavailable.append({"front":front,"reason":"signed source absent"});continue
            source=sources[front];observations=list(self.observations(front))
            if not source.jobs:unavailable.append({"front":front,"reason":"no frozen source jobs"})
            if any(e["kind"]=="proof_unavailable" for e in source.events):unavailable.append({"front":front,"reason":"source reports unavailable proof"})
            for key in sorted(set(source.jobs)|set(source.issued)):
                item={"front":front,"job_key":list(key),"job_issued_seqs":[int(e["source"]["seq"]) for e in source.issued.get(key,[])]}
                if key not in source.jobs:unavailable.append({**item,"reason":"advertised job lacks frozen source"});continue
                p=source.jobs[key]["payload"];economic=p.get("economic");item["distribution_frozen_seq"]=int(source.jobs[key]["source"]["seq"])
                if economic is None:unavailable.append({**item,"reason":"frozen economics absent"});continue
                item["revision"]=economic["revision"];errors=[]
                for record,observation in observations:
                    if record["revision"]!=economic["revision"]:continue
                    try:
                        result=replay_checkpoint(observation,economic,parent_view(p,views),source_contract=source.source_contract,migration_checksums=migrations)
                    except (Inconclusive,Invalid,KeyError,ValueError) as error:errors.append(str(error));continue
                    resolved.append({**item,"checkpoint":record,"history_watermark":result["history_watermark"]});break
                else:unavailable.append({**item,"reason":"exact coherent checkpoint unavailable","attempt_errors":errors})
        fatal=[f for f in self.failures if f["kind"]!="capture_unavailable"]
        blocked=bool(unavailable or fatal or self.poisoned or self.queue.unfinished_tasks)
        return {"status":"CERTIFICATION_BLOCKED" if blocked else "COMPLETE_CAPTURE","resolved":resolved,"unavailable":unavailable,
            "capture_failures":list(self.failures),"pending_reads":self.queue.unfinished_tasks,"journal_poisoned":self.poisoned,"later_state_reconstruction":False}

    def close(self,timeout=30,*,capture_barrier=True):
        if self.closed:return
        error=None
        try:
            if capture_barrier:
                for front in self.front_pools:self.barrier(front,timeout)
        except Exception as failure:error=failure
        with self.lock:self.closing=True
        try:self.queue.put(None,timeout=timeout)
        except queue.Full:raise MissingComponent("checkpoint_capture_close_queue_deadline")
        self.worker.join(timeout)
        if self.worker.is_alive():raise MissingComponent("checkpoint_capture_worker_did_not_stop")
        self.closed=True
        self.journal.close()
        if error:raise error


def retain_provider_checkpoint(capture,provider,front,key):
    """Exact source-bound hook, before a prepared bundle/ACK can escape."""
    capture.require_healthy()
    provider.capture_snapshot(front,key)
    slot=(front,key)
    if slot not in provider.snapshots or slot not in provider.snapshot_refs:
        raise MissingComponent("original_admitted_provider_checkpoint_artifact")
    observation=provider.snapshots[slot];job=provider.capture.sources[front].jobs[key]["payload"]
    record=capture.retain(front,provider.snapshot_refs[slot],{"kind":"provider_checkpoint_admitted","front":front,"job_key":list(key)},expected_observation=observation)
    require(record["front"]==front and record["revision"]==job["economic"]["revision"]
            and sql_bytes(record["anchor_hash"],32).hex()==job["prev_hash"]
            and record["history_watermark"]==observation["history_boundary"]["event_watermark"]
            and record["sha256"]==hashlib.sha256(canonical_json(observation)).hexdigest(),"provider checkpoint hook metadata mismatch")
    capture.require_healthy()
    return record
