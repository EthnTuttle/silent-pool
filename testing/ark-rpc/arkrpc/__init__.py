"""AP-1a safe read-only node capability probe (ark-rpc).

Read-only JSON-RPC capability probe that checks an rbitcoin regtest node
against the RPC surface and response shapes the pinned bark (c419159)
server and wallet actually parse. No node code is modified, no Core
version is faked, and no mutating path is exercised.

This package is Python 3 stdlib only. See README.md for exact commands.

Absorption hook: the future Rust `node_probe` (crates/spc-lab) must
consume the same JSON contract set exported by
`arkrpc.contracts.canonical_contract_json()`; that JSON and its sha256
(recorded in every run manifest) are the frozen interface between this
probe and the AP-1 integration.
"""

__version__ = "1.0.0"

PROBE_ID = "ap1a-ark-rpc-probe"
