"""Pre-start-registered, read-only collector for the actual owned front logs.

Retains every byte (including incomplete tails), independently seals each
observed prefix and writes actual source-derived JobProof components. No API
here authorizes mining or constructs a complete/trusted proof from missing data.
"""
import hashlib
import json
import os
from pathlib import Path
import stat
import threading
import time

from oracle import Inconclusive, Invalid, require, strict_json
from pool_source import PoolSource
from source_jobs import MissingComponent, verify_job, verify_publication, witness_republication, matches_coinbase_template
from storage import ProcFS, sha_file

MAX_LOG = 128 * 1024 * 1024


def full_window_evidence(source, views, policy, profile):
    """Existing full-Q contract only; this does not authorize underfilled work."""
    prevouts={tx["txid"]:tx["raw"].hex() for view in views.values() for _,tx in view.transactions.values()}
    verified=[];unavailable=[]
    for key in source.jobs:
        if key not in source.issued:
            continue
        try:
            verify_job(source,key,views,policy)
            source.normalized_job(key,prevouts)
            proof=source.verified_window(key,reference_target=int(profile["reference_target"],16),
                window_n=profile["window_n"],prevout_transactions=prevouts)
        except Inconclusive as error:
            unavailable.append({"job_key":list(key),"reason":str(error)})
            continue
        require(proof["window_work"] >= proof["window_target"] > 0,"full-N proof is below its actual Q")
        verified.append({"job_key":list(key),"Q":proof["window_target"],"selected_work":proof["window_work"],
            "selected_slice_ids":[m["slice_id"] for m in proof["mappings"]],
            "source_watermark":proof["source_watermark"],"source_log_sha256":source.log_sha256})
    if not verified:
        raise MissingComponent("eventual_verified_full_N_window")
    return {"verified_full_windows":verified,"other_issued_windows_unavailable":unavailable}


