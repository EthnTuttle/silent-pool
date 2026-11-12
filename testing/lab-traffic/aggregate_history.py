"""SPc-private aggregate/order/grant joins; PoolSent never means received."""
from collections import defaultdict
import hashlib
import itertools

import accounting_history
from oracle import Inconclusive, Reader, integer, require
from ruleb_sql import sql_bytes
from source_feed_codec import tagged, verify_signature


def order_bytes(raw,public_key):
    require(93<=len(raw)<=348,"Ark order byte bound")
    r=Reader(raw);oid=r.take(16);destination=r.take(r.uint(1));sats=r.uint(8);height=r.uint(4);signature=r.take(64);r.end()
    require(destination and sats>0,"Ark order empty destination/value")
    destination.decode("utf-8")
    verify_signature(tagged("SPc/ArkSendOrder",raw[:-64]),signature.hex(),public_key)
    return {"order_id":oid,"destination":destination,"sats":sats,"not_before_height":height,"digest":tagged("SPc/FrozenArkOrderV1",raw)}


def parse_grant(raw):
    require(len(raw)==149,"Ark grant exact size")
    r=Reader(raw);require(r.take(5)==b"SPAG\x01","Ark grant envelope")
    result={"order_id":r.take(16),"order_digest":r.take(32),"nonce":r.take(16),"revision":r.uint(8),"expires_at":r.uint(8)}
    signature=r.take(64);r.end()
    require(result["nonce"]!=bytes(16) and result["revision"]>0 and result["expires_at"]>0,"Ark grant nonce/revision/expiry")
    return result,signature


def grant_bytes(raw,public_key):
    result,signature=parse_grant(raw)
    verify_signature(tagged("SPc/ArkAuthorizationV1",raw[:-64]),signature.hex(),public_key)
    return result


def proved_receipt(raw,public_key):
    require(len(raw)<=4*1024*1024,"Ark public receipt bound")
    r=Reader(raw);require(r.take(5)==b"SPRP\x01","Ark proved receipt envelope")
    grant=grant_bytes(r.take(149),public_key);sats=r.uint(8);count=r.uint(2)
    require(0<count<=2048 and sats>0,"Ark proved receipt count/value")
    proofs=[]
    for _ in range(count):
        length=r.uint(4);require(0<length<=1024*1024,"Ark VTXO proof bound");proofs.append(r.take(length))
    r.end();require(len(set(proofs))==len(proofs),"Ark duplicate VTXO proof bytes")
    return {"grant":grant,"sats":sats,"proofs":proofs}


def frozen_members(aggregate,members,before):
    """Validate the real freeze transaction against its before-image state."""
    require(0<len(members)<=2048,"aggregate complete member census")
    members=sorted(members,key=lambda m:m["ordinal"])
    require([m["ordinal"] for m in members]==list(range(len(members))),"aggregate contiguous frozen ordinals")
    keys=[(m["block_height"],m["block_hash"],m["sp_id"]) for m in members]
    require(keys==sorted(set(keys)),"aggregate oldest-first immutable member ordering")
    require(all(m["order_id"]==aggregate["order_id"] and m["sp_id"]==aggregate["sp_id"] and m["reserved"] is True and m["orphaned"] is False for m in members),"aggregate freeze identity/reservation")
    require((aggregate["anchor_height"],aggregate["anchor_hash"])==(members[0]["block_height"],members[0]["block_hash"]),"aggregate oldest anchor")
    require(sum(integer(m["sats"],1) for m in members)==aggregate["sats"] and max(m["due_height"] for m in members)==aggregate["not_before_height"]<=aggregate["selected_tip"],"aggregate amount/due-height freeze")
    for m in members:
        p=before["ark_payout"].get((m["block_hash"],m["sp_id"]));b=before["block_found"].get((m["block_hash"],))
        require(p is not None and b is not None and p["status"]=="scheduled" and b["status"]=="confirmed"
            and p["sats"]==m["sats"] and p["due_height"]==m["due_height"] and b["height"]==m["block_height"],"aggregate member lacks actual scheduled confirmed freeze ancestry")
        require(not any(old["reserved"] and (old["block_hash"],old["sp_id"])==(m["block_hash"],m["sp_id"]) for old in before["ark_aggregate_member"].values()),"aggregate double reservation at freeze")
    oid=hashlib.sha256(b"SPc/ArkAggregateV1"+sql_bytes(aggregate["sp_id"],32)+sql_bytes(aggregate["anchor_hash"],32)+integer(aggregate["batch_seq"],1).to_bytes(8,"little")).digest()[:16]
    require(sql_bytes(aggregate["order_id"],16)==oid,"aggregate derived opaque order id")
    return members


