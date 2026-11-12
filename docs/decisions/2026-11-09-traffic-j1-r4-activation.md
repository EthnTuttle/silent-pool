# 2026-11-09 - Atomic J1 r4 selected-consumer activation

## Authority and scope

Explicit parent/user authorization activates the frozen canonical producer r4
consumer join. Workflow approval holds were removed by the user. This record
does **not** claim a Fable approval of r4 or relabel the historical r2 verdict.
The runtime plan remains `44daaa2b...`; control v1, journal/history 3,
derivation 2 and all audited wallet write/rollback algorithms remain unchanged.

## Exact input binding

- Producer r4 full `71b4ee6f57a16a0451af669f594491a8623b5eea036a558f61139b9a5bca5351`,
  code `22fd2fcdbd486191f3935738a1625ec9c5333d1068554046ab5b9f11cd180d70`.
- Canonical contract `754e1e6131955f9e5bbce7ef81b6f9aabaa2f1379ec6409f1c3a7a9e88be91f9`.
- Frozen production projection row `c194c9817e8fe500e9edb339353492dc3208048c8a5d3abbc53b60320df63de0`;
  focused producer test row `cc88be6f8caff2c24c9671871cba362a3a5b1c7b6737a5f8b55e87234d927752`.
  All 616 producer code rows and modes were verified before consumption. The
  report-only r5 change was not confused with a producer code change.
- Actual immutable metadata node `36e2387ada164e2c89b3d20f5d515bf19c33f30fb927e8ca11621eba3658d7d9`;
  source full `f1592c935e29b0b1fdf6429ad1b84e9ae925e9a9fcccd66b04f3d090680ff20f`.
- Actual canonical default/compat report hashes are respectively
  `4b12400733335ba11192b0646a8e8cd3bfe16c7fcbcfe8c58fbce1aee88052f0` and
  `98595842284f331d9069f5edc41d919f303b404a132594441a6330d9883bf4c3`.
  The node handoff records CLI/schema success, identical consensus facts,
  policy-sensitive normalized launches and reaped process identities. Those
  reports are read-only, height-zero evidence, not funded rejection proof.

## Atomic decision

1. The currently accepted lineage is exactly 754e1e61 above, paired with the
   canonical 20-row table, exact id/method/coverage/unknown-policy bindings and
   `spc-lab-tx-rpc-consumer/1`. The old approved f4fc3e04 17-row contract is
   historical, unsupported by this current consumer; raw reports and its
   original decision stay byte-unchanged. Old hash/new table and new hash/old
   table remain closed. Historical approval gets a distinct diagnostic.
2. `getmempoolinfo []` consumes only integer `size`. The full corepc-v30 model
   still records `shape: maxdatacarriersize: expected JSON integer, got null`.
   Its complete failed row is retained verbatim in consumer_model_failures,
   and its failed axes remain in check_states without gating the size-only
   consumer. Missing or inconsistent consumer/diagnostic metadata rejects.
3. The three added chain-consumer checks cover block-bound raw transaction
   lookup, header fields and best-block hash. Raw and default-verbose request
   forms remain distinct. Missing/duplicate ids, wrong methods, failed required
   rows, unapproved/unknown hashes and forged funded proof remain closed.
4. Read-only recognition never sets behavior_proven or full_acceptance. Actual
   fresh T1 startup still compares every launched node's binary, finalized
   argv/environment, normalized policy, consensus and genesis against a newly
   executed canonical probe. A packaged release with another hash must rebind.
5. The exact definitive-send-rejection catalog is independent and unchanged.
   Metadata/probe success does not enable terminal rejection rows.

## Tests and integration

The joined runtime consumes the producer's projection/test hunks narrowly,
preserving history-3, actor allocation, bootstrap execution limits, runtime,
operator and expanded-access changes. Only activation-specific candidate
expectations change from refusal to acceptance. All negative contracts stay.

Its source-bound gate must run production projection tests, the focused J1
fixtures, explicitly enabled immutable actual-report tests, and unchanged
runtime profile/consumer tests. Producer Rust checks were unrun at handoff;
joined test receipts are evidence for this joined source, not a claim that a
separate producer gate ran. Fresh-process acceptance is separately reported.
The matching one-bullet SPEC fragment is for the serial integrator; no direct
SPEC frontmatter edit is made here.
