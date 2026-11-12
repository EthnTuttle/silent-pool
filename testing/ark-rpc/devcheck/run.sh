#!/bin/sh
# devcheck runner: validates the arkrpc Python contract golden vectors
# against the real pinned Rust crate types bark c419159 uses.
#
# Dev-only. Never required by the probe, the self-test, or the unit
# tests; excluded from the packet manifest hash.
#
# Requirements:
#   - BARK_CHECKOUT: the pinned bark checkout (required; commit
#     c419159 per UPSTREAM_DEPS.md - clone it at that exact SHA). The
#     path in Cargo.toml must point at its REAL location (see
#     README.md).
#   - a cargo registry cache with the pinned crates (offline build).

set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
BARK_CHECKOUT="${BARK_CHECKOUT:-}"
EXPECT_REV="c419159fa54085572f8ca72d2ba1716556a9d3eb"

if [ ! -d "$BARK_CHECKOUT/bitcoin-ext" ]; then
    echo "devcheck: bark checkout not found at $BARK_CHECKOUT" >&2
    echo "devcheck: set BARK_CHECKOUT (and update the bitcoin-ext path" >&2
    echo "devcheck: in Cargo.toml to the real path; see README.md)" >&2
    exit 2
fi

rev=$(git -C "$BARK_CHECKOUT" rev-parse HEAD 2>/dev/null || echo unknown)
if [ "$rev" != "$EXPECT_REV" ]; then
    echo "devcheck: WARNING bark checkout is at $rev" >&2
    echo "devcheck: expected c419159 (pinned per UPSTREAM_DEPS.md);" >&2
    echo "devcheck: results may not reflect the pinned types" >&2
fi

cd "$here"
exec cargo run --offline
