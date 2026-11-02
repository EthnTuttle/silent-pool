//! Type-32 message set (project dialect; see
//! `docs/share-accounting-ext-deltas.md`).

use crate::{Error, ShareLeaf, ShareProof, SliceSummary};

pub const EXTENSION_TYPE: u16 = 0x0020;

pub const MSG_NEW_BLOCK_FOUND: u8 = 0x00;
pub const MSG_GET_WINDOW: u8 = 0x01;
pub const MSG_GET_WINDOW_SUCCESS: u8 = 0x02;
pub const MSG_GET_WINDOW_BUSY: u8 = 0x03;
pub const MSG_GET_SHARES: u8 = 0x04;
pub const MSG_GET_SHARES_SUCCESS: u8 = 0x05;
pub const MSG_GET_TRANSACTIONS_IN_JOB: u8 = 0x06;
pub const MSG_GET_TRANSACTIONS_IN_JOB_SUCCESS: u8 = 0x07;
pub const MSG_ERROR: u8 = 0x08;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    NewBlockFound {
        block_hash: [u8; 32],
    },
    GetWindow {
        request_id: u32,
        block_hash: [u8; 32],
    },
    GetWindowSuccess {
        request_id: u32,
        slices: Vec<SliceSummary>,
    },
    GetWindowBusy {
        request_id: u32,
        retry_in_seconds: u32,
    },
    GetShares {
        request_id: u32,
        slice_id: u64,
        indices: Vec<u32>,
    },
    GetSharesSuccess {
        request_id: u32,
        shares: Vec<ShareProof>,
    },
    GetTransactionsInJob {
        request_id: u32,
        job_id: u64,
    },
    GetTransactionsInJobSuccess {
        request_id: u32,
        txids: Vec<[u8; 32]>,
    },
    Error {
        request_id: u32,
        code: String,
    },
}

