# Public conformance inputs

- `native-evidence-v1/`: the original genuinely Schnorr-signed SYNTHETIC
  retention vector. `generate.py` reproduces all five original hashes from
  existing public test keys. `/unit` names and PID 1234 are fixture metadata,
  not paths/processes to adopt. No live wallet secret is included.
- `readiness-v3/directed-dial-proof.json`: exact producer v3 structural fixture,
  SHA256 `ec4a81f78fa5a921489ed91cd6625a06ca8a54cfe19c5ea4e2f590d854decdf8`.
  Its command ID is null; its Nix path is a string in a synthetic observation,
  not an executable prerequisite. It cannot qualify readiness for a campaign.
  The one readiness parser is owned by `testing/lab-ark-funded/readiness_v3.py`
  in the AP5 intake. Do not create a second parser or coordinator here.
- `contracts/FULLRAIL-OBSERVATIONS.md`: producer public v1 schema definitions,
  SHA256 `32b7d43f637a99af72f5b76acb91cde2f237ce6e7c283befa1177a0154f8f3a2`.
  `test_decision_observations.py` constructs deterministic public SQL-envelope,
  grant and SPAC/SPCA examples. Executed SQL dumps are not source fixtures.
- `verifier-source-policy.json`: exact public source/size/executable-bit policy
  for the already-approved stock VTXO verifier. The AP5 owner supplies that
  verifier's source package and Nix recipe; this directory adds no crypto code.

Private historical campaign archives, SQL exports, credentials, actual wallet
data and captured runtime tokens are deliberately external to this source tree.
