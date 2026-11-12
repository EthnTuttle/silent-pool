# DeepSeek V3.2 Review of docs/REGTEST-LAB.md

**Reviewer:** DeepSeek V3.2 (Amazon Bedrock)
**Plan SHA256:** 444060b2c1c7d377960116032dc88af114c0dbc43741a8c719b33188e8b99098
**Proposal Date:** 2026-11-05
**Review Date:** 2026-11-05
**Baseline:** silent-pool HEAD `b0ab4ca34fda07c61a6335d05f598eaacc69678c`

## Overall Verdict

The regtest lab plan presents a coherent vision for a real SV2 CPU mining test network, but contains **critical unsupported assumptions** about node retargeting capabilities, **significant implementation gaps** in SV2 protocol handling, and **underdefined failure modes** for distributed scenarios. The plan's architecture is sound, but its most complex claims rely on unimplemented node features.

**Overall risk:** High. P0 prerequisites for retargeting realism and share unit design are not resolved, making the core "self-adjusting network" goal currently unattainable.

## Ranked Findings (Highest Severity First)

### 1. Critical: Retargeting Capability Assumed Not Implemented

**Severity:** CRITICAL (blocks P0, P3, P4)
**Affected:** §3.2, §8 (P0), §4, §5
**Evidence:**
- Plan acknowledges (§3.2): "inspected rbitcoin regtest profile disables periodic retargeting"
- States: "an exposed, validated multi-process regtest configuration" is missing
- §3.2: "Retargeting experiment... requires an explicitly supported and tested rbitcoin consensus profile"
- Yet P4 assumes "Run sustained hash increases/decreases across adjustment boundaries"

**Analysis:** The plan correctly identifies the gap but packages retargeting as an implementation prerequisite rather than a fundamental plan defect. The `--regtest-pow-bits` parameter provides static PoW baseline only. Without node-side retargeting support:

1. "Self-adjusting network" claim is false for any experiment
2. Difficulty-response validation (§3.1) cannot be tested
3. P4 scenarios become meaningless

**Suggested Fix:** Split plan into two tracks:
1. **PoW baseline lab** - Static difficulty, valid for protocol and share testing
2. **Retargeting lab** - Requires rbitcoin feature implementation first

Add explicit preflight check: "if profile `--regtest-retarget-capable` not available, fail with 'unsupported capability'"

**Falsifiable Test:** Attempt to configure rbitcoin with `--regtest-retarget-enabled` and observe node startup failure. Verify via block timestamps that difficulty remains static after 2016 blocks with varying hash power.

### 2. Critical: Share Target/Unit Design Unresolved Contradicts Policy

**Severity:** CRITICAL (blocks P1)
**Affected:** §3.3, references decision 2026-11-02-share-target-policy.md
**Evidence:**
- Plan §3.3: "CPU-scale share targets through the real SV2 target policy"
- Decision enforces: `share_difficulty` MUST be > 0 integer, defaults to 1
- Plan §3.3: "exact 256-bit targets and a defined integer/fixed-point work representation"
- Decision: Credits `d = min(share_difficulty, max(1, round(network_difficulty(bits))))`
- On regtest: `network_difficulty << 1`, so `d = 1` always

**Analysis:** The plan proposes fractional/sub-integer work units but current policy rounds all sub-1 difficulties to 1. For CPU-scale mining on regtest (difficulty << 1), every share earns credit 1. Yet plan wants "frequent non-block shares" (§3.3). This contradicts:

1. Window semantics `N x d` where `d=1` results in window size = N shares
2. Policy prevents crediting fractional work
3. "CPU-scale" targets require violating current policy

**Suggested Fix:** Before P1, decide: (a) amend policy decision to allow fractional credit, or (b) accept that CPU miners earn 1 credit per share regardless of actual work. Document choice explicitly in plan.

**Falsifiable Test:** Configure `share_difficulty = 1000` (expecting 1/1000 work). Observe credit remains 1 on regtest due to `max(1, round(network_difficulty))` clamp.

### 3. High: SV2 Protocol Gaps Block Genuine Mining