impl Message {
    pub fn message_type(&self) -> u8 {
        match self {
            Self::NewBlockFound { .. } => MSG_NEW_BLOCK_FOUND,
            Self::GetWindow { .. } => MSG_GET_WINDOW,
            Self::GetWindowSuccess { .. } => MSG_GET_WINDOW_SUCCESS,
            Self::GetWindowBusy { .. } => MSG_GET_WINDOW_BUSY,
            Self::GetShares { .. } => MSG_GET_SHARES,
            Self::GetSharesSuccess { .. } => MSG_GET_SHARES_SUCCESS,
            Self::GetTransactionsInJob { .. } => MSG_GET_TRANSACTIONS_IN_JOB,
            Self::GetTransactionsInJobSuccess { .. } => MSG_GET_TRANSACTIONS_IN_JOB_SUCCESS,
            Self::Error { .. } => MSG_ERROR,
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        let mut w = Writer::default();
        match self {
            Self::NewBlockFound { block_hash } => {
                w.bytes(block_hash);
            }
            Self::GetWindow {
                request_id,
                block_hash,
            } => {
                w.u32(*request_id).bytes(block_hash);
            }
            Self::GetWindowSuccess { request_id, slices } => {
                w.u32(*request_id)
                    .u16(u16::try_from(slices.len()).map_err(|_| Error::Length)?);
                for s in slices {
                    w.u64(s.slice_id)
                        .u32(s.height)
                        .bytes(&s.prev_hash)
                        .u64(s.job_id)
                        .u128(s.difficulty)
                        .u64(s.fees)
                        .bytes(&s.root)
                        .u32(s.n_shares);
                }
            }
            Self::GetWindowBusy {
                request_id,
                retry_in_seconds,
            } => {
                w.u32(*request_id).u32(*retry_in_seconds);
            }
            Self::GetShares {
                request_id,
                slice_id,
                indices,
            } => {
                w.u32(*request_id)
                    .u64(*slice_id)
                    .u16(u16::try_from(indices.len()).map_err(|_| Error::Length)?);
                for i in indices {
                    w.u32(*i);
                }
            }
            Self::GetSharesSuccess { request_id, shares } => {
                w.u32(*request_id)
                    .u16(u16::try_from(shares.len()).map_err(|_| Error::Length)?);
                for s in shares {
                    w.u32(s.leaf.nonce)
                        .u32(s.leaf.ntime)
                        .u32(s.leaf.version)
                        .blob_u16(&s.leaf.extranonce)?
                        .u64(s.leaf.job_id)
                        .u32(s.leaf.share_index)
                        .u32(s.difficulty as u32)
                        .u64(s.job_fees)
                        .u16(u16::try_from(s.path.len()).map_err(|_| Error::Length)?);
                    for p in &s.path {
                        w.bytes(p);
                    }
                }
            }
            Self::GetTransactionsInJob { request_id, job_id } => {
                w.u32(*request_id).u64(*job_id);
            }
            Self::GetTransactionsInJobSuccess { request_id, txids } => {
                w.u32(*request_id)
                    .u16(u16::try_from(txids.len()).map_err(|_| Error::Length)?);
                for t in txids {
                    w.bytes(t);
                }
            }
            Self::Error { request_id, code } => {
                w.u32(*request_id).blob_u16(code.as_bytes())?;
            }
        }
        Ok(w.finish())
    }

    pub fn decode(msg_type: u8, bytes: &[u8]) -> Result<Self, Error> {
        let mut r = Reader::new(bytes);
        let msg = match msg_type {
            MSG_NEW_BLOCK_FOUND => Self::NewBlockFound {
                block_hash: r.array()?,
            },
            MSG_GET_WINDOW => Self::GetWindow {
                request_id: r.u32()?,
                block_hash: r.array()?,
            },
            MSG_GET_WINDOW_SUCCESS => {
                let request_id = r.u32()?;
                let n = r.u16()? as usize;
                let mut slices = Vec::with_capacity(n.min(4096));
                for _ in 0..n {
                    slices.push(SliceSummary {
                        slice_id: r.u64()?,
                        height: r.u32()?,
                        prev_hash: r.array()?,
                        job_id: r.u64()?,
                        difficulty: r.u128()?,
                        fees: r.u64()?,
                        root: r.array()?,
                        n_shares: r.u32()?,
                    });
                }
                Self::GetWindowSuccess { request_id, slices }
            }
            MSG_GET_WINDOW_BUSY => Self::GetWindowBusy {
                request_id: r.u32()?,
                retry_in_seconds: r.u32()?,
            },
            MSG_GET_SHARES => {
                let request_id = r.u32()?;
                let slice_id = r.u64()?;
                let n = r.u16()? as usize;
                let mut indices = Vec::with_capacity(n.min(4096));
                for _ in 0..n {
                    indices.push(r.u32()?);
                }
                Self::GetShares {
                    request_id,
                    slice_id,
                    indices,
                }
            }
            MSG_GET_SHARES_SUCCESS => {
                let request_id = r.u32()?;
                let n = r.u16()? as usize;
                let mut shares = Vec::with_capacity(n.min(4096));
                for _ in 0..n {
                    let leaf = ShareLeaf {
                        nonce: r.u32()?,
                        ntime: r.u32()?,
                        version: r.u32()?,
                        extranonce: r.blob_u16()?.to_vec(),
                        job_id: r.u64()?,
                        share_index: r.u32()?,
                    };
                    let difficulty = r.u32()? as u128;
                    let job_fees = r.u64()?;
                    let pn = r.u16()? as usize;
                    let mut path = Vec::with_capacity(pn.min(4096));
                    for _ in 0..pn {
                        path.push(r.array()?);
                    }
                    shares.push(ShareProof {
                        leaf,
                        path,
                        difficulty,
                        job_fees,
                    });
                }
                Self::GetSharesSuccess { request_id, shares }
            }
            MSG_GET_TRANSACTIONS_IN_JOB => Self::GetTransactionsInJob {
                request_id: r.u32()?,
                job_id: r.u64()?,
            },
            MSG_GET_TRANSACTIONS_IN_JOB_SUCCESS => {
                let request_id = r.u32()?;
                let n = r.u16()? as usize;
                let mut txids = Vec::with_capacity(n.min(4096));
                for _ in 0..n {
                    txids.push(r.array()?);
                }
                Self::GetTransactionsInJobSuccess { request_id, txids }
            }
            MSG_ERROR => Self::Error {
                request_id: r.u32()?,
                code: String::from_utf8(r.blob_u16()?.to_vec()).map_err(|_| Error::Length)?,
            },
            _ => return Err(Error::MessageType(msg_type)),
        };
        r.finish()?;
        Ok(msg)
    }
}

#[derive(Default)]
struct Writer(Vec<u8>);
impl Writer {
    fn u16(&mut self, v: u16) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u64(&mut self, v: u64) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u128(&mut self, v: u128) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn bytes(&mut self, b: &[u8]) -> &mut Self {
        self.0.extend_from_slice(b);
        self
    }
    fn blob_u16(&mut self, b: &[u8]) -> Result<&mut Self, Error> {
        let n = u16::try_from(b.len()).map_err(|_| Error::Length)?;
        self.u16(n).bytes(b);
        Ok(self)
    }
    fn finish(self) -> Vec<u8> {
        self.0
    }
}

struct Reader<'a> {
    b: &'a [u8],
    p: usize,
}
impl<'a> Reader<'a> {
    fn new(b: &'a [u8]) -> Self {
        Self { b, p: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self.p.checked_add(n).ok_or(Error::Length)?;
        let out = self.b.get(self.p..end).ok_or(Error::Eof)?;
        self.p = end;
        Ok(out)
    }
    fn u16(&mut self) -> Result<u16, Error> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Result<u32, Error> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn u64(&mut self) -> Result<u64, Error> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes(b.try_into().map_err(|_| Error::Length)?))
    }
    fn u128(&mut self) -> Result<u128, Error> {
        let b = self.take(16)?;
        Ok(u128::from_le_bytes(
            b.try_into().map_err(|_| Error::Length)?,
        ))
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        self.take(N)?.try_into().map_err(|_| Error::Length)
    }
    fn blob_u16(&mut self) -> Result<&'a [u8], Error> {
        let n = self.u16()? as usize;
        self.take(n)
    }
    fn finish(self) -> Result<(), Error> {
        if self.p == self.b.len() {
            Ok(())
        } else {
            Err(Error::Trailing)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary() -> SliceSummary {
        SliceSummary {
            slice_id: 7,
            height: 800_000,
            prev_hash: [1; 32],
            job_id: 9,
            difficulty: 12345678901234567890,
            fees: 50_000,
            root: [2; 32],
            n_shares: 10,
        }
    }

    #[test]
    fn window_round_trip() {
        let m = Message::GetWindowSuccess {
            request_id: 3,
            slices: vec![summary()],
        };
        let b = m.encode().unwrap();
        assert_eq!(Message::decode(m.message_type(), &b).unwrap(), m);
    }

    #[test]
    fn shares_round_trip() {
        let m = Message::GetSharesSuccess {
            request_id: 1,
            shares: vec![ShareProof {
                leaf: ShareLeaf {
                    nonce: 1,
                    ntime: 2,
                    version: 3,
                    extranonce: vec![4; 8],
                    job_id: 5,
                    share_index: 6,
                },
                path: vec![[7; 32], [8; 32]],
                difficulty: 9,
                job_fees: 10,
            }],
        };
        let b = m.encode().unwrap();
        assert_eq!(Message::decode(m.message_type(), &b).unwrap(), m);
    }

    #[test]
    fn unknown_type_fails() {
        assert_eq!(Message::decode(0xff, &[]), Err(Error::MessageType(0xff)));
    }
}
