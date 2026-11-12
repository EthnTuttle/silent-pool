Rule B reserves max(0,-actual validated pending_delta) per canonical unbooked
block, using the mined job's frozen available prior pending. New raw entitlement
is unchanged. Booking atomically applies the additive delta and releases its
reservation; unbooked cancellation only releases the reservation. Bootstrap and
publication require complete chain-bound economic state. SPCB/3 carries frozen
job B/R/A/provenance; absent historical state is unknown. See the dated decision.
