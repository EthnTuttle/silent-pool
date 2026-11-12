#!/usr/bin/env bash
# LP-4a isolation and link-fault test driver.
#
# This driver proves the network primitives the LP-4 link-fault backend needs:
#
#   * real TCP listeners exist in isolated namespaces and are reachable over the
#     link that connects them, over IPv4 and over IPv6 link-local;
#   * a fault on one owned link is SUSTAINED: repeated reconnect attempts keep
#     failing while the listener stays bound and the process stays alive;
#   * the fault covers IPv6 as well: the IPv6 link-local path of a
#     "partitioned" link is closed too;
#   * an already established bidirectional session is cut by the fault;
#   * reapplying the same fault is atomic: no window opens in which the link
#     carries traffic;
#   * a fault is link scoped: an unrelated link keeps working;
#   * healing restores delivery and the listener accepts a new session;
#   * there is NO direct bypass: from every namespace every other entity's
#     non-link address (IPv4 and IPv6) and every host address (IPv4 and IPv6)
#     is refused by the kernel, and forwarding is disabled everywhere;
#   * latency and loss can be injected with a qdisc and measured;
#   * the control plane stays responsive while a long command runs;
#   * a run directory has exactly one owner;
#   * teardown removes exactly the resources this run created.
#
# When the namespace backend is not available the driver runs the explicitly
# weaker relay fallback suite instead, and records that the partition and
# no-bypass assertions are `inconclusive` there.
#
# Usage: testing/lab-isolation/run.sh [--artifacts DIR] [--backend auto|netns|relay]
# Artifacts default to /tmp/opencode/LP4a-<utc>-<rand>.
# Exit code: 0 when every test passed, 1 otherwise, 2 when no backend applies.
set -uo pipefail

TEST_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(cd "$TEST_DIR/../.." && pwd)
LAB_DIR="$REPO_ROOT/scripts/lab"
FIXTURE="$TEST_DIR/tcp_fixture.py"
RELAY="$TEST_DIR/relay_fixture.py"

ART=""
FORCE_BACKEND="auto"
while [ $# -gt 0 ]; do
    case "$1" in
        --artifacts) ART="$2"; shift 2 ;;
        --artifacts=*) ART="${1#*=}"; shift ;;
        --backend) FORCE_BACKEND="$2"; shift 2 ;;
        --backend=*) FORCE_BACKEND="${1#*=}"; shift ;;
        *) printf 'unknown argument: %s\n' "$1" >&2; exit 2 ;;
    esac
done
if [ -z "$ART" ]; then
    ART="/tmp/opencode/LP4a-$(date -u +%Y%m%dT%H%M%SZ)-$(od -An -tx1 -N3 /dev/urandom | tr -d ' \n')"
fi
mkdir -p "$ART"
RUN_DIR="$ART/run"
RESULTS="$ART/results.jsonl"
RESULTS_TSV="$ART/results.tsv"
EXITS="$ART/exit-codes.txt"
: >"$RESULTS"
: >"$RESULTS_TSV"
: >"$EXITS"

PASS=0
FAIL=0
SKIP=0

record() {
    # record <name> <status:passed|failed|skipped|inconclusive> <detail>
    printf '%s\t%s\t%s\n' "$1" "$2" "$3" >>"$RESULTS_TSV"
    jq -nc --arg name "$1" --arg status "$2" --arg detail "$3" \
        '{name:$name, status:$status, detail:$detail}' >>"$RESULTS"
    case "$2" in
        passed) PASS=$((PASS + 1)) ;;
        failed) FAIL=$((FAIL + 1)) ;;
        *) SKIP=$((SKIP + 1)) ;;
    esac
    printf '[%-4s] %-34s %s\n' "$2" "$1" "$3"
}

exit_code() {
    printf '%s=%s\n' "$1" "$2" >>"$EXITS"
}

host_snapshot() {
    { ip -j addr show 2>/dev/null; echo; ip -j route show 2>/dev/null; } | sha256sum | awk '{print $1}'
}

# Hard host integrity check: the lab must not leave any trace on the host
# network.  It only looks for lab-owned state (the run token and the lab
# address space), because unrelated host state can change on its own.
host_lab_leak() {
    local token="$1"
    local ifs addrs routes
    ifs=$(ip -o link show 2>/dev/null | grep -c -- "$token" || true)
    addrs=$(ip -4 -o addr show 2>/dev/null | grep -c '10\.250\.' || true)
    routes=$(ip -4 -o route show 2>/dev/null | grep -c '10\.250\.' || true)
    printf '%s %s %s' "${ifs:-0}" "${addrs:-0}" "${routes:-0}"
}

wait_for_log() {
    # wait_for_log <file> <pattern> <timeout_s>
    local file="$1" pattern="$2" timeout_s="$3" waited=0
    while [ ! -f "$file" ] || ! grep -q "$pattern" "$file" 2>/dev/null; do
        sleep 0.1
        waited=$((waited + 1))
        [ "$waited" -ge $((timeout_s * 10)) ] && return 1
    done
    return 0
}

cleanup_all() {
    if [ -n "${RUN_DIR:-}" ] && [ -S "$RUN_DIR/control.sock" ]; then
        "$LAB_DIR/link-fault.sh" down --run-dir "$RUN_DIR" >"$ART/down.json" 2>&1 || true
    fi
    if [ -n "${RELAY_PID:-}" ]; then
        kill "$RELAY_PID" 2>/dev/null || true
    fi
    if [ -n "${HOST_SERVER_PID:-}" ]; then
        kill "$HOST_SERVER_PID" 2>/dev/null || true
    fi
}
trap cleanup_all EXIT

printf 'LP-4a isolation tests\n'
printf 'artifacts: %s\n' "$ART"
printf 'repository: %s\n\n' "$REPO_ROOT"

HOST_SNAPSHOT_BEFORE=$(host_snapshot)

# ------------------------------------------------------------------ T01 probe
PREFLIGHT_JSON="$ART/preflight.json"
"$LAB_DIR/netns-preflight.sh" --quiet --out "$PREFLIGHT_JSON" >/dev/null 2>&1
PREFLIGHT_RC=$?
exit_code preflight "$PREFLIGHT_RC"
PROBE_BACKEND=$(jq -r '.backend // "unsupported"' "$PREFLIGHT_JSON" 2>/dev/null)
BACKEND="$PROBE_BACKEND"
if [ "$FORCE_BACKEND" != "auto" ]; then
    # The backend can be forced so the fallback path is exercised on a host
    # that supports the namespace backend.  The preflight artifact keeps the
    # real capabilities; only the suite that runs changes.
    BACKEND="$FORCE_BACKEND"
fi
# The preflight must not contradict itself: a request may not be both
# guaranteed and unsupported.
OVERLAP=$(jq -c '. as $root | [$root.guarantees[] | select(. as $g | [$root.unsupported_requests[].request] | index($g))]' \
    "$PREFLIGHT_JSON" 2>/dev/null)
if [ "$PREFLIGHT_RC" = "0" ] || [ "$PREFLIGHT_RC" = "10" ]; then
    if [ "$OVERLAP" = "[]" ]; then
        record T01_preflight passed \
            "probe_backend=$PROBE_BACKEND exit=$PREFLIGHT_RC suite_backend=$BACKEND guarantees_and_unsupported_are_disjoint"
    else
        record T01_preflight failed "guarantees and unsupported_requests overlap: $OVERLAP"
    fi
else
    record T01_preflight failed \
        "probe_backend=$PROBE_BACKEND exit=$PREFLIGHT_RC suite_backend=$BACKEND"
fi
printf '\n'

