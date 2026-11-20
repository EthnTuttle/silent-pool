  - "2026-11-20: zero-rounded shares are on no rail (lab halt at the first 2-sat regtest block): an identity whose owed(id) = raw(id) + pending(id) is zero has no distribution entry, JD weight, frozen balance, ledger row or Ark order; its floor remainder is already in gross K (§4.4). Build and settlement apply one rule (`spc_accounting::published_ids`): settlement requires an entry for exactly the identities the frozen window owes and still fails closed (`WindowMismatch`) on any other absence. No amount, rail rule, ladder, forfeiture or Rule-B behavior changes; the Rule-B readiness hold on an unvalidated canonical pool block is kept. §4.6, §4.7, §7.4 and §11.1 state the rule; §7.4 also states that a found block whose frozen slice row is not yet persisted is held and retried, and reported as an error from the fifth attempt. See docs/decisions/2026-11-20-zero-rounded-shares.md."

Merge-session notes (not changelog bullets; delete with this fragment):

This packet did not edit `docs/SPEC.md`. Apply the body hunks below at merge
(exact old -> new text; non-ASCII characters are the SPEC's own).

### Hunk 1 - §4.6 routing pseudo-code: one inserted line

After the line

```
owed(id)  = raw(id) + pending(id)                      # pending is error-held only (2026-11-03: no capacity deferral)
```

insert

```
published = { id : owed(id) > 0 }                      # owed = 0 is on no rail: no entry, no ledger row (decision 2026-11-20)
```

### Hunk 2 - §4.6 invariant (e)

Old:

```
(e) an id is on exactly one rail per block (`kept`, `ark_due'`, `error pending'`, `forfeited` are disjoint):
```

New:

```
(e) an id with `owed > 0` is on exactly one rail per block and an id with `owed = 0` is on none (`kept`, `ark_due'`, `error pending'`, `forfeited` are disjoint; decision 2026-11-20-zero-rounded-shares):
```

### Hunk 3 - §4.7 distribution builder, `entries` line comment

Old:

```
    entries = [(sp_id, rail, {ser_x(P_id,k)} | dest_id, pay(id) | owed(id))]    # snapshot payload; scrubbed later
```

New:

```
    entries = [(sp_id, rail, {ser_x(P_id,k)} | dest_id, pay(id) | owed(id))]    # one per id with owed(id) > 0; snapshot payload; scrubbed later
```

### Hunk 4 - §7.4, appended to the end of the section's paragraph

Append after "...after which the block may be confirmed and booked.":

```
Settlement requires a snapshot entry for exactly the identities the reconstructed window owes - `owed(id) > 0`, the build's own publication rule (decision 2026-11-20-zero-rounded-shares): every id with fresh `raw(id) > 0` and, in pool-template mode, every id with frozen prior `pending(id) > 0`. An identity whose share floors to zero with nothing pending has no entry and is not a mismatch; any other absence fails closed. A found block whose frozen `slice_ids` name a slice row `payout` has not persisted yet (the `spc:slices`/`spc:blocks` read order can deliver the block first) is held unacked and retried, never acked and never reconstructed from another slice set (WIRE-FORMATS §3.1: missing slices hold settlement); from the fifth attempt on each attempt is reported as an error and the block stays held.
```

### Hunk 5 - §11.1, new bullet before "- **Denominations (decision 2026-11-03), policy constraints.**"

```
- **Zero owed (decision 2026-11-20).** An id the window credits whose `owed` is zero - `raw` floored to zero (§4.4) and nothing pending - takes none of the branches above: it is on no rail, has no entry and no ledger row, and nothing is forfeited from it. Its floor remainder is already in `pay_P` as part of gross `K`.
```

### Other shared files

None edited. `docs/OPERATIONS.md` has no hunk: the only operator-visible
change is log text (`found block waits for a frozen window slice not
persisted yet; retrying` at debug then warn, `frozen window slice <id> still
not persisted after <n> attempts` at error; `WindowMismatch` now reads
`window owes an id the distribution did not publish`).
