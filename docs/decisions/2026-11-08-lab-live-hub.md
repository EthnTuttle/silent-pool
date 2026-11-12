# 2026-11-08 - Live lab hub (`/lab`, `spc.lab.inventory.v1`)

This record covers the lab-tool read-only hub surface only, not pool/payout/
Ark protocol behavior. No SPEC body section changes; no payout, privacy-set
or Ark semantics are introduced or altered here. The Fable approval
(`docs/reviews/2026-11-08-live-lab-hub-fable.md`, code snapshot `712fa8cb...`)
is the authority for the transferred implementation; this record is the merge
condition R2 it required.

## Decisions

1. **Schema `spc.lab.inventory.v1`.** The projection served by
   `GET /api/lab/v1/runs/<id>/inventory` is a closed, versioned DTO built by
   `crates/spc-lab/src/inventory.rs` from three short snapshots (manifest,
   process records, evidence events). It is an allowlisted projection, never
   the raw private manifest or process records.
2. **Routes.** The existing loopback `HttpServer` gains three static routes
   (`/lab`, `/lab/`, `/lab/index.html`; `/lab.js`; `/lab.css`) and the
   inventory route scoped to the engine's own run id prefix. No new listener,
   daemon, proxy or command path; non-GET falls to the existing 404 arm. The
   public `/` page gains only a header link, a view-mode pill and a recorded
   banner.
3. **Read-only and recorded semantics.** Recorded-run review uses
   `spc-lab serve` (attach without adopt, `read_only = true`): the engine's
   `submit` rejects control with `rejected_precondition` before any effect,
   both pages identify recorded/read-only, miner controls are disabled, and a
   recorded `running` state is historical, not a current health claim. GET of
   any hub route mutates neither the run, the command log nor evidence.
4. **Health is evidence, not inference.** Every row's observation comes only
   from collected evidence with its real timestamp and age; absence is
   `unknown` with its source. The hub performs no probe and creates no
   synthetic heartbeat.
5. **Privacy allowlist.** Every static label is `&'static str`; the only
   dynamic strings are scenario identifiers, `run_id` and lifecycle. No
   manifest field, process record field, argv, env, artifact path, credential,
   descriptor or identity field (`sp_id`, `B_scan`, `B_spend`, pool payout
   descriptor) is serialized. Links are emitted only for known loopback routes
   (`/`, `/lab`, `/api/pool`, `/metrics`) and the browser side re-validates
   the same allowlist before rendering; namespace endpoints never become host
   browser links.
6. **Capability honesty.** `spc-agent` wallet, the independent Ark server,
   per-pool Ark delivery/float and SPc user wallets are recorded as
   not-running planned-required integrations, never "optional" or
   "implemented" (R1 wording in `docs/LAB-RUNTIME.md`).
7. **`docs/ARCHITECTURE.md` banner.** The file is marked a historical target
   map; its "the pool never speaks SV1" line is corrected against SPEC §1
   (2026-11-02 native SV1), a lower-document fix under the AGENTS.md
   precedence rule. `docs/LAB-ROLES.md` is the new source-cited role map.

## Consequences

The lab gains a read-only role/process/interface view bound to real recorded
evidence. Nothing here authorizes operator/agent UI work, Ark end-to-end
claims, a new auth platform, or a fresh native live experiment; the optional
R3/R6/R7 code nits are deferred to the next hub packet by the approval's
exact-binary scope, and the R5 doc nit ("A missing observation is shown as
unknown") was applied at merge with the approval's leave. The hub's browser
proof (84/84, binary `a577...`) stands on the candidate's sealed binary; the
merged tree re-proves routes/assets/serve behavior with its own scoped checks.
