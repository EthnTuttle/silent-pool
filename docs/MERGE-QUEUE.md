# Merge queue and conflict protocol

Purpose: let implementation packets run in parallel worktrees without fighting
over the same files, and make one session - the **merge session** - the only
writer to the shared integration surface.

The orchestrator dispatches packets and owns the board's dispatch rows. The
merge session owns integration: rebase, conflict resolution, the gate, the
SPEC changelog and the merge commits.

## Shared files (merge-session-owned)

These are the collision points. A packet may need a change here; it ships the
change as a *merge note* in its queue entry (exact hunk or a patch file under
`merge/<branch>/`) and lets the merge session apply it. A packet never lands a
commit that edits these on its own branch unless the queue entry says it owns
the file for that packet.

| File | Why it collides | Owner |
|---|---|---|
| `docs/SPEC.md` frontmatter `changelog:` | every decision packet adds an entry at the same line | merge session |
| `docs/STATUS.md` | every packet wants to report | merge session (dispatch rows: orchestrator) |
| `bins/spc-pool/src/config.rs` | every packet adds fields/validation | merge session until the config split lands |
| `bins/spc-pool/src/main.rs` | role wiring for every packet | merge session until the main split lands |
| `docs/OPERATIONS.md` | several packets add sections | merge session until the docs split lands |
| `Cargo.toml` / `Cargo.lock` / `flake.nix` | dependency and vendor-hash changes | merge session |
| `config/*.example.toml` | settings for several packets | merge session |

Everything else is packet-owned: a packet's module, its tests, and its
decision record (`docs/decisions/YYYY-MM-DD-<slug>.md`) are disjoint by
construction.

## Changelog fragments (removes the SPEC conflict)

A packet with normative changes does **not** edit the `SPEC.md` frontmatter.
It adds `docs/changelog/YYYY-MM-DD-<slug>.md` containing the exact bullet(s)
that belong under `changelog:`. At merge the merge session splices them in
(newest first, above the previous entry) and deletes the fragment. Git keeps
the history.

## Queue entry

The dispatching orchestrator appends one line per branch:

```
| branch | base SHA | ready SHA | owned paths | merge notes | gate |
```

The merge session appends the outcome:

```
merged <sha> | conflict: <files> <resolution> | gate: fmt/test/clippy/nix/e2e <result>
```

## Merge session procedure

1. `git fetch` and `scripts/merge-check.sh <branch>`; read the conflict list
   before touching anything.
2. Rebase the branch onto `master` **in the branch's worktree** if the session
   that wrote it is gone; otherwise ask it to rebase. Resolve only the files in
   the ownership table above; leave packet-owned files to the packet.
3. Splice changelog fragments; update the board.
4. Run `scripts/merge-gate.sh` on the merged tree. Any red stops the merge and
   the branch goes back to its author with the failing output.
5. Commit the merge with the why, push, and record the outcome here.
6. Never `stash`/`reset --hard`/`clean` the main tree while another packet's
   work is uncommitted in it.

## Enabling refactor (next, then parallelism scales)

The durable fix is to stop sharing those files:

- `config.rs` -> `config/mod.rs` + per-section modules (`template.rs`,
  `accounting.rs`, `ark.rs`, `reach.rs`, ...); a packet adds a module and one
  `mod` line, not a field in a hot struct.
- `main.rs` -> `main/roles/{front,payout,api,ark}.rs` plus a thin `main.rs`; a
  packet wires its role module.
- `docs/OPERATIONS.md` -> `docs/operations/*.md` with an index.
- `docs/STATUS.md` stays single-writer (merge session).

Until that lands, the merge session absorbs the config/main conflicts.
