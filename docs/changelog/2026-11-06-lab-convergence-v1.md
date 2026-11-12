# 2026-11-06 - Native scenario convergence v1

Added the lab-only optional `phase.convergence` operation with bounded timeout,
observation and worker-resume controls, plus typed `topology_phase` and
`topology_convergence` evidence payloads. Existing scenario artifacts without
the field have unchanged control behavior; old string-based records remain
readable. The declaration makes the final pause/re-read and explicit stop/
non-convergence outcomes reviewable instead of relying on a hidden runtime
fallback. There is no pool, payout, SV2, SPc or Ark semantic change.
