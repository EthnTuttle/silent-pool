# Independent funded traffic acceptance

This is executable test code, with an independent raw-byte oracle. It does
not yet claim a funded T3/T4b pass. The combined production binaries and the
remaining source-evidence adapters below are required for full acceptance.

Start with [PORTABLE.md](PORTABLE.md) for fresh-checkout source checks,
public fixtures, deterministic QA descriptors, actual local check receipts and
the single AP5/T3 coordinator API. Private historical review packets are not
installation prerequisites. Python 3.11+ and Linux are required.

## Run the offline proof vectors

```sh
python3 -B -m unittest discover -s testing/lab-traffic -p 'test_*.py' -v
```

The vectors cover canonical CompactSize, truncation, endian conventions,
amount overflow, duplicate inputs/transactions, witness encoding, txid versus
wtxid, Merkle mutation, compact target overflow (including valid exponents
33/34), contextual min-difficulty/overlay/boundary/inheritance/retarget rules,
halvings, raw prevout fees, coinbase overclaim, exact rollback headers, actual
100-block retained-coinbase maturity, no-send pause/restart/resume, reservation
debits versus measured attempts, observation-only native headers, the SPEC
worked payout example, immutable share-generation/credit checks, fixed ladder
amounts, and missing-evidence refusals. Fixture signing includes published
BIP340 test vector 0. These are offline vectors, never funded acceptance.

Use the interpreter from the checkout's pinned Nix development environment.
The primary publisher owns Nix check/app exports; the source-only QA snapshot
contains these testing directories and their public fixtures.

## Run real production actors

Copy `inputs.template.json` to a private input file and fill it with the
**actual combined binaries**, not paths inferred from this source baseline.
For every `binaries` entry supply:

```json
{
  "path": "/absolute/path/to/actual/binary",
  "sha256": "64 lowercase hex characters",
  "source_manifest": "/absolute/path/to/frozen/source-manifest.json",
  "source_manifest_sha256": "64 lowercase hex characters"
}
```

Required names: `lab`, `pool`, `miner`, `txd`, `node`, `scanner`,
`traffic-probe` (the actual interpreter), `redis-server`, `redis-cli`,
`initdb`, `postgres`, `pg_ctl`, `psql`, `createdb`. `probe_script` separately seals the
canonical AP-1a script. PostgreSQL utilities must share the actual bin directory.
For namespace topology add `topology: true`, sealed `unshare`/`nsenter`
binaries and a `broker_script` path/hash object. The existing broker's bounded
`exec` route invokes the actual sealed `spc-lab rpc` helper in the owned node
namespace. No unrelated inspection run is used.

An Ark-enabled scenario additionally requires actual Nix-store `captaind` and
`grpcurl` binary entries. Pass the AP-4/AP-5 combined scenario explicitly:
the included `scenario.json` is the 2-node/2-pool/4-miner/8-network-wallet/
4-payment-wallet **traffic-lane scenario**, with Ark effects disabled. It does
not stand in for stock-Ark receive/refresh/offboard acceptance. The harness
always requires the production fixed-ladder payout policy; there is no default
pre-ladder allowance.

`policy` must describe the bound node, including both independent difficulty
flags, genesis, spacing, timespan, interval, overlay, floor, BIP94 flag and
subsidy/maturity schedule. `consensus_field_map` maps each policy field to its
actual live `getblockchaininfo.consensus` key. Non-RPC constants may instead
appear in `consensus_source_facts` as `{value, source_path, source_sha256}`
bound to reviewed pinned node source. No field is defaulted from GBT or another
flag. Monetary metadata remains integers, never binary floats. `initial_target_bits`
does not mean every header must use those bits: a legitimate long-gap pow-limit
header is accepted only when the independently evaluated contextual rules
require it.

```sh
python3 -B testing/lab-traffic/regtest-traffic-e2e.py \
  --inputs /private/actual-combined-inputs.json \
  --scenario testing/lab-traffic/scenario.json \
  --run-root /tmp/opencode/NEW-owned-funded-run \
  --destination <explicit-regtest-payment-address> \
  --allow-reorg --witness --timeout 900
```

