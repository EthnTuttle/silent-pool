# SPEC changelog fragment: contextual native difficulty

Prepend this entry when integrating the contextual T2/T1 consumer amendment:

```yaml
- "2026-11-09: contextual native difficulty consumer amendment: required bound min-difficulty, spacing, interval and overlay facts; shared genesis-anchored raw-header verifier applies the node's exact gap/backtracking/boundary/inheritance/overlay order to GBT and observer/completion. No-periodic-retarget does not imply constant bits. Node policy and 40 H/s remain unchanged; periodic retargeting stays unsupported. See docs/decisions/2026-11-09-contextual-native-difficulty.md."
```

The T2 worktree already contains this entry. Do not add it twice. Control v1,
journal 3 and key derivation 2 are unchanged. Required private actor policy
fields have no serde defaults; old incomplete actor configurations refuse.
