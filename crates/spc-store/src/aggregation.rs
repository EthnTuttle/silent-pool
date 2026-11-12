//! SPc-side remainder membership primitives (SPEC §11.3).
//!
//! These do not select a destination or authorize emission. Aggregate destination
//! binding is unresolved in the authority and must precede send-loop integration.
use std::collections::BTreeSet;

pub const MIN_OVERAGE_SATS: u64 = 10_000;
pub const OVERAGE_MAX_DELAY_BLOCKS: u32 = 144;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Remainder {
    pub block_hash: [u8; 32],
    pub sp_id: [u8; 32],
    pub due_height: u32,
    pub sats: u64,
}

/// A complete, immutable covered set. The store must freeze this under its
/// identity lock before publication, not reconstruct it from today's rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoveredSet {
    pub rows: Vec<Remainder>,
    pub sats: u64,
    pub not_before_height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("invalid, duplicate or mixed-identity remainder membership")]
    Membership,
    #[error("aggregate amount overflow")]
    Overflow,
    #[error("covered membership no longer canonical; emission prohibited")]
    Reorg,
}

/// Input contains only unsent, unassigned, coinbase-row remainder components.
/// Full sub-base Ark rows are not aggregation candidates (§11.1 cadence).
pub fn select(
    rows: &[Remainder],
    tip: u32,
    threshold: u64,
    delay: u32,
) -> Result<Option<CoveredSet>, Error> {
    let mut rows = rows.to_vec();
    rows.sort_by_key(|r| (r.due_height, r.block_hash));
    let Some(first) = rows.first() else {
        return Ok(None);
    };
    let identity = first.sp_id;
    let mut hashes = BTreeSet::new();
    for row in &rows {
        if row.sp_id != identity
            || row.sats == 0
            || row.sats >= spc_accounting::LADDER_BASE_SATS
            || !hashes.insert(row.block_hash)
        {
            return Err(Error::Membership);
        }
    }
    rows.retain(|r| r.due_height <= tip);
    let Some(first) = rows.first() else {
        return Ok(None);
    };
    let oldest = first.due_height;
    let sats = rows
        .iter()
        .try_fold(0u64, |sum, r| sum.checked_add(r.sats))
        .ok_or(Error::Overflow)?;
    if sats < threshold && tip - oldest < delay {
        return Ok(None);
    }
    let not_before_height = rows.last().ok_or(Error::Membership)?.due_height;
    Ok(Some(CoveredSet {
        rows,
        sats,
        not_before_height,
    }))
}

impl CoveredSet {
    /// Every member, not only the anchor, must pass active-chain evidence
    /// immediately before first publication. Unknown observations fail closed.
    pub fn revalidate(&self, canonical: &BTreeSet<[u8; 32]>) -> Result<(), Error> {
        if self.rows.iter().any(|r| !canonical.contains(&r.block_hash)) {
            return Err(Error::Reorg);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row(hash: u8, due: u32, sats: u64) -> Remainder {
        Remainder {
            block_hash: [hash; 32],
            sp_id: [9; 32],
            due_height: due,
            sats,
        }
    }
    #[test]
    fn threshold_timeout_and_complete_reorg_membership() {
        let rows = vec![row(2, 12, 5_000), row(1, 10, 5_000)];
        assert!(
            select(&rows, 11, MIN_OVERAGE_SATS, OVERAGE_MAX_DELAY_BLOCKS)
                .unwrap()
                .is_none()
        );
        let set = select(&rows, 12, MIN_OVERAGE_SATS, OVERAGE_MAX_DELAY_BLOCKS)
            .unwrap()
            .unwrap();
        assert_eq!(set.rows[0].block_hash, [1; 32]);
        assert_eq!(set.sats, 10_000);
        assert_eq!(set.not_before_height, 12);
        assert_eq!(
            set.revalidate(&BTreeSet::from([[1; 32]])),
            Err(Error::Reorg)
        );
        assert!(set.revalidate(&BTreeSet::from([[1; 32], [2; 32]])).is_ok());
        assert!(select(&rows[..1], 155, 10_000, 144).unwrap().is_none());
        assert_eq!(
            select(&rows[..1], 156, 10_000, 144).unwrap().unwrap().sats,
            5_000
        );
        assert_eq!(
            select(&[rows[0].clone(), rows[0].clone()], 12, 10_000, 144),
            Err(Error::Membership)
        );
    }
}
