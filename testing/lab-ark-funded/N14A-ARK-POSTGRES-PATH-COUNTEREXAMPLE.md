# n14a: Ark PostgreSQL Unix socket path exceeded the platform bound

The corrected 98f/r23 n14a run was executed once and exited 2. Runtime failed
before bootstrap at ark-postgres/local-ark readiness. Its actual log reports
the socket path ending in private/ark-server-local-ark/socket/.s.PGSQL.43737
was too long: 112 bytes, versus the 107-byte Linux Unix socket pathname bound.
PostgreSQL had briefly bound its TCP port, then exited 1 because it could not
create a Unix socket. No node consensus, token, Git or Ark protocol guard failed.

The preparation check had covered the collector and native wallet sockets;
the latter fit at 106 bytes. It missed the longer Ark PostgreSQL layout. This
is an owned harness preflight defect. The fix checks every configured local
Ark PostgreSQL and pool PostgreSQL socket using a worst-case five-digit port,
alongside both existing wallet/collector checks. Preparation checks them before
creating keys, and shared binding checks the actual selected execution root
before QA.start, so changing --out cannot bypass the path check.

The failed n14a root and its inputs are preserved. Fresh candidate root
/home/ethan/.local/share/silent-pool/r/b14 has an Ark PostgreSQL maximum of
104 bytes and pool PostgreSQL maxima of 94 bytes. It has a new run ID, new
attestation keys and a new input/scenario seal; the selected 3cf bundle,
corrected 98f applications and r23 QA remain unchanged. Actual r23/shared
preflight passed without starting a Runtime.

The sole n14a reap returned 0 and listed the six launched node/PostgreSQL/Redis
actors exited. Scoped post-exit checks found all eight recorded identities
absent, all 25 checked planned TCP ports free, and protected external feedback
PID 27801/start 199982 unchanged. There was no second reap, signal or UID-wide
scan. Startup coverage was incomplete, so the overall cleanup and replayed
component verdict remain INCONCLUSIVE; this partial cleanup success is not
funding or whole-lab acceptance.

Durable raw run: /home/ethan/.local/share/silent-pool/ap5-runs/n14a.
Launch/source/replay evidence: durable release-98f/n14a-launch.
No application rebuild, source-contract waiver, server modification or
replacement receipt is needed for the shorter-root correction.
