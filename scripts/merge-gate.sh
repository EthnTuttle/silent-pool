#!/usr/bin/env bash
# Serial integration gate for the merge session.
#
#   scripts/merge-gate.sh            # fmt, tests, clippy, nix
#   scripts/merge-gate.sh --full     # + the regtest harnesses (needs the devshell)
#
# Harnesses bind fixed ports, so they always run sequentially. Run under
# `nix develop -c` (or with bitcoind/postgres/redis on PATH) for --full.
set -euo pipefail
root=$(git rev-parse --show-toplevel)
cd "$root"

fail() { echo "GATE FAILED: $*" >&2; exit 1; }

echo "== fmt =="
cargo fmt --all -- --check || fail "fmt"

echo "== tests =="
if ! cargo test --workspace --all-targets --features spc-pool/rpc \
    > /tmp/spc-merge-gate-tests.log 2>&1; then
  tail -40 /tmp/spc-merge-gate-tests.log
  fail "tests"
fi
grep -E "test result:" /tmp/spc-merge-gate-tests.log | tail -20

echo "== clippy =="
cargo clippy --workspace --all-targets --features spc-pool/rpc -- -D warnings || fail "clippy"

echo "== nix flake check =="
nix flake check || fail "nix flake check"

if [ "${1:-}" = "--full" ]; then
  echo "== regtest (bitcoind, front) =="
  ./testing/regtest-e2e.sh || fail "regtest-e2e"
  echo "== regtest (bitcoind, services) =="
  ./testing/regtest-services-e2e.sh || fail "regtest-services-e2e"
  if [ -x "${RBITCOIN_NODE:-/nonexistent}" ]; then
    echo "== regtest (rbitcoin, front) =="
    ./testing/regtest-rbitcoin-e2e.sh || fail "regtest-rbitcoin-e2e"
    echo "== regtest (rbitcoin, reorg) =="
    ./testing/regtest-rbitcoin-reorg-e2e.sh || fail "regtest-rbitcoin-reorg-e2e"
  else
    echo "== rbitcoin harnesses skipped (set RBITCOIN_NODE) =="
  fi
  if [ -x "${CAPTAIND_EXEC:-/nonexistent}" ]; then
    echo "== ark e2e (stock captaind) =="
    (cd spc_ark_e2e && CAPTAIND_EXEC="$CAPTAIND_EXEC" \
      BITCOIND_EXEC="${BITCOIND_EXEC:-bitcoind}" \
      POSTGRES_BINS="${POSTGRES_BINS:-/usr/bin}" ./run_e2e.sh) || fail "ark e2e"
  else
    echo "== ark e2e skipped (set CAPTAIND_EXEC) =="
  fi
fi

echo "GATE OK"
