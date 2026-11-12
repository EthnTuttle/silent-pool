"""Audit the one sealed post-drain boundary using retained files only."""
from copy import copy
import hashlib
import os
from pathlib import Path
import stat
import tomllib

from canonical_capture import validate_export, _read_blob
from case_contract import canonical_json
from economic_checkpoint import query
from oracle import Chain, Inconclusive, Invalid, require, strict_json
from pool_source import PoolSource
from payout_history import audit_pool
from source_jobs import MissingComponent
from storage import private_bytes, confined

SCHEMA="spc-lab-owned-economic-drain/2"
FINAL_GATES={"raw_chain","source_publication","full_window_coverage","payout_ladder","premining_proof_feed"}


def sha(raw):return hashlib.sha256(raw).hexdigest()


def read_boundary(c,runtime):
    ref=runtime.final_boundary_ref
    if runtime.drain_result is None or ref is None:raise MissingComponent("successful_sealed_final_drain_required")
    require(Path(ref["path"])==c.priv/"economic-final-boundary.json","final audit boundary path")
    raw=private_bytes(Path(ref["path"]),c.priv,128*1024*1024)
    require(sha(raw)==ref["sha256"],"final audit boundary seal changed")
    value=strict_json(raw)
    require(value["schema"]==SCHEMA and value["run_id"]==c.run_id and value["status"]=="CONVERGED"
        and value["stable_key"]==runtime.drain_result["stable_key"] and value["tip"]==runtime.drain_result["tip"],"final audit run/drain binding")
    return value


def retained_checkpoint(c,runtime,record,pool):
    path=Path(record["path"])
    require(path==runtime.retention.root/(record["sha256"]+".json") and record["pool_id"]==pool,"final checkpoint path/pool")
    raw=private_bytes(path,runtime.retention.root,128*1024*1024)
    require(sha(raw)==record["sha256"],"final checkpoint digest changed")
    original=record["original_artifact"]
    envelope_raw=private_bytes(Path(original["path"]),c.priv,128*1024*1024)
    require(sha(envelope_raw)==original["sha256"],"final original SQL envelope changed")
    envelope=strict_json(envelope_raw);observation=strict_json(raw)
    require(envelope["schema"]=="spc-lab-bound-sql-capture/1" and envelope["binding"]["run_id"]==c.run_id
        and envelope["binding"]["pool_id"]==pool and type(envelope["execution"]["exit_code"]) is int
        and envelope["execution"]["exit_code"]==0 and envelope["execution"]["timed_out"] is False
        and envelope["query_sha256"]==sha(query(source_contract=runtime.retention.sql_contract).encode())
        and canonical_json(envelope["observation"])==raw,"final SQL capture execution/query/original bytes")
    require(observation["history_boundary"]["event_watermark"]==record["history_watermark"]
        and observation["tables"]["economic_state"][0]["revision"]==record["revision"],"final SQL watermark/revision")
    return observation


def frozen_sources(c,boundary):
    require(set(boundary["source_EOF"])==set(c.source_capture.expected),"final EOF source census")
    sources={}
    for front,path in c.source_capture.expected.items():
        digest=boundary["source_EOF"][front]["digest"]
        if digest is None:raise MissingComponent("final_source_EOF_digest_missing")
        confined(path,c.run)
        fd=os.open(path,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK)
        try:
            before=os.fstat(fd)
            require(stat.S_ISREG(before.st_mode) and before.st_size<=128*1024*1024,"final original source file type/bound")
            with os.fdopen(fd,"rb",closefd=False) as stream:original=stream.read(128*1024*1024+1)
            after=os.fstat(fd)
            require((before.st_size,before.st_mtime_ns)==(after.st_size,after.st_mtime_ns) and len(original)==before.st_size,"final original source changed while read")
        finally:os.close(fd)
        retained=private_bytes(c.source_capture.output/(front+".log"),c.priv,128*1024*1024)
        require(sha(original)==sha(retained)==digest,"late or changed source after sealed EOF")
        sources[front]=PoolSource(retained,independently_captured_log_sha256=digest,
            source_contract=c.binding["source_feed"]["contract"],pool_pubkey=c.binding["source_feed"]["pool_pubkeys"][front],
            decision_contract=c.binding['source_feed'].get('sql_contract'))
    return sources


def frozen_views(c,boundary):
    ref=boundary.get("chain_evidence")
    if ref is None:raise MissingComponent("final_raw_chain_export_missing")
    require(Path(ref["path"])==c.priv/"drain-chain-evidence.json","final raw chain export path")
    checked=validate_export(ref["path"],c.run_id,{k:v["sha256"] for k,v in c.binding["binaries"].items()},
        expected_export_sha256=ref["sha256"],expected_source_snapshot_sha256=c.binding["source_snapshot_sha256"],
        expected_scenario_sha256=sha(private_bytes(c.priv/"scenario.json",c.priv)),expected_policy=c.policy,expected_node_ids=list(c.nodes))
    require(checked["verified_fences"][-1]["tip_hash"]==boundary["tip"]["hash"] and checked["verified_fences"][-1]["height"]==boundary["tip"]["height"],"final raw fence does not reach sealed tip")
    data=strict_json(private_bytes(Path(ref["path"]),c.priv,128*1024*1024));bodies={};views={}
    require(Path(data["private_blob_root"])==c.priv,"final raw evidence blob root")
    for item in data["block_bodies"]:
        raw=_read_blob(c.priv,item["raw_block"]);bh=item["block_hash"]
        require(bh not in bodies or bodies[bh]==raw,"final nodes disagree on raw body")
        bodies[bh]=raw
    for fence in checked["verified_fences"]:
        tip=fence["tip_hash"]
        if tip in views:continue
        raw=[];current=tip;seen=set()
        while True:
            require(current not in seen and current in bodies,"final raw ancestry missing/cycle")
            seen.add(current);raw.append(bodies[current])
            if current==c.policy.genesis:break
            current=bodies[current][4:36][::-1].hex()
        views[tip]=Chain(c.policy,raw[::-1])
    return views,checked