The root must not exist. Run `--case bootstrap-stop` with another fresh root
for cancellation during bootstrap. `--case bootstrap-maturity --allow-reorg`
holds the first real bootstrap distribution after signing, rolls back the
actual nodes while retaining its now-immature coinbase input, releases the
hold, and requires the exact core refusal with no intent/forward. This needs
the producer to admit operator hook control during provisioned/bootstrapping.
Bootstrap gets a durable original command
ID. Replays of manual payments reuse their exact original ID and payload.
Expanded-access scenarios read the private operator claim file on each request;
Bearer tokens are never placed in a URL, command line or observation log.
The included scenario explicitly sets `expanded_access: true` and
`traffic.lab_acceptance: true`. These are test-run opt-ins; the actor config
and manifest opt-ins are crosschecked.

The driver launches one real Runtime controller, observes its original declared
`bootstrap_fund` operation, waits for the measured boundary, captures both nodes'
raw blocks, exercises pause/resume and same-ID payments, and optionally
invalidates/reconsiders owned-node blocks containing a real parent/change-child
pair. Both restoration of the original block and still-displaced resubmission
are attempted separately. Only the production native miners advance branches.
The RPC allowlist contains no `generate*`, `setmocktime` or `submitblock`.
Ordinary and bootstrap hashing remain under the production 40 H/s aggregate
budgets; the observer never performs live header searches.

T4b funding uses that same production `send_user_payment` API. Two publicly
known fixture keys live in `../lab-witness/fixture.py`; they have no wallet,
seed derivation, UTXO ledger or funding API. One extra test actor and two keys
are recorded. The helper constructs actual BIP143 P2WPKH and BIP341 key-path
P2TR spends, sent through an explicitly owned test RPC channel. Miner pauses
allow an actual additional production legacy payment and the witness spend
to coexist in GBT before native mining resumes. The raw oracle checks fees,
weight, txid/wtxid, reserved value and BIP141 highest matching commitment;
the pool-policy check additionally requires a unique zero-valued final
commitment output. T1 production witness **inputs** are not claimed.

## Evidence and result semantics

Each run emits `acceptance.json`, `acceptance.md`, private command records,
`private/observations.jsonl`, raw block files, controller exits/reaper results
and read-only SQL captures. The final receipt is an allowlisted summary; the
raw private observations are not an HTML payload. The parent owns final HTML.
Exit codes: **0 passed**, **1 observed contradiction**, **2 inconclusive**.
Missing binaries, unknown schemas, missing anchors, truncated history or
unproved required cases cannot turn into a pass.

`oracle.py` independently reconstructs the canonical UTXO index, fees,
subsidies, header PoW, contextual bits, Merkle roots and witness commitments.
`journal.py` opens history 3 read-only and never invokes a wallet reopen,
repairs a tail or appends a session. It verifies raw funding and confirmation
claims, reservation input sets, signed destinations/change/fees, retry caps,
pause sequences, rollback branches and canonical balances. Unknown/reserved
funds are never inferred available. `native-bootstrap.jsonl` is explicitly
observation-only: its raw attempted headers must bind actual controller
reservations; charged unresolved reservations remain distinct from measured
hashes. A diagnostic record can never authorize hashing or replay funds.

`traffic_audit.py` consumes the actual T1 `traffic_audit/1` CommandLog schema:
sessions, forward attempts/results, watermarks, measured boundary,
AddConnection attempts/results and hold/loss transitions. Complete parameter
hashes are reconstructed from history-3 SignedTx bytes. Exact successful RPC
response preimages, barrier intervals and zero actual forward attempts are
checked. The controller uses the existing operator `traffic_acceptance`
command, with a bounded asynchronous payment request so release/cancel can
use the producer's explicit mutex bypass. AddConnection payloads are rebuilt
from actual recorded scenario/config, including the committed newline.
Missed polling tips become shared raw-chain prefixes only when actual audited
authority-node response hashes bind the corresponding journal revision.

