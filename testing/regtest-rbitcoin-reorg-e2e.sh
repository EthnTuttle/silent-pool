#!/usr/bin/env bash
# Full-stack regtest against rbitcoin-node instead of bitcoind: node + Postgres
# + Redis + spc-pool (front, payout, api), exercising SPEC §8.4 step 4:
# "invalidate the found block -> status='orphaned', no ledger rows, pending
# unchanged."
#
# Scenario and why the assertions are the right invariants:
#
#   confirmation_depth = 2 (dev-only; production is 6). Depth >= 2 is what
#   makes the orphan leg deterministic: a freshly found pool block can never
#   reach the gate's Confirm branch before this script invalidates it, so the
#   block event stays pending until a competitor is confirmed at its height.
#
#   Leg 1 (settle):  the pool finds block(s), this script mines 2 blocks on
#     top, and the block event must book status='confirmed' with ledger rows.
#     Proves the settlement path (chain gate, coinbase/economics reads via
#     getblock/getblockstats) works against rbitcoin's RPC.
#   Leg 2 (orphan):  the pool finds block B; this script calls
#     invalidateblock on B and mines a competing branch across B's height.
#     The payout role's chain gate must then book B status='orphaned' with
#     ZERO ledger rows (SPEC §7.4).
#
# Assertions after the orphan:
#   - /api/pool/blocks reports B with status "orphaned"     (API reflects it)
#   - exactly ONE block_found row exists for B's height     (booked once)
#   - ledger_history and ark_payout row counts are EXACTLY what they were
#     after leg 1 - the orphan books nothing, and re-processing the event is
#     absorbed, so the payout is never booked (neither once nor twice)
#   - /api/pool/liability is unchanged from after leg 1     ("pending
#     unchanged" - orphaned payout owes the miner nothing but must not take
#     anything away either)
#   - the pool keeps producing blocks on the competing branch afterwards.
#
# Env: RBITCOIN_NODE / RBITCOIN_CLI (see regtest-rbitcoin-e2e.sh), KEEP=1
# keeps the temp dir. Ports differ from both the bitcoind harnesses and
# testing/regtest-rbitcoin-e2e.sh so all of them can run side by side.
set -euo pipefail

RBITCOIN_NODE="${RBITCOIN_NODE:-/home/ethan/code/rbitcoin/target/dev/debug/rbitcoin-node}"
RBITCOIN_CLI="${RBITCOIN_CLI:-$(dirname "$RBITCOIN_NODE")/rbitcoin-cli}"
if [ ! -x "$RBITCOIN_NODE" ]; then
  echo "SKIP: rbitcoin-node not found at $RBITCOIN_NODE (set RBITCOIN_NODE)"
  exit 0
fi
command -v initdb >/dev/null || { echo "FAIL: initdb not in PATH (postgres tools required)" >&2; exit 1; }
command -v redis-server >/dev/null || { echo "FAIL: redis-server not in PATH" >&2; exit 1; }

ROOT=$(cd "$(dirname "$0")/.." && pwd)
TMP=$(mktemp -d)
DATADIR="$TMP/rbitcoin"
PGDATA="$TMP/pg"
COOKIE="$TMP/rpc.cookie"
POOL_CONFIG="$TMP/spc-pool.toml"
POOL_LOG="$TMP/pool.log"
NODE_LOG="$TMP/node.log"
RPC_PORT=29444
PG_PORT=55433
REDIS_PORT=56380
SV1_PORT=34334
SV2_PORT=34455
JD_PORT=34464
API_PORT=29081
POW_BITS=0x200fffff
# Fixed, valid regtest P2TR script; rbitcoin's generatetoaddress accepts a
# bare hex script, so no wallet is involved anywhere in this harness.
POOL_SCRIPT=5120a60869f0dbcf1dc659c9cecbaf8050135ea9e8cdc487053f1dc6880949dc684c

