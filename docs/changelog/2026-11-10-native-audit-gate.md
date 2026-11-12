# SPEC changelog fragment - native audit gate

Serial integration entry (do not overwrite unrelated SPEC frontmatter):

```yaml
- "2026-11-10: full lab native SV2 candidates require source-feed/2 committed wallet audit evidence before JobActivated/worker publication. The new public spc.native-job-gate/1 codec binds actual ReceivedJob, SP identity, session/candidate epoch, fresh nonce, wallet lifecycle instance and actual controller peer/config generation; CPU holds public material only. Signed underfilled facts remain explicit, unknown/expired/replayed proofs grant no work, and funding/registration barriers remain. See docs/decisions/2026-11-10-native-audit-gate.md and docs/NATIVE-JOB-GATE.md."
```

The required per-connection SourceAuditConfig is a deliberate private config
amendment. Explicit unaudited fixtures are not audited campaign evidence.
