//! Byte-level Redis stream payloads (docs/WIRE-FORMATS.md §3).
//!
//! Every entry has exactly one field, `d`, whose value is the encoding produced
//! here. Decoders reject unknown discriminants and trailing bytes.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::codec::{CodecError, Reader, Result, Writer};

/// The rail a distribution entry was built for (SPEC §4.7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Rail {
    Coinbase = 0,
    Ark = 1,
    Pending = 2,
    Forfeit = 3,
}

impl Rail {
    pub fn from_u8(v: u8) -> Result<Self> {
        match v {
            0 => Ok(Self::Coinbase),
            1 => Ok(Self::Ark),
            2 => Ok(Self::Pending),
            3 => Ok(Self::Forfeit),
            other => Err(CodecError::Discriminant(other)),
        }
    }

    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

/// One distribution entry (SPEC §4.7).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub sp_id: [u8; 32],
    pub rail: Rail,
    /// `ser_x(P_id)`; empty when the id was not kept.
    pub p_x: Vec<u8>,
    /// Ark destination (policy key + delivery); empty when not Ark-capable.
    pub dest_id: Vec<u8>,
    /// `pay(id)` for a kept id, `owed(id)` otherwise.
    pub sats: u64,
}

impl Entry {
    fn encode(&self, w: &mut Writer) -> Result<()> {
        w.bytes(&self.sp_id).u8(self.rail.as_u8());
        w.blob_u8(&self.p_x)?;
        w.blob_u16(&self.dest_id)?;
        w.u64(self.sats);
        Ok(())
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            sp_id: r.array()?,
            rail: Rail::from_u8(r.u8()?)?,
            p_x: r.blob_u8()?.to_vec(),
            dest_id: r.blob_u16()?.to_vec(),
            sats: r.u64()?,
        })
    }
}

/// `spc:blocks` payload (WIRE-FORMATS §3.1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockEvent {
    pub block_hash: [u8; 32],
    pub height: u32,
    pub front_id: u8,
    pub fingerprint: [u8; 32],
    /// Public x-only key of the height's `a_send`; the secret is never stored.
    pub a_send_x: [u8; 32],
    pub high_water: BTreeMap<u8, String>,
    pub entries: Vec<Entry>,
}

impl BlockEvent {
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut w = Writer::new();
        w.bytes(&self.block_hash);
        w.u32(self.height);
        w.u8(self.front_id);
        w.bytes(&self.fingerprint).bytes(&self.a_send_x);
        write_high_water(&mut w, &self.high_water)?;
        w.u16(u16::try_from(self.entries.len()).map_err(|_| CodecError::Range("entries"))?);
        for e in &self.entries {
            e.encode(&mut w)?;
        }
        Ok(w.finish())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes);
        let block_hash = r.array()?;
        let height = r.u32()?;
        let front_id = r.u8()?;
        let fingerprint = r.array()?;
        let a_send_x = r.array()?;
        let high_water = read_high_water(&mut r)?;
        let n = r.u16()? as usize;
        let mut entries = Vec::with_capacity(n.min(4096));
        for _ in 0..n {
            entries.push(Entry::decode(&mut r)?);
        }
        r.finish()?;
        Ok(Self {
            block_hash,
            height,
            front_id,
            fingerprint,
            a_send_x,
            high_water,
            entries,
        })
    }
}

fn write_high_water(w: &mut Writer, hw: &BTreeMap<u8, String>) -> Result<()> {
    w.u16(u16::try_from(hw.len()).map_err(|_| CodecError::Range("high_water"))?);
    for (front, id) in hw {
        w.u8(*front);
        w.blob_u8(id.as_bytes())?;
    }
    Ok(())
}

fn read_high_water(r: &mut Reader<'_>) -> Result<BTreeMap<u8, String>> {
    let n = r.u16()? as usize;
    let mut out = BTreeMap::new();
    for _ in 0..n {
        let front = r.u8()?;
        let id = std::str::from_utf8(r.blob_u8()?)
            .map_err(|_| CodecError::Range("high_water utf8"))?
            .to_string();
        out.insert(front, id);
    }
    Ok(out)
}

/// `spc:front:events` payloads (WIRE-FORMATS §3.2). Every payload starts with
/// a type byte and a payout-assigned revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrontEvent {
    ArkRegApplied {
        rev: u64,
        sp_id: [u8; 32],
    },
    LedgerApplied {
        rev: u64,
        block_hash: [u8; 32],
        republish: bool,
        pending_deltas: Vec<([u8; 32], i64)>,
    },
    BlockFound {
        rev: u64,
        block_hash: [u8; 32],
        height: u32,
        front_id: u8,
        provisional_paids: Vec<([u8; 32], u64)>,
        high_water: BTreeMap<u8, String>,
    },
    TipHeartbeat {
        rev: u64,
        tip_hash: [u8; 32],
        height: u32,
        unix_ms: u64,
    },
}

