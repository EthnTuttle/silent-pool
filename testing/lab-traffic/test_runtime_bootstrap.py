"""Actual Runtime command shapes; synthetic orchestration only, no funded run."""
import copy
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from types import SimpleNamespace

from case_contract import canonical_json, semantic_scenario, required
from campaign import load_controller
from oracle import Inconclusive, Invalid
from runtime_bootstrap import RuntimeBootstrap, selected, payload_hash, source_admission, G105_RUNTIME_SHA256

BASE=None


def fixture(expanded=True):
    kind={"type":"bootstrap_fund","driver_id":"funding-actor","node_id":"node-b"}
    scenario={"run":{"id":"run"},"expanded_access":expanded,"nodes":[{"id":"node-a"},{"id":"node-b"}],
        "traffic":{"driver":{"actors":[{"id":"funding-actor","hosts_bootstrap_funding":True}],
            "bootstrap":{"total_attempts":5000,"hashes_per_second":40,"cancel_grace_secs":10}}},
        "phases":[{"id":"declared-funding","commands":[{"command_id":"original-phase-id","kind":kind}]}]}
    command={"command_id":"original-phase-id","run_id":"run","issued_at_wall":100,"actor":"runtime-phase","kind":kind}
    begin={"record":"begin","seq":1,"command":command,"payload_hash":payload_hash(command,"controller" if expanded else None)}
    if expanded:begin.update(hash_scheme="expanded_v1",principal="controller")
    budget={"operation_id":"original-phase-id","driver_id":"funding-actor","boot_id":"unit-boot","deadline_monotonic_ms":120000,
        "total_attempts":5000,"hashes_per_second":40,"cancel_grace_secs":10}
    rows=[begin,{"record":"hash_start","seq":2,"operation":budget},
        {"record":"hash_reserve","seq":3,"operation_id":"original-phase-id","driver_incarnation":"real-format-unit-incarnation",
         "batch_id":1,"reserved_attempts":100,"cumulative_reserved":100}]
    return scenario,rows


def encoded(rows):return b"".join(canonical_json(r)+b"\n" for r in rows)


def finish(rows,state="succeeded"):
    outcome={"command_id":"original-phase-id","state":state,"detail":"bootstrap unit terminal","resolved_at":200,"journal_seq":0}
    rows.append({"record":"finish","seq":len(rows)+1,"command_id":"original-phase-id","outcome":outcome})
    detail={"schema_version":1,"phase":"declared-funding","outcome":outcome,"link":None,"pre_partition_common_anchor":None,
        "controller":{"reachable":True,"url":"http://unit/health","error":None},"nodes":[],"fronts":[],"broker_evidence":None}
    evidence={"run_id":"run","event_id":1,"source":"topology","source_cursor":"topology-phase-0","actor":{"kind":"observer","id":"runtime-topology"},
        "command_id":None,"occurred_at_wall":200,"occurred_at_mono":2000,"ingested_at":200,"payload":{"kind":"topology_phase","data":detail}}
    return encoded([evidence])


