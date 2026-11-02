//! In-memory type-32 window store. Slices are identity-free; `ShareProof`
//! carries only the leaf fields and merkle path.

use std::collections::BTreeMap;

use spc_accounting::Slice;
use spc_share_ext::{merkle_path, merkle_root, ShareLeaf, ShareProof, SliceSummary};

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
            difficulty: slice.difficulty,
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