cleanup() {
  set +e
  [[ -n "${POOL_PID:-}" ]] && kill "$POOL_PID" 2>/dev/null
  "${PG_BIN:-pg_ctl}" -D "$PGDATA" stop -m fast >/dev/null 2>&1
  redis-cli -p "$REDIS_PORT" shutdown nosave >/dev/null 2>&1
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
RPC_USER=$(cut -d: -f1 "$COOKIE")
RPC_PASS=$(cut -d: -f2- "$COOKIE")

rpc() {
  curl -sf -u "$RPC_USER:$RPC_PASS" -H 'content-type: application/json' \
    -d "$1" "http://127.0.0.1:$RPC_PORT/"
}
rpc_result() {
  local out m="$1"
  out=$(rpc "$m") || { echo "FAIL: RPC transport: $m" >&2; tail -20 "$NODE_LOG" >&2; exit 1; }
  echo "$out" | jq -e '.error == null' >/dev/null || {
    echo "FAIL: RPC error for $m" >&2; echo "$out" >&2; exit 1; }
  echo "$out" | jq -r '.result'
}
getblockcount()  { rpc_result '{"jsonrpc":"1.0","id":"h","method":"getblockcount","params":[]}'; }
getblockhash()   { rpc_result '{"jsonrpc":"1.0","id":"h","method":"getblockhash","params":['"$1"']}'; }
# Mine one pool block attempt batch; retry up to 3 batches until the chain tip
# advances (accepted shares meet the network target almost always on regtest,
# but do not assume it).
mine_pool_block() {
  local before after i
  before=$(getblockcount)
  for i in 1 2 3; do
    python3 "$ROOT/testing/regtest-miner.py" --host 127.0.0.1 --port "$SV1_PORT" \
      --address "$MINER_ADDR" --shares 2 >"$TMP/miner.$i.log" 2>&1 \
      || { echo "FAIL: miner (batch $i)" >&2; cat "$TMP/miner.$i.log" >&2; exit 1; }
    sleep 1
    after=$(getblockcount)
    [ "$after" -gt "$before" ] && { echo "$after"; return 0; }
  done
  echo "FAIL: pool did not submit a block in 3 miner batches" >&2
  tail -50 "$POOL_LOG" >&2
  exit 1
}
# Poll an expression until it is truthy or the timeout (seconds) expires.
wait_for() {
  local what="$1" timeout="$2" expr="$3" i
  for i in $(seq 1 "$timeout"); do
    if eval "$expr" 2>/dev/null; then return 0; fi
    sleep 1
  done
  echo "FAIL: timed out (${timeout}s) waiting for: $what" >&2
  tail -60 "$POOL_LOG" >&2
  return 1
}
blocks_api() { curl -sf "http://127.0.0.1:$API_PORT/api/pool/blocks?limit=50"; }
status_at_height() {
  blocks_api | jq -r '.blocks[] | select(.height == '"$1"') | .status' 2>/dev/null | head -1
}
liability() { curl -sf "http://127.0.0.1:$API_PORT/api/pool/liability" | jq -r .sats; }
psqlq() { "$PG_BIN/psql" -h 127.0.0.1 -p "$PG_PORT" -U postgres -d spc -tAXc "$1"; }

for _ in $(seq 1 80); do
  rpc '{"jsonrpc":"1.0","id":"h","method":"getblockcount","params":[]}' >/dev/null 2>&1 && break
  sleep 0.25
done
[ "$(getblockcount)" = "0" ] || { echo "FAIL: fresh regtest datadir not at height 0" >&2; exit 1; }

echo "==> fund chain to maturity (101 blocks to the fixed P2TR script)"
rpc_result '{"jsonrpc":"1.0","id":"h","method":"generatetoaddress","params":[101,"'"$POOL_SCRIPT"'"]}' >/dev/null
[ "$(getblockcount)" = "101" ] || { echo "FAIL: funding did not reach 101" >&2; exit 1; }

PG_BIN=$(dirname "$(command -v initdb)")
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

cat > "$POOL_CONFIG" <<EOF
# Same dev settings as testing/regtest-services-e2e.sh, except the template
# endpoint (rbitcoin cookie RPC) and confirmation_depth = 2 so the orphan leg
# below is deterministic (production default is 6, see default_confirmation_depth).
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
confirmation_depth = 2

[storage]
postgres = "postgres://postgres@127.0.0.1:$PG_PORT/spc"
redis = "redis://127.0.0.1:$REDIS_PORT"

[api]
listen = "127.0.0.1:$API_PORT"
EOF

echo "==> spc-pool (front,payout,api)"
"$ROOT/target/debug/spc-pool" --config "$POOL_CONFIG" --roles front,payout,api >"$POOL_LOG" 2>&1 &
POOL_PID=$!
for _ in $(seq 1 80); do
  (echo > /dev/tcp/127.0.0.1/$SV1_PORT) >/dev/null 2>&1 && break
  sleep 0.5
done

echo "==> leg 1: settle a found block (confirmed at confirmation_depth = 2)"
SETTLE_TIP=$(mine_pool_block)
echo "pool block(s) up to height $SETTLE_TIP"
rpc_result '{"jsonrpc":"1.0","id":"h","method":"generatetoaddress","params":[2,"'"$POOL_SCRIPT"'"]}' >/dev/null
wait_for "block at height $SETTLE_TIP booked confirmed" 120 \
  '[ "$(status_at_height '"$SETTLE_TIP"')" = "confirmed" ]'
echo "block at height $SETTLE_TIP: confirmed"
LIAB1=$(liability)
LH1=$(psqlq 'SELECT count(*) FROM ledger_history')
AP1=$(psqlq 'SELECT count(*) FROM ark_payout')
BF1=$(psqlq "SELECT count(*) FROM block_found WHERE height = $SETTLE_TIP AND status = 'confirmed'")
[ "$BF1" = "1" ] || { echo "FAIL: expected 1 confirmed block_found row at $SETTLE_TIP, got $BF1" >&2; exit 1; }
echo "after settle: liability=$LIAB1 ledger_rows=$LH1 ark_rows=$AP1"

echo "==> leg 2: invalidate the pool's block and mine a competing branch"
B_START=$(getblockcount)
B_END=$(mine_pool_block)
B_HEIGHT=$((B_START + 1)) # the pool's first new block; descendants (if any) go with it
B_HASH=$(getblockhash "$B_HEIGHT")
echo "pool's block to orphan: height $B_HEIGHT hash $B_HASH (pool mined to $B_END)"
rpc_result '{"jsonrpc":"1.0","id":"h","method":"invalidateblock","params":["'"$B_HASH"'"]}' >/dev/null
[ "$(getblockcount)" = "$B_START" ] || { echo "FAIL: invalidateblock did not roll back to $B_START" >&2; exit 1; }
rpc_result '{"jsonrpc":"1.0","id":"h","method":"generatetoaddress","params":['"$((B_END - B_START))"',"'"$POOL_SCRIPT"'"]}' >/dev/null
CB_HASH=$(getblockhash "$B_HEIGHT")
[ "$CB_HASH" != "$B_HASH" ] || { echo "FAIL: competitor at $B_HEIGHT equals the invalidated block" >&2; exit 1; }
echo "competing branch: height $B_HEIGHT hash $CB_HASH"

wait_for "block at height $B_HEIGHT booked orphaned" 120 \
  '[ "$(status_at_height '"$B_HEIGHT"')" = "orphaned" ]'
sleep 20 # let a full pending-retry cycle elapse: re-delivery must be absorbed

echo "==> assert the reorg invariants"
STATUS=$(status_at_height "$B_HEIGHT")
[ "$STATUS" = "orphaned" ] || { echo "FAIL: API status at $B_HEIGHT is '$STATUS'" >&2; exit 1; }
BF_ORPHANED=$(psqlq "SELECT count(*) FROM block_found WHERE height = $B_HEIGHT AND status = 'orphaned'")
BF_TOTAL=$(psqlq "SELECT count(*) FROM block_found WHERE height = $B_HEIGHT")
[ "$BF_TOTAL" = "1" ] && [ "$BF_ORPHANED" = "1" ] \
  || { echo "FAIL: block_found rows at $B_HEIGHT: total=$BF_TOTAL orphaned=$BF_ORPHANED (want 1/1)" >&2; exit 1; }
LH2=$(psqlq 'SELECT count(*) FROM ledger_history')
AP2=$(psqlq 'SELECT count(*) FROM ark_payout')
[ "$LH2" = "$LH1" ] || { echo "FAIL: ledger rows grew for an orphan: $LH1 -> $LH2" >&2; exit 1; }
[ "$AP2" = "$AP1" ] || { echo "FAIL: ark payouts grew for an orphan: $AP1 -> $AP2" >&2; exit 1; }
LIAB2=$(liability)
[ "$LIAB2" = "$LIAB1" ] || { echo "FAIL: liability changed across the orphan: $LIAB1 -> $LIAB2" >&2; exit 1; }
echo "orphan booked once, no ledger rows, liability unchanged ($LIAB1)"

echo "==> pool still produces on the competing branch"
LIVE_BEFORE=$(getblockcount)
LIVE_AFTER=$(mine_pool_block)
[ "$LIVE_AFTER" -gt "$LIVE_BEFORE" ] || { echo "FAIL: pool stalled after the reorg" >&2; exit 1; }
echo "blocks: $LIVE_BEFORE -> $LIVE_AFTER"

echo "OK: rbitcoin regtest reorg e2e passed"
