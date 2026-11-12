#!/usr/bin/env python3
"""Tiny Unix socket client for the LP-4a link-fault backend.

Usage: lab_sock.py <socket-path> <json-request>

Prints the JSON response on stdout.  Exit code 0 when the response has
"ok": true, 1 otherwise, 2 on a transport error.  Both the request and the
response are bounded so a stuck or hostile peer cannot exhaust memory.
"""

import json
import socket
import sys

# The backend caps each captured stream at 256 KiB and the request at 1 MiB.
# JSON escaping can expand one byte into six characters, so the response cap
# is sized for the worst case of two capped streams plus the envelope.
MAX_REQUEST_BYTES = 1024 * 1024
MAX_RESPONSE_BYTES = 8 * 1024 * 1024


def main() -> int:
    if len(sys.argv) != 3:
        sys.stderr.write("usage: lab_sock.py <socket-path> <json-request>\n")
        return 2
    path, request = sys.argv[1], sys.argv[2]
    if len(request.encode("utf-8")) > MAX_REQUEST_BYTES:
        sys.stderr.write("lab_sock: request too large\n")
        return 2
    try:
        sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        sock.settimeout(180)
        sock.connect(path)
        sock.sendall(request.encode("utf-8") + b"\n")
        data = b""
        while b"\n" not in data:
            chunk = sock.recv(65536)
            if not chunk:
                break
            data += chunk
            if len(data) > MAX_RESPONSE_BYTES:
                sys.stderr.write("lab_sock: response too large\n")
                sock.close()
                return 2
        sock.close()
    except OSError as exc:
        sys.stderr.write("lab_sock: %s\n" % exc)
        return 2
    line = data.split(b"\n", 1)[0]
    if not line:
        sys.stderr.write("lab_sock: empty response\n")
        return 2
    sys.stdout.write(line.decode("utf-8", "replace") + "\n")
    try:
        parsed = json.loads(line.decode("utf-8", "replace"))
    except json.JSONDecodeError:
        return 2
    return 0 if parsed.get("ok") else 1


if __name__ == "__main__":
    sys.exit(main())
