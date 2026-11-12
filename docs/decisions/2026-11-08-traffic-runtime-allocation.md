# 2026-11-08 - Traffic runtime allocation and execution limits

## Scope

Runtime plan v5 `44daaa2baa41a4b93eb46a9e69dfdcf2bba1e0ce73367e27e83a87ab3a6b3307`,
sections 3.2 and 4.1. This record accompanies an unverified implementation
candidate, not a funded runtime acceptance claim.

## Decisions

- Every actor declares positive network and user-payment wallet counts and a
  bootstrap-host flag. Exactly one host exists. Sorted raw actor ids receive
  cumulative disjoint ranges; sorted identity ids receive global user indices.
- Every actor receives the complete description and independently reconstructs
  the opaque core allocation token. No deserializer is added to that token.
- `history` is exactly `traffic/`. Driver filesystem components are `d-` plus
  unpadded base64url of the raw id; protocol/journal ids retain their raw value.
  Nested private directories and regular files are checked as 0700/0600.
- Driver bootstrap execution limits default to 40 hashes/s, 4,194,304 reserved
  attempts, 3600 seconds and 30 seconds cancellation grace. They are independent
  of ordinary transaction rate and the bootstrap distribution configuration.
- The bootstrap command includes the selected host driver and selected node.
  One declared phase command binds that node as every actor's authority before
  provisioning. User payment scope requires the claim's own identity and an
  explicit validated destination.

## Sections edited

`traffic/spec.rs`, `traffic/commands.rs`, actor configuration, controller
command/access routing, and their tests. The SPEC entry is a serial-merge
fragment; this packet does not edit SPEC frontmatter.
