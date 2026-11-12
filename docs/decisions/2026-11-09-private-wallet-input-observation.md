# Private wallet input observation for the owner UI

Date: 2026-11-09
Status: implemented; focused scoped gate passed

## Decision

The running wallet writes `<configured ark.datadir>/ui-inputs.json` as a
disposable private observation. In the AP-3b lab mapping this is
`private/wallet-<identity>/state/ui-inputs.json`, not the online provisioning
root. The existing wallet process/writer owns publication; no new daemon, RPC
pump, wallet instance, ledger or public HTTP route is introduced.

The closed DTO is `spc-agent-private-inputs/v1`: integer `updated_at_ms` and
`inputs` containing only `id`, `amount_sats`, `state`, `expiry_height`, `rail`.
IDs are lowercase 72-character hex encodings of complete 36-byte Bark outpoints;
amounts are canonical u64 decimal strings; expiry is null or u32; rails are
`derived`/`normal`. This producer emits only selectable `spendable` rows with a
known expiry. Pending, spent, unknown, reserved, queued-owner and pending-round
inputs are excluded. Unknown proof makes the observation unavailable.

Input observations are captured only from the existing successful SP receipt
and stock-input validation passes. Publication rechecks current SP state and
reservations, preventing SP/stock double counting after a round. It never
promotes arbitrary ledger rows to spendable input evidence. The observation
timestamp begins before proof reads, uses the older of the two rail batches,
and must be at most 15 seconds old at publication. Errors/overflow/limits do
not stamp old available data with a new time; the inert file is removed where
the validated private root permits it. Successful empty observations use an
empty input list, disabling actions without asserting a wallet balance.

The file is bounded to 4096 rows and 1 MiB. Its root must be the canonical,
owned, non-symlink 0700 initialized wallet datadir. Publication uses the pinned
Bark owner-only atomic writer (exclusive 0600 temp creation, full write, file
fsync, rename) plus directory fsync. Root identity is rechecked. Reader failures
do not grant or revoke funds authority and do not change wallet math/counters.

## Authority and privacy

The observation is not authorization. Existing owner command execution must
still revalidate actual ownership, canonical proofs and atomic reservations.
No seed, xprv, cookie, script/proof, third-party identity or private path appears
in the DTO. Neither this producer nor the UI derives balance by summing all
inventory entries. The existing SQLite and stock Bark persister remain the
sole wallet state. Registration and the approved R1-R6 changes are preserved.

## Documentation/evidence

- Changelog fragment for serial integration; no SPEC body/frontmatter edit.
- `docs/reviews/2026-11-09-wallet-ui-input-provider.md` records source binding,
  tests and integration paths. Selector-state fixtures are not funded proof.