impl FrontEvent {
    pub fn rev(&self) -> u64 {
        match self {
            Self::ArkRegApplied { rev, .. }
            | Self::LedgerApplied { rev, .. }
            | Self::BlockFound { rev, .. }
            | Self::TipHeartbeat { rev, .. } => *rev,
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut w = Writer::new();
        match self {
            Self::ArkRegApplied { rev, sp_id } => {
                w.u8(0x01).u64(*rev).bytes(sp_id);
            }
            Self::LedgerApplied {
                rev,
                block_hash,
                republish,
                pending_deltas,
            } => {
                w.u8(0x02)
                    .u64(*rev)
                    .bytes(block_hash)
                    .u8(u8::from(*republish));
                w.u16(
                    u16::try_from(pending_deltas.len())
                        .map_err(|_| CodecError::Range("pending_deltas"))?,
                );
                for (id, delta) in pending_deltas {
                    w.bytes(id).i64(*delta);
                }
            }
            Self::BlockFound {
                rev,
                block_hash,
                height,
                front_id,
                provisional_paids,
                high_water,
            } => {
                w.u8(0x03)
                    .u64(*rev)
                    .bytes(block_hash)
                    .u32(*height)
                    .u8(*front_id);
                w.u16(
                    u16::try_from(provisional_paids.len())
                        .map_err(|_| CodecError::Range("provisional_paids"))?,
                );
                for (id, sats) in provisional_paids {
                    w.bytes(id).u64(*sats);
                }
                write_high_water(&mut w, high_water)?;
            }
            Self::TipHeartbeat {
                rev,
                tip_hash,
                height,
                unix_ms,
            } => {
                w.u8(0x04)
                    .u64(*rev)
                    .bytes(tip_hash)
                    .u32(*height)
                    .u64(*unix_ms);
            }
        }
        Ok(w.finish())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes);
        let ty = r.u8()?;
        let rev = r.u64()?;
        let event = match ty {
            0x01 => Self::ArkRegApplied {
                rev,
                sp_id: r.array()?,
            },
            0x02 => {
                let block_hash = r.array()?;
                let republish = r.u8()? != 0;
                let n = r.u16()? as usize;
                let mut pending_deltas = Vec::with_capacity(n.min(4096));
                for _ in 0..n {
                    pending_deltas.push((r.array()?, r.i64()?));
                }
                Self::LedgerApplied {
                    rev,
                    block_hash,
                    republish,
                    pending_deltas,
                }
            }
            0x03 => {
                let block_hash = r.array()?;
                let height = r.u32()?;
                let front_id = r.u8()?;
                let n = r.u16()? as usize;
                let mut provisional_paids = Vec::with_capacity(n.min(4096));
                for _ in 0..n {
                    provisional_paids.push((r.array()?, r.u64()?));
                }
                let high_water = read_high_water(&mut r)?;
                Self::BlockFound {
                    rev,
                    block_hash,
                    height,
                    front_id,
                    provisional_paids,
                    high_water,
                }
            }
            0x04 => Self::TipHeartbeat {
                rev,
                tip_hash: r.array()?,
                height: r.u32()?,
                unix_ms: r.u64()?,
            },
            other => return Err(CodecError::Discriminant(other)),
        };
        r.finish()?;
        Ok(event)
    }
}

/// `spc:arkreg` payload (WIRE-FORMATS §3.3), kind 0.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArkRegEvent {
    pub sp_id: [u8; 32],
    pub b_scan: [u8; 33],
    pub b_spend: [u8; 33],
    pub ark_id: [u8; 4],
    pub b_ark: [u8; 33],
    pub mailbox_mode: u8,
    pub mailbox_pubkey: Vec<u8>,
    pub mailbox_dh: Vec<u8>,
    pub epoch_fallback: u8,
    pub sig: [u8; 64],
}

impl ArkRegEvent {
    pub const KIND: u8 = 0;

    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut w = Writer::new();
        w.u8(Self::KIND)
            .bytes(&self.sp_id)
            .bytes(&self.b_scan)
            .bytes(&self.b_spend);
        w.bytes(&self.ark_id)
            .bytes(&self.b_ark)
            .u8(self.mailbox_mode);
        w.blob_u8(&self.mailbox_pubkey)?;
        w.blob_u8(&self.mailbox_dh)?;
        w.u8(self.epoch_fallback).bytes(&self.sig);
        Ok(w.finish())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes);
        let kind = r.u8()?;
        if kind != Self::KIND {
            return Err(CodecError::Discriminant(kind));
        }
        let out = Self {
            sp_id: r.array()?,
            b_scan: r.array()?,
            b_spend: r.array()?,
            ark_id: r.array()?,
            b_ark: r.array()?,
            mailbox_mode: r.u8()?,
            mailbox_pubkey: r.blob_u8()?.to_vec(),
            mailbox_dh: r.blob_u8()?.to_vec(),
            epoch_fallback: r.u8()?,
            sig: r.array()?,
        };
        r.finish()?;
        Ok(out)
    }
}

