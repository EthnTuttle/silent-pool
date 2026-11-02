//! Type-32 audit client: request a window, sample slices/shares, verify the
//! merkle paths and the difficulty/fee bounds.

use std::net::SocketAddr;

use spc_share_ext::{verify_slice, Message, ShareProof, SliceSummary, EXTENSION_TYPE};
use stratum_apps::network_helpers::connect_with_noise;
use stratum_apps::network_helpers::noise_stream::{NoiseTcpReadHalf, NoiseTcpWriteHalf};
use stratum_core::binary_sv2::{self, Str0255};
use stratum_core::common_messages_sv2::{
    SetupConnection, SetupConnectionSuccess, MESSAGE_TYPE_SETUP_CONNECTION,
    MESSAGE_TYPE_SETUP_CONNECTION_SUCCESS,
};
use stratum_core::framing_sv2::framing::MessageFrame;
use thiserror::Error;
use tokio::net::TcpStream;

#[derive(Debug, Error)]
pub enum AuditError {
    #[error("network: {0}")]
    Network(String),
    #[error("codec: {0}")]
    Codec(String),
    #[error("verification: {0}")]
    Verify(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditReport {
    pub slices: usize,
    pub shares: usize,
    pub verified: usize,
}

pub struct Auditor {
    read: NoiseTcpReadHalf,
    write: NoiseTcpWriteHalf,
}

impl Auditor {
    pub async fn connect(addr: SocketAddr) -> Result<Self, AuditError> {
        let stream = TcpStream::connect(addr)
            .await
            .map_err(|e| AuditError::Network(e.to_string()))?;
        let stream = connect_with_noise(stream, None)
            .await
            .map_err(|e| AuditError::Network(e.to_string()))?;
        let (mut read, mut write) = stream.into_split();
        let s = |x: &'static str| Str0255::try_from(x).unwrap();
        let setup = SetupConnection {
            protocol: stratum_core::common_messages_sv2::Protocol::MiningProtocol,
            min_version: 2,
            max_version: 2,
            flags: 0,
            endpoint_host: s("127.0.0.1"),
            endpoint_port: 0,
            vendor: s("spc-scan"),
            hardware_version: s("1"),
            firmware: s("1"),
            device_id: s("1"),
        };
        let frame = MessageFrame::from_message(setup, MESSAGE_TYPE_SETUP_CONNECTION, 0, false)
            .map_err(|e| AuditError::Codec(e.to_string()))?;
        write
            .write_frame(frame)
            .await
            .map_err(|e| AuditError::Network(e.to_string()))?;
        let mut f = read
            .read_frame()
            .await
            .map_err(|e| AuditError::Network(e.to_string()))?;
        if f.header().msg_type() != MESSAGE_TYPE_SETUP_CONNECTION_SUCCESS {
            return Err(AuditError::Codec("expected SetupConnection.Success".into()));
        }
        let mut p = f.payload().to_vec();
        let _: SetupConnectionSuccess =
            binary_sv2::from_bytes(&mut p).map_err(|e| AuditError::Codec(e.to_string()))?;
        Ok(Self { read, write })
    }

    pub async fn audit(
        &mut self,
        block_hash: [u8; 32],
        slices: usize,
        shares: usize,
    ) -> Result<AuditReport, AuditError> {
        let req = Message::GetWindow {
            request_id: 1,
            block_hash,
        };
        self.send(&req).await?;
        let window = loop {
            let msg = self.recv().await?;
            if let Message::GetWindowSuccess { slices, .. } = msg {
                break slices;
            }
        };
        let mut verified = 0usize;
        let mut used_slices = 0usize;
        for (i, slice) in window.iter().take(slices.max(1)).enumerate() {
            let indices: Vec<u32> = (0..shares.min(slice.n_shares as usize) as u32).collect();
            let req = Message::GetShares {
                request_id: 100 + i as u32,
                slice_id: slice.slice_id,
                indices,
            };
            self.send(&req).await?;
            let proofs: Vec<ShareProof> = loop {
                let msg = self.recv().await?;
                if let Message::GetSharesSuccess { shares, .. } = msg {
                    break shares;
                }
            };
            verify_slice(slice, &proofs, 0).map_err(|e| AuditError::Verify(e.to_string()))?;
            verified += proofs.len();
            used_slices += 1;
        }
        Ok(AuditReport {
            slices: used_slices,
            shares: verified,
            verified,
        })
    }

    async fn send(&mut self, msg: &Message) -> Result<(), AuditError> {
        let payload = msg.encode().map_err(|e| AuditError::Codec(e.to_string()))?;
        let mut bytes = Vec::with_capacity(6 + payload.len());
        bytes.extend_from_slice(&EXTENSION_TYPE.to_le_bytes());
        bytes.push(msg.message_type());
        let len = u32::try_from(payload.len()).map_err(|_| AuditError::Codec("too long".into()))?;
        bytes.extend_from_slice(&len.to_le_bytes()[..3]);
        bytes.extend_from_slice(&payload);
        let frame = stratum_core::codec_sv2::SerializedFrame::from_bytes(bytes)
            .map_err(|e| AuditError::Codec(format!("{e:?}")))?;
        self.write
            .write_frame(frame)
            .await
            .map_err(|e| AuditError::Network(e.to_string()))
    }

    async fn recv(&mut self) -> Result<Message, AuditError> {
        loop {
            let mut f = self
                .read
                .read_frame()
                .await
                .map_err(|e| AuditError::Network(e.to_string()))?;
            if f.header().ext_type_without_channel_msg() != EXTENSION_TYPE {
                continue;
            }
            let ty = f.header().msg_type();
            let p = f.payload().to_vec();
            return Message::decode(ty, &p).map_err(|e| AuditError::Codec(e.to_string()));
        }
    }
}

/// Convenience for the CLI.
pub async fn run(
    addr: SocketAddr,
    block_hash: [u8; 32],
    slices: usize,
    shares: usize,
) -> Result<(AuditReport, Vec<SliceSummary>), AuditError> {
    let mut a = Auditor::connect(addr).await?;
    let report = a.audit(block_hash, slices, shares).await?;
    Ok((report, Vec::new()))
}
