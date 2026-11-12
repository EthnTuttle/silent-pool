import unittest

from oracle import Inconclusive, Invalid
from net_debits import available, block_economics, book, found, orphan_unbooked


class RuleBVectors(unittest.TestCase):
    def test_fresh_raw_is_not_subtracted_as_unbooked_paid(self):
        reservations = []
        for _ in range(2):
            p = available(0, reservations)
            self.assertEqual(1000000 + p, 1000000)
            e = block_economics(p, 0)
            reservations.append(found(0, reservations, e)["reservation"])
        self.assertEqual(reservations, [0, 0])

    def test_partial_error_pending_keeps_100k_available(self):
        e = block_economics(500000, 100000)
        self.assertEqual(e["net_debit"], 400000)
        self.assertEqual(found(500000, [], e)["available"], 100000)
        self.assertEqual(book(500000, [400000], 400000, e)["available"], 100000)

    def test_jd_contraction_reserves_full_frozen_prior_not_min_paid(self):
        e = block_economics(500000, 0)  # actual claim/paid may be only 300000
        self.assertEqual(e["net_debit"], 500000)
        self.assertEqual(found(500000, [], e)["available"], 0)

    def test_unbooked_positive_credit_cannot_be_spent(self):
        e = block_economics(100000, 300000)
        self.assertEqual(found(100000, [], e)["available"], 100000)
        self.assertEqual(book(100000, [0], 0, e)["available"], 300000)

    def test_unbooked_orphan_does_not_reverse_unbooked_delta(self):
        self.assertEqual(orphan_unbooked(500000, [400000], 400000), {"booked_pending": 500000, "reservations": [], "available": 500000})

    def test_missing_or_overdrawn_reservation_history_never_defaults_zero(self):
        with self.assertRaises(Inconclusive):
            available(500000, None)
        with self.assertRaises(Invalid):
            available(500000, [500001])

    def test_claimed_min_paid_cannot_override_derived_reservation(self):
        economic = {**block_economics(500000, 0), "net_debit": 300000}
        with self.assertRaisesRegex(Invalid, "inconsistent frozen"):
            found(500000, [], economic)
        with self.assertRaisesRegex(Invalid, "inconsistent frozen"):
            book(500000, [300000], 300000, economic)


if __name__ == "__main__":
    unittest.main()
