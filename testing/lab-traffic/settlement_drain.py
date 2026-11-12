"""Stop/converge/EOF/final-read sequencing, separate from entitlement approval."""
import hashlib
import time

from case_contract import canonical_json
from oracle import Inconclusive, integer, require
from ruleb_sql import sql_bytes, parse_block
from source_jobs import matches_coinbase_template


def convergence_gaps(chain,sources,observation,depth=6):
    """One pool's source fronts and original SQL, including foreign chain blocks.

    Other pools and the native funder have canonical economic_chain rows, not
    fabricated local economic_block/ledger rows. Local ownership comes from
    matching actual frozen coinbases and signed source publications.
    """
    integer(depth,1);tables=observation["tables"];gaps=[]
    states=tables["economic_state"]
    if len(states)!=1:return ["economic singleton missing"]
    state=states[0];tip=chain.tip
    if (state["complete"] is not True or state["anchor_height"]!=tip["height"]
            or state["anchor_hash"]!="\\x"+bytes.fromhex(tip["hash"])[::-1].hex()):
        gaps.append("economic anchor has not reached observed raw tip")
    chain_rows={r["height"]:r for r in tables["economic_chain"]}
    if len(chain_rows)!=len(tables["economic_chain"]) or set(chain_rows)!=set(range(tip["height"]+1)):
        gaps.append("canonical economic chain census incomplete")
    economic={r["block_hash"]:r for r in tables["economic_block"]};found={r["block_hash"]:r for r in tables["block_found"]}
    require(len(economic)==len(tables["economic_block"]) and len(found)==len(tables["block_found"]),"drain duplicate block identity")
    published=set();applied=set();local=set()
    for source in sources:
        published.update(source.block_publications)
        applied.update(e["payload"]["block_hash"] for e in source.events if e["kind"]=="pending_applied")
        by_height={}
        for e in source.jobs.values():by_height.setdefault(e["payload"]["height"],[]).append(e["payload"])
        for block in chain.blocks[1:]:
            if any(p["prev_hash"]==bytes.fromhex(block["prev"])[::-1].hex() and matches_coinbase_template(p,block["transactions"][0]) for p in by_height.get(block["height"],[])):
                local.add(block["hash"])
    for block in chain.blocks:
        height=block["height"];bh=block["hash"];sql_hash="\\x"+bytes.fromhex(bh)[::-1].hex();row=chain_rows.get(height)
        owned=bh in local or bh in published
        if row is None or row["block_hash"]!=sql_hash or row["parent_hash"]!="\\x"+bytes.fromhex(block["prev"])[::-1].hex() or row["pool_block"] is not owned:
            gaps.append(f"H{height}: canonical economic chain row/ownership missing")
        if row is not None:
            expected=bytes(32) if height==0 else hashlib.sha256(block["transactions"][0]["raw"]).digest()
            if sql_bytes(row["coinbase_digest"],32)!=expected:gaps.append(f"H{height}: canonical coinbase digest mismatch")
        if not owned:continue
        if bh not in published:gaps.append(f"H{height}: signed publication not drained")
        economic_row=economic.get(sql_hash)
        if economic_row is None:gaps.append(f"H{height}: economic acceptance/consumption missing");continue
        if any(economic_row.get(k) is None for k in ("frozen_job","validated_ledger","consumption")):
            gaps.append(f"H{height}: incomplete immutable economic record");continue
        parse_block(economic_row)
        if tip["height"]+1>=height+depth:
            if economic_row["lifecycle"]!="booked":gaps.append(f"H{height}: mature ledger still processing")
            actual_rows=[r for r in tables["ledger_history"] if r["block_hash"]==sql_hash]
            actual={r["sp_id"]:r for r in actual_rows}
            expected={"\\x"+bytes(r["sp_id"]).hex():r for r in economic_row["validated_ledger"]}
            if len(actual)!=len(actual_rows) or set(actual)!=set(expected) or any(any(actual[i][k]!=r[k] for k in ("paid_sats","ark_due_sats","forfeited_sats","pending_delta")) or actual[i]["rail"]!=r["rail"].lower() for i,r in expected.items() if i in actual):
                gaps.append(f"H{height}: exact ledger rows not drained")
            if sql_hash[2:] not in applied:gaps.append(f"H{height}: pending-applied publication not drained")
            if sql_hash in found and (found[sql_hash]["status"]!="confirmed" or found[sql_hash]["ledger_published"] is not True):
                gaps.append(f"H{height}: ledger publication still processing")
        elif economic_row["lifecycle"]!="unbooked":
            gaps.append(f"H{height}: shallow canonical liability not retained unbooked")
    return gaps


def drain(stop_mining,sample,stop_pools,final_sample,seal,*,before_pool_stop=None,timeout=60,stable_samples=3,interval=0.2,clock=time.monotonic,sleep=time.sleep):
    require(stable_samples>=3,"final drain needs at least three stable samples")
    deadline=clock()+timeout;stop_mining();previous=None;stable=0;last=["no sample"]
    while clock()<deadline:
        current=sample();last=current["gaps"]
        if clock()>=deadline:break
        signature=current["stable_key"]
        stable=stable+1 if not last and signature==previous else (1 if not last else 0)
        previous=signature
        if stable>=stable_samples:
            if before_pool_stop is not None:before_pool_stop(current)
            if clock()>=deadline:raise Inconclusive("final drain deadline expired before pool stop")
            stop_pools()
            final=final_sample()
            if clock()>=deadline:raise Inconclusive("final drain deadline expired after pool EOF")
            if final["gaps"] or final["stable_key"]!=signature:
                raise Inconclusive("post-EOF raw/source/SQL boundary changed: "+repr(final["gaps"]))
            seal(final)
            return final
        sleep(interval)
    raise Inconclusive("bounded final drain did not converge: "+repr(last))
