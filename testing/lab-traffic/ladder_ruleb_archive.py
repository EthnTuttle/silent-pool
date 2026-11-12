"""Read-only 419 tested archive component audit; never a live/funded grant."""
import argparse
from collections import Counter, defaultdict
import hashlib
import json
from pathlib import Path
import re
import tarfile
from types import SimpleNamespace

from accounting_history import _replay
from case_contract import canonical_json
from economic_checkpoint import validate_initial, TABLES_R3, KEYS_R3, SCHEMA, QUERY_ID, query
from economic_revenue import net_pool_output
from oracle import Inconclusive, decode_block, integer, ladder, raw_payouts, require, strict_json, unhex
from pool_source import PoolSource
from ruleb_sql import REDACTED, parse_block, sql_bytes, validate_observation
from ruleb_source import bootstrap_view
from source_feed_codec import COMBINED_TESTED_CONTRACT
from source_feed_provider import ReplayProvider

GENESIS="0f9188f13cb7b2c71f2a335e3a4fc328bf5beb436012afca590b1a11466e2206"


def audit(handoff_path):
    h=strict_json(Path(handoff_path).read_bytes())
    require(h["source_snapshot_sha256"]==COMBINED_TESTED_CONTRACT,"archive source binding")
    refs={}
    def read(path,expected=None):
        raw=Path(path).read_bytes();digest=hashlib.sha256(raw).hexdigest()
        if expected is not None:require(digest==expected,"archive input hash changed: "+str(path))
        refs[str(path)]={"sha256":digest,"bytes":len(raw)}
        return raw
    read(handoff_path)
    for r in h["native_evidence"]:read(r["path"],r["sha256"])
    manifest=strict_json(read(h["source_manifest"]));source_rows={r["path"]:r["sha256"] for r in manifest["files"]}
    read(h["source_archive"],h["source_archive_sha256"]);migrations={}
    with tarfile.open(h["source_archive"]) as archive:
        for m in archive.getmembers():
            name=m.name.removeprefix("source/")
            if not m.isfile() or not name.startswith("crates/spc-store/migrations/"):continue
            raw=archive.extractfile(m).read();require(hashlib.sha256(raw).hexdigest()==source_rows[name],"migration source seal")
            version=int(Path(name).name[:4])
            if version<=10:migrations[version]=hashlib.sha384(raw).hexdigest()
    roots={str(Path(r["path"]).parent.parent) for r in h["native_evidence"] if r["path"].endswith("/private/pool.log")}
    require(len(roots)==1,"unique owned archive root");root=Path(roots.pop())
    receipt=strict_json(read(root/"receipt.json"))
    require(receipt["scope"]=="native_rule_b_component" and receipt["wallet_native_gate_exercised"] is False
            and receipt["funded_ark_acceptance"] is False,"archive component scope")
    config=read(root/"private/pool.toml").decode()
    def scalar(name):
        values=re.findall(r"^"+re.escape(name)+r"\s*=\s*(.+)$",config,re.MULTILINE)
        require(len(values)==1,"archive public pool config scalar: "+name)
        return strict_json(values[0])
    reference=int(scalar("work_reference_target"),16);window_n=scalar("window_n");fee_bps=scalar("fee_bps")
    source_raw=read(root/"private/pool.log")
    source=PoolSource(source_raw,independently_captured_log_sha256=hashlib.sha256(source_raw).hexdigest(),
        source_contract=COMBINED_TESTED_CONTRACT,pool_pubkey=h["native"]["source_pubkey"])
    observations={};seeds={};replays={}
    for name in ("initial","final"):
        o=strict_json(read(root/("private/"+name+"-sql.json")))
        validate_observation(o,TABLES_R3,SCHEMA,QUERY_ID)
        seed=validate_initial(o,migration_checksums=migrations,source_contract=COMBINED_TESTED_CONTRACT)
        replays[name]=_replay(o,TABLES_R3,KEYS_R3,REDACTED,SCHEMA,initial_state=seed)
        seeds[name]={"snapshot_digest":seed.snapshot_digest,"history_event_id":seed.history_event_id,"preceding_watermark":seed.preceding_watermark,
            "transaction_id":seed.transaction_id,"state_bytes_sha256":hashlib.sha256(seed.state_bytes).hexdigest()}
        observations[name]=o
    binding=strict_json(read(root/"qa-collector-binding.json"))
    require(binding["actual_runtime_source_binding"]==COMBINED_TESTED_CONTRACT
            and binding["query_sha256"]==hashlib.sha256(query(source_contract=COMBINED_TESTED_CONTRACT).encode()).hexdigest(),"actual closed SQL query/source binding")
    blocks={};raw_facts=[];previous=GENESIS
    for height in range(1,integer(receipt["tip"],1)+1):
        captured=strict_json(read(root/(f"private/block-{height}.json")))
        # Only pow-limit, raw Merkle/witness/weight and below-halving subsidy are
        # used here. No synthetic raw genesis or full contextual Chain is made.
        b=decode_block(unhex(captured["raw_hex"]),SimpleNamespace(pow_limit_bits=0x207fffff),production=True)
        require(captured["height"]==height and b["hash"]==captured["hash"] and b["prev"]==previous,"raw block file/height/ancestry join")
        require(height<150 and len(b["transactions"])==1,"archive raw fee oracle requires captured prevouts for non-coinbase transactions")
        cb=b["transactions"][0];require(sum(o["value"] for o in cb["outputs"])==5000000000,"raw actual subsidy/revenue")
        b.update(height=height,subsidy=5000000000,fees=0);blocks[b["hash"]]=b;previous=b["hash"]
        raw_facts.append({"height":height,"hash":b["hash"],"bits":b["bits"],"outputs":len(cb["outputs"]),"weight":b["weight"],"fees_sats":0,"pool_output_sats":cb["outputs"][0]["value"]})
    final=observations["final"];chain=sorted(final["tables"]["economic_chain"],key=lambda r:r["height"])
    require(len(chain)==len(blocks)+1,"actual SQL/raw chain census")
    for row in chain:
        bh=sql_bytes(row["block_hash"],32)[::-1].hex()
        if row["height"]==0:
            require(bh==GENESIS and sql_bytes(row["coinbase_digest"],32)==bytes(32),"SQL genesis anchor")
            continue
        require(bh in blocks,"SQL chain row missing captured raw block");b=blocks[bh]
        require(row["height"]==b["height"] and sql_bytes(row["parent_hash"],32)[::-1].hex()==b["prev"]
                and sql_bytes(row["coinbase_digest"],32)==hashlib.sha256(b["transactions"][0]["raw"]).digest(),"SQL/raw ancestry or coinbase digest")
    state=final["tables"]["economic_state"][0]
    require(state["complete"] is True and state["anchor_height"]==receipt["tip"] and sql_bytes(state["anchor_hash"],32)[::-1].hex()==previous,"final economic raw anchor")
    # Same-transaction pending changes must equal booked ledger changes.
    pending=defaultdict(lambda:defaultdict(int));ledger=defaultdict(lambda:defaultdict(int))
    for e in final["tables"]["source_accounting_history"]:
        if e["table_name"] not in {"pending_balance","ledger_history"}:continue
        for sign,side in ((-1,"before_row"),(1,"after_row")):
            r=e[side]
            if r is None:continue
            target=pending if e["table_name"]=="pending_balance" else ledger
            target[e["transaction_id"]][r["sp_id"]]+=sign*r["sats" if e["table_name"]=="pending_balance" else "pending_delta"]
    for xid in set(pending)|set(ledger):
        require(all(pending[xid].get(i,0)==ledger[xid].get(i,0) for i in set(pending[xid])|set(ledger[xid])),"actual booked pending lacks same-transaction ledger delta")
    economic=[];active=[];frozen_prior_claims={};mined_joins=[]
    for row in sorted(final["tables"]["economic_block"],key=lambda r:r["height"]):
        parsed=parse_block(row);bh=sql_bytes(row["block_hash"],32)[::-1].hex()
        require(bh in blocks,"economic block is not in actual raw archive")
        if parsed["lifecycle"] in {"unbooked","loss_held"}:active.append(parsed)
        pubs=source.block_publications.get(bh,[]);require(pubs and len({e["payload"]["raw_d_hex"] for e,_ in pubs})==1,"actual source block publication")
        wire=pubs[0][1];require(wire["economic"]==row["frozen_job"],"source/SQL FrozenJob bytes differ")
        jobs=[(k,e["payload"]) for k,e in source.jobs.items() if e["payload"]["fingerprint"]==wire["coinbase_fingerprint"] and e["payload"]["height"]==row["height"]]
        require(len(jobs)==1,"actual mined frozen generation join");key,p=jobs[0]
        require(p["fee_bps"]==fee_bps,"source fee policy differs from actual public config")
        from source_jobs import matches_coinbase_template
        require(matches_coinbase_template(p,blocks[bh]["transactions"][0]),"raw mined coinbase differs from source template")
        prior={bytes(r["sp_id"]).hex():r["available"] for r in row["frozen_job"]["balances"]}
        frozen_prior_claims[bh]=prior
        economic.append({"height":row["height"],"lifecycle":row["lifecycle"],"frozen_prior_sats":sum(r["prior"] for r in row["consumption"]),
            "pending_delta":sum(r["pending_delta"] for r in row["validated_ledger"]),"consumed_sats":sum(r["consumed"] for r in row["consumption"])})
        try:
            w=source.verified_window(key,reference_target=reference,window_n=window_n,prevout_transactions={})
            expected=ReplayProvider._settlement(None,p,blocks[bh],prior,w)
            require(expected==row["validated_ledger"],"independent raw fee/window/output ledger differs from actual SQL")
            mined_joins.append({"height":row["height"],"Q":w["window_target"],"actual_sum":w["window_work"],"raw_output_ledger_join":"PASSED","frozen_P_scope":"retained source/SQL claim; historical checkpoint still required"})
        except Inconclusive as error:
            mined_joins.append({"height":row["height"],"raw_output_ledger_join":"INCONCLUSIVE","missing_component":str(error)})
    view={"version":1,"revision":state["revision"],"anchor_height":state["anchor_height"],"anchor_hash":list(sql_bytes(state["anchor_hash"],32)),"complete":True,
        "booked":[[list(sql_bytes(r["sp_id"],32)),r["sats"]] for r in sorted(final["tables"]["pending_balance"],key=lambda r:r["sp_id"])],
        "reservations":sorted(active,key=lambda r:(r["height"],r["block_hash"]))}
    balances=bootstrap_view(view)["balances"]
    full_windows=[];latest=None
    for key,event in source.jobs.items():
        try:w=source.verified_window(key,reference_target=reference,window_n=window_n,prevout_transactions={})
        except Inconclusive:continue
        full_windows.append({"job_key":list(key),"height":event["payload"]["height"],"revision":event["payload"]["economic"]["revision"],"Q":w["window_target"],"actual_sum":w["window_work"]})
        latest=key,w
    require(latest is not None,"actual full-Q source window absent")
    key,w=latest;p=source.jobs[key]["payload"]
    if any(e["delivery_state"]!="unregistered" or e["destination_digest"] is not None for e in p["entries"]):
        raise Inconclusive("failed-proposal recomputation requires actual historical registration/delivery provenance")
    fresh,K=raw_payouts(w["selected"],int(p["subsidy"]),int(p["fees"]),p["fee_bps"]);proposal=[]
    for identity in sorted(set(fresh)|set(balances)):
        prior=balances.get(identity,{}).get("available",0);pieces,rem=ladder(fresh.get(identity,0)+prior)
        if not pieces:raise Inconclusive("failed-proposal below-ladder identity requires full historical forfeit/registration proof")
        proposal.append({"sp_id":list(bytes.fromhex(identity)),"rail":"Coinbase","paid_sats":sum(pieces),"ark_due_sats":0,"forfeited_sats":0,"pending_delta":rem-prior})
    correction=net_pool_output(int(p["subsidy"])+int(p["fees"]),fresh,proposal)
    correction.update(scope="independent recomputation from final SQL balances and last verified window; NOT mined or a new FrozenJob",prior_available=sum(r["available"] for r in balances.values()),
        source_window_job_key=list(key),source_window_revision=p["economic"]["revision"],final_SQL_revision=state["revision"],
        actual_source_guard_error=h["native_qa_component"]["observed_revenue_guard"],old_gross_floor_would_reject=correction["net_pool_output"]<K)
    return {"schema":"spc-lab-419-readonly-component/1","scope":"raw archive and SQL/source components only","input_artifacts":refs,
        "source_trace":{"status":"PASSED","events":len(source.events),"kinds":dict(Counter(e["kind"] for e in source.events)),"heads":source.heads,"economic_hint_authority":"none"},
        "initial_checkpoints":seeds,"SQL_row_replay":{"status":"PASSED","initial_events":replays["initial"]["events"],"final_events":replays["final"]["events"],"raw_chain_anchor_bound":True},
        "raw_blocks":{"status":"PASSED","blocks":raw_facts,"contextual_chain_claimed":False},"economic_blocks":economic,"full_windows":full_windows,"mined_output_joins":mined_joins,
        "final_balances":balances,"failed_proposal_recomputation":correction,"actual_consecutive_pending":"FAILED_BEFORE_PRODUCER_FIX",
        "original_error":receipt["error"],"native_wallet_gate":False,"funded_Ark_acceptance":False,"whole_lab_acceptance":"INCONCLUSIVE",
        "missing_components":["raw genesis/contextual full-chain observation","actual per-job historical coherent SQL checkpoints for full entitlement, including underfilled eligible-history proof",
                              "corrected producer tested consecutive-pending run","full native wallet gate and funded Ark lifecycle"]}


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--handoff",required=True);parser.add_argument("--out",required=True);args=parser.parse_args()
    result=audit(args.handoff);out=Path(args.out);out.mkdir(mode=0o700,exist_ok=False)
    path=out/"receipt.json";path.write_bytes(canonical_json(result)+b"\n");path.chmod(0o600)
    print(json.dumps({"receipt":str(path),"source_events":result["source_trace"]["events"],"raw_blocks":len(result["raw_blocks"]["blocks"]),"actual_consecutive_pending":result["actual_consecutive_pending"]}))


if __name__=="__main__":main()
