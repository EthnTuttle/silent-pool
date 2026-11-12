# Actual source publication, witness and pre-mining components

The controller creates `SourceCapture` before spawning the runtime, while its
fresh runtime root is absent. The collector binds each actual front's
PID/start token, parent controller, executable and declared stdout log. It
reads `run/logs/front-<front-id>.log` without modifying producer files, retains
every observed byte, checks append-only prefixes/inodes and records independent
prefix hashes in the private observation stream. Incomplete tails and unknown
events remain INCONCLUSIVE on every poll, not only the first failed parse.

`source_jobs.py` consumes the already supplied source-evidence/2 records. It:

- Reconstructs the actual stripped coinbase at its declared extranonce slot,
  minimal BIP34 height and SPc carrier.
- Binds height/parent and contextual difficulty to independently decoded raw
  ancestry. It neither searches headers nor copies a GBT target fallback.
- Resolves actual fee prevouts, including in-template parents, rejects reused
  inputs, and verifies raw fees, subsidy/revenue and the coinbase Merkle path.
- Recomputes wtxid roots, reserved-value commitment and the unique final
  zero-value commitment output from the actual transaction bytes.
- Joins actual SPCB/2 bytes to frozen work/high-water policy, issued channel,
  unique winning share/header and mined raw coinbase/transaction/witness bytes.
  Retained raw blocks matching issued pool templates require publication
  evidence too; checking only the subset of published records is insufficient.
- Requires a same-parent transaction-set refresh, a prior issued old job,
  actual issued new job and subsequent `stale-share` for that old job on the
  same channel. A normal new-height witness block does not substitute for
  witness republication.

These are wired live collectors/checks in `regtest-traffic-e2e.py`, not a
renamed normalized fixture wrapper. Production execution still needs the
actual joined bundle.

## Pre-mining materialization and precise producer gaps

The r5 construction-gap inventory below is superseded by source-r3 where noted
in [SOURCE-FEED-IMPLEMENTATION.md](SOURCE-FEED-IMPLEMENTATION.md): finding-slice
fields, the Unix frame/ACK exchange and explicit initial checkpoint are now
implemented on the consumer side. The actual native activation route and
initial-window policy remain unresolved; no translator-only pass is claimed.

Every captured frozen generation is materialized under `private/pool-source/`.
Job, accepted-share and slice component fields use the frozen Rust
`source_audit.rs` byte-array layouts. They are derived from actual log framing,
raw ancestry and selected publications, not supplied owed scalars. Later
prefixes/ancestry can produce new immutable component revisions. Captured
prefixes remain independently sealed; old revisions are preserved.

These files have the diagnostic schema `spc-lab-source-components/1`. They are
deliberately not serialized as a complete Rust `SourceBundle` with invented
values. `for_job` verifies its component seal and raises a typed
`MissingComponent` rather than returning a `TrustedSource` or a mining grant.
The remaining producer joins are exact:

1. **RULEB contract:** source-owned sealed record schema for frozen per-block
   P, actual E/delta, canonical unbooked reservations, booking/orphan lifecycle,
   coherent bootstrap and chain/economic watermark before distribution. The
   offline parser now consumes the explicitly unverified working r2 API; see
   [RULEB-WORKING.md](RULEB-WORKING.md). Live acceptance awaits the tested
   producer binding. Current `pending_balance` is not substituted.
2. **Finding-slice boundary:** the frozen `SourceBundle` requires
   `finding_slice_id`, but `distribution_frozen` does not provide an authenticated
   future finding-slice boundary. A later winning share is retrospective, not
   proof of the pre-mining boundary. No max-slice-plus-one value is invented.
3. **Runtime feed consumption:** frozen `translator.rs` exposes only the
   in-process `SourceProofFeed::for_job` / `bind_and_run_with_auditor` Rust
   interface. There is no supplied external transport or live acknowledgement
   that the actual received SV2 job was checked before downstream publication.
   The assigned agent/runtime owner must supply that concrete interface and
   before-publication evidence. The collector cannot implement it by editing
   another owner's production source.
4. **Witness stale path:** the parser is ready for actual `share_rejected`
   records. A run with no same-parent old-job stale submission remains unknown;
   the harness does not fabricate a miner share or report a normal height
   change as this test.

`observed_before_issued_record` describes only the collected log prefix. It
does not establish that the runtime consumed a proof before hashing. The
`premining_proof_feed` gate requires actual consumption evidence; component
files never authorize mining, credit, sends, funding or Ark closure.

## Read-only real archive verification

```sh
python3 -B testing/lab-traffic/source-jobs-archive.py \
  --archive /tmp/opencode/ladder-completion-native-r3 \
  --output /tmp/opencode/NEW-source-job-component-audit
```

This reads the stock Bitcoin archived block file and its disk XOR encoding,
validates the target block's 13-block raw ancestry, then runs the same actual
job/publication adapter used live. Receipt:
`/tmp/opencode/T3-source-jobs-archive-r2/receipt.json`.
It proves the real frozen job, issued/winning share and 990-output block's
publication binding. The archive contains no witness-refresh case and no
complete RULEB ancestry. It is not rbitcoin T3/T4b or Ark acceptance.
