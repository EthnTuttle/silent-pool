"""Read-only archive regression vectors; no fabricated live checkpoint."""
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from ladder_final_archive import Audit, pool_script
from net_debits import available, block_economics, found, book
from oracle import Invalid, Inconclusive
from source_jobs import MissingComponent

DESCRIPTOR="tr(tpubD6NzVbkrYhZ4WUjXBVxZPvGhWxTRE5veAscTgokX65xcfCoDF5nyNdtaS4xGoJ7xnZuDTxCAEAnGpxJbWo6wtTAJsESmeJWpZzybZ6dvCia/0/*)"


class FinalArchiveTests(unittest.TestCase):
    def test_public_pool_descriptor_matches_sealed_actual_raw_output_vectors(self):
        expected={1:"5120949ff68ccdd3e5850f9969ddd9fcb649a805a6770d24530d3c0cdc321c9b5a71",
            16:"5120108e0a83ff5251a4a7eded94f5b006af375166678ef6a4efeb123d17228fbd12",
            17:"51206178e6917236a198d9bc41a93f618feb29576c3529c11abb51314bd2f4b17255",
            23:"51209d8d092ac7ee99e9b6193fb1cd03a7b1881077939004f56335f46c3710340a62"}
        for height,script in expected.items():self.assertEqual(pool_script(DESCRIPTOR,height).hex(),script)

    def test_descriptor_never_falls_back_on_private_hardened_or_ambiguous_paths(self):
        for value in (DESCRIPTOR.replace("tpub","tprv"),DESCRIPTOR.replace("/0/*","/0h/*"),DESCRIPTOR.replace("/0/*","/<0;1>/*"),
                      DESCRIPTOR.replace("/0/*","/0"),DESCRIPTOR.replace("tpubD6","tpubD7")):
            with self.assertRaises((Invalid,Inconclusive)):pool_script(value,16)
        with self.assertRaises(Invalid):pool_script(DESCRIPTOR,2**31)

    def test_actual_H17_positive_credits_do_not_offset_other_identity_reservations(self):
        B=[723053,446578,830365];c16=[625000,375000,500000]
        P17=[98053,71578,330365];E17=[119792,310708,69495]
        economics=[block_economics(p,e) for p,e in zip(P17,E17)]
        self.assertEqual(sum(e["delta"] for e in economics),-1)
        self.assertEqual(sum(e["net_debit"] for e in economics),260870)
        self.assertNotEqual(sum(e["net_debit"] for e in economics),max(0,-sum(e["delta"] for e in economics)))
        for b,prior_reservation,p,e in zip(B,c16,P17,economics):
            self.assertEqual(available(b,[prior_reservation]),p)
            c=found(b,[prior_reservation],e)["reservation"]
            first=book(b,[prior_reservation,c],prior_reservation,block_economics(b,b-prior_reservation))
            second=book(first["booked_pending"],first["reservations"],c,e)
            self.assertEqual(second["available"],e["error_pending"])

    def test_later_final_SQL_state_cannot_supply_an_absent_frozen_checkpoint(self):
        audit=object.__new__(Audit);key=("session",0,"00000019",19)
        audit.contexts={};audit.visiting=set();audit.source=SimpleNamespace(jobs={key:{"payload":{"height":19,"economic":{"revision":50}}}})
        audit.chain=SimpleNamespace(prefix=lambda height:SimpleNamespace(tip={"height":height,"hash":"11"*32}))
        audit.snapshots={62:[("actual-later-final.json",{"tables":{"economic_state":[{"revision":62,"complete":True,"anchor_height":22,"anchor_hash":"\\x"+"22"*32}]}})]}
        with patch("ladder_final_archive.replay_checkpoint") as replay:
            with self.assertRaisesRegex(MissingComponent,"height_19_revision_50"):audit.context(key)
            replay.assert_not_called()
        self.assertEqual(audit.contexts,{})


if __name__=="__main__":unittest.main()
