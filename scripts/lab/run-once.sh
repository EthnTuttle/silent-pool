#!/usr/bin/env bash
# Headless LP-3 fixture run. Real binaries are supplied by --config; this
# default config uses the explicitly-tagged scripted process fixture.
set -euo pipefail
umask 077

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCENARIO="$ROOT/crates/spc-lab/tests/fixtures/scenario_v1_one_pool_two_miners.json"
CONFIG=""
RUN_DIR=""
DURATION=""
KEEP=""
HEADLESS=0
RUN_ROOT="${SPC_LAB_RUN_ROOT:-${XDG_RUNTIME_DIR:-/tmp}/spc-lab/runs}"
LOG="${SPC_LAB_LOG:-$RUN_ROOT/run-once.log}"

usage() {
  cat <<'USAGE'
usage: scripts/lab/run-once.sh [--scenario FILE] [--config FILE] [--run-dir DIR]
                               [--duration-secs N] [--keep] [--headless]
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --scenario) SCENARIO="$2"; shift 2 ;;
    --config) CONFIG="$2"; shift 2 ;;
    --run-dir) RUN_DIR="$2"; shift 2 ;;
    --duration-secs) DURATION="$2"; shift 2 ;;
    --keep) KEEP="--keep"; shift ;;
    --headless) HEADLESS=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

case "$SCENARIO" in
  *.toml) echo "LP-3 implements the versioned JSON scenario (scenario_v1); convert the file before use" >&2; exit 2 ;;
esac

mkdir -p "$RUN_ROOT" "$(dirname "$LOG")"
if [[ -z "$RUN_DIR" ]]; then
  RUN_DIR="$RUN_ROOT/$(date -u +%Y%m%dT%H%M%SZ)-$$"
fi

export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
cargo build -j "$CARGO_BUILD_JOBS" -p spc-lab --bin spc-lab-fixture -p spc-lab-cli --bin spc-lab >"$LOG.build" 2>&1
FIXTURE_BIN="$TARGET_DIR/debug/spc-lab-fixture"
LAB_BIN="$TARGET_DIR/debug/spc-lab"
if [[ ! -x "$FIXTURE_BIN" || ! -x "$LAB_BIN" ]]; then
  echo "run-once: missing build artifacts ($FIXTURE_BIN, $LAB_BIN)" >&2
  exit 3
fi

mkdir -p "$RUN_DIR"
if [[ -z "$CONFIG" ]]; then
  CONFIG="$RUN_DIR/lab-config.json"
  sed "s|@FIXTURE_BIN@|$FIXTURE_BIN|g" "$ROOT/scripts/lab/fixture-config.json.in" >"$CONFIG"
fi

ARGS=(run --scenario "$SCENARIO" --config "$CONFIG" --run-dir "$RUN_DIR")
if [[ "$HEADLESS" -eq 1 ]]; then ARGS+=(--headless); fi
if [[ -n "$DURATION" ]]; then ARGS+=(--duration-secs "$DURATION"); fi
if [[ -n "$KEEP" ]]; then ARGS+=(--keep); fi

set +e
"$LAB_BIN" "${ARGS[@]}" 2>&1 | tee "$LOG"
status=${PIPESTATUS[0]}
set -e
echo "run-once: status=$status run_dir=$RUN_DIR log=$LOG"
exit "$status"