class SourceCapture:
    def __init__(self, run, private, capture, scenario, pool_binary, policy, source_feed=None):
        self.run, self.private, self.capture = Path(run), Path(private), capture
        require(not self.run.exists(), "source collector must register before runtime startup")
        self.scenario, self.pool_binary, self.policy = scenario, pool_binary, policy
        self.source_feed = source_feed
        self.output = self.private / "pool-source"
        self.output.mkdir(mode=0o700, exist_ok=False)
        self.logs, self.sources, self.issues, self.bindings, self.components = {}, {}, {}, {}, {}
        self.lock=threading.RLock();self.checkpoint_retention=None;self.notified={};self.eof={};self.stopping=set();self.process_records={}
        self.expected = {f["id"]: self.run / "logs" / ("front-" + f["id"] + ".log") for f in scenario["fronts"]}
        self.capture.add("source_capture_registered", {"registered_monotonic_ns": time.monotonic_ns(),
            "paths": {k: str(p) for k, p in self.expected.items()}, "pool_binary_sha256": pool_binary["sha256"],
            "authority": "observation_only", "runtime_absent": True})

    def bind(self, front, manifest):
        records = [p for p in manifest["processes"] if p["role"] == "front" and p["entity"] == front]
        require(len(records) == 1, "source front process missing/ambiguous")
        record = records[0]
        require(record["executable_sha256"] == self.pool_binary["sha256"]
                and Path(record["executable_path"]).resolve() == Path(self.pool_binary["path"]).resolve()
                and Path(record["log_path"]) == self.expected[front], "source process/binary/log binding")
        identity = ProcFS().identity(record["pid"])
        require(identity["start_time_ticks"] == record["start_time_ticks"]
                and identity["ppid"] == manifest["locks"]["pid"], "source front ownership/start token")
        require(Path(f"/proc/{record['pid']}/fd/1").resolve() == self.expected[front], "source stdout is not the declared log")
        binding = (record["pid"], record["start_time_ticks"], record["executable_sha256"])
        if front not in self.bindings:
            require(sha_file(f"/proc/{record['pid']}/exe") == self.pool_binary["sha256"], "actual source producer executable mismatch")
            self.bindings[front] = binding
            self.process_records[front]=dict(record)
        require(self.bindings[front] == binding, "source producer restart needs a new registered session collector")
        return binding

    def attach_checkpoints(self,retention):
        with self.lock:
            require(self.checkpoint_retention is None,"checkpoint collector already attached")
            self.checkpoint_retention=retention

    def mark_eof(self,front):
        from cleanup_proof import identity_observation
        with self.lock:
            require(front in self.bindings,"pool EOF lacks original process binding")
            pid,start,binary=self.bindings[front]
            observed=identity_observation(self.process_records[front])
            require(observed["status"] in {"gone","surviving_zombie"},"pool source writer has not exited")
            self.eof[front]={"process":observed,"binary_sha256":binary,"digest":None}
            self.capture.add("source_writer_eof",{"front":front,**self.eof[front]})

    def expect_stop(self,front):
        with self.lock:
            require(front in self.bindings,"pool stop lacks original bound source writer")
            self.stopping.add(front)

    def poll(self, manifest, views, *, materialize=True):
        with self.lock:self._poll(manifest,views,materialize)

    def _poll(self, manifest, views, materialize):
        for front, path in self.expected.items():
            try:
                if front not in self.eof and front not in self.stopping:self.bind(front, manifest)
                fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
                try:
                    info = os.fstat(fd)
                    require(stat.S_ISREG(info.st_mode) and info.st_uid == os.getuid(), "source log file ownership/type")
                    old = self.logs.get(front, {"data": b"", "identity": (info.st_dev, info.st_ino)})
                    require(old["identity"] == (info.st_dev, info.st_ino) and info.st_size >= len(old["data"]), "source log replaced/truncated")
                    if info.st_size > MAX_LOG:
                        raise MissingComponent("source_log_capture_bound")
                    with os.fdopen(fd, "rb", closefd=False) as stream:
                        data = stream.read(info.st_size)
                    require(len(data) == info.st_size and data.startswith(old["data"]), "source captured prefix changed")
                finally:
                    os.close(fd)
                digest=hashlib.sha256(data).hexdigest()
                if front in self.eof and self.eof[front]["digest"] is not None:
                    require(self.eof[front]["digest"]==digest,"source bytes changed after writer EOF")
                if data != old["data"]:
                    destination = self.output / (front + ".log")
                    flags = os.O_WRONLY | os.O_NOFOLLOW | (os.O_APPEND if front in self.logs else os.O_CREAT | os.O_EXCL)
                    target = os.open(destination, flags, 0o600)
                    with os.fdopen(target, "wb") as stream:
                        stream.write(data[len(old["data"]):]); stream.flush(); os.fsync(stream.fileno())
                    self.logs[front] = {"data": data, "identity": old["identity"]}
                    digest = hashlib.sha256(data).hexdigest()
                    self.capture.add("source_log_prefix", {"front": front, "path": str(destination), "bytes": len(data),
                        "sha256": digest, "complete_line_tail": data.endswith(b"\n"), "observed_monotonic_ns": time.monotonic_ns()})
                complete=data.rfind(b"\n")+1
                if self.checkpoint_retention is not None and complete>self.notified.get(front,0):
                    self.checkpoint_retention.notify(front,data[self.notified.get(front,0):complete])
                    self.notified[front]=complete
                if not data.endswith(b"\n"):
                    raise MissingComponent("source_log_incomplete_tail")
                digest = hashlib.sha256(data).hexdigest()
                if front not in self.sources or self.sources[front].log_sha256 != digest:
                    options = ({"source_contract": self.source_feed["contract"],
                                 "pool_pubkey": self.source_feed["pool_pubkeys"][front]} if self.source_feed else {})
                    if self.source_feed and self.source_feed.get('sql_contract'):
                        options['decision_contract']=self.source_feed['sql_contract']
                    self.sources[front] = PoolSource(data, independently_captured_log_sha256=digest, **options)
                if front not in self.sources:
                    raise MissingComponent("actual_source_evidence_prefix")
                self.issues.pop(front, None)
                if materialize:self.materialize(front, views)
                if front in self.eof:self.eof[front]["digest"]=digest
            except Invalid:
                raise
            except (Inconclusive, OSError, KeyError, ValueError, TypeError) as error:
                self.issues[front] = {"status": "INCONCLUSIVE", "component": getattr(error, "component", str(error))}

    def materialize(self, front, views):
        source = self.sources[front]
        pool_id = next(f["pool_id"] for f in self.scenario["fronts"] if f["id"] == front)
        profile = next(p["work_profile"] for p in self.scenario["pools"] if p["id"] == pool_id)
        for key, event in source.jobs.items():
            component_key = (front, key)
            old = self.components.get(component_key, {})
            if old.get("source_log_sha256") == source.log_sha256 and old.get("ancestry_tips") == sorted(views):
                continue
            material, missing = {}, []
            try:
                checked = verify_job(source, key, views, self.policy)
                material["job"] = checked["job"]
                material["witness_plan_verified"] = checked["witness"]
            except (Inconclusive, KeyError) as error:
                missing.append(getattr(error, "component", "job_source_component_missing"))
            try:
                prevouts = {tx["txid"]: tx["raw"].hex() for v in views.values() for _, tx in v.transactions.values()}
                window = source.verified_window(key, reference_target=int(profile["reference_target"], 16),
                                                 window_n=profile["window_n"], prevout_transactions=prevouts)
                material["verified_window"] = window
                jobs, slices = {}, []
                for mapping in window["mappings"]:
                    sid = mapping["slice_id"]
                    decoded = source.slice_publications[sid][0][1]
                    shares = []
                    for account in source.accounts[sid]:
                        a = account["payload"]
                        job_key = source.job_key(account)
                        if job_key not in jobs:
                            jobs[job_key] = verify_job(source, job_key, views, self.policy)["job"]
                        shares.append({"identity": list(bytes.fromhex(a["sp_id"])), "job_id": jobs[job_key]["job_id"],
                            "generation": job_key[3], "nonce": a["nonce"], "ntime": a["ntime"], "version": a["version"],
                            "extranonce": list(bytes.fromhex(a["extranonce"])), "share_index": a["share_index"],
                            "share_hash": list(bytes.fromhex(a["share_hash"])), "credited_work": a["d"]})
                    slices.append({"slice_id": sid, "height": decoded["height"], "front_id": decoded["front_id"],
                        "work_unit_version": decoded["work_unit_version"],
                        "work_unit_fingerprint": list(bytes.fromhex(decoded["work_unit_fingerprint"])),
                        "difficulty": str(decoded["difficulty"]), "root": list(bytes.fromhex(decoded["root"])), "shares": shares})
                material["jobs"], material["slices"] = list(jobs.values()), slices
                material["high_water"] = {f: int(s) for f, s in event["payload"]["high_water"].items()}
                material["reference_target_be"] = list(bytes.fromhex(profile["reference_target"]))
                material["window_n"], material["window_target"] = profile["window_n"], str(window["window_target"])
            except (Inconclusive, KeyError) as error:
                missing.append(getattr(error, "component", "complete_source_window"))
            if source.source_contract is not None:
                material["finding_slice_id"] = int(event["payload"]["finding_slice_id"])
            else:
                missing.append("actual_finding_slice_boundary")
            missing += ["sealed_RULEB_job_economic_history_checkpoint", "runtime_SourceProofFeed_consumption_on_actual_native_mining_path"]
            packet = {"schema": "spc-lab-source-components/1", "front": front, "job_key": list(key),
                      "source_log_sha256": source.log_sha256, "source_heads": source.heads,
                      "source_log_prefix_bytes": len(self.logs[front]["data"]), "source_event": event["source"],
                      "observed_before_issued_record": key not in source.issued,
                      "components": material, "missing_components": missing, "status": "INCONCLUSIVE",
                      "mining_authorized": False, "trusted_source_issued": False}
            raw = json.dumps(packet, sort_keys=True, separators=(",", ":")).encode() + b"\n"
            if len(raw) > 4 * 1024 * 1024:
                raise MissingComponent("source_component_packet_bound")
            digest = hashlib.sha256(raw).hexdigest()
            path = self.output / (digest + ".json")
            if path.exists():
                require(not path.is_symlink() and path.read_bytes() == raw, "source component artifact collision/change")
            else:
                fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
                with os.fdopen(fd, "wb") as stream:
                    stream.write(raw); stream.flush(); os.fsync(stream.fileno())
            self.components[component_key] = {"path": str(path), "sha256": digest, "missing_components": missing,
                                              "source_log_sha256": source.log_sha256, "ancestry_tips": sorted(views)}
            self.capture.add("source_job_components", {"front": front, "job_key": list(key), **self.components[component_key]})

    def publications(self, views):
        if self.issues or set(self.sources) != set(self.expected):
            raise MissingComponent("complete_front_source_census")
        results = {}
        all_claimed = set()
        for front, source in self.sources.items():
            if not source.block_publications:
                raise MissingComponent("actual_pool_block_publication:" + front)
            expected = set()
            jobs_by_height = {}
            for event in source.jobs.values():
                jobs_by_height.setdefault(event["payload"]["height"], []).append(event["payload"])
            for view in views.values():
                for block in view.blocks:
                    for p in jobs_by_height.get(block["height"], []):
                        if (block["prev"] == bytes.fromhex(p["prev_hash"])[::-1].hex()
                                and matches_coinbase_template(p,block["transactions"][0])):
                            expected.add(block["hash"])
            if not expected or not expected <= set(source.block_publications):
                raise MissingComponent("retained_raw_pool_block_missing_source_publication:" + front)
            require(not all_claimed.intersection(expected), "raw block attributed to multiple fronts")
            all_claimed.update(expected)
            results[front] = [verify_publication(source, block, views, self.policy) for block in source.block_publications]
        return results

    def witness_jobs(self, views):
        if self.issues:
            raise MissingComponent("complete_witness_source_capture")
        results = []
        for source in self.sources.values():
            try:
                results.extend(witness_republication(source, views, self.policy))
            except MissingComponent:
                continue
        if not results:
            raise MissingComponent("actual_witness_job_republication_and_stale_result")
        return results

    def full_window_coverage(self, views):
        """Eventual full-Q evidence for every declared front, independently of
        any future audited-underfill permission at an earlier activation.

        This uses the existing full-window producer contract only. It neither
        accepts a warmup flag nor promotes an incomplete/empty prefix to work.
        """
        if self.issues or set(self.sources) != set(self.expected):
            raise MissingComponent("complete_front_source_census_for_full_N")
        result={}
        for front,source in self.sources.items():
            pool=next(f["pool_id"] for f in self.scenario["fronts"] if f["id"]==front)
            profile=next(p["work_profile"] for p in self.scenario["pools"] if p["id"]==pool)
            try:
                result[front]={"pool":pool,**full_window_evidence(source,views,self.policy,profile)}
            except MissingComponent as error:
                raise MissingComponent(error.component+":"+front) from error
        return {"scope":"eventual_full_Q_window_work_only","fronts":result,"native_activation_verified":False}

    def for_job(self, front, key):
        """Diagnostic component lookup, never a complete proof authority.

        Source-r3 transport uses ReplayProvider and FeedServer. This historical
        component path cannot replace their SQL/actual-E/source-prefix checks.
        """
        if (front, key) not in self.components:
            raise MissingComponent("collected_job_components")
        record = self.components[(front, key)]
        raw = Path(record["path"]).read_bytes()
        require(hashlib.sha256(raw).hexdigest() == record["sha256"], "proof-feed component seal changed")
        packet = strict_json(raw)
        if packet["missing_components"]:
            raise MissingComponent(";".join(packet["missing_components"]))
        raise MissingComponent("runtime_SourceProofFeed_transport_and_before_publish_ack")
