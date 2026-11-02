//! Front bootstrap RPC framing (WIRE-FORMATS §3.5).
//!
//! Transport is a `U32` little-endian length followed by a JSON body. The
//! server lives in the `payout` role; the client is a `front` at start.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::models::*;

pub const MAX_FRAME: usize = 64 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("frame length {0} exceeds maximum")]
    TooLarge(u32),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BootstrapRequest {
    pub front_id: u8,
    pub instance: String,
    /// Last seen stream id per stream name, or null.
    pub cursors: BTreeMap<String, Option<String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BootstrapResponse {
    pub rev: u64,
    pub pending_balance: Vec<PendingBalanceJson>,
    pub ark_delivery: Vec<ArkDeliveryJson>,
    pub window_tail: Vec<SliceJson>,
    /// Base64-encoded share blobs for the type-32 service.
    pub type32_blobs: Vec<BlobJson>,
    pub resume_cursors: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingBalanceJson {
    pub sp_id: String,
    pub sats: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArkDeliveryJson {
    pub sp_id: String,
    pub ark_id: String,
    pub b_ark: String,
    pub mailbox_mode: String,
    pub mailbox_m: Option<String>,
    pub mailbox_d: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SliceJson {
    pub slice_id: u64,
    pub height: u32,
    pub prev_hash: String,
    pub ref_job_id: u64,
    pub ref_fees: u64,
    pub n_shares: u32,
    pub difficulty: String,
    pub root: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlobJson {
    pub slice_id: u64,
    /// Base64 of the identity-free share blob.
    pub shares: String,
}

/// Write one length-prefixed JSON frame.
pub async fn write_frame<W: AsyncWrite + Unpin, T: Serialize>(
    w: &mut W,
    value: &T,
) -> Result<(), FrameError> {
    let body = serde_json::to_vec(value)?;
    let len = u32::try_from(body.len()).map_err(|_| FrameError::TooLarge(u32::MAX))?;
    if body.len() > MAX_FRAME {
        return Err(FrameError::TooLarge(len));
    }
    w.write_all(&len.to_le_bytes()).await?;
    w.write_all(&body).await?;
    w.flush().await?;
    Ok(())
}

/// Read one length-prefixed JSON frame.
pub async fn read_frame<R: AsyncRead + Unpin, T: for<'de> Deserialize<'de>>(
    r: &mut R,
) -> Result<T, FrameError> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len).await?;
    let len = u32::from_le_bytes(len);
    if len as usize > MAX_FRAME {
        return Err(FrameError::TooLarge(len));
    }
    let mut body = vec![0u8; len as usize];
    r.read_exact(&mut body).await?;
    Ok(serde_json::from_slice(&body)?)
}

fn hex32(v: &[u8; 32]) -> String {
    v.iter().map(|b| format!("{b:02x}")).collect()
}

fn hex(v: &[u8]) -> String {
    v.iter().map(|b| format!("{b:02x}")).collect()
}

impl BootstrapResponse {
    pub fn from_snapshot(snap: BootstrapSnapshot, base64: impl Fn(&[u8]) -> String) -> Self {
        Self {
            rev: snap.rev,
            pending_balance: snap
                .pending_balance
                .into_iter()
                .map(|(sp_id, sats)| PendingBalanceJson {
                    sp_id: hex32(&sp_id),
                    sats,
                })
                .collect(),
            ark_delivery: snap
                .ark_delivery
                .into_iter()
                .map(|d| ArkDeliveryJson {
                    sp_id: hex32(&d.sp_id),
                    ark_id: hex(&d.ark_id),
                    b_ark: hex(&d.b_ark),
                    mailbox_mode: d.mailbox_mode.as_str().to_string(),
                    mailbox_m: d.mailbox_m.as_deref().map(hex),
                    mailbox_d: d.mailbox_d.as_deref().map(hex),
                })
                .collect(),
            window_tail: snap
                .window_tail
                .into_iter()
                .map(|s| SliceJson {
                    slice_id: s.slice_id,
                    height: s.height,
                    prev_hash: hex32(&s.prev_hash),
                    ref_job_id: s.ref_job_id,
                    ref_fees: s.ref_fees,
                    n_shares: s.n_shares,
                    difficulty: s.difficulty,
                    root: hex32(&s.root),
                })
                .collect(),
            type32_blobs: snap
                .type32_blobs
                .into_iter()
                .map(|(slice_id, shares)| BlobJson {
                    slice_id,
                    shares: base64(&shares),
                })
                .collect(),
            resume_cursors: snap
                .slices_cursor
                .into_iter()
                .map(|c| ("spc:slices".to_string(), c))
                .chain(
                    snap.front_events_cursor
                        .into_iter()
                        .map(|c| ("spc:front:events".to_string(), c)),
                )
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn frame_round_trip() {
        let req = BootstrapRequest {
            front_id: 1,
            instance: "uuid".into(),
            cursors: BTreeMap::from([("spc:slices".to_string(), Some("1-2".to_string()))]),
        };
        let mut buf = Vec::new();
        write_frame(&mut buf, &req).await.unwrap();
        let mut cursor: &[u8] = &buf;
        let got: BootstrapRequest = read_frame(&mut cursor).await.unwrap();
        assert_eq!(got, req);
    }
}
