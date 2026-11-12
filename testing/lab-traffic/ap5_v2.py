"""Independent consumer of the source-bound AP5 v2 archive/component formats.

Raw Bitcoin/policy/value checks run now. Full VTXO cryptography and same-run
canonical ancestry need their actual admitted evidence; decoding never passes
those gates. No producer Python verifier is imported or executed.
"""
import base64
from collections import Counter
import hashlib
import json
from pathlib import Path
import re

from case_contract import canonical_json, verdict
from oracle import Inconclusive, Policy, address_script, decode_block, decode_transaction, integer, require, strict_json, unhex

CONTRACT_SHA256="9cb6915a3ff8a0500e42c0846ccf3c0642fd4061c66c0701fd36e660cd85beda"
RETAINED_CONTRACT_SHA256="d5040b82414d4b5d97b36fcb93f1373e5396314b428cf6e7684baed40d22a295"
RETAINED_CONTRACT_PREDECESSOR="88ff7b242189e1aa157a873e29084803b47cb639a7c70d850792d840835760fe"
VERIFIER_LOCK="ac24ee3b09c7c2dd8731417672b9d7b072aa6e9e04127b9c96e8ad80ae842833"
USERS=("alice","bob","carol","dave")
ROLES={"node":2,"front":2,"wallet":4,"miner":4,"pool-ark":2,"ark":1,"ark-postgres":1,"postgres":2,"redis":2,"tx_driver":2}
LISTENERS={"ark.local-ark","ark.local-ark.admin","ark.local-ark.postgres"}
LISTENERS|={f"node.node-{n}.{k}" for n in ("a","b") for k in ("p2p","rpc","tdp")}
LISTENERS|={f"front.front-{n}1.{k}" for n in ("a","b") for k in ("api","jd","operator","sv2")}
LISTENERS|={f"wallet.{u}.status" for u in USERS}


def digest(raw):return hashlib.sha256(raw).hexdigest()


def read(path,maximum=64*1024*1024):
    path=Path(path)
    require(not path.is_symlink() and path.stat().st_size<=maximum,"AP5 evidence file type/bound")
    return path.read_bytes()


def artifact(ref):
    raw=read(ref["path"])
    require(digest(raw)==ref["sha256"],"AP5 referenced artifact hash changed")
    return raw


def inspect_verifier_handoff(value):
    if value["schema"]=="spc.lab.ap5.verifier-and-live-handoff/v1":
        from ap5_verifier import CompiledVerifier, ARTIFACT_SHA256
        require(value["contract"]["sha256"]==CONTRACT_SHA256 and digest(artifact(value["contract"]))==CONTRACT_SHA256
                and value["cargo_lock_sha256"]==VERIFIER_LOCK and value["verifier"]["sha256"]==ARTIFACT_SHA256
                and value["unique_crypto_cases"]==6 and value["all_tests_unconditional"] is True,"AP5 compiled contract/lock/test binding")
        verifier=CompiledVerifier(strict_json(artifact(value["verifier"])))
        return {**verifier.admission,"rust_crypto_tests_executed":True,"live_corpus_required_to_build":False,
                "first_actual_board_proof_required_for_funded_acceptance":True,"missing_components":["first_actual_funded_retained_board_proof"]}
    require(value["schema"]=="spc.lab.ap5.handoff/v2" and value["contract"]["sha256"]==RETAINED_CONTRACT_SHA256
            and value["contract_predecessor"]["sha256"]==RETAINED_CONTRACT_PREDECESSOR
            and value["verifier_lock"]["sha256"]==VERIFIER_LOCK,"AP5 verifier source/contract/lock lineage")
    predecessor=strict_json(artifact(value["preserved_predecessor"]))
    require(predecessor["schema"]=="spc.lab.ap5.handoff/v2"
             and predecessor["contract"]["sha256"]==RETAINED_CONTRACT_PREDECESSOR
            and predecessor["verifier_lock"]["sha256"]==VERIFIER_LOCK,"AP5 preserved verifier lineage")
    require(value["requires_live_corpus_for_compilation"] is False
            and predecessor["requires_live_corpus_for_compilation"] is False
            and predecessor["requires_live_corpus_for_conformance_tests"] is False
            and predecessor["fixture_kind"]=="synthetic_signed_conformance","AP5 conformance/live proof classification")
    # Preserve the historical source-only handoff as history. Current compiled
    # readiness is admitted separately above, not by rewriting an old flag.
    return {"status":"INCONCLUSIVE","conformance_fixture_kind":predecessor["fixture_kind"],
            "prepared_crypto_tests":value["rust_crypto_tests_prepared"],
            "rust_crypto_tests_executed":value["rust_crypto_tests_executed"],
            "live_corpus_required_to_build":False,"first_actual_board_proof_required_for_funded_acceptance":True,
            "missing_components":["executable_artifact_not_bound_by_this_historical_handoff"]}


