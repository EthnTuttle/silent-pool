//! Job Declarator Server state (SPEC §3.5, PR #203). The network loop is a
//! thin wrapper; the publication id, grace rule and recompute-and-compare
//! validation live here so they can be tested without a socket.

use spc_ext::{compute_payout_amounts, Error as ExtError, SetPayoutDistribution, TxOut};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum JdError {
    #[error("invalid-payout-distribution")]
    Invalid,
    #[error("stale-payout-distribution")]
    Stale,
    #[error("no distribution published")]
    Empty,
    #[error("ext: {0}")]
    Ext(String),
}

#[derive(Clone, Debug)]
struct Published {
    id: u64,
    height: u32,
    revenue: u64,
    distribution: SetPayoutDistribution,
}

pub struct Jds {
    front_id: u8,
    local_seq: u64,
    current: Option<Published>,
    previous: Option<Published>,
    now_ms: u64,
}

impl Jds {
    pub fn new(front_id: u8) -> Self {
        Self {
            front_id: front_id & 0x7f,
            local_seq: 0,
            current: None,
            previous: None,
            now_ms: 0,
        }
    }

    /// For deterministic tests; production passes the wall clock.
    pub fn set_clock(&mut self, now_ms: u64) {
        self.now_ms = now_ms;
    }

    fn next_id(&mut self) -> u64 {
        let n = self.local_seq.saturating_add(1).max(self.now_ms);
        self.local_seq = n;
        ((self.front_id as u64) << 56) | (n & 0x00ff_ffff_ffff_ffff)
    }

    /// Publish a distribution for `height`; returns its id.
    pub fn publish(
        &mut self,
        height: u32,
        revenue: u64,
        mut distribution: SetPayoutDistribution,
    ) -> u64 {
        let id = self.next_id();
        distribution.distribution_id = id;
        self.previous = self.current.take();
        self.current = Some(Published {
            id,
            height,
            revenue,
            distribution,
        });
        id
    }

    /// Recompute and compare a declaration using the revenue recorded when the
    /// distribution was published (the pool and JDS share state, PR #203 §10).
    pub fn validate_declared(
        &self,
        id: u64,
        height: u32,
        declared: &[spc_ext::TxOut],
    ) -> Result<(), JdError> {
        let Some(p) = self.current.as_ref() else {
            return Err(JdError::Empty);
        };
        let revenue = if p.id == id && p.height == height {
            p.revenue
        } else if let Some(prev) = &self.previous {
            if prev.id == id && prev.height == height && p.height == height {
                prev.revenue
            } else {
                return Err(JdError::Stale);
            }
        } else {
            return Err(JdError::Stale);
        };
        self.validate(id, height, revenue, declared)
    }

    pub fn current(&self) -> Option<&SetPayoutDistribution> {
        self.current.as_ref().map(|p| &p.distribution)
    }

    /// Same-front, same-height acceptance check used by the JDP loop before
    /// the full recompute-and-compare validation.
    pub fn accepts_id(&self, id: u64) -> bool {
        let current = self.current.as_ref().map(|p| (p.id, p.height));
        let previous = self.previous.as_ref().map(|p| (p.id, p.height));
        match (current, previous) {
            (Some((cid, _)), _) if cid == id => true,
            (Some((cid, ch)), Some((pid, ph))) if pid == id && ph == ch && cid != id => true,
            _ => false,
        }
    }