# --------------------------------------------------------------------- suite
if [ "$BACKEND" = "netns" ]; then

    # ---------------------------------------------------------------- T02 up
    cat >"$ART/topology.json" <<'JSON'
{
  "schema": "spc.lab.link_topology.v1",
  "endpoints": [
    {"name": "alpha", "role": "pool"},
    {"name": "beta", "role": "front"},
    {"name": "gamma", "role": "miner"},
    {"name": "delta", "role": "ark"}
  ],
  "links": [
    {"id": "alpha-beta", "a": "alpha", "b": "beta"},
    {"id": "alpha-gamma", "a": "alpha", "b": "gamma"},
    {"id": "beta-delta", "a": "beta", "b": "delta"}
  ]
}
JSON
    "$LAB_DIR/link-fault.sh" up --spec "$ART/topology.json" --run-dir "$RUN_DIR" \
        >"$ART/up.json" 2>&1
    UP_RC=$?
    exit_code topology_up "$UP_RC"
    if [ "$UP_RC" != "0" ]; then
        record T02_topology failed "link-fault.sh up exit=$UP_RC"
        printf '\n'
        jq -n --slurpfile results "$RESULTS" --arg backend "$BACKEND" --arg art "$ART" \
            '{backend:$backend, artifacts:$art, results:$results}' >"$ART/summary.json"
        exit 1
    fi
    MANIFEST="$RUN_DIR/manifest.json"
    NS_COUNT=$(jq '.endpoints | length' "$MANIFEST")
    LINK_COUNT=$(jq '.links | length' "$MANIFEST")
    HOLDER_PIDS=$(jq -r '.endpoints[].namespace.holder_pid' "$MANIFEST" | tr '\n' ' ')
    record T02_topology passed "endpoints=$NS_COUNT links=$LINK_COUNT holders=$HOLDER_PIDS"

    ALPHA_BETA_A=$(jq -r '.links[]|select(.id=="alpha-beta")|.addresses.alpha' "$MANIFEST")
    ALPHA_BETA_B=$(jq -r '.links[]|select(.id=="alpha-beta")|.addresses.beta' "$MANIFEST")
    ALPHA_GAMMA_G=$(jq -r '.links[]|select(.id=="alpha-gamma")|.addresses.gamma' "$MANIFEST")
    BETA_DELTA_D=$(jq -r '.links[]|select(.id=="beta-delta")|.addresses.delta' "$MANIFEST")
    ALPHA_IFACE=$(jq -r '.links[]|select(.id=="alpha-beta")|.interfaces.alpha' "$MANIFEST")
    BETA_LL=$(jq -r '.endpoints[]|select(.name=="beta")|.ipv6_link_local[]|select(.link_id=="alpha-beta")|.addr' "$MANIFEST")
    BETA_PORT=9101
    GAMMA_PORT=9102
    DELTA_PORT=9103
    V6_PORT=9301
    PERSIST_PORT=9302

    # ------------------------------------------------------- T03 listeners up
    mkdir -p "$ART/fixtures"
    "$LAB_DIR/link-fault.sh" spawn --run-dir "$RUN_DIR" --endpoint beta --log fixtures/beta-spawn.log \
        --label beta-server -- python3 "$FIXTURE" server --bind "$ALPHA_BETA_B:$BETA_PORT" \
        --log "$ART/fixtures/beta-server.jsonl" --label beta >"$ART/spawn-beta.json" 2>&1
    SPAWN_BETA_RC=$?
    exit_code spawn_beta "$SPAWN_BETA_RC"
    "$LAB_DIR/link-fault.sh" spawn --run-dir "$RUN_DIR" --endpoint gamma --log fixtures/gamma-spawn.log \
        --label gamma-server -- python3 "$FIXTURE" server --bind "$ALPHA_GAMMA_G:$GAMMA_PORT" \
        --log "$ART/fixtures/gamma-server.jsonl" --label gamma >"$ART/spawn-gamma.json" 2>&1
    SPAWN_GAMMA_RC=$?
    exit_code spawn_gamma "$SPAWN_GAMMA_RC"
    "$LAB_DIR/link-fault.sh" spawn --run-dir "$RUN_DIR" --endpoint delta --log fixtures/delta-spawn.log \
        --label delta-server -- python3 "$FIXTURE" server --bind "$BETA_DELTA_D:$DELTA_PORT" \
        --log "$ART/fixtures/delta-server.jsonl" --label delta >"$ART/spawn-delta.json" 2>&1
    SPAWN_DELTA_RC=$?
    exit_code spawn_delta "$SPAWN_DELTA_RC"
    # An IPv6 listener on the same link: a dual stack daemon is the normal
    # case and its link-local address is a second path across the link.
    "$LAB_DIR/link-fault.sh" spawn --run-dir "$RUN_DIR" --endpoint beta --log fixtures/v6-spawn.log \
        --label beta-v6 -- python3 "$FIXTURE" server --bind "[::]:$V6_PORT" --family inet6 \
        --log "$ART/fixtures/beta-v6.jsonl" --label beta-v6 >"$ART/spawn-v6.json" 2>&1
    SPAWN_V6_RC=$?
    exit_code spawn_v6 "$SPAWN_V6_RC"

    LISTEN_OK=1
    wait_for_log "$ART/fixtures/beta-server.jsonl" '"event": "listen"' 10 || LISTEN_OK=0
    wait_for_log "$ART/fixtures/gamma-server.jsonl" '"event": "listen"' 10 || LISTEN_OK=0
    wait_for_log "$ART/fixtures/delta-server.jsonl" '"event": "listen"' 10 || LISTEN_OK=0
    wait_for_log "$ART/fixtures/beta-v6.jsonl" '"event": "listen"' 10 || LISTEN_OK=0
    if [ "$LISTEN_OK" = "1" ] && [ "$SPAWN_BETA_RC" = "0" ] && [ "$SPAWN_GAMMA_RC" = "0" ] \
        && [ "$SPAWN_DELTA_RC" = "0" ] && [ "$SPAWN_V6_RC" = "0" ]; then
        record T03_listeners passed \
            "beta=$ALPHA_BETA_B:$BETA_PORT gamma=$ALPHA_GAMMA_G:$GAMMA_PORT delta=$BETA_DELTA_D:$DELTA_PORT v6=[::]:$V6_PORT"
    else
        record T03_listeners failed "not all fixture listeners reported listen"
    fi
    BETA_PID=$(jq -r '.pid // empty' "$ART/spawn-beta.json" 2>/dev/null)
    BETA_TOKEN=$(jq -r '.start_time_token // empty' "$ART/spawn-beta.json" 2>/dev/null)

    # ------------------------------------------------------ T04 normal traffic
    "$LAB_DIR/link-fault.sh" exec --run-dir "$RUN_DIR" --endpoint alpha --timeout 30 -- \
        python3 "$FIXTURE" client --connect "$ALPHA_BETA_B:$BETA_PORT" --attempts 3 --timeout 2 \
        --interval 0.1 --expect PONG --label alpha-client --log "$ART/fixtures/normal.jsonl" \
        >"$ART/normal.json" 2>&1
    NORMAL_RC=$?
    exit_code normal_traffic "$NORMAL_RC"
    NORMAL_OK=$(jq -r '.stdout | fromjson | .succeeded // 0' "$ART/normal.json" 2>/dev/null)
    if [ "$NORMAL_RC" = "0" ] && [ "${NORMAL_OK:-0}" = "3" ]; then
        record T04_normal_traffic passed "3/3 sessions delivered payload over the alpha-beta link"
    else
        record T04_normal_traffic failed "succeeded=$NORMAL_OK rc=$NORMAL_RC"
    fi

    # -------------------------------------------- T05 sustained partition
    "$LAB_DIR/link-fault.sh" apply --run-dir "$RUN_DIR" --link alpha-beta --policy partition \
        >"$ART/apply-partition.json" 2>&1
    APPLY_RC=$?
    exit_code apply_partition "$APPLY_RC"
    "$LAB_DIR/link-fault.sh" exec --run-dir "$RUN_DIR" --endpoint beta -- \
        ss -ltn >"$ART/listener-during-fault.json" 2>&1
    LISTENER_BOUND=0
    grep -q "$ALPHA_BETA_B:$BETA_PORT" "$ART/listener-during-fault.json" && LISTENER_BOUND=1
    SERVER_ALIVE=0
    if [ -n "$BETA_PID" ] && [ -r "/proc/$BETA_PID/stat" ]; then
        CUR=$(awk '{n=split($0,a,") "); split(a[2],f," "); print f[20]}' "/proc/$BETA_PID/stat" 2>/dev/null)
        [ "$CUR" = "$BETA_TOKEN" ] && SERVER_ALIVE=1
    fi
    "$LAB_DIR/link-fault.sh" exec --run-dir "$RUN_DIR" --endpoint alpha --timeout 60 -- \
        python3 "$FIXTURE" client --connect "$ALPHA_BETA_B:$BETA_PORT" --attempts 8 --timeout 1 \
        --interval 0.2 --expect PONG --label alpha-reconnect --log "$ART/fixtures/partition.jsonl" \
        >"$ART/partition-attempts.json" 2>&1
    PART_RC=$?
    exit_code sustained_partition "$PART_RC"
    PART_OK=$(jq -r '.stdout | fromjson | .succeeded // -1' "$ART/partition-attempts.json" 2>/dev/null)
    PART_ATTEMPTS=$(jq -r '.stdout | fromjson | .attempts // 0' "$ART/partition-attempts.json" 2>/dev/null)
    "$LAB_DIR/link-fault.sh" counters --run-dir "$RUN_DIR" --link alpha-beta >"$ART/counters-partition.json" 2>&1
    DROPPED=$(jq -r '.summary.nft_dropped_packets // 0' "$ART/counters-partition.json" 2>/dev/null)
    COUNTERS_COMPLETE=$(jq -r '.summary.complete // false' "$ART/counters-partition.json" 2>/dev/null)
    # The fault must be sustained over time, not a one time disconnect.
    FIRST_TS=$(jq -r '.stdout | fromjson | .results[0].ts_unix_ms // 0' "$ART/partition-attempts.json" 2>/dev/null)
    LAST_TS=$(jq -r '.stdout | fromjson | .results[-1].ts_unix_ms // 0' "$ART/partition-attempts.json" 2>/dev/null)
    SPAN_MS=$(( ${LAST_TS:-0} - ${FIRST_TS:-0} ))
    if [ "$APPLY_RC" = "0" ] && [ "$PART_OK" = "0" ] && [ "${PART_ATTEMPTS:-0}" = "8" ] \
        && [ "$LISTENER_BOUND" = "1" ] && [ "$SERVER_ALIVE" = "1" ] && [ "${DROPPED:-0}" -gt 0 ] \
        && [ "${SPAN_MS:-0}" -ge 4000 ] && [ "$COUNTERS_COMPLETE" = "true" ]; then
        record T05_sustained_partition passed \
            "8/8 reconnects failed over ${SPAN_MS}ms, listener still bound on $ALPHA_BETA_B:$BETA_PORT, nft dropped=$DROPPED"
    else
        record T05_sustained_partition failed \
            "succeeded=$PART_OK attempts=$PART_ATTEMPTS span_ms=$SPAN_MS listener_bound=$LISTENER_BOUND server_alive=$SERVER_ALIVE dropped=$DROPPED counters_complete=$COUNTERS_COMPLETE"
    fi

    # --------------------------------- T05b the IPv6 path is partitioned too
    "$LAB_DIR/link-fault.sh" exec --run-dir "$RUN_DIR" --endpoint alpha --timeout 30 -- \
        python3 "$FIXTURE" probe --connect "$BETA_LL:$V6_PORT" --family inet6 --scope-iface "$ALPHA_IFACE" \
        --timeout 3 >"$ART/v6-during-fault.json" 2>&1
    V6_RC=$?
    V6_RESULT=$(jq -r '.stdout | fromjson | .result // "unknown"' "$ART/v6-during-fault.json" 2>/dev/null)
    if [ "$V6_RC" != "0" ] && [ "$V6_RESULT" = "timeout" ]; then
        record T05b_ipv6_partition_blocks_linklocal passed \
            "the IPv6 link-local dial to $BETA_LL timed out during the partition (result=$V6_RESULT)"
    else
        record T05b_ipv6_partition_blocks_linklocal failed \
            "the IPv6 link-local path was not blocked: rc=$V6_RC result=$V6_RESULT"
    fi

    # ------------------------- T05c an established session is cut as well
    # The session is established on a healthy link and the fault is applied
    # while the session is open.
    "$LAB_DIR/link-fault.sh" heal --run-dir "$RUN_DIR" --link alpha-beta >/dev/null 2>&1
    "$LAB_DIR/link-fault.sh" spawn --run-dir "$RUN_DIR" --endpoint beta --log fixtures/persist-spawn.log \
        --label persist-server -- python3 "$FIXTURE" server --bind "$ALPHA_BETA_B:$PERSIST_PORT" --persistent \
        --log "$ART/fixtures/persist-server.jsonl" --label persist >"$ART/spawn-persist.json" 2>&1
    wait_for_log "$ART/fixtures/persist-server.jsonl" '"event": "listen"' 10
    "$LAB_DIR/link-fault.sh" spawn --run-dir "$RUN_DIR" --endpoint alpha --log fixtures/session-spawn.log \
        --label established-session -- python3 "$FIXTURE" session --connect "$ALPHA_BETA_B:$PERSIST_PORT" \
        --hold 6 --timeout 3 --log "$ART/fixtures/session.jsonl" --label established \
        >"$ART/spawn-session.json" 2>&1
    SESSION_SPAWN_RC=$?
    exit_code spawn_session "$SESSION_SPAWN_RC"
    waited=0
    while [ "$waited" -lt 100 ]; do
        grep -q '"event": "phase1"' "$ART/fixtures/session.jsonl" 2>/dev/null && break
        sleep 0.1
        waited=$((waited + 1))
    done
    # The fault is applied while the session is open.
    "$LAB_DIR/link-fault.sh" apply --run-dir "$RUN_DIR" --link alpha-beta --policy partition \
        >"$ART/apply-established.json" 2>&1
    APPLY2_RC=$?
    exit_code apply_during_session "$APPLY2_RC"
    waited=0
    while [ "$waited" -lt 120 ]; do
        grep -q '"event": "phase2"' "$ART/fixtures/session.jsonl" 2>/dev/null && break
        sleep 0.1
        waited=$((waited + 1))
    done
    PHASE2=$(jq -r 'select(.event=="phase2")|.response' "$ART/fixtures/session.jsonl" 2>/dev/null | tail -1)
    PHASE1=$(jq -r 'select(.event=="phase1")|.response' "$ART/fixtures/session.jsonl" 2>/dev/null | tail -1)
    if [ -n "$PHASE1" ] && [ "${PHASE2:-}" != "" ] && [ "${PHASE2#FAIL:}" != "$PHASE2" ]; then
        record T05c_established_session_is_cut passed \
            "the session answered before the fault ($PHASE1) and failed after it ($PHASE2)"
    else
        record T05c_established_session_is_cut failed "phase1=$PHASE1 phase2=$PHASE2"
    fi
    "$LAB_DIR/link-fault.sh" heal --run-dir "$RUN_DIR" --link alpha-beta >/dev/null 2>&1

    # ------------------------------------------------------- T06 no bypass
    "$LAB_DIR/link-fault.sh" no-bypass-check --run-dir "$RUN_DIR" >"$ART/no-bypass.json" 2>&1
    NB_RC=$?
    exit_code no_bypass "$NB_RC"
    NB_OK=$(jq -r '.ok // false' "$ART/no-bypass.json" 2>/dev/null)
    NB_VIOL=$(jq -r '.violations | length' "$ART/no-bypass.json" 2>/dev/null)
    NB_CHECKS=$(jq -r '[.checks[] | select(.kind=="non_link_address")] | length' "$ART/no-bypass.json" 2>/dev/null)
    NB_HOST=$(jq -r '[.checks[] | select(.kind=="host_address" and .unreachable==true)] | length' "$ART/no-bypass.json" 2>/dev/null)
    NB_V6=$(jq -r '[.checks[] | select(.family=="inet6" and .kind=="non_link_address" and .unreachable==true)] | length' "$ART/no-bypass.json" 2>/dev/null)
    NB_FWD=$(jq -r '[.checks[] | select(.kind=="forwarding" and .forwarding.ipv4==0 and .forwarding.ipv6==0)] | length' "$ART/no-bypass.json" 2>/dev/null)
    if [ "$NB_RC" = "0" ] && [ "$NB_OK" = "true" ] && [ "${NB_VIOL:-1}" = "0" ] \
        && [ "${NB_CHECKS:-0}" -gt 0 ] && [ "${NB_V6:-0}" -gt 0 ] && [ "${NB_FWD:-0}" -eq 4 ]; then
        record T06_no_bypass passed \
            "$NB_CHECKS non-link probes ($NB_V6 IPv6) refused, $NB_HOST host probes refused, forwarding off in $NB_FWD namespaces, 0 violations"
    else
        record T06_no_bypass failed \
            "ok=$NB_OK violations=$NB_Viol checks=$NB_CHECKS v6=$NB_V6 host=$NB_HOST forwarding=$NB_FWD"
    fi

    # ------------------------------- T06b an unexpected IPv6 route is a violation
    ALPHA_IFACE=$(jq -r '.links[]|select(.id=="alpha-beta")|.interfaces.alpha' "$MANIFEST")
    "$LAB_DIR/link-fault.sh" exec --run-dir "$RUN_DIR" --endpoint alpha --timeout 30 -- \
        ip -6 route add 2001:db8::/64 dev "$ALPHA_IFACE" >"$ART/route-v6-add.json" 2>&1
    ROUTE_V6_RC=$?
    "$LAB_DIR/link-fault.sh" exec --run-dir "$RUN_DIR" --endpoint alpha --timeout 30 -- \
        ip route add 10.99.0.0/24 dev "$ALPHA_IFACE" >"$ART/route-v4-add.json" 2>&1
    ROUTE_V4_RC=$?
    "$LAB_DIR/link-fault.sh" no-bypass-check --run-dir "$RUN_DIR" >"$ART/no-bypass-unexpected-route.json" 2>&1
    UNEXPECTED_RC=$?
    V6_FOREIGN=$(jq -r '[.violations[] | select(.kind=="unattached_route" and .family=="6")] | length' \
        "$ART/no-bypass-unexpected-route.json" 2>/dev/null)
    V4_FOREIGN=$(jq -r '[.violations[] | select(.kind=="unattached_route" and .family=="4")] | length' \
        "$ART/no-bypass-unexpected-route.json" 2>/dev/null)
    exit_code unexpected_route "$UNEXPECTED_RC"
    if [ "$ROUTE_V6_RC" = "0" ] && [ "$ROUTE_V4_RC" = "0" ] && [ "${V6_FOREIGN:-0}" -ge 1 ] \
        && [ "${V4_FOREIGN:-0}" -ge 1 ]; then
        record T06b_unexpected_ipv6_route_is_a_violation passed \
            "an injected 2001:db8::/64 route and an injected 10.99.0.0/24 route are both reported as unattached_route (v6=$V6_FOREIGN v4=$V4_FOREIGN)"
    else
        record T06b_unexpected_ipv6_route_is_a_violation failed \
            "route_add_rc=$ROUTE_V6_RC/$ROUTE_V4_RC v6_violations=$V6_FOREIGN v4_violations=$V4_FOREIGN"
    fi
    # The owned namespace must be restored to its normal routes.
    "$LAB_DIR/link-fault.sh" exec --run-dir "$RUN_DIR" --endpoint alpha --timeout 30 -- \
        ip -6 route del 2001:db8::/64 dev "$ALPHA_IFACE" >/dev/null 2>&1
    "$LAB_DIR/link-fault.sh" exec --run-dir "$RUN_DIR" --endpoint alpha --timeout 30 -- \
        ip route del 10.99.0.0/24 dev "$ALPHA_IFACE" >/dev/null 2>&1
    "$LAB_DIR/link-fault.sh" no-bypass-check --run-dir "$RUN_DIR" >"$ART/no-bypass-restored.json" 2>&1
    RESTORED_VIOL=$(jq -r '.violations | length' "$ART/no-bypass-restored.json" 2>/dev/null)
    if [ "${RESTORED_VIOL:-1}" = "0" ]; then
        record T06c_owned_routes_are_restored passed "the injected routes are gone and the check reports 0 violations again"
    else
        record T06c_owned_routes_are_restored failed "violations after restore=$RESTORED_VIOL"
    fi

    # ------------------------------------------- T07 unrelated link survives
    "$LAB_DIR/link-fault.sh" apply --run-dir "$RUN_DIR" --link alpha-beta --policy partition >/dev/null 2>&1
    "$LAB_DIR/link-fault.sh" exec --run-dir "$RUN_DIR" --endpoint alpha --timeout 30 -- \
        python3 "$FIXTURE" client --connect "$ALPHA_GAMMA_G:$GAMMA_PORT" --attempts 2 --timeout 2 \
        --interval 0.1 --expect PONG --label alpha-client --log "$ART/fixtures/unrelated.jsonl" \
        >"$ART/unrelated.json" 2>&1
    UNREL_RC=$?
    exit_code unrelated_link "$UNREL_RC"
    UNREL_OK=$(jq -r '.stdout | fromjson | .succeeded // 0' "$ART/unrelated.json" 2>/dev/null)
    if [ "$UNREL_RC" = "0" ] && [ "${UNREL_OK:-0}" = "2" ]; then
        record T07_unrelated_link_survives passed \
            "alpha-gamma still delivered 2/2 sessions while alpha-beta was partitioned"
    else
        record T07_unrelated_link_survives failed "succeeded=$UNREL_OK rc=$UNREL_RC"
    fi

    # ------------------------------------------------------------ T08 heal
    ACCEPTS_BEFORE=$(grep -c '"event": "accept"' "$ART/fixtures/beta-server.jsonl" 2>/dev/null || echo 0)
    "$LAB_DIR/link-fault.sh" heal --run-dir "$RUN_DIR" --link alpha-beta >"$ART/heal.json" 2>&1
    HEAL_RC=$?
    exit_code heal "$HEAL_RC"
    "$LAB_DIR/link-fault.sh" exec --run-dir "$RUN_DIR" --endpoint alpha --timeout 30 -- \
        python3 "$FIXTURE" client --connect "$ALPHA_BETA_B:$BETA_PORT" --attempts 2 --timeout 2 \
        --interval 0.1 --expect PONG --label alpha-client --log "$ART/fixtures/healed.jsonl" \
        >"$ART/healed.json" 2>&1
    HEALED_RC=$?
    exit_code heal_delivery "$HEALED_RC"
    HEALED_OK=$(jq -r '.stdout | fromjson | .succeeded // 0' "$ART/healed.json" 2>/dev/null)
    ACCEPTS_AFTER=$(grep -c '"event": "accept"' "$ART/fixtures/beta-server.jsonl" 2>/dev/null || echo 0)
    # The IPv6 path must come back too.
    "$LAB_DIR/link-fault.sh" exec --run-dir "$RUN_DIR" --endpoint alpha --timeout 30 -- \
        python3 "$FIXTURE" probe --connect "$BETA_LL:$V6_PORT" --family inet6 --scope-iface "$ALPHA_IFACE" \
        --timeout 3 >"$ART/v6-after-heal.json" 2>&1
    V6_HEAL_RC=$?
    if [ "$HEAL_RC" = "0" ] && [ "$HEALED_RC" = "0" ] && [ "${HEALED_OK:-0}" = "2" ] \
        && [ "$V6_HEAL_RC" = "0" ] && [ "${ACCEPTS_AFTER:-0}" -gt "${ACCEPTS_BEFORE:-0}" ]; then
        record T08_heal_restores_delivery passed \
            "2/2 IPv4 sessions and the IPv6 link-local session both delivered after the heal, listener accepted $((ACCEPTS_AFTER - ACCEPTS_BEFORE)) new sessions"
    else
        record T08_heal_restores_delivery failed \
            "succeeded=$HEALED_OK rc=$HEALED_RC v6_rc=$V6_HEAL_RC accepts_before=$ACCEPTS_BEFORE accepts_after=$ACCEPTS_AFTER"
    fi

    # ------------------------------- T06d a faulted link is not a bypass violation
    "$LAB_DIR/link-fault.sh" apply --run-dir "$RUN_DIR" --link alpha-beta --policy partition >/dev/null 2>&1
    "$LAB_DIR/link-fault.sh" no-bypass-check --run-dir "$RUN_DIR" >"$ART/no-bypass-during-fault.json" 2>&1
    FAULTED_VIOL=$(jq -r '[.violations[] | select(.kind=="link_address_unreachable")] | length' \
        "$ART/no-bypass-during-fault.json" 2>/dev/null)
    FAULTED_LIST=$(jq -c '.faulted_links' "$ART/no-bypass-during-fault.json" 2>/dev/null)
    "$LAB_DIR/link-fault.sh" heal --run-dir "$RUN_DIR" --link alpha-beta >/dev/null 2>&1
    if [ "${FAULTED_VIOL:-1}" = "0" ] && [ "$FAULTED_LIST" != "[]" ]; then
        record T06d_faulted_link_is_not_a_violation passed \
            "the no-bypass check during an intentional fault reports $FAULTED_LIST as faulted and raises no link_address_unreachable violation"
    else
        record T06d_faulted_link_is_not_a_violation failed "violations=$FAULTED_VIOL faulted_links=$FAULTED_LIST"
    fi

    # ---------------------------------------------------------- T09 latency
    LAT_AVAIL=$(jq -r '.netem_supported // false' "$PREFLIGHT_JSON" 2>/dev/null)
    if [ "$LAT_AVAIL" = "true" ]; then
        "$LAB_DIR/link-fault.sh" apply --run-dir "$RUN_DIR" --link alpha-beta --policy latency:150 \
            >"$ART/apply-latency.json" 2>&1
        "$LAB_DIR/link-fault.sh" exec --run-dir "$RUN_DIR" --endpoint alpha --timeout 40 -- \
            python3 "$FIXTURE" client --connect "$ALPHA_BETA_B:$BETA_PORT" --attempts 1 --timeout 10 \
            --expect PONG --label alpha-client --log "$ART/fixtures/latency.jsonl" \
            >"$ART/latency.json" 2>&1
        LAT_RC=$?
        exit_code latency "$LAT_RC"
        LAT_MS=$(jq -r '.stdout | fromjson | .results[0].elapsed_ms // 0' "$ART/latency.json" 2>/dev/null)
        LAT_SUCCEEDED=$(jq -r '.stdout | fromjson | .succeeded // 0' "$ART/latency.json" 2>/dev/null)
        "$LAB_DIR/link-fault.sh" heal --run-dir "$RUN_DIR" --link alpha-beta >/dev/null 2>&1
        "$LAB_DIR/link-fault.sh" exec --run-dir "$RUN_DIR" --endpoint alpha --timeout 30 -- \
            python3 "$FIXTURE" client --connect "$ALPHA_BETA_B:$BETA_PORT" --attempts 1 --timeout 5 \
            --expect PONG --label alpha-client --log "$ART/fixtures/latency-healed.jsonl" \
            >"$ART/latency-healed.json" 2>&1
        BASE_MS=$(jq -r '.stdout | fromjson | .results[0].elapsed_ms // 0' "$ART/latency-healed.json" 2>/dev/null)
        BASE_SUCCEEDED=$(jq -r '.stdout | fromjson | .succeeded // 0' "$ART/latency-healed.json" 2>/dev/null)
        if [ "$LAT_RC" = "0" ] && [ "${LAT_SUCCEEDED:-0}" = "1" ] && [ "${BASE_SUCCEEDED:-0}" = "1" ] \
            && [ "${LAT_MS:-0}" -ge 250 ] && [ "${BASE_MS:-99999}" -lt 150 ]; then
            record T09_latency_qdisc passed \
                "netem delay 150ms: session delivered in ${LAT_MS}ms, healed baseline ${BASE_MS}ms"
        else
            record T09_latency_qdisc failed \
                "delayed=${LAT_MS}ms (succeeded=${LAT_SUCCEEDED:-0}) baseline=${BASE_MS}ms (succeeded=${BASE_SUCCEEDED:-0}) rc=$LAT_RC"
        fi
    else
        record T09_latency_qdisc inconclusive "sch_netem is unavailable on this host"
        exit_code latency 20
    fi

    # ------------------------------------------------------------- T10 loss
    if [ "$LAT_AVAIL" = "true" ]; then
        "$LAB_DIR/link-fault.sh" apply --run-dir "$RUN_DIR" --link alpha-beta --policy loss:30 \
            >"$ART/apply-loss.json" 2>&1
        "$LAB_DIR/link-fault.sh" exec --run-dir "$RUN_DIR" --endpoint alpha --timeout 60 -- \
            python3 "$FIXTURE" client --connect "$ALPHA_BETA_B:$BETA_PORT" --attempts 10 --timeout 1 \
            --interval 0.1 --expect PONG --label alpha-client --log "$ART/fixtures/loss.jsonl" \
            >"$ART/loss.json" 2>&1
        LOSS_RC=$?
        exit_code loss "$LOSS_RC"
        LOSS_FAIL=$(jq -r '.stdout | fromjson | .failed // -1' "$ART/loss.json" 2>/dev/null)
        "$LAB_DIR/link-fault.sh" counters --run-dir "$RUN_DIR" --link alpha-beta >"$ART/counters-loss.json" 2>&1
        QDROPS=$(jq -r '.summary.qdisc_drops // 0' "$ART/counters-loss.json" 2>/dev/null)
        LOSS_COMPLETE=$(jq -r '.summary.complete // false' "$ART/counters-loss.json" 2>/dev/null)
        "$LAB_DIR/link-fault.sh" heal --run-dir "$RUN_DIR" --link alpha-beta >/dev/null 2>&1
        "$LAB_DIR/link-fault.sh" exec --run-dir "$RUN_DIR" --endpoint alpha --timeout 30 -- \
            python3 "$FIXTURE" client --connect "$ALPHA_BETA_B:$BETA_PORT" --attempts 2 --timeout 2 \
            --interval 0.1 --expect PONG --label alpha-client --log "$ART/fixtures/loss-healed.jsonl" \
            >"$ART/loss-healed.json" 2>&1
        LOSS_HEAL_OK=$(jq -r '.stdout | fromjson | .succeeded // 0' "$ART/loss-healed.json" 2>/dev/null)
        if [ "$LOSS_RC" = "0" ] && [ "${QDROPS:-0}" -gt 0 ] && [ "${LOSS_HEAL_OK:-0}" = "2" ] \
            && [ "$LOSS_COMPLETE" = "true" ]; then
            record T10_loss_qdisc passed \
                "netem loss 30%: qdisc dropped $QDROPS packets, $LOSS_FAIL/10 sessions failed, heal restored 2/2"
        else
            record T10_loss_qdisc failed \
                "qdisc_drops=$QDROPS failed=$LOSS_FAIL healed=$LOSS_HEAL_OK counters_complete=$LOSS_COMPLETE"
        fi
    else
        record T10_loss_qdisc inconclusive "sch_netem is unavailable on this host"
        exit_code loss 20
    fi

    # --------------------------- T11 an idempotent reapply has no open window
    "$LAB_DIR/link-fault.sh" apply --run-dir "$RUN_DIR" --link alpha-beta --policy partition >/dev/null 2>&1
    "$LAB_DIR/link-fault.sh" exec --run-dir "$RUN_DIR" --endpoint alpha --timeout 120 -- \
        python3 "$FIXTURE" client --connect "$ALPHA_BETA_B:$BETA_PORT" --attempts 150 --timeout 0.3 \
        --interval 0 --expect PONG --label reapply-hammer --log "$ART/fixtures/reapply.jsonl" \
        >"$ART/reapply-hammer.json" 2>&1 &
    HAMMER=$!
    sleep 1
    REAPPLY_FAILED=0
    for _ in $(seq 1 10); do
        "$LAB_DIR/link-fault.sh" apply --run-dir "$RUN_DIR" --link alpha-beta --policy partition \
            >/dev/null 2>&1 || REAPPLY_FAILED=1
    done
    wait $HAMMER
    exit_code atomic_reapply 0
    HAMMER_OK=$(jq -r '.stdout | fromjson | .succeeded // -1' "$ART/reapply-hammer.json" 2>/dev/null)
    HAMMER_ATTEMPTS=$(jq -r '.stdout | fromjson | .attempts // 0' "$ART/reapply-hammer.json" 2>/dev/null)
    if [ "$REAPPLY_FAILED" = "0" ] && [ "${HAMMER_OK:-1}" = "0" ] && [ "${HAMMER_ATTEMPTS:-0}" -ge 100 ]; then
        record T11_atomic_reapply_has_no_open_window passed \
            "$HAMMER_ATTEMPTS reconnects during 10 reapplies, 0 succeeded"
    else
        record T11_atomic_reapply_has_no_open_window failed \
            "reapply_failed=$REAPPLY_FAILED succeeded=$HAMMER_OK attempts=$HAMMER_ATTEMPTS"
    fi
    "$LAB_DIR/link-fault.sh" heal --run-dir "$RUN_DIR" --link alpha-beta >/dev/null 2>&1

    # ------------------------------------------- T12 a run directory has one owner
    "$LAB_DIR/link-fault.sh" up --spec "$ART/topology.json" --run-dir "$RUN_DIR" >"$ART/up-duplicate.json" 2>&1
    DUP_RC=$?
    exit_code duplicate_up "$DUP_RC"
    DUP_CODE=$(jq -r '.error.code // ""' "$ART/up-duplicate.json" 2>/dev/null)
    "$LAB_DIR/link-fault.sh" status --run-dir "$RUN_DIR" >"$ART/status-after-duplicate.json" 2>&1
    STATUS_RC=$?
    exit_code status_after_duplicate "$STATUS_RC"
    if [ "$DUP_RC" = "1" ] && [ "$DUP_CODE" = "run_dir_in_use" ] && [ "$STATUS_RC" = "0" ]; then
        record T12_duplicate_up_is_refused passed \
            "the second up was refused ($DUP_CODE) and the original run still answers status"
    else
        record T12_duplicate_up_is_refused failed \
            "duplicate_up_rc=$DUP_RC code=$DUP_CODE status_rc=$STATUS_RC"
    fi

    # ------------------------- T12b a stale run directory never satisfies a startup
    STALE_DIR="$ART/stale-run"
    "$LAB_DIR/link-fault.sh" up --spec "$ART/topology.json" --run-dir "$STALE_DIR" \
        >"$ART/stale-up.json" 2>&1
    STALE_UP_RC=$?
    STALE_PID=$(jq -r '.supervisor.pid // empty' "$ART/stale-up.json" 2>/dev/null)
    if [ "$STALE_UP_RC" = "0" ] && [ -n "$STALE_PID" ]; then
        kill -9 "$STALE_PID" 2>/dev/null || true
        sleep 1
    fi
    "$LAB_DIR/link-fault.sh" up --spec "$ART/topology.json" --run-dir "$STALE_DIR" \
        >"$ART/stale-reuse.json" 2>&1
    STALE_REUSE_RC=$?
    STALE_CODE=$(jq -r '.error.code // ""' "$ART/stale-reuse.json" 2>/dev/null)
    STALE_OK=$(jq -r '.ok // false' "$ART/stale-reuse.json" 2>/dev/null)
    STALE_FRESH=$(jq -r '.error.fresh_dir_required // false' "$ART/stale-reuse.json" 2>/dev/null)
    # The diagnostics of the dead run must still be there.
    STALE_MANIFEST_KEPT=0
    [ -f "$STALE_DIR/manifest.json" ] && STALE_MANIFEST_KEPT=1
    # A fresh directory must work.
    FRESH_DIR="$ART/fresh-run"
    "$LAB_DIR/link-fault.sh" up --spec "$ART/topology.json" --run-dir "$FRESH_DIR" \
        >"$ART/fresh-up.json" 2>&1
    FRESH_RC=$?
    "$LAB_DIR/link-fault.sh" status --run-dir "$FRESH_DIR" >"$ART/fresh-status.json" 2>&1
    FRESH_STATUS_RC=$?
    FRESH_PID=$(jq -r '.supervisor.pid // empty' "$ART/fresh-up.json" 2>/dev/null)
    FRESH_TOKEN=$(jq -r '.run_token // empty' "$ART/fresh-up.json" 2>/dev/null)
    STALE_TOKEN=$(jq -r '.run_token // empty' "$STALE_DIR/manifest.json" 2>/dev/null)
    "$LAB_DIR/link-fault.sh" down --run-dir "$FRESH_DIR" >/dev/null 2>&1
    exit_code stale_run_dir "$STALE_REUSE_RC"
    exit_code fresh_run_dir "$FRESH_RC"
    if [ "$STALE_REUSE_RC" = "1" ] && [ "$STALE_CODE" = "stale_run_dir" ] && [ "$STALE_OK" = "false" ] \
        && [ "$STALE_FRESH" = "true" ] && [ "$STALE_MANIFEST_KEPT" = "1" ] \
        && [ "$FRESH_RC" = "0" ] && [ "$FRESH_STATUS_RC" = "0" ] && [ -n "$FRESH_PID" ] \
        && [ "$FRESH_TOKEN" != "$STALE_TOKEN" ]; then
        record T12b_stale_run_dir_never_satisfies_a_startup passed \
            "the stale directory was refused (stale_run_dir, fresh_dir_required, nothing deleted) and a fresh directory started a new backend with a new token"
    else
        record T12b_stale_run_dir_never_satisfies_a_startup failed \
            "reuse_rc=$STALE_REUSE_RC code=$STALE_CODE ok=$STALE_OK fresh_required=$STALE_FRESH manifest_kept=$STALE_MANIFEST_KEPT fresh_rc=$FRESH_RC fresh_status=$FRESH_STATUS_RC"
    fi

    # --------------------------- T13 the control plane is not blocked by an exec
    "$LAB_DIR/link-fault.sh" exec --run-dir "$RUN_DIR" --endpoint alpha --timeout 30 -- sleep 5 \
        >/dev/null 2>&1 &
    SLEEPER=$!
    sleep 0.3
    APPLY_START=$(date +%s%3N)
    "$LAB_DIR/link-fault.sh" apply --run-dir "$RUN_DIR" --link alpha-beta --policy partition \
        >"$ART/apply-concurrent.json" 2>&1
    APPLY_RC=$?
    APPLY_MS=$(( $(date +%s%3N) - APPLY_START ))
    wait $SLEEPER
    exit_code concurrent_apply "$APPLY_RC"
    if [ "$APPLY_RC" = "0" ] && [ "$APPLY_MS" -lt 1000 ]; then
        record T13_control_plane_is_not_blocked passed \
            "apply completed in ${APPLY_MS}ms while an exec slept for 5s"
    else
        record T13_control_plane_is_not_blocked failed "apply took ${APPLY_MS}ms rc=$APPLY_RC"
    fi
    "$LAB_DIR/link-fault.sh" heal --run-dir "$RUN_DIR" --link alpha-beta >/dev/null 2>&1

    # --------------------------- T13b long execs do not starve the control path
    for _ in $(seq 1 8); do
        "$LAB_DIR/link-fault.sh" exec --run-dir "$RUN_DIR" --endpoint alpha --timeout 30 -- sleep 4 \
            >/dev/null 2>&1 &
    done
    sleep 0.5
    STATUS_START=$(date +%s%3N)
    "$LAB_DIR/link-fault.sh" status --run-dir "$RUN_DIR" >"$ART/status-under-load.json" 2>&1
    STATUS_LOAD_RC=$?
    STATUS_LOAD_MS=$(( $(date +%s%3N) - STATUS_START ))
    wait
    exit_code status_under_load "$STATUS_LOAD_RC"
    if [ "$STATUS_LOAD_RC" = "0" ] && [ "$STATUS_LOAD_MS" -lt 2000 ]; then
        record T13b_long_execs_do_not_starve_the_control_path passed \
            "status answered in ${STATUS_LOAD_MS}ms while 8 long execs were running"
    else
        record T13b_long_execs_do_not_starve_the_control_path failed \
            "status rc=$STATUS_LOAD_RC elapsed=${STATUS_LOAD_MS}ms"
    fi

    # --------------------------------------- T14 a spawn that cannot start fails
    "$LAB_DIR/link-fault.sh" spawn --run-dir "$RUN_DIR" --endpoint beta --log fixtures/bad-spawn.log \
        --label bad -- /nonexistent/lab4a-binary >"$ART/spawn-missing.json" 2>&1
    BAD_SPAWN_RC=$?
    exit_code spawn_missing_binary "$BAD_SPAWN_RC"
    BAD_CODE=$(jq -r '.error.code // ""' "$ART/spawn-missing.json" 2>/dev/null)
    BAD_OK=$(jq -r '.ok // false' "$ART/spawn-missing.json" 2>/dev/null)
    if [ "$BAD_SPAWN_RC" != "0" ] && [ "$BAD_OK" = "false" ] && [ "$BAD_CODE" = "spawn_failed" ]; then
        record T14_spawn_failure_is_honest passed \
            "spawning a missing binary returned $BAD_CODE instead of a false ok"
    else
        record T14_spawn_failure_is_honest failed "rc=$BAD_SPAWN_RC ok=$BAD_OK code=$BAD_CODE"
    fi

    # ------------------------------------- T15 the counters are complete
    "$LAB_DIR/link-fault.sh" counters --run-dir "$RUN_DIR" --link alpha-beta >"$ART/counters-complete.json" 2>&1
    COUNTERS_RC=$?
    COMPLETE=$(jq -r '.summary.complete // false' "$ART/counters-complete.json" 2>/dev/null)
    COUNTER_ERRORS=$(jq -r '.summary.errors | length' "$ART/counters-complete.json" 2>/dev/null)
    if [ "$COUNTERS_RC" = "0" ] && [ "$COMPLETE" = "true" ] && [ "${COUNTER_ERRORS:-1}" = "0" ]; then
        record T15_counters_are_complete passed "the counter summary is complete with no tool errors"
    else
        record T15_counters_are_complete failed "complete=$COMPLETE errors=$COUNTER_ERRORS rc=$COUNTERS_RC"
    fi

    # ------------------------------------- T15b a truncated output is reported
    "$LAB_DIR/link-fault.sh" exec --run-dir "$RUN_DIR" --endpoint alpha --timeout 30 -- \
        python3 -c 'print("x"*2000000)' >"$ART/truncated.json" 2>&1
    TRUNC_RC=$?
    TRUNC_FLAG=$(jq -r '.stdout_truncated // false' "$ART/truncated.json" 2>/dev/null)
    TRUNC_LEN=$(jq -r '.stdout | length' "$ART/truncated.json" 2>/dev/null)
    if [ "$TRUNC_RC" = "0" ] && [ "$TRUNC_FLAG" = "true" ] && [ "${TRUNC_LEN:-0}" -le 262144 ]; then
        record T15b_truncated_output_is_reported passed \
            "a 2 MB output is capped at ${TRUNC_LEN} bytes and stdout_truncated is true"
    else
        record T15b_truncated_output_is_reported failed "rc=$TRUNC_RC flag=$TRUNC_FLAG len=$TRUNC_LEN"
    fi

    # ------------------------------------- T16 the lifecycle budget is published
    LIMITS=$(jq -c '.limits' "$MANIFEST" 2>/dev/null)
    IDLE=$(jq -r '.limits.idle_timeout_s // "missing"' "$MANIFEST" 2>/dev/null)
    LIFETIME=$(jq -r '.limits.max_lifetime_s // "missing"' "$MANIFEST" 2>/dev/null)
    DEADLINE=$(jq -r '.limits.lifetime_deadline_unix_ms // "missing"' "$MANIFEST" 2>/dev/null)
    KILL=$(jq -r '.limits.kill_on_expiry // "missing"' "$MANIFEST" 2>/dev/null)
    if [ "$IDLE" != "missing" ] && [ "$LIFETIME" != "missing" ] && [ "$DEADLINE" != "missing" ] \
        && [ "$KILL" = "true" ]; then
        record T16_lifecycle_budget_is_published passed \
            "idle_timeout_s=$IDLE max_lifetime_s=$LIFETIME lifetime_deadline=$DEADLINE kill_on_expiry=$KILL"
    else
        record T16_lifecycle_budget_is_published failed "limits=$LIMITS"
    fi

    # ------------------------------------- T17 argv survives newlines
    "$LAB_DIR/link-fault.sh" exec --run-dir "$RUN_DIR" --endpoint alpha --timeout 30 -- \
        python3 -c 'import json,sys; print(json.dumps(sys.argv[1:]))' "$(printf 'a\nb')" c \
        >"$ART/argv.json" 2>&1
    ARGV_RC=$?
    ARGV_OUT=$(jq -r '.stdout | fromjson | .[0] // ""' "$ART/argv.json" 2>/dev/null)
    ARGV_LEN=$(jq -r '.stdout | fromjson | length // 0' "$ART/argv.json" 2>/dev/null)
    if [ "$ARGV_RC" = "0" ] && [ "$ARGV_LEN" = "2" ] && [ "$ARGV_OUT" = "$(printf 'a\nb')" ]; then
        record T17_argv_survives_newlines passed "an argument with a newline reached the namespace intact"
    else
        record T17_argv_survives_newlines failed "len=$ARGV_LEN value=$(printf '%q' "$ARGV_OUT") rc=$ARGV_RC"
    fi

    # ------------------------------------- T18 the command timeout is bounded
    "$LAB_DIR/link-fault.sh" exec --run-dir "$RUN_DIR" --endpoint alpha --timeout 0 -- true \
        >"$ART/timeout-zero.json" 2>&1
    T0_RC=$?
    "$LAB_DIR/link-fault.sh" exec --run-dir "$RUN_DIR" --endpoint alpha --timeout 100000 -- true \
        >"$ART/timeout-huge.json" 2>&1
    TH_RC=$?
    T0_CODE=$(jq -r '.error.code // ""' "$ART/timeout-zero.json" 2>/dev/null)
    TH_CODE=$(jq -r '.error.code // ""' "$ART/timeout-huge.json" 2>/dev/null)
    exit_code timeout_bounds "$T0_RC"
    if [ "$T0_RC" != "0" ] && [ "$TH_RC" != "0" ] && [ "$T0_CODE" = "invalid_request" ] \
        && [ "$TH_CODE" = "invalid_request" ]; then
        record T18_command_timeout_is_bounded passed "timeout_s 0 and 100000 are both refused"
    else
        record T18_command_timeout_is_bounded failed "zero=$T0_CODE/$T0_RC huge=$TH_CODE/$TH_RC"
    fi

    # ----------------------------------------------------------- T19 cleanup
    HOLDER_PIDS=$(jq -r '.endpoints[].namespace.holder_pid' "$MANIFEST")
    RUN_TOKEN=$(jq -r '.run_token' "$MANIFEST")
    CHILD_PIDS=$(jq -r '.children[]?.pid' "$MANIFEST" 2>/dev/null)
    "$LAB_DIR/link-fault.sh" down --run-dir "$RUN_DIR" >"$ART/down.json" 2>&1
    DOWN_RC=$?
    exit_code teardown "$DOWN_RC"
    sleep 1
    LEFTOVER=0
    LEFTOVER_PIDS=""
    while read -r pid; do
        [ -z "$pid" ] && continue
        if [ -r "/proc/$pid/stat" ]; then
            STATE=$(awk '{n=split($0,a,") "); split(a[2],f," "); print f[1]}' "/proc/$pid/stat" 2>/dev/null)
            if [ "$STATE" != "Z" ]; then
                LEFTOVER=1
                LEFTOVER_PIDS="$LEFTOVER_PIDS $pid"
            fi
        fi
    done <<<"$HOLDER_PIDS"
    LEFTOVER_CHILDREN=0
    while read -r pid; do
        [ -z "$pid" ] && continue
        if [ -r "/proc/$pid/stat" ]; then
            STATE=$(awk '{n=split($0,a,") "); split(a[2],f," "); print f[1]}' "/proc/$pid/stat" 2>/dev/null)
            [ "$STATE" != "Z" ] && LEFTOVER_CHILDREN=1
        fi
    done <<<"$CHILD_PIDS"
    pgrep -fa "$RUN_TOKEN" >"$ART/token-processes.txt" 2>/dev/null || true
    TOKEN_PROCS=$(wc -l <"$ART/token-processes.txt")
    read -r HOST_IF HOST_ADDRS HOST_ROUTES <<<"$(host_lab_leak "$RUN_TOKEN")"
    HOST_SNAPSHOT_AFTER=$(host_snapshot)
    if [ "$DOWN_RC" = "0" ] && [ "$LEFTOVER" = "0" ] && [ "$LEFTOVER_CHILDREN" = "0" ] \
        && [ "${TOKEN_PROCS:-1}" = "0" ] && [ "${HOST_IF:-1}" = "0" ] \
        && [ "${HOST_ADDRS:-1}" = "0" ] && [ "${HOST_ROUTES:-1}" = "0" ]; then
        record T19_teardown passed \
            "namespace holders and fixtures gone, no process carries the run token, no lab interface, address or route on the host"
    else
        record T19_teardown failed \
            "down_rc=$DOWN_RC leftover_pids=$LEFTOVER_PIDS leftover_children=$LEFTOVER_CHILDREN token_procs=$TOKEN_PROCS host_links=$HOST_IF host_addrs=$HOST_ADDRS host_routes=$HOST_ROUTES"
    fi

    FINAL_BACKEND="netns"
