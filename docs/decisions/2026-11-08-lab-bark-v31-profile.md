# 2026-11-08 - Lab bark-v31 opt-in Ark development node profile (AP-1b parent adoption)

## Status and authority

This is the **parent lab adoption decision** for the AP-1b r1 rbitcoin
named RPC compat profile candidate. It is distinct from the provider
approval: the Fable review
`docs/reviews/2026-11-08-bark-v31-profile-fable.md` (APPROVE, bounded)
approved the exact candidate bytes as a review input and explicitly
authorized the parent to take the candidate integration decision for the
lab; it did **not** approve any runtime replacement of the approved L2
node. This record is that parent act, and nothing in it claims Fable
approved a runtime replacement.

Parent authorization (recorded verbatim in substance): the CLI/conf-only
bark-v31 candidate may be prepared as a **separate, opt-in Ark
development node profile** - NOT a replacement of the standard L2 node,
its pin, or the v0.9 adoption.

## Exact bindings adopted

- Node patch `testing/rbitcoin-bark-v31/node-candidate.patch`, sha256
  `97f84755b8ca470a3cc64145649eafae732913c2e2a02a116abfbe855fbb2a95`
  (939 lines, 14 files).
- Candidate source manifest aggregate
  `f8a5111119d9e0df79cc8987f23d4b380c6ecad2df094b4c8196b2e2b04ba6d3`
  (25 records over base `e8cf9c9d61f7c63ae59cc11ab1179cace50ca142`).
- Candidate binary (built from the candidate tree, not the integrated
  tree) `b75dad84bdd6663673a29c07352d10c4f78d2dcd14e885f62ad3209d214443e4`.
- Packet freeze: the 82-record
  `/tmp/opencode/AP1b-r1-full-review-manifest.json`, aggregate
  `9375653ca90158446b99ae5c02aaee8482153984c68ba53aef447b115df40050`,
  transferred byte-exact into `testing/rbitcoin-bark-v31/` plus the two
  packet-listed implementation reports
  `docs/reviews/2026-11-08-bark-v31-profile-implementation{,-r1}.md`.
- Prepared integrated source tree:
  `/tmp/opencode/AP1b-lab-integration` - a fresh copy of the approved L2
  tree (`e8cf9c9d...`, snapshot
  `d2f37b9113e8855245866289ed393043aed6e6153e8101375072cbb23624d499`)
  excluding `target/`, `.git`, `docs/reviews/`, the `result` symlink and
  `__pycache__`, with the exact approved patch applied via `patch -p1`;
  verified byte-identical to the candidate outside `target/` and
  reproducing the `f8a51111...` aggregate. Preparation receipt:
  `docs/reviews/2026-11-08-bark-v31-profile-primary-preparation.md` and
  `/tmp/opencode/AP1b-lab-integration-prepared.json`.

## Decision

1. **Opt-in, isolated Ark development integration only.** The default
   lab rbitcoin node remains the approved L2 binary
   `eb257af7c10cdf0ded584adfadf2ae8124466304b13d26c17f855e76f9320f7c`
   with the standard L2 config. No pin, no `UPSTREAM_DEPS.md` row, and
   no v0.9 adoption changes. The profile exists only as a separate
   development node profile, selected explicitly per launch.
2. **Profile semantics are exactly the reviewed ones.** Default is off:
   `getnetworkinfo.version` stays `190000`, no `rbitcoin_compat`
   object, dialect unchanged. With `--rpc-compat-profile bark-v31` or
   conf `rpc_compat_profile=bark-v31` - valid **only on regtest** - the
   node reports `310000` as an explicit, machine-readable compatibility
   claim while the real `/rbitcoin:0.8.99/` identity and
   `protocolversion` stay unchanged, and `submitpackage` is actually
   gated by Core `IsChildWithParents`. Unknown profile names,
   non-regtest networks, and an explicitly-off dialect conflict fail
   closed at startup, before effects.
3. **No Core v31 parity and no full Bark acceptance are claimed.** The
   claim covers the tested read-only RPC surface only. Funded behavior
   stays `not_proven`, `full_acceptance` stays false; the funded,
   witness and real-Bark acceptance gates are separate and later.
4. **The removed env knob stays unapproved.** `RBITCOIN_RPC_COMPAT_PROFILE`
   is not implemented, is inert, and must stay so unless the node repo
   grants its own ask-first approval for a new `RBITCOIN_*` surface.
   `RBITCOIN_RPC_PACKAGE_DIALECT` remains the actual policy input
   (tri-state); explicit off plus the profile fails closed.
5. **The `getmempoolinfo` selected-model failure stays untouched.**
   `maxdatacarriersize: expected JSON integer, got null` against the
   selected v30 modelling consumer remains a known, deliberately
   unpatched traffic limitation; this profile does not launder it.
