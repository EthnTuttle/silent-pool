#!/usr/bin/env bash
# Capability probe for the LP-4 Linux network-namespace link-fault backend.
#
# Answers one question: can this host run the LP-4 namespace backend, and if
# not, which weaker backend applies?  It never fakes a capability.  Every
# capability is proven by doing the real thing inside a throwaway user +
# network namespace that the probe creates and destroys.
#
# Usage:
#   scripts/lab/netns-preflight.sh [--out FILE] [--quiet]
#
# Output: one JSON object on stdout (schema spc.lab.netns_preflight.v1).
#
# Exit codes:
#   0  backend "netns": namespace isolation, veth, nftables and netem all work
#   10 backend "netns": namespace isolation, veth and nftables work, but
#      sch_netem is unavailable, so latency/loss scenarios are unsupported
#   20 backend "relay": the namespace backend is unavailable; only the
#      explicitly weaker userspace relay fallback applies
#   21 backend "unsupported": no usable backend at all
set -uo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
LIB_DIR="$SCRIPT_DIR/lib"
# shellcheck source=scripts/lab/lib/lab_common.sh
. "$LIB_DIR/lab_common.sh"

OUT_FILE=""
QUIET=0
while [ $# -gt 0 ]; do
    case "$1" in
        --out) OUT_FILE="$2"; shift 2 ;;
        --out=*) OUT_FILE="${1#*=}"; shift ;;
        --quiet) QUIET=1; shift ;;
        -h|--help)
            sed -n '2,22p' "${BASH_SOURCE[0]}"
            exit 0 ;;
        *) lab_die "unknown argument: $1" ;;
    esac
done

for required in jq python3 ip nft tc unshare nsenter sysctl; do
    lab_have "$required" || lab_die "required tool not found: $required"
done

GEN_MS=$(lab_now_ms)
GEN_ISO=$(lab_now_iso)
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

# --------------------------------------------------------------------- host
uid=$(id -u)
euid=$(id -u)
groups=$(id -G | tr ' ' ',')
host_json=$(jq -n \
    --arg kernel "$(uname -r)" \
    --arg uname "$(uname -a)" \
    --arg os "$(uname -s)" \
    --arg arch "$(uname -m)" \
    --argjson uid "$uid" \
    --argjson euid "$euid" \
    --arg groups "$groups" \
    --arg seccomp "$(awk '/^Seccomp:/ {print $2}' /proc/self/status 2>/dev/null || echo unknown)" \
    --arg unpriv_userns_clone "$(cat /proc/sys/kernel/unprivileged_userns_clone 2>/dev/null || echo unknown)" \
    --arg max_userns "$(cat /proc/sys/user/max_user_namespaces 2>/dev/null || echo unknown)" \
    --arg in_userns "$(lab_in_user_namespace && echo true || echo false)" \
    --arg container "$(lab_in_container_hint)" \
    '{kernel_release:$kernel, uname:$uname, os:$os, arch:$arch, uid:$uid, euid:$euid,
      groups:($groups|split(",")), seccomp_mode:$seccomp,
      unprivileged_userns_clone:$unpriv_userns_clone, max_user_namespaces:$max_userns,
      in_user_namespace:($in_userns=="true"), container_hint:$container}')

# --------------------------------------------------------------------- tools
tools_json='{}'
for tool in unshare nsenter ip nft tc ss python3 jq; do
    path=$(lab_tool_path "$tool" || true)
    version=""
    [ -n "$path" ] && version=$(lab_tool_version "$tool" 2>/dev/null | head -1 || true)
    entry=$(jq -n --arg path "${path:-}" --arg version "$version" \
        '{path:(if $path=="" then null else $path end), available:($path!=""), version:(if $version=="" then null else $version end)}')
    tools_json=$(jq -c --arg k "$tool" --argjson v "$entry" '.[$k]=$v' <<<"$tools_json")
done

# --------------------------------------------------------------------- probes
probe_out="$TMP/probes.jsonl"
probe_err="$TMP/probes.err"
if unshare -Urn --map-root-user -- "$LIB_DIR/nsprobe-inner.sh" >"$probe_out" 2>"$probe_err"; then
    probe_status=0
else
    probe_status=$?
fi

