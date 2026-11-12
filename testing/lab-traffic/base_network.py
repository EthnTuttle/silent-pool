"""Historical host-network boundary evidence; no RPC, peering or mining calls."""
from collections import Counter
import hashlib
import json
from pathlib import Path
from types import SimpleNamespace

from ap5_v2 import archive, artifact
from case_contract import canonical_json
from oracle import Inconclusive, decode_block, integer, require, sha256d, strict_json, target, unhex
from runtime_bootstrap import RuntimeBootstrap, FIXED_CLI_SHA256
from source_jobs import MissingComponent


def reject_known_host_network_failure(binary):
    # The phase-wait fix remains verified. Its operational host P2P graph did
    # not work in the actual run; do not repeat it as a funding workaround.
    if binary["sha256"]==FIXED_CLI_SHA256:
        raise MissingComponent("base_Runtime_host_P2P_fix_tested_bundle_pending")


def historical(handoff_path):
    h=strict_json(Path(handoff_path).read_bytes());receipt=strict_json(artifact(h["live_receipt"]))
    root,records=archive(h["live_event_seal"]["path"],h["live_event_seal"]["sha256"])
    require(str(root)==h["live_run_root"]==receipt["run_root"],"network history archive binding")
    labels={}
    for label,value,row in records:labels.setdefault(label,[]).append((value,row))
    scenario=labels["scenario"][0][0];launch=labels["launch"][0][0]
    bundle=strict_json(artifact(receipt["bundle"]))
    require(launch["argv"][1]=="runtime" and bundle["binaries"]["lab"]["sha256"]==FIXED_CLI_SHA256,"network history actual Runtime binary")
    inputs={}
    def read(path,maximum=128*1024*1024):
        path=Path(path);require(path.is_file() and not path.is_symlink() and path.stat().st_size<=maximum,"network history file bound")
        raw=path.read_bytes();inputs[str(path)]={"sha256":hashlib.sha256(raw).hexdigest(),"bytes":len(raw)};return raw
    command_raw=read(root/"run/private/commands.jsonl");phase_raw=read(root/"run/private/evidence.jsonl")
    owner=RuntimeBootstrap(scenario);status=owner.observe(command_raw,phase_raw,require_phase=True)
    require(status["state"]=="cancelled","network history original bootstrap terminal")
    attempts={};reserves={};reconciles={};node_submits=Counter();last_tips={};commands=[];finishes={}
    for line in command_raw.splitlines():
        r=strict_json(line);kind=r["record"]
        if kind=="begin":commands.append(r["command"])
        elif kind=="finish":finishes[r["command_id"]]=r["outcome"]
        elif kind=="hash_reserve":reserves[(r["operation_id"],r["driver_incarnation"],r["batch_id"])]=r
        elif kind=="hash_reconcile":reconciles[(r["operation_id"],r["driver_incarnation"],r["batch_id"])]=r
        elif kind=="traffic_audit":
            e=r["event"]
            if e["event"]=="forward_attempt":
                a=e["attempt"];attempts[r["seq"]]=a
                if a["method"]=="submitblock":node_submits[a["node_id"]]+=1
            elif e["event"]=="forward_result":
                require(e["attempt_seq"] in attempts,"network history RPC result without attempt")
                a=attempts[e["attempt_seq"]]
                if a["method"]=="getbestblockhash" and e["outcome"]=="ok" and e["response_suppressed"] is False:
                    last_tips[a["node_id"]]=e["response_sha256"]
    require(not any(c["kind"]["type"]=="send_user_payment" for c in commands),"network diagnostic issued user payments")
    stops=[c for c in commands if c["kind"]["type"]=="stop_run"]
    require(len(stops)==1 and len(commands)==2,"network diagnostic unexpected command census")
    stop_reply=strict_json(artifact(receipt["stop_reply"]))
    require(stop_reply["command_id"]==stops[0]["command_id"] and stop_reply["state"]=="succeeded"
            and finishes[stop_reply["command_id"]]==stop_reply,"network diagnostic actual stop reply")
    native_paths=list((root/"run/private/traffic").glob("d-*/native-bootstrap.jsonl"))
    require(len(native_paths)==1,"network diagnostic native funding authority census")
    native_raw=read(native_paths[0]);require(native_raw.endswith(b"\n"),"network native log truncated")
    native=[strict_json(line) for line in native_raw.splitlines()];begins=[r for r in native if r["event"]=="begin"]
    require(len(begins)==1,"network native begin census");begin=begins[0]
    for k in ("boot_id","deadline_monotonic_ms","cancel_grace_secs","driver_id"):
        require(begin["request"][k]==status["budget"][k],"network original native deadline/budget binding")
    require(begin["request"]["command_id"]==status["operation_id"] and begin["request"]["node_id"]==owner.plan["kind"]["node_id"],"network original native command/authority node")
    seen=set();measured=0;headers=set()
    for r in native:
        if r["event"]!="hash_batch":continue
        a=r["authorization"];key=(a["operation_id"],a["driver_incarnation"],a["batch_id"])
        require(key in reserves and key in reconciles and key not in seen,"network hash authorization/reconcile census");seen.add(key)
        require(all(reserves[key][k]==v for k,v in a.items()) and r["measured_attempts"]==reconciles[key]["measured_attempts"]==len(r["raw_headers"]),"network actual measured/reserved attempts")
        require(r["monotonic_ms"]<status["budget"]["deadline_monotonic_ms"],"network hash after original deadline")
        for header in r["raw_headers"]:
            unhex(header,80);require(header not in headers,"network native header repeated");headers.add(header)
        measured+=r["measured_attempts"]
    require(seen==set(reserves)==set(reconciles) and measured==status["debits"]["measured_executed"],"network hash budget coverage")
    submissions=[r for r in native if r["event"]=="submitblock"]
    require(len(submissions)==1,"network diagnostic raw block census");submitted=submissions[0]
    block=decode_block(unhex(submitted["raw_block"]),SimpleNamespace(pow_limit_bits=begin["policy"]["pow_limit_bits"]))
    require(submitted["raw_block"][:160] in headers,"network submitted block was not in its authorized measured batch")
    require(block["hash"]==submitted["block_hash"] and block["prev"]==begin["policy"]["genesis"]
            and submitted["outcome"]=={"outcome":"ok","result":None} and submitted["transport_error"] is None,"network original native raw block/submit result")
    require(len(block["transactions"])==1 and sum(o["value"] for o in block["transactions"][0]["outputs"])==begin["policy"]["subsidy_initial_sats"],"network raw native subsidy/fee")
    require(node_submits=={owner.plan["kind"]["node_id"]:1},"network diagnostic relayed a block to another node")
    terminals=[r for r in native if r["event"]=="terminal"]
    require(len(terminals)==1 and terminals[0]["success"] is False and terminals[0]["error"]=="bootstrap_cancelled"
            and terminals[0]["measured_attempts"]==measured
            and terminals[0]["acknowledged_cumulative_reserved"]==status["debits"]["charged_reserved"],"network original native cancellation/debit terminal")
    expected_tips={"node-a":block["hash"],"node-b":begin["policy"]["genesis"]}
    require(last_tips=={node:hashlib.sha256(canonical_json({"outcome":"ok","result":tip})).hexdigest() for node,tip in expected_tips.items()},"network historical canonical-tip response hashes")
    manifest=strict_json(read(root/"run/run-manifest.json"));nodes=[p for p in manifest["processes"] if p["role"]=="node"]
    require({p["entity"] for p in nodes}==set(expected_tips),"network node roster")
    args=[]
    for node in nodes:
        argv=node["args"]
        require(node["executable_sha256"]==bundle["binaries"]["node"]["sha256"] and "--no-listen" in argv
                and "--no-seeds" in argv and "--no-discover" in argv
                and not any(v=="--connect" or v.startswith("--connect=") for v in argv),"network actual host argument classification")
        args.append({"node":node["entity"],"pid":node["pid"],"start_time_ticks":node["start_time_ticks"],"argv_sha256":hashlib.sha256(canonical_json(argv)).hexdigest(),"listening_disabled":True,"explicit_connect":False})
    observer_root=Path(handoff_path).parent
    peer_observations=strict_json(read(observer_root/"live-bootstrap-chain-observation.json"))
    require({r["node"]:r["peer_count"] for r in peer_observations}=={"a":0,"b":0},"network owner peer observation mismatch")
    cleanup=labels["cleanup-inventory"][0][0];inventory=cleanup["inventory"]
    require(cleanup["exhaustive"] is False and cleanup["all_resources_released"] is False
            and len(inventory["unreadable_same_uid_processes"])==4,"network cleanup completeness classification")
    require(all(p["status"]=="gone_or_reused" for p in inventory["processes"]),"network recorded actor survivor")
    return {"schema":"spc-lab-base-network-history/1","classification":"BASE_HOST_P2P_BOUNDARY_FAILED","phase_wait_fix_observed":True,
        "original_bootstrap":status,"stop_reply":stop_reply,"raw_native_block":{"hash":block["hash"],"previous":block["prev"],"fees_sats":0,"pow_merkle_subsidy_verified":True},
        "actual_tip_response_hashes_matched":expected_tips,"node_arguments":args,"recorded_submitblock_calls":dict(node_submits),"measured_hashes":measured,
        "owner_peer_observations":peer_observations,"peer_count_scope":"owner observation file retained by hash; not fabricated raw RPC response",
        "recorded_actors_gone":len(inventory["processes"]),"global_cleanup":"INCONCLUSIVE","unreadable_same_uid_count":4,"user_payments":0,
        "input_artifacts":inputs,"full_native_audit":False,"funded_AP5":False,"whole_lab_acceptance":"INCONCLUSIVE"}