6. **Pre-use gate (required BEFORE any opt-in use).** On the INTEGRATED
   binary built from `/tmp/opencode/AP1b-lab-integration`: full
   `cargo test -p rbitcoin-node --test suite` and
   `cargo test -p rbitcoin-rpc --test suite` receipts, plus fresh
   external-mode runs of the frozen approved probe (contract
   `f4fc3e043f420870675095ab24144ba14e0afba6cb9ad69d818d99b1bccb2f62`)
   in both default and profile-on configurations. Until these land, the
   profile is adopted on paper only: not wired into any run and not used
   by any acceptance. Funded/witness/real-Bark gates and native
   WITNESS/T4b acceptance remain separate later work.
7. **Build/test status at recording: PENDING.** The primary six-check
   Nix gate was still running during preparation; no compile, test,
   node launch or probe run was performed in the preparation step, and
   no "integration complete" claim is made by this record or the
   preparation receipt.

## Consequences

- `testing/rbitcoin-bark-v31/` is additive primary evidence (80 files;
  27 of the 82 frozen records are `*.log` files gitignored by
  `.gitignore:6` and remain local evidence only). The r0 archive and
  both implementation reports are preserved byte-exact.
- A one-bullet changelog fragment
  `docs/changelog/2026-11-08-lab-bark-v31-profile.md` is added per
  `docs/changelog/README.md`; `docs/SPEC.md` is NOT spliced while the
  primary gate is in progress (the merge session splices the fragment).
- Fable's non-blocking notes N1-N8 are carried in the preparation
  review, not fixed in code; any of them is a future node-packet change,
  never a fork of the approved patch.
- The promoted decision fragment is
  `testing/rbitcoin-bark-v31/decision-fragment.md` (r1, PROPOSED ->
   adopted by this record with the terms above).
- A second profile name, an env form, a free-form version integer,
  response rewriting, or patching the `getmempoolinfo` row would each be
  a new candidate and a new decision.

## Gate completion (append-only note, 2026-11-08)

The item 6 pre-use gate has been executed and **all required gates
PASSED** on the integrated binary built from
`/tmp/opencode/AP1b-lab-integration` (validation receipt:
`docs/reviews/2026-11-08-bark-v31-profile-primary-validation.md` and
`/tmp/opencode/AP1b-lab-integration-validation.json`):

- `cargo fmt --all -- --check` exit 0; `cargo clippy --locked -p
  rbitcoin-node -p rbitcoin-rpc --all-targets` exit 0 (pinned rustc/cargo
  1.95.0; targeted crates only, no whole workspace; `-j2`, serialized
  test threads; every gate passed on the first attempt - no reruns).
- Full suites: `rbitcoin-node --test suite` **118/118**;
  `rbitcoin-rpc --test suite` **214/214**.
- Frozen approved probe (contract `f4fc3e04...`), external mode,
  devcheck 12/12 first: default run `190000`, no compat object,
  verdict `incompatible [fail-version-gate, funded-unproven]`;
  profile-on run (`--rpc-compat-profile bark-v31`, clean env with every
  `RBITCOIN_*` stripped so the profile alone selects the dialect)
  `310000` claim with the real `/rbitcoin:0.8.99/` identity,
  `rbitcoin_compat` + banner, `-25 package-topology` rejection of the
  non-child-with-parents counterfactual, verdict `surface_compatible
  [funded-unproven]`. Fail-closed small checks (dialect-off conflict
  exit 2; non-regtest exit 1; datadir never created) also pass.
- Integrated binary
  `b75dad84bdd6663673a29c07352d10c4f78d2dcd14e885f62ad3209d214443e4`
  (byte-identical to the approved candidate - legitimate reproduction
  of identical source under the pinned toolchain; provenance is
  path/build/source-index per the parent correction, not hash
  inequality). The 2102-file source index
  (`da21f56d...`) and every frozen boundary (L2 tree/binary, candidate,
  packet, primary 82 records, probe freeze) verified unchanged after
  validation.

Item 7's "PENDING" status at recording is therefore superseded by this
note. The profile is a **validated opt-in artifact** - the default L2
node, pin and v0.9 adoption are unchanged, and the known limitations
(`getmempoolinfo` null row, funded-unproven, no Core v31 parity, not
full Ark end-to-end ready) stand as recorded. The changelog fragment
`docs/changelog/2026-11-08-lab-bark-v31-profile.md` **remains pending
splice** into the `SPEC.md` frontmatter: per `docs/MERGE-QUEUE.md` it
must not race the T1a primary writer; the parent/merge session
coordinates the slot.
