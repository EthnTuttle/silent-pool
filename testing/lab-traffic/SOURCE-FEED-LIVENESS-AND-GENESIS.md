# r26: source-feed liveness evidence, query representation and initial genesis

## What b17 actually establishes

The four normal CPU miners emitted contiguous candidate/unknown streams, no
COMMITTED/activation/first-hash events, and reported zero hash attempts. All
39 collector request beginnings match native received-job digests. The collector
retained no prepared, consumed or committed proof. Two actual source snapshot
attempts failed the exact checkpoint INSERT/readback equality guard; none was
admitted. The witness funding transaction appears in both fronts' proposed
height-107 templates, while the canonical chain stopped at the bootstrap 106.

This demonstrates a necessary source admission blocker on the required native
audit path. It does not identify every generic native Unknown/timeout or prove
that fixing timestamps alone completes all remaining native/funding/Ark work.
The compiled gate explicitly supports committed-underfilled jobs, so full-Q
coverage is an eventual requirement, not a reason to mine unaudited startup
work. No further deterministic native protocol defect was established from
the retained generic outcomes. Individual exchange error locations were missing.

## SQL representation

The storage-owner patch pins transaction-local TimeZone UTC and DateStyle
ISO, MDY, matching the independently verified sqlx writer startup policy.
Original JSONB journal strings are not rewritten. Exact row equality, wrong-
instant/content rejection and exact PID-file binding remain unchanged. The
stock PostgreSQL gate proves old failure/new success for both checkpoints;
a genuinely non-UTC writer image remains rejected. No SQL gate is rerun by QA.

The query ID/schema stay unchanged but fullrail bytes are now 8828 and SHA256
7d4c64aa165618cac9b73024042a858161d03e812612abd3dfe174185d204dac. Fresh inputs
must derive the full six-field sql_query object from this loaded QA source.

## Bounded source-feed diagnostics

Each completed exchange records agent, safe attempt identifier, stage, status,
elapsed/deadline metadata and bounded file/function/line frames on failure.
Exception messages, locals, SQL rows, request bodies and keys are excluded.
Capacity refusals are explicit. These records grant no proof or hash authority;
the SourceFeed2 frames, signing preimages and protocol deadlines are unchanged.
Loss of the diagnostic sink poisons further collector use rather than silently
certifying complete observation.

## Earlier real genesis observations

The reviewed Runtime starts node observers, then synchronously calls
publish_operator_urls(), then starts its HTTP server and execute_phases().
Its operator_observation records therefore form an existing pre-phase milestone
in the same ordered EvidenceStore stream. The real RPC tip sampler records
getbestblockhash/getblockheader/getblockhash observations as canonical_at.

For b17, node-a/node-b events 27/28 record the expected height-zero tip before
operator-publication events 31/32. Pre-HashStart CommandLog RPC results also
commit to node-a's exact genesis-tip reply. These are earlier observations,
not a later chain's inevitable genesis ancestor. The original b17 fresh_chain
receipt remains INCONCLUSIVE and is not rewritten.

For new runs using the reviewed 98f source profile, the verifier can use this
earlier evidence when QA's direct first common sample arrives after height zero.
It requires a complete same-run event prefix, exact node/front/marker census,
the actual RPC sampler origin/shape, both expected genesis tips before the
pre-phase marker, the original bootstrap/HashStart and correctly bound pre-
HashStart reply commitments. A late sample, missing marker, wrong source,
wrong node or changed commitment is refused. No bootstrap ID/deadline changes,
new hash source, authority override or additional producer hook is introduced.

The public regression fixture is a derived projection: relevant original
genesis/marker records and the pre-HashStart command prefix, with unrelated
evidence payloads replaced by digests. It is not a complete original run export.

Full native, payout, witness, funding, Ark renewals/offboard and exact final
drain/EOF/SQL audit still require the next actual qualified run. Global EACCES
coverage and older lost-evidence classifications are not upgraded.
