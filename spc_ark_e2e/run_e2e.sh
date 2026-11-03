#!/usr/bin/env bash
# Regtest e2e for the spc-pool ark role over a stock bark server.
#
# The test itself is gated on SPC_ARK_E2E=1; this script sets up the
# binaries it needs and runs it:
#   * captaind, built UNMODIFIED from bark at c419159 - either point
#     CAPTAIND_EXEC at an existing binary, or point BARK_CHECKOUT at a
#     clean bark checkout at c419159 and it is built here.
#   * bitcoind v31+ via BITCOIND_EXEC.
#   * postgres binaries via POSTGRES_BINS (ark-testing spawns a
#     throwaway instance for captaind).
set -euo pipefail
cd "$(dirname "$0")"

export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$PWD/target}

if [ -z "${CAPTAIND_EXEC:-}" ]; then
	BARK=${BARK_CHECKOUT:?set CAPTAIND_EXEC or BARK_CHECKOUT (a clean bark checkout at c419159)}
	# Refuse to run against a modified tree so "stock server" stays honest.
	(cd "$BARK" && git diff --quiet && git diff --cached --quiet) || { echo "bark checkout is modified" >&2; exit 1; }
	# Separate target dir: sharing one with this workspace corrupts artifacts.
	(cd "$BARK" && CARGO_TARGET_DIR=$CARGO_TARGET_DIR-captaind cargo build -p bark-server --bin captaind)
	export CAPTAIND_EXEC=$CARGO_TARGET_DIR-captaind/debug/captaind
fi

export BITCOIND_EXEC=${BITCOIND_EXEC:?set BITCOIND_EXEC to a bitcoind v31+ binary}
export POSTGRES_BINS=${POSTGRES_BINS:?set POSTGRES_BINS to a postgres bin directory}
export TEST_DIRECTORY=${TEST_DIRECTORY:-$PWD/test}
export RUST_LOG=${RUST_LOG:-info}
export SPC_ARK_E2E=1

cargo test --test e2e -- --nocapture "$@"
