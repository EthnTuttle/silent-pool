#!/usr/bin/env bash
# Regtest end-to-end: bitcoind (RPC template source) + spc-pool front with both
# native listeners + a synthetic SV1 miner. Requires bitcoind/bitcoin-cli,
# python3, jq and curl in PATH (the Nix devshell provides them).
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
TMP=$(mktemp -d)
DATADIR="$TMP/bitcoin"
POOL_CONFIG="$TMP/spc-pool.toml"
POOL_LOG="$TMP/pool.log"
MINER_LOG="$TMP/miner.log"
RPC_PORT=18443
SV1_PORT=43333
SV2_PORT=43454
RPC_USER=spc
RPC_PASS=spc

cleanup() {
  set +e
  [[ -n "${POOL_PID:-}" ]] && kill "$POOL_PID" 2>/dev/null
  bitcoin-cli -regtest -datadir="$DATADIR" -rpcuser=$RPC_USER -rpcpassword=$RPC_PASS stop >/dev/null 2>&1
  wait 2>/dev/null
  [[ "${KEEP:-0}" == "1" ]] || rm -rf "$TMP"
}
trap cleanup EXIT

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

echo "==> building"
cargo build -q -p spc-pool --features rpc --bin spc-pool
cargo build -q -p spc-scan --features rpc --bin spc-scan

POOL_ADDR=$($CLI getnewaddress "" bech32m)
POOL_SCRIPT=$($CLI getaddressinfo "$POOL_ADDR" | jq -r .scriptPubKey)
MINER_JSON=$("$ROOT/target/debug/spc-scan" gen-address --network regtest)
MINER_ADDR=$(echo "$MINER_JSON" | jq -r .address)
MINER_KEY=$(echo "$MINER_JSON" | jq -r .scan_key)

cat > "$POOL_CONFIG" <<EOF
[pool]
network = "regtest"
roles = ["front"]
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
listen = "127.0.0.1:18080"
EOF

echo "==> spc-pool"
"$ROOT/target/debug/spc-pool" --config "$POOL_CONFIG" --roles front >"$POOL_LOG" 2>&1 &
POOL_PID=$!
for _ in $(seq 1 60); do
  (echo > /dev/tcp/127.0.0.1/$SV1_PORT) >/dev/null 2>&1 && break
  sleep 0.5
done

echo "==> miner"
BEFORE=$($CLI getblockcount)
python3 "$ROOT/testing/regtest-miner.py" \
  --host 127.0.0.1 --port "$SV1_PORT" --address "$MINER_ADDR" --shares 2 \
  >"$MINER_LOG" 2>&1 || { cat "$MINER_LOG"; exit 1; }
sleep 1
AFTER=$($CLI getblockcount)
echo "blocks: $BEFORE -> $AFTER"
if [ "$AFTER" -le "$BEFORE" ]; then
  echo "FAIL: pool did not submit a block" >&2
  tail -50 "$POOL_LOG" >&2
  exit 1
fi

echo "==> scan"
"$ROOT/target/debug/spc-scan" find \
  --address "$MINER_ADDR" --scan-key "$MINER_KEY" \
  --from 1 --to "$AFTER" \
  --rpc "http://127.0.0.1:$RPC_PORT" --rpc-user "$RPC_USER" --rpc-pass "$RPC_PASS" \
  | tee "$TMP/scan.log"
if ! grep -q '"found":\[' "$TMP/scan.log"; then
  echo "WARN: scan did not print a found entry (first block has no miner output)"
fi

echo "OK: regtest e2e passed"
