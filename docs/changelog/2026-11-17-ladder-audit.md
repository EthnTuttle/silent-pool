# Changelog fragment — 2026-11-17 ladder implementation audit

- "2026-11-17: denomination-ladder implementation audit closes the stale
  STATUS item: the 2026-11-03 ladder decision is fully implemented on master
  (absorbed via the source-union integration and the 2026-11-09
  ladder-entry-v2 amendment); every normative item is mapped to code and
  tests in docs/reviews/2026-11-17-ladder-implementation-audit.md. The old
  `ladder` branch (tip df7c654) is fully superseded and safe to archive;
  three artifacts the union dropped are re-landed: the store test for
  coinbase-row remainder scheduling/orphan reversal (`ark_due_sats > 0`,
  not rail, keys the schedule), the regtest-services banded-coinbase
  assertion (990 x 5M-sat rungs, 4.95 BTC paid sum), and the era-1
  value-bound test (619 <= 1024 <= 2048). Also fixes a pre-existing master
  test breakage unrelated to the ladder: the agent's aggregate-API config
  test expected `AggregateApi` where the c55b737 chain gate now rejects the
  mainnet/tsp address pairing first; the off-regtest case uses signet.
  No behavioral change."
