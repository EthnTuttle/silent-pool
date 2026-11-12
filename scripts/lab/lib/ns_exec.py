#!/usr/bin/env python3
"""Spawn helper: start a process and report whether the exec succeeded.

The backend must not fork from a threaded controller and must not lose an
exec failure.  This helper is a fresh, single threaded process: it forks once,
reports the exec status synchronously through a close-on-exec error pipe, and
then stays alive as the parent of the spawned process so that:

* the caller learns the real errno when the binary does not exist;
* `PR_SET_PDEATHSIG` chains the lifetime of the spawned process to this
  helper and to the supervisor, so no fixture survives a dead backend.

Usage:
    ns_exec.py --log PATH --status-fd N -- CMD [ARG...]

The status line is written to the status file descriptor as one JSON object:
    {"exec_ok": true, "pid": 1234}
    {"exec_ok": false, "errno": 2, "error": "No such file or directory"}
"""

from __future__ import annotations

import argparse
import ctypes
import json
import os
import signal
import sys

PR_SET_PDEATHSIG = 1
_libc = ctypes.CDLL("libc.so.6", use_errno=True)


def set_pdeathsig() -> None:
    try:
        _libc.prctl(PR_SET_PDEATHSIG, signal.SIGKILL, 0, 0, 0)
    except Exception:
        pass


def main() -> int:
    parser = argparse.ArgumentParser(description="spawn helper with an exec status channel")
    parser.add_argument("--log", default="")
    parser.add_argument("--status-fd", type=int, default=1)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()

    argv = args.command
    if argv and argv[0] == "--":
        argv = argv[1:]
    if not argv:
        sys.stderr.write("ns_exec: no command given\n")
        return 2

    # This helper dies with the supervisor; the spawned process dies with it.
    set_pdeathsig()

    # The error pipe stays close-on-exec (the default for os.pipe): a
    # successful exec closes the write end and the parent sees EOF, while an
    # exec failure writes the errno before the child exits.
    err_r, err_w = os.pipe()

    pid = os.fork()
    if pid == 0:
        try:
            os.close(err_r)
            if args.log:
                log = os.open(args.log, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o600)
                os.dup2(log, 1)
                os.dup2(log, 2)
                if log > 2:
                    os.close(log)
            set_pdeathsig()
            os.execvp(argv[0], argv)
        except OSError as exc:
            # The real errno comes from the failed exec, not from
            # ctypes.get_errno(), which os.execvp never sets.
            err = exc.errno or 127
            try:
                os.write(err_w, json.dumps({"errno": err, "error": exc.strerror or str(exc)}).encode())
            except OSError:
                pass
            os._exit(127)
        except BaseException as exc:
            err = 127
            try:
                os.write(err_w, json.dumps({"errno": err, "error": str(exc)}).encode())
            except OSError:
                pass
            os._exit(127)

    os.close(err_w)
    payload = b""
    try:
        while True:
            chunk = os.read(err_r, 4096)
            if not chunk:
                break
            payload += chunk
    except OSError:
        pass
    os.close(err_r)

    status_fd = args.status_fd
    if payload:
        try:
            detail = json.loads(payload.decode("utf-8", "replace"))
        except json.JSONDecodeError:
            detail = {"errno": 127, "error": "exec failed"}
        status = {"exec_ok": False, "pid": pid, **detail}
        try:
            os.write(status_fd, (json.dumps(status, sort_keys=True) + "\n").encode())
        except OSError:
            pass
        try:
            os.kill(pid, signal.SIGKILL)
        except OSError:
            pass
        try:
            os.waitpid(pid, 0)
        except OSError:
            pass
        return 127

    status = {"exec_ok": True, "pid": pid}
    try:
        os.write(status_fd, (json.dumps(status, sort_keys=True) + "\n").encode())
    except OSError:
        pass

    # Forward termination to the child and wait for it.
    def _forward(signum, frame):
        try:
            os.kill(pid, signum)
        except OSError:
            pass

    signal.signal(signal.SIGTERM, _forward)
    signal.signal(signal.SIGINT, _forward)
    try:
        _, wstatus = os.waitpid(pid, 0)
    except OSError:
        return 0
    return os.waitstatus_to_exitcode(wstatus) if wstatus is not None else 0


if __name__ == "__main__":
    sys.exit(main())
