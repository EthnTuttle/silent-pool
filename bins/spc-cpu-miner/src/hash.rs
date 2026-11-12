use std::time::Instant;

use bitcoin::hashes::{sha256d as bitcoin_sha256d, Hash as _};
use bitcoin::BlockHash;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NtimePolicy {
    #[default]
    Hold,
    Roll,
}

impl NtimePolicy {
    /// `min_ntime + elapsed` is the SV2 rolling ceiling. Holding is valid and
    /// independent of pacing; neither policy delays a found solution.
    pub fn ntime(
        self,
        min_ntime: u32,
        activated_at: Instant,
        now: Instant,
        max_roll_seconds: u32,
    ) -> u32 {
        match self {
            Self::Hold => min_ntime,
            Self::Roll => {
                let elapsed = now.saturating_duration_since(activated_at).as_secs();
                let roll = elapsed.min(u64::from(max_roll_seconds));
                min_ntime.saturating_add(roll.min(u64::from(u32::MAX)) as u32)
            }
        }
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum HashError {
    #[error("miner rolling requires at least 4 extranonce bytes, got {0}")]
    ExtranonceTooShort(usize),
    #[error("worker index {index} is outside worker count {count}")]
    WorkerPartition { index: u32, count: u32 },
    #[error("reduced nonce domain is empty")]
    EmptyNonceDomain,
}

pub fn sha256d(bytes: &[u8]) -> [u8; 32] {
    bitcoin_sha256d::Hash::hash(bytes).to_byte_array()
}

pub fn assemble_coinbase(
    prefix: &[u8],
    extranonce_prefix: &[u8],
    extranonce: &[u8],
    suffix: &[u8],
) -> Vec<u8> {
    let mut coinbase = Vec::with_capacity(
        prefix.len() + extranonce_prefix.len() + extranonce.len() + suffix.len(),
    );
    coinbase.extend_from_slice(prefix);
    coinbase.extend_from_slice(extranonce_prefix);
    coinbase.extend_from_slice(extranonce);
    coinbase.extend_from_slice(suffix);
    coinbase
}

pub fn merkle_root(
    prefix: &[u8],
    extranonce_prefix: &[u8],
    extranonce: &[u8],
    suffix: &[u8],
    deepest_first_path: &[[u8; 32]],
) -> [u8; 32] {
    let coinbase = assemble_coinbase(prefix, extranonce_prefix, extranonce, suffix);
    merkle_root_for_coinbase(&coinbase, deepest_first_path)
}

pub fn merkle_root_for_coinbase(coinbase: &[u8], deepest_first_path: &[[u8; 32]]) -> [u8; 32] {
    let mut root = sha256d(coinbase);
    // SV2 supplies deepest-first siblings. Do not reverse this sequence.
    for sibling in deepest_first_path {
        let mut pair = [0u8; 64];
        pair[..32].copy_from_slice(&root);
        pair[32..].copy_from_slice(sibling);
        root = sha256d(&pair);
    }
    root
}

#[allow(clippy::too_many_arguments)]
pub fn assemble_header(
    version: u32,
    prev_hash: [u8; 32],
    coinbase: &[u8],
    deepest_first_path: &[[u8; 32]],
    ntime: u32,
    nbits: u32,
    nonce: u32,
) -> [u8; 80] {
    let merkle_root = merkle_root_for_coinbase(coinbase, deepest_first_path);
    let mut header = [0u8; 80];
    header[..4].copy_from_slice(&version.to_le_bytes());
    header[4..36].copy_from_slice(&prev_hash);
    header[36..68].copy_from_slice(&merkle_root);
    header[68..72].copy_from_slice(&ntime.to_le_bytes());
    header[72..76].copy_from_slice(&nbits.to_le_bytes());
    header[76..80].copy_from_slice(&nonce.to_le_bytes());
    header
}

/// An assembled coinbase/header prefix cached until extranonce or ntime moves.
#[derive(Clone, Debug)]
pub struct HeaderPreimage {
    header: [u8; 80],
}

impl HeaderPreimage {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        version: u32,
        prev_hash: [u8; 32],
        coinbase: &[u8],
        deepest_first_path: &[[u8; 32]],
        ntime: u32,
        nbits: u32,
    ) -> Self {
        Self {
            header: assemble_header(
                version,
                prev_hash,
                coinbase,
                deepest_first_path,
                ntime,
                nbits,
                0,
            ),
        }
    }

    pub fn hash_nonce(&self, nonce: u32) -> ([u8; 80], BlockHash) {
        let mut header = self.header;
        header[76..80].copy_from_slice(&nonce.to_le_bytes());
        let hash = BlockHash::from_byte_array(sha256d(&header));
        (header, hash)
    }
}

/// Disjoint nonce partition plus a unique worker/session extranonce allocation.
/// The first two bytes carry the reconnect-unique session slot, the third the
/// connection-local worker index, and all remaining bytes the roll counter.
#[derive(Clone, Debug)]
pub struct SearchCursor {
    nonce_start: u64,
    nonce_stride: u64,
    nonce_domain_size: u64,
    nonce_index: u64,
    extranonce: Vec<u8>,
    exhausted: bool,
}

impl SearchCursor {
    pub fn new(
        worker_slot: u16,
        session_slot: u16,
        worker_index: u32,
        worker_count: u32,
        extranonce_size: usize,
    ) -> Result<Self, HashError> {
        Self::with_nonce_domain_size(
            worker_slot,
            session_slot,
            worker_index,
            worker_count,
            extranonce_size,
            1u64 << 32,
        )
    }

