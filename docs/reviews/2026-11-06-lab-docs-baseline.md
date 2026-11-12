# 2026-11-06 - Lab documentation baseline and hash record

Base: `b0ab4ca34fda07c61a6335d05f598eaacc69678c`

Approved host docs source: `/tmp/opencode/lab-runtime-review-manifest.json`, aggregate `e891f34c69c70c2a6748be1ca2a77a71aba4a63377eda55a43afeb8613b68043`, tested code `d75ab391b0ca1c8ab5b03af27dd5cf45a8a384ef8ac056a1ceeb2b2f456d8684`.

This file records the pre-edit baseline and final hashes for the documentation-only packet. It does not change a decision or code.

## Owned delta

| Path | Base SHA-256 | Final SHA-256 |
|---|---|---|
| `README.md` | `43c8d20f2bf5970425854423f8f23e4b954ca0eab4363d4f78dd2715dbcbe1db` | `211f0bdc6c7ee8a377d918ddf23538c4f3b5d15c47c2fbd8972f5a4e639f7035` |
| `docs/SPEC.md` | `e5a7dc60c7c5dd5f4793c1007837b6de2ae91dfe2d6694c06ffd5759bdd92a73` | `5de914d7e2a38505beafa4410bd41798de5b2d2a65b46144832c3f769edbb788` |
| `docs/WIRE-FORMATS.md` | `57c2fac7832f8d728a2d8198ddeca8fee8891b5956014e476e5b0acdf307746a` | `46a50b1a74610a58691c613acc3248de93ab15424e86c0a94ef3ee3e1b3b2066` |
| `docs/OPERATIONS.md` | `1237e81b249bd12f3fad0f00d8caa000ddc1fd4c0b70d75c4fc98dfbf5df6c21` | `c2c3d6d398af67752064dde1b682035a86d369f75ef5f5279c9b40b9fa7fb232` |
| `docs/share-accounting-ext-deltas.md` | `20210dc540b779449da3d5cd55167ef92b97b6ba18a862f021f7b2ccc4dfdaf6` | `110f54ebbf54b1d00459003682fbca71cf2b7f2da03b3e0b7490dcfe5c217c8a` |
| `docs/REGTEST-LAB-IMPLEMENTATION.md` | `new file` | `ea50b836fa8e36f64eb67cb50dca856319cea527877ff1f9b3fd17845cfe9f76` |
| `docs/decisions/2026-11-02-share-target-policy.md` | `4511a8d0129043868068a2d55891fc979cea9cb214130f2e491dce53f2d88d1a` | `edfcb52a69c1c7489199647633fa487ea717992fdf731e75d2cf22bc01b831a3` |
| `docs/LAB-RUNTIME.md` | `new file` | `5cbdd6fc910d14a8899e4d764f9505bff75c61af9adaa146c0a4935cd1df6c77` |
| `docs/reviews/2026-11-06-lab-docs-implementation.md` | `new file` | `55c728f67c0bbd037d765d674b312aa73ef35ece55c14a174d1a66024d69b0a7` |

`docs/reviews/2026-11-06-lab-docs-baseline.md` (this record) is new; its hash is in the generated manifest.

## Copied approved documentation inputs

These files were copied byte-for-byte from the frozen approved runtime tree and match `/tmp/opencode/lab-runtime-review-manifest.json`.