`storage.py`, `storage_exec.py` and `storage_queries.py` implement placement-bound
SQL collection. PostgreSQL's actual PID/start token, executable, namespace,
launch args, postmaster file, listener ownership and rendered connection URL
select the route. A topology run's host-private Unix socket stays host-local;
genuinely namespaced PostgreSQL uses the owned broker's bounded typed helper.
There is no socket/TCP fallback across declared isolation. SQL is a closed
explicit-column, repeatable-read/read-only query with time and output caps.
Database credentials are never in argv or captured artifacts; the returned
database/role/port/datadir, cluster id, postmaster start and transport must
match the same owned instance before and after collection.

`ladder_wire.py` decodes actual `SPCB`/2 stream bytes, bounded k-ordered key
arrays, full owed, definite work profile and the separate
`SPc/DistributionSnapshotV2` snapshot key. `payout.py` verifies immutable window/share/job proofs and raw coinbase outputs,
then independently computes SPEC 4.4 and DESIGN's constant ladder. It requires
explicit independently replayed pending ancestry. Pool payout totals are only
claims to compare. Its normalized source bundle is currently a callable test
interface; it is **not yet a live adapter for the final ladder owner's stored
multi-output entry format**.

## Exact remaining integration gaps

1. Supply the sealed combined runtime with contextual T2, T1-DEST, the T1
   acceptance adapter and actual pool/miner/storage/stock-Ark components.
   The real T2 archive now has independently verified funding; its inert
   front/miner/storage components cannot satisfy full T3.
2. Wire the final immutable ladder entry/window shape to `payout.py`, retaining
   full miner job coinbase/Merkle bytes, per-share attribution, publication
   boundaries and complete prior settled/pending history. No normalized
   fixture may count as production publication.
3. Execute the implemented T1 adapter collectors against its compiled joined
   producer. The supplied QA v2 contract was source-frozen with its focused
   gate pending; consumer vectors do not imply a producer or live hook pass.
4. Exercise the source-bound definitive-rejection catalog plus controlled
   transport unknowns, and retain original-block restoration/reconfirmation
   and separate still-displaced resubmission proof. A mempool/RPC success is
   never substituted for canonical confirmation. The implemented
   bootstrap-maturity case uses the actual hold seam. The inspected QA v2
    stage guard and catalog gaps are resolved in the tested producer follow-up.
    The consumer now uses the closed before-sign v3 constructor and exact
    release-node rejection tuple. See `OWNER-API.md` for the handoff binding.
5. Bind actual witness plan/new-job/old-job-stale metadata to the miner's
   full job bytes. Canonical mixed witness blocks alone do not satisfy that
   separate republication case.

These gates remain `INCONCLUSIVE` even if the implemented mechanical subcases
succeed. This is an intentional fail-closed receipt, not completed T3/T4b
acceptance. No production behavior or normative policy was changed.

## Reproduce the independent real T2 archive check

```sh
python3 -B testing/lab-traffic/archive-oracle.py \
  --receipt /tmp/opencode/T2-T1DEST-verification-receipt.json \
  --freeze /tmp/opencode/T2-native-freeze-receipt.json \
  --output /tmp/opencode/NEW-independent-T2-audit
```

This reads the original archive without modifying it or invoking production
helpers. It verifies 33 seals, 106 raw blocks including genesis, contextual
bits, exact fees/maturity, 80 allocation outputs, both histories and
1809 measured/1810 charged attempts. The read-only public-seed oracle uses
standard BIP32/RIPEMD160, checked against public vectors; it reads no wallet
key files. All 189 funding scripts and six change scripts match the explicitly
source-bound fully hardened path. Its receipt records the source comment's
missing apostrophe, rather than silently trying alternate derivations.
The old node API logs truncate send params and omit read-response bodies;
that completeness gap stays inconclusive, not a full T3/T4b/Ark pass.

## Tested consumer follow-up (2026-11-10)