    /// Reduced domains are for unit tests. The production constructor above
    /// always uses the complete 2^32 nonce domain.
    pub fn with_nonce_domain_size(
        _worker_slot: u16,
        session_slot: u16,
        worker_index: u32,
        worker_count: u32,
        extranonce_size: usize,
        nonce_space_size: u64,
    ) -> Result<Self, HashError> {
        if extranonce_size < 4 || worker_index > u32::from(u8::MAX) {
            return Err(HashError::ExtranonceTooShort(extranonce_size));
        }
        if worker_count == 0 || worker_index >= worker_count {
            return Err(HashError::WorkerPartition {
                index: worker_index,
                count: worker_count,
            });
        }
        if nonce_space_size == 0 || nonce_space_size > (1u64 << 32) {
            return Err(HashError::EmptyNonceDomain);
        }
        let start = u64::from(worker_index);
        let stride = u64::from(worker_count);
        let domain_size = if start >= nonce_space_size {
            0
        } else {
            (nonce_space_size - start).div_ceil(stride)
        };
        let mut extranonce = vec![0u8; extranonce_size];
        extranonce[..2].copy_from_slice(&session_slot.to_be_bytes());
        extranonce[2] = worker_index as u8;
        Ok(Self {
            nonce_start: start,
            nonce_stride: stride,
            nonce_domain_size: domain_size,
            nonce_index: 0,
            extranonce,
            exhausted: domain_size == 0,
        })
    }

    pub fn extranonce(&self) -> &[u8] {
        &self.extranonce
    }

    pub fn next_nonce(&mut self) -> Option<u32> {
        if self.exhausted || self.nonce_index >= self.nonce_domain_size {
            self.exhausted = true;
            return None;
        }
        let nonce = self.nonce_start + self.nonce_index * self.nonce_stride;
        self.nonce_index += 1;
        Some(nonce as u32)
    }

    /// Move to the next coinbase after the partition's nonce domain is
    /// exhausted. Counter overflow closes the search rather than wrapping to a
    /// previously hashed header.
    pub fn roll_extranonce(&mut self) -> bool {
        if !self.exhausted {
            return false;
        }
        for byte in self.extranonce[3..].iter_mut().rev() {
            let (next, carry) = byte.overflowing_add(1);
            *byte = next;
            if !carry {
                self.nonce_index = 0;
                self.exhausted = false;
                return true;
            }
        }
        false
    }

    pub fn reset_nonce_for_new_time(&mut self) {
        self.nonce_index = 0;
        self.exhausted = self.nonce_domain_size == 0;
    }

    pub fn set_extranonce_counter_for_test(&mut self, counter: &[u8]) {
        assert_eq!(counter.len(), self.extranonce.len() - 3);
        self.extranonce[3..].copy_from_slice(counter);
    }
}
