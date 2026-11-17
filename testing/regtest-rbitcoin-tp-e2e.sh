#!/usr/bin/env bash
# Regtest end-to-end through rbitcoin-node's SV2 Template Provider: a
# full-stack spc-pool (front,payout,api) + Postgres + Redis, with
# [template] source = "sv2-tp". The node serves templates over Noise_NX
# TDP (--sv2-tp-listen), the pool submits the miner's block with
# SubmitSolution, and the node's cookie-authed TCP JSON-RPC is only the
# settlement chain view (decision 2026-11-05-sv2-tp-chain-view).
#
# Mirrors testing/regtest-rbitcoin-e2e.sh and regtest-services-e2e.sh on
# disjoint ports so every harness can run side by side.
#
# Env: RBITCOIN_NODE (default the pinned dev build), RBITCOIN_CLI (optional;
# derived from the node dir), KEEP=1 keeps the temp dir.
set -euo pipefail

RBITCOIN_NODE="${RBITCOIN_NODE:-/home/ethan/code/rbitcoin/target/dev/debug/rbitcoin-node}"
RBITCOIN_CLI="${RBITCOIN_CLI:-$(dirname "$RBITCOIN_NODE")/rbitcoin-cli}"
if [ ! -x "$RBITCOIN_NODE" ]; then
  echo "SKIP: rbitcoin-node not found at $RBITCOIN_NODE (set RBITCOIN_NODE)"
  exit 0
fi

ROOT=$(cd "$(dirname "$0")/.." && pwd)
TMP=$(mktemp -d)
DATADIR="$TMP/rbitcoin"
COOKIE="$TMP/rpc.cookie"
TP_KEY="$TMP/sv2-tp-authority.key"
PGDATA="$TMP/pg"
POOL_CONFIG="$TMP/spc-pool.toml"
POOL_LOG="$TMP/pool.log"
NODE_LOG="$TMP/node.log"
MINER_LOG="$TMP/miner.log"
RPC_PORT=39443
TP_PORT=39447
PG_PORT=55434
REDIS_PORT=56381
SV1_PORT=49333
SV2_PORT=49454
JD_PORT=49464
API_PORT=49080
POW_BITS=0x200fffff
# Fixed, valid regtest P2TR payout script, as the other rbitcoin harnesses.
POOL_SCRIPT=5120a60869f0dbcf1dc659c9cecbaf8050135ea9e8cdc487053f1dc6880949dc684c

cleanup() {
  set +e
  [[ -n "${POOL_PID:-}" ]] && kill "$POOL_PID" 2>/dev/null
  "$PG_BIN/pg_ctl" -D "$PGDATA" stop -m fast >/dev/null 2>&1
  redis-cli -p "$REDIS_PORT" shutdown nosave >/dev/null 2>&1
  if [ -x "$RBITCOIN_CLI" ]; then
    "$RBITCOIN_CLI" --datadir "$DATADIR" stop >/dev/null 2>&1
  fi
  [[ -n "${NODE_PID:-}" ]] && kill "$NODE_PID" 2>/dev/null
  wait 2>/dev/null
  [[ "${KEEP:-0}" == "1" ]] || rm -rf "$TMP"
}
trap cleanup EXIT

PG_BIN=$(dirname "$(command -v initdb)")

echo "==> rbitcoin-node (TDP on 127.0.0.1:$TP_PORT, cookie chain RPC on 127.0.0.1:$RPC_PORT)"
mkdir -p "$DATADIR"
RPC_USER=spc
RPC_PASS=$(python3 -c 'import secrets; print(secrets.token_hex(16))')
printf '%s:%s' "$RPC_USER" "$RPC_PASS" >"$COOKIE" # no trailing newline (Core)
chmod 600 "$COOKIE"                               # node rejects wider modes
python3 -c 'import secrets; print(secrets.token_hex(32))' >"$TP_KEY"
"$RBITCOIN_NODE" --network regtest --datadir "$DATADIR" \
  --rpc --rpc-listen "127.0.0.1:$RPC_PORT" --rpc-cookie-file "$COOKIE" \
  --sv2-tp-listen "127.0.0.1:$TP_PORT" --sv2-tp-authority-sec-file "$TP_KEY" \
  --regtest-pow-bits "$POW_BITS" >"$NODE_LOG" 2>&1 &
NODE_PID=$!

rpc() {
  curl -sf -u "$RPC_USER:$RPC_PASS" -H 'content-type: application/json' \
    -d "$1" "http://127.0.0.1:$RPC_PORT/"
}
rpc_result() {
  local out
  out=$(rpc "$1") || { echo "FAIL: RPC transport: $1" >&2; tail -20 "$NODE_LOG" >&2; exit 1; }
  echo "$out" | jq -e '.error == null' >/dev/null || {
    echo "FAIL: RPC error for $1" >&2; echo "$out" >&2; exit 1; }
  echo "$out" | jq -r '.result'
}