**Severity:** HIGH (blocks P1)
**Affected:** §5.1, "Existing SV2 gaps to resolve"
**Evidence:** Plan §5.1 lists 6 gaps:
1. "server announces target on channel open but does not emit `SetTarget`"
2. "early opens can get an all-ones target"
3. "connection holds one channel with fixed channel ID"
4. "client helpers don't fully correlate future jobs and prevhash activation"
5. "Noise authority pinning not wired"
6. "ingress success precedes front-side acceptance"

**Analysis:** These are not minor gaps but **fundamental protocol correctness issues**. Without `SetTarget`, miners cannot receive updated targets. All-ones target violates SV2 spec. Single-channel limits topology scaling. Gaps 4-6 affect correctness of share accounting.

**Suggested Fix:** Elevate from "gaps to resolve" to **P0 prerequisites**. Each gap needs:
- Specific test verifying SV2 spec compliance
- Reference to upstream SV2 spec section
- Proof of fix via protocol-compliance test vectors

**Falsifiable Test:** Implement test miner that validates `OpenExtendedMiningChannelSuccess.target != 0xff..ff` and expects `SetTarget` after network difficulty change.

### 4. High: Topology Assumes Partition Control Without Mechanism

**Severity:** HIGH (blocks P3)
**Affected:** §4, §5.2 commands
**Evidence:**
- Plan §4: "Control all relevant links: miner to pool, pool to template provider/chain RPC, node P2P"
- §5.2 commands include: "Partition/heal selected links; add latency"
- But no mechanism described for process-level network isolation
- Current test harnesses use fixed ports, no network namespace or firewall rules

**Analysis:** Commands assume precise link control but plan offers only "unique ports" (§5) and "containers or network namespaces can provide stronger... control later". Without isolation mechanism:
1. "Partition" command cannot be implemented
2. Multi-node experiments risk accidental communication
3. Independent pools might share Redis/Postgres inadvertently

**Suggested Fix:** Specify concrete isolation layer for P3:
- Network namespaces with veth pairs per link
- iptables/nftables rulesets for partition simulation
- Or require container runtime (Docker/podman) with network isolation

**Falsifiable Test:** Command `partition minerA-poolA` should drop 100% of TCP packets between those endpoints while other links remain functional.

### 5. Medium: Event Ordering and Causal Consistency Unspecified

**Severity:** MEDIUM (blocks assertions)
**Affected:** §6 "Observations, UI, and evidence"
**Evidence:**
- §6: "collector sequence is an ingestion order, not proof of a global causal order"
- But §5.2: "trigger such as 'after node B observes height H' names its node"
- §6: "assertions requiring complete evidence fail or become inconclusive if required data was lost"

**Analysis:** The plan acknowledges distributed observation problem but doesn't specify solution. For "after node B observes height H":
1. How does harness know node B observed it? (RPC polling interval?)
2. What if node C observed earlier/later?
3. Causal barriers need timeout policy

**Suggested Fix:** Define observation contracts:
- Per-node observation: RPC poll interval + allowable staleness
- Global predicate: quorum of N nodes observed X
- Causal guarantee: "after A then B" vs "A and B concurrently"

**Falsifiable Test:** Implement "after 2/3 nodes reach height H" trigger and verify it fires even if one node is partitioned.

### 6. Medium: Economic Warmup and Scale Assumptions Underdefined

**Severity:** MEDIUM (affects P4 validity)
**Affected:** §7 "Time, economics, reproducibility, and scale"
**Evidence:**
- §7: "rbitcoin currently starts with 50 BTC regtest subsidy and halves every 150 blocks"
- §7: "Long economic runs need a declared reward schedule and warmup state"
- But P4 includes "economic scenario suite" without defining:
  - Initial float sizes
  - Ark limits
  - Confirmation policy assumptions

**Analysis:** Economic scenarios require consistent initial conditions. Default rbitcoin subsidy schedule (halving every 150 blocks) vs Bitcoin mainnet (210,000 blocks) creates different economic dynamics. Without declaration:
1. Results not comparable across runs
2. Float exhaustion/scarcity timing unpredictable
3. Ark rail capacity assumptions unspecified

**Suggested Fix:** Add mandatory economic profile to scenario definition:
- Subsidy schedule override (if any)
- Initial pool float (sats)
- Ark daily limits
- Coinbase maturity depth

