"""Full-rail contradictions and exact immutable joins; no funding authority."""
import copy
import hashlib
import unittest

from aggregate_history import order_bytes, grant_bytes, proved_receipt, frozen_members, audit_aggregates
from economic_checkpoint import validate_initial
from oracle import Invalid, Inconclusive, ladder
from payout_history import rail_effects, transaction_effects
from source_feed_codec import CONTRACT, tagged, _sign_digest
from test_economic_checkpoint import captured
from test_ruleb_source import job, consumption_row

ID="33"*32;PUB="79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"


def rail_case(raw=1500000,prior=1999996,front_rail=0,settled="Coinbase",destination=b"",ark=0,forfeit=0):
    owed=raw+prior;pieces,rem=ladder(owed);paid=sum(pieces) if front_rail==0 else 0
    E=owed-paid-ark-forfeit;frozen=job(1,prior)
    keys=[format(i+1,"064x") for i in range(len(pieces))] if front_rail==0 else []
    entry={"sp_id":ID,"rail":front_rail,"p_x":keys,"sats":str(paid if front_rail==0 else raw if front_rail==3 else owed),"owed_sats":str(owed),
           "delivery_state":"registered_derived" if destination else "unregistered","destination_digest":hashlib.sha256(destination).hexdigest() if destination else None}
    p={"economic":frozen,"entries":[entry],"fee_bps":5000}
    block={"hash":"11"*32,"height":1,"subsidy":2*raw,"fees":0,"transactions":[{"outputs":[{"value":2*raw-paid,"script":b"pool"}]+[{"value":v,"script":b"\x51\x20"+bytes.fromhex(k)} for k,v in zip(keys,pieces)]}]}
    wire={"economic":frozen,"block_hash":block["hash"],"entries":[{**entry,"sats":int(entry["sats"]),"owed_sats":owed,"dest_id":destination.hex()}]}
    row={"block_hash":"\\x"+"11"*32,"parent_hash":"\\x"+"07"*32,"height":1,"payload_digest":"\\x"+"55"*32,"raw_event":None,
         "frozen_job":frozen,"validated_ledger":[{"sp_id":[0x33]*32,"rail":settled,"paid_sats":paid,"ark_due_sats":ark,"forfeited_sats":forfeit,"pending_delta":E-prior}],
         "consumption":[consumption_row(prior,E)],"lifecycle":"unbooked","transition":1,"revision":4}
    return p,wire,block,{ID:prior},{"selected":[[{"identity":ID,"work":1,"fees":0}]]},row


class FullRailTests(unittest.TestCase):
    def test_coinbase_all_outputs_and_net_pool_below_gross_K(self):
        result=rail_effects(*rail_case())
        self.assertEqual(result["per_identity"][0]["c"],1500000)
        self.assertLess(result["net_pool"]["net_pool_output"],result["net_pool"]["gross_K"])
        self.assertEqual(result["raw_matched_outputs"],1)
        self.assertFalse(result["owned_output_derivation_verified"])

    def test_full_subbase_Ark_routes_owed_including_prior_but_does_not_prove_delivery(self):
        result=rail_effects(*rail_case(raw=20000,prior=50,front_rail=1,settled="Ark",destination=b"actual-address-bytes",ark=20050))
        self.assertEqual(result["per_identity"][0]["c"],50)
        self.assertIn("settlement_delivery_read_at_reconstruction",{m["field"] for m in result["missing"]})

    def test_registration_lag_pending_is_not_forfeit_or_missing_zero(self):
        args=rail_case(raw=20000,prior=50,front_rail=3,settled="Pending")
        args[0]["entries"][0]["delivery_state"]="registered_underived"
        result=rail_effects(*args)
        self.assertEqual(result["per_identity"][0]["E"],20050)
        self.assertEqual(result["per_identity"][0]["forfeit"],0)

    def test_forfeit_preserves_prior_and_only_takes_fresh_earnings(self):
        result=rail_effects(*rail_case(raw=20000,prior=50,front_rail=3,settled="Forfeit",forfeit=20000))
        self.assertEqual(result["per_identity"][0]["E"],50)
        with self.assertRaisesRegex(Invalid,"impossible"):
            rail_effects(*rail_case(raw=20000,prior=50,front_rail=3,settled="Forfeit",forfeit=20050))

    def test_Ark_without_frozen_destination_and_large_forfeit_reject(self):
        for args in (rail_case(raw=20000,prior=0,front_rail=1,settled="Ark",ark=20000),
                     rail_case(raw=500000,prior=0,front_rail=3,settled="Forfeit",forfeit=500000)):
            with self.assertRaisesRegex(Invalid,"impossible"):rail_effects(*args)

    def test_raw_keys_k_order_extra_outputs_and_rail_cannot_be_substituted(self):
        for mutation in ("key_order","extra","rail","destination"):
            args=rail_case(raw=2000001,prior=1999999);p,w,b,_,_,_=args
            if mutation=="key_order":b["transactions"][0]["outputs"][1]["value"]-=1
            elif mutation=="extra":b["transactions"][0]["outputs"].append({"value":1,"script":b"other"})
            elif mutation=="rail":w["entries"][0]["rail"]=1
            else:w["entries"][0]["dest_id"]=b"foreign".hex()
            with self.subTest(mutation=mutation),self.assertRaises(Invalid):rail_effects(*args)

    def test_wrong_individual_reservation_rejects_even_with_same_pool_total(self):
        args=rail_case();args[-1]["consumption"][0]["consumed"]-=1
        with self.assertRaises(Invalid):rail_effects(*args)

    def test_booked_and_reserved_effects_replay_actual_complete_schema(self):
        o,_,_,m=captured()
        # Synthetic one-transaction bookkeeping, after the separate captured
        # seed transaction. Do not reorder or modify archived SQL evidence.
        for event in o["tables"]["source_accounting_history"][2:]:event["transaction_id"]=3
        seed=validate_initial(o,migration_checksums=m,source_contract=CONTRACT)
        result=transaction_effects(o,seed)
        self.assertTrue(any(p["per_identity"] for p in result))
        self.assertTrue(all(p["A_after"]>=0 for t in result for p in t["per_identity"]))

    def test_interleaved_transactions_do_not_invent_commit_order(self):
        o,_,_,m=captured();seed=validate_initial(o,migration_checksums=m,source_contract=CONTRACT)
        with self.assertRaisesRegex(Inconclusive,"commit_order"):transaction_effects(o,seed)


