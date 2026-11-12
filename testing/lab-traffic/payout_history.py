"""Independent full-rail arithmetic and immutable history joins.

Delivery decisions require the actual reconstruction-time read, not today's
ark_delivery or the earlier signed front cache. Available facts are checked
even when those missing facts prevent complete entitlement admission.
"""
from collections import defaultdict
from copy import deepcopy
import hashlib
import itertools

import accounting_history
from audited_window import prove_eligible, rewards
from economic_checkpoint import observation_layout, validate_initial
from economic_revenue import net_pool_output
from net_debits import available, block_economics
from oracle import Inconclusive, integer, ladder, require, unhex
from pool_source import decimal_uint
from ruleb_sql import REDACTED, parse_block, replay_checkpoint, sql_bytes, validate_observation
from source_jobs import MissingComponent, parent_view, verify_publication


def proposed_outputs(payload,coinbase,prior,window,subsidy,fees,derivation):
    """Verify a proposal's raw entitlement shape, never invent future E/ledger.

    Settlement-time delivery can differ from this front-cache observation.
    Actual post-block effects still require the reconstruction/read FK join.
    """
    fresh,_=rewards(window,subsidy,fees,payload['fee_bps']);entries=payload['entries'];outputs=coinbase['outputs'];used=set();seen=set();paid=0
    for entry in entries:
        identity=entry['sp_id'];require(identity not in seen,'proposal duplicate identity');seen.add(identity)
        owed=integer(prior.get(identity,0))+fresh.get(identity,0);pieces,_=ladder(owed);rail=integer(entry['rail'],0,3)
        require(decimal_uint(entry['owed_sats'])==owed,'proposal frozen owed mismatch')
        require(decimal_uint(entry['sats'])==(sum(pieces) if rail==0 else fresh.get(identity,0) if rail==3 else owed),'proposal full-rail shaped amount')
        observed=derivation['rows'][identity]
        if rail==0:
            require(observed['coinbase_key_status']=='available' and pieces and len(entry['p_x'])==len(pieces),'proposal complete coinbase keys/decomposition')
            for key,amount in zip(entry['p_x'],pieces):
                from ladder_wire import xonly
                script=b'\x51\x20'+unhex(xonly(unhex(key,32)),32)
                matches=[i for i,o in enumerate(outputs) if o['script']==script]
                require(len(matches)==1 and matches[0]!=0 and matches[0] not in used and outputs[matches[0]]['value']==amount,'proposal k-associated output')
                used.add(matches[0]);paid+=amount
        else:
            require(not entry['p_x'],'proposal noncoinbase keys')
            if owed>=500000:require(observed['coinbase_key_status']=='missing_miner_keys','proposal high-value hold lacks key-unavailability observation')
            if rail==1:require(observed['destination_bytes'] is not None,'proposal Ark destination unavailable')
    require({i for i in set(prior)|set(fresh) if prior.get(i,0)+fresh.get(i,0)>0}<=seen,'proposal omits positive owed')
    for i,o in enumerate(outputs[1:],1):require(i in used or o['value']==0 and o['script'].startswith(bytes.fromhex('6a24aa21a9ed')),'proposal unattributed output')
    total=subsidy+fees;require(paid<=total and outputs[0]['value']==total-paid and sum(o['value'] for o in outputs)==total,'proposal net pool/revenue')
    return {'paid':paid,'net_pool_output':total-paid,'future_settlement_ledger_claimed':False}