for _ in $(seq 1 80); do
  rpc '{"jsonrpc":"1.0","id":"h","method":"getblockcount","params":[]}' >/dev/null 2>&1 && break
  sleep 0.25
done
HEIGHT=$(rpc_result '{"jsonrpc":"1.0","id":"h","method":"getblockcount","params":[]}')
[ "$HEIGHT" = "0" ] || { echo "FAIL: fresh regtest datadir at height $HEIGHT" >&2; exit 1; }

# The TP logs its authority key in key-utils base58check form. The pool
# authenticates Noise_NX with exactly that key, never a wildcard.
TP_AUTHORITY=""
for _ in $(seq 1 80); do
  TP_AUTHORITY=$(sed -n 's/.*authority pubkey \([^)]*\)).*/\1/p' "$NODE_LOG" | head -1 || true)
  [ -n "$TP_AUTHORITY" ] && break
  sleep 0.25
done
if [ -z "$TP_AUTHORITY" ]; then
  echo "FAIL: no 'authority pubkey' line in rbitcoin-node log" >&2
  tail -30 "$NODE_LOG" >&2
  exit 1
fi
echo "TP authority pubkey: $TP_AUTHORITY"

echo "==> fund chain to maturity (101 blocks to the fixed P2TR script)"
rpc_result '{"jsonrpc":"1.0","id":"h","method":"generatetoaddress","params":[101,"'"$POOL_SCRIPT"'"]}' >/dev/null
HEIGHT=$(rpc_result '{"jsonrpc":"1.0","id":"h","method":"getblockcount","params":[]}')
[ "$HEIGHT" = "101" ] || { echo "FAIL: funding stopped at height $HEIGHT" >&2; exit 1; }

echo "==> postgres + redis"
initdb -D "$PGDATA" -A trust -U postgres >/dev/null
"$PG_BIN/pg_ctl" -D "$PGDATA" -o "-p $PG_PORT -h 127.0.0.1 -k $TMP" -l "$TMP/pg.log" start >/dev/null
for _ in $(seq 1 60); do
  "$PG_BIN/pg_isready" -h 127.0.0.1 -p "$PG_PORT" >/dev/null 2>&1 && break
  sleep 0.3
done
"$PG_BIN/createdb" -h 127.0.0.1 -p "$PG_PORT" -U postgres spc
redis-server --port "$REDIS_PORT" --daemonize yes --save '' --appendonly no --dir "$TMP" >/dev/null

echo "==> building"
cargo build -q -p spc-pool --features rpc --bin spc-pool --manifest-path "$ROOT/Cargo.toml"
cargo build -q -p spc-scan --features rpc --bin spc-scan --manifest-path "$ROOT/Cargo.toml"

MINER_JSON=$("$ROOT/target/debug/spc-scan" gen-address --network regtest)
MINER_ADDR=$(echo "$MINER_JSON" | jq -r .address)
MINER_KEY=$(echo "$MINER_JSON" | jq -r .scan_key)

cat > "$POOL_CONFIG" <<EOF
# Same dev settings as testing/regtest-services-e2e.sh; the template source
# is the node's SV2 Template Provider, the RPC endpoint is settlement-only.
[pool]
network = "regtest"
roles = ["front", "payout", "api"]
listen = "127.0.0.1:$SV2_PORT"
authority_public_key = "9auqWEzQDVyd2oe1JVGFLMLHZtCo2FFqZwtKA5gd9xbuEu7PH72"
authority_secret_key = "mkDLTBBRxdBv998612qipDYoTK3YUrqLe8uWw7gu3iXbSrn2n"
pool_payout_descriptor = "tr(tpubD6NzVbkrYhZ4WUjXBVxZPvGhWxTRE5veAscTgokX65xcfCoDF5nyNdtaS4xGoJ7xnZuDTxCAEAnGpxJbWo6wtTAJsESmeJWpZzybZ6dvCia/0/*)"
pool_payout_script = "$POOL_SCRIPT"
fee_bps = 100
front_id = 0

[template]
source = "sv2-tp"
sv2_tp_address = "127.0.0.1:$TP_PORT"
sv2_tp_authority_pubkey = "$TP_AUTHORITY"
# CHAIN VIEW ONLY for settlement: confirmations, orphan checks, coinbase
# reads and getblockstats. NO getblocktemplate and NO submitblock here.
rpc_url = "http://127.0.0.1:$RPC_PORT"
rpc_user = "$RPC_USER"
rpc_pass = "$RPC_PASS"
extranonce_bytes = 16

