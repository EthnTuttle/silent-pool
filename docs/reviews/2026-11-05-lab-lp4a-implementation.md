# LP-4a: network-namespace link-fault precursor

Date: 2026-11-05
Author: TFF implementation agent
Status: revised after the first and the second Fable review (REQUEST_CHANGES twice), awaiting the third review
Worktree: `/home/ethan/.local/share/opencode/worktree/ad8886/lab-link-preflight`
Base: `b0ab4ca` (silent-pool packet base `b0ab4ca34fda07c61a6335d05f598eaacc69678c`)
Contract: `docs/REGTEST-LAB-IMPLEMENTATION.md` §8.3 (approved L3 snapshot `3edcd4e7`), LP-4 packet
Reviews: `docs/reviews/2026-11-05-lab-lp4a-fable-review.md`
(round 1 REQUEST_CHANGES, snapshot `d004a6df`; round 2 REQUEST_CHANGES bounded,
snapshot `a9813527`; round 1 F1-F9 independently re-verified as fixed)

This packet is the LP-4 *precursor*: it implements and proves the network
primitives the LP-4 link-fault backend needs (network namespaces, veth pairs,
nftables drop rules, `tc netem`, capability preflight, resource ownership and
teardown). It does **not** implement the LP-4 integration: no full node, no
Ark stub, no mining or reorg scenario is started here. Those remain LP-4
integration gates and are listed in section 10.

Nothing in this packet was copied from or applied to the LP-3 runner
candidate. The LP-3 `crates/spc-lab/src/links.rs` and `scenario.rs` were read
only, to produce the merge notes in section 9.

---

## 1. Fable finding dispositions (F1-F9)

All four blockers and all five should-fix items are addressed. Each item below
names the change, the regression that covers it and the actual result.

### F1 (blocking) - IPv6 link-local traffic bypassed the partition

**Cause.** The partition rules matched `ip daddr`/`ip saddr` only, and every
lab veth receives a kernel generated `fe80::/64` address plus a matching
route, so a dual-stack listener stayed reachable across a "partitioned" link.
The no-bypass check listed only IPv4 routes, probed only `AF_INET` and never
looked at forwarding.

**Fix.**

* The drop is now interface scoped and family agnostic. The rules are
  `oifname "<iface>" counter drop` in the `output` chain and
  `iifname "<iface>" counter drop` in the `input` chain of an `inet` table.
  The `inet` family covers IPv4 and IPv6, and the veth *is* the link, so every
  packet that could cross the link is dropped in both directions, including
  IPv6 link-local traffic and the neighbour discovery it needs.
* IPv6 is kept enabled on the lab interfaces, on purpose: the bypass path
  stays real and the regression stays meaningful. `accept_dad=0` and
  `accept_ra=0` are set per interface inside the owned namespace, so the
  link-local address is immediately usable instead of `tentative` and no
  router advertisement can add a route.
* Forwarding is disabled inside every lab namespace at creation:
  `net.ipv4.ip_forward=0`, `net.ipv6.conf.all.forwarding=0`. These are
  network namespace scoped; the host values are untouched (verified: the host
  still reports `ip_forward=1` after a run).
* The no-bypass check is now IPv4 **and** IPv6 aware: it lists `ip -4 route`
  and `ip -6 route`, probes every non-link address of every other endpoint
  over `AF_INET` and `AF_INET6` (including the `fe80::` link-local
  addresses), probes every host address of both families, and asserts
  forwarding is off in every namespace.
* Reachability criterion, per family:
  * IPv4: unreachable only for `ENETUNREACH`/`EHOSTUNREACH`/`EADDRNOTAVAIL`.
    `ECONNREFUSED` or a timeout is a **bypass**.
  * IPv6 link-local: unreachable for the same errnos plus `ETIMEDOUT`,
    because the neighbour discovery request is dropped as well.
    `ECONNREFUSED` (something answered) is still a **bypass**.
  * Cross-check: the address of a **linked** neighbour must be *reachable*
    from that neighbour (it answers with `ECONNREFUSED` when nothing
    listens). A link address that is not reachable means the link itself is
    broken and is reported as a violation.
* The guarantees are now scoped to what the rules really do:
  `guarantee_scope.sustained_partition` = "all traffic on the link interface,
  IPv4 and IPv6, both directions, until the fault is healed".

**Regressions** (`T05b`, `T05c`, `T06`): the IPv6 link-local dial succeeds
before the partition and times out during it, an already established
bidirectional session is cut by the fault, and the no-bypass check reports
`24 non-link probes (12 IPv6) refused, 40 host probes refused, forwarding off
in 4 namespaces, 0 violations`.

### F2 (blocking) - a second `up` destroyed the live run

**Fix.**

* The CLI refuses a run directory that already holds a live backend: it
  reads the recorded supervisor PID and its `/proc/<pid>/stat` field 22
  start-time token, and if the pair still matches, it returns
  `{"ok":false,"error":{"code":"run_dir_in_use", ...}}` with exit code 1 and
  touches nothing. A *stale* run directory (the recorded supervisor is gone)
  may be replaced and the response says `replaced_stale_run: true`.
* The backend takes an exclusive `flock` on `<run-dir>/.backend.lock`
  **before** it creates a namespace or writes a manifest, and it holds that
  lock for its whole lifetime. A second backend on the same directory fails
  with `run_dir_in_use` and writes `up-error.json` instead of clobbering
  anything.
* The control socket is bound **before** the namespaces are created, and the
  backend only ever unlinks a socket path it created itself.
* `--run-id` is honoured: it is passed to the backend and published as
  `run_id` in the manifest.

**Regression** (`T12`): the second `up` is refused with `run_dir_in_use`
(exit 1) and `status` on the original run still answers.

### F3 (blocking) - reapplying `partition` opened the link

**Fix.**

* The whole partition ruleset is installed as **one** `nft -f -` transaction.
  `nft` sends a file as a single netlink batch, so the kernel applies it
  atomically: a reapply is `delete table` + `add table` + chains + rules in
  one batch and there is no window in which the link is open. The `delete`
  is only included when the table is known to exist (a successful
  `nft -j list table` query decides that), so the transaction never fails on
  a missing table.
* The heal path checks its own result: `nft delete table` must exit 0, the
  table is then re-queried and must be absent, and `tc qdisc show` must
  report no netem qdisc. Any deviation is `policy_apply_failed`, never a
  false `ok`.
* Every applied policy is verified after it is installed (the partition
  table exists with both rules; the netem qdisc is present with the expected
  options).

**Regression** (`T11`): 150 reconnect attempts during 10 reapplies, 0
succeeded. Fable's counterexample (4 of 300 connects succeeding) is not
reproducible any more.

### F4 (blocking, small) - the preflight contradicted itself

**Fix.** The three relay-only `inconclusive` entries
(`sustained_partition_assertion`, `no_bypass_assertion`,
`os_enforced_isolation`) are emitted **only** when the selected backend is not
`netns`. The namespace backend's `unsupported_requests` now contains only
the three LP-4 integration gates. The preflight also emits
`guarantee_scope` for each guarantee, refuses to assemble an artifact that
would contradict itself (it exits 3 with
`preflight_assembly_failed`), and the test driver asserts that
`guarantees` and `unsupported_requests[].request` are disjoint.

**Regression** (`T01`): `guarantees_and_unsupported_are_disjoint`.

