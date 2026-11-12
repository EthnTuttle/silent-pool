"""Owned capture sequencing only; fixtures grant no mining or wallet authority."""
import copy
import hashlib
import json
from pathlib import Path
import tempfile
import threading
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

from case_contract import canonical_json, required
from checkpoint_retention import CheckpointRetention, retain_provider_checkpoint
from economic_checkpoint import query
from oracle import Inconclusive, Invalid
from source_capture import SourceCapture
from source_feed_codec import CONTRACT
from source_feed_provider import ReplayProvider
from settlement_drain import drain, convergence_gaps
from test_economic_checkpoint import captured

BASE=None


class CheckpointRetentionTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory(prefix="t3-checkpoint-",dir=BASE);self.addCleanup(self.temp.cleanup)
        self.root=Path(self.temp.name);self.observation,self.economic,self.chain,self.migrations=captured()
        self.serial=0;self.capture_lock=threading.Lock()
        def capture_sql(front,key):
            with self.capture_lock:
                self.serial+=1;path=self.root/(f"sql-{self.serial}.json")
                envelope={"schema":"spc-lab-bound-sql-capture/1","binding":{"run_id":"owned","pool_id":"pool"},
                    "execution":{"exit_code":0,"timed_out":False},"query_sha256":hashlib.sha256(query(source_contract=CONTRACT).encode()).hexdigest(),
                    "observation":copy.deepcopy(self.observation)}
                raw=canonical_json(envelope);path.write_bytes(raw);path.chmod(0o600)
                return {"path":str(path),"sha256":hashlib.sha256(raw).hexdigest()}
        self.capture_sql=capture_sql
        self.retention=CheckpointRetention(self.root/"retained","owned",{"front":"pool"},capture_sql,CONTRACT)
        self.addCleanup(lambda:self.retention.close(capture_barrier=False))

    def provider(self):
        key=("session",0,"00000009",4)
        payload={"prev_hash":bytes(self.economic["parent_hash"]).hex(),"economic":self.economic}
        source=SimpleNamespace(jobs={key:{"payload":payload,"source":{"seq":"1"}}},issued={key:[{"source":{"seq":"2"}}]},events=[],source_contract=CONTRACT)
        capture=object.__new__(SourceCapture);capture.source_feed={"contract":CONTRACT};capture.sources={"front":source}
        capture.scenario={"run":{"id":"owned"},"fronts":[{"id":"front","pool_id":"pool"}]};capture.capture=Mock()
        provider=ReplayProvider(capture,{self.chain.tip["hash"]:self.chain},{},self.capture_sql,lambda:self.chain,self.migrations,self.retention)
        return provider,key

    def test_FIFO_barrier_waits_for_every_preceding_notification(self):
        self.retention.request("front",{"kind":"first"});self.retention.request("front",{"kind":"second"})
        self.retention.barrier("front")
        self.assertEqual([r["trigger"]["kind"] for r in self.retention.records],["first","second","barrier"])
        self.assertEqual(self.retention.queue.unfinished_tasks,0)
        rows=[json.loads(l) for l in (self.retention.root/"attempts.jsonl").read_bytes().splitlines()]
        self.assertEqual([r["seq"] for r in rows],list(range(1,len(rows)+1)))

    def test_untrusted_economic_hint_only_requests_original_SQL(self):
        raw=b'INFO spc_evidence: event="economic_change_observed" evidence={"economic_revision":"999999","source":{"session":"hint-only","seq":"1"}}\n'
        self.retention.notify("front",raw);self.retention.barrier("front")
        self.assertEqual(self.retention.records[0]["revision"],self.economic["revision"])
        self.assertNotEqual(self.retention.records[0]["revision"],999999)

    def test_provider_hook_reuses_original_admitted_cache_without_current_state_substitution(self):
        provider,key=self.provider();provider.capture_snapshot("front",key);original=provider.snapshots[("front",key)]
        first=retain_provider_checkpoint(self.retention,provider,"front",key)
        self.observation["tables"]["economic_state"][0]["revision"]+=1
        second=retain_provider_checkpoint(self.retention,provider,"front",key)
        self.assertIs(provider.snapshots[("front",key)],original)
        self.assertEqual(first["sha256"],second["sha256"]);self.assertEqual(self.serial,1)
        self.assertEqual(first["original_artifact"],provider.snapshot_refs[("front",key)])

    def test_hook_metadata_mismatch_and_missing_original_artifact_block_proof_boundary(self):
        provider,key=self.provider();real=self.retention.retain
        def corrupt(*args,**kwargs):return {**real(*args,**kwargs),"revision":999999}
        with patch.object(self.retention,"retain",side_effect=corrupt),self.assertRaisesRegex(Invalid,"metadata mismatch"):
            retain_provider_checkpoint(self.retention,provider,"front",key)
        provider.snapshot_refs.clear()
        with self.assertRaisesRegex(Inconclusive,"original_admitted"):retain_provider_checkpoint(self.retention,provider,"front",key)

    def test_mutated_cache_or_original_bytes_are_not_retained_as_approved(self):
        provider,key=self.provider();record=retain_provider_checkpoint(self.retention,provider,"front",key)
        provider.snapshots[("front",key)]["tables"]["economic_state"][0]["revision"]+=1
        with self.assertRaisesRegex(Invalid,"cached checkpoint"):retain_provider_checkpoint(self.retention,provider,"front",key)
        Path(record["original_artifact"]["path"]).write_bytes(b"changed")
        with self.assertRaisesRegex(Invalid,"original retained"):list(self.retention.observations("front"))

    def test_exact_coverage_includes_unadvertised_frozen_and_advertised_without_freeze(self):
        provider,key=self.provider();retain_provider_checkpoint(self.retention,provider,"front",key)
        sources=provider.capture.sources
        result=self.retention.coverage(sources,provider.views,self.migrations)
        self.assertEqual(result["status"],"COMPLETE_CAPTURE")
        sources["front"].issued[("session",0,"00000010",5)]=[{"source":{"seq":"3"}}]
        result=self.retention.coverage(sources,provider.views,self.migrations)
        self.assertEqual(result["status"],"CERTIFICATION_BLOCKED")
        self.assertTrue(any("advertised" in r["reason"] for r in result["unavailable"]))
        self.assertFalse(result["later_state_reconstruction"])

    def test_later_revision_and_source_reader_failure_cannot_close_coverage(self):
        provider,key=self.provider();self.observation["tables"]["economic_state"][0]["revision"]+=1
        self.retention.barrier("front")
        result=self.retention.coverage(provider.capture.sources,provider.views,self.migrations,{"front":{"component":"incomplete_tail"}})
        self.assertEqual(result["status"],"CERTIFICATION_BLOCKED")
        self.assertTrue(any(r.get("revision")==self.economic["revision"] for r in result["unavailable"]))

    def test_queue_overflow_is_retained_and_blocks_provider_hook(self):
        entered=threading.Event();release=threading.Event();original=self.retention.capture_sql
        self.retention.close(capture_barrier=False)
        def blocked(*args):entered.set();release.wait(3);return original(*args)
        self.retention=CheckpointRetention(self.root/"bounded","owned",{"front":"pool"},blocked,CONTRACT,capacity=1)
        try:
            self.retention.request("front",{"kind":"one"});self.assertTrue(entered.wait(2))
            self.retention.request("front",{"kind":"two"})
            with self.assertRaisesRegex(Inconclusive,"overflow"):self.retention.request("front",{"kind":"three"})
            with self.assertRaises(Inconclusive):self.retention.require_healthy()
            self.assertEqual(self.retention.failures[-1]["kind"],"capture_queue_overflow")
        finally:release.set()

    def test_quiesced_provider_cannot_start_a_new_capture_or_proof(self):
        provider,key=self.provider();provider.accepting=False
        with self.assertRaisesRegex(Inconclusive,"quiesced"):provider.prepare("unused",{})
        self.assertEqual(self.serial,0)


