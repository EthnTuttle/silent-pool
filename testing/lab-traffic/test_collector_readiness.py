"""Readiness regressions using existing contracts only, not warmup/native DTOs."""
import copy
import hashlib
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

from case_contract import required, verdict
from oracle import Chain, Inconclusive, Invalid, sha256d
from source_capture import SourceCapture, full_window_evidence
from source_feed_provider import ReplayProvider
from source_jobs import MissingComponent
from test_oracle import block_bytes, coinbase, policy
import test_economic_checkpoint
import test_pool_source

BASE=None
G="79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
G2="c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5"
G3="f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9"


def source_window(full=True, issued=True):
    events=test_pool_source.PoolSourceTests().fixture()
    q="2" if full else "3"
    events[0][1].update(window_target=q,window_underfilled=True,slices=[],high_water={"0":"0"})
    events[4][1].update(window_target=q,window_underfilled=not full)
    if issued:events.insert(5,("job_issued",{**events[4][1],"channel_id":1}))
    return test_pool_source.parse(test_pool_source.encode(events))


class FullWindowReadinessTests(unittest.TestCase):
    def profile(self,full=True):return {"reference_target":"7f"+"ff"*31,"window_n":1 if full else 2}

    def test_full_N_is_an_unconditional_funded_campaign_requirement(self):
        checks={name:{"status":"PASSED"} for name in required("funded",True)}
        self.assertIn("full_window_coverage",checks)
        del checks["full_window_coverage"]
        self.assertEqual(verdict(checks,required("funded",True)),"INCONCLUSIVE")
        checks["full_window_coverage"]={"status":"INCONCLUSIVE"}
        self.assertEqual(verdict(checks,required("funded",True)),"INCONCLUSIVE")

    def test_full_window_requires_actual_issued_and_verified_source_work(self):
        # The template verifier has separate raw-body tests. Isolate the
        # eventual-coverage mechanism while retaining real source/share joins.
        with patch("source_capture.verify_job"):
            result=full_window_evidence(source_window(),{},None,self.profile())
            self.assertEqual(result["verified_full_windows"][0]["Q"],2)
            self.assertEqual(result["verified_full_windows"][0]["selected_work"],2)
            self.assertTrue(result["other_issued_windows_unavailable"])
            for source,profile in ((source_window(False),self.profile(False)),(source_window(issued=False),self.profile())):
                with self.assertRaises(MissingComponent):full_window_evidence(source,{},None,profile)

    def test_one_full_front_cannot_cover_another_incomplete_front(self):
        collector=object.__new__(SourceCapture)
        collector.expected={"a":None,"b":None};collector.issues={};collector.policy=None
        collector.sources={"a":source_window(),"b":source_window(False)}
        collector.scenario={"fronts":[{"id":"a","pool_id":"pool-a"},{"id":"b","pool_id":"pool-b"}],
            "pools":[{"id":"pool-a","work_profile":self.profile()},{"id":"pool-b","work_profile":self.profile(False)}]}
        with patch("source_capture.verify_job"):
            with self.assertRaisesRegex(MissingComponent,"full_N_window:b"):collector.full_window_coverage({})
            collector.sources["b"]=source_window();collector.scenario["pools"][1]["work_profile"]=self.profile()
            self.assertEqual(set(collector.full_window_coverage({})["fronts"]),{"a","b"})
            collector.issues["b"]={"status":"INCONCLUSIVE","component":"truncated_tail"}
            with self.assertRaisesRegex(MissingComponent,"source_census"):collector.full_window_coverage({})

    def test_invalid_template_cannot_be_skipped_as_a_warmup_window(self):
        with patch("source_capture.verify_job",side_effect=Invalid("invalid actual template")):
            with self.assertRaises(Invalid):full_window_evidence(source_window(),{},None,self.profile())