### F5 (should fix) - a long `exec` blocked the control plane

**Fix.**

* Requests are served by bounded worker threads (8 at a time, `busy`
  otherwise), so a long `exec` no longer delays `apply`, `heal` or `down`.
  Every response carries `queue_wait_ms`.
* Per link mutations are serialized with a per-link lock, so two concurrent
  `apply` calls for the same link cannot interleave; different links proceed
  in parallel.
* **The controller never forks from a request thread and never changes its
  own namespace.** Every command that must run inside an endpoint namespace
  is an external `nsenter --net` process, which is a fresh single threaded
  process: the namespace change cannot leak between threads. The namespace
  holders are still created with `unshare(CLONE_NEWNET)` in a forked child,
  but only during setup, before any thread exists.
* **A real bug this exposed:** `PR_SET_PDEATHSIG` is delivered when the
  *thread* that created a child exits, not when the process exits. Fixtures
  spawned from a request worker thread were therefore killed as soon as the
  request completed. All spawns now happen in one dedicated, long lived
  spawner thread, so a fixture lives exactly as long as the supervisor and
  still dies with it.

**Regression** (`T13`): `apply` completed in 190 ms while an `exec sleep 5`
was running.

### F6 (should fix) - `spawn` returned `ok: true` for a command that never started

**Fix.** The spawn goes through `scripts/lab/lib/ns_exec.py`, a fresh single
threaded helper that forks once and reports the exec result over a
close-on-exec error pipe. The backend reads that status synchronously, so a
missing binary returns `spawn_failed` with the errno instead of a false `ok`,
and the failed child is reaped. `status` reports `alive` and the `/proc`
state of every child.

**Regression** (`T14`): `spawning a missing binary returned spawn_failed
instead of a false ok`.

### F7 (should fix) - tool failures degraded to empty or zero

**Fix.**

* In the no-bypass check, a failed `ip`/`nft`/`tc` query is a violation of
  kind `evidence_unavailable`, not an empty result that passes vacuously.
* `counters.summary` carries `complete` and an `errors` list. A *missing*
  nft table is not an error: it is the normal state when no fault is
  applied, and it is reported as `nft_absent: true` on the side. Any other
  tool failure is an error and makes `complete` false.
* The test driver treats `complete: false` as a failed assertion.

**Regression** (`T15`, and `T05`/`T10` require `complete: true`).

### F8 (should fix) - the lifecycle budget was invisible

**Fix.** The manifest publishes `idle_timeout_s`, `max_lifetime_s`,
`started_at_unix_ms`, `idle_deadline_unix_ms`,
`lifetime_deadline_unix_ms`, `kill_on_expiry: true`,
`zero_disables_the_budget: true` and `leave_for_adoption`. A live spawned
child counts as activity, so "idle" means *no requests and no children*.
`up` echoes the effective values through the manifest, and `--idle-timeout-s`
/ `--max-lifetime-s` let a caller size the budget for a planned run.

**Regression** (`T16`).

### F9 (minor) - input handling and hygiene

* **argv with newlines.** The CLI builds the argv array with
  `jq -n '$ARGS.positional' --args -- "${EXTRA_ARGV[@]}"`, so an argument
  that contains a newline survives the trip into the namespace
  (`T17`).
* **`timeout_s` is bounded** to `(0, 3600]`; `0`, a negative value, `inf` and
  `100000` are refused with `invalid_request` (`T18`).
* **Bounded capture.** The backend captures at most 1 MiB per stream and
  reports `stdout_truncated`/`stderr_truncated`; the request line is capped
  at 1 MiB; the socket client caps the request and the response.
* **Permissions.** The run directory is created `0700`, and the socket, the
  manifest, the topology copy and the lock file are `0600` from creation
  (`umask 077` in the backend).
* **`namespace_gone`.** `exec`, `spawn` and the connect probe all check that
  the holder is still alive before they use it.
* **Process groups.** A timed out command is killed with `killpg` on its own
  session, not with a bare `kill`.
* **Fixture leaks.** The teardown test now checks the recorded child PIDs
  directly, not only `pgrep -f <token>` (the fixtures carry the token in
  their environment, not in their argv).
* **The relay fails closed.** A missing or corrupt relay state file now means
  `down`, not `up` (`R05`).

---

## 1b. Second review dispositions (N1-N3 and the low items)

Round 2 confirmed the first round's F1-F9 as fixed (G1-G4 re-tests) and found
three new items. All are fixed here.

### N1 (blocking) - a stale run directory reported `ok: true` for a backend that never started

**Cause.** The CLI treated a stale directory (the recorded supervisor is gone)
as replaceable, but it removed nothing: the old `manifest.json` and
`control.sock` were still there, the wait returned immediately on the *old*
manifest, and the new backend then failed to bind the socket. The result was
a success that pointed at a dead manifest.

**Fix (the "refuse, do not adopt" option).**

* A run directory that is **not empty** is never reused. The CLI inspects it:
  * if the recorded supervisor PID still matches its start-time token, the
    directory is owned by a live backend -> `run_dir_in_use` (exit 1);
  * if the supervisor is gone, the directory is stale -> `stale_run_dir`
    (exit 1) with `fresh_dir_required: true`, the list of the files that are
    still there and the dead supervisor's PID and token.
* **Nothing is ever unlinked by the CLI.** The diagnostics of the dead run
  (manifest, socket, log, lock) stay exactly where they are, so the crash can
  be inspected.
* A stale manifest can no longer satisfy a startup even by accident: after
  the wait the CLI compares the manifest's `run_token` with the token it
  generated and fails with `stale_manifest` if they differ.
* The startup is only successful after a **live control handshake**: the new
  backend must answer a `ping` on its control socket. A backend that did not
  start can therefore never produce `ok: true`.
* The backend keeps its own safe path for the case where it is started
  directly (bypassing the CLI): it takes the exclusive `flock` on
  `.backend.lock` **first**, so no live owner can exist, and a socket file
  that is still there is proven stale (it does not answer) and is replaced.
  A socket that still answers is refused. Verified: a backend started
  directly on a crashed run directory wrote a **new** manifest with a new
  token and answered `status`.

**Regression** (`T12b`): the supervisor is SIGKILLed, the same directory is
refused with `stale_run_dir` (`fresh_dir_required: true`, nothing deleted),
and a **fresh** directory starts a new backend with a new token whose
`status` answers.

### N2 (blocking, small) - `--leave-for-adoption` was published but not implemented

**Fix.** The flag is **rejected**: `--leave-for-adoption` now fails with
`not_implemented` (exit 1) and explains that this backend always ties the
namespace holders and the fixtures to the supervisor lifetime. The flag was
removed from the backend as well, so no intent can be published that the
process does not enforce.

The manifest now publishes the truth:

```json
"limits": {"leave_for_adoption": false, "adoption_model": "pdeathsig",
           "supports_survive_supervisor": false},
"lifetime_semantics": {
  "graceful_stop": "`down` terminates the fixtures and the namespace holders, reaps them and removes the control socket",
  "supervisor_death": "the namespace holders and the spawned fixtures carry PR_SET_PDEATHSIG=SIGKILL, so they die with the supervisor; nothing is left for adoption",
  "supports_survive_supervisor": false,
  "adoption_model": "pdeathsig",
  "note": "surviving the supervisor is not implemented and is not claimed; reconnecting to a live backend is a different thing and is supported by the control socket"}
```

