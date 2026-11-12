#!/usr/bin/env python3
"""Launcher for the AP-1a ark-rpc probe.

Usage:
    ./probe.py --help
    ./probe.py node-probe --binary <rbitcoin-node> --label <name> --out <dir>
    ./probe.py probe --rpc-url http://127.0.0.1:PORT --cookie-file <path> --out <dir>
    ./probe.py self-test
    ./probe.py contract-show
    ./probe.py compare --reports a/report.json b/report.json --out gap.md

Python 3 stdlib only.
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from arkrpc.cli import main  # noqa: E402

if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
