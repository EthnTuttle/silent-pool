//! Rule-B economic state. Pure checked arithmetic; no chain/store authority.
use crate::SpId;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("economic accounting unavailable: {0}")]
    Unavailable(&'static str),
    #[error("economic arithmetic or provenance invalid: {0}")]
    Invalid(&'static str),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Balance {
    pub sp_id: SpId,
    pub booked: u64,
    pub reserved: u64,
    pub available: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    Unbooked,
    Booked,
    Cancelled,
    LossHeld,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Consumption {
    pub sp_id: SpId,
    pub prior: u64,
    pub error_pending: u64,
    pub delta: i64,
    pub consumed: u64,
}
impl Consumption {
    pub fn new(sp_id: SpId, prior: u64, error_pending: u64) -> Result<Self, Error> {
        let delta = i64::try_from(i128::from(error_pending) - i128::from(prior))
            .map_err(|_| Error::Invalid("delta range"))?;
        Ok(Self {
            sp_id,
            prior,
            error_pending,
            delta,
            consumed: delta.unsigned_abs() * u64::from(delta < 0),
        })
    }
    pub fn from_delta(sp_id: SpId, prior: u64, delta: i64) -> Result<Self, Error> {
        let after = u64::try_from(i128::from(prior) + i128::from(delta))
            .map_err(|_| Error::Invalid("negative error pending"))?;
        Self::new(sp_id, prior, after)
    }
    pub fn validate(&self) -> Result<(), Error> {
        if Self::new(self.sp_id, self.prior, self.error_pending)? != *self {
            return Err(Error::Invalid("consumption binding"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reservation {
    pub block_hash: [u8; 32],
    pub parent_hash: [u8; 32],
    pub height: u32,
    pub payload_digest: [u8; 32],
    pub job_id: u64,
    pub generation: u64,
    pub lifecycle: Lifecycle,
    pub transition: u64,
    pub revision: u64,
    pub rows: Vec<Consumption>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct View {
    pub version: u8,
    pub revision: u64,
    pub anchor_height: u32,
    pub anchor_hash: [u8; 32],
    pub complete: bool,
    pub booked: Vec<(SpId, u64)>,
    pub reservations: Vec<Reservation>,
}
impl View {
    pub fn balances(&self) -> Result<BTreeMap<SpId, Balance>, Error> {
        if self.version != 1 || !self.complete {
            return Err(Error::Unavailable("incomplete canonical reservation view"));
        }
        let mut map = BTreeMap::new();
        for (id, booked) in &self.booked {
            if map
                .insert(
                    *id,
                    Balance {
                        sp_id: *id,
                        booked: *booked,
                        reserved: 0,
                        available: *booked,
                    },
                )
                .is_some()
            {
                return Err(Error::Invalid("duplicate booked identity"));
            }
        }
        let mut blocks = BTreeSet::new();
        for record in &self.reservations {
            if !blocks.insert(record.block_hash) || record.revision > self.revision {
                return Err(Error::Invalid("reservation revision/duplicate"));
            }
            let mut ids = BTreeSet::new();
            for row in &record.rows {
                row.validate()?;
                if !ids.insert(row.sp_id) {
                    return Err(Error::Invalid("duplicate reservation identity"));
                }
                if record.lifecycle == Lifecycle::Unbooked {
                    let balance = map.entry(row.sp_id).or_insert(Balance {
                        sp_id: row.sp_id,
                        booked: 0,
                        reserved: 0,
                        available: 0,
                    });
                    balance.reserved = balance
                        .reserved
                        .checked_add(row.consumed)
                        .ok_or(Error::Invalid("reservation overflow"))?;
                }
            }
        }
        for balance in map.values_mut() {
            balance.available = balance
                .booked
                .checked_sub(balance.reserved)
                .ok_or(Error::Unavailable("reserved exceeds booked"))?;
        }
        Ok(map)
    }
    pub fn at_parent(&self, height: u32, hash: [u8; 32]) -> Result<(), Error> {
        self.balances()?;
        if self.anchor_height != height || self.anchor_hash != hash {
            return Err(Error::Unavailable("parent economic reconciliation pending"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenJob {
    pub version: u8,
    pub revision: u64,
    pub parent_hash: [u8; 32],
    pub job_id: u64,
    pub generation: u64,
    pub balances: Vec<Balance>,
    pub slice_ids: Vec<u64>,
}
impl FrozenJob {
    pub fn prior(&self) -> Result<BTreeMap<SpId, u64>, Error> {
        if self.version != 1 {
            return Err(Error::Unavailable("frozen economic version"));
        }
        if self.slice_ids.len() > 4096 || self.slice_ids.windows(2).any(|w| w[0] <= w[1]) {
            return Err(Error::Invalid("frozen slice bound/order"));
        }
        let mut out = BTreeMap::new();
        for b in &self.balances {
            if b.booked.checked_sub(b.reserved) != Some(b.available)
                || out.insert(b.sp_id, b.available).is_some()
            {
                return Err(Error::Invalid("frozen B/R/A"));
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn net_debit_vectors_do_not_suppress_fresh_raw_or_early_release_gains() {
        let id = [1; 32];
        let a = Consumption::new(id, 500_000, 100_000).unwrap();
        assert_eq!(a.delta, -400_000);
        assert_eq!(a.consumed, 400_000);
        let b = Consumption::new(id, 100_000, 0).unwrap();
        let jd = Consumption::new(id, 500_000, 0).unwrap();
        assert_eq!(jd.consumed, 500_000); // O11 claim paid 300k is irrelevant to net debit
        let positive = Consumption::new(id, 0, 1_000_000).unwrap();
        assert_eq!(positive.consumed, 0);
        for deltas in [[a.delta, b.delta], [b.delta, a.delta]] {
            let mut booked = 500_000i128;
            let mut reserved = 500_000u64;
            for delta in deltas {
                let before = booked - i128::from(reserved);
                booked += i128::from(delta);
                reserved -= delta.unsigned_abs();
                assert_eq!(booked - i128::from(reserved), before);
            }
            assert_eq!(booked, 0);
        }
        let zero = Consumption::new(id, 0, 0).unwrap();
        assert_eq!(zero.consumed, 0);
        let fresh_raw = 1_000_000u64;
        let mut view = View {
            version: 1,
            revision: 0,
            anchor_height: 0,
            anchor_hash: [0; 32],
            complete: true,
            booked: vec![],
            reservations: vec![],
        };
        for height in 1..=2 {
            view.revision = height as u64;
            view.reservations.push(Reservation {
                block_hash: [height as u8; 32],
                parent_hash: [height as u8 - 1; 32],
                height,
                payload_digest: [3; 32],
                job_id: height as u64,
                generation: height as u64,
                lifecycle: Lifecycle::Unbooked,
                transition: 1,
                revision: height as u64,
                rows: vec![zero.clone()],
            });
            let available = view.balances().unwrap()[&id].available;
            assert_eq!(fresh_raw.checked_add(available), Some(1_000_000));
        }
    }
    #[test]
    fn missing_or_over_reserved_state_is_unknown_not_zero() {
        let mut view = View {
            version: 1,
            revision: 1,
            anchor_height: 0,
            anchor_hash: [0; 32],
            complete: false,
            booked: vec![],
            reservations: vec![],
        };
        assert!(view.balances().is_err());
        view.complete = true;
        view.reservations.push(Reservation {
            block_hash: [1; 32],
            parent_hash: [0; 32],
            height: 1,
            payload_digest: [2; 32],
            job_id: 1,
            generation: 1,
            lifecycle: Lifecycle::Unbooked,
            transition: 1,
            revision: 1,
            rows: vec![Consumption::new([1; 32], 500_000, 0).unwrap()],
        });
        assert!(view.balances().is_err());
        view.booked.push(([1; 32], 500_000));
        assert_eq!(view.balances().unwrap()[&[1; 32]].available, 0);
        view.reservations[0].lifecycle = Lifecycle::Cancelled;
        assert_eq!(view.balances().unwrap()[&[1; 32]].available, 500_000);
    }
}
