#!/usr/bin/env bash
# Regtest end-to-end against rbitcoin-node instead of bitcoind: front-only
# spc-pool with [template] source = "rpc" over the node's cookie-authed TCP
# JSON-RPC, plus the synthetic SV1 miner. Mirrors testing/regtest-e2e.sh on
# disjoint ports so both harnesses can run side by side.
#
# rbitcoin reads an EXISTING Core-format `username:password` cookie (mode
# 0600, no trailing newline) from --rpc-cookie-file; this harness writes it
# before starting the node and reads rpc_user/rpc_pass back out of it.
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
POOL_CONFIG="$TMP/spc-pool.toml"
POOL_LOG="$TMP/pool.log"
NODE_LOG="$TMP/node.log"
MINER_LOG="$TMP/miner.log"
RPC_PORT=29443
SV1_PORT=34333
SV2_PORT=34454
POW_BITS=0x200fffff
# Fixed, valid regtest P2TR payout script (BIP86 test-vector output key, no
# wallet needed; rbitcoin's generatetoaddress accepts a bare hex script).
POOL_SCRIPT=5120a60869f0dbcf1dc659c9cecbaf8050135ea9e8cdc487053f1dc6880949dc684c

cleanup() {
  set +e
  [[ -n "${POOL_PID:-}" ]] && kill "$POOL_PID" 2>/dev/null
  if [ -x "$RBITCOIN_CLI" ]; then
    "$RBITCOIN_CLI" --datadir "$DATADIR" stop >/dev/null 2>&1
  fi
  [[ -n "${NODE_PID:-}" ]] && kill "$NODE_PID" 2>/dev/null
  wait 2>/dev/null
  [[ "${KEEP:-0}" == "1" ]] || rm -rf "$TMP"
}
trap cleanup EXIT

echo "==> rbitcoin-node (regtest, cookie RPC on 127.0.0.1:$RPC_PORT)"
mkdir -p "$DATADIR"
RPC_USER=spc
RPC_PASS=$(python3 -c 'import secrets; print(secrets.token_hex(16))')
printf '%s:%s' "$RPC_USER" "$RPC_PASS" >"$COOKIE" # cookie has no trailing newline
chmod 600 "$COOKIE"                               # node rejects wider modes
"$RBITCOIN_NODE" --network regtest --datadir "$DATADIR" \
  --rpc --rpc-listen "127.0.0.1:$RPC_PORT" --rpc-cookie-file "$COOKIE" \
  --regtest-pow-bits "$POW_BITS" >"$NODE_LOG" 2>&1 &
NODE_PID=$!

# Read the credentials back out of the cookie, as an operator would; the pool
# config and every RPC call below use exactly these.
RPC_USER=$(cut -d: -f1 "$COOKIE")
RPC_PASS=$(cut -d: -f2- "$COOKIE")

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

echo "==> fund chain to maturity (101 blocks to the fixed P2TR script)"
rpc_result '{"jsonrpc":"1.0","id":"h","method":"generatetoaddress","params":[101,"'"$POOL_SCRIPT"'"]}' >/dev/null
HEIGHT=$(rpc_result '{"jsonrpc":"1.0","id":"h","method":"getblockcount","params":[]}')
[ "$HEIGHT" = "101" ] || { echo "FAIL: funding stopped at height $HEIGHT" >&2; exit 1; }

echo "==> building"
cargo build -q -p spc-pool --features rpc --bin spc-pool --manifest-path "$ROOT/Cargo.toml"
cargo build -q -p spc-scan --features rpc --bin spc-scan --manifest-path "$ROOT/Cargo.toml"

MINER_JSON=$("$ROOT/target/debug/spc-scan" gen-address --network regtest)
MINER_ADDR=$(echo "$MINER_JSON" | jq -r .address)
MINER_KEY=$(echo "$MINER_JSON" | jq -r .scan_key)

