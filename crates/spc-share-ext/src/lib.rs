//! Type-32 share accounting: message codec and sampling verifier.
//!
//! Upstream is `demand-open-source/share-accounting-ext` @ `c661e93`; the
//! intentional divergences are listed in
//! `docs/share-accounting-ext-deltas.md`. This crate owns only the wire types
//! and the verifier, not the pool's slice store.

#![deny(unsafe_code)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const EXTENSION_TYPE: u16 = 0x0020;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum Error {
    #[error("unexpected end of input")]
    Eof,
    #[error("trailing bytes")]
    Trailing,
    #[error("unknown message type {0}")]
    MessageType(u8),
    #[error("merkle path does not verify")]
    Merkle,
    #[error("slice difficulty is understated")]
    Difficulty,
    #[error("share fees exceed the slice bound")]
    Fees,
    #[error("share index out of range")]
    Index,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareLeaf {
    pub nonce: u32,
    pub ntime: u32,
    pub version: u32,
    pub extranonce: Vec<u8>,
    pub job_id: u64,
    pub share_index: u32,
}

impl ShareLeaf {
    /// `sha256d(nonce ‖ ntime ‖ version ‖ extranonce ‖ job_id ‖ share_index)`
    /// (SPEC §4.2). Identity-free by construction.
    pub fn hash(&self) -> [u8; 32] {
        use bitcoin::hashes::{sha256d, Hash, HashEngine};
        let mut eng = sha256d::Hash::engine();
        eng.input(&self.nonce.to_le_bytes());
        eng.input(&self.ntime.to_le_bytes());
        eng.input(&self.version.to_le_bytes());
        eng.input(&self.extranonce);
        eng.input(&self.job_id.to_le_bytes());
        eng.input(&self.share_index.to_le_bytes());
        sha256d::Hash::from_engine(eng).to_byte_array()
    }
}

/// Bitcoin-style merkle root: duplicate the last node on an odd level.
pub fn merkle_root(leaves: &[[u8; 32]]) -> [u8; 32] {
    if leaves.is_empty() {
        return [0u8; 32];
    }
    let mut level = leaves.to_vec();
    while level.len() > 1 {
        if !level.len().is_multiple_of(2) {
            let last = *level.last().expect("non-empty");
            level.push(last);
        }
        level = level
            .chunks_exact(2)
            .map(|pair| {
                let mut buf = [0u8; 64];
                buf[..32].copy_from_slice(&pair[0]);
                buf[32..].copy_from_slice(&pair[1]);
                use bitcoin::hashes::{sha256d, Hash};
                sha256d::Hash::hash(&buf).to_byte_array()
            })
            .collect();
    }
    level[0]
}

/// Merkle path for `index`, leaf-to-root.
pub fn merkle_path(leaves: &[[u8; 32]], index: usize) -> Option<Vec<[u8; 32]>> {
    if index >= leaves.len() {
        return None;
    }
    let mut path = Vec::new();
    let mut level = leaves.to_vec();
    let mut idx = index;
    while level.len() > 1 {
        if !level.len().is_multiple_of(2) {
            let last = *level.last().expect("non-empty");
            level.push(last);
        }
        let sibling = idx ^ 1;
        path.push(level[sibling]);
        level = level
            .chunks_exact(2)
            .map(|pair| {
                let mut buf = [0u8; 64];
                buf[..32].copy_from_slice(&pair[0]);
                buf[32..].copy_from_slice(&pair[1]);
                use bitcoin::hashes::{sha256d, Hash};
                sha256d::Hash::hash(&buf).to_byte_array()
            })
            .collect();
        idx /= 2;
    }
    Some(path)
}

