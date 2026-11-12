# Live Lab hub: preimplementation plan review

Date: 2026-11-07.
Status: plan approved independently by two TFF models; implementation and
Fable checks are still required. This is not approval of the product mockups.

The user requested a simpler live experiment workspace with a hosted page
explaining the actual lab roles/daemons and linking to their interfaces.

## Agreed scope

- Serve a small read-only `/lab` directory from the existing loopback `spc-lab`
  server, sharing its lifetime and leaving the existing controls page intact.
- Project actual scenario/process/resource/observation state into an allowlisted
  inventory, rather than serving the private manifest.
- Explain logical roles versus processes, purpose, state/freshness and whether
  each interface is a browser UI, JSON API, daemon-control protocol or absent.
- Distinguish the real CPU test miner from `spc-agent`, in-process scanner from
  a separate service, and running roles from optional/unimplemented wallet/Ark
  capabilities.
- Add current `docs/LAB-ROLES.md`, with source references and an explicit boundary
  from the partly stale target architecture document.

## GLM 5.3 Flash

Session: `ses_e46bba74bffeM6huh0ZwvSX5Lo`.
Verdict: APPROVE-plan with bounded conditions.

Verified pool API is JSON/metrics only, standard scenarios co-locate
front/payout/api, Ark is absent, spc-agent is not launched by the runtime, and
existing catalog/process records support the proposed inventory.

Conditions: use the existing read-only loopback surface and control plane;
show recorded process states without inventing health; explain the shared
throwaway payout descriptor/no real wallet; cite actual code in the role matrix.

## DeepSeek V4.1 Flash

Session: `ses_e46bba710ffem9tUvHSnoaYLHe`.
Verdict: APPROVE-plan; no blocking clarification.

Conditions: no second server/control plane; no raw argv, private paths,
credential-bearing URLs or identity/key material in the inventory; distinguish
recorded state from observed liveness; show unimplemented/client/operator wallet
capabilities accurately; keep target architecture separate from current runtime.
Test recorded read-only mode, absent services, credential exclusion, and
namespaced endpoints that are not directly browser-accessible.

## Implementation handoff

TFF Kimi `ses_e46b967adffeM95EnJ79kzc2xx` owns the isolated `lab-live-hub` topic.
No large product redesign or wallet functionality is authorized by this plan.
Fable reviews the real hub and its evidence before integration and a new manual
lab session. Existing mockup gallery and user notes remain untouched.
