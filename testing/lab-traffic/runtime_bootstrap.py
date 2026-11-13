"""Observe Runtime's one declared bootstrap; never resubmit across principals.

g105 establishes these command/phase shapes but has a known phase-wait bug.
The exact corrected Nix tuple below is admitted for base Runtime readiness.
"""
import copy
import hashlib
import json
from pathlib import Path

from case_contract import canonical_json
from oracle import Inconclusive, integer, require, strict_json, unhex
from source_feed_codec import fields
from source_jobs import MissingComponent
from traffic_audit import read_commands
from journal import hash_budget

ENTRYPOINT="runtime"
G105_RUNTIME_SHA256="fb0a5ccc9ad0c16ed3f104de1cd1bf7b7d16f1414c4257279aec8aab0579ce65"
FIXED_RUNTIME_SHA256="e4c57fcd9b2f8bcf35d7cb2b60afd3da7f5327ca5d9084990cbce01ac3938478"
FIXED_CLI_SHA256="52e45eabfbcbc53921c670a19b36cb6940a7228f5d8b4bc15c43556bedce402b"
FIXED_MANIFEST_SHA256="1e3b3ddae2e19e9dfee66150e5fd7adfc02c46649a479e8b8ce5701540f3175f"
FIXED_BUNDLE_SHA256="4d3778e8d4c65cffa95af483828fe1013a1d4307da32b3469801ce1624f036fd"
# Verified against all 833 Nix source rows, the actual ELF, one-source
# derivation, sealed build/test logs and the three execute_phases regressions.
# A semantic marker, parent status string or novel tuple cannot admit a CLI.
ADMITTED_RUNTIME_FIXES=frozenset({(FIXED_RUNTIME_SHA256,FIXED_CLI_SHA256,FIXED_MANIFEST_SHA256)})


def source_admission(binary,*,bundle=None):
    raw=Path(binary["source_manifest"]).read_bytes()
    require(hashlib.sha256(raw).hexdigest()==binary["source_manifest_sha256"],"Runtime source manifest seal")
    manifest=strict_json(raw);rows={r["path"]:r["sha256"] for r in manifest["files"]}
    require(len(rows)==len(manifest["files"]),"Runtime source manifest duplicate path")
    runtime=rows.get("crates/spc-lab/src/runtime.rs")
    if runtime==G105_RUNTIME_SHA256:
        raise MissingComponent("g105_Runtime_phase_wait_bug_no_repeat_execution")
    from runtime_profile import MANIFEST_SHA256 as QUALIFIED_MANIFEST, admit
    if binary['source_manifest_sha256']==QUALIFIED_MANIFEST:
        return admit(binary,bundle)
    if (runtime,binary["sha256"],binary["source_manifest_sha256"]) not in ADMITTED_RUNTIME_FIXES:
        raise MissingComponent("corrected_Runtime_phase_wait_source_and_tested_CLI_binding_pending")
    from storage import sha_file
    require(sha_file(Path(manifest["nix_src_root"])/"crates/spc-lab/src/runtime.rs")==runtime,
            "admitted Runtime source bytes changed")
    require(sha_file(binary["path"])==binary["sha256"],"admitted Runtime executable bytes changed")
    if bundle is not None:
        require(set(bundle)=={'path','sha256'} and bundle['sha256']==FIXED_BUNDLE_SHA256
            and sha_file(bundle['path'])==bundle['sha256'],'legacy Runtime selected bundle mismatch')
    return {"entrypoint":ENTRYPOINT,"profile_id":"legacy-base-runtime-fix","runtime_source_sha256":runtime,"binary_sha256":binary["sha256"],"phase_owner":"runtime",
            "source_manifest_sha256":binary["source_manifest_sha256"],"verified_bundle_sha256":FIXED_BUNDLE_SHA256,
            "scope":"base_diagnostic_Runtime_entrypoint_only","full_native_audited_campaign":False}