cat > "$POOL_CONFIG" <<EOF
# Same dev settings as testing/regtest-e2e.sh; only the template endpoint and
# credentials differ (rbitcoin cookie auth instead of bitcoind rpcuser).
[pool]
network = "regtest"
roles = ["front"]
listen = "127.0.0.1:$SV2_PORT"
authority_public_key = "9auqWEzQDVyd2oe1JVGFLMLHZtCo2FFqZwtKA5gd9xbuEu7PH72"
authority_secret_key = "mkDLTBBRxdBv998612qipDYoTK3YUrqLe8uWw7gu3iXbSrn2n"
pool_payout_descriptor = "tr(tpubD6NzVbkrYhZ4WUjXBVxZPvGhWxTRE5veAscTgokX65xcfCoDF5nyNdtaS4xGoJ7xnZuDTxCAEAnGpxJbWo6wtTAJsESmeJWpZzybZ6dvCia/0/*)"
pool_payout_script = "$POOL_SCRIPT"
fee_bps = 100
front_id = 0

[template]
source = "rpc"
rpc_url = "http://127.0.0.1:$RPC_PORT"
rpc_user = "$RPC_USER"
rpc_pass = "$RPC_PASS"
extranonce_bytes = 16

# Regtest-only harness: no Postgres/Redis are expected to be running, so the
# front starts from an empty bootstrap snapshot instead of failing closed.
# NEVER set this on a network that matters.
[accounting]
allow_unchecked_chain = true

[sv1]
enabled = true
listen = "127.0.0.1:$SV1_PORT"
extranonce1_bytes = 4
extranonce2_bytes = 4

[storage]
postgres = "postgres://spc:spc@127.0.0.1:5432/spc"
redis = "redis://127.0.0.1:6379"

[api]
listen = "127.0.0.1:29080"
EOF

echo "==> spc-pool (front)"
"$ROOT/target/debug/spc-pool" --config "$POOL_CONFIG" --roles front >"$POOL_LOG" 2>&1 &
POOL_PID=$!
for _ in $(seq 1 60); do
  (echo > /dev/tcp/127.0.0.1/$SV1_PORT) >/dev/null 2>&1 && break
  sleep 0.5
done

echo "==> miner"
BEFORE=$(rpc_result '{"jsonrpc":"1.0","id":"h","method":"getblockcount","params":[]}')
python3 "$ROOT/testing/regtest-miner.py" \
  --host 127.0.0.1 --port "$SV1_PORT" --address "$MINER_ADDR" --shares 2 \
  >"$MINER_LOG" 2>&1 || { echo "FAIL: miner" >&2; cat "$MINER_LOG" >&2; exit 1; }
sleep 1
AFTER=$(rpc_result '{"jsonrpc":"1.0","id":"h","method":"getblockcount","params":[]}')
echo "blocks: $BEFORE -> $AFTER"
if [ "$AFTER" -le "$BEFORE" ]; then
  echo "FAIL: pool did not submit a block to rbitcoin-node" >&2
  tail -50 "$POOL_LOG" >&2
  exit 1
fi

echo "==> scan (assert the miner's silent-payment output is on-chain)"
"$ROOT/target/debug/spc-scan" find \
  --address "$MINER_ADDR" --scan-key "$MINER_KEY" \
  --from 1 --to "$AFTER" \
  --rpc "http://127.0.0.1:$RPC_PORT" --rpc-user "$RPC_USER" --rpc-pass "$RPC_PASS" \
  >"$TMP/scan.log" 2>&1 || { echo "FAIL: spc-scan" >&2; cat "$TMP/scan.log" >&2; exit 1; }
cat "$TMP/scan.log"
FOUND=$(jq -s '[.[].found | length] | add // 0' < <(grep '"found"' "$TMP/scan.log"))
if [ "$FOUND" -lt 1 ]; then
  echo "FAIL: scan found no miner output in blocks 1..$AFTER" >&2
  tail -50 "$POOL_LOG" >&2
  exit 1
fi
echo "found outputs: $FOUND"

echo "OK: rbitcoin regtest e2e passed"
