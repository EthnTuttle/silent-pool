#!/usr/bin/env bash
# Reproduces the committed AP-1a r2 evidence: probes the two approved
# rbitcoin binaries (old L2 and v0.9+lab) against fresh owned regtest
# nodes, read-only, and writes reports/2026-11-08-r2-<label>/{report.json,
# report.md,raw_responses.json,manifest.json} plus reports/gap-matrix.md.
#
# The devcheck golden-vector gate runs FIRST and fails closed: its
# receipt (exit code + output) is written to a temp JSON file and bound
# into each run manifest via --devcheck-receipt (D7), so no evidence is
# written that the pinned crate types do not back.
#
# The frozen v1 evidence (reports/l2-eb257af7, reports/v09lab-bcb6d5ee)
# and the historical r1 evidence (reports/2026-11-08-r1-*) are immutable
# history and are never rewritten by this script.
#
# Env overrides:
#   OLD_BIN   old approved L2 node binary
#   NEW_BIN   v0.9+lab node binary
#   SKIP_SELFTEST=1  skip the fixture self-test gate
#   SKIP_DEVCHECK=1  skip the devcheck gate (receipt omitted from manifests)
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
OLD_BIN="${OLD_BIN:-}"
NEW_BIN="${NEW_BIN:-}"
R2_DIR_L2="2026-11-08-r2-l2-eb257af7"
R2_DIR_V09="2026-11-08-r2-v09lab-bcb6d5ee"

for bin in "$OLD_BIN" "$NEW_BIN"; do
    if [ ! -x "$bin" ]; then
        echo "missing node binary: $bin (set OLD_BIN / NEW_BIN)" >&2
        exit 1
    fi
done

if [ "${SKIP_SELFTEST:-0}" != "1" ]; then
    echo "==> fixture self-test (no nodes involved)"
    (cd "$HERE" && ./probe.py self-test)
fi

DEVRECEIPT=""
if [ "${SKIP_DEVCHECK:-0}" != "1" ]; then
    echo "==> devcheck gate (golden vectors vs the pinned crate types)"
    DEVLOG="$(mktemp "${TMPDIR:-/tmp}/spc-lab-devcheck-log.XXXXXX")"
    DEVRECEIPT="$(mktemp "${TMPDIR:-/tmp}/spc-lab-devcheck-receipt.XXXXXX.json")"
    set +e
    (cd "$HERE/devcheck" && ./run.sh) >"$DEVLOG" 2>&1
    DEVEXIT=$?
    set -e
    python3 - "$DEVLOG" "$DEVEXIT" "$DEVRECEIPT" <<'PYEOF'
import json, sys
log, exit_code, out_path = sys.argv[1], int(sys.argv[2]), sys.argv[3]
text = open(log).read()
vectors = sum(1 for line in text.splitlines() if line.startswith("[PASS]"))
json.dump({"exit_code": exit_code, "vectors": vectors, "output": text},
          open(out_path, "w"), indent=1)
PYEOF
    rm -f "$DEVLOG"
    if [ "$DEVEXIT" != "0" ]; then
        echo "devcheck gate FAILED (exit $DEVEXIT); no evidence written" >&2
        cat "$DEVRECEIPT" >&2
        rm -f "$DEVRECEIPT"
        exit 1
    fi
    echo "    devcheck passed ($(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["vectors"])' "$DEVRECEIPT") vectors)"
fi

TOKEN="AP1a-$(python3 -c 'import secrets; print(secrets.token_hex(6))')"
echo "==> run token $TOKEN (node scratch is cleaned up by each probe)"

run_one() {
    local bin="$1" label="$2" source_name="$3" r2_dir="$4"
    echo "==> probing $label ($bin, pin $source_name)"
    (cd "$HERE" && ./probe.py node-probe \
        --binary "$bin" \
        --label "$label" \
        --source-name "$source_name" \
        --out "$HERE/reports/$r2_dir" \
        --run-token "$TOKEN" \
        --max-run-secs 240 \
        ${DEVRECEIPT:+--devcheck-receipt "$DEVRECEIPT"})
}

run_one "$OLD_BIN" "rbitcoin-lab-retarget-profile-r2" \
    "rbitcoin-lab-retarget-profile" "$R2_DIR_L2"
run_one "$NEW_BIN" "rbitcoin-v0.9-lab-r2" \
    "rbitcoin-v0.9-lab" "$R2_DIR_V09"

[ -n "$DEVRECEIPT" ] && rm -f "$DEVRECEIPT"

echo "==> gap matrix"
(cd "$HERE" && ./probe.py compare \
    --reports "reports/$R2_DIR_L2/report.json" \
              "reports/$R2_DIR_V09/report.json" \
    --out reports/gap-matrix.md)

echo "==> done. r2 evidence in $HERE/reports/$R2_DIR_L2 and $R2_DIR_V09;"
echo "    node scratch (incl. cookie credentials) was removed by the probes"
