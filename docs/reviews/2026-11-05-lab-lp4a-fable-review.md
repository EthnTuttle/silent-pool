# LP-4a precursor (netns isolation + preflight): Fable review

Date: 2026-11-05
Reviewer: Fable (Claude Fable 5.1), read-only review subagent
Candidate: worktree `/home/ethan/.local/share/opencode/worktree/ad8886/lab-link-preflight`, base `b0ab4ca34fda07c61a6335d05f598eaacc69678c`, 10 uncommitted files
Snapshot: `/tmp/opencode/lab-lp4a-review-manifest.json`, recipe hash `d004a6df21dbd7dea12713f0148dfa26b577e3f20f7f4d33eca8e54af27dc5c6` (recomputed: match; all 10 file SHA-256 and exec modes verified against the tree)
Contract: `docs/REGTEST-LAB-IMPLEMENTATION.md` §8.3 (link-fault backend), §8.1 (adoption), LP-4 packet, from the `lab-contracts` worktree at `b0ab4ca`
Candidate report: `docs/reviews/2026-11-05-lab-lp4a-implementation.md` (554 lines)

## Verdict

**REQUEST_CHANGES** for snapshot `d004a6df21dbd7dea12713f0148dfa26b577e3f20f7f4d33eca8e54af27dc5c6`.

The primitives are largely right and honestly scoped as a precursor: unprivileged `unshare -Urn`, PID-handle namespaces (no `/run/netns` writes), per-link veth with `/30` addresses and no default route, both-sided nft drop, `tc netem`, start-time-token guarded cleanup, SIGTERM and SIGKILL teardown all verified here. No host network modification, elevation, `sysctl` write or `modprobe` exists in the source. The relay fallback is correctly declared weaker and R04 proves the negative.