def audit_aggregates(observation,seed,views,public_key,publication_destinations=None,*,decisions=None):
    """Call after the complete accounting row replay has succeeded."""
    from economic_checkpoint import observation_layout
    _,keys,_,_=observation_layout(observation)
    aggregates=observation["tables"]["ark_aggregate"];members=observation["tables"]["ark_aggregate_member"]
    state={table:{} for table in keys};state.update(seed.replay_seed());freezes={};authorized=defaultdict(list);missing=[]
    if decisions is not None:
        try:decisions.authorization_history()
        except Inconclusive as error:missing.append({'field':str(error)})
    for xid,group in itertools.groupby(observation["tables"]["source_accounting_history"],key=lambda e:e["transaction_id"]):
        group=list(group)
        for event in group:
            if event["table_name"]=="ark_aggregate" and event["operation"]=="INSERT":
                a=event["after_row"];oid=a["order_id"]
                require(oid not in freezes and a["state"]=="frozen" and a["revision"]==1,"aggregate initial state/revision")
                inserted=[e["after_row"] for e in group if e["table_name"]=="ark_aggregate_member" and e["operation"]=="INSERT" and e["after_row"]["order_id"]==oid]
                frozen=frozen_members(a,inserted,state)
                freezes[oid]={"transaction_id":xid,"members":frozen,"aggregate":a}
            if event["table_name"]=="ark_aggregate" and event["after_row"] is not None and event["after_row"]["state"]=="authorized":
                authorized[event["after_row"]["order_id"]].append(event)
        for event in group:
            if event["operation"]=="history_start":continue
            table=event["table_name"];before,after=event["before_row"],event["after_row"]
            if table=="ark_aggregate_member" and before is not None:
                require(after is not None and all(before[k]==after[k] for k in ("order_id","ordinal","block_hash","sp_id","block_height","due_height","sats")),"aggregate membership rewritten/deleted")
            if table=="ark_aggregate" and before is not None:
                require(after is not None and all(before[k]==after[k] for k in ("batch_seq","order_id","sp_id","anchor_hash","anchor_height","sats","not_before_height","selected_tip","selected_tip_hash","server_pubkey")),"aggregate frozen fields rewritten/deleted")
            if before is not None:del state[table][accounting_history.row_key(table,before,keys)]
            if after is not None:state[table][accounting_history.row_key(table,after,keys)]=after
    require(set(freezes)=={a["order_id"] for a in aggregates},"aggregate freeze/current census")
    grants={g["order_id"]:g for g in observation["tables"]["ark_send_authorization"]}
    require(len(grants)==len(observation["tables"]["ark_send_authorization"]) and set(grants)<=set(freezes),"aggregate grant identity census")
    checked=[]
    for a in aggregates:
        oid=a["order_id"];freeze=freezes[oid];frozen=freeze["members"]
        current=sorted((m for m in members if m["order_id"]==oid),key=lambda m:m["ordinal"])
        require(len(current)==len(frozen) and all(all(m[k]==f[k] for k in ("order_id","ordinal","block_hash","sp_id","block_height","due_height","sats")) for m,f in zip(current,frozen)),"aggregate final membership differs from freeze")
        known=[v for v in views.values() if sql_bytes(a["selected_tip_hash"],32)[::-1].hex() in v.by_hash]
        if known:
            tip_hash=sql_bytes(a["selected_tip_hash"],32)[::-1].hex();view=known[0].prefix(a["selected_tip"])
            require(view.tip["hash"]==tip_hash and all(sql_bytes(m["block_hash"],32)[::-1].hex() in view.by_hash for m in frozen),"aggregate frozen members absent from declared raw branch")
        else:missing.append({"order_id":oid,"field":"raw_selected_tip_and_frozen_member_branch"})
        local={"order_id":oid,"freeze_transaction_id":freeze["transaction_id"],"members":len(frozen),"sats":a["sats"],"state":a["state"],"wallet_received":False}
        anchor=(a["anchor_hash"],a["sp_id"])
        if publication_destinations is not None and anchor in publication_destinations:
            require(sql_bytes(a["destination"])==publication_destinations[anchor],"aggregate destination differs from oldest member SPCB bytes")
            local["oldest_destination_matches_publication"]=True
        else:missing.append({"order_id":oid,"field":"oldest_member_actual_SPCB_destination"})
        if a["server_pubkey"] is not None:
            from signature_oracle import public_point
            public_point(sql_bytes(a["server_pubkey"],33))
            local["frozen_server_pubkey"]=a["server_pubkey"]
        else:missing.append({"order_id":oid,"field":"frozen_server_public_key"})
        missing.extend({"order_id":oid,"field":field} for field in ("selection_time_supported_server_policy_and_full_candidate_prefix","independently_observed_freeze_canonical_read","historical_oldest_anchor_destination_derivation"))
        if public_key is None:
            missing.append({"order_id":oid,"field":"configured_public_Ark_order_signing_key"});checked.append(local);continue
        order=order_bytes(sql_bytes(a["order_bytes"]),public_key)
        require(order=={"order_id":sql_bytes(oid,16),"destination":sql_bytes(a["destination"]),"sats":a["sats"],"not_before_height":a["not_before_height"],"digest":sql_bytes(a["order_digest"],32)},"aggregate signed order/frozen SQL mismatch")
        signed_grants=[]
        if decisions is not None:
            try:
                issued=decisions.authorization(oid,public_key,views)
                require(len(issued)<=1,'aggregate issued replacement monetary grant')
                signed_grants.extend(r['grant'] for r in issued)
                local['authorization_decision_ids']=[r['observation_id'] for r in issued]
                if authorized[oid] and not issued:missing.append({'order_id':oid,'field':'original_authorization_decision_missing_not_backfilled'})
            except Inconclusive as error:missing.append({'order_id':oid,'field':str(error)})
        if oid in grants:
            row=grants[oid];g=grant_bytes(sql_bytes(row["grant_bytes"]),public_key)
            require(all(g[k]==sql_bytes(row[k],16 if k in {"order_id","nonce"} else 32) for k in ("order_id","nonce","order_digest")) and g["revision"]==row["revision"] and g["expires_at"]==row["expires_at"],"aggregate stored signed grant mismatch")
            signed_grants.append(g)
        if a["state"]=="sent":
            receipt=proved_receipt(sql_bytes(a["resolution_proof"]),public_key)
            require(receipt["sats"]==a["sats"],"aggregate public receipt amount")
            signed_grants.append(receipt["grant"])
            local["public_vtxo_sha256"]=[hashlib.sha256(raw).hexdigest() for raw in receipt["proofs"]]
            missing.append({"order_id":oid,"field":"AP5_full_policy_server_anchor_each_VTXO_and_wallet_receipt"})
        for g in signed_grants:
            require(g["order_id"]==order["order_id"] and g["order_digest"]==order["digest"],"aggregate grant binds foreign order")
            events=[e for e in authorized[oid] if e["after_row"]["revision"]==g["revision"]]
            require(len(events)==1 and events[0]["before_row"]["state"]=="published" and g["revision"]==events[0]["before_row"]["revision"]+1,"aggregate grant lacks actual authorized revision transition")
        local["signed_grants_checked"]=len(signed_grants)
        if authorized[oid]:missing.append({"order_id":oid,"field":"actual_SPCA_ack_and_local_cosign_escape_capture" if decisions is not None else "issuance_and_consumption_time_canonical_read_and_complete_grant_history"})
        checked.append(local)
    return {"status":"INCONCLUSIVE" if missing else "NO_AGGREGATES" if not checked else "PASSED_COMPONENT","aggregates":checked,"missing":missing,"wallet_received":False}