class SnapshotReadinessTests(unittest.TestCase):
    def test_source_signature_cache_never_replaces_digest_or_key_binding(self):
        import source_feed_codec as codec
        from test_source_feed import pub
        digest=bytes([5])*32
        signature=codec._sign_digest(7,digest)
        codec._verify_signature_exact.cache_clear()
        with patch("source_feed_codec._multiply",wraps=codec._multiply) as multiply:
            codec.verify_signature(digest,signature,pub(7))
            first=multiply.call_count
            codec.verify_signature(digest,signature,pub(7))
            self.assertEqual(multiply.call_count,first)
            with self.assertRaises(Invalid):codec.verify_signature(bytes([6])*32,signature,pub(7))
            with self.assertRaises(Invalid):codec.verify_signature(digest,signature,pub(8))
            self.assertGreater(multiply.call_count,first)

    def test_stored_effect_must_follow_its_frozen_economic_revision(self):
        observation,economic,chain,migrations=test_economic_checkpoint.captured()
        row=next(r for r in observation["tables"]["economic_block"] if r["height"]==2)
        row["frozen_job"]["revision"]=row["revision"]
        event=next(r for r in observation["tables"]["source_accounting_history"]
                   if r["table_name"]=="economic_block" and r["operation"]=="INSERT" and r["after_row"]["height"]==2)
        event["after_row"]["frozen_job"]["revision"]=row["revision"]
        from ruleb_sql import replay_checkpoint
        from source_feed_codec import CONTRACT
        with self.assertRaisesRegex(Invalid,"does not follow"):
            replay_checkpoint(observation,economic,chain,source_contract=CONTRACT,migration_checksums=migrations)

    def test_incomplete_capture_is_retained_but_does_not_poison_later_coherent_snapshot(self):
        observation,economic,chain,migrations=test_economic_checkpoint.captured()
        bad=copy.deepcopy(observation)
        bad["tables"]["economic_state"][0]["complete"]=False
        last=next(e for e in reversed(bad["tables"]["source_accounting_history"]) if e["table_name"]=="economic_state")
        last["after_row"]["complete"]=False
        key=("session",0,"00000009",4)
        source=SimpleNamespace(jobs={key:{"payload":{"prev_hash":bytes(economic["parent_hash"]).hex(),"economic":economic}}})
        provider=object.__new__(ReplayProvider)
        recorder=Mock()
        provider.capture=SimpleNamespace(scenario={"run":{"id":"owned"},"fronts":[{"id":"front","pool_id":"pool"}]},
                                         sources={"front":source},capture=recorder)
        provider.views={chain.tip["hash"]:chain};provider.migrations=migrations
        provider.snapshots={};provider.snapshot_refs={};provider.snapshot_attempts={}
        with tempfile.TemporaryDirectory(prefix="t3-coherent-snapshot-",dir=BASE) as tmp:
            from economic_checkpoint import query
            from source_feed_codec import CONTRACT
            refs=[]
            for index,value in enumerate((bad,observation,observation,bad)):
                capture={"schema":"spc-lab-bound-sql-capture/1","binding":{"run_id":"owned","pool_id":"pool"},
                         "query_sha256":hashlib.sha256(query(source_contract=CONTRACT).encode()).hexdigest(),
                         "execution":{"exit_code":0,"timed_out":False},"observation":value}
                raw=json.dumps(capture).encode();path=Path(tmp)/f"capture-{index}.json";path.write_bytes(raw)
                refs.append({"path":str(path),"sha256":hashlib.sha256(raw).hexdigest()})
            provider.capture_sql=Mock(side_effect=refs)
            with self.assertRaises(Inconclusive):provider.capture_snapshot("front",key)
            self.assertNotIn(("front",key),provider.snapshots)
            provider.capture_snapshot("front",key)
            provider.capture_snapshot("front",key)
            self.assertEqual(provider.capture_sql.call_count,2)
            self.assertEqual([r["status"] for r in provider.snapshot_attempts[("front",key)]],
                             ["INCONCLUSIVE","COHERENT_ROW_CHECKPOINT_ONLY"])
            self.assertEqual([call.args[0] for call in recorder.add.call_args_list],
                             ["source_sql_snapshot_attempt","source_sql_snapshot_unavailable",
                              "source_sql_snapshot_attempt","source_sql_snapshot_admitted"])
            self.assertTrue(Path(refs[0]["path"]).is_file())
            self.assertEqual(hashlib.sha256(Path(refs[0]["path"]).read_bytes()).hexdigest(),refs[0]["sha256"])
            retained=provider.snapshots[("front",key)]
            provider.capture_snapshot("front",key,coherence_only=True)
            self.assertIs(provider.snapshots[("front",key)],retained)
            with self.assertRaises(Inconclusive):provider.capture_snapshot("front",key,coherence_only=True)
            self.assertIs(provider.snapshots[("front",key)],retained)
            self.assertEqual(provider.capture_sql.call_count,4)

    def graph(self,n=7):
        raws=[]
        for i in range(n):raws.append(block_bytes(sha256d(raws[-1][:80]) if raws else bytes(32),[coinbase(tag=b"\2"+i.to_bytes(2,"little"))],1600000000+i))
        chain=Chain(policy(genesis=sha256d(raws[0][:80])[::-1].hex(),regtest_pow_bits_overlay=None),raws)
        keys=[("session",0,f"{i:08x}",i) for i in range(n)]
        jobs={k:{"payload":{"height":100,"economic":{"revision":i}}} for i,k in enumerate(keys)}
        source=SimpleNamespace(jobs=jobs,verified_window=Mock(return_value={"selected":[]}))
        provider=object.__new__(ReplayProvider);provider.capture=SimpleNamespace(sources={"front":source},policy=chain.policy)
        provider.views={chain.tip["hash"]:chain};provider.migrations={};provider._profile=lambda _: {"reference_target":"7f"+"ff"*31,"window_n":1}
        provider._settlement=Mock(return_value=[])
        by_hash={b["hash"]:keys[i] for i,b in enumerate(chain.blocks)}
        provider.snapshots={}
        for i,key in enumerate(keys):
            provider.snapshots[("front",key)]={"tables":{"economic_block":[
                {"block_hash":"\\x"+bytes.fromhex(chain.blocks[j]["hash"])[::-1].hex(),"validated_ledger":[],"consumption":[]}
                for j in range(i)]}}
        return provider,source,chain,keys,by_hash

    def test_verified_context_DAG_is_memoized_per_exchange_and_orders_by_revision(self):
        p,source,chain,keys,by_hash=self.graph()
        # Raw job/ledger verification is tested separately. All heights here
        # coincide to model already-verified historical branches after reorg.
        with patch("source_feed_provider.parent_view",return_value=chain), \
             patch("source_feed_provider.replay_checkpoint",return_value={"available_from_replay":{}}), \
             patch("source_feed_provider.prove_eligible",side_effect=lambda source,key,*args:source.verified_window(key)), \
             patch("source_feed_provider.verify_job",return_value={}), \
             patch("source_feed_provider.verify_publication",side_effect=lambda s,h,v,policy:{"job_key":by_hash[h]}):
            p._context("front",keys[-1],set())
        self.assertEqual(source.verified_window.call_count,len(keys))

    def test_incomplete_context_is_not_reused_on_a_later_exchange(self):
        p,source,chain,keys,by_hash=self.graph(2)
        source.verified_window.side_effect=[Inconclusive("publication not retained yet"),{"selected":[]},{"selected":[]}]
        with patch("source_feed_provider.parent_view",return_value=chain), \
             patch("source_feed_provider.replay_checkpoint",return_value={"available_from_replay":{}}), \
             patch("source_feed_provider.prove_eligible",side_effect=lambda source,key,*args:source.verified_window(key)), \
             patch("source_feed_provider.verify_job",return_value={}), \
             patch("source_feed_provider.verify_publication",side_effect=lambda s,h,v,policy:{"job_key":by_hash[h]}):
            with self.assertRaises(Inconclusive):p._context("front",keys[-1],set())
            p._context("front",keys[-1],set())
        self.assertEqual(source.verified_window.call_count,3)