class AggregateCodecTests(unittest.TestCase):
    def order(self):
        destination=b"stock-address-bytes";payload=b"\1"*16+bytes([len(destination)])+destination+(50).to_bytes(8,"little")+(12).to_bytes(4,"little")
        return payload+bytes.fromhex(_sign_digest(1,tagged("SPc/ArkSendOrder",payload)))

    def grant(self,order=None):
        order=self.order() if order is None else order
        payload=b"SPAG\1"+b"\1"*16+tagged("SPc/FrozenArkOrderV1",order)+b"\2"*16+(3).to_bytes(8,"little")+(110).to_bytes(8,"little")
        return payload+bytes.fromhex(_sign_digest(1,tagged("SPc/ArkAuthorizationV1",payload)))

    def test_source_owned_signed_order_and_grant_join(self):
        order=order_bytes(self.order(),PUB);grant=grant_bytes(self.grant(),PUB)
        self.assertEqual((grant["order_id"],grant["order_digest"]),(order["order_id"],order["digest"]))
        self.assertEqual(grant["expires_at"],110) # No present-time or consumption assertion.

    def test_changed_order_signature_foreign_key_and_trailing_grant_reject(self):
        raw=self.order()
        with self.assertRaises(Invalid):order_bytes(raw[:-1]+bytes([raw[-1]^1]),PUB)
        with self.assertRaises(Invalid):order_bytes(raw,"11"*32)
        with self.assertRaises(Invalid):grant_bytes(self.grant()+b"\0",PUB)

    def test_proved_receipt_retains_every_proof_and_rejects_duplicate_or_truncated(self):
        def receipt(proofs):return b"SPRP\1"+self.grant()+(50).to_bytes(8,"little")+len(proofs).to_bytes(2,"little")+b"".join(len(p).to_bytes(4,"little")+p for p in proofs)
        result=proved_receipt(receipt([b"a",b"b"]),PUB);self.assertEqual(result["proofs"],[b"a",b"b"])
        for raw in (receipt([b"a",b"a"]),receipt([b"a"])[:-1]):
            with self.assertRaises((Invalid,Inconclusive)):proved_receipt(raw,PUB)


class AggregateMemberTests(unittest.TestCase):
    def fixture(self):
        sp="\\x"+ID;bh="\\x"+"11"*32
        oid="\\x"+hashlib.sha256(b"SPc/ArkAggregateV1"+bytes.fromhex(ID)+bytes.fromhex("11"*32)+(1).to_bytes(8,"little")).digest()[:16].hex()
        a={"order_id":oid,"sp_id":sp,"batch_seq":1,"anchor_hash":bh,"anchor_height":1,"sats":50,"not_before_height":7,"selected_tip":7}
        m={"order_id":oid,"sp_id":sp,"ordinal":0,"block_hash":bh,"block_height":1,"due_height":7,"sats":50,"reserved":True,"orphaned":False}
        before={"ark_payout":{(bh,sp):{"status":"scheduled","sats":50,"due_height":7}},"block_found":{(bh,):{"height":1,"status":"confirmed"}},"ark_aggregate_member":{}}
        return a,[m],before

    def test_original_scheduled_member_and_oldest_anchor(self):
        a,m,b=self.fixture();self.assertEqual(frozen_members(a,m,b),m)

    def test_wrong_anchor_ordinal_prior_reservation_and_missing_payout_reject(self):
        for mutation in ("anchor","ordinal","reserved","missing","amount"):
            a,m,b=self.fixture()
            if mutation=="anchor":a["anchor_height"]=2
            elif mutation=="ordinal":m[0]["ordinal"]=1
            elif mutation=="reserved":b["ark_aggregate_member"][("old",0)]=m[0]
            elif mutation=="missing":b["ark_payout"]={}
            else:a["sats"]=51
            with self.subTest(mutation=mutation),self.assertRaises(Invalid):frozen_members(a,m,b)