Reconnecting to a *live* backend is a different property and is supported:
the control socket answers while the backend lives. Surviving the death of
the backend is not supported and is not claimed.

### N3 (should fix) - the IPv6 half of the route check could not flag a route

**Cause.** The IPv6 branch tested `not (dst or "").startswith("fe80:") and
not dst`, which is only true for an empty destination, so a global IPv6 route
was never flagged.

**Fix.** The rule is now the mirror image of the IPv4 rule and compares the
*actual* owned subnets and interfaces:

* IPv4: the route network must be one of the connected `/30` subnets of the
  interfaces the endpoint owns.
* IPv6: the **only** routes a lab namespace may carry are the kernel
  generated `fe80::/64` routes of the interfaces it owns. Any other route -
  global, ULA, multicast (`ff00::/8`) or a documentation prefix such as
  `2001:db8::/64` - is a bypass path and is reported as
  `unattached_route`. The `dev` of a link-local route must also be one of the
  endpoint's own interfaces.

**Regression** (`T06b`): inside the owned `alpha` namespace a
`2001:db8::/64` route and a `10.99.0.0/24` route are injected; the check
reports **both** (`v6=1 v4=1`). `T06c` removes them again and the check
reports 0 violations, so the owned namespace is back to its normal routes
before the teardown.

### N4 (low) - truncated tool output was treated as complete evidence

**Fix.** Every evidence path now treats a truncated tool output as a tool
error: `_routes`, `_addresses`, `_table_exists`, `_verify_partition`,
`_verify_qdisc`, the ruleset listing of the no-bypass check and the
`counters` sides. A truncated output yields `evidence_unavailable` or
`complete: false`, never an empty list that passes vacuously.

The capture cap is now 256 KiB per stream (instead of 1 MiB) and the socket
client accepts up to 8 MiB, so the JSON escaping of a capped payload always
fits: a caller now sees the data **and** `stdout_truncated: true` instead of
an unreadable "response too large".

**Regression** (`T15b`): a 2 MB output is capped at 262144 bytes and
`stdout_truncated` is true.

### N5 (low) - a partial apply left an unrecorded half state

**Fix.** A failed `apply`/`heal` now records the real state:
`link.policy` becomes `{"type": "partial", "requested": <policy>,
"failed_side": <endpoint>, "error": <detail>}` and the manifest is written
before the error is raised. The recorded state can no longer disagree with
the installed rules.

### N6 (low) - the no-bypass check reported an intentional fault as a violation

**Fix.** The check knows the current policy of every link. The adjacency
cross-check ("the linked neighbour must be reachable") only applies to a link
whose policy is `normal`. A link that is intentionally faulted is reported as
`link_faulted` (informational) and the response carries `faulted_links`, and
the `criterion` string states that the cross-check only applies to healthy
links.

**Regression** (`T06d`): during an intentional partition the check reports
`["alpha-beta"]` as faulted and raises no `link_address_unreachable`
violation.

### N7 (low) - control plane saturation and hygiene

* **Admission pools.** Long running commands (`exec`, `spawn`) and control
  operations (`apply`, `heal`, `status`, `counters`, `no-bypass-check`,
  `manifest`) now have separate admission pools (8 and 4). Eight long execs
  can no longer starve `status`/`apply`/`down`.
* **Accurate exec errno.** `ns_exec.py` catches the `OSError` of the failed
  exec and reports `exc.errno` (and its `strerror`) instead of the
  meaningless `ctypes.get_errno()`.
* **Manifest write.** `write_manifest()` takes the state lock and writes to a
  **unique** temp file (`manifest.json.tmp.<pid>.<tid>`) before the atomic
  `os.replace`, so two writers can never share a temp path.
* **Spawn bookkeeping.** The wrapper's start-time token is now recorded, so
  the wrapper branch of `terminate_children` is live, and the wrapper's
  stdout/stderr pipes are closed after the status line is read.
* **Worker bookkeeping.** Finished worker threads are pruned from the worker
  list.
* **Error file naming.** A backend that fails to start writes
  `up-error-<run-token>.json`, so it cannot be mistaken for the error of the
  live run in the same directory.

**Regression** (`T13b`): `status` answered in 91 ms while 8 long execs were
running.

---

## 2. Delivered files (all new, all uncommitted)

| Path | Lines | Purpose |
|---|---|---|
| `scripts/lab/netns-preflight.sh` | 256 | Capability probe. Emits `spc.lab.netns_preflight.v1` JSON. |
| `scripts/lab/link-fault.sh` | 373 | Link-fault CLI (`up`, `apply`, `heal`, `status`, `counters`, `no-bypass-check`, `exec`, `spawn`, `manifest`, `down`). |
| `scripts/lab/lib/lab_common.sh` | 119 | Shared bash helpers (run token, start-time token, bounded waits). |
| `scripts/lab/lib/nsprobe-inner.sh` | 184 | Probe body that runs *inside* the throwaway namespace. |
| `scripts/lab/lib/lab_backend.py` | 1810 | Supervisor: owns the namespace tree, veths, nft tables, qdiscs and fixture processes. |
| `scripts/lab/lib/lab_sock.py` | 62 | Unix-socket JSON transport used by the CLI. |
| `scripts/lab/lib/netns_probe.py` | 83 | Connect probe executed inside an owned namespace. |
| `scripts/lab/lib/ns_exec.py` | 148 | Spawn helper with a close-on-exec error channel. |
| `testing/lab-isolation/run.sh` | 908 | Standalone test driver (netns suite + relay fallback suite). |
| `testing/lab-isolation/tcp_fixture.py` | 371 | Real TCP fixture endpoints (server/client/session/probe, IPv4 and IPv6). Fixtures only: no miner, no PoW, no SV2. |
| `testing/lab-isolation/relay_fixture.py` | 202 | Userspace TCP relay used only by the explicitly weaker fallback backend. |
| `docs/reviews/2026-11-05-lab-lp4a-implementation.md` | this file | Report. |

No shared file was edited: `flake.nix`, `Cargo.toml`, `Cargo.lock`, `justfile`,
`docs/SPEC.md` and `docs/STATUS.md` are untouched. Section 11 lists the merge
notes for them.

---

## 3. What this host actually supports (probed, not assumed)

`scripts/lab/netns-preflight.sh` creates its own `unshare -Urn
--map-root-user` namespace, performs every probe inside it and deletes
everything it created. Verbatim from
`/tmp/opencode/LP4a-frozen/preflight.json`:

```json
{
  "backend": "netns",
  "backend_reason": "unprivileged user+network namespaces, veth, nftables and PID-handle namespace entry all verified",
  "exit_code": 0,
  "netns_backend_supported": true,
  "netem_supported": true,
  "relay_fallback": {"available": true, "reason": "userspace TCP relay fallback is available; it provides a controlled link outage only and cannot enforce isolation"}
}
```