    /// Recompute the declared outputs from the distribution and `T`, and
    /// compare. The grace window is same-height-only and same-front.
    pub fn validate(
        &self,
        id: u64,
        height: u32,
        template_revenue: u64,
        declared: &[TxOut],
    ) -> Result<(), JdError> {
        let Some(current) = &self.current else {
            return Err(JdError::Empty);
        };
        let published = if current.id == id {
            current
        } else if let Some(prev) = &self.previous {
            if prev.id == id && prev.height == height && current.height == height {
                prev
            } else {
                return Err(JdError::Stale);
            }
        } else {
            return Err(JdError::Stale);
        };

        let fixed = published
            .distribution
            .fixed_value()
            .map_err(|e| JdError::Ext(e.to_string()))?;
        let weights: Vec<u64> = published
            .distribution
            .payouts
            .iter()
            .map(|p| p.value)
            .collect();
        let amounts = compute_payout_amounts(
            template_revenue,
            published.distribution.pool_payout.value,
            &weights,
            &published.distribution.dust_limits,
            fixed,
        )
        .map_err(|e| match e {
            ExtError::Fixed | ExtError::Weight | ExtError::LengthMismatch => JdError::Invalid,
            other => JdError::Ext(other.to_string()),
        })?;

        let mut expected: Vec<TxOut> = Vec::with_capacity(declared.len());
        expected.push(TxOut {
            script: published.distribution.pool_payout.script.clone(),
            value: amounts.pay_p,
        });
        for (out, pay) in published.distribution.payouts.iter().zip(&amounts.pays) {
            if let Some(v) = pay {
                expected.push(TxOut {
                    script: out.script.clone(),
                    value: *v,
                });
            }
        }
        expected.extend(published.distribution.additional_outputs.iter().cloned());

        if declared.len() < expected.len() {
            return Err(JdError::Invalid);
        }
        for (want, got) in expected.iter().zip(declared) {
            if want.script != got.script || want.value != got.value {
                return Err(JdError::Invalid);
            }
        }
        // Trailing JDC/TP outputs must be 0-valued.
        if declared[expected.len()..].iter().any(|o| o.value != 0) {
            return Err(JdError::Invalid);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spc_ext::TxOut;

    fn dist() -> SetPayoutDistribution {
        SetPayoutDistribution {
            distribution_id: 0,
            pool_payout: TxOut {
                script: vec![0x51, 0x20, 1, 1],
                value: 100,
            },
            payouts: vec![TxOut {
                script: vec![0x51, 0x20, 2, 2],
                value: 300,
            }],
            dust_limits: vec![0],
            additional_outputs: vec![TxOut {
                script: vec![0x6a, 1, 2, 3],
                value: 0,
            }],
        }
    }

    fn declared(dist: &SetPayoutDistribution) -> Vec<TxOut> {
        let a = compute_payout_amounts(1_000, dist.pool_payout.value, &[300], &[0], 0).unwrap();
        vec![
            TxOut {
                script: dist.pool_payout.script.clone(),
                value: a.pay_p,
            },
            TxOut {
                script: dist.payouts[0].script.clone(),
                value: a.pays[0].unwrap(),
            },
            dist.additional_outputs[0].clone(),
        ]
    }

    #[test]
    fn validates_current_and_previous_same_height() {
        let mut jds = Jds::new(2);
        let d = dist();
        let id1 = jds.publish(800_000, 1_000, d.clone());
        assert_eq!(id1 >> 56, 2);
        let out = declared(&d);
        jds.validate(id1, 800_000, 1_000, &out).unwrap();

        let id2 = jds.publish(800_000, 1_000, d.clone());
        jds.validate(id2, 800_000, 1_000, &declared(&d)).unwrap();
        // Previous id, same height: accepted.
        jds.validate(id1, 800_000, 1_000, &out).unwrap();
        // Previous id after a height change: stale.
        jds.publish(800_001, 1_000, d.clone());
        assert_eq!(jds.validate(id2, 800_001, 1_000, &out), Err(JdError::Stale));
    }

    #[test]
    fn rejects_mutated_amounts_and_order() {
        let mut jds = Jds::new(0);
        let d = dist();
        let id = jds.publish(1, 1_000, d.clone());
        let mut out = declared(&d);
        out[0].value += 1;
        assert_eq!(jds.validate(id, 1, 1_000, &out), Err(JdError::Invalid));

        let mut out = declared(&d);
        out.swap(0, 1);
        assert_eq!(jds.validate(id, 1, 1_000, &out), Err(JdError::Invalid));

        let mut out = declared(&d);
        out.push(TxOut {
            script: vec![0x6a],
            value: 1,
        });
        assert_eq!(jds.validate(id, 1, 1_000, &out), Err(JdError::Invalid));
    }

    #[test]
    fn id_is_strictly_increasing_per_front() {
        let mut jds = Jds::new(1);
        jds.set_clock(100);
        let a = jds.publish(1, 1_000, dist());
        let b = jds.publish(1, 1_000, dist());
        assert!(b > a);
        assert_eq!(a >> 56, 1);
    }
}
