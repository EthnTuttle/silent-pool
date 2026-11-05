# Changelog fragments

Each normative change adds one file here instead of editing the `changelog:`
list in `docs/SPEC.md` frontmatter. The merge session splices the fragment into
the SPEC frontmatter at merge and deletes it; git keeps the history.

## Format

Name: `YYYY-MM-DD-<slug>.md`, matching the decision record slug when there is
one. Content: the exact bullet line(s) that belong under `changelog:`,
including the leading two spaces and `- "` quoting used by the frontmatter.
No frontmatter of its own; one entry per file.

Example:

```
  - "2026-11-05: one sentence of what changed and which sections moved. See docs/decisions/2026-11-05-<slug>.md."
```

## Why

Every packet used to add its entry at the same line of `SPEC.md`, so every
parallel branch conflicted there. A fragment per packet is disjoint by
construction and the merge order lives in one place.