def joined_profile_checks(validation,bundle):
    require(validation["schema"]=="spc.lab.ap5.join-validation/v1"
            and validation["source_manifest_sha256"]==bundle["joined_source_manifest"]["sha256"],"AP5 joined validation source")
    purposes={"agent-ark-tests","agent-ark-build","pool-ark-tests","pool-ark-build","ui-build","ui-browser-tests"}
    checks=validation["checks"];selected=[c for c in checks if c.get("purpose") in purposes]
    require(len(selected)==6 and {c["purpose"] for c in selected}==purposes,"AP5 explicit merged feature/UI check census")
    for check in selected:
        require(type(check["exit"]) is int and check["exit"]==0
                and check["source_manifest_sha256"]==validation["source_manifest_sha256"],"AP5 joined check failure/source mismatch")
        log=artifact(check["log"]).decode()
        argv=check["argv"]
        require(isinstance(argv,list) and argv and all(isinstance(a,str) for a in argv),"AP5 actual check argv")
        purpose=check["purpose"]
        if purpose.startswith(("agent-","pool-")):
            package="spc-agent" if purpose.startswith("agent-") else "spc-pool"
            package_ok=any(argv[i] in {"-p","--package"} and argv[i+1]==package for i in range(len(argv)-1)) or "--package="+package in argv
            features=[]
            for i,arg in enumerate(argv):
                if arg=="--features" and i+1<len(argv):features.extend(argv[i+1].replace(","," ").split())
                elif arg.startswith("--features="):features.extend(arg.split("=",1)[1].replace(","," ").split())
            require(package_ok and ("ark" in features or package+"/ark" in features),"AP5 default-feature check cannot cover Ark")
            if purpose.endswith("tests"):
                require("test" in argv and not {"--no-run","--list"}.intersection(argv)
                        and "test result: ok." in log and re.search(r"test .*?(?:ark|wallet).*?\.\.\. ok",log,re.I),"AP5 Ark tests were not executed")
            else:
                require("build" in argv and check["binary_sha256"]==bundle["binaries"]["agent" if package=="spc-agent" else "pool"]["sha256"],"AP5 Ark build/binary binding")
        elif purpose=="ui-build":
            package_ok=any(argv[i] in {"-p","--package"} and argv[i+1]=="spc-lab-cli" for i in range(len(argv)-1)) or "--package=spc-lab-cli" in argv
            require("build" in argv and package_ok and check["binary_sha256"]==bundle["binaries"]["lab"]["sha256"],"AP5 compiled UI build binding")
        else:
            require(check["tested_binary_sha256"]==bundle["binaries"]["lab"]["sha256"]
                    and check["required_cases"] and check["passed_cases"]==check["required_cases"],"AP5 actual compiled browser check incomplete")
    return {"status":"PASSED_CHECK_RECEIPTS_ONLY","actual_Nix_source_admission_required":True}


def archive(seal_path,seal_hash):
    raw=read(seal_path,1024*1024)
    require(digest(raw)==seal_hash,"AP5 v2 external seal mismatch")
    seal=strict_json(raw)
    require(set(seal)=={"root","events_sha256","last_seq","last_digest"},"AP5 v2 seal shape")
    root=Path(seal["root"])
    require(root.is_absolute() and root.resolve()==root,"AP5 root confinement")
    raw=read(root/"events.jsonl",32*1024*1024)
    if not raw.endswith(b"\n"):raise Inconclusive("AP5 event tail truncated")
    require(digest(raw)==seal["events_sha256"],"AP5 event stream differs from external seal")
    previous="00"*32;records=[]
    for seq,line in enumerate(raw.splitlines(),1):
        row=strict_json(line)
        require(set(row)=={"seq","file","sha256","previous","unix_ms","record_sha256"},"AP5 event shape")
        require(row["seq"]==seq and row["previous"]==previous,"AP5 event sequence/link")
        previous=digest(canonical_json({k:v for k,v in row.items() if k!="record_sha256"}))
        require(previous==row["record_sha256"],"AP5 event hash")
        match=re.fullmatch(r"([0-9]{6})-([a-z0-9-]+)\.json",row["file"])
        require(match is not None and int(match[1])==seq,"AP5 event path/sequence")
        body=read(root/row["file"]);require(digest(body)==row["sha256"],"AP5 event body hash")
        records.append((match[2],strict_json(body),row))
    require(len(records)==seal["last_seq"] and previous==seal["last_digest"],"AP5 terminal seal")
    require(records and records[-1][0]=="result" and records[-1][1]["schema"]=="spc.lab.ap5.result/v2","AP5 v2 terminal result")
    return root,records


