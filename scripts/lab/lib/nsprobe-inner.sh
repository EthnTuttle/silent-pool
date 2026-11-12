#!/usr/bin/env bash
# Inner probe for `scripts/lab/netns-preflight.sh`.
#
# This script runs INSIDE an ephemeral user + network namespace created by the
# caller with `unshare -Urn --map-root-user`.  It probes the capabilities the
# LP-4 link-fault backend needs and prints one JSON object per line.
#
# It never touches the host network, host routes, host nftables rules or host
# sysctls: everything it creates lives in the throwaway namespace it was
# entered into, and it deletes what it creates before exiting.
set -u

emit() {
    # emit <probe> <supported:true|false> <reason> <evidence>
    jq -nc \
        --arg probe "$1" \
        --argjson supported "$2" \
        --arg reason "$3" \
        --arg evidence "$4" \
        '{probe:$probe, supported:$supported, reason:(if $reason=="" then null else $reason end), evidence:$evidence}'
}

capeff=$(awk '/^CapEff:/ {print $2}' /proc/self/status 2>/dev/null || echo 0)
uid=$(id -u)
cap_sys_admin=0
cap_net_admin=0
if [ -n "$capeff" ]; then
    # shellcheck disable=SC2004
    cap_sys_admin=$(( (0x$capeff & (1 << 21)) != 0 ? 1 : 0 ))
    # shellcheck disable=SC2004
    cap_net_admin=$(( (0x$capeff & (1 << 12)) != 0 ? 1 : 0 ))
fi

# ---------------------------------------------------------------- user namespace
if [ "$uid" = "0" ] && [ "$cap_sys_admin" = "1" ] && [ "$cap_net_admin" = "1" ]; then
    emit user_namespace true "" \
        "uid=0 capeff=0x$capeff cap_sys_admin=1 cap_net_admin=1"
else
    emit user_namespace false \
        "unprivileged user namespace did not yield uid 0 with CAP_SYS_ADMIN and CAP_NET_ADMIN" \
        "uid=$uid capeff=0x$capeff cap_sys_admin=$cap_sys_admin cap_net_admin=$cap_net_admin"
fi

# ---------------------------------------------------------------- network namespace
links=$(ip -o link show 2>/dev/null | wc -l)
if ip -o link show 2>/dev/null | grep -q ' lo:'; then
    emit network_namespace true "" "fresh namespace contains only loopback (link_count=$links)"
else
    emit network_namespace false "no loopback device inside the created namespace" "link_count=$links"
fi

# ---------------------------------------------------------------- veth
veth_ok=false
veth_ev="ip link add failed"
tmp=$(mktemp -d 2>/dev/null || echo /tmp/lab4a-probe.$$)
mkdir -p "$tmp"
if ip link add labprobe0 type veth peer name labprobe1 2>"$tmp/veth.err"; then
    if ip -o link show labprobe0 2>/dev/null | grep -q 'labprobe0@'; then
        veth_ok=true
        veth_ev="created veth pair labprobe0/labprobe1 inside the probe namespace"
    fi
fi
emit veth "$veth_ok" "$( [ "$veth_ok" = true ] || cat "$tmp/veth.err" )" "$veth_ev"

# ---------------------------------------------------------------- nftables
nft_ok=false
nft_ev="nft add table failed"
if nft add table inet labprobe 2>"$tmp/nft.err"; then
    if nft add chain inet labprobe out '{ type filter hook output priority 0; policy accept; }' 2>>"$tmp/nft.err"; then
        if nft add rule inet labprobe out ip daddr 198.51.100.7 counter drop 2>>"$tmp/nft.err"; then
            if nft -j list table inet labprobe 2>>"$tmp/nft.err" | jq -e . >/dev/null 2>&1; then
                nft_ok=true
                nft_ev="added table/chain/rule inet labprobe and listed it as JSON"
            fi
        fi
    fi
    nft delete table inet labprobe >/dev/null 2>&1
fi
emit nftables "$nft_ok" "$( [ "$nft_ok" = true ] || cat "$tmp/nft.err" )" "$nft_ev"

# ---------------------------------------------------------------- sch_netem
netem_ok=false
netem_ev="tc qdisc add netem failed"
if tc qdisc add dev labprobe0 root netem delay 1ms 2>"$tmp/netem.err"; then
    if tc qdisc show dev labprobe0 2>>"$tmp/netem.err" | grep -q 'netem'; then
        netem_ok=true
        netem_ev="installed root netem qdisc on labprobe0"
    fi
fi
tc qdisc del dev labprobe0 root >/dev/null 2>&1
emit sch_netem "$netem_ok" "$( [ "$netem_ok" = true ] || cat "$tmp/netem.err" )" "$netem_ev"

