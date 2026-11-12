#!/usr/bin/env bash
# Validate the scenario and the adapter capability/profile gates without
# starting a process.
set -euo pipefail
umask 077
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCENARIO="${1:-$ROOT/crates/spc-lab/tests/fixtures/scenario_v1_one_pool_two_miners.json}"
CONFIG="${2:-}"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
RUN_ROOT="${SPC_LAB_RUN_ROOT:-${XDG_RUNTIME_DIR:-/tmp}/spc-lab}"
mkdir -p "$RUN_ROOT"
cargo build -j "$CARGO_BUILD_JOBS" -p spc-lab --bin spc-lab-fixture -p spc-lab-cli --bin spc-lab >"$RUN_ROOT/preflight-build.log" 2>&1
FIXTURE_BIN="$TARGET_DIR/debug/spc-lab-fixture"
if [[ -z "$CONFIG" ]]; then
  CONFIG="$(mktemp "$RUN_ROOT/preflight-config.XXXXXX.json")"
  sed "s|@FIXTURE_BIN@|$FIXTURE_BIN|g" "$ROOT/scripts/lab/fixture-config.json.in" >"$CONFIG"
fi
"$TARGET_DIR/debug/spc-lab" validate --scenario "$SCENARIO" --config "$CONFIG"