class SettlementReadinessTests(unittest.TestCase):
    def fixture(self):
        p={"fee_bps":0,"entries":[{"sp_id":"33"*32,"owed_sats":"1600000","sats":"1500000","rail":0,
                                    "p_x":[G,G2],"delivery_state":"unregistered","destination_digest":None}]}
        block={"subsidy":1600000,"fees":0,"transactions":[{"outputs":[{"value":100000,"script":b"pool"},
                 {"value":1000000,"script":bytes.fromhex("5120"+G)},{"value":500000,"script":bytes.fromhex("5120"+G2)}]}]}
        window={"selected":[[{"identity":"33"*32,"work":1,"fees":0}]]}
        return p,block,window

    def settle(self,p,block,window,prior=None):return object.__new__(ReplayProvider)._settlement(p,block,prior or {},window)

    def test_known_undeliverable_coinbase_remainder_is_error_pending_not_forfeit(self):
        p,b,w=self.fixture();row=self.settle(p,b,w)[0]
        self.assertEqual((row["pending_delta"],row["ark_due_sats"],row["forfeited_sats"]),(100000,0,0))
        p["entries"][0].update(delivery_state="registered_derived",destination_digest="44"*32)
        row=self.settle(p,b,w)[0]
        self.assertEqual((row["pending_delta"],row["ark_due_sats"]),(0,100000))

    def test_unknown_or_missing_delivery_cannot_silently_become_zero_Ark_due(self):
        for state in ("future-delivery","registered_derived"):
            p,b,w=self.fixture();p["entries"][0]["delivery_state"]=state
            with self.assertRaises(MissingComponent):self.settle(p,b,w)
        p,b,w=self.fixture();p["entries"][0]["destination_digest"]="44"*32
        with self.assertRaises(Invalid):self.settle(p,b,w)

    def test_omitted_earned_or_pending_identity_is_detected_before_pool_conservation(self):
        p,b,w=self.fixture()
        with self.assertRaisesRegex(Invalid,"omits"):self.settle(p,b,w,{"55"*32:500000})
        w["selected"][0].append({"identity":"55"*32,"work":1,"fees":0})
        with self.assertRaisesRegex(Invalid,"omits"):self.settle(p,b,w)

    def test_available_pending_for_an_identity_outside_current_window_is_routed(self):
        p,b,w=self.fixture();p["fee_bps"]=2500;b["subsidy"]=2000000
        p["entries"][0]["owed_sats"]="1500000"
        p["entries"].append({"sp_id":"55"*32,"owed_sats":"500000","sats":"500000","rail":0,
                             "p_x":[G3],"delivery_state":"unregistered","destination_digest":None})
        b["transactions"][0]["outputs"][0]["value"]=0
        b["transactions"][0]["outputs"].append({"value":500000,"script":bytes.fromhex("5120"+G3)})
        rows=self.settle(p,b,w,{"55"*32:500000})
        self.assertEqual(rows[1]["pending_delta"],-500000)

    def test_declared_paid_scalar_cannot_disagree_with_actual_ladder(self):
        p,b,w=self.fixture();p["entries"][0]["sats"]="1499999"
        with self.assertRaisesRegex(Invalid,"shaped amount"):self.settle(p,b,w)

    def test_error_and_forfeit_rails_name_the_actual_missing_provenance(self):
        for rail,reason in ((1,"historical_Ark"),(2,"actual_error_rail"),(3,"historical_no_registration")):
            p,b,w=self.fixture();p["entries"][0]["rail"]=rail
            with self.assertRaisesRegex(MissingComponent,reason):self.settle(p,b,w)


if __name__=="__main__":unittest.main()