[sv1]
enabled = true
listen = "127.0.0.1:$SV1_PORT"
extranonce1_bytes = 4
extranonce2_bytes = 4

[jd]
listen = "127.0.0.1:$JD_PORT"

[accounting]
# Dev-only: depth 1 settles after the single block this harness mines.
confirmation_depth = 1

[storage]
postgres = "postgres://postgres@127.0.0.1:$PG_PORT/spc"
redis = "redis://127.0.0.1:$REDIS_PORT"

[api]
listen = "127.0.0.1:$API_PORT"
EOF

echo "==> spc-pool (front,payout,api over TDP)"
"$ROOT/target/debug/spc-pool" --config "$POOL_CONFIG" --roles front,payout,api >"$POOL_LOG" 2>&1 &
POOL_PID=$!
READY=0
for _ in $(seq 1 80); do
  if grep -q "sv2-tp first template ready" "$POOL_LOG" 2>/dev/null; then
    READY=1
    break
  fi
  if ! kill -0 "$POOL_PID" 2>/dev/null; then break; fi
  sleep 0.25
done
if [ "$READY" != "1" ]; then
  echo "FAIL: no sv2-tp template reached the pool" >&2
  tail -60 "$POOL_LOG" >&2
  exit 1
fi
for _ in $(seq 1 80); do
  (echo > /dev/tcp/127.0.0.1/$SV1_PORT) >/dev/null 2>&1 && break
  sleep 0.5
done

echo "==> miner"
BEFORE=$(rpc_result '{"jsonrpc":"1.0","id":"h","method":"getblockcount","params":[]}')
python3 "$ROOT/testing/regtest-miner.py" --host 127.0.0.1 --port "$SV1_PORT" \
  --address "$MINER_ADDR" --shares 2 >"$MINER_LOG" 2>&1 || { echo "FAIL: miner" >&2; cat "$MINER_LOG" >&2; exit 1; }
sleep 2
AFTER=$(rpc_result '{"jsonrpc":"1.0","id":"h","method":"getblockcount","params":[]}')
echo "blocks: $BEFORE -> $AFTER"
if [ "$AFTER" -le "$BEFORE" ]; then
  echo "FAIL: no block was accepted through the TDP path" >&2
  tail -60 "$POOL_LOG" >&2
  exit 1
fi
if ! grep -q "sv2-tp: SubmitSolution accepted by the TP" "$POOL_LOG"; then
  echo "FAIL: height moved without a confirmed SubmitSolution" >&2
  tail -60 "$POOL_LOG" >&2
  exit 1
fi

echo "==> scan (assert the miner's silent-payment output is on-chain)"
"$ROOT/target/debug/spc-scan" find --network regtest --address "$MINER_ADDR" --scan-key "$MINER_KEY" \
  --from 1 --to "$AFTER" --rpc "http://127.0.0.1:$RPC_PORT" --rpc-user "$RPC_USER" --rpc-pass "$RPC_PASS" \
  >"$TMP/scan.log" 2>&1 || { echo "FAIL: spc-scan" >&2; cat "$TMP/scan.log" >&2; exit 1; }
cat "$TMP/scan.log"
FOUND=$(jq -s '[.[].found | length] | add // 0' < <(grep '"found"' "$TMP/scan.log"))
if [ "$FOUND" -lt 1 ]; then
  echo "FAIL: scan found no miner output in blocks 1..$AFTER" >&2
  tail -50 "$POOL_LOG" >&2
  exit 1
fi
echo "found outputs: $FOUND"

echo "==> payout/API (settled on the RPC chain view, not on TDP)"
CONFIRMED=0
for _ in $(seq 1 60); do
  CONFIRMED=$(curl -sf "http://127.0.0.1:$API_PORT/api/pool/blocks?limit=5" \
    | jq '[.blocks[] | select(.status == "confirmed")] | length' 2>/dev/null || echo 0)
  [ "$CONFIRMED" -ge 1 ] && break
  sleep 0.5
done
curl -sf "http://127.0.0.1:$API_PORT/api/pool/blocks?limit=5" | jq .
if [ "$CONFIRMED" -lt 1 ]; then
  echo "FAIL: the pool's block never reached status=confirmed" >&2
  tail -80 "$POOL_LOG" >&2
  exit 1
fi
LIAB=$(curl -sf "http://127.0.0.1:$API_PORT/api/pool/liability" | jq -r .sats)
echo "confirmed blocks: $CONFIRMED, liability=$LIAB"

echo "OK: rbitcoin SV2-TP full-stack regtest e2e passed"