- `journal.py` accepts only the absent construction effect (version 2) or
  exactly `{"schema":"spc-lab-version-rejection/v1"}` (version 3). Unknown
  fields/effects reject. `signature_oracle.py` independently checks every
  legacy SIGHASH_ALL input against native chain prevouts. The controller
  exercises the actual closed constructor, same-ID replay, changed-semantics
  conflict, fresh rejection release and retained prior-Unknown reservations.
- `pool_source.py` consumes actual source-evidence/2 log framing, hash chains,
  frozen/issued jobs, attributed shares and SPCB/2/slice publication bytes.
  Its external log seal must come from the collector. RAM slice publication
  may follow the freeze; numeric slice IDs remain distinct from Redis IDs.
- SQL query v2 adds `source_accounting_history`, `ark_aggregate`,
  `ark_aggregate_member` and `ark_send_authorization`, plus the same-snapshot
  event watermark and WAL location. `accounting_history.py` replays actual
  before/after images and DELETE tombstones against that snapshot. Sequence
  gaps from rolled-back allocations are legal. External-checkpoint starts,
  missing rows and unknown economic producer tables remain INCONCLUSIVE.
  This row-consistency check does not prove a past job's pending balance.
- `net_debits.py` independently implements RULEB arithmetic. It has no record
  adapter until the new producer schema and frozen handoff are supplied.
- `rpc_capture.py` now backs controller node reads and explicitly authorized
  test writes. It records durable begin/end observations and complete private
  content-addressed request/response bodies, including node errors and malformed
  responses. Credentials stay out of records. `canonical_capture.py` records
  bracketing per-node canonical reads and preserves historical body timestamps.
  `private/chain-evidence.json` is a read-only AP5 export; its consumer requires
  external export, binary, source, scenario, policy and node-census bindings.
  Consistent observations do not assert continuous canonicality between reads.

The stock-Bitcoin ladder archive audit at
`/tmp/opencode/T3-independent-ladder-r3-archive-v2/receipt.json` verifies 118
source events, Q=9 and all 990 derived outputs/tweaks totaling 4,950,000,000
sats. Full entitlement remains INCONCLUSIVE because that r1 archive has no
complete RULEB reservation history. It is separate from rbitcoin T3 and Ark.

The live joined run, full RULEB payout proof and runtime pre-mining proof-feed
consumption remain outstanding. The tested
producer CLI/txd candidate omits the native installer and is not the joined
bundle. The frozen A1-A4 packet remains available for same-auditor review.

The serial multi-case executor and receipt conjunction are now implemented in
`campaign.py`; see [CAMPAIGN.md](CAMPAIGN.md) for the required-case matrix,
common source/input/config bindings, fresh roots, watchdog/cleanup budgets and
separate T3/T4b versus AP5/whole-lab statuses. The actual live source collector
and job/witness/publication adapter are wired into the controller; see
[SOURCE-ADAPTER.md](SOURCE-ADAPTER.md) for the concrete remaining producer joins.

Source-r3 adds the actual collector transport and explicit migration-0011 seed:
[SOURCE-FEED-IMPLEMENTATION.md](SOURCE-FEED-IMPLEMENTATION.md) documents the
implemented Unix server/replay path and the confirmed native activation bypass
plus initial-window dependency. [AP5-V2-CONSUMER.md](AP5-V2-CONSUMER.md) describes
the new externally sealed AP5 v2 component/cleanup consumer and the distinction
between synthetic conformance and actual funded board proof. A1-A4 approval is
bound to the separately preserved r4 correction; these additions do not change
those five source rows or imply a whole-lab pass.

[COLLECTOR-READINESS.md](COLLECTOR-READINESS.md) records the follow-up full-Q
campaign gate, coherent-snapshot recovery, revision-ordered history replay and
closed settlement provenance checks. Audited warmup is parent-authorized, but
its producer contract and the native activation codec must be supplied before
their parsers are implemented. Neither permission can replace eventual full
Q coverage or the actual pre-hash verifier-ACK proof.