def selected(scenario,entrypoint=ENTRYPOINT):
    if entrypoint!=ENTRYPOINT:
        raise MissingComponent("selected_entrypoint_auto_phase_contract_not_admitted")
    commands=[]
    for phase in scenario["phases"]:
        require(phase.get("convergence") is None,"campaign cannot autoexecute extra convergence commands")
        for raw in phase["commands"]:commands.append((phase["id"],raw))
    require(len(commands)==1,"Runtime campaign requires exactly one declared bootstrap and no other automatic command")
    phase,raw=commands[0];require(isinstance(raw,dict) and set(raw)<={"command_id","run_id","issued_at_wall","actor","kind"},"declared phase command fields")
    require({"command_id","kind"}<=set(raw),"declared bootstrap must retain its original explicit command id")
    require(isinstance(raw["command_id"],str) and 0<len(raw["command_id"])<=128,"declared bootstrap command id")
    kind=raw["kind"];fields(kind,("type","driver_id","node_id"))
    require(kind["type"]=="bootstrap_fund","Runtime campaign phase must be bootstrap")
    hosts=[a["id"] for a in scenario["traffic"]["driver"]["actors"] if a["hosts_bootstrap_funding"]]
    require(hosts==[kind["driver_id"]] and kind["node_id"] in {n["id"] for n in scenario["nodes"]},"declared bootstrap actual host/node route")
    require(raw.get("run_id",scenario["run"]["id"])==scenario["run"]["id"],"declared phase foreign run")
    if "issued_at_wall" in raw:integer(raw["issued_at_wall"])
    return {"entrypoint":entrypoint,"phase_id":phase,"command_id":raw["command_id"],"run_id":scenario["run"]["id"],
        "actor":raw.get("actor","runtime-phase"),"principal":"controller" if scenario.get("expanded_access",False) else None,
        "kind":copy.deepcopy(kind),"declared":copy.deepcopy(raw),"budget":copy.deepcopy(scenario["traffic"]["driver"]["bootstrap"]),
        "actor_command_id":"bootstrap/"+hashlib.sha256(raw["command_id"].encode()).hexdigest()+"/00000000"}


def payload_hash(command,principal):
    k=command["kind"]
    # Actual CommandKind serde struct order, not sorted JSON.
    if k["type"]=="bootstrap_fund":
        value={"type":"bootstrap_fund","driver_id":k["driver_id"],"node_id":k["node_id"]}
    else:
        fields(k,("type","operation_id","action"))
        require(k["type"]=="bootstrap_control" and k["action"] in {"pause","resume","cancel"},"closed native bootstrap control")
        value={"type":"bootstrap_control","operation_id":k["operation_id"],"action":k["action"]}
    encoded=json.dumps(value,ensure_ascii=False,separators=(",",":")).encode()
    prefix=b"" if principal is None else b"expanded_v1\n"+principal.encode()+b"\n"+command["run_id"].encode()+b"\n"
    return hashlib.sha256(prefix+encoded).hexdigest()


def phase_outcome(raw,plan,terminal):
    if not raw or not raw.endswith(b"\n"):raise MissingComponent("complete_actual_Runtime_phase_evidence")
    matches=[]
    for seq,line in enumerate(raw.splitlines(),1):
        row=strict_json(line)
        require(row["run_id"]==plan["run_id"] and row["event_id"]==seq,"Runtime phase evidence run/prefix")
        payload=row["payload"]
        if payload["kind"]=="event_lost":raise MissingComponent("Runtime_phase_evidence_gap")
        if payload["kind"]!="topology_phase":continue
        detail=payload["data"]
        if detail["outcome"]["command_id"]!=plan["command_id"]:continue
        require(row["source"]=="topology" and row["actor"]=={"kind":"observer","id":"runtime-topology"}
                and detail["schema_version"]==1 and detail["phase"]==plan["phase_id"],"Runtime actual phase owner/id")
        if detail["outcome"]["state"] in {"accepted","running"}:
            raise MissingComponent("Runtime_phase_observed_nonterminal_outcome_wait_bug")
        require(terminal is not None and detail["outcome"]==terminal["outcome"],"Runtime phase outcome differs from original durable terminal")
        matches.append(row)
    if not matches:raise MissingComponent("original_Runtime_phase_terminal_observation")
    require(len(matches)==1,"Runtime phase operation executed more than once")
    return {"event_id":matches[0]["event_id"],"phase_id":plan["phase_id"],"outcome":matches[0]["payload"]["data"]["outcome"]}


