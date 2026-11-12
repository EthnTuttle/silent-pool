# AGENTS.md — entry point for implementation agents

This repo is a **specification set plus validated prototypes**. There is no `spc-pool` or `spc-agent` code yet; your job is to build it from the docs below. Read this file first, then follow the reading order.

## Reading order

1. `README.md` — what the pool is, one page.
2. `DESIGN.md` — payout derivation, tier policy, Ark delivery. **Authoritative** for those topics.
3. `docs/SPEC.md` — the normative technical spec (RFC 2119 language). Read the frontmatter changelog first; it records every decision applied and when.
4. `docs/ARCHITECTURE.md` — binaries, roles, crates, process boundaries, Redis/Postgres layout.
5. `docs/WIRE-FORMATS.md` — byte-level formats bespoke to this project (streams, bootstrap RPC, `0x5350` messages). Upstream SV2 formats are referenced, never restated.
6. `docs/MULTI-REGION.md` — multi-front deployment design and rationale. Only needed for multi-region work.
7. `docs/ROADMAP.md` — phases, entry criteria, and per-phase acceptance checklists keyed to SPEC sections. Work packets come from here.
8. `MAILBOX.md`, `RESULTS.md`, `UPSTREAM_DEPS.md` — Ark mailbox design, what the prototypes proved, and the pinned upstream commits.

## Precedence when documents disagree

```
DESIGN.md  >  docs/SPEC.md  >  ARCHITECTURE.md / MULTI-REGION.md / WIRE-FORMATS.md  >  everything else
```

- A disagreement between `SPEC.md` and a lower document is a bug in the lower document. Fix the lower document, do not reinterpret the spec.
- A disagreement between `DESIGN.md` and `SPEC.md` on derivation/tiers/Ark delivery is a bug in the spec (`SPEC.md` frontmatter `authority:` line).
- `docs/decisions/*.md` are **binding**. Each records a decision, its rationale, and which sections were edited. If a decision record and the spec disagree, the spec was not updated — raise it, do not pick one silently.

## What is NOT normative

- `docs/research/**` — background reading and raw source notes. Several documents describe approaches that were **rejected**; superseded files carry a banner. Do not implement anything from here unless `SPEC.md` cites it.
- `docs/analysis/**` — modeling behind decisions (e.g., the O11 formula). Supports a decision; does not define behavior.
- `docs/reviews/**` — audit history. Useful for understanding *why* things changed; not a source of requirements.
- Code comments in the prototype crates are not spec. Their **test vectors** are (see below).

## Prototype crates

`spc_ark/`, `spc_ark_mailbox/`, `spc_ark_client/` are validated reference prototypes (`RESULTS.md`), to be absorbed into `spc-crypto` / `spc-agent` per `ARCHITECTURE.md`. Treat their frozen test vectors as conformance tests. `spc_ark_client/run_e2e.sh` needs `BARK_CHECKOUT` pointing at bark `c419159` and a `bitcoind`; `spc_ark_client/test/` is a runtime-generated regtest datadir and is gitignored.

## Upstream dependencies

Every upstream commit is pinned by SHA in `UPSTREAM_DEPS.md` (with a verification procedure). Pins are reference points, **not gates**: this project implements the whole tooling suite itself and absorbs upstream drift in-tree (`docs/decisions/2026-10-14-upstream-own-suite.md`). Never bump a pin without updating `UPSTREAM_DEPS.md` and the lockfile in the same change.

## Working conventions

- **Decisions**: any change to normative behavior gets a dated record in `docs/decisions/` (copy the format of an existing one) and a `SPEC.md` frontmatter changelog entry, in the same change.
- **Cross-references**: cite sections as `§x.y`; when you edit a section, grep for inbound references and fix them.
- **Plain ASCII punctuation** in Markdown: hyphen-minus `-`, not U+2011 or other lookalikes. They have broken links here before.
- **Privacy constraints are load-bearing.** No identity field (`sp_id`, `B_scan`, `B_spend`, `P_i`, block hash) may cross into the `ark` role or its streams (`SPEC.md` §11.3). Tests in ROADMAP Phase 7 assert this; do not weaken them.
- **Vanilla Ark interoperability is required.** Silent-pool must work with
  unmodified Second.tech/Bark (`captaind`) servers through their standard
  interfaces (SPEC §11.3 and O17). SPc-specific registration, derivation and
  wallet handling belong in the pool/client adapters, not in an Ark-server
  fork, plugin or custom RPC. Normal regtest configuration is allowed for
  tests; discover real server policy/fees through the supported interfaces.
  Verify send/receive/refresh/offboard against the unmodified server before
  claiming end-to-end readiness. Lab Bitcoin-node RPC profiles are separate
  from Ark-server compatibility and do not authorize server modifications.
- **Lab Ark is self-hosted.** Build the lab's Ark server and tooling from
  pinned source repositories using Nix. The lab owns the server, storage,
  wallets, private configuration and process lifecycle; hosted-provider
  endpoints are not the expanded lab deployment target.
- **Fail closed.** Address/descriptor parsing, order verification, and distribution validation reject on any ambiguity. There is no donation fallback.
- Do not git-commit on behalf of a human unless explicitly asked in the current task.

## Build disk budget

- Before a Rust/Nix build or test invocation, check available space on the
  actual target/store filesystem. Do not start below 50 GiB available, or if
  estimated peak growth would leave less than 20 GiB. Report the blocked step;
  continue lightweight source/review work rather than retrying into ENOSPC.
- Run only one heavy build across cooperating agents. Coordinate with the
  parent and hold `/tmp/opencode/silent-pool-heavy-build.lock` with nonblocking
  `flock` for the build's lifetime; check space after acquiring the lock.
- Respect `/tmp/opencode/silent-pool-build-priority.json` when present: only
  its named session may acquire the next build slot. Other sessions finish
  any already-held invocation, then yield; the parent clears the reservation.
- Reuse the existing target directory for a topic and its reviews. Do not
  create another target per revision or repeat passing broad checks without a
  new change or unresolved verification requirement.
- Source snapshots must exclude `.git`, all `target` directories, runtime
  datadirs and generated caches. Record snapshot size before Nix ingestion;
  never feed a multi-gigabyte build tree to a raw `path:` source.
- Reclaim completed build caches only with explicit cleanup authorization,
  after checking ownership, active users and executable/evidence references.
  Preserve referenced binaries by hash and keep sources, frozen manifests,
  receipts and user feedback. Age or filesystem owner alone is not proof that
  another session's artifacts are disposable. No unapproved shared-cache or
  Nix-store garbage collection.

## Current status (2026-10-15)

All findings from `docs/reviews/2026-10-14-implementation-readiness-review.md` are resolved. Phases 0-3 and single-region Phase 4/7 are ready to implement; multi-region work (Phase 3 multi-front, Phase 8) should start from `MULTI-REGION.md` with all §10 deltas now applied to the spec.
