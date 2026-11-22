# One-command launcher for the self-hosted regtest lab (docs/LAB-QUICKSTART.md).
#
# Pure tooling around existing packages: it runs the unchanged
# `spc-lab-self-hosted-ark` wrapper, `spc-lab-observability` and
# `spc-lab reap`. It chooses a fresh, short run directory, fills in the
# provenance arguments, and turns Ctrl-C / `stop` into the lab's own
# interactive stop. No lab, pool or scenario behavior changes.
#
# The controller runs in its own session with a FIFO as stdin, so a terminal
# Ctrl-C never reaches it and any process may request the clean stop by
# writing `stop` to `<home>/<name>/control`.
{ pkgs, runtime, observability, lab, revision, sourceHash }:
pkgs.writeShellApplication {
  name = "spc-lab-quickstart";
  runtimeInputs = [ pkgs.coreutils pkgs.gnugrep pkgs.gnused pkgs.util-linux ];
  text = ''
    self="$(realpath "$0")"

    usage() {
      cat <<'EOF'
    spc-lab-quickstart [start] [--name <name>] [--home <dir>] [lab options]
    spc-lab-quickstart grafana [--name <name>] [--home <dir>] [dashboard options]
    spc-lab-quickstart status  [--name <name>] [--home <dir>]
    spc-lab-quickstart stop    [--name <name>] [--home <dir>]
    spc-lab-quickstart reap    [--name <name>] [--home <dir>]

    start    (default) start a fresh regtest lab in the foreground and print
             the UI addresses once it is up. Stop it with Ctrl-C or by typing
             stop and Enter. Other options go to the lab unchanged, for
             example --pow-bits 0x20008888 or --duration-secs 600.
    grafana  Prometheus and Grafana for a running lab; options such as
             --grafana-port and --prometheus-port are passed on.
    status   print the addresses and token file of a run again.
    stop     ask a running lab to stop cleanly, from any terminal.
    reap     after a crash: end what is left of a run whose controller died.

      --name <name>  run name. start: default qs-<month day>-<time>, must be
                     new. Other commands: default is the most recent run.
      --home <dir>   where runs are kept (default $SPC_LAB_HOME or ~/.spc-lab).
                     Keep it short: <home>/<name>/runtime may be at most 56
                     bytes long.
    EOF
    }

    die() {
      echo "spc-lab-quickstart: $*" >&2
      exit 1
    }

    subcommand=start
    case "''${1:-}" in
      start | grafana | status | stop | reap) subcommand="$1"; shift ;;
    esac

    home="''${SPC_LAB_HOME:-''${HOME:?HOME is not set}/.spc-lab}"
    name=""
    pass=()
    base_revision=(--base-revision ${pkgs.lib.escapeShellArg revision})
    tree_hash=(--dirty-tree-hash ${pkgs.lib.escapeShellArg sourceHash})
    listen=(--listen 127.0.0.1:0)
    while [ "$#" -gt 0 ]; do
      case "$1" in
        --name | --home)
          [ "$#" -ge 2 ] || die "$1 needs a value"
          if [ "$1" = --name ]; then name="$2"; else home="$2"; fi
          shift 2
          ;;
        -h | --help) usage; exit 0 ;;
        *)
          if [ "$subcommand" = start ]; then
            case "$1" in
              --run-dir | --run-dir=*)
                die "the run directory is chosen for you; use --name and --home" ;;
              # Always on; the lab rejects a repeated flag.
              --interactive | --operator-ui) shift; continue ;;
              --base-revision | --base-revision=*) base_revision=() ;;
              --dirty-tree-hash | --dirty-tree-hash=*) tree_hash=() ;;
              --listen | --listen=*) listen=() ;;
            esac
          fi
          pass+=("$1")
          shift
          ;;
      esac
    done
    case "$subcommand" in
      status | stop | reap)
        [ "''${#pass[@]}" -eq 0 ] || die "unexpected argument: ''${pass[0]}" ;;
    esac
    case "$name" in
      "") ;;
      latest | *[!A-Za-z0-9._-]*) die "--name may use letters, digits, '.', '_' and '-' only" ;;
    esac

    umask 077

    # The process named in <work>/controller.pid, only if it still is this
    # run's controller (its command line names the run directory).
    controller_pid() {
      local pid
      pid="$(cat "$1/controller.pid" 2>/dev/null)" || return 1
      case "$pid" in "" | *[!0-9]*) return 1 ;; esac
      tr '\0' '\n' 2>/dev/null < "/proc/$pid/cmdline" \
        | grep -qxF -- "$1/runtime" || return 1
      echo "$pid"
    }

    request_stop() {
      # Opened read-write so this never blocks, even with no reader left.
      if [ -p "$1/control" ]; then echo stop 1<> "$1/control"; fi
    }

    describe() {
      local work="$1" log="$1/controller.log" url pools
      url="$(sed -n 's/^spc-lab-runtime: http=//p' "$log" | head -n 1)"
      pools="$(sed -n 's/^spc-lab-runtime: operator front-\(.*\)=\(.*\)$/  Pool page \1   : \2/p' "$log")"
      echo
      echo "================ silent-pool regtest lab ================"
      if [ -n "$url" ]; then
        echo "  Controls and wallets : $url/"
        echo "  Operator directory   : $url/lab"
        if [ -n "$pools" ]; then echo "$pools"; fi
        echo
        echo "  The pages ask for a run token. Open this file and paste the"
        echo "  value of its \"token\" field (alice.json, bob.json, carol.json"
        echo "  and dave.json beside it open one user's view):"
        echo "    $work/runtime/private/access/operator.json"
      else
        echo "  No UI address was printed by this run."
      fi
      echo
      echo "  Run directory  : $work/runtime"
      echo "  Controller log : $log"
      echo "  Dashboards     : $self grafana"
      echo "  Stop           : $self stop"
      echo "========================================================="
    }

    # ---------------------------------------------------------------- start
    if [ "$subcommand" = start ]; then
      [ "$(id -u)" -ne 0 ] || die "run this as an ordinary user, not root (PostgreSQL refuses root)"
      mkdir -p "$home"
      home="$(cd "$home" && pwd -P)"
      [ -n "$name" ] || name="qs-$(date +%m%d-%H%M%S)"
      work="$home/$name"
      run="$work/runtime"
      run_bytes="$(printf %s "$run" | wc -c)"
      if [ "$run_bytes" -gt 56 ]; then
        die "run directory path is too long ($run_bytes bytes, limit 56): $run
    The lab's PostgreSQL socket lives under it and socket paths are limited to
    107 bytes. Choose a shorter place, for example: --home /var/tmp/lab"
      fi
      mkdir "$work" 2>/dev/null \
        || die "$work already exists; every run needs a new directory (choose another --name)"
      free_gb="$(df --output=avail -BG "$work" | tail -n 1 | tr -dc 0-9)"
      if [ "''${free_gb:-0}" -lt 20 ]; then
        echo "spc-lab-quickstart: warning: only ''${free_gb} GB free under $home; the run directory grows about 1.7 GB per hour" >&2
      fi
      if [ ! -e "$home/latest" ] || [ -L "$home/latest" ]; then
        ln -sfn "$name" "$home/latest"
      fi

      log="$work/controller.log"
      mkfifo "$work/control"
      : > "$log"
      # Own session: a terminal Ctrl-C reaches this launcher only, which turns
      # it into the lab's clean stop instead of killing the controller. A
      # background command of a script starts with SIGINT and SIGQUIT ignored;
      # `env` restores the defaults so the lab sees an ordinary environment.
      setsid env --default-signal=INT,QUIT \
        ${runtime}/bin/spc-lab-self-hosted-ark \
        --run-dir "$run" "''${base_revision[@]}" "''${tree_hash[@]}" \
        "''${listen[@]}" --operator-ui --interactive "''${pass[@]}" \
        0<> "$work/control" > "$log" 2>&1 &
      lab_pid=$!
      echo "$lab_pid" > "$work/controller.pid"

      echo "spc-lab-quickstart: starting the lab in $run"
      echo "spc-lab-quickstart: startup takes from 15 seconds to two minutes and prints nothing until it is done"

      (trap "" INT HUP TERM; exec tail -n +1 -f --pid="$lab_pid" "$log" 2>/dev/null) &
      tail_pid=$!

      started=$SECONDS
      next_note=30
      announced=0
      stop_at=""
      forced=0
      on_signal() {
        if [ -z "$stop_at" ]; then
          stop_at=$SECONDS
          request_stop "$work"
          echo
          echo "spc-lab-quickstart: stopping the lab cleanly; this takes a few seconds, please wait"
          if [ "$announced" -eq 0 ]; then
            echo "spc-lab-quickstart: (it is still starting and stops as soon as startup has finished)"
          fi
        else
          echo
          echo "spc-lab-quickstart: still stopping ($((SECONDS - stop_at)) s so far)"
        fi
      }
      # shellcheck disable=SC2329  # reached through the EXIT trap only
      on_exit() {
        if kill -0 "$lab_pid" 2>/dev/null; then
          echo "spc-lab-quickstart: the lab is still running; stop it with: $self stop --name $name" >&2
        fi
      }
      trap on_signal INT TERM HUP
      trap on_exit EXIT
      trap "" TTIN

      # From here a command interrupted by Ctrl-C must not end the launcher.
      set +o errexit
      read_stdin=1
      while kill -0 "$lab_pid" 2>/dev/null; do
        elapsed=$((SECONDS - started))
        if [ "$announced" -eq 0 ] && grep -q '^spc-lab-runtime: http=' "$log"; then
          announced=1
          if [ -z "$stop_at" ]; then
            sleep 1
            describe "$work"
            if [ -t 0 ]; then
              echo "  Here: press Ctrl-C once, or type stop and Enter, then wait."
              echo
            fi
          fi
        elif [ "$announced" -eq 0 ] && [ -z "$stop_at" ] && [ "$elapsed" -ge "$next_note" ]; then
          echo "spc-lab-quickstart: still starting ($elapsed s)"
          next_note=$((next_note + 30))
        fi
        if [ -n "$stop_at" ] && [ "$forced" -eq 0 ] && [ "$announced" -eq 1 ] \
          && [ $((SECONDS - stop_at)) -ge 120 ]; then
          # The lab's own processes are bound to the controller's lifetime, so
          # they end with it; the cleanup below then confirms each one.
          forced=1
          echo "spc-lab-quickstart: no clean stop after 120 s; ending the controller" >&2
          kill -KILL "$lab_pid" 2>/dev/null
        fi
        if [ "$read_stdin" -eq 1 ]; then
          line=""
          read -r -t 1 line
          status=$?
          if [ "$status" -eq 0 ]; then
            if [ "$(echo "$line" | tr -d '[:space:]')" = stop ]; then on_signal; fi
          elif [ "$status" -le 128 ]; then
            # End of input (no terminal, or a detached unit): just wait.
            read_stdin=0
          fi
        else
          sleep 1
        fi
      done

      wait "$lab_pid" 2>/dev/null
      status=$?
      wait "$tail_pid" 2>/dev/null
      rm -f "$work/control"
      echo
      if [ "$status" -eq 0 ]; then
        echo "spc-lab-quickstart: the lab stopped cleanly; nothing is left running"
      else
        if [ "$announced" -eq 0 ]; then
          echo "spc-lab-quickstart: the lab did not start (status $status); its error is printed above" >&2
          echo "spc-lab-quickstart: starting again is safe, every start uses a new directory" >&2
        elif [ "$forced" -eq 1 ]; then
          echo "spc-lab-quickstart: the lab was ended without a clean stop (status $status)" >&2
        else
          echo "spc-lab-quickstart: the lab controller ended unexpectedly (status $status)" >&2
        fi
        echo "spc-lab-quickstart: ending whatever the run left behind" >&2
        if ${lab}/bin/spc-lab reap --run-dir "$run" > "$work/reap.log" 2>&1; then
          echo "spc-lab-quickstart: done; nothing is left running (details in $work/reap.log)" >&2
        else
          echo "spc-lab-quickstart: cleanup failed, see $work/reap.log; retry with: $self reap --name $name" >&2
        fi
        echo "spc-lab-quickstart: process logs are in $run/logs" >&2
      fi
      echo "spc-lab-quickstart: run kept in $work (delete it when you no longer need it)"
      exit "$status"
    fi

    # ------------------------------------------------- commands on a run
    [ -d "$home" ] || die "no runs under $home"
    home="$(cd "$home" && pwd -P)"
    if [ -z "$name" ]; then
      name="$(readlink "$home/latest" 2>/dev/null)" \
        || die "no most recent run recorded under $home; pass --name"
    fi
    work="$home/$name"
    run="$work/runtime"
    [ -d "$work" ] || die "no run at $work"

    case "$subcommand" in
      grafana)
        controller_pid "$work" > /dev/null \
          || die "run $name is not running; start a lab first"
        exec ${observability}/bin/spc-lab-observability --run-dir "$run" "''${pass[@]}"
        ;;
      status)
        if pid="$(controller_pid "$work")"; then
          echo "spc-lab-quickstart: run $name is running (controller pid $pid)"
          describe "$work"
        else
          echo "spc-lab-quickstart: run $name is not running; its files are in $work"
        fi
        ;;
      stop)
        pid="$(controller_pid "$work")" \
          || die "run $name has no live controller; if processes are left, run: $self reap --name $name"
        request_stop "$work"
        echo "spc-lab-quickstart: asked run $name to stop; waiting for controller pid $pid"
        waited=0
        while kill -0 "$pid" 2>/dev/null; do
          if [ "$waited" -ge 300 ]; then
            die "the controller is still running after 300 s (a stop only takes effect once startup has finished); see $work/controller.log"
          fi
          sleep 1
          waited=$((waited + 1))
        done
        echo "spc-lab-quickstart: run $name stopped"
        ;;
      reap)
        if pid="$(controller_pid "$work")"; then
          die "run $name is still running (controller pid $pid); use: $self stop --name $name"
        fi
        exec ${lab}/bin/spc-lab reap --run-dir "$run"
        ;;
    esac
  '';
}