class RuntimeBootstrapTests(unittest.TestCase):
    def test_Runtime_uses_original_declared_id_driver_node_and_controller_principal(self):
        scenario,rows=fixture();owner=RuntimeBootstrap(scenario);result=owner.observe(encoded(rows))
        self.assertEqual(result["operation_id"],"original-phase-id");self.assertEqual(result["principal"],"controller")
        self.assertEqual(owner.plan["kind"]["node_id"],"node-b")
        self.assertEqual(result["actor_command_id"],"bootstrap/"+hashlib.sha256(b"original-phase-id").hexdigest()+"/00000000")
        self.assertEqual(result["state"],"observed_in_flight");self.assertIsNone(result["terminal"])

    def test_missing_duplicate_or_unrelated_declared_phases_cannot_remove_route_guard(self):
        scenario,_=fixture()
        for change in ("empty","duplicate","payment","generated_id","foreign_run"):
            value=copy.deepcopy(scenario)
            if change=="empty":value["phases"]=[]
            elif change=="duplicate":value["phases"]*=2
            elif change=="payment":value["phases"][0]["commands"][0]["kind"]["type"]="send_user_payment"
            elif change=="generated_id":del value["phases"][0]["commands"][0]["command_id"]
            else:value["phases"][0]["commands"][0]["run_id"]="foreign"
            with self.subTest(change=change),self.assertRaises(Invalid):selected(value)
        with self.assertRaises(Inconclusive):selected(scenario,entrypoint="run")

    def test_expanded_hash_uses_controller_not_operator_or_actor_text(self):
        scenario,rows=fixture();command=rows[0]["command"]
        kind=b'{"type":"bootstrap_fund","driver_id":"funding-actor","node_id":"node-b"}'
        self.assertEqual(rows[0]["payload_hash"],hashlib.sha256(b"expanded_v1\ncontroller\nrun\n"+kind).hexdigest())
        for principal in ("operator","runtime-phase"):
            changed=copy.deepcopy(rows);changed[0].update(principal=principal,payload_hash=payload_hash(command,principal))
            with self.assertRaises(Invalid):RuntimeBootstrap(scenario).observe(encoded(changed))
        legacy,legacy_rows=fixture(False)
        self.assertIsNone(RuntimeBootstrap(legacy).observe(encoded(legacy_rows))["principal"])

    def test_second_bootstrap_even_same_kind_and_new_ID_is_rejected(self):
        scenario,rows=fixture();other=copy.deepcopy(rows[0]);other["seq"]=4;other["command"]["command_id"]="second-bootstrap";rows.append(other)
        with self.assertRaisesRegex(Invalid,"second bootstrap"):RuntimeBootstrap(scenario).observe(encoded(rows))

    def test_budget_deadline_and_debit_survive_pause_status_and_actor_restart(self):
        scenario,rows=fixture();owner=RuntimeBootstrap(scenario);initial=owner.observe(encoded(rows))
        rows.append({"record":"hash_reconcile","seq":4,"operation_id":"original-phase-id","driver_incarnation":"real-format-unit-incarnation","batch_id":1,"measured_attempts":None})
        rows.append({"record":"hash_reserve","seq":5,"operation_id":"original-phase-id","driver_incarnation":"next-incarnation","batch_id":2,"reserved_attempts":80,"cumulative_reserved":180})
        current=owner.observe(encoded(rows))
        self.assertEqual(current["budget"],initial["budget"]);self.assertEqual(current["debits"]["charged_reserved"],180)
        self.assertEqual(current["debits"]["charged_unresolved"],180)
        rows[1]["operation"]["deadline_monotonic_ms"]+=10000
        with self.assertRaisesRegex(Invalid,"changed/pruned"):owner.observe(encoded(rows))

    def test_second_hash_start_or_new_operation_cannot_reset_original_budget(self):
        scenario,rows=fixture()
        changed=copy.deepcopy(rows);new=copy.deepcopy(rows[1]);new["seq"]=4;changed.append(new)
        with self.assertRaises(Invalid):RuntimeBootstrap(scenario).observe(encoded(changed))
        rows[2]["operation_id"]="another"
        with self.assertRaises(Invalid):RuntimeBootstrap(scenario).observe(encoded(rows))

    def test_original_terminal_and_matching_Runtime_phase_evidence_are_both_required(self):
        scenario,rows=fixture();owner=RuntimeBootstrap(scenario);owner.observe(encoded(rows));evidence=finish(rows)
        result=owner.observe(encoded(rows),evidence,require_phase=True)
        self.assertEqual(result["state"],"succeeded");self.assertEqual(result["phase"]["outcome"],rows[-1]["outcome"])
        with self.assertRaises(Inconclusive):owner.observe(encoded(rows),None,require_phase=True)
        changed=json.loads(evidence);changed["payload"]["data"]["outcome"]["state"]="cancelled"
        with self.assertRaises(Invalid):owner.observe(encoded(rows),encoded([changed]),require_phase=True)

    def test_g105_running_phase_and_conflict_are_never_native_success(self):
        scenario,rows=fixture();evidence=finish(rows,"cancelled");phase=json.loads(evidence);phase["payload"]["data"]["outcome"]["state"]="running"
        with self.assertRaisesRegex(Inconclusive,"wait_bug"):RuntimeBootstrap(scenario).observe(encoded(rows),encoded([phase]),True)
        for state in ("rejected_conflict","rejected_precondition"):
            _,rows=fixture();finish(rows,state)
            with self.assertRaisesRegex(Inconclusive,"replay"):RuntimeBootstrap(scenario).observe(encoded(rows))
        _,rows=fixture();rows.append({"record":"conflict","seq":4,"command_id":"original-phase-id","existing_hash":"11"*32,"incoming_hash":"22"*32})
        with self.assertRaisesRegex(Inconclusive,"not_original"):RuntimeBootstrap(scenario).observe(encoded(rows))

    def test_declared_per_case_run_binding_normalizes_only_the_matching_run(self):
        scenario,_=fixture();scenario["phases"][0]["commands"][0]["run_id"]="run"
        fresh=copy.deepcopy(scenario);fresh["run"]["id"]="fresh";fresh["phases"][0]["commands"][0]["run_id"]="fresh"
        self.assertEqual(semantic_scenario(scenario),semantic_scenario(fresh))
        fresh["phases"][0]["commands"][0]["run_id"]="foreign"
        self.assertNotEqual(semantic_scenario(scenario),semantic_scenario(fresh))

    def test_real_controller_bootstrap_operation_is_observation_only_and_HTTP_resubmit_refuses(self):
        scenario,rows=fixture();module=load_controller();c=object.__new__(module.Controller)
        c.phase_bootstrap=RuntimeBootstrap(scenario);c.args=SimpleNamespace(start_timeout=10);calls=[]
        c.bootstrap_status=lambda **kw:c.phase_bootstrap.observe(encoded(rows))
        c.until=lambda callback,*args,**kwargs:callback()
        c.http=lambda *args,**kwargs:calls.append((args,kwargs))
        self.assertEqual(c.bootstrap_operation(),"original-phase-id");self.assertEqual(c.driver,"funding-actor");self.assertEqual(calls,[])
        with self.assertRaisesRegex(Invalid,"no HTTP resubmission"):
            c.prepare_command("bootstrap",scenario["phases"][0]["commands"][0]["kind"])
        self.assertEqual(calls,[])

    def test_pause_resume_hooks_target_original_id_without_changing_budget(self):
        scenario,rows=fixture();owner=RuntimeBootstrap(scenario);before=owner.observe(encoded(rows))
        for action in ("pause","resume"):
            command={"command_id":"operator-"+action,"run_id":"run","issued_at_wall":150,"actor":"operator-actor",
                "kind":{"type":"bootstrap_control","operation_id":owner.plan["command_id"],"action":action}}
            # Separate authenticated control command; never replay BootstrapFund.
            digest=hashlib.sha256(b"expanded_v1\noperator\nrun\n"+json.dumps(command["kind"],separators=(",",":")).encode()).hexdigest()
            rows.append({"record":"begin","seq":len(rows)+1,"command":command,"payload_hash":digest,"hash_scheme":"expanded_v1","principal":"operator"})
        after=owner.observe(encoded(rows))
        self.assertEqual(after["budget"],before["budget"]);self.assertEqual(after["debits"],before["debits"])
        self.assertEqual([c["action"] for c in after["operator_controls"]],["pause","resume"])
        rows[-1]["principal"]="controller"
        with self.assertRaises(Invalid):RuntimeBootstrap(scenario).observe(encoded(rows))
        self.assertIn("runtime_bootstrap_ownership",required("bootstrap-stop"))

    def test_known_broken_and_unreviewed_new_Runtime_binaries_cannot_launch(self):
        with tempfile.TemporaryDirectory(prefix="t3-runtime-source-",dir=BASE) as tmp:
            path=Path(tmp)/"manifest.json"
            for runtime,reason in ((G105_RUNTIME_SHA256,"no_repeat_execution"),("12"*32,"tested_CLI")):
                path.write_bytes(canonical_json({"files":[{"path":"crates/spc-lab/src/runtime.rs","sha256":runtime}]}))
                binary={"source_manifest":str(path),"source_manifest_sha256":hashlib.sha256(path.read_bytes()).hexdigest(),"sha256":"34"*32}
                with self.assertRaisesRegex(Inconclusive,reason):source_admission(binary)

    def test_corrected_runtime_and_CLI_labels_cannot_admit_a_counterfeit_source_manifest(self):
        from runtime_bootstrap import FIXED_RUNTIME_SHA256, FIXED_CLI_SHA256
        with tempfile.TemporaryDirectory(prefix="t3-runtime-counterfeit-",dir=BASE) as tmp:
            path=Path(tmp)/"manifest.json"
            path.write_bytes(canonical_json({"files":[{"path":"crates/spc-lab/src/runtime.rs","sha256":FIXED_RUNTIME_SHA256}],
                "fixed":True,"parent_status":"passed"}))
            binary={"source_manifest":str(path),"source_manifest_sha256":hashlib.sha256(path.read_bytes()).hexdigest(),
                    "sha256":FIXED_CLI_SHA256,"path":"/nonexistent/counterfeit-cli"}
            with self.assertRaisesRegex(Inconclusive,"tested_CLI"):source_admission(binary)


if __name__=="__main__":unittest.main()