| Path | SHA-256 | Source |
|---|---|---|
| `docs/REGTEST-LAB-IMPLEMENTATION.md` | `7a031f414ae45f480949cd0859150e0b61a3cf9cf86a1620de582b0695759861` | `lab-lp1` |
| `docs/SPEC.md` | `559548d24ed8cbe50c11b9e288f4bf5db5e68a262b05c9a920c1325a2a486f16` | `integration` |
| `docs/WIRE-FORMATS.md` | `aa75c101de38439e8b2e9c4b7dc2992eed6dda9bb892bf313e270ba6f6b0d595` | `lab-lp1` |
| `docs/changelog/2026-11-05-lab-lp1-integration.md` | `d26ef85d83c535faf69ead04c8c36c6d5e8ad43f9d270b6f1f95402ad710692d` | `lab-lp1` |
| `docs/changelog/2026-11-05-lab-share-work-units.md` | `15167cf60d32f24e11deb35a9a2b04a96b5c3c2fb1a87ae05a966208d941c2ce` | `lab-lp1` |
| `docs/changelog/2026-11-05-sv2-native-conformance.md` | `696e72da17eccb404cac1d3925269aef1477beec5615b583a8024e5167b58fbb` | `lab-lp1` |
| `docs/changelog/2026-11-05-window-network-block-work.md` | `e79b55e20a3675e7501bbf76787bbb37e709d88797990ba3f4760c40d0e1ef4b` | `lab-lp1` |
| `docs/changelog/2026-11-06-lab-runtime-host.md` | `9c9d0732b99c227ec84d6dab7effd972a8e1667be1da66205636b9676f49f749` | `integration` |
| `docs/changelog/2026-11-06-pool-reader-outcome-latency.md` | `b8240a98283c463576850fa74e59a554f0f0a6948c82397c0ede8a1f98ebe661` | `integration` |
| `docs/decisions/2026-11-05-lab-lp1-integration.md` | `9acedb4af53443dd9a82101df2433b704401333505df3567e21992ad983ebfb5` | `lab-lp1` |
| `docs/decisions/2026-11-05-lab-share-work-units.md` | `d8b86f4a09a6283d266496c4dc74312e64965f1397e79a401756d457dd9844b9` | `lab-lp1` |
| `docs/decisions/2026-11-05-sv2-native-conformance.md` | `34361d4b903647935303f3fd200d5f757bd954ae0944625f2f93feb68aec4dfb` | `lab-lp1` |
| `docs/decisions/2026-11-05-window-network-block-work.md` | `f66374426d1ea27697a84db07a9c8bbac02e69314d6a4ee9421c9924a38624d1` | `lab-lp1` |
| `docs/decisions/2026-11-06-lab-runtime-host.md` | `4bea5346088271c49076e61374bbd51a78156a3d76588c51535078a76fc1c1df` | `integration` |
| `docs/decisions/2026-11-06-pool-reader-outcome-latency.md` | `da8846d1260b2fcf725a2e4a4e49be2dadf854d76ee0efc581f63c015a2fef0e` | `integration` |
| `docs/reviews/2026-11-05-lab-l1-implementation.md` | `f0416c3a6b3b2972c366526ad3967aa9d4542c9f52438506c5b32cdc434ddbd5` | `lab-lp1` |
| `docs/reviews/2026-11-05-lab-l3-contracts.md` | `5d00a49fe843a129d267566aae28aeed3ec3b9a69fd58ac14f0830c64a9e5177` | `lab-lp1` |
| `docs/reviews/2026-11-05-lab-live-implementation.md` | `e57df1616a0e817f54dfab5467e2a495c7392029db9fbc6b3b0bb7d4df8d6da8` | `lab-live` |
| `docs/reviews/2026-11-05-lab-lp1-implementation.md` | `0e4799c36aa2ebe115cb37e2d56cf4b7025230de42fd3d1fd4bb64cd3622758f` | `lab-lp1` |
| `docs/reviews/2026-11-05-lab-lp1a-implementation.md` | `b80fa70e1c7ff2ba46c87a3895860d8ded507835389818904274b0c98ec8fcfb` | `lab-lp1` |
| `docs/reviews/2026-11-05-lab-lp2-implementation.md` | `52e4ca4687d8ae80426371abeb32b7e657ff19e98f4cc76344b84076bcf963de` | `lab-lp2` |
| `docs/reviews/2026-11-05-lab-lp3-http-implementation.md` | `a7552915a7c8aea3c172d52a61820563420176214a0e0239a779298c85822a3d` | `lab-lp3-r3` |
| `docs/reviews/2026-11-05-lab-lp3-implementation.md` | `ae9fc1b5ec9060a07f313c34ec1cf74e52155271b6ed1446d9680836c2414ec6` | `lab-lp3-r3` |
| `docs/reviews/2026-11-05-lab-lp4a-implementation.md` | `03815a2984850f7bcf04ada58a9513607c054b573bd308c258fd19138da9a8c1` | `lab-lp4a` |
| `docs/reviews/2026-11-06-lab-runtime-implementation.md` | `73c06da29f5af68b68e740eff79a527ad0b6693ddcaa0121c6612390b8cc0b78` | `integration` |

## Merge order

Apply only this owned delta over the approved inputs, in this order:

1. `docs/SPEC.md`
2. `docs/WIRE-FORMATS.md`
3. `docs/OPERATIONS.md`
4. `docs/share-accounting-ext-deltas.md`
5. `docs/REGTEST-LAB-IMPLEMENTATION.md`
6. `docs/decisions/2026-11-02-share-target-policy.md` (superseded-in-part note)
7. `README.md`
8. `docs/LAB-RUNTIME.md`
9. `docs/reviews/2026-11-06-lab-docs-implementation.md`
10. `docs/reviews/2026-11-06-lab-docs-baseline.md`

## Excluded review histories

The packet does not copy or rewrite main-tree review histories: `2026-11-06-lab-live-fable-review.md`, `2026-11-06-lab-runtime-fable-review.md`, `2026-11-06-lab-runtime-pool-fable-review.md`, `2026-11-06-lab-stale-work-diagnostic.md`, `2026-11-06-lab-topology-fable-review.md`. They are read-only context.

## Manifest recipe

Aggregate = `sha256(json.dumps({"base": base, "files": [{"path", "sha256"}]}, sort_keys=True, separators=(",", ":")))` over all changed/untracked docs and `README.md`; see `/tmp/opencode/lab-docs-review-manifest.json`.
