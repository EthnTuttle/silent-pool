import copy
import hashlib
import json
from pathlib import Path
import tempfile
import unittest

import ap5_v2
from case_contract import canonical_json
from oracle import Invalid, compact_size
from test_oracle import block_bytes, coinbase
import test_audit_regressions

BASE=None


class AP5V2Tests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.funding=test_audit_regressions.FundedOutcomeRegressions
        cls.funding.setUpClass()

    def transaction_views(self):
        f=self.funding
        raw=block_bytes(bytes.fromhex(f.chain.tip["hash"])[::-1],
                        [coinbase(5000001000,tag=b"\2\x66\0"),f.raw],1600000102)
        parent=f.chain.blocks[1];tx=parent["transactions"][0]
        parent_raw=parent["raw"]+compact_size(len(parent["transactions"]))+b"".join(t["raw"] for t in parent["transactions"])
        record={"raw":f.raw.hex(),"txid":f.tx["txid"],"raw_block":raw.hex(),
                "block_hash":ap5_v2.decode_block(raw,f.chain.policy)["hash"],"height":102,"fee_sats":1000,
                "inputs":[{"txid":tx["txid"],"vout":0,"raw":tx["raw"].hex(),"raw_block":parent_raw.hex(),
                           "block_hash":parent["hash"],"height":1,"amount_sats":5000000000}]}
        return [record,copy.deepcopy(record)]

    def test_independent_two_node_raw_bytes_fees_and_maturity(self):
        views=self.transaction_views();result=ap5_v2.raw_transaction(views,self.funding.chain.policy)
        self.assertEqual(result["fee_sats"],1000)
        self.assertEqual(result["txid"],self.funding.tx["txid"])
        for field,value in (("fee_sats",0),("height",100),("block_hash","00"*32)):
            altered=copy.deepcopy(views);altered[1][field]=value
            with self.assertRaises(Invalid):ap5_v2.raw_transaction(altered,self.funding.chain.policy)

    def test_missing_or_changed_prevout_proof_does_not_become_zero_fee(self):
        for change in ("missing","value","bytes"):
            views=self.transaction_views()
            if change=="missing":views[0]["inputs"]=[]
            elif change=="value":views[0]["inputs"][0]["amount_sats"]+=1
            else:views[0]["inputs"][0]["raw"]="00"
            with self.assertRaises(Invalid):ap5_v2.raw_transaction(views,self.funding.chain.policy)

    def test_v2_prepared_envelope_requires_external_seal_and_is_inconclusive(self):
        with tempfile.TemporaryDirectory(prefix="t3-ap5-v2-",dir=BASE) as tmp:
            root=Path(tmp);previous="00"*32;lines=[]
            for seq,(label,value) in enumerate((("declared-default",{}),("result",{"schema":"spc.lab.ap5.result/v2",
                    "scope":"full","status":"prepared-not-executed","whole_lab_complete":False})),1):
                name=f"{seq:06d}-{label}.json";raw=canonical_json(value)+b"\n";(root/name).write_bytes(raw)
                row={"seq":seq,"file":name,"sha256":hashlib.sha256(raw).hexdigest(),"previous":previous,"unix_ms":seq}
                previous=hashlib.sha256(canonical_json(row)).hexdigest()
                lines.append(canonical_json({**row,"record_sha256":previous})+b"\n")
            events=b"".join(lines);(root/"events.jsonl").write_bytes(events)
            seal=canonical_json({"root":str(root),"events_sha256":hashlib.sha256(events).hexdigest(),"last_seq":2,"last_digest":previous})
            path=root/"seal.json";path.write_bytes(seal)
            result=ap5_v2.inspect(path,hashlib.sha256(seal).hexdigest(),{})
            self.assertEqual(result["status"],"INCONCLUSIVE")
            self.assertEqual(result["source_result"],"prepared-not-executed")
            with self.assertRaises(Invalid):ap5_v2.inspect(path,"00"*32,{})

    def inventory(self):
        processes=[]
        for role,count in {**ap5_v2.ROLES,"controller":1}.items():
            for _ in range(count):
                pid=len(processes)+1
                processes.append({"pid":pid,"start_time_ticks":pid+100,"role":role,"run_token":str(pid),"status":"gone_or_reused"})
        listeners={key:{"host":"127.0.0.1","port":10000+i} for i,key in enumerate(sorted(ap5_v2.LISTENERS))}
        resources={"listeners":listeners,"postgres_databases":[{"port":20000},{"port":20001}],"redis_instances":[{"port":20002},{"port":20003}]}
        active={"processes":[{k:p[k] for k in ("pid","start_time_ticks","role","run_token")} for p in processes if p["role"]!="controller"],"resources":resources}
        endpoints=list(listeners.values())+[{"host":"127.0.0.1","port":p} for p in range(20000,20004)]
        inv={"active_manifest":active,"coverage_errors":[],"processes":processes,
             "listeners":[{"kind":"planned_tcp_endpoint",**e,"released":True} for e in endpoints],
             "observed_socket_inodes":[],"observed_unix_paths":[],"run_tokens":[p["run_token"] for p in processes],
             "observed_file_inodes":[],"file_references":[],"locks":[],"capture_errors":[],"unreadable_same_uid_processes":[]}
        receipt={"schema":"spc.lab.ap5.cleanup/v2","inventory":inv,"inventory_sha256":hashlib.sha256(canonical_json(inv)).hexdigest(),
                 "scope":"offline fixture inventory","exhaustive":True,"all_resources_released":True,"status":"passed"}
        return receipt,[("active-resource-census",active,{})]

    def test_exhaustive_cleanup_v2_replays_census_not_status_flags(self):
        receipt,records=self.inventory()
        self.assertEqual(ap5_v2.cleanup(receipt,records)["status"],"PASSED")
        for alteration in ("role","listener","tool","files"):
            r,records=self.inventory();inv=r["inventory"]
            if alteration=="role":inv["active_manifest"]["processes"].pop()
            elif alteration=="listener":inv["listeners"].pop()
            elif alteration=="tool":records.append(("tool-intent",{},{}))
            else:inv["file_references"].append({"pid":1})
            r["inventory_sha256"]=hashlib.sha256(canonical_json(inv)).hexdigest()
            with self.subTest(alteration=alteration),self.assertRaises(Invalid):ap5_v2.cleanup(r,records)

    def test_signed_synthetic_vectors_do_not_require_live_corpus_or_establish_funded_board(self):
        handoff={"schema":"spc.lab.ap5.handoff/v2","contract":{"sha256":ap5_v2.RETAINED_CONTRACT_SHA256},
                 "contract_predecessor":{"sha256":ap5_v2.RETAINED_CONTRACT_PREDECESSOR},"verifier_lock":{"sha256":ap5_v2.VERIFIER_LOCK},
                 "requires_live_corpus_for_compilation":False,"rust_crypto_tests_prepared":6,"rust_crypto_tests_executed":False}
        predecessor={"schema":"spc.lab.ap5.handoff/v2","contract":{"sha256":ap5_v2.RETAINED_CONTRACT_PREDECESSOR},
                     "verifier_lock":{"sha256":ap5_v2.VERIFIER_LOCK},"requires_live_corpus_for_compilation":False,
                     "requires_live_corpus_for_conformance_tests":False,"fixture_kind":"synthetic_signed_conformance"}
        with tempfile.TemporaryDirectory(prefix="t3-ap5-verifier-lineage-",dir=BASE) as tmp:
            raw=canonical_json(predecessor);path=Path(tmp)/"predecessor.json";path.write_bytes(raw)
            handoff["preserved_predecessor"]={"path":str(path),"sha256":hashlib.sha256(raw).hexdigest()}
            result=ap5_v2.inspect_verifier_handoff(handoff)
            self.assertEqual(result["status"],"INCONCLUSIVE")
            self.assertFalse(result["live_corpus_required_to_build"])
            self.assertTrue(result["first_actual_board_proof_required_for_funded_acceptance"])
            handoff["requires_live_corpus_for_compilation"]=True
            with self.assertRaises(Invalid):ap5_v2.inspect_verifier_handoff(handoff)

    def test_default_feature_or_unexecuted_merged_checks_are_not_Ark_admission(self):
        validation={"schema":"spc.lab.ap5.join-validation/v1","source_manifest_sha256":"11"*32,"checks":[]}
        bundle={"joined_source_manifest":{"sha256":"11"*32}}
        with self.assertRaisesRegex(Invalid,"check census"):ap5_v2.joined_profile_checks(validation,bundle)

    def test_default_agent_test_argv_cannot_pass_even_with_wallet_named_log_lines(self):
        with tempfile.TemporaryDirectory(prefix="t3-ap5-ark-feature-",dir=BASE) as tmp:
            path=Path(tmp)/"tests.log";raw=b"test wallet::fixture ... ok\ntest result: ok.\n";path.write_bytes(raw)
            purposes=["agent-ark-tests","agent-ark-build","pool-ark-tests","pool-ark-build","ui-build","ui-browser-tests"]
            checks=[{"purpose":p,"source_manifest_sha256":"11"*32,"exit":0,
                     "argv":["cargo","test","-p","spc-agent"],"log":{"path":str(path),"sha256":hashlib.sha256(raw).hexdigest()}} for p in purposes]
            validation={"schema":"spc.lab.ap5.join-validation/v1","source_manifest_sha256":"11"*32,"checks":checks}
            bundle={"joined_source_manifest":{"sha256":"11"*32}}
            with self.assertRaisesRegex(Invalid,"default-feature"):
                ap5_v2.joined_profile_checks(validation,bundle)


if __name__=="__main__":unittest.main()
