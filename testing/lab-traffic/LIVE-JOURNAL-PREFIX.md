# r24 live CommandLog and EvidenceStore prefix observation

The b14/b15 final command logs reproduce all 57 retained successful bootstrap
observations. The 761 complete suffix boundaries after their final successful
observations also pass the original r23 parser and budget checks. Neither lost
exception message can be recovered; both original execution-abort causes remain
UNKNOWN. b14's config-binding failure was definite but was recorded rather than
raised, so it is not proved to have caused its later observer abort.

AP5's sealed copied-fixture experiments separately demonstrated two reachable
r23 guards: a valid append trips immutable private_bytes' size/mtime check,
and a last record without its newline trips strict read_commands. Those are
source hazards, not an attribution of the missing historical exceptions.

## Live prefix contract

`LiveJournalPrefix` is used only for the owned live commands.jsonl and
evidence.jsonl observations. Immutable configs/keys still use the unchanged
strict `storage.private_bytes`. The strict journal, phase and hash-budget
parsers are unchanged.

Each read walks the path with directory-relative descriptors and NOFOLLOW,
checks run-root confinement, directory/file ownership, private regular-file
mode and single-link status, and pins the root/file device/inode identity.
It reads only the initially bounded file length, then verifies those exact
bytes through a second independently opened descriptor. A final path/metadata
check catches replacement or truncation during verification. Concurrent growth
is permitted only beyond the captured bytes. No descriptor remains open between
observations, so these readers do not create cleanup file-reference survivors.

The complete newline-terminated prefix is passed to the original strict parser.
A pending tail contributes no record, reservation, reconciliation or terminal.
Its bytes are remembered too: later replacement of a previously seen fragment
is rejected. Observed file sizes may only grow; global/record byte bounds remain
enforced, including growth during verification. Missing previously observed
paths, changed root/inode, rewritten prefixes, hardlinks, wrong ownership/mode,
short reads and resource violations fail closed and poison that reader.

The bootstrap observer is serialized with the existing observation lock. It
retains the original operation/principal, deadline and reservation/debit checks.
No general Inconclusive exception is swallowed or retried. The existing
bounded polling loop still waits only for the explicitly permitted not-yet-
begun/not-yet-terminal states. Captured prefix metadata says
`verified_complete_live_prefix_not_final_EOF`; a live read never supplies the
final writer-stop/EOF/SQL seal or proves fsync durability.

## Finalization API

Before final ordinary-traffic pause requests, the AP5 coordinator calls
`qa.require_active_finalization()`. It reads the actual run/stage and verifies
the successful original bootstrap and its actual phase outcome. It has no
RPC, hash, cancellation or stop authority. During bootstrap it raises the
specific `traffic_not_active_no_final_quiescence` missing-component result.

Collector startup completion is recorded separately from native activation.
The collector's one final_drain refuses an unstarted collector or non-active
traffic before marking miners stopping or issuing remove_miner. On the active
path it rechecks original bootstrap completion before the existing removal,
convergence, EOF and final-audit sequence. An early failure stays INCONCLUSIVE;
the coordinator still closes readers and performs its one owned stop/reap.
AP5 owns the coordinator-side call and safe code-location diagnostics.

Tests use deterministic writer-thread barriers, public miniature command/phase
fixtures, replacement/pruning/ownership/resource negatives, pending fragments,
malformed complete records, budget resets and concurrent observer calls. The
already-published AP5 copied b15 fixtures are also consumed read-only with the
new reader. No runtime applications, public wire encodings, keys or sealed
historical runs are changed. r23 remains a separately frozen historical release.
