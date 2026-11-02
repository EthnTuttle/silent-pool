//! Translator data shape. Kept separate so `spc-agent` can reuse the pool's
//! `sv1::Job` semantics without depending on the pool binary.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sv1Job {
    pub job_id: String,
    pub prev_hash: [u8; 32],
    pub height: u32,
    pub version: u32,
    pub nbits: u32,
    pub ntime: u32,
    pub coinb1: Vec<u8>,
    pub coinb2: Vec<u8>,
    pub merkle_branches: Vec<[u8; 32]>,
    pub clean_jobs: bool,
}
