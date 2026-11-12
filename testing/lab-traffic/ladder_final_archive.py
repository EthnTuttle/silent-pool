"""Independent corrected Rule B archive replay; no live process/RPC or writes to inputs."""
import argparse
from collections import Counter, defaultdict
from datetime import datetime, timezone
import hashlib
import hmac
import itertools
import json
from pathlib import Path
import re
import tarfile

import accounting_history
from audited_window import prove_eligible, rewards
from case_contract import canonical_json
from economic_checkpoint import validate_initial, TABLES_R3, KEYS_R3, SCHEMA, QUERY_ID
from economic_revenue import net_pool_output
from net_debits import block_economics, book
from oracle import Chain, Inconclusive, Invalid, Policy, integer, ladder, require, sha256d, strict_json, unhex
from pool_source import PoolSource, ANSI
from public_derivation import _N, _GX, _GY, _point_add
from ruleb_sql import REDACTED, parse_block, replay_checkpoint, sql_bytes, validate_observation
from ruleb_source import bootstrap_view
from signature_oracle import _multiply, public_point
from source_feed_codec import FINAL_CORRECTED_CONTRACT, tagged
from source_feed_provider import ReplayProvider
from source_feed_runtime import public_identity
from source_jobs import MissingComponent, verify_publication
from storage import sha_file


def compact_point(point):
    require(point is not None,"derived public point at infinity")
    return bytes([2+(point[1]&1)])+point[0].to_bytes(32,"big")


