//! Job Declarator Server state (SPEC §3.5, PR #203). The network loop is a
//! thin wrapper; the publication id, grace rule and recompute-and-compare
//! validation live here so they can be tested without a socket.

use std::collections::BTreeMap;

use spc_accounting::{p2tr_script, Rail, Weights};
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

impl JdError {
    /// The wire `error_code` answering a declined declaration (SPEC §3.5
    /// "Declaration answers"). The JDS answers every parseable declaration,
    /// so a JDC can always tell a coded decline from a transport error.
    pub fn decline_code(&self) -> &'static str {
        match self {
            JdError::Invalid => "invalid-payout-distribution",
            JdError::Stale => "stale-payout-distribution",
            JdError::Empty => "no-payout-distribution",
            JdError::Ext(_) => "internal-error",
        }
    }
}

#[derive(Clone, Debug)]
struct Published {
    id: u64,
    height: u32,
    revenue: u64,
    distribution: SetPayoutDistribution,
}

/// The low 56 bits hold the per-front sequence; the high 8 hold `front_id`.
const ID_SEQ_MASK: u64 = 0x00ff_ffff_ffff_ffff;

fn unix_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
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

    /// For deterministic tests; production seeds every id from the wall clock.
    pub fn set_clock(&mut self, now_ms: u64) {
        self.now_ms = now_ms;
    }

    /// `id = front_id << 56 | max(last_local_seq + 1, unix_time_ms)` (SPEC
    /// §3.5). The clock is read on every call so a restarted process cannot
    /// reuse ids from a previous run; the Redis `front_id` lease keeps a
    /// second live process from producing the same sequence.
    fn next_id(&mut self) -> u64 {
        let n = self
            .local_seq
            .saturating_add(1)
            .max(self.now_ms)
            .max(unix_time_ms())
            .min(ID_SEQ_MASK);
        self.local_seq = n;
        ((self.front_id as u64) << 56) | (n & ID_SEQ_MASK)
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
    ///
    /// `T` here is deliberately the pool's `coinbase_tx_value_remaining` at
    /// publication, not the JDC's current template value. A JDC that built for
    /// a different `T` is rejected at declaration time; it must fetch/redeclare
    /// against the published distribution. Settlement is unaffected: the
    /// ledger recomputes every claim with the on-chain `T_actual` through O11
    /// (`spc_accounting::claim_jd`), so a rejected declaration never becomes a
    /// settlement basis.
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
            // A declaration for the current id is only valid at the current
            // height; height straddling is stale (SPEC §3.5).
            if current.height != height {
                return Err(JdError::Stale);
            }
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

/// Convert `spc-accounting` JD weights into the wire distribution the JDS
/// publishes (SPEC §3.5).
///
/// `weights.weights` is the kept-id list (the real `build_weights` output);
/// `p_x_by_id` supplies the per-id `P_i` x-only keys, which are not part of
/// `Weights` (they are fixed per `(sp_id, H)` and live with the pool-template
/// distribution). A `p_x` already present in `weights.entries` is accepted as
/// a fallback. Any kept id without a key or a mismatched pool script is
/// rejected rather than guessed.
///
/// The OP_RETURN carrier is always emitted as the single additional output and
/// carries `A_send`, zero-valued.
pub fn distribution_from_weights(
    weights: &Weights,
    p_x_by_id: &BTreeMap<[u8; 32], [u8; 32]>,
    pool_script: &[u8],
    a_send_x: &[u8; 32],
    dust_limit_sats: u32,
) -> Result<SetPayoutDistribution, JdError> {
    if pool_script.len() != 34 || pool_script[0] != 0x51 || pool_script[1] != 0x20 {
        return Err(JdError::Invalid);
    }
    let mut payouts = Vec::with_capacity(weights.weights.len());
    for (id, weight) in &weights.weights {
        let p_x = p_x_by_id.get(id).copied().or_else(|| {
            weights
                .entries
                .iter()
                .find(|entry| entry.sp_id == *id && entry.rail == Rail::Coinbase)
                .and_then(|entry| entry.p_x)
        });
        let p_x = p_x.ok_or(JdError::Invalid)?;
        payouts.push(TxOut {
            script: p2tr_script(&p_x),
            value: *weight,
        });
    }
    Ok(SetPayoutDistribution {
        distribution_id: 0,
        pool_payout: TxOut {
            script: pool_script.to_vec(),
            value: weights.pool_weight,
        },
        dust_limits: vec![dust_limit_sats; payouts.len()],
        payouts,
        additional_outputs: vec![TxOut {
            script: spc_crypto::emit_op_return(a_send_x).to_bytes(),
            value: 0,
        }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use spc_ext::TxOut;

    /// SPEC §3.5 "Declaration answers": every JdError maps to a stable,
    /// project-defined wire code (or upstream `internal-error`), so a coded
    /// decline is always distinguishable from a transport error.
    #[test]
    fn decline_codes_match_the_spec_table() {
        assert_eq!(JdError::Invalid.decline_code(), "invalid-payout-distribution");
        assert_eq!(JdError::Stale.decline_code(), "stale-payout-distribution");
        assert_eq!(JdError::Empty.decline_code(), "no-payout-distribution");
        assert_eq!(JdError::Ext("x".into()).decline_code(), "internal-error");
    }

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
    fn rejects_a_missing_output() {
        let mut jds = Jds::new(0);
        let d = dist();
        let id = jds.publish(1, 1_000, d.clone());
        let mut out = declared(&d);
        out.pop();
        assert_eq!(jds.validate(id, 1, 1_000, &out), Err(JdError::Invalid));

        let mut out = declared(&d);
        out.remove(0);
        assert_eq!(jds.validate(id, 1, 1_000, &out), Err(JdError::Invalid));
    }

    #[test]
    fn rejects_height_straddling_even_for_the_current_id() {
        let mut jds = Jds::new(0);
        let d = dist();
        let id = jds.publish(800_000, 1_000, d.clone());
        let out = declared(&d);
        assert_eq!(jds.validate(id, 800_000, 1_000, &out), Ok(()));
        assert_eq!(jds.validate(id, 800_001, 1_000, &out), Err(JdError::Stale));
        // The declaration lookup must reject the same straddle.
        assert_eq!(
            jds.validate_declared(id, 800_001, &out),
            Err(JdError::Stale)
        );
    }

    #[test]
    fn tolerates_a_duplicate_trailing_carrier() {
        let mut jds = Jds::new(0);
        let d = dist();
        let id = jds.publish(1, 1_000, d.clone());
        let mut out = declared(&d);
        // A JDC that also emitted its own carrier appends a second, 0-valued
        // copy. Trailing 0-value outputs are the JDC/TP slot, so this is fine.
        out.push(TxOut {
            script: vec![0x6a, 1, 2, 3],
            value: 0,
        });
        assert_eq!(jds.validate(id, 1, 1_000, &out), Ok(()));
    }

    #[test]
    fn validate_declared_uses_the_published_revenue() {
        let mut jds = Jds::new(0);
        let d = dist();
        let id = jds.publish(1, 1_000, d.clone());
        let out = declared(&d);
        assert_eq!(jds.validate_declared(id, 1, &out), Ok(()));
        // A JDC that built for a different `T` (its own template revenue) is
        // rejected at declaration time by design: the pool/JDS share the
        // published `T`, and the declaration must match the distribution.
        // Settlement later recomputes the claim from the mined coinbase's
        // actual `T` via O11 (`spc_accounting::claim_jd`), so this rejection
        // is not a settlement path.
        let wrong = {
            let a = compute_payout_amounts(1_001, 100, &[300], &[0], 0).unwrap();
            vec![
                TxOut {
                    script: d.pool_payout.script.clone(),
                    value: a.pay_p,
                },
                TxOut {
                    script: d.payouts[0].script.clone(),
                    value: a.pays[0].unwrap(),
                },
                d.additional_outputs[0].clone(),
            ]
        };
        assert_eq!(jds.validate_declared(id, 1, &wrong), Err(JdError::Invalid));
        // Settlement does not use the published `T`: O11 recomputes each claim
        // from the mined coinbase's `T_actual`, so the rejected declaration is
        // never a settlement input.
        assert_eq!(spc_accounting::claim_jd(300, 1_001, 0, 400), Ok(750));
        assert_eq!(jds.validate_declared(id + 1, 1, &out), Err(JdError::Stale));
        assert_eq!(jds.validate_declared(id, 2, &out), Err(JdError::Stale));
    }

    #[test]
    fn accepts_id_tracks_current_and_same_height_previous_only() {
        let mut jds = Jds::new(0);
        let a = jds.publish(1, 1_000, dist());
        let b = jds.publish(1, 1_000, dist());
        assert!(jds.accepts_id(a));
        assert!(jds.accepts_id(b));
        let c = jds.publish(2, 1_000, dist());
        // `a` is no longer the previous id and `b` now straddles heights.
        assert!(!jds.accepts_id(a));
        assert!(!jds.accepts_id(b));
        assert!(jds.accepts_id(c));
    }

    fn weights_fixture() -> Weights {
        let id_a = [0x0a; 32];
        let id_b = [0x0b; 32];
        Weights {
            pool_weight: 50,
            weights: vec![(id_a, 300), (id_b, 200)],
            // `build_weights` leaves `p_x` empty for every rail; the keys come
            // from the separate per-height map.
            entries: vec![
                spc_accounting::Entry {
                    sp_id: id_a,
                    rail: Rail::Coinbase,
                    p_x: None,
                    dest_id: None,
                    sats: 300,
                },
                spc_accounting::Entry {
                    sp_id: id_b,
                    rail: Rail::Coinbase,
                    p_x: None,
                    dest_id: None,
                    sats: 200,
                },
                spc_accounting::Entry {
                    sp_id: [0x0c; 32],
                    rail: Rail::Pending,
                    p_x: None,
                    dest_id: None,
                    sats: 90,
                },
            ],
        }
    }

    fn key_map() -> BTreeMap<[u8; 32], [u8; 32]> {
        let mut map = BTreeMap::new();
        map.insert([0x0a; 32], [0x11; 32]);
        map.insert([0x0b; 32], [0x22; 32]);
        map
    }

    #[test]
    fn weights_convert_to_a_wire_distribution() {
        let a_x = [0x33; 32];
        let pool_script = p2tr_script(&[0x44; 32]);
        let dist =
            distribution_from_weights(&weights_fixture(), &key_map(), &pool_script, &a_x, 330)
                .unwrap();
        assert_eq!(dist.pool_payout.script, pool_script);
        assert_eq!(dist.pool_payout.value, 50);
        assert_eq!(dist.payouts.len(), 2);
        assert_eq!(dist.payouts[0].value, 300);
        assert_eq!(dist.payouts[1].value, 200);
        assert_eq!(dist.payouts[0].script, p2tr_script(&[0x11; 32]));
        assert_eq!(dist.dust_limits, vec![330, 330]);
        assert_eq!(dist.additional_outputs.len(), 1);
        assert_eq!(dist.additional_outputs[0].value, 0);
        assert_eq!(
            dist.additional_outputs[0].script,
            spc_crypto::emit_op_return(&a_x).to_bytes()
        );
        // Round-trip through the wire codec.
        let decoded = SetPayoutDistribution::decode(&dist.encode().unwrap()).unwrap();
        assert_eq!(decoded, dist);
    }

    #[test]
    fn weights_accept_a_p_x_carried_in_the_entries() {
        let mut w = weights_fixture();
        w.entries[0].p_x = Some([0x77; 32]);
        let dist = distribution_from_weights(
            &w,
            &BTreeMap::from([([0x0b; 32], [0x22; 32])]),
            &p2tr_script(&[0x44; 32]),
            &[0x33; 32],
            330,
        )
        .unwrap();
        assert_eq!(dist.payouts[0].script, p2tr_script(&[0x77; 32]));
    }

    #[test]
    fn weights_without_keys_or_with_wrong_pool_script_are_rejected() {
        let a_x = [0x33; 32];
        let pool_script = p2tr_script(&[0x44; 32]);

        let mut keys = key_map();
        keys.remove(&[0x0a; 32]);
        assert_eq!(
            distribution_from_weights(&weights_fixture(), &keys, &pool_script, &a_x, 330),
            Err(JdError::Invalid)
        );

        assert_eq!(
            distribution_from_weights(&weights_fixture(), &key_map(), &[0x51, 0x20, 1], &a_x, 330),
            Err(JdError::Invalid)
        );
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

    #[test]
    fn ids_are_strictly_increasing_across_a_restart() {
        let mut first = Jds::new(5);
        let old = first.publish(1, 1_000, dist());
        // A restarted process starts with `local_seq = 0`; the wall clock must
        // carry the sequence past the previous run's ids.
        std::thread::sleep(std::time::Duration::from_millis(5));
        let mut restarted = Jds::new(5);
        let new = restarted.publish(1, 1_000, dist());
        assert!(
            new > old,
            "restart reused a distribution id: {old} -> {new}"
        );
        assert_eq!(new >> 56, 5);
    }
}
