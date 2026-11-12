#!/usr/bin/env bash
# LP-4a link-fault CLI.
#
# Drives the Linux network-namespace link-fault backend
# (`scripts/lab/lib/lab_backend.py`).  The backend owns one topology: one
# network namespace per endpoint, one veth pair per link, explicit addresses
# and no default route.
#
# Commands:
#   up              --spec FILE [--run-dir DIR] [--run-id ID]
#                  [--idle-timeout-s S] [--max-lifetime-s S]
#                  (a run directory that is not empty is never reused; use a
#                   fresh directory after a crash)
#   apply           --run-dir DIR --link ID --policy normal|partition|latency:MS|loss:PCT
#   heal            --run-dir DIR --link ID
#   status          --run-dir DIR [--link ID]
#   counters        --run-dir DIR --link ID
#   no-bypass-check --run-dir DIR
#   exec            --run-dir DIR --endpoint NAME [--timeout S] [--raw] -- CMD [ARG...]
#   spawn           --run-dir DIR --endpoint NAME --log FILE --label NAME -- CMD [ARG...]
#   manifest        --run-dir DIR
#   down            --run-dir DIR
#
# Every command prints one JSON object on stdout.  Exit codes: 0 success,
# 1 the backend rejected the request, 2 usage error, 3 the backend is not
# reachable.
#
# The backend never writes to /run/netns, never touches host interfaces,
# host routes, host nftables rules or host sysctls, and never needs sudo.
set -uo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
LIB_DIR="$SCRIPT_DIR/lib"
REPO_ROOT=$(cd "$SCRIPT_DIR/../.." && pwd)
# shellcheck source=scripts/lab/lib/lab_common.sh
. "$LIB_DIR/lab_common.sh"

for required in jq python3 ip nft tc unshare nsenter sysctl; do
    lab_have "$required" || lab_die "required tool not found: $required"
done

usage() {
    sed -n '2,28p' "${BASH_SOURCE[0]}"
}

fail_usage() {
    printf '{"ok":false,"error":{"code":"usage_error","detail":"%s"}}\n' "$1" >&2
    usage >&2
    exit 2
}

CMD="${1:-}"
[ -n "$CMD" ] || fail_usage "a command is required"
shift

RUN_DIR=""
RUN_ID=""
SPEC=""
LINK=""
POLICY=""
ENDPOINT=""
LOG_FILE=""
LABEL=""
TIMEOUT=""
IDLE_TIMEOUT=""
MAX_LIFETIME=""
RAW=0

while [ $# -gt 0 ]; do
    case "$1" in
        --run-dir) RUN_DIR="$2"; shift 2 ;;
        --run-dir=*) RUN_DIR="${1#*=}"; shift ;;
        --run-id) RUN_ID="$2"; shift 2 ;;
        --run-id=*) RUN_ID="${1#*=}"; shift ;;
        --spec) SPEC="$2"; shift 2 ;;
        --spec=*) SPEC="${1#*=}"; shift ;;
        --link) LINK="$2"; shift 2 ;;
        --link=*) LINK="${1#*=}"; shift ;;
        --policy) POLICY="$2"; shift 2 ;;
        --policy=*) POLICY="${1#*=}"; shift ;;
        --endpoint) ENDPOINT="$2"; shift 2 ;;
        --endpoint=*) ENDPOINT="${1#*=}"; shift ;;
        --log) LOG_FILE="$2"; shift 2 ;;
        --log=*) LOG_FILE="${1#*=}"; shift ;;
        --label) LABEL="$2"; shift 2 ;;
        --label=*) LABEL="${1#*=}"; shift ;;
        --timeout) TIMEOUT="$2"; shift 2 ;;
        --timeout=*) TIMEOUT="${1#*=}"; shift ;;
        --idle-timeout-s) IDLE_TIMEOUT="$2"; shift 2 ;;
        --idle-timeout-s=*) IDLE_TIMEOUT="${1#*=}"; shift ;;
        --max-lifetime-s) MAX_LIFETIME="$2"; shift 2 ;;
        --max-lifetime-s=*) MAX_LIFETIME="${1#*=}"; shift ;;
        --leave-for-adoption)
            printf '{"ok":false,"error":{"code":"not_implemented","detail":"--leave-for-adoption is not implemented: this backend always ties the namespace holders and the fixtures to the supervisor lifetime with PR_SET_PDEATHSIG (adoption_model=pdeathsig, supports_survive_supervisor=false). A runner that needs adoption has to own the lifetime itself."}}\n' | jq .
            exit 1 ;;
        --raw) RAW=1; shift ;;
        --) shift; break ;;
        -h|--help) usage; exit 0 ;;
        *) fail_usage "unknown argument: $1" ;;
    esac