caps_json='{}'
if [ -s "$probe_out" ]; then
    while IFS= read -r line; do
        [ -n "$line" ] || continue
        name=$(jq -r '.probe // empty' <<<"$line" 2>/dev/null) || continue
        [ -n "$name" ] || continue
        caps_json=$(jq -c --arg k "$name" --argjson v "$line" '.[$k]=$v' <<<"$caps_json")
    done <"$probe_out"
fi

# A probe that never ran is unsupported, with the captured reason.
probe_reason=$(head -c 400 "$probe_err" 2>/dev/null | tr '\n' ' ' | sed 's/[[:space:]]\+/ /g')
for cap in user_namespace network_namespace veth nftables sch_netem nested_namespace_pid_handle \
           ipv6_link_local netns_forwarding; do
    if ! jq -e --arg k "$cap" 'has($k)' <<<"$caps_json" >/dev/null 2>&1; then
        reason="probe did not run"
        [ -n "$probe_reason" ] && reason="$probe_reason"
        caps_json=$(jq -c --arg k "$cap" --arg r "$reason" \
            '.[$k]={probe:$k, supported:false, reason:$r, evidence:"no probe output"}' <<<"$caps_json")
    fi
done

cap_supported() {
    jq -e --arg k "$1" '.[$k].supported == true' <<<"$caps_json" >/dev/null 2>&1
}

# ------------------------------------------------------------------- backend
NETNS_OK=1
for cap in user_namespace network_namespace veth nftables nested_namespace_pid_handle; do
    cap_supported "$cap" || NETNS_OK=0
done
NETEM_OK=0
cap_supported sch_netem && NETEM_OK=1

RELAY_OK=0
RELAY_REASON="python3 is unavailable, so the userspace relay fallback cannot run"
if lab_have python3; then
    RELAY_OK=1
    RELAY_REASON="userspace TCP relay fallback is available; it provides a controlled link outage only and cannot enforce isolation"
fi

if [ "$NETNS_OK" = "1" ]; then
    BACKEND="netns"
    EXIT_CODE=0
    BACKEND_REASON="unprivileged user+network namespaces, veth, nftables and PID-handle namespace entry all verified"
    if [ "$NETEM_OK" != "1" ]; then
        EXIT_CODE=10
        BACKEND_REASON="$BACKEND_REASON; sch_netem is unavailable so latency/loss scenarios are unsupported"
    fi
elif [ "$RELAY_OK" = "1" ]; then
    BACKEND="relay"
    EXIT_CODE=20
    BACKEND_REASON="the namespace backend could not be proven on this host; only the explicitly weaker relay backend applies"
else
    BACKEND="unsupported"
    EXIT_CODE=21
    BACKEND_REASON="neither the namespace backend nor the relay fallback is available"
fi

# ---------------------------------------------------------------- guarantees
if [ "$BACKEND" = "netns" ]; then
    guarantees='["isolated_listeners","sustained_partition","link_scoped_fault","heal_restores_delivery",
        "listener_stays_bound_during_fault","no_bypass_routing_enforced","reconnect_failures_observed",
        "ipv6_link_local_isolated","namespace_forwarding_disabled"]'
    if [ "$NETEM_OK" = "1" ]; then
        guarantees=$(jq -c '. + ["netem_latency_and_loss"]' <<<"$guarantees")
    fi
else
    guarantees='["controlled_link_outage","heal_restores_delivery","reconnect_failures_observed"]'
fi

# The unsupported list is scoped to the backend that was actually selected.
# A namespace backend does not carry the relay-only inconclusive entries:
# emitting them would tell an LP-3 preflight that every namespace scenario is
# inconclusive.
unsupported_requests_json=$(cat <<'JSON'
[
 {"request":"full_node_p2p_links","reason":"LP-4 integration gate: no full node is started by this precursor","assertion_state":"not_applicable"},
 {"request":"ark_backend_links","reason":"LP-4 integration gate: the Ark stub and its links are not part of this precursor","assertion_state":"not_applicable"},
 {"request":"mining_and_reorg_scenarios","reason":"LP-4 integration gate: no mining workload runs in this precursor","assertion_state":"not_applicable"}
]
JSON
)
if [ "$BACKEND" != "netns" ]; then
    unsupported_requests_json=$(jq -c '. + [
        {"request":"sustained_partition_assertion","reason":"the relay backend cannot enforce a kernel-level partition","assertion_state":"inconclusive"},
        {"request":"no_bypass_assertion","reason":"a relay cannot prevent a direct dial to the listener","assertion_state":"inconclusive"},
        {"request":"os_enforced_isolation","reason":"a relay is a userspace proxy, not an isolation boundary","assertion_state":"inconclusive"}]' <<<"$unsupported_requests_json")
