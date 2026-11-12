"""Replay actual native event envelopes against owned collector/lifecycle facts.

An ordered event component is not full native approval without retained outer
request/Approved bytes. Caller dictionaries never create a wallet capability.
"""
import hashlib

from oracle import Inconclusive, integer, require, strict_json, unhex
from case_contract import canonical_json
import native_codec as native
from source_feed_codec import (fields, array, received_digest, checkpoint_digest,
    consumed_digest, committed_digest, verify_public_evidence, bundle_window, validate_window)
from source_jobs import MissingComponent

ENVELOPE="protocol version kind source event_id source_cursor miner_id occurred_at_wall_ms occurred_at_mono_ms dropped_before event".split()
EVENTS={
    "job_candidate":"connection_id session_slot candidate_epoch request_digest job",
    "source_audit_committed":"connection_id session_slot candidate_epoch capability job_id generation request_digest checkpoint_digest consumed_digest committed_digest expires_at_unix_ms window",
    "source_audit_unknown":"connection_id session_slot candidate_epoch code",
    "audit_first_hash":"connection_id worker_id session_slot candidate_epoch request_digest job_id",
    "explicit_unaudited":"connection_id",
    "connection_state":"connection_id session_slot state detail",
    "target_updated":"connection_id target_generation share_target_le",
    "job_activated":"connection_id job",
    "jobs_invalidated":"connection_id job_ids reason",
    "submitted":"share",
    "ingress_ack":"connection_id sequence_number accepted_count new_shares_sum",
    "ingress_rejected":"connection_id sequence_number error_code",
    "submissions_unknown":"connection_id sequence_numbers reason",
    "submissions_evicted":"connection_id sequence_numbers reason",
    "late_verdict":"connection_id sequence_numbers outcome",
    "block_qualified":"connection_id sequence_number block_hash candidate_submission",
    "worker_search_exhausted":"connection_id worker_id job_id ntime reason",
}


def controller_events(raw,run_id,incarnations):
    """Decode actual EvidenceStore::MinerEvent projection, not health counters.

    The pump preserves miner wall time and IDs; its monotonic time is controller
    ingestion time. The internal normalized rows below never claim otherwise.
    """
    if not raw or not raw.endswith(b"\n"):raise MissingComponent("native_controller_evidence_truncated")
    result=[];last=0
    for line in raw.splitlines():
        e=strict_json(line)
        fields(e,"run_id event_id source source_cursor actor command_id occurred_at_wall occurred_at_mono ingested_at payload".split())
        require(e["run_id"]==run_id,"native evidence foreign run")
        if e["event_id"]!=last+1:raise MissingComponent("native_controller_evidence_pruned_prefix")
        last=e["event_id"];payload=e["payload"];fields(payload,("kind","data"))
        if payload["kind"]=="event_lost":raise MissingComponent("native_controller_evidence_recorded_gap")
        if payload["kind"]!="miner_event":continue
        p=payload["data"];fields(p,"miner_id incarnation driver_event_id name payload dropped_before".split())
        miner=p["miner_id"]
        require(miner in incarnations and p["incarnation"]==incarnations[miner] and e["source"]=="miner-"+miner
                and e["actor"]=={"kind":"miner","id":miner} and e["command_id"] is None
                and e["source_cursor"]==p["incarnation"]+":"+str(p["driver_event_id"]),"native controller process/incarnation/cursor binding")
        result.append({"protocol":"spc-cpu-miner-events","version":1,"kind":"event","source":"process",
            "event_id":p["driver_event_id"],"source_cursor":p["driver_event_id"],"miner_id":miner,
            "occurred_at_wall_ms":e["occurred_at_wall"],"occurred_at_mono_ms":integer(e["occurred_at_mono"])//1000000,
            "dropped_before":p["dropped_before"],"event":{"name":p["name"],"payload":p["payload"]}})
    return result


