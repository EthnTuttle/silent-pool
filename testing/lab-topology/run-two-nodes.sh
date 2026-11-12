#!/usr/bin/env bash
# LP-topology: two real rbitcoin nodes in network namespaces, with an explicit
# P2P link that is partitioned and healed.
#
# The run uses the approved LP-4a namespace broker for the namespaces, the
# veth pair and the nftables fault, and the real rbitcoin node binary for the
# consensus.  Nothing here parses a generated configuration for placement:
# the addresses come from the broker's published manifest.
#
# The node binary, its sha256 and the provenance of this run are recorded in
# the artifact.  The run is PROVISIONAL: the runtime candidate it borrows the
# node contract from is not approved, and this driver makes no pool, miner,
# payout or consensus claim.
#
# Usage: testing/lab-topology/run-two-nodes.sh [--artifacts DIR]
#                                        [--node /path/to/rbitcoin-node]
set -uo pipefail

TEST_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(cd "$TEST_DIR/../.." && pwd)
LAB_DIR="$REPO_ROOT/scripts/lab"

ART=""
NODE_BIN="${SPC_LAB_RBITCOIN_NODE:-}"
while [ $# -gt 0 ]; do
    case "$1" in
        --artifacts) ART="$2"; shift 2 ;;
        --artifacts=*) ART="${1#*=}"; shift ;;
        --node) NODE_BIN="$2"; shift 2 ;;
        --node=*) NODE_BIN="${1#*=}"; shift ;;
        *) printf 'unknown argument: %s\n' "$1" >&2; exit 2 ;;
    esac
done
if [ -z "$ART" ]; then
    ART="${SPC_LAB_RUN_ROOT:-${XDG_RUNTIME_DIR:-/tmp}/spc-lab}/LP-topology-$(date -u +%Y%m%dT%H%M%SZ)-$(od -An -tx1 -N3 /dev/urandom | tr -d ' \n')"
fi
mkdir -p "$ART"

RUN_DIR="$ART/run"
POW_BITS="${SPC_LAB_POW_BITS:-0x200fffff}"
RPC_PORT=18443
P2P_PORT=18444

say() { printf '[topology] %s\n' "$*"; }
fail() { printf '[topology] FAIL %s\n' "$*" >&2; exit 1; }

[ -x "$NODE_BIN" ] || fail "node binary not set: pass --node or set SPC_LAB_RBITCOIN_NODE to a rbitcoin-node built from the pinned source (see UPSTREAM_DEPS.md); got '$NODE_BIN'"
command -v nsenter >/dev/null || fail "nsenter is missing"
command -v python3 >/dev/null || fail "python3 is missing"

NODE_SHA=$(sha256sum "$NODE_BIN" | cut -d' ' -f1)
say "node binary: $NODE_BIN"
say "node sha256: $NODE_SHA"

# ---------------------------------------------------------------- the topology
# Two nodes and the P2P link between them.  The plan mirrors the Rust
# `TopologyPlan`: a node is an endpoint, and the consensus link joins them.
cat >"$ART/topology.json" <<'JSON'
{
  "schema": "spc.lab.link_topology.v1",
  "endpoints": [
    {"name": "node-a", "role": "node"},
    {"name": "node-b", "role": "node"}
  ],
  "links": [
    {"id": "p2p-ab", "a": "node-a", "b": "node-b"}
  ]
}
JSON

say "starting the namespace broker"
"$LAB_DIR/link-fault.sh" up --spec "$ART/topology.json" --run-dir "$RUN_DIR" \
    >"$ART/up.json" 2>&1 || { cat "$ART/up.json"; fail "the namespace broker did not start"; }

MANIFEST="$RUN_DIR/manifest.json"
[ -f "$MANIFEST" ] || fail "the broker published no manifest"
BROKER_PID=$(jq -r '.supervisor.pid' "$MANIFEST")
NS_A=$(jq -r '.endpoints[]|select(.name=="node-a")|.namespace.netns_path' "$MANIFEST")
NS_B=$(jq -r '.endpoints[]|select(.name=="node-b")|.namespace.netns_path' "$MANIFEST")
ADDR_A=$(jq -r '.links[]|select(.id=="p2p-ab")|.addresses["node-a"]' "$MANIFEST")
ADDR_B=$(jq -r '.links[]|select(.id=="p2p-ab")|.addresses["node-b"]' "$MANIFEST")
say "broker pid=$BROKER_PID"
say "node-a $ADDR_A in $NS_A, node-b $ADDR_B in $NS_B"

cleanup() {
    for pid in "${NODE_A_PID:-}" "${NODE_B_PID:-}"; do
        [ -n "$pid" ] && kill -TERM "$pid" 2>/dev/null
    done
    sleep 1
    for pid in "${NODE_A_PID:-}" "${NODE_B_PID:-}"; do
        [ -n "$pid" ] && kill -KILL "$pid" 2>/dev/null
    done
    "$LAB_DIR/link-fault.sh" down --run-dir "$RUN_DIR" >"$ART/down.json" 2>&1
}
trap cleanup EXIT

