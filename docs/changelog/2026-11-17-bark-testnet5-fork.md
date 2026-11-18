# Changelog fragment: 2026-11-17 bark/rust-bitcoin testnet5 fork adoption

Splice under `changelog:` (newest first), then delete this file:

- "2026-11-17: testnet5 stack adopted onto project-owned network-identity-only forks (user-directed): bitcoin 0.32.102 -> github.com/EthnTuttle/rust-bitcoin @ cf4d11ca778fd86698ec7cc00bd081c15b12a2de (Network::Testnet5 ported from open upstream PR #7160) via [patch.crates-io], and the bark pin -> github.com/EthnTuttle/bark @ 8c38e07ee70b046529a416b170f1ae5c77cc66a0 (stock c419159 + testnet5 network identity only; all bark crates move together, ark-lib crate identity preserved). stock_bitcoin_network() gains the Testnet5 backend; the agent WalletKeys adapter selects explicit Testnet4/Testnet5 backends chain-exactly (chain-exact store labels, legacy mapping only without an explicit chain), lifting both the agent Testnet5 refusals and the pool ark-role Testnet5 rejection from decision 2026-11-15 item 8. §11.3/§11.9/§11.10: ark.server_is_self extends to testnet5 (no third-party Ark server exists there; sender-equals-server test posture, never production); template source \"mock\", accounting.allow_unchecked_chain and [ark] transport = \"command\" stay regtest/signet-only and the Testnet5 genesis/BIP54 checks stay on. §9 topology may co-locate ark on testnet5. Vanilla-interop posture preserved: the fork is stock behavior plus a network; SPc logic stays in adapters; stock-pin lab evidence (nix/lab/ark.nix at c419159) remains buildable beside the opt-in captaind-testnet5/bark-testnet5 packages. Upstream re-convergence plan in UPSTREAM_DEPS.md \"Adopted forks (2026-11-17)\". See docs/decisions/2026-11-17-bark-testnet5-fork.md."

## Merge notes: SPEC body hunks outside this packet's §11.3/§11.9 edits

The §11.3 (server choice) and §11.9 (negative matrix) body edits are already
on this branch. The following three hunks touch sections this packet does not
own outright; apply them at merge (exact old -> new text).

### Hunk 1 - §9 Deployment, "Testnet5 staging" paragraph

Old:

```
**Testnet5 staging (draft 2026-11-15).** Identity support is not deployment
authorization. Future integration requires owner lab approval, BIP54 template/
coinbase support, separately approved public-network work/audit policy and
supported stock Ark. v2 stays regtest-only. Old unbound SQL/Redis state is held
for explicit migration, never automatically adopted by the staged source.
```

New:

```
**Testnet5 staging (2026-11-17).** Identity support is not deployment
authorization. The Ark/wallet backend gap is closed by the adopted
network-identity-only forks (UPSTREAM_DEPS.md "Adopted forks (2026-11-17)";
decision 2026-11-17-bark-testnet5-fork.md). Future deployment still requires
owner lab approval and the separately approved public-network work/audit
policy. v2 stays regtest-only. Old unbound SQL/Redis state is held for
explicit migration, never automatically adopted.
```

### Hunk 2 - §9 Topology sentence (ark co-location)

Old:

```
dev/testnet runs `--roles front,payout,api` in one process, and MAY add `ark` to it on regtest/signet only;
```

New:

```
dev/testnet runs `--roles front,payout,api` in one process, and MAY add `ark` to it on regtest/signet/testnet5 only (testnet5 has no third-party Ark server; decision 2026-11-17-bark-testnet5-fork.md);
```

### Hunk 3 - §11.5, "Unsupported new-chain runtime" paragraph

Old:

```
**Unsupported new-chain runtime (draft 2026-11-15).** The agent recognizes an
explicit Testnet5 selection but refuses execution until job/source and wallet
interfaces carry real chain identity. Pinned stock Bark has no Testnet5 backend;
encoding compatibility MUST NOT select another testnet on its behalf. Current
WalletKeys also refuses explicit Testnet4 rather than selecting its old
Testnet3 backend. Existing wallet schemas/labels/derivations are not relabeled.
```

New:

```
**Explicit new-chain runtime (2026-11-17, supersedes the 2026-11-15 draft
refusals).** On the adopted forks (decision 2026-11-17-bark-testnet5-fork.md)
the agent's wallet contract carries chain identity: an explicit `[ark]
network` chain selects its exact stock backend (Testnet4 -> Testnet4,
Testnet5 -> Testnet5) and a chain-exact store label, so a store bound to one
test chain never reopens on another. Encoding compatibility still MUST NOT
select a chain: without an explicit chain the historical mapping stands
(encoding `testnet` means the legacy Testnet3 backend) and existing wallet
schemas/labels/derivations are not relabeled. A chain whose encoding
disagrees with the SP selector fails closed.
```

### Hunk 4 - §11.10 configuration comments (two lines)

Old:

```
server_is_self = false                    # true only on regtest/signet
```

New:

```
server_is_self = false                    # true only on regtest/signet/testnet5 (§11.3; decision 2026-11-17-bark-testnet5-fork.md)
```

Old:

```
network = "regtest"                       # explicit chain; Testnet5 is recognized but runtime unsupported
```

New:

```
network = "regtest"                       # explicit chain; chain-exact wallet backend (testnet4/testnet5 supported on the adopted forks)
```

### Hunk 5 - docs/OPERATIONS.md intro paragraph (merge-session-owned file)

Old:

```
Draft Testnet5 integration: SPEC §2.1/§5.1/§5.3 requires exact chain identity,
known endpoint genesis and matching SQL/Redis bindings before startup. This
staged source holds old unbound data for reviewed migration; it is not an
upgrade procedure for the lab. Use a dedicated fresh deployment tree/database
only after owner lab approval and separate deployment authorization. Stock
Testnet5 Ark and agent runtime remain unsupported; v2 stays regtest-only.
RPC scanner find now requires `--network <canonical-chain-name>`; address HRP
cannot select it. Offline file/proof recovery makes no chain ancestry claim.
```

New:

```
Testnet5 integration: SPEC §2.1/§5.1/§5.3 requires exact chain identity,
known endpoint genesis and matching SQL/Redis bindings before startup. This
source holds old unbound data for reviewed migration; it is not an upgrade
procedure for the lab. Use a dedicated fresh deployment tree/database only
after owner lab approval and separate deployment authorization. The Testnet5
Ark/agent runtime runs on the adopted network-identity-only forks
(UPSTREAM_DEPS.md "Adopted forks (2026-11-17)"; decision
2026-11-17-bark-testnet5-fork.md); v2 stays regtest-only.
RPC scanner find now requires `--network <canonical-chain-name>`; address HRP
cannot select it. Offline file/proof recovery makes no chain ancestry claim.
```