def bind_connection(config,w,reg,observed,route):
    """Route is independently derived from actual scenario/front/worker config."""
    native.validate_instance(w);native.validate_process(observed)
    fields(config,"id endpoint identity authority_public_key source_audit min_extranonce_size reconnect workers".split())
    a=config["source_audit"]
    if a.get("mode")!="required":raise MissingComponent("explicit_unaudited_profile_is_not_full_native_audited")
    fields(a,native.AUDIT)
    require(config["endpoint"]==route["endpoint"] and config["authority_public_key"]==route["authority_public_key"]
            and config["identity"]==w["identity"]==route["identity"] and config["id"]==route["connection_id"]
            and config["workers"]==route["workers"],"native direct SV2/identity/worker route")
    require(a["run_id"]==w["run_id"]==route["run_id"] and a["miner_id"]==route["miner_id"]
            and w["pool_id"]==route["pool_id"] and w["front_id"]==route["front_id"],"native run/pool/front binding")
    for key,field in (("wallet_instance","instance_id"),("wallet_process","process"),("wallet_socket","socket_path"),
                      ("wallet_socket_device","socket_device"),("wallet_socket_inode","socket_inode"),
                      ("collector_pubkey","collector_pubkey"),("pool_pubkey","pool_pubkey")):
        require(a[key]==w[field],"native AddConnection wallet binding: "+key)
    require(array(a["connection_config_digest"],32)==native.connection_digest(config),"native complete connection config digest")
    fields(reg,("schema","run_id","wallet_instance","generation","peers"))
    require(reg["schema"]==native.REGISTRY_SCHEMA and reg["run_id"]==w["run_id"]
            and reg["wallet_instance"]==w["instance_id"],"native actual peer registry binding")
    integer(reg["generation"],1,(1<<64)-1);require(isinstance(reg["peers"],list) and len(reg["peers"])<=256,"native peer capacity")
    seen=set();matches=[]
    for peer in reg["peers"]:
        fields(peer,native.PEER);native.validate_process(peer["process"]);integer(peer["generation"],1,reg["generation"])
        pair=(peer["miner_id"],peer["connection_id"]);require(pair not in seen,"native duplicate registry peer");seen.add(pair)
        if pair==(a["miner_id"],config["id"]):matches.append(peer)
    require(len(matches)==1,"native actual registered peer missing");peer=matches[0]
    require(peer["process"]==observed and peer["generation"]==a["peer_generation"]
            and peer["identity"]==config["identity"] and peer["pool_id"]==w["pool_id"] and peer["front_id"]==w["front_id"]
            and peer["connection_config_digest"]==a["connection_config_digest"]
            and peer["config_path"]==route["miner_config_path"] and observed["uid"]==w["process"]["uid"]
            and observed["boot_id"]==w["process"]["boot_id"],"native registry observed process/config mismatch")
    require(all(worker["ntime_policy"]=="hold" for worker in config["workers"]),"native immutable ntime required")
    return {"config":config,"wallet":w,"registry":reg,"observed_miner":observed,"route":route}


def collector_entries(servers):
    """Only durable entries from the actual live owned collector, not event counts."""
    result={}
    for server in servers:
        with server.lock:
            require(not server.journal.poisoned,"native collector journal poisoned")
            for token,entry in server.attempts.items():
                if "committed" not in entry:continue
                proof=server.journal.get_blob(entry["prepared"]);ack=server.journal.get_blob(entry["consumed"])["ack"]
                commit=server.journal.get_blob(entry["committed"])
                bundle=bytes.fromhex(proof["bundle_hex"])
                require(hashlib.sha256(bundle).digest()==array(proof["checkpoint"]["bundle_digest"],32)
                        and bundle_window(bundle)==ack["window"],"native retained bundle binding")
                evidence={"schema":proof["schema"],"checkpoint":proof["checkpoint"],"consumed":ack,
                    "committed":{k:commit[k] for k in ("attempt","ack_digest","signature")}}
                key=consumed_digest(ack).hex();require(key not in result,"native duplicate collector ACK")
                result[key]={"agent":entry["agent"],"binding":server.agents[entry["agent"]],"evidence":evidence,
                    "bundle":strict_json(bundle),"collector_pubkey":server.signer.public_key,"attempt":token}
    return result