# ------------------------------------------------------------- launch the nodes
# The nodes run inside the broker's namespaces.  The launcher joins the
# broker's user namespace (which owns the namespaces) and keeps the host
# network namespace, so the controller stays reachable and no host interface is
# touched.
launch_node() {
    # launch_node <name> <netns> <p2p addr> <connect addr or empty>
    local name="$1" netns="$2" p2p="$3" connect="$4"
    local datadir="$ART/private/node-$name"
    mkdir -p "$datadir"
    chmod 700 "$datadir"
    printf 'rpcuser:rpcpass' >"$ART/private/node-rpc-$name.cookie"
    chmod 600 "$ART/private/node-rpc-$name.cookie"
    local -a args=(
        --network regtest
        --datadir "$datadir"
        --rpc
        --rpc-listen "127.0.0.1:$RPC_PORT"
        --rpc-cookie-file "$ART/private/node-rpc-$name.cookie"
        --regtest-pow-bits "$POW_BITS"
        --listen "$p2p:$P2P_PORT"
        --no-seeds
        --no-discover
        --max-outbound 1
        --api-log "$ART/private/node-api-$name.jsonl"
        --log-level info
    )
    [ -n "$connect" ] && args+=(--connect "$connect:$P2P_PORT")
    nsenter -U --preserve-credentials -t "$BROKER_PID" -- \
        nsenter --net="$netns" -- "$NODE_BIN" "${args[@]}" \
        >"$ART/node-$name.log" 2>&1 &
    echo $!
}

say "launching node-a and node-b"
NODE_A_PID=$(launch_node node-a "$NS_A" "$ADDR_A" "$ADDR_B")
NODE_B_PID=$(launch_node node-b "$NS_B" "$ADDR_B" "")
say "node-a pid=$NODE_A_PID node-b pid=$NODE_B_PID"

# ------------------------------------------------------------------- observe
# The RPC is namespace local, so it is called from inside the namespace.
rpc_in_ns() {
    local netns="$1" method="$2" params="${3:-[]}"
    nsenter -U --preserve-credentials -t "$BROKER_PID" -- \
        nsenter --net="$netns" -- python3 -c '
import base64, json, sys, urllib.request
method, params = sys.argv[1], sys.argv[2]
request = urllib.request.Request(
    "http://127.0.0.1:18443/",
    data=json.dumps({"jsonrpc": "1.0", "id": 1, "method": method, "params": json.loads(params)}).encode(),
    headers={"Content-Type": "application/json",
             "Authorization": "Basic " + base64.b64encode(b"rpcuser:rpcpass").decode()},
)
try:
    with urllib.request.urlopen(request, timeout=5) as response:
        print(response.read().decode())
except Exception as error:
    print(json.dumps({"error": str(error)}))
' "$method" "$params" 2>/dev/null
}

mine() {
    # mine <netns> <count> -> the node mines blocks on its own chain
    rpc_in_ns "$1" generate "[$2]"
}

wait_ready() {
    # wait_ready <netns> <name> <timeout_s>
    local netns="$1" name="$2" timeout="$3" waited=0
    while [ "$waited" -lt $((timeout * 2)) ]; do
        if rpc_in_ns "$netns" getblockcount | grep -q '"result"'; then
            return 0
        fi
        sleep 0.5
        waited=$((waited + 1))
    done
    return 1
}

observe() {
    # observe <label> -> a JSON object with the per node canonical view
    local label="$1"
    local a_count a_hash b_count b_hash
    a_count=$(rpc_in_ns "$NS_A" getblockcount | jq -r '.result // "unknown"')
    a_hash=$(rpc_in_ns "$NS_A" getblockhash "{\"height\":$a_count}" | jq -r '.result // "unknown"')
    b_count=$(rpc_in_ns "$NS_B" getblockcount 2>/dev/null | jq -r '.result // "unknown"')
    b_hash=$(rpc_in_ns "$NS_B" getblockhash "{\"height\":$b_count}" | jq -r '.result // "unknown"')
    jq -nc --arg lbl "$label" \
        --argjson a_height "${a_count:-0}" --arg a_hash "${a_hash:-unknown}" \
        --argjson b_height "${b_count:-0}" --arg b_hash "${b_hash:-unknown}" \
        '{label:$lbl, nodes:{"node-a":{height:$a_height, hash:$a_hash}, "node-b":{height:$b_height, hash:$b_hash}}}'
}

say "waiting for the node RPC"
wait_ready "$NS_A" node-a 60 || fail "node-a RPC did not answer"
wait_ready "$NS_B" node-b 60 || fail "node-b RPC did not answer"
say "both nodes answer their RPC"

# Let the nodes connect to each other, then produce blocks so the P2P link
# really carries traffic.  A regtest node does not mine on its own: the
# `generate` RPC is the regtest block source.
sleep 8
say "mining three blocks on node-a"
mine "$NS_A" 3 >/dev/null
sleep 6
HEALTHY=$(observe healthy)
say "healthy: $HEALTHY"
printf '%s\n' "$HEALTHY" >"$ART/observe-healthy.json"