if [ -d /sys/module/sch_netem ]; then
    emit sch_netem_module true "" "sch_netem is present in /sys/module"
else
    emit sch_netem_module false "sch_netem module directory not visible" "ls /sys/module/sch_netem failed"
fi

# --------------------------------------------- nested namespace by PID handle
# The backend never writes to /run/netns (root owned).  It creates a child
# namespace and keeps the holder PID, then enters it by /proc/<pid>/ns/net.
nested_ok=false
nested_ev="nested namespace creation failed"
child_pid=""
if command -v unshare >/dev/null 2>&1; then
    unshare --net --fork -- bash -c 'echo $$ > "$1"; exec sleep 30' _ "$tmp/child.pid" >/dev/null 2>&1 &
    waited=0
    while [ ! -s "$tmp/child.pid" ] && [ "$waited" -lt 50 ]; do
        sleep 0.1
        waited=$((waited + 1))
    done
    child_pid=$(cat "$tmp/child.pid" 2>/dev/null || true)
    if [ -n "$child_pid" ] && [ -e "/proc/$child_pid/ns/net" ]; then
        self_ns=$(readlink /proc/self/ns/net 2>/dev/null || echo "")
        child_ns=$(readlink "/proc/$child_pid/ns/net" 2>/dev/null || echo "")
        if [ -n "$child_ns" ] && [ "$self_ns" != "$child_ns" ]; then
            if command -v nsenter >/dev/null 2>&1 && nsenter -t "$child_pid" -n ip -o link show >/dev/null 2>&1; then
                if python3 - "$child_pid" >"$tmp/setns.out" 2>&1 <<'PY'
import ctypes, os, sys
pid = sys.argv[1]
libc = ctypes.CDLL("libc.so.6", use_errno=True)
fd = os.open("/proc/%s/ns/net" % pid, os.O_RDONLY)
if libc.setns(fd, 0x40000000) != 0:
    err = ctypes.get_errno()
    print("SETNS_FAIL", os.strerror(err))
    sys.exit(1)
print("SETNS_OK")
PY
                then
                    if grep -q SETNS_OK "$tmp/setns.out"; then
                        nested_ok=true
                        nested_ev="child namespace $child_ns reachable by PID handle; nsenter and setns(CLONE_NEWNET) both succeeded"
                    else
                        nested_ev="setns into the child namespace failed: $(cat "$tmp/setns.out")"
                    fi
                else
                    nested_ev="python3 setns probe failed: $(cat "$tmp/setns.out")"
                fi
            else
                nested_ev="nsenter into the child namespace failed"
            fi
        else
            nested_ev="child namespace handle did not resolve to a distinct namespace"
        fi
    fi
fi
emit nested_namespace_pid_handle "$nested_ok" \
    "$( [ "$nested_ok" = true ] || echo "nested namespace PID-handle design not usable in this environment" )" \
    "$nested_ev"

# ------------------------------------------- IPv6 link-local on a lab veth
# Every veth in the lab gets a kernel generated `fe80::/64` address and a
# matching route.  That is an extra path across the link, so the backend has
# to know whether it exists on this kernel.
ip link set labprobe0 up 2>/dev/null
ip link set labprobe1 up 2>/dev/null
sleep 0.3
ll_addr=$(ip -6 -o addr show dev labprobe0 2>/dev/null | awk '$4 ~ /^fe80:/ {print $4}' | head -1)
ll_route=$(ip -6 route show 2>/dev/null | grep -c 'fe80::/64' || true)
if [ -n "$ll_addr" ]; then
    emit ipv6_link_local true "" \
        "the kernel assigns $ll_addr with $ll_route fe80::/64 route(s); the partition must cover IPv6"
else
    emit ipv6_link_local false \
        "no IPv6 link-local address appeared on the probe veth" \
        "ip -6 addr show dev labprobe0 returned nothing"
fi

# ------------------------------------------------------- inherited forwarding
fwd4=$(cat /proc/sys/net/ipv4/ip_forward 2>/dev/null || echo unknown)
fwd6=$(cat /proc/sys/net/ipv6/conf/all/forwarding 2>/dev/null || echo unknown)
if [ "$fwd4" = "unknown" ]; then
    emit netns_forwarding false "the forwarding sysctls could not be read" "unreadable"
else
    emit netns_forwarding true "" \
        "inherited net.ipv4.ip_forward=$fwd4 net.ipv6.conf.all.forwarding=$fwd6; the backend disables both inside every lab namespace"
fi

if [ -n "$child_pid" ]; then
    kill "$child_pid" 2>/dev/null || true
fi
ip link del labprobe0 >/dev/null 2>&1
rm -rf "$tmp"
exit 0