It cannot be approved as-is because the manifest and preflight publish `sustained_partition` and `no_bypass_routing_enforced` as unqualified guarantees while the partition is IPv4-only and IPv6 link-local traffic crosses the "partitioned" veth (F1, reproduced). Three further defects corrupt or weaken a live run (F2 double `up` destroys the first backend's control socket; F3 reapply fail-open window, reproduced with 4 bypassing connects; F4 preflight JSON contradicts its own `guarantees`). These are small fixes, but §8.3 and the LP-4 Fable gate are explicit that a fail-open or a bypass is sent back.

Nothing here is a judgement on node P2P, mining, reorg or Ark delivery: the fixtures are TCP echo endpoints and the candidate says so. Section 9 of the candidate report (remaining LP-4 gates) is accurate and should stay.

## Probes run by this review

All probes used throwaway namespaces created by the candidate's own `link-fault.sh up`; every run was torn down (`down`, SIGTERM, or SIGKILL) and the host was checked afterwards: 0 lab interfaces, 0 `10.250.*` addresses or routes, `/run/netns` empty, no `lab_backend.py`/fixture processes left. Artifacts: `/tmp/opencode/Fable-LP4a-probe`, `/tmp/opencode/Fable-LP4a-kill`, `/tmp/opencode/Fable-LP4a-nb`.

Toolchain: host tools (Ubuntu, kernel 6.8.0-138, util-linux 2.37.2, iproute2 5.15.0, nftables 1.0.2, python 3.10), the same set the candidate used. `nix` is present but `flake.nix` contains none of `iproute2`/`nftables`/`util-linux`, so the suite cannot run from the devShell today; the candidate's merge note (section 10) is correct and necessary. No performance claims are made in this review.

Did not re-run the full 12-test suite five times; the five existing summaries (`LP4a-final2`, `final4`, `stab1`, `stab2`, `verify`: 12/12, exit 0; `relay2`: 5 passed, R04 inconclusive) were checked and match the report.

## Findings (ranked)

### F1 (blocking) - IPv6 link-local traffic bypasses the partition; guarantee scope does not match the rules

Code: `scripts/lab/lib/lab_backend.py:523-534` (`_nft_partition`: `ip daddr`/`ip saddr` only), `:494-502` and `scripts/lab/netns-preflight.sh:150-156` (unqualified `sustained_partition`, `no_bypass_routing_enforced`), `:671-678` and `:729-749` (`ip -j route show` is IPv4 only; `_connect_probe` is `AF_INET` only), `scripts/lab/link-fault.sh:151` (host addresses: `ip -4 ... scope global` only).

Counterexample (`/tmp/opencode/Fable-LP4a-probe`): every veth gets a kernel-assigned `fe80::/64` address and route (`ip -6 route show` in each namespace lists `fe80::/64 dev l<token>..`). With an IPv6 listener bound to `[::]:9301` in `beta` and `apply --policy partition` on the link:

```
IPv4 dial 10.250.0.2:9301 during partition  -> timeout (dropped, as designed)
IPv6 dial fe80::50e9:c7ff:fe98:556d%l..00a  -> {"result":"ok","response":"PONG6 PING6"}
counters: nft_dropped_packets=2            (only the IPv4 SYNs)
```

Any real endpoint that binds dual-stack (bitcoind, most daemons by default) is reachable across a "partitioned" link by its link-local address. The no-bypass check never lists IPv6 routes, never probes IPv6 targets and never probes IPv6 host addresses, so it reports `ok: true` for a topology where this path is open.

Also observed: `net.ipv4.ip_forward` is `1` inside every lab namespace (inherited from this host, which has forwarding on). With the current `/30`-only routes nothing can transit `alpha`, but the check neither pins nor asserts it.

Fix (any one of the first two, plus the third and fourth):
1. Make the drop family-agnostic and interface-scoped: `oifname "<iface>" counter drop` in `output` and `iifname "<iface>" counter drop` in `input` (inet family covers v4+v6; the veth is the link, so no address match is needed). Or:
2. Disable IPv6 on every lab interface at link creation (`sysctl -w net.ipv6.conf.<iface>.disable_ipv6=1`, allowed inside the owned netns) and record `ipv6: disabled` per link in the manifest.
3. Extend `no_bypass_check`: list `ip -6 route`, probe `AF_INET6` for any non-link global/LL address of every other endpoint, enumerate host `ip -6 addr scope global` too, and assert `ip_forward=0` / `conf.all.forwarding=0` in every namespace (set them at namespace creation; they are netns-scoped and writable by the owner).
4. Until 3 lands, qualify the guarantees: `sustained_partition_ipv4`, `no_bypass_routing_enforced_ipv4`, and add `unsupported_requests: {request: ipv6_partition, assertion_state: inconclusive}`.

Test: add `T05b_partition_blocks_ipv6_linklocal` (the probe above must time out or be `ENETUNREACH`), and `T06` must include at least one `AF_INET6` probe and the forwarding assertion.

### F2 (blocking) - `up` on an existing run directory destroys the live run and reports success

Code: `scripts/lab/link-fault.sh:139-186` (no check for an existing manifest/socket; `lab_wait_for_file manifest.json` returns at once on the old file), `lab_backend.py:1096-1113` (second backend overwrites `manifest.json` at `:1100`, fails `bind` at `:1007`, then `cleanup()` at `:889-892` unlinks the first backend's `control.sock`).

Counterexample (`/tmp/opencode/Fable-LP4a-probe/up2.json`): second `up --run-dir <same>` returned exit 0 and printed the first backend's manifest with `ok: true`. Afterwards: `manifest.json` names the second backend's dead PIDs, `control.sock` is gone, the first backend (and its holders and spawned fixtures) is alive, unreachable, and will persist until its 3600 s idle timeout. `status` returns `backend_unavailable` (exit 3). Recovery required a manual SIGTERM.

Fix: in `cmd_up`, refuse a run directory that already contains `manifest.json` or `control.sock` (exit 1, `run_dir_in_use`, include the recorded supervisor PID/token and whether it still matches its start-time token). In the backend, bind the control socket **before** creating namespaces or writing the manifest, and never `unlink` a socket path this process did not create. Also honor or drop `--run-id` (currently parsed and unused, `link-fault.sh:71-72`).

Test: `up` twice on the same dir; second must fail without touching the first's socket or manifest; `status` on the first must still succeed.

### F3 (blocking) - reapplying `partition` opens the link transiently; heal and clear results are ignored

Code: `lab_backend.py:561-575` (delete table + clear qdisc, then five separate `nft add` invocations), `:564-565` (return values of `_nft_heal`/`_tc_clear` discarded for every policy, including `normal`).

Counterexample (`/tmp/opencode/Fable-LP4a-probe/run/hammer.log`): with `partition` already applied and a client in `alpha` dialling `10.250.0.2:9302` 300 times (0.3 s timeout, no interval), eight consecutive `apply --policy partition` calls let **4 of 300** connects succeed (attempts 6-9). Each reapply is a fail-open window of a few milliseconds; an idempotent retry from a runner is exactly when it happens.

Separately, `heal` (`apply normal`) returns `ok: true` and records `policy: normal` whether or not `nft delete table` succeeded; nothing verifies the table is gone. That is a false-ok path.

Fix: build the whole partition ruleset as one `nft -f -` transaction (`add table` / `flush table` / chains / rules in one file; nftables applies a file atomically), so reapply replaces without a gap. For `normal`, check the result of `nft delete table` (accept only exit 0 or "No such file or directory"), then verify with `nft list table` that it is absent, and verify `tc qdisc show` has no netem; fail with `policy_apply_failed` otherwise.

Test: the hammer above must report `succeeded: 0` across 10 reapplies; a heal must fail if the table still exists (can be simulated by making the nft binary unavailable on PATH for the backend in a test).

### F4 (blocking, small) - preflight JSON contradicts itself in netns mode; report does not match the artifact

Code: `scripts/lab/netns-preflight.sh:157-170` (relay-only `inconclusive` entries are emitted unconditionally).

Evidence: `/tmp/opencode/LP4a-final4/preflight.json` has `backend: netns`, `guarantees: [.., sustained_partition, no_bypass_routing_enforced, ..]` **and** `unsupported_requests: [{sustained_partition_assertion: inconclusive}, {no_bypass_assertion: inconclusive}, {os_enforced_isolation: inconclusive}, ..]`. The candidate report (section 5) labels these "(relay only)"; the script does not. An LP-3 preflight that trusts `unsupported_requests` (as section 5 recommends) would mark every netns scenario inconclusive; one that trusts `guarantees` would pass it. The manifest (`lab_backend.py:503-507`) omits the three entries and is internally consistent.

Fix: emit the three relay entries only when `BACKEND != netns`; add a unit check that `guarantees` and `unsupported_requests[].request` never overlap. Update section 5 of the report to match.

### F5 (should fix) - control plane is single-threaded; a long `exec` delays `apply`/`heal` unboundedly

Code: `lab_backend.py:1005-1043` (`serve` handles one connection to completion; `_op_exec` blocks up to `timeout_s`).

Evidence (`/tmp/opencode/Fable-LP4a-probe/established2.out`): with an `exec` client holding the socket for 6 s, `apply partition` issued at 03:12:08.130 completed at 03:12:12.875 (4.7 s later, `applied_at` is honest), so the intended fault landed after the client's second exchange. The same experiment with the client started via `spawn` applied in 0.2 s and the **established** TCP session was cut as required (phase 2 `TimeoutError` after 3003 ms, `nft_dropped_packets: 6`). So the kernel path is right; the control path is not.

Fix: run `exec` (and `no-bypass-check`, which forks probes) off the accept loop, or document that `exec` is for short commands and reject `timeout_s` > a small bound. Add `queue_wait_ms` to every response so a runner can see the slip. The manifest's `default_command_timeout_s` should not be the only bound.

Test: concurrent `exec sleep 5` and `apply partition`; `apply` must complete in < 1 s.

### F6 (should fix) - `spawn` returns `ok: true` for a command that never started

Code: `lab_backend.py:806-821` (sleeps 50 ms, records the token, returns ok; `spawn_in_ns` exits 127 on `execvp` failure), `_op_status` (`:918-939`: children have no liveness field).

Counterexample: `spawn -- /nonexistent/binary` returned `{"ok": true, "pid": 2731221, "start_time_token": "21666714"}`; the process was a zombie (`Z`) immediately afterwards and `status` still lists it with no indication.

Fix: in `spawn_in_ns`, use a close-on-exec pipe (or `os.posix_spawn`-style error channel) so the parent learns an exec failure synchronously and returns `spawn_failed` with the errno; reap on failure. Add `alive` (start-time token match and not `Z`) to each child in `status`, reusing `pid_matches`.

### F7 (should fix) - command failures in evidence paths degrade to empty/zero instead of failing closed

Code: `lab_backend.py:671-678` (`_routes` returns `[]` when `ip -j route show` fails: the routing-table check then passes vacuously), `:778-792` (nft ruleset listing failure yields `tables=[]`, no foreign tables, pass), `:609-668` (`counters` records `nft_error`/`qdisc_error`/`link_error` but `summary` sums zeros and the test driver reads only `summary`).

Not reproduced (requires inducing tool failure) but a direct reading of the code; the candidate's own criterion ("counter/status/probe command failures cannot be ignored or read as zero") is not met.

Fix: any failed `ip`/`nft`/`tc` invocation in `no_bypass_check` is a violation of kind `evidence_unavailable`; `counters.summary` gains `complete: false` plus an `errors` list whenever a side reports an error, and `run.sh` must treat `complete: false` as a failed assertion.

### F8 (should fix) - lifecycle budget is invisible in the manifest and kills quiet long runs

Code: `lab_backend.py:995-1003` (idle 3600 s / lifetime 7200 s default, then `cleanup()` kills every holder and child), `:508-512` (manifest `limits` lacks both values; verified: `{"default_command_timeout_s":30,"max_endpoints":32,"max_links":64}`). `0` disables a budget (`if self.max_lifetime_s and ...`) but this is undocumented.

Risk: an LP-4 scenario that mines quietly for > 1 h without control-plane traffic loses its namespaces and processes mid-run. The report's section 6 mentions the budgets; the artifact a runner reads does not.

Fix: publish `idle_timeout_s`, `max_lifetime_s`, `started_at_unix_ms` and the `0 = disabled` rule in `limits`; count a live spawned child as activity (idle should mean "no requests and no children"), or require the runner to pass `--idle-timeout-s 0` explicitly and have `up` echo the effective values.

### F9 (minor) - input handling and hygiene

- `link-fault.sh:267,277`: argv elements are split on newlines (`printf '%s\n' | jq -R`), so `exec -- printf '%s|' $'a\nb' c` reaches the namespace as `["a","b","c"]`. Use `jq -n '$ARGS.positional' --args -- "${EXTRA_ARGV[@]}"`. No shell injection exists: `execvp` is used everywhere and the argv is JSON end to end.
- `lab_backend.py:968`: `timeout_s` accepts `"inf"` (unbounded wait) and negatives (instant SIGKILL, reported as `timed_out`). Bound to `(0, 3600]`.
- `lab_backend.py:1027-1032`: request line read is unbounded; `run_in_ns` captures unbounded stdout/stderr into the JSON response. Cap both (for example 1 MiB) and report truncation.
- `lab_backend.py:1006-1008`: bind then `chmod 0600` leaves a short window at umask mode; set `os.umask(0o077)` before bind. Run directory is created `0775` (`link-fault.sh:139`); make it `0700`.
- `lab_backend.py:960-974, 806-813, 680-690`: `exec`, `spawn` and `_connect_probe` skip `ep.ns.alive()` (only `_run_in` checks). In practice `setns` into a recycled host PID fails with `EPERM` from inside the user namespace, but the error surfaces as exit 127, not `namespace_gone`.
- `lab_backend.py:201-205`: a timed-out `exec` kills the PID, not its process group; use `os.setpgid` in the child and `os.killpg`.
- `testing/lab-isolation/run.sh:411` relies on `pgrep -f <token>`; spawned fixtures carry the token only in their environment (`LAB_RUN_TOKEN`), not argv, so T11 cannot see a leaked fixture. Check `/proc/<pid>/environ` for recorded children, or add `--lab-run-token` to fixture argv.
- `relay_fixture.py:61-66`: a missing or unreadable state file reads as `up` (fail-open). Default to `down`.

## What was verified and holds

- No `sudo`, `sysctl -w`, `modprobe`, host `ip`/`nft`/`tc` mutation anywhere in the ten files; all mutations happen after `setns` into a holder whose PID was validated against its recorded start-time token (`_run_in`), and the holder lives in the run's own user namespace.
- SIGTERM to the supervisor: holders, namespaces, veths, nft tables and spawned fixtures gone within 2 s, socket unlinked. SIGKILL to the supervisor: holders and spawned fixtures die via `PR_SET_PDEATHSIG`, host untouched; only the stale socket file remains (feeds F2).
- Partition cuts already-established bidirectional TCP (spawn-driven probe: second exchange timed out, 6 drops counted), not only new SYNs. Reconnect behaviour in T05 is consistent (8 SYN drops for 8 attempts at 1 s timeout).
- Validation rejects `latency:0`, `loss:101`, unknown link, unknown op, malformed JSON, self-links, duplicate names, > 32 endpoints / > 64 links.
- `up` fails closed with `backend_unavailable` rather than falling back to the relay; the relay suite must be requested with `--backend relay` and records R04 as `inconclusive`.
- Teardown assertions in the five recorded runs are real and the artifacts match the report's numbers.

## Integration interface assessment (read-only; LP-3 runner under rework)

The candidate's section 8 is a fair reading of the gap: the scenario vocabulary has no address allocation, namespaces must be keyed by entity, listeners must bind to backend-published addresses, and there is no control-plane RPC in `spc-lab` yet. Two additions for that merge: the §8.1 adoption model vs `PR_SET_PDEATHSIG` question (candidate section 11.1) should be decided as "runner owns lifetime; backend must support both `--pdeathsig` and `--leave-for-adoption` with the manifest recording which", and the backend-allocated `10.250.0.0/16` is acceptable for LP-4 only if the topology plan records the allocation so scenarios assert published, not assumed, addresses. Neither blocks this precursor.

## Answers to the candidate's review questions

1. Lifetime: see above; make it a flag, record it, default to `pdeathsig` for the standalone CLI and adoption when driven by the runner.
2. Addresses: backend-allocated is fine for LP-4 if published in the manifest and copied into the run manifest; do not put addresses in the scenario vocabulary yet.
3. Relay: keep it, exactly as scoped (`isolation=relay`, assertions inconclusive). The current implementation honours §8.3.8.
4. Drop vs reject: keep `drop`; §8.3 wants the reconnect loop to keep failing and a silent drop is the stronger fault. Offer `partition:reject` later only if a scenario needs it.

## Required for APPROVE

F1 (family-agnostic or IPv6-disabled drop, IPv6-aware and forwarding-aware no-bypass check, or qualified guarantees), F2, F3, F4, each with the named test; F5-F8 fixed or explicitly listed as limits in the manifest and report. Re-snapshot and re-hash after the rework.

---

# Second review: revised LP-4a (F1-F9 rework)

Date: 2026-11-05
Candidate: same worktree, base `b0ab4ca34fda07c61a6335d05f598eaacc69678c`, 12 uncommitted files (two new helpers: `scripts/lab/lib/ns_exec.py`, `scripts/lab/lib/netns_probe.py`)
Snapshot: `/tmp/opencode/lab-lp4a-review-manifest.json`, recipe `sha256(compact sorted JSON of {base, files[path, sha256, mode(100644|100755)]})` = `a981352732f2312967866a5d7a5469167f0131e84606785a11a165b7bb8c4ef4` (recomputed: match; all 12 SHA-256 and Git exec modes verified against the tree)
Candidate report: `docs/reviews/2026-11-05-lab-lp4a-implementation.md` (852 lines); artifacts `LP4a-revD`, `stab1`, `stab2`, `final`, `freeze` (22/22 each) and `final-relay`/`freeze-relay` (6 passed, R04 inconclusive) checked and consistent with the report.

## Verdict

**REQUEST_CHANGES (bounded)** for snapshot `a981352732f2312967866a5d7a5469167f0131e84606785a11a165b7bb8c4ef4`.

All four first-round blockers are fixed and independently reproduced as fixed (G1-G4 below). The rework introduced one new false-ok path in the exact area F2 was meant to close (N1: stale-run replacement reports success for a backend that failed to start), and two published fields are untrue (N2: `--leave-for-adoption` is a no-op flag that the manifest reports as honoured; N3: the IPv6 half of the `no_bypass_routing_enforced` route check does not flag non-link-local routes). These are small, but N1 is the same class of defect as F2 and N2/N3 are manifest/guarantee truthfulness, which is what this gate is for. Everything else is low and can be carried as stated limits.

On fixing N1-N3 (with the named tests) this precursor is approvable with the LP-4 integration gates exactly as listed in the candidate's section 10: no node, Ark, mining or reorg claim is made or accepted here.

## Independent re-tests of the first-round counterexamples (all against the frozen code)

Artifacts: `/tmp/opencode/Fable-LP4a-r2`, `-r3`, `-r4`. Every run was torn down (`down`, SIGKILL, or `rm -rf` of the run dir); afterwards: no `lab_backend`/`ns_exec`/fixture processes, `/run/netns` empty, no lab interfaces, `10.250.*` addresses or routes on the host, host `ip_forward` still `1`. Host tools only (util-linux 2.37.2, iproute2 5.15.0, nftables 1.0.2, python 3.10); see "Nix" below.

- **G1 (F1 fixed).** IPv6 link-local dial `fe80::14cc:...%l<token>00a` to a `[::]:9301` listener in `beta`: `ok` before `partition`, `timed out` (3003 ms) during it; IPv4 dial also times out; `nft_dropped_packets: 6`. Rules are `oifname`/`iifname` drops in an `inet` table; manifest `forwarding: {ipv4: 0, ipv6: 0}` in every namespace; `host_addresses_v6` populated (7). Guarantee scope strings are published.
- **G2 (F2 fixed, live path).** Second `up` on a live run dir: exit 1, `run_dir_in_use`, manifest and socket untouched, `status` still answers. A second backend started directly (bypassing the CLI) on the same dir fails on the `flock` with `run_dir_in_use` and does not disturb the first.
- **G3 (F3 fixed).** 300 connects at 0.3 s timeout during 10 `partition` reapplies: `succeeded: 0` (first round: 4/300). Exactly one lab table remains afterwards. `heal` failure paths now verify absence of table and qdisc.
- **G4 (F4 fixed).** Netns preflight: `unsupported_requests` = the three LP-4 gates only; `guarantees` (10) disjoint; `guarantee_scope` present; the IPv6 link-local and forwarding capabilities are probed and recorded.
- **F5.** `heal` completed in 163 ms with an `exec sleep 5` in flight (first round: 4.7 s). Established TCP session cut by a spawn-driven probe (phase 2 `TimeoutError`, 121 ms apply latency). Per-link lock allows `ag` mutation while `ab` is busy.
- **F6.** `spawn /nonexistent/binary` -> `spawn_failed` (exit 1); `status.children[].alive/state` present.
- **F7.** Owned-namespace IPv4 route `10.99.0.0/24` is flagged `unattached_route`; `counters.summary.complete/errors` present.
- **F8.** `limits` publishes idle/lifetime/deadlines/zero-disable; `_out_of_budget` counts live children as activity.
- **F9.** argv with embedded newline arrives intact; `timeout_s` bounded; run dir `0700`, socket/manifest/lock `0600`; timed-out `exec` kills the whole session (`sh -c 'sleep 30 & sleep 30'` left nothing); SIGKILL of the supervisor kills holders and fixtures through the `ns_exec` PDEATHSIG chain; relay fails closed (R05).

## New findings (ranked)

### N1 (blocking) - stale-run replacement reports `ok: true` for a backend that did not start

Code: `scripts/lab/link-fault.sh:154-174` (stale detection sets `stale=1` but removes nothing), `:224-240` (`lab_wait_for_file manifest.json` and `lab_wait_for_socket control.sock` return immediately on the *old* files), `scripts/lab/lib/lab_backend.py:508-511` (`_bind_socket` refuses an existing socket path with `run_dir_in_use`).

Counterexample (`/tmp/opencode/Fable-LP4a-r2/up3.json`): SIGKILL the supervisor (leaves `control.sock`, `manifest.json`, `.backend.lock`), then `up --run-dir <same>`. Result in 109 ms: `{"ok": true, "replaced_stale_run": true, "supervisor": {"pid": <dead pid>}, "run_token": "<old token>"}`. Meanwhile `backend.log`/`up-error.json`: `run_dir_in_use: a control socket already exists`. No backend is running; `status` -> `Connection refused` / `backend_unavailable`. A runner that retries after a crash gets a success with a dead manifest, which is the F2 failure mode again on the recovery path.

Fix: in the stale branch, after the start-time-token check proves the supervisor dead, remove `manifest.json`, `control.sock` and `up-error.json` (under the `flock` on `.backend.lock`, which the CLI can take briefly with `flock -n`) **before** launching; then wait for the *new* manifest (compare `supervisor.pid` != old pid, or wait on `up-error.json` as well and return its error with exit 1). In the backend, treat an existing socket path whose connect yields `ECONNREFUSED` while holding the flock as stale and unlink it.

Test: `T12b_stale_run_is_replaced`: SIGKILL supervisor, `up` again; must return a manifest with a new `supervisor.pid`/`run_token`, `status` must answer, and the token-process count must be 1 supervisor + holders.

### N2 (blocking, small) - `--leave-for-adoption` is published as honoured but changes nothing

Code: `lab_backend.py:436, 667, 1618, 1631` (stored and published only); holders still get `PR_SET_PDEATHSIG` (`:263`), `ns_exec.py` sets it on helper and child, `cleanup()` runs in `finally` (`:1667-1668`).

Counterexample (`/tmp/opencode/Fable-LP4a-r3`): `up --leave-for-adoption` -> manifest `limits.leave_for_adoption: true`; SIGKILL supervisor -> 0 holders, 0 fixtures, 0 token processes survive. The manifest field is false, and §8.1 step 6 ("leave processes and resources in place for adoption") is exactly what a consumer would read into it.

Fix: either implement it (skip `set_pdeathsig()` for holders and pass `--no-pdeathsig` to `ns_exec.py` when the flag is set; keep `cleanup()` on graceful stop only) or reject the flag with `not_implemented` and publish `leave_for_adoption: false, adoption_model: "pdeathsig"`. Do not publish an intent the process does not enforce. Fable's answer to question 12.1 stands: runner owns lifetime, backend supports both and records which; until both exist, record `pdeathsig` only.

Test: with the flag, SIGKILL supervisor -> holders and children alive, manifest still valid; without it -> all gone (current behaviour).

### N3 (should fix before merge) - IPv6 route check cannot flag a non-link-local route

Code: `lab_backend.py:1093` - `if not (dst or "").startswith("fe80:") and not dst:` is true only for an empty `dst`, which the loop already skipped; a global IPv6 route is therefore never `foreign`.

Counterexample (`/tmp/opencode/Fable-LP4a-r3`): inside owned `alpha`, `ip -6 route add 2001:db8::/64 dev <alpha iface>` and `ip route add 10.99.0.0/24 dev <same>`; `no-bypass-check` reports **one** violation (`unattached_route`, family 4). The IPv6 route passes. The published scope says "IPv4 and IPv6 routing of the lab namespaces".

Fix: `if dst and not dst.startswith("fe80:"): foreign.append(route)` (keep the `dev` ownership check for `fe80::/64`); also treat `ff00::/8` multicast routes explicitly. Test: the two-route probe above must yield two violations.

### N4 (low) - evidence paths ignore capture truncation

Code: `lab_backend.py:934-949, 1000-1016, 1185-1199` use `res.ok` and `_parse_json(res.stdout)` without checking `res.truncated_out`. A tool output over 1 MiB would parse as `None`/`[]`, sum to zero and still report `complete: true` or an empty table list. Not reachable at lab scale (rulesets are a few KiB) but it is the one remaining "truncated -> false complete" path. Fix: in `counters`, `_routes`, `_addresses`, `_table_exists`, `_verify_partition` and the ruleset listing, treat `truncated_out` as a tool error (`evidence_unavailable` / `complete: false`).

Related: `exec` with 3 MiB of stdout is capped at 1 MiB by the backend, but the JSON escaping of the payload exceeds `lab_sock.py`'s 2 MiB response cap, so the CLI reports `response too large` and the caller sees neither the data nor `stdout_truncated`. Lower the backend capture cap (256 KiB) or raise the client cap above the worst-case escaped size.

### N5 (low) - partial apply/heal leaves unrecorded half-state

Code: `lab_backend.py:842-913`. If side A succeeds and side B raises `policy_apply_failed`, `link.policy` is left at the previous value (e.g. `normal` with A's drop table installed, or `partition` with A's table deleted) and the manifest says so. The error is honest; the recorded state is not. Fix: on failure set `link.policy = {"type": "partial", "requested": policy, "failed_side": ep_name}` and write the manifest, or roll back side A and verify before re-raising. Not reproduced (requires inducing a tool failure); code-path finding.

### N6 (low) - `no-bypass-check` during a fault reports the fault as a violation

Observed (`/tmp/opencode/Fable-LP4a-r2`): with `ab` partitioned, `no-bypass-check` returns `ok: false` with four `link_address_unreachable` violations. The adjacency cross-check assumes a healthy link. The backend knows `link.policy`; for a link not in `normal` policy the adjacent probe should be reported as `link_faulted` (informational) rather than a violation, or the check should state in `criterion` that it is only valid in the all-normal state. `run.sh` already heals before T06, so the suite is unaffected.

### N7 (low) - control-plane saturation and small hygiene items

- 10 concurrent `exec sleep 8` -> 8 run, the rest (including `status`) wait 5 s and fail `busy`; `down` is exempt (good). F5 holds up to 8 concurrent long commands; reserve one or two slots for `apply`/`heal`/`status`/`counters`, or give `exec` its own pool.
- `ns_exec.py:76-77`: exec failure errno is read with `ctypes.get_errno()`, which `os.execvp` never sets; observed detail `Key has expired (errno 127)` for ENOENT. Catch `OSError as exc` and report `exc.errno`.
- `lab_backend.py:433` `_state_lock` is defined and never used; `write_manifest()` is called from worker threads and the spawner thread with one shared `manifest.json.tmp`. 15 rounds of concurrent apply/heal/spawn did not corrupt the manifest here, but the race exists; take `_state_lock` around `write_manifest()` and the `children`/`links` mutations.
- `terminate_children` checks `wrapper_start_time_token`, which `spawn` never records (`:1283-1293`), so the wrapper branch is dead code; the wrapper's stdout/stderr pipes are never closed or reaped (2 fds and a zombie per spawn until exit). Record the token or drop the branch; close the pipes after the status line.
- `serve()` appends every worker thread to `self._workers` forever (`:1555`); prune finished threads.
- A failed second backend writes `up-error.json` into the live run's directory (`:1655`); harmless but misleading next to a valid manifest; name it by the failed token or remove it on the CLI's live-refusal path.

## Nix

`nix` is present but `flake.nix` still lacks `iproute2`/`nftables`/`util-linux`/`procps`; the candidate's attempt to realise a temporary shell failed on source downloads (`download.savannah.gnu.org` timeout). All results above are host-tool results. The Nix merge gate (section 11 of the candidate report) is **open, not passed**, and this review does not claim otherwise.

## Required for APPROVE

N1 (stale replacement must start a new backend or fail), N2 (make `leave_for_adoption` true or refuse it), N3 (IPv6 foreign-route check), each with the named regression; N4-N7 fixed or listed as limits in section 7 of the report. Re-snapshot and re-hash after the rework. No other blocker remains from the first round.

---

# Third review: N1-N3 rework and folded low items

Date: 2026-11-06
Candidate: same worktree, base `b0ab4ca34fda07c61a6335d05f598eaacc69678c`, 12 uncommitted files
Snapshot: `/tmp/opencode/lab-lp4a-review-manifest.json`, recipe `sha256(compact sorted JSON of {base, files[path, sha256, mode]})` = `bce931514318110699a5aeb3ae0bdebd8d9204cc7e07b429b5961b9e9977036e` (recomputed: match; all 12 SHA-256 and Git modes verified against the tree)
Candidate report: `docs/reviews/2026-11-05-lab-lp4a-implementation.md` (1062 lines); artifacts `LP4a-final3` and `LP4a-frozen` (28/28, exit 0) and `LP4a-frozen-relay` (6 passed, `T02_netns_suite` skipped, `R04` inconclusive) checked and consistent with the report. Scope of this pass: N1-N3, the folded N4-N7 items, and regressions. The first- and second-round positives were spot-checked, not re-run five times.

## Verdict

**APPROVE (bounded precursor)** for snapshot `bce931514318110699a5aeb3ae0bdebd8d9204cc7e07b429b5961b9e9977036e`.

Approval is for exactly what the candidate claims: the network primitives of the §8.3 link-fault backend (unprivileged user+net namespaces by PID handle, per-link veth with explicit `/30` addresses and no default route, family-agnostic interface-scoped nft drop applied atomically, `tc netem`, capability preflight, IPv4+IPv6 no-bypass check with forwarding assertion, start-time-token-guarded ownership and teardown) proven with TCP fixtures. It is **not** approval of any LP-4 scenario: no full node, P2P link, Ark stub, mining, reorg, retarget or accounting claim is made by the candidate or accepted here. The conditions and open gates are listed at the end and must be carried into the LP-4 integration packet.

## Independent verification (`/tmp/opencode/Fable-LP4a-r5`, host tools, torn down afterwards; host clean: no lab processes, `/run/netns` empty, no lab interfaces/addresses/routes, host `ip_forward` unchanged)

- **N1 fixed.** Live duplicate `up` -> `run_dir_in_use`, `fresh_dir_required: false`. SIGKILL the supervisor (stale `control.sock`, `manifest.json`, `.backend.lock` left) then `up` on the same dir -> exit 1, `stale_run_dir`, `fresh_dir_required: true`, `supervisor.alive: false`; nothing deleted, no backend started, no stale manifest echoed. A successful `up` now requires the manifest to carry the **new** `run_token` and a live `ping` (`control_handshake: true`). The internal path (direct `lab_backend.py` on a stale dir) takes the `flock`, proves the old socket dead by connect, replaces it, and publishes its own token; a CLI `up` against that live socket is refused. The CLI and the backend guarantees do not contradict: the CLI never reuses a non-empty dir; the backend only ever replaces a socket that does not answer while it holds the exclusive lock.
- **N2 fixed.** `--leave-for-adoption` -> `not_implemented` with the model spelled out, run dir left empty. Manifest `limits`: `leave_for_adoption: false`, `adoption_model: "pdeathsig"`, `supports_survive_supervisor: false`; `unsupported_requests` carries the same. SIGKILL still removes holders and fixtures (consistent with what is published).
- **N3 fixed.** Injected `10.99.0.0/24` and `2001:db8::/64` on an owned interface -> two `unattached_route` violations (family 4 and 6); after removal the check is clean (58 checks, 0 violations). Only kernel `fe80::/64` routes on owned interfaces are allowed.
- **F1/F3 still hold.** IPv6 link-local dial times out during `partition`; 200 connects during 8 reapplies -> 0 succeeded; one table during the fault, zero after `heal`.
- **N4.** Capture cap 256 KiB, `stdout_truncated: true` surfaced to the CLI (3 MiB `exec` readable); `_truncated()` feeds evidence paths; client cap 8 MiB.
- **N5.** `_apply_policy_locked` records `{"type": "partial", "requested", "failed_side", "error"}` and rewrites the manifest before re-raising (code read; not inducible here).
- **N6.** `no-bypass-check` during a partition -> `ok: true`, 0 violations, 8 checks marked `link_faulted` (informational), criterion text updated.
- **N7.** Separate admission: 8 `exec`/`spawn` slots, 4 control slots; with 10 concurrent `exec sleep 6`, `status` answered in 67 ms and `apply` in 93 ms (2 execs correctly `busy`). `ns_exec` now reports the real errno (`No such file or directory (errno 2)`). `write_manifest` is under `_state_lock` with a per-thread temp name. Failed-start diagnostics are written per token (`up-error-<token>.json`).

## Nix

Unchanged status, honestly recorded by the candidate: the locked `nixpkgs` input resolves offline (`26.11.20261101`), but realising `iproute2`/`nftables`/`util-linux`/`procps` needs bootstrap downloads this host cannot make. All results in this review and in the candidate's artifacts are **host-tool** results (util-linux 2.37.2, iproute2 5.15.0, nftables 1.0.2, python 3.10). The devShell additions and a run of `testing/lab-isolation/run.sh` from that shell remain an **external merge gate**; this approval does not mark it passed.

## Conditions carried into LP-4 integration (binding on the next packet, not on this one)

1. **Addresses.** Backend-allocated `10.250.0.0/16` per link is acceptable only while the manifest publishes it; before an LP-4 scenario asserts an endpoint address, the allocation must be copied into the runner's run manifest (or a topology plan) so assertions are against published, not assumed, values.
2. **Lifetime.** The precursor model is `pdeathsig` only (`supports_survive_supervisor: false`). §8.1 adoption is not provided; a runner that needs resources to survive its own crash must own the holder lifetime itself or extend the backend with a tested survive mode. Idle/lifetime budgets (3600 s / 7200 s, `0` disables, live children count as activity) are published and must be sized by the runner for long runs.
3. **Guarantee scope.** `sustained_partition` = all traffic on the link interface, both families, until healed (ARP is outside the `inet` family and is not claimed). `no_bypass_routing_enforced` = IPv4+IPv6 routing of the lab namespaces with forwarding disabled, validated by probes only in the all-`normal` state (faulted links are reported, not asserted). Any LP-4 scenario must cite these scopes, not a stronger paraphrase.
4. **Integration gates (candidate section 10, unchanged):** two pools with separate Redis/Postgres; a real pool reconnect loop failing for the declared duration with the pool's own logs; node P2P listeners inside namespaces; the Ark stub links and Ark-specific pending/failed state; retarget boundary coverage; accounting/reorg/statistics; `scenarios/lab/*.json` and `scripts/lab/ci.sh`.
5. **Residual low items** (may be carried as stated limits): `inet`-family drop leaves ARP visible on the link (no data path); `exec` saturation beyond 8 concurrent commands returns `busy` rather than queueing; relay backend remains `isolation=relay` smoke-only with partition/no-bypass `inconclusive`.
