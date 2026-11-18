# 2026-11-17 - Testnet5 stack: adopted rust-bitcoin/bark forks, self-hosted captaind permission

**Trigger.** User directive to run silent-pool end to end on public BIP95
Testnet5. The blocker recorded in decision 2026-11-15-testnet5-chain-identity
item 8 and in UPSTREAM_DEPS.md "bark testnet5 tracking (2026-11-17)" is purely
upstream availability: no released `bitcoin` crate and no bark revision can
represent `Network::Testnet5`. Both gaps are now closed by live, validated
forks owned by this project's operator; no upstream movement has occurred
(rust-bitcoin PR #7160 still open, bark has zero testnet5 refs).

## Decision

1. **Adopt two network-identity-only forks** (full records and the
   re-convergence plan in UPSTREAM_DEPS.md "Adopted forks (2026-11-17)"):

   - `github.com/EthnTuttle/rust-bitcoin` branch `testnet5-0.32` @
     `cf4d11ca778fd86698ec7cc00bd081c15b12a2de`: bitcoin 0.32.102 plus
     `Network::Testnet5` ported from upstream PR #7160 (genesis
     `00000000000008c1637e7d3252dc1c333d4546fd6fff6a474b200d674a91d497`,
     magic `0x46495645` "FIVE", HRP `tb`, canonical name exactly
     `testnet5`). The whole rust-bitcoin workspace test suite passes on
     the fork.
   - `github.com/EthnTuttle/bark` branch `testnet5` @
     `8c38e07ee70b046529a416b170f1ae5c77cc66a0`: stock
     `c419159fa54085572f8ca72d2ba1716556a9d3eb` plus three commits -
     `[patch.crates-io] bitcoin` -> the rust-bitcoin fork,
     network-identity-only testnet5 support in bark/captaind/barkd/
     bark-json/lightning, and the regenerated lock. 705 unit, 48 regtest
     round/arkoor/board and 14 mailbox/registration integration tests are
     green on the fork; zero SPc-specific and zero behavior changes on
     existing networks.

   The workspace `[patch.crates-io]` maps `bitcoin` to the rust-bitcoin
   fork (pinned by rev, immutable) and `ark-lib` to the bark fork; the
   `bark-wallet`/`server_rpc` git dependencies in `spc-pool`/`spc-agent`
   and the `spc_ark_e2e` harness move to the same bark SHA so `ark-lib`
   crate identity (SPEC references in UPSTREAM_DEPS.md row `c419159`)
   is preserved - all bark crates move together, in one change with both
   lockfiles.

2. **Vanilla-interop posture is preserved, not weakened.** AGENTS.md
   requires silent-pool to work against unmodified captaind through
   standard interfaces, with SPc logic confined to pool/client adapters.
   The adopted bark fork IS stock behavior plus a network identity: it
   adds the `testnet5` network name and its parameters and nothing else -
   no SPc-specific registration, derivation, wallet handling, RPC or
   protocol change. On every previously supported network the fork is
   behavior-identical to `c419159` (its full stock test suite passes),
   and the regtest `spc_ark_e2e` gate runs against the forked `captaind`
   to prove the server still behaves stock. SPc-specific logic stays in
   the adapters.