def replay(events,bindings,collectors,outer=None,*,require_durable=False):
    """`outer` contains retained decoded public request/response, keyed by digest.

    Live capture must supply actual retained bytes; this adapter neither opens a
    second wallet exchange nor reconstructs native nonce/expiry from timestamps.
    """
    outer={} if outer is None else outer
    cursors={};states={};epochs={};nonces=set();coverage=set();workers=set();missing=set();checked=[]
    candidates=set();activated=set();abandoned={}
    for row in events:
        fields(row,ENVELOPE);miner=row["miner_id"];native.identifier(miner)
        require(row["protocol"]=="spc-cpu-miner-events" and integer(row["version"],1,1)==1 and row["kind"]=="event" and row["source"]=="process","native event envelope")
        seq=integer(row["event_id"],1);mono=integer(row["occurred_at_mono_ms"]);now=integer(row["occurred_at_wall_ms"])
        prior=cursors.get(miner,(0,0,0))
        if seq!=prior[0]+1 or row["source_cursor"]!=seq or row["dropped_before"]!=0:
            raise MissingComponent("complete_native_event_prefix_no_drops")
        if mono<prior[1] or now<prior[2]:raise MissingComponent("native_event_clock_order")
        cursors[miner]=(seq,mono,now)
        event=row["event"];fields(event,("name","payload"));name=event["name"];p=event["payload"]
        if name not in EVENTS:raise MissingComponent("unsupported_native_event")
        optional=["durable_evidence"] if name=="source_audit_committed" and "durable_evidence" in p else []
        fields(p,EVENTS[name].split()+optional)
        for field in ("candidate_epoch","generation","expires_at_unix_ms","job_id"):
            if field in p:integer(p[field],0,(1<<64)-1)
        if "session_slot" in p and p["session_slot"] is not None:integer(p["session_slot"],0,65535)
        for field in ("request_digest","checkpoint_digest","consumed_digest","committed_digest"):
            if field in p:unhex(p[field],32)
        if "window" in p:validate_window(p["window"])
        con=p["share"]["connection_id"] if name=="submitted" else p["connection_id"]
        key=(miner,con)
        require(key in bindings,"native event outside actual declared connection roster")
        binding=bindings[key];config=binding["config"];w=binding["wallet"]
        state=states.get(key)
        if name=="explicit_unaudited":raise MissingComponent("explicit_unaudited_native_event")
        if name in {"target_updated","jobs_invalidated","connection_state"}:
            if state is not None and not state["activated"]:abandoned[state["candidate"]["request_digest"]]=name
            states.pop(key,None)
        elif name=="job_candidate":
            epoch=(integer(p["session_slot"],0,65535),integer(p["candidate_epoch"],1))
            require(key not in epochs or epoch>epochs[key],"native candidate epoch replay/regression")
            epochs[key]=epoch;unhex(p["request_digest"],32);fields(p["job"],native.JOB_SNAPSHOT)
            if state is not None and not state["activated"]:abandoned[state["candidate"]["request_digest"]]="superseded"
            candidates.add(p["request_digest"])
            states[key]={"candidate":p,"candidate_wall_ms":now,"epoch":epoch,"commit":None,"activated":False,"first":set()}
        elif name=="source_audit_committed":
            require(state is not None and state["commit"] is None,"native commit lacks unique current candidate")
            candidate=state["candidate"]
            require((p["session_slot"],p["candidate_epoch"])==state["epoch"] and p["request_digest"]==candidate["request_digest"]
                    and p["capability"]==native.CAPABILITY,"native committed candidate/capability")
            if p["consumed_digest"] not in collectors:raise MissingComponent("native_commit_actual_durable_collector_ACK")
            entry=collectors[p["consumed_digest"]];e=entry["evidence"];cp=e["checkpoint"]
            require(entry["binding"]["miner_id"]==miner and entry["binding"]["front"]==w["front_id"]
                    and entry["binding"]["pid"]==w["process"]["pid"]
                    and entry["binding"]["start_time_ticks"]==w["process"]["start_time_ticks"]
                    and entry["collector_pubkey"]==w["collector_pubkey"] and entry["binding"]["pool_pubkey"]==w["pool_pubkey"],"native collector wallet/process/authority route")
            length=entry["bundle"]["job"]["extranonce_len"]-len(unhex(candidate["job"]["extranonce_prefix"]))
            received=native.received_from_snapshot(candidate["job"],length)
            scan,_,_=native.keys(w["identity"],w["collector_pubkey"],w["pool_pubkey"])
            facts=verify_public_evidence(e,cp["attempt"],received_digest(received),w["pool_pubkey"],w["collector_pubkey"],scan,now)
            require(entry["binding"]["scan_pubkey"]==scan and p["window"]==facts
                    and p["job_id"]==cp["job_id"]==native.full_job_id(received) and p["generation"]==cp["generation"]
                    and p["checkpoint_digest"]==checkpoint_digest(cp).hex() and p["consumed_digest"]==consumed_digest(e["consumed"]).hex()
                    and p["committed_digest"]==committed_digest(e["committed"]["attempt"],e["committed"]["ack_digest"]).hex(),"native signed job/window/receipt digest mismatch")
            require(now<p["expires_at_unix_ms"]<=min(cp["expires_at_unix_ms"],now+10000),"native effective expiry")
            if require_durable and not p.get("durable_evidence"):
                raise MissingComponent("actual_native_commit_DurableHandle_missing")
            if candidate["request_digest"] in outer:
                from native_retention import RetainedExchange
                retained=outer[candidate["request_digest"]]
                if isinstance(retained,RetainedExchange):
                    if not p.get("durable_evidence"):raise MissingComponent("actual_native_commit_DurableHandle_missing")
                    r,response,proof=retained.join(p["durable_evidence"],binding,state["candidate_wall_ms"],now)
                else:
                    if require_durable or p.get("durable_evidence"):raise MissingComponent("actual_readonly_retained_native_exchange_required")
                    r,response=retained
                    proof=native.verify_response(r,response,w["collector_pubkey"],w["pool_pubkey"],now)
                native.authorize(binding["registry"],w,r,binding["observed_miner"],now)
                require(r["job"]==received and r["session_epoch"]==p["session_slot"] and r["candidate_epoch"]==p["candidate_epoch"]
                        and r["miner_id"]==miner and r["connection_id"]==con and response["receipt"]["evidence"]==e,"native retained outer/source event correlation")
                require(all(p[k]==v for k,v in proof.items()),"native VerifiedActivation/event mismatch")
                nonce=bytes(r["nonce"]);require(nonce not in nonces,"native attempt reused across candidates");nonces.add(nonce)
            else:missing.add("retained_actual_native_request_and_signed_Approved_bytes")
            state["commit"]=p;state["commit_seq"]=seq;coverage.add(key);checked.append({"miner_id":miner,**p,"event_id":seq})
        elif name=="job_activated":
            require(state is not None and state["commit"] is not None and not state["activated"],"native activation before committed proof")
            require(canonical_json(p["job"])==canonical_json(state["candidate"]["job"]),"native activated immutable job changed")
            require(now<state["commit"]["expires_at_unix_ms"],"native activation expired")
            state["activated"]=True
            activated.add(state["candidate"]["request_digest"])
        elif name=="source_audit_unknown":
            # Timeout invalidation may advance the epoch before this event.
            if state is not None:require((p["session_slot"],p["candidate_epoch"])>=state["epoch"],"native stale Unknown")
            if state is not None and not state["activated"]:abandoned[state["candidate"]["request_digest"]]="unknown:"+p["code"]
            states.pop(key,None)
        elif name=="audit_first_hash":
            if state is None or (p["session_slot"],p["candidate_epoch"])!=state["epoch"]:
                raise MissingComponent("first_hash_emission_after_candidate_clear_requires_hash_time_evidence")
            require(state["activated"] and state["commit"] is not None,"native first hash before committed activation")
            if now>=state["commit"]["expires_at_unix_ms"]:
                raise MissingComponent("first_hash_emission_after_expiry_requires_hash_time_evidence")
            require(p["request_digest"]==state["candidate"]["request_digest"] and p["job_id"]==state["candidate"]["job"]["job_id"],"native first-hash candidate mismatch")
            require(p["worker_id"] in {v["id"] for v in config["workers"]} and p["worker_id"] not in state["first"],"native duplicate/foreign first-hash worker")
            state["first"].add(p["worker_id"]);workers.add((miner,con,p["worker_id"]))
        elif name=="submitted":
            share=p["share"]
            require(state is not None and state["activated"] and share["worker_id"] in state["first"],"native submitted share without first-hash/activation")
            require(share["job_id"]==state["candidate"]["job"]["job_id"] and share["session_slot"]==state["epoch"][0]
                    and share["target_generation"]==state["candidate"]["job"]["target_generation"],"native submitted stale job")
            if now>=state["commit"]["expires_at_unix_ms"]:raise MissingComponent("native_submission_expiry_order")
    expected_workers={(m,c,w["id"]) for (m,c),b in bindings.items() for w in b["config"]["workers"]}
    if coverage!=set(bindings) or workers!=expected_workers:missing.add("actual_committed_activation_first_hash_for_every_declared_worker")
    if require_durable and set(outer)-candidates:missing.add("retained_record_lacks_actual_candidate_event")
    return {"scope":"native_event_collector_and_outer_binding_only","status":"INCONCLUSIVE" if missing else "PASSED",
        "missing_components":sorted(missing),"verified_commits":checked,"event_cursors":{m:v[0] for m,v in cursors.items()},
        "first_hash_workers":[list(v) for v in sorted(workers)],"full_Q_campaign_still_required":True,
        "durable_event_join_required":require_durable,
        "retained_not_activated":[{"request_digest":digest,"reason":abandoned.get(digest,"no_activation_event")} for digest in sorted(set(outer)-activated)]}