class DrainSequenceTests(unittest.TestCase):
    def runtime(self):
        from source_feed_runtime import CollectorRuntime
        return CollectorRuntime(SimpleNamespace(run_id="owned",capture=Mock()))

    def test_missing_pre_stop_checkpoint_leaves_pool_writers_and_final_seal_untouched(self):
        stop_pool=Mock();seal=Mock();clock=[0]
        def missing(current):raise Inconclusive("exact revision missing")
        with self.assertRaisesRegex(Inconclusive,"exact revision missing"):
            drain(lambda:None,lambda:{"gaps":[],"stable_key":"same"},stop_pool,Mock(),seal,
                  before_pool_stop=missing,clock=lambda:clock[0],sleep=lambda n:clock.__setitem__(0,clock[0]+n))
        stop_pool.assert_not_called();seal.assert_not_called()

    def test_pre_stop_coverage_elapsed_deadline_cannot_stop_writers(self):
        stop_pool=Mock();seal=Mock();clock=[0]
        def slow(current):clock[0]+=61
        with self.assertRaisesRegex(Inconclusive,"before pool stop"):
            drain(lambda:None,lambda:{"gaps":[],"stable_key":"same"},stop_pool,Mock(),seal,
                  before_pool_stop=slow,clock=lambda:clock[0],sleep=lambda n:clock.__setitem__(0,clock[0]+n))
        stop_pool.assert_not_called();seal.assert_not_called()

    def test_after_EOF_coverage_elapsed_deadline_cannot_seal(self):
        seal=Mock();clock=[0];stop_pool=Mock()
        def slow_final():clock[0]+=61;return {"gaps":[],"stable_key":"same"}
        with self.assertRaisesRegex(Inconclusive,"after pool EOF"):
            drain(lambda:None,lambda:{"gaps":[],"stable_key":"same"},stop_pool,slow_final,seal,
                  clock=lambda:clock[0],sleep=lambda n:clock.__setitem__(0,clock[0]+n))
        stop_pool.assert_called_once();seal.assert_not_called()

    def test_coverage_change_is_detected_while_writers_still_available(self):
        runtime=self.runtime();runtime._checkpoint_coverage=Mock()
        runtime._drain_sample=Mock(return_value={"gaps":[],"stable_key":"new-job"})
        with self.assertRaisesRegex(Inconclusive,"changed_during_pre_stop"):
            runtime._before_pool_stop({"gaps":[],"stable_key":"previous"})

    def test_concurrent_close_and_drain_refuse_without_stopping_readers(self):
        runtime=self.runtime();runtime._close=Mock();runtime._final_drain=Mock()
        entered=threading.Event();release=threading.Event()
        def hold():
            with runtime.finalize_lock:entered.set();release.wait(3)
        owner=threading.Thread(target=hold);owner.start()
        try:
            self.assertTrue(entered.wait(2))
            for action in (runtime.close,runtime.final_drain):
                with self.assertRaisesRegex(Inconclusive,"already_active"):action()
            runtime._close.assert_not_called();runtime._final_drain.assert_not_called()
            self.assertFalse(runtime.drain_attempted)
        finally:release.set();owner.join(3)

    def test_failed_drain_retains_failure_and_cannot_be_retried_but_allows_cleanup(self):
        runtime=self.runtime();runtime._final_drain=Mock(side_effect=Inconclusive("missing checkpoint"))
        with self.assertRaisesRegex(Inconclusive,"missing checkpoint"):runtime.final_drain()
        with self.assertRaisesRegex(Inconclusive,"cannot_restart"):runtime.final_drain()
        runtime._final_drain.assert_called_once()
        events=runtime.controller.capture.add.call_args_list
        self.assertEqual([row.args[0] for row in events],["economic_final_drain_started","economic_final_drain_incomplete"])
        self.assertFalse(events[-1].args[1]["converged"])
        runtime.close();self.assertTrue(runtime.closed)

    def test_successful_drain_reuse_is_observation_only_and_caller_cannot_mutate_seal(self):
        runtime=self.runtime();runtime._final_drain=Mock(return_value={"status":"CONVERGED_COMPLETE_CAPTURE","tip":{"height":23}})
        first=runtime.final_drain();first["tip"]["height"]=999
        runtime.close();second=runtime.final_drain()
        self.assertEqual(second["tip"]["height"],23);runtime._final_drain.assert_called_once()

    def test_closed_collector_cannot_start_a_drain(self):
        runtime=self.runtime();runtime._final_drain=Mock();runtime.close()
        with self.assertRaisesRegex(Inconclusive,"cannot_restart"):runtime.final_drain()
        runtime._final_drain.assert_not_called()

    def test_runtime_checks_both_coverage_boundaries_before_sealing(self):
        import time
        for lost_after_EOF in (False,True):
            with self.subTest(lost_after_EOF=lost_after_EOF),tempfile.TemporaryDirectory(prefix="t3-drain-boundary-",dir=BASE) as tmp:
                runtime=self.runtime();c=runtime.controller;c.priv=Path(tmp);c.deadline=time.monotonic()+60
                c.source_capture=SimpleNamespace(expected={"front":None},eof={})
                runtime.retention=Mock();order=[]
                runtime._stop_mining=lambda:order.append("stop-mining")
                def sample():
                    order.append("raw-source-SQL")
                    return {"gaps":[],"stable_key":"same","tip":{"height":23},"pools":{}}
                runtime._drain_sample=sample
                def coverage(phase):
                    order.append(phase)
                    if phase=="after_pool_EOF" and lost_after_EOF:raise Inconclusive("retained original disappeared")
                    return {"status":"COMPLETE_CAPTURE"}
                runtime._checkpoint_coverage=coverage
                def stop_pools():
                    order.append("pool-EOF");c.source_capture.eof={"front":{"digest":"11"*32}}
                runtime._stop_pools=stop_pools
                if lost_after_EOF:
                    with self.assertRaisesRegex(Inconclusive,"original disappeared"):runtime.final_drain()
                    self.assertFalse((c.priv/"economic-final-boundary.json").exists())
                else:
                    self.assertEqual(runtime.final_drain()["status"],"CONVERGED_COMPLETE_CAPTURE")
                    sealed=json.loads((c.priv/"economic-final-boundary.json").read_bytes())
                    self.assertEqual(sealed["checkpoint_coverage"]["status"],"COMPLETE_CAPTURE")
                self.assertEqual(order[:8],["stop-mining","raw-source-SQL","raw-source-SQL","raw-source-SQL",
                    "before_pool_stop","raw-source-SQL","pool-EOF","after_pool_EOF"])
                self.assertEqual(order[8:],[] if lost_after_EOF else ["raw-source-SQL"])

    def test_quiesce_does_not_wait_for_the_active_provider_replay_lock(self):
        from source_feed_runtime import CollectorRuntime
        runtime=CollectorRuntime(SimpleNamespace())
        runtime.provider=SimpleNamespace(accepting=True,lock=object())
        worker=Mock();worker.is_alive.return_value=False;runtime.threads=[worker]
        runtime.quiesce_feed()
        self.assertFalse(runtime.provider.accepting);self.assertTrue(runtime.stop_event.is_set())
        self.assertTrue(runtime.feed_quiet);worker.join.assert_called_once()

    def test_failed_feed_quiescence_still_closes_source_and_checkpoint_workers(self):
        from source_feed_runtime import CollectorRuntime
        runtime=CollectorRuntime(SimpleNamespace());runtime.quiesce_feed=Mock(side_effect=Inconclusive("handler active"))
        runtime.source_reader=Mock();runtime.source_reader.is_alive.return_value=False;runtime.retention=Mock()
        with self.assertRaisesRegex(Inconclusive,"close incomplete"):runtime.close()
        self.assertTrue(runtime.source_stop.is_set())
        runtime.source_reader.join.assert_called_once()
        runtime.retention.close.assert_called_once_with(35,capture_barrier=False)

    def test_three_stable_samples_follow_mining_stop_and_precede_pool_EOF_and_final_recheck(self):
        order=[];clock=[0]
        def sample():order.append("sample");return {"gaps":[],"stable_key":"same"}
        result=drain(lambda:order.append("stop-mining"),sample,lambda:order.append("pool-EOF"),
            lambda:(order.append("final-raw-source-SQL") or {"gaps":[],"stable_key":"same"}),lambda final:order.append("seal"),
            clock=lambda:clock[0],sleep=lambda n:clock.__setitem__(0,clock[0]+n))
        self.assertEqual(order,["stop-mining","sample","sample","sample","pool-EOF","final-raw-source-SQL","seal"])
        self.assertEqual(result["gaps"],[])

    def test_gap_or_changed_key_resets_stability_and_final_race_never_seals(self):
        values=iter([{"gaps":[],"stable_key":"a"},{"gaps":["acceptance missing"],"stable_key":"a"},
                     {"gaps":[],"stable_key":"b"},{"gaps":[],"stable_key":"b"},{"gaps":[],"stable_key":"b"}])
        seal=Mock();clock=[0]
        with self.assertRaisesRegex(Inconclusive,"post-EOF"):
            drain(lambda:None,lambda:next(values),lambda:None,lambda:{"gaps":[],"stable_key":"changed"},seal,
                  clock=lambda:clock[0],sleep=lambda n:clock.__setitem__(0,clock[0]+n))
        seal.assert_not_called()

    def test_timeout_neither_stops_pool_early_nor_forces_booking(self):
        clock=[0];stop_pool=Mock();seal=Mock()
        with self.assertRaisesRegex(Inconclusive,"did not converge"):
            drain(lambda:None,lambda:{"gaps":["H23: economic acceptance/consumption missing"],"stable_key":"x"},stop_pool,Mock(),seal,
                  timeout=1,clock=lambda:clock[0],sleep=lambda n:clock.__setitem__(0,clock[0]+n))
        stop_pool.assert_not_called();seal.assert_not_called()
        self.assertIn("economic_final_drain",required("funded"))

    def test_shallow_canonical_liability_is_required_unbooked_not_dropped(self):
        zero="00"*32;bh="11"*32;raw=b"raw-coinbase-fixture"
        block={"height":1,"hash":bh,"prev":zero,"transactions":[{"raw":raw}]}
        chain=SimpleNamespace(tip=block,blocks=[{"height":0,"hash":zero,"prev":zero},block])
        sql=lambda v:"\\x"+bytes.fromhex(v)[::-1].hex()
        rows=[{"height":0,"block_hash":sql(zero),"parent_hash":sql(zero),"pool_block":False,"coinbase_digest":"\\x"+zero},
              {"height":1,"block_hash":sql(bh),"parent_hash":sql(zero),"pool_block":True,"coinbase_digest":"\\x"+hashlib.sha256(raw).hexdigest()}]
        economic={"block_hash":sql(bh),"lifecycle":"unbooked","frozen_job":{},"validated_ledger":[],"consumption":[]}
        observation={"tables":{"economic_state":[{"complete":True,"anchor_height":1,"anchor_hash":sql(bh)}],"economic_chain":rows,
            "economic_block":[economic],"block_found":[],"ledger_history":[]}}
        source=SimpleNamespace(block_publications={bh:[]},events=[],jobs={})
        with patch("settlement_drain.parse_block"):
            self.assertEqual(convergence_gaps(chain,[source],observation,6),[])
            economic["lifecycle"]="booked"
            self.assertTrue(any("shallow" in gap for gap in convergence_gaps(chain,[source],observation,6)))
            observation["tables"]["economic_block"]=[]
            self.assertTrue(any("acceptance/consumption" in gap for gap in convergence_gaps(chain,[source],observation,6)))

    def test_source_partial_tail_is_retained_and_EOF_rejects_later_bytes(self):
        from test_pool_source import encode
        raw=encode([("bootstrap_applied",{"front_id":0})])
        with tempfile.TemporaryDirectory(prefix="t3-source-EOF-",dir=BASE) as tmp:
            root=Path(tmp);run=root/"run";private=root/"qa";private.mkdir(mode=0o700)
            source=SourceCapture(run,private,Mock(),{"fronts":[{"id":"front"}]},{"sha256":"11"*32},None)
            (run/"logs").mkdir(parents=True);path=source.expected["front"];path.write_bytes(raw[:-3])
            source.bindings["front"]=(111,1,"11"*32);source.process_records["front"]={"pid":111}
            notify=Mock();source.attach_checkpoints(notify)
            with patch.object(source,"bind"):
                source.poll({}, {},materialize=False)
                self.assertIn("front",source.issues);notify.notify.assert_not_called()
                with path.open("ab") as stream:stream.write(raw[-3:])
                source.poll({}, {},materialize=False)
                self.assertNotIn("front",source.issues);notify.notify.assert_called_once_with("front",raw)
            with patch("cleanup_proof.identity_observation",return_value={"status":"gone"}):source.mark_eof("front")
            source.poll({}, {},materialize=False)
            with path.open("ab") as stream:stream.write(b"late bytes\n")
            with self.assertRaisesRegex(Invalid,"after writer EOF"):source.poll({}, {},materialize=False)


if __name__=="__main__":unittest.main()