elif [ "$BACKEND" = "relay" ]; then
    # ------------------------------------------------- relay fallback suite
    record T02_netns_suite skipped "the namespace backend is unavailable on this host"
    printf '\n'
    printf 'Running the explicitly weaker relay fallback suite.\n'
    printf 'Partition and no-bypass assertions are inconclusive in this backend.\n\n'
    HOST_SERVER_PID=""
    python3 "$FIXTURE" server --bind 127.0.0.1:9201 --log "$ART/fixtures/relay-server.jsonl" \
        --label relay-target >"$ART/relay-server.log" 2>&1 &
    HOST_SERVER_PID=$!
    wait_for_log "$ART/fixtures/relay-server.jsonl" '"event": "listen"' 10
    printf 'up\n' >"$ART/relay-state"
    python3 "$RELAY" relay --listen 127.0.0.1:9202 --target 127.0.0.1:9201 \
        --state-file "$ART/relay-state" --log "$ART/fixtures/relay.jsonl" --label relay \
        >"$ART/relay.log" 2>&1 &
    RELAY_PID=$!
    sleep 1

    python3 "$FIXTURE" client --connect 127.0.0.1:9202 --attempts 2 --timeout 2 --interval 0.1 \
        --expect PONG --label relay-client --log "$ART/fixtures/relay-up.jsonl" >"$ART/relay-up.json" 2>&1
    RELAY_UP_RC=$?
    exit_code relay_up "$RELAY_UP_RC"
    RELAY_UP_OK=$(jq -r '.succeeded // 0' "$ART/relay-up.json" 2>/dev/null)
    if [ "$RELAY_UP_RC" = "0" ] && [ "${RELAY_UP_OK:-0}" = "2" ]; then
        record R01_relay_up passed "2/2 sessions delivered through the relay"
    else
        record R01_relay_up failed "succeeded=$RELAY_UP_OK rc=$RELAY_UP_RC"
    fi

    printf 'down\n' >"$ART/relay-state"
    sleep 0.5
    python3 "$FIXTURE" client --connect 127.0.0.1:9202 --attempts 6 --timeout 1 --interval 0.2 \
        --expect PONG --label relay-client --log "$ART/fixtures/relay-down.jsonl" >"$ART/relay-down.json" 2>&1
    RELAY_DOWN_RC=$?
    exit_code relay_outage "$RELAY_DOWN_RC"
    RELAY_DOWN_OK=$(jq -r '.succeeded // -1' "$ART/relay-down.json" 2>/dev/null)
    RELAY_DOWN_ATTEMPTS=$(jq -r '.attempts // 0' "$ART/relay-down.json" 2>/dev/null)
    if [ "$RELAY_DOWN_RC" = "0" ] && [ "$RELAY_DOWN_OK" = "0" ] && [ "${RELAY_DOWN_ATTEMPTS:-0}" = "6" ]; then
        record R02_relay_sustained_outage passed "6/6 attempts failed while the relay was cut"
    else
        record R02_relay_sustained_outage failed "succeeded=$RELAY_DOWN_OK attempts=$RELAY_DOWN_ATTEMPTS"
    fi

    printf 'up\n' >"$ART/relay-state"
    sleep 0.5
    python3 "$FIXTURE" client --connect 127.0.0.1:9202 --attempts 2 --timeout 2 --interval 0.1 \
        --expect PONG --label relay-client --log "$ART/fixtures/relay-healed.jsonl" >"$ART/relay-healed.json" 2>&1
    RELAY_HEAL_RC=$?
    exit_code relay_heal "$RELAY_HEAL_RC"
    RELAY_HEAL_OK=$(jq -r '.succeeded // 0' "$ART/relay-healed.json" 2>/dev/null)
    if [ "$RELAY_HEAL_RC" = "0" ] && [ "${RELAY_HEAL_OK:-0}" = "2" ]; then
        record R03_relay_heal passed "2/2 sessions delivered after the relay was restored"
    else
        record R03_relay_heal failed "succeeded=$RELAY_HEAL_OK rc=$RELAY_HEAL_RC"
    fi

    # The relay is NOT an isolation boundary: prove it.
    printf 'down\n' >"$ART/relay-state"
    sleep 0.3
    python3 "$FIXTURE" probe --connect 127.0.0.1:9201 --timeout 2 --expect PONG \
        --label direct-dial >"$ART/relay-direct.json" 2>&1
    DIRECT_RC=$?
    exit_code relay_no_bypass "$DIRECT_RC"
    DIRECT_RESULT=$(jq -r '.result // "unknown"' "$ART/relay-direct.json" 2>/dev/null)
    if [ "$DIRECT_RESULT" = "ok" ]; then
        record R04_relay_no_bypass_not_enforced inconclusive \
            "a direct dial to the listener succeeded while the relay was cut: the relay backend cannot enforce no-bypass"
    else
        record R04_relay_no_bypass_not_enforced failed \
            "expected the direct dial to succeed (proving the relay is not an isolation boundary), got $DIRECT_RESULT"
    fi

    # A missing or corrupt state file must fail closed.
    rm -f "$ART/relay-state"
    sleep 0.5
    python3 "$FIXTURE" client --connect 127.0.0.1:9202 --attempts 2 --timeout 1 --interval 0.1 \
        --expect PONG --label relay-client --log "$ART/fixtures/relay-nostate.jsonl" >"$ART/relay-nostate.json" 2>&1
    NOSTATE_RC=$?
    NOSTATE_OK=$(jq -r '.succeeded // -1' "$ART/relay-nostate.json" 2>/dev/null)
    if [ "$NOSTATE_RC" = "0" ] && [ "$NOSTATE_OK" = "0" ]; then
        record R05_relay_missing_state_fails_closed passed \
            "the relay treated a missing state file as down, so no traffic was forwarded"
    else
        record R05_relay_missing_state_fails_closed failed "succeeded=$NOSTATE_OK rc=$NOSTATE_RC"
    fi
    printf 'up\n' >"$ART/relay-state"
    FINAL_BACKEND="relay"