| Capability | Supported | Evidence recorded by the probe |
|---|---|---|
| `user_namespace` | yes | `uid=0 capeff=0x000001ffffffffff cap_sys_admin=1 cap_net_admin=1` |
| `network_namespace` | yes | `fresh namespace contains only loopback (link_count=1)` |
| `veth` | yes | `created veth pair labprobe0/labprobe1 inside the probe namespace` |
| `nftables` | yes | `added table/chain/rule inet labprobe and listed it as JSON` |
| `sch_netem` | yes | `installed root netem qdisc on labprobe0` |
| `sch_netem_module` | yes | `sch_netem is present in /sys/module` |
| `nested_namespace_pid_handle` | yes | `child namespace net:[4026534421] reachable by PID handle; nsenter and setns(CLONE_NEWNET) both succeeded` |
| `ipv6_link_local` | yes | `the kernel assigns fe80::... with 1 fe80::/64 route(s); the partition must cover IPv6` |
| `netns_forwarding` | yes | `inherited net.ipv4.ip_forward=1 net.ipv6.conf.all.forwarding=0; the backend disables both inside every lab namespace` |

Host facts recorded in the same artifact: kernel `6.8.0-138-generic`, uid 1000
(not root), `Seccomp: 0`, `unprivileged_userns_clone=1`,
`max_user_namespaces=126211`, `in_user_namespace=false`, container hint
`none`. Tool versions: `unshare`/`nsenter` util-linux 2.37.2, `ip`/`tc`
iproute2-5.15.0, `nftables` 1.0.2, `jq` 1.6, `python3` 3.10.12, `sysctl`
procps.

Host facts that shaped the design:

* `/run/netns` exists and is **root owned**, so the backend never writes
  there. It uses the PID-handle design (`unshare(CLONE_NEWNET)` in a forked
  holder, entered through `/proc/<holder-pid>/ns/net`), proven here by both
  `nsenter -t <pid> -n` and a direct `setns(CLONE_NEWNET)` syscall.
* `net.ipv4.ip_forward` is **1** on this host and is inherited by a new
  namespace. The backend sets it to 0 inside every lab namespace. This is
  network namespace scoped: the host value is unchanged after a run.
* Every lab veth gets a kernel generated `fe80::/64` address. The backend
  records it (`ipv6_link_local` in the manifest, with the owning link) and
  the partition covers it.
* No `sudo`, no `modprobe`, no `sysctl -w` outside an owned namespace, no
  host interface, route or nftables change is performed anywhere in this
  packet.

Preflight exit codes: `0` = netns backend fully capable, `10` = netns backend
without `sch_netem` (latency/loss unsupported), `20` = relay fallback only,
`21` = no backend.

---

## 4. CLI contract (changes against the reviewed snapshot are marked NEW)

```
scripts/lab/netns-preflight.sh [--out FILE] [--quiet]
```

Exit codes 0/10/20/21 as above. Always prints one JSON object
(`spc.lab.netns_preflight.v1`) on stdout. NEW: `guarantee_scope`,
`probe_subprocess`, and the relay-only `unsupported_requests` entries are
now conditional.

```
scripts/lab/link-fault.sh up              --spec FILE [--run-dir DIR] [--run-id ID]
                                         [--idle-timeout-s S] [--max-lifetime-s S]
                                         (a run directory that is not empty is never
                                          reused: use a fresh directory after a crash)
scripts/lab/link-fault.sh apply           --run-dir DIR --link ID --policy normal|partition|latency:MS|loss:PCT
scripts/lab/link-fault.sh heal            --run-dir DIR --link ID
scripts/lab/link-fault.sh status          --run-dir DIR [--link ID]
scripts/lab/link-fault.sh counters        --run-dir DIR --link ID
scripts/lab/link-fault.sh no-bypass-check --run-dir DIR
scripts/lab/link-fault.sh exec            --run-dir DIR --endpoint NAME [--timeout S] [--raw] -- CMD [ARG...]
scripts/lab/link-fault.sh spawn           --run-dir DIR --endpoint NAME --log FILE --label NAME -- CMD [ARG...]
scripts/lab/link-fault.sh manifest        --run-dir DIR
scripts/lab/link-fault.sh down            --run-dir DIR
```

Every command prints exactly one JSON object on stdout. Exit codes: `0`
success, `1` the backend rejected the request, `2` usage error, `3` the
backend is not reachable. For `exec`, `ok` reports whether the executed
command succeeded and `command_exit_code` carries its status.

NEW response fields: `queue_wait_ms` on every response; `limits` (with the
lifecycle budget) on `status`; `nft_present`/`nft_absent` and
`summary.complete`/`summary.errors` on `counters`.

NEW error codes: `run_dir_in_use` (a live backend owns the run directory),
`stale_run_dir` (the directory is not empty and its backend is gone; a fresh
directory is required, nothing was deleted), `stale_manifest` (the manifest in
the directory belongs to another run), `not_implemented` (a flag that this
backend does not implement, currently only `--leave-for-adoption`),
`spawn_failed` (the command did not start), `invalid_request` (a malformed
or out of range request), `namespace_gone` (the endpoint namespace is gone),
`busy` (too many concurrent requests of that class), `request_too_large`.

`up` now only reports success after the new backend answered a `ping` on its
control socket, and only if the manifest it reads carries the run token it
generated. A stale manifest can therefore never satisfy a startup.

### 4.1 Manifest (`<run-dir>/manifest.json`, schema `spc.lab.link_manifest.v1`)

```json
{
  "run_id": "LP4a-...", "run_token": "c3dc9f4d", "backend": "netns", "isolation": "netns",
  "control_socket": "/tmp/opencode/LP4a-.../run/control.sock",
  "supervisor": {"pid": 2603763, "start_time_token": "...", "argv_contains_token": true},
  "tools": {"nsenter": "/usr/bin/nsenter", "ip": "/usr/sbin/ip", "nft": "/usr/sbin/nft",
            "tc": "/usr/sbin/tc", "sysctl": "/usr/sbin/sysctl", "cat": "/usr/bin/cat"},
  "endpoints": [
    {"name": "alpha", "role": "pool",
     "namespace": {"holder_pid": 2603765, "start_time_token": "...", "netns_path": "/proc/2603765/ns/net", "alive": true},
     "addresses": [{"link_id": "alpha-beta", "addr": "10.250.0.1", "prefix": 30,
                    "iface": "lc3dc9f4d00a", "family": "inet"}],
     "ipv6_link_local": [{"iface": "lc3dc9f4d00a", "addr": "fe80::c072:38ff:feef:9d13", "link_id": "alpha-beta"}],
     "forwarding": {"ipv4": 0, "ipv6": 0}}
  ],
  "links": [
    {"id": "alpha-beta", "from": "alpha", "to": "beta", "subnet": "10.250.0.0/30",
     "addresses": {"alpha": "10.250.0.1", "beta": "10.250.0.2"},
     "interfaces": {"alpha": "lc3dc9f4d00a", "beta": "lc3dc9f4d00b"},
     "nft_tables": {"alpha": "lab4a_c3dc9f4d_00a", "beta": "lab4a_c3dc9f4d_00b"},
     "policy": {"type": "normal"}, "applied_at_unix_ms": null}
  ],
  "children": [{"pid": 2603771, "start_time_token": "...", "wrapper_pid": 2603770,
                "endpoint": "beta", "label": "beta-server", "argv": [...], "alive": true}],
  "host_addresses": ["192.168.1.184", "..."],
  "host_addresses_v6": ["2605:59c8:2840:3308::825", "..."],
  "guarantees": [...],
  "guarantee_scope": {"sustained_partition": "all traffic on the link interface, IPv4 and IPv6, both directions, until the fault is healed"},
  "unsupported_requests": [...],
  "limits": {"max_endpoints": 32, "max_links": 64, "default_command_timeout_s": 30,
             "max_command_timeout_s": 3600, "max_capture_bytes": 1048576,
             "max_request_bytes": 1048576, "max_concurrent_requests": 8,
             "idle_timeout_s": 3600, "max_lifetime_s": 7200, "zero_disables_the_budget": true,
             "kill_on_expiry": true, "leave_for_adoption": false,
             "started_at_unix_ms": 1793944589104, "idle_deadline_unix_ms": 1793948189104,
             "lifetime_deadline_unix_ms": 1793951789104, "address_space": "10.250.0.0/16"}
}
```

