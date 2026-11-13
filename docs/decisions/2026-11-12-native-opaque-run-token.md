# Native wallet opaque run-token binding

Date: 2026-11-12
Status: source prepared; primary integration and focused execution pending

## Authority

Parent-confirmed fresh Runtime startup counterexample: traffic hashes the
run/owner marker before Supervisor export, whereas the wallet incorrectly
required a literal run-ID prefix. This corrects the private implementation
contract of SPEC §3.6 and the existing wallet service decision (§11.5).
The serial owner applies the accompanying SPEC frontmatter fragment.

## Decision

- Preserve Runtime's opaque marker generation and Supervisor export. The
  renderer hashes the actual `Supervisor::run_token()` before wallet spawn.
  Required private `ark.native_audit.run_token_hash` is 64 lowercase hex
  characters encoding a nonzero SHA256. It is generated alongside run, pool,
  front, identity and collector bindings and is covered by the loaded config
  digest. It is not a user/browser profile field and has no default.
- Both wallet self-startup and kernel-observed CPU authorization require one
  nonempty environment marker of at most 1024 bytes, with that exact expected
  digest. The proc environment is bounded at 1 MiB and read directly into
  zeroizing storage. No literal prefix, marker-shape guess or missing-field
  fallback authorizes a process.
- Startup takes the expectation from loaded private config. CPU authorization
  takes it from the published wallet instance and then requires full controller
  PeerRegistry equality. PID/start/UID/boot/executable/config/network-namespace,
  endpoint and request lifecycle checks remain mandatory.
- Before rewriting existing wallet config, and on readiness/bind/live attach,
  validate its complete native scope against the configured profile and actual
  recorded marker. Missing records or expected fields and contradictions fail;
  attach never remints a marker. A stale instance must retain the exact config
  path/hash and run-token digest before cleanup/republication is allowed.
- The unaudited profile (no native config) retains its existing behavior. The
  public codec, SourceFeed v2, native retention, N1/N2 and payout/Ark rules are
  unchanged. No key material is added to CPU configuration or evidence.

## Verification boundary

The joined test uses real engine generation/render/Supervisor export and the
real loaded-agent/native-service startup path. Its infrastructure adapters and
public profile are explicit fixtures. Full Runtime/captaind startup, controller
attach and funded acceptance remain primary joined gates. Earlier literal-token
component passes do not cover this counterexample and are not relabelled.
