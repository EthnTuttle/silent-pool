//! Minimal SV2 miner-side client used by the translator and JDC roles.

use std::net::SocketAddr;

use bitcoin::hashes::Hash;
use spc_crypto::Network;
use stratum_apps::network_helpers::connect_with_noise;
use stratum_apps::network_helpers::noise_stream::{NoiseTcpReadHalf, NoiseTcpWriteHalf};
use stratum_core::binary_sv2::{self, Str0255, B032, U256};
use stratum_core::common_messages_sv2::{
    Protocol, SetupConnection, SetupConnectionSuccess, MESSAGE_TYPE_SETUP_CONNECTION,
    MESSAGE_TYPE_SETUP_CONNECTION_SUCCESS,
};
use stratum_core::framing_sv2::framing::MessageFrame;
use stratum_core::mining_sv2::{
    NewExtendedMiningJob, OpenExtendedMiningChannel, OpenExtendedMiningChannelSuccess,
    SetNewPrevHash, SubmitSharesExtended, MESSAGE_TYPE_MINING_SET_NEW_PREV_HASH,
    MESSAGE_TYPE_NEW_EXTENDED_MINING_JOB, MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL,
    MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL_SUCCESS, MESSAGE_TYPE_SUBMIT_SHARES_EXTENDED,
};
use thiserror::Error;
use tokio::net::TcpStream;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("network: {0}")]
    Network(String),
    #[error("codec: {0}")]
    Codec(String),
    #[error("unexpected message {0}")]
    Unexpected(u8),
    #[error("invalid address")]
    Address,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoolJob {
    pub job_id: u32,
    pub prev_hash: [u8; 32],
    pub version: u32,
    pub nbits: u32,
    pub ntime: u32,
    pub coinbase_prefix: Vec<u8>,
    pub coinbase_suffix: Vec<u8>,
    pub merkle_path: Vec<[u8; 32]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoolShare {
    pub job_id: u32,
    pub nonce: u32,
    pub ntime: u32,
    pub version: u32,
    pub extranonce: Vec<u8>,
    pub sequence_number: u32,
}

pub struct Sv2Client {
    read: NoiseTcpReadHalf,
    write: NoiseTcpWriteHalf,
    channel_id: u32,
    pub extranonce_prefix: Vec<u8>,
    pub extranonce_size: u16,
    sequence: u32,
}

impl Sv2Client {
    pub async fn connect(
        addr: SocketAddr,
        sp_address: &str,
        _network: Network,
    ) -> Result<Self, ClientError> {
        let stream = TcpStream::connect(addr)
            .await
            .map_err(|e| ClientError::Network(e.to_string()))?;
        let stream = connect_with_noise(stream, None)
            .await
            .map_err(|e| ClientError::Network(e.to_string()))?;
        let (mut read, mut write) = stream.into_split();

        let s = |x: &'static str| Str0255::try_from(x).unwrap();
        let setup = SetupConnection {
            protocol: Protocol::MiningProtocol,
            min_version: 2,
            max_version: 2,
            flags: 0,
            endpoint_host: s("127.0.0.1"),
            endpoint_port: 0,
            vendor: s("spc-agent"),
            hardware_version: s("1"),
            firmware: s("1"),
            device_id: s("1"),
        };
        write_msg(&mut write, setup, MESSAGE_TYPE_SETUP_CONNECTION, false).await?;
        let mut f = read
            .read_frame()
            .await
            .map_err(|e| ClientError::Network(e.to_string()))?;
        if f.header().msg_type() != MESSAGE_TYPE_SETUP_CONNECTION_SUCCESS {
            return Err(ClientError::Unexpected(f.header().msg_type()));
        }
        let mut p = f.payload().to_vec();
        let _: SetupConnectionSuccess =
            binary_sv2::from_bytes(&mut p).map_err(|e| ClientError::Codec(e.to_string()))?;

        let max_target_bytes = [0xffu8; 32];
        let open = OpenExtendedMiningChannel {
            request_id: 1,
            user_identity: Str0255::try_from(sp_address).map_err(|_| ClientError::Address)?,
            nominal_hash_rate: 1.0,
            max_target: U256::from(&max_target_bytes),
            min_extranonce_size: 0,
        };
        write_msg(
            &mut write,
            open,
            MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL,
            false,
        )
        .await?;
        let mut f = read
            .read_frame()
            .await
            .map_err(|e| ClientError::Network(e.to_string()))?;
        if f.header().msg_type() != MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL_SUCCESS {
            return Err(ClientError::Unexpected(f.header().msg_type()));
        }
        let mut p = f.payload().to_vec();
        let success: OpenExtendedMiningChannelSuccess =
            binary_sv2::from_bytes(&mut p).map_err(|e| ClientError::Codec(e.to_string()))?;
        Ok(Self {
            read,
            write,
            channel_id: success.channel_id,
            extranonce_prefix: success.extranonce_prefix.as_bytes().to_vec(),
            extranonce_size: success.extranonce_size,
            sequence: 0,
        })
    }

    /// Read messages until the next job is complete.
    pub async fn next_job(&mut self) -> Result<PoolJob, ClientError> {
        let mut prev_hash = None;
        let mut ntime = None;
        let mut nbits = None;
        loop {
            let mut f = self
                .read
                .read_frame()
                .await
                .map_err(|e| ClientError::Network(e.to_string()))?;
            let ty = f.header().msg_type();
            let mut p = f.payload().to_vec();
            match ty {
                MESSAGE_TYPE_MINING_SET_NEW_PREV_HASH => {
                    let m: SetNewPrevHash = binary_sv2::from_bytes(&mut p)
                        .map_err(|e| ClientError::Codec(e.to_string()))?;
                    let mut h = [0u8; 32];
                    h.copy_from_slice(m.prev_hash.as_bytes());
                    prev_hash = Some(h);
                    ntime = Some(m.min_ntime);
                    nbits = Some(m.nbits);
                }
                MESSAGE_TYPE_NEW_EXTENDED_MINING_JOB => {
                    let m: NewExtendedMiningJob = binary_sv2::from_bytes(&mut p)
                        .map_err(|e| ClientError::Codec(e.to_string()))?;
                    let mut merkle_path = Vec::with_capacity(m.merkle_path.len());
                    for b in m.merkle_path.iter() {
                        let mut h = [0u8; 32];
                        h.copy_from_slice(b.as_bytes());
                        merkle_path.push(h);
                    }
                    return Ok(PoolJob {
                        job_id: m.job_id,
                        prev_hash: prev_hash.ok_or(ClientError::Unexpected(ty))?,
                        version: m.version,
                        nbits: nbits.ok_or(ClientError::Unexpected(ty))?,
                        ntime: ntime.ok_or(ClientError::Unexpected(ty))?,
                        coinbase_prefix: m.coinbase_tx_prefix.as_bytes().to_vec(),
                        coinbase_suffix: m.coinbase_tx_suffix.as_bytes().to_vec(),
                        merkle_path,
                    });
                }
                _ => {}
            }
        }
    }

    pub async fn submit(&mut self, share: PoolShare) -> Result<(), ClientError> {
        let msg = SubmitSharesExtended {
            channel_id: self.channel_id,
            sequence_number: self.sequence,
            job_id: share.job_id,
            nonce: share.nonce,
            ntime: share.ntime,
            version: share.version,
            extranonce: B032::new(&share.extranonce)
                .map_err(|e| ClientError::Codec(e.to_string()))?,
        };
        self.sequence = self.sequence.wrapping_add(1);
        write_msg(
            &mut self.write,
            msg,
            MESSAGE_TYPE_SUBMIT_SHARES_EXTENDED,
            true,
        )
        .await
    }

    pub fn channel_id(&self) -> u32 {
        self.channel_id
    }
}

async fn write_msg<T>(
    w: &mut NoiseTcpWriteHalf,
    msg: T,
    msg_type: u8,
    channel_msg: bool,
) -> Result<(), ClientError>
where
    T: binary_sv2::Serialize + binary_sv2::GetSize,
{
    let frame = MessageFrame::from_message(msg, msg_type, 0, channel_msg)
        .map_err(|e| ClientError::Codec(e.to_string()))?;
    w.write_frame(frame)
        .await
        .map_err(|e| ClientError::Network(e.to_string()))
}

/// Convert a pool job to the SV1 job shape. The translator uses an empty
/// extranonce1 and gives the miner the pool's miner-extranonce bytes, so the
/// pool's own `extranonce_prefix` is never rolled by the miner.
pub fn to_sv1_job(job: &PoolJob) -> crate::translate::Sv1Job {
    crate::translate::Sv1Job {
        job_id: format!("{:08x}", job.job_id),
        prev_hash: job.prev_hash,
        height: 0,
        version: job.version,
        nbits: job.nbits,
        ntime: job.ntime,
        coinb1: job.coinbase_prefix.clone(),
        coinb2: job.coinbase_suffix.clone(),
        merkle_branches: job.merkle_path.clone(),
        clean_jobs: true,
    }
}

/// Merkle root and header hash for a share, given the full coinbase.
pub fn share_hash(
    version: u32,
    prev_hash: [u8; 32],
    coinbase: &[u8],
    merkle_path: &[[u8; 32]],
    ntime: u32,
    nbits: u32,
    nonce: u32,
) -> [u8; 32] {
    let mut root = bitcoin::hashes::sha256d::Hash::hash(coinbase).to_byte_array();
    for b in merkle_path.iter().rev() {
        let mut buf = Vec::with_capacity(64);
        buf.extend_from_slice(&root);
        buf.extend_from_slice(b);
        root = bitcoin::hashes::sha256d::Hash::hash(&buf).to_byte_array();
    }
    let mut h = Vec::with_capacity(80);
    h.extend_from_slice(&version.to_le_bytes());
    h.extend_from_slice(&prev_hash);
    h.extend_from_slice(&root);
    h.extend_from_slice(&ntime.to_le_bytes());
    h.extend_from_slice(&nbits.to_le_bytes());
    h.extend_from_slice(&nonce.to_le_bytes());
    bitcoin::hashes::sha256d::Hash::hash(&h).to_byte_array()
}
