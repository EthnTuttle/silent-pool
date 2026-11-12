# Published full-bundle verifier admission

The single parent-verified compiled verifier is now wired into
`common.check_bundle(..., full=True)` via `verifier_admission.py`.
No second verifier, compilation, cryptographic test rerun or mutable-worktree
source inference is involved.

Use the contents of this artifact descriptor as the full bundle's
`vtxo_verifier` value:

`/tmp/opencode/AP5-verifier-build-r1/vtxo-verifier-artifact.json`

SHA256: `8aa3f792529a46dc73983af9265f8caa23b1ac79ea522bb30f8a1028329e5d34`.

The minimal accepted descriptor contains its exact `path`, `sha256` and
`cargo_lock_sha256`. Optional published descriptor fields must match the
approved values exactly; a different source/deriver/receipt label is rejected.

- Binary: `/nix/store/kyq7kv5wb38353l2abp2iiwy0hf01yq9-ap5-vtxo-verifier-0.1.0/bin/ap5-vtxo-verifier`
- Binary SHA256: `5330c3ce8eaa65a4b96076badbeae420f654c33c2a611927932fab49bb8357dd`
- Actual source: `/nix/store/3grmjmz22whyp9armd4lqpy28idr5346-source`
- Frozen lock SHA256: `ac24ee3b09c7c2dd8731417672b9d7b072aa6e9e04127b9c96e8ad80ae842833`
- Published CONTRACT-v2 SHA256: `9cb6915a3ff8a0500e42c0846ccf3c0642fd4061c66c0701fd36e660cd85beda`

`verifier-admission.json` pins the parent-verified handoff and that contract.
Admission verifies the actual Nix deriver/src and all nine frozen source rows,
installed source/fixture/lock bytes, all nine compiled receipt/log seals, six
unconditional Cargo and Nix test results, and the two installed-CLI case
receipts. It does not compare against whatever currently happens to be in
`testing/lab-ark-funded/verifier/`.

The metadata tests include a guard that refuses any read from that mutable
verifier directory while admitting the real artifact; wrong binary/lock/
source/receipt labels and changed actual derivation source are rejected.
These tests read metadata only and do not execute the verifier again.

This satisfies only the compiled-verifier component of full admission.
Corrected joined application source, R4 control, peer readiness, native audit,
Rule B/aggregate and live full-proof requirements remain separate. In
particular, this descriptor is not added to the no-control native diagnostic,
and the failed A1/B0 P2P case is not a funded node or Ark positive.

The new producer peer-ready/event/graph contract has not yet been provided.
No field names or acceptance shortcuts are guessed. The existing source
guards remain checked; observed peer readiness will be required according to
that exact contract when supplied, before another fresh live attempt.
