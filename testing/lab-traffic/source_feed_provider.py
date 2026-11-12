"""Build feed bundles only from owned source capture, SQL replay and raw E.

No ready-bundle file, `complete` switch, or producer-computed available balance
can enter this provider. Unsupported settlement rails remain unavailable.
"""
from dataclasses import dataclass
import hashlib
import threading

from case_contract import canonical_json
from oracle import Chain, ChainPrefix, Inconclusive, decode_transaction, integer, ladder, raw_payouts, require, strict_json, unhex
from pool_source import decimal_uint
from ruleb_sql import replay_checkpoint, sql_bytes
from source_feed_codec import CONTRACT, MAX_BUNDLE, array, received_digest
from source_jobs import MissingComponent, parent_view, verify_job, verify_publication
from audited_window import prove_eligible, rewards

_REPLAY_TOKEN = object()


@dataclass(frozen=True)
class VerifiedEvidence:
    bundle: bytes
    pool: dict
    job_id: int
    generation: int
    history_watermark: int
    history_digest: bytes
    _authority: object

    def __post_init__(self):
        require(self._authority is _REPLAY_TOKEN,"complete evidence must come from independent replay")


def history_digest(watermark, pending):
    return hashlib.sha256(b"SPc/VerifiedHistoryCheckpointV1"+watermark.to_bytes(8,"little")+canonical_json(pending)).digest()


