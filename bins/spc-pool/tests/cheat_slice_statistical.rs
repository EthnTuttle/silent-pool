//! SPEC §8.3 type-32 statistical test: a pool build compiled with
//! `--features cheat-slice-difficulty` is caught by the sampling verifier
//! within 3 windows at `4 × 8` sampling; the honest build passes the same
//! seeded corpus. Seed-pinned - there is no flaky statistic here: the share
//! corpus, the block hashes and the `Sampler` picks are all deterministic.
//!
//! The cheating build (bins/spc-pool/src/type32.rs) under-reports the served
//! difficulty of every even-numbered slice to 1% of its true credit sum, so a
//! sampled misstated slice always trips `Σ d ≤ slice.difficulty`
//! (`Error::Difficulty`). Detection is therefore governed by the slice
//! sampling alone: with half the window misstated and 4 slice picks per
//! window, the per-window detection probability is `1 − (1/2)^4 = 0.9375`
//! and the within-3-windows bound is `1 − (1/16)^3 ≈ 0.99976` - caught over
//! windows, not within one window (docs/share-accounting-ext-deltas.md).

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use spc_accounting::{ShareRec, Slice};
use spc_pool::type32::Type32Store;
use spc_share_ext::{verify_slice, Sampler};

const SEED: u64 = 0x5350_6333_8300; // "SPc3", §8.3; pinned, never randomized
const TRIALS: usize = 400;
const WINDOWS: usize = 3;
const SLICES_PER_WINDOW: usize = 8;
const SHARES_PER_SLICE: usize = 32;
const FEES: u64 = 50_000;

/// One sealed window: 8 consecutive slice ids (so exactly half are even and
/// misstated under the cheat feature), 32 shares each, seeded `d`.
fn build_window(rng: &mut StdRng, slice_base: u64) -> Type32Store {
    let mut store = Type32Store::default();
    for s in 0..SLICES_PER_WINDOW as u64 {
        let mut slice = Slice::new(slice_base + s, 800_000, [1; 32], 0, 7, FEES);
        for i in 0..SHARES_PER_SLICE as u32 {
            slice.push(ShareRec {
                sp_id: [(i % 251) as u8; 32],
                d: rng.gen_range(1_000..2_000),
                job_fees: FEES,
                job_id: 7,
                nonce: rng.gen(),
                ntime: 1_700_000_000 + i,
                version: 0x2000_0000,
                extranonce: vec![s as u8; 8],
                share_index: i,
            });
        }
        slice.seal();
        store.add(&slice);
    }
    store
}

/// The `4 × 8` audit of one window: `Sampler` picks from the block hash
/// exactly as a verifying client would, share indices deduplicated (the audit
/// client requests distinct indices).
fn audit_window(store: &Type32Store, block_hash: [u8; 32]) -> Result<(), spc_share_ext::Error> {
    let window = store.window();
    let (slice_picks, share_picks) =
        Sampler::new(4, 8).pick(&block_hash, window.len(), SHARES_PER_SLICE);
    let mut indices: Vec<u32> = share_picks.iter().map(|i| *i as u32).collect();
    indices.sort_unstable();
    indices.dedup();
    for pick in slice_picks {
        let summary = &window[pick];
        let proofs = store.shares(summary.slice_id, &indices);
        verify_slice(summary, &proofs, 0)?;
    }
    Ok(())
}

/// Replays the seeded corpus: per trial, up to `WINDOWS` audited windows.
/// Returns the first window (0-based) that detected the cheat, or `None`.
fn run_trial(trial: usize) -> Option<usize> {
    let mut rng = StdRng::seed_from_u64(SEED.wrapping_add(trial as u64));
    for w in 0..WINDOWS {
        let slice_base = ((trial * WINDOWS + w) * SLICES_PER_WINDOW) as u64;
        let store = build_window(&mut rng, slice_base);
        let mut block_hash = [0u8; 32];
        rng.fill(&mut block_hash[..]);
        match audit_window(&store, block_hash) {
            Ok(()) => {}
            Err(spc_share_ext::Error::Difficulty) => return Some(w),
            Err(e) => panic!("trial {trial} window {w}: unexpected verifier error: {e}"),
        }
    }
    None
}

#[cfg(feature = "cheat-slice-difficulty")]
mod cheating_build {
    use super::*;

