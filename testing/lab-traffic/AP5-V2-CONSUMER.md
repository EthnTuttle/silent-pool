# Independent AP5 v2 campaign consumer

Latest inspected contract: `/tmp/opencode/AP5-verifier-build-r1/handoff.json`,
CONTRACT SHA256 `9cb6915a3ff8a0500e42c0846ccf3c0642fd4061c66c0701fd36e660cd85beda`.
The retained-board contract `d5040b82...` and its explicit `88ff7b24...`
predecessor remain preserved and parseable as historical handoffs.
The frozen verifier Cargo.lock is
`ac24ee3b09c7c2dd8731417672b9d7b072aa6e9e04127b9c96e8ad80ae842833`.

The campaign accepts `--ap5-seal <seal.json> --ap5-seal-sha256 <external pin>`.
`ap5_v2.py` independently reads the actual result/v2, component-verdict/v2,
raw-component/v2 and cleanup/v2 records. It checks the external seal, full
event/artifact chain, component/dependency census, separately sealed active
resource census, tool exits and captured cleanup consistency. It does not
import or execute the AP5 producer's Python verification helpers.

Raw Bitcoin checks use this consumer's decoder: exact txid/wtxid, PoW/Merkle/
witness commitments, actual BIP34 heights, complete prevouts, maturity and fees,
and agreement of both node observations. ArkInfo bytes, policy keys and fee
schedule hashes are crosschecked. Component-specific checks cover funding
scripts/amounts, board fee and net amount, no-op versus actual boarding, 115M
float target and expiry horizon, signed refresh checkpoint bytes, automatic
renewal identity/round selection, and offboard command/destination/raw bytes.

The compiled verifier is now admitted and wired into component/archive replay
through `ap5_verifier.py`. It executes only the exact Nix binary under the closed
artifact configuration, after source/deriver/lock/test-receipt checks. Requests
and replies bind the exact public bytes and expected full-policy fields; call
receipts retain input/output hashes and exit/timeout results.

Board original ID/full anchor, current float ID/value/expiry, payout attribution,
refresh inputs/children/value, automatic renewal identities/expiry, and offboard
input/value/destination now join to compiled proof results. A bundle must bind
the exact verifier artifact object; merely having a verifier installed is not
a component binding. Same-run canonical ancestry and other acceptance gates
remain INCONCLUSIVE where their actual evidence is missing. No second Rust
verifier or remote pruned-proof lookup is introduced here.

Joined validation checks require the six actual Ark test/build and compiled
UI/browser purposes. Default-feature agent tests, no-run/list-only checks,
missing browser cases or mismatched compiled binaries do not count. Actual
Nix source/lock admission and corrected aggregate/pool-control policies remain
required independently of those check receipts.

The owner's six unconditional Rust crypto cases **passed Cargo and Nix**: six
unique cases, not twelve. Nine actual Nix source rows, the binary/deriver,
installed unchanged lock and nine compiled receipt hashes were independently
verified. The valid-signature HTLC fixture is a full-policy negative; its public
bytes and production verification were not changed. Two CLI protocol cases
also passed. Synthetic conformance is not a live corpus. The first actual funded board must
retain its original full proof and anchor. Historical proofs need not remain
currently spendable. Retrieval now uses the tested same-wallet `board_proof_v1`
interface; no remote GetVtxo or current key-row lookup is used for the original.

## Retained-board proof consumer

`ap5_retained_board.py` consumes the exact tested control handoff
`pool-wallet-control-proof-handoff-r3.json`, SHA256
`3b341bfc0b000994d02083cb98244bc180afc29ca08b75b08101cfddb4905531`.
Producer source is `dd9f0c37...`, tested code `8ec102f5...`. This retained r3
field contract and the later R4 source approval do not establish joined
grant-backend or funded acceptance.

The read ID must differ from the immutable original board ID; money fields are
forbidden on the read. Historical replies have null current balances, time,
tip and effect fields. Typed missing/unavailable remains INCONCLUSIVE. Legacy
Confirmed replies are parseable, but absence of the retained digest cannot
establish proof and the old terminal cannot be rewritten to add it.

The original envelope, nested request/board and terminal context are serialized
in the declared Rust struct order, not sorted-map order. The context must equal
the actual original Confirmed receipt after omitting **only**
`retained_proof_sha256`; that removed value must match the envelope hash. Every
other observation, amount, operation, policy and board field remains bound.
The exact original full VTXO, raw anchor, owner/full Pubkey policy and server
are joined to the component's public proof inputs; compiled VTXO validation
is still mandatory and not supplied by envelope availability.

Archive replay also joins the separately sealed request/reply events. It
reconstructs the actual executor argv hash using the preserved request file and
pool config path, and its serde stdout hash including the newline. A read
cannot silently switch wallets, reuse a mutation ID, replace an original
terminal or borrow an unrelated successful tool result. Component copies must
match those exact event histories.

Partial lineage remains explicitly incomplete: at most 256 movement records,
sorted unique movement/input/output IDs, no conflicts, and explicit ID paths
from the original. Amount equality is not ancestry. An unavailable lineage may
coexist with a valid retained original; neither case substitutes for the
separate current float snapshot and full VTXO set. The provided NOT-FUNDED
producer example is used only to verify these byte/field bindings.

The real sealed AP5 prepare envelope was read-only replayed at
`/tmp/opencode/T3-source-feed-checkpoint-AP5-r6/AP5-real-prepared-envelope-replay.json`.
It correctly remains INCONCLUSIVE under that historical freeze. Current QA
executes the admitted Rust CLI only on public proof requests; no funded run is
inferred from those calls.

## Compiled artifact

Artifact: `/tmp/opencode/AP5-verifier-build-r1/vtxo-verifier-artifact.json`, SHA256
`8aa3f792529a46dc73983af9265f8caa23b1ac79ea522bb30f8a1028329e5d34`.
Binary SHA256: `5330c3ce8eaa65a4b96076badbeae420f654c33c2a611927932fab49bb8357dd`.
Actual source: `/nix/store/3grmjmz22whyp9armd4lqpy28idr5346-source`.
The wrapper performs no wallet, signer, VTXO decoder or network operation.
Synthetic stock anchors pass the stock crypto verifier but are not necessarily
valid mined Bitcoin transactions; actual component inclusion/maturity checks
remain mandatory and are not weakened for conformance fixtures.
