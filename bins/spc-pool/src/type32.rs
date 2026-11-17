//! In-memory type-32 window store. Slices are identity-free; `ShareProof`
//! carries only the leaf fields and merkle path.

use std::collections::BTreeMap;

use spc_accounting::Slice;
use spc_share_ext::{merkle_path, merkle_root, ShareLeaf, ShareProof, SliceSummary};

// Fail closed: the cheat fixture is a dev/test artifact (SPEC §8.3) and must
// never exist in a release binary. Dev and test profiles carry
// debug_assertions; a release profile does not, so the combination refuses to
// compile rather than trusting operator discipline.
#[cfg(all(feature = "cheat-slice-difficulty", not(debug_assertions)))]
compile_error!(
    "cheat-slice-difficulty is a SPEC §8.3 test fixture and must not be compiled into a release build"
);

#[cfg(not(feature = "cheat-slice-difficulty"))]
fn served_difficulty(_slice_id: u64, difficulty: u128) -> u128 {
    difficulty
}

/// SPEC §8.3 cheating pool: under-report the served difficulty of every
/// even-numbered slice to 1% of the true credit sum. Any sampled share from a
/// misstated slice then violates `Σ d ≤ slice.difficulty`, so detection is
/// governed purely by which slices the `4 × 8` sampler hits - caught over
/// windows, not within one window (docs/share-accounting-ext-deltas.md).
#[cfg(feature = "cheat-slice-difficulty")]
fn served_difficulty(slice_id: u64, difficulty: u128) -> u128 {
    static WARN: std::sync::Once = std::sync::Once::new();
    WARN.call_once(|| {
        tracing::error!(
            "cheat-slice-difficulty fixture active: served slice difficulty is deliberately misstated (dev/test only, SPEC §8.3)"
        );
    });
    if slice_id.is_multiple_of(2) {
        difficulty / 100
    } else {
        difficulty
    }
}

#[derive(Clone, Default)]
pub struct Type32Store {
    slices: Vec<SliceSummary>,
    shares: BTreeMap<u64, Vec<ShareProof>>,
}

impl Type32Store {
    pub fn add(&mut self, slice: &Slice) {
        let leaves: Vec<ShareLeaf> = slice
            .shares
            .iter()
            .map(|s| ShareLeaf {
                nonce: s.nonce,
                ntime: s.ntime,
                version: s.version,
                extranonce: s.extranonce.clone(),
                job_id: s.job_id,
                share_index: s.share_index,
            })
            .collect();
        let hashes: Vec<[u8; 32]> = leaves.iter().map(|l| l.hash()).collect();
        let root = merkle_root(&hashes);
        let mut proofs = Vec::with_capacity(leaves.len());
        for (i, leaf) in leaves.into_iter().enumerate() {
            if let Some(path) = merkle_path(&hashes, i) {
                proofs.push(ShareProof {
                    leaf,
                    path,
                    difficulty: slice.shares[i].d as u128,
                    job_fees: slice.shares[i].job_fees,
                });
            }
        }
        self.slices.push(SliceSummary {
            slice_id: slice.slice_id,
            height: slice.height,
            prev_hash: slice.prev_hash,
            job_id: slice.ref_job_id,
            difficulty: served_difficulty(slice.slice_id, slice.difficulty),
            fees: slice.ref_fees,
            root,
            n_shares: slice.shares.len() as u32,
        });
        self.shares.insert(slice.slice_id, proofs);
    }

    pub fn window(&self) -> Vec<SliceSummary> {
        self.slices.clone()
    }

    pub fn shares(&self, slice_id: u64, indices: &[u32]) -> Vec<ShareProof> {
        let Some(all) = self.shares.get(&slice_id) else {
            return Vec::new();
        };
        indices
            .iter()
            .filter_map(|i| all.get(*i as usize).cloned())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spc_accounting::{ShareRec, Slice};

    #[test]
    fn builds_verifiable_window() {
        let mut s = Slice::new(1, 800_000, [1; 32], 0, 7, 50_000);
        for i in 0..3 {
            s.push(ShareRec {
                sp_id: [i; 32],
                d: 10,
                job_fees: 50_000,
                job_id: 7,
                nonce: i as u32,
                ntime: 1_700_000_000,
                version: 0x2000_0000,
                extranonce: vec![i; 8],
                share_index: i as u32,
            });
        }
        s.seal();
        let mut store = Type32Store::default();
        store.add(&s);
        let w = store.window();
        assert_eq!(w.len(), 1);
        let proofs = store.shares(1, &[0, 1, 2]);
        assert_eq!(proofs.len(), 3);
        assert!(spc_share_ext::verify_slice(&w[0], &proofs, 0).is_ok());
    }
}