def rail_effects(payload, wire, block, prior, window, stored,*,delivery_read=None,derivation=None):
    """Check actual raw payouts/ledger on all rails; report unproved decisions.

    prior/window must come from replay_checkpoint/prove_eligible. Missing keys
    in prior are zero only because that caller proved complete-from-empty B/R.
    This function alone grants neither that ancestry nor wallet ownership.
    """
    parse_block(stored)
    require(wire["economic"]==payload["economic"]==stored["frozen_job"],"fullrail frozen economic binding")
    require(wire["block_hash"]==block["hash"] and stored["block_hash"]=="\\x"+unhex(block["hash"],32)[::-1].hex(),"fullrail block identity")
    require(len(wire["entries"])==len(payload["entries"]),"fullrail entry census")
    fresh,_=rewards(window,block["subsidy"],block["fees"],payload["fee_bps"])
    ledger={bytes(row["sp_id"]).hex():row for row in stored["validated_ledger"]}
    consumption={bytes(row["sp_id"]).hex():row for row in stored["consumption"]}
    identities=[e["sp_id"] for e in payload["entries"]]
    require(len(set(identities))==len(identities) and set(ledger)==set(consumption)==set(identities),"fullrail identity census")
    require({i for i in set(fresh)|set(prior) if fresh.get(i,0)+prior.get(i,0)>0}<=set(identities),"fullrail positive owed omitted")
    outputs=block["transactions"][0]["outputs"];used=set();missing=[];checked=[]
    for entry,actual in zip(payload["entries"],wire["entries"]):
        identity=entry["sp_id"];unhex(identity,32);P=integer(prior.get(identity,0));raw=integer(fresh.get(identity,0));owed=P+raw
        rail=integer(entry["rail"],0,3);pieces,remainder=ladder(owed)
        require(actual["sp_id"]==identity and actual["rail"]==rail and actual["p_x"]==entry["p_x"],"fullrail SPCB/source rail/key mismatch")
        require(actual["owed_sats"]==decimal_uint(entry["owed_sats"])==owed,"fullrail owed differs from raw plus frozen P")
        expected_sats=sum(pieces) if rail==0 else raw if rail==3 else owed
        require(actual["sats"]==decimal_uint(entry["sats"])==expected_sats,"fullrail frozen shaped amount")
        destination=unhex(actual["dest_id"])
        digest=hashlib.sha256(destination).hexdigest() if destination else None
        require(entry["destination_digest"]==digest,"fullrail SPCB destination digest")
        require(entry["delivery_state"] in {"registered_derived","registered_underived","unregistered"},"fullrail frozen delivery state")
        if destination:require(entry["delivery_state"]=="registered_derived","fullrail destination contradicts frozen delivery state")
        row=ledger[identity];paid=0
        if rail==0:
            require(pieces and len(entry["p_x"])==len(pieces),"fullrail complete k-ordered decomposition")
            for key,value in zip(entry["p_x"],pieces):
                script=b"\x51\x20"+unhex(key,32)
                matches=[i for i,o in enumerate(outputs) if o["script"]==script]
                require(len(matches)==1 and matches[0]!=0 and matches[0] not in used,"fullrail missing/duplicate output")
                index=matches[0];require(outputs[index]["value"]==value,"fullrail k-associated denomination")
                used.add(index);paid+=value
            require(row["rail"]=="Coinbase" and row["forfeited_sats"]==0,"fullrail kept remainder cannot forfeit")
            possible={(0,remainder)}
            if destination:possible.add((remainder,0))
            if delivery_read is not None:
                due=remainder if destination and identity in delivery_read['delivered'] else 0
                possible={(due,remainder-due)}
            E=P+row["pending_delta"]
            require((row["ark_due_sats"],E) in possible,"fullrail coinbase residual routing")
            if destination and remainder and delivery_read is None:missing.append({"sp_id":identity,"field":"settlement_delivery_read_at_reconstruction"})
            missing.append({"sp_id":identity,"field":"wallet_all_k_tweak_derivation_provenance"})
        else:
            require(not entry["p_x"] and row["paid_sats"]==0,"fullrail noncoinbase paid output")
            # Enumerate only routes permitted by actual immutable destination
            # bytes; never infer a registration row from the chosen ledger.
            possible=[]
            if destination:possible.append(("Ark",owed,0,0))
            if not destination or owed>=500_000:possible.append(("Pending",0,0,owed))
            if owed<500_000:possible.append(("Forfeit",0,raw,P))
            if delivery_read is not None:
                delivered=identity in delivery_read['delivered']
                possible=[('Ark',owed,0,0)] if delivered and destination else [('Pending',0,0,owed)] if delivered or owed>=500000 else [('Forfeit',0,raw,P)]
            E=P+row["pending_delta"]
            require((row["rail"],row["ark_due_sats"],row["forfeited_sats"],E) in possible,"fullrail impossible Ark/pending/forfeit effects")
            if delivery_read is None:missing.append({"sp_id":identity,"field":"settlement_delivery_read_at_reconstruction"})
            if owed>=500_000:
                if derivation is not None:require(derivation['rows'][identity]['coinbase_key_status']=='missing_miner_keys',"high-value pending has available frozen coinbase keys")
                else:missing.append({"sp_id":identity,"field":"frozen_high_value_key_unavailability_or_derivation_failure"})
        require(row["paid_sats"]==paid,"fullrail ledger paid differs from raw outputs")
        require(paid+row["ark_due_sats"]+row["forfeited_sats"]+E==owed,"fullrail per-identity conservation")
        econ=block_economics(P,E);actual_c=consumption[identity]
        require(actual_c["prior"]==P and actual_c["error_pending"]==E and actual_c["delta"]==econ["delta"] and actual_c["consumed"]==econ["net_debit"],"fullrail per-identity P/E/delta/c")
        if destination:missing.append({"sp_id":identity,"field":"independent_Ark_destination_tweak_and_full_policy_proof" if derivation is not None else "historical_registration_server_mailbox_destination_derivation"})
        checked.append({"sp_id":identity,"raw":raw,"P":P,"E":E,"delta":econ["delta"],"c":econ["net_debit"],"paid":paid,
            "ark_due":row["ark_due_sats"],"forfeit":row["forfeited_sats"],"rail":row["rail"],"destination_sha256":digest})
    for i,output in enumerate(outputs[1:],1):
        require(i in used or output["value"]==0 and output["script"].startswith(bytes.fromhex("6a24aa21a9ed")),"fullrail unattributed raw output")
    total=block["subsidy"]+block["fees"]
    require(sum(o["value"] for o in outputs)==total,"fullrail raw block revenue")
    net=net_pool_output(total,fresh,stored["validated_ledger"])
    require(outputs[0]["value"]==net["net_pool_output"],"fullrail net pool value")
    return {"status":"VERIFIED_RAW_RAIL_EFFECTS","block_hash":block["hash"],"height":block["height"],"per_identity":checked,
        "owned_output_derivation_verified":False,"raw_matched_outputs":len(used),"net_pool":net,"missing":missing}


