# Durable native evidence - assigned-turn results

Status: targeted CPU tests/checks passed; final source and binary are bound by
`/tmp/opencode/native-evidence-tested-r1/handoff.json`. This is a retention
component result, not controller/full-history/funded campaign acceptance.

## Executed scope

33 distinct tests passed at their original source-bound receipts:

- Seven authored retention cases: exact signed record/manifest/handle parsing,
  private paths/NOREPLACE, rotation budgets, durable-before-publication plus
  replay/conflict/missing bytes, I/O and record/byte budget failure, blocked
  retention supersession/stop, and original-deadline expiry during blocked I/O.
- Eight N1/N2 regressions, including actual partial-write backpressure,
  stop-all before joins, original monotonic deadline and no expiry resurrection.
- Eleven native asynchronous/configuration cases over the real Noise path.
- Seven control/event-schema cases affected by the new optional locator field.

CPU all-target check and strict clippy pass. The actual controller render/bind
library also passes check and strict clippy. Formatting/whitespace checks pass.
The older runtime unit-test pool-dev-dependency limitation is not relabelled;
the corrected primary union must execute its own controller lifecycle suite.

Logs, preflights, before/after source manifests and exit receipts are retained
under `/tmp/opencode/native-audit-build-turn/retention-*` and indexed in the
final handoff. Every heavy invocation held the nonblocking global lock, checked
this session's ticket and actual target/store free space, and reused T1a with
`CARGO_INCREMENTAL=0`. No further cleanup, restore or garbage collection ran.

## Corrections and interface stability

Strict clippy flagged an explicit parity check in the QA-bound public module.
A documented module-level style-lint allowance preserves the exact `017fa69...`
file; no type, schema, validation, config or digest change was made. The new
test-only artifact exporter initially returned a boxed event where an owned
event was expected; this failed compile and its source are retained, then fixed
by unboxing. The affected durable-record case passed again after that fix.

Public `native_activation.rs` remains `39c75d44...`, source-feed/2 remains
`612ec34e...`, and QA's `native_evidence.rs` remains `017fa69...`. No QA schema
rebind is needed. Prior approved ccf53/7b622 artifacts retain their old scope.

## Actual produced component artifact

`/tmp/opencode/native-evidence-actual-component-r1/receipt.json` binds a record,
manifest, locator, handle and observed events emitted by the executed retention
writer/protocol/hash-worker test. The captured record SHA256 is
`2dd4666ed443d3577edb73da111750135f489d534a77ae1319812147a8579108`.
The request signing digest is
`49d54b6fe00ccd6e2dfa985d5bbd8dd71c84f84ae382afdc9d393574a0afa98b`.
Before capture, attempted hashes were zero; commit/activation/first-hash event
IDs were 3/4/5. The existing independent native verifier accepted the exact
retained pair at its recorded verification time, and all file/payload hashes
were checked. This uses signed simulated wallet responses and fixture startup
metadata; exported paths record the original test capture location, not a live
controller archive. It must not substitute for production campaign evidence.

## Primary/Nix packaging

1. Apply the original retention delta from the approved ccf53 packet, then the
   small tested followup delta. Use the captured-primary runtime hunk for the
   controller side; reconcile it with the current primary instead of replacing
   primary with this older focused workspace. Keep funding/registration barriers.
2. Include both new Rust modules, their module declarations, CPU rustix Cargo
   edge/lock entry, optional MinerConfig field, main/store/gate/event plumbing,
   runtime initial/dynamic renderer, Snapshot capability check and private
   manifest/process validation. Full lab audited profiles require
   `native_public_evidence_durable_v1`; omission is a rejected precondition,
   never a fallback to event-only evidence.
3. Produce a fresh bounded source-only union snapshot. Exclude `.git`, every
   target directory, generated caches and runtime/evidence datadirs; record
   paths/modes/hashes and total bytes before any Nix `path:` ingestion. This
   packet's source archive is about 11.5 MB and includes test fixtures, not
   captured runtime data. Include new untracked modules when preparing the union.
4. Build the existing CPU/package or joined lab-tools derivation from that
   exact snapshot under the parent's resource guard. Keep upstream pins intact;
   resolve the exact union's fetchCargoVendor hash if Nix reports a change, not
   an unrelated packet's stale hash. This turn used the existing Nix devshell
   plus locked/offline Cargo; it did not execute the final primary Nix build.
5. Record the resulting Nix binary's own SHA256/source closure in the joined
   run manifest. It is a separate artifact from this debug CPU seal. Render
   `[native_evidence]` into the actual private miner config and preserve the
   emitted process capture directories in the final QA archive.

The later wallet joint test must pin the new CPU hash explicitly. Neither the
old joint success nor this retention component rerun counts as the eventual
full primary/controller/wallet/collector/funded gate.