`run_id` is the basename of the run directory unless `--run-id` is given.
`run_token` is the 8 hex character ownership token; it appears in every
resource name the run creates.

### 4.2 Fault semantics

* `partition` installs, in **both** endpoint namespaces, one nftables
  transaction that creates a table `lab4a_<token>_<link><side>` with an
  `output` chain rule (`oifname "<iface>" counter drop`) and an `input`
  chain rule (`iifname "<iface>" counter drop`). The drop is silent, so a
  reconnecting client times out instead of being refused: this is the
  sustained-fault behaviour, not a one-shot disconnect. The rules are
  interface scoped and family agnostic, so IPv4 and IPv6 are both covered.
* `latency:MS` applies `tc qdisc replace dev <iface> root netem delay <MS>ms`
  on both sides of the link; the exact qdisc string is returned in
  `evidence.qdisc`.
* `loss:PCT` applies `tc qdisc replace dev <iface> root netem loss <PCT>%` on
  both sides.
* `heal` (or `apply --policy normal`) removes both nft tables and both root
  qdiscs **and verifies the result**. Applying any policy always starts from
  a clean state, and the partition is installed as one atomic transaction.

### 4.3 `no-bypass-check` output

The check is a real reachability test, not a config inspection. For every
ordered endpoint pair it connects from namespace A to every address of B that
is **not** on a link incident to A (IPv4 and IPv6 link-local), and it
connects from every namespace to every host address of both families. It also
asserts that no namespace has a default route, that no route points outside
the attached subnets, that forwarding is off everywhere, and that no
nftables table other than the run's own tables exists in any namespace.

```
ENETUNREACH / EHOSTUNREACH / EADDRNOTAVAIL          -> unreachable, OK
ETIMEDOUT (IPv6 link-local only)                     -> unreachable, OK
ECONNREFUSED or a successful connect                 -> BYPASS, violation
```

`ECONNREFUSED` means the target answered, so the path is real. The check
records the criterion it used in its own output.

---

## 5. Test evidence (actual runs on this host)

Driver: `testing/lab-isolation/run.sh`. Artifacts are written under
`/tmp/opencode/LP4a-<utc>-<rand>/`, including `preflight.json`,
`results.jsonl`, `results.tsv`, `exit-codes.txt`, `summary.json`, the
manifest, all fixture logs and every command response.

### 5.1 Namespace backend

`/tmp/opencode/LP4a-frozen/summary.json` -> `{"backend":"netns","passed":28,"failed":0,"skipped":0}`.
The final code was run twice against the frozen snapshot (the namespace suite
and the relay suite); both exited 0 and left no `lab_backend`, `tcp_fixture`, `relay_fixture` or
`ns_exec` process behind, `/run/netns` stayed empty and the host's
`net.ipv4.ip_forward` stayed `1`. The earlier rounds of the same suite are
`LP4a-revD`, `LP4a-stab1`, `LP4a-stab2` and `LP4a-final` (22/22 each, before
the N1-N7 additions).

| Test | Result | Recorded detail |
|---|---|---|
| T01_preflight | passed | `probe_backend=netns exit=0 guarantees_and_unsupported_are_disjoint` |
| T02_topology | passed | `endpoints=4 links=3` |
| T03_listeners | passed | `beta=10.250.0.2:9101 gamma=10.250.0.6:9102 delta=10.250.0.10:9103 v6=[::]:9301` |
| T04_normal_traffic | passed | `3/3 sessions delivered payload over the alpha-beta link` |
| T05_sustained_partition | passed | `8/8 reconnects failed over 8414ms, listener still bound on 10.250.0.2:9101, nft dropped=8` |
| T05b_ipv6_partition_blocks_linklocal | passed | `the IPv6 link-local dial to fe80::c072:38ff:feef:9d13 timed out during the partition (result=timeout)` |
| T05c_established_session_is_cut | passed | `the session answered before the fault (PONG persist 1 ...) and failed after it (FAIL:TimeoutError:timed out)` |
| T06_no_bypass | passed | `24 non-link probes (12 IPv6) refused, 40 host probes refused, forwarding off in 4 namespaces, 0 violations` |
| T07_unrelated_link_survives | passed | `alpha-gamma still delivered 2/2 sessions while alpha-beta was partitioned` |
| T08_heal_restores_delivery | passed | `2/2 IPv4 sessions and the IPv6 link-local session both delivered after the heal` |
| T09_latency_qdisc | passed | `netem delay 150ms: session delivered in 600ms, healed baseline 0ms` |
| T10_loss_qdisc | passed | `netem loss 30%: qdisc dropped 14 packets, 6/10 sessions failed, heal restored 2/2` |
| T11_atomic_reapply_has_no_open_window | passed | `150 reconnects during 10 reapplies, 0 succeeded` |
| T12_duplicate_up_is_refused | passed | `the second up was refused (run_dir_in_use) and the original run still answers status` |
| T13_control_plane_is_not_blocked | passed | `apply completed in 113ms while an exec slept for 5s` |
| T14_spawn_failure_is_honest | passed | `spawning a missing binary returned spawn_failed instead of a false ok` |
| T15_counters_are_complete | passed | `the counter summary is complete with no tool errors` |
| T16_lifecycle_budget_is_published | passed | `idle_timeout_s=3600 max_lifetime_s=7200 lifetime_deadline=1793949424322 kill_on_expiry=true` |
| T17_argv_survives_newlines | passed | `an argument with a newline reached the namespace intact` |
| T18_command_timeout_is_bounded | passed | `timeout_s 0 and 100000 are both refused` |
| T06b_unexpected_ipv6_route_is_a_violation | passed | `an injected 2001:db8::/64 route and an injected 10.99.0.0/24 route are both reported as unattached_route (v6=1 v4=1)` |
| T06c_owned_routes_are_restored | passed | `the injected routes are gone and the check reports 0 violations again` |
| T06d_faulted_link_is_not_a_violation | passed | `the no-bypass check during an intentional fault reports ["alpha-beta"] as faulted and raises no link_address_unreachable violation` |
| T12b_stale_run_dir_never_satisfies_a_startup | passed | `the stale directory was refused (stale_run_dir, fresh_dir_required, nothing deleted) and a fresh directory started a new backend with a new token` |
| T13b_long_execs_do_not_starve_the_control_path | passed | `status answered in 91ms while 8 long execs were running` |
| T15b_truncated_output_is_reported | passed | `a 2 MB output is capped at 262144 bytes and stdout_truncated is true` |
| T19_teardown | passed | `namespace holders and fixtures gone, no process carries the run token, no lab interface, address or route on the host` |
| T99_host_integrity | passed | `no lab interface, address or route on the host` |