/// Recompute the root from a leaf and its path.
pub fn root_from_path(leaf: &[u8; 32], path: &[[u8; 32]], index: usize) -> [u8; 32] {
    let mut cur = *leaf;
    let mut idx = index;
    for sibling in path {
        let mut buf = [0u8; 64];
        if idx.is_multiple_of(2) {
            buf[..32].copy_from_slice(&cur);
            buf[32..].copy_from_slice(sibling);
        } else {
            buf[..32].copy_from_slice(sibling);
            buf[32..].copy_from_slice(&cur);
        }
        use bitcoin::hashes::{sha256d, Hash};
        cur = sha256d::Hash::hash(&buf).to_byte_array();
        idx /= 2;
    }
    cur
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceSummary {
    pub slice_id: u64,
    pub height: u32,
    pub prev_hash: [u8; 32],
    pub job_id: u64,
    pub difficulty: u128,
    pub fees: u64,
    pub root: [u8; 32],
    pub n_shares: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareProof {
    pub leaf: ShareLeaf,
    pub path: Vec<[u8; 32]>,
    /// The share's difficulty `d`.
    pub difficulty: u128,
    /// The job fees `f` carried by this share.
    pub job_fees: u64,
}

/// Verify one sampled slice: every provided share includes in `root`, the
/// summed difficulty does not exceed `difficulty`, and fees respect the bound.
pub fn verify_slice(
    summary: &SliceSummary,
    shares: &[ShareProof],
    fee_delta: u64,
) -> Result<(), Error> {
    let mut sum_d: u128 = 0;
    for share in shares {
        let index = share.leaf.share_index as usize;
        if index >= summary.n_shares as usize {
            return Err(Error::Index);
        }
        let root = root_from_path(&share.leaf.hash(), &share.path, index);
        if root != summary.root {
            return Err(Error::Merkle);
        }
        sum_d = sum_d.saturating_add(share.difficulty);
        if share.job_fees > summary.fees.saturating_add(fee_delta) {
            return Err(Error::Fees);
        }
    }
    if sum_d > summary.difficulty {
        return Err(Error::Difficulty);
    }
    Ok(())
}

/// Deterministic sample selection for a block: `s` slices, `m` shares each.
pub struct Sampler {
    pub slices: usize,
    pub shares: usize,
}

impl Sampler {
    pub fn new(slices: usize, shares: usize) -> Self {
        Self { slices, shares }
    }

    /// Indices chosen by hashing the block hash with the sample number. This
    /// is reproducible for a verifier and testable for the pool.
    pub fn pick(
        &self,
        block_hash: &[u8; 32],
        n_slices: usize,
        n_shares: usize,
    ) -> (Vec<usize>, Vec<usize>) {
        use bitcoin::hashes::{sha256d, Hash};
        let mut slices = Vec::with_capacity(self.slices);
        let mut shares = Vec::with_capacity(self.shares);
        for i in 0..self.slices.min(n_slices) {
            let h = sha256d::Hash::hash(
                &[block_hash.as_slice(), b"/slice", &(i as u64).to_le_bytes()].concat(),
            );
            let b = h.to_byte_array();
            slices
                .push(u64::from_le_bytes(b[..8].try_into().expect("8")) as usize % n_slices.max(1));
        }
        for i in 0..self.shares.min(n_shares) {
            let h = sha256d::Hash::hash(
                &[block_hash.as_slice(), b"/share", &(i as u64).to_le_bytes()].concat(),
            );
            let b = h.to_byte_array();
            shares
                .push(u64::from_le_bytes(b[..8].try_into().expect("8")) as usize % n_shares.max(1));
        }
        (slices, shares)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(i: u32) -> ShareLeaf {
        ShareLeaf {
            nonce: i,
            ntime: 1_700_000_000 + i,
            version: 0x2000_0000,
            extranonce: vec![0xaa; 8],
            job_id: 1,
            share_index: i,
        }
    }

    #[test]
    fn merkle_paths_verify_for_every_index() {
        for n in 1..=17usize {
            let leaves: Vec<[u8; 32]> = (0..n).map(|i| leaf(i as u32).hash()).collect();
            let root = merkle_root(&leaves);
            for i in 0..n {
                let path = merkle_path(&leaves, i).unwrap();
                assert_eq!(root_from_path(&leaves[i], &path, i), root);
            }
        }
    }

    #[test]
    fn wrong_path_fails() {
        let leaves: Vec<[u8; 32]> = (0..4).map(|i| leaf(i).hash()).collect();
        let mut path = merkle_path(&leaves, 0).unwrap();
        path[0][0] ^= 1;
        assert_ne!(root_from_path(&leaves[0], &path, 0), merkle_root(&leaves));
    }

    #[test]
    fn sampler_is_deterministic_and_bounded() {
        let s = Sampler::new(4, 8);
        let block = [7u8; 32];
        assert_eq!(s.pick(&block, 10, 100), s.pick(&block, 10, 100));
        let (a, b) = s.pick(&block, 2, 3);
        assert_eq!(a.len(), 2);
        assert_eq!(b.len(), 3);
        assert!(a.iter().all(|i| *i < 2));
        assert!(b.iter().all(|i| *i < 3));
    }

    #[test]
    fn leaf_hash_is_identity_free_and_stable() {
        let h1 = leaf(0).hash();
        let h2 = leaf(0).hash();
        assert_eq!(h1, h2);
        let mut l = leaf(0);
        l.share_index = 1;
        assert_ne!(l.hash(), h1);
    }
}
