# Native wallet audit service scaffold

Date: 2026-11-10
Status: additive source scaffolding; codec/auditor/runtime contracts pending

## Preserved baseline

Verified all 708 rows, file hashes, sizes and modes from the aggregate R3 freeze:
full `dde900a10546b883acc2940b66260cc9135eceb7085991a22d643a2aa63c23d8`, code
`3c79436675f475dfef6797019612ef626e4a95e77fc3cdd5afed9031c9ddd4f0`.
The source-only archive is
`/tmp/opencode/aggregate-wallet-r3-preserved-source.tar.gz`, SHA256
`6cda0a9d2b0542810ad50e733412fffbbee96afd4c710098bf4d858b292bfea1`.
It contains 11,358,471 source bytes, compressed to 3,210,108 bytes, without Git,
targets, runtime datadirs or caches. Existing R3 receipts/interfaces remain frozen.

## Authored independent scaffold

`wallet/native_audit_service.rs` supplies the bounded Unix execution shell in
the existing agent wallet crate. It runs one current-thread Tokio executor on
one dedicated thread inside the wallet process. A synchronous bounded source
verification step cannot occupy the main wallet command/maintenance executor.
Admission uses kernel Unix credentials and the owned endpoint UID before any
protocol work. The active-session limit is 1-4, overload closes immediately,
and one deadline (at most 10 seconds) covers admission, preparation, revalidation,
completion and response write. Shutdown cancels outstanding async sessions.

The required injected interfaces are `EndpointLease`, `PeerAuthorizer` and
`ProtocolHandler`. There is no permissive default. The actual runtime adapter
must supply endpoint ownership/inode/placement checks and PID/start-token,
run/miner/identity/connection, executable and config-scope authorization.
Those production adapters are not implemented by the scaffold. The handler
must use the owner's native codec and shared UnixSourceAuditor, then recheck
newest-candidate/expiry/session rules before activation signing. The shell
revalidates peer/endpoint before completion and again before releasing bytes.

No request/response fields, binary tag, framing, ACK exchange or cryptographic
verification were duplicated. The service accepts an already leased listener;
it does not invent a collector path validator or filesystem socket lifecycle.
Its API takes no wallet store, Bark wallet, maintenance operation lock or key.
No scan secret was cloned or exported, and no CPU config/event was changed.
The module is deliberately not called by `run_from_config` until the exact
typed/native and loaded-key-auditor contracts are available. Consequently there
is no functioning native activation endpoint or success response in this packet.

## Contract requests and runtime join

Exact early coordination is in
`/tmp/opencode/aggregate-wallet-native-service-needs.json`.

1. Native owner `ses_e4220291effeJCDinxPtFrpAvy`: actual
   `spc_scan::native_activation` types, frame bounds/helpers, canonical digests,
   activation domain tag, expiry/error rules and AddConnection public fields.
   Include a wallet-instance/restart correlation mechanism (or agreed durable
   replay state) so restart does not create a replay allowance. No alternate
   wire is defined here.
2. Ladder owner: public loaded-B_scan constructor and `verify_received`-style
   API on the SAME `UnixSourceAuditor`. It must verify actual ReceivedJob,
   SourceBundle, full owned payout/Rule B/history/checkpoint, sign Consumed and
   wait for verified collector durable Committed before returning exact evidence.
   Existing translator behavior delegates to it. Its owned scan clone must be
   zeroized on success, failure and cancellation. Warming is allowed only as an
   explicit audited result from that helper.
3. Runtime/native owner: finalize optional `ark.native_audit` configuration and
   the `wallet.<identity>.native_audit` Unix lease. Proposed fixed socket is
   `<ark.datadir>/native-audit.sock`; actual collector/pool key bindings remain
   wallet-side. Wallets start before CPUs, so the controller needs to publish
   the actual current peer/config/connection authorization before AddConnection,
   with a generation update on removal or scope change. Request frames cannot
   choose file/RPC paths. Attach validates current processes, executable/scope
   and socket ownership without reprovisioning. Current host-local placement
   is explicit; foreign/unsupported namespaces stay unavailable.

The aggregate API/watch keys, private commands, balance ledger and public UI
observations remain their current interfaces. This additive service does not
add a fifth wallet, another seed, a translator, an SV1 gateway or mining budget.
The native builder owner must enable the mandatory gate for the final scenario.

## Authored tests and remaining work

Transport-only test fixtures cover foreign kernel UID/PID admission, denied
history/scope results, endpoint revocation, bounded overload, total admission
deadline, response bounds and shutdown. A current-thread wallet test holds a
scripted verifier on the audit thread while the REAL existing private command
queue and SQLite maintenance reservation make progress before releasing it.
Mock authorizers stand in for the future runtime policy; probe bytes are not
native frames or cryptographic evidence. Full foreign run/identity/config,
start-token/restart, superseded nonce/signature, shared-auditor/mock-collector
integration and real native-feed tests await the actual handoffs.

No Cargo build, Rust test, live process/socket fixture or heavy-lock attempt
was run. The other owner's priority remains in force. Future checks use
`CARGO_INCREMENTAL=0`, the existing T1a target and nonblocking 50/20-GiB guard.
This source-only scaffold is not native gate readiness or funded acceptance.