    /// SPEC §8.3: the cheating build is caught within 3 windows at `4 × 8`
    /// sampling, every trial, under the pinned seed; and the measured
    /// per-window detection rate matches the `1 − (1/2)^4 = 0.9375` model.
    #[test]
    fn caught_within_three_windows_at_4x8() {
        let mut caught_by_window = [0usize; WINDOWS];
        let mut missed = 0usize;
        for trial in 0..TRIALS {
            match run_trial(trial) {
                Some(w) => caught_by_window[w] += 1,
                None => missed += 1,
            }
        }
        let first_window = caught_by_window[0] as f64 / TRIALS as f64;
        eprintln!(
            "cheat-slice-difficulty detection: caught per window {caught_by_window:?} of \
             {TRIALS} trials, missed {missed}; window-1 rate {first_window} (model 0.9375)"
        );
        assert_eq!(
            missed, 0,
            "cheating pool escaped a 3-window audit in {missed}/{TRIALS} seeded trials \
             (caught per window: {caught_by_window:?})"
        );
        assert!(
            (0.90..=0.97).contains(&first_window),
            "per-window detection rate {first_window} outside the 0.9375 model band \
             (caught per window: {caught_by_window:?})"
        );
    }

    /// The same cheat is reported over the wire: `spc-scan audit` against a
    /// live cheating server fails verification with the understated-difficulty
    /// error, where the honest server passes (e2e_type32.rs and the honest
    /// control below).
    #[tokio::test]
    async fn wire_audit_reports_understated_difficulty() {
        use std::sync::{Arc, Mutex};
        use tokio::sync::mpsc;

        let mut rng = StdRng::seed_from_u64(SEED);
        let store = build_window(&mut rng, 0);
        let type32 = Arc::new(Mutex::new(store));
        let (tx, _rx) = mpsc::channel(8);
        let server =
            spc_pool::sv2_server::Sv2Server::new(spc_crypto::Network::Regtest, tx, type32, 8);
        let (addr, _h) = server.bind("127.0.0.1:0").await.unwrap();
        // The audit client samples the first 4 slices (ids 0..4), of which
        // ids 0 and 2 are misstated by the cheat build.
        let err = spc_scan::audit::run(addr, [9; 32], 4, 8).await.unwrap_err();
        match err {
            spc_scan::audit::AuditError::Verify(msg) => {
                assert!(
                    msg.contains("understated"),
                    "wrong verification error: {msg}"
                )
            }
            other => panic!("expected a verification failure, got: {other}"),
        }
    }
}

#[cfg(not(feature = "cheat-slice-difficulty"))]
mod honest_build {
    use super::*;

    /// Honest control over the identical seeded corpus: no trial detects
    /// anything in any window, so the §8.3 statistic measures the cheat, not
    /// a verifier bias.
    #[test]
    fn honest_pool_passes_every_seeded_audit() {
        for trial in 0..TRIALS {
            assert_eq!(
                run_trial(trial),
                None,
                "honest window failed the sampling audit in trial {trial}"
            );
        }
    }

    /// Full (non-sampled) verification of one seeded window: every share of
    /// every slice verifies and the served difficulty bounds the total.
    #[test]
    fn honest_window_passes_a_full_audit() {
        let mut rng = StdRng::seed_from_u64(SEED);
        let store = build_window(&mut rng, 0);
        let all: Vec<u32> = (0..SHARES_PER_SLICE as u32).collect();
        for summary in store.window() {
            let proofs = store.shares(summary.slice_id, &all);
            assert_eq!(proofs.len(), SHARES_PER_SLICE);
            verify_slice(&summary, &proofs, 0).unwrap();
        }
    }

    /// Honest wire control at the same `4 × 8` parameters as the cheat test.
    #[tokio::test]
    async fn wire_audit_passes_at_4x8() {
        use std::sync::{Arc, Mutex};
        use tokio::sync::mpsc;

        let mut rng = StdRng::seed_from_u64(SEED);
        let store = build_window(&mut rng, 0);
        let type32 = Arc::new(Mutex::new(store));
        let (tx, _rx) = mpsc::channel(8);
        let server =
            spc_pool::sv2_server::Sv2Server::new(spc_crypto::Network::Regtest, tx, type32, 8);
        let (addr, _h) = server.bind("127.0.0.1:0").await.unwrap();
        let report = spc_scan::audit::run(addr, [9; 32], 4, 8).await.unwrap().0;
        assert_eq!(report.slices, 4);
        assert_eq!(report.verified, 32);
    }
}
