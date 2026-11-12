"""Independent RULEB arithmetic, awaiting its concrete producer record schema.

No database, wallet or default-zero history. Inputs are explicit values from a
separately verified checkpoint and immutable per-block economic records.
"""
from oracle import Inconclusive, integer, require


def available(booked_pending, canonical_unbooked_reservations):
    b = integer(booked_pending)
    if canonical_unbooked_reservations is None:
        raise Inconclusive("reservation history missing; cannot infer zero")
    r = sum(integer(value) for value in canonical_unbooked_reservations)
    require(r <= b, "canonical unbooked net-debit reservations exceed booked pending")
    return b - r


def block_economics(frozen_prior, actual_error_pending):
    p, e = integer(frozen_prior), integer(actual_error_pending)
    delta = e - p
    return {"frozen_prior": p, "error_pending": e, "delta": delta,
            "net_debit": max(0, -delta), "positive_credit": max(0, delta)}


def found(booked_pending, existing_reservations, economic):
    require(economic == block_economics(economic["frozen_prior"], economic["error_pending"]), "inconsistent frozen block economics")
    before = available(booked_pending, existing_reservations)
    c = economic["net_debit"]
    after = available(booked_pending, list(existing_reservations) + [c])
    require(after == before - c, "found block changed booked pending or wrong reservation")
    return {"booked_pending": booked_pending, "reservation": c, "available": after}


def book(booked_pending, all_reservations, own_reservation, economic):
    require(economic == block_economics(economic["frozen_prior"], economic["error_pending"]), "inconsistent frozen block economics")
    before = available(booked_pending, all_reservations)
    require(own_reservation == economic["net_debit"], "booked block reservation differs from frozen net debit")
    remaining = list(all_reservations)
    require(own_reservation in remaining, "booked block reservation missing")
    remaining.remove(own_reservation)
    b = booked_pending + economic["delta"]
    integer(b)
    after = available(b, remaining)
    require(after == before + economic["positive_credit"], "booking did not release exactly the positive booked credit")
    return {"booked_pending": b, "reservations": remaining, "available": after}


def orphan_unbooked(booked_pending, all_reservations, own_reservation):
    before = available(booked_pending, all_reservations)
    remaining = list(all_reservations)
    require(own_reservation in remaining, "orphaned unbooked reservation missing")
    remaining.remove(own_reservation)
    after = available(booked_pending, remaining)
    require(after == before + own_reservation, "unbooked orphan applied a nonexistent booked delta")
    return {"booked_pending": booked_pending, "reservations": remaining, "available": after}