**Falsifiable Test:** Run identical hash power with 50 BTC vs 6.25 BTC subsidy, observe different Ark usage patterns.

### 7. Low: Web UI Decoupling Assumption Untested

**Severity:** LOW (minor risk)
**Affected:** §5, §6
**Evidence:**
- §1: "Browser presence never controls run lifetime"
- §5: "HTTP service and web UI" component
- But: What if browser crashes? Refreshes? Loses SSE connection?
- No recovery/resync protocol defined

**Analysis:** Plan correctly prioritizes headless operation but doesn't specify UI reconnection semantics. "Browser presence never controls run lifetime" is a requirement, not an implementation. Need to define:
- UI reconnect gets consistent snapshot + tail replay
- Command deduplication across reconnections
- Observation buffering limits

**Suggested Fix:** Specify in §5.2 that command IDs are session-persistent (survive UI disconnect/reconnect) and command preconditions are rechecked on reissue.

**Falsifiable Test:** Start run via CLI, attach browser, send command, kill browser, restart browser, ensure command not duplicated.

### 8. Low: Artifact Retention Privacy Boundary Weak

**Severity:** LOW (compliance issue)
**Affected:** §6
**Evidence:**
- §6: "No `sp_id`, `B_scan`, `B_spend`, `P_i`, block hash or snapshot is added to Ark-role inputs/streams"
- But also: "Harness artifact retention is separate from the system's scrub behavior"
- Artifacts may contain `sp_id` in slice credits, `B_scan` in identity events

**Analysis:** While "cannot be used as evidence of production privacy guarantees" (§6), retaining privacy-sensitive data in artifacts creates compliance liability. SPEC §11.3 is load-bearing for production but lab artifacts could leak test vectors.

**Suggested Fix:** Define artifact scrubbing policy:
- Option A: Redact privacy fields from persisted artifacts
- Option B: Generate disposable test identities only
- Option C: Accept risk for test environment only

**Falsifiable Test:** Check if artifact JSON contains `sp_id` or `B_scan` values. If yes, verify they are test-only disposable values.

## Open Questions Requiring Resolution

1. **Node capability timeline:** When will rbitcoin expose retargeting regtest profile? Can this project implement it as a patch?
2. **Share unit design decision:** Awaiting outcome of "generalized fractional difficulty vs regtest-specific work scale" debate (§3.3).
3. **SV2 completeness:** Which gaps can be worked around vs must be fixed before P1?
4. **Isolation mechanism choice:** Network namespaces, containers, or simulated links?
5. **Economic profile defaults:** What values yield meaningful tests without excessive runtime?

## Recommended Minimal Plan Amendments

### Immediate (Before Implementation):

1. **Split capability declaration:** Add "Capabilities" section distinguishing:
   - **PoW Baseline**: Static difficulty, `--regtest-pow-bits`
   - **Retargeting**: Requires rbitcoin feature (TBD)
   - **SV2 Complete**: All gaps resolved
   - **SV2 Basic**: Current implementation level

2. **Add preflight validation:** Runner validates available capabilities against scenario requirements, fails early with clear error.

3. **Clarify share unit conflict:** State explicitly that current policy `d = max(1, round(network_difficulty))` means CPU-scale shares earn 1 credit on regtest unless policy amended.

### Before P1:

4. **Resolve SV2 gaps:** Convert §5.1 gap list to concrete acceptance criteria with protocol-compliance tests.

5. **Define isolation mechanism:** Choose and specify network namespace setup for P3.

### Documentation Only:

6. **Add assumption registry:** Table of plan assumptions vs current evidence, updated as implementation progresses.

## Conclusion

The regtest lab plan is ambitious and architecturally sound but currently **overpromises on retargeting realism** and **understates SV2 protocol gaps**. The strongest finding is that the "self-adjusting network" goal requires node capabilities not yet available. Recommendations:

1. **Tier the plan** into achievable phases: PoW baseline first, retargeting when available.
2. **Confront share unit contradiction** between policy and CPU-scale intent.
3. **Elevate SV2 protocol gaps** to blocking issues.
4. **Specify concrete isolation** for distributed experiments.