def transaction_effects(observation,seed):
    """Replay all booked/reserved effects, not just r16's selected pair."""
    tables,keys,schema,_=observation_layout(observation)
    accounting_history._replay(observation,tables,keys,REDACTED,schema,initial_state=seed)
    state={table:{} for table in keys};state.update(seed.replay_seed());seen=set();proofs=[]
    def balances():
        B={r["sp_id"]:r["sats"] for r in state["pending_balance"].values()};R=defaultdict(dict)
        for block in state["economic_block"].values():
            if block["lifecycle"] not in {"unbooked","loss_held"}:continue
            for row in block["consumption"]:R["\\x"+bytes(row["sp_id"]).hex()][block["block_hash"]]=row["consumed"]
        for identity in set(B)|set(R):available(B.get(identity,0),list(R[identity].values()))
        return B,R
    for xid,items in itertools.groupby(observation["tables"]["source_accounting_history"],key=lambda e:e["transaction_id"]):
        if xid in seen:raise MissingComponent("interleaved_SQL_transactions_require_commit_order")
        seen.add(xid);items=list(items);beforeB,beforeR=balances();ledger_delta=defaultdict(int);transitions=[]
        for event in items:
            if event["operation"]=="history_start":continue
            table=event["table_name"];before,after=event["before_row"],event["after_row"]
            if table=="economic_block":
                if before is not None and after is not None:
                    immutable=("block_hash","parent_hash","height","payload_digest","frozen_job","validated_ledger","consumption")
                    if 'reconstruction_observation' in after:immutable+=('reconstruction_observation',)
                    require(all(before[k]==after[k] for k in immutable),"fullrail immutable economic rewrite")
                if before is None or after is None or before["lifecycle"]!=after["lifecycle"]:
                    transitions.append({"block_hash":(after or before)["block_hash"],"before":before["lifecycle"] if before else None,"after":after["lifecycle"] if after else None})
            if table=="ledger_history":
                for sign,row in ((-1,before),(1,after)):
                    if row is not None:ledger_delta[row["sp_id"]]+=sign*row["pending_delta"]
            if before is not None:del state[table][accounting_history.row_key(table,before,keys)]
            if after is not None:state[table][accounting_history.row_key(table,after,keys)]=deepcopy(after)
        afterB,afterR=balances();per_identity=[]
        for identity in sorted(set(beforeB)|set(afterB)|set(beforeR)|set(afterR)|set(ledger_delta)):
            b0=beforeB.get(identity,0);b1=afterB.get(identity,0)
            require(b1-b0==ledger_delta[identity],"fullrail booked pending lacks same-transaction ledger delta")
            r0,r1=beforeR[identity],afterR[identity]
            if b0!=b1 or r0!=r1:
                per_identity.append({"sp_id":sql_bytes(identity,32).hex(),"B_before":b0,"B_after":b1,"R_before":sum(r0.values()),"R_after":sum(r1.values()),
                    "A_before":available(b0,list(r0.values())),"A_after":available(b1,list(r1.values())),
                    "reservations_before":r0,"reservations_after":r1,"booked_delta":ledger_delta[identity]})
        if transitions or per_identity:proofs.append({"transaction_id":xid,"event_ids":[e["event_id"] for e in items],"transitions":transitions,"per_identity":per_identity})
    return proofs