class AggregateHistoryTests(unittest.TestCase):
    def fixture(self):
        from test_ruleb_sql import change
        from accounting_history import REDACTED
        o,_,chain,migrations=captured()
        for event in o["tables"]["source_accounting_history"][2:]:event["transaction_id"]=3
        seed=validate_initial(o,migration_checksums=migrations,source_contract=CONTRACT)
        def add(table,row,xid,before=None):
            change(o,table,before,row,xid)
            event=o["tables"]["source_accounting_history"][-1]
            for side in ("before_row","after_row"):
                if event[side] is not None:event[side]={k:v for k,v in event[side].items() if k not in REDACTED}
        bh="\\x"+bytes.fromhex(chain.blocks[1]["hash"])[::-1].hex();tip="\\x"+bytes.fromhex(chain.tip["hash"])[::-1].hex();sp="\\x"+ID
        oid=hashlib.sha256(b"SPc/ArkAggregateV1"+bytes.fromhex(ID)+bytes.fromhex(bh[2:])+(1).to_bytes(8,"little")).digest()[:16]
        destination=b"stock-address-bytes";payload=oid+bytes([len(destination)])+destination+(50).to_bytes(8,"little")+(2).to_bytes(4,"little")
        order=payload+bytes.fromhex(_sign_digest(1,tagged("SPc/ArkSendOrder",payload)));digest=tagged("SPc/FrozenArkOrderV1",order)
        add("block_found",{"block_hash":bh,"height":1,"fingerprint":"\\x"+"55"*32,"status":"confirmed","found_at":"now","settled_at":"now","ledger_published":True},4)
        add("ark_payout",{"block_hash":bh,"sp_id":sp,"sats":50,"due_height":2,"status":"scheduled","vtxo_id":None,"expiry_height":None},4)
        a={"batch_seq":1,"order_id":"\\x"+oid.hex(),"sp_id":sp,"anchor_hash":bh,"anchor_height":1,"destination":"\\x"+destination.hex(),"sats":50,"not_before_height":2,"selected_tip":2,"selected_tip_hash":tip,
           "order_bytes":"\\x"+order.hex(),"order_digest":"\\x"+digest.hex(),"state":"frozen","revision":1,"vtxo_id":None,"expiry_height":None,"pool_loss_sats":0,"resolution_proof":None,"created_at":"now","server_pubkey":"\\x02"+PUB}
        member={"order_id":a["order_id"],"ordinal":0,"block_hash":bh,"sp_id":sp,"block_height":1,"due_height":2,"sats":50,"reserved":True,"orphaned":False}
        add("ark_aggregate",a,5);add("ark_aggregate_member",member,5)
        published={**a,"state":"published","revision":2};add("ark_aggregate",published,6,a)
        authorized={**a,"state":"authorized","revision":3};add("ark_aggregate",authorized,7,published)
        payload=b"SPAG\1"+oid+digest+b"\2"*16+(3).to_bytes(8,"little")+(110).to_bytes(8,"little")
        grant=payload+bytes.fromhex(_sign_digest(1,tagged("SPc/ArkAuthorizationV1",payload)))
        o["tables"]["ark_send_authorization"]=[{"order_id":a["order_id"],"nonce":"\\x"+"02"*16,"revision":3,"order_digest":a["order_digest"],"checked_tip":2,"checked_tip_hash":tip,"expires_at":110,"grant_bytes":"\\x"+grant.hex(),"created_at":"now"}]
        o["counts"]["ark_send_authorization"]=1
        transaction_effects(o,seed)
        return o,seed,{chain.tip["hash"]:chain},PUB,{(bh,sp):destination}

    def test_actual_schema_freeze_order_grant_and_oldest_destination_join(self):
        result=audit_aggregates(*self.fixture())
        self.assertEqual(result["aggregates"][0]["signed_grants_checked"],1)
        self.assertTrue(result["aggregates"][0]["oldest_destination_matches_publication"])
        self.assertFalse(result["wallet_received"])
        self.assertEqual(result["status"],"INCONCLUSIVE")
        self.assertIn("issuance_and_consumption_time_canonical_read_and_complete_grant_history",{m["field"] for m in result["missing"]})

    def test_current_grant_mutation_and_foreign_oldest_destination_reject(self):
        for mutation in ("grant","destination"):
            args=self.fixture();o=args[0]
            if mutation=="grant":o["tables"]["ark_send_authorization"][0]["expires_at"]+=1
            else:args[-1][next(iter(args[-1]))]=b"other-member-destination"
            with self.subTest(mutation=mutation),self.assertRaises(Invalid):audit_aggregates(*args)


if __name__=="__main__":unittest.main()