3. **Code gates lifted, chain-exactly.**
   `ChainNetwork::stock_bitcoin_network()` now returns
   `bitcoin::Network::Testnet5` (the `UnsupportedArkNetwork` error is
   gone - every recognized chain has a stock backend on the forks). The
   agent's `WalletKeys` adapter gains the explicit-chain backend selector
   decision 2026-11-15 item 8 demanded: `KeySources`/`WalletSettings`
   carry the explicit `ChainNetwork`, the bark wallet/persister and every
   `require_network` call use its exact backend (`Testnet4 -> Testnet4`,
   `Testnet5 -> Testnet5`, never a legacy alias), and the persistent
   store identity labels explicit Testnet4/Testnet5 stores chain-exactly
   so a store bound to one test chain never reopens on another. This
   fixes the Testnet4-refusal workaround ("WalletKeys still stores the
   old encoding selector") properly for both chains. Without an explicit
   chain the historical mapping stands unchanged (encoding `testnet`
   means the legacy Testnet3 backend; existing store labels unchanged).
   The spc-agent main.rs Testnet5 refusal and config/`WalletSettings`
   rejections, and the spc-pool config Testnet5 ark rejection, are
   removed.

4. **Self-hosting permission extends to testnet5; chain checks do not
   relax.** On testnet5 there are no third-party Ark servers; running the
   forked `captaind` ourselves is the only option. `ark.server_is_self`
   (SPEC §11.3 server choice, §11.9 negative matrix, §11.10) now permits
   `network ∈ {regtest, signet, testnet5}`. The trust posture on testnet5
   is explicitly sender-equals-server (the arkoor "at least one honest
   party" assumption is void) and is a test-network posture only, never a
   production pattern. Deliberately NOT extended, because testnet5 is a
   real public chain with a real genesis to check: `template.source =
   "mock"`, `accounting.allow_unchecked_chain` and `[ark] transport =
   "command"` stay regtest/signet-only; the Testnet5 genesis/BIP54
   template checks (decision 2026-11-15 items 3-4, `chain_identity.rs`)
   stay on.

## Decision 2026-11-15 item 8 gates, reconciled one by one

- "Ark enabled, effective Ark role and explicit Testnet5 wallet requests
  fail before state or RPC effects": LIFTED - the premise ("stock Bark
  c419159/rust-bitcoin 0.32.9 cannot select Testnet5") no longer holds on
  the adopted forks, and the wallet contract now carries chain identity
  (clause 3).
- "No encoding alias": PRESERVED - the backend is selected from the
  explicit `ChainNetwork`, never from HRP/tprv encoding; a chain/encoding
  disagreement fails closed (`ChainEncodingMismatch`).
- "No dependency bump ahead of upstream": superseded deliberately by this
  record, per the UPSTREAM_DEPS.md adoption-path rule that a re-pin moves
  pins, lockfiles and docs in one change citing a decision. Upstream has
  not moved; the forks are project-owned and the re-convergence plan is
  recorded.
- "No RPC spoof": PRESERVED - the forked node/wallet speak `testnet5`
  natively; no label rewriting anywhere.
- "No server fork or self-host exception": superseded by clause 4 within
  the stated bounds (network-identity-only fork; self-host on test
  networks without third-party servers; mainnet/testnet4 unchanged).
- "Agent Testnet5 execution remains unavailable until wallet and
  authenticated job/source contracts carry chain identity": the wallet
  contract now carries it (clause 3). Job/source chain identity remains
  as built in the dev/testnet5 integration: the pool binds every
  template/RPC endpoint to the configured genesis and requires active
  BIP54 facts on Testnet5 (`chain_identity.rs`); the agent inherits the
  pool's checked chain. The main.rs refusal is lifted on that basis.
- "Explicit Testnet4 wallet refuses rather than selecting legacy
  Testnet3": replaced by correct behavior - explicit Testnet4 now selects
  the Testnet4 backend (clause 3).

## Preserved policies

SPEC §3.4/§4.1/§9 v2 remains regtest-only. Fail-closed parsing, the §11.3
privacy boundary (no identity fields to the `ark` role), genesis binding
before endpoint use, BIP54 Testnet5 template facts, and the frozen
prototype vectors (spc_ark/spc_ark_mailbox/spc_ark_client stay at stock
`c419159`) are unchanged. The lab's stock-server interop evidence remains
buildable: the Nix `arkPackages` (`nix/lab/ark.nix`, gitlab `c419159`) is
untouched; the forked build is the separate opt-in `captaind-testnet5` /
`bark-testnet5` packages (`nix/lab/ark-testnet5.nix`). No deployment or
complete-suite readiness claim; lab adoption of a live testnet5 service
remains separately authorized.

## Sections

SPEC §11.3 (server choice), §11.9 (negative matrix) edited in this packet.
§9 (topology; Testnet5 staging paragraph), §11.5 (unsupported-new-chain
runtime paragraph) and §11.10 (`server_is_self` / agent `network` comments)
ship as exact hunks in the changelog fragment
`docs/changelog/2026-11-17-bark-testnet5-fork.md` for the merge session.
UPSTREAM_DEPS.md gains "Adopted forks (2026-11-17)" and the bark pin row /
tracking section updates.

## References

- UPSTREAM_DEPS.md "Adopted forks (2026-11-17)", "bark testnet5 tracking
  (2026-11-17)"; decisions 2026-11-15-testnet5-chain-identity,
  2026-11-02-bark-adapter, 2026-10-14-upstream-own-suite.
- rust-bitcoin PR #7160 (upstream source of the ported Testnet5 support);
  BIP95; Core `ee505609` `CTestNet5Params`.
