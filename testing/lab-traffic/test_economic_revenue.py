"""Row/UTXO conservation only; no fixture is a funded or ownership proof."""
import unittest

from economic_revenue import net_pool_output
from oracle import Invalid, ladder, raw_payouts
from source_feed_provider import ReplayProvider
import test_audited_window as window_vectors

ID="11"*32


def row(paid,delta,ark=0,forfeit=0):
    return {"sp_id":list(bytes.fromhex(ID)),"paid_sats":paid,"pending_delta":delta,"ark_due_sats":ark,"forfeited_sats":forfeit}


class EconomicRevenueTests(unittest.TestCase):
    def test_actual_failed_proposal_numbers_fit_revenue_below_gross_K(self):
        result=net_pool_output(5000000000,{ID:4949999999},[row(4950500000,-500001)])
        self.assertEqual(result["gross_K"],50000001)
        self.assertEqual(result["net_pool_output"],49500000)
        self.assertLess(result["net_pool_output"],result["gross_K"])

    def test_prior_only_empty_rewards_can_repay_liability_without_P_zero(self):
        result=net_pool_output(5000000000,{},[row(500000,-500000)])
        self.assertEqual(result["gross_K"],5000000000)
        self.assertEqual(result["net_pool_output"],4999500000)

    def test_Ark_forfeit_and_signed_pending_delta_obey_same_conservation(self):
        result=net_pool_output(1000,{ID:900},[row(800,40,ark=50,forfeit=10)])
        self.assertEqual(result["net_pool_output"],200)
        self.assertEqual(result["gross_K"]+result["ark_due"]+result["forfeit"]+result["pending_delta"],200)

    def test_actual_sumPaid_above_revenue_and_unexplained_delta_reject(self):
        with self.assertRaisesRegex(Invalid,"sumPaid"):net_pool_output(1000,{},[row(1001,-1001)])
        with self.assertRaisesRegex(Invalid,"row paid"):net_pool_output(1000,{ID:900},[row(950,0)])
        with self.assertRaises(Invalid):net_pool_output(1000,{ID:900},[])

    def test_real_settlement_helper_handles_empty_partial_full_with_positive_prior(self):
        # The independently proven empty history certificate is from signed
        # synthetic source/SQL fixtures. The following output shapes isolate
        # settlement arithmetic, not wallet-owned script derivation.
        empty,_,_,_,identity=window_vectors.AuditedWindowTests().verify(0)
        _,public_keys=window_vectors.identity_keys()
        for work in (0,1,9):
            window=empty if work==0 else {"selected":[[{"identity":identity,"work":work,"fees":0}]]}
            fresh={} if work==0 else raw_payouts(window["selected"],5000000000,0,100)[0]
            prior=750000;owed=fresh.get(identity,0)+prior;pieces,rem=ladder(owed)
            keys=list(public_keys)+["11"*32]
            p={"fee_bps":100,"entries":[{"sp_id":identity,"owed_sats":str(owed),"sats":str(sum(pieces)),"rail":0,
                "p_x":keys[:len(pieces)],"delivery_state":"unregistered","destination_digest":None}]}
            outputs=[{"value":5000000000-sum(pieces),"script":bytes.fromhex("5120"+"22"*32)}]
            outputs += [{"value":amount,"script":bytes.fromhex("5120"+key)} for amount,key in zip(pieces,keys)]
            block={"subsidy":5000000000,"fees":0,"transactions":[{"outputs":outputs}]}
            ledger=ReplayProvider._settlement(None,p,block,{identity:prior},window)
            self.assertEqual(ledger[0]["pending_delta"],rem-prior)
            self.assertLess(outputs[0]["value"],5000000000-sum(fresh.values()))
            with self.assertRaises(Invalid):ReplayProvider._settlement(None,p,block,{},window)


if __name__=="__main__":unittest.main()