def audit_pool(sources,observations,final,views,policy,profile,migrations,*,descriptor=None,order_pubkey=None):
    """Inputs are original retained captures; no current-view backfill."""
    tables,_,schema,query_id=observation_layout(final)
    validate_observation(final,tables,schema,query_id)
    contracts={s.source_contract for s in sources.values()};require(len(contracts)==1,"fullrail source contract census")
    contract=next(iter(contracts));seed=validate_initial(final,migration_checksums=migrations,source_contract=contract)
    transactions=transaction_effects(final,seed);blocks=[];missing=[];owned={};destinations={}
    from decision_observations import SCHEMA as DECISIONS, Index
    decisions=Index(final) if schema==DECISIONS else None
    for front,source in sources.items():
        for bh in source.block_publications:
            require(bh not in owned,"fullrail duplicate cross-front block publication")
            owned[bh]=(front,source)
    rows={sql_bytes(r["block_hash"],32)[::-1].hex():r for r in final["tables"]["economic_block"]}
    require(len(rows)==len(final["tables"]["economic_block"]),"fullrail duplicate SQL block")
    for bh in sorted(set(rows)|set(owned)):
        try:
            if bh not in owned:raise MissingComponent("actual_signed_publication_for_economic_block")
            if bh not in rows:raise MissingComponent("actual_economic_acceptance_consumption_row")
            front,source=owned[bh];publication=verify_publication(source,bh,views,policy);key=tuple(publication["job_key"])
            p=source.jobs[key]["payload"];parent=parent_view(p,views);snapshot=None;errors=[]
            for record,observation in observations[front]:
                if record["revision"]!=p["economic"]["revision"]:continue
                try:replayed=replay_checkpoint(observation,p["economic"],parent,source_contract=contract,migration_checksums=migrations)
                except Inconclusive as error:errors.append(str(error));continue
                snapshot=(record,observation,replayed);break
            if snapshot is None:raise MissingComponent("exact_frozen_checkpoint_revision_"+str(p["economic"]["revision"])+":"+repr(errors))
            record,observation,replayed=snapshot
            window=prove_eligible(source,key,observation,views,policy,profile,migrations)
            block=next(v.by_hash[bh] for v in views.values() if bh in v.by_hash)
            wire=source.block_publications[bh][0][1]
            delivery=derivation=None;decision_gaps=[]
            if decisions is not None:
                for kind in ('delivery','derivation'):
                    try:
                        if kind=='delivery':delivery=decisions.settlement(rows[bh],wire)
                        else:derivation=decisions.front(source,key)
                    except Inconclusive as error:decision_gaps.append({'field':str(error)})
            result=rail_effects(p,wire,block,replayed["available_from_replay"],window,rows[bh],delivery_read=delivery,derivation=derivation)
            result['missing'].extend(decision_gaps)
            if derivation is not None:result['missing'].extend(derivation['missing'])
            if decisions is not None:result['decision_observations']={'delivery':delivery,'derivation':derivation}
            for entry in wire["entries"]:
                destinations[(rows[bh]["block_hash"],"\\x"+entry["sp_id"])]=unhex(entry["dest_id"])
            result.update(front=front,job_key=list(key),checkpoint=record,window=window["window"],history_watermark=replayed["history_watermark"])
            if descriptor is None:result["missing"].append({"field":"actual_public_pool_descriptor"})
            else:
                from ladder_final_archive import pool_script
                require(block["transactions"][0]["outputs"][0]["script"]==pool_script(descriptor,block["height"]),"fullrail pool descriptor output")
            blocks.append(result);missing.extend({"block_hash":bh,**item} for item in result["missing"])
        except Inconclusive as error:missing.append({"block_hash":bh,"field":str(error)})
    if not blocks:missing.append({"field":"actual_mined_pool_entitlement_cases"})
    for tx in transactions:
        for transition in tx["transitions"]:
            if transition["after"] in {"cancelled","loss_held"} or transition["before"] in {"cancelled","loss_held"}:
                missing.append({"transaction_id":tx["transaction_id"],"field":"transaction_bound_raw_orphan_restoration_observation"})
    from aggregate_history import audit_aggregates
    aggregates=audit_aggregates(final,seed,views,order_pubkey,destinations,decisions=decisions);missing.extend(aggregates["missing"])
    return {"status":"INCONCLUSIVE" if missing else "PASSED","scope":"independent_fullrail_history_and_raw_effects",
        "blocks":blocks,"transactions":transactions,"aggregates":aggregates,"missing":missing,"wallet_received":False}