Shell exit codes from `/tmp/opencode/LP4a-frozen/exit-codes.txt` (the non-zero
values are the negative tests, which are expected to fail the command):

```
preflight=0 topology_up=0 spawn_beta=0 spawn_gamma=0 spawn_delta=0 spawn_v6=0
normal_traffic=0 apply_partition=0 sustained_partition=0 spawn_session=0
apply_during_session=0 no_bypass=0 unrelated_link=0 heal=0 heal_delivery=0
latency=0 loss=0 atomic_reapply=0 duplicate_up=1 status_after_duplicate=0
concurrent_apply=0 spawn_missing_binary=1 timeout_bounds=1 teardown=0
unexpected_route=0 stale_run_dir=1 fresh_run_dir=0 status_under_load=0
```

### 5.2 The F1 regression, before and after the fault

`/tmp/opencode/LP4a-frozen/v6-during-fault.json`:

```json
{"result": "timeout", "elapsed_ms": 3003, "errno": null}
```

The same IPv6 link-local dial to `fe80::c072:38ff:feef:9d13` returns
`"result": "ok"` with a `PONG` payload before the partition and times out
during it, and succeeds again after the heal (`T08`). Fable's counterexample
was reproduced on the reviewed snapshot and does not reproduce now.

### 5.3 The established session is cut

`/tmp/opencode/LP4a-frozen/fixtures/session.jsonl`:

```json
{"event": "phase1", "response": "PONG persist 1 1793937404469", ...}
{"event": "phase2", "response": "FAIL:TimeoutError:timed out", "elapsed_ms": 3002, ...}
```

The session was established on a healthy link, the fault was applied while it
was open, and the next exchange on the *same* socket failed.

### 5.4 The reapply has no open window

`/tmp/opencode/LP4a-frozen/reapply-hammer.json`:

```json
{"attempts": 150, "succeeded": 0, "failed": 150}
```

150 reconnect attempts during 10 partition reapplies, none succeeded. On the
reviewed snapshot the same experiment let 4 of 300 connects through.

### 5.5 Counters

```json
{"nft_dropped_packets": 8, "nft_dropped_bytes": 480, "tx_packets": 40, "rx_packets": 40,
 "qdisc_drops": 0, "complete": true, "errors": []}
{"nft_dropped_packets": 0, "tx_packets": 215, "qdisc_drops": 14, "qdisc_packets": 43,
 "complete": true, "errors": []}
```

### 5.6 Relay fallback (weaker, exercised explicitly)

`testing/lab-isolation/run.sh --backend relay`
(`/tmp/opencode/LP4a-frozen-relay/summary.json`): `passed=6 failed=0 skipped=2`.

| Test | Result | Detail |
|---|---|---|
| R01_relay_up | passed | `2/2 sessions delivered through the relay` |
| R02_relay_sustained_outage | passed | `6/6 attempts failed while the relay was cut` |
| R03_relay_heal | passed | `2/2 sessions delivered after the relay was restored` |
| R04_relay_no_bypass_not_enforced | **inconclusive** | `a direct dial to the listener succeeded while the relay was cut: the relay backend cannot enforce no-bypass` |
| R05_relay_missing_state_fails_closed | passed | `the relay treated a missing state file as down, so no traffic was forwarded` |

R04 is the important one: the test *proves* the negative. While the relay was
cut, a direct dial to the listener's own port still succeeded. The relay
backend therefore offers a **controlled link outage**, and nothing more.

---

## 6. Guarantees and unsupported requests (for the LP-3 preflight)

The preflight JSON and the manifest carry the same lists, so an LP-3
preflight can fail a scenario without re-deriving anything.

Namespace backend (`backend: "netns"`):

```
guarantees: isolated_listeners, sustained_partition, link_scoped_fault,
            heal_restores_delivery, listener_stays_bound_during_fault,
            no_bypass_routing_enforced, reconnect_failures_observed,
            ipv6_link_local_isolated, namespace_forwarding_disabled,
            netem_latency_and_loss
```

Relay backend (`backend: "relay"`):

```
guarantees: controlled_link_outage, heal_restores_delivery,
            reconnect_failures_observed
```

`unsupported_requests` for the namespace backend contains only the LP-4
integration gates. The three relay-only entries are added **only** when the
selected backend is the relay:

| Request | Assertion state | Reason |
|---|---|---|
| `sustained_partition_assertion` | `inconclusive` (relay only) | the relay cannot enforce a kernel-level partition |
| `no_bypass_assertion` | `inconclusive` (relay only) | a relay cannot prevent a direct dial to the listener |
| `os_enforced_isolation` | `inconclusive` (relay only) | a relay is a userspace proxy, not an isolation boundary |
| `latency_and_loss_scenarios` | `inconclusive` | only when `sch_netem` is missing |
| `full_node_p2p_links` | `not_applicable` | LP-4 integration gate; this precursor starts no node |
| `ark_backend_links` | `not_applicable` | LP-4 integration gate; this precursor starts no Ark stub |
| `mining_and_reorg_scenarios` | `not_applicable` | LP-4 integration gate; this precursor runs no mining workload |

The backend never silently downgrades. `link-fault.sh up` fails with
`backend_unavailable` (exit 3) rather than falling back to the relay; the
relay is a separate, explicitly declared backend that a caller must ask for.

---

## 7. Capability limits and explicit non-claims

* **This is a precursor, not LP-4.** No full node, pool, front, miner or Ark
  stub is started. The proved entities are TCP *fixtures* that exist only to
  exercise the network primitives.
* **No-bypass is a routing guarantee.** It is enforced by the absence of any
  route (and any default route) in the namespace, verified by real connect
  attempts over both address families, with forwarding disabled. It is not a
  firewall: a namespace can only reach what its own routing table permits.
* **The IPv6 criterion is family specific.** For an IPv6 link-local target a
  timeout counts as unreachable, because the neighbour discovery request is
  dropped too. `ECONNREFUSED` still counts as a bypass for every family, and
  the linked neighbour's address must be reachable, which is the cross-check
  that keeps the timeout interpretation honest.
* **The partition is a silent drop.** A reconnecting client sees a connect
  timeout, not a refusal. That is the intended sustained-partition semantics,
  but a scenario must budget a connect timeout per attempt.
* **Loss is per-direction.** `loss:PCT` installs the qdisc on both sides of
  the link, so the effective round-trip loss is higher than `PCT`. The
  recorded counters are per side and are reported separately.
* **Address space.** The lab uses `10.250.0.0/16` in `/30` blocks, one per
  link, assigned in link order. It never overlaps the host.
* **Bounded resources.** 32 endpoints, 64 links, 30 s default command
  timeout, 3600 s idle timeout, 7200 s maximum lifetime, 8 concurrent
  requests, 1 MiB capture and request caps. The backend exits and cleans up
  when its budget is exhausted or its run directory is removed.
* **No packet capture** was performed anywhere.
* **No mainnet data, no production keys, no real funds** are involved.
* **The relay backend proves less.** It is a controlled outage only. Its
  no-bypass result is `inconclusive`, and the fallback test proves that
  directly instead of asserting it.