def pool_script(descriptor,height):
    """Exact archived tr(tpub/unhardened/*) subset, wildcard = block height."""
    match=re.fullmatch(r"tr\((tpub[1-9A-HJ-NP-Za-km-z]+)((?:/(?:0|[1-9][0-9]*))*)/\*\)",descriptor)
    if match is None:raise MissingComponent("unsupported_archived_public_pool_descriptor")
    alphabet="123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";number=0
    for c in match[1]:number=number*58+alphabet.index(c)
    raw=number.to_bytes((number.bit_length()+7)//8,"big")
    require(len(raw)==82 and sha256d(raw[:-4])[:4]==raw[-4:] and raw[:4]==bytes.fromhex("043587cf"),"pool tpub checksum/network/length")
    payload=raw[:-4];chaincode=payload[13:45];point=public_point(payload[45:])
    steps=[int(v) for v in match[2].split("/") if v]+[height]
    for child in steps:
        integer(child,0,0x7fffffff)
        material=hmac.digest(chaincode,compact_point(point)+child.to_bytes(4,"big"),"sha512")
        tweak=int.from_bytes(material[:32],"big");require(tweak<_N,"invalid public descriptor child")
        point=_point_add(point,_multiply(tweak,(_GX,_GY)));require(point is not None,"public child at infinity");chaincode=material[32:]
    x=point[0].to_bytes(32,"big");point=public_point(b"\2"+x)
    tweak=int.from_bytes(tagged("TapTweak",x),"big");require(tweak<_N,"invalid key-path taproot tweak")
    output=_point_add(point,_multiply(tweak,(_GX,_GY)));require(output is not None,"taproot output at infinity")
    return b"\x51\x20"+output[0].to_bytes(32,"big")


def policy_from_profile(value):
    c=value["consensus"]
    return Policy(c["genesis_hash"],int(c["pow_limit_bits"],16),int(c["initial_target_bits"],16),int(c["regtest_pow_bits_overlay"],16),
        c["no_pow_retargeting"],c["allow_min_difficulty_blocks"],c["pow_target_spacing"],c["pow_target_timespan"],c["difficulty_adjustment_interval"],
        c["enforce_bip94"],int(c["retarget_floor_bits"],16),int(c["subsidy_initial_sats"]),c["subsidy_halving_interval"],c["coinbase_maturity"])


class Audit:
    def __init__(self,handoff):
        self.handoff=Path(handoff);self.h=strict_json(self.handoff.read_bytes())
        require(self.h["source_sha256"]==FINAL_CORRECTED_CONTRACT,"corrected archive source binding")
        self.root=Path(self.h["native"]["run"]);self.refs={};self.seals={r["path"]:r for r in self.h["native_evidence"]}
        require(len(self.seals)==len(self.h["native_evidence"]),"duplicate native artifact seal")
        self.migrations={};self.contexts={};self.visiting=set();self.publications={};self.owned={}

    def read(self,path):
        path=Path(path);require(str(path) in self.seals,"archive read is not externally sealed: "+str(path))
        raw=path.read_bytes();seal=self.seals[str(path)]
        require(len(raw)==seal["bytes"] and hashlib.sha256(raw).hexdigest()==seal["sha256"],"actual artifact changed: "+str(path))
        self.refs[str(path)]={"sha256":seal["sha256"],"bytes":len(raw)}
        return raw

    def load(self):
        for item in self.h["native_evidence"]:require(sha_file(item["path"])==item["sha256"],"native artifact seal")
        for item in self.h["binaries"]:require(sha_file(item["path"])==item["sha256"],"native binary hash")
        for item in self.h["gate_receipts"]:require(item["exit"]==0 and sha_file(item["path"])==item["sha256"],"producer gate log seal")
        manifest=strict_json(Path(self.h["source_manifest"]).read_bytes());source_rows={r["path"]:r["sha256"] for r in manifest["files"]}
        require(sha_file(self.h["source_archive"])==self.h["source_archive_sha256"],"producer source archive seal")
        with tarfile.open(self.h["source_archive"]) as archive:
            members={m.name.removeprefix("source/"):m for m in archive.getmembers() if m.isfile()}
            require(set(members)==set(source_rows) and len(members)==723,"producer source row census")
            for name,member in members.items():
                raw=archive.extractfile(member).read();require(hashlib.sha256(raw).hexdigest()==source_rows[name],"source archive row: "+name)
                if name.startswith("crates/spc-store/migrations/") and int(Path(name).name[:4])<=10:
                    self.migrations[int(Path(name).name[:4])]=hashlib.sha384(raw).hexdigest()
        self.profile=strict_json(self.read(self.root/"node-profile.json"));self.policy=policy_from_profile(self.profile)
        raws=[]
        for height in range(self.h["native"]["tip"]+1):
            r=strict_json(self.read(self.root/f"private/block-{height}.json"));require(r["height"]==height,"raw archive height label")
            raw=unhex(r["raw_hex"]);require(sha256d(raw[:80])[::-1].hex()==r["hash"],"raw block hash label");raws.append(raw)
        self.chain=Chain(self.policy,raws);self.views={self.chain.tip["hash"]:self.chain}
        self.log=self.read(self.root/"private/pool.log")
        self.log_lines=self.log.decode().splitlines()
        self.source=PoolSource(self.log,independently_captured_log_sha256=hashlib.sha256(self.log).hexdigest(),
            source_contract=FINAL_CORRECTED_CONTRACT,pool_pubkey=self.h["native"]["source_pubkey"])
        self.config=self.read(self.root/"private/pool.toml").decode()
        def scalar(name):
            values=re.findall(r"^"+re.escape(name)+r"\s*=\s*(.+)$",self.config,re.M)
            require(len(values)==1,"archive public pool config field: "+name);return strict_json(values[0])
        self.window_profile={"reference_target":scalar("work_reference_target"),"window_n":scalar("window_n")}
        self.descriptor=scalar("pool_payout_descriptor");self.fee_bps=scalar("fee_bps")
        section=re.search(r'^\[pool\]\s*\n(.*?)(?=^\[|\Z)',self.config,re.M|re.S)
        require(section is not None,"actual pool config section")
        endpoint=re.search(r'^listen\s*=\s*(.+)$',section[1],re.M)
        require(endpoint is not None,"actual pool SV2 listener")
        pool_endpoint=strict_json(endpoint[1])
        authority=scalar("authority_public_key")
        miner_config=self.read(self.root/"private/miner.toml").decode();self.native_configs={}
        for body in miner_config.split("[[connections]]")[1:]:
            def value(name):
                values=re.findall(r"^"+re.escape(name)+r"\s*=\s*(.+)$",body,re.M);require(len(values)==1,"actual native connection field")
                return strict_json(values[0])
            worker=re.search(r'^workers=\[\{id="([^"]+)",budget=\{hashes_per_second=([0-9]+),total_attempts=([0-9]+)\},ntime_policy="hold",max_ntime_roll_seconds=600,batch_size=1\}\]$',body,re.M)
            if worker is None:raise MissingComponent("unsupported_archived_native_worker_config")
            identity=value("identity");scan,sp_id=public_identity(identity)
            require(value("endpoint")==pool_endpoint and value("authority_public_key")==authority,"actual direct pool SV2/Noise route")
            cid=value("id");require(cid not in self.native_configs,"duplicate archived native connection")
            self.native_configs[cid]={"identity":identity,"sp_id":sp_id,"worker_id":worker[1],"hashes_per_second":int(worker[2]),"total_attempts":int(worker[3])}
        require(len(self.native_configs)==3 and sum(c["hashes_per_second"] for c in self.native_configs.values())==40,"archived component native budget/census")
        self.snapshots={};self.snapshot_audits=[]
        for name in self.seals:
            if not ("/coherent-sql-captures/" in name or name.endswith(("/initial-sql.json","/final-sql.json","/consecutive-reservations-sql.json"))):continue
            observation=strict_json(self.read(name));validate_observation(observation,TABLES_R3,SCHEMA,QUERY_ID)
            seed=validate_initial(observation,migration_checksums=self.migrations,source_contract=FINAL_CORRECTED_CONTRACT)
            replay=accounting_history._replay(observation,TABLES_R3,KEYS_R3,REDACTED,SCHEMA,initial_state=seed)
            state=observation["tables"]["economic_state"][0]
            self.snapshot_audits.append({"path":name,"sha256":self.seals[name]["sha256"],"revision":state["revision"],"history_watermark":replay["event_watermark"],"events":replay["events"]})
            self.snapshots.setdefault(state["revision"],[]).append((name,observation))
            if name.endswith("/final-sql.json"):self.final=observation;self.seed=seed
        self.final_blocks={sql_bytes(r["block_hash"],32)[::-1].hex():r for r in self.final["tables"]["economic_block"]}
        for bh in self.source.block_publications:
            self.publications[bh]=verify_publication(self.source,bh,self.views,self.policy)
        pre=strict_json(self.read(self.root/"pre-mining.json"))
        require(pre["pool_config_sha256"]==self.seals[str(self.root/"private/pool.toml")]["sha256"]
                and pre["miner_config_sha256"]==self.seals[str(self.root/"private/miner.toml")]["sha256"]
                and pre["initial_sql_sha256"]==self.seals[str(self.root/"private/initial-sql.json")]["sha256"] and pre["budget_hps"]==40,"actual pre-mining bindings")
        self.identity_records=strict_json(self.read(self.root/"private/identities.json"));self.keys={}
        for item in self.identity_records:
            public={k:item[k] for k in ("address","b_scan","b_spend")};require(public in pre["identities"],"archived wallet/public identity mismatch")
            scan_pub,identity=public_identity(item["address"]);scan=int.from_bytes(unhex(item["scan_key"],32),"big")
            require(0<scan<_N and compact_point(_multiply(scan,(_GX,_GY))).hex()==item["b_scan"] and item["b_scan"][2:]==scan_pub,"archived scan authority mismatch")
            # Spend secret is not used. Only the supplied public spend key is
            # needed to independently check output tweaks against raw scripts.
            self.keys[identity]=(scan,public_point(unhex(item["b_spend"],33)))
        require(len(self.keys)==len(pre["identities"])==3,"component identity census")
        self.miner=self.read(self.root/"private/miner.log");require(self.miner.endswith(b"\n"),"native miner log truncated")
        self.miner_events=[strict_json(line) for line in self.miner.splitlines()]
        for seq,r in enumerate(self.miner_events,1):
            require(r["protocol"]=="spc-cpu-miner-events" and r["version"]==1 and r["kind"]=="event" and r["source"]=="process"
                    and r["event_id"]==seq and r["source_cursor"]==seq and r["dropped_before"]==0,"complete actual native event prefix")

    def context(self,key):
        if key in self.contexts:return self.contexts[key]
        if key in self.visiting:raise Invalid("economic ancestry cycle")
        p=self.source.jobs[key]["payload"];parent=self.chain.prefix(p["height"]-1);revision=p["economic"]["revision"]
        candidates=[(name,o) for name,o in self.snapshots.get(revision,[]) if o["tables"]["economic_state"][0]["complete"] is True
                    and o["tables"]["economic_state"][0]["anchor_height"]==parent.tip["height"]
                    and sql_bytes(o["tables"]["economic_state"][0]["anchor_hash"],32)[::-1].hex()==parent.tip["hash"]]
        if not candidates:raise MissingComponent(f"actual_exact_frozen_SQL_checkpoint_height_{p['height']}_revision_{revision}")
        name,o=sorted(candidates,key=lambda x:(x[1]["history_boundary"]["event_watermark"],x[0]))[0]
        row_replay=replay_checkpoint(o,p["economic"],parent,source_contract=FINAL_CORRECTED_CONTRACT,migration_checksums=self.migrations)
        self.visiting.add(key)
        try:
            for old in o["tables"]["economic_block"]:
                bh=sql_bytes(old["block_hash"],32)[::-1].hex();old_key=tuple(self.publications[bh]["job_key"])
                require(old["frozen_job"]["revision"]<revision,"economic dependency not prior to frozen revision")
                previous=self.context(old_key);block=self.chain.by_hash[bh]
                expected=ReplayProvider._settlement(None,self.source.jobs[old_key]["payload"],block,previous["prior"],previous["window"])
                require(expected==old["validated_ledger"],"historical actual raw E differs from stored ledger")
                for row in old["consumption"]:
                    identity=bytes(row["sp_id"]).hex();e=next(r for r in expected if r["sp_id"]==row["sp_id"])
                    require(row["prior"]==previous["prior"].get(identity,0)
                            and row["error_pending"]==row["prior"]+e["pending_delta"]
                            and row["consumed"]==max(0,-e["pending_delta"]),"historical per-identity P/E/c mismatch")
            window=prove_eligible(self.source,key,o,self.views,self.policy,self.window_profile,self.migrations)
            result={"prior":row_replay["available_from_replay"],"window":window,"replay":row_replay,"snapshot":name}
            self.contexts[key]=result;return result
        finally:self.visiting.remove(key)

    def ownership(self,bh,key):
        if bh in self.owned:return self.owned[bh]
        block=self.chain.by_hash[bh];p=self.source.jobs[key]["payload"];cb=block["transactions"][0]
        require(cb["outputs"][0]["script"]==pool_script(self.descriptor,block["height"]),"raw net pool output is not the height-indexed configured descriptor")
        send=public_point(b"\2"+unhex(p["a_send_x"],32));ih=int.from_bytes(tagged("SPc/Inputs",block["height"].to_bytes(4,"big")+compact_point(send)),"big")
        require(0<ih<_N,"SPc input hash scalar range")
        scripts=defaultdict(list)
        for index,output in enumerate(cb["outputs"]):scripts[output["script"]].append((index,output["value"]))
        used=set();identities=[];commitment=hashlib.sha256()
        for entry in p["entries"]:
            identity=entry["sp_id"]
            if identity not in self.keys:raise MissingComponent("actual_archived_wallet_scan_authority_for_output_tweaks")
            scan,spend=self.keys[identity];shared=compact_point(_multiply(ih*scan,send));pieces,rem=ladder(int(entry["owed_sats"]))
            require(len(pieces)==len(entry["p_x"]),"archived owned rung count")
            total=0
            for k,(value,px) in enumerate(zip(pieces,entry["p_x"])):
                tweak_raw=tagged("SPc/TierSpc",shared+k.to_bytes(4,"big"));tweak=int.from_bytes(tweak_raw,"big")
                require(0<tweak<_N,"SPc tier tweak scalar range")
                point=_point_add(spend,_multiply(tweak,(_GX,_GY)));require(point is not None and point[0].to_bytes(32,"big").hex()==px,"independently derived SPc tweak/key mismatch")
                matches=scripts[b"\x51\x20"+unhex(px,32)]
                require(len(matches)==1 and matches[0][0]!=0 and matches[0][0] not in used and matches[0][1]==value,"actual k-associated owned raw output")
                used.add(matches[0][0]);total+=value
                commitment.update(bytes.fromhex(identity)+k.to_bytes(4,"big")+hashlib.sha256(tweak_raw).digest())
            identities.append({"sp_id":identity,"outputs":len(pieces),"sats":total,"declared_owed":int(entry["owed_sats"]),"remainder":rem})
        require(all(i in used or i==0 or output["value"]==0 and output["script"].startswith(b"\x6a") for i,output in enumerate(cb["outputs"])),"unattributed raw payout output")
        result={"pool_descriptor_height_script_verified":True,"owned_outputs":len(used),"owned_sats":sum(r["sats"] for r in identities),"identities":identities,
            "derived_tweak_hash_commitment":commitment.hexdigest(),"private_scalar_material_emitted":False,"scope":"archived scan-authority derivation; not entropy or live wallet ACK proof"}
        self.owned[bh]=result;return result

    def native_join(self,bh,key):
        block=self.chain.by_hash[bh];p=self.source.jobs[key]["payload"]
        matches=[r for r in self.miner_events if r["event"]["name"]=="submitted" and r["event"]["payload"]["share"]["block_hash"]==bh
                 and r["event"]["payload"]["share"]["job_id"]==int(key[2],16)]
        require(len(matches)==1,"actual native winning submission census");event=matches[0];share=event["event"]["payload"]["share"]
        require(share["header"]==block["raw"].hex() and share["job_id"]==int(key[2],16) and share["block_qualified"] is True,"native submitted header/job binding")
        connection=self.native_configs[share["connection_id"]]
        winners=[e for group in self.source.accounts.values() for e in group if e["payload"]["share_hash"]==bytes.fromhex(bh)[::-1].hex() and self.source.job_key(e)==key]
        require(len(winners)==1 and winners[0]["payload"]["sp_id"]==connection["sp_id"] and share["worker_id"]==connection["worker_id"]
                and share["ntime"]==p["ntime"],"actual native worker/identity/ntime attribution")
        active=[r for r in self.miner_events if r["event_id"]<event["event_id"] and r["event"]["name"]=="job_activated"
                and r["event"]["payload"]["connection_id"]==share["connection_id"] and r["event"]["payload"]["job"]["job_id"]==share["job_id"]]
        require(active,"native winning share has no observed direct-SV2 job activation");job=active[-1]["event"]["payload"]["job"]
        require(job["coinbase_tx_prefix"]==p["coinbase_prefix"] and job["coinbase_tx_suffix"]==p["coinbase_suffix"]
                and job["prev_hash_le"]==p["prev_hash"] and job["nbits"]==p["nbits"] and job["version"]==p["version"]
                and job["merkle_path_le"]==p["merkle_branch"] and job["share_target_le"]==bytes.fromhex(p["share_target"])[::-1].hex()
                and job["target_generation"]==share["target_generation"],"actual native SV2 context differs from frozen source")
        require(unhex(job["extranonce_prefix"])+unhex(share["extranonce"])==unhex(winners[0]["payload"]["extranonce"]),
                "actual native pool-prefix/miner-extranonce differs from attributed full nonce")
        return {"miner_event_id":event["event_id"],"activated_event_id":active[-1]["event_id"],"connection_id":share["connection_id"],
            "worker_id":share["worker_id"],"session_slot":share["session_slot"],"header_sha256":hashlib.sha256(block["raw"]).hexdigest(),"wallet_gate_ACK_claimed":False}

    def wall_ns(self,event):
        text=ANSI.sub("",self.log_lines[event["line"]-1])
        match=re.match(r"(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d)(?:\.([0-9]{1,9}))?Z",text)
        if match is None:raise MissingComponent("archived_source_log_wall_timestamp")
        seconds=int(datetime.fromisoformat(match[1]).replace(tzinfo=timezone.utc).timestamp())
        return seconds*1000000000+int((match[2] or "").ljust(9,"0"))

    def block(self,bh):
        row=self.final_blocks.get(bh)
        if row is not None:parse_block(row)
        key=tuple(self.publications[bh]["job_key"]);p=self.source.jobs[key]["payload"];block=self.chain.by_hash[bh]
        candidates=[e for e in self.source.events if e["kind"]=="provisional_candidate" and e["source"]["session"]==key[0]
            and e["payload"]["height"]==p["height"] and e["payload"]["job_id"]==key[2] and int(e["payload"]["generation"])==key[3]]
        require(len(candidates)==1 and candidates[0]["payload"]["canonical_ack_required"] is True,"actual provisional candidate evidence")
        candidate=candidates[0];publication=self.source.block_publications[bh][0][0]
        issued=[int(e["source"]["seq"]) for e in self.source.issued[key]]
        require(max(issued)<int(candidate["source"]["seq"])<int(publication["source"]["seq"]),"actual source issue/candidate/publication order")
        if row is not None:
            expected_c={bytes(r["sp_id"]).hex():r["consumed"] for r in row["consumption"] if r["consumed"]>0}
            require(dict(candidate["payload"]["net_debits"])==expected_c,"signed provisional per-identity net debits differ from immutable SQL")
        result={"height":p["height"],"block_hash":bh,"lifecycle":row["lifecycle"] if row else None,"job_key":list(key),
            "source_order":{"frozen":int(self.source.jobs[key]["source"]["seq"]),"issued":issued,"candidate":int(candidate["source"]["seq"]),"publication":int(publication["source"]["seq"])},
            "native_SV2":self.native_join(bh,key),"raw_owned_outputs":self.ownership(bh,key),"entitlement_status":"INCONCLUSIVE"}
        if row is None:
            result["missing_component"]=f"actual_SQL_economic_block_and_exact_frozen_checkpoint_height_{p['height']}_revision_{p['economic']['revision']}"
            return result
        try:
            context=self.context(key);prior=context["prior"];window=context["window"]
            ledger=ReplayProvider._settlement(None,p,block,prior,window)
            require(ledger==row["validated_ledger"],"actual per-identity raw E differs from frozen ledger")
            fresh,_=rewards(window,block["subsidy"],block["fees"],p["fee_bps"])
            conservation=net_pool_output(block["subsidy"]+block["fees"],fresh,ledger)
            require(conservation["net_pool_output"]==block["transactions"][0]["outputs"][0]["value"],"actual net pool coinbase value")
            balances=bootstrap_view(context["replay"]["bootstrap"])["balances"];rows=[]
            for entry in ledger:
                identity=bytes(entry["sp_id"]).hex();P=prior.get(identity,0);E=P+entry["pending_delta"];economics=block_economics(P,E)
                stored=next(r for r in row["consumption"] if r["sp_id"]==entry["sp_id"])
                require(stored["prior"]==P and stored["error_pending"]==E and stored["delta"]==economics["delta"] and stored["consumed"]==economics["net_debit"],"actual per-identity P/E/delta/c mismatch")
                rows.append({"sp_id":identity,**balances.get(identity,{"booked":0,"reserved":0,"available":0}),"P":P,"raw":fresh.get(identity,0),"paid":entry["paid_sats"],"E":E,"delta":economics["delta"],"c":economics["net_debit"]})
            timing={"candidate_log_wall_ns":self.wall_ns(candidate),"publication_log_wall_ns":self.wall_ns(publication),
                "snapshot_file_precedes_candidate":Path(context["snapshot"]).stat().st_mtime_ns<=self.wall_ns(candidate),
                "scope":"sealed log wall timestamps and observed file metadata; not a live wallet activation ACK"}
            result.update(entitlement_status="PASSED_COMPONENT_ENTITLEMENT",window=window["window"],capture_timing=timing,snapshot={"path":context["snapshot"],"sha256":self.seals[context["snapshot"]]["sha256"],
                "revision":context["replay"]["revision"],"history_watermark":context["replay"]["history_watermark"],"file_mtime_ns_observed":Path(context["snapshot"]).stat().st_mtime_ns},
                per_identity=rows,total_P=sum(r["P"] for r in rows),total_delta=sum(r["delta"] for r in rows),total_c=sum(r["c"] for r in rows),net_pool=conservation)
        except Inconclusive as error:result["missing_component"]=str(error)
        return result

    def booking(self,results):
        """Replay real transactions, never synthesize missing job checkpoints."""
        history=self.final["tables"]["source_accounting_history"];state={table:{} for table in KEYS_R3};state.update(self.seed.replay_seed())
        seen=set();proofs=[];by_hash={r["block_hash"]:r for r in results}
        def balance_view():
            B={r["sp_id"]:r["sats"] for r in state["pending_balance"].values()};R=defaultdict(list)
            for block in state["economic_block"].values():
                if block["lifecycle"] not in {"unbooked","loss_held"}:continue
                for r in block["consumption"]:R["\\x"+bytes(r["sp_id"]).hex()].append(r["consumed"])
            return B,R
        for xid,group in itertools.groupby(history,key=lambda e:e["transaction_id"]):
            if xid in seen:raise MissingComponent("interleaved_SQL_transactions_need_commit_order_proof")
            seen.add(xid);group=list(group)
            targets=[e for e in group if e["table_name"]=="economic_block" and e["operation"]=="UPDATE" and e["before_row"]["lifecycle"]=="unbooked"
                     and e["after_row"]["lifecycle"]=="booked" and e["after_row"]["height"] in (16,17)]
            beforeB,beforeR=balance_view()
            for e in group:
                if e["operation"]=="history_start":continue
                table=e["table_name"]
                if e["before_row"] is not None:del state[table][accounting_history.row_key(table,e["before_row"],KEYS_R3)]
                if e["after_row"] is not None:state[table][accounting_history.row_key(table,e["after_row"],KEYS_R3)]=e["after_row"]
            afterB,afterR=balance_view()
            for event in targets:
                row=event["after_row"];bh=sql_bytes(row["block_hash"],32)[::-1].hex();verified=by_hash[bh]
                require(verified["entitlement_status"]=="PASSED_COMPONENT_ENTITLEMENT","booking lacks independently verified P/E")
                ledger={bytes(r["sp_id"]).hex():r for r in row["validated_ledger"]};per_identity=[]
                for v in verified["per_identity"]:
                    identity=v["sp_id"];sql_id="\\x"+identity;e=block_economics(v["P"],v["E"])
                    expected=book(beforeB.get(sql_id,0),beforeR[sql_id],v["c"],e)
                    require(afterB.get(sql_id,0)==expected["booked_pending"] and sum(afterR[sql_id])==sum(expected["reservations"]),"same-transaction booking B/R retirement mismatch")
                    inserts=[x["after_row"] for x in group if x["table_name"]=="ledger_history" and x["operation"]=="INSERT" and x["after_row"]["sp_id"]==sql_id and x["after_row"]["block_hash"]==row["block_hash"]]
                    require(len(inserts)==1 and all(inserts[0][k]==ledger[identity][k] for k in ("paid_sats","ark_due_sats","forfeited_sats","pending_delta")),"booking ledger INSERT does not match verified actual effects")
                    per_identity.append({"sp_id":identity,"B_before":beforeB.get(sql_id,0),"B_after":afterB.get(sql_id,0),"R_before":sum(beforeR[sql_id]),"R_after":sum(afterR[sql_id]),
                        "own_c_retired":v["c"],"delta_booked":v["delta"],"A_after":expected["available"]})
                proofs.append({"height":row["height"],"transaction_id":xid,"event_ids":[e["event_id"] for e in group],"per_identity":per_identity,
                    "scope":"actual same-transaction own reservation retirement; later unproved blocks are not certified by this arithmetic"})
        require({p["height"] for p in proofs}=={16,17},"consecutive pair booking transaction census")
        return proofs

    def run(self):
        self.load();results=[]
        require(set(self.source.block_publications)==set(self.chain.by_hash)-{self.policy.genesis},"raw chain/source publication census")
        for bh in sorted(self.source.block_publications,key=lambda b:self.chain.by_hash[b]["height"]):results.append(self.block(bh))
        pair=[r for r in results if r["height"] in (16,17)];booking=self.booking(results)
        require(len(pair)==2 and all(r["lifecycle"]=="booked" and r["entitlement_status"]=="PASSED_COMPONENT_ENTITLEMENT" for r in pair),"actual consecutive booked pair not proved")
        require(pair[0]["total_c"]==1500000 and pair[1]["total_c"]==260870 and pair[1]["total_delta"]==-1,"independent pair arithmetic differs from the declared target")
        early=[r for r in results if r["height"]<=8];missing=[{"height":r["height"],"component":r["missing_component"]} for r in results if r["entitlement_status"]!="PASSED_COMPONENT_ENTITLEMENT"]
        return {"schema":"spc-lab-independent-corrected-ruleb/1","scope":"three-identity single-front archived native Rule B component",
            "status":"PARTIAL_COMPONENT_PASS","source_sha256":FINAL_CORRECTED_CONTRACT,"source_events":len(self.source.events),"source_heads":self.source.heads,
            "source_event_kinds":dict(Counter(e["kind"] for e in self.source.events)),
            "archive_binaries":self.h["binaries"],"native_component_configuration":self.native_configs,"configured_aggregate_hps":40,
            "raw_chain":{"blocks_after_genesis":self.chain.tip["height"],"tip":self.chain.tip["hash"],"contextual_chain_verified":True,"total_fees":sum(b.get("fees",0) for b in self.chain.blocks)},
            "SQL_history_events":len(self.final["tables"]["source_accounting_history"]),"captured_SQL_snapshots":self.snapshot_audits,
            "final_SQL_anchor":self.final["tables"]["economic_state"][0],
            "pair_status":"PASSED_INDEPENDENT_P_E_PER_IDENTITY_RESERVATION_AND_BOOKING_COMPONENT","pair":pair,"booking_transactions":booking,
            "early_warmup_status":"PASSED_COMPONENT_ENTITLEMENT" if all(r["entitlement_status"]=="PASSED_COMPONENT_ENTITLEMENT" for r in early) else "INCONCLUSIVE",
            "blocks":results,"missing_entitlement_checkpoints":missing,"input_artifacts":self.refs,
            "prior_failed_archives_unchanged":True,"live_wallet_activation_ACK_verified":False,"full_run_entropy_verified":False,"current_float_verified":False,
            "Ark_lifecycle_verified":False,"full_native_audited_RuleB_acceptance":False,"whole_lab_acceptance":"INCONCLUSIVE",
            "remaining_obligations":["actual exact frozen checkpoints at H19-H22 and H23 plus H23 economic acceptance/consumption row; final SQL anchor is H22, not raw tip H23; no later-state reconstruction",
                "full-run entropy and actual current float/Ark send receive refresh offboard proofs","full native wallet gate source/commit/ACK/first-hash evidence on the joined tested bundle",
                "full campaign topology/census/budget and cleanup evidence; this older CPU component is not the complete campaign","separate installed restricted-ACL auditor recheck"]}


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--handoff",required=True);parser.add_argument("--out",required=True);args=parser.parse_args()
    out=Path(args.out);out.mkdir(mode=0o700,exist_ok=False)
    result=Audit(args.handoff).run();path=out/"receipt.json";path.write_bytes(canonical_json(result)+b"\n");path.chmod(0o600)
    print(json.dumps({"receipt":str(path),"pair_status":result["pair_status"],"early_warmup_status":result["early_warmup_status"],"missing":result["missing_entitlement_checkpoints"],"whole_lab_acceptance":"INCONCLUSIVE"}))


if __name__=="__main__":main()
