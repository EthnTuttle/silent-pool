# Type-32 share-accounting extension: silent-pool deltas

Status: implementation note for SPEC §3.7. Upstream is
`demand-open-source/share-accounting-ext` `extension.md` @ `c661e93` (crate
v0.0.13). This file lists every divergence we intentionally carry.

| Item | Upstream `c661e93` | silent-pool | Why |
|---|---|---|---|
| Activation | `Activate` / `Activate.Success` handshake, with the sv2-spec #95 negotiation still TODO | negotiated through extension `0x0001`; `Activate` is accepted as a no-op for compatibility | #95 has no settled wire format; using `0x0001` keeps one negotiation path |
| Codec base | demand fork of `roles_logic_sv2` | current `stratum-core` 0.6.0 (`binary_sv2`, `codec_sv2`) | the fork is stale; the project baseline is a released codec |
| Type names | `GetTransationsInJob`, `ProvideMissinTransactions` | spelled correctly | wire bytes are unchanged; only Rust/JSON names differ |
| Extension id | type 32 | `0x0020` | same value, explicit hex |
| `Slice.job_id` | "last 4 bytes MUST be the same as the job's" | `job_id = (H << 32) | mining_job_id`, so the low 32 bits are the mining job id | satisfies the rule while keeping the height visible |
| Window older than retention | unspecified | `GetWindow.Busy{retry_in_seconds: 0}` means *gone*, not *retry later* | a busy response with zero retries is unambiguous for the client |
| Window source | pool's live slice state | bootstrap-RPC blobs plus the live `spc:slices` tail | a front can serve a window it did not aggregate itself |
| Identity fields | none in the types | none; the `Share` leaf is `sha256d(nonce ‖ ntime ‖ version ‖ extranonce ‖ job_id ‖ share_index)` | SPEC §4.2; retained blobs and `GetShares` never carry `sp_id` |
| Fee bound | `fees <= slice.fees + delta` | same, with `delta` from `[accounting] slice_fee_delta_sats` | one configuration value for both sealing and verification |
| Sampling | unspecified | `s = 4` slices × `m = 8` shares by default; statistical, not total | bounded audit cost per window |
| Transactions in job | pool's job | pool-template: the pool's tx set; JD: the JDC's declared set | JD miners build their own template |

## Server behaviour

- `NewBlockFound{block_hash}` is broadcast on every pool block.
- `GetWindow` returns the slices selected by SPEC §4.3, walking back from the
  finding slice until `Σ difficulty >= N × D`, excluding the finding slice.
- `GetShares` returns `nonce, ntime, version, extranonce, job_id,
  reference_job_id, share_index, merkle_path` from the stored identity-free
  blob.
- `GetTransactionsInJob` returns the transaction set that built the job.
- `GetWindow.Busy{retry_in_seconds}` is used for a window inside the hot range
  that is not yet sealed; zero means the window is gone.

## Client verifier

For each sampled slice and share:

1. recompute the share hash and check it meets the share target;
2. check `merkle_path(share) + share_hash == slice.root`;
3. check `Σ d <= slice.difficulty`;
4. check `share.job_fees <= slice.fees + delta`.

A pool that under-reports slice difficulty is caught over windows, not within
one window. The seeded `--features cheat-slice-difficulty` build is the CI
fixture for that statistical test (SPEC §8.3).