* **The host-wide snapshot hash changed during a run** in one observation
  (`host-wide snapshot hash unchanged=no`). The cause is unrelated host
  state on this machine (a libvirt/DHCP interface appearing or disappearing),
  not the lab: the hard assertions are that no host interface, address or
  route carries the run token or the lab address range, and those passed
  every time. A whole-host hash is not a usable lab invariant on a shared
  host and is reported informationally only.
* **Filesystem and security sandbox claims are not made.** This packet proves
  network isolation inside namespaces it owns. It does not claim a security
  sandbox, a capability drop for arbitrary code, or anything beyond the
  network guarantees listed above.
* **Surviving the supervisor is not supported and not claimed.** The
  namespace holders and the fixtures die with the supervisor
  (`PR_SET_PDEATHSIG=SIGKILL`). `--leave-for-adoption` is refused with
  `not_implemented`. The manifest publishes `adoption_model: "pdeathsig"`,
  `leave_for_adoption: false` and `supports_survive_supervisor: false`.
  Reconnecting to a *live* backend is a different property and is supported.
* **A crashed run directory is never reused.** The operator must use a fresh
  directory; the old one keeps the diagnostics of the dead run.
* **Explicitly deferred** (not implemented, not claimed):
  * an IPv6 *disabled* lab interface (IPv6 stays enabled and the partition
    covers it through the interface scoped rules);
  * a `reject` instead of `drop` partition (the drop is silent on purpose);
  * adoption of a crashed supervisor's resources (§8.1 of the contract) -
    the runner would have to own the lifetime itself;
  * a control-plane RPC inside `spc-lab` (the precursor has its own Unix
    socket; the runner interface is still to be defined).
* **Nix was not validated here** (section 11).

---

## 8. Resource ownership and teardown

* Every run gets an 8-hex-character **run token**. It appears in the run
  directory name, every interface name (`l<token><link><side>`), every nft
  table name (`lab4a_<token>_<link><side>`), the backend argv, the fixture
  environment and the control socket path.
* Every PID the backend records (namespace holders, spawned fixtures) is
  recorded with its `/proc/<pid>/stat` field 22 **start-time token**. Teardown
  only signals a PID whose start-time token still matches; a recycled or
  unrelated PID is never touched.
* The run directory has exactly one owner: the backend holds an exclusive
  `flock` on `<run-dir>/.backend.lock` for its whole life, and the CLI
  refuses a directory that a live backend owns.
* Namespace holders and the spawner thread get `PR_SET_PDEATHSIG=SIGKILL`,
  so a backend that dies unexpectedly does not leak namespaces or fixtures.
  The spawner thread exists for exactly this reason: the death signal is
  delivered when the *creating thread* exits, so the fixtures must be created
  by a thread that lives as long as the supervisor.
* This is deliberately *different* from the §8.1 adoption model, which
  applies to the LP-3 supervisor: this precursor prefers "no stale
  namespace" over "leave it for adoption". The manifest records the model
  (`adoption_model: "pdeathsig"`, `supports_survive_supervisor: false`) and
  the flag that would ask for the other model is refused. The runner, which
  owns the lifetime, would have to implement adoption itself.
* Teardown order: fixtures first (SIGTERM, wait, SIGKILL), then namespace
  holders, then reap, then unlink the socket (only if this process created
  it), then release the lock. The nft tables and veth pairs disappear with
  their namespaces.
* The test asserts the teardown: no live holder PID, no live fixture PID, no
  process carrying the run token, no lab interface, address or route on the
  host.

---

## 9. Merge notes for the future `crates/spc-lab/src/links/netns.rs`

Read-only inspection of the LP-3 candidate
(`crates/spc-lab/src/links.rs`, `crates/spc-lab/src/scenario.rs`). The LP-4
netns backend must satisfy this trait:

```rust
pub trait LinkBackend: Send + Sync {
    fn name(&self) -> &'static str;                       // "netns"
    fn capabilities(&self) -> Vec<String>;                // e.g. "lab.link.netns"
    fn supports_sustained_fault(&self) -> bool;            // true
    fn preflight(&self, scenario: &Scenario) -> Result<(), LinkError>;
    fn apply(&self, link: &LinkSpec, policy: &LinkPolicy) -> Result<(), LinkError>;
    fn current(&self, link: &LinkId) -> Option<LinkPolicy>;
}
```

Required work and the decisions this precursor already made:

1. **`backend_for("netns")`** must return the new backend instead of
   `UnsupportedLinkBackend`. The current `UnsupportedLinkBackend` fails
   closed, which is the correct default until the netns backend exists.
2. **`LinkPolicy` mapping** (`scenario.rs`): `Normal` -> remove nft tables and
   root qdisc; `Partition` -> the interface scoped drop; `Latency{delay_ms}`
   -> `netem delay <ms>ms`; `Loss{loss_percent}` -> `netem loss <pct>%`. The
   scenario vocabulary and the backend vocabulary already match one to one.
   `scenario.rs` rejects `loss_percent > 100`; the backend must reject
   `delay_ms == 0` and `loss_percent > 100` as well.
3. **Addresses are not in the scenario.** `LinkSpec` has only `id`, `from`,
   `to` and `policy`. The netns backend therefore has to **allocate**
   addresses itself and publish them, exactly as the precursor does
   (`10.250.x.y/30` per link, in link order). The allocation must move into
   `LinkSpec`/`Scenario` (or a new topology plan) before LP-4 scenarios can
   assert a specific endpoint address. The precursor's manifest is the shape
   of that publication.
4. **A link has exactly two endpoints, but a scenario entity can be on
   several links.** The precursor models one veth pair per link, so an entity
   with three links gets three veth pairs and three addresses. The netns
   backend must therefore key namespaces by *entity*, not by link, and
   address allocation must be per (entity, link) pair.
5. **`preflight(&Scenario)`** must fail closed. The precursor's equivalent is
   `netns-preflight.sh`: it must be callable as a library function that
   returns the same capability structure (`supported`, `reason`, `evidence`)
   and the same `guarantees` / `unsupported_requests` / `guarantee_scope`
   lists, so the runner can map them onto `LinkError::Unavailable` and onto
   `supports_sustained_fault() == false`. The guarantee and the unsupported
   list must stay disjoint; the precursor's preflight refuses to publish an
   artifact that contradicts itself.
6. **The runner's `Scenario` names node/pool/front/miner/identity/ark
   endpoints.** The netns backend must map `EndpointKind` to a namespace. An
   entity that owns no process (for example an identity) still needs one if
   it is a link endpoint. `EndpointKind::Redis` and `EndpointKind::Postgres`
   are in the LP-3 vocabulary but are not link endpoints in the LP-4
   scenarios; the backend should reject them explicitly.
7. **Namespace handle design.** Use PID handles
   (`/proc/<pid>/ns/net`), never `/run/netns`. A Rust implementation should
   use `unshare(CLONE_NEWNET)` in a forked holder and `setns` on
   `/proc/<pid>/ns/net`. Three traps this precursor hit and that a Rust port
   must avoid:
   * joining a user namespace from outside needs the creator's `fsuid` to
     equal the namespace owner, and `nsenter`'s default `setgroups` call
     fails with `EPERM` for an unprivileged user (use `setns` directly, or
     `nsenter --preserve-credentials`);
   * `PR_SET_PDEATHSIG` is delivered when the creating **thread** exits. A
     multi threaded runner must not create long lived children from a
     request thread;
   * the runner owns the lifetime. This backend implements the `pdeathsig`
     model only (`adoption_model: "pdeathsig"`,
     `supports_survive_supervisor: false`); if the runner wants adoption
     after a supervisor crash it has to implement it, and the backend would
     then need a mode that does not install the death signal.