class ReplayProvider:
    def __init__(self, capture, views, agents, capture_sql, refresh, migration_checksums, checkpoint_retention=None):
        from source_capture import SourceCapture
        require(isinstance(capture,SourceCapture) and capture.source_feed is not None
                and capture.source_feed["contract"]==CONTRACT,"actual registered signed source capture required")
        self.capture,self.views,self.agents=capture,views,agents
        self.capture_sql,self.refresh,self.migrations=capture_sql,refresh,migration_checksums
        self.snapshots={};self.snapshot_refs={};self.snapshot_attempts={};self.lock=threading.RLock()
        self.checkpoint_retention=checkpoint_retention;self.accepting=True

    def capture_snapshot(self, front, key, *, coherence_only=False):
        """Callback executes the inherited migration-0011 query and returns a seal.

        Invoked on the actual candidate before mining. Future historical lookups
        reuse this captured snapshot; a later current DB is never substituted.
        """
        slot=(front,key)
        if slot in self.snapshots and not coherence_only:
            return
        record=self.capture_sql(front,key)
        from pathlib import Path
        raw=Path(record["path"]).read_bytes()
        require(hashlib.sha256(raw).hexdigest()==record["sha256"],"owned SQL capture seal changed")
        captured=strict_json(raw)
        require(captured["schema"]=="spc-lab-bound-sql-capture/1"
                and captured["binding"]["run_id"]==self.capture.scenario["run"]["id"],"source SQL run binding")
        from economic_checkpoint import query
        sql_contract=getattr(getattr(self,'checkpoint_retention',None),'sql_contract',CONTRACT)
        require(captured["query_sha256"]==hashlib.sha256(query(source_contract=sql_contract).encode()).hexdigest(),
                "source SQL capture is not the closed checkpoint query")
        pool=next(f["pool_id"] for f in self.capture.scenario["fronts"] if f["id"]==front)
        require(captured["binding"]["pool_id"]==pool and type(captured["execution"]["exit_code"]) is int
                and captured["execution"]["exit_code"]==0 and captured["execution"]["timed_out"] is False,
                "source SQL pool/execution binding")
        attempt={"artifact":dict(record),"status":"INCONCLUSIVE"}
        self.snapshot_attempts.setdefault(slot,[]).append(attempt)
        binding={"front":front,"job_key":list(key),"artifact":dict(record)}
        self.capture.capture.add("source_sql_snapshot_attempt",binding)
        if getattr(self,"checkpoint_retention",None) is not None:
            # Retain the original read even if its current revision is too new
            # for this requested key. It may only be admitted for an exactly
            # matching captured job by the existing checkpoint oracle.
            self.checkpoint_retention.retain(front,record,{"kind":"provider_capture_attempt","front":front,"job_key":list(key)},expected_observation=captured["observation"])
        # An early incomplete checkpoint must remain evidence, not poison the
        # historical lookup for every later, fresh exchange of the same job.
        try:
            p=self.capture.sources[front].jobs[key]["payload"]
            if p.get("economic") is None:
                raise MissingComponent("historical_job_frozen_economics")
            parent=parent_view(p,self.views)
            checked=replay_checkpoint(captured["observation"],p["economic"],parent,
                                      source_contract=CONTRACT,migration_checksums=self.migrations)
            if slot in self.snapshots:
                original=replay_checkpoint(self.snapshots[slot],p["economic"],parent,
                                           source_contract=CONTRACT,migration_checksums=self.migrations)
                require(checked["bootstrap"]==original["bootstrap"],"reload hint requires a new coherent frozen generation; historical snapshot cannot be replaced")
        except Exception as error:
            self.capture.capture.add("source_sql_snapshot_unavailable",{**binding,"class":type(error).__name__})
            raise
        self.capture.capture.add("source_sql_snapshot_admitted",{**binding,"scope":"coherent_row_checkpoint_only"})
        attempt["status"]="COHERENT_ROW_CHECKPOINT_ONLY"
        if slot not in self.snapshots:
            self.snapshots[slot]=captured["observation"]
            self.snapshot_refs[slot]=dict(record)

    @staticmethod
    def reload_hint(source,session):
        return max((decimal_uint(e["source"]["seq"]) for e in source.events
                    if e["kind"]=="economic_change_observed" and e["source"]["session"]==session),default=0)

    def _context(self, front, key, visiting, memo=None):
        if not getattr(self,"accepting",True):raise MissingComponent("source_feed_quiesced_for_final_drain")
        memo={} if memo is None else memo
        if (front,key) in memo:
            return memo[(front,key)]
        if (front,key) in visiting or len(visiting)>=4096:
            raise Inconclusive("economic ancestry cycle/bound")
        if (front,key) not in self.snapshots:
            raise MissingComponent("actual_historical_job_SQL_snapshot")
        source=self.capture.sources[front];p=source.jobs[key]["payload"]
        parent=parent_view(p,self.views)
        require(isinstance(parent,(Chain,ChainPrefix)),"raw economic parent evidence required")
        economic=p.get("economic")
        if economic is None:
            raise MissingComponent("historical_job_frozen_economics")
        observation=self.snapshots[(front,key)]
        replay=replay_checkpoint(observation,economic,parent,source_contract=CONTRACT,migration_checksums=self.migrations)
        from decision_observations import SCHEMA as DECISIONS, Index
        decisions=Index(observation) if observation.get('schema')==DECISIONS else None
        next_visiting=visiting | {(front,key)}
        for block_row in observation["tables"]["economic_block"]:
            block_hash=sql_bytes(block_row["block_hash"],32)[::-1].hex()
            publication=verify_publication(source,block_hash,self.views,self.capture.policy)
            old_key=tuple(publication["job_key"])
            old_economic=source.jobs[old_key]["payload"].get("economic")
            if old_economic is None:
                raise MissingComponent("historical_job_frozen_economics")
            # Orphaned historical branches can have a higher height than the
            # current parent. Their captured economic revisions, not height,
            # order this immutable dependency graph.
            require(integer(old_economic["revision"]) < integer(economic["revision"]),
                    "economic block is not prior to the current frozen revision")
            prior,window,old_job=self._context(front,old_key,next_visiting,memo)
            blocks=[v.by_hash[block_hash] for v in self.views.values() if block_hash in v.by_hash]
            if decisions is None:expected=self._settlement(source.jobs[old_key]["payload"],blocks[0],prior,window)
            else:
                from payout_history import rail_effects
                wire=source.block_publications[block_hash][0][1]
                derivation=decisions.front(source,old_key)
                if derivation['missing']:raise MissingComponent('historical_front_registration_provenance_incomplete')
                rail_effects(source.jobs[old_key]['payload'],wire,blocks[0],prior,window,block_row,
                    delivery_read=decisions.settlement(block_row,wire),derivation=derivation)
                expected=block_row['validated_ledger']
            require(expected==block_row["validated_ledger"],"actual mined E/ledger differs from immutable stored effects")
            for row in block_row["consumption"]:
                identity=bytes(row["sp_id"]).hex()
                ledger=next(r for r in expected if r["sp_id"]==row["sp_id"])
                require(row["prior"]==prior.get(identity,0) and row["error_pending"]==prior.get(identity,0)+ledger["pending_delta"],
                        "consumption uses current DB instead of verified frozen P/actual E")
        material=verify_job(source,key,self.views,self.capture.policy)
        profile=self._profile(front)
        window=prove_eligible(source,key,observation,self.views,self.capture.policy,profile,self.migrations)
        result=replay["available_from_replay"],window,material
        memo[(front,key)]=result
        return result

    def _profile(self,front):
        pool=next(f["pool_id"] for f in self.capture.scenario["fronts"] if f["id"]==front)
        return next(p["work_profile"] for p in self.capture.scenario["pools"] if p["id"]==pool)

    def _settlement(self,p,block,prior,window):
        fresh,_=rewards(window,block["subsidy"],block["fees"],p["fee_bps"])
        expected=[];used=set();outputs=block["transactions"][0]["outputs"]
        identities=[entry["sp_id"] for entry in p["entries"]]
        require(len(set(identities))==len(identities),"duplicate settlement identity")
        for identity in identities:unhex(identity,32)
        for value in prior.values():integer(value)
        owed_ids={identity for identity in set(fresh)|set(prior) if fresh.get(identity,0)+prior.get(identity,0)>0}
        require(owed_ids<=set(identities),"settlement omits an earned or available-pending identity")
        for entry in p["entries"]:
            identity=entry["sp_id"];before=prior.get(identity,0);owed=fresh.get(identity,0)+before
            require(decimal_uint(entry["owed_sats"])==owed,"actual owed differs from raw earnings plus frozen A")
            pieces,rem=ladder(owed)
            rail=integer(entry["rail"],0,3)
            if rail!=0 or not pieces:
                gap={1:"historical_Ark_registration_and_destination_provenance",
                     2:"actual_error_rail_registration_or_derivation_failure_provenance",
                     3:"historical_no_registration_proof_for_forfeiture"}.get(rail,"coinbase_rail_without_a_payable_rung")
                raise MissingComponent(gap)
            require(len(entry["p_x"])==len(pieces),"actual settlement ladder count")
            require(decimal_uint(entry["sats"])==sum(pieces),"frozen shaped amount differs from actual ladder")
            for key,amount in zip(entry["p_x"],pieces):
                script=bytes.fromhex("5120"+key)
                matches=[i for i,o in enumerate(outputs) if o["script"]==script and o["value"]==amount]
                require(len(matches)==1 and matches[0]!=0 and matches[0] not in used,"actual k-associated payout output")
                used.add(matches[0])
            # Kept-id residuals are never forfeited. Exact delivery provenance
            # stays frozen in the authenticated distribution, not current DB.
            delivery=entry["delivery_state"]
            if delivery not in {"registered_derived","registered_underived","unregistered"}:
                raise MissingComponent("unsupported_frozen_delivery_state")
            if delivery=="registered_derived":
                if entry["destination_digest"] is None:
                    raise MissingComponent("registered_delivery_destination_digest_missing")
                unhex(entry["destination_digest"],32)
                due=rem
            else:
                require(entry["destination_digest"] is None,"undeliverable state contradicts frozen destination")
                due=0
            error=rem-due
            expected.append({"sp_id":list(bytes.fromhex(identity)),"rail":"Coinbase","paid_sats":sum(pieces),
                             "ark_due_sats":due,"forfeited_sats":0,"pending_delta":error-before})
        for i,output in enumerate(outputs[1:],1):
            require(i in used or output["value"]==0 and output["script"].startswith(bytes.fromhex("6a24aa21a9ed")),"unattributed actual settlement output")
        require(sum(o["value"] for o in outputs)==block["subsidy"]+block["fees"],"actual settlement revenue")
        from economic_revenue import net_pool_output
        conservation=net_pool_output(block["subsidy"]+block["fees"],fresh,expected)
        require(outputs[0]["value"]==conservation["net_pool_output"],"actual net pool output conservation")
        return expected

    def prepare(self,agent,received):
        with self.lock:
            if not self.accepting:raise MissingComponent("source_feed_quiesced_for_final_drain")
            if self.checkpoint_retention is None:raise MissingComponent("pre_activation_checkpoint_retention_not_installed")
            current=self.refresh()
            if not isinstance(current,(Chain,ChainPrefix)) or array(received["prev_hash"],32)[::-1].hex()!=current.tip["hash"]:
                raise MissingComponent("current_two_node_canonical_parent_for_received_job")
            binding=self.agents[agent];front=binding["front"]
            if front in self.capture.issues or front not in self.capture.sources:
                raise MissingComponent("complete_signed_source_prefix")
            source=self.capture.sources[front]
            require(source.source_contract==CONTRACT and source.pool_pubkey==binding["pool_pubkey"],"source key must come from agent/front configuration")
            key=max(source.jobs,key=lambda k:int(source.jobs[k]["source"]["seq"]))
            p=source.jobs[key]["payload"]
            actual={"job_id":int(p["job_id"],16),"prev_hash":list(bytes.fromhex(p["prev_hash"])),
                "version":p["version"],"nbits":p["nbits"],"ntime":p["ntime"],"coinbase_prefix":list(bytes.fromhex(p["coinbase_prefix"])),
                "coinbase_suffix":list(bytes.fromhex(p["coinbase_suffix"])),"merkle_path":[list(bytes.fromhex(v)) for v in p["merkle_branch"]],
                "share_target_be":list(bytes.fromhex(p["share_target"])),"pool_extranonce":received["pool_extranonce"],
                "miner_extranonce_len":received["miner_extranonce_len"]}
            require(received_digest(actual)==received_digest(received)
                    and len(array(received["pool_extranonce"]))+received["miner_extranonce_len"]==p["extranonce_len"],"actual received job differs from latest captured frozen generation")
            hint=self.reload_hint(source,key[0])
            # A hint only schedules a fresh read. Neither its claimed revision
            # nor anchor supplies state. Preserve previously admitted snapshots.
            if hint>decimal_uint(source.jobs[key]["source"]["seq"]):
                self.capture_snapshot(front,key,coherence_only=True)
            elif (front,key) not in self.snapshots:self.capture_snapshot(front,key)
            from checkpoint_retention import retain_provider_checkpoint
            retain_provider_checkpoint(self.checkpoint_retention,self,front,key)
            prior,window,checked=self._context(front,key,set())
            # Public denomination/revenue/error routing checks also run on the
            # actual proposed job. Wallet-owned secret derivation is additional,
            # never replaced by these collector checks.
            from decision_observations import SCHEMA as DECISIONS, Index
            if self.snapshots[(front,key)].get('schema')==DECISIONS:
                from payout_history import proposed_outputs
                derivation=Index(self.snapshots[(front,key)]).front(source,key)
                if derivation['missing']:raise MissingComponent('current_front_registration_provenance_incomplete')
                proposed_outputs(p,checked['coinbase'],prior,window,int(p['subsidy']),checked['fees'],derivation)
            else:self._settlement(p,{"subsidy":int(p["subsidy"]),"fees":checked["fees"],"transactions":[checked["coinbase"]]},prior,window)
            pieces=self._window_material(source,window)
            require(all(j==checked["job"] for j in pieces["jobs"] if j["job_id"]==checked["job"]["job_id"]),
                    "current and referenced combined job ID have different immutable generation/bytes")
            finding=int(p["finding_slice_id"])
            require(p["finding_slice_excluded"] is True and finding not in [s["slice_id"] for s in pieces["slices"]],"finding slice is not excluded")
            fresh,_=rewards(window,int(p["subsidy"]),checked["fees"],p["fee_bps"])
            identities=set(fresh)|set(prior)|{binding["sp_id"]}|{r["sp_id"] for r in p["entries"]}
            pending={identity:prior.get(identity,0) for identity in identities}
            owed={identity:fresh.get(identity,0)+pending[identity] for identity in identities}
            require({i for i,n in owed.items() if n>0}<={r["sp_id"] for r in p["entries"]},"current job omits positive earned/available-pending identity")
            observation=self.snapshots[(front,key)]
            watermark=observation["history_boundary"]["event_watermark"]
            history=history_digest(watermark,pending)
            seal=source.events[-1]["source"]["pool_seal"]
            bundle={"schema":"spc.payout-proof/1","source_complete":True,"source_watermark":seal["seq"],
                "job":checked["job"],"jobs":pieces["jobs"],"reference_target_be":list(bytes.fromhex(self._profile(front)["reference_target"])),
                "work_unit_fingerprint":list(bytes.fromhex(p["work_unit_fingerprint"])),"window_n":self._profile(front)["window_n"],
                "window_target":str(window["window_target"]),"high_water":{k:int(v) for k,v in p["high_water"].items()},
                "finding_slice_id":finding,"slices":pieces["slices"],"subsidy":int(p["subsidy"]),"fees":checked["fees"],
                "fee_bps":p["fee_bps"],"prior":{"watermark":watermark,"digest":list(history),"pending":pending},"owed":owed,
                "window":window["window"],"witness_reserved":pieces["witness_reserved"]}
            current_reserved=p["witness_plan"]["reserved"] if p.get("witness_plan") is not None else None
            if current_reserved is not None:
                packed=str(checked["job"]["job_id"])
                require(packed not in bundle["witness_reserved"] or bundle["witness_reserved"][packed]==list(bytes.fromhex(current_reserved)),"current/referenced reserved value conflict")
                bundle["witness_reserved"][packed]=list(bytes.fromhex(current_reserved))
            encoded=canonical_json(bundle)
            if len(encoded)>MAX_BUNDLE:raise Inconclusive("complete source bundle exceeds bound")
            current=self.refresh()
            latest=self.capture.sources.get(front)
            if (front in self.capture.issues or not isinstance(current,(Chain,ChainPrefix))
                    or current.tip["hash"]!=array(received["prev_hash"],32)[::-1].hex()
                    or latest is None or max(latest.jobs,key=lambda k:int(latest.jobs[k]["source"]["seq"]))!=key
                    or self.reload_hint(latest,key[0])>hint):
                raise MissingComponent("source_candidate_or_canonical_parent_changed_during_replay")
            if not self.accepting:raise MissingComponent("source_feed_quiesced_for_final_drain")
            self.checkpoint_retention.require_healthy()
            return VerifiedEvidence(encoded,seal,checked["job"]["job_id"],key[3],watermark,history,_REPLAY_TOKEN)

    def _window_material(self,source,window):
        jobs={};slices=[];reserved={};ids=set()
        for mapping in window["mappings"]:
            sid=mapping["slice_id"];decoded=source.slice_publications[sid][0][1];shares=[]
            for event in source.accounts[sid]:
                key=source.job_key(event);a=event["payload"]
                if key not in jobs:
                    checked=verify_job(source,key,self.views,self.capture.policy)
                    j=checked["job"];require(j["job_id"] not in ids,"duplicate combined job identity across source generations")
                    ids.add(j["job_id"]);jobs[key]=j
                    plan=source.jobs[key]["payload"].get("witness_plan")
                    if plan is not None:reserved[str(j["job_id"])]=list(bytes.fromhex(plan["reserved"]))
                shares.append({"identity":list(bytes.fromhex(a["sp_id"])),"job_id":jobs[key]["job_id"],"generation":key[3],
                    "nonce":a["nonce"],"ntime":a["ntime"],"version":a["version"],"extranonce":list(bytes.fromhex(a["extranonce"])),
                    "share_index":a["share_index"],"share_hash":list(bytes.fromhex(a["share_hash"])),"credited_work":a["d"]})
            slices.append({"slice_id":sid,"height":decoded["height"],"front_id":decoded["front_id"],
                "work_unit_version":decoded["work_unit_version"],"work_unit_fingerprint":list(bytes.fromhex(decoded["work_unit_fingerprint"])),
                "difficulty":str(decoded["difficulty"]),"root":list(bytes.fromhex(decoded["root"])),"shares":shares})
        return {"jobs":list(jobs.values()),"slices":slices,"witness_reserved":reserved}