class RuntimeBootstrap:
    def __init__(self,scenario):
        self.plan=selected(scenario);self.prefix=b"";self.initial_budget=None

    def observe(self,raw,evidence=None,require_phase=False):
        if not raw:raise MissingComponent("original_Runtime_bootstrap_begin_not_observed")
        require(raw.startswith(self.prefix),"original Runtime command history changed/pruned")
        rows=read_commands(raw);self.prefix=raw;p=self.plan
        begins=[r for r in rows if r["record"]=="begin" and r["command"]["kind"]["type"]=="bootstrap_fund"]
        if not begins:raise MissingComponent("original_Runtime_bootstrap_begin_not_observed")
        require(len(begins)==1,"second bootstrap operation/new ID is forbidden")
        begin=begins[0];command=begin["command"]
        require(command["command_id"]==p["command_id"] and command["run_id"]==p["run_id"]
                and command["actor"]==p["actor"] and command["kind"]==p["kind"],"Runtime original declared command mismatch")
        if "issued_at_wall" in p["declared"]:require(command["issued_at_wall"]==p["declared"]["issued_at_wall"],"Runtime declared issue time changed")
        require(begin.get("principal")==p["principal"] and begin.get("hash_scheme")==("expanded_v1" if p["principal"] else None)
                and begin["payload_hash"]==payload_hash(command,p["principal"]),"Runtime original principal/hash binding")
        controls=[]
        for row in rows:
            if row["record"]!="begin" or row["command"]["kind"]["type"]!="bootstrap_control":continue
            control=row["command"];principal="operator" if p["principal"] else None
            require(control["run_id"]==p["run_id"] and control["command_id"]!=p["command_id"]
                    and control["kind"]["operation_id"]==p["command_id"] and row.get("principal")==principal
                    and row.get("hash_scheme")==("expanded_v1" if principal else None)
                    and row["payload_hash"]==payload_hash(control,principal),"operator control must bind original Runtime operation under its own principal")
            controls.append({"command_id":control["command_id"],"operation_id":p["command_id"],"action":control["kind"]["action"],"principal":principal,"begin_seq":row["seq"]})
        conflicts=[r for r in rows if r["record"]=="conflict" and r["command_id"]==p["command_id"]]
        if conflicts:raise MissingComponent("bootstrap_ID_conflict_is_not_original_operation_outcome")
        starts=[r for r in rows if r["record"]=="hash_start"]
        if not starts:raise MissingComponent("original_Runtime_bootstrap_hash_start")
        require(len(starts)==1 and starts[0]["operation"]["operation_id"]==p["command_id"] and starts[0]["seq"]>begin["seq"],"Runtime bootstrap budget reset/new operation")
        budget=starts[0]["operation"]
        fields(budget,"operation_id driver_id boot_id deadline_monotonic_ms total_attempts hashes_per_second cancel_grace_secs".split())
        require(budget["driver_id"]==p["kind"]["driver_id"] and budget["boot_id"] and integer(budget["deadline_monotonic_ms"],1)>0,"Runtime original budget driver/deadline")
        for k in ("total_attempts","hashes_per_second","cancel_grace_secs"):
            require(integer(budget[k],1)==p["budget"][k],"Runtime original bootstrap budget changed")
        if self.initial_budget is None:self.initial_budget=copy.deepcopy(budget)
        require(budget==self.initial_budget,"Runtime original deadline/hash debit reset")
        reservations=[r for r in rows if r["record"] in {"hash_reserve","hash_reconcile"}]
        require(all(r["operation_id"]==p["command_id"] for r in reservations),"unrelated bootstrap hash authorization")
        debits=hash_budget(rows,p["budget"]["total_attempts"]) if reservations else {"charged_reserved":0,"measured_executed":0,"charged_unresolved":0}
        terminals=[r for r in rows if r["record"]=="finish" and r["command_id"]==p["command_id"]]
        require(len(terminals)<=1,"Runtime original terminal rewritten")
        terminal=terminals[0] if terminals else None
        if terminal:
            outcome=terminal["outcome"];fields(outcome,"command_id state detail resolved_at journal_seq".split())
            integer(outcome["journal_seq"],0,0);integer(outcome["resolved_at"])
            require(isinstance(outcome["detail"],str),"Runtime terminal detail type")
            require(outcome["command_id"]==p["command_id"] and terminal["seq"]>starts[0]["seq"]
                    and outcome["journal_seq"]==0,"Runtime original terminal sequence/id")
            require(outcome["state"] in {"succeeded","failed","cancelled","blocked_on_height","unknown","rejected_precondition","rejected_conflict"},"Runtime durable nonterminal finish")
            if outcome["state"] in {"rejected_conflict","rejected_precondition"}:
                raise MissingComponent("bootstrap_rejected_replay_is_not_native_execution_outcome")
        phase=phase_outcome(evidence,p,terminal) if require_phase else None
        return {"operation_id":p["command_id"],"actor_command_id":p["actor_command_id"],"principal":p["principal"],
            "state":terminal["outcome"]["state"] if terminal else "observed_in_flight", "begin":begin,"budget":copy.deepcopy(budget),
            "debits":debits,"operator_controls":controls,"terminal":terminal,"phase":phase,"command_prefix_sha256":hashlib.sha256(raw).hexdigest(),"command_prefix_bytes":len(raw)}