With these amendments, the plan provides a valuable roadmap for building a genuine SV2 CPU mining testbed that can evolve as underlying capabilities mature.

## Clarification and Disposition

### Plan vs Code Distinction Clarifications

Several points in my original review mischaracterized the plan's position on existing requirements:

1. **Retargeting separation already specified**: The plan §3.2 already distinguishes "PoW baseline" vs "retargeting experiment" and explicitly states unsupported retarget preflight fails. My characterization as "absent plan requirement" was incorrect - the plan correctly identifies this as an implementation prerequisite. **Clarification**: The plan's handling is correct; my criticism should have focused on severity of this prerequisite being unmet.

2. **Share unit decision explicitly P0**: The plan §3.3 and P0 explicitly require policy decision before P1. My finding restated an acknowledged prerequisite rather than identifying an absent requirement.

3. **UI reconnection already specified**: Plan §6 already specifies "consistent snapshot/cursor or replays the retained tail" for reconnections. My concern about undefined recovery was overstated.

4. **Command ID deduplication**: Plan §5.2 already states "Retries of the same command ID do not accidentally add a second miner." My suggested fix duplicates existing specification.

### Corrections to Evidence Claims

1. **`--regtest-retarget-*` flags**: These were illustrative examples, not claimed existing CLI options. The capability test mechanism remains unspecified but flags are placeholders, not evidence of missing option discovery.

2. **`share_difficulty=1000` misunderstanding**: I incorrectly described this as "1/1000 work." Actually, `share_difficulty=1000` means 1000x harder than difficulty-1 target, not fractional work. The core issue remains: on regtest with `network_difficulty << 1`, the `max(1, round(network_difficulty))` clamp still forces `d = 1`.

3. **All-ones target SV2 spec violation**: I overstated "violates SV2 spec" without upstream evidence. The main issue per plan is announced-vs-validated mismatch - a channel opened before job arrival receiving `0xff..ff` target that doesn't match later job validation target, not necessarily numeric illegality.

4. **Single channel limitation**: Plan explicitly designs initial topology as "one independent connection/channel per identity" (§5.1). Single channel per connection doesn't block N miners over N connections. My scaling concern was misplaced.

5. **Privacy compliance requirement**: I introduced "compliance liability" without basis. The correct assessment is whether fixture observation preserves actual Ark input boundary and secret-erasure requirements (§9 Q9), not external compliance.

### Severity Re-evaluation with Plan-vs-Code Distinction

Original CRITICAL/HIGH findings should be recategorized as:

- **Finding 1** (Retargeting): Severity remains HIGH (not CRITICAL) because plan acknowledges gap but P4 scenarios depend on it.
- **Finding 2** (Share units): Severity MEDIUM - plan correctly marks as P0 decision, not implementation blocker.
- **Finding 3** (SV2 gaps): Severity remains HIGH - plan lists as "gaps to resolve" but these affect protocol correctness.
- **Finding 4** (Isolation): Severity MEDIUM - plan acknowledges need, defers mechanism choice.
- **Finding 5** (Observation): Severity LOW - plan addresses distributed observation challenge.
- **Finding 6** (Economics): Severity MEDIUM - useful concrete proposal for scenario definitions.
- **Finding 7** (UI): Severity LOW - plan already specifies reconnection.
- **Finding 8** (Privacy): Severity LOW - correctly assesses fixture boundary.

### Useful Concrete Proposals Retained

Despite overstatements, these concrete proposals remain useful:

1. **Isolation mechanism specification** for P3 (network namespaces, iptables, containers)
2. **Observation contracts** for distributed triggers (quorum semantics, timeout policies)
3. **Economic profile augmentation** with explicit float sizes, Ark limits, confirmation policy
4. **Capability registry** tracking assumption vs evidence

### Final Disposition

The plan correctly identifies prerequisites and gaps. My review overstated "absent requirements" but correctly highlights:
- Retargeting dependency makes P4 scenarios currently unimplementable
- SV2 protocol gaps affect mining correctness
- Isolation mechanism needs concrete specification before P3
- Economic scenario definitions need augmentation for reproducibility

The plan provides adequate framework; implementation must resolve the identified prerequisites while maintaining the architectural vision.
