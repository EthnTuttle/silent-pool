# r25 measured pause and exact PostgreSQL identity

## Actual b16 scope defect

The original safe traceback identifies measured's all-journal no-send guard.
Only txd-0 had been paused. Its acknowledged send_barrier was seq 382, with no
later rows/sends. Two ordinary txd-1 sends, sequences 17 and 18, match that
driver's journal and CommandLog forwards after txd-0's ACK and before failure.
txd-1 had no pause command. This is an observer coverage bug, not evidence that
the actually paused producer sent. Original in-memory snapshot lengths were
not retained; the offline reconstruction's cuts are explicitly reconstructed.

Measured pause now sends one durable pause to every declared actor. All ACKs
precede the common baseline/interval. Each ACK's run/command identity, journal
sequence, last-send identity and clock bind its actual send_barrier. Snapshots
are keyed by the header's driver, with exact declared/manifest/journal census,
paths and run IDs; missing, duplicate, foreign or changed journals fail closed.
Before/after counts and hashes are retained so exact cuts remain replayable.

The no-send guard covers every driver's suffix after its own barrier, including
the time spent waiting for other actors' ACKs. Broadcast, reorg-resubmit and
premature resume records are Invalid. Other pre-ACK actors' legitimate activity
is not mislabelled. All actors resume only after the verified common interval.
An attempted resume invalidates local paused-state reuse even on an unknown ACK.
Full pause acceptance still requires TrafficAudit's actual forwarding proof.

## AP5 reuse API

`acknowledged_traffic_pause(driver_id)` is read-only. It returns None when no
successful pause is reusable, a later resume was attempted, or the current
driver incarnation differs. Otherwise it checks the actual CommandLog session
and controller/actor executable bindings, controller principal/payload hash,
successful outcome, actor ACK, owned journal barrier, closed watermarks and
absence of later resumed/sent records or forwarded effects. Contradictions
raise; they are not converted into a new pause or a pass.

The proof identifies run_id, driver_id, incarnation, boot_id, command_id,
controller begin/finish sequences, ack, closed watermark sequences and journal
path/hash/record-count/barrier sequence. Its scope is only the current recorded
incarnation's acknowledged pause. AP5 keeps require_active_finalization(),
per-driver failure isolation and one final drain/stop. Reuse a proof, or make
one new pause after None and require a newly established proof. Phase booleans
and qa.commands membership alone are not pause proof. AP5 owns that integration.

## Exact storage identity, no timestamp tolerance

The merged storage-owner patch binds the SQL server's bounded
`pg_read_file('postmaster.pid', 0, 8192)` bytes by exact SHA256 to the locally
ownership-bound PID file, then explicitly matches PID, directory, start second
and port. Existing UID/process/socket/namespace/resource and pre/post-query
checks remain. PostgreSQL's earlier PID-file MyStartTime and later PgStartTime
are distinct timestamps; no rounding/tolerance can establish identity.

The stock pinned PostgreSQL 17.11 gate executed the 22-table fullrail query after
12 migrations, validated its result, rejected six wrong bindings, and stopped
its owned test cluster cleanly. That real gate is consumed, not rerun here.
The rejected b16 pool-A SQL timestamp was not retained and remains UNKNOWN.
Old captures retain their old frozen query/validator, without conversion.

The new server field/query bytes require fresh input seals. Both QA prerequisites
and AP5's shared preflight compare source_feed.sql_query against actual loaded
query bytes (contract, ID, schema, hash, bytes and table count). No query ID-only
compatibility or arbitrary hash override is allowed. Application source/binaries
and the selected 98f/3cf bundle remain unchanged.

## Limits

b16 bootstrap succeeded, and 96 allocation outpoints for 12 journal-labelled
roles match both canonical chains at height 106. Four SP registration leases
were observed live. The four later AP5 server/pool Ark funding transfers and
full receive/refresh/second-renewal/offboard lifecycle were not executed. Global
cleanup remains INCONCLUSIVE because of the unreadable same-UID processes; none
is allowlisted. b14/b15 abort causes remain UNKNOWN. No historical run or raw
capture is rewritten by this release.