fi
if [ "$BACKEND" = "netns" ] && [ "$NETEM_OK" != "1" ]; then
    unsupported_requests_json=$(jq -c '. + [{"request":"latency_and_loss_scenarios","reason":"sch_netem is unavailable on this host","assertion_state":"inconclusive"}]' <<<"$unsupported_requests_json")
fi

guarantee_scope_json='{}'
if [ "$BACKEND" = "netns" ]; then
    guarantee_scope_json=$(jq -nc '{
        sustained_partition:"all traffic on the link interface, IPv4 and IPv6, both directions, until the fault is healed",
        no_bypass_routing_enforced:"IPv4 and IPv6 routing of the lab namespaces; forwarding is disabled inside every namespace",
        heal_restores_delivery:"the fault is removed and the link carries traffic again"}')
else
    guarantee_scope_json=$(jq -nc '{
        controlled_link_outage:"a userspace relay can be cut; it cannot enforce isolation",
        heal_restores_delivery:"the relay is restored and traffic flows again"}')
fi

# Consistency: a request must not be both guaranteed and unsupported.
overlap=$(jq -nc --argjson g "$guarantees" --argjson u "$unsupported_requests_json" \
    '[$g[] | select(. as $x | [$u[].request] | index($x))]')
if [ "$overlap" != "[]" ]; then
    printf '{"ok":false,"error":{"code":"inconsistent_preflight","detail":"guarantees and unsupported_requests overlap","overlap":%s}}\n' \
        "$overlap" | jq .
    exit 3
fi

result=$(jq -n \
    --arg schema "spc.lab.netns_preflight.v1" \
    --argjson generated_ms "$GEN_MS" \
    --arg generated_iso "$GEN_ISO" \
    --argjson host "$host_json" \
    --argjson tools "$tools_json" \
    --argjson capabilities "$caps_json" \
    --arg backend "$BACKEND" \
    --arg backend_reason "$BACKEND_REASON" \
    --argjson exit_code "$EXIT_CODE" \
    --argjson guarantees "$guarantees" \
    --argjson guarantee_scope "$guarantee_scope_json" \
    --argjson unsupported_requests "$unsupported_requests_json" \
    --argjson netns_backend_supported "$([ "$NETNS_OK" = 1 ] && echo true || echo false)" \
    --argjson netem_supported "$([ "$NETEM_OK" = 1 ] && echo true || echo false)" \
    --argjson relay_available "$([ "$RELAY_OK" = 1 ] && echo true || echo false)" \
    --arg relay_reason "$RELAY_REASON" \
    --argjson probe_status "$probe_status" \
    '{
       schema:$schema,
       generated_at:{unix_ms:$generated_ms, iso:$generated_iso},
       host:$host,
       tools:$tools,
       capabilities:$capabilities,
       backend:$backend,
       backend_reason:$backend_reason,
       exit_code:$exit_code,
       netns_backend_supported:$netns_backend_supported,
       netem_supported:$netem_supported,
       relay_fallback:{available:$relay_available, reason:$relay_reason},
       probe_subprocess:{exit_code:$probe_status},
       guarantees:$guarantees,
       guarantee_scope:$guarantee_scope,
       unsupported_requests:$unsupported_requests
     }')

if [ -z "$result" ] || ! printf '%s' "$result" | jq -e . >/dev/null 2>&1; then
    printf '{"ok":false,"error":{"code":"preflight_assembly_failed","detail":"the preflight artifact could not be assembled"}}\n' | jq .
    exit 3
fi

if [ -n "$OUT_FILE" ]; then
    mkdir -p "$(dirname "$OUT_FILE")"
    printf '%s\n' "$result" >"$OUT_FILE"
    [ "$QUIET" = "1" ] || printf '%s\n' "$result"
else
    printf '%s\n' "$result"
fi

exit "$EXIT_CODE"
