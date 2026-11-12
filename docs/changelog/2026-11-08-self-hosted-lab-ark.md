# SPEC merge fragment: self-hosted lab Ark (2026-11-08)

Insert into SPEC frontmatter `changelog`:

```yaml
  - "2026-11-08: the lab Ark deployment is self-hosted and Nix-built from stock Bark c419159fa54085572f8ca72d2ba1716556a9d3eb. The run owns one provider Postgres cluster and separate per-pool Ark wallets/processes, discovers and binds real ArkInfo before front startup, and records initialized_unfunded separately from funded validation. Hosted-provider fallback is forbidden in the lab. Production independent-server and identity-stream boundaries are unchanged. See docs/decisions/2026-11-08-self-hosted-lab-ark.md."
```

Append to SPEC §9's lab deployment discussion:

The expanded lab MUST build its own stock pinned Ark server and tooling through
Nix, and own its storage, wallets, private configuration and process lifetime.
It MUST NOT fall back to a hosted Ark provider. The lab's provider database MUST
be independent of every pool database, and each pool MUST retain a separate
Ark-role wallet/process and the §11.3 identity boundary. Startup MUST bind
actual ArkInfo server/mailbox keys and policy before rendering front delivery
metadata. RPC health and initialized wallets MUST NOT imply funded readiness:
native funding, boarding, receive, refresh and offboard require their own real
transaction evidence. An infrastructure-only run MAY remain initialized and
unfunded while the native funding and registered-agent runtime are integrated.
