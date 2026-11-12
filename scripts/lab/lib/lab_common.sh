#!/usr/bin/env bash
# Shared helpers for the LP-4a lab scripts.
#
# This file is sourced, never executed.  It contains only helpers that are safe
# to use from `netns-preflight.sh` and `link-fault.sh`.
#
# Nothing in here touches the host network, the host routing table, host
# nftables rules or host sysctls.  Every helper either inspects the calling
# process or operates on resources whose names carry the per-run token.

LAB_COMMON_SH=1

# Bounded deadlines.  Every wait in the lab scripts is bounded so a broken
# host fails the run instead of hanging it.
LAB_DEFAULT_DEADLINE_S=30
LAB_POLL_INTERVAL_S=0.05

lab_die() {
    printf 'lab: %s\n' "$*" >&2
    exit 1
}

lab_have() {
    command -v "$1" >/dev/null 2>&1
}

lab_now_ms() {
    date +%s%3N
}

lab_now_iso() {
    date -u +%Y-%m-%dT%H:%M:%SZ
}

# 8 hex characters of per-run entropy.  The token appears in every resource the
# run creates (interface names, nft table names, holder argv, socket path,
# run directory) so ownership can be proven and cleanup can be scoped.
lab_run_token() {
    local t
    t=$(od -An -tx1 -N4 /dev/urandom | tr -d ' \n')
    printf '%s' "${t:-00000000}"
}

# Field 22 (starttime) of /proc/<pid>/stat.  Used as the process identity
# token: a PID can be recycled, a (pid, starttime) pair cannot.
lab_start_time_token() {
    local pid="$1"
    [ -r "/proc/${pid}/stat" ] || return 1
    awk '{ n=split($0, a, ") "); if (n < 2) exit 1; split(a[2], f, " "); print f[20] }' \
        "/proc/${pid}/stat"
}

# True when the pid is still the exact process that was recorded.
lab_pid_matches_token() {
    local pid="$1" token="$2" cur
    [ -n "$pid" ] && [ -n "$token" ] || return 1
    cur=$(lab_start_time_token "$pid" 2>/dev/null) || return 1
    [ "$cur" = "$token" ]
}

lab_tool_path() {
    command -v "$1" 2>/dev/null || true
}

lab_tool_version() {
    local tool="$1"
    case "$tool" in
        ip|tc) "$tool" -V 2>&1 | head -1 ;;
        *) "$tool" --version 2>&1 | head -1 ;;
    esac
}

# True when the current process already runs inside a non-initial user
# namespace.  Used by the preflight so the report does not claim more than
# the environment provides.
lab_in_user_namespace() {
    local m
    m=$(awk 'NF>=3 {print $1" "$2" "$3; exit}' /proc/self/uid_map 2>/dev/null || true)
    case "$m" in
        # The initial user namespace identity map.
        "0 0 4294967295"|"0 0 65536") return 1 ;;
        "") return 1 ;;
        *) return 0 ;;
    esac
}

lab_in_container_hint() {
    if [ -f /.dockerenv ]; then
        printf 'docker'
    elif grep -qaE '(docker|containerd|kubepods)' /proc/1/cgroup 2>/dev/null; then
        printf 'container-cgroup'
    elif [ -n "${container:-}" ]; then
        printf 'env'
    else
        printf 'none'
    fi
}

# Wait for a file to appear, bounded.  Returns 0 on success, 1 on timeout.
lab_wait_for_file() {
    local path="$1" deadline_s="${2:-$LAB_DEFAULT_DEADLINE_S}" waited=0
    while [ ! -s "$path" ]; do
        sleep "$LAB_POLL_INTERVAL_S"
        waited=$(awk -v w="$waited" -v i="$LAB_POLL_INTERVAL_S" 'BEGIN{printf "%d", w + i*1000}')
        [ "$waited" -ge $((deadline_s * 1000)) ] && return 1
    done
    return 0
}

# Wait for a unix socket to accept a connection, bounded.
lab_wait_for_socket() {
    local path="$1" deadline_s="${2:-$LAB_DEFAULT_DEADLINE_S}" waited=0
    while [ ! -S "$path" ]; do
        sleep "$LAB_POLL_INTERVAL_S"
        waited=$(awk -v w="$waited" -v i="$LAB_POLL_INTERVAL_S" 'BEGIN{printf "%d", w + i*1000}')
        [ "$waited" -ge $((deadline_s * 1000)) ] && return 1
    done
    return 0
}