8. **Namespace scope.** A network namespace is per thread. The precursor
   never changes the namespace of its own threads: every in-namespace command
   is an external `nsenter --net` process. A Rust port can do the same with
   `nix::sched::setns` in a forked child, or with `pre_exec` in a `Command`,
   but it must not `setns` in a worker thread.
9. **`supports_sustained_fault`** must be `true` for the netns backend and
   `false` for the fixture backend. The precursor never claims a fixture
   backend can satisfy a partition assertion.
10. **Evidence.** The runner needs counters and timestamps. The precursor
    exposes them as `counters` (nft rule counters, qdisc drops, interface
    packet counters, all per side, with a `complete` flag) and as timestamps
    on every fixture event and every client attempt. The `spc-lab` evidence
    store should carry the same fields; `netns-preflight.sh`'s JSON is the
    proposed capability record.
11. **Atomic fault application.** The precursor applies the partition as one
    `nft -f -` transaction. A Rust port must do the same; a sequence of
    `nft add` calls reintroduces the fail-open window Fable measured.
12. **The fixture backend must stay honest.** The LP-3 `FixtureLinkBackend`
    records a policy but cannot sustain a fault, and says so. Nothing in this
    packet changes that.

---

## 10. Remaining LP-4 integration gates (not done here)

* Two pools with separate Redis instances and Postgres databases that cannot
  read each other's state (§8.2).
* A pool that loses a link while its **real** reconnect loop keeps failing
  for the declared fault duration, with the pool's own logs and counters as
  evidence (this packet proves the mechanism with a TCP fixture, not with a
  pool).
* Full-node P2P links and the node's listen addresses inside namespaces.
* The Ark stub and the explicit `pool -> ark-stub` and
  `identity/miner -> ark-stub` links, including the Ark-specific
  pending/failed state while the Ark link is down.
* A retarget run covering an adjustment boundary.
* Accounting pool-locality, greatest-work reorg selection, no double
  settlement, and the statistics scope (estimator, planned N, stopping rule)
  from §9 of the contract.
* The `scenarios/lab/*.json` files and `scripts/lab/ci.sh` required by the
  LP-4 packet's required-test list. This packet deliberately ships the
  topology spec it uses inline in the test driver rather than claiming a
  scenario vocabulary.

---

## 11. Shared-file merge notes (not applied here)

* `flake.nix`, `devShells.default.packages`: add `pkgs.iproute2`,
  `pkgs.nftables`, `pkgs.util-linux`, `pkgs.python3` and `pkgs.procps`.
  `pkgs.jq` is already present. `iproute2` provides `ip`, `tc` and `ss`;
  `nftables` provides `nft`; `util-linux` provides `unshare` and `nsenter`;
  `procps` provides `sysctl`, which the backend uses for the namespace scoped
  forwarding and IPv6 knobs; `python3` is needed by the supervisor, the
  socket transport, the probe helper, the spawn helper and the TCP fixtures.
  This is exactly the dependency set named in LP-4 ("`flake.nix` must add
  `iproute2`, `nftables` and `util-linux`"), with `python3` and `procps`
  added because this precursor implements the backend in shell and Python
  rather than in Rust. If LP-4 lands the backend as Rust instead, `python3`
  and `procps` are only needed by the fixture scripts and the sysctl
  writes could be done directly through `/proc/sys`.
* **The Nix environment could not be validated on this host, and the gate is
  open.** The attempt used the project's **already locked** input, exactly as
  Fable suggested, and the repository was not touched:

  ```sh
  nix eval --offline --inputs-from /home/ethan/code/silent-pool \
      --raw nixpkgs#lib.version
  # -> 26.11.20261101.0bb2eedr        (the locked nixpkgs resolves offline)

  nix shell --offline --inputs-from /home/ethan/code/silent-pool \
      nixpkgs#iproute2 nixpkgs#nftables nixpkgs#util-linux \
      nixpkgs#python3 nixpkgs#jq nixpkgs#procps -c ...
  # -> error: unable to download
  #    'https://download.savannah.gnu.org/releases/acl/acl-2.4.0.tar.gz':
  #    Timeout was reached (28) Failed to connect to server
  ```

  The exact blocker: the locked nixpkgs input is present, but the store holds
  only *derivations* for the tool packages (`iproute2-7.2.0.tar.xz`,
  `nftables-1.1.7.tar.xz`), not built outputs, so every build has to realise
  the stdenv bootstrap (bash 5.3 and its patches, bash-2.05b, acl, attr,
  autoconf, automake) from source. This host cannot reach the source mirrors,
  so the build cannot start. The binary cache does not cover the pinned
  inputs either.
  The honest record is therefore the host tool versions in section 3 (util-linux
  2.37.2, iproute2 5.15.0, nftables 1.0.2, jq 1.6, python3 3.10.12, procps
  `sysctl`) plus an **open Nix integration gate**: the devShell must gain the
  packages above and the suite must be run from it in CI. No Nix result is
  claimed by this packet.
* `flake.nix`: a `checks.lab-isolation` entry that runs
  `testing/lab-isolation/run.sh` would be a natural CI gate, but it needs a
  kernel that permits unprivileged user namespaces, so it belongs behind an
  explicit opt-in rather than in the default `nix flake check`.
* `justfile`: a `lab-isolation:` recipe calling
  `./testing/lab-isolation/run.sh`. Not added here; the justfile is shared.
* `Cargo.toml` / `Cargo.lock`: **not needed** by this packet. No Rust crate
  was added. If the netns backend moves into `spc-lab`, the crate needs
  `libc` (or `nix`) for `unshare`/`setns`/`prctl`, and `Cargo.lock` changes
  in the same change.
* `docs/REGTEST-LAB-STATUS.md`: the merge session should record this packet's
  state (revised, Fable re-review pending) next to LP-3.

---

## 12. Review questions for Fable

1. **Namespace lifetime versus the adoption model.** This precursor kills
   namespaces when its backend dies (`PR_SET_PDEATHSIG`) and refuses
   `--leave-for-adoption` instead of publishing an intent it does not
   enforce. If the runner needs adoption, the backend would need a mode that
   does not install the death signal; that mode is not implemented here and
   is listed as an explicit limit.
2. **Address allocation.** The scenario vocabulary has no address field. Is
   backend-allocated `10.250.0.0/16` acceptable for LP-4, or must the
   topology plan carry explicit addresses so a scenario can assert them?
   (Fable's answer was that backend allocation is fine if it is published in
   the manifest and copied into the run manifest; this packet publishes it
   and does not copy it into a runner manifest, because no runner manifest
   exists yet.)
3. **Relay scope.** The fallback is implemented and tested, and it is honest
   about its limits. Should the relay backend exist at all in the runner, or
   should a missing namespace capability simply be a preflight failure with
   no fallback? The contract (§8.3.8) permits the relay only for a smoke
   scenario tagged `isolation=relay`, and this packet follows that.
4. **Drop versus reject.** The partition drops silently, so clients time
   out. If the LP-4 scenarios need a rejection instead, that is a one-line
   nftables change (`reject` instead of `drop`) but it changes the reconnect
   timing model.