def checkpoint_fence(runtime,boundary):
    runtime.retention.require_healthy()
    journal=boundary["checkpoint_journal"]
    if journal is None:raise MissingComponent("final_checkpoint_journal_seal_missing")
    require(runtime.retention.sequence==journal["sequence"] and runtime.retention.previous==journal["head"]
        and len(runtime.retention.records)==journal["records"] and not runtime.retention.queue.unfinished_tasks
        and sha(private_bytes(runtime.retention.root/"attempts.jsonl",runtime.retention.root,128*1024*1024))==journal["sha256"],"checkpoint retention advanced after final seal")


def inputs(c,runtime):
    boundary=read_boundary(c,runtime);checkpoint_fence(runtime,boundary)
    sources=frozen_sources(c,boundary);views,chain_proof=frozen_views(c,boundary);finals={};stable=[]
    require(set(boundary["pools"])=={p["id"] for p in c.scenario["pools"]},"final SQL pool census")
    for pool in c.scenario["pools"]:
        name=pool["id"];sealed=boundary["pools"][name];observation=retained_checkpoint(c,runtime,sealed["checkpoint"],name)
        heads={f["id"]:sources[f["id"]].heads for f in c.scenario["fronts"] if f["pool_id"]==name}
        require(canonical_json(heads)==canonical_json(sealed["source_heads"]),"final source heads differ from SQL seal")
        stable.append([name,observation["tables"]["economic_state"][0]["revision"],observation["history_boundary"]["event_watermark"],heads]);finals[name]=observation
    require(sha(canonical_json([boundary["tip"]["hash"],stable]))==boundary["stable_key"],"final raw/source/SQL stable key mismatch")
    return boundary,sources,views,finals,chain_proof


def assess(c,runtime):
    boundary,sources,views,finals,chain_proof=inputs(c,runtime);checks={}
    capture=copy(c.source_capture);capture.sources=sources;capture.issues={}
    def check(name,callback):
        try:
            value=callback();state="INCONCLUSIVE" if isinstance(value,dict) and value.get("status")=="INCONCLUSIVE" else "PASSED"
            checks[name]={"status":state,"detail":value}
        except Invalid as error:checks[name]={"status":"FAILED","detail":str(error)}
        except (Inconclusive,OSError,KeyError,ValueError,TypeError) as error:checks[name]={"status":"INCONCLUSIVE","detail":getattr(error,"component",str(error))}
    check("raw_chain",lambda:{"tip":boundary["tip"],"final_export":boundary["chain_evidence"],"proof":chain_proof})
    check("source_publication",lambda:capture.publications(views))
    check("full_window_coverage",lambda:capture.full_window_coverage(views))
    if c.args.witness:check("witness_job_republication",lambda:capture.witness_jobs(views))
    def payout():
        coverage=runtime.retention.coverage(sources,views,runtime.migrations,{})
        require(coverage["status"]=="COMPLETE_CAPTURE","final exact checkpoint coverage incomplete")
        results={}
        for pool in c.scenario["pools"]:
            front_ids=[f["id"] for f in c.scenario["fronts"] if f["pool_id"]==pool["id"]]
            configs=[private_bytes(c.run/"private"/("pool-"+front+".toml"),c.run) for front in front_ids]
            expected=boundary["pool_config_sha256"][pool["id"]]
            require([sha(raw) for raw in configs]==expected,"final public pool config changed")
            parsed=[tomllib.loads(raw.decode()) for raw in configs]
            descriptors={p["pool"]["pool_payout_descriptor"] for p in parsed};keys={p.get("ark",{}).get("order_pubkey") for p in parsed}
            require(len(descriptors)==len(keys)==1,"final pool public bindings disagree")
            from settlement_drain import convergence_gaps
            require(not convergence_gaps(views[boundary["tip"]["hash"]],[sources[f] for f in front_ids],finals[pool["id"]]),"final economic convergence no longer holds")
            result=audit_pool({f:sources[f] for f in front_ids},{f:list(runtime.retention.observations(f)) for f in front_ids},
                finals[pool["id"]],views,c.policy,pool["work_profile"],runtime.migrations,descriptor=next(iter(descriptors)),order_pubkey=next(iter(keys)))
            results[pool["id"]]=result
        return {"status":"INCONCLUSIVE" if any(v["status"]!="PASSED" for v in results.values()) else "PASSED","pools":results}
    check("payout_ladder",payout)
    def native():
        if c.native_capture is None or boundary.get("native_boundary") is None:raise MissingComponent("sealed_final_native_prefix_unavailable")
        return c.native_capture.audit_after_drain(runtime.servers,boundary["native_boundary"])
    check("premining_proof_feed",native)
    # No new read/capture authority is issued. Check the original seal again
    # after potentially expensive verification to detect late writes.
    require(read_boundary(c,runtime)==boundary,"final boundary changed during audit")
    frozen_sources(c,boundary)
    checkpoint_fence(runtime,boundary)
    for pool,sealed in boundary["pools"].items():retained_checkpoint(c,runtime,sealed["checkpoint"],pool)
    if checks["premining_proof_feed"]["status"]=="PASSED":native()
    states={v["status"] for v in checks.values()}
    return {"status":"FAILED" if "FAILED" in states else "INCONCLUSIVE" if "INCONCLUSIVE" in states else "PASSED",
        "scope":"final_raw_source_SQL_native_prefix_only","boundary":runtime.final_boundary_ref,"stable_key":boundary["stable_key"],"checks":checks}
