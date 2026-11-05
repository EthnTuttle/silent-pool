#!/usr/bin/env bash
# Dry-run a merge of <branch> into the current HEAD without touching the
# worktree or the index. Prints the conflicted paths when it would conflict.
#
#   scripts/merge-check.sh <branch>
#
# Exit 0 clean, 1 conflicts, 2 usage.
set -euo pipefail
branch=${1:-}
if [ -z "$branch" ]; then
  echo "usage: scripts/merge-check.sh <branch>" >&2
  exit 2
fi
root=$(git rev-parse --show-toplevel)
cd "$root"

out=$(git merge-tree --write-tree --name-only HEAD "$branch" 2>&1) && {
  echo "clean: $branch merges into $(git rev-parse --short HEAD)"
  exit 0
}

echo "CONFLICTS: $branch does not merge cleanly into $(git rev-parse --short HEAD)"
# First line is the tree oid; the conflicted paths follow, one per line, until
# the informational section. Keep it to the paths.
echo "$out" | tail -n +2 | sed -n '1,40p'
exit 1