def checked_height(block,claimed,policy):
    claimed=integer(claimed)
    if block["hash"]==policy.genesis:
        require(claimed==0,"AP5 genesis height mismatch");return claimed
    script=block["transactions"][0]["inputs"][0]["script"]
    if 0x51<=script[0]<=0x60:height=script[0]-0x50
    else:
        size=script[0]
        require(1<=size<=5 and len(script)>size and not script[size]&128,"AP5 raw BIP34 height encoding")
        height=int.from_bytes(script[1:1+size],"little")
    require(height==claimed and height>0,"AP5 claimed height differs from raw coinbase")
    return height


def raw_transaction(views,policy):
    require(isinstance(views,list) and len(views)==2,"AP5 requires exactly two node transaction observations")
    results=[]
    for record in views:
        tx=decode_transaction(unhex(record["raw"]))
        require(not tx["coinbase"] and tx["txid"]==record["txid"],"AP5 raw txid/type")
        block=decode_block(unhex(record["raw_block"]),policy)
        require(block["hash"]==record["block_hash"] and any(t["raw"]==tx["raw"] for t in block["transactions"]),"AP5 transaction raw block inclusion")
        height=checked_height(block,record["height"],policy)
        inputs=record["inputs"]
        require(isinstance(inputs,list) and len(inputs)==len(tx["inputs"]),"AP5 input proof census")
        by_point={(p["txid"],integer(p["vout"],0,0xffffffff)):p for p in inputs}
        require(len(by_point)==len(inputs),"AP5 duplicate input proof")
        amount=0
        for inp in tx["inputs"]:
            txid,index=inp["outpoint"].split(":");index=int(index)
            if (txid,index) not in by_point:raise Inconclusive("AP5 raw prevout proof missing")
            p=by_point[(txid,index)];parent=decode_transaction(unhex(p["raw"]))
            parent_block=decode_block(unhex(p["raw_block"]),policy)
            require(parent["txid"]==txid and index<len(parent["outputs"]) and parent_block["hash"]==p["block_hash"]
                    and any(t["raw"]==parent["raw"] for t in parent_block["transactions"]),"AP5 prevout inclusion bytes")
            ph=checked_height(parent_block,p["height"],policy);require(ph<=height,"AP5 future prevout height")
            if parent["coinbase"]:require(ph>0 and height-ph>=policy.coinbase_maturity,"AP5 immature/genesis coinbase")
            value=parent["outputs"][index]["value"]
            require(value==integer(p["amount_sats"]),"AP5 claimed prevout value differs from bytes")
            amount+=value
        fee=amount-sum(o["value"] for o in tx["outputs"])
        require(fee>=0 and fee==integer(record["fee_sats"]),"AP5 raw fee mismatch")
        results.append({"txid":tx["txid"],"wtxid":tx["wtxid"],"fee_sats":fee,"vsize":tx["vbytes"],"height":height,"block_hash":block["hash"]})
    require(results[0]==results[1] and views[0]["raw"]==views[1]["raw"],"AP5 two-node transaction disagreement")
    return results[0]


def ark_policy(value):
    raw=strict_json(unhex(value["raw_response_hex"]))
    require(canonical_json(raw)==canonical_json(value["ark_info"]) and raw["network"]=="regtest","AP5 actual ArkInfo raw/decoded binding")
    for wire,bound in (("serverPubkey","server_pubkey"),("mailboxPubkey","mailbox_pubkey")):
        key=base64.b64decode(raw[wire],validate=True)
        require(key.hex()==value["bound"][bound],"AP5 full server/mailbox key binding")
    require(digest(canonical_json(raw["fees"]))==value["bound"]["fees_sha256"],"AP5 fee schedule hash")
    return raw


def stock_bytes(identifier):
    tx,index=identifier.split(":")
    return (unhex(tx,32)[::-1]+integer(int(index),0,0xffffffff).to_bytes(4,"little")).hex()


def proto_uint(value):
    require(isinstance(value,str) and value.isascii() and value.isdecimal() and str(int(value))==value,"ArkInfo uint64 encoding")
    return integer(int(value))


