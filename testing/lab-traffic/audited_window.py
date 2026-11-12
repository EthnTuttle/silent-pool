"""Independent complete eligible-history proof for source-feed/2 warmup.

Supports the declared single-front-per-pool topology with a captured fresh
bootstrap. Restored/unmapped/pruned history is unavailable, never empty.
"""
from dataclasses import dataclass
import hashlib

from case_contract import canonical_json
from oracle import Inconclusive, integer, raw_payouts, require, strict_json, target
from pool_source import decimal_uint
from ruleb_sql import replay_checkpoint, sql_bytes
from source_feed_codec import READ_COMPATIBLE_CONTRACTS, window_facts
from source_jobs import MissingComponent, parent_view, verify_job

_COMPLETE=object()


@dataclass(frozen=True)
class EligibleHistory:
    job_key: tuple
    source_log_sha256: str
    snapshot_sha256: str
    selected_ids: tuple
    eligible_ids: tuple
    _authority: object

    def __post_init__(self):
        require(self._authority is _COMPLETE,"eligible history requires independent inventory proof")


def prove_eligible(source,key,snapshot,views,policy,profile,migrations):
    require(source.source_contract in READ_COMPATIBLE_CONTRACTS and source.pool_pubkey is not None,"warmup requires the pinned signed source contract")
    p=source.jobs[key]["payload"];seq=decimal_uint(source.jobs[key]["source"]["seq"])
    reference=bytes.fromhex(profile["reference_target"])
    require(len(reference)==32 and p["work_unit_version"]==2
            and hashlib.sha256(b"spc-lab/work-unit/v2\0"+reference).hexdigest()==p["work_unit_fingerprint"],"audited window work-profile fingerprint")
    threshold=int.from_bytes(bytes.fromhex(p["share_target"]),"big")
    quotient,remainder=divmod(int.from_bytes(reference,"big")+1,threshold+1)
    require(remainder==0 and 1<=quotient<=2**32 and quotient&(quotient-1)==0
            and type(p["m_issue"]) is int and p["m_issue"]==quotient,"audited current job issued-work family")
    current=verify_job(source,key,views,policy)
    parent=parent_view(p,views)
    replay_checkpoint(snapshot,p.get("economic"),parent,source_contract=source.source_contract,migration_checksums=migrations)
    starts=[e for e in source.events if e["kind"]=="bootstrap_applied" and e["source"]["session"]==key[0]
            and e["payload"].get("front_id")==key[1] and decimal_uint(e["source"]["seq"])<seq]
    if len(starts)!=1 or starts[0]["payload"].get("restored_slice_ids")!=[]:
        raise MissingComponent("independent_restored_or_missing_eligible_history_checkpoint")
    if set(source.heads)!={key[0]}:
        raise MissingComponent("cross_session_eligible_history_checkpoint")
    finding=decimal_uint(p["finding_slice_id"])
    require(finding>0 and finding>>56==key[1] and p["finding_slice_excluded"] is True,"warmup open/finding slice exclusion")
    if set(p["high_water"])!={str(key[1])}:
        raise MissingComponent("all_front_high_water_and_visibility_mapping")
    if any(e["kind"]=="proof_unavailable" and e["source"]["session"]==key[0]
           and decimal_uint(e["source"]["seq"])<=seq for e in source.events):
        raise MissingComponent("eligible_source_prefix_reports_missing_evidence")
    publications={sid:values[0][1] for sid,values in source.slice_publications.items()}
    # SQL is a complete same-snapshot census, not the selection authority. Every
    # stored slice must have actual source bytes; RAM-selected rows may not yet
    # have reached SQL, but still require captured publication/share bytes.
    for row in snapshot["tables"]["slice"]:
        sid=integer(row["slice_id"])
        if sid not in publications:raise MissingComponent("SQL_slice_has_no_captured_source_bytes")
        decoded=publications[sid]
        require(row["height"]==decoded["height"] and sql_bytes(row["root"],32).hex()==decoded["root"]
                and integer(row["difficulty"],0,(1<<128)-1)==decoded["difficulty"]
                and canonical_json(strict_json(sql_bytes(row["shares"])))==canonical_json(decoded["shares"]),"SQL/source eligible slice mismatch")
        credits=[r for r in snapshot["tables"]["slice_credit"] if r["slice_id"]==sid]
        actual={sql_bytes(r["sp_id"],32).hex():(integer(r["diff_sum"],0,(1<<128)-1),integer(r["diff_fee_sum"],0,(1<<128)-1)) for r in credits}
        require(len(actual)==len(credits) and actual==decoded["credits"],"SQL/source eligible credit census")
    sql_ids={r["slice_id"] for r in snapshot["tables"]["slice"]}
    require(all(r["slice_id"] in sql_ids for r in snapshot["tables"]["slice_credit"]),"SQL orphan slice credit")
    candidates=set()
    for sid,accounts in source.accounts.items():
        before=[e for e in accounts if e["source"]["session"]==key[0] and decimal_uint(e["source"]["seq"])<seq]
        if not before or sid==finding:continue
        if sid not in publications:raise MissingComponent("completed_eligible_slice_publication_missing")
        require(len(before)==len(accounts),"closed eligible slice gained post-freeze shares")
        candidates.add(sid)
    for sid,values in source.slice_publications.items():
        if sid==finding or sid in candidates:continue
        prior_publication=any(e["source"]["session"]==key[0] and decimal_uint(e["source"]["seq"])<seq for e,_ in values)
        accounts=source.accounts.get(sid,[])
        if prior_publication or not accounts or sid<finding or any(e["source"]["session"]!=key[0] or decimal_uint(e["source"]["seq"])<seq for e in accounts):
            raise MissingComponent("published_eligible_slice_missing_accounted_prefix_or_visibility")
    eligible=[];checked_jobs={}
    for sid in sorted(candidates,reverse=True):
        row=publications[sid]
        if row["front_id"]!=key[1]:raise MissingComponent("all_front_eligible_source_mapping")
        require(row["work_unit_version"]==p["work_unit_version"] and row["work_unit_fingerprint"]==p["work_unit_fingerprint"],"mixed eligible work profile")
        for account in source.accounts[sid]:
            job_key=source.job_key(account)
            if job_key not in source.jobs:raise MissingComponent("eligible_slice_immutable_job_missing")
            if job_key not in checked_jobs:checked_jobs[job_key]=verify_job(source,job_key,views,policy)
            require(checked_jobs[job_key]["job"]["height"]==row["height"]
                    and source.jobs[job_key]["payload"]["work_unit_fingerprint"]==row["work_unit_fingerprint"],
                    "eligible slice height/profile differs from actual source job")
        if row["height"]>p["height"]:raise MissingComponent("eligible_slice_from_future_job")
        if row["height"]>=max(0,p["height"]-4096):eligible.append(sid)
    q=(profile["window_n"]*(int(profile["reference_target"],16)+1)+target(p["nbits"]))//(target(p["nbits"])+1)
    require(q==decimal_uint(p["window_target"],128),"eligible inventory contextual Q mismatch")
    selected=[];work=0
    for sid in eligible:
        if work>=q:break
        selected.append(sid);work+=publications[sid]["difficulty"]
    declared=[decimal_uint(r["slice_id"]) for r in p["slices"]]
    if declared!=selected:raise MissingComponent("eligible_completed_slice_omitted_or_pruned")
    mark=str(selected[0]) if selected else "0"
    require(p["high_water"]=={str(key[1]):mark},"complete eligible high-water vector mismatch")
    proof=EligibleHistory(key,source.log_sha256,hashlib.sha256(canonical_json(snapshot)).hexdigest(),tuple(selected),tuple(eligible),_COMPLETE)
    prevouts={tx["txid"]:tx["raw"].hex() for v in views.values() for _,tx in v.transactions.values()}
    window=source.verified_window(key,reference_target=int(profile["reference_target"],16),window_n=profile["window_n"],
                                  prevout_transactions=prevouts,eligible_history=proof)
    window["window"]={"facts":window_facts(q,window["window_work"]),"eligible_history_complete":True}
    window["eligible_inventory_snapshot_sha256"]=proof.snapshot_sha256
    if not selected:
        outputs=current["coinbase"]["outputs"];revenue=int(p["subsidy"])+current["fees"]
        # Zero fresh rewards does not erase old available liabilities. Actual
        # prior/owned outputs and row conservation are checked by _settlement
        # before a bundle can be prepared. There is no gross pool=T UTXO floor.
        require(sum(o["value"] for o in outputs)==revenue and len(outputs[0]["script"])==34
                and outputs[0]["script"][:2]==b"\x51\x20","empty-window actual pool routing/revenue")
    return window


def rewards(window,subsidy,fees,fee_bps):
    """No caller boolean can turn missing history into the empty adapter."""
    if window["selected"]:return raw_payouts(window["selected"],subsidy,fees,fee_bps)
    if not isinstance(window.get("eligible_history"),EligibleHistory):
        raise MissingComponent("complete_empty_window_eligible_history")
    return {},integer(subsidy)+integer(fees)
