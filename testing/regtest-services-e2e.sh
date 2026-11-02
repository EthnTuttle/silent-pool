#!/usr/bin/env bash
# Full-stack regtest: bitcoind + Postgres + Redis + spc-pool front/payout/api.
# Requires bitcoind/bitcoin-cli, postgres tools, redis-server, python3, jq, curl.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
TMP=$(mktemp -d)
DATADIR="$TMP/bitcoin"
PGDATA="$TMP/pg"
POOL_CONFIG="$TMP/spc-pool.toml"
POOL_LOG="$TMP/pool.log"
RPC_PORT=18443
PG_PORT=55432
REDIS_PORT=56379
SV1_PORT=43333
SV2_PORT=43454
JD_PORT=43464
API_PORT=48080
RPC_USER=spc
RPC_PASS=spc

cleanup() {
  set +e
  [[ -n "${POOL_PID:-}" ]] && kill "$POOL_PID" 2>/dev/null
  "$PG_BIN/pg_ctl" -D "$PGDATA" stop -m fast >/dev/null 2>&1
  redis-cli -p "$REDIS_PORT" shutdown nosave >/dev/null 2>&1
  bitcoin-cli -regtest -datadir="$DATADIR" -rpcuser=$RPC_USER -rpcpassword=$RPC_PASS stop >/dev/null 2>&1
  wait 2>/dev/null
  [[ "${KEEP:-0}" == "1" ]] || rm -rf "$TMP"
}
trap cleanup EXIT

PG_BIN=$(dirname "$(command -v initdb)")
echo "==> bitcoind"
mkdir -p "$DATADIR"
bitcoind -regtest -datadir="$DATADIR" -server \
  -rpcuser=$RPC_USER -rpcpassword=$RPC_PASS -rpcport=$RPC_PORT \
  -fallbackfee=0.0001 -txindex -daemon >/dev/null
for _ in $(seq 1 60); do
  bitcoin-cli -regtest -datadir="$DATADIR" -rpcuser=$RPC_USER -rpcpassword=$RPC_PASS getblockchaininfo >/dev/null 2>&1 && break
  sleep 0.5
done
CLI="bitcoin-cli -regtest -datadir=$DATADIR -rpcuser=$RPC_USER -rpcpassword=$RPC_PASS"
$CLI createwallet spc >/dev/null
$CLI generatetoaddress 101 "$($CLI getnewaddress)" >/dev/null

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
cargo build -q -p spc-pool --features rpc --bin spc-pool
cargo build -q -p spc-scan --features rpc --bin spc-scan

POOL_ADDR=$($CLI getnewaddress "" bech32m)
POOL_SCRIPT=$($CLI getaddressinfo "$POOL_ADDR" | jq -r .scriptPubKey)
MINER_JSON=$("$ROOT/target/debug/spc-scan" gen-address --network regtest)
MINER_ADDR=$(echo "$MINER_JSON" | jq -r .address)
MINER_KEY=$(echo "$MINER_JSON" | jq -r .scan_key)

cat > "$POOL_CONFIG" <<EOF2
[pool]
network = "regtest"
roles = ["front", "payout", "api"]
listen = "127.0.0.1:$SV2_PORT"
authority_public_key = "02aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
pool_payout_descriptor = "tr(tpubD6NzVbkrYhZ4YTest/0/*)"
pool_payout_script = "$POOL_SCRIPT"
fee_bps = 100
front_id = 0

[template]
source = "rpc"
rpc_url = "http://127.0.0.1:$RPC_PORT"
rpc_user = "$RPC_USER"
rpc_pass = "$RPC_PASS"

[sv1]
enabled = true
listen = "127.0.0.1:$SV1_PORT"
extranonce1_bytes = 4
extranonce2_bytes = 4

[jd]
listen = "127.0.0.1:$JD_PORT"

[accounting]
confirmation_depth = 6

[storage]
postgres = "postgres://postgres@127.0.0.1:$PG_PORT/spc"
redis = "redis://127.0.0.1:$REDIS_PORT"

[api]
listen = "127.0.0.1:$API_PORT"
EOF2

echo "==> spc-pool (front,payout,api)"
"$ROOT/target/debug/spc-pool" --config "$POOL_CONFIG" --roles front,payout,api >"$POOL_LOG" 2>&1 &
POOL_PID=$!
for _ in $(seq 1 80); do
  (echo > /dev/tcp/127.0.0.1/$SV1_PORT) >/dev/null 2>&1 && break
  sleep 0.5
done

echo "==> miner"
BEFORE=$($CLI getblockcount)
python3 "$ROOT/testing/regtest-miner.py" --host 127.0.0.1 --port "$SV1_PORT" \
  --address "$MINER_ADDR" --shares 2 >"$TMP/miner.log" 2>&1 || { cat "$TMP/miner.log"; exit 1; }
sleep 2
AFTER=$($CLI getblockcount)
echo "blocks: $BEFORE -> $AFTER"
[ "$AFTER" -gt "$BEFORE" ] || { echo "FAIL: no block"; tail -60 "$POOL_LOG"; exit 1; }

echo "==> payout/API"
for _ in $(seq 1 40); do
  LIAB=$(curl -sf "http://127.0.0.1:$API_PORT/api/pool/liability" | jq -r .sats 2>/dev/null || echo x)
  [ "$LIAB" != "x" ] && break
  sleep 0.5
done
echo "liability=$LIAB"
curl -sf "http://127.0.0.1:$API_PORT/api/pool/blocks?limit=5" | jq .
BLOCKS=$(curl -sf "http://127.0.0.1:$API_PORT/api/pool/blocks?limit=5" | jq '.blocks | length')
[ "$BLOCKS" -ge 1 ] || { echo "FAIL: payout did not settle a block"; tail -80 "$POOL_LOG"; exit 1; }

echo "==> scan"
"$ROOT/target/debug/spc-scan" find --address "$MINER_ADDR" --scan-key "$MINER_KEY" \
  --from 1 --to "$AFTER" --rpc "http://127.0.0.1:$RPC_PORT" --rpc-user "$RPC_USER" --rpc-pass "$RPC_PASS" \
  | tee "$TMP/scan.log"

echo "OK: full-stack regtest e2e passed"