def replay_component(data,policy,verifier=None):
    if verifier is not None:
        from ap5_verifier import CompiledVerifier
        require(type(verifier) is CompiledVerifier,"AP5 requires the admitted compiled verifier, not a supplied verifier function")
    require(data["schema"]=="spc.lab.ap5.raw-component/v2","AP5 raw component schema")
    name=data["id"]
    require(integer(data["maturity"])==policy.coinbase_maturity,"AP5 component cannot choose its own maturity")
    info=ark_policy(data["ark_info"])
    txs=[raw_transaction(v,policy) for v in data.get("transactions",[])]
    public=[]
    for proof in data.get("vtxos",[]):
        raw=unhex(proof["raw_hex"])
        require(raw and len(raw)<=4*1024*1024,"AP5 full VTXO proof byte bound")
        anchor=raw_transaction(proof["anchor_views"],policy);expected=proof["expected"]
        require(expected["server_pubkey"]==data["ark_info"]["bound"]["server_pubkey"]
                and expected["exit_delta"]==info["vtxoExitDelta"]
                and expected["max_exit_depth"]<=info["maxVtxoExitDepth"],"AP5 VTXO expected policy/server mismatch")
        for k in ("amount_sats","expiry_height","exit_delta","max_exit_depth"):integer(expected[k])
        crypto=verifier.verify(proof["raw_hex"],proof["anchor_views"][0]["raw"],expected) if verifier is not None else None
        if crypto is not None:require(crypto["chain_anchor"].split(":")[0]==anchor["txid"],"AP5 compiled proof/raw Bitcoin anchor identity")
        public.append({"raw_sha256":digest(raw),"expected":expected,"anchor":anchor,"crypto":crypto})
    require(len({p["raw_sha256"] for p in public})==len(public),"AP5 duplicate full VTXO proof")
    cryptos=[p["crypto"] for p in public if p["crypto"] is not None]
    require(len({p["id"] for p in cryptos})==len(cryptos),"AP5 duplicate verified VTXO identity")
    by_id={p["id"]:p for p in cryptos};by_bytes={stock_bytes(p["id"]):p for p in cryptos}
    dependencies=[];facts={"transactions":txs,"full_vtxo_proofs_retained":len(public),"full_vtxo_crypto_verified":len(cryptos),"verified_vtxos":cryptos}
    if name.startswith("fund-"):
        allowed={"fund-server-alice":200000000,"fund-server-bob":200000000,"fund-pool-a":120000000,"fund-pool-b":120000000}
        require(name in allowed and len(data["payments"])==len(txs)==1,"AP5 funding census")
        payment=data["payments"][0];require(payment["sats"]==allowed[name] and payment["txid"]==txs[0]["txid"],"AP5 declared funding amount/id")
        tx=decode_transaction(unhex(data["transactions"][0][0]["raw"]))
        require(any(o["script"].hex()==payment["script"] and o["value"]==payment["sats"] for o in tx["outputs"]),"AP5 actual funding script/value")
    elif name.startswith("board-"):
        require(len(txs)==len(public)==1,"AP5 original board raw proof census")
        from ap5_retained_board import bind
        original=bind(data["proof_read_request"],data["proof_read_result"],data["board_request"],data["confirmed_receipt"])
        require(data["board"]==original["board"],"AP5 component board differs from retained original")
        require(data["vtxos"][0]["raw_hex"]==original["original_full_vtxo_hex"]
                and all(v["raw"]==original["raw_anchor_tx_hex"] for v in data["transactions"][0])
                and all(v["raw"]==original["raw_anchor_tx_hex"] for v in data["vtxos"][0]["anchor_views"]),
                "AP5 component substitutes another original proof or raw anchor")
        expected=data["vtxos"][0]["expected"]
        require(expected["user_pubkey"]==original["owner_pubkey"] and expected["server_pubkey"]==original["server_pubkey"]
                and expected["amount_sats"]==original["board"]["net_board_sats"]
                and expected["expiry_height"]==original["board"]["expiry_height"]
                and expected["exit_delta"]==original["board"]["vtxo_exit_delta"],"AP5 original full owner/policy/server expectation changed")
        require(original["checked_tip_height"]>=txs[0]["height"]+original["funding_confirmations"]-1,
                "retained confirmation claim exceeds actual raw anchor height")
        board=data["board"];fees=info["fees"]["board"]
        fee=max(proto_uint(fees["minFeeSat"]),proto_uint(fees["baseFeeSat"])+(integer(board["gross_board_sats"])*proto_uint(fees["ppm"])+999999)//1000000)
        require(fee==board["board_fee_sats"] and board["onchain_fee_sats"]==txs[0]["fee_sats"]
                and board["actual_fee_sats"]==fee+txs[0]["fee_sats"]
                and board["gross_board_sats"]-fee==board["net_board_sats"]==public[0]["expected"]["amount_sats"],"AP5 original board fees/net target")
        require(board["required_confirmations"]==info["requiredBoardConfirmations"]
                and board["min_board_sats"]==proto_uint(info["minBoardAmount"])
                and board["server_pubkey"]==data["ark_info"]["bound"]["server_pubkey"],"AP5 board actual policy")
        facts["historical_board_need_not_be_currently_spendable"]=True
        facts["retained_proof_sha256"]=data["proof_read_result"]["proof"]["proof_sha256"]
        facts["original_vtxo_id"]=original["original_vtxo_id"]
        facts["original_chain_anchor"]=f"{original['anchor_txid']}:{original['anchor_vout']}"
        if cryptos:
            require(cryptos[0]["id"]==original["original_vtxo_id"]
                    and cryptos[0]["chain_anchor"]==facts["original_chain_anchor"],"AP5 retained original VTXO/full anchor differs from compiled proof")
        facts["full_VTXO_crypto_still_required"]=not bool(cryptos)
    elif name.startswith("float-"):
        require(data["target_sats"]==115000000 and data["already_at_target"] is True,"AP5 exact float target")
        req,res=data["board_request"],data["board_result"]
        require(req["operation_id"]==res["operation_id"] and req["action"]=="board_float" and req["net_target_sats"]==115000000,"AP5 float request correlation")
        require(res["state"] in {"confirmed","already_at_target"},"AP5 unresolved/no-effect board is not a float proof")
        if res["state"]=="already_at_target":require(res["funding_txid"] is None and res["board"] is None,"AP5 noop invented a board transaction")
        else:dependencies.append("board-"+name.removeprefix("float-"))
        snapshot=data["fresh_snapshot"]
        require(snapshot["state"]=="observed" and integer(snapshot["float_spendable_sats"])>=115000000
                and snapshot["operation_id"]!=req["operation_id"],"AP5 fresh float observation")
        rows=data["float_rows"]
        require(len(rows)==len(public)>0 and len({r["id"] for r in rows})==len(rows),"AP5 float row/proof census")
        require(all(r["state"]=="Spendable" and integer(r["expiry_height"])>=integer(snapshot["observed_tip_height"])+64 for r in rows),"AP5 actual float expiry horizon")
        require(sum(integer(r["amount_sats"]) for r in rows)>=115000000,"AP5 float balance not backed by rows")
        if cryptos:
            require({r["id"] for r in rows}==set(by_id),"AP5 float row/compiled proof identity mismatch")
            for r in rows:
                v=by_id[r["id"]]
                require(r["amount_sats"]==v["amount_sats"] and r["expiry_height"]==v["expiry_height"]
                        and v["expiry_height"]>=snapshot["observed_tip_height"]+64,"AP5 float row differs from full proof")
            require(sum(v["amount_sats"] for v in cryptos)>=115000000,"AP5 float proofs below target")
        facts["unverified_float_rows"]=0 if cryptos else len(rows)
    elif name.startswith("receive-"):
        require(public and data.get("payout_attribution") and all(r["status"]=="sent" for r in data["payout_attribution"]),"AP5 receive requires raw proofs and mining attribution")
        if cryptos:
            ids=[unhex(r["vtxo_id"].removeprefix("\\x"),36).hex() for r in data["payout_attribution"]]
            require(len(ids)==len(set(ids)) and set(ids)==set(by_bytes),"AP5 payout attribution differs from compiled VTXO identities")
    elif name.startswith("sp-refresh-"):
        r=data["round"];cp=strict_json(r["checkpoint_raw"])
        require(canonical_json(cp)==canonical_json(r["checkpoint"]) and cp["version"]==2 and cp["locked"] and cp["released"],"AP5 signed refresh checkpoint")
        preimage=[cp[k] for k in ("version","input_ids","outputs","unlock_hash","funding","locked")]
        # Keep nested serde insertion order; sorting this preimage is wrong.
        raw=json.dumps(preimage,separators=(",",":"),ensure_ascii=False).encode()
        require(list(hashlib.sha256(raw).digest())==cp["proof_binding"]
                and [bytes(i).hex() for i in cp["input_ids"]]==r["inputs"]
                and decode_transaction(bytes(cp["funding"]))["txid"]==r["funding_txid"],"AP5 refresh input/funding/proof binding")
        require(len(cp["released"])==len(cp["locked"])==len(cp["outputs"])==len(r["outputs"]),"AP5 refresh output census")
        require({bytes(v).hex() for v in cp["released"]} <= {v["raw_hex"] for v in data["vtxos"]},"AP5 released output proofs not retained")
        if cryptos:
            require(r["inputs"] and r["outputs"] and len(set(r["inputs"]))==len(r["inputs"]) and len(set(r["outputs"]))==len(r["outputs"])
                    and set(r["inputs"])<=set(by_bytes) and set(r["outputs"])<=set(by_id),"AP5 round input/output compiled-proof census")
            before=sum(by_bytes[i]["amount_sats"] for i in r["inputs"]);after=sum(by_id[i]["amount_sats"] for i in r["outputs"])
            require(0<after<=before and r["value"]=={"input_sats":before,"output_sats":after,"user_paid_fee_sats":before-after}
                    and all(by_id[i]["chain_anchor"].split(":")[0]==r["funding_txid"] for i in r["outputs"]),"AP5 round proof value/anchor mismatch")
            output_raw={proof["raw_hex"] for proof,item in zip(data["vtxos"],cryptos) if item["id"] in r["outputs"]}
            require(output_raw=={bytes(v).hex() for v in cp["released"]},"AP5 released checkpoint is not the verified child proof set")
    elif name.startswith("second-renewal-"):
        r=data["renewal"];require(r["automatic"] is True and r["operations"],"AP5 automatic renewal evidence")
        consumed=set();rounds=set();operation_ids=set()
        for op in r["operations"]:
            require(op["id"] not in operation_ids,"AP5 duplicate renewal operation");operation_ids.add(op["id"])
            payload=strict_json(op["payload"])
            require(payload["action"]=={"kind":"refresh"} and len(payload["vtxo_ids"])==1,"AP5 auto renewal operation shape")
            raw_id=unhex(payload["vtxo_ids"][0],36);height=integer(op["verified_auto_trigger_height"],0,0xffffffff)
            require(digest(b"SPc/StockAutoRefresh/v1"+raw_id+height.to_bytes(4,"big"))==op["id"]==payload["operation_id"]
                    and op["id"] not in r["manual_operation_ids"],"AP5 renewal is not bound to automatic selector")
            require(op["stage"]=="stock_submitted" and op["result"].startswith("round_submitted:"),"AP5 renewal result")
            rounds.add(op["result"].split(":",1)[1]);consumed.update(payload["vtxo_ids"])
        require(consumed==set(r["inputs"]) and rounds==set(r["round_ids"]) and r["outputs"],"AP5 renewal input/round census")
        if cryptos:
            require(consumed<=set(by_bytes) and len(set(r["outputs"]))==len(r["outputs"]) and set(r["outputs"])<=set(by_id)
                    and integer(r["previous_expiry"],0,0xffffffff)==min(by_bytes[i]["expiry_height"] for i in consumed)
                    and all(by_id[i]["expiry_height"]>r["previous_expiry"] and by_id[i]["chain_anchor"].split(":")[0] in rounds for i in r["outputs"]),
                    "AP5 automatic renewal proof identity/expiry/anchor mismatch")
    elif name.startswith(("stock-offboard-","sp-offboard-")):
        require(len(txs)==1 and public,"AP5 offboard raw input/transaction census")
        o=data["offboard"];op=o["operation"];payload=strict_json(op["payload"])
        require(payload["operation_id"]==op["id"] and payload["action"]=={"kind":"offboard","address":o["destination_address"]},"AP5 offboard immutable operation")
        require(address_script(o["destination_address"]).hex()==o["destination_script"],"AP5 offboard allocated script/address")
        tx=decode_transaction(unhex(data["transactions"][0][0]["raw"]))
        require(any(v["script"].hex()==o["destination_script"] and v["value"]>0 for v in tx["outputs"]),"AP5 offboard paid another script")
        require(op["result"]==("offboard_broadcast:" if name.startswith("stock-") else "")+tx["txid"],"AP5 offboard result/raw txid")
        if name.startswith("sp-"):
            cp=strict_json(op["checkpoint"])
            require(op["stage"]=="offboard_confirmed" and bytes(cp["signed"])==tx["raw"],"AP5 direct offboard signed bytes")
        if cryptos:
            require(o["inputs"] and len(set(o["inputs"]))==len(o["inputs"]),"AP5 offboard input census")
            inputs=[]
            for i in o["inputs"]:
                require(i in by_id or i in by_bytes,"AP5 offboard input lacks compiled proof")
                inputs.append(by_id[i] if i in by_id else by_bytes[i])
            require(set(payload["vtxo_ids"])=={stock_bytes(v["id"]) for v in inputs},"AP5 offboard operation/proof input mismatch")
            amount=sum(v["amount_sats"] for v in inputs)
            received=sum(v["value"] for v in tx["outputs"] if v["script"].hex()==o["destination_script"])
            require(0<received<=amount and o["value"]=={**txs[0],"ark_input_sats":amount,"onchain_received_sats":received,
                    "user_paid_fee_sats":amount-received,"destination_script":o["destination_script"]},"AP5 offboard proof value/destination mismatch")
    else:raise Inconclusive("unsupported AP5 component identity")
    missing=["same_AP5_run_raw_canonical_ancestry_and_observation_binding"]
    if public and verifier is None:missing.append("component_bound_admitted_VTXO_verifier_artifact")
    return {"status":"INCONCLUSIVE","id":name,"requires":dependencies,"raw_checks":facts,"missing_components":missing,
            "classification":"archive_observation_not_yet_funded_verified", "synthetic_conformance_cannot_replace_actual_board_proof":True}


def cleanup(receipt,records):
    require(receipt["schema"]=="spc.lab.ap5.cleanup/v2","AP5 cleanup-v2 schema")
    inv=receipt["inventory"];require(digest(canonical_json(inv))==receipt["inventory_sha256"],"AP5 cleanup inventory hash")
    active=inv["active_manifest"];counts=Counter(p["role"] for p in active.get("processes",[]))
    errors=["role:"+role for role,count in ROLES.items() if counts[role]<count]
    resources=active.get("resources",{})
    errors += ["listener:"+key for key in sorted(LISTENERS-set(resources.get("listeners",{})))]
    errors += [key for key in ("postgres_databases","redis_instances") if len(resources.get(key,[]))!=2]
    if not active.get("processes") or any(not p.get("run_token") for p in active.get("processes",[])):errors.append("run_tokens")
    require(inv["coverage_errors"]==errors,"AP5 cleanup coverage misreported")
    ids={(p["pid"],p["start_time_ticks"]) for p in inv["processes"]}
    require(len(ids)==len(inv["processes"]) and all((p["pid"],p["start_time_ticks"]) in ids for p in active.get("processes",[])),"AP5 cleanup omitted/reused actor identity")
    require({p["run_token"] for p in active.get("processes",[])}<=set(inv["run_tokens"]),"AP5 cleanup token census")
    planned={(p["host"],p["port"]) for p in inv["listeners"] if p["kind"]=="planned_tcp_endpoint"}
    endpoints=list(resources.get("listeners",{}).values())
    endpoints += [{"host":"127.0.0.1","port":r["port"]} for g in ("postgres_databases","redis_instances") for r in resources.get(g,[])]
    require(all((e["host"],e["port"]) in planned for e in endpoints),"AP5 cleanup planned listener coverage")
    require(set(inv["observed_socket_inodes"])=={r["inode"] for r in inv["listeners"] if r["kind"]=="owned_socket_inode"}
            and set(inv["observed_unix_paths"])=={r["path"] for r in inv["listeners"] if r["kind"]=="run_unix_socket"},"AP5 cleanup observed socket coverage")
    complete=not(errors or inv["capture_errors"] or inv["unreadable_same_uid_processes"])
    released=complete and not(inv["file_references"] or inv["locks"]) and all(p["status"]=="gone_or_reused" for p in inv["processes"]) and all(r["released"] is True for r in inv["listeners"])
    require(receipt["exhaustive"] is complete and receipt["all_resources_released"] is released,"AP5 cleanup release contradicts observations")
    status="passed" if released else "inconclusive" if not complete else "failed"
    require(receipt["status"]==status,"AP5 cleanup status contradiction")
    if complete:
        census=[v for label,v,_ in records if label=="active-resource-census"]
        require(census==[active] and any(p["role"]=="controller" for p in inv["processes"]),"AP5 separately sealed active/controller census")
        tools=[v for label,v,_ in records if label=="tool-exit"]
        require(len(tools)==sum(label=="tool-intent" for label,_,_ in records)
                and all((t["pid"],t["ticks"]) in ids for t in tools),"AP5 auxiliary tool exit census")
    return {"status":status.upper(),"scope":"captured_OS_inventory_replay_only"}


def inspect(seal_path,seal_hash,common):
    root,records=archive(seal_path,seal_hash);terminal=records[-1][1]
    candidates=[(v,r) for label,v,r in records if label=="ap5-component-verdict"]
    if not candidates:
        return {"status":"INCONCLUSIVE","source_result":terminal["status"],"missing_components":["executed_AP5_v2_component_verdict"],"external_seal_sha256":seal_hash}
    require(len(candidates)==1,"AP5 v2 verdict census")
    producer,row=candidates[0]
    require(terminal["component_verdict"]=={"file":row["file"],"sha256":row["sha256"]}
            and producer["schema"]=="spc.lab.ap5.component-verdict/v2","AP5 result/verdict reference")
    require(producer["whole_lab_complete"] is False and terminal["whole_lab_complete"] is False
            and producer["external_qa_verdict"] is None,"producer may not self-approve external QA")
    bindings=producer["source_bindings"];bundle=strict_json(artifact(bindings["bundle_artifact"]))
    require(bindings["bundle_sha256"]==bindings["bundle_artifact"]["sha256"],"AP5 bundle source hash")
    for k in ("binaries","joined_source_manifest","joined_validation_receipt","vtxo_verifier","pool_wallet_executor"):
        require(bindings.get(k)==bundle.get(k),"AP5 source binding changed: "+k)
    for name in ("lab","txd","pool","miner","scanner","node"):
        require(bindings["binaries"][name]["sha256"]==common["binary_hashes"][name],"AP5/T3 joined binary mismatch")
    joined_profile_checks(strict_json(artifact(bindings["joined_validation_receipt"])),bundle)
    from ap5_verifier import CompiledVerifier
    verifier=CompiledVerifier(bindings["vtxo_verifier"]) if bindings.get("vtxo_verifier") is not None else None
    from ap5_retained_board import PoolEvents
    if bindings["pool_wallet_executor"] is None:raise Inconclusive("AP5 same-wallet executor binding missing")
    pool_events=PoolEvents(records,root,bindings["pool_wallet_executor"])
    raw={ (r["file"],r["sha256"]):v for label,v,r in records if label=="raw-component" }
    components=producer["components"];by_id={c["id"]:c for c in components}
    require(len(by_id)==len(components) and {(c["evidence"]["file"],c["evidence"]["sha256"]) for c in components}==set(raw),"AP5 raw component census mismatch")
    required={"fund-server-alice","fund-server-bob","fund-pool-a","fund-pool-b","float-pool-a","float-pool-b"}
    required|={p+u for u in USERS for p in ("receive-","sp-refresh-","second-renewal-","stock-offboard-","sp-offboard-")}
    checked={};policy=Policy(**common["policy"])
    for c in components:
        require(c["status"] in {"passed","failed","inconclusive"},"AP5 component status")
        if c["status"]!="passed":checked[c["id"]]={"status":c["status"].upper()};continue
        data=raw[(c["evidence"]["file"],c["evidence"]["sha256"])]
        require(data["id"]==c["id"],"AP5 component relabelled")
        pool_events.bind_component(data)
        result=replay_component(data,policy,verifier)
        require(c["requires"]==result["requires"],"AP5 dependency census differs from raw facts")
        required.update(result["requires"]);checked[c["id"]]=result
    clean=[v for label,v,_ in records if label=="cleanup-inventory"]
    require(clean==[producer["cleanup"]],"AP5 cleanup event/verdict mismatch")
    checked["cleanup"]=cleanup(clean[0],records);required.add("cleanup")
    declared_required=(required-{"cleanup"})|{dep for c in components for dep in c.get("requires",[])}
    require(producer["missing"]==sorted(declared_required-set(by_id))
            and producer["failed"]==sorted(c["id"] for c in components if c["status"]=="failed")
            and producer["unknown"]==sorted(c["id"] for c in components if c["status"]=="inconclusive"),"AP5 producer component census contradiction")
    declared=verdict({**{c["id"]:{"status":c["status"].upper()} for c in components},"cleanup":checked["cleanup"]},declared_required|{"cleanup"})
    require(producer["component_status"]==terminal["component_status"]==declared.lower(),"AP5 producer terminal status contradiction")
    status=verdict(checked,required)
    return {"status":status,"components":checked,"missing":sorted(required-set(checked)),"external_seal_sha256":seal_hash,
            "verifier_admission":verifier.admission if verifier is not None else None,"verifier_executions":verifier.executions if verifier is not None else [],
            "source_result":terminal["status"],"missing_components":([] if verifier is not None else ["component_bound_admitted_VTXO_verifier_artifact"])+[
                "same_AP5_run_full_canonical_chain_binding","corrected_tested_aggregate_and_pool_control_source_admission"]}