else
    record T02_backend unsupported "no backend is available on this host"
    FINAL_BACKEND="unsupported"
fi

HOST_SNAPSHOT_FINAL=$(host_snapshot)
RUN_TOKEN_GUESS=$(jq -r '.run_token // empty' "$RUN_DIR/manifest.json" 2>/dev/null)
read -r FINAL_IF FINAL_ADDRS FINAL_ROUTES <<<"$(host_lab_leak "${RUN_TOKEN_GUESS:-nonexistenttoken}")"
if [ "${FINAL_IF:-1}" = "0" ] && [ "${FINAL_ADDRS:-1}" = "0" ] && [ "${FINAL_ROUTES:-1}" = "0" ]; then
    record T99_host_integrity passed \
        "no lab interface, address or route on the host (host-wide snapshot hash unchanged=$([ "$HOST_SNAPSHOT_BEFORE" = "$HOST_SNAPSHOT_FINAL" ] && echo yes || echo no, unrelated host state may change on its own))"
else
    record T99_host_integrity failed \
        "host_links=$FINAL_IF host_addrs=$FINAL_ADDRS host_routes=$FINAL_ROUTES"
fi

jq -n \
    --arg backend "$FINAL_BACKEND" \
    --arg artifacts "$ART" \
    --argjson passed "$PASS" \
    --argjson failed "$FAIL" \
    --argjson skipped "$SKIP" \
    --slurpfile results "$RESULTS" \
    --rawfile exits "$EXITS" \
    '{backend:$backend, artifacts:$artifacts, passed:$passed, failed:$failed, skipped:$skipped,
      results:$results, exit_codes:($exits|split("\n")|map(select(length>0)))}' >"$ART/summary.json"

printf '\npassed=%s failed=%s skipped=%s\n' "$PASS" "$FAIL" "$SKIP"
printf 'artifacts: %s\n' "$ART"

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
if [ "$FINAL_BACKEND" = "unsupported" ]; then
    exit 2
fi
exit 0