/// `spc:slices` payload. WIRE-FORMATS §3.6 (added with this implementation):
/// a sealed slice header, its identity-free share blob, and per-identity
/// aggregates. The share blob is opaque here; `spc-accounting` owns its
/// contents.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceEvent {
    pub slice_id: u64,
    pub height: u32,
    pub prev_hash: [u8; 32],
    pub ref_job_id: u64,
    pub ref_fees: u64,
    pub n_shares: u32,
    pub difficulty: u128,
    pub root: [u8; 32],
    pub shares_blob: Vec<u8>,
    pub credits: Vec<SliceCredit>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceCredit {
    pub sp_id: [u8; 32],
    pub diff_sum: u128,
    pub diff_fee_sum: u128,
}

impl SliceEvent {
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut w = Writer::new();
        w.u64(self.slice_id).u32(self.height).bytes(&self.prev_hash);
        w.u64(self.ref_job_id).u64(self.ref_fees).u32(self.n_shares);
        w.u128(self.difficulty).bytes(&self.root);
        w.blob_u32(&self.shares_blob)?;
        w.u16(u16::try_from(self.credits.len()).map_err(|_| CodecError::Range("credits"))?);
        for c in &self.credits {
            w.bytes(&c.sp_id).u128(c.diff_sum).u128(c.diff_fee_sum);
        }
        Ok(w.finish())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes);
        let slice_id = r.u64()?;
        let height = r.u32()?;
        let prev_hash = r.array()?;
        let ref_job_id = r.u64()?;
        let ref_fees = r.u64()?;
        let n_shares = r.u32()?;
        let difficulty = r.u128()?;
        let root = r.array()?;
        let shares_blob = r.blob_u32()?.to_vec();
        let n = r.u16()? as usize;
        let mut credits = Vec::with_capacity(n.min(4096));
        for _ in 0..n {
            credits.push(SliceCredit {
                sp_id: r.array()?,
                diff_sum: r.u128()?,
                diff_fee_sum: r.u128()?,
            });
        }
        r.finish()?;
        Ok(Self {
            slice_id,
            height,
            prev_hash,
            ref_job_id,
            ref_fees,
            n_shares,
            difficulty,
            root,
            shares_blob,
            credits,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u8) -> Entry {
        Entry {
            sp_id: [id; 32],
            rail: Rail::Coinbase,
            p_x: vec![id; 32],
            dest_id: vec![],
            sats: 1000 + id as u64,
        }
    }

    #[test]
    fn block_event_round_trip() {
        let e = BlockEvent {
            block_hash: [1; 32],
            height: 800_000,
            front_id: 3,
            fingerprint: [2; 32],
            a_send_x: [3; 32],
            high_water: BTreeMap::from([(0u8, "1-1".to_string()), (3, "9-2".to_string())]),
            entries: vec![entry(1), entry(2)],
        };
        let bytes = e.encode().unwrap();
        assert_eq!(BlockEvent::decode(&bytes).unwrap(), e);
        let mut bad = bytes.clone();
        bad.push(0);
        assert_eq!(BlockEvent::decode(&bad), Err(CodecError::Trailing));
    }

    #[test]
    fn front_events_round_trip() {
        let events = vec![
            FrontEvent::ArkRegApplied {
                rev: 1,
                sp_id: [1; 32],
            },
            FrontEvent::LedgerApplied {
                rev: 2,
                block_hash: [2; 32],
                republish: true,
                pending_deltas: vec![([3; 32], -5), ([4; 32], 9)],
            },
            FrontEvent::BlockFound {
                rev: 3,
                block_hash: [5; 32],
                height: 10,
                front_id: 1,
                provisional_paids: vec![([6; 32], 7)],
                high_water: BTreeMap::from([(1u8, "3-4".to_string())]),
            },
            FrontEvent::TipHeartbeat {
                rev: 4,
                tip_hash: [7; 32],
                height: 11,
                unix_ms: 12,
            },
        ];
        for e in events {
            let bytes = e.encode().unwrap();
            assert_eq!(FrontEvent::decode(&bytes).unwrap(), e);
        }
    }

    #[test]
    fn arkreg_round_trip() {
        let e = ArkRegEvent {
            sp_id: [1; 32],
            b_scan: [2; 33],
            b_spend: [3; 33],
            ark_id: [4; 4],
            b_ark: [5; 33],
            mailbox_mode: 2,
            mailbox_pubkey: vec![6; 33],
            mailbox_dh: vec![7; 33],
            epoch_fallback: 0,
            sig: [8; 64],
        };
        let bytes = e.encode().unwrap();
        assert_eq!(ArkRegEvent::decode(&bytes).unwrap(), e);
    }

    #[test]
    fn slice_event_round_trip() {
        let e = SliceEvent {
            slice_id: 42,
            height: 800_000,
            prev_hash: [1; 32],
            ref_job_id: 7,
            ref_fees: 50_000,
            n_shares: 10,
            difficulty: 12345678901234567890,
            root: [2; 32],
            shares_blob: vec![3; 100],
            credits: vec![SliceCredit {
                sp_id: [4; 32],
                diff_sum: 5,
                diff_fee_sum: 6,
            }],
        };
        let bytes = e.encode().unwrap();
        assert_eq!(SliceEvent::decode(&bytes).unwrap(), e);
    }
}