done

# Commands that take a trailing command line.
EXTRA_ARGV=()
if [ "$CMD" = "exec" ] || [ "$CMD" = "spawn" ]; then
    if [ $# -gt 0 ]; then
        EXTRA_ARGV=("$@")
    fi
fi

policy_json() {
    local raw="$1"
    case "$raw" in
        normal) jq -nc '{type:"normal"}' ;;
        partition) jq -nc '{type:"partition"}' ;;
        latency:*)
            local ms="${raw#latency:}"
            case "$ms" in
                ''|*[!0-9]*) return 1 ;;
            esac
            jq -nc --argjson ms "$ms" '{type:"latency", delay_ms:$ms}' ;;
        loss:*)
            local pct="${raw#loss:}"
            case "$pct" in
                ''|*[!0-9]*) return 1 ;;
            esac
            jq -nc --argjson pct "$pct" '{type:"loss", loss_percent:$pct}' ;;
        *) return 1 ;;
    esac
}

sock_request() {
    local sock="$1" request="$2"
    python3 "$LIB_DIR/lab_sock.py" "$sock" "$request"
}

# The argv is carried as JSON end to end.  Newlines and other bytes inside an
# argument survive; nothing is split on whitespace.
argv_json() {
    if [ ${#EXTRA_ARGV[@]} -eq 0 ]; then
        printf '[]'
        return
    fi
    jq -n '$ARGS.positional' --args -- "${EXTRA_ARGV[@]}"
}

# ------------------------------------------------------------------ up
cmd_up() {
    [ -n "$SPEC" ] || fail_usage "up needs --spec"
    [ -f "$SPEC" ] || fail_usage "spec file not found: $SPEC"
    if [ -z "$RUN_DIR" ]; then
        RUN_DIR="/tmp/opencode/LP4a-$(date -u +%Y%m%dT%H%M%SZ)-$(lab_run_token)"
    fi
    mkdir -p -m 0700 "$RUN_DIR" || fail_usage "cannot create run directory: $RUN_DIR"
    chmod 0700 "$RUN_DIR"
    RUN_DIR=$(cd "$RUN_DIR" && pwd)

    # A run directory belongs to exactly one backend.  A directory that is
    # not empty is never reused: a live backend keeps it, and a stale one
    # (for example after a crash) keeps its diagnostics and must be replaced
    # by a fresh directory.  Nothing is ever unlinked here, so no other
    # owner's files or socket can be destroyed.
    if [ -n "$(ls -A "$RUN_DIR" 2>/dev/null)" ]; then
        local sup_pid sup_token alive
        sup_pid=$(jq -r '.supervisor.pid // empty' "$RUN_DIR/manifest.json" 2>/dev/null)
        sup_token=$(jq -r '.supervisor.start_time_token // empty' "$RUN_DIR/manifest.json" 2>/dev/null)
        alive=false
        if [ -n "$sup_pid" ] && [ -n "$sup_token" ]; then
            local cur
            cur=$(lab_start_time_token "$sup_pid" 2>/dev/null || true)
            [ -n "$cur" ] && [ "$cur" = "$sup_token" ] && alive=true
        fi
        if [ "$alive" = "true" ]; then
            printf '{"ok":false,"error":{"code":"run_dir_in_use","detail":"a live backend already owns this run directory","supervisor":{"pid":%s,"start_time_token":"%s","alive":true},"control_socket":"%s","fresh_dir_required":false}}\n' \
                "$sup_pid" "$sup_token" "$RUN_DIR/control.sock" | jq .
            return 1
        fi
        # The recorded supervisor is gone: the directory is stale.  It is not
        # reused and nothing in it is deleted, so the diagnostics of the dead
        # run stay available.
        printf '{"ok":false,"error":{"code":"stale_run_dir","detail":"the run directory is not empty and its backend is gone; use a fresh directory (nothing was deleted, the previous diagnostics are preserved)","supervisor":{"pid":%s,"start_time_token":"%s","alive":false},"files":["%s"],"fresh_dir_required":true}}\n' \
            "${sup_pid:-0}" "${sup_token:-}" \
            "$(cd "$RUN_DIR" && ls -A | tr '\n' ',')" | jq .
        return 1
    fi

    local token
    token=$(lab_run_token)
    local spec_copy="$RUN_DIR/topology.json"
    cp "$SPEC" "$spec_copy" || fail_usage "cannot copy the topology spec"
    chmod 0600 "$spec_copy"

    # Host addresses are captured outside the namespace so the backend can
    # prove that no namespace can reach the host.  Both families matter: an
    # IPv6 path would be a bypass too.
    local host_args=()
    while IFS= read -r addr; do
        [ -n "$addr" ] && host_args+=(--host-address "$addr")
    done < <(ip -4 -o addr show scope global 2>/dev/null | awk '{print $4}' | cut -d/ -f1)
    while IFS= read -r addr; do
        [ -n "$addr" ] && host_args+=(--host-address-v6 "$addr")
    done < <(ip -6 -o addr show scope global 2>/dev/null | awk '{print $4}' | cut -d/ -f1)

    local -a backend_cmd=(
        python3 "$LIB_DIR/lab_backend.py"
        --run-dir "$RUN_DIR"
        --spec "$spec_copy"
        --run-token "$token"
        --fixture-script "$REPO_ROOT/testing/lab-isolation/tcp_fixture.py"
    )
    if [ -n "$RUN_ID" ]; then
        backend_cmd+=(--run-id "$RUN_ID")
    fi
    if [ ${#host_args[@]} -gt 0 ]; then
        backend_cmd+=("${host_args[@]}")
    fi
    if [ -n "$IDLE_TIMEOUT" ]; then
        backend_cmd+=(--idle-timeout-s "$IDLE_TIMEOUT")
    fi
    if [ -n "$MAX_LIFETIME" ]; then
        backend_cmd+=(--max-lifetime-s "$MAX_LIFETIME")
    fi

    # The backend runs inside an ephemeral user + network namespace.  It never
    # needs sudo and it never touches the host network.
    setsid unshare -Urn --map-root-user -- "${backend_cmd[@]}" \
        >"$RUN_DIR/backend.log" 2>&1 &

    if ! lab_wait_for_file "$RUN_DIR/manifest.json" 30; then
        local detail="the backend did not create a manifest within 30s"
        if [ -f "$RUN_DIR/up-error-$token.json" ]; then
            jq . "$RUN_DIR/up-error-$token.json"
            return 1
        fi
        printf '{"ok":false,"error":{"code":"backend_unavailable","detail":"%s","backend_log":"%s"}}\n' \
            "$detail" "$RUN_DIR/backend.log" | jq .
        return 3
    fi
    # The manifest that appeared must be the one of THIS backend: a stale
    # manifest must never satisfy a startup.
    local manifest_token manifest_pid
    manifest_token=$(jq -r '.run_token // empty' "$RUN_DIR/manifest.json" 2>/dev/null)
    manifest_pid=$(jq -r '.supervisor.pid // 0' "$RUN_DIR/manifest.json" 2>/dev/null)
    if [ "$manifest_token" != "$token" ]; then
        printf '{"ok":false,"error":{"code":"stale_manifest","detail":"the manifest in the run directory belongs to another run (token %s, expected %s); the startup is not satisfied","manifest_run_token":"%s","expected_run_token":"%s"}}\n' \
            "${manifest_token:-none}" "$token" "${manifest_token:-none}" "$token" | jq .
        return 3
    fi
    if ! lab_wait_for_socket "$RUN_DIR/control.sock" 15; then
        printf '{"ok":false,"error":{"code":"backend_unavailable","detail":"the backend did not create its control socket","backend_log":"%s"}}\n' \
            "$RUN_DIR/backend.log" | jq .
        return 3
    fi
    # The control socket must answer before the startup counts as successful.
    local ping
    ping=$(sock_request "$RUN_DIR/control.sock" '{"op":"ping"}' 2>/dev/null || true)
    if [ -z "$ping" ] || [ "$(jq -r '.ok // false' <<<"$ping" 2>/dev/null)" != "true" ]; then
        printf '{"ok":false,"error":{"code":"backend_unavailable","detail":"the new backend did not answer a control request","backend_log":"%s"}}\n' \
            "$RUN_DIR/backend.log" | jq .
        return 3
    fi
    cat "$RUN_DIR/manifest.json" | jq --argjson pid "$manifest_pid" \
        '. + {ok:true, op:"up", control_handshake:true, supervisor_pid:$pid}'
    return 0
}

# -------------------------------------------------------------- requests
cmd_request() {
    local op="$1" request="$2"
    if [ -z "$RUN_DIR" ]; then
        fail_usage "$op needs --run-dir"
    fi
    local sock="$RUN_DIR/control.sock"
    if [ ! -S "$sock" ]; then
        printf '{"ok":false,"error":{"code":"backend_unavailable","detail":"no control socket at %s"}}\n' "$sock" | jq .
        return 3
    fi
    local json rc
    json=$(sock_request "$sock" "$request")
    rc=$?
    if [ -z "$json" ]; then
        printf '{"ok":false,"error":{"code":"backend_unavailable","detail":"no response from the backend"}}\n' | jq .
        return 3
    fi
    if [ "$rc" = "2" ]; then
        printf '%s\n' "$json" | jq . 2>/dev/null || printf '%s\n' "$json"
        return 3
    fi
    if [ "$RAW" = "1" ]; then
        printf '%s' "$(jq -r '.stdout // ""' <<<"$json")"
    else
        printf '%s\n' "$json" | jq .
    fi
    return "$rc"
}

case "$CMD" in
    up)
        cmd_up
        exit $?
        ;;
    apply)
        [ -n "$LINK" ] || fail_usage "apply needs --link"
        [ -n "$POLICY" ] || fail_usage "apply needs --policy"
        pjson=$(policy_json "$POLICY") || fail_usage "invalid policy: $POLICY"
        req=$(jq -nc --arg link "$LINK" --argjson policy "$pjson" \
            '{op:"apply", link:$link, policy:$policy}')
        cmd_request apply "$req"
        exit $?
        ;;
    heal)
        [ -n "$LINK" ] || fail_usage "heal needs --link"
        req=$(jq -nc --arg link "$LINK" '{op:"heal", link:$link}')
        cmd_request heal "$req"
        exit $?
        ;;
    status)
        if [ -n "$LINK" ]; then
            req=$(jq -nc --arg link "$LINK" '{op:"status", link:$link}')
        else
            req=$(jq -nc '{op:"status"}')
        fi
        cmd_request status "$req"
        exit $?
        ;;
    counters)
        [ -n "$LINK" ] || fail_usage "counters needs --link"
        req=$(jq -nc --arg link "$LINK" '{op:"counters", link:$link}')
        cmd_request counters "$req"
        exit $?
        ;;
    no-bypass-check)
        req=$(jq -nc '{op:"no-bypass-check"}')
        cmd_request no-bypass-check "$req"
        exit $?
        ;;
    manifest)
        req=$(jq -nc '{op:"manifest"}')
        cmd_request manifest "$req"
        exit $?
        ;;
    exec)
        [ -n "$ENDPOINT" ] || fail_usage "exec needs --endpoint"
        [ ${#EXTRA_ARGV[@]} -gt 0 ] || fail_usage "exec needs a command after --"
        req=$(jq -nc --arg ep "$ENDPOINT" --arg timeout "$TIMEOUT" \
            --argjson argv "$(argv_json)" \
            '{op:"exec", endpoint:$ep, timeout_s:(if $timeout=="" then null else ($timeout|tonumber) end), argv:$argv}')
        cmd_request exec "$req"
        exit $?
        ;;
    spawn)
        [ -n "$ENDPOINT" ] || fail_usage "spawn needs --endpoint"
        [ -n "$LOG_FILE" ] || fail_usage "spawn needs --log"
        [ ${#EXTRA_ARGV[@]} -gt 0 ] || fail_usage "spawn needs a command after --"
        req=$(jq -nc --arg ep "$ENDPOINT" --arg log "$LOG_FILE" --arg lbl "$LABEL" \
            --argjson argv "$(argv_json)" \
            '{op:"spawn", endpoint:$ep, log_path:$log, label:$lbl, argv:$argv}')
        cmd_request spawn "$req"
        exit $?
        ;;
    down)
        req=$(jq -nc '{op:"down"}')
        cmd_request down "$req"
        exit $?
        ;;
    -h|--help)
        usage
        exit 0
        ;;
    *)
        fail_usage "unknown command: $CMD"
        ;;
esac