# ------------------------------------------------------------- partition the link
say "partitioning the node to node link"
"$LAB_DIR/link-fault.sh" apply --run-dir "$RUN_DIR" --link p2p-ab --policy partition \
    >"$ART/apply-partition.json" 2>&1 || fail "the partition could not be applied"
sleep 2
say "mining three more blocks on node-a while the link is partitioned"
mine "$NS_A" 3 >/dev/null
sleep 8
PARTITIONED=$(observe partitioned)
say "partitioned: $PARTITIONED"
printf '%s\n' "$PARTITIONED" >"$ART/observe-partitioned.json"

# The counters are read after the faulted traffic, so the dropped blocks are
# visible in the link counters.
COUNTERS_PARTITIONED=$("$LAB_DIR/link-fault.sh" counters --run-dir "$RUN_DIR" --link p2p-ab \
    | jq -c '.summary')
say "counters during the partition: $COUNTERS_PARTITIONED"
printf '%s\n' "$COUNTERS_PARTITIONED" >"$ART/counters-partition.json"

# The no-bypass proof must hold while the fault is active.
"$LAB_DIR/link-fault.sh" no-bypass-check --run-dir "$RUN_DIR" >"$ART/no-bypass.json" 2>&1
NO_BYPASS_OK=$(jq -r '.ok // false' "$ART/no-bypass.json")
NO_BYPASS_VIOLATIONS=$(jq -r '.violations | length' "$ART/no-bypass.json")
say "no-bypass during the partition: ok=$NO_BYPASS_OK violations=$NO_BYPASS_VIOLATIONS"

# --------------------------------------------------------------------- heal
say "healing the node to node link"
"$LAB_DIR/link-fault.sh" heal --run-dir "$RUN_DIR" --link p2p-ab >"$ART/heal.json" 2>&1 \
    || fail "the heal failed"
COUNTERS_HEALED=$("$LAB_DIR/link-fault.sh" counters --run-dir "$RUN_DIR" --link p2p-ab \
    | jq -c '.summary')
printf '%s\n' "$COUNTERS_HEALED" >"$ART/counters-healed.json"
sleep 20
HEALED=$(observe healed)
say "healed: $HEALED"
printf '%s\n' "$HEALED" >"$ART/observe-healed.json"

# ------------------------------------------------------------------ evidence
# The verdict: node-b must fall behind while the link is partitioned and
# catch up after the heal.  The observation is per node, not a tip sample.
A_HEALTHY=$(jq -r '.nodes["node-a"].height' <<<"$HEALTHY")
B_HEALTHY=$(jq -r '.nodes["node-b"].height' <<<"$HEALTHY")
A_PART=$(jq -r '.nodes["node-a"].height' <<<"$PARTITIONED")
B_PART=$(jq -r '.nodes["node-b"].height' <<<"$PARTITIONED")
A_HEALED=$(jq -r '.nodes["node-a"].height' <<<"$HEALED")
B_HEALED=$(jq -r '.nodes["node-b"].height' <<<"$HEALED")
SPLIT_OBSERVED=false
REJOIN_OBSERVED=false
[ "$A_PART" -gt "$B_PART" ] && SPLIT_OBSERVED=true
[ "$B_HEALED" -ge "$A_PART" ] && REJOIN_OBSERVED=true
if [ "$SPLIT_OBSERVED" = "true" ] && [ "$REJOIN_OBSERVED" = "true" ]; then
    VERDICT="passed"
else
    VERDICT="inconclusive"
fi
say "split observed=$SPLIT_OBSERVED rejoin observed=$REJOIN_OBSERVED verdict=$VERDICT"

jq -n \
    --arg node_binary "$NODE_BIN" --arg node_sha256 "$NODE_SHA" \
    --arg pow_bits "$POW_BITS" --arg verdict "$VERDICT" \
    --argjson split_observed "$SPLIT_OBSERVED" --argjson rejoin_observed "$REJOIN_OBSERVED" \
    --argjson healthy "$HEALTHY" --argjson partitioned "$PARTITIONED" --argjson healed "$HEALED" \
    --argjson counters_partitioned "$COUNTERS_PARTITIONED" --argjson counters_healed "$COUNTERS_HEALED" \
    --argjson no_bypass_ok "$NO_BYPASS_OK" --argjson no_bypass_violations "$NO_BYPASS_VIOLATIONS" \
    '{
      schema:"spc.lab.topology_two_nodes.v1",
      status:"provisional",
      verdict:$verdict,
      split_observed:$split_observed,
      rejoin_observed:$rejoin_observed,
      note:"the runtime candidate this borrows the node contract from is not approved; no pool, miner, payout or consensus claim is made here",
      binaries:{node:{path:$node_binary, sha256:$node_sha256}},
      pow_limit_bits:$pow_bits,
      observations:{healthy:$healthy, partitioned:$partitioned, healed:$healed},
      link_counters:{partitioned:$counters_partitioned, healed:$counters_healed},
      no_bypass:{ok:$no_bypass_ok, violations:$no_bypass_violations}
    }' >"$ART/two-nodes.json"

say "artifact: $ART/two-nodes.json"
jq -c '{status, observations, link_counters, no_bypass}' "$ART/two-nodes.json"
say "done"
