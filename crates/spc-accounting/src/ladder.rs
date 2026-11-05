//! Constant denomination ladder shared by build and settlement (SPEC §4.4).

use crate::Error;

/// FINAL 2026-11-03: fixed satoshis, with no height or era inputs.
pub const LADDER_BASE_RUNGS_SATS: [u64; 4] = [500_000, 1_000_000, 3_000_000, 5_000_000];
pub const LADDER_BASE_SATS: u64 = LADDER_BASE_RUNGS_SATS[0];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LadderMode {
    PoolTemplate,
    JobDeclaration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decomposition {
    /// Rung values in canonical largest-first order (repetition is allowed).
    pub outputs: Vec<u64>,
    pub paid: u64,
    pub remainder: u64,
}

/// Validate the configured floors against every fixed rung.
pub fn validate_ladder(coinbase_threshold_sats: u64, dust_limit_sats: u64) -> Result<(), Error> {
    if coinbase_threshold_sats < dust_limit_sats {
        return Err(Error::Threshold);
    }
    if let Some(rung) = LADDER_BASE_RUNGS_SATS
        .iter()
        .copied()
        .find(|rung| *rung < coinbase_threshold_sats)
    {
        return Err(Error::RungBelowThreshold {
            rung,
            threshold: coinbase_threshold_sats,
        });
    }
    Ok(())
}

/// The coinbase/Ark split, without allocating the output list.
pub fn band_amount(owed: u64) -> (u64, u64) {
    let remainder = owed % LADDER_BASE_SATS;
    (owed - remainder, remainder)
}

/// Number of outputs in the canonical greedy decomposition.
pub fn decomposition_len(owed: u64) -> u64 {
    let mut remaining = owed;
    let mut count = 0u64;
    for rung in LADDER_BASE_RUNGS_SATS.iter().rev().copied() {
        count = count.saturating_add(remaining / rung);
        remaining %= rung;
    }
    count
}

/// Greedy largest-first decomposition. The caller supplies the block's output
/// policy cap so an untrusted amount cannot request an unbounded allocation.
pub fn decompose(owed: u64, max_outputs: usize) -> Result<Decomposition, Error> {
    let required = decomposition_len(owed);
    if required > max_outputs as u64 {
        return Err(Error::PayoutSlots {
            required,
            max: max_outputs,
        });
    }
    let capacity = usize::try_from(required).map_err(|_| Error::Overflow)?;
    let (paid, remainder) = band_amount(owed);
    let mut outputs = Vec::with_capacity(capacity);
    let mut remaining = paid;
    for rung in LADDER_BASE_RUNGS_SATS.iter().rev().copied() {
        let count = remaining / rung;
        outputs.extend(std::iter::repeat_n(rung, count as usize));
        remaining %= rung;
    }
    debug_assert_eq!(remaining, 0);
    Ok(Decomposition {
        outputs,
        paid,
        remainder,
    })
}

/// The sole banding entry point used by routing, distribution and settlement.
/// JD has no pool-side banding convention, so it is rejected rather than
/// silently falling back to exact owed amounts.
pub fn decompose_for_mode(
    owed: u64,
    max_outputs: usize,
    mode: LadderMode,
) -> Result<Decomposition, Error> {
    if mode == LadderMode::JobDeclaration {
        return Err(Error::JdBanding);
    }
    decompose(owed, max_outputs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn largest_first_canonical_vectors() {
        assert_eq!(decompose(499_999, 1).unwrap().outputs, Vec::<u64>::new());
        assert_eq!(decompose(500_000, 1).unwrap().outputs, vec![500_000]);
        assert_eq!(
            decompose(9_500_001, 4).unwrap().outputs,
            vec![5_000_000, 3_000_000, 1_000_000, 500_000]
        );
        assert_eq!(
            decompose(10_000_000, 2).unwrap().outputs,
            vec![5_000_000, 5_000_000]
        );
    }

    #[test]
    fn validation_and_slot_cap_fail_closed() {
        assert_eq!(validate_ladder(329, 330), Err(Error::Threshold));
        assert!(matches!(
            validate_ladder(500_001, 330),
            Err(Error::RungBelowThreshold { rung: 500_000, .. })
        ));
        assert!(matches!(
            decompose(1_000_000, 0),
            Err(Error::PayoutSlots {
                required: 1,
                max: 0
            })
        ));
        assert_eq!(
            decompose_for_mode(1_000_000, 1, LadderMode::JobDeclaration),
            Err(Error::JdBanding)
        );
    }

    proptest! {
        #[test]
        fn decomposition_properties(owed in 0u64..=100_000_000u64) {
            let upper_bound = owed / LADDER_BASE_SATS;
            let d = decompose(owed, upper_bound as usize).unwrap();
            prop_assert_eq!(d.paid + d.remainder, owed);
            prop_assert_eq!(d.outputs.iter().sum::<u64>(), d.paid);
            prop_assert!(d.outputs.len() as u64 <= upper_bound);
            prop_assert!(d.outputs.iter().all(|v| LADDER_BASE_RUNGS_SATS.contains(v)));
            prop_assert!(d.outputs.windows(2).all(|w| w[0] >= w[1]));
            prop_assert_eq!(d.outputs.len() as u64, decomposition_len(owed));
        }
    }
}
