//! Constant denomination ladder shared by build, settlement and audit (SPEC §4.4).
use crate::Error;

pub const LADDER_BASE_RUNGS_SATS: [u64; 4] = [500_000, 1_000_000, 3_000_000, 5_000_000];
pub const LADDER_BASE_SATS: u64 = LADDER_BASE_RUNGS_SATS[0];
/// Hard representation bound; deployments normally use 1024 policy slots.
pub const MAX_LADDER_OUTPUTS: usize = 2048;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decomposition {
    /// Canonical largest-first values, associated with k before script sorting.
    pub outputs: Vec<u64>,
    pub paid: u64,
    pub remainder: u64,
}

pub fn validate_ladder(threshold: u64, dust: u64) -> Result<(), Error> {
    if threshold < dust || threshold > LADDER_BASE_SATS {
        return Err(Error::Threshold);
    }
    Ok(())
}

pub fn band_amount(owed: u64) -> (u64, u64) {
    let remainder = owed % LADDER_BASE_SATS;
    (owed - remainder, remainder)
}

pub fn decomposition_len(owed: u64) -> u64 {
    let mut remaining = owed;
    let mut count = 0;
    for rung in LADDER_BASE_RUNGS_SATS.iter().rev() {
        // Sum is bounded by owed / 500_000, including at u64::MAX.
        count += remaining / rung;
        remaining %= rung;
    }
    count
}

pub fn decompose(owed: u64, max_outputs: usize) -> Result<Decomposition, Error> {
    let required = decomposition_len(owed);
    let max = max_outputs.min(MAX_LADDER_OUTPUTS);
    if required > max as u64 {
        return Err(Error::PayoutSlots { required, max });
    }
    let (paid, remainder) = band_amount(owed);
    let mut remaining = paid;
    let mut outputs = Vec::with_capacity(required as usize);
    for rung in LADDER_BASE_RUNGS_SATS.iter().rev().copied() {
        outputs.extend(std::iter::repeat_n(rung, (remaining / rung) as usize));
        remaining %= rung;
    }
    Ok(Decomposition {
        outputs,
        paid,
        remainder,
    })
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
        assert!(matches!(
            decompose(u64::MAX, usize::MAX),
            Err(Error::PayoutSlots { .. })
        ));
    }

    #[test]
    fn worked_vector_and_regtest_maximum() {
        let amounts = [180_991_854, 115_178_612, 33_004_532];
        let ds: Vec<_> = amounts.map(|v| decompose(v, 1024).unwrap()).into();
        assert_eq!(ds.iter().map(|d| d.outputs.len()).sum::<usize>(), 67);
        assert_eq!(
            ds.iter().map(|d| d.paid).collect::<Vec<_>>(),
            vec![180_500_000, 115_000_000, 33_000_000]
        );
        assert_eq!(
            ds.iter().map(|d| d.remainder).collect::<Vec<_>>(),
            vec![491_854, 178_612, 4_532]
        );
        assert_eq!(decompose(4_950_000_000, 1024).unwrap().outputs.len(), 990);
    }

    #[test]
    fn era_one_value_bound_sits_under_policy_and_representation_caps() {
        // SPEC §4.1/§4.8: every output is >= 500k sats, so the era-1 owed pool
        // (309 375 000 sats, §4.5) bounds outputs at 619, under the 1024-slot
        // policy default and the 2048-output fee-headroom representation cap.
        let era_one_t_owed: u64 = 309_375_000;
        let bound = era_one_t_owed.div_ceil(LADDER_BASE_SATS);
        assert_eq!(bound, 619);
        assert!(decomposition_len(era_one_t_owed) <= bound);
        let policy_default: u64 = 1024;
        assert!(bound <= policy_default && policy_default <= MAX_LADDER_OUTPUTS as u64);
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
